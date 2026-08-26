//! Tauri-independent FaultSift core foundation.
//!
//! FaultSift business capabilities are intentionally absent during M0.

#[cfg(test)]
mod tests {
    #[test]
    fn core_test_harness_is_available() {
        assert_eq!(env!("CARGO_PKG_NAME"), "faultsift-core");
    }
}
