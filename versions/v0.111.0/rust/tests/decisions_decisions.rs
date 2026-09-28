//! Decisions owns three complete free AutoPlay commands on one physical Skill.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
};
fn entry(level: u8, selected: &str) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({"hp":100,"max_hp":100,"stars":6,"exact_piles":true,"player_phase":3,"next_card_uid":20})).unwrap();
    doc.monsters =
        vec![serde_json::from_value(json!({"kind":"SEWER_CLAM","hp":1000,"max_hp":1000})).unwrap()];
    doc.piles = serde_json::from_value(json!({"hand":[{"id":"DECISIONS_DECISIONS","upgrade":level,"uid":0},{"id":selected,"upgrade":0,"uid":1}],"draw":(2..8).map(|uid|json!({"id":"STRIKE_IRONCLAD","upgrade":0,"uid":uid})).collect::<Vec<_>>(),"discard":[]})).unwrap();
    doc
}
fn play(doc: &CanonicalStateV2) -> (CanonicalStateV2, Vec<Event>) {
    let cat = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &cat).unwrap();
    engine::admit(doc, &state, &cat).unwrap();
    let action = Action::Play {
        uid: 0,
        target: None,
        selection: SelectionRef::NONE,
    };
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &cat, &action, &mut events).unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &cat).unwrap();
    let cold_cat = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_cat).unwrap();
    if !cold.history.over {
        engine::admit(&wire, &cold, &cold_cat).unwrap();
    }
    (wire, events)
}
#[test]
fn three_separate_plays_spend_stars_once_and_read_both_draw_amounts() {
    for level in [0, 1] {
        let (wire, events) = play(&entry(level, "DEFEND_IRONCLAD"));
        assert!(!wire.player.contains_key("pending"));
        assert_eq!(wire.player["block"], 15);
        assert_eq!(
            wire.player
                .get("stars")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            0
        );
        assert_eq!(
            wire.player
                .get("energy")
                .and_then(|v| v.as_i64())
                .unwrap_or(3),
            3
        );
        assert_eq!(wire.piles["hand"].len(), if level == 0 { 3 } else { 5 });
        let plays = events
            .iter()
            .filter_map(|event| match event {
                Event::CardPlayed { uid, energy, .. } => Some((*uid, *energy)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(plays, [(0, 0), (1, 0), (1, 0), (1, 0)]);
        assert_eq!(
            wire.piles["discard"]
                .iter()
                .filter(|card| card.uid == Some(1))
                .count(),
            1
        );
    }
}
#[test]
fn exhausted_skill_is_still_the_same_object_on_later_plays() {
    let (wire, events) = play(&entry(0, "IMPERVIOUS"));
    assert_eq!(wire.player["block"], 90);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
    assert_eq!(
        wire.piles["exhaust"]
            .iter()
            .filter(|card| card.uid == Some(1))
            .count(),
        1
    );
}

#[test]
fn three_selecting_children_resume_each_complete_play_after_cold_reload() {
    let (mut wire, mut all_events) = play(&entry(0, "SURVIVOR"));
    let mut choices = 0;
    while wire.player.contains_key("pending") {
        choices += 1;
        assert!(choices <= 3);
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        engine::admit(&wire, &state, &catalog).unwrap();
        let actions = engine::legal_actions(&state, &catalog);
        assert!(
            !actions.is_empty(),
            "choice {choices}: {}",
            wire.canonical_json()
        );
        let action = actions[0];
        let mut events = Vec::new();
        let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
        all_events.extend(events);
        wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    }
    assert_eq!(choices, 2, "last remaining card is selected automatically");
    assert_eq!(wire.player["block"], 24);
    assert_eq!(
        all_events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
    assert!(wire.continuations.is_empty());
}

#[test]
fn altered_repeated_object_and_unrooted_sequence_are_rejected() {
    let (wire, _) = play(&entry(0, "SURVIVOR"));
    let mut forged = wire.clone();
    let batch = forged
        .continuations
        .iter_mut()
        .find(|f| f.fields.get("source") == Some(&json!("Decisions Decisions")))
        .unwrap();
    batch
        .fields
        .get_mut("entries")
        .unwrap()
        .as_array_mut()
        .unwrap()[1]["uid"] = json!(2);
    let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
    assert!(HotBoundary::from_canonical(&forged, &catalog).is_err());
    let mut rootless = wire.clone();
    rootless.continuations.remove(0);
    let catalog = HotBoundary::catalog_from_canonical(&rootless).unwrap();
    if let Ok(state) = HotBoundary::from_canonical(&rootless, &catalog) {
        assert!(engine::admit(&rootless, &state, &catalog).is_err());
    }
}

#[test]
fn no_eligible_skill_finishes_after_draw_without_autoplay() {
    let mut doc = entry(1, "STRIKE_IRONCLAD");
    let (wire, events) = play(&doc);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { .. }))
            .count(),
        1
    );
    assert!(!wire.player.contains_key("pending"));
    assert_eq!(wire.piles["hand"].len(), 6);
    doc.piles.get_mut("draw").unwrap().clear();
    assert!(play(&doc).0.continuations.is_empty());
}

#[test]
fn three_commands_consume_glam_on_the_first_command_only() {
    let mut doc = entry(0, "DEFEND_IRONCLAD");
    let card = &mut doc.piles.get_mut("hand").unwrap()[1];
    let mut value = serde_json::to_value(&*card).unwrap();
    value["enchantment"] = json!(["GLAM", 1]);
    *card = serde_json::from_value(value).unwrap();
    let (wire, events) = play(&doc);
    assert_eq!(wire.player["block"], 20);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
}

#[test]
fn a_child_can_upgrade_the_parent_without_repeating_its_draw_prefix() {
    let (wire, events) = play(&entry(0, "APOTHEOSIS"));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
    let card = wire.piles["exhaust"]
        .iter()
        .find(|card| card.uid == Some(1))
        .unwrap();
    assert_eq!(card.upgrade, 0, "Apotheosis excludes its own source");
    assert_eq!(
        wire.piles["exhaust"]
            .iter()
            .find(|c| c.uid == Some(0))
            .unwrap()
            .upgrade,
        1
    );
    assert_eq!(wire.piles["hand"].len(), 3);
}

#[test]
fn decisions_and_invincible_share_the_replay_capability_without_aliasing() {
    let (wire, events) = play(&entry(0, "I_AM_INVINCIBLE"));
    assert_eq!(wire.player["block"], 30);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { uid: 1, .. }))
            .count(),
        3
    );
}

#[test]
fn ending_combat_in_the_first_child_skips_later_commands() {
    let mut doc = entry(0, "DEFEND_IRONCLAD");
    doc.player.insert("juggernaut".into(), json!(10));
    doc.player.insert(
        "after_block_gained_power_order".into(),
        json!(["juggernaut"]),
    );
    doc.monsters[0].insert("hp".into(), json!(5));
    let (wire, events) = play(&doc);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardPlayed { uid: 1, .. }))
            .count(),
        1
    );
    assert!(wire.continuations.is_empty());
}

#[test]
fn parent_choice_replays_the_selected_skill_only() {
    let mut doc = entry(0, "DEFEND_IRONCLAD");
    doc.piles
        .get_mut("hand")
        .unwrap()
        .push(serde_json::from_value(json!({"id":"IMPERVIOUS","upgrade":0,"uid":8})).unwrap());
    let (wire, _) = play(&doc);
    let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    let actions = engine::legal_actions(&state, &catalog);
    assert_eq!(actions.len(), 2);
    let mut blocks = Vec::new();
    for action in actions {
        let next = engine::apply_action(&state, &catalog, &action).unwrap();
        let result = HotBoundary::try_to_canonical(&next.state, &catalog).unwrap();
        assert!(result.continuations.is_empty());
        blocks.push(result.player["block"].as_i64().unwrap());
    }
    blocks.sort();
    assert_eq!(blocks, [15, 90]);
}

#[test]
fn selecting_children_leave_hand_before_their_body() {
    for skill in ["BEGONE", "GUARDS", "DECISIONS_DECISIONS"] {
        let (mut wire, _) = play(&entry(0, skill));
        let mut choices = 0;
        while wire.player.contains_key("pending") {
            choices += 1;
            assert!(choices <= 3, "{skill}");
            assert!(
                wire.piles["hand"].iter().all(|card| card.uid != Some(1)),
                "{skill}"
            );
            assert!(
                wire.piles["play"].iter().any(|card| card.uid == Some(1)),
                "{skill}"
            );
            let cat = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let state = HotBoundary::from_canonical(&wire, &cat).unwrap();
            engine::admit(&wire, &state, &cat).unwrap();
            let actions = engine::legal_actions(&state, &cat);
            // Begone selects one drawn Strike; Guards enumerates optional ordered subsets.
            assert!(!actions.is_empty(), "{skill}");
            for action in &actions {
                let next =
                    engine::apply_action_into(&state, &cat, action, &mut Vec::new()).unwrap();
                let cold = HotBoundary::try_to_canonical(&next, &cat).unwrap();
                let cold_cat = HotBoundary::catalog_from_canonical(&cold).unwrap();
                let cold_state = HotBoundary::from_canonical(&cold, &cold_cat).unwrap();
                engine::admit(&cold, &cold_state, &cold_cat).unwrap();
            }
            let next =
                engine::apply_action_into(&state, &cat, &actions[0], &mut Vec::new()).unwrap();
            wire = HotBoundary::try_to_canonical(&next, &cat).unwrap();
        }
        assert_eq!(choices, if skill == "DECISIONS_DECISIONS" { 0 } else { 3 });
        assert_eq!(
            wire.piles
                .values()
                .flatten()
                .filter(|card| card.uid == Some(1))
                .count(),
            1,
            "{skill}"
        );
    }
}
