//! stormlight_modloader tests -- property. Bolero only (property / fuzz / structural).
//!
//! One module per feature, one tiny file per "chih". As you add a feature,
//! drop a <feature>.rs beside this file and declare `mod <feature>;` here.
//! Keep invariants directional/structural, never magnitude-only.

mod client_adopt;
mod client_animation_adopt;
mod client_effect_adopt;
mod client_notify_adopt;
mod client_ui_adopt;
