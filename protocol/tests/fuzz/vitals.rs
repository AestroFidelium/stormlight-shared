//! Fuzz the decode side of the vitals + pools wire format
//! (`stormlight/server#57`). A hostile or corrupt peer can hand these decoders
//! any bytes at all, and what they produce goes straight into UI arithmetic —
//! one NaN fill fraction poisons every layout that multiplies by it, and a
//! `current > max` would draw a bar past the end of its own track.
//!
//! The pool decoder is the riskier of the two: its length is attacker-controlled
//! (a count byte followed by that many entries), so a frame that claims more
//! pools than it carries must yield only the entries that are actually there
//! rather than over-reading.
//!
//!   cargo bolero test vitals::decoding_arbitrary_frames_stays_sane -T 600sec
//!   cargo bolero test vitals::decoding_arbitrary_pool_frames_stays_sane -T 600sec

use bolero::{TypeGenerator, check};
use stormlight_shared::pools::{POOL_ENTRY_LEN, decode as decode_pools};
use stormlight_shared::vitals::{VITALS_LEN, decode as decode_vitals};

#[derive(Debug, TypeGenerator)]
struct Frame([u8; VITALS_LEN]);

#[test]
fn decoding_arbitrary_frames_stays_sane() {
    check!().with_type::<Frame>().for_each(|Frame(bytes)| {
        let v = decode_vitals(bytes);
        assert!(
            v.hp.is_finite() && v.max_hp.is_finite() && v.shield.is_finite(),
            "every field must be finite for any input frame"
        );
        assert!(v.hp >= 0.0 && v.max_hp >= 0.0 && v.shield >= 0.0, "no field may go negative");
        assert!(v.hp <= v.max_hp, "no frame may decode to health above its maximum");
        assert!(
            (0.0..=1.0).contains(&v.fraction()),
            "the fill fraction must stay in [0, 1] for any input frame"
        );
    });
}

#[test]
fn decoding_arbitrary_pool_frames_stays_sane() {
    check!().with_type::<Vec<u8>>().for_each(|bytes| {
        let pools = decode_pools(bytes);
        assert!(
            pools.0.len() <= bytes.len().saturating_sub(1) / POOL_ENTRY_LEN,
            "the count byte must never be trusted past the bytes that follow it"
        );
        for pool in &pools.0 {
            assert!(pool.current.is_finite() && pool.max.is_finite());
            assert!(pool.current >= 0.0 && pool.max >= 0.0);
            assert!(pool.current <= pool.max, "no frame may decode a balance above its ceiling");
            assert!((0.0..=1.0).contains(&pool.fraction()));
        }
    });
}
