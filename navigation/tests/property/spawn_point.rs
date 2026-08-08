//! Property test (server#56): where a match puts its players is read off the
//! **map's own geometry**, never written into the engine.
//!
//! An engine that picks a coordinate has quietly authored content, so start
//! positions are derived from the baked walkable region instead: spread around a
//! ring inside the map's extent, each snapped to walkable ground.
//!
//! Invariants over generated arenas (a rectangular outline with a central block,
//! the shape a real map has):
//! - every derived position is **inside the walkable region** — a player never
//!   starts in a wall or off the map, whatever the geometry;
//! - positions are inside the map's bounds;
//! - it is a pure function of the map and the seat: the same seat always yields the
//!   same point, so two servers running the same content agree;
//! - seats wrap modulo the arrangement, so a lobby bigger than the format reuses
//!   positions rather than drifting off the map;
//! - distinct seats of the same arrangement are distinct positions — players do not
//!   all start on top of each other.

use bolero::{TypeGenerator, check};

use bevy::math::Vec2;
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::{NavMeshDescriptor, Point2};
use stormlight_navigation::NavMesh;

#[derive(Debug, TypeGenerator)]
struct Arena {
    /// Half-extents of the map and of the central block it is built around.
    half: u16,
    block: u16,
    /// How many seats the arrangement splits players across.
    seats: u8,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// An axis-aligned rectangle as a counter-clockwise ring of ground points.
fn rect(cx: f32, cz: f32, half_x: f32, half_z: f32) -> Vec<Point2> {
    vec![
        [cx - half_x, cz - half_z],
        [cx + half_x, cz - half_z],
        [cx + half_x, cz + half_z],
        [cx - half_x, cz + half_z],
    ]
}

/// Bake a square arena with a central block — big enough that the derived ring
/// clears the block, which is what a real map looks like.
fn arena(half: f32, block: f32) -> NavMesh {
    NavMesh::bake(&NavMeshDescriptor {
        id: NavMeshId(0),
        outline: rect(0.0, 0.0, half, half),
        obstacles: vec![rect(0.0, 0.0, block, block)],
        agent_radius: 0.5,
        placements: Vec::new(),
    })
    .expect("a rectangular arena bakes")
}

#[test]
fn every_derived_start_is_walkable_ground_inside_the_map() {
    check!().with_type::<Arena>().for_each(|a| {
        let half = 20.0 + frac(a.half) * 60.0; // [20, 80]
        // A block small enough that the 0.75 ring is clear of it.
        let block = 1.0 + frac(a.block) * (half * 0.5);
        let seats = u32::from(a.seats % 7) + 1; // [1, 7]
        let mesh = arena(half, block);
        let (min, max) = mesh.bounds();

        for seat in 0..seats {
            let point = mesh.spawn_point(seat, seats).expect("a real arena yields a start");
            assert!(
                mesh.contains(point),
                "seat {seat} must start on walkable ground, got {point:?}"
            );
            assert!(
                point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y,
                "seat {seat} must start inside the map bounds, got {point:?}"
            );
        }
    });
}

#[test]
fn a_seat_always_lands_on_the_same_point_and_wraps_around_the_arrangement() {
    check!().with_type::<Arena>().for_each(|a| {
        let half = 20.0 + frac(a.half) * 60.0;
        let block = 1.0 + frac(a.block) * (half * 0.5);
        let seats = u32::from(a.seats % 7) + 1;
        let mesh = arena(half, block);

        for seat in 0..seats {
            let first = mesh.spawn_point(seat, seats).expect("a real arena yields a start");
            let again = mesh.spawn_point(seat, seats).expect("a real arena yields a start");
            assert_eq!(first, again, "the same seat must always derive the same start");

            // Seat n and seat n + seats are the same person's slot, one lap on.
            let wrapped =
                mesh.spawn_point(seat + seats, seats).expect("a real arena yields a start");
            assert_eq!(
                first, wrapped,
                "seat {seat} must wrap modulo {seats} rather than drift off the map"
            );
        }
    });
}

#[test]
fn players_of_one_arrangement_do_not_share_a_start() {
    check!().with_type::<Arena>().for_each(|a| {
        let half = 20.0 + frac(a.half) * 60.0;
        let block = 1.0 + frac(a.block) * (half * 0.5);
        // Two to four seats around a symmetric arena: far enough apart to be
        // unambiguous, whatever the map's size.
        let seats = u32::from(a.seats % 3) + 2;
        let mesh = arena(half, block);

        let points: Vec<Vec2> =
            (0..seats).map(|s| mesh.spawn_point(s, seats).expect("a start")).collect();
        for i in 0..points.len() {
            for j in (i + 1)..points.len() {
                assert!(
                    points[i].distance(points[j]) > 1.0,
                    "seats {i} and {j} must not start on top of each other \
                     ({:?} vs {:?})",
                    points[i],
                    points[j]
                );
            }
        }
    });
}

#[test]
fn a_start_is_the_innermost_walkable_point_on_its_ray() {
    check!().with_type::<Arena>().for_each(|a| {
        let half = 20.0 + frac(a.half) * 60.0;
        let block = 1.0 + frac(a.block) * (half * 0.5);
        let seats = u32::from(a.seats % 4) + 1;
        let mesh = arena(half, block);
        let (min, max) = mesh.bounds();
        let centre = (min + max) * 0.5;

        for seat in 0..seats {
            let point = mesh.spawn_point(seat, seats).expect("a real arena yields a start");
            let reach = point.distance(centre);
            let ray = (point - centre).normalize_or_zero();

            // Players start as close together as the map allows: nothing on the
            // same ray, between the floor and this point, is walkable ground.
            // (A generous margin below the point itself, so the scan's own step
            // granularity is not mistaken for a violation.)
            let floor = half * 0.15;
            let mut probe = floor;
            while probe < reach - half * 0.05 {
                let inner = centre + ray * probe;
                assert!(
                    !mesh.contains(inner),
                    "seat {seat} started at {reach} from the centre while {probe} was \
                     already walkable — players must not be flung further out than the \
                     geometry requires"
                );
                probe += half * 0.02;
            }
        }
    });
}
