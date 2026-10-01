//! Native physical draw-top AutoPost behavior, including mixed listener order.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event},
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({"hp":100,"max_hp":100,
        "exact_piles":true,"player_phase":3,"next_card_uid":20}))
    .unwrap();
    doc.monsters =
        vec![serde_json::from_value(json!({"kind":"SEWER_CLAM","hp":100,"max_hp":100})).unwrap()];
    doc.piles = serde_json::from_value(json!({"hand":[],"draw":[
        {"id":"I_AM_INVINCIBLE","upgrade":0,"uid":0},
        {"id":"I_AM_INVINCIBLE","upgrade":1,"uid":1},
        {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2}],"discard":[]}))
    .unwrap();
    doc
}
fn end_turn(doc: &CanonicalStateV2) -> (CanonicalStateV2, Vec<Event>) {
    let cat = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &cat).unwrap();
    engine::admit(doc, &state, &cat).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &cat, &Action::EndTurn, &mut events).unwrap();
    let projected = HotBoundary::try_to_canonical(&next, &cat).unwrap();
    let next_cat = HotBoundary::catalog_from_canonical(&projected).unwrap();
    let cold = HotBoundary::from_canonical(&projected, &next_cat).unwrap();
    if !cold.history.over {
        engine::admit(&projected, &cold, &next_cat).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &next_cat).unwrap(),
        projected
    );
    (projected, events)
}
fn played(events: &[Event]) -> Vec<(u32, i16)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::CardPlayed { uid, energy, .. } => Some((*uid, *energy)),
            _ => None,
        })
        .collect()
}
#[test]
fn invincible_serial_top_copies_play_free_at_both_levels() {
    let (after, events) = end_turn(&entry());
    assert_eq!(played(&events), [(0, 0), (1, 0)]);
    assert_eq!(after.player["hp"], 100);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PlayerBlockGained { amount: 10, .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PlayerBlockGained { amount: 13, .. }))
    );
}
#[test]
fn invincible_below_a_nonlistener_does_not_autoplay() {
    let mut doc = entry();
    doc.piles.get_mut("draw").unwrap().swap(0, 2);
    let (after, events) = end_turn(&doc);
    assert!(played(&events).is_empty());
    assert_eq!(after.player["hp"], 89);
}
#[test]
fn invincible_does_not_reshuffle_to_find_itself() {
    let mut doc = entry();
    let draw = doc.piles.get_mut("draw").unwrap();
    let cards = std::mem::take(draw);
    doc.piles.insert("discard".into(), cards);
    let (_, events) = end_turn(&doc);
    assert!(played(&events).is_empty());
}
#[test]
fn stampede_then_draw_cards_then_exhaust_howl_keep_native_order() {
    let mut doc = entry();
    doc.player.insert("stampede".into(), json!(1));
    doc.piles.get_mut("hand").unwrap().push(
        serde_json::from_value(json!({
        "id":"STRIKE_IRONCLAD","upgrade":0,"uid":3}))
        .unwrap(),
    );
    doc.piles.insert(
        "exhaust".into(),
        serde_json::from_value(json!([{
        "id":"HOWL_FROM_BEYOND","upgrade":0,"uid":4}]))
        .unwrap(),
    );
    let (_, events) = end_turn(&doc);
    assert_eq!(played(&events), [(3, 0), (0, 0), (1, 0), (4, 0)]);
}

#[test]
fn invincible_exhaust_draw_choice_survives_cold_reload_and_rejects_forgery() {
    let mut doc = entry();
    doc.player.insert("corruption".into(), json!(true));
    doc.player
        .insert("result_location_power_order".into(), json!(["corruption"]));
    doc.player.insert("dark_embrace".into(), json!(1));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["dark_embrace", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player.insert("stratagem".into(), json!(1));
    doc.player.insert(
        "after_card_exhausted_power_order".into(),
        json!(["dark_embrace"]),
    );
    doc.piles.get_mut("draw").unwrap().truncate(1);
    doc.piles.insert(
        "discard".into(),
        serde_json::from_value(json!([
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":3},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":4}
        ]))
        .unwrap(),
    );
    let (parked, first_events) = end_turn(&doc);
    assert!(parked.player.contains_key("pending"));
    assert_eq!(played(&first_events), [(0, 0)]);
    assert!(
        parked
            .continuations
            .iter()
            .any(|frame| frame.fields.contains_key("listeners"))
    );
    let cat = HotBoundary::catalog_from_canonical(&parked).unwrap();
    let state = HotBoundary::from_canonical(&parked, &cat).unwrap();
    let actions = engine::legal_actions(&state, &cat);
    assert!(!actions.is_empty());
    let mut events = Vec::new();
    let mut next = engine::apply_action_into(&state, &cat, &actions[0], &mut events).unwrap();
    for _ in 0..10 {
        if next.pending.is_none() {
            break;
        }
        let wire = HotBoundary::try_to_canonical(&next, &cat).unwrap();
        let rebuilt_cat = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_cat).unwrap();
        engine::admit(&wire, &rebuilt, &rebuilt_cat).unwrap();
        let answer = engine::legal_actions(&rebuilt, &rebuilt_cat)[0];
        next = engine::apply_action_into(&rebuilt, &rebuilt_cat, &answer, &mut events).unwrap();
    }
    assert!(next.pending.is_none());
    assert!(next.frames.is_empty());
    let mut forged = parked.clone();
    let phase = forged
        .continuations
        .iter_mut()
        .find(|frame| frame.fields.contains_key("listeners"))
        .unwrap();
    phase.fields.insert(
        "listeners".into(),
        json!([["card", 4, "card", "I_AM_INVINCIBLE"]]),
    );
    let forged_cat = HotBoundary::catalog_from_canonical(&forged).unwrap();
    assert!(HotBoundary::from_canonical(&forged, &forged_cat).is_err());
    let mut rootless = parked.clone();
    rootless.continuations.remove(0);
    let rootless_cat = HotBoundary::catalog_from_canonical(&rootless).unwrap();
    if let Ok(state) = HotBoundary::from_canonical(&rootless, &rootless_cat) {
        assert!(engine::admit(&rootless, &state, &rootless_cat).is_err());
    }
}
