# stormlight / shared

[![CI](https://github.com/AestroFidelium/stormlight-shared/actions/workflows/ci.yml/badge.svg)](https://github.com/AestroFidelium/stormlight-shared/actions/workflows/ci.yml)
![unsafe: forbidden](https://img.shields.io/badge/unsafe-forbidden-success)
![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

**A sandboxed WebAssembly host that lets untrusted mods define an entire game —
units, abilities, projectiles, status effects — as data, which an authoritative
Rust server then runs.**

Stormlight is a MOBA engine on [Bevy](https://bevyengine.org) 0.18 that ships
with no game content at all. Every hero, ability and map comes from a mod
compiled to `wasm32-unknown-unknown`. This repository holds the engine-side
code that both the server and the client depend on. The centrepiece is
`stormlight_modloader`, the host those mods run inside.

> This repository is a read-only mirror of a private upstream, where development
> and planning happen. Bug reports and feedback are welcome as
> [issues](https://github.com/AestroFidelium/stormlight-shared/issues); pull requests
> are disabled.

## What a mod looks like

Abridged from a real, complete mod
([example-mod](https://github.com/AestroFidelium/stormlight-example-mod)) that declares a unit
with a skillshot and a self-buff. It contains no engine code and no `unsafe`:

```rust
register_mod!(|ctx: &mut ModContext| {
    // Names, not numbers: reserved names collapse onto the engine's own ids.
    let cooldown = ctx.param("cooldown");
    let armor = ctx.stat("armor");
    let energy = ctx.resource("energy");
    let arcane = ctx.damage_type("arcane");
    let (hero, _) = ctx.register_tag_class("hero", "playable");

    let guarded = ctx.buff("guarded", guarded(armor));
    let spark = ctx.ability("spark", spark(cooldown, energy, arcane));
    let guard = ctx.ability("guard", guard(cooldown, energy, guarded));

    ctx.unit("sentinel", UnitDescriptor {
        tags: vec![hero],
        abilities: vec![(Slot(0), spark), (Slot(1), guard)],
        resources: vec![ResourcePool { id: energy, max: Value::Const(100.0), /* … */ }],
        /* health, stats, … */
    });
});

/// A skillshot: spend energy, throw a missile, damage the first enemy it touches.
fn spark(cooldown: ParamId, energy: ResourceId, arcane: DamageTypeId) -> AbilityDescriptor {
    AbilityDescriptor {
        targeting: Targeting::Vector,
        cast: CastSpec::Cast { time: Value::Const(0.2), movable: true },
        cost: vec![Cost::Resource { res: energy, amount: Value::Const(25.0) }],
        params: Params(vec![(cooldown, Value::Const(3.0))]),
        on_cast: vec![Impact::Spawn {
            body: BodyDescriptor {
                kind: BodyKind::Missile { speed: Value::Const(18.0), /* range, … */ },
                // The payload travels with the body and resolves on contact.
                on_hit: vec![Impact::Damage { amount: Value::Const(90.0), dtype: arcane, /* … */ }],
                /* collides with enemies only */
            },
            /* … */
        }],
        /* … */
    }
}
```

Build it, then ask the real host whether it would accept it:

```console
$ tools/package.sh                                    # in example-mod
$ cargo run -p stormlight_modloader --bin modload -- ../example-mod/dist/example
mod example v0.1.0 (Server)
  name:      Example
  abi:       0.1.0
  units:     1
  abilities: 2
  talents:   0
  buffs:     1
  curves:    0
  tags:      1 (1 class links)
```

## Why this is harder than it looks

**1. The ABI boundary has no imports.** A guest imports nothing, and there is
no WASI. Everything crosses through the guest's own exports. For registration,
`mod_register` serialises its content with postcard, leaks the buffer, and
returns a packed `(ptr, len)` as one `u64`. The host bounds-checks that region
against linear memory before it reads a byte. For runtime calls (`mod_handle`,
`mod_tick`, `mod_trigger`), the host first asks the guest for memory through
`mod_alloc`, writes the context into it, and decodes the effects that come back
the same way. Every decode is total: an out-of-range pointer or malformed bytes
give an `Err`, never a panic. A fuzz target feeds arbitrary bytes to the host to
prove it
([`sandbox.rs`](modloader/tests/fuzz/sandbox.rs)).

**2. The guest is `no_std`, and the author never sees that.**
`wasm32-unknown-unknown` has no allocator and no panic machinery. The guest SDK
([sdk](https://github.com/AestroFidelium/stormlight-sdk)) installs `dlmalloc` as the global
allocator and a panic handler that lowers to a wasm `unreachable`. The
`register_mod!` macro generates all five exports. A mod author writes one
closure. The same macro also exposes the registration builder on the host
target, so a mod's content is unit-tested natively before it is ever compiled
to wasm.

**3. A mod is untrusted input.** Every call gets a fresh `Store` with:
- a **fuel** budget. It is deterministic and needs no watchdog thread; an
  infinite loop traps instead of hanging a server tick;
- a hard **memory cap** (64 MiB by default) enforced by wasmtime's `StoreLimits`. An
  oversized initial memory or a runaway `memory.grow` is refused;
- NaN canonicalisation and no threads, so every host computes the same result.

Mod assets resolve through a `mod://<id>/<path>` virtual filesystem. It rejects
`..`, absolute paths and backslashes before touching the disk, so a mod cannot
read outside its own package.

**4. A panicking mod is an error, not a crash.** The panic handler turns a guest
panic into a trap, and the trap reaches the host as an `Err` from the call that
caused it. The host process never unwinds on a mod's behalf. Because guests hold
**no state between calls** (each entry re-runs the mod's builder in a fresh
store), there is no half-updated guest state to recover afterwards.

## Rigor

- **`unsafe_code = "forbid"`** in every crate here. The only `unsafe` in the
  whole mod pipeline is one slice view in the guest SDK's FFI shim, plus the
  `#[unsafe(no_mangle)]` attributes on the generated exports. That crate uses
  `deny` with each site explicitly allowed, so every one is auditable.
- **Property-based and fuzz testing** with
  [bolero](https://github.com/camshaft/bolero): 198 tests in this workspace
  (69 of them in the mod host) and 313 in the SDK. Most assert invariants over
  generated inputs rather than hand-picked values.
- Tests are split by kind into separate binaries (`property/`, `fuzz/`,
  `integration/`, and `systems/` for headless Bevy schedules), with one small
  file per feature.
- CI runs `fmt`, `clippy -D warnings` and the full suite on a pinned nightly.

## Crates

| Crate | Path | What it is |
| --- | --- | --- |
| `stormlight_modloader` | `modloader/` | The wasmtime host: loading (folder or `.zip`), sandboxing, the registration and runtime bridges, the `mod://` VFS, and the `modload` CLI. |
| `stormlight_shared` | `protocol/` | The client/server wire protocol on [Lightyear](https://github.com/cBournhonesque/lightyear) 0.26, plus compact codecs for replicated state. |
| `stormlight_navigation` | `navigation/` | The walkable map geometry the server and the predicting client must agree on. |

The mod ABI itself (`stormlight_mod_abi`) lives in the
[sdk](https://github.com/AestroFidelium/stormlight-sdk) repository. It is a git dependency, so
this repository builds on its own.

## Limitations

- **The ABI is unstable (0.x).** The host checks only the major version, and
  the schema still grows with every engine milestone. A guest built against an
  older SDK may fail to decode.
- **The server and client are not public yet.** You can build, load and validate
  mods with `modload`, but you cannot play one from these repositories alone.
- **One sync host, no async.** Calls are synchronous and each one instantiates
  a fresh store. That keeps guests stateless and isolated, but it spends
  instantiation time on every call.
- **Registration buffers are leaked by design.** It is safe only because each
  store is dropped whole after the call.
- **Requires nightly Rust** for the workspace (`rust-toolchain.toml`). Mods
  themselves build on stable.

## Building

```console
$ cargo nextest run --workspace
$ cargo run -p stormlight_modloader --bin modload -- <mod folder or .zip>
```

A `flake.nix` provides the full dev shell (`direnv allow`). Linking uses clang +
mold (`.cargo/config.toml`).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.
