//! Native CalculatedGamble (v111): OnPlay 0x390e00; OnUpgrade 0xda903.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
};

fn entry(level: u8) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(
        json!({"hp":100,"max_hp":100,"exact_piles":true,"player_phase":3,"next_card_uid":20}),
    )
    .unwrap();
    doc.monsters =
        vec![serde_json::from_value(json!({"kind":"SEWER_CLAM","hp":1000,"max_hp":1000})).unwrap()];
    doc.piles = serde_json::from_value(json!({"hand":[{"id":"CALCULATED_GAMBLE","upgrade":level,"uid":0},{"id":"DEFEND_SILENT","upgrade":0,"uid":1},{"id":"STRIKE_SILENT","upgrade":0,"uid":2}],"draw":(3..13).map(|uid|json!({"id":"DEFEND_SILENT","upgrade":0,"uid":uid})).collect::<Vec<_>>(),"discard":[]})).unwrap();
    doc
}
fn step(doc: &CanonicalStateV2, action: Action) -> CanonicalStateV2 {
    let cat = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &cat).unwrap();
    engine::admit(doc, &state, &cat).unwrap();
    let next = engine::apply_action_into(&state, &cat, &action, &mut Vec::new()).unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &cat).unwrap();
    let cold_cat = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_cat).unwrap();
    engine::admit(&wire, &cold, &cold_cat).unwrap();
    wire
}
#[test]
fn upgrade_retains_in_hand_but_both_levels_exhaust_after_discard_and_draw() {
    for level in [0, 1] {
        let doc = entry(level);
        let played = step(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert_eq!(
            played.piles["hand"]
                .iter()
                .map(|c| c.uid.unwrap())
                .collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(
            played.piles["discard"]
                .iter()
                .map(|c| c.uid.unwrap())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(
            played.piles["exhaust"]
                .iter()
                .map(|c| c.uid.unwrap())
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            played
                .player
                .get("energy")
                .and_then(|v| v.as_i64())
                .unwrap_or(3),
            3
        );
        let ended = step(&doc, Action::EndTurn);
        assert_eq!(
            ended.piles["hand"].iter().any(|c| c.uid == Some(0)),
            level == 1
        );
        assert_eq!(
            ended.piles["discard"].iter().any(|c| c.uid == Some(0)),
            level == 0
        );
    }
}

#[test]
fn decisions_replays_the_same_upgraded_gamble_after_each_exhaust() {
    let mut doc = entry(1);
    doc.player.insert("stars".into(), json!(6));
    for card in doc.piles.get_mut("draw").unwrap() {
        card.id = "STRIKE_SILENT".into();
    }
    doc.piles.get_mut("hand").unwrap()[0] =
        serde_json::from_value(json!({"id":"DECISIONS_DECISIONS","upgrade":0,"uid":0})).unwrap();
    doc.piles.get_mut("hand").unwrap()[1] =
        serde_json::from_value(json!({"id":"CALCULATED_GAMBLE","upgrade":1,"uid":1})).unwrap();
    let cat = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &cat).unwrap();
    engine::admit(&doc, &state, &cat).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(
        &state,
        &cat,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
        &mut events,
    )
    .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, engine::Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
    let wire = HotBoundary::try_to_canonical(&next, &cat).unwrap();
    assert_eq!(
        wire.piles
            .values()
            .flatten()
            .filter(|c| c.uid == Some(1))
            .count(),
        1
    );
    assert!(
        wire.piles["exhaust"]
            .iter()
            .any(|c| c.uid == Some(1) && c.upgrade == 1)
    );
    assert_eq!(wire.piles["hand"].len(), 4);
    let cold_cat = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_cat).unwrap();
    engine::admit(&wire, &cold, &cold_cat).unwrap();
}
