//! The owner's ability slots on the wire — what each key is bound to and
//! whether it is ready (`stormlight/server#58`).
//!
//! Everything a player needs to read their own ability bar lives server-side:
//! the slot→ability binding (after talents resolve it), the per-slot cooldown,
//! the charge balance and the affordability check. Without this the client
//! presses a key and finds out by whether anything happened. This is the small,
//! read-only projection of that state.
//!
//! # Owner-scoped, by the one visibility mechanism there is
//!
//! Unlike [vitals](crate::vitals) and [pools](crate::pools) — public, because any
//! client that can see a unit may see its bars — a caster's slot state is
//! **privileged per-player**: what your abilities cost and when they come back up
//! is not something an opponent gets to read.
//!
//! Rather than layering a second, per-component visibility rule over the
//! entity-level area-of-interest culling (`stormlight/server#8`), the slot view
//! rides its **own entity**, replicated with `NetworkTarget::Single(owner)`. One
//! mechanism still decides who receives what, and "no other player sees my slots"
//! is a property of *which entity* the state sits on rather than of a peer→sender
//! map that has to stay correct across every connect and disconnect. A client
//! therefore receives exactly one of these: its own.
//!
//! The view deliberately carries **no reference to the caster**. It could — an
//! entity field mapped across the wire — but a mapped reference is resolved once,
//! at the moment the component is deserialized, and is never retried: a view
//! whose frame happens to arrive before its caster's spawn keeps a permanently
//! dangling reference. Since the view is already scoped to one player, the client
//! does not need to be told which unit it is about; it drives exactly one, and
//! Lightyear already marks that unit `Predicted` for it. The pairing stays
//! server-side, where both entities are real.
//!
//! # A deadline, not a countdown
//!
//! A cooldown is replicated as the tick it becomes **ready at**, plus the length
//! it started from. The client sweeps toward that deadline against its own
//! timeline, so a twelve-slot bar costs bytes when a cooldown *starts* — not on
//! every one of the sixty-four ticks a second it is running. It also means the
//! client cannot drift: it never integrates a remaining time, it just compares
//! two ticks.
//!
//! [`Tick`] is a wrapping `u16` compared by signed difference, so a deadline
//! stays meaningful across the wrap. The representable window is
//! [`MAX_COOLDOWN_TICKS`] ticks ahead — past that a deadline would alias into the
//! past, so the server clamps rather than publishing a cooldown that reads as
//! already expired.
//!
//! # Encoding
//!
//! Variable-length, like [`crate::pools`]: a `u8` count followed by that many
//! fixed [`SLOT_ENTRY_LEN`]-byte entries. A caster's slot count is content-defined
//! — the protocol has no idea how many abilities a mod gives a unit.
//!
//! | field            | encoding | bytes |
//! | ---------------- | -------- | ----- |
//! | `slot`           | `u8`     | 1     |
//! | `ability`        | `u32`    | 4     |
//! | `ready_at`       | `u16`    | 2     |
//! | `cooldown_ticks` | `u16`    | 2     |
//! | `charges`        | `u8`     | 1     |
//! | `flags`          | `u8`     | 1     |
//!
//! Content-free: an opaque ability id, a raw slot index, and a tick. Nothing here
//! knows which hero, ability or mod the binding came from.

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_serde::SerializationError;
use lightyear_serde::reader::{ReadInteger, Reader};
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::{WriteInteger, Writer};
use serde::{Deserialize, Serialize};

/// Bytes per encoded slot: 1 (slot) + 4 (ability) + 2 (deadline) + 2 (length) +
/// 1 (charges) + 1 (flags).
pub const SLOT_ENTRY_LEN: usize = 11;

/// Most slots one caster can replicate — the `u8` count's range. Far past the
/// handful a unit realistically binds; a longer list is truncated rather than
/// corrupting the frame.
pub const MAX_SLOTS: usize = u8::MAX as usize;

/// Furthest a deadline may sit ahead of the current tick. [`Tick`] is a wrapping
/// `u16` ordered by signed difference, so anything beyond half the window would
/// read as being in the *past* — a cooldown that presents itself as already
/// expired. At 64 Hz this is still upwards of eight minutes, past any cooldown a
/// mod plausibly declares.
pub const MAX_COOLDOWN_TICKS: u16 = i16::MAX as u16;

/// Flag bit: the caster can currently pay this slot's costs.
pub const AFFORDABLE: u8 = 1 << 0;

/// Flag bit: nothing is blocking the cast — the ability's own cast gate passes
/// and the caster carries no cast-blocking status.
pub const UNGATED: u8 = 1 << 1;

/// One ability slot as its owner sees it.
///
/// Deliberately *not* interpolated: a deadline is a discrete fact, and easing two
/// snapshots of it would invent a tick the server never published. The client
/// derives every continuous quantity (remaining time, fill fraction) from the
/// deadline against its own clock instead.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotState {
    /// The slot index this binding sits in — the raw key the player presses.
    pub slot: u8,
    /// The bound ability's opaque global id: the client's key for the icon and
    /// the cosmetic a mod declared for it. `0` means no binding.
    pub ability: u32,
    /// The tick the slot comes off cooldown. At or before the client's current
    /// tick, the slot is ready.
    pub ready_at: Tick,
    /// The cooldown's full length in ticks — the denominator of the sweep, so the
    /// client can draw how far through it is. `0` for a slot with no cooldown.
    pub cooldown_ticks: u16,
    /// Charges the caster can spend on this slot. Charges are a caster-level
    /// balance rather than a per-slot one, so every slot reports the same count;
    /// what differs is whether the slot's costs actually draw on it.
    pub charges: u8,
    /// [`AFFORDABLE`] / [`UNGATED`] — the reasons a key greys out that are *not*
    /// the cooldown. Kept as separate bits so the HUD can say which one it is.
    pub flags: u8,
}

impl SlotState {
    /// Ticks left before the slot is ready, at the client's current tick.
    ///
    /// Signed-difference comparison, so this stays correct across the `u16` wrap.
    /// Saturates at zero: a deadline already passed reports no time remaining
    /// rather than a huge one.
    #[must_use]
    pub fn remaining_ticks(&self, now: Tick) -> u16 {
        u16::try_from(self.ready_at - now).unwrap_or(0)
    }

    /// Whether the cooldown has elapsed at `now`.
    #[must_use]
    pub fn ready(&self, now: Tick) -> bool {
        self.remaining_ticks(now) == 0
    }

    /// How much of the cooldown is still to run, in `[0, 1]` — `1` the tick it
    /// started, `0` once ready. A slot with no cooldown length reads as ready
    /// rather than dividing by zero, and a frame whose deadline outruns its own
    /// declared length clamps instead of overfilling its track.
    #[must_use]
    pub fn fraction(&self, now: Tick) -> f32 {
        if self.cooldown_ticks == 0 {
            return 0.0;
        }
        (f32::from(self.remaining_ticks(now)) / f32::from(self.cooldown_ticks)).clamp(0.0, 1.0)
    }

    /// Whether the caster can currently pay this slot's costs.
    #[must_use]
    pub fn affordable(&self) -> bool {
        self.flags & AFFORDABLE != 0
    }

    /// Whether the cast gate and the caster's status allow the cast.
    #[must_use]
    pub fn ungated(&self) -> bool {
        self.flags & UNGATED != 0
    }

    /// The verdict a bar greys out on: off cooldown, affordable, and unblocked.
    /// The cooldown half is *derived* from the deadline rather than sent, so it
    /// stays true between snapshots as the client's clock advances.
    #[must_use]
    pub fn castable(&self, now: Tick) -> bool {
        self.ready(now) && self.affordable() && self.ungated()
    }
}

/// A caster's replicated slot set, in slot order.
///
/// Lives on its own owner-scoped entity (see the module docs), never on the
/// caster itself — the caster is replicated to everyone.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicatedSlots(pub Vec<SlotState>);

impl ReplicatedSlots {
    /// The state of one slot, if the caster binds it.
    #[must_use]
    pub fn slot(&self, slot: u8) -> Option<&SlotState> {
        self.0.iter().find(|s| s.slot == slot)
    }
}

/// Encode a slot set into its self-describing wire frame.
#[must_use]
pub fn encode(slots: &ReplicatedSlots) -> Vec<u8> {
    let kept = slots.0.len().min(MAX_SLOTS);
    let mut out = Vec::with_capacity(1 + kept * SLOT_ENTRY_LEN);
    out.push(kept as u8);
    for slot in slots.0.iter().take(kept) {
        out.push(slot.slot);
        out.extend_from_slice(&slot.ability.to_be_bytes());
        out.extend_from_slice(&slot.ready_at.0.to_be_bytes());
        out.extend_from_slice(&slot.cooldown_ticks.to_be_bytes());
        out.push(slot.charges);
        out.push(slot.flags);
    }
    out
}

/// Decode a slot frame. Total: a truncated or hostile frame yields only the
/// entries actually present, never an over-read or a panic.
#[must_use]
pub fn decode(bytes: &[u8]) -> ReplicatedSlots {
    let Some((&count, mut rest)) = bytes.split_first() else {
        return ReplicatedSlots::default();
    };
    // The count byte is attacker-controlled: trust it only as far as the bytes
    // that actually follow it.
    let count = usize::from(count).min(rest.len() / SLOT_ENTRY_LEN);
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, tail) = rest.split_at(SLOT_ENTRY_LEN);
        rest = tail;
        slots.push(SlotState {
            slot: entry[0],
            ability: u32::from_be_bytes([entry[1], entry[2], entry[3], entry[4]]),
            ready_at: Tick(u16::from_be_bytes([entry[5], entry[6]])),
            cooldown_ticks: u16::from_be_bytes([entry[7], entry[8]]),
            charges: entry[9],
            flags: entry[10],
        });
    }
    ReplicatedSlots(slots)
}

/// Serialize a slot set into the Lightyear wire buffer using [`encode`].
fn serialize(slots: &ReplicatedSlots, writer: &mut Writer) -> Result<(), SerializationError> {
    for byte in encode(slots) {
        writer.write_u8(byte)?;
    }
    Ok(())
}

/// Deserialize a slot set from the Lightyear wire buffer. Reads the count byte
/// first, then exactly that many entries — the frame sizes itself, so it can
/// share a buffer with whatever follows it.
fn deserialize(reader: &mut Reader) -> Result<ReplicatedSlots, SerializationError> {
    let count = usize::from(reader.read_u8()?);
    let mut frame = Vec::with_capacity(1 + count * SLOT_ENTRY_LEN);
    frame.push(count as u8);
    for _ in 0..count * SLOT_ENTRY_LEN {
        frame.push(reader.read_u8()?);
    }
    Ok(decode(&frame))
}

/// The custom (de)serialization pair handed to `register_component_custom_serde`.
#[must_use]
pub fn serialize_fns() -> SerializeFns<ReplicatedSlots> {
    SerializeFns { serialize, deserialize }
}

/// Register the owner-scoped slot view on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so the protocol matches.
///
/// Not predicted and not interpolated: the view entity is a plain confirmed
/// replica, not a mirror of a simulated unit. Easing a deadline would invent
/// ticks the server never published, and predicting one would let the client's
/// own guess decide when its abilities come up — which is exactly the authority
/// this view exists to hand back to the server.
pub fn register(app: &mut App) {
    app.register_component_custom_serde::<ReplicatedSlots>(serialize_fns());
}
