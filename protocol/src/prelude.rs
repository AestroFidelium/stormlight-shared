//! The one import every engine crate uses: `use stormlight_api::prelude::*;`.

// Declarative ECS-item macros (exported at crate root via `#[macro_export]`).
pub use crate::easy_components;
pub use crate::easy_resources;

// Re-export the protocol surface as it lands. (`balance` is re-exported here
// once it has public items; an empty glob would warn.)
pub use crate::connection::*;
pub use crate::projectiles::*;
pub use crate::protocol::ProtocolPlugin;
