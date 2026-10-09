//! A task's payout as an occurrence on the wire (stormlight/server#184).
//!
//! The paid record answers "where am I"; this answers "what just happened" — one
//! message per task per tick, carrying the whole range of rungs that tick paid.
//! Invariants:
//!   - **It says exactly the rungs it paid**: the range from the first unpaid rung
//!     to one past the last paid, never empty unless the shortcut alone fired;
//!   - **It survives the trip** byte for byte, so the owner is told what the
//!     server decided rather than something near it.

use bevy::prelude::Entity;
use bolero::{TypeGenerator, check};
use stormlight_shared::task_payout::TaskPayout;
use stormlight_shared::tasks::TaskRef;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    talent: u32,
    #[generator(0u32..=6)]
    from: u32,
    #[generator(0u32..=6)]
    gained: u32,
    shortcut: bool,
}

#[test]
fn a_payout_says_exactly_the_rungs_it_paid() {
    check!().with_type::<Scenario>().for_each(|s| {
        let payout = TaskPayout {
            unit: Entity::from_bits((1 << 32) | 7),
            task: TaskRef::Talent(s.talent),
            from: s.from,
            to: s.from + s.gained,
            shortcut: s.shortcut,
        };
        assert_eq!(payout.rungs(), s.gained, "{s:?}");
        assert_eq!(
            payout.paid().collect::<Vec<_>>(),
            (s.from..s.from + s.gained).collect::<Vec<_>>()
        );
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(payout, cfg).expect("a payout encodes");
        let (back, _): (TaskPayout, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("and decodes");
        assert_eq!(back, payout, "{s:?}");
    });
}
