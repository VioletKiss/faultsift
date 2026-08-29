# FaultSift Architecture

## Responsibility of This Document

This document describes the target system structure, component boundaries, data flow, and architectural invariants currently established for FaultSift.

It does not define product priority, release numbers, or the scope of a particular change. Rationale for durable choices belongs in ADRs; executable work belongs in task specs.

## Current State

M0 established the Rust workspace, the Tauri 2 desktop shell under `apps/desktop/src-tauri`, the React + TypeScript frontend directly under `apps/desktop`, and the Tauri-independent `faultsift-core` crate. The shell contains no FaultSift business capability.

[ADR-0003](adr/0003-large-file-byte-access-strategy.md) establishes `faultsift-file-access` as the byte-only infrastructure layer below core consumers. The safe buffered baseline, snapshot lifecycle, conditional Windows mapping backend, and reproducible benchmark baseline are implemented: regular files have a fixed captured identity, length, and opaque generation; Windows identity uses the opened handle's complete volume serial number and 128-bit file ID; explicit validation can make a snapshot permanently stale; and `reopen()` creates a separate generation. On 64-bit Windows, non-empty resolved targets on fixed local NTFS or ReFS volumes may use a read-only whole-file mapping after a restrictive stability handle proves compatible sharing; all uncertainty or mapping failure retains the already-open buffered snapshot. Checked `u64` byte ranges, bounded views, and caller-buffer reads remain its only data-access concepts. `DEFAULT_MAX_VIEW_BYTES` is a configurable 1 MiB resource guard supported by the FS-005 warm-cache baseline, not a performance promise.

[ADR-0004](adr/0004-physical-line-access-and-adaptive-sparse-index.md) establishes `faultsift-line-access` as the independent safe-Rust layer above File Access. FS-006 implements the physical-line types, explicit scan resources, shared LF/CRLF scanner, and content-bearing bounded cursor. FS-007 implements the complete eager adaptive sparse index build, exact physical-line count, chunk-boundary progress and cancellation, and immutable ready metadata bound to one `Arc<FileSnapshot>`. Exact line/range lookup and the benchmark baseline remain approved but not implemented. Other remaining components in this document likewise describe approved target boundaries rather than implemented features.

## System Context

The target desktop structure is:

```text
apps/desktop/ → apps/desktop/src-tauri/ ┐
                                        ├→ crates/faultsift-core/
future CLI ─────────────────────────────┘           │
                                                    ▼
                                  crates/faultsift-line-access/
                                                    │
                                                    ▼
                                  crates/faultsift-file-access/
                                                    │
                                                    ▼
                                             OS file APIs
```

React source lives directly under `apps/desktop`; there is no separate `web/ui` package. Rust owns file access and analysis. React owns presentation and interaction. Tauri is an adapter between the desktop UI and domain APIs; core analysis must remain usable by a future CLI without depending on Tauri. Optional AI adapters consume selected structured analysis output downstream of deterministic core analysis and do not alter this file-access dependency direction.

## M0 Desktop Bootstrap Baseline

- Product and window name: `FaultSift`
- Bundle identifier: `cn.violetkiss.faultsift`
- Pre-release manifest version: `0.0.0`
- Frontend: React + TypeScript + Vite
- Package manager: pnpm
- Frontend tests: Vitest + React Testing Library
- Frontend lint and format: ESLint + Prettier
- Primary platform: Windows
- Secondary build/test platform: Ubuntu
- Deferred platform: macOS
- License: Apache-2.0

M0 does not include installers, signing, publishing, automatic updates, Storybook, Playwright, Biome, an additional UI framework, or a separate web package.

See [ADR-0001](adr/0001-technology-stack.md) and [ADR-0002](adr/0002-repository-layout.md) for the durable decisions and trade-offs behind this baseline.

## Logical Components

The documented architecture identifies these logical responsibilities:

| Component | Responsibility |
|---|---|
| File Access / FileSnapshot | Stable, read-only, bounded access to local file bytes |
| Physical Line Access / LineIndex | Stream physical-line content and locate exact lines or ranges without retaining all raw lines |
| EventParser | Parse timestamps, levels, threads, loggers, messages, identifiers, and logical event boundaries |
| PatternMiner | Normalize dynamic values and form similar-event patterns |
| TimelineAggregator | Aggregate WARN, ERROR, exception, and later pattern activity into time buckets |
| AnomalyDetector | Later identify first-seen, rare, burst, and frequency-change signals |
| SearchEngine | Later provide text, regex, field, and time-range queries |
| AIContextBuilder | Select and structure incident evidence for optional AI analysis |

`faultsift-file-access`, `faultsift-line-access`, and `faultsift-core` are accepted physical crate boundaries. The other entries remain logical boundaries; they do not imply a separate crate without another approved design or ADR.

## Processing Data Flow

```text
local file bytes
      ↓
bounded byte access
      ↓
physical-line streaming and adaptive sparse line index
      ↓
logical event assembly
      ├── Java header fields
      ├── ordinary text / structured records
      └── complete Java stack trace as one event
      ↓
normalization and pattern identity
      ↓
counts, first/last seen, representative samples
      ↓
WARN / ERROR / pattern time buckets
      ↓
suspicious interval → pattern → original context
      ↓
optional structured incident context for AI
```

Raw log text remains on disk. Indexes and aggregates retain offsets, lengths, fields, identifiers, counts, and other bounded metadata needed for navigation and analysis.

## Large-File Invariants

- Never read an entire log file into a single string, byte vector, or collection of lines.
- Algorithms must have bounded memory relative to file size or document and justify any index that grows with input size.
- Prefer byte slices, offsets, iterators, bounded buffers, and lazy or ranged access over raw-string duplication.
- UI queries return only the current range or aggregate needed for display.
- The frontend never renders or retains the complete result set.
- Offset and length types must remain safe for files larger than 4 GiB.
- Performance-sensitive work needs reproducible benchmarks before enforcing unapproved absolute thresholds.

## Large-File Byte Access

[ADR-0003](adr/0003-large-file-byte-access-strategy.md) establishes these target boundaries:

- `faultsift-file-access` is a byte-only infrastructure crate below `faultsift-core`.
- Opening a supported regular file creates a static `FileSnapshot` with a fixed length, file identity, and generation.
- Snapshots use 64-bit targets, `u64` file coordinates, checked range arithmetic, bounded `RangeView` values, and caller-buffer `read_at` operations.
- Normal reads do not perform complete metadata validation. `validate()` is explicit; unexpected EOF, relevant OS errors, or backend invalidation can also mark a snapshot stale.
- Snapshot state is one-way from fresh to stale. A stale snapshot cannot become fresh; `reopen()` creates a new snapshot and generation.
- Linux uses positioned buffered I/O in the first implementation.
- Windows uses conditional read-only memory mapping only when the resolved target is a non-empty regular file on a fixed local NTFS or ReFS volume, a restrictive read handle can exclude write/delete access, the complete mapping is representable, and mapping creation succeeds; otherwise access falls back transparently to buffered I/O.
- Empty files are valid and are not mapped. Network files, removable media, and uncertain filesystems use buffered access.
- File Access recognizes bytes, not lines, text encodings, parser records, search semantics, or UI concepts. Invalid UTF-8 remains unchanged.
- Snapshots support concurrent positioned reads and do not expose a shared seek cursor or concrete backend to domain code.

The architecture deliberately does not select a Windows mapping crate or binding. The focused Windows file-identity FFI does not constrain that later mapping implementation choice.

The workspace default continues to forbid unsafe Rust. Only the audited Windows platform FFI modules `identity.rs` and `mapping.rs` inside `faultsift-file-access` contain reviewed, minimal unsafe boundaries. They isolate the full Windows file-identity query, resolved-volume eligibility queries, read-only mapping creation, immutable slice construction, and deterministic unmapping. Unsafe Rust anywhere else is a blocking architecture violation unless superseded by another accepted ADR.

## Physical Line Access and Index

[ADR-0004](adr/0004-physical-line-access-and-adaptive-sparse-index.md) establishes these boundaries:

- `faultsift-line-access` is a safe-Rust crate between File Access and core, future parser, future search, CLI, or adapter consumers. It depends on `faultsift-file-access` and does not depend on `faultsift-core`, Tauri, React, Parser, Search, or AI code.
- File Access remains byte-only. Physical newline recognition, line numbers, line ranges, descriptors, cursor state, and index errors belong to Line Access.
- LF terminates a physical line. An immediately preceding CR forms one CRLF terminator and is excluded from content; a lone CR is content. The layer does not decode UTF-8 and preserves invalid bytes and NULs.
- An empty file has zero lines. A terminal LF or CRLF does not create a fictitious trailing empty line. A final non-empty byte sequence without LF is an unterminated line.
- `PhysicalLineCursor` is a content-bearing bounded streaming cursor. It supplies ordered borrowed content chunks from a fixed reusable buffer and returns a complete immutable descriptor only when the line boundary is known. Empty lines may supply no content chunks. A failed visitor or read does not yield a partial descriptor or silently resume the cursor.
- A physical line need not fit in one allocation or `RangeView`. Arbitrarily long lines are streamed as more bounded chunks and are not line-access errors.
- A `LineDescriptor` identifies its snapshot generation, zero-based line number, content range, physical range, and terminator. A `LineSpan` represents one half-open line range and one contiguous physical byte range without retaining a line collection.
- Exact line-number lookup, exact line-range lookup, and exact total line count are available only after one complete eager index build. Sequential cursor access does not require an index.
- The sparse index starts at stride 256 and has an explicit checkpoint budget. When its checkpoint ceiling would be exceeded, it compacts in place by retaining every other checkpoint and doubling stride. Total line count remains exact and independent of checkpoint density.
- Checkpoint storage is bounded by its configured budget, plus fixed metadata and bounded per-builder or per-lookup scan buffers. The architecture does not prescribe a serialized layout, compressed encoding, hierarchical structure, or byte-for-byte process RSS bound.
- Build is synchronous, single-threaded, and single-pass. A chunk-boundary control callback reports monotonic progress and may cancel without producing a partial index, changing snapshot lifecycle, or enabling resume. Adapters may run the synchronous build on their own worker.
- A ready `LineIndex` owns the `Arc<FileSnapshot>` from which it was built. It remains bound to that snapshot instance, identity, generation, captured length, lifecycle, and backend resources. `reopen()` requires a new index.
- Stale snapshots reject every index or cursor operation that needs source bytes or local scanning. Completed index metadata remains inspectable, but checkpoints and old descriptors never authorize stale or cross-generation reads. Line Access does not add implicit per-lookup validation or refresh.
- Ready lookup selects the nearest checkpoint and scans locally with bounded memory. No O(1) lookup or fixed byte-latency guarantee is made. Offset-to-containing-line lookup and approximate seek are deferred.
- The first index is process-local and memory-only. Persistence, sidecars, cache directories, serialization, partial files, resume, recovery, and disk-format versioning are deferred.
- Checkpoint budget and scan-chunk size are explicit non-zero resource options. No named defaults are approved until a reproducible Line Access benchmark provides evidence for a separate decision.

## Event and Parsing Invariants

The initial parser focus is Java application logs. Parser implementations consume the unified physical-line contract from `faultsift-line-access` and must not reinterpret or independently trim LF/CRLF terminators. A stack trace containing frames, `Caused by`, `Suppressed`, or common-frame omission markers belongs to one logical event.

Relevant file and parser designs must address the applicable cases among:

- LF and CRLF;
- empty files;
- EOF without a final newline;
- invalid UTF-8;
- extremely long lines;
- incomplete or non-exception multiline input;
- files growing, changing, or being truncated while open;
- Windows and Linux file behavior.

Exact timestamp formats, timezone rules, event-boundary heuristics, and invalid-input recovery remain component design decisions.

## Pattern and Timeline Boundaries

Pattern processing is expected to recognize dynamic values such as UUIDs, IP addresses, numbers, dates and times, URLs, paths, hexadecimal values, trace/request/session identifiers, and dynamic business identifiers.

The exact normalization rules, collision behavior, fingerprint inputs, and whether a Drain-style algorithm is used are not yet decided. Do not hard-code those choices into an unrelated task.

The first product MVP requires WARN and ERROR timelines. Additional views for exceptions, patterns, first-seen events, bursts, threads, loggers, and identifiers are later extensions.

## AI Boundary

AI is downstream of deterministic analysis:

```text
parser + patterns + timeline + selected context
                        ↓
             structured incident payload
                        ↓
                 optional model
```

An AI adapter must not receive the entire log file. Core behavior and tests cannot depend on a model being present. External APIs are opt-in and user-configured; local processing remains the default.

## Dependency Direction

The intended dependency direction is:

```text
desktop UI → Tauri adapter ┐
                           ├→ faultsift-core ────────────┐
future CLI ────────────────┘                              │
future parser / search ──────────────────────────────────┤
                                                        ▼
                                          faultsift-line-access
                                                        │
                                                        ▼
                                          faultsift-file-access
                                                        │
                                                        ▼
                                                   OS file APIs

AI adapter ← selected structured analysis outputs
```

`faultsift-core` owns domain and orchestration behavior. `faultsift-line-access` owns physical-line streaming and indexing. `faultsift-file-access` owns file snapshots and backend-neutral byte ranges. None of these core layers depends on desktop UI concepts. Dependencies never point from File Access back to Line Access or from Line Access back to core/parser/search consumers. React must not become a second implementation of file access, parsing, indexing, search, pattern, timeline, or anomaly logic.

## Architectural Open Questions

- Beyond the accepted `faultsift-core`, `faultsift-line-access`, and `faultsift-file-access` boundaries, what future Rust crate splits are justified by demonstrated contracts?
- What precisely defines pattern identity, especially for exception type, message normalization, and stack context?
- What timestamp, timezone, malformed-input, and mixed-format rules form the Java parser contract?
- What are the Tauri range-query and cancellation/progress contracts?

Resolve durable, high-impact choices through design review and an ADR rather than silently embedding them in implementation.
