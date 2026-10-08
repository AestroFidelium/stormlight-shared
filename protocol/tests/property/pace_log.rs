//! The record of when a unit's pace changed (stormlight/server#225).
//!
//! A predicting client replays past ticks on a rollback, and each has to run at the
//! pace the server ran it at. Model-based, over arbitrary sequences of pace changes
//! at increasing ticks:
//! - on any tick the log still covers, it answers the pace the model says that
//!   tick ran at — the latest change at or before it, the world's pace before any;
//! - it never grows past its depth, and a change that restates the pace in force
//!   is not kept.

use bolero::{TypeGenerator, check};
use lightyear::prelude::Tick;
use stormlight_shared::pace_log::{PACE_LOG_DEPTH, PaceLog};
use stormlight_shared::time_scale::TimeScale;

#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Change {
    /// Ticks after the previous change (0 = the same tick).
    #[generator(0..=20u16)]
    after: u16,
    /// A handful of paces, so restating one is common.
    #[generator(0..=4u8)]
    pace: u8,
}

fn pace(seed: u8) -> TimeScale {
    TimeScale::from_per_mille(u16::from(seed) * 500)
}

#[derive(Debug, TypeGenerator)]
struct Story {
    start: u16,
    #[generator(bolero::produce::<Vec<Change>>().with().len(0usize..=40))]
    changes: Vec<Change>,
}

/// (tick, pace) as applied, the last word for each tick only.
fn model(story: &Story) -> Vec<(u16, TimeScale)> {
    let mut out: Vec<(u16, TimeScale)> = Vec::new();
    let mut tick = story.start;
    for change in &story.changes {
        tick = tick.wrapping_add(change.after);
        match out.last_mut() {
            Some(last) if last.0 == tick => last.1 = pace(change.pace),
            _ => out.push((tick, pace(change.pace))),
        }
    }
    out
}

fn logged(story: &Story) -> PaceLog {
    let mut log = PaceLog::default();
    let mut tick = story.start;
    for change in &story.changes {
        tick = tick.wrapping_add(change.after);
        log.record(Tick(tick), pace(change.pace));
    }
    log
}

#[test]
fn the_log_answers_the_pace_every_covered_tick_ran_at() {
    check!().with_type::<Story>().for_each(|story| {
        let log = logged(story);
        let applied = model(story);
        // The ticks the log still covers: from its oldest kept change on, or every
        // tick if it never had to drop one.
        let span = applied.last().map_or(0, |l| l.0.wrapping_sub(story.start)) + 3;
        for offset in 0..span {
            let tick = story.start.wrapping_add(offset);
            if !log.covers(Tick(tick)) {
                continue;
            }
            // The ticks here span far less than half the wrapping range, so offsets
            // from the start order them.
            let expected = applied
                .iter()
                .rev()
                .find(|(from, _)| from.wrapping_sub(story.start) <= offset)
                .map_or(TimeScale::NORMAL, |(_, p)| *p);
            assert_eq!(log.pace_at(Tick(tick)), expected, "tick {tick}");
        }
    });
}

#[test]
fn the_log_is_bounded_and_keeps_only_real_changes() {
    check!().with_type::<Story>().for_each(|story| {
        let log = logged(story);
        assert!(log.changes().len() <= PACE_LOG_DEPTH);
        for pair in log.changes().windows(2) {
            assert_ne!(pair[0].1, pair[1].1, "a change restating the pace in force was kept");
        }
        // Before anything was dropped, the first change is a change from the world's
        // pace.
        if let Some(first) = log.changes().first()
            && log.covers(Tick(story.start))
        {
            assert_ne!(first.1, TimeScale::NORMAL, "the world's pace was logged as a change");
        }
    });
}
