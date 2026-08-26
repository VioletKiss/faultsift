# ADR-0002: Repository Layout

- Status: Accepted
- Date: 2026-08-25
- Deciders: FaultSift product owner
- Related tasks: FS-001
- Supersedes: None
- Superseded by: None

## Context

The historical product source suggested both `apps/desktop` and `web/ui`, while the historical workflow placed React directly in the desktop application. Maintaining two frontend packages during M0 would introduce package boundaries, build routing, and synchronization costs without an approved standalone web product.

The Rust core also needs a physical boundary that prevents desktop dependencies from leaking into reusable domain code, without pre-creating every possible future module as an empty crate.

## Decision Drivers

- One obvious location for the only approved UI
- Minimal bootstrap surface and CI complexity
- Enforceable separation between domain code and the Tauri adapter
- No speculative packages or empty future modules
- Room to split Rust crates later when real contracts justify it

## Considered Options

### React directly under `apps/desktop`

Keep the React source, tests, and frontend configuration with the Tauri desktop application. Put the Tauri adapter under `apps/desktop/src-tauri` and the initial domain crate under `crates/faultsift-core`.

### Separate `web/ui` package

Create a reusable frontend package consumed by the desktop application. This adds a package boundary before a second consumer or standalone web product exists.

### Single mixed desktop package

Keep Rust domain code inside `apps/desktop/src-tauri`. This is minimal initially but violates the requirement that core analysis remain independent of Tauri and reusable by a future CLI.

## Decision

Use this M0 structure:

```text
faultsift/
├── apps/
│   └── desktop/
│       ├── src/              React + TypeScript
│       └── src-tauri/        Tauri 2 adapter
└── crates/
    └── faultsift-core/       Tauri-independent domain foundation
```

Do not create `web/ui`. Do not create empty crates for future logical components.

The desktop application identity is:

- product/window name: `FaultSift`;
- bundle identifier: `cn.violetkiss.faultsift`;
- M0 manifest version: `0.0.0`.

A later standalone web product or proven shared frontend contract requires an explicit architecture decision before adding a separate web package.

## Consequences

### Positive

- The only UI has one clear owner and build root.
- Frontend and Tauri shell changes can be verified together.
- Domain dependency direction is visible in the filesystem.
- M0 avoids speculative packages and crate proliferation.

### Negative

- A future second frontend consumer may require extracting shared UI code.
- Some root scripts must coordinate Cargo and pnpm workspaces.

### Neutral or Follow-up

- `crates/AGENTS.md` and `apps/desktop/AGENTS.md` will add directory-specific constraints in FS-001.
- Future Rust crate splits require demonstrated module contracts; they do not require changing the desktop layout.

## Validation

FS-001 must verify that `faultsift-core` has no Tauri dependency, the desktop shell builds from `apps/desktop`, no `web/ui` exists, and manifest identity matches the accepted values.

## References

- `docs/ARCHITECTURE.md`
- `docs/tasks/FS-001-repository-bootstrap.md`
