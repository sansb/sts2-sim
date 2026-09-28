//! Recursive Sly Hidden Daggers (#2677).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
//! `HiddenDaggers/<OnPlay>d__6::MoveNext` (RVA `0x3a5480`) awaits the Hand
//! choice at IL_0057, awaits plural `CardCmd::Discard` (including every
//! nested Sly child) at IL_00c0, creates the Shivs in hand at IL_013f, and
//! upgrades only the freshly returned objects when the source is upgraded
//! (IL_01a3..01c1). A nested Sly Hidden Daggers child therefore suspends on
//! its own manual two-card choice (both pick orders, #2675), completes its
//! inner batch, generates exactly two Shivs, and returns once to the outer
//! frozen cursor.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionAnswer, SelectionRef},
};

fn entry(hand: serde_json::Value) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(30));
    // Master Planner provenance for the hand-set local Sly keywords.
    doc.player.insert("master_planner".to_owned(), json!(1));
    doc.player.insert(
        "after_card_played_power_order".to_owned(),
        json!([["master_planner", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
    // Two same-identity Hidden Daggers with divergent Sly state need exact piles.
    doc.player.insert("exact_piles".to_owned(), json!(true));
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".to_owned(), json!(100));
    doc.monsters[0].insert("max_hp".to_owned(), json!(100));
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": [],
        "discard": []
    }))
    .unwrap();
    doc
}

fn main_hand(outer_upgrade: u8, inner_upgrade: u8) -> serde_json::Value {
    json!([
        {"id":"HIDDEN_DAGGERS","upgrade":outer_upgrade,"uid":0},
        {"id":"HIDDEN_DAGGERS","upgrade":inner_upgrade,"uid":1,"local_keywords":["Sly"]},
        {"id":"DEFEND_SILENT","upgrade":0,"uid":2,"local_keywords":["Sly"]},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":3},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":4},
        {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":5}
    ])
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

fn played(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::CardPlayed { uid, .. } => Some(*uid),
            _ => None,
        })
        .collect()
}

fn picked_uids(doc: &CanonicalStateV2, action: &Action) -> Vec<u32> {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    let Action::Select { answer } = *action else {
        panic!("expected a selection answer");
    };
    engine::selected_card_uids(&state, &catalog, answer)
        .unwrap()
        .expect("Hidden Daggers choice has physical UIDs")
}

fn select_exact_uids(doc: &CanonicalStateV2, wanted: &[u32]) -> Action {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| picked_uids(doc, action) == wanted)
        .unwrap_or_else(|| panic!("missing exact physical answer {wanted:?}"))
}

fn play_uid(uid: u32) -> Action {
    Action::Play {
        uid,
        target: None,
        selection: SelectionRef::NONE,
    }
}

/// (upgrade, uid) of every Shiv across Hand and Discard.
fn shivs(doc: &CanonicalStateV2) -> Vec<(u64, i64)> {
    ["hand", "discard"]
        .into_iter()
        .flat_map(|pile| doc.piles.get(pile).unwrap().iter())
        .filter(|card| card.id == "SHIV")
        .map(|card| (card.uid.unwrap(), card.upgrade))
        .collect()
}

#[test]
fn nested_hidden_daggers_completes_through_both_pick_orders_at_both_upgrades() {
    for (outer_upgrade, inner_upgrade) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let tag = format!("outer+{outer_upgrade}/inner+{inner_upgrade}");
        let doc = entry(main_hand(outer_upgrade, inner_upgrade));
        reload_admitted(&doc);
        let (parked, first_events) = cold_apply(&doc, play_uid(0));
        assert!(
            parked.player.contains_key("pending"),
            "{tag}: outer suspends"
        );
        reload_admitted(&parked);
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let parked_state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let outer_actions = engine::legal_actions(&parked_state, &catalog);
        assert_eq!(
            outer_actions.len(),
            20,
            "{tag}: 5 candidates choose 2 in both orders"
        );
        // Every outer answer discarding the inner child parks the nested
        // selector; answers avoiding it complete without inner work.
        for outer_action in &outer_actions {
            let outer_picked = picked_uids(&parked, outer_action);
            let (mid, mid_events) = cold_apply(&parked, *outer_action);
            if !outer_picked.contains(&1) {
                assert!(
                    !mid.player.contains_key("pending"),
                    "{tag}: outer answer {outer_picked:?} avoids the nested child"
                );
                continue;
            }
            assert!(mid.player.contains_key("pending"), "{tag}: nested suspends");
            reload_admitted(&mid);
            let mid_catalog = HotBoundary::catalog_from_canonical(&mid).unwrap();
            let mid_state = HotBoundary::from_canonical(&mid, &mid_catalog).unwrap();
            let inner_actions = engine::legal_actions(&mid_state, &mid_catalog);
            assert_eq!(
                inner_actions.len(),
                6,
                "{tag}: 3 remaining candidates choose 2 in both orders"
            );
            // First answer and its reverse both complete the whole
            // trajectory with identical Shiv multisets. Either level can
            // discard the later sibling; each discard plays it exactly once.
            let first = picked_uids(&mid, &inner_actions[0]);
            let mut reversed = first.clone();
            reversed.reverse();
            for wanted in [first, reversed] {
                let inner_action = select_exact_uids(&mid, &wanted);
                let expected_sibling_plays =
                    usize::from(outer_picked.contains(&2)) + usize::from(wanted.contains(&2));
                let (done, later_events) = cold_apply(&mid, inner_action);
                reload_admitted(&done);
                assert!(
                    !done.player.contains_key("pending"),
                    "{tag}: nested answer {wanted:?} returns through the outer cursor"
                );
                let mut shiv_upgrades: Vec<i64> =
                    shivs(&done).iter().map(|(_, upgrade)| *upgrade).collect();
                shiv_upgrades.sort_unstable();
                let mut expected = vec![
                    i64::from(inner_upgrade),
                    i64::from(inner_upgrade),
                    i64::from(outer_upgrade),
                    i64::from(outer_upgrade),
                ];
                expected.sort_unstable();
                assert_eq!(
                    shiv_upgrades, expected,
                    "{tag}: exactly two Shivs per generation, upgraded only from an upgraded source"
                );
                // Parent and nested child always play exactly once; the
                // later Sly sibling plays exactly once when the outer
                // answer discards it and otherwise stays in hand.
                let mut all_events = first_events.clone();
                all_events.extend(mid_events.clone());
                all_events.extend(later_events);
                for uid in [0, 1] {
                    assert_eq!(
                        played(&all_events)
                            .iter()
                            .filter(|played| **played == uid)
                            .count(),
                        1,
                        "{tag}: card {uid} plays exactly once"
                    );
                }
                let sibling_plays = played(&all_events)
                    .iter()
                    .filter(|played| **played == 2)
                    .count();
                assert_eq!(
                    sibling_plays, expected_sibling_plays,
                    "{tag}: sibling plays once per discard, outer {outer_picked:?} inner {wanted:?}"
                );
                if expected_sibling_plays == 0 {
                    assert!(
                        done.piles["hand"].iter().any(|card| card.uid == Some(2)),
                        "{tag}: kept sibling stays in hand"
                    );
                }
                // Exact identity, randomness, and discard accounting.
                assert_eq!(
                    done.player["next_card_uid"],
                    json!(34),
                    "{tag}: four generated Shivs consume four UIDs"
                );
                assert_eq!(done.rng, doc.rng, "{tag}: no RNG consumed");
                assert_eq!(
                    done.player["discarded_cards_this_turn"],
                    json!(4),
                    "{tag}: two outer plus two inner discards"
                );
            }
        }
    }
}

#[test]
fn nested_hidden_daggers_third_level_sly_follows_inner_answer_order() {
    // Both Sly Defends survive the outer discard, so the inner choice
    // discards two live Sly cards in both orders.
    let doc = entry(json!([
        {"id":"HIDDEN_DAGGERS","upgrade":0,"uid":0},
        {"id":"HIDDEN_DAGGERS","upgrade":1,"uid":1,"local_keywords":["Sly"]},
        {"id":"DEFEND_SILENT","upgrade":0,"uid":2,"local_keywords":["Sly"]},
        {"id":"DEFEND_SILENT","upgrade":0,"uid":3,"local_keywords":["Sly"]},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":4},
        {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":5}
    ]));
    reload_admitted(&doc);
    let (parked, first_events) = cold_apply(&doc, play_uid(0));
    let mid_action = select_exact_uids(&parked, &[1, 4]);
    let (mid, _) = cold_apply(&parked, mid_action);
    assert!(mid.player.contains_key("pending"));
    reload_admitted(&mid);
    for wanted in [vec![2, 3], vec![3, 2]] {
        let inner_action = select_exact_uids(&mid, &wanted);
        let (done, later_events) = cold_apply(&mid, inner_action);
        reload_admitted(&done);
        assert!(!done.player.contains_key("pending"));
        let mut all_events = first_events.clone();
        all_events.extend(later_events);
        let third_level: Vec<u32> = played(&all_events)
            .into_iter()
            .filter(|uid| *uid == 2 || *uid == 3)
            .collect();
        assert_eq!(
            third_level, wanted,
            "third-level Sly children run in inner answer order"
        );
        let mut shiv_upgrades: Vec<i64> =
            shivs(&done).iter().map(|(_, upgrade)| *upgrade).collect();
        shiv_upgrades.sort_unstable();
        assert_eq!(shiv_upgrades, vec![0, 0, 1, 1]);
    }
}

#[test]
fn nested_hidden_daggers_terminal_position_and_hand_cap_redirect() {
    // Inner child discarded last still completes identically.
    let doc = entry(main_hand(0, 0));
    let (parked, _) = cold_apply(&doc, play_uid(0));
    let (mid, _) = cold_apply(&parked, select_exact_uids(&parked, &[2, 1]));
    assert!(mid.player.contains_key("pending"));
    reload_admitted(&mid);
    let (done, events) = cold_apply(&mid, select_exact_uids(&mid, &[3, 4]));
    reload_admitted(&done);
    assert!(!done.player.contains_key("pending"));
    assert_eq!(shivs(&done).len(), 4);
    assert!(
        played(&events).is_empty(),
        "inner completion plays no further cards in this window"
    );

    // Fourteen-card hand: the inner Shivs land on a full hand, so Add's
    // live Hand-cap redirect carries the overflow to Discard at both levels.
    let mut hand = vec![
        json!({"id":"HIDDEN_DAGGERS","upgrade":0,"uid":0}),
        json!({"id":"HIDDEN_DAGGERS","upgrade":0,"uid":1,"local_keywords":["Sly"]}),
        json!({"id":"DEFEND_SILENT","upgrade":0,"uid":2,"local_keywords":["Sly"]}),
    ];
    for uid in 3..14 {
        hand.push(json!({"id":"STRIKE_IRONCLAD","upgrade":0,"uid":uid}));
    }
    let full = entry(serde_json::Value::Array(hand));
    reload_admitted(&full);
    let (parked, _) = cold_apply(&full, play_uid(0));
    let (mid, _) = cold_apply(&parked, select_exact_uids(&parked, &[1, 2]));
    assert!(mid.player.contains_key("pending"));
    reload_admitted(&mid);
    let inner_actions = {
        let catalog = HotBoundary::catalog_from_canonical(&mid).unwrap();
        let state = HotBoundary::from_canonical(&mid, &catalog).unwrap();
        engine::legal_actions(&state, &catalog).len()
    };
    assert_eq!(inner_actions, 110, "11 candidates choose 2 in both orders");
    let (done, _) = cold_apply(&mid, select_exact_uids(&mid, &[12, 13]));
    reload_admitted(&done);
    let hand_shivs = done.piles["hand"]
        .iter()
        .filter(|card| card.id == "SHIV")
        .count();
    let discard_shivs = done.piles["discard"]
        .iter()
        .filter(|card| card.id == "SHIV")
        .count();
    assert_eq!(done.piles["hand"].len(), 10);
    assert_eq!(hand_shivs, 1, "one inner Shiv fits the full hand");
    assert_eq!(
        discard_shivs, 3,
        "the other inner Shiv and both outer Shivs take the redirect"
    );
}

#[test]
fn nested_hidden_daggers_refuses_forged_receipts_tools_and_unrelated_children() {
    let doc = entry(main_hand(0, 0));
    let (parked, _) = cold_apply(&doc, play_uid(0));
    let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
    let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
    let before = state.clone();
    let mut events = Vec::new();
    // Out-of-range outer ordinal is atomic.
    assert!(
        engine::apply_action_into(
            &state,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(u32::MAX),
            },
            &mut events,
        )
        .is_err()
    );
    assert_eq!(state, before, "invalid outer answer is atomic");
    assert!(events.is_empty());
    // Out-of-range inner ordinal is atomic.
    let (mid, _) = cold_apply(&parked, select_exact_uids(&parked, &[1, 2]));
    let mid_catalog = HotBoundary::catalog_from_canonical(&mid).unwrap();
    let mid_state = HotBoundary::from_canonical(&mid, &mid_catalog).unwrap();
    let mid_before = mid_state.clone();
    assert!(
        engine::apply_action_into(
            &mid_state,
            &mid_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(u32::MAX),
            },
            &mut Vec::new(),
        )
        .is_err()
    );
    assert_eq!(mid_state, mid_before, "invalid inner answer is atomic");

    // Forged Sly-discard source, cursor, candidate, and root, taken on the
    // inner-parked doc where the frozen Sly batch frame is live.
    let (mid, _) = cold_apply(&parked, select_exact_uids(&parked, &[1, 2]));
    assert!(mid.player.contains_key("pending"));
    reload_admitted(&mid);

    let mut forged_source = mid.clone();
    forged_source
        .continuations
        .iter_mut()
        .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
        .unwrap()
        .fields
        .insert("source".to_owned(), json!("Tools of the Trade"));
    cold_refuses(&forged_source);

    let mut forged_cursor = mid.clone();
    forged_cursor
        .continuations
        .iter_mut()
        .find(|frame| frame.fields.get("source") == Some(&json!("Sly discard")))
        .unwrap()
        .fields
        .insert("cursor".to_owned(), json!(99));
    cold_refuses(&forged_cursor);

    let mut missing_candidate = mid.clone();
    missing_candidate
        .piles
        .get_mut("discard")
        .unwrap()
        .retain(|card| card.uid != Some(2));
    cold_refuses(&missing_candidate);

    let mut forged_root = mid.clone();
    forged_root.continuations[1]
        .fields
        .insert("source".to_owned(), json!("auto"));
    cold_refuses(&forged_root);

    // An unrelated Sly child in the nested slot stays refused.
    let mut unrelated = entry(main_hand(0, 0));
    unrelated
        .piles
        .get_mut("hand")
        .unwrap()
        .iter_mut()
        .find(|card| card.uid == Some(1))
        .unwrap()
        .id = "GLIMMER".to_owned();
    cold_refuses(&unrelated);

    // Plural Tools, the wall pin this slice moves (#2671 PR 2).
    //
    // BEFORE: this document refused outright, because a Tools listener at
    // amount two with two selecting Sly siblings tripped the replay branch's
    // blanket `effective_sly.len() > 1` cardinality disjunct.
    //
    // NOW: the batch is authenticated per child, and both siblings here
    // (`HIDDEN_DAGGERS`, `DEFEND_SILENT`) are certified, so it admits. The
    // refusal moves onto the child grammar: the same document with an
    // uncertified selecting Sly sibling still refuses. Every other assertion in
    // this test is unchanged.
    let tools_document = |sly_sibling: &str| {
        let mut hand = main_hand(0, 0);
        hand.as_array_mut().unwrap()[2] = json!({
            "id": sly_sibling, "upgrade": 0, "uid": 2, "local_keywords": ["Sly"]
        });
        let mut doc = entry(hand);
        doc.player.insert("tools_of_the_trade".to_owned(), json!(2));
        doc.player.insert(
            "turn_start_hand_choice_order".to_owned(),
            json!(["tools_of_the_trade"]),
        );
        doc
    };

    let certified = tools_document("DEFEND_SILENT");
    let catalog = HotBoundary::catalog_from_canonical(&certified).unwrap();
    let state = HotBoundary::from_canonical(&certified, &catalog).unwrap();
    engine::admit(&certified, &state, &catalog)
        .expect("a certified plural Tools batch admits since #2671 PR 2");

    cold_refuses(&tools_document("HEADBUTT"));
}
