use std::fs::File;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffered;
use crate::{
    ByteLength, ByteOffset, ByteRange, FileAccessDiagnostics, FileAccessError, FileAccessOptions,
    FileAccessResult,
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
    file: File,
    path: PathBuf,
    length: ByteLength,
    generation: SnapshotGeneration,
    options: FileAccessOptions,
    diagnostics: FileAccessDiagnostics,
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

        let file = File::open(&path).map_err(|source| FileAccessError::OpenFailed {
            path: path.clone(),
            source,
        })?;
        let metadata = file
            .metadata()
            .map_err(|source| FileAccessError::MetadataFailed {
                path: path.clone(),
                source,
            })?;

        if !metadata.is_file() {
            return Err(FileAccessError::UnsupportedFileType { path });
        }

        Ok(Self {
            file,
            path,
            length: ByteLength::new(metadata.len()),
            generation: SnapshotGeneration::next()?,
            options,
            diagnostics: FileAccessDiagnostics::buffered(),
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
        self.ensure_in_bounds(range.offset(), range.length(), range.end())?;

        if range.length() > self.options.max_view_bytes() {
            return Err(FileAccessError::RangeTooLarge {
                requested: range.length(),
                maximum: self.options.max_view_bytes(),
            });
        }

        ensure_platform_range(range.offset(), range.length(), range.end())?;
        let length = range.length().try_to_usize()?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|source| FileAccessError::AllocationFailed {
                requested: range.length(),
                source,
            })?;
        bytes.resize(length, 0);

        if !bytes.is_empty() {
            buffered::read_exact_at(&self.file, range.offset(), &mut bytes)?;
        }

        Ok(RangeView {
            bytes: bytes.into_boxed_slice(),
            range,
            generation: self.generation,
        })
    }

    /// Reads from an explicit offset into a caller-provided buffer.
    ///
    /// The method fills the portion of `buffer` that lies within the captured
    /// snapshot boundary. It returns zero at exact EOF and rejects offsets
    /// beyond EOF.
    pub fn read_at(&self, offset: ByteOffset, buffer: &mut [u8]) -> FileAccessResult<usize> {
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
        buffered::read_exact_at(&self.file, offset, &mut buffer[..to_read_usize])?;
        Ok(to_read_usize)
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
}

/// Opaque immutable bytes returned by [`FileSnapshot::view`].
#[derive(Debug)]
pub struct RangeView {
    bytes: Box<[u8]>,
    range: ByteRange,
    generation: SnapshotGeneration,
}

impl RangeView {
    /// Returns the bytes held by this view.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
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
        self.bytes.len()
    }

    /// Returns whether this view contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
