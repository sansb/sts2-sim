//! #3612: Omnislice's spill draws Thorns retaliation from a bystander.
//!
//! `ThornsPower/<BeforeDamageReceived>d__4::MoveNext` RVA `0x349954`
//! (v0.111.0 `sts2.dll`, SHA-256 `9cb4f1ad…fbf12b4`) retaliates for a powered
//! attack OR an Omnislice card source (IL_0040-IL_0058), and Omnislice's
//! spill (`Omnislice/<OnPlay>d__3::MoveNext` RVA `0x3aff94` IL_01ea-IL_0218)
//! is an unpowered `Damage` whose card source is the Omnislice.
//!
//! The fixture is the KD13JGCDPB3U Toadpoles entry with an Omnislice on top of
//! the draw pile. Ending turn one, Toadpole 0 buffs Thorns 2 and Toadpole 1
//! attacks; on turn two Omnislice is played into Toadpole 1, the one without
//! Thorns. The live engine logs the same three rows for that shape (headless
//! harness, build v0.111.0 `41cef1ea`, `TOADPOLES_WEAK`, Thorns 2 on the
//! first Toadpole, Omnislice on the second): the target's 8, the player's 2,
//! the bystander's 8.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event},
    exact_solve_v1::ExactSolveActionV1,
    solo_v1::admit_exact_solve,
};

#[test]
fn omnislice_played_beside_a_thorns_toadpole_costs_the_player_its_thorns() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["draw"][0]["id"] = "OMNISLICE".into();
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();

    let line = r#"[{"kind":"end"},{"kind":"play","target":1,"uid":5}]"#;
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut events = Vec::new();
    let mut hp_before = state.hp;
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        events.clear();
        hp_before = state.hp;
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
    }

    let rows: Vec<_> = events
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
            rows.as_slice(),
            [
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 8,
                    ..
                },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 8,
                    ..
                },
            ]
        ),
        "{rows:?}"
    );
    assert_eq!(state.hp, hp_before - 2);

    // The result is ordinary state: it survives the canonical projection and
    // a cold reload.
    let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    assert_eq!(cold.hp, state.hp);
    assert_eq!(
        cold.monsters
            .iter()
            .map(|monster| monster.hp)
            .collect::<Vec<_>>(),
        state
            .monsters
            .iter()
            .map(|monster| monster.hp)
            .collect::<Vec<_>>()
    );
}
