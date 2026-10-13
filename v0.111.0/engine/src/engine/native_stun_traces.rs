//! The generic stun against stored native traces, on real encounter roots
//! (#3823).
//!
//! `fixtures/native_stun_traces_v1.json` holds runs of the real game
//! (`sts2.dll` v0.111.0, the headless harness): Ironclad at ascension 10, HP
//! 9000, one encounter, and a schedule of stuns and card plays. Each turn
//! records what the game showed after the enemy turn: every monster's HP,
//! block, powers and next move, the player's HP loss and powers (`pp`), and
//! how far each RNG stream moved.
//!
//! A monster whose machine rolls carries its move log in the document
//! (`move_log`, `next_move`). For those the native row holds the tail of the
//! game's `StateLog`, and the two are compared by name: a stun logs the parked
//! move a second time, and that entry is what the next roll's weights read.
//!
//! The player's powers are what a stunned debuffer moves: its Weak, Frail,
//! Shrink, Tangled or Smoggy lands one turn late, and with no card played
//! that is the only place the shift shows.
//!
//! The test opens the same fight through the entry-facts opening (the roster
//! the game rolled is the fixture's `node_index`), drives the engine through
//! the same schedule and compares turn by turn. A stun is
//! [`install_generic_stun`](super::monsters::install_generic_stun), the body
//! of Whistle's `CreatureCmd::Stun(target, null)`; a play is the public
//! action; `h` sets a monster's current HP, as the probe did. After every schedule step and every enemy turn the state is
//! projected, hydrated and admitted again, so the stunned state round-trips
//! the canonical document in its real roster.
//!
//! Nothing here is derived from the engine's own unstunned run.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::boundary::HotBoundary;
use crate::canonical::CanonicalStateV2;
use crate::catalog::Catalog;
use crate::engine::{self, Action};
use crate::hot::HotState;

const FIXTURE: &str = include_str!("../../fixtures/native_stun_traces_v1.json");

fn project(state: &HotState, catalog: &Catalog) -> CanonicalStateV2 {
    HotBoundary::try_to_canonical(state, catalog).expect("the state projects")
}

/// Project, hydrate, admit, project again: one document, one state.
fn round_trip(state: &HotState, catalog: &Catalog, at: &str, problems: &mut Vec<String>) {
    let document = match HotBoundary::try_to_canonical(state, catalog) {
        Ok(document) => document,
        Err(refusal) => {
            problems.push(format!("{at}: does not project: {refusal:?}"));
            return;
        }
    };
    let rebuilt = HotBoundary::catalog_from_canonical(&document).expect("catalog");
    let restored = match HotBoundary::from_canonical(&document, &rebuilt) {
        Ok(restored) => restored,
        Err(refusal) => {
            problems.push(format!("{at}: does not hydrate: {refusal:?}"));
            return;
        }
    };
    if !state.history.over
        && let Err(refusal) = engine::admit(&document, &restored, &rebuilt)
    {
        problems.push(format!("{at}: reload not admitted: {refusal:?}"));
    }
    if HotBoundary::try_to_canonical(&restored, &rebuilt)
        .ok()
        .as_ref()
        != Some(&document)
    {
        problems.push(format!("{at}: the reloaded state projects differently"));
    }
}

fn open(fixture: &Value, case: &Value) -> (Catalog, HotState) {
    let document: CanonicalStateV2 = if let Some(entry) = case.get("entry") {
        // An event-started encounter the entry facts cannot name: the root is
        // the fixture's own document.
        serde_json::from_value(entry.clone()).expect("entry document")
    } else {
        let key = case["facts"].as_str().expect("facts key");
        let row = &fixture["facts"][key];
        let mut facts = fixture["facts_template"][row[0].as_str().unwrap()].clone();
        facts["entry"]["encounter_id"] = row[1].clone();
        facts["entry"]["node_index"] = row[2].clone();
        facts["entry"]["node_type"] = row[3].clone();
        facts["entry"]["deck_entering"] = Value::Array(
            row[4]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| json!({ "id": id, "upgrade_level": 0 }))
                .collect(),
        );
        let parsed =
            crate::entry::facts::parse(&facts.to_string(), crate::catalog::GameBuild::V0_111_0)
                .unwrap_or_else(|refusal| panic!("{key}: facts refused: {refusal:?}"));
        match crate::entry::root_from_document(parsed, None) {
            crate::entry::RootOutcome::Opened(opening) => opening.document,
            _ => panic!("{key}: the opening refused"),
        }
    };
    let catalog = HotBoundary::catalog_from_canonical(&document).expect("catalog");
    let mut state = HotBoundary::from_canonical(&document, &catalog).expect("root hydrates");
    // The probe sets the player's HP so that nine unanswered turns are survived.
    state.hp = 9_000;
    state.max_hp = 9_000;
    (catalog, state)
}

/// One player power as the document carries it: a number, a flag (1), or
/// Constrict's per-source list (its total). Absent is 0.
fn player_power(player: &Value, field: &str) -> i64 {
    match player.get(field) {
        None | Some(Value::Null) => 0,
        Some(Value::Bool(flag)) => i64::from(*flag),
        Some(Value::Array(sources)) => sources
            .iter()
            .map(|source| source[1].as_i64().expect("a [source, amount] row"))
            .sum(),
        Some(value) => value.as_i64().expect("a numeric player power"),
    }
}

/// A move's name as the fixture spells it: the native id without `_MOVE`.
fn move_name(name: &str) -> &str {
    name.strip_suffix("_MOVE").unwrap_or(name)
}

fn monster_index(state: &HotState, uid: u64) -> Option<usize> {
    state
        .monsters
        .iter()
        .position(|monster| u64::from(monster.uid) == uid)
}

#[test]
fn the_generic_stun_matches_the_stored_native_traces_on_real_rosters() {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    assert_eq!(fixture["schema"], "native-stun-traces-v1");
    let vocabulary: BTreeSet<&str> = fixture["power_vocabulary"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    // Native powers the document carries by kind, with the amounts seen.
    let unwired: BTreeMap<(&str, &str), Vec<i64>> = fixture["unwired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                (row[0].as_str().unwrap(), row[1].as_str().unwrap()),
                row[2]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|amount| amount.as_i64().unwrap())
                    .collect(),
            )
        })
        .collect();

    // Native player power -> the document's player field, and the native
    // player powers the document does not carry, with the amounts seen.
    let player_fields: BTreeMap<&str, &str> = fixture["player_power_fields"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(native, field)| (native.as_str(), field.as_str().unwrap()))
        .collect();
    let player_unwired: BTreeMap<&str, Vec<i64>> = fixture["player_unwired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row[0].as_str().unwrap(),
                row[1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|amount| amount.as_i64().unwrap())
                    .collect(),
            )
        })
        .collect();

    let mut problems: Vec<String> = Vec::new();
    let mut player_powers_compared = 0_usize;
    let mut move_logs_compared = 0_usize;
    let mut kinds: BTreeSet<String> = BTreeSet::new();
    let mut turns_compared = 0_usize;
    let mut stuns_installed = 0_usize;
    let mut plays = 0_usize;
    // (native monster type, native move) -> the engine's names for it, and back.
    let mut names: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let cases = fixture["cases"].as_array().unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        kinds.insert(case["kind"].as_str().unwrap().to_owned());
        let (catalog, mut state) = open(&fixture, case);
        let before_case = problems.len();
        'turns: for (turn_index, turn) in case["turns"].as_array().unwrap().iter().enumerate() {
            let at = format!("{name} turn {}", turn_index + 1);
            let hp_before = state.hp;
            let rng_before =
                serde_json::to_value(project(&state, &catalog)).unwrap()["rng"].clone();
            for act in turn["acts"].as_array().unwrap() {
                let kind = act[0].as_str().unwrap();
                let uid = act[act.as_array().unwrap().len() - 1].as_u64().unwrap();
                let Some(index) = monster_index(&state, uid) else {
                    problems.push(format!("{at}: no monster uid {uid} for {act}"));
                    break 'turns;
                };
                if state.monsters[index].hp <= 0 {
                    continue;
                }
                if state.history.over {
                    // The card before this one ended the fight.
                    break;
                }
                match kind {
                    "s" | "ss" => {
                        for _ in 0..if kind == "ss" { 2 } else { 1 } {
                            if let Err(refusal) =
                                super::monsters::install_generic_stun(&mut state, index)
                            {
                                problems.push(format!("{at}: stun refused: {refusal:?}"));
                                break 'turns;
                            }
                        }
                        stuns_installed += 1;
                        let shown = super::turn::monster_intents(&state, &catalog)[index].intent;
                        if shown != Some("STUNNED") {
                            problems.push(format!("{at}: stunned monster shows {shown:?}"));
                        }
                    }
                    "p" => {
                        let card = act[1].as_str().unwrap();
                        let document = serde_json::to_value(project(&state, &catalog)).unwrap();
                        let Some(card_uid) = document["piles"]["hand"]
                            .as_array()
                            .and_then(|hand| {
                                hand.iter()
                                    .find(|held| held["id"].as_str().unwrap().contains(card))
                            })
                            .and_then(|held| held["uid"].as_u64())
                        else {
                            problems.push(format!("{at}: no {card} in hand"));
                            break 'turns;
                        };
                        let Some(action) = engine::legal_actions(&state, &catalog)
                            .into_iter()
                            .find(|action| {
                                matches!(action, Action::Play { uid, target, .. }
                                    if u64::from(*uid) == card_uid
                                        && *target == Some(index as u8))
                            })
                        else {
                            problems.push(format!("{at}: {card} on uid {uid} is not legal"));
                            break 'turns;
                        };
                        match engine::apply_action_into(&state, &catalog, &action, &mut Vec::new())
                        {
                            Ok(next) => state = next,
                            Err(refusal) => {
                                problems.push(format!("{at}: {card} refused: {refusal:?}"));
                                break 'turns;
                            }
                        }
                        plays += 1;
                    }
                    // A root edit, like the player's 9,000 HP: the probe set
                    // this monster's current HP (so that one real card can
                    // kill a monster its block would otherwise keep alive).
                    "h" => {
                        let hp = i32::try_from(act[1].as_i64().unwrap()).unwrap();
                        state.monsters_mut()[index].hp = hp;
                    }
                    other => panic!("{at}: unknown act {other}"),
                }
                round_trip(
                    &state,
                    &catalog,
                    &format!("{at} after {act}"),
                    &mut problems,
                );
            }
            // A kill that ends the fight in the player's turn leaves no enemy
            // turn to run; the native run shows the fight over at this turn.
            if !state.history.over {
                match engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut Vec::new())
                {
                    Ok(next) => state = next,
                    Err(refusal) => {
                        problems.push(format!("{at}: end turn refused: {refusal:?}"));
                        break 'turns;
                    }
                }
                turns_compared += 1;
                round_trip(
                    &state,
                    &catalog,
                    &format!("{at} after the enemy turn"),
                    &mut problems,
                );
            }
            let document = serde_json::to_value(project(&state, &catalog)).unwrap();

            let native_over = turn["over"].as_bool().unwrap();
            if native_over != state.history.over {
                problems.push(format!(
                    "{at}: fight over: native {native_over}, engine {}",
                    state.history.over
                ));
            }
            let loss = i64::from(hp_before) - i64::from(state.hp);
            if loss != turn["loss"].as_i64().unwrap() {
                problems.push(format!(
                    "{at}: player HP loss: native {}, engine {loss}",
                    turn["loss"]
                ));
            }
            let mut engine_rng = BTreeMap::new();
            for (stream, after) in document["rng"].as_object().unwrap() {
                let delta = after["counter"].as_i64().unwrap()
                    - rng_before[stream]["counter"].as_i64().unwrap_or(0);
                if delta != 0 {
                    engine_rng.insert(stream.clone(), delta);
                }
            }
            let native_rng: BTreeMap<String, i64> = turn["rng"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(stream, delta)| (stream.clone(), delta.as_i64().unwrap()))
                .collect();
            if native_rng != engine_rng {
                problems.push(format!(
                    "{at}: RNG draws: native {native_rng:?}, engine {engine_rng:?}"
                ));
            }
            if native_over || state.history.over {
                break;
            }

            // The player's powers after the enemy turn.
            let player = &document["player"];
            let native_powers = turn["pp"].as_object().unwrap();
            for (power, amount) in native_powers {
                let amount = amount.as_i64().unwrap();
                let Some(field) = player_fields.get(power.as_str()) else {
                    if !player_unwired
                        .get(power.as_str())
                        .is_some_and(|amounts| amounts.contains(&amount))
                    {
                        problems.push(format!(
                            "{at}: player {power} {amount} natively, no engine field"
                        ));
                    }
                    continue;
                };
                let held = player_power(player, field);
                // The document carries Shrink as a count, native as -count.
                let amount = if power == "shrink" { -amount } else { amount };
                if held != amount {
                    problems.push(format!(
                        "{at}: player {power} native {amount}, engine {held}"
                    ));
                }
                player_powers_compared += 1;
            }
            for (native, field) in &player_fields {
                if !native_powers.contains_key(*native) && player_power(player, field) != 0 {
                    problems.push(format!(
                        "{at}: engine-only player power {field} = {}",
                        player[*field]
                    ));
                }
            }

            let intents = super::turn::monster_intents(&state, &catalog);
            let mut seen = BTreeSet::new();
            for native in turn["mons"].as_array().unwrap() {
                let uid = native[0].as_u64().unwrap();
                let kind = native[1].as_str().unwrap();
                let dead = native[2].as_bool().unwrap();
                seen.insert(uid);
                let tag = format!("{at}: {kind}#{uid}");
                let Some(index) = monster_index(&state, uid) else {
                    if !dead {
                        problems.push(format!("{tag}: alive natively, absent in the engine"));
                    }
                    continue;
                };
                let monster = &document["monsters"][index];
                let hp = monster["hp"].as_i64().unwrap();
                if dead {
                    if hp > 0 {
                        problems.push(format!("{tag}: dead natively, engine HP {hp}"));
                    }
                    continue;
                }
                if hp != native[3].as_i64().unwrap() {
                    problems.push(format!("{tag}: HP native {}, engine {hp}", native[3]));
                }
                let block = monster["block"].as_i64().unwrap_or(0);
                if block != native[4].as_i64().unwrap() {
                    problems.push(format!("{tag}: block native {}, engine {block}", native[4]));
                }
                let powers = native[6].as_object().unwrap();
                for (power, amount) in powers {
                    let amount = amount.as_i64().unwrap();
                    // A flag power is a document bool; native counts it as 1.
                    let held = monster
                        .get(power)
                        .and_then(|held| held.as_i64().or_else(|| held.as_bool().map(i64::from)));
                    match held {
                        Some(held) if held == amount => {}
                        Some(held) => {
                            problems.push(format!("{tag}: {power} native {amount}, engine {held}"))
                        }
                        None => {
                            if !unwired
                                .get(&(kind, power.as_str()))
                                .is_some_and(|amounts| amounts.contains(&amount))
                            {
                                problems.push(format!(
                                    "{tag}: {power} {amount} natively, no engine field"
                                ));
                            }
                        }
                    }
                }
                for (field, held) in monster.as_object().unwrap() {
                    if vocabulary.contains(field.as_str()) && !powers.contains_key(field) {
                        problems.push(format!("{tag}: engine-only power {field} = {held}"));
                    }
                }
                let next = native[5].as_str().unwrap();
                let shown = intents[index].intent;
                if (next == "STUNNED") != (shown == Some("STUNNED")) {
                    problems.push(format!("{tag}: next native {next}, engine {shown:?}"));
                }
                if next != "STUNNED" && monster.get("override").is_some() {
                    problems.push(format!(
                        "{tag}: engine override {} where native shows {next}",
                        monster["override"]
                    ));
                }
                names
                    .entry((kind.to_owned(), next.to_owned()))
                    .or_default()
                    .insert(shown.unwrap_or("(unnamed)").to_owned());
                // A rolling machine: the document's log is the tail of the
                // native StateLog, and its next move is the native one.
                if let Some(engine_log) = monster.get("move_log").and_then(Value::as_array) {
                    let engine_log: Vec<&str> = engine_log
                        .iter()
                        .map(|entry| move_name(entry.as_str().unwrap()))
                        .collect();
                    let Some(native_log) = native.get(7).and_then(Value::as_array) else {
                        problems.push(format!("{tag}: no native StateLog in the fixture"));
                        continue;
                    };
                    let native_log: Vec<&str> = native_log
                        .iter()
                        .map(|entry| entry.as_str().unwrap())
                        .collect();
                    let tail = &native_log[native_log.len().saturating_sub(engine_log.len())..];
                    if tail != engine_log.as_slice() {
                        problems.push(format!(
                            "{tag}: StateLog native {native_log:?}, engine {engine_log:?}"
                        ));
                    }
                    let engine_next = monster["next_move"].as_str().map(move_name);
                    if engine_next != Some(next) {
                        problems.push(format!(
                            "{tag}: next move native {next}, engine {engine_next:?}"
                        ));
                    }
                    move_logs_compared += 1;
                }
            }
            for monster in state.monsters.iter() {
                if monster.hp > 0 && !seen.contains(&u64::from(monster.uid)) {
                    problems.push(format!(
                        "{at}: engine-only live monster {:?} uid {}",
                        monster.kind, monster.uid
                    ));
                }
            }
        }
        // One divergence fans out over the rest of its case; keep the first few.
        problems.truncate(before_case + 4);
    }

    // A native move must have one engine name, and one engine name one native
    // move, per monster type: the next-move comparison is this bijection.
    let mut inverse: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for ((kind, native), engine_names) in &names {
        if engine_names.len() != 1 {
            problems.push(format!("{kind} {native}: engine names {engine_names:?}"));
        }
        for engine_name in engine_names {
            inverse
                .entry((kind.clone(), engine_name.clone()))
                .or_default()
                .insert(native.clone());
        }
    }
    for ((kind, engine_name), natives) in &inverse {
        if natives.len() != 1 {
            problems.push(format!(
                "{kind} engine {engine_name}: native moves {natives:?}"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "{} divergences from the native traces:\n{}",
        problems.len(),
        problems.join("\n")
    );
    assert_eq!(
        (
            cases.len(),
            kinds.len(),
            turns_compared,
            stuns_installed,
            plays,
            player_powers_compared,
            move_logs_compared
        ),
        (621, 79, 5336, 798, 367, 1977, 2770),
        "cases, kinds, enemy turns compared, stuns installed, cards played, player powers compared, move logs compared"
    );

    // Which generic stun owners have no native trace here. The four Fabricator
    // bots are spawned by a Fabricator, whose own machine still refuses the
    // stun (a conditional branch), so no admitted root holds them; their
    // witness is the row sweep in `turn.rs` and the IL of their machines.
    let untraced: Vec<String> = crate::ids::MonsterKind::ALL
        .into_iter()
        .filter(|kind| super::monsters::generic_stun_owner_loop_is_fully_resolvable(*kind))
        .map(|kind| format!("{kind:?}"))
        .filter(|kind| !kinds.contains(kind))
        .collect();
    assert_eq!(untraced, ["Guardbot", "Noisebot", "Stabbot", "Zapbot"]);
}
