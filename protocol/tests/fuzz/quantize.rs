//! Fuzz the decode side of [`stormlight_shared::quantize`] against arbitrary
//! wire bytes (`stormlight/server#10`). A hostile or corrupt peer can hand the
//! decoder any 22-byte frame; it must never panic and must always yield a
//! finite, non-NaN `Transform` (a NaN slipping into the ECS would poison every
//! downstream system). The smallest-three reconstruction is the risky part —
//! `sqrt` of a possibly-negative remainder — so this pins that it is clamped.
//!
//!   cargo bolero test decode_never_panics_on_arbitrary_bytes --time 600

use bolero::{TypeGenerator, check};
use stormlight_shared::quantize::{QUANTIZED_LEN, decode};

#[derive(Debug, TypeGenerator)]
struct Frame([u8; QUANTIZED_LEN]);

#[test]
fn decode_never_panics_on_arbitrary_bytes() {
    check!().with_type::<Frame>().for_each(|Frame(bytes)| {
        let t = decode(bytes);
        assert!(
            t.translation.is_finite() && t.scale.is_finite(),
            "decoded translation/scale must be finite for any input frame"
        );
        assert!(
            t.rotation.length().is_finite() && !t.rotation.length().is_nan(),
            "decoded rotation must be finite (no NaN from the smallest-three sqrt)"
        );
    });
}
