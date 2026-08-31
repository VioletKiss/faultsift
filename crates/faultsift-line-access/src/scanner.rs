use std::sync::Arc;

use faultsift_file_access::{ByteLength, ByteOffset, ByteRange, FileAccessError, FileSnapshot};

use crate::types::checked_range;
use crate::{LineAccessError, LineContentChunk, LineTerminator, ScanOptions};

const CR_BYTE: [u8; 1] = *b"\r";

pub(crate) struct ScannedLine {
    pub(crate) content_range: ByteRange,
    pub(crate) physical_range: ByteRange,
    pub(crate) terminator: LineTerminator,
}

pub(crate) enum ScanError<E> {
    Visitor(E),
    FileAccess(FileAccessError),
    Scanner(LineAccessError),
}

/// Single source of truth for bounded physical-newline recognition.
///
/// This scanner is crate-internal so later in-crate consumers can reuse the
/// exact LF/CRLF state machine without exposing premature index APIs.
pub(crate) struct ByteScanner {
    snapshot: Arc<FileSnapshot>,
    buffer: Box<[u8]>,
    buffer_start: u64,
    buffer_len: usize,
    buffer_pos: usize,
    buffer_end_reported: bool,
    captured_length: ByteLength,
}

impl ByteScanner {
    pub(crate) fn new(
        snapshot: Arc<FileSnapshot>,
        options: ScanOptions,
    ) -> Result<Self, LineAccessError> {
        Self::new_at(snapshot, options, ByteOffset::new(0))
    }

    /// Starts the shared scanner at an exact known physical-line boundary.
    pub(crate) fn new_at(
        snapshot: Arc<FileSnapshot>,
        options: ScanOptions,
        start: ByteOffset,
    ) -> Result<Self, LineAccessError> {
        let captured_length = snapshot.len();
        if start.get() > captured_length.get() {
            return Err(LineAccessError::FileAccess(FileAccessError::OutOfBounds {
                offset: start,
                length: ByteLength::new(0),
                snapshot_length: captured_length,
            }));
        }
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(options.scan_chunk_usize())
            .map_err(|source| LineAccessError::ScanBufferAllocationFailed {
                requested: options.scan_chunk_bytes(),
                source,
            })?;
        buffer.resize(options.scan_chunk_usize(), 0);

        Ok(Self {
            snapshot,
            buffer: buffer.into_boxed_slice(),
            buffer_start: start.get(),
            buffer_len: 0,
            buffer_pos: 0,
            buffer_end_reported: false,
            captured_length,
        })
    }

    pub(crate) const fn captured_length(&self) -> ByteLength {
        self.captured_length
    }

    pub(crate) fn snapshot(&self) -> &Arc<FileSnapshot> {
        &self.snapshot
    }

    #[cfg(test)]
    pub(crate) fn install_test_window(
        &mut self,
        start: u64,
        bytes: &[u8],
        captured_length: ByteLength,
    ) {
        assert!(bytes.len() <= self.buffer.len());
        self.buffer[..bytes.len()].copy_from_slice(bytes);
        self.buffer_start = start;
        self.buffer_len = bytes.len();
        self.buffer_pos = 0;
        self.buffer_end_reported = false;
        self.captured_length = captured_length;
    }

    pub(crate) fn scan_next_line<E>(
        &mut self,
        visitor: &mut impl FnMut(LineContentChunk<'_>) -> Result<(), E>,
    ) -> Result<Option<ScannedLine>, ScanError<E>> {
        self.scan_next_line_with_chunk_boundaries(visitor, &mut |_| Ok(()))
    }

    pub(crate) fn scan_next_line_with_chunk_boundaries<E>(
        &mut self,
        visitor: &mut impl FnMut(LineContentChunk<'_>) -> Result<(), E>,
        on_chunk_boundary: &mut impl FnMut(u64) -> Result<(), LineAccessError>,
    ) -> Result<Option<ScannedLine>, ScanError<E>> {
        let line_start = self.position().map_err(ScanError::Scanner)?;
        if line_start == self.captured_length.get() {
            return Ok(None);
        }

        let mut content_end = line_start;
        let mut pending_cr = None;

        loop {
            if !self
                .ensure_data(on_chunk_boundary)
                .map_err(|error| match error {
                    LineAccessError::FileAccess(source) => ScanError::FileAccess(source),
                    other => ScanError::Scanner(other),
                })?
            {
                if let Some(cr_offset) = pending_cr.take() {
                    Self::emit(visitor, cr_offset, &CR_BYTE)?;
                    content_end = cr_offset.checked_add(1).ok_or_else(|| {
                        ScanError::Scanner(LineAccessError::CoordinateOverflow {
                            offset: ByteOffset::new(cr_offset),
                            length: ByteLength::new(1),
                        })
                    })?;
                }

                return Ok(Some(Self::finish_line(
                    line_start,
                    content_end,
                    self.captured_length.get(),
                    LineTerminator::None,
                )?));
            }

            if let Some(cr_offset) = pending_cr.take() {
                if self.buffer[self.buffer_pos] == b'\n' {
                    let lf_offset = self.position().map_err(ScanError::Scanner)?;
                    self.buffer_pos += 1;
                    let physical_end = lf_offset.checked_add(1).ok_or_else(|| {
                        ScanError::Scanner(LineAccessError::CoordinateOverflow {
                            offset: ByteOffset::new(lf_offset),
                            length: ByteLength::new(1),
                        })
                    })?;
                    return Ok(Some(Self::finish_line(
                        line_start,
                        cr_offset,
                        physical_end,
                        LineTerminator::CrLf,
                    )?));
                }

                Self::emit(visitor, cr_offset, &CR_BYTE)?;
                content_end = cr_offset.checked_add(1).ok_or_else(|| {
                    ScanError::Scanner(LineAccessError::CoordinateOverflow {
                        offset: ByteOffset::new(cr_offset),
                        length: ByteLength::new(1),
                    })
                })?;
            }

            let chunk_start = self.buffer_pos;
            let remaining = &self.buffer[chunk_start..self.buffer_len];
            if let Some(relative_lf) = remaining.iter().position(|byte| *byte == b'\n') {
                let lf_index = chunk_start + relative_lf;
                let lf_offset =
                    self.buffer_start
                        .checked_add(lf_index as u64)
                        .ok_or_else(|| {
                            ScanError::Scanner(LineAccessError::CoordinateOverflow {
                                offset: ByteOffset::new(self.buffer_start),
                                length: ByteLength::new(lf_index as u64),
                            })
                        })?;

                let (content_index_end, terminator) =
                    if lf_index > chunk_start && self.buffer[lf_index - 1] == b'\r' {
                        (lf_index - 1, LineTerminator::CrLf)
                    } else {
                        (lf_index, LineTerminator::Lf)
                    };

                if content_index_end > chunk_start {
                    let absolute_start = self
                        .buffer_start
                        .checked_add(chunk_start as u64)
                        .ok_or_else(|| {
                            ScanError::Scanner(LineAccessError::CoordinateOverflow {
                                offset: ByteOffset::new(self.buffer_start),
                                length: ByteLength::new(chunk_start as u64),
                            })
                        })?;
                    Self::emit(
                        visitor,
                        absolute_start,
                        &self.buffer[chunk_start..content_index_end],
                    )?;
                }

                content_end = match terminator {
                    LineTerminator::CrLf => lf_offset.checked_sub(1).ok_or_else(|| {
                        ScanError::Scanner(LineAccessError::CoordinateOverflow {
                            offset: ByteOffset::new(lf_offset),
                            length: ByteLength::new(1),
                        })
                    })?,
                    LineTerminator::Lf => lf_offset,
                    LineTerminator::None => unreachable!("LF scan cannot produce no terminator"),
                };
                self.buffer_pos = lf_index + 1;
                let physical_end = lf_offset.checked_add(1).ok_or_else(|| {
                    ScanError::Scanner(LineAccessError::CoordinateOverflow {
                        offset: ByteOffset::new(lf_offset),
                        length: ByteLength::new(1),
                    })
                })?;

                return Ok(Some(Self::finish_line(
                    line_start,
                    content_end,
                    physical_end,
                    terminator,
                )?));
            }

            let chunk_end = self.buffer_len;
            let has_trailing_cr = self.buffer[chunk_end - 1] == b'\r';
            let content_index_end = if has_trailing_cr {
                chunk_end - 1
            } else {
                chunk_end
            };

            if content_index_end > chunk_start {
                let absolute_start = self
                    .buffer_start
                    .checked_add(chunk_start as u64)
                    .ok_or_else(|| {
                        ScanError::Scanner(LineAccessError::CoordinateOverflow {
                            offset: ByteOffset::new(self.buffer_start),
                            length: ByteLength::new(chunk_start as u64),
                        })
                    })?;
                Self::emit(
                    visitor,
                    absolute_start,
                    &self.buffer[chunk_start..content_index_end],
                )?;
                content_end = self
                    .buffer_start
                    .checked_add(content_index_end as u64)
                    .ok_or_else(|| {
                        ScanError::Scanner(LineAccessError::CoordinateOverflow {
                            offset: ByteOffset::new(self.buffer_start),
                            length: ByteLength::new(content_index_end as u64),
                        })
                    })?;
            }

            if has_trailing_cr {
                pending_cr = Some(
                    self.buffer_start
                        .checked_add((chunk_end - 1) as u64)
                        .ok_or_else(|| {
                            ScanError::Scanner(LineAccessError::CoordinateOverflow {
                                offset: ByteOffset::new(self.buffer_start),
                                length: ByteLength::new((chunk_end - 1) as u64),
                            })
                        })?,
                );
            }
            self.buffer_pos = chunk_end;
        }
    }

    pub(crate) fn report_consumed_chunk_boundary(
        &mut self,
        on_chunk_boundary: &mut impl FnMut(u64) -> Result<(), LineAccessError>,
    ) -> Result<(), LineAccessError> {
        if self.buffer_len == 0 || self.buffer_pos != self.buffer_len || self.buffer_end_reported {
            return Ok(());
        }

        let bytes_scanned = self.position()?;
        on_chunk_boundary(bytes_scanned)?;
        self.buffer_end_reported = true;
        Ok(())
    }

    fn position(&self) -> Result<u64, LineAccessError> {
        self.buffer_start.checked_add(self.buffer_pos as u64).ok_or(
            LineAccessError::CoordinateOverflow {
                offset: ByteOffset::new(self.buffer_start),
                length: ByteLength::new(self.buffer_pos as u64),
            },
        )
    }

    fn ensure_data(
        &mut self,
        on_chunk_boundary: &mut impl FnMut(u64) -> Result<(), LineAccessError>,
    ) -> Result<bool, LineAccessError> {
        if self.buffer_pos < self.buffer_len {
            return Ok(true);
        }

        let offset = self.position()?;
        if offset == self.captured_length.get() {
            return Ok(false);
        }

        self.report_consumed_chunk_boundary(on_chunk_boundary)?;

        let read = self
            .snapshot
            .read_at(ByteOffset::new(offset), &mut self.buffer)
            .map_err(LineAccessError::FileAccess)?;
        if read == 0 {
            return Err(LineAccessError::UnexpectedScannerEof {
                offset: ByteOffset::new(offset),
                snapshot_length: self.captured_length,
            });
        }
        self.buffer_start = offset;
        self.buffer_len = read;
        self.buffer_pos = 0;
        self.buffer_end_reported = false;
        Ok(true)
    }

    fn emit<E>(
        visitor: &mut impl FnMut(LineContentChunk<'_>) -> Result<(), E>,
        start: u64,
        bytes: &[u8],
    ) -> Result<(), ScanError<E>> {
        if bytes.is_empty() {
            return Ok(());
        }
        let end = start.checked_add(bytes.len() as u64).ok_or_else(|| {
            ScanError::Scanner(LineAccessError::CoordinateOverflow {
                offset: ByteOffset::new(start),
                length: ByteLength::new(bytes.len() as u64),
            })
        })?;
        let range = checked_range(start, end).map_err(ScanError::Scanner)?;
        visitor(LineContentChunk::new(range, bytes)).map_err(ScanError::Visitor)
    }

    fn finish_line<E>(
        start: u64,
        content_end: u64,
        physical_end: u64,
        terminator: LineTerminator,
    ) -> Result<ScannedLine, ScanError<E>> {
        let content_range = checked_range(start, content_end).map_err(ScanError::Scanner)?;
        let physical_range = checked_range(start, physical_end).map_err(ScanError::Scanner)?;
        Ok(ScannedLine {
            content_range,
            physical_range,
            terminator,
        })
    }
}
