#[allow(dead_code)]
#[path = "../benches/support/mod.rs"]
mod benchmark_support;

use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;
use std::thread;

use faultsift_file_access::{BackendKind, SnapshotState};
use faultsift_line_access::{BuildControl, LineIndex, LineNumber, LineRange, LineSpan};

use benchmark_support::{
    BASELINE_CHECKPOINT_BUDGET_BYTES, BackendMode, BenchmarkConfig, CHECKPOINT_BUDGET_CANDIDATES,
    FIXTURE_WRITE_BUFFER_BYTES, FixtureFile, FixtureSpec, KIB, MIB,
    PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES, RANDOM_SEED, SCAN_CHUNK_CANDIDATES, TerminatorStyle,
    bounded_mib_override, cancellation_at_callback, criterion_group_identity, empty_ranges,
    generate_line_numbers, index_evidence, index_options, open_snapshot, prepare_line_requests,
    prepare_range_requests, run_cursor, run_line_requests, run_range_requests, seeded_ranges,
    stream_file_checksum, visit_line, visit_span,
};

#[test]
fn criterion_group_identity_is_complete_unique_and_not_truncated() {
    let base = BenchmarkConfig {
        smoke: false,
        representative_bytes: 16 * MIB,
        huge_line_bytes: 16 * MIB,
        lookup_requests: 256,
    };
    let identity = criterion_group_identity("line", BackendMode::Automatic, base);
    assert!(identity.len() <= 64);
    assert!(identity.contains("c0.7.0"));
    assert!(identity.contains("f16-h16-q256"));
    assert!(identity.contains(&format!("s{RANDOM_SEED:016x}")));
    assert!(identity.ends_with("-warm"));

    #[cfg(windows)]
    assert!(identity.contains("-map-"));
    #[cfg(target_os = "linux")]
    assert!(identity.contains("-buf-"));

    for changed in [
        BenchmarkConfig {
            smoke: true,
            ..base
        },
        BenchmarkConfig {
            representative_bytes: 32 * MIB,
            ..base
        },
        BenchmarkConfig {
            huge_line_bytes: 32 * MIB,
            ..base
        },
        BenchmarkConfig {
            lookup_requests: 512,
            ..base
        },
    ] {
        assert_ne!(
            identity,
            criterion_group_identity("line", BackendMode::Automatic, changed)
        );
    }
    assert_ne!(
        identity,
        criterion_group_identity("rng", BackendMode::Automatic, base)
    );

    #[cfg(windows)]
    {
        let forced = criterion_group_identity("line", BackendMode::ForcedBuffered, base);
        assert!(forced.contains("-fbuf-"));
        assert_ne!(identity, forced);
    }

    let maximum = BenchmarkConfig {
        smoke: false,
        representative_bytes: 512 * MIB,
        huge_line_bytes: 256 * MIB,
        lookup_requests: 16_384,
    };
    assert!(criterion_group_identity("line", BackendMode::Automatic, maximum).len() <= 64);
    #[cfg(windows)]
    assert!(criterion_group_identity("line", BackendMode::ForcedBuffered, maximum).len() <= 64);
}

#[test]
fn representative_fixtures_are_streamed_deterministic_and_seeded() {
    assert_eq!(FIXTURE_WRITE_BUFFER_BYTES, 64 * KIB as usize);
    for average in [80, 200, 500] {
        for terminator in [TerminatorStyle::Lf, TerminatorStyle::CrLf] {
            let spec = FixtureSpec::representative(average, terminator, 256 * KIB, RANDOM_SEED);
            let first = FixtureFile::generate(&spec).unwrap();
            let second = FixtureFile::generate(&spec).unwrap();
            assert_eq!(first.metadata(), second.metadata());
            assert_streams_equal(first.path(), second.path()).unwrap();
            let (bytes, checksum) = stream_file_checksum(first.path()).unwrap();
            assert_eq!(bytes, first.metadata().length);
            assert_eq!(checksum, first.metadata().physical_checksum);
            assert!((first.metadata().average_physical_bytes() - average as f64).abs() < 0.01);

            let different_seed = FixtureFile::generate(&FixtureSpec::representative(
                average,
                terminator,
                256 * KIB,
                RANDOM_SEED ^ 1,
            ))
            .unwrap();
            assert_ne!(
                first.metadata().physical_checksum,
                different_seed.metadata().physical_checksum
            );
        }
    }
}

#[test]
fn seeded_line_and_range_sequences_are_reproducible_and_seed_sensitive() {
    let first = generate_line_numbers(10_000, 512, RANDOM_SEED);
    let second = generate_line_numbers(10_000, 512, RANDOM_SEED);
    let different = generate_line_numbers(10_000, 512, RANDOM_SEED ^ 1);
    assert_eq!(first, second);
    assert_ne!(first, different);

    let first = seeded_ranges(10_000, 512, 17, RANDOM_SEED);
    let second = seeded_ranges(10_000, 512, 17, RANDOM_SEED);
    let different = seeded_ranges(10_000, 512, 17, RANDOM_SEED ^ 1);
    assert_eq!(first, second);
    assert_ne!(first, different);

    let first = empty_ranges(10_000, 512, RANDOM_SEED);
    let second = empty_ranges(10_000, 512, RANDOM_SEED);
    let different = empty_ranges(10_000, 512, RANDOM_SEED ^ 1);
    assert_eq!(first, second);
    assert_ne!(first, different);
}

#[test]
fn bounded_smoke_builds_cover_distributions_and_resource_candidates() {
    let mut specs = Vec::new();
    for average in [80, 200, 500] {
        for terminator in [TerminatorStyle::Lf, TerminatorStyle::CrLf] {
            specs.push(FixtureSpec::representative(
                average,
                terminator,
                256 * KIB,
                RANDOM_SEED,
            ));
        }
    }
    specs.push(FixtureSpec::newline_dense(256 * KIB, RANDOM_SEED));
    specs.push(FixtureSpec::huge_line(256 * KIB + 37, RANDOM_SEED));

    for spec in &specs {
        let fixture = FixtureFile::generate(spec).unwrap();
        let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
        let checkpoint_budget = if spec.name == "newline-dense-lf" {
            PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES
        } else {
            CHECKPOINT_BUDGET_CANDIDATES[0]
        };
        let index = LineIndex::build(
            Arc::clone(&snapshot),
            index_options(checkpoint_budget, SCAN_CHUNK_CANDIDATES[0]),
        )
        .unwrap();
        assert_eq!(index.line_count(), fixture.metadata().line_count);
        assert_eq!(index.snapshot_length().get(), fixture.metadata().length);
        let evidence = index_evidence(&index);
        assert!(evidence.checkpoint_count <= evidence.checkpoint_capacity);
        assert_eq!(
            evidence.checkpoint_payload_bytes,
            evidence.checkpoint_count * 8
        );
        assert_eq!(
            evidence.checkpoint_capacity_bytes,
            evidence.checkpoint_capacity * 8
        );
    }

    let fixture = FixtureFile::generate(&FixtureSpec::representative(
        200,
        TerminatorStyle::Lf,
        256 * KIB,
        RANDOM_SEED,
    ))
    .unwrap();
    let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    for checkpoint_budget in CHECKPOINT_BUDGET_CANDIDATES {
        for scan_chunk in SCAN_CHUNK_CANDIDATES {
            let index = LineIndex::build(
                Arc::clone(&snapshot),
                index_options(checkpoint_budget, scan_chunk),
            )
            .unwrap();
            assert_eq!(index.line_count(), fixture.metadata().line_count);
            assert_eq!(index.checkpoint_budget_bytes().get(), checkpoint_budget);
            assert_eq!(index.scan_chunk_bytes().get(), scan_chunk);
        }
    }
}

#[test]
fn cursor_lookup_range_and_readers_consume_exact_streamed_bytes() {
    let fixture = FixtureFile::generate(&FixtureSpec::representative(
        80,
        TerminatorStyle::CrLf,
        512 * KIB,
        RANDOM_SEED,
    ))
    .unwrap();
    let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    let cursor = run_cursor(Arc::clone(&snapshot), 4 * KIB).unwrap();
    assert_eq!(cursor.lines, fixture.metadata().line_count);
    assert_eq!(cursor.physical_bytes, fixture.metadata().length);
    assert_eq!(cursor.content_bytes, fixture.metadata().content_bytes);
    assert_eq!(cursor.checksum, fixture.metadata().content_checksum);

    let index = LineIndex::build(
        snapshot,
        index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, 4 * KIB),
    )
    .unwrap();
    let numbers = generate_line_numbers(index.line_count(), 256, RANDOM_SEED);
    let requests = prepare_line_requests(&index, &numbers).unwrap();
    let lookup = run_line_requests(&index, &requests).unwrap();
    assert_eq!(lookup.operations, requests.len() as u64);
    assert!(lookup.scanned_lines >= lookup.operations);
    assert!(lookup.scanned_bytes >= lookup.operations);

    let ranges = seeded_ranges(index.line_count(), 128, 257, RANDOM_SEED);
    let ranges = prepare_range_requests(&index, &ranges).unwrap();
    assert_ne!(run_range_requests(&index, &ranges).unwrap(), 0);

    let descriptor = index.line(LineNumber::new(17)).unwrap();
    let line = visit_line(&index, &descriptor).unwrap();
    assert_eq!(
        line.content_bytes,
        descriptor.content_range().length().get()
    );
    assert!(line.chunks >= 1);

    let span = index
        .line_range(LineRange::new(LineNumber::new(10), LineNumber::new(1_010)).unwrap())
        .unwrap();
    let visited = visit_span(&index, &span).unwrap();
    assert_eq!(visited.physical_bytes, span.physical_range().length().get());
    assert_eq!(visited.lines, 1_000);
    assert_eq!(
        std::mem::size_of_val(&span),
        std::mem::size_of::<LineSpan>()
    );
    assert!(std::mem::size_of::<LineSpan>() <= 64);
}

#[test]
fn newline_dense_progress_depends_on_chunks_not_line_count_and_cancel_is_bounded() {
    let fixture_bytes = 256 * KIB;
    let dense =
        FixtureFile::generate(&FixtureSpec::newline_dense(fixture_bytes, RANDOM_SEED)).unwrap();
    let huge =
        FixtureFile::generate(&FixtureSpec::huge_line(fixture_bytes - 1, RANDOM_SEED)).unwrap();
    assert_eq!(dense.len(), huge.len());
    assert_ne!(dense.metadata().line_count, huge.metadata().line_count);

    let scan_chunk = 4 * KIB;
    for fixture in [&dense, &huge] {
        let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
        let mut callbacks = 0_u64;
        let index = LineIndex::build_with_control(
            Arc::clone(&snapshot),
            index_options(PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES, scan_chunk),
            |_| {
                callbacks += 1;
                BuildControl::Continue
            },
        )
        .unwrap();
        assert_eq!(callbacks, fixture.len().div_ceil(scan_chunk));
        assert_eq!(index.line_count(), fixture.metadata().line_count);

        let cancellation = cancellation_at_callback(
            Arc::clone(&snapshot),
            PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES,
            scan_chunk,
            2,
        )
        .unwrap();
        assert_eq!(cancellation.callback_count, 2);
        assert_eq!(cancellation.bytes_scanned, 2 * scan_chunk);
        assert_eq!(snapshot.state(), SnapshotState::Fresh);
    }
}

#[test]
fn huge_line_stays_many_chunk_and_does_not_require_owned_materialization() {
    let fixture =
        FixtureFile::generate(&FixtureSpec::huge_line(2 * 1_024 * KIB + 37, RANDOM_SEED)).unwrap();
    let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    let scan_chunk = 4 * KIB;
    let cursor = run_cursor(Arc::clone(&snapshot), scan_chunk).unwrap();
    assert_eq!(cursor.lines, 1);
    assert_eq!(cursor.content_bytes, fixture.metadata().content_bytes);
    assert!(cursor.chunks > 500);

    let index = LineIndex::build(
        snapshot,
        index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, scan_chunk),
    )
    .unwrap();
    let descriptor = index.line(LineNumber::new(0)).unwrap();
    let reader = visit_line(&index, &descriptor).unwrap();
    assert_eq!(reader.content_bytes, fixture.metadata().content_bytes);
    assert_eq!(reader.checksum, fixture.metadata().content_checksum);
    assert!(reader.chunks > 500);
}

#[test]
fn bounded_concurrent_lookup_releases_all_worker_state() {
    const CONCURRENCY: usize = 4;
    let fixture = FixtureFile::generate(&FixtureSpec::representative(
        200,
        TerminatorStyle::Lf,
        256 * KIB,
        RANDOM_SEED,
    ))
    .unwrap();
    let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    let index = Arc::new(
        LineIndex::build(
            snapshot,
            index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, 4 * KIB),
        )
        .unwrap(),
    );
    let numbers = generate_line_numbers(index.line_count(), 64, RANDOM_SEED);
    let requests = Arc::new(prepare_line_requests(&index, &numbers).unwrap());
    let mut workers = Vec::with_capacity(CONCURRENCY);
    for _ in 0..CONCURRENCY {
        let index = Arc::clone(&index);
        let requests = Arc::clone(&requests);
        workers.push(thread::spawn(move || {
            run_line_requests(&index, &requests).unwrap()
        }));
    }
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(Arc::strong_count(&index), 1);
    assert_eq!(Arc::strong_count(&requests), 1);
}

#[test]
fn hard_fixture_overrides_reject_accidental_unbounded_values() {
    assert_eq!(bounded_mib_override(None, 16, 1, 512, "fixture"), Ok(16));
    assert_eq!(
        bounded_mib_override(Some("64"), 16, 1, 512, "fixture"),
        Ok(64)
    );
    assert!(bounded_mib_override(Some("0"), 16, 1, 512, "fixture").is_err());
    assert!(bounded_mib_override(Some("513"), 16, 1, 512, "fixture").is_err());
    assert!(bounded_mib_override(Some("huge"), 16, 1, 512, "fixture").is_err());
}

#[cfg(windows)]
#[test]
fn windows_modes_report_actual_mapped_and_forced_buffered_diagnostics() {
    use faultsift_file_access::MappingFallbackReason;

    let fixture = FixtureFile::generate(&FixtureSpec::representative(
        80,
        TerminatorStyle::Lf,
        256 * KIB,
        RANDOM_SEED,
    ))
    .unwrap();
    let forced = open_snapshot(fixture.path(), BackendMode::ForcedBuffered).unwrap();
    assert_eq!(forced.diagnostics().backend(), BackendKind::Buffered);
    assert_eq!(
        forced.diagnostics().mapping_fallback_reason(),
        Some(MappingFallbackReason::IncompatibleWriter)
    );
    drop(forced);

    let automatic = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    assert_eq!(automatic.diagnostics().backend(), BackendKind::Mapped);
    assert_eq!(automatic.diagnostics().mapping_fallback_reason(), None);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_benchmark_mode_reports_buffered_backend() {
    let fixture = FixtureFile::generate(&FixtureSpec::representative(
        80,
        TerminatorStyle::Lf,
        256 * KIB,
        RANDOM_SEED,
    ))
    .unwrap();
    let snapshot = open_snapshot(fixture.path(), BackendMode::Automatic).unwrap();
    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);
}

fn assert_streams_equal(
    first_path: &std::path::Path,
    second_path: &std::path::Path,
) -> io::Result<()> {
    let mut first = File::open(first_path)?;
    let mut second = File::open(second_path)?;
    let mut first_buffer = [0_u8; 8 * KIB as usize];
    let mut second_buffer = [0_u8; 8 * KIB as usize];
    loop {
        let first_read = first.read(&mut first_buffer)?;
        let second_read = second.read(&mut second_buffer)?;
        if first_read != second_read || first_buffer[..first_read] != second_buffer[..second_read] {
            return Err(io::Error::other("deterministic fixture bytes differ"));
        }
        if first_read == 0 {
            return Ok(());
        }
    }
}
