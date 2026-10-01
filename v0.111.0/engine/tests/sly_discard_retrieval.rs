//! Rooted native Sly children may retrieve a physical sibling without
//! changing the already-captured AutoPlay batch order.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
};

fn entry(id: &str, upgrade: u8) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(4));
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
            {"id": "SHADOW_STEP", "upgrade": 1, "uid": 0},
            {"id": "DEFEND_IRONCLAD", "upgrade": 0, "uid": 2, "local_keywords": ["Sly"]},
            {"id": id, "upgrade": upgrade, "uid": 1, "local_keywords": ["Sly"]},
            {"id": "DEFEND_IRONCLAD", "upgrade": 0, "uid": 3, "local_keywords": ["Sly"]}
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

fn cold_refuses(doc: &CanonicalStateV2) {
    match HotBoundary::catalog_from_canonical(doc).and_then(|catalog| {
        HotBoundary::from_canonical(doc, &catalog).map(|state| (catalog, state))
    }) {
        Err(_) => {}
        Ok((catalog, state)) => assert!(engine::admit(doc, &state, &catalog).is_err()),
    }
}

fn with_base_replay(doc: &CanonicalStateV2, uid: u32) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let mut state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    let mut physical = state.card_states.get(uid);
    physical.set_base_replay_count(Some(1)).unwrap();
    state.card_states.set(uid, physical);
    for pile in [
        sts_sim::hot::PileId::Hand,
        sts_sim::hot::PileId::Draw,
        sts_sim::hot::PileId::Discard,
    ] {
        if let Some(card) = state
            .piles
            .get_mut(pile)
            .make_mut()
            .iter_mut()
            .find(|card| card.uid == uid)
        {
            card.flags |= sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            return HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        }
    }
    panic!("replay source is live");
}

#[test]
fn rooted_sly_discard_retrieval_keeps_live_batch_identity_and_order() {
    for (id, upgrade, hologram_exhausts) in [
        ("HOLOGRAM", 0, true),
        ("HOLOGRAM", 1, false),
        ("COSMIC_INDIFFERENCE", 0, false),
        ("COSMIC_INDIFFERENCE", 1, false),
    ] {
        let doc = entry(id, upgrade);
        reload_admitted(&doc);
        let (parked, mut events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert!(parked.player.contains_key("pending"), "{id}+{upgrade}");
        assert_eq!(
            parked
                .piles
                .get("play")
                .unwrap()
                .iter()
                .map(|card| card.uid.unwrap())
                .collect::<Vec<_>>(),
            [0, 1],
            "the parent and paused child, but not either sibling, are in Play"
        );
        let batch = parked
            .continuations
            .iter()
            .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
            .expect("Sly batch is persisted at the public choice");
        assert_eq!(
            batch.fields["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["uid"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [2, 1, 3],
            "DiscardAndDraw's physical capture order stays immutable"
        );
        reload_admitted(&parked);

        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let before = state.clone();
        let mut refusal_events = Vec::new();
        assert!(
            engine::apply_action_into(
                &state,
                &catalog,
                &Action::Select {
                    answer: engine::SelectionAnswer::OptionIndex(u32::MAX)
                },
                &mut refusal_events,
            )
            .is_err()
        );
        assert_eq!(state, before, "an invalid child choice is atomic");
        assert!(refusal_events.is_empty());
        let choices = engine::legal_actions(&state, &catalog);
        assert_eq!(
            choices.len(),
            2,
            "only the two Discard siblings are choices"
        );
        let parked_events = events.clone();
        let mut earlier_retrievals = 0;
        for choice in choices {
            let (done, more) = cold_apply(&parked, choice);
            events = parked_events.clone();
            events.extend(more);
            assert!(!done.player.contains_key("pending"));
            let played: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    Event::CardPlayed { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect();
            assert_eq!(
                played,
                [0, 2, 1, 3],
                "each captured UID plays once in order"
            );
            // Hologram adds its choice to Hand/Bottom; Cosmic Indifference
            // to Draw/Top (#3151, `CosmicIndifference/<OnPlay>d__5`
            // RVA 0x3950c0 IL_01a5-IL_01ab `CardPileCmd.Add(card, 1, 2)`).
            let retrieved_pile = if id == "HOLOGRAM" { "hand" } else { "draw" };
            earlier_retrievals += usize::from(
                done.piles
                    .get(retrieved_pile)
                    .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(2))),
            );
            assert!(
                done.piles
                    .get(retrieved_pile)
                    .is_none_or(|pile| pile.iter().all(|card| card.uid != Some(3)))
            );
            let route = if hologram_exhausts {
                "exhaust"
            } else {
                "discard"
            };
            assert!(
                done.piles
                    .get(route)
                    .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(1)))
            );
            reload_admitted(&done);
        }
        assert_eq!(
            earlier_retrievals, 1,
            "the earlier sibling never replays after retrieval"
        );

        let mut malformed_source = parked.clone();
        malformed_source
            .continuations
            .iter_mut()
            .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
            .unwrap()
            .fields
            .insert("source".to_owned(), json!("Draw pile flip"));
        cold_refuses(&malformed_source);

        let mut malformed_order = parked.clone();
        let entries = malformed_order
            .continuations
            .iter_mut()
            .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
            .unwrap()
            .fields
            .get_mut("entries")
            .unwrap()
            .as_array_mut()
            .unwrap();
        entries.swap(0, 1);
        cold_refuses(&malformed_order);

        let mut missing_later = parked.clone();
        missing_later
            .piles
            .get_mut("discard")
            .unwrap()
            .retain(|card| card.uid != Some(3));
        cold_refuses(&missing_later);

        let mut forged_caller = parked.clone();
        forged_caller.continuations[1]
            .fields
            .insert("source".to_owned(), json!("auto"));
        cold_refuses(&forged_caller);
    }
}

#[test]
fn replayed_rooted_hologram_retrieval_reloads_each_choice() {
    let doc = with_base_replay(&entry("HOLOGRAM", 1), 1);
    let (mut parked, _) = cold_apply(
        &doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    for choice_count in 0..2 {
        reload_admitted(&parked);
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert!(
            !choices.is_empty(),
            "replay {choice_count} retains the public retrieval choice"
        );
        (parked, _) = cold_apply(&parked, choices[0]);
    }
    assert!(!parked.player.contains_key("pending"));
    reload_admitted(&parked);
}

#[test]
fn terminal_retrieved_sly_child_stops_the_later_captured_slot() {
    let mut doc = entry("HOLOGRAM", 0);
    let hand = doc.piles.get_mut("hand").unwrap();
    hand.swap(1, 2);
    hand[2].id = "FLICK_FLACK".to_owned();
    hand[2].local_keywords.clear();
    doc.monsters[0].insert("hp".to_owned(), json!(1));
    doc.monsters[0].insert("max_hp".to_owned(), json!(1));
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
    let terminal = engine::legal_actions(&state, &catalog)
        .into_iter()
        .find_map(|choice| {
            let (candidate, events) = cold_apply(&parked, choice);
            (candidate.monsters[0]["hp"]
                .as_i64()
                .is_some_and(|hp| hp <= 0))
            .then_some((candidate, events))
        })
        .expect("one Hologram choice retrieves the lethal Sly Flick Flack");
    assert!(!terminal.0.player.contains_key("pending"));
    assert!(
        terminal
            .1
            .iter()
            .any(|event| matches!(event, Event::CardPlayed { uid: 2, .. }))
    );
    assert!(
        terminal
            .1
            .iter()
            .all(|event| !matches!(event, Event::CardPlayed { uid: 3, .. }))
    );
}

fn scrape_entry(id: &str) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(5));
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
        "hand": [{"id":"SCRAPE","upgrade":0,"uid":0}],
        "draw": [
            {"id":id,"upgrade":0,"uid":1,"local_keywords":["Sly"]},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2,"local_keywords":["Sly"]},
            {"id":"DEFEND_SILENT","upgrade":0,"uid":3},
            {"id":"STRIKE_DEFECT","upgrade":0,"uid":4}
        ],
        "discard": []
    }))
    .unwrap();
    doc
}

fn select_uid(doc: &CanonicalStateV2, wanted: u32) -> Action {
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
                .is_some_and(|uids| uids == [wanted])
        })
        .unwrap_or_else(|| panic!("missing public selector for uid {wanted}"))
}

#[test]
fn corrected_scrape_returns_a_nonzero_sly_retrieval_child_to_its_frozen_later_slot() {
    for (id, retrieved_pile) in [
        ("HOLOGRAM", sts_sim::hot::PileId::Hand),
        // #3151: Cosmic Indifference adds its choice to Draw/Top.
        ("COSMIC_INDIFFERENCE", sts_sim::hot::PileId::Draw),
    ] {
        let doc = scrape_entry(id);
        reload_admitted(&doc);
        let (parked, first_events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        );
        reload_admitted(&parked);
        assert!(parked.player.contains_key("pending"), "{id}");
        let (done, later_events) = cold_apply(&parked, select_uid(&parked, 2));
        reload_admitted(&done);
        assert!(!done.player.contains_key("pending"));
        let mut events = first_events;
        events.extend(later_events);
        let played: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::CardPlayed { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect();
        assert_eq!(played, vec![0, 1, 2], "{id} retains Scrape's frozen order");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    Event::CardResolved { uid: 2, pile }
                        if *pile == retrieved_pile
                ))
                .count(),
            1,
            "{id} retrieves the later nonzero-cost Sly sibling first"
        );
        assert!(
            done.piles
                .get("discard")
                .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(2))),
            "the retrieved sibling executes at its original batch slot"
        );
    }
}

/// #3151 witness: an ordinary (non-Sly) Cosmic Indifference puts its Discard
/// choice on top of Draw, not at the bottom of Hand (Hologram's destination).
///
/// Current `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `CosmicIndifference/<OnPlay>d__5::MoveNext` RVA `0x3950c0` IL_01a5-IL_01ab
/// calls `CardPileCmd.Add(card, 1 = PileType.Draw, 2 = CardPilePosition.Top)`.
/// Draw index 0 is the pile top (`draw.rs` removes index 0 when drawing).
#[test]
fn cosmic_indifference_moves_its_discard_choice_to_the_top_of_draw() {
    for upgrade in [0, 1] {
        let mut doc = entry("COSMIC_INDIFFERENCE", upgrade);
        doc.piles = serde_json::from_value(json!({
            "hand": [{"id": "COSMIC_INDIFFERENCE", "upgrade": upgrade, "uid": 0}],
            "draw": [
                {"id": "DEFEND_IRONCLAD", "upgrade": 0, "uid": 1},
                {"id": "DEFEND_IRONCLAD", "upgrade": 0, "uid": 2}
            ],
            "discard": [
                {"id": "STRIKE_IRONCLAD", "upgrade": 0, "uid": 3},
                {"id": "BASH", "upgrade": 0, "uid": 4}
            ]
        }))
        .unwrap();
        doc.player.insert("next_card_uid".to_owned(), json!(5));
        reload_admitted(&doc);
        let (parked, _) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert!(parked.player.contains_key("pending"));
        let (done, events) = cold_apply(&parked, select_uid(&parked, 4));
        reload_admitted(&done);
        assert!(events.iter().any(|event| matches!(
            event,
            Event::CardResolved {
                uid: 4,
                pile: sts_sim::hot::PileId::Draw
            }
        )));
        let catalog = HotBoundary::catalog_from_canonical(&done).unwrap();
        let state = HotBoundary::from_canonical(&done, &catalog).unwrap();
        assert_eq!(
            state
                .piles
                .get(sts_sim::hot::PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [4, 1, 2],
            "Cosmic Indifference+{upgrade} puts the choice on top of Draw"
        );
        assert!(
            state
                .piles
                .get(sts_sim::hot::PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| card.uid != 4)
        );
    }
}
