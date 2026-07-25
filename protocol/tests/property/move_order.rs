//! Invariant of the move-order wire contract ([`stormlight_shared::movement`]) —
//! the client→server "walk here" command. Structural only:
//!   - **Round-trip**: any `MoveOrder` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes (a stable wire payload).
//!
//! A bare point, so (unlike `CastIntent`) there is no entity mapping to exercise.

use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::movement::MoveOrder;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// A bounded, finite world coordinate so value-equality after a round-trip is
/// meaningful (no NaN) and shrinking lands on clean values.
fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 400.0
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    v: (u16, u16, u16),
}

fn order(s: &Scenario) -> MoveOrder {
    MoveOrder { target: Vec3::new(coord(s.v.0), coord(s.v.1), coord(s.v.2)) }
}

#[test]
fn any_move_order_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let mo = order(s);
        let bytes = bincode::serde::encode_to_vec(mo, cfg).expect("serialize");
        let (back, _): (MoveOrder, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(mo, back, "move order did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "move order encoding is not stable");
    });
}
