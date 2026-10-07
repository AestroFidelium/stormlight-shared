//! The client learns **which resource each ability costs** (stormlight/server#210).
//!
//! An interface can bind "the resource the ability in slot N costs" — the pool a
//! refund written once for every unit lands in. The client resolves it the way it
//! resolves an aim mode: from the gameplay mods it loads, re-keyed to global ids.
//! Laws, over two gameplay mods naming their resources in different orders:
//!   - two abilities paying the same-named resource map to one global resource;
//!   - differently named resources stay apart;
//!   - an ability with no resource cost has no entry.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use stormlight_mod_abi::abilities::{AbilityDescriptor, CastSpec, Cost, Params, Targeting};
use stormlight_mod_abi::conditions::Condition;
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{AbilityId, ResourceId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::math::Value;
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

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

/// A gameplay `.zip` package carrying the manifest + guest wasm.
fn package(id: &str, payload: &[u8]) -> Vec<u8> {
    let wasm = guest_wasm(payload);
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

/// An ability with local handle `local`, costing local resource `pays` if any.
fn ability(local: u32, pays: Option<u16>) -> AbilityDescriptor {
    AbilityDescriptor {
        id: AbilityId(local),
        params: Params(Vec::new()),
        targeting: Targeting::NoTarget,
        cast: CastSpec::Instant,
        cost: pays
            .map(|res| Cost::Resource { res: ResourceId(res), amount: Value::Const(10.0) })
            .into_iter()
            .collect(),
        cast_gate: Condition::Always,
        on_cast_start: Vec::new(),
        on_cast: Vec::new(),
        tags: Vec::new(),
    }
}

/// A gameplay package naming `resources` and `abilities`, each ability paying the
/// local resource at the paired index, if any.
fn gameplay(id: &str, resources: &[&str], abilities: &[(&str, Option<u16>)]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            abilities: abilities.iter().map(|(n, _)| (*n).into()).collect(),
            resources: resources.iter().map(|n| (*n).into()).collect(),
            ..Names::default()
        },
        abilities: abilities.iter().zip(0u32..).map(|((_, pays), i)| ability(i, *pays)).collect(),
        ..Registration::default()
    };
    package(id, &postcard::to_allocvec(&reg).unwrap())
}

#[test]
fn each_ability_maps_to_the_global_resource_it_costs() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "one",
        &["energy"],
        &[("bolt", Some(0)), ("dash", None)],
    ))
    .expect("first gameplay mod loads");
    host.load_gameplay_zip_bytes(&gameplay(
        "two",
        &["mana", "energy"],
        &[("nova", Some(0)), ("lance", Some(1))],
    ))
    .expect("second gameplay mod loads");

    let costs: BTreeMap<AbilityId, ResourceId> = host.cost_resources_by_id().collect();
    // Global ability ids follow interning order: bolt 0, dash 1, nova 2, lance 3.
    let (bolt, dash, nova, lance) = (AbilityId(0), AbilityId(1), AbilityId(2), AbilityId(3));
    assert!(!costs.contains_key(&dash), "an ability with no resource cost has an entry");
    assert_eq!(costs.get(&bolt), costs.get(&lance), "two energy costs map apart");
    assert_ne!(costs.get(&bolt), costs.get(&nova), "energy and mana map together");
    assert!(costs.contains_key(&bolt) && costs.contains_key(&nova));
}
