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
use stormlight_mod_abi::manifest::{ABI_VERSION, ModKind};
use stormlight_mod_abi::visuals::{ClientRegistration, VisualModel};

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
}

impl AdoptedVisuals {
    /// Build the unit-name → visual table from a decoded [`ClientRegistration`].
    ///
    /// Total over hostile input: a major ABI mismatch, or a visual referencing a
    /// unit handle with no [`names.units`](stormlight_mod_abi::descriptors::Names)
    /// entry (a dangling reference), is an `Err` — never a panic. When two visuals
    /// name the same unit the later one wins (deterministic, insertion order).
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
        Ok(Self { by_name })
    }

    /// The visual declared for the unit named `unit_name`, if any.
    #[must_use]
    pub fn get(&self, unit_name: &str) -> Option<&VisualModel> {
        self.by_name.get(unit_name)
    }

    /// Number of units this mod dresses.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether this mod declares no visuals.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Iterate the `(unit name, visual)` pairs, in unit-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &VisualModel)> {
        self.by_name.iter()
    }
}

/// The client-side host: loads cosmetic mods, runs each guest's registration,
/// merges the adopted visuals into one unit-name → model table, and resolves
/// every loaded mod's `mod://` assets.
pub struct ClientHost {
    host: Host,
    vfs: Vfs,
    visuals: BTreeMap<String, VisualModel>,
}

impl ClientHost {
    /// A fresh host with no mods loaded.
    pub fn new() -> Result<Self> {
        Ok(Self { host: Host::new()?, vfs: Vfs::new(), visuals: BTreeMap::new() })
    }

    /// Load a cosmetic mod from `path` — a folder or a `.zip` package.
    pub fn load(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            let loaded = loader::load_dir(path)?;
            self.adopt(loaded, ModSource::Dir(path.to_path_buf()))
        } else {
            let bytes =
                fs::read(path).map_err(|e| anyhow!("reading {}: {e}", path.display()))?;
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
        // Merge into the combined table; a later mod overrides an earlier unit.
        for (name, model) in adopted.iter() {
            self.visuals.insert(name.clone(), model.clone());
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
}
