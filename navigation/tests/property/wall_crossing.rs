//! Where a straight move first leaves the walkable region (stormlight/server#212).
//!
//! Forced movement used to find walls by sampling the path every quarter unit, at
//! most 512 times. Past 128 units the samples spread out, and at any length a wall
//! thinner than the spacing fits between two of them — so a long enough shove, or
//! a thin enough wall, went straight through.
//!
//! [`NavMesh::first_exit`] answers the question exactly instead: the first point on
//! the segment where it crosses the region's boundary **outward**, and the wall's
//! normal there. Checked against an analytic oracle — a room with a thin slab in it,
//! whose exits are a slab test away — so the answer is compared to geometry, not to
//! another sampler:
//!
//! - an exit is reported **iff** the segment leaves the region, at any length up to
//!   a million units and any wall thickness down to a hundredth;
//! - it is the **first** exit, at the right place;
//! - its normal is the face it went through, pointing out of the walkable side;
//! - a segment that only moves *into* the region, or stays inside it, reports none.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_navigation::NavMesh;

/// Half-width of the square room.
const ROOM: f32 = 50.0;
/// The slab's west face, and its half-length along y. Short of the room's walls, so
/// a move can go round it.
const WALL_X: f32 = 10.0;
const SPAN: f32 = 20.0;

/// The slab's thickness in hundredths of a unit — down to one hundredth, which no
/// sampler at a quarter-unit spacing could ever see.
#[derive(Debug, Clone, Copy, TypeGenerator)]
struct Thickness(#[generator(1u16..=200)] u16);

/// How far the move is pointed, in thousandths of a unit — up to a million units.
#[derive(Debug, Clone, Copy, TypeGenerator)]
struct Reach(#[generator(1u32..=1_000_000_000)] u32);

#[derive(Debug, TypeGenerator)]
struct Scenario {
    thickness: Thickness,
    /// Where the move starts: anywhere west of the slab, inside the room.
    from_x: u16,
    from_y: u16,
    /// Which way it points, as a fraction of a turn.
    heading: u16,
    reach: Reach,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

impl Scenario {
    fn thickness(&self) -> f32 {
        f32::from(self.thickness.0) / 100.0
    }

    fn from(&self) -> Vec2 {
        let west = -ROOM + 1.0;
        let east = WALL_X - 0.5;
        Vec2::new(
            west + frac(self.from_x) * (east - west),
            (frac(self.from_y) - 0.5) * 2.0 * (ROOM - 1.0),
        )
    }

    fn to(&self) -> Vec2 {
        let angle = frac(self.heading) * std::f32::consts::TAU;
        #[allow(clippy::cast_precision_loss)]
        let reach = self.reach.0 as f32 / 1000.0;
        self.from() + Vec2::from_angle(angle) * reach
    }
}

fn baked(thickness: f32) -> NavMesh {
    let east = WALL_X + thickness;
    NavMesh::bake(&NavMeshDescriptor {
        id: NavMeshId(0),
        outline: vec![[-ROOM, -ROOM], [ROOM, -ROOM], [ROOM, ROOM], [-ROOM, ROOM]],
        obstacles: vec![vec![[WALL_X, -SPAN], [WALL_X, SPAN], [east, SPAN], [east, -SPAN]]],
        // Exact geometry: the oracle below knows nothing of an inset.
        agent_radius: 0.0,
        placements: Vec::new(),
    })
    .expect("the room should bake")
}

/// The analytic first exit: the fraction of the segment at which it leaves the room
/// or enters the slab, and the outward normal of the face it crosses there.
fn oracle(from: Vec2, to: Vec2, thickness: f32) -> Option<(f32, Vec2)> {
    let d = to - from;
    let mut best: Option<(f32, Vec2)> = None;
    let mut keep = |t: f32, normal: Vec2| {
        if (0.0..=1.0).contains(&t) && best.is_none_or(|(b, _)| t < b) {
            best = Some((t, normal));
        }
    };
    // Leaving the room: the first of the four outer faces the segment reaches.
    for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
        let (p, v) = if axis == 0 { (from.x, d.x) } else { (from.y, d.y) };
        if v * sign > 0.0 {
            let t = (sign * ROOM - p) / v;
            let normal = if axis == 0 { Vec2::new(sign, 0.0) } else { Vec2::new(0.0, sign) };
            keep(t, normal);
        }
    }
    if let Some((t, normal)) = slab_entry(from, d, thickness) {
        keep(t, normal);
    }
    best
}

/// Where the segment `from + t·d` enters the slab, by the slab test on the box
/// `[WALL_X, WALL_X + thickness] × [-SPAN, SPAN]`, with the walkable side's outward
/// normal there — which points *into* the slab.
fn slab_entry(from: Vec2, d: Vec2, thickness: f32) -> Option<(f32, Vec2)> {
    let (lo, hi) = (Vec2::new(WALL_X, -SPAN), Vec2::new(WALL_X + thickness, SPAN));
    let mut enter = f32::NEG_INFINITY;
    let mut leave = f32::INFINITY;
    let mut face = Vec2::ZERO;
    for axis in 0..2 {
        let (p, v, l, h) =
            if axis == 0 { (from.x, d.x, lo.x, hi.x) } else { (from.y, d.y, lo.y, hi.y) };
        if v == 0.0 {
            if p < l || p > h {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((l - p) / v, (h - p) / v);
        let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
        if near > enter {
            enter = near;
            let into = if v > 0.0 { 1.0 } else { -1.0 };
            face = if axis == 0 { Vec2::new(into, 0.0) } else { Vec2::new(0.0, into) };
        }
        leave = leave.min(far);
    }
    (enter <= leave).then_some((enter, face))
}

#[test]
fn a_move_leaves_the_region_exactly_where_the_geometry_says() {
    check!().with_type::<Scenario>().for_each(|s| {
        let thickness = s.thickness();
        let map = baked(thickness);
        let (from, to) = (s.from(), s.to());
        let length = from.distance(to);
        // f32 resolution along a long segment: a few ulps of its length.
        let tolerance = 1e-3 + length * 4e-6;

        let expected = oracle(from, to, thickness);
        // A segment ending within resolution of a boundary is ambiguous either way.
        if expected.is_some_and(|(t, _)| ((1.0 - t) * length).abs() < tolerance) {
            return;
        }
        let found = map.first_exit(from, to);
        match (found, expected) {
            (None, None) => {}
            (Some(exit), Some((t, normal))) => {
                assert!(
                    (exit.fraction - t).abs() * length < tolerance,
                    "{from} → {to}: left the region at {} along it, the geometry says {}",
                    exit.fraction * length,
                    t * length,
                );
                assert!(
                    exit.at.distance(from.lerp(to, t)) < tolerance,
                    "{from} → {to}: the reported exit point {} is not where it crossed",
                    exit.at,
                );
                assert!(
                    exit.normal.distance(normal) < 1e-3,
                    "{from} → {to}: crossed a face whose outward normal is {normal}, reported {}",
                    exit.normal,
                );
            }
            (found, expected) => panic!(
                "{from} → {to} through a {thickness}-thick wall: found {found:?}, the geometry \
                 says {expected:?}",
            ),
        }
    });
}

#[test]
fn a_move_that_stays_inside_or_comes_back_in_reports_no_exit() {
    check!().with_type::<Scenario>().for_each(|s| {
        let map = baked(s.thickness());
        let from = s.from();
        // Short enough never to reach a face: the start is at least half a unit off
        // every boundary.
        let inside = from + (s.to() - from).normalize_or_zero() * 0.25;
        assert_eq!(map.first_exit(from, inside), None, "{from} → {inside} stays inside");
        // From outside the room straight back in: entering is not leaving.
        let outside = Vec2::new(ROOM + 5.0, from.y.clamp(-ROOM + 1.0, ROOM - 1.0));
        let back_in = Vec2::new(ROOM - 1.0, outside.y);
        assert_eq!(map.first_exit(outside, back_in), None, "{outside} → {back_in} only enters");
    });
}
