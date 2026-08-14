//! The interface's own wire request (stormlight/server#69) — a declared widget
//! asking the simulation for something the closed verbs do not cover.
//!
//! A mod's HUD can already ask for the two things a player does to their own unit:
//! [`CastIntent`](crate::cast::CastIntent) casts a slot and
//! [`TalentPick`](crate::talents::TalentPick) takes a talent. Both are requests
//! the server validates, and neither needed a new message for the interface —
//! a clicked ability slot sends the *same* `CastIntent` the keybind does, which is
//! the point.
//!
//! [`UiTrigger`] is the third: a button that raises a **mod-defined event**. It is
//! what keeps the action set closed without making the interface useless — a mod
//! that wants a recall button, a ping, a shop purchase declares an event, exports
//! `mod_trigger`, and gets the whole effect ISA back. What it does *not* get is a
//! way around the engine: the guest runs server-side, its effects are dispatched
//! against the clicking player's own unit, and the client's part is one `u32`.
//!
//! # Why this is not a client-side call
//!
//! The obvious-looking alternative is to run the trigger in the *cosmetic* mod's
//! own guest, on the client. It cannot work, and the reasons are worth writing
//! down because the shape looks so close: a guest's only output is a list of
//! [`Impact`](stormlight_mod_abi::impacts::Impact)s, the client holds no
//! simulation state to apply them to, and anything it *could* apply locally would
//! be a client deciding the game. The trigger belongs on the authoritative side,
//! and this message is how it gets there.
//!
//! # It carries no entity, and no payload
//!
//! No entity, for [`TalentPick`]'s reason: the server already knows which unit a
//! peer drives, so putting the caster on the wire would only be a reference to get
//! wrong or to lie about.
//!
//! No payload either, which is the sharper call. An `Emit` inside the ISA carries
//! one because the *simulation* computed it; a number computed on the client and
//! handed to a guest would be an unvalidated value from an untrusted process
//! reaching straight into effect evaluation. A guest that needs a number reads it
//! off the world it already has.
//!
//! # Encoding
//!
//! Reliable and unordered, exactly like the two requests it sits beside. A dropped
//! trigger reads as the game ignoring a click, and two triggers need no mutual
//! ordering — each names its own event, and a mod that cares about the order of
//! its own events is deciding that server-side where the tick is.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Client→server request: raise the mod-defined event with this **global** id.
///
/// The field is raw and untrusted. The server routes it to whichever mods declared
/// that event and exported `mod_trigger`, against the unit the sender actually
/// controls; an id no mod subscribes to reaches nobody and changes nothing, which
/// is the honest outcome for a stale client and for a hostile one alike.
///
/// Content-free: an opaque handle, never an event *name* and never what it does.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiTrigger {
    /// The event's opaque global id, as the declaring mod's `Names` interned it.
    pub event: u32,
}

/// Reliable channel the UI triggers ride. Unordered-reliable, like the cast
/// intents and talent picks: every one must arrive, none needs ordering.
pub struct UiTriggerChannel;

/// Register the UI-trigger wire contract on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// No entity mapping, because the message carries no entity; no replicated
/// component, because nothing about a trigger is state — it is an instantaneous
/// request, and what it *changes* replicates through whatever its effects touched.
pub fn register(app: &mut App) {
    app.add_channel::<UiTriggerChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);

    app.register_event::<UiTrigger>().add_direction(NetworkDirection::ClientToServer);
}
