use faultsift_file_access::ByteLength;

use crate::{LineAccessError, LineAccessResult};

/// Explicit resource configuration for physical-line scanning.
///
/// The chunk size bounds the cursor's reusable scan buffer. It is not a line
/// length limit, and this type intentionally has no `Default` implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanOptions {
    scan_chunk_bytes: ByteLength,
    scan_chunk_usize: usize,
}

impl ScanOptions {
    /// Creates scanner options with an explicit non-zero chunk size.
    pub fn new(scan_chunk_bytes: ByteLength) -> LineAccessResult<Self> {
        if scan_chunk_bytes.get() == 0 {
            return Err(LineAccessError::InvalidScanChunkBytes {
                value: scan_chunk_bytes,
            });
        }

        let scan_chunk_usize = usize::try_from(scan_chunk_bytes.get()).map_err(|_| {
            LineAccessError::ScanChunkNotRepresentable {
                value: scan_chunk_bytes,
            }
        })?;

        Ok(Self {
            scan_chunk_bytes,
            scan_chunk_usize,
        })
    }

    /// Returns the configured reusable scan-buffer size.
    #[must_use]
    pub const fn scan_chunk_bytes(self) -> ByteLength {
        self.scan_chunk_bytes
    }

    pub(crate) const fn scan_chunk_usize(self) -> usize {
        self.scan_chunk_usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_chunk_size_is_explicit_and_non_zero() {
        let error = ScanOptions::new(ByteLength::new(0)).unwrap_err();
        assert!(matches!(
            error,
            LineAccessError::InvalidScanChunkBytes { value } if value.get() == 0
        ));

        let options = ScanOptions::new(ByteLength::new(17)).unwrap();
        assert_eq!(options.scan_chunk_bytes().get(), 17);
        assert_eq!(options.scan_chunk_usize(), 17);
    }
}
