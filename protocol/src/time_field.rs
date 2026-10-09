//! Standing time fields, as both ends derive them (stormlight/server#232).
//!
//! A time field bends the pace of everything its filter keeps inside its radius
//! (stormlight/server#221). The server derives every unit's pace from the fields
//! each tick and publishes the result ([`PaceLog`](crate::pace_log::PaceLog)) — but
//! a predicting client learns that only a round trip later, so for the prediction
//! lead it walks its hero at the old pace and a rollback pulls it back: the hero
//! is nudged the moment its time stops.
//!
//! For a field that already **stands**, none of that is unforeseeable: where it
//! is, how far it reaches, what pace it imposes and for which ticks are fixed when
//! it appears. So both ends derive a unit's pace by one law, here:
//!
//! - the server publishes each field's [`FieldFact`] beside its body's own fact;
//! - and publishes, on each predicted unit, its [`PaceTerms`]: the pace its own
//!   stats impose, and which fields' filters keep it — the two halves of the
//!   derivation only the server can decide, because they read tags, sides and
//!   buffs the client is never sent;
//! - the client then works the geometric half out itself, tick by tick, from where
//!   its own prediction puts the unit — so walking in, walking out and a field's
//!   natural end are predicted exactly.
//!
//! A field that **appears** on a unit cannot be foreseen this way, and is not
//! pretended to be: it is the same server-decided interruption a stun is
//! (stormlight/server#231).

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bodies::BodyFact;
use crate::pace_log::PaceLog;
use crate::time_scale::TimeScale;

/// Whether a point `at` on the ground plane is inside a field centred on `centre`
/// with radius `radius` — the geometric half of "does this field reach that unit",
/// asked by both ends of the same law.
#[must_use]
pub fn within(centre: Vec2, radius: f32, at: Vec2) -> bool {
    centre.distance(at) <= radius
}

/// A standing time field: which one it is, the pace it imposes, and the ticks it
/// bends. Its centre is its entity's position and its radius its
/// [`BodyFact`](crate::bodies::BodyFact)'s.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldFact {
    /// Its key — what a unit's [`PaceTerms`] names it by. Never reused in a match.
    pub key: u32,
    /// The pace it imposes on what it reaches.
    pub pace: TimeScale,
    /// The first tick it bends.
    pub opens: Tick,
    /// The last tick it bends.
    pub closes: Tick,
}

impl FieldFact {
    /// Whether the field bends tick `tick` at all.
    #[must_use]
    pub fn open_at(&self, tick: Tick) -> bool {
        tick - self.opens >= 0 && self.closes - tick >= 0
    }
}

/// What only the server can say about a unit's pace (stormlight/server#232): the
/// pace its own stats impose, and which fields' filters keep it.
///
/// Distance is deliberately not in here. It is the half a predicting client can
/// answer itself, every tick, from where it has put its own unit — which is the
/// whole point.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaceTerms {
    /// The pace the unit's own `time_scale` stat imposes.
    pub own: TimeScale,
    /// The keys of the fields whose filter keeps this unit, in key order.
    pub fields: Vec<u32>,
}

/// One standing field as a client knows it: where it is, how far it reaches, and
/// its fact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownField {
    pub centre: Vec2,
    pub radius: f32,
    pub fact: FieldFact,
}

/// The pace a unit standing at `at` runs at on tick `tick`, by the one law both
/// ends hold (stormlight/server#232): its own pace, times every field whose filter
/// keeps it (`terms`), that is open on that tick and covers that point.
///
/// Composed with [`TimeScale::compose`], which is order-free, so the server's walk
/// over its fields and a client's over the ones it knows give the same pace.
#[must_use]
pub fn derive_pace<'a>(
    terms: &PaceTerms,
    fields: impl IntoIterator<Item = &'a KnownField>,
    at: Vec2,
    tick: Tick,
) -> TimeScale {
    let around = TimeScale::compose(
        fields
            .into_iter()
            .filter(|f| terms.fields.binary_search(&f.fact.key).is_ok())
            .filter(|f| f.fact.open_at(tick) && within(f.centre, f.radius, at))
            .map(|f| f.fact.pace),
    );
    TimeScale::compose([around, terms.own])
}

/// How long, in ticks, a predicting client keeps a field it was told about after
/// the field has closed. A rollback re-steps ticks the field was open on after
/// its body has already left this client; keeping it a while longer than any
/// rollback reaches is what lets that re-step agree with the first one.
pub const FIELD_MEMORY_TICKS: i16 = 256;

/// Every standing field this client has been told about, kept a little past its
/// close (stormlight/server#232).
#[derive(Resource, Debug, Default)]
pub struct KnownFields {
    fields: Vec<KnownField>,
    /// Bumped whenever what is known changes, so a cache stepped through the old
    /// knowledge can tell it is stale.
    generation: u64,
}

impl KnownFields {
    /// The fields known, in key order.
    #[must_use]
    pub fn fields(&self) -> &[KnownField] {
        &self.fields
    }

    /// How many times what is known has changed.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Learn `field`, replacing what was known under its key.
    pub fn learn(&mut self, field: KnownField) {
        match self.fields.binary_search_by_key(&field.fact.key, |f| f.fact.key) {
            Ok(at) if self.fields[at] == field => return,
            Ok(at) => self.fields[at] = field,
            Err(at) => self.fields.insert(at, field),
        }
        self.generation += 1;
    }

    /// Forget every field that closed more than [`FIELD_MEMORY_TICKS`] before `now`.
    pub fn forget_before(&mut self, now: Tick) {
        let before = self.fields.len();
        self.fields.retain(|f| now - f.fact.closes <= FIELD_MEMORY_TICKS);
        if self.fields.len() != before {
            self.generation += 1;
        }
    }
}

/// **Prediction**: learn every standing field the server has replicated, where it
/// stands and how far it reaches, and forget the long-closed ones.
pub fn remember_fields(
    timeline: Option<Res<LocalTimeline>>,
    mut known: ResMut<KnownFields>,
    fields: Query<(&FieldFact, &BodyFact, &Transform)>,
) {
    for (fact, body, transform) in &fields {
        known.learn(KnownField {
            centre: Vec2::new(transform.translation.x, transform.translation.z),
            radius: body.radius,
            fact: *fact,
        });
    }
    if let Some(timeline) = timeline {
        known.forget_before(timeline.tick());
    }
}

/// The pace a predicted unit runs at on `tick` standing at `at`: derived from the
/// fields when the server has told this client its terms, else what the server's
/// own log says (stormlight/server#225).
#[must_use]
pub fn predicted_pace(
    terms: Option<&PaceTerms>,
    known: Option<&KnownFields>,
    log: Option<&PaceLog>,
    at: Vec2,
    tick: Tick,
) -> TimeScale {
    match (terms, known) {
        (Some(terms), Some(known)) => derive_pace(terms, known.fields(), at, tick),
        _ => log.map_or(TimeScale::NORMAL, |log| log.pace_at(tick)),
    }
}

/// Register the field facts for replication on both ends.
///
/// Plain, with no interpolation or prediction: both are the server's facts, fixed
/// (a field's) or rarely changed (a unit's terms), and nothing on a client writes
/// them.
pub fn register(app: &mut App) {
    app.register_component::<FieldFact>();
    app.register_component::<PaceTerms>();
}
