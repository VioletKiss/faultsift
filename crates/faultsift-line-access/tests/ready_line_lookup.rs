mod support;

use std::convert::Infallible;
use std::fmt;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
use std::thread;

use faultsift_file_access::{
    ByteLength, FileAccessError, SnapshotState, SnapshotValidation, StaleReason,
};
use faultsift_line_access::{
    LineAccessError, LineDescriptor, LineIndex, LineIndexOptions, LineNumber, LineRange, LineSpan,
    VisitBytesError,
};

use support::{TestFile, cursor, snapshot};

fn options(max_checkpoints: u64, scan_chunk_bytes: u64) -> LineIndexOptions {
    let checkpoint_bytes = std::mem::size_of::<u64>() as u64;
    LineIndexOptions::new(
        ByteLength::new(max_checkpoints * checkpoint_bytes),
        ByteLength::new(scan_chunk_bytes),
    )
    .unwrap()
}

fn collect_descriptors(
    snapshot: Arc<faultsift_file_access::FileSnapshot>,
    scan_chunk_bytes: u64,
) -> Vec<LineDescriptor> {
    let mut cursor = cursor(snapshot, scan_chunk_bytes);
    let mut descriptors = Vec::new();
    while let Some(descriptor) = cursor
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap()
    {
        descriptors.push(descriptor);
    }
    descriptors
}

fn range(start: u64, end: u64) -> LineRange {
    LineRange::new(LineNumber::new(start), LineNumber::new(end)).unwrap()
}

fn assert_chunk_ranges(ranges: &[faultsift_file_access::ByteRange], start: u64, end: u64) {
    let mut next = start;
    for chunk in ranges {
        assert!(!chunk.is_empty());
        assert_eq!(chunk.offset().get(), next);
        next = chunk.end().get();
    }
    assert_eq!(next, end);
}

#[test]
fn exact_lookup_matches_sequential_cursor_for_approved_byte_cases() {
    let cases: &[&[u8]] = &[
        b"",
        b"\n",
        b"\n\n",
        b"first\nmiddle\nlast",
        b"first\r\nmiddle\r\nlast\r\n",
        b"a\rX\n\r\r\nend\r",
        &[0xff, 0x00, b'X', b'\n', 0x80, b'\r', b'\n', 0x00],
    ];

    for bytes in cases {
        for chunk_bytes in 1..=7 {
            let fixture = TestFile::from_bytes(bytes).unwrap();
            let snapshot = snapshot(&fixture);
            let expected = collect_descriptors(Arc::clone(&snapshot), chunk_bytes);
            let index = LineIndex::build(Arc::clone(&snapshot), options(8, chunk_bytes)).unwrap();

            assert_eq!(index.line_count(), expected.len() as u64, "input={bytes:?}");
            for (number, expected) in expected.iter().copied().enumerate() {
                assert_eq!(
                    index.line(LineNumber::new(number as u64)).unwrap(),
                    expected
                );
            }

            let at_count = index.line(LineNumber::new(index.line_count())).unwrap_err();
            assert!(matches!(
                at_count,
                LineAccessError::LineNumberOutOfBounds {
                    line_number,
                    line_count
                } if line_number.get() == line_count && line_count == index.line_count()
            ));
            assert!(matches!(
                index.line(LineNumber::new(u64::MAX)),
                Err(LineAccessError::LineNumberOutOfBounds { .. })
            ));
        }
    }
}

#[test]
fn checkpoint_boundaries_and_adaptive_compaction_remain_exact() {
    let mut bytes = Vec::new();
    for number in 0..4_101_u64 {
        bytes.extend_from_slice(format!("line-{number}\n").as_bytes());
    }
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let snapshot = snapshot(&fixture);
    let expected = collect_descriptors(Arc::clone(&snapshot), 19);

    for max_checkpoints in [2, 3, 5, 32] {
        let index = LineIndex::build(Arc::clone(&snapshot), options(max_checkpoints, 19)).unwrap();
        assert!(index.final_stride() >= 256);
        let stride = index.final_stride();
        let mut targets = vec![0, 1, stride.saturating_sub(1), stride, stride + 1, 4_100];
        targets.retain(|target| *target < index.line_count());
        targets.sort_unstable();
        targets.dedup();
        for target in targets {
            assert_eq!(
                index.line(LineNumber::new(target)).unwrap(),
                expected[target as usize]
            );
        }
    }
}

#[test]
fn line_ranges_have_exact_physical_bounds_and_empty_anchors() {
    let bytes = b"zero\r\none\n\nthree";
    let fixture = TestFile::from_bytes(bytes).unwrap();
    let snapshot = snapshot(&fixture);
    let index = LineIndex::build(Arc::clone(&snapshot), options(4, 2)).unwrap();
    let lines = collect_descriptors(snapshot, 2);
    assert_eq!(lines.len(), 4);

    for (start, end) in [(0, 0), (0, 1), (0, 4), (1, 1), (1, 3), (3, 4), (4, 4)] {
        let span = index.line_range(range(start, end)).unwrap();
        assert_eq!(span.generation(), index.generation());
        assert_eq!(span.line_range(), range(start, end));
        let expected_start = if start == index.line_count() {
            index.snapshot_length().get()
        } else {
            lines[start as usize].physical_range().offset().get()
        };
        let expected_end = if start == end {
            expected_start
        } else {
            lines[(end - 1) as usize].physical_range().end().get()
        };
        assert_eq!(span.physical_range().offset().get(), expected_start);
        assert_eq!(span.physical_range().end().get(), expected_end);
    }

    assert!(matches!(
        LineRange::new(LineNumber::new(3), LineNumber::new(2)),
        Err(LineAccessError::InvalidLineRange { .. })
    ));
    assert!(matches!(
        index.line_range(range(0, 5)),
        Err(LineAccessError::LineRangeOutOfBounds { line_count: 4, .. })
    ));
}

#[test]
fn empty_file_accepts_only_the_eof_empty_span() {
    let fixture = TestFile::from_bytes(b"").unwrap();
    let index = LineIndex::build(snapshot(&fixture), options(2, 3)).unwrap();
    let span = index.line_range(range(0, 0)).unwrap();
    assert!(span.physical_range().is_empty());
    assert_eq!(span.physical_range().offset().get(), 0);
    assert!(matches!(
        index.line(LineNumber::new(0)),
        Err(LineAccessError::LineNumberOutOfBounds { line_count: 0, .. })
    ));
    assert!(matches!(
        index.line_range(range(0, 1)),
        Err(LineAccessError::LineRangeOutOfBounds { line_count: 0, .. })
    ));
}

#[test]
fn bounded_readers_separate_content_from_raw_physical_bytes() {
    let bytes = b"alpha\r\nbeta\n\rX\nlast";
    let fixture = TestFile::from_bytes(bytes).unwrap();
    let index = LineIndex::build(snapshot(&fixture), options(4, 3)).unwrap();

    let line = index.line(LineNumber::new(0)).unwrap();
    let mut content = Vec::new();
    let mut content_ranges = Vec::new();
    index
        .visit_line_content(&line, |chunk| {
            assert!(chunk.bytes().len() <= 3);
            content.extend_from_slice(chunk.bytes());
            content_ranges.push(chunk.range());
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(content, b"alpha");
    assert_chunk_ranges(
        &content_ranges,
        line.content_range().offset().get(),
        line.content_range().end().get(),
    );

    let span = index.line_range(range(0, 3)).unwrap();
    let mut physical = Vec::new();
    let mut physical_ranges = Vec::new();
    index
        .visit_span_physical(&span, |chunk| {
            assert!(chunk.bytes().len() <= 3);
            physical.extend_from_slice(chunk.bytes());
            physical_ranges.push(chunk.range());
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(physical, b"alpha\r\nbeta\n\rX\n");
    assert_chunk_ranges(
        &physical_ranges,
        span.physical_range().offset().get(),
        span.physical_range().end().get(),
    );
}

#[test]
fn empty_content_and_empty_spans_emit_no_chunks() {
    let fixture = TestFile::from_bytes(b"\nnext").unwrap();
    let index = LineIndex::build(snapshot(&fixture), options(2, 1)).unwrap();
    let empty_line = index.line(LineNumber::new(0)).unwrap();
    let mut line_calls = 0;
    index
        .visit_line_content(&empty_line, |_| {
            line_calls += 1;
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(line_calls, 0);

    for empty in [range(0, 0), range(1, 1), range(2, 2)] {
        let span = index.line_range(empty).unwrap();
        let mut span_calls = 0;
        index
            .visit_span_physical(&span, |_| {
                span_calls += 1;
                Ok::<(), Infallible>(())
            })
            .unwrap();
        assert_eq!(span_calls, 0);
    }
}

#[test]
fn huge_line_and_span_use_many_bounded_chunks() {
    const LINE_BYTES: u64 = 2 * 1024 * 1024 + 37;
    const CHUNK_BYTES: u64 = 257;
    let fixture = TestFile::streamed_line(LINE_BYTES).unwrap();
    let index = LineIndex::build(snapshot(&fixture), options(2, CHUNK_BYTES)).unwrap();
    let descriptor = index.line(LineNumber::new(0)).unwrap();
    assert_eq!(descriptor.content_range().length().get(), LINE_BYTES);
    assert_eq!(descriptor.physical_range().length().get(), LINE_BYTES + 1);

    let mut content_calls = 0_u64;
    let mut content_bytes = 0_u64;
    index
        .visit_line_content(&descriptor, |chunk| {
            assert!(chunk.bytes().len() as u64 <= CHUNK_BYTES);
            assert!(chunk.bytes().iter().all(|byte| *byte == 0x80));
            content_calls += 1;
            content_bytes += chunk.bytes().len() as u64;
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert!(content_calls > 8_000);
    assert_eq!(content_bytes, LINE_BYTES);

    let span = index.line_range(range(0, 1)).unwrap();
    let mut physical_calls = 0_u64;
    let mut physical_bytes = 0_u64;
    let mut final_byte = None;
    index
        .visit_span_physical(&span, |chunk| {
            assert!(chunk.bytes().len() as u64 <= CHUNK_BYTES);
            physical_calls += 1;
            physical_bytes += chunk.bytes().len() as u64;
            final_byte = chunk.bytes().last().copied();
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert!(physical_calls > 8_000);
    assert_eq!(physical_bytes, LINE_BYTES + 1);
    assert_eq!(final_byte, Some(b'\n'));
}

#[derive(Debug, Eq, PartialEq)]
enum VisitorStop {
    Deliberate,
}

impl fmt::Display for VisitorStop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("deliberate stop")
    }
}

impl std::error::Error for VisitorStop {}

#[test]
fn visitor_failure_is_typed_and_does_not_poison_later_operations() {
    let fixture = TestFile::from_bytes(b"abcdefgh\nsecond\n").unwrap();
    let snapshot = snapshot(&fixture);
    let index = LineIndex::build(Arc::clone(&snapshot), options(2, 2)).unwrap();
    let descriptor = index.line(LineNumber::new(0)).unwrap();
    let span = index.line_range(range(0, 2)).unwrap();

    let mut content_calls = 0;
    let error = index
        .visit_line_content(&descriptor, |_| {
            content_calls += 1;
            if content_calls == 2 {
                Err(VisitorStop::Deliberate)
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert!(matches!(
        error,
        VisitBytesError::Visitor(VisitorStop::Deliberate)
    ));
    assert_eq!(snapshot.state(), SnapshotState::Fresh);

    let span_error = index
        .visit_span_physical(&span, |_| Err(VisitorStop::Deliberate))
        .unwrap_err();
    assert!(matches!(
        span_error,
        VisitBytesError::Visitor(VisitorStop::Deliberate)
    ));
    assert_eq!(snapshot.state(), SnapshotState::Fresh);

    let mut later = Vec::new();
    index
        .visit_line_content(&descriptor, |chunk| {
            later.extend_from_slice(chunk.bytes());
            Ok::<(), VisitorStop>(())
        })
        .unwrap();
    assert_eq!(later, b"abcdefgh");
    assert_eq!(
        index.line(LineNumber::new(1)).unwrap().line_number().get(),
        1
    );
}

#[test]
fn reopened_generation_rejects_old_descriptor_and_span_before_reading() {
    let fixture = TestFile::from_bytes(b"one\ntwo\n").unwrap();
    let first_snapshot = snapshot(&fixture);
    let first = LineIndex::build(Arc::clone(&first_snapshot), options(2, 2)).unwrap();
    let old_descriptor = first.line(LineNumber::new(0)).unwrap();
    let old_span = first.line_range(range(0, 2)).unwrap();

    let second_snapshot = Arc::new(first_snapshot.reopen().unwrap());
    let second = LineIndex::build(Arc::clone(&second_snapshot), options(2, 2)).unwrap();
    assert_ne!(first.generation(), second.generation());

    assert!(matches!(
        second.visit_line_content(&old_descriptor, |_| Ok::<(), Infallible>(())),
        Err(VisitBytesError::LineAccess(
            LineAccessError::DescriptorGenerationMismatch { .. }
        ))
    ));
    assert!(matches!(
        second.visit_span_physical(&old_span, |_| Ok::<(), Infallible>(())),
        Err(VisitBytesError::LineAccess(
            LineAccessError::SpanGenerationMismatch { .. }
        ))
    ));
    assert_eq!(second_snapshot.state(), SnapshotState::Fresh);
}

#[test]
fn stale_ready_index_rejects_lookups_and_readers_but_keeps_metadata() {
    let fixture = TestFile::from_bytes(b"one\ntwo\n").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    let index = LineIndex::build(Arc::clone(&snapshot), options(2, 2)).unwrap();
    let descriptor = index.line(LineNumber::new(0)).unwrap();
    let span = index.line_range(range(0, 2)).unwrap();
    let generation = index.generation();

    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"growth").unwrap();
    writer.flush().unwrap();
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    assert_eq!(index.generation(), generation);
    assert_eq!(index.line_count(), 2);
    assert_eq!(index.snapshot_length().get(), 8);
    assert_eq!(index.final_stride(), 256);
    assert_eq!(index.checkpoint_count(), 1);
    assert!(matches!(
        index.line(LineNumber::new(0)),
        Err(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
    assert!(matches!(
        index.line_range(range(2, 2)),
        Err(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
    assert!(matches!(
        index.visit_line_content(&descriptor, |_| Ok::<(), Infallible>(())),
        Err(VisitBytesError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        )))
    ));
    assert!(matches!(
        index.visit_span_physical(&span, |_| Ok::<(), Infallible>(())),
        Err(VisitBytesError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        )))
    ));
}

#[test]
fn reader_propagates_unexpected_eof_and_preserves_file_access_stale_rules() {
    let fixture = TestFile::from_bytes(b"abcdef").unwrap();
    let writer = fixture.write_handle().unwrap();
    let snapshot = snapshot(&fixture);
    let index = LineIndex::build(Arc::clone(&snapshot), options(2, 4)).unwrap();
    let descriptor = index.line(LineNumber::new(0)).unwrap();
    writer.set_len(1).unwrap();

    let error = index
        .visit_line_content(&descriptor, |_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(matches!(
        error,
        VisitBytesError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::UnexpectedEof { .. }
        ))
    ));
    assert_eq!(
        snapshot.state(),
        SnapshotState::Stale(StaleReason::UnexpectedEof)
    );
}

#[test]
fn large_line_range_returns_one_constant_sized_span() {
    const LINE_COUNT: usize = 100_000;
    let fixture = TestFile::from_bytes(&vec![b'\n'; LINE_COUNT]).unwrap();
    let index = LineIndex::build(snapshot(&fixture), options(3, 1_023)).unwrap();
    let span = index.line_range(range(0, LINE_COUNT as u64)).unwrap();

    assert_eq!(span.line_range(), range(0, LINE_COUNT as u64));
    assert_eq!(span.physical_range().length().get(), LINE_COUNT as u64);
    assert_eq!(
        std::mem::size_of_val(&span),
        std::mem::size_of::<LineSpan>()
    );
    assert!(std::mem::size_of::<LineSpan>() <= 64);
}

#[test]
fn immutable_index_supports_parallel_lookup_range_and_read_operations() {
    let mut bytes = Vec::new();
    for number in 0..2_000_u64 {
        bytes.extend_from_slice(format!("line-{number:04}\r\n").as_bytes());
    }
    let fixture = TestFile::from_bytes(&bytes).unwrap();
    let index = Arc::new(LineIndex::build(snapshot(&fixture), options(3, 17)).unwrap());
    let mut workers = Vec::new();

    for _worker in 0..8_u64 {
        let index = Arc::clone(&index);
        workers.push(thread::spawn(move || {
            let mut checksum = 0_u64;
            for iteration in 0..200_u64 {
                let target = (iteration * 97) % index.line_count();
                let descriptor = index.line(LineNumber::new(target)).unwrap();
                assert_eq!(descriptor.line_number().get(), target);
                index
                    .visit_line_content(&descriptor, |chunk| {
                        checksum = checksum.wrapping_add(
                            chunk
                                .bytes()
                                .iter()
                                .map(|byte| u64::from(*byte))
                                .sum::<u64>(),
                        );
                        Ok::<(), Infallible>(())
                    })
                    .unwrap();
                let end = (target + 3).min(index.line_count());
                let span = index.line_range(range(target, end)).unwrap();
                index
                    .visit_span_physical(&span, |chunk| {
                        checksum = checksum.wrapping_add(chunk.bytes().len() as u64);
                        Ok::<(), Infallible>(())
                    })
                    .unwrap();
            }
            checksum
        }));
    }

    let first_run: Vec<u64> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(first_run.iter().all(|checksum| *checksum != 0));
    assert!(first_run.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(index.snapshot().state(), SnapshotState::Fresh);
}
