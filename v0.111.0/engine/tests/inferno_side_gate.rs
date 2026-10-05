//! #3622: Inferno answers its owner's HP loss only on the owner's own side.
//!
//! `InfernoPower/<AfterDamageReceived>d__8::MoveNext` RVA `0x33d084`
//! (v0.111.0 `sts2.dll`, SHA-256 `9cb4f1ad…fbf12b4`) leaves at IL_0063 unless
//! `Owner.CombatState.CurrentSide == Owner.Side` (IL_0046-IL_0061).
//!
//! The enemy side reaches the listener through one admitted path. A monster's
//! hit arms Centennial Puzzle, whose Draw runs inside that attack; Hellraiser
//! AutoPlays a drawn Strike (`HellraiserPower/<AfterCardDrawnEarly>d__7::
//! MoveNext` RVA `0x33c1a8`, no side test); the Strike enters a Thorns holder
//! and the retaliation costs the owner HP while `CurrentSide` is Enemy.
//!
//! The fixture is the KD13JGCDPB3U Toadpoles entry with Inferno and
//! Hellraiser put in the opening hand, Centennial Puzzle owned and armed, and
//! the Injury on top of the draw pile replaced by a Strike. Ending turn one
//! then runs, in order: Toadpole 0 buffs Thorns, Toadpole 1 attacks, the
//! Puzzle draws three Strikes on the enemy side, and the turn-two hand draw
//! plays a fourth Strike on the player side.
//!
//! The live engine logs the same two shapes (headless harness, build
//! v0.111.0 `41cef1ea`, Ironclad against `TOADPOLES_WEAK` with Inferno 6,
//! Hellraiser, Centennial Puzzle and Thorns 2 on both Toadpoles): a Strike
//! drawn during the Toadpole's attack logs the Thorns result and its own
//! result only, and a Strike of the next hand draw logs the Thorns result,
//! Inferno's 6 on each Toadpole, then its own result.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event},
    exact_solve_v1::ExactSolveActionV1,
    solo_v1::admit_exact_solve,
};

#[test]
fn a_puzzle_drawn_strike_into_thorns_on_the_enemy_side_fires_no_inferno() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["hand"][0]["id"] = "INFERNO".into();
    value["piles"]["hand"][3]["id"] = "HELLRAISER".into();
    value["piles"]["draw"][0]["id"] = "STRIKE_IRONCLAD".into();
    value["player"]["relics_entering"]
        .as_array_mut()
        .unwrap()
        .push("RELIC.CENTENNIAL_PUZZLE".into());
    value["player"]["puzzle"] = true.into();
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();

    let line = r#"[{"kind":"play","uid":0},{"kind":"play","uid":3},{"kind":"end"}]"#;
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut events = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
    }

    // Enemy side: from the Toadpole's hit to the last Puzzle-drawn Strike.
    let attack = events
        .iter()
        .position(|event| matches!(event, Event::PlayerDamaged { hp_lost: 8, .. }))
        .unwrap();
    let hand_draw = events
        .iter()
        .position(|event| matches!(event, Event::CardDrawn { uid: 8 }))
        .unwrap();
    let enemy_side: Vec<_> = events[attack..hand_draw]
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::PlayerDamaged { .. } | Event::MonsterDamaged { .. }
            )
        })
        .collect();
    assert!(
        matches!(
            enemy_side.as_slice(),
            [
                Event::PlayerDamaged { hp_lost: 8, .. },
                // Strike into Toadpole 0 (Thorns 2): the retaliation, then
                // the hit. No Inferno.
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 6,
                    hp: 20,
                    ..
                },
                // Strike into Toadpole 1 (no Thorns).
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 6,
                    hp: 19,
                    ..
                },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 6,
                    hp: 14,
                    ..
                },
            ]
        ),
        "{enemy_side:?}"
    );

    // Player side: the turn-two hand draw plays one more Strike into
    // Toadpole 0. Its retaliation fans Inferno's 6 over both Toadpoles before
    // the hit lands.
    let player_side: Vec<_> = events[hand_draw..]
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::PlayerDamaged { .. } | Event::MonsterDamaged { .. }
            )
        })
        .take(4)
        .collect();
    assert!(
        matches!(
            player_side.as_slice(),
            [
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 6,
                    hp: 8,
                    ..
                },
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 6,
                    hp: 13,
                    ..
                },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 6,
                    hp: 2,
                    ..
                },
            ]
        ),
        "{player_side:?}"
    );
    assert!(state.player_side_active);
}
