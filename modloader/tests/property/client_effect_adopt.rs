//! Invariants of cosmetic-mod *effect*-visual adoption (`AdoptedVisuals::adopt`)
//! — turning a decoded `ClientRegistration`'s `effects` into an
//! `(ability name, role) → visual` table the client keys on. Parallel to
//! `client_adopt` (which covers unit visuals); directional/structural, total over
//! hostile input:
//!   - **Dangling ability handle is rejected, never panics**: adoption errs
//!     exactly when some effect visual references an ability handle with no entry
//!     in the `names.abilities` table.
//!   - **Every declared effect visual is reachable**: on success, each effect's
//!     `(ability name, role)` resolves back to a model in the table.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, PrimitiveShape, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many distinct ability names the cosmetic mod interned.
    names_len: u8,
    /// One `(raw ability handle, role selector)` per effect visual it declares.
    handles: Vec<(u16, u8)>,
}

fn role_of(sel: u8) -> EffectRole {
    match sel % 3 {
        0 => EffectRole::Projectile,
        1 => EffectRole::Impact,
        _ => EffectRole::CastIndicator,
    }
}

fn build(s: &Scenario) -> (ClientRegistration, Vec<(usize, EffectRole)>) {
    // Distinct names `a0..a{n-1}` so name-keying is unambiguous.
    let abilities: Vec<String> = (0..s.names_len).map(|i| format!("a{i}")).collect();
    // Fold each raw handle into `[0, names_len + 1]` so we see valid indices and
    // one-past-the-end dangling references with good frequency.
    let ceil = u16::from(s.names_len).saturating_add(2).max(1);
    let keys: Vec<(usize, EffectRole)> =
        s.handles.iter().map(|&(h, r)| (usize::from(h % ceil), role_of(r))).collect();
    let effects = keys
        .iter()
        .map(|&(i, role)| EffectVisualDescriptor {
            ability: AbilityId(i as u32),
            role,
            model: VisualModel::Primitive { shape: PrimitiveShape::Sphere, color: [0.0; 4] },
        })
        .collect();
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { abilities, ..Names::default() },
        visuals: Vec::new(),
        effects,
    };
    (reg, keys)
}

#[test]
fn a_dangling_ability_handle_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        let any_dangling = keys.iter().any(|&(i, _)| i >= usize::from(s.names_len));
        let adopted = AdoptedVisuals::adopt(&reg);
        assert_eq!(
            adopted.is_err(),
            any_dangling,
            "adoption error disagrees with dangling ability-handle presence",
        );
    });
}

#[test]
fn every_adopted_effect_is_reachable_by_ability_name_and_role() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        if let Ok(adopted) = AdoptedVisuals::adopt(&reg) {
            for &(i, role) in &keys {
                let name = format!("a{i}");
                assert!(
                    adopted.effect(&name, role).is_some(),
                    "declared effect visual for `{name}`/{role:?} is not reachable after adoption",
                );
            }
        }
    });
}
