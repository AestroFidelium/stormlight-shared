//! A mod package is untrusted: loading arbitrary bytes as a `.zip` must never
//! panic. A corrupt archive, a missing `manifest.toml`, malformed TOML, or a
//! missing entry all surface as `Err`.

use bolero::check;
use stormlight_modloader::loader::load_zip_bytes;

#[test]
fn arbitrary_zip_bytes_never_panic() {
    check!().with_type::<Vec<u8>>().for_each(|bytes| {
        let _ = load_zip_bytes(bytes);
    });
}
