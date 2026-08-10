//! Invariants of [`stormlight_shared::connection::lerp_transform`] — the
//! generic smoothing primitive every renderer leans on. These pin the contract
//! the client relies on when easing between two confirmed server snapshots:
//! endpoints are exact, the path is a straight segment with no overshoot, and
//! the rotation never leaves the unit sphere (otherwise remote models shear).

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;
use bolero::{TypeGenerator, check};
use stormlight_shared::connection::lerp_transform;

/// Raw bolero seeds, mapped onto bounded finite values so shrinking lands on
/// numeric edge cases (0, ±range) instead of NaN/Inf bit-patterns.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    a: (u16, u16, u16),
    b: (u16, u16, u16),
    qa: (u8, u8, u8, u8),
    qb: (u8, u8, u8, u8),
    t_seed: u8,
}

fn axis(seed: u16) -> f32 {
    (f32::from(seed) / f32::from(u16::MAX) - 0.5) * 200.0 // ≈ [-100, 100]
}

fn t_of(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX) // [0, 1]
}

fn unit_quat((x, y, z, w): (u8, u8, u8, u8)) -> Quat {
    let v = [x, y, z, w].map(|c| f32::from(c) - 127.5);
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
    if len < 1e-4 {
        Quat::IDENTITY
    } else {
        Quat::from_xyzw(v[0] / len, v[1] / len, v[2] / len, v[3] / len)
    }
}

fn translated((x, y, z): (u16, u16, u16)) -> Transform {
    Transform::from_translation(Vec3::new(axis(x), axis(y), axis(z)))
}

#[test]
fn endpoints_are_exact() {
    check!().with_type::<Scenario>().for_each(|s| {
        let a = translated(s.a);
        let b = translated(s.b);
        assert!(
            lerp_transform(a, b, 0.0).translation.distance(a.translation) < 1e-3,
            "t=0 must reproduce the start translation"
        );
        assert!(
            lerp_transform(a, b, 1.0).translation.distance(b.translation) < 1e-3,
            "t=1 must reproduce the end translation"
        );
    });
}

#[test]
fn translation_lies_on_segment_without_overshoot() {
    check!().with_type::<Scenario>().for_each(|s| {
        let a = translated(s.a);
        let b = translated(s.b);
        let t = t_of(s.t_seed);
        let out = lerp_transform(a, b, t).translation;
        let seg = b.translation - a.translation;
        let part = out - a.translation;
        let seg_len_sq = seg.length_squared();
        if seg_len_sq < 1e-6 {
            assert!(
                out.distance(a.translation) < 1e-3,
                "degenerate segment must collapse onto the endpoint"
            );
            return;
        }
        // Parallel to the segment (lies on the a→b line).
        let cross_mag = part.cross(seg).length();
        let tol = (seg.length() * 1e-3).max(1e-3);
        assert!(cross_mag <= tol, "result must lie on the a–b line");
        // Between the endpoints — no overshoot.
        let dot = part.dot(seg);
        assert!(
            (-1e-3..=seg_len_sq + 1e-3).contains(&dot),
            "result must lie between a and b along the segment"
        );
    });
}

#[test]
fn rotation_stays_unit_quaternion() {
    check!().with_type::<Scenario>().for_each(|s| {
        let a = Transform::from_rotation(unit_quat(s.qa));
        let b = Transform::from_rotation(unit_quat(s.qb));
        let t = t_of(s.t_seed);
        let len = lerp_transform(a, b, t).rotation.length();
        assert!((len - 1.0).abs() < 1e-3, "slerp output must stay a unit quaternion: |q|={len}");
    });
}

#[test]
fn translation_is_symmetric_under_reversal() {
    // lerp(a,b,t) ≈ lerp(b,a,1-t) — catches asymmetric ordering bugs.
    check!().with_type::<Scenario>().for_each(|s| {
        let a = translated(s.a);
        let b = translated(s.b);
        let t = t_of(s.t_seed);
        let fwd = lerp_transform(a, b, t).translation;
        let rev = lerp_transform(b, a, 1.0 - t).translation;
        assert!(fwd.distance(rev) < 1e-2, "lerp must be reversal-symmetric");
    });
}
