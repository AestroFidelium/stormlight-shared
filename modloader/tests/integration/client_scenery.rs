//! A map's scenery through the real client host: every loaded cosmetic package's
//! pieces reach [`ClientHost::scenery`], **appended in load order** and never
//! merged — scenery has no key to collide on, and two packages that each dress
//! part of a map both get drawn — and a piece's `mod://` asset resolves within the
//! package that declared it.
//!
//! Hermetic `.wat` fixtures, like `client.rs`.

use std::io::{Cursor, Write};

use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::scenery::{SceneryPiece, SceneryPlacement};
use stormlight_mod_abi::visuals::{ClientRegistration, ModelClips};
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "client.wasm";
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

/// A client package `id` declaring `pieces` and shipping each piece's asset.
fn package(id: &str, pieces: &[SceneryPiece]) -> Vec<u8> {
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        scenery: pieces.to_vec(),
        ..ClientRegistration::default()
    };
    let wasm = guest_wasm(&postcard::to_allocvec(&reg).expect("encode"));
    let manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nkind = \"client\"\n\
         entry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
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
        for p in pieces {
            let path = p.asset.split_once(&format!("mod://{id}/")).expect("own package").1;
            zip.start_file(path, opts).unwrap();
            zip.write_all(p.asset.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

fn piece(id: &str, n: usize, live: &str) -> SceneryPiece {
    SceneryPiece {
        asset: format!("mod://{id}/scenery/{n}.glb"),
        clips: ModelClips { birth: String::new(), live: live.into() },
        placements: vec![
            SceneryPlacement::IDENTITY,
            SceneryPlacement { translation: [n as f32, 0.0, -2.0], ..SceneryPlacement::IDENTITY },
        ],
    }
}

#[test]
fn scenery_from_every_package_is_appended_in_load_order() {
    let ground = vec![piece("map_a", 0, ""), piece("map_a", 1, "")];
    let props = vec![piece("map_b", 0, "idle")];

    let mut host = ClientHost::new().unwrap();
    host.load_zip_bytes(&package("map_a", &ground)).expect("first package loads");
    host.load_zip_bytes(&package("map_b", &props)).expect("second package loads");

    let want: Vec<SceneryPiece> = ground.iter().chain(&props).cloned().collect();
    assert_eq!(host.scenery(), want.as_slice(), "pieces must arrive whole, in load order");

    for p in host.scenery() {
        let bytes = host.read_asset(&p.asset).expect("a piece's asset resolves");
        assert_eq!(bytes, p.asset.as_bytes(), "asset resolved into the wrong package");
    }
}

#[test]
fn a_package_with_an_unplaceable_piece_is_refused_whole() {
    let mut bad = piece("map_a", 0, "");
    bad.placements[1].rotation = [0.0; 4];
    let mut host = ClientHost::new().unwrap();
    assert!(host.load_zip_bytes(&package("map_a", &[bad])).is_err());
    assert!(host.scenery().is_empty(), "nothing from a refused package may be drawn");
}
