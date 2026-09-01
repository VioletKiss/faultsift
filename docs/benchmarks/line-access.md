# Line Access Benchmark Baseline

## Purpose and Decision Boundary

This document defines the reproducible Line Access benchmark method introduced by FS-009 and records one controlled Windows baseline. It measures the completed M2 physical-line cursor, eager adaptive index build, ready lookup, line-range metadata, bounded readers, progress/cancellation, and pathological line distributions.

It does not change any scanner, cursor, index, lookup, reader, snapshot, or cancellation semantics. It does not define a product SLA, CI timing gate, named resource default, cold-cache result, or Linux performance claim. The values below are single-machine evidence, not a FaultSift performance promise.

The FS-005 mapped `RangeView` result is deliberately not reused as a scanning number. Every Line Access throughput workload performs the required newline scan or bounded byte visit and validates exact bytes, coordinates, line counts, and deterministic checksums.

## Harness

- Tool: Criterion 0.7.0, stable Rust, optimized `bench` profile.
- Target: `crates/faultsift-line-access/benches/line_access.rs`.
- Bounded support: `crates/faultsift-line-access/benches/support/mod.rs`.
- Correctness/determinism coverage: `crates/faultsift-line-access/tests/benchmark_harness.rs`.
- Criterion is a dev-dependency with default features disabled and only `cargo_bench_support` enabled. It is absent from the normal runtime dependency path.
- Full configuration: 15 samples, 500 ms warm-up, 1 second requested measurement time. Criterion extends collection for slow operations.
- Smoke configuration: 10 samples, 100 ms warm-up, 300 ms requested measurement time.
- Criterion group names use a tested compact identity of at most 64 characters so Criterion does not truncate distinct configurations onto one result directory. The identity includes API, actual backend mode, smoke/full profile, Criterion version, representative fixture MiB, huge-line MiB, request count, the complete seed, and warm-cache label. Child IDs add fixture/workload, checkpoint budget, scan chunk, and final stride as relevant. The harness test requires every configuration component to change the identity and verifies the Windows backend identities differ.

All fixture creation, streaming checksum verification, snapshot/backend establishment, ready-index preparation, seeded request generation, expected-result generation, and preflight correctness checks occur outside Criterion's measured iterations. Index-build iterations use `iter_custom`: each iteration clones the already-open snapshot before starting an `Instant`, constructs one fresh index, records only that build interval, consumes the index through `black_box`, and drops it before the next iteration. This prevents Criterion batching from retaining multiple 16/32/64 MiB checkpoint reservations simultaneously. Cursor construction is intentionally part of each cursor operation. Bounded-reader construction of its one reusable buffer is intentionally part of each independent reader call because that is the current API contract.

The concurrency workload creates its fixed worker set before starting the timer. The main thread starts the timer before releasing the shared barrier, so no worker can execute measured lookups early. Workers and their per-operation lookup buffers are joined and released before the sample returns.

## Fixtures and Determinism

The fixed seed is `0x46533030395f4c41`. Fixture content is printable deterministic byte data that cannot accidentally introduce LF or CR. Representative physical lengths use the repeating delta sequence `[-8, -4, 0, 4, 8]`, rotated by the seed, around the requested average. Generation uses one fixed 64 KiB setup buffer and never retains a fixture or huge line in memory. Generated files live in the OS temporary directory and are deleted on drop; no large fixture is committed.

The default full profile uses:

| Fixture | Exact bytes | Exact lines | Physical bytes/line | Terminators |
|---|---:|---:|---:|---|
| `lf-avg-80` | 16,777,200 | 209,715 | 80.000 | LF |
| `crlf-avg-80` | 16,777,200 | 209,715 | 80.000 | CRLF |
| `lf-avg-200` | 16,777,200 | 83,886 | 200.000 | LF |
| `crlf-avg-200` | 16,777,200 | 83,886 | 200.000 | CRLF |
| `lf-avg-500` | 16,777,004 | 33,554 | 500.000 | LF |
| `crlf-avg-500` | 16,777,004 | 33,554 | 500.000 | CRLF |
| `newline-dense-lf` | 16,777,216 | 16,777,216 | 1.000 | LF, empty content |
| `huge-line-16777216-bytes` | 16,777,217 | 1 | 16,777,217 | one LF after 16 MiB content |

Tests generate every representative distribution twice and compare exact streamed bytes and metadata. Seeded line/range sequences are generated twice and compared; changing the seed must change the sequence. The harness also streams each generated file again before benchmarking and requires its exact length and physical checksum.

Environment overrides are hard-limited:

| Variable | Meaning | Full default | Allowed range |
|---|---|---:|---:|
| `FAULTSIFT_LINE_BENCH_FIXTURE_MIB` | representative and newline-dense target | 16 MiB | 1–512 MiB |
| `FAULTSIFT_LINE_BENCH_HUGE_MIB` | huge-line content | 16 MiB | 1–256 MiB |
| `FAULTSIFT_LINE_BENCH_LOOKUPS` | typical lookup/range request count | 256 | 8–16,384 |
| `FAULTSIFT_LINE_BENCH_STORAGE_CLASS` | manually supplied storage description | auto/manual | text metadata only |
| `FAULTSIFT_LINE_BENCH_SMOKE` | bounded smoke profile | unset | set and not `0` |

The smoke profile uses 2 MiB representative and huge-line fixtures and 32 typical lookup requests. Pathological newline-dense and huge-line lookup batches are capped at 16 requests even when the typical override is larger; their per-request scanned work is reported explicitly.

The greater-than-4-GiB coordinate tests remain separate correctness tests from FS-006 through FS-008. This performance harness does not treat sparse holes as real storage throughput.

## Backends and Cache Semantics

Windows measures the same fixture serially in this order:

1. `forced-buffered`: a normal writer is held only while opening the snapshot; diagnostics must be `Buffered` with `IncompatibleWriter`, then the writer is released before any measured scan;
2. `automatic`: diagnostics must be `Mapped` with no fallback reason.

If either required diagnostic cannot be established, the Windows benchmark aborts instead of comparing a backend with itself. No force-backend production API was added. Linux has only the automatic buffered mode and requires `BackendKind::Buffered`.

All timings are **warm-cache with uncontrolled OS page cache**. Fixtures are generated immediately before repeated measurements. Repetition is not called cold-cache behavior, and the harness never performs a dangerous system cache flush. Controlled cold-cache work remains deferred.

## Commands

Full local baseline:

```text
cargo bench -p faultsift-line-access --bench line_access
```

Build-only checks:

```text
cargo check -p faultsift-line-access --benches
cargo bench -p faultsift-line-access --bench line_access --no-run
```

Windows PowerShell smoke:

```powershell
$env:FAULTSIFT_LINE_BENCH_SMOKE = "1"
cargo bench -p faultsift-line-access --bench line_access
Remove-Item Env:\FAULTSIFT_LINE_BENCH_SMOKE
```

Linux smoke:

```text
FAULTSIFT_LINE_BENCH_SMOKE=1 cargo bench -p faultsift-line-access --bench line_access
```

CI compiles the benchmark target on Windows and Ubuntu and runs the bounded integration tests through normal Rust tests. It does not run Criterion and has no throughput or latency threshold.

## Recorded Windows Environment

The table below was recorded on 2026-08-31 from base commit `8810e540e6376f09184b217cc0603853f8ca29a9` with `worktree_dirty=true`. The dirty policy includes staged, unstaged, and ordinary untracked files. This accurately identifies the pre-commit FS-009 implementation worktree. A full final-source rerun on 2026-09-01 completed both forced-buffered and mapped groups after the independent-review fixes; an exact implementation-commit confirmation is recorded after local commit.

| Field | Value |
|---|---|
| OS | Windows build 10.0.26200.9168, x86_64 |
| CPU | 11th Gen Intel Core i5-11400, 6 cores / 12 logical processors |
| Memory | 34,067,255,296 bytes physical |
| Filesystem | NTFS, fixed local C: volume |
| Storage | Samsung SSD 870 QVO 1TB, SATA SSD |
| Rust | rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Profile/tool | optimized bench; Criterion 0.7.0 |
| Fixture | 16 MiB target per distribution; 16 MiB huge-line content |
| Seed | `0x46533030395f4c41` |
| Cache | warm; OS page cache uncontrolled |
| Backends | `Buffered(IncompatibleWriter)` and `Mapped(None)` |
| CPU time | not collected reliably |

## Index Build Results

The coverage configuration is explicitly 32 MiB checkpoint budget and 1 MiB scan chunk. It is a measurement configuration, not a default. The newline-dense case instead uses a deliberately constrained 24-byte/three-checkpoint budget to make repeated compaction observable.

Criterion central estimates:

| Fixture | Forced buffered throughput | Mapped throughput | Lines | Final stride | Checkpoints | Payload | Logical capacity | Compactions |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| LF ~80 | 0.920 GiB/s | 1.052 GiB/s | 209,715 | 256 | 820 | 6,560 B | 32 MiB | 0 |
| CRLF ~80 | 0.966 GiB/s | 1.104 GiB/s | 209,715 | 256 | 820 | 6,560 B | 32 MiB | 0 |
| LF ~200 | 1.283 GiB/s | 1.423 GiB/s | 83,886 | 256 | 328 | 2,624 B | 32 MiB | 0 |
| CRLF ~200 | 1.280 GiB/s | 1.486 GiB/s | 83,886 | 256 | 328 | 2,624 B | 32 MiB | 0 |
| LF ~500 | 1.421 GiB/s | 1.678 GiB/s | 33,554 | 256 | 132 | 1,056 B | 32 MiB | 0 |
| CRLF ~500 | 1.340 GiB/s | 1.740 GiB/s | 33,554 | 256 | 132 | 1,056 B | 32 MiB | 0 |
| Newline-dense | 37.89 MiB/s | 38.85 MiB/s | 16,777,216 | 8,388,608 | 2 | 16 B | 24 B | 15 |
| Huge line | 1.513 GiB/s | 1.948 GiB/s | 1 | 256 | 1 | 8 B | 32 MiB | 0 |

The pathological newline-dense result is intentionally separated from representative log throughput. It verifies the compaction and bounded-capacity behavior; it is not an ordinary-log claim.

## Resource Candidate Comparison

The matrix uses the same 16,777,200-byte LF ~200 fixture. All six cases produce 83,886 exact lines, final stride 256, 328 checkpoints, 2,624-byte payload, and zero compactions.

| Checkpoint budget | Scan chunk | Forced buffered | Mapped | Logical checkpoint capacity | Maximum cancellation poll bytes |
|---:|---:|---:|---:|---:|---:|
| 16 MiB | 64 KiB | 1.232 GiB/s | 1.568 GiB/s | 2,097,152 offsets | 64 KiB |
| 16 MiB | 1 MiB | 1.080 GiB/s | 1.467 GiB/s | 2,097,152 offsets | 1 MiB |
| 32 MiB | 64 KiB | 1.257 GiB/s | 1.536 GiB/s | 4,194,304 offsets | 64 KiB |
| 32 MiB | 1 MiB | 1.119 GiB/s | 1.531 GiB/s | 4,194,304 offsets | 1 MiB |
| 64 MiB | 64 KiB | 1.230 GiB/s | 1.596 GiB/s | 8,388,608 offsets | 64 KiB |
| 64 MiB | 1 MiB | 1.252 GiB/s | 1.494 GiB/s | 8,388,608 offsets | 1 MiB |

Differences are small and non-monotonic on this one warm-cache machine. Larger checkpoint budgets increase reserved logical capacity linearly but do not reduce lookup work unless compaction would otherwise occur. A 1 MiB scan chunk permits up to 16× more bytes between cancellation polls than 64 KiB. This matrix is insufficient to approve either a checkpoint-budget or scan-chunk default.

## Cursor Results

Every cursor iteration constructs a content-bearing cursor, visits and checksums every content byte, verifies exact content/physical byte totals, counts descriptors and terminators, and reports both content callback count and scanner chunk count. It never constructs an owned line.

Criterion central throughput estimates:

| Fixture | Forced 64 KiB | Forced 1 MiB | Mapped 64 KiB | Mapped 1 MiB |
|---|---:|---:|---:|---:|
| LF ~80 | 388.08 MiB/s | 414.67 MiB/s | 492.77 MiB/s | 482.56 MiB/s |
| CRLF ~80 | 452.41 MiB/s | 459.62 MiB/s | 494.22 MiB/s | 486.20 MiB/s |
| LF ~200 | 493.33 MiB/s | 501.85 MiB/s | 566.91 MiB/s | 559.22 MiB/s |
| CRLF ~200 | 511.77 MiB/s | 517.53 MiB/s | 557.25 MiB/s | 556.81 MiB/s |
| LF ~500 | 532.83 MiB/s | 532.37 MiB/s | 599.59 MiB/s | 557.92 MiB/s |
| CRLF ~500 | 517.65 MiB/s | 503.83 MiB/s | 596.54 MiB/s | 591.67 MiB/s |
| Newline-dense | 26.39 MiB/s | 26.37 MiB/s | 27.04 MiB/s | 27.23 MiB/s |
| Huge line | 590.13 MiB/s | 580.50 MiB/s | 641.45 MiB/s | 640.84 MiB/s |

The 16 MiB huge line produced 256 content callbacks with a 64 KiB chunk and 16 with a 1 MiB chunk. The file required one additional scanner read because its LF makes the physical length 16 MiB + 1 byte. The complete content checksum was identical across chunks and backends.

## Ready Lookup Results

Typical profiles use 256 fixed seeded line numbers. Pathological and huge-line profiles use 16 to keep the full manual run bounded. Expected descriptors and nearest-checkpoint scanned work are prepared outside timing. The reported percentiles come from individual optimized-build `Instant` samples; Criterion separately measures the complete fixed request batch.

| Backend / fixture | Requests | Final stride | Scanned lines | Scanned bytes | p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|---:|
| Forced LF ~80 | 256 | 256 | 34,618 | 2,769,508 | 447.2 µs | 750.8 µs | 863.5 µs |
| Mapped LF ~80 | 256 | 256 | 34,618 | 2,769,508 | 332.2 µs | 484.1 µs | 596.2 µs |
| Forced CRLF ~200 | 256 | 256 | 32,577 | 6,515,436 | 412.3 µs | 626.6 µs | 746.3 µs |
| Mapped CRLF ~200 | 256 | 256 | 32,577 | 6,515,436 | 340.5 µs | 524.7 µs | 593.4 µs |
| Forced newline-dense | 16 | 8,388,608 | 55,992,964 | 55,992,964 | 89.67 ms | 196.25 ms | 196.25 ms |
| Mapped newline-dense | 16 | 8,388,608 | 55,992,964 | 55,992,964 | 84.52 ms | 193.10 ms | 193.10 ms |
| Forced huge-line target | 16 | 256 | 16 | 268,435,472 | 14.10 ms | 15.68 ms | 15.68 ms |
| Mapped huge-line target | 16 | 256 | 16 | 268,435,472 | 8.44 ms | 9.27 ms | 9.27 ms |

The pathological and huge-line tails are visible rather than hidden behind an average. They are expected consequences of checkpoint-local scanning and the frozen huge-line semantics, not a correctness error.

## `line_range()` Results

The range profile uses the LF ~80 index with stride 256 and covers three-line spans, 4,096-line spans, full-file metadata, empty anchors, and checkpoint-crossing spans. Every result is one 48-byte `LineSpan`; no intermediate descriptor collection is constructed, so result memory remains O(1) with respect to range line count.

| Backend / workload | Requests | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Forced small span | 256 | 1.070 ms | 2.315 ms | 2.692 ms |
| Forced 4,096 lines | 256 | 830.6 µs | 1.337 ms | 1.511 ms |
| Forced full file | 32 | 246.1 µs | 430.4 µs | 436.6 µs |
| Forced empty | 256 | 487.6 µs | 822.7 µs | 1.060 ms |
| Forced checkpoint crossing | 256 | 697.1 µs | 1.283 ms | 1.952 ms |
| Mapped small span | 256 | 642.0 µs | 935.3 µs | 1.054 ms |
| Mapped 4,096 lines | 256 | 652.7 µs | 943.2 µs | 1.127 ms |
| Mapped full file | 32 | 310.4 µs | 362.4 µs | 436.9 µs |
| Mapped empty | 256 | 309.4 µs | 462.9 µs | 623.0 µs |
| Mapped checkpoint crossing | 256 | 582.5 µs | 848.3 µs | 1.504 ms |

Large line-count and full-file ranges do not scan every intermediate line; they locate the two coordinate boundaries under the existing contract. Variability between the two backend runs reinforces that this baseline is not a hard threshold.

## Bounded Reader Results

These are bounded streaming measurements, not owned materialization throughput. Visitors checksum every delivered byte and verify the exact logical byte and callback counts.

| API / workload | Logical bytes | Chunks | Forced central estimate | Mapped central estimate |
|---|---:|---:|---:|---:|
| `visit_line_content`, normal line | 191 | 1 | 288.0 µs | 261.5 µs |
| `visit_line_content`, huge line | 16,777,216 | 16 | 678.69 MiB/s | 860.07 MiB/s |
| `visit_span_physical`, 128 lines | 25,612 | 1 | 86.36 MiB/s | 81.02 MiB/s |
| `visit_span_physical`, full file | 16,777,200 | 16 | 689.28 MiB/s | 859.68 MiB/s |

The normal-line case includes allocating the configured 1 MiB bounded reader buffer for one 191-byte visit. It therefore describes the current independent-call contract, not raw storage throughput.

## Progress, Cancellation, and Concurrency

With a 1 MiB scan chunk:

- the 16,777,216-byte newline-dense file produced exactly 16 progress callbacks despite 16,777,216 lines;
- the 16,777,217-byte huge-line file produced exactly 17 callbacks despite one line;
- cancellation at callback two observed exactly 2,097,152 scanned bytes in both workloads;
- the newline-dense case had completed 2,097,152 lines at that boundary, while the huge line had completed zero;
- callback detection-to-build-return observations were 40.5–56.5 µs across the two Windows backends/workloads;
- an external cancellation request is polled only at a chunk boundary, so up to one configured scan chunk can be consumed before detection. This is not a hard real-time guarantee.

The concurrency workload uses 64 fixed requests per worker:

| Backend | 1 worker | 4 workers |
|---|---:|---:|
| Forced buffered | 1.815 K lookup/s | 3.297 K lookup/s |
| Mapped | 3.046 K lookup/s | 6.544 K lookup/s |

No ideal application concurrency is inferred from one machine. The purpose is to prove a fixed bounded level, correct barrier timing, exact results, and release of all worker/cursor state between samples.

## Memory Evidence

Primary logical bounds are exact:

- checkpoint payload is `checkpoint_count * 8` bytes;
- checkpoint capacity is `checkpoint_budget_bytes / 8` offsets and never exceeds the configured budget;
- the representative 32 MiB configuration reserves 4,194,304 offsets even though the 16 MiB fixtures retain only 132–820 checkpoints;
- the compaction fixture retains two checkpoints in a three-offset/24-byte capacity after 15 in-place compactions;
- scanner and bounded-reader buffers are exactly the configured 64 KiB or 1 MiB per live operation;
- the 16 MiB huge line does not alter either bound and is never retained as an owned line;
- a range result is one 48-byte `LineSpan` regardless of represented line count.

The corrected pre-commit full run observed 6,127,616-byte RSS after fixture creation, 6,160,384 bytes with forced-buffered snapshots open, 8,716,288 bytes after forced-buffered groups, 8,728,576 bytes with mapped snapshots open, 143,130,624 bytes after mapped groups, and 8,908,800 bytes after all snapshots and groups were released. Corresponding VAS observations ranged from 4,355,837,952 bytes initially to 4,496,465,920 bytes while mappings were live, then returned to 4,362,244,096 bytes. These endpoints are supplementary observations, not allocation proofs or peak measurements. Mapped pages, Criterion state, allocator reuse, and OS cache activity contribute to RSS; the drop back after backend release is consistent with the harness not retaining benchmark indexes across groups.

Windows mapped VAS is not resident memory, allocator metadata is not included in the logical checkpoint proof, and OS page cache is outside the Line Access resource contract. No allocator replacement or complex instrumentation was added.

## Default Recommendation

**No default is approved yet.** FS-009 implements no `Default`, `DEFAULT_CHECKPOINT_BUDGET_BYTES`, or `DEFAULT_SCAN_CHUNK_BYTES`.

Evidence supports only these recommendations for a later decision:

- 16 MiB is the lowest measured checkpoint candidate and already has ample capacity for the representative 16 MiB fixtures, but larger real fixtures/distributions are needed to quantify where compaction begins to affect lookup tails.
- 32 and 64 MiB did not produce a consistent throughput advantage here and reserve 2×/4× the logical checkpoint memory of 16 MiB.
- 64 KiB and 1 MiB scan chunks have overlapping, workload-dependent throughput; 64 KiB offers a 16× tighter cancellation polling byte bound, while 1 MiB uses fewer scanner reads/content splits for huge lines.
- Linux performance and a controlled-cache study are absent.

A named default remains a separate product/architecture decision after broader evidence. The candidate values in this document are not API defaults.

## Interpretation Limits

- Antivirus, filesystem compression, page cache, storage firmware, thermal state, background activity, and timer overhead affect results.
- Individual p50/p95/p99 probes use `Instant` around optimized single operations; Criterion batch estimates are the more statistically controlled throughput measurement.
- The printable deterministic fixtures model physical-line distributions, not Java parsing, Search, Pattern, Timeline, UI, or AI work.
- Newline-dense and huge-line cases are pathological boundedness/tail evidence and must remain separate from representative throughput.
- No Linux performance run was performed locally. Linux compilation, Clippy/tests, and GitHub CI are compatibility evidence only.
- No cold-cache, CPU-time, allocator-operation, page-fault, network-storage, removable-media, or greater-than-4-GiB populated throughput result is claimed.
- Future regression gates require controlled hardware, repeated baselines, and a separate approved design. Shared GitHub runners must not enforce these timings.
