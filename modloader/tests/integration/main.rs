//! stormlight_modloader tests -- integration. Bolero only (property / fuzz / structural).
//!
//! One module per feature, one tiny file per "chih". As you add a feature,
//! drop a <feature>.rs beside this file and declare `mod <feature>;` here.
//! Keep invariants directional/structural, never magnitude-only.

mod client;
mod client_aiming;
mod client_id_bridge;
mod invoke;
mod loader;
mod register;
mod report;
mod sandbox;
mod vfs;
