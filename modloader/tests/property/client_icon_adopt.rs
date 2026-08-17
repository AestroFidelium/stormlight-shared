//! Invariants of cosmetic-mod *icon* adoption (`AdoptedVisuals::adopt`,
//! server#94) — turning a decoded `ClientRegistration`'s `icons` into the
//! `ability name → picture` table the client resolves a slot's icon through.
//! Parallel to `client_effect_adopt` (the same handle, a different fact about
//! it); directional/structural, total over hostile input:
//!   - **Dangling ability handle is rejected, never panics**: adoption errs
//!     exactly when some icon references an ability handle with no entry in the
//!     `names.abilities` table. An icon that survived on a dangling handle would
//!     be keyed to no ability at all and could never be drawn.
//!   - **Every declared icon is reachable**: on success each icon's ability name
//!     resolves back to a path.
//!   - **The later declaration wins**: two icons for one ability leave the table
//!     holding the last one, deterministically — the same "later wins" rule the
//!     visual and effect tables follow.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{AbilityIcon, ClientRegistration};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many distinct ability names the cosmetic mod interned.
    names_len: u8,
    /// One raw ability handle per icon it declares.
    handles: Vec<u16>,
}

/// The path an icon declared against the `i`-th handle carries, made unique per
/// declaration index so "the later one wins" is observable.
fn path(index: usize, handle: usize) -> String {
    format!("mod://pack/icon_{handle}_{index}.png")
}

fn build(s: &Scenario) -> (ClientRegistration, Vec<usize>) {
    let abilities: Vec<String> = (0..s.names_len).map(|i| format!("a{i}")).collect();
    // Fold each raw handle into `[0, names_len + 1]` so valid indices and
    // one-past-the-end dangling references both show up with good frequency.
    let ceil = u16::from(s.names_len).saturating_add(2).max(1);
    let keys: Vec<usize> = s.handles.iter().map(|&h| usize::from(h % ceil)).collect();
    let icons = keys
        .iter()
        .enumerate()
        .map(|(i, &handle)| AbilityIcon {
            ability: AbilityId(handle as u32),
            image: path(i, handle),
        })
        .collect();
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { abilities, ..Names::default() },
        icons,
        ..ClientRegistration::default()
    };
    (reg, keys)
}

#[test]
fn a_dangling_ability_handle_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        let any_dangling = keys.iter().any(|&i| i >= usize::from(s.names_len));
        let adopted = AdoptedVisuals::adopt(&reg, "pack");
        assert_eq!(
            adopted.is_err(),
            any_dangling,
            "adoption error disagrees with dangling ability-handle presence",
        );
    });
}

#[test]
fn every_adopted_icon_is_reachable_by_ability_name() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        if let Ok(adopted) = AdoptedVisuals::adopt(&reg, "pack") {
            for &i in &keys {
                let name = format!("a{i}");
                assert!(
                    adopted.icon(&name).is_some(),
                    "declared icon for `{name}` is not reachable after adoption",
                );
            }
        }
    });
}

#[test]
fn the_last_icon_declared_for_an_ability_is_the_one_adopted() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        let Ok(adopted) = AdoptedVisuals::adopt(&reg, "pack") else { return };
        for (i, &handle) in keys.iter().enumerate() {
            // The last declaration index this ability appears at.
            let last = keys.iter().rposition(|&k| k == handle).expect("the handle is in the list");
            if i != last {
                continue;
            }
            let name = format!("a{handle}");
            assert_eq!(
                adopted.icon(&name),
                Some(path(last, handle).as_str()),
                "`{name}` kept an earlier icon instead of its last declaration",
            );
        }
    });
}
