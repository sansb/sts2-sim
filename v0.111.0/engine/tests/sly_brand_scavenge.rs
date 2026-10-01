//! Rooted Sly Brand/Scavenge Exhaust continuations retain their physical owner.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
    hot::PileId,
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
            {"id": "SURVIVOR", "upgrade": 0, "uid": 0},
            {"id": id, "upgrade": upgrade, "uid": 1, "local_keywords": ["Sly"]},
            {"id": "STRIKE_SILENT", "upgrade": 0, "uid": 2},
            {"id": "DEFEND_SILENT", "upgrade": 0, "uid": 3}
        ],
        "draw": [], "discard": []
    }))
    .unwrap();
    doc
}

fn plural_entry(id: &str, upgrade: u8) -> CanonicalStateV2 {
    let mut doc = entry(id, upgrade);
    doc.player.insert("next_card_uid".to_owned(), json!(7));
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"PREPARED","upgrade":1,"uid":0},
            {"id":id,"upgrade":upgrade,"uid":1,"local_keywords":["Sly"]},
            {"id":"TACTICIAN","upgrade":0,"uid":2,"local_keywords":["Sly"]},
            {"id":"STRIKE_SILENT","upgrade":0,"uid":3},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":4}
        ],
        "draw": [
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":5},
            {"id":"STRIKE_DEFECT","upgrade":0,"uid":6}
        ], "discard": []
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

fn select(index: u32) -> Action {
    Action::Select {
        answer: engine::SelectionAnswer::OptionIndex(index),
    }
}

fn select_uid(doc: &CanonicalStateV2, uid: u32) -> Action {
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
                .is_some_and(|uids| uids.contains(&uid))
        })
        .unwrap_or_else(|| panic!("missing public selector for uid {uid}"))
}

fn select_pair(doc: &CanonicalStateV2, wanted: [u32; 2]) -> Action {
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
                .is_some_and(|uids| {
                    uids.len() == wanted.len() && wanted.into_iter().all(|uid| uids.contains(&uid))
                })
        })
        .unwrap_or_else(|| panic!("missing public selector for {wanted:?}"))
}

/// Survivor's native Hand discard leaves two ordinary cards in Hand while the
/// selected local-Sly child runs from its rooted captured Discard batch.
fn parked_sly_child(doc: &CanonicalStateV2) -> (CanonicalStateV2, Vec<Event>) {
    let (outer, _) = cold_apply(
        doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    reload_admitted(&outer);
    cold_apply(&outer, select_uid(&outer, 1))
}

fn parked_plural_sly_child(doc: &CanonicalStateV2) -> (CanonicalStateV2, Vec<Event>) {
    let (outer, _) = cold_apply(
        doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    reload_admitted(&outer);
    cold_apply(&outer, select_pair(&outer, [1, 2]))
}

fn played(events: &[Event], uid: u32) -> usize {
    events
        .iter()
        .filter(
            |event| matches!(event, Event::CardPlayed { uid: candidate, .. } if *candidate == uid),
        )
        .count()
}

fn resolved_in(events: &[Event], uid: u32, pile: PileId) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::CardResolved { uid: candidate, pile: candidate_pile } if *candidate == uid && *candidate_pile == pile))
        .count()
}

#[test]
fn rooted_sly_brand_and_scavenge_exhaust_then_apply_their_exact_suffixes() {
    for (id, upgrade, suffix, amount) in [
        ("BRAND", 0, "strength", 1),
        ("BRAND", 1, "strength", 2),
        ("SCAVENGE", 0, "energy_next_turn", 2),
        ("SCAVENGE", 1, "energy_next_turn", 3),
    ] {
        let doc = entry(id, upgrade);
        reload_admitted(&doc);
        let (parked, first_events) = parked_sly_child(&doc);
        assert!(parked.player.contains_key("pending"), "{id}+{upgrade}");
        assert_eq!(played(&first_events, 1), 1, "{id}+{upgrade}");
        assert_eq!(
            parked.player.get("hp").and_then(serde_json::Value::as_i64),
            Some(if id == "BRAND" { 67 } else { 68 }),
            "Brand's HpLoss completes before its public selector"
        );
        assert!(
            !parked.player.contains_key(suffix),
            "the post-Exhaust suffix is not early for {id}+{upgrade}"
        );
        reload_admitted(&parked);

        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        assert_eq!(engine::legal_actions(&state, &catalog).len(), 2);
        let before = state.clone();
        let mut refusal_events = Vec::new();
        assert!(
            engine::apply_action_into(&state, &catalog, &select(u32::MAX), &mut refusal_events)
                .is_err()
        );
        assert_eq!(state, before, "invalid Exhaust answer is atomic");
        assert!(refusal_events.is_empty());

        let (done, second_events) = cold_apply(&parked, select_uid(&parked, 2));
        assert!(!done.player.contains_key("pending"));
        assert_eq!(done.player[suffix], json!(amount), "{id}+{upgrade}");
        assert_eq!(resolved_in(&second_events, 2, PileId::Exhaust), 1);
        assert_eq!(played(&second_events, 1), 0, "the source did not replay");
        assert!(
            done.piles
                .get("discard")
                .is_some_and(|pile| pile.iter().any(|card| card.uid == Some(1))),
            "the native Sly result route remains Discard"
        );
        reload_admitted(&done);
    }
}

#[test]
fn rooted_brand_exhaust_waits_for_cold_dark_embrace_stratagem_and_hellraiser() {
    let mut doc = plural_entry("BRAND", 0);
    doc.player.insert("dark_embrace".into(), json!(1));
    doc.player.insert("stratagem".into(), json!(1));
    doc.player.insert("hellraiser".into(), json!(1));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["dark_embrace", 1], ["hellraiser", 2]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(3));
    doc.player.insert(
        "after_card_exhausted_power_order".into(),
        json!(["dark_embrace"]),
    );
    doc.player.insert("next_card_uid".into(), json!(9));
    doc.piles.insert(
        "discard".into(),
        serde_json::from_value(json!([
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":7},
            {"id":"STRIKE_SILENT","upgrade":0,"uid":8}
        ]))
        .unwrap(),
    );

    let (parked, _) = parked_plural_sly_child(&doc);
    reload_admitted(&parked);
    let (nested, exhaust_events) = cold_apply(&parked, select_uid(&parked, 3));
    assert!(nested.player.contains_key("pending"));
    assert!(!nested.player.contains_key("strength"));
    assert_eq!(resolved_in(&exhaust_events, 3, PileId::Exhaust), 1);
    assert!(
        nested
            .continuations
            .iter()
            .any(|frame| frame.frame_type == "AfterCardExhaustedPowerFrame"),
        "the ordinary Exhaust return owner remains above the Draw"
    );
    reload_admitted(&nested);

    let (done, resume_events) = cold_apply(&nested, select(0));
    assert!(!done.player.contains_key("pending"));
    assert_eq!(done.player["strength"], json!(1));
    let hellraiser = resume_events
        .iter()
        .position(|event| matches!(event, Event::CardPlayed { uid: 6, .. }))
        .expect("Dark Embrace's drawn Strike replays under Hellraiser");
    let suffix = resume_events
        .iter()
        .position(|event| matches!(event, Event::PowerChanged { .. }))
        .expect("Brand applies Strength after the nested callbacks");
    assert!(hellraiser < suffix);
    assert_eq!(resolved_in(&resume_events, 1, PileId::Discard), 1);
    let sibling = resume_events
        .iter()
        .position(|event| matches!(event, Event::CardPlayed { uid: 2, .. }))
        .expect("the later captured Sly sibling still plays");
    assert!(
        suffix < sibling,
        "Brand's suffix precedes its later Sly sibling"
    );
    reload_admitted(&done);

    let mut malformed_owner = nested.clone();
    malformed_owner
        .continuations
        .iter_mut()
        .find(|frame| frame.frame_type == "AfterCardExhaustedPowerFrame")
        .unwrap()
        .fields
        .insert("source_uid".to_owned(), json!(0));
    cold_refuses(&malformed_owner);
}

#[test]
fn plural_prepared_sly_batch_runs_each_exhaust_suffix_before_its_later_sibling() {
    for (id, upgrade, suffix, amount) in [
        ("BRAND", 1, "strength", 2),
        ("SCAVENGE", 1, "energy_next_turn", 3),
    ] {
        let doc = plural_entry(id, upgrade);
        reload_admitted(&doc);
        let (parked, first_events) = parked_plural_sly_child(&doc);
        assert_eq!(played(&first_events, 1), 1);
        assert_eq!(
            played(&first_events, 2),
            0,
            "the later sibling is still parked"
        );
        reload_admitted(&parked);

        let (done, events) = cold_apply(&parked, select_uid(&parked, 3));
        assert_eq!(done.player[suffix], json!(amount));
        let suffix_event = events
            .iter()
            .position(|event| matches!(event, Event::PowerChanged { .. }))
            .expect("the selected source applies its printed suffix");
        let sibling_event = events
            .iter()
            .position(|event| matches!(event, Event::CardPlayed { uid: 2, .. }))
            .expect("the precommitted later Sly sibling runs once");
        assert!(suffix_event < sibling_event, "{id}+{upgrade}");
        assert_eq!(played(&events, 2), 1);
        reload_admitted(&done);
    }
}

#[test]
fn rooted_sly_exhaust_replay_and_cold_frames_reject_forged_parent_state() {
    let doc = entry("SCAVENGE", 1);
    let (parked, _) = parked_sly_child(&doc);
    reload_admitted(&parked);

    for (label, frame_type, field, value) in [
        (
            "batch source",
            "FrozenAutoBatchFrame",
            "source",
            json!("Draw pile flip"),
        ),
        ("batch cursor", "FrozenAutoBatchFrame", "cursor", json!(0)),
        ("child uid", "CardPlayFrame", "uid", json!(3)),
    ] {
        let mut forged = parked.clone();
        let frame = forged
            .continuations
            .iter_mut()
            .rev()
            .find(|frame| frame.frame_type == frame_type)
            .unwrap();
        frame.fields.insert(field.to_owned(), value);
        cold_refuses(&forged);
        assert!(!label.is_empty());
    }
}

#[test]
fn terminal_brand_before_an_empty_hand_selector_stops_later_sly_work() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(3));
    doc.player.insert("master_planner".to_owned(), json!(1));
    doc.player.insert(
        "after_card_played_power_order".to_owned(),
        json!([["master_planner", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
    doc.player.insert("hp".to_owned(), json!(1));
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"SHADOW_STEP","upgrade":1,"uid":0,"local_keywords":["Sly"]},
            {"id":"BRAND","upgrade":0,"uid":1,"local_keywords":["Sly"]},
            {"id":"DEFEND_SILENT","upgrade":0,"uid":2,"local_keywords":["Sly"]}
        ],
        "draw": [], "discard": []
    }))
    .unwrap();
    reload_admitted(&doc);
    let (done, events) = cold_apply(
        &doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    assert_eq!(
        done.player.get("hp").and_then(serde_json::Value::as_i64),
        None
    );
    assert_eq!(played(&events, 1), 1);
    assert_eq!(played(&events, 2), 0, "death suppresses the later Sly slot");
    assert!(
        !done.player.contains_key("pending"),
        "empty Hand never parks after death"
    );
}

#[test]
fn empty_hand_sly_exhaust_selectors_continue_directly_to_their_suffixes() {
    for (id, upgrade, suffix, amount) in [
        ("BRAND", 0, "strength", 1),
        ("BRAND", 1, "strength", 2),
        ("SCAVENGE", 0, "energy_next_turn", 2),
        ("SCAVENGE", 1, "energy_next_turn", 3),
    ] {
        let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        doc.player.insert("next_card_uid".to_owned(), json!(2));
        doc.player.insert("master_planner".to_owned(), json!(1));
        doc.player.insert(
            "after_card_played_power_order".to_owned(),
            json!([["master_planner", 0]]),
        );
        doc.player
            .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
        doc.piles = serde_json::from_value(json!({
            "hand": [
                {"id":"SHADOW_STEP","upgrade":1,"uid":0,"local_keywords":["Sly"]},
                {"id":id,"upgrade":upgrade,"uid":1,"local_keywords":["Sly"]}
            ],
            "draw": [], "discard": []
        }))
        .unwrap();
        reload_admitted(&doc);
        let (done, events) = cold_apply(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert!(!done.player.contains_key("pending"), "{id}+{upgrade}");
        assert_eq!(done.player[suffix], json!(amount), "{id}+{upgrade}");
        assert_eq!(played(&events, 1), 1, "{id}+{upgrade}");
        reload_admitted(&done);
    }
}

fn with_base_replay(doc: &CanonicalStateV2, uid: u32) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let mut state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    let mut physical = state.card_states.get(uid);
    physical.set_base_replay_count(Some(1)).unwrap();
    state.card_states.set(uid, physical);
    for pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
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
fn replayed_rooted_brand_reuses_its_physical_child_and_each_exhaust_suffix() {
    let doc = with_base_replay(&entry("BRAND", 0), 1);
    let (first, first_events) = parked_sly_child(&doc);
    reload_admitted(&first);
    let (done, replay_events) = cold_apply(&first, select_uid(&first, 2));
    assert!(!done.player.contains_key("pending"));
    assert_eq!(done.player["hp"], json!(66));
    assert_eq!(done.player["strength"], json!(2));
    assert_eq!(played(&first_events, 1), 1);
    assert_eq!(resolved_in(&replay_events, 2, PileId::Exhaust), 1);
    assert_eq!(resolved_in(&replay_events, 3, PileId::Exhaust), 1);
    reload_admitted(&done);
}
