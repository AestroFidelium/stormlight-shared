//! Load-time adoption of animation notifies (stormlight/server#76) — the step
//! that makes a mod's own effect names safe to merge with every other mod's.
//!
//! A notify names its effect with a string the declaring mod chose, so two
//! packages that both ship a `footstep` would collide the moment their tables are
//! merged — one character silently spraying the other's art. Adoption is where
//! that is settled: each declared key is **qualified with its package**, and every
//! notify inside that mod's animations is rewritten to the qualified form, so a
//! key resolves to the effect its own author declared and to nothing else.
//!
//! Laws, all hostile-input safe:
//!   - **Qualified and reachable**: every declared key is reachable under its
//!     package-qualified name, and every effect notify carries the qualified key.
//!   - **No cross-mod capture**: two mods declaring the same local name keep two
//!     distinct entries, and each mod's notifies point at its own.
//!   - **A dangling event is rejected**: a trigger notify naming an event with no
//!     `names.events` entry fails adoption with a reason — never a panic, never a
//!     notify pointing at nothing.
//!   - **Unknown keys survive adoption**: a notify naming an effect the mod never
//!     declared still loads (the runtime reports and skips it), because a mistyped
//!     puff must not cost the author their whole character.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::animation::{
    AnimLayer, AnimState, AnimationDescriptor, BlendMode, BoneMask, ClipRef, RateBinding, StateClip,
};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::{EventId, UnitId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::notify::{NotifyAction, NotifyAttach, NotifyPoint, NotifyTime};
use stormlight_mod_abi::visuals::{ClientRegistration, NamedEffect, PrimitiveShape, VisualModel};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// 0..=3 named effects the mod declares (`fx0`, `fx1`, …).
    effects: u8,
    /// 0..=3 event names it interns.
    events: u8,
    /// The event handle its trigger notify names — folded past the end often
    /// enough that dangling references show up.
    event_handle: u16,
    /// Whether the clip carries a trigger notify at all.
    triggering: bool,
    /// Whether one notify names a key the mod never declared.
    unknown_key: bool,
}

fn a_model(seed: usize) -> VisualModel {
    VisualModel::Primitive {
        shape: match seed % 3 {
            0 => PrimitiveShape::Cube,
            1 => PrimitiveShape::Sphere,
            _ => PrimitiveShape::Capsule,
        },
        color: [0.2, 0.4, 0.6, 1.0],
    }
}

fn effect_notify(key: &str) -> NotifyPoint {
    NotifyPoint {
        at: NotifyTime::Normalized(0.5),
        action: NotifyAction::Effect {
            key: key.to_string(),
            attach: NotifyAttach::Socket {
                bone: vec!["Root".to_string(), "Foot.L".to_string()],
                offset: [0.0, 0.05, 0.0],
            },
            lifetime: 0.25,
        },
    }
}

fn an_animation(unit: u32, notifies: Vec<NotifyPoint>) -> AnimationDescriptor {
    AnimationDescriptor {
        unit: UnitId(unit),
        mask_groups: Vec::new(),
        layers: vec![AnimLayer {
            name: "base".to_string(),
            mask: BoneMask::Whole,
            blend: BlendMode::Override,
            weight: 1.0,
            states: vec![StateClip {
                state: AnimState::Walk,
                clip: ClipRef { asset: "mod://c/rig.glb".to_string(), clip: "walk".to_string() },
                window: None,
                looping: true,
                blend_in: 0.1,
                blend_out: 0.1,
                rate: RateBinding::Fixed(1.0),
                priority: 0,
                notifies,
            }],
            transitions: Vec::new(),
        }],
    }
}

/// The bundle a cosmetic mod would emit for this scenario, plus how many keys it
/// declared and which event index its trigger names.
struct Built {
    reg: ClientRegistration,
    keys: usize,
    event: usize,
}

fn build(s: &Scenario) -> Built {
    let keys = usize::from(s.effects % 4);
    let events = usize::from(s.events % 4);
    let ceil = u16::try_from(events).expect("small").saturating_add(2).max(1);
    let event = usize::from(s.event_handle % ceil);

    let mut notifies: Vec<NotifyPoint> =
        (0..keys).map(|k| effect_notify(&format!("fx{k}"))).collect();
    if s.unknown_key {
        notifies.push(effect_notify("never-declared"));
    }
    if s.triggering {
        notifies.push(NotifyPoint {
            at: NotifyTime::Seconds(0.2),
            action: NotifyAction::Trigger { event: EventId(event as u16) },
        });
    }

    Built {
        reg: ClientRegistration {
            abi: ABI_VERSION,
            names: Names {
                units: vec!["hero".to_string()],
                events: (0..events).map(|e| format!("ev{e}")).collect(),
                ..Names::default()
            },
            animations: vec![an_animation(0, notifies)],
            named_effects: (0..keys)
                .map(|k| NamedEffect { name: format!("fx{k}"), model: a_model(k) })
                .collect(),
            ..ClientRegistration::default()
        },
        keys,
        event,
    }
}

/// Every notify point in an adopted animation, in declaration order.
fn notifies_of(adopted: &AdoptedVisuals) -> Vec<NotifyPoint> {
    adopted
        .animations()
        .flat_map(|(_, a)| a.layers.iter())
        .flat_map(|l| l.states.iter())
        .flat_map(|s| s.notifies.iter().cloned())
        .collect()
}

#[test]
fn a_trigger_naming_an_unknown_event_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let b = build(s);
        let events = usize::from(s.events % 4);
        let dangling = s.triggering && b.event >= events;

        let adopted = AdoptedVisuals::adopt(&b.reg, "pack");
        assert_eq!(
            adopted.is_err(),
            dangling,
            "adoption error disagrees with whether the trigger's event exists",
        );
    });
}

#[test]
fn every_declared_key_is_qualified_by_its_package_and_so_is_every_notify() {
    check!().with_type::<Scenario>().for_each(|s| {
        let b = build(s);
        let Ok(adopted) = AdoptedVisuals::adopt(&b.reg, "pack") else { return };

        for k in 0..b.keys {
            let key = format!("pack/fx{k}");
            assert!(
                adopted.named_effect(&key).is_some(),
                "declared effect `fx{k}` is not reachable as `{key}` after adoption",
            );
            assert!(
                adopted.named_effect(&format!("fx{k}")).is_none(),
                "an unqualified key still resolves, so two packages could collide",
            );
        }

        for point in notifies_of(&adopted) {
            if let NotifyAction::Effect { key, .. } = &point.action {
                assert!(
                    key.starts_with("pack/"),
                    "notify key `{key}` was not qualified with its package",
                );
            }
        }
    });
}

#[test]
fn an_unknown_key_still_adopts_and_stays_unresolvable() {
    check!().with_type::<Scenario>().for_each(|s| {
        let b = build(s);
        if !s.unknown_key {
            return;
        }
        let Ok(adopted) = AdoptedVisuals::adopt(&b.reg, "pack") else { return };
        // Loaded, qualified like every other key, and resolving to nothing — which
        // is what leaves the runtime a report entry to make instead of a mystery.
        assert!(
            adopted.named_effect("pack/never-declared").is_none(),
            "a key the mod never declared must not resolve to some other effect",
        );
        assert!(
            notifies_of(&adopted).iter().any(|p| matches!(
                &p.action,
                NotifyAction::Effect { key, .. } if key == "pack/never-declared"
            )),
            "the notify itself survived adoption",
        );
    });
}

#[test]
fn two_packages_declaring_the_same_name_keep_their_own_effects() {
    // Additive over one file: the collision the qualification exists to prevent.
    let mut reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { units: vec!["hero".to_string()], ..Names::default() },
        animations: vec![an_animation(0, vec![effect_notify("footstep")])],
        named_effects: vec![NamedEffect { name: "footstep".to_string(), model: a_model(0) }],
        ..ClientRegistration::default()
    };
    let first = AdoptedVisuals::adopt(&reg, "boots").expect("adopts");
    reg.named_effects[0].model = a_model(1);
    let second = AdoptedVisuals::adopt(&reg, "sandals").expect("adopts");

    let boots = first.named_effect("boots/footstep").expect("the first package's puff");
    let sandals = second.named_effect("sandals/footstep").expect("the second package's puff");
    assert_ne!(boots, sandals, "one package's effect was captured by the other's name");
    assert!(
        first.named_effect("sandals/footstep").is_none(),
        "a package resolves only its own keys",
    );
}
