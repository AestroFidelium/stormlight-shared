//! Invariants of cosmetic-mod *talent card* adoption (`AdoptedVisuals::adopt`,
//! server#95) — turning a decoded `ClientRegistration`'s `cards` into the
//! `talent name → card` table a talent panel resolves an offered option through.
//!
//! Parallel to `client_icon_adopt` one family along: the same shape of
//! declaration, keyed by talent instead of ability, because a panel addresses a
//! cell by tier and option index and so has nothing of its own to print there.
//! Directional/structural, total over hostile input:
//!   - **Dangling talent handle is rejected, never panics**: adoption errs exactly
//!     when some card references a talent handle with no entry in the
//!     `names.talents` table. A card that survived on a dangling handle would be
//!     keyed to no talent at all and would describe whichever one happened to land
//!     on that raw index.
//!   - **Every declared card is reachable**: on success each card's talent name
//!     resolves back to the words it carried.
//!   - **The later declaration wins**: two cards for one talent leave the table
//!     holding the last one, deterministically — the same "later wins" rule the
//!     visual, effect and icon tables follow.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::TalentId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{ClientRegistration, TalentCard, TalentInfo};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many distinct talent names the cosmetic mod interned.
    names_len: u8,
    /// One raw talent handle per card it declares.
    handles: Vec<u16>,
}

/// The words a card declared against the `i`-th handle carries, made unique per
/// declaration index so "the later one wins" is observable.
fn info(index: usize, handle: usize) -> TalentInfo {
    TalentInfo {
        name: format!("Talent {handle} ({index})"),
        description: format!("what talent {handle} does, take {index}"),
        image: format!("mod://pack/talent_{handle}_{index}.png"),
    }
}

fn build(s: &Scenario) -> (ClientRegistration, Vec<usize>) {
    let talents: Vec<String> = (0..s.names_len).map(|i| format!("t{i}")).collect();
    // Fold each raw handle into `[0, names_len + 1]` so valid indices and
    // one-past-the-end dangling references both show up with good frequency.
    let ceil = u16::from(s.names_len).saturating_add(2).max(1);
    let keys: Vec<usize> = s.handles.iter().map(|&h| usize::from(h % ceil)).collect();
    let cards = keys
        .iter()
        .enumerate()
        .map(|(i, &handle)| TalentCard { talent: TalentId(handle as u32), info: info(i, handle) })
        .collect();
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { talents, ..Names::default() },
        cards,
        ..ClientRegistration::default()
    };
    (reg, keys)
}

#[test]
fn a_dangling_talent_handle_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        let any_dangling = keys.iter().any(|&i| i >= usize::from(s.names_len));
        let adopted = AdoptedVisuals::adopt(&reg, "pack");
        assert_eq!(
            adopted.is_err(),
            any_dangling,
            "adoption error disagrees with dangling talent-handle presence",
        );
    });
}

#[test]
fn every_adopted_card_is_reachable_by_talent_name() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        if let Ok(adopted) = AdoptedVisuals::adopt(&reg, "pack") {
            for &i in &keys {
                let name = format!("t{i}");
                assert!(
                    adopted.card(&name).is_some(),
                    "declared card for `{name}` is not reachable after adoption",
                );
            }
        }
    });
}

#[test]
fn the_last_card_declared_for_a_talent_is_the_one_adopted() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, keys) = build(s);
        let Ok(adopted) = AdoptedVisuals::adopt(&reg, "pack") else { return };
        for (i, &handle) in keys.iter().enumerate() {
            // The last declaration index this talent appears at.
            let last = keys.iter().rposition(|&k| k == handle).expect("the handle is in the list");
            if i != last {
                continue;
            }
            let name = format!("t{handle}");
            assert_eq!(
                adopted.card(&name),
                Some(&info(last, handle)),
                "`{name}` kept an earlier card instead of its last declaration",
            );
        }
    });
}
