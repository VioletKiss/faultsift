mod support;

use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use faultsift_file_access::{ByteRange, FileAccessOptions, FileSnapshot};
use support::{
    AccessApi, AccessPattern, BENCHMARK_MAX_VIEW_BYTES, BackendMode, BackendModeGuard,
    CONCURRENCY_LEVELS, CRITERION_VERSION, FixtureFile, MIB, RANDOM_SEED, RANGE_SIZES,
    backend_modes, generate_ranges, mode_diagnostics, open_snapshot, open_snapshot_with_guard,
    print_environment_metadata, print_memory, process_memory, run_read_at, run_view,
    total_range_bytes, verify_sparse_snapshot,
};

struct BenchContext {
    fixture: FixtureFile,
    workload_bytes: u64,
    smoke: bool,
}

impl BenchContext {
    fn create() -> Self {
        let smoke = environment_flag("FAULTSIFT_BENCH_SMOKE");
        let fixture_size = environment_mib(
            "FAULTSIFT_BENCH_FILE_MIB",
            if smoke { 16 } else { 64 },
            8,
            1024,
        );
        let workload_bytes = environment_mib(
            "FAULTSIFT_BENCH_WORKLOAD_MIB",
            if smoke { 1 } else { 64 },
            1,
            256,
        );
        assert!(
            fixture_size >= BENCHMARK_MAX_VIEW_BYTES,
            "benchmark fixture must fit the largest calibrated view candidate"
        );

        let fixture = FixtureFile::populated(fixture_size)
            .expect("deterministic populated fixture creation must succeed");
        Self {
            fixture,
            workload_bytes,
            smoke,
        }
    }

    fn profile(&self) -> String {
        format!(
            "criterion-{CRITERION_VERSION}-{}-fixture-{}MiB-workload-{}MiB",
            if self.smoke { "smoke" } else { "full" },
            self.fixture.len() / MIB,
            self.workload_bytes / MIB
        )
    }
}

fn benchmark_file_access(criterion: &mut Criterion) {
    let context = BenchContext::create();
    print_environment_metadata(&context.fixture, context.smoke);
    println!("benchmark_profile={}", context.profile());
    print_memory("after-populated-fixture-open", &process_memory());

    if context.smoke {
        println!(
            "[faultsift-benchmark-sparse] not-run: smoke mode excludes the >4 GiB manual fixture"
        );
    } else {
        verify_sparse_boundary();
    }

    benchmark_open_latency(criterion, &context);
    for &mode in backend_modes() {
        let snapshot = Arc::new(
            open_snapshot(context.fixture.path(), BENCHMARK_MAX_VIEW_BYTES, mode).unwrap_or_else(
                |error| {
                    panic!(
                        "backend {} could not be established for {}: {error}",
                        mode.label(),
                        context.fixture.path().display()
                    )
                },
            ),
        );
        println!(
            "[faultsift-benchmark-backend] mode={} {}",
            mode.label(),
            mode_diagnostics(&snapshot)
        );
        verify_fixture_contents(&context, &snapshot);
        benchmark_access_patterns(criterion, &context, mode, Arc::clone(&snapshot));
        benchmark_concurrency(criterion, &context, mode, Arc::clone(&snapshot));
        drop(snapshot);
    }

    print_memory("after-benchmark-groups", &process_memory());
}

fn verify_fixture_contents(context: &BenchContext, snapshot: &FileSnapshot) {
    let offsets = [0, context.fixture.len() / 2, context.fixture.len() - 64];
    for offset in offsets {
        let mut bytes = [0_u8; 64];
        let read = snapshot
            .read_at(offset.into(), &mut bytes)
            .expect("fixture verification read must succeed");
        assert_eq!(read, bytes.len());
        assert!(context.fixture.verify_populated_bytes(offset, &bytes));
    }
}

fn verify_sparse_boundary() {
    let before = process_memory();
    let (fixture, verification) = FixtureFile::sparse_boundary()
        .expect("filesystem must confirm bounded sparse semantics for the full benchmark");
    let snapshot = open_snapshot(
        fixture.path(),
        FileAccessOptions::default().max_view_bytes().get(),
        BackendMode::Automatic,
    )
    .expect("sparse boundary snapshot must open using the platform's automatic backend");
    let bytes = verify_sparse_snapshot(&snapshot).expect("sparse sentinels must be readable");
    println!(
        "[faultsift-benchmark-sparse] status=pass logical_bytes={} sentinel_bytes={} verification={} {}",
        fixture.len(),
        bytes,
        verification,
        mode_diagnostics(&snapshot)
    );
    print_memory("sparse-boundary-open", &process_memory());
    drop(snapshot);
    drop(fixture);
    print_memory("sparse-boundary-dropped", &process_memory());
    print_memory("sparse-boundary-before", &before);
}

fn benchmark_open_latency(criterion: &mut Criterion, context: &BenchContext) {
    let mut group = criterion.benchmark_group(format!("open/{}/warm-cache", context.profile()));
    for &mode in backend_modes() {
        let path = context.fixture.path();
        let guard = BackendModeGuard::acquire(path, mode)
            .expect("benchmark backend forcing guard must be established outside timed work");
        group.bench_function(BenchmarkId::from_parameter(mode.label()), |bencher| {
            bencher.iter(|| {
                let snapshot = open_snapshot_with_guard(
                    black_box(path),
                    FileAccessOptions::default().max_view_bytes().get(),
                    mode,
                    black_box(&guard),
                )
                .expect("timed snapshot open must preserve the selected backend");
                black_box(snapshot.diagnostics());
            });
        });
    }
    group.finish();
}

fn benchmark_access_patterns(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshot: Arc<FileSnapshot>,
) {
    for pattern in [AccessPattern::Sequential, AccessPattern::Random] {
        let group_name = format!(
            "access/{}/{}/{}",
            context.profile(),
            mode.label(),
            pattern.label()
        );
        let mut group = criterion.benchmark_group(group_name);

        for range_size in RANGE_SIZES {
            let ranges = generate_ranges(
                context.fixture.len(),
                range_size,
                context.workload_bytes,
                pattern,
                RANDOM_SEED,
            )
            .expect("fixed workload ranges must be valid");
            let total_bytes = total_range_bytes(&ranges);
            group.throughput(Throughput::Bytes(total_bytes));
            println!(
                "[faultsift-benchmark-workload] backend={} api=view pattern={} range_bytes={} operations={} bytes_per_iteration={} concurrency=1 seed=0x{RANDOM_SEED:016x}",
                mode.label(),
                pattern.label(),
                range_size,
                ranges.len(),
                total_bytes
            );

            let view_snapshot = Arc::clone(&snapshot);
            group.bench_function(
                BenchmarkId::new(AccessApi::View.label(), format_bytes(range_size)),
                |bencher| {
                    bencher.iter(|| {
                        let outcome = run_view(&view_snapshot, black_box(&ranges))
                            .expect("view workload must remain in bounds");
                        assert_eq!(outcome.bytes, total_bytes);
                        black_box(outcome)
                    });
                },
            );

            println!(
                "[faultsift-benchmark-workload] backend={} api=read_at pattern={} range_bytes={} operations={} bytes_per_iteration={} concurrency=1 seed=0x{RANDOM_SEED:016x}",
                mode.label(),
                pattern.label(),
                range_size,
                ranges.len(),
                total_bytes
            );
            let read_snapshot = Arc::clone(&snapshot);
            let mut caller_buffer = vec![0_u8; range_size as usize];
            group.bench_function(
                BenchmarkId::new(AccessApi::ReadAt.label(), format_bytes(range_size)),
                |bencher| {
                    bencher.iter(|| {
                        let outcome = run_read_at(
                            &read_snapshot,
                            black_box(&ranges),
                            black_box(&mut caller_buffer),
                        )
                        .expect("caller-buffer workload must remain in bounds");
                        assert_eq!(outcome.bytes, total_bytes);
                        black_box(outcome)
                    });
                },
            );
        }
        group.finish();
    }
}

fn benchmark_concurrency(
    criterion: &mut Criterion,
    context: &BenchContext,
    mode: BackendMode,
    snapshot: Arc<FileSnapshot>,
) {
    const CONCURRENT_RANGE_BYTES: u64 = 64 * support::KIB;
    let worker_bytes = if context.smoke { MIB / 4 } else { 4 * MIB };

    let mut group = criterion.benchmark_group(format!(
        "concurrency/{}/{}/seeded-random",
        context.profile(),
        mode.label()
    ));
    for concurrency in CONCURRENCY_LEVELS {
        let worker_ranges: Vec<Vec<ByteRange>> = (0..concurrency)
            .map(|worker| {
                generate_ranges(
                    context.fixture.len(),
                    CONCURRENT_RANGE_BYTES,
                    worker_bytes,
                    AccessPattern::Random,
                    RANDOM_SEED ^ worker as u64,
                )
                .expect("concurrent ranges must be valid")
            })
            .collect();
        let bytes_per_iteration = worker_ranges
            .iter()
            .map(|ranges| total_range_bytes(ranges))
            .sum();
        group.throughput(Throughput::Bytes(bytes_per_iteration));

        for api in [AccessApi::View, AccessApi::ReadAt] {
            println!(
                "[faultsift-benchmark-workload] backend={} api={} pattern=seeded-random range_bytes={} operations_per_worker={} bytes_per_iteration={} concurrency={} seed=0x{RANDOM_SEED:016x}",
                mode.label(),
                api.label(),
                CONCURRENT_RANGE_BYTES,
                worker_ranges[0].len(),
                bytes_per_iteration,
                concurrency
            );
            let benchmark_snapshot = Arc::clone(&snapshot);
            group.bench_function(
                BenchmarkId::new(api.label(), format!("threads-{concurrency}")),
                |bencher| {
                    bencher.iter_custom(|iterations| {
                        run_concurrent_iterations(
                            Arc::clone(&benchmark_snapshot),
                            &worker_ranges,
                            api,
                            iterations,
                        )
                    });
                },
            );
        }
    }
    group.finish();
}

fn run_concurrent_iterations(
    snapshot: Arc<FileSnapshot>,
    worker_ranges: &[Vec<ByteRange>],
    api: AccessApi,
    iterations: u64,
) -> Duration {
    let barrier = Arc::new(Barrier::new(worker_ranges.len() + 1));
    thread::scope(|scope| {
        let mut workers = Vec::with_capacity(worker_ranges.len());
        for ranges in worker_ranges {
            let snapshot = Arc::clone(&snapshot);
            let barrier = Arc::clone(&barrier);
            workers.push(scope.spawn(move || {
                let mut buffer = vec![0_u8; ranges[0].length().get() as usize];
                barrier.wait();
                let mut checksum = 0_u64;
                for _ in 0..iterations {
                    let outcome = match api {
                        AccessApi::View => run_view(&snapshot, ranges),
                        AccessApi::ReadAt => run_read_at(&snapshot, ranges, &mut buffer),
                    }
                    .expect("concurrent benchmark access must succeed");
                    checksum ^= outcome.checksum;
                }
                checksum
            }));
        }

        let start = Instant::now();
        barrier.wait();
        let checksum = workers
            .into_iter()
            .map(|worker| worker.join().expect("benchmark worker must not panic"))
            .fold(0_u64, |combined, worker| combined ^ worker);
        let elapsed = start.elapsed();
        black_box(checksum);
        elapsed
    })
}

fn criterion_config() -> Criterion {
    let smoke = environment_flag("FAULTSIFT_BENCH_SMOKE");
    Criterion::default()
        .sample_size(if smoke { 10 } else { 20 })
        .warm_up_time(Duration::from_millis(if smoke { 100 } else { 1_000 }))
        .measurement_time(Duration::from_millis(if smoke { 300 } else { 2_000 }))
}

fn environment_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value != "0")
}

fn environment_mib(name: &str, default_mib: u64, minimum_mib: u64, maximum_mib: u64) -> u64 {
    let value_mib = std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_mib);
    assert!(
        (minimum_mib..=maximum_mib).contains(&value_mib),
        "{name} must be between {minimum_mib} and {maximum_mib} MiB"
    );
    value_mib
        .checked_mul(MIB)
        .expect("benchmark MiB setting must fit u64")
}

fn format_bytes(bytes: u64) -> String {
    if bytes.is_multiple_of(MIB) {
        format!("{}MiB", bytes / MIB)
    } else if bytes.is_multiple_of(support::KIB) {
        format!("{}KiB", bytes / support::KIB)
    } else {
        bytes.to_string()
    }
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = benchmark_file_access
}
criterion_main!(benches);
