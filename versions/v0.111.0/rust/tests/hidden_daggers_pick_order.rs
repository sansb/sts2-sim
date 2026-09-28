//! Hidden Daggers admits both manual pick orders of its exact two-card
//! Discard choice (#2675).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
//! `HiddenDaggers/<OnPlay>d__6::MoveNext` (RVA `0x3a5480`) awaits
//! `FromHandForDiscard` at IL_0057 and passes the same returned enumerable
//! to plural `CardCmd::Discard` at IL_00c0, which captures Sly children in
//! input order. Reversed answers therefore produce reversed Discard order
//! and reversed two-child Sly execution. Since #2524 every multi-pick
//! Discard/Exhaust select enumerates its pick orders (Prepared+ is pinned in
//! `discard_exhaust_pick_order.rs`), and automatic answers with at most two
//! candidates stay a single pile-order action.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionAnswer, SelectionRef},
};

fn hidden_daggers_entry(hand: serde_json::Value) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(20));
    // Master Planner provenance for the hand-set local Sly keywords, mirroring
    // the rooted Sly retrieval tests.
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
        "hand": hand,
        "draw": [],
        "discard": []
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

fn discard_uids(doc: &CanonicalStateV2) -> Vec<u64> {
    doc.piles
        .get("discard")
        .map(|pile| pile.iter().filter_map(|card| card.uid).collect())
        .unwrap_or_default()
}

fn sly_hand(upgrade: u8) -> serde_json::Value {
    json!([
        {"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0},
        {"id":"DEFEND_SILENT","upgrade":0,"uid":1,"local_keywords":["Sly"]},
        {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2,"local_keywords":["Sly"]},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":3}
    ])
}

fn plain_hand(upgrade: u8) -> serde_json::Value {
    json!([
        {"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0},
        {"id":"DEFEND_SILENT","upgrade":0,"uid":1},
        {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":3}
    ])
}

fn play_hidden_daggers(doc: &CanonicalStateV2) -> (CanonicalStateV2, Vec<Event>) {
    cold_apply(
        doc,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
}

fn suspended_actions(parked: &CanonicalStateV2) -> Vec<Action> {
    assert!(
        parked.player.contains_key("pending"),
        "Hidden Daggers must suspend on a manual two-card choice"
    );
    reload_admitted(parked);
    let catalog = HotBoundary::catalog_from_canonical(parked).unwrap();
    let state = HotBoundary::from_canonical(parked, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
}

fn picked_uids(parked: &CanonicalStateV2, action: &Action) -> Vec<u32> {
    let catalog = HotBoundary::catalog_from_canonical(parked).unwrap();
    let state = HotBoundary::from_canonical(parked, &catalog).unwrap();
    let Action::Select { answer } = *action else {
        panic!("Hidden Daggers choice is a selection answer");
    };
    engine::selected_card_uids(&state, &catalog, answer)
        .unwrap()
        .expect("Hidden Daggers choice has physical UIDs")
}

#[test]
fn hidden_daggers_manual_two_card_choice_enumerates_both_pick_orders() {
    for upgrade in 0..=1 {
        // Plain (non-Sly) candidates, so the final Discard pile order is the
        // answer order with no Sly replay interleaving; Sly execution order
        // is pinned by the reversed-answers test below.
        let doc = hidden_daggers_entry(plain_hand(upgrade));
        reload_admitted(&doc);
        let (parked, _) = play_hidden_daggers(&doc);
        let actions = suspended_actions(&parked);
        // Three candidates choose two, in both pick orders.
        assert_eq!(
            actions.len(),
            6,
            "HiddenDaggers+{upgrade}: 3 choose 2, times 2 orders"
        );
        let mut picked: Vec<Vec<u32>> = actions
            .iter()
            .map(|action| picked_uids(&parked, action))
            .collect();
        picked.sort_unstable();
        assert_eq!(
            picked,
            vec![
                vec![1, 2],
                vec![1, 3],
                vec![2, 1],
                vec![2, 3],
                vec![3, 1],
                vec![3, 2],
            ],
            "HiddenDaggers+{upgrade}: every unordered pair in both orders"
        );
        // Cold reload preserves the action surface, and each ordinal replays
        // to the same result hot and cold.
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let hot_state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let hot_actions = engine::legal_actions(&hot_state, &catalog);
        assert_eq!(hot_actions, actions);
        for action in &actions {
            let picked = picked_uids(&parked, action);
            assert_eq!(picked.len(), 2);
            let (hot_done, _) = cold_apply(&parked, *action);
            reload_admitted(&hot_done);
            assert!(
                !hot_done.player.contains_key("pending"),
                "answer completes Hidden Daggers"
            );
            // The Discard pile order is the answer order.
            let discarded = discard_uids(&hot_done);
            let answer_position = |uid: u32| {
                discarded
                    .iter()
                    .position(|discarded| *discarded == u64::from(uid))
                    .unwrap()
            };
            assert!(
                answer_position(picked[0]) < answer_position(picked[1]),
                "HiddenDaggers+{upgrade}: Discard order follows the answer order"
            );
        }
    }
}

#[test]
fn hidden_daggers_reversed_answers_reverse_discard_and_sly_execution() {
    for upgrade in 0..=1 {
        let doc = hidden_daggers_entry(sly_hand(upgrade));
        let (parked, first_events) = play_hidden_daggers(&doc);
        let actions = suspended_actions(&parked);
        let mut by_answer: std::collections::BTreeMap<Vec<u32>, Action> =
            std::collections::BTreeMap::new();
        for action in actions {
            by_answer.insert(picked_uids(&parked, &action), action);
        }
        let forward = by_answer[&vec![1, 2]];
        let reverse = by_answer[&vec![2, 1]];
        let (forward_done, forward_events) = cold_apply(&parked, forward);
        let (reverse_done, reverse_events) = cold_apply(&parked, reverse);
        let position_of = |done: &CanonicalStateV2, uid: u64| {
            discard_uids(done)
                .iter()
                .position(|discarded| *discarded == uid)
                .unwrap()
        };
        assert!(
            position_of(&forward_done, 1) < position_of(&forward_done, 2),
            "HiddenDaggers+{upgrade}: [1, 2] discards 1 before 2"
        );
        assert!(
            position_of(&reverse_done, 2) < position_of(&reverse_done, 1),
            "HiddenDaggers+{upgrade}: [2, 1] discards 2 before 1"
        );
        // Both Sly children autoplay, in discard order, after the parent.
        let mut forward_played = first_events.clone();
        forward_played.extend(forward_events);
        let mut reverse_played = first_events.clone();
        reverse_played.extend(reverse_events);
        let forward_sly: Vec<u32> = played(&forward_played)
            .into_iter()
            .filter(|uid| *uid == 1 || *uid == 2)
            .collect();
        let reverse_sly: Vec<u32> = played(&reverse_played)
            .into_iter()
            .filter(|uid| *uid == 1 || *uid == 2)
            .collect();
        assert_eq!(
            forward_sly,
            vec![1, 2],
            "HiddenDaggers+{upgrade}: Sly children run in answer order"
        );
        assert_eq!(
            reverse_sly,
            vec![2, 1],
            "HiddenDaggers+{upgrade}: reversed answer reverses Sly execution"
        );
        assert_eq!(
            played(&forward_played)[0],
            0,
            "HiddenDaggers+{upgrade}: parent plays before its Sly children"
        );
    }
}

#[test]
fn hidden_daggers_duplicate_payloads_keep_distinct_ordered_answers() {
    for upgrade in 0..=1 {
        let doc = hidden_daggers_entry(json!([
            {"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":1},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":2},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":3}
        ]));
        let (parked, _) = play_hidden_daggers(&doc);
        let actions = suspended_actions(&parked);
        assert_eq!(
            actions.len(),
            6,
            "HiddenDaggers+{upgrade}: duplicate payloads still fan out"
        );
        let mut picked: Vec<Vec<u32>> = actions
            .iter()
            .map(|action| picked_uids(&parked, action))
            .collect();
        picked.sort_unstable();
        assert!(
            picked.contains(&vec![1, 2]) && picked.contains(&vec![2, 1]),
            "HiddenDaggers+{upgrade}: equal-payload UIDs keep both orders"
        );
        // The two orders land the duplicate payloads in different Discard positions.
        let answer_for = |wanted: Vec<u32>| {
            actions
                .iter()
                .find(|action| picked_uids(&parked, action) == wanted)
                .copied()
                .unwrap()
        };
        let (forward_done, _) = cold_apply(&parked, answer_for(vec![1, 2]));
        let (reverse_done, _) = cold_apply(&parked, answer_for(vec![2, 1]));
        assert_ne!(
            discard_uids(&forward_done),
            discard_uids(&reverse_done),
            "HiddenDaggers+{upgrade}: duplicate-payload orders are observably different"
        );
    }
}

#[test]
fn hidden_daggers_small_hands_auto_answer_without_suspending() {
    for upgrade in 0..=1 {
        // Zero candidates.
        let solo = hidden_daggers_entry(json!([{"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0}]));
        let (done, _) = play_hidden_daggers(&solo);
        assert!(
            !done.player.contains_key("pending"),
            "HiddenDaggers+{upgrade}: empty hand completes"
        );
        // One candidate.
        let single = hidden_daggers_entry(json!([
            {"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":1}
        ]));
        let (done, _) = play_hidden_daggers(&single);
        assert!(
            !done.player.contains_key("pending"),
            "HiddenDaggers+{upgrade}: single candidate completes"
        );
        assert_eq!(
            discard_uids(&done).iter().filter(|uid| **uid == 1).count(),
            1,
            "HiddenDaggers+{upgrade}: lone candidate is discarded"
        );
        // Exactly two candidates: one automatic pile-order action, no choice.
        let pair = hidden_daggers_entry(json!([
            {"id":"HIDDEN_DAGGERS","upgrade":upgrade,"uid":0},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":1},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2}
        ]));
        let (done, _) = play_hidden_daggers(&pair);
        assert!(
            !done.player.contains_key("pending"),
            "HiddenDaggers+{upgrade}: exact pair auto-answers"
        );
        let discarded = discard_uids(&done);
        assert!(
            discarded.contains(&1) && discarded.contains(&2),
            "HiddenDaggers+{upgrade}: exact pair discards both candidates"
        );
    }
}

#[test]
fn hidden_daggers_malformed_answer_is_refused_atomically() {
    let doc = hidden_daggers_entry(sly_hand(0));
    let (parked, _) = play_hidden_daggers(&doc);
    let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
    let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
    let before = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let bad = Action::Select {
        answer: SelectionAnswer::OptionIndex(6),
    };
    assert!(
        engine::apply_action_into(&state, &catalog, &bad, &mut Vec::new()).is_err(),
        "out-of-range Hidden Daggers ordinal is refused"
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        before,
        "refused answer leaves state untouched"
    );
}
