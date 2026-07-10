//! The `modload` CLI payload: `describe` summarizes a valid mod and errors on a
//! bad one (so the CLI exits non-zero for CI smoke checks).

use std::path::Path;

use stormlight_mod_abi::descriptors::Registration;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_modloader::report::describe;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

/// A guest module whose data segment holds `payload` and whose `mod_register`
/// returns the packed (offset, len).
fn guest_wasm(payload: &[u8]) -> Vec<u8> {
    let escaped: String = payload.iter().map(|b| format!("\\{b:02x}")).collect();
    let packed = (u64::from(DATA_OFFSET) << 32) | payload.len() as u64;
    let wat = format!(
        "(module (memory (export \"memory\") 1) \
         (data (i32.const {DATA_OFFSET}) \"{escaped}\") \
         (func (export \"mod_register\") (result i64) (i64.const {packed})))"
    );
    wat::parse_str(&wat).expect("fixture wat compiles")
}

fn write_folder_mod(dir: &Path) {
    let reg = Registration { abi: ABI_VERSION, ..Registration::default() };
    let payload = postcard::to_allocvec(&reg).unwrap();
    let manifest = format!(
        "id = \"guest\"\nname = \"Guest\"\nversion = \"0.4.0\"\n\
         kind = \"server\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
        ABI_VERSION.major
    );
    std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
    std::fs::write(dir.join(ENTRY), guest_wasm(&payload)).unwrap();
}

#[test]
fn describe_summarizes_a_valid_mod() {
    let dir = tempfile::tempdir().unwrap();
    write_folder_mod(dir.path());

    let summary = describe(dir.path()).expect("valid mod described");
    assert!(summary.contains("mod guest v0.4.0"), "summary missing id/version:\n{summary}");
    assert!(summary.contains("abilities: 0"), "summary missing descriptor counts:\n{summary}");
}

#[test]
fn describe_errors_on_a_missing_path() {
    assert!(describe(Path::new("/no/such/mod")).is_err());
}
