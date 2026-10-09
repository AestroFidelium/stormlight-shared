//! A unit's declared shadow through adoption (stormlight/server#179).
//!
//! Invariants:
//!   - **What was declared is what is adopted**: fitting, a radius, or none;
//!   - **A radius no renderer can draw is refused**, naming the unit, rather than
//!     reaching a renderer that would draw a disc of nothing — or of infinity.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::UnitId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::shadow::ModelShadow;
use stormlight_mod_abi::visuals::{ClientRegistration, ModelClips, VisualDescriptor, VisualModel};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Declared {
    Fit,
    /// Hundredths of a world unit; zero and below are not drawable.
    Radius(#[generator(-50i16..=300)] i16),
    None,
}

impl Declared {
    fn shadow(self) -> ModelShadow {
        match self {
            Self::Fit => ModelShadow::Fit,
            Self::Radius(h) => ModelShadow::Radius(f32::from(h) / 100.0),
            Self::None => ModelShadow::None,
        }
    }
}

#[test]
fn a_units_shadow_is_adopted_as_declared_or_refused() {
    check!().with_type::<Declared>().for_each(|d| {
        let model = VisualModel::Model {
            asset: "mod://pack/hero.glb".into(),
            scale: 1.0,
            yaw_offset: 0.0,
            launch: None,
            impact: None,
            clips: ModelClips::default(),
            offset: [0.0; 3],
            shadow: d.shadow(),
        };
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names { units: vec!["hero".into()], ..Names::default() },
            visuals: vec![VisualDescriptor { unit: UnitId(0), model: model.clone() }],
            ..ClientRegistration::default()
        };
        match AdoptedVisuals::adopt(&reg, "pack") {
            Ok(adopted) => {
                assert!(d.shadow().is_valid(), "{d:?}: an undrawable shadow was adopted");
                assert_eq!(adopted.get("hero"), Some(&model), "{d:?}");
            }
            Err(error) => {
                assert!(!d.shadow().is_valid(), "{d:?}: a drawable shadow was refused: {error}");
                assert!(error.to_string().contains("hero"), "the refusal names no unit: {error}");
            }
        }
    });
}
