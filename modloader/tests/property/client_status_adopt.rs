//! A cosmetic mod's status visuals through adoption (stormlight/server#171).
//!
//! Invariants:
//!   - **Every declared look is reachable by its buff's name**, the latest
//!     declaration of a buff winning, and is the look the mod declared;
//!   - **A look nobody can see is refused** at adoption, naming the buff, rather
//!     than carried to a renderer that would draw nothing for it forever.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::BuffId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::status_visual::{StatusLook, StatusVisual};
use stormlight_mod_abi::visuals::{ClientRegistration, PrimitiveShape, VisualModel};
use stormlight_modloader::client::AdoptedVisuals;

const BUFFS: u8 = 3;

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Half {
    Nothing,
    Tinted(u8),
}

impl Half {
    fn model(self) -> Option<VisualModel> {
        match self {
            Self::Nothing => None,
            Self::Tinted(t) => Some(VisualModel::Primitive {
                shape: PrimitiveShape::Capsule,
                color: [f32::from(t) / 255.0, 0.3, 0.6, 1.0],
            }),
        }
    }
}

#[derive(Debug, TypeGenerator)]
struct Declared {
    buff: u8,
    own: Half,
    others: Half,
}

impl Declared {
    fn look(&self) -> StatusLook {
        StatusLook { own: self.own.model(), others: self.others.model(), attach: None }
    }
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Declared>>().with().len(0usize..=6))]
    declared: Vec<Declared>,
}

#[test]
fn every_seen_look_survives_adoption_under_its_buffs_name() {
    check!().with_type::<Scenario>().for_each(|s| {
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names {
                buffs: (0..BUFFS).map(|b| format!("b{b}")).collect(),
                ..Names::default()
            },
            status_visuals: s
                .declared
                .iter()
                .map(|d| StatusVisual { buff: BuffId(u16::from(d.buff % BUFFS)), look: d.look() })
                .collect(),
            ..ClientRegistration::default()
        };
        let blind: Vec<String> = s
            .declared
            .iter()
            .filter(|d| !d.look().shows_anything())
            .map(|d| format!("b{}", d.buff % BUFFS))
            .collect();
        let adopted = match AdoptedVisuals::adopt(&reg, "pack") {
            Ok(adopted) => adopted,
            Err(error) => {
                let error = error.to_string();
                assert!(
                    blind.iter().any(|name| error.contains(name.as_str())),
                    "the refusal `{error}` names none of {blind:?}",
                );
                return;
            }
        };
        assert!(blind.is_empty(), "a look nobody can see was adopted: {blind:?}");
        for (i, d) in s.declared.iter().enumerate() {
            if s.declared[i + 1..].iter().any(|l| l.buff % BUFFS == d.buff % BUFFS) {
                continue;
            }
            let name = format!("b{}", d.buff % BUFFS);
            assert_eq!(adopted.status_look(&name), Some(&d.look()), "{name}");
        }
    });
}
