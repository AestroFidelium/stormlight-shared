//! The hero the player drives turns, it does not flick (stormlight/server#65).
//!
//! Position was smoothed from every angle — predicted, corrected, frame
//! interpolated — while heading was left as a snap: whatever direction the mover
//! travelled this tick became its rotation outright. Walking is a sequence of
//! straight legs (a corridor's corners, a re-order, a step steered by the crowd),
//! so the model held one angle, jumped to the next, and held that: a character that
//! appears to be able to stand only at certain angles, sliding between them.
//!
//! This is the visible half of the fallback rate pinned in `property/turn_rate.rs`,
//! through the very system the client runs on the unit it controls. Two things
//! matter and they pull against each other: no single tick may move the heading
//! more than a tick's worth of turning (that is the flick), and the heading must
//! still arrive at the direction of travel (that is the lag).

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::Predicted;
use stormlight_shared::movement::{
    DEFAULT_TURN_RATE, MoveGoal, MoveSpeed, PredictedMovementPlugin, angle_delta, yaw_of, yaw_to,
};

/// The tick the prediction is driven at, and how far/fast the hero walks.
const STEP: f32 = 1.0 / 64.0;
const SPEED: f32 = 8.0;
const REACH: f32 = 30.0;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// The heading the hero already stands at, and the one its order sends it to.
    facing: u16,
    heading: u16,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn angle(seed: u16) -> f32 {
    (frac(seed) - 0.5) * (4.0 * core::f32::consts::PI)
}

/// A headless client predicting the one unit its player controls, stepped by hand
/// (a headless app's real elapsed time never fills the fixed-step accumulator).
fn predicting_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(PredictedMovementPlugin);
    app.insert_resource(Time::<Fixed>::from_seconds(f64::from(STEP)));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(core::time::Duration::from_secs_f32(
        STEP,
    )));
    app
}

/// Order a hero that already stands at `facing` to walk along `heading`, and return
/// the yaw it was drawn at after every tick.
fn headings(app: &mut App, facing: f32, heading: f32) -> Vec<f32> {
    let goal = Vec2::new(-heading.sin(), -heading.cos()) * REACH;
    let hero = app
        .world_mut()
        .spawn((
            Predicted,
            Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(facing)),
            MoveSpeed(SPEED),
            MoveGoal(goal),
        ))
        .id();
    // Long enough for the widest turn (half a circle) to complete several times over.
    let ticks = (core::f32::consts::PI / (DEFAULT_TURN_RATE * STEP)).ceil() as u32 * 3;
    (0..ticks)
        .map(|_| {
            app.update();
            yaw_of(app.world().entity(hero).get::<Transform>().expect("the hero has a place"))
        })
        .collect()
}

#[test]
fn no_tick_flicks_the_heroes_heading() {
    check!().with_type::<Scenario>().for_each(|s| {
        let facing = angle(s.facing);
        let mut app = predicting_app();
        let track = headings(&mut app, facing, angle(s.heading));

        // The first sample is measured against where the hero was placed; every
        // later one against the tick before it. None may exceed a tick of turning.
        let budget = DEFAULT_TURN_RATE * STEP + 1e-3;
        let mut previous = facing;
        for (tick, yaw) in track.iter().enumerate() {
            let applied = angle_delta(previous, *yaw).abs();
            assert!(applied <= budget, "tick {tick} flicked the heading by {applied} rad");
            previous = *yaw;
        }
    });
}

#[test]
fn the_hero_still_ends_up_facing_where_it_walks() {
    check!().with_type::<Scenario>().for_each(|s| {
        let heading = angle(s.heading);
        let mut app = predicting_app();
        let track = headings(&mut app, angle(s.facing), heading);

        let ended = *track.last().expect("the walk was stepped at least once");
        let want = yaw_to(Vec2::new(-heading.sin(), -heading.cos()));
        assert!(
            angle_delta(ended, want).abs() < 1e-2,
            "the hero walked its whole order without ever facing the way it went",
        );
    });
}
