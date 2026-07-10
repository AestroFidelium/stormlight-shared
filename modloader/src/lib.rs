//! `stormlight_modloader` — engine-side WASM mod host.
//!
//! A wasmtime runtime + host-function ABI that loads Stormlight mods (folder or
//! `.zip`), validates their manifest against [`stormlight_mod_abi`], and runs
//! `mod_register` / `mod_tick` / trigger entry points. `default-features` off on
//! wasmtime → synchronous host calls only (cranelift-friendly, no fiber asm).
//!
//! [`client`] holds the cosmetic client.wasm runtime; [`host`] the wasmtime
//! sandbox; [`loader`] reads + validates a mod package (folder or `.zip`);
//! [`registry`] decodes emitted registrations into a host-side `ModRegistry`.

pub mod client;
pub mod host;
pub mod loader;
pub mod registry;
pub mod vfs;
