//! A status visual crosses the same name→global-id bridge every other cosmetic
//! does (stormlight/server#171).
//!
//! The wire names a buff by its mod-global id, and the cosmetic mod that dresses it
//! names it by the string the gameplay mod interned it under. So the client host
//! interns every gameplay mod's buff names in load order — exactly as server
//! adoption does — and re-keys each status visual onto the id that results.
//! Invariants:
//!   - **A dressed buff lands on the id at its interning position**, counted across
//!     every gameplay mod loaded before it;
//!   - **A buff no gameplay mod declares is dropped**, never given an id of its own.

use std::io::{Cursor, Write};

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::BuffId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::status_visual::{StatusLook, StatusVisual};
use stormlight_mod_abi::visuals::{ClientRegistration, PrimitiveShape, VisualModel};
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

/// The names the gameplay mods might declare, and one nobody does.
const NAMES: [&str; 5] = ["warded", "harried", "hastened", "bulwark", "nobody"];

fn look(tag: usize) -> StatusLook {
    StatusLook {
        own: None,
        others: Some(VisualModel::Primitive {
            shape: PrimitiveShape::Sphere,
            color: [tag as f32 / 10.0, 0.5, 0.5, 1.0],
        }),
        attach: None,
    }
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many of the first four names the first gameplay mod declares; the second
    /// declares the rest of them.
    #[generator(0u8..=4)]
    first: u8,
    /// Which names the cosmetic dresses, by index into [`NAMES`].
    #[generator(bolero::produce::<Vec<u8>>().with().len(1usize..=5))]
    dressed: Vec<u8>,
}

#[test]
fn a_status_visual_lands_on_its_buffs_global_id() {
    check!().with_type::<Scenario>().for_each(|s| {
        let split = usize::from(s.first);
        let mut host = ClientHost::new().unwrap();
        for (id, names) in [("first", &NAMES[..split]), ("second", &NAMES[split..4])] {
            let reg = Registration {
                abi: ABI_VERSION,
                names: Names {
                    buffs: names.iter().map(|n| (*n).to_string()).collect(),
                    ..Names::default()
                },
                ..Registration::default()
            };
            host.load_gameplay_zip_bytes(&package(
                id,
                "server",
                &postcard::to_allocvec(&reg).unwrap(),
            ))
            .expect("a gameplay mod loads");
        }

        // The cosmetic interns its own handles in its own order.
        let dressed: Vec<usize> = s.dressed.iter().map(|d| usize::from(*d) % NAMES.len()).collect();
        let cosmetic = ClientRegistration {
            abi: ABI_VERSION,
            names: Names {
                buffs: NAMES.iter().rev().map(|n| (*n).to_string()).collect(),
                ..Names::default()
            },
            status_visuals: dressed
                .iter()
                .map(|&i| StatusVisual {
                    buff: BuffId((NAMES.len() - 1 - i) as u16),
                    look: look(i),
                })
                .collect(),
            ..ClientRegistration::default()
        };
        host.load_zip_bytes(&package(
            "cosmetic",
            "client",
            &postcard::to_allocvec(&cosmetic).unwrap(),
        ))
        .expect("the cosmetic loads");

        let by_id: Vec<(BuffId, StatusLook)> =
            host.status_looks_by_id().map(|(id, look)| (id, look.clone())).collect();
        let mut expected: Vec<(BuffId, StatusLook)> =
            dressed.iter().filter(|&&i| i < 4).map(|&i| (BuffId(i as u16), look(i))).collect();
        expected.sort_by_key(|(id, _)| id.0);
        expected.dedup_by_key(|(id, _)| id.0);
        let mut got = by_id;
        got.sort_by_key(|(id, _)| id.0);
        assert_eq!(got, expected, "{s:?}");
    });
}
