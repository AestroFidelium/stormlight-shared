//! Invariants of the projectile motion law
//! ([`stormlight_shared::projectiles`]) — the single straight-line kinematics
//! both the authoritative server and every client integrate. Pinning it here is
//! what lets a client spawn a projectile from a one-shot "fired" event and have
//! its visual path agree with the server's authoritative flight (server#9): same
//! law, same launch, same clock ⇒ same trajectory.
//!
//! Directional/structural invariants only: the path is a straight ray from the
//! origin along the velocity, progress and expiry are monotone in elapsed time,
//! and the step between two times never exceeds `speed · Δt` (no teleport).

use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::projectiles::{
    projectile_expired, projectile_position, projectile_traveled,
};

/// Raw seeds mapped onto bounded finite values so shrinking lands on numeric
/// edges (0, ±range) rather than NaN/Inf bit patterns.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    origin: (u16, u16, u16),
    dir: (u16, u16, u16),
    speed_seed: u16,
    range_seed: u16,
    e1_seed: u16,
    e2_seed: u16,
}

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// A world-space coordinate in ≈[-100, 100].
fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

fn origin_of(s: &Scenario) -> Vec3 {
    let (x, y, z) = s.origin;
    Vec3::new(coord(x), coord(y), coord(z))
}

/// A unit-ish direction; a degenerate (near-zero) raw sample collapses to +X so
/// the direction is always well defined.
fn dir_of(s: &Scenario) -> Vec3 {
    let (x, y, z) = s.dir;
    let raw = Vec3::new(frac(x) - 0.5, frac(y) - 0.5, frac(z) - 0.5);
    raw.try_normalize().unwrap_or(Vec3::X)
}

/// Speed in ≈[0, 50] world units/second.
fn speed_of(s: &Scenario) -> f32 {
    frac(s.speed_seed) * 50.0
}

/// Max flight distance in ≈[0, 100] world units.
fn range_of(s: &Scenario) -> f32 {
    frac(s.range_seed) * 100.0
}

/// Two elapsed samples in ≈[0, 10] seconds, ordered `lo ≤ hi`.
fn elapsed_pair(s: &Scenario) -> (f32, f32) {
    let a = frac(s.e1_seed) * 10.0;
    let b = frac(s.e2_seed) * 10.0;
    (a.min(b), a.max(b))
}

#[test]
fn launch_is_at_origin_and_negative_time_clamps() {
    check!().with_type::<Scenario>().for_each(|s| {
        let o = origin_of(s);
        let v = dir_of(s) * speed_of(s);
        assert!(
            projectile_position(o, v, 0.0).distance(o) < 1e-3,
            "elapsed 0 must sit exactly at the launch origin"
        );
        assert!(
            projectile_position(o, v, -5.0).distance(o) < 1e-3,
            "negative elapsed must clamp to the launch origin, never fly backwards"
        );
        assert!(
            projectile_traveled(v, -5.0) < 1e-3,
            "negative elapsed must report zero distance travelled"
        );
    });
}

#[test]
fn path_is_a_straight_ray_along_velocity() {
    check!().with_type::<Scenario>().for_each(|s| {
        let o = origin_of(s);
        let speed = speed_of(s);
        let v = dir_of(s) * speed;
        let (_, e) = elapsed_pair(s);
        // Direction is only meaningful once the shot is actually moving.
        if speed < 1e-2 || e < 1e-2 {
            return;
        }
        let offset = projectile_position(o, v, e) - o;
        // Collinear with velocity: the cross product vanishes (scaled tol).
        let cross = offset.cross(v).length();
        let tol = (offset.length() * v.length() * 1e-3).max(1e-3);
        assert!(cross <= tol, "position must lie on the origin→velocity ray (cross={cross})");
        // Same direction as velocity, never behind it.
        assert!(offset.dot(v) >= -1e-3, "projectile must travel along +velocity, not backwards");
    });
}

#[test]
fn step_never_exceeds_speed_times_dt() {
    check!().with_type::<Scenario>().for_each(|s| {
        let o = origin_of(s);
        let speed = speed_of(s);
        let v = dir_of(s) * speed;
        let (e1, e2) = elapsed_pair(s);
        let moved = projectile_position(o, v, e2).distance(projectile_position(o, v, e1));
        let expected = speed * (e2 - e1);
        // Exact straight-line motion: displacement equals speed·Δt (tight, scaled).
        let tol = (expected * 1e-3).max(1e-2);
        assert!(
            (moved - expected).abs() <= tol,
            "displacement {moved} must equal speed·Δt {expected} (no teleport, no lag)"
        );
    });
}

#[test]
fn distance_travelled_is_monotone_and_matches_position() {
    check!().with_type::<Scenario>().for_each(|s| {
        let o = origin_of(s);
        let v = dir_of(s) * speed_of(s);
        let (e1, e2) = elapsed_pair(s);
        let (d1, d2) = (projectile_traveled(v, e1), projectile_traveled(v, e2));
        assert!(d2 + 1e-3 >= d1, "travelled distance must be monotone in elapsed time");
        // The scalar odometer agrees with the actual displacement from origin.
        let from_origin = projectile_position(o, v, e2).distance(o);
        let tol = (d2 * 1e-3).max(1e-2);
        assert!(
            (from_origin - d2).abs() <= tol,
            "odometer {d2} must equal displacement-from-origin {from_origin}"
        );
    });
}

#[test]
fn expiry_is_monotone_and_agrees_with_travelled() {
    check!().with_type::<Scenario>().for_each(|s| {
        let v = dir_of(s) * speed_of(s);
        let range = range_of(s);
        let (e1, e2) = elapsed_pair(s);
        // Once expired, always expired: no coming back to life at a later time.
        if projectile_expired(v, range, e1) {
            assert!(
                projectile_expired(v, range, e2),
                "expiry must be monotone: expired at {e1} but not at later {e2}"
            );
        }
        // Expiry is exactly the odometer crossing the range.
        assert_eq!(
            projectile_expired(v, range, e2),
            projectile_traveled(v, e2) >= range.max(0.0),
            "expired iff travelled distance has reached the range"
        );
    });
}
