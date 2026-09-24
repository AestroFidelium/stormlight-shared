//! What the mod host costs (stormlight/server#196).
//!
//! Every runtime call gets a fresh store: instantiate, push the context, run the
//! guest, decode its effects, drop. That keeps guests stateless and isolated;
//! these benches put a number on what it costs.
//!
//! - `baseline_*`: a hand-written guest whose entries return at once. This is
//!   the host's own overhead: instantiate, push, call, decode, drop.
//! - `small` / `large`: a real mod built with the SDK, with 2 or 64 abilities.
//!   Each runtime entry re-runs the mod's builder, so the gap between the two is
//!   what a bigger mod pays on every call.
//!
//! The guest is built on first use (`cargo build --target wasm32-unknown-unknown`
//! in `benches/guest`), so the wasm32 target must be installed.
//!
//! Run with `cargo bench -p stormlight_modloader`.

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use divan::Bencher;
use stormlight_mod_abi::ids::EventId;
use stormlight_mod_abi::math::Value;
use stormlight_mod_abi::runtime::{TickContext, TriggerContext};
use stormlight_modloader::host::{GuestModule, Host};

fn main() {
    // Build both guests before divan starts, so cargo's output does not land in
    // the middle of the results table.
    LazyLock::force(&SMALL);
    LazyLock::force(&LARGE);
    divan::main();
}

/// Build the SDK guest with or without the `large` feature and return its wasm.
fn build_guest(large: bool) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/guest");
    let target = dir.join(if large { "target/large" } else { "target/small" });
    let mut cargo = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cargo
        .current_dir(&dir)
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .arg("--target-dir")
        .arg(&target);
    if large {
        cargo.args(["--features", "large"]);
    }
    let status = cargo.status().expect("running cargo for the bench guest");
    assert!(status.success(), "building the bench guest failed");
    std::fs::read(target.join("wasm32-unknown-unknown/release/stormlight_bench_guest.wasm"))
        .expect("reading the built bench guest")
}

static SMALL: LazyLock<Vec<u8>> = LazyLock::new(|| build_guest(false));
static LARGE: LazyLock<Vec<u8>> = LazyLock::new(|| build_guest(true));

/// Every entry returns at once: `mod_alloc` hands back a fixed offset and the
/// calls return the "no effects" sentinel. What is left is the host's overhead.
static BASELINE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "mod_alloc") (param i32) (result i32) (i32.const 1024))
            (func (export "mod_tick") (param i32 i32) (result i64) (i64.const 0))
            (func (export "mod_trigger") (param i32 i32 i32) (result i64) (i64.const 0))
            (func (export "mod_handle") (param i32 i32 i32) (result i64) (i64.const 0)))"#,
    )
    .expect("baseline guest compiles")
});

fn guest(size: &str) -> &'static [u8] {
    match size {
        "small" => &SMALL,
        "large" => &LARGE,
        _ => &BASELINE,
    }
}

const SIZES: &[&str] = &["baseline", "small", "large"];
const MOD_SIZES: &[&str] = &["small", "large"];

fn tick_ctx() -> Vec<u8> {
    postcard::to_allocvec(&TickContext { tick: 1234 }).unwrap()
}

fn trigger_ctx() -> Vec<u8> {
    postcard::to_allocvec(&TriggerContext { event: EventId(0), payload: Value::Const(1.0) })
        .unwrap()
}

/// Compiling a guest: paid once per mod per session.
#[divan::bench(args = MOD_SIZES, sample_count = 20)]
fn compile(bencher: Bencher, size: &str) {
    let host = Host::new().unwrap();
    let wasm = guest(size);
    bencher.bench_local(|| host.load_module(wasm).unwrap());
}

/// Loading a mod: compile, instantiate, run `mod_register`, decode. Paid once.
#[divan::bench(args = MOD_SIZES, sample_count = 20)]
fn register(bencher: Bencher, size: &str) {
    let host = Host::new().unwrap();
    let wasm = guest(size);
    bencher.bench_local(|| host.register(wasm).unwrap());
}

fn compiled(size: &str) -> (Host, GuestModule) {
    let host = Host::new().unwrap();
    let module = host.load_module(guest(size)).unwrap();
    (host, module)
}

/// One `mod_tick` on a precompiled guest: what a mod costs every tick.
#[divan::bench(args = SIZES)]
fn tick(bencher: Bencher, size: &str) {
    let (host, module) = compiled(size);
    let ctx = tick_ctx();
    bencher.bench_local(|| host.call_tick(&module, &ctx).unwrap());
}

/// One `mod_trigger` on a precompiled guest.
#[divan::bench(args = SIZES)]
fn trigger(bencher: Bencher, size: &str) {
    let (host, module) = compiled(size);
    let ctx = trigger_ctx();
    bencher.bench_local(|| host.call_trigger(&module, 0, &ctx).unwrap());
}

/// One `Custom` handler call on a precompiled guest.
#[divan::bench(args = SIZES)]
fn handler(bencher: Bencher, size: &str) {
    let (host, module) = compiled(size);
    let params = [7_u8; 16];
    bencher.bench_local(|| host.call_handler(&module, 0, &params).unwrap());
}
