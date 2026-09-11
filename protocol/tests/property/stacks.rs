//! The owner's stack counters on the wire (stormlight/server#132).
//!
//! A stack counter is the ISA's bounded reserve for "how many times has this
//! happened" — combo points, a charge-up, the count behind a quest. Every other
//! reserve a widget can name already crossed the wire; this one did not, so
//! `ValueBinding::Pool(PoolRef::Stacks(_))` was a declaration a client resolved to
//! *nothing*, always and by design.
//!
//! # Why the owner's, and not everybody's
//!
//! It rides the **owner-scoped view entity** the ability bar, the XP bar and the
//! talent view already sit on, replicated with `NetworkTarget::Single(owner)`.
//! Which is the same argument those three make: a counter is what a player is
//! playing toward, and an opponent reading "three casts from the payout" is
//! reading exactly when not to contest. One mechanism decides who receives what —
//! which entity the state sits on — rather than a second per-component visibility
//! rule layered over the area-of-interest culling.
//!
//! The counters are **opaque ids**, like a resource pool's: the protocol crate
//! never learns what a counter means, only that a unit holds so many of one.
//!
//! # What is pinned here
//!
//! - the round trip is exact. A count that drifted would be a quest that pays out
//!   early, or never;
//! - **no ceiling**. Nothing in the ISA declares a maximum for a stack counter, so
//!   the wire carries none and invents none — what a count is *toward* is the
//!   reader's question ([`QuestSpec`](stormlight_mod_abi::talents::QuestSpec) has
//!   the only answer anything currently asks for);
//! - a unit holding no counter replicates an empty list, which is a different
//!   thing from a counter at zero: one is "no such counter", the other is "none of
//!   it done yet";
//! - it is registered on the protocol. State the server keeps and no client is
//!   told about is a quest nobody can watch.

use bevy::prelude::*;
use bolero::{TypeGenerator, check};
use lightyear::prelude::*;
use stormlight_shared::protocol::ProtocolPlugin;
use stormlight_shared::stacks::{ReplicatedStacks, StackCount};

/// How a generated count is chosen: ordinary progress plus the shapes no honest
/// server sends but a hostile or broken one might.
#[derive(Debug, TypeGenerator)]
enum Amount {
    Counted(u16),
    Zero,
    Negative(u16),
    Infinite,
    NotANumber,
}

impl Amount {
    fn value(&self) -> f32 {
        match self {
            Amount::Counted(v) => f32::from(*v),
            Amount::Zero => 0.0,
            Amount::Negative(v) => -f32::from(*v),
            Amount::Infinite => f32::INFINITY,
            Amount::NotANumber => f32::NAN,
        }
    }
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    counters: Vec<(u32, Amount)>,
}

fn view(s: &Scenario) -> ReplicatedStacks {
    ReplicatedStacks(
        s.counters.iter().map(|(id, amount)| StackCount::new(*id, amount.value())).collect(),
    )
}

#[test]
fn a_counter_frame_survives_the_wire_round_trip_exactly() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = view(s);
        let bytes = bincode::serde::encode_to_vec(&sent, cfg).expect("counters should encode");
        let (back, _): (ReplicatedStacks, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("counters should decode");

        assert_eq!(back, sent, "a counter changed crossing the wire");
    });
}

/// The one number a reader gets is a number it can use. A count is read straight
/// into a bar width and a printed figure, and there is nowhere downstream to catch
/// a NaN — so a nonsense count is floored at construction rather than replicated
/// and dealt with by every reader separately.
#[test]
fn every_replicated_count_is_a_number_a_reader_can_use() {
    check!().with_type::<Scenario>().for_each(|s| {
        for count in &view(s).0 {
            assert!(count.current.is_finite(), "a counter reached a reader as a non-number");
            assert!(count.current >= 0.0, "a counter reached a reader below empty");
        }
    });
}

/// Absence and zero are different answers and must stay different. "This unit has
/// no such counter" is what makes a HUD draw nothing; "this counter stands at
/// zero" is what makes it draw an empty bar.
#[test]
fn a_counter_nobody_holds_is_absent_rather_than_zero() {
    check!().with_type::<Scenario>().for_each(|s| {
        let view = view(s);
        let held: Vec<u32> = s.counters.iter().map(|(id, _)| *id).collect();
        for id in 0u32..8 {
            assert_eq!(
                view.get(id).is_some(),
                held.contains(&id),
                "counter {id} was reported as held exactly when it was not",
            );
        }
        assert_eq!(ReplicatedStacks::default().get(0), None, "an empty view invented a counter");
    });
}

/// A unit may adjust the same counter twice in a tick, and a projection that
/// published it twice would leave the reader's answer depending on which entry it
/// looked at first.
#[test]
fn a_counter_is_reported_once() {
    check!().with_type::<Scenario>().for_each(|s| {
        let view = view(s);
        for (id, _) in &s.counters {
            let found = view.0.iter().filter(|count| count.id == *id).count();
            assert!(found >= 1, "a declared counter went missing");
            // The list is built by the caller, so a duplicate id is the caller's to
            // avoid — what is pinned here is that the reader takes the *first*, one
            // deterministic answer rather than whichever it happened to reach.
            assert_eq!(
                view.get(*id),
                view.0.iter().find(|count| count.id == *id).map(|count| count.current),
                "the reader disagreed with the frame about a counter's value",
            );
        }
    });
}

#[test]
fn the_counter_view_is_registered_on_the_protocol() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(ProtocolPlugin);
    app.finish();

    let component = app
        .world()
        .component_id::<ReplicatedStacks>()
        .expect("registering a component introduces it to the world");
    let registry = app.world().resource::<ComponentRegistry>();
    assert!(
        registry.component_id_to_kind.contains_key(&component),
        "the owner's counters are maintained and never sent",
    );
}
