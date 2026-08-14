//! The talent half of the client-side name→global-id bridge (server#68).
//!
//! A chosen talent reaches the client as an opaque global id
//! ([`ReplicatedTalents`](stormlight_shared::talents::ReplicatedTalents)), and a
//! HUD binding to that list has to print *something a player reads*. The bridge
//! the unit and ability families already cross (server#44) is widened by one
//! family: the gameplay pass interns `names.talents` in load order, exactly as
//! server adoption does, so `id → declared name` comes back out.
//!
//! Invariants (hermetic `.wat` fixtures, the shape `client_id_bridge.rs` uses):
//! - a talent resolves to the name at its **interning position** across several
//!   gameplay mods loaded in order — the position the server assigned, not an
//!   accidental 0;
//! - an id no gameplay mod declared resolves to **nothing** rather than to
//!   whatever name sits at that raw index — a HUD prints no line rather than
//!   another talent's.

use std::io::{Cursor, Write};

use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::TalentId;
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

/// A gameplay `.zip` package whose `Names` intern `talents` in that order.
fn gameplay(id: &str, talents: &[&str]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names { talents: talents.iter().map(|s| (*s).into()).collect(), ..Names::default() },
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
fn a_talent_resolves_to_the_name_at_its_interning_position() {
    // Two mods in load order: the second mod's talents continue the first's index
    // space, which is what server adoption does and therefore what the wire's ids
    // mean. A per-mod restart would print the wrong talent for every id above 0.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay("first", &["swift", "sturdy"])).expect("first loads");
    host.load_gameplay_zip_bytes(&gameplay("second", &["keen"])).expect("second loads");

    let names: Vec<(TalentId, String)> =
        host.talent_names().map(|(id, name)| (id, name.to_string())).collect();
    assert_eq!(
        names,
        vec![
            (TalentId(0), "swift".to_string()),
            (TalentId(1), "sturdy".to_string()),
            (TalentId(2), "keen".to_string()),
        ],
        "talents must intern cumulatively in gameplay load order, like every other family",
    );
}

#[test]
fn an_undeclared_talent_id_resolves_to_nothing() {
    // The list a HUD binds to carries raw ids from the server. One the client has
    // no name for prints no line at all — never the name that happens to sit at
    // that index, which is what a bare `Vec` index would hand back.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay("first", &["swift"])).expect("first loads");

    assert_eq!(host.talent_name(TalentId(0)), Some("swift"), "a declared talent must resolve");
    assert_eq!(host.talent_name(TalentId(1)), None, "an id past the table must resolve to nothing");
    assert_eq!(
        host.talent_name(TalentId(9999)),
        None,
        "a far-out id must resolve to nothing rather than panicking",
    );
}

#[test]
fn with_no_gameplay_mod_no_talent_has_a_name() {
    // The content-free rule: names come from mods, so a client with none loaded
    // prints an empty talent list rather than inventing labels.
    let host = ClientHost::new().unwrap();
    assert_eq!(host.talent_names().count(), 0, "the engine must ship no talent names of its own");
}
