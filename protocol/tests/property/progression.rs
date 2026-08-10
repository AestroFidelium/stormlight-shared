//! The replicated progression state (stormlight/server#62) — what a client is
//! told about levels, and who is told it.
//!
//! Two components, on purpose, because they answer two different questions with
//! two different audiences:
//!
//! - [`UnitLevel`] is **public**. Every player may read how far along an opponent
//!   is; it is a fact about a unit on the field, like its health bar, and it rides
//!   the unit entity so the same area-of-interest culling decides who sees it.
//! - [`XpProgress`] is **the owner's alone**. How close *you* are to your next
//!   level is privileged: an opponent reading it knows exactly when to contest.
//!   It rides the owner-scoped view entity, so scoping stays a property of which
//!   entity carries the state rather than a second visibility rule.
//!
//! The invariants worth pinning here:
//!
//! - the round trip preserves both, exactly — a level that drifted crossing the
//!   wire would gate the wrong talent tier, and a progress bar that drifted would
//!   lie about the one number a player is watching;
//! - [`XpProgress::fraction`] is **total** and lands in `[0, 1]` for every input,
//!   including the ones no honest server sends. It is read straight into a bar
//!   width, and a NaN or a negative there is a broken HUD rather than a caught
//!   error;
//! - both are registered on the protocol. State the server maintains and no
//!   client is told about is a level-up nobody can see.

use bevy::prelude::*;
use bolero::{TypeGenerator, check};
use lightyear::prelude::*;
use stormlight_shared::progression::{UnitLevel, XpProgress};
use stormlight_shared::protocol::ProtocolPlugin;

/// How a generated number is chosen: ordinary progress plus the shapes a bar must
/// survive being handed.
#[derive(Debug, TypeGenerator)]
enum Amount {
    Declared(u16),
    Zero,
    Negative(u16),
    Infinite,
    NotANumber,
}

impl Amount {
    fn value(&self) -> f32 {
        match self {
            Amount::Declared(v) => f32::from(*v),
            Amount::Zero => 0.0,
            Amount::Negative(v) => -f32::from(*v),
            Amount::Infinite => f32::INFINITY,
            Amount::NotANumber => f32::NAN,
        }
    }
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    level: u8,
    current: Amount,
    floor: Amount,
    next: Option<Amount>,
}

fn progress(s: &Scenario) -> XpProgress {
    XpProgress {
        current: s.current.value(),
        floor: s.floor.value(),
        next: s.next.as_ref().map(Amount::value),
    }
}

#[test]
fn a_level_survives_the_wire_round_trip_exactly() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = UnitLevel(s.level);
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("a level should encode");
        let (back, _): (UnitLevel, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("a level should decode");

        assert_eq!(back, sent, "a unit's level changed crossing the wire");
        // What a unit that has never levelled presents: the level everything
        // starts at, never a zeroth level no talent tier is keyed on.
        assert_eq!(UnitLevel::default().0, 1, "the absence of progress reads as no level at all");
    });
}

#[test]
fn a_progress_frame_survives_the_wire_round_trip_exactly() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = progress(s);
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("progress should encode");
        let (back, _): (XpProgress, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("progress should decode");

        // Compared through the accessor, because NaN is not equal to itself: the
        // contract is that the *drawn* bar crosses unchanged.
        assert_eq!(back.fraction(), sent.fraction(), "a progress bar changed crossing the wire");
        assert_eq!(
            back.next.is_none(),
            sent.next.is_none(),
            "a unit at its ceiling gained a next level crossing the wire (or lost one)",
        );
    });
}

#[test]
fn a_drawn_fraction_is_always_a_finite_number_between_zero_and_one() {
    check!().with_type::<Scenario>().for_each(|s| {
        let fraction = progress(s).fraction();
        assert!(fraction.is_finite(), "a progress bar was handed a width no renderer can use");
        assert!((0.0..=1.0).contains(&fraction), "a progress bar ran outside its own track");
    });
}

#[test]
fn a_unit_at_its_ceiling_reads_as_complete() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut capped = progress(s);
        capped.next = None;
        // Nothing left to earn is a full bar, not an empty one: a hero at max
        // level has finished the track rather than fallen off the start of it.
        assert_eq!(capped.fraction(), 1.0, "a maxed unit's progress bar read as empty");
    });
}

#[test]
fn ordinary_progress_reads_as_the_share_of_the_span_already_earned() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (Amount::Declared(floor), Some(Amount::Declared(next))) = (&s.floor, &s.next) else {
            return;
        };
        let (floor, next) = (f32::from(*floor), f32::from(*next));
        if next <= floor {
            return;
        }
        // Halfway between the two thresholds is halfway along the bar — the one
        // reading a HUD actually depends on.
        let midpoint =
            XpProgress { current: floor + (next - floor) / 2.0, floor, next: Some(next) };
        assert!(
            (midpoint.fraction() - 0.5).abs() <= 1e-4,
            "progress halfway to the next level did not draw as a half-full bar",
        );
        // And the ends are the ends, whichever way the span is entered.
        let at_floor = XpProgress { current: floor, floor, next: Some(next) };
        let at_next = XpProgress { current: next, floor, next: Some(next) };
        assert_eq!(at_floor.fraction(), 0.0, "a freshly-levelled unit's bar was not empty");
        assert_eq!(at_next.fraction(), 1.0, "a unit at the next threshold's bar was not full");
    });
}

#[test]
fn the_protocol_registers_both_halves_of_progression() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(ProtocolPlugin);
    app.finish();

    for (component, what) in [
        (app.world().component_id::<UnitLevel>(), "the public level"),
        (app.world().component_id::<XpProgress>(), "the owner's XP progress"),
    ] {
        let component = component.expect("registering a component introduces it to the world");
        let registry = app.world().resource::<ComponentRegistry>();
        assert!(
            registry.component_id_to_kind.contains_key(&component),
            "{what} is not on the protocol, so no client is ever told about it",
        );
    }
}
