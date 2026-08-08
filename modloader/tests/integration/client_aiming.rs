//! The client learns **how each ability is aimed** (stormlight/server#59).
//!
//! The client has to build the right kind of `Aim` the instant a key is pressed:
//! a ground point for one ability, a direction for another, nothing at all for a
//! self-cast. It must never decide that locally — the mode is mod data, declared
//! on the gameplay ability descriptor, exactly like the cosmetic visuals it
//! already re-keys ([`super::client_id_bridge`]).
//!
//! So the same bridge carries it: the gameplay mods are run through the client
//! host purely to rebuild the name→global-id map, and each ability's declared
//! [`Targeting`] is recorded against the **global id** the wire uses. Invariants:
//! - a mode re-keys to the ability's interning position, cumulatively across
//!   several gameplay mods loaded in the server's order;
//! - a descriptor referencing a name-table entry that does not exist is a load
//!   error, never a panic and never a silently mis-keyed mode.

use std::io::{Cursor, Write};

use stormlight_mod_abi::abilities::{AbilityDescriptor, CastSpec, Params, Targeting};
use stormlight_mod_abi::common::{Affiliation, TargetFilter};
use stormlight_mod_abi::conditions::Condition;
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::AbilityId;
use stormlight_mod_abi::manifest::ABI_VERSION;
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

/// A bare ability with local handle `local`, declaring `mode`.
fn ability(local: u32, mode: Targeting) -> AbilityDescriptor {
    AbilityDescriptor {
        id: AbilityId(local),
        params: Params(Vec::new()),
        targeting: mode,
        cast: CastSpec::Instant,
        cost: Vec::new(),
        cast_gate: Condition::Always,
        on_cast_start: Vec::new(),
        on_cast: Vec::new(),
        tags: Vec::new(),
    }
}

/// A gameplay package whose abilities are named (and handled) in `abilities`
/// order, each declaring the paired mode.
fn gameplay(id: &str, abilities: &[(&str, Targeting)]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            abilities: abilities.iter().map(|(n, _)| (*n).into()).collect(),
            ..Names::default()
        },
        abilities: abilities
            .iter()
            .enumerate()
            .map(|(i, (_, m))| ability(i as u32, m.clone()))
            .collect(),
        ..Registration::default()
    };
    package(id, &postcard::to_allocvec(&reg).unwrap())
}

fn unit_mode() -> Targeting {
    Targeting::Unit { filter: TargetFilter::of(Affiliation::Enemies) }
}

#[test]
fn a_declared_mode_rekeys_to_the_abilitys_global_id() {
    // "bolt" is the SECOND ability interned, so its global id is 1, not 0.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "one",
        &[("ward", Targeting::SelfCast), ("bolt", Targeting::Vector)],
    ))
    .expect("gameplay names load");

    let modes: Vec<(AbilityId, Targeting)> =
        host.aiming_by_id().map(|(id, m)| (id, m.clone())).collect();
    assert_eq!(
        modes,
        vec![(AbilityId(0), Targeting::SelfCast), (AbilityId(1), Targeting::Vector)],
        "each mode must land on its ability's interned global id",
    );
}

#[test]
fn modes_accumulate_across_gameplay_mods_in_load_order() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay("one", &[("ward", Targeting::SelfCast)]))
        .expect("first gameplay mod loads");
    host.load_gameplay_zip_bytes(&gameplay(
        "two",
        &[("strike", unit_mode()), ("meteor", Targeting::Point)],
    ))
    .expect("second gameplay mod loads");

    let modes: Vec<(AbilityId, Targeting)> =
        host.aiming_by_id().map(|(id, m)| (id, m.clone())).collect();
    assert_eq!(
        modes,
        vec![
            (AbilityId(0), Targeting::SelfCast),
            (AbilityId(1), unit_mode()),
            (AbilityId(2), Targeting::Point),
        ],
        "the second mod's abilities must intern above the first's, as on the server",
    );
}

#[test]
fn a_descriptor_with_no_name_entry_is_an_error() {
    // Handle 3 with a one-entry name table: a dangling reference. Adoption must
    // report it rather than dropping the ability's mode (which would leave the
    // client sending the wrong aim shape forever).
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names { abilities: vec!["bolt".into()], ..Names::default() },
        abilities: vec![ability(3, Targeting::Point)],
        ..Registration::default()
    };
    let mut host = ClientHost::new().unwrap();
    let err = host
        .load_gameplay_zip_bytes(&package("broken", &postcard::to_allocvec(&reg).unwrap()))
        .expect_err("a dangling ability handle must not load");
    assert!(
        format!("{err:#}").contains("name-table"),
        "the error should name the dangling reference, got: {err:#}"
    );
}
