//! #3036: Misery+ Retains; it does not Exhaust. Base Misery carries neither.
//!
//! v0.111.0 `Misery::OnUpgrade` RVA 0xe5c23 raises Damage by 2, then IL_0018
//! `ldc.i4.5` feeds `CardModel::AddKeyword` at IL_0019; `CardKeyword` 5 is
//! Retain (the enum's field constants: None 0, Exhaust 1, Ethereal 2, Innate 3,
//! Unplayable 4, Retain 5). Misery declares no `CanonicalKeywords`. The frozen
//! registry gave Misery+ Exhaust instead; `generate_content.py`'s
//! `apply_rust_overlays` corrects the row. These witnesses pin the native
//! behaviour end to end, cold-reloading every published state.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::{HotState, PileId},
};

fn entry(level: u8) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":20
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"MISERY","uid":0,"upgrade":level},
            {"id":"STRIKE_IRONCLAD","uid":1,"upgrade":0}
        ],
        "draw": (2..12)
            .map(|uid| json!({"id":"DEFEND_IRONCLAD","uid":uid,"upgrade":0}))
            .collect::<Vec<_>>(),
        "discard": [],
    }))
    .unwrap();
    doc
}

/// Admit `doc`, apply `action`, and cold-reload plus re-admit the result.
fn step(doc: &CanonicalStateV2, action: Action) -> HotState {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let next = engine::apply_action(&state, &catalog, &action)
        .unwrap()
        .state;
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    assert_eq!(cold, next, "cold publication preserves exact state");
    if !next.history.over {
        let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt_state = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
        engine::admit(&wire, &rebuilt_state, &rebuilt).unwrap();
    }
    next
}

fn holds(state: &HotState, pile: PileId, uid: u32) -> bool {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .any(|c| c.uid == uid)
}

#[test]
fn misery_plus_is_retained_at_turn_end_and_base_misery_is_not() {
    for level in 0..=1 {
        let ended = step(&entry(level), Action::EndTurn);
        assert!(!ended.history.over);
        let retained = level == 1;
        assert_eq!(
            holds(&ended, PileId::Hand, 0),
            retained,
            "level {level}: Misery in Hand after turn end"
        );
        assert_eq!(
            holds(&ended, PileId::Discard, 0),
            !retained,
            "level {level}: Misery in Discard after turn end"
        );
        assert!(!holds(&ended, PileId::Exhaust, 0));
        // The unretained Strike is discarded at both levels.
        assert!(holds(&ended, PileId::Discard, 1));
    }
}

#[test]
fn misery_plus_is_not_exhausted_on_play() {
    for level in 0..=1 {
        let played = step(
            &entry(level),
            Action::Play {
                uid: 0,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        );
        assert!(
            holds(&played, PileId::Discard, 0),
            "level {level}: played Misery goes to Discard"
        );
        assert!(
            !holds(&played, PileId::Exhaust, 0),
            "level {level}: played Misery does not exhaust"
        );
        assert_eq!(played.history.owner_cards_exhausted_combat, 0);
        // Damage 7 at L0, 9 at L1 (OnUpgrade IL_0012 UpgradeValueBy(2)).
        assert_eq!(played.monsters[0].hp, 100 - if level == 1 { 9 } else { 7 });
    }
}
