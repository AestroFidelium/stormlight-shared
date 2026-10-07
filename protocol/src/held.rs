//! What is waiting to land on a unit whose time is stopped
//! (stormlight/server#223).
//!
//! Blows, heals and buffs aimed at a unit in stopped time are held on its own
//! clock and land the moment it moves again. Without a word on the wire that looks
//! like nothing at all: a player rains blows on a frozen enemy and sees no number
//! move. [`HeldImpacts`] is that word — how many deliveries are waiting on a unit,
//! and how much damage they carry — so a cosmetic mod can draw them piling up.
//!
//! # Attach once, then only ever update
//!
//! Same rule as [`LifeState`](crate::death::LifeState): the component appears the
//! first time anything is held on a unit and is then kept, reading zero when
//! nothing is waiting. A removal would reach a predicting client's own unit only
//! through a rollback, leaving the owner looking at blows that already landed.
//! Absent means nothing is waiting, so a unit that is never stopped never pays.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The deliveries waiting on a unit whose time is stopped.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeldImpacts {
    /// How many are waiting.
    pub count: u16,
    /// The damage they carry between them, as the sender fixed it at the hit —
    /// before the target's own armour and shields, which are read when it lands.
    pub damage: f32,
}

/// Register the held tally for replication on both ends.
///
/// Interpolated with the confirmed value (a count has no in-between), and **not
/// predicted**: nothing on a client holds anything.
pub fn register(app: &mut App) {
    app.register_component::<HeldImpacts>().add_interpolation_with(|_start, end, _t| end);
}
