//! #3022: Sculpting Strike applies Ethereal (not Retain) to the chosen card.
//!
//! v0.111.0 `SculptingStrike/<OnPlay>d__7::MoveNext` RVA 0x3b8cec builds a
//! one-element `CardKeyword[]` holding 2 (Ethereal) at IL_0187 and passes it to
//! `CardCmd::ApplyKeyword` at IL_0189. Its `FromHand` filter
//! `<>c::<OnPlay>b__7_0` (RVA 0x3b8cda) admits only cards whose
//! `GetKeywordsWithSources(2)` lacks Ethereal. The frozen registry carried
//! Snap's Retain step; these witnesses pin the native behaviour end to end:
//! the chosen card exhausts at turn end, and an already-Ethereal card (canonical
//! or local keyword) is never a candidate.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::{HotState, PileId},
};

fn entry(hand: serde_json::Value) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":10
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": [
            {"id":"STRIKE_IRONCLAD","uid":7,"upgrade":0},
            {"id":"STRIKE_IRONCLAD","uid":8,"upgrade":0},
            {"id":"STRIKE_IRONCLAD","uid":9,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    doc
}

/// Cold roundtrip plus admission: the Ethereal marker must survive the wire
/// and pass the local-Ethereal provenance gate.
fn wire(doc: &CanonicalStateV2, state: &HotState) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    assert_eq!(cold, *state, "cold publication preserves exact state");
    if !state.history.over {
        let rebuilt_state = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
        engine::admit(&wire, &rebuilt_state, &rebuilt).unwrap();
    }
    wire
}

fn load(doc: &CanonicalStateV2) -> (HotState, sts_sim::catalog::Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn play_sculpting(doc: &CanonicalStateV2) -> (HotState, sts_sim::catalog::Catalog) {
    let (state, catalog) = load(doc);
    let next = engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: Some(0),
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
    .state;
    (next, catalog)
}

/// The card's published local keywords (the wire is the public surface).
fn local_keywords(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> Vec<String> {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    wire.piles
        .values()
        .flatten()
        .find(|card| card.uid == Some(u64::from(uid)))
        .map(|card| card.local_keywords.clone())
        .unwrap_or_default()
}

fn local_ethereal(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> bool {
    local_keywords(doc, state, uid)
        .iter()
        .any(|k| k == "Ethereal")
}

fn local_retain(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> bool {
    local_keywords(doc, state, uid)
        .iter()
        .any(|k| k == "Retain")
}

fn pile_uids(state: &HotState, pile: PileId) -> Vec<u32> {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .map(|c| c.uid)
        .collect()
}

#[test]
fn chosen_card_gains_ethereal_not_retain_and_exhausts_at_turn_end() {
    for level in 0..=1 {
        let doc = entry(json!([
            {"id":"SCULPTING_STRIKE","uid":0,"upgrade":level},
            {"id":"DEFEND_IRONCLAD","uid":1,"upgrade":0},
            {"id":"BASH","uid":2,"upgrade":0}
        ]));
        let (parked, catalog) = play_sculpting(&doc);
        assert!(parked.pending.is_some(), "two candidates park a choice");
        let choices = engine::legal_actions(&parked, &catalog);
        assert_eq!(
            choices.len(),
            2,
            "level {level}: both hand cards are candidates"
        );
        let chosen = choices
            .iter()
            .map(|choice| {
                engine::apply_action(&parked, &catalog, choice)
                    .unwrap()
                    .state
            })
            .find(|state| local_ethereal(&doc, state, 1))
            .expect("one answer marks Defend Ethereal");
        assert!(
            !local_ethereal(&doc, &chosen, 2),
            "only the chosen card changes"
        );
        assert!(
            !local_retain(&doc, &chosen, 1),
            "Sculpting Strike never writes Retain"
        );
        wire(&doc, &chosen);

        let ended = engine::apply_action(&chosen, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let exhaust = pile_uids(&ended, PileId::Exhaust);
        assert!(
            exhaust.contains(&1),
            "level {level}: the Ethereal Defend exhausts"
        );
        assert!(
            !exhaust.contains(&2),
            "level {level}: the unchosen Bash does not"
        );
        // The unchosen Bash was discarded, then reshuffled by next turn's
        // draw; it is live outside Exhaust.
        assert!(
            [PileId::Hand, PileId::Draw, PileId::Discard]
                .into_iter()
                .any(|pile| pile_uids(&ended, pile).contains(&2))
        );
        assert!(!ended.history.over);
    }
}

#[test]
fn already_ethereal_cards_are_not_candidates() {
    // Dazed is canonically Ethereal; the Defend carries a local Ethereal
    // keyword (a prior Sculpting Strike). Only the Bash is selectable, so the
    // one-card choice resolves on it.
    let doc = entry(json!([
        {"id":"SCULPTING_STRIKE","uid":0,"upgrade":0},
        {"id":"DAZED","uid":1,"upgrade":0},
        {"id":"DEFEND_IRONCLAD","uid":2,"upgrade":0,"local_keywords":["Ethereal"]},
        {"id":"BASH","uid":3,"upgrade":0}
    ]));
    let (state, catalog) = play_sculpting(&doc);
    let resolved = if state.pending.is_some() {
        let choices = engine::legal_actions(&state, &catalog);
        assert_eq!(choices.len(), 1, "only the Bash is a candidate");
        engine::apply_action(&state, &catalog, &choices[0])
            .unwrap()
            .state
    } else {
        state
    };
    assert!(
        local_ethereal(&doc, &resolved, 3),
        "the Bash is the one choice"
    );
    assert!(
        !local_ethereal(&doc, &resolved, 1),
        "Dazed keeps only its canonical keyword"
    );
    assert!(local_ethereal(&doc, &resolved, 2));
    wire(&doc, &resolved);
}

#[test]
fn no_candidate_when_every_hand_card_is_ethereal() {
    let doc = entry(json!([
        {"id":"SCULPTING_STRIKE","uid":0,"upgrade":0},
        {"id":"DAZED","uid":1,"upgrade":0}
    ]));
    let (state, _catalog) = play_sculpting(&doc);
    assert!(
        state.pending.is_none(),
        "an empty candidate set asks nothing"
    );
    assert!(!local_ethereal(&doc, &state, 1));
    wire(&doc, &state);
}
