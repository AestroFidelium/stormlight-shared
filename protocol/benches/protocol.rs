//! Protocol encode/decode hot-path benches (divan). Add a `#[divan::bench]`
//! per measured function; run with `cargo bench -p stormlight_api`.
fn main() {
    divan::main();
}
