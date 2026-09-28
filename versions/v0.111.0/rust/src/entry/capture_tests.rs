//! Witnesses for `entry/capture.rs` (#2972): one per mapping branch and per
//! refusal, plus one real corpus pair built both ways.

use super::*;
use crate::catalog::GameBuild;
use crate::entry::{EntryInput, EntryOutcome, EntryRequest, build, build_root};
use serde_json::json;

const PAIR_META: &str = include_str!("../../fixtures/capture_run_pair_v1/pair.json");
const PAIR_CAPTURE: &str = include_str!("../../fixtures/capture_run_pair_v1/capture_run.json");
const PAIR_SAVE: &str = include_str!("../../fixtures/capture_run_pair_v1/save.json");

fn stream(seed: u64) -> Value {
    json!([seed, seed + 1, seed + 2, seed + 3])
}

/// A small capture run spelled exactly as `mcr_parser` writes one.
fn minimal() -> Value {
    let mut counters = Map::new();
    let mut states = Map::new();
    for (index, name) in RUN_RNG_TYPE_NAMES.iter().enumerate() {
        counters.insert((*name).to_string(), json!(index));
        states.insert((*name).to_string(), stream(index as u64 * 10));
    }
    json!({
        "schema_version": 20,
        "acts": [{
            "id": "OVERGROWTH",
            "rooms": {
                "event_ids": ["TEA_MASTER"], "events_visited": 0,
                "normal_encounter_ids": ["TOADPOLES_WEAK", "SEAPUNK_WEAK"],
                "normal_encounters_visited": 1,
                "elite_encounter_ids": ["KNIGHTS_ELITE", "TERROR_EEL_ELITE"],
                "elites_visited": 1, "bosses_visited": 0,
                "boss_id": "VANTOM_BOSS", "second_boss_id": null, "ancient_id": "NEOW"
            },
            "saved_map": {
                "grid_width": 7, "grid_height": 16,
                "boss_point": {"coord": [3, 17], "point_type": "Boss",
                               "can_be_modified": false, "child_coords": []},
                "starting_point": {"coord": [3, 0], "point_type": "Ancient",
                                   "can_be_modified": false, "child_coords": [[3, 1]]},
                "second_boss_point": null,
                "points": [
                    {"coord": [3, 1], "point_type": "Monster", "can_be_modified": true,
                     "child_coords": [[4, 2]]},
                    {"coord": [4, 2], "point_type": "Elite", "can_be_modified": true,
                     "child_coords": [[4, 3]]},
                    {"coord": [4, 3], "point_type": "RestSite", "can_be_modified": false,
                     "child_coords": []}
                ],
                "start_map_point_coords": [[3, 1]]
            }
        }],
        "modifiers": [],
        "daily_time": null,
        "game_mode": "Standard",
        "current_act_index": 0,
        "events_seen": ["NEOW"],
        "pre_finished_room": null,
        "run_odds": {"card_reward_odds": 0.100_000_001_490_116_12, "potion_reward_odds": -5.0,
                     "relic_reward_odds": 0.25, "gold_reward_odds": 0.029_999_999_329_447_746},
        "players": [{
            "net_id": 1, "character": "IRONCLAD", "current_hp": 70, "max_hp": 80,
            "max_energy": 3, "max_potion_slots": 3, "gold": 99, "base_orb_slots": 0,
            "deck": [
                {"id": "STRIKE_IRONCLAD", "upgrade_level": 0, "enchantment": null,
                 "props": null, "floor_added_to_deck": 1},
                {"id": "BASH", "upgrade_level": 1,
                 "enchantment": {"id": "SHARP", "level": 3, "props": null},
                 "props": null, "floor_added_to_deck": null}
            ],
            "relics": [{"id": "BURNING_BLOOD", "props": null, "floor_added": 1}],
            "potions": [{"id": "FIRE_POTION", "slot": 1}],
            "player_rng": {"seed": 18_358_128_862_073_348_558_u64,
                           "counters": {"Rewards": 4, "Shops": 0, "Transformations": 2},
                           "states": {"Rewards": stream(100), "Shops": stream(200),
                                      "Transformations": stream(300)}},
            "odds": {"card_rarity_odds_value": -0.050_000_000_745_058_06,
                     "potion_reward_odds_value": 0.400_000_005_960_464_5},
            "relic_grab_bag": {"Common": ["VAJRA"], "Shop": []},
            "extra_fields": {"card_shop_removals_used": 1, "wongo_points": 0,
                             "cccombo_badge_unlocked": true, "damage_dealt": 5,
                             "debuffs_applied": 2},
            "unlock_state": {"unlocked_epochs": ["IRONCLAD2_EPOCH"],
                             "encounters_seen": ["TOADPOLES_WEAK"], "number_of_runs": 4},
            "discovered_cards": ["CARD.BASH"], "discovered_enemies": [],
            "discovered_epochs": ["IRONCLAD2_EPOCH"], "discovered_potions": [],
            "discovered_relics": ["RELIC.VAJRA"]
        }],
        "rng": {"seed": "ZPJHU3WSH2", "counters": counters, "states": states},
        "shared_relic_grab_bag": {"Rare": ["GIRYA"]},
        "visited_map_coords": [[3, 0], [3, 1], [4, 2]],
        "map_point_history": [[{"map_point_type": "Ancient"}, {"map_point_type": "Monster"}]],
        "save_time": 11, "start_time": 7, "run_time": 3, "win_time": 0,
        "ascension": 0,
        "map_drawings": [{"player_id": 1, "lines": []}],
        "extra_fields": {"started_with_neow": true, "test_subject_kills": 0,
                         "freed_repy": false},
        "num_reloads": 2
    })
}

fn text(value: &Value) -> String {
    serde_json::to_string(value).unwrap()
}

fn refusal(value: &Value) -> EntryRefusal {
    parse(&text(value)).unwrap_err()
}

#[test]
fn every_field_maps_by_its_il_position() {
    let run = parse(&text(&minimal())).unwrap();
    let act = &run.acts[0];
    assert_eq!(act.id.as_deref(), Some("ACT.OVERGROWTH"));
    assert_eq!(act.rooms.event_ids, ["EVENT.TEA_MASTER"]);
    assert_eq!(act.rooms.normal_encounters_visited, 1);
    // `elites_visited` / `bosses_visited` are the decoder's labels for
    // EliteEncountersVisited / BossEncountersVisited (RVA 0x41f1c).
    assert_eq!(act.rooms.elite_encounters_visited, 1);
    assert_eq!(act.rooms.boss_encounters_visited, 0);
    assert_eq!(act.rooms.boss_id.as_deref(), Some("ENCOUNTER.VANTOM_BOSS"));
    assert_eq!(act.rooms.second_boss_id, None);
    assert_eq!(act.rooms.ancient_id.as_deref(), Some("EVENT.NEOW"));
    let map = act.saved_map.as_ref().unwrap();
    assert_eq!((map.width, map.height), (Some(7), Some(16)));
    assert_eq!(
        map.boss.as_ref().unwrap().point_type.as_deref(),
        Some("boss")
    );
    assert_eq!(
        map.start.as_ref().unwrap().point_type.as_deref(),
        Some("ancient")
    );
    assert_eq!(map.second_boss, None);
    assert_eq!(map.start_coords, [Coord { col: 3, row: 1 }]);
    let rest = &map.points[2];
    assert_eq!(rest.coord, Some(Coord { col: 4, row: 3 }));
    assert_eq!(rest.point_type.as_deref(), Some("rest_site"));
    assert_eq!(rest.can_modify, Some(false));
    assert_eq!(map.points[0].children, [Coord { col: 4, row: 2 }]);

    // SerializableRunOddsSet::Serialize writes Monster, Elite, Treasure,
    // Shop (RVA 0x42010); the decoder calls them card/potion/relic/gold.
    let odds = run.odds.as_ref().unwrap();
    assert_eq!(odds.unknown_map_point_monster_odds_value, Some(0.1));
    assert_eq!(odds.unknown_map_point_elite_odds_value, Some(-5.0));
    assert_eq!(odds.unknown_map_point_treasure_odds_value, Some(0.25));
    assert_eq!(odds.unknown_map_point_shop_odds_value, Some(0.03));

    let player = run.single_player().unwrap();
    assert_eq!(player.character_id.as_deref(), Some("CHARACTER.IRONCLAD"));
    assert_eq!(player.max_potion_slot_count, Some(3));
    assert_eq!(player.base_orb_slot_count, Some(0));
    assert_eq!(player.net_id, Some(1));
    assert_eq!(player.deck[0].id.as_deref(), Some("CARD.STRIKE_IRONCLAD"));
    assert_eq!(player.deck[0].current_upgrade_level, Some(0));
    assert_eq!(player.deck[0].floor_added_to_deck, Some(1));
    let sharp = player.deck[1].enchantment.as_ref().unwrap();
    assert_eq!(
        (sharp.id.as_str(), sharp.amount),
        ("ENCHANTMENT.SHARP", Some(3))
    );
    assert_eq!(player.deck[1].floor_added_to_deck, None);
    assert_eq!(player.relics[0].id.as_deref(), Some("RELIC.BURNING_BLOOD"));
    assert_eq!(player.relics[0].floor_added_to_deck, Some(1));
    assert_eq!(player.potions[0].id.as_deref(), Some("POTION.FIRE_POTION"));
    assert_eq!(player.potions[0].slot_index, Some(1));
    let player_rng = player.rng.as_ref().unwrap();
    assert_eq!(player_rng.seed, Some(18_358_128_862_073_348_558));
    assert_eq!(player_rng.rngs["rewards"].counter, 4);
    assert_eq!(player_rng.rngs["transformations"].s3, 303);
    let player_odds = player.odds.as_ref().unwrap();
    assert_eq!(player_odds.card_rarity_odds_value, Some(-0.05));
    assert_eq!(player_odds.potion_reward_odds_value, Some(0.4));
    let bag = &player.relic_grab_bag.as_ref().unwrap().relic_id_lists;
    assert_eq!(bag["common"], ["RELIC.VAJRA"]);
    assert!(bag["shop"].is_empty());
    assert_eq!(
        player.extra_fields.as_ref().unwrap().cccombo_badge_unlocked,
        Some(true)
    );
    let unlocks = player.unlock_state.as_ref().unwrap();
    assert_eq!(
        unlocks.encounters_seen.as_deref(),
        Some(&["ENCOUNTER.TOADPOLES_WEAK".to_string()][..])
    );
    assert_eq!(unlocks.number_of_runs, Some(4));
    assert_eq!(player.discovered_relics, ["RELIC.VAJRA"]);

    let rng = run.run_rng().unwrap();
    assert_eq!(rng.seed, "ZPJHU3WSH2");
    let rngs = rng.rngs.as_ref().unwrap();
    assert_eq!(rngs.len(), 12);
    for (index, snake) in RUN_RNG_STREAMS.iter().enumerate() {
        assert_eq!(rngs[*snake].counter, index as u64);
        assert_eq!(rngs[*snake].s0, index as u64 * 10);
    }
    assert_eq!(
        run.shared_relic_grab_bag.as_ref().unwrap().relic_id_lists["rare"],
        ["RELIC.GIRYA"]
    );
    assert_eq!(
        run.visited_map_coords.last(),
        Some(&Coord { col: 4, row: 2 })
    );
    assert_eq!(run.map_point_history.as_ref().map(Vec::len), Some(1));
    assert_eq!(run.events_seen, ["EVENT.NEOW"]);
    assert_eq!(run.game_mode.as_deref(), Some("standard"));
    assert_eq!(run.ascension, Some(0));
    assert_eq!(run.platform_type, None);
    assert_eq!(run.pre_finished_room, None);
    assert_eq!(run.start_time, Some(7));
    assert_eq!(run.num_reloads, Some(2));
    assert_eq!(
        run.extra_fields.as_ref().unwrap().started_with_neow,
        Some(true)
    );
}

#[test]
fn the_node_and_its_encounter_derive_from_the_mapped_run() {
    // The current coordinate is the Elite point, and the second elite is
    // scheduled because `elites_visited` is 1.
    let capture = text(&minimal());
    let outcome = build(&EntryRequest {
        input: EntryInput::CaptureRun(&capture),
        encounter_id: None,
        node_type: None,
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    });
    let EntryOutcome::Built(document) = outcome else {
        panic!("the minimal capture builds: {outcome:?}");
    };
    assert_eq!(document.node_type.as_deref(), Some("elite"));
    assert_eq!(document.encounter_id, "ENCOUNTER.TERROR_EEL_ELITE");
    assert_eq!(document.node_index, 2);
    assert_eq!(document.ascension, Some(0));
}

fn pair_request<'a>(input: EntryInput<'a>, meta: &'a Value) -> EntryRequest<'a> {
    EntryRequest {
        input,
        encounter_id: meta["encounter_id"].as_str(),
        node_type: meta["node_type"].as_str(),
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    }
}

#[test]
fn a_real_capture_and_its_first_save_build_the_same_entry_and_opening() {
    // The corpus pair: Necrobinder, act 2, an enchanted Defy, relic property
    // bags, and Mad Science's two per-instance ints.
    let meta: Value = serde_json::from_str(PAIR_META).unwrap();
    let from_capture = build(&pair_request(EntryInput::CaptureRun(PAIR_CAPTURE), &meta));
    let from_save = build(&pair_request(EntryInput::Save(PAIR_SAVE), &meta));
    assert!(matches!(from_capture, EntryOutcome::Built(_)));
    assert_eq!(from_capture.to_json(), from_save.to_json());
    let opened_capture = build_root(
        &pair_request(EntryInput::CaptureRun(PAIR_CAPTURE), &meta),
        None,
    );
    let opened_save = build_root(&pair_request(EntryInput::Save(PAIR_SAVE), &meta), None);
    assert_eq!(opened_capture.to_json(), opened_save.to_json());
}

#[test]
fn a_real_capture_maps_the_same_act_maps_and_derives_the_same_node() {
    // `SerializableMapPoint::Serialize` (RVA 0x40d80) writes Coord before
    // PointType. A decoder that reads PointType first hands this module
    // `point_type` = Coord.col and `coord` = [row, PointType]; the explicit
    // `--node-type` the review path passes hides that, so the maps are
    // compared whole here and the node is derived with no hints (#2988 review).
    let capture = parse(PAIR_CAPTURE).expect("the pair's capture maps");
    let save = crate::entry::save::SerializedRun::parse(PAIR_SAVE).expect("the pair's save");
    assert_eq!(capture.acts.len(), save.acts.len());
    let mut maps = 0;
    // The binary writer always writes CanBeModified; the JSON save omits it
    // at `false` (module doc, "Absence"), so only that default is normalized.
    let omit_false = |map: &Option<crate::entry::save::SerializedActMap>| {
        map.clone().map(|mut map| {
            let points = map.points.iter_mut().chain(map.boss.iter_mut());
            for point in points
                .chain(map.start.iter_mut())
                .chain(map.second_boss.iter_mut())
            {
                point.can_modify = point.can_modify.filter(|modifiable| *modifiable);
            }
            map
        })
    };
    for (from_capture, from_save) in capture.acts.iter().zip(&save.acts) {
        assert_eq!(omit_false(&from_capture.saved_map), from_save.saved_map);
        maps += usize::from(from_save.saved_map.is_some());
    }
    assert!(maps > 0, "the pair carries at least one saved map");
    let unhinted = |input| EntryRequest {
        input,
        encounter_id: None,
        node_type: None,
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    };
    let meta: Value = serde_json::from_str(PAIR_META).unwrap();
    let EntryOutcome::Built(document) = build(&unhinted(EntryInput::CaptureRun(PAIR_CAPTURE)))
    else {
        panic!("the pair's capture derives its node and encounter");
    };
    assert_eq!(document.node_type.as_deref(), meta["node_type"].as_str());
    assert_eq!(
        Some(document.encounter_id.as_str()),
        meta["encounter_id"].as_str()
    );
    assert_eq!(
        build(&unhinted(EntryInput::CaptureRun(PAIR_CAPTURE))).to_json(),
        build(&unhinted(EntryInput::Save(PAIR_SAVE))).to_json()
    );
}

#[test]
fn property_rows_keep_the_writers_order_not_a_sorted_one() {
    // The save lists TinkerTimeType before TinkerTimeRider; a sorted map
    // would put Rider first and change the entry document.
    let meta: Value = serde_json::from_str(PAIR_META).unwrap();
    let EntryOutcome::Built(document) =
        build(&pair_request(EntryInput::CaptureRun(PAIR_CAPTURE), &meta))
    else {
        panic!("the pair builds");
    };
    let mad_science = document
        .deck_entering
        .iter()
        .find(|row| row.id == "CARD.MAD_SCIENCE")
        .unwrap();
    let names: Vec<&str> = mad_science.props.as_ref().unwrap()["ints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["TinkerTimeType", "TinkerTimeRider"]);
}

/// `minimal()` with the given raw property-bag text on relic 0.
fn with_relic_props(props: &str) -> String {
    let mut value = minimal();
    value["players"][0]["relics"][0]["props"] = json!("__PROPS__");
    text(&value).replace("\"__PROPS__\"", props)
}

fn group(bag: &Map<String, Value>, name: &str) -> Vec<(String, Value)> {
    bag[name]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap().to_string(),
                row["value"].clone(),
            )
        })
        .collect()
}

#[test]
fn property_groups_come_from_the_declared_type() {
    let card = r#"{"id":"BASH","upgrade_level":1,"enchantment":null,"props":null,"floor_added_to_deck":3}"#;
    let props = format!(
        r#"{{"TurnsSeen":2,"IsUsed":true,"Skin":"wings","StarterRelic":"RELIC.VAJRA",
           "AncientCard":{card},"StarterCard":{card},"SerializableCards":[{card}],
           "FurCoatCoordCols":[1,2],"CharacterId":"CHARACTER.IRONCLAD"}}"#
    );
    let run = parse(&with_relic_props(&props)).unwrap();
    let bag = run.single_player().unwrap().relics[0]
        .props
        .clone()
        .unwrap();
    assert_eq!(group(&bag, "ints"), [("TurnsSeen".to_string(), json!(2))]);
    assert_eq!(group(&bag, "bools"), [("IsUsed".to_string(), json!(true))]);
    assert_eq!(
        group(&bag, "strings"),
        [("Skin".to_string(), json!("wings"))]
    );
    assert_eq!(
        group(&bag, "model_ids"),
        [
            ("StarterRelic".to_string(), json!("RELIC.VAJRA")),
            ("CharacterId".to_string(), json!("CHARACTER.IRONCLAD"))
        ]
    );
    let nested = json!({"id": "CARD.BASH", "current_upgrade_level": 1, "floor_added_to_deck": 3});
    assert_eq!(
        group(&bag, "cards"),
        [
            ("AncientCard".to_string(), nested.clone()),
            ("StarterCard".to_string(), nested.clone())
        ]
    );
    assert_eq!(
        group(&bag, "card_arrays"),
        [("SerializableCards".to_string(), json!([nested]))]
    );
    assert_eq!(
        group(&bag, "int_arrays"),
        [("FurCoatCoordCols".to_string(), json!([1, 2]))]
    );
    // The same name, declared as a ModelId on another model.
    let run = parse(&with_relic_props(r#"{"AncientCard":"CARD.BASH"}"#)).unwrap();
    let bag = run.single_player().unwrap().relics[0]
        .props
        .clone()
        .unwrap();
    assert_eq!(
        group(&bag, "model_ids"),
        [("AncientCard".to_string(), json!("CARD.BASH"))]
    );
}

#[test]
fn a_nested_card_is_mapped_with_its_enchantment_and_props() {
    let card = r#"{"id":"DEFY","upgrade_level":0,"enchantment":{"id":"NIMBLE","level":2,"props":{"TimesUsed":1}},"props":{"SpoilsActIndex":1},"floor_added_to_deck":null}"#;
    let run = parse(&with_relic_props(&format!(r#"{{"StarterCard":{card}}}"#))).unwrap();
    let bag = run.single_player().unwrap().relics[0]
        .props
        .clone()
        .unwrap();
    assert_eq!(
        group(&bag, "cards")[0].1,
        json!({"id": "CARD.DEFY",
               "enchantment": {"id": "ENCHANTMENT.NIMBLE", "amount": 2,
                               "props": {"ints": [{"name": "TimesUsed", "value": 1}]}},
               "props": {"ints": [{"name": "SpoilsActIndex", "value": 1}]}})
    );
}

#[test]
fn an_empty_property_bag_stays_an_empty_bag() {
    let run = parse(&with_relic_props("{}")).unwrap();
    assert_eq!(
        run.single_player().unwrap().relics[0].props,
        Some(Map::new())
    );
}

#[test]
fn a_property_outside_the_il_set_refuses_by_name() {
    assert_eq!(
        parse(&with_relic_props(r#"{"SoulPower":3}"#)).unwrap_err(),
        EntryRefusal::CapturePropertyUnknown {
            owner: "RELIC.BURNING_BLOOD".to_string(),
            name: "SoulPower".to_string(),
        }
    );
}

#[test]
fn a_property_value_of_the_wrong_type_refuses() {
    for (props, name, group) in [
        (r#"{"IsUsed":3}"#, "IsUsed", "bools"),
        (r#"{"TurnsSeen":true}"#, "TurnsSeen", "ints"),
        (r#"{"Skin":1}"#, "Skin", "strings"),
        (
            r#"{"FurCoatCoordRows":[true]}"#,
            "FurCoatCoordRows",
            "int_arrays",
        ),
    ] {
        assert_eq!(
            parse(&with_relic_props(props)).unwrap_err(),
            EntryRefusal::CapturePropertyType {
                owner: "RELIC.BURNING_BLOOD".to_string(),
                name: name.to_string(),
                group,
            },
            "{props}"
        );
    }
    // A card-typed name whose value is not a card is a shape refusal.
    for props in [r#"{"StarterCard":"CARD.BASH"}"#, r#"{"AncientCard":3}"#] {
        assert_eq!(
            parse(&with_relic_props(props)).unwrap_err().class(),
            "unknown_capture_field",
            "{props}"
        );
    }
}

#[test]
fn an_untaught_or_missing_capture_key_refuses_by_name() {
    let mut value = minimal();
    value["soul_power"] = json!(3);
    let refused = refusal(&value);
    assert_eq!(refused.class(), "unknown_capture_field");
    assert!(refused.to_string().contains("soul_power"));

    // The save's spelling is not a synonym for the decoder's.
    let mut value = minimal();
    let rooms = value["acts"][0]["rooms"].as_object_mut().unwrap();
    let visited = rooms.remove("elites_visited").unwrap();
    rooms.insert("elite_encounters_visited".to_string(), visited);
    let refused = refusal(&value);
    assert_eq!(refused.class(), "unknown_capture_field");
    assert!(refused.to_string().contains("elite_encounters_visited"));

    let mut value = minimal();
    value["players"][0]
        .as_object_mut()
        .unwrap()
        .remove("max_potion_slots");
    assert!(refusal(&value).to_string().contains("max_potion_slots"));
}

#[test]
fn a_present_pre_finished_room_refuses_rather_than_mapping_unexercised_fields() {
    let mut value = minimal();
    value["pre_finished_room"] = json!({"room_type": 1});
    assert_eq!(
        refusal(&value),
        EntryRefusal::CaptureFieldUnmapped {
            field: "pre_finished_room"
        }
    );
}

type Damage = fn(&mut Value);

#[test]
fn an_enum_member_outside_its_enum_refuses() {
    let cases: [(&str, Damage); 5] = [
        ("MapPointType", |v| {
            v["acts"][0]["saved_map"]["points"][0]["point_type"] = json!("Swamp");
        }),
        ("GameMode", |v| v["game_mode"] = json!("Endless")),
        ("RelicRarity", |v| {
            v["shared_relic_grab_bag"] = json!({"Mythic": []});
        }),
        ("RunRngType", |v| {
            v["rng"]["counters"]["Weather"] = json!(0);
            v["rng"]["states"]["Weather"] = json!([1, 2, 3, 4]);
        }),
        ("PlayerRngType", |v| {
            v["players"][0]["player_rng"]["counters"]["Bets"] = json!(0);
            v["players"][0]["player_rng"]["states"]["Bets"] = json!([1, 2, 3, 4]);
        }),
    ];
    for (expected, damage) in cases {
        let mut value = minimal();
        damage(&mut value);
        let refused = refusal(&value);
        assert!(
            matches!(&refused, EntryRefusal::CaptureEnumUnknown { enumeration, .. }
                if *enumeration == expected),
            "{expected}: {refused:?}"
        );
    }
}

#[test]
fn a_stream_needs_both_its_counter_and_its_state_words() {
    let mut value = minimal();
    value["rng"]["states"]
        .as_object_mut()
        .unwrap()
        .remove("Niche");
    assert_eq!(
        refusal(&value),
        EntryRefusal::CaptureRngStateMissing {
            stream: "Niche".to_string()
        }
    );
    let mut value = minimal();
    value["rng"]["counters"]
        .as_object_mut()
        .unwrap()
        .remove("Shuffle");
    assert_eq!(
        refusal(&value),
        EntryRefusal::CaptureRngStateMissing {
            stream: "Shuffle".to_string()
        }
    );
    let mut value = minimal();
    value["rng"]["counters"]["Shuffle"] = json!(-1);
    assert!(refusal(&value).to_string().contains("negative"));
}

#[test]
fn seeds_and_net_ids_outside_their_types_refuse() {
    let mut value = minimal();
    value["rng"]["seed"] = json!(5);
    assert!(refusal(&value).to_string().contains("rng.seed"));
    let mut value = minimal();
    value["players"][0]["player_rng"]["seed"] = json!("SEED");
    assert!(refusal(&value).to_string().contains("player_rng.seed"));
    let mut value = minimal();
    value["players"][0]["net_id"] = json!(u64::MAX);
    assert!(refusal(&value).to_string().contains("net_id"));
}

#[test]
fn the_schema_is_decided_before_any_field() {
    let mut value = minimal();
    value["schema_version"] = json!(19);
    value["soul_power"] = json!(1);
    assert_eq!(refusal(&value), EntryRefusal::UnsupportedSaveSchema(19));
    let mut value = minimal();
    value.as_object_mut().unwrap().remove("schema_version");
    assert_eq!(
        refusal(&value),
        EntryRefusal::MissingSaveField {
            owner: "run",
            field: "schema_version"
        }
    );
    assert_eq!(parse("[").unwrap_err().class(), "malformed_save_json");
}

#[test]
fn a_stored_float_reads_back_at_its_own_precision() {
    assert_eq!(float(0.100_000_001_490_116_12), 0.1);
    assert_eq!(float(-5.0), -5.0);
}
