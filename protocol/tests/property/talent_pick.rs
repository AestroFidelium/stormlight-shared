//! Invariants of the talent wire contract ([`stormlight_shared::talents`],
//! stormlight/server#63) — the player's second command, and the owner-scoped view
//! it reads its own tree state from.
//!
//! Structural only; what the server does with a pick is the server's own test.
//!
//!   - **Round-trip**: a `TalentPick` and a `ReplicatedTalents` view survive a
//!     serde encode/decode unchanged and re-encode to identical bytes. A tier index
//!     that shifted crossing the wire would apply a choice to the wrong row of the
//!     tree;
//!   - **A pick carries no entity**, so there is nothing in it for the wire's entity
//!     mapping to get wrong: the server resolves the caster from the sender, the way
//!     it does for a move order;
//!   - **"Pending" is derived**, never sent: a tier is awaiting a choice exactly
//!     when it is unlocked and holds none. Two flags that could disagree would be
//!     two flags a HUD had to reconcile.

use bolero::{TypeGenerator, check};
use stormlight_shared::talents::{ReplicatedTalents, TalentPick, TierView};

/// One generated tier row, unconstrained: locked rows carrying a choice and
/// unlocked rows carrying none are both generated on purpose.
#[derive(Debug, TypeGenerator)]
struct Row {
    tier: u8,
    unlocked: bool,
    chosen: Option<u32>,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    rows: Vec<Row>,
    pick_tier: u8,
    pick_talent: u32,
}

fn view(s: &Scenario) -> ReplicatedTalents {
    ReplicatedTalents(
        s.rows
            .iter()
            .map(|r| TierView { tier: r.tier, unlocked: r.unlocked, chosen: r.chosen })
            .collect(),
    )
}

fn pick(s: &Scenario) -> TalentPick {
    TalentPick { tier: s.pick_tier, talent: s.pick_talent }
}

#[test]
fn any_talent_pick_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let sent = pick(s);
        let bytes = bincode::serde::encode_to_vec(sent, cfg).expect("serialize");
        let (back, _): (TalentPick, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");

        assert_eq!(back, sent, "a talent pick did not round-trip");
        assert_eq!(back.tier, sent.tier, "a pick's tier index moved crossing the wire");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "talent pick encoding is not stable");
    });
}

#[test]
fn any_talent_view_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let published = view(s);
        let bytes = bincode::serde::encode_to_vec(&published, cfg).expect("serialize");
        let (back, _): (ReplicatedTalents, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");

        assert_eq!(back, published, "a talent view did not round-trip");
        let again = bincode::serde::encode_to_vec(&back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "talent view encoding is not stable");
    });
}

#[test]
fn a_tier_is_pending_exactly_when_it_is_unlocked_and_unchosen() {
    check!().with_type::<Scenario>().for_each(|s| {
        let view = view(s);

        for row in &view.0 {
            assert_eq!(
                row.pending(),
                row.unlocked && row.chosen.is_none(),
                "a tier's pending state disagreed with the two facts it is derived from",
            );
            // A locked tier is never waiting on the player: there is nothing to
            // choose there yet, so a HUD must not light it up.
            if !row.unlocked {
                assert!(!row.pending(), "a locked tier asked the player for a choice");
            }
        }

        // The whole-view accessors are the same predicate, per row.
        let pending: Vec<u8> = view.pending().collect();
        for row in &view.0 {
            assert_eq!(
                pending.contains(&row.tier),
                view.0.iter().any(|r| r.tier == row.tier && r.pending()),
                "the pending list disagreed with the rows it is built from",
            );
        }
        assert_eq!(
            view.chosen().count(),
            view.0.iter().filter(|r| r.chosen.is_some()).count(),
            "the chosen list dropped or invented a choice",
        );
    });
}

#[test]
fn a_view_answers_for_a_tier_it_does_not_carry() {
    check!().with_type::<Scenario>().for_each(|s| {
        let view = view(s);
        let asked = s.pick_tier;

        match view.tier(asked) {
            Some(row) => assert_eq!(row.tier, asked, "the view returned some other tier's row"),
            // Total: a client may ask about any tier index, including one this
            // unit's tree never had.
            None => assert!(
                !view.0.iter().any(|r| r.tier == asked),
                "the view claimed not to carry a tier that is right there",
            ),
        }
    });
}
