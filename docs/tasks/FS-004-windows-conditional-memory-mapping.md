# FS-004: Windows Conditional Memory Mapping

- Status: Proposed
- Owner: Unassigned
- Related ADRs: ADR-0003
- Roadmap stage: M1 — File Access

## Goal

Add a conditional read-only Windows memory-mapping backend that preserves the existing `FileSnapshot` byte contract, proves its file-stability preconditions, and falls back transparently to safe buffered I/O.

## Context

ADR-0003 permits memory mapping only as a Windows optimization behind the backend-neutral byte-access abstraction. Linux remains buffered. Mapping is allowed only when FaultSift can hold a suitable local regular file stable against existing and future write, truncate, rename, and delete operations for the full mapping lifetime.

This task owns the repository's sole approved unsafe boundary. It must not weaken unsafe linting elsewhere or turn a mapping implementation choice into a durable architecture dependency.

## In Scope

- evaluate current official documentation and select one focused Windows mapping/binding implementation for this task;
- add only the dependency required by that implementation and record the choice and rationale in completion evidence;
- conservatively identify Windows files eligible for mapping, treating uncertain, network, and removable storage as buffered-only;
- acquire and retain a Windows file handle whose sharing conditions exclude existing writers and prevent new write, truncate, rename, or delete access while mapped;
- create a read-only mapping only after every stability precondition succeeds;
- keep the stable file handle and mapped storage alive for every dependent `RangeView` lifetime;
- implement mapped `RangeView` access without exposing mapping types to callers;
- implement `read_at` with the same public semantics regardless of backend;
- use buffered fallback when the file is empty, ineligible, actively writable, too large for the selected mapping representation, cannot be mapped, or has uncertain capabilities;
- expose selected backend and fallback reason only through diagnostics;
- confine all required unsafe Rust to one reviewed module under `crates/faultsift-file-access/src/platform/windows/mapping.rs` or an equivalently single, explicitly documented file;
- add Windows-specific safety, fallback, lifetime, and mutation tests while keeping Linux builds and tests buffered-only.

## Out of Scope

- Linux, macOS, network-filesystem, or removable-media memory mapping;
- direct exposure of a mapping object, mapping library, OS handle, or backend trait to `faultsift-core`;
- requiring mmap for correctness or returning a mapping error when buffered fallback succeeds;
- bypassing stability checks through a force-mmap production option;
- changing static snapshot, validation, stale, reopen, range, or error semantics approved by ADR-0003;
- mapping actively growing log files while a writer remains open;
- general-purpose window cache, prefetching, read-ahead policy, or whole-file byte caching;
- Line Index, Parser, Search, Tauri IPC/UI, AI, or CLI work;
- global relaxation of workspace unsafe linting.

## Dependencies

- FS-002 — Safe Byte Access Baseline
- FS-003 — Snapshot Validation and Reopen
- [ADR-0003: Large File Byte Access Strategy](../adr/0003-large-file-byte-access-strategy.md), accepted

## Technical Constraints

- The architecture requires conditional read-only Windows mapping; it does not require `memmap2`, `windows-sys`, the `windows` crate, or direct Win32 calls. Select the smallest maintainable implementation using current primary documentation during execution.
- Mapping is an optimization. All public observable byte, range, EOF, snapshot, stale, and generation behavior must match the buffered backend.
- Do not map a zero-byte file.
- Do not map unless local regular-file eligibility and stable sharing conditions are positively established. Unknown or failed checks must choose buffered access.
- The stable handle must reject existing write access at acquisition time and prevent subsequent write/delete sharing for the mapping lifetime.
- A retained `RangeView` must keep every mapping and handle resource required for safe byte access alive, even if the originating `FileSnapshot` value is dropped.
- Mapping size, view size, alignment, and slice conversions require checked arithmetic. Files that cannot be represented safely must fall back rather than panic.
- A test-only backend selector may force buffered operation or inject mapping failure, but no test hook may bypass production safety invariants.
- Linux and other non-Windows builds must not compile or expose the Windows unsafe implementation and must continue using positioned buffered I/O.
- Keep the workspace default `unsafe_code = "forbid"` for ordinary crates.
- `faultsift-file-access` remains deny-by-default. Permit unsafe Rust only inside the single Windows mapping implementation module; safe wrappers and validation must remain outside it.
- Every unsafe operation requires a local safety comment naming the handle lifetime, file-stability, mapping bounds, alignment, and aliasing invariant it relies on as applicable.
- Unsafe Rust in any other source file is a blocking failure for this task.

## Acceptance Criteria

- [ ] An eligible stable local Windows regular file can use the mapping backend through the existing `FileSnapshot` API.
- [ ] Linux and every ineligible or uncertain Windows file continue to use positioned buffered I/O.
- [ ] Empty files never enter the mapping path and retain their existing valid-empty semantics.
- [ ] A pre-existing writer prevents mapping eligibility and causes transparent buffered fallback when buffered open succeeds.
- [ ] Failure to create a stable handle or mapping falls back without exposing a backend-specific error when buffered access succeeds.
- [ ] Mapped and buffered backends return identical bytes and typed range behavior for the same snapshot fixture.
- [ ] `RangeView` keeps the mapped bytes and the stability handle alive for its full lifetime.
- [ ] Write, truncate, rename, and delete attempts against an active mapped snapshot are denied or otherwise prevented by the established Windows sharing contract.
- [ ] Mutation-safety child processes terminate normally; no supported external file operation causes an invalid mapped access or abnormal FaultSift process exit.
- [ ] Backend information is available for diagnostics/tests but domain correctness does not branch on it.
- [ ] The only repository-owned unsafe Rust is the reviewed Windows mapping module.
- [ ] Existing ordinary crates still compile with unsafe forbidden, and the file-access crate denies unsafe outside its single mapping module.
- [ ] No public API names or documentation make the selected mapping library part of the architecture contract.

## Test Cases

- Open a stable non-empty local Windows file and verify diagnostics select mapping and byte results match forced-buffered results.
- Open an empty file and verify no mapping attempt occurs.
- Hold a separate writable handle before opening the snapshot; verify mapping is not selected and buffered fallback remains functional when sharing permits it.
- While a mapped snapshot is alive, attempt write, truncate, rename, and delete operations from coordinated child processes; verify they cannot invalidate the mapping and all processes exit normally.
- Drop the originating snapshot while retaining a `RangeView`; verify the view remains valid until it is dropped and the stability resources are then released.
- Inject each eligible mapping-creation failure and verify transparent buffered fallback and a diagnostic reason.
- Exercise range boundaries, invalid UTF-8, exact EOF, and `max_view_bytes` through both mapped and buffered backends.
- Exercise a file larger than 4 GiB when the filesystem supports a bounded sparse fixture; verify checked offsets and fallback if mapping representation is unavailable.
- On Linux CI, verify the crate builds and tests without compiling or selecting the Windows mapping implementation.
- Audit mapping tests to ensure no force option can skip production safety checks.

## Verification

Run on Windows from the repository root:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p faultsift-file-access --test windows_mapping
cargo test -p faultsift-file-access --test mutation_process
cargo test -p faultsift-file-access
cargo test --workspace
rg -n '\bunsafe\s+(fn|trait|impl|extern)|\bunsafe\s*\{' crates --glob '*.rs'
```

The unsafe scan must identify only the single approved Windows mapping implementation file. Inspect every match and its safety comment.

Run on Ubuntu through the existing supported environment or CI:

```text
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Do not claim the Ubuntu or external CI result passed unless it actually ran.

## Expected Files

- `crates/faultsift-file-access/Cargo.toml` and `Cargo.lock` for one focused Windows implementation dependency if required;
- `crates/faultsift-file-access/src/platform/windows/` safe eligibility/handle orchestration;
- `crates/faultsift-file-access/src/platform/windows/mapping.rs` as the sole unsafe implementation boundary, or one equivalent documented file;
- safe backend-selection and diagnostics changes under `crates/faultsift-file-access/src/`;
- `crates/faultsift-file-access/tests/windows_mapping.rs`;
- focused additions to the existing mutation-process tests;
- scoped `AGENTS.md` clarification if the final single-file unsafe boundary needs an exact path.

## Risks

- Incorrect Windows share flags can appear to stabilize a file while still permitting an existing or future invalidating operation.
- A mapping or file handle dropped before the last `RangeView` creates a use-after-unmap safety defect.
- Mapping offsets and lengths can exceed OS, address-space, or Rust slice limits even on a 64-bit target.
- A permissive test-only force mode can accidentally become a production escape hatch around safety checks.
- Antivirus, filesystem filters, and non-local files can change eligibility or performance; uncertain cases must fall back safely.
- Moving `unsafe_code` policy to make this task compile can silently permit unsafe Rust in unrelated crates.
- Choosing a mapping library in architecture-facing APIs or docs would make a replaceable implementation detail durable.

## Open Questions

None. The concrete Windows mapping/binding library is an implementation selection explicitly delegated by ADR-0003; the implementation thread must document and verify its choice without changing the architecture contract.

## Review Focus

- proof of Windows handle-sharing and local-file eligibility preconditions;
- lifetime relationship among snapshot, `RangeView`, mapping, and stable file handle;
- transparent fallback for every failed or uncertain condition;
- byte and error parity between mapped and buffered backends;
- checked mapping bounds, offsets, alignment, and slice construction;
- complete audit of every unsafe operation and its documented invariant;
- lint configuration proving unsafe remains forbidden or denied everywhere else;
- absence of Linux mapping, backend leakage, and later roadmap capabilities.
