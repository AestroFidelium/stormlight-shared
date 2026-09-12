//! What a HUD's *actions* need from the id bridge (stormlight/server#69).
//!
//! [`client_ui_bridge`](super::client_ui_bridge) pins the same property for what a
//! widget **reads**. Interaction adds two more things the client cannot get from
//! the wire, and both come from the gameplay mods loaded for the id map:
//!
//! - a **fourth family** crosses the bridge. A [`UiAction::Trigger`] names an
//!   `EventId`, and the gameplay side is what decides which global id that name
//!   interned to — so a cosmetic mod's local `EventId(0)` must be re-keyed exactly
//!   as its stat bindings are. Left alone it would raise whichever event happened
//!   to intern first, which is a button doing something nobody declared.
//! - **talent trees**, keyed by global `UnitId`. A [`UiAction::PickTalent`] names
//!   a tier and an *option index*, and the wire deliberately carries no options
//!   ([`ReplicatedTalents`] is a per-player view of choices, not of content), so
//!   the option can only be turned into a talent id against the unit's own
//!   declared tree. This is the same reconstruction the aim modes and talent names
//!   already ride on.
//!
//! Hermetic `.wat` fixtures throughout, like its neighbours — no filesystem, no
//! real mod.

use std::io::{Cursor, Write};

use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{EventId, Slot, TalentId, UnitId};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::math::Value;
use stormlight_mod_abi::talent_tree::{RepickPolicy, TalentTier, TalentTree};
use stormlight_mod_abi::ui::{
    Layout, RootVisibility, Strip, Style, SummonGate, UiAction, UiRoot, UiSubject, Widget,
    WidgetKind,
};
use stormlight_mod_abi::units::UnitDescriptor;
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

/// The barest unit a registration can carry — nothing but the fields a
/// descriptor requires, so a fixture states only the tree it is about.
fn bare_unit() -> UnitDescriptor {
    UnitDescriptor {
        id: UnitId(0),
        health: Value::Const(100.0),
        stats: Vec::new(),
        tags: Vec::new(),
        abilities: Vec::new(),
        resources: Vec::new(),
        talents: Vec::new(),
        talent_tree: None,
        respawn: None,
        progression: None,
        turn_rate: None,
        tasks: Vec::new(),
        attack: None,
    }
}

/// A unit declaring a tree over the talents named at `talents`, by their **local**
/// handle — exactly how a mod authors one.
fn a_unit(tiers: &[(u8, &[u32])]) -> UnitDescriptor {
    UnitDescriptor {
        talent_tree: Some(TalentTree {
            tiers: tiers
                .iter()
                .map(|(level, options)| TalentTier {
                    level: *level,
                    options: options.iter().map(|raw| TalentId(*raw)).collect(),
                    recommended: None,
                })
                .collect(),
            repick: RepickPolicy::Locked,
        }),
        ..bare_unit()
    }
}

/// A gameplay package naming `events`, `talents` and `units`, the last declaring
/// the given trees in order.
fn gameplay(events: &[&str], talents: &[&str], units: &[(&str, UnitDescriptor)]) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            events: events.iter().map(|s| (*s).into()).collect(),
            talents: talents.iter().map(|s| (*s).into()).collect(),
            units: units.iter().map(|(name, _)| (*name).into()).collect(),
            ..Names::default()
        },
        units: units.iter().map(|(_, unit)| unit.clone()).collect(),
        ..Registration::default()
    };
    package("gameplay", "server", &postcard::to_allocvec(&reg).unwrap())
}

fn a_widget(kind: WidgetKind) -> Widget {
    Widget { name: String::new(), layout: Layout::default(), style: Style::default(), kind }
}

/// A cosmetic package whose HUD holds one trigger button per event it names — each
/// at its own local handle, all starting from 0 — followed by the two handle-free
/// actions.
fn cosmetic(events: &[&str]) -> Vec<u8> {
    let mut children: Vec<Widget> = (0..events.len())
        .map(|raw| {
            a_widget(WidgetKind::Button {
                action: UiAction::Trigger { event: EventId(raw as u16) },
                children: Vec::new(),
            })
        })
        .collect();
    children.push(a_widget(WidgetKind::Button {
        action: UiAction::CastSlot(Slot(2)),
        children: Vec::new(),
    }));
    children.push(a_widget(WidgetKind::Button {
        action: UiAction::PickTalent { tier: 1, option: 1 },
        children: Vec::new(),
    }));

    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { events: events.iter().map(|s| (*s).into()).collect(), ..Names::default() },
        ui: vec![UiRoot {
            name: "hud".into(),
            when: RootVisibility::Always,
            summon: SummonGate::Ignored,
            subject: UiSubject::LocalPlayer,
            strip: Strip::default(),
            root: a_widget(WidgetKind::Panel { children }),
        }],
        ..ClientRegistration::default()
    };
    package("cosmetic", "client", &postcard::to_allocvec(&reg).unwrap())
}

/// Every action the host's single adopted root declares, in declaration order.
fn actions(host: &ClientHost) -> Vec<UiAction> {
    let root = host.ui().first().expect("one root was declared");
    let WidgetKind::Panel { children } = &root.root.kind else { panic!("root is a panel") };
    children.iter().filter_map(|w| w.kind.action()).collect()
}

#[test]
fn a_trigger_action_rekeys_to_the_gameplay_sides_event_id() {
    let mut host = ClientHost::new().unwrap();
    // The gameplay mod interns "rally" as its *second* event, so the global id is
    // not the local 0 the cosmetic authored — an unmapped action cannot pass for a
    // mapped one.
    host.load_gameplay_zip_bytes(&gameplay(&["banner", "rally"], &[], &[]))
        .expect("gameplay names load");
    host.load_zip_bytes(&cosmetic(&["rally"])).expect("cosmetic loads");

    assert_eq!(
        actions(&host),
        vec![
            UiAction::Trigger { event: EventId(1) },
            // The handle-free actions ride through untouched: a slot is mod
            // convention and an option is an index into the unit's own tree.
            UiAction::CastSlot(Slot(2)),
            UiAction::PickTalent { tier: 1, option: 1 },
        ],
        "a trigger must land on the id the gameplay side interned its name at",
    );
}

#[test]
fn an_event_no_gameplay_mod_declares_keeps_the_tree_on_an_id_of_its_own() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["banner"], &[], &[])).expect("gameplay loads");
    host.load_zip_bytes(&cosmetic(&["mystery"])).expect("cosmetic loads");

    // Same rule as an unexplained binding: the HUD survives whole, and the unknown
    // event lands on an id no gameplay mod subscribed to — a button that does
    // nothing, never a button firing someone else's event.
    assert_eq!(host.ui().len(), 1, "an unresolved action must not cost the tree");
    match actions(&host).as_slice() {
        [UiAction::Trigger { event }, ..] => {
            assert_ne!(*event, EventId(0), "must not collide with the declared `banner`");
        }
        other => panic!("expected a trigger first, got {other:?}"),
    }
}

#[test]
fn a_trigger_handle_the_bundles_own_names_do_not_explain_is_refused() {
    // A registration contradicting itself: a button naming a local event its own
    // table has no entry for. There is nothing to intern, and resolving it
    // positionally would fire whatever unrelated event sits at that raw index.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&["banner"], &[], &[])).expect("gameplay loads");

    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { events: vec!["banner".into()], ..Names::default() },
        ui: vec![UiRoot {
            name: "hud".into(),
            when: RootVisibility::Always,
            summon: SummonGate::Ignored,
            subject: UiSubject::LocalPlayer,
            strip: Strip::default(),
            root: a_widget(WidgetKind::Button {
                action: UiAction::Trigger { event: EventId(7) },
                children: Vec::new(),
            }),
        }],
        ..ClientRegistration::default()
    };
    let bundle = package("cosmetic", "client", &postcard::to_allocvec(&reg).unwrap());
    assert!(host.load_zip_bytes(&bundle).is_err(), "a dangling event handle must be refused");
}

#[test]
fn a_units_talent_tree_is_reachable_by_its_global_id_with_global_talent_ids() {
    let mut host = ClientHost::new().unwrap();
    // Two units, so the second one's tree is not reachable at a lucky index 0, and
    // a tree whose options are the mod's *local* talent handles 0 and 1.
    host.load_gameplay_zip_bytes(&gameplay(
        &[],
        &["swift", "sturdy", "sharp"],
        &[("dummy", bare_unit()), ("fighter", a_unit(&[(1, &[0, 1]), (4, &[2])]))],
    ))
    .expect("gameplay loads");

    let tree = host.talent_tree(UnitId(1)).expect("the second unit declares a tree");
    assert_eq!(tree.tiers.len(), 2);
    assert_eq!(tree.tiers[0].level, 1);
    // The options are the ids the *wire* carries, not the mod's local handles —
    // the client compares them against `ReplicatedTalents`, which is global.
    assert_eq!(tree.tiers[0].options, vec![TalentId(0), TalentId(1)]);
    assert_eq!(tree.tiers[1].options, vec![TalentId(2)]);

    // A unit that declares none has none — never an empty tree standing in, which
    // a HUD would draw as a talent panel with nothing in it.
    assert!(host.talent_tree(UnitId(0)).is_none(), "a unit with no tree reports none");
    assert!(host.talent_tree(UnitId(9)).is_none(), "an unknown unit reports none");
}

#[test]
fn a_second_gameplay_mods_tree_options_keep_pointing_at_its_own_talents() {
    // The hazard the whole bridge exists for: both mods author their talents from
    // local handle 0. A tree adopted without re-keying would have the second mod's
    // tier offering the first mod's talents.
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(&[], &["swift"], &[("dummy", a_unit(&[(1, &[0])]))]))
        .expect("first gameplay mod loads");
    host.load_gameplay_zip_bytes(&gameplay(&[], &["sturdy"], &[("fighter", a_unit(&[(1, &[0])]))]))
        .expect("second gameplay mod loads");

    let first = host.talent_tree(UnitId(0)).expect("first unit's tree");
    let second = host.talent_tree(UnitId(1)).expect("second unit's tree");
    assert_eq!(first.tiers[0].options, vec![TalentId(0)], "`swift` interned first");
    assert_eq!(
        second.tiers[0].options,
        vec![TalentId(1)],
        "`sturdy` is the second talent, and the second mod's tier must offer it",
    );
}
