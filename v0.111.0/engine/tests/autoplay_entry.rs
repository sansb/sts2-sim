//! Native AutoPlay moves even an already-gathered card to Play's bottom.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
};

fn cold_step(doc: &CanonicalStateV2, action: Option<Action>) -> (CanonicalStateV2, Vec<Event>) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let action = action.unwrap_or_else(|| engine::legal_actions(&state, &catalog)[0]);
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    (
        HotBoundary::try_to_canonical(&next, &catalog).unwrap(),
        events,
    )
}

#[test]
fn cascade_gathered_children_reposition_in_play_but_keep_frozen_execution_order() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({"hp":100,"max_hp":100,"energy":2,"exact_piles":true,"player_phase":3,"next_card_uid":6})).unwrap();
    doc.monsters =
        vec![serde_json::from_value(json!({"kind":"SEWER_CLAM","hp":1000,"max_hp":1000})).unwrap()];
    doc.piles = serde_json::from_value(json!({
        "hand":[{"id":"CASCADE","upgrade":0,"uid":0},{"id":"DEFEND_IRONCLAD","upgrade":0,"uid":3},{"id":"DEFEND_IRONCLAD","upgrade":0,"uid":4},{"id":"DEFEND_IRONCLAD","upgrade":0,"uid":5}],
        "draw":[{"id":"ARMAMENTS","upgrade":0,"uid":1},{"id":"ARMAMENTS","upgrade":0,"uid":2}],"discard":[]
    })).unwrap();
    let (parked, mut events) = cold_step(
        &doc,
        Some(Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        }),
    );
    assert_eq!(
        parked.piles["play"]
            .iter()
            .map(|c| c.uid.unwrap())
            .collect::<Vec<_>>(),
        [0, 2, 1]
    );
    // #3147: Cascade's row cost is now the native canonical 0, not -1; X
    // still resolves to the whole 2 energy (two autoplayed children) and
    // spends all of it.
    assert_eq!(parked.player["energy"], json!(0));
    let batch = parked
        .continuations
        .iter()
        .find(|f| f.fields.get("source") == Some(&json!("draw-pile flip")))
        .unwrap();
    assert_eq!(
        batch.fields["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["uid"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let mut forged = parked.clone();
    forged.piles.get_mut("play").unwrap().swap(1, 2);
    let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
    if let Ok(state) = HotBoundary::from_canonical(&forged, &catalog) {
        assert!(engine::admit(&forged, &state, &catalog).is_err());
    }
    let (second, more) = cold_step(&parked, None);
    events.extend(more);
    assert!(second.player.contains_key("pending"));
    assert_eq!(
        second.piles["play"]
            .iter()
            .map(|c| c.uid.unwrap())
            .collect::<Vec<_>>(),
        [0, 2]
    );
    let (done, more) = cold_step(&second, None);
    events.extend(more);
    assert!(!done.player.contains_key("pending"));
    assert!(done.piles.get("play").is_none_or(|p| p.is_empty()));
    assert_eq!(
        events
            .iter()
            .filter_map(|e| match e {
                Event::CardPlayed { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let catalog = HotBoundary::catalog_from_canonical(&done).unwrap();
    let state = HotBoundary::from_canonical(&done, &catalog).unwrap();
    engine::admit(&done, &state, &catalog).unwrap();
}
