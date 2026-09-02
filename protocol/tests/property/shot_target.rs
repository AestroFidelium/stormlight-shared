//! The launch says who it was aimed at (stormlight/server#155).
//!
//! A `ProjectileFired` already names the shooter, so a client can ask that unit's
//! art where its shots leave from. The far end of the same flight needs the same
//! thing: the point a shot is *drawn arriving on* is a socket in the target's
//! skeleton, and the client has no way to find that rig without being told which
//! unit the shot was aimed at.
//!
//! Structural only, and the same shape the shooter's handle is pinned in:
//!   - **Round-trip**: a launch with a target and one without both survive a serde
//!     encode/decode unchanged and re-encode to identical bytes;
//!   - **Mapping is targeted**: `map_entities` rewrites the shooter *and* the
//!     target, each independently, and touches no number;
//!   - **An unaimed shot stays unaimed**: a skillshot carries no target, and no
//!     mapping may invent one — a drawing re-aimed at a unit nobody aimed at is a
//!     lie about what the simulation is doing.

use bevy::ecs::entity::{Entity, EntityMapper, MapEntities};
use bevy::math::Vec3;
use bolero::{TypeGenerator, check};
use stormlight_shared::projectiles::ProjectileFired;

fn frac(seed: u16) -> f32 {
    f32::from(seed) / f32::from(u16::MAX)
}

fn coord(seed: u16) -> f32 {
    (frac(seed) - 0.5) * 200.0
}

/// Raw seeds mapped to a bounded, finite launch. `has_shooter` / `has_target` are
/// independent because every combination is real: a visible shooter firing a
/// skillshot, an unseen shooter's shot aimed at a unit this client *can* see, and
/// both or neither.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    origin: (u16, u16, u16),
    velocity: (u16, u16, u16),
    range_seed: u16,
    vfx: u32,
    shot: u32,
    has_shooter: bool,
    shooter_bits: u32,
    has_target: bool,
    target_bits: u32,
}

/// The generation half must be non-zero for `from_bits` to name a valid entity.
fn entity(bits: u32) -> Entity {
    Entity::from_bits(u64::from(bits) | (1 << 32))
}

fn launch(s: &Scenario) -> ProjectileFired {
    ProjectileFired {
        origin: Vec3::new(coord(s.origin.0), coord(s.origin.1), coord(s.origin.2)),
        velocity: Vec3::new(coord(s.velocity.0), coord(s.velocity.1), coord(s.velocity.2)),
        range: frac(s.range_seed) * 100.0,
        vfx: s.vfx,
        shot: s.shot,
        shooter: s.has_shooter.then(|| entity(s.shooter_bits)),
        target: s.has_target.then(|| entity(s.target_bits)),
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

struct Identity;
impl EntityMapper for Identity {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        source
    }
    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

#[test]
fn any_launch_survives_a_serde_round_trip_with_or_without_a_target() {
    check!().with_type::<Scenario>().for_each(|s| {
        let cfg = bincode::config::standard();
        let fired = launch(s);
        let bytes = bincode::serde::encode_to_vec(fired, cfg).expect("serialize");
        let (back, _): (ProjectileFired, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("deserialize");
        assert_eq!(fired, back, "the launch did not round-trip");
        let again = bincode::serde::encode_to_vec(back, cfg).expect("reserialize");
        assert_eq!(bytes, again, "the launch encoding is not stable");
    });
}

#[test]
fn map_entities_rewrites_both_handles_and_no_number() {
    check!().with_type::<Scenario>().for_each(|s| {
        let original = launch(s);

        let mut id = original;
        id.map_entities(&mut Identity);
        assert_eq!(id, original, "identity mapping must be a no-op");

        let elsewhere = entity(777);
        let mut mapped = original;
        mapped.map_entities(&mut Redirect(elsewhere));
        assert_eq!(mapped.origin, original.origin, "the launch point must never be remapped");
        assert_eq!(mapped.velocity, original.velocity, "nor the flight");
        assert_eq!(mapped.range, original.range, "nor how far it may travel");
        assert_eq!(mapped.vfx, original.vfx, "nor the cosmetic key");
        assert_eq!(mapped.shot, original.shot, "nor which shot it is");

        match original.shooter {
            Some(_) => assert_eq!(mapped.shooter, Some(elsewhere), "the shooter is remapped"),
            None => assert_eq!(mapped.shooter, None, "a shot from nobody stays from nobody"),
        }
        match original.target {
            Some(_) => assert_eq!(
                mapped.target,
                Some(elsewhere),
                "the aimed-at unit is not remapped, so the drawing would be turned onto \
                 whatever entity happens to hold that index in this client's world",
            ),
            None => assert_eq!(
                mapped.target, None,
                "a shot aimed at nobody arrived aimed at something — a skillshot would be \
                 drawn curving into a unit the server never pointed it at",
            ),
        }
    });
}
