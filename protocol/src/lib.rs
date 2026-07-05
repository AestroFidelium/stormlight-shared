//! `stormlight_api` — the shared crate.
//!
//! Single source of truth for the wire format (`connection`), its Lightyear
//! registration (`protocol`), balance constants/macros (`balance`), and the
//! declarative ECS-item macros (`support`). Pull everything from
//! [`prelude`].

pub mod balance;
pub mod connection;
pub mod projectiles;
pub mod protocol;
pub mod stress;

// `#[macro_export]` puts the macros at the crate root; the module still has to
// be declared so the file is compiled.
mod support;

pub mod prelude;
