//! The client-side cosmetic host — the `client.wasm` counterpart to [`crate::host`].
//!
//! A `*_client` mod (`kind = "client"`) is loaded here rather than on the server.
//! Its guest emits a [`ClientRegistration`] of [`VisualModel`]s keyed by unit; the
//! host runs that registration, [adopts](AdoptedVisuals::adopt) the visuals into a
//! unit-name → model table the content-free client keys on, and resolves the
//! mod's `mod://` assets through the shared [`Vfs`] (which rejects path traversal).
//!
//! The engine still ships nothing: [`ClientHost`] only holds whatever a cosmetic
//! mod declared. A unit with no declared visual simply has no entry — the client
//! falls back to its placeholder at the call site rather than failing here.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use stormlight_mod_abi::abilities::Targeting;
use stormlight_mod_abi::ids::{AbilityId, Handle, UnitId};
use stormlight_mod_abi::interner::Interner;
use stormlight_mod_abi::manifest::{ABI_VERSION, ModKind};
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_mod_abi::visuals::{ClientRegistration, EffectRole, VisualModel};

use crate::host::Host;
use crate::loader::{self, LoadedMod};
use crate::vfs::{ModSource, Vfs};

/// One cosmetic mod's visuals after adoption, keyed by unit **name** — the stable
/// string the gameplay and cosmetic sides share. Keying by name (not a local
/// handle) is what lets a separately-authored `*_client` mod dress a gameplay
/// mod's units.
#[derive(Clone, Debug, Default)]
pub struct AdoptedVisuals {
    by_name: BTreeMap<String, VisualModel>,
    by_effect: BTreeMap<(String, EffectRole), VisualModel>,
}

impl AdoptedVisuals {
    /// Build the visual tables from a decoded [`ClientRegistration`]: units keyed
    /// by unit name, ability feedback keyed by `(ability name, role)`.
    ///
    /// Total over hostile input: a major ABI mismatch, a unit visual referencing a
    /// unit handle with no [`names.units`](stormlight_mod_abi::descriptors::Names)
    /// entry, or an effect visual referencing an ability handle with no
    /// `names.abilities` entry (a dangling reference), is an `Err` — never a panic.
    /// When two entries name the same key the later one wins (deterministic,
    /// insertion order).
    pub fn adopt(reg: &ClientRegistration) -> Result<Self> {
        if reg.abi.major != ABI_VERSION.major {
            bail!(
                "cosmetic registration abi major {} != engine {}",
                reg.abi.major,
                ABI_VERSION.major
            );
        }
        let mut by_name = BTreeMap::new();
        for v in &reg.visuals {
            let name = reg.names.units.get(v.unit.0 as usize).ok_or_else(|| {
                anyhow!("visual references unit handle {} with no name-table entry", v.unit.0)
            })?;
            by_name.insert(name.clone(), v.model.clone());
        }
        let mut by_effect = BTreeMap::new();
        for e in &reg.effects {
            let name = reg.names.abilities.get(e.ability.0 as usize).ok_or_else(|| {
                anyhow!(
                    "effect visual references ability handle {} with no name-table entry",
                    e.ability.0
                )
            })?;
            by_effect.insert((name.clone(), e.role), e.model.clone());
        }
        Ok(Self { by_name, by_effect })
    }

    /// The visual declared for the unit named `unit_name`, if any.
    #[must_use]
    pub fn get(&self, unit_name: &str) -> Option<&VisualModel> {
        self.by_name.get(unit_name)
    }

    /// The feedback visual declared for `ability_name` in `role`, if any.
    #[must_use]
    pub fn effect(&self, ability_name: &str, role: EffectRole) -> Option<&VisualModel> {
        self.by_effect.get(&(ability_name.to_string(), role))
    }

    /// Number of units this mod dresses.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether this mod declares no visuals at all (neither units nor effects).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty() && self.by_effect.is_empty()
    }

    /// Iterate the `(unit name, visual)` pairs, in unit-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &VisualModel)> {
        self.by_name.iter()
    }

    /// Iterate the `((ability name, role), visual)` pairs, in key order.
    pub fn effects(&self) -> impl Iterator<Item = (&(String, EffectRole), &VisualModel)> {
        self.by_effect.iter()
    }
}

/// The client-side host: loads cosmetic mods, runs each guest's registration,
/// merges the adopted visuals into one unit-name → model table, and resolves
/// every loaded mod's `mod://` assets.
pub struct ClientHost {
    host: Host,
    vfs: Vfs,
    visuals: BTreeMap<String, VisualModel>,
    effects: BTreeMap<(String, EffectRole), VisualModel>,
    /// The gameplay-side name→global-id maps, rebuilt from each loaded gameplay
    /// mod's `Names` in load order — the same interning server adoption does, so
    /// a cosmetic's name resolves to the id the wire carries (`UnitTag` / `vfx`).
    /// `UnitId`/`AbilityId` have no reserved names, so cumulative interning from 0
    /// reproduces the server's ids exactly.
    unit_ids: Interner<UnitId>,
    ability_ids: Interner<AbilityId>,
    /// How each gameplay ability is aimed, keyed by the **global `AbilityId`**
    /// (server#59). The client cannot decide an aim mode locally — it is mod data
    /// like everything else — so the same gameplay pass that rebuilds the id map
    /// records the declared [`Targeting`] alongside it, and the client resolves a
    /// keypress into an `Aim` of exactly that shape.
    aiming: BTreeMap<AbilityId, Targeting>,
    /// Map geometry declared by the loaded gameplay mods, in load order. The
    /// client bakes this into the same walkable region the server routes over, so
    /// it can draw the map and agree with the server about where a unit may stand
    /// (server#79). Only the geometry is kept — the descriptor's own id is local
    /// to its mod and nothing client-side names a mesh by id.
    navmeshes: Vec<NavMeshDescriptor>,
}

impl ClientHost {
    /// A fresh host with no mods loaded.
    pub fn new() -> Result<Self> {
        Ok(Self {
            host: Host::new()?,
            vfs: Vfs::new(),
            visuals: BTreeMap::new(),
            effects: BTreeMap::new(),
            unit_ids: Interner::new(),
            ability_ids: Interner::new(),
            aiming: BTreeMap::new(),
            navmeshes: Vec::new(),
        })
    }

    /// Load a gameplay (server) mod package purely to learn its name→global-id
    /// mapping — run its registration and intern the unit/ability names it
    /// declared, in load order. Call this for each gameplay mod in the **same
    /// order the server loads them**, so the reconstructed ids line up with the
    /// wire. A cosmetic-kind mod is rejected. Does not touch the visual tables.
    pub fn load_gameplay(&mut self, path: &Path) -> Result<()> {
        self.adopt_gameplay(loader::load(path)?)
    }

    /// Rebuild the name→id map from the bytes of a gameplay `.zip` package.
    /// Byte-oriented so the id bridge is exercised hermetically without touching
    /// the filesystem; the counterpart to [`load_gameplay`](Self::load_gameplay).
    pub fn load_gameplay_zip_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.adopt_gameplay(loader::load_zip_bytes(bytes)?)
    }

    /// Intern one gameplay mod's unit/ability names in load order (rejecting a
    /// cosmetic-kind package), reproducing the ids server adoption assigns, and
    /// record each ability's declared aim mode against the id it landed on.
    fn adopt_gameplay(&mut self, loaded: LoadedMod) -> Result<()> {
        if loaded.manifest.kind != ModKind::Server {
            bail!("mod `{}` is not a gameplay (server) mod", loaded.manifest.id);
        }
        let reg = self.host.register(&loaded.wasm)?;
        for name in &reg.names.units {
            self.unit_ids.intern(name);
        }
        for name in &reg.names.abilities {
            self.ability_ids.intern(name);
        }
        // Aim modes, re-keyed local → global through the names just interned. A
        // descriptor pointing at a missing name entry is a broken registration:
        // report it rather than dropping the mode, which would leave the client
        // sending the wrong aim shape for that ability forever.
        for ability in &reg.abilities {
            let name = reg.names.abilities.get(ability.id.raw() as usize).ok_or_else(|| {
                anyhow!("ability descriptor {} has no name-table entry", ability.id.raw())
            })?;
            let global = self
                .ability_ids
                .get(name)
                .ok_or_else(|| anyhow!("ability `{name}` was not interned"))?;
            self.aiming.insert(global, ability.targeting.clone());
        }
        self.navmeshes.extend(reg.navmeshes.iter().cloned());
        Ok(())
    }

    /// Every gameplay ability's declared aim mode, keyed by the **global
    /// `AbilityId`** the wire carries — how the client learns what kind of `Aim`
    /// to build when a slot's key is pressed.
    pub fn aiming_by_id(&self) -> impl Iterator<Item = (AbilityId, &Targeting)> {
        self.aiming.iter().map(|(id, mode)| (*id, mode))
    }

    /// The map geometry the loaded gameplay mods declared, in load order. Empty
    /// when none of them ships a map — the client then draws no ground and falls
    /// back to unclamped movement, exactly as it behaved before server#79.
    pub fn navmeshes(&self) -> &[NavMeshDescriptor] {
        &self.navmeshes
    }

    /// Every adopted unit visual keyed by the **global `UnitId`** the wire uses,
    /// resolved through the name→id map [`load_gameplay`](Self::load_gameplay)
    /// built. A visual for a unit no loaded gameplay mod defines (no id) is
    /// skipped — the client falls back to its placeholder for it.
    pub fn visuals_by_id(&self) -> impl Iterator<Item = (UnitId, &VisualModel)> {
        self.visuals.iter().filter_map(|(name, model)| Some((self.unit_ids.get(name)?, model)))
    }

    /// Every adopted ability-feedback visual keyed by the **global `AbilityId`**
    /// (the `vfx` key) plus its role. An effect for an ability no loaded gameplay
    /// mod defines is skipped, exactly like [`visuals_by_id`](Self::visuals_by_id).
    pub fn effects_by_id(&self) -> impl Iterator<Item = ((AbilityId, EffectRole), &VisualModel)> {
        self.effects
            .iter()
            .filter_map(|((name, role), model)| Some(((self.ability_ids.get(name)?, *role), model)))
    }

    /// Load a cosmetic mod from `path` — a folder or a `.zip` package.
    pub fn load(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            let loaded = loader::load_dir(path)?;
            self.adopt(loaded, ModSource::Dir(path.to_path_buf()))
        } else {
            let bytes = fs::read(path).map_err(|e| anyhow!("reading {}: {e}", path.display()))?;
            self.load_zip_bytes(&bytes)
        }
    }

    /// Load a cosmetic mod from the bytes of a `.zip` package. Byte-oriented so it
    /// is exercised hermetically without touching the filesystem.
    pub fn load_zip_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let loaded = loader::load_zip_bytes(bytes)?;
        self.adopt(loaded, ModSource::Zip(bytes.to_vec()))
    }

    /// Reject a non-cosmetic mod, run + adopt the registration, and register the
    /// package root so its `mod://` assets resolve.
    fn adopt(&mut self, loaded: LoadedMod, source: ModSource) -> Result<()> {
        if loaded.manifest.kind != ModKind::Client {
            bail!("mod `{}` is not a client (cosmetic) mod", loaded.manifest.id);
        }
        let reg = self.host.register_client(&loaded.wasm)?;
        let adopted = AdoptedVisuals::adopt(&reg)?;
        // Merge into the combined tables; a later mod overrides an earlier entry.
        for (name, model) in adopted.iter() {
            self.visuals.insert(name.clone(), model.clone());
        }
        for (key, model) in adopted.effects() {
            self.effects.insert(key.clone(), model.clone());
        }
        self.vfs.insert(loaded.manifest.id, source);
        Ok(())
    }

    /// The visual a loaded cosmetic mod declared for the unit named `unit_name`,
    /// or `None` if none did (the client uses its placeholder then).
    #[must_use]
    pub fn visual(&self, unit_name: &str) -> Option<&VisualModel> {
        self.visuals.get(unit_name)
    }

    /// Resolve a `mod://<id>/<path>` asset URL to its bytes, staying within the
    /// package root (traversal is rejected by the [`Vfs`]).
    pub fn read_asset(&self, url: &str) -> Result<Vec<u8>> {
        self.vfs.read(url)
    }

    /// Consume the host, yielding the [`Vfs`] of every loaded mod's package root.
    /// The client hands this to its `mod://` asset reader so cosmetic art keeps
    /// resolving after the startup loader (and the host itself) is gone.
    #[must_use]
    pub fn into_vfs(self) -> Vfs {
        self.vfs
    }

    /// Iterate every adopted `(unit name, visual)` across all loaded cosmetic
    /// mods, in unit-name order — how the client fills its render-side table.
    pub fn visuals(&self) -> impl Iterator<Item = (&String, &VisualModel)> {
        self.visuals.iter()
    }

    /// Number of distinct units dressed across all loaded cosmetic mods.
    #[must_use]
    pub fn visual_count(&self) -> usize {
        self.visuals.len()
    }

    /// The ability-feedback visual a loaded cosmetic mod declared for
    /// `ability_name` in `role`, or `None` (the client uses its placeholder then).
    #[must_use]
    pub fn effect(&self, ability_name: &str, role: EffectRole) -> Option<&VisualModel> {
        self.effects.get(&(ability_name.to_string(), role))
    }

    /// Iterate every adopted `((ability name, role), visual)` across all loaded
    /// cosmetic mods, in key order — how the client fills its effect table.
    pub fn effects(&self) -> impl Iterator<Item = (&(String, EffectRole), &VisualModel)> {
        self.effects.iter()
    }

    /// Number of distinct `(ability, role)` effect visuals across all loaded
    /// cosmetic mods.
    #[must_use]
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }
}
