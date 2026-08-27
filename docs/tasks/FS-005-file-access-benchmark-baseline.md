# FS-005: File Access Benchmark Baseline

- Status: In Progress
- Owner: Codex
- Related ADRs: ADR-0003
- Roadmap stage: M1 — File Access

## Goal

Establish a reproducible performance and memory baseline for the completed byte-access abstraction, including buffered and conditional Windows mapping behavior, without inventing product performance thresholds.

## Context

ADR-0003 treats performance as product behavior but deliberately leaves the exact `max_view_bytes` default and regression thresholds to evidence. FS-002 through FS-004 provide the byte contract, lifecycle, and platform backends. This task makes their access patterns measurable and records a conservative implementation default without extending File Access into line-oriented or higher-level analysis.

## In Scope

- add a stable-compatible benchmark harness dedicated to `faultsift-file-access`;
- generate deterministic bounded fixtures without committing large log files;
- benchmark sequential and seeded random byte ranges through `RangeView` and `read_at`;
- compare forced-buffered and automatic conditional mapping on eligible Windows files;
- compile and exercise the buffered benchmark path on Linux;
- cover representative range sizes, bounded concurrency levels, warm-cache runs, and clearly documented cold-cache limitations;
- record open latency, throughput or operation latency, allocation/copy behavior, and process memory observations available in the execution environment;
- include fixtures and access positions around the 4 GiB boundary without allocating or checking in an entire multi-GB byte vector;
- choose and document a conservative named default for `max_view_bytes` based on bounded-memory behavior and benchmark evidence;
- document the benchmark method, environment, commands, backend diagnostics, baseline results, and interpretation limits;
- ensure the benchmark harness compiles in the supported Windows and Ubuntu verification paths without running noisy performance gates in CI.

## Out of Scope

- absolute pass/fail performance targets, release SLAs, or CI regression thresholds;
- Line Index, line scanning, parser throughput, search, pattern, timeline, anomaly, AI, or UI benchmarks;
- changing static snapshot, stale, reopen, fallback, or unsafe-boundary decisions;
- adding prefetching, caching, window management, compression, asynchronous I/O, or other optimizations merely to improve benchmark numbers;
- mapping on Linux, network storage, removable media, or macOS support;
- committing generated GB-scale fixtures or benchmark result binaries;
- Tauri IPC or frontend performance work.

## Dependencies

- FS-002 — Safe Byte Access Baseline
- FS-003 — Snapshot Validation and Reopen
- FS-004 — Windows Conditional Memory Mapping
- [ADR-0003: Large File Byte Access Strategy](../adr/0003-large-file-byte-access-strategy.md), accepted

## Technical Constraints

- Use fixed fixture contents, fixed random seeds, recorded range sizes, and recorded concurrency levels so runs are comparable.
- Fixture generation must stream or use sparse files; it must not construct the complete large fixture in memory.
- Do not infer physical-disk performance from sparse zero pages alone. Label sparse-boundary checks separately from representative populated-file measurements.
- The benchmark must report which backend actually ran. A Windows mapping comparison is invalid if the file fell back to buffered access.
- `RangeView` retention and concurrent operations must remain explicitly bounded during every benchmark case.
- Record OS version, CPU, memory, filesystem, storage class when known, Rust profile, fixture size, access pattern, and cache condition with the result.
- Treat cold-cache control as platform-specific and best-effort; never label an uncontrolled run cold.
- Do not run full performance benchmarks as blocking CI jobs. CI may compile the harness and run bounded correctness smoke cases only.
- Do not choose an absolute regression threshold until repeated measurements on controlled hardware justify a later approved decision.
- The selected `max_view_bytes` default is an implementation resource guard, not a product performance promise. It remains configurable and must not enable whole-file access.
- Benchmark-only dependencies must not enter normal runtime dependency paths unless independently justified.
- No unsafe Rust may be added by the benchmark code or outside the FS-004 Windows mapping boundary.

## Acceptance Criteria

- [x] A deterministic benchmark harness runs on stable Rust through one documented command.
- [x] Sequential and seeded random access cover both `RangeView` and caller-buffer `read_at`.
- [x] Windows results distinguish automatic mapped access from forced-buffered access using diagnostics rather than assumptions.
- [ ] Linux compiles and runs the buffered benchmark path without any mapping implementation.
- [x] Benchmark cases use bounded live views, buffers, fixture generation, and concurrency independent of total file size.
- [x] Greater-than-4-GiB coordinate coverage uses a bounded sparse fixture and is clearly separated from populated-file throughput results.
- [x] The recorded baseline includes environment, fixture, access-pattern, cache-condition, backend, latency/throughput, and available memory observations.
- [x] A conservative, configurable named default for `max_view_bytes` is documented with its resource rationale and evidence.
- [ ] CI or its existing Rust jobs compile the benchmark harness on Windows and Ubuntu without enforcing noisy timing thresholds.
- [x] No benchmark adds or changes file-access semantics, unsafe boundaries, or later product capabilities.

## Test Cases

- Run sequential reads over a populated bounded fixture with several fixed range sizes and verify the benchmark consumes the expected total byte count.
- Run seeded random reads twice and verify the generated offset sequence is identical.
- Compare `RangeView` and `read_at` for identical ranges and verify byte counts and backend diagnostics before interpreting timing.
- On eligible Windows storage, run automatic mapping and forced-buffered cases and confirm the reported backends differ as expected.
- On Windows fallback conditions and on Linux, verify the harness reports buffered access rather than mislabeling results as mapped.
- Generate a sparse file beyond 4 GiB, access sentinel regions around the boundary, and verify fixture creation and benchmark memory remain bounded.
- Hold the configured maximum number of concurrent views used by a case and confirm the case does not retain views between iterations unintentionally.
- Compile the benchmark harness without running it on both supported CI platforms.
- Run a bounded smoke configuration suitable for correctness checks and confirm it does not depend on cache dropping, administrator rights, external services, or checked-in large fixtures.

## Verification

Run from the repository root:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p faultsift-file-access
cargo test --workspace
cargo check -p faultsift-file-access --benches
cargo bench -p faultsift-file-access --bench byte_access --no-run
cargo bench -p faultsift-file-access --bench byte_access
```

Review the generated baseline document and confirm that it reports actual backend diagnostics, environment, fixture construction, cache limitations, and memory methodology. External Windows/Ubuntu CI compilation must be reported separately and must not be claimed as passed from a local benchmark run.

## Local Completion Evidence

Implementation and Windows local verification completed on 2026-08-27. Independent review returned `PASS_WITH_WARNINGS`; only exact-SHA remote CI and Ubuntu runtime evidence remain pending.

- Criterion 0.7.0 is a benchmark-only dev-dependency with default features disabled and only `cargo_bench_support` enabled. Version 0.7 was selected over 0.8 because 0.8 unconditionally introduced a native C build dependency that prevented the supported Windows-to-Linux bench cross-check without an unrelated cross-C toolchain.
- The populated fixture defaults to 64 MiB, streams deterministic bytes through a 1 MiB setup buffer, and uses hard maximums of 1 GiB fixture size and 256 MiB workload size for manual overrides.
- Sequential and xorshift64* seeded-random ranges use fixed seed `0x46533030355f5255`. Tests prove repeated generation is identical and a different seed changes the sequence.
- The Windows full run reported `Buffered(IncompatibleWriter)` for the forced case and `Mapped` with no fallback for the automatic case. Backends are measured serially against the same fixture so the mapped stability handle cannot interfere with the writer used to prove buffered fallback.
- The open-latency group acquires its forced-buffered writer guard outside the timed loop, so writer setup is excluded and both cases time only snapshot/backend establishment. Benchmark dirty metadata checks both unstaged and staged tracked changes against `HEAD`.
- The full warm-cache run covered `view` and reused caller-buffer `read_at`, 4 KiB / 64 KiB / 1 MiB / 8 MiB ranges, sequential and seeded-random patterns, and concurrency 1 / 4. The concurrency timer starts before the main thread releases the worker start barrier; the corrected concurrency group was rerun after independent review. No cold-cache claim or timing threshold was made.
- The >4-GiB fixture reported logical length 4,295,032,832 bytes, confirmed the Windows sparse flag, verified both boundary sentinels, selected `Mapped`, increased VAS by its logical length while open, and returned VAS to the prior level after drop without a corresponding RSS increase.
- `DEFAULT_MAX_VIEW_BYTES` and `FileAccessOptions::default()` use a 1 MiB bound. The baseline shows that 8 MiB has no consistent copied-byte advantage while increasing each possible buffered live view by 8×; the value is documented as a resource guard, not a performance target.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed.
- `cargo test -p faultsift-file-access` passed: 48 tests, 0 failures.
- `cargo test --workspace` passed: 49 tests, 0 failures.
- `cargo check -p faultsift-file-access --benches` passed.
- `cargo bench -p faultsift-file-access --bench byte_access --no-run` passed.
- `cargo bench -p faultsift-file-access --bench byte_access` passed locally on Windows with Criterion 0.7.0. The recorded result remains explicitly tied to the current tracked working tree and will be rerun for the exact implementation commit before completion.
- `cargo check -p faultsift-file-access --target x86_64-unknown-linux-gnu --benches` passed.
- `cargo clippy -p faultsift-file-access --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings` passed. This is cross-compilation evidence only; Ubuntu execution remains pending exact-SHA CI.
- `.github/workflows/ci.yml` adds bench compilation to both Rust matrix platforms. It does not run performance measurements or enforce timing thresholds.
- No unsafe Rust, backend control in production options, Linux mapping, line-index, parser, search, UI, or AI capability was added.
- Independent review initially found a blocking concurrency timer boundary and two measurement-attribution warnings. All three were fixed, their affected measurements were rerun, and re-review returned `PASS_WITH_WARNINGS` with no Blocking issues; the remaining warnings are exact-SHA CI and Ubuntu runtime evidence only.

## Expected Files

- `crates/faultsift-file-access/benches/byte_access.rs` and focused benchmark support;
- `crates/faultsift-file-access/Cargo.toml` and `Cargo.lock` for a benchmark-only dependency if justified;
- bounded fixture helpers under the crate's test/benchmark support area;
- `docs/benchmarks/file-access.md` or an equivalently focused reproducibility and baseline document;
- a minimal `.github/workflows/ci.yml` adjustment only if required to compile benches on Windows and Ubuntu;
- the named `max_view_bytes` default and focused tests under `crates/faultsift-file-access`.

## Risks

- OS page cache, filesystem compression, antivirus, storage hardware, and background load can dominate results and produce misleading backend comparisons.
- Sparse files can make memory and throughput results look unrealistically favorable.
- A benchmark that fails to verify backend diagnostics can compare buffered access with itself.
- Excessive fixture sizes or concurrency can turn the benchmark into an unbounded local or CI resource consumer.
- Choosing a default solely for benchmark throughput can violate the more important bounded-memory requirement.
- Adding optimization work to make the baseline look better would hide current behavior and expand task scope.

## Open Questions

None. This task records a baseline and a conservative configurable implementation default; any absolute product target or automated regression gate requires later approval.

## Review Focus

- reproducibility of fixtures, seeds, commands, and environment recording;
- separation of sparse boundary checks from populated-file performance claims;
- truthful backend selection and cache-condition reporting;
- bounded memory, live views, and concurrency during every case;
- evidence supporting the configurable `max_view_bytes` default;
- absence of invented thresholds and benchmark-driven scope expansion;
- no changes to unsafe policy, public semantics, or later roadmap capabilities.
