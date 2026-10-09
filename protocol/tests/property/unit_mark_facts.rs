//! What a client marks units by (stormlight/server#181): a unit's side, its body
//! radius, and the unit the owner's own unit is attacking.
//!
//! Structural only:
//!   - **Round-trip**: each fact survives a serde encode/decode unchanged and
//!     re-encodes to identical bytes;
//!   - **The target is mapped**: `map_entities` rewrites an engaged target into
//!     the receiver's world, and **nobody engaged stays nobody** — a mapping must
//!     never invent a unit to mark.

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bolero::{TypeGenerator, check};
use serde::Serialize;
use serde::de::DeserializeOwned;
use stormlight_shared::body_radius::BodyRadius;
use stormlight_shared::engaged::EngagedTarget;
use stormlight_shared::team::TeamTag;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    team: u32,
    /// Hundredths of a world unit.
    radius: u16,
    engaged: Option<u32>,
}

/// The generation half must be non-zero for `from_bits` to name a valid entity.
fn entity(bits: u32) -> Entity {
    Entity::from_bits(u64::from(bits) | (1 << 32))
}

fn round_trips<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let cfg = bincode::config::standard();
    let bytes = bincode::serde::encode_to_vec(value, cfg).expect("serialize");
    let (back, _): (T, usize) =
        bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
    assert_eq!(&back, value, "did not round-trip");
    let again = bincode::serde::encode_to_vec(&back, cfg).expect("reserialize");
    assert_eq!(bytes, again, "the encoding is not stable");
}

struct Redirect(Entity);
impl EntityMapper for Redirect {
    fn get_mapped(&mut self, _source: Entity) -> Entity {
        self.0
    }
    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

#[test]
fn every_fact_survives_the_wire() {
    check!().with_type::<Scenario>().for_each(|s| {
        round_trips(&TeamTag(s.team));
        round_trips(&BodyRadius(f32::from(s.radius) / 100.0));
        round_trips(&EngagedTarget(s.engaged.map(entity)));
    });
}

#[test]
fn an_engaged_target_is_mapped_and_nobody_stays_nobody() {
    check!().with_type::<Scenario>().for_each(|s| {
        let elsewhere = entity(777);
        let mut mapped = EngagedTarget(s.engaged.map(entity));
        mapped.map_entities(&mut Redirect(elsewhere));
        assert_eq!(
            mapped.0,
            s.engaged.map(|_| elsewhere),
            "{s:?}: a target must be remapped, and an absent one must stay absent",
        );
    });
}
