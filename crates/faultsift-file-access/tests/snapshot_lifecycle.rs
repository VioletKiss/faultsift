mod support;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

use faultsift_file_access::{
    ByteOffset, FileAccessError, FileSnapshot, SnapshotState, SnapshotValidation, StaleReason,
};

use support::{TestFile, open_buffered_snapshot, options, range, unique_path};

#[test]
fn unchanged_validation_preserves_fresh_identity_and_generation() {
    let fixture = TestFile::from_bytes(b"unchanged").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(32)).unwrap();
    let original_identity = snapshot.identity().clone();
    let original_generation = snapshot.generation();

    assert_eq!(snapshot.state(), SnapshotState::Fresh);
    assert_eq!(snapshot.validate().unwrap(), SnapshotValidation::Unchanged);
    assert_eq!(snapshot.validate().unwrap(), SnapshotValidation::Unchanged);
    assert_eq!(snapshot.state(), SnapshotState::Fresh);
    assert_eq!(snapshot.identity(), &original_identity);
    assert_eq!(snapshot.generation(), original_generation);
}

#[test]
fn growth_is_invisible_to_reads_until_explicit_validation() {
    let fixture = TestFile::from_bytes(b"old").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));
    let original_identity = snapshot.identity().clone();
    let original_generation = snapshot.generation();

    fixture.append(b"-new").unwrap();

    assert_eq!(snapshot.view(range(0, 3)).unwrap().as_bytes(), b"old");
    assert_eq!(snapshot.state(), SnapshotState::Fresh);
    assert_eq!(snapshot.len().get(), 3);
    let mut buffer = [0; 1];
    assert_eq!(
        snapshot.read_at(ByteOffset::new(3), &mut buffer).unwrap(),
        0
    );

    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );
    assert!(matches!(
        snapshot.view(range(0, 1)),
        Err(FileAccessError::StaleSnapshot {
            reason: StaleReason::Grown
        })
    ));

    let reopened = snapshot.reopen().unwrap();
    assert_eq!(reopened.len().get(), 7);
    assert_ne!(reopened.generation(), original_generation);
    assert_eq!(reopened.identity(), &original_identity);
    assert_eq!(reopened.state(), SnapshotState::Fresh);

    assert_eq!(snapshot.len().get(), 3);
    assert_eq!(snapshot.generation(), original_generation);
    assert_eq!(snapshot.identity(), &original_identity);
    assert_eq!(snapshot.state(), SnapshotState::Stale(StaleReason::Grown));
}

#[test]
fn unexpected_eof_marks_stale_and_the_first_reason_is_permanent() {
    let fixture = TestFile::from_bytes(b"abcdefgh").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));
    let original_identity = snapshot.identity().clone();
    let original_generation = snapshot.generation();

    fixture.truncate(2).unwrap();

    assert!(matches!(
        snapshot.view(range(4, 2)),
        Err(FileAccessError::UnexpectedEof { .. })
    ));
    assert_eq!(
        snapshot.state(),
        SnapshotState::Stale(StaleReason::UnexpectedEof)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::UnexpectedEof)
    );
    assert!(matches!(
        snapshot.view(range(0, 1)),
        Err(FileAccessError::StaleSnapshot {
            reason: StaleReason::UnexpectedEof
        })
    ));

    let reopened = snapshot.reopen().unwrap();
    assert_eq!(reopened.len().get(), 2);
    assert_ne!(reopened.generation(), original_generation);
    assert_eq!(reopened.identity(), &original_identity);
    assert_eq!(snapshot.len().get(), 8);
    assert_eq!(snapshot.generation(), original_generation);
}

#[test]
fn explicit_validation_distinguishes_truncation() {
    let fixture = TestFile::from_bytes(b"abcdefgh").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));
    let original_identity = snapshot.identity().clone();
    let original_generation = snapshot.generation();

    fixture.truncate(2).unwrap();

    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Truncated)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Truncated)
    );

    let reopened = snapshot.reopen().unwrap();
    assert_eq!(reopened.len().get(), 2);
    assert_ne!(reopened.generation(), original_generation);
    assert_eq!(reopened.identity(), &original_identity);
    assert_eq!(snapshot.len().get(), 8);
    assert_eq!(snapshot.generation(), original_generation);
}

#[test]
fn replacement_is_detected_by_handle_identity_even_at_equal_length() {
    let fixture = TestFile::from_bytes(b"first").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));
    let original_identity = snapshot.identity().clone();
    let original_generation = snapshot.generation();

    fixture.replace(b"other").unwrap();

    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );
    let reopened = snapshot.reopen().unwrap();
    assert_ne!(reopened.identity(), &original_identity);
    assert_ne!(reopened.generation(), original_generation);
    assert_eq!(reopened.len().get(), 5);
    assert_eq!(reopened.view(range(0, 5)).unwrap().as_bytes(), b"other");

    assert_eq!(snapshot.identity(), &original_identity);
    assert_eq!(snapshot.generation(), original_generation);
    assert_eq!(snapshot.len().get(), 5);
    assert_eq!(
        snapshot.state(),
        SnapshotState::Stale(StaleReason::Replaced)
    );
}

#[test]
fn symlink_identity_tracks_the_resolved_target_and_detects_retargeting() {
    let original = TestFile::from_bytes(b"first").unwrap();
    let replacement = TestFile::from_bytes(b"other").unwrap();
    let link_path = unique_path("symlink");

    if let Err(error) = create_file_symlink(original.path(), &link_path) {
        if symlink_capability_unavailable(&error) {
            return;
        }
        panic!("failed to create test symlink: {error}");
    }
    let _link_guard = SymlinkGuard(link_path.clone());

    let snapshot = open_buffered_snapshot(&link_path, options(16));
    let resolved_target = FileSnapshot::open(original.path(), options(16)).unwrap();
    assert_eq!(snapshot.identity(), resolved_target.identity());

    fs::remove_file(&link_path).unwrap();
    create_file_symlink(replacement.path(), &link_path).unwrap();

    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );
}

#[test]
fn deletion_is_only_observed_by_explicit_validation() {
    let fixture = TestFile::from_bytes(b"retained").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));

    fixture.remove().unwrap();

    assert_eq!(snapshot.view(range(0, 8)).unwrap().as_bytes(), b"retained");
    assert_eq!(snapshot.state(), SnapshotState::Fresh);
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Missing)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Missing)
    );
    assert!(matches!(
        snapshot.reopen(),
        Err(FileAccessError::OpenFailed { .. })
    ));
    assert_eq!(snapshot.state(), SnapshotState::Stale(StaleReason::Missing));
}

#[test]
fn relevant_metadata_change_is_detected_without_reading_file_bytes() {
    let fixture = TestFile::from_bytes(b"same bytes").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));

    fixture.set_distinct_modified_time().unwrap();

    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Modified)
    );
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Modified)
    );
}

#[test]
fn equal_length_overwrite_is_explicitly_best_effort() {
    let fixture = TestFile::from_bytes(b"before").unwrap();
    let snapshot = open_buffered_snapshot(fixture.path(), options(16));

    fixture.overwrite(b"after!").unwrap();

    match snapshot.validate().unwrap() {
        SnapshotValidation::Unchanged => assert_eq!(snapshot.state(), SnapshotState::Fresh),
        SnapshotValidation::Stale(StaleReason::Modified) => {
            assert_eq!(
                snapshot.state(),
                SnapshotState::Stale(StaleReason::Modified)
            );
            assert_eq!(
                snapshot.validate().unwrap(),
                SnapshotValidation::Stale(StaleReason::Modified)
            );
        }
        outcome => panic!("unexpected equal-length validation outcome: {outcome:?}"),
    }
}

#[test]
fn reopen_of_unchanged_input_still_creates_a_new_generation() {
    let fixture = TestFile::from_bytes(b"same").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(16)).unwrap();
    let reopened = snapshot.reopen().unwrap();

    assert_ne!(snapshot.generation(), reopened.generation());
    assert_eq!(snapshot.identity(), reopened.identity());
    assert_eq!(snapshot.len(), reopened.len());
    assert_eq!(snapshot.state(), SnapshotState::Fresh);
    assert_eq!(reopened.state(), SnapshotState::Fresh);
}

#[test]
fn readers_and_validation_race_without_deadlock_or_cursor_dependence() {
    let bytes: Vec<u8> = (0..65_536).map(|value| (value % 251) as u8).collect();
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = Arc::new(open_buffered_snapshot(fixture.path(), options(256)));
    let expected = Arc::new(bytes);
    let start = Arc::new(Barrier::new(9));

    let readers: Vec<_> = (0..8)
        .map(|worker| {
            let snapshot = Arc::clone(&snapshot);
            let expected = Arc::clone(&expected);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                for iteration in 0..2_048 {
                    let offset = ((worker * 977 + iteration * 131) % (65_536 - 128)) as u64;
                    match snapshot.view(range(offset, 128)) {
                        Ok(view) => {
                            let offset = usize::try_from(offset).unwrap();
                            assert_eq!(view.as_bytes(), &expected[offset..offset + 128]);
                        }
                        Err(FileAccessError::StaleSnapshot {
                            reason: StaleReason::Grown,
                        }) => break,
                        Err(error) => panic!("unexpected concurrent read error: {error}"),
                    }
                }
            })
        })
        .collect();

    start.wait();
    fixture.append(b"growth").unwrap();
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    for reader in readers {
        reader.join().unwrap();
    }
    assert_eq!(snapshot.state(), SnapshotState::Stale(StaleReason::Grown));
}

struct SymlinkGuard(PathBuf);

impl Drop for SymlinkGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(target_os = "linux")]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(target_os = "linux")]
fn symlink_capability_unavailable(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
fn symlink_capability_unavailable(error: &io::Error) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1_314;

    error.kind() == io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
}
