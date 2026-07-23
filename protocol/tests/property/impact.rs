//! Invariants of the impact wire contract ([`stormlight_shared::impact`]) — the
//! one-shot "a shot landed here" event, server→client. Structural only:
//!   - **Round-trip**: any `ImpactEvent` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes (a stable wire payload).

use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::impact::ImpactEvent;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

/// Raw seeds mapped to a bounded, finite `ImpactEvent` so value-equality after a
/// round-trip is meaningful (no NaN).
#[derive(Debug, TypeGenerator)]
struct Scenario {
    at: (u16, u16, u16),
    vfx: u32,
}

fn event(s: &Scenario) -> ImpactEvent {
    ImpactEvent { at: Vec3::new(coord(s.at.0), coord(s.at.1), coord(s.at.2)), vfx: s.vfx }
}

#[test]
fn any_impact_event_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let ev = event(s);
        let bytes = bincode::serde::encode_to_vec(ev, cfg).expect("serialize");
        let (back, _): (ImpactEvent, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(ev, back, "impact event did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "impact event encoding is not stable");
    });
}
