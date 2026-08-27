# FS-006: Physical Line Scanner and Cursor Foundation

- Status: Proposed
- Owner: Unassigned
- Related ADRs: ADR-0003, ADR-0004
- Roadmap stage: M2 — Line Access / Index

## Goal

Create the real `faultsift-line-access` crate with one bounded byte scanner and a content-bearing `PhysicalLineCursor` that implements the approved physical-line contract without requiring an index or retaining complete lines.

## Context

[ADR-0004](../adr/0004-physical-line-access-and-adaptive-sparse-index.md) establishes an independent safe-Rust Line Access layer above `faultsift-file-access`. The scanner and cursor are the shared correctness foundation for later adaptive index build and ready lookup tasks. Parser and Search are later consumers and are not implemented here.

## In Scope

- add `crates/faultsift-line-access` as a real Rust workspace member with a normal dependency on `faultsift-file-access`;
- create `crates/faultsift-line-access/AGENTS.md` together with the real crate, recording its safe-Rust, dependency, bounded-memory, and scope rules;
- define checked zero-based line-number, line-terminator, line-descriptor, content-chunk, cursor-state, scan-resource, result, and typed error concepts needed by this task;
- implement one shared bounded scanner/newline state machine for LF, CRLF, lone CR, and EOF-without-newline behavior;
- implement `PhysicalLineCursor` over `Arc<FileSnapshot>` with explicit non-zero scan-chunk configuration and no hidden or named default;
- expose each physical line through ordered borrowed content chunks during a visitor call and return its complete immutable descriptor only after its terminator or EOF is known;
- bind every descriptor to the source `SnapshotGeneration` and exact content and physical `ByteRange` coordinates;
- make visitor or scanner failure terminal for the cursor and prevent a partial descriptor from being returned;
- add focused unit and integration tests for byte preservation, chunk boundaries, line numbering, descriptor coordinates, stale snapshots, and huge-line streaming.

## Out of Scope

- `LineIndex`, checkpoint storage, index build, total-line-count metadata, random line lookup, or line-range lookup;
- `LineRange`, `LineSpan`, descriptor/span rereaders, or offset-to-containing-line lookup;
- progressive, lazy, background, resumable, or persistent indexing;
- owned whole-line APIs, `String`, `Vec<u8>` per line, `Vec<RangeView>`, or a guaranteed single `RangeView` per line;
- a line-size limit or `LineTooLarge` error;
- Java header parsing, Java stack-trace assembly, logical multiline events, Search semantics, Pattern, Timeline, Tauri, UI, CLI commands, or AI;
- parallel scanning, internal worker threads, async runtimes, caching, compression, or prefetching;
- changing File Access semantics, backends, mapping behavior, or unsafe boundaries;
- approving a default scan-chunk size or a performance threshold.

## Dependencies

- FS-002 — Safe Byte Access Baseline, completed
- FS-003 — Snapshot Validation and Reopen, completed
- FS-004 — Windows Conditional Memory Mapping, completed
- FS-005 — File Access Benchmark Baseline, completed
- [ADR-0004: Physical Line Access and Adaptive Sparse Index](../adr/0004-physical-line-access-and-adaptive-sparse-index.md), accepted

## Technical Constraints

- `faultsift-line-access` depends on `faultsift-file-access` and the standard library only. It must not depend on `faultsift-core`, Tauri, React, Parser, Search, an async runtime, or a text-decoding library.
- All crate code is safe Rust. The Windows FFI exceptions in `faultsift-file-access` do not apply to this crate, and the workspace unsafe policy must not be weakened.
- Only LF (`0x0A`) terminates a line. A CR immediately before LF is part of one CRLF terminator and is excluded from content; every other CR is content.
- An empty file has zero lines. Terminal LF/CRLF does not create a trailing phantom line. A final non-empty byte sequence without LF is a line with terminator `None`.
- `content_range` and `physical_range` are checked half-open `u64` byte ranges. They share a start; their end difference is zero, one, or two according to terminator kind.
- The scanner performs no UTF-8 decoding and must preserve invalid UTF-8, NUL, and arbitrary bytes.
- The scan buffer is allocated once per cursor when practical, reused, bounded by explicit configuration, and independent of file length, line count, and single-line length.
- Content chunks are borrowed only for the visitor call, are delivered in byte order without overlap or gaps, exclude terminator bytes, and concatenate exactly to the final descriptor's `content_range`.
- CR at a scan-buffer boundary may require at most bounded pending scanner state; it must not force a line-sized allocation.
- Empty lines may deliver zero content chunks while still returning a valid descriptor.
- A visitor error or File Access error returns immediately, returns no descriptor for the incomplete line, and leaves the cursor explicitly unusable rather than attempting implicit recovery.
- The cursor must use positioned bounded byte access without a shared seek cursor and must obey the bound snapshot's current stale state. It must not call `validate()` implicitly.
- The internal scanner contract introduced here must be reusable by FS-007 rather than copied into a second index-specific newline implementation.

## Acceptance Criteria

- [ ] `faultsift-line-access` is a buildable workspace member containing real scanner/cursor behavior and a scoped `AGENTS.md`, not placeholder modules.
- [ ] The crate's normal dependency graph contains `faultsift-file-access` but no `faultsift-core`, Tauri, React, async runtime, text decoder, Parser, or Search dependency.
- [ ] A cursor streams each non-empty line through ordered bounded borrowed chunks and returns one exact generation-tagged `LineDescriptor` after the line completes.
- [ ] Empty files, empty lines, terminal newlines, and final unterminated lines follow ADR-0004 exactly without phantom lines.
- [ ] LF, CRLF, lone CR, and CRLF split across scan buffers produce the correct content range, physical range, and terminator kind.
- [ ] Invalid UTF-8, NUL, and arbitrary bytes are delivered unchanged without decoding or allocation into a complete owned line.
- [ ] A line larger than the configured scan chunk is delivered through multiple chunks with memory independent of line length and without `LineTooLarge`.
- [ ] Every line's chunk ranges are ordered, non-overlapping, gap-free, and concatenate exactly to its descriptor content range; terminator bytes are never delivered as content.
- [ ] A visitor or read error produces no partial descriptor and makes subsequent cursor use return an explicit terminal-state error.
- [ ] A stale snapshot prevents new cursor byte access through the existing typed stale behavior, and descriptors identify the original snapshot generation.
- [ ] All crate code is safe Rust, and File Access byte/snapshot behavior is unchanged.

## Test Cases

- Scan an empty file and verify the first cursor call returns `None` with no visitor invocation.
- Cover `"\n"`, `"\n\n"`, `"a\n"`, `"a\n\n"`, `"a"`, `"a\nb"`, `"\r"`, and `"\r\n"` with exact zero-based line numbers, ranges, and terminators.
- Exercise LF and CRLF at the first byte, last byte, and every relevant scan-buffer boundary, including a CR as the last byte of one buffer and LF as the first byte of the next.
- Exercise a pending final CR at EOF and verify it is content in an unterminated line.
- Scan consecutive blank lines with a one-byte scan chunk and verify empty lines deliver zero content chunks without callback storms outside normal line visits.
- Deliver invalid UTF-8, embedded NUL, CR, LF, and ordinary bytes and compare visitor bytes and absolute ranges byte-for-byte.
- Generate a line many times larger than the scan buffer using bounded fixture writes; verify multiple chunks, one descriptor, exact coordinates, and no whole-line retention.
- Make the visitor fail after an early chunk; verify no descriptor is returned and every later cursor call reports the terminal failed state.
- Make the snapshot stale before cursor access and verify the existing stale reason is propagated without implicit reopen or validation.
- Exercise checked coordinates and scanner state near and beyond the 4 GiB boundary through focused internal coordinate tests or an environment-gated bounded sparse fixture without allocating the full logical file.
- Compare an exhaustive bounded set of short byte sequences over ordinary byte, CR, LF, NUL, and invalid UTF-8 symbols against a simple test-only reference model across several scan-chunk sizes.

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

The unsafe scan must return no matches. Review the dependency tree and source to confirm no complete-line allocation, hidden scan-size default, duplicate newline state machine, implicit snapshot validation, or later M2 capability is present. Required Windows and Ubuntu CI must pass for the exact pushed commit before the task completes.

## Expected Files

- root `Cargo.toml` workspace membership and `Cargo.lock` only as required;
- `crates/faultsift-line-access/Cargo.toml`;
- `crates/faultsift-line-access/AGENTS.md`;
- `crates/faultsift-line-access/src/` for line types, scan options, errors, scanner, and cursor;
- `crates/faultsift-line-access/tests/` for focused cursor and boundary integration tests.

These paths do not authorize index, parser, search, desktop, UI, or persistence changes.

## Risks

- Emitting a trailing CR before seeing the next byte can misclassify CRLF split across buffers.
- Treating terminal LF as the start of another line can create phantom descriptors and incorrect later indexes.
- A conventional owned-item iterator can hide line-sized allocation or make safe reusable-buffer borrowing impossible.
- Visitor failure recovery can accidentally skip or duplicate bytes if the cursor is allowed to continue.
- Tests that construct huge lines in memory would violate the behavior they intend to prove; fixtures must be streamed and bounded.

## Open Questions

None. Exact Rust naming and generic visitor-error ergonomics may be chosen during implementation only if they preserve the accepted borrowed-chunk, terminal-failure, and no-default contracts.

## Review Focus

- exact LF/CRLF/lone-CR semantics at chunk and EOF boundaries;
- borrowed chunk lifetime, order, coverage, and exclusion of terminators;
- absence of whole-line ownership, decoding, or hidden rereads solely to deliver content;
- terminal behavior after visitor/read failure;
- generation and stale behavior through `Arc<FileSnapshot>`;
- one reusable scanner suitable for FS-007 rather than duplicated boundary logic;
- dependency direction, safe-Rust enforcement, and strict exclusion of later M2 and product capabilities.

