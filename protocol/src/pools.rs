//! Replicated resource pools — the wire view of a unit's generic wallet
//! (`stormlight/server#57`).
//!
//! A pool is `(opaque id → current/max)`. The engine never learns what the id
//! *means*: the server's authoritative wallet is keyed by a mod-global resource
//! handle, and the raw number is all that crosses the wire — the protocol crate
//! stays content- and mod-ABI-free, exactly as it does for
//! [`UnitTag`](crate::identity::UnitTag).
//!
//! # Visibility policy
//!
//! Pools are **public**, on the same terms as [vitals](crate::vitals): every
//! client that can already see the unit receives them. The alternative —
//! owner-only wallets, so an enemy cannot read that you are out of resource — is
//! expressible (Lightyear's per-component overrides), but it would mean a *second*
//! visibility mechanism layered over the entity-level area-of-interest culling
//! (`stormlight/server#8`), keyed on a peer→sender mapping that has to stay
//! correct across every connect and disconnect. One mechanism deciding who sees a
//! unit — and everything about it — is the rule worth keeping; a mod that wants
//! resource state hidden is a later, deliberate feature rather than a silent
//! divergence between two components on the same entity.
//!
//! # Encoding
//!
//! Variable-length, because a unit's pool count is content-defined: a `u8` count
//! followed by that many fixed [`POOL_ENTRY_LEN`]-byte entries, each an id plus
//! the same ceiling-and-fraction pair [`crate::vitals`] uses (so a pool bar
//! inherits the same "never above its ceiling", "no divide by zero" guarantees).
//! A unit with no pools costs a single byte and decodes back to no pools at all —
//! never a phantom zero-bar.

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_serde::SerializationError;
use lightyear_serde::reader::{ReadInteger, Reader};
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::{WriteInteger, Writer};
use serde::{Deserialize, Serialize};

use crate::vitals::{dequantize_amount, dequantize_ratio, quantize_amount, quantize_ratio};

/// Bytes per encoded pool: 4 (id) + 4 (ceiling) + 2 (fraction of it).
pub const POOL_ENTRY_LEN: usize = 10;

/// Most pools one unit can replicate — the `u8` count's range. Far past the one
/// or two a unit realistically declares; a longer list is truncated rather than
/// corrupting the frame.
pub const MAX_POOLS: usize = u8::MAX as usize;

/// One resource pool's replicated state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PoolState {
    /// The pool's opaque (mod-global) resource id — the client's only handle for
    /// telling one of a unit's pools from another.
    pub id: u32,
    /// Current balance, always within `[0, max]`.
    pub current: f32,
    /// The pool's ceiling.
    pub max: f32,
}

impl PoolState {
    /// Build with the invariants forced: a non-negative ceiling, a balance
    /// clamped into `[0, max]`, non-finite inputs collapsed to zero.
    #[must_use]
    pub fn new(id: u32, current: f32, max: f32) -> Self {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let max = finite(max).max(0.0);
        Self { id, current: finite(current).clamp(0.0, max), max }
    }

    /// The fill fraction an orb draws, in `[0, 1]`. A pool with no ceiling reads
    /// empty rather than dividing by zero.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.max > 0.0 { (self.current / self.max).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// A unit's replicated pools, in the server's (deterministic, id-ordered) order.
/// Attached only to units that actually have a pool, so its presence is precisely
/// "this unit has resource state worth drawing".
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplicatedPools(pub Vec<PoolState>);

/// Encode a pool list into its self-describing wire frame.
#[must_use]
pub fn encode(pools: &ReplicatedPools) -> Vec<u8> {
    let kept = pools.0.len().min(MAX_POOLS);
    let mut out = Vec::with_capacity(1 + kept * POOL_ENTRY_LEN);
    out.push(kept as u8);
    for pool in pools.0.iter().take(kept) {
        out.extend_from_slice(&pool.id.to_be_bytes());
        out.extend_from_slice(&quantize_amount(pool.max).to_be_bytes());
        out.extend_from_slice(&quantize_ratio(pool.current, pool.max).to_be_bytes());
    }
    out
}

/// Decode a pool frame. Total: a truncated or hostile frame yields only the
/// entries actually present, never an over-read or a panic.
#[must_use]
pub fn decode(bytes: &[u8]) -> ReplicatedPools {
    let Some((&count, mut rest)) = bytes.split_first() else {
        return ReplicatedPools::default();
    };
    // The count byte is attacker-controlled: trust it only as far as the bytes
    // that actually follow it.
    let count = usize::from(count).min(rest.len() / POOL_ENTRY_LEN);
    let mut pools = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, tail) = rest.split_at(POOL_ENTRY_LEN);
        rest = tail;
        let id = u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let max = dequantize_amount(u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]]));
        let current = dequantize_ratio(u16::from_be_bytes([entry[8], entry[9]]), max);
        pools.push(PoolState { id, current, max });
    }
    ReplicatedPools(pools)
}

/// Ease between two confirmed pool snapshots so an orb slides instead of stepping
/// at the replication rate.
///
/// The **incoming** snapshot decides which pools exist — easing may never invent
/// or drop one. A pool the previous snapshot did not carry (a unit that just
/// gained a resource) shows its confirmed value straight away rather than filling
/// up from nothing.
#[must_use]
pub fn lerp_pools(start: ReplicatedPools, end: ReplicatedPools, t: f32) -> ReplicatedPools {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| a + (b - a) * t;
    ReplicatedPools(
        end.0
            .iter()
            .map(|target| match start.0.iter().find(|p| p.id == target.id) {
                Some(from) => PoolState {
                    id: target.id,
                    current: mix(from.current, target.current),
                    max: mix(from.max, target.max),
                },
                None => *target,
            })
            .collect(),
    )
}

/// Serialize pools into the Lightyear wire buffer using [`encode`].
fn serialize(pools: &ReplicatedPools, writer: &mut Writer) -> Result<(), SerializationError> {
    for byte in encode(pools) {
        writer.write_u8(byte)?;
    }
    Ok(())
}

/// Deserialize pools from the Lightyear wire buffer. Reads the count byte first,
/// then exactly that many entries — the frame sizes itself, so it can share a
/// buffer with whatever follows it.
fn deserialize(reader: &mut Reader) -> Result<ReplicatedPools, SerializationError> {
    let count = usize::from(reader.read_u8()?);
    let mut frame = Vec::with_capacity(1 + count * POOL_ENTRY_LEN);
    frame.push(count as u8);
    for _ in 0..count * POOL_ENTRY_LEN {
        frame.push(reader.read_u8()?);
    }
    Ok(decode(&frame))
}

/// The custom (de)serialization pair handed to `register_component_custom_serde`.
#[must_use]
pub fn serialize_fns() -> SerializeFns<ReplicatedPools> {
    SerializeFns { serialize, deserialize }
}

/// Register the replicated pools on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so the protocol matches.
///
/// Reaches both client-side mirrors for the same reason vitals do: a unit is
/// drawn on its `Interpolated` copy, the player's own hero only on its
/// `Predicted` one (server#50), and the owner's resource orb is the single most
/// read piece of UI in the game.
///
/// Unpredicted for the same reason too (server#86): spending is decided by the
/// server and simulated by nobody on the client, so a predicted wallet would
/// never move on the one mirror that draws the player's own orb. See
/// [`crate::vitals::register`].
pub fn register(app: &mut App) {
    app.register_component_custom_serde::<ReplicatedPools>(serialize_fns())
        .add_interpolation_with(lerp_pools);
}
