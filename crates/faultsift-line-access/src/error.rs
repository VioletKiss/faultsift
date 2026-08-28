use std::collections::TryReserveError;
use std::error::Error;
use std::fmt;

use faultsift_file_access::{ByteLength, ByteOffset, FileAccessError};

use crate::LineNumber;

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
    CoordinateOverflow {
        offset: ByteOffset,
        length: ByteLength,
    },
    LineNumberOverflow {
        line_number: LineNumber,
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
            Self::ScanBufferAllocationFailed { source, .. } => Some(source),
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
