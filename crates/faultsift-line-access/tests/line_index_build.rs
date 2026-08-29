mod support;

use std::convert::Infallible;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use faultsift_file_access::{
    ByteLength, FileAccessError, SnapshotState, SnapshotValidation, StaleReason,
};
use faultsift_line_access::{
    BuildControl, BuildProgress, LineAccessError, LineIndex, LineIndexOptions,
};

use support::{TestFile, cursor, snapshot};

fn options(max_checkpoints: u64, scan_chunk_bytes: u64) -> LineIndexOptions {
    LineIndexOptions::new(
        ByteLength::new(max_checkpoints * std::mem::size_of::<u64>() as u64),
        ByteLength::new(scan_chunk_bytes),
    )
    .unwrap()
}

fn build(bytes: &[u8], max_checkpoints: u64, scan_chunk_bytes: u64) -> LineIndex {
    let fixture = TestFile::from_bytes(bytes).unwrap();
    LineIndex::build(
        snapshot(&fixture),
        options(max_checkpoints, scan_chunk_bytes),
    )
    .unwrap()
}

#[test]
fn invalid_configuration_is_typed_before_build_can_scan() {
    let offset_bytes = std::mem::size_of::<u64>() as u64;
    for budget in [0, offset_bytes, offset_bytes * 2 - 1] {
        assert!(matches!(
            LineIndexOptions::new(ByteLength::new(budget), ByteLength::new(1)),
            Err(LineAccessError::InvalidCheckpointBudgetBytes { .. })
        ));
    }
    assert!(matches!(
        LineIndexOptions::new(ByteLength::new(offset_bytes * 2), ByteLength::new(0)),
        Err(LineAccessError::InvalidScanChunkBytes { .. })
    ));
}

#[cfg(target_pointer_width = "32")]
#[test]
fn unrepresentable_scan_chunk_is_rejected() {
    assert!(matches!(
        LineIndexOptions::new(
            ByteLength::new(16),
            ByteLength::new(u64::from(u32::MAX) + 1)
        ),
        Err(LineAccessError::ScanChunkNotRepresentable { .. })
    ));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn impossible_checkpoint_allocation_is_typed_before_scanning() {
    let fixture = TestFile::from_bytes(b"must not be scanned").unwrap();
    let snapshot = snapshot(&fixture);
    let options = LineIndexOptions::new(ByteLength::new(u64::MAX), ByteLength::new(1)).unwrap();
    assert!(matches!(
        LineIndex::build(snapshot, options),
        Err(LineAccessError::CheckpointAllocationFailed { .. })
    ));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn impossible_scan_buffer_allocation_is_typed_before_scanning() {
    let fixture = TestFile::from_bytes(b"must not be scanned").unwrap();
    let snapshot = snapshot(&fixture);
    let options = LineIndexOptions::new(ByteLength::new(16), ByteLength::new(u64::MAX)).unwrap();
    assert!(matches!(
        LineIndex::build(snapshot, options),
        Err(LineAccessError::ScanBufferAllocationFailed { .. })
    ));
}

#[test]
fn approved_newline_and_arbitrary_byte_counts_are_exact() {
    let cases: &[(&[u8], u64)] = &[
        (b"", 0),
        (b"\n", 1),
        (b"\n\n", 2),
        (b"a\n", 1),
        (b"a\n\n", 2),
        (b"a", 1),
        (b"\r", 1),
        (b"\r\n", 1),
        (b"a\r\n\r\n", 2),
        (&[0xff, 0x00, b'\r', b'X', b'\n', 0x80, 0x00], 2),
    ];

    for (bytes, expected) in cases {
        for chunk_bytes in 1..=4 {
            let index = build(bytes, 3, chunk_bytes);
            assert_eq!(index.line_count(), *expected, "input={bytes:?}");
            assert_eq!(index.snapshot_length().get(), bytes.len() as u64);
            assert_eq!(index.checkpoint_count(), u64::from(*expected != 0));
            assert_eq!(index.final_stride(), 256);
        }
    }
}

#[test]
fn ready_metadata_and_snapshot_instance_are_immutable() {
    let fixture = TestFile::from_bytes(b"first\nsecond").unwrap();
    let snapshot = snapshot(&fixture);
    let generation = snapshot.generation();
    let options = options(7, 3);
    let index = LineIndex::build(Arc::clone(&snapshot), options).unwrap();

    assert!(Arc::ptr_eq(index.snapshot(), &snapshot));
    assert_eq!(index.generation(), generation);
    assert_eq!(index.snapshot_length().get(), 12);
    assert_eq!(index.line_count(), 2);
    assert_eq!(index.final_stride(), 256);
    assert_eq!(index.checkpoint_count(), 1);
    assert_eq!(index.checkpoint_budget_bytes().get(), 56);
    assert_eq!(index.scan_chunk_bytes().get(), 3);
}

#[test]
fn initial_stride_and_terminal_lf_never_create_a_phantom_checkpoint() {
    let mut bytes = vec![b'\n'; 257];
    let index = build(&bytes, 4, 19);
    assert_eq!(index.line_count(), 257);
    assert_eq!(index.final_stride(), 256);
    assert_eq!(index.checkpoint_count(), 2);

    bytes.truncate(256);
    let index = build(&bytes, 4, 19);
    assert_eq!(index.line_count(), 256);
    assert_eq!(index.checkpoint_count(), 1);
}

#[test]
fn newline_dense_input_forces_repeated_bounded_compaction() {
    let bytes = vec![b'\n'; 4_097];
    let index = build(&bytes, 3, 31);
    assert_eq!(index.line_count(), 4_097);
    assert_eq!(index.final_stride(), 2_048);
    assert_eq!(index.checkpoint_count(), 3);
    assert!(index.checkpoint_count() <= 3);
}

#[test]
fn progress_is_monotonic_exact_and_chunk_bounded() {
    let bytes = vec![b'\n'; 4_097];
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = snapshot(&fixture);
    let mut observed = Vec::new();
    let index = LineIndex::build_with_control(snapshot, options(3, 31), |progress| {
        observed.push(progress);
        BuildControl::Continue
    })
    .unwrap();

    assert_eq!(observed.len(), bytes.len().div_ceil(31));
    for pair in observed.windows(2) {
        assert!(pair[0].bytes_scanned() <= pair[1].bytes_scanned());
        assert!(pair[0].physical_lines_completed() <= pair[1].physical_lines_completed());
        assert!(pair[0].current_stride() <= pair[1].current_stride());
    }
    for progress in &observed {
        assert!(progress.bytes_scanned() <= progress.snapshot_length());
        assert!(progress.checkpoint_count() <= 3);
        assert_eq!(progress.current_stride() % 256, 0);
        assert!(progress.current_stride().is_power_of_two());
    }
    let final_progress = observed.last().copied().unwrap();
    assert_eq!(final_progress.bytes_scanned().get(), bytes.len() as u64);
    assert_eq!(final_progress.snapshot_length().get(), bytes.len() as u64);
    assert_eq!(
        final_progress.physical_lines_completed(),
        index.line_count()
    );
    assert_eq!(final_progress.current_stride(), index.final_stride());
    assert_eq!(final_progress.checkpoint_count(), index.checkpoint_count());
}

#[test]
fn one_long_line_reports_and_cancels_at_chunk_boundaries() {
    let fixture = TestFile::streamed_line(100).unwrap();
    let snapshot = snapshot(&fixture);
    let descriptor = cursor(Arc::clone(&snapshot), 11)
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap()
        .unwrap();
    assert_eq!(descriptor.content_range().length().get(), 100);
    let mut observed = Vec::new();
    let index = LineIndex::build_with_control(Arc::clone(&snapshot), options(2, 11), |progress| {
        observed.push(progress);
        BuildControl::Continue
    })
    .unwrap();
    assert_eq!(observed.len(), 101_usize.div_ceil(11));
    assert!(
        observed[..observed.len() - 1]
            .iter()
            .all(|progress| progress.physical_lines_completed() == 0)
    );
    assert_eq!(observed.last().unwrap().physical_lines_completed(), 1);
    assert_eq!(index.line_count(), 1);

    let mut calls = 0;
    let result = LineIndex::build_with_control(snapshot, options(2, 11), |_| {
        calls += 1;
        BuildControl::Cancel
    });
    assert!(matches!(result, Err(LineAccessError::IndexBuildCancelled)));
    assert_eq!(calls, 1);
}

#[test]
fn empty_snapshot_reports_one_exact_terminal_boundary() {
    let fixture = TestFile::from_bytes(b"").unwrap();
    let mut observed = Vec::new();
    let index = LineIndex::build_with_control(snapshot(&fixture), options(2, 7), |progress| {
        observed.push(progress);
        BuildControl::Continue
    })
    .unwrap();
    assert_eq!(index.line_count(), 0);
    assert_eq!(index.checkpoint_count(), 0);
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].bytes_scanned().get(), 0);
    assert_eq!(observed[0].physical_lines_completed(), 0);
}

#[test]
fn cancellation_at_first_middle_and_final_boundaries_returns_no_index() {
    let bytes = vec![b'\n'; 101];
    for cancel_at in [1, 3, bytes.len().div_ceil(11)] {
        let fixture = TestFile::from_bytes(&bytes).unwrap();
        let snapshot = snapshot(&fixture);
        let mut calls = 0;
        let result = LineIndex::build_with_control(Arc::clone(&snapshot), options(2, 11), |_| {
            calls += 1;
            if calls == cancel_at {
                BuildControl::Cancel
            } else {
                BuildControl::Continue
            }
        });
        assert!(matches!(result, Err(LineAccessError::IndexBuildCancelled)));
        assert_eq!(calls, cancel_at);
        assert_eq!(snapshot.state(), SnapshotState::Fresh);

        let retry = LineIndex::build(Arc::clone(&snapshot), options(2, 11)).unwrap();
        assert_eq!(retry.line_count(), bytes.len() as u64);
        assert_eq!(snapshot.state(), SnapshotState::Fresh);
    }
}

#[test]
fn stale_before_build_is_rejected_without_a_ready_index() {
    let fixture = TestFile::from_bytes(b"first\nsecond\n").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"growth").unwrap();
    writer.flush().unwrap();
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    assert!(matches!(
        LineIndex::build(snapshot, options(2, 4)),
        Err(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
}

#[test]
fn truncation_read_failure_is_propagated_and_marks_snapshot_stale() {
    let fixture = TestFile::from_bytes(b"abcdef").unwrap();
    let writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    writer.set_len(1).unwrap();

    assert!(matches!(
        LineIndex::build(Arc::clone(&snapshot), options(2, 4)),
        Err(LineAccessError::FileAccess(
            FileAccessError::UnexpectedEof { .. }
        ))
    ));
    assert_eq!(
        snapshot.state(),
        SnapshotState::Stale(StaleReason::UnexpectedEof)
    );
}

#[test]
fn stale_during_control_callback_aborts_before_ready() {
    let fixture = TestFile::from_bytes(&[b'\n'; 101]).unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    let generation = snapshot.generation();
    let captured_length = snapshot.len();
    let mut callback_ran = false;

    let result = LineIndex::build_with_control(Arc::clone(&snapshot), options(2, 11), |_| {
        if !callback_ran {
            writer.seek(SeekFrom::End(0)).unwrap();
            writer.write_all(b"growth").unwrap();
            writer.flush().unwrap();
            assert_eq!(
                snapshot.validate().unwrap(),
                SnapshotValidation::Stale(StaleReason::Grown)
            );
            callback_ran = true;
        }
        BuildControl::Continue
    });

    assert!(callback_ran);
    assert!(matches!(
        result,
        Err(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
    assert_eq!(snapshot.generation(), generation);
    assert_eq!(snapshot.len(), captured_length);
}

#[test]
fn reopen_has_a_new_generation_and_requires_a_separate_index() {
    let fixture = TestFile::from_bytes(b"one\ntwo").unwrap();
    let first_snapshot = snapshot(&fixture);
    let first = LineIndex::build(Arc::clone(&first_snapshot), options(2, 2)).unwrap();
    let second_snapshot = Arc::new(first_snapshot.reopen().unwrap());
    let second = LineIndex::build(Arc::clone(&second_snapshot), options(2, 2)).unwrap();

    assert_ne!(first.generation(), second.generation());
    assert!(Arc::ptr_eq(first.snapshot(), &first_snapshot));
    assert!(Arc::ptr_eq(second.snapshot(), &second_snapshot));
    assert!(!Arc::ptr_eq(first.snapshot(), second.snapshot()));
}

#[test]
fn ready_metadata_remains_inspectable_after_snapshot_becomes_stale() {
    let fixture = TestFile::from_bytes(b"one\ntwo").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    let index = LineIndex::build(Arc::clone(&snapshot), options(2, 2)).unwrap();
    let generation = index.generation();

    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"growth").unwrap();
    writer.flush().unwrap();
    assert!(matches!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(_)
    ));

    assert_eq!(index.generation(), generation);
    assert_eq!(index.snapshot_length().get(), 7);
    assert_eq!(index.line_count(), 2);
    assert_eq!(index.final_stride(), 256);
    assert_eq!(index.checkpoint_count(), 1);
}

#[test]
fn ready_index_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LineIndex>();
    assert_send_sync::<BuildProgress>();
}
