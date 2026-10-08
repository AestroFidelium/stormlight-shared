//! The predicting client steps its own unit at the pace each tick ran at
//! (stormlight/server#225).
//!
//! The owner's unit is predicted: the client walks it, and carries it along its
//! motions, a tick at a time — and on a rollback, steps the same ticks again. The
//! server bends those ticks by the unit's pace; the client reads the pace each tick
//! ran at from the [`PaceLog`] the server publishes. Pinned against the authority's
//! own stepping:
//!
//! - walking, the unit is where the authority has it on every tick, through a stop
//!   and out of it — and after a rollback across the moment it stopped;
//! - carried by a motion, likewise: every tick of the motion runs at that tick's
//!   pace.

use core::num::NonZeroU32;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::{LocalTimeline, Predicted, Tick};
use stormlight_shared::connection::tick_duration;
use stormlight_shared::motion::{MotionFact, MotionPath, MotionState, PathLeg, Rounds, Walls};
use stormlight_shared::movement::{
    DEFAULT_TURN_RATE, MoveGoal, MoveSpeed, PredictedMovementPlugin, advance_mover,
};
use stormlight_shared::pace_log::PaceLog;
use stormlight_shared::time_scale::TimeScale;

const SPEED: f32 = 4.0;

/// When the pace changes, relative to the start, and to what.
#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Change {
    #[generator(1..=30u16)]
    at: u16,
    #[generator(0..=3000u16)]
    pace: u16,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Change>>().with().len(0usize..=4))]
    changes: Vec<Change>,
    #[generator(1u8..=10)]
    rollback: u8,
}

impl Scenario {
    fn log(&self, start: Tick) -> PaceLog {
        let mut changes = self.changes.clone();
        changes.sort_by_key(|c| c.at);
        let mut log = PaceLog::default();
        for c in changes {
            log.record(start + i16::try_from(c.at).unwrap_or(0), TimeScale::from_per_mille(c.pace));
        }
        log
    }
}

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
        .add_systems(FixedFirst, tick);
    app.update();
    app
}

fn now(app: &App) -> Tick {
    app.world().resource::<LocalTimeline>().tick()
}

fn ground(app: &App, unit: Entity) -> Vec2 {
    let t = app.world().entity(unit).get::<Transform>().expect("the unit stands");
    Vec2::new(t.translation.x, t.translation.z)
}

/// Where the authority has a walker `ticks` ticks after `start`, each tick bent
/// by the pace the log says it ran at.
fn walked(log: &PaceLog, start: Tick, ticks: u16, from: Vec2, goal: Vec2) -> Vec2 {
    let dt = tick_duration().as_secs_f32();
    let (mut pos, mut yaw) = (from, 0.0_f32);
    for k in 1..=ticks {
        let pace = log.pace_at(start + i16::try_from(k).unwrap_or(0));
        let step = advance_mover(pos, yaw, goal, SPEED, DEFAULT_TURN_RATE, pace.apply(dt));
        // Facing is written only on a tick the unit moved, as both ends do.
        if step.pos != pos {
            yaw = step.facing;
        }
        pos = step.pos;
    }
    pos
}

#[test]
fn a_predicted_walker_keeps_the_pace_each_tick_ran_at() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let start = now(&app);
        let goal = Vec2::new(100.0, 0.0);
        let log = s.log(start);
        let unit = app
            .world_mut()
            .spawn((Predicted, Transform::default(), MoveSpeed(SPEED), MoveGoal(goal), log.clone()))
            .id();
        for _ in 0..40 {
            app.update();
            let ticks = u16::try_from(now(&app) - start).unwrap_or(0);
            let expected = walked(&log, start, ticks, Vec2::ZERO, goal);
            assert!(
                ground(&app, unit).distance(expected) < 1e-3,
                "{ticks} ticks in: predicted {}, authority {expected}",
                ground(&app, unit),
            );
        }
    });
}

fn motion(start: Tick) -> MotionFact {
    MotionFact {
        path: MotionPath {
            start: Vec2::ZERO,
            heading: Vec2::X,
            legs: vec![PathLeg::Straight { turn: 0.0, dist: 30.0 }].into(),
            rounds: Rounds::Times(NonZeroU32::MIN),
        },
        speed: 6.0,
        limit: None,
        walls: Walls::Pass,
        started: start,
    }
}

/// Where the authority's motion has the unit `ticks` ticks after it started.
fn carried(fact: &MotionFact, log: &PaceLog, ticks: u16) -> Vec2 {
    let dt = tick_duration().as_secs_f32();
    let mut state = MotionState::new(fact.path.clone(), fact.speed, fact.limit, fact.walls);
    let mut at = fact.path.start;
    let mut trace = Vec::new();
    for k in 1..=ticks {
        let pace = log.pace_at(fact.started + i16::try_from(k).unwrap_or(0));
        let dt = pace.apply(dt);
        if dt <= 0.0 {
            continue;
        }
        trace.clear();
        let stepped = state.step(None, dt, &mut trace);
        if let Some(last) = trace.last() {
            at = *last;
        }
        if stepped.ended.is_some() {
            break;
        }
    }
    at
}

#[test]
fn a_predicted_motion_keeps_the_pace_each_tick_ran_at() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let start = now(&app);
        let log = s.log(start);
        let fact = motion(start);
        let unit = app
            .world_mut()
            .spawn((Predicted, Transform::default(), fact.clone(), log.clone()))
            .id();
        for _ in 0..40 {
            app.update();
            let ticks = u16::try_from(now(&app) - start).unwrap_or(0);
            assert!(
                ground(&app, unit).distance(carried(&fact, &log, ticks)) < 1e-4,
                "{ticks} ticks in, the client and the authority disagree",
            );
        }
        // A rollback re-runs earlier ticks, each at the pace it ran at.
        app.world_mut().resource_mut::<LocalTimeline>().apply_delta(-i16::from(s.rollback) - 1);
        app.update();
        let ticks = u16::try_from(now(&app) - start).unwrap_or(0);
        assert!(
            ground(&app, unit).distance(carried(&fact, &log, ticks)) < 1e-4,
            "after a rollback to {ticks} ticks in, the client and the authority disagree",
        );
    });
}
