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
use bevy::prelude::*;
use bolero::{TypeGenerator, check};
use lightyear::prelude::ComponentRegistry;
use stormlight_shared::movement::{
    DEFAULT_TURN_RATE, MIN_FACING_SPEED, MoveSpeed, TurnRate, angle_delta, face_travel,
    turn_rate_of, turn_toward, wrap_angle, yaw_to,
};
use stormlight_shared::protocol::ProtocolPlugin;

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

/// A unit that has all but stopped must not be re-aimed by whatever direction its
/// last sliver of travel happened to point.
///
/// The desired heading is derived from the distance a mover *realized* this tick,
/// and that is not always its intended step: local avoidance steers it, and near a
/// goal or against an obstacle the realized travel shrinks toward nothing while its
/// direction stays as noisy as ever. Taking a heading from a vector that short is
/// reading a heading out of noise, and the unit spins on the spot chasing it — the
/// jitter a player sees as the model twitching while it stands.
///
/// So the threshold is a **speed**, not a distance: what matters is whether the
/// mover is going anywhere, which a length alone cannot say without the tick it was
/// covered in.
#[test]
fn travel_too_slow_to_be_a_direction_never_re_aims_the_unit() {
    check!().with_type::<Scenario>().for_each(|s| {
        let facing = angle(s.facing);
        let dt = dt(s.dt).max(1e-4);
        // A direction, scaled to a speed strictly below the floor.
        let dir = Vec2::new(angle(s.tx).cos(), angle(s.tz).sin());
        if dir.length() < 1e-3 {
            return;
        }
        let speed = MIN_FACING_SPEED * frac(s.declared) * 0.99;
        let travel = dir.normalize() * speed * dt;

        let turned = face_travel(facing, travel, turn_rate_of(None), dt);
        assert!(
            angle_delta(facing, turned).abs() <= 1e-6,
            "travel at {speed} u/s re-aimed a unit that is effectively standing still: \
             {facing} -> {turned}",
        );
    });
}

/// The floor must not swallow real movement: anything at or above it still steers.
#[test]
fn travel_fast_enough_to_be_a_direction_still_turns_the_unit() {
    check!().with_type::<Scenario>().for_each(|s| {
        let facing = angle(s.facing);
        let dt = dt(s.dt).max(1e-4);
        let dir = Vec2::new(angle(s.tx).cos(), angle(s.tz).sin());
        if dir.length() < 1e-3 {
            return;
        }
        let dir = dir.normalize();
        // Comfortably a walk, so this is about the floor and not about precision.
        let travel = dir * (MIN_FACING_SPEED * 10.0).max(1.0) * dt;
        let want = angle_delta(facing, yaw_to(travel));
        if want.abs() < 1e-3 {
            return;
        }

        let turned = face_travel(facing, travel, turn_rate_of(None), dt);
        let got = angle_delta(facing, turned);
        assert!(
            got.abs() > 0.0 && got.signum() == want.signum(),
            "a unit moving at a walk was not steered toward where it went",
        );
    });
}

/// Both halves of the shared movement law must be told the *same* turn rate.
///
/// [`turn_rate_of`] is documented as "shared, and named by both the authoritative
/// server and the predicting client, so the two cannot disagree about how fast the
/// hero the player is watching turns". That promise is only kept if the component
/// it reads actually reaches the client: the predicting half takes an
/// `Option<&TurnRate>`, and an unreplicated component is `None` there forever — so
/// the client would predict every unit at [`DEFAULT_TURN_RATE`] while the server
/// turned it at whatever its content declared.
///
/// The two then disagree by a fixed factor on every tick the unit is turning, and
/// the correction that reconciles them is what a player sees as the model snapping
/// round in steps instead of sweeping. `MoveSpeed`, the law's other input, has been
/// replicated and predicted all along; this is the one that was missed.
#[test]
fn the_protocol_replicates_every_input_of_the_movement_law() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(ProtocolPlugin);
    app.finish();

    for (component, what) in [
        (app.world().component_id::<MoveSpeed>(), "how fast a unit moves"),
        (app.world().component_id::<TurnRate>(), "how fast a unit turns"),
    ] {
        let component = component.expect("registering a component introduces it to the world");
        let registry = app.world().resource::<ComponentRegistry>();
        assert!(
            registry.component_id_to_kind.contains_key(&component),
            "{what} is not on the protocol, so the predicting client never learns it \
             and silently substitutes the engine default",
        );
    }
}
