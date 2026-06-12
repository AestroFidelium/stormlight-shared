# stormlight / shared

Code shared by the Stormlight server and client. **No game content, no assets.**

- `protocol/` → `stormlight_shared`: wire protocol (`connection.rs`), Lightyear
  registration (`protocol.rs`), balance (`balance.rs`), declarative ECS macros
  (`support.rs`). Re-exported via `stormlight_shared::prelude`.
- `modloader/` → `stormlight_modloader`: the engine-side wasmtime host that loads
  mods and runs their entry points. Used by both server and client.

A Cargo workspace. Cross-repo dep: `stormlight_mod_abi` from the sibling `sdk`
repo. See the umbrella `../CLAUDE.md` for the full standard.
