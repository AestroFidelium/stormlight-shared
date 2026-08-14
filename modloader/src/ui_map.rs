//! The client-side id space a HUD's bindings are resolved into
//! (stormlight/server#67).
//!
//! A cosmetic mod binds a bar to `"power"` and authors it as its own local
//! [`StatId`]; the client resolves that bar against state keyed by the **global**
//! id the gameplay side interned `"power"` at. The unit and ability families
//! already cross that bridge in [`crate::client`] (server#44) — a HUD reaches three
//! more: stats, resource pools and stack counters.
//!
//! Two rules make the reconstruction match the server's:
//!
//! 1. **Seed the reserved names first.** The engine reserves the low stat ids for
//!    the stats it reads generically ([`stats::RESERVED`]), so a client that
//!    interned a mod's first stat at 0 would be off by the whole reserved block.
//!    Resources and stacks reserve nothing and start empty.
//! 2. **Intern in gameplay-load order**, exactly as adoption does, and intern
//!    rather than look up. A cosmetic naming something no gameplay mod declared
//!    lands on a fresh id at the end — an id no unit's state is ever keyed by — so
//!    the bar reads nothing. Looking it up instead would leave the local handle in
//!    place, and a local `StatId(0)` is the *reserved* movement speed: a bar
//!    confidently showing an unrelated number, which is worse than showing none.

use core::cell::RefCell;
use core::fmt;

use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::{
    AbilityId, AnimStateId, BuffId, CurveId, DamageTypeId, EventId, HandlerId, NavMeshId, ParamId,
    ResourceId, StackId, StatId, TagClassId, TagId, TalentId, UnitId,
};
use stormlight_mod_abi::interner::Interner;
use stormlight_mod_abi::remap::IdMap;
use stormlight_mod_abi::stats;

/// The global id space the client rebuilds for the families a widget can name.
/// Seeded with the reserved stat names, then grown by each gameplay mod's `Names`
/// in load order.
pub struct BindingIds {
    stats: Interner<StatId>,
    resources: Interner<ResourceId>,
    stacks: Interner<StackId>,
    /// The family a widget's *action* names rather than its binding
    /// (stormlight/server#69): a `UiAction::Trigger` raises a mod-defined event,
    /// and which global id that is was decided by the gameplay side. Reserves
    /// nothing and so starts empty, like resources and stacks.
    events: Interner<EventId>,
}

impl Default for BindingIds {
    fn default() -> Self {
        let mut ids = Self {
            stats: Interner::new(),
            resources: Interner::new(),
            stacks: Interner::new(),
            events: Interner::new(),
        };
        for name in stats::RESERVED {
            ids.stats.intern(name);
        }
        ids
    }
}

impl BindingIds {
    /// An id space holding nothing but the reserved names.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern one gameplay mod's stat / resource / stack names, in declaration
    /// order — the same walk server adoption performs, so the ids come out equal.
    pub fn adopt(&mut self, names: &Names) {
        for name in &names.stats {
            self.stats.intern(name);
        }
        for name in &names.resources {
            self.resources.intern(name);
        }
        for name in &names.stacks {
            self.stacks.intern(name);
        }
        for name in &names.events {
            self.events.intern(name);
        }
    }
}

/// A local handle a bundle's own name tables do not explain — a registration that
/// contradicts itself, reported rather than resolved to whatever sits at that raw
/// index in the global space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dangling {
    /// Which family the handle belongs to.
    pub family: &'static str,
    /// The raw local index it carried.
    pub raw: u32,
}

impl fmt::Display for Dangling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ui binding references {} handle {} with no name-table entry",
            self.family, self.raw
        )
    }
}

impl std::error::Error for Dangling {}

/// One cosmetic bundle's local→global map: resolve the local handle to the name
/// that bundle gave it, then intern that name into the shared id space.
///
/// Four families translate: the three a
/// [`ValueBinding`](stormlight_mod_abi::ui::ValueBinding) reaches, plus the events
/// a [`UiAction::Trigger`](stormlight_mod_abi::ui::UiAction::Trigger) raises
/// (server#69). Every other method is the identity, because no other handle is
/// reachable from a widget tree — a `Slot` is mod convention and a talent option
/// is an index into the unit's own tree, neither of them interned — and inventing
/// a translation for one would be inventing a mapping nothing feeds.
pub struct LocalToGlobal<'a> {
    names: &'a Names,
    ids: RefCell<&'a mut BindingIds>,
}

impl<'a> LocalToGlobal<'a> {
    /// Map `names` (the bundle's own tables) into `ids` (the shared space).
    pub fn new(names: &'a Names, ids: &'a mut BindingIds) -> Self {
        Self { names, ids: RefCell::new(ids) }
    }
}

impl IdMap for LocalToGlobal<'_> {
    type Error = Dangling;

    fn stat(&self, id: StatId) -> Result<StatId, Dangling> {
        let name = self
            .names
            .stats
            .get(id.0 as usize)
            .ok_or(Dangling { family: "stat", raw: u32::from(id.0) })?;
        Ok(self.ids.borrow_mut().stats.intern(name))
    }
    fn resource(&self, id: ResourceId) -> Result<ResourceId, Dangling> {
        let name = self
            .names
            .resources
            .get(id.0 as usize)
            .ok_or(Dangling { family: "resource", raw: u32::from(id.0) })?;
        Ok(self.ids.borrow_mut().resources.intern(name))
    }
    fn stack(&self, id: StackId) -> Result<StackId, Dangling> {
        let name = self
            .names
            .stacks
            .get(id.0 as usize)
            .ok_or(Dangling { family: "stack", raw: u32::from(id.0) })?;
        Ok(self.ids.borrow_mut().stacks.intern(name))
    }

    fn tag(&self, id: TagId) -> Result<TagId, Dangling> {
        Ok(id)
    }
    fn tag_class(&self, id: TagClassId) -> Result<TagClassId, Dangling> {
        Ok(id)
    }
    fn param(&self, id: ParamId) -> Result<ParamId, Dangling> {
        Ok(id)
    }
    fn event(&self, id: EventId) -> Result<EventId, Dangling> {
        let name = self
            .names
            .events
            .get(id.0 as usize)
            .ok_or(Dangling { family: "event", raw: u32::from(id.0) })?;
        Ok(self.ids.borrow_mut().events.intern(name))
    }
    fn buff(&self, id: BuffId) -> Result<BuffId, Dangling> {
        Ok(id)
    }
    fn curve(&self, id: CurveId) -> Result<CurveId, Dangling> {
        Ok(id)
    }
    fn damage_type(&self, id: DamageTypeId) -> Result<DamageTypeId, Dangling> {
        Ok(id)
    }
    fn ability(&self, id: AbilityId) -> Result<AbilityId, Dangling> {
        Ok(id)
    }
    fn talent(&self, id: TalentId) -> Result<TalentId, Dangling> {
        Ok(id)
    }
    fn handler(&self, id: HandlerId) -> Result<HandlerId, Dangling> {
        Ok(id)
    }
    fn unit(&self, id: UnitId) -> Result<UnitId, Dangling> {
        Ok(id)
    }
    fn navmesh(&self, id: NavMeshId) -> Result<NavMeshId, Dangling> {
        Ok(id)
    }
    fn anim_state(&self, id: AnimStateId) -> Result<AnimStateId, Dangling> {
        Ok(id)
    }
}
