use std::fmt;
use std::fs::File;
use std::ops::Deref;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffered;
use crate::identity::CapturedMetadata;
use crate::lifecycle::SnapshotLifecycle;
#[cfg(windows)]
use crate::platform::windows::mapping::{MappedFile, MappingCandidate};
use crate::{
    ByteLength, ByteOffset, ByteRange, FileAccessDiagnostics, FileAccessError, FileAccessOptions,
    FileAccessResult, FileIdentity, MappingFallbackReason, SnapshotState, SnapshotValidation,
    StaleReason, ValidationTarget,
};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
const MAX_PLATFORM_OFFSET: u64 = i64::MAX as u64;

/// Opaque process-local generation assigned when a snapshot is opened.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SnapshotGeneration(u64);

impl SnapshotGeneration {
    fn next() -> FileAccessResult<Self> {
        NEXT_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(Self)
            .map_err(|_| FileAccessError::GenerationExhausted)
    }
}

/// Immutable, fixed-boundary view of an opened regular file.
#[derive(Debug)]
pub struct FileSnapshot {
    identity: FileIdentity,
    backend: Backend,
    path: PathBuf,
    length: ByteLength,
    captured_metadata: CapturedMetadata,
    generation: SnapshotGeneration,
    options: FileAccessOptions,
    diagnostics: FileAccessDiagnostics,
    lifecycle: SnapshotLifecycle,
}

#[derive(Debug)]
enum Backend {
    Buffered,
    #[cfg(windows)]
    Mapped(Arc<MappedFile>),
}

impl FileSnapshot {
    /// Opens a regular file without reading its contents into memory.
    pub fn open(path: impl AsRef<Path>, options: FileAccessOptions) -> FileAccessResult<Self> {
        let path = path.as_ref().to_path_buf();

        // Reject known non-regular paths before opening. On Windows, opening a
        // directory for ordinary file reads fails before handle metadata can be
        // inspected; on Unix, this also avoids blocking while opening a FIFO.
        // The handle metadata check below remains authoritative if the path is
        // replaced between these two operations.
        if std::fs::metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
            return Err(FileAccessError::UnsupportedFileType { path });
        }

        Self::open_platform(path, options)
    }

    #[cfg(target_os = "linux")]
    fn open_platform(path: PathBuf, options: FileAccessOptions) -> FileAccessResult<Self> {
        let (identity, captured_metadata) = Self::open_buffered_identity(&path)?;
        let length = captured_metadata.length;
        let fallback_reason = (length.get() == 0).then_some(MappingFallbackReason::EmptyFile);

        Self::from_parts(
            identity,
            Backend::Buffered,
            path,
            captured_metadata,
            options,
            FileAccessDiagnostics::buffered(fallback_reason),
        )
    }

    #[cfg(windows)]
    fn open_platform(path: PathBuf, options: FileAccessOptions) -> FileAccessResult<Self> {
        Self::open_windows_with(path, options, crate::platform::windows::mapping::try_create)
    }

    #[cfg(windows)]
    fn open_windows_with<Attempt>(
        path: PathBuf,
        options: FileAccessOptions,
        mapping_attempt: Attempt,
    ) -> FileAccessResult<Self>
    where
        Attempt: FnOnce(
            &Path,
            &FileIdentity,
            CapturedMetadata,
        ) -> Result<MappingCandidate, MappingFallbackReason>,
    {
        let (buffered_identity, captured_metadata) = Self::open_buffered_identity(&path)?;

        if captured_metadata.length.get() == 0 {
            return Self::from_parts(
                buffered_identity,
                Backend::Buffered,
                path,
                captured_metadata,
                options,
                FileAccessDiagnostics::buffered(Some(MappingFallbackReason::EmptyFile)),
            );
        }

        match mapping_attempt(&path, &buffered_identity, captured_metadata) {
            Ok(candidate) => Self::from_parts(
                candidate.identity,
                Backend::Mapped(candidate.mapping),
                path,
                captured_metadata,
                options,
                FileAccessDiagnostics::mapped(),
            ),
            Err(reason) => Self::from_parts(
                buffered_identity,
                Backend::Buffered,
                path,
                captured_metadata,
                options,
                FileAccessDiagnostics::buffered(Some(reason)),
            ),
        }
    }

    fn open_buffered_identity(path: &Path) -> FileAccessResult<(FileIdentity, CapturedMetadata)> {
        let file = File::open(path).map_err(|source| FileAccessError::OpenFailed {
            path: path.to_path_buf(),
            source,
        })?;
        let metadata = file
            .metadata()
            .map_err(|source| FileAccessError::MetadataFailed {
                path: path.to_path_buf(),
                source,
            })?;

        if !metadata.is_file() {
            return Err(FileAccessError::UnsupportedFileType {
                path: path.to_path_buf(),
            });
        }

        let captured_metadata = CapturedMetadata::from_metadata(&metadata);
        let identity =
            FileIdentity::from_file(file).map_err(|source| FileAccessError::IdentityFailed {
                path: path.to_path_buf(),
                source,
            })?;

        Ok((identity, captured_metadata))
    }

    fn from_parts(
        identity: FileIdentity,
        backend: Backend,
        path: PathBuf,
        captured_metadata: CapturedMetadata,
        options: FileAccessOptions,
        diagnostics: FileAccessDiagnostics,
    ) -> FileAccessResult<Self> {
        Ok(Self {
            identity,
            backend,
            path,
            length: captured_metadata.length,
            captured_metadata,
            generation: SnapshotGeneration::next()?,
            options,
            diagnostics,
            lifecycle: SnapshotLifecycle::fresh(),
        })
    }

    /// Returns the path used to open this snapshot.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the captured file length.
    #[must_use]
    pub const fn len(&self) -> ByteLength {
        self.length
    }

    /// Returns whether the captured file was empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length.get() == 0
    }

    /// Returns this snapshot's opaque generation.
    #[must_use]
    pub const fn generation(&self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns the opaque identity captured from the opened file handle.
    #[must_use]
    pub const fn identity(&self) -> &FileIdentity {
        &self.identity
    }

    /// Returns the snapshot's current one-way lifecycle state.
    #[must_use]
    pub fn state(&self) -> SnapshotState {
        self.lifecycle.state()
    }

    /// Returns the first stale reason, if the snapshot is stale.
    #[must_use]
    pub fn stale_reason(&self) -> Option<StaleReason> {
        self.lifecycle.stale_reason()
    }

    /// Returns the configured maximum view size.
    #[must_use]
    pub const fn max_view_bytes(&self) -> ByteLength {
        self.options.max_view_bytes()
    }

    /// Returns diagnostic backend information.
    #[must_use]
    pub const fn diagnostics(&self) -> FileAccessDiagnostics {
        self.diagnostics
    }

    /// Reads an exact, bounded range into an immutable owned view.
    pub fn view(&self, range: ByteRange) -> FileAccessResult<RangeView> {
        match &self.backend {
            Backend::Buffered => self.view_with_reader(range, buffered::read_exact_at),
            #[cfg(windows)]
            Backend::Mapped(mapping) => self.mapped_view(range, Arc::clone(mapping)),
        }
    }

    fn view_with_reader<ReadExact>(
        &self,
        range: ByteRange,
        read_exact_at: ReadExact,
    ) -> FileAccessResult<RangeView>
    where
        ReadExact: FnOnce(&File, ByteOffset, &mut [u8]) -> FileAccessResult<()>,
    {
        let length = self.validate_view_request(range)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|source| FileAccessError::AllocationFailed {
                requested: range.length(),
                source,
            })?;
        bytes.resize(length, 0);

        if !bytes.is_empty()
            && let Err(error) = read_exact_at(self.identity.file(), range.offset(), &mut bytes)
        {
            self.mark_read_failure(&error);
            return Err(error);
        }

        Ok(RangeView {
            backing: RangeBacking::Buffered(bytes.into_boxed_slice()),
            range,
            generation: self.generation,
        })
    }

    #[cfg(windows)]
    fn mapped_view(
        &self,
        range: ByteRange,
        mapping: Arc<MappedFile>,
    ) -> FileAccessResult<RangeView> {
        let _ = self.validate_view_request(range)?;
        let start = byte_offset_to_usize(range.offset())?;
        let end = byte_offset_to_usize(range.end())?;

        Ok(RangeView {
            backing: RangeBacking::Mapped {
                mapping,
                start,
                end,
            },
            range,
            generation: self.generation,
        })
    }

    fn validate_view_request(&self, range: ByteRange) -> FileAccessResult<usize> {
        self.ensure_fresh()?;
        self.ensure_in_bounds(range.offset(), range.length(), range.end())?;

        if range.length() > self.options.max_view_bytes() {
            return Err(FileAccessError::RangeTooLarge {
                requested: range.length(),
                maximum: self.options.max_view_bytes(),
            });
        }

        ensure_platform_range(range.offset(), range.length(), range.end())?;
        range.length().try_to_usize()
    }

    /// Reads from an explicit offset into a caller-provided buffer.
    ///
    /// The method fills the portion of `buffer` that lies within the captured
    /// snapshot boundary. It returns zero at exact EOF and rejects offsets
    /// beyond EOF.
    pub fn read_at(&self, offset: ByteOffset, buffer: &mut [u8]) -> FileAccessResult<usize> {
        self.ensure_fresh()?;
        let requested = usize_to_byte_length(buffer.len())?;

        if offset.get() > self.length.get() {
            return Err(FileAccessError::OutOfBounds {
                offset,
                length: requested,
                snapshot_length: self.length,
            });
        }

        if offset.get() == self.length.get() || buffer.is_empty() {
            return Ok(0);
        }

        let remaining = self.length.get() - offset.get();
        let to_read = ByteLength::new(requested.get().min(remaining));
        let end = ByteOffset::new(offset.get().checked_add(to_read.get()).ok_or(
            FileAccessError::RangeOverflow {
                offset,
                length: to_read,
            },
        )?);
        ensure_platform_range(offset, to_read, end)?;

        let to_read_usize = to_read.try_to_usize()?;
        match &self.backend {
            Backend::Buffered => {
                if let Err(error) = buffered::read_exact_at(
                    self.identity.file(),
                    offset,
                    &mut buffer[..to_read_usize],
                ) {
                    self.mark_read_failure(&error);
                    return Err(error);
                }
            }
            #[cfg(windows)]
            Backend::Mapped(mapping) => {
                let start = byte_offset_to_usize(offset)?;
                let end =
                    start
                        .checked_add(to_read_usize)
                        .ok_or(FileAccessError::RangeOverflow {
                            offset,
                            length: to_read,
                        })?;
                buffer[..to_read_usize].copy_from_slice(mapping.slice(start, end));
            }
        }
        Ok(to_read_usize)
    }

    /// Explicitly compares the opened file with the path's current target.
    ///
    /// This is the only normal operation that performs complete identity and
    /// metadata validation. Successful `view` and `read_at` calls do not call
    /// this method or query path metadata.
    pub fn validate(&self) -> FileAccessResult<SnapshotValidation> {
        if let Some(reason) = self.lifecycle.stale_reason() {
            return Ok(SnapshotValidation::Stale(reason));
        }

        let open_metadata = self
            .identity
            .metadata()
            .map_err(|source| self.validation_failure(ValidationTarget::OpenFile, source))?;
        let open_metadata = CapturedMetadata::from_metadata(&open_metadata);

        let current_identity = match FileIdentity::from_path(&self.path) {
            Ok(identity) => identity,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(self.mark_stale(StaleReason::Missing));
            }
            Err(source) => {
                return Err(self.validation_failure(ValidationTarget::CurrentPath, source));
            }
        };
        let current_metadata = current_identity
            .metadata()
            .map_err(|source| self.validation_failure(ValidationTarget::CurrentPath, source))?;

        if !current_metadata.is_file() || self.identity != current_identity {
            return Ok(self.mark_stale(StaleReason::Replaced));
        }

        let current_metadata = CapturedMetadata::from_metadata(&current_metadata);
        let original_length = self.captured_metadata.length;
        let observed_growth =
            open_metadata.length > original_length || current_metadata.length > original_length;
        let observed_truncate =
            open_metadata.length < original_length || current_metadata.length < original_length;

        if observed_growth && observed_truncate {
            return Ok(self.mark_stale(StaleReason::Modified));
        }
        if observed_growth {
            return Ok(self.mark_stale(StaleReason::Grown));
        }
        if observed_truncate {
            return Ok(self.mark_stale(StaleReason::Truncated));
        }

        if open_metadata != current_metadata {
            return Ok(self.mark_stale(StaleReason::Modified));
        }

        match (
            self.captured_metadata.modification,
            open_metadata.modification,
        ) {
            (Some(original), Some(current)) if original != current => {
                return Ok(self.mark_stale(StaleReason::Modified));
            }
            (Some(_), Some(_)) => {}
            _ => return Ok(self.mark_stale(StaleReason::Unverifiable)),
        }

        match self.lifecycle.state() {
            SnapshotState::Fresh => Ok(SnapshotValidation::Unchanged),
            SnapshotState::Stale(reason) => Ok(SnapshotValidation::Stale(reason)),
        }
    }

    /// Opens the path's current target as a separate snapshot and generation.
    pub fn reopen(&self) -> FileAccessResult<Self> {
        Self::open(&self.path, self.options)
    }

    fn ensure_in_bounds(
        &self,
        offset: ByteOffset,
        length: ByteLength,
        end: ByteOffset,
    ) -> FileAccessResult<()> {
        if end.get() > self.length.get() {
            return Err(FileAccessError::OutOfBounds {
                offset,
                length,
                snapshot_length: self.length,
            });
        }

        Ok(())
    }

    fn ensure_fresh(&self) -> FileAccessResult<()> {
        match self.lifecycle.state() {
            SnapshotState::Fresh => Ok(()),
            SnapshotState::Stale(reason) => Err(FileAccessError::StaleSnapshot { reason }),
        }
    }

    fn mark_stale(&self, reason: StaleReason) -> SnapshotValidation {
        SnapshotValidation::Stale(self.lifecycle.mark_stale(reason))
    }

    fn mark_read_failure(&self, error: &FileAccessError) {
        if matches!(error, FileAccessError::UnexpectedEof { .. }) {
            self.lifecycle.mark_stale(StaleReason::UnexpectedEof);
        }
    }

    fn validation_failure(
        &self,
        target: ValidationTarget,
        source: std::io::Error,
    ) -> FileAccessError {
        self.lifecycle.mark_stale(StaleReason::Unverifiable);
        FileAccessError::ValidationFailed {
            path: self.path.clone(),
            target,
            source,
        }
    }
}

/// Opaque immutable bytes returned by [`FileSnapshot::view`].
pub struct RangeView {
    backing: RangeBacking,
    range: ByteRange,
    generation: SnapshotGeneration,
}

enum RangeBacking {
    Buffered(Box<[u8]>),
    #[cfg(windows)]
    Mapped {
        mapping: Arc<MappedFile>,
        start: usize,
        end: usize,
    },
}

impl RangeView {
    /// Returns the bytes held by this view.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.backing {
            RangeBacking::Buffered(bytes) => bytes,
            #[cfg(windows)]
            RangeBacking::Mapped {
                mapping,
                start,
                end,
            } => mapping.slice(*start, *end),
        }
    }

    /// Returns the exact source range represented by this view.
    #[must_use]
    pub const fn range(&self) -> ByteRange {
        self.range
    }

    /// Returns the snapshot generation that produced this view.
    #[must_use]
    pub const fn generation(&self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns the number of owned view bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Returns whether this view contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }
}

impl fmt::Debug for RangeView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RangeView")
            .field("bytes", &self.as_bytes())
            .field("range", &self.range)
            .field("generation", &self.generation)
            .finish()
    }
}

impl AsRef<[u8]> for RangeView {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Deref for RangeView {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

fn ensure_platform_range(
    offset: ByteOffset,
    length: ByteLength,
    end: ByteOffset,
) -> FileAccessResult<()> {
    if length.get() == 0 {
        return Ok(());
    }

    let last = end.get() - 1;
    if last > MAX_PLATFORM_OFFSET {
        return Err(FileAccessError::OffsetNotRepresentable {
            offset: ByteOffset::new(last),
        });
    }

    let _ = offset;
    Ok(())
}

fn usize_to_byte_length(value: usize) -> FileAccessResult<ByteLength> {
    let value = u64::try_from(value).map_err(|_| FileAccessError::LengthNotRepresentable {
        length: ByteLength::new(u64::MAX),
    })?;
    Ok(ByteLength::new(value))
}

#[cfg(windows)]
fn byte_offset_to_usize(offset: ByteOffset) -> FileAccessResult<usize> {
    usize::try_from(offset.get()).map_err(|_| FileAccessError::OffsetNotRepresentable { offset })
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::atomic::AtomicU64;

    use super::*;

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn platform_range_rejects_offsets_above_signed_os_limit() {
        let offset = ByteOffset::new(MAX_PLATFORM_OFFSET);
        let length = ByteLength::new(2);
        let end = ByteOffset::new(MAX_PLATFORM_OFFSET + 2);

        let error = ensure_platform_range(offset, length, end).unwrap_err();
        assert!(matches!(
            error,
            FileAccessError::OffsetNotRepresentable { .. }
        ));
    }

    #[test]
    fn file_snapshot_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FileSnapshot>();
        assert_send_sync::<RangeView>();
    }

    #[test]
    fn generic_positioned_read_failure_does_not_poison_snapshot() {
        let id = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "faultsift-read-failure-{}-{id}.tmp",
            std::process::id()
        ));
        std::fs::write(&path, b"healthy bytes").unwrap();

        let options = FileAccessOptions::new(ByteLength::new(64)).unwrap();
        let snapshot = FileSnapshot::open(&path, options).unwrap();
        let range = ByteRange::new(ByteOffset::new(0), ByteLength::new(7)).unwrap();

        let error = snapshot
            .view_with_reader(range, |_file, offset, _buffer| {
                Err(FileAccessError::ReadFailed {
                    offset,
                    source: io::Error::other("injected transient read failure"),
                })
            })
            .unwrap_err();

        assert!(matches!(error, FileAccessError::ReadFailed { .. }));
        assert_eq!(snapshot.state(), SnapshotState::Fresh);
        assert_eq!(snapshot.view(range).unwrap().as_bytes(), b"healthy");
        assert_eq!(snapshot.state(), SnapshotState::Fresh);

        drop(snapshot);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn injected_mapping_failure_uses_working_buffered_fallback() {
        let id = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "faultsift-mapping-failure-{}-{id}.tmp",
            std::process::id()
        ));
        std::fs::write(&path, b"fallback bytes").unwrap();

        let options = FileAccessOptions::new(ByteLength::new(64)).unwrap();
        let snapshot = FileSnapshot::open_windows_with(
            path.clone(),
            options,
            |_path, _identity, _metadata| Err(MappingFallbackReason::MappingCreationFailed),
        )
        .unwrap();

        assert_eq!(
            snapshot.diagnostics().backend(),
            crate::BackendKind::Buffered
        );
        assert_eq!(
            snapshot.diagnostics().mapping_fallback_reason(),
            Some(MappingFallbackReason::MappingCreationFailed)
        );
        assert!(snapshot.diagnostics().used_buffered_fallback());
        assert_eq!(
            snapshot
                .view(ByteRange::new(ByteOffset::new(0), ByteLength::new(8)).unwrap())
                .unwrap()
                .as_bytes(),
            b"fallback"
        );
        let mut buffer = [0_u8; 5];
        assert_eq!(
            snapshot.read_at(ByteOffset::new(9), &mut buffer).unwrap(),
            5
        );
        assert_eq!(&buffer, b"bytes");

        drop(snapshot);
        std::fs::remove_file(path).unwrap();
    }
}
