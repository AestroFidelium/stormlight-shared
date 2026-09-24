//! The wasmtime host: a deterministic, sandboxed engine that bounds runaway
//! guests.
//!
//! A mod is untrusted input. The [`Host`] builds a wasmtime [`Engine`] with **no
//! WASI and no ambient capabilities** — the guest imports nothing at all. Data
//! crosses only through the guest's own exports: the host pushes bytes in via
//! `mod_alloc` and reads the result back out of linear memory. Two bounds
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
use stormlight_mod_abi::bridge::unpack_ptr_len;
use stormlight_mod_abi::descriptors::Registration;
use stormlight_mod_abi::runtime::{
    ALLOC_EXPORT, GuestEffects, HANDLE_EXPORT, TICK_EXPORT, TRIGGER_EXPORT,
};
use stormlight_mod_abi::visuals::ClientRegistration;
use wasmtime::{Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

use crate::registry::{decode_client_registration, decode_registration};

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

    /// Instantiate `wasm`, run its `mod_register` export, and decode the
    /// [`ClientRegistration`] a cosmetic (`client.wasm`) guest emitted. Same
    /// bridge as [`Self::register`], decoding the client-side payload instead of
    /// the server gameplay one; equally bounded against a hostile guest.
    pub fn register_client(&self, wasm: &[u8]) -> Result<ClientRegistration> {
        let module = Module::new(&self.engine, wasm)?;
        let mut store = self.new_store()?;
        let linker: Linker<StoreState> = Linker::new(&self.engine);
        let instance = linker.instantiate(&mut store, &module)?;

        let func = instance.get_typed_func::<(), i64>(&mut store, "mod_register")?;
        let packed = func.call(&mut store, ())? as u64;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("guest exports no `memory`"))?;
        decode_client_registration(memory.data(&store), packed)
    }

    /// Precompile `wasm` into a reusable [`GuestModule`]. Compilation is the
    /// expensive step; a [`GuestModule`] is instantiated cheaply into a fresh
    /// bounded store on every invocation, so the caller keeps this and reuses it
    /// for the whole session (the server retains one per adopted mod).
    pub fn load_module(&self, wasm: &[u8]) -> Result<GuestModule> {
        Ok(GuestModule { module: Module::new(&self.engine, wasm)? })
    }

    /// Invoke the `Custom`-handler entry (`mod_handle`) with local id `handler`,
    /// pushing `params` and decoding the returned effects. See [`Self::invoke`].
    pub fn call_handler(
        &self,
        module: &GuestModule,
        handler: u32,
        params: &[u8],
    ) -> Result<GuestEffects> {
        self.invoke(module, Entry::Handle(handler), params)
    }

    /// Invoke the per-tick entry (`mod_tick`) over a serialized `TickContext`.
    pub fn call_tick(&self, module: &GuestModule, ctx: &[u8]) -> Result<GuestEffects> {
        self.invoke(module, Entry::Tick, ctx)
    }

    /// Invoke the trigger entry (`mod_trigger`) for `event` over a serialized
    /// `TriggerContext`.
    pub fn call_trigger(
        &self,
        module: &GuestModule,
        event: u32,
        ctx: &[u8],
    ) -> Result<GuestEffects> {
        self.invoke(module, Entry::Trigger(event), ctx)
    }

    /// The shared runtime-invocation pipeline: instantiate a fresh bounded store,
    /// push `input` into it through the guest's `mod_alloc`, call `entry`, and
    /// decode the [`GuestEffects`] it returns.
    ///
    /// Every step is fallible and bounded — a runaway guest traps on fuel and a
    /// result region outside linear memory is rejected, so a hostile mod returns
    /// `Err` and never hangs the tick or reads out of bounds.
    fn invoke(&self, module: &GuestModule, entry: Entry, input: &[u8]) -> Result<GuestEffects> {
        let mut store = self.new_store()?;
        let linker: Linker<StoreState> = Linker::new(&self.engine);
        let instance = linker.instantiate(&mut store, &module.module)?;

        // Push the context blob: ask the guest for a region, then write into it.
        let (ptr, len) = self.push(&mut store, &instance, input)?;

        // Call the entry point; its signature depends on whether it takes a
        // selector (handler id / event id) alongside `(ptr, len)`.
        let packed = match entry {
            Entry::Handle(id) => instance
                .get_typed_func::<(u32, u32, u32), u64>(&mut store, HANDLE_EXPORT)?
                .call(&mut store, (id, ptr, len))?,
            Entry::Trigger(event) => instance
                .get_typed_func::<(u32, u32, u32), u64>(&mut store, TRIGGER_EXPORT)?
                .call(&mut store, (event, ptr, len))?,
            Entry::Tick => instance
                .get_typed_func::<(u32, u32), u64>(&mut store, TICK_EXPORT)?
                .call(&mut store, (ptr, len))?,
        };

        decode_effects(self.memory_of(&mut store, &instance)?.data(&store), packed)
    }

    /// Reserve `bytes.len()` guest bytes via `mod_alloc` and copy `bytes` in,
    /// returning their `(ptr, len)`. The guest owns the allocation; a fresh store
    /// per invocation reclaims it afterward.
    fn push(
        &self,
        store: &mut Store<StoreState>,
        instance: &Instance,
        bytes: &[u8],
    ) -> Result<(u32, u32)> {
        let len = u32::try_from(bytes.len()).map_err(|_| anyhow!("input blob exceeds u32"))?;
        let alloc = instance.get_typed_func::<u32, u32>(&mut *store, ALLOC_EXPORT)?;
        let ptr = alloc.call(&mut *store, len)?;
        let memory = self.memory_of(store, instance)?;
        // `write` bounds-checks against linear memory: a bad `ptr` is an Err.
        memory.write(store, ptr as usize, bytes)?;
        Ok((ptr, len))
    }

    /// The guest's exported linear memory, or an error if it exports none.
    fn memory_of(
        &self,
        store: &mut Store<StoreState>,
        instance: &Instance,
    ) -> Result<wasmtime::Memory> {
        instance.get_memory(store, "memory").ok_or_else(|| anyhow!("guest exports no `memory`"))
    }
}

/// A compiled guest ready for repeated runtime invocation. Cheap to instantiate;
/// hold one per adopted mod and reuse it for the session.
pub struct GuestModule {
    module: Module,
}

impl GuestModule {
    /// Whether the guest exports a function named `name`. Lets the host probe the
    /// optional runtime entries (`mod_tick` / `mod_trigger`) once at load and
    /// invoke only the mods that opted in, instead of trapping on a missing export
    /// every tick.
    #[must_use]
    pub fn exports_func(&self, name: &str) -> bool {
        self.module.exports().any(|e| e.name() == name && e.ty().func().is_some())
    }
}

/// Which runtime entry point to call, plus its selector where one applies.
#[derive(Clone, Copy)]
enum Entry {
    /// `mod_handle(handler_id, ptr, len)`.
    Handle(u32),
    /// `mod_tick(ptr, len)`.
    Tick,
    /// `mod_trigger(event_id, ptr, len)`.
    Trigger(u32),
}

/// Decode a [`GuestEffects`] from `memory` at the region named by `packed`. A
/// `(0, 0)` return is the "no effects" sentinel. Total over arbitrary memory +
/// packed values: an out-of-bounds or malformed payload is an `Err`, never a
/// panic (the guest is untrusted).
fn decode_effects(memory: &[u8], packed: u64) -> Result<GuestEffects> {
    if packed == 0 {
        return Ok(GuestEffects::none());
    }
    let (ptr, len) = unpack_ptr_len(packed);
    let (ptr, len) = (ptr as usize, len as usize);
    let end = ptr.checked_add(len).ok_or_else(|| anyhow!("effects ptr+len overflows"))?;
    let bytes =
        memory.get(ptr..end).ok_or_else(|| anyhow!("effects region {ptr}..{end} out of bounds"))?;
    postcard::from_bytes(bytes).map_err(|e| anyhow!("decoding effects: {e}"))
}
