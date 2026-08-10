//! A unit that declares no turn rate still turns at a *rate* (stormlight/server#65).
//!
//! The mover law has always integrated facing toward travel at a `TurnRate`, but
//! nothing ever declared one, and "absent" meant **instant**. Facing was therefore
//! a step function of the direction of travel: a corridor's corners, and every
//! re-aim of a straight leg, landed on the model as a snap — the heading jumping
//! between a handful of discrete values while the body slid smoothly underneath it.
//! It reads as a character that can only stand at certain angles.
//!
//! The fallback is now a finite rate, so heading is a curve like position is. What
//! is pinned here is the fallback itself, not a number: it is finite and positive
//! (a heading that must be *travelled* to), a declared rate always wins over it,
//! and a nonsense declaration degrades instead of spinning the unit backwards.

use core::f32::consts::PI;

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use stormlight_shared::movement::{
    DEFAULT_TURN_RATE, TurnRate, angle_delta, face_travel, turn_rate_of, turn_toward, wrap_angle,
    yaw_to,
};

#[derive(Debug, TypeGenerator)]
struct Scenario {
    facing: u16,
    tx: u16,
    tz: u16,
    declared: u16,
    dt: u8,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn angle(seed: u16) -> f32 {
    (frac(seed) - 0.5) * (4.0 * PI) // [-τ, τ]
}

fn dir(x: u16, z: u16) -> Vec2 {
    Vec2::new((frac(x) - 0.5) * 20.0, (frac(z) - 0.5) * 20.0)
}

/// A tick's worth of time, at most — the scale the fallback is actually used at.
fn dt(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX) * (1.0 / 32.0)
}

#[test]
fn the_fallback_is_a_rate_and_not_a_snap() {
    check!().with_type::<Scenario>().for_each(|s| {
        // The whole point, stated through the law rather than through the literal:
        // asked for the heading furthest from the one it holds, a tick of turning at
        // the fallback rate moves the unit (the rate is not zero — it could never
        // look where it walks) and does not arrive (the rate is not infinite — the
        // old "instant", which is what made heading a step function).
        let facing = angle(s.facing);
        let opposite = wrap_angle(facing + PI);
        let after = turn_toward(facing, opposite, turn_rate_of(None), 1.0 / 64.0);
        assert!(angle_delta(facing, after).abs() > 0.0, "the fallback never turns the unit");
        assert!(
            angle_delta(after, opposite).abs() > 0.0,
            "the fallback crossed a half turn in a single tick — that is a snap",
        );
    });
}

#[test]
fn a_declared_rate_always_wins_over_the_fallback() {
    check!().with_type::<Scenario>().for_each(|s| {
        let declared = frac(s.declared) * 40.0;
        assert_eq!(
            turn_rate_of(Some(&TurnRate(declared))),
            declared,
            "a unit that declared its own turn rate was turned at another one",
        );
        assert_eq!(
            turn_rate_of(None),
            DEFAULT_TURN_RATE,
            "a unit that declared nothing was not given the engine's fallback",
        );
    });
}

#[test]
fn a_nonsense_rate_degrades_instead_of_reversing_the_turn() {
    check!().with_type::<Scenario>().for_each(|s| {
        // A negative rate is a sign error in content, not a request to turn the
        // other way: it clamps to "does not turn", and the mover holds its heading.
        let rate = turn_rate_of(Some(&TurnRate(-frac(s.declared) * 40.0)));
        assert!(rate >= 0.0, "a negative declared rate stayed negative");
        let facing = angle(s.facing);
        let held = turn_toward(facing, angle(s.tx), rate, dt(s.dt));
        assert!(
            angle_delta(facing, held).abs() <= 1e-4,
            "a unit with a clamped-away turn rate turned anyway",
        );
    });
}

#[test]
fn the_fallback_heading_is_travelled_to_rather_than_jumped_to() {
    check!().with_type::<Scenario>().for_each(|s| {
        let facing = angle(s.facing);
        let travel = dir(s.tx, s.tz);
        if travel.length() < 1.0 {
            return;
        }
        let dt = dt(s.dt);
        let rate = turn_rate_of(None);
        let turned = face_travel(facing, travel, rate, dt);

        // One tick moves the heading by at most a tick's worth of turning …
        let applied = angle_delta(facing, turned).abs();
        assert!(applied <= rate * dt + 1e-4, "the fallback turned {applied} in a single tick");
        // … and always toward where the unit actually went, never away from it.
        let want = angle_delta(facing, yaw_to(travel));
        let got = angle_delta(facing, turned);
        assert!(
            got.abs() <= want.abs() + 1e-4 && (got == 0.0 || got.signum() == want.signum()),
            "the fallback turned away from the direction of travel",
        );
    });
}

#[test]
fn the_fallback_still_settles_onto_the_travel_direction() {
    check!().with_type::<Scenario>().for_each(|s| {
        // Finite must not mean sluggish: holding a heading for a bounded stretch of
        // ticks gets the unit all the way onto it, so facing lags travel by a turn,
        // never forever. The bound is the half-turn a fallback rate must cover.
        let mut facing = angle(s.facing);
        let travel = dir(s.tx, s.tz);
        if travel.length() < 1.0 {
            return;
        }
        let dt = 1.0 / 64.0;
        let ticks = (PI / (DEFAULT_TURN_RATE * dt)).ceil() as u32 + 1;
        for _ in 0..ticks {
            facing = face_travel(facing, travel, turn_rate_of(None), dt);
        }
        assert!(
            angle_delta(facing, yaw_to(travel)).abs() < 1e-3,
            "the fallback rate never got the unit onto its direction of travel",
        );
    });
}
