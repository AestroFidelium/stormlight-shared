//! The `mod://<id>/<path>` asset VFS, shared by server and client.
//!
//! A mod addresses its own assets by a stable URL that resolves to the mod's
//! package root — a folder on disk or an entry inside a `.zip`. The core is
//! [`parse_mod_url`], which validates the reference and, crucially, **rejects
//! path traversal**: no `..`/`.` component, no absolute path, no backslash. A
//! validated [`ModPath`] therefore always resolves *within* the mod root, so a
//! hostile `mod://x/../../etc/passwd` never escapes.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use zip::ZipArchive;

const SCHEME: &str = "mod://";

/// A parsed, validated `mod://<id>/<path>` reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModPath {
    /// The target mod's id.
    pub id: String,
    /// A safe relative path: non-empty, `/`-separated, with no `.`/`..`/empty
    /// components and no backslashes.
    pub path: String,
}

/// Parse and validate a `mod://<id>/<path>` URL. Total over arbitrary input: a
/// malformed or traversing URL is an `Err`, never a panic.
pub fn parse_mod_url(url: &str) -> Result<ModPath> {
    let rest = url.strip_prefix(SCHEME).ok_or_else(|| anyhow!("not a `mod://` url"))?;
    let (id, path) = rest.split_once('/').ok_or_else(|| anyhow!("missing `/` after mod id"))?;
    validate_id(id)?;
    Ok(ModPath { id: id.to_owned(), path: validate_path(path)? })
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("empty mod id");
    }
    if !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        bail!("mod id `{id}` has a character outside `[a-z0-9_]`");
    }
    Ok(())
}

/// Validate a relative resource path and return it normalized. Rejects anything
/// that could escape the package root.
fn validate_path(path: &str) -> Result<String> {
    if path.is_empty() {
        bail!("empty resource path");
    }
    if path.contains('\\') {
        bail!("backslash in resource path");
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" => bail!("empty path component (leading, trailing, or doubled `/`)"),
            "." | ".." => bail!("`{component}` traversal is not allowed"),
            c if c.contains('\0') => bail!("nul byte in path"),
            c => components.push(c),
        }
    }
    Ok(components.join("/"))
}

/// Read a validated resource from a folder-backed package.
pub fn read_from_dir(root: &Path, resource: &ModPath) -> Result<Vec<u8>> {
    let target = root.join(&resource.path);
    // The validated path has no `..`, so this holds; assert it as defense in depth.
    if !target.starts_with(root) {
        bail!("resource path escapes the mod root");
    }
    fs::read(&target).with_context(|| format!("reading {}", target.display()))
}

/// Read a validated resource from a `.zip`-backed package.
pub fn read_from_zip_bytes(zip_bytes: &[u8], resource: &ModPath) -> Result<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(zip_bytes)).context("opening zip package")?;
    read_entry(&mut zip, &resource.path)
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut file = zip.by_name(name).with_context(|| format!("zip entry `{name}`"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).with_context(|| format!("reading zip entry `{name}`"))?;
    Ok(buf)
}

/// Where a mod's assets live.
#[derive(Clone, Debug)]
pub enum ModSource {
    /// A folder on disk.
    Dir(PathBuf),
    /// The bytes of a `.zip` package.
    Zip(Vec<u8>),
}

/// Resolves `mod://` URLs across all loaded mods.
#[derive(Default)]
pub struct Vfs {
    roots: BTreeMap<String, ModSource>,
}

impl Vfs {
    /// An empty VFS.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a mod's package root under its id.
    pub fn insert(&mut self, id: impl Into<String>, source: ModSource) {
        self.roots.insert(id.into(), source);
    }

    /// Read the bytes a `mod://<id>/<path>` URL points at.
    pub fn read(&self, url: &str) -> Result<Vec<u8>> {
        let resource = parse_mod_url(url)?;
        let source = self
            .roots
            .get(&resource.id)
            .ok_or_else(|| anyhow!("no mod registered as `{}`", resource.id))?;
        match source {
            ModSource::Dir(root) => read_from_dir(root, &resource),
            ModSource::Zip(bytes) => read_from_zip_bytes(bytes, &resource),
        }
    }
}
