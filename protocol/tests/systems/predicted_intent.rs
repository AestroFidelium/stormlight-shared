//! A predicted mirror walks what the **authority** is walking, not only what the
//! player clicked (stormlight/server#151).
//!
//! Client-side prediction was designed around one source of motion: a plain leg
//! the client itself ordered and derived a goal from. Every other way the engine
//! moves a unit — an attack-move's leg, a chase the Combat phase writes, a hold
//! that stops a unit mid-swing without ending its leg — was written only on the
//! server, so the mirror stood still (or kept walking) while the authority did
//! the opposite, and every snapshot arrived as a correction rather than a
//! confirmation. A position corrected back and forth every tick yields a travel
//! direction that flips every tick, which is what a player sees as the model
//! spinning.
//!
//! So the mirror is *told* the intent instead of deriving it. What is pinned here
//! is that being told actually drives it, and — the other half, and the easier
//! one to lose — that being told never runs over the leg the player just ordered
//! and the authority has not seen yet.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bolero::{TypeGenerator, check};
use lightyear::prelude::Predicted;
use stormlight_shared::movement::{
    MoveGoal, MoveIntent, MoveSpeed, PredictedMovementPlugin, wrap_angle, yaw_of,
};

/// Fixed step the prediction is driven at, and the rate the mirror walks.
const STEP: f32 = 1.0 / 64.0;
const SPEED: f32 = 10.0;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    dir: u8,
    dist: u8,
    yaw: u8,
}

fn frac(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX)
}

impl Scenario {
    /// A goal well outside one step, in a generated direction.
    fn goal(&self) -> Vec2 {
        let angle = frac(self.dir) * std::f32::consts::TAU;
        Vec2::new(angle.cos(), angle.sin()) * (20.0 + frac(self.dist) * 20.0)
    }

    fn yaw(&self) -> f32 {
        wrap_angle(frac(self.yaw) * std::f32::consts::TAU - std::f32::consts::PI)
    }
}

/// A headless client that predicts, with no map: the straight-line law, so the
/// only thing under test is which inputs it reads.
fn predicting_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(PredictedMovementPlugin);
    app.insert_resource(Time::<Fixed>::from_seconds(f64::from(STEP)));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(core::time::Duration::from_secs_f32(
        STEP,
    )));
    app
}

/// One resting mirror at the origin, carrying the replicated fallback rate.
fn mirror(app: &mut App) -> Entity {
    let e = app.world_mut().spawn((Predicted, Transform::default(), MoveSpeed(SPEED))).id();
    app.update(); // Bevy's first update lands a zero delta; absorb it.
    e
}

fn pos(app: &App, e: Entity) -> Vec2 {
    let t = app.world().entity(e).get::<Transform>().expect("the mirror has a place");
    Vec2::new(t.translation.x, t.translation.z)
}

fn step(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

/// The whole of divergences 1 and 3: an attack-move's leg and a chase are both
/// goals the *server* wrote. The mirror never ordered either and must walk them
/// anyway.
#[test]
fn a_mirror_walks_a_goal_it_was_never_ordered() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let goal = s.goal();

        app.world_mut().entity_mut(e).insert(MoveIntent {
            goal: Some(goal),
            speed: SPEED,
            held: false,
            facing: None,
        });
        step(&mut app, 40);

        let at = pos(&app, e);
        assert!(
            at.length() > 1e-3,
            "a mirror told where the authority is walking its unit stood still ({at})",
        );
        assert!(
            at.distance(goal) < goal.length(),
            "a mirror walked away from the goal it was told about ({at} vs {goal})",
        );
        assert_eq!(
            app.world().entity(e).get::<MoveGoal>().map(|g| g.0),
            Some(goal),
            "the authority's goal was not taken up as the mirror's own",
        );
    });
}

/// Divergence 2: the hold stops a unit *without* ending its leg, so the mirror
/// has to stop too — and keep the goal, so the interrupted leg resumes rather
/// than counting as arrived.
#[test]
fn a_held_mirror_stops_but_keeps_its_leg() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let goal = s.goal();
        let walking = MoveIntent { goal: Some(goal), speed: SPEED, held: false, facing: None };

        app.world_mut().entity_mut(e).insert(walking);
        step(&mut app, 20);
        let travelled = pos(&app, e);
        assert!(travelled.length() > 1e-3, "the scenario needs a mirror that was walking");

        // The authority stops it mid-leg (a swing, or an order to hold).
        app.world_mut().entity_mut(e).insert(MoveIntent { held: true, ..walking });
        step(&mut app, 30);
        let held_at = pos(&app, e);
        assert!(
            held_at.distance(travelled) < 1e-2,
            "a mirror kept walking while the authority was holding its unit still \
             (moved from {travelled} to {held_at})",
        );
        assert_eq!(
            app.world().entity(e).get::<MoveGoal>().map(|g| g.0),
            Some(goal),
            "a hold ended the mirror's leg instead of interrupting it",
        );

        // …and lets it go again.
        app.world_mut().entity_mut(e).insert(walking);
        step(&mut app, 20);
        assert!(
            pos(&app, e).distance(held_at) > 1e-3,
            "a mirror never resumed the leg the authority let it go on",
        );
    });
}

/// The authority also says when a unit has *stopped* walking — it arrived, was
/// stopped, or the crowd jammed it — and the mirror must let the leg go, or it
/// pushes at a goal nothing else believes in.
#[test]
fn a_mirror_drops_a_leg_the_authority_gave_up() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let goal = s.goal();

        app.world_mut().entity_mut(e).insert(MoveIntent {
            goal: Some(goal),
            speed: SPEED,
            held: false,
            facing: None,
        });
        step(&mut app, 20);
        let travelled = pos(&app, e);

        app.world_mut().entity_mut(e).insert(MoveIntent {
            goal: None,
            speed: SPEED,
            held: false,
            facing: None,
        });
        step(&mut app, 30);

        assert!(
            app.world().entity(e).get::<MoveGoal>().is_none(),
            "the mirror kept a leg the authority had already dropped",
        );
        assert!(
            pos(&app, e).distance(travelled) < 1e-2,
            "the mirror walked on after the authority stopped its unit",
        );
    });
}

/// The other half, and the one that is easy to lose: an intent that changed for
/// some *other* reason must not run over the leg the player just ordered.
///
/// A unit standing in a fight is held, with no goal at all, and its heading churns
/// as it turns to swing — so an intent update lands on the mirror constantly in
/// precisely the state a player clicks out of. If every update re-asserted "the
/// authority says you are walking nowhere", the head start would be cancelled a
/// tick after it was written and the hero would not move until the round trip
/// came back. That is the latency prediction exists to hide.
#[test]
fn an_intent_update_does_not_cancel_the_leg_the_player_just_ordered() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let goal = s.goal();

        // Standing and fighting: held, no goal, heading turning every tick.
        let fighting = MoveIntent { goal: None, speed: SPEED, held: true, facing: Some(s.yaw()) };
        app.world_mut().entity_mut(e).insert(fighting);
        step(&mut app, 5);

        // The player clicks: the client head-starts the leg locally.
        app.world_mut().entity_mut(e).insert(MoveGoal(goal));

        // …and the authority, which has not heard about it yet, keeps publishing
        // the same intent with a freshly turned heading.
        for _ in 0..20 {
            app.world_mut()
                .entity_mut(e)
                .insert(MoveIntent { facing: Some(s.yaw() * 0.5), ..fighting });
            app.update();
        }

        assert_eq!(
            app.world().entity(e).get::<MoveGoal>().map(|g| g.0),
            Some(goal),
            "an intent the authority had not changed cancelled the freshly ordered leg",
        );
        assert!(
            pos(&app, e).length() > 1e-3,
            "the hero stood still after the click: the stale hold was applied to a leg \
             the authority never stopped",
        );
    });
}

/// While a unit is not travelling, its heading is whatever the authority is
/// holding it at — the Combat phase turns it to face what it is swinging at, and
/// that is not something travel can imply. So it is sent, and adopted.
///
/// The common shape of this is a unit with no leg at all: an idle unit defends
/// itself where it stands, so nothing about its goal says which way it is
/// pointing.
#[test]
fn a_standing_mirror_adopts_the_authority_heading() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let yaw = s.yaw();

        app.world_mut().entity_mut(e).insert(MoveIntent {
            goal: None,
            speed: SPEED,
            held: true,
            facing: Some(yaw),
        });
        step(&mut app, 5);

        let drawn = yaw_of(app.world().entity(e).get::<Transform>().expect("a place"));
        assert!(
            wrap_angle(drawn - yaw).abs() < 1e-3,
            "a standing mirror did not face where the authority is pointing its unit \
             (drawn {drawn}, authoritative {yaw})",
        );
    });
}

/// …and the mirror of that: a heading the authority published while it thought
/// the unit was standing must not be pinned onto a unit that is walking. The
/// travel-derived heading is the shared law, and the mirror is running it.
#[test]
fn a_travelling_mirror_keeps_the_heading_its_travel_implies() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut app = predicting_app();
        let e = mirror(&mut app);
        let goal = s.goal();

        // The authority believes the unit is standing and facing `yaw`; the player
        // has just ordered a leg the authority has not seen.
        app.world_mut().entity_mut(e).insert((
            MoveGoal(goal),
            MoveIntent { goal: None, speed: SPEED, held: true, facing: Some(s.yaw()) },
        ));
        step(&mut app, 60);

        let drawn = yaw_of(app.world().entity(e).get::<Transform>().expect("a place"));
        let travel = stormlight_shared::movement::yaw_to(goal);
        assert!(
            wrap_angle(drawn - travel).abs() < 1e-2,
            "a walking mirror was pinned to a heading from before it started walking \
             (drawn {drawn}, travelling toward {travel})",
        );
    });
}
