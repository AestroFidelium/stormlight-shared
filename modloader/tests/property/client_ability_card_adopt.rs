//! A cosmetic mod's ability cards survive adoption whole (stormlight/server#116).
//!
//! Invariants:
//!   - **Every card is reachable by its ability's name**, words and picture both,
//!     and the later declaration of one ability wins whole;
//!   - **The picture is still an icon**: a card with a picture answers the icon
//!     lookup the slot's art is drawn through, and a card with no picture does not
//!     pretend to have one.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{AbilityCard, CardInfo, ClientRegistration};
use stormlight_modloader::client::AdoptedVisuals;

const ABILITIES: u32 = 4;

#[derive(Debug, TypeGenerator)]
struct Declared {
    ability: u8,
    named: bool,
    described: bool,
    pictured: bool,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Declared>>().with().len(0usize..=8))]
    cards: Vec<Declared>,
}

fn info(i: usize, d: &Declared) -> CardInfo {
    let pick = |on: bool, text: String| if on { text } else { String::new() };
    CardInfo {
        name: pick(d.named, format!("Name {i}")),
        description: pick(d.described, format!("Does {i}")),
        image: pick(d.pictured, format!("mod://pack/{i}.png")),
    }
}

#[test]
fn every_card_survives_adoption_and_the_later_wins_whole() {
    check!().with_type::<Scenario>().for_each(|s| {
        let abilities: Vec<String> = (0..ABILITIES).map(|i| format!("a{i}")).collect();
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            names: Names { abilities: abilities.clone(), ..Names::default() },
            ability_cards: s
                .cards
                .iter()
                .enumerate()
                .map(|(i, d)| AbilityCard {
                    ability: AbilityId(u32::from(d.ability) % ABILITIES),
                    info: info(i, d),
                })
                .collect(),
            ..ClientRegistration::default()
        };
        let adopted = AdoptedVisuals::adopt(&reg, "pack").expect("every handle is named");
        for (index, name) in abilities.iter().enumerate() {
            let last = s
                .cards
                .iter()
                .enumerate()
                .rev()
                .find(|(_, d)| usize::from(d.ability) % ABILITIES as usize == index);
            match last {
                None => assert_eq!(adopted.ability_card(name), None, "{name} grew a card"),
                Some((i, d)) => {
                    let expected = info(i, d);
                    assert_eq!(adopted.ability_card(name), Some(&expected), "{name}'s card");
                    let icon = adopted.icon(name);
                    if d.pictured {
                        assert_eq!(icon, Some(expected.image.as_str()), "the picture is lost");
                    } else {
                        assert_eq!(icon, None, "a card without a picture claimed one");
                    }
                }
            }
        }
    });
}
