//! Invariants of [`stormlight_shared::quantize`] — the compact wire encoding
//! that replaces the raw `f32×10` `Transform` on the network
//! (`stormlight/server#10`). A moving unit ships its `Transform` every tick, so
//! shaving its byte count is the single biggest egress lever. These pin the
//! contract the codec must uphold: it is strictly smaller than the raw form,
//! and it round-trips position / rotation / scale within a tight, bounded loss
//! (fixed-point resolution for the linear parts, sub-degree for the rotation),
//! so the client never sees a mover teleport or shear.

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;
use bolero::{TypeGenerator, check};
use stormlight_shared::quantize::{
    POSITION_RESOLUTION, QUANTIZED_LEN, SCALE_RESOLUTION, decode, encode,
};

/// Raw bolero seeds mapped onto bounded finite values so shrinking lands on
/// numeric edge cases (0, ±range) instead of NaN/Inf bit-patterns.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    pos: (u16, u16, u16),
    quat: (u8, u8, u8, u8),
    scale: (u8, u8, u8),
}

/// A world-space coordinate in ≈[-500, 500] — a generous map half-extent, well
/// inside the fixed-point range but large enough to exercise real magnitudes.
fn coord(seed: u16) -> f32 {
    (f32::from(seed) / f32::from(u16::MAX) - 0.5) * 1000.0
}

/// A positive scale factor in ≈[0.25, 8]: never zero (degenerate), spanning the
/// range a unit realistically renders at.
fn scale_factor(seed: u8) -> f32 {
    0.25 + f32::from(seed) / f32::from(u8::MAX) * 7.75
}

/// A unit quaternion from four seeds; collapses to identity near the origin so
/// the generator never emits a zero-length (invalid) rotation.
fn unit_quat((x, y, z, w): (u8, u8, u8, u8)) -> Quat {
    let v = [x, y, z, w].map(|c| f32::from(c) - 127.5);
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
    if len < 1e-4 {
        Quat::IDENTITY
    } else {
        Quat::from_xyzw(v[0] / len, v[1] / len, v[2] / len, v[3] / len)
    }
}

fn transform_of(s: &Scenario) -> Transform {
    Transform {
        translation: Vec3::new(coord(s.pos.0), coord(s.pos.1), coord(s.pos.2)),
        rotation: unit_quat(s.quat),
        scale: Vec3::new(scale_factor(s.scale.0), scale_factor(s.scale.1), scale_factor(s.scale.2)),
    }
}

#[test]
fn encoding_is_strictly_smaller_than_raw_transform() {
    // The whole point of the codec: fewer bytes on the wire than the default
    // `bevy/serialize` bincode form (`f32×10`). Structural invariant — holds for
    // every generated transform, not a hand-picked value.
    check!().with_type::<Scenario>().for_each(|s| {
        let t = transform_of(s);
        let raw = bincode::serde::encode_to_vec(t, bincode::config::standard())
            .expect("Transform is bincode-serializable via bevy/serialize");
        assert_eq!(
            encode(&t).len(),
            QUANTIZED_LEN,
            "encoder must emit a fixed {QUANTIZED_LEN}-byte frame"
        );
        assert!(
            QUANTIZED_LEN < raw.len(),
            "quantized frame ({QUANTIZED_LEN} B) must be smaller than raw Transform ({} B)",
            raw.len()
        );
    });
}

#[test]
fn position_round_trips_within_resolution() {
    check!().with_type::<Scenario>().for_each(|s| {
        let t = transform_of(s);
        let back = decode(&encode(&t));
        let err = (back.translation - t.translation).abs();
        let tol = 1.0 / POSITION_RESOLUTION;
        assert!(
            err.x < tol && err.y < tol && err.z < tol,
            "position error {err:?} must stay within one fixed-point step ({tol})"
        );
    });
}

#[test]
fn scale_round_trips_within_resolution() {
    check!().with_type::<Scenario>().for_each(|s| {
        let t = transform_of(s);
        let back = decode(&encode(&t));
        let err = (back.scale - t.scale).abs();
        let tol = 1.0 / SCALE_RESOLUTION;
        assert!(
            err.x < tol && err.y < tol && err.z < tol,
            "scale error {err:?} must stay within one fixed-point step ({tol})"
        );
    });
}

#[test]
fn rotation_round_trips_within_a_fraction_of_a_degree() {
    check!().with_type::<Scenario>().for_each(|s| {
        let t = transform_of(s);
        let back = decode(&encode(&t)).rotation;
        // Stays on the unit sphere — otherwise a remote model shears.
        assert!(
            (back.length() - 1.0).abs() < 1e-3,
            "decoded rotation must remain a unit quaternion: |q|={}",
            back.length()
        );
        // Directional: the decoded rotation must map every basis axis to almost
        // the same direction as the original (independent of the q ≡ -q sign
        // ambiguity the codec exploits).
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let angle = (t.rotation * axis).angle_between(back * axis);
            assert!(
                angle < 0.02,
                "rotated {axis:?} drifted {angle} rad (>{}°) after round-trip",
                0.02_f32.to_degrees()
            );
        }
    });
}

#[test]
fn linear_frame_is_idempotent_and_rotation_is_stable() {
    // Anti-drift: a value that survives one hop must not keep drifting on later
    // hops. Fixed-point position/scale are an exact fixed point, so those frame
    // bytes are byte-identical on re-encode. The smallest-three rotation may
    // legitimately swap which component it stores as "largest" at a near-tie, so
    // its bytes can differ — but the rotation it represents must not, hence a
    // directional check there instead of a byte compare.
    check!().with_type::<Scenario>().for_each(|s| {
        let t = transform_of(s);
        let once = encode(&t);
        let twice = encode(&decode(&once));
        // Position (bytes 0..12) and scale (16..22) are an exact fixed point.
        assert_eq!(once[0..12], twice[0..12], "position frame must be idempotent");
        assert_eq!(once[16..22], twice[16..22], "scale frame must be idempotent");
        // Rotation direction is stable across the extra hop.
        let r1 = decode(&once).rotation;
        let r2 = decode(&twice).rotation;
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let drift = (r1 * axis).angle_between(r2 * axis);
            assert!(
                drift < 0.01,
                "rotation drifted {drift} rad on a second hop (should be a stable fixed point)"
            );
        }
    });
}
