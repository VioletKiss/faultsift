#![allow(unsafe_code)]

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Arc;

use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_READ, GetDriveTypeW, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
    GetVolumePathNameW,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READONLY,
    UnmapViewOfFile,
};
use windows_sys::Win32::System::WindowsProgramming::{
    DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_UNKNOWN,
};

use crate::diagnostics::MappingFallbackReason;
use crate::identity::{CapturedMetadata, FileIdentity};

const FILESYSTEM_NAME_CAPACITY: usize = 261;

pub(crate) struct MappingCandidate {
    pub(crate) identity: FileIdentity,
    pub(crate) mapping: Arc<MappedFile>,
}

/// Tries to replace an already-open buffered handle with a stable mapped one.
///
/// Every failure is diagnostic-only. The caller retains the original buffered
/// identity and can therefore complete the open without exposing mapping errors.
pub(crate) fn try_create(
    path: &Path,
    expected_identity: &FileIdentity,
    expected_metadata: CapturedMetadata,
) -> Result<MappingCandidate, MappingFallbackReason> {
    let stability_file = open_stability_file(path)?;
    let metadata = stability_file
        .metadata()
        .map_err(|_| MappingFallbackReason::StabilityHandleUnavailable)?;

    if !metadata.is_file() || CapturedMetadata::from_metadata(&metadata) != expected_metadata {
        return Err(MappingFallbackReason::FileChangedDuringSelection);
    }

    let length = mapping_length(metadata.len())?;
    let stability_file = Arc::new(stability_file);
    let identity = FileIdentity::from_shared_file(Arc::clone(&stability_file))
        .map_err(|_| MappingFallbackReason::StabilityHandleUnavailable)?;

    if &identity != expected_identity {
        return Err(MappingFallbackReason::FileChangedDuringSelection);
    }

    ensure_supported_location(&stability_file)?;
    let mapping = MappedFile::create(stability_file, length)
        .map_err(|_| MappingFallbackReason::MappingCreationFailed)?;

    Ok(MappingCandidate {
        identity,
        mapping: Arc::new(mapping),
    })
}

fn open_stability_file(path: &Path) -> Result<File, MappingFallbackReason> {
    OpenOptions::new()
        .read(true)
        // Desired access is read-only and only FILE_SHARE_READ is granted.
        // Win32's mutual share check rejects existing write/delete access and
        // prevents new write/delete opens until this handle is dropped.
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                MappingFallbackReason::IncompatibleWriter
            } else {
                MappingFallbackReason::StabilityHandleUnavailable
            }
        })
}

fn mapping_length(length: u64) -> Result<usize, MappingFallbackReason> {
    let length =
        usize::try_from(length).map_err(|_| MappingFallbackReason::MappingSizeNotRepresentable)?;
    if length == 0 || length > isize::MAX as usize {
        return Err(MappingFallbackReason::MappingSizeNotRepresentable);
    }
    Ok(length)
}

fn ensure_supported_location(file: &File) -> Result<(), MappingFallbackReason> {
    let final_path = final_path(file).map_err(|_| MappingFallbackReason::UnknownLocation)?;
    let volume_path =
        volume_path(&final_path).map_err(|_| MappingFallbackReason::UnknownLocation)?;

    // SAFETY: `volume_path` is a live, NUL-terminated UTF-16 buffer returned by
    // GetVolumePathNameW. The pointer remains valid and read-only for the call.
    let drive_type = unsafe { GetDriveTypeW(volume_path.as_ptr()) };
    ensure_fixed_drive(drive_type)?;

    let filesystem = filesystem_name(file).map_err(|_| MappingFallbackReason::UnknownLocation)?;
    if !supported_filesystem(&filesystem) {
        return Err(MappingFallbackReason::UnsupportedFilesystem);
    }

    Ok(())
}

fn ensure_fixed_drive(drive_type: u32) -> Result<(), MappingFallbackReason> {
    match drive_type {
        DRIVE_FIXED => Ok(()),
        DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => Err(MappingFallbackReason::UnknownLocation),
        _ => Err(MappingFallbackReason::UnsupportedLocation),
    }
}

fn supported_filesystem(filesystem: &str) -> bool {
    filesystem.eq_ignore_ascii_case("NTFS") || filesystem.eq_ignore_ascii_case("ReFS")
}

fn final_path(file: &File) -> io::Result<Vec<u16>> {
    // SAFETY: `file` is borrowed across the call, so the raw handle is valid.
    // A null output pointer with a zero length is the documented size query.
    let required =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }

    let capacity = usize::try_from(required)
        .map_err(|_| io::Error::other("final path length is not representable"))?;
    let mut path = reserved_zeroed_utf16(capacity)?;

    // SAFETY: `file` remains borrowed and valid. `path` is an initialized,
    // writable UTF-16 buffer whose checked u32 capacity matches the argument.
    let written =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), path.as_mut_ptr(), required, 0) };
    if written == 0 {
        return Err(io::Error::last_os_error());
    }
    if written >= required {
        return Err(io::Error::other(
            "resolved path changed while mapping eligibility was checked",
        ));
    }

    path.truncate(
        usize::try_from(written)
            .map_err(|_| io::Error::other("final path length is not representable"))?,
    );
    path.push(0);
    Ok(path)
}

fn volume_path(final_path: &[u16]) -> io::Result<Vec<u16>> {
    let capacity = final_path
        .len()
        .checked_add(1)
        .ok_or_else(|| io::Error::other("volume path capacity overflow"))?;
    let capacity_u32 = u32::try_from(capacity)
        .map_err(|_| io::Error::other("volume path capacity is not representable"))?;
    let mut volume = reserved_zeroed_utf16(capacity)?;

    // SAFETY: `final_path` is NUL-terminated and immutable for the call.
    // `volume` is initialized, writable, and has the checked capacity supplied.
    let succeeded =
        unsafe { GetVolumePathNameW(final_path.as_ptr(), volume.as_mut_ptr(), capacity_u32) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(volume)
}

fn filesystem_name(file: &File) -> io::Result<String> {
    let mut filesystem = [0_u16; FILESYSTEM_NAME_CAPACITY];

    // SAFETY: `file` is borrowed for the whole call. Optional output pointers
    // are null, while `filesystem` is initialized, aligned, writable, and its
    // exact checked capacity is supplied to Win32.
    let succeeded = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    let end = filesystem
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(filesystem.len());
    String::from_utf16(&filesystem[..end])
        .map_err(|_| io::Error::other("filesystem name is not valid UTF-16"))
}

fn reserved_zeroed_utf16(length: usize) -> io::Result<Vec<u16>> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| io::Error::other("could not allocate Windows path buffer"))?;
    buffer.resize(length, 0);
    Ok(buffer)
}

pub(crate) struct MappedFile {
    base: NonNull<u8>,
    length: usize,
    _mapping_handle: OwnedHandle,
    _stability_file: Arc<File>,
}

impl MappedFile {
    fn create(stability_file: Arc<File>, length: usize) -> io::Result<Self> {
        // SAFETY: the borrowed stability handle is valid and was opened for
        // GENERIC_READ. Its restrictive share mode remains effective through
        // `stability_file`. Null security/name pointers create an unnamed,
        // non-inheritable PAGE_READONLY object for the file's current size.
        let raw_mapping = unsafe {
            CreateFileMappingW(
                stability_file.as_raw_handle(),
                std::ptr::null(),
                PAGE_READONLY,
                0,
                0,
                std::ptr::null(),
            )
        };
        if raw_mapping.is_null() {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: CreateFileMappingW returned a fresh, valid owned HANDLE. It
        // is transferred exactly once into OwnedHandle and is not used raw
        // after ownership transfer except through AsRawHandle borrows.
        let mapping_handle = unsafe { OwnedHandle::from_raw_handle(raw_mapping) };

        // SAFETY: `mapping_handle` is valid for the call and refers to a
        // PAGE_READONLY mapping. FILE_MAP_READ with zero offset/length maps the
        // complete non-empty file, whose checked `length` fits one Rust slice.
        let view = unsafe { MapViewOfFile(mapping_handle.as_raw_handle(), FILE_MAP_READ, 0, 0, 0) };
        let base = NonNull::new(view.Value.cast::<u8>()).ok_or_else(io::Error::last_os_error)?;

        Ok(Self {
            base,
            length,
            _mapping_handle: mapping_handle,
            _stability_file: stability_file,
        })
    }

    pub(crate) fn slice(&self, start: usize, end: usize) -> &[u8] {
        self.bytes()
            .get(start..end)
            .expect("mapped range was validated against the captured snapshot")
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `base` came from a successful whole-file MapViewOfFile call.
        // `length` is the unchanged mapped file length and is non-zero and no
        // larger than isize::MAX. The retained stability file handle excludes
        // write, truncate, rename, and delete access for this object's lifetime.
        // The view is PAGE_READONLY, no mutable reference is ever produced, and
        // Drop cannot unmap it while this shared borrow is alive.
        unsafe { std::slice::from_raw_parts(self.base.as_ptr().cast_const(), self.length) }
    }
}

impl fmt::Debug for MappedFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MappedFile")
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

// SAFETY: MappedFile exposes only immutable bytes from a PAGE_READONLY view.
// Its stability and mapping handles are owned and retained, and Drop cannot run
// until the final Arc reference and all borrows derived from it are gone.
unsafe impl Send for MappedFile {}

// SAFETY: The mapping has no mutable API or shared cursor. Concurrent readers
// access immutable pages while the retained handle prevents supported external
// mutations that could invalidate those pages.
unsafe impl Sync for MappedFile {}

impl Drop for MappedFile {
    fn drop(&mut self) {
        // SAFETY: `base` is the exact address returned by MapViewOfFile and has
        // not been unmapped. Drop has exclusive access and runs only after all
        // Arc owners (including RangeView backings) are gone. Handles remain
        // live through this call and are released afterward by field Drop.
        let _ = unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.base.as_ptr().cast(),
            })
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::WindowsProgramming::{DRIVE_REMOTE, DRIVE_REMOVABLE};

    #[test]
    fn only_fixed_drive_type_is_eligible() {
        assert_eq!(ensure_fixed_drive(DRIVE_FIXED), Ok(()));
        assert_eq!(
            ensure_fixed_drive(DRIVE_REMOTE),
            Err(MappingFallbackReason::UnsupportedLocation)
        );
        assert_eq!(
            ensure_fixed_drive(DRIVE_REMOVABLE),
            Err(MappingFallbackReason::UnsupportedLocation)
        );
        assert_eq!(
            ensure_fixed_drive(DRIVE_UNKNOWN),
            Err(MappingFallbackReason::UnknownLocation)
        );
    }

    #[test]
    fn only_explicitly_supported_local_filesystems_are_eligible() {
        assert!(supported_filesystem("NTFS"));
        assert!(supported_filesystem("refs"));
        assert!(!supported_filesystem("FAT32"));
        assert!(!supported_filesystem("unknown"));
    }
}
