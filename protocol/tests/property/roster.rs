//! Invariants of [`stormlight_shared::roster`] — who is playing this match, on
//! which side, driving which unit (`stormlight/server#145`).
//!
//! The roster is the first replicated fact that is *not* about a unit. Everything
//! else on the wire hangs off a body; this outlives one, and has to reach a client
//! that cannot see the body at all — an opponent walking around outside the
//! interest area is still somebody the top bar draws.
//!
//! That shapes what these pin. The join is by **seat**, never by an entity
//! reference, so nothing here can dangle; the unit a seat drives is an explicit
//! absence rather than a sentinel id, because unit id `0` is a perfectly ordinary
//! unit; and the frame is self-describing, because how many players a match seats
//! is the match setup's business and not the protocol's.

use bolero::{TypeGenerator, check};
use stormlight_shared::roster::{
    MAX_SEATED, ROSTER_ENTRY_LEN, Roster, RosterEntry, decode, encode,
};

/// Raw seeds for one seated player. `drives` picks whether the seat currently has
/// a body at all, which is the case a dead or not-yet-spawned player is in.
#[derive(Debug, TypeGenerator)]
struct SeatSeed {
    seat: u16,
    side: u16,
    unit: u32,
    drives: bool,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    seats: Vec<SeatSeed>,
}

fn entry_of(seed: &SeatSeed) -> RosterEntry {
    RosterEntry { seat: seed.seat, side: seed.side, unit: seed.drives.then_some(seed.unit) }
}

fn roster_of(s: &Scenario) -> Roster {
    Roster(s.seats.iter().map(entry_of).collect())
}

#[test]
fn the_frame_length_follows_from_the_seat_count() {
    // Self-describing framing, for the same reason the slot frame is: how many
    // players a match seats is declared by the match setup, and the protocol is
    // not allowed to know a number like "ten".
    check!().with_type::<Scenario>().for_each(|s| {
        let roster = roster_of(s);
        let kept = roster.0.len().min(MAX_SEATED);
        assert_eq!(
            encode(&roster).len(),
            1 + kept * ROSTER_ENTRY_LEN,
            "frame must be a count byte plus one fixed entry per seated player"
        );
    });
}

#[test]
fn a_seated_player_survives_the_wire_exactly() {
    // All three fields are keys rather than quantities: the seat joins the entry
    // to a body, the side decides which half of the bar it is drawn in, and the
    // unit id resolves the picture. An approximation of any of them is a roster
    // describing somebody else.
    check!().with_type::<Scenario>().for_each(|s| {
        let roster = roster_of(s);
        let back = decode(&encode(&roster));
        let kept = roster.0.len().min(MAX_SEATED);
        assert_eq!(back.0.len(), kept, "the seat count must survive the wire");
        for (before, after) in roster.0.iter().take(kept).zip(back.0.iter()) {
            assert_eq!(before.seat, after.seat, "the seat must survive exactly");
            assert_eq!(before.side, after.side, "the side must survive exactly");
            assert_eq!(before.unit, after.unit, "the unit link must survive exactly");
        }
    });
}

#[test]
fn a_seat_with_no_unit_stays_absent_rather_than_becoming_a_unit() {
    // The whole reason the link is an `Option` and not a sentinel id: `0` is an
    // ordinary interned unit, so a roster that spelled "nobody" as `0` would draw
    // whichever hero happened to be interned first over every dead player.
    check!().with_type::<Scenario>().for_each(|s| {
        let roster = roster_of(s);
        let back = decode(&encode(&roster));
        for (before, after) in roster.0.iter().zip(back.0.iter()) {
            assert_eq!(
                before.unit.is_none(),
                after.unit.is_none(),
                "an absent unit must stay absent across the wire, whatever id sits beside it"
            );
        }
    });
}

#[test]
fn a_seat_is_found_by_its_own_number_and_nothing_else() {
    // The join the client performs every frame. A lookup that answered for a seat
    // the roster does not hold would pair a player's row with somebody else's body.
    #[derive(Debug, TypeGenerator)]
    struct Lookup {
        scenario: Scenario,
        seat: u16,
    }
    check!().with_type::<Lookup>().for_each(|l| {
        let roster = roster_of(&l.scenario);
        let found = roster.entry(l.seat);
        match found {
            Some(entry) => {
                assert_eq!(entry.seat, l.seat, "a lookup answered with a different seat");
                assert_eq!(
                    roster.side_of(l.seat),
                    Some(entry.side),
                    "the side lookup must agree with the entry it came from"
                );
            }
            None => {
                assert!(
                    !roster.0.iter().any(|e| e.seat == l.seat),
                    "a seated player was not found by their own seat"
                );
                assert_eq!(roster.side_of(l.seat), None, "an unseated seat has no side");
            }
        }
    });
}

#[test]
fn seating_and_unseating_one_player_leaves_the_others_untouched() {
    // "A player joining or leaving changes the roster and nothing else." Pinned on
    // the wire form, because that is where it would break: a frame is decoded as a
    // whole, so a badly framed entry corrupts every entry after it rather than
    // just its own.
    #[derive(Debug, TypeGenerator)]
    struct Change {
        before: Scenario,
        joiner: SeatSeed,
        /// Which of the standing seats leaves, as an index into them.
        leaves: u8,
    }
    check!().with_type::<Change>().for_each(|c| {
        let before = roster_of(&c.before);
        if before.0.len() >= MAX_SEATED {
            return;
        }

        let mut joined = before.clone();
        joined.0.push(entry_of(&c.joiner));
        let joined_back = decode(&encode(&joined));
        for (i, entry) in before.0.iter().enumerate() {
            assert_eq!(joined_back.0[i], *entry, "seating a player disturbed seat index {i}");
        }

        if before.0.is_empty() {
            return;
        }
        let gone = usize::from(c.leaves) % before.0.len();
        let mut left = before.clone();
        left.0.remove(gone);
        let left_back = decode(&encode(&left));
        for (i, entry) in before.0.iter().enumerate() {
            if i == gone {
                continue;
            }
            let moved = usize::from(i > gone);
            assert_eq!(
                left_back.0[i - moved],
                *entry,
                "unseating index {gone} disturbed the player at index {i}"
            );
        }
    });
}

#[test]
fn every_byte_sequence_decodes_without_panicking() {
    // Totality at the trust boundary. The count byte is attacker-controlled, so a
    // frame claiming more players than it carries must yield only the entries
    // actually present, and every lookup over the result must stay total.
    #[derive(Debug, TypeGenerator)]
    struct Hostile {
        bytes: Vec<u8>,
        seat: u16,
    }
    check!().with_type::<Hostile>().for_each(|h| {
        let roster = decode(&h.bytes);
        let available = h.bytes.len().saturating_sub(1) / ROSTER_ENTRY_LEN;
        assert!(
            roster.0.len() <= available,
            "decoded {} seats from a frame carrying at most {available}",
            roster.0.len()
        );
        assert!(roster.0.len() <= MAX_SEATED, "decoded more seats than the count byte can hold");
        // Both lookups must agree with each other whatever bytes produced them.
        assert_eq!(
            roster.entry(h.seat).map(|e| e.side),
            roster.side_of(h.seat),
            "the two lookups disagreed about a decoded roster"
        );
    });
}
