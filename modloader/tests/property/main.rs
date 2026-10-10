//! stormlight_modloader tests -- property. Bolero only (property / fuzz / structural).
//!
//! One module per feature, one tiny file per "chih". As you add a feature,
//! drop a <feature>.rs beside this file and declare `mod <feature>;` here.
//! Keep invariants directional/structural, never magnitude-only.

mod client_ability_card_adopt;
mod client_adopt;
mod client_animation_adopt;
mod client_card_adopt;
mod client_decal_adopt;
mod client_effect_adopt;
mod client_effect_attach;
mod client_effect_life;
mod client_environment_adopt;
mod client_icon_adopt;
mod client_notify_adopt;
mod client_particles_adopt;
mod client_scenery_adopt;
mod client_shadow_adopt;
mod client_sound_adopt;
mod client_status_adopt;
mod client_ui_adopt;
mod client_unit_marks_adopt;
