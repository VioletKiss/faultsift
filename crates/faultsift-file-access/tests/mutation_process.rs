mod support;

#[cfg(windows)]
use std::fs::{self, OpenOptions};
#[cfg(windows)]
use std::io::{self, Write};
#[cfg(windows)]
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use faultsift_file_access::{BackendKind, ByteOffset, FileSnapshot};
use faultsift_file_access::{FileAccessError, SnapshotState, SnapshotValidation, StaleReason};

use support::{TestFile, open_buffered_snapshot, options, range};

const CHILD_ENV: &str = "FAULTSIFT_FS003_MUTATION_CHILD";
#[cfg(windows)]
const MAPPED_OWNER_ENV: &str = "FAULTSIFT_FS004_MAPPED_OWNER";
#[cfg(windows)]
const MAPPED_MUTATOR_ENV: &str = "FAULTSIFT_FS004_MAPPED_MUTATOR";
#[cfg(windows)]
const MAPPED_MUTATION_PATH_ENV: &str = "FAULTSIFT_FS004_MUTATION_PATH";
#[cfg(windows)]
const MAPPED_MUTATION_ACTION_ENV: &str = "FAULTSIFT_FS004_MUTATION_ACTION";

#[test]
fn mutation_scenarios_terminate_normally() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mutation_child_entry", "--nocapture"])
        .env(CHILD_ENV, "1")
        .status()
        .unwrap();

    assert!(status.success(), "mutation child exited with {status}");
}

#[test]
fn mutation_child_entry() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let growth = TestFile::from_bytes(b"old").unwrap();
    let growth_snapshot = open_buffered_snapshot(growth.path(), options(16));
    growth.append(b"new").unwrap();
    assert_eq!(
        growth_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    let explicit_truncate = TestFile::from_bytes(b"abcdefgh").unwrap();
    let explicit_truncate_snapshot = open_buffered_snapshot(explicit_truncate.path(), options(16));
    explicit_truncate.truncate(2).unwrap();
    assert_eq!(
        explicit_truncate_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Truncated)
    );

    let truncate = TestFile::from_bytes(b"abcdefgh").unwrap();
    let truncate_snapshot = open_buffered_snapshot(truncate.path(), options(16));
    truncate.truncate(2).unwrap();
    assert!(matches!(
        truncate_snapshot.view(range(4, 2)),
        Err(FileAccessError::UnexpectedEof { .. })
    ));
    assert_eq!(
        truncate_snapshot.state(),
        SnapshotState::Stale(StaleReason::UnexpectedEof)
    );

    let replacement = TestFile::from_bytes(b"first").unwrap();
    let replacement_snapshot = open_buffered_snapshot(replacement.path(), options(16));
    replacement.replace(b"other").unwrap();
    assert_eq!(
        replacement_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );

    let missing = TestFile::from_bytes(b"gone").unwrap();
    let missing_snapshot = open_buffered_snapshot(missing.path(), options(16));
    missing.remove().unwrap();
    assert_eq!(
        missing_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Missing)
    );

    let metadata_change = TestFile::from_bytes(b"metadata").unwrap();
    let metadata_snapshot = open_buffered_snapshot(metadata_change.path(), options(16));
    metadata_change.set_distinct_modified_time().unwrap();
    assert_eq!(
        metadata_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Modified)
    );

    let equal_length = TestFile::from_bytes(b"before").unwrap();
    let equal_length_snapshot = open_buffered_snapshot(equal_length.path(), options(16));
    equal_length.overwrite(b"after!").unwrap();
    assert!(matches!(
        equal_length_snapshot.validate().unwrap(),
        SnapshotValidation::Unchanged | SnapshotValidation::Stale(StaleReason::Modified)
    ));
}

#[cfg(windows)]
#[test]
fn mapped_mutation_scenarios_terminate_normally() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "mapped_mutation_owner_child_entry",
            "--nocapture",
        ])
        .env(MAPPED_OWNER_ENV, "1")
        .status()
        .unwrap();

    assert!(status.success(), "mapped owner child exited with {status}");
}

#[cfg(windows)]
#[test]
fn mapped_mutation_owner_child_entry() {
    if std::env::var_os(MAPPED_OWNER_ENV).is_none() {
        return;
    }

    let fixture = TestFile::from_bytes(b"stable mapped bytes").unwrap();
    let snapshot = FileSnapshot::open(fixture.path(), options(64)).unwrap();
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Mapped);
    let retained_view = snapshot.view(range(0, 19)).unwrap();

    for action in ["write", "truncate", "delete", "rename", "replace"] {
        run_mapped_mutator(fixture.path(), action);
        assert_eq!(retained_view.as_bytes(), b"stable mapped bytes");

        let mut buffer = [0_u8; 6];
        assert_eq!(
            snapshot.read_at(ByteOffset::new(7), &mut buffer).unwrap(),
            buffer.len()
        );
        assert_eq!(&buffer, b"mapped");
    }

    drop(snapshot);
    run_mapped_mutator(fixture.path(), "delete");
    assert_eq!(retained_view.as_bytes(), b"stable mapped bytes");

    drop(retained_view);
    fixture.remove().unwrap();
}

#[cfg(windows)]
fn run_mapped_mutator(path: &Path, action: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mapped_mutator_child_entry", "--nocapture"])
        .env(MAPPED_MUTATOR_ENV, "1")
        .env(MAPPED_MUTATION_PATH_ENV, path.as_os_str())
        .env(MAPPED_MUTATION_ACTION_ENV, action)
        .status()
        .unwrap();

    assert!(
        status.success(),
        "mapped {action} mutator exited with {status}"
    );
}

#[cfg(windows)]
#[test]
fn mapped_mutator_child_entry() {
    if std::env::var_os(MAPPED_MUTATOR_ENV).is_none() {
        return;
    }

    let path = std::path::PathBuf::from(
        std::env::var_os(MAPPED_MUTATION_PATH_ENV).expect("mutation path must be supplied"),
    );
    let action =
        std::env::var(MAPPED_MUTATION_ACTION_ENV).expect("mutation action must be supplied");

    let result = match action.as_str() {
        "write" => OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut file| file.write_all(b"damage")),
        "truncate" => OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .map(drop),
        "delete" => fs::remove_file(&path),
        "rename" => {
            let renamed = path.with_extension(format!("renamed-{}", std::process::id()));
            let result = fs::rename(&path, &renamed);
            if result.is_err() {
                let _ = fs::remove_file(&renamed);
            }
            result
        }
        "replace" => attempt_replace(&path),
        other => panic!("unsupported mutation action: {other}"),
    };

    assert!(result.is_err(), "mapped {action} unexpectedly succeeded");
}

#[cfg(windows)]
fn attempt_replace(path: &Path) -> io::Result<()> {
    let replacement = path.with_extension(format!("replacement-{}", std::process::id()));
    fs::write(&replacement, b"replacement bytes")?;

    let result = fs::remove_file(path).and_then(|()| fs::rename(&replacement, path));
    if result.is_err() {
        let _ = fs::remove_file(&replacement);
    }
    result
}
