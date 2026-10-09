//! Which side a unit is on (stormlight/server#181).
//!
//! The server resolves every allegiance — who a shot may hit, who a heal may
//! reach — against a plain team number on each unit. The client never needed it
//! until something had to be *drawn* by side: the ring under an enemy target is
//! not the one under an ally's. So the number is published as it stands, and the
//! client decides the relation from it, the same rule the server targets by: one
//! side is allied with itself and with nothing else.
//!
//! Public, like the body it sits on: which side a unit fights for is legible from
//! watching it fight, so the wire reveals nothing an opponent cannot see.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The team a unit is on, as the server's allegiance checks read it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamTag(pub u32);

impl TeamTag {
    /// The team a unit is on when its spawner assigned none: on no side at all.
    pub const NEUTRAL: Self = Self(0);
}

/// Register the replicated team tag. Neither predicted nor interpolated: nothing
/// on the client changes a unit's side, and a side is a fact, not a curve.
pub fn register(app: &mut App) {
    app.register_component::<TeamTag>();
}
