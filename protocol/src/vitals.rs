//! Replicated combat vitals — the health/shield state a client needs to draw a
//! unit's bar (`stormlight/server#57`).
//!
//! The authoritative state lives server-side in the reversible `Life` component;
//! this is its small, read-only projection on the wire. It is a *separate*
//! component on purpose: the server's vitals carry a rewind history and an
//! absorbing-shield rule no client needs, and projecting rather than replicating
//! them keeps the reversible history from having to survive a round-trip.
//!
//! # Encoding
//!
//! A health bar is a **ratio** first and a number second, so the codec stores the
//! ceiling absolutely and the current value as a fraction of it:
//!
//! | field     | encoding                                | bytes |
//! | --------- | --------------------------------------- | ----- |
//! | `max_hp`  | `u32` fixed-point, [`HEALTH_RESOLUTION`] | 4     |
//! | `hp`      | `u16` fraction of `max_hp`               | 2     |
//! | `shield`  | `u32` fixed-point, [`HEALTH_RESOLUTION`] | 4     |
//!
//! Ten bytes against twelve for the raw `f32` triple — a modest saving, because
//! unlike [`crate::quantize`] the real win here is structural rather than
//! volumetric. Storing health as a fraction makes "current never exceeds max" a
//! property of the *representation* (no frame can express an overfull bar), makes
//! the fill fraction's error independent of how large the ceiling is, and gives
//! `max_hp == 0` a defined meaning (an empty bar) instead of a division by zero.
//! Vitals also change far less often than a `Transform`, so they ride the wire
//! only on the ticks they actually move.
//!
//! Content-free: health, a ceiling, and an absorbing shield are generic unit
//! state. Nothing here knows which unit, hero, or mod the numbers came from.

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_serde::SerializationError;
use lightyear_serde::reader::{ReadInteger, Reader};
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::{WriteInteger, Writer};
use serde::{Deserialize, Serialize};

/// Fixed-point steps per point of health for the absolute quantities (`max_hp`,
/// `shield`). `16` steps ⇒ 0.0625 precision, finer than any readout prints, while
/// a `u32` count still spans ≈268 million health — past any plausible
/// fortification a mod could declare.
pub const HEALTH_RESOLUTION: f32 = 16.0;

/// Quantization steps the fill fraction is stored in (the `u16` range). A bar is
/// therefore exact to ≈0.0015% of its width, at any ceiling.
pub const RATIO_STEPS: f32 = u16::MAX as f32;

/// Size of one encoded vitals frame: 4 (`max_hp`) + 2 (`hp`) + 4 (`shield`).
pub const VITALS_LEN: usize = 10;

/// A unit's replicated combat vitals: current health, its ceiling, and the
/// absorbing shield that soaks damage ahead of health.
///
/// Present on every replicated unit that has health, and **public** — any client
/// that can already see the unit sees its vitals, exactly as it sees the unit's
/// `Transform`. Visibility is therefore decided in one place only: the
/// entity-level area-of-interest culling (`stormlight/server#8`). See
/// [`crate::pools`] for why the resource wallet follows the same rule.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplicatedVitals {
    /// Current health, always within `[0, max_hp]`.
    pub hp: f32,
    /// The health ceiling. `0` for a unit whose numbers have not landed yet.
    pub max_hp: f32,
    /// Absorbing shield on top of health. Unbounded by `max_hp` — a shield may
    /// legitimately exceed a unit's whole health pool.
    pub shield: f32,
}

impl ReplicatedVitals {
    /// Build with the invariants forced: a non-negative ceiling, health clamped
    /// into `[0, max_hp]`, and a non-negative shield. Non-finite inputs collapse
    /// to zero, so a garbage number can never reach the wire.
    #[must_use]
    pub fn new(hp: f32, max_hp: f32, shield: f32) -> Self {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let max_hp = finite(max_hp).max(0.0);
        Self { hp: finite(hp).clamp(0.0, max_hp), max_hp, shield: finite(shield).max(0.0) }
    }

    /// The fill fraction a health bar draws, in `[0, 1]`. A unit with no ceiling
    /// reads empty rather than dividing by zero.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.max_hp > 0.0 { (self.hp / self.max_hp).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Whether health has been spent — the client-side read of "this unit is
    /// down", without needing a separate death flag on the wire.
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.hp <= 0.0
    }
}

/// Quantize a non-negative quantity to fixed point. Saturating: a value past the
/// representable range clamps to the ceiling rather than wrapping to zero.
pub(crate) fn quantize_amount(v: f32) -> u32 {
    if v.is_finite() && v > 0.0 {
        // A saturating `as` cast: past the `u32` range this pins at the ceiling
        // rather than wrapping a huge ceiling round to an empty bar.
        (v * HEALTH_RESOLUTION).round() as u32
    } else {
        0
    }
}

/// Inverse of [`quantize_amount`].
pub(crate) fn dequantize_amount(q: u32) -> f32 {
    q as f32 / HEALTH_RESOLUTION
}

/// Quantize `current` as a fraction of `max`. A zero (or absent) ceiling encodes
/// as empty — the one representation of "no bar to fill".
pub(crate) fn quantize_ratio(current: f32, max: f32) -> u16 {
    if !(current.is_finite() && max.is_finite()) || max <= 0.0 || current <= 0.0 {
        return 0;
    }
    ((current / max).clamp(0.0, 1.0) * RATIO_STEPS).round() as u16
}

/// Rebuild an absolute quantity from a stored fraction and ceiling. Structurally
/// bounded by `max`, which is what keeps a decoded bar from overflowing.
pub(crate) fn dequantize_ratio(fraction: u16, max: f32) -> f32 {
    (f32::from(fraction) / RATIO_STEPS * max).clamp(0.0, max)
}

/// Encode vitals into their fixed-length wire frame.
#[must_use]
pub fn encode(v: &ReplicatedVitals) -> [u8; VITALS_LEN] {
    let mut out = [0u8; VITALS_LEN];
    out[0..4].copy_from_slice(&quantize_amount(v.max_hp).to_be_bytes());
    out[4..6].copy_from_slice(&quantize_ratio(v.hp, v.max_hp).to_be_bytes());
    out[6..10].copy_from_slice(&quantize_amount(v.shield).to_be_bytes());
    out
}

/// Decode a wire frame back into vitals. Total: every `VITALS_LEN`-byte input
/// maps to finite state with `hp <= max_hp`.
#[must_use]
pub fn decode(bytes: &[u8; VITALS_LEN]) -> ReplicatedVitals {
    let max_hp = dequantize_amount(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    let hp = dequantize_ratio(u16::from_be_bytes([bytes[4], bytes[5]]), max_hp);
    let shield = dequantize_amount(u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]));
    ReplicatedVitals { hp, max_hp, shield }
}

/// Ease between two confirmed vitals snapshots so a bar slides instead of
/// stepping at the replication rate. Every field eases linearly, which keeps
/// `hp <= max_hp` (a convex combination of two states that each satisfy it does
/// too) even while a buff is moving the ceiling.
#[must_use]
pub fn lerp_vitals(start: ReplicatedVitals, end: ReplicatedVitals, t: f32) -> ReplicatedVitals {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| a + (b - a) * t;
    ReplicatedVitals {
        hp: mix(start.hp, end.hp),
        max_hp: mix(start.max_hp, end.max_hp),
        shield: mix(start.shield, end.shield),
    }
}

/// Serialize vitals into the Lightyear wire buffer using [`encode`].
fn serialize(v: &ReplicatedVitals, writer: &mut Writer) -> Result<(), SerializationError> {
    for byte in encode(v) {
        writer.write_u8(byte)?;
    }
    Ok(())
}

/// Deserialize vitals from the Lightyear wire buffer using [`decode`].
fn deserialize(reader: &mut Reader) -> Result<ReplicatedVitals, SerializationError> {
    let mut bytes = [0u8; VITALS_LEN];
    for slot in &mut bytes {
        *slot = reader.read_u8()?;
    }
    Ok(decode(&bytes))
}

/// The custom (de)serialization pair handed to `register_component_custom_serde`.
#[must_use]
pub fn serialize_fns() -> SerializeFns<ReplicatedVitals> {
    SerializeFns { serialize, deserialize }
}

/// Register the replicated vitals on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so the protocol matches.
///
/// Registered to reach **both** client-side mirrors of a replicated unit, for the
/// same reason [`UnitTag`](crate::identity::UnitTag) is: every other player's unit
/// is drawn on its smoothed `Interpolated` copy, while the player's own hero is
/// only ever `Predicted` (server#50). A bar has to appear over both — and the
/// owner's own health bar is the one that matters most.
pub fn register(app: &mut App) {
    app.register_component_custom_serde::<ReplicatedVitals>(serialize_fns())
        .add_interpolation_with(lerp_vitals)
        .add_prediction();
}
