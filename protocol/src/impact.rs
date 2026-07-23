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
//! Content-free: it carries a world point and an opaque cosmetic key, never a
//! hero/ability name. The client's cosmetic half maps the key to the burst it
//! draws (`EffectRole::Impact`); an unknown/`0` key degrades to a neutral
//! placeholder so a hit is always *seen*.

use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The one-shot "a shot landed here" event, server→client. Small and fixed-size:
/// a world point plus a cosmetic key. The client plays a transient impact visual
/// at `at`, picking the burst from `vfx`; nothing is replicated afterwards.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImpactEvent {
    /// Where the shot authoritatively impacted, in world space.
    pub at: Vec3,
    /// Cosmetic key: the (global) id of the ability whose shot landed, so the
    /// client can pick the impact burst (`EffectRole::Impact`). `0` means "no
    /// specific visual" — the client draws its neutral placeholder. Content-free:
    /// an opaque id, never a hero/ability name.
    pub vfx: u32,
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

    app.register_event::<ImpactEvent>().add_direction(NetworkDirection::ServerToClient);
}
