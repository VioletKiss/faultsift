use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

#[cfg(windows)]
use faultsift_file_access::MappingFallbackReason;
use faultsift_file_access::{
    BackendKind, ByteLength, ByteOffset, ByteRange, FileAccessOptions, FileSnapshot,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

pub struct TestFile {
    path: PathBuf,
}

impl TestFile {
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let (fixture, mut file) = Self::create()?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(fixture)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(bytes)?;
        file.flush()
    }

    pub fn truncate(&self, length: u64) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(&self.path)?
            .set_len(length)
    }

    pub fn overwrite(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        file.write_all(bytes)?;
        file.flush()
    }

    pub fn replace(&self, bytes: &[u8]) -> io::Result<()> {
        let replacement = unique_path("replacement");
        let mut replacement_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&replacement)?;
        replacement_file.write_all(bytes)?;
        replacement_file.flush()?;
        drop(replacement_file);

        fs::remove_file(&self.path)?;
        if let Err(error) = fs::rename(&replacement, &self.path) {
            let _ = fs::remove_file(&replacement);
            return Err(error);
        }
        Ok(())
    }

    pub fn remove(&self) -> io::Result<()> {
        fs::remove_file(&self.path)
    }

    pub fn set_distinct_modified_time(&self) -> io::Result<()> {
        let file = OpenOptions::new().write(true).open(&self.path)?;
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        file.set_times(FileTimes::new().set_modified(modified))
    }

    fn create() -> io::Result<(Self, File)> {
        loop {
            let path = unique_path("fixture");
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((Self { path }, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn options(max_view_bytes: u64) -> FileAccessOptions {
    FileAccessOptions::new(ByteLength::new(max_view_bytes)).unwrap()
}

pub fn range(offset: u64, length: u64) -> ByteRange {
    ByteRange::new(ByteOffset::new(offset), ByteLength::new(length)).unwrap()
}

pub fn open_buffered_snapshot(path: &Path, options: FileAccessOptions) -> FileSnapshot {
    #[cfg(windows)]
    let existing_writer = OpenOptions::new().write(true).open(path).unwrap();

    let snapshot = FileSnapshot::open(path, options).unwrap();
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);

    #[cfg(windows)]
    {
        assert_eq!(
            snapshot.diagnostics().mapping_fallback_reason(),
            Some(MappingFallbackReason::IncompatibleWriter)
        );
        drop(existing_writer);
    }

    snapshot
}

pub fn unique_path(kind: &str) -> PathBuf {
    let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "faultsift-fs003-{kind}-{}-{id}.tmp",
        std::process::id()
    ))
}
