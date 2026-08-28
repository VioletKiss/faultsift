use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use faultsift_file_access::{ByteLength, FileAccessOptions, FileSnapshot};
use faultsift_line_access::{PhysicalLineCursor, ScanOptions};

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

    pub fn streamed_line(content_bytes: u64) -> io::Result<Self> {
        let (fixture, mut file) = Self::create()?;
        let block = [0x80_u8; 4096];
        let mut remaining = content_bytes;
        while remaining != 0 {
            let write_len = usize::try_from(remaining.min(block.len() as u64)).unwrap();
            file.write_all(&block[..write_len])?;
            remaining -= write_len as u64;
        }
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(fixture)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write_handle(&self) -> io::Result<File> {
        OpenOptions::new().write(true).open(&self.path)
    }

    fn create() -> io::Result<(Self, File)> {
        loop {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "faultsift-line-access-{}-{id}.log",
                std::process::id()
            ));
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

pub fn snapshot(fixture: &TestFile) -> Arc<FileSnapshot> {
    Arc::new(FileSnapshot::open(fixture.path(), FileAccessOptions::default()).unwrap())
}

pub fn cursor(snapshot: Arc<FileSnapshot>, scan_chunk_bytes: u64) -> PhysicalLineCursor {
    PhysicalLineCursor::new(
        snapshot,
        ScanOptions::new(ByteLength::new(scan_chunk_bytes)).unwrap(),
    )
    .unwrap()
}
