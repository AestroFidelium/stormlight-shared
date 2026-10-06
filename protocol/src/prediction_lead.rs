//! How far ahead of the server a predicting client runs (stormlight/server#146).
//!
//! Prediction only ever corrects itself by comparing an authoritative snapshot
//! against what the client predicted *for that same tick*, and Lightyear skips the
//! comparison for a snapshot stamped later than the client's own tick: it has no
//! prediction for the future to compare against. Its timeline aims at the server's
//! tick plus half the round trip plus a jitter margin — on a near-zero round trip
//! (a local server, an in-process test) that is a fraction of a tick, so the client
//! settles level with the server or a tick behind it, every snapshot arrives "from
//! the future", and no rollback ever happens. Whatever the client predicted wrong —
//! a walk the server steered around a body, say — then stays wrong for good.
//!
//! Raising the jitter margin to a whole number of ticks keeps the client strictly
//! ahead whatever the round trip. On a real network it costs those ticks of extra
//! rollback depth and input lead, both of which the margin was already spending.

use bevy::prelude::*;
use lightyear::prelude::client::{Client, InputTimelineConfig};
use lightyear::prelude::{SyncConfig, Tick};

use crate::connection::tick_duration;

/// The fewest ticks a predicting client's timeline runs ahead of the server, on
/// top of half the round trip. Two rather than one: the sync tolerates about a
/// tick of error before it steers, so a one-tick lead can still dip level.
pub const PREDICTION_LEAD_TICKS: u32 = 2;

/// The server tick a request the client sends now lands on, in the server's count.
///
/// The client runs half a round trip plus [`PREDICTION_LEAD_TICKS`] ahead of the
/// server. The half round trip is spent in flight; the lead is not — so anything
/// the client compares against a *server* deadline to decide whether a request
/// would be accepted (a cooldown coming back up) must compare this, not its own
/// tick, or it lights a key the lead's worth of ticks before the server takes it.
#[must_use]
pub fn arrival_tick(local: Tick) -> Tick {
    // Tick space wraps, so the step back does too.
    Tick(local.0.wrapping_sub(u16::try_from(PREDICTION_LEAD_TICKS).unwrap_or(u16::MAX)))
}

/// The sync configuration every predicting client runs with.
#[must_use]
pub fn prediction_sync_config() -> SyncConfig {
    SyncConfig { jitter_margin: tick_duration() * PREDICTION_LEAD_TICKS, ..SyncConfig::default() }
}

/// `Add`-observer: give every client endpoint the lead, whoever spawned it — the
/// real client and the in-process test rigs alike — so none can forget it.
pub(crate) fn lead_the_server(add: On<Add, Client>, mut commands: Commands) {
    commands
        .entity(add.entity)
        .insert(InputTimelineConfig::default().with_sync_config(prediction_sync_config()));
}
