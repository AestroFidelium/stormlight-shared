//! Lightyear registration of the wire format. Both server and client add
//! [`ProtocolPlugin`] during bootstrap so they agree on exactly which
//! components replicate and how they are (de)serialized.
//!
//! Milestone 1 registers a single component: Bevy's [`Transform`]. The
//! `bevy/serialize` feature makes it `Serialize`/`Deserialize`, so server
//! mutations ship to every client that can see the entity — no wrapper
//! components, no hand-written sync systems.
//!
//! Transform is registered **with interpolation**: an entity spawned with an
//! `InterpolationTarget` is mirrored on the client as a smooth `Interpolated`
//! entity that eases between confirmed snapshots via
//! [`crate::connection::lerp_transform`], instead of snapping at the (20 Hz)
//! replication rate. `Transform` doesn't implement `Ease`, so we hand Lightyear
//! the lerp explicitly.

use bevy::prelude::*;
use lightyear::prelude::*;

/// Registers every replicated component shared by server and client. Added on
/// both ends so the protocol matches byte-for-byte.
#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Bevy's Transform replicated directly (enabled by `bevy/serialize`).
        // The `Replicate` / `InterpolationTarget` bundle at spawn time decides,
        // per entity, who receives it and whether it is interpolated.
        app.register_component::<Transform>()
            .add_interpolation_with(crate::connection::lerp_transform);

        // Representative per-unit state components, registered on both ends so
        // the `cube_demo` stress test can measure how replication scales past a
        // lone Transform (stormlight/server#1). Registration is free until an
        // entity carries them; the demo attaches them behind `STRESS_RICH`.
        crate::stress::register(app);

        // Event-driven projectiles (stormlight/server#9): the one-shot launch
        // event + its reliable channel. Server emits, client simulates locally —
        // no per-tick Transform on the wire for a projectile.
        crate::projectiles::register(app);
    }
}
