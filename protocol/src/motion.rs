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
use serde::{Deserialize, Serialize};

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
