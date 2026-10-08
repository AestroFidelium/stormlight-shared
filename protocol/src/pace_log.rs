//! When a unit's pace changed, by tick (stormlight/server#225).
//!
//! A predicting client steps its own unit tick by tick — walking, and carrying out
//! a motion — and on a rollback steps the same ticks *again*. Both have to use the
//! pace the server used on each of those ticks, not the pace the unit happens to
//! have now: re-simulating the ticks before a unit walked into a stop with the
//! stop's pace would put it short of where it really stopped, and every snapshot
//! after would correct it again.
//!
//! [`PaceLog`] is the server's record of when the pace changed: tick, new pace,
//! newest last, a few entries deep. From it, [`PaceLog::pace_at`] answers what the
//! pace was on any tick the log still covers — and the log covers far more than
//! the rollback window, so a replay never reaches past it.
//!
//! Absent on a unit whose pace never changed, which then runs at the world's pace
//! on every tick. Attached on the first change and only updated after, like every
//! other server-authoritative fact a predicted unit carries.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::time_scale::TimeScale;

/// How many changes the log keeps. A pace changes when a unit crosses a field's
/// edge or a field comes or goes — a handful of times a fight — and the oldest
/// entry a replay can need is one rollback window old.
pub const PACE_LOG_DEPTH: usize = 16;

/// When a unit's pace changed, oldest first. See the module docs.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaceLog {
    changes: Vec<(Tick, TimeScale)>,
    /// Whether an old change has been dropped — after which the log only answers
    /// for ticks from its oldest kept change on.
    truncated: bool,
}

impl PaceLog {
    /// From tick `from` on, the pace is `pace`.
    ///
    /// A change that restates the pace already in force is not a change and is not
    /// kept; a change at a tick already logged replaces it (the last word for a
    /// tick is the pace that tick ran at). Past [`PACE_LOG_DEPTH`] the oldest
    /// change is dropped.
    pub fn record(&mut self, from: Tick, pace: TimeScale) {
        if let Some(&(last, _)) = self.changes.last()
            && last == from
        {
            // The last word for this tick — and if it only restates the pace in
            // force before it, the tick changed nothing after all.
            self.changes.pop();
            if self.pace_at(from) != pace {
                self.changes.push((from, pace));
            }
            return;
        }
        if self.pace_at(from) == pace {
            return;
        }
        self.changes.push((from, pace));
        if self.changes.len() > PACE_LOG_DEPTH {
            self.changes.remove(0);
            self.truncated = true;
        }
    }

    /// Whether the log still knows what pace `tick` ran at: always, until an old
    /// change has been dropped; from the oldest kept change on, after.
    #[must_use]
    pub fn covers(&self, tick: Tick) -> bool {
        !self.truncated || self.changes.first().is_some_and(|(oldest, _)| tick - *oldest >= 0)
    }

    /// The pace tick `tick` ran at: the latest change at or before it, or the
    /// world's pace before the first. Meaningful for the ticks the log
    /// [covers](PaceLog::covers) — far more than any rollback reaches.
    #[must_use]
    pub fn pace_at(&self, tick: Tick) -> TimeScale {
        self.changes
            .iter()
            .rev()
            .find(|(from, _)| tick - *from >= 0)
            .map_or(TimeScale::NORMAL, |(_, pace)| *pace)
    }

    /// Every change still kept, oldest first.
    #[must_use]
    pub fn changes(&self) -> &[(Tick, TimeScale)] {
        &self.changes
    }
}

/// Register the log for replication on both ends.
///
/// **Not predicted**: it is the server's record and nothing on a client writes it.
/// Interpolated with the confirmed value, so a smoothed copy carries it too.
pub fn register(app: &mut App) {
    app.register_component::<PaceLog>().add_interpolation_with(|_start, end, _t| end.clone());
}
