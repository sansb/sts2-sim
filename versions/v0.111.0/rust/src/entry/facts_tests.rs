//! Witnesses for the entry-facts input (`entry/facts.rs`, #2827 item B).
//!
//! The load-bearing claim is the round trip: every save the fixtures carry
//! builds entry facts, those facts written as a facts document parse back to
//! the same [`EntryDocument`], and the Rust opening on the parsed facts is the
//! opening on the save, byte for byte. Every refusal branch then gets one
//! mutation of a valid document that reaches it and no other.

use serde_json::{Value, json};

use super::*;
use crate::entry::{EntryInput, EntryOutcome, EntryRequest, build, build_root};

const BUILDER_FIXTURE: &str = include_str!("../../fixtures/entry_builder_v1.json");
const PAIR_META: &str = include_str!("../../fixtures/capture_run_pair_v1/pair.json");
const PAIR_SAVE: &str = include_str!("../../fixtures/capture_run_pair_v1/save.json");

/// `(save text, encounter, node type)` for every save the fixtures carry, plus
/// the builder fixture with an empty belt so its opening builds past the
/// potion-belt boundary ceiling (`opening/fixture_tests.rs`).
fn saves() -> Vec<(String, String, String)> {
    let builder: Value = serde_json::from_str(BUILDER_FIXTURE).unwrap();
    let mut out = Vec::new();
    for case in builder["cases"].as_array().unwrap() {
        let encounter = case["encounter_id"].as_str().unwrap().to_string();
        let node_type = case["node_type"].as_str().unwrap().to_string();
        out.push((
            case["save"].to_string(),
            encounter.clone(),
            node_type.clone(),
        ));
        let mut empty_belt = case["save"].clone();
        empty_belt["players"][0]["max_potion_slot_count"] = json!(0);
        out.push((empty_belt.to_string(), encounter, node_type));
    }
    let meta: Value = serde_json::from_str(PAIR_META).unwrap();
    out.push((
        PAIR_SAVE.to_string(),
        meta["encounter_id"].as_str().unwrap().to_string(),
        meta["node_type"].as_str().unwrap().to_string(),
    ));
    out
}

fn request<'a>(save: &'a str, encounter: &'a str, node_type: &'a str) -> EntryRequest<'a> {
    EntryRequest {
        input: EntryInput::Save(save),
        encounter_id: Some(encounter),
        node_type: Some(node_type),
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    }
}

fn document_of(save: &str, encounter: &str, node_type: &str) -> EntryDocument {
    match build(&request(save, encounter, node_type)) {
        EntryOutcome::Built(document) => *document,
        EntryOutcome::Refused(refusal) => panic!("fixture save builds: {refusal}"),
    }
}

/// A valid facts document: the builder fixture's empty-belt save, whose
/// opening builds.
fn valid() -> Value {
    let (save, encounter, node_type) = saves().swap_remove(1);
    document_of(&save, &encounter, &node_type).facts_json()
}

fn refusal_of(value: &Value) -> EntryRefusal {
    parse(&value.to_string(), GameBuild::V0_111_0).expect_err("the mutation refuses")
}

#[test]
fn every_fixture_save_round_trips_through_the_facts_document() {
    let mut opened = 0;
    for (save, encounter, node_type) in saves() {
        let document = document_of(&save, &encounter, &node_type);
        let text = document.facts_json().to_string();
        let parsed = parse(&text, GameBuild::V0_111_0)
            .unwrap_or_else(|refusal| panic!("{encounter}: facts parse: {refusal}"));
        assert_eq!(parsed, document, "{encounter}");
        // And the opening on the parsed facts is the opening on the save.
        let from_save = build_root(&request(&save, &encounter, &node_type), None).to_json();
        let from_facts = crate::entry::root_from_document(parsed, None).to_json();
        assert_eq!(from_facts, from_save, "{encounter}");
        if from_save["schema"] == json!("sts-sim-canonical-v2") {
            opened += 1;
        }
    }
    // Not vacuous: the empty-belt case opens; the other two refuse their
    // openings by name, and the refusal documents compare equal too.
    assert!(opened >= 1, "no fixture opening built");
}

#[test]
fn the_entry_wire_form_without_streams_is_refused_by_name() {
    // `to_json` alone is what `--save` prints; the facts input also needs the
    // stream words the opening reads.
    let (save, encounter, node_type) = saves().swap_remove(1);
    let wire = document_of(&save, &encounter, &node_type).to_json();
    assert_eq!(
        refusal_of(&wire),
        EntryRefusal::UnknownFactsField("facts.streams is absent".to_string())
    );
}

#[test]
fn a_document_that_is_not_json_refuses() {
    let refusal = parse("{", GameBuild::V0_111_0).unwrap_err();
    assert_eq!(refusal.class(), "malformed_facts_json");
}

/// The two optional off-wire keys (#3162): absent or `null` is `None`, a
/// present one round-trips, and a spelling outside `MapPointType` or a
/// non-canonical encounter id refuses.
#[test]
fn the_optional_map_point_and_next_encounter_keys_round_trip_and_validate() {
    let mut value = valid();
    value.as_object_mut().unwrap().remove("map_point_type");
    value
        .as_object_mut()
        .unwrap()
        .remove("next_normal_encounter");
    let bare = parse(&value.to_string(), GameBuild::V0_111_0).expect("absent keys parse");
    assert_eq!(bare.map_point_type, None);
    assert_eq!(bare.next_normal_encounter, None);
    value["map_point_type"] = json!(null);
    value["next_normal_encounter"] = json!(null);
    let null = parse(&value.to_string(), GameBuild::V0_111_0).expect("null keys parse");
    assert_eq!(null.map_point_type, None);
    value["map_point_type"] = json!("unknown");
    value["next_normal_encounter"] = json!("ENCOUNTER.CHOMPERS_NORMAL");
    let named = parse(&value.to_string(), GameBuild::V0_111_0).expect("named keys parse");
    assert_eq!(named.map_point_type.as_deref(), Some("unknown"));
    assert_eq!(
        named.next_normal_encounter.as_deref(),
        Some("ENCOUNTER.CHOMPERS_NORMAL")
    );
    assert_eq!(
        parse(&named.facts_json().to_string(), GameBuild::V0_111_0).unwrap(),
        named
    );
    let mut bad_point = value.clone();
    bad_point["map_point_type"] = json!("dungeon");
    assert_eq!(refusal_of(&bad_point).class(), "facts_inconsistent");
    let mut bad_encounter = value;
    bad_encounter["next_normal_encounter"] = json!("CHOMPERS_NORMAL");
    assert!(parse(&bad_encounter.to_string(), GameBuild::V0_111_0).is_err());
}

#[test]
fn the_key_surface_is_exact_at_every_level() {
    let mut extra = valid();
    extra["surprise"] = json!(1);
    assert_eq!(refusal_of(&extra).class(), "unknown_facts_field");
    let mut entry_extra = valid();
    entry_extra["entry"]["potions_used"] = json!([]);
    assert_eq!(refusal_of(&entry_extra).class(), "unknown_facts_field");
    let mut missing = valid();
    missing["entry"]
        .as_object_mut()
        .unwrap()
        .remove("gold_entering");
    assert_eq!(refusal_of(&missing).class(), "unknown_facts_field");
    let mut wrong_type = valid();
    wrong_type["entry"]["hp_entering"] = json!("80");
    assert_eq!(refusal_of(&wrong_type).class(), "unknown_facts_field");
    let mut card_extra = valid();
    card_extra["entry"]["deck_entering"][0]["floor_added_to_deck"] = json!(1);
    assert_eq!(refusal_of(&card_extra).class(), "unknown_facts_field");
    let mut unlock_extra = valid();
    unlock_extra["unlocks"]["epochs"] = json!([]);
    assert_eq!(refusal_of(&unlock_extra).class(), "unknown_facts_field");
    let mut stream_extra = valid();
    stream_extra["streams"]["shuffle"]["s4"] = json!(0);
    assert_eq!(refusal_of(&stream_extra).class(), "unknown_facts_field");
    let mut slot_missing = valid();
    slot_missing["entry"]["potion_slots_entering"] = json!([{"id": "POTION.FYSH_OIL"}]);
    slot_missing["entry"]["potions_entering"] = json!(["POTION.FYSH_OIL"]);
    assert_eq!(refusal_of(&slot_missing).class(), "unknown_facts_field");
}

#[test]
fn schema_build_and_save_schema_are_checked() {
    let mut schema = valid();
    schema["schema"] = json!("sts-sim-canonical-v2");
    assert_eq!(
        refusal_of(&schema),
        EntryRefusal::UnsupportedFactsSchema("sts-sim-canonical-v2".to_string())
    );
    let mut build = valid();
    build["game_build"] = json!("v0.110.1");
    assert_eq!(refusal_of(&build).class(), "facts_build_mismatch");
    let mut save_schema = valid();
    save_schema["save_schema_version"] = json!(19);
    assert_eq!(
        refusal_of(&save_schema),
        EntryRefusal::UnsupportedSaveSchema(19)
    );
}

#[test]
fn ids_must_already_be_canonical() {
    for (pointer, bare, category) in [
        ("/entry/deck_entering/0/id", "STRIKE_IRONCLAD", "CARD"),
        ("/entry/relics_entering/0", "BURNING_BLOOD", "RELIC"),
        ("/entry/encounter_id", "TOADPOLES_WEAK", "ENCOUNTER"),
        ("/character", "IRONCLAD", "CHARACTER"),
    ] {
        let mut value = valid();
        *value.pointer_mut(pointer).unwrap() = json!(bare);
        assert_eq!(
            refusal_of(&value),
            EntryRefusal::MalformedModelId {
                category,
                value: bare.to_string()
            },
            "{pointer}"
        );
    }
    let mut enchanted = valid();
    enchanted["entry"]["deck_entering"][0]["enchantment"] = json!("SHARP");
    enchanted["entry"]["deck_entering"][0]["enchant_amount"] = json!(2);
    assert_eq!(refusal_of(&enchanted).class(), "malformed_model_id");
    let mut potion = valid();
    potion["entry"]["potion_slots_entering"] = json!([{"id": "FYSH_OIL", "slot_index": 0}]);
    potion["entry"]["potions_entering"] = json!(["FYSH_OIL"]);
    assert_eq!(refusal_of(&potion).class(), "malformed_model_id");
}

fn field_of(refusal: EntryRefusal) -> &'static str {
    match refusal {
        EntryRefusal::FactsInconsistent { field, .. } => field,
        other => panic!("expected facts_inconsistent, got {other}"),
    }
}

#[test]
fn deck_rows_carry_only_what_the_save_path_writes() {
    let mut props = valid();
    props["entry"]["deck_entering"][0]["props"] = json!({"ints": []});
    assert_eq!(field_of(refusal_of(&props)), "entry.deck_entering");
    let mut half_enchanted = valid();
    half_enchanted["entry"]["deck_entering"][0]["enchantment"] = json!("ENCHANTMENT.SHARP");
    assert_eq!(field_of(refusal_of(&half_enchanted)), "entry.deck_entering");
    // A per-instance card does carry its props through.
    let mut scythe = valid();
    scythe["entry"]["deck_entering"][0] = json!({
        "id": "CARD.THE_SCYTHE", "upgrade_level": 0,
        "props": {"ints": [{"name": "CurrentDamage", "value": 13}]}});
    let parsed = parse(&scythe.to_string(), GameBuild::V0_111_0).unwrap();
    assert!(parsed.deck_entering[0].props.is_some());
}

#[test]
fn belt_and_relic_facts_must_agree_with_each_other() {
    let mut potions = valid();
    potions["entry"]["potions_entering"] = json!(["POTION.FYSH_OIL"]);
    assert_eq!(field_of(refusal_of(&potions)), "entry.potions_entering");

    let mut unowned = valid();
    unowned["entry"]["relic_counters"] = json!({"RELIC.NUNCHAKU": 3});
    assert_eq!(field_of(refusal_of(&unowned)), "entry.relic_counters");
    let mut empty_bag = valid();
    empty_bag["entry"]["relic_counters"] = json!({});
    assert_eq!(field_of(refusal_of(&empty_bag)), "entry.relic_counters");
    let mut not_a_bag = valid();
    not_a_bag["entry"]["relic_counters"] = json!([]);
    assert_eq!(refusal_of(&not_a_bag).class(), "unknown_facts_field");

    let relic = valid()["entry"]["relics_entering"][0]
        .as_str()
        .unwrap()
        .to_string();
    for bad in [
        json!(true),
        json!("3"),
        json!(1.5),
        json!({}),
        json!({"A": "b"}),
    ] {
        let mut shaped = valid();
        shaped["entry"]["relic_counters"] = json!({ relic.clone(): bad });
        assert_eq!(
            field_of(refusal_of(&shaped)),
            "entry.relic_counters",
            "{bad}"
        );
    }
    for good in [json!(3), json!({"CardsExhausted": 0, "EtherealCount": 0})] {
        let mut shaped = valid();
        shaped["entry"]["relic_counters"] = json!({ relic.clone(): good });
        assert!(parse(&shaped.to_string(), GameBuild::V0_111_0).is_ok());
    }
    // Lizard Tail's `WasUsed` and Maw Bank's `HasItemBeenBought` (#3328) are
    // the two boolean counters.
    for boolean in ["RELIC.LIZARD_TAIL", "RELIC.MAW_BANK"] {
        let mut owner = valid();
        owner["entry"]["relics_entering"]
            .as_array_mut()
            .unwrap()
            .push(json!(boolean));
        owner["entry"]["relic_counters"] = json!({ boolean: false });
        assert!(
            parse(&owner.to_string(), GameBuild::V0_111_0).is_ok(),
            "{boolean}"
        );
    }

    for (key, field) in [
        ("tea_set_charged", "entry.tea_set_charged"),
        ("fake_tea_set_charged", "entry.fake_tea_set_charged"),
    ] {
        let mut tea = valid();
        tea["entry"][key] = json!(true);
        assert_eq!(field_of(refusal_of(&tea)), field, "{key}");
    }
    let mut owned_tea = valid();
    owned_tea["entry"]["relics_entering"]
        .as_array_mut()
        .unwrap()
        .push(json!("RELIC.VENERABLE_TEA_SET"));
    owned_tea["entry"]["tea_set_charged"] = json!(true);
    assert_eq!(
        parse(&owned_tea.to_string(), GameBuild::V0_111_0)
            .unwrap()
            .relic_entry
            .tea_set_charged,
        Some(true)
    );

    let mut fur = valid();
    fur["entry"]["fur_coat_active"] = json!(false);
    assert_eq!(field_of(refusal_of(&fur)), "entry.fur_coat_active");
    let mut ambiguous = valid();
    ambiguous["entry"]["entry_ambiguous"] = json!(true);
    assert_eq!(field_of(refusal_of(&ambiguous)), "entry.entry_ambiguous");
}

#[test]
fn node_facts_are_checked() {
    let mut negative = valid();
    negative["entry"]["node_index"] = json!(-1);
    assert_eq!(field_of(refusal_of(&negative)), "entry.node_index");
    let mut empty = valid();
    empty["entry"]["node_type"] = json!("");
    assert_eq!(field_of(refusal_of(&empty)), "entry.node_type");
    // `null` is the save path's `None`, which `to_json` renders as "monster".
    let mut absent = valid();
    absent["entry"]["node_type"] = Value::Null;
    assert_eq!(
        parse(&absent.to_string(), GameBuild::V0_111_0)
            .unwrap()
            .node_type,
        None
    );
}

#[test]
fn streams_are_run_streams_with_the_required_six_and_matching_counters() {
    let mut unknown_stream = valid();
    let shuffle = unknown_stream["streams"]["shuffle"].clone();
    unknown_stream["streams"]["rewards"] = shuffle;
    assert_eq!(
        refusal_of(&unknown_stream),
        EntryRefusal::UnknownRngStream("rewards".to_string())
    );
    let mut missing = valid();
    missing["streams"].as_object_mut().unwrap().remove("niche");
    missing["counters"].as_object_mut().unwrap().remove("niche");
    assert_eq!(
        refusal_of(&missing),
        EntryRefusal::MissingRngStream("niche")
    );
    let mut counters = valid();
    counters["counters"]["shuffle"] = json!(1_000);
    assert_eq!(field_of(refusal_of(&counters)), "counters");
    let mut not_a_map = valid();
    not_a_map["streams"] = json!([]);
    assert_eq!(refusal_of(&not_a_map).class(), "unknown_facts_field");
}

#[test]
fn stream_words_must_be_the_seed_derived_state_at_their_counter() {
    let mut tampered = valid();
    let word = tampered["streams"]["niche"]["s0"].as_u64().unwrap();
    tampered["streams"]["niche"]["s0"] = json!(word ^ 1);
    assert_eq!(
        refusal_of(&tampered),
        EntryRefusal::FactsStreamDisagreesWithSeed {
            stream: "niche".to_string()
        }
    );
    // Advancing the counter without advancing the words also disagrees.
    let mut advanced = valid();
    advanced["streams"]["niche"]["counter"] = json!(1);
    advanced["counters"]["niche"] = json!(1);
    assert_eq!(
        refusal_of(&advanced).class(),
        "facts_stream_disagrees_with_seed"
    );
    // A counter the check will not walk refuses by name.
    let mut far = valid();
    far["streams"]["niche"]["counter"] = json!(MAX_VERIFIED_STREAM_COUNTER + 1);
    far["counters"]["niche"] = json!(MAX_VERIFIED_STREAM_COUNTER + 1);
    assert_eq!(
        refusal_of(&far).class(),
        "facts_stream_counter_unverifiable"
    );
}

/// The derivation, checked against `sts2_rng.RunRngSet` vectors the oracle
/// printed (not re-derived from the same expression):
///
/// ```text
/// cd versions/v0.111.0/solver && python3.12 -c "
/// import sts2_rng as r
/// rs = r.RunRngSet('COACHV1000000', {'Niche': 3}, build='v0.111.0')
/// for n in ('Shuffle', 'Niche'):
///     m = rs.rngs[n]._random; print(n, rs.rngs[n].counter, m.s0, m.s1, m.s2, m.s3)"
/// ```
#[test]
fn the_seed_check_agrees_with_the_oracle_run_rng_set() {
    for (name, counter, words) in [
        (
            "shuffle",
            0,
            [
                8153009640228836705_u64,
                11614080326854330087,
                6758263434890465332,
                14283085356205178616,
            ],
        ),
        (
            "niche",
            3,
            [
                15582291813005773327,
                11975374115168942981,
                16150209915060383249,
                16257945270987892141,
            ],
        ),
    ] {
        let state = SerializedRng {
            counter,
            s0: words[0],
            s1: words[1],
            s2: words[2],
            s3: words[3],
        };
        verify_against_seed("COACHV1000000", name, &state)
            .unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
    }
}

#[test]
fn the_opening_block_is_the_entry_documents_own() {
    let mut built = valid();
    built["opening"]["built"] = json!(true);
    assert_eq!(field_of(refusal_of(&built)), "opening");
    let mut spliced = valid();
    spliced["opening"]["mcr_splice_applied"] = json!(true);
    assert_eq!(field_of(refusal_of(&spliced)), "opening");
    let mut absent = valid();
    absent.as_object_mut().unwrap().remove("opening");
    assert!(parse(&absent.to_string(), GameBuild::V0_111_0).is_ok());
    let mut refused = valid();
    refused["opening"]["refusal_class"] = json!("some_class");
    assert!(parse(&refused.to_string(), GameBuild::V0_111_0).is_ok());
    let mut odd = valid();
    odd["opening"]["refusal_class"] = json!(1);
    assert_eq!(refusal_of(&odd).class(), "unknown_facts_field");
}

#[test]
fn epochs_must_be_in_the_save_paths_normal_form() {
    for bad in [
        json!(["IRONCLAD3_EPOCH", "IRONCLAD2_EPOCH"]),
        json!(["IRONCLAD2_EPOCH", "IRONCLAD2_EPOCH"]),
        json!(["EPOCH.IRONCLAD2_EPOCH"]),
        json!([""]),
    ] {
        let mut value = valid();
        value["unlocks"]["unlocked_card_pool_epochs"] = bad.clone();
        assert_eq!(
            field_of(refusal_of(&value)),
            "unlocks.unlocked_card_pool_epochs",
            "{bad}"
        );
    }
    let mut not_a_list = valid();
    not_a_list["unlocks"]["unlocked_card_pool_epochs"] = json!("IRONCLAD2_EPOCH");
    assert_eq!(refusal_of(&not_a_list).class(), "unknown_facts_field");
    let mut full = valid();
    full["unlocks"]["unlocked_card_pool_epochs"] = json!(["IRONCLAD2_EPOCH", "SILENT1_EPOCH"]);
    full["unlocks"]["fully_unlocked_card_pool"] = json!(true);
    full["unlocks"]["fully_unlocked_potion_pool"] = json!(true);
    let parsed = parse(&full.to_string(), GameBuild::V0_111_0).unwrap();
    assert_eq!(parsed.fully_unlocked_potion_pool, Some(true));
}

/// A facts document that cannot vouch for relic dispatch order (the legacy
/// `.run` path, #2827 C2) still opens. `State.relics_entering_dispatch_ordered`
/// defaults to `False` and the boundary refuses an explicit `false` as a
/// non-canonical default, so the opening elides it rather than writing it;
/// a vouched order is still written as `true`.
#[test]
fn an_unvouched_relic_order_opens_with_the_flag_elided() {
    let vouched = valid();
    assert_eq!(
        vouched["entry"]["relics_entering_dispatch_ordered"],
        json!(true)
    );
    let mut unvouched = vouched.clone();
    unvouched["entry"]["relics_entering_dispatch_ordered"] = json!(false);
    for (value, expected) in [(vouched, Some(json!(true))), (unvouched, None)] {
        let parsed = parse(&value.to_string(), GameBuild::V0_111_0).unwrap();
        let root = crate::entry::root_from_document(parsed, None).to_json();
        assert_eq!(root["schema"], json!("sts-sim-canonical-v2"), "{root}");
        assert_eq!(
            root["player"]
                .get("relics_entering_dispatch_ordered")
                .cloned(),
            expected
        );
    }
}

/// #3248 on the real builders: the save path applies the event allow-list and
/// the facts path refuses every event encounter.
///
/// The builder fixture's save (Ironclad, Burning Blood, 68 of 80 HP) entered as
/// Dense Vegetation heals by `MimicRestSiteHeal` (`floor(0.3 * 80) = 24`, capped
/// at 80); with Tiny Mailbox added, as ZU7KKADQBNCR node 10 held, it refuses on
/// the rest heal's rewards; entered as a Battleworn Dummy it keeps the saved
/// HP; a run modifier and the Fake Merchant refuse it.
#[test]
fn event_roots_are_admitted_only_through_the_allow_list() {
    let (save, _, node_type) = saves().swap_remove(1);
    let dv = "ENCOUNTER.DENSE_VEGETATION_EVENT_ENCOUNTER";
    let bw = "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V2_ENCOUNTER";
    let refused = |save: &str, encounter: &str| match build(&request(save, encounter, &node_type)) {
        EntryOutcome::Refused(refusal) => refusal,
        EntryOutcome::Built(_) => panic!("{encounter} refuses"),
    };

    let mut low: Value = serde_json::from_str(&save).unwrap();
    low["players"][0]["current_hp"] = json!(8);
    let low = low.to_string();
    assert_eq!(document_of(&low, dv, &node_type).hp_entering, 8 + 24);
    assert_eq!(document_of(&save, dv, &node_type).hp_entering, 80);
    assert_eq!(document_of(&low, bw, &node_type).hp_entering, 8);
    assert_eq!(
        document_of(&low, "ENCOUNTER.TOADPOLES_WEAK", &node_type).hp_entering,
        8
    );

    let mut mailbox: Value = serde_json::from_str(&low).unwrap();
    let relics = mailbox["players"][0]["relics"].as_array_mut().unwrap();
    let mut row = relics[0].clone();
    row["id"] = json!("RELIC.TINY_MAILBOX");
    relics.push(row);
    let mailbox = mailbox.to_string();
    let refusal = refused(&mailbox, dv);
    assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
    assert!(refusal.to_string().contains("RELIC.TINY_MAILBOX"));
    assert_eq!(document_of(&mailbox, bw, &node_type).hp_entering, 8);

    // #3351, #3353: the room-entry listeners whose body is a no-op on the
    // `EventRoom` build through the real save builder, each on its own. The
    // two that also listen on the rest heal still refuse Dense Vegetation.
    let knight = "ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER";
    let rest_listener = |id: &str| {
        crate::entry::event_combat::REST_HEAL_LISTENERS
            .iter()
            .any(|(_, rest, _)| *rest == id)
    };
    for (_, id, _) in crate::entry::event_combat::EVENT_ROOM_INERT_LISTENERS {
        let mut held: Value = serde_json::from_str(&low).unwrap();
        let relics = held["players"][0]["relics"].as_array_mut().unwrap();
        let mut row = relics[0].clone();
        row["id"] = json!(id);
        relics.push(row);
        let held = held.to_string();
        assert_eq!(
            document_of(&held, knight, &node_type).hp_entering,
            8,
            "{id}"
        );
        assert_eq!(document_of(&held, bw, &node_type).hp_entering, 8, "{id}");
        if rest_listener(id) {
            let refusal = refused(&held, dv);
            assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
            assert!(refusal.to_string().contains(id), "{id}");
        } else {
            assert_eq!(document_of(&held, dv, &node_type).hp_entering, 32, "{id}");
        }
    }
    // #3353: a listener whose `EventRoom` body acts still refuses beside the
    // inert ones.
    let mut maw: Value = serde_json::from_str(&low).unwrap();
    let relics = maw["players"][0]["relics"].as_array_mut().unwrap();
    for id in ["RELIC.VAJRA", "RELIC.MAW_BANK"] {
        let mut row = relics[0].clone();
        row["id"] = json!(id);
        relics.push(row);
    }
    let refusal = refused(&maw.to_string(), knight);
    assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
    assert!(refusal.to_string().contains("RELIC.MAW_BANK"));

    let mut modded: Value = serde_json::from_str(&low).unwrap();
    modded["modifiers"] = json!([{"id": "MODIFIER.NIGHT_TERRORS"}]);
    let refusal = refused(&modded.to_string(), bw);
    assert_eq!(refusal.class(), "event_combat_run_modifiers");

    let refusal = refused(&low, "ENCOUNTER.FAKE_MERCHANT_EVENT_ENCOUNTER");
    assert_eq!(refusal.class(), "event_combat_path_not_modeled");

    let mut facts = valid();
    facts["entry"]["encounter_id"] = json!(bw);
    assert_eq!(refusal_of(&facts).class(), "event_combat_facts_unvouched");
}
