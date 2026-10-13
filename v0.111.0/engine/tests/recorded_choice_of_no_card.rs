//! #3756: a recorded `PlayerChoice` that names no card.
//!
//! The game records the player's whole answer, so a selection with a minimum
//! of zero, confirmed with nothing picked, is `combat_card_indexes: []`
//! (Gambling Chip with nothing discarded; `rust_replay._selection` carries
//! the IL). The resolver takes it as the one offered answer selecting no
//! card. The Python twin is
//! `python/test_issue3756_recorded_choice_of_no_card.py`.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::{mcr, recorded};

const CAPTURE: &str = "6P96T755CNZ3_mawler_win.mcr";
const FIXTURE: &str = "f04442cd475cdc72";

fn sim() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixture(name: &str) -> Value {
    let path = sim().join("eval/fights").join(FIXTURE).join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn replay_with(prefix: Vec<Value>) -> Value {
    let mut replay =
        mcr::decode(&std::fs::read(sim().join("python/testdata").join(CAPTURE)).unwrap()).unwrap();
    let events = replay["events"].as_array_mut().unwrap();
    events.splice(0..0, prefix);
    replay
}

/// The Mawler root, parked on a Gambling Chip choice over its dealt hand.
fn parked_on_gambling_chip() -> CanonicalStateV2 {
    let mut entry = fixture("entry.canonical.json");
    let hand: Vec<Value> = entry["piles"]["hand"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| {
            json!([
                card["uid"],
                [card["id"], card["upgrade"].as_u64().unwrap_or(0)]
            ])
        })
        .collect();
    let player = entry["player"].as_object_mut().unwrap();
    player.insert("pending".to_owned(), json!(["gambling_chip_select", hand]));
    player.insert("gambling_chip".to_owned(), json!(true));
    player["relics_entering"]
        .as_array_mut()
        .unwrap()
        .push(json!("RELIC.GAMBLING_CHIP"));
    serde_json::from_value(entry).unwrap()
}

fn choice(indexes: Value) -> Value {
    json!({"event_type": "PlayerChoice", "choice_id": 0, "result": {
        "type": "CombatCard", "combat_card_indexes": indexes}})
}

#[test]
fn a_gambling_chip_choice_of_no_card_resolves_and_the_fight_goes_on_as_recorded() {
    let entry = parked_on_gambling_chip();
    let hook = json!({"event_type": "HookAction", "hook_id": 0});
    let resume = json!({"event_type": "ResumeAction", "action_id": 0});
    let line = recorded::recorded_line(&entry, &replay_with(vec![hook, choice(json!([])), resume]))
        .unwrap();
    // One select, discarding nothing, and then the fixture's own line: with
    // no card discarded and none drawn, the fight is the one the capture
    // recorded.
    let expected = fixture("human_line.json");
    assert!(matches!(line.actions[0], ExactSolveActionV1::Select { .. }));
    assert_eq!(
        serde_json::to_value(&line.actions[1..]).unwrap(),
        expected["actions"]
    );
    assert_eq!(line.terminal, expected["terminal"]);
}

#[test]
fn an_empty_choice_is_not_waved_through() {
    // The control: the same empty choice where the engine is waiting on
    // none (the unparked Mawler root) has no answer and diverges by name.
    let entry: CanonicalStateV2 = serde_json::from_value(fixture("entry.canonical.json")).unwrap();
    let diverged =
        recorded::recorded_line(&entry, &replay_with(vec![choice(json!([]))])).unwrap_err();
    assert_eq!((diverged.check, diverged.step), ("selection", 0));
    // A card list that is not a list is refused by name too.
    let parked = parked_on_gambling_chip();
    let diverged =
        recorded::recorded_line(&parked, &replay_with(vec![choice(json!(null))])).unwrap_err();
    assert_eq!(diverged.check, "selection");
    assert!(
        diverged.detail.contains("list of physical cards"),
        "{diverged:?}"
    );
}
