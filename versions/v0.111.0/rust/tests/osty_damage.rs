//! One native WHIRL hit through the public action and canonical reload boundary.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action},
};
#[test]
fn pet_spill_retaliation_and_delayed_death_survive_a_public_turn_and_cold_reload() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.monsters[0].insert("loop_pos".into(), json!(1)); // WHIRL =8x1.
    doc.player.insert("ally".into(), json!(["OSTY", 3, 3]));
    doc.player.insert("block".into(), json!(2));
    doc.player.insert("necro_mastery".into(), json!(1));
    doc.player.insert("flame_barrier".into(), json!(4));
    doc.player.insert("reflect".into(), json!(1));
    doc.player.insert(
        "after_damage_received_power_order".into(),
        json!(["reflect", "flame_barrier"]),
    );
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 65); //68 - (8 -2 Block -3 Osty).
    assert_eq!(next.monsters[0].hp, 17); //26 -3 Necro -2 Reflect -4 Flame.
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    engine::admit(&wire, &cold, &cold_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
    assert_eq!(
        cold.card_states.get(0).local_cost_modifiers.resolve(3),
        2,
        "one actual pet death"
    );
}

#[test]
fn arbitrary_melancholy_death_discount_does_not_gain_admission() {
    use sts_sim::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, LocalCostExpiration, LocalCostModifier,
        LocalCostModifierKind, PileId,
    };
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    state.piles.get_mut(PileId::Hand).make_mut()[0].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    state.card_states.append_local_cost_modifier(
        0,
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: -2,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    );
    let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert!(engine::admit(&wire, &state, &catalog).is_err());
}

#[test]
fn paper_cuts_nested_player_death_does_not_repeat_gambit_kill() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters = vec![
        serde_json::from_value(json!({
            "kind": "SCROLL_OF_BITING", "hp": 39, "max_hp": 39
        }))
        .unwrap(),
    ];
    for (key, value) in [
        ("hp", json!(2)),
        ("max_hp", json!(2)),
        ("block", json!(14)),
        ("ally", json!(["OSTY", 1, 1])),
        ("the_gambit", json!(1)),
    ] {
        doc.player.insert(key.into(), value);
    }
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert!(next.history.over);
    assert_eq!(
        next.card_states.get(0).local_cost_modifiers.resolve(3),
        2,
        "one nested player death; owner hooks deactivate before delayed Osty death"
    );
    assert_eq!(
        next.card_states
            .get(0)
            .local_cost_modifiers
            .as_slice()
            .len(),
        1
    );
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player.get("osty_corpse"), Some(&json!(true)));
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
}

#[test]
fn pending_lethal_player_keeps_hooks_during_necro_victory_and_pet_death() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".into(), json!(1));
    doc.monsters[0].insert("loop_pos".into(), json!(1));
    doc.player.insert("hp".into(), json!(2));
    doc.player.insert("ally".into(), json!(["OSTY", 1, 1]));
    doc.player.insert("necro_mastery".into(), json!(1));
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert_eq!(next.monsters[0].hp, 0);
    assert_eq!(
        next.card_states
            .get(0)
            .local_cost_modifiers
            .as_slice()
            .len(),
        3,
        "enemy death while player Kill pending, queued pet death, then actual player death"
    );
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );
}

#[test]
fn gambit_actual_death_deactivates_owner_before_queued_pet_death() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.monsters[0].insert("loop_pos".into(), json!(1));
    doc.player.insert("ally".into(), json!(["OSTY", 1, 1]));
    doc.player.insert("the_gambit".into(), json!(1));
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert_eq!(
        next.card_states
            .get(0)
            .local_cost_modifiers
            .as_slice()
            .len(),
        1
    );
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player.get("osty_corpse"), Some(&json!(true)));
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );
}

#[test]
fn live_player_cannot_import_deactivated_hooks() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player
        .insert("player_hooks_deactivated".into(), json!(true));
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    assert!(
        engine::admit(&doc, &state, &catalog)
            .unwrap_err()
            .to_string()
            .contains("actual player death hook state")
    );
}

#[test]
fn actual_player_death_kills_live_osty_before_deactivating_owner_hooks() {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.monsters.truncate(1);
    doc.player.insert("hp".into(), json!(2));
    doc.player.insert("player_doom".into(), json!(2));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["doom", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player.insert("ally".into(), json!(["OSTY", 3, 3]));
    doc.player.insert("necro_mastery".into(), json!(1));
    doc.piles.get_mut("hand").unwrap()[0].id = "MELANCHOLY".into();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert_eq!(
        next.monsters[0].hp, 26,
        "ordinary Necro is removed before live-pet Kill"
    );
    assert_eq!(
        next.card_states
            .get(0)
            .local_cost_modifiers
            .as_slice()
            .len(),
        2,
        "player and still-live pet actual deaths both precede hook deactivation"
    );
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player.get("osty_corpse"), Some(&json!(true)));
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );
}

fn toadpole_doc() -> CanonicalStateV2 {
    serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap()
}

/// A lone Toadpole plus `extra`, with Doom set to kill the player at end of
/// turn (the same lethal as
/// `actual_player_death_kills_live_osty_before_deactivating_owner_hooks`).
fn doomed_beside(extra: &str, extra_hp: i32) -> CanonicalStateV2 {
    let mut doc = toadpole_doc();
    doc.monsters.truncate(1);
    let mut monster = json!({
        "kind": extra, "hp": extra_hp, "max_hp": extra_hp.max(20), "slot": 1, "uid": 1
    });
    if extra_hp == 0 {
        // A retained dead Illusion holder parks in its revive stage.
        monster["revive_stage"] = json!(1);
    }
    doc.monsters.push(serde_json::from_value(monster).unwrap());
    doc.player.insert("hp".into(), json!(2));
    doc.player.insert("player_doom".into(), json!(2));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["doom", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc
}

/// Load `doc` and require that it is admitted, so every runtime witness
/// below starts from a root the engine accepts.
fn load_admitted(doc: &CanonicalStateV2) -> (sts_sim::hot::HotState, sts_sim::catalog::Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn cleanup_refusal(doc: &CanonicalStateV2) -> bool {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog)
        .err()
        .is_some_and(|refusal| {
            refusal
                .to_string()
                .contains("Illusion retained Necro owner-death cleanup")
        })
}

#[test]
fn illusion_necro_compositions_have_no_cold_admission_wall() {
    // #2646: the interval is refused at runtime, at the one state that
    // reaches it. Live, retained-dead and future holders, live and future
    // Necro, and every Osty summon writer no longer refuse at admission.
    for kind in ["PARAFRIGHT", "EYE_WITH_TEETH", "THE_OBSCURA", "FOGMOG"] {
        for hp in [0, 20] {
            for future_necro in [false, true] {
                let mut doc = toadpole_doc();
                doc.monsters.push(
                    serde_json::from_value(json!({
                        "kind": kind, "hp": hp, "max_hp": 20
                    }))
                    .unwrap(),
                );
                doc.player.insert("ally".into(), json!(["OSTY", 3, 3]));
                if future_necro {
                    doc.piles.get_mut("hand").unwrap()[0].id = "NECRO_MASTERY".into();
                } else {
                    doc.player.insert("necro_mastery".into(), json!(1));
                }
                assert!(
                    !cleanup_refusal(&doc),
                    "{kind} hp={hp} future_necro={future_necro}"
                );
            }
        }
    }
    for writer in [
        "BOUND_PHYLACTERY",
        "PHYLACTERY_UNBOUND",
        "summon_next_turn",
        "devour_life",
        "AFTERLIFE",
        "INVOKE",
    ] {
        let mut doc = toadpole_doc();
        doc.monsters.push(
            serde_json::from_value(json!({
                "kind": "PARAFRIGHT", "hp": 0, "max_hp": 20
            }))
            .unwrap(),
        );
        doc.player.insert("necro_mastery".into(), json!(1));
        doc.player.insert("osty_corpse".into(), json!(true));
        match writer {
            "BOUND_PHYLACTERY" | "PHYLACTERY_UNBOUND" => {
                doc.player.insert(writer.to_ascii_lowercase(), json!(true));
                doc.player
                    .insert("relics_entering".into(), json!([format!("RELIC.{writer}")]));
            }
            "AFTERLIFE" | "INVOKE" => doc.piles.get_mut("hand").unwrap()[0].id = writer.into(),
            "devour_life" => {
                doc.player.insert(writer.into(), json!(1));
                doc.player.insert(
                    "after_card_played_power_order".into(),
                    json!([["devour_life", 0]]),
                );
                doc.player
                    .insert("next_after_side_turn_end_power_uid".into(), json!(1));
            }
            _ => {
                doc.player.insert(writer.into(), json!(1));
            }
        }
        assert!(!cleanup_refusal(&doc), "{writer}");
    }
}

#[test]
fn illusion_holder_live_pet_necro_death_refuses_at_runtime_without_mutation() {
    // Alive and retained-dead holders both veto (#2646).
    for kind in ["PARAFRIGHT", "EYE_WITH_TEETH"] {
        for hp in [0, 20] {
            let mut doc = doomed_beside(kind, hp);
            doc.player.insert("ally".into(), json!(["OSTY", 3, 3]));
            doc.player.insert("necro_mastery".into(), json!(1));
            let (state, catalog) = load_admitted(&doc);
            let before = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            let error =
                engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new())
                    .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Illusion retained Necro owner-death cleanup"),
                "{kind} hp={hp}: {error}"
            );
            assert_eq!(
                HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
                before
            );
        }
    }
}

#[test]
fn future_necro_and_osty_reach_the_runtime_wall_through_play() {
    // Necro Mastery summons Osty and applies the power; admission no longer
    // refuses the root, so the later death is what refuses (#2646).
    let mut doc = doomed_beside("PARAFRIGHT", 21);
    doc.piles.get_mut("hand").unwrap()[0].id = "NECRO_MASTERY".into();
    let (state, catalog) = load_admitted(&doc);
    let played = engine::apply_action_into(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: None,
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let wire = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    assert!(wire.player.contains_key("ally"));
    assert_eq!(wire.player.get("necro_mastery"), Some(&json!(1)));
    let error = engine::apply_action_into(&played, &catalog, &Action::EndTurn, &mut Vec::new())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Illusion retained Necro owner-death cleanup")
    );
}

#[test]
fn a_summoner_without_its_illusion_takes_the_ordinary_necro_removal() {
    // The Obscura and Fogmog hold no IllusionPower (#2646), so Necro is
    // removed at IL_04f1 before the live Osty's Kill and the Toadpole keeps
    // its HP, exactly as with no summoner at all.
    for kind in ["THE_OBSCURA", "FOGMOG"] {
        let mut doc = doomed_beside(kind, 100);
        if kind == "THE_OBSCURA" {
            // The opening intent, as `sts-sim entry --opening` emits it.
            doc.monsters[1].insert("move_log".into(), json!(["ILLUSION_MOVE"]));
            doc.monsters[1].insert("next_move".into(), json!("ILLUSION_MOVE"));
        }
        doc.player.insert("ally".into(), json!(["OSTY", 3, 3]));
        doc.player.insert("necro_mastery".into(), json!(1));
        let (state, catalog) = load_admitted(&doc);
        let next =
            engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
        assert_eq!(next.hp, 0, "{kind}");
        assert_eq!(next.monsters[0].hp, 26, "{kind}");
        let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
        assert_eq!(wire.player.get("osty_corpse"), Some(&json!(true)), "{kind}");
        assert_eq!(wire.player.get("necro_mastery"), None, "{kind}");
        assert_eq!(
            wire.player.get("player_hooks_deactivated"),
            Some(&json!(true)),
            "{kind}"
        );
    }
}

#[test]
fn an_illusion_death_missing_one_interval_condition_stays_modeled() {
    // Live Osty, no Necro: the pet's Kill has no Necro listener to run.
    let mut no_necro = doomed_beside("PARAFRIGHT", 0);
    no_necro.player.insert("ally".into(), json!(["OSTY", 3, 3]));
    let (state, catalog) = load_admitted(&no_necro);
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert_eq!(next.monsters[0].hp, 26);
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player.get("osty_corpse"), Some(&json!(true)));
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );

    // Necro, no Osty: the veto keeps Necro, and nothing ever triggers it
    // before Player.DeactivateHooks.
    let mut no_pet = doomed_beside("PARAFRIGHT", 0);
    no_pet.player.insert("necro_mastery".into(), json!(1));
    let (state, catalog) = load_admitted(&no_pet);
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert_eq!(next.hp, 0);
    assert_eq!(next.monsters[0].hp, 26);
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert_eq!(wire.player.get("necro_mastery"), Some(&json!(1)));
    assert_eq!(
        wire.player.get("player_hooks_deactivated"),
        Some(&json!(true))
    );

    // A prevented death returns before RemoveAllPowersAfterDeath, so the
    // live pet is never killed and Necro stays active.
    let mut prevented = doomed_beside("PARAFRIGHT", 0);
    prevented
        .player
        .insert("ally".into(), json!(["OSTY", 3, 3]));
    prevented.player.insert("necro_mastery".into(), json!(1));
    prevented
        .player
        .insert("potions".into(), json!(["FAIRY_IN_A_BOTTLE"]));
    prevented
        .player
        .insert("potion_slots".into(), json!(["FAIRY_IN_A_BOTTLE"]));
    prevented
        .player
        .insert("fully_unlocked_potion_pool".into(), json!(true));
    let (state, catalog) = load_admitted(&prevented);
    let next =
        engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new()).unwrap();
    assert!(next.hp > 0);
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    assert!(wire.player.contains_key("ally"));
    assert_eq!(wire.player.get("necro_mastery"), Some(&json!(1)));
    assert_eq!(wire.player.get("player_hooks_deactivated"), None);
}
