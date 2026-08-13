//! The HUD's half of the name→global-id bridge (stormlight/server#67).
//!
//! A cosmetic mod authors `ValueBinding::Stat("power")` in its **own** id space,
//! but the client resolves that binding against state keyed by the gameplay mod's
//! **global** id. [`ClientHost`] already rebuilds the unit/ability id space from
//! the loaded gameplay mods (server#44); a HUD reaches three more families — stats,
//! resource pools and stack counters — so the same pass interns those, seeded with
//! the ABI's reserved stat names so the client and the server agree about which id
//! `armor` is.
//!
//! Invariants (hermetic `.wat` fixtures, like `client_id_bridge.rs`):
//! - a bound handle lands on the id at its **interning position** on the gameplay
//!   side, reserved names included (deliberately non-zero, so "same id space as
//!   adoption" is pinned rather than an accidental 0);
//! - a binding no gameplay mod explains keeps its tree and lands on an id of its
//!   own — a bar that reads nothing, never a bar reading someone else's number,
//!   and never a refused HUD;
//! - the bridge is order-sensitive by construction, so loading a gameplay mod
//!   *after* a cosmetic one is refused rather than silently shifting ids.

use std::io::{Cursor, Write};

use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{ResourceId, StackId, StatId};
use stormlight_mod_abi::impacts::PoolRef;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::stats;
use stormlight_mod_abi::ui::{
    Layout, RootVisibility, Style, UiRoot, UiSubject, ValueBinding, Widget, WidgetKind,
};
use stormlight_mod_abi::visuals::ClientRegistration;
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

/// A guest returning `packed(DATA_OFFSET, len)` for a data segment holding
/// `payload` — the shared fixture shape.
fn guest_wasm(payload: &[u8]) -> Vec<u8> {
    let escaped: String = payload.iter().map(|b| format!("\\{b:02x}")).collect();
    let packed = (u64::from(DATA_OFFSET) << 32) | payload.len() as u64;
    let wat = format!(
        "(module\n\
         \x20 (memory (export \"memory\") 1)\n\
         \x20 (data (i32.const {DATA_OFFSET}) \"{escaped}\")\n\
         \x20 (func (export \"mod_register\") (result i64) (i64.const {packed})))"
    );
    wat::parse_str(&wat).expect("fixture wat compiles")
}

/// A `.zip` package of `kind` carrying the manifest + guest wasm.
fn package(id: &str, kind: &str, payload: &[u8]) -> Vec<u8> {
    let wasm = guest_wasm(payload);
    let manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\n\
         kind = \"{kind}\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
        ABI_VERSION.major
    );
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.toml", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file(ENTRY, opts).unwrap();
        zip.write_all(&wasm).unwrap();
        zip.finish().unwrap();
    }
    buf
}

/// A gameplay package interning the given stat / resource / stack names, in order.
fn gameplay(stats: &[&str], resources: &[&str], stacks: &[&str]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            stats: stats.iter().map(|s| (*s).into()).collect(),
            resources: resources.iter().map(|s| (*s).into()).collect(),
            stacks: stacks.iter().map(|s| (*s).into()).collect(),
            ..Names::default()
        },
        ..Registration::default()
    };
    package("gameplay", "server", &postcard::to_allocvec(&reg).unwrap())
}

fn a_widget(kind: WidgetKind) -> Widget {
    Widget { name: String::new(), layout: Layout::default(), style: Style::default(), kind }
}

/// A cosmetic package whose HUD binds one bar per given name, in the order named —
/// each authored at its own local handle, all of them starting from 0.
fn cosmetic(stats: &[&str], resources: &[&str], stacks: &[&str]) -> Vec<u8> {
    let mut children = Vec::new();
    for (raw, _) in stats.iter().enumerate() {
        children.push(a_widget(WidgetKind::Bar { value: ValueBinding::Stat(StatId(raw as u16)) }));
    }
    for (raw, _) in resources.iter().enumerate() {
        children.push(a_widget(WidgetKind::Bar {
            value: ValueBinding::Pool(PoolRef::Resource(ResourceId(raw as u16))),
        }));
    }
    for (raw, _) in stacks.iter().enumerate() {
        children.push(a_widget(WidgetKind::Bar {
            value: ValueBinding::Pool(PoolRef::Stacks(StackId(raw as u16))),
        }));
    }
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names {
            stats: stats.iter().map(|s| (*s).into()).collect(),
            resources: resources.iter().map(|s| (*s).into()).collect(),
            stacks: stacks.iter().map(|s| (*s).into()).collect(),
            ..Names::default()
        },
        ui: vec![UiRoot {
            name: "hud".into(),
            when: RootVisibility::Always,
            subject: UiSubject::LocalPlayer,
            root: a_widget(WidgetKind::Panel { children }),
        }],
        ..ClientRegistration::default()
    };
    package("cosmetic", "client", &postcard::to_allocvec(&reg).unwrap())
}

/// The bindings of the host's single adopted root, in declaration order.
fn bindings(host: &ClientHost) -> Vec<ValueBinding> {
    let root = host.ui().first().expect("one root was declared");
    let WidgetKind::Panel { children } = &root.root.kind else { panic!("root is a panel") };
    children
        .iter()
        .map(|w| match &w.kind {
            WidgetKind::Bar { value } => *value,
            other => panic!("fixture declares bars only, got {other:?}"),
        })
        .collect()
}

#[test]
fn a_bound_handle_rekeys_to_the_gameplay_sides_interning_position() {
    let mut host = ClientHost::new().unwrap();
    // The gameplay mod interns "power" above the reserved stats, and "energy" as
    // its *second* resource — so neither global id is the local one the cosmetic
    // authored, and an unmapped binding cannot pass for a mapped one.
    host.load_gameplay_zip_bytes(&gameplay(&["power"], &["mana", "energy"], &["heat", "chill"]))
        .expect("gameplay names load");
    host.load_zip_bytes(&cosmetic(&["power", "armor"], &["energy"], &["chill"]))
        .expect("cosmetic loads");

    let reserved = stats::RESERVED.len() as u16;
    assert_eq!(
        bindings(&host),
        vec![
            // "power" is the first stat above the reserved block.
            ValueBinding::Stat(StatId(reserved)),
            // "armor" is reserved, and a mod naming it collapses onto that id.
            ValueBinding::Stat(StatId(stats::reserved_index(stats::ARMOR).unwrap() as u16)),
            ValueBinding::Pool(PoolRef::Resource(ResourceId(1))),
            ValueBinding::Pool(PoolRef::Stacks(StackId(1))),
        ],
        "a HUD binding must land on the id the gameplay side interned the name at",
    );
}

#[test]
fn a_binding_no_gameplay_mod_explains_keeps_its_tree_on_an_id_of_its_own() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["power"], &[], &[])).expect("gameplay loads");
    host.load_zip_bytes(&cosmetic(&["mystery"], &[], &[])).expect("cosmetic loads");

    // The HUD survives whole — dropping it would take the working half of the
    // interface with it — and the unexplained stat lands somewhere no unit has,
    // which is what makes the bar read nothing instead of someone else's number.
    assert_eq!(host.ui().len(), 1, "an unresolved binding must not cost the tree");
    let power = StatId(stats::RESERVED.len() as u16);
    match bindings(&host).as_slice() {
        [ValueBinding::Stat(id)] => {
            assert_ne!(*id, power, "an unexplained name must not collide with a declared stat");
            assert!(
                id.0 >= stats::RESERVED.len() as u16,
                "an unexplained name must not collide with a reserved stat",
            );
        }
        other => panic!("expected one stat binding, got {other:?}"),
    }
}

#[test]
fn a_handle_the_bundles_own_names_do_not_explain_is_refused() {
    // Distinct from the case above: not "a name nobody declared" but a *handle*
    // with no name at all — a registration contradicting itself. There is nothing
    // to intern, and resolving it positionally would land it on whatever unrelated
    // stat sits at that raw index, so the load fails instead.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["power"], &[], &[])).expect("gameplay loads");

    // One stat name interned, one bar bound past the end of that table.
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { stats: vec!["power".into()], ..Names::default() },
        ui: vec![UiRoot {
            name: "hud".into(),
            when: RootVisibility::Always,
            subject: UiSubject::LocalPlayer,
            root: a_widget(WidgetKind::Bar { value: ValueBinding::Stat(StatId(7)) }),
        }],
        ..ClientRegistration::default()
    };
    let bundle = package("cosmetic", "client", &postcard::to_allocvec(&reg).unwrap());
    assert!(host.load_zip_bytes(&bundle).is_err(), "a dangling binding handle must be refused");
}

#[test]
fn a_gameplay_mod_loaded_after_a_cosmetic_one_is_refused() {
    // The bridge interns as it goes, so a gameplay mod arriving late would intern
    // its names *above* whatever the cosmetic already claimed and land every one of
    // them on an id the server never assigned. Refused loudly: the alternative is a
    // whole HUD quietly reading the wrong numbers.
    let mut host = ClientHost::new().unwrap();
    host.load_zip_bytes(&cosmetic(&["power"], &[], &[])).expect("cosmetic loads");
    let late = host.load_gameplay_zip_bytes(&gameplay(&["power"], &[], &[]));
    assert!(late.is_err(), "a gameplay mod loaded after a cosmetic one must be refused");
}
