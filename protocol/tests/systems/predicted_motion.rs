//! The predicting client follows its unit's motion (stormlight/server#216).
//!
//! The owner's unit is a predicted `Transform`: whatever the client does not
//! simulate itself, it gets corrected for, and a motion the server advances while
//! the client walks on is a correction every tick of a dash. So the server publishes
//! the motion as a fact — the path, the pace, the walls, the tick it started — and
//! the client steps it by the very same per-tick law. Pinned:
//!
//! - on every tick the client puts the unit **exactly** where the authority's own
//!   stepping does — bit for bit, ricochets included, because both run one step
//!   function over the same pieces and the same walls;
//! - a fact that arrives late (it always does: the client runs ahead of the server)
//!   is caught up to the client's tick at once;
//! - a rollback to an earlier tick lands the unit where the motion had it then;
//! - while it is carried, the unit's own walking does not move it.

use core::num::NonZeroU32;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::{LocalTimeline, Predicted};
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_navigation::{ActiveNavMesh, NavMesh};
use stormlight_shared::connection::tick_duration;
use stormlight_shared::motion::{MotionFact, MotionPath, MotionState, PathLeg, Rounds, Walls};
use stormlight_shared::movement::{MoveGoal, MoveSpeed, PredictedMovementPlugin};

const HALF: f32 = 10.0;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(0u16..=359)]
    heading: u16,
    /// A curve's bend, in degrees.
    #[generator(-180i16..=180)]
    bend: i16,
    #[generator(10u16..=600)]
    tenths: u16,
    #[generator(2u16..=200)]
    speed: u16,
    #[generator(0u8..=4)]
    bounces: u8,
    /// How many ticks late the fact reaches the client.
    #[generator(0u8..=12)]
    lag: u8,
    /// How far back a rollback goes.
    #[generator(1u8..=10)]
    rollback: u8,
}

impl Scenario {
    fn fact(&self, started: lightyear::prelude::Tick) -> MotionFact {
        MotionFact {
            path: MotionPath {
                start: Vec2::new(1.0, -2.0),
                heading: Vec2::from_angle(f32::from(self.heading).to_radians()),
                legs: vec![PathLeg::Arc {
                    turn: 0.0,
                    dist: f32::from(self.tenths) / 10.0,
                    bend: f32::from(self.bend).to_radians(),
                }]
                .into(),
                rounds: Rounds::Times(NonZeroU32::MIN),
            },
            speed: f32::from(self.speed),
            limit: None,
            walls: Walls::Bounce(u32::from(self.bounces)),
            started,
        }
    }
}

fn room() -> NavMesh {
    NavMesh::bake(&NavMeshDescriptor {
        id: NavMeshId(0),
        outline: vec![[-HALF, -HALF], [HALF, -HALF], [HALF, HALF], [-HALF, HALF]],
        obstacles: Vec::new(),
        agent_radius: 0.0,
        placements: Vec::new(),
    })
    .expect("the room should bake")
}

/// Advance the timeline once per fixed step, as lightyear does.
fn tick(mut timeline: ResMut<LocalTimeline>) {
    timeline.apply_delta(1);
}

fn predicting_app() -> App {
    let step = tick_duration();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(PredictedMovementPlugin)
        .insert_resource(Time::<Fixed>::from_duration(step))
        .insert_resource(TimeUpdateStrategy::ManualDuration(step))
        .insert_resource(LocalTimeline::default())
        .insert_resource(ActiveNavMesh(room()))
        .add_systems(FixedFirst, tick);
    // Absorb Bevy's zero-delta first update.
    app.update();
    app
}

/// Where the authority's own stepping has the unit after `steps` ticks, and the
/// heading it faces.
fn authority(fact: &MotionFact, steps: u32) -> Vec2 {
    let map = room();
    let mut state = MotionState::new(fact.path.clone(), fact.speed, fact.limit, fact.walls);
    let mut at = fact.path.start;
    let mut trace = Vec::new();
    for _ in 0..steps {
        trace.clear();
        let stepped = state.step(Some(&map), tick_duration().as_secs_f32(), &mut trace);
        if let Some(last) = trace.last() {
            at = *last;
        }
        if stepped.ended.is_some() {
            break;
        }
    }
    at
}

fn ground(app: &App, unit: Entity) -> Vec2 {
    let t = app.world().entity(unit).get::<Transform>().expect("the unit stands");
    Vec2::new(t.translation.x, t.translation.z)
}

fn now(app: &App) -> lightyear::prelude::Tick {
    app.world().resource::<LocalTimeline>().tick()
}

#[test]
fn the_client_steps_a_motion_exactly_as_the_authority_does() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let started = now(&app);
        // Ordered to walk somewhere else entirely, at a brisk pace.
        let unit = app
            .world_mut()
            .spawn((
                Predicted,
                Transform::from_xyz(1.0, 0.0, -2.0),
                MoveSpeed(8.0),
                MoveGoal(Vec2::new(-9.0, 9.0)),
            ))
            .id();
        // The fact arrives `lag` ticks after the motion began.
        for _ in 0..s.lag {
            app.update();
        }
        let fact = s.fact(started);
        app.world_mut().entity_mut(unit).insert(fact.clone());

        for _ in 0..40 {
            app.update();
            let steps = u32::try_from(now(&app) - started).unwrap_or(0);
            assert_eq!(
                ground(&app, unit),
                authority(&fact, steps),
                "{steps} ticks in, the client and the authority disagree",
            );
        }

        // A rollback re-runs earlier ticks: the unit is where the motion had it then.
        app.world_mut().resource_mut::<LocalTimeline>().apply_delta(-i16::from(s.rollback) - 1);
        app.update();
        let steps = u32::try_from(now(&app) - started).unwrap_or(0);
        assert_eq!(
            ground(&app, unit),
            authority(&fact, steps),
            "after a rollback to {steps} ticks in, the client and the authority disagree",
        );
    });
}
