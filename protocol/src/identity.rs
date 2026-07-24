//! Replicated unit identity — the one generic key the client needs to map a
//! replicated entity back to the mod-declared unit it was spawned from.
//!
//! A replicated unit otherwise carries only `Transform` (and provisional stress
//! state); nothing says *which* descriptor it is. [`UnitTag`] adds that: the
//! (global) id of the `UnitDescriptor` the server spawned it from, so the
//! content-free client cosmetic half can resolve it to the visual a cosmetic mod
//! declared for that id (server#41 → #44). The wire carries an **opaque `u32`**,
//! never a hero/unit name — the protocol crate stays content- and mod-ABI-free
//! (the server wraps its `UnitId` into this raw id at spawn).

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The (global) id of the `UnitDescriptor` a replicated unit was spawned from.
/// Stable for the entity's whole life — set once at spawn, never mutated — so a
/// client keys on it once and never has to chase a changing identity. An unknown
/// or absent id degrades to the placeholder visual on the client, never panics.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitTag(pub u32);

/// Register the identity component for replication. Called from
/// [`crate::protocol::ProtocolPlugin`] on both ends so the protocol matches.
pub fn register(app: &mut App) {
    app.register_component::<UnitTag>();
}
