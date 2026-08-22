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
///
/// Registered to sync onto **both** client-side mirrors of a replicated unit:
///
/// - the smoothed `Interpolated` copy every other player's unit is rendered on;
/// - the `Predicted` copy the player's *own* unit is rendered on (server#50).
///
/// Both matter, because the cosmetic layer resolves a unit's visual from its
/// identity wherever that unit is drawn — and a controlled unit is only ever
/// predicted, never interpolated.
///
/// A discrete id has no meaningful in-between, so the interpolation "lerp" is the
/// identity — take the confirmed value (server#41/#44).
///
/// Reaching the predicted mirror needs no prediction registration (server#86):
/// prediction is for state the client *simulates*, and a replicated component
/// lands on the mirror either way. Registering an identity for prediction only
/// bought a per-tick history buffer for a number that never changes.
pub fn register(app: &mut App) {
    app.register_component::<UnitTag>().add_interpolation_with(|_start, end, _t| end);
}
