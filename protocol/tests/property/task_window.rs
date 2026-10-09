//! Whether a task's shortcut window is open, on the wire (stormlight/server#185).
//!
//! The owner's task view carries the paid record and, beside it, the one fact
//! about the present: whether the shortcut would fire now. Invariants:
//!   - **The window is read off the entry, and only off it**: a task with no entry
//!     reads closed, never open;
//!   - **It is independent of the record**: telling a task's window never changes
//!     what was paid for it, and the paid half never opens a window.

use bolero::{TypeGenerator, check};
use stormlight_shared::tasks::{ReplicatedTasks, TaskPaid, TaskRef};

#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Entry {
    #[generator(0u32..=4)]
    talent: u32,
    rungs: u8,
    shortcut: bool,
    window: bool,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Entry>>().with().len(0usize..=5))]
    entries: Vec<Entry>,
    #[generator(0u32..=6)]
    asked: u32,
}

#[test]
fn a_tasks_window_is_its_entrys_and_closed_without_one() {
    check!().with_type::<Scenario>().for_each(|s| {
        // One entry per task, as the server publishes it: the first wins.
        let mut seen = Vec::new();
        let entries: Vec<TaskPaid> = s
            .entries
            .iter()
            .filter(|e| {
                let fresh = !seen.contains(&e.talent);
                seen.push(e.talent);
                fresh
            })
            .map(|e| TaskPaid {
                task: TaskRef::Talent(e.talent),
                rungs: u32::from(e.rungs),
                shortcut: e.shortcut,
                window: e.window,
            })
            .collect();
        let view = ReplicatedTasks(entries.clone());
        let task = TaskRef::Talent(s.asked);
        let entry = entries.iter().find(|e| e.task == task);
        assert_eq!(view.window(task), entry.is_some_and(|e| e.window), "{s:?}");
        assert_eq!(view.rungs(task), entry.map_or(0, |e| e.rungs), "{s:?}");
        assert_eq!(view.shortcut(task), entry.is_some_and(|e| e.shortcut), "{s:?}");
    });
}
