//! #3639: a powered multi-receiver hit does not retaliate Thorns against a
//! player the same hit already killed.
//!
//! The retaliation is its own `CreatureCmd.Damage` at the incoming dealer
//! (`ThornsPower/<BeforeDamageReceived>d__4::MoveNext` RVA `0x349954`
//! IL_0065-IL_0086; v0.111.0 `sts2.dll`, SHA-256 `9cb4f1ad…fbf12b4`), and
//! `CreatureCmd/<Damage>d__12::MoveNext` RVA `0x3e96c8` skips a dead target
//! at its receiver entry (IL_0167-IL_0172). The powered batch itself tests
//! its dealer once, at entry (IL_0079-IL_00af), so both receivers still
//! commit.
//!
//! The fixture is the KD13JGCDPB3U Toadpoles entry with the second Toadpole
//! put on the first one's move, so both buff Thorns 2 on turn one, the player
//! at 2 HP, and a Thunderclap on top of the draw pile. Turn two plays it.
//!
//! The live engine leaves the same terminal state (headless harness, build
//! v0.111.0 `41cef1ea`, Ironclad at 5 HP against `TOADPOLES_WEAK` with Thorns
//! 5 on both, Thunderclap): one `DamageReceived` on the player, the player at
//! 0, both Toadpoles down 4.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event},
    exact_solve_v1::ExactSolveActionV1,
    solo_v1::admit_exact_solve,
};

#[test]
fn thunderclap_into_two_thorns_toadpoles_costs_a_dying_player_one_retaliation() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["draw"][0]["id"] = "THUNDERCLAP".into();
    value["monsters"][1]["loop_pos"] = 2.into();
    value["player"]["hp"] = 2.into();
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();

    let line = r#"[{"kind":"end"},{"kind":"play","uid":5}]"#;
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut events = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
    }

    let rows: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::PlayerDamaged { .. }
                    | Event::MonsterDamaged { .. }
                    | Event::CombatOver { .. }
            )
        })
        .collect();
    assert!(
        matches!(
            rows.as_slice(),
            [
                // Toadpole 0's Thorns kills the player before its own commit.
                Event::PlayerDamaged {
                    hp_lost: 2,
                    hp: 0,
                    ..
                },
                Event::CombatOver { player_won: false },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 4,
                    hp: 22,
                    ..
                },
                // Toadpole 1's Thorns finds nobody; its commit still lands.
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 4,
                    hp: 21,
                    ..
                },
            ]
        ),
        "{rows:?}"
    );
    assert_eq!(state.hp, 0);
    assert!(state.history.over);
}
