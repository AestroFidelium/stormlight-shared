//! Invariants of the **order** wire contract ([`stormlight_shared::orders`]) —
//! the one client→server command that says what a unit should do next
//! (stormlight/server#90).
//!
//! Every intent a player can express is one entry of one kind, and every entry
//! carries the same two things: what to do, and whether it *replaces* the plan or
//! is appended to it. Two structural properties matter on the wire:
//!
//!   - **Round-trip**: any order survives an encode/decode unchanged and
//!     re-encodes to identical bytes. A plan is a sequence, so a payload that
//!     shifted in transit does not misfire one action — it misfires the rest of
//!     the sequence too.
//!   - **Entity mapping is targeted**: exactly the two order kinds that name a
//!     unit (an attack order, and a queued cast aimed at a unit) are rewritten
//!     from the client's replicated id into the server's. A kind that carries a
//!     ground point must come through untouched, or a shift-queued walk would
//!     land wherever an unrelated entity happens to live.
//!
//! The *mode* is deliberately part of the payload and never a key: the modifier a
//! player holds down is pure client convention, exactly like a keybind→slot
//! mapping. The wire carries "append" or "replace"; it never carries "shift".

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::cast::Aim;
use stormlight_shared::orders::{OrderKind, OrderMode, OrderRequest};

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

fn entity(bits: u32) -> Entity {
    Entity::from_bits(u64::from(bits) | (1 << 32))
}

/// Raw seeds mapped onto a bounded, finite order, so value-equality after a
/// round-trip is meaningful (no NaN) and shrinking lands on the variant tags.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    kind: u8,
    aim: u8,
    slot: u8,
    append: bool,
    entity_bits: u32,
    v: (u16, u16, u16),
}

impl Scenario {
    fn point(&self) -> Vec3 {
        Vec3::new(coord(self.v.0), coord(self.v.1), coord(self.v.2))
    }

    fn aim(&self) -> Aim {
        match self.aim % 4 {
            0 => Aim::None,
            1 => Aim::Unit(entity(self.entity_bits)),
            2 => Aim::Point(self.point()),
            _ => Aim::Vector(self.point()),
        }
    }

    fn order(&self) -> OrderRequest {
        let kind = match self.kind % 6 {
            0 => OrderKind::Move { target: self.point() },
            1 => OrderKind::Attack { target: entity(self.entity_bits) },
            2 => OrderKind::AttackMove { target: self.point() },
            3 => OrderKind::Cast { slot: self.slot, aim: self.aim() },
            4 => OrderKind::Stop,
            _ => OrderKind::Hold,
        };
        let mode = if self.append { OrderMode::Append } else { OrderMode::Replace };
        OrderRequest { kind, mode }
    }
}

/// A mapper that redirects every entity to one fixed target — enough to observe
/// exactly which fields `map_entities` touches.
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
fn any_order_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let order = s.order();
        let bytes = bincode::serde::encode_to_vec(order, cfg).expect("serialize");
        let (back, _): (OrderRequest, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(order, back, "an order did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "the order encoding is not stable");
    });
}

#[test]
fn the_mode_survives_independently_of_the_kind() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let order = s.order();
        let bytes = bincode::serde::encode_to_vec(order, cfg).expect("serialize");
        let (back, _): (OrderRequest, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(
            back.mode, order.mode,
            "whether an order appends to the plan or replaces it changed in transit",
        );
    });
}

#[test]
fn map_entities_rewrites_exactly_the_kinds_that_name_a_unit() {
    check!().with_type::<Scenario>().for_each(|s| {
        let original = s.order();

        // Identity mapping never changes anything, for any kind.
        let mut id = original;
        id.map_entities(&mut Identity);
        assert_eq!(id, original, "identity mapping must be a no-op");

        let target = entity(777);
        let mut mapped = original;
        mapped.map_entities(&mut Redirect(target));
        assert_eq!(mapped.mode, original.mode, "the mode must never be remapped");

        match &original.kind {
            OrderKind::Attack { .. } => assert_eq!(
                mapped.kind,
                OrderKind::Attack { target },
                "an attack order's target must be remapped into the server's id space",
            ),
            OrderKind::Cast { slot, aim: Aim::Unit(_) } => assert_eq!(
                mapped.kind,
                OrderKind::Cast { slot: *slot, aim: Aim::Unit(target) },
                "a queued cast aimed at a unit must be remapped",
            ),
            other => assert_eq!(
                &mapped.kind, other,
                "an order that names no unit must cross the wire untouched",
            ),
        }
    });
}
