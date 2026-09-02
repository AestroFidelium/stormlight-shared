//! Event-driven projectiles — the wire contract and the shared motion law
//! (stormlight/server#9).
//!
//! A projectile is not replicated as a per-tick stream of `Transform` diffs (one
//! more mover per shot per client — the cost that dominates a horde). Instead the
//! server sends the **launch** once as a small one-shot [`ProjectileFired`] event
//! and every client spawns, simulates, and renders the shot **locally**; the
//! server stays authoritative on the actual impact. Egress is one datagram per
//! shot regardless of flight time, instead of `Transform` bytes every tick.
//!
//! This module is content-free: it carries pure kinematics, no hero/ability
//! specifics. The straight-line law below is the single source of truth both the
//! authoritative server and every client integrate — same launch, same law, same
//! clock ⇒ agreeing trajectories, so the local visual tracks the authoritative
//! flight without streaming a single position.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// World position of a projectile `elapsed` seconds after launch: straight-line
/// travel from `origin` along `velocity` (world units/second). Negative
/// `elapsed` clamps to the launch instant — a shot never flies backwards in
/// time. This is the one motion law the server and clients share.
#[inline]
#[must_use]
pub fn projectile_position(origin: Vec3, velocity: Vec3, elapsed: f32) -> Vec3 {
    origin + velocity * elapsed.max(0.0)
}

/// Distance a projectile with this `velocity` has covered `elapsed` seconds after
/// launch. The scalar odometer that [`projectile_expired`] compares against the
/// range; matches the displacement from the launch origin.
#[inline]
#[must_use]
pub fn projectile_traveled(velocity: Vec3, elapsed: f32) -> f32 {
    velocity.length() * elapsed.max(0.0)
}

/// Whether a projectile with this `velocity` and maximum `range` has reached the
/// end of its flight after `elapsed` seconds. Monotone in `elapsed`: once true it
/// stays true. A non-positive `range` is spent immediately.
#[inline]
#[must_use]
pub fn projectile_expired(velocity: Vec3, range: f32, elapsed: f32) -> bool {
    projectile_traveled(velocity, elapsed) >= range.max(0.0)
}

/// The one-shot "a projectile was launched" event, server→client. Small and
/// fixed-size: the client reconstructs the whole flight from it via the shared
/// motion law ([`projectile_position`]), so no per-tick `Transform` ever crosses
/// the wire for the projectile. Content-free — a straight-line launch, nothing
/// hero- or ability-specific; a mod's client half chooses the mesh/VFX.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectileFired {
    /// Launch position in world space.
    pub origin: Vec3,
    /// Direction × speed, world units/second. Its length is the speed the client
    /// integrates; its direction is the flight path.
    pub velocity: Vec3,
    /// Maximum flight distance before the shot expires, world units. Bounds the
    /// client's local simulation so it despawns the visual on its own.
    pub range: f32,
    /// Cosmetic key: the (global) id of the launching ability, so the client's
    /// cosmetic half can pick the missile's visual (see the client's
    /// `EffectVisuals` / `EffectRole::Projectile`). `0` means "no specific visual"
    /// — the client draws its neutral placeholder. Content-free: an opaque id,
    /// never a hero/ability name.
    pub vfx: u32,
    /// Which shot this is — an opaque id, unique among the shots in flight, echoed
    /// by the [`ImpactEvent`](crate::impact::ImpactEvent) that ends it
    /// (stormlight/server#153).
    ///
    /// The client's missile is a **local** body reconstructed from this event, not
    /// a replicated entity, so an impact had no way to name the drawing it ends.
    /// Without one, a shot could only be retired by flying its whole declared
    /// range — and a shot that hits something well inside that range carried on
    /// through it, which reads as a piercing attack that damages once.
    ///
    /// Never `0`: that value is reserved for an impact no shot produced.
    pub shot: u32,
    /// The unit that fired it, mapped to the receiver's local entity on arrival —
    /// `None` when the shot came from something the receiver cannot see, or from
    /// no unit at all (stormlight/server#154).
    ///
    /// It rides along so a client can ask the shooter's **art** where the shot
    /// leaves from: an attachment point is an animated bone, and only the client
    /// has one. Without the shooter there is nothing to ask, and the drawing falls
    /// back to `origin` — which is what every shot did before sockets existed, so
    /// the fallback is not a degraded mode, it is the old one.
    ///
    /// A one-shot event is the right place for a mapped reference: it is resolved
    /// against the entity map the moment it arrives, unlike a replicated component,
    /// which resolves once and keeps a placeholder forever if it was early.
    pub shooter: Option<Entity>,
    /// The unit it was aimed at, mapped to the receiver's local entity on arrival
    /// — `None` when the shot was aimed at a direction or a point rather than at
    /// anybody (stormlight/server#155).
    ///
    /// The counterpart of `shooter` at the other end of the flight, and it rides
    /// along for the same reason: the point a shot is *drawn arriving on* is a
    /// socket in the target's skeleton, and only the client has one to ask. Without
    /// it the drawing keeps the authoritative velocity and flies parallel to the
    /// real shot, missing the model by however far the muzzle sits from the
    /// shooter's own anchor.
    ///
    /// Set only for a shot genuinely aimed at a unit. A skillshot resolves its
    /// target to the caster, which is a resolution convenience and not something
    /// aimed at, so it carries `None` and is drawn exactly where the server sent
    /// it.
    ///
    /// It changes no hitbox, no range and no contact test: the server's flight is
    /// what it always was, and this only says which rig the drawing may ask about
    /// the far end.
    pub target: Option<Entity>,
}

impl MapEntities for ProjectileFired {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // The origin, the velocity and the two keys are plain values; the two ends
        // of the flight are the only handles. A shot from nobody stays from nobody,
        // and a shot aimed at nobody stays unaimed — inventing either would point
        // the drawing at whatever entity happens to hold that index here.
        if let Some(shooter) = &mut self.shooter {
            *shooter = entity_map.get_mapped(*shooter);
        }
        if let Some(target) = &mut self.target {
            *target = entity_map.get_mapped(*target);
        }
    }
}

/// Reliable channel the one-shot launch events ride. Reliable (not per-tick) so a
/// launch is delivered exactly once and never dropped — a missed shot would leave
/// a client with no visual for a real projectile. One small message per shot is
/// still far cheaper than replicating a mover every tick.
pub struct ProjectileChannel;

/// Register the projectile wire contract on both ends: the reliable channel and
/// the server→client [`ProjectileFired`] event. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<ProjectileChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ServerToClient);

    app.register_event::<ProjectileFired>()
        // The shooter is an entity handle, so the launch has to be mapped into the
        // receiver's own world (stormlight/server#154).
        .add_map_entities()
        .add_direction(NetworkDirection::ServerToClient);
}
