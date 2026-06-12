//! `modload` — standalone CLI to load + validate a mod (folder or `.zip`)
//! against the host ABI, without spinning up the engine. Handy for CI smoke
//! checks of third-party mods.

fn main() -> anyhow::Result<()> {
    // M6: parse argv[1] as a mod path, instantiate it in the host, print its
    // registered descriptors.
    Ok(())
}
