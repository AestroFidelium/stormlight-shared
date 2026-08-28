//! The rules a predicting client reads the **authoritative movement intent** by
//! (stormlight/server#151).
//!
//! Prediction was built when a unit had exactly one source of motion — a leg the
//! client itself ordered, from which it derived the same goal the server would.
//! Everything else the engine can do to a mover (walk it under an attack-move,
//! stop it mid-swing, chase with it, slow it) was invisible to the client, so the
//! mirror integrated different inputs from the authority and the rollback fought
//! the difference every tick.
//!
//! [`MoveIntent`] is that missing half of the inputs, and these are the two pure
//! rules that read it. What they have to get right is the seam between "the
//! authority told me" and "the player just clicked": the authority outranks the
//! mirror on everything it has actually seen, and on nothing it has not.

use bevy::prelude::Vec2;
use bolero::{TypeGenerator, check};
use stormlight_shared::movement::{MoveIntent, MoveSpeed, intent_suppresses, predicted_speed};

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// The rate the authority says the unit walks at.
    speed: u16,
    /// The fallback rate replicated onto the mirror.
    fallback: u16,
    /// Whether the authority is suppressing the unit's locomotion.
    held: bool,
    /// Whether the authority has a goal at all.
    has_goal: bool,
    /// Where the authority's goal is.
    goal_x: u8,
    goal_z: u8,
    /// Where the mirror is actually walking to.
    walk_x: u8,
    walk_z: u8,
}

fn frac(seed: u8) -> f32 {
    f32::from(seed) / f32::from(u8::MAX)
}

impl Scenario {
    fn speed(&self) -> f32 {
        f32::from(self.speed) / 512.0
    }

    fn fallback(&self) -> f32 {
        f32::from(self.fallback) / 512.0
    }

    fn authority_goal(&self) -> Vec2 {
        Vec2::new(frac(self.goal_x) * 40.0 - 20.0, frac(self.goal_z) * 40.0 - 20.0)
    }

    fn walking_to(&self) -> Vec2 {
        Vec2::new(frac(self.walk_x) * 40.0 - 20.0, frac(self.walk_z) * 40.0 - 20.0)
    }

    fn intent(&self) -> MoveIntent {
        MoveIntent {
            goal: self.has_goal.then(|| self.authority_goal()),
            speed: self.speed(),
            held: self.held,
            facing: None,
        }
    }
}

#[test]
fn a_predicted_rate_is_never_negative() {
    check!().with_type::<Scenario>().for_each(|s| {
        let intent = s.intent();
        let with = predicted_speed(Some(&intent), Some(&MoveSpeed(s.fallback())), s.walking_to());
        let without = predicted_speed(None, Some(&MoveSpeed(s.fallback())), s.walking_to());
        let bare = predicted_speed(None, None, s.walking_to());
        for rate in [with, without, bare] {
            assert!(rate >= 0.0, "a mirror was told to walk backwards at {rate}");
        }
        assert_eq!(bare, 0.0, "a mirror with nothing to walk by must rest, not guess");
    });
}

/// The authority's word outranks the replicated fallback whenever there is one —
/// which is the whole of divergence 4: the server prefers the aggregated
/// `move_speed` stat, so a mirror reading `MoveSpeed` walked at a different rate
/// than the unit it mirrors for as long as any modifier was up.
#[test]
fn the_authority_outranks_the_fallback() {
    check!().with_type::<Scenario>().for_each(|s| {
        let intent = s.intent();
        let walking_to = s.walking_to();
        let rate = predicted_speed(Some(&intent), Some(&MoveSpeed(s.fallback())), walking_to);
        if intent_suppresses(&intent, walking_to) {
            assert_eq!(rate, 0.0, "a suppressed mirror kept walking");
        } else {
            assert_eq!(
                rate,
                s.speed().max(0.0),
                "a mirror ignored the rate the authority walked its unit at",
            );
        }
    });
}

/// A suppression is a statement about **the leg the authority knows about**.
///
/// It has to be, or the head start dies: a unit standing in a fight is held with
/// no goal at all, and that is exactly the state a player clicks out of. Scoping
/// the hold to the authority's own goal is what lets the freshly ordered leg walk
/// immediately while a hold on the leg the authority *is* walking still stops it.
#[test]
fn a_hold_only_stops_the_leg_the_authority_gave() {
    check!().with_type::<Scenario>().for_each(|s| {
        let intent = s.intent();
        let walking_to = s.walking_to();
        let suppressed = intent_suppresses(&intent, walking_to);

        if !intent.held {
            assert!(!suppressed, "an unheld intent stopped a mirror");
        }
        if intent.goal != Some(walking_to) {
            assert!(
                !suppressed,
                "a hold on {:?} stopped a mirror walking to {walking_to} — the leg the \
                 player just ordered is not the leg the authority stopped",
                intent.goal,
            );
        }
        // And on the leg it *is* about, the authority is obeyed.
        let same = MoveIntent { goal: Some(walking_to), ..intent };
        assert_eq!(
            intent_suppresses(&same, walking_to),
            same.held,
            "on its own leg a mirror must do exactly what the authority says",
        );
    });
}
