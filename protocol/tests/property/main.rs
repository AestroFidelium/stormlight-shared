//! stormlight_api tests -- property. Bolero only (property / fuzz / structural).
//!
//! One module per feature, one tiny file per "chih". As you add a feature,
//! drop a <feature>.rs beside this file and declare `mod <feature>;` here.
//! Keep invariants directional/structural, never magnitude-only.

mod cast_intent;
mod cast_progress;
mod death;
mod impact;
mod lerp_transform;
mod move_intent;
mod mover;
mod orders;
mod pools;
mod progression;
mod projectile;
mod quantize;
mod roster;
mod slots;
mod talent_pick;
mod turn_rate;
mod ui_trigger;
mod vital_feed;
mod vitals;
