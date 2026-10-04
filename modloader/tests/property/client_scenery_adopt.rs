//! Invariants of scenery adoption (`AdoptedVisuals::adopt`) — the point where a
//! map's cosmetic half hands over the art it is dressed in. Directional /
//! structural, total over hostile input:
//!   - **An unplaceable piece is refused, never drawn**: adoption errs exactly
//!     when some placement has a non-finite component or a rotation that is not
//!     a turn at all (a zero quaternion), and never panics. Past adoption a NaN
//!     placement reaches a renderer that would draw the piece nowhere, or
//!     everywhere, and say nothing.
//!   - **Declaration order is preserved**: pieces come back exactly as declared —
//!     a map lays its ground before the props on it.
//!   - **Scenery alone is content**: a bundle that ships only scenery is not
//!     "empty", so a map's cosmetic half with no units to dress still counts.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::scenery::{SceneryPiece, SceneryPlacement};
use stormlight_mod_abi::visuals::{ClientRegistration, ModelClips};
use stormlight_modloader::client::AdoptedVisuals;

/// How each generated placement is broken, if at all.
#[derive(Debug, TypeGenerator, Clone, Copy, PartialEq, Eq)]
enum Break {
    None,
    NanTranslation,
    InfiniteScale,
    ZeroRotation,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    pieces: Vec<Vec<(Break, [i8; 3])>>,
}

fn placement(brk: Break, t: [i8; 3]) -> SceneryPlacement {
    let mut p = SceneryPlacement { translation: t.map(f32::from), ..SceneryPlacement::IDENTITY };
    match brk {
        Break::None => {}
        Break::NanTranslation => p.translation[1] = f32::NAN,
        Break::InfiniteScale => p.scale[0] = f32::INFINITY,
        Break::ZeroRotation => p.rotation = [0.0; 4],
    }
    p
}

fn build(s: &Scenario) -> ClientRegistration {
    ClientRegistration {
        abi: ABI_VERSION,
        scenery: s
            .pieces
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, ps)| SceneryPiece {
                asset: format!("mod://map/scenery/{i}.glb"),
                clips: ModelClips::default(),
                placements: ps.iter().take(8).map(|(b, t)| placement(*b, *t)).collect(),
            })
            .collect(),
        ..ClientRegistration::default()
    }
}

#[test]
fn an_unplaceable_piece_is_refused_and_the_rest_kept_in_order() {
    check!().with_type::<Scenario>().for_each(|s| {
        let reg = build(s);
        let broken = reg.scenery.iter().flat_map(|p| &p.placements).any(|p| {
            p.translation.iter().chain(&p.rotation).chain(&p.scale).any(|v| !v.is_finite())
                || p.rotation.iter().all(|c| *c == 0.0)
        });
        let adopted = AdoptedVisuals::adopt(&reg, "map");
        assert_eq!(adopted.is_err(), broken, "adoption disagrees with placement soundness");
        if let Ok(adopted) = adopted {
            assert_eq!(adopted.scenery(), reg.scenery.as_slice(), "pieces altered or reordered");
        }
    });
}

#[test]
fn scenery_alone_is_not_empty() {
    check!().with_type::<u8>().for_each(|n| {
        let pieces = usize::from(*n % 5);
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            scenery: (0..pieces)
                .map(|i| SceneryPiece {
                    asset: format!("mod://map/{i}.glb"),
                    clips: ModelClips::default(),
                    placements: vec![SceneryPlacement::IDENTITY],
                })
                .collect(),
            ..ClientRegistration::default()
        };
        let adopted = AdoptedVisuals::adopt(&reg, "map").expect("sound scenery adopts");
        assert_eq!(adopted.is_empty(), pieces == 0);
    });
}

/// A ground field that cannot be sampled is refused at adoption, with the mod
/// named, rather than lifting every unit by garbage; a sound one is carried whole.
#[test]
fn an_unusable_ground_is_refused_and_a_sound_one_kept() {
    use stormlight_mod_abi::scenery::HeightField;
    check!().with_type::<(u8, u8, bool, bool)>().for_each(|(w, h, short, nan)| {
        let (w, h) = (u32::from(*w % 5) + 1, u32::from(*h % 5) + 1);
        let mut heights = vec![0.5; (w * h) as usize];
        if *short {
            heights.pop();
        }
        if *nan && !heights.is_empty() {
            heights[0] = f32::NAN;
        }
        let field = HeightField { origin: [0.0; 2], cell: 1.0, size: [w, h], heights };
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            ground: Some(field.clone()),
            ..ClientRegistration::default()
        };
        let adopted = AdoptedVisuals::adopt(&reg, "map");
        assert_eq!(adopted.is_err(), !field.is_valid(), "adoption disagrees with field soundness");
        if let Ok(a) = adopted {
            assert_eq!(a.ground(), Some(&field));
            assert!(!a.is_empty(), "a map that only declares its ground still declares something");
        }
    });
}
