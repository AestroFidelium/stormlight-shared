//! A task's payout, told as an occurrence (stormlight/server#184).
//!
//! The owner's paid record ([`crate::tasks`]) is a **level**: how far each task has
//! been paid, true for as long as it stays that way. What it cannot say is *again*:
//! a widget that lights on "at least one rung paid" fires once, and the second and
//! third rungs of a ladder land in silence. "A rung was just paid" is a fact about
//! an occurrence — it happened, on a tick — so it crosses as an event beside the
//! record, exactly as a unit's health and a damage popup are two facts about one
//! blow.
//!
//! # One occurrence per task per tick
//!
//! One increment can jump a whole ladder, and a shortcut pays every remaining rung
//! at once. Both are **one** occurrence carrying the range it paid, rather than one
//! per rung: they happened at one moment, and a flash per rung on the same frame is
//! a single flash drawn several times over. An interface that wants to say "three
//! rungs" reads the range.
//!
//! # The owner's, and only theirs — and never a backlog
//!
//! Sent to the owner of the unit and to no one else, like the record it
//! accompanies. It is an event, not state: a client that connects late, or
//! reconnects, is told what happens from then on and is never replayed what it
//! missed — a burst of old flashes on joining would announce payouts that are long
//! over.

use core::ops::Range;

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::tasks::TaskRef;

/// One tick's payout on one task.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPayout {
    /// The unit whose task it is — mapped to the receiver's local entity on arrival.
    pub unit: Entity,
    /// Which of its tasks.
    pub task: TaskRef,
    /// The first rung this tick paid.
    pub from: u32,
    /// One past the last rung it paid.
    pub to: u32,
    /// Whether the shortcut fired, paying the rest of the ladder and its own bonus.
    pub shortcut: bool,
}

impl TaskPayout {
    /// How many rungs this occurrence paid.
    #[must_use]
    pub fn rungs(&self) -> u32 {
        self.to.saturating_sub(self.from)
    }

    /// The rungs it paid, lowest first.
    #[must_use]
    pub fn paid(&self) -> Range<u32> {
        self.from..self.to.max(self.from)
    }
}

impl MapEntities for TaskPayout {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.unit = mapper.get_mapped(self.unit);
    }
}

/// The channel a payout travels on: reliable, because a payout is the one moment
/// in a task worth celebrating and losing it costs exactly that moment.
pub struct TaskPayoutChannel;

/// Register the occurrence on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
pub fn register(app: &mut App) {
    app.add_channel::<TaskPayoutChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: core::time::Duration::default(),
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ServerToClient);

    app.register_event::<TaskPayout>()
        .add_map_entities()
        .add_direction(NetworkDirection::ServerToClient);
}
