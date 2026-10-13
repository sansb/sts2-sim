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
    // (The capture's checksums are dropped: with them, an input after the
    // won fight is one the game cancelled, tested below.)
    let mut unproved = with(&|e| e.push(e[end].clone()));
    unproved["checksums"] = json!([]);
    assert_eq!(check_of(&entry, &unproved), "premature_combat_end");
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

/// The Mawler root, parked on a Gambling Chip choice over its dealt hand.
fn parked_on_gambling_chip() -> CanonicalStateV2 {
    let path = sim()
        .join("eval/fights")
        .join(PAIRS[0].1)
        .join("entry.canonical.json");
    let mut entry: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
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

/// #3756: an input recorded while a turn-start hook's hand choice is open. A
/// card play is cancelled by the game and dropped here; a potion waits and is
/// applied once the choice is answered.
#[test]
fn an_input_recorded_behind_an_open_hook_choice_is_dropped_or_waits() {
    let entry = parked_on_gambling_chip();
    let good = replay(PAIRS[0].0);
    let play = good["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"]["type"] == "NetPlayCardAction")
        .unwrap()
        .clone();
    let hook = json!({"event_type": "HookAction", "hook_id": 0});
    let potion = json!({"event_type": "GameAction", "action": {
        "type": "NetUsePotionAction", "potion_index": 0, "target_id": 1}});
    // The hook's own answer: discard the first dealt card.
    let choice = json!({"event_type": "PlayerChoice", "choice_id": 0, "result": {
        "type": "CombatCard", "combat_card_indexes": [0]}});
    let resume = json!({"event_type": "ResumeAction", "action_id": 0});
    let line_of = |events: Vec<Value>| {
        let mut replay = good.clone();
        replay["events"] = json!(events);
        recorded::recorded_line(&entry, &replay).unwrap_err()
    };

    // The play never ran: only the select was applied, and the line then
    // stops where the inputs do.
    let dropped = line_of(vec![
        hook.clone(),
        play.clone(),
        choice.clone(),
        resume.clone(),
    ]);
    assert_eq!(
        (dropped.check, dropped.step),
        ("combat_not_complete", 1),
        "{dropped:?}"
    );
    // The potion ran after the choice: two actions.
    let waited = line_of(vec![
        hook.clone(),
        potion.clone(),
        choice.clone(),
        resume.clone(),
    ]);
    assert_eq!(
        (waited.check, waited.step),
        ("combat_not_complete", 2),
        "{waited:?}"
    );
    // A choice the capture never answers leaves the potion waiting.
    let unanswered = line_of(vec![hook, potion]);
    assert_eq!(
        (unanswered.check, unanswered.step),
        ("not_legal", 0),
        "{unanswered:?}"
    );
    assert!(
        unanswered.detail.contains("input 1 waits behind a choice"),
        "{unanswered:?}"
    );
}

/// #3756: inputs queued behind the killing blow. The game cancels them when
/// the combat ends, so they are in the event list and were never run. They
/// are dropped only on the capture's own evidence.
#[test]
fn inputs_recorded_after_the_won_fight_are_dropped_only_on_the_captures_evidence() {
    let (entry, expected) = fixture(PAIRS[0].1);
    let good = replay(PAIRS[0].0);
    let events = good["events"].as_array().unwrap();
    let of = |kind: &str| {
        events
            .iter()
            .find(|e| e["action"]["type"] == kind)
            .unwrap()
            .clone()
    };
    let mut queued = good.clone();
    let trailing = queued["events"].as_array_mut().unwrap();
    trailing.push(of("NetPlayCardAction"));
    trailing.push(of("NetEndPlayerTurnAction"));
    // The fight was won, every finished play has its checksum, and the final
    // checksum has no living enemy: the two trailing inputs never ran.
    let line = recorded::recorded_line(&entry, &queued).unwrap();
    assert_eq!(
        serde_json::to_value(&line.actions).unwrap(),
        expected["actions"]
    );
    assert_eq!(line.terminal, expected["terminal"]);
    assert_eq!(line.terminal["won"], true);

    let diverges = |edit: &dyn Fn(&mut Vec<Value>)| {
        let mut damaged = queued.clone();
        edit(damaged["checksums"].as_array_mut().unwrap());
        let diverged = recorded::recorded_line(&entry, &damaged).unwrap_err();
        assert_eq!(diverged.check, "premature_combat_end", "{diverged:?}");
    };
    let finished = |c: &Value| {
        c["context"]
            .as_str()
            .is_some_and(|context| context.starts_with("finished action execution PlayCardAction "))
    };
    // The control: the capture holds one more finished play than the engine
    // applied, so the game's combat went on past the engine's.
    diverges(&|c| {
        let extra = c.iter().find(|c| finished(c)).unwrap().clone();
        c.insert(0, extra);
    });
    // One fewer than applied is not this reading either.
    diverges(&|c| {
        let at = c.iter().position(finished).unwrap();
        c.remove(at);
    });
    // A final checksum with a living enemy: the game's combat was not over.
    diverges(&|c| {
        let enemy = json!({"monster_id": "MAWLER", "player_id": null, "current_hp": 7});
        c.last_mut().unwrap()["full_state"]["creatures"]
            .as_array_mut()
            .unwrap()
            .push(enemy);
    });
    // A dead player in it, and a capture with no checksum at all.
    diverges(&|c| {
        for creature in c.last_mut().unwrap()["full_state"]["creatures"]
            .as_array_mut()
            .unwrap()
        {
            creature["current_hp"] = json!(0);
        }
    });
    diverges(&|c| c.clear());
    // A pet is the player's, not an enemy.
    let mut with_pet = queued.clone();
    with_pet["checksums"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["full_state"]["creatures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"monster_id": "OSTY", "player_id": null, "current_hp": 4}));
    assert!(recorded::recorded_line(&entry, &with_pet).is_ok());
}

/// The Mawler root with a Burning Pact as its first dealt card and `hand`
/// dealt cards in Hand (the rest on top of the Draw pile, so the deal reads
/// as before), and the capture's deck changed to match.
fn burning_pact_fight(hand: usize, second: &str) -> (CanonicalStateV2, Value) {
    let path = sim()
        .join("eval/fights")
        .join(PAIRS[0].1)
        .join("entry.canonical.json");
    let mut entry: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut replay = replay(PAIRS[0].0);
    let mut dealt = entry["piles"]["hand"].as_array().unwrap().clone();
    let swap = |card: &mut Value, id: &str, replay: &mut Value| {
        let old = card["id"].as_str().unwrap().to_owned();
        let row = replay["run"]["players"][0]["deck"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["id"].as_str().unwrap().ends_with(&old))
            .unwrap();
        row["id"] = json!(format!("CARD.{id}"));
        card["id"] = json!(id);
    };
    swap(&mut dealt[0], "BURNING_PACT", &mut replay);
    swap(&mut dealt[1], second, &mut replay);
    let mut draw = dealt.split_off(hand);
    draw.extend(entry["piles"]["draw"].as_array().unwrap().iter().cloned());
    entry["piles"]["hand"] = json!(dealt);
    entry["piles"]["draw"] = json!(draw);
    (serde_json::from_value(entry).unwrap(), replay)
}

/// #3756: a selecting play recorded with no choice. With one other card in
/// Hand the game resolves Burning Pact's exhaust without asking, and the
/// capture holds the play and nothing else; the resolver takes the one
/// selection the engine offers.
#[test]
fn a_selecting_play_with_no_recorded_choice_takes_its_one_selection() {
    let pact = json!({"event_type": "GameAction", "action": {
        "type": "NetPlayCardAction", "combat_card_index": 0,
        "card_id": "CARD.BURNING_PACT", "target_id": null}});
    let resume = json!({"event_type": "ResumeAction", "action_id": 0});
    let choice = |indexes: Value| {
        json!({"event_type": "PlayerChoice", "choice_id": 0, "result": {
            "type": "CombatCard", "combat_card_indexes": indexes}})
    };
    let outcome = |hand: usize, second: &str, events: Vec<Value>| {
        let (entry, mut replay) = burning_pact_fight(hand, second);
        replay["events"] = json!(events);
        let diverged = recorded::recorded_line(&entry, &replay).unwrap_err();
        (diverged.check, diverged.step)
    };
    let alone = vec![pact.clone(), resume.clone()];
    // Burning Pact and one other card: the play resolves, and the line then
    // stops where the inputs do.
    assert_eq!(
        outcome(2, "BREAKTHROUGH", alone.clone()),
        ("combat_not_complete", 1)
    );
    assert_eq!(
        outcome(2, "BREAKTHROUGH", vec![pact.clone()]),
        ("combat_not_complete", 1)
    );
    // The recorded choice is still what resolves a play that has one.
    assert_eq!(
        outcome(3, "BREAKTHROUGH", vec![pact.clone(), choice(json!([1]))]),
        ("combat_not_complete", 1)
    );
    // Two different candidates and no recorded choice: nothing is guessed.
    assert_eq!(outcome(3, "BREAKTHROUGH", alone.clone()), ("not_legal", 0));
    // Two IDENTICAL candidates are one offered selection, and still two
    // cards the game would have asked about.
    assert_eq!(outcome(3, "STRIKE_IRONCLAD", alone), ("not_legal", 0));
    // A recorded choice the play cannot take is not replaced by the sole
    // selection.
    assert_eq!(
        outcome(2, "BREAKTHROUGH", vec![pact, choice(json!([1, 2]))]),
        ("not_legal", 0)
    );
}
