use crate::{FileAccessError, FileAccessResult};

/// Absolute byte offset from the start of a file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteOffset(u64);

impl ByteOffset {
    /// Creates a byte offset.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying coordinate.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for ByteOffset {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

/// Number of bytes in a file range or buffer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteLength(u64);

impl ByteLength {
    /// Creates a byte length.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying length.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn try_to_usize(self) -> FileAccessResult<usize> {
        usize::try_from(self.0)
            .map_err(|_| FileAccessError::LengthNotRepresentable { length: self })
    }
}

impl From<u64> for ByteLength {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

/// Checked half-open byte range `[offset, end)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ByteRange {
    offset: ByteOffset,
    length: ByteLength,
    end: ByteOffset,
}

impl ByteRange {
    /// Creates a range and rejects `offset + length` overflow.
    pub fn new(offset: ByteOffset, length: ByteLength) -> FileAccessResult<Self> {
        let end = offset
            .get()
            .checked_add(length.get())
            .ok_or(FileAccessError::RangeOverflow { offset, length })?;

        Ok(Self {
            offset,
            length,
            end: ByteOffset::new(end),
        })
    }

    /// Returns the first byte coordinate.
    #[must_use]
    pub const fn offset(self) -> ByteOffset {
        self.offset
    }

    /// Returns the number of bytes in the range.
    #[must_use]
    pub const fn length(self) -> ByteLength {
        self.length
    }

    /// Returns the exclusive end coordinate.
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }

    /// Returns whether this range contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.length.get() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_uses_half_open_coordinates() {
        let range = ByteRange::new(ByteOffset::new(7), ByteLength::new(5)).unwrap();

        assert_eq!(range.offset().get(), 7);
        assert_eq!(range.length().get(), 5);
        assert_eq!(range.end().get(), 12);
        assert!(!range.is_empty());
    }

    #[test]
    fn range_rejects_u64_overflow() {
        let error = ByteRange::new(ByteOffset::new(u64::MAX), ByteLength::new(1)).unwrap_err();

        assert!(matches!(error, FileAccessError::RangeOverflow { .. }));
    }
}
