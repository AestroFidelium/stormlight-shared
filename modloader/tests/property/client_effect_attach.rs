//! A cosmetic mod's visuals keep the point they asked to hang on through adoption
//! (stormlight/server#160).
//!
//! Invariants:
//!   - **Every declared point is reachable** by the same `(ability name, role)` key
//!     the visual itself is, and is the request the mod declared;
//!   - **A visual that asked for no point has none** — adoption never invents one,
//!     and never carries one across to another role of the same ability.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::attach::AttachPoint;
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::lifetime::EffectLifetime;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, PrimitiveShape, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

/// One visual a mod declares.
#[derive(Debug, TypeGenerator)]
struct Declared {
    /// Which of the mod's abilities, folded into its name table.
    ability: u8,
    /// Whether it is the cast's visual or the shot's.
    cast: bool,
    /// The point it asks for, by index into [`POINTS`], if any.
    point: Option<u8>,
}

const POINTS: [&str; 3] = ["Ref_Weapon", "Ref_Chest", "Ref_Head"];
const ABILITIES: usize = 4;

fn role(d: &Declared) -> EffectRole {
    if d.cast { EffectRole::CastIndicator } else { EffectRole::Projectile }
}

fn request(d: &Declared) -> Option<AttachPoint> {
    d.point.map(|p| AttachPoint::new(POINTS[usize::from(p) % POINTS.len()]))
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Declared>>().with().len(0usize..=8))]
    declared: Vec<Declared>,
}

#[test]
fn every_declared_point_survives_adoption_under_its_visuals_key() {
    check!().with_type::<Scenario>().for_each(|s| {
        let abilities: Vec<String> = (0..ABILITIES).map(|i| format!("a{i}")).collect();
        let effects = s
            .declared
            .iter()
            .map(|d| EffectVisualDescriptor {
                ability: AbilityId(u32::from(d.ability) % ABILITIES as u32),
                role: role(d),
                model: VisualModel::Primitive { shape: PrimitiveShape::Cube, color: [1.0; 4] },
                attach: request(d),
                lifetime: EffectLifetime::Default,
            })
            .collect();
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names { abilities: abilities.clone(), ..Names::default() },
            effects,
            ..ClientRegistration::default()
        };
        let adopted = AdoptedVisuals::adopt(&reg, "pack").expect("every handle is named");
        // A later declaration of the same key overrides an earlier one, as visuals do.
        for (i, d) in s.declared.iter().enumerate() {
            let name = &abilities[usize::from(d.ability) % ABILITIES];
            let overridden = s.declared[i + 1..].iter().any(|later| {
                later.ability % ABILITIES as u8 == d.ability % ABILITIES as u8
                    && role(later) == role(d)
            });
            if overridden {
                continue;
            }
            assert_eq!(
                adopted.effect_attach(name, role(d)),
                request(d).as_ref(),
                "{name}'s {:?} point was lost or invented",
                role(d),
            );
        }
    });
}
