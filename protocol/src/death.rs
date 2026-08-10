//! The replicated alive/dead state (stormlight/server#61).
//!
//! A unit that has been killed but not removed is a state a client cannot infer.
//! Its `Transform` still arrives (it is standing where it fell), its
//! [`UnitTag`](crate::identity::UnitTag) is unchanged, and its
//! [`ReplicatedVitals`](crate::vitals::ReplicatedVitals) read zero health — which
//! is *nearly* the same question, but not the same one: the server's death state
//! is latched, it survives the moment health is refilled on the way back, and a
//! client that guessed "hp == 0" would be reading a quantized number to decide
//! whether to draw a corpse.
//!
//! So the state is stated outright, and a client that sees [`LifeState::Downed`]
//! stops drawing a live unit as live (a death animation in M9, a grayed-out
//! portrait in M10).
//!
//! # Why a state and not a marker
//!
//! The obvious encoding is a marker component whose *presence* means "dead", and
//! it is the wrong one. A marker has to be **removed** when the unit comes back,
//! and a removal does not reach a client's `Predicted` mirror the way an update
//! does: Lightyear reconciles a removed component only through a rollback, while
//! an ordinary value change is synced every tick. The failure that buys you is
//! precise and terrible — the player's own hero respawns, every other client sees
//! it stand up, and the player who owns it keeps looking at a corpse. Pinned by
//! `crossbeam_death`, which reads both mirrors for exactly this reason.
//!
//! A value that flips instead is immune: after the first death the component is
//! simply there, and every later transition is an update. **Absent means alive**,
//! so a unit that never dies never pays for the component at all.
//!
//! Content-free, like everything else here: nothing in this file knows what died,
//! why, or whether it will return.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Whether a replicated unit is up or down.
///
/// The server-side state this projects (`stormlight_server::death::Dead`) carries
/// the tick it died on and the deadline it returns at; neither crosses the wire.
/// A client draws a corpse, not a countdown — a respawn timer, when a HUD wants
/// one, is state the owning player alone may read, not a public fact about every
/// body on the field.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeState {
    /// Up and acting. Also what the absence of this component means, so a unit
    /// that has never died carries nothing.
    #[default]
    Alive,
    /// Killed, and not yet returned.
    Downed,
}

impl LifeState {
    /// Whether this reads as dead. Written as a method so a client asking the
    /// question never has to spell out the match — including on the `Option` a
    /// unit that has never died presents.
    #[must_use]
    pub fn is_downed(self) -> bool {
        matches!(self, LifeState::Downed)
    }
}

/// Register the death state for replication. Called from
/// [`crate::protocol::ProtocolPlugin`] on both ends so the protocol matches.
///
/// Interpolated, so the smoothed copy every other player's unit is drawn on
/// carries it. Alive and dead have no in-between to ease through, so the "lerp"
/// takes the confirmed value.
///
/// **Deliberately not predicted**, unlike
/// [`UnitTag`](crate::identity::UnitTag) beside it. Prediction is for state the
/// client simulates alongside the server: a predicted component is driven by the
/// client's own simulation and only reconciled with the server through a
/// rollback. Nothing on a client simulates dying, so a predicted death state
/// would sit at whatever it was when the entity was spawned — leaving the owning
/// player looking at their own corpse long after everyone else watched them stand
/// up. Left unpredicted, replication writes each update straight onto the entity,
/// which is exactly the behaviour server-authoritative state wants. Pinned
/// end-to-end by `crossbeam_death`, which reads both mirrors.
pub fn register(app: &mut App) {
    app.register_component::<LifeState>().add_interpolation_with(|_start, end, _t| end);
}
