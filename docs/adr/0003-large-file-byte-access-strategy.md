# ADR-0003: Large File Byte Access Strategy

- Status: Accepted
- Date: 2026-08-26
- Deciders: FaultSift product owner
- Related tasks: FS-002, FS-003, FS-004, FS-005
- Supersedes: None
- Superseded by: None

## Context

FaultSift must inspect GB to tens-of-GB log files without loading the whole file into memory. The byte-access foundation must support bounded random reads, concurrent consumers, and files larger than 4 GiB while remaining reusable by domain logic, a future CLI, and the Tauri desktop adapter.

Read-only memory mapping can provide efficient random access, but mapping a file that another process may truncate or replace has platform-specific safety consequences. Linux cannot use advisory locks to prove that a mapping will remain valid. Windows can establish stronger sharing constraints for suitable local files, but those constraints may be unavailable for an actively written file or an unsupported filesystem.

The workspace currently forbids unsafe Rust. A mapping backend requires a small, auditable unsafe boundary, without weakening the rule for domain, parser, index, pattern, timeline, or other analysis code.

## Decision Drivers

- Bounded memory for files larger than available RAM
- No process crash caused by supported file-mutation scenarios
- Stable byte-range semantics independent of the selected backend
- Correct offsets and lengths beyond 4 GiB
- Windows as the primary platform, with Linux support
- Reuse by core domain logic, a future CLI, and the desktop adapter
- Concurrent reads without a shared seek cursor
- Strict containment and review of unsafe Rust
- Freedom to replace a mapping library without changing architecture

## Considered Options

### Direct memory mapping

Expose a file mapping or borrowed mapping slices directly to callers. This offers simple zero-copy access, but leaks backend lifetimes and platform behavior into domain code. It also cannot provide the required mutation-safety contract across Windows and Linux.

### Positioned buffered I/O only

Use bounded caller buffers and positioned reads on every platform. This provides the simplest safety model and remains the baseline implementation, but it gives up a useful Windows optimization where a stable mapping can be established.

### Stable abstraction with buffered baseline and conditional mapping

Expose a backend-neutral snapshot and byte-range API. Use positioned buffered I/O as the safe baseline and a conditional read-only Windows mapping only when its safety preconditions are established. Fall back transparently when they are not.

## Decision

Choose a stable byte-range abstraction with positioned buffered I/O as the baseline and conditional read-only memory mapping as a Windows optimization.

The approved dependency direction is:

```text
desktop adapter / future CLI
             ↓
       faultsift-core
             ↓
 faultsift-file-access
             ↓
       OS file APIs
```

`faultsift-file-access` is a justified infrastructure crate rather than a speculative package. It owns file handles, snapshots, byte ranges, backend selection, and platform-specific file access. `faultsift-core` remains the domain and orchestration layer above it. Neither layer depends on Tauri or React.

### Snapshot semantics

- Opening a supported regular file creates an immutable `FileSnapshot` with a fixed byte length, opaque generation, and captured file identity.
- A snapshot fixes the readable boundary; it is not a byte-for-byte copy of the source file.
- `validate()` is an explicit operation. Normal `view()` and `read_at()` calls do not perform a complete metadata, identity, and timestamp validation before every read.
- Reads may mark a snapshot stale when they encounter an unexpected EOF, an OS error indicating invalidation, or another backend-detected identity anomaly.
- A snapshot state transition is one-way:

```text
Fresh
  │
  ├── explicit validation detects change
  ├── unexpected EOF
  └── backend detects invalidation
  ▼
Stale
```

- A stale snapshot never becomes fresh through another validation call.
- Continuing with the current path requires `reopen()`, which returns a new `FileSnapshot` with a new generation.
- File growth does not extend an existing snapshot or expose bytes beyond its captured length.
- On Linux, equal-length in-place writes that cannot be detected through available metadata may produce mixed-version bytes. The layer guarantees bounded reads and no crash, but not a historical byte copy against an uncooperative writer.

Static snapshot therefore means a fixed identity and byte boundary with explicit validation, not re-proving file immutability before every range read.

### Public byte-access contract

- File Access understands bytes only. It does not define lines, text encoding, parsing, search, or logical events.
- Invalid UTF-8 is returned unchanged and is not an error at this layer.
- Only 64-bit targets are supported initially.
- File offsets, lengths, ranges, and persistent file coordinates use `u64`.
- Range construction and platform conversion use checked arithmetic. `usize` is limited to validated per-view or per-buffer sizes.
- `RangeView` provides an opaque, immutable, bounded byte view whose lifetime keeps its storage valid.
- `read_at` reads into a caller-provided buffer for allocation reuse.
- `RangeView` is limited by a configurable `max_view_bytes`; larger logical ranges are processed in chunks.
- Snapshots support concurrent positioned reads and have no shared seek cursor.
- The concrete backend is not part of domain behavior. Backend identity and fallback reasons may be exposed only for diagnostics, tests, and benchmarks.

### Platform backends

- Linux uses positioned buffered I/O in the first implementation. Advisory locking is not treated as proof that a mapping is safe.
- Windows may use a read-only mapping only for a suitable local regular file after establishing handle-sharing conditions that exclude existing writers and prevent write, truncate, rename, or delete operations for the mapping lifetime.
- Empty files are valid snapshots but are never mapped.
- Network files, removable media, unsupported file types, uncertain filesystem capabilities, failed stability checks, and mapping failures use the buffered backend or return a typed open error if buffered access also fails.
- Mapping failure is transparent to domain code when buffered fallback succeeds.
- The architecture does not select `memmap2`, `windows-sys`, the `windows` crate, or direct Win32 calls. That choice belongs to the Windows mapping implementation task and may change without superseding this ADR.

### Unsafe boundary and lint policy

- The workspace default remains `unsafe_code = "forbid"` for `faultsift-core` and all other ordinary crates.
- Only `faultsift-file-access` may opt out of the workspace-level `forbid`, solely because Rust does not allow a nested module to override `forbid`.
- Within `faultsift-file-access`, unsafe Rust remains denied by default. A narrowly scoped platform module such as `platform/windows/mapping` may explicitly permit the minimum unsafe operations required for the mapping implementation.
- Snapshot logic, range validation, buffered I/O, backend selection, and all non-Windows code remain safe Rust.
- Every unsafe operation must document the safety invariant that makes the mapping valid and must be covered by focused platform tests and review.
- Unsafe Rust anywhere outside the approved Windows mapping boundary is a blocking architecture violation unless a later accepted ADR explicitly changes this rule.

## Consequences

### Positive

- Core consumers use one stable byte API on Windows and Linux.
- Normal reads avoid a metadata syscall and identity check for every range.
- Buffered I/O provides a safe fallback for active files and uncertain environments.
- Conditional mapping can improve Windows random access without becoming a domain dependency.
- One-way stale state keeps concurrent snapshot behavior deterministic.
- The physical crate boundary makes unsafe review and lint enforcement explicit.
- A mapping implementation or library can be replaced without changing the public architecture.

### Negative

- Two backend implementations and transparent fallback add testing and diagnostic complexity.
- Windows files already held by a writer may not qualify for mapping.
- Linux buffered `RangeView` creation requires a bounded copy.
- Explicit validation means callers must choose appropriate workflow boundaries at which to check for source changes.
- Linux cannot guarantee historical byte consistency against undetected equal-length in-place writes without copying the source file.

### Neutral or Follow-up

- The exact `max_view_bytes` default requires implementation validation and benchmarks; this ADR sets no unapproved absolute performance threshold.
- The first task derived from this decision must define the special crate lint configuration without weakening the workspace default.
- Line indexing, parsing, search, UI, and AI remain outside this decision.
- This ADR approves the target architecture; it does not claim that `faultsift-file-access` is already implemented.

## Validation

Implementation must verify:

- empty files and exact byte preservation, including invalid UTF-8;
- checked range arithmetic and offsets on both sides of the 4 GiB boundary;
- concurrent positioned reads without a shared cursor;
- explicit validation with a one-way fresh-to-stale transition;
- growth, truncate, replace, and unexpected-EOF behavior in platform integration tests;
- new generation creation through reopen;
- Windows mapping preconditions and transparent buffered fallback;
- Linux buffered behavior under concurrent source mutation;
- process-level mutation tests that terminate normally with a result or typed error;
- reproducible backend benchmarks without invented absolute thresholds;
- lint enforcement that rejects unsafe Rust outside the approved mapping module.

## References

- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- [ADR-0001: Technology Stack](0001-technology-stack.md)
- [ADR-0002: Repository Layout](0002-repository-layout.md)
- [Microsoft: Creating a File Mapping Object](https://learn.microsoft.com/en-us/windows/win32/memory/creating-a-file-mapping-object)
- [Microsoft: CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
- [Linux mmap(2)](https://man7.org/linux/man-pages/man2/mmap.2.html)
