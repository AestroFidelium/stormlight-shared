//! The law a unit moves along a declared path by (stormlight/server#213).
//!
//! A motion is a path of legs travelled at a speed. The server advances it a tick
//! at a time; the client that predicts the moving unit recomputes it from its
//! start. Both run this one law over the same resolved [`MotionPath`], so they
//! cannot disagree about where the unit is.
//!
//! # Why a cursor, and why it never rests at a leg's end
//!
//! A [`Cursor`] is the position *along* the path: which round, which leg, how far
//! into it. Where the unit is follows from the leg's own start and the distance into
//! it — never from adding up per-tick steps — so nothing drifts however many ticks a
//! motion lasts.
//!
//! The cursor is kept **canonical**: it never rests at the very end of a leg, but
//! steps into the next one at once (turning by that leg's turn). Otherwise a tick
//! that happened to end exactly on a corner would face one way and an advance that
//! ran through the corner would face another, and the one property the whole
//! mechanism needs — splitting an advance into pieces changes nothing — would fail
//! on exactly the paths authors draw corners into.
//!
//! # The trace
//!
//! What a motion *meets* is decided on the ground it covered, not where it stopped
//! (stormlight/server#211). So an advance also hands back the polyline it travelled:
//! one segment per straight stretch, and chords for a curve, short enough that none
//! strays from the curve by more than [`CHORD_SAGITTA`]. A wall or a unit beside a
//! curve is therefore met where it is, at any speed.
//!
//! Angles are radians here, positive counter-clockwise on the ground plane; the
//! ABI's degrees are converted when the motion is resolved.

use core::f32::consts::PI;
use core::num::NonZeroU32;

use bevy::prelude::*;
use lightyear::prelude::{LocalTimeline, Predicted, Tick};
use serde::{Deserialize, Serialize};
use stormlight_navigation::{ActiveNavMesh, NavMesh, WallHit};

use crate::connection::tick_duration;
use crate::movement::yaw_to;
use crate::pace_log::PaceLog;
use crate::time_scale::TimeScale;

/// The furthest a chord of a curve may stray from the curve, in world units. Below
/// any contact radius a unit has, so cutting a corner of a curve never decides
/// whether something on it was met.
pub const CHORD_SAGITTA: f32 = 0.02;

/// The most a single chord may turn through, radians. On a very tight curve the
/// sagitta alone would allow a chord across the whole circle; this keeps a chord a
/// faithful stand-in for the arc's length too (a chord this wide is within 0.3% of
/// its arc).
const MAX_CHORD_TURN: f32 = PI / 12.0;

/// The most legs an advance may step through, and the most chords it may hand back.
/// Bounds a tick however absurd the path or the speed: an endless motion pointed a
/// million units round a small ring stops tracing in fine detail rather than
/// stalling the simulation.
///
/// The leg bound is the one place the law gives up exactness: an advance that
/// would step through more legs than this stops there, so it lands short of where
/// the same distance in smaller pieces would. Each step is a handful of flops, so
/// the bound sits far beyond anything a tick at a playable speed reaches.
const MAX_LEG_STEPS: u32 = 65_536;
const MAX_CHORDS: usize = 8192;

/// How close to a leg's end counts as its end. Far below a tick's travel at any
/// speed a unit moves at, far above f32 noise on a leg a few hundred units long.
const LEG_EPS: f32 = 1e-5;

/// Below this curvature a curve is drawn as the straight line it all but is.
const STRAIGHT: f32 = 1e-6;

/// One leg of a resolved path: numbers, no expressions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathLeg {
    /// Turn by `turn`, then go straight for `dist`.
    Straight { turn: f32, dist: f32 },
    /// Turn by `turn`, then curve for `dist` while the heading turns by `bend`.
    Arc { turn: f32, dist: f32, bend: f32 },
    /// Straight back to the path's start.
    Back,
    /// Straight to this ground point.
    To(Vec2),
}

/// How many times a path is travelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounds {
    /// This many times through.
    Times(NonZeroU32),
    /// Until something outside the path ends it. A round that covers no ground at
    /// all ends it too: there is nothing to go round.
    Endless,
}

/// A motion's path, fully resolved: where it starts, which way it faces, its legs
/// and how many times it travels them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionPath {
    pub start: Vec2,
    /// Unit vector the first leg turns from.
    pub heading: Vec2,
    pub legs: Box<[PathLeg]>,
    pub rounds: Rounds,
}

/// The leg a cursor is in, as geometry: where it began, which way it set off, how
/// long it is and how sharply it curves.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stretch {
    origin: Vec2,
    heading: Vec2,
    length: f32,
    /// Radians of heading per unit travelled; zero for a straight stretch.
    curvature: f32,
}

impl Stretch {
    /// Where the stretch is `s` along it, and which way it faces there.
    fn at(&self, s: f32) -> (Vec2, Vec2) {
        if self.curvature.abs() < STRAIGHT {
            return (self.origin + self.heading * s, self.heading);
        }
        let facing = Vec2::from_angle(self.curvature * s).rotate(self.heading);
        // ∫ heading(t) dt over [0, s] for a heading turning at a constant rate.
        let displacement =
            Vec2::new(facing.y - self.heading.y, self.heading.x - facing.x) / self.curvature;
        (self.origin + displacement, facing)
    }
}

/// How far along a [`MotionPath`] a motion has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    round: u32,
    leg: usize,
    into: f32,
    stretch: Stretch,
    /// Kept in f64: it is a sum of every piece a motion was ever advanced by, and
    /// in f32 a long motion's total would drift with how it was split.
    travelled: f64,
    /// What the current round has covered, so an endless path whose rounds cover
    /// nothing can be told apart from one that goes somewhere.
    round_travelled: f32,
    finished: bool,
}

impl Cursor {
    /// The cursor at the start of `path`, already stepped into its first leg that
    /// has any length — or finished, if no leg has any.
    #[must_use]
    pub fn start(path: &MotionPath) -> Self {
        let heading = path.heading.try_normalize().unwrap_or(Vec2::X);
        let mut cursor = Self {
            round: 0,
            leg: 0,
            into: 0.0,
            stretch: Stretch { origin: path.start, heading, length: 0.0, curvature: 0.0 },
            travelled: 0.0,
            round_travelled: 0.0,
            finished: path.legs.is_empty(),
        };
        if !cursor.finished {
            cursor.enter(path);
            let mut steps = 0;
            cursor.settle(path, &mut steps);
        }
        cursor
    }

    /// Where the moving unit is.
    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.stretch.at(self.into).0
    }

    /// Which way it is heading — the way the leg it is in carries it.
    #[must_use]
    pub fn heading(&self) -> Vec2 {
        self.stretch.at(self.into).1
    }

    /// How far along the path it has come, in all.
    #[must_use]
    pub fn travelled(&self) -> f32 {
        #[allow(clippy::cast_possible_truncation)]
        let travelled = self.travelled as f32;
        travelled
    }

    /// Whether the path has run out.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// How much of the leg it is in is left to travel.
    #[must_use]
    pub fn leg_left(&self) -> f32 {
        (self.stretch.length - self.into).max(0.0)
    }

    /// Glance off a wall whose outward normal is `normal` (stormlight/server#215):
    /// the rest of the current leg is travelled mirrored in the wall — the heading
    /// reflected, a curve's bend turned the other way, its remaining length kept.
    /// What follows the leg follows on from where the mirrored leg ends.
    pub fn reflect(&mut self, path: &MotionPath, normal: Vec2) {
        let Some(normal) = normal.try_normalize() else { return };
        if self.finished {
            return;
        }
        let (origin, facing) = self.stretch.at(self.into);
        let heading =
            (facing - 2.0 * facing.dot(normal) * normal).try_normalize().unwrap_or(facing);
        self.stretch = Stretch {
            origin,
            heading,
            length: self.leg_left(),
            curvature: -self.stretch.curvature,
        };
        self.into = 0.0;
        let mut steps = 0;
        self.settle(path, &mut steps);
    }

    /// Move `distance` further along `path`, appending the ground covered to
    /// `trace` as a polyline that starts where the unit was and ends where it now
    /// is. Nothing is appended when nothing is covered.
    pub fn advance(&mut self, path: &MotionPath, distance: f32, trace: &mut Vec<Vec2>) {
        // What is left to cover is counted in f64. It starts as the whole advance
        // and loses a leg's length at a time, and in f32 each subtraction from a
        // large remainder rounds away more than a short leg's worth of precision —
        // a thousand-unit advance round a small ring then lands visibly short of
        // the same distance covered in pieces.
        let mut left = if distance.is_finite() { f64::from(distance.max(0.0)) } else { 0.0 };
        if self.finished || left <= 0.0 {
            return;
        }
        let first = trace.len();
        trace.push(self.position());
        let mut steps = 0;
        while !self.finished && left > 0.0 && steps < MAX_LEG_STEPS {
            let room = f64::from((self.stretch.length - self.into).max(0.0));
            #[allow(clippy::cast_possible_truncation)]
            let go = left.min(room) as f32;
            self.chords(self.into, self.into + go, first, trace);
            self.into += go;
            self.travelled += left.min(room);
            self.round_travelled += go;
            left -= left.min(room);
            steps += 1;
            self.settle(path, &mut steps);
        }
        if trace.len() == first + 1 {
            trace.truncate(first);
        }
    }

    /// Step past every leg the cursor is at the end of, so it rests inside a leg
    /// with ground left in it, or finished.
    fn settle(&mut self, path: &MotionPath, steps: &mut u32) {
        while !self.finished && self.stretch.length - self.into <= LEG_EPS {
            if *steps >= MAX_LEG_STEPS {
                // A path of nothing but empty legs in an endless loop; see
                // `Rounds::Endless`.
                self.finished = true;
                return;
            }
            *steps += 1;
            // A finished leg hands the next one its exact end. The cursor stops
            // within `LEG_EPS` of it by however the distance happened to be split,
            // and entering from there tilts the next leg by the curvature times that
            // shortfall — nothing on one corner, a visible drift over hundreds of
            // rounds of an endless curve, and different on the two ends of the wire
            // if they split the same distance differently.
            self.into = self.stretch.length;
            self.leg += 1;
            if self.leg >= path.legs.len() {
                self.leg = 0;
                self.round += 1;
                let done = match path.rounds {
                    Rounds::Times(times) => self.round >= times.get(),
                    Rounds::Endless => self.round_travelled <= LEG_EPS,
                };
                self.round_travelled = 0.0;
                if done {
                    // Rest exactly at the end of the leg it finished on.
                    self.into = self.stretch.length;
                    self.finished = true;
                    return;
                }
            }
            self.enter(path);
        }
    }

    /// Begin the current leg from wherever the previous one ended.
    fn enter(&mut self, path: &MotionPath) {
        let (origin, facing) = self.stretch.at(self.into);
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        // A leg already at its destination keeps the heading it arrived with. Within
        // `LEG_EPS` counts as there: a cursor back at its start after a round of
        // float arithmetic sits a few ulps off it, and normalising that offset would
        // point the unit in an arbitrary direction.
        let toward = |point: Vec2| {
            let offset = point - origin;
            let length = offset.length();
            if length <= LEG_EPS { (facing, 0.0) } else { (offset / length, length) }
        };
        let (heading, length, curvature) = match path.legs.get(self.leg) {
            Some(PathLeg::Straight { turn, dist }) => {
                (Vec2::from_angle(finite(*turn)).rotate(facing), finite(*dist).max(0.0), 0.0)
            }
            Some(PathLeg::Arc { turn, dist, bend }) => {
                let length = finite(*dist).max(0.0);
                let curvature = if length > 0.0 { finite(*bend) / length } else { 0.0 };
                (Vec2::from_angle(finite(*turn)).rotate(facing), length, curvature)
            }
            Some(PathLeg::Back) => {
                let (heading, length) = toward(path.start);
                (heading, length, 0.0)
            }
            Some(PathLeg::To(point)) => {
                let (heading, length) = toward(*point);
                (heading, length, 0.0)
            }
            None => (facing, 0.0, 0.0),
        };
        self.stretch = Stretch { origin, heading, length, curvature };
        self.into = 0.0;
    }

    /// Append the chords covering `[from, to]` of the current stretch.
    fn chords(&self, from: f32, to: f32, first: usize, trace: &mut Vec<Vec2>) {
        if to <= from {
            return;
        }
        let curvature = self.stretch.curvature.abs();
        let pieces = if curvature < STRAIGHT {
            1
        } else {
            let radius = 1.0 / curvature;
            // The widest chord whose middle stays within the sagitta of the arc.
            let by_sagitta = if radius > CHORD_SAGITTA {
                2.0 * (1.0 - CHORD_SAGITTA / radius).acos()
            } else {
                MAX_CHORD_TURN
            };
            let widest = by_sagitta.clamp(f32::EPSILON, MAX_CHORD_TURN);
            let room = MAX_CHORDS.saturating_sub(trace.len() - first).max(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let wanted = ((to - from) * curvature / widest).ceil() as usize;
            wanted.clamp(1, room)
        };
        #[allow(clippy::cast_precision_loss)]
        let step = (to - from) / pieces as f32;
        for i in 1..=pieces {
            #[allow(clippy::cast_precision_loss)]
            let s = if i == pieces { to } else { from + step * i as f32 };
            trace.push(self.stretch.at(s).0);
        }
    }
}

/// What a wall does to a motion, in numbers (stormlight/server#215).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Walls {
    /// Ends against the first wall.
    Stop,
    /// Glances off walls, this many more times; then stops at the next.
    Bounce(u32),
    /// Goes through walls.
    Pass,
}

/// Why a tick of motion ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepEnd {
    /// Its path ran out, or its time did.
    Ran,
    /// A wall stopped it.
    Wall,
}

/// What one tick of a motion came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stepped {
    /// Walls met this tick: the one that stopped it, and every ricochet.
    pub collisions: u32,
    /// Whether, and why, the motion ended this tick.
    pub ended: Option<StepEnd>,
    /// The share of the tick it was moving in — less than all of it when it ended
    /// partway through.
    pub span: f32,
}

/// A motion under way: its path and where along it, its pace, its time limit and
/// what walls do to it (stormlight/server#216).
///
/// One tick of it is [`Self::step`], which the authority runs and a predicting
/// client runs identically, piece for piece: the same per-tick distance, the same
/// walls, the same ricochets. Recomputing a motion from its total distance instead
/// is *not* the same — paths that re-aim at fixed points are chaotic in float — so
/// both ends step it.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionState {
    path: MotionPath,
    cursor: Cursor,
    speed: f32,
    limit: Option<f32>,
    elapsed: f32,
    walls: Walls,
}

impl MotionState {
    /// A motion about to start along `path`.
    #[must_use]
    pub fn new(path: MotionPath, speed: f32, limit: Option<f32>, walls: Walls) -> Self {
        let cursor = Cursor::start(&path);
        let limit = limit.filter(|l| l.is_finite()).map(|l| l.max(0.0));
        Self { path, cursor, speed, limit, elapsed: 0.0, walls }
    }

    /// Where the moving unit is.
    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.cursor.position()
    }

    /// Which way it is heading.
    #[must_use]
    pub fn heading(&self) -> Vec2 {
        self.cursor.heading()
    }

    /// What walls do to it now — with the ricochets it has left.
    #[must_use]
    pub fn walls(&self) -> Walls {
        self.walls
    }

    /// Advance by one tick of `dt` seconds against `navmesh`, appending the ground
    /// covered to `trace` (cut short at a wall it stops at).
    ///
    /// A tick is travelled one pass per wall met: a ricochet rewinds to where the
    /// cursor came to rest against the wall, reflects it off the wall's normal and
    /// spends the rest of the tick from there (stormlight/server#215). Each pass
    /// spends a ricochet, so a tick does bounded work.
    pub fn step(&mut self, navmesh: Option<&NavMesh>, dt: f32, trace: &mut Vec<Vec2>) -> Stepped {
        // A time limit cuts the tick's travel to the time that was left.
        let time = self.limit.map_or(dt, |limit| dt.min((limit - self.elapsed).max(0.0)));
        self.elapsed += dt;
        let timed_out = self.limit.is_some_and(|limit| self.elapsed >= limit - f32::EPSILON);
        let first = trace.len();
        let mut collisions = 0;
        let mut ended = None;
        let mut left = self.speed * time;
        let mut piece = Vec::new();
        loop {
            let before = self.cursor;
            piece.clear();
            self.cursor.advance(&self.path, left, &mut piece);
            let hit = match (navmesh, self.walls) {
                (Some(map), Walls::Stop | Walls::Bounce(_)) => first_wall(map, &piece),
                _ => None,
            };
            let Some((along, wall)) = hit else {
                extend(trace, &piece);
                break;
            };
            collisions += 1;
            match (self.walls, wall.normal) {
                (Walls::Bounce(remaining), Some(normal)) if remaining > 0 => {
                    self.walls = Walls::Bounce(remaining - 1);
                    // Back to where the wall was met, then off it.
                    self.cursor = before;
                    piece.clear();
                    self.cursor.advance(&self.path, along, &mut piece);
                    extend(trace, &piece);
                    self.cursor.reflect(&self.path, normal);
                    left -= along;
                    if left <= 0.0 || self.cursor.finished() {
                        break;
                    }
                }
                _ => {
                    // Keep the ground covered up to the wall, so what stood before
                    // it is met.
                    let reached = piece.len().min(segment_of(&piece, along) + 1);
                    piece.truncate(reached);
                    piece.push(wall.at);
                    extend(trace, &piece);
                    ended = Some(StepEnd::Wall);
                    break;
                }
            }
        }
        if ended.is_none() && (self.cursor.finished() || timed_out) {
            ended = Some(StepEnd::Ran);
        }
        let covered: f32 =
            trace.get(first..).unwrap_or_default().windows(2).map(|w| w[0].distance(w[1])).sum();
        let pace = self.speed * dt;
        let span = if pace > 0.0 { (covered / pace).clamp(0.0, 1.0) } else { 1.0 };
        Stepped { collisions, ended, span }
    }
}

/// The first wall `piece` runs into, and how far along it the move comes to rest
/// against it.
fn first_wall(map: &NavMesh, piece: &[Vec2]) -> Option<(f32, WallHit)> {
    let mut covered = 0.0;
    for w in piece.windows(2) {
        let (from, to) = (w[0], w[1]);
        if let Some(hit) = map.wall_hit(from, to) {
            return Some((covered + from.distance(hit.at), hit));
        }
        covered += from.distance(to);
    }
    None
}

/// Which segment of `piece` the point `along` it falls in.
fn segment_of(piece: &[Vec2], along: f32) -> usize {
    let mut covered = 0.0;
    for (i, w) in piece.windows(2).enumerate() {
        covered += w[0].distance(w[1]);
        if along <= covered {
            return i;
        }
    }
    piece.len().saturating_sub(2)
}

/// Append `piece` to `trace`, without repeating the point they share.
fn extend(trace: &mut Vec<Vec2>, piece: &[Vec2]) {
    let skip = usize::from(trace.last().is_some() && trace.last() == piece.first());
    trace.extend(piece.iter().skip(skip));
}

/// A motion under way, as the server publishes it (stormlight/server#216): enough
/// for a predicting client to step it exactly as the authority does — the path,
/// the pace, the time limit, the walls, and the tick it started on.
///
/// Replicated plainly, never predicted: it is a fact the server decides, and its
/// removal (the motion ended or was cut off) reaches the client as an ordinary
/// update.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionFact {
    pub path: MotionPath,
    pub speed: f32,
    pub limit: Option<f32>,
    pub walls: Walls,
    /// The tick the motion began on; it is first stepped the tick after.
    pub started: Tick,
}

impl MotionFact {
    /// The motion as it stood when it began.
    #[must_use]
    pub fn state(&self) -> MotionState {
        MotionState::new(self.path.clone(), self.speed, self.limit, self.walls)
    }
}

/// How far a predicting client has stepped the motion it follows — a cache, so a
/// tick costs one step rather than a replay from the start.
#[derive(Component, Clone, Debug)]
pub struct FollowedMotion {
    fact: MotionFact,
    /// The pace record it was stepped by. A change to it — a stop the server saw
    /// after this client had already stepped past it — is a replay from the start.
    log: Option<PaceLog>,
    state: MotionState,
    steps: u32,
    /// Where the unit rests: the end of its last tick's travel, which is short of
    /// the cursor when a wall stopped it.
    rest: Vec2,
    ended: bool,
}

impl FollowedMotion {
    fn new(fact: &MotionFact, log: Option<&PaceLog>) -> Self {
        let state = fact.state();
        let rest = fact.path.start;
        Self { fact: fact.clone(), log: log.cloned(), state, steps: 0, rest, ended: false }
    }

    /// Step up to `steps` ticks in, by the authority's own pieces — each tick at
    /// the pace `log` says it ran at (stormlight/server#225), and a stopped tick
    /// not at all, as the authority skips it.
    fn catch_up(
        &mut self,
        steps: u32,
        log: Option<&PaceLog>,
        navmesh: Option<&NavMesh>,
        trace: &mut Vec<Vec2>,
    ) {
        while self.steps < steps && !self.ended {
            self.steps += 1;
            let tick = self.fact.started + i16::try_from(self.steps).unwrap_or(i16::MAX);
            let pace = log.map_or(TimeScale::NORMAL, |log| log.pace_at(tick));
            let dt = pace.apply(tick_duration().as_secs_f32());
            if dt <= 0.0 {
                continue;
            }
            trace.clear();
            let stepped = self.state.step(navmesh, dt, trace);
            if let Some(last) = trace.last() {
                self.rest = *last;
            }
            self.ended = stepped.ended.is_some();
        }
    }
}

/// The most ticks a client will step a motion to catch up in one go. A fact that
/// claims to be older than this is a clock out of step, not a motion to replay —
/// a minute of play, far past any motion's natural length.
const MAX_CATCH_UP: u32 = 64 * 60;

/// A predicted unit carrying a motion: the fact, how far it has been followed, its
/// place, and the record of the pace each tick ran at (stormlight/server#225).
type Follower<'a> = (
    Entity,
    &'a MotionFact,
    Option<&'a mut FollowedMotion>,
    &'a mut Transform,
    Option<&'a PaceLog>,
);

/// **Prediction**: put every predicted unit carrying a motion where the motion has
/// it on this tick (stormlight/server#216) — stepped from the published fact by
/// the same law and the same per-tick pieces the authority uses.
///
/// A rollback re-runs earlier ticks; the cache is ahead of them then, so the motion
/// is replayed from its start up to the earlier tick. Every tick is stepped at the
/// pace it ran at, so a motion through a time field is followed exactly too.
pub fn follow_motions(
    // Optional: an app with no timeline has no ticks to follow a motion by.
    timeline: Option<Res<LocalTimeline>>,
    navmesh: Option<Res<ActiveNavMesh>>,
    mut commands: Commands,
    mut movers: Query<Follower, With<Predicted>>,
) {
    let Some(timeline) = timeline else { return };
    let mut trace = Vec::new();
    for (entity, fact, followed, mut transform, log) in &mut movers {
        let elapsed = timeline.tick() - fact.started;
        let steps = u32::try_from(elapsed).unwrap_or(0).min(MAX_CATCH_UP);
        let map = navmesh.as_deref().map(|m| &m.0);
        match followed {
            Some(mut followed)
                if followed.fact == *fact
                    && followed.log.as_ref() == log
                    && followed.steps <= steps =>
            {
                followed.catch_up(steps, log, map, &mut trace);
                place(&followed, &mut transform);
            }
            _ => {
                let mut fresh = FollowedMotion::new(fact, log);
                fresh.catch_up(steps, log, map, &mut trace);
                place(&fresh, &mut transform);
                commands.entity(entity).insert(fresh);
            }
        }
    }
}

/// Put the unit where the motion it follows has it.
fn place(follow: &FollowedMotion, transform: &mut Transform) {
    transform.translation.x = follow.rest.x;
    transform.translation.z = follow.rest.y;
    transform.rotation = Quat::from_rotation_y(yaw_to(follow.state.heading()));
}

/// Forget the cache of a motion whose fact is gone.
pub fn forget_ended_motions(
    mut commands: Commands,
    stale: Query<Entity, (With<FollowedMotion>, Without<MotionFact>)>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<FollowedMotion>();
    }
}
