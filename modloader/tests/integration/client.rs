//! End-to-end cosmetic-mod host path: a client (`kind = "client"`) package whose
//! guest emits a `ClientRegistration` is loaded, run, and adopted by
//! [`ClientHost`]; its declared visual is retrievable by unit name and its
//! `mod://` asset resolves within the package root.
//!
//! Hermetic like `register.rs`: a hand-written `.wat` guest whose data segment
//! holds the postcard encoding of a known `ClientRegistration`, so the real host
//! bridge is exercised without a cross-repo wasm build.

use std::io::{Cursor, Write};

use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::UnitId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{ClientRegistration, VisualDescriptor, VisualModel};
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "client.wasm";
const MOD_ID: &str = "cosmetic";
const ASSET_PATH: &str = "art/hero.png";
const ASSET_BYTES: &[u8] = b"\x89PNG not-really-a-png-but-bytes";
const DATA_OFFSET: u32 = 1024;

/// A cosmetic registration: one unit `hero`, drawn from a `mod://` model asset.
fn sample() -> ClientRegistration {
    ClientRegistration {
        abi: ABI_VERSION,
        names: Names { units: vec!["hero".into()], ..Names::default() },
        visuals: vec![VisualDescriptor {
            unit: UnitId(0),
            model: VisualModel::Model {
                asset: format!("mod://{MOD_ID}/{ASSET_PATH}"),
                // Integer-valued so equality is exact after decode.
                scale: 2.0,
                yaw_offset: 0.0,
                launch: Some("Ref_Launch".into()),
            },
        }],
        ..ClientRegistration::default()
    }
}

/// A guest returning `packed(DATA_OFFSET, len)` for a data segment holding
/// `payload` — the same fixture shape `register.rs` uses.
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
        "id = \"{MOD_ID}\"\nname = \"Cosmetic\"\nversion = \"0.1.0\"\n\
         kind = \"client\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
        ABI_VERSION.major
    )
}

/// A `.zip` client package carrying the manifest, the guest wasm, and one asset.
fn zip_package(wasm: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest_toml().as_bytes()).unwrap();
        zip.start_file(ENTRY, opts).unwrap();
        zip.write_all(wasm).unwrap();
        zip.start_file(ASSET_PATH, opts).unwrap();
        zip.write_all(ASSET_BYTES).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn a_cosmetic_mod_loads_and_exposes_its_visual_and_asset() {
    let reg = sample();
    let payload = postcard::to_allocvec(&reg).expect("encode");
    let package = zip_package(&guest_wasm(&payload));

    let mut host = ClientHost::new().unwrap();
    host.load_zip_bytes(&package).expect("cosmetic package loads");

    // The declared visual is retrievable by the unit's stable name.
    match host.visual("hero") {
        Some(VisualModel::Model { asset, scale, .. }) => {
            assert_eq!(asset, &format!("mod://{MOD_ID}/{ASSET_PATH}"));
            assert_eq!(*scale, 2.0);
        }
        other => panic!("expected a Model visual for `hero`, got {other:?}"),
    }

    // And its `mod://` asset resolves to the bytes we packaged.
    let bytes = host.read_asset(&format!("mod://{MOD_ID}/{ASSET_PATH}")).expect("asset resolves");
    assert_eq!(bytes, ASSET_BYTES, "resolved asset bytes differ from the package");
}

#[test]
fn a_server_kind_package_is_rejected_by_the_client_host() {
    // Same payload, but a `kind = "server"` manifest: the client host must refuse
    // to adopt a gameplay mod as a cosmetic one.
    let reg = sample();
    let payload = postcard::to_allocvec(&reg).expect("encode");
    let wasm = guest_wasm(&payload);
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        let manifest = format!(
            "id = \"{MOD_ID}\"\nname = \"Cosmetic\"\nversion = \"0.1.0\"\n\
             kind = \"server\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
            ABI_VERSION.major
        );
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file(ENTRY, opts).unwrap();
        zip.write_all(&wasm).unwrap();
        zip.finish().unwrap();
    }
    let mut host = ClientHost::new().unwrap();
    assert!(host.load_zip_bytes(&buf).is_err(), "a server-kind package must not adopt as cosmetic");
}
