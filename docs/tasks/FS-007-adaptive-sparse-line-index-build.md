# FS-007: Adaptive Sparse Line Index Build

- Status: Proposed
- Owner: Unassigned
- Related ADRs: ADR-0003, ADR-0004
- Roadmap stage: M2 — Line Access / Index

## Goal

Build one complete memory-only `LineIndex` with an exact physical-line count and strictly budgeted adaptive sparse checkpoints, including synchronous chunk-boundary progress and cancellation, without adding random lookup yet.

## Context

FS-006 supplies the approved physical-line scanner and cursor semantics. [ADR-0004](../adr/0004-physical-line-access-and-adaptive-sparse-index.md) chooses a complete eager index with initial stride 256, explicit resource limits, in-place adaptive compaction, and ready-only results so target-scale files and pathological short-line inputs remain bounded.

## In Scope

- define `LineIndexOptions` with explicit `checkpoint_budget_bytes` and `scan_chunk_bytes`, early validation, and no `Default` implementation;
- derive a maximum checkpoint count from the checkpoint byte budget and require capacity for at least two `u64` offsets;
- implement a memory-only `LineIndex` that owns the source `Arc<FileSnapshot>` and records generation, captured length, exact line count, final stride, checkpoint count, and configured resource metadata;
- implement `LineIndex::build` and synchronous `build_with_control` using the FS-006 scanner/newline state machine for one complete sequential pass;
- start with stride 256 and maintain stride as `256 * 2^k` using checked arithmetic;
- compact checkpoint storage in place when its ceiling would be exceeded, retaining every other checkpoint, doubling stride, and reusing bounded storage without a comparable temporary checkpoint collection;
- maintain exact `u64` physical-line count independently of checkpoint density;
- report monotonic chunk-boundary `BuildProgress` and accept infallible `Continue` or `Cancel` control;
- return a typed cancellation result with no partial index, coverage, resume state, or snapshot lifecycle change;
- propagate File Access/stale/allocation/arithmetic errors without returning an index;
- expose ready-index metadata introspection required to verify the build, but not line-number or line-range lookup;
- add focused build, budget, compaction, progress, cancellation, snapshot, and pathological-input tests.

## Out of Scope

- `line(number)`, `line_range`, `LineSpan`, descriptor/span chunk readers, cursor-from-line convenience, offset-to-line lookup, or approximate seek;
- partial, progressive, lazy, background, resumable, incremental-append, or persistent indexing;
- parallel newline scanning, prefix-sum reconciliation, internal workers, async runtimes, mutable caches, compression, varints, delta blocks, hierarchical indexes, or byte-aware checkpoint levels;
- a checkpoint disk format, serialization, sidecar, cache key, persistent identity, or recovery policy;
- approving or implementing named defaults for checkpoint budget or scan-chunk size;
- benchmarking or optimization work assigned to FS-009;
- Parser, Search semantics, logical events, Pattern, Timeline, Tauri, UI, CLI commands, or AI;
- changes to File Access semantics, backends, snapshot validation, mapping, or unsafe boundaries.

## Dependencies

- FS-006 — Physical Line Scanner and Cursor Foundation
- [ADR-0004: Physical Line Access and Adaptive Sparse Index](../adr/0004-physical-line-access-and-adaptive-sparse-index.md), accepted

## Technical Constraints

- The builder must reuse the FS-006 scanner and physical-line state machine. No second LF/CRLF implementation is permitted.
- Build is single-threaded, synchronous, single-pass, and allocation-bounded. The crate creates no worker thread and adds no async runtime.
- `checkpoint_budget_bytes` and `scan_chunk_bytes` are explicit non-zero values. Configuration and representability errors must be returned before file scanning begins.
- `max_checkpoints` is the floor of checkpoint budget divided by `size_of::<u64>()` and must be at least two. The logical checkpoint storage count/capacity may not exceed that ceiling; allocator metadata and exact RSS are outside the byte-for-byte promise.
- Initial stride is 256. Every later stride is produced only by checked doubling and remains `256 * 2^k`.
- A non-empty file has a line-zero checkpoint at byte zero. An empty file has no line checkpoint. A terminal LF must not create a checkpoint for a nonexistent trailing line.
- When the next checkpoint would exceed the ceiling, compaction must happen in place, keep every other checkpoint in order, reuse the existing bounded storage, and avoid allocating a second collection of comparable size. Shrinking after compaction is not required.
- The exact line count remains independent of compaction and uses checked `u64` arithmetic.
- A successfully returned `LineIndex` proves that the scanner consumed the complete captured snapshot boundary and produced one ready metadata set for that generation. No intermediate `LineIndex` value may escape.
- Progress callback frequency is tied to bounded scan chunks, not lines. Bytes scanned and completed lines are monotonic; checkpoint count stays within budget; stride only stays equal or increases by the compaction rule.
- Cancellation is observed between bounded reads/chunks, returns `IndexBuildCancelled`, does not poison or stale the snapshot, and cannot be resumed.
- The index owns the exact `Arc<FileSnapshot>` passed to build. It does not store a path for refresh, accept a substitute snapshot, reopen, or expose persistent identity.
- Build performs no implicit complete `validate()`. It obeys current snapshot lifecycle checks and propagates actual read/stale errors according to ADR-0003.
- The task must not freeze the chosen in-memory container or checkpoint layout as a future persistent format.

## Acceptance Criteria

- [ ] Invalid checkpoint or scan options fail with typed errors before the first source scan, and `LineIndexOptions` has no `Default` implementation or hidden magic-number fallback.
- [ ] Build performs one complete sequential scan using the FS-006 scanner and returns one ready memory-only `LineIndex` only after the captured boundary is consumed.
- [ ] Ready metadata reports exact generation, captured length, total physical-line count, final stride, checkpoint count, checkpoint budget, and scan-chunk size.
- [ ] Initial stride is 256 and every compaction doubles it while preserving valid ordered checkpoints for the new stride.
- [ ] Checkpoint count/capacity never exceeds the derived ceiling, including pathological short-line input and repeated compaction.
- [ ] Compaction reuses bounded storage and does not allocate a comparable temporary checkpoint collection or require shrink.
- [ ] Empty, LF/CRLF, blank-line, and final-unterminated fixtures produce exact total counts without phantom lines.
- [ ] A newline-dense fixture can trigger multiple compactions while exact total count and retained checkpoint coordinates remain correct.
- [ ] Progress callbacks occur at scan-chunk boundaries, remain monotonic, expose only observation/control data, and do not scale with physical-line count for a newline-only fixture.
- [ ] Successful final progress reports captured snapshot length and exact total line count.
- [ ] Cancellation returns `IndexBuildCancelled`, returns no index or coverage, leaves the snapshot lifecycle unchanged, and requires a new build to retry.
- [ ] Snapshot stale/read/allocation/arithmetic failure returns a typed error and no index.
- [ ] `LineIndex` retains the original `Arc<FileSnapshot>` and cannot bind or switch to a reopened generation.
- [ ] No random lookup, persistence, parallel scan, resource defaults, Parser, Search, Tauri, UI, or AI behavior is introduced.

## Test Cases

- Reject zero checkpoint budget, a budget that holds fewer than two `u64` offsets, zero scan chunk, an unrepresentable scan chunk, and relevant arithmetic overflow before scanning.
- Build empty, one-line, newline-only, CRLF, consecutive-empty-line, and final-no-newline fixtures; compare exact line counts with the FS-006 reference behavior.
- Build a non-empty file and verify line-zero checkpoint at offset zero; build a file ending in newline and verify no checkpoint or count for a phantom trailing line.
- Use a tiny valid checkpoint ceiling and enough one-byte lines to force one and then several compactions; verify stride progression, retained offset order, exact line count, and final checkpoint metadata.
- Exercise a ceiling that is not a power of two and verify compaction still produces checkpoints aligned to the doubled stride.
- Track internal test instrumentation or equivalent evidence showing checkpoint count/capacity remains within the ceiling and compaction does not create a comparable temporary collection.
- Use a newline-only fixture with a small scan chunk and verify progress callback count follows scanned chunks rather than line count.
- Cancel at the first, middle, and final pre-completion callback; verify no index, no resumable state, and a fresh snapshot remains fresh.
- Complete a build and verify the final progress values exactly equal captured length and ready line count.
- Make the bound snapshot stale during a controlled build and verify build fails without a ready or partial index.
- Reopen the same path and verify the new snapshot has a different generation and requires a separate build.
- Exercise checked line count, stride, and checkpoint coordinates around and beyond 4 GiB through focused internal state tests or an environment-gated bounded fixture without whole-file memory allocation.

## Verification

Run from the repository root:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p faultsift-line-access
cargo test --workspace
cargo tree -p faultsift-line-access --edges normal
rg -n '\bunsafe\s+(fn|trait|impl|extern)|\bunsafe\s*\{' crates/faultsift-line-access --glob '*.rs'
```

Inspect focused tests or bounded instrumentation proving checkpoint ceiling and in-place compaction behavior. Inspect production source to confirm one shared scanner, no `Default` or hidden resource constants, no partial index state, and no internal thread/async runtime. Required Windows and Ubuntu CI must pass for the exact pushed commit.

## Expected Files

- focused additions under `crates/faultsift-line-access/src/` for options, index metadata, build control/progress, checkpoint state, and builder orchestration;
- focused additions under `crates/faultsift-line-access/tests/` for build, compaction, cancellation, and snapshot lifecycle;
- `crates/faultsift-line-access/AGENTS.md` only if the real crate rules require clarification discovered during implementation;
- `Cargo.lock` only if an independently justified test-only dependency is required.

No parser, search, desktop, UI, persistence, benchmark-result, or file-access backend files are expected.

## Risks

- Committing a checkpoint immediately after terminal LF can reintroduce a phantom line.
- Incorrect in-place compaction can retain offsets that no longer match implicit line numbers at the doubled stride.
- Ordinary `Vec` growth can exceed the logical checkpoint ceiling unless capacity changes are explicitly bounded.
- A cancellation callback tied to lines instead of chunks can become unbounded on newline-dense input.
- Build can appear complete after cancellation or stale failure if intermediate state is exposed through the ready type.
- Holding `Arc<FileSnapshot>` intentionally prolongs mapped stability/file handles and must not be weakened with a path reopen or weak reference.

## Open Questions

None. Resource values remain caller-supplied; FS-009 will measure candidates but cannot approve defaults by itself.

## Review Focus

- exact adaptive-stride and retained-checkpoint invariants across repeated compaction;
- checkpoint ceiling enforcement including storage capacity and absence of comparable temporary collections;
- separation of exact line count from checkpoint density;
- final-line and empty-file behavior inherited from the shared scanner;
- progress frequency, monotonic fields, cancellation latency boundary, and no partial result;
- snapshot ownership, stale failure, reopen isolation, and no implicit validation;
- absence of lookup, persistence, parallelism, compression, defaults, and later product scope.

