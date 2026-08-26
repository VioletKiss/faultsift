# FaultSift

> **Sift through logs. Find the fault.**
> **Find the incident, not the keyword.**

FaultSift is an open-source, local-first log incident forensics tool for huge log files. It is intended to reduce millions of noisy log lines to a small set of suspicious patterns, time ranges, and incident clues without uploading raw logs to the cloud.

中文简介：FaultSift 是一个面向开发者的本地优先日志故障取证工具，聚焦 GB 级日志的异常聚类、时间线定位和原始上下文还原。

## Project Status

**M0 — Repository Foundation is complete.** The repository now contains the Rust workspace, a Tauri 2 desktop shell, a React + TypeScript frontend, local quality commands, and Windows/Ubuntu CI configuration established by the FS-001 repository bootstrap.

- Bootstrap manifests use `0.0.0`; M0 is not a product release.
- No FaultSift file access, parsing, pattern, timeline, search, anomaly, or AI capability has been implemented.
- Windows is primary, Ubuntu is the secondary build/test target, and macOS remains deferred.
- The project is licensed under Apache-2.0.

## Development Baseline

Pinned versions are recorded in `rust-toolchain.toml`, `.node-version`, and the root `packageManager` field. Install Node.js 24.19.0 and pnpm 11.19.0, then install JavaScript dependencies with:

```text
pnpm install --frozen-lockfile
```

On Windows, run Rust and Tauri commands from an x64 Visual Studio Developer shell with the C++ Build Tools and Windows SDK installed. WebView2 is also required. On Ubuntu 22.04, install the native Tauri prerequisites listed in `.github/workflows/ci.yml`.

Repository verification commands:

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

Launch the empty Windows desktop shell for a bounded manual smoke check with:

```text
pnpm tauri dev
```

The root commands route frontend work to `apps/desktop`. Rust domain code belongs under `crates/` and must remain independent of Tauri and React.

## Product Direction

The first product MVP is centered on five capabilities:

1. open huge local log files without loading the whole file;
2. recognize Java multiline exceptions as logical events;
3. aggregate similar errors into patterns;
4. show WARN and ERROR timelines;
5. navigate from a pattern to its original log context.

AI is optional and comes after deterministic parsing, pattern, and timeline capabilities. It receives structured incident context rather than an entire log file.

## Repository Knowledge

| File | Responsibility |
|---|---|
| [Product](docs/PRODUCT.md) | Users, problems, value, scope, principles, and non-goals |
| [Architecture](docs/ARCHITECTURE.md) | Target system boundaries, data flow, and technical invariants |
| [Roadmap](docs/ROADMAP.md) | Engineering milestones, product releases, sequencing, and current status |
| [ADR guide](docs/adr/README.md) | Durable architectural decision process |

The original product design input remains under `docs/` as a frozen, non-authoritative historical reference:

- `docs/FaultSift.md`

## Engineering Workflow

The intended flow is:

```text
discuss and challenge a design
        ↓
record approved architecture / ADR
        ↓
create a focused FS-XXX task contract
        ↓
implement only that task
        ↓
run local and CI verification
        ↓
perform an independent review
```

Repository documents are long-term memory. Task specs are contracts. Automated checks provide evidence. Product and architectural trade-offs remain user decisions.

Engineering work is tracked as `M0`, `M1`, `M2`, and so on. User-visible releases use SemVer tags such as `v0.1.0`. The first `v0.1.0` will be the first actually usable FaultSift MVP, not the repository bootstrap.
