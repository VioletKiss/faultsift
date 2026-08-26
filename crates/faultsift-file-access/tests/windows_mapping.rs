#![cfg(windows)]

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use faultsift_file_access::{
    BackendKind, ByteLength, ByteOffset, ByteRange, FileAccessError, FileAccessOptions,
    FileSnapshot, MappingFallbackReason, SnapshotValidation,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct TestFile {
    path: PathBuf,
}

impl TestFile {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        loop {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "faultsift-fs004-mapping-{}-{id}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(bytes)?;
                    file.flush()?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(&self) -> io::Result<()> {
        fs::remove_file(&self.path)
    }
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
fn eligible_local_regular_file_selects_mapping_and_preserves_byte_contract() {
    let bytes = [0xFF, b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', 0x80];
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(4)).unwrap();

    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Mapped);
    assert_eq!(snapshot.diagnostics().mapping_fallback_reason(), None);
    assert!(!snapshot.diagnostics().used_buffered_fallback());

    let beginning = snapshot.view(range(0, 4)).unwrap();
    assert_eq!(beginning.as_bytes(), &bytes[0..4]);
    assert_eq!(beginning.range(), range(0, 4));
    assert_eq!(beginning.generation(), snapshot.generation());

    assert_eq!(snapshot.view(range(3, 4)).unwrap().as_bytes(), &bytes[3..7]);
    assert_eq!(
        snapshot.view(range(7, 3)).unwrap().as_bytes(),
        &bytes[7..10]
    );
    assert!(snapshot.view(range(10, 0)).unwrap().is_empty());
    assert!(matches!(
        snapshot.view(range(0, 5)),
        Err(FileAccessError::RangeTooLarge { .. })
    ));
    assert!(matches!(
        snapshot.view(range(9, 2)),
        Err(FileAccessError::OutOfBounds { .. })
    ));

    let mut beginning_buffer = [0_u8; 3];
    assert_eq!(
        snapshot
            .read_at(ByteOffset::new(0), &mut beginning_buffer)
            .unwrap(),
        3
    );
    assert_eq!(beginning_buffer, bytes[0..3]);

    let mut middle_buffer = [0_u8; 4];
    assert_eq!(
        snapshot
            .read_at(ByteOffset::new(3), &mut middle_buffer)
            .unwrap(),
        4
    );
    assert_eq!(middle_buffer, bytes[3..7]);

    let mut end_buffer = [0xAA_u8; 4];
    assert_eq!(
        snapshot
            .read_at(ByteOffset::new(8), &mut end_buffer)
            .unwrap(),
        2
    );
    assert_eq!(&end_buffer[..2], &bytes[8..10]);
    assert_eq!(&end_buffer[2..], &[0xAA, 0xAA]);
    assert_eq!(
        snapshot
            .read_at(ByteOffset::new(10), &mut end_buffer)
            .unwrap(),
        0
    );
}

#[test]
fn empty_file_is_valid_and_never_attempts_mapping() {
    let fixture = TestFile::from_bytes(&[]).unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(1)).unwrap();

    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);
    assert_eq!(
        snapshot.diagnostics().mapping_fallback_reason(),
        Some(MappingFallbackReason::EmptyFile)
    );
    assert!(!snapshot.diagnostics().used_buffered_fallback());
    assert!(snapshot.is_empty());
    assert!(snapshot.view(range(0, 0)).unwrap().is_empty());

    let mut byte = [0_u8; 1];
    assert_eq!(snapshot.read_at(ByteOffset::new(0), &mut byte).unwrap(), 0);
    assert!(matches!(
        snapshot.view(range(0, 1)),
        Err(FileAccessError::OutOfBounds { .. })
    ));
}

#[test]
fn incompatible_writer_causes_transparent_working_buffered_fallback() {
    let fixture = TestFile::from_bytes(b"writer fallback bytes").unwrap();
    let writer = OpenOptions::new().write(true).open(fixture.path()).unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(32)).unwrap();

    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);
    assert_eq!(
        snapshot.diagnostics().mapping_fallback_reason(),
        Some(MappingFallbackReason::IncompatibleWriter)
    );
    assert!(snapshot.diagnostics().used_buffered_fallback());
    assert_eq!(snapshot.view(range(7, 8)).unwrap().as_bytes(), b"fallback");
    let mut buffer = [0_u8; 5];
    assert_eq!(
        snapshot.read_at(ByteOffset::new(16), &mut buffer).unwrap(),
        5
    );
    assert_eq!(&buffer, b"bytes");

    drop(snapshot);
    drop(writer);
}

#[test]
fn retained_range_view_keeps_mapping_and_stability_handle_alive() {
    let fixture = TestFile::from_bytes(b"mapped lifetime").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(32)).unwrap();
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Mapped);
    let view = snapshot.view(range(0, 15)).unwrap();

    drop(snapshot);
    assert_eq!(view.as_bytes(), b"mapped lifetime");
    assert!(
        fixture.remove().is_err(),
        "the RangeView must retain the stability handle"
    );

    drop(view);
    fixture.remove().unwrap();
}

#[test]
fn reopen_creates_new_generation_without_changing_old_mapping() {
    let fixture = TestFile::from_bytes(b"generation bytes").unwrap();
    let original = FileSnapshot::open(fixture.path(), options(32)).unwrap();
    let original_identity = original.identity().clone();
    let original_generation = original.generation();
    let original_view = original.view(range(0, 10)).unwrap();

    let reopened = original.reopen().unwrap();
    assert_eq!(reopened.diagnostics().backend(), BackendKind::Mapped);
    assert_ne!(reopened.generation(), original_generation);
    assert_eq!(reopened.identity(), &original_identity);
    assert_eq!(reopened.validate().unwrap(), SnapshotValidation::Unchanged);

    drop(original);
    assert_eq!(original_view.as_bytes(), b"generation");
    assert_eq!(original_view.generation(), original_generation);
    assert_eq!(reopened.view(range(11, 5)).unwrap().as_bytes(), b"bytes");
}

#[test]
fn mapped_snapshot_supports_concurrent_views_and_caller_buffer_reads() {
    let bytes: Vec<u8> = (0..65_536).map(|value| (value % 251) as u8).collect();
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = Arc::new(FileSnapshot::open(fixture.path(), options(256)).unwrap());
    let expected = Arc::new(bytes);
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Mapped);

    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let snapshot = Arc::clone(&snapshot);
            let expected = Arc::clone(&expected);
            thread::spawn(move || {
                for iteration in 0..256 {
                    let offset = ((worker * 977 + iteration * 131) % (65_536 - 128)) as u64;
                    let start = usize::try_from(offset).unwrap();
                    assert_eq!(
                        snapshot.view(range(offset, 128)).unwrap().as_bytes(),
                        &expected[start..start + 128]
                    );

                    let mut buffer = [0_u8; 128];
                    assert_eq!(
                        snapshot
                            .read_at(ByteOffset::new(offset), &mut buffer)
                            .unwrap(),
                        buffer.len()
                    );
                    assert_eq!(&buffer, &expected[start..start + 128]);
                }
            })
        })
        .collect();

    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn public_ranges_remain_checked_before_mapped_access() {
    let fixture = TestFile::from_bytes(b"range").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(8)).unwrap();

    let overflow =
        faultsift_file_access::ByteRange::new(ByteOffset::new(u64::MAX), ByteLength::new(1))
            .unwrap_err();
    assert!(matches!(overflow, FileAccessError::RangeOverflow { .. }));
    assert!(matches!(
        snapshot.read_at(ByteOffset::new(6), &mut [0_u8; 1]),
        Err(FileAccessError::OutOfBounds { .. })
    ));
}
