#[allow(dead_code)]
#[path = "../benches/support/mod.rs"]
mod benchmark_support;

use std::sync::{Arc, Barrier};
use std::thread;

use faultsift_file_access::{BackendKind, ByteLength, ByteOffset, ByteRange, FileAccessOptions};

use benchmark_support::{
    AccessPattern, BENCHMARK_MAX_VIEW_BYTES, BackendMode, FIXTURE_WRITE_BUFFER_BYTES, FixtureFile,
    KIB, MIB, RANDOM_SEED, RANGE_SIZES, generate_ranges, open_snapshot, run_read_at, run_view,
    total_range_bytes,
};

#[test]
fn populated_fixture_generation_and_smoke_workloads_are_bounded_and_exact() {
    let fixture = FixtureFile::populated(16 * MIB).unwrap();
    assert_eq!(FIXTURE_WRITE_BUFFER_BYTES, MIB as usize);
    let snapshot = open_snapshot(
        fixture.path(),
        BENCHMARK_MAX_VIEW_BYTES,
        BackendMode::Automatic,
    )
    .unwrap();

    for pattern in [AccessPattern::Sequential, AccessPattern::Random] {
        for range_size in RANGE_SIZES {
            let ranges =
                generate_ranges(fixture.len(), range_size, MIB, pattern, RANDOM_SEED).unwrap();
            let expected_bytes = total_range_bytes(&ranges);

            let view = run_view(&snapshot, &ranges).unwrap();
            let mut buffer = vec![0_u8; range_size as usize];
            let read_at = run_read_at(&snapshot, &ranges, &mut buffer).unwrap();

            assert_eq!(view.bytes, expected_bytes);
            assert_eq!(read_at.bytes, expected_bytes);
            assert_eq!(view.checksum, read_at.checksum);
        }
    }
}

#[test]
fn seeded_random_ranges_are_reproducible_and_seed_sensitive() {
    let first = generate_ranges(
        16 * MIB,
        64 * KIB,
        2 * MIB,
        AccessPattern::Random,
        RANDOM_SEED,
    )
    .unwrap();
    let second = generate_ranges(
        16 * MIB,
        64 * KIB,
        2 * MIB,
        AccessPattern::Random,
        RANDOM_SEED,
    )
    .unwrap();
    let different_seed = generate_ranges(
        16 * MIB,
        64 * KIB,
        2 * MIB,
        AccessPattern::Random,
        RANDOM_SEED ^ 1,
    )
    .unwrap();

    assert_eq!(first, second);
    assert_ne!(first, different_seed);
}

#[test]
fn concurrent_view_batch_holds_only_the_configured_number_of_live_views() {
    const CONCURRENCY: usize = 4;
    let fixture = FixtureFile::populated(2 * MIB).unwrap();
    let snapshot = Arc::new(
        open_snapshot(
            fixture.path(),
            FileAccessOptions::default().max_view_bytes().get(),
            BackendMode::Automatic,
        )
        .unwrap(),
    );
    let ready = Arc::new(Barrier::new(CONCURRENCY + 1));
    let release = Arc::new(Barrier::new(CONCURRENCY + 1));

    thread::scope(|scope| {
        let mut workers = Vec::new();
        for worker in 0..CONCURRENCY {
            let snapshot = Arc::clone(&snapshot);
            let ready = Arc::clone(&ready);
            let release = Arc::clone(&release);
            workers.push(scope.spawn(move || {
                let range = ByteRange::new(
                    ByteOffset::new((worker as u64) * 64 * KIB),
                    ByteLength::new(64 * KIB),
                )
                .unwrap();
                let view = snapshot.view(range).unwrap();
                ready.wait();
                release.wait();
                assert_eq!(view.len(), 64 * KIB as usize);
            }));
        }

        ready.wait();
        release.wait();
        for worker in workers {
            worker.join().unwrap();
        }
    });
}

#[cfg(windows)]
#[test]
fn windows_forced_buffered_mode_reports_real_fallback_and_remains_usable() {
    use faultsift_file_access::MappingFallbackReason;

    let fixture = FixtureFile::populated(2 * MIB).unwrap();
    let snapshot = open_snapshot(
        fixture.path(),
        FileAccessOptions::default().max_view_bytes().get(),
        BackendMode::ForcedBuffered,
    )
    .unwrap();

    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);
    assert_eq!(
        snapshot.diagnostics().mapping_fallback_reason(),
        Some(MappingFallbackReason::IncompatibleWriter)
    );
    let ranges = generate_ranges(
        fixture.len(),
        64 * KIB,
        MIB,
        AccessPattern::Sequential,
        RANDOM_SEED,
    )
    .unwrap();
    let view = run_view(&snapshot, &ranges).unwrap();
    let mut buffer = vec![0_u8; 64 * KIB as usize];
    let read = run_read_at(&snapshot, &ranges, &mut buffer).unwrap();
    assert_eq!(view, read);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_benchmark_mode_reports_buffered_backend() {
    let fixture = FixtureFile::populated(2 * MIB).unwrap();
    let snapshot = open_snapshot(
        fixture.path(),
        FileAccessOptions::default().max_view_bytes().get(),
        BackendMode::Automatic,
    )
    .unwrap();

    assert_eq!(snapshot.diagnostics().backend(), BackendKind::Buffered);
}
