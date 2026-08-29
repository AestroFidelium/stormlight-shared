//! Invariants of the replicated swing state
//! ([`stormlight_shared::swing::SwingProgress`]) — the server→client fact that a
//! unit is committed to a basic attack (stormlight/server#152). Structural only:
//!
//! - **Round-trip**: any swing survives a serde encode/decode unchanged and
//!   re-encodes to identical bytes, so what the animation is scaled to on the
//!   client is the window the server actually measured.

use bolero::{TypeGenerator, check};
use stormlight_shared::swing::SwingProgress;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// Raw seeds mapped to a bounded, finite swing so value-equality after a
/// round-trip is meaningful (no NaN). `progress` is kept in its `[0, 1]` domain.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    progress_seed: u16,
    total_seed: u16,
    vfx: u32,
}

fn state(s: &Scenario) -> SwingProgress {
    SwingProgress { progress: frac(s.progress_seed), total: frac(s.total_seed) * 4.0, vfx: s.vfx }
}

#[test]
fn any_swing_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let swing = state(s);
        let bytes = bincode::serde::encode_to_vec(swing, cfg).expect("serialize");
        let (back, _): (SwingProgress, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(swing, back, "a swing did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "a swing's encoding is not stable");
    });
}
