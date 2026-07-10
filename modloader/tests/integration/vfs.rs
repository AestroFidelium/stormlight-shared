//! Resolving `mod://` URLs against real folder- and zip-backed packages: a
//! valid reference reads the right bytes; a traversal, a bad scheme, or an
//! unknown id is rejected. (Traversal safety over arbitrary URLs is fuzzed in
//! `fuzz/vfs.rs`.)

use std::io::{Cursor, Write};

use stormlight_modloader::vfs::{ModSource, Vfs, parse_mod_url};
use zip::write::SimpleFileOptions;

#[test]
fn a_valid_url_parses_into_id_and_path() {
    let resource = parse_mod_url("mod://base/textures/hero.png").unwrap();
    assert_eq!(resource.id, "base");
    assert_eq!(resource.path, "textures/hero.png");
}

#[test]
fn traversal_and_bad_scheme_are_rejected() {
    for bad in [
        "mod://base/../secret",
        "mod://base/a/../b",
        "mod://base/./b",
        "mod://base//x",
        "mod://base/",
        "mod://base/a\\b",
        "mod:///x",       // empty id
        "mod://BASE/x",   // invalid id charset
        "file://base/x",  // wrong scheme
        "base/x",         // no scheme
    ] {
        assert!(parse_mod_url(bad).is_err(), "should reject `{bad}`");
    }
}

#[test]
fn folder_and_zip_packages_resolve_the_same_bytes() {
    let payload = b"pixels".to_vec();

    // Folder-backed mod.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("textures")).unwrap();
    std::fs::write(dir.path().join("textures/hero.png"), &payload).unwrap();

    // Zip-backed mod with the same layout.
    let mut zip_bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut zip_bytes));
        zip.start_file("textures/hero.png", SimpleFileOptions::default()).unwrap();
        zip.write_all(&payload).unwrap();
        zip.finish().unwrap();
    }

    let mut vfs = Vfs::new();
    vfs.insert("foldermod", ModSource::Dir(dir.path().to_path_buf()));
    vfs.insert("zipmod", ModSource::Zip(zip_bytes));

    assert_eq!(vfs.read("mod://foldermod/textures/hero.png").unwrap(), payload);
    assert_eq!(vfs.read("mod://zipmod/textures/hero.png").unwrap(), payload);
}

#[test]
fn an_unknown_mod_id_is_rejected() {
    let vfs = Vfs::new();
    assert!(vfs.read("mod://nope/x").is_err());
}

#[test]
fn a_traversal_url_never_reaches_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ok.txt"), b"ok").unwrap();
    let mut vfs = Vfs::new();
    vfs.insert("m", ModSource::Dir(dir.path().to_path_buf()));

    assert_eq!(vfs.read("mod://m/ok.txt").unwrap(), b"ok");
    assert!(vfs.read("mod://m/../ok.txt").is_err(), "traversal must be refused before any read");
}
