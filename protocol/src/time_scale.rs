//! How fast an entity's time runs, relative to the world's (stormlight/server#221).
//!
//! A mod can stop time in an area, slow it or speed it up. Everything that happens
//! *over time* to an entity — walking, a missile's flight, a dash, a zone's period —
//! advances by the world's tick delta bent by the entity's [`TimeScale`].
//!
//! # Why fixed-point
//!
//! The scale is a count of thousandths, not an `f32`. A predicting client steps its
//! own unit by the same scale the server does, so the two must compute the *same*
//! number from the same sources; and "a stopped entity changes nothing over N
//! ticks" is then an equality rather than a tolerance.
//!
//! # Composition
//!
//! Every source acting on an entity multiplies in ([`TimeScale::compose`]): a
//! half-speed area inside a one-and-a-half-speed one runs at three quarters. The
//! sources are sorted first, so the result depends on *which* sources apply and
//! never on the order a query visited them in. A stop absorbs everything, and only
//! a stop does: two very slow sources never round down into one, because a stopped
//! target is a categorically different thing (every hit aimed at it is held).

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Thousandths in one whole factor.
const PER_MILLE: u32 = 1000;

/// How fast an entity's time runs, in thousandths of the world's pace: `0` is
/// stopped, [`TimeScale::NORMAL`] is the world's own pace, and nothing runs faster
/// than [`TimeScale::MAX`].
///
/// An entity without one runs at the world's pace — the [`Default`].
///
/// ```
/// use stormlight_shared::time_scale::TimeScale;
///
/// let slow = TimeScale::from_factor(0.5);
/// let fast = TimeScale::from_factor(1.5);
/// assert_eq!(TimeScale::compose([slow, fast]), TimeScale::from_factor(0.75));
/// assert_eq!(slow.apply(0.1), 0.05);
/// assert!(TimeScale::compose([fast, TimeScale::STOPPED]).is_stopped());
/// ```
#[derive(
    Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
)]
pub struct TimeScale(u16);

impl TimeScale {
    /// Time does not move at all.
    pub const STOPPED: Self = Self(0);
    /// The world's own pace.
    #[expect(clippy::cast_possible_truncation, reason = "1000 fits a u16")]
    pub const NORMAL: Self = Self(PER_MILLE as u16);
    /// The fastest an entity's time may run: ten times the world's pace. Past it,
    /// one tick would carry a unit across a room and a periodic effect through
    /// dozens of periods, which nothing a fight needs is worth.
    pub const MAX: Self = Self(10_000);

    /// A scale of `per_mille` thousandths, capped at [`TimeScale::MAX`].
    #[must_use]
    pub fn from_per_mille(per_mille: u16) -> Self {
        Self(per_mille.min(Self::MAX.0))
    }

    /// The scale a mod's number declares, rounded to the nearest thousandth.
    ///
    /// Total: a number that is not one bends nothing (it reads as the world's pace,
    /// so a broken expression cannot freeze a unit forever), zero or less is a stop,
    /// and anything past the cap is the cap.
    #[must_use]
    pub fn from_factor(factor: f32) -> Self {
        if factor.is_nan() {
            return Self::NORMAL;
        }
        let thousandths = (factor * PER_MILLE as f32).round();
        if thousandths <= 0.0 {
            return Self::STOPPED;
        }
        if thousandths >= f32::from(Self::MAX.0) {
            return Self::MAX;
        }
        #[expect(clippy::cast_possible_truncation, reason = "bounded to (0, MAX) above")]
        #[expect(clippy::cast_sign_loss, reason = "bounded to (0, MAX) above")]
        Self(thousandths as u16)
    }

    /// The scale in thousandths of the world's pace.
    #[must_use]
    pub fn per_mille(self) -> u16 {
        self.0
    }

    /// The scale as a plain factor, `1.0` being the world's pace.
    #[must_use]
    pub fn factor(self) -> f32 {
        f32::from(self.0) / PER_MILLE as f32
    }

    /// Whether time does not move at all.
    #[must_use]
    pub fn is_stopped(self) -> bool {
        self.0 == 0
    }

    /// The share of the world's `dt` that passes for this entity.
    ///
    /// The world's pace returns `dt` itself rather than `dt × 1.0`, so an entity no
    /// source touches integrates exactly what it did before scales existed.
    #[must_use]
    pub fn apply(self, dt: f32) -> f32 {
        if self == Self::NORMAL { dt } else { dt * self.factor() }
    }

    /// The scale every one of `scales` acting together produces: their product,
    /// within the cap. No sources at all is the world's pace.
    ///
    /// The sources are sorted before they are multiplied, so the result is a
    /// function of the set alone; each partial product is rounded to the nearest
    /// thousandth but never below one, so only a source that is itself a stop
    /// stops the whole.
    #[must_use]
    pub fn compose(scales: impl IntoIterator<Item = Self>) -> Self {
        // Nearly every entity is in no field or in one, and this runs for each of
        // them every tick: those two answers need no sorting and no allocation.
        let mut scales = scales.into_iter();
        let Some(first) = scales.next() else { return Self::NORMAL };
        let Some(second) = scales.next() else { return first };
        let mut sorted: Vec<u16> =
            [first.0, second.0].into_iter().chain(scales.map(|s| s.0)).collect();
        sorted.sort_unstable();
        let mut acc = u32::from(Self::NORMAL.0);
        for factor in sorted {
            if factor == 0 {
                return Self::STOPPED;
            }
            let product = (acc * u32::from(factor) + PER_MILLE / 2) / PER_MILLE;
            acc = product.clamp(1, u32::from(Self::MAX.0));
        }
        // Bounded by the clamp above (or NORMAL, untouched).
        Self(u16::try_from(acc).unwrap_or(Self::MAX.0))
    }
}

impl Default for TimeScale {
    fn default() -> Self {
        Self::NORMAL
    }
}
