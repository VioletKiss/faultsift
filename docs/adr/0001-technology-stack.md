# ADR-0001: Technology Stack

- Status: Accepted
- Date: 2026-08-25
- Deciders: FaultSift product owner
- Related tasks: FS-001
- Supersedes: None
- Superseded by: None

## Context

FaultSift is a local-first desktop tool for investigating huge log files. Its core must support high-performance, bounded-memory file processing, while its desktop UI needs a productive component model and access to native capabilities. Core analysis must remain reusable by a future CLI and must not depend on desktop UI types.

M0 needs one reproducible stack so repository bootstrap, local checks, and CI do not embed an accidental framework choice.

## Decision Drivers

- Native local file access and predictable memory behavior
- Cross-platform desktop delivery without a cloud service
- A strict boundary between core analysis and desktop presentation
- Productive, testable desktop UI development
- Ecosystem maturity and maintainability for a long-lived open-source project
- Reuse of Rust domain APIs by a possible future CLI

## Considered Options

### Rust + Tauri 2 + React + TypeScript

Rust owns domain and file-processing code. Tauri 2 provides the desktop adapter. React + TypeScript provides presentation and interaction.

### Electron + TypeScript

This would simplify a single-language application but makes Rust-quality huge-file processing and memory control less direct and carries a heavier desktop runtime.

### Rust-native desktop UI

This would keep one systems language but offers a less suitable UI development ecosystem for the planned desktop interaction model.

## Decision

Use:

- Rust for core/domain and performance-sensitive processing;
- Tauri 2 for the desktop shell and adapter boundary;
- React + TypeScript for presentation and interaction;
- Vite as the frontend build tool;
- pnpm as the frontend package manager;
- Vitest and React Testing Library for the initial frontend test harness;
- ESLint and Prettier for frontend linting and formatting.

Core/domain crates must not depend on Tauri, React, or desktop UI concepts. Tauri commands adapt bounded domain APIs for the UI.

Supporting tool choices may be reconsidered through focused tasks when evidence justifies the cost. Replacing Rust, Tauri, or React requires a superseding ADR.

## Consequences

### Positive

- Rust can provide bounded-memory, byte-oriented processing and reusable domain APIs.
- Tauri keeps native integration at an explicit adapter boundary.
- React + TypeScript provides a mature component and testing ecosystem.
- The stack supports Windows and Linux development while leaving macOS expansion possible.

### Negative

- The repository must maintain both Rust and frontend toolchains.
- Tauri builds require platform-specific native dependencies.
- Cross-boundary API design and IPC payload size require deliberate review.

### Neutral or Follow-up

- FS-001 will pin mutually compatible stable versions and commit lockfiles.
- Desktop installer, signing, updater, and release tooling decisions are deferred.
- Huge-file algorithms remain separate later decisions.

## Validation

FS-001 must prove that Rust formatting/lint/tests, frontend lint/typecheck/tests/build, and the minimal Tauri compile path run locally or in the approved initial CI environments.

## References

- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/tasks/FS-001-repository-bootstrap.md`
