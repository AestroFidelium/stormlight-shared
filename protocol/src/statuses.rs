//! A unit's statuses on the wire (stormlight/server#171).
//!
//! A buff is server state of a privileged kind: what it modifies and by how much,
//! how long it has left, who applied it, what it does when it ends. None of that
//! crosses. What does is the one thing a drawing needs — **which buffs a unit
//! carries, and how many stacks of each** — on the unit itself, so it reaches
//! exactly the clients that can see the unit and no one else (the same
//! entity-level culling every other unit fact rides, stormlight/server#8).
//!
//! # Four edges, collapsed to two — on purpose
//!
//! Art keyed to a status may distinguish the effect becoming *active* and
//! *inactive* from the instance being *created* and *destroyed*: a stack that
//! refreshes is the first pair without the second, and collapsing the two makes a
//! refreshing debuff blink. The engine has no buff that exists without being
//! active, so here the pairs coincide — a buff **appears** and **goes** — and the
//! blink is prevented where it starts: a refresh re-publishes exactly the value
//! already on the wire, so nothing changes and nothing is sent. A change of stacks
//! is an update of the same fact, never a remove-and-add. When a buff that can be
//! present but suppressed exists, it gains an `active` flag here rather than a
//! second meaning for presence.
//!
//! # Bandwidth
//!
//! A state, not a stream of events: the component changes only when a buff is
//! applied, ends or changes its stacks, and replication sends whatever it holds at
//! the next send — so a burst of changes inside one send interval costs one update,
//! the coalescing the combat text needs a window for (stormlight/server#93) for free.
//! Plain serde for the reason [`crate::stacks`] gives: a handful of small entries
//! per unit, changing on events, not every tick.

use std::collections::BTreeMap;

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// One buff a unit carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusFact {
    /// The buff's opaque (mod-global) id.
    pub buff: u32,
    /// Its stacks, summed over every instance of it the unit holds.
    pub stacks: u16,
}

/// Every buff a unit carries, one fact per buff in buff order.
///
/// Present on a unit only while it carries something, so its absence is "nothing
/// is affecting this unit" rather than an empty list to special-case.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitStatuses(Vec<StatusFact>);

impl UnitStatuses {
    /// The set a unit holding these `(buff, stacks)` instances publishes, in
    /// whatever order they are walked.
    #[must_use]
    pub fn from_instances(instances: impl IntoIterator<Item = (u32, u16)>) -> Self {
        let mut by_buff: BTreeMap<u32, u16> = BTreeMap::new();
        for (buff, stacks) in instances {
            let total = by_buff.entry(buff).or_default();
            *total = total.saturating_add(stacks);
        }
        Self(by_buff.into_iter().map(|(buff, stacks)| StatusFact { buff, stacks }).collect())
    }

    /// Every fact, in buff order.
    #[must_use]
    pub fn facts(&self) -> &[StatusFact] {
        &self.0
    }

    /// The stacks of `buff`, or `None` for a buff the unit does not carry.
    #[must_use]
    pub fn stacks(&self, buff: u32) -> Option<u16> {
        self.0.iter().find(|fact| fact.buff == buff).map(|fact| fact.stacks)
    }

    /// Whether the unit carries nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What changed between two of a unit's sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusEdges {
    /// Buffs carried now and not before.
    pub appeared: Vec<u32>,
    /// Buffs carried before and not now.
    pub gone: Vec<u32>,
    /// Buffs carried both times with different stacks.
    pub restacked: Vec<u32>,
}

impl StatusEdges {
    /// Whether nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.appeared.is_empty() && self.gone.is_empty() && self.restacked.is_empty()
    }
}

/// The edges from `before` to `after`. Total over any two sets, however they were
/// built: it reads them as maps, not as sorted lists.
#[must_use]
pub fn edges(before: &UnitStatuses, after: &UnitStatuses) -> StatusEdges {
    let was: BTreeMap<u32, u16> = before.0.iter().map(|f| (f.buff, f.stacks)).collect();
    let now: BTreeMap<u32, u16> = after.0.iter().map(|f| (f.buff, f.stacks)).collect();
    StatusEdges {
        appeared: now.keys().filter(|b| !was.contains_key(b)).copied().collect(),
        gone: was.keys().filter(|b| !now.contains_key(b)).copied().collect(),
        restacked: now
            .iter()
            .filter(|(b, n)| was.get(b).is_some_and(|w| w != *n))
            .map(|(b, _)| *b)
            .collect(),
    }
}

/// Register a unit's statuses on both ends. Called from
/// [`ProtocolPlugin`](crate::protocol::ProtocolPlugin) so server and client agree
/// byte-for-byte.
///
/// Neither predicted nor interpolated: nothing on the client applies a buff, so
/// there is no rule to predict one with, and a status is either there or not —
/// there is nothing between two snapshots of it to ease through.
pub fn register(app: &mut App) {
    app.register_component::<UnitStatuses>();
}
