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

/// Explicit resource configuration for one complete line-index build.
///
/// The checkpoint byte budget is a logical ceiling for stored `u64` offsets.
/// The scan chunk bounds one reusable scanner buffer and is not a line-length
/// limit. This type intentionally has no `Default` implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineIndexOptions {
    checkpoint_budget_bytes: ByteLength,
    max_checkpoints: usize,
    scan_options: ScanOptions,
}

impl LineIndexOptions {
    /// Validates explicit checkpoint and scanner resource limits.
    pub fn new(
        checkpoint_budget_bytes: ByteLength,
        scan_chunk_bytes: ByteLength,
    ) -> LineAccessResult<Self> {
        let checkpoint_bytes = u64::try_from(std::mem::size_of::<u64>())
            .map_err(|_| LineAccessError::CheckpointArithmeticOverflow)?;
        let minimum_budget = checkpoint_bytes
            .checked_mul(2)
            .ok_or(LineAccessError::CheckpointArithmeticOverflow)?;
        let max_checkpoints_u64 = checkpoint_budget_bytes
            .get()
            .checked_div(checkpoint_bytes)
            .ok_or(LineAccessError::CheckpointArithmeticOverflow)?;
        if max_checkpoints_u64 < 2 {
            return Err(LineAccessError::InvalidCheckpointBudgetBytes {
                value: checkpoint_budget_bytes,
                minimum: ByteLength::new(minimum_budget),
            });
        }
        let max_checkpoints = usize::try_from(max_checkpoints_u64).map_err(|_| {
            LineAccessError::CheckpointCountNotRepresentable {
                count: max_checkpoints_u64,
            }
        })?;
        let scan_options = ScanOptions::new(scan_chunk_bytes)?;

        Ok(Self {
            checkpoint_budget_bytes,
            max_checkpoints,
            scan_options,
        })
    }

    /// Returns the configured logical checkpoint-storage budget.
    #[must_use]
    pub const fn checkpoint_budget_bytes(self) -> ByteLength {
        self.checkpoint_budget_bytes
    }

    /// Returns the configured reusable scan-buffer size.
    #[must_use]
    pub const fn scan_chunk_bytes(self) -> ByteLength {
        self.scan_options.scan_chunk_bytes()
    }

    pub(crate) const fn max_checkpoints(self) -> usize {
        self.max_checkpoints
    }

    pub(crate) const fn scan_options(self) -> ScanOptions {
        self.scan_options
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

    #[test]
    fn line_index_resources_are_explicit_and_checked() {
        let offset_bytes = std::mem::size_of::<u64>() as u64;
        for budget in [0, offset_bytes, offset_bytes * 2 - 1] {
            assert!(matches!(
                LineIndexOptions::new(ByteLength::new(budget), ByteLength::new(1)),
                Err(LineAccessError::InvalidCheckpointBudgetBytes { .. })
            ));
        }

        let options =
            LineIndexOptions::new(ByteLength::new(offset_bytes * 3 + 7), ByteLength::new(17))
                .unwrap();
        assert_eq!(
            options.checkpoint_budget_bytes().get(),
            offset_bytes * 3 + 7
        );
        assert_eq!(options.max_checkpoints(), 3);
        assert_eq!(options.scan_chunk_bytes().get(), 17);
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn unrepresentable_checkpoint_count_is_rejected() {
        let budget = (u64::from(u32::MAX) + 1) * std::mem::size_of::<u64>() as u64;
        assert!(matches!(
            LineIndexOptions::new(ByteLength::new(budget), ByteLength::new(1)),
            Err(LineAccessError::CheckpointCountNotRepresentable { .. })
        ));
    }
}
