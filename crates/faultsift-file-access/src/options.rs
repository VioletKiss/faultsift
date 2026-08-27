use crate::{ByteLength, FileAccessError, FileAccessResult};

/// Conservative default upper bound for one live [`crate::RangeView`].
///
/// One MiB keeps buffered views and bounded concurrency inexpensive while
/// allowing sequential consumers to amortize positioned-read overhead. It is
/// a resource guard, not a throughput promise, and callers may select another
/// non-zero bound with [`FileAccessOptions::new`].
pub const DEFAULT_MAX_VIEW_BYTES: ByteLength = ByteLength::new(1024 * 1024);

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

impl Default for FileAccessOptions {
    fn default() -> Self {
        Self {
            max_view_bytes: DEFAULT_MAX_VIEW_BYTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_require_a_non_zero_view_bound_and_have_a_named_default() {
        let error = FileAccessOptions::new(ByteLength::new(0)).unwrap_err();
        assert!(matches!(error, FileAccessError::InvalidMaxViewBytes { .. }));

        let options = FileAccessOptions::new(ByteLength::new(4096)).unwrap();
        assert_eq!(options.max_view_bytes().get(), 4096);
        assert_eq!(
            FileAccessOptions::default().max_view_bytes(),
            DEFAULT_MAX_VIEW_BYTES
        );
    }
}
