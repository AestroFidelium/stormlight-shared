//! Invariants of the vital-change wire contract ([`stormlight_shared::vital_feed`])
//! — what the server tells a client about a hit that landed, so a mod's HUD can
//! print the number (stormlight/server#93). Structural only:
//!   - **Round-trip**: any `VitalChange` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes.
//!   - **Entity mapping is targeted**: `map_entities` rewrites the unit it happened
//!     to and touches neither the amounts nor the cause.
//!   - **The kind's wire tag is its declaration index.** The protocol is
//!     unversioned, so a further kind may only ever be appended.
//!   - **Coalescing is addition, and it is order-blind.** The server merges a
//!     window's worth of occurrences into one message; whatever order they arrive
//!     in, the totals are the same and the latest is the last one folded — which is
//!     the whole reason a beam reads as one climbing number rather than a column of
//!     ones.

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bolero::{TypeGenerator, check};
use stormlight_shared::vital_feed::{VitalChange, VitalChangeKind};

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

/// A bounded, finite magnitude, so value-equality after a round-trip is meaningful
/// (no NaN).
fn magnitude(seed: u16) -> f32 {
    frac(seed) * 5000.0
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    amount: u16,
    absorbed: u16,
    hits: u8,
    healed: bool,
    cause: u32,
    entity_bits: u32,
}

fn change(s: &Scenario) -> VitalChange {
    VitalChange {
        // The generation half must be non-zero for `from_bits` to be a valid entity.
        unit: Entity::from_bits(u64::from(s.entity_bits) | (1 << 32)),
        amount: magnitude(s.amount),
        // Never more than the whole blow: a shield cannot absorb what did not land.
        absorbed: magnitude(s.absorbed).min(magnitude(s.amount)),
        hits: s.hits.max(1),
        kind: if s.healed { VitalChangeKind::Heal } else { VitalChangeKind::Damage },
        cause: s.cause,
    }
}

/// A mapper that redirects every entity to one fixed target.
struct Redirect(Entity);
impl EntityMapper for Redirect {
    fn get_mapped(&mut self, _source: Entity) -> Entity {
        self.0
    }
    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

/// A mapper that returns every entity unchanged.
struct Identity;
impl EntityMapper for Identity {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        source
    }
    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

#[test]
fn any_vital_change_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let ev = change(s);
        let bytes = bincode::serde::encode_to_vec(ev, cfg).expect("serialize");
        let (back, _): (VitalChange, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(ev, back, "vital change did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "vital change encoding is not stable");
    });
}

#[test]
fn map_entities_rewrites_the_unit_and_nothing_else() {
    check!().with_type::<Scenario>().for_each(|s| {
        let original = change(s);

        let mut id = original;
        id.map_entities(&mut Identity);
        assert_eq!(id, original, "identity mapping must be a no-op");

        let target = Entity::from_bits((1 << 32) | 777);
        let mut mapped = original;
        mapped.map_entities(&mut Redirect(target));
        assert_eq!(mapped.unit, target, "the unit it happened to is remapped");
        assert_eq!(mapped.amount, original.amount, "the amount must never be remapped");
        assert_eq!(mapped.absorbed, original.absorbed, "nor the absorbed part");
        assert_eq!(mapped.hits, original.hits, "nor the count");
        assert_eq!(mapped.cause, original.cause, "nor the opaque cause");
        assert_eq!(mapped.kind, original.kind, "nor the kind");
    });
}

#[test]
fn a_kinds_wire_tag_is_its_declaration_index() {
    check!().with_type::<Scenario>().for_each(|_| {
        let cfg = bincode::config::standard();
        for (tag, kind) in [VitalChangeKind::Damage, VitalChangeKind::Heal].iter().enumerate() {
            let bytes = bincode::serde::encode_to_vec(kind, cfg).expect("serialize");
            assert_eq!(bytes, vec![tag as u8], "{kind:?} moved off wire tag {tag}");
        }
    });
}

#[test]
fn folding_a_window_adds_up_whatever_order_it_arrives_in() {
    check!().with_type::<(Scenario, Scenario)>().for_each(|(a, b)| {
        let (first, second) = (change(a), change(b));

        let mut forward = first;
        forward.fold(&second);
        let mut backward = second;
        backward.fold(&first);

        // The sums do not care which blow landed first…
        assert_eq!(forward.amount, backward.amount, "coalescing is not commutative in the total");
        assert_eq!(forward.absorbed, backward.absorbed, "nor in the absorbed part");
        assert_eq!(forward.hits, backward.hits, "nor in the count");
        // …and folding never loses one.
        assert!(
            forward.amount >= first.amount && forward.amount >= second.amount,
            "a merged window reports less than one of the blows in it",
        );
        assert_eq!(
            forward.hits,
            first.hits.saturating_add(second.hits),
            "a merged window lost a hit",
        );
        // What a fold must never do is move the blow somewhere else, or turn a
        // damage report into a healing one: an accumulator keeps the identity it
        // was opened with, and it is that identity the server keys the window on.
        assert_eq!(forward.unit, first.unit, "folding moved the report to another unit");
        assert_eq!(forward.kind, first.kind, "folding changed what kind of report it is");
        assert_eq!(forward.cause, first.cause, "folding changed what caused it");
    });
}
