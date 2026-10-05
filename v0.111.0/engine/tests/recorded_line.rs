//! The recorded-line resolver (#3578 slice B1), without Python and without
//! the capture corpus.
//!
//! `tools/recorded_parity.py` is the parity check over every eval fixture
//! whose capture is on the machine (575 of 575 identical on 2026-10-01). Two
//! of those captures are committed under `python/testdata`, so these tests
//! hold the resolver to their fixtures' `human_line.json` in CI, and reach
//! each named divergence by damaging a decoded replay.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::{mcr, recorded};

/// (capture under python/testdata, its eval fixture).
const PAIRS: [(&str, &str); 2] = [
    ("6P96T755CNZ3_mawler_win.mcr", "f04442cd475cdc72"),
    ("YLVVPKPH1MTW_f33_the_insatiable.mcr", "fc78829c88941121"),
];

fn sim() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn replay(capture: &str) -> Value {
    mcr::decode(&std::fs::read(sim().join("python/testdata").join(capture)).unwrap()).unwrap()
}

fn fixture(id: &str) -> (CanonicalStateV2, Value) {
    let dir = sim().join("eval/fights").join(id);
    let entry =
        serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).unwrap()).unwrap();
    let line =
        serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
    (entry, line)
}

fn check_of(entry: &CanonicalStateV2, replay: &Value) -> &'static str {
    recorded::recorded_line(entry, replay)
        .expect_err("the damaged replay must diverge")
        .check
}

#[test]
fn a_committed_capture_resolves_to_its_fixtures_line() {
    for (capture, id) in PAIRS {
        let (entry, expected) = fixture(id);
        let line = recorded::recorded_line(&entry, &replay(capture))
            .unwrap_or_else(|d| panic!("{capture}: {d:?}"));
        assert_eq!(
            serde_json::to_value(&line.actions).unwrap(),
            expected["actions"],
            "{capture}: actions"
        );
        assert_eq!(
            json!(line.step_digests),
            expected["step_digests"],
            "{capture}: digests"
        );
        assert_eq!(line.terminal, expected["terminal"], "{capture}: terminal");
        assert_eq!(line.actions.len(), line.step_digests.len());
    }
}

#[test]
fn a_capture_of_another_fight_is_refused_at_the_deal() {
    // The Insatiable's capture against the Mawler root: another deck.
    let (mawler, _) = fixture(PAIRS[0].1);
    let diverged = recorded::recorded_line(&mawler, &replay(PAIRS[1].0)).unwrap_err();
    assert_eq!((diverged.check, diverged.step), ("entry_deal", 0));
}

#[test]
fn each_damaged_input_diverges_by_name() {
    let (entry, expected) = fixture(PAIRS[0].1);
    let good = replay(PAIRS[0].0);
    let events = good["events"].as_array().unwrap();
    let position = |kind: &str| {
        events
            .iter()
            .position(|e| e["action"]["type"] == kind)
            .unwrap()
    };
    let with = |edit: &dyn Fn(&mut Vec<Value>)| {
        let mut damaged = good.clone();
        edit(damaged["events"].as_array_mut().unwrap());
        damaged
    };
    let play = position("NetPlayCardAction");
    let end = position("NetEndPlayerTurnAction");

    // No inputs at all, and more than the budget allows.
    assert_eq!(
        check_of(&entry, &with(&|e| e.clear())),
        "recorded_input_budget"
    );
    assert_eq!(
        check_of(&entry, &with(&|e| *e = vec![e[0].clone(); 2001])),
        "recorded_input_budget"
    );
    // A card index that names another card than the recorded id.
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e[play]["action"]["card_id"] = json!("NOT_A_CARD"))
        ),
        "card_identity"
    );
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e[play]["action"]["combat_card_index"] = json!(-1))
        ),
        "card_identity"
    );
    // A target the roster does not hold.
    let targeted = events
        .iter()
        .position(|e| e["action"]["target_id"].is_u64())
        .unwrap();
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e[targeted]["action"]["target_id"] = json!(60))
        ),
        "target_identity"
    );
    // The wrong turn number on End Turn.
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e[end]["action"]["turn_number"] = json!(9))
        ),
        "turn_number"
    );
    // An event type the line does not model.
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e[play] = json!({"event_type": "Mystery"}))
        ),
        "unsupported_event"
    );
    // A choice where the engine is waiting on none.
    assert_eq!(
        check_of(
            &entry,
            &with(&|e| e.insert(
                play,
                json!({"event_type": "PlayerChoice", "result": {"type": "Index", "indexes": [0]}})
            ))
        ),
        "selection"
    );
    // Inputs that stop before the fight ends, and one that continues after it.
    assert_eq!(
        check_of(&entry, &with(&|e| e.truncate(play + 1))),
        "combat_not_complete"
    );
    assert_eq!(
        check_of(&entry, &with(&|e| e.push(e[end].clone()))),
        "premature_combat_end"
    );
    // The same play twice in a row: the second names a card no longer in hand.
    let diverged =
        recorded::recorded_line(&entry, &with(&|e| e.insert(play, e[play].clone()))).unwrap_err();
    assert!(
        ["card_identity", "not_legal"].contains(&diverged.check),
        "{diverged:?}"
    );
    assert_eq!(
        diverged.step, 1,
        "one action was applied before the divergence"
    );
    // A trailing no-decision event after the end changes nothing.
    let trailing = with(&|e| e.push(json!({"event_type": "HookAction"})));
    let line = recorded::recorded_line(&entry, &trailing).unwrap();
    assert_eq!(line.terminal, expected["terminal"]);
}

#[test]
fn a_replay_without_its_shuffle_stream_or_with_two_players_is_refused_at_the_deal() {
    let (entry, _) = fixture(PAIRS[0].1);
    let good = replay(PAIRS[0].0);
    let mut no_stream = good.clone();
    no_stream["run"]["rng"]["states"]
        .as_object_mut()
        .unwrap()
        .remove("Shuffle");
    assert_eq!(check_of(&entry, &no_stream), "entry_deal");
    let mut co_op = good.clone();
    let player = co_op["run"]["players"][0].clone();
    co_op["run"]["players"].as_array_mut().unwrap().push(player);
    assert_eq!(check_of(&entry, &co_op), "entry_deal");
    // A different shuffle state deals another order: the identities differ.
    let mut reshuffled = good;
    reshuffled["run"]["rng"]["states"]["Shuffle"][0] = json!(12345u64);
    assert_eq!(check_of(&entry, &reshuffled), "entry_deal");
}
