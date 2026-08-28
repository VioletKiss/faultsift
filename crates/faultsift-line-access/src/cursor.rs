use std::sync::Arc;

use faultsift_file_access::{
    ByteLength, FileAccessError, FileSnapshot, SnapshotGeneration, SnapshotState,
};

use crate::scanner::{ByteScanner, ScanError};
use crate::{
    CursorFailure, CursorState, LineAccessError, LineAccessResult, LineContentChunk,
    LineDescriptor, LineNumber, ScanOptions, VisitLineError,
};

/// Stateful, single-owner cursor that streams physical-line content.
pub struct PhysicalLineCursor {
    scanner: ByteScanner,
    options: ScanOptions,
    generation: SnapshotGeneration,
    next_line_number: LineNumber,
    state: CursorState,
}

impl PhysicalLineCursor {
    /// Binds a new cursor to one snapshot instance and explicit scan resources.
    pub fn new(snapshot: Arc<FileSnapshot>, options: ScanOptions) -> LineAccessResult<Self> {
        let generation = snapshot.generation();
        let scanner = ByteScanner::new(snapshot, options)?;
        Ok(Self {
            scanner,
            options,
            generation,
            next_line_number: LineNumber::new(0),
            state: CursorState::Active,
        })
    }

    /// Returns the bound snapshot generation.
    #[must_use]
    pub const fn generation(&self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns the captured snapshot boundary used by this cursor.
    #[must_use]
    pub const fn captured_length(&self) -> ByteLength {
        self.scanner.captured_length()
    }

    /// Returns the explicitly configured reusable scan-buffer size.
    #[must_use]
    pub const fn scan_chunk_bytes(&self) -> ByteLength {
        self.options.scan_chunk_bytes()
    }

    /// Returns the cursor's current active, exhausted, or terminal state.
    #[must_use]
    pub const fn state(&self) -> CursorState {
        self.state
    }

    /// Returns the snapshot instance retained for this cursor's lifetime.
    #[must_use]
    pub fn snapshot(&self) -> &Arc<FileSnapshot> {
        self.scanner.snapshot()
    }

    /// Streams the next line's content chunks and then returns its descriptor.
    ///
    /// A visitor or scanner/read failure returns no descriptor and permanently
    /// transitions this cursor to a failed state.
    pub fn visit_next_line<E>(
        &mut self,
        mut visitor: impl FnMut(LineContentChunk<'_>) -> Result<(), E>,
    ) -> Result<Option<LineDescriptor>, VisitLineError<E>> {
        match self.state {
            CursorState::Exhausted => return Ok(None),
            CursorState::Failed(failure) => {
                return Err(LineAccessError::CursorFailed { failure }.into());
            }
            CursorState::Active => {}
        }

        if let SnapshotState::Stale(reason) = self.scanner.snapshot().state() {
            self.state = CursorState::Failed(CursorFailure::Read);
            return Err(VisitLineError::LineAccess(LineAccessError::FileAccess(
                FileAccessError::StaleSnapshot { reason },
            )));
        }

        match self.scanner.scan_next_line(&mut visitor) {
            Ok(Some(scanned)) => {
                let line_number = self.next_line_number;
                self.next_line_number = line_number.checked_next().map_err(|error| {
                    self.state = CursorState::Failed(CursorFailure::Scanner);
                    VisitLineError::LineAccess(error)
                })?;
                Ok(Some(LineDescriptor::from_parts(
                    self.generation,
                    line_number,
                    scanned.content_range,
                    scanned.physical_range,
                    scanned.terminator,
                )))
            }
            Ok(None) => {
                self.state = CursorState::Exhausted;
                Ok(None)
            }
            Err(ScanError::Visitor(error)) => {
                self.state = CursorState::Failed(CursorFailure::Visitor);
                Err(VisitLineError::Visitor(error))
            }
            Err(ScanError::FileAccess(error)) => {
                self.state = CursorState::Failed(CursorFailure::Read);
                Err(VisitLineError::LineAccess(LineAccessError::FileAccess(
                    error,
                )))
            }
            Err(ScanError::Scanner(error)) => {
                self.state = CursorState::Failed(CursorFailure::Scanner);
                Err(VisitLineError::LineAccess(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use faultsift_file_access::{ByteLength, FileAccessOptions};

    use super::*;
    use crate::LineTerminator;

    #[test]
    fn scanner_and_descriptor_coordinates_cross_four_gib() {
        const FOUR_GIB: u64 = 1_u64 << 32;
        let snapshot = Arc::new(
            FileSnapshot::open(
                std::env::current_exe().unwrap(),
                FileAccessOptions::default(),
            )
            .unwrap(),
        );
        let generation = snapshot.generation();
        let mut cursor =
            PhysicalLineCursor::new(snapshot, ScanOptions::new(ByteLength::new(4)).unwrap())
                .unwrap();
        cursor
            .scanner
            .install_test_window(FOUR_GIB - 2, b"ab\r\n", ByteLength::new(FOUR_GIB + 2));

        let mut content = Vec::new();
        let mut chunk_ranges = Vec::new();
        let descriptor = cursor
            .visit_next_line(|chunk| {
                content.extend_from_slice(chunk.bytes());
                chunk_ranges.push(chunk.range());
                Ok::<(), Infallible>(())
            })
            .unwrap()
            .unwrap();

        assert_eq!(content, b"ab");
        assert_eq!(chunk_ranges.len(), 1);
        assert_eq!(chunk_ranges[0].offset().get(), FOUR_GIB - 2);
        assert_eq!(chunk_ranges[0].end().get(), FOUR_GIB);
        assert_eq!(descriptor.generation(), generation);
        assert_eq!(descriptor.line_number().get(), 0);
        assert_eq!(descriptor.content_range().offset().get(), FOUR_GIB - 2);
        assert_eq!(descriptor.content_range().end().get(), FOUR_GIB);
        assert_eq!(descriptor.physical_range().offset().get(), FOUR_GIB - 2);
        assert_eq!(descriptor.physical_range().end().get(), FOUR_GIB + 2);
        assert_eq!(descriptor.terminator(), LineTerminator::CrLf);
        assert!(
            cursor
                .visit_next_line(|_| Ok::<(), Infallible>(()))
                .unwrap()
                .is_none()
        );
    }
}
