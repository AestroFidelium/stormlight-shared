//! Invariants of the replicated cast-indicator state
//! ([`stormlight_shared::cast::CastProgress`]) — the server→client render key for
//! a cast/channel indicator. Structural only:
//!   - **Round-trip**: any `CastProgress` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes (a stable replicated payload).

use bolero::{TypeGenerator, check};
use stormlight_shared::cast::CastProgress;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// Raw seeds mapped to a bounded, finite `CastProgress` so value-equality after a
/// round-trip is meaningful (no NaN). `progress` is kept in its `[0, 1]` domain.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    slot: u8,
    progress_seed: u16,
    total_seed: u16,
    vfx: u32,
}

fn state(s: &Scenario) -> CastProgress {
    CastProgress {
        slot: s.slot,
        progress: frac(s.progress_seed),
        total: frac(s.total_seed) * 10.0,
        vfx: s.vfx,
    }
}

#[test]
fn any_cast_progress_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let cp = state(s);
        let bytes = bincode::serde::encode_to_vec(cp, cfg).expect("serialize");
        let (back, _): (CastProgress, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(cp, back, "cast progress did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "cast progress encoding is not stable");
    });
}
