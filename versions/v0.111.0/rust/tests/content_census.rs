//! Census pins that need to name generated string keys.
//!
//! These live outside the crate because `hot_path_contract.rs` forbids text
//! equality anywhere in the hot modules — including their test bodies — and
//! `STOKE_POOL_EXCLUSIONS_V109` is keyed by `&str`. Asserting the key here
//! keeps the pin honest instead of restating it in a form that dodges the
//! guard.

use sts_sim::content_tables::{
    MULTIPLAYER_ONLY_CANPLAY_CARDS, STOKE_POOL_EXCLUSIONS_V109, TEAMMATE_REQUIRED_CANPLAY_CARDS,
};
use sts_sim::ids::CardId;

/// The decoy that #1613 documents.
///
/// `STOKE_POOL_EXCLUSIONS_V109["multiplayer_only"]` is a five-member
/// *generation-pool* exclusion set. The playability census this crate gates
/// `can_play` on is a different, larger set from `_card_can_play`. They
/// overlap on four names, which is exactly enough for a fix reaching for the
/// wrong one to look plausible — and the one card the defect was actually
/// about is absent from the decoy.
#[test]
fn the_stoke_pool_multiplayer_only_key_is_not_the_playability_census() {
    let stoke = STOKE_POOL_EXCLUSIONS_V109
        .iter()
        .find(|(key, _)| *key == "multiplayer_only")
        .map(|(_, members)| *members)
        .expect("the stoke exclusion key exists");

    assert!(
        !stoke.contains(&CardId::BeaconOfHope),
        "the decoy omits the card #1613 was about; using it fixes nothing"
    );
    assert!(
        MULTIPLAYER_ONLY_CANPLAY_CARDS.contains(&CardId::BeaconOfHope),
        "the playability census contains it"
    );

    let overlap = stoke
        .iter()
        .filter(|id| MULTIPLAYER_ONLY_CANPLAY_CARDS.contains(id))
        .count();
    assert_eq!(
        (stoke.len(), overlap, MULTIPLAYER_ONLY_CANPLAY_CARDS.len()),
        (5, 4, 31),
        "the two sets and their overlap are what the #1613 note describes; a \
         change here means one of them moved and the note needs rereading"
    );
}

/// `UNDERWORLD` reaches the same constant through a different Python gate
/// (`not s.teammate_present`), so it is generated into its own table rather
/// than folded into the multiplayer-only census.
#[test]
fn the_teammate_required_gate_stays_separate_from_the_multiplayer_census() {
    assert_eq!(
        TEAMMATE_REQUIRED_CANPLAY_CARDS.as_slice(),
        [CardId::Underworld].as_slice()
    );
    assert!(!MULTIPLAYER_ONLY_CANPLAY_CARDS.contains(&CardId::Underworld));
}
