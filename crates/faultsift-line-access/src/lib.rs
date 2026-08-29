//! Bounded, byte-oriented physical-line streaming over file snapshots.
//!
//! This crate recognizes only LF and CRLF physical terminators. It performs no
//! decoding and deliberately owns no parser, search, or UI semantics. Its
//! ready-only index build retains sparse coordinates but no raw line content.

mod cursor;
mod error;
mod index;
mod options;
mod scanner;
mod types;

pub use cursor::PhysicalLineCursor;
pub use error::{CursorFailure, LineAccessError, LineAccessResult, VisitLineError};
pub use index::{BuildControl, BuildProgress, LineIndex};
pub use options::{LineIndexOptions, ScanOptions};
pub use types::{CursorState, LineContentChunk, LineDescriptor, LineNumber, LineTerminator};
