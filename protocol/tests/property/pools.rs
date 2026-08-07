//! Invariants of [`stormlight_shared::pools`] — the replicated view of a unit's
//! generic resource pools (`stormlight/server#57`).
//!
//! A pool is `(opaque id → current/max)`; the engine never learns what the id
//! *means*. Unlike vitals, a unit may have any number of pools including none,
//! so the frame is variable-length and its length must be derivable from its own
//! bytes. These pin that: the pool list survives the wire in order and identity,
//! a unit with no pools costs an (almost) empty frame and decodes back to no
//! pools at all — never a phantom zero-bar — and no byte sequence can make the
//! decoder panic, over-read, or emit a balance above its ceiling.

use bolero::{TypeGenerator, check};
use stormlight_shared::pools::{
    MAX_POOLS, POOL_ENTRY_LEN, PoolState, ReplicatedPools, decode, encode, lerp_pools,
};
use stormlight_shared::vitals::{HEALTH_RESOLUTION, RATIO_STEPS};

/// Raw seeds for one pool, mapped onto bounded finite values.
#[derive(Debug, TypeGenerator)]
struct PoolSeed {
    id: u32,
    max: u16,
    fill: u16,
}

/// Up to a handful of pools — the realistic shape (a unit has one or two), with
/// the empty case reachable so the "no phantom bars" invariant gets exercised.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    pools: Vec<PoolSeed>,
}

fn pool_of(seed: &PoolSeed) -> PoolState {
    let max = f32::from(seed.max);
    let fill = f32::from(seed.fill) / f32::from(u16::MAX);
    PoolState::new(seed.id, max * fill, max)
}

fn pools_of(s: &Scenario) -> ReplicatedPools {
    ReplicatedPools(s.pools.iter().map(pool_of).collect())
}

#[test]
fn the_frame_length_follows_from_the_pool_count() {
    // Self-describing framing: a reader must be able to size the frame from the
    // bytes alone, since a unit's pool count is content-defined and unknown to
    // the protocol.
    check!().with_type::<Scenario>().for_each(|s| {
        let pools = pools_of(s);
        let frame = encode(&pools);
        let kept = pools.0.len().min(MAX_POOLS);
        assert_eq!(
            frame.len(),
            1 + kept * POOL_ENTRY_LEN,
            "frame must be a count byte plus one fixed entry per replicated pool"
        );
    });
}

#[test]
fn emptiness_round_trips_exactly() {
    // No phantom zero-bars: the absence of pools must survive as an absence, not
    // as one empty pool — a UI that draws a bar per replicated pool would
    // otherwise show an orb on every unit that has none. And the converse: a unit
    // that *does* have pools never arrives with them silently dropped.
    check!().with_type::<Scenario>().for_each(|s| {
        let pools = pools_of(s);
        let back = decode(&encode(&pools));
        assert_eq!(
            back.0.is_empty(),
            pools.0.is_empty(),
            "whether a unit has pools at all must survive the wire exactly"
        );

        let empty = ReplicatedPools::default();
        assert!(decode(&encode(&empty)).0.is_empty(), "an empty list must decode back to empty");
        assert_eq!(encode(&empty).len(), 1, "an empty list must cost only its count byte");
    });
}

#[test]
fn pool_identity_and_order_survive_the_wire() {
    // The id is the only handle a client has to tell one pool from another (they
    // are opaque to the engine), and the order is how a UI keeps a unit's bars
    // from swapping places between frames.
    check!().with_type::<Scenario>().for_each(|s| {
        let pools = pools_of(s);
        let back = decode(&encode(&pools));
        let kept = pools.0.len().min(MAX_POOLS);
        assert_eq!(back.0.len(), kept, "pool count must survive the wire");
        for (before, after) in pools.0.iter().take(kept).zip(back.0.iter()) {
            assert_eq!(before.id, after.id, "pool id must survive the wire exactly");
        }
    });
}

#[test]
fn each_pool_round_trips_its_fill_and_never_exceeds_its_ceiling() {
    check!().with_type::<Scenario>().for_each(|s| {
        let pools = pools_of(s);
        let back = decode(&encode(&pools));
        for (before, after) in pools.0.iter().zip(back.0.iter()) {
            let err = (after.fraction() - before.fraction()).abs();
            assert!(
                err <= 1.0 / RATIO_STEPS,
                "pool {} fill drifted {err} (> one ratio step)",
                before.id
            );
            assert!(
                (after.max - before.max).abs() <= 1.0 / HEALTH_RESOLUTION,
                "pool {} ceiling drifted past one fixed-point step",
                before.id
            );
            assert!(
                after.current <= after.max,
                "pool {} decoded above its ceiling ({} > {})",
                before.id,
                after.current,
                after.max
            );
        }
    });
}

#[test]
fn every_byte_sequence_decodes_without_panicking() {
    // Totality at the trust boundary. The count byte is attacker-controlled, so a
    // frame claiming more pools than it carries must yield only the entries that
    // are actually there rather than over-reading or panicking.
    check!().with_type::<Vec<u8>>().for_each(|bytes| {
        let back = decode(bytes);
        let available = bytes.len().saturating_sub(1) / POOL_ENTRY_LEN;
        assert!(
            back.0.len() <= available,
            "decoded {} pools from a frame that carries at most {available}",
            back.0.len()
        );
        for pool in &back.0 {
            assert!(pool.current.is_finite() && pool.max.is_finite());
            assert!(pool.current <= pool.max, "no frame may decode a pool above its ceiling");
            assert!(pool.current >= 0.0 && pool.max >= 0.0);
            assert!(pool.fraction().is_finite());
        }
    });
}

#[test]
fn interpolation_keeps_the_incoming_pool_set() {
    // Easing must never invent or drop a pool: what the newest snapshot says the
    // unit has is what gets drawn. A pool the previous snapshot did not carry
    // simply appears at its confirmed value instead of easing up from nothing.
    #[derive(Debug, TypeGenerator)]
    struct Pair {
        start: Scenario,
        end: Scenario,
        t: u8,
    }
    check!().with_type::<Pair>().for_each(|p| {
        let (start, end) = (pools_of(&p.start), pools_of(&p.end));
        let t = f32::from(p.t) / f32::from(u8::MAX);
        let mid = lerp_pools(start.clone(), end.clone(), t);

        assert_eq!(mid.0.len(), end.0.len(), "interpolation must keep the incoming pool set");
        for (eased, target) in mid.0.iter().zip(end.0.iter()) {
            assert_eq!(eased.id, target.id, "interpolation must not re-key a pool");
            let source = start.0.iter().find(|p| p.id == target.id);
            match source {
                // Present in both: the eased value sits between the two.
                Some(from) => {
                    let (lo, hi) =
                        (from.current.min(target.current), from.current.max(target.current));
                    // Slack scales with the largest magnitude in play — that is
                    // what bounds an `f32` lerp's rounding error.
                    let tol = 1e-4 * lo.abs().max(hi.abs()).max(1.0);
                    assert!(
                        eased.current >= lo - tol && eased.current <= hi + tol,
                        "eased pool {} escaped its endpoints [{lo}, {hi}]",
                        target.id
                    );
                }
                // New this snapshot: shown as confirmed, not eased from zero.
                None => assert_eq!(
                    eased.current, target.current,
                    "a newly appearing pool must show its confirmed value"
                ),
            }
        }
    });
}
