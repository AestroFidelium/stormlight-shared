//! The law a **guided** shot is re-aimed by (stormlight/server#158).
//!
//! It is run independently by two machines — the server on the authoritative
//! missile, the client on the drawing of it — over one replicated target position
//! and nothing else on the wire. That only works while the rule is a pure function
//! of what both of them already have, and while every step of it agrees exactly.
//! So the properties here are the ones both ends depend on:
//!
//! - **Speed is preserved**, horizontally and in total. A guided shot's odometer is
//!   still `speed × elapsed`, which is what lets it expire at its declared range
//!   through the same check a straight one uses. A turn that changed the speed
//!   would make range mean something different for guided shots.
//! - **The climb is untouched.** The simulation is planar and contact is decided on
//!   the ground, so a shot's height is how it looks. A guided shot that steered in
//!   three dimensions would nose down at the ground origin its target is tracked
//!   by, and skim the floor — undoing the muzzle height that exists precisely to
//!   stop that.
//! - **It really turns.** After one step the shot is heading at the target on the
//!   ground plane, from wherever it was pointing.
//! - **Nothing degenerate invents a heading.** A target standing exactly on the
//!   shot leaves the velocity as it was.

use bevy::math::{Vec2, Vec3};
use bolero::{TypeGenerator, check};
use stormlight_shared::projectiles::home_velocity;

fn coord(seed: u16) -> f32 {
    (f32::from(seed) / f32::from(u16::MAX) - 0.5) * 80.0
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

#[derive(Debug, TypeGenerator)]
struct Chase {
    fx: u16,
    fy: u16,
    fz: u16,
    vx: u16,
    vy: u16,
    vz: u16,
    tx: u16,
    ty: u16,
    tz: u16,
}

impl Chase {
    fn from(&self) -> Vec3 {
        Vec3::new(coord(self.fx), coord(self.fy), coord(self.fz))
    }
    fn velocity(&self) -> Vec3 {
        Vec3::new(coord(self.vx), coord(self.vy), coord(self.vz))
    }
    fn toward(&self) -> Vec3 {
        Vec3::new(coord(self.tx), coord(self.ty), coord(self.tz))
    }
}

#[test]
fn guidance_turns_without_speeding_up_or_diving() {
    check!().with_type::<Chase>().for_each(|c| {
        let (from, velocity, toward) = (c.from(), c.velocity(), c.toward());
        let turned = home_velocity(from, velocity, toward);
        assert!(turned.is_finite(), "a guided step must be a real velocity");

        let to_target = flat(toward) - flat(from);
        if to_target.length() < 1e-4 {
            // Standing on it: there is no direction to turn onto.
            assert_eq!(turned, velocity, "a target on top of the shot must not turn it");
            return;
        }

        // The climb is exactly what it was — this is the one that stops a guided
        // shot nosing into the floor.
        assert!(
            (turned.y - velocity.y).abs() < 1e-4,
            "guidance changed the climb: {} became {}",
            velocity.y,
            turned.y,
        );
        // Horizontal speed preserved, and therefore the total as well: the range
        // check is `speed × elapsed` and both ends share it.
        assert!(
            (flat(turned).length() - flat(velocity).length()).abs() < 1e-3,
            "guidance changed the ground speed",
        );
        assert!(
            (turned.length() - velocity.length()).abs() < 1e-3,
            "guidance changed the total speed, so range would mean something else",
        );

        // And it is actually aimed there now, on the plane contact is decided on.
        if flat(velocity).length() > 1e-3 {
            let heading = flat(turned).normalize();
            assert!(
                heading.dot(to_target.normalize()) > 0.999,
                "after a guided step the shot heads {heading} rather than at its target",
            );
        }
    });
}

#[test]
fn guidance_is_idempotent_once_it_is_on_target() {
    check!().with_type::<Chase>().for_each(|c| {
        let (from, velocity, toward) = (c.from(), c.velocity(), c.toward());
        if (flat(toward) - flat(from)).length() < 1e-3 || flat(velocity).length() < 1e-3 {
            return;
        }
        // Steering twice from the same place is steering once: the two machines
        // running this do not have to agree on *how often* they run it, only on
        // where the target is.
        let once = home_velocity(from, velocity, toward);
        let twice = home_velocity(from, once, toward);
        assert!(
            (twice - once).length() < 1e-3,
            "a second step from the same place moved the shot again: {once} then {twice}",
        );
    });
}
