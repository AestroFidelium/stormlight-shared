//! End-to-end host bridge (S7): a guest's `mod_register` returns a packed
//! `(ptr, len)` into its linear memory; the host reads that region and decodes
//! the `Registration`. The whole load → instantiate → register → decode path
//! must reproduce exactly what the guest emitted.
//!
//! The guest here is a hand-written `.wat` whose data segment holds the postcard
//! encoding of a known `Registration` — a hermetic fixture that exercises the
//! real host path without a cross-repo wasm build. (Decode robustness over
//! arbitrary bytes is fuzzed in `fuzz/registry.rs`.)

use std::io::{Cursor, Write};

use stormlight_mod_abi::abilities::{AbilityDescriptor, CastSpec, Params, Targeting};
use stormlight_mod_abi::conditions::Condition;
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{AbilityId, TagClassId, TagId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_modloader::host::Host;
use stormlight_modloader::loader::load_zip_bytes;
use stormlight_modloader::registry::ModRegistry;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

/// A small, representative registration (no floats, so value-equality is exact).
fn sample_registration() -> Registration {
    Registration {
        abi: ABI_VERSION,
        names: Names {
            stats: vec!["health".into(), "mana".into()],
            abilities: vec!["strike".into()],
            tags: vec!["rooted".into()],
            tag_classes: vec!["blocks_move".into()],
            ..Names::default()
        },
        abilities: vec![AbilityDescriptor {
            id: AbilityId(0),
            params: Params(Vec::new()),
            targeting: Targeting::NoTarget,
            cast: CastSpec::Instant,
            cost: Vec::new(),
            cast_gate: Condition::Always,
            on_cast_start: Vec::new(),
            on_cast: Vec::new(),
            tags: Vec::new(),
        }],
        talents: Vec::new(),
        buffs: Vec::new(),
        tag_classes: vec![(TagId(0), TagClassId(0))],
        curves: Vec::new(),
        units: Vec::new(),
    }
}

/// A guest module that returns `packed(DATA_OFFSET, len)` for a data segment
/// holding `payload`.
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

fn manifest_toml() -> String {
    format!(
        "id = \"guest\"\nname = \"Guest\"\nversion = \"0.1.0\"\n\
         kind = \"server\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
        ABI_VERSION.major
    )
}

fn zip_package(wasm: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest_toml().as_bytes()).unwrap();
        zip.start_file(ENTRY, opts).unwrap();
        zip.write_all(wasm).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn a_guests_registration_round_trips_through_the_host() {
    let reg = sample_registration();
    let payload = postcard::to_allocvec(&reg).expect("encode");
    let package = zip_package(&guest_wasm(&payload));

    // Load (S6) → instantiate + register + decode (S7).
    let loaded = load_zip_bytes(&package).expect("package loads");
    let host = Host::new().unwrap();
    let decoded = host.register(&loaded.wasm).expect("mod_register decodes");
    assert_eq!(decoded, reg, "decoded registration differs from what the guest emitted");

    // Adopt into the host-side registry, keyed by mod id.
    let mut registry = ModRegistry::new();
    registry.insert(loaded.manifest.clone(), decoded).unwrap();
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.get("guest").unwrap().registration, reg);
}

#[test]
fn an_out_of_bounds_registration_region_is_rejected() {
    // ptr 0, len 100000 > one 64 KiB page: the host must refuse, not panic.
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "mod_register") (result i64) (i64.const 100000)))"#;
    let wasm = wat::parse_str(wat).unwrap();
    let host = Host::new().unwrap();
    assert!(host.register(&wasm).is_err(), "out-of-bounds region must be rejected");
}
