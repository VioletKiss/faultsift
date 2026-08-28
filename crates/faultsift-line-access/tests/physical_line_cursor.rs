mod support;

use std::convert::Infallible;
use std::fmt;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use faultsift_file_access::{
    ByteLength, ByteOffset, ByteRange, FileAccessError, FileAccessOptions, FileSnapshot,
    SnapshotState, SnapshotValidation, StaleReason,
};
use faultsift_line_access::{
    CursorFailure, CursorState, LineAccessError, LineDescriptor, LineTerminator,
    PhysicalLineCursor, ScanOptions, VisitLineError,
};

use support::{TestFile, cursor, snapshot};

#[derive(Debug, Eq, PartialEq)]
struct ObservedLine {
    descriptor: LineDescriptor,
    content: Vec<u8>,
    chunks: Vec<ByteRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceLine {
    content_start: u64,
    content_end: u64,
    physical_end: u64,
    terminator: LineTerminator,
    content: Vec<u8>,
}

fn collect(mut cursor: PhysicalLineCursor) -> Vec<ObservedLine> {
    let mut lines = Vec::new();
    loop {
        let mut content = Vec::new();
        let mut chunks = Vec::new();
        let descriptor = cursor
            .visit_next_line(|chunk| {
                content.extend_from_slice(chunk.bytes());
                chunks.push(chunk.range());
                Ok::<(), Infallible>(())
            })
            .unwrap();
        match descriptor {
            Some(descriptor) => lines.push(ObservedLine {
                descriptor,
                content,
                chunks,
            }),
            None => break,
        }
    }
    assert_eq!(cursor.state(), CursorState::Exhausted);
    lines
}

fn reference_lines(bytes: &[u8]) -> Vec<ReferenceLine> {
    let mut lines = Vec::new();
    let mut line_start = 0_usize;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let (content_end, terminator) = if index > line_start && bytes[index - 1] == b'\r' {
            (index - 1, LineTerminator::CrLf)
        } else {
            (index, LineTerminator::Lf)
        };
        lines.push(ReferenceLine {
            content_start: line_start as u64,
            content_end: content_end as u64,
            physical_end: (index + 1) as u64,
            terminator,
            content: bytes[line_start..content_end].to_vec(),
        });
        line_start = index + 1;
    }
    if line_start < bytes.len() {
        lines.push(ReferenceLine {
            content_start: line_start as u64,
            content_end: bytes.len() as u64,
            physical_end: bytes.len() as u64,
            terminator: LineTerminator::None,
            content: bytes[line_start..].to_vec(),
        });
    }
    lines
}

fn assert_matches_reference(bytes: &[u8], scan_chunk_bytes: u64) {
    let fixture = TestFile::from_bytes(bytes).unwrap();
    let snapshot = snapshot(&fixture);
    let generation = snapshot.generation();
    let actual = collect(cursor(snapshot, scan_chunk_bytes));
    let expected = reference_lines(bytes);
    assert_eq!(actual.len(), expected.len(), "input={bytes:?}");

    for (line_number, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        let descriptor = actual.descriptor;
        assert_eq!(descriptor.generation(), generation);
        assert_eq!(descriptor.line_number().get(), line_number as u64);
        assert_eq!(
            descriptor.content_range().offset().get(),
            expected.content_start
        );
        assert_eq!(descriptor.content_range().end().get(), expected.content_end);
        assert_eq!(
            descriptor.physical_range().offset(),
            descriptor.content_range().offset()
        );
        assert_eq!(
            descriptor.physical_range().end().get(),
            expected.physical_end
        );
        assert_eq!(descriptor.terminator(), expected.terminator);
        assert_eq!(actual.content, expected.content);
        assert_chunk_contract(actual, scan_chunk_bytes);
        assert_descriptor_invariants(descriptor);
    }
}

fn assert_chunk_contract(line: &ObservedLine, scan_chunk_bytes: u64) {
    let descriptor = line.descriptor;
    let mut expected_offset = descriptor.content_range().offset().get();
    for chunk in &line.chunks {
        assert!(!chunk.is_empty());
        assert!(chunk.length().get() <= scan_chunk_bytes);
        assert_eq!(chunk.offset().get(), expected_offset);
        expected_offset = chunk.end().get();
    }
    assert_eq!(expected_offset, descriptor.content_range().end().get());
    assert_eq!(
        line.content.len() as u64,
        descriptor.content_range().length().get()
    );
}

fn assert_descriptor_invariants(descriptor: LineDescriptor) {
    let content = descriptor.content_range();
    let physical = descriptor.physical_range();
    assert_eq!(physical.offset(), content.offset());
    let terminator_bytes = match descriptor.terminator() {
        LineTerminator::None => 0,
        LineTerminator::Lf => 1,
        LineTerminator::CrLf => 2,
    };
    assert_eq!(physical.end().get(), content.end().get() + terminator_bytes);
}

#[test]
fn approved_counting_and_newline_cases_are_exact() {
    for bytes in [
        b"".as_slice(),
        b"\n",
        b"\n\n",
        b"a\n",
        b"a\n\n",
        b"a",
        b"a\nb",
        b"\r",
        b"\r\n",
        b"a\r\n",
        b"a\r\n\r\n",
        b"\r\r",
        b"\r\rx",
    ] {
        for chunk_size in 1..=4 {
            assert_matches_reference(bytes, chunk_size);
        }
    }
}

#[test]
fn exact_required_line_counts_never_include_a_phantom_tail() {
    let cases: &[(&[u8], usize)] = &[
        (b"", 0),
        (b"\n", 1),
        (b"\n\n", 2),
        (b"a\n", 1),
        (b"a\n\n", 2),
        (b"a", 1),
        (b"\r", 1),
        (b"\r\n", 1),
    ];
    for (bytes, expected_count) in cases {
        let fixture = TestFile::from_bytes(bytes).unwrap();
        let lines = collect(cursor(snapshot(&fixture), 1));
        assert_eq!(lines.len(), *expected_count, "input={bytes:?}");
    }
}

#[test]
fn empty_lines_emit_zero_content_chunks() {
    let fixture = TestFile::from_bytes(b"\n\r\n\n").unwrap();
    let lines = collect(cursor(snapshot(&fixture), 1));
    assert_eq!(lines.len(), 3);
    for line in lines {
        assert!(line.content.is_empty());
        assert!(line.chunks.is_empty());
    }
}

#[test]
fn crlf_and_content_cross_every_small_chunk_boundary() {
    let cases: &[&[u8]] = &[
        b"\r\n",
        b"a\r\n",
        b"ab\r\n",
        b"abc\r\n",
        b"abcd\r\n",
        b"abcde\r\n",
        b"abc\ndef",
        b"abc\r",
        b"\r\r\n",
        b"\rX\n",
    ];
    for bytes in cases {
        for chunk_size in 1..=6 {
            assert_matches_reference(bytes, chunk_size);
        }
    }
}

#[test]
fn arbitrary_bytes_are_streamed_without_decoding_or_filtering() {
    let bytes = [0xff, 0x00, 0x80, b'\r', b'X', b'\n', 0xfe, 0x00, b'\r'];
    for chunk_size in 1..=5 {
        assert_matches_reference(&bytes, chunk_size);
    }
}

#[test]
fn exhaustive_short_inputs_match_the_reference_model() {
    const ALPHABET: [u8; 5] = [b'x', b'\r', b'\n', 0x00, 0xff];
    for length in 0_u32..=4 {
        let combinations = ALPHABET.len().pow(length);
        for mut ordinal in 0..combinations {
            let mut bytes = vec![0; length as usize];
            for byte in &mut bytes {
                *byte = ALPHABET[ordinal % ALPHABET.len()];
                ordinal /= ALPHABET.len();
            }
            let fixture = TestFile::from_bytes(&bytes).unwrap();
            let snapshot = snapshot(&fixture);
            let expected = reference_lines(&bytes);
            for chunk_size in 1..=3 {
                let actual = collect(cursor(Arc::clone(&snapshot), chunk_size));
                assert_eq!(actual.len(), expected.len(), "input={bytes:?}");
                for (line_number, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                    assert_eq!(actual.descriptor.line_number().get(), line_number as u64);
                    assert_eq!(
                        actual.descriptor.content_range().offset().get(),
                        expected.content_start
                    );
                    assert_eq!(
                        actual.descriptor.content_range().end().get(),
                        expected.content_end
                    );
                    assert_eq!(
                        actual.descriptor.physical_range().end().get(),
                        expected.physical_end
                    );
                    assert_eq!(actual.descriptor.terminator(), expected.terminator);
                    assert_eq!(actual.content, expected.content);
                    assert_chunk_contract(actual, chunk_size);
                }
            }
        }
    }
}

#[test]
fn huge_line_uses_many_bounded_chunks_and_one_descriptor() {
    const LINE_BYTES: u64 = 2 * 1024 * 1024 + 37;
    const CHUNK_BYTES: u64 = 257;
    let fixture = TestFile::streamed_line(LINE_BYTES).unwrap();
    let snapshot = snapshot(&fixture);
    let generation = snapshot.generation();
    let mut cursor = cursor(snapshot, CHUNK_BYTES);
    let mut chunk_count = 0_u64;
    let mut visited_bytes = 0_u64;
    let mut next_offset = 0_u64;

    let descriptor = cursor
        .visit_next_line(|chunk| {
            assert!(chunk.bytes().iter().all(|byte| *byte == 0x80));
            assert!(chunk.bytes().len() as u64 <= CHUNK_BYTES);
            assert_eq!(chunk.range().offset().get(), next_offset);
            next_offset = chunk.range().end().get();
            visited_bytes += chunk.bytes().len() as u64;
            chunk_count += 1;
            Ok::<(), Infallible>(())
        })
        .unwrap()
        .unwrap();

    assert!(chunk_count > 8_000);
    assert_eq!(visited_bytes, LINE_BYTES);
    assert_eq!(descriptor.generation(), generation);
    assert_eq!(descriptor.content_range().length().get(), LINE_BYTES);
    assert_eq!(descriptor.physical_range().length().get(), LINE_BYTES + 1);
    assert_eq!(descriptor.terminator(), LineTerminator::Lf);
    assert!(
        cursor
            .visit_next_line(|_| Ok::<(), Infallible>(()))
            .unwrap()
            .is_none()
    );
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
fn visitor_failure_preserves_type_and_terminally_fails_cursor() {
    let fixture = TestFile::from_bytes(b"abcdefgh\nnext\n").unwrap();
    let mut cursor = cursor(snapshot(&fixture), 2);
    let mut calls = 0;
    let error = cursor
        .visit_next_line(|_| {
            calls += 1;
            if calls == 2 {
                Err(VisitorStop::Deliberate)
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(calls, 2);
    assert!(matches!(
        error,
        VisitLineError::Visitor(VisitorStop::Deliberate)
    ));
    assert_eq!(cursor.state(), CursorState::Failed(CursorFailure::Visitor));

    let later = cursor
        .visit_next_line(|_| Ok::<(), VisitorStop>(()))
        .unwrap_err();
    assert!(matches!(
        later,
        VisitLineError::LineAccess(LineAccessError::CursorFailed {
            failure: CursorFailure::Visitor
        })
    ));
}

#[test]
fn stale_snapshot_read_is_propagated_and_cursor_becomes_terminal() {
    let fixture = TestFile::from_bytes(b"first\n").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot =
        Arc::new(FileSnapshot::open(fixture.path(), FileAccessOptions::default()).unwrap());
    let generation = snapshot.generation();
    let mut cursor = cursor(Arc::clone(&snapshot), 2);

    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"growth").unwrap();
    writer.flush().unwrap();
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    let error = cursor
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(matches!(
        error,
        VisitLineError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
    assert_eq!(cursor.generation(), generation);
    assert_eq!(cursor.state(), CursorState::Failed(CursorFailure::Read));
    assert!(matches!(
        cursor.visit_next_line(|_| Ok::<(), Infallible>(())),
        Err(VisitLineError::LineAccess(LineAccessError::CursorFailed {
            failure: CursorFailure::Read
        }))
    ));
}

#[test]
fn stale_transition_rejects_lines_already_prefetched_into_the_scan_buffer() {
    let fixture = TestFile::from_bytes(b"first\nsecond\n").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot =
        Arc::new(FileSnapshot::open(fixture.path(), FileAccessOptions::default()).unwrap());
    let mut cursor = cursor(Arc::clone(&snapshot), 64);

    let first = cursor
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap()
        .unwrap();
    assert_eq!(first.line_number().get(), 0);
    assert_eq!(first.physical_range().end().get(), 6);

    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"growth").unwrap();
    writer.flush().unwrap();
    assert_eq!(
        snapshot.validate().unwrap(),
        SnapshotValidation::Stale(StaleReason::Grown)
    );

    let error = cursor
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(matches!(
        error,
        VisitLineError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::StaleSnapshot {
                reason: StaleReason::Grown
            }
        ))
    ));
    assert_eq!(cursor.state(), CursorState::Failed(CursorFailure::Read));
}

#[test]
fn cursor_never_reads_past_its_captured_boundary() {
    let fixture = TestFile::from_bytes(b"a\n").unwrap();
    let mut writer = fixture.write_handle().unwrap();
    let snapshot =
        Arc::new(FileSnapshot::open(fixture.path(), FileAccessOptions::default()).unwrap());
    let mut cursor = cursor(Arc::clone(&snapshot), 1);
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"hidden\n").unwrap();
    writer.flush().unwrap();

    assert_eq!(cursor.captured_length().get(), 2);
    let mut content = Vec::new();
    let descriptor = cursor
        .visit_next_line(|chunk| {
            content.extend_from_slice(chunk.bytes());
            Ok::<(), Infallible>(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(content, b"a");
    assert_eq!(descriptor.physical_range().end().get(), 2);
    assert!(
        cursor
            .visit_next_line(|_| Ok::<(), Infallible>(()))
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot.state(), SnapshotState::Fresh);
}

#[test]
fn truncation_read_failure_returns_no_descriptor_and_is_terminal() {
    let fixture = TestFile::from_bytes(b"abcdef").unwrap();
    let writer = fixture.write_handle().unwrap();
    let snapshot =
        Arc::new(FileSnapshot::open(fixture.path(), FileAccessOptions::default()).unwrap());
    let mut cursor = cursor(Arc::clone(&snapshot), 4);
    writer.set_len(1).unwrap();

    let error = cursor
        .visit_next_line(|_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(matches!(
        error,
        VisitLineError::LineAccess(LineAccessError::FileAccess(
            FileAccessError::UnexpectedEof { .. }
        ))
    ));
    assert_eq!(
        snapshot.state(),
        SnapshotState::Stale(StaleReason::UnexpectedEof)
    );
    assert_eq!(cursor.state(), CursorState::Failed(CursorFailure::Read));
}

#[test]
fn cursor_configuration_fails_before_scanning() {
    let fixture = TestFile::from_bytes(b"data").unwrap();
    let snapshot = snapshot(&fixture);
    let invalid = ScanOptions::new(ByteLength::new(0)).unwrap_err();
    assert!(matches!(
        invalid,
        LineAccessError::InvalidScanChunkBytes { .. }
    ));

    let range = ByteRange::new(ByteOffset::new(0), ByteLength::new(4)).unwrap();
    assert_eq!(snapshot.view(range).unwrap().as_bytes(), b"data");
}
