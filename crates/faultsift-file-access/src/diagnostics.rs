/// Internal byte-access implementation selected for a snapshot.
///
/// Callers may use this value for diagnostics and tests, but correctness must
/// not branch on the selected backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackendKind {
    /// Safe positioned reads into bounded buffers.
    Buffered,
    /// Read-only mapped access selected after Windows stability checks.
    Mapped,
}

/// Diagnostic reason that a snapshot retained positioned buffered access.
///
/// These values are support and test evidence only. Callers must not use them
/// to choose correctness behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MappingFallbackReason {
    /// Empty files are valid snapshots but cannot be mapped.
    EmptyFile,
    /// A pre-existing writer or delete-capable handle conflicted with the
    /// restrictive stability-handle share mode.
    IncompatibleWriter,
    /// A restrictive stability handle could not otherwise be established.
    StabilityHandleUnavailable,
    /// The resolved file target is on network, removable, or other unsupported
    /// storage.
    UnsupportedLocation,
    /// The resolved file target's storage location could not be established.
    UnknownLocation,
    /// The local filesystem is not one of the explicitly supported filesystems.
    UnsupportedFilesystem,
    /// The path changed between the baseline and stability-handle opens.
    FileChangedDuringSelection,
    /// The complete mapping cannot be represented as one safe Rust slice.
    MappingSizeNotRepresentable,
    /// Windows could not create the mapping object or mapped view.
    MappingCreationFailed,
}

/// Read-only diagnostics for a file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileAccessDiagnostics {
    backend: BackendKind,
    mapping_fallback_reason: Option<MappingFallbackReason>,
}

impl FileAccessDiagnostics {
    pub(crate) const fn buffered(mapping_fallback_reason: Option<MappingFallbackReason>) -> Self {
        Self {
            backend: BackendKind::Buffered,
            mapping_fallback_reason,
        }
    }

    #[cfg(windows)]
    pub(crate) const fn mapped() -> Self {
        Self {
            backend: BackendKind::Mapped,
            mapping_fallback_reason: None,
        }
    }

    /// Returns the selected implementation backend.
    #[must_use]
    pub const fn backend(self) -> BackendKind {
        self.backend
    }

    /// Returns why Windows mapping was not selected, when one was considered.
    #[must_use]
    pub const fn mapping_fallback_reason(self) -> Option<MappingFallbackReason> {
        self.mapping_fallback_reason
    }

    /// Returns whether mapping was considered and buffered fallback succeeded.
    #[must_use]
    pub const fn used_buffered_fallback(self) -> bool {
        matches!(self.backend, BackendKind::Buffered)
            && !matches!(
                self.mapping_fallback_reason,
                None | Some(MappingFallbackReason::EmptyFile)
            )
    }
}
