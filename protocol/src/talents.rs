//! Talent picking on the wire (stormlight/server#63) — the player's choice, and
//! the owner-scoped view they choose from.
//!
//! The talent machinery has always been able to change what a unit's abilities do;
//! what it lacked was a way for the **player** to say which talents. That is two
//! halves, and they are not symmetric.
//!
//! # The pick is a request, not a write
//!
//! [`TalentPick`] names a tier and the talent wanted from it. Like
//! [`CastIntent`](crate::cast::CastIntent) it is only a request: the server
//! resolves the sender to the unit they control, checks the tier is unlocked at
//! that unit's level, that the talent is one that tier offers, and that the tier is
//! not already decided — and applies nothing at all if any of that fails. A client
//! that sends a pick for a tier it has not earned changes nothing.
//!
//! It carries **no entity**. The server already knows which unit a peer drives (the
//! same resolution a move order uses), so putting the caster on the wire would only
//! add a mapped reference for a client to get wrong, or to lie about.
//!
//! Reliable and unordered, for the reason a cast intent is: a dropped choice would
//! read as the game ignoring a click, while two picks need no mutual ordering —
//! each names its own tier, and two picks at the same tier are a conflict the
//! server's rules settle rather than the channel's.
//!
//! # The view is privileged
//!
//! [`ReplicatedTalents`] is what the player reads their own tree state from: which
//! tiers are unlocked, what has been chosen, and therefore what is still waiting on
//! them. It rides the **owner-scoped view entity** — the one the ability bar
//! ([`crate::slots`]) and the XP bar ([`crate::progression`]) already sit on,
//! replicated with `NetworkTarget::Single(owner)` — because knowing an opponent's
//! picks is knowing exactly which cooldowns to play around. One mechanism decides
//! who receives what: which entity the state sits on.
//!
//! What it deliberately does **not** carry is the tree's *options*. Which talents a
//! tier offers is content, identical for every player driving that unit and known
//! before the match starts; replicating it per player per tick would be paying a
//! wire cost for a constant.
//!
//! # Encoding
//!
//! Plain serde, like [`crate::progression`] and for the same reason: this changes
//! when a player levels or picks — a handful of times a match, for one entity per
//! player — so a hand-rolled codec would buy bytes nobody is sending.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Client→server request: take `talent` from the tier at index `tier`.
///
/// Both fields are raw and untrusted. `tier` is an index into the unit's declared
/// tree and `talent` an opaque global talent id; the server interprets them against
/// the unit the sender actually controls, and drops anything that does not check
/// out. Content-free: a row number and a handle, never a hero or a talent name.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TalentPick {
    /// Which tier of the unit's tree the choice is being made in.
    pub tier: u8,
    /// The chosen talent's opaque global id.
    pub talent: u32,
}

/// Reliable channel the talent picks ride. Unordered-reliable, like the cast
/// intents: every pick must arrive, and two picks need no mutual ordering.
pub struct TalentPickChannel;

/// One tier as its owner sees it.
///
/// Three facts, of which only two are sent: whether the tier is open, and what is
/// chosen in it. Whether it is *waiting* on the player is derived from those two
/// ([`pending`](Self::pending)) rather than published beside them, so there is no
/// third flag that can disagree with the pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierView {
    /// The tier's index in the unit's tree — the number a [`TalentPick`] names.
    pub tier: u8,
    /// Whether the unit's current level has opened this tier.
    ///
    /// Derived server-side from the level every time it is published, never
    /// latched: a unit that loses a level closes the tier again.
    pub unlocked: bool,
    /// The talent chosen here, if any — an opaque global id.
    pub chosen: Option<u32>,
}

impl TierView {
    /// Whether this tier is waiting on the player: open, and nothing taken yet.
    #[must_use]
    pub fn pending(&self) -> bool {
        self.unlocked && self.chosen.is_none()
    }
}

/// The owner's whole talent state, one row per reachable tier of its unit's tree,
/// in tier order.
///
/// Lives on the owner-scoped view entity (see the module docs), never on the unit
/// itself — the unit is replicated to everybody.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicatedTalents(pub Vec<TierView>);

impl ReplicatedTalents {
    /// The row for one tier, if the unit's tree has it. Total: a client may ask
    /// about any index, including one this tree never declared.
    #[must_use]
    pub fn tier(&self, tier: u8) -> Option<&TierView> {
        self.0.iter().find(|row| row.tier == tier)
    }

    /// Every tier still waiting on a choice — what a HUD badges.
    pub fn pending(&self) -> impl Iterator<Item = u8> + '_ {
        self.0.iter().filter(|row| row.pending()).map(|row| row.tier)
    }

    /// Every talent chosen so far, in tier order.
    pub fn chosen(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().filter_map(|row| row.chosen)
    }
}

/// Register the talent wire contract on both ends: the reliable client→server
/// [`TalentPick`] channel/event and the replicated owner-scoped
/// [`ReplicatedTalents`] view. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// The pick registers **no entity mapping**, because it carries no entity.
///
/// The view is neither predicted nor interpolated, for the reasons its neighbours
/// on the same entity are not ([`crate::slots`]): it is a plain confirmed replica
/// of server-authoritative state, nothing on the client simulates a talent choice,
/// and easing two snapshots of a discrete choice would invent a state the server
/// never published.
pub fn register(app: &mut App) {
    app.add_channel::<TalentPickChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);

    app.register_event::<TalentPick>().add_direction(NetworkDirection::ClientToServer);

    app.register_component::<ReplicatedTalents>();
}
