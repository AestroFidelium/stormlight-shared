//! Load-time adoption of a cosmetic mod's animations (stormlight/server#72) —
//! turning declared `AnimationDescriptor`s into a unit-name → animation table the
//! client keys on, and refusing the ones it could not animate.
//!
//! Adoption is the last place a broken animation can be reported *with a reason*.
//! Past it the descriptor reaches a renderer that will silently play nothing, so
//! the two hostile-input laws are:
//!   - **Structurally broken is rejected**: a descriptor `validate` refuses (here,
//!     a layer that binds no states) fails adoption rather than being adopted and
//!     ignored.
//!   - **A dangling state handle is rejected**: a custom state with no entry in
//!     the bundle's `anim_states` table is an error, exactly like a dangling unit
//!     handle — never a panic.
//!
//! And the success law: every declared animation is reachable by its unit name.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::animation::{
    AnimLayer, AnimState, AnimationDescriptor, BlendMode, BoneMask, ClipRef, RateBinding, StateClip,
};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::{AnimStateId, UnitId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::ClientRegistration;
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many distinct unit names the cosmetic mod interned.
    units: u8,
    /// How many distinct animation-state names it interned.
    states: u8,
    /// One raw unit handle per animation it declares.
    handles: Vec<u16>,
    /// The custom-state handle each of those animations binds (folded past the
    /// end often enough that dangling references are exercised).
    state_handles: Vec<u16>,
    /// Strip the states off the first animation's only layer.
    empty_layer: bool,
}

fn an_animation(unit: u32, state: AnimState, empty: bool) -> AnimationDescriptor {
    AnimationDescriptor {
        unit: UnitId(unit),
        mask_groups: Vec::new(),
        layers: vec![AnimLayer {
            name: "base".to_string(),
            mask: BoneMask::Whole,
            blend: BlendMode::Override,
            weight: 1.0,
            states: if empty {
                Vec::new()
            } else {
                vec![StateClip {
                    state,
                    clip: ClipRef { asset: "mod://c/rig.glb".to_string(), clip: "run".to_string() },
                    window: None,
                    looping: true,
                    blend_in: 0.1,
                    blend_out: 0.1,
                    rate: RateBinding::Fixed(1.0),
                    priority: 0,
                    notifies: Vec::new(),
                }]
            },
            transitions: Vec::new(),
        }],
    }
}

/// The bundle, plus the unit indices and state indices it referenced.
struct Built {
    reg: ClientRegistration,
    units: Vec<usize>,
    states: Vec<usize>,
}

fn build(s: &Scenario) -> Built {
    let units: Vec<String> = (0..s.units).map(|i| format!("u{i}")).collect();
    let state_names: Vec<String> = (0..s.states).map(|i| format!("s{i}")).collect();

    // Fold each raw handle just past its table so valid and one-past-the-end
    // references both show up with good frequency.
    let unit_ceil = u16::from(s.units).saturating_add(2).max(1);
    let state_ceil = u16::from(s.states).saturating_add(2).max(1);
    let unit_idx: Vec<usize> = s.handles.iter().map(|h| usize::from(h % unit_ceil)).collect();
    let state_idx: Vec<usize> = unit_idx
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let raw = s.state_handles.get(i).copied().unwrap_or(0);
            usize::from(raw % state_ceil)
        })
        .collect();

    let animations = unit_idx
        .iter()
        .zip(&state_idx)
        .enumerate()
        .map(|(i, (&u, &st))| {
            let state = AnimState::Custom(AnimStateId(st as u16));
            an_animation(u as u32, state, s.empty_layer && i == 0)
        })
        .collect();

    Built {
        reg: ClientRegistration {
            abi: ABI_VERSION,
            names: Names { units, anim_states: state_names, ..Names::default() },
            animations,
            ..ClientRegistration::default()
        },
        units: unit_idx,
        states: state_idx,
    }
}

#[test]
fn a_dangling_handle_or_broken_layer_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let b = build(s);
        let dangling_unit = b.units.iter().any(|&i| i >= usize::from(s.units));
        let dangling_state = b.states.iter().any(|&i| i >= usize::from(s.states));
        let broken = s.empty_layer && !b.reg.animations.is_empty();

        let adopted = AdoptedVisuals::adopt(&b.reg, "pack");
        assert_eq!(
            adopted.is_err(),
            dangling_unit || dangling_state || broken,
            "adoption error disagrees with the bundle's defects",
        );
    });
}

#[test]
fn every_adopted_animation_is_reachable_by_unit_name() {
    check!().with_type::<Scenario>().for_each(|s| {
        let b = build(s);
        if let Ok(adopted) = AdoptedVisuals::adopt(&b.reg, "pack") {
            for &i in &b.units {
                let name = format!("u{i}");
                assert!(
                    adopted.animation(&name).is_some(),
                    "declared animation for `{name}` is not reachable after adoption",
                );
            }
        }
    });
}

#[test]
fn a_bundle_with_no_animations_adopts_cleanly() {
    // Additive over one file: every cosmetic mod written before animation existed
    // must still load, with an empty animation table.
    let reg = ClientRegistration { abi: ABI_VERSION, ..ClientRegistration::default() };
    let adopted =
        AdoptedVisuals::adopt(&reg, "pack").expect("an animation-free bundle still adopts");
    assert!(adopted.animations().next().is_none(), "no animations were declared");
}
