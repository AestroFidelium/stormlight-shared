//! A unit's statuses on the wire (stormlight/server#171).
//!
//! What crosses is which buffs a unit carries and how many stacks of each — never
//! what they modify, for how long, or who applied them. Invariants:
//!   - **One fact per buff, in buff order**: however many instances the server
//!     holds, and in whatever order it walks them, the published set is the same;
//!   - **A refresh does not blink**: re-applying what is already there publishes
//!     exactly the value that was there, so nothing on the wire changes;
//!   - **Stacks add up** across a buff's instances, and never wrap;
//!   - **The edges are the whole difference**: what appeared, what went and what
//!     changed its stacks between two sets are disjoint, and together with what
//!     stayed they rebuild the later set from the earlier one.

use std::collections::BTreeMap;

use bolero::{TypeGenerator, check};
use stormlight_shared::statuses::{UnitStatuses, edges};

/// One buff instance the server holds: which buff, and its stacks.
#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Instance {
    #[generator(0u8..=6)]
    buff: u8,
    stacks: u16,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Instance>>().with().len(0usize..=10))]
    before: Vec<Instance>,
    #[generator(bolero::produce::<Vec<Instance>>().with().len(0usize..=10))]
    after: Vec<Instance>,
    /// A rotation of the walk order, so the same instances arrive differently.
    turn: u8,
}

fn published(instances: &[Instance]) -> UnitStatuses {
    UnitStatuses::from_instances(instances.iter().map(|i| (u32::from(i.buff), i.stacks)))
}

/// The model: a map from buff to its saturating stack sum.
fn model(instances: &[Instance]) -> BTreeMap<u32, u16> {
    let mut map = BTreeMap::new();
    for i in instances {
        let entry = map.entry(u32::from(i.buff)).or_insert(0u16);
        *entry = entry.saturating_add(i.stacks);
    }
    map
}

#[test]
fn a_units_statuses_are_one_fact_per_buff_whatever_the_walk() {
    check!().with_type::<Scenario>().for_each(|s| {
        let set = published(&s.before);
        let facts: Vec<(u32, u16)> = set.facts().iter().map(|f| (f.buff, f.stacks)).collect();
        let expected: Vec<(u32, u16)> = model(&s.before).into_iter().collect();
        assert_eq!(facts, expected, "{s:?}");

        let mut walked = s.before.clone();
        if !walked.is_empty() {
            let len = walked.len();
            walked.rotate_left(usize::from(s.turn) % len);
        }
        assert_eq!(published(&walked), set, "the walk order leaked onto the wire");
        // A refresh re-lists exactly what is there.
        assert_eq!(published(&s.before), set, "a refresh changed the published set");
        assert_eq!(set.is_empty(), s.before.is_empty());
    });
}

#[test]
fn the_edges_are_the_whole_difference_between_two_sets() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (before, after) = (published(&s.before), published(&s.after));
        let e = edges(&before, &after);
        let (b, a) = (model(&s.before), model(&s.after));

        for buff in &e.appeared {
            assert!(!b.contains_key(buff) && a.contains_key(buff), "{buff} did not appear");
        }
        for buff in &e.gone {
            assert!(b.contains_key(buff) && !a.contains_key(buff), "{buff} did not go");
        }
        for buff in &e.restacked {
            assert!(b.get(buff).is_some_and(|n| a.get(buff).is_some_and(|m| m != n)), "{buff}");
        }
        // Rebuild the later key set from the earlier one and the edges.
        let mut keys: Vec<u32> = b.keys().copied().filter(|k| !e.gone.contains(k)).collect();
        keys.extend(&e.appeared);
        keys.sort_unstable();
        assert_eq!(keys, a.keys().copied().collect::<Vec<_>>(), "{s:?}");
        // Every restack is caught.
        let changed = b.iter().filter(|(k, n)| a.get(k).is_some_and(|m| m != *n)).count();
        assert_eq!(e.restacked.len(), changed, "{s:?}");
        assert!(edges(&after, &after).is_empty(), "a set differs from itself");
    });
}
