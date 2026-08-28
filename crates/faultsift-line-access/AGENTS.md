# Line Access Crate Instructions

- Keep this crate limited to byte-oriented physical-line access; parser, search, logical-event, index persistence, adapter, and UI semantics belong elsewhere.
- Preserve bounded-memory streaming. Scanner-owned memory must remain `O(scan_chunk_bytes)`, independent of file size, line count, and line length.
- Keep all crate code safe Rust. The platform FFI exceptions in `faultsift-file-access` do not apply here.
- Depend only on `std` and `faultsift-file-access` unless a later approved task demonstrates that another dependency is necessary.
- Keep the shared LF/CRLF scanner and pending-CR state machine as the single source of physical-newline truth for cursors and future in-crate index building.
- Preserve snapshot generation, captured-boundary, and stale-lifecycle behavior from `faultsift-file-access`; do not validate, reopen, or refresh implicitly.
