# FS-003: Snapshot Validation and Reopen

- Status: Completed
- Owner: Unassigned
- Related ADRs: ADR-0003
- Roadmap stage: M1 — File Access

## Goal

Implement the explicit validation, one-way stale state, file-identity checks, and reopen semantics required for concurrent static `FileSnapshot` lifecycles.

## Context

FS-002 provides fixed-boundary snapshots and safe buffered byte reads. ADR-0003 defines static snapshots as explicit-validation objects, not byte copies and not per-read metadata checks. A snapshot can transition from fresh to stale but can never become fresh again; continued work requires a new snapshot and generation from `reopen()`.

This task makes that lifecycle observable and testable without adding memory mapping or higher-level file concepts.

## In Scope

- capture platform-appropriate file identity and validation metadata when a snapshot opens;
- use the complete opened-handle Windows `FileIdInfo` identity without a legacy 64-bit fallback;
- add explicit `validate()` behavior for unchanged, grown, truncated, modified, replaced, missing, and unverifiable sources;
- implement a thread-safe, one-way fresh-to-stale transition;
- transition to stale when positioned reads encounter unexpected EOF or a backend-detected invalidation condition;
- add `reopen()` returning a distinct snapshot with a new opaque generation;
- keep the captured snapshot length unchanged for the lifetime of the old snapshot;
- provide typed validation and stale reasons without exposing platform identity structures as domain contracts;
- add focused cross-platform mutation and concurrency tests, including child-process coverage where process safety is relevant.

## Out of Scope

- memory mapping or Windows stable mapping handles;
- unsafe Rust outside the minimal audited Windows file-identity FFI module;
- automatic complete metadata validation before every `view()` or `read_at()`;
- copying the source file to create a historical byte-for-byte snapshot;
- guaranteeing detection of an equal-length in-place write when the filesystem exposes no distinguishing metadata;
- watching filesystem events or implementing live-tail behavior;
- automatically reopening or refreshing a snapshot in place;
- Line Index, Parser, Search, Tauri IPC/UI, AI, or any text interpretation;
- benchmark thresholds or final `max_view_bytes` calibration.

## Dependencies

- FS-002 — Safe Byte Access Baseline
- [ADR-0003: Large File Byte Access Strategy](../adr/0003-large-file-byte-access-strategy.md), accepted

## Technical Constraints

- `validate()` is explicit. Normal successful `view()` and `read_at()` calls must not perform a full stat, identity, and timestamp check for every range.
- Static snapshot means fixed file identity, generation, and readable boundary. Growth never extends the old snapshot.
- The state machine permits `Fresh → Stale` only. No code path, including a later successful validation, may transition `Stale → Fresh`.
- Once stale is observed, subsequent byte reads from that snapshot return a typed stale error. A read already in progress may complete according to the documented concurrency contract, but cannot restore freshness.
- `reopen()` creates a new snapshot and generation even if the path still resolves to the same unchanged file. It does not mutate or recycle the old snapshot.
- Compare the identity of the open file and the path's current target using stable platform metadata where available. Do not use a path string alone as file identity.
- Validation errors must never be reported as `Unchanged`. If validation cannot establish a result, return an explicit unverifiable or typed validation error.
- Equal-length in-place modification is best-effort detection only. Do not hash the whole file or copy it to strengthen the guarantee.
- A truncate discovered as early EOF within the captured boundary must make the snapshot stale rather than return a normal partial success.
- A generic, transient, or unclassified `ReadFailed` is returned only for that call and must not transition the snapshot to stale without separate invalidation evidence.
- Lifecycle and backend-neutral identity code remains safe Rust. Only the audited Windows `FileIdInfo` FFI wrapper may contain the minimum required unsafe block, without weakening the workspace lint or other crate modules.
- Do not add filesystem watcher, polling loop, timer, async runtime, or background validation thread.
- No validation result or error may expose a concrete buffered or future mapping backend to domain code.

## Acceptance Criteria

- [x] A newly opened snapshot reports fresh and has an opaque generation and captured identity.
- [x] Explicit validation distinguishes an unchanged source from detected growth, truncate, replacement, deletion, and relevant metadata modification.
- [x] A successful normal read does not implicitly run complete validation.
- [x] A file that grows remains readable within the original boundary until explicit validation detects the change; added bytes are never exposed by the old snapshot.
- [x] Every detected mutation performs a one-way transition to stale.
- [x] A stale snapshot cannot return to fresh after any subsequent validation call.
- [x] Unexpected EOF inside the captured boundary marks the snapshot stale and produces a typed stale reason.
- [x] Generic or transient `ReadFailed` errors do not poison a fresh snapshot, and a later successful read remains possible.
- [x] Windows identity uses the full opened-handle volume serial number and 128-bit file ID, with typed failure and no legacy fallback.
- [x] Symlink snapshots compare resolved target identity and detect a retargeted path.
- [x] `reopen()` returns a separate snapshot with a new generation and metadata captured from the path's current target.
- [x] Concurrent validation and reads are data-race-free and do not introduce a shared seek cursor or serialize all normal reads.
- [x] Unverifiable metadata is surfaced explicitly and is never treated as proof that the snapshot is unchanged.
- [x] Supported mutation scenarios complete with a result or typed error and do not panic or terminate the process abnormally.
- [x] No mapping, line, parser, search, UI, Tauri, or AI behavior is introduced, and unsafe is confined to the audited Windows identity FFI module.

## Test Cases

- Validate an unchanged file repeatedly and verify it remains fresh without changing generation.
- Append bytes after opening; read a range inside the original boundary before validation, then validate and verify a one-way stale transition.
- Truncate below a requested original range and verify unexpected EOF produces stale rather than successful partial data.
- Replace the path with a different regular file and verify identity change is detected even when length is unchanged.
- Delete or make the current path unavailable and verify validation does not report unchanged.
- Perform an equal-length in-place modification; assert stale only when the platform metadata can distinguish it, and document the best-effort limitation without a flaky universal expectation.
- Call `validate()` again after each stale reason and verify state never returns to fresh.
- Reopen after growth, truncate, replace, and unchanged input; verify every returned snapshot has a new generation and its own fixed length.
- Retain the old stale snapshot after reopen and verify it remains stale.
- Race multiple positioned readers with explicit validation and confirm deterministic state transitions without deadlock or global read serialization.
- Run mutation scenarios through a child-process integration harness and assert normal process exit with success or typed errors.
- Inject a generic positioned-read failure; verify the call returns `ReadFailed`, the snapshot stays fresh, and a later normal read succeeds.
- Open through a file symlink and verify identity belongs to the resolved target; after retargeting the symlink, validation must detect replacement. Windows may skip only when symlink capability is unavailable.

## Verification

Run from the repository root:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p faultsift-file-access --test snapshot_lifecycle
cargo test -p faultsift-file-access --test mutation_process
cargo test -p faultsift-file-access
cargo test --workspace
```

Review the lifecycle tests to confirm growth is not detected by an ordinary successful read alone and that no test helper inserts hidden per-read validation into production paths.

## Completion Evidence

Recompleted on 2026-08-26 on the Windows primary platform after resolving the independent review's two blocking findings.

- `cargo fmt --check` passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed with no warnings.
- `cargo test -p faultsift-file-access --test snapshot_lifecycle` passed: 11 tests, 0 failures.
- `cargo test -p faultsift-file-access --test mutation_process` passed: 2 tests, including a child-process mutation harness, 0 failures.
- `cargo test -p faultsift-file-access` passed: 31 tests, 0 failures.
- `cargo test --workspace` passed: 32 tests, 0 failures.
- The bounded unsafe scan found one FFI call in `src/platform/windows/identity.rs` and no unsafe block elsewhere under `crates/**/*.rs`.
- Windows identity uses `GetFileInformationByHandleEx(FileIdInfo)` over the opened handle and compares the complete volume serial number plus 128-bit file ID; query failures become typed identity or validation errors without a legacy 64-bit fallback.
- `same-file` is no longer a dependency of `faultsift-file-access`; its remaining workspace lockfile entry comes transitively from Tauri's `walkdir` dependency and is unrelated to snapshot identity.
- A deterministic injected generic `ReadFailed` leaves the snapshot fresh and is followed by a successful normal read.
- The symlink target lifecycle test passed on the Windows primary platform without capability gating.
- Source inspection confirms normal `view()` and `read_at()` paths perform only an atomic lifecycle check before their existing bounds and positioned-I/O behavior; complete metadata and identity checks occur only in explicit `validate()`.
- Linux identity and metadata code is present behind the Linux target configuration; external Ubuntu CI was not run from this local completion.

## Expected Files

- `crates/faultsift-file-access/src/` snapshot state, identity, validation, reopen, and typed stale reasons;
- `crates/faultsift-file-access/src/platform/windows/identity.rs` audited Windows file-identity FFI;
- `crates/faultsift-file-access/tests/snapshot_lifecycle.rs`;
- `crates/faultsift-file-access/tests/mutation_process.rs` and any bounded child-process test support;
- `Cargo.lock` only if a focused test dependency is justified.

## Risks

- Filesystem timestamps can have coarse resolution, so equal-length writes are not reliably detectable on every supported filesystem.
- Path replacement and open-handle identity differ between Windows and Linux; path metadata alone can misclassify the file.
- A naive per-read stat would satisfy mutation detection tests while causing unacceptable syscall overhead.
- Incorrect synchronization can allow a stale snapshot to appear fresh to another thread or can serialize all range reads.
- Mutation tests can become flaky if they depend on timing rather than explicit process coordination.

## Open Questions

None. Equal-length in-place changes remain explicitly best-effort under ADR-0003 and are not a blocker.

## Review Focus

- absence of hidden metadata validation on ordinary successful reads;
- correctness and irreversibility of `Fresh → Stale`;
- file identity versus path identity on Windows and Linux;
- fixed old-snapshot length and new generation on reopen;
- unexpected EOF and mutation behavior without panic or abnormal process exit;
- concurrency behavior and absence of a global read lock;
- no strengthening of the snapshot contract through whole-file hashing or copying;
- confinement and documentation of unsafe Rust inside audited Windows platform FFI modules, with no later roadmap capabilities.
