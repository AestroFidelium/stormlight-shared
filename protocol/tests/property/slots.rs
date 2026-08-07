//! Invariants of [`stormlight_shared::slots`] — the owner-scoped view of a
//! caster's ability slots (`stormlight/server#58`).
//!
//! The frame is variable-length (a unit's slot count is content-defined) and
//! carries a **deadline**, not a countdown: the server publishes "ready at tick
//! N" once and the client sweeps toward it locally, so a full ability bar costs
//! bytes when a cooldown *starts*, not on every tick it is running.
//!
//! That makes the sweep the load-bearing part. These pin it: the deadline
//! survives the wire exactly (a quantized cooldown would let a client fire
//! early), the locally-swept remaining time never reaches zero before the
//! deadline tick, the sweep is monotone as the client's clock advances, and no
//! byte sequence can make the decoder panic, over-read, or produce a fill
//! fraction outside `[0, 1]`.

use bolero::{TypeGenerator, check};
use lightyear::prelude::Tick;
use stormlight_shared::slots::{
    AFFORDABLE, MAX_COOLDOWN_TICKS, MAX_SLOTS, ReplicatedSlots, SLOT_ENTRY_LEN, SlotState, UNGATED,
    decode, encode,
};

/// Raw seeds for one slot, mapped onto a representable slot state.
#[derive(Debug, TypeGenerator)]
struct SlotSeed {
    slot: u8,
    ability: u32,
    /// Ticks the cooldown lasts, before clamping to the representable ceiling.
    cooldown: u16,
    /// How far into the cooldown the deadline sits, as a fraction of it.
    elapsed: u8,
    charges: u8,
    flags: u8,
}

/// A caster's slot set plus the client tick it is being read at.
#[derive(Debug, TypeGenerator)]
struct Scenario {
    slots: Vec<SlotSeed>,
    now: u16,
}

fn slot_of(seed: &SlotSeed, now: Tick) -> SlotState {
    let cooldown = seed.cooldown.min(MAX_COOLDOWN_TICKS);
    // The deadline sits `remaining` ticks ahead of `now`, so every generated
    // slot is somewhere legitimate in its own sweep.
    let remaining =
        u16::try_from(u32::from(cooldown) * u32::from(seed.elapsed) / u32::from(u8::MAX))
            .unwrap_or(cooldown);
    SlotState {
        slot: seed.slot,
        ability: seed.ability,
        ready_at: Tick(now.0.wrapping_add(remaining)),
        cooldown_ticks: cooldown,
        charges: seed.charges,
        flags: seed.flags,
    }
}

fn slots_of(s: &Scenario) -> (ReplicatedSlots, Tick) {
    let now = Tick(s.now);
    (ReplicatedSlots(s.slots.iter().map(|seed| slot_of(seed, now)).collect()), now)
}

#[test]
fn the_frame_length_follows_from_the_slot_count() {
    // Self-describing framing: a reader sizes the frame from its own bytes,
    // because how many slots a unit binds is content-defined and unknown here.
    check!().with_type::<Scenario>().for_each(|s| {
        let (slots, _) = slots_of(s);
        let kept = slots.0.len().min(MAX_SLOTS);
        assert_eq!(
            encode(&slots).len(),
            1 + kept * SLOT_ENTRY_LEN,
            "frame must be a count byte plus one fixed entry per replicated slot"
        );
    });
}

#[test]
fn a_slot_binding_survives_the_wire_exactly() {
    // The slot index and the ability id are the two keys a bar is drawn from:
    // which key lights up, and which icon it shows. Neither may be approximated —
    // a drifted ability id resolves to a different (or no) visual.
    check!().with_type::<Scenario>().for_each(|s| {
        let (slots, _) = slots_of(s);
        let back = decode(&encode(&slots));
        let kept = slots.0.len().min(MAX_SLOTS);
        assert_eq!(back.0.len(), kept, "slot count must survive the wire");
        for (before, after) in slots.0.iter().take(kept).zip(back.0.iter()) {
            assert_eq!(before.slot, after.slot, "slot index must survive exactly");
            assert_eq!(before.ability, after.ability, "ability id must survive exactly");
            assert_eq!(before.charges, after.charges, "charge count must survive exactly");
            assert_eq!(before.flags, after.flags, "the castability verdict must survive exactly");
        }
    });
}

#[test]
fn the_deadline_survives_the_wire_exactly() {
    // The whole point of a deadline over a countdown: it is sent once and swept
    // locally, so any loss of precision here is a permanent error in every frame
    // the client draws from it — and a deadline that rounded *down* would let a
    // client believe an ability is ready before the server does.
    check!().with_type::<Scenario>().for_each(|s| {
        let (slots, _) = slots_of(s);
        let back = decode(&encode(&slots));
        for (before, after) in slots.0.iter().zip(back.0.iter()) {
            assert_eq!(before.ready_at, after.ready_at, "the deadline tick must survive exactly");
            assert_eq!(
                before.cooldown_ticks, after.cooldown_ticks,
                "the cooldown length must survive exactly"
            );
        }
    });
}

#[test]
fn the_local_sweep_never_reports_ready_before_the_deadline() {
    // The invariant the whole owner-scoped view exists to keep. The client sweeps
    // its own clock toward a deadline the server set; if that sweep could reach
    // zero even one tick early, every player would press a key the server then
    // silently refuses.
    #[derive(Debug, TypeGenerator)]
    struct Sweep {
        seed: SlotSeed,
        now: u16,
        /// How far the client's clock advances past `now`, bounded well inside
        /// the wrapping window.
        advance: u16,
    }
    check!().with_type::<Sweep>().for_each(|s| {
        let now = Tick(s.now);
        let slot = slot_of(&s.seed, now);
        let advance = s.advance % MAX_COOLDOWN_TICKS;
        let later = Tick(now.0.wrapping_add(advance));

        let remaining_now = slot.remaining_ticks(now);
        if advance < remaining_now {
            assert!(
                !slot.ready(later),
                "slot {} reported ready {} ticks before its deadline",
                slot.slot,
                remaining_now - advance
            );
            assert!(
                slot.remaining_ticks(later) > 0,
                "a slot short of its deadline must still report time remaining"
            );
        }
        if advance >= remaining_now {
            assert!(slot.ready(later), "a slot past its deadline must report ready");
            assert_eq!(slot.remaining_ticks(later), 0, "a ready slot has no time remaining");
        }
    });
}

#[test]
fn the_local_sweep_only_ever_runs_down() {
    // A cooldown bar that ticked backwards would read as the ability re-arming
    // itself. Between two client ticks the remaining time may only shrink, and
    // the fill fraction with it.
    #[derive(Debug, TypeGenerator)]
    struct Sweep {
        seed: SlotSeed,
        now: u16,
        early: u16,
        extra: u16,
    }
    check!().with_type::<Sweep>().for_each(|s| {
        let now = Tick(s.now);
        let slot = slot_of(&s.seed, now);
        // Two observations of the same unchanged deadline, the second later.
        // Both stay inside half the wrapping window so the pair is still ordered
        // by the wrapping comparison rather than aliasing around it.
        let first = Tick(now.0.wrapping_add(s.early % (MAX_COOLDOWN_TICKS / 2)));
        let second = Tick(first.0.wrapping_add(s.extra % (MAX_COOLDOWN_TICKS / 2)));

        assert!(
            slot.remaining_ticks(second) <= slot.remaining_ticks(first),
            "the local sweep ran backwards on slot {}",
            slot.slot
        );
        assert!(
            slot.fraction(second) <= slot.fraction(first) + f32::EPSILON,
            "the cooldown fill fraction ran backwards on slot {}",
            slot.slot
        );
        for at in [first, second] {
            assert!(
                (0.0..=1.0).contains(&slot.fraction(at)),
                "the fill fraction must stay in [0, 1]"
            );
        }
        assert!(slot.ready(second) || !slot.ready(first), "a ready slot must not un-ready itself");
    });
}

#[test]
fn castability_needs_the_cooldown_and_every_flag() {
    // The verdict is a conjunction, and the cooldown half of it is derived from
    // the deadline rather than sent — so an affordable, ungated slot that is
    // still running must not read as castable, and neither must a ready slot the
    // caster cannot pay for.
    check!().with_type::<Scenario>().for_each(|s| {
        let (slots, now) = slots_of(s);
        for slot in &slots.0 {
            let expected = slot.ready(now) && slot.affordable() && slot.ungated();
            assert_eq!(
                slot.castable(now),
                expected,
                "slot {} disagreed with its own parts (ready {}, affordable {}, ungated {})",
                slot.slot,
                slot.ready(now),
                slot.affordable(),
                slot.ungated()
            );
            if !slot.ready(now) {
                assert!(!slot.castable(now), "a slot on cooldown may never read castable");
            }
        }
    });
}

#[test]
fn the_flags_are_independent_bits() {
    // Affordability and the cast gate are separate reasons a key greys out; the
    // HUD tells them apart, so one may never imply the other.
    check!().with_type::<u8>().for_each(|bits| {
        let slot = SlotState { flags: *bits, ..SlotState::default() };
        assert_eq!(slot.affordable(), bits & AFFORDABLE != 0);
        assert_eq!(slot.ungated(), bits & UNGATED != 0);
    });
}

#[test]
fn every_byte_sequence_decodes_without_panicking() {
    // Totality at the trust boundary: the count byte is attacker-controlled, so a
    // frame claiming more slots than it carries must yield only the entries
    // actually present rather than over-reading, and every decoded slot must
    // still produce a finite fill fraction whatever tick it is read at.
    #[derive(Debug, TypeGenerator)]
    struct Hostile {
        bytes: Vec<u8>,
        now: u16,
    }
    check!().with_type::<Hostile>().for_each(|h| {
        let slots = decode(&h.bytes);
        let available = h.bytes.len().saturating_sub(1) / SLOT_ENTRY_LEN;
        assert!(
            slots.0.len() <= available,
            "decoded {} slots from a frame carrying at most {available}",
            slots.0.len()
        );
        let now = Tick(h.now);
        for slot in &slots.0 {
            let f = slot.fraction(now);
            assert!(f.is_finite() && (0.0..=1.0).contains(&f), "fill fraction escaped [0, 1]");
            assert!(
                slot.remaining_ticks(now) <= MAX_COOLDOWN_TICKS,
                "a decoded deadline must stay inside the representable window"
            );
        }
    });
}
