//! Invariants of [`stormlight_shared::vitals`] — the compact wire encoding of a
//! unit's combat vitals (`stormlight/server#57`).
//!
//! A health bar is a *ratio* first and a number second, so the codec stores the
//! ceiling absolutely and the current value as a fraction of it. That choice is
//! what these pin: the round-trip preserves the fill fraction to far below one
//! pixel of any bar, the decoded state can never claim more health than its
//! maximum (a bar that overflows its track is a visible bug), a unit with no
//! maximum degrades to an empty bar instead of dividing by zero, and every
//! frame — including bit patterns no sane server would send — decodes to finite
//! numbers rather than NaN.

use bolero::{TypeGenerator, check};
use stormlight_shared::vitals::{
    HEALTH_RESOLUTION, RATIO_STEPS, ReplicatedVitals, VITALS_LEN, decode, encode,
};

/// Raw bolero seeds mapped onto bounded finite values, so shrinking lands on
/// numeric edge cases (0, full, empty) instead of NaN/Inf bit patterns.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// Seeds the maximum health.
    max: u16,
    /// Seeds the current health as a fraction of that maximum.
    fill: u16,
    /// Seeds the absorbing shield.
    shield: u16,
}

/// A health ceiling in `[0, 65535]` — spans a chip creep through a fortification,
/// and includes the degenerate `0` the ratio must survive.
fn max_hp(seed: u16) -> f32 {
    f32::from(seed)
}

/// A fill fraction in `[0, 1]`.
fn fraction(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn vitals_of(s: &Scenario) -> ReplicatedVitals {
    let max = max_hp(s.max);
    ReplicatedVitals::new(max * fraction(s.fill), max, f32::from(s.shield))
}

#[test]
fn the_frame_is_fixed_length_and_smaller_than_the_raw_form() {
    // The wire contract: one fixed-size frame per unit, strictly smaller than the
    // three raw `f32`s it replaces. Structural — holds for every generated state.
    check!().with_type::<Scenario>().for_each(|s| {
        let v = vitals_of(s);
        assert_eq!(
            encode(&v).len(),
            VITALS_LEN,
            "encoder must emit a fixed {VITALS_LEN}-byte frame"
        );
        assert!(
            VITALS_LEN < 3 * size_of::<f32>(),
            "quantized frame ({VITALS_LEN} B) must be smaller than the raw f32 triple"
        );
    });
}

#[test]
fn the_fill_fraction_round_trips_within_one_ratio_step() {
    // What a health bar actually draws. The fraction is stored directly, so the
    // error is one quantization step regardless of how large the ceiling is —
    // a 20-hp creep and a 20 000-hp fort fill their bars equally precisely.
    check!().with_type::<Scenario>().for_each(|s| {
        let v = vitals_of(s);
        let back = decode(&encode(&v));
        let err = (back.fraction() - v.fraction()).abs();
        let tol = 1.0 / RATIO_STEPS;
        assert!(
            err <= tol,
            "fill fraction drifted {err} (> one ratio step {tol}) on {v:?} -> {back:?}"
        );
    });
}

#[test]
fn health_never_decodes_above_its_maximum() {
    // Storing health as a fraction makes this structural rather than a clamp we
    // have to remember: there is no representable frame that says "more than
    // full". A bar can therefore never overflow its own track.
    check!().with_type::<Scenario>().for_each(|s| {
        let back = decode(&encode(&vitals_of(s)));
        assert!(
            back.hp <= back.max_hp,
            "decoded {} hp exceeds the decoded maximum {}",
            back.hp,
            back.max_hp
        );
        assert!(back.hp >= 0.0, "decoded hp {} went negative", back.hp);
        assert!(back.shield >= 0.0, "decoded shield {} went negative", back.shield);
    });
}

#[test]
fn a_unit_without_a_maximum_reads_empty_rather_than_dividing_by_zero() {
    // `max_hp == 0` is reachable (an entity that carries vitals before its
    // descriptor's numbers land). The ratio must degrade to a well-defined empty
    // bar, never NaN — one NaN here poisons every layout that multiplies by it.
    check!().with_type::<u16>().for_each(|&fill| {
        let v = ReplicatedVitals::new(f32::from(fill), 0.0, 0.0);
        let back = decode(&encode(&v));
        assert_eq!(v.fraction(), 0.0, "a zero maximum must read as an empty bar");
        assert_eq!(back.fraction(), 0.0, "a zero maximum must survive the wire empty");
        assert!(back.hp.is_finite() && back.max_hp.is_finite());
    });
}

#[test]
fn the_absolute_quantities_round_trip_within_one_fixed_point_step() {
    // The numbers a "1234 / 2000" readout prints, as opposed to the bar's ratio.
    // Bounded by the fixed-point resolution, so a displayed number is never off
    // by a visible amount.
    check!().with_type::<Scenario>().for_each(|s| {
        let v = vitals_of(s);
        let back = decode(&encode(&v));
        let tol = 1.0 / HEALTH_RESOLUTION;
        assert!(
            (back.max_hp - v.max_hp).abs() <= tol,
            "maximum drifted {} (> one step {tol})",
            (back.max_hp - v.max_hp).abs()
        );
        assert!(
            (back.shield - v.shield).abs() <= tol,
            "shield drifted {} (> one step {tol})",
            (back.shield - v.shield).abs()
        );
    });
}

#[test]
fn a_decoded_frame_is_a_stable_fixed_point() {
    // Anti-drift: state that survives one hop must not keep sliding on later
    // hops. The server re-reads its own projection every tick, so a codec that
    // drifted would bleed a unit's health down over a long match.
    check!().with_type::<Scenario>().for_each(|s| {
        let once = encode(&vitals_of(s));
        let twice = encode(&decode(&once));
        assert_eq!(once, twice, "re-encoding a decoded frame must be byte-identical");
    });
}

#[test]
fn every_frame_decodes_to_finite_state() {
    // Totality against a hostile/corrupt wire: any `VITALS_LEN` bytes must decode
    // to usable numbers. The decoder is the trust boundary — it may not panic and
    // may not hand the renderer a NaN.
    check!().with_type::<[u8; VITALS_LEN]>().for_each(|bytes| {
        let back = decode(bytes);
        assert!(back.hp.is_finite(), "hp must be finite for any frame");
        assert!(back.max_hp.is_finite(), "max_hp must be finite for any frame");
        assert!(back.shield.is_finite(), "shield must be finite for any frame");
        assert!(back.fraction().is_finite(), "fraction must be finite for any frame");
        assert!(back.hp <= back.max_hp, "no frame may decode to hp above max");
    });
}

#[test]
fn interpolation_stays_between_its_endpoints() {
    // Bars are interpolated between snapshots (20 Hz replication, 60+ Hz render).
    // The eased value must always sit within the two confirmed states — an
    // overshoot would flash a bar past full or below empty mid-transition.
    #[derive(Debug, TypeGenerator)]
    struct Pair {
        start: Scenario,
        end: Scenario,
        t: u8,
    }
    check!().with_type::<Pair>().for_each(|p| {
        let (start, end) = (vitals_of(&p.start), vitals_of(&p.end));
        let t = f32::from(p.t) / f32::from(u8::MAX);
        let mid = stormlight_shared::vitals::lerp_vitals(start, end, t);

        // Slack scales with the *largest magnitude in play*, which is what bounds
        // an `f32` lerp's rounding error: easing from 6 hp to 35 000 loses the low
        // digits of the small endpoint, and a fixed epsilon would only be
        // measuring float spacing rather than the easing itself.
        let slack = |lo: f32, hi: f32| 1e-4 * lo.abs().max(hi.abs()).max(1.0);
        for (name, a, b, m) in [
            ("hp", start.hp, end.hp, mid.hp),
            ("max_hp", start.max_hp, end.max_hp, mid.max_hp),
            ("shield", start.shield, end.shield, mid.shield),
        ] {
            let (lo, hi) = (a.min(b), a.max(b));
            let tol = slack(lo, hi);
            assert!(
                m >= lo - tol && m <= hi + tol,
                "interpolated {name} {m} escaped its endpoints [{lo}, {hi}]"
            );
        }
        // The invariant the whole encoding exists to protect survives easing too.
        assert!(
            mid.hp <= mid.max_hp + slack(mid.hp, mid.max_hp),
            "interpolation produced hp {} above max {}",
            mid.hp,
            mid.max_hp
        );
    });
}
