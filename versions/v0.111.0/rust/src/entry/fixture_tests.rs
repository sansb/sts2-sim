//! The acceptance fixture as a live guard, not a document.
//!
//! `fixtures/entry_builder_v1.json` (#2511) records the IL surface of the save
//! writer, the corpus field census, the nine-stream map, and a complete
//! synthetic schema-20 save with the digest it roots to. These tests make the
//! builder's own vocabularies answer to it, so a table edited on one side and
//! not the other goes red rather than drifting.
//!
//! Everything here is `include_str!`'d, so the tests need no filesystem, no
//! capture corpus, and no game install — they run on any checkout.

use serde_json::Value;

use crate::catalog::GameBuild;
use crate::entry::counters::COMBAT_STREAMS;
use crate::entry::props::SAVED_PROPERTY_GROUPS;
use crate::entry::save::{ADMITTED_SAVE_SCHEMA, MAP_POINT_TYPES, RUN_RNG_STREAMS};
use crate::entry::{EntryOutcome, EntryRequest, build};

/// The v0.111.0 assembly every IL citation in this module tree names.
pub const DLL_SHA256: &str = "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4";

const FIXTURE: &str = include_str!("../../fixtures/entry_builder_v1.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("the entry-builder fixture is JSON")
}

#[test]
fn the_fixture_names_the_assembly_every_citation_rests_on() {
    let value = fixture();
    assert_eq!(value["dll"]["sha256"], Value::from(DLL_SHA256));
    assert_eq!(
        value["dll"]["build"],
        Value::from(GameBuild::V0_111_0.as_str())
    );
    assert_eq!(
        value["save_schema_version"],
        Value::from(ADMITTED_SAVE_SCHEMA)
    );
}

#[test]
fn the_nine_stream_map_agrees_with_the_fixture() {
    let value = fixture();
    let combat = value["stream_map"]["combat"]
        .as_object()
        .expect("stream_map.combat");
    assert_eq!(combat.len(), COMBAT_STREAMS.len());
    for (save_name, canonical) in COMBAT_STREAMS {
        assert_eq!(
            combat.get(save_name).and_then(Value::as_str),
            Some(canonical),
            "stream {save_name} maps differently in the fixture"
        );
    }
}

#[test]
fn the_taught_run_stream_set_is_the_one_the_corpus_census_saw() {
    let value = fixture();
    let observed = value["save_field_surface"]["run_rng_streams"]
        .as_object()
        .expect("run_rng_streams");
    let mut names: Vec<&str> = observed.keys().map(String::as_str).collect();
    names.sort_unstable();
    let mut taught: Vec<&str> = RUN_RNG_STREAMS.to_vec();
    taught.sort_unstable();
    assert_eq!(names, taught);
}

#[test]
fn every_map_point_type_the_corpus_saw_is_taught() {
    let value = fixture();
    let observed = value["save_field_surface"]["saved_map_point_types"]
        .as_object()
        .expect("saved_map_point_types");
    for kind in observed.keys() {
        assert!(
            MAP_POINT_TYPES.contains(&kind.as_str()),
            "corpus map point type {kind:?} is not in MapPointType"
        );
    }
}

#[test]
fn every_saved_property_group_the_corpus_saw_is_taught() {
    let value = fixture();
    let observed = value["save_field_surface"]["props_groups"]
        .as_object()
        .expect("props_groups");
    for group in observed.keys() {
        assert!(
            SAVED_PROPERTY_GROUPS.contains(&group.as_str()),
            "corpus SavedProperties group {group:?} is not one of the seven"
        );
    }
    // The IL declares seven and the corpus exercises five; the two unobserved
    // ones must still be taught (Fur Coat's latch is an `int_arrays` pair).
    assert_eq!(SAVED_PROPERTY_GROUPS.len(), 7);
    assert!(observed.len() < SAVED_PROPERTY_GROUPS.len());
}

#[test]
fn the_seeding_falsification_is_recorded_and_clean() {
    // The reason this module reads recorded stream state instead of deriving
    // it. If the fixture ever records a disagreeing save, the builder's
    // "read, never derive" premise needs re-examining before it is trusted.
    let value = fixture();
    let falsification = &value["seeding_falsification"];
    assert_eq!(
        falsification["saves_with_a_disagreeing_stream"],
        Value::from(0)
    );
    assert!(
        falsification["schema_20_saves_checked"]
            .as_u64()
            .is_some_and(|checked| checked >= 3000)
    );
}

#[test]
fn the_synthetic_save_builds_the_entry_facts_the_fixture_expects() {
    let value = fixture();
    let case = &value["cases"][0];
    let save_text = serde_json::to_string(&case["save"]).expect("the case's save");
    let outcome = build(&EntryRequest {
        input: crate::entry::EntryInput::Save(&save_text),
        encounter_id: case["encounter_id"].as_str(),
        node_type: case["node_type"].as_str(),
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    });
    let EntryOutcome::Built(document) = outcome else {
        panic!("the synthetic fixture save must build: {outcome:?}");
    };
    let built = document.to_json();
    let entry = &built["entry"];
    assert_eq!(entry["encounter_id"], case["encounter_id"]);
    assert_eq!(entry["node_type"], case["node_type"]);
    assert_eq!(entry["node_index"], case["node_index"]);

    // The two fields §2(b) measured as the ONLY difference between a
    // save-rooted document and the hand-entry-rooted pinned fixture. Both come
    // from this half of `start_combat`, so both are decided here.
    let witnesses = &case["expected_witnesses"];
    assert_eq!(
        entry["max_potion_slot_count"],
        Value::from(
            witnesses["player.potion_slots"]
                .as_array()
                .expect("potion_slots witness")
                .len()
        )
    );
    assert_eq!(
        entry["relics_entering_dispatch_ordered"],
        witnesses["player.relics_entering_dispatch_ordered"]
    );
    assert!(
        entry["potion_slots_entering"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    // The deal is over the entering deck in save-array order, so the hand
    // witness has to be drawn from these ids.
    let deck: Vec<String> = entry["deck_entering"]
        .as_array()
        .expect("deck_entering")
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .expect("a card id")
                .trim_start_matches("CARD.")
                .to_string()
        })
        .collect();
    for card in witnesses["piles.hand"].as_array().expect("hand witness") {
        assert!(
            deck.contains(&card.as_str().expect("a hand card").to_string()),
            "hand witness {card} is not in the entering deck"
        );
    }

    // The synthetic save omits `combat_orbs`, which the oracle reads with
    // `.get`; the entry carries what the save records and nothing more.
    let counters = built["counters"].as_object().expect("counters");
    assert!(!counters.contains_key("combat_orbs"));
    assert_eq!(counters["shuffle"], Value::from(0));
    assert_eq!(built["seed"], case["save"]["rng"]["seed"]);
}

#[test]
fn the_statically_derivable_refusal_names_are_all_emittable() {
    // The fixture lists the refusal names derivable before the corpus run.
    // Each must be a class this builder can actually produce, or the
    // vocabulary and the spec have drifted.
    let value = fixture();
    let cases = value["refusal_cases"].as_object().expect("refusal_cases");
    let emittable = [
        "unsupported_save_schema",
        "multiplayer_save",
        "missing_map_point_history",
        "unknown_node_type",
        "no_encounter_for_node",
        "saved_property_group_unhandled",
    ];
    for name in cases.keys() {
        if name == "note" {
            continue;
        }
        if name == "card_owner_provenance" {
            // Deliberately NOT a variant of this slice's vocabulary: whether a
            // generator needs an owner is decided inside the run-RNG opening,
            // which this slice does not build. Declaring an unreachable
            // variant would promise a refusal nothing can raise.
            continue;
        }
        assert!(
            emittable.contains(&name.as_str()),
            "fixture names refusal {name:?}, which this builder cannot emit"
        );
    }
}
