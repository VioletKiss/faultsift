# File Access Crate Instructions

- Keep this crate byte-only and independent of `faultsift-core`, Tauri, React, and desktop types.
- Keep allocations bounded by caller buffers or the configured `max_view_bytes`; never add a whole-file read API.
- Use checked `u64` file coordinates and explicit-offset I/O without a shared seek-position mutex.
- FS-002 code must remain safe Rust. Unsafe is permitted only inside audited Windows platform FFI modules under `src/platform/windows/`; unsafe anywhere else is blocking.
- Do not add line, parser, search, UI, AI, refresh, reopen, or file-validation behavior without the corresponding approved task.
