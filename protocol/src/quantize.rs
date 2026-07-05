//! Compact wire encoding for [`Transform`] (`stormlight/server#10`).
//!
//! A moving unit replicates its `Transform` every tick, so at scale those bytes
//! dominate egress. The default `bevy/serialize` path ships the full struct —
//! translation, rotation, and scale as ten raw `f32`s (~40 bytes via bincode).
//! This codec packs the same information into a fixed **22-byte** frame with a
//! bounded, imperceptible loss:
//!
//! | field       | encoding                                   | bytes |
//! | ----------- | ------------------------------------------ | ----- |
//! | translation | `i32` fixed-point per axis                  | 12    |
//! | rotation    | smallest-three unit quaternion in one `u32`| 4     |
//! | scale       | `i16` fixed-point per axis                  | 6     |
//!
//! The linear parts use fixed-point quantization (an integer count of
//! [`POSITION_RESOLUTION`] / [`SCALE_RESOLUTION`] steps per world unit); the
//! rotation uses the classic *smallest-three* scheme — store which component is
//! largest plus the other three at 10 bits each, and reconstruct the largest
//! from the unit-length constraint. Because `q` and `-q` are the same rotation,
//! the largest component is always stored non-negative, freeing its sign bit.
//!
//! The engine stays content-free: this is a generic `Transform` codec, nothing
//! hero- or map-specific. It is wired into the protocol via
//! [`serialize_fns`], which [`crate::protocol`] hands to
//! `register_component_custom_serde`.

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;
use lightyear_serde::SerializationError;
use lightyear_serde::reader::{ReadInteger, Reader};
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::{WriteInteger, Writer};

/// Fixed-point steps per world unit for translation. `2^10`, so each step is
/// ≈0.001 units — finer than any renderer resolves — while an `i32` count still
/// spans ≈±2.1 million units, far beyond any playable map.
pub const POSITION_RESOLUTION: f32 = 1024.0;

/// Fixed-point steps per unit for scale. Coarser than position (`2^8`, ≈0.004
/// per step) because scale needs less precision, and an `i16` count still spans
/// ≈±128× — every realistic unit size.
pub const SCALE_RESOLUTION: f32 = 256.0;

/// Bits per stored quaternion component in the smallest-three packing. Three of
/// them plus a 2-bit index fill exactly one `u32`.
const QUAT_COMPONENT_BITS: u32 = 10;
/// Mask for one packed quaternion component.
const QUAT_COMPONENT_MASK: u32 = (1 << QUAT_COMPONENT_BITS) - 1;
/// Largest magnitude any non-largest quaternion component can have: when a
/// component is the largest, the other three are each bounded by `1/√2`.
const QUAT_COMPONENT_RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Size of one encoded frame: 12 (position) + 4 (rotation) + 6 (scale).
pub const QUANTIZED_LEN: usize = 22;

/// Quantize one world coordinate to a fixed-point integer. The saturating
/// `as i32` cast keeps absurd inputs (NaN → 0, ±∞ → clamp) from wrapping.
fn quantize_axis(v: f32, resolution: f32) -> i32 {
    (v * resolution).round() as i32
}

/// Inverse of [`quantize_axis`].
fn dequantize_axis(q: i32, resolution: f32) -> f32 {
    q as f32 / resolution
}

/// Pack a unit quaternion into a `u32` via the smallest-three scheme. The input
/// is normalized defensively; a zero-length quaternion collapses to identity.
fn pack_quat(q: Quat) -> u32 {
    let q = q.normalize();
    let q = if q.is_finite() { q } else { Quat::IDENTITY };
    let c = [q.x, q.y, q.z, q.w];

    // Index of the largest-magnitude component — the one we drop and rebuild.
    let mut largest = 0usize;
    for i in 1..4 {
        if c[i].abs() > c[largest].abs() {
            largest = i;
        }
    }
    // q ≡ -q: flip so the dropped (largest) component is non-negative, which is
    // the assumption the decoder makes when it rebuilds it as a positive root.
    let sign = if c[largest] < 0.0 { -1.0 } else { 1.0 };

    let mut bits = largest as u32;
    for (i, &comp) in c.iter().enumerate() {
        if i == largest {
            continue;
        }
        // Map [-range, range] → [0, MASK] and store.
        let normalized = (comp * sign / QUAT_COMPONENT_RANGE).clamp(-1.0, 1.0);
        let quantized = ((normalized * 0.5 + 0.5) * QUAT_COMPONENT_MASK as f32).round() as u32;
        bits = (bits << QUAT_COMPONENT_BITS) | (quantized & QUAT_COMPONENT_MASK);
    }
    bits
}

/// Rebuild a unit quaternion from its smallest-three packing. Never panics and
/// never yields NaN: the reconstructed largest component takes `max(0, …)`
/// before the square root, so a corrupt frame degrades to a valid rotation.
fn unpack_quat(bits: u32) -> Quat {
    // The three stored components occupy the low 30 bits, the index the top 2.
    let largest = (bits >> (3 * QUAT_COMPONENT_BITS)) as usize & 0b11;
    let mut stored = [0.0f32; 3];
    let mut sum_sq = 0.0f32;
    for (slot, s) in stored.iter_mut().enumerate() {
        let shift = (2 - slot) as u32 * QUAT_COMPONENT_BITS;
        let quantized = (bits >> shift) & QUAT_COMPONENT_MASK;
        let normalized = quantized as f32 / QUAT_COMPONENT_MASK as f32 * 2.0 - 1.0;
        let value = normalized * QUAT_COMPONENT_RANGE;
        *s = value;
        sum_sq += value * value;
    }
    let recovered = (1.0 - sum_sq).max(0.0).sqrt();

    // Scatter the three stored components around the reconstructed largest one.
    let mut c = [0.0f32; 4];
    let mut slot = 0;
    for (i, out) in c.iter_mut().enumerate() {
        if i == largest {
            *out = recovered;
        } else {
            *out = stored[slot];
            slot += 1;
        }
    }
    Quat::from_xyzw(c[0], c[1], c[2], c[3]).normalize()
}

/// Encode a `Transform` into its fixed-length wire frame.
#[must_use]
pub fn encode(t: &Transform) -> [u8; QUANTIZED_LEN] {
    let mut out = [0u8; QUANTIZED_LEN];
    let mut cursor = 0;
    let mut put = |bytes: &[u8]| {
        out[cursor..cursor + bytes.len()].copy_from_slice(bytes);
        cursor += bytes.len();
    };

    for axis in [t.translation.x, t.translation.y, t.translation.z] {
        put(&quantize_axis(axis, POSITION_RESOLUTION).to_be_bytes());
    }
    put(&pack_quat(t.rotation).to_be_bytes());
    for axis in [t.scale.x, t.scale.y, t.scale.z] {
        put(&(quantize_axis(axis, SCALE_RESOLUTION) as i16).to_be_bytes());
    }
    out
}

/// Decode a wire frame back into a `Transform`. Total function: every 22-byte
/// input maps to a finite `Transform`.
#[must_use]
pub fn decode(bytes: &[u8; QUANTIZED_LEN]) -> Transform {
    let i32_at = |o: usize| i32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let u32_at = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let i16_at = |o: usize| i16::from_be_bytes([bytes[o], bytes[o + 1]]);

    let translation = Vec3::new(
        dequantize_axis(i32_at(0), POSITION_RESOLUTION),
        dequantize_axis(i32_at(4), POSITION_RESOLUTION),
        dequantize_axis(i32_at(8), POSITION_RESOLUTION),
    );
    let rotation = unpack_quat(u32_at(12));
    let scale = Vec3::new(
        dequantize_axis(i16_at(16).into(), SCALE_RESOLUTION),
        dequantize_axis(i16_at(18).into(), SCALE_RESOLUTION),
        dequantize_axis(i16_at(20).into(), SCALE_RESOLUTION),
    );
    Transform {
        translation,
        rotation,
        scale,
    }
}

/// Serialize a `Transform` into the Lightyear wire buffer using [`encode`].
fn serialize(t: &Transform, writer: &mut Writer) -> Result<(), SerializationError> {
    for byte in encode(t) {
        writer.write_u8(byte)?;
    }
    Ok(())
}

/// Deserialize a `Transform` from the Lightyear wire buffer using [`decode`].
fn deserialize(reader: &mut Reader) -> Result<Transform, SerializationError> {
    let mut bytes = [0u8; QUANTIZED_LEN];
    for slot in &mut bytes {
        *slot = reader.read_u8()?;
    }
    Ok(decode(&bytes))
}

/// The custom (de)serialization pair handed to `register_component_custom_serde`
/// so replicated `Transform`s travel as the compact 22-byte frame.
#[must_use]
pub fn serialize_fns() -> SerializeFns<Transform> {
    SerializeFns {
        serialize,
        deserialize,
    }
}
