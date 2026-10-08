//! Bodies an ability leaves standing in the world, as a client sees them
//! (stormlight/server#220).
//!
//! A missile reaches a client as its launch event and is simulated there; the
//! three bodies that do not travel — a ground zone, an item lying on the ground
//! and a summoned unit — stay where they are put, so each is an ordinary
//! replicated entity carrying [`BodyFact`]: what kind of body it is, how far it
//! reaches, and the cosmetic key its art is resolved from. Its position is the
//! entity's own `Transform`, and its end is the entity's despawn: a zone running
//! out and an item's last charge going are the server despawning the body, which
//! takes it off every client with nothing more said.
//!
//! Content-free: a kind, a radius and an opaque key. Nothing here knows what was
//! spawned — the key is the spawning ability's global id, exactly as a missile's
//! `vfx` is.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Which of the non-travelling bodies an entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BodyKindFact {
    /// A unit an ability summoned. It is a unit in every other respect — vitals,
    /// death, a nameplate — and this says only who to ask for its art.
    Summon,
    /// A ground area that works on what stands in it.
    Zone,
    /// An item lying on the ground, waiting to be collected.
    Pickup,
}

/// The public fact about a body that stays in the world.
///
/// Set once at spawn and never changed: a body's kind, reach and key are fixed by
/// the payload that made it, so a client reads them once.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyFact {
    /// What kind of body it is.
    pub kind: BodyKindFact,
    /// How far it reaches on the ground, in world units — a zone's area, an item's
    /// collection reach; zero for a summon. Always finite and non-negative.
    pub radius: f32,
    /// The spawning ability's global id, the key its art is resolved from (`0` =
    /// none).
    pub vfx: u32,
}

impl BodyFact {
    /// The fact for a body of `kind` reaching `radius`, keyed by `vfx`.
    ///
    /// A radius a payload computed as non-finite or negative is carried as zero:
    /// the body's simulation clamps it the same way, and a NaN reaching a client
    /// would cost the frame the body is drawn in rather than the body.
    #[must_use]
    pub fn new(kind: BodyKindFact, radius: f32, vfx: u32) -> Self {
        let radius = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
        Self { kind, radius, vfx }
    }
}

/// Register the body fact for replication on both ends.
///
/// Plain, with no interpolation: the fact never changes, so there is nothing to
/// ease, and it must land **with** the spawn rather than on the interpolation
/// timeline — a summon is dressed the moment it is placed, and is dressed from
/// this.
pub fn register(app: &mut App) {
    app.register_component::<BodyFact>();
}
