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
// (`python/cards_census.json`, `python/card_templates.json`, and the literal
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
    // Byte-identical to `python/cards_census.json` when frozen (#2999); the
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
        // The frozen file stays what `solve_fight.py` said. A row the port
        // has added since is named here, so each departure is deliberate.
        for (registry, added) in ADDED_SINCE_THE_FREEZE {
            if *registry == name {
                python.extend(added.iter().map(|s| (*s).to_owned()));
            }
        }
        if name == "MONSTERAI_CONSUMERS" {
            // A tuple: order is part of nothing, but keep it identical anyway.
            assert_eq!(rust, python, "{name}");
        }
        python.sort();
        rust.sort();
        assert_eq!(rust, python, "{name}");
    }
}

/// Rows the Rust registries hold that the frozen `solve_fight.py` registries
/// did not. #2997: the whole-DLL `Niche` census found Distinguished Cape's
/// `AfterObtained` draw, which the Python registry had missed.
const ADDED_SINCE_THE_FREEZE: &[(&str, &[&str])] =
    &[("NICHE_ONOBTAIN_RELICS", &["RELIC.DISTINGUISHED_CAPE"])];

/// Spawners the census added after the frozen four (#2997), each at the
/// conservative threshold 0.
const SPAWNERS_ADDED_SINCE_THE_FREEZE: [&str; 4] =
    ["OVICOPTER", "FOGMOG", "THE_OBSCURA", "FABRICATOR"];

#[test]
fn the_niche_spawner_registry_extends_its_frozen_solve_fight_original() {
    let frozen = frozen_registries();
    // `NICHE_MIDFIGHT_SPAWNERS` in the dict's insertion order, as
    // `[monster id, fewest turns before a creation]`.
    let mut expected: Vec<(String, i64)> = frozen["niche_midfight_spawners"]
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
    assert_eq!(expected.len(), 4, "the frozen original");
    expected.extend(
        SPAWNERS_ADDED_SINCE_THE_FREEZE
            .iter()
            .map(|mid| ((*mid).to_owned(), 0)),
    );
    let rust: Vec<(String, i64)> = super::streams::registries::NICHE_SPAWNERS
        .iter()
        .map(|(mid, min_turns, _)| ((*mid).to_owned(), *min_turns))
        .collect();
    assert_eq!(rust, expected);
}

// ---------------------------------------------------------------------------
// #2997: the whole-DLL Niche census, and what each consumer does to a claim.
// ---------------------------------------------------------------------------

/// `tools/niche_consumer_census.py`'s scan of the archived DLL.
fn niche_census() -> Value {
    serde_json::from_str(include_str!("../../fixtures/niche_consumer_census_v1.json")).unwrap()
}

/// The census keys of one class, sorted and deduplicated.
fn census_keys(census: &Value, class: &str) -> Vec<String> {
    let mut keys: Vec<String> = census["sites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|site| site["class"] == class)
        .map(|site| site["key"].as_str().unwrap().to_owned())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn sorted(items: impl IntoIterator<Item = impl ToString>) -> Vec<String> {
    let mut out: Vec<String> = items.into_iter().map(|s| s.to_string()).collect();
    out.sort();
    out
}

#[test]
fn the_niche_registries_cover_the_whole_dll_census() {
    use super::streams::registries;
    let census = niche_census();
    let sites = census["sites"].as_array().unwrap();

    // Every site has a class this test knows what to do with. A new build's
    // extra caller is unclassified in the tool, and a new class is refused
    // here.
    let known = [
        "setup",
        "midfight",
        "midfight_recorded",
        "layout_event",
        "relic",
        "modifier",
        "player_side",
        "test_only",
    ];
    for site in sites {
        let class = site["class"].as_str().unwrap();
        assert!(known.contains(&class), "unknown census class {class}");
    }
    // The known-positive control from the issue: Ovicopter's egg laying.
    assert!(
        sites.iter().any(|site| {
            site["type"] == "Ovicopter/<LayEggsMove>d__26"
                && site["rva"] == "0x3651e8"
                && site["il"] == "IL_00e7"
                && site["class"] == "midfight"
                && site["key"] == "OVICOPTER"
        }),
        "the census lost its known-positive control"
    );
    // Exactly one site is encounter setup, the roster `monster_ids` counts.
    let setup: Vec<&Value> = sites.iter().filter(|s| s["class"] == "setup").collect();
    assert_eq!(setup.len(), 1);
    assert_eq!(setup[0]["type"], "CombatRoom/<StartCombat>d__46");

    // Mid-fight spawners: the registry is exactly the census.
    assert_eq!(
        sorted(registries::NICHE_SPAWNERS.iter().map(|(mid, _, _)| *mid)),
        census_keys(&census, "midfight")
    );
    // The one mid-fight creator the roster counts exactly has no row (the
    // doc comment on the registry carries the IL): both of its sites are the
    // Gremlin Merc's death.
    assert_eq!(census_keys(&census, "midfight_recorded"), ["GREMLIN_MERC"]);
    assert_eq!(
        sites
            .iter()
            .filter(|s| s["class"] == "midfight_recorded")
            .map(|s| (s["type"].as_str().unwrap(), s["il"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [
            ("SurprisePower/<AfterDeath>d__4", "IL_0063"),
            ("SurprisePower/<AfterDeath>d__4", "IL_0198"),
        ]
    );
    // Relics: every census relic is registered. The registry's one extra row
    // is the frozen original's Sere Talon, which has no site in this build.
    let mut relics = census_keys(&census, "relic");
    assert_eq!(relics.len(), 15);
    relics.push("RELIC.SERE_TALON".to_owned());
    assert_eq!(sorted(registries::NICHE_ONOBTAIN_RELICS), sorted(relics));
    // Modifiers and combat-layout events.
    assert_eq!(
        sorted(registries::NICHE_MODIFIERS),
        census_keys(&census, "modifier")
    );
    assert_eq!(
        sites
            .iter()
            .filter(|s| s["class"] == "layout_event")
            .count(),
        1
    );
    assert_eq!(
        sorted(registries::NICHE_COMBAT_LAYOUT_EVENTS),
        sorted(
            census["combat_layout_events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
        )
    );
    // The spawning powers are keyed by the one monster that applies each.
    for (power, owner) in [
        ("InfestedPower", "PhrogParasite/"),
        ("StockPower", "Axebot/"),
        ("SurprisePower", "GremlinMerc/"),
    ] {
        let rows = census["spawning_power_appliers"][power].as_array().unwrap();
        assert!(!rows.is_empty(), "{power}");
        for row in rows {
            assert!(row["type"].as_str().unwrap().starts_with(owner), "{power}");
        }
    }
}

/// `clean_prefix` with its first fight replaced: two Nibbits become `monsters`.
fn after_a_fight_against(monsters: &[&str], turns: i64) -> Value {
    let mut history = case("clean_prefix")["history"].clone();
    history["fights"][0]["monster_ids"] = json!(monsters);
    history["fights"][0]["turns_taken"] = json!(turns);
    history["run"]["map_point_history"][0][1]["rooms"][0]["monster_ids"] = json!(monsters);
    history["run"]["map_point_history"][0][1]["rooms"][0]["turns_taken"] = json!(turns);
    history
}

fn niche(history: &Value, fight: usize) -> Value {
    answer(history, fight).unwrap()["counters"]["niche"].clone()
}

#[test]
fn a_fight_with_no_niche_consumer_keeps_the_counter_exact() {
    let history = case("clean_prefix")["history"].clone();
    let got = niche(&history, 1);
    assert_eq!(got["value"], 2);
    assert_eq!(got["status"], "exact");
}

#[test]
fn an_ovicopter_fight_makes_every_later_niche_counter_a_baseline() {
    // Run 1787331639, fight 9: the roster records the egg id once, the
    // captured saves show seven draws for it. The count is unrecoverable.
    let history = after_a_fight_against(&["MONSTER.OVICOPTER", "MONSTER.TOUGH_EGG"], 5);
    let got = niche(&history, 1);
    assert_eq!(got["value"], 2);
    assert_eq!(got["status"], "baseline");
    let caveats = got["caveats"].as_array().unwrap();
    assert_eq!(caveats.len(), 1);
    assert!(
        caveats[0]
            .as_str()
            .unwrap()
            .starts_with("fight 0 (ENCOUNTER.NIBBITS_WEAK): Ovicopter may have laid Tough Eggs"),
        "{caveats:?}"
    );
    // Even an Ovicopter killed before it acted: the threshold is 0.
    let history = after_a_fight_against(&["MONSTER.OVICOPTER"], 0);
    assert_eq!(niche(&history, 1)["status"], "baseline");
    // The fight's own entering counter is untouched by what happens in it.
    assert_eq!(niche(&history, 0)["status"], "exact");
}

#[test]
fn every_spawner_the_census_added_makes_the_niche_counter_a_baseline() {
    for (monster, needle) in [
        ("MONSTER.FOGMOG", "Fogmog may have summoned Eyes With Teeth"),
        (
            "MONSTER.THE_OBSCURA",
            "The Obscura may have summoned Parafrights",
        ),
        ("MONSTER.FABRICATOR", "Fabricator may have built bots"),
    ] {
        let history = after_a_fight_against(&[monster], 0);
        let got = niche(&history, 1);
        assert_eq!(got["value"], 1, "{monster}");
        assert_eq!(got["status"], "baseline", "{monster}");
        assert!(
            got["caveats"][0].as_str().unwrap().contains(needle),
            "{monster}: {got}"
        );
    }
}

#[test]
fn a_gremlin_merc_fight_is_counted_exactly_from_its_roster() {
    // The Merc's death creates one Fat and one Sneaky Gremlin, both recorded.
    let history = after_a_fight_against(
        &[
            "MONSTER.GREMLIN_MERC",
            "MONSTER.FAT_GREMLIN",
            "MONSTER.SNEAKY_GREMLIN",
        ],
        4,
    );
    let got = niche(&history, 1);
    assert_eq!(got["value"], 3);
    assert_eq!(got["status"], "exact");
}

#[test]
fn an_unfought_combat_layout_event_makes_the_niche_counter_a_baseline() {
    let event = |id: &str| json!({"model_id": id, "room_type": "event", "turns_taken": 0});
    for id in [
        "EVENT.PUNCH_OFF",
        "EVENT.THE_ARCHITECT",
        "EVENT.THE_LANTERN_KEY",
    ] {
        // Entered at node 0 and not fought there.
        let mut history = case("clean_prefix")["history"].clone();
        history["run"]["map_point_history"][0][0]["rooms"] = json!([event(id)]);
        let got = niche(&history, 1);
        assert_eq!(got["value"], 2, "{id}");
        assert_eq!(got["status"], "baseline", "{id}");
        assert!(
            got["caveats"][0]
                .as_str()
                .unwrap()
                .starts_with(&format!("node 0 ({id}): a combat-layout event")),
            "{got}"
        );
        // Fought: the node is a recorded fight, so its roster is counted.
        let mut history = case("clean_prefix")["history"].clone();
        history["run"]["map_point_history"][0][1]["rooms"]
            .as_array_mut()
            .unwrap()
            .insert(0, event(id));
        assert_eq!(niche(&history, 1)["status"], "exact", "{id}");
        // At the target's own node: not a prior node.
        let mut history = case("clean_prefix")["history"].clone();
        history["run"]["map_point_history"][0][2]["rooms"]
            .as_array_mut()
            .unwrap()
            .insert(0, event(id));
        assert_eq!(niche(&history, 1)["status"], "exact", "{id}");
    }
    // Any other event is not a consumer.
    let mut history = case("clean_prefix")["history"].clone();
    history["run"]["map_point_history"][0][0]["rooms"] = json!([event("EVENT.FAKE_MERCHANT")]);
    assert_eq!(niche(&history, 1)["status"], "exact");
}

#[test]
fn the_cursed_run_modifier_makes_every_niche_counter_a_baseline() {
    let mut history = case("clean_prefix")["history"].clone();
    history["run"]["modifiers"] =
        json!([{"id": "MODIFIER.HOARDER"}, {"id": "MODIFIER.CURSED_RUN"}]);
    for fight in [0, 1] {
        let got = niche(&history, fight);
        assert_eq!(got["status"], "baseline", "fight {fight}");
        assert!(
            got["caveats"][0]
                .as_str()
                .unwrap()
                .starts_with("MODIFIER.CURSED_RUN draws Niche on every act entry"),
            "{got}"
        );
    }
    history["run"]["modifiers"] = json!([{"id": "MODIFIER.HOARDER"}]);
    assert_eq!(niche(&history, 1)["status"], "exact");
}

#[test]
fn distinguished_cape_is_a_niche_consuming_relic() {
    let mut history = case("clean_prefix")["history"].clone();
    history["fights"][1]["relics_entering"] = json!(["RELIC.DISTINGUISHED_CAPE"]);
    let got = niche(&history, 1);
    assert_eq!(got["status"], "baseline");
    assert_eq!(
        got["caveats"],
        json!(["Niche-consuming on-obtain relics owned: {'RELIC.DISTINGUISHED_CAPE'}"])
    );
}

// ---------------------------------------------------------------------------
// #2997: a floor-1 removal row that was gained at the first node.
// ---------------------------------------------------------------------------

fn first_node_gain_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../fixtures/issue2997_first_node_gain_removal.json"
    ))
    .unwrap()
}

#[test]
fn a_stolen_neow_card_is_rebuilt_after_the_bane_and_the_shuffle_counter_matches_the_saves() {
    let fixture = first_node_gain_fixture();
    let history = &fixture["history"];
    // The rebuilt entry deck is the captured start save's, card for card.
    let saved: Vec<String> = fixture["captured_start_save_decks"]["0"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        super::shuffle::test_support::entry_deck_ids(&history["run"], 2),
        saved
    );
    assert!(super::shuffle::test_support::entry_deck_unplaceable(&history["run"], 2).is_empty());
    // And every counter the saves recorded is predicted. The first three are
    // exact claims; before the fix fights 1 and 2 claimed exact 20 and 33.
    for (fight, status) in [(0, "exact"), (1, "exact"), (2, "exact"), (3, "baseline")] {
        let got = answer(history, fight).unwrap()["counters"]["shuffle"].clone();
        assert_eq!(
            got["value"],
            fixture["captured_shuffle_counters"][fight.to_string()],
            "fight {fight}"
        );
        assert_eq!(got["status"], status, "fight {fight}");
    }
    // Without the first node's record of the gain, the row is taken for a
    // starter card and lands in the Bane's slot: the old answer.
    let mut blind = history.clone();
    blind["run"]["map_point_history"][0][0]["player_stats"][0]
        .as_object_mut()
        .unwrap()
        .remove("cards_gained");
    let ids = super::shuffle::test_support::entry_deck_ids(&blind["run"], 2);
    let at = |id: &str| ids.iter().position(|c| c == id).unwrap();
    assert!(at("CARD.SUNDER") < at("CARD.ASCENDERS_BANE"));
    assert_eq!(
        answer(&blind, 1).unwrap()["counters"]["shuffle"]["value"],
        20
    );
}

/// A run whose first node is `first_node` and whose node 3 logs `removal`,
/// with two fights before it. The deck holds the Bane between `before` and
/// `after`, all floor 1.
fn bane_history(before: &[&str], after: &[&str], first_node: Value, removal: Value) -> Value {
    let card = |id: &str| json!({"id": format!("CARD.{id}"), "floor_added_to_deck": 1});
    let deck: Vec<Value> = before
        .iter()
        .copied()
        .chain(["ASCENDERS_BANE"])
        .chain(after.iter().copied())
        .map(card)
        .collect();
    let fight_node = |encounter: &str| {
        json!({
            "map_point_type": "monster",
            "player_stats": [{}],
            "rooms": [{"model_id": encounter, "room_type": "monster", "turns_taken": 3}],
        })
    };
    let fight_row = |node: i64, encounter: &str| {
        json!({
            "node_index": node,
            "encounter_id": encounter,
            "monster_ids": ["MONSTER.NIBBIT"],
            "turns_taken": 3,
            "relics_entering": [],
            "potions_used": [],
            "deck_entering": [],
        })
    };
    json!({
        "schema": "sts-sim-run-history-v1",
        "run": {
            "build_id": "v0.111.0",
            "seed": "ISSUE2997BANE",
            "players": [{"deck": deck}],
            "map_point_history": [[
                {
                    "map_point_type": "ancient",
                    "player_stats": [first_node],
                    "rooms": [{"model_id": "EVENT.NEOW", "room_type": "event", "turns_taken": 0}],
                },
                fight_node("ENCOUNTER.NIBBITS_WEAK"),
                fight_node("ENCOUNTER.SHRINKER_BEETLE_WEAK"),
                {"map_point_type": "shop", "player_stats": [removal], "rooms": []},
            ]],
        },
        "fights": [
            fight_row(1, "ENCOUNTER.NIBBITS_WEAK"),
            fight_row(2, "ENCOUNTER.SHRINKER_BEETLE_WEAK"),
        ],
    })
}

fn short_ids(history: &Value, floor: i64) -> Vec<String> {
    super::shuffle::test_support::entry_deck_ids(&history["run"], floor)
        .iter()
        .map(|id| id.trim_start_matches("CARD.").to_owned())
        .collect()
}

#[test]
fn a_first_node_transform_result_is_rebuilt_after_the_bane() {
    // Neow transformed a Strike into Feel No Pain (appended after the Bane,
    // as the captured saves of run 1786835938 show), and a shop removed it.
    let history = bane_history(
        &["STRIKE_IRONCLAD", "BASH"],
        &["INFLAME"],
        json!({"cards_transformed": [{
            "original_card": {"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1},
            "final_card": {"id": "CARD.FEEL_NO_PAIN", "floor_added_to_deck": 1},
        }]}),
        json!({"cards_removed": [{"id": "CARD.FEEL_NO_PAIN", "floor_added_to_deck": 1}]}),
    );
    assert_eq!(
        short_ids(&history, 2),
        [
            "STRIKE_IRONCLAD",
            "BASH",
            "ASCENDERS_BANE",
            "INFLAME",
            "FEEL_NO_PAIN"
        ]
    );
    assert!(super::shuffle::test_support::entry_deck_unplaceable(&history["run"], 2).is_empty());
    // The same removal of a card the first node did not add is a starter
    // card: the Bane's slot, as before.
    let history = bane_history(
        &["STRIKE_IRONCLAD", "BASH"],
        &["INFLAME"],
        json!({}),
        json!({"cards_removed": [{"id": "CARD.FEEL_NO_PAIN", "floor_added_to_deck": 1}]}),
    );
    assert_eq!(
        short_ids(&history, 2),
        [
            "STRIKE_IRONCLAD",
            "BASH",
            "FEEL_NO_PAIN",
            "ASCENDERS_BANE",
            "INFLAME"
        ]
    );
}

#[test]
fn a_removed_card_that_is_both_starter_and_first_node_gain_makes_shuffle_a_baseline() {
    // Run 1787114128: Large Capsule added a Strike after the Bane, and an
    // event later transformed one of the five floor-1 Strikes. The run does
    // not say which side of the Bane it came from (the captured save says
    // the starter side; the sibling rule rebuilds the other).
    let history = bane_history(
        &["STRIKE_IRONCLAD", "BASH"],
        &["STRIKE_IRONCLAD"],
        json!({"cards_gained": [{"id": "CARD.STRIKE_IRONCLAD"}]}),
        json!({"cards_transformed": [{
            "original_card": {"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1},
            "final_card": {"id": "CARD.FIEND_FIRE", "floor_added_to_deck": 4},
        }]}),
    );
    assert_eq!(
        super::shuffle::test_support::entry_deck_unplaceable(&history["run"], 2),
        ["CARD.STRIKE_IRONCLAD"]
    );
    let got = answer(&history, 1).unwrap()["counters"]["shuffle"].clone();
    assert_eq!(got["status"], "baseline");
    let caveat = got["caveats"][0].as_str().unwrap();
    assert!(
        caveat.starts_with(
            "fight 0 (ENCOUNTER.NIBBITS_WEAK): a floor-1 copy of STRIKE_IRONCLAD was removed later"
        ),
        "{caveat}"
    );
    // Entering the first fight nothing has been replayed, so nothing is
    // claimed about that deck.
    assert_eq!(
        answer(&history, 0).unwrap()["counters"]["shuffle"]["status"],
        "exact"
    );
    // With no Bane the order carries no information: nothing to caveat.
    let mut history = history;
    history["run"]["players"][0]["deck"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "CARD.ASCENDERS_BANE");
    assert!(super::shuffle::test_support::entry_deck_unplaceable(&history["run"], 2).is_empty());
}

#[test]
fn malformed_first_node_gain_rows_refuse_by_path() {
    type Mutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "$.run.nodes[0].cards_gained",
            Box::new(|stats| stats["cards_gained"] = json!({})),
        ),
        (
            "$.run.nodes[0].cards_gained[0].id",
            Box::new(|stats| stats["cards_gained"] = json!([{"id": 7}])),
        ),
        (
            "$.run.nodes[0].cards_transformed[0].final_card",
            Box::new(|stats| {
                stats["cards_transformed"] =
                    json!([{"original_card": {"id": "CARD.BASH", "floor_added_to_deck": 1}}]);
            }),
        ),
        (
            "$.run.nodes[0].cards_transformed[0].final_card.id",
            Box::new(|stats| {
                stats["cards_transformed"] = json!([{
                    "original_card": {"id": "CARD.BASH", "floor_added_to_deck": 1},
                    "final_card": {},
                }]);
            }),
        ),
    ];
    for (path, mutate) in cases {
        let mut history = base();
        mutate(&mut history["run"]["map_point_history"][0][0]["player_stats"][0]);
        match refusal(&history, 1) {
            RunCountersRefusal::MalformedHistory { path: got, .. } => assert_eq!(got, path),
            other => panic!("{path}: {other:?}"),
        }
    }
    // A null `cards_gained` is an absent one.
    let mut history = base();
    history["run"]["map_point_history"][0][0]["player_stats"][0]["cards_gained"] = json!(null);
    assert!(answer(&history, 1).is_ok());
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
