//! Lightyear registration of the wire format. Both server and client add
//! [`ProtocolPlugin`] during bootstrap so they agree on exactly which
//! components replicate and how they are (de)serialized.
//!
//! The one per-tick component is Bevy's [`Transform`], registered with a custom
//! quantized codec ([`crate::quantize`]) so a mover's position/rotation/scale
//! ships as a compact 22-byte frame rather than the raw `f32×10`
//! (stormlight/server#10). Server mutations ship to every client that can see
//! the entity — no wrapper components, no hand-written sync systems.
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
        // Transform replicated through the compact quantized codec
        // (stormlight/server#10) instead of the raw `bevy/serialize` bincode
        // form: a fixed 22-byte frame per mover per tick instead of ~40. The
        // `Replicate` / `InterpolationTarget` bundle at spawn time still decides,
        // per entity, who receives it and whether it is interpolated.
        app.register_component_custom_serde::<Transform>(crate::quantize::serialize_fns())
            .add_interpolation_with(crate::connection::lerp_transform)
            // Client-side prediction of the controlled unit (stormlight/server#50):
            // the owner's client rolls its predicted Transform back onto the
            // authoritative one only when they diverge past a small epsilon, so a
            // smoothly predicted unit is not yanked by float noise every snapshot.
            .add_prediction()
            .add_should_rollback(crate::movement::transform_should_rollback);

        // Representative per-unit state components, registered on both ends so
        // the `cube_demo` stress test can measure how replication scales past a
        // lone Transform (stormlight/server#1). Registration is free until an
        // entity carries them; the demo attaches them behind `STRESS_RICH`.
        crate::stress::register(app);

        // Replicated unit identity (stormlight/server#41): the one generic key a
        // client uses to resolve a replicated entity to the visual a cosmetic mod
        // declared for its unit descriptor. An opaque global id, never a name.
        crate::identity::register(app);

        // Event-driven projectiles (stormlight/server#9): the one-shot launch
        // event + its reliable channel. Server emits, client simulates locally —
        // no per-tick Transform on the wire for a projectile.
        crate::projectiles::register(app);

        // Cast intents (stormlight/server#30): the one player command on the
        // wire. Client requests, server validates + resolves authoritatively.
        // Also registers the replicated `CastProgress` cast-indicator state.
        crate::cast::register(app);

        // Authoritative impact feedback (stormlight/server#36): the one-shot
        // "a shot landed here" event + its reliable channel. Server announces the
        // hit; each client plays a transient impact visual there.
        crate::impact::register(app);

        // Combat vitals + resource pools (stormlight/server#57): the health/shield
        // and wallet state a client draws bars from. Quantized ceiling-and-fraction
        // frames, interpolated so the bars slide rather than step at the
        // replication rate, and public — visibility is decided once, by the
        // entity-level area-of-interest culling (see `pools`).
        crate::vitals::register(app);
        crate::pools::register(app);

        // Player move orders (stormlight/server#47): a client asks its controlled
        // unit to walk to a ground point; the server moves it authoritatively and
        // replicates the Transform like any mover.
        crate::movement::register(app);
    }
}
