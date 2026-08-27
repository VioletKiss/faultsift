# FaultSift Roadmap

## Responsibility of This Document

This document tracks engineering milestones, product releases, current status, and capability sequencing. It does not redefine product scope or architectural constraints.

## Current Status

| Area | Status |
|---|---|
| Product and architecture foundation | Current |
| Bootstrap decisions | Approved and recorded |
| ADR and task process | Current |
| FaultSift planner/task/review skills | Current |
| M0 / FS-001 Repository Bootstrap | Completed |
| Rust, Tauri, React, and CI foundation | Current |
| M1 / FS-002–FS-005 File Access | Completed |
| M2 / FS-006–FS-009 Line Access / Index | Planned; implementation not started |
| Later Parser, UI, and AI work | Not started |

## Two Independent Numbering Systems

FaultSift deliberately separates internal engineering progress from user-visible releases.

### Engineering Milestones

Engineering milestones use `M0`, `M1`, `M2`, and so on. A milestone groups dependent engineering tasks and does not create a product release or Git tag by itself.

| Milestone | Intended outcome | Representative tasks |
|---|---|---|
| M0 — Repository Foundation | Governance, workspace shell, toolchain, local verification, and initial CI | FS-001 |
| M1 — File Access | Read-only huge-file byte access, stable snapshots, platform backends, and a reproducible benchmark baseline | FS-002–FS-005 |
| M2 — Line Access / Index | Bounded physical-line streaming, adaptive sparse indexing, exact ready lookup, and a reproducible benchmark baseline | FS-006–FS-009 |
| M3 — Java Parsing | Java headers and multiline exception events | FS-010–FS-011 |
| M4 — Patterns | Normalization, fingerprints, counts, first/last seen, and samples | FS-020–FS-022 |
| M5 — Timeline | WARN, ERROR, and pattern time buckets | FS-030 |
| M6 — Desktop MVP | File open, timeline, pattern list, and original context in the desktop workflow | FS-040 and focused follow-ups |

M1 completed on 2026-08-27 after FS-002 through FS-005 passed local verification, independent review, and required Windows/Ubuntu CI. The M2 Line Access / Index architecture is accepted and its FS-006 through FS-009 implementation tasks are proposed; implementation has not started. Parser, UI, and AI work remains not started.

Later search, anomaly, AI, correlation, and format-expansion work remains in the documented product direction but receives milestone numbers only when its design and dependencies are approved.

### Product Releases

Product releases use SemVer tags such as `v0.1.0` and `v0.2.0`.

- M0 manifests use `0.0.0` and M0 creates no product tag.
- A milestone completing does not automatically imply a product release.
- `v0.1.0` is reserved for the first actually usable FaultSift MVP containing the five capabilities defined in `PRODUCT.md`.
- Later release scope is assigned from completed, integrated capabilities rather than by reusing milestone numbers.

## M0 Completion Gate

M0 includes the governance foundation and the internal FS-001 repository bootstrap task.

M0 is complete only when FS-001 acceptance criteria pass, including the initial local verification and CI baseline. M0 must not implement file mapping, line indexing, log parsing, pattern mining, timeline aggregation, search, anomaly analysis, or AI.

M0 completed on 2026-08-26. The external GitHub Actions workflow was created but had not yet been run when local FS-001 verification completed.

Initial platform policy for M0:

- Windows is the primary development and runtime target.
- Ubuntu is the secondary build/test target.
- Rust tests and the Tauri compile check run on both Windows and Ubuntu.
- Frontend lint, typecheck, test, and build run once on Ubuntu.
- macOS, installers, signing, publishing, and release artifacts are deferred.

## Historical Numbering Resolution

The original source documents used overlapping `V0.x` labels for product releases and internal engineering phases. That ambiguity is resolved by the two independent systems above.

`FaultSift.md` and the private historical workflow document retain their original numbering only as historical references. Their version labels are not authoritative for current planning.

## Roadmap Open Questions

- What measurable performance gates should become milestone or release gates after reproducible baselines exist?
- How should post-MVP product directions be grouped into engineering milestones after their designs are approved?
