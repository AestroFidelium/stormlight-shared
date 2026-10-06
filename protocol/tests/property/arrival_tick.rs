//! The tick a request lands on (stormlight/server#146 follow-up). Laws: it is
//! exactly the prediction lead behind the client's own tick, across the wrap of
//! the tick space, so a key lit when `arrival_tick(now)` reaches a deadline is lit
//! the moment a press of it would be accepted.

use bolero::check;
use lightyear::prelude::Tick;
use stormlight_shared::prediction_lead::{PREDICTION_LEAD_TICKS, arrival_tick};

#[test]
fn a_request_lands_the_lead_behind_the_clients_tick() {
    check!().with_type::<u16>().for_each(|&local| {
        let local = Tick(local);
        let lead = i16::try_from(PREDICTION_LEAD_TICKS).unwrap();
        assert_eq!(local - arrival_tick(local), lead, "not exactly the lead behind");
    });
}
