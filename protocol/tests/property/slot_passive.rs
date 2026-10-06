//! A passive slot on the wire (stormlight/server#189). Laws:
//!   - the passive bit survives the codec like the other flags;
//!   - a passive slot is never castable, whatever its other flags and cooldown say,
//!     so a bar can never draw a carried trait as a live key;
//!   - for any slot without the bit, the verdict is exactly what it was before.

use bolero::{TypeGenerator, check};
use lightyear::prelude::Tick;
use stormlight_shared::slots::{
    AFFORDABLE, PASSIVE, ReplicatedSlots, SlotState, UNGATED, decode, encode,
};

#[derive(Debug, Clone, Copy, TypeGenerator)]
struct Scenario {
    passive: bool,
    affordable: bool,
    ungated: bool,
    /// Ticks until ready, relative to `now`.
    #[generator(0u16..=600)]
    wait: u16,
    #[generator(0u16..=600)]
    cooldown: u16,
    now: u16,
}

impl Scenario {
    fn state(self) -> SlotState {
        let bit = |on: bool, flag: u8| if on { flag } else { 0 };
        SlotState {
            slot: 0,
            ability: 7,
            ready_at: Tick(self.now) + i16::try_from(self.wait).unwrap_or(i16::MAX),
            cooldown_ticks: self.cooldown,
            charges: 0,
            flags: bit(self.passive, PASSIVE)
                | bit(self.affordable, AFFORDABLE)
                | bit(self.ungated, UNGATED),
        }
    }
}

#[test]
fn a_passive_slot_is_never_castable_and_says_so_across_the_wire() {
    check!().with_type::<Scenario>().for_each(|&s| {
        let state = s.state();
        let back = decode(&encode(&ReplicatedSlots(vec![state])));
        let arrived = back.slot(0).copied().expect("the slot survives the codec");
        assert_eq!(arrived.passive(), s.passive, "the passive bit did not survive");

        let now = Tick(s.now);
        let ordinary = s.wait == 0 && s.affordable && s.ungated;
        assert_eq!(arrived.castable(now), ordinary && !s.passive);
    });
}
