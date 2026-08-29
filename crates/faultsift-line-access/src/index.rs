use std::convert::Infallible;
use std::sync::Arc;

use faultsift_file_access::{
    ByteLength, FileAccessError, FileSnapshot, SnapshotGeneration, SnapshotState,
};

use crate::scanner::{ByteScanner, ScanError, ScannedLine};
use crate::{LineAccessError, LineAccessResult, LineContentChunk, LineIndexOptions};

/// First and only initial checkpoint stride approved for the eager index.
pub const INITIAL_STRIDE: u64 = 256;

/// Infallible current-thread control returned from a build progress callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildControl {
    Continue,
    Cancel,
}

/// Monotonic observation emitted only at bounded scanner chunk boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildProgress {
    bytes_scanned: ByteLength,
    snapshot_length: ByteLength,
    physical_lines_completed: u64,
    current_stride: u64,
    checkpoint_count: u64,
}

impl BuildProgress {
    /// Returns source bytes completely consumed at this scan boundary.
    #[must_use]
    pub const fn bytes_scanned(self) -> ByteLength {
        self.bytes_scanned
    }

    /// Returns the immutable captured snapshot boundary.
    #[must_use]
    pub const fn snapshot_length(self) -> ByteLength {
        self.snapshot_length
    }

    /// Returns the exact number of complete physical lines seen so far.
    #[must_use]
    pub const fn physical_lines_completed(self) -> u64 {
        self.physical_lines_completed
    }

    /// Returns the current adaptive checkpoint stride.
    #[must_use]
    pub const fn current_stride(self) -> u64 {
        self.current_stride
    }

    /// Returns the current bounded checkpoint count.
    #[must_use]
    pub const fn checkpoint_count(self) -> u64 {
        self.checkpoint_count
    }
}

/// Immutable metadata and sparse checkpoints from one complete snapshot scan.
pub struct LineIndex {
    snapshot: Arc<FileSnapshot>,
    generation: SnapshotGeneration,
    snapshot_length: ByteLength,
    physical_line_count: u64,
    final_stride: u64,
    checkpoints: Vec<u64>,
    options: LineIndexOptions,
}

impl LineIndex {
    /// Builds one complete eager index without cancellation.
    pub fn build(snapshot: Arc<FileSnapshot>, options: LineIndexOptions) -> LineAccessResult<Self> {
        Self::build_with_control(snapshot, options, |_| BuildControl::Continue)
    }

    /// Builds one complete eager index with synchronous chunk-boundary control.
    pub fn build_with_control(
        snapshot: Arc<FileSnapshot>,
        options: LineIndexOptions,
        mut control: impl FnMut(BuildProgress) -> BuildControl,
    ) -> LineAccessResult<Self> {
        ensure_snapshot_fresh(&snapshot)?;

        let generation = snapshot.generation();
        let snapshot_length = snapshot.len();
        let mut scanner = ByteScanner::new(Arc::clone(&snapshot), options.scan_options())?;
        let mut build = CheckpointState::new(options.max_checkpoints())?;

        loop {
            let scanned = {
                let mut on_chunk_boundary = |bytes_scanned| {
                    report_progress(
                        &snapshot,
                        snapshot_length,
                        bytes_scanned,
                        &build,
                        &mut control,
                    )
                };
                let mut ignore_content = ignore_line_content;
                scanner.scan_next_line_with_chunk_boundaries(
                    &mut ignore_content,
                    &mut on_chunk_boundary,
                )
            };

            let scanned = match scanned {
                Ok(Some(scanned)) => scanned,
                Ok(None) => break,
                Err(ScanError::Visitor(never)) => match never {},
                Err(ScanError::FileAccess(source)) => {
                    return Err(LineAccessError::FileAccess(source));
                }
                Err(ScanError::Scanner(error)) => return Err(error),
            };

            build.record_completed_line(&scanned)?;
            let mut on_chunk_boundary = |bytes_scanned| {
                report_progress(
                    &snapshot,
                    snapshot_length,
                    bytes_scanned,
                    &build,
                    &mut control,
                )
            };
            scanner.report_consumed_chunk_boundary(&mut on_chunk_boundary)?;
        }

        if snapshot_length.get() == 0 {
            report_progress(&snapshot, snapshot_length, 0, &build, &mut control)?;
        }
        ensure_snapshot_fresh(&snapshot)?;

        Ok(Self {
            snapshot,
            generation,
            snapshot_length,
            physical_line_count: build.physical_lines_completed,
            final_stride: build.current_stride,
            checkpoints: build.checkpoints,
            options,
        })
    }

    /// Returns the exact completed physical-line count.
    #[must_use]
    pub const fn line_count(&self) -> u64 {
        self.physical_line_count
    }

    /// Returns the generation captured from the bound snapshot instance.
    #[must_use]
    pub const fn generation(&self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns the immutable captured snapshot length.
    #[must_use]
    pub const fn snapshot_length(&self) -> ByteLength {
        self.snapshot_length
    }

    /// Returns the final adaptive checkpoint stride.
    #[must_use]
    pub const fn final_stride(&self) -> u64 {
        self.final_stride
    }

    /// Returns the retained checkpoint count.
    #[must_use]
    pub fn checkpoint_count(&self) -> u64 {
        self.checkpoints.len() as u64
    }

    /// Returns the configured logical checkpoint-storage budget.
    #[must_use]
    pub const fn checkpoint_budget_bytes(&self) -> ByteLength {
        self.options.checkpoint_budget_bytes()
    }

    /// Returns the configured reusable scan-buffer size.
    #[must_use]
    pub const fn scan_chunk_bytes(&self) -> ByteLength {
        self.options.scan_chunk_bytes()
    }

    /// Returns the exact snapshot instance retained by this ready index.
    #[must_use]
    pub const fn snapshot(&self) -> &Arc<FileSnapshot> {
        &self.snapshot
    }
}

fn ignore_line_content(_: LineContentChunk<'_>) -> Result<(), Infallible> {
    Ok(())
}

struct CheckpointState {
    checkpoints: Vec<u64>,
    max_checkpoints: usize,
    current_stride: u64,
    physical_lines_completed: u64,
}

impl CheckpointState {
    fn new(max_checkpoints: usize) -> LineAccessResult<Self> {
        let max_checkpoints_u64 = u64::try_from(max_checkpoints)
            .map_err(|_| LineAccessError::CheckpointArithmeticOverflow)?;
        let mut checkpoints = Vec::new();
        checkpoints
            .try_reserve_exact(max_checkpoints)
            .map_err(|source| LineAccessError::CheckpointAllocationFailed {
                max_checkpoints: max_checkpoints_u64,
                source,
            })?;
        if checkpoints.capacity() > max_checkpoints {
            return Err(LineAccessError::CheckpointCapacityExceeded {
                capacity: u64::try_from(checkpoints.capacity())
                    .map_err(|_| LineAccessError::CheckpointArithmeticOverflow)?,
                max_checkpoints: max_checkpoints_u64,
            });
        }

        Ok(Self {
            checkpoints,
            max_checkpoints,
            current_stride: INITIAL_STRIDE,
            physical_lines_completed: 0,
        })
    }

    fn record_completed_line(&mut self, scanned: &ScannedLine) -> LineAccessResult<()> {
        let line_number = self.physical_lines_completed;
        if line_number.is_multiple_of(self.current_stride) {
            self.make_room_for(line_number)?;
            if line_number.is_multiple_of(self.current_stride) {
                debug_assert!(self.checkpoints.len() < self.max_checkpoints);
                self.checkpoints.push(scanned.physical_range.offset().get());
            }
        }
        self.physical_lines_completed = self.physical_lines_completed.checked_add(1).ok_or(
            LineAccessError::LineNumberOverflow {
                line_number: crate::LineNumber::new(line_number),
            },
        )?;
        Ok(())
    }

    fn make_room_for(&mut self, line_number: u64) -> LineAccessResult<()> {
        while self.checkpoints.len() == self.max_checkpoints
            && line_number.is_multiple_of(self.current_stride)
        {
            self.compact()?;
        }
        Ok(())
    }

    fn compact(&mut self) -> LineAccessResult<()> {
        let doubled =
            self.current_stride
                .checked_mul(2)
                .ok_or(LineAccessError::StrideOverflow {
                    current_stride: self.current_stride,
                })?;
        let old_len = self.checkpoints.len();
        let mut write = 1;
        for read in (2..old_len).step_by(2) {
            self.checkpoints[write] = self.checkpoints[read];
            write += 1;
        }
        self.checkpoints.truncate(write.min(old_len));
        self.current_stride = doubled;
        Ok(())
    }

    fn checkpoint_count(&self) -> u64 {
        self.checkpoints.len() as u64
    }
}

fn report_progress(
    snapshot: &FileSnapshot,
    snapshot_length: ByteLength,
    bytes_scanned: u64,
    build: &CheckpointState,
    control: &mut impl FnMut(BuildProgress) -> BuildControl,
) -> LineAccessResult<()> {
    debug_assert!(bytes_scanned <= snapshot_length.get());
    let progress = BuildProgress {
        bytes_scanned: ByteLength::new(bytes_scanned),
        snapshot_length,
        physical_lines_completed: build.physical_lines_completed,
        current_stride: build.current_stride,
        checkpoint_count: build.checkpoint_count(),
    };
    if control(progress) == BuildControl::Cancel {
        return Err(LineAccessError::IndexBuildCancelled);
    }
    ensure_snapshot_fresh(snapshot)
}

fn ensure_snapshot_fresh(snapshot: &FileSnapshot) -> LineAccessResult<()> {
    match snapshot.state() {
        SnapshotState::Fresh => Ok(()),
        SnapshotState::Stale(reason) => Err(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot { reason },
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use faultsift_file_access::{ByteOffset, ByteRange};
    use faultsift_file_access::{FileAccessOptions, FileSnapshot};

    use super::*;
    use crate::LineTerminator;

    static NEXT_INDEX_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct IndexFixture {
        path: PathBuf,
    }

    impl IndexFixture {
        fn from_bytes(bytes: &[u8]) -> Self {
            loop {
                let id = NEXT_INDEX_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "faultsift-line-index-unit-{}-{id}.log",
                    std::process::id()
                ));
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(mut file) => {
                        file.write_all(bytes).unwrap();
                        file.flush().unwrap();
                        return Self { path };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create index fixture: {error}"),
                }
            }
        }
    }

    impl Drop for IndexFixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn scanned_line(start: u64, end: u64) -> ScannedLine {
        let range = ByteRange::new(ByteOffset::new(start), ByteLength::new(end - start)).unwrap();
        ScannedLine {
            content_range: range,
            physical_range: range,
            terminator: LineTerminator::None,
        }
    }

    #[test]
    fn checkpoint_state_compacts_in_place_and_preserves_alignment() {
        let mut state = CheckpointState::new(3).unwrap();
        let initial_capacity = state.checkpoints.capacity();
        for line in 0..=2_048_u64 {
            state
                .record_completed_line(&scanned_line(line * 2, line * 2 + 1))
                .unwrap();
        }

        assert_eq!(initial_capacity, 3);
        assert_eq!(state.checkpoints.capacity(), 3);
        assert!(state.checkpoints.len() <= 3);
        assert!(state.current_stride >= 1_024);
        for (index, offset) in state.checkpoints.iter().copied().enumerate() {
            assert_eq!(offset, index as u64 * state.current_stride * 2);
        }
    }

    #[test]
    fn complete_build_retains_exact_scanner_line_start_offsets() {
        let bytes = vec![b'\n'; 4_097];
        let fixture = IndexFixture::from_bytes(&bytes);
        let snapshot =
            Arc::new(FileSnapshot::open(&fixture.path, FileAccessOptions::default()).unwrap());
        let options = LineIndexOptions::new(ByteLength::new(24), ByteLength::new(31)).unwrap();
        let index = LineIndex::build(snapshot, options).unwrap();

        assert_eq!(index.line_count(), 4_097);
        assert_eq!(index.final_stride(), 2_048);
        assert_eq!(index.checkpoints, [0, 2_048, 4_096]);
        assert_eq!(index.checkpoints.capacity(), 3);
    }

    #[test]
    fn odd_checkpoint_count_keeps_even_old_indices() {
        let mut state = CheckpointState::new(5).unwrap();
        state.checkpoints.extend([0, 256, 512, 768, 1_024]);
        let capacity = state.checkpoints.capacity();
        state.compact().unwrap();
        assert_eq!(state.checkpoints, [0, 512, 1_024]);
        assert_eq!(state.current_stride, 512);
        assert_eq!(state.checkpoints.capacity(), capacity);
    }

    #[test]
    fn stride_overflow_is_typed_and_does_not_wrap() {
        let mut state = CheckpointState::new(2).unwrap();
        state.checkpoints.extend([0, 1]);
        state.current_stride = 1_u64 << 63;
        assert!(matches!(
            state.compact(),
            Err(LineAccessError::StrideOverflow {
                current_stride
            }) if current_stride == 1_u64 << 63
        ));
    }

    #[test]
    fn checkpoint_offsets_and_counts_remain_u64_beyond_four_gib() {
        const BEYOND_FOUR_GIB: u64 = (1_u64 << 32) + 17;
        let mut state = CheckpointState::new(2).unwrap();
        state
            .record_completed_line(&scanned_line(BEYOND_FOUR_GIB, BEYOND_FOUR_GIB + 1))
            .unwrap();
        assert_eq!(state.checkpoints, [BEYOND_FOUR_GIB]);

        state.physical_lines_completed = u64::MAX;
        assert!(matches!(
            state.record_completed_line(&scanned_line(0, 1)),
            Err(LineAccessError::LineNumberOverflow { .. })
        ));
    }
}
