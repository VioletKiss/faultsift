mod support;

use std::process::Command;

use faultsift_file_access::{
    FileAccessError, FileSnapshot, SnapshotState, SnapshotValidation, StaleReason,
};

use support::{TestFile, options, range};

const CHILD_ENV: &str = "FAULTSIFT_FS003_MUTATION_CHILD";

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
    let growth_snapshot = FileSnapshot::open(growth.path(), options(16)).unwrap();
    growth.append(b"new").unwrap();
    assert_eq!(
        growth_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    let explicit_truncate = TestFile::from_bytes(b"abcdefgh").unwrap();
    let explicit_truncate_snapshot =
        FileSnapshot::open(explicit_truncate.path(), options(16)).unwrap();
    explicit_truncate.truncate(2).unwrap();
    assert_eq!(
        explicit_truncate_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Truncated)
    );

    let truncate = TestFile::from_bytes(b"abcdefgh").unwrap();
    let truncate_snapshot = FileSnapshot::open(truncate.path(), options(16)).unwrap();
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
    let replacement_snapshot = FileSnapshot::open(replacement.path(), options(16)).unwrap();
    replacement.replace(b"other").unwrap();
    assert_eq!(
        replacement_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Replaced)
    );

    let missing = TestFile::from_bytes(b"gone").unwrap();
    let missing_snapshot = FileSnapshot::open(missing.path(), options(16)).unwrap();
    missing.remove().unwrap();
    assert_eq!(
        missing_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Missing)
    );

    let metadata_change = TestFile::from_bytes(b"metadata").unwrap();
    let metadata_snapshot = FileSnapshot::open(metadata_change.path(), options(16)).unwrap();
    metadata_change.set_distinct_modified_time().unwrap();
    assert_eq!(
        metadata_snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Modified)
    );

    let equal_length = TestFile::from_bytes(b"before").unwrap();
    let equal_length_snapshot = FileSnapshot::open(equal_length.path(), options(16)).unwrap();
    equal_length.overwrite(b"after!").unwrap();
    assert!(matches!(
        equal_length_snapshot.validate().unwrap(),
        SnapshotValidation::Unchanged | SnapshotValidation::Stale(StaleReason::Modified)
    ));
}
