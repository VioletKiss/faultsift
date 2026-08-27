# Architecture Decision Records

Architecture Decision Records (ADRs) preserve important technical decisions, their context, alternatives, and consequences. `docs/ARCHITECTURE.md` says what the current architecture is; ADRs explain why durable choices were made and what would be affected by changing them.

## When to Write an ADR

Create an ADR for a decision that is costly to reverse, affects several tasks or modules, constrains future designs, or protects a non-obvious trade-off. Examples include:

- primary technology stack;
- file-access and memory-mapping strategy;
- line-index structure;
- persistent index format;
- module and dependency boundaries;
- plugin API;
- concurrency or cancellation model;
- pattern identity rules;
- AI data and privacy boundaries.

Do not create an ADR for routine implementation details, local refactors, formatting, or choices fully contained within one task and easy to reverse.

## Lifecycle

1. Copy [TEMPLATE.md](TEMPLATE.md) to `NNNN-short-kebab-title.md`.
2. Use the next unused four-digit number; numbers are never reused.
3. Start with `Proposed` while the decision is under discussion.
4. The product owner or designated decision maker changes it to `Accepted` after agreement.
5. Update `docs/ARCHITECTURE.md` when an accepted ADR changes current architecture.
6. If a later decision replaces it, keep the old record and mark it `Superseded by ADR-NNNN`.

Supported status values are `Proposed`, `Accepted`, `Rejected`, `Deprecated`, and `Superseded by ADR-NNNN`.

Do not edit an accepted ADR to make history appear different. Use a new ADR for a materially new decision. Small factual corrections should be visibly noted.

## Review Rule

If a task or implementation conflicts with an accepted ADR, stop and report the conflict. Codex may compare options and recommend a path, but must not decide to overturn the ADR on its own.

Accepted architecture decisions:

- [ADR-0001: Technology Stack](0001-technology-stack.md)
- [ADR-0002: Repository Layout](0002-repository-layout.md)
- [ADR-0003: Large File Byte Access Strategy](0003-large-file-byte-access-strategy.md)
- [ADR-0004: Physical Line Access and Adaptive Sparse Index](0004-physical-line-access-and-adaptive-sparse-index.md)
