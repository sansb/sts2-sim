//! Witnesses for the run-history counter prediction.
//!
//! * `fixtures/run_counters_v1.json` holds the Python oracle's document for
//!   every fight of every case. Its generator, `tools/gen_run_counters_pins.py`,
//!   was deleted with the Python simulator (#2827 item F); the fixture is frozen
//!   data, sha-pinned by `tools/frozen_oracle_data.py`. Rust must reproduce
//!   each document byte for byte, as JSON values.
//! * The registries and data tables are pinned to their Python sources, frozen
//!   as crate data since #2999 (item F deletes `solve_fight.py`).
//! * Every refusal has one mutation that reaches it.

use serde_json::{Value, json};

use super::history::RunHistory;
use super::{RunCountersRefusal, predict, predict_history};
use crate::catalog::GameBuild;

const BUILD: GameBuild = GameBuild::V0_111_0;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/run_counters_v1.json")).unwrap()
}

fn case(name: &str) -> Value {
    fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no fixture case {name}"))
        .clone()
}

fn answer(history: &Value, fight: usize) -> Result<Value, RunCountersRefusal> {
    predict(&history.to_string(), BUILD, fight).map(|p| p.to_json(BUILD))
}

fn refusal(history: &Value, fight: usize) -> RunCountersRefusal {
    answer(history, fight).expect_err("expected a refusal")
}

#[test]
fn every_fixture_fight_matches_the_python_oracle_document() {
    let fixture = fixture();
    let mut fights = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let history = RunHistory::from_value(case["history"].clone(), BUILD).unwrap();
        for (index, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            let got = predict_history(&history, index).unwrap().to_json(BUILD);
            assert_eq!(&got, expected, "{} fight {index}", case["name"]);
            fights += 1;
        }
    }
    // One real current-build run plus the synthetic branch cases.
    assert!(fights >= 25, "{fights}");
}

#[test]
fn the_fixture_reaches_every_status_and_every_caveat_family() {
    let text = include_str!("../../fixtures/run_counters_v1.json");
    for status in ["exact", "baseline", "unknown", "assumed", "not_predicted"] {
        assert!(
            text.contains(&format!("\"status\": \"{status}\"")),
            "{status}"
        );
    }
    for needle in [
        // replay_fight.predict
        "can leave/alter the cycle unrecorded",
        "(not in cards_census)",
        "is an event-node combat",
        // shuffle_counter_caveats
        "add cards mid-combat",
        "modeled draw/cycle relics",
        "modeled pile-changing potions used",
        "modeled draw-cycle sources",
        "ENCHANTMENT.IMBUED",
        "Forge sources",
        "BIIIG_HUG generated one Soot",
        // niche / monsterai
        "BLOAT spawned GasBombs",
        "rats may have summoned",
        "4 Wrigglers spawned",
        "Stock may have replaced Axebot",
        "Niche-consuming on-obtain relics owned: {'RELIC.ASTROLABE', \
         'RELIC.BEAUTIFUL_BRACELET'}",
        "Beautiful Bracelet shuffles",
        "consumed MonsterAi draws",
        // card selection
        "STONE_CRACKER owned but CARD.NOT_A_CARD upgradability",
        "STONE_CRACKER owned but CARD.STRIKE_IRONCLAD upgradability",
        "deck held CombatCardSelection consumers (ANOINTED, TRUE_GRIT)",
        "a generated card could be a CombatCardSelection consumer",
        "relics ['RELIC.BOOKMARK'] consume CombatCardSelection",
        // targets
        "deck held CombatTargets consumers",
        "targeted IMBUED cards (STRIKE_IRONCLAD)",
        "consume CombatTargets draws per unrecorded in-fight events",
        "a generated card could be a CombatTargets consumer",
        "POTION.DISTILLED_CHAOS auto-played null-targeted cards",
        // energy costs
        "Slither cards (BASH)",
        "relics ['RELIC.FAKE_SNECKO_EYE', 'RELIC.SNECKO_EYE'] applied ConfusedPower",
        "used potions ['POTION.SNECKO_OIL']",
        // generation, orbs
        "only fight 0 is inferred exactly",
        "deck held CombatOrbGeneration consumers (CHAOS, TRASH_TO_TREASURE)",
        "could consume CombatOrbGeneration",
    ] {
        assert!(text.contains(needle), "no fixture reaches {needle:?}");
    }
    // TRUE_GRIT+ is not a selection consumer; the level-0 copy alone names it,
    // and DEFEND_IRONCLAD (self-targeted) is not a targeted Imbued card.
    assert!(!text.contains("targeted IMBUED cards (DEFEND_IRONCLAD"));
}

/// `RunRngSet::.ctor` (`0x4de0c`) seeds this build with XxHash64; the
/// build-less `replay_fight.predict` replayed the v0.108 djb2 seeding and,
/// on this deck, answered 20 where the build's own stream answers 19.
#[test]
fn the_shuffle_replay_uses_this_builds_seeding_not_the_v108_default() {
    let history = case("seeded_by_build")["history"].clone();
    let document = answer(&history, 1).unwrap();
    assert_eq!(document["counters"]["shuffle"]["value"], 19);
    assert_eq!(document["counters"]["shuffle"]["status"], "exact");
    let v108_answer = 20;
    assert_ne!(document["counters"]["shuffle"]["value"], v108_answer);
}

#[test]
fn a_stone_cracker_deck_of_known_cards_counts_upgradable_minus_one() {
    // Four upgradable cards (three Strikes and a Defend; Bash+ is maxed).
    let document = answer(&case("clean_prefix")["history"], 1).unwrap();
    assert_eq!(document["counters"]["combat_card_selection"]["value"], 3);
    assert_eq!(
        document["counters"]["combat_card_selection"]["status"],
        "exact"
    );
}

// The data tables and registries below were pinned to their Python originals
// (`solver/cards_census.json`, `solver/card_templates.json`, and the literal
// registries in `solver/solve_fight.py`) by reading those files at compile
// time. #2827 item F deletes `solve_fight.py`, and a crate test that
// `include_str!`s a deleted file stops the whole test binary compiling
// (#2999). So the Python half is frozen data now: `data/cards_census.json`
// and `data/card_max_upgrade.json` were byte-verified against the solver's
// copies, and `fixtures/frozen_python_run_counter_registries_v1.json` holds
// the registries as the old test's own parser read them from `solve_fight.py`
// (its sha256 is recorded in the fixture). `tools/frozen_oracle_data.py`
// pins all three, so they change only by a deliberate pin refresh. The
// guard `tests/no_python_source_reach.rs` keeps a new `include_str!` of a
// Python source from coming back.

/// The frozen registries document.
fn frozen_registries() -> Value {
    serde_json::from_str(include_str!(
        "../../fixtures/frozen_python_run_counter_registries_v1.json"
    ))
    .unwrap()
}

#[test]
fn the_cards_census_is_the_frozen_solver_copy() {
    // Byte-identical to `solver/cards_census.json` when frozen (#2999); the
    // sha256 pin in `frozen_oracle_data.py` holds it there. Parse it here so
    // a truncated or hand-mangled copy fails in the crate too.
    let census: Value = serde_json::from_str(include_str!("../../data/cards_census.json")).unwrap();
    assert!(census.is_object(), "cards_census.json is an object");
}

#[test]
fn the_max_upgrade_table_is_the_frozen_card_templates_table() {
    let copied: Value =
        serde_json::from_str(include_str!("../../data/card_max_upgrade.json")).unwrap();
    let source = copied.as_object().unwrap();
    assert!(!source.is_empty());
    for (id, level) in source {
        assert_eq!(
            super::streams::registries::max_upgrade(id.trim_start_matches("CARD.")),
            level.as_i64(),
            "{id}"
        );
    }
    // The whole-DLL table, not the modeled rows: one row, maximum 1.
    assert_eq!(
        super::streams::registries::max_upgrade("CALCULATED_GAMBLE"),
        Some(1)
    );
    assert_eq!(super::streams::registries::max_upgrade("NOT_A_CARD"), None);
}

#[test]
fn every_registry_is_its_frozen_solve_fight_original() {
    let frozen = frozen_registries();
    let python = frozen["registries"].as_object().unwrap();
    assert_eq!(
        python.len(),
        super::streams::registries::ALL.len(),
        "one frozen registry per transcribed one"
    );
    for (name, rust) in super::streams::registries::ALL {
        let mut python: Vec<String> = python
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not in the frozen registries"))
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let mut rust: Vec<String> = rust.iter().map(|s| (*s).to_owned()).collect();
        assert!(!python.is_empty(), "{name}");
        if name == "MONSTERAI_CONSUMERS" {
            // A tuple: order is part of nothing, but keep it identical anyway.
            assert_eq!(rust, python, "{name}");
        }
        python.sort();
        rust.sort();
        assert_eq!(rust, python, "{name}");
    }
}

#[test]
fn the_niche_spawner_registry_is_its_frozen_solve_fight_original() {
    let frozen = frozen_registries();
    // `NICHE_MIDFIGHT_SPAWNERS` in the dict's insertion order, as
    // `[monster id, fewest turns before a creation]`.
    let python: Vec<(String, i64)> = frozen["niche_midfight_spawners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row[0].as_str().unwrap().to_owned(),
                row[1].as_i64().unwrap(),
            )
        })
        .collect();
    let rust: Vec<(String, i64)> = super::streams::registries::NICHE_SPAWNERS
        .iter()
        .map(|(mid, min_turns, _)| ((*mid).to_owned(), *min_turns))
        .collect();
    assert_eq!(rust, python);
}

// ---------------------------------------------------------------------------
// Refusals: one mutation each.
// ---------------------------------------------------------------------------

fn base() -> Value {
    case("shuffle_model")["history"].clone()
}

#[test]
fn malformed_json_refuses() {
    assert_eq!(predict("{", BUILD, 0).unwrap_err().code(), "malformed_json");
}

#[test]
fn a_wrong_schema_refuses() {
    let mut history = base();
    history["schema"] = json!("sts-sim-entry-v1");
    assert_eq!(refusal(&history, 0).code(), "history_schema_mismatch");
}

#[test]
fn a_run_from_another_build_refuses() {
    let mut history = base();
    history["run"]["build_id"] = json!("v0.110.1");
    assert!(matches!(
        refusal(&history, 0),
        RunCountersRefusal::RunBuildMismatch { recorded: Some(ref b), .. } if b == "v0.110.1"
    ));
}

#[test]
fn a_multiplayer_run_refuses() {
    let mut history = base();
    let player = history["run"]["players"][0].clone();
    history["run"]["players"] = json!([player.clone(), player]);
    assert_eq!(
        refusal(&history, 0),
        RunCountersRefusal::MultiplayerRun("2".to_owned())
    );
    history["run"].as_object_mut().unwrap().remove("players");
    assert_eq!(
        refusal(&history, 0),
        RunCountersRefusal::MultiplayerRun("missing".to_owned())
    );
}

#[test]
fn untaught_and_mistyped_fields_refuse_by_path() {
    type Mutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Mutation)> = vec![
        ("$.extra", Box::new(|h| h["extra"] = json!(1))),
        (
            "$.fights[0].hp_entering",
            Box::new(|h| h["fights"][0]["hp_entering"] = json!(1)),
        ),
        (
            "$.fights[0].turns_taken",
            Box::new(|h| h["fights"][0]["turns_taken"] = json!(null)),
        ),
        (
            "$.fights[0].monster_ids[0]",
            Box::new(|h| h["fights"][0]["monster_ids"][0] = json!(3)),
        ),
        (
            "$.fights[0].deck_entering[0].upgrade_level",
            Box::new(|h| h["fights"][0]["deck_entering"][0]["upgrade_level"] = json!(true)),
        ),
        (
            "$.fights[0].deck_entering[0].enchantment",
            Box::new(|h| h["fights"][0]["deck_entering"][0]["enchantment"] = json!(["IMBUED"])),
        ),
        (
            "$.fights[0].deck_entering[0].upgrade_ambiguous",
            Box::new(|h| h["fights"][0]["deck_entering"][0]["upgrade_ambiguous"] = json!(1)),
        ),
        (
            "$.fights[0].deck_entering[0].props",
            Box::new(|h| h["fights"][0]["deck_entering"][0]["props"] = json!({})),
        ),
        ("$.fights", Box::new(|h| h["fights"] = json!({}))),
        ("$.run.seed", Box::new(|h| h["run"]["seed"] = json!(7))),
        (
            "$.run.players[0].deck[0].floor_added_to_deck",
            Box::new(|h| {
                h["run"]["players"][0]["deck"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("floor_added_to_deck");
            }),
        ),
        (
            "$.run.nodes[1].player_stats[0]",
            Box::new(|h| h["run"]["map_point_history"][0][1]["player_stats"] = json!([])),
        ),
        (
            "$.run.nodes[1].cards_removed",
            Box::new(|h| {
                h["run"]["map_point_history"][0][1]["player_stats"][0]["cards_removed"] =
                    json!(null)
            }),
        ),
        (
            "$.run.nodes[3].cards_transformed[0].original_card",
            Box::new(|h| {
                h["run"]["map_point_history"][0][3]["player_stats"][0]["cards_transformed"][0] =
                    json!({"final_card": {}});
            }),
        ),
        (
            "$.run.nodes[1].turns_taken",
            Box::new(|h| {
                h["run"]["map_point_history"][0][1]["rooms"][0]["turns_taken"] = json!("4")
            }),
        ),
        (
            "$.run.nodes[1].model_id",
            Box::new(|h| h["run"]["map_point_history"][0][1]["rooms"][0]["model_id"] = json!(null)),
        ),
    ];
    for (path, mutate) in cases {
        let mut history = base();
        mutate(&mut history);
        match refusal(&history, 1) {
            RunCountersRefusal::MalformedHistory { path: got, .. } => assert_eq!(got, path),
            other => panic!("{path}: {other:?}"),
        }
    }
}

#[test]
fn an_out_of_range_fight_refuses() {
    assert_eq!(
        refusal(&base(), 5),
        RunCountersRefusal::FightIndexOutOfRange {
            index: 5,
            fights: 5
        }
    );
}

#[test]
fn parsed_fights_on_other_nodes_than_the_run_refuse() {
    let mut history = base();
    history["fights"][1]["node_index"] = json!(3);
    assert_eq!(
        refusal(&history, 1),
        RunCountersRefusal::CombatEnumerationDisagrees {
            fight: 1,
            run_node: Some(2),
            parsed_node: 3
        }
    );
    // A parsed fight the raw run never lists.
    let mut history = base();
    history["run"]["map_point_history"][0]
        .as_array_mut()
        .unwrap()
        .truncate(6);
    assert_eq!(
        refusal(&history, 4),
        RunCountersRefusal::CombatEnumerationDisagrees {
            fight: 4,
            run_node: None,
            parsed_node: 6
        }
    );
}

#[test]
fn a_prior_fight_without_turns_refuses_but_the_target_does_not_need_them() {
    for turns in [json!(-1), json!(null)] {
        let mut history = base();
        history["run"]["map_point_history"][0][1]["rooms"][0]["turns_taken"] = turns.clone();
        assert_eq!(
            refusal(&history, 1),
            RunCountersRefusal::PriorFightTurnsUnknown { fight: 0 },
            "{turns}"
        );
        assert!(answer(&history, 0).is_ok(), "{turns}");
    }
    // A combat node with no combat room lists as turns -1.
    let mut history = base();
    history["run"]["map_point_history"][0][1]["rooms"] = json!([]);
    assert_eq!(
        refusal(&history, 1),
        RunCountersRefusal::PriorFightTurnsUnknown { fight: 0 }
    );
}

#[test]
fn a_floorless_removal_row_refuses_only_when_a_later_removal_compares_against_it() {
    // The shop's removal loses its floor: it re-inserts as floor 1 (the
    // default), and the transformed Bash logged after it then has to compare
    // `(id, floor)` against it, which `entry_deck` raised `KeyError` on.
    let mut history = base();
    history["run"]["map_point_history"][0][3]["player_stats"][0]["cards_removed"][0]
        .as_object_mut()
        .unwrap()
        .remove("floor_added_to_deck");
    assert_eq!(
        refusal(&history, 0),
        RunCountersRefusal::RemovalRowWithoutFloor { node: 3 }
    );
    // The transformed Bash is the node's last removal: nothing scans it.
    let mut history = base();
    history["run"]["map_point_history"][0][3]["player_stats"][0]["cards_transformed"][0]
        ["original_card"]
        .as_object_mut()
        .unwrap()
        .remove("floor_added_to_deck");
    assert!(answer(&history, 1).is_ok());
}

#[test]
fn a_removal_row_without_an_id_refuses() {
    let mut history = base();
    history["run"]["map_point_history"][0][1]["player_stats"][0]["cards_removed"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert!(matches!(
        refusal(&history, 0),
        RunCountersRefusal::MalformedHistory { .. }
    ));
}

#[test]
fn refusal_display_carries_the_code() {
    let text = RunCountersRefusal::PriorFightTurnsUnknown { fight: 2 }.to_string();
    assert!(
        text.starts_with("refusal: run-counters: prior_fight_turns_unknown: "),
        "{text}"
    );
}

#[test]
fn entry_deck_reinserts_removals_at_sibling_bane_and_floor_slots() {
    let run = base()["run"].clone();
    // Fight at node 1 (floor 2): the Defend removed during it is still
    // present, after its last same-(id, floor) sibling; the shop's two
    // floor-1 removals (Perfected Strike, the transformed Bash) take the
    // Ascender's Bane slot, in log order (each insert pushes the Bane on).
    let ids = super::shuffle::test_support::entry_deck_ids(&run, 2);
    let short: Vec<&str> = ids.iter().map(|s| s.trim_start_matches("CARD.")).collect();
    assert_eq!(
        short,
        [
            "STRIKE_IRONCLAD",
            "STRIKE_IRONCLAD",
            "STRIKE_IRONCLAD",
            "STRIKE_IRONCLAD",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
            "PERFECTED_STRIKE",
            "BASH",
            "ASCENDERS_BANE",
        ]
    );
    // Fight at node 4 (floor 5): the node-1 removal is gone, the node-4
    // floor-3 Pommel Strike lands after the last card of floor <= 3.
    let ids = super::shuffle::test_support::entry_deck_ids(&run, 5);
    let pommel = ids.iter().position(|c| c == "CARD.POMMEL_STRIKE").unwrap();
    assert_eq!(ids[pommel - 1], "CARD.INFLAME");
    assert!(!ids.contains(&"CARD.BASH".to_owned()));
    assert_eq!(
        ids.iter().filter(|c| *c == "CARD.DEFEND_IRONCLAD").count(),
        4
    );
}

#[test]
fn cycle_leavers_follow_the_census_and_skip_the_bane() {
    use super::shuffle::test_support::cycle_leavers;
    assert_eq!(
        cycle_leavers(&[
            "CARD.ASCENDERS_BANE",
            "CARD.STRIKE_IRONCLAD",
            "CARD.INFLAME",
            "CARD.INFLAME",
            "CARD.NOT_IN_CENSUS",
        ]),
        ["CARD.INFLAME", "CARD.NOT_IN_CENSUS (not in cards_census)"]
    );
}

#[test]
fn the_cli_answers_what_the_library_answers_and_refuses_argv_by_name() {
    use super::cli::{RunCountersCliRefusal, main};
    let argv = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let dir = std::env::temp_dir().join(format!("run-counters-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.json");
    let history = base();
    std::fs::write(&path, history.to_string()).unwrap();
    let path = path.to_str().unwrap();
    let out = main(&argv(&[
        "--build",
        "v0.111.0",
        "--history",
        path,
        "--fight",
        "3",
    ]))
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap(),
        answer(&history, 3).unwrap()
    );

    assert_eq!(
        main(&argv(&["--history", path, "--fight", "0"])),
        Err(RunCountersCliRefusal::UnadmittedBuild(None))
    );
    assert_eq!(
        main(&argv(&[
            "--build",
            "v0.110.1",
            "--history",
            path,
            "--fight",
            "0"
        ])),
        Err(RunCountersCliRefusal::UnadmittedBuild(Some(
            "v0.110.1".to_owned()
        )))
    );
    assert_eq!(
        main(&argv(&["--build", "v0.111.0", "--fight", "0"])),
        Err(RunCountersCliRefusal::MissingFlag("--history"))
    );
    assert_eq!(
        main(&argv(&["--build", "v0.111.0", "--history", path])),
        Err(RunCountersCliRefusal::MissingFlag("--fight"))
    );
    assert_eq!(
        main(&argv(&[
            "--build",
            "v0.111.0",
            "--history",
            path,
            "--fight",
            "-1"
        ])),
        Err(RunCountersCliRefusal::BadFightIndex("-1".to_owned()))
    );
    assert_eq!(
        main(&argv(&["--build", "v0.111.0", "--build", "v0.111.0"])),
        Err(RunCountersCliRefusal::RepeatedFlag("--build".to_owned()))
    );
    assert_eq!(
        main(&argv(&["--build"])),
        Err(RunCountersCliRefusal::MissingValue("--build".to_owned()))
    );
    assert_eq!(
        main(&argv(&["--save", path])),
        Err(RunCountersCliRefusal::UnknownFlag("--save".to_owned()))
    );
    assert!(matches!(
        main(&argv(&[
            "--build",
            "v0.111.0",
            "--history",
            "/nonexistent",
            "--fight",
            "0"
        ])),
        Err(RunCountersCliRefusal::Unreadable { .. })
    ));
    assert!(matches!(
        main(&argv(&[
            "--build",
            "v0.111.0",
            "--history",
            path,
            "--fight",
            "9"
        ])),
        Err(RunCountersCliRefusal::Prediction(
            RunCountersRefusal::FightIndexOutOfRange { .. }
        ))
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `MonsterAi` is exact 0 entering the run's first combat and only `assumed`
/// after it (`monster_ai_entering`): the stream's only accessor is called
/// from combat code alone.
#[test]
fn monster_ai_is_exact_only_entering_the_first_combat() {
    let history = case("6P96T755CNZ3")["history"].clone();
    let first = answer(&history, 0).unwrap();
    assert_eq!(first["counters"]["monster_ai"]["status"], json!("exact"));
    assert_eq!(first["counters"]["monster_ai"]["value"], json!(0));
    for fight in 1..4 {
        let later = answer(&history, fight).unwrap();
        assert_eq!(
            later["counters"]["monster_ai"]["status"],
            json!("assumed"),
            "fight {fight}"
        );
        assert_eq!(later["counters"]["monster_ai"]["value"], json!(0));
    }
}
