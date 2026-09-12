//! The client-side cosmetic host — the `client.wasm` counterpart to [`crate::host`].
//!
//! A `*_client` mod (`kind = "client"`) is loaded here rather than on the server.
//! Its guest emits a [`ClientRegistration`] of [`VisualModel`]s and
//! [`AnimationDescriptor`]s keyed by unit; the host runs that registration,
//! [adopts](AdoptedVisuals::adopt) them into unit-name → model / animation tables
//! the content-free client keys on, and resolves the mod's `mod://` assets through
//! the shared [`Vfs`] (which rejects path traversal). Adoption is also where an
//! animation is structurally validated — the last point it can be refused with a
//! reason instead of silently playing nothing.
//!
//! The engine still ships nothing: [`ClientHost`] only holds whatever a cosmetic
//! mod declared. A unit with no declared visual simply has no entry — the client
//! falls back to its placeholder at the call site rather than failing here.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use stormlight_mod_abi::abilities::Targeting;
use stormlight_mod_abi::animation::{AnimState, AnimationDescriptor};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::{AbilityId, Handle, TalentId, UnitId};
use stormlight_mod_abi::interner::Interner;
use stormlight_mod_abi::manifest::{ABI_VERSION, ModKind};
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_mod_abi::notify::{NotifyAction, NotifyPoint};
use stormlight_mod_abi::remap::{IdMap, RemapIds};
use stormlight_mod_abi::talent_tree::TalentTree;
use stormlight_mod_abi::talents::AbilityFocus;
use stormlight_mod_abi::tasks::QuestSpec;
use stormlight_mod_abi::ui::UiRoot;
use stormlight_mod_abi::visuals::{ClientRegistration, EffectRole, TalentInfo, VisualModel};

use crate::host::Host;
use crate::loader::{self, LoadedMod};
use crate::ui_map::{BindingIds, LocalToGlobal};
use crate::vfs::{ModSource, Vfs};

/// One cosmetic mod's visuals after adoption, keyed by unit **name** — the stable
/// string the gameplay and cosmetic sides share. Keying by name (not a local
/// handle) is what lets a separately-authored `*_client` mod dress a gameplay
/// mod's units.
#[derive(Clone, Debug, Default)]
pub struct AdoptedVisuals {
    by_name: BTreeMap<String, VisualModel>,
    by_effect: BTreeMap<(String, EffectRole), VisualModel>,
    /// The icon each ability wears in an interface (server#94), keyed by the same
    /// ability **name** its feedback visuals are keyed by. A picture rather than a
    /// [`VisualModel`]: an icon is a flat asset in a widget.
    by_icon: BTreeMap<String, String>,
    /// The portrait each unit wears in an interface (server#145), keyed by the same
    /// unit **name** its model is keyed by. A flat picture rather than a
    /// [`VisualModel`], for the reason an ability icon is one: it lands in a widget.
    by_unit_icon: BTreeMap<String, String>,
    /// What each talent is called, does and looks like (server#95), keyed by the
    /// talent **name** — the same bridge the icons cross, one family along. Words
    /// rather than a picture: a panel addresses a cell by tier and option index, so
    /// it has nothing of its own to print there.
    by_card: BTreeMap<String, TalentInfo>,
    by_animation: BTreeMap<String, AnimationDescriptor>,
    by_notify_key: BTreeMap<String, VisualModel>,
    /// The widget trees this mod declared (server#67), in **declaration order** —
    /// the only table here that is a list rather than a map. Nothing outside a
    /// tree names one, and the client draws every root it is given, so there is no
    /// key to collide over; the order is the author's own back-to-front ordering
    /// of the interface, and sorting them would reorder the HUD.
    ui: Vec<UiRoot>,
}

/// A cosmetic effect key, qualified with the package that declared it
/// (`"<mod id>/<name>"`).
///
/// Notify keys are named by the mod itself, so two packages both shipping a
/// `footstep` would fight over one entry the moment their tables are merged.
/// Qualifying at adoption gives each package its own namespace — the same thing
/// `mod://<id>/…` does for assets — and keeps a key resolving to the effect its
/// own author declared.
fn qualify(mod_id: &str, key: &str) -> String {
    format!("{mod_id}/{key}")
}

/// Every notify point an animation declares, in declaration order.
fn notifies_of(anim: &AnimationDescriptor) -> impl Iterator<Item = &NotifyPoint> {
    anim.layers.iter().flat_map(|layer| layer.states.iter()).flat_map(|s| s.notifies.iter())
}

/// The same, mutably — how adoption rewrites the keys in place.
fn notifies_of_mut(anim: &mut AnimationDescriptor) -> impl Iterator<Item = &mut NotifyPoint> {
    anim.layers
        .iter_mut()
        .flat_map(|layer| layer.states.iter_mut())
        .flat_map(|s| s.notifies.iter_mut())
}

/// Every semantic state an animation names, in declaration order — bindings
/// first, then the transitions between them.
fn states_of(anim: &AnimationDescriptor) -> impl Iterator<Item = AnimState> + '_ {
    anim.layers.iter().flat_map(|layer| {
        layer
            .states
            .iter()
            .map(|binding| binding.state)
            .chain(layer.transitions.iter().flat_map(|t| [t.from, t.to]))
    })
}

impl AdoptedVisuals {
    /// Build the cosmetic tables from a decoded [`ClientRegistration`]: unit
    /// visuals and animations keyed by unit name, ability feedback keyed by
    /// `(ability name, role)`, and each ability's interface icon keyed by ability
    /// name.
    ///
    /// Total over hostile input — each of these is an `Err`, never a panic: a
    /// major ABI mismatch; a visual or animation referencing a unit handle with no
    /// [`names.units`](stormlight_mod_abi::descriptors::Names) entry; an effect
    /// visual or an icon referencing an ability handle with no `names.abilities`
    /// entry; an
    /// animation naming a custom state with no `names.anim_states` entry; and an
    /// animation that fails
    /// [`validate`](stormlight_mod_abi::animation::AnimationDescriptor::validate);
    /// a notify announcing a mod-defined event with no `names.events` entry; and a
    /// widget tree that fails [`UiRoot::validate`].
    /// When two entries name the same key the later one wins (deterministic,
    /// insertion order).
    ///
    /// `mod_id` is the declaring package, which every cosmetic effect key is
    /// [qualified](qualify) with — both in the table and inside the notifies that
    /// name them, so the adopted animations are keyed the way the merged table is.
    pub fn adopt(reg: &ClientRegistration, mod_id: &str) -> Result<Self> {
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
        let mut by_icon = BTreeMap::new();
        for icon in &reg.icons {
            let name = reg.names.abilities.get(icon.ability.0 as usize).ok_or_else(|| {
                anyhow!(
                    "icon references ability handle {} with no name-table entry",
                    icon.ability.0
                )
            })?;
            by_icon.insert(name.clone(), icon.image.clone());
        }
        let mut by_unit_icon = BTreeMap::new();
        for icon in &reg.unit_icons {
            let name = reg.names.units.get(icon.unit.0 as usize).ok_or_else(|| {
                anyhow!("unit icon references unit handle {} with no name-table entry", icon.unit.0)
            })?;
            by_unit_icon.insert(name.clone(), icon.image.clone());
        }
        let mut by_card = BTreeMap::new();
        for card in &reg.cards {
            let name = reg.names.talents.get(card.talent.0 as usize).ok_or_else(|| {
                anyhow!("card references talent handle {} with no name-table entry", card.talent.0)
            })?;
            by_card.insert(name.clone(), card.info.clone());
        }
        let mut by_animation = BTreeMap::new();
        for a in &reg.animations {
            let name = reg.names.units.get(a.unit.0 as usize).ok_or_else(|| {
                anyhow!("animation references unit handle {} with no name-table entry", a.unit.0)
            })?;
            // Structural breakage is reported here or nowhere: past adoption the
            // descriptor reaches a renderer that would silently play nothing.
            a.validate().map_err(|e| anyhow!("animation for unit `{name}`: {e}"))?;
            for state in states_of(a) {
                if let AnimState::Custom(id) = state
                    && reg.names.anim_states.get(id.raw() as usize).is_none()
                {
                    bail!(
                        "animation for unit `{name}` names state handle {} \
                         with no name-table entry",
                        id.raw()
                    );
                }
            }
            for point in notifies_of(a) {
                if let NotifyAction::Trigger { event } = point.action
                    && reg.names.events.get(event.raw() as usize).is_none()
                {
                    bail!(
                        "animation for unit `{name}` announces event handle {} \
                         with no name-table entry",
                        event.raw()
                    );
                }
            }
            // Rewrite each notify's key into the package-qualified form the merged
            // table is keyed by. Done here, once, rather than at every lookup: the
            // renderer holds the descriptor for the character's whole life and has
            // no idea which package it came from.
            let mut adopted = a.clone();
            for point in notifies_of_mut(&mut adopted) {
                if let NotifyAction::Effect { key, .. } = &mut point.action {
                    *key = qualify(mod_id, key);
                }
            }
            by_animation.insert(name.clone(), adopted);
        }
        let by_notify_key =
            reg.named_effects.iter().map(|e| (qualify(mod_id, &e.name), e.model.clone())).collect();
        // Structural validation of the interface, for the same reason animations
        // are validated here: past this point the tree reaches a renderer that
        // would draw a blank rectangle and explain nothing.
        for root in &reg.ui {
            root.validate().map_err(|e| anyhow!("ui root `{}`: {e}", root.name))?;
        }
        Ok(Self {
            by_name,
            by_effect,
            by_icon,
            by_unit_icon,
            by_card,
            by_animation,
            by_notify_key,
            ui: reg.ui.clone(),
        })
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

    /// The icon declared for `ability_name`, if any — the picture an interface
    /// draws in whichever slot binds that ability (server#94).
    #[must_use]
    pub fn icon(&self, ability_name: &str) -> Option<&str> {
        self.by_icon.get(ability_name).map(String::as_str)
    }

    /// Iterate the `(ability name, icon path)` pairs, in name order.
    pub fn icons(&self) -> impl Iterator<Item = (&String, &String)> {
        self.by_icon.iter()
    }

    /// The portrait declared for `unit_name`, if any — the flat picture a roster
    /// row or a hero panel draws for that unit (server#145).
    #[must_use]
    pub fn unit_icon(&self, unit_name: &str) -> Option<&str> {
        self.by_unit_icon.get(unit_name).map(String::as_str)
    }

    /// Iterate the `(unit name, portrait path)` pairs, in name order.
    pub fn unit_icons(&self) -> impl Iterator<Item = (&String, &String)> {
        self.by_unit_icon.iter()
    }

    /// The card declared for `talent_name`, if any — what a talent panel prints and
    /// draws for whichever cell offers that talent (server#95).
    #[must_use]
    pub fn card(&self, talent_name: &str) -> Option<&TalentInfo> {
        self.by_card.get(talent_name)
    }

    /// Iterate the `(talent name, card)` pairs, in name order.
    pub fn cards(&self) -> impl Iterator<Item = (&String, &TalentInfo)> {
        self.by_card.iter()
    }

    /// The animation declared for the unit named `unit_name`, if any.
    #[must_use]
    pub fn animation(&self, unit_name: &str) -> Option<&AnimationDescriptor> {
        self.by_animation.get(unit_name)
    }

    /// The cosmetic effect declared under the package-qualified `key`, if any —
    /// what an animation notify spawns (server#76).
    #[must_use]
    pub fn named_effect(&self, key: &str) -> Option<&VisualModel> {
        self.by_notify_key.get(key)
    }

    /// Iterate the `(qualified key, effect)` pairs, in key order.
    pub fn named_effects(&self) -> impl Iterator<Item = (&String, &VisualModel)> {
        self.by_notify_key.iter()
    }

    /// Iterate the `(unit name, animation)` pairs, in unit-name order.
    pub fn animations(&self) -> impl Iterator<Item = (&String, &AnimationDescriptor)> {
        self.by_animation.iter()
    }

    /// Number of units this mod dresses.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether this mod declares nothing at all — no unit visual, no effect
    /// visual, no icon, no talent card, no animation, no notify effect, no
    /// interface.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
            && self.by_effect.is_empty()
            && self.by_icon.is_empty()
            && self.by_unit_icon.is_empty()
            && self.by_card.is_empty()
            && self.by_animation.is_empty()
            && self.by_notify_key.is_empty()
            && self.ui.is_empty()
    }

    /// The widget trees this mod declared, in declaration order — still in the
    /// mod's own id space, which [`ClientHost`] translates as it merges them.
    #[must_use]
    pub fn ui(&self) -> &[UiRoot] {
        &self.ui
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
    /// Each ability's interface icon, keyed by ability name until
    /// [`icons_by_id`](ClientHost::icons_by_id) crosses the same name→global-id
    /// bridge the visuals cross (server#94).
    icons: BTreeMap<String, String>,
    /// Each unit's portrait, keyed by unit name until
    /// [`unit_icons_by_id`](ClientHost::unit_icons_by_id) crosses the same
    /// name→global-id bridge the visuals cross (server#145).
    unit_icons: BTreeMap<String, String>,
    /// Each talent's card, keyed by talent name until
    /// [`cards_by_id`](ClientHost::cards_by_id) crosses the same name→global-id
    /// bridge the icons cross (server#95).
    cards: BTreeMap<String, TalentInfo>,
    animations: BTreeMap<String, AnimationDescriptor>,
    /// Cosmetic effects an animation notify spawns (server#76), keyed by the
    /// package-qualified name their declaring mod gave them.
    notify_effects: BTreeMap<String, VisualModel>,
    /// The gameplay-side name→global-id maps, rebuilt from each loaded gameplay
    /// mod's `Names` in load order — the same interning server adoption does, so
    /// a cosmetic's name resolves to the id the wire carries (`UnitTag` / `vfx`).
    /// `UnitId`/`AbilityId` have no reserved names, so cumulative interning from 0
    /// reproduces the server's ids exactly.
    unit_ids: Interner<UnitId>,
    ability_ids: Interner<AbilityId>,
    /// The same bridge for talents (server#68). A chosen talent reaches the client
    /// as a bare global id, and a HUD list bound to it has nothing to print until
    /// that id can be turned back into the name its mod declared. Kept beside the
    /// other two rather than in [`BindingIds`] because it is the *gameplay* side's
    /// id space, reconstructed from gameplay `Names` — no cosmetic mod ever names
    /// a talent.
    talent_ids: Interner<TalentId>,
    /// How each gameplay ability is aimed, keyed by the **global `AbilityId`**
    /// (server#59). The client cannot decide an aim mode locally — it is mod data
    /// like everything else — so the same gameplay pass that rebuilds the id map
    /// records the declared [`Targeting`] alongside it, and the client resolves a
    /// keypress into an `Aim` of exactly that shape.
    aiming: BTreeMap<AbilityId, Targeting>,
    /// Each unit's declared talent tree, keyed by the **global `UnitId`** the wire
    /// carries and with its options already re-keyed to global `TalentId`s
    /// (server#69).
    ///
    /// The client needs this to interpret a
    /// [`UiAction::PickTalent`](stormlight_mod_abi::ui::UiAction::PickTalent),
    /// which names a tier and an *option index*: what a tier offers is content —
    /// identical for every player driving that unit, and known before the match —
    /// so the wire deliberately does not carry it
    /// ([`ReplicatedTalents`](stormlight_shared::talents) is a view of *choices*).
    /// Rebuilt here from the same gameplay pass that rebuilds the id maps, which
    /// is the only place the client ever learns content.
    talent_trees: BTreeMap<UnitId, TalentTree>,
    /// Which one of the caster's abilities each talent changes, keyed by the
    /// **global `TalentId`** (server#129), with any ability it names already
    /// re-keyed global.
    ///
    /// Derived from the gameplay descriptor rather than declared beside it — see
    /// [`TalentDescriptor::changes`] — and only present for a talent where that
    /// question has a single answer. A HUD reads it to say *which key* a talent
    /// lands on, which is the difference between a tier that reads as four related
    /// choices and one that reads as four unrelated sentences.
    talent_focus: BTreeMap<TalentId, AbilityFocus>,
    /// Which talents set the player a **task** rather than handing over a step
    /// (server#132), keyed by the **global `TalentId`**.
    ///
    /// The declaration only: what a HUD needs in order to say a row is a quest at
    /// all. Counting it is the effect system's and the running count is not on the
    /// wire, so this is a mark and a target and nothing else.
    talent_quests: BTreeMap<TalentId, QuestSpec>,
    /// Map geometry declared by the loaded gameplay mods, in load order. The
    /// client bakes this into the same walkable region the server routes over, so
    /// it can draw the map and agree with the server about where a unit may stand
    /// (server#79). Only the geometry is kept — the descriptor's own id is local
    /// to its mod and nothing client-side names a mesh by id.
    navmeshes: Vec<NavMeshDescriptor>,
    /// The id space a HUD's [`ValueBinding`](stormlight_mod_abi::ui::ValueBinding)s
    /// are translated into (server#67) — stats, resource pools and stack counters,
    /// the three families the unit/ability bridge above does not reach. Separate
    /// because it needs the engine's reserved stat names seeded first; see
    /// [`BindingIds`].
    binding_ids: BindingIds,
    /// Every widget tree the loaded cosmetic mods declared, in load order, already
    /// translated into that global id space.
    ui: Vec<UiRoot>,
    /// Whether a cosmetic mod has been adopted yet. The binding bridge interns as
    /// it goes, so a gameplay mod arriving *after* a cosmetic one would intern its
    /// names above whatever that cosmetic already claimed, landing every one of
    /// them on an id the server never assigned. Refused rather than shifted.
    cosmetics_loaded: bool,
}

impl ClientHost {
    /// A fresh host with no mods loaded.
    pub fn new() -> Result<Self> {
        Ok(Self {
            host: Host::new()?,
            vfs: Vfs::new(),
            visuals: BTreeMap::new(),
            effects: BTreeMap::new(),
            icons: BTreeMap::new(),
            unit_icons: BTreeMap::new(),
            cards: BTreeMap::new(),
            animations: BTreeMap::new(),
            notify_effects: BTreeMap::new(),
            unit_ids: Interner::new(),
            ability_ids: Interner::new(),
            talent_ids: Interner::new(),
            aiming: BTreeMap::new(),
            talent_trees: BTreeMap::new(),
            talent_focus: BTreeMap::new(),
            talent_quests: BTreeMap::new(),
            navmeshes: Vec::new(),
            binding_ids: BindingIds::new(),
            ui: Vec::new(),
            cosmetics_loaded: false,
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
        if self.cosmetics_loaded {
            bail!(
                "gameplay mod `{}` was loaded after a cosmetic one; \
                 every gameplay mod must be loaded first, in the server's own \
                 order, or the ids a HUD binds to shift out from under it",
                loaded.manifest.id
            );
        }
        let reg = self.host.register(&loaded.wasm)?;
        for name in &reg.names.units {
            self.unit_ids.intern(name);
        }
        for name in &reg.names.abilities {
            self.ability_ids.intern(name);
        }
        // Talents, for the one binding that yields a list of them (server#68).
        for name in &reg.names.talents {
            self.talent_ids.intern(name);
        }
        // The families a HUD binding names, interned in the same order adoption
        // interns them so the client's ids match the server's (server#67).
        self.binding_ids.adopt(&reg.names);
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
        // Each unit's talent tree, re-keyed local→global on both ends: the unit it
        // hangs on and every talent its tiers offer (server#69). Both mods author
        // from raw 0, so a tree adopted verbatim would have the second mod's tier
        // offering the first mod's talents.
        for (raw, unit) in reg.units.iter().enumerate() {
            let Some(tree) = &unit.talent_tree else { continue };
            let name = reg
                .names
                .units
                .get(raw)
                .ok_or_else(|| anyhow!("unit descriptor {raw} has no name-table entry"))?;
            let global =
                self.unit_ids.get(name).ok_or_else(|| anyhow!("unit `{name}` was not interned"))?;
            self.talent_trees.insert(global, self.globalize_tree(tree, &reg.names)?);
        }
        // Which of the caster's buttons each talent is about (server#129), keyed
        // and named in the global id space. A talent whose answer is not a single
        // button contributes nothing, so a HUD asking about one and getting nothing
        // back is being told the honest answer rather than being failed.
        for (raw, talent) in reg.talents.iter().enumerate() {
            if talent.changes().is_none() && talent.quest.is_none() {
                continue;
            }
            let name = reg
                .names
                .talents
                .get(raw)
                .ok_or_else(|| anyhow!("talent descriptor {raw} has no name-table entry"))?;
            let global = self
                .talent_ids
                .get(name)
                .ok_or_else(|| anyhow!("talent `{name}` was not interned"))?;
            if let Some(focus) = talent.changes() {
                self.talent_focus.insert(global, self.globalize_focus(focus, &reg.names)?);
            }
            // The task it sets, if it sets one (server#132). The counter **is**
            // re-keyed now that a client reads one: it names the same stack family
            // a HUD's own `Pool` binding names, and the binding bridge interns that
            // family in the server's own order — so the id here is the id the
            // owner's replicated counters arrive under.
            //
            // The prizes are deliberately dropped ([`QuestSpec::stripped`]). A client
            // never pays a task out; carrying `Impact` trees full of handles nothing
            // here has translated would be keeping ids that mean another mod's
            // content, waiting for somebody to read them. The *thresholds* stay —
            // they are what a bar is drawn against.
            if let Some(quest) = &talent.quest {
                let counter =
                    LocalToGlobal::new(&reg.names, &mut self.binding_ids).stack(quest.counter)?;
                self.talent_quests.insert(global, QuestSpec { counter, ..quest.stripped() });
            }
        }
        self.navmeshes.extend(reg.navmeshes.iter().cloned());
        Ok(())
    }

    /// One focus in the global id space.
    ///
    /// A slot crosses nothing — it is a position on the caster's bar and means the
    /// same thing in every mod, which is why it is the shape a HUD can act on at
    /// once. An ability is a local handle and has to be re-keyed, and one its own
    /// mod never named is a broken registration rather than a focus to drop: a
    /// talent silently about nothing is a plate an author cannot find the cause of.
    fn globalize_focus(&self, focus: AbilityFocus, names: &Names) -> Result<AbilityFocus> {
        match focus {
            AbilityFocus::Slot(slot) => Ok(AbilityFocus::Slot(slot)),
            AbilityFocus::Ability(local) => {
                let name = names.abilities.get(local.0 as usize).ok_or_else(|| {
                    anyhow!("talent is about ability handle {} with no name-table entry", local.0)
                })?;
                let global = self
                    .ability_ids
                    .get(name)
                    .ok_or_else(|| anyhow!("ability `{name}` was not interned"))?;
                Ok(AbilityFocus::Ability(global))
            }
        }
    }

    /// One tree with every option translated into the global talent id space.
    ///
    /// A tier offering a handle its own mod never named is a broken registration:
    /// reported rather than dropped, because a silently shortened tier is a talent
    /// panel with a hole in it that no author can see the cause of.
    fn globalize_tree(&self, tree: &TalentTree, names: &Names) -> Result<TalentTree> {
        let mut out = tree.clone();
        for tier in &mut out.tiers {
            for option in &mut tier.options {
                let name = names.talents.get(option.0 as usize).ok_or_else(|| {
                    anyhow!("talent tier offers handle {} with no name-table entry", option.0)
                })?;
                *option = self
                    .talent_ids
                    .get(name)
                    .ok_or_else(|| anyhow!("talent `{name}` was not interned"))?;
            }
        }
        Ok(out)
    }

    /// The talent tree declared for the unit with this **global** id, or `None`
    /// for a unit that declares none (a creep, a projectile) and for an id no
    /// loaded gameplay mod defines.
    ///
    /// `None` rather than an empty tree: a HUD that drew a talent panel for a unit
    /// with nothing to choose would be an empty panel the player cannot dismiss.
    #[must_use]
    pub fn talent_tree(&self, unit: UnitId) -> Option<&TalentTree> {
        self.talent_trees.get(&unit)
    }

    /// Every declared tree as `(global unit id, tree)` — how the client fills the
    /// table a `PickTalent` option index is resolved through.
    pub fn talent_trees(&self) -> impl Iterator<Item = (UnitId, &TalentTree)> {
        self.talent_trees.iter().map(|(id, tree)| (*id, tree))
    }

    /// The name the declaring gameplay mod gave the talent with this **global**
    /// id, or `None` for an id no loaded mod declared (server#68).
    ///
    /// `None` rather than a raw-index lookup on purpose: the ids arrive from the
    /// server, and a client whose mod list is a talent short would otherwise print
    /// a *neighbouring* talent's name — a label that is wrong rather than missing.
    #[must_use]
    pub fn talent_name(&self, id: TalentId) -> Option<&str> {
        self.talent_ids.resolve(id)
    }

    /// Which of the caster's abilities each talent changes, as `(global talent id,
    /// focus)` in id order — how the client fills the table a hotkey plate is drawn
    /// from (server#129).
    ///
    /// Only the talents where that question has a single answer appear. A talent
    /// selecting by tag, changing no ability, or granting two of them is absent,
    /// because naming one of its buttons would be a label that is confidently wrong.
    pub fn talent_focus(&self) -> impl Iterator<Item = (TalentId, AbilityFocus)> {
        self.talent_focus.iter().map(|(id, focus)| (*id, *focus))
    }

    /// Which talents set the player a task, as `(global talent id, spec)` in id
    /// order — how the client fills the table a quest mark is drawn from
    /// (server#132).
    ///
    /// Only the talents that declare one, which is almost none of them. The
    /// counter is already in the client's own id space and the reward is empty —
    /// see [`adopt_gameplay`](Self::adopt_gameplay).
    pub fn talent_quests(&self) -> impl Iterator<Item = (TalentId, &QuestSpec)> {
        self.talent_quests.iter().map(|(id, quest)| (*id, quest))
    }

    /// Every declared talent as `(global id, name)`, in interning order — how the
    /// client fills the table a chosen-talents list is printed through.
    pub fn talent_names(&self) -> impl Iterator<Item = (TalentId, &str)> {
        (0..self.talent_ids.len() as u32).filter_map(|raw| {
            let id = TalentId::from_raw(raw);
            self.talent_ids.resolve(id).map(|name| (id, name))
        })
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

    /// Every adopted ability icon keyed by the **global `AbilityId`** the wire
    /// carries, the picture an interface draws for whichever slot binds that
    /// ability (server#94). An icon for an ability no loaded gameplay mod defines
    /// is skipped, exactly like [`effects_by_id`](Self::effects_by_id) — the slot
    /// then wears whatever its HUD declared for an empty socket.
    pub fn icons_by_id(&self) -> impl Iterator<Item = (AbilityId, &str)> {
        self.icons
            .iter()
            .filter_map(|(name, image)| Some((self.ability_ids.get(name)?, image.as_str())))
    }

    /// Every adopted unit portrait keyed by the **global `UnitId`** the wire
    /// carries as [`UnitTag`](stormlight_shared::identity::UnitTag) and a roster row
    /// carries as its unit link (server#145). A portrait for a unit no loaded
    /// gameplay mod defines is skipped, exactly like
    /// [`visuals_by_id`](Self::visuals_by_id) — the row then wears whatever its HUD
    /// declared for an empty socket.
    pub fn unit_icons_by_id(&self) -> impl Iterator<Item = (UnitId, &str)> {
        self.unit_icons
            .iter()
            .filter_map(|(name, image)| Some((self.unit_ids.get(name)?, image.as_str())))
    }

    /// Every adopted talent card keyed by the **global `TalentId`** the tier tables
    /// carry, the words and picture a panel shows for whichever cell offers that
    /// talent (server#95). A card for a talent no loaded gameplay mod declares is
    /// skipped, exactly like [`icons_by_id`](Self::icons_by_id) — the cell then
    /// prints the interned identifier and wears the interface's own empty socket.
    pub fn cards_by_id(&self) -> impl Iterator<Item = (TalentId, &TalentInfo)> {
        self.cards.iter().filter_map(|(name, card)| Some((self.talent_ids.get(name)?, card)))
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
        let adopted = AdoptedVisuals::adopt(&reg, &loaded.manifest.id)?;
        // Merge into the combined tables; a later mod overrides an earlier entry.
        for (name, model) in adopted.iter() {
            self.visuals.insert(name.clone(), model.clone());
        }
        for (key, model) in adopted.effects() {
            self.effects.insert(key.clone(), model.clone());
        }
        for (ability, image) in adopted.icons() {
            self.icons.insert(ability.clone(), image.clone());
        }
        for (unit, image) in adopted.unit_icons() {
            self.unit_icons.insert(unit.clone(), image.clone());
        }
        for (talent, card) in adopted.cards() {
            self.cards.insert(talent.clone(), card.clone());
        }
        for (name, animation) in adopted.animations() {
            self.animations.insert(name.clone(), animation.clone());
        }
        // Keys are already package-qualified, so two mods never overwrite each
        // other here — the merge is a union, not a race.
        for (key, model) in adopted.named_effects() {
            self.notify_effects.insert(key.clone(), model.clone());
        }
        // Interfaces are appended, never merged: two mods each declaring a HUD
        // both get one, and a later mod cannot silently replace an earlier one's
        // (there is no key to collide on — see `AdoptedVisuals::ui`).
        let mut roots = adopted.ui().to_vec();
        {
            let map = LocalToGlobal::new(&reg.names, &mut self.binding_ids);
            roots.remap_ids(&map).map_err(|e| anyhow!("mod `{}`: {e}", loaded.manifest.id))?;
        }
        self.ui.extend(roots);
        self.cosmetics_loaded = true;
        self.vfs.insert(loaded.manifest.id, source);
        Ok(())
    }

    /// Every widget tree the loaded cosmetic mods declared, in load order, with
    /// every bound handle already translated into the global id space — what the
    /// client walks to build its interface (server#67). Empty when no cosmetic mod
    /// declares one, which is the content-free client showing no HUD at all.
    #[must_use]
    pub fn ui(&self) -> &[UiRoot] {
        &self.ui
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

    /// The animation a loaded cosmetic mod declared for the unit named
    /// `unit_name`, or `None` if none did (the unit then stands still).
    #[must_use]
    pub fn animation(&self, unit_name: &str) -> Option<&AnimationDescriptor> {
        self.animations.get(unit_name)
    }

    /// Every cosmetic effect an animation notify can spawn, keyed by the
    /// package-qualified name (server#76) — how the client fills the table its
    /// notify runtime resolves against. Unlike the unit and ability tables this
    /// needs no id bridge: the key never crosses the wire, and is resolved only
    /// against the animation that named it.
    pub fn named_effects(&self) -> impl Iterator<Item = (&String, &VisualModel)> {
        self.notify_effects.iter()
    }

    /// Every adopted animation keyed by the **global `UnitId`** the wire uses,
    /// resolved through the name→id map [`load_gameplay`](Self::load_gameplay)
    /// built — the same bridge [`visuals_by_id`](Self::visuals_by_id) crosses. An
    /// animation for a unit no loaded gameplay mod defines is skipped.
    pub fn animations_by_id(&self) -> impl Iterator<Item = (UnitId, &AnimationDescriptor)> {
        self.animations.iter().filter_map(|(name, anim)| Some((self.unit_ids.get(name)?, anim)))
    }
}
