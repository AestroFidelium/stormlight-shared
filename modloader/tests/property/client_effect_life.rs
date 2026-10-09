//! A cosmetic mod's effects keep how long they live and what they do when their
//! unit goes through adoption (stormlight/server#167).
//!
//! Invariants:
//!   - **Every declared lifetime is reachable** under the same key its visual is —
//!     `(ability name, role)` for an ability's feedback, the package-qualified name
//!     for a notify effect — and is the one the mod declared, the latest winning;
//!   - **A lifetime no clock can run is refused at adoption**, naming an effect
//!     that declared one, rather than reaching a renderer that would retire it at
//!     once or never;
//!   - **The merged host tables agree with the adopted ones**, keyed by the global
//!     id the wire carries.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::lifetime::{EffectLifetime, HostEnd};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, NamedEffect, PrimitiveShape,
    VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

const ABILITIES: u8 = 3;
const EFFECTS: u8 = 3;

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Lasting {
    Default,
    /// Tenths of a second; zero is the one no clock can run.
    Seconds(#[generator(0u8..=40)] u8),
    Art,
}

impl Lasting {
    fn lifetime(self) -> EffectLifetime {
        match self {
            Self::Default => EffectLifetime::Default,
            Self::Seconds(tenths) => EffectLifetime::Seconds(f32::from(tenths) / 10.0),
            Self::Art => EffectLifetime::Art,
        }
    }
}

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Ending {
    Follow,
    Detach,
}

impl Ending {
    fn host_end(self) -> HostEnd {
        match self {
            Self::Follow => HostEnd::Follow,
            Self::Detach => HostEnd::Detach,
        }
    }
}

/// One piece of an ability's feedback.
#[derive(Debug, TypeGenerator)]
struct Feedback {
    ability: u8,
    miss: bool,
    lasting: Lasting,
}

/// One effect for a notify to spawn.
#[derive(Debug, TypeGenerator)]
struct Notified {
    name: u8,
    lasting: Lasting,
    ending: Ending,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Feedback>>().with().len(0usize..=6))]
    feedback: Vec<Feedback>,
    #[generator(bolero::produce::<Vec<Notified>>().with().len(0usize..=6))]
    notified: Vec<Notified>,
}

fn model() -> VisualModel {
    VisualModel::Primitive { shape: PrimitiveShape::Sphere, color: [1.0; 4] }
}

fn role(f: &Feedback) -> EffectRole {
    if f.miss { EffectRole::Miss } else { EffectRole::Impact }
}

fn registration(s: &Scenario) -> ClientRegistration {
    ClientRegistration {
        abi: ABI_VERSION,
        names: Names {
            abilities: (0..ABILITIES).map(|i| format!("a{i}")).collect(),
            ..Names::default()
        },
        effects: s
            .feedback
            .iter()
            .map(|f| EffectVisualDescriptor {
                ability: AbilityId(u32::from(f.ability % ABILITIES)),
                role: role(f),
                model: model(),
                attach: None,
                lifetime: f.lasting.lifetime(),
            })
            .collect(),
        named_effects: s
            .notified
            .iter()
            .map(|n| NamedEffect {
                name: format!("fx{}", n.name % EFFECTS),
                model: model(),
                lifetime: n.lasting.lifetime(),
                on_host_end: n.ending.host_end(),
            })
            .collect(),
        ..ClientRegistration::default()
    }
}

#[test]
fn every_declared_life_survives_adoption_under_its_effects_key() {
    check!().with_type::<Scenario>().for_each(|s| {
        // The names of every declaration whose lifetime no clock can run.
        let unrunnable: Vec<String> = s
            .feedback
            .iter()
            .filter(|f| !f.lasting.lifetime().is_runnable())
            .map(|f| format!("a{}", f.ability % ABILITIES))
            .chain(
                s.notified
                    .iter()
                    .filter(|n| !n.lasting.lifetime().is_runnable())
                    .map(|n| format!("fx{}", n.name % EFFECTS)),
            )
            .collect();
        let adopted = match AdoptedVisuals::adopt(&registration(s), "pack") {
            Ok(adopted) => adopted,
            Err(error) => {
                let error = error.to_string();
                assert!(
                    unrunnable.iter().any(|name| error.contains(name.as_str())),
                    "the refusal `{error}` names none of {unrunnable:?}",
                );
                return;
            }
        };
        assert!(unrunnable.is_empty(), "a lifetime no clock can run was adopted: {unrunnable:?}");

        for (i, f) in s.feedback.iter().enumerate() {
            let later = s.feedback[i + 1..]
                .iter()
                .any(|l| l.ability % ABILITIES == f.ability % ABILITIES && role(l) == role(f));
            if later {
                continue;
            }
            let name = format!("a{}", f.ability % ABILITIES);
            assert_eq!(adopted.effect_lifetime(&name, role(f)), f.lasting.lifetime(), "{name}");
        }
        for (i, n) in s.notified.iter().enumerate() {
            if s.notified[i + 1..].iter().any(|l| l.name % EFFECTS == n.name % EFFECTS) {
                continue;
            }
            let key = format!("pack/fx{}", n.name % EFFECTS);
            let effect = adopted.named_effect(&key).expect("a declared effect is adopted");
            assert_eq!(effect.lifetime, n.lasting.lifetime(), "{key}");
            assert_eq!(effect.on_host_end, n.ending.host_end(), "{key}");
        }
    });
}
