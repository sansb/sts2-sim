//! #3551: Music Box's Ethereal copy must pass the local-Ethereal provenance
//! gate.
//!
//! v0.111.0 `MusicBox/<AfterCardPlayed>d__13::MoveNext` RVA 0x32ae04 clones
//! the owner's first Attack of the turn (`CreateClone` IL_0046), applies
//! keyword 2 (Ethereal, IL_004c-0057) and adds the clone to Hand
//! (IL_005c-0065). The gate named only Call of the Void, Ghost Seed and
//! Sculpting Strike as writers, so every state after that copy enumerated no
//! legal action and the solver refused `no_legal_actions`.
//!
//! The root is the fight from the issue: run 5QDR1YYSVUJN floor 49, Defect
//! A10 against Queen + Torch Head Amalgam, as served by `/api/fight-roots`.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, LegalActionBuffer},
    hot::{HotState, PileId},
};

const STRIKE_UID: u32 = 2;

fn root() -> (CanonicalStateV2, Catalog, HotState) {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/issue3551_queen_music_box_root.json"
    ))
    .unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    (entry, catalog, state)
}

fn strike_at_target_zero(state: &HotState, catalog: &Catalog) -> Action {
    engine::legal_actions(state, catalog)
        .into_iter()
        .find(|action| {
            matches!(
                action,
                Action::Play {
                    uid: STRIKE_UID,
                    target: Some(0),
                    ..
                }
            )
        })
        .expect("Strike at target 0 is legal at the root")
}

#[test]
fn strike_at_target_zero_leaves_a_playable_state_with_an_ethereal_copy() {
    let (_, catalog, state) = root();
    let strike = strike_at_target_zero(&state, &catalog);
    let hand_before = state.piles.get(PileId::Hand).as_slice().len();
    let mut events = Vec::new();
    let after = engine::apply_action_into(&state, &catalog, &strike, &mut events).unwrap();

    // The played Strike left the hand and its copy entered as the last card.
    let hand = after.piles.get(PileId::Hand).as_slice();
    assert_eq!(hand.len(), hand_before);
    let copy = *hand.last().unwrap();
    assert_ne!(copy.uid, STRIKE_UID);
    assert!(hand.iter().all(|card| card.uid != STRIKE_UID));

    let projected = HotBoundary::try_to_canonical(&after, &catalog).unwrap();
    let wire_copy = projected.piles["hand"].last().unwrap();
    assert_eq!(wire_copy.id, "STRIKE_DEFECT");
    assert_eq!(wire_copy.local_keywords, ["Ethereal"]);

    let mut buffer = LegalActionBuffer::new();
    let legal = engine::legal_actions_checked(&after, &catalog, &mut buffer).unwrap();
    assert!(legal.contains(&Action::EndTurn));
    assert!(
        legal
            .iter()
            .any(|action| matches!(action, Action::Play { uid, .. } if *uid == copy.uid)),
        "the Ethereal copy is playable"
    );
}

/// Every first Attack play at the root (the ten the issue lists) reaches a
/// state that enumerates actions.
#[test]
fn every_first_attack_play_leaves_legal_actions() {
    let (_, catalog, state) = root();
    let mut events = Vec::new();
    let mut attacks = 0;
    for action in engine::legal_actions(&state, &catalog) {
        let Action::Play { uid, .. } = action else {
            continue;
        };
        let card = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| card.uid == uid)
            .unwrap();
        if !catalog.spec(card.atom).unwrap().is_attack {
            continue;
        }
        attacks += 1;
        let after = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
        let mut buffer = LegalActionBuffer::new();
        let legal = engine::legal_actions_checked(&after, &catalog, &mut buffer)
            .unwrap_or_else(|refusal| panic!("{action:?}: {refusal:?}"));
        assert!(legal.contains(&Action::EndTurn), "{action:?}");
    }
    assert_eq!(attacks, 10);
}

/// An unplayed copy exhausts at end of turn; the turn itself completes.
#[test]
fn the_unplayed_copy_exhausts_at_end_of_turn() {
    let (_, catalog, state) = root();
    let strike = strike_at_target_zero(&state, &catalog);
    let mut events = Vec::new();
    let after = engine::apply_action_into(&state, &catalog, &strike, &mut events).unwrap();
    let copy = after.piles.get(PileId::Hand).as_slice().last().unwrap().uid;
    let ended = engine::apply_action_into(&after, &catalog, &Action::EndTurn, &mut events).unwrap();
    assert!(
        ended
            .piles
            .get(PileId::Exhaust)
            .as_slice()
            .iter()
            .any(|card| card.uid == copy)
    );
    let mut buffer = LegalActionBuffer::new();
    engine::legal_actions_checked(&ended, &catalog, &mut buffer).unwrap();
}
