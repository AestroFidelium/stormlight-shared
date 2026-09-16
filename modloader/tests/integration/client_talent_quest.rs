//! The task a talent sets, in the client's own id space (stormlight/server#132).
//!
//! A quest names two things the client has to cross an id space to use: the
//! **talent** that sets it, and the **counter** it is counted in. The first is the
//! bridge every talent table crosses (server#129). The second is new, and it is
//! the one that makes a running count drawable at all: the counter names the same
//! stack family a HUD's own `Pool` binding names, and the binding bridge interns
//! that family in the gameplay-load order the server adopts in — so the id a quest
//! ends up on is the id the owner's replicated counters arrive under.
//!
//! Before this the counter was kept exactly as its mod authored it, on the honest
//! grounds that nothing client-side read a stack counter and translating a handle
//! into a family the client did not intern would be inventing an id. The client
//! interns that family now, so the translation is a translation.
//!
//! Two things are deliberately *not* carried across:
//!
//!   - **the prize.** A client never pays a quest out. An `Impact` tree full of
//!     handles nothing here has translated would be ids meaning another mod's
//!     content, sitting in a table waiting for somebody to read them;
//!   - **a counter its own mod never named.** That is a registration contradicting
//!     itself, and it is refused rather than resolved to whatever happens to sit at
//!     that raw index — the same call [`ClientHost`] already makes for an ability a
//!     talent claims to be about.
//!
//! Hermetic `.wat` fixtures, the shape `client_talent_focus.rs` uses.

use std::io::{Cursor, Write};

use stormlight_mod_abi::common::{ImpactTarget, NumOp};
use stormlight_mod_abi::descriptors::{Names, Registration};
use stormlight_mod_abi::ids::{BuffId, ParamId, Slot, StackId, TalentId};
use stormlight_mod_abi::impacts::Impact;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::math::Value;
use stormlight_mod_abi::talents::{AbilityFocus, AbilitySelector, ParamPatch, TalentDescriptor};
use stormlight_mod_abi::tasks::QuestSpec;
use stormlight_modloader::client::ClientHost;
use zip::write::SimpleFileOptions;

const ENTRY: &str = "guest.wasm";
const DATA_OFFSET: u32 = 1024;

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

/// A talent patching the ability in slot 0, setting `quest` as its task.
fn talent(id: u32, quest: Option<QuestSpec>) -> TalentDescriptor {
    TalentDescriptor {
        id: TalentId(id),
        selector: vec![AbilitySelector::Slot(Slot(0))],
        patches: vec![ParamPatch {
            param: ParamId(0),
            op: NumOp::Add,
            value: Value::Const(1.0),
            selector: None,
        }],
        riders: Vec::new(),
        add_reactions: Vec::new(),
        grants: Vec::new(),
        modifiers: Vec::new(),
        tags: Vec::new(),
        quest,
    }
}

/// A prize: the shape a real quest hands over, in the declaring mod's own handles.
fn prize() -> Vec<Impact> {
    vec![Impact::ApplyModifiers {
        buff: BuffId(0),
        stacks: Value::Const(1.0),
        duration_override: None,
        target: ImpactTarget::Caster,
    }]
}

/// A gameplay `.zip` whose registration is exactly these names and talents.
fn gameplay(
    id: &str,
    stacks: &[&str],
    talents: &[&str],
    declared: Vec<TalentDescriptor>,
) -> Vec<u8> {
    let reg = Registration {
        abi: ABI_VERSION,
        names: Names {
            abilities: vec!["bolt".into()],
            talents: talents.iter().map(|s| (*s).into()).collect(),
            stacks: stacks.iter().map(|s| (*s).into()).collect(),
            ..Names::default()
        },
        talents: declared,
        ..Registration::default()
    };
    let payload = postcard::to_allocvec(&reg).unwrap();
    let wasm = guest_wasm(&payload);
    let manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\n\
         kind = \"server\"\nentry = \"{ENTRY}\"\nabi = \"{}.0.0\"\n",
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

/// The load-bearing one. Both mods author their counter as local `StackId(0)` and
/// both are keyed by their own first talent — so a client that kept either handle
/// as authored would have the second mod's quest watching the first mod's counter,
/// and would show a count that belongs to somebody else's talent.
#[test]
fn a_counter_is_re_keyed_into_the_space_the_owners_counts_arrive_in() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolts_fired"],
        &["swift"],
        vec![talent(0, Some(QuestSpec::single(StackId(0), 40.0, prize())))],
    ))
    .expect("first loads");
    host.load_gameplay_zip_bytes(&gameplay(
        "second",
        &["blows_landed"],
        &["hunt"],
        vec![talent(0, Some(QuestSpec::single(StackId(0), 10.0, prize())))],
    ))
    .expect("second loads");

    let quests: Vec<(TalentId, StackId, Option<f32>)> =
        host.talent_quests().map(|(id, quest)| (id, quest.counter, quest.goal())).collect();
    assert_eq!(
        quests,
        vec![(TalentId(0), StackId(0), Some(40.0)), (TalentId(1), StackId(1), Some(10.0))],
        "two mods authoring the same local counter did not land on two global ones",
    );
}

/// A client never pays a quest out, and a handle it has not translated is a handle
/// that means somebody else's content.
#[test]
fn the_prize_does_not_cross() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolts_fired"],
        &["swift"],
        vec![talent(0, Some(QuestSpec::single(StackId(0), 40.0, prize())))],
    ))
    .expect("first loads");

    let carried: Vec<usize> = host
        .talent_quests()
        .map(|(_, quest)| (0..quest.rungs()).map(|rung| quest.reward(rung).len()).sum())
        .collect();
    assert_eq!(carried, vec![0], "the client kept effects it can neither run nor read");
}

/// A registration that contradicts itself. Refused with a reason, exactly as an
/// ability a talent claims to be about but never named is — a quest silently
/// counting the wrong thing is a bug an author cannot find the cause of.
#[test]
fn a_counter_the_mod_never_named_is_refused() {
    let mut host = ClientHost::new().unwrap();
    let err = host
        .load_gameplay_zip_bytes(&gameplay(
            "first",
            &[],
            &["swift"],
            vec![talent(0, Some(QuestSpec::single(StackId(4), 40.0, prize())))],
        ))
        .expect_err("a dangling counter must not load");
    assert!(
        format!("{err:#}").contains("stack"),
        "the refusal must name the family that dangled: {err:#}",
    );
}

/// A task is a second thing a talent is, never a replacement for the first. A quest
/// row still tells the player which of their buttons it lands on (server#129).
#[test]
fn a_talent_that_sets_a_task_still_reports_the_button_it_changes() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolts_fired"],
        &["swift"],
        vec![talent(0, Some(QuestSpec::single(StackId(0), 40.0, prize())))],
    ))
    .expect("first loads");

    assert_eq!(
        host.talent_focus().collect::<Vec<_>>(),
        vec![(TalentId(0), AbilityFocus::Slot(Slot(0)))],
        "a talent that sets a task stopped reporting which button it is about",
    );
}

#[test]
fn an_ordinary_talent_still_sets_no_task() {
    let mut host = ClientHost::new().unwrap();
    host.load_gameplay_zip_bytes(&gameplay(
        "first",
        &["bolts_fired"],
        &["swift"],
        vec![talent(0, None)],
    ))
    .expect("first loads");

    assert_eq!(host.talent_quests().count(), 0, "a talent that declared no task carries one");
}
