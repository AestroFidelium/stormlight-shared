//! What the owner's tasks have been **paid** (stormlight/server#139).
//!
//! The count already crosses the wire ([`crate::stacks`]) and the declaration is
//! content both ends hold, so a client could tell how far along a task was — and
//! could not tell whether it was **finished**, which is the thing a player is
//! actually waiting for.
//!
//! # Why this cannot be derived
//!
//! It was, once. When a task was one counter and one goal, "finished" was
//! `count >= goal` and both ends could work it out from content. Two things ended
//! that:
//!
//!   - a **shortcut** finishes a task the count never got to the top of
//!     (stormlight/server#137). No amount of staring at the number says whether one
//!     fired;
//!   - a ladder pays **rung by rung**, and the rungs already handed over are a fact
//!     about the simulation's latch rather than about the tally. The counter behind
//!     a task is an ordinary reserve a mod may spend and re-earn, so a count that
//!     fell says nothing about what was paid while it was high.
//!
//! A client that decided either for itself would disagree with the simulation at
//! exactly the moments a player is watching hardest. So it is **told**: the server
//! publishes what it paid, and an interface draws that.
//!
//! # Privileged, like the four views it rides beside
//!
//! [`ReplicatedTasks`] lives on the **owner-scoped view entity** — the one the
//! ability bar ([`crate::slots`]), the XP bar ([`crate::progression`]), the talent
//! view ([`crate::talents`]) and the counters ([`crate::stacks`]) already sit on,
//! replicated with `NetworkTarget::Single(owner)`. Knowing an opponent is one rung
//! from a payout is knowing exactly when not to contest them, the same way knowing
//! their cooldowns is. One mechanism decides who receives what — which entity the
//! state sits on — rather than a second, per-component visibility rule layered over
//! the entity-level culling (stormlight/server#8).
//!
//! # What is not published
//!
//! The **thresholds**, the rewards and the shortcut's condition: all content, held
//! by both ends before the match starts and unchanged for its duration. Only the
//! record of what has been handed over moves, and it moves when a player does
//! something that pays.
//!
//! # Opaque references
//!
//! A task is named here by a tag and a number and nothing else, exactly as a counter
//! is ([`crate::stacks`]). This crate is the wire and never learns what content
//! means: it does not depend on the modding ABI, and a `TaskKey` crossing it would
//! be the protocol holding a type whose meaning lives in another layer. The two ends
//! translate at their own boundary — the server from its own key, the client through
//! the tree and the descriptors it already holds — and the wire carries two numbers.
//!
//! # Encoding
//!
//! Plain serde, like [`crate::stacks`] and [`crate::talents`] beside it, rather than
//! the hand-rolled codecs [`crate::vitals`] and [`crate::pools`] use. Those exist
//! because their state changes every tick for every visible unit; this is a handful
//! of small records, for exactly one entity per player, changing when a rung is
//! crossed.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Which of a unit's tasks a record is about, in opaque terms.
///
/// Two arms because a task reaches a unit two ways and a reader has to tell them
/// apart: one is keyed by the talent that sets it, the other by its position in the
/// unit's own list. Both are numbers this crate never interprets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TaskRef {
    /// The task set by the talent with this opaque (mod-global) handle.
    Talent(u32),
    /// The unit's own task at this position in its descriptor.
    Own(u32),
}

/// One task's paid record: which task, and how far it has been paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPaid {
    /// Which of the owner's tasks this is about.
    pub task: TaskRef,
    /// How many rungs have been handed over.
    pub rungs: u32,
    /// Whether the shortcut has fired. The half no count can be read for.
    pub shortcut: bool,
}

/// A unit's paid task records, in the server's (deterministic, key-ordered) order.
///
/// Present on the owner's view entity only while that owner's unit has actually been
/// paid something, so its presence is precisely "there is progress here worth
/// reading". A task nobody has been paid a rung of is **absent**, and an interface
/// reads that as a task at the start rather than as no task at all — which one it is
/// comes from the declaration, which the client already holds.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicatedTasks(pub Vec<TaskPaid>);

impl ReplicatedTasks {
    /// What has been paid for one task, or `None` for a task with no record.
    ///
    /// Absent rather than zeroed, because the two really are different for a reader
    /// that has to decide whether to draw anything at all — though every reader that
    /// wants a number treats them the same, which is what
    /// [`rungs`](Self::rungs) is for.
    #[must_use]
    pub fn get(&self, task: TaskRef) -> Option<TaskPaid> {
        self.0.iter().copied().find(|paid| paid.task == task)
    }

    /// How many rungs of one task have been paid. A task with no record has been
    /// paid **none**, which is what a match starts at.
    #[must_use]
    pub fn rungs(&self, task: TaskRef) -> u32 {
        self.get(task).map_or(0, |paid| paid.rungs)
    }

    /// Whether one task's shortcut has fired.
    #[must_use]
    pub fn shortcut(&self, task: TaskRef) -> bool {
        self.get(task).is_some_and(|paid| paid.shortcut)
    }
}

/// Register the owner's paid-task view on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// Neither predicted nor interpolated, for the reasons its neighbours on the same
/// entity are not ([`crate::slots`], [`crate::stacks`]): nothing on the client
/// simulates a payout, so predicting one would predict a decision the client has no
/// rule for; and easing between two snapshots would invent a fractional rung the
/// server never published — "two and a half rungs paid" is not a state this quantity
/// has.
pub fn register(app: &mut App) {
    app.register_component::<ReplicatedTasks>();
}
