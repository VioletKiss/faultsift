use crate::{ByteLength, FileAccessError, FileAccessResult};

/// Resource limits applied when a file snapshot is opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileAccessOptions {
    max_view_bytes: ByteLength,
}

impl FileAccessOptions {
    /// Creates options with an explicit non-zero bound for each `RangeView`.
    pub fn new(max_view_bytes: ByteLength) -> FileAccessResult<Self> {
        if max_view_bytes.get() == 0 {
            return Err(FileAccessError::InvalidMaxViewBytes {
                value: max_view_bytes,
            });
        }

        Ok(Self { max_view_bytes })
    }

    /// Returns the maximum number of bytes one `RangeView` may own.
    #[must_use]
    pub const fn max_view_bytes(self) -> ByteLength {
        self.max_view_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_require_an_explicit_non_zero_view_bound() {
        let error = FileAccessOptions::new(ByteLength::new(0)).unwrap_err();
        assert!(matches!(error, FileAccessError::InvalidMaxViewBytes { .. }));

        let options = FileAccessOptions::new(ByteLength::new(4096)).unwrap();
        assert_eq!(options.max_view_bytes().get(), 4096);
    }
}
