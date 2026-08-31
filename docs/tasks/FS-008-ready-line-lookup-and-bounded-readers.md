# FS-008: Ready Line Lookup and Bounded Readers

- Status: Completed
- Owner: Unassigned
- Related ADRs: ADR-0003, ADR-0004
- Roadmap stage: M2 — Line Access / Index

## Goal

Expose exact ready-index line-number and line-range lookup plus generation-safe bounded content/physical readers, without retaining line collections or introducing reverse/approximate lookup.

## Context

FS-007 produces an immutable complete `LineIndex` with exact count and adaptive sparse checkpoints but no public random lookup. This task turns that ready metadata into the exact navigation contract required by future raw-log/context consumers while preserving sparse local-scan, huge-line, snapshot-stale, and bounded-result semantics from [ADR-0004](../adr/0004-physical-line-access-and-adaptive-sparse-index.md).

## In Scope

- define checked zero-based half-open `LineRange` and immutable generation-tagged `LineSpan` concepts;
- implement exact `LineIndex::line(LineNumber) -> LineDescriptor` by selecting the nearest checkpoint and using the shared bounded scanner for local forward scanning;
- implement exact `LineIndex::line_range(LineRange) -> LineSpan` by locating only the required endpoint boundaries without constructing intermediate descriptors;
- support every valid empty range `[n, n)`, including `[line_count, line_count)`, with the approved zero-length byte anchor;
- implement bounded visitor-based reading of one descriptor's content bytes, excluding terminators;
- implement bounded visitor-based reading of one span's physical bytes, including all stored terminators exactly as present;
- validate descriptor/span generation against the owning index before byte access and reject coordinates from another reopened snapshot generation;
- obey ready-index stale behavior for every operation requiring source bytes or local scanning while leaving pure completed metadata introspection available;
- support concurrent immutable lookups without a shared seek cursor, global line lock, or mutable detailed cache;
- add focused exact-lookup, line-range, chunk-reader, huge-line, generation, stale, and concurrency tests.

## Out of Scope

- approximate line seek, approximate byte seek, offset-to-containing-line, `line_containing(offset)`, or any O(1) lookup guarantee;
- progressive, partial, lazy, background, append, resumable, or persistent indexes;
- detailed-segment caches, LRU state, compression, hierarchical indexes, delta/varint encoding, or checkpoint-strategy changes;
- `Vec<LineDescriptor>`, `Vec<RangeView>`, complete line/span ownership, `String`, `read_entire_line`, or an implicit full-span `RangeView`;
- `LineTooLarge` for ordinary descriptor/span access;
- cursor-from-line convenience; it may be proposed later only if a real consumer requires it;
- changing build, compaction, progress, cancellation, or resource-option defaults from FS-007;
- Parser, Java stack traces, Search semantics, Pattern, Timeline, Tauri IPC, UI, CLI commands, or AI;
- persistence, sidecars, serialization, cache keys, or disk format.

## Dependencies

- FS-007 — Adaptive Sparse Line Index Build
- [ADR-0004: Physical Line Access and Adaptive Sparse Index](../adr/0004-physical-line-access-and-adaptive-sparse-index.md), accepted

## Technical Constraints

- Line numbers are zero-based. `line(number)` accepts only `number < line_count`; `line(line_count)` is out of bounds.
- `LineRange` is half-open `[start, end)`, requires `start <= end`, and requires `end <= line_count`.
- For non-empty ranges, `LineSpan::physical_range` starts at the first line's physical start and ends at the last included line's physical end.
- `[n, n)` anchors to `line(n).physical_range.start` when `n < line_count`; `[line_count, line_count)` anchors to `snapshot_length`. Empty spans read zero chunks.
- `line_range` returns O(1)-sized metadata and must not materialize descriptors or raw bytes for the intervening lines. Endpoint lookup may perform only the sparse local scanning required to locate those boundaries.
- Lookup chooses the checkpoint implied by final stride and scans with a bounded reusable buffer. No fixed byte-latency or O(1) performance promise is made; a huge line can dominate scan time.
- All lookup and read coordinates use checked `u64` arithmetic and remain correct beyond 4 GiB.
- `LineDescriptor` content reading excludes LF/CRLF. `LineSpan` physical reading includes terminator bytes. No multi-line “strip all terminators and concatenate” behavior is defined.
- Descriptor/span readers use bounded chunks and do not assume the byte range fits `max_view_bytes`, one allocation, or one `RangeView`.
- A descriptor/span is immutable metadata, not proof that bytes remain readable. Every read verifies generation and current snapshot freshness before and during bounded access as provided by File Access.
- Checkpoint hits do not bypass stale checks. No lookup calls `validate()` implicitly or refreshes/reopens by path.
- Ready `LineIndex` remains immutable and `Send + Sync` as practical. Independent concurrent lookups use independent bounded scanner state and no mutable cache.
- The task must reuse the FS-006 scanner/newline contract for local descriptor discovery; it must not implement a lookup-specific CRLF parser.

## Acceptance Criteria

- [x] Every valid line number returns the exact generation-tagged descriptor, and `line_count` or larger returns a typed bounds error.
- [x] Descriptor coordinates and terminator kind match a sequential FS-006 cursor over the same snapshot for LF, CRLF, lone CR, blank, invalid-byte, and final-unterminated cases.
- [x] Every valid non-empty `LineRange` returns one exact O(1)-sized `LineSpan` without constructing the intervening line descriptors or bytes.
- [x] Invalid reversed or out-of-bounds ranges return typed errors.
- [x] Every valid empty range has the approved zero-length anchor, including EOF on empty and non-empty files.
- [x] Descriptor content readers deliver exactly `content_range` in ordered bounded chunks and never deliver terminators.
- [x] Span physical readers deliver exactly `physical_range` in ordered bounded chunks including original LF/CRLF bytes, and empty spans deliver no chunks.
- [x] A huge single line and a huge line span remain bounded-memory and do not produce `LineTooLarge`, owned complete bytes, or a view collection.
- [x] Old-generation descriptors/spans are rejected by a new index after `reopen()` before source bytes are read.
- [x] A stale bound snapshot causes every byte-reading or local-scanning lookup/reader to return the existing stale reason, while ready metadata introspection remains available.
- [x] Multiple threads can perform deterministic independent lookups on one ready index without a shared seek cursor, global line lock, mutable detailed cache, or data race.
- [x] No offset reverse lookup, approximate seek, persistence, resource defaults, Parser, Search, Tauri, UI, or AI behavior is introduced.

## Test Cases

- Compare `line(n)` for every line in bounded LF, CRLF, mixed-CR, blank-line, invalid-UTF-8, NUL, and final-unterminated fixtures against sequential cursor descriptors.
- Exercise targets at checkpoint zero, immediately before and after checkpoints, the final checkpoint interval, and after one or more adaptive stride compactions.
- Verify `line(line_count)` and a larger checked value return typed out-of-bounds errors without scanning.
- Verify ranges `[0, 0)`, `[0, line_count)`, `[n, n)`, `[n, n + 1)`, and `[line_count, line_count)` with exact physical anchors and bounds.
- Request a very large line range and confirm the returned structure remains one `LineSpan` with no intermediate descriptor collection.
- Stream descriptor content for LF and CRLF lines and verify the visitor never receives terminator bytes.
- Stream a multi-line physical span and verify LF/CRLF bytes are included byte-for-byte and chunk concatenation equals the span range.
- Stream empty descriptor content and empty span physical bytes; verify zero visitor chunks and successful completion.
- Use a line much larger than `scan_chunk_bytes` and `FileSnapshot::max_view_bytes`; verify exact descriptor plus multiple bounded read chunks without a size error.
- Build two generations through `reopen()` and attempt to read old descriptors/spans with the new index; verify typed generation mismatch before I/O.
- Mark a ready index's bound snapshot stale through growth, truncate, or replacement validation; verify metadata getters remain available and all local-scanning/reader operations fail.
- Perform seeded concurrent lookups and reads from one index; compare exact results and enforce bounded live buffers per worker.
- Exercise ranges and lookup coordinates beyond 4 GiB through focused internal coordinate tests or an environment-gated bounded fixture without loading the logical file.

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

Inspect focused tests and source to confirm large ranges return constant-sized metadata, readers remain chunked, stale/generation checks precede byte access, and no reverse/approximate lookup or mutable cache exists. Required Windows and Ubuntu CI must pass for the exact pushed commit.

## Completion Evidence

Implemented, independently reviewed, and remotely verified on 2026-08-31.

- `LineIndex::line` checks snapshot freshness and bounds, selects `floor(line_number / final_stride)` by checked `u64` arithmetic and direct checkpoint indexing, then creates an operation-local `ByteScanner` at that exact checkpoint offset. It scans at most the local checkpoint interval plus the required target line and returns the scanner's exact descriptor; it never rescans from file start unless checkpoint zero is the nearest checkpoint.
- `LineRange` is checked, zero-based, and half-open. `LineIndex::line_range` returns one generation-tagged `LineSpan`; it locates only the start boundary and last included line, retains no intermediate descriptors or bytes, and implements every approved empty anchor including `[line_count, line_count)` at captured EOF.
- `visit_line_content` streams only `content_range` through borrowed ordered chunks; `visit_span_physical` streams the contiguous raw `physical_range` including LF/CRLF. Both allocate at most one operation-local `scan_chunk_bytes` buffer, skip allocation for empty ranges, preserve visitor error types, and do not poison the immutable index or snapshot after consumer failure.
- Descriptor and span readers reject reopened-generation coordinates with typed mismatch errors before byte access. Lookup and reader operations check the bound snapshot lifecycle, propagate File Access errors, preserve the existing `UnexpectedEof` stale transition, and leave ready metadata inspectable after stale.
- The only production LF/CRLF and pending-CR state machine remains `ByteScanner`. Lookup adds only an exact-start constructor and does not add a second newline parser, shared seek cursor, mutable descriptor cache, global lock, or unsafe Rust.
- Focused tests compare every lookup descriptor with the FS-006 sequential cursor across empty, LF, CRLF, lone-CR, blank, invalid-byte, NUL, and final-unterminated cases. They cover checkpoint boundaries, multiple adaptive final strides, huge-line readers, exact physical span bytes, empty readers, visitor failure recovery, reopen mismatch, stale/read failure, 100,000-line O(1)-sized span metadata, deterministic parallel operations, and coordinates beyond 4 GiB.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed.
- `cargo test -p faultsift-line-access` passed 55 tests across four suites.
- `cargo test --workspace` passed 104 tests across the workspace suites.
- `cargo tree -p faultsift-line-access --edges normal` showed only the expected direct `faultsift-file-access` dependency and its existing platform dependencies; no dependency was added or removed.
- `cargo check -p faultsift-line-access --all-targets --all-features --target x86_64-unknown-linux-gnu` and the matching target Clippy command passed.
- `git diff --check` passed, and the required unsafe scan returned no matches under `crates/faultsift-line-access/**/*.rs`.
- Independent `faultsift-review` returned `PASS_WITH_WARNINGS` with no Blocking issue, source defect, material memory concern, dependency concern, or scope violation. Its sole warning was the then-pending exact-pushed-commit CI evidence, resolved by the run below.
- Implementation commit `64956530b3f20cec2e311d430eda2b52042486da` passed exact-SHA [GitHub Actions run 33347263272](https://github.com/VioletKiss/faultsift/actions/runs/33347263272): Frontend quality, Rust Windows, and Rust Ubuntu all completed with `success`.
- No cursor-from-line convenience, reverse/approximate lookup, persistence, benchmark/default work, Parser, Search, Tauri, UI, or AI behavior was introduced.

## Expected Files

- focused additions under `crates/faultsift-line-access/src/` for line ranges, spans, ready lookup, generation validation, and bounded readers;
- focused additions under `crates/faultsift-line-access/tests/` for lookup, range, readers, stale generations, huge lines, and concurrency;
- `Cargo.lock` only if an independently justified test-only dependency is required.

No file-access backend, parser, search, desktop, UI, persistence, or benchmark-result files are expected.

## Risks

- Looking up span endpoints by iterating every intervening line would make large ranges unexpectedly linear and may retain unbounded metadata.
- Treating descriptor content and span physical bytes as the same operation can silently drop or duplicate terminators.
- A checkpoint-aligned target can accidentally bypass stale or generation checks because its start offset is already known.
- Huge lines make byte-latency unbounded even though memory is bounded; public docs and tests must not imply constant-time lookup.
- A shared scratch buffer or mutable hot cache can serialize or race otherwise independent lookups.

## Open Questions

None. Cursor-from-line convenience remains deliberately unimplemented until a concrete consumer proves it necessary.

## Review Focus

- exact descriptor parity with the sequential cursor across all newline and EOF cases;
- direct checkpoint selection plus bounded local scan without an O(1) claim;
- O(1)-sized line-span result and correct empty-range anchors;
- precise separation of content-only and physical-span readers;
- huge-line bounded memory and absence of whole-range views/collections;
- stale and generation checks on checkpoint hits and readers;
- concurrent immutable lookup with no shared seek cursor or mutable cache;
- absence of reverse lookup, persistence, defaults, parser/search semantics, and adapter/UI scope.
