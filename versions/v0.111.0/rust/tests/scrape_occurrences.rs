//! Scrape consumes an ordered returned list, not a set of physical UIDs.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, SelectionRef},
    hot::PileId,
};

fn entry(level: u8) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":2,
        "hellraiser":1,"after_side_turn_end_power_order":[["hellraiser",0]],
        "next_after_side_turn_end_power_uid":1
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc.piles = serde_json::from_value(json!({
        "hand":[{"id":"SCRAPE","uid":0,"upgrade":level}],
        "draw":[{"id":"STRIKE_IRONCLAD","uid":1,"upgrade":0}],"discard":[]
    }))
    .unwrap();
    doc
}

fn play(doc: &CanonicalStateV2) -> engine::Transition {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: Some(0),
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
}

fn wire(doc: &CanonicalStateV2, state: &sts_sim::hot::HotState) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    assert_eq!(cold, *state, "cold publication preserves exact state");
    if !state.history.over {
        let rebuilt_state = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
        engine::admit(&wire, &rebuilt_state, &rebuilt).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt_state, &rebuilt).unwrap(),
            wire
        );
    }
    wire
}

#[test]
fn ordinary_scrape_repeats_keep_one_card_and_every_discard_callback() {
    for level in 0..=1 {
        let mut doc = entry(level);
        doc.player.insert(
            "relics_entering".into(),
            json!(["RELIC.TINGSHA", "RELIC.TOUGH_BANDAGES"]),
        );
        doc.player.insert("tingsha".into(), json!(true));
        doc.player.insert("tough_bandages".into(), json!(true));
        let result = play(&doc);
        let count = 4 + i32::from(level);
        assert_eq!(
            i64::from(result.state.history.discarded_cards_this_turn),
            i64::from(count)
        );
        assert_eq!(
            result.state.monsters[0].hp,
            100 - (if level == 0 { 7 } else { 10 }) - count * 9
        );
        assert_eq!(result.state.block, count * 3);
        let wire = wire(&doc, &result.state);
        assert_eq!(
            wire.piles
                .values()
                .flatten()
                .filter(|card| card.uid == Some(1))
                .count(),
            1
        );
        let discarded = result
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::CardResolved {
                        uid: 1,
                        pile: PileId::Discard
                    }
                )
            })
            .count();
        assert_eq!(
            discarded,
            (count * 2) as usize,
            "each AutoPlay and each returned occurrence moves the same object"
        );
    }
}

#[test]
fn repeated_live_discard_repositions_in_returned_order() {
    let mut doc = entry(1);
    doc.player.insert("next_card_uid".into(), json!(3));
    doc.piles
        .get_mut("draw")
        .unwrap()
        .push(serde_json::from_value(json!({"id":"STRIKE_IRONCLAD","uid":2,"upgrade":0})).unwrap());
    let result = play(&doc);
    let drawn = result
        .events
        .iter()
        .filter_map(|event| {
            if let Event::CardDrawn { uid } = event {
                Some(*uid)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(drawn.len(), 5);
    let mut expected = Vec::new();
    for uid in drawn {
        expected.retain(|earlier| *earlier != uid);
        expected.push(uid);
    }
    expected.push(0);
    assert_eq!(
        result
            .state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect::<Vec<_>>(),
        expected
    );
    wire(&doc, &result.state);
}

#[test]
fn terminal_discard_hook_stops_moves_but_keeps_occurrence_history() {
    let mut doc = entry(0);
    doc.player.insert(
        "relics_entering".into(),
        json!(["RELIC.TINGSHA", "RELIC.TOUGH_BANDAGES"]),
    );
    doc.player.insert("tingsha".into(), json!(true));
    doc.player.insert("tough_bandages".into(), json!(true));
    doc.monsters[0].insert("hp".into(), json!(34));
    let result = play(&doc);
    assert!(result.state.history.over);
    assert_eq!(result.state.monsters[0].hp, 0);
    assert_eq!(result.state.history.discarded_cards_this_turn, 4);
    assert_eq!(
        result.state.block, 0,
        "Tingsha ends before the paired block callback"
    );
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(
                event,
                Event::CardResolved {
                    uid: 1,
                    pile: PileId::Discard
                }
            ))
            .count(),
        5,
        "four played routes plus one successful discard, then ending-gated Adds"
    );
    wire(&doc, &result.state);
}

#[test]
fn repeated_strike_and_unique_sly_skill_keep_existing_cold_choice_owner() {
    for level in 0..=1 {
        let mut doc = entry(0);
        doc.player.insert("next_card_uid".into(), json!(4));
        doc.player.insert("master_planner".into(), json!(1));
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["master_planner", 1]]),
        );
        doc.player
            .insert("next_after_side_turn_end_power_uid".into(), json!(2));
        doc.piles.insert(
            "draw".into(),
            serde_json::from_value(json!([
                {"id":"HOLOGRAM","uid":2,"upgrade":level,"local_keywords":["Sly"]},
                {"id":"STRIKE_IRONCLAD","uid":1,"upgrade":0},
                {"id":"DEFEND_IRONCLAD","uid":3,"upgrade":0}
            ]))
            .unwrap(),
        );
        let result = play(&doc);
        assert_eq!(result.state.history.discarded_cards_this_turn, 4);
        let parked = wire(&doc, &result.state);
        assert!(parked.player.contains_key("pending"));
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let cold = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let choices = engine::legal_actions(&cold, &catalog);
        assert_eq!(choices.len(), 2);
        for choice in choices {
            let next = engine::apply_action(&cold, &catalog, &choice)
                .unwrap()
                .state;
            let done = wire(&parked, &next);
            assert!(!done.player.contains_key("pending"));
            assert!(done.continuations.is_empty());
            assert_eq!(next.history.discarded_cards_this_turn, 4);
            assert_eq!(
                done.piles
                    .get(if level == 0 { "exhaust" } else { "discard" })
                    .unwrap()
                    .iter()
                    .filter(|card| card.uid == Some(2))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn hellraiser_strikes_cannot_enter_the_sly_occurrence_subset() {
    // Native Hellraiser 0x33c1a8 only AutoPlays Strike-tag objects. The current
    // row closure is Attack/non-Sly, and the two local writers (Master Planner
    // and Hand Trick) only apply to Skills. A new writer/row must re-audit the
    // unique Sly suffix proof before expanding admission.
    let strikes = sts_sim::content_tables::CARD_ROWS
        .iter()
        .filter(|row| row.strike_tag)
        .collect::<Vec<_>>();
    assert_eq!(strikes.len(), 44);
    for row in strikes {
        assert_eq!(row.card_type, sts_sim::content_tables::CardType::Attack);
        assert!(!row.sly && !row.is_skill);
    }
    for temporary in [false, true] {
        let mut doc = entry(0);
        doc.player.insert("master_planner".into(), json!(1));
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["master_planner", 1]]),
        );
        doc.player
            .insert("next_after_side_turn_end_power_uid".into(), json!(2));
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        if temporary {
            state.card_states.set_transient_sly(1);
        } else {
            state.card_states.set_local_sly(1);
        }
        for card in state.piles.get_mut(PileId::Draw).make_mut() {
            card.flags |= sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        }
        let forged = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let restored = HotBoundary::from_canonical(&forged, &catalog).unwrap();
        assert!(engine::admit(&forged, &restored, &catalog).is_err());
    }
}

#[test]
fn repeated_selecting_strike_authenticates_each_draw_pause_and_all_answers() {
    let mut doc = entry(0);
    doc.player.insert("next_card_uid".into(), json!(6));
    doc.piles.get_mut("draw").unwrap()[0].id = "SCULPTING_STRIKE".into();
    for uid in 2..6 {
        doc.piles.get_mut("hand").unwrap().push(
            serde_json::from_value(json!({"id":"DEFEND_IRONCLAD","uid":uid,"upgrade":0})).unwrap(),
        );
    }
    let result = play(&doc);
    let parked = wire(&doc, &result.state);
    let mut frontier = vec![parked];
    let mut terminals = 0;
    let mut saw_repeated = false;
    while let Some(current) = frontier.pop() {
        let catalog = HotBoundary::catalog_from_canonical(&current).unwrap();
        let state = HotBoundary::from_canonical(&current, &catalog).unwrap();
        engine::admit(&current, &state, &catalog).unwrap();
        if !current.player.contains_key("pending") {
            terminals += 1;
            assert_eq!(state.history.discarded_cards_this_turn, 4);
            assert_eq!(state.monsters[0].hp, 57);
            continue;
        }
        for frame in &current.continuations {
            if let Some(entries) = frame.fields.get("drawn").and_then(|v| v.as_array())
                && entries.len() > 1
            {
                saw_repeated = true;
                let mut forged = current.clone();
                let owner = forged
                    .continuations
                    .iter_mut()
                    .find(|f| {
                        f.fields
                            .get("drawn")
                            .and_then(|v| v.as_array())
                            .is_some_and(|a| a.len() > 1)
                    })
                    .unwrap();
                owner
                    .fields
                    .get_mut("drawn")
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .pop();
                let refused = HotBoundary::catalog_from_canonical(&forged)
                    .and_then(|c| HotBoundary::from_canonical(&forged, &c).map(|s| (c, s)));
                assert!(
                    refused.is_err()
                        || refused.is_ok_and(|(c, s)| engine::admit(&forged, &s, &c).is_err())
                );
            }
        }
        for choice in engine::legal_actions(&state, &catalog) {
            let next = engine::apply_action(&state, &catalog, &choice)
                .unwrap()
                .state;
            frontier.push(wire(&current, &next));
        }
    }
    assert!(saw_repeated);
    assert_eq!(terminals, 24);
}

#[test]
fn scrape_filters_current_object_cost_after_all_repeated_draws() {
    let mut doc = entry(0);
    doc.piles.get_mut("draw").unwrap()[0].id = "MOMENTUM_STRIKE".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    state.piles.get_mut(PileId::Draw).make_mut()[0].flags |=
        sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let result = play(&doc);
    assert_eq!(
        result.state.history.discarded_cards_this_turn, 0,
        "the first AutoPlay makes the original free for all returned aliases"
    );
    assert_eq!(result.state.monsters[0].hp, 49);
    wire(&doc, &result.state);

    let mut upgraded = entry(0);
    upgraded
        .player
        .insert("relics_entering".into(), json!(["RELIC.RAZOR_TOOTH"]));
    upgraded.player.insert("razor_tooth".into(), json!(true));
    let result = play(&upgraded);
    assert_eq!(
        result.state.monsters[0].hp, 60,
        "each later draw resolves the upgraded original"
    );
    assert_eq!(result.state.history.discarded_cards_this_turn, 4);
    wire(&upgraded, &result.state);
}

#[test]
fn live_dupe_scrape_moves_without_hellraiser_and_counts_a_removed_result_with_it() {
    for hellraiser in [false, true] {
        let mut doc = entry(0);
        if !hellraiser {
            for field in [
                "hellraiser",
                "after_side_turn_end_power_order",
                "next_after_side_turn_end_power_uid",
            ] {
                doc.player.remove(field);
            }
        }
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        state.piles.get_mut(PileId::Draw).make_mut()[0].flags |=
            sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | sts_sim::hot::CARD_FLAG_DUPE;
        let canonical = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        if hellraiser {
            // #3136: Hellraiser auto-plays the DUPE Strike (6) and removes it.
            // Scrape still selects that returned object: CardPileCmd.Add
            // moves nothing, but the discard is counted.
            let result = play(&canonical);
            assert_eq!(result.state.history.discarded_cards_this_turn, 1);
            assert_eq!(result.state.monsters[0].hp, 100 - 7 - 6);
            assert!(!result.events.iter().any(|event| matches!(
                event,
                Event::CardResolved {
                    uid: 1,
                    pile: PileId::Discard
                }
            )));
            let done = wire(&canonical, &result.state);
            assert!(
                done.piles
                    .values()
                    .flatten()
                    .all(|card| card.uid != Some(1))
            );
        } else {
            let result = play(&canonical);
            assert_eq!(result.state.history.discarded_cards_this_turn, 1);
            assert_eq!(result.state.monsters[0].hp, 93);
            let done = wire(&canonical, &result.state);
            assert_eq!(
                done.piles["discard"]
                    .iter()
                    .filter(|card| card.uid == Some(1))
                    .count(),
                1
            );
        }
    }
}
