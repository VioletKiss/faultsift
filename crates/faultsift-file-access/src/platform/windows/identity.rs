#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowsFileIdentity {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

pub(crate) fn identity_from_file(file: &File) -> io::Result<WindowsFileIdentity> {
    let mut information = FILE_ID_INFO::default();
    let buffer_size = u32::try_from(size_of::<FILE_ID_INFO>())
        .expect("FILE_ID_INFO size must fit in the Win32 u32 buffer length");

    // SAFETY: `file` is borrowed for the whole call, so its raw handle remains
    // valid. `information` is an initialized, aligned, writable FILE_ID_INFO,
    // and `buffer_size` is its exact checked size. The result is read only when
    // the API reports success.
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            std::ptr::addr_of_mut!(information).cast::<c_void>(),
            buffer_size,
        )
    };

    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(WindowsFileIdentity {
        volume_serial_number: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}
