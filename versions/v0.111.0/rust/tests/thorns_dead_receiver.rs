//! #2655: a powered hit whose receiver died to a nested kill during Thorns
//! retaliation, where that kill ended the combat.
//!
//! Both lines are the stored `first_refusal.actions` of the KD13JGCDPB3U
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
