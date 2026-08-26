# FS-001: Repository Bootstrap

- Status: Completed
- Owner: Unassigned
- Related ADRs: ADR-0001, ADR-0002
- Roadmap stage: M0 — Repository Foundation

## Goal

Create a minimal, buildable, testable FaultSift repository shell for the approved Rust + Tauri 2 + React + TypeScript architecture, with reproducible local quality commands and initial CI, without implementing any FaultSift business capability.

## Context

The repository currently contains the Agent Engineering foundation only. Product, architecture, roadmap, accepted ADRs, task contracts, and agent rules already exist and must be preserved as the authoritative project memory.

This task completes M0 by creating the executable project baseline required by later tasks. It does not deliver any part of the log-analysis MVP and does not create a product release.

## In Scope

- a minimal Rust workspace;
- a minimal `crates/faultsift-core` crate proving domain code remains independent of Tauri;
- a Tauri 2 desktop shell under `apps/desktop/src-tauri`;
- a React + TypeScript + Vite frontend directly under `apps/desktop`;
- product name `FaultSift`, bundle identifier `cn.violetkiss.faultsift`, and manifest version `0.0.0`;
- a pnpm workspace with Vitest, React Testing Library, ESLint, and Prettier;
- package scripts for linting, formatting, type checking, tests, and builds;
- minimal smoke tests proving the Rust and frontend test harnesses run;
- Rust formatting and lint configuration required by the verification commands;
- a root `LICENSE` containing the unmodified Apache License 2.0 text and `Apache-2.0` package metadata;
- `crates/AGENTS.md` and `apps/desktop/AGENTS.md` with the directory-specific rules described by the governance foundation;
- GitHub Actions checks for the initial Windows and Ubuntu policy;
- focused `README.md` updates replacing the pre-bootstrap status with commands that were actually verified.

## Out of Scope

- opening, mapping, reading, scanning, indexing, or searching log files;
- Java log or stack-trace parsing;
- pattern normalization, mining, fingerprints, or aggregation;
- timeline or anomaly analysis;
- AI, Ollama, or external API integration;
- production UI, virtualized log views, dashboard widgets, or product workflows;
- a separate `web/ui` package;
- generated performance fixtures or benchmark thresholds;
- CLI or MCP implementation;
- creating future logical components as empty crates;
- Storybook, Playwright, Biome, or an additional UI framework;
- installers, signing, publishing, automatic updates, or release tags;
- macOS build or CI support;
- a `NOTICE` file or per-file license headers while the repository has no attribution content requiring them;
- branded visual asset design.

## Dependencies

- Root `AGENTS.md`
- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/ROADMAP.md`
- [ADR-0001 Technology Stack](../adr/0001-technology-stack.md)
- [ADR-0002 Repository Layout](../adr/0002-repository-layout.md)

No prior implementation task is required.

## Technical Constraints

- Use stable, mutually compatible Rust, Node LTS, pnpm, Tauri 2, React, TypeScript, and Vite versions verified against current official documentation during execution.
- Record tool versions using the ecosystems' normal toolchain and package-manager fields, and commit generated lockfiles.
- Core/domain code must not depend on Tauri, React, or desktop UI types.
- The desktop shell must contain no FaultSift business logic.
- React source lives under `apps/desktop`; do not create `web/ui`.
- Use `FaultSift` as product/window name, `cn.violetkiss.faultsift` as bundle identifier, and `0.0.0` in initial manifests.
- Use pnpm, Vite, Vitest, React Testing Library, ESLint, and Prettier; do not substitute another tool without an approved task change.
- Use the exact Apache-2.0 license text without custom clauses.
- Windows is primary; Ubuntu is a secondary build/test target; macOS is deferred.
- CI uses explicit supported Windows and Ubuntu runner labels selected during implementation.
- Rust tests and the Tauri compile check run on Windows and Ubuntu.
- Frontend lint, typecheck, test, and build run once on Ubuntu.
- Ubuntu CI installs the system dependencies required by the selected Tauri 2 version.
- Keep early CI bounded; do not build release installers.
- Local commands and CI must agree; do not claim CI success from local execution alone.
- Preserve existing documentation and skills; do not regenerate them from framework templates.

## Acceptance Criteria

- [x] The Rust workspace is valid and contains `faultsift-core` with no Tauri dependency.
- [x] The desktop package contains a minimal Tauri 2 + React + TypeScript + Vite shell and no product feature implementation.
- [x] Product/window name, bundle identifier, and manifest version match the approved values.
- [x] Rust formatting, linting, and tests pass from documented repository commands.
- [x] Frontend linting, formatting check, type checking, tests, and production build pass from documented repository commands.
- [x] The Tauri Rust shell passes an appropriate compile check without requiring a release installer build.
- [x] A bounded manual smoke check launches and closes the empty desktop shell on Windows without external services.
- [x] `crates/AGENTS.md` and `apps/desktop/AGENTS.md` enforce the relevant core and UI constraints without duplicating the root file wholesale.
- [x] CI runs Rust tests and the Tauri compile check on Windows and Ubuntu.
- [x] CI runs frontend checks once on Ubuntu and installs required Linux system packages.
- [x] The root license and package metadata consistently use Apache-2.0; no empty NOTICE is created.
- [x] README instructions match the commands that were actually verified.
- [x] No file-access, parsing, pattern, timeline, anomaly, search, or AI behavior is introduced.
- [x] No installer, release artifact, product tag, or macOS job is introduced.

## Test Cases

- Rust smoke test executes successfully in the workspace on Windows and Ubuntu CI.
- Frontend render smoke test executes successfully with Vitest and React Testing Library.
- Frontend type checking and production asset build succeed.
- Tauri Rust configuration and code compile on Windows and Ubuntu CI.
- Empty desktop shell launches on Windows without a log file, network service, AI service, or credentials.
- Manifest metadata reports `FaultSift`, `cn.violetkiss.faultsift`, `0.0.0`, and Apache-2.0 consistently.

## Verification

Exact root script routing may be introduced by this task, but the resulting repository must support and document commands equivalent to:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm build
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Manual Windows smoke check:

```text
pnpm tauri dev
```

Launch the empty window, verify that it does not require external services, then close it. Record any platform prerequisite that prevents this check. The task is not complete until all locally applicable commands pass; external CI status must be reported separately.

## Completion Evidence

Completed on 2026-08-26.

- Rust 1.98.0, Node.js 24.19.0, and pnpm 11.19.0 were used with committed Cargo and pnpm lockfiles.
- Every automated command in the Verification section passed locally on Windows; clippy also passed with warnings treated as errors.
- The frontend render smoke test passed: 1 test, 0 failures.
- The Rust workspace smoke test passed: 1 test, 0 failures; the empty desktop targets contain no tests or business behavior.
- The Windows shell launched with a responsive `FaultSift` window, established no application-process network connection, and closed without leaving its process or development port running.
- The Apache-2.0 text was compared with the official unmodified license text, and package metadata is consistent.
- `.github/workflows/ci.yml` defines pinned-action jobs on `windows-2022` and `ubuntu-22.04`; external GitHub Actions had not run at completion and is not claimed as passed.
- A scope audit found no implementation of the business capabilities excluded by this task.

## Expected Files

- root Rust, Node/pnpm, formatter, lint, ignore, and toolchain/version configuration;
- root `LICENSE`;
- `crates/AGENTS.md`;
- `crates/faultsift-core/` minimal crate and smoke tests;
- `apps/desktop/AGENTS.md`;
- `apps/desktop/` React + TypeScript + Vite source and tests;
- `apps/desktop/src-tauri/` minimal Tauri 2 shell;
- `.github/workflows/ci.yml` or an equivalently named CI workflow;
- lockfiles required for reproducibility;
- focused bootstrap updates to `README.md`.

Framework generators may produce additional required files. Review all generated output and remove unrelated example features, permissive capabilities, telemetry, and unused assets. A neutral temporary icon is allowed only when the desktop build requires one.

## Risks

- Tauri native prerequisites vary by operating system; local and CI failures must distinguish missing prerequisites from code failures.
- Framework templates can introduce unnecessary example logic, capabilities, or assets that look like product work.
- Dependency versions and generated configuration can drift if toolchain fields and lockfiles are not coherent.
- A green frontend build does not prove the desktop shell compiles, and a Rust workspace check does not prove the frontend does; both paths require evidence.
- Ubuntu compile/test coverage does not imply a commitment to publish Linux installers in `v0.1.0`.

## Open Questions

None. Exact compatible dependency, runner, and Action versions are implementation selections that must be documented and locked during execution.

## Review Focus

- absence of business logic and premature future-module scaffolding;
- conformance to ADR-0001 and ADR-0002;
- core/Tauri dependency direction;
- exact application identity and `0.0.0` versioning;
- reproducibility of tool versions and lockfiles;
- Windows/Ubuntu CI responsibility split;
- parity between local commands and CI;
- Apache-2.0 consistency without an empty NOTICE;
- preservation of the existing governance foundation.
