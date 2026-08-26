/// Internal byte-access implementation selected for a snapshot.
///
/// Callers may use this value for diagnostics and tests, but correctness must
/// not branch on the selected backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackendKind {
    /// Safe positioned reads into bounded buffers.
    Buffered,
}

/// Read-only diagnostics for a file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileAccessDiagnostics {
    backend: BackendKind,
}

impl FileAccessDiagnostics {
    pub(crate) const fn buffered() -> Self {
        Self {
            backend: BackendKind::Buffered,
        }
    }

    /// Returns the selected implementation backend.
    #[must_use]
    pub const fn backend(self) -> BackendKind {
        self.backend
    }
}
