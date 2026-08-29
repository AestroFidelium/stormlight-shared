//! Authoritative impact feedback — the one-shot "a shot landed here" event
//! (server→client), the counterpart to the projectile launch ([`crate::projectiles`]).
//!
//! A projectile's flight is drawn locally by every client from a single launch
//! event; its **impact** is the server's authority — only the server knows where
//! and when a shot actually connects. So the server announces the hit once as a
//! small [`ImpactEvent`], and each client plays a transient impact visual there.
//! Like the launch it is a reliable one-shot, not a per-tick stream: the impact
//! either happened or it didn't.
//!
//! Content-free: it carries a world point, an opaque cosmetic key and the unit
//! that was struck, never a hero/ability name. The client's cosmetic half maps the
//! key to the burst it draws (`EffectRole::Impact`); an unknown/`0` key degrades to
//! a neutral placeholder so a hit is always *seen*.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The one-shot "a shot landed here" event, server→client. Small and fixed-size:
/// a world point, a cosmetic key, and whoever it landed on. The client plays a
/// transient impact visual at `at`, picking the burst from `vfx`; nothing is
/// replicated afterwards.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImpactEvent {
    /// Where the shot authoritatively impacted, in world space.
    pub at: Vec3,
    /// Cosmetic key: the (global) id of the ability whose shot landed, so the
    /// client can pick the impact burst (`EffectRole::Impact`). `0` means "no
    /// specific visual" — the client draws its neutral placeholder. Content-free:
    /// an opaque id, never a hero/ability name.
    pub vfx: u32,
    /// The unit the shot struck, if it struck one — mapped to the receiver's local
    /// entity on arrival. `None` for a shot that reached the end of its flight
    /// without connecting, which is a burst on the ground and nothing more.
    ///
    /// It rides along so a client can react *on the unit that was hit* (the hit
    /// animation, server#74) rather than guessing from the impact point. A
    /// one-shot event is the right place for a mapped reference — unlike a
    /// replicated component, it is resolved against the entity map at the moment
    /// it arrives, and a victim the receiver cannot see resolves to nothing and is
    /// simply ignored.
    pub victim: Option<Entity>,
    /// The shot this impact ended, echoing
    /// [`ProjectileFired::shot`](crate::projectiles::ProjectileFired::shot), or
    /// `0` when no shot produced it (stormlight/server#153).
    ///
    /// **Zero is the common case**, not an error: a melee blow, a zone tick and a
    /// notify-driven burst all land without anything having travelled. What the id
    /// buys is the other case — the client retiring the drawing of the shot that
    /// just landed, instead of letting it sail on to the end of its range.
    pub shot: u32,
}

impl MapEntities for ImpactEvent {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // The impact point and the cosmetic key are plain values; the struck unit
        // is the only handle here, and a victimless impact stays victimless.
        if let Some(victim) = &mut self.victim {
            *victim = entity_map.get_mapped(*victim);
        }
    }
}

/// Reliable channel the one-shot impact events ride. Reliable (not per-tick) so a
/// hit is delivered exactly once and never dropped — a missed impact would leave
/// a client with a projectile that vanished with no feedback. One small message
/// per hit, flat in projectile count like the launch.
pub struct ImpactChannel;

/// Register the impact wire contract on both ends: the reliable channel and the
/// server→client [`ImpactEvent`]. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<ImpactChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ServerToClient);

    app.register_event::<ImpactEvent>()
        .add_map_entities()
        .add_direction(NetworkDirection::ServerToClient);
}
