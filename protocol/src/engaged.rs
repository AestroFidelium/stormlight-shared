//! The unit a player's own unit is attacking (stormlight/server#181).
//!
//! A player keeps track of which of several similar units their orders are
//! landing on by the ring drawn under it. That unit is the server's to decide: an
//! attack order names one, but an attack-move, a hold, or an idle unit defending
//! itself picks one up on its own, and only the attack cycle knows which. So the
//! server publishes it — on the owner's own view entity, beside their slots and
//! talents, because nobody else needs to know whom this player is hitting.
//!
//! # A mapped reference, resolved once
//!
//! The target is an entity, mapped into the receiver's world on arrival. Lightyear
//! resolves a mapped reference once and never retries, so a target that becomes
//! visible on the very tick it is engaged would arrive unresolved. A target is in
//! attack range of the player's unit and so, in practice, long since replicated;
//! the client reads an unresolved one as "no target" rather than marking a stray
//! entity, and the next change of target corrects it.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The unit the owner's unit is attacking, if any.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngagedTarget(pub Option<Entity>);

impl MapEntities for EngagedTarget {
    fn map_entities<M: EntityMapper>(&mut self, entity_map: &mut M) {
        // Nobody engaged stays nobody: a mapping must never invent a target.
        if let Some(target) = &mut self.0 {
            *target = entity_map.get_mapped(*target);
        }
    }
}

/// Register the replicated engaged target, mapped into the receiver's world.
pub fn register(app: &mut App) {
    app.register_component::<EngagedTarget>().add_map_entities();
}
