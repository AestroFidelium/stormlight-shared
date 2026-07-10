//! Loading a well-formed mod package yields its validated manifest and entry
//! wasm bytes — the same result whether the package is a folder or a `.zip`.
//! (Malformed-input robustness is fuzzed in `fuzz/loader.rs`.)

use std::io::{Cursor, Write};

use stormlight_mod_abi::manifest::{ABI_VERSION, ModKind};
use stormlight_modloader::loader::{load, load_zip_bytes};
use zip::write::SimpleFileOptions;

const ENTRY: &str = "stormlight_mod_test.wasm";

/// A canonical, valid manifest whose `abi` major matches the engine's.
fn manifest_toml() -> String {
    format!(
        "id = \"testmod\"\n\
         name = \"Test Mod\"\n\
         version = \"0.3.0\"\n\
         kind = \"server\"\n\
         entry = \"{ENTRY}\"\n\
         abi = \"{}.0.0\"\n",
        ABI_VERSION.major
    )
}

/// A minimal but real wasm module, so the entry bytes are genuine.
fn entry_wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (func (export "mod_register") (result i64) (i64.const 0)))"#).unwrap()
}

fn zip_package(manifest: &str, entry_name: &str, wasm: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file(entry_name, opts).unwrap();
        zip.write_all(wasm).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn a_valid_zip_package_loads_manifest_and_entry() {
    let wasm = entry_wasm();
    let package = zip_package(&manifest_toml(), ENTRY, &wasm);

    let loaded = load_zip_bytes(&package).expect("valid package loads");
    assert_eq!(loaded.manifest.id, "testmod");
    assert_eq!(loaded.manifest.kind, ModKind::Server);
    assert_eq!(loaded.manifest.entry, ENTRY);
    assert_eq!(loaded.manifest.abi.major, ABI_VERSION.major);
    assert_eq!(loaded.wasm, wasm, "entry bytes are read verbatim");
}

#[test]
fn a_valid_folder_package_loads_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let wasm = entry_wasm();
    std::fs::write(dir.path().join("manifest.toml"), manifest_toml()).unwrap();
    std::fs::write(dir.path().join(ENTRY), &wasm).unwrap();

    let loaded = load(dir.path()).expect("valid folder loads");
    assert_eq!(loaded.manifest.id, "testmod");
    assert_eq!(loaded.manifest.entry, ENTRY);
    assert_eq!(loaded.wasm, wasm);
}

#[test]
fn a_major_abi_mismatch_is_rejected() {
    // Same package, but the manifest declares an incompatible ABI major.
    let bad = manifest_toml().replace(
        &format!("abi = \"{}.0.0\"", ABI_VERSION.major),
        &format!("abi = \"{}.0.0\"", ABI_VERSION.major + 1),
    );
    let package = zip_package(&bad, ENTRY, &entry_wasm());
    assert!(load_zip_bytes(&package).is_err(), "incompatible ABI major must be rejected");
}
