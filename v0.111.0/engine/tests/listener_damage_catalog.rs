//! Catalog-bearing nested orb/power deaths and native owner membership.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
};

fn entry(serpent_first: bool, enemy_hp: i64, left: i64) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".into(), json!(enemy_hp));
    doc.monsters[0].insert("max_hp".into(), json!(enemy_hp));
    doc.player.insert("serpent_form".into(), json!(3));
    doc.player.insert("panache".into(), json!(10));
    doc.player.insert("panache_left".into(), json!(left));
    let panache_uid = if serpent_first { 1 } else { 0 };
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
        if serpent_first {
            json!([["serpent_form", 0], ["panache", 1]])
        } else {
            json!([["panache", 0], ["serpent_form", 1]])
        },
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(2));
    doc.piles.get_mut("hand").unwrap()[0].id = "DEFEND_IRONCLAD".into();
    doc
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

fn nested_root(left: i64) -> CanonicalStateV2 {
    let mut doc = entry(true, 1, left);
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
    doc
}

#[test]
fn nested_serpent_horn_hellraiser_death_skips_later_panache_object() {
    let doc = nested_root(2);
    let next = play_and_reload(&doc, 0);
    assert_eq!(
        next.player.get("hp").and_then(|v| v.as_i64()).unwrap_or(0),
        0
    );
    assert_eq!(next.player["player_hooks_deactivated"], true);
    assert_eq!(
        next.monsters[1]["hp"], 94,
        "lethal Thorns cannot cancel the already computed Strike"
    );
    assert_eq!(
        next.player["panache_instances"][0][2], 2,
        "owner hooks deactivate inside Serpent Form's nested Horn/Strike before later Panache"
    );
}

#[test]
fn prevented_owner_death_keeps_later_power_callbacks_active() {
    let mut doc = nested_root(4);
    doc.player.insert(
        "relics_entering".into(),
        json!(["RELIC.GREMLIN_HORN", "RELIC.LIZARD_TAIL"]),
    );
    let next = play_and_reload(&doc, 0);
    assert_eq!(next.player["hp"], 40);
    assert_eq!(next.player["lizard_tail_used"], 1);
    assert!(!next.player.contains_key("player_hooks_deactivated"));
    assert_eq!(
        next.player["panache_instances"][0][2], 2,
        "nested Strike and outer Defend each dispatch Panache after prevented death"
    );
}

#[test]
fn channel_evoke_retains_catalog_through_lightning_and_dark_death_draws() {
    for orb in [json!(["LIGHTNING", null]), json!(["DARK", 8])] {
        let mut doc = nested_root(4);
        for key in [
            "serpent_form",
            "panache",
            "panache_left",
            "panache_instances",
            "after_card_played_power_order",
        ] {
            doc.player.remove(key);
        }
        doc.player.insert(
            "after_side_turn_end_power_order".into(),
            json!([["hellraiser", 2]]),
        );
        doc.player.insert("orb_slots".into(), json!(1));
        doc.player.insert("orbs".into(), json!([orb]));
        doc.piles.get_mut("hand").unwrap()[0].id = "ZAP".into();
        doc.player.insert("hp".into(), json!(50));
        let next = play_and_reload(&doc, 0);
        assert!(!next.player.contains_key("player_hooks_deactivated"));
        assert_eq!(
            next.monsters[1]["hp"], 94,
            "Horn drew and Hellraiser played the physical Strike before channel resumed"
        );
        assert_eq!(
            next.player["orbs"],
            json!([["LIGHTNING", null]]),
            "Channel enqueues its new orb after the awaited evoke"
        );
    }
}

#[test]
fn panache_batch_commits_peers_before_horn_and_keeps_reentrant_countdown() {
    let mut doc = nested_root(1);
    doc.player.remove("serpent_form");
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["panache", 1]]),
    );
    doc.player.insert("hp".into(), json!(50));
    doc.monsters[0].insert("hp".into(), json!(5));
    doc.monsters[0].insert("max_hp".into(), json!(5));
    doc.monsters[1].insert("hp".into(), json!(30));
    doc.monsters[1].insert("max_hp".into(), json!(30));
    doc.monsters[1].remove("thorns");
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
        &mut events,
    )
    .unwrap();
    let damages: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            engine::Event::MonsterDamaged {
                uid, unblocked, hp, ..
            } => Some((*uid, *unblocked, *hp)),
            _ => None,
        })
        .collect();
    assert_eq!(damages, [(0, 10, -5), (1, 10, 20), (1, 6, 14), (1, 10, 4)]);
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.monsters[1]["hp"], 4);
    assert_eq!(wire.player["panache_instances"][0][2], 5);
    assert_eq!(play_and_reload(&doc, 0), wire);
}

#[test]
fn glass_batch_damages_every_target_before_death_draw_autoplay() {
    let mut doc = nested_root(4);
    for key in [
        "serpent_form",
        "panache",
        "panache_left",
        "panache_instances",
        "after_card_played_power_order",
    ] {
        doc.player.remove(key);
    }
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["hellraiser", 2]]),
    );
    doc.player.insert("hp".into(), json!(50));
    doc.player.insert("orb_slots".into(), json!(1));
    doc.player.insert("orbs".into(), json!([["GLASS", 4]]));
    doc.piles.get_mut("hand").unwrap()[0].id = "ZAP".into();
    doc.monsters[1].remove("thorns");
    let next = play_and_reload(&doc, 0);
    assert_eq!(next.monsters[1]["hp"], 86);
    assert_eq!(next.player["orbs"], json!([["LIGHTNING", null]]));
}

#[test]
fn simultaneous_last_enemy_deaths_do_not_draw_or_gain_energy_from_horn() {
    let mut doc = nested_root(1);
    doc.player.remove("serpent_form");
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["panache", 1]]),
    );
    doc.player.insert("hp".into(), json!(50));
    doc.monsters[1].insert("hp".into(), json!(5));
    doc.monsters[1].remove("thorns");
    let next = play_and_reload(&doc, 0);
    assert_eq!(next.player["energy"], 2);
    assert_eq!(next.piles["draw"], doc.piles["draw"]);
    assert_eq!(next.player["panache_instances"][0][2], 5);
}

#[test]
fn horn_selecting_strike_and_recursive_stratagem_park_after_the_panache_play() {
    for id in ["SCULPTING_STRIKE", "POMMEL_STRIKE"] {
        let mut doc = nested_root(1);
        doc.player.remove("serpent_form");
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["panache", 1]]),
        );
        doc.player.insert("hp".into(), json!(50));
        doc.monsters[1].remove("thorns");
        doc.piles.get_mut("draw").unwrap()[0].id = id.into();
        if id == "POMMEL_STRIKE" {
            doc.player.insert("stratagem".into(), json!(1));
            let remaining = doc.piles.get_mut("draw").unwrap().split_off(1);
            doc.piles.get_mut("discard").unwrap().extend(remaining);
        }
        // #3387: the Horn's Draw runs on the resumable frame. The choice
        // its Hellraiser AutoPlay begins inside the synchronous Panache
        // damage is queued as a deferred hook action: the Sculpting Strike's
        // own selection, or Pommel's reshuffle selection above the
        // AutoPlayed Pommel. The play finishes first, and the choice then
        // parks on the Play root with the AutoPlayed card still in Play.
        // `play_and_reload` round-trips and admits the parked state.
        let wire = play_and_reload(&doc, 0);
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let parked = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        assert!(parked.pending.is_some(), "{id}");
        assert_eq!(parked.history.card_plays_finished_combat, 1, "{id}");
        assert_eq!(wire.piles["play"].len(), 1, "{id}");
        assert_eq!(wire.piles["play"][0].id, id);
    }
}

#[test]
fn cold_panache_zero_and_negative_counters_cannot_forge_an_inflight_callback() {
    for left in [0, -1, -2] {
        let mut doc = entry(false, 100, left);
        // A private synchronous receipt has no wire representation.
        doc.player.insert("panache_left".into(), json!(left));
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        assert!(HotBoundary::from_canonical(&doc, &catalog).is_err());
    }
}

#[test]
fn actual_death_clears_remaining_orbs_and_capacity_before_channel_resumes() {
    for prevented in [false, true] {
        let mut doc = nested_root(4);
        for key in [
            "serpent_form",
            "panache",
            "panache_left",
            "panache_instances",
            "after_card_played_power_order",
        ] {
            doc.player.remove(key);
        }
        doc.player.insert(
            "after_side_turn_end_power_order".into(),
            json!([["hellraiser", 2]]),
        );
        doc.player.insert("orb_slots".into(), json!(2));
        doc.player
            .insert("orbs".into(), json!([["LIGHTNING", null], ["FROST", null]]));
        doc.piles.get_mut("hand").unwrap()[0].id = "ZAP".into();
        if prevented {
            doc.player.insert(
                "relics_entering".into(),
                json!(["RELIC.GREMLIN_HORN", "RELIC.LIZARD_TAIL"]),
            );
        }
        let next = play_and_reload(&doc, 0);
        if prevented {
            assert_eq!(next.player["orb_slots"], 2);
            assert_eq!(
                next.player["orbs"],
                json!([["FROST", null], ["LIGHTNING", null]])
            );
        } else {
            assert_eq!(next.player["player_hooks_deactivated"], true);
            assert!(!next.player.contains_key("orb_slots"));
            assert!(!next.player.contains_key("orbs"));
        }
    }
}

#[test]
fn cold_hologram_selection_resumes_the_same_owner_membership_checks() {
    let mut doc = nested_root(2);
    doc.piles.get_mut("hand").unwrap()[0].id = "HOLOGRAM".into();
    for _ in 0..2 {
        let discard = doc.piles.get_mut("hand").unwrap().remove(1);
        doc.piles.get_mut("discard").unwrap().push(discard);
    }
    let pending = play_and_reload(&doc, 0);
    let catalog = HotBoundary::catalog_from_canonical(&pending).unwrap();
    let state = HotBoundary::from_canonical(&pending, &catalog).unwrap();
    assert!(state.pending.is_some());
    let actions = engine::legal_actions(&state, &catalog);
    assert_eq!(actions.len(), 2);
    let next = engine::apply_action_into(&state, &catalog, &actions[0], &mut Vec::new()).unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player["player_hooks_deactivated"], true);
    assert_eq!(wire.player["panache_instances"][0][2], 2);
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
}

#[test]
fn public_panache_queen_death_cascades_and_round_trips_for_all_lethal_subsets() {
    for (amalgam_hp, queen_hp) in [(5, 5), (100, 5), (5, 100)] {
        let mut doc = entry(false, 1, 1);
        doc.player.remove("serpent_form");
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["panache", 0]]),
        );
        doc.player.insert("exact_piles".into(), json!(true));
        doc.monsters = serde_json::from_value(json!([
            {"kind":"TORCH_HEAD_AMALGAM", "hp":amalgam_hp, "max_hp":211, "secondary":true},
            {"kind":"QUEEN", "hp":queen_hp, "max_hp":419, "slot":1, "uid":1}
        ]))
        .unwrap();
        let next = play_and_reload(&doc, 0);
        assert!(
            next.monsters[0]
                .get("hp")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                <= 0
        );
        let final_queen_hp = next.monsters[1]
            .get("hp")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        assert_eq!(final_queen_hp, queen_hp - 10);
        assert_eq!(next.player["panache_instances"][0][2], 5);
    }
}

#[test]
fn nested_horn_panache_retains_the_outer_amalgam_death_callback() {
    for queen_hp in [20, 30] {
        let mut doc = nested_root(1);
        doc.player.remove("serpent_form");
        doc.player.insert("hp".into(), json!(50));
        doc.player.insert(
            "after_card_played_power_order".into(),
            json!([["panache", 1]]),
        );
        doc.player.insert("exact_piles".into(), json!(true));
        doc.monsters = serde_json::from_value(json!([
            {"kind":"TORCH_HEAD_AMALGAM", "hp":5, "max_hp":211, "secondary":true},
            {"kind":"QUEEN", "hp":queen_hp, "max_hp":419, "slot":1, "uid":1}
        ]))
        .unwrap();
        let next = play_and_reload(&doc, 0);
        assert_eq!(next.monsters[1]["hp"], queen_hp - 26);
        assert_eq!(next.player["panache_instances"][0][2], 5);
    }
}

#[test]
fn pending_knockdown_slug_does_not_block_horn_autoplay_against_survivor() {
    let mut doc = nested_root(1);
    doc.player.remove("serpent_form");
    doc.player.insert("hp".into(), json!(50));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["panache", 1]]),
    );
    doc.monsters = serde_json::from_value(json!([
        {"kind":"CORPSE_SLUG", "hp":5, "max_hp":27, "ravenous":5},
        {"kind":"CORPSE_SLUG", "hp":5, "max_hp":28, "slot":1, "uid":1, "ravenous":5,
         "knockdown":[[2,"player"]], "misery_debuff_order":["knockdown"]},
        {"kind":"CORPSE_SLUG", "hp":29, "max_hp":29, "slot":2, "uid":2, "ravenous":5}
    ]))
    .unwrap();
    doc.piles.get_mut("draw").unwrap()[1].id = "DEFEND_IRONCLAD".into();
    let next = play_and_reload(&doc, 0);
    assert_eq!(next.monsters[2]["hp"], 3);
    assert_eq!(next.monsters[2]["strength"], 10);
    assert!(!next.monsters[1].contains_key("knockdown"));
    assert_eq!(next.player["panache_instances"][0][2], 5);
}
