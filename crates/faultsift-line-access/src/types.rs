use faultsift_file_access::{ByteLength, ByteOffset, ByteRange, SnapshotGeneration};

use crate::{CursorFailure, LineAccessError, LineAccessResult};

/// Zero-based physical line number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LineNumber(u64);

impl LineNumber {
    /// Creates a zero-based physical line number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the zero-based numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn checked_next(self) -> LineAccessResult<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(LineAccessError::LineNumberOverflow { line_number: self })
    }
}

/// Physical bytes that terminate one line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LineTerminator {
    /// The final physical line reaches EOF without LF.
    None,
    /// One LF byte terminates the line.
    Lf,
    /// A CR immediately followed by LF terminates the line.
    CrLf,
}

/// Immutable coordinates for one complete physical line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineDescriptor {
    generation: SnapshotGeneration,
    line_number: LineNumber,
    content_range: ByteRange,
    physical_range: ByteRange,
    terminator: LineTerminator,
}

/// Checked zero-based half-open physical-line range `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LineRange {
    start: LineNumber,
    end: LineNumber,
}

impl LineRange {
    /// Creates a half-open range and rejects reversed coordinates.
    pub fn new(start: LineNumber, end: LineNumber) -> LineAccessResult<Self> {
        if start > end {
            return Err(LineAccessError::InvalidLineRange { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the first included line number.
    #[must_use]
    pub const fn start(self) -> LineNumber {
        self.start
    }

    /// Returns the exclusive line-number boundary.
    #[must_use]
    pub const fn end(self) -> LineNumber {
        self.end
    }

    /// Returns whether this range contains no lines.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }
}

/// Immutable O(1)-sized coordinates for one physical-line range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineSpan {
    generation: SnapshotGeneration,
    line_range: LineRange,
    physical_range: ByteRange,
}

impl LineSpan {
    pub(crate) const fn from_parts(
        generation: SnapshotGeneration,
        line_range: LineRange,
        physical_range: ByteRange,
    ) -> Self {
        Self {
            generation,
            line_range,
            physical_range,
        }
    }

    /// Returns the snapshot generation that owns these coordinates.
    #[must_use]
    pub const fn generation(self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns the represented half-open physical-line range.
    #[must_use]
    pub const fn line_range(self) -> LineRange {
        self.line_range
    }

    /// Returns the contiguous physical byte range, including terminators.
    #[must_use]
    pub const fn physical_range(self) -> ByteRange {
        self.physical_range
    }
}

impl LineDescriptor {
    pub(crate) const fn from_parts(
        generation: SnapshotGeneration,
        line_number: LineNumber,
        content_range: ByteRange,
        physical_range: ByteRange,
        terminator: LineTerminator,
    ) -> Self {
        Self {
            generation,
            line_number,
            content_range,
            physical_range,
            terminator,
        }
    }

    /// Returns the snapshot generation that owns these coordinates.
    #[must_use]
    pub const fn generation(self) -> SnapshotGeneration {
        self.generation
    }

    /// Returns this line's zero-based number.
    #[must_use]
    pub const fn line_number(self) -> LineNumber {
        self.line_number
    }

    /// Returns the content-only half-open byte range.
    #[must_use]
    pub const fn content_range(self) -> ByteRange {
        self.content_range
    }

    /// Returns the half-open range including the physical terminator.
    #[must_use]
    pub const fn physical_range(self) -> ByteRange {
        self.physical_range
    }

    /// Returns the physical terminator kind.
    #[must_use]
    pub const fn terminator(self) -> LineTerminator {
        self.terminator
    }
}

/// Borrowed content exposed during one visitor callback.
///
/// The bytes cannot outlive the callback. Consumers that need longer-lived
/// evidence must copy a bounded selection or retain the absolute range.
#[derive(Clone, Copy, Debug)]
pub struct LineContentChunk<'a> {
    range: ByteRange,
    bytes: &'a [u8],
}

impl<'a> LineContentChunk<'a> {
    pub(crate) const fn new(range: ByteRange, bytes: &'a [u8]) -> Self {
        Self { range, bytes }
    }

    /// Returns this chunk's absolute byte range.
    #[must_use]
    pub const fn range(self) -> ByteRange {
        self.range
    }

    /// Returns bytes valid for the current callback only.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

/// Borrowed raw physical bytes exposed during one span-reader callback.
///
/// The bytes cannot outlive the callback and retain every LF/CRLF byte present
/// in the represented physical range.
#[derive(Clone, Copy, Debug)]
pub struct PhysicalSpanChunk<'a> {
    range: ByteRange,
    bytes: &'a [u8],
}

impl<'a> PhysicalSpanChunk<'a> {
    pub(crate) const fn new(range: ByteRange, bytes: &'a [u8]) -> Self {
        Self { range, bytes }
    }

    /// Returns this chunk's absolute byte range.
    #[must_use]
    pub const fn range(self) -> ByteRange {
        self.range
    }

    /// Returns bytes valid for the current callback only.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

/// Observable state of a single-owner physical-line cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorState {
    Active,
    Exhausted,
    Failed(CursorFailure),
}

pub(crate) fn checked_range(start: u64, end: u64) -> LineAccessResult<ByteRange> {
    let length = end
        .checked_sub(start)
        .ok_or(LineAccessError::CoordinateOverflow {
            offset: ByteOffset::new(start),
            length: ByteLength::new(end),
        })?;
    ByteRange::new(ByteOffset::new(start), ByteLength::new(length))
        .map_err(LineAccessError::FileAccess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_preserve_coordinates_beyond_four_gib() {
        let start = (1_u64 << 32) + 7;
        let range = checked_range(start, start + 19).unwrap();
        assert_eq!(range.offset().get(), start);
        assert_eq!(range.length().get(), 19);
        assert_eq!(range.end().get(), start + 19);
    }

    #[test]
    fn reversed_coordinates_are_rejected() {
        assert!(matches!(
            checked_range(9, 8),
            Err(LineAccessError::CoordinateOverflow { .. })
        ));
    }

    #[test]
    fn line_ranges_are_half_open_and_checked() {
        let range = LineRange::new(LineNumber::new(7), LineNumber::new(9)).unwrap();
        assert_eq!(range.start().get(), 7);
        assert_eq!(range.end().get(), 9);
        assert!(!range.is_empty());

        let empty = LineRange::new(LineNumber::new(9), LineNumber::new(9)).unwrap();
        assert!(empty.is_empty());
        assert!(matches!(
            LineRange::new(LineNumber::new(10), LineNumber::new(9)),
            Err(LineAccessError::InvalidLineRange { .. })
        ));
    }

    #[test]
    fn line_span_coordinates_preserve_values_beyond_four_gib() {
        let start = (1_u64 << 32) + 11;
        let physical_range = checked_range(start, start + 29).unwrap();
        let line_range = LineRange::new(LineNumber::new(3), LineNumber::new(5)).unwrap();
        let generation = faultsift_file_access::FileSnapshot::open(
            std::env::current_exe().unwrap(),
            faultsift_file_access::FileAccessOptions::default(),
        )
        .unwrap()
        .generation();
        let span = LineSpan::from_parts(generation, line_range, physical_range);

        assert_eq!(span.generation(), generation);
        assert_eq!(span.line_range(), line_range);
        assert_eq!(span.physical_range().offset().get(), start);
        assert_eq!(span.physical_range().end().get(), start + 29);
    }
}
