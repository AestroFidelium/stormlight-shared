//! Which of the player's buttons each talent is about, in the client's own id space
//! (stormlight/server#129).
//!
//! [`TalentDescriptor::changes`] answers the question from one mod's declaration;
//! this is the bridge that makes the answer usable by a HUD. Two id spaces have to
//! be crossed and they are crossed in opposite directions:
//!
//!   - the **talent** the answer is keyed by is a local handle in its own mod, and
//!     the wire carries the global one — so a HUD that looked a focus up by raw
//!     index would show the second mod's talents wearing the first mod's keys;
//!   - the **ability** an answer names is a local handle too, and the loadout it
//!     will be matched against is global.
//!
//! A slot crosses neither: it is a position on the caster's bar and means the same
//! thing in every mod, which is exactly why it is the shape a HUD can act on
//! immediately.
//!
//! Hermetic `.wat` fixtures, the shape `client_talent_names.rs` uses.

use std::io::{Cursor, Write};

use stormlight_mod_abi::common::NumOp;
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{AbilityId, ParamId, Slot, TalentId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::math::Value;
use stormlight_mod_abi::talents::{
    AbilityFocus, AbilitySelector, GrantAbility, GrantTarget, ParamPatch, TalentDescriptor,
};
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

fn guest_wasm(payload: &[u8]) -> Vec<u8> {
    let escaped: String = payload.iter().map(|b| format!("\\{b:02x}")).collect();
    let packed = (u64::from(DATA_OFFSET) << 32) | payload.len() as u64;
    let wat = format!(
        "(module\n\
         \x20 (memory (export \"memory\") 1)\n\
         \x20 (data (i32.const {DATA_OFFSET}) \"{escaped}\")\n\
         \x20 (func (export \"mod_register\") (result i64) (i64.const {packed})))"
    );
    wat::parse_str(&wat).expect("fixture wat compiles")
}

/// A talent patching the ability in `slot`.
fn patches_slot(id: u32, slot: u8) -> TalentDescriptor {
    TalentDescriptor {
        id: TalentId(id),
        selector: vec![AbilitySelector::Slot(Slot(slot))],
        patches: vec![ParamPatch {
            param: ParamId(0),
            op: NumOp::Add,
            value: Value::Const(1.0),
            selector: None,
        }],
        riders: Vec::new(),
        add_reactions: Vec::new(),
        grants: Vec::new(),
        modifiers: Vec::new(),
        tags: Vec::new(),
        quest: None,
    }
}

/// A talent patching one **named** ability, wherever the caster carries it.
fn patches_ability(id: u32, ability: u32) -> TalentDescriptor {
    TalentDescriptor {
        selector: vec![AbilitySelector::Ability(AbilityId(ability))],
        ..patches_slot(id, 0)
    }
}

/// A talent handing over a new button and patching nothing.
fn grants(id: u32, slot: u8, ability: u32) -> TalentDescriptor {
    TalentDescriptor {
        selector: vec![AbilitySelector::Any],
        patches: Vec::new(),
        grants: vec![GrantAbility {
            ability: AbilityId(ability),
            into: GrantTarget::Exact(Slot(slot)),
        }],
        ..patches_slot(id, 0)
    }
}

/// A gameplay `.zip` whose registration is exactly these names and talents.
fn gameplay(
    id: &str,
    abilities: &[&str],
    talents: &[&str],
    declared: Vec<TalentDescriptor>,
) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            abilities: abilities.iter().map(|s| (*s).into()).collect(),
            talents: talents.iter().map(|s| (*s).into()).collect(),
            ..Names::default()
        },
        talents: declared,
        ..Registration::default()
    };
    let payload = postcard::to_allocvec(&reg).unwrap();
    let wasm = guest_wasm(&payload);
    let manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\n\
         kind = \"server\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
        ABI_VERSION.major
    );
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file(ENTRY, opts).unwrap();
        zip.write_all(&wasm).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn a_focus_is_keyed_by_the_global_talent_and_names_a_global_ability() {
    // Two mods in load order. The second mod's talent 0 and ability 0 are the
    // *third* of each globally, and a bridge that forgot either would hand a HUD
    // the first mod's answer for the second mod's talent.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolt", "ward"],
        &["swift", "sturdy"],
        vec![patches_slot(0, 1), grants(1, 3, 0)],
    ))
    .expect("first loads");
    host.load_gameplay_zip_bytes(&gameplay(
        "second",
        &["lance"],
        &["keen"],
        vec![patches_ability(0, 0)],
    ))
    .expect("second loads");

    let focus: Vec<(TalentId, AbilityFocus)> = host.talent_focus().collect();
    assert_eq!(
        focus,
        vec![
            // Patching by slot: a position on the bar, the same in every mod.
            (TalentId(0), AbilityFocus::Slot(Slot(1))),
            // Granting: the slot the new button lands in.
            (TalentId(1), AbilityFocus::Slot(Slot(3))),
            // Patching by name: the second mod's local ability 0 is `lance`, which
            // is global 2 — the id the wire carries for it.
            (TalentId(2), AbilityFocus::Ability(AbilityId(2))),
        ],
        "a talent's focus must be keyed and named in the global id space",
    );
}

/// A talent about no single button contributes nothing at all, rather than an entry
/// a HUD has to know to ignore.
#[test]
fn a_talent_about_no_single_button_is_simply_absent() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolt"],
        &["vague", "aimed"],
        vec![
            TalentDescriptor { selector: vec![AbilitySelector::Any], ..patches_slot(0, 0) },
            patches_slot(1, 2),
        ],
    ))
    .expect("first loads");

    let focus: Vec<(TalentId, AbilityFocus)> = host.talent_focus().collect();
    assert_eq!(
        focus,
        vec![(TalentId(1), AbilityFocus::Slot(Slot(2)))],
        "a talent selecting no single ability must contribute no focus",
    );
}

#[test]
fn with_no_gameplay_mod_no_talent_is_about_anything() {
    let host = ClientHost::new().unwrap();
    assert_eq!(
        host.talent_focus().count(),
        0,
        "the engine must ship no talent of its own, and therefore no focus",
    );
}
