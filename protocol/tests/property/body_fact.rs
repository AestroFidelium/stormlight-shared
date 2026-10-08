//! The replicated fact about a body that stays in the world (stormlight/server#220).
//!
//! A missile reaches a client as its launch; a zone, an item on the ground and a
//! summoned unit stay where they are put, so they reach it as an entity carrying
//! [`BodyFact`]. Invariants:
//!
//! - **The round trip preserves it**: kind, radius and cosmetic key decode as they
//!   were sent, so a client keys the art the server meant;
//! - **A radius is always drawable**: whatever number a payload computed, the fact
//!   carries a finite, non-negative radius — a NaN reaching the client would cost
//!   the frame its zone is drawn in, not the zone;
//! - **It is registered on the protocol**: a body the server tells nobody about is
//!   the invisible fire this issue was filed for.

use bevy::prelude::*;
use bolero::{TypeGenerator, check};
use lightyear::prelude::*;
use stormlight_shared::bodies::{BodyFact, BodyKindFact};
use stormlight_shared::protocol::ProtocolPlugin;

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Kind {
    Summon,
    Zone,
    Pickup,
}

impl Kind {
    fn fact(self) -> BodyKindFact {
        match self {
            Self::Summon => BodyKindFact::Summon,
            Self::Zone => BodyKindFact::Zone,
            Self::Pickup => BodyKindFact::Pickup,
        }
    }
}

/// A radius a payload might have computed, hostile ones included.
#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Radius {
    Hundredths(u16),
    Negative(u16),
    Undefined,
    Infinite,
}

impl Radius {
    fn value(self) -> f32 {
        match self {
            Self::Hundredths(v) => f32::from(v) / 100.0,
            Self::Negative(v) => -f32::from(v) / 100.0 - 0.01,
            Self::Undefined => f32::NAN,
            Self::Infinite => f32::INFINITY,
        }
    }
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    kind: Kind,
    radius: Radius,
    vfx: u32,
}

#[test]
fn a_fact_survives_the_wire() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = BodyFact::new(s.kind.fact(), s.radius.value(), s.vfx);
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("a fact encodes");
        let (back, _): (BodyFact, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("and decodes");
        assert_eq!(back, sent, "a body fact changed on the wire");
    });
}

#[test]
fn a_radius_is_always_drawable() {
    check!().with_type::<Scenario>().for_each(|s| {
        let fact = BodyFact::new(s.kind.fact(), s.radius.value(), s.vfx);
        assert!(fact.radius.is_finite(), "a {:?} radius reached the wire", s.radius);
        assert!(fact.radius >= 0.0, "a negative radius reached the wire");
        if let Radius::Hundredths(v) = s.radius {
            assert_eq!(fact.radius, f32::from(v) / 100.0, "a sound radius was altered");
        }
        assert_eq!(fact.kind, s.kind.fact());
        assert_eq!(fact.vfx, s.vfx);
    });
}

#[test]
fn the_fact_is_registered_for_replication() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(ProtocolPlugin);
    app.finish();

    let component = app
        .world()
        .component_id::<BodyFact>()
        .expect("registering the fact introduces it to the world");
    let registry = app.world().resource::<ComponentRegistry>();
    assert!(
        registry.component_id_to_kind.contains_key(&component),
        "a body fact is not on the protocol, so no client is ever told a zone is there",
    );
}
