//! Invariants of environment adoption — where a map's cosmetic half hands over
//! the light it is seen in:
//!   - **A broken environment is refused, never drawn**: adoption errs exactly
//!     when the environment is invalid, and never panics. Past adoption a NaN
//!     colour or a zero direction reaches a shader and blacks out the map.
//!   - **Kept as declared**: a valid one comes back unchanged.
//!   - **An environment alone is content**: a bundle that only lights a map is
//!     not "empty".

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::environment::{Environment, SunLight};
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::ClientRegistration;
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    color: [i8; 3],
    direction: [i8; 3],
    exposure: i8,
    nan_ambient: bool,
}

fn build(s: &Scenario) -> ClientRegistration {
    let mut ambient = [0.4, 0.5, 0.7];
    if s.nan_ambient {
        ambient[0] = f32::NAN;
    }
    ClientRegistration {
        abi: ABI_VERSION,
        environment: Some(Environment {
            ambient,
            lights: vec![SunLight {
                color: s.color.map(|v| f32::from(v) / 64.0),
                direction: s.direction.map(f32::from),
            }],
            shadow: None,
            exposure: f32::from(s.exposure) / 32.0,
            backdrop: [0.0; 3],
            bloom: None,
        }),
        ..ClientRegistration::default()
    }
}

#[test]
fn a_broken_environment_is_refused_and_a_sound_one_kept() {
    check!().with_type::<Scenario>().for_each(|s| {
        let reg = build(s);
        let env = reg.environment.as_ref().expect("built with one");
        let adopted = AdoptedVisuals::adopt(&reg, "map");
        assert_eq!(adopted.is_err(), !env.is_valid(), "adoption disagrees with validity");
        if let Ok(adopted) = adopted {
            assert_eq!(adopted.environment(), Some(env));
            assert!(!adopted.is_empty(), "an environment alone counted as empty");
        }
    });
}
