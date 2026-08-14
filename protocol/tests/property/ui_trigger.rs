//! Invariants of the interface's trigger request ([`stormlight_shared::ui`],
//! stormlight/server#69).
//!
//! Structural only; what the server does with a trigger is the server's own test.
//! The message is one `u32`, so the properties worth pinning are less about
//! encoding than about what the type *cannot* say:
//!
//!   - **Round-trip**: the event id survives a serde encode/decode unchanged and
//!     re-encodes to identical bytes. An id that shifted crossing the wire would
//!     raise a different mod's event — the exact failure the client-side id bridge
//!     exists to prevent, so the wire must not reintroduce it;
//!   - **It carries no entity**, so there is nothing for the wire's entity mapping
//!     to get wrong or for a client to lie about: the server resolves the acting
//!     unit from the sender, the way a move order and a talent pick do;
//!   - **It carries no payload**, which is the sharper constraint. A number
//!     computed on the client and handed to a guest would be unvalidated input from
//!     an untrusted process reaching straight into effect evaluation. Pinned as a
//!     size property rather than a comment, so adding a field is a failing test
//!     rather than a quiet widening of the trust boundary.

use bolero::{TypeGenerator, check};
use stormlight_shared::ui::UiTrigger;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    event: u32,
}

#[test]
fn any_ui_trigger_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = UiTrigger { event: s.event };
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("serialize");
        let (back, _): (UiTrigger, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");

        assert_eq!(back, sent, "a ui trigger did not round-trip");
        assert_eq!(back.event, sent.event, "an event id moved crossing the wire");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "ui trigger encoding is not stable");
    });
}

#[test]
fn a_trigger_is_an_event_id_and_nothing_else() {
    check!().with_type::<Scenario>().for_each(|s| {
        let trigger = UiTrigger { event: s.event };
        // No entity to map, no payload to evaluate — the whole message is the
        // handle. Anything else in here would be state a client asserted about the
        // simulation, and the server would have to disbelieve it anyway.
        assert_eq!(
            size_of_val(&trigger),
            size_of::<u32>(),
            "a ui trigger grew a field: a client must not hand the guest anything \
             but the event it wants raised",
        );
        assert_eq!(trigger, UiTrigger { event: s.event }, "two triggers for one event differ");
    });
}
