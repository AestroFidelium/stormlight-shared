//! Invariants of the shared kinematic law (stormlight/server#49): `step_toward`,
//! `turn_toward`, and the combined `advance_mover` the authoritative server and
//! the predicting client (server#50) both integrate. Directional/structural only
//! — bounded steps, shortest-arc turns, clean arrival — never magnitude-on-a-
//! constant. Because both ends run this exact code, a predicted unit and its
//! server ghost can only diverge by float noise.

use core::f32::consts::{PI, TAU};

use bevy::math::{Quat, Vec2, Vec3};
use bolero::{TypeGenerator, check};
use stormlight_shared::movement::{
    advance_mover, angle_delta, face_travel, step_toward, turn_toward, wrap_angle, yaw_to,
};

#[derive(Debug, TypeGenerator)]
struct Kin {
    px: u16,
    pz: u16,
    gx: u16,
    gz: u16,
    speed: u16,
    dt: u8,
    facing: u16,
    target: u16,
    rate: u16,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0 // [-100, 100]
}

/// A few turns either side of zero, so wrapping is exercised, not avoided.
fn angle(seed: u16) -> f32 {
    (frac(seed) - 0.5) * (4.0 * TAU) // ~[-2τ, 2τ]
}

struct S {
    pos: Vec2,
    goal: Vec2,
    speed: f32,
    dt: f32,
    facing: f32,
    target: f32,
    rate: f32,
}

fn build(k: &Kin) -> S {
    S {
        pos: Vec2::new(coord(k.px), coord(k.pz)),
        goal: Vec2::new(coord(k.gx), coord(k.gz)),
        speed: frac(k.speed) * 20.0,              // [0, 20] units/s
        dt: f32::from(k.dt) / f32::from(u8::MAX), // [0, 1] s
        facing: angle(k.facing),
        target: angle(k.target),
        rate: frac(k.rate) * 12.0, // [0, 12] rad/s
    }
}

// --- step_toward ----------------------------------------------------------

#[test]
fn step_never_overshoots_budget() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let next = step_toward(s.pos, s.goal, s.speed, s.dt);
        let moved = (next - s.pos).length();
        assert!(
            moved <= s.speed * s.dt + 1e-3 || next == s.goal,
            "moved {moved} beyond the step budget without arriving",
        );
    });
}

#[test]
fn step_never_recedes_from_goal() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let next = step_toward(s.pos, s.goal, s.speed, s.dt);
        assert!(
            (next - s.goal).length() <= (s.pos - s.goal).length() + 1e-3,
            "a step moved away from the goal",
        );
    });
}

#[test]
fn step_snaps_within_reach() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        if (s.goal - s.pos).length() <= s.speed * s.dt {
            assert_eq!(step_toward(s.pos, s.goal, s.speed, s.dt), s.goal, "did not snap onto goal");
        }
    });
}

// --- wrap_angle / turn_toward --------------------------------------------

#[test]
fn wrap_lands_in_canonical_range() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let w = wrap_angle(s.facing);
        assert!(w > -PI - 1e-4 && w <= PI + 1e-4, "wrap {w} escaped (-PI, PI]");
        // A pure representative: it differs from the input by a whole number of turns.
        let turns = (s.facing - w) / TAU;
        assert!((turns - turns.round()).abs() < 1e-3, "wrap shifted by a fractional turn");
    });
}

#[test]
fn turn_is_bounded_by_rate() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let out = turn_toward(s.facing, s.target, s.rate, s.dt);
        let applied = angle_delta(s.facing, out).abs();
        assert!(applied <= s.rate * s.dt + 1e-3, "turned {applied} beyond the rate budget");
    });
}

#[test]
fn turn_takes_the_shortest_arc() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let out = turn_toward(s.facing, s.target, s.rate, s.dt);
        let before = angle_delta(s.facing, s.target).abs();
        let after = angle_delta(out, s.target).abs();
        assert!(after <= before + 1e-3, "turned away from target: {after} > {before}");
    });
}

#[test]
fn turn_settles_within_reach_and_rests() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        if angle_delta(s.facing, s.target).abs() <= s.rate * s.dt {
            let out = turn_toward(s.facing, s.target, s.rate, s.dt);
            assert!(angle_delta(out, s.target).abs() < 1e-3, "did not settle onto target");
            // Idempotent at rest: re-stepping from the target stays on it.
            let rest = turn_toward(out, s.target, s.rate, s.dt);
            assert!(angle_delta(rest, s.target).abs() < 1e-3, "drifted off target at rest");
        }
    });
}

// --- advance_mover --------------------------------------------------------

#[test]
fn advance_matches_step_and_arrival_flag() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        let m = advance_mover(s.pos, s.facing, s.goal, s.speed, s.rate, s.dt);
        assert_eq!(
            m.pos,
            step_toward(s.pos, s.goal, s.speed, s.dt),
            "advance position diverged from step_toward",
        );
        assert_eq!(m.arrived, m.pos == s.goal, "arrived flag disagrees with reaching the goal");
        assert!(m.facing > -PI - 1e-4 && m.facing <= PI + 1e-4, "facing escaped (-PI, PI]");
    });
}

#[test]
fn advance_faces_travel_when_turn_is_instant() {
    check!().with_type::<Kin>().for_each(|k| {
        let s = build(k);
        // An "instant" turn rate must align facing with the direction of travel.
        let m = advance_mover(s.pos, s.facing, s.goal, s.speed, f32::INFINITY, s.dt);
        let travel = m.pos - s.pos;
        if travel.length() > 1e-2 {
            assert!(
                angle_delta(m.facing, yaw_to(travel)).abs() < 1e-3,
                "instant facing did not align with travel",
            );
        }
    });
}

#[test]
fn yaw_orients_forward_along_travel() {
    check!().with_type::<Kin>().for_each(|k| {
        // The facing convention: rotating the engine's **forward** by `yaw_to(d)`
        // about `+Y` yields a world direction whose ground projection points along
        // `d`. This is what makes a unit written `Transform.rotation =
        // from_rotation_y(facing)` look where it walks — pinned here in the law,
        // independent of any ECS timing.
        //
        // Forward is `-Z`, which is Bevy's own (`Transform::forward`, and what
        // `looking_at` points at a target). It used to be `+X`, and every model
        // authored the ordinary way therefore ran a quarter turn off — the hero
        // walked sideways across the field (stormlight/server#65). The engine
        // agreeing with the engine it is built on is what makes standard art work
        // without a per-model correction.
        let d = Vec2::new(coord(k.gx) - coord(k.px), coord(k.gz) - coord(k.pz));
        if d.length() < 1.0 {
            return;
        }
        let fwd = Quat::from_rotation_y(yaw_to(d)) * Vec3::NEG_Z;
        let fwd_ground = Vec2::new(fwd.x, fwd.z).normalize_or_zero();
        assert!(
            fwd_ground.dot(d.normalize()) > 0.999,
            "yaw_to did not orient forward along the travel direction (dot {})",
            fwd_ground.dot(d.normalize()),
        );
    });
}

#[test]
fn facing_follows_realized_travel_and_a_still_mover_keeps_its_heading() {
    check!().with_type::<Kin>().for_each(|k| {
        // `face_travel` is the facing half of the law, split out so a mover whose
        // realized travel was steered away from its goal (local avoidance,
        // server#51) still faces where it actually went.
        let facing = angle(k.facing);
        let travel = Vec2::new(coord(k.gx) - coord(k.px), coord(k.gz) - coord(k.pz));
        let rate = frac(k.rate) * 12.0;
        let dt = f32::from(k.dt) / 255.0 * 0.1;

        // Standing still keeps the heading, exactly — no snap to a default.
        assert_eq!(
            face_travel(facing, Vec2::ZERO, rate, dt),
            wrap_angle(facing),
            "a mover that did not travel changed its heading",
        );

        let turned = face_travel(facing, travel, rate, dt);
        assert!(
            angle_delta(facing, turned).abs() <= rate * dt + 1e-4,
            "facing turned faster than the mover's turn rate",
        );
        if travel.length() > 1e-2 {
            // The turn is toward the travel direction, never away from it.
            let want = angle_delta(facing, yaw_to(travel));
            let got = angle_delta(facing, turned);
            assert!(
                got.abs() <= want.abs() + 1e-4 && (got == 0.0 || got.signum() == want.signum()),
                "facing did not turn toward the direction of travel",
            );
        }
    });
}
