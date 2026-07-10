//! Decoding a registration out of guest memory is untrusted-input handling: any
//! memory snapshot and any packed `(ptr, len)` a guest returns must yield an
//! `Err` at worst, never a panic (no out-of-bounds slice, no decode unwind).

use bolero::check;
use stormlight_modloader::registry::decode_registration;

#[test]
fn decoding_arbitrary_memory_never_panics() {
    check!().with_type::<(Vec<u8>, u64)>().for_each(|(memory, packed)| {
        let _ = decode_registration(memory, *packed);
    });
}
