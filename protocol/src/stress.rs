//! Provisional, generic per-unit state components used **only** to measure how
//! replication cost scales with the number of components on a unit
//! (`stormlight/server#1`). The bare stress test replicates one `Transform`;
//! a real unit will carry health, mana, shields, cooldown timers, … — up to a
//! few dozen components. This registers a representative handful of distinct
//! small components so the measurement captures Lightyear's **per-component**
//! framing overhead (net id + change detection), not just raw payload bytes.
//!
//! These are throwaway measurement stand-ins, not the real schema — the actual
//! generic unit schema lands later in the server's `components.rs`. They live
//! here because both server and client must register the identical component set
//! for the protocol to match; they cost nothing until an entity actually carries
//! them (the `cube_demo` example attaches them behind `STRESS_RICH`).

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Declare a one-`f32` generic state component (a vital or a cooldown timer).
macro_rules! vital {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        pub struct $name(pub f32);
    };
}

vital!(/// Current hit points.
    Health);
vital!(/// Current mana / resource pool.
    Mana);
vital!(/// Absorb shield remaining.
    Shield);
vital!(/// Movement/attack stamina.
    Stamina);
vital!(/// Ability-slot 0 cooldown remaining (a per-tick-ticking timer — the
    /// worst case for naive replication).
    Cooldown0);
vital!(/// Ability-slot 1 cooldown remaining.
    Cooldown1);
vital!(/// Ability-slot 2 cooldown remaining.
    Cooldown2);
vital!(/// Ability-slot 3 cooldown remaining.
    Cooldown3);

/// How many distinct representative components this module registers. The demo
/// extrapolates from this to a full ~30-component unit.
pub const STRESS_COMPONENT_COUNT: usize = 8;

/// Register every representative component for replication. Called from
/// [`crate::protocol::ProtocolPlugin`] on both ends so the protocol matches.
pub fn register(app: &mut App) {
    app.register_component::<Health>();
    app.register_component::<Mana>();
    app.register_component::<Shield>();
    app.register_component::<Stamina>();
    app.register_component::<Cooldown0>();
    app.register_component::<Cooldown1>();
    app.register_component::<Cooldown2>();
    app.register_component::<Cooldown3>();
}
