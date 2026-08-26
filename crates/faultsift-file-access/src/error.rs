use std::collections::TryReserveError;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::{ByteLength, ByteOffset};

/// Result type used by the byte-access layer.
pub type FileAccessResult<T> = Result<T, FileAccessError>;

/// Stable error categories produced by bounded byte access.
#[derive(Debug)]
#[non_exhaustive]
pub enum FileAccessError {
    /// The configured per-view bound was zero.
    InvalidMaxViewBytes { value: ByteLength },
    /// The file could not be opened for reading.
    OpenFailed { path: PathBuf, source: io::Error },
    /// Metadata for an opened file could not be read.
    MetadataFailed { path: PathBuf, source: io::Error },
    /// The opened object was not a seekable regular file.
    UnsupportedFileType { path: PathBuf },
    /// The process-local generation counter was exhausted.
    GenerationExhausted,
    /// Adding an offset and length overflowed `u64`.
    RangeOverflow {
        offset: ByteOffset,
        length: ByteLength,
    },
    /// A requested range was outside the captured snapshot boundary.
    OutOfBounds {
        offset: ByteOffset,
        length: ByteLength,
        snapshot_length: ByteLength,
    },
    /// A view exceeded its configured allocation bound.
    RangeTooLarge {
        requested: ByteLength,
        maximum: ByteLength,
    },
    /// A byte offset cannot be represented by the supported OS file APIs.
    OffsetNotRepresentable { offset: ByteOffset },
    /// A byte length cannot be represented as an in-process buffer size.
    LengthNotRepresentable { length: ByteLength },
    /// The bounded view buffer could not be allocated.
    AllocationFailed {
        requested: ByteLength,
        source: TryReserveError,
    },
    /// The source reached EOF before the captured snapshot boundary.
    UnexpectedEof {
        offset: ByteOffset,
        expected: ByteLength,
        actual: ByteLength,
    },
    /// A positioned operating-system read failed.
    ReadFailed {
        offset: ByteOffset,
        source: io::Error,
    },
}

impl fmt::Display for FileAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaxViewBytes { value } => {
                write!(
                    formatter,
                    "max_view_bytes must be non-zero, got {}",
                    value.get()
                )
            }
            Self::OpenFailed { path, source } => {
                write!(formatter, "failed to open {}: {source}", path.display())
            }
            Self::MetadataFailed { path, source } => {
                write!(
                    formatter,
                    "failed to read metadata for {}: {source}",
                    path.display()
                )
            }
            Self::UnsupportedFileType { path } => {
                write!(formatter, "{} is not a regular file", path.display())
            }
            Self::GenerationExhausted => formatter.write_str("snapshot generation space exhausted"),
            Self::RangeOverflow { offset, length } => write!(
                formatter,
                "byte range overflow: offset {} + length {}",
                offset.get(),
                length.get()
            ),
            Self::OutOfBounds {
                offset,
                length,
                snapshot_length,
            } => write!(
                formatter,
                "byte range at offset {} with length {} exceeds snapshot length {}",
                offset.get(),
                length.get(),
                snapshot_length.get()
            ),
            Self::RangeTooLarge { requested, maximum } => write!(
                formatter,
                "view length {} exceeds max_view_bytes {}",
                requested.get(),
                maximum.get()
            ),
            Self::OffsetNotRepresentable { offset } => {
                write!(
                    formatter,
                    "byte offset {} is not representable",
                    offset.get()
                )
            }
            Self::LengthNotRepresentable { length } => {
                write!(
                    formatter,
                    "byte length {} is not representable",
                    length.get()
                )
            }
            Self::AllocationFailed { requested, source } => {
                write!(
                    formatter,
                    "failed to allocate {} view bytes: {source}",
                    requested.get()
                )
            }
            Self::UnexpectedEof {
                offset,
                expected,
                actual,
            } => write!(
                formatter,
                "unexpected EOF at offset {}: expected {} bytes, read {}",
                offset.get(),
                expected.get(),
                actual.get()
            ),
            Self::ReadFailed { offset, source } => {
                write!(
                    formatter,
                    "positioned read failed at offset {}: {source}",
                    offset.get()
                )
            }
        }
    }
}

impl Error for FileAccessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OpenFailed { source, .. }
            | Self::MetadataFailed { source, .. }
            | Self::ReadFailed { source, .. } => Some(source),
            Self::AllocationFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}
