use std::fmt;
use std::fs::{File, Metadata};
use std::io;
use std::path::Path;
use std::sync::Arc;

use crate::ByteLength;

#[cfg(windows)]
use crate::platform::windows::identity::{WindowsFileIdentity, identity_from_file};

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlatformFileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
type PlatformFileIdentity = WindowsFileIdentity;

/// Opaque identity of the opened filesystem object backing a snapshot.
///
/// Equality compares platform file identity while the underlying handle stays
/// open. Platform-specific identity fields are intentionally not exposed.
#[derive(Clone)]
pub struct FileIdentity {
    file: Arc<File>,
    platform: PlatformFileIdentity,
}

impl FileIdentity {
    pub(crate) fn from_file(file: File) -> io::Result<Self> {
        let platform = platform_identity_from_file(&file)?;
        Ok(Self {
            file: Arc::new(file),
            platform,
        })
    }

    pub(crate) fn from_path(path: &Path) -> io::Result<Self> {
        Self::from_file(File::open(path)?)
    }

    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    pub(crate) fn metadata(&self) -> io::Result<Metadata> {
        self.file().metadata()
    }
}

impl fmt::Debug for FileIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FileIdentity(..)")
    }
}

impl PartialEq for FileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.platform == other.platform
    }
}

impl Eq for FileIdentity {}

#[cfg(target_os = "linux")]
fn platform_identity_from_file(file: &File) -> io::Result<PlatformFileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(PlatformFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_identity_from_file(file: &File) -> io::Result<PlatformFileIdentity> {
    identity_from_file(file)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapturedMetadata {
    pub(crate) length: ByteLength,
    pub(crate) modification: Option<ModificationStamp>,
}

impl CapturedMetadata {
    pub(crate) fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            length: ByteLength::new(metadata.len()),
            modification: modification_stamp(metadata),
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModificationStamp {
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(target_os = "linux")]
fn modification_stamp(metadata: &Metadata) -> Option<ModificationStamp> {
    use std::os::unix::fs::MetadataExt;

    Some(ModificationStamp {
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModificationStamp {
    last_write_time: u64,
    file_attributes: u32,
}

#[cfg(windows)]
fn modification_stamp(metadata: &Metadata) -> Option<ModificationStamp> {
    use std::os::windows::fs::MetadataExt;

    let last_write_time = metadata.last_write_time();
    if last_write_time == 0 {
        return None;
    }

    Some(ModificationStamp {
        last_write_time,
        file_attributes: metadata.file_attributes(),
    })
}
