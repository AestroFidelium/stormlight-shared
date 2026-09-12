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
            .add_should_rollback(crate::movement::transform_should_rollback)
            // …and when it *does* roll back, ease the correction out instead of
            // teleporting the hero onto it (stormlight/server#65). A rollback is a
            // statement about the past; applying it to the present as a jump is what
            // a player reads as their character "snapping". The error between where
            // the hero was drawn and where it should have been decays over a couple
            // of hundred milliseconds, so a divergence the client cannot predict —
            // the server's local avoidance steering it around a body, say — is
            // absorbed as a lean rather than a lurch. Inert on the server, which
            // never predicts anything.
            .add_linear_correction_fn::<Isometry3d>();

        // Representative per-unit state components, registered on both ends so
        // the `cube_demo` stress test can measure how replication scales past a
        // lone Transform (stormlight/server#1). Registration is free until an
        // entity carries them; the demo attaches them behind `STRESS_RICH`.
        crate::stress::register(app);

        // Replicated unit identity (stormlight/server#41): the one generic key a
        // client uses to resolve a replicated entity to the visual a cosmetic mod
        // declared for its unit descriptor. An opaque global id, never a name.
        crate::identity::register(app);

        // The replicated alive/dead state (stormlight/server#61): whether a unit
        // has been killed and not yet come back, so a client stops drawing a
        // live unit as live.
        crate::death::register(app);

        // Event-driven projectiles (stormlight/server#9): the one-shot launch
        // event + its reliable channel. Server emits, client simulates locally —
        // no per-tick Transform on the wire for a projectile.
        crate::projectiles::register(app);

        // Cast intents (stormlight/server#30): the one player command on the
        // wire. Client requests, server validates + resolves authoritatively.
        // Also registers the replicated `CastProgress` cast-indicator state.
        crate::cast::register(app);

        // The swing (stormlight/server#152): present exactly while a unit is
        // committed to a basic attack, carrying the window an animation is scaled
        // to and the cosmetic key its shot is dressed by. The attack's half of
        // what `CastProgress` does for a cast.
        crate::swing::register(app);

        // Authoritative impact feedback (stormlight/server#36): the one-shot
        // "a shot landed here" event + its reliable channel. Server announces the
        // hit; each client plays a transient impact visual there.
        crate::impact::register(app);

        // What a blow actually cost (stormlight/server#93): the coalesced
        // server→client report a mod's HUD prints as floating combat text. The
        // impact above says a shot landed; this says how much it took, from
        // whatever dealt it — including everything that fires no projectile at all.
        crate::vital_feed::register(app);

        // Combat vitals + resource pools (stormlight/server#57): the health/shield
        // and wallet state a client draws bars from. Quantized ceiling-and-fraction
        // frames, interpolated so the bars slide rather than step at the
        // replication rate, and public — visibility is decided once, by the
        // entity-level area-of-interest culling (see `pools`).
        crate::vitals::register(app);
        crate::pools::register(app);

        // The owner's ability slots (stormlight/server#58): what each key binds
        // to, when it comes off cooldown, and whether it is castable right now.
        // Privileged per-player state — it rides its own entity, replicated to
        // the controlling player alone, so no opponent reads your cooldowns.
        crate::slots::register(app);

        // The match roster (stormlight/server#145): who is playing, on which side,
        // and the unit each of them drives. The one replicated fact that is not
        // about a body — it outlives one, and it reaches every client whether or
        // not the body it describes is inside their interest area.
        crate::roster::register(app);

        // Progression (stormlight/server#62): a unit's level, public because it
        // changes how you play against it, and the owner's own XP progress,
        // privileged because knowing when someone is about to level is knowing
        // when to contest them. The two ride different entities for that reason.
        crate::progression::register(app);

        // Talent picking (stormlight/server#63): the player's choice of talent
        // within a tier, and the owner-scoped view of which tiers are open, what
        // has been taken, and what is still waiting on them.
        crate::talents::register(app);

        // The owner's stack counters (stormlight/server#132): the running count
        // behind a quest, and every other "how many times has this happened"
        // reserve a mod keeps. Privileged for the reason its neighbours on that
        // entity are — three casts from a payout is exactly when not to be
        // contested.
        crate::stacks::register(app);
        crate::tasks::register(app);

        // Interface triggers (stormlight/server#69): a declared widget raising a
        // mod-defined event into that mod's own gameplay guest. The interface's
        // other two actions need no message of their own — a clicked ability slot
        // sends the very `CastIntent` its keybind does.
        crate::ui::register(app);

        // The mover's replicated stats (stormlight/server#47/#50): what a
        // predicting client needs to advance the unit it controls at the
        // authoritative speed.
        crate::movement::register(app);

        // Player orders (stormlight/server#90): the one client→server command
        // saying what a unit should do next, and whether it replaces the plan or
        // is appended to it. Ordered-reliable, because a plan is a sequence.
        crate::orders::register(app);
    }
}
