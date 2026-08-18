//! What a unit just lost or regained — the report a mod's HUD prints as floating
//! combat text (stormlight/server#93), server→client.
//!
//! The sibling of [`crate::impact`], and deliberately not the same message. An
//! impact says *a shot landed here*, which is a cosmetic fact about a projectile: it
//! is announced in the impact phase, before the payload's damage has been resolved,
//! and only a projectile ever produces one. What a popup needs is the opposite end
//! of the pipeline — how much actually landed, after mitigation, from **whatever**
//! dealt it: a melee blow, an area effect, a beam that ticks, a buff expiring. That
//! is known only where the damage is applied, so that is where this is reported
//! from.
//!
//! ## Coalesced before it is sent
//!
//! One message per blow would be honest and wasteful: a beam ticking every
//! simulation tick on a dozen units is a thousand datagrams a second, to say
//! something no player can read at that rate. So the server accumulates a window's
//! worth of blows per `(unit, kind, cause)` and sends **one** report carrying their
//! sum — see [`FEED_WINDOW`]. The number a client receives is therefore "this much
//! landed on this unit, from this cause, over the last moment", which is both
//! cheaper and more legible than the six ones it replaces.
//!
//! [`VitalChange::fold`] is that arithmetic, and it lives here rather than on the
//! server because both ends do it: the server folds a window into a message, and the
//! client folds messages into the popup already on screen (the ABI's
//! [`Coalesce`](stormlight_mod_abi::ui_event::Coalesce)). One definition, so the
//! total a HUD prints cannot disagree with the total the server meant.
//!
//! Content-free: a unit, two magnitudes, a count and an **opaque** cause key — never
//! an ability's or a hero's name.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// How long the server accumulates blows before reporting them.
///
/// The one number that decides what this costs. At zero it would be a message per
/// blow per client; at a tenth of a second it is at most ten per unit per cause,
/// **whatever the tick rate**, which is already faster than a number can be read.
/// Long enough to collapse a per-tick beam by an order of magnitude, short enough
/// that a burst of separate hits still arrives as separate numbers.
///
/// It bounds the wire only. How long a *popup* lives, and whether a second report
/// joins it, is the mod's — declared on its
/// [`RootVisibility::OnEvent`](stormlight_mod_abi::ui::RootVisibility::OnEvent).
pub const FEED_WINDOW: f32 = 0.1;

/// Which way a unit's vitals moved. Two verbs, because damage and healing are two
/// verbs in the effect ISA as well, and because a HUD paints them differently — a
/// signed number would make "was I hit or healed" a question about a sign bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum VitalChangeKind {
    /// Health (and any shield in front of it) was spent.
    Damage,
    /// Health was restored.
    Heal,
}

/// One authoritative report: what a unit lost or regained over the server's
/// accumulation window, and what caused it.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VitalChange {
    /// The unit it happened to — mapped to the receiver's local entity on arrival.
    ///
    /// A report about a unit this client cannot see resolves to nothing and is
    /// dropped, which is the right answer: there is nowhere on screen to draw a
    /// number for something that is not on screen.
    pub unit: Entity,
    /// How much landed in total, **after mitigation and including whatever a shield
    /// absorbed**.
    ///
    /// The shield part is counted deliberately. A blow taken entirely on a shield
    /// costs exactly what it says, and a popup that reported only the health lost
    /// would show nothing at all for it — the player would watch their shield
    /// evaporate to a screen insisting nothing was happening.
    pub amount: f32,
    /// Of [`Self::amount`], the part a shield took rather than health. Always `0.0`
    /// for [`VitalChangeKind::Heal`] and for a unit with no shield up.
    pub absorbed: f32,
    /// How many individual blows this report is the sum of. At least `1`.
    pub hits: u8,
    /// Which way the vitals moved.
    pub kind: VitalChangeKind,
    /// **Opaque** cosmetic key: the global id of the ability responsible, the same
    /// key a projectile carries as its `vfx`. `0` means "nothing named it" — a blow
    /// from a buff expiring, or from anything the engine drove itself.
    ///
    /// It exists for one reason: it is what lets the client tell a beam's own ticks
    /// apart from a blow that lands beside them, so the two get separate numbers
    /// without the engine ever knowing what a beam *is*. Never a name.
    pub cause: u32,
}

impl VitalChange {
    /// A single blow, before anything has been folded into it.
    #[must_use]
    pub fn one(
        unit: Entity,
        amount: f32,
        absorbed: f32,
        kind: VitalChangeKind,
        cause: u32,
    ) -> Self {
        Self { unit, amount, absorbed, hits: 1, kind, cause }
    }

    /// Whether two reports describe the same thing happening to the same unit —
    /// the key a window accumulates on.
    #[must_use]
    pub fn same_source(&self, other: &Self) -> bool {
        self.unit == other.unit && self.kind == other.kind && self.cause == other.cause
    }

    /// Absorb `other` into this report: the magnitudes add and the count grows.
    ///
    /// The identity — which unit, which kind, which cause — is this report's and is
    /// never taken from the one being folded in. A caller that folds two unrelated
    /// blows together has already made its mistake at the key; this is arithmetic,
    /// not a decision about what belongs with what (see [`Self::same_source`]).
    ///
    /// The count saturates rather than wrapping. A popup that has absorbed 255 blows
    /// and says so is a popup telling the truth about a beam; one that wrapped to
    /// zero would say the hit never happened.
    pub fn fold(&mut self, other: &Self) {
        self.amount += other.amount;
        self.absorbed += other.absorbed;
        self.hits = self.hits.saturating_add(other.hits);
    }
}

impl MapEntities for VitalChange {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // The magnitudes and the cause key are plain values; the unit it happened
        // to is the only handle here.
        self.unit = entity_map.get_mapped(self.unit);
    }
}

/// Reliable channel the vital reports ride.
///
/// Reliable, like the impact it accompanies: a dropped report is a number the
/// player never sees for damage they definitely took, and the accumulation window
/// means there are few enough of them that reliability is affordable. Unordered —
/// each report is already a self-contained sum, so nothing is read out of a
/// sequence.
pub struct VitalFeedChannel;

/// Register the vital-feed wire contract on both ends: the reliable channel and the
/// server→client [`VitalChange`]. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<VitalFeedChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ServerToClient);

    app.register_event::<VitalChange>()
        .add_map_entities()
        .add_direction(NetworkDirection::ServerToClient);
}
