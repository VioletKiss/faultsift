# ADR-0004: Physical Line Access and Adaptive Sparse Index

- Status: Accepted
- Date: 2026-08-27
- Deciders: FaultSift product owner
- Related tasks: FS-006, FS-007, FS-008, FS-009
- Supersedes: None
- Superseded by: None

## Context

FaultSift must locate and stream physical lines in GB to tens-of-GB logs without loading the file, every line, or an arbitrarily long single line into memory. Parser and future Search consumers need single-pass line content, while a raw-log or context viewer needs exact line-number and line-range navigation after indexing. These responsibilities must build on the static `FileSnapshot` contract from [ADR-0003](0003-large-file-byte-access-strategy.md) without making File Access understand lines or weakening snapshot generation and stale semantics.

A full `u64` line-start index is too large at the target scale: a 50 GB file averaging 80 bytes per physical line would require about 4.66 GiB of offsets. A fixed sparse index greatly reduces normal memory use, but a valid pathological file containing billions of one-byte newline-terminated lines can still exceed a practical memory bound. The line layer also has to handle CRLF across scan-buffer boundaries, final input without a newline, invalid UTF-8, and single lines larger than `FileSnapshot::max_view_bytes()`.

## Decision Drivers

- Bounded memory for ordinary and pathological line distributions
- Exact physical-line coordinates and counts after one complete build
- Single-pass content delivery for Parser and future Search consumers
- No whole-line allocation assumption, including 100 MB or 1 GB lines
- Stable binding to one `FileSnapshot` generation and captured length
- Testable newline, chunk-boundary, cancellation, and stale behavior
- Reuse by core, future parser/search code, a future CLI, and desktop adapters without Tauri or React dependencies
- A simple safe-Rust baseline before parallel scanning, caching, compression, or persistence

## Considered Options

### Full line-offset index

Store a `u64` start offset for every physical line. This provides near-direct lookup but consumes hundreds of MiB to several GiB for representative tens-of-GB logs and scales especially poorly for very short lines.

### Fixed sparse index

Store one line-start checkpoint every fixed number of lines and scan locally for lookup. This is simple and compact for representative logs, but checkpoint memory still grows without a strict ceiling for pathological short-line input.

### Lazy or progressive index

Build coverage in response to access. This can expose an early viewport before a complete scan, but requires partial-index states, unknown total counts, coverage contracts, unresolved random lookups, recovery, and more complex concurrency.

### Adaptive sparse index

Build one complete sparse index with an initial stride and an explicit checkpoint budget. When the checkpoint ceiling is reached, compact checkpoints in place by retaining every other checkpoint and doubling the stride. This preserves a complete ready index and exact results while bounding checkpoint storage.

### Hybrid or compressed index

Combine sparse checkpoints with detailed caches, hierarchical byte blocks, fixed-width local offsets, delta encoding, or varints. These can reduce selected lookup or memory costs but add cache invalidation, decoding, synchronization, and verification complexity without current benchmark evidence.

## Decision

Create `faultsift-line-access` as an independent safe-Rust crate. The dependency direction is:

```text
faultsift-core / future parser / future search
                    ↓
        faultsift-line-access
                    ↓
        faultsift-file-access
                    ↓
               OS file APIs
```

`faultsift-file-access` remains byte-only. It continues to own snapshots, byte coordinates and ranges, bounded byte views, positioned reads, identity, lifecycle, generation, and platform backends. It does not define lines, newline interpretation, parser records, search semantics, or UI behavior.

### Physical-line contract

- Only LF (`0x0A`) terminates a physical line.
- A CR immediately before LF forms one CRLF terminator; neither byte belongs to line content.
- A CR not immediately before LF is an ordinary content byte.
- The layer performs no UTF-8 decoding. Invalid UTF-8, NUL, and arbitrary bytes are preserved.
- An empty file has zero physical lines.
- A file ending in LF or CRLF does not gain a fictitious trailing empty line.
- A final non-empty byte sequence without LF is one line with no terminator.
- A line descriptor records its zero-based line number, snapshot generation, content byte range, physical byte range, and terminator kind (`None`, `Lf`, or `CrLf`).

### Sequential access and huge lines

`PhysicalLineCursor` is a content-bearing bounded streaming cursor, not an iterator of owned lines and not a descriptor-only scanner. It uses a fixed reusable scan buffer, supplies ordered non-overlapping content chunks to a visitor during one pass, and returns a complete line descriptor only after the line terminator or EOF is known. Content chunks exclude terminator bytes and cannot outlive their callback.

Empty lines may deliver zero content chunks. A visitor or read failure terminates the current operation without returning a partial descriptor, and the failed cursor is not silently resumed. Arbitrarily long lines produce more bounded chunks; they are not `LineTooLarge` errors and need not fit in one `RangeView`, `String`, byte vector, or collection of views.

### Index build and representation contract

The first index is memory-only and is built eagerly by one complete sequential scan. Exact random lookup and exact total line count are available only from the successfully completed ready index. No partial coverage or progressive-index state is exposed.

- Initial checkpoint stride is 256 lines.
- Stride is always `256 * 2^k`.
- Callers provide an explicit checkpoint byte budget, which is converted to a maximum checkpoint count and must permit at least two `u64` offsets.
- When the ceiling would be exceeded, checkpoint storage is compacted in place by retaining every other checkpoint and doubling the stride with checked arithmetic.
- Compaction reuses the bounded storage and does not allocate another checkpoint collection of comparable size. Storage need not shrink after compaction.
- The exact total physical-line count is maintained independently as `u64`.
- The ready index records final stride, checkpoint count, captured snapshot length, generation, and configured resource limits.
- Checkpoint storage is bounded by the configured checkpoint budget, plus fixed metadata and bounded scan buffers. Allocator bookkeeping, OS page cache, mapping address space, and process RSS are not promised byte-for-byte.

This ADR does not freeze a particular in-memory container, serialized layout, compressed encoding, or future disk representation.

### Build execution and control

Build is single-threaded, synchronous, and single-pass. Line Access creates no worker thread and introduces no async runtime. Callers may run the synchronous operation on an adapter-managed worker.

The builder and `PhysicalLineCursor` share one scanner and CRLF state machine. A build-with-control operation reports monotonic bytes scanned, snapshot length, completed physical lines, current stride, and checkpoint count at bounded scan-chunk boundaries. The infallible control callback may continue or cancel. Cancellation returns a typed error, does not mark the snapshot stale, does not return a partial index, and is not resumable.

### Snapshot binding and lookup

A ready `LineIndex` owns the `Arc<FileSnapshot>` used to build it and remains permanently bound to that snapshot instance, identity, generation, captured length, and lifecycle. Holding the index intentionally keeps the underlying file handle and any mapped stability resources alive.

If the snapshot becomes stale during build, build fails without an index. If it becomes stale after build, metadata such as completed line count, generation, captured length, final stride, and checkpoint count remains inspectable, but every operation that reads source bytes or performs local scanning returns the existing stale error. Checkpoints never authorize stale reads.

`reopen()` produces a new snapshot and generation that require a new index. An old descriptor cannot be used to read from a different generation. Line Access performs no implicit per-lookup `validate()`, path refresh, append indexing, or automatic snapshot replacement.

The ready index exposes exact zero-based line-number lookup and half-open line-range lookup. Line ranges return bounded coordinate metadata rather than collections of lines or implicit whole-range views. Lookups select the nearest checkpoint and perform a bounded-memory local scan; no O(1) latency guarantee is made, and an extremely long line can dominate scan time.

Approximate seek and byte-offset-to-containing-line lookup are deferred. Sequential Parser and Search consumers and future event indexes are expected to preserve line and byte coordinates while scanning.

### Resource configuration and deferred persistence

Callers explicitly configure checkpoint budget and scan-chunk size. Both are validated before scanning, and neither has a default until a reproducible Line Access benchmark provides evidence for a separately approved named default.

The first index is process-local and memory-only. Sidecars, serialization, persistent identity, cache directories, partial files, resume, recovery, invalidation, and disk-format versioning are deferred. `SnapshotGeneration` remains opaque and process-local, and `FileIdentity` remains opaque.

### Safety and scope

`faultsift-line-access` contains safe Rust only. It does not inherit or expand the audited Windows FFI exceptions in `faultsift-file-access`. It has no dependency on `faultsift-core`, Parser, Search, Tauri, React, or AI code.

## Consequences

### Positive

- File Access preserves its stable byte-only boundary.
- Parser and Search can consume physical-line content once through bounded borrowed chunks.
- Exact ready-index lookup is available without retaining raw lines.
- Adaptive compaction bounds checkpoint storage even for pathological short-line input.
- Immutable snapshot binding prevents indexes and descriptors from silently crossing generations.
- The synchronous baseline is deterministic, testable, and adapter-neutral.

### Negative

- Exact random lookup and total count wait for a complete initial scan.
- Sparse lookup requires local rescanning and has no fixed byte-latency bound for huge lines.
- A larger final stride after compaction increases local lookup work.
- Holding a ready index prolongs snapshot handles and Windows mapping stability resources.
- Explicit resource options reduce initial convenience until benchmark-backed defaults are approved.

### Neutral or Follow-up

- `docs/ARCHITECTURE.md` and `docs/ROADMAP.md` must reflect the new layer and milestone sequence.
- The real crate and its scoped `AGENTS.md` are created together in the first implementation task; this decision change creates no placeholder crate.
- A reproducible benchmark will measure scanner throughput, build time, checkpoint memory, compaction, random lookup, cancellation, concurrency, and huge-line behavior without immediately approving defaults or absolute thresholds.
- Parallel scanning, progressive indexing, offset reverse lookup, caches, compression, and persistence require later evidence and separate design approval.

## Validation

Implementation must verify:

- LF, CRLF, lone CR, empty files, consecutive blank lines, final input without newline, invalid UTF-8, and NUL bytes;
- CRLF and final-line correctness across every relevant scan-chunk boundary;
- ordered, non-overlapping, gap-free content chunks and terminal cursor behavior after visitor failure;
- bounded-memory streaming for lines much larger than the scan buffer;
- exact line counts, descriptors, line spans, and empty-range anchors;
- one or more adaptive compactions without exceeding the checkpoint ceiling or allocating a comparable temporary checkpoint collection;
- checked coordinates and line counts beyond 4 GiB;
- cancellation only at bounded chunk boundaries with no partial index or stale transition;
- build-time and ready-index stale behavior, reopen isolation, and descriptor generation mismatch;
- concurrent ready-index lookups without a shared seek cursor or mutable line cache;
- safe-Rust enforcement and the absence of Parser, Search, persistence, Tauri, UI, and AI behavior;
- reproducible performance and memory measurements before any resource default or regression threshold is approved.

## References

- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/ROADMAP.md`
- [ADR-0002: Repository Layout](0002-repository-layout.md)
- [ADR-0003: Large File Byte Access Strategy](0003-large-file-byte-access-strategy.md)
- `docs/benchmarks/file-access.md`

