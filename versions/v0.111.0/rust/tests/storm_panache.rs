//! Native ordered Storm/Panache callbacks, including fight-ending prefixes.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
};

fn entry(storm_first: bool, enemy_hp: i64, left: i64) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".into(), json!(enemy_hp));
    doc.monsters[0].insert("max_hp".into(), json!(enemy_hp));
    doc.player.insert("storm".into(), json!(1));
    doc.player.insert("panache".into(), json!(10));
    doc.player.insert("panache_left".into(), json!(left));
    let panache_uid = if storm_first { 1 } else { 0 };
    doc.player.insert(
        "panache_instances".into(),
        json!([[panache_uid, 10, left, true]]),
    );
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["panache", panache_uid]]),
    );
    doc.player.insert(
        "after_card_played_power_order".into(),
        if storm_first {
            json!([["storm", 0], ["panache", 1]])
        } else {
            json!([["panache", 0], ["storm", 1]])
        },
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(2));
    doc.player.insert("orb_slots".into(), json!(1));
    doc.player
        .insert("orbs".into(), json!([["LIGHTNING", null]]));
    doc.piles.get_mut("hand").unwrap()[0].id = "INFLAME".into();
    doc
}

#[test]
fn both_orders_run_once_and_keep_panache_counter_after_terminal_prefix() {
    for storm_first in [false, true] {
        for (enemy_hp, left) in [(100, 1), (5, 1), (5, 2)] {
            let doc = entry(storm_first, enemy_hp, left);
            let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
            engine::admit(&doc, &state, &catalog).unwrap();
            let next = engine::apply_action_into(
                &state,
                &catalog,
                &Action::Play {
                    uid: 0,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                &mut Vec::new(),
            )
            .unwrap();
            let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
            let expected_hp = if enemy_hp == 100 {
                82
            } else if storm_first || left == 2 {
                -3
            } else {
                -5
            };
            assert_eq!(
                next.monsters[0].hp, expected_hp,
                "order={storm_first}, left={left}"
            );
            assert_eq!(
                wire.player["panache_instances"][0][2],
                json!(if left == 1 { 5 } else { 1 })
            );
            let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
            if !next.history.over {
                engine::admit(&wire, &cold, &cold_catalog).unwrap();
            }
            assert_eq!(
                HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
                wire
            );
        }
    }
}

#[test]
fn missing_or_forged_shared_order_still_refuses() {
    for malformed in [
        json!([]),
        json!([["storm", 0], ["panache", 0]]),
        json!([["panache", 1], ["storm", 0]]),
    ] {
        let mut doc = entry(true, 100, 1);
        doc.player
            .insert("after_card_played_power_order".into(), malformed);
        match HotBoundary::catalog_from_canonical(&doc).and_then(|catalog| {
            HotBoundary::from_canonical(&doc, &catalog).map(|state| (catalog, state))
        }) {
            Err(_) => {}
            Ok((catalog, state)) => assert!(engine::admit(&doc, &state, &catalog).is_err()),
        }
    }
}

fn play_and_reload(doc: &CanonicalStateV2, uid: u32) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let next = engine::apply_action_into(
        &state,
        &catalog,
        &Action::Play {
            uid,
            target: None,
            selection: SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    if !next.history.over {
        engine::admit(&wire, &cold, &cold_catalog).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
    wire
}

#[test]
fn native_source_plays_create_both_orders_and_do_not_trigger_new_storm() {
    for storm_first in [false, true] {
        let mut doc = entry(storm_first, 100, 1);
        for key in [
            "storm",
            "panache",
            "panache_left",
            "panache_instances",
            "after_side_turn_end_power_order",
            "after_card_played_power_order",
            "next_after_side_turn_end_power_uid",
        ] {
            doc.player.remove(key);
        }
        let hand = doc.piles.get_mut("hand").unwrap();
        hand[0].id = if storm_first { "STORM" } else { "PANACHE" }.into();
        hand[1].id = if storm_first { "PANACHE" } else { "STORM" }.into();
        hand[2].id = "INFLAME".into();
        let first = play_and_reload(&doc, 0);
        assert_eq!(first.monsters[0]["hp"], 100);
        let second = play_and_reload(&first, 1);
        assert_eq!(second.monsters[0]["hp"], if storm_first { 92 } else { 100 });
        assert_eq!(
            second.player["after_card_played_power_order"],
            if storm_first {
                json!([["storm", 0], ["panache", 1]])
            } else {
                json!([["panache", 0], ["storm", 1]])
            }
        );
        assert_eq!(
            second.player["panache_instances"][0][2],
            if storm_first { 5 } else { 4 }
        );
        let third = play_and_reload(&second, 2);
        assert_eq!(third.monsters[0]["hp"], if storm_first { 84 } else { 92 });
    }
}

#[test]
fn repeated_panache_objects_straddle_storm_without_collapsing_counters() {
    for enemy_hp in [100, 5] {
        let mut doc = entry(false, enemy_hp, 1);
        doc.player.insert("panache".into(), json!(24));
        doc.player.remove("panache_left");
        doc.player.insert(
            "panache_instances".into(),
            json!([[0, 10, 1, true], [2, 14, 2, true]]),
        );
        doc.player.insert(
            "after_side_turn_end_power_order".into(),
            json!([["panache", 0], ["panache", 2]]),
        );
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["panache", 0], ["storm", 1], ["panache", 2]]),
        );
        doc.player
            .insert("next_after_side_turn_end_power_uid".into(), json!(3));
        let next = play_and_reload(&doc, 0);
        assert_eq!(
            next.monsters[0]["hp"],
            if enemy_hp == 100 { 82 } else { -5 }
        );
        assert_eq!(
            next.player["panache_instances"],
            json!([[0, 10, 5, true], [2, 14, 1, true]])
        );
    }
}

#[test]
fn replayed_power_advances_each_callback_once_per_body_in_both_orders() {
    for storm_first in [false, true] {
        let doc = entry(storm_first, 100, 2);
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let mut physical = state.card_states.get(0);
        physical.set_base_replay_count(Some(1)).unwrap();
        state.card_states.set(0, physical);
        state.piles.get_mut(sts_sim::hot::PileId::Hand).make_mut()[0].flags |=
            sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let replay = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let next = play_and_reload(&replay, 0);
        assert_eq!(next.monsters[0]["hp"], 74);
        assert_eq!(next.player["panache_instances"][0][2], 5);
    }
}

#[test]
fn autoplayed_power_child_preserves_pair_order_and_parent_panache_callback() {
    for storm_first in [false, true] {
        for enemy_hp in [100, 5] {
            let mut doc = entry(storm_first, enemy_hp, 1);
            doc.piles.get_mut("hand").unwrap()[0].id = "HAVOC".into();
            doc.piles.get_mut("draw").unwrap()[0].id = "INFLAME".into();
            let next = play_and_reload(&doc, 0);
            assert_eq!(
                next.monsters[0]["hp"],
                if enemy_hp == 100 {
                    82
                } else if storm_first {
                    -3
                } else {
                    -5
                }
            );
            assert_eq!(next.player["panache_instances"][0][2], 4);
        }
    }
}

#[test]
fn nested_horn_hellraiser_owner_death_skips_later_panache_object() {
    let mut doc = entry(true, 5, 2);
    doc.player.insert("hp".into(), json!(1));
    doc.player.insert("hellraiser".into(), json!(1));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["panache", 1], ["hellraiser", 2]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(3));
    doc.player.insert("gremlin_horn".into(), json!(true));
    doc.player
        .insert("relics_entering".into(), json!(["RELIC.GREMLIN_HORN"]));
    doc.monsters.push(
        serde_json::from_value(json!({
            "kind":"TOADPOLE", "hp":100, "max_hp":100, "slot":1, "uid":1, "thorns":10
        }))
        .unwrap(),
    );
    doc.rng.insert(
        "targets".into(),
        serde_json::from_value(json!({
            "words":[1,2,3,4], "counter":0
        }))
        .unwrap(),
    );
    let next = play_and_reload(&doc, 0);
    assert_eq!(
        next.player.get("hp").and_then(|v| v.as_i64()).unwrap_or(0),
        0
    );
    assert_eq!(next.player["player_hooks_deactivated"], true);
    assert!(!next.player.contains_key("orbs"));
    assert!(!next.player.contains_key("orb_slots"));
    assert_eq!(
        next.monsters[1]["hp"], 92,
        "Inflame-enhanced Strike still commits after lethal Thorns"
    );
    assert_eq!(
        next.player["panache_instances"][0][2], 2,
        "owner hooks deactivate inside Storm's nested Horn/Strike before later Panache"
    );
}
