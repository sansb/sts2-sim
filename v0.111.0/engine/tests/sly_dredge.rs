//! Rooted Sly Dredge preserves its frozen sibling batch while moving Discard
//! cards through the existing native hand-cap continuation.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
    hot::PileId,
};

fn shadow_entry(upgrade: u8) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(6));
    doc.player.insert("master_planner".to_owned(), json!(1));
    doc.player.insert(
        "after_card_played_power_order".to_owned(),
        json!([["master_planner", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".to_owned(), json!(100));
    doc.monsters[0].insert("max_hp".to_owned(), json!(100));
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"SHADOW_STEP","upgrade":1,"uid":0},
            {"id":"TACTICIAN","upgrade":0,"uid":2,"local_keywords":["Sly"]},
            {"id":"DREDGE","upgrade":upgrade,"uid":1,"local_keywords":["Sly"]},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":3,"local_keywords":["Sly"]},
            {"id":"DEFEND_SILENT","upgrade":0,"uid":4,"local_keywords":["Sly"]},
            {"id":"DEFEND_DEFECT","upgrade":0,"uid":5,"local_keywords":["Sly"]}
        ],
        "draw": [], "discard": []
    }))
    .unwrap();
    doc
}

fn cold_apply(doc: &CanonicalStateV2, action: Action) -> (CanonicalStateV2, Vec<Event>) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    (
        HotBoundary::try_to_canonical(&next, &catalog).unwrap(),
        events,
    )
}

fn reload_admitted(doc: &CanonicalStateV2) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        *doc
    );
}

fn played(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::CardPlayed { uid, .. } => Some(*uid),
            _ => None,
        })
        .collect()
}

#[test]
fn rooted_sly_dredge_manual_three_of_four_preserves_physical_batch_order() {
    for upgrade in 0..=1 {
        let doc = shadow_entry(upgrade);
        reload_admitted(&doc);
        let (parked, first_events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert!(parked.player.contains_key("pending"), "Dredge+{upgrade}");
        reload_admitted(&parked);
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert_eq!(
            choices.len(),
            24,
            "Dredge+{upgrade}: 4 choose 3 in every order"
        );
        for choice in choices {
            let Action::Select { answer } = choice else {
                panic!("unexpected non-choice")
            };
            let picked = engine::selected_card_uids(&state, &catalog, answer)
                .unwrap()
                .expect("Dredge choice has physical UIDs");
            assert_eq!(picked.len(), 3);
            assert!(picked.iter().all(|uid| (2..=5).contains(uid)));
            let (done, later) = cold_apply(&parked, Action::Select { answer });
            reload_admitted(&done);
            assert!(!done.player.contains_key("pending"));
            let mut events = first_events.clone();
            events.extend(later);
            assert_eq!(played(&events), vec![0, 2, 1, 3, 4, 5]);
            assert!(
                done.piles
                    .get("exhaust")
                    .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(1)))
            );
        }
    }
}

fn direct_entry(hand_count: u32) -> CanonicalStateV2 {
    assert!((1..=3).contains(&(11 - hand_count)));
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(20));
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".to_owned(), json!(100));
    doc.monsters[0].insert("max_hp".to_owned(), json!(100));
    let mut hand = vec![json!({"id":"DREDGE","upgrade":0,"uid":0})];
    hand.extend((1..hand_count).map(|uid| {
        json!({
            "id":"DEFEND_IRONCLAD", "upgrade":0, "uid":uid
        })
    }));
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": [],
        "discard": [
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":10},
            {"id":"STRIKE_SILENT","upgrade":0,"uid":11},
            {"id":"STRIKE_DEFECT","upgrade":0,"uid":12},
            {"id":"DEFEND_SILENT","upgrade":0,"uid":13}
        ]
    }))
    .unwrap();
    doc
}

#[test]
fn direct_dredge_uses_the_live_one_two_or_three_card_hand_capacity() {
    for (hand_count, expected) in [(10, 1), (9, 2), (8, 3)] {
        let doc = direct_entry(hand_count);
        reload_admitted(&doc);
        let (parked, _) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        reload_admitted(&parked);
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        let expected_count = match expected {
            1 => 4,
            2 => 12,
            3 => 24,
            _ => unreachable!(),
        };
        assert_eq!(choices.len(), expected_count, "hand space {expected}");
        for choice in choices {
            let Action::Select { answer } = choice else {
                panic!("Dredge did not publish Select")
            };
            let selected = engine::selected_card_uids(&state, &catalog, answer)
                .unwrap()
                .expect("physical Dredge answer");
            assert_eq!(selected.len(), expected);
            let (done, _) = cold_apply(&parked, Action::Select { answer });
            assert!(!done.player.contains_key("pending"));
            assert_eq!(done.piles.get("hand").unwrap().len(), 10);
            reload_admitted(&done);
        }
    }
}

fn shadow_auto_entry(upgrade: u8) -> CanonicalStateV2 {
    let mut doc = shadow_entry(upgrade);
    doc.player.insert("next_card_uid".to_owned(), json!(4));
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"SHADOW_STEP","upgrade":1,"uid":0},
            {"id":"TACTICIAN","upgrade":0,"uid":2,"local_keywords":["Sly"]},
            {"id":"DREDGE","upgrade":upgrade,"uid":1,"local_keywords":["Sly"]},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":3,"local_keywords":["Sly"]}
        ], "draw": [], "discard": []
    }))
    .unwrap();
    doc
}

#[test]
fn rooted_sly_dredge_auto_moves_the_whole_discard_pile_in_physical_order() {
    for upgrade in 0..=1 {
        let doc = shadow_auto_entry(upgrade);
        reload_admitted(&doc);
        let (done, events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert!(!done.player.contains_key("pending"), "Dredge+{upgrade}");
        assert_eq!(played(&events), vec![0, 2, 1, 3]);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    Event::CardResolved {
                        uid: 2,
                        pile: PileId::Hand
                    }
                ))
                .count(),
            1,
            "the completed earlier sibling is recovered only to Hand"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    Event::CardResolved {
                        uid: 3,
                        pile: PileId::Hand
                    }
                ))
                .count(),
            1,
            "the later physical sibling reaches Hand before its frozen batch slot"
        );
        assert!(
            done.piles
                .get("hand")
                .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(2))),
            "the earlier completed sibling does not replay"
        );
        assert!(
            done.piles
                .get("discard")
                .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(3))),
            "the later sibling still executes at its captured slot"
        );
        assert!(
            done.piles
                .get("exhaust")
                .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(1)))
        );
        reload_admitted(&done);
    }
}

fn cold_refuses(doc: &CanonicalStateV2) {
    match HotBoundary::catalog_from_canonical(doc).and_then(|catalog| {
        HotBoundary::from_canonical(doc, &catalog).map(|state| (catalog, state))
    }) {
        Err(_) => {}
        Ok((catalog, state)) => assert!(engine::admit(doc, &state, &catalog).is_err()),
    }
}

#[test]
fn rooted_sly_dredge_refuses_forged_batch_and_selector_receipts_atomically() {
    let doc = shadow_entry(0);
    let (parked, _) = cold_apply(
        &doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
    let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
    let before = state.clone();
    let mut events = Vec::new();
    assert!(
        engine::apply_action_into(
            &state,
            &catalog,
            &Action::Select {
                answer: engine::SelectionAnswer::OptionIndex(u32::MAX)
            },
            &mut events,
        )
        .is_err()
    );
    assert_eq!(state, before, "invalid Dredge answer is atomic");
    assert!(events.is_empty());

    let mut forged_source = parked.clone();
    forged_source
        .continuations
        .iter_mut()
        .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
        .unwrap()
        .fields
        .insert("source".to_owned(), json!("Tools of the Trade"));
    cold_refuses(&forged_source);

    let mut forged_cursor = parked.clone();
    forged_cursor
        .continuations
        .iter_mut()
        .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
        .unwrap()
        .fields
        .insert("cursor".to_owned(), json!(99));
    cold_refuses(&forged_cursor);

    let mut missing_candidate = parked.clone();
    missing_candidate
        .piles
        .get_mut("discard")
        .unwrap()
        .retain(|card| card.uid != Some(5));
    cold_refuses(&missing_candidate);

    let mut forged_root = parked.clone();
    forged_root.continuations[1]
        .fields
        .insert("source".to_owned(), json!("auto"));
    cold_refuses(&forged_root);
}

#[test]
fn rooted_sly_dredge_keeps_unrelated_sly_children_outside_its_exact_exception() {
    let mut unrelated = shadow_entry(0);
    unrelated
        .piles
        .get_mut("hand")
        .unwrap()
        .iter_mut()
        .find(|card| card.uid == Some(1))
        .unwrap()
        .id = "GLIMMER".to_owned();
    cold_refuses(&unrelated);
}

fn select_exact_uids(doc: &CanonicalStateV2, wanted: &[u32]) -> Action {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| {
            let Action::Select { answer } = action else {
                return false;
            };
            engine::selected_card_uids(&state, &catalog, *answer)
                .unwrap()
                .is_some_and(|actual| actual == wanted)
        })
        .unwrap_or_else(|| panic!("missing exact physical answer {wanted:?}"))
}

#[test]
fn dredge_retrieved_later_brand_or_scavenge_keeps_its_frozen_slot_and_suffix() {
    for (id, suffix, amount) in [
        ("BRAND", "strength", 1),
        ("SCAVENGE", "energy_next_turn", 2),
    ] {
        let mut doc = shadow_entry(0);
        let brand = doc
            .piles
            .get_mut("hand")
            .unwrap()
            .iter_mut()
            .find(|card| card.uid == Some(3))
            .unwrap();
        brand.id = id.to_owned();
        reload_admitted(&doc);
        let (dredge_parked, first_events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        let (brand_parked, dredge_events) = cold_apply(
            &dredge_parked,
            select_exact_uids(&dredge_parked, &[2, 3, 4]),
        );
        reload_admitted(&brand_parked);
        assert!(brand_parked.player.contains_key("pending"), "{id}");
        assert!(
            !brand_parked.player.contains_key(suffix),
            "{id} suffix is not early"
        );
        let mut events = first_events;
        events.extend(dredge_events);
        assert_eq!(
            played(&events),
            vec![0, 2, 1, 3],
            "{id} occupies its original later slot"
        );

        let (done, suffix_events) =
            cold_apply(&brand_parked, select_exact_uids(&brand_parked, &[2]));
        reload_admitted(&done);
        events.extend(suffix_events);
        assert_eq!(
            played(&events),
            vec![0, 2, 1, 3, 4, 5],
            "{id} suffix resumes the outer batch"
        );
        assert_eq!(
            done.player[suffix],
            json!(amount),
            "{id} suffix survives Dredge's move"
        );
        assert!(done.piles.get("exhaust").is_some_and(|pile| {
            pile.iter().any(|card| card.uid == Some(1))
                && pile.iter().any(|card| card.uid == Some(2))
        }));
        assert!(
            done.piles
                .get("discard")
                .is_some_and(|pile| { pile.iter().any(|card| card.uid == Some(3)) })
        );
    }
}
