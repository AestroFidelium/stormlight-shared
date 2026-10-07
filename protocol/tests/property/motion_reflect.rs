//! A motion glancing off a wall (stormlight/server#215).
//!
//! A ricochet is the rest of the current leg, mirrored in the wall. So whatever the
//! leg is — straight or curved — the ground covered after the bounce is the mirror
//! image, across the wall's line, of the ground the unit would have covered had the
//! wall not been there. Pinned as exactly that, plus what follows from it:
//!
//! - nothing of the path is lost or gained: the leg's remaining length is the same
//!   after the bounce as before it;
//! - the heading is reflected — the angle of incidence is the angle of reflection;
//! - bouncing twice off the same wall faces the way it started.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use core::num::NonZeroU32;
use stormlight_shared::motion::{Cursor, MotionPath, PathLeg, Rounds};

const CLOSE: f32 = 2e-3;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// The leg: a straight one, or a curve bending this many degrees.
    #[generator(-360i16..=360)]
    bend: i16,
    #[generator(0u16..=359)]
    heading: u16,
    /// How far into the leg the wall is met, and how far past it is checked, in
    /// tenths of a unit; the leg is long enough for both.
    #[generator(0u16..=200)]
    before: u16,
    #[generator(1u16..=200)]
    after: u16,
    /// The wall's facing.
    #[generator(0u16..=359)]
    normal: u16,
}

impl Scenario {
    fn path(&self) -> MotionPath {
        MotionPath {
            start: Vec2::new(1.0, 2.0),
            heading: Vec2::from_angle(f32::from(self.heading).to_radians()),
            legs: vec![PathLeg::Arc {
                turn: 0.0,
                dist: 50.0,
                bend: f32::from(self.bend).to_radians(),
            }]
            .into(),
            rounds: Rounds::Times(NonZeroU32::MIN),
        }
    }
    fn normal(&self) -> Vec2 {
        Vec2::from_angle(f32::from(self.normal).to_radians())
    }
}

/// `p` mirrored across the line through `on` with unit normal `n`.
fn mirror(p: Vec2, on: Vec2, n: Vec2) -> Vec2 {
    p - 2.0 * (p - on).dot(n) * n
}

#[test]
fn after_a_bounce_the_path_is_its_own_mirror_image() {
    check!().with_type::<Scenario>().for_each(|s| {
        let path = s.path();
        let n = s.normal();
        let mut cursor = Cursor::start(&path);
        cursor.advance(&path, f32::from(s.before) / 10.0, &mut Vec::new());
        let wall = cursor.position();
        let left = cursor.leg_left();

        let mut straight_on = cursor;
        let mut bounced = cursor;
        bounced.reflect(&path, n);
        assert!(
            (bounced.leg_left() - left).abs() < CLOSE,
            "a bounce changed what is left of the leg from {left} to {}",
            bounced.leg_left(),
        );
        let incoming = cursor.heading();
        assert!(
            bounced.heading().distance(incoming - 2.0 * incoming.dot(n) * n) < CLOSE,
            "came in along {incoming}, left along {} off a wall facing {n}",
            bounced.heading(),
        );

        let after = f32::from(s.after) / 10.0;
        straight_on.advance(&path, after, &mut Vec::new());
        bounced.advance(&path, after, &mut Vec::new());
        let expected = mirror(straight_on.position(), wall, n);
        assert!(
            bounced.position().distance(expected) < CLOSE,
            "after the bounce the unit is at {}, the mirror of where it was going is {expected}",
            bounced.position(),
        );
    });
}

#[test]
fn bouncing_twice_off_one_wall_faces_the_way_it_started() {
    check!().with_type::<Scenario>().for_each(|s| {
        let path = s.path();
        let mut cursor = Cursor::start(&path);
        cursor.advance(&path, f32::from(s.before) / 10.0, &mut Vec::new());
        let heading = cursor.heading();
        cursor.reflect(&path, s.normal());
        cursor.reflect(&path, s.normal());
        assert!(
            cursor.heading().distance(heading) < CLOSE,
            "two bounces off one wall turned {heading} into {}",
            cursor.heading(),
        );
    });
}
