//! What a move order clicked **into solid ground** must mean (stormlight/server#65).
//!
//! A player clicks where they want to go, not where the geometry allows: retreating
//! from a fight, the click lands on a wall as often as not. The engine's answer has
//! to be "walk as close to there as you can" — a unit that simply stands still has
//! ignored an order the player very much meant, and in a fight that is fatal.
//!
//! [`clamp_goal`] is the one place that reading is made, on **both** ends (the
//! authoritative decoder and the predicting client), so these invariants are the
//! whole contract:
//!
//! - the destination is always somewhere the unit may actually stand;
//! - it makes real progress toward the click — it closes most of the open ground
//!   between the unit and the obstacle rather than returning where it started;
//! - it is the ground nearest **the click**, not merely the nearest on the unit's
//!   own side of the wall, so "as close as possible" means what it says;
//! - a unit that has been nudged off the region can still be ordered about.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use stormlight_mod_abi::ids::NavMeshId;
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_navigation::{ActiveNavMesh, NavMesh, clamp_goal};

/// Half-width of the room, and of the solid block at its centre. Sized like a real
/// arena's keep: big enough that a click into the middle of it is nowhere near any
/// walkable ground, which is exactly the case a nearest-point search cannot answer.
const ROOM: f32 = 40.0;
const KEEP: f32 = 8.0;
/// The body radius the region is inset by, as a real map declares one.
const BODY: f32 = 0.6;

/// A click into the middle of the block, from open ground on its east face.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How far east of the block's walkable edge the unit stands.
    gap: u8,
    /// Where along the face it stands, and where inside the block it clicks.
    along: u8,
    into: u8,
}

fn frac(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX)
}

/// A square room with a solid square block at its centre, inset for a body radius.
fn baked() -> NavMesh {
    let block = |half: f32| vec![[-half, -half], [half, -half], [half, half], [-half, half]];
    NavMesh::bake(&NavMeshDescriptor {
        id: NavMeshId(0),
        outline: block(ROOM),
        obstacles: vec![block(KEEP)],
        agent_radius: BODY,
        placements: Vec::new(),
    })
    .expect("the room should bake")
}

/// The unit's stance and its click, for one scenario: standing in open ground east
/// of the block, clicking somewhere inside it.
fn stance(s: &Scenario) -> (Vec2, Vec2) {
    // Anywhere from a hair off the wall to well clear of it.
    let gap = 0.2 + frac(s.gap) * 8.0;
    let anchor = Vec2::new(KEEP + BODY + gap, (frac(s.along) - 0.5) * 2.0 * (KEEP - 1.0));
    // Inside the block, never on its rim: solid ground the unit can never occupy.
    let target = Vec2::new((frac(s.into) - 0.5) * (KEEP - 1.0), (frac(s.along) - 0.5) * KEEP);
    (anchor, target)
}

#[test]
fn a_click_into_solid_ground_stays_walkable() {
    check!().with_type::<Scenario>().for_each(|s| {
        let map = ActiveNavMesh(baked());
        let (anchor, target) = stance(s);
        assert!(map.0.contains(anchor), "the fixture stood the unit off the walkable region");

        let goal = clamp_goal(Some(&map), anchor, target);
        assert!(
            map.0.contains(goal),
            "a click into solid ground produced a destination the unit cannot stand on ({goal})",
        );
    });
}

#[test]
fn a_click_into_solid_ground_lands_next_to_where_it_was_clicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let map = ActiveNavMesh(baked());
        // Standing off the block's *east* face, clicking deep inside it but hard
        // against its *west* one. "As close as the geometry allows" means the west
        // rim — the nearest ground to the click — not the near rim the unit happens
        // to be facing. Getting this wrong is what makes a click read as ignored.
        let anchor = Vec2::new(KEEP + BODY + 1.0 + frac(s.gap) * 6.0, (frac(s.along) - 0.5) * 4.0);
        let target = Vec2::new(-KEEP + 1.0, (frac(s.into) - 0.5) * 2.0 * (KEEP - 1.0));

        let goal = clamp_goal(Some(&map), anchor, target);
        // The true nearest walkable ground to that click is one body radius off the
        // west face; allow a little slack for the boundary margin and the search.
        let reachable = (KEEP + BODY) - (-target.x) + 1.0;
        assert!(
            goal.distance(target) <= reachable,
            "the destination stopped {} from the click, {reachable} was reachable \
             (anchor {anchor}, target {target}, goal {goal})",
            goal.distance(target),
        );
    });
}

#[test]
fn a_unit_nudged_off_the_region_can_still_be_ordered_about() {
    check!().with_type::<Scenario>().for_each(|s| {
        let map = ActiveNavMesh(baked());
        // Standing *inside the inset band*: physically clear of the wall, but not on
        // the walkable region. Nothing stops a unit ending up here — local avoidance
        // steers around bodies, not geometry, so a crowd can press one into the band.
        let anchor = Vec2::new(KEEP + 0.05 + frac(s.gap) * 0.2, (frac(s.along) - 0.5) * KEEP);
        assert!(!map.0.contains(anchor), "the fixture failed to stand the unit off-region");
        // Open ground, well clear of everything — an ordinary "walk over there".
        let target = Vec2::new(ROOM - BODY - 4.0, (frac(s.into) - 0.5) * 2.0 * (ROOM - 10.0));

        let goal = clamp_goal(Some(&map), anchor, target);
        assert!(
            map.0.contains(goal),
            "an order from off-region produced an unwalkable destination ({goal})",
        );
        assert!(
            goal.distance(target) < anchor.distance(target),
            "a unit nudged off the region ignored an order into open ground \
             (anchor {anchor}, target {target}, goal {goal})",
        );
    });
}

#[test]
fn a_click_into_solid_ground_walks_up_to_it() {
    check!().with_type::<Scenario>().for_each(|s| {
        let map = ActiveNavMesh(baked());
        let (anchor, target) = stance(s);
        let open = anchor.x - (KEEP + BODY); // Free ground between the unit and the wall.

        let goal = clamp_goal(Some(&map), anchor, target);
        let closed = anchor.distance(target) - goal.distance(target);
        assert!(
            closed >= open * 0.5,
            "the unit barely moved toward a click it could have walked up to: closed {closed} of \
             {open} open ground (anchor {anchor}, target {target}, goal {goal})",
        );
    });
}
