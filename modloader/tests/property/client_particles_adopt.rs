//! A declared particle emitter through adoption (stormlight/server#141).
//!
//! Invariants:
//!   - **a playable emitter is adopted as declared** on an ability's impact;
//!   - **a malformed one is refused at load, naming the effect** — a negative
//!     lifetime, a non-finite rate, an empty burst — never clamped into something
//!     the mod did not say.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::lifetime::EffectLifetime;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::particles::{
    EmitterShape, ParticleBlend, ParticleEmission, ParticleEmitter, ParticleSpray,
};
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Fault {
    None,
    NegativeLifetime,
    InfiniteRate,
    EmptyBurst,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// Tenths of a second.
    #[generator(1u8..=40)]
    life: u8,
    burst: bool,
    fault: Fault,
}

fn emitter(s: &Scenario) -> ParticleEmitter {
    let life = f32::from(s.life) / 10.0;
    let mut e = ParticleEmitter {
        shape: EmitterShape::Point,
        emission: if s.burst {
            ParticleEmission::Burst { count: 16, delay: 0.0 }
        } else {
            ParticleEmission::Stream { rate: 30.0 }
        },
        lifetime: [life / 2.0, life],
        speed: [1.0, 2.0],
        spray: ParticleSpray::Radial,
        gravity: -3.0,
        drag: 0.0,
        size: Vec::new(),
        color: Vec::new(),
        texture: None,
        blend: ParticleBlend::Add,
        world_space: true,
        capacity: 64,
    };
    match s.fault {
        Fault::None => {}
        Fault::NegativeLifetime => e.lifetime = [-1.0, life],
        Fault::InfiniteRate => e.emission = ParticleEmission::Stream { rate: f32::INFINITY },
        Fault::EmptyBurst => e.emission = ParticleEmission::Burst { count: 0, delay: 0.0 },
    }
    e
}

#[test]
fn a_playable_emitter_is_adopted_and_a_malformed_one_refused() {
    check!().with_type::<Scenario>().for_each(|s| {
        let model = VisualModel::Particles(emitter(s));
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names { abilities: vec!["frost".into()], ..Names::default() },
            effects: vec![EffectVisualDescriptor {
                ability: AbilityId(0),
                role: EffectRole::Impact,
                model: model.clone(),
                attach: None,
                lifetime: EffectLifetime::Default,
            }],
            ..ClientRegistration::default()
        };
        match AdoptedVisuals::adopt(&reg, "pack") {
            Ok(adopted) => {
                assert!(matches!(s.fault, Fault::None), "{s:?}: a malformed emitter was adopted");
                assert_eq!(adopted.effect("frost", EffectRole::Impact), Some(&model), "{s:?}");
            }
            Err(error) => {
                assert!(
                    !matches!(s.fault, Fault::None),
                    "{s:?}: a playable emitter was refused: {error}"
                );
                assert!(
                    error.to_string().contains("frost"),
                    "the refusal names no effect: {error}"
                );
            }
        }
    });
}
