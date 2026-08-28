//! The **ground mover**: the kinematic law a unit walks by, the components that
//! law reads, and the client-side prediction of it.
//!
//! Everything here is shared rather than server-side because the client predicts
//! the unit it controls (server#50) and must integrate it *identically* — the same
//! step, the same turn, the same corridor. What a player asks a unit to do is a
//! separate thing and lives in [`crate::orders`]; this module is only how a unit
//! that has been asked actually moves.
//!
//! Content-free: a speed, a turn rate and a ground point. What "move" costs or how
//! fast is the unit's own generic movement, server-side.

use core::f32::consts::{PI, TAU};

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use stormlight_navigation::ActiveNavMesh;

/// Register the mover's replicated stats. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// Walking somewhere is no longer a message of its own: a move is one kind of
/// [`OrderRequest`](crate::orders::OrderRequest) among the rest (server#90), so a
/// plain click and a shift-queued leg travel the same door and land in the same
/// queue. A separate move message would have been a second way to change a unit's
/// intent that the queue never saw, which is exactly the state the queue exists
/// to end.
pub fn register(app: &mut App) {
    // Replicate the movement stats a predicting client reads (stormlight/server#50):
    // the ghost of the unit it controls has to advance at the *authoritative* rate,
    // not an engine default. `predict_movement` reads an `Option<&TurnRate>`, so a
    // stat that never crosses the wire is not "absent" on the client — it is
    // silently the default, while the server moves the same unit at whatever its
    // content declared. The two halves then disagree by a fixed factor on every
    // tick, and the correction reconciling them is what a player sees as the model
    // snapping round in steps instead of sweeping.
    //
    // Replicated plainly, **not predicted** (stormlight/server#86). These are stats
    // the client only ever *reads*; nothing on it simulates a unit getting faster.
    // A predicted component is driven by the client's own simulation and reconciled
    // only through a rollback, so a predicted speed stayed at whatever the unit
    // spawned with — a buff or a talent moved the server's unit and the owner's
    // ghost kept walking at the old rate, rubber-banding for as long as the change
    // lasted. Unpredicted, replication writes each change straight onto the mirror.
    // Inert on the server (no prediction runs there).
    app.register_component::<MoveSpeed>();
    app.register_component::<TurnRate>();
}

// ---------------------------------------------------------------------------
// Kinematic law — the pure ground-mover integrator (stormlight/server#49).
//
// Lives here, beside the wire type and mirroring the projectile motion law
// (`projectile_position`), so the authoritative server and the predicting
// client (server#50) integrate a unit **identically** — a predicted unit and
// its server ghost never disagree beyond float noise. No `World`, no `Time`:
// every function is a pure map, property-tested directly.
// ---------------------------------------------------------------------------

/// Step a mover at `pos` straight toward `goal` by at most `speed * dt`, never
/// overshooting. Pure — no `World`/`Time`. For finite inputs with `speed, dt >= 0`:
/// - the step length is `<= speed * dt` (no teleport);
/// - the distance to `goal` never increases (monotone approach);
/// - once within reach it returns **exactly** `goal` (clean arrival, no jitter).
#[must_use]
pub fn step_toward(pos: Vec2, goal: Vec2, speed: f32, dt: f32) -> Vec2 {
    let to = goal - pos;
    let dist = to.length();
    let step = (speed * dt).max(0.0);
    if dist <= step || dist == 0.0 { goal } else { pos + to * (step / dist) }
}

/// Normalize any yaw to the canonical half-open turn range `(-PI, PI]`, shifting
/// by a whole number of turns. The single representative both ends agree on.
#[must_use]
pub fn wrap_angle(a: f32) -> f32 {
    let mut d = a % TAU;
    if d <= -PI {
        d += TAU;
    }
    if d > PI {
        d -= TAU;
    }
    d
}

/// The shortest signed rotation turning yaw `from` onto yaw `target`, in
/// `(-PI, PI]` (positive is counter-clockwise). `angle_delta(a, a) == 0`.
#[must_use]
pub fn angle_delta(from: f32, target: f32) -> f32 {
    wrap_angle(target - from)
}

/// Turn yaw `from` toward `target` by at most `rate * dt` radians along the
/// shortest arc, snapping exactly onto `target` once within reach. Result is
/// normalized to `(-PI, PI]`. Pure. For finite inputs with `rate, dt >= 0`:
/// - the applied rotation has magnitude `<= rate * dt` (bounded, no snap-spin);
/// - it never turns away from `target` (shortest arc, monotone approach);
/// - within reach it returns exactly `wrap_angle(target)` (clean settle).
///
/// A very large `rate * dt` (e.g. `f32::INFINITY` for "instant") always reaches
/// `target` in one step, since `|angle_delta| <= PI`.
#[must_use]
pub fn turn_toward(from: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let max = (rate * dt).max(0.0);
    let d = angle_delta(from, target);
    if d.abs() <= max { wrap_angle(target) } else { wrap_angle(from + d.signum() * max) }
}

/// The yaw (rotation about `+Y`) whose forward points along the ground direction
/// `dir = (x, z)`: `Quat::from_rotation_y(yaw_to(dir)) * Vec3::NEG_Z` lies along
/// `dir`. Returns `0.0` for a (near-)zero vector — the caller keeps the current
/// facing when a unit is not moving.
///
/// **Forward is `-Z`**, which is Bevy's own convention (`Transform::forward`, and
/// the direction `looking_at` points at its target) and glTF's. It was `+X` until
/// stormlight/server#65, which meant a model authored the ordinary way faced a
/// quarter turn off its travel direction: heroes walked sideways across the field.
/// An engine that disagrees with the engine it is built on taxes every piece of art
/// ever imported into it, so the disagreement is settled here, once, rather than
/// with a correction on each model.
#[must_use]
pub fn yaw_to(dir: Vec2) -> f32 {
    if dir.length_squared() <= f32::EPSILON { 0.0 } else { (-dir.x).atan2(-dir.y) }
}

/// Below this ground speed a mover is not going anywhere, and the direction of what
/// little it covered is noise rather than a heading (world units per second).
///
/// It is a **speed** and not a distance because the same sliver of travel means
/// different things over a long tick and a short one; only dividing by the tick
/// says whether the unit is moving.
///
/// The number this replaced was `f32::EPSILON` on the squared length — small enough
/// that a nanometre of drift counted as a direction. That is not a hypothetical:
/// realized travel is what local avoidance (server#51) leaves after steering, and
/// against an obstacle or on the last step into a goal it shrinks toward nothing
/// while its direction keeps swinging. The unit then chases a heading that is
/// re-rolled every tick and visibly twitches on the spot.
pub const MIN_FACING_SPEED: f32 = 0.25;

/// Turn `facing` toward the direction a mover actually travelled this tick, at
/// `turn_rate` radians/second. A mover that did not meaningfully move keeps its
/// heading — there is no travel direction to face, and snapping to a default would
/// spin a resting unit. Pure.
///
/// Split out of [`advance_mover`] because a mover's realized travel is not always
/// its straight step toward the goal: local avoidance (server#51) steers it, and
/// the unit must face where it *went*, not where it wanted to go. That is also why
/// the heading is only taken from travel above [`MIN_FACING_SPEED`]: the same
/// steering that makes realized travel the honest source makes it a noisy one at
/// the point the mover is nearly stationary.
#[must_use]
pub fn face_travel(facing: f32, travel: Vec2, turn_rate: f32, dt: f32) -> f32 {
    let moving = dt > 0.0 && dt.is_finite() && travel.length() / dt >= MIN_FACING_SPEED;
    let desired = if moving { yaw_to(travel) } else { facing };
    turn_toward(facing, desired, turn_rate, dt)
}

/// One kinematic step's result: the new ground position, the new facing yaw, and
/// whether the mover reached its `goal` this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveStep {
    /// New ground position (XZ world plane, stored as `Vec2(x, z)`).
    pub pos: Vec2,
    /// New facing yaw, normalized to `(-PI, PI]`.
    pub facing: f32,
    /// Whether the goal was reached exactly this step.
    pub arrived: bool,
}

/// Advance a ground mover one tick toward `goal`: translate toward it at `speed`
/// (never overshooting — [`step_toward`]) and rotate `facing` toward the direction
/// of travel at `turn_rate` ([`turn_toward`]). The single law the authoritative
/// server and the predicting client both integrate, so a predicted unit matches
/// its server ghost. When the unit does not move this step the facing is left
/// unchanged (there is no travel direction to face). Pure.
#[must_use]
pub fn advance_mover(
    pos: Vec2,
    facing: f32,
    goal: Vec2,
    speed: f32,
    turn_rate: f32,
    dt: f32,
) -> MoveStep {
    let next = step_toward(pos, goal, speed, dt);
    MoveStep {
        pos: next,
        facing: face_travel(facing, next - pos, turn_rate, dt),
        arrived: next == goal,
    }
}

// ---------------------------------------------------------------------------
// Generic mover components + client-side prediction (stormlight/server#49/#50).
//
// These live in `shared` — not `server` — because the client predicts the unit
// it controls (server#50) and must name the same component types the
// authoritative server drives. `MoveSpeed` additionally replicates, so a
// predicting client advances its ghost at the authoritative speed.
// ---------------------------------------------------------------------------

/// The destination a unit is walking to — a world **XZ** ground point (stored as
/// `Vec2(x, z)`). Set authoritatively server-side when a move order starts, and
/// on the controlled unit's own client set locally the instant the order is issued
/// so prediction starts the same tick (server#50). Removed on arrival. Not
/// replicated — it is intent, derived identically on both ends from the order.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveGoal(pub Vec2);

/// A unit's **fallback** move speed, world units/second — used when a unit has no
/// aggregated `move_speed` stat (content-free demos). Replicated + predicted so a
/// client predicting the unit it controls advances its ghost at the authoritative
/// speed. The server prefers the aggregated `move_speed` stat when present (see the
/// server's `effective_move_speed`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveSpeed(pub f32);

/// A unit's facing yaw (rotation about `+Y`), integrated toward its travel
/// direction each Movement tick and written into `Transform.rotation` — so heading
/// replicates for free. Absent ⇒ the unit faces its travel direction instantly.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Facing(pub f32);

/// How fast a unit may turn, radians/second. Absent ⇒ the engine's
/// [`DEFAULT_TURN_RATE`]. Generic; a mod/stat can drive it later.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRate(pub f32);

/// The angular speed a unit turns at when it declares no [`TurnRate`] of its own —
/// radians/second, a full circle in half a second.
///
/// It used to be `f32::INFINITY` ("instant"), and nothing has ever declared a rate,
/// so *every* unit snapped its heading (stormlight/server#65). Since a unit travels
/// in straight legs — corridor corner to corridor corner, re-aimed on each new
/// order, steered a little by the crowd — the direction of travel is a step
/// function, and a heading that follows it instantly is one too: the model holds an
/// angle, jumps, and holds the next, while the body slides smoothly underneath. A
/// finite fallback makes heading a curve like position already is.
///
/// Fast enough that control still reads as immediate (a half turn takes a quarter
/// second — less than a walk-up to anything), slow enough that the turn is visible
/// as a turn. Content that wants another feel declares [`TurnRate`]; a heavy siege
/// unit that pivots slowly and a scout that spins on the spot are both a number in
/// a descriptor, not an engine change.
pub const DEFAULT_TURN_RATE: f32 = 4.0 * PI;

/// The turn rate to integrate a unit's facing at: its declared [`TurnRate`], else
/// [`DEFAULT_TURN_RATE`]. A negative declaration is a sign error rather than a
/// request to turn backwards, so it clamps to "does not turn".
///
/// Shared, and named by both the authoritative server and the predicting client, so
/// the two cannot disagree about how fast the hero the player is watching turns.
#[must_use]
pub fn turn_rate_of(declared: Option<&TurnRate>) -> f32 {
    declared.map_or(DEFAULT_TURN_RATE, |t| t.0.max(0.0))
}

/// The corridor a unit is walking: the route's remaining waypoints, in order,
/// ending at its [`MoveGoal`]. Present only while a map is loaded — without
/// geometry there is nothing to route around and the goal is walked directly.
///
/// This lives in `shared`, and that is the whole point (stormlight/server#65).
/// While only the server routed, the predicting client walked a **straight line**
/// at every goal the server walked *around*: the two disagreed by the width of the
/// building between them, every wall bounced the player backwards, and every turn
/// read as a teleport. Rollback cannot absorb a disagreement about the route —
/// only about the noise along it — so both ends plan the same corridor from the
/// same region and the correction shrinks to float noise.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MovePath {
    /// Waypoints still ahead, nearest first.
    waypoints: Vec<Vec2>,
}

impl MovePath {
    /// A corridor over `waypoints` (nearest first).
    #[must_use]
    pub fn new(waypoints: Vec<Vec2>) -> Self {
        Self { waypoints }
    }

    /// The corner being walked to, if any.
    #[must_use]
    pub fn next(&self) -> Option<Vec2> {
        self.waypoints.first().copied()
    }

    /// Drop the corner just reached.
    pub fn advance(&mut self) {
        if !self.waypoints.is_empty() {
            self.waypoints.remove(0);
        }
    }

    /// Whether the whole corridor has been walked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }

    /// How many corners are still ahead.
    #[must_use]
    pub fn len(&self) -> usize {
        self.waypoints.len()
    }
}

/// What a mover should steer at this tick: the next corner of its corridor when it
/// has one, else the goal itself. The goal is only ever the *last* corner, so a
/// mover with no route walks straight at it exactly as before any map existed.
///
/// Split out as a free function so the authoritative server and the predicting
/// client cannot drift apart on it.
#[must_use]
pub fn steer_target(path: Option<&MovePath>, goal: Vec2) -> Vec2 {
    path.and_then(MovePath::next).unwrap_or(goal)
}

/// Book the arrival a mover just made and answer whether the **whole order** is
/// finished. Reaching a waypoint only ends the current leg — the corner is dropped
/// and the mover heads for the next one; the order is done when the last one is
/// behind it. A mover with no corridor is finished the moment it arrives.
///
/// The other half of [`steer_target`], and shared for the same reason: an end that
/// retired a leg the other kept would be steering at a different corner within a
/// tick.
#[must_use]
pub fn consume_arrival(path: Option<&mut MovePath>, arrived: bool) -> bool {
    match path {
        Some(path) if arrived => {
            path.advance();
            path.is_empty()
        }
        _ => arrived,
    }
}

/// The corridor from `from` to `goal` over the loaded map, or `None` when there is
/// no map, or no route (in which case the caller walks the goal directly — the
/// same behaviour as having no geometry at all, never worse).
#[must_use]
pub fn plan_path(map: Option<&ActiveNavMesh>, from: Vec2, goal: Vec2) -> Option<MovePath> {
    map?.0.route(from, goal).map(MovePath::new)
}

/// How far a predicted `Transform` may drift from the confirmed server value
/// before a rollback corrects it. Below this, tiny float differences between the
/// client's prediction and the server's authority are ignored, so a smoothly
/// predicted unit is not yanked back every snapshot. World units.
pub const ROLLBACK_POSITION_EPS: f32 = 0.05;

/// Rollback decision for the predicted `Transform`: roll back only when the
/// confirmed (authoritative) position differs from the predicted one by more than
/// [`ROLLBACK_POSITION_EPS`]. Rotation (facing) is cosmetic and never forces a
/// rollback on its own. Registered via Lightyear's `add_should_rollback`.
#[must_use]
pub fn transform_should_rollback(confirmed: &Transform, predicted: &Transform) -> bool {
    confirmed.translation.distance(predicted.translation) > ROLLBACK_POSITION_EPS
}

/// Extract a mover's facing yaw back out of a `Transform.rotation` that was written
/// by `Quat::from_rotation_y` — the inverse of the facing write, so a predicting
/// client resumes turning from the heading it last rendered.
#[must_use]
pub fn yaw_of(transform: &Transform) -> f32 {
    transform.rotation.to_euler(EulerRot::YXZ).0
}

/// Client-side predicted movement (server#50): advance every `Predicted` unit
/// toward its [`MoveGoal`] with the same shared [`advance_mover`] law the server
/// runs, so the unit the player controls responds the tick they order it — not
/// after a server round-trip. Runs in `FixedUpdate`, so Lightyear re-runs it during
/// rollback to reconcile against the authoritative `Transform`.
///
/// Reads the replicated [`MoveSpeed`]; a `Predicted` unit with no goal or no speed
/// simply rests. Facing is taken from and written back into `Transform.rotation`,
/// exactly as the server does — so the predicted ghost turns to face its path too.
///
/// With a map loaded it steers at the corridor [`plan_predicted_paths`] laid out
/// for it rather than straight at the goal — the same [`steer_target`] /
/// [`consume_arrival`] bookkeeping the authoritative mover runs (server#65).
#[allow(clippy::type_complexity)]
pub fn predict_movement(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<
        (Entity, &MoveGoal, &MoveSpeed, Option<&TurnRate>, &mut Transform, Option<&mut MovePath>),
        With<Predicted>,
    >,
) {
    let dt = time.delta_secs();
    for (entity, goal, speed, turn, mut transform, mut path) in &mut q {
        let pos = Vec2::new(transform.translation.x, transform.translation.z);
        let cur_yaw = yaw_of(&transform);
        let turn_rate = turn_rate_of(turn);
        let target = steer_target(path.as_deref(), goal.0);
        let step = advance_mover(pos, cur_yaw, target, speed.0.max(0.0), turn_rate, dt);
        transform.translation.x = step.pos.x;
        transform.translation.z = step.pos.y;
        if step.pos != pos {
            transform.rotation = Quat::from_rotation_y(step.facing);
        }
        if consume_arrival(path.as_deref_mut(), step.arrived) {
            commands.entity(entity).remove::<MoveGoal>();
            commands.entity(entity).remove::<MovePath>();
        }
    }
}

/// Route a freshly ordered predicted mover around the map, the mirror of the
/// server's own planning pass (stormlight/server#65).
///
/// Runs on a changed goal only — pathing is a per-order cost, not a per-tick one —
/// and against the [`ActiveNavMesh`] the client baked from the very same descriptor
/// the server did, so the corridor is the same corridor. Without a map (or without
/// a route) the mover keeps no corridor and walks the goal directly, which is
/// exactly how it behaved before any geometry existed.
#[allow(clippy::type_complexity)] // A Bevy query's data+filter tuple; idiomatic.
pub fn plan_predicted_paths(
    map: Option<Res<ActiveNavMesh>>,
    mut commands: Commands,
    movers: Query<(Entity, &MoveGoal, &Transform), (Changed<MoveGoal>, With<Predicted>)>,
) {
    for (entity, goal, transform) in &movers {
        let from = Vec2::new(transform.translation.x, transform.translation.z);
        match plan_path(map.as_deref(), from, goal.0) {
            Some(path) => commands.entity(entity).insert(path),
            None => commands.entity(entity).remove::<MovePath>(),
        };
    }
}

/// Client-side plugin (server#50): predict the controlled unit's motion in
/// `FixedUpdate` so it responds with zero perceived latency, reconciled against the
/// authoritative server `Transform` by Lightyear's rollback. Added by the client
/// (and the prediction integration test), **never** the server — the server is the
/// authority the prediction reconciles against.
///
/// Planning is chained before walking, exactly as the server chains its own two
/// passes: a corridor computed after the step would leave the mover one tick of
/// straight-line travel ahead of the route it is supposed to be on.
pub struct PredictedMovementPlugin;

impl Plugin for PredictedMovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (plan_predicted_paths, predict_movement).chain());
    }
}
