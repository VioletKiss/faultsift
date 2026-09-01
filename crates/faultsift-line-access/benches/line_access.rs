mod support;

use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use faultsift_file_access::FileSnapshot;
use faultsift_line_access::{BuildControl, LineIndex, LineNumber, LineRange, LineSpan};
use support::{
    BASELINE_CHECKPOINT_BUDGET_BYTES, BASELINE_SCAN_CHUNK_BYTES, BENCHMARK_CONCURRENCY_LEVELS,
    BackendMode, BenchmarkConfig, CHECKPOINT_BUDGET_CANDIDATES, CRITERION_VERSION,
    CancellationEvidence, FixtureFile, FixtureSpec, IndexEvidence, LineRequest, MIB,
    PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES, RANDOM_SEED, RangeRequest, SCAN_CHUNK_CANDIDATES,
    backend_diagnostics, backend_modes, cancellation_at_callback, checkpoint_crossing_ranges,
    criterion_group_identity, empty_ranges, generate_line_numbers, index_evidence, index_options,
    open_snapshot, prepare_line_requests, prepare_range_requests, print_environment_metadata,
    print_memory, run_cursor, run_line_requests, run_range_requests, seeded_ranges,
    stream_file_checksum, visit_line, visit_span,
};

struct BenchContext {
    config: BenchmarkConfig,
    fixtures: Vec<FixtureFile>,
    newline_dense_index: usize,
    huge_line_index: usize,
}

impl BenchContext {
    fn create() -> Self {
        let config = BenchmarkConfig::from_environment();
        let mut specs = config.representative_specs();
        let newline_dense_index = specs.len();
        specs.push(FixtureSpec::newline_dense(
            config.representative_bytes,
            RANDOM_SEED,
        ));
        let huge_line_index = specs.len();
        specs.push(FixtureSpec::huge_line(config.huge_line_bytes, RANDOM_SEED));
        let fixtures: Vec<FixtureFile> = specs
            .iter()
            .map(|spec| {
                FixtureFile::generate(spec)
                    .unwrap_or_else(|error| panic!("fixture {} failed: {error}", spec.name))
            })
            .collect();
        for fixture in &fixtures {
            let (bytes, checksum) = stream_file_checksum(fixture.path())
                .expect("fixture checksum preflight must stream successfully");
            assert_eq!(bytes, fixture.metadata().length);
            assert_eq!(checksum, fixture.metadata().physical_checksum);
        }
        Self {
            config,
            fixtures,
            newline_dense_index,
            huge_line_index,
        }
    }

    fn profile(&self) -> String {
        format!(
            "criterion-{CRITERION_VERSION}-{}-representative-{}MiB-huge-{}MiB",
            if self.config.smoke { "smoke" } else { "full" },
            self.config.representative_bytes / MIB,
            self.config.huge_line_bytes / MIB
        )
    }

    fn group_identity(&self, api: &str, mode: BackendMode) -> String {
        criterion_group_identity(api, mode, self.config)
    }

    fn fixture_named(&self, name: &str) -> usize {
        self.fixtures
            .iter()
            .position(|fixture| fixture.metadata().name == name)
            .unwrap_or_else(|| panic!("fixture {name} must exist"))
    }
}

fn benchmark_line_access(criterion: &mut Criterion) {
    let context = BenchContext::create();
    print_environment_metadata(context.config, &context.fixtures[0]);
    println!("benchmark_profile={}", context.profile());
    for fixture in &context.fixtures {
        println!("[faultsift-line-benchmark-fixture] {}", fixture.metadata());
    }
    print_memory("fixtures-created");

    for &mode in backend_modes() {
        benchmark_backend(criterion, &context, mode);
    }

    print_memory("all-groups-finished");
}

fn benchmark_backend(criterion: &mut Criterion, context: &BenchContext, mode: BackendMode) {
    let snapshots: Vec<Arc<FileSnapshot>> = context
        .fixtures
        .iter()
        .map(|fixture| {
            open_snapshot(fixture.path(), mode).unwrap_or_else(|error| {
                panic!(
                    "backend {} unavailable for {}: {error}",
                    mode.label(),
                    fixture.path().display()
                )
            })
        })
        .collect();
    println!(
        "[faultsift-line-benchmark-backend] mode={} {}",
        mode.label(),
        backend_diagnostics(&snapshots[0])
    );
    print_memory(&format!("backend-{}-snapshots-open", mode.label()));

    benchmark_index_builds(criterion, context, mode, &snapshots);
    benchmark_resource_candidates(criterion, context, mode, &snapshots);
    benchmark_cursors(criterion, context, mode, &snapshots);
    benchmark_lookups_and_ranges(criterion, context, mode, &snapshots);
    benchmark_readers(criterion, context, mode, &snapshots);
    record_cancellation_evidence(context, mode, &snapshots);
    benchmark_concurrent_lookup(criterion, context, mode, &snapshots);
    print_memory(&format!("backend-{}-groups-finished", mode.label()));
    drop(snapshots);
}

fn benchmark_index_builds(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let mut group = criterion.benchmark_group(context.group_identity("ib", mode));
    for (fixture, snapshot) in context.fixtures.iter().zip(snapshots) {
        let started = Instant::now();
        let preflight = LineIndex::build(
            Arc::clone(snapshot),
            index_options(
                if fixture.metadata().name == "newline-dense-lf" {
                    PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES
                } else {
                    BASELINE_CHECKPOINT_BUDGET_BYTES
                },
                BASELINE_SCAN_CHUNK_BYTES,
            ),
        )
        .expect("index build preflight must succeed");
        let elapsed = started.elapsed();
        verify_index(fixture, &preflight);
        let evidence = index_evidence(&preflight);
        print_build_evidence(mode, fixture, elapsed, evidence, "coverage");
        drop(preflight);

        let checkpoint_budget = if fixture.metadata().name == "newline-dense-lf" {
            PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES
        } else {
            BASELINE_CHECKPOINT_BUDGET_BYTES
        };
        group.throughput(Throughput::Bytes(fixture.len()));
        group.bench_with_input(
            BenchmarkId::new(
                &fixture.metadata().name,
                format!(
                    "budget-{checkpoint_budget}-chunk-{BASELINE_SCAN_CHUNK_BYTES}-stride-{}",
                    evidence.final_stride
                ),
            ),
            snapshot,
            |bencher, snapshot| {
                let options = index_options(checkpoint_budget, BASELINE_SCAN_CHUNK_BYTES);
                bencher.iter_custom(|iterations| {
                    let mut measured = Duration::ZERO;
                    for _ in 0..iterations {
                        let snapshot = Arc::clone(snapshot);
                        let started = Instant::now();
                        let index = LineIndex::build(snapshot, options)
                            .expect("timed index build must succeed");
                        measured += started.elapsed();
                        black_box(&index);
                        drop(index);
                    }
                    measured
                });
            },
        );
    }
    group.finish();
}

fn benchmark_resource_candidates(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let fixture_index = context.fixture_named("lf-avg-200");
    let fixture = &context.fixtures[fixture_index];
    let snapshot = &snapshots[fixture_index];
    let mut group = criterion.benchmark_group(context.group_identity("rc", mode));
    group.throughput(Throughput::Bytes(fixture.len()));

    for checkpoint_budget in CHECKPOINT_BUDGET_CANDIDATES {
        for scan_chunk in SCAN_CHUNK_CANDIDATES {
            let started = Instant::now();
            let preflight = LineIndex::build(
                Arc::clone(snapshot),
                index_options(checkpoint_budget, scan_chunk),
            )
            .expect("candidate build preflight must succeed");
            let elapsed = started.elapsed();
            verify_index(fixture, &preflight);
            let evidence = index_evidence(&preflight);
            print_build_evidence(mode, fixture, elapsed, evidence, "resource-candidate");
            drop(preflight);

            group.bench_with_input(
                BenchmarkId::new(
                    format!("budget-{checkpoint_budget}"),
                    format!("chunk-{scan_chunk}-stride-{}", evidence.final_stride),
                ),
                snapshot,
                |bencher, snapshot| {
                    let options = index_options(checkpoint_budget, scan_chunk);
                    bencher.iter_custom(|iterations| {
                        let mut measured = Duration::ZERO;
                        for _ in 0..iterations {
                            let snapshot = Arc::clone(snapshot);
                            let started = Instant::now();
                            let index = LineIndex::build(snapshot, options)
                                .expect("timed candidate build must succeed");
                            measured += started.elapsed();
                            black_box(&index);
                            drop(index);
                        }
                        measured
                    });
                },
            );
        }
    }
    group.finish();
}

fn benchmark_cursors(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let mut group = criterion.benchmark_group(context.group_identity("cur", mode));
    for (fixture, snapshot) in context.fixtures.iter().zip(snapshots) {
        for scan_chunk in SCAN_CHUNK_CANDIDATES {
            let preflight = run_cursor(Arc::clone(snapshot), scan_chunk)
                .expect("cursor preflight must consume the fixture");
            assert_eq!(preflight.lines, fixture.metadata().line_count);
            assert_eq!(preflight.physical_bytes, fixture.metadata().length);
            assert_eq!(preflight.content_bytes, fixture.metadata().content_bytes);
            assert_eq!(preflight.checksum, fixture.metadata().content_checksum);
            assert_eq!(preflight.lf_lines, fixture.metadata().lf_lines);
            assert_eq!(preflight.crlf_lines, fixture.metadata().crlf_lines);
            println!(
                "[faultsift-line-benchmark-cursor] backend={} fixture={} scan_chunk_bytes={} physical_bytes={} content_bytes={} lines={} content_chunk_count={} scan_chunk_count={} lf={} crlf={} none={} checksum=0x{:016x}",
                mode.label(),
                fixture.metadata().name,
                scan_chunk,
                preflight.physical_bytes,
                preflight.content_bytes,
                preflight.lines,
                preflight.chunks,
                fixture.len().div_ceil(scan_chunk),
                preflight.lf_lines,
                preflight.crlf_lines,
                preflight.unterminated_lines,
                preflight.checksum
            );

            group.throughput(Throughput::Bytes(fixture.len()));
            group.bench_with_input(
                BenchmarkId::new(&fixture.metadata().name, format!("chunk-{scan_chunk}")),
                snapshot,
                |bencher, snapshot| {
                    bencher.iter_batched(
                        || Arc::clone(snapshot),
                        |snapshot| {
                            black_box(
                                run_cursor(snapshot, scan_chunk)
                                    .expect("timed cursor must consume the fixture"),
                            )
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }
    group.finish();
}

fn benchmark_lookups_and_ranges(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let lookup_profiles = [
        (
            context.fixture_named("lf-avg-80"),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
        ),
        (
            context.fixture_named("crlf-avg-200"),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
        ),
        (
            context.newline_dense_index,
            PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES,
        ),
        (context.huge_line_index, BASELINE_CHECKPOINT_BUDGET_BYTES),
    ];

    let mut lookup_group = criterion.benchmark_group(context.group_identity("line", mode));
    for (fixture_index, checkpoint_budget) in lookup_profiles {
        let fixture = &context.fixtures[fixture_index];
        let index = LineIndex::build(
            Arc::clone(&snapshots[fixture_index]),
            index_options(checkpoint_budget, BASELINE_SCAN_CHUNK_BYTES),
        )
        .expect("lookup index build must succeed");
        let request_count = if fixture_index == context.newline_dense_index
            || fixture_index == context.huge_line_index
        {
            context.config.lookup_requests.min(16)
        } else {
            context.config.lookup_requests
        };
        let numbers = generate_line_numbers(index.line_count(), request_count, RANDOM_SEED);
        let requests = prepare_line_requests(&index, &numbers)
            .expect("lookup request preparation must succeed");
        let expected = run_line_requests(&index, &requests).expect("lookup preflight must succeed");
        let latency = measure_line_latency(&index, &requests);
        println!(
            "[faultsift-line-benchmark-lookup] backend={} fixture={} requests={} final_stride={} checkpoint_budget_bytes={} scan_chunk_bytes={} scanned_lines={} scanned_bytes={} p50_ns={} p95_ns={} p99_ns={} checksum=0x{:016x}",
            mode.label(),
            fixture.metadata().name,
            expected.operations,
            index.final_stride(),
            checkpoint_budget,
            BASELINE_SCAN_CHUNK_BYTES,
            expected.scanned_lines,
            expected.scanned_bytes,
            latency.p50_ns,
            latency.p95_ns,
            latency.p99_ns,
            expected.checksum
        );
        lookup_group.throughput(Throughput::Elements(requests.len() as u64));
        lookup_group.bench_function(
            BenchmarkId::new(
                &fixture.metadata().name,
                format!(
                    "stride-{}-budget-{checkpoint_budget}-chunk-{BASELINE_SCAN_CHUNK_BYTES}",
                    index.final_stride()
                ),
            ),
            |bencher| {
                bencher.iter(|| {
                    black_box(
                        run_line_requests(black_box(&index), black_box(&requests))
                            .expect("timed lookup batch must remain exact"),
                    )
                });
            },
        );
    }
    lookup_group.finish();

    let fixture_index = context.fixture_named("lf-avg-80");
    let fixture = &context.fixtures[fixture_index];
    let index = LineIndex::build(
        Arc::clone(&snapshots[fixture_index]),
        index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, BASELINE_SCAN_CHUNK_BYTES),
    )
    .expect("range index build must succeed");
    let request_count = context.config.lookup_requests;
    let full_count = (request_count / 8).max(8);
    let full_range = LineRange::new(LineNumber::new(0), LineNumber::new(index.line_count()))
        .expect("full range is valid");
    let workloads = [
        (
            "small-span-3",
            seeded_ranges(index.line_count(), request_count, 3, RANDOM_SEED ^ 1),
        ),
        (
            "large-line-count-4096",
            seeded_ranges(
                index.line_count(),
                request_count,
                4_096.min(index.line_count()),
                RANDOM_SEED ^ 2,
            ),
        ),
        ("full-file", vec![full_range; full_count]),
        (
            "empty",
            empty_ranges(index.line_count(), request_count, RANDOM_SEED ^ 3),
        ),
        (
            "checkpoint-crossing",
            checkpoint_crossing_ranges(index.line_count(), index.final_stride(), request_count),
        ),
    ];
    let mut range_group = criterion.benchmark_group(context.group_identity("rng", mode));
    println!(
        "[faultsift-line-benchmark-range-memory] line_span_size_bytes={} result_cardinality=1 complexity=O(1)",
        std::mem::size_of::<LineSpan>()
    );
    for (label, ranges) in workloads {
        let requests = prepare_range_requests(&index, &ranges)
            .expect("range request preparation must succeed");
        let checksum = run_range_requests(&index, &requests).expect("range preflight must succeed");
        let latency = measure_range_latency(&index, &requests);
        println!(
            "[faultsift-line-benchmark-range] backend={} fixture={} workload={} requests={} final_stride={} checkpoint_budget_bytes={} scan_chunk_bytes={} p50_ns={} p95_ns={} p99_ns={} checksum=0x{checksum:016x}",
            mode.label(),
            fixture.metadata().name,
            label,
            requests.len(),
            index.final_stride(),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
            BASELINE_SCAN_CHUNK_BYTES,
            latency.p50_ns,
            latency.p95_ns,
            latency.p99_ns
        );
        range_group.throughput(Throughput::Elements(requests.len() as u64));
        range_group.bench_function(
            BenchmarkId::new(
                fixture.metadata().name.as_str(),
                format!(
                    "{label}-stride-{}-budget-{}-chunk-{}",
                    index.final_stride(),
                    BASELINE_CHECKPOINT_BUDGET_BYTES,
                    BASELINE_SCAN_CHUNK_BYTES
                ),
            ),
            |bencher| {
                bencher.iter(|| {
                    black_box(
                        run_range_requests(black_box(&index), black_box(&requests))
                            .expect("timed range batch must remain exact"),
                    )
                });
            },
        );
    }
    range_group.finish();
}

fn benchmark_readers(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let normal_fixture_index = context.fixture_named("lf-avg-200");
    let normal_fixture = &context.fixtures[normal_fixture_index];
    let normal_index = LineIndex::build(
        Arc::clone(&snapshots[normal_fixture_index]),
        index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, BASELINE_SCAN_CHUNK_BYTES),
    )
    .expect("normal reader index must build");
    let normal_descriptor = normal_index
        .line(LineNumber::new(normal_index.line_count() / 2))
        .expect("normal reader line must exist");
    let normal_span = normal_index
        .line_range(
            LineRange::new(LineNumber::new(10), LineNumber::new(138))
                .expect("normal span is valid"),
        )
        .expect("normal span lookup must succeed");
    let full_span = normal_index
        .line_range(
            LineRange::new(
                LineNumber::new(0),
                LineNumber::new(normal_index.line_count()),
            )
            .expect("full span is valid"),
        )
        .expect("full span lookup must succeed");

    let huge_fixture = &context.fixtures[context.huge_line_index];
    let huge_index = LineIndex::build(
        Arc::clone(&snapshots[context.huge_line_index]),
        index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, BASELINE_SCAN_CHUNK_BYTES),
    )
    .expect("huge reader index must build");
    let huge_descriptor = huge_index
        .line(LineNumber::new(0))
        .expect("huge reader line must exist");

    let mut line_group = criterion.benchmark_group(context.group_identity("read", mode));
    for (label, index, descriptor, expected_checksum) in [
        ("normal-line", &normal_index, normal_descriptor, None),
        (
            "huge-line",
            &huge_index,
            huge_descriptor,
            Some(huge_fixture.metadata().content_checksum),
        ),
    ] {
        let outcome = visit_line(index, &descriptor).expect("line reader preflight must succeed");
        if let Some(expected_checksum) = expected_checksum {
            assert_eq!(outcome.checksum, expected_checksum);
        }
        println!(
            "[faultsift-line-benchmark-reader] backend={} api=visit_line_content workload={} logical_bytes={} chunks={} final_stride={} checkpoint_budget_bytes={} scan_chunk_bytes={} checksum=0x{:016x}",
            mode.label(),
            label,
            outcome.content_bytes,
            outcome.chunks,
            index.final_stride(),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
            BASELINE_SCAN_CHUNK_BYTES,
            outcome.checksum
        );
        line_group.throughput(Throughput::Bytes(outcome.content_bytes.max(1)));
        line_group.bench_function(
            BenchmarkId::new(
                label,
                format!(
                    "line-stride-{}-budget-{}-chunk-{}",
                    index.final_stride(),
                    BASELINE_CHECKPOINT_BUDGET_BYTES,
                    BASELINE_SCAN_CHUNK_BYTES
                ),
            ),
            |bencher| {
                bencher.iter(|| {
                    black_box(
                        visit_line(black_box(index), black_box(&descriptor))
                            .expect("timed content reader must succeed"),
                    )
                });
            },
        );
    }
    line_group.finish();

    let mut span_group = criterion.benchmark_group(context.group_identity("span", mode));
    for (label, span, expected_checksum) in [
        ("normal-128-lines", normal_span, None),
        (
            "large-full-file",
            full_span,
            Some(normal_fixture.metadata().physical_checksum),
        ),
    ] {
        let outcome = visit_span(&normal_index, &span).expect("span reader preflight must succeed");
        if let Some(expected_checksum) = expected_checksum {
            assert_eq!(outcome.checksum, expected_checksum);
        }
        println!(
            "[faultsift-line-benchmark-reader] backend={} api=visit_span_physical workload={} logical_bytes={} lines={} chunks={} final_stride={} checkpoint_budget_bytes={} scan_chunk_bytes={} checksum=0x{:016x}",
            mode.label(),
            label,
            outcome.physical_bytes,
            outcome.lines,
            outcome.chunks,
            normal_index.final_stride(),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
            BASELINE_SCAN_CHUNK_BYTES,
            outcome.checksum
        );
        span_group.throughput(Throughput::Bytes(outcome.physical_bytes.max(1)));
        span_group.bench_function(
            BenchmarkId::new(
                label,
                format!(
                    "physical-stride-{}-budget-{}-chunk-{}",
                    normal_index.final_stride(),
                    BASELINE_CHECKPOINT_BUDGET_BYTES,
                    BASELINE_SCAN_CHUNK_BYTES
                ),
            ),
            |bencher| {
                bencher.iter(|| {
                    black_box(
                        visit_span(black_box(&normal_index), black_box(&span))
                            .expect("timed physical span reader must succeed"),
                    )
                });
            },
        );
    }
    span_group.finish();
}

fn record_cancellation_evidence(
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    for fixture_index in [context.newline_dense_index, context.huge_line_index] {
        let fixture = &context.fixtures[fixture_index];
        let snapshot = &snapshots[fixture_index];
        let mut callbacks = 0_u64;
        let index = LineIndex::build_with_control(
            Arc::clone(snapshot),
            index_options(
                PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES,
                BASELINE_SCAN_CHUNK_BYTES,
            ),
            |_| {
                callbacks += 1;
                BuildControl::Continue
            },
        )
        .expect("progress-count build must succeed");
        assert_eq!(callbacks, fixture.len().div_ceil(BASELINE_SCAN_CHUNK_BYTES));
        assert_eq!(index.line_count(), fixture.metadata().line_count);
        println!(
            "[faultsift-line-benchmark-progress] backend={} fixture={} bytes={} lines={} scan_chunk_bytes={} callback_count={} expected_chunk_count={} callback_basis=consumed-scan-chunks",
            mode.label(),
            fixture.metadata().name,
            fixture.len(),
            fixture.metadata().line_count,
            BASELINE_SCAN_CHUNK_BYTES,
            callbacks,
            fixture.len().div_ceil(BASELINE_SCAN_CHUNK_BYTES)
        );

        let cancel_at = callbacks.clamp(1, 2);
        let evidence = cancellation_at_callback(
            Arc::clone(snapshot),
            PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES,
            BASELINE_SCAN_CHUNK_BYTES,
            cancel_at,
        )
        .expect("fixed callback cancellation must succeed");
        print_cancellation(mode, fixture, evidence);
    }
}

fn benchmark_concurrent_lookup(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshots: &[Arc<FileSnapshot>],
) {
    let fixture_index = context.fixture_named("lf-avg-200");
    let fixture = &context.fixtures[fixture_index];
    let index = Arc::new(
        LineIndex::build(
            Arc::clone(&snapshots[fixture_index]),
            index_options(BASELINE_CHECKPOINT_BUDGET_BYTES, BASELINE_SCAN_CHUNK_BYTES),
        )
        .expect("concurrent lookup index must build"),
    );
    let numbers = generate_line_numbers(
        index.line_count(),
        context.config.lookup_requests.min(64),
        RANDOM_SEED ^ 0x434f_4e43,
    );
    let requests = Arc::new(
        prepare_line_requests(&index, &numbers).expect("concurrent lookup requests must prepare"),
    );
    let mut group = criterion.benchmark_group(context.group_identity("conc", mode));
    for concurrency in BENCHMARK_CONCURRENCY_LEVELS {
        group.throughput(Throughput::Elements(
            requests.len() as u64 * concurrency as u64,
        ));
        println!(
            "[faultsift-line-benchmark-concurrency] backend={} fixture={} concurrency={} operations_per_worker={} final_stride={} checkpoint_budget_bytes={} scan_chunk_bytes={} timing=timer-starts-before-barrier-release live_lookup_buffers={}",
            mode.label(),
            fixture.metadata().name,
            concurrency,
            requests.len(),
            index.final_stride(),
            BASELINE_CHECKPOINT_BUDGET_BYTES,
            BASELINE_SCAN_CHUNK_BYTES,
            concurrency
        );
        group.bench_function(
            BenchmarkId::new(
                fixture.metadata().name.as_str(),
                format!(
                    "workers-{concurrency}-stride-{}-budget-{}-chunk-{}-barrier-start",
                    index.final_stride(),
                    BASELINE_CHECKPOINT_BUDGET_BYTES,
                    BASELINE_SCAN_CHUNK_BYTES
                ),
            ),
            |bencher| {
                bencher.iter_custom(|iterations| {
                    run_concurrent_iterations(
                        Arc::clone(&index),
                        Arc::clone(&requests),
                        concurrency,
                        iterations,
                    )
                });
            },
        );
    }
    group.finish();
}

fn run_concurrent_iterations(
    index: Arc<LineIndex>,
    requests: Arc<Vec<LineRequest>>,
    concurrency: usize,
    iterations: u64,
) -> Duration {
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    thread::scope(|scope| {
        let mut workers = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let index = Arc::clone(&index);
            let requests = Arc::clone(&requests);
            let barrier = Arc::clone(&barrier);
            workers.push(scope.spawn(move || {
                barrier.wait();
                let mut checksum = 0_u64;
                for _ in 0..iterations {
                    checksum ^= run_line_requests(&index, &requests)
                        .expect("concurrent lookup must remain exact")
                        .checksum;
                }
                checksum
            }));
        }
        let started = Instant::now();
        barrier.wait();
        let checksum = workers
            .into_iter()
            .map(|worker| worker.join().expect("lookup worker must not panic"))
            .fold(0_u64, |combined, worker| combined ^ worker);
        let elapsed = started.elapsed();
        black_box(checksum);
        elapsed
    })
}

fn verify_index(fixture: &FixtureFile, index: &LineIndex) {
    assert_eq!(index.line_count(), fixture.metadata().line_count);
    assert_eq!(index.snapshot_length().get(), fixture.metadata().length);
    assert!(index.checkpoint_count() <= index.checkpoint_budget_bytes().get() / 8);
}

fn print_build_evidence(
    mode: BackendMode,
    fixture: &FixtureFile,
    elapsed: Duration,
    evidence: IndexEvidence,
    kind: &str,
) {
    let throughput = fixture.len() as f64 / elapsed.as_secs_f64();
    println!(
        "[faultsift-line-benchmark-build] kind={} backend={} fixture={} bytes={} lines={} wall_time_ns={} throughput_bytes_per_second={:.3} final_stride={} checkpoint_count={} checkpoint_capacity={} checkpoint_payload_bytes={} checkpoint_capacity_bytes={} compaction_count={} checkpoint_budget_bytes={} scan_chunk_bytes={}",
        kind,
        mode.label(),
        fixture.metadata().name,
        fixture.len(),
        evidence.line_count,
        elapsed.as_nanos(),
        throughput,
        evidence.final_stride,
        evidence.checkpoint_count,
        evidence.checkpoint_capacity,
        evidence.checkpoint_payload_bytes,
        evidence.checkpoint_capacity_bytes,
        evidence.compaction_count,
        evidence.checkpoint_budget_bytes,
        evidence.scan_chunk_bytes
    );
}

fn print_cancellation(mode: BackendMode, fixture: &FixtureFile, evidence: CancellationEvidence) {
    println!(
        "[faultsift-line-benchmark-cancellation] backend={} fixture={} callback_count={} bytes_scanned={} physical_lines_completed={} current_stride={} checkpoint_count={} detection_to_return_ns={} maximum_poll_bytes={} hard_realtime=false",
        mode.label(),
        fixture.metadata().name,
        evidence.callback_count,
        evidence.bytes_scanned,
        evidence.physical_lines_completed,
        evidence.final_stride,
        evidence.checkpoint_count,
        evidence.detection_to_return_ns,
        BASELINE_SCAN_CHUNK_BYTES
    );
}

#[derive(Clone, Copy, Debug)]
struct Percentiles {
    p50_ns: u128,
    p95_ns: u128,
    p99_ns: u128,
}

fn measure_line_latency(index: &LineIndex, requests: &[LineRequest]) -> Percentiles {
    measure_latency(requests.len(), |ordinal| {
        let request = &requests[ordinal];
        let actual = index
            .line(request.line_number)
            .expect("latency probe lookup must succeed");
        assert_eq!(actual, request.expected);
        black_box(actual);
    })
}

fn measure_range_latency(index: &LineIndex, requests: &[RangeRequest]) -> Percentiles {
    measure_latency(requests.len(), |ordinal| {
        let request = &requests[ordinal];
        let actual = index
            .line_range(request.range)
            .expect("latency probe range must succeed");
        assert_eq!(actual, request.expected);
        black_box(actual);
    })
}

fn measure_latency(count: usize, mut operation: impl FnMut(usize)) -> Percentiles {
    assert!(count > 0);
    let mut samples = Vec::with_capacity(count);
    for ordinal in 0..count {
        let started = Instant::now();
        operation(ordinal);
        samples.push(started.elapsed().as_nanos());
    }
    samples.sort_unstable();
    Percentiles {
        p50_ns: percentile(&samples, 50),
        p95_ns: percentile(&samples, 95),
        p99_ns: percentile(&samples, 99),
    }
}

fn percentile(samples: &[u128], percentile: usize) -> u128 {
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index]
}

fn criterion_config() -> Criterion {
    let config = BenchmarkConfig::from_environment();
    Criterion::default()
        .sample_size(if config.smoke { 10 } else { 15 })
        .warm_up_time(Duration::from_millis(if config.smoke { 100 } else { 500 }))
        .measurement_time(Duration::from_millis(if config.smoke {
            300
        } else {
            1_000
        }))
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = benchmark_line_access
}
criterion_main!(benches);
