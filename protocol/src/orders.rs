//! Player **orders** on the wire (client→server) — the queue half of the control
//! model (stormlight/server#90).
//!
//! A unit used to hold exactly one intent: the latest move order replaced
//! whatever it was doing, and nothing could be taken back once it was in flight.
//! A player could not express a plan and could not cancel an action, and both
//! gaps are the same missing thing — orders are a **queue**, not a variable.
//!
//! So there is one message, and every intent is an entry of the same kind. That
//! is what lets queueing, cancelling and reporting be written once instead of
//! once per intent: the server's queue never asks *which* kind an entry is in
//! order to decide whether it may be appended or dropped.
//!
//! Two things travel with every order:
//!
//! - the **kind** and its payload ([`OrderKind`]) — walk here, attack that, walk
//!   there fighting what you meet, cast this, stop, hold;
//! - whether it **replaces** the plan or is **appended** to it ([`OrderMode`]).
//!
//! The modifier a player holds to queue (conventionally shift) is pure client
//! convention, exactly like the keybind→slot mapping: the wire carries "append"
//! or "replace" and never a key. A different client, or a rebound key, changes
//! nothing here.
//!
//! **Every order is a request.** The server owns the queue: it resolves the
//! sender to the unit it controls, validates the payload, and applies the whole
//! entry or none of it. Nothing in this module is authoritative.
//!
//! Content-free: world points, a target entity, a raw slot index. Nothing here
//! names a hero, an ability, or a unit type.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cast::Aim;

/// What a single order tells a unit to do. One flat vocabulary of generic
/// intents — no kind is hero- or ability-specific.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderKind {
    /// Walk to a world-space ground point. Retires on arrival.
    Move { target: Vec3 },
    /// Engage a specific unit: close to attack range, then attack it until it
    /// dies or stops being a legal target. Retires when there is nothing left to
    /// attack.
    Attack { target: Entity },
    /// Walk to a ground point, engaging whatever legal target comes into range
    /// on the way and resuming the walk once it is gone. Retires on arrival.
    AttackMove { target: Vec3 },
    /// Cast the ability in `slot`, aimed by `aim`, when this entry comes up.
    ///
    /// This is the *queued* form. An immediate cast is not an order at all — it
    /// is [`CastIntent`](crate::cast::CastIntent), and it deliberately leaves the
    /// queue alone, because a player casting while walking has not changed their
    /// plan. Only a cast a player explicitly put *in* the plan lands here.
    Cast { slot: u8, aim: Aim },
    /// Drop everything: clear the queue and abandon the current action. Never
    /// becomes a queue entry of its own — a unit that has stopped is a unit with
    /// an empty queue.
    Stop,
    /// Clear the queue and hold position: engage what comes into range, but never
    /// move to do it. Unlike [`Stop`](OrderKind::Stop) this *is* a standing
    /// entry, because holding is something a unit keeps doing.
    Hold,
}

/// Whether an order replaces the plan or extends it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderMode {
    /// Clear the queue and start this order now — a plain click.
    #[default]
    Replace,
    /// Add this order to the end of the plan — a queued (shift) click.
    Append,
}

/// Client→server request to give the sender's controlled unit an order.
///
/// Only a request: the server validates ownership and the payload, and owns the
/// queue the entry lands in. A sender that controls nothing is ignored.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub kind: OrderKind,
    pub mode: OrderMode,
}

impl OrderRequest {
    /// A plain (queue-replacing) order.
    #[must_use]
    pub fn now(kind: OrderKind) -> Self {
        Self { kind, mode: OrderMode::Replace }
    }

    /// A queued order, appended behind whatever is already planned.
    #[must_use]
    pub fn queued(kind: OrderKind) -> Self {
        Self { kind, mode: OrderMode::Append }
    }
}

impl MapEntities for OrderRequest {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // Exactly the two kinds that name a unit; the rest are pure scalars and
        // world points, which must cross untouched.
        match &mut self.kind {
            OrderKind::Attack { target } => *target = entity_map.get_mapped(*target),
            OrderKind::Cast { aim: Aim::Unit(target), .. } => {
                *target = entity_map.get_mapped(*target);
            }
            OrderKind::Move { .. }
            | OrderKind::AttackMove { .. }
            | OrderKind::Cast { .. }
            | OrderKind::Stop
            | OrderKind::Hold => {}
        }
    }
}

/// Reliable channel the orders ride.
///
/// **Ordered**, unlike the cast-intent channel, and that is the whole difference
/// a queue makes: two casts are each self-describing so their arrival order is
/// irrelevant, but two appends are positions in a plan. Delivered out of order
/// they build a different plan from the one the player typed — the hero walks the
/// waypoints backwards — which is indistinguishable from the client having sent
/// something else.
pub struct OrderChannel;

/// Register the order wire contract on both ends: the reliable, ordered
/// client→server [`OrderRequest`] channel + event, with entity mapping for the
/// kinds that name a unit. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<OrderChannel>(ChannelSettings {
        mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);

    app.register_event::<OrderRequest>()
        .add_map_entities()
        .add_direction(NetworkDirection::ClientToServer);
}
