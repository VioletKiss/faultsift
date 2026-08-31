use std::collections::TryReserveError;
use std::error::Error;
use std::fmt;

use faultsift_file_access::{ByteLength, ByteOffset, FileAccessError, SnapshotGeneration};

use crate::{LineNumber, LineRange};

/// Result type for line-access configuration and cursor construction.
pub type LineAccessResult<T> = Result<T, LineAccessError>;

/// Stable category explaining why a cursor is terminally failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorFailure {
    Visitor,
    Read,
    Scanner,
}

/// Errors produced by line-access configuration, scanning, and cursor state.
#[derive(Debug)]
#[non_exhaustive]
pub enum LineAccessError {
    InvalidScanChunkBytes {
        value: ByteLength,
    },
    ScanChunkNotRepresentable {
        value: ByteLength,
    },
    ScanBufferAllocationFailed {
        requested: ByteLength,
        source: TryReserveError,
    },
    InvalidCheckpointBudgetBytes {
        value: ByteLength,
        minimum: ByteLength,
    },
    CheckpointCountNotRepresentable {
        count: u64,
    },
    CheckpointAllocationFailed {
        max_checkpoints: u64,
        source: TryReserveError,
    },
    CheckpointCapacityExceeded {
        capacity: u64,
        max_checkpoints: u64,
    },
    CheckpointArithmeticOverflow,
    StrideOverflow {
        current_stride: u64,
    },
    IndexBuildCancelled,
    CoordinateOverflow {
        offset: ByteOffset,
        length: ByteLength,
    },
    LineNumberOverflow {
        line_number: LineNumber,
    },
    LineNumberOutOfBounds {
        line_number: LineNumber,
        line_count: u64,
    },
    InvalidLineRange {
        start: LineNumber,
        end: LineNumber,
    },
    LineRangeOutOfBounds {
        range: LineRange,
        line_count: u64,
    },
    DescriptorGenerationMismatch {
        expected: SnapshotGeneration,
        actual: SnapshotGeneration,
    },
    SpanGenerationMismatch {
        expected: SnapshotGeneration,
        actual: SnapshotGeneration,
    },
    UnexpectedScannerEof {
        offset: ByteOffset,
        snapshot_length: ByteLength,
    },
    FileAccess(FileAccessError),
    CursorFailed {
        failure: CursorFailure,
    },
}

impl fmt::Display for LineAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScanChunkBytes { value } => {
                write!(
                    formatter,
                    "scan_chunk_bytes must be non-zero, got {}",
                    value.get()
                )
            }
            Self::ScanChunkNotRepresentable { value } => write!(
                formatter,
                "scan_chunk_bytes {} is not representable as usize",
                value.get()
            ),
            Self::ScanBufferAllocationFailed { requested, source } => write!(
                formatter,
                "failed to allocate {} scan buffer bytes: {source}",
                requested.get()
            ),
            Self::InvalidCheckpointBudgetBytes { value, minimum } => write!(
                formatter,
                "checkpoint_budget_bytes must hold at least two u64 offsets ({} bytes), got {}",
                minimum.get(),
                value.get()
            ),
            Self::CheckpointCountNotRepresentable { count } => write!(
                formatter,
                "maximum checkpoint count {count} is not representable as usize"
            ),
            Self::CheckpointAllocationFailed {
                max_checkpoints,
                source,
            } => write!(
                formatter,
                "failed to allocate storage for {max_checkpoints} checkpoints: {source}"
            ),
            Self::CheckpointCapacityExceeded {
                capacity,
                max_checkpoints,
            } => write!(
                formatter,
                "checkpoint storage capacity {capacity} exceeds configured ceiling {max_checkpoints}"
            ),
            Self::CheckpointArithmeticOverflow => {
                formatter.write_str("checkpoint resource arithmetic overflowed")
            }
            Self::StrideOverflow { current_stride } => write!(
                formatter,
                "checkpoint stride overflowed while doubling {current_stride}"
            ),
            Self::IndexBuildCancelled => formatter.write_str("line-index build was cancelled"),
            Self::CoordinateOverflow { offset, length } => write!(
                formatter,
                "line coordinate arithmetic overflowed at offset {} with value {}",
                offset.get(),
                length.get()
            ),
            Self::LineNumberOverflow { line_number } => write!(
                formatter,
                "physical line number overflowed after {}",
                line_number.get()
            ),
            Self::LineNumberOutOfBounds {
                line_number,
                line_count,
            } => write!(
                formatter,
                "physical line number {} is outside line count {line_count}",
                line_number.get()
            ),
            Self::InvalidLineRange { start, end } => write!(
                formatter,
                "physical line range start {} exceeds end {}",
                start.get(),
                end.get()
            ),
            Self::LineRangeOutOfBounds { range, line_count } => write!(
                formatter,
                "physical line range [{}, {}) exceeds line count {line_count}",
                range.start().get(),
                range.end().get()
            ),
            Self::DescriptorGenerationMismatch { expected, actual } => write!(
                formatter,
                "line descriptor generation {actual:?} does not match index generation {expected:?}"
            ),
            Self::SpanGenerationMismatch { expected, actual } => write!(
                formatter,
                "line span generation {actual:?} does not match index generation {expected:?}"
            ),
            Self::UnexpectedScannerEof {
                offset,
                snapshot_length,
            } => write!(
                formatter,
                "scanner read zero bytes at offset {} before captured length {}",
                offset.get(),
                snapshot_length.get()
            ),
            Self::FileAccess(source) => write!(formatter, "file access failed: {source}"),
            Self::CursorFailed { failure } => {
                write!(
                    formatter,
                    "physical-line cursor is terminally failed: {failure}"
                )
            }
        }
    }
}

impl Error for LineAccessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ScanBufferAllocationFailed { source, .. }
            | Self::CheckpointAllocationFailed { source, .. } => Some(source),
            Self::FileAccess(source) => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for CursorFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Visitor => "visitor error",
            Self::Read => "byte-read error",
            Self::Scanner => "scanner invariant or coordinate error",
        })
    }
}

/// Error from one content-bearing line visit.
///
/// Visitor errors remain in their original caller-defined type and are never
/// converted to text.
#[derive(Debug)]
pub enum VisitLineError<E> {
    LineAccess(LineAccessError),
    Visitor(E),
}

impl<E> From<LineAccessError> for VisitLineError<E> {
    fn from(error: LineAccessError) -> Self {
        Self::LineAccess(error)
    }
}

impl<E: fmt::Display> fmt::Display for VisitLineError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineAccess(error) => error.fmt(formatter),
            Self::Visitor(error) => write!(formatter, "line content visitor failed: {error}"),
        }
    }
}

impl<E: Error + 'static> Error for VisitLineError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LineAccess(error) => Some(error),
            Self::Visitor(error) => Some(error),
        }
    }
}

/// Error from one independent ready-index bounded byte visit.
///
/// Visitor errors remain in their original caller-defined type and do not
/// poison the immutable index or its snapshot lifecycle.
#[derive(Debug)]
pub enum VisitBytesError<E> {
    LineAccess(LineAccessError),
    Visitor(E),
}

impl<E> From<LineAccessError> for VisitBytesError<E> {
    fn from(error: LineAccessError) -> Self {
        Self::LineAccess(error)
    }
}

impl<E: fmt::Display> fmt::Display for VisitBytesError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineAccess(error) => error.fmt(formatter),
            Self::Visitor(error) => write!(formatter, "bounded byte visitor failed: {error}"),
        }
    }
}

impl<E: Error + 'static> Error for VisitBytesError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LineAccess(error) => Some(error),
            Self::Visitor(error) => Some(error),
        }
    }
}
