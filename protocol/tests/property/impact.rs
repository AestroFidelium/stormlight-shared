//! Invariants of the impact wire contract ([`stormlight_shared::impact`]) — the
//! one-shot "a shot landed here" event, server→client. Structural only:
//!   - **Round-trip**: any `ImpactEvent` survives a serde encode/decode unchanged
//!     and re-encodes to identical bytes (a stable wire payload).
//!   - **Entity mapping is targeted**: `map_entities` rewrites the struck unit and
//!     nothing else, and leaves a victimless impact (a shot that hit the ground)
//!     victimless — a hit reaction must never be manufactured out of a miss.

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::impact::ImpactEvent;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

/// Raw seeds mapped to a bounded, finite `ImpactEvent` so value-equality after a
/// round-trip is meaningful (no NaN). `hit` decides whether the shot struck a unit
/// or landed on nothing, so both shapes are generated.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    at: (u16, u16, u16),
    vfx: u32,
    hit: bool,
    entity_bits: u32,
    /// Which shot the impact ends — `0` for the common case where nothing
    /// travelled (a melee blow, a zone tick, a notify burst).
    shot: u32,
}

fn event(s: &Scenario) -> ImpactEvent {
    ImpactEvent {
        at: Vec3::new(coord(s.at.0), coord(s.at.1), coord(s.at.2)),
        vfx: s.vfx,
        // The generation half must be non-zero for `from_bits` to be a valid entity.
        victim: s.hit.then(|| Entity::from_bits(u64::from(s.entity_bits) | (1 << 32))),
        shot: s.shot,
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
fn any_impact_event_survives_a_serde_round_trip() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let ev = event(s);
        let bytes = bincode::serde::encode_to_vec(ev, cfg).expect("serialize");
        let (back, _): (ImpactEvent, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(ev, back, "impact event did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "impact event encoding is not stable");
    });
}

#[test]
fn map_entities_rewrites_the_victim_and_nothing_else() {
    check!().with_type::<Scenario>().for_each(|s| {
        let original = event(s);

        // Identity mapping never changes anything, struck or not.
        let mut id = original;
        id.map_entities(&mut Identity);
        assert_eq!(id, original, "identity mapping must be a no-op");

        let target = Entity::from_bits((1 << 32) | 777);
        let mut mapped = original;
        mapped.map_entities(&mut Redirect(target));
        assert_eq!(mapped.at, original.at, "the impact point must never be remapped");
        assert_eq!(mapped.vfx, original.vfx, "nor the cosmetic key");
        match original.victim {
            Some(_) => assert_eq!(mapped.victim, Some(target), "the struck unit is remapped"),
            None => assert_eq!(
                mapped.victim, None,
                "a shot that struck nothing stays victimless — no reaction out of a miss",
            ),
        }
    });
}
