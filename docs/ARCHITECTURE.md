# FaultSift Architecture

## Responsibility of This Document

This document describes the target system structure, component boundaries, data flow, and architectural invariants currently established for FaultSift.

It does not define product priority, release numbers, or the scope of a particular change. Rationale for durable choices belongs in ADRs; executable work belongs in task specs.

## Current State

M0 established the Rust workspace, the Tauri 2 desktop shell under `apps/desktop/src-tauri`, the React + TypeScript frontend directly under `apps/desktop`, and the Tauri-independent `faultsift-core` crate. The shell contains no FaultSift business capability; the remaining components in this document describe approved target boundaries rather than implemented features.

## System Context

The target desktop structure is:

```text
apps/desktop/                 React + TypeScript presentation
      │
      │ Tauri 2 commands / bounded responses
      ▼
apps/desktop/src-tauri/       desktop adapter
      │
      ▼
crates/faultsift-core/        Rust domain foundation
      │
      ├── local log files
      └── optional structured AI context
                 ├── Ollama
                 └── user-configured OpenAI-compatible API
```

React source lives directly under `apps/desktop`; there is no separate `web/ui` package. Rust owns file access and analysis. React owns presentation and interaction. Tauri is an adapter between the desktop UI and domain APIs; core analysis must remain usable by a future CLI without depending on Tauri.

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
| FileReader | Read-only, bounded access to local file bytes |
| LineIndexer | Locate lines or ranges without retaining all raw lines |
| EventParser | Parse timestamps, levels, threads, loggers, messages, identifiers, and logical event boundaries |
| PatternMiner | Normalize dynamic values and form similar-event patterns |
| TimelineAggregator | Aggregate WARN, ERROR, exception, and later pattern activity into time buckets |
| AnomalyDetector | Later identify first-seen, rare, burst, and frequency-change signals |
| SearchEngine | Later provide text, regex, field, and time-range queries |
| AIContextBuilder | Select and structure incident evidence for optional AI analysis |

These are logical boundaries, not yet a decision that every component must be a separate Rust crate. Physical package boundaries require an approved design or ADR.

## Processing Data Flow

```text
local file bytes
      ↓
bounded file access and line boundaries
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

Read-only memory mapping is the documented planned direction for random byte access, but its lifecycle and file-mutation semantics must be designed and recorded before FS-002 is implemented.

## Event and Parsing Invariants

The initial parser focus is Java application logs. A stack trace containing frames, `Caused by`, `Suppressed`, or common-frame omission markers belongs to one logical event.

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
desktop UI → Tauri adapter → domain APIs → file / analysis primitives
optional CLI ────────────────────┘
AI adapter → structured analysis outputs
```

Domain and file-processing layers do not depend on desktop UI concepts. React must not become a second implementation of parsing, indexing, search, pattern, timeline, or anomaly logic.

## Architectural Open Questions

- Beyond the M0 `faultsift-core` foundation, what future Rust crate boundaries are justified?
- What read-only mapping abstraction and lifecycle rules apply when a mapped file grows, changes, or is truncated?
- Which line-index strategy balances lookup latency and bounded memory: full, sparse, lazy, or a hybrid?
- Are indexes transient or persisted, and if persisted, what invalidation and versioning rules apply?
- What precisely defines pattern identity, especially for exception type, message normalization, and stack context?
- What timestamp, timezone, malformed-input, and mixed-format rules form the Java parser contract?
- What are the Tauri range-query and cancellation/progress contracts?

Resolve durable, high-impact choices through design review and an ADR rather than silently embedding them in implementation.
