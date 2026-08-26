//! Backend-neutral, bounded byte access for large local files.
//!
//! This crate deliberately understands bytes only. Text decoding, line
//! boundaries, parsing, search, and desktop integration belong to later
//! layers.

#[cfg(not(target_pointer_width = "64"))]
compile_error!("faultsift-file-access supports only 64-bit targets");

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("faultsift-file-access currently supports Windows and Linux targets");

mod buffered;
mod diagnostics;
mod error;
mod identity;
mod lifecycle;
mod options;
mod platform;
mod range;
mod snapshot;

pub use diagnostics::{BackendKind, FileAccessDiagnostics, MappingFallbackReason};
pub use error::{FileAccessError, FileAccessResult};
pub use identity::FileIdentity;
pub use lifecycle::{SnapshotState, SnapshotValidation, StaleReason, ValidationTarget};
pub use options::FileAccessOptions;
pub use range::{ByteLength, ByteOffset, ByteRange};
pub use snapshot::{FileSnapshot, RangeView, SnapshotGeneration};
