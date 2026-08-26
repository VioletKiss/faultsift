use std::fs::File;
use std::io;

use crate::{ByteLength, ByteOffset, FileAccessError, FileAccessResult};

pub(crate) fn read_exact_at(
    file: &File,
    offset: ByteOffset,
    buffer: &mut [u8],
) -> FileAccessResult<()> {
    let expected = usize_to_byte_length(buffer.len())?;
    let mut filled = 0_usize;

    while filled < buffer.len() {
        let filled_length = usize_to_byte_length(filled)?;
        let current = offset.get().checked_add(filled_length.get()).ok_or(
            FileAccessError::RangeOverflow {
                offset,
                length: expected,
            },
        )?;
        let current_offset = ByteOffset::new(current);

        match platform_read_at(file, &mut buffer[filled..], current) {
            Ok(0) => {
                return Err(FileAccessError::UnexpectedEof {
                    offset,
                    expected,
                    actual: filled_length,
                });
            }
            Ok(read) => {
                filled += read;
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(FileAccessError::ReadFailed {
                    offset: current_offset,
                    source,
                });
            }
        }
    }

    Ok(())
}

fn usize_to_byte_length(value: usize) -> FileAccessResult<ByteLength> {
    let value = u64::try_from(value).map_err(|_| FileAccessError::LengthNotRepresentable {
        length: ByteLength::new(u64::MAX),
    })?;
    Ok(ByteLength::new(value))
}

#[cfg(target_os = "linux")]
fn platform_read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, offset)
}

#[cfg(windows)]
fn platform_read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    // Windows updates the handle cursor, but every call supplies its own
    // explicit offset. Correctness never observes or depends on that cursor,
    // and this layer does not use Seek or a seek-position mutex.
    file.seek_read(buffer, offset)
}
