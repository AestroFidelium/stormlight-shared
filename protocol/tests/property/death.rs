//! The replicated alive/dead flag (stormlight/server#61) — the wire contract for
//! "this unit is down".
//!
//! [`LifeState`] is the client's whole answer to "should I draw a corpse", so its
//! invariants are small but load-bearing:
//!
//! - **It stays a flat two-state fact.** The encoded form carries the state and
//!   nothing else, so it costs a unit almost nothing and can never disagree with
//!   itself. If a field ever grows here it must be a deliberate protocol decision
//!   rather than something that drifted in — this test is the tripwire.
//! - **The round trip preserves the fact**, in both directions: a downed unit
//!   decodes downed and a live one decodes live. Neither may turn into the other,
//!   and the default — what an absent component means — is alive.
//! - **It is registered on the protocol**, on both mirrors a client draws a unit
//!   on. A state the server sets and no client is told about is worse than none:
//!   the corpse keeps walking.

use bevy::prelude::*;
use bolero::{TypeGenerator, check};
use lightyear::prelude::*;
use stormlight_shared::death::LifeState;
use stormlight_shared::protocol::ProtocolPlugin;

/// The scenario is the alive/dead fact itself — there is nothing else in this
/// component to generate, which is exactly the property being pinned.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    down: bool,
    /// Unrelated bytes sharing the frame, so the flag's encoding is exercised
    /// where it really lives: in the middle of someone else's buffer.
    padding: [u8; 3],
}

#[test]
fn the_state_costs_a_unit_next_to_nothing_to_carry() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let state = if s.down { LifeState::Downed } else { LifeState::Alive };

        // One discriminant and no payload: the frame grows by a single byte over
        // the data it rides with, whichever state it is in.
        let bare = bincode::serde::encode_to_vec(s.padding, cfg).expect("padding should encode");
        let with_state = bincode::serde::encode_to_vec((s.padding, state), cfg)
            .expect("the pair should encode");
        assert_eq!(
            with_state.len(),
            bare.len() + 1,
            "the death state is carrying more than the one fact it declares",
        );
    });
}

#[test]
fn a_downed_unit_decodes_downed_and_a_live_one_decodes_live() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = if s.down { LifeState::Downed } else { LifeState::Alive };
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("the state should encode");
        let (back, _): (LifeState, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("the state should decode");

        assert_eq!(back, sent, "a unit changed between alive and dead crossing the wire");
        assert_eq!(back.is_downed(), s.down, "the decoded state answers the wrong question");
        // What a unit that has never died presents: no component at all, read as
        // alive. A default that read as dead would kill every unit on the field.
        assert!(!LifeState::default().is_downed(), "the absence of a death reads as a death");
    });
}

#[test]
fn the_protocol_registers_the_death_state_for_replication() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(ProtocolPlugin);
    app.finish();

    let component = app
        .world()
        .component_id::<LifeState>()
        .expect("registering the state introduces it to the world");
    let registry = app.world().resource::<ComponentRegistry>();
    assert!(
        registry.component_id_to_kind.contains_key(&component),
        "the death state is not on the protocol, so no client is ever told a unit died",
    );
}
