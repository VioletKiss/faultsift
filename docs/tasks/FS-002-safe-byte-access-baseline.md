# FS-002: Safe Byte Access Baseline

- Status: Completed
- Owner: Unassigned
- Related ADRs: ADR-0003
- Roadmap stage: M1 — File Access

## Goal

Deliver the first usable, backend-neutral `faultsift-file-access` crate with safe positioned buffered reads, bounded byte views, checked file coordinates, and no whole-file loading.

## Context

[ADR-0003](../adr/0003-large-file-byte-access-strategy.md) establishes `faultsift-file-access` as the byte-only infrastructure layer below `faultsift-core`. Buffered positioned I/O is the required cross-platform baseline and the fallback for every later optimization.

This task implements that baseline without file-mutation validation or memory mapping. It must establish the special crate lint configuration required by ADR-0003 without weakening the workspace-wide unsafe policy.

## In Scope

- add `crates/faultsift-file-access` as a real Rust workspace member;
- configure the crate's lint policy so unsafe Rust is denied locally while every existing ordinary crate continues to inherit the workspace `unsafe_code = "forbid"` policy;
- add a concise scoped `AGENTS.md` for the file-access crate, including the approved future Windows mapping exception and the rule that all current FS-002 code remains safe Rust;
- define backend-neutral byte coordinate and range types using `u64`;
- provide checked construction and platform conversion for byte ranges;
- define typed errors for open, unsupported file type, range overflow, out-of-bounds access, oversized views, unrepresentable offsets, unexpected EOF, and I/O failure;
- open seekable regular files as immutable snapshots with a captured length and opaque generation;
- implement positioned buffered reads without a shared seek cursor on Windows and Linux;
- provide an opaque, immutable buffered `RangeView` limited by `max_view_bytes`;
- provide `read_at` into a caller-owned buffer;
- expose backend information only through diagnostics suitable for tests and benchmarks;
- add focused unit and integration tests for the safe baseline.

## Out of Scope

- `validate()` metadata checks, file-identity comparison, `reopen()`, or the complete stale lifecycle beyond reporting an unexpected EOF;
- memory mapping or any mapping dependency;
- unsafe Rust anywhere in the repository;
- Windows handle-sharing optimization or filesystem capability detection;
- final calibration of the default `max_view_bytes` value;
- Line Index, line iteration, line-ending interpretation, or extremely-long-line policy;
- UTF-8 decoding or conversion to `String`;
- Parser, Search, Pattern, Timeline, Anomaly, AI, Tauri IPC, UI, CLI, or MCP behavior;
- changes to product requirements or accepted architecture.

## Dependencies

- FS-001 — Repository Bootstrap, completed
- [ADR-0003: Large File Byte Access Strategy](../adr/0003-large-file-byte-access-strategy.md), accepted
- `AGENTS.md` and `crates/AGENTS.md`

## Technical Constraints

- Support only 64-bit targets; unsupported target widths must fail clearly at compile time.
- Use `u64` for file offsets, lengths, ranges, and generation-independent file coordinates. Do not use `u32` for any value that can identify a byte position or file-sized count.
- Construct ranges with checked arithmetic. Convert to `usize` or an OS offset type only after checking representability and the configured per-view bound.
- Use half-open byte ranges `[offset, offset + length)`.
- `RangeView` is exact-range: the complete requested range must fit within the captured snapshot length.
- `read_at` reads at most the remaining bytes in the captured snapshot. An offset equal to the snapshot length returns zero; a larger offset is out of bounds.
- An unexpected EOF before the captured boundary is not a normal short read and must be represented by the approved typed error so FS-003 can transition the snapshot to stale.
- Accept empty regular files. Do not special-case them through a mapping API.
- Reject directories, pipes, devices, and other non-regular or non-seekable objects with a typed error.
- Return invalid UTF-8 and arbitrary binary bytes unchanged.
- Do not call complete metadata validation before every `view` or `read_at` operation.
- Snapshots and their read operations must be `Send + Sync` and must not serialize all reads through a shared seek-position mutex.
- Memory retained internally must be independent of total file size. A buffered view may allocate only its checked requested length, capped by `max_view_bytes`.
- Until FS-005 calibrates a default, callers must be able to supply an explicit non-zero `max_view_bytes`. Any provisional named default must be documented as an implementation default, not a product performance target.
- Do not add an async runtime or a general caching layer.
- `faultsift-file-access` must not depend on `faultsift-core`, Tauri, React, or desktop types.
- Do not globally change workspace `unsafe_code` to `allow`, `warn`, or `deny`. Any lint-table adjustment must preserve `forbid` for existing ordinary crates and `deny` unsafe Rust throughout the new crate in this task.

## Acceptance Criteria

- [x] `faultsift-file-access` is a buildable workspace member containing real buffered byte-access behavior rather than placeholder modules.
- [x] Existing ordinary crates still forbid unsafe Rust, while the new crate has an explicit local deny-by-default policy suitable for the later narrowly scoped Windows exception.
- [x] All Rust source introduced by this task is safe Rust.
- [x] A regular file can be opened as a snapshot without reading the complete file into memory.
- [x] Snapshot length and every public file coordinate remain correct beyond 4 GiB.
- [x] Byte range construction rejects arithmetic overflow before indexing or allocation.
- [x] `RangeView` returns the exact requested bytes and rejects out-of-bounds and over-limit requests.
- [x] `read_at` correctly handles normal reads, partial final buffers, exact EOF, and offsets beyond EOF.
- [x] Empty files open successfully and support an empty view and zero-byte EOF read.
- [x] Invalid UTF-8 bytes are returned exactly as stored.
- [x] Multiple threads can read independent ranges from one snapshot without a shared seek cursor.
- [x] Unsupported file types and relevant I/O failures produce stable typed errors rather than panic.
- [x] File-access code contains no line, parser, search, UI, Tauri, or AI behavior.

## Test Cases

- Open a small regular file and verify several exact beginning, middle, and ending ranges.
- Read into a caller buffer smaller than, equal to, and larger than the bytes remaining before snapshot EOF.
- Verify `read_at(snapshot_len, buffer)` returns zero and `read_at(snapshot_len + 1, buffer)` returns out of bounds.
- Open a zero-byte file; verify the empty view succeeds and every non-empty range fails safely.
- Read bytes containing invalid UTF-8, embedded NUL, CR, and LF and compare byte-for-byte without interpreting them.
- Construct ranges at `u64::MAX`, including an overflowing `offset + length`, and verify typed errors without allocation or panic.
- Use a bounded sparse fixture larger than 4 GiB with sentinel bytes around the 4 GiB boundary; verify offsets on both sides without whole-file allocation.
- Request exactly `max_view_bytes` and one byte more.
- Concurrently read deterministic disjoint and overlapping ranges from one snapshot and compare the results with the source bytes.
- Attempt to open a directory and any reliably constructible non-regular object for the platform.
- Simulate or provoke an early EOF after snapshot length capture and verify it is not reported as a successful short read.

## Verification

Run from the repository root:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p faultsift-file-access
cargo test --workspace
cargo tree -p faultsift-file-access --edges normal
```

Manual bounded checks:

```text
rg -n '\bunsafe\s+(fn|trait|impl|extern)|\bunsafe\s*\{' crates --glob '*.rs'
```

The unsafe scan must return no matches after FS-002. Inspect the dependency tree to confirm `faultsift-file-access` has no dependency on `faultsift-core`, Tauri, React, an async runtime, or a mapping library.

## Completion Evidence

Completed on 2026-08-26 on the Windows primary platform.

- `cargo fmt --check` passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed with no warnings.
- `cargo test -p faultsift-file-access` passed: 16 tests, 0 failures.
- `cargo test --workspace` passed: 17 tests, 0 failures.
- `cargo tree -p faultsift-file-access --edges normal` reports no normal dependencies.
- The bounded unsafe scan returned no matches under `crates/**/*.rs`.
- The greater-than-4-GiB sparse fixture executed successfully rather than being skipped.
- Linux behavior remains covered by the Ubuntu CI matrix; external CI was not run from this local completion.

## Expected Files

- root `Cargo.toml` workspace membership and lint configuration only as required;
- `Cargo.lock` if dependencies change;
- `crates/faultsift-file-access/Cargo.toml`;
- `crates/faultsift-file-access/AGENTS.md`;
- `crates/faultsift-file-access/src/` safe byte types, errors, snapshot, buffered backend, and diagnostics;
- `crates/faultsift-file-access/tests/` focused integration tests.

These paths are guidance, not permission to modify Tauri, React, product, parser, search, or UI areas.

## Risks

- Incorrect checked conversions can make a 64-bit API appear safe while still truncating at an OS or slice boundary.
- A buffered implementation that uses a shared seek cursor can pass single-threaded tests but corrupt concurrent results.
- A large sparse test can consume real disk space on filesystems without sparse-file behavior; tests must probe capability or skip with a recorded reason rather than create an unbounded fixture.
- Opting the new crate out of inherited workspace lints can accidentally drop unrelated lint coverage; preserve equivalent non-unsafe lint rigor locally.
- A public backend trait or mapping-specific type introduced prematurely would freeze implementation details that ADR-0003 deliberately hides.

## Open Questions

None. The exact conservative default for `max_view_bytes` is intentionally assigned to FS-005 after reproducible measurements; FS-002 must keep the bound explicit and configurable.

## Review Focus

- file-access/core/Tauri dependency direction;
- absence of whole-file APIs and file-size-proportional raw-byte retention;
- `u64` coordinates, checked arithmetic, and `usize` conversion boundaries;
- exact EOF, empty-file, invalid-byte, and greater-than-4-GiB behavior;
- positioned concurrent reads with no shared seek cursor;
- typed errors rather than panic paths;
- preservation of workspace unsafe lint policy and absence of unsafe Rust;
- strict exclusion of Line Index and all later capabilities.
