use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use faultsift_file_access::{
    BackendKind, ByteLength, ByteOffset, ByteRange, FileAccessError, FileAccessOptions,
    FileSnapshot,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct TestFile {
    path: PathBuf,
}

impl TestFile {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let (fixture, mut file) = Self::create()?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(fixture)
    }

    fn sparse_with_boundary_bytes(boundary: u64) -> io::Result<Self> {
        let (fixture, mut file) = Self::create()?;
        prepare_sparse_file(fixture.path())?;
        file.set_len(boundary + 2)?;
        file.seek(SeekFrom::Start(boundary - 1))?;
        file.write_all(b"ABC")?;
        file.flush()?;

        if !sparse_storage_is_bounded(&file)? {
            file.set_len(0)?;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "filesystem did not retain the fixture as a bounded sparse file",
            ));
        }

        Ok(fixture)
    }

    fn create() -> io::Result<(Self, File)> {
        loop {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("faultsift-fs002-{}-{id}.tmp", std::process::id()));
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

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(windows)]
fn prepare_sparse_file(path: &Path) -> io::Result<()> {
    let output = Command::new("fsutil.exe")
        .args(["sparse", "setflag"])
        .arg(path)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "could not mark the Windows fixture sparse: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

#[cfg(target_os = "linux")]
fn prepare_sparse_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn sparse_storage_is_bounded(_file: &File) -> io::Result<bool> {
    // A successful `fsutil sparse setflag` above establishes the sparse-file
    // attribute before the logical length is extended.
    Ok(true)
}

#[cfg(target_os = "linux")]
fn sparse_storage_is_bounded(file: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    const MAX_FIXTURE_STORAGE_BYTES: u64 = 1024 * 1024;
    let allocated_bytes = file.metadata()?.blocks().saturating_mul(512);
    Ok(allocated_bytes <= MAX_FIXTURE_STORAGE_BYTES)
}

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn options(max_view_bytes: u64) -> FileAccessOptions {
    FileAccessOptions::new(ByteLength::new(max_view_bytes)).unwrap()
}

fn range(offset: u64, length: u64) -> ByteRange {
    ByteRange::new(ByteOffset::new(offset), ByteLength::new(length)).unwrap()
}

#[test]
fn reads_exact_views_and_caller_buffers() {
    let fixture = TestFile::from_bytes(b"0123456789").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(8)).unwrap();

    assert_eq!(snapshot.len().get(), 10);
    assert!(!snapshot.is_empty());
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);

    let view = snapshot.view(range(2, 4)).unwrap();
    assert_eq!(view.as_bytes(), b"2345");
    assert_eq!(view.range(), range(2, 4));
    assert_eq!(view.generation(), snapshot.generation());

    let mut smaller_buffer = [0; 3];
    assert_eq!(
        snapshot
            .read_at(ByteOffset::new(1), &mut smaller_buffer)
            .unwrap(),
        3
    );
    assert_eq!(&smaller_buffer, b"123");

    let mut buffer = [0xAA; 4];
    let read = snapshot.read_at(ByteOffset::new(8), &mut buffer).unwrap();
    assert_eq!(read, 2);
    assert_eq!(&buffer[..2], b"89");
    assert_eq!(&buffer[2..], &[0xAA, 0xAA]);
}

#[test]
fn handles_exact_eof_and_out_of_bounds_offsets() {
    let fixture = TestFile::from_bytes(b"abc").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(4)).unwrap();
    let mut buffer = [0; 2];

    assert_eq!(
        snapshot.read_at(ByteOffset::new(3), &mut buffer).unwrap(),
        0
    );
    assert!(matches!(
        snapshot.read_at(ByteOffset::new(4), &mut buffer),
        Err(FileAccessError::OutOfBounds { .. })
    ));
    assert!(matches!(
        snapshot.view(range(2, 2)),
        Err(FileAccessError::OutOfBounds { .. })
    ));
}

#[test]
fn accepts_empty_files_without_special_read_failures() {
    let fixture = TestFile::from_bytes(&[]).unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(1)).unwrap();

    assert!(snapshot.is_empty());
    assert!(snapshot.view(range(0, 0)).unwrap().is_empty());

    let mut buffer = [0; 1];
    assert_eq!(
        snapshot.read_at(ByteOffset::new(0), &mut buffer).unwrap(),
        0
    );
    assert!(matches!(
        snapshot.view(range(0, 1)),
        Err(FileAccessError::OutOfBounds { .. })
    ));
}

#[test]
fn preserves_arbitrary_bytes_without_utf8_decoding() {
    let bytes = [0xFF, 0x00, b'\r', b'\n', 0x80];
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(5)).unwrap();

    assert_eq!(snapshot.view(range(0, 5)).unwrap().as_bytes(), bytes);
}

#[test]
fn enforces_view_limit_without_limiting_caller_buffers() {
    let fixture = TestFile::from_bytes(b"abcdefgh").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(4)).unwrap();

    assert_eq!(snapshot.view(range(0, 4)).unwrap().len(), 4);
    assert!(matches!(
        snapshot.view(range(0, 5)),
        Err(FileAccessError::RangeTooLarge { .. })
    ));

    let mut buffer = [0; 8];
    assert_eq!(
        snapshot.read_at(ByteOffset::new(0), &mut buffer).unwrap(),
        8
    );
    assert_eq!(&buffer, b"abcdefgh");
}

#[test]
fn rejects_overflow_before_file_access() {
    let error = ByteRange::new(ByteOffset::new(u64::MAX), ByteLength::new(1)).unwrap_err();
    assert!(matches!(error, FileAccessError::RangeOverflow { .. }));
}

#[test]
fn reads_offsets_on_both_sides_of_four_gib() {
    const FOUR_GIB: u64 = 1_u64 << 32;

    let fixture = match TestFile::sparse_with_boundary_bytes(FOUR_GIB) {
        Ok(fixture) => fixture,
        Err(error) => {
            eprintln!("skipping sparse >4 GiB fixture: {error}");
            return;
        }
    };
    let snapshot = FileSnapshot::open(fixture.path(), options(3)).unwrap();

    assert_eq!(snapshot.len().get(), FOUR_GIB + 2);
    assert_eq!(
        snapshot.view(range(FOUR_GIB - 1, 3)).unwrap().as_bytes(),
        b"ABC"
    );
    assert_eq!(
        snapshot.view(range(FOUR_GIB - 1, 1)).unwrap().as_bytes(),
        b"A"
    );
    assert_eq!(snapshot.view(range(FOUR_GIB, 1)).unwrap().as_bytes(), b"B");
    assert_eq!(
        snapshot.view(range(FOUR_GIB + 1, 1)).unwrap().as_bytes(),
        b"C"
    );
}

#[test]
fn supports_concurrent_explicit_offset_reads() {
    let bytes: Vec<u8> = (0..65_536).map(|value| (value % 251) as u8).collect();
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = Arc::new(FileSnapshot::open(fixture.path(), options(256)).unwrap());
    let expected = Arc::new(bytes);

    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let snapshot = Arc::clone(&snapshot);
            let expected = Arc::clone(&expected);
            thread::spawn(move || {
                for iteration in 0..256 {
                    let offset = ((worker * 977 + iteration * 131) % (65_536 - 128)) as u64;
                    let view = snapshot.view(range(offset, 128)).unwrap();
                    let start = usize::try_from(offset).unwrap();
                    assert_eq!(view.as_bytes(), &expected[start..start + 128]);
                }
            })
        })
        .collect();

    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn rejects_directories_as_unsupported_file_types() {
    let error = FileSnapshot::open(std::env::temp_dir(), options(16)).unwrap_err();
    assert!(matches!(error, FileAccessError::UnsupportedFileType { .. }));
}

#[test]
fn reports_missing_files_as_open_failures() {
    let (fixture, file) = TestFile::create().unwrap();
    let path = fixture.path().to_path_buf();
    drop(file);
    drop(fixture);

    let error = FileSnapshot::open(path, options(16)).unwrap_err();
    assert!(matches!(error, FileAccessError::OpenFailed { .. }));
}

#[test]
fn reports_unexpected_eof_after_truncate() {
    let fixture = TestFile::from_bytes(b"captured bytes").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(32)).unwrap();

    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(fixture.path())
        .unwrap();

    let error = snapshot.view(range(0, 8)).unwrap_err();
    assert!(matches!(error, FileAccessError::UnexpectedEof { .. }));
}
