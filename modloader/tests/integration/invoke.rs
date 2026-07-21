//! The runtime invocation path: the host **pushes** a context blob into a fresh
//! guest instance (via `mod_alloc`), calls a runtime entry point, and decodes the
//! `GuestEffects` it returns — all bounded by fuel + the memory cap.
//!
//! The core fixture is an *echo* guest: its entry point returns the very
//! `(ptr, len)` it was handed. So `call_*(bytes)` must decode back to exactly the
//! `GuestEffects` whose encoding we passed as `bytes` — proving, end to end, that
//! the host allocated guest memory, wrote the blob there, and read the guest's
//! result faithfully. Boundedness is proven separately: a looping entry point
//! returns `Err` (the test completing is the proof it did not hang).

use std::panic::AssertUnwindSafe;

use bolero::check;
use stormlight_mod_abi::common::ImpactTarget;
use stormlight_mod_abi::ids::{DamageTypeId, HandlerId};
use stormlight_mod_abi::impacts::{DamageFlags, HealFlags, Impact};
use stormlight_mod_abi::math::Value;
use stormlight_mod_abi::runtime::GuestEffects;
use stormlight_modloader::host::{Host, Limits};

/// A host with tight bounds so a runaway fixture traps quickly.
fn test_host() -> Host {
    Host::with_limits(Limits { fuel: 5_000_000, max_memory_bytes: 4 * 1024 * 1024 }).unwrap()
}

fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("fixture wat compiles")
}

/// An echo guest: `mod_alloc` hands out a fixed scratch offset; each entry point
/// returns `pack_ptr_len(ptr, len)` of the region it received. Two pages of memory
/// so a generated blob comfortably fits above the scratch base.
fn echo_guest() -> Vec<u8> {
    wasm(
        r#"(module
             (memory (export "memory") 2)
             (func (export "mod_alloc") (param i32) (result i32) i32.const 2048)
             (func (export "mod_handle") (param i32 i32 i32) (result i64)
               (i64.or (i64.shl (i64.extend_i32_u (local.get 1)) (i64.const 32))
                       (i64.extend_i32_u (local.get 2))))
             (func (export "mod_tick") (param i32 i32) (result i64)
               (i64.or (i64.shl (i64.extend_i32_u (local.get 0)) (i64.const 32))
                       (i64.extend_i32_u (local.get 1))))
             (func (export "mod_trigger") (param i32 i32 i32) (result i64)
               (i64.or (i64.shl (i64.extend_i32_u (local.get 1)) (i64.const 32))
                       (i64.extend_i32_u (local.get 2)))))"#,
    )
}

/// Build a small `GuestEffects` from an opcode stream — enough shape to exercise
/// the decode (leaves, the `Custom` blob), bounded so its encoding fits the page.
fn sample_effects(ops: &[(u16, u16)]) -> GuestEffects {
    let effects = ops
        .iter()
        .take(16)
        .map(|&(op, n)| {
            let target = ImpactTarget::ResolvedTarget;
            match op % 3 {
                0 => Impact::Damage {
                    amount: Value::Const(f32::from(n)),
                    dtype: DamageTypeId(n),
                    target,
                    flags: DamageFlags::default(),
                },
                1 => Impact::Heal {
                    amount: Value::Const(f32::from(n)),
                    target,
                    flags: HealFlags::default(),
                },
                _ => Impact::Custom {
                    handler: HandlerId(u32::from(n)),
                    params: vec![n as u8; (n % 4) as usize],
                    target,
                },
            }
        })
        .collect();
    GuestEffects { effects }
}

#[test]
fn a_pushed_blob_round_trips_through_the_guest_and_decodes_to_the_same_effects() {
    // The wasmtime `Engine` behind `Host` is not `RefUnwindSafe`; asserting it is
    // safe here is sound — we only read it, and an invocation trap surfaces as an
    // `Err`, not shared state broken across the unwind boundary.
    let host = test_host();
    let module = host.load_module(&echo_guest()).expect("echo guest loads");
    let bundle = AssertUnwindSafe((host, module));
    check!().with_type::<Vec<(u16, u16)>>().for_each(|ops| {
        let AssertUnwindSafe((host, module)) = &bundle;
        let effects = sample_effects(ops);
        let bytes = postcard::to_allocvec(&effects).unwrap();

        // Through the handler entry (3-arg) and the tick entry (2-arg): both must
        // faithfully push, invoke, and decode.
        let back = host.call_handler(module, 0, &bytes).expect("handler invoke");
        assert_eq!(back, effects, "handler push/invoke/decode must be identity");

        let back = host.call_tick(module, &bytes).expect("tick invoke");
        assert_eq!(back, effects, "tick push/invoke/decode must be identity");

        let back = host.call_trigger(module, 0, &bytes).expect("trigger invoke");
        assert_eq!(back, effects, "trigger push/invoke/decode must be identity");
    });
}

#[test]
fn an_empty_blob_yields_no_effects() {
    let host = test_host();
    // A guest that returns (0,0) — the "no effects" sentinel — for any handler.
    let guest = wasm(
        r#"(module
             (memory (export "memory") 1)
             (func (export "mod_alloc") (param i32) (result i32) i32.const 1024)
             (func (export "mod_handle") (param i32 i32 i32) (result i64) i64.const 0))"#,
    );
    let module = host.load_module(&guest).unwrap();
    assert!(host.call_handler(&module, 0, &[]).unwrap().effects.is_empty());
}

#[test]
fn a_runaway_handler_is_bounded_not_hung() {
    let host = test_host();
    let guest = wasm(
        r#"(module
             (memory (export "memory") 1)
             (func (export "mod_alloc") (param i32) (result i32) i32.const 1024)
             (func (export "mod_handle") (param i32 i32 i32) (result i64)
               (loop br 0) i64.const 0))"#,
    );
    let module = host.load_module(&guest).unwrap();
    // Returns (with an Err) rather than spinning forever: fuel guarantees this.
    assert!(host.call_handler(&module, 0, &[]).is_err(), "runaway handler must trap on fuel");
}

#[test]
fn a_result_pointing_out_of_bounds_is_an_error_not_a_panic() {
    let host = test_host();
    // Returns a (ptr,len) far past linear memory: the host must reject it.
    let guest = wasm(
        r#"(module
             (memory (export "memory") 1)
             (func (export "mod_alloc") (param i32) (result i32) i32.const 1024)
             (func (export "mod_handle") (param i32 i32 i32) (result i64)
               (i64.const 0x7fffffff_0000ffff)))"#,
    );
    let module = host.load_module(&guest).unwrap();
    assert!(host.call_handler(&module, 0, &[]).is_err(), "OOB result region must error");
}
