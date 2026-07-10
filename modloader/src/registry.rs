//! Decoding a guest's emitted [`Registration`] and collecting registrations
//! into a host-side [`ModRegistry`] keyed by mod id.
//!
//! [`decode_registration`] is the pure half: given a snapshot of the guest's
//! linear memory and the packed `(ptr, len)` its `mod_register` returned, it
//! bounds-checks the region and postcard-decodes it. It is **total** over
//! arbitrary memory + packed values — an out-of-bounds or malformed payload is
//! an `Err`, never a panic (the guest is untrusted).

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use stormlight_mod_abi::bridge::unpack_ptr_len;
use stormlight_mod_abi::descriptors::Registration;
use stormlight_mod_abi::manifest::{ABI_VERSION, Manifest};

/// Decode a [`Registration`] out of `memory` at the region named by `packed`.
pub fn decode_registration(memory: &[u8], packed: u64) -> Result<Registration> {
    let (ptr, len) = unpack_ptr_len(packed);
    let (ptr, len) = (ptr as usize, len as usize);
    let end = ptr.checked_add(len).ok_or_else(|| anyhow!("registration ptr+len overflows"))?;
    let bytes =
        memory.get(ptr..end).ok_or_else(|| anyhow!("registration region {ptr}..{end} out of bounds"))?;
    postcard::from_bytes(bytes).map_err(|e| anyhow!("decoding registration: {e}"))
}

/// A mod that has been loaded, instantiated, and registered: its validated
/// manifest and the descriptors it emitted.
#[derive(Clone, Debug)]
pub struct ModEntry {
    pub manifest: Manifest,
    pub registration: Registration,
}

/// All registered mods, keyed by their (unique) manifest id.
#[derive(Default, Debug)]
pub struct ModRegistry {
    mods: BTreeMap<String, ModEntry>,
}

impl ModRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt a mod's registration under its manifest id. Rejects a major ABI
    /// mismatch in the *emitted* registration (a second gate beyond the manifest
    /// check) and a duplicate id.
    pub fn insert(&mut self, manifest: Manifest, registration: Registration) -> Result<()> {
        if registration.abi.major != ABI_VERSION.major {
            bail!(
                "mod `{}` registration abi major {} != engine {}",
                manifest.id,
                registration.abi.major,
                ABI_VERSION.major
            );
        }
        if self.mods.contains_key(&manifest.id) {
            bail!("duplicate mod id `{}`", manifest.id);
        }
        self.mods.insert(manifest.id.clone(), ModEntry { manifest, registration });
        Ok(())
    }

    /// The entry registered under `id`, if any.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ModEntry> {
        self.mods.get(id)
    }

    /// Number of registered mods.
    #[must_use]
    pub fn len(&self) -> usize {
        self.mods.len()
    }

    /// Whether no mods are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }
}
