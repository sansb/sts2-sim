//! #3632: Rupture answers its owner's HP loss only on the owner's own side.
//!
//! `RupturePower/<AfterDamageReceived>d__9::MoveNext` RVA `0x342fec`
//! (v0.111.0 `sts2.dll`, SHA-256 `9cb4f1ad…fbf12b4`) leaves at IL_005e unless
//! `CombatState.CurrentSide == Owner.Side` (IL_0046-IL_005c). Both of its
//! arms sit after that test.
//!
//! The enemy side reaches the listener through the path #3622 found for
//! Inferno (`tests/inferno_side_gate.rs`). A monster's hit arms Centennial
//! Puzzle, whose Draw runs inside that attack; Hellraiser AutoPlays a drawn
//! Strike (`HellraiserPower/<AfterCardDrawnEarly>d__7::MoveNext` RVA
//! `0x33c1a8`, no side test); the Strike enters a Thorns holder and the
//! retaliation costs the owner HP while `CurrentSide` is Enemy.
//!
//! The fixture is the KD13JGCDPB3U Toadpoles entry with Rupture and
//! Hellraiser put in the opening hand, Centennial Puzzle owned and armed, and
//! the Injury on top of the draw pile replaced by a Strike. Ending turn one
//! then runs, in order: Toadpole 0 buffs Thorns, Toadpole 1 attacks, the
//! Puzzle draws three Strikes on the enemy side, and the turn-two hand draw
//! plays a fourth Strike on the player side.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, Subject},
    exact_solve_v1::ExactSolveActionV1,
    ids::PowerId,
    solo_v1::admit_exact_solve,
};

#[test]
fn a_puzzle_drawn_strike_into_thorns_on_the_enemy_side_grants_no_rupture_strength() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["hand"][0]["id"] = "RUPTURE".into();
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

    // The Thorns results on the player and every Strength grant, in order.
    let trace: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::PlayerDamaged { .. }
                    | Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::Strength,
                        ..
                    }
            )
        })
        .collect();
    assert!(
        matches!(
            trace.as_slice(),
            [
                // Enemy side: Toadpole 1's hit, then the two Puzzle-drawn
                // Strikes that enter Toadpole 0 (Thorns 2). No Strength.
                Event::PlayerDamaged { hp_lost: 8, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                // Player side: the turn-two hand draw plays one more Strike
                // into Toadpole 0, and Rupture answers its retaliation.
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PowerChanged { amount: 1, .. },
            ]
        ),
        "{trace:?}"
    );
    assert_eq!(state.powers.value(PowerId::Strength), 1);
    assert_eq!(state.hp, 50);
    assert!(state.player_side_active);
}
