//! The wasm sandbox bounds hostile guests instead of hanging on them.
//!
//! Structural behavior test (a mod is untrusted, so its failure modes are the
//! contract): a benign export returns `Ok`; an infinite loop is stopped by fuel;
//! an oversized memory is rejected by the store limiter. The test *completing*
//! is itself the proof that a runaway guest does not hang — fuel guarantees
//! termination.

use stormlight_modloader::host::{Host, Limits};

/// A host with tight bounds so the loop fixture traps quickly.
fn test_host() -> Host {
    Host::with_limits(Limits { fuel: 5_000_000, max_memory_bytes: 4 * 1024 * 1024 }).unwrap()
}

fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("fixture wat compiles")
}

#[test]
fn a_benign_export_runs_to_completion() {
    let host = test_host();
    let module = wasm(r#"(module (func (export "run")))"#);
    assert!(host.run_unit_export(&module, "run").is_ok(), "benign guest should succeed");
}

#[test]
fn an_infinite_loop_is_bounded_by_fuel() {
    let host = test_host();
    let module = wasm(r#"(module (func (export "run") (loop br 0)))"#);
    // Returns (with an Err) rather than spinning forever.
    assert!(host.run_unit_export(&module, "run").is_err(), "runaway loop should trap on fuel");
}

#[test]
fn an_oversized_memory_is_rejected_by_the_limiter() {
    let host = test_host();
    // 100000 pages ≈ 6.25 GiB, far past the 4 MiB store cap.
    let module = wasm(r#"(module (memory 100000) (func (export "run")))"#);
    assert!(
        host.run_unit_export(&module, "run").is_err(),
        "memory beyond the cap should be refused"
    );
}
