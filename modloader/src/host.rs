//! The wasmtime host: a deterministic, sandboxed engine that bounds runaway
//! guests.
//!
//! A mod is untrusted input. The [`Host`] builds a wasmtime [`Engine`] with **no
//! WASI and no ambient capabilities** — the only surface a guest sees is the
//! host functions we register (none yet in M3's static pipeline). Two bounds
//! keep a hostile or buggy guest from hanging or exhausting memory:
//!
//! - **Fuel** (deterministic, no background thread): every store starts with a
//!   fuel budget; an infinite loop traps when it runs out.
//! - **`StoreLimits`**: a hard cap on linear-memory growth; an oversized initial
//!   memory or a runaway `memory.grow` is rejected, not honored.
//!
//! Determinism knobs (NaN canonicalization, no threads) are set now so replays
//! and the eventual guest-side effect calls behave identically across hosts.

use anyhow::{Result, anyhow};
use stormlight_mod_abi::descriptors::Registration;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

use crate::registry::decode_registration;

/// Resource bounds applied to every guest invocation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Fuel budget per store; one unit is roughly one wasm operation.
    pub fuel: u64,
    /// Hard cap on a store's linear memory, in bytes.
    pub max_memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 200_000_000, max_memory_bytes: 64 * 1024 * 1024 }
    }
}

/// Per-store host state. Holds the memory limiter; host-function state (and the
/// registration sink) attach here in later slices.
pub struct StoreState {
    limits: StoreLimits,
}

/// A configured, reusable wasm host. The [`Engine`] is shared across mods; each
/// invocation gets a fresh bounded [`Store`].
pub struct Host {
    engine: Engine,
    limits: Limits,
}

impl Host {
    /// A host with default [`Limits`].
    pub fn new() -> Result<Self> {
        Self::with_limits(Limits::default())
    }

    /// A host with explicit bounds.
    pub fn with_limits(limits: Limits) -> Result<Self> {
        let mut config = Config::new();
        // Deterministic execution: bound loops with fuel and canonicalize NaNs.
        // Threads/atomics (a nondeterminism source) are already absent — the
        // `threads` wasmtime feature is off under `default-features = false`.
        config.consume_fuel(true);
        config.cranelift_nan_canonicalization(true);
        let engine = Engine::new(&config)?;
        Ok(Self { engine, limits })
    }

    /// The shared engine, for building modules ahead of instantiation.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The active resource bounds.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// A fresh store carrying the memory limiter and a full fuel budget.
    fn new_store(&self) -> Result<Store<StoreState>> {
        let limits = StoreLimitsBuilder::new().memory_size(self.limits.max_memory_bytes).build();
        let mut store = Store::new(&self.engine, StoreState { limits });
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.limits.fuel)?;
        Ok(store)
    }

    /// Compile, instantiate (no imports, no WASI), and call a no-argument export,
    /// bounded by fuel and the memory cap.
    ///
    /// Returns `Err` on any trap — crucially, a runaway guest (infinite loop or
    /// oversized allocation) is *bounded*: this returns an error promptly rather
    /// than hanging. The base primitive S7 builds on to run `mod_register`.
    pub fn run_unit_export(&self, wasm: &[u8], export: &str) -> Result<()> {
        let module = Module::new(&self.engine, wasm)?;
        let mut store = self.new_store()?;
        // An empty linker: the guest imports nothing. No WASI, no host calls yet.
        let linker: Linker<StoreState> = Linker::new(&self.engine);
        let instance = linker.instantiate(&mut store, &module)?;
        let func = instance.get_typed_func::<(), ()>(&mut store, export)?;
        func.call(&mut store, ())?;
        Ok(())
    }

    /// Instantiate `wasm`, run its `mod_register` export, and decode the
    /// [`Registration`] it emitted from the guest's linear memory.
    ///
    /// `mod_register` returns a packed `(ptr, len)` into the guest's exported
    /// `memory` (the pull/return bridge). Bounded by fuel + the memory cap; a
    /// hostile guest cannot hang here or make us read out of bounds.
    pub fn register(&self, wasm: &[u8]) -> Result<Registration> {
        let module = Module::new(&self.engine, wasm)?;
        let mut store = self.new_store()?;
        let linker: Linker<StoreState> = Linker::new(&self.engine);
        let instance = linker.instantiate(&mut store, &module)?;

        // `mod_register` returns `u64`; at the wasm ABI that is an `i64`.
        let func = instance.get_typed_func::<(), i64>(&mut store, "mod_register")?;
        let packed = func.call(&mut store, ())? as u64;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("guest exports no `memory`"))?;
        decode_registration(memory.data(&store), packed)
    }
}
