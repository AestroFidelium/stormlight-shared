//! The client-side name→global-id bridge (server#44): a cosmetic mod dresses
//! units/abilities **by name**, but the wire carries the gameplay mod's **global
//! id** (a unit's `UnitTag`, an ability's `vfx`). [`ClientHost::load_gameplay`]
//! rebuilds the same name→id map server adoption builds — interning each gameplay
//! mod's `Names` in load order — so [`ClientHost::visuals_by_id`] /
//! [`effects_by_id`] re-key the cosmetic's visuals onto those global ids.
//!
//! Invariants (hermetic `.wat` fixtures, like `client.rs`):
//! - a cosmetic visual/effect re-keys to the id at its **interning position** in
//!   the gameplay mod (a deliberately non-zero index pins "same as adoption", not
//!   an accidental 0);
//! - a visual whose unit no loaded gameplay mod defines has no id and is dropped
//!   (the client falls back to its placeholder), never a panic.

use std::io::{Cursor, Write};

use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{AbilityId, UnitId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, EffectRole, EffectVisualDescriptor, PrimitiveShape, VisualDescriptor,
    VisualModel,
};
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

/// A distinct primitive so equality pins which visual came back.
fn model(tag: f32) -> VisualModel {
    VisualModel::Primitive { shape: PrimitiveShape::Cube, color: [tag, 0.2, 0.4, 1.0] }
}

/// A guest returning `packed(DATA_OFFSET, len)` for a data segment holding
/// `payload` — the shared fixture shape.
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

/// A `.zip` package of `kind` carrying the manifest + guest wasm.
fn package(id: &str, kind: &str, payload: &[u8]) -> Vec<u8> {
    let wasm = guest_wasm(payload);
    let manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\n\
         kind = \"{kind}\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
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

/// A gameplay package whose `Names` intern `units` and `abilities` in that order.
fn gameplay(units: &[&str], abilities: &[&str]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            units: units.iter().map(|s| (*s).into()).collect(),
            abilities: abilities.iter().map(|s| (*s).into()).collect(),
            ..Names::default()
        },
        ..Registration::default()
    };
    package("gameplay", "server", &postcard::to_allocvec(&reg).unwrap())
}

/// A cosmetic package dressing unit `unit_name` (local handle 0) and ability
/// `ability_name` (local handle 0, Projectile role) with distinct models.
fn cosmetic(unit_name: &str, ability_name: &str) -> Vec<u8> {
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names {
            units: vec![unit_name.into()],
            abilities: vec![ability_name.into()],
            ..Names::default()
        },
        visuals: vec![VisualDescriptor { unit: UnitId(0), model: model(0.1) }],
        effects: vec![EffectVisualDescriptor {
            ability: AbilityId(0),
            role: EffectRole::Projectile,
            model: model(0.9),
        }],
    };
    package("cosmetic", "client", &postcard::to_allocvec(&reg).unwrap())
}

#[test]
fn a_cosmetic_visual_rekeys_to_the_gameplay_units_interning_position() {
    // "skirmisher" is the SECOND unit and "bolt" the SECOND ability the gameplay
    // mod interns, so their global ids are 1 — not 0. Re-keying must reproduce it.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["grunt", "skirmisher"], &["dash", "bolt"]))
        .expect("gameplay names load");
    host.load_zip_bytes(&cosmetic("skirmisher", "bolt")).expect("cosmetic loads");

    let units: Vec<(UnitId, VisualModel)> =
        host.visuals_by_id().map(|(id, m)| (id, m.clone())).collect();
    assert_eq!(
        units,
        vec![(UnitId(1), model(0.1))],
        "the unit visual must re-key to the gameplay unit's interned global id",
    );

    let effects: Vec<((AbilityId, EffectRole), VisualModel)> =
        host.effects_by_id().map(|(k, m)| (k, m.clone())).collect();
    assert_eq!(
        effects,
        vec![((AbilityId(1), EffectRole::Projectile), model(0.9))],
        "the effect visual must re-key to the gameplay ability's interned global id",
    );
}

#[test]
fn a_visual_for_an_unknown_unit_is_dropped_not_mapped() {
    // The gameplay mod defines no "ghost"; the cosmetic dressing it has no id, so
    // it is left out of the by-id view (client falls back to the placeholder).
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["skirmisher"], &["bolt"])).expect("gameplay loads");
    host.load_zip_bytes(&cosmetic("ghost", "phantom")).expect("cosmetic loads");

    assert_eq!(host.visuals_by_id().count(), 0, "an un-mapped unit visual must be dropped");
    assert_eq!(host.effects_by_id().count(), 0, "an un-mapped effect visual must be dropped");
}
