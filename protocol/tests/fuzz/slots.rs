//! Fuzz the decode side of the ability-slot wire format
//! (`stormlight/server#58`). This frame is owner-scoped privileged state, and it
//! feeds the one piece of UI a player watches every second of a match: a bar
//! whose fills come straight out of these numbers.
//!
//! Its length is attacker-controlled (a count byte followed by that many
//! entries), so a frame claiming more slots than it carries must yield only the
//! entries actually present. And because the cooldown is a *deadline* swept
//! locally, a decoded slot is read at arbitrarily many later ticks — every one of
//! them has to produce a finite fraction inside `[0, 1]`, whatever the frame said.
//!
//!   cargo bolero test slots::decoding_arbitrary_slot_frames_stays_sane -T 600sec

use bolero::{TypeGenerator, check};
use lightyear::prelude::Tick;
use stormlight_shared::slots::{MAX_COOLDOWN_TICKS, SLOT_ENTRY_LEN, decode};

/// A hostile frame plus a handful of ticks it gets swept at.
#[derive(Debug, TypeGenerator)]
struct Frame {
    bytes: Vec<u8>,
    now: u16,
    advance: u16,
}

#[test]
fn decoding_arbitrary_slot_frames_stays_sane() {
    check!().with_type::<Frame>().for_each(|f| {
        let slots = decode(&f.bytes);
        assert!(
            slots.0.len() <= f.bytes.len().saturating_sub(1) / SLOT_ENTRY_LEN,
            "the count byte must never be trusted past the bytes that follow it"
        );

        let now = Tick(f.now);
        let later = Tick(now.0.wrapping_add(f.advance % (MAX_COOLDOWN_TICKS / 2)));
        for slot in &slots.0 {
            for at in [now, later] {
                let fill = slot.fraction(at);
                assert!(fill.is_finite(), "a decoded slot produced a non-finite fill fraction");
                assert!((0.0..=1.0).contains(&fill), "fill fraction escaped [0, 1]");
                assert!(
                    slot.remaining_ticks(at) <= MAX_COOLDOWN_TICKS,
                    "a decoded deadline must stay inside the representable window"
                );
                // The verdict is a conjunction: a slot still running its cooldown
                // may never read castable, however the flag byte was crafted.
                assert!(slot.ready(at) || !slot.castable(at));
            }
            // Sweeping forward may only ever consume time — for a deadline that
            // is actually pending. A hostile frame may name one in the *past*
            // half of the wrap: that reads as ready (which is the safe answer),
            // and a clock advancing far enough eventually carries it back into
            // the future half. No cooldown the server publishes can do that —
            // it clamps to `MAX_COOLDOWN_TICKS` — so the claim worth holding is
            // that a pending deadline only ever runs down.
            if slot.remaining_ticks(now) > 0 {
                assert!(slot.remaining_ticks(later) <= slot.remaining_ticks(now));
            }
        }
    });
}
