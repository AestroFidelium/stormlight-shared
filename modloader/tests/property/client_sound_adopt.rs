//! A declared sound through adoption (stormlight/server#176).
//!
//! Invariants:
//!   - **a playable sound is adopted as declared** on an ability's impact, alone
//!     or layered beside the impact's drawing;
//!   - **one the client cannot play is refused**, naming the effect — no file, a
//!     volume below zero or not finite, or a falloff no mixer can apply — and so is
//!     a layering that hides one among playable layers.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::lifetime::EffectLifetime;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::sound::{SoundFalloff, SoundPlayback};
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, PrimitiveShape, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Fault {
    None,
    NoFile,
    NegativeVolume,
    InfiniteVolume,
    InvertedFalloff,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// Hundredths of the recorded volume.
    volume: u8,
    looping: bool,
    layered: bool,
    fault: Fault,
}

fn sound(s: &Scenario) -> VisualModel {
    let mut volume = f32::from(s.volume) / 100.0;
    let mut falloff = SoundFalloff { near: 3.0, far: 30.0 };
    let mut asset = String::from("mod://pack/hit.ogg");
    match s.fault {
        Fault::None => {}
        Fault::NoFile => asset.clear(),
        Fault::NegativeVolume => volume = -volume - 0.1,
        Fault::InfiniteVolume => volume = f32::INFINITY,
        Fault::InvertedFalloff => falloff = SoundFalloff { near: 30.0, far: 3.0 },
    }
    let playback = if s.looping { SoundPlayback::Loop } else { SoundPlayback::Once };
    VisualModel::Sound { asset, volume, playback, falloff }
}

#[test]
fn a_playable_sound_is_adopted_and_any_other_refused() {
    check!().with_type::<Scenario>().for_each(|s| {
        let burst = VisualModel::Primitive { shape: PrimitiveShape::Sphere, color: [1.0; 4] };
        let model = if s.layered { VisualModel::Layered(vec![burst, sound(s)]) } else { sound(s) };
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
                assert!(matches!(s.fault, Fault::None), "{s:?}: an unplayable sound was adopted");
                assert_eq!(adopted.effect("frost", EffectRole::Impact), Some(&model), "{s:?}");
            }
            Err(error) => {
                assert!(
                    !matches!(s.fault, Fault::None),
                    "{s:?}: a playable sound was refused: {error}"
                );
                assert!(
                    error.to_string().contains("frost"),
                    "the refusal names no effect: {error}"
                );
            }
        }
    });
}
