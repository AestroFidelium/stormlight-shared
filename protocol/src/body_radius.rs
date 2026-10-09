//! How wide a unit's body is (stormlight/server#181).
//!
//! The simulation decides where a shot makes contact against each unit's body
//! radius — its declared `body_radius` stat, or the engine's default for a unit
//! that declares none. The client draws things sized to that body: the ring under
//! a selected unit fits a siege engine and a rat alike with one declaration,
//! because it is the body's width, not a number per unit. So the radius the
//! simulation uses is published as it stands, never re-derived: a client cannot
//! aggregate stats, and a ring sized by a guess would disagree with the hits.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// A unit's body radius, world units: the distance a shot aimed at it makes
/// contact within.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyRadius(pub f32);

/// Register the replicated body radius. Neither predicted nor interpolated: it
/// changes when a buff changes the stat, and then it changes at once.
pub fn register(app: &mut App) {
    app.register_component::<BodyRadius>();
}
