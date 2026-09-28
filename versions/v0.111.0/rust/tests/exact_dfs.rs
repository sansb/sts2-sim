//! Contract tests for the pre-cutover solo-v1 exact DFS library.

use std::time::Duration;

use serde_json::Value;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action, SelectionRef};
use sts_sim::exact_dfs::{
    ExactCancellation, ExactDfsConfig, ExactDfsRefusal, ExactDfsStatus, solve,
};
use sts_sim::solo_v1::SoloV1Refusal;

const CORPUS: &str = include_str!("../fixtures/exact_solve_corpus_v1.json");

fn exact_row() -> Value {
    serde_json::from_str::<Value>(CORPUS).expect("corpus parses")["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .find(|row| row["python"]["status"] == "exact")
        .expect("one exact authority row")
        .clone()
}

fn document(row: &Value) -> CanonicalStateV2 {
    serde_json::from_value(row["entry"].clone()).expect("canonical corpus entry")
}

fn corpus_actions(row: &Value) -> Vec<Action> {
    row["python"]["action_line"]
        .as_array()
        .expect("action line")
        .iter()
        .map(|action| match action["kind"].as_str().expect("kind") {
            "end" => Action::EndTurn,
            "play" => Action::Play {
                uid: action["uid"].as_u64().expect("uid") as u32,
                target: action
                    .get("target")
                    .and_then(Value::as_u64)
                    .map(|target| target as u8),
                selection: SelectionRef::new(None),
            },
            other => panic!("unsupported corpus action {other:?}"),
        })
        .collect()
}

fn config(memo: bool) -> ExactDfsConfig {
    ExactDfsConfig {
        max_turns: 4,
        deadline: None,
        memo,
    }
}

fn compact_winnable_document(row: &Value) -> CanonicalStateV2 {
    let mut document = document(row);
    for monster in &mut document.monsters {
        monster.insert("hp".to_owned(), serde_json::json!(7));
    }
    document
}

#[test]
fn exact_corpus_row_matches_python_objective_uid_line_and_final_digest() {
    let row = exact_row();
    let document = document(&row);
    let result = solve(&document, config(true), &ExactCancellation::default());
    assert_eq!(result.status, ExactDfsStatus::Exact, "{:?}", result.refusal);
    let best = result.best.expect("Toadpoles has a winning exact line");
    assert_eq!(best.actions, corpus_actions(&row));
    assert_eq!(best.final_digest, row["python"]["final_state_digest"]);
    assert_eq!(best.objective.won, row["python"]["objective"][0] == 1);
    // #3369: an ordinary fight's win always keeps its reward tier.
    assert!(best.objective.event_reward);
    assert_eq!(best.objective.final_hp, row["python"]["objective"][1]);
    assert_eq!(
        i64::from(best.objective.potions_kept),
        row["python"]["objective"][2]
    );
    assert_eq!(
        i64::from(best.objective.negative_turns),
        row["python"]["objective"][3]
    );

    let catalog = HotBoundary::catalog_from_canonical(&document).expect("corpus catalog");
    let mut replay = HotBoundary::from_canonical(&document, &catalog).expect("corpus hot state");
    engine::admit(&document, &replay, &catalog).expect("corpus is admitted");
    let mut events = Vec::new();
    for action in &best.actions {
        replay = engine::apply_action_into(&replay, &catalog, action, &mut events)
            .expect("returned action line replays through Rust");
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&replay, &catalog)
            .expect("terminal replay projects")
            .differential_digest(),
        best.final_digest,
    );
}

#[test]
fn memo_on_and_off_preserve_the_exact_result_and_canonical_line() {
    let row = exact_row();
    // Keep the memo-off proof small but genuinely winnable: this preserves a
    // concrete action line while avoiding an exponential no-memo replay of
    // the full authority corpus merely to test memo transparency.
    let document = compact_winnable_document(&row);
    let memo_on = solve(&document, config(true), &ExactCancellation::default());
    let memo_off = solve(&document, config(false), &ExactCancellation::default());
    assert_eq!(memo_on.status, ExactDfsStatus::Exact);
    assert_eq!(memo_off.status, ExactDfsStatus::Exact);
    assert_eq!(memo_on.best, memo_off.best);
    assert!(
        memo_on.telemetry.memo_entries > 0,
        "{:?}",
        memo_on.telemetry
    );
    assert_eq!(memo_off.telemetry.memo_probes, 0);
}

#[test]
fn deadline_and_cancellation_are_explicitly_incomplete() {
    let row = exact_row();
    let deadline = solve(
        &document(&row),
        ExactDfsConfig {
            deadline: Some(Duration::ZERO),
            ..config(true)
        },
        &ExactCancellation::default(),
    );
    assert_eq!(deadline.status, ExactDfsStatus::Deadline);
    assert!(!deadline.status.is_exact());
    assert!(
        deadline.best.is_none(),
        "zero deadline cannot invent a line"
    );

    let cancellation = ExactCancellation::default();
    cancellation.cancel();
    let cancelled = solve(&document(&row), config(true), &cancellation);
    assert_eq!(cancelled.status, ExactDfsStatus::Cancelled);
    assert!(!cancelled.status.is_exact());
    assert!(
        cancelled.best.is_none(),
        "cancel before root cannot invent a line"
    );

    let unbounded_duration = solve(
        &compact_winnable_document(&row),
        ExactDfsConfig {
            deadline: Some(Duration::MAX),
            ..config(true)
        },
        &ExactCancellation::default(),
    );
    assert_eq!(unbounded_duration.status, ExactDfsStatus::Exact);
}

#[test]
fn mid_search_cancellation_keeps_an_achieved_line_but_never_claims_exactness() {
    let row = exact_row();
    let document = compact_winnable_document(&row);
    // This is not a pre-cancelled root: a deterministic node boundary lets
    // the DFS discover a real terminal win, then interrupts a still-open
    // search. Search order is intentionally not the assertion; the first
    // budget that witnesses the contract is pinned by replay below.
    let result = (2..=128)
        .find_map(|budget| {
            let cancellation = ExactCancellation::default();
            cancellation.cancel_after_nodes(budget);
            let result = solve(&document, config(true), &cancellation);
            (result.status == ExactDfsStatus::Cancelled && result.best.is_some()).then_some(result)
        })
        .expect("compact fixture reaches a win before its complete DFS closes");
    assert!(!result.status.is_exact());
    assert!(result.telemetry.nodes > 1);
    let best = result.best.expect("only a fully terminal win is retained");
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut replay = HotBoundary::from_canonical(&document, &catalog).unwrap();
    let mut events = Vec::new();
    for action in &best.actions {
        replay = engine::apply_action_into(&replay, &catalog, action, &mut events).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&replay, &catalog)
            .unwrap()
            .differential_digest(),
        best.final_digest
    );
}

#[test]
fn solo_and_boundary_failures_refuse_before_search() {
    let row = exact_row();
    let mut multiplayer = document(&row);
    multiplayer
        .player
        .insert("teammate_present".to_owned(), serde_json::json!(false));
    let denied = solve(&multiplayer, config(true), &ExactCancellation::default());
    assert_eq!(denied.status, ExactDfsStatus::Refused);
    assert_eq!(denied.telemetry.nodes, 0);
    assert_eq!(
        denied.refusal,
        Some(ExactDfsRefusal::Solo(SoloV1Refusal::TeammatePresent))
    );

    let mut damage_pending = document(&row);
    damage_pending.player.insert(
        "teammate_damage_pending".to_owned(),
        serde_json::json!(null),
    );
    let denied_damage = solve(&damage_pending, config(true), &ExactCancellation::default());
    assert_eq!(denied_damage.status, ExactDfsStatus::Refused);
    assert_eq!(denied_damage.telemetry.nodes, 0);
    assert_eq!(
        denied_damage.refusal,
        Some(ExactDfsRefusal::Solo(SoloV1Refusal::TeammateDamagePending))
    );

    let mut unsupported = document(&row);
    unsupported.player.insert(
        "unexpected_exact_dfs_field".to_owned(),
        serde_json::json!(1),
    );
    let boundary = solve(&unsupported, config(true), &ExactCancellation::default());
    assert_eq!(boundary.status, ExactDfsStatus::Refused);
    assert_eq!(boundary.telemetry.nodes, 0);
    assert!(matches!(
        boundary.refusal,
        Some(ExactDfsRefusal::Boundary(_))
    ));
}

#[test]
fn terminal_scoring_handles_large_max_hp_and_rejects_negative_turns() {
    let row = exact_row();
    let mut large = compact_winnable_document(&row);
    large
        .player
        .insert("hp".to_owned(), serde_json::json!(24_000_000));
    large
        .player
        .insert("max_hp".to_owned(), serde_json::json!(50_000_000));
    large.player.insert(
        "relics_entering".to_owned(),
        serde_json::json!(["RELIC.MEAT_ON_THE_BONE"]),
    );
    large
        .player
        .insert("meat_on_the_bone".to_owned(), serde_json::json!(true));
    let scored = solve(&large, config(true), &ExactCancellation::default());
    assert_eq!(scored.status, ExactDfsStatus::Exact, "{:?}", scored.refusal);
    assert_eq!(
        scored
            .best
            .expect("large-HP fixture wins")
            .objective
            .final_hp,
        24_000_012,
    );

    let mut negative_turn = document(&row);
    negative_turn
        .player
        .insert("turn".to_owned(), serde_json::json!(-1));
    let refused = solve(&negative_turn, config(true), &ExactCancellation::default());
    assert_eq!(refused.status, ExactDfsStatus::Refused);
    assert_eq!(refused.telemetry.nodes, 0);
    assert_eq!(refused.refusal, Some(ExactDfsRefusal::InvalidTurn(-1)));
}

/// A Battleworn Dummy root on its last timer tick (#3369): the corpus
/// Ironclad with `hand` and a lone V2 dummy at `dummy_hp`. Ending the turn
/// escapes the dummy (a won timeout); killing it first keeps the reward.
fn battleworn_last_tick(hand: &[&str], dummy_hp: i32) -> CanonicalStateV2 {
    let mut document = document(&exact_row());
    let dummy = serde_json::json!({
        "kind": "BATTLE_FRIEND_V2", "hp": dummy_hp, "max_hp": 150,
        "battleworn_time_limit": 1,
    });
    document.monsters = vec![serde_json::from_value(dummy).expect("dummy row")];
    let hand = hand
        .iter()
        .enumerate()
        .map(|(uid, id)| serde_json::json!({"id": id, "uid": uid, "upgrade": 0}))
        .collect::<Vec<_>>();
    let mut piles = serde_json::to_value(&document.piles).expect("piles");
    piles["hand"] = Value::Array(hand);
    // Keep the dealt uids disjoint from the new hand.
    for (offset, card) in piles["draw"]
        .as_array_mut()
        .expect("draw")
        .iter_mut()
        .enumerate()
    {
        card["uid"] = serde_json::json!(10 + offset);
    }
    document.piles = serde_json::from_value(piles).expect("piles round-trip");
    document
        .player
        .insert("next_card_uid".to_owned(), serde_json::json!(20));
    document
}

fn solved(document: &CanonicalStateV2) -> sts_sim::exact_dfs::ExactSolution {
    let result = solve(document, config(true), &ExactCancellation::default());
    assert_eq!(result.status, ExactDfsStatus::Exact, "{:?}", result.refusal);
    result
        .best
        .expect("a survivable Battleworn root has a best line")
}

fn timeout_objective(document: &CanonicalStateV2) -> sts_sim::exact_dfs::ExactObjective {
    // Replaying EndTurn alone must be the timeout the scorer ranks below.
    let catalog = HotBoundary::catalog_from_canonical(document).expect("catalog");
    let root = HotBoundary::from_canonical(document, &catalog).expect("root");
    let mut events = Vec::new();
    let end = engine::apply_action_into(&root, &catalog, &Action::EndTurn, &mut events)
        .expect("the timer escapes the dummy");
    assert!(end.history.over && end.monsters.is_empty());
    assert!(engine::BattlewornObjective::of_root(&root).timed_out(&end));
    sts_sim::exact_dfs::ExactObjective {
        won: true,
        event_reward: false,
        final_hp: end.hp,
        potions_kept: 0,
        negative_turns: -end.turn,
    }
}

/// #3369: at equal HP, killing the dummy beats letting it escape.
#[test]
fn a_battleworn_kill_beats_a_timeout_at_equal_hp() {
    let document = battleworn_last_tick(&["STRIKE_IRONCLAD"], 6);
    let best = solved(&document);
    let timeout = timeout_objective(&document);
    assert_eq!(
        best.objective.final_hp, timeout.final_hp,
        "Strike costs no HP"
    );
    assert!(
        best.objective.won && best.objective.event_reward,
        "{best:?}"
    );
    assert!(best.objective > timeout);
    assert!(
        matches!(best.actions.first(), Some(Action::Play { .. })),
        "the kill is the chosen line: {:?}",
        best.actions
    );
}

/// #3369: a kill that costs HP still beats the higher-HP timeout, since the
/// timeout forfeits the event's reward. Hemokinesis loses 2 HP to deal 15.
#[test]
fn a_low_hp_battleworn_kill_beats_a_high_hp_timeout() {
    let document = battleworn_last_tick(&["HEMOKINESIS"], 10);
    let best = solved(&document);
    let timeout = timeout_objective(&document);
    assert!(best.objective.event_reward, "{best:?}");
    assert!(
        best.objective.final_hp < timeout.final_hp,
        "{best:?} vs {timeout:?}"
    );
    assert!(best.objective > timeout);
}

/// #3369: when no kill is reachable the timeout is still a win, ranked above
/// every loss, and it keeps its HP.
#[test]
fn an_unkillable_battleworn_root_returns_its_timeout() {
    let document = battleworn_last_tick(&["DEFEND_IRONCLAD"], 150);
    let best = solved(&document);
    assert_eq!(best.objective, timeout_objective(&document));
    assert!(best.objective > sts_sim::exact_dfs::ExactObjective::LOSS);
    assert_eq!(best.actions.last(), Some(&Action::EndTurn));
}

#[cfg(feature = "allocation-counting")]
#[test]
fn allocation_telemetry_is_marked_when_the_instrumented_build_runs() {
    let row = exact_row();
    let result = solve(
        &compact_winnable_document(&row),
        config(true),
        &ExactCancellation::default(),
    );
    assert_eq!(result.status, ExactDfsStatus::Exact);
    assert!(result.telemetry.allocation_instrumented);
    // The integration-test crate does not install the library test allocator,
    // so counts can be zero here; the contract is that this result labels the
    // build accurately and exposes both counters to the allocation lane.
    assert!(result.telemetry.allocations <= result.telemetry.allocated_bytes);
}
