//! The predicting client walks its own unit through a standing time field at the
//! pace the authority would (stormlight/server#232).
//!
//! The server derives a unit's pace each tick from where the unit stands as the
//! tick starts; a predicting client that knows the field and its own terms does the
//! same, so walking in and walking out are predicted rather than corrected.
//! Pinned against the authority's own stepping, with no pace log at all:
//!
//! - a walker crossing a field is where the authority has it on every tick —
//!   slowed (or stopped) from the tick after it reaches the edge, back to pace the
//!   tick after it leaves;
//! - a field the unit's terms do not name bends nothing, wherever it stands;
//! - a field outside its open span bends nothing.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::{LocalTimeline, Predicted, Tick};
use stormlight_shared::bodies::{BodyFact, BodyKindFact};
use stormlight_shared::connection::tick_duration;
use stormlight_shared::movement::{
    DEFAULT_TURN_RATE, MoveGoal, MoveSpeed, PredictedMovementPlugin, advance_mover,
};
use stormlight_shared::time_field::{FieldFact, PaceTerms};
use stormlight_shared::time_scale::TimeScale;

const SPEED: f32 = 4.0;

/// How the field stands across the walker's road.
#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Field {
    /// Where along the road its centre is, in tenths.
    #[generator(20u16..=120)]
    along: u16,
    /// Its radius, in tenths.
    #[generator(5u16..=40)]
    radius: u16,
    /// Its pace, per mille (0 stops time).
    #[generator(0u16..=1500)]
    pace: u16,
    /// Whether the walker's terms name it.
    kept: bool,
    /// Ticks after the start it opens and how long it stands.
    #[generator(0u16..=30)]
    opens: u16,
    #[generator(1u16..=200)]
    lasts: u16,
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

#[test]
fn a_walker_crosses_a_standing_field_exactly_as_the_authority_does() {
    check!().with_type::<Field>().for_each(|f| {
        let mut app = predicting_app();
        let start = now(&app);
        let goal = Vec2::new(30.0, 0.0);
        let centre = Vec2::new(f32::from(f.along) / 10.0, 0.0);
        let opens = start + i16::try_from(f.opens).unwrap_or(0);
        let fact = FieldFact {
            key: 9,
            pace: TimeScale::from_per_mille(f.pace),
            opens,
            closes: opens + i16::try_from(f.lasts).unwrap_or(0),
        };
        let radius = f32::from(f.radius) / 10.0;
        app.world_mut().spawn((
            fact,
            BodyFact::new(BodyKindFact::Zone, radius, 0),
            Transform::from_xyz(centre.x, 0.0, centre.y),
        ));
        let terms =
            PaceTerms { own: TimeScale::NORMAL, fields: if f.kept { vec![9] } else { vec![] } };
        app.world_mut().spawn(terms.clone());
        let unit = app
            .world_mut()
            .spawn((Predicted, Transform::default(), MoveSpeed(SPEED), MoveGoal(goal)))
            .id();

        let dt = tick_duration().as_secs_f32();
        let (mut pos, mut yaw) = (Vec2::ZERO, 0.0_f32);
        let mut stepped = start;
        for _ in 0..200 {
            app.update();
            // The authority, stepped to the same tick by the same law.
            while now(&app) - stepped > 0 {
                stepped = stepped + 1;
                // Written out rather than asked of the shared law, so the law is what is
                // being checked: named, open on this tick, and covering where the
                // walker stands as the tick starts.
                let open = stepped - fact.opens >= 0 && fact.closes - stepped >= 0;
                let inside = pos.distance(centre) <= radius;
                let pace = if f.kept && open && inside { fact.pace } else { TimeScale::NORMAL };
                let step = advance_mover(pos, yaw, goal, SPEED, DEFAULT_TURN_RATE, pace.apply(dt));
                if step.pos != pos {
                    yaw = step.facing;
                }
                pos = step.pos;
            }
            assert!(
                ground(&app, unit).distance(pos) < 1e-4,
                "tick {:?}: predicted {}, authority {pos}",
                stepped,
                ground(&app, unit),
            );
        }
    });
}
