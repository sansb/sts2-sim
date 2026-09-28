//! The opening shuffle: a bare Fisher-Yates over the run's `Shuffle` stream.
//!
//! # Why "bare" is a measured fact, not an assumption
//!
//! `CardPile::RandomizeOrderInternal` (`0x11ea84`) is
//! `ListExtensions.UnstableShuffle` — one descending Fisher-Yates pass,
//! `n - 1` draws — and then calls `Hook::ModifyShuffleOrder` (`0x105ec4`) with
//! `isInitialShuffle: true` (`ldc.i4.1`).
//!
//! At v0.111.0 that hook has exactly one implementor besides the no-op base:
//! `PerfectFit::ModifyShuffleOrder` (`0xd6259`), whose first two instructions
//! are `ldarg.3; brfalse.s` — it returns immediately when `isInitialShuffle`
//! is true. So the opening shuffle is the Fisher-Yates and nothing else.
//! Python states the same conclusion in
//! `_perfect_fit_modify_shuffle_order`'s docstring
//! (frozen Python, deleted #2827).
//!
//! **If a future build gives that hook Glam-style breadth, this is the line
//! that breaks** — which is why the claim lives here with its RVA rather than
//! being implied by the absence of a call.
//!
//! # Order of the input matters
//!
//! The pile enters in **save-array order**: `pile = list(deck)`
//! (frozen Python `start_combat`, deleted #2827), matching `Player::PopulateCombatState`
//! (`0x117a90`), which clones `Deck.Cards.ToList()` into the draw pile in
//! order. The shuffle permutes *that* list, and the replay deal maps input-log
//! instance `k` to the k-th card of the first shuffle cycle over it.

use crate::rng::Xoshiro256StarStar;

/// Permute `pile` in place with one `UnstableShuffle` pass, consuming exactly
/// `pile.len() - 1` draws from `stream`.
///
/// Returns the number of draws consumed, so the caller can assert the
/// consumption against the recorded counter rather than trusting it (I6).
pub fn opening_shuffle<T>(stream: &mut Xoshiro256StarStar, pile: &mut [T]) -> u64 {
    let before = stream.counter;
    // `Xoshiro256StarStar::shuffle` is the `UnstableShuffle` port; it only
    // errors on a bound that does not fit `i32`, which a pile length cannot
    // reach.
    stream
        .shuffle(pile)
        .expect("a pile length always fits the shuffle bound");
    stream.counter - before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opening_shuffle_consumes_exactly_n_minus_one_draws() {
        let mut stream = Xoshiro256StarStar::from_seed(11);
        let mut pile: Vec<u32> = (0..10).collect();
        assert_eq!(opening_shuffle(&mut stream, &mut pile), 9);
        assert_eq!(stream.counter, 9);
    }

    #[test]
    fn a_single_card_pile_spends_nothing() {
        let mut stream = Xoshiro256StarStar::from_seed(11);
        let mut pile = [7_u32];
        assert_eq!(opening_shuffle(&mut stream, &mut pile), 0);
        assert_eq!(stream.counter, 0);
    }

    #[test]
    fn an_empty_pile_spends_nothing() {
        let mut stream = Xoshiro256StarStar::from_seed(11);
        let mut pile: [u32; 0] = [];
        assert_eq!(opening_shuffle(&mut stream, &mut pile), 0);
    }
}
