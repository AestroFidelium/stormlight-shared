//! How fast an entity's time runs (stormlight/server#221).
//!
//! A scale is composed from every source acting on an entity — two fields it stands
//! in, later a personal haste. Composition has to be a pure function of *which*
//! sources apply, never of the order a query happened to visit them in, or the
//! server and a predicting client could disagree about one unit's time. And a stop
//! has to be exactly a stop: only a source that is itself zero may produce zero,
//! because "this target's time is not moving" is what holds every hit aimed at it
//! (server#223).

use bolero::{TypeGenerator, check};
use stormlight_shared::time_scale::TimeScale;

/// A scale a source might declare, in per-mille, anywhere from stopped to past
/// the cap (so the clamp is exercised too).
#[derive(Clone, Copy, Debug, TypeGenerator)]
struct Declared(#[generator(0..=12_000u16)] u16);

impl Declared {
    fn scale(self) -> TimeScale {
        TimeScale::from_per_mille(self.0)
    }
}

/// The sources acting on one entity, and a second ordering of the same sources.
#[derive(Debug, TypeGenerator)]
struct Sources {
    #[generator(bolero::produce::<Vec<Declared>>().with().len(0usize..=6))]
    declared: Vec<Declared>,
    /// How far to rotate the list for the second visiting order.
    rotate: u8,
    /// Which source to raise, and by how much, for the monotonicity check.
    raise_at: u8,
    raise_by: u16,
}

impl Sources {
    fn scales(&self) -> Vec<TimeScale> {
        self.declared.iter().map(|d| d.scale()).collect()
    }
}

#[test]
fn composition_does_not_depend_on_the_visiting_order() {
    check!().with_type::<Sources>().for_each(|s| {
        let forward = s.scales();
        let mut other = forward.clone();
        if !other.is_empty() {
            let by = usize::from(s.rotate) % other.len();
            other.rotate_left(by);
            other.reverse();
        }
        assert_eq!(TimeScale::compose(forward), TimeScale::compose(other));
    });
}

#[test]
fn normal_time_is_neutral_and_nothing_is_normal_time() {
    check!().with_type::<Sources>().for_each(|s| {
        let mut with_normal = s.scales();
        with_normal.push(TimeScale::NORMAL);
        assert_eq!(TimeScale::compose(with_normal), TimeScale::compose(s.scales()));
        assert_eq!(TimeScale::compose(Vec::new()), TimeScale::NORMAL);
    });
}

#[test]
fn only_a_stop_composes_into_a_stop() {
    check!().with_type::<Sources>().for_each(|s| {
        let any_stop = s.scales().iter().any(|t| t.is_stopped());
        assert_eq!(
            TimeScale::compose(s.scales()).is_stopped(),
            any_stop,
            "a composed stop must come from a source that is itself a stop",
        );
    });
}

#[test]
fn composition_stays_within_the_cap() {
    check!().with_type::<Sources>().for_each(|s| {
        let composed = TimeScale::compose(s.scales());
        assert!(composed <= TimeScale::MAX);
        assert!(s.scales().iter().all(|t| *t <= TimeScale::MAX));
    });
}

#[test]
fn speeding_up_one_source_never_slows_the_whole() {
    check!().with_type::<Sources>().for_each(|s| {
        if s.declared.is_empty() {
            return;
        }
        let at = usize::from(s.raise_at) % s.declared.len();
        let before = TimeScale::compose(s.scales());
        let mut raised = s.scales();
        let old = raised[at].per_mille();
        raised[at] = TimeScale::from_per_mille(old.saturating_add(s.raise_by));
        assert!(TimeScale::compose(raised) >= before);
    });
}

/// Any number a mod's expression could evaluate to.
#[derive(Debug, TypeGenerator)]
struct Factor {
    bits: u32,
    /// A second, ordered value in the meaningful range, in thousandths.
    #[generator(0..=10_000u16)]
    low: u16,
    #[generator(0..=10_000u16)]
    high: u16,
}

#[test]
fn any_declared_number_becomes_a_valid_scale() {
    check!().with_type::<Factor>().for_each(|f| {
        let x = f32::from_bits(f.bits);
        let scale = TimeScale::from_factor(x);
        assert!(scale <= TimeScale::MAX);
        if x.is_nan() {
            assert_eq!(scale, TimeScale::NORMAL, "a nonsense number must not bend time");
        } else if x <= 0.0 {
            assert!(scale.is_stopped());
        }
    });
}

#[test]
fn a_declared_factor_keeps_its_order_and_its_value() {
    check!().with_type::<Factor>().for_each(|f| {
        let (lo, hi) = (f.low.min(f.high), f.low.max(f.high));
        let a = TimeScale::from_factor(f32::from(lo) / 1000.0);
        let b = TimeScale::from_factor(f32::from(hi) / 1000.0);
        assert!(a <= b, "a larger factor became a slower scale");
        assert_eq!(a.per_mille(), lo, "a factor on the per-mille grid must survive exactly");
    });
}

/// A tick's delta, in microseconds, up to a slow frame.
#[derive(Debug, TypeGenerator)]
struct Step {
    #[generator(0..=100_000u32)]
    dt_us: u32,
    slow: Declared,
    fast: Declared,
}

#[test]
fn a_scale_bends_a_delta_in_its_own_direction() {
    check!().with_type::<Step>().for_each(|s| {
        #[allow(clippy::cast_precision_loss)]
        let dt = s.dt_us as f32 / 1_000_000.0;
        assert_eq!(TimeScale::STOPPED.apply(dt), 0.0);
        assert_eq!(TimeScale::NORMAL.apply(dt), dt, "normal time must not drift");
        let (slow, fast) = (s.slow.scale().min(s.fast.scale()), s.slow.scale().max(s.fast.scale()));
        assert!(slow.apply(dt) <= fast.apply(dt));
        assert!(slow.apply(dt) >= 0.0);
    });
}
