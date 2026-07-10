//! Feeding arbitrary bytes to the host must never panic and never hang: an
//! invalid module fails to compile, a valid-but-hostile one is bounded by fuel
//! and the memory cap. Either way `run_unit_export` returns.

use std::panic::AssertUnwindSafe;

use bolero::check;
use stormlight_modloader::host::{Host, Limits};

#[test]
fn arbitrary_bytes_never_panic_the_host() {
    // One engine, reused across inputs (module compilation is per-call). The
    // host holds a wasmtime `Engine` that is not `RefUnwindSafe`; asserting it
    // is fine here — we only read it, and a trap is an `Err`, not shared state.
    let host = AssertUnwindSafe(
        Host::with_limits(Limits { fuel: 1_000_000, max_memory_bytes: 4 * 1024 * 1024 })
            .expect("host builds"),
    );
    check!().with_type::<Vec<u8>>().for_each(|bytes| {
        // Destructure the whole binding so the closure captures the
        // `AssertUnwindSafe` wrapper, not the inner `Host` field.
        let AssertUnwindSafe(host) = &host;
        let _ = host.run_unit_export(bytes, "run");
    });
}
