//! A declared ground decal through adoption (stormlight/server#170).
//!
//! Invariants:
//!   - **A drawable decal is adopted as declared**, on an ability's feedback;
//!   - **One no renderer can draw is refused**, naming the effect — a size of zero
//!     or below, a non-finite tint, or a fade no clock can run.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::decal::{DecalBlend, DecalFade};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::lifetime::EffectLifetime;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Fault {
    None,
    FlatSize,
    NotANumberTint,
    BrokenFade,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// Tenths of a world unit.
    #[generator(1u8..=60)]
    width: u8,
    #[generator(1u8..=60)]
    length: u8,
    additive: bool,
    faded: bool,
    fault: Fault,
}

fn decal(s: &Scenario) -> VisualModel {
    let mut size = [f32::from(s.width) / 10.0, f32::from(s.length) / 10.0];
    let mut tint = [1.0, 0.8, 0.6, 1.0];
    let mut fade =
        s.faded.then_some(DecalFade { attack: 0.2, hold: 1.0, decay: 0.5, alpha: [0.0, 1.0, 0.0] });
    match s.fault {
        Fault::None => {}
        Fault::FlatSize => size[1] = 0.0,
        Fault::NotANumberTint => tint[2] = f32::NAN,
        Fault::BrokenFade => {
            fade = Some(DecalFade { attack: -1.0, hold: 0.0, decay: 0.0, alpha: [0.0; 3] })
        }
    }
    VisualModel::Decal {
        asset: "mod://pack/ring.png".into(),
        size,
        tint,
        blend: if s.additive { DecalBlend::Add } else { DecalBlend::Blend },
        fade,
    }
}

#[test]
fn a_drawable_decal_is_adopted_and_any_other_refused() {
    check!().with_type::<Scenario>().for_each(|s| {
        let model = decal(s);
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names { abilities: vec!["frost".into()], ..Names::default() },
            effects: vec![EffectVisualDescriptor {
                ability: AbilityId(0),
                role: EffectRole::Zone,
                model: model.clone(),
                attach: None,
                lifetime: EffectLifetime::Default,
            }],
            ..ClientRegistration::default()
        };
        match AdoptedVisuals::adopt(&reg, "pack") {
            Ok(adopted) => {
                assert!(matches!(s.fault, Fault::None), "{s:?}: an undrawable decal was adopted");
                assert_eq!(adopted.effect("frost", EffectRole::Zone), Some(&model), "{s:?}");
            }
            Err(error) => {
                assert!(
                    !matches!(s.fault, Fault::None),
                    "{s:?}: a drawable decal was refused: {error}"
                );
                assert!(
                    error.to_string().contains("frost"),
                    "the refusal names no effect: {error}"
                );
            }
        }
    });
}
