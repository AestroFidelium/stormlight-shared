//! Marks under units through adoption (stormlight/server#181).
//!
//! Invariants:
//!   - **Drawable marks are adopted as declared, in order** — order is precedence;
//!   - **a mark no renderer can draw refuses the whole mod**, naming the mark;

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::decal::DecalBlend;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::unit_mark::{MarkLook, MarkRole, Relation, UnitMark};
use stormlight_mod_abi::visuals::ClientRegistration;
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, Clone, Copy, TypeGenerator)]
enum Fault {
    Sound,
    NoPicture,
    ZeroWidth,
    NotANumberTint,
}

#[derive(Debug, Clone, Copy, TypeGenerator)]
struct Declared {
    target: bool,
    enemy_only: bool,
    /// Tenths of the body's width.
    #[generator(1u8..=40)]
    scale: u8,
    fault: Fault,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Declared>>().with().len(0usize..=6))]
    declared: Vec<Declared>,
}

fn mark(i: usize, d: Declared) -> UnitMark {
    let mut look = MarkLook {
        asset: format!("mod://pack/mark{i}.png"),
        tint: [0.3, 1.0, 0.4, 0.9],
        blend: DecalBlend::Blend,
        scale: f32::from(d.scale) / 10.0,
        turns: !d.target,
        yaw_offset: 0.0,
    };
    match d.fault {
        Fault::Sound => {}
        Fault::NoPicture => look.asset.clear(),
        Fault::ZeroWidth => look.scale = 0.0,
        Fault::NotANumberTint => look.tint[0] = f32::NAN,
    }
    UnitMark {
        role: if d.target { MarkRole::Target } else { MarkRole::Driven },
        relation: d.enemy_only.then_some(Relation::Enemy),
        look,
    }
}

#[test]
fn drawable_marks_are_adopted_in_order_and_any_other_refuses_the_mod() {
    check!().with_type::<Scenario>().for_each(|s| {
        let marks: Vec<UnitMark> =
            s.declared.iter().enumerate().map(|(i, d)| mark(i, *d)).collect();
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            unit_marks: marks.clone(),
            ..ClientRegistration::default()
        };
        let sound = s.declared.iter().all(|d| matches!(d.fault, Fault::Sound));
        match AdoptedVisuals::adopt(&reg, "pack") {
            Ok(adopted) => {
                assert!(sound, "{s:?}: a mark no renderer can draw was adopted");
                assert_eq!(adopted.unit_marks(), marks.as_slice(), "{s:?}: not as declared");
                assert_eq!(adopted.is_empty(), marks.is_empty(), "{s:?}");
            }
            Err(e) => {
                assert!(!sound, "{s:?}: drawable marks refused: {e}");
                let first = s.declared.iter().position(|d| !matches!(d.fault, Fault::Sound));
                assert!(
                    e.to_string().contains(&format!("unit mark {}", first.unwrap_or_default())),
                    "{s:?}: the refusal does not name the mark: {e}",
                );
            }
        }
    });
}
