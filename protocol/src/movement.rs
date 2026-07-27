//! Player **move order** on the wire (client→server).
//!
//! A MOBA move command: the client asks its controlled unit to walk to a ground
//! point. Only a request — the server is authoritative: it resolves the sender to
//! the unit it controls (`ControlledBy`) and moves that unit itself, replicating
//! the resulting `Transform` back like any other mover (so motion is as smooth as
//! the `cube_demo` cubes). Reliable, unordered: a dropped order would strand the
//! unit, but two orders need no mutual ordering — the latest simply wins.
//!
//! Content-free: a bare world-space point, nothing hero- or ability-specific.
//! What "move" costs or how fast is the unit's own generic movement, server-side.

use core::f32::consts::{PI, TAU};

use bevy::math::Vec3;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Client→server request to move the sender's controlled unit to `target`
/// (world-space ground point). Only a request: the server validates ownership and
/// moves the unit authoritatively; a sender that controls nothing is ignored.
#[derive(Event, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveOrder {
    /// The world-space ground point to walk to.
    pub target: Vec3,
}

/// Reliable channel the move orders ride. Unordered-reliable: every order is
/// delivered (a lost move feels broken) but two orders need no ordering — the
/// last one the server applies wins.
pub struct MoveOrderChannel;

/// Register the move-order wire contract on both ends: the reliable client→server
/// [`MoveOrder`] channel + event. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte. No entity mapping — the payload is a pure point.
pub fn register(app: &mut App) {
    app.add_channel::<MoveOrderChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);

    app.register_event::<MoveOrder>().add_direction(NetworkDirection::ClientToServer);

    // Replicate the fallback move speed and enable prediction on it
    // (stormlight/server#50): a client predicting the unit it controls needs the
    // authoritative speed on its ghost to advance at the right rate. Inert on the
    // server (no prediction runs there).
    app.register_component::<MoveSpeed>().add_prediction();
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
    if dist <= step || dist == 0.0 {
        goal
    } else {
        pos + to * (step / dist)
    }
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
    if d.abs() <= max {
        wrap_angle(target)
    } else {
        wrap_angle(from + d.signum() * max)
    }
}

/// The yaw (rotation about `+Y`) whose forward (`+X`) points along the ground
/// direction `dir = (x, z)`: `Quat::from_rotation_y(yaw_to(dir)) * Vec3::X` lies
/// along `dir`. Returns `0.0` for a (near-)zero vector — the caller keeps the
/// current facing when a unit is not moving.
#[must_use]
pub fn yaw_to(dir: Vec2) -> f32 {
    if dir.length_squared() <= f32::EPSILON {
        0.0
    } else {
        (-dir.y).atan2(dir.x)
    }
}

/// Turn `facing` toward the direction a mover actually travelled this tick, at
/// `turn_rate` radians/second. A mover that did not move keeps its heading —
/// there is no travel direction to face, and snapping to a default would spin a
/// resting unit. Pure.
///
/// Split out of [`advance_mover`] because a mover's realized travel is not always
/// its straight step toward the goal: local avoidance (server#51) steers it, and
/// the unit must face where it *went*, not where it wanted to go.
#[must_use]
pub fn face_travel(facing: f32, travel: Vec2, turn_rate: f32, dt: f32) -> f32 {
    let desired = if travel.length_squared() > f32::EPSILON {
        yaw_to(travel)
    } else {
        facing
    };
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
/// `Vec2(x, z)`). Set authoritatively server-side from a player `MoveOrder`, and
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

/// How fast a unit may turn, radians/second. Absent ⇒ instant facing. Generic; a
/// mod/stat can drive it later.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRate(pub f32);

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
#[allow(clippy::type_complexity)]
pub fn predict_movement(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<
        (Entity, &MoveGoal, &MoveSpeed, Option<&TurnRate>, &mut Transform),
        With<Predicted>,
    >,
) {
    let dt = time.delta_secs();
    for (entity, goal, speed, turn, mut transform) in &mut q {
        let pos = Vec2::new(transform.translation.x, transform.translation.z);
        let cur_yaw = yaw_of(&transform);
        let turn_rate = turn.map_or(f32::INFINITY, |t| t.0);
        let step = advance_mover(pos, cur_yaw, goal.0, speed.0.max(0.0), turn_rate, dt);
        transform.translation.x = step.pos.x;
        transform.translation.z = step.pos.y;
        if step.pos != pos {
            transform.rotation = Quat::from_rotation_y(step.facing);
        }
        if step.arrived {
            commands.entity(entity).remove::<MoveGoal>();
        }
    }
}

/// Client-side plugin (server#50): predict the controlled unit's motion in
/// `FixedUpdate` so it responds with zero perceived latency, reconciled against the
/// authoritative server `Transform` by Lightyear's rollback. Added by the client
/// (and the prediction integration test), **never** the server — the server is the
/// authority the prediction reconciles against.
pub struct PredictedMovementPlugin;

impl Plugin for PredictedMovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, predict_movement);
    }
}
