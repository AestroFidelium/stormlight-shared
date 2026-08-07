//! Generic navigation — turning mod-supplied map geometry into routes.
//!
//! The engine ships no map. A mod declares a walkable region as a
//! [`NavMeshDescriptor`] (outline, holes, agent radius); this module bakes that
//! into a polygon navmesh and answers "how do I walk from here to there" with a
//! corridor of waypoints. Everything downstream — the mover, local avoidance — is
//! unchanged and unaware of what the geometry represents.
//!
//! # Why this lives in `shared`
//!
//! It moved out of `server` for the same reason the kinematic law in
//! `stormlight_shared::movement` lives there (server#79): the **client predicts
//! the unit it controls**, so it has to answer "where can this unit actually
//! stand?" exactly as the authoritative server does. Two implementations of
//! walkability would disagree at every wall, and the player would see the
//! disagreement as their hero snapping backwards. One region, baked from one
//! descriptor, read by both ends.
//!
//! # Why polygon A*, and why this crate
//!
//! Grids force a resolution choice and produce staircase paths that need
//! smoothing; a polygon mesh gives exact any-angle routes over arbitrary shapes,
//! which is what a lane around a building needs. The solver is `polyanya` (the
//! Polyanya any-angle algorithm), used **directly** rather than through its Bevy
//! wrapper: that wrapper hard-requires bevy's render feature, and both the
//! dedicated server and the headless tests are render-free by construction.
//!
//! # Layering
//!
//! Routing is the *global* plan — the corridor around the building. Local
//! avoidance (server-side) is the *local* plan — sliding past the ally walking
//! toward you. They compose: routing hands the mover its next waypoint, avoidance
//! decides how to get there this tick.

use bevy::prelude::*;
use polyanya::{Mesh, Triangulation};
use stormlight_mod_abi::navmesh::NavMeshDescriptor;


/// How far short of a walkable boundary a fallback destination is placed, world
/// units. Small enough to be visually "right there", large enough that the point
/// is unambiguously inside the region.
const EDGE_MARGIN: f32 = 0.05;

/// How many times a refused destination is stepped back toward the unit before
/// the route is given up on. `PULLBACK_STEPS * EDGE_MARGIN` is the total distance
/// — comfortably past the router's own point-location tolerance.
const PULLBACK_STEPS: u32 = 8;

/// The closest a derived starting position may sit to the map's centre, as a
/// fraction of the region's smaller half-extent. A floor, not a target: it only
/// guarantees participants are not on top of each other on a wide-open map. See
/// [`NavMesh::spawn_point`].
const SPAWN_MIN_RING: f32 = 0.15;

/// How many rings [`NavMesh::spawn_point`] tries between the floor and the map's
/// edge before giving up on finding walkable ground along a seat's ray. Fine
/// enough to hug whatever sits in the middle, cheap enough to run per join.
const SPAWN_RING_STEPS: u32 = 32;

/// Why a descriptor could not be baked into a walkable region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BakeError {
    /// The outline has fewer than three points, so it encloses no area.
    DegenerateOutline,
}

/// A baked walkable region: the geometry a mod supplied, ready to route over.
#[derive(Debug)]
pub struct NavMesh {
    mesh: Mesh,
    bounds: (Vec2, Vec2),
}

impl NavMesh {
    /// Bake a content-supplied descriptor into a walkable region.
    ///
    /// The outline becomes the boundary and each obstacle a hole; the region is
    /// then inset by `agent_radius` so a unit of that size never clips a corner —
    /// the standard "shrink the world instead of fattening the agent" trick, which
    /// keeps routing a pure geometry query.
    ///
    /// # Errors
    /// [`BakeError::DegenerateOutline`] when the outline encloses no area.
    pub fn bake(descriptor: &NavMeshDescriptor) -> Result<Self, BakeError> {
        if descriptor.outline.len() < 3 {
            return Err(BakeError::DegenerateOutline);
        }
        let point = |p: &[f32; 2]| Vec2::new(p[0], p[1]);
        let outline: Vec<Vec2> = descriptor.outline.iter().map(point).collect();
        let mut triangulation = Triangulation::from_outer_edges(&outline);
        for obstacle in &descriptor.obstacles {
            // A ring of fewer than three points is not a hole; skip it rather than
            // fail the whole map over one malformed prop.
            if obstacle.len() >= 3 {
                triangulation.add_obstacle(obstacle.iter().map(point).collect::<Vec<_>>());
            }
        }
        if descriptor.agent_radius > 0.0 {
            triangulation.set_agent_radius(descriptor.agent_radius);
        }
        let bounds = outline.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(min, max), p| {
            (min.min(*p), max.max(*p))
        });
        let mut mesh = triangulation.as_navmesh();
        // Pre-compute the search acceleration once, at bake, instead of paying it
        // on the first query in the middle of a tick.
        mesh.bake();
        Ok(Self { mesh, bounds })
    }

    /// The corridor from `from` to `to`: the ordered waypoints a mover walks,
    /// ending at the destination. `None` when neither end can be placed on the
    /// region at all.
    ///
    /// Endpoints outside the walkable region are pulled to the nearest point
    /// inside it, so an order clicked onto a building walks the unit as close as
    /// it can get instead of stranding it — a player's click is a request, and the
    /// server answers with the best legal reading of it.
    #[must_use]
    pub fn route(&self, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
        let start = self.nearest_walkable(from)?;
        let mut end = self.reachable_from(to, start);
        // Point location carries a small tolerance — a destination a hair inside a
        // wall can still *locate* to the polygon next to it, and only the router
        // says otherwise. So ask the router, and on refusal step the destination
        // back toward the unit until it accepts one. Bounded, and only ever
        // exercised for a destination that was not walkable to begin with.
        for _ in 0..PULLBACK_STEPS {
            if start.distance(end) <= EDGE_MARGIN {
                return Some(vec![end]);
            }
            if let Some(path) = self.mesh.path(start, end) {
                return Some(path.path);
            }
            end += (start - end).normalize_or_zero() * EDGE_MARGIN;
        }
        None
    }

    /// The closest point inside the walkable region — the point itself when it is
    /// already inside. `None` only when the region is unreachable from `point`
    /// within the mesh's own search radius; [`Self::reachable_from`] is the total
    /// version used for destinations.
    #[must_use]
    pub fn nearest_walkable(&self, point: Vec2) -> Option<Vec2> {
        if self.mesh.point_in_mesh(point) {
            return Some(point);
        }
        self.mesh.get_closest_point(point).map(|coords| coords.position())
    }

    /// The point closest to `target` that lies inside the walkable region, seen
    /// from the walkable `anchor`. Total: it always returns a point, falling back
    /// to `anchor` itself. Purely geometric — [`Self::route`] is what confirms the
    /// router will actually accept the result as a destination.
    ///
    /// A player may click anywhere — the middle of a building, off the map — and
    /// the answer must never be "nothing happens". The mesh's own outward search
    /// handles a target just past an edge; for one buried deep inside an obstacle
    /// it cannot reach, this walks the segment from the anchor toward the target
    /// and stops at the last walkable point, which is exactly "get as close to
    /// where I clicked as the geometry allows".
    #[must_use]
    pub fn reachable_from(&self, target: Vec2, anchor: Vec2) -> Vec2 {
        if let Some(point) = self.nearest_walkable(target) {
            return point;
        }
        // Bisect the anchor→target segment: `lo` is always walkable, `hi` never is.
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = f32::midpoint(lo, hi);
            if self.mesh.point_in_mesh(anchor.lerp(target, mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        // Stop a hair short of the boundary. The bisection converges *onto* the
        // edge, and a destination sitting exactly on one is ambiguous — the router
        // may fail to place it in any polygon, and a unit standing there has half
        // its body in the wall.
        let span = anchor.distance(target).max(f32::EPSILON);
        anchor.lerp(target, (lo - EDGE_MARGIN / span).max(0.0))
    }

    /// The axis-aligned bounds of the map, `(min, max)` — the extent any grid
    /// built over it needs to cover.
    #[must_use]
    pub fn bounds(&self) -> (Vec2, Vec2) {
        self.bounds
    }

    /// Whether a point is inside the walkable region.
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        self.mesh.point_in_mesh(point)
    }

    /// A starting position for the `index`-th of `total` participants, derived
    /// **entirely from the map's own geometry** (stormlight/server#56).
    ///
    /// A match has to put players somewhere, and an engine that invents a
    /// coordinate has quietly authored content. Until a map declares its own start
    /// positions, this reads them off the region instead: each seat gets a ray out
    /// from the map's centre, evenly spaced around the circle, and takes the
    /// **innermost walkable point** on it. So participants start as close together
    /// as the geometry allows — a training ground where you can see the other
    /// player — hugging whatever sits in the middle rather than being flung to
    /// opposite edges.
    ///
    /// Scanning outward rather than picking a fixed fraction of the extent is what
    /// makes this robust: a blind fraction can land *inside* the map's central
    /// obstruction, where every seat snaps onto the same wall and players pile up.
    /// [`SPAWN_MIN_RING`] keeps them apart on a map with nothing in the middle.
    ///
    /// `index` wraps modulo `total`, so a lobby larger than the arrangement reuses
    /// positions rather than drifting off the map.
    ///
    /// `None` only when no point along the ray is walkable and the region is
    /// unreachable from its far end — a degenerate map, where the caller has no
    /// honest position to offer.
    #[must_use]
    pub fn spawn_point(&self, index: u32, total: u32) -> Option<Vec2> {
        let (min, max) = self.bounds;
        let centre = (min + max) * 0.5;
        let half = ((max - min) * 0.5).min_element();
        let total = total.max(1);
        let angle = std::f32::consts::TAU * (index % total) as f32 / total as f32;
        let (sin, cos) = angle.sin_cos();
        let ray = Vec2::new(cos, sin);

        let floor = half * SPAWN_MIN_RING;
        let span = half - floor;
        for step in 0..=SPAWN_RING_STEPS {
            let reach = span.mul_add(step as f32 / SPAWN_RING_STEPS as f32, floor);
            let candidate = centre + ray * reach;
            if self.contains(candidate) {
                return Some(candidate);
            }
        }
        // Nothing along the ray was walkable — fall back to the region's own
        // outward search from the far end rather than refusing to seat anyone.
        self.nearest_walkable(centre + ray * half)
    }
}

/// The map every mover currently routes over. Absent until content supplies
/// geometry — with no navmesh the simulation walks straight lines, exactly as it
/// did before this slice.
#[derive(Resource, Debug)]
pub struct ActiveNavMesh(pub NavMesh);

/// The walkable destination a move order really means, seen from `here`.
///
/// A player may click anywhere — into a building, off the map — and "nothing
/// happens" is never an acceptable answer, so the click is read as the nearest
/// place the unit can actually stand. With no map loaded there is no geometry to
/// clamp against and the raw point stands, so a mapless simulation behaves exactly
/// as it did before any of this.
///
/// **Both ends call this** (server#79): the authoritative server when it decodes
/// the order, and the predicting client when it sets the same goal locally. That
/// is the whole point — a click that lands in a wall has to become the *same*
/// destination on both sides, or the client walks somewhere the server will not go
/// and the player watches their hero snap backwards.
#[must_use]
pub fn clamp_goal(map: Option<&ActiveNavMesh>, here: Vec2, target: Vec2) -> Vec2 {
    map.map_or(target, |m| m.0.reachable_from(target, here))
}
