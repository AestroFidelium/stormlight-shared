//! Human-readable summary of a loaded mod — the payload behind the `modload`
//! CLI. Kept in the library (not the binary) so it is testable directly.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::host::Host;
use crate::loader::load;

/// Load, instantiate, and register the mod at `path`, returning a summary of its
/// manifest and the descriptor counts it registered. Errors (bad path, invalid
/// manifest, ABI mismatch, a hostile guest) propagate so the CLI can exit
/// non-zero.
pub fn describe(path: &Path) -> Result<String> {
    let loaded = load(path)?;
    let host = Host::new()?;
    let reg = host.register(&loaded.wasm)?;
    let m = &loaded.manifest;

    let mut out = String::new();
    let _ = writeln!(out, "mod {} v{} ({:?})", m.id, m.version, m.kind);
    let _ = writeln!(out, "  name:      {}", m.name);
    let _ = writeln!(out, "  abi:       {}", reg.abi);
    let _ = writeln!(out, "  abilities: {}", reg.abilities.len());
    let _ = writeln!(out, "  talents:   {}", reg.talents.len());
    let _ = writeln!(out, "  buffs:     {}", reg.buffs.len());
    let _ = writeln!(out, "  curves:    {}", reg.curves.len());
    let _ = writeln!(out, "  tags:      {} ({} class links)", reg.names.tags.len(), reg.tag_classes.len());
    Ok(out)
}
