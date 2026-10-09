//! The one law both ends derive a unit's pace by (stormlight/server#232).
//!
//! Invariants:
//!   - **A field bends only what its filter keeps, only while it is open, only
//!     inside its radius** — dropping any one of the three conditions changes the
//!     answer for some input;
//!   - **Order-free**: the same fields in any order give the same pace, so the
//!     server's walk and a client's agree whatever order each holds them in;
//!   - **The unit's own pace always applies**, field or no field;
//!   - **A field's span is inclusive and wraps with the tick count**.

use bevy::math::Vec2;
use bolero::{TypeGenerator, check};
use lightyear::prelude::Tick;
use stormlight_shared::time_field::{FieldFact, KnownField, PaceTerms, derive_pace, within};
use stormlight_shared::time_scale::TimeScale;

#[derive(Clone, Debug, TypeGenerator)]
struct Field {
    /// Whether the unit's terms name it.
    kept: bool,
    #[generator(-40i16..=40)]
    x: i16,
    #[generator(-40i16..=40)]
    z: i16,
    #[generator(0u16..=60)]
    radius_tenths: u16,
    #[generator(0u16..=3000)]
    pace: u16,
    /// Ticks from the asked tick to its opening (negative = opened before).
    #[generator(-20i16..=20)]
    opens: i16,
    #[generator(0u16..=40)]
    lasts: u16,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    #[generator(bolero::produce::<Vec<Field>>().with().len(0usize..=6))]
    fields: Vec<Field>,
    #[generator(-40i16..=40)]
    at_x: i16,
    #[generator(-40i16..=40)]
    at_z: i16,
    #[generator(0u16..=3000)]
    own: u16,
    /// The tick asked about, anywhere in the wrapping count.
    now: u16,
    /// A rotation of the field list.
    rotate: u8,
}

fn known(s: &Scenario) -> (Vec<KnownField>, PaceTerms) {
    let now = Tick(s.now);
    let mut keys = Vec::new();
    let fields = s
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let key = u32::try_from(i).unwrap_or(0) + 1;
            if f.kept {
                keys.push(key);
            }
            let opens = now + f.opens;
            KnownField {
                centre: Vec2::new(f32::from(f.x) / 4.0, f32::from(f.z) / 4.0),
                radius: f32::from(f.radius_tenths) / 10.0,
                fact: FieldFact {
                    key,
                    pace: TimeScale::from_per_mille(f.pace),
                    opens,
                    closes: opens + i16::try_from(f.lasts).unwrap_or(0),
                },
            }
        })
        .collect();
    (fields, PaceTerms { own: TimeScale::from_per_mille(s.own), fields: keys })
}

fn at(s: &Scenario) -> Vec2 {
    Vec2::new(f32::from(s.at_x) / 4.0, f32::from(s.at_z) / 4.0)
}

#[test]
fn a_field_bends_only_what_it_keeps_while_open_and_inside() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (fields, terms) = known(s);
        let now = Tick(s.now);
        // The fields first, then the unit's own pace — the order the server composes
        // them in. Composition rounds at each step, so the grouping is part of the
        // law, not a detail of it.
        let around = TimeScale::compose(
            fields
                .iter()
                .zip(&s.fields)
                .filter(|(k, f)| {
                    f.kept
                        && f.opens <= 0
                        && i32::from(f.opens) + i32::from(f.lasts) >= 0
                        && k.centre.distance(at(s)) <= k.radius
                })
                .map(|(k, _)| k.fact.pace),
        );
        let expected = TimeScale::compose([around, terms.own]);
        assert_eq!(derive_pace(&terms, &fields, at(s), now), expected, "{s:?}");
    });
}

#[test]
fn the_order_fields_are_held_in_changes_nothing() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (mut fields, terms) = known(s);
        let before = derive_pace(&terms, &fields, at(s), Tick(s.now));
        if !fields.is_empty() {
            let by = usize::from(s.rotate) % fields.len();
            fields.rotate_left(by);
        }
        assert_eq!(derive_pace(&terms, &fields, at(s), Tick(s.now)), before);
    });
}

#[test]
fn no_field_leaves_the_units_own_pace() {
    check!().with_type::<Scenario>().for_each(|s| {
        let terms = PaceTerms { own: TimeScale::from_per_mille(s.own), fields: Vec::new() };
        let (fields, _) = known(s);
        assert_eq!(derive_pace(&terms, &fields, at(s), Tick(s.now)), terms.own);
    });
}

#[test]
fn the_edge_of_a_field_is_inside_it() {
    check!().with_type::<Scenario>().for_each(|s| {
        // About the origin, so the distance to the edge is the radius exactly and
        // the comparison is about the rule rather than about rounding.
        let radius = f32::from(s.own % 50) / 10.0;
        assert!(within(Vec2::ZERO, radius, Vec2::new(radius, 0.0)), "the edge was outside");
        assert!(within(Vec2::ZERO, radius, Vec2::ZERO), "the centre was outside");
        assert!(
            !within(Vec2::ZERO, radius, Vec2::new(0.0, radius + 0.01)),
            "past the edge was inside"
        );
    });
}
