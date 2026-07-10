//! Mod loading — locate and validate a mod's `manifest.toml` and entry `.wasm`,
//! from either a **folder** or a **`.zip`** package.
//!
//! This is the read + validate half of the pipeline: parse the manifest against
//! the ABI (which rejects a major [`ABI_VERSION`](stormlight_mod_abi::manifest::ABI_VERSION)
//! mismatch, a bad id, a non-`.wasm` entry, …) and pull the entry bytes. The
//! `mod://` asset VFS and instantiation come later (S8/S7).
//!
//! Every entry point is **total over hostile input**: malformed TOML or a
//! corrupt zip yields an `Err`, never a panic.

use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use stormlight_mod_abi::manifest::{Manifest, parse_manifest};
use zip::ZipArchive;

/// A loaded mod: its validated manifest and the raw bytes of its entry wasm.
#[derive(Clone, Debug)]
pub struct LoadedMod {
    pub manifest: Manifest,
    pub wasm: Vec<u8>,
}

/// Load a mod from `path`, dispatching on whether it is a directory (folder
/// package) or a file (`.zip`).
pub fn load(path: &Path) -> Result<LoadedMod> {
    if path.is_dir() {
        load_dir(path)
    } else {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        load_zip_bytes(&bytes)
    }
}

/// Load a mod laid out as a folder: `<dir>/manifest.toml` + `<dir>/<entry>`.
pub fn load_dir(dir: &Path) -> Result<LoadedMod> {
    let src = fs::read_to_string(dir.join("manifest.toml")).context("reading manifest.toml")?;
    let manifest = parse_manifest(&src).map_err(|e| anyhow!("invalid manifest: {e}"))?;
    let wasm = fs::read(dir.join(&manifest.entry))
        .with_context(|| format!("reading entry `{}`", manifest.entry))?;
    Ok(LoadedMod { manifest, wasm })
}

/// Load a mod from the bytes of a `.zip` package. Kept byte-oriented so it is
/// trivially fuzzable without touching the filesystem.
pub fn load_zip_bytes(bytes: &[u8]) -> Result<LoadedMod> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).context("opening zip package")?;
    let src = read_entry_string(&mut zip, "manifest.toml")?;
    let manifest = parse_manifest(&src).map_err(|e| anyhow!("invalid manifest: {e}"))?;
    let wasm = read_entry_bytes(&mut zip, &manifest.entry)?;
    Ok(LoadedMod { manifest, wasm })
}

fn read_entry_bytes<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut file = zip.by_name(name).with_context(|| format!("zip entry `{name}`"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).with_context(|| format!("reading zip entry `{name}`"))?;
    Ok(buf)
}

fn read_entry_string<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<String> {
    let bytes = read_entry_bytes(zip, name)?;
    String::from_utf8(bytes).with_context(|| format!("zip entry `{name}` is not UTF-8"))
}
