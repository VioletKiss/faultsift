//! Bounded, byte-oriented physical-line streaming over file snapshots.
//!
//! This crate recognizes only LF and CRLF physical terminators. It performs no
//! decoding and deliberately owns no parser, search, index, or UI semantics.

mod cursor;
mod error;
mod options;
mod scanner;
mod types;

pub use cursor::PhysicalLineCursor;
pub use error::{CursorFailure, LineAccessError, LineAccessResult, VisitLineError};
pub use options::ScanOptions;
pub use types::{CursorState, LineContentChunk, LineDescriptor, LineNumber, LineTerminator};
