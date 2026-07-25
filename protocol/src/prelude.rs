//! The one import every engine crate uses: `use stormlight_api::prelude::*;`.

// Declarative ECS-item macros (exported at crate root via `#[macro_export]`).
pub use crate::easy_components;
pub use crate::easy_resources;

// Re-export the protocol surface as it lands. (`balance` is re-exported here
// once it has public items; an empty glob would warn.)
// Selective (not a glob): `cast::register` would collide with
// `projectiles::register` under a glob re-export.
pub use crate::cast::{Aim, CastIntent, CastIntentChannel, CastProgress};
pub use crate::connection::*;
pub use crate::identity::UnitTag;
pub use crate::impact::{ImpactChannel, ImpactEvent};
pub use crate::movement::{
    MoveStep, advance_mover, angle_delta, step_toward, turn_toward, wrap_angle, yaw_to,
};
pub use crate::projectiles::*;
pub use crate::protocol::ProtocolPlugin;
