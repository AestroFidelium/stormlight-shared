//! Invariants of the cast-intent wire contract ([`stormlight_shared::cast`]) —
//! the one player command client→server. Structural only:
//!   - **Round-trip**: any `CastIntent` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes (a stable wire payload).
//!   - **Entity mapping is targeted**: `map_entities` rewrites the target of a
//!     `Unit` aim and nothing else; an identity mapping is a no-op for every aim.

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::cast::{Aim, CastIntent};

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

/// Raw seeds mapped to a bounded, finite `CastIntent` so value-equality after a
/// round-trip is meaningful (no NaN) and shrinking lands on the variant tags.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    slot: u8,
    kind: u8,
    entity_bits: u32,
    v: (u16, u16, u16),
}

fn intent(s: &Scenario) -> CastIntent {
    let aim = match s.kind % 4 {
        0 => Aim::None,
        1 => Aim::Unit(Entity::from_bits(u64::from(s.entity_bits) | (1 << 32))),
        2 => Aim::Point(Vec3::new(coord(s.v.0), coord(s.v.1), coord(s.v.2))),
        _ => Aim::Vector(Vec3::new(coord(s.v.0), coord(s.v.1), coord(s.v.2))),
    };
    CastIntent { slot: s.slot, aim }
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
fn any_cast_intent_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let ci = intent(s);
        let bytes = bincode::serde::encode_to_vec(ci, cfg).expect("serialize");
        let (back, _): (CastIntent, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(ci, back, "cast intent did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "cast intent encoding is not stable");
    });
}

#[test]
fn map_entities_only_rewrites_a_unit_aim() {
    check!().with_type::<Scenario>().for_each(|s| {
        let original = intent(s);

        // Identity mapping never changes anything, for any aim.
        let mut id = original;
        id.map_entities(&mut Identity);
        assert_eq!(id, original, "identity mapping must be a no-op");

        // A redirecting mapping changes exactly the Unit case (to the target),
        // and leaves slot + non-unit aims untouched.
        let target = Entity::from_bits((1 << 32) | 777);
        let mut mapped = original;
        mapped.map_entities(&mut Redirect(target));
        assert_eq!(mapped.slot, original.slot, "slot must never be remapped");
        match original.aim {
            Aim::Unit(_) => {
                assert_eq!(mapped.aim, Aim::Unit(target), "unit target must be remapped")
            }
            other => assert_eq!(mapped.aim, other, "non-unit aim must be untouched"),
        }
    });
}
