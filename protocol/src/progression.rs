//! Progression on the wire (stormlight/server#62) — a unit's level, and its
//! owner's progress toward the next one.
//!
//! Two components, because these are two facts with two audiences.
//!
//! # A level is public
//!
//! [`UnitLevel`] rides the unit entity, like [`UnitTag`](crate::identity::UnitTag)
//! and its [vitals](crate::vitals). How far along an opponent is changes how you
//! play against them, the same way their health bar does, and hiding it would only
//! mean every client guessed. It is one byte, and it changes a handful of times a
//! match.
//!
//! # Progress toward the next level is not
//!
//! [`XpProgress`] is privileged, for the reason a cooldown is
//! ([`crate::slots`]): knowing an opponent is one kill from a talent tier is
//! knowing exactly when to contest them. So it rides the **owner-scoped view
//! entity** — the same one the ability bar's state sits on, replicated with
//! `NetworkTarget::Single(owner)` — rather than layering a second, per-component
//! visibility rule over the entity-level culling. One mechanism decides who
//! receives what.
//!
//! It carries both ends of the span (`floor` and `next`) rather than a
//! pre-divided ratio, so the client can draw a bar *and* label it with the real
//! numbers, and so a HUD's rounding is a client decision.
//!
//! # Encoding
//!
//! Plain serde, unlike the hand-rolled codecs beside it. Those exist because their
//! state changes every tick for every visible unit; this changes when somebody
//! earns XP, for exactly one entity per player. Spending a custom codec here would
//! buy bytes nobody is sending.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The level every unit stands at before it has earned anything. Mirrors the
/// server's own floor, so the two ends cannot disagree about what "unlevelled"
/// means.
pub const FIRST_LEVEL: u8 = 1;

/// A unit's current level — the public half of progression.
///
/// Defaults to [`FIRST_LEVEL`], not zero: a unit that has never earned anything is
/// at the first level, and a zeroth level is a number no talent tier is keyed on.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitLevel(pub u8);

impl Default for UnitLevel {
    fn default() -> Self {
        Self(FIRST_LEVEL)
    }
}

/// The owner's progress toward its next level.
///
/// All three numbers are totals on the same scale — accumulated XP — rather than a
/// remainder, so the client never has to know how the server chose to subtract.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XpProgress {
    /// Total XP accumulated.
    pub current: f32,
    /// Total XP the current level required — the left end of the bar.
    pub floor: f32,
    /// Total XP the next level requires — the right end. `None` for a unit at its
    /// declared ceiling, which has no next level to reach.
    pub next: Option<f32>,
}

impl XpProgress {
    /// How full the bar is, in `[0, 1]`.
    ///
    /// Total over every input, because this number is read straight into a bar
    /// width and there is nowhere downstream to catch a bad one:
    ///
    /// - a unit at its ceiling reads **full**. Nothing left to earn is a finished
    ///   track, not an empty one;
    /// - a zero-width or inverted span (`next <= floor`) reads full too, for the
    ///   same reason — there is no distance left to cover;
    /// - anything non-finite reads **empty**, the honest answer when the frame
    ///   itself is nonsense;
    /// - the result is clamped, so a frame that arrives a tick after a level-up
    ///   draws a full bar rather than overflowing its track.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        let Some(next) = self.next else {
            return 1.0;
        };
        let span = next - self.floor;
        if !span.is_finite() || span <= 0.0 {
            return 1.0;
        }
        let earned = self.current - self.floor;
        if !earned.is_finite() {
            return 0.0;
        }
        (earned / span).clamp(0.0, 1.0)
    }
}

/// Register both halves for replication. Called from
/// [`crate::protocol::ProtocolPlugin`] on both ends so the protocol matches.
///
/// [`UnitLevel`] is interpolated so the smoothed copy every *other* player's unit
/// is drawn on carries it; there is nothing to ease between two integers, so the
/// "lerp" takes the confirmed value.
///
/// **Neither is predicted.** Both are server-authoritative — nothing on a client
/// simulates earning XP — and a predicted copy of state the client never advances
/// would sit at its spawn value while the real one moved on (the same trap
/// [`LifeState`](crate::death::LifeState) documents). [`XpProgress`] additionally
/// rides an entity that exists only for its owner, which is never a prediction or
/// interpolation target at all.
pub fn register(app: &mut App) {
    app.register_component::<UnitLevel>().add_interpolation_with(|_start, end, _t| end);
    app.register_component::<XpProgress>();
}
