//! The one player command on the wire: a **cast intent** (client→server).
//!
//! The client asks to cast the ability bound to a slot, aimed at a unit, a
//! point, or a direction; the server is authoritative and treats it as a request
//! it validates before acting (slot in range, the sender actually controls a
//! caster — see the server's `Input` phase). Reliable, unordered: a dropped cast
//! would feel broken, but stale ordering between two casts does not matter (each
//! carries its own aim).
//!
//! Content-free: a slot index plus a geometric/entity target, nothing ability-
//! or hero-specific. What slot *means* and which ability it binds to is the
//! unit's loadout (mod content), resolved server-side.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// How a cast is aimed. `Unit` references a target entity (mapped across the wire
/// by [`CastIntent`]'s [`MapEntities`]); `Point`/`Vector` carry world-space
/// coordinates; `None` is a self / no-target cast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum Aim {
    /// No target — a self-cast or an ability that needs no aim.
    #[default]
    None,
    /// A target unit (a replicated entity, mapped to the server's id on receipt).
    Unit(Entity),
    /// A world-space ground point.
    Point(Vec3),
    /// A world-space aim direction.
    Vector(Vec3),
}

/// Client→server request to cast the ability in `slot`, aimed by `aim`. Only a
/// request: the server validates and resolves it against the caster's loadout
/// (the engine stays authoritative). `slot` is a raw index — it is interpreted
/// against the caster's ability slots server-side, never trusted blindly.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CastIntent {
    pub slot: u8,
    pub aim: Aim,
}

impl MapEntities for CastIntent {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // Only a unit-targeted aim carries an entity; the rest are pure scalars.
        if let Aim::Unit(target) = &mut self.aim {
            *target = entity_map.get_mapped(*target);
        }
    }
}

/// Reliable channel the cast intents ride. Unordered-reliable: every cast is
/// delivered exactly once (a dropped input is unacceptable) but two casts need no
/// mutual ordering — each is self-describing.
pub struct CastIntentChannel;

/// Replicated live-cast state, server→client — the render key for a cast/channel
/// **indicator**. The server authoritatively runs the timed cast on the caster;
/// it mirrors that timing here so a client can draw an indicator that fills as the
/// cast completes. Attached to the caster entity while a cast is in progress and
/// removed when it ends, so its presence *is* "this unit is casting".
///
/// Content-free: a raw slot, a completion fraction, and an opaque cosmetic key —
/// never an ability/hero name. Because the caster is already a replicated entity,
/// this needs no `Replicate` of its own; being present on the caster replicates
/// it to every client that can see the unit.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CastProgress {
    /// The slot being cast (raw index; interpreted against the caster's loadout).
    pub slot: u8,
    /// Completion in `[0, 1]` — 0 at cast start, 1 at resolution. The client fills
    /// its indicator by this fraction.
    pub progress: f32,
    /// The cast window length in seconds (for an indicator that wants absolute
    /// time, not just the fraction). `0` for an effectively-instant cast.
    pub total: f32,
    /// Cosmetic key: the (global) id of the ability being cast, so the client can
    /// pick the indicator visual (`EffectRole::CastIndicator`). `0` means "no
    /// specific visual" — the client draws its neutral placeholder.
    pub vfx: u32,
}

/// Register the cast wire contract on both ends: the reliable client→server
/// [`CastIntent`] channel/event (with entity mapping for `Aim::Unit`), and the
/// replicated server→client [`CastProgress`] indicator state. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<CastIntentChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);

    app.register_event::<CastIntent>()
        .add_map_entities()
        .add_direction(NetworkDirection::ClientToServer);

    app.register_component::<CastProgress>();
}
