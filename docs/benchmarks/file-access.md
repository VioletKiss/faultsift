# File Access Benchmark Baseline

## Purpose

This document defines the reproducible File Access benchmark method introduced by FS-005 and records one controlled local baseline. It does not define a performance SLA, a CI timing gate, or a storage-performance promise.

The harness measures the existing backend-neutral `FileSnapshot` contract. It does not change snapshot lifecycle, mapping eligibility, fallback, identity, or range lifetime semantics.

## Harness

- Tool: Criterion 0.7.0, stable Rust, optimized `bench` profile.
- Target: `crates/faultsift-file-access/benches/byte_access.rs`.
- Fixture/workload support: `crates/faultsift-file-access/benches/support/mod.rs`.
- Criterion is a dev-dependency with default features disabled and only `cargo_bench_support` enabled. It is absent from normal runtime dependency paths.
- Criterion sample configuration: 20 samples, 1 second warm-up, 2 second requested measurement time. Criterion may extend collection when an operation is too slow for that target time.
- Smoke configuration: 10 samples, 100 ms warm-up, 300 ms requested measurement time.

Every benchmark group name contains its tool version, fixture, and workload profile. Smoke, full, and future tool-version measurements therefore cannot be compared accidentally as if they were the same workload.

## Fixtures

### Populated throughput fixture

The default full fixture is 64 MiB. It is streamed to a temporary file with a fixed byte function and a 1 MiB setup buffer; setup memory does not scale with fixture length. No generated fixture is committed.

Default full workload:

- 64 MiB logical bytes per single-thread access iteration;
- range candidates: 4 KiB, 64 KiB, 1 MiB, and 8 MiB;
- fixed fixture and random seed: `0x46533030355f5255`;
- each view is dropped before the next range, so views are not accumulated;
- each `read_at` case reuses one caller buffer;
- the 8 MiB benchmark-only view limit is explicit and bounded so the default can be calibrated against a larger candidate.

Environment variables may override fixture and workload size:

```text
FAULTSIFT_BENCH_FILE_MIB
FAULTSIFT_BENCH_WORKLOAD_MIB
FAULTSIFT_BENCH_STORAGE_CLASS
```

Any override changes the benchmark profile name and must be recorded with results.
The populated fixture override is hard-limited to 8–1,024 MiB and the per-iteration workload override to 1–256 MiB, so an accidental environment value cannot create an unbounded allocation or range list.

### Greater-than-4-GiB sparse boundary fixture

The full harness creates a temporary file of 4,295,032,832 bytes (4 GiB + 64 KiB), writes only bounded sentinels across the 4 GiB boundary and near EOF, then verifies them with both `view` and `read_at`.

- Windows explicitly sets and queries the sparse-file flag with the built-in `fsutil` command before extending the file.
- Linux verifies that allocated blocks remain below one sixteenth of logical length.
- Failure to prove sparse semantics aborts the full benchmark. The harness never silently creates an ordinary multi-GB allocation.
- Sparse-boundary access is a correctness and address-space observation, not a populated-file throughput result.
- Smoke mode skips this fixture, so CI smoke correctness does not need cache control, elevation, or a multi-GB logical file.

## Backend Selection

Windows cases are measured serially against the same populated fixture:

1. `forced-buffered`: hold an ordinary writer while opening the snapshot, require diagnostics `Buffered` plus `IncompatibleWriter`, then release the writer before access-pattern measurement;
2. `automatic`: require diagnostics `Mapped` with no fallback reason.

Serial measurement is required because an active mapped stability handle correctly prevents the writer used to establish the forced-buffered case. If the automatic Windows case falls back, the full benchmark fails instead of mislabelling buffered access as mapped.

For the open-latency group, one forcing writer guard is acquired before the timed loop and retained across its forced-buffered samples. Writer creation and destruction are therefore excluded from both backend open measurements; each timed iteration measures snapshot/backend establishment and verifies the selected diagnostics.

Linux measures only the automatic buffered backend and asserts `BackendKind::Buffered`. No Linux mapping is implemented.

## Workloads

### Sequential and random

Both access APIs consume identical pre-generated `ByteRange` sequences:

- `view`: creates a bounded `RangeView`, consumes its first and last bytes, and drops it immediately;
- `read_at`: copies into one reused caller buffer and consumes its first and last bytes;
- sequential: advances by the configured range size and wraps within the fixture;
- seeded random: xorshift64* positions generated from the fixed seed above;
- tests generate the random sequence twice and require exact equality.

For buffered access, `view` allocates and copies the complete range while `read_at` copies into reused storage. For mapped access, `view` creates an owned range handle without copying the range; `read_at` still copies the complete range.

Criterion's mapped-`view` throughput is therefore **logical range exposure**, not scanned bytes, page-fault cost, or disk throughput. It must be interpreted with the operation latency and access contract, not compared directly with copied-byte throughput.

### Concurrency

The concurrent workload uses 64 KiB deterministic random ranges, 4 MiB per worker per iteration, and fixed concurrency levels 1 and 4. Worker threads are created outside the timed interval for each Criterion sample, but the timer starts before the main thread releases their shared start barrier so no worker can perform measured work before timing begins. The workers use no unbounded task creation, global mutex, or shared seek cursor. `RangeView` values are dropped within each operation; a focused test separately holds exactly four live views and then releases the complete batch.

## Cache Semantics

All recorded timings are **warm-cache with uncontrolled OS page cache**. The fixture is written immediately before repeated measurements. Repetition is not described as cold-cache behavior.

The harness never drops system caches. A controlled cold-cache study requires dedicated hardware and platform-specific external preparation and remains a manual limitation.

## Commands

Full local baseline:

```text
cargo bench -p faultsift-file-access --bench byte_access
```

Build only:

```text
cargo check -p faultsift-file-access --benches
cargo bench -p faultsift-file-access --bench byte_access --no-run
```

Windows PowerShell smoke:

```powershell
$env:FAULTSIFT_BENCH_SMOKE = "1"
cargo bench -p faultsift-file-access --bench byte_access
Remove-Item Env:\FAULTSIFT_BENCH_SMOKE
```

Linux smoke:

```text
FAULTSIFT_BENCH_SMOKE=1 cargo bench -p faultsift-file-access --bench byte_access
```

CI compiles the bench on Windows and Ubuntu and runs the bounded integration tests through normal Rust tests. CI does not execute Criterion measurements and has no throughput threshold.

## Recorded Windows Baseline

The tabulated baseline was captured from the final tracked FS-005 source immediately before its implementation commit, based on `ef8b977dd402c8edeebf1f70ffc4563c57d1e44e`; the harness reported `tracked_worktree_dirty=true`. A complete confirmation run on the byte-identical implementation commit `9b086ccf8708138e15887612dea5bf4293455a0b` reported that exact SHA with `tracked_worktree_dirty=false` and again passed backend selection, the >4 GiB sparse check, all workloads, and bounded memory reporting. The table preserves one internally consistent calibration sample rather than mixing estimates from successive environmentally variable runs.

Environment:

| Field | Value |
|---|---|
| Date | 2026-08-27, Asia/Shanghai |
| OS | Windows build 10.0.26200.9168, x86_64 |
| CPU | 11th Gen Intel Core i5-11400, 6 cores / 12 logical processors |
| Memory | 34,067,255,296 bytes physical |
| Filesystem | NTFS, fixed local C: volume |
| Storage | Samsung SSD 870 QVO 1TB, SATA SSD |
| Rust | rustc 1.98.0, optimized bench profile |
| Tool | Criterion 0.7.0 |
| Fixture | 64 MiB populated temporary file |
| Workload | 64 MiB per access iteration |
| Cache | warm, uncontrolled OS page cache |
| Backends | `Mapped` automatic; `Buffered(IncompatibleWriter)` forced |
| Seed | `0x46533030355f5255` |

The values below are Criterion central estimates from this one run. They are evidence for resource calibration, not universal targets.

### Open latency

| Backend | Central estimate |
|---|---:|
| Forced buffered | 68.786 µs |
| Automatic mapped | 751.41 µs |

The mapped open includes stability-handle, identity/location/filesystem checks, mapping-object creation, and view creation. Open is not repeated for each byte-range operation in normal use.

### Sequential access

| Range | Buffered `view` | Buffered `read_at` | Mapped `view` logical exposure | Mapped `read_at` copied bytes |
|---|---:|---:|---:|---:|
| 4 KiB | 1.122 GiB/s | 1.147 GiB/s | 37.74 GiB/s | 15.94 GiB/s |
| 64 KiB | 3.400 GiB/s | 4.239 GiB/s | 1,552 GiB/s | 14.78 GiB/s |
| 1 MiB | 1.736 GiB/s | 4.331 GiB/s | 36,440 GiB/s | 11.54 GiB/s |
| 8 MiB | 1.960 GiB/s | 3.823 GiB/s | 310,593 GiB/s | 11.26 GiB/s |

### Seeded random access

| Range | Buffered `view` | Buffered `read_at` | Mapped `view` logical exposure | Mapped `read_at` copied bytes |
|---|---:|---:|---:|---:|
| 4 KiB | 727.1 MiB/s | 761.0 MiB/s | 49.73 GiB/s | 11.83 GiB/s |
| 64 KiB | 3.565 GiB/s | 3.808 GiB/s | 1,735 GiB/s | 14.82 GiB/s |
| 1 MiB | 1.619 GiB/s | 4.503 GiB/s | 39,501 GiB/s | 11.50 GiB/s |
| 8 MiB | 2.115 GiB/s | 3.987 GiB/s | 336,505 GiB/s | 11.40 GiB/s |

### Concurrency

The concurrent case uses 64 KiB ranges and 4 MiB per worker.

| Backend/API | 1 worker | 4 workers |
|---|---:|---:|
| Buffered `view` | 4.418 GiB/s | 3.505 GiB/s |
| Buffered `read_at` | 4.789 GiB/s | 3.990 GiB/s |
| Mapped `view`, logical exposure | 2,409 GiB/s | 1,820 GiB/s |
| Mapped `read_at`, copied bytes | 31.05 GiB/s | 49.14 GiB/s |

No conclusion about ideal application concurrency is drawn from one warm-cache machine. The result only establishes that the harness measures fixed, bounded levels.

### Memory and sparse observations

| Observation | RSS | Virtual address space |
|---|---:|---:|
| Populated fixture opened | 6,098,944 bytes | 4,355,661,824 bytes |
| 4 GiB + 64 KiB sparse mapping open | 6,152,192 bytes | 8,650,694,656 bytes |
| Sparse mapping dropped | 6,144,000 bytes | 4,355,661,824 bytes |
| End of all groups | 8,577,024 bytes | 4,361,019,392 bytes |

The sparse mapping increased virtual address space by exactly its logical length and returned to the prior level after drop, while RSS changed only slightly. This supports the existing mapping lifetime implementation but is not evidence of physical disk throughput.

Windows minor/major page-fault counts and stable per-case allocation counts were not collected. CPU time was also not separated reliably from Criterion and OS activity. These metrics are marked unavailable rather than inferred. Copied bytes are known from the API contract and verified workload totals; allocator operation counts are not.

## `max_view_bytes` Calibration

FS-005 names `DEFAULT_MAX_VIEW_BYTES` as 1 MiB and implements `FileAccessOptions::default()` with that bound. Explicit `FileAccessOptions::new` remains available.

Evidence and trade-off:

- 4 KiB pays high per-operation overhead on the buffered backend.
- 64 KiB frequently provides the best buffered sequential throughput on this machine, but it requires many more calls for future chunked consumers.
- 1 MiB amortizes call overhead and produced competitive buffered caller-buffer throughput without requiring a large live allocation.
- 8 MiB did not provide a consistent copied-byte advantage over 1 MiB and increases one buffered live view by 8×.
- At the tested concurrency level 4, the named default bounds four simultaneous buffered views to 4 MiB of payload, compared with 32 MiB for the 8 MiB candidate.
- Mapped views do not allocate the complete range, but the public default must remain safe for the buffered fallback and Linux baseline.

The 1 MiB value is therefore a conservative resource default, not the range size every caller must use and not a performance commitment. Evidence from one machine is insufficient for an automated regression threshold or a larger default.

## Interpretation Limits

- Results vary with page cache, filesystem compression, antivirus, storage firmware, thermal state, and background load.
- The populated fixture is deterministic byte data, not a parser or line-index workload.
- Sparse pages are excluded from throughput claims.
- Mapped `view` logical throughput does not mean all returned bytes were read by the CPU.
- Open latency and access latency represent one local fixed-volume configuration.
- No cold-cache, network, removable-media, Linux-mapping, parser, line-index, search, UI, or AI behavior is measured.
- Later performance gates require repeated measurements on controlled hardware and a separate approved decision.
