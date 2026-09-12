//! `stormlight_api` — the shared crate.
//!
//! Single source of truth for the wire format (`connection`), its Lightyear
//! registration (`protocol`), balance constants/macros (`balance`), and the
//! declarative ECS-item macros (`support`). Pull everything from
//! [`prelude`].

pub mod balance;
pub mod cast;
pub mod connection;
pub mod death;
pub mod identity;
pub mod impact;
pub mod movement;
pub mod orders;
pub mod pools;
pub mod progression;
pub mod projectiles;
pub mod protocol;
pub mod quantize;
pub mod roster;
pub mod slots;
pub mod stacks;
pub mod stress;
pub mod swing;
pub mod talents;
pub mod tasks;
pub mod ui;
pub mod vital_feed;
pub mod vitals;

// `#[macro_export]` puts the macros at the crate root; the module still has to
// be declared so the file is compiled.
mod support;

pub mod prelude;
