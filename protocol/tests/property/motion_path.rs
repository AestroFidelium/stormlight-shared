//! The law a unit moves along a declared path by (stormlight/server#213).
//!
//! A motion is advanced a tick at a time by the server and recomputed from its
//! start by the client, so the one property everything rests on is that **how the
//! distance is split does not matter**: one advance of `a + b` and two advances of
//! `a` then `b` put the unit in the same place, facing the same way, with the same
//! amount of path left. Beside it:
//!
//! - the distance travelled along the path is the distance asked for, until the
//!   path runs out — a motion neither gains nor loses ground between ticks;
//! - the trace an advance hands back follows the path: a straight leg is one
//!   segment, and a curve is chords that never stray from it by more than
//!   [`CHORD_SAGITTA`], so a wall or a unit beside a curve is met where it is;
//! - `Back` ends where the motion started, a curve turns by exactly its bend, and a
//!   full ring comes back round to where it began;
//! - every advance terminates, even an endless motion whose rounds cover no ground.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use core::num::NonZeroU32;
use stormlight_shared::motion::{CHORD_SAGITTA, Cursor, MotionPath, PathLeg, Rounds};

/// Positions agree to this, in world units: a few f32 ulps over a path a few
/// hundred units long, travelled in up to a few hundred pieces.
const CLOSE: f32 = 2e-3;

/// A leg, generated in whole degrees and tenths of a unit so every run is finite.
///
/// Straight and curved legs are at least a unit long. Shorter ones are legal, but
/// a path of tenth-unit legs travelled thousands of units in one advance steps
/// through more legs than the law's per-advance bound allows — the one documented
/// place it trades exactness for a bounded tick.
#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Leg {
    Straight {
        #[generator(-180i16..=180)]
        turn: i16,
        #[generator(10u16..=400)]
        tenths: u16,
    },
    Arc {
        #[generator(-180i16..=180)]
        turn: i16,
        #[generator(10u16..=400)]
        tenths: u16,
        #[generator(-720i16..=720)]
        bend: i16,
    },
    Back,
    To {
        #[generator(-300i16..=300)]
        x: i16,
        #[generator(-300i16..=300)]
        y: i16,
    },
}

impl Leg {
    fn resolved(self) -> PathLeg {
        let deg = |d: i16| f32::from(d).to_radians();
        let units = |t: u16| f32::from(t) / 10.0;
        match self {
            Self::Straight { turn, tenths } => {
                PathLeg::Straight { turn: deg(turn), dist: units(tenths) }
            }
            Self::Arc { turn, tenths, bend } => {
                PathLeg::Arc { turn: deg(turn), dist: units(tenths), bend: deg(bend) }
            }
            Self::Back => PathLeg::Back,
            Self::To { x, y } => PathLeg::To(Vec2::new(f32::from(x) / 10.0, f32::from(y) / 10.0)),
        }
    }
}

#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Times {
    Once,
    Thrice,
    Endless,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Leg>>().with().len(0usize..=5))]
    legs: Vec<Leg>,
    times: Times,
    #[generator(0u16..=359)]
    heading: u16,
    /// How the advance is split, in tenths of a unit per piece.
    #[generator(bolero::produce::<Vec<u16>>().with().len(1usize..=40).values(0u16..=600))]
    pieces: Vec<u16>,
}

impl Scenario {
    fn path(&self) -> MotionPath {
        MotionPath {
            start: Vec2::new(3.0, -2.0),
            heading: Vec2::from_angle(f32::from(self.heading).to_radians()),
            legs: self.legs.iter().map(|l| l.resolved()).collect(),
            rounds: match self.times {
                Times::Once => Rounds::Times(NonZeroU32::MIN),
                Times::Thrice => Rounds::Times(NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN)),
                Times::Endless => Rounds::Endless,
            },
        }
    }

    fn pieces(&self) -> impl Iterator<Item = f32> + '_ {
        self.pieces.iter().map(|p| f32::from(*p) / 10.0)
    }
}

/// The length of a polyline.
fn length(trace: &[Vec2]) -> f32 {
    trace.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Whether the path re-aims at fixed points round after round, forever.
///
/// A `Back` or `To` leg aims at an absolute point, so its heading is the direction
/// to that point from wherever the previous leg ended — and when that is very near
/// the point, a rounding-sized difference in position swings the heading through a
/// large angle. Once, that is a rounding error; in an endless loop that re-aims
/// every round it compounds chaotically, so two ways of summing the same distance
/// can diverge by whole units. That is the geometry, not the law, and it is why
/// both ends advance a motion by the same per-tick pieces rather than recomputing it
/// from a total.
fn re_aims_forever(s: &Scenario) -> bool {
    matches!(s.times, Times::Endless)
        && s.legs.iter().any(|l| matches!(l, Leg::Back | Leg::To { .. }))
}

#[test]
fn splitting_an_advance_changes_nothing() {
    check!().with_type::<Scenario>().for_each(|s| {
        if re_aims_forever(s) {
            return;
        }
        let path = s.path();
        let total: f32 = s.pieces().sum();

        let mut whole = Cursor::start(&path);
        whole.advance(&path, total, &mut Vec::new());

        let mut split = Cursor::start(&path);
        for piece in s.pieces() {
            split.advance(&path, piece, &mut Vec::new());
        }
        // Every corner the path turns rounds once, and over hundreds of rounds of an
        // endless path that adds up — linearly, in proportion to the ground covered.
        // Four millionths of a unit per unit is half of what the f32 remainder bug
        // this test caught cost (8.7e-6), so that bug would still be seen.
        let close = CLOSE + total * 4e-6;

        assert!(
            whole.position().distance(split.position()) < close,
            "one advance of {total} ended at {}, the same in pieces at {}",
            whole.position(),
            split.position(),
        );
        // Heading is discontinuous at a corner: a cursor a hair short of one still
        // faces along the leg it is finishing, a hair past it faces the next. Two
        // ways of summing the same distance can land on either side, so a mismatch
        // is allowed only if it is that — and a step on, past the corner, they agree.
        if whole.heading().distance(split.heading()) >= close {
            let (mut a, mut b) = (whole, split);
            a.advance(&path, 0.01, &mut Vec::new());
            b.advance(&path, 0.01, &mut Vec::new());
            assert!(
                a.heading().distance(b.heading()) < close,
                "one advance faces {}, the same in pieces faces {}, and not because they \
                 sit either side of one corner",
                whole.heading(),
                split.heading(),
            );
        }
        assert_eq!(whole.finished(), split.finished(), "one finished and the other did not");
        assert!(
            (whole.travelled() - split.travelled()).abs() < close,
            "one advance covered {}, the same in pieces {}",
            whole.travelled(),
            split.travelled(),
        );
    });
}

#[test]
fn a_motion_covers_exactly_the_ground_it_is_given() {
    check!().with_type::<Scenario>().for_each(|s| {
        let path = s.path();
        let mut cursor = Cursor::start(&path);
        for piece in s.pieces() {
            let before = cursor.travelled();
            let mut trace = Vec::new();
            let was_finished = cursor.finished();
            cursor.advance(&path, piece, &mut trace);
            let covered = cursor.travelled() - before;

            if was_finished {
                assert!(covered.abs() < CLOSE, "a finished motion moved on by {covered}");
                continue;
            }
            if !cursor.finished() {
                assert!(
                    (covered - piece).abs() < CLOSE,
                    "asked to go {piece}, went {covered} with path to spare",
                );
            }
            assert!(covered <= piece + CLOSE, "asked to go {piece}, went {covered}");
            // A chord is never longer than the curve it cuts, and never much shorter.
            let drawn = length(&trace);
            // f32 noise grows with the number of chords summed.
            let slack = CLOSE + covered * 1e-4;
            assert!(
                drawn <= covered + slack,
                "the trace is longer ({drawn}) than the path ({covered})"
            );
            assert!(
                drawn >= covered * 0.98 - slack,
                "the trace ({drawn}) cuts the path ({covered}) short",
            );
            if let Some(last) = trace.last() {
                assert!(trace.len() >= 2, "a trace that moves has two ends");
                assert!(
                    last.distance(cursor.position()) < CLOSE,
                    "the trace does not end at the unit"
                );
            }
        }
    });
}

#[test]
fn every_chord_of_a_curve_stays_on_it() {
    check!().with_type::<Scenario>().for_each(|s| {
        let path = s.path();
        let mut cursor = Cursor::start(&path);
        let mut trace = Vec::new();
        cursor.advance(&path, s.pieces().sum(), &mut trace);
        // Replay the path finely and check each chord's midpoint lies within the
        // sagitta of the true path.
        let mut fine = Cursor::start(&path);
        let mut on_path = vec![fine.position()];
        while !fine.finished() && fine.travelled() < cursor.travelled() - CLOSE {
            fine.advance(&path, 0.05, &mut Vec::new());
            on_path.push(fine.position());
        }
        for chord in trace.windows(2) {
            let mid = (chord[0] + chord[1]) / 2.0;
            let nearest = on_path.iter().map(|p| p.distance(mid)).fold(f32::INFINITY, f32::min);
            assert!(
                nearest <= CHORD_SAGITTA + 0.05,
                "a chord's middle {mid} is {nearest} off the path it stands for",
            );
        }
    });
}

#[test]
fn back_returns_to_the_start_and_a_ring_closes() {
    check!().with_type::<Scenario>().for_each(|s| {
        let start = Vec2::new(3.0, -2.0);
        let heading = Vec2::from_angle(f32::from(s.heading).to_radians());

        let mut legs: Vec<PathLeg> = s.legs.iter().map(|l| l.resolved()).collect();
        legs.push(PathLeg::Back);
        let path = MotionPath {
            start,
            heading,
            legs: legs.into(),
            rounds: Rounds::Times(NonZeroU32::MIN),
        };
        let mut cursor = Cursor::start(&path);
        cursor.advance(&path, 1e6, &mut Vec::new());
        assert!(cursor.finished(), "a once-through path did not finish given a million units");
        assert!(
            cursor.position().distance(start) < CLOSE,
            "a path ending in Back stopped at {}, not where it began",
            cursor.position(),
        );

        let ring = MotionPath {
            start,
            heading,
            legs: vec![PathLeg::Arc { turn: 0.0, dist: 12.0, bend: core::f32::consts::TAU }].into(),
            rounds: Rounds::Times(NonZeroU32::MIN),
        };
        let mut cursor = Cursor::start(&ring);
        cursor.advance(&ring, 12.0, &mut Vec::new());
        assert!(
            cursor.position().distance(start) < CLOSE,
            "a full ring ended at {}",
            cursor.position()
        );
        assert!(cursor.heading().distance(heading) < CLOSE, "a full ring faces a new way");
    });
}

/// One curve on its own.
#[derive(Debug, TypeGenerator)]
struct Curve {
    #[generator(-720i16..=720)]
    bend: i16,
    #[generator(1u16..=400)]
    tenths: u16,
    #[generator(0u16..=359)]
    heading: u16,
}

#[test]
fn a_curve_turns_by_its_bend() {
    check!().with_type::<Curve>().for_each(|c| {
        let bend = f32::from(c.bend).to_radians();
        let dist = f32::from(c.tenths) / 10.0;
        let h = Vec2::from_angle(f32::from(c.heading).to_radians());
        let path = MotionPath {
            start: Vec2::ZERO,
            heading: h,
            legs: vec![PathLeg::Arc { turn: 0.0, dist, bend }].into(),
            rounds: Rounds::Times(NonZeroU32::MIN),
        };
        let mut cursor = Cursor::start(&path);
        cursor.advance(&path, dist, &mut Vec::new());
        let expected = Vec2::from_angle(bend).rotate(h);
        assert!(
            cursor.heading().distance(expected) < CLOSE,
            "a curve bending {bend} rad ended facing {}, expected {expected}",
            cursor.heading(),
        );
    });
}

#[test]
fn an_endless_motion_that_covers_no_ground_still_ends_its_advance() {
    let path = MotionPath {
        start: Vec2::ZERO,
        heading: Vec2::X,
        legs: vec![PathLeg::Back, PathLeg::Straight { turn: 1.0, dist: 0.0 }].into(),
        rounds: Rounds::Endless,
    };
    let mut cursor = Cursor::start(&path);
    cursor.advance(&path, 1e9, &mut Vec::new());
    assert!(cursor.finished(), "an endless round of no length is finished, not looped forever");
}
