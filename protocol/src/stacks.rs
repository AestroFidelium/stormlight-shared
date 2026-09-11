//! The owner's stack counters on the wire (stormlight/server#132).
//!
//! A stack counter is the ISA's reserve for *how many times something has
//! happened* — combo points, a charge-up, the count behind a quest. Every other
//! reserve a widget can name already crossed the wire; this one did not, so a HUD
//! binding to one resolved to nothing, always, and a talent that set the player a
//! task could show the task but never the progress.
//!
//! # Privileged, like the three views it rides beside
//!
//! [`ReplicatedStacks`] lives on the **owner-scoped view entity** — the one the
//! ability bar ([`crate::slots`]), the XP bar ([`crate::progression`]) and the
//! talent view ([`crate::talents`]) already sit on, replicated with
//! `NetworkTarget::Single(owner)`. The reason is the reason those three give:
//! knowing an opponent is three casts from a payout is knowing exactly when not to
//! contest them, the same way knowing their cooldowns is. One mechanism decides who
//! receives what — which entity the state sits on — rather than a second,
//! per-component visibility rule layered over the entity-level culling
//! (stormlight/server#8).
//!
//! A counter a mod wants everybody to see is a later, deliberate feature. It is not
//! a silent divergence between two components on the same entity.
//!
//! # Opaque ids, and no ceiling
//!
//! Each entry is `(opaque id → count)`. The protocol crate never learns what a
//! counter *means*, exactly as it does not for a resource pool: the server's
//! authoritative store is keyed by a mod-global handle and the raw number is all
//! that crosses.
//!
//! There is deliberately **no maximum**. Nothing in the ISA declares one for a
//! stack counter, so inventing one here would be the wire asserting a fact no mod
//! stated. What a count is *toward* belongs to whoever is reading it — for a quest
//! that is the goal on the talent's own
//! [`QuestSpec`](stormlight_mod_abi::talents::QuestSpec), which both ends hold as
//! content and neither pays a wire cost for.
//!
//! # Encoding
//!
//! Plain serde, like [`crate::talents`] and [`crate::progression`] beside it,
//! rather than the hand-rolled codecs [`crate::vitals`] and [`crate::pools`] use.
//! Those exist because their state changes every tick for every visible unit; this
//! is a handful of small counters, for exactly one entity per player, changing when
//! that player does something that counts. A custom codec here would buy bytes
//! nobody is sending.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// One counter's replicated state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StackCount {
    /// The counter's opaque (mod-global) stack id — the reader's only handle for
    /// telling one of a unit's counters from another.
    pub id: u32,
    /// How many. Always finite and non-negative; see [`StackCount::new`].
    pub current: f32,
}

impl StackCount {
    /// Build with the invariants forced: a non-finite count collapses to zero and a
    /// negative one to empty.
    ///
    /// Floored here rather than by each reader, because this number is read
    /// straight into a printed figure and a bar width, and there is nowhere
    /// downstream to catch a bad one — a NaN does not cost one widget but the whole
    /// layout.
    #[must_use]
    pub fn new(id: u32, current: f32) -> Self {
        Self { id, current: if current.is_finite() { current.max(0.0) } else { 0.0 } }
    }
}

/// A unit's replicated stack counters, in the server's (deterministic, id-ordered)
/// order.
///
/// Present on the owner's view entity only while that owner's unit actually holds a
/// counter, so its presence is precisely "there is progress here worth reading".
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplicatedStacks(pub Vec<StackCount>);

impl ReplicatedStacks {
    /// The count standing in one counter, or `None` for a counter this unit does
    /// not hold.
    ///
    /// Absence and zero are different answers on purpose: "no such counter" is what
    /// makes a reader draw nothing at all, while "this counter stands at zero" is an
    /// empty bar. Collapsing the two would have a quest look finished-and-forgotten
    /// exactly as often as it looked unstarted.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<f32> {
        self.0.iter().find(|count| count.id == id).map(|count| count.current)
    }
}

/// Register the owner's counter view on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// Neither predicted nor interpolated, for the reasons its neighbours on the same
/// entity are not ([`crate::slots`], [`crate::talents`]): nothing on the client
/// simulates a counter, so predicting one would predict a number the client has no
/// rule for; and easing between two snapshots of a count would invent fractional
/// progress the server never published — "37.4 of 40" is not a state this quantity
/// has.
pub fn register(app: &mut App) {
    app.register_component::<ReplicatedStacks>();
}
