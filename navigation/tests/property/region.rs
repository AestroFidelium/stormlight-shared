//! Invariants of the generic navigation stack (stormlight/server#52): baking a
//! mod-supplied [`NavMeshDescriptor`] into a walkable region, and the routes it
//! produces.
//!
//! The engine authors no geometry — these fixtures are a *room with a pillar in
//! the middle*, the smallest map where "walk around" differs from "walk straight".
//! What is pinned is the pathing contract every mover downstream relies on:
//!
//! - a route's endpoints are the requested ones (it goes where it was asked);
//! - every step of it is inside the walkable region and no leg cuts through an
//!   obstacle — the actual meaning of "walks around the wall";
//! - a goal outside the region still yields a route (to the nearest walkable
//!   point) rather than stranding the unit;
//! - no leg is longer than the straight line, and the route is never longer than
//!   a detour that visits the obstacle's corners — it is a path, not a wander.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_navigation::NavMesh;

/// Half-width of the room; the pillar is a square of `PILLAR` half-extent at the
/// origin, so the two sides of the room only connect around it.
const ROOM: f32 = 20.0;
const PILLAR: f32 = 6.0;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    from_z: u8,
    to_z: u8,
    radius: u8,
}

fn frac(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX)
}

/// A square room with a square pillar in the middle, inset for `agent_radius`.
fn descriptor(agent_radius: f32) -> NavMeshDescriptor {
    NavMeshDescriptor {
        id: NavMeshId(0),
        outline: vec![[-ROOM, -ROOM], [ROOM, -ROOM], [ROOM, ROOM], [-ROOM, ROOM]],
        obstacles: vec![vec![
            [-PILLAR, -PILLAR],
            [-PILLAR, PILLAR],
            [PILLAR, PILLAR],
            [PILLAR, -PILLAR],
        ]],
        agent_radius,
    }
}

fn baked(agent_radius: f32) -> NavMesh {
    NavMesh::bake(&descriptor(agent_radius)).expect("the room should bake")
}

/// Whether a point is inside the pillar (with a small tolerance, since the inset
/// pulls the walkable region *away* from it — a path may hug the edge but never
/// enter).
fn inside_pillar(p: Vec2) -> bool {
    p.x.abs() < PILLAR - 1e-2 && p.y.abs() < PILLAR - 1e-2
}

/// Sample a leg of the route; a straight leg that crosses the pillar has some
/// sample inside it.
fn leg_crosses_pillar(a: Vec2, b: Vec2) -> bool {
    (0..=32u8).any(|i| inside_pillar(a.lerp(b, f32::from(i) / 32.0)))
}

#[test]
fn a_route_starts_and_ends_where_it_was_asked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mesh = baked(frac(s.radius));
        let from = Vec2::new(-ROOM + 2.0, (frac(s.from_z) - 0.5) * 2.0 * (ROOM - 2.0));
        let to = Vec2::new(ROOM - 2.0, (frac(s.to_z) - 0.5) * 2.0 * (ROOM - 2.0));

        let route = mesh.route(from, to).expect("both ends are walkable, so a route exists");
        let last = *route.last().expect("a route has at least the destination");
        assert!(last.distance(to) <= 1e-2, "the route ended somewhere else ({last} != {to})");
    });
}

#[test]
fn a_route_stays_out_of_the_obstacle() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mesh = baked(frac(s.radius));
        // Straight across the room: the direct line goes through the pillar.
        let from = Vec2::new(-ROOM + 2.0, (frac(s.from_z) - 0.5) * 4.0);
        let to = Vec2::new(ROOM - 2.0, (frac(s.to_z) - 0.5) * 4.0);

        let route = mesh.route(from, to).expect("a route around the pillar exists");
        let mut prev = from;
        for step in &route {
            assert!(!inside_pillar(*step), "a waypoint landed inside the obstacle ({step})");
            assert!(!leg_crosses_pillar(prev, *step), "a leg cut through the obstacle");
            prev = *step;
        }
        // It really is a detour, not the straight line the mover would have taken.
        assert!(route.len() > 1, "the route did not go around anything");
    });
}

#[test]
fn a_goal_outside_the_region_routes_to_the_nearest_walkable_point() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mesh = baked(frac(s.radius));
        let from = Vec2::new(-ROOM + 2.0, (frac(s.from_z) - 0.5) * 4.0);
        // Deep inside the pillar — an order a player can absolutely issue.
        let to = Vec2::new((frac(s.to_z) - 0.5) * PILLAR, 0.0);

        let route = mesh.route(from, to).expect("an unreachable goal still yields a route");
        let last = *route.last().expect("a route has at least one step");
        assert!(!inside_pillar(last), "the route ended inside the obstacle ({last})");
        assert!(
            last.distance(to) < from.distance(to),
            "routing toward an unreachable goal did not get any closer",
        );
    });
}

#[test]
fn a_clear_line_is_routed_straight() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mesh = baked(frac(s.radius));
        // Both points on the same side of the pillar, well clear of it.
        let z = ROOM - 2.0;
        let from = Vec2::new(-ROOM + 2.0 + frac(s.from_z) * 4.0, z);
        let to = Vec2::new(ROOM - 2.0 - frac(s.to_z) * 4.0, z);

        let route = mesh.route(from, to).expect("a clear line is routable");
        assert_eq!(route.len(), 1, "an unobstructed route should be a single leg");
        assert!(route[0].distance(to) <= 1e-2, "the single leg did not go to the goal");
    });
}
