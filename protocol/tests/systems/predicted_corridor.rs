//! The predicting client walks the map, not through it (stormlight/server#65).
//!
//! Client-side prediction only works while both ends run the same law. The mover
//! law was shared from the start, but the *route* was not: the server planned a
//! corridor around the map's geometry while the client predicted a straight line
//! at the goal. Every order past a wall therefore predicted a path the server
//! would never take, and the player watched their hero walk into the wall and get
//! yanked back — once per correction, for as long as the order lasted.
//!
//! Rollback is for absorbing noise along a route, never a disagreement about which
//! route it is. So what is pinned here is that the *prediction itself* respects the
//! geometry:
//!
//! - a predicted mover ordered across the map never enters solid ground;
//! - it still arrives, and still walks straight when the line is clear;
//! - with no map loaded it behaves exactly as it did before any of this.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::Predicted;
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_navigation::{ActiveNavMesh, NavMesh};
use stormlight_shared::movement::{MoveGoal, MoveSpeed, PredictedMovementPlugin};

/// The room and the solid block in the middle of it — the smallest map where
/// "walk around" and "walk straight" are different answers.
const ROOM: f32 = 40.0;
const KEEP: f32 = 8.0;
const BODY: f32 = 0.6;

/// Fixed step the prediction is driven at, and how many of them one order gets.
/// Generous enough for the longest crossing at [`SPEED`].
const STEP: f32 = 1.0 / 64.0;
const STEPS: u32 = 600;
const SPEED: f32 = 12.0;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// Where along the west and east faces the crossing starts and ends.
    from_z: u8,
    to_z: u8,
}

fn frac(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX)
}

fn block(half: f32) -> Vec<[f32; 2]> {
    vec![[-half, -half], [half, -half], [half, half], [-half, half]]
}

fn baked() -> NavMesh {
    NavMesh::bake(&NavMeshDescriptor {
        id: NavMeshId(0),
        outline: block(ROOM),
        obstacles: vec![block(KEEP)],
        agent_radius: BODY,
        placements: Vec::new(),
    })
    .expect("the room should bake")
}

/// Whether a point is inside the solid block, with a hair of tolerance: the region
/// is inset away from it, so a route may hug the face but never cross it.
fn inside_keep(p: Vec2) -> bool {
    p.x.abs() < KEEP - 1e-2 && p.y.abs() < KEEP - 1e-2
}

/// A headless client predicting one controlled mover, with `mapped` deciding
/// whether it has the region to route over.
fn predicting_app(mapped: bool) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(PredictedMovementPlugin);
    // Drive the clock by hand: a headless app's real elapsed time between updates
    // is microseconds, so the fixed-step accumulator would never fill and the
    // prediction — which runs in `FixedUpdate` — would never step at all. One
    // manual step per update makes the run deterministic as well.
    app.insert_resource(Time::<Fixed>::from_seconds(f64::from(STEP)));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(core::time::Duration::from_secs_f32(
        STEP,
    )));
    if mapped {
        app.insert_resource(ActiveNavMesh(baked()));
    }
    app
}

/// Order `mover` from `from` to `goal` and run the prediction, returning every
/// position it passed through.
fn walk(app: &mut App, from: Vec2, goal: Vec2) -> Vec<Vec2> {
    let mover = app
        .world_mut()
        .spawn((
            Predicted,
            Transform::from_xyz(from.x, 0.0, from.y),
            MoveSpeed(SPEED),
            MoveGoal(goal),
        ))
        .id();
    let mut track = Vec::new();
    for _ in 0..STEPS {
        app.update();
        let at = app.world().entity(mover).get::<Transform>().expect("the mover has a place");
        track.push(Vec2::new(at.translation.x, at.translation.z));
    }
    track
}

/// A crossing of the room whose straight line goes through the block.
fn crossing(s: &Scenario) -> (Vec2, Vec2) {
    let from = Vec2::new(-ROOM + 4.0, (frac(s.from_z) - 0.5) * 8.0);
    let to = Vec2::new(ROOM - 4.0, (frac(s.to_z) - 0.5) * 8.0);
    (from, to)
}

#[test]
fn a_predicted_mover_never_walks_into_solid_ground() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app(true);
        let (from, to) = crossing(s);

        for at in walk(&mut app, from, to) {
            assert!(
                !inside_keep(at),
                "the prediction walked the hero into the building it was routed around ({at})",
            );
        }
    });
}

#[test]
fn a_predicted_mover_still_arrives() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app(true);
        let (from, to) = crossing(s);

        let track = walk(&mut app, from, to);
        let last = *track.last().expect("the walk was stepped at least once");
        assert!(
            last.distance(to) <= 1e-1,
            "an order across the room never got there (ended at {last}, wanted {to})",
        );
    });
}

#[test]
fn a_clear_line_is_still_walked_straight() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app(true);
        // Both ends north of the block, so nothing is in the way.
        let z = ROOM - 4.0;
        let from = Vec2::new(-ROOM + 4.0 + frac(s.from_z) * 4.0, z);
        let to = Vec2::new(ROOM - 4.0 - frac(s.to_z) * 4.0, z);

        for at in walk(&mut app, from, to) {
            // A straight walk holds its line: it never leaves the segment it is on.
            assert!(
                (at.y - z).abs() <= 1e-2,
                "an unobstructed order wandered off its line (at {at}, line z = {z})",
            );
        }
    });
}

#[test]
fn without_a_map_the_prediction_is_the_straight_line_it_always_was() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app(false);
        let (from, to) = crossing(s);

        let track = walk(&mut app, from, to);
        // No geometry, no corridor: the mover heads straight at the goal, which
        // takes it clean through where the block would have been.
        let last = *track.last().expect("the walk was stepped at least once");
        assert!(
            last.distance(to) <= 1e-2,
            "a mapless order did not walk its straight line to the goal (ended at {last})",
        );
        assert!(
            track.iter().any(|at| inside_keep(*at)),
            "a mapless prediction avoided geometry it was never given",
        );
    });
}
