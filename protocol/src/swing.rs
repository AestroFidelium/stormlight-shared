//! The replicated **swing** state (stormlight/server#152): the one fact a client
//! needs about a unit's basic attack.
//!
//! A cast reaches the client as [`CastProgress`](crate::cast::CastProgress) —
//! present while the cast runs, gone when it ends, carrying the window and the
//! cosmetic key. The attack cycle published nothing at all, so a swing was
//! invisible twice over: no animation could be claimed for it, and no cosmetic
//! mod could dress what it fired.
//!
//! This is the attack's half of that, and it is deliberately the same shape.
//! **Presence is the state**: nothing on the client has to guess when a swing
//! starts or track when it should stop, and an interrupted swing is the component
//! disappearing — which stops the animation claim, which lets the layer blend back
//! to whatever the unit was doing. A cancelled wind-up fades rather than snapping,
//! for free, because leaving a state is always a blend.
//!
//! Content-free: a swing is a swing. Which unit, what its attack does, and what
//! any of it looks like are the descriptor's and the cosmetic mod's.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// A unit's basic attack, in flight.
///
/// Present exactly while the unit is **committed to a swing** — from the moment
/// the swing starts to the end of the recovery it owes. The wind-up alone would be
/// wrong: it ends at the swing point, which is the middle of the motion, and a
/// character that returned to its gait there would be animated for half of every
/// attack it makes.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwingProgress {
    /// Completion in `[0, 1]` — 0 as the swing starts, 1 as the commitment ends.
    pub progress: f32,
    /// The whole commitment's length in seconds: wind-up plus recovery, as the
    /// attacker's own stats had them when this swing began.
    ///
    /// Sent so a clip bound to
    /// [`RateBinding::ActionWindow`](stormlight_mod_abi::animation::RateBinding::ActionWindow)
    /// can be time-scaled to it. That is what makes one piece of art fit a unit
    /// whose attack speed a buff has doubled: without it the swing animation runs
    /// at its authored rate while the unit attacks twice as often, and the
    /// character visibly stops swinging between blows it is still landing.
    pub total: f32,
    /// Cosmetic key: the handle the attack's descriptor declared, so the client
    /// can dress the swing's shot and its impact. `0` means "no specific look" and
    /// the client draws its neutral placeholder.
    pub vfx: u32,
}

/// Register the replicated swing state. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so both ends agree.
///
/// Replicated plainly, **not predicted**: nothing on the client simulates an
/// attack cycle, so a predicted copy would be driven by the client's own
/// simulation and reconciled only through a rollback (stormlight/server#86).
pub fn register(app: &mut App) {
    app.register_component::<SwingProgress>();
}
