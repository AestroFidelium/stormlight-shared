//! `stormlight_api` — the shared crate.
//!
//! Single source of truth for the wire format (`connection`), its Lightyear
//! registration (`protocol`), balance constants/macros (`balance`), and the
//! declarative ECS-item macros (`support`). Pull everything from
//! [`prelude`].

pub mod balance;
pub mod bodies;
pub mod body_radius;
pub mod cast;
pub mod connection;
pub mod death;
pub mod engaged;
pub mod held;
pub mod identity;
pub mod impact;
pub mod motion;
pub mod movement;
pub mod orders;
pub mod pace_log;
pub mod pools;
pub mod prediction_lead;
pub mod progression;
pub mod projectiles;
pub mod protocol;
pub mod quantize;
pub mod roster;
pub mod slots;
pub mod stacks;
pub mod statuses;
pub mod stress;
pub mod swing;
pub mod talents;
pub mod task_payout;
pub mod tasks;
pub mod team;
pub mod time_field;
pub mod time_scale;
pub mod ui;
pub mod vital_feed;
pub mod vitals;

// `#[macro_export]` puts the macros at the crate root; the module still has to
// be declared so the file is compiled.
mod support;

pub mod prelude;
