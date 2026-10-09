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
use stormlight_mod_abi::abilities::{Cost, Targeting};
use stormlight_mod_abi::animation::{AnimState, AnimationDescriptor};
use stormlight_mod_abi::attach::AttachPoint;
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::environment::Environment;
use stormlight_mod_abi::ids::{AbilityId, BuffId, Handle, ResourceId, TalentId, UnitId};
use stormlight_mod_abi::interner::Interner;
use stormlight_mod_abi::lifetime::EffectLifetime;
use stormlight_mod_abi::manifest::{ABI_VERSION, ModKind};
use stormlight_mod_abi::navmesh::NavMeshDescriptor;
use stormlight_mod_abi::notify::{NotifyAction, NotifyPoint};
use stormlight_mod_abi::remap::{IdMap, RemapIds};
use stormlight_mod_abi::scenery::{HeightField, SceneryPiece, SceneryPlacement};
use stormlight_mod_abi::status_visual::StatusLook;
use stormlight_mod_abi::talent_tree::TalentTree;
use stormlight_mod_abi::talents::AbilityFocus;
use stormlight_mod_abi::tasks::QuestSpec;
use stormlight_mod_abi::ui::UiRoot;
use stormlight_mod_abi::visuals::{
    CardInfo, ClientRegistration, EffectRole, NamedEffect, VisualModel,
};

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
    /// Where on its unit's rig each effect visual asked to be hung
    /// (stormlight/server#160), under the same `(ability name, role)` key as the
    /// visual — only for the visuals that asked.
    by_effect_attach: BTreeMap<(String, EffectRole), AttachPoint>,
    /// How long each effect visual lives (stormlight/server#167), under the same
    /// `(ability name, role)` key as the visual — for every visual, the default
    /// included, so a later declaration never inherits an earlier one's.
    by_effect_lifetime: BTreeMap<(String, EffectRole), EffectLifetime>,
    /// The icon each ability wears in an interface (server#94), keyed by the same
    /// ability **name** its feedback visuals are keyed by. A picture rather than a
    /// [`VisualModel`]: an icon is a flat asset in a widget.
    by_icon: BTreeMap<String, CardInfo>,
    /// The portrait each unit wears in an interface (server#145), keyed by the same
    /// unit **name** its model is keyed by. A flat picture rather than a
    /// [`VisualModel`], for the reason an ability icon is one: it lands in a widget.
    by_unit_icon: BTreeMap<String, String>,
    /// What each talent is called, does and looks like (server#95), keyed by the
    /// talent **name** — the same bridge the icons cross, one family along. Words
    /// rather than a picture: a panel addresses a cell by tier and option index, so
    /// it has nothing of its own to print there.
    by_card: BTreeMap<String, CardInfo>,
    by_animation: BTreeMap<String, AnimationDescriptor>,
    /// The effects an animation notify can spawn (server#76), keyed by the
    /// package-qualified name — which is also the `name` each one carries here.
    by_notify_key: BTreeMap<String, NamedEffect>,
    /// How each status this mod dresses is drawn (server#171), keyed by the buff
    /// **name** — the same bridge the abilities cross, one family along.
    by_status: BTreeMap<String, StatusLook>,
    /// The widget trees this mod declared (server#67), in **declaration order** —
    /// the only table here that is a list rather than a map. Nothing outside a
    /// tree names one, and the client draws every root it is given, so there is no
    /// key to collide over; the order is the author's own back-to-front ordering
    /// of the interface, and sorting them would reorder the HUD.
    ui: Vec<UiRoot>,
    /// The map scenery this mod declared, in declaration order. Like `ui`, a list:
    /// nothing looks a piece up, so there is no key to merge on.
    scenery: Vec<SceneryPiece>,
    /// The height this mod's map draws units at, if it declares one.
    ground: Option<HeightField>,
    /// The light this mod's map is seen in, if it declares one.
    environment: Option<Environment>,
}

/// Why a scenery placement cannot be drawn, if it cannot: a non-finite component
/// puts the piece nowhere (or everywhere), and an all-zero quaternion is not a
/// rotation at all.
fn unplaceable(p: &SceneryPlacement) -> Option<&'static str> {
    if p.translation.iter().chain(&p.rotation).chain(&p.scale).any(|v| !v.is_finite()) {
        return Some("a non-finite component");
    }
    if p.rotation.iter().all(|c| *c == 0.0) {
        return Some("a zero rotation");
    }
    None
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
            // A model no renderer can draw is refused here, naming the unit, for the
            // reason scenery placements are (server#179, server#170).
            if !v.model.is_drawable() {
                bail!("unit visual `{name}`: a shadow or decal no renderer can draw");
            }
            by_name.insert(name.clone(), v.model.clone());
        }
        let mut by_effect = BTreeMap::new();
        let mut by_effect_attach = BTreeMap::new();
        let mut by_effect_lifetime = BTreeMap::new();
        for e in &reg.effects {
            let name = reg.names.abilities.get(e.ability.0 as usize).ok_or_else(|| {
                anyhow!(
                    "effect visual references ability handle {} with no name-table entry",
                    e.ability.0
                )
            })?;
            // Refused here rather than handed on: a burst that retires on the frame
            // it appears is never seen, and one that never retires is a leak per hit.
            if !e.lifetime.is_runnable() {
                bail!("effect visual `{name}` {:?}: a lifetime no clock can run", e.role);
            }
            if !e.model.is_drawable() {
                bail!(
                    "effect visual `{name}` {:?}: a shadow or decal no renderer can draw",
                    e.role
                );
            }
            by_effect.insert((name.clone(), e.role), e.model.clone());
            by_effect_lifetime.insert((name.clone(), e.role), e.lifetime);
            // Replaced or cleared with the visual, so a later declaration of the
            // same key never inherits an earlier one's point.
            match &e.attach {
                Some(point) => by_effect_attach.insert((name.clone(), e.role), point.clone()),
                None => by_effect_attach.remove(&(name.clone(), e.role)),
            };
        }
        let mut by_icon = BTreeMap::new();
        for card in &reg.ability_cards {
            let name = reg.names.abilities.get(card.ability.0 as usize).ok_or_else(|| {
                anyhow!(
                    "ability card references ability handle {} with no name-table entry",
                    card.ability.0
                )
            })?;
            by_icon.insert(name.clone(), card.info.clone());
        }
        let mut by_status = BTreeMap::new();
        for status in &reg.status_visuals {
            let name = reg.names.buffs.get(usize::from(status.buff.0)).ok_or_else(|| {
                anyhow!(
                    "status visual references buff handle {} with no name-table entry",
                    status.buff.0
                )
            })?;
            // Refused rather than carried: a look nobody can see is a declaration the
            // renderer would honour by drawing nothing, forever, and saying nothing.
            if !status.look.shows_anything() {
                bail!("status visual `{name}` shows nothing to anyone");
            }
            let undrawable = [&status.look.own, &status.look.others]
                .into_iter()
                .flatten()
                .any(|model| !model.is_drawable());
            if undrawable {
                bail!("status visual `{name}`: a shadow or decal no renderer can draw");
            }
            by_status.insert(name.clone(), status.look.clone());
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
        let mut by_notify_key = BTreeMap::new();
        for e in &reg.named_effects {
            if !e.lifetime.is_runnable() {
                bail!("notify effect `{}`: a lifetime no clock can run", e.name);
            }
            if !e.model.is_drawable() {
                bail!("notify effect `{}`: a shadow or decal no renderer can draw", e.name);
            }
            let key = qualify(mod_id, &e.name);
            by_notify_key.insert(key.clone(), NamedEffect { name: key, ..e.clone() });
        }
        // Structural validation of the interface, for the same reason animations
        // are validated here: past this point the tree reaches a renderer that
        // would draw a blank rectangle and explain nothing.
        for root in &reg.ui {
            root.validate().map_err(|e| anyhow!("ui root `{}`: {e}", root.name))?;
        }
        if let Some(ground) = &reg.ground
            && !ground.is_valid()
        {
            bail!("ground height field is unusable (size, spacing or a non-finite sample)");
        }
        if let Some(environment) = &reg.environment
            && !environment.is_valid()
        {
            bail!(
                "environment is unusable (a non-finite or negative colour, a zero direction, or a non-positive exposure)"
            );
        }
        // Scenery is checked here for the same reason: a placement that cannot be
        // drawn is reported with its asset, or it is silently drawn nowhere.
        for piece in &reg.scenery {
            for (i, p) in piece.placements.iter().enumerate() {
                if let Some(why) = unplaceable(p) {
                    bail!("scenery `{}` placement {i} has {why}", piece.asset);
                }
            }
        }
        Ok(Self {
            by_name,
            by_effect,
            by_effect_attach,
            by_effect_lifetime,
            by_icon,
            by_unit_icon,
            by_card,
            by_animation,
            by_notify_key,
            by_status,
            ui: reg.ui.clone(),
            scenery: reg.scenery.clone(),
            ground: reg.ground.clone(),
            environment: reg.environment.clone(),
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

    /// Where the feedback visual for `ability_name` in `role` asked to be hung on
    /// its unit's rig (stormlight/server#160), if it asked.
    #[must_use]
    pub fn effect_attach(&self, ability_name: &str, role: EffectRole) -> Option<&AttachPoint> {
        self.by_effect_attach.get(&(ability_name.to_string(), role))
    }

    /// How long the feedback visual for `ability_name` in `role` lives
    /// (stormlight/server#167) — the default for a visual that never said, or for
    /// one that was never declared.
    #[must_use]
    pub fn effect_lifetime(&self, ability_name: &str, role: EffectRole) -> EffectLifetime {
        self.by_effect_lifetime.get(&(ability_name.to_string(), role)).copied().unwrap_or_default()
    }

    /// The picture declared for `ability_name`, if its card has one — what an
    /// interface draws in whichever slot binds that ability (server#94).
    #[must_use]
    pub fn icon(&self, ability_name: &str) -> Option<&str> {
        self.by_icon
            .get(ability_name)
            .map(|card| card.image.as_str())
            .filter(|image| !image.is_empty())
    }

    /// The card declared for `ability_name`, if any — its name, words and picture
    /// (server#116).
    #[must_use]
    pub fn ability_card(&self, ability_name: &str) -> Option<&CardInfo> {
        self.by_icon.get(ability_name)
    }

    /// Iterate the `(ability name, card)` pairs, in name order.
    pub fn ability_cards(&self) -> impl Iterator<Item = (&String, &CardInfo)> {
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
    pub fn card(&self, talent_name: &str) -> Option<&CardInfo> {
        self.by_card.get(talent_name)
    }

    /// Iterate the `(talent name, card)` pairs, in name order.
    pub fn cards(&self) -> impl Iterator<Item = (&String, &CardInfo)> {
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
    pub fn named_effect(&self, key: &str) -> Option<&NamedEffect> {
        self.by_notify_key.get(key)
    }

    /// Iterate the `(qualified key, effect)` pairs, in key order.
    pub fn named_effects(&self) -> impl Iterator<Item = (&String, &NamedEffect)> {
        self.by_notify_key.iter()
    }

    /// Iterate the `(unit name, animation)` pairs, in unit-name order.
    pub fn animations(&self) -> impl Iterator<Item = (&String, &AnimationDescriptor)> {
        self.by_animation.iter()
    }

    /// How the status the buff named `buff` grants is drawn (server#171), if this
    /// mod dresses it.
    #[must_use]
    pub fn status_look(&self, buff: &str) -> Option<&StatusLook> {
        self.by_status.get(buff)
    }

    /// Iterate the `(buff name, look)` pairs, in buff-name order.
    pub fn status_looks(&self) -> impl Iterator<Item = (&String, &StatusLook)> {
        self.by_status.iter()
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
            && self.by_status.is_empty()
            && self.ui.is_empty()
            && self.scenery.is_empty()
            && self.ground.is_none()
            && self.environment.is_none()
    }

    /// The light this mod's map is seen in, if it declares one.
    #[must_use]
    pub fn environment(&self) -> Option<&Environment> {
        self.environment.as_ref()
    }

    /// The height this mod's map draws units at, if it declares one.
    #[must_use]
    pub fn ground(&self) -> Option<&HeightField> {
        self.ground.as_ref()
    }

    /// The map scenery this mod declared, in declaration order.
    #[must_use]
    pub fn scenery(&self) -> &[SceneryPiece] {
        &self.scenery
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

    /// Iterate the `((ability name, role), point)` pairs of the visuals that asked
    /// to hang on a point, in key order.
    pub fn effect_attaches(&self) -> impl Iterator<Item = (&(String, EffectRole), &AttachPoint)> {
        self.by_effect_attach.iter()
    }

    /// Iterate the `((ability name, role), lifetime)` pairs of every effect visual,
    /// in key order.
    pub fn effect_lifetimes(
        &self,
    ) -> impl Iterator<Item = (&(String, EffectRole), &EffectLifetime)> {
        self.by_effect_lifetime.iter()
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
    /// Where each effect visual asked to hang on its unit's rig (server#160),
    /// merged across mods under the visual's own key.
    effect_attach: BTreeMap<(String, EffectRole), AttachPoint>,
    /// How long each effect visual lives (server#167), merged with the visual.
    effect_lifetime: BTreeMap<(String, EffectRole), EffectLifetime>,
    /// Each ability's interface icon, keyed by ability name until
    /// [`icons_by_id`](ClientHost::icons_by_id) crosses the same name→global-id
    /// bridge the visuals cross (server#94).
    icons: BTreeMap<String, CardInfo>,
    /// Each unit's portrait, keyed by unit name until
    /// [`unit_icons_by_id`](ClientHost::unit_icons_by_id) crosses the same
    /// name→global-id bridge the visuals cross (server#145).
    unit_icons: BTreeMap<String, String>,
    /// Each talent's card, keyed by talent name until
    /// [`cards_by_id`](ClientHost::cards_by_id) crosses the same name→global-id
    /// bridge the icons cross (server#95).
    cards: BTreeMap<String, CardInfo>,
    animations: BTreeMap<String, AnimationDescriptor>,
    /// Cosmetic effects an animation notify spawns (server#76), keyed by the
    /// package-qualified name their declaring mod gave them.
    notify_effects: BTreeMap<String, NamedEffect>,
    /// How each dressed status is drawn (server#171), keyed by buff name until
    /// [`status_looks_by_id`](ClientHost::status_looks_by_id) crosses the bridge.
    statuses: BTreeMap<String, StatusLook>,
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
    /// Buffs, for the statuses a unit carries on the wire (server#171).
    buff_ids: Interner<BuffId>,
    /// How each gameplay ability is aimed, keyed by the **global `AbilityId`**
    /// (server#59). The client cannot decide an aim mode locally — it is mod data
    /// like everything else — so the same gameplay pass that rebuilds the id map
    /// records the declared [`Targeting`] alongside it, and the client resolves a
    /// keypress into an `Aim` of exactly that shape.
    aiming: BTreeMap<AbilityId, Targeting>,
    /// Which global resource each ability pays its cost in (server#210).
    costs: BTreeMap<AbilityId, ResourceId>,
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
    /// The tasks each unit carries of its own, with nothing chosen
    /// (stormlight/server#139). Declaration order, because that order is the task's
    /// identity. Only the units that declare one, which is almost none of them.
    unit_tasks: BTreeMap<UnitId, Vec<QuestSpec>>,
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
    /// Every piece of map scenery the loaded cosmetic mods declared, appended in
    /// load order. Carries no handle, so nothing needs translating.
    scenery: Vec<SceneryPiece>,
    /// The drawn ground height the loaded cosmetic mods declared; the last one
    /// loaded wins, like any other map-wide setting.
    ground: Option<HeightField>,
    /// The map environment the loaded cosmetic mods declared; the last one loaded
    /// wins, like the ground.
    environment: Option<Environment>,
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
            effect_attach: BTreeMap::new(),
            effect_lifetime: BTreeMap::new(),
            icons: BTreeMap::new(),
            unit_icons: BTreeMap::new(),
            cards: BTreeMap::new(),
            animations: BTreeMap::new(),
            notify_effects: BTreeMap::new(),
            statuses: BTreeMap::new(),
            unit_ids: Interner::new(),
            ability_ids: Interner::new(),
            talent_ids: Interner::new(),
            buff_ids: Interner::new(),
            aiming: BTreeMap::new(),
            costs: BTreeMap::new(),
            talent_trees: BTreeMap::new(),
            talent_focus: BTreeMap::new(),
            talent_quests: BTreeMap::new(),
            unit_tasks: BTreeMap::new(),
            navmeshes: Vec::new(),
            binding_ids: BindingIds::new(),
            ui: Vec::new(),
            scenery: Vec::new(),
            ground: None,
            environment: None,
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
        // Buffs, for the statuses a unit carries on the wire (server#171).
        for name in &reg.names.buffs {
            self.buff_ids.intern(name);
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
            // The resource it costs, re-keyed the same way: what an interface reading
            // "the resource this ability costs" shows (server#210).
            let pays = ability.cost.iter().find_map(|cost| match cost {
                Cost::Resource { res, .. } => Some(*res),
                Cost::Charge | Cost::Health { .. } => None,
            });
            if let Some(res) = pays {
                let res =
                    LocalToGlobal::new(&reg.names, &mut self.binding_ids).resource(res).map_err(
                        |_| anyhow!("ability `{name}` costs a resource with no name-table entry"),
                    )?;
                self.costs.insert(global, res);
            }
        }
        // Each unit's talent tree, re-keyed local→global on both ends: the unit it
        // hangs on and every talent its tiers offer (server#69). Both mods author
        // from raw 0, so a tree adopted verbatim would have the second mod's tier
        // offering the first mod's talents.
        for (raw, unit) in reg.units.iter().enumerate() {
            if unit.talent_tree.is_none() && unit.tasks.is_empty() {
                continue;
            }
            let name = reg
                .names
                .units
                .get(raw)
                .ok_or_else(|| anyhow!("unit descriptor {raw} has no name-table entry"))?;
            let global =
                self.unit_ids.get(name).ok_or_else(|| anyhow!("unit `{name}` was not interned"))?;
            if let Some(tree) = &unit.talent_tree {
                self.talent_trees.insert(global, self.globalize_tree(tree, &reg.names)?);
            }
            // The unit's *own* tasks, with nothing chosen (server#139). Kept in
            // declaration order, because that order is the task's identity — it is
            // what the latch is keyed by and what a widget addresses one with.
            //
            // Stripped, like a talent's: a client never pays a task out. The counter
            // is re-keyed for the same reason a talent's is — it names the stack
            // family a HUD's own `Pool` binding names.
            if !unit.tasks.is_empty() {
                let mut tasks = Vec::with_capacity(unit.tasks.len());
                for task in &unit.tasks {
                    let counter = LocalToGlobal::new(&reg.names, &mut self.binding_ids)
                        .stack(task.counter)?;
                    tasks.push(QuestSpec { counter, ..task.stripped() });
                }
                self.unit_tasks.insert(global, tasks);
            }
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

    /// The tasks each unit carries of its own, in global id order
    /// (stormlight/server#139).
    ///
    /// Declaration order within a unit, because that order is the only key such a
    /// task has: it is what the simulation latches by and what a widget addresses
    /// one with.
    pub fn unit_tasks(&self) -> impl Iterator<Item = (UnitId, &[QuestSpec])> {
        self.unit_tasks.iter().map(|(id, tasks)| (*id, tasks.as_slice()))
    }

    /// Every declared talent as `(global id, name)`, in interning order — how the
    /// client fills the table a chosen-talents list is printed through.
    pub fn talent_names(&self) -> impl Iterator<Item = (TalentId, &str)> {
        (0..self.talent_ids.len() as u32).filter_map(|raw| {
            let id = TalentId::from_raw(raw);
            self.talent_ids.resolve(id).map(|name| (id, name))
        })
    }

    /// Every gameplay ability's interned name, keyed by its **global `AbilityId`** —
    /// what an ability whose mod carded no name prints (server#116), exactly as an
    /// uncarded talent prints its own.
    pub fn ability_names(&self) -> impl Iterator<Item = (AbilityId, &str)> {
        (0..self.ability_ids.len() as u32).filter_map(|raw| {
            let id = AbilityId::from_raw(raw);
            self.ability_ids.resolve(id).map(|name| (id, name))
        })
    }

    /// Every gameplay ability's declared aim mode, keyed by the **global
    /// `AbilityId`** the wire carries — how the client learns what kind of `Aim`
    /// to build when a slot's key is pressed.
    pub fn aiming_by_id(&self) -> impl Iterator<Item = (AbilityId, &Targeting)> {
        self.aiming.iter().map(|(id, mode)| (*id, mode))
    }

    /// Every gameplay ability's cost resource, both keyed by **global** id — the
    /// resource an interface binding of `PoolRef::AbilityCost` reads. An ability
    /// that costs no resource has no entry (stormlight/server#210).
    pub fn cost_resources_by_id(&self) -> impl Iterator<Item = (AbilityId, ResourceId)> + '_ {
        self.costs.iter().map(|(ability, res)| (*ability, *res))
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

    /// Where each adopted effect visual asked to hang on its unit's rig
    /// (stormlight/server#160), keyed exactly as [`effects_by_id`](Self::effects_by_id)
    /// keys the visuals — and skipped for the same abilities.
    pub fn effect_attach_by_id(
        &self,
    ) -> impl Iterator<Item = ((AbilityId, EffectRole), &AttachPoint)> {
        self.effect_attach
            .iter()
            .filter_map(|((name, role), point)| Some(((self.ability_ids.get(name)?, *role), point)))
    }

    /// How long each adopted effect visual lives (stormlight/server#167), keyed
    /// exactly as [`effects_by_id`](Self::effects_by_id) keys the visuals — and
    /// skipped for the same abilities.
    pub fn effect_lifetime_by_id(
        &self,
    ) -> impl Iterator<Item = ((AbilityId, EffectRole), EffectLifetime)> + '_ {
        self.effect_lifetime.iter().filter_map(|((name, role), lifetime)| {
            Some(((self.ability_ids.get(name)?, *role), *lifetime))
        })
    }

    /// Every adopted status look keyed by the **global `BuffId`** a unit's
    /// statuses carry on the wire (server#171). A look for a buff no loaded gameplay
    /// mod declares is skipped, exactly like [`effects_by_id`](Self::effects_by_id).
    pub fn status_looks_by_id(&self) -> impl Iterator<Item = (BuffId, &StatusLook)> {
        self.statuses.iter().filter_map(|(name, look)| Some((self.buff_ids.get(name)?, look)))
    }

    /// Every adopted ability icon keyed by the **global `AbilityId`** the wire
    /// carries, the picture an interface draws for whichever slot binds that
    /// ability (server#94). An icon for an ability no loaded gameplay mod defines
    /// is skipped, exactly like [`effects_by_id`](Self::effects_by_id) — the slot
    /// then wears whatever its HUD declared for an empty socket.
    pub fn icons_by_id(&self) -> impl Iterator<Item = (AbilityId, &str)> {
        self.ability_cards_by_id()
            .map(|(ability, card)| (ability, card.image.as_str()))
            .filter(|(_, image)| !image.is_empty())
    }

    /// Every adopted ability card keyed by the **global `AbilityId`**: what a HUD
    /// prints for whichever slot binds that ability (server#116). Skipped for the
    /// abilities no loaded gameplay mod defines, like the icons.
    pub fn ability_cards_by_id(&self) -> impl Iterator<Item = (AbilityId, &CardInfo)> {
        self.icons.iter().filter_map(|(name, card)| Some((self.ability_ids.get(name)?, card)))
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
    pub fn cards_by_id(&self) -> impl Iterator<Item = (TalentId, &CardInfo)> {
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
            // A later mod's visual replaces the earlier one whole, point and all.
            match adopted.effect_attach(&key.0, key.1) {
                Some(point) => self.effect_attach.insert(key.clone(), point.clone()),
                None => self.effect_attach.remove(key),
            };
            self.effect_lifetime.insert(key.clone(), adopted.effect_lifetime(&key.0, key.1));
        }
        for (ability, card) in adopted.ability_cards() {
            self.icons.insert(ability.clone(), card.clone());
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
        for (key, effect) in adopted.named_effects() {
            self.notify_effects.insert(key.clone(), effect.clone());
        }
        // A later mod's look for a status replaces an earlier one whole.
        for (buff, look) in adopted.status_looks() {
            self.statuses.insert(buff.clone(), look.clone());
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
        self.scenery.extend(adopted.scenery().iter().cloned());
        if let Some(ground) = adopted.ground() {
            self.ground = Some(ground.clone());
        }
        if let Some(environment) = adopted.environment() {
            self.environment = Some(environment.clone());
        }
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

    /// Every piece of map scenery the loaded cosmetic mods declared, in load order
    /// and, within a mod, declaration order. Empty when no loaded mod dresses a
    /// map, which is the content-free client drawing only the map's geometry.
    #[must_use]
    pub fn scenery(&self) -> &[SceneryPiece] {
        &self.scenery
    }

    /// The drawn ground height the loaded cosmetic mods declared, if any.
    #[must_use]
    pub fn ground(&self) -> Option<&HeightField> {
        self.ground.as_ref()
    }

    /// The map environment the loaded cosmetic mods declared, if any.
    #[must_use]
    pub fn environment(&self) -> Option<&Environment> {
        self.environment.as_ref()
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
    pub fn named_effects(&self) -> impl Iterator<Item = (&String, &NamedEffect)> {
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
