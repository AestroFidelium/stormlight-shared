//! Who is playing this match, on which side, driving which unit
//! (`stormlight/server#145`).
//!
//! Every other replicated fact in this crate hangs off a **unit**: vitals, pools,
//! the cast in progress, the identity tag. A roster is the first one that does
//! not, and it exists precisely because a unit cannot carry it:
//!
//! - it outlives a body. A player whose unit is dead, not yet spawned, or gone is
//!   still in the match, and a scoreboard, a death timer or a kill feed all have
//!   to name them while there is nothing to hang a component on;
//! - it has to reach a client that cannot see the body at all. Replication is
//!   culled by area of interest (`stormlight/server#8`), so an opponent across the
//!   map is not replicated to you — and they are exactly who the other half of a
//!   top bar is about.
//!
//! # A seat, not an entity reference
//!
//! The link between an entry and the body it describes is a [`Seat`]: a small
//! stable number the match setup hands out, replicated onto the unit and repeated
//! in the entry. The client joins the two locally, every frame, the same way it
//! rebuilds a unit's visual from its
//! [`UnitTag`](crate::identity::UnitTag).
//!
//! It is deliberately **not** a mapped `Entity` field. Lightyear resolves a mapped
//! reference once, at the moment the component is deserialized, and never retries:
//! a roster frame that arrives before the unit it points at — which is the ordinary
//! case, since the roster reaches everybody and the unit does not — would hold a
//! dangling reference for the rest of the match. A number that is re-joined every
//! frame cannot go stale, and it degrades correctly: a seat whose body this client
//! cannot see simply finds nothing, which is the honest answer rather than a
//! reference to the wrong entity.
//!
//! # What it deliberately does not say
//!
//! Identity, side and the unit link, and nothing else. A roster is replicated to
//! every client unconditionally, so anything added here is a fact every opponent
//! learns for free — a position, a resource, a cooldown put here would quietly
//! undo the area-of-interest culling it is designed to sidestep. What a player is
//! *drawn as* is resolved from the unit id by the cosmetic layer, client-side, and
//! costs the wire nothing.
//!
//! # The side is the match setup's, not the unit's
//!
//! An entry's side comes from the seating, not from the team its body happens to
//! carry — otherwise a player with no body would have no side, which is the one
//! state the roster exists to describe. Nothing here knows how many sides there
//! are, how many players each holds, or what any of them is called.
//!
//! # Encoding
//!
//! Variable-length, like [`crate::slots`] and [`crate::pools`]: a `u8` count
//! followed by that many fixed [`ROSTER_ENTRY_LEN`]-byte entries. How many players
//! a match seats is the match setup's business, so the frame sizes itself.
//!
//! | field  | encoding | bytes |
//! | ------ | -------- | ----- |
//! | `seat` | `u16`    | 2     |
//! | `side` | `u16`    | 2     |
//! | `unit` | `u32`    | 4     |
//! | flags  | `u8`     | 1     |
//!
//! The unit id needs the flag byte beside it because **`0` is an ordinary unit**:
//! ids are interned into a dense `0..len` space, so the first unit any mod declares
//! is id zero. A roster that spelled "drives nothing" as `0` would draw that hero's
//! portrait over every dead player. [`DRIVING`] says whether the four bytes mean
//! anything at all.

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_serde::SerializationError;
use lightyear_serde::reader::{ReadInteger, Reader};
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::{WriteInteger, Writer};
use serde::{Deserialize, Serialize};

/// Bytes per encoded seat: 2 (seat) + 2 (side) + 4 (unit) + 1 (flags).
pub const ROSTER_ENTRY_LEN: usize = 9;

/// Most players one roster can replicate — the `u8` count's range. Far past any
/// match format; a longer list is truncated rather than corrupting the frame.
pub const MAX_SEATED: usize = u8::MAX as usize;

/// Flag bit: this seat currently has a body, so its `unit` id is meaningful.
pub const DRIVING: u8 = 1 << 0;

/// Which seated player drives a unit — the link a roster entry is joined to its
/// body by, and the reason the join can be done locally rather than resolved on
/// the wire.
///
/// Replicated onto the unit itself and stable across a respawn, because a respawn
/// stands the *same* entity back up rather than spawning a fresh one — so the seat
/// on it never has to be re-established. Absent on everything a player does not
/// drive: a creep, a building, a map-placed dummy.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Seat(pub u16);

/// One seated player, as everybody in the match sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    /// The seat they hold — stable for as long as they are in the match, never
    /// reused by a later joiner, and the key their body carries as [`Seat`].
    pub seat: u16,
    /// Which side they fight for, as the match setup grouped them. An opaque
    /// number: two players are on the same side iff it is equal.
    pub side: u16,
    /// The global id of the unit descriptor they are currently driving, or `None`
    /// while they have no body at all.
    ///
    /// A **copy**, not a reference to the live entity, and that is what lets a
    /// client draw an opponent it has never received: the portrait resolves out of
    /// the cosmetic tables from this id alone. The live entity — needed for
    /// anything that reads the body's state — is found by joining [`Seat`], and is
    /// legitimately absent for a unit outside this client's interest area.
    pub unit: Option<u32>,
}

/// Everyone seated in this match, in seat order.
///
/// Lives on one match-global entity replicated to every client. Not a per-player
/// view: a roster that was culled would be a roster that could not answer the one
/// question it exists for.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster(pub Vec<RosterEntry>);

impl Roster {
    /// The player holding `seat`, if anybody does.
    ///
    /// The first match, which is also the only one for any roster the server
    /// published: seats are handed out from a monotonic counter and never reused.
    /// A decoded frame is not trusted to have kept that promise, and answering with
    /// the first entry is the one reading that cannot depend on the rest of it.
    #[must_use]
    pub fn entry(&self, seat: u16) -> Option<&RosterEntry> {
        self.0.iter().find(|e| e.seat == seat)
    }

    /// Which side the player holding `seat` fights for, if anybody holds it.
    #[must_use]
    pub fn side_of(&self, seat: u16) -> Option<u16> {
        self.entry(seat).map(|e| e.side)
    }

    /// How many players are seated.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nobody has joined yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Encode a roster into its self-describing wire frame.
#[must_use]
pub fn encode(roster: &Roster) -> Vec<u8> {
    let kept = roster.0.len().min(MAX_SEATED);
    let mut out = Vec::with_capacity(1 + kept * ROSTER_ENTRY_LEN);
    out.push(kept as u8);
    for entry in roster.0.iter().take(kept) {
        out.extend_from_slice(&entry.seat.to_be_bytes());
        out.extend_from_slice(&entry.side.to_be_bytes());
        out.extend_from_slice(&entry.unit.unwrap_or(0).to_be_bytes());
        out.push(if entry.unit.is_some() { DRIVING } else { 0 });
    }
    out
}

/// Decode a roster frame. Total: a truncated or hostile frame yields only the
/// entries actually present, never an over-read or a panic.
#[must_use]
pub fn decode(bytes: &[u8]) -> Roster {
    let Some((&count, mut rest)) = bytes.split_first() else {
        return Roster::default();
    };
    // The count byte is attacker-controlled: trust it only as far as the bytes
    // that actually follow it.
    let count = usize::from(count).min(rest.len() / ROSTER_ENTRY_LEN);
    let mut seats = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, tail) = rest.split_at(ROSTER_ENTRY_LEN);
        rest = tail;
        let unit = u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]]);
        seats.push(RosterEntry {
            seat: u16::from_be_bytes([entry[0], entry[1]]),
            side: u16::from_be_bytes([entry[2], entry[3]]),
            unit: (entry[8] & DRIVING != 0).then_some(unit),
        });
    }
    Roster(seats)
}

/// Serialize a roster into the Lightyear wire buffer using [`encode`].
fn serialize(roster: &Roster, writer: &mut Writer) -> Result<(), SerializationError> {
    for byte in encode(roster) {
        writer.write_u8(byte)?;
    }
    Ok(())
}

/// Deserialize a roster from the Lightyear wire buffer. Reads the count byte
/// first, then exactly that many entries — the frame sizes itself, so it can share
/// a buffer with whatever follows it.
fn deserialize(reader: &mut Reader) -> Result<Roster, SerializationError> {
    let count = usize::from(reader.read_u8()?);
    let mut frame = Vec::with_capacity(1 + count * ROSTER_ENTRY_LEN);
    frame.push(count as u8);
    for _ in 0..count * ROSTER_ENTRY_LEN {
        frame.push(reader.read_u8()?);
    }
    Ok(decode(&frame))
}

/// The custom (de)serialization pair handed to `register_component_custom_serde`.
#[must_use]
pub fn serialize_fns() -> SerializeFns<Roster> {
    SerializeFns { serialize, deserialize }
}

/// Register the roster and the seat link on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so the protocol matches.
///
/// [`Seat`] syncs onto **both** client-side mirrors of a replicated unit, exactly
/// as [`UnitTag`](crate::identity::UnitTag) does and for the same reason: a roster
/// row resolves its player's body wherever that body is drawn, and the unit this
/// player drives is only ever `Predicted` while everybody else's is `Interpolated`.
/// A seat number has no meaningful in-between, so its interpolation is the identity
/// — take the confirmed value.
///
/// Neither is predicted. Prediction is for state the client *simulates*, and
/// nothing on it decides who is playing (`stormlight/server#86`).
pub fn register(app: &mut App) {
    app.register_component::<Seat>().add_interpolation_with(|_start, end, _t| end);
    app.register_component_custom_serde::<Roster>(serialize_fns());
}
