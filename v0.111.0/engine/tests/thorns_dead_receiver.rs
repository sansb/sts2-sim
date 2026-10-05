//! #2655: a powered hit whose receiver died to a nested kill during Thorns
//! retaliation. The first test is the kill that ended the combat; the second
//! is the same fight with the other Toadpole still alive (`BATCH_LINE`).
//!
//! The first two lines are the stored `first_refusal.actions` of the KD13JGCDPB3U
//! fight 0 (Toadpoles, floor 2) review searches that used to stop at
//! `PowerOrderNotModeled("Thorns retained dead or replaced receiver")`
//! (`uct 7` and `random 16`, see `search_refused_branch.rs` history). Each
//! ends with Pact's End into the last live Toadpole at 1 HP with Thorns 2:
//! the retaliation costs the player 2 HP, the player's Inferno 6 kills the
//! Toadpole and wins the fight, and native `CreatureCmd.Damage` `0x3e96c8`
//! then commits the already computed hit as a zero, unkilled result.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event},
    exact_solve_v1::ExactSolveActionV1,
    hot::HotState,
    solo_v1::admit_exact_solve,
};

const UCT_LINE: &str = r#"[{"kind":"play","target":0,"uid":1},{"kind":"play","uid":4},{"kind":"play","target":1,"uid":13},{"kind":"play","target":0,"uid":14},{"kind":"end"},{"kind":"play","uid":8},{"kind":"play","target":0,"uid":6},{"kind":"play","uid":9},{"kind":"end"},{"kind":"play","uid":4},{"kind":"play","uid":16},{"kind":"play","uid":18},{"kind":"end"},{"kind":"play","target":1,"uid":6},{"kind":"play","target":1,"uid":14},{"kind":"play","uid":8},{"kind":"end"},{"kind":"play","uid":4},{"kind":"play","uid":22},{"kind":"play","uid":23},{"kind":"play","uid":21}]"#;
const RANDOM_LINE: &str = r#"[{"kind":"play","uid":4},{"kind":"end"},{"kind":"play","target":0,"uid":6},{"kind":"play","target":0,"uid":7},{"kind":"play","uid":8},{"kind":"end"},{"kind":"play","target":1,"uid":10},{"kind":"play","uid":4},{"kind":"end"},{"kind":"play","uid":8},{"kind":"play","uid":15},{"kind":"end"},{"kind":"play","target":0,"uid":14},{"kind":"play","target":0,"uid":10},{"kind":"play","target":1,"uid":13},{"kind":"end"},{"kind":"play","uid":4},{"kind":"play","uid":23},{"kind":"play","uid":22},{"kind":"play","uid":21}]"#;

/// Replay `line` from the fixture entry; return the final state and the last
/// action's events.
fn replay(line: &str) -> (HotState, Vec<Event>) {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let document: CanonicalStateV2 =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut events = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
    }
    (state, events)
}

#[test]
fn pacts_end_into_a_thorns_inferno_kill_wins_with_a_zero_result() {
    for (line, receiver, hp) in [(UCT_LINE, 0, 28), (RANDOM_LINE, 1, 14)] {
        let (state, events) = replay(line);
        assert!(state.history.over);
        assert_eq!(state.hp, hp);
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        // Thorns 2 -> Inferno 6 kills the 1-HP receiver and ends the fight;
        // the in-flight Pact's End hit then commits zero onto the corpse.
        let tail: Vec<_> = events
            .iter()
            .skip_while(|event| !matches!(event, Event::PlayerDamaged { .. }))
            .collect();
        assert!(
            matches!(
                tail.as_slice(),
                [
                    Event::PlayerDamaged { hp_lost: 2, .. },
                    Event::MonsterDamaged { uid: a, unblocked: 6, .. },
                    Event::MonsterDied { uid: b },
                    Event::CombatOver { player_won: true },
                    Event::MonsterDamaged { uid: c, blocked: 0, unblocked: 0, .. },
                ] if *a == receiver && *b == receiver && *c == receiver
            ),
            "{tail:?}"
        );
    }
}

/// The same fight with both Toadpoles alive when Pact's End lands, so the hit
/// is a powered batch. Receiver 0 (1 HP, Thorns 2) retaliates, the player's
/// Inferno 6 kills it and leaves receiver 1 at 11 HP, and the combat goes on.
/// Native `0x3e96c8` then commits the already computed hit onto the corpse as
/// a zero result (IL_02b9 runs on to IL_04bc with no second liveness test),
/// advances to receiver 1 (IL_0a99-IL_0aa4), and Pact's End kills it. Before
/// this slice the last action refused at "Thorns retained dead receiver
/// inside a damage batch".
const BATCH_LINE: &str = r#"[{"kind":"play","target":0,"uid":1},{"kind":"play","uid":4},{"kind":"play","target":1,"uid":13},{"kind":"play","target":0,"uid":14},{"kind":"end"},{"kind":"play","uid":8},{"kind":"play","target":0,"uid":6},{"kind":"play","uid":9},{"kind":"end"},{"kind":"play","uid":4},{"kind":"play","uid":16},{"kind":"play","uid":18},{"kind":"end"},{"kind":"play","uid":8},{"kind":"end"},{"kind":"play","uid":4},{"kind":"play","uid":23},{"kind":"play","uid":21}]"#;

#[test]
fn pacts_end_batch_commits_zero_onto_the_inferno_killed_receiver_and_goes_on() {
    let (state, events) = replay(BATCH_LINE);
    assert!(state.history.over);
    assert_eq!(state.hp, 25);
    assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
    // The corpse's zero result is recorded: the combat was not ending when it
    // committed. Receiver 1 is not asserted. Its killing blow ends the
    // combat, so native skips its history row (`0x3e96c8` IL_0738-IL_0742),
    // while `commit_monster_damage` counts every powered result ungated: a
    // pre-existing difference, unobservable once the combat is over.
    assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 1);
    // The recorded row is ordinary state: it survives the canonical
    // projection and a cold reload.
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let document: CanonicalStateV2 =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    assert_eq!(
        cold.monsters
            .iter()
            .map(|monster| (monster.uid, monster.hp <= 0))
            .collect::<Vec<_>>(),
        [(0, true), (1, true)]
    );
    assert_eq!(cold.monsters[0].owner_powered_damage_results_this_turn, 1);
    let tail: Vec<_> = events
        .iter()
        .skip_while(|event| !matches!(event, Event::PlayerDamaged { .. }))
        .collect();
    assert!(
        matches!(
            tail.as_slice(),
            [
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::MonsterDamaged {
                    uid: 0,
                    unblocked: 6,
                    ..
                },
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 6,
                    hp: 11,
                    ..
                },
                Event::MonsterDied { uid: 0 },
                Event::MonsterDamaged {
                    uid: 0,
                    blocked: 0,
                    unblocked: 0,
                    ..
                },
                Event::MonsterDamaged {
                    uid: 1,
                    unblocked: 20,
                    ..
                },
                Event::MonsterDied { uid: 1 },
                Event::CombatOver { player_won: true },
            ]
        ),
        "{tail:?}"
    );
}
