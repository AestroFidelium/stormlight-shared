//! Every cosmetic mod's marks under units reach the client in precedence order
//! (stormlight/server#181).
//!
//! A mark is not keyed by anything a mod owns, so two mods' marks never collide:
//! they join one list. Invariant: **the host keeps them in load order, and each
//! mod's in its own declaration order** — the first that fits a unit is the one it
//! wears, so a mod loaded earlier speaks first, as a mark declared earlier does.

use std::io::{Cursor, Write};

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::decal::DecalBlend;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::unit_mark::{MarkLook, MarkRole, UnitMark};
use stormlight_mod_abi::visuals::ClientRegistration;
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

/// How many marks one cosmetic mod declares.
#[derive(Debug, Clone, Copy, TypeGenerator)]
struct Count(#[generator(0u8..=3)] u8);

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// One entry per loaded cosmetic mod, in load order.
    #[generator(bolero::produce::<Vec<Count>>().with().len(1usize..=3))]
    per_mod: Vec<Count>,
}

fn mark(loaded: usize, i: u8) -> UnitMark {
    UnitMark {
        role: MarkRole::Hovered,
        relation: None,
        look: MarkLook {
            asset: format!("mod://cosmetic{loaded}/mark{i}.png"),
            tint: [1.0; 4],
            blend: DecalBlend::Blend,
            scale: 1.0,
            turns: false,
            yaw_offset: 0.0,
        },
    }
}

#[test]
fn every_mods_marks_join_one_list_in_load_order() {
    check!().with_type::<Scenario>().for_each(|s| {
        let mut host = ClientHost::new().expect("a host starts");
        let mut expected = Vec::new();
        for (loaded, &Count(count)) in s.per_mod.iter().enumerate() {
            let marks: Vec<UnitMark> = (0..count).map(|i| mark(loaded, i)).collect();
            expected.extend(marks.iter().cloned());
            let cosmetic = ClientRegistration {
                abi: ABI_VERSION,
                unit_marks: marks,
                ..ClientRegistration::default()
            };
            host.load_zip_bytes(&package(
                &format!("cosmetic{loaded}"),
                "client",
                &postcard::to_allocvec(&cosmetic).unwrap(),
            ))
            .expect("the cosmetic loads");
        }
        assert_eq!(host.unit_marks(), expected.as_slice(), "{s:?}");
    });
}
