//! The opening, measured against the oracle's own save-rooted document.
//!
//! # The comparison, and why it is stated this way
//!
//! Acceptance (a) for E4 is *"Rust's save-rooted canonical v2 document is
//! digest-identical to **Python's** save-rooted document on the same save"* —
//! never "equals the checked-in fixture", because the pinned
//! `fixtures/canonical_state_v2_ironclad_toadpoles.json` was rooted from a
//! hand-written entry dict rather than from a save
//! (`fixtures/entry_builder_v1.json`,
//! `cases[0].relation_to_the_pinned_canonical_fixture`).
//!
//! `fixtures/entry_builder_v1.json` records exactly that comparison: a
//! complete synthetic schema-20 save and the digest **Python's**
//! `start_from_save` roots it to. `PYTHON_SAVE_ROOTED` below is that document,
//! and every assertion here is against it rather than against a number typed
//! into a test.
//!
//! Nothing here registers a roster builder. `TOADPOLES` is
//! `encounters::weak::build_toadpoles`, landed by #2531's F1 lane and reached
//! through the generated `content_tables::ENCOUNTER_ROSTER_BUILDERS` and the
//! substring dispatch in [`super::roster`] — so this is the production path
//! end to end: save bytes in, canonical document out.

use serde_json::Value;

use crate::catalog::GameBuild;
use crate::entry::opening::refusal::OpeningRefusal;
use crate::entry::opening::{
    Opening, OpeningOptions, PreHook, build as build_opening, build_pre_hook,
};
use crate::entry::{EntryOutcome, EntryRequest, build as build_entry};

const FIXTURE: &str = include_str!("../../../fixtures/entry_builder_v1.json");

/// Python's save-rooted canonical v2 document for the fixture's Toadpoles
/// case, reproduced from the oracle rather than transcribed:
///
/// ```text
/// cd sim/v0.111.0/engine && python3.12 -c "
/// import json, sys; sys.path[:0] = ['tools', '../solver']
/// import project_state as projector, mcr_replay
/// case = json.load(open('fixtures/entry_builder_v1.json'))['cases'][0]
/// st = mcr_replay.start_from_save(case['save'], case['encounter_id'],
///                                 case['node_type'], build='v0.111.0')
/// doc = projector.project_state(st, game_build='v0.111.0')
/// print(projector.differential_digest(doc))
/// print(json.dumps(doc, sort_keys=True))"
/// ```
///
/// The first line is `cases[0].expected_entry_digest`, asserted below, so this
/// constant cannot drift from the fixture without a test going red.
const PYTHON_SAVE_ROOTED: &str =
    include_str!("../../../fixtures/python_save_rooted_toadpoles.json");

fn case() -> Value {
    serde_json::from_str::<Value>(FIXTURE).expect("the entry-builder fixture is JSON")["cases"]
        .as_array()
        .expect("the fixture carries cases")
        .iter()
        .find(|case| case["name"].as_str() == Some("ironclad_starter_vs_toadpoles"))
        .expect("the Toadpoles case")
        .clone()
}

fn oracle() -> Value {
    serde_json::from_str(PYTHON_SAVE_ROOTED).expect("the oracle document is JSON")
}

fn entry_facts(case: &Value) -> crate::entry::document::EntryDocument {
    let save = serde_json::to_string(&case["save"]).expect("the synthetic save serializes");
    match build_entry(&EntryRequest {
        input: crate::entry::EntryInput::Save(&save),
        encounter_id: case["encounter_id"].as_str(),
        node_type: case["node_type"].as_str(),
        game_build: GameBuild::V0_111_0,
        mcr_splice: false,
    }) {
        EntryOutcome::Built(entry) => *entry,
        outcome => panic!("the synthetic save builds its entry facts: {outcome:?}"),
    }
}

fn pre_hook(case: &Value) -> Result<PreHook, OpeningRefusal> {
    build_pre_hook(&entry_facts(case))
}

/// The frozen oracle's digest of an opening with combat-start pets (#3039).
///
/// The oracle never tracked native `CombatState._nextCreatureId`, which the
/// pets advance (`engine::seed_creature_uid_counter`), so the Rust document
/// carries one field it could not. Check that field, then digest the rest,
/// which is still the oracle's document byte for byte.
fn oracle_pet_digest(
    document: &crate::canonical::CanonicalStateV2,
    next_creature_uid: i64,
) -> String {
    let mut document = document.clone();
    assert_eq!(
        document.player.remove("next_creature_uid"),
        Some(Value::from(next_creature_uid))
    );
    document.differential_digest()
}

/// The opening as the frozen oracle documents record it.
///
/// The oracle never carried the AfterEnergyReset ledger: its review root
/// stamped `after_energy_reset_order` onto the document afterwards (frozen
/// Python `rust_review._replay_opening`, deleted #2827). The opening emits
/// the ledger itself since #3687, so every oracle digest and document here
/// compares without it. [`open_with_reset_ledger`] is the unstripped opening,
/// and the tests that use it pin the ledger.
fn open(case: &Value) -> Result<Opening, OpeningRefusal> {
    let mut opening = open_with_reset_ledger(case)?;
    opening.document.player.remove("after_energy_reset_order");
    Ok(opening)
}

fn open_with_reset_ledger(case: &Value) -> Result<Opening, OpeningRefusal> {
    build_opening(&entry_facts(case), &OpeningOptions::default())
}

#[test]
fn the_checked_in_oracle_document_is_the_fixtures_own() {
    // The whole comparison rests on this constant being the oracle's answer
    // for this save, so it is checked against the digest the fixture records
    // rather than trusted.
    let document: crate::canonical::CanonicalStateV2 =
        serde_json::from_str(PYTHON_SAVE_ROOTED).expect("the oracle document parses as v2");
    assert_eq!(
        Value::from(document.differential_digest()),
        case()["expected_entry_digest"]
    );
}

#[test]
fn the_opening_shuffle_reproduces_the_oracles_pile_order() {
    // The oracle's post-deal document splits the shuffled pile across `hand`
    // and `draw`; concatenating them in `_ALL_CARD_PILES` order recovers it.
    let built = pre_hook(&case()).expect("the pre-hook opening builds");
    let oracle = oracle();
    let expected: Vec<&str> = oracle["piles"]["hand"]
        .as_array()
        .unwrap()
        .iter()
        .chain(oracle["piles"]["draw"].as_array().unwrap())
        .map(|card| card["id"].as_str().unwrap())
        .collect();
    let observed: Vec<&str> = built.document.piles["draw"]
        .iter()
        .map(|card| card.id.as_str())
        .collect();
    assert_eq!(observed, expected);
}

#[test]
fn creation_reproduces_the_oracles_roster_and_hp_rolls() {
    // Through the PRODUCTION registry: `TOADPOLES` is F1's
    // `encounters::weak::build_toadpoles`, reached by the substring dispatch.
    let built = pre_hook(&case()).expect("the pre-hook opening builds");
    let oracle = oracle();
    let observed = Value::Array(
        built
            .document
            .monsters
            .iter()
            .map(|monster| Value::Object(monster.clone().into_iter().collect()))
            .collect(),
    );
    assert_eq!(observed, oracle["monsters"]);
}

#[test]
fn the_opening_consumes_exactly_the_draws_the_oracle_does() {
    // I6, and the #2527 pre-splice net in miniature. The oracle's document is
    // post-deal, and the deal draws from an already-shuffled pile without
    // touching a stream, so every one of the nine must already agree here.
    let built = pre_hook(&case()).expect("the pre-hook opening builds");
    let oracle = oracle();
    let observed: Value = serde_json::to_value(&built.rng).expect("streams serialize");
    assert_eq!(observed, oracle["rng"]);
    // Named separately so a failure says which half moved: nine `Shuffle`
    // draws for a ten-card deck, two `Niche` draws for a two-monster roster.
    assert_eq!(built.rng["rng"].counter, 9);
    assert_eq!(built.rng["niche"].counter, 2);
}

#[test]
fn every_player_field_the_opening_writes_agrees_with_the_oracle() {
    // The post-deal fields (`cards_drawn_combat`, `next_card_uid`,
    // `player_phase`) are the engine's and are absent here by construction;
    // everything else the opening computes is compared value for value, in
    // both directions, so a field this builder invents fails as loudly as one
    // it omits.
    const POST_DEAL: [&str; 3] = ["cards_drawn_combat", "next_card_uid", "player_phase"];
    let built = pre_hook(&case()).expect("the pre-hook opening builds");
    let oracle = oracle();
    let oracle_player = oracle["player"].as_object().expect("player");
    for (name, value) in oracle_player {
        if POST_DEAL.contains(&name.as_str()) {
            continue;
        }
        assert_eq!(
            built.document.player.get(name),
            Some(value),
            "player.{name} differs from the oracle"
        );
    }
    // The one field the opening writes that the oracle document never held:
    // the AfterEnergyReset ledger's known-empty witness (#3687), which the
    // oracle's review root stamped on afterwards. Pinned by value instead.
    const RUST_ONLY: &str = "after_energy_reset_order";
    assert!(!oracle_player.contains_key(RUST_ONLY));
    assert_eq!(
        built.document.player.get(RUST_ONLY),
        Some(&Value::Array(Vec::new()))
    );
    for name in built.document.player.keys() {
        assert!(
            oracle_player.contains_key(name)
                || POST_DEAL.contains(&name.as_str())
                || name == RUST_ONLY,
            "player.{name} is written by the opening and absent from the oracle"
        );
    }
}

/// The post-opening half stops at a **pre-existing** boundary narrowing, and
/// this test pins that it is pre-existing rather than something the opening
/// introduced.
///
/// `HotBoundary::from_canonical` refuses any state with a materialised potion
/// belt and no proof of the generation pool: *"exact current-build
/// potion-pool provenance is required"* (`boundary.rs`, `hydrate_potion_belt`).
/// Since #3347 either `fully_unlocked_potion_pool` or a recorded
/// `splash_unlock_epochs` profile is that proof. A schema-20 save records
/// `max_potion_slot_count`, so the belt materialises as `[null, null]`, and
/// this fixture's `unlock_state.unlocked_epochs` is empty, so the flag is
/// false and the opening elides the empty profile: neither proof holds.
///
/// The second half of the test is the load-bearing half: the oracle's **own**
/// document is refused by the same gate. Measured independently on the real
/// binary on 2026-09-16 — feeding it to `sts-sim diff-serve` returns exactly
/// this refusal — so this is the crate's standing admission ceiling and not an
/// opening defect. Widening it is a boundary decision with its own
/// justification, not something an engine lane takes in passing.
#[test]
fn the_post_deal_document_stops_at_the_existing_potion_belt_boundary() {
    let refusal = open(&case()).unwrap_err();
    assert_eq!(refusal.class(), "boundary_unrepresentable");
    assert!(
        refusal.to_string().contains("fully_unlocked_potion_pool"),
        "{refusal}"
    );

    let oracle: crate::canonical::CanonicalStateV2 =
        serde_json::from_str(PYTHON_SAVE_ROOTED).expect("the oracle document parses as v2");
    let catalog =
        crate::boundary::HotBoundary::catalog_from_canonical(&oracle).expect("catalog builds");
    let oracle_refusal = crate::boundary::HotBoundary::from_canonical(&oracle, &catalog)
        .expect_err("the oracle's own document is refused by the same gate");
    assert!(
        format!("{oracle_refusal:?}").contains("fully_unlocked_potion_pool"),
        "{oracle_refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// The high-tier review findings on #2544, each pinned against the oracle
// ---------------------------------------------------------------------------
//
// These four cases need a state the boundary actually admits, and the fixture
// save's materialised two-slot potion belt is the standing ceiling the test
// above pins. `max_potion_slot_count: 0` empties the belt without touching
// anything else in the save, and on that variant Rust and Python agree
// end to end — which is what makes the digests below a comparison and not a
// transcription. Every expected value here was printed by the oracle:
//
// ```text
// cd sim/v0.111.0/engine && python3.12 -c "
// import copy, json, sys; sys.path[:0] = ['tools', '../solver']
// import project_state as projector, mcr_replay
// case = [c for c in json.load(open('fixtures/entry_builder_v1.json'))['cases']
//         if c['name'] == 'ironclad_starter_vs_toadpoles'][0]
// save = copy.deepcopy(case['save'])
// save['players'][0]['max_potion_slot_count'] = 0
// # ... the per-case mutation named in each test ...
// st = mcr_replay.start_from_save(save, case['encounter_id'], case['node_type'],
//                                 build='v0.111.0')
// doc = projector.project_state(st, game_build='v0.111.0')
// print(projector.differential_digest(doc)); print(json.dumps(doc, sort_keys=True))"
// ```

/// The fixture case with an empty potion belt, so the opening reaches the
/// post-deal document instead of the potion-pool boundary ceiling.
fn empty_belt_case() -> Value {
    let mut case = case();
    case["save"]["players"][0]["max_potion_slot_count"] = Value::from(0);
    case
}

/// The oracle's digest for [`empty_belt_case`] untouched — the control the
/// three mutated cases below are read against.
const EMPTY_BELT_DIGEST: &str = "5b8097299f56febb786e4958e197784d1bdfb5c7adb0899eacb5ff9b71c1c904";

fn deck_row(case: &mut Value, row: Value) {
    case["save"]["players"][0]["deck"]
        .as_array_mut()
        .expect("the save's deck is an array")
        .insert(0, row);
}

fn relic(case: &mut Value, row: Value) {
    case["save"]["players"][0]["relics"]
        .as_array_mut()
        .expect("the save's relics are an array")
        .push(row);
}

fn card_named(document: &crate::canonical::CanonicalStateV2, id: &str) -> Vec<Value> {
    document
        .piles
        .values()
        .flatten()
        .filter(|card| card.id == id)
        .map(|card| serde_json::to_value(card).expect("a canonical card serializes"))
        .collect()
}

#[test]
fn the_empty_belt_variant_opens_end_to_end_and_matches_the_oracle() {
    // The control. Without it the three pins below could pass for the wrong
    // reason — a shared upstream divergence that happens to cancel.
    let opening = open(&empty_belt_case()).expect("the empty-belt variant opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

/// Gold Plated Cables opens (#3381): it has no body on a hook the opening
/// fires, only the orb-passive trigger-count modifier the engine already runs
/// (the IL is on `TURN_ONE_ORB_RELICS`). The opened document is the control's
/// except for the ownership mirrors. Infused Core, still in the table, still
/// refuses by name.
#[test]
fn gold_plated_cables_opens_and_changes_only_its_ownership_mirror() {
    let strip = |document: &crate::canonical::CanonicalStateV2| {
        let mut value = serde_json::to_value(document).unwrap();
        let player = value["player"].as_object_mut().unwrap();
        player.remove("gold_plated");
        player.remove("relics_entering");
        value
    };
    let control = open(&empty_belt_case()).expect("the control opens");
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.GOLD_PLATED_CABLES"}),
    );
    let opening = open(&case).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    assert_eq!(
        opening.document.player.get("gold_plated"),
        Some(&Value::Bool(true))
    );
    assert_eq!(strip(&opening.document), strip(&control.document));

    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.INFUSED_CORE"}),
    );
    assert_eq!(
        open(&case).unwrap_err(),
        OpeningRefusal::RoomEntryRelicNotModeled {
            relic: "RELIC.INFUSED_CORE".to_string()
        }
    );
}

/// High-tier review finding P1-2 (#2544): a saved Genetic Algorithm copy's
/// growth and deck-row link were silently dropped.
///
/// Mutation: `save['players'][0]['deck'].insert(0, {'floor_added_to_deck': 1,
/// 'id': 'CARD.GENETIC_ALGORITHM', 'props': {'ints': [
/// {'name': 'CurrentBlock', 'value': 4},
/// {'name': 'IncreasedBlock', 'value': 3}]}})`.
#[test]
fn a_saved_genetic_algorithm_copy_carries_its_growth_and_deck_row() {
    let mut case = empty_belt_case();
    deck_row(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1,
            "id": "CARD.GENETIC_ALGORITHM",
            "props": {"ints": [
                {"name": "CurrentBlock", "value": 4},
                {"name": "IncreasedBlock", "value": 3}]}
        }),
    );
    let opening = open(&case).expect("a Genetic Algorithm deck opens");

    assert_eq!(
        card_named(&opening.document, "GENETIC_ALGORITHM"),
        vec![serde_json::json!({
            "id": "GENETIC_ALGORITHM",
            "upgrade": 0,
            "uid": 4,
            "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []],
            "extra": [["GENETIC_ALGORITHM_STATE", 3, 0]],
        })],
        "the copy must carry its saved growth and its DeckVersion row"
    );
    assert_eq!(
        opening.document.player.get("genetic_algorithm_deck_growth"),
        Some(&serde_json::json!([[0, 3]])),
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "160e0f48b211596818cc4fe9eaf2972f4ccc926345a40dcbe8d73c9bd818f63e"
    );
    // And it is a different document from the control, so the assertion above
    // is measuring the state rather than an elision that happens to match.
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

#[test]
fn a_genetic_algorithm_copy_outside_the_native_domain_refuses() {
    for props in [
        // No props at all.
        Value::Null,
        // `CurrentBlock != 1 + IncreasedBlock`.
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": 9},
            {"name": "IncreasedBlock", "value": 3}]}),
        // A negative growth.
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": -1},
            {"name": "IncreasedBlock", "value": -2}]}),
        // A stranger field in place of one of the two.
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": 4},
            {"name": "Stranger", "value": 3}]}),
        // Extreme values: `1 + IncreasedBlock` must refuse, not overflow.
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": i64::MIN},
            {"name": "IncreasedBlock", "value": i64::MAX}]}),
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": i64::MAX},
            {"name": "IncreasedBlock", "value": i64::MAX}]}),
    ] {
        let mut case = empty_belt_case();
        let mut row = serde_json::json!({
            "floor_added_to_deck": 1, "id": "CARD.GENETIC_ALGORITHM"});
        if !props.is_null() {
            row["props"] = props.clone();
        }
        deck_row(&mut case, row);
        let refusal = open(&case).expect_err("an inexact payload refuses");
        assert_eq!(refusal.class(), "card_entry_props_not_exact", "{props}");
    }
}

/// A save's Mad Science props, in the decoder's `TinkerTimeType`-first order.
fn mad_science_props(tinker_type: i64, rider: i64) -> Value {
    serde_json::json!({"ints": [
        {"name": "TinkerTimeType", "value": tinker_type},
        {"name": "TinkerTimeRider", "value": rider}]})
}

/// An empty-belt case whose deck gains one Mad Science copy per `(upgrade,
/// props)`.
fn mad_science_case(copies: &[(i64, Value)]) -> Value {
    let mut case = empty_belt_case();
    for (upgrade, props) in copies {
        deck_row(
            &mut case,
            serde_json::json!({
                "floor_added_to_deck": 1,
                "id": "CARD.MAD_SCIENCE",
                "current_upgrade_level": upgrade,
                "props": props,
            }),
        );
    }
    case
}

/// Every one of the eighteen saved variants (#2942), at both levels, opens as
/// its own card or refuses by name, and every opened document round-trips the
/// boundary with its Tinker row intact.
///
/// The ported seven bodies (Chaos since #3322) open: the copy carries exactly
/// one `["MAD_SCIENCE_TINKER", type, rider]` `extra` row and no slot-7
/// payload, the catalog built from the document records that fight variant,
/// and the Mad Science spec is the generated variant row (type, target, body,
/// Innate at level 1, Nimble eligibility from `get_GainsBlock`). The two
/// unported bodies refuse as `card_entry_variant_not_modeled`, naming the
/// rider.
#[test]
fn every_saved_mad_science_variant_opens_as_its_own_card_or_refuses_by_name() {
    use crate::content_tables::MAD_SCIENCE_VARIANT_ROWS;
    assert_eq!(MAD_SCIENCE_VARIANT_ROWS.len(), 18);
    let mut opened = 0;
    for variant in &MAD_SCIENCE_VARIANT_ROWS {
        let upgrade = MAD_SCIENCE_VARIANT_ROWS
            .iter()
            .position(|row| std::ptr::eq(row, variant))
            .unwrap()
            / 9;
        let mut case = mad_science_case(&[(
            upgrade as i64,
            mad_science_props(variant.tinker_type.into(), variant.rider.into()),
        )]);
        if variant.rider_name == "Chaos" {
            // Chaos is an owner-pool generator (#3322), which the opening
            // roots only for a recorded owner and unlock profile.
            case = owner_deck_with("CHARACTER.DEFECT", &[]);
            deck_row(
                &mut case,
                serde_json::json!({
                    "floor_added_to_deck": 1,
                    "id": "CARD.MAD_SCIENCE",
                    "current_upgrade_level": upgrade,
                    "props": mad_science_props(2, 6),
                }),
            );
        }
        let Some(row) = variant.row.as_ref() else {
            let refusal = open(&case).expect_err("an unported body refuses");
            assert_eq!(refusal.class(), "card_entry_variant_not_modeled");
            assert!(
                refusal.to_string().contains(variant.rider_name),
                "{refusal}"
            );
            continue;
        };
        opened += 1;
        let opening = open(&case)
            .unwrap_or_else(|refusal| panic!("{} L{upgrade} opens: {refusal}", variant.rider_name));
        let [ref copy] = card_named(&opening.document, "MAD_SCIENCE")[..] else {
            panic!("one Mad Science copy");
        };
        let mut shape = copy.clone();
        shape.as_object_mut().unwrap().remove("uid");
        assert_eq!(
            shape,
            serde_json::json!({
                "id": "MAD_SCIENCE",
                "upgrade": upgrade,
                "extra": [["MAD_SCIENCE_TINKER", variant.tinker_type, variant.rider]],
            }),
            "{} L{upgrade}",
            variant.rider_name
        );
        let catalog =
            crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
        assert_eq!(
            catalog.mad_science_variant(),
            Some(crate::catalog::MadScienceVariant {
                tinker_type: variant.tinker_type,
                rider: variant.rider,
            })
        );
        let spec = catalog
            .specs()
            .find(|spec| spec.identity.id == crate::ids::CardId::MadScience)
            .expect("the copy is interned");
        assert!(std::ptr::eq(spec.row, row), "the spec is the variant row");
        assert_eq!(spec.card_type as u8, variant.tinker_type);
        assert_eq!(
            spec.target_type,
            if variant.tinker_type == 1 {
                crate::catalog::CardTargetType::AnyEnemy
            } else {
                crate::catalog::CardTargetType::SelfTarget
            }
        );
        assert_eq!(
            spec.innate,
            upgrade == 1,
            "MadScience::OnUpgrade adds Innate"
        );
        assert_eq!(row.nimble_eligible, variant.tinker_type == 2);
        let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
            .unwrap_or_else(|refusal| panic!("{} L{upgrade}: {refusal}", variant.rider_name));
        crate::engine::admit(&opening.document, &state, &catalog).unwrap_or_else(|refusal| {
            panic!("{} L{upgrade} admits: {refusal}", variant.rider_name)
        });
        assert_eq!(
            crate::boundary::HotBoundary::to_canonical(&state, &catalog),
            opening.document,
            "{} L{upgrade}: the Tinker row round-trips",
            variant.rider_name
        );
    }
    assert_eq!(opened, 16, "eight ported bodies at two levels");
}

/// A Mad Science copy whose saved props are not a pair `TinkerTime` can
/// write refuses (#2942): the `entry_props_growth` shape rules, plus the legal
/// `(type, rider)` domain of `TinkerTime::ChooseRiderEffect`.
#[test]
fn a_mad_science_copy_outside_the_saved_domain_refuses() {
    for props in [
        // No props at all: `FromSerializable` would leave CardType.None.
        Value::Null,
        // Legal fields, illegal pairs: a rider of another type, RiderEffect
        // None, CardType None, a Status type, and out-of-range values.
        mad_science_props(1, 4),
        mad_science_props(3, 1),
        mad_science_props(2, 0),
        mad_science_props(0, 0),
        mad_science_props(4, 1),
        mad_science_props(1, 10),
        mad_science_props(-1, 1),
        mad_science_props(i64::MAX, i64::MIN),
        // A stranger name, a duplicated name, one row, a second key.
        serde_json::json!({"ints": [
            {"name": "TinkerTimeType", "value": 2},
            {"name": "Stranger", "value": 4}]}),
        serde_json::json!({"ints": [
            {"name": "TinkerTimeType", "value": 2},
            {"name": "TinkerTimeType", "value": 4}]}),
        serde_json::json!({"ints": [{"name": "TinkerTimeType", "value": 2}]}),
        serde_json::json!({
            "ints": [
                {"name": "TinkerTimeType", "value": 2},
                {"name": "TinkerTimeRider", "value": 4}],
            "bools": []}),
    ] {
        let mut case = empty_belt_case();
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.MAD_SCIENCE"});
        if !props.is_null() {
            row["props"] = props.clone();
        }
        deck_row(&mut case, row);
        let refusal = open(&case).expect_err("an inexact payload refuses");
        assert_eq!(refusal.class(), "card_entry_props_not_exact", "{props}");
    }
    // The rows arriving Rider-first is the same pair.
    let case = mad_science_case(&[(
        0,
        serde_json::json!({"ints": [
            {"name": "TinkerTimeRider", "value": 4},
            {"name": "TinkerTimeType", "value": 2}]}),
    )]);
    open(&case).expect("the row order is not the pair");
}

/// Each ported Mad Science body plays its own native program (#2942), one
/// witness per rider branch of `MadScience/<OnPlay>d__51::MoveNext`
/// (`0x3aaf08`): the type body (`ExecuteAttack` `0x3aa530`, `ExecuteSkill`
/// `0x3aae28`, `ExecutePower` `0x3aa660`), then `ExecuteRider` (`0x3aa9c4`)
/// only for Sapping, Choking, Energized and Wisdom. The numbers are the
/// canonical vars (Damage 12, ViolenceHits 3, SappingWeak/Vulnerable 2,
/// ChokingDamage 6, Block 8, EnergizedEnergy 2, WisdomCards 3,
/// ExpertiseStrength/Dexterity 2).
#[test]
fn each_ported_mad_science_body_plays_its_native_program() {
    use crate::hot::PileId;
    use crate::ids::{CardId, PowerId};
    let root = |tinker_type: i64, rider: i64| {
        let case = mad_science_case(&[(0, mad_science_props(tinker_type, rider))]);
        let (_document, catalog, mut state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
        // A target that survives every hit, so the witnesses read whole
        // amounts rather than a death clamp.
        for monster in state.monsters_mut() {
            monster.hp = 500;
            monster.max_hp = 500;
        }
        let uid = hand_card_uid(&state, &catalog, CardId::MadScience);
        (catalog, state, uid)
    };
    let pile_len = |state: &crate::hot::HotState, pile| state.piles.get(pile).as_slice().len();

    // Attack + Sapping: 12 damage, then Weak 2 and Vulnerable 2 on the target.
    let (catalog, state, uid) = root(1, 1);
    let after = play(&state, &catalog, uid, Some(0)).expect("Sapping plays");
    assert_eq!(state.monsters[0].hp - after.monsters[0].hp, 12);
    assert_eq!(after.monsters[0].powers.value(PowerId::Weak), 2);
    assert_eq!(after.monsters[0].powers.value(PowerId::Vuln), 2);
    // Attack + Violence: three hits of 12 and no rider.
    let (catalog, state, uid) = root(1, 2);
    let after = play(&state, &catalog, uid, Some(0)).expect("Violence plays");
    assert_eq!(state.monsters[0].hp - after.monsters[0].hp, 36);
    assert_eq!(after.monsters[0].powers.value(PowerId::Weak), 0);
    // Attack + Choking: 12 damage, then Strangle 6 on the target.
    let (catalog, state, uid) = root(1, 3);
    let after = play(&state, &catalog, uid, Some(0)).expect("Choking plays");
    assert_eq!(state.monsters[0].hp - after.monsters[0].hp, 12);
    assert_eq!(after.monsters[0].powers.value(PowerId::Strangle), 6);
    // An Attack variant needs its enemy target; the Skill and Power ones take
    // none.
    assert!(play(&state, &catalog, uid, None).is_err());

    // Skill + Energized: 8 Block, then 2 Energy (net +1 after the cost).
    let (catalog, state, uid) = root(2, 4);
    assert!(
        play(&state, &catalog, uid, Some(0)).is_err(),
        "a Self card takes no target"
    );
    let after = play(&state, &catalog, uid, None).expect("Energized plays");
    assert_eq!(after.block - state.block, 8);
    assert_eq!(after.energy - state.energy, 1);
    assert_eq!(
        pile_len(&after, PileId::Discard),
        pile_len(&state, PileId::Discard) + 1
    );
    // Skill + Wisdom: 8 Block, then a Draw of 3.
    let (catalog, state, uid) = root(2, 5);
    let after = play(&state, &catalog, uid, None).expect("Wisdom plays");
    assert_eq!(after.block - state.block, 8);
    assert_eq!(
        pile_len(&after, PileId::Hand),
        pile_len(&state, PileId::Hand) - 1 + 3
    );
    assert_eq!(
        pile_len(&after, PileId::Draw),
        pile_len(&state, PileId::Draw) - 3
    );

    // Power + Expertise: Strength 2 then Dexterity 2, and the played Power
    // leaves every pile.
    let (catalog, state, uid) = root(3, 7);
    let after = play(&state, &catalog, uid, None).expect("Expertise plays");
    assert_eq!(after.powers.value(PowerId::Strength), 2);
    assert_eq!(after.powers.value(PowerId::Dexterity), 2);
    assert_eq!(after.block, state.block, "a Power variant gains no Block");
    assert!(
        PileId::ALL.into_iter().all(|pile| after
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .all(|card| card.uid != uid)),
        "the played Power is removed from combat"
    );
}

/// Mad Science's Curious rider (#3427) plays its native program:
/// `<ExecutePower>d__54` `0x3aa660` IL_01f5-IL_0228 applies CuriousPower
/// `CuriousReduction` (1), and no `ExecuteRider` runs for rider 8. The live
/// power then lowers the next Power card's cost through
/// `CuriousPower::TryModifyEnergyCostInCombat` `0xa102c`: here that is the
/// second Curious copy (a Power costing 1), which becomes free, and playing
/// it stacks the power to 2 without spending Energy.
#[test]
fn a_curious_mad_science_applies_curious_and_frees_the_next_power() {
    use crate::hot::PileId;
    use crate::ids::{CardId, PowerId};
    let props = mad_science_props(3, 8);
    let case = mad_science_case(&[(0, props.clone()), (1, props)]);
    let (document, catalog, state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
    crate::engine::admit(&document, &state, &catalog).expect("a Curious root admits");
    let copies: Vec<crate::hot::HotCard> = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice().iter().copied())
        .filter(|card| catalog.spec(card.atom).unwrap().identity.id == CardId::MadScience)
        .collect();
    assert_eq!(copies.len(), 2);
    for card in &copies {
        let spec = catalog.spec(card.atom).unwrap();
        assert_eq!(
            crate::engine::play::resolved_energy_cost(&state, *card, spec),
            1
        );
    }
    let uid = hand_card_uid(&state, &catalog, CardId::MadScience);
    let after = play(&state, &catalog, uid, None).expect("Curious plays");
    assert_eq!(after.powers.value(PowerId::Curious), 1);
    assert_eq!(state.energy - after.energy, 1);
    assert_eq!(after.block, state.block, "a Power variant gains no Block");
    let other = *copies.iter().find(|card| card.uid != uid).unwrap();
    let spec = catalog.spec(other.atom).unwrap();
    assert_eq!(
        crate::engine::play::resolved_energy_cost(&after, other, spec),
        0,
        "Curious frees the next Power"
    );
    let boundary = crate::boundary::HotBoundary::to_canonical(&after, &catalog);
    assert_eq!(boundary.player.get("curious"), Some(&Value::from(1)));

    // Put the second copy in hand and play it for free: Curious stacks.
    let mut ready = after.clone();
    for pile in [PileId::Draw, PileId::Discard] {
        ready
            .piles
            .get_mut(pile)
            .make_mut()
            .retain(|card| card.uid != other.uid);
    }
    if !ready
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .any(|card| card.uid == other.uid)
    {
        ready.piles.get_mut(PileId::Hand).make_mut().push(other);
    }
    let twice = play(&ready, &catalog, other.uid, None).expect("the free copy plays");
    assert_eq!(twice.powers.value(PowerId::Curious), 2);
    assert_eq!(twice.energy, ready.energy, "the second Power cost nothing");
}

/// Mad Science's Chaos rider (#3322) plays its native program: the Skill body
/// (`<ExecuteSkill>d__53` `0x3aae28`, Block 8), then `<ExecuteRider>d__57`
/// `0x3aa9c4` IL_031e-IL_0395 — the first card of one `GetDistinctForCombat`
/// shuffle of the owner's unlocked pool, `SetToFreeThisTurn`, into Hand.
///
/// One witness per branch the port can take:
/// * the pool is the RECORDED profile's (`DEFECT7_EPOCH` hidden drops its
///   rows) and the fully-unlocked control's, for both levels;
/// * the minted card is free this turn (the Energy Set-0 `ThisTurnOrPlayed`
///   row and one temporary Star row, as Distraction's tail writes);
/// * the Generation stream advances by exactly one complete shuffle;
/// * the minted card round-trips the boundary;
/// * a document with no recorded owner refuses admission by name
///   (`mad science chaos generation provenance`), as does one beside a seeded
///   Bundle of Joy preview (`mad science chaos generation ordering`).
#[test]
fn mad_science_chaos_mints_one_free_card_of_the_recorded_owner_pool() {
    use crate::ids::CardId;
    for upgrade in [0, 1] {
        for (hidden, partial) in [(&["DEFECT7_EPOCH"][..], true), (&[][..], false)] {
            let mut case = with_hidden_epochs(owner_deck_with("CHARACTER.DEFECT", &[]), hidden);
            deck_row(
                &mut case,
                serde_json::json!({
                    "floor_added_to_deck": 1,
                    "id": "CARD.MAD_SCIENCE",
                    "current_upgrade_level": upgrade,
                    "props": mad_science_props(2, 6),
                }),
            );
            let (document, catalog, state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
            crate::engine::admit(&document, &state, &catalog)
                .unwrap_or_else(|refusal| panic!("L{upgrade} hidden={hidden:?}: {refusal}"));
            let pool = catalog.owner_generation_pool(crate::catalog::RewardPool::Defect);
            let full = crate::steps::neutral::owner_generation_pool(
                crate::catalog::RewardPool::Defect,
                None,
            );
            assert_eq!(pool.len() < full.len(), partial);
            // The complete closure is interned whatever the stream position.
            for id in &pool {
                assert!(
                    catalog
                        .atom(&crate::catalog::CardIdentity {
                            id: *id,
                            upgrade: 0,
                            enchantment: None,
                        })
                        .is_some(),
                    "{id:?} is interned"
                );
            }
            let mut rng = generation_rng(&state);
            let expected = distinct_prefix(&mut rng, &pool, 1)[0];
            let uid = hand_card_uid(&state, &catalog, CardId::MadScience);
            let played = play(&state, &catalog, uid, None)
                .unwrap_or_else(|refusal| panic!("Chaos plays: {refusal:?}"));
            assert_eq!(played.block - state.block, 8, "the Skill body's Block");
            assert_eq!(
                played.rng.get(crate::hot::RngStream::Generation).counter,
                rng.counter,
                "one complete shuffle"
            );
            let minted = *played
                .piles
                .get(crate::hot::PileId::Hand)
                .as_slice()
                .last()
                .expect("the minted card is in Hand");
            let spec = catalog.spec(minted.atom).unwrap();
            assert_eq!(spec.identity.id, expected, "L{upgrade} hidden={hidden:?}");
            assert_eq!(spec.identity.upgrade, 0);
            let instance = played.card_states.get(minted.uid);
            if spec.cost >= 0 {
                assert_eq!(
                    instance.local_cost_modifiers.resolve(99),
                    0,
                    "free this turn"
                );
            }
            assert_eq!(instance.free_star_cost_this_turn_or_played_rows, 1);
            let wire = crate::boundary::HotBoundary::to_canonical(&played, &catalog);
            let back = crate::boundary::HotBoundary::from_canonical(&wire, &catalog)
                .expect("the played state hydrates");
            assert_eq!(
                crate::boundary::HotBoundary::to_canonical(&back, &catalog),
                wire
            );
        }
    }

    // The Sly-parent census counts a Chaos copy as two future copies of every
    // owner-pool card (Scrape is a Defect discard parent), none of a card
    // outside the pool, and nothing for any other variant.
    for (rider, expected) in [(6, 2), (4, 0)] {
        let mut case = owner_deck_with("CHARACTER.DEFECT", &[]);
        deck_row(
            &mut case,
            serde_json::json!({
                "floor_added_to_deck": 1,
                "id": "CARD.MAD_SCIENCE",
                "props": mad_science_props(2, rider),
            }),
        );
        let (_document, catalog, state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
        assert!(
            catalog
                .owner_generation_pool(crate::catalog::RewardPool::Defect)
                .contains(&CardId::Scrape)
        );
        assert_eq!(
            crate::engine::admission::mad_science_chaos_sly_parent_multiplicity(
                &state,
                &catalog,
                CardId::Scrape
            ),
            expected,
            "rider {rider}"
        );
        assert_eq!(
            crate::engine::admission::mad_science_chaos_sly_parent_multiplicity(
                &state,
                &catalog,
                CardId::StrikeDefect
            ),
            0
        );
    }

    // No recorded owner: the owner-pool provenance is missing.
    let mut case = owner_deck_with("CHARACTER.DEFECT", &[]);
    deck_row(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1,
            "id": "CARD.MAD_SCIENCE",
            "props": mad_science_props(2, 6),
        }),
    );
    let (mut document, _catalog, _state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
    document.player.remove("reward_card_pool");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog).unwrap();
    let refusal = crate::engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("mad science chaos generation provenance"),
        "{refusal}"
    );

    // A catalog that does not hold the owner's pool: a Defect root whose hot
    // owner is swapped for Silent, whose pool the boundary never interned.
    let mut case = owner_deck_with("CHARACTER.DEFECT", &[]);
    deck_row(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1,
            "id": "CARD.MAD_SCIENCE",
            "props": mad_science_props(2, 6),
        }),
    );
    let (document, catalog, mut state) = owner_pool_root(&case, &["MAD_SCIENCE"]);
    state.reward_card_pool = Some(crate::catalog::RewardPool::Silent);
    let refusal = crate::engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("mad science chaos generation closure"),
        "{refusal}"
    );
    // A Regent owner without its Colorless-pool provenance, the guard
    // Discovery shares.
    state.reward_card_pool = Some(crate::catalog::RewardPool::Regent);
    state.set_spectrum_shift_generation_pool(false);
    let refusal = crate::engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("mad science chaos generation provenance"),
        "{refusal}"
    );
    // The body refuses both on its own, before any write: the Regent guard,
    // and a missing owner.
    let uid = hand_card_uid(&state, &catalog, CardId::MadScience);
    assert!(matches!(
        play(&state, &catalog, uid, None),
        Err(crate::engine::EngineRefusal::MalformedArgs(
            "mad_science_chaos_exact Regent pool provenance"
        ))
    ));
    state.reward_card_pool = None;
    assert!(matches!(
        play(&state, &catalog, uid, None),
        Err(crate::engine::EngineRefusal::MalformedArgs(
            "mad_science_chaos_exact"
        ))
    ));

    // Beside a Bundle of Joy, whose seeded preview a Chaos shuffle would move.
    let mut case = owner_deck_with("CHARACTER.DEFECT", &["BUNDLE_OF_JOY"]);
    deck_row(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1,
            "id": "CARD.MAD_SCIENCE",
            "props": mad_science_props(2, 6),
        }),
    );
    let (document, catalog, state) = owner_pool_root(&case, &["MAD_SCIENCE", "BUNDLE_OF_JOY"]);
    let refusal = crate::engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("mad science chaos generation ordering"),
        "{refusal}"
    );
}

/// The boundary's half of the Tinker row (#2942), both directions: every
/// document that misspells, misplaces, omits or disagrees about the row is
/// refused by name, never read as a different card.
#[test]
fn the_boundary_refuses_every_malformed_mad_science_tinker_row() {
    use crate::boundary::{BoundaryRefusal, HotBoundary};
    use crate::catalog::{CatalogError, MadScienceVariant};
    let opening = open(&mad_science_case(&[(0, mad_science_props(2, 4))])).unwrap();
    let with_extra = |document: &crate::canonical::CanonicalStateV2, id: &str, extra: Value| {
        let mut document = document.clone();
        let card = document
            .piles
            .values_mut()
            .flatten()
            .find(|card| card.id == id)
            .expect("the card is in a pile");
        card.extra = serde_json::from_value(extra).unwrap();
        document
    };
    let load = |document: &crate::canonical::CanonicalStateV2| {
        HotBoundary::catalog_from_canonical(document)
            .and_then(|catalog| HotBoundary::from_canonical(document, &catalog).map(|_| ()))
    };
    let unrepresentable = |refusal: BoundaryRefusal| match refusal {
        BoundaryRefusal::UnrepresentableValue { field, detail, .. } if field == "extra" => detail,
        other => panic!("expected an `extra` refusal, got {other}"),
    };
    assert!(load(&opening.document).is_ok(), "the control loads");

    // No row: no fight variant, so Mad Science cannot be interned.
    assert_eq!(
        load(&with_extra(
            &opening.document,
            "MAD_SCIENCE",
            serde_json::json!([])
        )),
        Err(BoundaryRefusal::Catalog(
            CatalogError::MadScienceVariantUnknown
        ))
    );
    // A row naming a pair TinkerTime cannot write, or of the wrong arity.
    for row in [
        serde_json::json!(["MAD_SCIENCE_TINKER", 1, 4]),
        serde_json::json!(["MAD_SCIENCE_TINKER", 2]),
        serde_json::json!(["MAD_SCIENCE_TINKER", 2, 4, 0]),
        serde_json::json!(["MAD_SCIENCE_TINKER", "2", 4]),
    ] {
        let detail = unrepresentable(
            load(&with_extra(
                &opening.document,
                "MAD_SCIENCE",
                serde_json::json!([row]),
            ))
            .unwrap_err(),
        );
        assert!(detail.contains("not a legal saved"), "{row}: {detail}");
    }
    // A legal but unported variant: the catalog refuses it by name.
    assert_eq!(
        load(&with_extra(
            &opening.document,
            "MAD_SCIENCE",
            serde_json::json!([["MAD_SCIENCE_TINKER", 3, 9]])
        )),
        Err(BoundaryRefusal::Catalog(
            CatalogError::MadScienceVariantNotModeled(MadScienceVariant {
                tinker_type: 3,
                rider: 9
            })
        ))
    );
    // The row after another tail row, or on a card that is not Mad Science.
    let misplaced = with_extra(
        &opening.document,
        "MAD_SCIENCE",
        serde_json::json!([
            ["CARD_AFFLICTION", "BOUND", 3],
            ["MAD_SCIENCE_TINKER", 2, 4]
        ]),
    );
    assert!(unrepresentable(load(&misplaced).unwrap_err()).contains("first"));
    let foreign = with_extra(
        &opening.document,
        "STRIKE_IRONCLAD",
        serde_json::json!([["MAD_SCIENCE_TINKER", 2, 4]]),
    );
    assert!(unrepresentable(load(&foreign).unwrap_err()).contains("belongs only"));
    // Two copies that disagree.
    let mut mixed = opening.document.clone();
    let mut second = mixed
        .piles
        .values()
        .flatten()
        .find(|card| card.id == "MAD_SCIENCE")
        .unwrap()
        .clone();
    second.uid = Some(999);
    second.extra = vec![serde_json::json!(["MAD_SCIENCE_TINKER", 2, 5])];
    mixed.piles.get_mut("discard").unwrap().push(second);
    let detail = unrepresentable(HotBoundary::catalog_from_canonical(&mixed).unwrap_err());
    assert!(
        detail.contains("different Tinker Time variants"),
        "{detail}"
    );
}

/// An in-combat copy of a Mad Science keeps its variant (#2942): the engine's
/// one physical-clone primitive (`engine::cards::inject_generated_clones_bottom`,
/// the Juggling / relic clone path) copies the Sapping Attack, and the clone
/// is the same generated row and re-emits the same Tinker row. A native clone
/// is `CardModel` memberwise (`AbstractModel::MutableClone` `0x79f0c`
/// IL_000d), and `MadScience` overrides no `AfterCloned`, so both saved fields
/// travel with it.
#[test]
fn an_in_combat_mad_science_copy_keeps_its_variant() {
    use crate::ids::CardId;
    let case = mad_science_case(&[(0, mad_science_props(1, 1))]);
    let (_document, catalog, mut cloned) = owner_pool_root(&case, &["MAD_SCIENCE"]);
    let mad_science_in_hand = |state: &crate::hot::HotState| -> Vec<crate::hot::HotCard> {
        state
            .piles
            .get(crate::hot::PileId::Hand)
            .as_slice()
            .iter()
            .filter(|card| catalog.spec(card.atom).unwrap().identity.id == CardId::MadScience)
            .copied()
            .collect()
    };
    let [source] = mad_science_in_hand(&cloned)[..] else {
        panic!("one Mad Science in hand");
    };
    crate::engine::cards::inject_generated_clones_bottom(
        &mut cloned,
        &catalog,
        source,
        1,
        crate::hot::PileId::Hand,
        &mut Vec::new(),
    )
    .expect("a Mad Science clones");
    let copies = mad_science_in_hand(&cloned);
    assert_eq!(copies.len(), 2, "the source and its clone");
    let sapping = crate::catalog::MadScienceVariant {
        tinker_type: 1,
        rider: 1,
    };
    for copy in &copies {
        assert!(std::ptr::eq(
            catalog.spec(copy.atom).unwrap().row,
            sapping.row(0).unwrap().unwrap()
        ));
    }
    let projected = crate::boundary::HotBoundary::to_canonical(&cloned, &catalog);
    let rows: Vec<_> = card_named(&projected, "MAD_SCIENCE")
        .into_iter()
        .map(|card| card["extra"].clone())
        .collect();
    assert_eq!(
        rows,
        vec![serde_json::json!([["MAD_SCIENCE_TINKER", 1, 1]]); 2]
    );
}

/// Two copies in one deck: the same variant opens (and the upgraded copy is
/// the same variant's level-1 row), two different variants refuse by name,
/// because the catalog keys every Mad Science spec by one fight variant.
#[test]
fn mad_science_copies_share_one_fight_variant() {
    let same = mad_science_case(&[(0, mad_science_props(2, 4)), (1, mad_science_props(2, 4))]);
    let opening = open(&same).expect("two copies of one variant open");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    let levels: Vec<(u8, bool)> = catalog
        .specs()
        .filter(|spec| spec.identity.id == crate::ids::CardId::MadScience)
        .map(|spec| (spec.identity.upgrade, spec.is_skill))
        .collect();
    assert!(
        levels.contains(&(0, true)) && levels.contains(&(1, true)),
        "{levels:?}"
    );

    let mixed = mad_science_case(&[(0, mad_science_props(2, 4)), (0, mad_science_props(2, 5))]);
    let refusal = open(&mixed).expect_err("two variants refuse");
    assert_eq!(refusal.class(), "card_entry_variant_not_modeled");
}

/// One oracle-printed witness for the default slot-7 physical payload (#2827).
struct PhysicalWitness {
    id: &'static str,
    upgrade: i64,
    enchantment: Option<(&'static str, i64)>,
    copies: usize,
    digest: &'static str,
    /// The oracle's post-deal rows for the inserted copies, in `uid` order.
    cards: &'static str,
}

/// Every `_PHYSICAL_COST_CARD_IDS` id except `THE_SCYTHE` and `WITHER` (whose
/// opening stops at an existing boundary gate; see the test after this one),
/// one fresh copy each
/// at the head of the empty-belt deck, plus three variants on the same branch:
/// an upgraded copy, a two-copy group (the `exact_piles` deck rule's
/// divergent-card arm), and an upgraded copy carrying a mutable enchantment
/// (slot 5 and slot 7 together).
///
/// Every digest and every card row was printed by the oracle with the recipe
/// above `empty_belt_case`, the mutation being `copies` insertions at index 0
/// of `{'floor_added_to_deck': 1, 'id': 'CARD.<id>'}`, with
/// `'current_upgrade_level': <upgrade>` when it is nonzero and
/// `'enchantment': {'id': 'ENCHANTMENT.<name>', 'amount': <amount>}` when one
/// is named.
///
/// The Kingly Kick and Kingly Punch rows are *not* the default payload: both
/// are drawn into the opening hand, and their `AfterCardDrawn` bodies
/// (`KinglyKick::AfterCardDrawn` `0xe3abb`, `KinglyPunch::AfterCardDrawn`
/// `0xe3b80`) write a cost row and damage growth there. They pin that the
/// opening hands the engine the default payload and the engine's draw hook
/// does the rest, exactly as it does for a Python-rooted document.
const PHYSICAL_WITNESSES: [PhysicalWitness; 22] = [
    PhysicalWitness {
        id: "BANSHEES_CRY",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "c0519540faf28827bd8f92b03ad113db0f32027c8e71172751000920d5ea68c3",
        cards: r#"[{"id": "BANSHEES_CRY", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "BULLET_TIME",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "1c0ded0a39833f475c1473bd913263f663f8854d31b17e68b640e388f540c789",
        cards: r#"[{"id": "BULLET_TIME", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "CLAW",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "f0cbcdfa6f5e274a0699fbaaae1b10776afacc81b1098f1e24727737a9de964c",
        cards: r#"[{"id": "CLAW", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "ENLIGHTENMENT",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "83dd12ab4eca384124a0d1f64f0d28c10ed0a79d833c69828783bae49c335c13",
        cards: r#"[{"id": "ENLIGHTENMENT", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "FLATTEN",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "67ffe8c9aa2b3a001cfc0927b9894fd44660e1c20e0620e782d8e4c15681262a",
        cards: r#"[{"id": "FLATTEN", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "KINGLY_KICK",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "ae8c6b90cfe77785094609f4d65b9d9704a9ba36e991d44f4c4c6d54f9da6a80",
        cards: r#"[{"id": "KINGLY_KICK", "physical_state": ["PHYSICAL_CARD_STATE", [[2, -1, 0, false]], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "KINGLY_PUNCH",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "b3f98b10c855d86749f3414fbf93bce1754d0a72197f898b502de032fd92255c",
        cards: r#"[{"id": "KINGLY_PUNCH", "physical_state": ["PHYSICAL_CARD_STATE", [], 4, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "MAUL",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "100bb3bd0b401100498d4cf557f64150af605e4cc8c16dfab46722de2762c298",
        cards: r#"[{"id": "MAUL", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "MELANCHOLY",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "7dd9e0e224e36537f1071452035138cbd8355663ae90a46a3a6cca4df6c38013",
        cards: r#"[{"id": "MELANCHOLY", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "MIDNIGHT",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "7b0e7868f41032ac9ab200d6fc96beb3b65bc12f0c90eba308fc782286ef23a0",
        cards: r#"[{"id": "MIDNIGHT", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "MODDED",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "2a01faae783d4a61b010236131f442441359ad7fb5daa3b436eef60857f7e31e",
        cards: r#"[{"id": "MODDED", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "MOMENTUM_STRIKE",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "a6f926cc423b6f371c81df59a8b469e0cad0c0dea478f5b4229420905819e8d6",
        cards: r#"[{"id": "MOMENTUM_STRIKE", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "PINPOINT",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "a3326813591fc8e42b15995aa4dfb67ab5ff3409e2cf01ecdeb2ae19b33e16ae",
        cards: r#"[{"id": "PINPOINT", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "RAMPAGE",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "de7946e04a115a928c38aa96adda06426290841a765c76b4d207582d196011b9",
        cards: r#"[{"id": "RAMPAGE", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "ROCKET_PUNCH",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "ec44755a6b0d591321d7eeafa06ff1ef276fab088ae6b160c092a6edd73fc20e",
        cards: r#"[{"id": "ROCKET_PUNCH", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "STOMP",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "5336939c0587b25af3579bd7abbc2eb80561e4b688e472d79ba6ab5ecf2185bc",
        cards: r#"[{"id": "STOMP", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "THE_BALL",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "b2efcae1cc82e0775e70d29f2ab5f5718425e6683ffe0ec35d4f98b123160132",
        cards: r#"[{"id": "THE_BALL", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "THRASH",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "0a5a833e14d643627fd14092fdf68e4273a02d3df4e80c70963d02e7c584cbf4",
        cards: r#"[{"id": "THRASH", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "UP_MY_SLEEVE",
        upgrade: 0,
        enchantment: None,
        copies: 1,
        digest: "6a8630e170abb66ddb3b388dc350dbb00286eaa13066799b29543caaad840271",
        cards: r#"[{"id": "UP_MY_SLEEVE", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "RAMPAGE",
        upgrade: 1,
        enchantment: None,
        copies: 1,
        digest: "a3eac7fd612df61ac6619ce9664e0c576225a669544d131e5694277961a4b5bf",
        cards: r#"[{"id": "RAMPAGE", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 1}]"#,
    },
    PhysicalWitness {
        id: "CLAW",
        upgrade: 0,
        enchantment: None,
        copies: 2,
        digest: "1f0047472eed286b78fee997ca0e82875821760c9a14f092db5394b9ebc1412a",
        cards: r#"[{"id": "CLAW", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 5, "upgrade": 0}, {"id": "CLAW", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 7, "upgrade": 0}]"#,
    },
    PhysicalWitness {
        id: "RAMPAGE",
        upgrade: 1,
        enchantment: Some(("VIGOROUS", 3)),
        copies: 1,
        digest: "8c0d3e7cb63e76a0e3f5ff5fda44c5e6133c0e00e396ae49c81f9e337afbacc7",
        cards: r#"[{"enchantment": ["VIGOROUS", 3], "enchantment_state": ["VIGOROUS", 0], "id": "RAMPAGE", "physical_state": ["PHYSICAL_CARD_STATE", [], 0, false, []], "uid": 4, "upgrade": 1}]"#,
    },
];

#[test]
fn every_physical_cost_card_but_the_scythe_opens_with_the_oracles_default_payload() {
    let mut ids = std::collections::BTreeSet::new();
    for witness in &PHYSICAL_WITNESSES {
        let mut case = empty_belt_case();
        let mut row = serde_json::json!({
            "floor_added_to_deck": 1, "id": format!("CARD.{}", witness.id)});
        if witness.upgrade != 0 {
            row["current_upgrade_level"] = Value::from(witness.upgrade);
        }
        if let Some((name, amount)) = witness.enchantment {
            row["enchantment"] =
                serde_json::json!({"id": format!("ENCHANTMENT.{name}"), "amount": amount});
        }
        for _ in 0..witness.copies {
            deck_row(&mut case, row.clone());
        }
        let opening =
            open(&case).unwrap_or_else(|refusal| panic!("{} opens: {refusal}", witness.id));
        let mut cards = card_named(&opening.document, witness.id);
        cards.sort_by_key(|card| card["uid"].as_u64());
        let expected: Value = serde_json::from_str(witness.cards).expect("oracle rows parse");
        assert_eq!(Value::Array(cards), expected, "{} card rows", witness.id);
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            witness.digest,
            "{} digest",
            witness.id
        );
        ids.insert(witness.id);
    }
    // The table covers the whole id set the branch writes, so a new
    // `PHYSICAL_STATE_CARDS` entry without a witness fails here.
    let covered: std::collections::BTreeSet<&str> = super::PHYSICAL_STATE_CARDS
        .iter()
        .map(|id| id.as_str())
        .filter(|id| !matches!(*id, "THE_SCYTHE" | "WITHER"))
        .collect();
    assert_eq!(ids, covered);
}

/// Wither takes the same default-payload arm, but its opening stops at the
/// boundary's pre-existing Aeonglass gate
/// (`engine::cards::aeonglass_state_is_exact`): a Wither card makes the
/// Aeonglass surface reachable (`_aeonglass_state_reachable`, frozen Python, deleted #2827) and a Toadpoles roster has no Aeonglass owner.
///
/// The oracle roots this save (digest
/// `f6f661648366accca40e38eeb3945a9d1b1c9a2424fbf70e63e84171b3871189`, the
/// Wither row being exactly the default payload), and its OWN document is
/// refused by `HotBoundary::from_canonical` with this same field and detail —
/// checked by feeding the oracle's printed document through the boundary on
/// 2026-09-23. So this is the crate's standing admission ceiling for a deck
/// Wither, not something the opening introduced, and it is a refusal by name
/// rather than a silently different document. The pre-hook document still
/// pins the payload the opening wrote.
#[test]
fn a_deck_wither_carries_the_default_payload_and_stops_at_the_aeonglass_gate() {
    let mut case = empty_belt_case();
    deck_row(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.WITHER"}),
    );
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    let wither: Vec<Value> = card_named(&built.document, "WITHER");
    assert_eq!(wither.len(), 1);
    assert_eq!(
        wither[0]["physical_state"],
        serde_json::json!(["PHYSICAL_CARD_STATE", [], 0, false, []])
    );
    let refusal = open(&case).expect_err("the boundary's Aeonglass gate refuses");
    assert_eq!(refusal.class(), "boundary_unrepresentable");
    assert!(
        refusal
            .to_string()
            .contains("Aeonglass owner/counters/countdown/Wither state is not exact"),
        "{refusal}"
    );
}

/// A saved The Scythe row, as `save['players'][0]['deck']` spells it.
fn scythe_row(current: i64, increased: i64, upgrade: i64) -> Value {
    let mut row = serde_json::json!({
        "floor_added_to_deck": 1,
        "id": "CARD.THE_SCYTHE",
        "props": {"ints": [
            {"name": "CurrentDamage", "value": current},
            {"name": "IncreasedDamage", "value": increased}]}
    });
    if upgrade != 0 {
        row["current_upgrade_level"] = Value::from(upgrade);
    }
    row
}

/// One oracle-printed The Scythe opening (#2827).
struct ScytheWitness {
    /// The inserted deck rows, in save-array order (row 0 first).
    rows: Vec<Value>,
    /// The oracle's `differential_digest` for the save, recorded as the
    /// provenance of the rows below (see the test's doc for why the Rust
    /// document cannot be digest-compared yet).
    oracle_digest: &'static str,
    /// The oracle's slot-7 payloads for the Scythe copies, in deck-row order.
    payloads: Value,
    /// The oracle's `player.scythe_deck_growth`.
    deck_growth: Value,
    /// The oracle's `player.exact_piles` (absent = its `false` default).
    exact_piles: Option<bool>,
}

/// A saved The Scythe copy enters with its growth and its `DeckVersion` row,
/// exactly as `start_combat` builds it, and opens to the oracle's document
/// (#2941).
///
/// Every value was printed by the oracle with the recipe above
/// `empty_belt_case`, the mutation being the rows inserted at the head of the
/// deck in the order listed (`scythe_row` spells each). The five shapes: a
/// grown copy, a fresh copy (growth 0, which skips `_card_add_damage_growth`
/// and must land on the same seven-field payload), an upgraded copy (the base
/// is still 13: `UpdateDamage` `0xee9a6`), two copies with different growth
/// (two deck rows, and the oracle's `exact_piles` deck rule), and an enchanted
/// copy (slots 2 and 7 together).
///
/// The oracle's post-deal document links each copy to its deck row (`0`,
/// `1`) while numbering it by pile position (`uid` 4, 5, 7). Until #2941 the
/// canonical boundary admitted a seven-field Scythe payload only as a
/// Thieving Hopper master-deck link, whose row *is* the uid; it now carries
/// an ordinary fight's link as `CardStates::scythe_deck_links`, so the
/// opening's document is compared whole with the oracle's digest.
#[test]
fn a_saved_scythe_copy_carries_its_growth_and_deck_row_and_opens() {
    let witnesses = [
        ScytheWitness {
            rows: vec![scythe_row(16, 3, 0)],
            oracle_digest: "e5ca8fae816f502d4b5fac86ed530810251868170e505c5d76684c6c47ee5978",
            payloads: serde_json::json!([["PHYSICAL_CARD_STATE", [], 3, false, [], 0, 0]]),
            deck_growth: serde_json::json!([[0, 3]]),
            exact_piles: None,
        },
        ScytheWitness {
            rows: vec![scythe_row(13, 0, 0)],
            oracle_digest: "9f246b60856315335d53916fc461fbfc173546dec2906fce6e56c831955d3abe",
            payloads: serde_json::json!([["PHYSICAL_CARD_STATE", [], 0, false, [], 0, 0]]),
            deck_growth: serde_json::json!([[0, 0]]),
            exact_piles: None,
        },
        ScytheWitness {
            rows: vec![scythe_row(20, 7, 1)],
            oracle_digest: "745fe35ec35763883e5027f38c3670356deac69c6a897c2032063139f003f732",
            payloads: serde_json::json!([["PHYSICAL_CARD_STATE", [], 7, false, [], 0, 0]]),
            deck_growth: serde_json::json!([[0, 7]]),
            exact_piles: None,
        },
        ScytheWitness {
            rows: vec![scythe_row(20, 7, 0), scythe_row(27, 14, 0)],
            oracle_digest: "8249dc3d8a963cfde09d850dc315f4a7eeb8f4dff564a38332984431d556a054",
            payloads: serde_json::json!([
                ["PHYSICAL_CARD_STATE", [], 7, false, [], 0, 0],
                ["PHYSICAL_CARD_STATE", [], 14, false, [], 0, 1]
            ]),
            deck_growth: serde_json::json!([[0, 7], [1, 14]]),
            exact_piles: Some(true),
        },
        ScytheWitness {
            rows: vec![{
                let mut row = scythe_row(16, 3, 0);
                row["enchantment"] = serde_json::json!({"id": "ENCHANTMENT.SHARP", "amount": 2});
                row
            }],
            oracle_digest: "a60afda71e7f7f41a5dc015cdc336677523f31c0ac4cefbc0d911b3244367909",
            payloads: serde_json::json!([["PHYSICAL_CARD_STATE", [], 3, false, [], 0, 0]]),
            deck_growth: serde_json::json!([[0, 3]]),
            exact_piles: None,
        },
    ];
    for witness in witnesses {
        let mut case = empty_belt_case();
        for row in witness.rows.iter().rev() {
            deck_row(&mut case, row.clone());
        }
        let built = pre_hook(&case).expect("the pre-hook opening builds");
        let mut scythes = card_named(&built.document, "THE_SCYTHE");
        // Deck-row order: the payload's seventh field.
        scythes.sort_by_key(|card| card["physical_state"][6].as_u64());
        let payloads: Vec<Value> = scythes
            .iter()
            .map(|card| card["physical_state"].clone())
            .collect();
        assert_eq!(
            Value::Array(payloads),
            witness.payloads,
            "{}",
            witness.oracle_digest
        );
        for (card, row) in scythes.iter().zip(&witness.rows) {
            assert_eq!(
                card["upgrade"],
                row.get("current_upgrade_level")
                    .cloned()
                    .unwrap_or(Value::from(0))
            );
            match row.get("enchantment") {
                Some(_) => assert_eq!(card["enchantment"], serde_json::json!(["SHARP", 2])),
                None => assert!(card.get("enchantment").is_none()),
            }
        }
        assert_eq!(
            built.document.player.get("scythe_deck_growth"),
            Some(&witness.deck_growth),
            "{}",
            witness.oracle_digest
        );
        assert_eq!(
            built.document.player.get("exact_piles"),
            witness.exact_piles.map(Value::from).as_ref(),
            "{}",
            witness.oracle_digest
        );
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            witness.oracle_digest
        );
    }
}

/// The Scythe's saved props outside the native domain refuse by name, each
/// one a save the oracle also refuses (printed with the recipe above
/// `empty_belt_case`: *"violate CurrentDamage == 13 + IncreasedDamage"* or
/// *"are not the exact two native integer fields"*), except the last row,
/// which is the validator's one documented narrowing: the oracle ignores a
/// second key of `props` and roots that save (digest `e5ca8fae…`, the same
/// document as the first Scythe witness), this refuses it.
#[test]
fn a_scythe_copy_outside_the_native_domain_refuses() {
    for props in [
        // No props at all.
        Value::Null,
        // `CurrentDamage != 13 + IncreasedDamage`.
        serde_json::json!({"ints": [
            {"name": "CurrentDamage", "value": 9},
            {"name": "IncreasedDamage", "value": 3}]}),
        // A negative growth that satisfies the relation.
        serde_json::json!({"ints": [
            {"name": "CurrentDamage", "value": 11},
            {"name": "IncreasedDamage", "value": -2}]}),
        // A stranger field in place of one of the two.
        serde_json::json!({"ints": [
            {"name": "CurrentDamage", "value": 16},
            {"name": "Stranger", "value": 3}]}),
        // A duplicated field.
        serde_json::json!({"ints": [
            {"name": "IncreasedDamage", "value": 3},
            {"name": "IncreasedDamage", "value": 3}]}),
        // Genetic Algorithm's names on a Scythe.
        serde_json::json!({"ints": [
            {"name": "CurrentBlock", "value": 4},
            {"name": "IncreasedBlock", "value": 3}]}),
        // Extreme values: `13 + IncreasedDamage` must refuse, not overflow.
        serde_json::json!({"ints": [
            {"name": "CurrentDamage", "value": i64::MIN},
            {"name": "IncreasedDamage", "value": i64::MAX}]}),
        // The narrowing: a second `props` key.
        serde_json::json!({"ints": [
            {"name": "CurrentDamage", "value": 16},
            {"name": "IncreasedDamage", "value": 3}], "bools": []}),
    ] {
        let mut case = empty_belt_case();
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.THE_SCYTHE"});
        if !props.is_null() {
            row["props"] = props.clone();
        }
        deck_row(&mut case, row);
        let refusal = open(&case).expect_err("an inexact payload refuses");
        assert_eq!(refusal.class(), "card_entry_props_not_exact", "{props}");
        assert!(refusal.to_string().starts_with("THE_SCYTHE"), "{refusal}");
    }
}

/// High-tier review finding P1-3 (#2544): Ember Tea was excluded from the
/// room-entry relic gate and then never seeded or applied.
///
/// Mutation: `save['players'][0]['relics'].append({'floor_added_to_deck': 1,
/// 'id': 'RELIC.EMBER_TEA', 'props': {'ints':
/// [{'name': 'CombatsLeft', 'value': 3}]}})`.
#[test]
fn an_ember_tea_charge_of_three_grants_strength_and_decrements() {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA",
            "props": {"ints": [{"name": "CombatsLeft", "value": 3}]}}),
    );
    let opening = open(&case).expect("an Ember Tea fight opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(2)),
        "EmberTea applies its CanonicalVars Strength on entering the room"
    );
    assert_eq!(
        opening.document.player.get("ember_tea_combats_left"),
        Some(&Value::from(2)),
        "and the persistent charge decrements after the awaited Apply"
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "84b44de1edd2b904a6332259fd5679f30b2a26449772571aeeaefc4e57178b52"
    );
}

/// The grant goes through the engine's own owner-Strength command, so Ruined
/// Helmet's first-positive doubling and its latch are the engine's single
/// implementation rather than a second copy in the opening.
#[test]
fn ruined_helmet_doubles_the_ember_tea_grant_and_latches() {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA",
            "props": {"ints": [{"name": "CombatsLeft", "value": 3}]}}),
    );
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.RUINED_HELMET"}),
    );
    let opening = open(&case).expect("Ember Tea with Ruined Helmet opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(4))
    );
    assert_eq!(
        opening.document.player.get("ruined_helmet_used"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        opening.document.player.get("ember_tea_combats_left"),
        Some(&Value::from(2))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "29007eefe4749faa069fa16d55762ad4c7f45d5738df7be2d434c3ac5ce671cd"
    );

    // A second source with a DIFFERENT amount makes the doubling depend on
    // unrecorded acquisition order, and the oracle refuses the same fight:
    // `RUINED_HELMET with distinct AfterRoomEntered Strength sources
    // [('RELIC.SWORD_OF_JADE', 3), ('RELIC.EMBER_TEA', 2)]`.
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.SWORD_OF_JADE"}),
    );
    let refusal = open(&case).expect_err("distinct Strength amounts refuse");
    assert_eq!(refusal.class(), "ruined_helmet_strength_order");
    assert!(
        refusal.to_string().contains("RELIC.SWORD_OF_JADE=3"),
        "{refusal}"
    );
}

/// An untouched Ember Tea (its constructor's charge of 5,
/// `EmberTea::.ctor` RVA `0x92fdf` IL_0002) opens, grants its Strength and
/// leaves 4 (#3381). The boundary's compact counter used to stop at 4, so the
/// pre-hook document carrying 5 refused as `ember_tea_combats_left` out of
/// domain (TZZ1FU1T6K9G n24). The boundary now takes 5 and still refuses 6.
#[test]
fn a_fresh_ember_tea_charge_of_five_opens() {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA",
            "props": {"ints": [{"name": "CombatsLeft", "value": 5}]}}),
    );
    let opening = open(&case).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(2))
    );
    assert_eq!(
        opening.document.player.get("ember_tea_combats_left"),
        Some(&Value::from(4))
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    for (charge, accepted) in [(5, true), (6, false)] {
        let mut document = opening.document.clone();
        document
            .player
            .insert("ember_tea_combats_left".to_string(), Value::from(charge));
        let loaded = crate::boundary::HotBoundary::from_canonical(&document, &catalog);
        assert_eq!(loaded.is_ok(), accepted, "charge {charge}");
        if let Ok(state) = loaded {
            assert_eq!(state.ember_tea_combats_left(), 5);
            assert!(state.relic_state_is_exact());
        }
    }
}

#[test]
fn an_ember_tea_counter_outside_its_native_domain_refuses() {
    for counter in [Value::from(-1), Value::from(6), Value::Null] {
        let mut case = empty_belt_case();
        let props = if counter.is_null() {
            serde_json::json!({})
        } else {
            serde_json::json!({"ints": [{"name": "CombatsLeft", "value": counter}]})
        };
        relic(
            &mut case,
            serde_json::json!({
                "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA", "props": props}),
        );
        let refusal = open(&case).expect_err("an unseeded charge refuses");
        assert_eq!(refusal.class(), "relic_counter_not_exact", "{counter}");
        assert!(refusal.to_string().contains("RELIC.EMBER_TEA"), "{refusal}");
    }
}

/// Every room-entry Strength source the oracle lists reaches
/// [`super::ruined_helmet_room_entry_strength_is_ordered`] counted: none is
/// still behind the body gate. (`RELIC.GIRYA` left it when #2827 modeled its
/// lifts, and `RELIC.SLING_OF_COURAGE` when #2827 item B modeled its elite
/// Strength; `sling_of_courage_opens_and_matches_the_oracle` pins its row.)
#[test]
fn no_room_entry_strength_source_is_still_gated_above() {
    for relic in [
        "RELIC.VAJRA",
        "RELIC.SLING_OF_COURAGE",
        "RELIC.SWORD_OF_JADE",
        "RELIC.GIRYA",
        "RELIC.EMBER_TEA",
        "RELIC.RED_SKULL",
    ] {
        assert!(
            !super::OPENING_WINDOW_RELIC_BODIES.contains(&relic),
            "{relic} would refuse before the Ruined Helmet gate counts it"
        );
    }
}

/// `RELIC.VAJRA` grants one Strength on entering a combat room (#2827).
///
/// `Vajra/<AfterRoomEntered>d__6::MoveNext` (`0x333abc`) applies
/// `StrengthPower` for `Decimal::One` at `IL_0062`. Every digest here was
/// printed by the oracle with the recipe above `empty_belt_case`, appending
/// `{'floor_added_to_deck': 1, 'id': 'RELIC.VAJRA'}` (plus the Ember Tea row
/// of [`an_ember_tea_charge_of_three_grants_strength_and_decrements`] or
/// `RELIC.RUINED_HELMET` where named), so these compare against Python rather
/// than transcribing Rust.
#[test]
fn a_vajra_save_grants_one_strength_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.VAJRA")).expect("Vajra opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(1))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "bf8a03fd050b8a4fac7bbe4330fe33a431a6d29b669eb9e0766b2054e99b224d"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

/// Vajra's grant goes through the engine's owner-Strength command, so Ruined
/// Helmet doubles it and latches; with Ember Tea both grants stack; and with
/// both plus Ruined Helmet the doubled amount depends on unrecorded
/// acquisition order, which the oracle refuses by the same name.
#[test]
fn vajra_stacks_with_ember_tea_and_ruined_helmet_like_the_oracle() {
    let tea = serde_json::json!({
        "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA",
        "props": {"ints": [{"name": "CombatsLeft", "value": 3}]}});
    let helmet = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.RUINED_HELMET"});

    let mut with_helmet = with_relic("RELIC.VAJRA");
    relic(&mut with_helmet, helmet.clone());
    let opening = open(&with_helmet).expect("Vajra with Ruined Helmet opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(2))
    );
    assert_eq!(
        opening.document.player.get("ruined_helmet_used"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "4b88dbad70231f66142957b5d8f8185018c5ffa7f5be9a5eeb1e37847ac1b763"
    );

    let mut with_tea = with_relic("RELIC.VAJRA");
    relic(&mut with_tea, tea);
    let opening = open(&with_tea).expect("Vajra with Ember Tea opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(3))
    );
    assert_eq!(
        opening.document.player.get("ember_tea_combats_left"),
        Some(&Value::from(2))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "6033267eb985270870ebe8e5694d94bb1d3a543f30eb137190ac29e964dc4c04"
    );

    // Oracle: `RUINED_HELMET with distinct AfterRoomEntered Strength sources
    // [('RELIC.VAJRA', 1), ('RELIC.EMBER_TEA', 2)]`.
    relic(&mut with_tea, helmet);
    let refusal = open(&with_tea).expect_err("distinct Strength amounts refuse");
    assert_eq!(refusal.class(), "ruined_helmet_strength_order");
    assert!(refusal.to_string().contains("RELIC.VAJRA=1"), "{refusal}");
    assert!(
        refusal.to_string().contains("RELIC.EMBER_TEA=2"),
        "{refusal}"
    );
}

/// `RELIC.CRACKED_CORE` channels one Lightning on the first turn-start walk
/// (#2827).
///
/// `CrackedCore/<BeforeSideTurnStart>d__7::MoveNext` (`0x3221d4`); the body is
/// `engine::relics::cracked_core_before_side_turn_start`. On this Ironclad save
/// the base slot count is zero, so `channel` bootstraps a one-slot queue,
/// the oracle's `[['LIGHTNING', None]]` with `orb_slots` 1. Before #2827
/// every corpus fight holding this relic produced a document missing that
/// queue, which is why it was gated. The Metronome case proves the channel
/// runs through the shared command (Metronome counts channels), not a direct
/// queue write. Digests printed by the oracle with the recipe above
/// `empty_belt_case`.
#[test]
fn a_cracked_core_save_channels_one_lightning_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.CRACKED_CORE")).expect("Cracked Core opens");
    assert_eq!(
        opening.document.player.get("orbs"),
        Some(&serde_json::json!([["LIGHTNING", null]]))
    );
    assert_eq!(
        opening.document.player.get("orb_slots"),
        Some(&Value::from(1))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "868c397417e08b33563774e92f1a127fc35d0e61160ef21155f48c7f9d72a4da"
    );

    let mut with_metronome = with_relic("RELIC.CRACKED_CORE");
    relic(
        &mut with_metronome,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.METRONOME"}),
    );
    let opening = open(&with_metronome).expect("Cracked Core with Metronome opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "f78c8a8496185dcf6b326df32d2b9ea411f2ac2f0380aa3f6556658c38c0d3c5"
    );
}

fn with_voltaic(mut case: Value, upgrade: i64) -> Value {
    let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.VOLTAIC"});
    if upgrade != 0 {
        row["current_upgrade_level"] = Value::from(upgrade);
    }
    deck_row(&mut case, row);
    case
}

fn opens_and_admits(case: &Value, what: &str) -> Opening {
    let opening = open(case).unwrap_or_else(|refusal| panic!("{what} opens: {refusal}"));
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .unwrap_or_else(|refusal| panic!("{what} catalog: {refusal}"));
    let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
        .unwrap_or_else(|refusal| panic!("{what} loads: {refusal}"));
    crate::engine::admission::admit(&opening.document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("{what} admits: {refusal}"));
    opening
}

/// Voltaic's Lightning count is seeded by the opening (#3003).
///
/// See `voltaic_lightning_channeled_seed` for the IL: Voltaic counts the
/// combat's `OrbChanneledEntry` Lightning history, which is empty at combat
/// start, so a Voltaic deck opens at `0` and a deck without one keeps the
/// `-1` sentinel (the field elided). Before #3003 the opening never wrote the
/// field, and `engine::admission`'s Voltaic/sentinel pairing refused every
/// Voltaic root as `OrbState`. Cracked Core's turn-one `BeforeSideTurnStart`
/// Lightning runs through the shared channel command, so beside a Voltaic it
/// is counted (`1`) and without one it is not.
#[test]
fn a_voltaic_deck_opens_with_its_lightning_count_and_admits() {
    let control = opens_and_admits(&empty_belt_case(), "the control");
    assert_eq!(control.document.player.get("lightning_channeled"), None);

    for upgrade in [0, 1] {
        let opening = opens_and_admits(
            &with_voltaic(empty_belt_case(), upgrade),
            &format!("Voltaic L{upgrade}"),
        );
        assert_eq!(
            opening.document.player.get("lightning_channeled"),
            Some(&Value::from(0)),
            "Voltaic L{upgrade}"
        );
    }

    let opening = opens_and_admits(
        &with_voltaic(with_relic("RELIC.CRACKED_CORE"), 0),
        "Voltaic + Cracked Core",
    );
    assert_eq!(
        opening.document.player.get("orbs"),
        Some(&serde_json::json!([["LIGHTNING", null]]))
    );
    assert_eq!(
        opening.document.player.get("lightning_channeled"),
        Some(&Value::from(1))
    );

    let opening = opens_and_admits(&with_relic("RELIC.CRACKED_CORE"), "Cracked Core alone");
    assert_eq!(opening.document.player.get("lightning_channeled"), None);
}

/// `document` with the `lightning_channeled: 0` seed that #3389 added for a
/// Voltaic reachable only by generation removed, after asserting it is there
/// exactly when `seeded`.
///
/// The frozen oracle's opening digests predate #3389: it seeded the count
/// only for a deck Voltaic. A deck-only opening is otherwise unchanged, so
/// stripping the one field recovers the oracle's document.
/// #3660: the opening writes the bookkeeping record itself, and the oracle
/// digests above are compared with it removed, so the record is pinned here:
/// present, with exactly the bits its deck's closure keeps, and absent for a
/// deck that keeps none.
#[test]
fn the_opening_records_exactly_the_bookkeeping_its_deck_keeps() {
    for (cards, expected) in [
        (&[][..], None),
        (&["MISERY"][..], Some(serde_json::json!(["misery_ledger"]))),
        (
            &["NORMALITY"][..],
            Some(serde_json::json!(["normality_count"])),
        ),
        (
            &["THRUMMING_HATCHET"][..],
            Some(serde_json::json!(["self_return_uids"])),
        ),
        (
            &["MISERY", "NORMALITY", "THRUMMING_HATCHET"][..],
            Some(serde_json::json!([
                "misery_ledger",
                "normality_count",
                "self_return_uids"
            ])),
        ),
    ] {
        let opening = open(&owner_deck_with("CHARACTER.IRONCLAD", cards))
            .unwrap_or_else(|refusal| panic!("{cards:?} opens: {refusal}"));
        assert_eq!(
            opening.document.player.get("session_bookkeeping"),
            expected.as_ref(),
            "{cards:?}"
        );
        // The record is the opening catalog's own: loading the document
        // without it derives the same bits.
        let mut without = opening.document.clone();
        without.player.remove("session_bookkeeping");
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&without).unwrap();
        let state = crate::boundary::HotBoundary::from_canonical(&without, &catalog).unwrap();
        assert_eq!(
            crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
            opening.document,
            "{cards:?}"
        );
    }
}

/// The document as the frozen Python oracle would have written it: without
/// the Rust-only bookkeeping record (#3660, `catalog::SessionBookkeeping`).
/// The oracle digests below were taken before the record existed.
fn python_view(
    document: &crate::canonical::CanonicalStateV2,
) -> crate::canonical::CanonicalStateV2 {
    let mut document = document.clone();
    document.player.remove("session_bookkeeping");
    document
}

fn without_generated_voltaic_seed(
    document: &crate::canonical::CanonicalStateV2,
    seeded: bool,
) -> crate::canonical::CanonicalStateV2 {
    let mut document = python_view(document);
    let seed = document.player.remove("lightning_channeled");
    assert_eq!(seed, seeded.then(|| Value::from(0)), "the #3389 seed");
    document
}

fn admits_as_loaded(document: &crate::canonical::CanonicalStateV2) -> Result<(), String> {
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document)
        .map_err(|refusal| refusal.to_string())?;
    let state = crate::boundary::HotBoundary::from_canonical(document, &catalog)
        .map_err(|refusal| refusal.to_string())?;
    crate::engine::admission::admit(document, &state, &catalog)
        .map(|_| ())
        .map_err(|refusal| refusal.to_string())
}

/// A Voltaic reachable only by generation is counted from combat start
/// (#3389), one assertion block per branch of the seed and of
/// `engine::admission::lightning_channeled_tracking_is_admissible`.
///
/// Voltaic's multiplier reads the combat's `OrbChanneledEntry` history
/// (`<>c::<get_CanonicalVars>b__5_0`, RVA `0x3c6c9c`, `IL_0019`–`IL_0039`),
/// which exists whether or not a Voltaic does, so a Skill Potion Voltaic
/// counts the Lightning channeled before it was generated. The opening
/// therefore seeds the count for any closure Voltaic, not only a deck one.
#[test]
fn a_generation_reachable_voltaic_opens_tracked_and_admits() {
    use crate::ids::CardId;
    let voltaic = crate::catalog::CardIdentity {
        id: CardId::Voltaic,
        upgrade: 0,
        enchantment: None,
    };
    let in_a_pile = |document: &crate::canonical::CanonicalStateV2| {
        document
            .piles
            .values()
            .flatten()
            .any(|card| card.id == "VOLTAIC")
    };

    // Skill Potion, full profile: Voltaic is in the closure and in no pile,
    // and the opening seeds its count.
    let (document, catalog, state) =
        generation_potion_root_with("CHARACTER.DEFECT", "SKILL_POTION", &[], &[]);
    assert!(
        catalog.atom(&voltaic).is_some(),
        "Skill Potion reaches Voltaic"
    );
    assert!(crate::engine::admission::closure_holds_voltaic(&catalog));
    assert!(!in_a_pile(&document));
    assert_eq!(
        document.player.get("lightning_channeled"),
        Some(&Value::from(0))
    );
    assert_eq!(state.orbs.lightning_channeled(), 0);
    crate::engine::admit(&document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the tracked root admits: {refusal}"));

    // The same root untracked still admits: only a generated Voltaic can
    // read it, and `channel_voltaic` refuses that play by name.
    let mut untracked = document.clone();
    untracked.player.remove("lightning_channeled");
    assert_eq!(admits_as_loaded(&untracked), Ok(()));

    // Control: a Block Potion closure holds no Voltaic. The count stays
    // untracked, and a forced count has no reader and refuses.
    let (control, control_catalog, _) =
        generation_potion_root_with("CHARACTER.DEFECT", "BLOCK_POTION", &[], &[]);
    assert!(control_catalog.atom(&voltaic).is_none());
    assert!(!crate::engine::admission::closure_holds_voltaic(
        &control_catalog
    ));
    assert_eq!(control.player.get("lightning_channeled"), None);
    assert_eq!(admits_as_loaded(&control), Ok(()));
    let mut forced = control.clone();
    forced
        .player
        .insert("lightning_channeled".to_string(), Value::from(0));
    assert!(
        admits_as_loaded(&forced)
            .unwrap_err()
            .contains("unsupported orb queue state")
    );

    // A physically present Voltaic still needs its count.
    let present = opens_and_admits(&with_voltaic(empty_belt_case(), 0), "Voltaic L0");
    assert!(in_a_pile(&present.document));
    let mut present_untracked = present.document.clone();
    present_untracked.player.remove("lightning_channeled");
    assert!(
        admits_as_loaded(&present_untracked)
            .unwrap_err()
            .contains("unsupported orb queue state")
    );
}

/// `RELIC.BOUND_PHYLACTERY` summons a 1/1 Osty at combat start (#2827).
///
/// `BoundPhylactery/<BeforeCombatStart>d__8::MoveNext` (`0x3206b8`) calls
/// `SummonPet`, which is `OstyCmd::Summon(Owner, Summon = Decimal::One)`
/// (`0x32077c`, `IL_0039`). The body is
/// `engine::phylactery_before_combat_start`. The oracle projects
/// `ally: ['OSTY', 1, 1]` for this save, and the digest is the one it printed
/// with the recipe above `empty_belt_case`. This is not the control: a gate
/// that dropped the relic would still carry `bound_phylactery` but no Osty.
#[test]
fn a_bound_phylactery_save_summons_a_one_hp_osty_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.BOUND_PHYLACTERY")).expect("Bound Phylactery opens");
    assert_eq!(
        opening.document.player.get("ally"),
        Some(&serde_json::json!(["OSTY", 1, 1]))
    );
    assert_eq!(
        oracle_pet_digest(&opening.document, 3),
        "0cdfb01c3e67095bd2d671687ca64434ea30cc0e7ad7ea0c2555be95d9c3c7f6"
    );
}

/// `RELIC.BYRDPIP` adds its 9999/9999 passive pet at combat start (#2827).
///
/// `Byrdpip/<BeforeCombatStart>d__17::MoveNext` (`0x321120`) calls
/// `SummonPet`, which is `PlayerCmd::AddPet<Byrdpip>(Owner)` (`0x3211e4`,
/// `IL_0023`). The body is `engine::passive_relic_pets_before_combat_start`.
/// The oracle projects `relic_pets: [['BYRDPIP', 0, 9999, 9999]]` for this
/// save, and the digest is the one it printed with the recipe above
/// `empty_belt_case`.
#[test]
fn a_byrdpip_save_adds_its_passive_pet_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.BYRDPIP")).expect("Byrdpip opens");
    assert_eq!(
        opening.document.player.get("relic_pets"),
        Some(&serde_json::json!([["BYRDPIP", 0, 9999, 9999]]))
    );
    assert_eq!(
        oracle_pet_digest(&opening.document, 3),
        "3dda2292883d74e65fd42d0560a3c22874576cbe624b51d0606d1de7863c519e"
    );
}

/// `RELIC.PAELS_LEGION` adds its passive pet at combat start and leaves its
/// cooldown at the ready value through turn 1's `AfterSideTurnStart` (#2827).
///
/// `PaelsLegion/<BeforeCombatStart>d__32::MoveNext` (`0x32cbc4`) calls
/// `SummonPet` → `PlayerCmd::AddPet<PaelsLegion>(Owner)` (`0x32cc88`,
/// `IL_0023`). The turn-1 tick (`<AfterSideTurnStart>d__36`, `0x32c9d4`) runs
/// in `engine::relics::after_side_turn_start_late`. The cooldown starts at 0,
/// so the tick changes nothing the oracle projects. Digest printed by the
/// oracle with the recipe above `empty_belt_case`.
#[test]
fn a_paels_legion_save_adds_its_passive_pet_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.PAELS_LEGION")).expect("Pael's Legion opens");
    let player = &opening.document.player;
    assert_eq!(
        player.get("relic_pets"),
        Some(&serde_json::json!([["PAELS_LEGION", 0, 9999, 9999]]))
    );
    assert_eq!(
        player.get("paels_legion_cooldown"),
        Some(&serde_json::json!(0))
    );
    assert_eq!(
        oracle_pet_digest(&opening.document, 3),
        "dcc68834740254818d02a77d791c921fc912319ee55a969cca75f600a0000dc9"
    );
}

/// Both pets together, in both acquisition orders (#2827).
///
/// The oracle sorts the pet roster by kind (frozen Python `start_combat`, deleted #2827), so
/// `relic_pets` is the same either way. The digests differ only because
/// `relics_entering` keeps the save's order. Each is the oracle's, from the
/// recipe above `empty_belt_case` with both relics appended in the stated
/// order.
#[test]
fn byrdpip_and_paels_legion_together_match_the_oracle_in_either_order() {
    let pets = serde_json::json!([["BYRDPIP", 0, 9999, 9999], ["PAELS_LEGION", 0, 9999, 9999]]);
    for (order, digest) in [
        (
            ["RELIC.BYRDPIP", "RELIC.PAELS_LEGION"],
            "354485171224ac13b4f2c226223c4d5b4f67aeb603b1a3f2ba2534b8e10ad4ed",
        ),
        (
            ["RELIC.PAELS_LEGION", "RELIC.BYRDPIP"],
            "ca3b580f6ac12243a926f89c0f6571900f09c28a7b2fb41124098a51b2d82633",
        ),
    ] {
        let mut case = empty_belt_case();
        for id in order {
            relic(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": id}),
            );
        }
        let opening = open(&case).expect("both pets open");
        assert_eq!(opening.document.player.get("relic_pets"), Some(&pets));
        assert_eq!(oracle_pet_digest(&opening.document, 4), digest, "{order:?}");
    }
}

/// The Hand as `(id, upgrade)`, in pile order.
fn hand_levels(document: &crate::canonical::CanonicalStateV2) -> Vec<(String, i64)> {
    document.piles["hand"]
        .iter()
        .map(|card| (card.id.clone(), card.upgrade))
        .collect()
}

/// Every monster's `hp`, in roster order.
fn monster_hps(document: &crate::canonical::CanonicalStateV2) -> Vec<Value> {
    document
        .monsters
        .iter()
        .map(|monster| monster.get("hp").cloned().unwrap_or(Value::Null))
        .collect()
}

/// `RELIC.FESTIVE_POPPER` deals 9 unpowered damage to every hittable enemy on
/// the first `AfterPlayerTurnStart` walk (#2827).
///
/// `FestivePopper/<AfterPlayerTurnStart>d__4::MoveNext` (`0x324b94`); the body
/// is `engine::relics::festive_popper_after_player_turn_start`. The two
/// Toadpoles open at 26 and 25 in the control ([`EMPTY_BELT_DIGEST`]) and at 17
/// and 16 here, which is the oracle's document value for value. Popper beside
/// Emotion Chip is admitted by the oracle and runs in the shared fixed suffix,
/// so it is pinned too. Digests printed by the oracle with the recipe above
/// `empty_belt_case`.
#[test]
fn a_festive_popper_save_hits_every_enemy_for_nine_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.FESTIVE_POPPER")).expect("Festive Popper opens");
    assert_eq!(
        monster_hps(&opening.document),
        vec![Value::from(17), Value::from(16)]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "9dc55be92c755d89d197a512e3aab3bfd88cd4999b9fd4c738e8787fe5e7d8cc"
    );

    let mut with_chip = with_relic("RELIC.FESTIVE_POPPER");
    relic(
        &mut with_chip,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.EMOTION_CHIP"}),
    );
    let opening = open(&with_chip).expect("Festive Popper with Emotion Chip opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "192b054863495269f5e1afa31f23df0848a758f98adc21eebdccbd01f5e5fd76"
    );
}

/// `RELIC.BELLOWS` upgrades the dealt Hand once on the first
/// `AfterPlayerTurnStart` walk (#2827).
///
/// `Bellows::AfterPlayerTurnStart` (`0x906dc`, synchronous); the body is
/// `engine::relics::bellows_after_player_turn_start`. Three shapes:
///
/// * the starter Hand, every card taken from level 0 to 1;
/// * a deck with a Curse (`CLUMSY`, not upgradable) and `BASH`+1 (already at
///   its maximum) dealt into the Hand: both are skipped, and the rest upgrade.
///   `CardCmd.Upgrade` skips a card it cannot upgrade instead of refusing;
/// * Bellows with Festive Popper, the two new bodies in the one suffix.
///
/// Digests printed by the oracle with the recipe above `empty_belt_case`, the
/// deck rows inserted at index 0 as `{'floor_added_to_deck': 1, 'id': …,
/// 'current_upgrade_level': …}` in the order listed.
#[test]
fn a_bellows_save_upgrades_the_opening_hand_once_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.BELLOWS")).expect("Bellows opens");
    assert!(
        hand_levels(&opening.document)
            .iter()
            .all(|(_, upgrade)| *upgrade == 1),
        "{:?}",
        hand_levels(&opening.document)
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "4bb9769633a4223996a11d951020c7416582805400f7ec11ff7e3a59a5c5f401"
    );

    let mut mixed = with_relic("RELIC.BELLOWS");
    // Inserted at index 0 one by one, so the listed order is reversed in the
    // save, exactly as the oracle's recipe inserts them.
    for (id, level) in [
        ("CARD.BASH", 1),
        ("CARD.BASH", 1),
        ("CARD.CLUMSY", 0),
        ("CARD.CLUMSY", 0),
        ("CARD.STRIKE_IRONCLAD", 1),
    ] {
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": id});
        if level > 0 {
            row["current_upgrade_level"] = Value::from(level);
        }
        deck_row(&mut mixed, row);
    }
    let opening = open(&mixed).expect("Bellows with a mixed deck opens");
    assert_eq!(
        hand_levels(&opening.document),
        [
            ("CLUMSY", 0),
            ("DEFEND_IRONCLAD", 1),
            ("STRIKE_IRONCLAD", 1),
            ("STRIKE_IRONCLAD", 1),
            ("BASH", 1),
        ]
        .map(|(id, level)| (id.to_string(), level))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "8eb0e135b12cbcca84d97f48358650af884b8255cf4ce37715b1a6229f9973cf"
    );

    let mut with_popper = with_relic("RELIC.BELLOWS");
    relic(
        &mut with_popper,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.FESTIVE_POPPER"}),
    );
    let opening = open(&with_popper).expect("Bellows with Festive Popper opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "f36be49df65cbdae0749bf8c6404995bf24c615c3b1662074021a886c4bd4b5b"
    );
}

/// The same-`AfterPlayerTurnStart` co-ownerships the oracle **admits** beside
/// Bellows open digest-equal to it (#2827).
///
/// Both engines run the hook as one fixed suffix, and `start_combat` admits
/// Bellows beside each of these. So the order Rust runs them in has to be the
/// oracle's, and it is, peer by peer. Mercury Hourglass is a template relic, so
/// this also pins that the hand-authored suffix runs before
/// `_fire_relic_templates(s, "AfterPlayerTurnStart")`. Digests printed by the
/// oracle with the recipe above `empty_belt_case`.
#[test]
fn bellows_beside_each_oracle_admitted_peer_matches_the_oracle() {
    for (peer, digest) in [
        (
            "RELIC.MR_STRUGGLES",
            "acf7c596a34cfb7f4fdb43c63b2acdc35c8d7eb2085c6226ba6bf0e224ffd60b",
        ),
        (
            "RELIC.MERCURY_HOURGLASS",
            "7d2558dfb3cca123844298290b1c99c5f4b7c64c422d988cb58a13e3242b99d1",
        ),
        (
            "RELIC.ROYAL_POISON",
            "e5de23c7106ec8a1df5fccee8feecba31e8a7e8a430758a3d1849297129585cc",
        ),
        (
            "RELIC.EMOTION_CHIP",
            "240ae22ced3b731c7badcd5513448de0f29723d09c02a73b5117d4f77849ef06",
        ),
    ] {
        let mut case = with_relic("RELIC.BELLOWS");
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": peer}),
        );
        let opening = open(&case).unwrap_or_else(|refusal| panic!("Bellows + {peer}: {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "Bellows + {peer}"
        );
    }
}

/// Every same-`AfterPlayerTurnStart` co-ownership the oracle refuses around
/// the two new bodies refuses here too, by name (#2827).
///
/// Each pair below was fed to the oracle on the fixture save, and each raised
/// `NotImplementedError` in `start_combat`. Gambling Chip and Toasty Mittens
/// refuse for their peers (frozen Python, deleted #2827). Choices Paradox and
/// Vexing Puzzlebox refuse on this save for its partial unlock, ahead of their
/// peer rule. Bone Tea refuses beside Popper
/// once its charge is live. The enemy-damage count and Royal Poison refuse. Rust refuses all of them by
/// [`OpeningRefusal::TurnStartRelicOrderUnrecorded`] instead of emitting a
/// fixed-order document, which the pre-#2827 gate did only because it refused
/// the two relics outright.
///
/// Each peer is appended after the owner, so the fixture's vouched inventory
/// records the owner first. That is the order #2884 leaves refused for Bellows
/// beside Choices Paradox (the reverse is admitted, see
/// `choices_paradox_then_bellows_is_admitted_and_the_reverse_refuses`), and
/// Festive Popper beside Toasty Mittens has left this list: it is admitted in
/// either vouched order
/// (`festive_popper_runs_on_its_recorded_side_of_toasty_mittens`).
#[test]
fn the_oracles_same_hook_refusals_around_bellows_and_popper_refuse_by_name() {
    let cases = [
        ("RELIC.BELLOWS", "RELIC.GAMBLING_CHIP"),
        ("RELIC.BELLOWS", "RELIC.TOASTY_MITTENS"),
        ("RELIC.BELLOWS", "RELIC.CHOICES_PARADOX"),
        ("RELIC.BELLOWS", "RELIC.VEXING_PUZZLEBOX"),
        ("RELIC.FESTIVE_POPPER", "RELIC.GAMBLING_CHIP"),
        ("RELIC.FESTIVE_POPPER", "RELIC.CHOICES_PARADOX"),
        ("RELIC.FESTIVE_POPPER", "RELIC.VEXING_PUZZLEBOX"),
        ("RELIC.FESTIVE_POPPER", "RELIC.BONE_TEA"),
        ("RELIC.FESTIVE_POPPER", "RELIC.MR_STRUGGLES"),
        ("RELIC.FESTIVE_POPPER", "RELIC.MERCURY_HOURGLASS"),
        ("RELIC.FESTIVE_POPPER", "RELIC.ROYAL_POISON"),
    ];
    for (owner, peer) in cases {
        let mut case = with_relic(owner);
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": peer});
        if peer == "RELIC.BONE_TEA" {
            row["props"] = serde_json::json!({"ints": [{"name": "CombatsLeft", "value": 1}]});
        }
        relic(&mut case, row);
        let refusal = open(&case).unwrap_err();
        assert_eq!(
            refusal.class(),
            "turn_start_relic_order_unrecorded",
            "{owner} + {peer}: {refusal}"
        );
        assert!(refusal.to_string().contains(owner), "{refusal}");
        assert!(refusal.to_string().contains(peer), "{refusal}");
    }
}

// ---------------------------------------------------------------------------
// Same-`AfterPlayerTurnStart` relic order read from the inventory (#2884)
// ---------------------------------------------------------------------------

/// [`coach_case`] holding `relics` in that inventory order, opened with the
/// inventory vouched for as dispatch order or not. A `full_profile` case also
/// records the `combat_orbs` stream, which a generation relic's pool reaches.
fn open_in_order(
    relics: &[&str],
    full_profile: bool,
    dispatch_ordered: bool,
) -> Result<Opening, OpeningRefusal> {
    let mut facts = entry_facts(&coach_case(None, relics, &[], full_profile, full_profile));
    facts.relic_entry.dispatch_ordered = dispatch_ordered;
    build_opening(&facts, &OpeningOptions::default())
}

fn monster_damage_taken(document: &crate::canonical::CanonicalStateV2) -> Vec<i64> {
    document
        .monsters
        .iter()
        .map(|monster| monster["max_hp"].as_i64().unwrap() - monster["hp"].as_i64().unwrap())
        .collect()
}

/// Resolve the opening's pending relic choice with its first legal action.
fn resolve_first_choice(opening: &Opening) -> (crate::hot::HotState, crate::catalog::Catalog) {
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .expect("the opened document builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
        .expect("the opened document hydrates");
    let actions = crate::engine::legal_actions(&state, &catalog);
    assert!(
        actions
            .iter()
            .all(|action| matches!(action, crate::engine::Action::Select { .. })),
        "the opening waits on the relic choice: {actions:?}"
    );
    let next = crate::engine::apply_action(&state, &catalog, &actions[0])
        .expect("the relic choice resolves");
    (next.state, catalog)
}

/// Festive Popper and Toasty Mittens do not commute, and the engine runs
/// Popper on whichever side of Toasty the vouched inventory records (#2884).
///
/// `FestivePopper/<AfterPlayerTurnStart>d__4` (RVA `0x324b94`) hits every
/// enemy for 9 on turn one; `ToastyMittens/<AfterPlayerTurnStart>d__6` (RVA
/// `0x33285c`) pauses on a Hand choice (`CardSelectCmd::FromHand`,
/// `IL_0068`), then exhausts it and grants 1 Strength. Native awaits them in
/// `Player.Relics` order, so the two orders offer the choice against different
/// enemy HP:
///
/// * **Popper first** (every corpus fight that met the refusal, e.g.
///   MT2A8Y7JG6V5 with Popper at index 3 and Toasty at 8): the choice is
///   offered after the 9 has landed, and resolving it deals nothing more;
/// * **Toasty first:** the choice is offered to undamaged enemies, and Popper's
///   9 lands after it resolves.
///
/// An unvouched inventory cannot say which, and refuses by name either way.
#[test]
fn festive_popper_runs_on_its_recorded_side_of_toasty_mittens() {
    const POPPER: &str = "RELIC.FESTIVE_POPPER";
    const TOASTY: &str = "RELIC.TOASTY_MITTENS";
    let popper_first = open_in_order(&[POPPER, TOASTY], false, true)
        .unwrap_or_else(|refusal| panic!("Popper then Toasty opens: {refusal}"));
    let toasty_first = open_in_order(&[TOASTY, POPPER], false, true)
        .unwrap_or_else(|refusal| panic!("Toasty then Popper opens: {refusal}"));
    for opening in [&popper_first, &toasty_first] {
        assert_eq!(
            opening.document.player["pending"][0],
            Value::from("toasty_mittens_select")
        );
    }
    assert_eq!(monster_damage_taken(&popper_first.document), [9, 9]);
    assert_eq!(monster_damage_taken(&toasty_first.document), [0, 0]);
    assert_ne!(
        popper_first.document.differential_digest(),
        toasty_first.document.differential_digest(),
        "the two orders are distinct decision points"
    );

    for (opening, order) in [
        (&popper_first, "Popper first"),
        (&toasty_first, "Toasty first"),
    ] {
        let (state, catalog) = resolve_first_choice(opening);
        let after = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(
            monster_damage_taken(&after),
            [9, 9],
            "{order}: Popper hit once"
        );
        assert_eq!(
            state.powers.value(crate::ids::PowerId::Strength),
            1,
            "{order}: Toasty's Strength"
        );
        assert_eq!(
            state.piles.get(crate::hot::PileId::Exhaust).len(),
            1,
            "{order}"
        );
        assert!(state.pending.is_none(), "{order}: the turn is the player's");
    }

    for relics in [[POPPER, TOASTY], [TOASTY, POPPER]] {
        let refusal = open_in_order(&relics, false, false).unwrap_err();
        assert_eq!(
            refusal.class(),
            "turn_start_relic_order_unrecorded",
            "{relics:?}: {refusal}"
        );
        let text = refusal.to_string();
        assert!(text.contains(POPPER) && text.contains(TOASTY), "{text}");
    }
}

/// Choices Paradox and Bellows do not commute, and only the order the engine's
/// fixed suffix runs is admitted (#2884).
///
/// `ChoicesParadox/<AfterPlayerTurnStart>d__6` (RVA `0x321b0c`) adds its pick
/// to the Hand (`CardPileCmd::AddGeneratedCardToCombat`, `IL_01d0`), and
/// `Bellows::AfterPlayerTurnStart` (RVA `0x906dc`) upgrades the Hand
/// (`CardCmd::Upgrade`, `IL_004c`), so the pick is upgraded exactly when
/// Choices Paradox runs first. That is the corpus order (TZZ1FU1T6K9G, Choices
/// Paradox at index 13 and Bellows at 15) and the fixed suffix's, so it opens,
/// and after the pick the whole Hand, the pick included, is upgraded. Bellows
/// first, or an unvouched inventory, refuses by name. The profile is fully
/// unlocked so the Paradox's owner pool is exact.
#[test]
fn choices_paradox_then_bellows_is_admitted_and_the_reverse_refuses() {
    const PARADOX: &str = "RELIC.CHOICES_PARADOX";
    const BELLOWS: &str = "RELIC.BELLOWS";
    let opening = open_in_order(&[PARADOX, BELLOWS], true, true)
        .unwrap_or_else(|refusal| panic!("Choices Paradox then Bellows opens: {refusal}"));
    assert!(
        opening
            .document
            .piles
            .get("hand")
            .into_iter()
            .flatten()
            .all(|card| card.upgrade == 0),
        "Bellows has not run while the Paradox choice is open"
    );
    let (state, catalog) = resolve_first_choice(&opening);
    let after = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let hand = &after.piles["hand"];
    assert_eq!(hand.len(), 6, "the five dealt cards and the pick");
    for card in hand {
        assert_eq!(card.upgrade, 1, "{} is upgraded by Bellows", card.id);
    }

    for (relics, ordered) in [
        ([BELLOWS, PARADOX], true),
        ([BELLOWS, PARADOX], false),
        ([PARADOX, BELLOWS], false),
    ] {
        let refusal = open_in_order(&relics, true, ordered).unwrap_err();
        assert_eq!(
            refusal.class(),
            "turn_start_relic_order_unrecorded",
            "{relics:?} vouched {ordered}: {refusal}"
        );
        let text = refusal.to_string();
        assert!(text.contains(BELLOWS) && text.contains(PARADOX), "{text}");
    }
}

/// The #2884 rows among *other* same-hook relics: Gambling Chip or Toasty
/// Mittens beside Mr Struggles never run as a silent fixed order.
///
/// Both pause on a Hand choice (`GamblingChip/<AfterPlayerTurnStart>d__2` RVA
/// `0x325788`, `CardSelectCmd::FromHandForDiscard` at `IL_0071`; Toasty at
/// `IL_0068` as above), and Mr Struggles acts every turn, so native runs its
/// damage before or after that pause by inventory order. At the pause the
/// engine's `after_player_turn_start_pause_order_is_native` admits the vouched
/// order it runs (the pauser first) and refuses the reverse and an unvouched
/// inventory by name.
#[test]
fn hand_choice_relics_beside_mr_struggles_follow_the_recorded_order() {
    const STRUGGLES: &str = "RELIC.MR_STRUGGLES";
    const PAUSE_ORDER: &str =
        "AfterPlayerTurnStart relic order around a paused SetupPlayerTurn choice";
    for pauser in ["RELIC.GAMBLING_CHIP", "RELIC.TOASTY_MITTENS"] {
        let opening = open_in_order(&[pauser, STRUGGLES], false, true)
            .unwrap_or_else(|refusal| panic!("{pauser} then Mr Struggles opens: {refusal}"));
        assert!(
            opening.document.player.contains_key("pending"),
            "{pauser}: the choice is open"
        );
        assert_eq!(
            monster_damage_taken(&opening.document),
            [0, 0],
            "{pauser}: Mr Struggles runs after the choice"
        );
        for (relics, ordered) in [([STRUGGLES, pauser], true), ([pauser, STRUGGLES], false)] {
            let refusal = open_in_order(&relics, false, ordered).unwrap_err();
            assert!(
                refusal.to_string().contains(PAUSE_ORDER),
                "{relics:?} vouched {ordered}: {refusal}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mercury Hourglass and Vexing Puzzlebox recorded before Choices Paradox (#3321)
// ---------------------------------------------------------------------------

/// A `character` owner's starter deck on a fully unlocked profile, holding
/// `relics` in that inventory order, opened with the inventory vouched for
/// as dispatch order or not.
fn open_owner_in_order(
    character: &str,
    relics: &[&str],
    dispatch_ordered: bool,
) -> Result<Opening, OpeningRefusal> {
    let mut case = with_orb_stream(owner_deck_with(character, &[]));
    for id in relics {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    let mut facts = entry_facts(&case);
    facts.relic_entry.dispatch_ordered = dispatch_ordered;
    build_opening(&facts, &OpeningOptions::default())
}

fn generation_options(document: &crate::canonical::CanonicalStateV2) -> Vec<String> {
    assert_eq!(
        document.player["pending"][0],
        Value::from("generation_relic_select")
    );
    document.player["pending"][2]
        .as_array()
        .expect("the options are an array")
        .iter()
        .map(|option| option[0].as_str().unwrap().to_owned())
        .collect()
}

/// The BR2R60965GJ1 shape (#3321): a Necrobinder holding Mercury Hourglass
/// (inventory index 2), Vexing Puzzlebox (14) and Choices Paradox (15), the
/// inventory vouched for. Native awaits the three in `Player.Relics` order
/// (`Hook/<AfterPlayerTurnStart>d__56` RVA `0x3cff0c`, `IL_012d`/`IL_017a`), so
/// Hourglass's 3 (`MercuryHourglass/<AfterPlayerTurnStart>d__4` RVA
/// `0x32a854`, `IL_0067`) and the Puzzlebox card (`0x333de0`, `IL_00b7`) are
/// both in place when the Paradox grid pauses (`0x321b0c`, `IL_0144`), and
/// the Puzzlebox draws `CombatCardGeneration` first, so the grid differs from
/// the fixed order's. Both orders are witnessed:
///
/// * recorded first, the opening pauses with the enemies hit and a six-card
///   Hand;
/// * the fixed order recorded (Paradox, Puzzlebox, Hourglass), it pauses
///   before either, on a different grid;
/// * after the pick, both hold seven cards and exactly one Hourglass hit;
/// * each move alone: Puzzlebox before Paradox without Hourglass;
/// * an unvouched inventory, and Hourglass recorded between the two (so it
///   cannot lead), still refuse at the pause by name.
#[test]
fn hourglass_and_puzzlebox_recorded_before_choices_paradox_run_ahead_of_its_grid() {
    const NECROBINDER: &str = "CHARACTER.NECROBINDER";
    const HOURGLASS: &str = "RELIC.MERCURY_HOURGLASS";
    const PUZZLEBOX: &str = "RELIC.VEXING_PUZZLEBOX";
    const PARADOX: &str = "RELIC.CHOICES_PARADOX";
    const PAUSE_ORDER: &str =
        "AfterPlayerTurnStart relic order around a paused SetupPlayerTurn choice";
    let recorded = open_owner_in_order(NECROBINDER, &[HOURGLASS, PUZZLEBOX, PARADOX], true)
        .unwrap_or_else(|refusal| panic!("the recorded order opens: {refusal}"));
    let fixed = open_owner_in_order(NECROBINDER, &[PARADOX, PUZZLEBOX, HOURGLASS], true)
        .unwrap_or_else(|refusal| panic!("the fixed order opens: {refusal}"));
    assert!(
        monster_damage_taken(&recorded.document)
            .iter()
            .all(|taken| *taken == 3),
        "Hourglass hit ahead of the grid"
    );
    assert!(
        monster_damage_taken(&fixed.document)
            .iter()
            .all(|taken| *taken == 0),
        "Hourglass waits behind the grid"
    );
    assert_eq!(
        recorded.document.piles["hand"].len(),
        6,
        "the Puzzlebox card"
    );
    assert_eq!(fixed.document.piles["hand"].len(), 5);
    assert_ne!(
        generation_options(&recorded.document),
        generation_options(&fixed.document),
        "the Puzzlebox drew CombatCardGeneration first"
    );
    for (opening, order) in [(&recorded, "recorded"), (&fixed, "fixed")] {
        let (state, catalog) = resolve_first_choice(opening);
        let after = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(
            monster_damage_taken(&after).iter().all(|taken| *taken == 3),
            "{order}: Hourglass hit once"
        );
        assert_eq!(after.piles["hand"].len(), 7, "{order}");
        assert!(state.pending.is_none(), "{order}");
    }

    let puzzlebox_first = open_owner_in_order(NECROBINDER, &[PUZZLEBOX, PARADOX], true)
        .unwrap_or_else(|refusal| panic!("Puzzlebox then Paradox opens: {refusal}"));
    let paradox_first = open_owner_in_order(NECROBINDER, &[PARADOX, PUZZLEBOX], true)
        .unwrap_or_else(|refusal| panic!("Paradox then Puzzlebox opens: {refusal}"));
    assert_eq!(puzzlebox_first.document.piles["hand"].len(), 6);
    assert_eq!(paradox_first.document.piles["hand"].len(), 5);
    assert_eq!(
        generation_options(&recorded.document),
        generation_options(&puzzlebox_first.document),
        "Hourglass draws no RNG"
    );

    for (relics, ordered) in [
        ([HOURGLASS, PUZZLEBOX, PARADOX], false),
        ([PUZZLEBOX, HOURGLASS, PARADOX], true),
    ] {
        let refusal = open_owner_in_order(NECROBINDER, &relics, ordered).unwrap_err();
        assert!(
            refusal.to_string().contains(PAUSE_ORDER),
            "{relics:?} vouched {ordered}: {refusal}"
        );
    }
}

/// Choices Paradox draws its owner's unlocked pool (#3321):
/// `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext` (RVA `0x321b0c`)
/// reads `Owner.Character.CardPool.GetUnlockedCards(..)` at
/// `IL_0064`-`IL_0089`. A Necrobinder's grid is Necrobinder cards, and its
/// pick lands in the Hand; an Ironclad's is the frozen row, unchanged; a
/// profile hiding the Necrobinder's gating epochs refuses by name, as Vexing
/// Puzzlebox's does.
#[test]
fn choices_paradox_offers_its_owners_pool() {
    use crate::catalog::RewardPool;
    use crate::steps::neutral::owner_generation_pool;
    const PARADOX: &str = "RELIC.CHOICES_PARADOX";
    let necrobinder = open_owner_in_order("CHARACTER.NECROBINDER", &[PARADOX], true)
        .unwrap_or_else(|refusal| panic!("a Necrobinder Paradox opens: {refusal}"));
    let pool = owner_generation_pool(RewardPool::Necrobinder, None);
    let options = generation_options(&necrobinder.document);
    assert_eq!(options.len(), 5);
    for option in &options {
        let id = crate::ids::CardId::from_str(option).unwrap();
        assert!(pool.contains(&id), "{option} is a Necrobinder card");
    }
    let (state, catalog) = resolve_first_choice(&necrobinder);
    let after = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert_eq!(
        after.piles["hand"].last().map(|card| card.id.as_str()),
        Some(options[0].as_str())
    );

    let ironclad = open_owner_in_order("CHARACTER.IRONCLAD", &[PARADOX], true)
        .unwrap_or_else(|refusal| panic!("an Ironclad Paradox opens: {refusal}"));
    let frozen = crate::content_tables::GENERATION_RELIC_POOLS
        .iter()
        .find_map(|(name, pool)| (*name == "CHOICES_PARADOX").then_some(*pool))
        .unwrap();
    for option in generation_options(&ironclad.document) {
        let id = crate::ids::CardId::from_str(&option).unwrap();
        assert!(frozen.contains(&id), "{option} is in the frozen row");
    }

    let mut hidden = with_orb_stream(owner_deck_with("CHARACTER.NECROBINDER", &[]));
    relic(
        &mut hidden,
        serde_json::json!({"floor_added_to_deck": 1, "id": PARADOX}),
    );
    hidden["save"]["players"][0]["unlock_state"]["unlocked_epochs"]
        .as_array_mut()
        .expect("the profile is an array")
        .retain(|epoch| {
            !matches!(
                epoch.as_str(),
                Some(
                    "EPOCH.NECROBINDER2_EPOCH"
                        | "EPOCH.NECROBINDER5_EPOCH"
                        | "EPOCH.NECROBINDER7_EPOCH"
                )
            )
        });
    let refusal = open(&hidden).expect_err("a hidden owner profile refuses");
    assert!(
        refusal
            .to_string()
            .contains("Choices Paradox owner pool under a profile hiding"),
        "{refusal}"
    );
}

/// Ghost Seed's room-entry Ethereal survives Bellows' turn-one upgrade (#2827,
/// the cross-slice pass against #2905).
///
/// Ghost Seed marks every Basic Strike and Defend with a **local** Ethereal
/// keyword at `AfterRoomEntered`, long before the first `AfterPlayerTurnStart`
/// walk, and Bellows then upgrades the dealt Hand. `CardCmd.Upgrade` changes
/// the card's level in place and keeps its local keywords —
/// `CardModel::UpgradeInternal` (v0.111.0 RVA `0x7e0c8`) only bumps
/// `CurrentUpgradeLevel` on the same instance, calls `OnUpgrade` (for the
/// starter Strike and Defend, `DynamicVar::UpgradeValueBy` on Damage/Block:
/// `0xed3c7`, `0xdd23f`) and recalculates dynamic vars, touching no keyword —
/// and
/// `engine::cards::upgrade_live_cards_once` rewrites only the pile atom, so the
/// per-uid keyword state is untouched. Both acquisition orders are pinned
/// (they differ only in `relics_entering`), and a third case adds Ring of the
/// Snake so the upgrade reaches the two extra turn-one draws as well. Digests
/// printed by the oracle with the recipe above `empty_belt_case`.
#[test]
fn ghost_seed_ethereal_survives_the_bellows_upgrade_and_matches_the_oracle() {
    for (relics, hand, digest) in [
        (
            &["RELIC.GHOST_SEED", "RELIC.BELLOWS"][..],
            5,
            "d0f4d593b89650d3bd8976ecab8c0c57bbf5c284e3eab6d6057613ca3608d8d8",
        ),
        (
            &["RELIC.BELLOWS", "RELIC.GHOST_SEED"][..],
            5,
            "40090634bd62acf56e01af1f57fb28231d89b76602d2170d2328cf1d47afcc49",
        ),
        (
            &[
                "RELIC.GHOST_SEED",
                "RELIC.BELLOWS",
                "RELIC.RING_OF_THE_SNAKE",
            ][..],
            7,
            "3cead984f2d9623d01be2d1876356c67e66ad5c6dde6367eb71f05a0a1c62ed0",
        ),
    ] {
        let mut case = empty_belt_case();
        for id in relics {
            relic(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": id}),
            );
        }
        let opening = open(&case).unwrap_or_else(|err| panic!("{relics:?} opens: {err}"));
        let dealt = &opening.document.piles["hand"];
        assert_eq!(dealt.len(), hand, "{relics:?}");
        for card in dealt {
            assert_eq!(card.upgrade, 1, "{relics:?}: {} upgraded", card.id);
            assert!(
                card.local_keywords.iter().any(|k| k == "Ethereal"),
                "{relics:?}: {} kept Ghost Seed's Ethereal through the upgrade",
                card.id
            );
        }
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relics:?}"
        );
    }
}

/// The two new bodies beside each relic the parallel #2827 lanes retired, all
/// admitted by the oracle and digest-equal to it (cross-slice pass).
///
/// None of these share `AfterPlayerTurnStart`, so none reorders the fixed
/// suffix; what the pass has to rule out is an earlier hook changing what the
/// suffix sees. Ring of the Snake and Pollinous Core (`TurnsSeen` 3, which
/// fires on turn one) grow the Hand Bellows upgrades to seven. Girya and Vajra
/// give Strength, which Festive Popper's unpowered damage must ignore (both
/// stay at 17 and 16). Pael's Flesh and Letter Opener are `AfterSideTurnStart`
/// peers whose turn-one effect is nil, Symbiotic Virus channels on turn one,
/// Vambrace, Kusarigama, Phylactery Unbound, Byrdpip and Pael's Legion act at
/// combat start. Digests printed by the oracle with the recipe above
/// `empty_belt_case`, the owner appended first and the peer second (the order
/// is in `relics_entering`, so it moves the digest), counter rows in
/// `with_relic_counter`'s shape.
#[test]
fn bellows_and_popper_beside_the_parallel_lanes_relics_match_the_oracle() {
    // (owner, (peer, saved counter), dealt Hand size, oracle digest)
    type Peer = (&'static str, Option<(&'static str, i64)>);
    let cases: [(&str, Peer, usize, &str); 16] = [
        (
            "RELIC.BELLOWS",
            ("RELIC.RING_OF_THE_SNAKE", None),
            7,
            "d571a83cf5238bbf12f93efb3ba4b98f10087f7d3001f87d8f08ce4afa7e3b6f",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.POLLINOUS_CORE", Some(("TurnsSeen", 3))),
            7,
            "e4b77c657257b7d77c076eac6a818c98345c2a6259055e920f9018284d288104",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.PAELS_FLESH", None),
            5,
            "dc167ca9cc3e07061a81f7ccde18134463e79661afaf69cad44f69d1f7572481",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.LETTER_OPENER", None),
            5,
            "4c1794e45eb10febbd3505731dc41d500256a44d744c276d2633089a4df1f51f",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.KUSARIGAMA", None),
            5,
            "ea410af5950fa4819bbd8c7fa145e371e36f1c211f127090e8d04a5a20ff977f",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.PHYLACTERY_UNBOUND", None),
            5,
            "904ac10ee0cc80451e4f5f66f8e6176a6f1a2cd68f14502552f1268b48ae49cd",
        ),
        (
            "RELIC.BELLOWS",
            ("RELIC.SYMBIOTIC_VIRUS", None),
            5,
            "9ff47c94639207624495ef81b79f3a8d79be0d9636d3fc5c602f0b7baa870888",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.GIRYA", Some(("TimesLifted", 2))),
            5,
            "f46f9aee12dfe0a16399466038217f3b1766685ad05d6e0a783a5aa37abf01d7",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.VAJRA", None),
            5,
            "7b6305d8629f647535b57b60d9d74c9bce1dd8e40cda518268babe2528a54a70",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.PAELS_FLESH", None),
            5,
            "e00cdca472431dfc49e13833e746c976592f3c8a289144e0362fbfefb9692e98",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.LETTER_OPENER", None),
            5,
            "52e27689f8cc84676583c4d7dce822d084b9cdeac94d4eb40aba50b8f7034282",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.SYMBIOTIC_VIRUS", None),
            5,
            "d7cbf834f367e444c84b5f1e81cc78e33d4ad474b03bfef17879ecc2002b3ce2",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.VAMBRACE", None),
            5,
            "35741501d8e9660e8e0d05f07eb835e1169fc784f78a2b065ed56712817e6bd2",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.BYRDPIP", None),
            5,
            "2bb8e6e9e7bc50e45544381caf871b41f2fdfc1d66441b716be38ef3f3681b59",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.PAELS_LEGION", None),
            5,
            "a6fc563290f8e1a402debed93554f9e1ec7a3eef4ec4ba014f6f6702535e4e1b",
        ),
        (
            "RELIC.FESTIVE_POPPER",
            ("RELIC.GHOST_SEED", None),
            5,
            "472ebff468b7e7e115c3b3f92f2a3ab006a8be85b15358e25365ecef76a8def8",
        ),
    ];
    for (owner, (peer, counter), hand, digest) in cases {
        let mut case = with_relic(owner);
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": peer});
        if let Some((name, value)) = counter {
            row["props"] = serde_json::json!({"ints": [{"name": name, "value": value}]});
        }
        relic(&mut case, row);
        let opening = open(&case).unwrap_or_else(|err| panic!("{owner} + {peer} opens: {err}"));
        assert_eq!(hand_size(&opening.document), hand, "{owner} + {peer}");
        if owner == "RELIC.BELLOWS" {
            assert!(
                hand_levels(&opening.document)
                    .iter()
                    .all(|(_, upgrade)| *upgrade == 1),
                "{owner} + {peer}"
            );
        } else {
            assert_eq!(
                monster_hps(&opening.document),
                vec![Value::from(17), Value::from(16)],
                "{owner} + {peer}"
            );
        }
        // A pet peer advances the creature counter the oracle never
        // tracked (#3039); the rest is still its document.
        let actual = if matches!(
            peer,
            "RELIC.BYRDPIP" | "RELIC.PAELS_LEGION" | "RELIC.PHYLACTERY_UNBOUND"
        ) {
            oracle_pet_digest(&opening.document, 3)
        } else {
            python_view(&opening.document).differential_digest()
        };
        assert_eq!(actual, digest, "{owner} + {peer}");
    }
}

// ---------------------------------------------------------------------------
// Girya and Petrified Toad (#2827)
// ---------------------------------------------------------------------------
//
// Every digest below was printed by the oracle with the recipe in the comment
// block above `empty_belt_case`, the per-case mutation named in each test, and
// the control `EMPTY_BELT_DIGEST` reproduced by the same script in the same
// run. Girya's saved property is the shape every corpus save records:
// `{'id': 'RELIC.GIRYA', 'props': {'ints': [{'name': 'TimesLifted',
// 'value': n}]}}`.

fn with_girya(lifts: i64) -> Value {
    with_relic_counter("RELIC.GIRYA", "TimesLifted", lifts)
}

/// `RELIC.GIRYA` applies its saved lift count as Strength on entering a
/// combat room, and nothing at all with no lifts.
///
/// `Girya/<AfterRoomEntered>d__14::MoveNext` (`0x325a60`) `leave`s at
/// `IL_0029` unless `TimesLifted > 0`, then applies `StrengthPower` for
/// `(decimal)TimesLifted` at `IL_006e`. The zero case is not the control: its
/// document still carries the relic in `relics_entering`, so a digest equal to
/// `EMPTY_BELT_DIGEST` would mean the relic was dropped.
#[test]
fn a_girya_save_grants_its_lift_count_as_strength_and_matches_the_oracle() {
    for (lifts, strength, digest) in [
        (
            0,
            None,
            "2cbdfd09836668ef1608f697a1e14f386659e0feb782a23b6f699e17434e6737",
        ),
        (
            1,
            Some(1),
            "1235db293ff94fc69497eb5af500fe2164dc862325495f5de0e4743797963082",
        ),
        (
            2,
            Some(2),
            "bc5ac6fb4d8c9d4d7dfbfa11c4d609330250eaf918ebc56a000931371169c5d0",
        ),
        (
            3,
            Some(3),
            "37d9f0b6754e05611b937db67c32192ac8b9ecb00602b249cdb53ecb3d0e8c85",
        ),
    ] {
        let opening = open(&with_girya(lifts)).expect("a Girya save opens");
        assert_eq!(
            opening.document.player.get("strength"),
            strength.map(Value::from).as_ref(),
            "TimesLifted {lifts}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "TimesLifted {lifts}"
        );
        assert_ne!(
            python_view(&opening.document).differential_digest(),
            EMPTY_BELT_DIGEST
        );
    }
}

/// Girya's grant goes through the engine's owner-Strength command beside
/// Vajra's and Ember Tea's, so Ruined Helmet doubles the first one and
/// latches; equal amounts commute and open; distinct amounts refuse by the
/// oracle's own name.
#[test]
fn girya_stacks_with_the_other_room_entry_strength_sources_like_the_oracle() {
    let tea = serde_json::json!({
        "floor_added_to_deck": 1, "id": "RELIC.EMBER_TEA",
        "props": {"ints": [{"name": "CombatsLeft", "value": 3}]}});
    let helmet = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.RUINED_HELMET"});
    let vajra = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.VAJRA"});
    let build = |lifts: i64, rows: &[&Value]| {
        let mut case = with_girya(lifts);
        for row in rows {
            relic(&mut case, (*row).clone());
        }
        case
    };

    // (case, Strength, Ruined Helmet latched, Ember Tea charge left, digest)
    for (name, case, strength, latched, tea_left, digest) in [
        (
            "Girya 2 + Ruined Helmet",
            build(2, &[&helmet]),
            4,
            Some(true),
            None,
            "b7ca6ed7e8c738b56aa9a382cba321c2d9516b78ba54a9ca39590ba1e0044f5c",
        ),
        (
            "Girya 1 + Ruined Helmet",
            build(1, &[&helmet]),
            2,
            Some(true),
            None,
            "fc99a75ba828e8fcd69628f695a590e9388267904b7df52ccdc8ea772e8f2fdf",
        ),
        (
            "Girya 2 + Ember Tea",
            build(2, &[&tea]),
            4,
            None,
            Some(2),
            "96fa41686aeca35391912cc914db9e3903b7f3d3fd816059ffea8489e8fbf2e3",
        ),
        // Girya 2 and Ember Tea 2 are equal amounts, so the doubled one is
        // the same whichever the game applies first.
        (
            "Girya 2 + Ember Tea + Ruined Helmet",
            build(2, &[&tea, &helmet]),
            6,
            Some(true),
            Some(2),
            "c7130904e52b3814cdf2d88b4d6c823831f37c115f4e25fdf549cc63087f8d2c",
        ),
        (
            "Girya 1 + Vajra + Ruined Helmet",
            build(1, &[&vajra, &helmet]),
            3,
            Some(true),
            None,
            "ff580d06a7f0aa63dcb2745fc8228b2fc84367d85efd6ed18f27f98d8dcfd97e",
        ),
    ] {
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        let player = &opening.document.player;
        assert_eq!(
            player.get("strength"),
            Some(&Value::from(strength)),
            "{name}"
        );
        assert_eq!(
            player.get("ruined_helmet_used"),
            latched.map(Value::Bool).as_ref(),
            "{name}"
        );
        assert_eq!(
            player.get("ember_tea_combats_left"),
            tea_left.map(Value::from).as_ref(),
            "{name}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{name}"
        );
    }

    // Oracle: `RUINED_HELMET with distinct AfterRoomEntered Strength sources
    // [('RELIC.GIRYA', 1), ('RELIC.EMBER_TEA', 2)]`.
    let refusal = open(&build(1, &[&tea, &helmet])).expect_err("distinct amounts refuse");
    assert_eq!(refusal.class(), "ruined_helmet_strength_order");
    assert!(refusal.to_string().contains("RELIC.GIRYA=1"), "{refusal}");
    assert!(
        refusal.to_string().contains("RELIC.EMBER_TEA=2"),
        "{refusal}"
    );

    // A Girya with no lifts applies nothing and is not a source, exactly as
    // the oracle leaves it out of the list: the refusal names only the two
    // relics that apply (`[('RELIC.VAJRA', 1), ('RELIC.EMBER_TEA', 2)]`).
    let refusal = open(&build(0, &[&vajra, &tea, &helmet])).expect_err("Vajra and Tea refuse");
    assert_eq!(refusal.class(), "ruined_helmet_strength_order");
    assert!(!refusal.to_string().contains("RELIC.GIRYA"), "{refusal}");
}

/// A Girya counter outside `0..=GIRYA_MAX_LIFTS`, or absent, refuses as the
/// oracle does (`GIRYA TimesLifted is not an exact integer in the native 0..3
/// range: 4` / `-1` / `None`).
#[test]
fn a_girya_counter_outside_its_native_domain_refuses() {
    for counter in [Value::from(-1), Value::from(4), Value::Null] {
        let mut case = empty_belt_case();
        let props = if counter.is_null() {
            serde_json::json!({})
        } else {
            serde_json::json!({"ints": [{"name": "TimesLifted", "value": counter}]})
        };
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.GIRYA", "props": props}),
        );
        let refusal = open(&case).expect_err("an inexact lift count refuses");
        assert_eq!(refusal.class(), "relic_counter_not_exact", "{counter}");
        assert!(refusal.to_string().contains("RELIC.GIRYA"), "{refusal}");
    }
}

/// The fixture case with its standing two-slot belt admitted: every epoch in
/// the universe unlocked, which makes the potion pool fully unlocked and
/// proves a materialised belt's pool at the boundary (see
/// `the_post_deal_document_stops_at_the_existing_potion_belt_boundary`).
/// Mutation: `save['players'][0]['unlock_state']['unlocked_epochs'] =
/// ['EPOCH.' + e for e in UNLOCK_EPOCH_UNIVERSE_V1101]`; the oracle's digest
/// for it untouched is [`FULL_PROFILE_DIGEST`].
fn full_profile_case() -> Value {
    let mut case = case();
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
        crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
            .collect(),
    );
    case
}

/// The oracle's digest for [`full_profile_case`] untouched: the control for
/// the Petrified Toad witnesses.
const FULL_PROFILE_DIGEST: &str =
    "8d3e194f933e064a6d202f48db18c2905f6bb78bfddd8d08f40efc7aa3ae093d";

fn potion_row(id: &str, slot_index: i64) -> Value {
    serde_json::json!({"id": id, "slot_index": slot_index})
}

#[test]
fn the_full_profile_control_opens_and_matches_the_oracle() {
    let opening = open(&full_profile_case()).expect("the full-profile control opens");
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!([null, null]))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        FULL_PROFILE_DIGEST
    );
}

/// #3347, end to end through Rust's own opening: a save whose profile hides
/// one of the three epochs the solo-Ironclad flag needs (`IRONCLAD4`,
/// `POTION1`, `POTION2`) writes no `fully_unlocked_potion_pool`, but it does
/// record `splash_unlock_epochs`, and that recorded profile now proves the
/// materialised `[null, null]` belt. Before #3347 each of these refused at the
/// boundary on `fully_unlocked_potion_pool`. Mutation on
/// [`full_profile_case`]: the named epoch removed from `unlocked_epochs`.
#[test]
fn issue3347_a_profile_hiding_a_flag_epoch_still_opens_with_its_belt() {
    for hidden in ["IRONCLAD4_EPOCH", "POTION1_EPOCH", "POTION2_EPOCH"] {
        let mut case = full_profile_case();
        case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
            crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .filter(|epoch| **epoch != hidden)
                .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
                .collect(),
        );
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{hidden}: {refusal}"));
        let player = &opening.document.player;
        assert_eq!(
            player.get("potion_slots"),
            Some(&serde_json::json!([null, null])),
            "{hidden}"
        );
        assert_eq!(player.get("fully_unlocked_potion_pool"), None, "{hidden}");
        let recorded = player
            .get("splash_unlock_epochs")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{hidden}: the profile is recorded"));
        assert!(!recorded.contains(&Value::from(hidden)), "{hidden}");
    }
}

/// `RELIC.PETRIFIED_TOAD` procures one Shaped Rock into the first empty slot
/// in `BeforeCombatStartLate`, and a full belt makes it a no-op (#2827).
///
/// `PetrifiedToad/<BeforeCombatStartLate>d__4::MoveNext` (`0x32dd04`) is an
/// unconditional `TryToProcure<PotionShapedRock>(Owner)` at `IL_0029`. The body
/// is `engine::petrified_toad_before_combat_start_late`. Mutations on
/// [`full_profile_case`]: `RELIC.PETRIFIED_TOAD` appended, plus the stated
/// `save['players'][0]['potions']` rows.
#[test]
fn a_petrified_toad_save_procures_a_shaped_rock_and_matches_the_oracle() {
    for (name, potions, slots, digest) in [
        (
            "empty belt: the rock takes slot 0",
            vec![],
            serde_json::json!(["POTION_SHAPED_ROCK", null]),
            "207c7b132e85fab78a97852f5e85bc5b70c10fc0bd2c5cea1e2051ed99e01143",
        ),
        (
            "slot 0 held: the rock takes slot 1",
            vec![potion_row("POTION.FIRE_POTION", 0)],
            serde_json::json!(["FIRE_POTION", "POTION_SHAPED_ROCK"]),
            "dbac245b4f631b64c6f3356f60d2a94b413313034985467b1e0eb83977d8f130",
        ),
        (
            "slot 1 held: the rock takes the FIRST empty slot, 0",
            vec![potion_row("POTION.FIRE_POTION", 1)],
            serde_json::json!(["POTION_SHAPED_ROCK", "FIRE_POTION"]),
            "bc744484529d7775fbcd5f32bdb6e4dcbf1504c03025119a8d8029d103d32ec4",
        ),
        (
            "full belt: no procurement",
            vec![
                potion_row("POTION.FIRE_POTION", 0),
                potion_row("POTION.BLOCK_POTION", 1),
            ],
            serde_json::json!(["FIRE_POTION", "BLOCK_POTION"]),
            "8824d40be653a7f51701d4d7d53d0c1245c82159e7ed15711ae540e33da65772",
        ),
    ] {
        let mut case = full_profile_case();
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.PETRIFIED_TOAD"}),
        );
        case["save"]["players"][0]["potions"] = Value::Array(potions);
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(
            opening.document.player.get("potion_slots"),
            Some(&slots),
            "{name}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{name}"
        );
        assert_ne!(
            python_view(&opening.document).differential_digest(),
            FULL_PROFILE_DIGEST,
            "{name}"
        );
    }
}

fn sozu_row() -> Value {
    serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.SOZU"})
}

/// [`full_profile_case`] owning Sozu: the control for the Sozu witnesses.
fn sozu_case() -> Value {
    let mut case = full_profile_case();
    relic(&mut case, sozu_row());
    case
}

/// The player fields of `opening` that differ from `control`'s, by name.
fn player_fields_differing(opening: &Opening, control: &Opening) -> Vec<String> {
    let (a, b) = (&opening.document.player, &control.document.player);
    let names: std::collections::BTreeSet<&String> = a
        .keys()
        .chain(b.keys())
        .filter(|key| a.get(*key) != b.get(*key))
        .collect();
    names.into_iter().cloned().collect()
}

/// A Sozu save opens, carrying the veto and the extra energy (#2890).
///
/// `Sozu` (v0.111.0) is `ShouldProcurePotion` (RVA `0x9bbf3`, false for its
/// owner) and `ModifyMaxEnergy` (`0x9bc01`, `+ EnergyVar(1)`), with no window
/// hook and no per-fight state; `pre_hook_document` carries the IL. So against
/// the same save without it, the opened document differs in the relic lists,
/// the `sozu` carrier and turn one's energy, and in nothing else: the belt,
/// every pile, every monster and every stream are the control's. Before the
/// seed this save refused as `boundary_unrepresentable`.
#[test]
fn a_sozu_save_opens_with_the_veto_carrier_and_one_more_energy() {
    let control = open(&full_profile_case()).expect("the control opens");
    let opening = open(&sozu_case()).expect("a Sozu save opens");

    assert_eq!(control.document.player.get("sozu"), None);
    assert_eq!(
        opening.document.player.get("sozu"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!([null, null]))
    );
    // The control's 3 is the field default and so elided.
    assert_eq!(control.document.player.get("energy"), None);
    assert_eq!(opening.document.player.get("energy"), Some(&Value::from(4)));
    assert_eq!(
        player_fields_differing(&opening, &control),
        ["energy", "relics_entering", "sozu", "template_relics"]
    );
    assert_eq!(opening.document.piles, control.document.piles);
    assert_eq!(opening.document.monsters, control.document.monsters);
    assert_eq!(opening.document.rng, control.document.rng);

    // The empty-topology belt (capacity 0) takes the carrier too: the veto is
    // a fact about the owner, not about a slot, and the boundary accepts it
    // with the relic.
    let opening = open(&with_relic("RELIC.SOZU")).expect("a Sozu save without a belt opens");
    assert_eq!(
        opening.document.player.get("sozu"),
        Some(&Value::Bool(true))
    );
    assert_eq!(opening.document.player.get("potion_slots"), None);
}

/// Sozu vetoes Petrified Toad's rock: the belt stays as it was (#2890).
///
/// `PetrifiedToad/<BeforeCombatStartLate>d__4::MoveNext` (`0x32dd04`) calls
/// `PotionCmd::TryToProcure<PotionShapedRock>` unconditionally, and
/// `<TryToProcure>d__1::MoveNext` (`0x3ef588`) returns a failed result at
/// `IL_0052`-`IL_0072` when `Hook::ShouldProcurePotion` (`IL_004b`) is false,
/// before `AddPotionInternal` (`IL_008b`). So Toad + Sozu is the Sozu save
/// with one more relic row: no rock in an empty belt, a held potion kept and
/// its neighbour left empty, and no stream moved. Without Sozu the same saves
/// take the rock
/// (`a_petrified_toad_save_procures_a_shaped_rock_and_matches_the_oracle`).
///
/// The digest is the deleted Python oracle's for the empty-belt save, as
/// #2890 recorded it before the deletion. It cannot be regenerated, so it is
/// corroboration of the IL read above and not the authority for it.
#[test]
fn a_petrified_toad_save_with_sozu_opens_with_the_rock_vetoed() {
    let toad = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.PETRIFIED_TOAD"});
    let sozu_only = open(&sozu_case()).expect("the Sozu control opens");

    for (name, potions, slots) in [
        ("empty belt", vec![], serde_json::json!([null, null])),
        (
            "slot 1 held",
            vec![potion_row("POTION.FIRE_POTION", 1)],
            serde_json::json!([null, "FIRE_POTION"]),
        ),
    ] {
        let mut case = full_profile_case();
        relic(&mut case, toad.clone());
        relic(&mut case, sozu_row());
        let empty = potions.is_empty();
        case["save"]["players"][0]["potions"] = Value::Array(potions);
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(
            opening.document.player.get("potion_slots"),
            Some(&slots),
            "{name}"
        );
        assert_eq!(
            opening.document.player.get("sozu"),
            Some(&Value::Bool(true)),
            "{name}"
        );
        assert_eq!(opening.document.rng, sozu_only.document.rng, "{name}");
        assert_eq!(opening.document.piles, sozu_only.document.piles, "{name}");
        if empty {
            assert_eq!(
                python_view(&opening.document).differential_digest(),
                "1d654bed3c3ec4e2745d216d67da23e6dc7dfb9a2cd7514caa4da6f3ef34d208",
                "{name}"
            );
        }
    }

    // Acquisition order does not matter: the veto is a `Should` hook over
    // every listener, not a same-hook race with the Toad.
    let mut case = sozu_case();
    relic(&mut case, toad);
    let opening = open(&case).expect("Sozu before the Toad opens");
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!([null, null]))
    );
}

/// Sozu vetoes Delicate Frond's fill after one generated potion (#2890).
///
/// `DelicateFrond/<BeforeCombatStart>d__2::MoveNext` (`0x3229bc`) enters its
/// loop on an open slot, generates one potion (`IL_0044`, two
/// `CombatPotionGeneration` draws) and leaves on the failed procurement
/// (`IL_00b7`-`IL_00bc`). So the belt stays empty and the stream moves two
/// draws, not two per open slot; a full belt never enters the loop.
#[test]
fn a_delicate_frond_save_with_sozu_spends_one_generation_and_fills_nothing() {
    let control = open(&sozu_case()).expect("the Sozu control opens");
    let base = potion_generation_counter(&control);

    let mut case = frond_case(vec![]);
    relic(&mut case, sozu_row());
    let opening = open(&case).expect("Frond + Sozu opens");
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!([null, null]))
    );
    assert_eq!(potion_generation_counter(&opening), base + 2);

    let mut full = frond_case(vec![
        potion_row("POTION.FIRE_POTION", 0),
        potion_row("POTION.BLOCK_POTION", 1),
    ]);
    relic(&mut full, sozu_row());
    let opening = open(&full).expect("a full belt opens");
    assert_eq!(potion_generation_counter(&opening), base);
}

/// What stays refused under Sozu, each by its own name (#2890).
///
/// The seed admits the carrier; it widens no other gate. A procuring relic
/// still needs the exact belt capacity (its guard reads the slots before Sozu
/// is asked), and Belt Buckle, whose latch a procurement clears, is still
/// refused by the room-entry gate exactly as it is without Sozu.
#[test]
fn a_sozu_save_keeps_the_procuring_relics_own_refusals() {
    for procurer in ["RELIC.PETRIFIED_TOAD", "RELIC.DELICATE_FROND"] {
        let refusal = open(&with_relics(procurer, "RELIC.SOZU")).expect_err("capacity 0 refuses");
        assert_eq!(refusal.class(), "potion_belt_not_exact", "{procurer}");
        assert!(refusal.to_string().contains(procurer), "{refusal}");
    }
    let buckle = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BELT_BUCKLE"});
    let mut without = full_profile_case();
    relic(&mut without, buckle.clone());
    let without = open(&without).expect_err("Belt Buckle refuses without Sozu");
    let mut with = sozu_case();
    relic(&mut with, buckle);
    let with = open(&with).expect_err("and with it");
    assert_eq!(with.class(), without.class());
    assert_eq!(with.to_string(), without.to_string());
    assert_ne!(with.class(), "boundary_unrepresentable");
}

/// A Petrified Toad fight without an exact positive belt capacity refuses,
/// as the oracle does (`_potion_slots_from_entry` with `require_capacity`:
/// *"requires exact positive max_potion_slot_count and save-backed potion slot
/// rows"*). `empty_belt_case` is exactly that save: capacity 0.
#[test]
fn a_petrified_toad_save_without_an_exact_belt_capacity_refuses() {
    let refusal = open(&with_relic("RELIC.PETRIFIED_TOAD")).expect_err("capacity 0 refuses");
    assert_eq!(refusal.class(), "potion_belt_not_exact");
    assert!(
        refusal.to_string().contains("RELIC.PETRIFIED_TOAD"),
        "{refusal}"
    );

    // A slot row past the recorded capacity is the other arm.
    let mut case = full_profile_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.PETRIFIED_TOAD"}),
    );
    case["save"]["players"][0]["potions"] = Value::Array(vec![potion_row("POTION.FIRE_POTION", 2)]);
    let refusal = open(&case).expect_err("an out-of-range row refuses");
    // Since #2791 the malformed row refuses by its own name, before the
    // Toad's capacity gate is consulted.
    assert_eq!(refusal.class(), "potion_belt_row_malformed");
}

/// The opened document's `CombatPotionGeneration` counter.
fn potion_generation_counter(opening: &Opening) -> u64 {
    opening.document.rng["potion_generation"].counter
}

fn frond_case(potions: Vec<Value>) -> Value {
    let mut case = full_profile_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.DELICATE_FROND"}),
    );
    case["save"]["players"][0]["potions"] = Value::Array(potions);
    case
}

/// `RELIC.DELICATE_FROND` fills every open slot at `BeforeCombatStart`
/// (#3533).
///
/// `DelicateFrond/<BeforeCombatStart>d__2::MoveNext` (`0x3229bc`) is
/// `while (Owner.HasOpenPotionSlots)` around one out-of-combat factory potion
/// and a first-empty-slot `TryToProcure`. The body is
/// `engine::potions::delicate_frond_before_combat_start`, whose own tests
/// carry the native witness (a recorded run's ten fights). There is no oracle
/// digest for these saves, since the Python simulator was deleted before the
/// port, so this pins what the opening does with the body: which slots fill,
/// that held potions stay, and that the stream moves two draws per open slot
/// and nothing else moves with it.
#[test]
fn a_delicate_frond_save_fills_every_open_slot_from_the_potion_stream() {
    let control = open(&full_profile_case()).expect("the control opens");
    let base = potion_generation_counter(&control);

    let empty = open(&frond_case(vec![])).expect("an empty belt opens");
    let filled = empty.document.player["potion_slots"]
        .as_array()
        .expect("the belt materialises")
        .clone();
    assert_eq!(filled.len(), 2);
    assert!(filled.iter().all(Value::is_string), "{filled:?}");
    assert_eq!(potion_generation_counter(&empty), base + 4);

    for (name, held_slot, open_slot) in [("slot 0 held", 0, 1), ("slot 1 held", 1, 0)] {
        let opening = open(&frond_case(vec![potion_row(
            "POTION.FIRE_POTION",
            held_slot,
        )]))
        .unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        let slots = &opening.document.player["potion_slots"];
        assert_eq!(slots[held_slot as usize], "FIRE_POTION", "{name}");
        // One open slot takes the stream's first potion, which is the empty
        // belt's slot 0.
        assert_eq!(slots[open_slot], filled[0], "{name}");
        assert_eq!(potion_generation_counter(&opening), base + 2, "{name}");
    }

    // Everything but the belt and its stream is the control's.
    let mut rng = empty.document.rng.clone();
    rng.insert(
        "potion_generation".to_string(),
        control.document.rng["potion_generation"].clone(),
    );
    assert_eq!(rng, control.document.rng);
    assert_eq!(empty.document.piles, control.document.piles);
    assert_eq!(empty.document.monsters, control.document.monsters);
}

/// A full belt never enters the loop (`IL_0023` branches to the guard at
/// `IL_00be`), so the fight is the same save without the relic's effect: no
/// draw, no procurement.
#[test]
fn a_delicate_frond_save_with_a_full_belt_draws_nothing() {
    let potions = vec![
        potion_row("POTION.FIRE_POTION", 0),
        potion_row("POTION.BLOCK_POTION", 1),
    ];
    let mut control = full_profile_case();
    control["save"]["players"][0]["potions"] = Value::Array(potions.clone());
    let control = open(&control).expect("the full-belt control opens");
    let opening = open(&frond_case(potions)).expect("a full belt opens");
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!(["FIRE_POTION", "BLOCK_POTION"]))
    );
    assert_eq!(opening.document.rng, control.document.rng);
}

/// Frond runs in the ordinary `BeforeCombatStart` walk and Petrified Toad in
/// the `…Late` one (`Hook/<BeforeCombatStart>d__18::MoveNext` `0x3d2574`), so
/// the Frond fills the belt first and the Toad's Shaped Rock finds no slot,
/// whichever relic was acquired first.
#[test]
fn a_delicate_frond_fills_the_belt_before_petrified_toad_procures() {
    let frond_only = open(&frond_case(vec![])).expect("Frond alone opens");
    for toad_first in [false, true] {
        let mut case = frond_case(vec![]);
        let toad = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.PETRIFIED_TOAD"});
        let relics = case["save"]["players"][0]["relics"]
            .as_array_mut()
            .expect("the save's relics are an array");
        if toad_first {
            relics.insert(0, toad);
        } else {
            relics.push(toad);
        }
        let opening = open(&case).expect("Frond + Toad opens");
        assert_eq!(
            opening.document.player.get("potion_slots"),
            frond_only.document.player.get("potion_slots"),
            "toad_first={toad_first}"
        );
        assert_eq!(opening.document.rng, frond_only.document.rng);
    }
}

/// Like the Toad, a Frond fight without an exact positive belt capacity
/// refuses: "every open slot" is a count only the recorded capacity gives.
#[test]
fn a_delicate_frond_save_without_an_exact_belt_capacity_refuses() {
    let refusal = open(&with_relic("RELIC.DELICATE_FROND")).expect_err("capacity 0 refuses");
    assert_eq!(refusal.class(), "potion_belt_not_exact");
    assert!(
        refusal.to_string().contains("RELIC.DELICATE_FROND"),
        "{refusal}"
    );
}

/// A Frond save whose owner's pool is not proven never reaches the body: the
/// boundary already refuses a materialised belt without a pool proof
/// (`potions::potion_belt_state_is_exact`), Frond or not. [`case`] records no
/// unlock profile. The body's own provenance refusal is witnessed in
/// `engine::potions::delicate_frond_tests`.
#[test]
fn a_delicate_frond_save_without_a_pool_proof_refuses_like_the_same_save_without_it() {
    let control = open(&case()).expect_err("an unproven belt refuses at the boundary");
    let mut with_frond = case();
    relic(
        &mut with_frond,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.DELICATE_FROND"}),
    );
    let refusal = open(&with_frond).expect_err("so does the same save with the Frond");
    assert_eq!(refusal.class(), "boundary_unrepresentable");
    assert_eq!(refusal.to_string(), control.to_string());
}

// ---------------------------------------------------------------------------
// Every potion-belt row is placed exactly or refused by name (#2791)
// ---------------------------------------------------------------------------
//
// Until #2791 three `?` arms in `potion_slots` — a row with no `slot_index`, a
// negative one, one at or past the recorded capacity — returned `None`, which
// *elided* the belt: a well-formed document silently missing the saved
// potions. Each arm is witnessed twice below: directly on `potion_slots`, and
// end to end through a synthetic save, where the refusal must surface from
// `open` rather than a document without `potion_slots`.

/// The fixture save with `rows` as its potion belt and `capacity` recorded.
fn recorded_belt_case(capacity: i64, rows: Vec<Value>) -> Value {
    let mut case = full_profile_case();
    case["save"]["players"][0]["max_potion_slot_count"] = Value::from(capacity);
    case["save"]["players"][0]["potions"] = Value::Array(rows);
    case
}

fn belt_refusal(slots: Result<Option<Vec<Value>>, OpeningRefusal>) -> OpeningRefusal {
    match slots {
        Err(refusal) => refusal,
        Ok(slots) => panic!("expected a named refusal, got {slots:?}"),
    }
}

#[test]
fn a_well_formed_belt_materialises_every_row_at_its_slot() {
    // Out of save order, with an empty slot between: the rows land at their
    // recorded indices and the recorded capacity fixes the trailing empties.
    let entry = entry_facts(&recorded_belt_case(
        4,
        vec![
            potion_row("POTION.BLOCK_POTION", 2),
            potion_row("POTION.FIRE_POTION", 0),
        ],
    ));
    assert_eq!(
        super::potion_slots(&entry).expect("a well-formed belt places"),
        Some(vec![
            Value::from("FIRE_POTION"),
            Value::Null,
            Value::from("BLOCK_POTION"),
            Value::Null,
        ])
    );

    // The last in-range slot, capacity - 1, is admitted.
    let entry = entry_facts(&recorded_belt_case(
        2,
        vec![potion_row("POTION.FIRE_POTION", 1)],
    ));
    assert_eq!(
        super::potion_slots(&entry).unwrap(),
        Some(vec![Value::Null, Value::from("FIRE_POTION")])
    );
    let opening = open(&recorded_belt_case(
        2,
        vec![potion_row("POTION.FIRE_POTION", 1)],
    ))
    .expect("an in-range belt opens");
    assert_eq!(
        opening.document.player.get("potion_slots"),
        Some(&serde_json::json!([null, "FIRE_POTION"]))
    );

    // No rows and no positive capacity is the elided default, natively a belt
    // of no slots; it is not a refusal.
    for capacity in [0, -1] {
        let entry = entry_facts(&recorded_belt_case(capacity, vec![]));
        assert_eq!(super::potion_slots(&entry).unwrap(), None, "{capacity}");
    }
}

#[test]
fn a_belt_row_without_a_slot_index_refuses_by_name() {
    let rows = vec![serde_json::json!({"id": "POTION.FIRE_POTION"})];
    let entry = entry_facts(&recorded_belt_case(3, rows.clone()));
    assert_eq!(
        belt_refusal(super::potion_slots(&entry)),
        OpeningRefusal::PotionBeltRowMalformed {
            potion: "POTION.FIRE_POTION".to_string(),
            slot_index: None,
            capacity: Some(3),
        }
    );
    let refusal = open(&recorded_belt_case(3, rows)).expect_err("a missing slot_index refuses");
    assert_eq!(refusal.class(), "potion_belt_row_malformed");
}

#[test]
fn a_negative_belt_slot_index_refuses_by_name() {
    // Natively `AddPotionInternal` would redirect it to the first empty slot;
    // the game never writes one, so it refuses rather than guessing.
    let rows = vec![
        potion_row("POTION.BLOCK_POTION", 0),
        potion_row("POTION.FIRE_POTION", -1),
    ];
    let entry = entry_facts(&recorded_belt_case(3, rows.clone()));
    assert_eq!(
        belt_refusal(super::potion_slots(&entry)),
        OpeningRefusal::PotionBeltRowMalformed {
            potion: "POTION.FIRE_POTION".to_string(),
            slot_index: Some(-1),
            capacity: Some(3),
        }
    );
    let refusal = open(&recorded_belt_case(3, rows)).expect_err("a negative slot_index refuses");
    assert_eq!(refusal.class(), "potion_belt_row_malformed");
}

#[test]
fn a_belt_row_at_or_past_the_recorded_capacity_refuses_by_name() {
    // At the capacity, past it, and any row under a zero capacity (natively a
    // belt of no slots, where every index is out of range).
    for (capacity, index) in [(2, 2), (2, 7), (0, 0)] {
        let rows = vec![potion_row("POTION.FIRE_POTION", index)];
        let entry = entry_facts(&recorded_belt_case(capacity, rows.clone()));
        assert_eq!(
            belt_refusal(super::potion_slots(&entry)),
            OpeningRefusal::PotionBeltRowMalformed {
                potion: "POTION.FIRE_POTION".to_string(),
                slot_index: Some(index),
                capacity: Some(capacity),
            },
            "capacity {capacity}, index {index}"
        );
        let refusal = open(&recorded_belt_case(capacity, rows))
            .expect_err("an out-of-range slot_index refuses end to end");
        assert_eq!(refusal.class(), "potion_belt_row_malformed");
    }
}

#[test]
fn without_a_recorded_capacity_the_belt_runs_to_the_highest_row_and_malformed_rows_refuse() {
    // No save omits `max_potion_slot_count`, but an entry-facts document may,
    // so the capacity-free branch is witnessed on the document directly.
    let mut entry = entry_facts(&recorded_belt_case(
        3,
        vec![potion_row("POTION.FIRE_POTION", 1)],
    ));
    entry.belt.max_potion_slot_count = None;
    assert_eq!(
        super::potion_slots(&entry).unwrap(),
        Some(vec![Value::Null, Value::from("FIRE_POTION")])
    );
    // With no rows either, nothing materialises.
    let mut empty = entry.clone();
    empty.belt.slots.clear();
    assert_eq!(super::potion_slots(&empty).unwrap(), None);
    // The missing and negative arms refuse on this branch too; before #2791
    // an all-unindexed belt fell out of the `max()?` and was elided.
    for slot_index in [None, Some(-2)] {
        let mut malformed = entry.clone();
        malformed.belt.slots[0].slot_index = slot_index;
        assert_eq!(
            belt_refusal(super::potion_slots(&malformed)),
            OpeningRefusal::PotionBeltRowMalformed {
                potion: "POTION.FIRE_POTION".to_string(),
                slot_index,
                capacity: None,
            }
        );
    }
}

// ---------------------------------------------------------------------------
// The gate tables are DERIVED, so the derivation is pinned (#2731)
// ---------------------------------------------------------------------------
//
// Until #2731 the derivation of `OPENING_WINDOW_RELIC_BODIES` lived only in a
// doc comment, and nothing went red when the oracle named a relic with an
// opening-window body that this crate has no subscriber for. That is exactly
// how Red Mask, Bag of Marbles, Twisted Funnel and Lantern came to emit a
// document silently missing their effect for the whole life of the slice.
//
// `fixtures/opening_relic_hooks_v1.json` is the oracle's own relic-hook
// manifest, written by `tools/gen_opening_relic_pins.py` from frozen
// `combat_sim`. Since #2827 item D it is frozen data pinned by
// `tools/frozen_oracle_data.py`, and #2999 deleted the generator. The two
// tests below re-derive the gate from it in both directions.

const ORACLE_RELIC_HOOKS: &str = include_str!("../../../fixtures/opening_relic_hooks_v1.json");

fn manifest() -> Value {
    serde_json::from_str(ORACLE_RELIC_HOOKS).expect("the relic-hook manifest is JSON")
}

/// One relic row of the manifest, in the three terms the derivation uses.
struct ManifestRow {
    relic: String,
    /// Declares a body on one of the five combat-start / first-deal hooks.
    window: bool,
    /// Declares a body on one of the three turn-start hooks.
    turn_one: bool,
    /// In `combat_sim.TEMPLATE_RELICS`, so its rules compile in Rust.
    template: bool,
    /// Named from somewhere in Rust that can run a body — derived by the
    /// generator (deleted #2999, frozen), never by hand.
    rust_body: bool,
}

fn manifest_rows() -> Vec<ManifestRow> {
    let manifest = manifest();
    let rows = manifest["relics"]
        .as_array()
        .expect("the manifest carries relic rows")
        .clone();
    rows.iter()
        .map(|row| {
            let hooks = |key: &str| {
                !row[key]
                    .as_array()
                    .expect("hook lists are arrays")
                    .is_empty()
            };
            let flag = |key: &str| row[key].as_bool().expect("a boolean flag");
            ManifestRow {
                relic: row["relic"].as_str().expect("a relic name").to_string(),
                window: hooks("window_hooks"),
                turn_one: hooks("turn_one_hooks"),
                template: flag("template"),
                rust_body: flag("rust_body_site"),
            }
        })
        .collect()
}

/// The manifest is this build's, and its hook groups are the eight the two
/// tables were derived over.
///
/// Without this the tests below could pass against a manifest derived over a
/// narrower hook set — which is the original defect restated, not a fix.
#[test]
fn the_relic_hook_manifest_is_this_builds_and_covers_the_eight_hooks() {
    let manifest = manifest();
    assert_eq!(
        manifest["schema"],
        Value::from("sts-sim-opening-relic-hooks-v1")
    );
    assert_eq!(manifest["game_build"], Value::from("v0.111.0"));
    assert_eq!(
        manifest["opening_window_hooks"],
        serde_json::json!([
            "AfterRoomEntered",
            "BeforeCombatStart",
            "BeforeCombatStartLate",
            "BeforeHandDraw",
            "ModifyHandDraw"
        ])
    );
    assert_eq!(
        manifest["turn_one_hooks"],
        serde_json::json!([
            "BeforeSideTurnStart",
            "AfterSideTurnStart",
            "AfterPlayerTurnStart"
        ])
    );
}

/// A non-template relic with a body on one of the eight hooks is gated, or has
/// a Rust body, or is an explicitly justified exclusion — and every gate entry
/// is one the oracle still names.
///
/// This is the assertion the slice was missing. It fails if a relic acquires a
/// turn-1 opening-window body with no Rust subscriber and no gate entry, and it
/// fails the other way too: an entry left in a table after its relic gained a
/// body, or stopped declaring one, is a stale refusal that over-refuses real
/// fights.
#[test]
fn every_opening_window_relic_body_is_gated_or_explicitly_excluded() {
    let rows = manifest_rows();
    let mut window = 0;
    let mut turn_one = 0;
    let mut turn_one_unsubscribed = 0;
    for row in &rows {
        if row.template {
            continue;
        }
        let relic = row.relic.as_str();
        if row.window {
            window += 1;
            // The five-hook half is unchanged by #2731 and rests on the
            // template argument alone: the opening's gate positions are the
            // oracle's, and the five state-seeding exclusions are written by
            // the opening itself.
            assert!(
                super::OPENING_WINDOW_RELIC_BODIES.contains(&relic)
                    || super::OPENING_WINDOW_GATE_EXCLUSIONS.contains(&relic),
                "{relic} declares a combat-start/first-deal body and is \
                 neither gated nor an explicit exclusion"
            );
        }
        if !row.turn_one {
            continue;
        }
        turn_one += 1;
        if row.rust_body {
            // A Rust body exists, so the hook reaches real code. Whether that
            // body is *correct* is the engine's business and its own tests';
            // this gate is only about bodies that are not there at all.
            continue;
        }
        turn_one_unsubscribed += 1;
        // The turn-1 half accepts the five-hook table too: `POCKETWATCH`
        // declares bodies in both groups and already refuses there, and
        // `BOOMING_CONCH` is a window exclusion with its own gate.
        // (`PAELS_LEGION`, `LETTER_OPENER`, `PAELS_FLESH` and
        // `PHYLACTERY_UNBOUND` were also here until #2827 retired their window
        // gates; their turn-1 bodies are in `engine::relics`, so they never
        // reach here.)
        assert!(
            super::TURN_ONE_RELIC_BODIES.contains(&relic)
                || super::TURN_ONE_ORB_RELICS.contains(&relic)
                || super::TURN_ONE_GATE_EXCLUSIONS.contains(&relic)
                || super::OPENING_WINDOW_RELIC_BODIES.contains(&relic)
                || super::OPENING_WINDOW_GATE_EXCLUSIONS.contains(&relic),
            "{relic} declares a turn-1 body the opening runs, this crate has \
             no subscriber for it, and it is neither gated nor an explicit \
             exclusion — so the opening would emit a document missing the \
             effect with nothing to refuse it (#2731)"
        );
    }
    // The counts the doc comments quote, so prose and code cannot drift. The
    // third one is what keeps the loop above non-vacuous: a manifest whose
    // `rust_body_site` was true everywhere would skip every assertion.
    assert_eq!(window, 46, "non-template relics with a five-hook body");
    assert_eq!(turn_one, 53, "non-template relics with a turn-1 body");
    // 11 until 2026-09-22; `RED_MASK`, `BAG_OF_MARBLES` and `LANTERN` gained
    // engine bodies in this slice, so the generator now finds all three from a
    // body root (`engine/relics.rs`). 7 since `CRACKED_CORE` gained its
    // turn-1 channel there the same day (#2827). 6 since `SYMBIOTIC_VIRUS`
    // gained its turn-1 Dark channel there too (#2827,
    // `engine::relics::symbiotic_virus_after_side_turn_start`). 4 since
    // `BELLOWS` and `FESTIVE_POPPER` gained their `AfterPlayerTurnStart`
    // bodies in `engine::relics::continue_after_toasty_mittens` (#2827,
    // 2026-09-23). 3 since `FENCING_MANUAL` gained its turn-1 Forge in
    // `engine::relics::fencing_manual_after_side_turn_start` (#3090,
    // 2026-09-25; the manifest's `rust_body_site` flipped by hand, since its
    // generator is deleted).
    assert_eq!(
        turn_one_unsubscribed, 3,
        "of those, the ones with no Rust body at all"
    );

    // The reverse direction: no table entry is stale, and no relic is in two
    // tables with two different reasons.
    let named: std::collections::BTreeMap<&str, &ManifestRow> =
        rows.iter().map(|row| (row.relic.as_str(), row)).collect();
    let row_for = |relic: &str| -> &ManifestRow {
        named
            .get(relic)
            .copied()
            .unwrap_or_else(|| panic!("{relic} is not in the oracle's manifest at all"))
    };
    for relic in super::OPENING_WINDOW_RELIC_BODIES
        .into_iter()
        .chain(super::OPENING_WINDOW_GATE_EXCLUSIONS)
    {
        let row = row_for(relic);
        assert!(row.window, "{relic} no longer declares a five-hook body");
        assert!(
            !row.template,
            "{relic} is a template relic with a subscriber"
        );
    }
    for relic in super::TURN_ONE_RELIC_BODIES
        .into_iter()
        .chain(super::TURN_ONE_GATE_EXCLUSIONS)
    {
        let row = row_for(relic);
        assert!(row.turn_one, "{relic} no longer declares a turn-1 body");
        assert!(
            !row.template,
            "{relic} is a template relic with a subscriber"
        );
        assert!(
            !row.rust_body,
            "{relic} now HAS a Rust body, so gating it over-refuses — retire \
             the entry with the modeling slice that added the body"
        );
        assert!(
            !super::OPENING_WINDOW_RELIC_BODIES.contains(&relic)
                && !super::TURN_ONE_ORB_RELICS.contains(&relic),
            "{relic} is gated twice, so its census class depends on table order"
        );
    }
    for relic in super::TURN_ONE_RELIC_BODIES {
        assert!(
            !super::TURN_ONE_GATE_EXCLUSIONS.contains(&relic),
            "{relic} cannot be both gated and excluded"
        );
    }

    // The five-hook half cannot borrow the turn-1 half's `!rust_body`
    // assertion: 18 of its gate entries name a `RelicId` from a body root for
    // some OTHER hook (Pocketwatch's turn-start play counting, …), so "has a
    // Rust body" does not mean "has one for its window hook"; Letter Opener
    // was the other example here until #2827. That is exactly how
    // `RELIC.PENDULUM` sat in the gate while `engine::relics` ran both halves
    // of its body for its whole life (#2693). What replaces the assertion is a counter: a body added to a
    // gated relic moves this number and the author has to say which it is.
    let gated_with_a_body: Vec<&str> = super::OPENING_WINDOW_RELIC_BODIES
        .into_iter()
        .filter(|relic| row_for(relic).rust_body)
        .collect();
    // 24 until #2827 retired `RELIC.BOUND_PHYLACTERY`: its body site was the
    // turn>1 `AfterEnergyResetLate` re-summon, and its WINDOW body (the
    // `BeforeCombatStart` summon) is now `engine::phylactery_before_combat_start`,
    // with the parity witness below. 22 since #2827 retired
    // `RELIC.PAELS_LEGION`: its body sites were the `AfterSideTurnStart`
    // cooldown tick and the `AfterCardPlayed` trigger, and its WINDOW body
    // (the `BeforeCombatStart` pet) is now
    // `engine::passive_relic_pets_before_combat_start`, with the parity
    // witnesses below. `RELIC.BYRDPIP` left the gate in the same change but
    // never counted here: it had no Rust body site until that function named it.
    // 21 since #2827 retired `RELIC.RING_OF_THE_SNAKE`, the one entry whose
    // WINDOW body (`ModifyHandDraw`, folded in `engine::relics::modifier_total`)
    // this crate already ran: it was a measured over-refusal, and it now has
    // the parity witnesses beside Bag of Preparation's below.
    // 20 since #2827 retired `RELIC.PETRIFIED_TOAD`: its only body site was
    // `engine::fire_before_combat_start`'s by-name refusal, and that site is
    // now its WINDOW body (`engine::petrified_toad_before_combat_start_late`),
    // with the parity witnesses below. `RELIC.GIRYA` left in the same change
    // and never counted here: its body is in `entry::opening`, whose
    // `mod fixture_tests;` marker ends the generator's production text.
    // 18 since #2827 retired `RELIC.LETTER_OPENER` and `RELIC.PAELS_FLESH`:
    // their body sites were the `AfterCardPlayed` skill counter and the
    // turn>=3 `ModifyMaxEnergy` row, and their WINDOW bodies
    // (`BeforeCombatStart`) are inert in IL — a counter reset a fresh state
    // already holds, and a display call — with parity witnesses below.
    // 17 since #2827 retired `RELIC.GHOST_SEED`: its body site was the
    // generated-card `AfterCardEnteredCombat` half in `engine::cards`, and its
    // WINDOW body (the `AfterRoomEntered` Ethereal pass) is now
    // `engine::cards::ghost_seed_after_room_entered`, with the parity
    // witnesses below. `RELIC.ETERNAL_FEATHER` left in the same change and
    // never counted here: its heal is rest-site only, so it has no body site.
    // 15 after #2827 retired `RELIC.POLLINOUS_CORE` and `RELIC.VAMBRACE`.
    // Pollinous Core's body site WAS its window body (`BeforeHandDraw` /
    // `ModifyHandDraw` in `engine::relics::before_hand_draw`); the gate was
    // an over-refusal that also hid the opening's wrong seed modulus, fixed
    // with it. Vambrace's sites were its in-combat block doubling; its WINDOW
    // body (`BeforeCombatStart`) is now `engine::vambrace_before_combat_start`.
    // Parity witnesses for both are at the end of this file.
    // 13 since #2827 also retired `RELIC.KUSARIGAMA` and
    // `RELIC.PHYLACTERY_UNBOUND`:
    // their body sites were Kusarigama's `AfterCardPlayed` hit and
    // `AfterSideTurnEnd` reset, and Phylactery Unbound's turn-start summon in
    // `after_side_turn_start_late`. Their WINDOW bodies are Kusarigama's inert
    // counter reset (already seeded) and Phylactery Unbound's combat-start
    // summon, now `engine::phylactery_before_combat_start`, with parity
    // witnesses below.
    // 12 since #2827 item B retired `RELIC.TOOLBOX`, whose body site WAS its
    // window body (the turn-1 `BeforeHandDraw` choice in
    // `engine::relics::before_hand_draw`), with the parity witnesses at the
    // end of this file. `RELIC.SLING_OF_COURAGE` left in the same change and
    // never counted here: its body is in `entry::opening`.
    // 11 since #2827 item B also retired `RELIC.POCKETWATCH`: its body sites
    // were the turn>1 `ModifyHandDraw` row and the play counting, and its
    // WINDOW body is inert on turn one in IL, with parity witnesses at the end
    // of this file. `RELIC.MEAT_ON_THE_BONE` left in the same change and never
    // counted here: it has no Rust body site.
    // 10 since #2827 item B retired `RELIC.UNSETTLING_LAMP`: its body sites
    // were the in-fight card-debuff doubling, and its WINDOW body (the
    // `BeforeCombatStart` armed-state reset) is now a
    // `PER_FIGHT_RELIC_STATE_SEEDED` row, with parity witnesses at the end of
    // this file.
    // 9 since #2992 retired `RELIC.BLESSED_ANTLER`: its body site WAS its
    // window body (the turn-1 `BeforeHandDraw` Dazed row of
    // `engine::relics::continue_before_hand_draw_after_toolbox`), so the gate
    // was an over-refusal; parity witnesses are at the end of this file.
    // 5 since #3162 retired `RELIC.JEWELED_MASK`, `RELIC.FUNERARY_MASK`,
    // `RELIC.RADIANT_PEARL` and `RELIC.BIG_MUSHROOM`: each body site WAS its
    // window body (the three turn-1 `BeforeHandDraw` rows in
    // `engine::relics` and Big Mushroom's turn-1 `ModifyHandDraw` row; Big
    // Mushroom's `AfterRoomEntered` is a visual scale in IL). The census
    // opening checkpoint is their witness, with fixture witnesses at the end
    // of this file. `RELIC.TEA_OF_DISCOURTESY` left in the same change and
    // never counted here: it has no Rust body site, and a spent Tea's
    // `BeforeCombatStart` is inert in IL (`tea_of_discourtesy_spent`).
    // Still 5 after #3533 retired `RELIC.DELICATE_FROND`: it had no Rust
    // body site while gated, and its manifest row flipped by hand with the
    // port (`engine::potions::delicate_frond_before_combat_start`).
    // 4 since #2758 retired `RELIC.RING_OF_THE_DRAKE`: its body site WAS its
    // window body (the `turn <= 3` `ModifyHandDraw` row), with fixture
    // witnesses at the end of this file. The four left are Fake Snecko Eye,
    // Snecko Eye, Fur Coat and Philosopher's Stone, each named from a body
    // root for a hook other than the one that keeps it gated (the audit above
    // `OPENING_WINDOW_RELIC_BODIES` says which).
    assert_eq!(
        gated_with_a_body.len(),
        4,
        "a five-hook gate entry gained or lost a Rust body site — check whether \
         the body is for its WINDOW hook, and if it is, retire the entry with \
         a parity witness instead of adjusting this number: {gated_with_a_body:?}"
    );
}

/// The half of the derivation that is a claim about **Rust**: a template relic
/// has a compiled subscriber and a non-template one does not.
///
/// The whole gate rests on "this crate reaches a subscriber for those hooks
/// only through `content_tables::TEMPLATE_RELIC_STEPS`". If a gated relic
/// turned out to have a compiled program the gate would be over-refusing, and
/// if a template relic lost its program the hook would fire into nothing with
/// no gate to catch it.
#[test]
fn template_membership_and_compiled_subscribers_agree() {
    let relic_id = |name: &str| {
        crate::content_tables::RELIC_LEDGER
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} has no RelicId"))
            .id
    };
    for row in manifest_rows() {
        let compiled = crate::content_tables::template_relic(relic_id(&row.relic)).is_some();
        assert_eq!(
            compiled, row.template,
            "{}: oracle template={} but compiled subscriber={compiled}",
            row.relic, row.template
        );
    }
    for relic in super::OPENING_WINDOW_RELIC_BODIES
        .into_iter()
        .chain(super::TURN_ONE_RELIC_BODIES)
        .chain(super::TURN_ONE_ORB_RELICS)
    {
        assert!(
            crate::content_tables::template_relic(relic_id(relic)).is_none(),
            "{relic} is gated but HAS a compiled subscriber, so the gate \
             over-refuses"
        );
    }
}

// ---------------------------------------------------------------------------
// Witnesses for the turn-1 gate (#2731)
// ---------------------------------------------------------------------------
//
// Expected values printed by the oracle with the recipe in the comment block
// above `empty_belt_case`, adding `save['players'][0]['relics'].append(
// {'floor_added_to_deck': 1, 'id': <relic>})` per case.

fn with_relic(id: &str) -> Value {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": id}),
    );
    case
}

/// Every still-gated turn-1 relic refuses by its own name, so a census row
/// points at one relic.
///
/// `RELIC.RED_MASK`, `RELIC.BAG_OF_MARBLES` and `RELIC.LANTERN` were in this
/// loop until 2026-09-22 and are now witnessed by their parity tests further
/// down, as are `RELIC.BELLOWS` and `RELIC.FESTIVE_POPPER` since 2026-09-23
/// (#2827); the two that remain still owe their bodies. The refusal must also say
/// *which half* of the opening the body is in — a reader chasing a census row
/// to the wrong half is what the distinct variant exists to prevent.
#[test]
fn every_turn_one_gated_relic_refuses_by_its_own_name() {
    assert!(
        !super::TURN_ONE_RELIC_BODIES.is_empty(),
        "an empty table would make this loop vacuous"
    );
    for relic in super::TURN_ONE_RELIC_BODIES {
        let refusal = open(&with_relic(relic)).unwrap_err();
        assert_eq!(
            refusal.class(),
            "turn_one_relic_body_not_modeled",
            "{relic} must refuse in the turn-1 class"
        );
        assert!(refusal.to_string().contains(relic), "{refusal}");
        assert!(
            refusal.to_string().contains("begin_player_turn"),
            "the refusal must say which half of the opening the body is in: \
             {refusal}"
        );
    }
}

/// `RELIC.VERY_HOT_COCOA` is **not** touched by the turn-1 gate: it is a
/// template relic with an `AfterSideTurnStart` rule, so the hook reaches a
/// compiled subscriber and there is nothing for that gate to refuse.
///
/// Until #2736 it did not open either: [`super::pre_hook_document`] never wrote
/// the `template_relics` player field, and the boundary requires it whenever
/// the catalog derives a non-empty effective-template inventory (`boundary.rs`,
/// *"effective template inventory is missing"*), so every fight holding any of
/// the eighteen template relics refused there. The field now comes from
/// `boundary::relic_derived_player_scalars` — the oracle's
/// `tuple(r for r in entry["relics_entering"] if r in TEMPLATE_RELICS)`
/// (frozen Python `start_combat`, deleted #2827) — and the save opens **digest-equal to the
/// oracle**: `5cf0008c…`, with `energy: 7` (the Cocoa's +4 over the default 3
/// applied by its compiled `AfterSideTurnStart` rule) and the field carrying
/// exactly the one template relic.
///
/// Together with [`a_brimstone_save_still_opens_with_its_strength_rows`] this is
/// the positive half of the turn-1 gate's scope: a template relic and a
/// hand-authored one both reach a document, and both agree with the oracle.
#[test]
fn a_very_hot_cocoa_save_opens_with_its_template_relics_field_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.VERY_HOT_COCOA")).expect("Very Hot Cocoa opens");
    assert_eq!(
        opening.document.player.get("template_relics"),
        Some(&serde_json::json!(["RELIC.VERY_HOT_COCOA"]))
    );
    assert_eq!(opening.document.player.get("energy"), Some(&Value::from(7)));
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "5cf0008c562e813dc6d97dfa647eef77c38d746c900d50f372ba0054c0882a83"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

/// A Regent save under a **fully-unlocked** profile opens with its
/// `spectrum_shift_generation_pool`, digest-equal to the oracle.
///
/// This is the corpus shape #2736 reached: every Regent save holds
/// `RELIC.DIVINE_RIGHT`, a template relic, so no Regent fight opened before
/// `template_relics` was written — and the first two that did diverged from
/// the oracle by exactly this field, which `start_combat` writes for a Regent
/// with a non-empty profile (frozen Python, deleted #2827) and the opening did
/// not. Mutation on the fixture case with its standing two-slot belt (a
/// fully-unlocked profile also unlocks the potion pool, which is what admits
/// that belt, and is incoherent with the empty-belt variant):
/// `character_id = "CHARACTER.REGENT"`, `unlock_state.unlocked_epochs` = the
/// two corpus Regent fights' own 55-epoch profile (the universe less
/// `DEFECT6_EPOCH`/`DEFECT7_EPOCH`), `RELIC.DIVINE_RIGHT` appended. That
/// profile is still *partial* to the catalog — the two missing epochs gate
/// Defect rows — but no Colorless row is gated on them, so the profile-exact
/// pool is the full 50 rows (`ANOINTED, AUTOMATION, BEAT_DOWN, …`), which is
/// what the boundary re-emits, and the document round-trips losslessly.
///
/// The digest was the frozen oracle's `f8152b81…` until #3024, which found
/// that the oracle (and Rust with it) zeroed `stars_gained_this_turn` at the
/// opening's first turn start. Native keeps Divine Right's three pre-turn
/// Stars for turn one (`engine::turn::roll_stars_gained_window` carries the
/// IL), so the document now carries `stars_gained_this_turn: 3` and the pin
/// moved deliberately to the Rust value.
#[test]
fn a_regent_save_under_a_full_profile_writes_its_spectrum_shift_pool_and_matches_the_oracle() {
    let mut case = case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.DIVINE_RIGHT"}),
    );
    case["save"]["players"][0]["character_id"] = Value::from("CHARACTER.REGENT");
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
        crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .filter(|epoch| !matches!(**epoch, "DEFECT6_EPOCH" | "DEFECT7_EPOCH"))
            .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
            .collect(),
    );
    let opening = open(&case).expect("a Regent save under a full profile opens");
    let pool = opening.document.player["spectrum_shift_generation_pool"]
        .as_array()
        .expect("the pool is written as an array");
    assert_eq!(pool.len(), 50);
    assert_eq!(
        &pool[..3],
        &[
            Value::from("ANOINTED"),
            Value::from("AUTOMATION"),
            Value::from("BEAT_DOWN")
        ]
    );
    assert_eq!(
        opening.document.player.get("template_relics"),
        Some(&serde_json::json!(["RELIC.DIVINE_RIGHT"]))
    );
    // #3024: Divine Right's pre-turn Stars count for all of turn one, so the
    // opening's first `begin_player_turn` must not roll them.
    assert_eq!(opening.document.player.get("stars"), Some(&Value::from(3)));
    assert_eq!(
        opening.document.player.get("stars_gained_this_turn"),
        Some(&Value::from(3))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "0bc879479ec11e27d15608625b53532793245a8cf2d1e5c26e84e107227886a4"
    );
}

/// A Regent save under a **partial** profile opens with its profile-exact
/// 42-card pool, and the opened document round-trips losslessly (#2739).
///
/// Under `COLORLESS1_EPOCH` + `COLORLESS2_EPOCH` the derivation yields the
/// oracle's 42-card pool (`AUTOMATION, BOLAS, CATASTROPHE, …`). Until #2739
/// `HotBoundary::to_canonical` re-emitted the fully-unlocked 50-row constant
/// for this field whatever profile it was admitted under, and the opening's
/// output IS that re-emission, so this case refused by name
/// (`GenerationPoolPartialUnlock { source: "Spectrum Shift" }`) rather than
/// open to a document the oracle disagrees with. The projection is now the
/// same derivation in both directions and the refusal is retired.
///
/// The frozen oracle's digest for this case is `e38d3ebd…`. Rust's differs
/// from it by exactly the #3024 correction the full-profile sibling above
/// carries (Divine Right's three pre-turn Stars count for turn one, so
/// `stars_gained_this_turn` is 3 where the oracle zeroed and so elided it):
/// dropping that one field reproduces `e38d3ebd…` byte for byte.
#[test]
fn a_regent_save_under_a_partial_profile_opens_with_its_profile_exact_pool() {
    let mut case = with_relic("RELIC.DIVINE_RIGHT");
    case["save"]["players"][0]["character_id"] = Value::from("CHARACTER.REGENT");
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] =
        serde_json::json!(["EPOCH.COLORLESS1_EPOCH", "EPOCH.COLORLESS2_EPOCH"]);
    let opening = open(&case).expect("a partial-profile Regent opens since #2739");
    let pool: Vec<&str> = opening.document.player["spectrum_shift_generation_pool"]
        .as_array()
        .expect("the pool is written as an array")
        .iter()
        .map(|id| id.as_str().expect("pool rows are ids"))
        .collect();
    assert_eq!(pool.len(), 42);
    assert_eq!(&pool[..3], &["AUTOMATION", "BOLAS", "CATASTROPHE"]);
    for dropped in [
        "ANOINTED",
        "BEAT_DOWN",
        "CALAMITY",
        "NOSTALGIA",
        "PROWESS",
        "REND",
        "SCRAWL",
        "SPLASH",
    ] {
        assert!(
            !pool.contains(&dropped),
            "{dropped} is gated on an unrecorded epoch"
        );
    }
    // The pre-hook document carries the same pool: nothing downstream of the
    // opening's input changed it on the way through the hot state.
    let built = pre_hook(&case).expect("the pre-hook document builds");
    assert_eq!(
        built.document.player.get("spectrum_shift_generation_pool"),
        opening
            .document
            .player
            .get("spectrum_shift_generation_pool")
    );
    // from_canonical -> to_canonical is the identity on the opened document.
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
    assert_eq!(
        crate::boundary::HotBoundary::to_canonical(&state, &catalog),
        opening.document
    );

    assert_eq!(
        opening.document.player.get("stars_gained_this_turn"),
        Some(&Value::from(3))
    );
    // The oracle zeroed the counter, and zero is the default it elides.
    let mut oracle_shape = opening.document.clone();
    oracle_shape.player.remove("stars_gained_this_turn");
    assert_eq!(
        oracle_shape.differential_digest(),
        "e38d3ebd4f0c5ee1a60d17631e9e42efbb96e32791aa69862024f03e2162cdd1"
    );
}

/// The same Regent with **no** recorded profile writes no pool at all: the
/// oracle's condition is `character == "CHARACTER.REGENT" and
/// normalized_card_pool_epochs`, and an empty profile is the empty tuple,
/// elided. Oracle digest `45ecbbe7…` (no Divine Right here, so the only
/// difference from the empty-belt control is the character and its pool).
#[test]
fn a_regent_save_without_a_profile_writes_no_spectrum_shift_pool() {
    let mut case = empty_belt_case();
    case["save"]["players"][0]["character_id"] = Value::from("CHARACTER.REGENT");
    let opening = open(&case).expect("a Regent save without a profile opens");
    assert_eq!(
        opening
            .document
            .player
            .get("spectrum_shift_generation_pool"),
        None
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "45ecbbe7e9da5fae6be7b3d0f532c69e46d59e0fbe7e2aab6879898f297a388f"
    );
}

/// The pre-hook document itself carries the field, before any hook fires.
///
/// The digest test above could pass with the field arriving from somewhere
/// downstream (the boundary's own `to_canonical` re-emits it); this pins that
/// the opening's **input** document is boundary-admissible on its own, which
/// is the claim #2736 makes. Ectoplasm is a second template relic with no
/// opening-window body at all, so the only thing that can differ from the
/// control is the field itself and its two-relic order.
#[test]
fn the_pre_hook_document_writes_template_relics_in_inventory_order() {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.ECTOPLASM"}),
    );
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 2, "id": "RELIC.VERY_HOT_COCOA"}),
    );
    let built = pre_hook(&case).expect("two template relics build a pre-hook");
    assert_eq!(
        built.document.player.get("template_relics"),
        Some(&serde_json::json!([
            "RELIC.ECTOPLASM",
            "RELIC.VERY_HOT_COCOA"
        ]))
    );
    // And the control carries none: the empty tuple is the default and is
    // elided, exactly as the oracle elides it.
    let control = pre_hook(&empty_belt_case()).expect("the control builds");
    assert_eq!(control.document.player.get("template_relics"), None);
}

/// `RELIC.BRIMSTONE` still opens with its Strength rows unchanged.
///
/// Brimstone shares `AfterSideTurnStart` with `RELIC.LANTERN` and is the reason
/// the gate had to be derived rather than swept: it is a non-template relic
/// with a hand-authored engine body (`engine::relics::after_side_turn_start_late`,
/// `Brimstone/<AfterSideTurnStart>d__8::MoveNext` `0x32098c`), so a gate keyed
/// on "non-template" alone would have refused it. Oracle: `cd63aaa2…`, owner
/// Strength 2 and +1 on each Toadpole.
#[test]
fn a_brimstone_save_still_opens_with_its_strength_rows() {
    let opening = open(&with_relic("RELIC.BRIMSTONE")).expect("Brimstone opens");
    assert_eq!(
        opening.document.player.get("strength"),
        Some(&Value::from(2))
    );
    let monster_strength: Vec<Option<i64>> = opening
        .document
        .monsters
        .iter()
        .map(|monster| monster.get("strength").and_then(Value::as_i64))
        .collect();
    assert_eq!(monster_strength, vec![Some(1), Some(1)]);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "cd63aaa2d02acdfa4eb5000d2d578f10795d6fd55155150675fac8667622b265"
    );
}

// ---------------------------------------------------------------------------
// The three room-entry relic bodies (#2693)
// ---------------------------------------------------------------------------
//
// `BAG_OF_PREPARATION`, `MEAL_TICKET` and `PENDULUM` left
// `OPENING_WINDOW_RELIC_BODIES` on 2026-09-22; these are their parity
// witnesses. Every expected digest below was printed by the oracle with the
// recipe in the comment block above `empty_belt_case`, and the fixture's own
// `EMPTY_BELT_DIGEST` was reproduced by the same script in the same run — so
// these are comparisons against Python, not transcriptions of Rust's output.
//
// The measured hand sizes are what makes each one non-vacuous: the control
// deals five, and a relic whose effect were dropped would still produce a
// document (a different one) rather than a refusal, which is the failure mode
// #2731 was filed about.

/// The fixture case with an empty belt plus one relic carrying a saved
/// single-integer counter (the `relic_counters` shape `entry::relics` routes a
/// one-row `"ints"` bag into — the same shape Ember Tea's `CombatsLeft` uses).
fn with_relic_counter(id: &str, counter: &str, value: i64) -> Value {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": id,
            "props": {"ints": [{"name": counter, "value": value}]}}),
    );
    case
}

fn hand_size(document: &crate::canonical::CanonicalStateV2) -> usize {
    document.piles["hand"].len()
}

/// `RELIC.BAG_OF_PREPARATION` draws two extra cards on the first player turn.
///
/// The gate entry was not stale: nothing in the crate read the `bag_draws`
/// mirror `boundary.rs` has always written, so before this slice the relic
/// reached the projected inventory and its effect did not — the same shape as
/// Red Mask's missing Weak tokens. `engine::relics::modifier_total` now folds
/// it, and the oracle's document for this save says the hand is seven cards.
#[test]
fn a_bag_of_preparation_save_draws_two_extra_cards_on_turn_one() {
    let opening = open(&with_relic("RELIC.BAG_OF_PREPARATION")).expect("Bag of Preparation opens");
    assert_eq!(
        opening.document.player.get("bag_draws"),
        Some(&Value::from(2)),
        "the mirror is `2 * (owns(BagOfPreparation) + owns(RingOfTheSnake))`"
    );
    assert_eq!(
        hand_size(&opening.document),
        7,
        "five dealt plus the bag's two"
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "d73d6f982193ed80c2495cb1bf45b9daeba3dbd10d1b72106540a350950ef818"
    );
    // Not the control: the two cards really moved out of the draw pile.
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

/// `RELIC.RING_OF_THE_SNAKE` draws two extra cards on the first player turn
/// (#2827).
///
/// `RingOfTheSnake::ModifyHandDraw` (v0.111.0 RVA `0x9a749`) is Bag of
/// Preparation's body byte for byte — `TurnNumber; ldc.i4.1; ble.s` at
/// `IL_000c`-`IL_001d`, then `draw + DynamicVars.Cards.BaseValue` at
/// `IL_0021`-`IL_0037`, with `get_CanonicalVars` (`0x9a73c`) `CardsVar(2)` —
/// and it declares no other hook. The digest is the oracle's for the fixture
/// save with `{'floor_added_to_deck': 1, 'id': 'RELIC.RING_OF_THE_SNAKE'}`
/// appended, printed with the recipe above `empty_belt_case`. The hand matches
/// Bag of Preparation's card for card (same save, same shuffle, same +2), and
/// the digest differs from it only by the relic row.
#[test]
fn a_ring_of_the_snake_save_draws_two_extra_cards_on_turn_one() {
    let opening = open(&with_relic("RELIC.RING_OF_THE_SNAKE")).expect("Ring of the Snake opens");
    assert_eq!(
        opening.document.player.get("bag_draws"),
        Some(&Value::from(2))
    );
    assert_eq!(
        hand_size(&opening.document),
        7,
        "five dealt plus the ring's two"
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "5399443dd900944cef7becfb935360734a38de2769acf63dcc42f082684ef783"
    );
    let bag = open(&with_relic("RELIC.BAG_OF_PREPARATION")).expect("Bag of Preparation opens");
    assert_eq!(opening.document.piles, bag.document.piles);
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        bag.document.differential_digest()
    );
}

/// Ring of the Snake beside Bag of Preparation, the relic its `ModifyHandDraw`
/// row shares a field with (#2827).
///
/// The digest is the oracle's, from the recipe above `empty_belt_case` with
/// the ring then the bag appended. The shared `bag_draws` field reads 4 and the
/// hand is nine (frozen Python `start_combat`, deleted #2827, sums both). A fold that made
/// either row exclusive (the #2755 guarded-arm shape) draws seven and fails.
///
/// Big Mushroom, the other turn-1 row in that fold, is not paired here: it is
/// still in `OPENING_WINDOW_RELIC_BODIES` for its room-entry body and refuses
/// before the deal.
#[test]
fn ring_of_the_snake_and_bag_of_preparation_sum_to_four_extra_cards() {
    let mut case = with_relic("RELIC.RING_OF_THE_SNAKE");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BAG_OF_PREPARATION"}),
    );
    let opening = open(&case).expect("ring and bag open");
    assert_eq!(
        opening.document.player.get("bag_draws"),
        Some(&Value::from(4))
    );
    assert_eq!(hand_size(&opening.document), 9);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "1819320bafc571556fb0c5143024b9b1d7d13959474200a23c00f0fed426aa40"
    );
}

/// A Silent starter save with Ring of the Snake, the relic's real home (#2827).
///
/// Mutation, on top of the recipe above `empty_belt_case`:
/// `character_id = 'CHARACTER.SILENT'`, `max_hp = current_hp = 70`, the deck
/// replaced by five `CARD.STRIKE_SILENT`, five `CARD.DEFEND_SILENT`,
/// `CARD.NEUTRALIZE` and `CARD.SURVIVOR`, and the relics replaced by
/// `[RELIC.RING_OF_THE_SNAKE]`. The digest is the oracle's for that save. No
/// starter card is a generation source, so the Silent generation-pool gate is
/// not reached; the corpus Silent fights that do hold one refuse there by name.
#[test]
fn a_silent_starter_save_with_ring_of_the_snake_matches_the_oracle() {
    let mut case = empty_belt_case();
    let player = &mut case["save"]["players"][0];
    player["character_id"] = Value::from("CHARACTER.SILENT");
    player["max_hp"] = Value::from(70);
    player["current_hp"] = Value::from(70);
    let card = |id: &str| serde_json::json!({"floor_added_to_deck": 1, "id": id});
    let mut deck = Vec::new();
    deck.extend(std::iter::repeat_n(card("CARD.STRIKE_SILENT"), 5));
    deck.extend(std::iter::repeat_n(card("CARD.DEFEND_SILENT"), 5));
    deck.push(card("CARD.NEUTRALIZE"));
    deck.push(card("CARD.SURVIVOR"));
    player["deck"] = Value::Array(deck);
    player["relics"] = serde_json::json!([card("RELIC.RING_OF_THE_SNAKE")]);
    let opening = open(&case).expect("the Silent starter opens");
    assert_eq!(hand_size(&opening.document), 7);
    assert_eq!(opening.document.player.get("hp"), Some(&Value::from(70)));
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "f1d2d82685572cde20e42071ab26a965a1873012be353591d74bc2e1b6ebf872"
    );
}

/// `RELIC.LETTER_OPENER` opens with the oracle's document (#2827).
///
/// Its `BeforeCombatStart` (`0x963a0`) zeroes `SkillsPlayedThisTurn`, which a
/// fresh combat already holds, and its turn-1 `AfterSideTurnStart`
/// (`0x963b8`) returns on turn one. The digest is Python's for the fixture save
/// with the relic appended (recipe above `empty_belt_case`), and it is **not**
/// the control's: the ownership mirror `letter_opener` is projected, so a gate
/// that dropped the relic would fail here. Nothing else moves.
#[test]
fn a_letter_opener_save_opens_with_the_oracles_document() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.LETTER_OPENER")).expect("Letter Opener opens");
    assert_eq!(
        opening.document.player.get("letter_opener"),
        Some(&Value::from(true))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "5f1f3b70e0ab4ab729c4f1f41c5d6b94d9c224f5c09ddfa01f3971e30c2d581f"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
    assert_eq!(opening.document.piles, control.document.piles);
    assert_eq!(opening.document.rng, control.document.rng);
    assert_eq!(opening.document.monsters, control.document.monsters);
}

/// `RELIC.PAELS_FLESH` opens with the oracle's document (#2827).
///
/// Its three opening hooks (`BeforeCombatStart` `0x98351`,
/// `BeforeSideTurnStart` `0x9835e`, `AfterSideTurnStart` `0x98384`) are
/// display calls on turn one, and its `ModifyMaxEnergy` (`0x9831e`) adds only
/// from turn three. The digest is Python's for the fixture save with the relic
/// appended; the turn-one energy is the control's, so a `ModifyMaxEnergy` row
/// that fired early would fail here.
#[test]
fn a_paels_flesh_save_opens_with_the_oracles_document_and_no_turn_one_energy() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.PAELS_FLESH")).expect("Pael's Flesh opens");
    assert_eq!(
        opening.document.player.get("paels_flesh"),
        Some(&Value::from(true))
    );
    assert_eq!(
        opening.document.player.get("energy"),
        control.document.player.get("energy")
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "61df26872521d3d72525ee1859f7c0d564212ae1af85c724484e590acf86b4fc"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
    assert_eq!(opening.document.piles, control.document.piles);
    assert_eq!(opening.document.rng, control.document.rng);
}

/// Both relics together, Letter Opener appended first (#2827). They share
/// `BeforeCombatStart` and `AfterSideTurnStart`, and every body on either is
/// inert on turn one, so the order cannot matter; the digest is Python's for
/// that save.
#[test]
fn letter_opener_and_paels_flesh_open_together_with_the_oracles_document() {
    let mut case = with_relic("RELIC.LETTER_OPENER");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.PAELS_FLESH"}),
    );
    let opening = open(&case).expect("Letter Opener and Pael's Flesh open");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "2f2fbb06d3f093dd19c9a9b4c1cfb3c28ebc3affaea2de0dbaaffe1a1b523f12"
    );
}

/// `RELIC.MEAL_TICKET` opens and changes nothing, because its body is
/// combat-inert in IL.
///
/// `MealTicket/<AfterRoomEntered>d__5::MoveNext` (`0x32a63c`) `leave`s at
/// `IL_0041` unless `room isinst MerchantRoom`, so in a combat room the only
/// thing it can do is return. The oracle agrees by running no body for it at
/// all. Both halves are load-bearing here: the digest is **Python's** for this
/// save, and it is **not** the control's — the relic is in `relics_entering`
/// and its immutable mirrors, so a gate that had simply dropped the relic would
/// fail this test too.
#[test]
fn a_meal_ticket_save_opens_because_its_heal_is_merchant_room_only() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.MEAL_TICKET")).expect("Meal Ticket opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "4d7c17672d952b978bd5209bad807c40c711033a069835286429a220097c3887"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
    // No heal, and nothing else moved either: the relic row is the whole diff.
    assert_eq!(
        opening.document.player.get("hp"),
        control.document.player.get("hp")
    );
    assert_eq!(hand_size(&opening.document), 5);
    assert_eq!(opening.document.piles, control.document.piles);
    assert_eq!(opening.document.rng, control.document.rng);
}

/// `RELIC.PENDULUM`'s counter advances every player turn and grants on the wrap.
///
/// Three saved counters, three different documents. `TurnsSeen = 2` wraps to
/// `0` and draws the sixth card; `0` and `1` advance without granting, and
/// their documents still differ from each other and from the control because
/// the counter itself is projected. That is what distinguishes a modeled
/// advance from an absent one — a gate that ran no body at all would leave the
/// seeded value untouched and produce neither `1`, `2` nor `0`.
#[test]
fn a_pendulum_save_advances_its_counter_and_grants_on_the_wrap() {
    for (seen, after, hand, digest) in [
        (
            0,
            1,
            5,
            "df7d7412073e61c2056a52f3c5d8f5d4159d25b687f913b6c9a965d771cd1116",
        ),
        (
            1,
            2,
            5,
            "e624b0df980427b9b0a162efa71dfa278e2f4b547db289b01c2448c7a0533787",
        ),
        (
            2,
            0,
            6,
            "cf93c0239a3110333c5abf848369bb56fdbccec1d96861e0ab4850220e432d9f",
        ),
    ] {
        let case = with_relic_counter("RELIC.PENDULUM", "TurnsSeen", seen);
        let opening = open(&case).unwrap_or_else(|err| panic!("Pendulum {seen} opens: {err}"));
        assert_eq!(
            opening.document.player.get("pendulum"),
            Some(&Value::from(after)),
            "TurnsSeen {seen} advances to {after}"
        );
        assert_eq!(hand_size(&opening.document), hand, "TurnsSeen {seen}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "TurnsSeen {seen}"
        );
        assert_ne!(
            python_view(&opening.document).differential_digest(),
            EMPTY_BELT_DIGEST
        );
    }
}

/// The two draw relics on one save, because the fold they share used to be
/// exclusive.
///
/// Bag of Preparation's `+2` and Pendulum's `+1` are separate statements over
/// one running total in the oracle (`_begin_player_turn_hand_draw`, frozen Python, deleted #2827) and separate listeners in native, so the hand is eight. The
/// same commit made `engine::relics::modifier_total`'s `ModifyHandDraw` arm one
/// non-exclusive fold; before it, Big Mushroom's turn-1 arm silently skipped
/// every row below it, and the two relics here are the ones that would have
/// inherited that trap.
#[test]
fn the_turn_one_draw_relics_stack_on_one_save() {
    // Appended in the oracle recipe's order: `relics_entering` is projected as
    // a list, so a different acquisition order is a different document.
    let mut case = with_relic("RELIC.BAG_OF_PREPARATION");
    let pendulum = serde_json::json!({
        "floor_added_to_deck": 1, "id": "RELIC.PENDULUM",
        "props": {"ints": [{"name": "TurnsSeen", "value": 2}]}});
    relic(&mut case, pendulum.clone());
    let opening = open(&case).expect("both draw relics open");
    assert_eq!(hand_size(&opening.document), 8, "five plus two plus one");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "140e13ad0c90c89a12de8135e8efd9be0ad04e5ddfa406d8be5b0b47287c1a09"
    );

    // And all three of this slice's relics together, which is the relic subset
    // the #2693 residual roots carry.
    let mut case = with_relic("RELIC.BAG_OF_PREPARATION");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.MEAL_TICKET"}),
    );
    relic(&mut case, pendulum);
    let opening = open(&case).expect("all three open");
    assert_eq!(hand_size(&opening.document), 8, "Meal Ticket adds nothing");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "9ac4703ab664a6d990578988e5dc7ab8c2466da463513aa36890e860fefbfdab"
    );
}

/// The relic whose per-fight field this crate cannot represent refuses by
/// name, and it really would have opened.
///
/// The second half is what makes this non-vacuous and is the whole reason the
/// gate exists: on the tree that retired the three room-entry bodies of #2755,
/// every member of the original six **opened** and emitted a document Python
/// disagrees with — measured, two of them in the corpus census itself
/// (`f9fa9758fb0ef32c` and `f59f9a50bb55bf0b`, both on
/// `player.throwing_axe_available`) and all six by the per-relic sweep the
/// gate's doc comment records. Five are modeled by #2756 (the four flags below
/// plus Blood Vial's heal) and the sixth, `RELIC.DRAGON_FRUIT`, by #3320
/// (witnessed below), so the table is empty and its loops are vacuous today.
#[test]
fn the_unseeded_per_fight_state_refusal_names_the_relic_and_the_field() {
    for (relic, field) in super::PER_FIGHT_RELIC_STATE_UNSEEDED {
        let refusal = open(&with_relic(relic)).expect_err("it refuses");
        assert_eq!(
            refusal.class(),
            "per_fight_relic_state_not_seeded",
            "{relic} must refuse in its own class"
        );
        let text = refusal.to_string();
        assert!(text.contains(relic), "{text}");
        assert!(text.contains(field), "{text}");
    }
    // No relic is in this table and a body table too, so its census class does
    // not depend on the order the loops run in — and, since #2756, no relic is
    // in both the gated and the seeded table, which would be a document that
    // is written and then refused.
    for (relic, _) in super::PER_FIGHT_RELIC_STATE_UNSEEDED {
        assert!(
            !super::OPENING_WINDOW_RELIC_BODIES.contains(&relic)
                && !super::TURN_ONE_RELIC_BODIES.contains(&relic)
                && !super::TURN_ONE_ORB_RELICS.contains(&relic)
                && !super::PER_FIGHT_RELIC_STATE_SEEDED
                    .iter()
                    .any(|(seeded, _)| *seeded == relic),
            "{relic} is gated twice"
        );
    }
    // #3320 emptied the table; the class and its message survive for the
    // #2779 derivation.
    let refusal = OpeningRefusal::PerFightRelicStateNotSeeded {
        relic: "RELIC.X".to_string(),
        field: "x_field",
    };
    assert_eq!(refusal.class(), "per_fight_relic_state_not_seeded");
    let text = refusal.to_string();
    assert!(
        text.contains("RELIC.X") && text.contains("x_field"),
        "{text}"
    );
}

/// Dragon Fruit opens (#3320): its `dragon_fruit` mirror is written and
/// nothing else in the document moves, because the relic has no instance
/// field and no body on any hook the opening fires.
///
/// Before #3320 this save refused as `per_fight_relic_state_not_seeded`. The
/// comparison against the control is what makes the witness non-vacuous: the
/// document must differ from the control in exactly `relics_entering` and
/// `dragon_fruit`, so a max-HP or HP write at combat start would fail it. The
/// opened root must then load and admit.
#[test]
fn a_dragon_fruit_save_opens_with_only_its_ownership_mirror() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.DRAGON_FRUIT")).expect("Dragon Fruit opens");
    assert_eq!(
        opening.document.player.get("dragon_fruit"),
        Some(&Value::Bool(true))
    );
    assert_eq!(control.document.player.get("dragon_fruit"), None);
    let mut expected = control.document.clone();
    expected
        .player
        .get_mut("relics_entering")
        .and_then(Value::as_array_mut)
        .expect("the control owns its starter relic")
        .push(Value::from("RELIC.DRAGON_FRUIT"));
    expected
        .player
        .insert("dragon_fruit".to_owned(), Value::Bool(true));
    assert_eq!(opening.document, expected);
    let catalog =
        crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).expect("catalog");
    let state =
        crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).expect("state");
    assert!(state.max_hp == control_max_hp(&control), "max HP unchanged");
    crate::engine::admission::admit(&opening.document, &state, &catalog)
        .expect("a Dragon Fruit root admits");
}

fn control_max_hp(control: &Opening) -> i32 {
    control.document.player["max_hp"].as_i64().expect("max_hp") as i32
}

/// The fixture case at `hp`, with `relics` appended and then a Maw Bank whose
/// saved `HasItemBeenBought` is `bought` (`None` omits `props`).
///
/// Mutation: `p['current_hp'] = hp`, each relic
/// `relics.append({'floor_added_to_deck': 1, 'id': id})`, then
/// `relics.append({'floor_added_to_deck': 1, 'id': 'RELIC.MAW_BANK', 'props':
/// {'bools': [{'name': 'HasItemBeenBought', 'value': bought}]}})`.
fn with_maw_bank(hp: i64, relics: &[&str], bought: Option<bool>) -> Value {
    let mut case = at_hp(hp, relics);
    let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.MAW_BANK"});
    if let Some(bought) = bought {
        row["props"] =
            serde_json::json!({"bools": [{"name": "HasItemBeenBought", "value": bought}]});
    }
    relic(&mut case, row);
    case
}

/// `(gold, hp, max_hp)` of an opened document; an elided gold is the default 0.
fn gold_hp_max(document: &crate::canonical::CanonicalStateV2) -> (i64, i64, i64) {
    let field = |name: &str| document.player.get(name).and_then(Value::as_i64);
    (
        field("gold").unwrap_or(0),
        field("hp").expect("hp"),
        field("max_hp").expect("max_hp"),
    )
}

/// Maw Bank's room-entry gold (#3328), one row per branch of
/// `MawBank/<AfterRoomEntered>d__12::MoveNext` (`0x32a534`) and of the
/// `GainGold` it calls:
///
/// * nothing bought: `+12` gold (`IL_0064`), and nothing else moves;
/// * an item bought: the body `leave`s at `IL_0042`, the document is the
///   control's;
/// * Ectoplasm: `ModifyGoldGained` zeroes the gain, so no gold;
/// * Bowler Hat: `floor(12 * 1.25)` = `+15`;
/// * Dragon Fruit: the gain's `AfterGoldGained` adds 1 max HP and heals 1;
/// * Dragon Fruit with Planisphere on a `?` point: the heal commutes, 6 in
///   all.
///
/// Every armed row is compared with the same save opened without the Maw
/// Bank row, so the only difference is what the body wrote. Every opened root
/// must load and admit.
#[test]
fn maw_bank_gains_its_room_entry_gold_only_when_nothing_was_bought() {
    const HP: i64 = 40;
    for (label, relics, bought, delta) in [
        ("nothing bought", &[][..], false, (12, 0, 0)),
        ("item bought", &[][..], true, (0, 0, 0)),
        ("Ectoplasm", &["RELIC.ECTOPLASM"][..], false, (0, 0, 0)),
        ("Bowler Hat", &["RELIC.BOWLER_HAT"][..], false, (15, 0, 0)),
        (
            "Dragon Fruit",
            &["RELIC.DRAGON_FRUIT"][..],
            false,
            (12, 1, 1),
        ),
        (
            "Dragon Fruit, item bought",
            &["RELIC.DRAGON_FRUIT"][..],
            true,
            (0, 0, 0),
        ),
    ] {
        let control = open(&at_hp(HP, relics)).unwrap_or_else(|r| panic!("{label} control: {r}"));
        let opening = open(&with_maw_bank(HP, relics, Some(bought)))
            .unwrap_or_else(|r| panic!("{label}: {r}"));
        let (gold, hp, max_hp) = gold_hp_max(&control.document);
        assert_eq!(
            gold_hp_max(&opening.document),
            (gold + delta.0, hp + delta.1, max_hp + delta.2),
            "{label}"
        );
        // Nothing but relics_entering and the three scalars moves.
        let mut expected = control.document.clone();
        expected
            .player
            .get_mut("relics_entering")
            .and_then(Value::as_array_mut)
            .expect("relics_entering")
            .push(Value::from("RELIC.MAW_BANK"));
        for (field, value) in [
            ("gold", gold + delta.0),
            ("hp", hp + delta.1),
            ("max_hp", max_hp + delta.2),
        ] {
            if value != 0 {
                expected.player.insert(field.to_owned(), Value::from(value));
            }
        }
        assert_eq!(opening.document, expected, "{label}");
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
            .expect("catalog");
        let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
            .expect("state");
        crate::engine::admission::admit(&opening.document, &state, &catalog)
            .unwrap_or_else(|r| panic!("{label} admits: {r:?}"));
    }
}

/// Planisphere's `?`-point heal and Dragon Fruit's `+1/+1` commute
/// ([`super::maw_bank_after_room_entered`]): at 1 below max the pair ends at
/// the new max whichever runs first, and far below max it heals 6.
#[test]
fn maw_bank_dragon_fruit_heal_commutes_with_planisphere() {
    let relics = ["RELIC.DRAGON_FRUIT", "RELIC.PLANISPHERE"];
    for (hp_below_max, healed) in [(1, 2), (20, 6)] {
        let at_point = |case: &Value| {
            let mut entry = entry_facts(case);
            entry.hp_entering = entry.max_hp_entering - hp_below_max;
            entry.map_point_type = Some("unknown".to_string());
            entry.next_normal_encounter = Some(entry.encounter_id.clone());
            entry
        };
        let control = at_point(&at_hp(40, &relics));
        let armed = at_point(&with_maw_bank(40, &relics, Some(false)));
        let (hp, max_hp) = (armed.hp_entering, armed.max_hp_entering);
        let control = open_facts(&control).expect("the ? control opens");
        assert_eq!(gold_hp_max(&control.document).1, (hp + 5).min(max_hp));
        let opening = open_facts(&armed).expect("the armed ? case opens");
        let (_, opened_hp, opened_max) = gold_hp_max(&opening.document);
        assert_eq!(
            (opened_hp, opened_max),
            (hp + healed, max_hp + 1),
            "{hp_below_max} below max"
        );
    }
}

/// An armed Maw Bank beside Dragon Fruit **and** Red Skull refuses by Maw
/// Bank's name: Red Skull's room-entry threshold and latch would see a heal in
/// an acquisition order this opening does not run. A spent Maw Bank beside the
/// same pair is inert, so it does not take the refusal, and neither does an
/// armed one beside either relic alone.
#[test]
fn an_armed_maw_bank_beside_dragon_fruit_and_red_skull_refuses_by_name() {
    let both = ["RELIC.DRAGON_FRUIT", "RELIC.RED_SKULL"];
    assert_eq!(
        open(&with_maw_bank(40, &both, Some(false))).expect_err("the armed triple refuses"),
        OpeningRefusal::RoomEntryRelicNotModeled {
            relic: "RELIC.MAW_BANK".to_string()
        }
    );
    for (label, relics, bought) in [
        ("spent", &both[..], true),
        ("Dragon Fruit alone", &both[..1], false),
        ("Red Skull alone", &both[1..], false),
    ] {
        assert!(
            !matches!(
                open(&with_maw_bank(40, relics, Some(bought))),
                Err(OpeningRefusal::RoomEntryRelicNotModeled { .. })
            ),
            "{label} is not this refusal"
        );
    }
}

/// A Maw Bank the save does not vouch for refuses rather than guessing which
/// branch the body takes: no `props`, and the flag outside `bools`.
#[test]
fn an_unproven_maw_bank_flag_refuses_as_an_inexact_counter() {
    let mut in_ints = at_hp(40, &[]);
    relic(
        &mut in_ints,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.MAW_BANK",
                           "props": {"ints": [{"name": "HasItemBeenBought", "value": 0}]}}),
    );
    for (label, case) in [("absent", with_maw_bank(40, &[], None)), ("ints", in_ints)] {
        assert_eq!(
            open(&case).expect_err(label),
            OpeningRefusal::RelicCounterNotExact {
                relic: "RELIC.MAW_BANK",
                value: "absent".to_string()
            },
            "{label}"
        );
    }
}

// ---------------------------------------------------------------------------
// The five per-fight relic states the opening now seeds (#2756)
// ---------------------------------------------------------------------------
//
// Every expected digest below was printed by the oracle with the recipe in the
// comment block above `empty_belt_case`, and the fixture's own
// `EMPTY_BELT_DIGEST` was reproduced by the same script in the same run — so
// these are comparisons against Python, not transcriptions of Rust's output.
//
// The field assertion beside each digest is what makes it non-vacuous: before
// this slice each relic already reached `relics_entering` and its immutable
// mirrors, so an `assert_ne!(…, EMPTY_BELT_DIGEST)` alone would have passed on
// a document whose per-fight flag was still `false`. That document is exactly
// what #2755 measured diverging.

/// The fixture case with an empty belt and the player's entering HP moved, so
/// Blood Vial's clamp is observable.
fn at_hp(hp: i64, relics: &[&str]) -> Value {
    let mut case = empty_belt_case();
    case["save"]["players"][0]["current_hp"] = Value::from(hp);
    for id in relics {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    case
}

fn flag(document: &crate::canonical::CanonicalStateV2, field: &str) -> Option<Value> {
    document.player.get(field).cloned()
}

/// Each of the four seeded flags opens digest-equal to Python with its own
/// field set, and the flag is what distinguishes it from the control.
#[test]
fn the_four_seeded_per_fight_flags_match_the_oracle_one_relic_at_a_time() {
    for (relic, field, digest) in [
        (
            "RELIC.THROWING_AXE",
            "throwing_axe_available",
            "a371c61d9c36156d0a6f72b8a17cd8df35439ee9f00983f2c2bb2ef7e207a593",
        ),
        (
            "RELIC.PERMAFROST",
            "permafrost",
            "9fb1041851a959c1bb29fb7dbe9f7a821e7052ece441dbccb04d84a35b5b7164",
        ),
        (
            "RELIC.UNCEASING_TOP",
            "unceasing_top",
            "840a2642933d5fa2346700365f0f49b4e1631f6c27f7c19f1cba5579dc7ffd50",
        ),
        (
            "RELIC.CENTENNIAL_PUZZLE",
            "puzzle",
            "38523157c7c8914384a3699e67e6f83782f8f395605dcd4ff70196af96e29470",
        ),
    ] {
        let opening = open(&with_relic(relic)).unwrap_or_else(|e| panic!("{relic} opens: {e:?}"));
        assert_eq!(
            flag(&opening.document, field),
            Some(Value::Bool(true)),
            "{relic} must seed {field}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relic} must match the oracle"
        );
    }
    // The control writes none of them, so the flags are the relic's and not a
    // default this builder emits unconditionally.
    let control = open(&empty_belt_case()).expect("the control opens");
    for (_, field) in super::PER_FIGHT_RELIC_STATE_SEEDED {
        assert_eq!(
            flag(&control.document, field),
            None,
            "{field} is not a default"
        );
    }
}

/// All the seeded flags on one save, which is the combination the corpus
/// reaches (the four #2756 seeded), plus Unsettling Lamp's since #2827 item B.
///
/// Stacked rather than only measured singly because each flag has its own
/// `PlayerSlot` and a write that clobbered a neighbour would pass every
/// single-relic case above. Both digests were printed by the oracle with
/// `at_hp`'s mutation.
#[test]
fn the_four_seeded_per_fight_flags_stack_on_one_save() {
    let four = [
        "RELIC.THROWING_AXE",
        "RELIC.PERMAFROST",
        "RELIC.UNCEASING_TOP",
        "RELIC.CENTENNIAL_PUZZLE",
    ];
    let opening = open(&at_hp(68, &four)).expect("all four open together");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "1fd6ecc72af1a23d318fc3b8fff78511a99ac78849e855f580e8443a7dba7352"
    );
    let mut five = four.to_vec();
    five.push("RELIC.UNSETTLING_LAMP");
    let opening = open(&at_hp(68, &five)).expect("all five open together");
    for (_, field) in super::PER_FIGHT_RELIC_STATE_SEEDED {
        assert_eq!(flag(&opening.document, field), Some(Value::Bool(true)));
    }
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "34dcc93004c0320f4695a7dc48fed7d8f58dcde3584a0d54b841516a4dca01a2"
    );
}

/// `RELIC.BLOOD_VIAL` heals 2 at the player's first `AfterPlayerTurnStartLate`.
///
/// Its `blood_vial` flag was always projected (it is a catalog mirror, not hot
/// state); what the opening was missing is the `hp` the heal writes, which is
/// why the assertion is on HP and not on the flag. The oracle's document for
/// this save has the entering 68 at **70**.
#[test]
fn a_blood_vial_save_heals_two_at_the_first_player_turn_start_late() {
    let opening = open(&with_relic("RELIC.BLOOD_VIAL")).expect("Blood Vial opens");
    assert_eq!(
        flag(&opening.document, "blood_vial"),
        Some(Value::Bool(true))
    );
    assert_eq!(flag(&opening.document, "hp"), Some(Value::from(70)));
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "e78241839d6144038af0c46017dbceedb09080bfa4fc8702c26a840372eedf52"
    );
}

/// The heal clamps at `max_hp` rather than overshooting it.
///
/// Both directions, because a body that simply added 2 would pass the case
/// above: entering at 79 of 80 heals **one** point, and entering at 80 heals
/// none — so the two documents are digest-**identical** to each other and both
/// differ from a no-relic control at the same entering HP.
#[test]
fn the_blood_vial_heal_clamps_at_max_hp() {
    const CAPPED: &str = "6e2902962a05320c3db7b587d02b49c1d80ba22f8b1a2e0946b9032720dc68aa";
    let at_79 = open(&at_hp(79, &["RELIC.BLOOD_VIAL"])).expect("79 of 80 opens");
    assert_eq!(flag(&at_79.document, "hp"), Some(Value::from(80)));
    assert_eq!(at_79.document.differential_digest(), CAPPED);

    let at_80 = open(&at_hp(80, &["RELIC.BLOOD_VIAL"])).expect("80 of 80 opens");
    assert_eq!(flag(&at_80.document, "hp"), Some(Value::from(80)));
    assert_eq!(at_80.document.differential_digest(), CAPPED);

    // The controls at the same entering HP, so "clamped" is not "the relic did
    // nothing": at 79 the relic moves the document, at 80 only its own mirror
    // separates the two.
    let control_79 = open(&at_hp(79, &[])).expect("the 79 control opens");
    assert_eq!(flag(&control_79.document, "hp"), Some(Value::from(79)));
    assert_eq!(
        control_79.document.differential_digest(),
        "9ef9dcaf1acc58dc067b6e7aceda9d368dbdd21446b6935552147fd8ef200dd5"
    );
    let control_80 = open(&at_hp(80, &[])).expect("the 80 control opens");
    assert_eq!(
        control_80.document.differential_digest(),
        "e9b0d71b2dca13c5a5fad57addcee4c6365e6e0c4900640395553e7eeaa3c7bc"
    );
    assert_ne!(control_80.document.differential_digest(), CAPPED);
}

/// The engine's bag fold and the boundary's `bag_draws` mirror are one fact.
///
/// `modifier_total` computes the pair's contribution from ownership and
/// `boundary.rs` projects the same sum as a player field; if they ever
/// disagreed the document would contradict the draw it produced. Read off the
/// same catalog rather than asserted twice.
#[test]
fn the_bag_relic_fold_agrees_with_the_projected_mirror() {
    let opening = open(&with_relic("RELIC.BAG_OF_PREPARATION")).expect("Bag of Preparation opens");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .expect("the opened document builds a catalog");
    assert_eq!(
        crate::boundary::relic_derived_player_scalars(&catalog).get("bag_draws"),
        Some(&Value::from(2))
    );
    let mut state = crate::hot::HotState::at_defaults();
    state.turn = 1;
    assert_eq!(
        crate::engine::modifier_total(&catalog, crate::hooks::HookEvent::ModifyHandDraw, &state),
        Ok(2),
        "the fold must contribute exactly the mirror"
    );
    state.turn = 2;
    assert_eq!(
        crate::engine::modifier_total(&catalog, crate::hooks::HookEvent::ModifyHandDraw, &state),
        Ok(0),
        "and nothing past the first player turn (`ble` at IL_001d)"
    );
}

// ---------------------------------------------------------------------------
// The two turn-1 bodies this slice modeled (#2693)
// ---------------------------------------------------------------------------
//
// `RED_MASK`, `BAG_OF_MARBLES` and `LANTERN` left `TURN_ONE_RELIC_BODIES` on
// 2026-09-22. Every expected digest below was printed by the oracle with the
// recipe in the comment block above `empty_belt_case`, in the same run that
// reproduced `EMPTY_BELT_DIGEST` — so these compare against Python, not
// against Rust's own output.
//
// What makes each non-vacuous is that the pre-gate Rust document (#2735's
// mutation table) was **not** the control's: the relic always reached
// `relics_entering` and its immutable mirrors, and only the effect was
// missing. So `assert_ne!(…, EMPTY_BELT_DIGEST)` alone would have passed
// before this slice; the positive assertion on the effect is the witness.

fn monster_field(document: &crate::canonical::CanonicalStateV2, field: &str) -> Vec<Value> {
    document
        .monsters
        .iter()
        .map(|monster| monster.get(field).cloned().unwrap_or(Value::Null))
        .collect()
}

/// `RELIC.RED_MASK` applies Weak 1 to every hittable enemy on turn 1.
///
/// `RedMask/<BeforeSideTurnStart>d__6::MoveNext` `0x32f4b8`, cited at the body
/// in `engine::relics::TURN_ONE_ALL_ENEMY_DEBUFFS`. Both halves matter: the
/// scalar **and** the `misery_debuff_order` token, because the acquisition
/// position is what #2693's residual turns on — a Weak that landed without
/// recording its token would leave a later Strength row at the wrong index.
#[test]
fn a_red_mask_save_applies_weak_to_every_enemy_and_records_its_token() {
    let opening = open(&with_relic("RELIC.RED_MASK")).expect("Red Mask opens");
    assert_eq!(
        monster_field(&opening.document, "weak"),
        vec![Value::from(1), Value::from(1)]
    );
    assert_eq!(
        monster_field(&opening.document, "misery_debuff_order"),
        vec![serde_json::json!(["weak"]), serde_json::json!(["weak"])]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "1fada8ea2c0f842ff2913e4c88ee7f235bde6a18af34e320693607ce53c47ce2"
    );
}

/// `RELIC.BAG_OF_MARBLES` applies Vulnerable 1, from the same loop.
///
/// It rides this slice because its body is the same shape **offset for
/// offset** (`0x31ee48`, same guard at `IL_0043`-`IL_004e`, same
/// `Owner.Creature` applier, same `Decimal::One`), which is the only reason
/// one table-driven loop is exact for both. Witnessed separately because
/// "same shape" is a claim, and a shared loop that silently applied the wrong
/// power or amount would pass Red Mask's test alone.
#[test]
fn a_bag_of_marbles_save_applies_vulnerable_to_every_enemy() {
    let opening = open(&with_relic("RELIC.BAG_OF_MARBLES")).expect("Bag of Marbles opens");
    assert_eq!(
        monster_field(&opening.document, "vuln"),
        vec![Value::from(1), Value::from(1)]
    );
    assert_eq!(
        monster_field(&opening.document, "misery_debuff_order"),
        vec![serde_json::json!(["vuln"]), serde_json::json!(["vuln"])]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "2058ca5e5fa45eff0f5005a2ea99f54fe4ec5ddbc87cad31620db8e3576c27eb"
    );
}

/// Both together, in the oracle's order: `weak` then `vuln`.
///
/// This is the ledger-position assertion, and it is the one the fixed order in
/// `TURN_ONE_ALL_ENEMY_DEBUFFS` owes a witness. On a roster with no Artifact
/// the two applications commute in every observable except the token order,
/// and the token order is exactly what `Misery` reads.
#[test]
fn the_two_all_enemy_debuff_relics_record_their_tokens_in_the_oracles_order() {
    let mut case = empty_belt_case();
    for id in ["RELIC.RED_MASK", "RELIC.BAG_OF_MARBLES"] {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    let opening = open(&case).expect("both debuff relics open");
    assert_eq!(
        monster_field(&opening.document, "misery_debuff_order"),
        vec![
            serde_json::json!(["weak", "vuln"]),
            serde_json::json!(["weak", "vuln"])
        ]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "90dea6c213c0d130a987f01576cf4c47a52cc23b0e20f0ff5618ff31ee105965"
    );
}

/// `RELIC.LANTERN` grants one Energy on the first `AfterSideTurnStart`.
///
/// `Lantern/<AfterSideTurnStart>d__6::MoveNext` `0x328268`, `get_CanonicalVars`
/// `0x95adf`. The oracle's document for this save carries `energy: 4` — the
/// default 3 plus one — and the relic is **additive**, not an exclusive arm:
/// the row sits beside Booming Conch's and the flowers' in the same function.
#[test]
fn a_lantern_save_gains_one_energy_on_the_first_turn() {
    let opening = open(&with_relic("RELIC.LANTERN")).expect("Lantern opens");
    assert_eq!(
        opening.document.player.get("energy"),
        Some(&Value::from(4)),
        "the default 3 plus Lantern's EnergyVar(1)"
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "7276d40b102c5a29751d0b162d31cd8c326a389d972d4512b3de61ea918c632b"
    );
}

/// Red Mask beside Lantern: one document, both effects, still Python's.
///
/// The two residual #2693 roots that stop on `LANTERN` also hold `RED_MASK`,
/// so the combination is the shape that actually has to open — not either
/// relic alone.
#[test]
fn red_mask_and_lantern_together_still_match_the_oracle() {
    let mut case = empty_belt_case();
    for id in ["RELIC.RED_MASK", "RELIC.LANTERN"] {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    let opening = open(&case).expect("Red Mask and Lantern open together");
    assert_eq!(opening.document.player.get("energy"), Some(&Value::from(4)));
    assert_eq!(
        monster_field(&opening.document, "weak"),
        vec![Value::from(1), Value::from(1)]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "747158793815a0b4811be9ed8583e00c878971fad6821e595bfb94a1200181df"
    );
}

/// An entering `ArtifactPower` **eats** the turn-1 debuff (#148).
///
/// This is the interaction the scoping named, and it is measured rather than
/// argued: the innate Artifact is applied at `AfterAddedToRoom`, which precedes
/// the player's first `BeforeSideTurnStart`, so it is already on the monster
/// when Red Mask fires and consumes one stack per blocked **application**
/// instead of being stacked on top of a landed Weak.
/// `ENCOUNTER.MECHA_KNIGHT_ELITE` is the one registered roster with an innate
/// Artifact (`encounters::elite::build_mecha_knight_elite`, amount **3**), and
/// the oracle's three documents for it differ in exactly the Artifact count:
/// 3 with no debuff relic, 2 with Red Mask, 1 with Red Mask and Bag of Marbles
/// — and **no** `weak` or `vuln` in any of them.
///
/// The gate itself is not re-implemented in the relic body: it is
/// `damage::apply_relic_monster_debuff`'s shared received-side
/// `ArtifactPower.TryModifyPowerAmountReceived` command, which is why this test
/// is a witness for the wiring and not for a second copy of the rule.
#[test]
fn an_entering_artifact_eats_the_turn_one_debuffs_instead_of_stacking_under_them() {
    fn mecha(relics: &[&str]) -> Value {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from("ENCOUNTER.MECHA_KNIGHT_ELITE");
        case["node_type"] = Value::from("elite");
        for id in relics {
            relic(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": id}),
            );
        }
        case
    }
    for (relics, artifact, digest) in [
        (
            &[][..],
            3,
            "55d870fecf3c3bde7e8c28e749224d24bcf41e567be4709f5feb104c6a4f60c5",
        ),
        (
            &["RELIC.RED_MASK"][..],
            2,
            "6d22d76424f627fa9f60c28e8b56bbf5577e36d51c3a135ea4836b717f38e1db",
        ),
        (
            &["RELIC.RED_MASK", "RELIC.BAG_OF_MARBLES"][..],
            1,
            "81f1d5c4fee38e5fe7d651717274b73463f5813e688208e5e490615cb3ee3c57",
        ),
    ] {
        let opening = open(&mecha(relics)).unwrap_or_else(|refusal| {
            panic!("the Mecha Knight roster opens with {relics:?}: {refusal}")
        });
        assert_eq!(
            monster_field(&opening.document, "artifact"),
            vec![Value::from(artifact)],
            "one stack per blocked application, with {relics:?}"
        );
        assert_eq!(
            monster_field(&opening.document, "weak"),
            vec![Value::Null],
            "a blocked application adds nothing, with {relics:?}"
        );
        assert_eq!(
            monster_field(&opening.document, "vuln"),
            vec![Value::Null],
            "a blocked application adds nothing, with {relics:?}"
        );
        assert_eq!(
            monster_field(&opening.document, "misery_debuff_order"),
            vec![Value::Null],
            "and no acquisition token, with {relics:?}"
        );
        assert_eq!(python_view(&opening.document).differential_digest(), digest);
    }
}

/// A roster whose Artifact blocks **some but not all** of the owner's debuff
/// relics refuses rather than guessing their `Player.Relics` order.
///
/// The oracle's `partial_artifact` (frozen Python `start_combat`, deleted #2827), ported as
/// `OpeningRefusal::TurnOneDebuffPartialArtifact`. Asserted against
/// `turn_one_debuff_artifact_is_unambiguous` directly rather than end to end,
/// because it is **not reachable through the current roster registry**: the
/// condition needs `0 < artifact < n`, `n` is at most 2 while
/// `RELIC.TWISTED_FUNNEL` is still gated, and the one registered roster with an
/// innate Artifact carries 3 (above). That makes this an exact guard with no
/// corpus witness yet — stated here rather than left as an unexplained
/// unreachable branch, and the boundary cases either side of it are pinned in
/// the same loop so the comparison is not off by one.
#[test]
fn a_partial_artifact_roster_refuses_rather_than_guessing_the_relic_order() {
    use crate::encounters::MonsterSpec;
    use crate::ids::MonsterKind;

    let roster = |artifact: i64| {
        vec![
            MonsterSpec::new(MonsterKind::Toadpole, 26),
            MonsterSpec::new(MonsterKind::Toadpole, 25)
                .slot(1)
                .with_state("artifact", artifact),
        ]
    };
    fn owned<'a>(ids: &[&'a str]) -> std::collections::BTreeSet<&'a str> {
        ids.iter().copied().collect()
    }

    // One relic: no amount of Artifact is ambiguous, because there is nothing
    // to order.
    for artifact in 0..=3 {
        assert!(
            super::turn_one_debuff_artifact_is_unambiguous(
                &owned(&["RELIC.RED_MASK"]),
                &roster(artifact),
            )
            .is_ok(),
            "a single debuff relic is order-free at artifact {artifact}"
        );
    }
    // Two relics: 0 lands both, 2 and 3 block both, and exactly 1 refuses.
    for artifact in [0, 2, 3] {
        assert!(
            super::turn_one_debuff_artifact_is_unambiguous(
                &owned(&["RELIC.RED_MASK", "RELIC.BAG_OF_MARBLES"]),
                &roster(artifact),
            )
            .is_ok(),
            "artifact {artifact} against two relics is order-free"
        );
    }
    let refusal = super::turn_one_debuff_artifact_is_unambiguous(
        &owned(&["RELIC.RED_MASK", "RELIC.BAG_OF_MARBLES"]),
        &roster(1),
    )
    .expect_err("artifact 1 against two relics is ambiguous");
    assert_eq!(refusal.class(), "turn_one_debuff_partial_artifact");
    assert!(refusal.to_string().contains("RELIC.RED_MASK"), "{refusal}");
    assert!(
        refusal.to_string().contains("(1, TOADPOLE, 1)"),
        "the refusal carries the oracle's own (slot, kind, artifact) tuple: \
         {refusal}"
    );
    // The gated third member still counts toward `n`, which is what keeps this
    // exact when `RELIC.TWISTED_FUNNEL` is retired.
    assert!(
        super::TURN_ONE_ALL_ENEMY_DEBUFF_RELICS.contains(&"RELIC.TWISTED_FUNNEL"),
        "the oracle's set is carried whole"
    );
}

/// The #2693 S4b shape, on a save: a Brimstone Strength row that sits **behind
/// Red Mask's Weak token**, because Rust ran the opening.
///
/// This is the residual the 2026-09-22 decision record holds #2637 for. The
/// three census roots carry `weak: 1` (Red Mask, `BeforeSideTurnStart`) beside
/// `strength: 1` (Brimstone, `AfterSideTurnStart`, applier null at `0x32098c`
/// `IL_012d`); natively `Before` strictly precedes `After`, so the order is
/// unique — but a root built through frozen Python's opening cannot record it,
/// because `combat_sim.Monster` has no attachment ledger. A **Rust-built** root
/// records it, which is what "Rust owns the opening" was supposed to buy.
///
/// Measured here rather than only on the corpus. `power_attachments` is only
/// written while `catalog.misery_is_reachable()`, so the case adds one
/// `CARD.MISERY` to the deck — which is also the closure the three census roots
/// reach, there through two Splash copies rather than a physical copy.
///
/// Both directions are asserted: with Red Mask the row is at `after_tokens` 1,
/// and without it at 0. A row that ignored the ledger would pass one and fail
/// the other.
#[test]
fn a_rust_opened_root_records_the_brimstone_strength_behind_red_masks_weak_token() {
    fn misery_case(relics: &[&str]) -> Value {
        let mut case = empty_belt_case();
        deck_row(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.MISERY"}),
        );
        for id in relics {
            relic(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": id}),
            );
        }
        case
    }
    for (relics, after_tokens, python_digest) in [
        (
            &["RELIC.BRIMSTONE"][..],
            0,
            "c05eb1b0d863285b44a975e8fc478ab4d81f6a89d04aa8f98df03ca8f8b486fe",
        ),
        (
            &["RELIC.BRIMSTONE", "RELIC.RED_MASK"][..],
            1,
            "0aede52036d021abd6844665f5a4c3db2a84d6d754855db9de15e675dfbc8e4c",
        ),
    ] {
        let opening = open(&misery_case(relics))
            .unwrap_or_else(|refusal| panic!("the Misery-reachable case opens: {refusal}"));
        assert_eq!(
            monster_field(&opening.document, "power_attachments"),
            vec![
                serde_json::json!([["strength", "none", 0, 1, after_tokens]]),
                serde_json::json!([["strength", "none", 0, 1, after_tokens]])
            ],
            "Brimstone's enemy application passes a literal null applier \
             (`0x32098c` IL_012d), amount 1, behind {after_tokens} token(s)"
        );
        // Digest-equal to Python **except** for the registered Rust-only slot
        // (#2747's decision record: strip registered slots from the advisory
        // comparison). Nothing else may differ, so the stripped document's
        // digest is compared rather than a field-by-field diff.
        let mut stripped = python_view(&opening.document);
        for monster in &mut stripped.monsters {
            monster.remove("power_attachments");
        }
        assert_eq!(
            stripped.differential_digest(),
            python_digest,
            "the only difference from Python may be `power_attachments`, with \
             {relics:?}"
        );
        assert_ne!(
            python_view(&opening.document).differential_digest(),
            python_digest,
            "and the row really is in the unstripped document, with {relics:?}"
        );
    }
}

/// #2908: a Mysterious Knight opening carries **Block 6** and, when `Misery`
/// is reachable, a **`["strength", "monster", 0, 6, 0]`** attachment row that
/// Python's save-rooted document lacks. Both are the IL-correct answer, so
/// this pins them rather than "fixing" them toward the oracle.
///
/// Native authority: v0.111.0 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * **Spawn state.** `MysteriousKnight/<AfterAddedToRoom>d__0::MoveNext`
///   `0x3642a0` awaits the base hook, then `PowerCmd::Apply<StrengthPower>`
///   with amount `6` and the knight's own `Creature` as applier
///   (`IL_0087`-`IL_00a0`), then `Apply<PlatingPower>` with `6`
///   (`IL_00fb`-`IL_0114`). Both engines seed `strength` / `mplating` 6.
/// * **Block 6.** `PlatingPower::BeforeSideTurnStart` `0xa5b9c` continues for
///   `side == Player` (`IL_000c`-`IL_000e`) and a non-player owner
///   (`IL_0017`-`IL_0021`), and while `RoundNumber <= 1`
///   (`IL_0029`-`IL_0031`) calls `CreatureCmd::GainBlock(Owner, Amount,
///   Unpowered, null)` (`IL_0039`-`IL_004d`). So the knight holds Block 6
///   through round one's player turn — the document's checkpoint. Frozen
///   Python grants Plating Block only at the monster's side start for
///   `round_number > 1`; #2809 measured that timing wrong against the
///   Lagavulin capture and made Rust the authority
///   ([`crate::engine::turn`]'s `monster_plating_before_player_side_start`).
/// * **The attachment row.** `power_attachments` is the registered Rust-only
///   slot (`boundary.rs` `// Rust-only:`, `eval_suite.RUST_ONLY_PROVENANCE_SLOTS`,
///   the #2751 decision): `combat_sim.Monster` has no ledger, so Python can
///   never emit it. The row is
///   `damage::materialize_entering_strength_provenance`'s entering-Strength
///   record (#2693 S4), written only while `Misery` is reachable: amount 6 is
///   the native `Apply` amount, position 0 behind no token is forced because
///   nothing is recorded before `AfterAddedToRoom`, and the applier is the
///   knight itself (uid 0, `monster`). The ledger records an entering
///   instance `unknown`, because a mid-fight document cannot say whose it
///   is; the opening can, because its pre-hook roster is exactly what
///   `AfterAddedToRoom` left, so
///   `damage::attribute_spawn_strength_to_its_owner` names the self-applier
///   the IL proves (#3364). Deliberately moved from `unknown` by #3364: a
///   `Misery` copy of the row now carries the knight, which Sleight of Flesh
///   ignores, instead of refusing by name (#2727).
///
/// Both directions are asserted: without `Misery` the only difference from
/// the oracle is `block`; with it, `block` and the row. Stripping exactly
/// those fields gives the oracle's digest, so nothing else differs. The
/// digests were printed by the oracle recipe above
/// [`empty_belt_case`], with `encounter_id` set to
/// `ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER` and, for the second, a
/// `CARD.MISERY` deck row inserted at 0.
#[test]
fn a_mysterious_knight_opens_with_its_round_one_plating_block_and_strength_row() {
    for (misery, python_digest) in [
        (
            false,
            "15286922859b816b5fb8547ce4af3e2634ee1669477861f3e8666e44024bded9",
        ),
        (
            true,
            "6eeaeadaf656e23188ffe3900a8190896faf0de1a8b5b6156a66159abdcf3566",
        ),
    ] {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from("ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER");
        if misery {
            deck_row(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.MISERY"}),
            );
        }
        let opening = open(&case)
            .unwrap_or_else(|refusal| panic!("the Mysterious Knight case opens: {refusal}"));
        assert_eq!(
            monster_field(&opening.document, "kind"),
            vec![Value::from("MYSTERIOUS_KNIGHT")]
        );
        assert_eq!(
            monster_field(&opening.document, "strength"),
            vec![Value::from(6)],
            "`Apply<StrengthPower>` 6 at `0x3642a0` IL_0092"
        );
        assert_eq!(
            monster_field(&opening.document, "mplating"),
            vec![Value::from(6)],
            "`Apply<PlatingPower>` 6 at `0x3642a0` IL_0106"
        );
        assert_eq!(
            monster_field(&opening.document, "block"),
            vec![Value::from(6)],
            "round-one Plating Block at the player side start (`0xa5b9c`), \
             misery {misery}"
        );
        let expected_rows = if misery {
            serde_json::json!([["strength", "monster", 0, 6, 0]])
        } else {
            Value::Null
        };
        assert_eq!(
            monster_field(&opening.document, "power_attachments"),
            vec![expected_rows],
            "the entering-Strength row exists exactly when Misery is reachable"
        );

        let mut stripped = python_view(&opening.document);
        for monster in &mut stripped.monsters {
            monster.remove("power_attachments");
            monster.remove("block");
        }
        assert_eq!(
            stripped.differential_digest(),
            python_digest,
            "the only differences from Python are `block` and the Rust-only \
             slot, misery {misery}"
        );
        assert_ne!(
            python_view(&opening.document).differential_digest(),
            python_digest,
            "and the Block really is in the unstripped document, misery {misery}"
        );
    }
}

/// #2693 S4b criterion 5, on a **Rust-opened** root: a loss wrapper drives the
/// Brimstone Strength negative and `Misery` then reads it, with nothing
/// refusing anywhere along the line.
///
/// This is the line the whole #2637 hold was about. On a Python-built root the
/// entering `strength: 1` sits beside Red Mask's `weak` token with no recorded
/// order, so a `Crush Under` taking it negative left it *unrecorded* and the
/// following `Misery` refused `Misery concrete Type-2 power state` inside an
/// admitted root. Rust's opening records the row
/// (`a_rust_opened_root_records_the_brimstone_strength_behind_red_masks_weak_token`
/// above), so the wrapper's `ModifyAmount` keeps it in step and the snapshot
/// is exact.
///
/// **Two** copies of `Crush Under`, for an arithmetic reason worth stating so
/// the shape is not mistaken for padding: its wrapper is **1**
/// (`content_tables`, `AttackAllTempStrengthSnapshot(8, 1)`), so the first copy
/// takes the entering `strength: 1` to exactly zero — where
/// `PowerModel::ShouldRemoveDueToAmount` `0x83b0d` IL_0012-IL_0023 **removes**
/// the instance rather than making it Type-2 — and only the second drives it
/// below zero, on a row re-appended behind the wrapper that erased the first.
/// That is a strictly harder line than a single big wrapper, and it is the
/// removal-then-reattach edge `write_monster_strength` owes. (`Dying Star`, the
/// other producer the three corpus roots reach, would do it in one play but
/// costs 3 stars, which this Ironclad save has none of.)
///
/// The corpus roots reach it through two `Splash` copies; this case puts the
/// card in the deck directly, because what criterion 5 asks about is the
/// *line*, not the generation closure, and the generation closure is already
/// what `admission::monster_strength_can_be_driven_negative` rides. Stated
/// rather than hidden: this is a constructed line, not a recorded one.
#[test]
fn a_rust_opened_root_replays_a_wrapper_then_misery_without_refusing() {
    let mut case = empty_belt_case();
    for id in ["CARD.MISERY", "CARD.CRUSH_UNDER", "CARD.CRUSH_UNDER"] {
        deck_row(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    for id in ["RELIC.BRIMSTONE", "RELIC.RED_MASK"] {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id}),
        );
    }
    let opening = open(&case).expect("the criterion-5 case opens under Rust's opening");
    assert_eq!(
        monster_field(&opening.document, "power_attachments"),
        vec![
            serde_json::json!([["strength", "none", 0, 1, 1]]),
            serde_json::json!([["strength", "none", 0, 1, 1]])
        ],
        "the entering Brimstone Strength is recorded behind Red Mask's token",
    );

    // Rust's opening decides the ROW; the shuffle decides which pile each card
    // landed in, and that is not what criterion 5 is about. Both cards are
    // moved into hand so the line is deterministic, leaving every other field
    // of the Rust-built document — `power_attachments` included — untouched.
    let mut document = opening.document.clone();
    let mut piles: Value = serde_json::to_value(&document.piles).expect("the piles serialize");
    let wanted = ["CRUSH_UNDER", "MISERY"];
    let mut moved: Vec<Value> = Vec::new();
    for name in ["hand", "draw", "discard"] {
        let Some(pile) = piles.get_mut(name).and_then(Value::as_array_mut) else {
            continue;
        };
        let mut kept = Vec::new();
        for card in pile.drain(..) {
            if card
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| wanted.contains(&id))
            {
                moved.push(card);
            } else {
                kept.push(card);
            }
        }
        *pile = kept;
    }
    assert_eq!(
        moved.len(),
        3,
        "two Crush Under and one Misery, found once each"
    );
    let uids_of = |id: &str| {
        moved
            .iter()
            .filter(|card| card.get("id").and_then(Value::as_str) == Some(id))
            .map(|card| {
                card.get("uid")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| panic!("{id} carries a uid")) as u32
            })
            .collect::<Vec<_>>()
    };
    let wrappers = uids_of("CRUSH_UNDER");
    assert_eq!(wrappers.len(), 2);
    let misery = uids_of("MISERY")[0];
    piles["hand"] = Value::Array(moved);
    document.piles = serde_json::from_value(piles).expect("the piles round-trip");

    let document = &document;
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document)
        .expect("the Rust-opened root builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(document, &catalog)
        .expect("the Rust-opened root hydrates");
    // The #2693 S4b gate does NOT fire: the Strength is recorded, so there is
    // no unplaceable position for a reducer to threaten.
    crate::engine::admit(document, &state, &catalog)
        .expect("a Rust-built root admits with a reducer and a Misery reachable");

    let play = |from: &crate::hot::HotState, card: u32, target: Option<u8>| {
        crate::engine::apply_action_into(
            from,
            &catalog,
            &crate::engine::Action::Play {
                uid: card,
                target,
                selection: crate::engine::SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
    };

    // `CRUSH_UNDER` is `targeted: false` / `AllEnemies`; `MISERY` is
    // `targeted: true` / `AnyEnemy` (`content_tables`).
    let once = play(&state, wrappers[0], None).expect("the first Crush Under plays");
    assert_eq!(
        once.monsters[0].powers.value(crate::ids::PowerId::Strength),
        0,
        "wrapper 1 takes the entering Strength to exactly zero",
    );
    assert!(
        crate::engine::monsters::strength_attachment(&once.monsters[0]).is_none(),
        "`ShouldRemoveDueToAmount` `0x83b0d` erases the instance at exactly \
         zero, so the row goes with it",
    );
    let wrapped = play(&once, wrappers[1], None).expect("the second Crush Under plays");
    assert_eq!(
        wrapped.monsters[0]
            .powers
            .value(crate::ids::PowerId::Strength),
        -1,
        "wrapper 2 drives it below zero, which is what makes it \
         `Misery`-selectable (`0x83a94` IL_0028-IL_003e)",
    );
    assert!(
        crate::engine::monsters::strength_attachment(&wrapped.monsters[0]).is_some(),
        "and the re-application APPENDS a fresh row, so the provenance is \
         recorded again rather than left unplaceable",
    );
    let after = play(&wrapped, misery, Some(0)).expect(
        "#2693 S4b: the frozen row makes `Misery`'s snapshot exact, so the \
         play that used to refuse inside an admitted root now replays",
    );
    assert!(!after.history.over || after.hp > 0);
}

#[test]
fn the_mcr_splice_is_a_step_and_not_a_default() {
    use crate::entry::counters::COMBAT_STREAMS;
    use crate::entry::opening::mcr::{McrOpeningChecksum, OPENING_CHECKSUM_CONTEXT};

    let mut rng = serde_json::Map::new();
    for (_save, canonical) in COMBAT_STREAMS {
        rng.insert(
            canonical.to_string(),
            serde_json::json!({"counter": 41, "words": [5, 6, 7, 8]}),
        );
    }
    let checksum = McrOpeningChecksum::from_first_checksum(&serde_json::json!({
        "context": OPENING_CHECKSUM_CONTEXT,
        "full_state": {"rng": Value::Object(rng)},
    }))
    .expect("the synthetic checksum parses");

    // The splice acts on the post-opening document, so on this fixture it is
    // reached only if the boundary admits — which it does not. What is pinned
    // here is the contract itself: the checksum parses, applying it overwrites
    // all nine streams, and the pre-splice reading survives (§A5 obligation 3).
    let mut streams = std::collections::BTreeMap::new();
    streams.insert(
        "rng".to_string(),
        crate::canonical::CanonicalRngV2 {
            counter: 9,
            words: [1, 2, 3, 4],
        },
    );
    let before = streams.clone();
    checksum.splice_into(&mut streams);
    assert_eq!(streams["rng"].counter, 41);
    assert_eq!(before["rng"].counter, 9);
    assert_eq!(streams.len(), COMBAT_STREAMS.len());
}

// ---------------------------------------------------------------------------
// The held potion identity (#2770)
// ---------------------------------------------------------------------------
//
// The belt is the one surface where the opening spoke a *different name space*
// from the boundary: `entry::belt` carries full `POTION.` model ids (the save's
// own `SerializablePotion.Id`), while `State.potion_slots`, `State.potions` and
// `KNOWN_POTIONS` all speak the bare identity, because
// `_potion_slots_from_entry` strips the prefix (frozen Python `unmodeled_potion_caveat`, deleted #2827) and
// `KNOWN_POTIONS = set(_REGISTRY.potions)` (`_route_card_play_result`) is a set of bare names.
// `pre_hook_document` stripped it on the dense `potions` mirror and not on the
// authoritative sparse `potion_slots`, so `PotionId::from_str` was handed
// `"POTION.WEAK_POTION"`, found no variant, and `hydrate_potion_belt` refused
// the belt as an *"unknown/inert potion identity"*.
//
// Why no existing witness caught it: the fixture save's
// `unlock_state.unlocked_epochs` is empty, so every belt it can materialise is
// refused one gate earlier on `fully_unlocked_potion_pool` — the ceiling
// `the_post_deal_document_stops_at_the_existing_potion_belt_boundary` pins.
// **No case in the tree had both a belt and a proven pool**, which is exactly
// the combination the corpus's three #2693 residual roots have. These cases
// add the three epochs `live_coach.potion_pool_fully_unlocked` requires
// (`entry::unlocks::FULLY_UNLOCKED_POTION_POOL_FLAG_EPOCHS`) so the belt survives to the
// identity decode.
//
// Every digest below was printed by the oracle with the recipe in the comment
// block above `empty_belt_case`, plus the belt/epoch mutation each test names,
// in the same run that reproduced `EMPTY_BELT_DIGEST` — so these are
// comparisons against Python, not transcriptions of Rust's output.

/// The three epochs that prove the solo-Ironclad potion pool, in the save's
/// own `EPOCH.`-prefixed spelling.
const POOL_EPOCHS: [&str; 3] = [
    "EPOCH.IRONCLAD4_EPOCH",
    "EPOCH.POTION1_EPOCH",
    "EPOCH.POTION2_EPOCH",
];

/// The fixture case with a two-slot belt, a proven potion pool, and the given
/// `(model_id, slot_index)` rows — the shape the corpus's residual roots have.
fn belt_case(rows: &[(&str, i64)]) -> Value {
    let mut case = case();
    case["save"]["players"][0]["max_potion_slot_count"] = Value::from(2);
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
        POOL_EPOCHS
            .iter()
            .map(|epoch| Value::from(*epoch))
            .collect(),
    );
    case["save"]["players"][0]["potions"] = Value::Array(
        rows.iter()
            .map(|(id, index)| serde_json::json!({"id": *id, "slot_index": *index}))
            .collect(),
    );
    case
}

/// The control: a proven pool and an **empty** two-slot belt. It opens on
/// `origin/main` too, which is what makes it a control — it isolates the
/// epochs (`potion_slots: [null, null]` plus the flag) from the identity
/// decode the three cases below add.
#[test]
fn a_proven_pool_opens_an_empty_materialised_belt() {
    let opening = open(&belt_case(&[])).expect("the proven-pool empty belt opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "573d5e9486dc47d8a8ff0e6a44f0ed054447c2d90802c86e54856cf30c8c8ba5"
    );
    assert_eq!(
        flag(&opening.document, "potion_slots"),
        Some(serde_json::json!([null, null]))
    );
    assert_eq!(
        flag(&opening.document, "fully_unlocked_potion_pool"),
        Some(Value::from(true))
    );
    // The dense mirror is absent, not empty: `State.potions` defaults to `()`
    // and an at-default field is elided.
    assert_eq!(flag(&opening.document, "potions"), None);
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
}

/// A held identity opens digest-equal to Python, and the **bare** name is what
/// both belt mirrors carry.
///
/// Three rows, because a single occupied slot cannot separate the two mirrors:
/// with the potion at index 0 the sparse slots and the dense list are the same
/// sequence, so only the index-1 case witnesses that the sparse topology keeps
/// its hole while the dense mirror does not — the cross-field check
/// `hydrate_potion_belt` makes (*"dense mirror must equal the sparse slots'
/// ordered non-null identities"*), which was unreachable behind the identity
/// refusal.
#[test]
fn a_held_potion_identity_opens_digest_equal_to_the_oracle() {
    for (rows, digest, slots, dense) in [
        (
            &[("POTION.WEAK_POTION", 0)][..],
            "82062dc6e4e37c61d4450a6d272b0beae27909fc4e4fdb34bfa27b73b8ecd29b",
            serde_json::json!(["WEAK_POTION", null]),
            serde_json::json!(["WEAK_POTION"]),
        ),
        (
            &[("POTION.WEAK_POTION", 1)][..],
            "6ca00bd44c07ce8fc75e4391c0f1ee5250d5c4175eb5922d38617d6aa49f6696",
            serde_json::json!([null, "WEAK_POTION"]),
            serde_json::json!(["WEAK_POTION"]),
        ),
        (
            &[("POTION.FIRE_POTION", 0), ("POTION.BEETLE_JUICE", 1)][..],
            "4be62575b4704e1f8b74c34e10035dad70d47f8eb8427a793694489d83e115d6",
            serde_json::json!(["FIRE_POTION", "BEETLE_JUICE"]),
            serde_json::json!(["FIRE_POTION", "BEETLE_JUICE"]),
        ),
    ] {
        let opening = open(&belt_case(rows))
            .unwrap_or_else(|refusal| panic!("the belt {rows:?} opens, got {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "belt {rows:?} must be Python's document"
        );
        // The field assertions are what make each digest non-vacuous: a belt
        // whose identity had been dropped entirely would also differ from the
        // control's digest.
        assert_eq!(
            flag(&opening.document, "potion_slots"),
            Some(slots),
            "belt {rows:?} sparse slots"
        );
        assert_eq!(
            flag(&opening.document, "potions"),
            Some(dense),
            "belt {rows:?} dense mirror"
        );
    }
}

/// The opened belt survives a **cold reload**: hydrating the document the
/// opening emitted and re-projecting the engine's own state returns the same
/// document.
///
/// This is the half a digest comparison cannot see. `pre_hook_document` writes
/// the belt; `hydrate_potion_belt` decodes it into `HotState.fanouts`; the
/// projection re-emits it from `PlayerSlot::PotionSlots`/`Potions`. A strip
/// applied on only one of those three edges would still produce a document
/// equal to Python's and then fail to round-trip — the same class of asymmetry
/// as the defect itself.
#[test]
fn the_opened_belt_round_trips_through_the_boundary() {
    let opening = open(&belt_case(&[
        ("POTION.FIRE_POTION", 0),
        ("POTION.BEETLE_JUICE", 1),
    ]))
    .expect("the two-potion belt opens");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .expect("the opened document builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
        .expect("the opened document hydrates");
    let reprojected = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog)
        .expect("the hydrated state re-projects");
    assert_eq!(
        reprojected.player.get("potion_slots"),
        Some(&serde_json::json!(["FIRE_POTION", "BEETLE_JUICE"])),
        "the hydrated belt is the typed identity, in slot order"
    );
    assert_eq!(
        reprojected.differential_digest(),
        python_view(&opening.document).differential_digest(),
        "the cold reload must be the state the engine built"
    );
}

/// The fail-closed guard is **preserved**, not widened: an identity the crate
/// genuinely has no variant for still refuses, and the refusal detail now
/// describes only that case.
///
/// This case is the one where Rust and Python deliberately differ, and it is a
/// named narrowing rather than a divergence: `combat_sim` *roots* it, carrying
/// `potion_slots: ["NOT_A_POTION", null]` and `inert_potions:
/// ["NOT_A_POTION"]` (oracle digest
/// `1b4455059a65cfb567749bab74d17cd7e4dd72c5f95dc1a54fb33ee3f5687141`), on the
/// #634 argument that an unmodeled potion nobody drinks is exactly an
/// undrinkable item. This crate has no `inert_potions` counterpart and refuses
/// instead — the standing *"Part A refuses every unknown/inert potion
/// identity"* narrowing — so Rust's admitted set stays a subset of Python's
/// here. Measured unreachable from the capture corpus: all 54 distinct
/// identities held across 2,342 occupied belts in 3,387 saves are in
/// `PotionId::NAMES`, so no corpus root turns on it either way.
///
/// It therefore also fails on `origin/main` — for the *wrong reason*, which is
/// why it is carried: before the strip, every identity took this arm.
#[test]
fn an_identity_outside_the_vocabulary_still_refuses_by_name() {
    let refusal = open(&belt_case(&[("POTION.NOT_A_POTION", 0)]))
        .expect_err("an unknown potion identity is refused");
    assert_eq!(refusal.class(), "boundary_unrepresentable");
    let text = refusal.to_string();
    assert!(text.contains("potion_slots"), "{text}");
    assert!(
        text.contains("unknown/inert potion identity is not represented"),
        "{text}"
    );
    // And a known identity beside it in the same belt does not rescue it — the
    // decode is per identity, not per belt.
    let mixed = open(&belt_case(&[
        ("POTION.FIRE_POTION", 0),
        ("POTION.NOT_A_POTION", 1),
    ]))
    .expect_err("a mixed belt refuses on the unknown member");
    assert!(
        mixed.to_string().contains("unknown/inert potion identity"),
        "{mixed}"
    );
}

// ---------------------------------------------------------------------------
// #2539: the save's ascension selects the roster's tier
// ---------------------------------------------------------------------------

/// The oracle's save-rooted digests for [`empty_belt_case`] at A7 and A8, from
/// the documented `mcr_replay.start_from_save` command with
/// `case['save']['ascension'] = 7` / `8` (and `max_potion_slot_count = 0`).
/// At A7 the two Toadpoles roll from `GetValueIfAscension(8, 22..26, 21..25)`
/// (25 and 24); at A8 they are the A10 bytes' 26 and 25 plus
/// `player.ascension: 8`. At 10 the digest is [`EMPTY_BELT_DIGEST`], i.e. the
/// fixture's bytes are unchanged by #2539.
const A7_EMPTY_BELT_DIGEST: &str =
    "ed79d98872d9145cdfadfc224e36aa9362d926b859e8691f45301e28ca534aa4";
const A8_EMPTY_BELT_DIGEST: &str =
    "3f88ba5313fd57f0a13bcfc4ad53ba49ba2bc5f52f7fe5822ea012f52433da2b";

fn at_ascension(ascension: Value) -> Value {
    let mut case = empty_belt_case();
    case["save"]["ascension"] = ascension;
    case
}

#[test]
fn the_opening_rolls_the_saves_ascension_tier_digest_equal_to_the_oracle() {
    for (ascension, digest, hps) in [
        (7, A7_EMPTY_BELT_DIGEST, [25, 24]),
        (8, A8_EMPTY_BELT_DIGEST, [26, 25]),
    ] {
        let opening = open(&at_ascension(Value::from(ascension))).expect("the case opens");
        assert_eq!(
            opening.document.player.get("ascension"),
            Some(&Value::from(ascension))
        );
        let rolled: Vec<i64> = opening
            .document
            .monsters
            .iter()
            .map(|monster| monster["max_hp"].as_i64().unwrap())
            .collect();
        assert_eq!(rolled, hps, "A{ascension}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "A{ascension}"
        );
    }
    let modeled = open(&at_ascension(Value::from(10))).expect("the case opens");
    assert!(!modeled.document.player.contains_key("ascension"));
    assert_eq!(modeled.document.differential_digest(), EMPTY_BELT_DIGEST);
}

#[test]
fn an_absent_or_out_of_range_ascension_refuses_by_name() {
    let mut absent = case();
    absent["save"]
        .as_object_mut()
        .expect("the save is an object")
        .remove("ascension");
    assert_eq!(
        pre_hook(&absent).map(|_| ()),
        Err(OpeningRefusal::AscensionNotExact { recorded: None })
    );
    assert_eq!(
        pre_hook(&at_ascension(Value::from(11))).map(|_| ()),
        Err(OpeningRefusal::AscensionNotExact { recorded: Some(11) })
    );
    assert_eq!(
        pre_hook(&at_ascension(Value::from(-1))).map(|_| ()),
        Err(OpeningRefusal::AscensionNotExact { recorded: Some(-1) })
    );
}

/// Cross-lane witness for #2829 (Vajra, #2827) and #2828 (tiered move rows).
///
/// A Vajra save opened at A8 and at A10. The opening's catalog is built from
/// the pre-hook document, which carries `player.ascension`, so its monster
/// move rows compile at the fight's tier: `Toadpole::get_WhirlDamage` RVA
/// `0xc2722` is `GetValueIfAscension(9, 8, 7)`. Vajra's one Strength
/// (`<AfterRoomEntered>d__6` `0x333abc` `IL_0062`) is the **player's**, so it
/// never reaches a monster's hit. A monster's own Strength does: it adds to
/// the tiered base in `monster_attack_hit_damage`, which the intents query and
/// the executed enemy phase share, so the priced intent is what is dealt.
#[test]
fn vajra_strength_composes_with_the_tiered_monster_rows() {
    use crate::ids::{MonsterKind, PowerId};
    let row = |name: &str| {
        i32::try_from(
            crate::content_tables::monster_loop(MonsterKind::Toadpole)
                .unwrap()
                .iter()
                .position(|row| row.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    for (ascension, whirl) in [(8_i64, 7_i64), (10, 8)] {
        let mut case = with_relic("RELIC.VAJRA");
        case["save"]["ascension"] = Value::from(ascension);
        let opening = open(&case).expect("the Vajra case opens");
        assert_eq!(
            opening.document.player.get("strength"),
            Some(&Value::from(1))
        );
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
            .expect("the opened document loads");
        assert_eq!(i64::from(catalog.ascension()), ascension);
        let mut state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
            .expect("the opened document decodes");

        // One Toadpole on WHIRL, the other on SPIKEN (no attack), so every
        // point of damage the enemy phase deals is the one priced WHIRL.
        let (whirl_row, spiken_row) = (row("WHIRL"), row("SPIKEN"));
        for (index, monster) in state.monsters_mut().iter_mut().enumerate() {
            assert_eq!(monster.kind, MonsterKind::Toadpole);
            monster.loop_pos = if index == 0 { whirl_row } else { spiken_row };
        }
        let whirl_intent = |state: &crate::hot::HotState| {
            crate::engine::turn::monster_intents(state, &catalog)
                .into_iter()
                .find(|intent| intent.intent == Some("WHIRL"))
                .expect("WHIRL is named")
        };
        let priced = whirl_intent(&state);
        assert_eq!(
            (priced.damage, priced.hits),
            (Some(whirl), Some(1)),
            "A{ascension}: the player's Vajra Strength leaves the monster hit alone"
        );

        // The same opening with the WHIRL Toadpole carrying Strength 2, loaded
        // through the boundary like any document.
        let mut strong = opening.document.clone();
        strong.monsters[0].insert("strength".to_owned(), Value::from(2));
        let mut state = crate::boundary::HotBoundary::from_canonical(&strong, &catalog)
            .expect("the Strength document decodes");
        for (index, monster) in state.monsters_mut().iter_mut().enumerate() {
            monster.loop_pos = if index == 0 { whirl_row } else { spiken_row };
        }
        assert_eq!(whirl_intent(&state).damage, Some(whirl + 2), "A{ascension}");
        state.block = 0;
        let hp = state.hp;
        let next = crate::engine::apply_action(&state, &catalog, &crate::engine::Action::EndTurn)
            .expect("the enemy phase runs");
        assert_eq!(i64::from(hp - next.state.hp), whirl + 2, "A{ascension}");
        assert_eq!(next.state.powers.value(PowerId::Strength), 1);
    }
}

// ---------------------------------------------------------------------------
// Eternal Feather and Ghost Seed (#2827)
// ---------------------------------------------------------------------------
//
// Both left `OPENING_WINDOW_RELIC_BODIES` on 2026-09-23. Every expected digest
// below was printed by the oracle with the recipe in the comment block above
// `empty_belt_case`, appending `{'floor_added_to_deck': 1, 'id': <relic>}` to
// `save['players'][0]['relics']` per relic in the order named, and the
// fixture's own `EMPTY_BELT_DIGEST` was reproduced by the same script in the
// same run.

/// Every Basic Strike and Defend in the opened document, and whether each
/// carries the local Ethereal keyword, in hand-then-draw order.
fn ethereal_basics(document: &crate::canonical::CanonicalStateV2) -> Vec<(String, bool)> {
    ["hand", "draw"]
        .into_iter()
        .flat_map(|pile| document.piles[pile].iter())
        .filter(|card| card.id.starts_with("STRIKE_") || card.id.starts_with("DEFEND_"))
        .map(|card| {
            (
                card.id.clone(),
                card.local_keywords
                    .iter()
                    .any(|keyword| keyword == "Ethereal"),
            )
        })
        .collect()
}

/// `RELIC.ETERNAL_FEATHER` opens and changes nothing, because its heal is
/// rest-site only in IL.
///
/// `EternalFeather/<AfterRoomEntered>d__4::MoveNext` (`0x323ea8`) `leave`s at
/// `IL_002d` unless `room isinst RestSiteRoom` (`IL_0026`), so in a combat room
/// it only returns. The oracle runs no body for it. As with Meal Ticket, the
/// digest is **Python's** and is **not** the control's: the relic row is the
/// whole difference.
#[test]
fn an_eternal_feather_save_opens_because_its_heal_is_rest_site_only() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.ETERNAL_FEATHER")).expect("Eternal Feather opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "ff6f32b73e331b9752ed2229c64ff3f46ba12feb57240bc8103d69c0f39fd0a8"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );
    assert_eq!(
        opening.document.player.get("hp"),
        control.document.player.get("hp")
    );
    assert_eq!(opening.document.piles, control.document.piles);
    assert_eq!(opening.document.rng, control.document.rng);
}

/// `RELIC.GHOST_SEED` marks every Basic Strike and Defend Ethereal on
/// entering a combat room, and nothing else.
///
/// `GhostSeed::AfterRoomEntered` (`0x9453c`) walks `AllCards` and applies
/// Ethereal to each card `CanAffect` (`0x945bc`) accepts. The deck is five
/// Strikes, four Defends and a Bash: nine marked, Bash untouched. The shuffle,
/// the deal and every stream are the control's, because the body reads no RNG,
/// and `exact_piles` stays unset because every copy of each group is marked
/// alike (the oracle's document carries no `exact_piles` either).
#[test]
fn a_ghost_seed_save_marks_every_basic_strike_and_defend_ethereal() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.GHOST_SEED")).expect("Ghost Seed opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "b9b46ba806931ff5e78ff8140791ee7b0c4a64660f5c60e623ecde2122ce1c53"
    );
    let marked = ethereal_basics(&opening.document);
    assert_eq!(marked.len(), 9);
    assert!(marked.iter().all(|(_, ethereal)| *ethereal), "{marked:?}");
    assert!(
        ethereal_basics(&control.document)
            .iter()
            .all(|(_, ethereal)| !*ethereal)
    );
    assert_eq!(
        card_named(&opening.document, "BASH"),
        card_named(&control.document, "BASH")
    );
    assert_eq!(opening.document.player.get("exact_piles"), None);
    assert_eq!(opening.document.rng, control.document.rng);
    assert_eq!(hand_size(&opening.document), 5);
}

/// Ghost Seed beside Eternal Feather, in both acquisition orders, and beside
/// Vajra, whose room-entry Strength runs before the template walk Ghost Seed
/// follows.
///
/// The two Feather orders are different documents only because
/// `relics_entering` is projected as a list; both digests are the oracle's.
#[test]
fn ghost_seed_opens_beside_eternal_feather_and_vajra() {
    for (relics, digest) in [
        (
            ["RELIC.GHOST_SEED", "RELIC.ETERNAL_FEATHER"],
            "d678b0c1ffbcd3e4d679e4549247c9fe29ae63885c95388e8ba4303caf956055",
        ),
        (
            ["RELIC.ETERNAL_FEATHER", "RELIC.GHOST_SEED"],
            "812ad5f9ae52949b43f2ef74633bffd6edef20c765a54e8df8101f13614de9eb",
        ),
        (
            ["RELIC.GHOST_SEED", "RELIC.VAJRA"],
            "29c6c83e3da9dc4132b1d787a6d87deed5372b6ccb16927333729b9d1da46121",
        ),
    ] {
        let mut case = with_relic(relics[0]);
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": relics[1]}),
        );
        let opening = open(&case).unwrap_or_else(|err| panic!("{relics:?} opens: {err}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relics:?}"
        );
        assert!(
            ethereal_basics(&opening.document)
                .iter()
                .all(|(_, ethereal)| *ethereal),
            "{relics:?}"
        );
    }
}

/// An upgraded Strike is still Basic and still tagged Strike, so it is marked
/// too: `CanAffect` reads rarity and tags, never the upgrade level.
///
/// Mutation: `save['players'][0]['deck'].insert(0, {'floor_added_to_deck': 1,
/// 'id': 'CARD.STRIKE_IRONCLAD', 'current_upgrade_level': 1})`, then Ghost Seed
/// appended.
#[test]
fn ghost_seed_marks_an_upgraded_strike() {
    let mut case = with_relic("RELIC.GHOST_SEED");
    deck_row(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "CARD.STRIKE_IRONCLAD",
            "current_upgrade_level": 1}),
    );
    let opening = open(&case).expect("an upgraded-Strike Ghost Seed save opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "011fa969c47d6cf8a2f84d3e27af055998bdc2213fd57efc502098dce7e3b3fe"
    );
    let upgraded: Vec<&crate::canonical::CanonicalCardV2> = opening
        .document
        .piles
        .values()
        .flatten()
        .filter(|card| card.id == "STRIKE_IRONCLAD" && card.upgrade == 1)
        .collect();
    assert_eq!(upgraded.len(), 1);
    assert_eq!(upgraded[0].local_keywords, vec!["Ethereal".to_owned()]);
    assert_eq!(ethereal_basics(&opening.document).len(), 10);
}

/// An enchanted Strike is marked too, and the document is the oracle's.
///
/// Mutation: `save['players'][0]['deck'].insert(0, {'floor_added_to_deck': 1,
/// 'id': 'CARD.STRIKE_IRONCLAD', 'enchantment': {'id': 'ENCHANTMENT.SHARP',
/// 'amount': 2}})`, with and without Ghost Seed appended. The Sharp copy makes
/// the `(STRIKE_IRONCLAD, 0)` group mixed-payload, so the oracle's
/// `start_combat` sets `exact_piles` (frozen Python, deleted #2827), and since
/// #2885 so does the opening. Ghost Seed does not change that: its own effect
/// (the Sharp Strike carries Ethereal beside its enchantment, which
/// `CanAffect` never reads) is exact in both documents.
#[test]
fn ghost_seed_marks_an_enchanted_strike_in_a_mixed_payload_group() {
    for (with_seed, oracle) in [
        (
            false,
            "e650eb795ec97950c9f477c9b62be51dad3e4379253d2125e7ca58fc082485d1",
        ),
        (
            true,
            "952bb23b0b32510960e7e6cc677bf0bc09ccb2c16cb8d5d4496fe7e564c3822f",
        ),
    ] {
        let mut case = if with_seed {
            with_relic("RELIC.GHOST_SEED")
        } else {
            empty_belt_case()
        };
        deck_row(
            &mut case,
            serde_json::json!({
                "floor_added_to_deck": 1, "id": "CARD.STRIKE_IRONCLAD",
                "enchantment": {"id": "ENCHANTMENT.SHARP", "amount": 2}}),
        );
        let opening = open(&case).expect("a Sharp Strike save opens");
        let sharp: Vec<&crate::canonical::CanonicalCardV2> = opening
            .document
            .piles
            .values()
            .flatten()
            .filter(|card| card.enchantment.is_some())
            .collect();
        assert_eq!(sharp.len(), 1);
        assert_eq!(
            sharp[0].local_keywords.iter().any(|k| k == "Ethereal"),
            with_seed
        );
        assert_eq!(
            opening.document.player.get("exact_piles"),
            Some(&Value::Bool(true)),
            "Ghost Seed {with_seed}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            oracle,
            "Ghost Seed {with_seed}"
        );
    }
}

// ---------------------------------------------------------------------------
// `exact_piles` at combat start (#2885)
// ---------------------------------------------------------------------------
//
// Every digest below was printed by the oracle with the recipe above
// `empty_belt_case`, inserting the named deck rows at the front of
// `save['players'][0]['deck']` (a Strike row is `{'floor_added_to_deck': 1,
// 'id': 'CARD.STRIKE_IRONCLAD', 'current_upgrade_level': <up>,
// 'enchantment': {'id': 'ENCHANTMENT.<id>', 'amount': <n>}}`). The same run
// printed whether the oracle's document carries `exact_piles`, which is the
// third column. The fixture deck already holds five plain `(STRIKE_IRONCLAD,
// 0)` copies, so every upgraded-Strike row forms its own `(id, 1)` group and
// cannot be mixed with them — that separation is itself one of the rows.

fn strike_row(upgrade: i64, enchantment: &str, amount: i64) -> Value {
    serde_json::json!({
        "floor_added_to_deck": 1, "id": "CARD.STRIKE_IRONCLAD",
        "current_upgrade_level": upgrade,
        "enchantment": {"id": format!("ENCHANTMENT.{enchantment}"), "amount": amount}})
}

fn genetic_algorithm_row() -> Value {
    serde_json::json!({
        "floor_added_to_deck": 1, "id": "CARD.GENETIC_ALGORITHM",
        "props": {"ints": [
            {"name": "CurrentBlock", "value": 1},
            {"name": "IncreasedBlock", "value": 0}]}})
}

/// The oracle's grouping rule, one row per arm, each on both sides of the
/// line.
///
/// * **Distinct payloads.** Sharp 2 beside Sharp 3 in one `(id, upgrade)`
///   group is exact; two identical Sharp 2 copies are not, and neither is one
///   Sharp copy at an upgrade level no plain copy shares (the grouping key is
///   `(id, upgrade)`, never the id alone).
/// * **Divergent-identity enchantments.** Two identical copies of seven of
///   the eight classes are exact, because the payload flips after one copy
///   plays; a single Glam, Momentum or Vigorous copy is not (a group of one
///   has no sibling to diverge from). Goopy is held out: its `CanEnchant`
///   needs a Defend, so it cannot go on a Strike. Momentum and Vigorous rows
///   also carry their seeded `enchantment_state` (#2891, pinned row by row in
///   `mutable_enchantments_enter_with_their_zero_slot_five_state`). A single
///   Slither copy is not a deck-rule case at all: a dealt copy is promoted
///   after the deal by its draw-time cost roll (#2926, pinned in
///   `one_slither_copy_dealt_in_the_opening_rolls_its_cost`).
/// * **Divergent physical cards.** Two fresh Genetic Algorithm copies are
///   exact; one is not.
#[test]
fn exact_piles_follows_the_oracles_deck_group_rule() {
    let cases: Vec<(&str, Vec<Value>, &str, bool)> = vec![
        (
            "Sharp 2 beside Sharp 3",
            vec![strike_row(1, "SHARP", 2), strike_row(1, "SHARP", 3)],
            "7ad1a9d02db7a240bb39bbbe23b2e0f7b17dd63b434a473b234c6a5b5d37f6b2",
            true,
        ),
        (
            "two identical Sharp 2",
            vec![strike_row(1, "SHARP", 2), strike_row(1, "SHARP", 2)],
            "4d9942285f7843bb7d1907c04faf551f5896723d9ae620dceb24967a8409156e",
            false,
        ),
        (
            "one Sharp at an unshared upgrade",
            vec![strike_row(1, "SHARP", 2)],
            "c6d0de9cdfe4029ef036f867b8c32a283862828e8546f4feffb516d4d2975e47",
            false,
        ),
        (
            "two identical Glam",
            vec![strike_row(1, "GLAM", 1), strike_row(1, "GLAM", 1)],
            "7e19ce9040b4bb89ec489c06486d4dbbd27b3ea52996bc7f6f3969c462348cb0",
            true,
        ),
        (
            "one Glam",
            vec![strike_row(1, "GLAM", 1)],
            "3256df6ec11f2ad762a0adaf3b12ba904d92884a7d3bf39b6016e5580d1774b2",
            false,
        ),
        (
            "two identical Swift",
            vec![strike_row(1, "SWIFT", 1), strike_row(1, "SWIFT", 1)],
            "5c48cbab27eb23fa2163314ad876c2e89f990583b27eabf8711e765eba788114",
            true,
        ),
        (
            "two identical Sown",
            vec![strike_row(1, "SOWN", 1), strike_row(1, "SOWN", 1)],
            "2c18ff4cbf75c3eadfab9d76e7308c36b5d3bfe4b95f747c04c71052a667bbc5",
            true,
        ),
        (
            "two identical Momentum",
            vec![strike_row(1, "MOMENTUM", 2), strike_row(1, "MOMENTUM", 2)],
            "2d2c0552ac7b5c28723181a845485d56a5f2b16895d618d7f901ed946a614063",
            true,
        ),
        (
            "one Momentum",
            vec![strike_row(1, "MOMENTUM", 2)],
            "2bf6221ca6e86be98fd397f17d6939a0ce24347f276cee8c174a30a078fa65df",
            false,
        ),
        (
            "two identical Vigorous",
            vec![strike_row(1, "VIGOROUS", 3), strike_row(1, "VIGOROUS", 3)],
            "8d13bf13b1441005eba8ba130dc4040d8add469fcbb831fb695ff7adb2d29e54",
            true,
        ),
        (
            "one Vigorous",
            vec![strike_row(1, "VIGOROUS", 3)],
            "626fa89a3cea412a656cea188375e91e1adfc97db4993424bfa4e355607fb07d",
            false,
        ),
        (
            "two identical Slither",
            vec![strike_row(1, "SLITHER", 1), strike_row(1, "SLITHER", 1)],
            "66a4ce4e753111b7173f026b91eabff3fc45c6bafa20888fb79a26c4eff70348",
            true,
        ),
        (
            "two identical Slumbering Essence",
            vec![
                strike_row(1, "SLUMBERING_ESSENCE", 1),
                strike_row(1, "SLUMBERING_ESSENCE", 1),
            ],
            "d6eaa37b688b6f9431f5b381aa4b74bbd1773b70f9b13c47c43874a54b2df2c5",
            true,
        ),
        (
            "two fresh Genetic Algorithms",
            vec![genetic_algorithm_row(), genetic_algorithm_row()],
            "618b0996e72fe59f8951d66d9ac03983557a30bf859b2288f83d64ca6e51f289",
            true,
        ),
        (
            "one fresh Genetic Algorithm",
            vec![genetic_algorithm_row()],
            "2b4c6d90c7fa8fcf5f5d994ca9e789c8da51c873f3b3c8d9ce9e193c45cf5cf7",
            false,
        ),
    ];
    for (name, rows, oracle, exact) in cases {
        let mut case = empty_belt_case();
        for row in rows {
            deck_row(&mut case, row);
        }
        let opening = open(&case).unwrap_or_else(|err| panic!("{name} opens: {err}"));
        assert_eq!(
            opening.document.player.get("exact_piles"),
            exact.then_some(&Value::Bool(true)),
            "{name}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            oracle,
            "{name}"
        );
    }
}

/// #2926: a single Slither Strike dealt into the opening hand rolls its
/// combat-long cost on `CombatEnergyCosts` (`Slither::AfterCardDrawn` RVA
/// `0xd636c`; the IL is on `engine::draw::slither_after_card_drawn`).
///
/// The digest and the three moved fields are the oracle's, recorded on the
/// issue: the recipe above `empty_belt_case` with
/// `strike_row(1, "SLITHER", 1)` inserted at the front of the deck. The copy
/// lands at Hand index 4 and rolls 3; the dedicated stream advances once and
/// the cost row promotes `exact_piles`.
#[test]
fn one_slither_copy_dealt_in_the_opening_rolls_its_cost() {
    let mut case = empty_belt_case();
    deck_row(&mut case, strike_row(1, "SLITHER", 1));
    let opening = open(&case).expect("a single Slither Strike opens");
    let document = &opening.document;
    let slither = &document.piles["hand"][4];
    assert_eq!(
        serde_json::to_value(slither).expect("a canonical card serializes")["physical_state"],
        serde_json::json!(["PHYSICAL_CARD_STATE", [[1, 3, 0, false]], 0, false, []])
    );
    assert_eq!(document.player.get("exact_piles"), Some(&Value::Bool(true)));
    assert_eq!(
        serde_json::to_value(&document.rng["energy_costs"]).expect("rng serializes")["counter"],
        Value::from(1)
    );
    assert_eq!(
        document.differential_digest(),
        "7eeb1991b801ef7f4222e550a2eb2ed9485338f7f59792d16ccee5f411c3410c"
    );
}

/// #2891: every entering copy enchanted with a mutable-per-combat enchantment
/// carries the state the game's combat clone starts with — `[NAME, 0]` for
/// Momentum (`_extraDamage`) and Vigorous (`Status`), nothing for any other
/// enchantment (Glam/Sown/Swift carry theirs as the unspent plain id; the IL
/// is on `super::entry_enchantment_state`).
///
/// Every row and the digest were printed by the oracle with the recipe above
/// `empty_belt_case`, inserting `strike_row(1, "MOMENTUM", 2)` and then
/// `strike_row(1, "VIGOROUS", 3)` at the front of the deck.
#[test]
fn mutable_enchantments_enter_with_their_zero_slot_five_state() {
    let mut case = empty_belt_case();
    deck_row(&mut case, strike_row(1, "MOMENTUM", 2));
    deck_row(&mut case, strike_row(1, "VIGOROUS", 3));
    let opening = open(&case).expect("a Momentum + Vigorous deck opens");
    let enchanted: Vec<Value> = opening
        .document
        .piles
        .values()
        .flatten()
        .filter(|card| card.enchantment.is_some())
        .map(|card| serde_json::to_value(card).expect("a canonical card serializes"))
        .collect();
    assert_eq!(
        enchanted,
        vec![
            serde_json::json!({
                "enchantment": ["VIGOROUS", 3], "enchantment_state": ["VIGOROUS", 0],
                "id": "STRIKE_IRONCLAD", "uid": 5, "upgrade": 1}),
            serde_json::json!({
                "enchantment": ["MOMENTUM", 2], "enchantment_state": ["MOMENTUM", 0],
                "id": "STRIKE_IRONCLAD", "uid": 7, "upgrade": 1}),
        ]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "5037c5b859b94cb89f72767f7de2ce3863c7623d2f80793f8621a6b8f76bcff7"
    );
}

/// The seeded set is exactly Momentum and Vigorous over every enchantment this
/// crate names; the other twenty enter with no slot-5 row, as the oracle's
/// `_MUTABLE_ENCHANTMENTS` (frozen Python, deleted #2827) has it.
#[test]
fn only_momentum_and_vigorous_seed_a_slot_five_state() {
    for id in crate::ids::EnchantmentId::ALL {
        let state = super::entry_enchantment_state(id.as_str());
        match id {
            crate::ids::EnchantmentId::Momentum | crate::ids::EnchantmentId::Vigorous => {
                assert_eq!(state, Some(serde_json::json!([id.as_str(), 0])), "{id:?}");
            }
            _ => assert_eq!(state, None, "{id:?}"),
        }
    }
    assert_eq!(super::entry_enchantment_state("NOT_AN_ENCHANTMENT"), None);
}

// ---------------------------------------------------------------------------
// Thieving Hopper's master deck (#2928)
// ---------------------------------------------------------------------------
//
// Every digest below was printed by the oracle with the recipe above
// `empty_belt_case`, with `encounter_id` replaced by
// `'ENCOUNTER.THIEVING_HOPPER_WEAK'` in the `start_from_save` call and the
// named rows `insert(0, ...)`-ed into `save['players'][0]['deck']` (the same
// row shapes `strike_row`, `scythe_row` and `genetic_algorithm_row` build).

fn hopper_case(rows: &[Value]) -> Value {
    let mut case = empty_belt_case();
    case["encounter_id"] = Value::from("ENCOUNTER.THIEVING_HOPPER_WEAK");
    for row in rows.iter().rev() {
        deck_row(&mut case, row.clone());
    }
    case
}

/// A Thieving Hopper fight opens with the oracle's `hopper_master_deck`: one
/// `(deck_row, payload)` row per entering card in save-array order, each
/// combat copy's uid equal to its row, and `exact_piles` forced
/// (frozen Python `start_combat`, deleted #2827). Each witness is byte-identical to the
/// oracle's document.
///
/// * the fixture's uniform deck, where only the forced `exact_piles` and the
///   master rows distinguish the document from an ordinary fight's;
/// * a `SHARP 2` Strike, an enchantment payload on a master row, beside five
///   equal-payload siblings the deck-row uids keep distinct;
/// * a saved The Scythe (growth 3), whose master row carries the seven-field
///   slot-7 row with its deck row, the one Scythe link the boundary admits;
/// * a saved Genetic Algorithm, whose master row carries its state row;
/// * a `MOMENTUM 2` Strike, whose master row carries the zero mutable
///   enchantment state;
/// * a `CLONE 1` Strike, which the oracle keeps on a Hopper copy
///   (frozen Python `start_combat`, deleted #2827).
#[test]
fn a_thieving_hopper_fight_opens_with_the_oracles_master_deck() {
    let witnesses: [(&str, Vec<Value>, &str, Value); 6] = [
        (
            "uniform deck",
            Vec::new(),
            "8fbc997cb3ff2e5830886bfac0c15ac76baf6ffb0f83afc698afaba2af7b39fd",
            serde_json::json!(["STRIKE_IRONCLAD", 0]),
        ),
        (
            "Sharp Strike",
            vec![strike_row(0, "SHARP", 2)],
            "d0723d3cdfa3b6ee2e43a1ee4af41c5c9dcfa2596fda48229beabb7291da1081",
            serde_json::json!(["STRIKE_IRONCLAD", 0, ["SHARP", 2]]),
        ),
        (
            "The Scythe",
            vec![scythe_row(16, 3, 0)],
            "4b99f54d34f6b94e8c5718c56e816b48cc1a2f0c2738b4b6497be9ab33bbf88f",
            serde_json::json!([
                "THE_SCYTHE",
                0,
                null,
                [],
                [],
                null,
                null,
                ["PHYSICAL_CARD_STATE", [], 3, false, [], 0, 0]
            ]),
        ),
        (
            "Genetic Algorithm",
            vec![genetic_algorithm_row()],
            "b096ae3d6906cbf5dd54512b469cf6dd3f9fd687b6c4e6d870edadfeb36fdd70",
            serde_json::json!([
                "GENETIC_ALGORITHM",
                0,
                null,
                [],
                [],
                null,
                null,
                ["PHYSICAL_CARD_STATE", [], 0, false, []],
                ["GENETIC_ALGORITHM_STATE", 0, 0]
            ]),
        ),
        (
            "Momentum Strike",
            vec![strike_row(0, "MOMENTUM", 2)],
            "66cebf2dce2cbb656c51075296f7f24dbf16661bcc22b4a7fed2998b1b86089f",
            serde_json::json!([
                "STRIKE_IRONCLAD",
                0,
                ["MOMENTUM", 2],
                [],
                [],
                ["MOMENTUM", 0]
            ]),
        ),
        (
            "Clone Strike",
            vec![strike_row(0, "CLONE", 1)],
            "2350a187ee6f8e794efae1f5ae2c116e209c7417c9c77278d776b41307620cbf",
            serde_json::json!(["STRIKE_IRONCLAD", 0, ["CLONE", 1]]),
        ),
    ];
    for (name, rows, digest, first_payload) in witnesses {
        let case = hopper_case(&rows);
        let built = pre_hook(&case).expect("the Hopper pre-hook half builds");
        let deck_len = case["save"]["players"][0]["deck"]
            .as_array()
            .expect("deck")
            .len();
        let master = built.document.player["hopper_master_deck"]
            .as_array()
            .expect("the master deck is a row list")
            .clone();
        assert_eq!(master.len(), deck_len, "{name}");
        for (row, entry) in master.iter().enumerate() {
            assert_eq!(entry[0], Value::from(row), "{name}: rows in save order");
        }
        assert_eq!(master[0][1], first_payload, "{name}");
        assert_eq!(
            built.document.player.get("exact_piles"),
            Some(&Value::Bool(true)),
            "{name}"
        );
        // Each combat copy's uid is its row, carried through the shuffle.
        let mut uids: Vec<u64> = built.document.piles["draw"]
            .iter()
            .map(|card| card.uid.expect("identified"))
            .collect();
        uids.sort_unstable();
        assert_eq!(uids, (0..deck_len as u64).collect::<Vec<_>>(), "{name}");

        let opening = open(&case).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{name}"
        );
    }
}

/// Stone Cracker on a Hopper fight: the master rows are taken before the
/// upgrade, stay at upgrade 0, and the fight opens (#2965).
///
/// Oracle, with the deck replaced by `stone_distinct_deck` and Stone Cracker
/// appended (so the oracle's first-equal-copy upgrade lands on the chosen
/// card): it roots (digest
/// `c20bc3c3c0cbdf994c83ab5054982ba3d7dad7d4db7197180697c9adf41a4c34`) with
/// Defend and Strike upgraded in the piles and every master row at upgrade 0.
/// That matches native: `CardCmd::Upgrade` (RVA `0x12f660`) upgrades the
/// combat object only (`CardModel::UpgradeInternal` IL_0081, whose body
/// `0x7e0c8` never reads `DeckVersion`). The engine's Hopper validator now
/// holds a live copy to its master row's card id and enchantment only
/// (`monsters::hopper_live_identity_matches_master`), so the deal admits the
/// upgraded copies. Corpus: `f67d5d884f70c34b`.
#[test]
fn a_stone_cracker_hopper_fight_keeps_its_master_rows_unupgraded_and_opens() {
    let mut case = stone_case(stone_distinct_deck(), true);
    case["encounter_id"] = Value::from("ENCOUNTER.THIEVING_HOPPER_WEAK");
    let built = pre_hook(&case).expect("the Hopper pre-hook half builds");
    let master = built.document.player["hopper_master_deck"]
        .as_array()
        .expect("the master deck is a row list")
        .clone();
    assert_eq!(master.len(), 9);
    assert!(master.iter().all(|row| row[1][1] == 0), "{master:?}");
    assert_eq!(
        built.document.piles["draw"]
            .iter()
            .filter(|card| card.upgrade == 1)
            .count(),
        2
    );
    let opening = open(&case).unwrap_or_else(|refusal| panic!("{refusal}"));
    let master = opening.document.player["hopper_master_deck"]
        .as_array()
        .expect("the master deck is a row list")
        .clone();
    assert!(master.iter().all(|row| row[1][1] == 0), "{master:?}");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "c20bc3c3c0cbdf994c83ab5054982ba3d7dad7d4db7197180697c9adf41a4c34"
    );
}

/// A Hopper pile's expected post-deal uids are the deck rows it carried, and
/// that answer is taken ahead of the Tea of Discourtesy strict order
/// (`expected_identity_order`). Tea refuses earlier in `build`, so the branch
/// order is witnessed on the function directly.
#[test]
fn a_hopper_pile_is_held_to_its_deck_rows_ahead_of_the_tea_order() {
    use super::expected_identity_order;
    let built = pre_hook(&hopper_case(&[])).expect("the Hopper pre-hook half builds");
    let carried: Vec<Option<u64>> = built.document.piles["draw"]
        .iter()
        .map(|card| card.uid)
        .collect();
    assert_ne!(carried, (0..10).map(Some).collect::<Vec<_>>());
    for relics in [vec![], vec!["RELIC.TEA_OF_DISCOURTESY".to_string()]] {
        assert_eq!(
            expected_identity_order(&built.document, &relics, None),
            carried
        );
    }
}

// ---------------------------------------------------------------------------
// Pollinous Core, Symbiotic Virus and Vambrace (#2827)
// ---------------------------------------------------------------------------
//
// Every expected digest below was printed by the oracle with the recipe above
// `empty_belt_case`, appending the named relic rows (and, for Pollinous Core,
// `'props': {'ints': [{'name': 'TurnsSeen', 'value': N}]}`) to
// `save['players'][0]['relics']`. The same run reproduced
// `EMPTY_BELT_DIGEST` for the unmutated save.

/// `RELIC.POLLINOUS_CORE` advances its saved `TurnsSeen` at the first
/// `BeforeHandDraw` and grants two cards on the fourth turn.
///
/// Five saved counters. `0`, `1` and `2` advance without granting, and `3`
/// is the turn that grants: the hand is seven and the counter resets to `0`.
/// A saved `4` is reduced modulo the oracle's `POLLINOUS_CORE_TURNS` and opens
/// exactly as `0` does. The `3` case is the one the opening's old modulus of 3
/// got wrong (it would have opened at `0`, advanced to `1`, and dealt five), so
/// it is the load-bearing row.
#[test]
fn a_pollinous_core_save_advances_its_counter_and_grants_on_the_fourth_turn() {
    for (seen, after, hand, digest) in [
        (
            0,
            1,
            5,
            "fb4468d94d720149484c6e2d11830bc326b4e44a7ced686ac6c9850348429b42",
        ),
        (
            1,
            2,
            5,
            "f60ea242aa86565804b39cf37a252fd79da3feec25c8351751481d7556ee7582",
        ),
        (
            2,
            3,
            5,
            "8086b3e106fcd4ce2a42a4dc917308f07bec28fc1efb1cfd40ec04c38b77bd7c",
        ),
        (
            3,
            0,
            7,
            "48486845c945f238dcbe94433125ce1775eb41eb7b437035798227d6f8014234",
        ),
        (
            4,
            1,
            5,
            "fb4468d94d720149484c6e2d11830bc326b4e44a7ced686ac6c9850348429b42",
        ),
    ] {
        let case = with_relic_counter("RELIC.POLLINOUS_CORE", "TurnsSeen", seen);
        let opening =
            open(&case).unwrap_or_else(|err| panic!("Pollinous Core {seen} opens: {err}"));
        assert_eq!(
            opening.document.player.get("pollinous_core"),
            Some(&Value::from(after)),
            "TurnsSeen {seen} advances to {after}"
        );
        assert_eq!(hand_size(&opening.document), hand, "TurnsSeen {seen}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "TurnsSeen {seen}"
        );
    }
}

/// `RELIC.FAKE_HAPPY_FLOWER` seeds its saved `TurnsSeen` modulo 5, not 2
/// (#2906).
///
/// `FakeHappyFlower::get_CanonicalVars` (RVA `0x932f3`) builds
/// `DynamicVar("Turns", 5)` at `IL_0012`-`IL_001d`. The first turn start
/// advances the counter, and a saved `4` is the turn that grants the energy
/// and resets to `0`. Under the old modulus of 2, saved `2`, `3` and `4` opened
/// as `0`, `1` and `0`, so all three rows below failed; `4` is the one whose
/// energy grant moved. Every digest was printed by the oracle with the recipe
/// above `empty_belt_case`, appending the relic with
/// `'props': {'ints': [{'name': 'TurnsSeen', 'value': N}]}`.
#[test]
fn a_fake_happy_flower_save_seeds_its_counter_modulo_five() {
    for (seen, after, digest) in [
        (
            0,
            1,
            "8b7ea4eb61e27fff589014df5dbfc2e46866010bcd6026329b934a8051afc8b7",
        ),
        (
            1,
            2,
            "91e799746abf01b5486cb9ce5e7a5c67e8e657d5152d4a2d916c079cf3d4cba3",
        ),
        (
            2,
            3,
            "21c6546182ccff1bc6fa2fdb1c91f99a10a568be67d5c674cb287b04fbd0716c",
        ),
        (
            3,
            4,
            "adda8e32a764e1f898af37a6fac59a96505ed1ad0892f488c0429de287c20426",
        ),
        (
            4,
            0,
            "8533a83905c8609bafdd81efa1f26dca98acf5d4a1e6893e653e15c6cc5fbe99",
        ),
    ] {
        let case = with_relic_counter("RELIC.FAKE_HAPPY_FLOWER", "TurnsSeen", seen);
        let opening =
            open(&case).unwrap_or_else(|err| panic!("Fake Happy Flower {seen} opens: {err}"));
        assert_eq!(
            opening.document.player.get("fake_flower"),
            Some(&Value::from(after)),
            "TurnsSeen {seen} advances to {after}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "TurnsSeen {seen}"
        );
    }
}

/// Pollinous Core beside Blessed Antler refuses, as the oracle's
/// `BeforeHandDraw` acquisition-order gate does (frozen Python `start_combat`, deleted #2827).
///
/// The oracle refuses this save outright ("BeforeHandDraw relic acquisition
/// order is unrecorded for ['RELIC.BLESSED_ANTLER', 'RELIC.POLLINOUS_CORE']
/// …"); the opening must not produce a document for it. Until #2992 the
/// refusal was Blessed Antler's own room-entry gate; since Antler left
/// `OPENING_WINDOW_RELIC_BODIES` it is the oracle's peer gate, ported as
/// `refuse_unordered_blessed_antler_peers`, naming the same two relics.
#[test]
fn pollinous_core_beside_blessed_antler_refuses_like_the_oracle() {
    let mut case = with_relic("RELIC.BLESSED_ANTLER");
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.POLLINOUS_CORE",
            "props": {"ints": [{"name": "TurnsSeen", "value": 3}]}}),
    );
    let refusal = open(&case).unwrap_err();
    assert_eq!(refusal.class(), "relic_combination_refused", "{refusal}");
    match refusal {
        OpeningRefusal::RelicCombinationRefused { relics, .. } => assert_eq!(
            relics,
            ["RELIC.BLESSED_ANTLER", "RELIC.POLLINOUS_CORE"].map(String::from)
        ),
        other => panic!("wrong refusal: {other}"),
    }
}

/// `RELIC.SYMBIOTIC_VIRUS` channels one Dark orb on the first turn-start walk.
///
/// `SymbioticVirus/<AfterSideTurnStart>d__7::MoveNext` (`0x332298`); the body
/// is `engine::relics::symbiotic_virus_after_side_turn_start`. On this
/// Ironclad save the base slot count is zero, so `channel` bootstraps a
/// one-slot queue: the oracle's `[['DARK', 6]]` with `orb_slots` 1.
///
/// The Cracked Core case is the one that exercises order and the evoke path:
/// Cracked Core channels its Lightning at `BeforeSideTurnStart`, filling the
/// bootstrapped slot, and the Dark channel at `AfterSideTurnStart` then evokes
/// that Lightning (one `CombatTargets` roll and a hit) before queueing. The
/// oracle's document is the Dark orb alone, and the digest covers the damage
/// and the stream. Metronome proves the channel runs through the shared
/// command, and Brimstone is the late-group peer that follows it.
#[test]
fn a_symbiotic_virus_save_channels_one_dark_orb_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.SYMBIOTIC_VIRUS")).expect("Symbiotic Virus opens");
    assert_eq!(
        opening.document.player.get("orbs"),
        Some(&serde_json::json!([["DARK", 6]]))
    );
    assert_eq!(
        opening.document.player.get("orb_slots"),
        Some(&Value::from(1))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "e4dc26f0e2a549000d17f552e89335b84563272e235d6bd076be71eeb61771e5"
    );

    for (peer, digest) in [
        (
            "RELIC.CRACKED_CORE",
            "02b5057a00f2f75ec9524717db53ef27a775237ba40b156d6f673b9e744039bb",
        ),
        (
            "RELIC.METRONOME",
            "7674c43dc0cb318978cbce41d20e5a1a6b6f22b9550ad408d549a3efecdeb943",
        ),
        (
            "RELIC.BRIMSTONE",
            "d9ffe9121977fd93876c95635cd66e309fed53c99bb3ac5aa6612385fd3a1be7",
        ),
    ] {
        let mut case = with_relic("RELIC.SYMBIOTIC_VIRUS");
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": peer}),
        );
        let opening =
            open(&case).unwrap_or_else(|err| panic!("Symbiotic Virus with {peer} opens: {err}"));
        assert_eq!(
            opening.document.player.get("orbs"),
            Some(&serde_json::json!([["DARK", 6]])),
            "{peer}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{peer}"
        );
    }
}

/// Symbiotic Virus + Cracked Core + Brimstone at zero base slots refuses, as
/// the oracle's `start_combat` does (frozen Python, deleted #2827): the Dark
/// channel's evoke can kill before or after Brimstone's live-opponent
/// snapshot depending on unrecorded acquisition order.
#[test]
fn symbiotic_virus_with_cracked_core_and_brimstone_refuses_like_the_oracle() {
    let mut case = with_relic("RELIC.SYMBIOTIC_VIRUS");
    for peer in ["RELIC.CRACKED_CORE", "RELIC.BRIMSTONE"] {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": peer}),
        );
    }
    let refusal = open(&case).unwrap_err();
    assert_eq!(refusal.class(), "symbiotic_virus_turn_start_order");
    assert!(
        refusal.to_string().contains("RELIC.BRIMSTONE")
            && refusal.to_string().contains("RELIC.CRACKED_CORE"),
        "{refusal}"
    );
}

/// Every arm of the oracle's Symbiotic Virus refusal, and the capacities at
/// which it does not refuse (frozen Python `start_combat`, deleted #2827).
///
/// Called directly because two of the four peers (Infused Core, Runic
/// Capacitor) are still gated before the call, so no save can reach those arms
/// through `open` yet; the Fencing Manual arm is reached through `open` by
/// `fencing_manual_with_symbiotic_virus_and_cracked_core_refuses_like_the_oracle`. The base-slot argument is the
/// character's (`orb_base_slots`): 0 for Ironclad, 3 for Defect.
#[test]
fn every_symbiotic_virus_order_refusal_arm_matches_the_oracle() {
    let check = |owned: &[&str], base: i64| {
        let relics: std::collections::BTreeSet<&str> = owned.iter().copied().collect();
        super::symbiotic_virus_turn_start_order_is_recorded(&relics, base)
            .err()
            .map(|refusal| match refusal {
                OpeningRefusal::SymbioticVirusTurnStartOrder { peers } => peers,
                other => panic!("wrong refusal {other:?}"),
            })
    };
    const SV: &str = "RELIC.SYMBIOTIC_VIRUS";
    // Infused Core refuses at any capacity.
    for base in [0, 3] {
        assert_eq!(
            check(&[SV, "RELIC.INFUSED_CORE"], base),
            Some(vec!["RELIC.INFUSED_CORE".to_string()]),
            "base {base}"
        );
    }
    // Runic Capacitor only at zero base slots.
    assert_eq!(
        check(&[SV, "RELIC.RUNIC_CAPACITOR"], 0),
        Some(vec!["RELIC.RUNIC_CAPACITOR".to_string()])
    );
    assert_eq!(check(&[SV, "RELIC.RUNIC_CAPACITOR"], 3), None);
    // Cracked Core only at zero base slots, and only with Fencing Manual or
    // Brimstone.
    for peer in ["RELIC.FENCING_MANUAL", "RELIC.BRIMSTONE"] {
        assert_eq!(
            check(&[SV, "RELIC.CRACKED_CORE", peer], 0),
            Some(vec!["RELIC.CRACKED_CORE".to_string(), peer.to_string()]),
            "{peer}"
        );
        assert_eq!(check(&[SV, "RELIC.CRACKED_CORE", peer], 3), None, "{peer}");
        assert_eq!(check(&[SV, peer], 0), None, "{peer} without Cracked Core");
    }
    assert_eq!(check(&[SV, "RELIC.CRACKED_CORE"], 0), None);
    // Without Symbiotic Virus nothing here refuses.
    assert_eq!(
        check(&["RELIC.INFUSED_CORE", "RELIC.CRACKED_CORE"], 0),
        None
    );
    assert_eq!(check(&[SV], 0), None);
}

/// `RELIC.VAMBRACE`'s `BeforeCombatStart` makes the first-card-block doubling
/// available (`engine::vambrace_before_combat_start`).
///
/// The control emits no `vambrace_available`, so the flag is the relic's and
/// not a default. Before #2827 the opening wrote nothing here, which is why
/// the relic was gated: the document would have said `false`.
#[test]
fn a_vambrace_save_arms_its_block_doubling_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.VAMBRACE")).expect("Vambrace opens");
    assert_eq!(
        flag(&opening.document, "vambrace_available"),
        Some(Value::Bool(true))
    );
    assert_eq!(flag(&opening.document, "vambrace_trigger_uid"), None);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "a32283db076573a0076654a9f3178a76cecebc763e8545dbd6bedca97fe5e930"
    );
    let control = open(&empty_belt_case()).expect("the control opens");
    assert_eq!(flag(&control.document, "vambrace_available"), None);
}

/// All three on one save, in the oracle recipe's order (Vambrace, Pollinous
/// Core at `TurnsSeen` 3, Symbiotic Virus): seven cards, the Dark orb and the
/// armed Vambrace together.
#[test]
fn pollinous_core_symbiotic_virus_and_vambrace_stack_on_one_save() {
    let mut case = with_relic("RELIC.VAMBRACE");
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.POLLINOUS_CORE",
            "props": {"ints": [{"name": "TurnsSeen", "value": 3}]}}),
    );
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.SYMBIOTIC_VIRUS"}),
    );
    let opening = open(&case).expect("all three open");
    assert_eq!(hand_size(&opening.document), 7);
    assert_eq!(
        opening.document.player.get("orbs"),
        Some(&serde_json::json!([["DARK", 6]]))
    );
    assert_eq!(
        flag(&opening.document, "vambrace_available"),
        Some(Value::Bool(true))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "c1612978430b14d41fc1574e86c90c25172ad90cf6148d63b9e30ba175b99c6f"
    );
}

// ---------------------------------------------------------------------------
// Kusarigama and Phylactery Unbound (#2827)
// ---------------------------------------------------------------------------
//
// Expected digests printed by the oracle with the recipe above
// `empty_belt_case`, appending each relic as `with_relic` does (and, for the
// Juggernaut cases, inserting `{'floor_added_to_deck': 1, 'id':
// 'CARD.JUGGERNAUT'}` at the front of the deck as `deck_row` does).

fn with_relics(first: &str, second: &str) -> Value {
    let mut case = with_relic(first);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": second}),
    );
    case
}

fn with_juggernaut(mut case: Value) -> Value {
    deck_row(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.JUGGERNAUT"}),
    );
    case
}

/// `RELIC.KUSARIGAMA`'s `BeforeCombatStart` (`0x959e4`) only zeroes
/// `AttacksPlayedThisTurn` and `Status`, and the opening already seeds the
/// `kusarigama` counter to 0 for an owner (frozen Python `start_combat`, deleted #2827). Not the
/// control: the control projects no `kusarigama` field at all.
#[test]
fn a_kusarigama_save_opens_with_a_zero_counter_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.KUSARIGAMA")).expect("Kusarigama opens");
    assert_eq!(
        opening.document.player.get("kusarigama"),
        Some(&Value::from(0))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "b7968c7a831959a2ff48a55d02fde2b78c82ff9e827497c9682d3a132a2ac279"
    );
}

/// `RELIC.PHYLACTERY_UNBOUND` summons Osty 5 at `BeforeCombatStart`
/// (`<BeforeCombatStart>d__10` `0x32e234`, `IL_003e`) and 2 more on the first
/// `AfterSideTurnStart` (`<AfterSideTurnStart>d__11` `0x32e134`, `IL_005b`,
/// no turn test). Both hooks are covered, so the oracle's 7/7 is what the
/// opening emits: a 5/5 would mean the turn-1 half was lost, a 2/2 the
/// combat-start half.
#[test]
fn a_phylactery_unbound_save_summons_a_seven_hp_osty_and_matches_the_oracle() {
    let opening = open(&with_relic("RELIC.PHYLACTERY_UNBOUND")).expect("Phylactery Unbound opens");
    assert_eq!(
        opening.document.player.get("ally"),
        Some(&serde_json::json!(["OSTY", 7, 7]))
    );
    assert_eq!(
        oracle_pet_digest(&opening.document, 3),
        "4da050da1f7d0ef68f5a2b98987de6a8d822c86050d3b85a2810c595aec9cf34"
    );
}

/// Both relics of the bundle on one save, which the oracle roots.
#[test]
fn kusarigama_and_phylactery_unbound_open_together_and_match_the_oracle() {
    let opening = open(&with_relics("RELIC.KUSARIGAMA", "RELIC.PHYLACTERY_UNBOUND"))
        .expect("Kusarigama with Phylactery Unbound opens");
    assert_eq!(
        oracle_pet_digest(&opening.document, 3),
        "1c13df1200d766b18ab57c39e03a3e8d28bbb0ff13aba6f5a3b766dc62edf5b3"
    );
}

/// The near misses of the Kusarigama order gate, each rooted by the oracle:
/// Daughter of the Wind without Juggernaut, and Juggernaut without Daughter of
/// the Wind. The gate must not over-refuse either.
#[test]
fn kusarigama_near_misses_of_the_order_gate_open_and_match_the_oracle() {
    let daughter = open(&with_relics(
        "RELIC.DAUGHTER_OF_THE_WIND",
        "RELIC.KUSARIGAMA",
    ))
    .expect("Daughter of the Wind with Kusarigama opens");
    assert_eq!(
        daughter.document.differential_digest(),
        "da60e0fba97017bb732435c0c2a102f42b7906f84464963872b5a3b2d13039ba"
    );
    let juggernaut = open(&with_juggernaut(with_relic("RELIC.KUSARIGAMA")))
        .expect("Kusarigama with Juggernaut opens");
    assert_eq!(
        juggernaut.document.differential_digest(),
        "bca1e5cac13f77d3575be1b1b14274b286d45b01e86d25166a60800be1fe41a6"
    );
}

/// The three `start_combat` combinations the oracle refuses for this bundle,
/// each refused by name. The oracle's messages for the same saves are
/// "incompatible phylactery bootstrap relic ownership", "MUSIC_BOX +
/// KUSARIGAMA: AfterCardPlayed relic acquisition order …" and "Daughter of the
/// Wind + Kusarigama with Juggernaut: same-hook …".
#[test]
fn the_oracles_kusarigama_and_phylactery_combinations_refuse_by_name() {
    for (case, relics) in [
        (
            with_relics("RELIC.BOUND_PHYLACTERY", "RELIC.PHYLACTERY_UNBOUND"),
            ["RELIC.BOUND_PHYLACTERY", "RELIC.PHYLACTERY_UNBOUND"],
        ),
        (
            with_relics("RELIC.MUSIC_BOX", "RELIC.KUSARIGAMA"),
            ["RELIC.MUSIC_BOX", "RELIC.KUSARIGAMA"],
        ),
        (
            with_juggernaut(with_relics(
                "RELIC.DAUGHTER_OF_THE_WIND",
                "RELIC.KUSARIGAMA",
            )),
            ["RELIC.DAUGHTER_OF_THE_WIND", "RELIC.KUSARIGAMA"],
        ),
    ] {
        let refusal = open(&case).unwrap_err();
        assert_eq!(refusal.class(), "relic_combination_refused", "{refusal}");
        match refusal {
            OpeningRefusal::RelicCombinationRefused { relics: named, .. } => {
                assert_eq!(named, relics.map(String::from).to_vec());
            }
            other => panic!("wrong refusal: {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Blessed Antler (#2992)
// ---------------------------------------------------------------------------
//
// Expected digests printed by the oracle with the recipe above
// `empty_belt_case`, appending each relic as `with_relic` does (Pendulum with
// `TurnsSeen` 2 as `with_relic_counter` writes it).

/// `RELIC.BLESSED_ANTLER`'s turn-1 `BeforeHandDraw`
/// (`<BeforeHandDraw>d__7::MoveNext` `0x31fe64`, one
/// `AddGeneratedCardsToCombat(Draw, Random)` of three fresh Dazed at
/// `IL_0094`-`IL_009d`) runs in the opening, before the five-card deal, and its
/// `ModifyMaxEnergy` `+1` (`0x90d38`) is read by the first energy reset.
///
/// Pinned piecewise as well as by digest, so a failure says which half moved:
/// the hand is the control's five cards, the three Dazed (the oracle's uids
/// 10, 11, 12) sit in Draw at the oracle's random indices, the Shuffle-stream
/// `rng` counter is three draws past the control's, energy is 4, and
/// `exact_piles` is promoted. The first digest this produced missed only that
/// last field: the engine row had never set it, because before #2992 no root
/// reached the row.
#[test]
fn a_blessed_antler_save_shuffles_three_dazed_into_draw_and_matches_the_oracle() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.BLESSED_ANTLER")).expect("Blessed Antler opens");
    let ids = |pile: &str| -> Vec<(String, u64)> {
        opening.document.piles[pile]
            .iter()
            .map(|card| (card.id.clone(), card.uid.expect("card uid")))
            .collect()
    };
    assert_eq!(
        opening.document.piles["hand"],
        control.document.piles["hand"]
    );
    let draw = ids("draw");
    let dazed: Vec<(usize, u64)> = draw
        .iter()
        .enumerate()
        .filter(|(_, (id, _))| id == "DAZED")
        .map(|(index, (_, uid))| (index, *uid))
        .collect();
    assert_eq!(dazed, [(2, 11), (3, 12), (6, 10)], "{draw:?}");
    assert_eq!(draw.len(), 8, "five of the ten dealt, plus three Dazed");
    assert_eq!(
        opening.document.rng["rng"].counter,
        control.document.rng["rng"].counter + 3,
        "one Shuffle-stream draw per Dazed"
    );
    assert_eq!(opening.document.player.get("energy"), Some(&Value::from(4)));
    // The oracle's `force_exact=True` commit; the control carries no field.
    assert_eq!(
        opening.document.player.get("exact_piles"),
        Some(&Value::Bool(true))
    );
    assert_eq!(control.document.player.get("exact_piles"), None);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "8f558403d121b917d3aea8192d239982a47a4fdfedd3b8f51d4ae0c09d3fbf9e"
    );
}

/// Blessed Antler beside the two turn-1 draw relics the oracle does NOT group
/// with it on `BeforeHandDraw`, each rooted by the oracle: Pendulum (its
/// counter advance shares the hook but touches no pile or stream) at the
/// granting `TurnsSeen` 2, and Bag of Preparation (`ModifyHandDraw` only). The
/// Dazed land before the larger draw, so the draw can reach them.
#[test]
fn blessed_antler_beside_the_ungrouped_draw_relics_opens_and_matches_the_oracle() {
    let mut pendulum = with_relic("RELIC.BLESSED_ANTLER");
    relic(
        &mut pendulum,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.PENDULUM",
            "props": {"ints": [{"name": "TurnsSeen", "value": 2}]}}),
    );
    let pendulum = open(&pendulum).expect("Blessed Antler with Pendulum opens");
    assert_eq!(hand_size(&pendulum.document), 6);
    assert_eq!(
        pendulum.document.differential_digest(),
        "5e0c80246d736d326eaa26cd274bb6b26b65bcb27ef300ead7dcadc80d3cc441"
    );
    let bag = open(&with_relics(
        "RELIC.BLESSED_ANTLER",
        "RELIC.BAG_OF_PREPARATION",
    ))
    .expect("Blessed Antler with Bag of Preparation opens");
    assert_eq!(hand_size(&bag.document), 7);
    assert_eq!(
        bag.document.differential_digest(),
        "660a6746a63f22d20c8e9b00831a6602e0ea6bd377190860ae905dda2bbf36d5"
    );
}

/// The uid check's pre-deal arm (#2992), on the real Antler document: it
/// admits the three Dazed wherever the deal left them, and refuses when the
/// arm is not told about them (they are then neither the hand's tail nor the
/// deck), when one of them is not a Dazed, and when `next_card_uid` claims
/// fewer allocations than were inserted.
#[test]
fn the_uid_check_admits_blessed_antlers_pre_deal_dazed_and_nothing_else() {
    use super::{check_card_identity_allocation, expected_identity_order};
    let case = with_relic("RELIC.BLESSED_ANTLER");
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    let opened = open(&case).expect("Blessed Antler opens").document;
    let expected = expected_identity_order(&built.document, &[], None);
    assert_eq!(expected.len(), 10);
    assert_eq!(
        check_card_identity_allocation(&opened, &expected, 3),
        Ok(())
    );
    let refuses = |document: &crate::canonical::CanonicalStateV2, pre_deal: u64| {
        matches!(
            check_card_identity_allocation(document, &expected, pre_deal),
            Err(OpeningRefusal::CardIdentityAllocationDiverged { .. })
        )
    };
    assert!(refuses(&opened, 0), "unannounced pre-deal cards refuse");
    let mut renamed = opened.clone();
    let dazed = renamed
        .piles
        .get_mut("draw")
        .expect("draw")
        .iter_mut()
        .find(|card| card.id == "DAZED")
        .expect("a Dazed in Draw");
    dazed.id = "WOUND".to_string();
    assert!(
        refuses(&renamed, 3),
        "a non-Dazed at an inserted uid refuses"
    );
    let mut undercounted = opened.clone();
    undercounted
        .player
        .insert("next_card_uid".to_string(), Value::from(12));
    assert!(
        refuses(&undercounted, 3),
        "next_card_uid short of the inserts refuses"
    );
}

/// Every other member of the oracle's `BeforeHandDraw` group beside Blessed
/// Antler refuses (frozen Python `start_combat`, deleted #2827). Pollinous Core reaches the
/// ported gate (its own test above), and so, since #3162, do Funerary Mask,
/// Jeweled Mask and Radiant Pearl; Ninja Scroll is still in
/// `OPENING_WINDOW_RELIC_BODIES`, which is reached later but refuses it by its
/// own name. Either way no document is emitted, as the oracle emits
/// none.
#[test]
fn every_before_hand_draw_peer_of_blessed_antler_refuses() {
    for peer in super::BEFORE_HAND_DRAW_ORDER_RELICS
        .into_iter()
        .filter(|relic| *relic != "RELIC.BLESSED_ANTLER")
    {
        let refusal = open(&with_relics("RELIC.BLESSED_ANTLER", peer))
            .err()
            .unwrap_or_else(|| panic!("Blessed Antler with {peer} must refuse"));
        match &refusal {
            OpeningRefusal::RelicCombinationRefused { relics, .. } => {
                assert!(
                    relics.contains(&"RELIC.BLESSED_ANTLER".to_string()),
                    "{refusal}"
                );
                assert!(relics.contains(&peer.to_string()), "{refusal}");
            }
            OpeningRefusal::RoomEntryRelicNotModeled { relic } => {
                assert_eq!(relic, peer, "{refusal}");
            }
            other => panic!("Blessed Antler with {peer}: wrong refusal {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// #2847: the charged Tea Sets and the owner-pool listener provenance
// ---------------------------------------------------------------------------
//
// Every digest below was printed by the oracle with the recipe in the comment
// block above `empty_belt_case`, in the same run that reproduced
// `EMPTY_BELT_DIGEST`. The per-case mutations are named on each test.

/// The fixture case with an empty belt plus one Tea Set whose saved
/// `GainEnergyInNextCombat` is `charged` (`None`: the property is absent).
///
/// Mutation: `save['players'][0]['relics'].append({'floor_added_to_deck': 1,
/// 'id': <relic>, 'props': {'bools': [{'name': 'GainEnergyInNextCombat',
/// 'value': <charged>}]}})`, the `props` key omitted for `None`.
fn with_tea_set(id: &str, charged: Option<bool>) -> Value {
    let mut case = empty_belt_case();
    let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": id});
    if let Some(charged) = charged {
        row["props"] = serde_json::json!({"bools": [
            {"name": "GainEnergyInNextCombat", "value": charged}]});
    }
    relic(&mut case, row);
    case
}

/// A charged `RELIC.VENERABLE_TEA_SET` grants its 2 energy on turn 1 and the
/// charge is spent (#2847).
///
/// Before the fix the opening never seeded the charge the save carries, so the
/// document came back at 3 energy: a well-formed, silently wrong answer.
///
/// The one field the Rust document deliberately differs from the oracle's on
/// is `tea_set`. Native clears `_gainEnergyInNextCombat` right after the grant
/// (`VenerableTeaSet/<AfterEnergyReset>d__11::MoveNext`, RVA `0x333bc4`:
/// `PlayerCmd::GainEnergy` at `IL_0059`, then `ldc.i4.0;
/// set_GainEnergyInNextCombat` at `IL_00b0`-`IL_00b2`, unconditionally once
/// the await completes), and `engine::relics::after_energy_reset` does the
/// same. Frozen Python keeps `tea_set` true for the whole fight and gates the
/// grant on `turn == 1` instead (frozen Python `begin_player_turn`, deleted #2827), so its post-opening
/// document says `tea_set: true` where native's relic is already spent, while
/// the same oracle clears the identically shaped Fake Tea Set
/// (`begin_player_turn`; witnessed below). Rust is the engine authority (#1282) and
/// follows the IL. So the pin is: Rust equals the oracle's document with
/// `tea_set` removed (`31437103…`), and the oracle's full digest (`79da0f5f…`)
/// differs from Rust's by exactly that key.
#[test]
fn a_charged_venerable_tea_set_grants_two_energy_and_is_spent() {
    let opening = open(&with_tea_set("RELIC.VENERABLE_TEA_SET", Some(true)))
        .expect("a charged Tea Set opens");
    let player = &opening.document.player;
    assert_eq!(player.get("energy"), Some(&Value::from(5)));
    assert_eq!(player.get("tea_set"), None, "the charge is spent on turn 1");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "31437103f09bf0a8b1778c5e155280014a618342f0c4d33988c9e42d9c20a079",
        "the oracle's document with `tea_set` removed"
    );
    let mut with_python_flag = opening.document.clone();
    with_python_flag
        .player
        .insert("tea_set".to_string(), Value::Bool(true));
    assert_eq!(
        with_python_flag.differential_digest(),
        "79da0f5f8ff85f00d2b3589f0938e283d28d3c4918547165aaadd8ba4bbd09a1",
        "the oracle's full document differs by `tea_set` alone"
    );
}

/// An uncharged Tea Set is inert, as it was before.
#[test]
fn an_uncharged_venerable_tea_set_is_inert() {
    let opening = open(&with_tea_set("RELIC.VENERABLE_TEA_SET", Some(false)))
        .expect("an uncharged Tea Set opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "9fdc34eb611e6b2b8ed67db166afce63380f703ce93e950080a5566d6468a3a5"
    );
}

/// A charged `RELIC.FAKE_VENERABLE_TEA_SET` grants 1 and is spent. The oracle
/// clears this one too, so here the whole document matches.
/// `FakeVenerableTeaSet/<AfterEnergyReset>d__13::MoveNext` (`0x32497c`) has
/// the same shape: `GainEnergy` at `IL_0059`, `set_GainEnergyInNextCombat(false)`
/// at `IL_00b1`-`IL_00b2`. It was silently inert in the opening before #2847.
#[test]
fn a_charged_fake_tea_set_grants_one_energy_and_matches_the_oracle() {
    let opening = open(&with_tea_set("RELIC.FAKE_VENERABLE_TEA_SET", Some(true)))
        .expect("a charged Fake Tea Set opens");
    assert_eq!(opening.document.player.get("energy"), Some(&Value::from(4)));
    assert_eq!(opening.document.player.get("fake_tea_set_charged"), None);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "2daf4b79b9ff02977736a6da185c38c8822ae524957288bbe38c78664ca00359"
    );
    let uncharged = open(&with_tea_set("RELIC.FAKE_VENERABLE_TEA_SET", Some(false)))
        .expect("an uncharged Fake Tea Set opens");
    assert_eq!(
        uncharged.document.differential_digest(),
        "786daf21f557931b9b4056b68e717b51250b73fdbd1949215f98f39759269a34"
    );
}

/// Either Tea Set with no saved charge refuses by its own name, as the oracle
/// does (frozen Python `start_combat`, deleted #2827; the Fake one's exact-bool gate too).
#[test]
fn an_undated_tea_set_refuses_by_its_own_name() {
    for relic in ["RELIC.VENERABLE_TEA_SET", "RELIC.FAKE_VENERABLE_TEA_SET"] {
        let refusal = open(&with_tea_set(relic, None)).unwrap_err();
        assert_eq!(refusal.class(), "tea_set_charge_undated");
        assert!(
            matches!(
                refusal,
                OpeningRefusal::TeaSetChargeUndated { relic: named } if named == relic
            ),
            "{refusal}"
        );
    }
}

/// The other two saved run-state relics the entry routes (#2847's sweep for
/// Tea-Set-shaped state): a spent `RELIC.LIZARD_TAIL` opened unspent, and a
/// `RELIC.PUMPKIN_CANDLE` owner could not open at all. Both now carry the
/// saved value.
///
/// Mutations: `relics.append({'floor_added_to_deck': 1, 'id':
/// 'RELIC.LIZARD_TAIL', 'props': {'bools': [{'name': 'WasUsed', 'value':
/// <used>}]}})`, and the same for `RELIC.PUMPKIN_CANDLE` with `{'ints':
/// [{'name': 'KindleCount', 'value': <n>}]}`; the absent rows omit `props`.
/// Oracle: *"LIZARD_TAIL WasUsed is not an exact saved bool: None"* and
/// *"PUMPKIN_CANDLE persistent counter is not an exact integer in >=0: None"*.
#[test]
fn saved_lizard_tail_and_pumpkin_candle_state_is_seeded_from_the_save() {
    for (id, props, digest) in [
        (
            "RELIC.LIZARD_TAIL",
            serde_json::json!({"bools": [{"name": "WasUsed", "value": true}]}),
            "2dc4120fb5dab3e411963b75824c265040a64de05edd8279a5e597aaeed91b25",
        ),
        (
            "RELIC.LIZARD_TAIL",
            serde_json::json!({"bools": [{"name": "WasUsed", "value": false}]}),
            "97f7f7d60adbc5ec8410885e12b6a011c9cb4a96f1222f28571ddd7cec637a60",
        ),
        (
            "RELIC.PUMPKIN_CANDLE",
            serde_json::json!({"ints": [{"name": "KindleCount", "value": 3}]}),
            "9cc078e37dbe794e31a162221b085d9a4567f60da05d69ea9a86f72938fb6bba",
        ),
        (
            "RELIC.PUMPKIN_CANDLE",
            serde_json::json!({"ints": [{"name": "KindleCount", "value": 0}]}),
            "64803b6ae8ef9f87fedc18302d199ebee23333697f5d485f42141245e31da7ca",
        ),
    ] {
        let mut case = empty_belt_case();
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": id, "props": props}),
        );
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{id} {props}: {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{id} {props}"
        );
    }
    for id in ["RELIC.LIZARD_TAIL", "RELIC.PUMPKIN_CANDLE"] {
        let refusal = open(&with_relic(id)).unwrap_err();
        assert!(
            matches!(
                refusal,
                OpeningRefusal::RelicCounterNotExact { relic, .. } if relic == id
            ),
            "{refusal}"
        );
    }
}

/// `unlocked_epochs` for the listener-pool witnesses: two Defect epochs, so
/// the derived pools are **partial** (Hello World loses `BARRAGE`, Creative AI
/// loses `SMOKESTACK`) and a fully-unlocked fallback would be caught.
const TWO_DEFECT_EPOCHS: [&str; 2] = ["DEFECT1_EPOCH", "DEFECT2_EPOCH"];

/// The oracle's `combat_orbs` stream at counter 0 for the fixture seed, read
/// off its projected control document, so a save can record the stream
/// Creative AI's gate needs without changing any other byte.
fn with_orb_stream(mut case: Value) -> Value {
    case["save"]["rng"]["rngs"]["combat_orbs"] = serde_json::json!({
        "counter": 0,
        "s0": 16_452_943_454_101_452_638_u64,
        "s1": 5_525_124_908_612_069_843_u64,
        "s2": 10_373_153_506_651_793_454_u64,
        "s3": 1_658_981_437_511_799_834_u64,
    });
    case
}

/// The fixture case as a Defect deck holding `cards`, under `epochs`.
///
/// Mutation: `p['character_id'] = 'CHARACTER.DEFECT'`, each card
/// `p['deck'].insert(0, {'floor_added_to_deck': 1, 'id': 'CARD.' + card})` in
/// the order given, and `p['unlock_state']['unlocked_epochs'] = epochs`.
fn defect_with(cards: &[&str], epochs: &[&str]) -> Value {
    let mut case = empty_belt_case();
    case["save"]["players"][0]["character_id"] = Value::from("CHARACTER.DEFECT");
    for card in cards {
        deck_row(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": format!("CARD.{card}")}),
        );
    }
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = serde_json::json!(epochs);
    case
}

fn pool(document: &crate::canonical::CanonicalStateV2, field: &str) -> Vec<String> {
    document.player[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} is written"))
        .iter()
        .map(|id| id.as_str().expect("a card id").to_string())
        .collect()
}

/// Hello World's pool is written from the fight's own profile (#2847). The
/// four corpus rows that diverged on exactly this field
/// (`f38bb375c766e99c`, `f4ca0c5a5b106a68`, `ffc952a3c07213ae`,
/// `f10c37c16e49f633`) now match the oracle in the opening census.
#[test]
fn a_hello_world_deck_writes_its_profile_exact_owner_pool() {
    let opening = open(&defect_with(&["HELLO_WORLD"], &TWO_DEFECT_EPOCHS))
        .expect("a Hello World Defect deck opens");
    assert_eq!(
        pool(&opening.document, "hello_world_generation_pool"),
        [
            "BALL_LIGHTNING",
            "BEAM_CELL",
            "BOOST_AWAY",
            "CHARGE_BATTERY",
            "CLAW",
            "COLD_SNAP",
            "COMPILE_DRIVER",
            "COOLHEADED",
            "FOCUSED_STRIKE",
            "GO_FOR_THE_EYES",
            "GUNK_UP",
            "HOLOGRAM",
            "HOTFIX",
            "LEAP",
            "LIGHTNING_ROD",
            "MOMENTUM_STRIKE",
            "SWEEPING_BEAM",
            "UPROAR",
        ]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "708939275b33807ab1e3beda6e85ed850eae80431bd6b7317a3099963a56cf49"
    );
    // The control: the same Defect save without the card writes no pool.
    let plain = open(&defect_with(&[], &TWO_DEFECT_EPOCHS)).expect("the plain Defect opens");
    assert!(
        !plain
            .document
            .player
            .contains_key("hello_world_generation_pool")
    );
    assert_eq!(
        plain.document.differential_digest(),
        "6b92edbb7745aeee265238a52bc20d50d9818ba90d02ca8433c0f54451e5bbeb"
    );
}

/// Creative AI's pool, alone and beside Hello World's.
#[test]
fn a_creative_ai_deck_writes_its_profile_exact_power_pool() {
    let opening = open(&with_orb_stream(defect_with(
        &["CREATIVE_AI"],
        &TWO_DEFECT_EPOCHS,
    )))
    .expect("a Creative AI Defect deck opens");
    assert_eq!(
        pool(&opening.document, "creative_ai_generation_pool"),
        [
            "BUFFER",
            "BULK_UP",
            "CAPACITOR",
            "CONSUMING_SHADOW",
            "COOLANT",
            "CREATIVE_AI",
            "DEFRAGMENT",
            "ECHO_FORM",
            "FERAL",
            "HAILSTORM",
            "ITERATION",
            "LOOP",
            "MACHINE_LEARNING",
            "SPINNER",
            "STORM",
            "SUBROUTINE",
            "THUNDER",
            "TRASH_TO_TREASURE",
        ]
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "69f3c842aecd761d45e475879150c5c256ca036376cb28e9d99fb7582cc236f9"
    );
    let both = open(&with_orb_stream(defect_with(
        &["HELLO_WORLD", "CREATIVE_AI"],
        &TWO_DEFECT_EPOCHS,
    )))
    .expect("both listener cards open");
    assert_eq!(
        both.document.differential_digest(),
        "17bd8cc78cff848331988cdd1520b4f1dc644484e7cc30ad356c3888b2dcb927"
    );
}

/// The oracle's listener-card gates refuse here too, by name, rather than
/// opening a document without the pool (frozen Python `start_combat`, deleted #2827). Each row was measured refusing in the oracle:
/// *"... requires a recorded card-pool unlock profile"*, *"CombatCardGeneration
/// counter must be an exact non-negative integer for Hello World"* and
/// *"CombatOrbGeneration counter unknown for this fight"*. The oracle's fourth,
/// *"Hello World requires an explicit Defect owner"*, is retired (#3375):
/// only a fight with no character card pool at all refuses on the owner now.
#[test]
fn the_oracles_listener_card_gates_refuse_by_name() {
    for card in ["HELLO_WORLD", "CALL_OF_THE_VOID", "CREATIVE_AI"] {
        let mut ownerless = with_orb_stream(defect_with(&[card], &TWO_DEFECT_EPOCHS));
        ownerless["save"]["players"][0]
            .as_object_mut()
            .unwrap()
            .remove("character_id");
        assert_eq!(
            open(&ownerless).unwrap_err(),
            OpeningRefusal::ListenerPoolProvenance {
                source: card,
                requires: "an explicit character owner with a card pool",
            },
            "{card}"
        );
    }
    for card in ["HELLO_WORLD", "CREATIVE_AI"] {
        let refusal = open(&with_orb_stream(defect_with(&[card], &[]))).unwrap_err();
        assert!(
            matches!(
                refusal,
                OpeningRefusal::ListenerPoolProvenance {
                    source,
                    requires: "a recorded card-pool unlock profile",
                } if source == card
            ),
            "{refusal}"
        );
    }
    let mut no_generation = defect_with(&["HELLO_WORLD"], &TWO_DEFECT_EPOCHS);
    no_generation["save"]["rng"]["rngs"]
        .as_object_mut()
        .expect("the save's streams are an object")
        .remove("combat_card_generation");
    assert_eq!(
        open(&no_generation).unwrap_err(),
        OpeningRefusal::CombatStreamAbsent {
            stream: "combat_card_generation",
        }
    );
    assert_eq!(
        open(&defect_with(&["CREATIVE_AI"], &TWO_DEFECT_EPOCHS)).unwrap_err(),
        OpeningRefusal::CombatStreamAbsent {
            stream: "combat_orbs",
        }
    );
}

/// Call of the Void, the Necrobinder row of `OWNER_LISTENER_POOL_FIELDS`
/// (#2847 review): its pool, its control, and each of its four oracle gates.
///
/// Mutation: `p['character_id'] = 'CHARACTER.NECROBINDER'`,
/// `p['deck'].insert(0, {'floor_added_to_deck': 1, 'id':
/// 'CARD.CALL_OF_THE_VOID'})` (absent for the control),
/// `p['unlock_state']['unlocked_epochs'] = ['NECROBINDER1_EPOCH',
/// 'NECROBINDER2_EPOCH']`, and the `combat_orbs` stream of [`with_orb_stream`].
/// Oracle: pool `0e2d491d…` (72 cards), control `d46c357a…`, and the refusals
/// *"Call of the Void requires an explicit Necrobinder owner"*, *"... requires
/// a recorded card-pool unlock profile"*, *"CombatCardGeneration counter must
/// be an exact non-negative integer for Call of the Void"* and
/// *"CombatOrbGeneration counter unknown for this fight"*.
#[test]
fn a_call_of_the_void_deck_writes_its_owner_pool_and_refuses_by_name() {
    const EPOCHS: [&str; 2] = ["NECROBINDER1_EPOCH", "NECROBINDER2_EPOCH"];
    let necrobinder_with = |cards: &[&str], epochs: &[&str]| {
        let mut case = defect_with(cards, epochs);
        case["save"]["players"][0]["character_id"] = Value::from("CHARACTER.NECROBINDER");
        with_orb_stream(case)
    };
    let opening = open(&necrobinder_with(&["CALL_OF_THE_VOID"], &EPOCHS))
        .expect("a Call of the Void Necrobinder deck opens");
    let written = pool(&opening.document, "call_of_the_void_generation_pool");
    assert_eq!(written.len(), 72, "{written:?}");
    assert_eq!(written.first().map(String::as_str), Some("BLIGHT_STRIKE"));
    assert_eq!(written.last().map(String::as_str), Some("WISP"));
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "0e2d491dde41d9ed6247bc96b2c366c2d3997241c0ebf15ce369bac8f75fd5b8"
    );
    let plain = open(&necrobinder_with(&[], &EPOCHS)).expect("the plain Necrobinder opens");
    assert!(
        !plain
            .document
            .player
            .contains_key("call_of_the_void_generation_pool")
    );
    assert_eq!(
        plain.document.differential_digest(),
        "d46c357acf05497f190530d8bd81ce1a6ca8e98e655bbc329f8aed40daf9e966"
    );

    // A Defect owner is no longer refused (#3375): the pool is the Defect's
    // own, pinned in `a_listener_card_owned_by_any_character_writes_its_owners_pool`.
    assert_eq!(
        open(&necrobinder_with(&["CALL_OF_THE_VOID"], &[])).unwrap_err(),
        OpeningRefusal::ListenerPoolProvenance {
            source: "CALL_OF_THE_VOID",
            requires: "a recorded card-pool unlock profile",
        }
    );
    for stream in ["combat_card_generation", "combat_orbs"] {
        let mut absent = necrobinder_with(&["CALL_OF_THE_VOID"], &EPOCHS);
        absent["save"]["rng"]["rngs"]
            .as_object_mut()
            .expect("the save's streams are an object")
            .remove(stream);
        assert_eq!(
            open(&absent).unwrap_err(),
            OpeningRefusal::CombatStreamAbsent { stream },
            "{stream}"
        );
    }
}

/// Hello World, Call of the Void and Creative AI in ANY character's deck
/// (#3375): each opens, writes its OWNER's pool under the recorded profile,
/// and the document round-trips through `HotBoundary` and `engine::admit`.
///
/// `CreativeAiPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x338180` reads
/// `player.Character.CardPool` (IL_0044-IL_0055) under `player.UnlockState`
/// (IL_005a-IL_0060); `HelloWorldPower` `0x33bfd0` (IL_0062-IL_0091) and
/// `CallOfTheVoidPower` `0x336b08` (IL_0043-IL_0068) read the same. No body
/// names a character, so the frozen oracle's own-character owner gate was a
/// modeling limit, not native semantics. The pinned lengths are the owner
/// derivation under `TWO_DEFECT_EPOCHS`, which reveals no gating epoch of any
/// other character: an Ironclad's Creative AI pool is its Power rows minus
/// its epoch-gated ones.
#[test]
fn a_listener_card_owned_by_any_character_writes_its_owners_pool() {
    use crate::catalog::RewardPool;
    use crate::ids::PowerId;
    let owners = [
        ("CHARACTER.IRONCLAD", RewardPool::Ironclad),
        ("CHARACTER.SILENT", RewardPool::Silent),
        ("CHARACTER.DEFECT", RewardPool::Defect),
        ("CHARACTER.NECROBINDER", RewardPool::Necrobinder),
        ("CHARACTER.REGENT", RewardPool::Regent),
    ];
    let cards = [
        (
            "HELLO_WORLD",
            "hello_world_generation_pool",
            PowerId::HelloWorld,
        ),
        (
            "CALL_OF_THE_VOID",
            "call_of_the_void_generation_pool",
            PowerId::CallOfTheVoid,
        ),
        (
            "CREATIVE_AI",
            "creative_ai_generation_pool",
            PowerId::CreativeAi,
        ),
    ];
    let mut lengths = Vec::new();
    let mut engine_refusals: Vec<String> = Vec::new();
    for (character, owner) in owners {
        for (card, field, power) in cards {
            let mut case = with_orb_stream(defect_with(&[card], &TWO_DEFECT_EPOCHS));
            case["save"]["players"][0]["character_id"] = Value::from(character);
            let opening = open_with_reset_ledger(&case)
                .unwrap_or_else(|refusal| panic!("{character} {card} opens: {refusal}"));
            assert_eq!(
                opening.document.player["reward_card_pool"],
                Value::from(owner.as_str()),
                "{character}"
            );
            let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
                .unwrap_or_else(|refusal| panic!("{character} {card} catalog: {refusal}"));
            let expected: Vec<String> = catalog
                .owner_listener_pool(owner, power)
                .iter()
                .map(|id| id.as_str().to_string())
                .collect();
            assert!(!expected.is_empty(), "{character} {card}");
            if power == PowerId::CreativeAi {
                // The same one-type projection White Noise draws, whose
                // fully-unlocked rows are pinned as `ABUNDANCE_POWER_POOLS_V1101`.
                let white_noise = crate::steps::neutral::owner_type_generation_pool(
                    owner,
                    catalog.splash_unlock_epochs(),
                    crate::content_tables::CardType::Power,
                );
                assert_eq!(
                    catalog.owner_listener_pool(owner, power),
                    white_noise,
                    "{character}"
                );
            }
            assert_eq!(
                pool(&opening.document, field),
                expected,
                "{character} {card}"
            );
            for (other, other_field, _) in cards {
                if other != card {
                    assert!(
                        !opening.document.player.contains_key(other_field),
                        "{character} {card} writes no {other} pool"
                    );
                }
            }
            let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
                .unwrap_or_else(|refusal| panic!("{character} {card} admits: {refusal}"));
            if let Err(refusal) = crate::engine::admit(&opening.document, &state, &catalog) {
                engine_refusals.push(format!("{character} {card}: {refusal}"));
            }
            assert_eq!(
                crate::boundary::HotBoundary::to_canonical(&state, &catalog),
                opening.document,
                "{character} {card} round-trips"
            );
            lengths.push((character, card, expected.len()));
        }
    }
    assert_eq!(
        lengths,
        [
            ("CHARACTER.IRONCLAD", "HELLO_WORLD", 17),
            ("CHARACTER.IRONCLAD", "CALL_OF_THE_VOID", 69),
            ("CHARACTER.IRONCLAD", "CREATIVE_AI", 16),
            ("CHARACTER.SILENT", "HELLO_WORLD", 19),
            ("CHARACTER.SILENT", "CALL_OF_THE_VOID", 69),
            ("CHARACTER.SILENT", "CREATIVE_AI", 13),
            ("CHARACTER.DEFECT", "HELLO_WORLD", 18),
            ("CHARACTER.DEFECT", "CALL_OF_THE_VOID", 74),
            ("CHARACTER.DEFECT", "CREATIVE_AI", 18),
            ("CHARACTER.NECROBINDER", "HELLO_WORLD", 17),
            ("CHARACTER.NECROBINDER", "CALL_OF_THE_VOID", 69),
            ("CHARACTER.NECROBINDER", "CREATIVE_AI", 15),
            ("CHARACTER.REGENT", "HELLO_WORLD", 17),
            ("CHARACTER.REGENT", "CALL_OF_THE_VOID", 70),
            ("CHARACTER.REGENT", "CREATIVE_AI", 13),
        ]
    );
    // Until #3687 the Regent's two orb-reaching listeners refused here by
    // name, on the star/orb `AfterEnergyReset` ordering gate: the opened root
    // dropped the ledger, so it reloaded legacy-unknown. The opening now
    // emits the known-empty witness and all fifteen admit.
    assert_eq!(engine_refusals, Vec::<String>::new());
}

/// A saved Bone Tea charge reaches the opening (#2884, reached by #2827's
/// slot-7 freed fights).
///
/// The charge is `exact_finite_counter("RELIC.BONE_TEA", 1)`
/// (frozen Python `start_combat`, deleted #2827); the IL for the `0..=1` domain is on
/// `super::seeded_counters`. A charged copy upgrades the dealt Hand on turn
/// one and spends itself; a spent one does nothing; anything outside the
/// native domain refuses. Bellows beside it is admitted by both engines
/// (the oracle's only same-hook exception), in either acquisition order.
///
/// Digests printed by the oracle with the recipe above `empty_belt_case`,
/// appending `{'floor_added_to_deck': 1, 'id': 'RELIC.BONE_TEA', 'props':
/// {'ints': [{'name': 'CombatsLeft', 'value': N}]}}` (and, for the last two,
/// `{'floor_added_to_deck': 1, 'id': 'RELIC.BELLOWS'}` after or before it).
/// Before the seed the first case opened as `4b01ff0b…`, the spent-charge
/// document: the Hand upgrade was silently dropped.
#[test]
fn a_saved_bone_tea_charge_upgrades_the_turn_one_hand_like_the_oracle() {
    let bone_tea = |value: Option<i64>| {
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BONE_TEA"});
        if let Some(value) = value {
            row["props"] = serde_json::json!({"ints": [{"name": "CombatsLeft", "value": value}]});
        }
        row
    };
    let bellows = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BELLOWS"});
    for (label, rows, upgrade, digest) in [
        (
            "charged",
            vec![bone_tea(Some(1))],
            1,
            "2652d3a03f2d21d554417dfd9bc37c453f68853ff5d478a0f45c6dbe414ab807",
        ),
        (
            "spent",
            vec![bone_tea(Some(0))],
            0,
            "4b01ff0b3d9e58aac2379445566241270f253a4e82a88099cb7e59708a21e9cc",
        ),
        (
            "Bellows then Bone Tea",
            vec![bellows.clone(), bone_tea(Some(1))],
            1,
            "e2ae477e43fb72117d8da5cb8883f15e6594eecc702ed17fc8583db1882541c9",
        ),
        (
            "Bone Tea then Bellows",
            vec![bone_tea(Some(1)), bellows.clone()],
            1,
            "22bd76636b4eb29a98dfb7b4b7cee3b50632fd2e4dceb6d43d1c4653baa21b15",
        ),
    ] {
        let mut case = empty_belt_case();
        for row in rows {
            relic(&mut case, row);
        }
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{label} opens: {refusal}"));
        let hand = &opening.document.piles["hand"];
        assert_eq!(hand.len(), 5, "{label}");
        assert!(
            hand.iter().all(|card| card.upgrade == upgrade),
            "{label}: every dealt Hand card at level {upgrade}"
        );
        assert_eq!(
            opening.document.player.get("bone_tea_combats_left"),
            Some(&Value::from(0)),
            "{label}: the charge is spent (or was already)"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{label}"
        );
    }
    // The oracle refuses all three: `BONE_TEA persistent counter is not an
    // exact integer in 0..1`.
    for value in [None, Some(2), Some(-1)] {
        let mut case = empty_belt_case();
        relic(&mut case, bone_tea(value));
        let refusal = open(&case).expect_err("an inexact Bone Tea charge refuses");
        assert_eq!(refusal.class(), "relic_counter_not_exact", "{value:?}");
        assert!(refusal.to_string().contains("BONE_TEA"), "{refusal}");
    }
}

/// Kaiser Crab's `SurroundedPower` facing is seeded at `0` (#2827).
///
/// `start_combat` writes `kaiser_facing = 0` for exactly the `(CRUSHER,
/// ROCKET)` roster (frozen Python, deleted #2827); the IL for the zero-default
/// `_facing` is at the seed in `super::pre_hook_document`. Before it the
/// opening left the field at its `-1` default for the owned roster. The
/// digests were printed by the oracle with the recipe above
/// `empty_belt_case`, `encounter_id` set to `ENCOUNTER.KAISER_CRAB_BOSS` and,
/// for the second, `node_type` set to `boss` (which also moves the reward
/// odds field).
#[test]
fn a_kaiser_crab_opening_seeds_the_surrounded_facing_like_the_oracle() {
    for (node_type, digest) in [
        (
            None,
            "f0d41d2fca4caac2f683d133a7af225b3b45b6c581abc5d41d30a5a35910cc28",
        ),
        (
            Some("boss"),
            "791c148c60faeec0a120c5d76d16236a6a6732867b7d075fbf61d0114aa05fbc",
        ),
    ] {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from("ENCOUNTER.KAISER_CRAB_BOSS");
        if let Some(node_type) = node_type {
            case["node_type"] = Value::from(node_type);
        }
        let opening =
            open(&case).unwrap_or_else(|refusal| panic!("the Kaiser Crab case opens: {refusal}"));
        assert_eq!(
            monster_field(&opening.document, "kind"),
            vec![Value::from("CRUSHER"), Value::from("ROCKET")]
        );
        assert_eq!(
            opening.document.player.get("kaiser_facing"),
            Some(&Value::from(0))
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{node_type:?}"
        );
    }
    // And a roster without the pair keeps the elided `-1` default.
    let control = open(&empty_belt_case()).expect("the control opens");
    assert_eq!(control.document.player.get("kaiser_facing"), None);
}

// ---------------------------------------------------------------------------
// Physical-card uid allocation after the turn-one fixup (#2827)
// ---------------------------------------------------------------------------
//
// Digests printed by the oracle with the recipe above `empty_belt_case`, the
// named deck rows inserted at the FRONT of `save['players'][0]['deck']` (in
// the order listed) and the named relic rows appended to
// `save['players'][0]['relics']`. Every one of these decks is reordered by
// `_apply_turn_one_pile_fixups`, so on `main` before this change every one
// refused `card_identity_allocation_diverged`.

fn with_deck_front(rows: &[Value]) -> Value {
    let mut case = empty_belt_case();
    for row in rows.iter().rev() {
        deck_row(&mut case, row.clone());
    }
    case
}

fn deck_card(id: &str) -> Value {
    serde_json::json!({"floor_added_to_deck": 1, "id": format!("CARD.{id}")})
}

fn hand_ids_and_uids(document: &crate::canonical::CanonicalStateV2) -> Vec<(String, Option<u64>)> {
    document.piles["hand"]
        .iter()
        .map(|card| (card.id.clone(), card.uid))
        .collect()
}

/// One Innate card the shuffle left mid-pile: the oracle numbers the pile
/// AFTER the fixup moved it to the front.
///
/// Rows: `CARD.WRITHE`. The shuffle leaves Writhe (deck row 0) at pile
/// position 4; the fixup lifts it to the top, and the oracle's first
/// allocation — after the deal, since nothing earlier normalizes — gives it
/// uid 0. The pre-fixup stamping gave it 4 and refused.
#[test]
fn an_innate_card_the_shuffle_left_mid_pile_is_numbered_after_the_fixup() {
    let case = with_deck_front(&[deck_card("WRITHE")]);
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    assert_eq!(
        built.pile.iter().position(|row| *row == 0),
        Some(4),
        "the shuffle leaves Writhe mid-pile, so the fixup moves it"
    );
    assert_eq!(built.document.piles["draw"][0].id, "WRITHE");
    let opening = open(&case).expect("an Innate deck opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "35b8e3b326a401c793c597e3366d77858307a582a289a879c04d33eb2ad28338"
    );
    assert_eq!(
        hand_ids_and_uids(&opening.document)[0],
        ("WRITHE".to_string(), Some(0))
    );
}

/// Two Innate cards: the fixup reverses them onto the front, and the uids
/// follow that reversed order.
///
/// Rows: `CARD.WRITHE`, `CARD.BACKSTAB`. The oracle's hand opens
/// `BACKSTAB:0, WRITHE:1`; numbering by the Innate cards' shuffled order
/// instead would swap them.
#[test]
fn two_innate_cards_are_numbered_in_the_fixups_reversed_order() {
    let case = with_deck_front(&[deck_card("WRITHE"), deck_card("BACKSTAB")]);
    let opening = open(&case).expect("a two-Innate deck opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "1d1470d86efd71ca12e674fbb230a78f876ce66a8bb0b4b06f78f2d4467c856d"
    );
    assert_eq!(
        hand_ids_and_uids(&opening.document)[..2],
        [
            ("BACKSTAB".to_string(), Some(0)),
            ("WRITHE".to_string(), Some(1))
        ]
    );
}

/// #3178: a `ROYALLY_APPROVED` card is Innate (`RoyallyApproved::OnEnchant`
/// RVA `0xd62c9` IL_0007-0008 is `Card.AddKeyword(3)`), so the turn-one fixup
/// lifts it to the front, counts it in `innate_min_draw` and numbers it 0,
/// exactly as a canonical Innate row. Its bare twin stays where the shuffle
/// left it.
#[test]
fn a_royally_approved_card_is_lifted_by_the_turn_one_fixup() {
    let royal = with_deck_front(&[stone_row("ANGER", 0, Some(("ROYALLY_APPROVED", 1)))]);
    let built = pre_hook(&royal).expect("the pre-hook opening builds");
    let shuffled = built
        .pile
        .iter()
        .position(|row| *row == 0)
        .expect("deck row 0 is in the pile");
    assert_ne!(
        shuffled, 0,
        "the shuffle leaves it mid-pile, so the fixup moves it"
    );
    let top = &built.document.piles["draw"][0];
    assert_eq!(top.id, "ANGER");
    assert_eq!(
        top.enchantment,
        Some(serde_json::json!(["ROYALLY_APPROVED", 1]))
    );
    let opening = open(&royal).expect("a Royally Approved deck opens");
    assert_eq!(
        opening.document.player.get("innate_min_draw"),
        Some(&Value::from(1))
    );
    assert_eq!(
        hand_ids_and_uids(&opening.document)[0],
        ("ANGER".to_string(), Some(0))
    );

    let bare = with_deck_front(&[deck_card("ANGER")]);
    let built = pre_hook(&bare).expect("the bare control builds");
    assert_eq!(
        built.pile.iter().position(|row| *row == 0),
        Some(shuffled),
        "the enchantment does not move the shuffle"
    );
    assert_ne!(built.document.piles["draw"][0].id, "ANGER");
    let opening = open(&bare).expect("the bare control opens");
    assert_eq!(opening.document.player.get("innate_min_draw"), None);
}

/// A Ghost Seed owner is numbered over the SHUFFLED pile: its
/// `AfterRoomEntered` body allocates before the fixup, and the fixup then
/// carries the identified cards.
///
/// Rows: `CARD.WRITHE`; relic `RELIC.GHOST_SEED`. The oracle's hand opens
/// `WRITHE:4, STRIKE_IRONCLAD:0, …` — Writhe keeps its shuffled position as
/// its uid. The post-fixup numbering the Innate witnesses above use would
/// give it 0 here and still pass the prefix-draw check, which is why the
/// allocator relic decides the numbering rather than the check.
#[test]
fn a_ghost_seed_owner_keeps_the_shuffled_numbering_through_the_fixup() {
    let mut case = with_deck_front(&[deck_card("WRITHE")]);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.GHOST_SEED"}),
    );
    let opening = open(&case).expect("Ghost Seed with an Innate deck opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "448397019649c5445dc4ef7b015117cd60231d053dae3ec6474f15468396884c"
    );
    assert_eq!(
        hand_ids_and_uids(&opening.document)[..2],
        [
            ("WRITHE".to_string(), Some(4)),
            ("STRIKE_IRONCLAD".to_string(), Some(0))
        ]
    );
}

/// A Tea of Discourtesy owner is held to the strict `0..n-1` order, whatever
/// the fixup did (`expected_identity_order`).
///
/// Not reachable through `build` today — Tea refuses earlier as a turn-one
/// relic body with no subscriber — so the branch is witnessed on the two
/// functions directly. The oracle, for the record: with `CombatsLeft = 0`
/// and the Writhe row, Tea allocates nothing and the hand opens `WRITHE:0`
/// (`a55a448c…`); with `CombatsLeft = 1` it allocates over the shuffled pile
/// and opens `WRITHE:4` with two Dazed at uids 11 and 12 (`d5c4c1c5…`). The
/// strict order refuses the spent-Tea case's reordered Writhe rather than
/// read the counter, and the live one's two extra cards on the count.
#[test]
fn a_tea_of_discourtesy_owner_is_held_to_the_strict_order() {
    use super::{check_card_identity_allocation, expected_identity_order};
    let case = with_deck_front(&[deck_card("WRITHE")]);
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    let opened = open(&case).expect("the Innate deck opens").document;
    let strict: Vec<Option<u64>> = (0..11).map(Some).collect();
    assert_eq!(expected_identity_order(&built.document, &[], None), strict);
    assert_eq!(
        expected_identity_order(
            &built.document,
            &["RELIC.TEA_OF_DISCOURTESY".to_string()],
            None
        ),
        strict
    );

    // The Ghost Seed numbering (shuffled positions carried through the fixup)
    // passes its own expectation and fails the strict one at position 0.
    let mut ghost = case.clone();
    relic(
        &mut ghost,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.GHOST_SEED"}),
    );
    let ghost_pre = pre_hook(&ghost).expect("the Ghost Seed pre-hook builds");
    let ghost_opened = open(&ghost).expect("Ghost Seed opens").document;
    let carried = expected_identity_order(&ghost_pre.document, &[], None);
    assert_eq!(carried[0], Some(4));
    assert_eq!(
        check_card_identity_allocation(&ghost_opened, &carried, 0),
        Ok(())
    );
    assert!(matches!(
        check_card_identity_allocation(&ghost_opened, &strict, 0),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.starts_with("hand + draw position 0 carries uid Some(4)")
    ));
    // And a document with more live cards than the deck (Tea's two Dazed)
    // refuses. Its `next_card_uid` (11) says two uids were allocated past a
    // nine-card deck, which the fresh-tail arm admits only as the Hand's tail
    // in allocation order (uids 9, 10); they are not, so that arm refuses.
    // With the allocator claiming none past the deck, the count arm does.
    assert!(matches!(
        check_card_identity_allocation(&opened, &strict[..9], 0),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.contains("not the hand's tail")
    ));
    let mut unallocated = opened.clone();
    unallocated
        .player
        .insert("next_card_uid".to_string(), Value::from(9));
    assert!(matches!(
        check_card_identity_allocation(&unallocated, &strict[..9], 0),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail == "hand + draw holds 11 cards, the entering deck 9"
    ));
}

/// An `IMBUED` deck card is auto-played at turn one's AutoPre phase (#3381,
/// `engine::play::autoplay_imbued_turn_one`), and two refuse by name.
///
/// Rows: `CARD.DEFEND_IRONCLAD` with `{'id': 'ENCHANTMENT.IMBUED',
/// 'amount': 1}`. The frozen oracle played it (`b8a7ef53…`: `block` 5, the
/// Defend in the discard pile, `card_plays_finished_combat` 1), and so does
/// this opening, with the oracle's digest prefix. The played Defend leaves
/// `hand ++ draw`, which the uid allocation check allows for exactly that
/// card. The opened root admits: its Imbued listener has run, so the
/// enchantment is inert (`engine::play::imbued_identity_is_inert`).
#[test]
fn an_imbued_deck_card_is_auto_played_on_turn_one() {
    let imbued_defend = serde_json::json!({
        "floor_added_to_deck": 1, "id": "CARD.DEFEND_IRONCLAD",
        "enchantment": {"id": "ENCHANTMENT.IMBUED", "amount": 1}});
    let case = with_deck_front(std::slice::from_ref(&imbued_defend));
    let opening = open(&case).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    let document = &opening.document;
    assert_eq!(document.player.get("block"), Some(&Value::from(5)));
    let discard: Vec<_> = document.piles["discard"]
        .iter()
        .map(|card| (card.id.as_str(), card.enchantment.is_some()))
        .collect();
    assert_eq!(discard, [("DEFEND_IRONCLAD", true)]);
    assert!(
        document.differential_digest().starts_with("b8a7ef53"),
        "{}",
        document.differential_digest()
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(document, &catalog).unwrap();
    crate::engine::admit(document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the post-AutoPre root admits: {refusal}"));

    // #3392: native's opening checksum precedes `RunAutoPrePlayPhase`
    // (`CombatManager/<StartTurn>d__100` RVA `0x3f781c` IL_096f, then
    // IL_0b1e). A recording opening reports that state: the Imbued Defend not
    // yet played, the root itself unchanged.
    let recording = OpeningOptions {
        record_native_checkpoints: true,
        ..OpeningOptions::default()
    };
    let recorded = build_opening(&entry_facts(&case), &recording)
        .unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    assert_eq!(
        recorded.document,
        open_with_reset_ledger(&case).expect("opens").document
    );
    assert!(opening.native_checkpoints.is_none(), "recording is opt-in");
    let checkpoints = recorded.native_checkpoints.expect("recorded");
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].kind,
        crate::engine::native_checkpoint::NativeCheckpointKind::AfterPlayerTurnStart
    );
    let before = checkpoints[0].state.as_ref().expect("projects");
    assert_ne!(before.player.get("block"), Some(&Value::from(5)));
    assert!(before.piles.get("discard").is_none_or(Vec::is_empty));
    let unplayed = ["hand", "draw"]
        .into_iter()
        .filter_map(|pile| before.piles.get(pile))
        .flatten()
        .filter(|card| card.id == "DEFEND_IRONCLAD" && card.enchantment.is_some())
        .count();
    assert_eq!(unplayed, 1, "the Imbued Defend is still in hand ++ draw");

    // With no AutoPre work the checkpoint is the root's own state.
    let plain = build_opening(&entry_facts(&empty_belt_case()), &recording)
        .unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    let checkpoints = plain.native_checkpoints.as_ref().expect("recorded");
    assert_eq!(checkpoints.len(), 1);
    let before = checkpoints[0].state.as_ref().expect("projects");
    assert_eq!(before.piles, plain.document.piles);
    assert_eq!(before.rng, plain.document.rng);

    let two = with_deck_front(&[imbued_defend.clone(), imbued_defend]);
    assert_eq!(
        open(&two).unwrap_err(),
        OpeningRefusal::ImbuedAutoPlayNotModeled {
            card: "DEFEND_IRONCLAD".to_string(),
        }
    );
}

/// The fixture case as `character`'s deck holding `cards`, under the corpus's
/// own Necrobinder profile: every epoch this build declares except
/// `DEFECT7_EPOCH`, which is what all 24 corpus fights this slice freed carry.
///
/// Mutation: `p['character_id'] = character`, each card
/// `p['deck'].insert(0, {'floor_added_to_deck': 1, 'id': 'CARD.' + card})` in
/// the order given, and `p['unlock_state']['unlocked_epochs'] = ['EPOCH.' + e
/// for e in UNLOCK_EPOCH_UNIVERSE_V1101 if e != 'DEFECT7_EPOCH']`. The belt is
/// the fixture's own two-slot one, which this profile's potion pool admits.
fn owner_deck_with(character: &str, cards: &[&str]) -> Value {
    let mut case = case();
    case["save"]["players"][0]["character_id"] = Value::from(character);
    for card in cards {
        deck_row(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": format!("CARD.{card}")}),
        );
    }
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
        crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .filter(|epoch| **epoch != "DEFECT7_EPOCH")
            .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
            .collect(),
    );
    case
}

/// The Necrobinder owner-pool generators open and match the oracle (#2827).
///
/// Jackpot and Discovery draw from their owner's `CharacterCardPool`
/// (`Jackpot/<OnPlay>d__3::MoveNext` RVA `0x3a80d4`,
/// `Discovery/<OnPlay>d__4::MoveNext` RVA `0x399254` IL_0043..IL_0063), and
/// nothing about that pool is written into the combat-entry document: the
/// oracle's entry gates (frozen Python `start_combat`, deleted #2827) only decide whether the
/// fight roots, and its `entropy_card_pool` stays `None` for every owner but
/// Ironclad and Regent (`start_combat`). So the opening's only
/// Necrobinder question is the gate, and each document below is the oracle's
/// byte for byte. Digests printed by the oracle with the recipe above
/// `empty_belt_case`, starting from `case` rather than the empty-belt variant
/// and applying [`owner_deck_with`]'s mutation. Before this slice the three
/// generator decks refused as `generation_pool_unmodeled`; the plain deck is
/// the control.
#[test]
fn necrobinder_owner_pool_generators_open_and_match_the_oracle() {
    for (cards, digest) in [
        (
            &[][..],
            "5735b86708275c7059e70174b09f0c905a24a607ba170b0ae449e5fc5edcac0f",
        ),
        (
            &["JACKPOT"][..],
            "d65f2000958b8c1f47fbcc2af80fe5a42881c93f1cb3754fb2bbc5dfefa1a9b5",
        ),
        (
            &["DISCOVERY"][..],
            "1211078ccc4286234bf6a24a3b31cc7d5b75952e7a0936ba6087f1281a1970c9",
        ),
        (
            &["JACKPOT", "DISCOVERY"][..],
            "41cbc3e9ee80e25d1d6a4dc3bc4eb3688fd02fddc60c5e984ab4e6e7b5a8bd54",
        ),
    ] {
        let opening = open(&owner_deck_with("CHARACTER.NECROBINDER", cards))
            .unwrap_or_else(|refusal| panic!("{cards:?} opens: {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{cards:?}"
        );
        assert_eq!(
            opening.document.player.get("reward_card_pool"),
            Some(&Value::from("Necrobinder")),
            "{cards:?}"
        );
        // The oracle writes no Entropy owner provenance for a Necrobinder.
        assert!(
            !opening.document.player.contains_key("entropy_card_pool"),
            "{cards:?}"
        );
    }
}

/// What the Necrobinder slice deliberately does NOT widen, each pinned.
///
/// * The profile gate is reachable for a Necrobinder owner. Since #3336 it
///   refuses only an UNRECORDED profile: a partial one opens, because the
///   engine draws the Necrobinder pool that profile derives. The oracle
///   refused this partial profile on the Ironclad 2/5/7 epochs (*"Jackpot
///   requires the explicitly fully-unlocked owner card pool"*, frozen Python
///   `start_combat`, deleted #2827), which cannot move a Necrobinder row.
/// * Infernal Blade stays Ironclad-only, as the oracle's *"INFERNAL_BLADE
///   requires an explicit Ironclad owner"* (`start_combat`).
/// * Silent and Defect joined `MODELED_GENERATION_CHARACTERS` in #2827 item B;
///   `silent_and_defect_generators_open_and_match_the_oracle` is their parity
///   witness.
#[test]
fn the_necrobinder_slice_keeps_its_remaining_gates() {
    let mut partial = with_hidden_epochs(
        owner_deck_with("CHARACTER.NECROBINDER", &["JACKPOT"]),
        &["IRONCLAD7_EPOCH", "NECROBINDER7_EPOCH"],
    );
    open(&partial).expect("a recorded partial profile opens (#3336)");
    partial["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = serde_json::json!([]);
    let refusal = open(&partial).expect_err("an unrecorded profile refuses");
    assert_eq!(refusal.class(), "generation_pool_partial_unlock");
    assert!(refusal.to_string().contains("JACKPOT"), "{refusal}");

    assert_eq!(
        open(&owner_deck_with(
            "CHARACTER.NECROBINDER",
            &["INFERNAL_BLADE"]
        ))
        .unwrap_err(),
        OpeningRefusal::GenerationPoolUnmodeled {
            source: "INFERNAL_BLADE".to_string(),
            character: "CHARACTER.NECROBINDER".to_string(),
        }
    );
}

/// The opened Necrobinder document is ADMITTED by the solver (#2946; this pin
/// flipped when it landed).
///
/// Before #2946 the engine's Jackpot, Discovery and Calamity provenance
/// required `entropy_card_pool == reward_card_pool` and every card-pool gating
/// epoch of all six pools. The Necrobinder document carries neither — the
/// opening writes no Entropy owner for a Necrobinder, and every freed corpus
/// fight lacks `DEFECT7_EPOCH` — so each refused by name. Native reads only the
/// owner's pool through `GetUnlockedCards(Owner.UnlockState, ..)`
/// (`Jackpot/<OnPlay>d__3::MoveNext` `0x3a80d4` IL_010b,
/// `Discovery/<OnPlay>d__4::MoveNext` `0x399254` IL_0063,
/// `CalamityPower/<AfterCardPlayed>d__7::MoveNext` `0x3368bc` IL_0089), so a
/// recorded profile is the whole provenance
/// (`engine::cards::owner_pool_generation_provenance_is_exact`).
///
/// Jack of All Trades was the refusal control until the Colorless half of
/// #2560: it reads the COLORLESS pool (`JackOfAllTrades/<OnPlay>d__6::MoveNext`
/// `0x3a7ecc` IL_0026-IL_0046), and its body now shuffles
/// `Catalog::jack_of_all_trades_pool` under the recorded profile, so it admits
/// too. The refusal half is `an_unrecorded_profile_still_refuses_the_owner_pool_generators_by_name`.
#[test]
fn an_opened_necrobinder_generator_deck_is_admitted_on_its_recorded_profile() {
    for card in ["JACKPOT", "DISCOVERY", "CALAMITY", "JACK_OF_ALL_TRADES"] {
        let opening = open(&owner_deck_with("CHARACTER.NECROBINDER", &[card])).expect("opens");
        assert!(!opening.document.player.contains_key("entropy_card_pool"));
        let catalog =
            crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
        let state =
            crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
        assert!(
            catalog.splash_unlock_epochs().is_some(),
            "{card}: the profile is partial"
        );
        assert!(!state.fully_unlocked_card_pool_epochs, "{card}");
        crate::engine::admit(&opening.document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("{card}: {refusal}"));
    }
}

/// The Necrobinder Jack of All Trades and Calamity documents open and match
/// the oracle (#2947 review); since #2946 Calamity also admits.
///
/// `MODELED_GENERATION_CHARACTERS` gates every id in
/// `OWNER_POOL_GENERATION_CARDS`, not only Jackpot and Discovery, so admitting
/// Necrobinder also opens Jack of All Trades and Calamity for that owner. Both
/// documents are the oracle's byte for byte (digests printed by the oracle
/// with [`owner_deck_with`]'s mutation, as in the test above). Calamity reads
/// the owner's pool and admits on the recorded profile; Jack of All Trades
/// reads the Colorless pool and, since the Colorless half of #2560, admits on
/// it as well. Splash is deliberately
/// not pinned here: the oracle refuses it for every owner on this fixture
/// (`CombatOrbGeneration counter unknown`) while the Rust opening roots
/// Ironclad, Regent and Necrobinder alike, the pre-existing absent-stream gap
/// tracked as #2931.
#[test]
fn necrobinder_jack_and_calamity_open_and_match_the_oracle() {
    for (card, digest, missing) in [
        (
            "JACK_OF_ALL_TRADES",
            "5ffef2c1497b895c64b569d9b199bf92fd201f4d6c465a2792e397975a5f9f2c",
            None::<&str>,
        ),
        (
            "CALAMITY",
            "c664ae85721bf0e096adbffc20c78b0c39b3bdac67200cb9d7c61287fd19902c",
            None,
        ),
    ] {
        let opening = open(&owner_deck_with("CHARACTER.NECROBINDER", &[card]))
            .unwrap_or_else(|refusal| panic!("{card} opens: {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{card}"
        );
        let catalog =
            crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
        let state =
            crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
        let admitted = crate::engine::admit(&opening.document, &state, &catalog);
        match missing {
            Some(missing) => {
                let refusal = admitted.expect_err("the Colorless list is still frozen");
                assert!(
                    format!("{refusal:?}").contains(missing),
                    "{card}: {refusal:?}"
                );
            }
            None => {
                admitted.unwrap_or_else(|refusal| panic!("{card}: {refusal}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Turn-1 intents rolled during combat setup (#2827)
// ---------------------------------------------------------------------------
//
// Digests printed by the oracle with the recipe above `empty_belt_case`,
// `encounter_id` set as named. The `advanced` rows also replace
// `save['rng']['rngs']['monster_ai']` with the stream one draw on, as
// `sts2_rng.RunRngSet('ZPJHU3WSH2', {'MonsterAi': 1}, build='v0.111.0')`
// prints it: the fixture's counter-0 stream draws `NextFloat(2.0f)` =
// 0.8636 (the first branch) and the counter-1 stream 1.6497 (the second),
// so the two rows reach both arms of every kind's roll. On `main` before
// this change every one refused `initial_random_ai_not_modeled`.

/// The fixture's `MonsterAi` stream advanced to counter 1.
fn monster_ai_at_counter_one(case: &mut Value) {
    case["save"]["rng"]["rngs"]["monster_ai"] = serde_json::json!({
        "counter": 1,
        "s0": 2_980_984_770_716_981_116_u64,
        "s1": 6_782_460_650_723_619_195_u64,
        "s2": 4_327_062_275_640_979_698_u64,
        "s3": 9_861_800_941_660_424_172_u64,
    });
}

fn ai_counter(document: &crate::canonical::CanonicalStateV2) -> u64 {
    document.rng["ai"].counter
}

#[test]
fn the_advanced_monster_ai_stream_alone_moves_no_intent() {
    // The control for the mutation: Toadpoles roll nothing at setup, so the
    // counter-1 stream reaches the document untouched.
    let mut case = empty_belt_case();
    monster_ai_at_counter_one(&mut case);
    let opening = open(&case).expect("the counter-1 control opens");
    assert_eq!(ai_counter(&opening.document), 1);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "7346cb679572842c93b859cba940df98a81d4360e734dc04fde60fee407bb459"
    );
}

/// One row per INITIAL-RAND kind and stream position: the encounter, whether
/// the stream is advanced, the rolled creature's roster index, the move it
/// rolls, and the oracle's digest.
const INITIAL_RANDOM_AI_WITNESSES: [(&str, bool, usize, &str, &str); 8] = [
    (
        "ENCOUNTER.SLIMES_WEAK",
        false,
        0,
        "TACKLE_MOVE",
        "e9db37b712c01d1ce4e3643e47be4098281a90a03c888091246292650bdb10ca",
    ),
    (
        "ENCOUNTER.SLIMES_WEAK",
        true,
        0,
        "GOOP_MOVE",
        "6b265f8e2e7a3014a32c768fd794094aa76386226005674d33a12c73e67431b2",
    ),
    (
        "ENCOUNTER.EXOSKELETONS_NORMAL",
        false,
        3,
        "SKITTER_MOVE",
        "b53eb3874f4b40d63733037dae8b7d5873d041ac0641fa95c900c2c4e14fca68",
    ),
    (
        "ENCOUNTER.EXOSKELETONS_NORMAL",
        true,
        3,
        "MANDIBLES_MOVE",
        "31b0731f1c8845a4f8c8c25d8df29aa25e62760462c30ffcd11cfe6328e1aec9",
    ),
    (
        "ENCOUNTER.FLYCONID_NORMAL",
        false,
        1,
        "FRAIL_SPORES_MOVE",
        "611941a765207720b031c2530d37c91344728320cb0935ccd4e02c08090f9bf8",
    ),
    (
        "ENCOUNTER.FLYCONID_NORMAL",
        true,
        1,
        "SMASH_MOVE",
        "c8a9df4d6fd4fa56eb784a745ddb520d611392fd9fc252f348545cfc16bda6b5",
    ),
    (
        "ENCOUNTER.FABRICATOR_NORMAL",
        false,
        0,
        "FABRICATE_MOVE",
        "69d1d9dca1d6fa18c0424acb88b67a91c5df95a2f7dbca17a1e2230576667818",
    ),
    (
        "ENCOUNTER.FABRICATOR_NORMAL",
        true,
        0,
        "FABRICATING_STRIKE_MOVE",
        "f3b7fced679acae0b33eae3499ff7904bdbab77078e481f8c2ea650262db6f50",
    ),
];

#[test]
fn each_initial_random_ai_kind_rolls_its_turn_one_intent_like_the_oracle() {
    for (encounter, advanced, index, rolled, digest) in INITIAL_RANDOM_AI_WITNESSES {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from(encounter);
        if advanced {
            monster_ai_at_counter_one(&mut case);
        }
        let opening = open(&case)
            .unwrap_or_else(|refusal| panic!("{encounter} (advanced {advanced}) opens: {refusal}"));
        let monster = &opening.document.monsters[index];
        assert_eq!(
            monster.get("next_move"),
            Some(&Value::from(rolled)),
            "{encounter}"
        );
        assert_eq!(
            monster.get("move_log"),
            Some(&serde_json::json!([rolled])),
            "{encounter}"
        );
        // Exactly one `MonsterAi` draw, whichever position it started at.
        assert_eq!(
            ai_counter(&opening.document),
            if advanced { 2 } else { 1 },
            "{encounter}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{encounter} (advanced {advanced})"
        );
    }
}

/// Two enrolled creatures roll in roster order off one shared stream.
///
/// No registered roster holds two, so this drives the roll directly. The
/// draws are the fixture stream's first two `NextFloat(2.0f)` values as
/// `sts2_rng` prints them (0.8636, 1.6497): the first creature takes its
/// first branch and the second its second, and the stream ends at counter 2.
#[test]
fn two_initial_random_ai_creatures_roll_in_roster_order_off_one_stream() {
    use crate::encounters::MonsterSpec;
    use crate::ids::MonsterKind;
    let pre = pre_hook(&empty_belt_case()).expect("the control pre-hook builds");
    let mut rng = pre.rng.clone();
    assert_eq!(rng["ai"].counter, 0);
    let mut monsters = vec![
        MonsterSpec::new(MonsterKind::LeafSlimeS, 12),
        MonsterSpec::new(MonsterKind::TwigSlimeS, 8).slot(1),
        MonsterSpec::new(MonsterKind::LeafSlimeS, 13).slot(2),
    ];
    super::roll_initial_random_ai(&mut monsters, &mut rng).expect("both roll");
    assert_eq!(monsters[0].next_move, "TACKLE_MOVE");
    assert_eq!(monsters[0].move_log, &["TACKLE_MOVE"]);
    assert_eq!(monsters[1].next_move, "");
    assert_eq!(monsters[2].next_move, "GOOP_MOVE");
    assert_eq!(monsters[2].move_log, &["GOOP_MOVE"]);
    assert_eq!(rng["ai"].counter, 2);
}

/// Every refusal the roll keeps, and the stream left untouched by each.
///
/// A non-empty entering state is outside the empty-log weight reading, and a
/// Fabricator beside three more living enemies would take the DISINTEGRATE
/// follow (`get_CanFabricate` `0xb3d2c`), which no corpus roster reaches. A
/// dead creature is not enrolled at all, as `_initialize_random_ai` filters
/// `m.hp > 0`, and an Exoskeleton outside slot 3 is not either.
#[test]
fn the_initial_random_ai_roll_refuses_outside_its_exact_domain() {
    use crate::encounters::MonsterSpec;
    use crate::ids::MonsterKind;
    let pre = pre_hook(&empty_belt_case()).expect("the control pre-hook builds");
    let refusal = |kind: &'static str| OpeningRefusal::InitialRandomAiNotModeled { kind };

    let mut rng = pre.rng.clone();
    let mut entered = vec![
        MonsterSpec::new(MonsterKind::Flyconid, 50).opening_move("SMASH_MOVE", &["SMASH_MOVE"]),
    ];
    assert_eq!(
        super::roll_initial_random_ai(&mut entered, &mut rng),
        Err(refusal("FLYCONID"))
    );
    assert_eq!(rng, pre.rng);

    let mut crowded = vec![
        MonsterSpec::new(MonsterKind::Fabricator, 150).slot(2),
        MonsterSpec::new(MonsterKind::Zapbot, 20),
        MonsterSpec::new(MonsterKind::Stabbot, 20).slot(1),
        MonsterSpec::new(MonsterKind::Noisebot, 20).slot(3),
    ];
    assert_eq!(
        super::roll_initial_random_ai(&mut crowded, &mut rng),
        Err(refusal("FABRICATOR"))
    );
    assert_eq!(rng, pre.rng);
    // The same roster with one bot dead is back under the threshold.
    crowded[3].hp = 0;
    super::roll_initial_random_ai(&mut crowded, &mut rng).expect("three living enemies fabricate");
    assert_eq!(crowded[0].next_move, "FABRICATE_MOVE");
    assert_eq!(rng["ai"].counter, 1);

    let mut rng = pre.rng.clone();
    let mut not_enrolled = vec![
        MonsterSpec::new(MonsterKind::LeafSlimeS, 0),
        MonsterSpec::new(MonsterKind::Exoskeleton, 27).slot(2),
    ];
    super::roll_initial_random_ai(&mut not_enrolled, &mut rng).expect("nothing to roll");
    assert_eq!(not_enrolled[0].next_move, "");
    assert_eq!(not_enrolled[1].next_move, "");
    assert_eq!(rng, pre.rng);
}

/// Every enrolled kind's two branch indexes name, in the generated
/// random-move table, the moves its IL `AddBranch` calls add (tabulated on
/// `roll_initial_random_ai`).
#[test]
fn the_initial_random_ai_branches_name_the_il_moves() {
    use crate::encounters::MonsterSpec;
    use crate::ids::MonsterKind;
    for (spec, names) in [
        (
            MonsterSpec::new(MonsterKind::LeafSlimeS, 12),
            ["TACKLE_MOVE", "GOOP_MOVE"],
        ),
        (
            MonsterSpec::new(MonsterKind::Exoskeleton, 27).slot(3),
            ["SKITTER_MOVE", "MANDIBLES_MOVE"],
        ),
        (
            MonsterSpec::new(MonsterKind::Flyconid, 50),
            ["FRAIL_SPORES_MOVE", "SMASH_MOVE"],
        ),
        (
            MonsterSpec::new(MonsterKind::Fabricator, 150),
            ["FABRICATE_MOVE", "FABRICATING_STRIKE_MOVE"],
        ),
    ] {
        let branches = super::initial_random_ai_branches(&spec).expect("enrolled");
        let moves = crate::content_tables::random_moves(spec.kind).expect("a random-move table");
        assert_eq!(
            branches.map(|index| moves[usize::from(index)].name),
            names,
            "{:?}",
            spec.kind
        );
    }
}

// ---------------------------------------------------------------------------
// Stone Cracker's pre-draw upgrade (#2827)
// ---------------------------------------------------------------------------
//
// `super::stone_cracker_upgrades` carries the IL. Every digest below except
// the divergence witness's Rust-side one was printed by the oracle with the
// recipe above `empty_belt_case`, the save's `players[0]['deck']` replaced by
// the rows each test names (`{'floor_added_to_deck': 1, 'id': 'CARD.X'}`, plus
// `'current_upgrade_level': n` and `'enchantment': {'id': 'ENCHANTMENT.E',
// 'amount': a}` where given) and `players[0]['relics']` extended by
// `{'floor_added_to_deck': 1, 'id': 'RELIC.STONE_CRACKER'}`.
//
// Every deck here except the starter's has its upgradable cards pairwise
// distinct, so the oracle's first-equal-copy upgrade and native's
// chosen-object upgrade land on the same card and the two documents must be
// byte-identical. The starter deck is the divergence witness.

/// One saved deck row.
fn stone_row(id: &str, upgrade: i64, enchantment: Option<(&str, i64)>) -> Value {
    let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": format!("CARD.{id}")});
    if upgrade != 0 {
        row["current_upgrade_level"] = Value::from(upgrade);
    }
    if let Some((id, amount)) = enchantment {
        row["enchantment"] =
            serde_json::json!({"id": format!("ENCHANTMENT.{id}"), "amount": amount});
    }
    row
}

/// `n` copies of a plain saved row.
fn stone_rows(id: &str, upgrade: i64, n: usize) -> Vec<Value> {
    (0..n).map(|_| stone_row(id, upgrade, None)).collect()
}

/// The empty-belt case with its deck replaced and, when `stone_cracker`,
/// Stone Cracker appended to its relics.
fn stone_case(deck: Vec<Value>, stone_cracker: bool) -> Value {
    let mut case = empty_belt_case();
    case["save"]["players"][0]["deck"] = Value::Array(deck);
    if stone_cracker {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.STONE_CRACKER"}),
        );
    }
    case
}

/// Nine distinct upgradable cards, so the pool is the whole pile.
fn stone_distinct_deck() -> Vec<Value> {
    [
        "BASH",
        "STRIKE_IRONCLAD",
        "DEFEND_IRONCLAD",
        "ANGER",
        "IRON_WAVE",
        "POMMEL_STRIKE",
        "SHRUG_IT_OFF",
        "TWIN_STRIKE",
        "HEADBUTT",
    ]
    .into_iter()
    .map(|id| stone_row(id, 0, None))
    .collect()
}

fn upgrades(document: &crate::canonical::CanonicalStateV2, pile: &str) -> Vec<(String, i64)> {
    document.piles[pile]
        .iter()
        .map(|card| (card.id.clone(), card.upgrade))
        .collect()
}

fn sel_counter(document: &crate::canonical::CanonicalStateV2) -> u64 {
    document.rng["sel"].counter
}

/// The normal case: a nine-card pool, two upgraded, eight `Sel` draws (the
/// `StableShuffle`'s `n - 1`). The oracle's hand shows the two it took.
#[test]
fn stone_cracker_upgrades_two_of_a_distinct_pool_like_the_oracle() {
    let control = open(&stone_case(stone_distinct_deck(), false)).expect("the control opens");
    let opening = open(&stone_case(stone_distinct_deck(), true)).expect("Stone Cracker opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "e33d737fed5be369292f42f67c1a39777a0ba9f87fc6569ab55191e75413adad"
    );
    assert_eq!(sel_counter(&control.document), 0);
    assert_eq!(sel_counter(&opening.document), 8, "len(pool) - 1 draws");
    let upgraded: Vec<(String, i64)> = upgrades(&opening.document, "hand")
        .into_iter()
        .chain(upgrades(&opening.document, "draw"))
        .filter(|(_, upgrade)| *upgrade == 1)
        .collect();
    assert_eq!(
        upgraded,
        vec![
            ("DEFEND_IRONCLAD".to_string(), 1),
            ("STRIKE_IRONCLAD".to_string(), 1)
        ]
    );
    // Same pile order as the control: the upgrade moves no card.
    let ids = |document: &crate::canonical::CanonicalStateV2| -> Vec<String> {
        upgrades(document, "hand")
            .into_iter()
            .chain(upgrades(document, "draw"))
            .map(|(id, _)| id)
            .collect()
    };
    assert_eq!(ids(&opening.document), ids(&control.document));
}

/// Fewer than two: a one-card pool is upgraded with **no** `Sel` draw, and an
/// empty pool changes nothing. The two saves differ only in Bash's saved
/// level, so both documents are the same document — which is the point.
#[test]
fn stone_cracker_takes_what_a_short_pool_holds_and_draws_nothing() {
    let deck = |bash: i64| {
        let mut deck = stone_rows("STRIKE_IRONCLAD", 1, 5);
        deck.extend(stone_rows("DEFEND_IRONCLAD", 1, 4));
        deck.push(stone_row("BASH", bash, None));
        deck
    };
    const DIGEST: &str = "a47e5538b9bcc53ed1196681dc704b97ac0d9df3fcdc74f1b83f330074d3c82a";
    let one = open(&stone_case(deck(0), true)).expect("a one-card pool opens");
    let none = open(&stone_case(deck(1), true)).expect("an empty pool opens");
    let unowned = open(&stone_case(deck(0), false)).expect("the control opens");
    assert_eq!(one.document.differential_digest(), DIGEST);
    assert_eq!(none.document.differential_digest(), DIGEST);
    assert_eq!(sel_counter(&one.document), 0);
    assert_eq!(sel_counter(&none.document), 0);
    // And the relic did the work: without it, Bash stays at 0.
    assert_ne!(unowned.document.differential_digest(), DIGEST);
    assert!(
        upgrades(&unowned.document, "draw").contains(&("BASH".to_string(), 0)),
        "{:?}",
        upgrades(&unowned.document, "draw")
    );
}

/// Already-upgraded and unupgradable cards stay out of the pool: an
/// Ascender's Bane (`MaxUpgradeLevel` 0) and seven `+1` basics beside exactly
/// two upgradable cards. Both are taken, for one `Sel` draw.
#[test]
fn stone_cracker_skips_maxed_and_unupgradable_cards_like_the_oracle() {
    let mut deck = vec![stone_row("ASCENDERS_BANE", 0, None)];
    deck.extend(stone_rows("STRIKE_IRONCLAD", 1, 4));
    deck.extend(stone_rows("DEFEND_IRONCLAD", 1, 3));
    deck.push(stone_row("BASH", 0, None));
    deck.push(stone_row("ANGER", 0, None));
    let opening = open(&stone_case(deck, true)).expect("a mixed deck opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "a60ae01d980118fe8b2544fc81279bc07364576dce12875e98bac1ce006abc2a"
    );
    assert_eq!(sel_counter(&opening.document), 1);
    let all: Vec<(String, i64)> = upgrades(&opening.document, "hand")
        .into_iter()
        .chain(upgrades(&opening.document, "draw"))
        .collect();
    assert!(all.contains(&("ASCENDERS_BANE".to_string(), 0)));
    assert!(all.contains(&("BASH".to_string(), 1)));
    assert!(all.contains(&("ANGER".to_string(), 1)));
}

/// `exact = exact or _has_distinguishable_sort_ties(pile)`
/// (frozen Python `start_combat`, deleted #2827), one witness per way the post-upgrade pile can
/// need exact order that the entering deck did not. In each deck the one
/// upgradable card is a `+0` copy beside a `+1` copy of the same id, and every
/// other card is maxed (the rows named, then `DEFEND_IRONCLAD+1` x7 and
/// `BASH+1`), so the upgrade merges the two into one `(id, 1)` group:
///
/// * `sharp` — the two differ (a Sharp copy and a plain one);
/// * `rampage` — equal copies of a card whose copies diverge on play
///   (`engine::cards::divergent_physical_card`);
/// * `momentum` — equal copies carrying a diverging enchantment
///   (`engine::cards::divergent_identity_enchantment`).
///
/// Each deck's control, without the relic, keeps `exact_piles` unset.
#[test]
fn a_stone_cracker_upgrade_that_merges_distinguishable_copies_sets_exact_piles() {
    let filler = || {
        let mut rows = stone_rows("DEFEND_IRONCLAD", 1, 7);
        rows.push(stone_row("BASH", 1, None));
        rows
    };
    for (label, pair, control_digest, digest) in [
        (
            "sharp",
            vec![
                stone_row("STRIKE_IRONCLAD", 1, Some(("SHARP", 2))),
                stone_row("STRIKE_IRONCLAD", 0, None),
            ],
            "f5ea8a30a65a19d5080c6e14ac7b8c4d519e504db02f21ddfe91cc70f52ea906",
            "690e5fec3a06cdb61b0a22267b22346251e586d280930cbe6c3539b9ea026110",
        ),
        (
            "rampage",
            // The maxed Strike keeps `ps_strikes` off its zero default, which
            // the boundary refuses as non-canonical for a Strike-less deck.
            vec![
                stone_row("RAMPAGE", 1, None),
                stone_row("RAMPAGE", 0, None),
                stone_row("STRIKE_IRONCLAD", 1, None),
            ],
            "6cbc67bb5ac752c10127d6e553a437e5178b7894cd62956c24f71022fb54a0a5",
            "58f43ed56bfca553b031349f6598a42ec8cc89294415cf62cc08615912c92a82",
        ),
        (
            "momentum",
            vec![
                stone_row("STRIKE_IRONCLAD", 1, Some(("MOMENTUM", 1))),
                stone_row("STRIKE_IRONCLAD", 0, Some(("MOMENTUM", 1))),
            ],
            "b75c27b338576027d4548af9849e510f2339eefdaf44645ee253cb1f73e3ec0b",
            "56882de744ea8f3ee28e814ec35005596a1f9932129aa83f16259676ae19269e",
        ),
    ] {
        let deck: Vec<Value> = pair.into_iter().chain(filler()).collect();
        let unowned = open(&stone_case(deck.clone(), false))
            .unwrap_or_else(|refusal| panic!("{label} control opens: {refusal}"));
        assert_eq!(unowned.document.player.get("exact_piles"), None, "{label}");
        assert_eq!(
            unowned.document.differential_digest(),
            control_digest,
            "{label}"
        );
        let opening = open(&stone_case(deck, true))
            .unwrap_or_else(|refusal| panic!("{label} opens: {refusal}"));
        assert_eq!(
            opening.document.player.get("exact_piles"),
            Some(&Value::Bool(true)),
            "{label}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{label}"
        );
    }
}

/// An upgrade that grants Innate (`JUGGLING+1`) is written before the turn-1
/// fixup reads the pile, so the upgraded copy is lifted to the front and
/// dealt first, as in the oracle.
#[test]
fn a_stone_cracker_upgrade_that_grants_innate_is_dealt_first() {
    let mut deck = vec![stone_row("JUGGLING", 0, None)];
    deck.extend(stone_rows("STRIKE_IRONCLAD", 1, 5));
    deck.extend(stone_rows("DEFEND_IRONCLAD", 1, 4));
    deck.push(stone_row("BASH", 1, None));
    let opening = open(&stone_case(deck, true)).expect("the Juggling deck opens");
    assert_eq!(
        upgrades(&opening.document, "hand")[0],
        ("JUGGLING".to_string(), 1)
    );
    assert_eq!(
        opening.document.player.get("innate_min_draw"),
        Some(&Value::from(1))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "cf4af12545ac9a1f0dbb9eb49bda007030e192939dd9e0e1716676d119c0a31d"
    );
}

/// Ghost Seed's `AfterRoomEntered` Ethereal pass runs later, on the hot
/// state; the two commute (the oracle's own
/// `test_ghost_seed_and_stone_cracker_commute_on_exact_physical_payload`).
#[test]
fn stone_cracker_with_ghost_seed_matches_the_oracle() {
    let mut case = stone_case(stone_distinct_deck(), true);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.GHOST_SEED"}),
    );
    let opening = open(&case).expect("Stone Cracker with Ghost Seed opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "c7a769ab3bf8d7182a65163f3855d64f33829f1d40e8a5acb93ad7c744f69ffd"
    );
}

/// The divergence witness: the starter deck, whose pool holds equal copies.
///
/// The `StableShuffle` takes one Strike and one Defend. Native upgrades the
/// two **objects** `Take` returned (`<AfterRoomEntered>d__4::MoveNext`
/// IL_00a6-IL_00a9) — the Strike at pile position 3 and the Defend at 6. The
/// oracle upgrades `pile[pile.index(chosen)]`, the first equal copy of each
/// value (frozen Python `start_combat`, deleted #2827): positions 0 and 2. Same values, same
/// `Sel` advance, different cards.
///
/// The Rust digest is therefore **not** the oracle's
/// (`f8397d7a…`). It is the oracle's document with exactly those two upgrades
/// moved, digested by the oracle's own projector:
///
/// ```text
/// d = copy.deepcopy(doc)   # the oracle's document for this save
/// d['piles']['hand'][0]['upgrade'] = 0; d['piles']['hand'][2]['upgrade'] = 0
/// d['piles']['hand'][3]['upgrade'] = 1; d['piles']['draw'][1]['upgrade'] = 1
/// print(projector.differential_digest(d))
/// ```
///
/// That the object rule is the game's is measured, not argued: on the eleven
/// corpus captures holding Stone Cracker that have an encounter, the `.mcr`'s
/// first checksum (`After player turn start`, the game's own ordered Hand and
/// Draw with each card's upgrade level) equals Rust's opening on all eight
/// Rust opens, and differs from the oracle's on five of eleven, each time
/// only in which equal copy carries an upgrade (#2954). The walk file records
/// the ids.
#[test]
fn stone_cracker_upgrades_the_chosen_copy_not_the_first_equal_one() {
    let case = with_relic("RELIC.STONE_CRACKER");
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    assert_eq!(built.stone_cracker_upgraded, vec![6, 3]);
    let opening = open(&case).expect("the starter deck opens");
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "c9783371b0bde500f0d075c1aba701cf278b8f6338162e9915ce15dd452d9315"
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        "f8397d7a3241408e73c5928da519ea68cacafc299d2c6e1281d69a3ed171e7ed",
        "the oracle's first-equal-copy document"
    );
    let levels = |pile: &str| -> Vec<i64> {
        upgrades(&opening.document, pile)
            .into_iter()
            .map(|(_, upgrade)| upgrade)
            .collect()
    };
    assert_eq!(levels("hand"), vec![0, 0, 0, 1, 0]);
    assert_eq!(levels("draw"), vec![0, 1, 0, 0, 0]);
    assert_eq!(sel_counter(&opening.document), 9);
}

/// A pile card whose upgrade ladder the crate lacks refuses by name: its
/// `IsUpgradable` decides the pool's length and so the stream's advance.
/// Reached by editing the parsed entry, since the save parser only admits
/// known ids and in-domain levels.
#[test]
fn stone_cracker_over_an_unknown_ladder_refuses_by_name() {
    for (id, upgrade) in [
        ("CARD.NOT_A_CARD", 0),
        ("CARD.BASH", -1),
        ("CARD.BASH", 256),
    ] {
        let mut entry = entry_facts(&with_relic("RELIC.STONE_CRACKER"));
        entry.deck_entering[0].id = id.to_string();
        entry.deck_entering[0].upgrade_level = upgrade;
        let refusal = build_pre_hook(&entry).expect_err("an unknown ladder refuses");
        assert_eq!(
            refusal.class(),
            "stone_cracker_upgrade_ladder_unknown",
            "{id}+{upgrade}"
        );
    }
}

// ---------------------------------------------------------------------------
// Boundary representability on Python-rooted fights, and Slumbering Beetle
// (#2827, #2848)
// ---------------------------------------------------------------------------
//
// Digests printed by the oracle with the recipe above `empty_belt_case`, with
// the per-test mutation named in each doc comment.

/// A broke player's `gold` and a Strike-less deck's `ps_strikes` are both
/// their declared `FieldDefault::Int(0)`, and are elided like every other
/// default (#2827). Before this the opening wrote the `0`s and the boundary
/// refused the document as `NonCanonicalDefault` — ten and six corpus fights.
///
/// Mutations: `save['players'][0]['gold'] = 0`; and
/// `save['players'][0]['deck'] = [c for c in deck if 'STRIKE' not in c['id']]`.
#[test]
fn zero_gold_and_zero_strikes_are_elided_like_the_oracle() {
    let mut broke = empty_belt_case();
    broke["save"]["players"][0]["gold"] = Value::from(0);
    let opening = open(&broke).expect("a broke player opens");
    assert_eq!(opening.document.player.get("gold"), None);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "125dc8096e96d6efa24e31dc8b68a4a930ec2222b47feb31621d55a9a7c87fb9"
    );

    let mut strikeless = empty_belt_case();
    strikeless["save"]["players"][0]["deck"]
        .as_array_mut()
        .expect("the save's deck is an array")
        .retain(|card| !card["id"].as_str().unwrap_or("").contains("STRIKE"));
    let opening = open(&strikeless).expect("a Strike-less deck opens");
    assert_eq!(opening.document.player.get("ps_strikes"), None);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "200b62afee2ad7bf466b62eaf34a7abaf9bd02cc5c6c903fdfdfb28f7d7d373f"
    );

    // The control keeps both non-zero values on the wire.
    let control = open(&empty_belt_case()).expect("the control opens");
    assert_eq!(control.document.player.get("gold"), Some(&Value::from(99)));
    assert_eq!(
        control.document.player.get("ps_strikes"),
        Some(&Value::from(5))
    );
}

/// A saved Joss Paper's `CardsExhausted` reaches the opening (#2827).
///
/// The oracle's gate is frozen Python `start_combat`, deleted #2827; the IL for the `0..=4`
/// domain and the entry `EtherealCount` of `0` is on
/// `super::seeded_counters`. Before the seed an owner reached the boundary
/// with the `-1` "not owned" default and refused as "Batch 9 relic ownership
/// and mutable state disagree" — two corpus fights.
///
/// Mutation: append `{'floor_added_to_deck': 1, 'id': 'RELIC.JOSS_PAPER'}`,
/// with `'props': {'ints': [{'name': N, 'value': V}]}` where named. An
/// absent `props` is both properties at their `SaveIfNotTypeDefault` zero.
#[test]
fn a_saved_joss_paper_counter_is_seeded_like_the_oracle() {
    let joss = |props: Option<(&str, i64)>| {
        let mut row = serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JOSS_PAPER"});
        if let Some((name, value)) = props {
            row["props"] = serde_json::json!({"ints": [{"name": name, "value": value}]});
        }
        row
    };
    for (label, props, exhausted, digest) in [
        (
            "zeros omitted",
            None,
            0,
            "4045833d3ae3cfcf7b7e5ba87a059ccc25a1ba9fbb155684ebb04dee1efd0fa6",
        ),
        (
            "three exhausted",
            Some(("CardsExhausted", 3)),
            3,
            "79e245c0c65b8e48aabefec0a29c0cb44a2cd88d7e713cf2a361eb5ff09cc832",
        ),
        (
            "four exhausted",
            Some(("CardsExhausted", 4)),
            4,
            "01d1041b1b7c15da91e00f5ea2a68105c47be3b264a0dc48e25f554fa5563927",
        ),
    ] {
        let mut case = empty_belt_case();
        relic(&mut case, joss(props));
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{label} opens: {refusal}"));
        assert_eq!(
            opening.document.player.get("joss_paper_cards_exhausted"),
            Some(&Value::from(exhausted)),
            "{label}"
        );
        assert_eq!(
            opening.document.player.get("joss_paper_ethereal_count"),
            None,
            "{label}: the entry EtherealCount is the elided 0"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{label}"
        );
    }
    // The oracle refuses all three: `JOSS_PAPER requires exact saved
    // CardsExhausted 0..4 and entry EtherealCount 0`.
    for props in [
        ("CardsExhausted", 5),
        ("CardsExhausted", -1),
        ("EtherealCount", 1),
    ] {
        let mut case = empty_belt_case();
        relic(&mut case, joss(Some(props)));
        let refusal = open(&case).expect_err("an inexact Joss Paper counter refuses");
        assert_eq!(refusal.class(), "relic_counter_not_exact", "{props:?}");
        assert!(refusal.to_string().contains("JOSS_PAPER"), "{refusal}");
    }
}

/// A saved Iron Club `CardsPlayed` opens as its residue modulo the native
/// `Cards` 4 (#3228).
///
/// The IL is on `super::IRON_CLUB_CARDS`: the saved counter is a lifetime
/// count native never resets, and the draw fires at `CardsPlayed % 4 == 0`.
/// The seed read modulo 3 before, so the corpus values 35, 87 and 147 opened
/// as 2, 0 and 0 instead of 3, 3 and 3 and the first card play's draw was
/// missed (HCJ8F13QA6B3 nodes 38 and 45). The rows below separate the two
/// moduli everywhere they disagree.
///
/// Mutation: append `{'floor_added_to_deck': 1, 'id': 'RELIC.IRON_CLUB',
/// 'props': {'ints': [{'name': 'CardsPlayed', 'value': V}]}}`.
#[test]
fn a_saved_iron_club_counter_opens_modulo_its_native_cards() {
    let iron_club = |saved: i64| {
        serde_json::json!({
            "floor_added_to_deck": 1,
            "id": "RELIC.IRON_CLUB",
            "props": {"ints": [{"name": "CardsPlayed", "value": saved}]},
        })
    };
    for (saved, seeded) in [
        (0_i64, 0_i64),
        (3, 3),
        (4, 0),
        (5, 1),
        (35, 3),
        (87, 3),
        (147, 3),
    ] {
        let mut case = empty_belt_case();
        relic(&mut case, iron_club(saved));
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{saved} opens: {refusal}"));
        assert_eq!(
            opening.document.player.get("iron_club_cards"),
            Some(&Value::from(seeded)),
            "saved CardsPlayed {saved}"
        );
        assert_eq!(seeded, saved.rem_euclid(super::IRON_CLUB_CARDS));
    }
    // A negative saved count is not one native can write; it refuses.
    let mut case = empty_belt_case();
    relic(&mut case, iron_club(-1));
    let refusal = open(&case).expect_err("a negative Iron Club counter refuses");
    assert_eq!(refusal.class(), "relic_counter_not_exact");
}

/// `SLUMBERING_BEETLE_NORMAL` opens through `build_slumbering_beetle_normal`
/// and its `override = "SNORE"` text field (#2848).
///
/// Mutation: `encounter_id = 'ENCOUNTER.SLUMBERING_BEETLE_NORMAL'`, and for
/// the second case also `save['ascension'] = 1`, below the A8 gate on the HP
/// bands and on `PlatingAmount`.
///
/// The documents differ from the oracle's at exactly one field, and that is
/// Python's error, not Rust's: the beetle's round-one `block`. Monster
/// `PlatingPower` grants its Amount as Block at the player side's round-one
/// start on v0.111.0, which Rust models and frozen Python does not (#2809;
/// the same class as the Frog Knight, Sewer Clam and Lagavulin Matriarch rows
/// of the opening census). So the witness is two-part: with that one field
/// removed the digest IS the oracle's, and the field itself equals the
/// Plating Amount.
#[test]
fn a_slumbering_beetle_opens_with_its_snore_state_like_the_oracle() {
    for (ascension, plating, oracle_digest) in [
        (
            None,
            18,
            "10d8a70b65e5c8c1433f90c90c6ddc4b51548eaf0c3eddfee071828ec8214936",
        ),
        (
            Some(1),
            15,
            "7a6226bd16a4b2720eafbe17475db23190e56fdf8aed1e2c89bc6623597e26bd",
        ),
    ] {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from("ENCOUNTER.SLUMBERING_BEETLE_NORMAL");
        if let Some(ascension) = ascension {
            case["save"]["ascension"] = Value::from(ascension);
        }
        let opening = open(&case)
            .unwrap_or_else(|refusal| panic!("the beetle opens at {ascension:?}: {refusal}"));
        let document = &opening.document;
        assert_eq!(
            monster_field(document, "kind"),
            vec![
                Value::from("BOWLBUG_ROCK"),
                Value::from("BOWLBUG_SILK"),
                Value::from("SLUMBERING_BEETLE")
            ]
        );
        let beetle = &document.monsters[2];
        assert_eq!(beetle.get("override"), Some(&Value::from("SNORE")));
        assert_eq!(beetle.get("mplating"), Some(&Value::from(plating)));
        assert_eq!(beetle.get("slumber"), Some(&Value::from(3)));
        // #2809: Rust's round-one Plating Block, absent from the oracle's.
        assert_eq!(beetle.get("block"), Some(&Value::from(plating)));
        let mut without_block = document.clone();
        without_block.monsters[2].remove("block");
        assert_eq!(
            without_block.differential_digest(),
            oracle_digest,
            "{ascension:?}: everything but #2809's Block is the oracle's"
        );
    }
}

/// `AEONGLASS_BOSS` opens with its Withering Presence seeded and with
/// whichever `exact_piles` the deck rule gives, like the oracle (#2957).
///
/// Mutation: `encounter_id = 'ENCOUNTER.AEONGLASS_BOSS'`,
/// `node_type = 'boss'`; the second case also sets `save['ascension'] = 1`
/// (below the A8 HP gate), and the third inserts
/// `{'floor_added_to_deck': 1, 'id': 'CARD.STRIKE_IRONCLAD', 'enchantment':
/// {'id': 'ENCHANTMENT.SHARP', 'amount': 2}}` at the head of the deck, a
/// mixed-payload Strike group the oracle's deck rule marks exact. The oracle
/// writes `withering_cards_left` 6, `after_card_played_power_order`
/// `[["withering", 0]]` and `next_after_side_turn_end_power_uid` 1 in all
/// three, and `exact_piles` only in the third. Before #2957 the first two
/// refused at the boundary's Aeonglass gate, which demanded exact piles the
/// oracle never sets at combat start.
#[test]
fn an_aeonglass_opens_with_withering_presence_like_the_oracle() {
    for (label, ascension, sharp_strike, exact_piles, hp, digest) in [
        (
            "A10",
            None,
            false,
            None,
            535,
            "35b3f0949357482f6ec33e1a34f49042571f31fcdb1f5658dc7788b2b56e3412",
        ),
        (
            "A1",
            Some(1),
            false,
            None,
            512,
            "6925508e42f949e56e684a2cf442f8513df0cc50cb4496865a10c7b2d827499a",
        ),
        (
            "exact deck",
            None,
            true,
            Some(true),
            535,
            "dd57e92ccdba27f185894b455303ad4f91bda37a62d813896c258d7251eb715a",
        ),
    ] {
        let mut case = empty_belt_case();
        case["encounter_id"] = Value::from("ENCOUNTER.AEONGLASS_BOSS");
        case["node_type"] = Value::from("boss");
        if let Some(ascension) = ascension {
            case["save"]["ascension"] = Value::from(ascension);
        }
        if sharp_strike {
            deck_row(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": "CARD.STRIKE_IRONCLAD",
                    "enchantment": {"id": "ENCHANTMENT.SHARP", "amount": 2}}),
            );
        }
        let opening =
            open(&case).unwrap_or_else(|refusal| panic!("{label}: the Aeonglass opens: {refusal}"));
        let player = &opening.document.player;
        assert_eq!(
            player.get("withering_cards_left"),
            Some(&Value::from(6)),
            "{label}"
        );
        assert_eq!(
            player.get("after_card_played_power_order"),
            Some(&serde_json::json!([["withering", 0]])),
            "{label}"
        );
        assert_eq!(
            player.get("next_after_side_turn_end_power_uid"),
            Some(&Value::from(1)),
            "{label}"
        );
        assert_eq!(
            player.get("exact_piles"),
            exact_piles.map(Value::from).as_ref(),
            "{label}"
        );
        assert_eq!(
            monster_field(&opening.document, "hp"),
            vec![Value::from(hp)],
            "{label}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{label}"
        );
    }
}

/// The fixture case with a Vexing Puzzlebox, for `character`, under
/// [`owner_deck_with`]'s profile and with [`with_orb_stream`]'s stream.
fn puzzlebox_case(character: &str) -> Value {
    let mut case = with_orb_stream(owner_deck_with(character, &[]));
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.VEXING_PUZZLEBOX"}),
    );
    case
}

/// Vexing Puzzlebox draws from its OWNER's pool, and its turn-one card takes
/// the first uid past the entering deck at the Hand's tail (#2827).
///
/// Mutation: [`owner_deck_with`]`(character, [])`, then
/// `p['relics'].append({'floor_added_to_deck': 1, 'id':
/// 'RELIC.VEXING_PUZZLEBOX'})` and the `combat_orbs` stream of
/// [`with_orb_stream`]; the refusal cases then delete one stream, or remove
/// `EPOCH.NECROBINDER{2,5,7}_EPOCH` from the profile. Every digest printed by
/// the oracle with the recipe above `empty_belt_case`, starting from `case`.
///
/// * Necrobinder and Ironclad open and match byte for byte; the generated
///   card is `BONE_SHARDS` / `ASHEN_STRIKE` at uid 10, the deck being uids
///   `0..9`, so `check_card_identity_allocation`'s fresh-tail arm is what
///   admits them.
/// * Without `combat_orbs` or `combat_card_generation` the oracle refuses
///   (*"CombatOrbGeneration counter unknown"*, *"CombatCardGeneration counter
///   must be an exact non-negative integer for Vexing Puzzlebox"*), and so
///   does this opening, by stream name.
/// * A profile hiding the owner's gating epochs: the oracle roots it
///   (`5079f827…`) with the fully unlocked pool, native reads the owner's
///   unlocked pool (`VexingPuzzlebox/<AfterPlayerTurnStart>d__2::MoveNext`
///   `0x333de0` IL_005d-IL_0082), and this refuses rather than choose — a
///   narrowing.
#[test]
fn a_vexing_puzzlebox_opens_for_its_owner_like_the_oracle() {
    for (character, card, digest) in [
        (
            "CHARACTER.NECROBINDER",
            "BONE_SHARDS",
            "3b6e50c98c141ac557d44b82945a4b129eeb53fe8931226d1903812caab7600d",
        ),
        (
            "CHARACTER.IRONCLAD",
            "ASHEN_STRIKE",
            "d366e9c28cb2560c491934b0c1c990647c8d1b31fc89ee047b4e1b58dcc91d02",
        ),
    ] {
        let opening = open(&puzzlebox_case(character))
            .unwrap_or_else(|refusal| panic!("{character}: the Puzzlebox opens: {refusal}"));
        assert_eq!(
            hand_ids_and_uids(&opening.document).last(),
            Some(&(card.to_string(), Some(10))),
            "{character}"
        );
        assert_eq!(
            opening.document.player.get("next_card_uid"),
            Some(&Value::from(11)),
            "{character}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{character}"
        );
    }

    for stream in ["combat_orbs", "combat_card_generation"] {
        let mut case = puzzlebox_case("CHARACTER.NECROBINDER");
        case["save"]["rng"]["rngs"]
            .as_object_mut()
            .expect("the save's streams are an object")
            .remove(stream);
        assert_eq!(
            open(&case).unwrap_err(),
            OpeningRefusal::CombatStreamAbsent { stream },
            "{stream}"
        );
    }

    let mut hidden = puzzlebox_case("CHARACTER.NECROBINDER");
    hidden["save"]["players"][0]["unlock_state"]["unlocked_epochs"]
        .as_array_mut()
        .expect("the profile is an array")
        .retain(|epoch| {
            !matches!(
                epoch.as_str(),
                Some(
                    "EPOCH.NECROBINDER2_EPOCH"
                        | "EPOCH.NECROBINDER5_EPOCH"
                        | "EPOCH.NECROBINDER7_EPOCH"
                )
            )
        });
    let refusal = open(&hidden).expect_err("a hidden owner profile refuses");
    assert_eq!(refusal.class(), "engine_not_implemented");
    assert!(
        refusal
            .to_string()
            .contains("Vexing Puzzlebox owner pool under a profile hiding"),
        "{refusal}"
    );
}

/// `check_card_identity_allocation`'s fresh-tail arm (#2827): every uid the
/// document allocated past the entering deck must be the Hand's tail, in
/// allocation order. Witnessed on the Necrobinder Puzzlebox document above
/// (uid 10 at the tail) by moving that card to the Hand's front, and by
/// claiming one more allocation than the Hand carries.
#[test]
fn a_fresh_uid_off_the_hands_tail_refuses() {
    use super::{check_card_identity_allocation, expected_identity_order};
    let case = puzzlebox_case("CHARACTER.NECROBINDER");
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    let opened = open(&case).expect("the Puzzlebox opens").document;
    let expected = expected_identity_order(&built.document, &[], None);
    assert_eq!(
        check_card_identity_allocation(&opened, &expected, 0),
        Ok(())
    );

    let mut moved = opened.clone();
    let hand = moved.piles.get_mut("hand").expect("a hand");
    let fresh = hand.pop().expect("the generated card");
    hand.insert(0, fresh);
    assert!(matches!(
        check_card_identity_allocation(&moved, &expected, 0),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.contains("not the hand's tail")
    ));

    let mut overclaimed = opened.clone();
    overclaimed
        .player
        .insert("next_card_uid".to_string(), Value::from(12));
    assert!(matches!(
        check_card_identity_allocation(&overclaimed, &expected, 0),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.contains("not the hand's tail")
    ));
}

/// The fixture case with a Crossbow, for `character`, under
/// [`owner_deck_with`]'s profile (or every declared epoch when `full`) and
/// with [`with_orb_stream`]'s stream.
fn crossbow_case(character: &str, full: bool) -> Value {
    let mut case = with_orb_stream(owner_deck_with(character, &[]));
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.CROSSBOW"}),
    );
    if full {
        case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
            crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
                .collect(),
        );
    }
    case
}

/// Crossbow's turn-start body draws the OWNER's unlocked Attack pool (#2827,
/// #2970); Big Hat's stays Ironclad-only (#3264).
///
/// Mutation: [`crossbow_case`]'s — `owner_deck_with(character, [])`,
/// `p['relics'].append({'floor_added_to_deck': 1, 'id': 'RELIC.CROSSBOW'})`,
/// the `combat_orbs` stream of [`with_orb_stream`], and for `full` every
/// epoch of `UNLOCK_EPOCH_UNIVERSE_V1101`. Oracle output, recipe above
/// `empty_belt_case`, starting from `case`:
///
/// * Ironclad, full profile: roots `1f70a8e5…`, generating `TEAR_ASUNDER` at
///   uid 10 from the frozen `INFERNAL_BLADE_ATTACK_POOL_V109`. This opens
///   and matches byte for byte.
/// * Necrobinder (`ca2f5a61…`, `SNAP`) and Ironclad without `DEFECT7_EPOCH`
///   (`2bc1341b…`, `TEAR_ASUNDER`): the oracle roots both from the owner's
///   Attack pool, which is what native reads
///   (`Crossbow/<AfterSideTurnStart>d__2::MoveNext` `0x322340`,
///   IL_003d-IL_00c7) and what `engine::relics::crossbow_after_side_turn_start`
///   now draws. Both open and match the oracle's digest prefix (the frozen
///   record keeps eight hex digits of each), where before #2970 both refused.
/// * Full profile with no `combat_card_generation` stream, or no
///   `combat_orbs` stream: the oracle refuses (*"CombatCardGeneration counter
///   must be an exact non-negative integer for Crossbow"*, *"CombatOrbGeneration
///   counter unknown for this fight (consumers: generation-relic card
///   pools)"*), and so does this opening, by stream name
///   (`generation_relic_streams_are_recorded`). Before that gate the first
///   case OPENED, drawing from a stream the save never recorded.
/// * Big Hat, full profile: an Ironclad roots `85ee82a2…` with or without
///   `combat_orbs` (its Ethereal pool is empty, so the oracle counts no
///   source) and opens here byte for byte either way. A Necrobinder roots
///   `fe45b672…` generating `LETHALITY` and `SEANCE`; a draft of the native
///   owner-Ethereal draw (`BigHat/<AfterSideTurnStart>d__6::MoveNext`
///   `0x31f64c`) generated those two cards but not that digest, so it still
///   refuses by name (#3264), and without `combat_orbs` both sides refuse on
///   the stream.
///
/// Every opened document also loads and admits (`engine::admit`), so the
/// widened Crossbow admission provenance and closure are witnessed on the
/// same roots.
#[test]
fn crossbow_draws_the_owner_attack_pool_and_names_the_rest() {
    let admits = |opening: &Opening, label: &str| {
        let document = &opening.document;
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document)
            .unwrap_or_else(|refusal| panic!("{label}: catalog: {refusal}"));
        let state = crate::boundary::HotBoundary::from_canonical(document, &catalog)
            .unwrap_or_else(|refusal| panic!("{label}: hydrate: {refusal}"));
        crate::engine::admit(document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("{label}: admit: {refusal}"));
    };
    let opening =
        open(&crossbow_case("CHARACTER.IRONCLAD", true)).expect("a full Ironclad Crossbow opens");
    assert_eq!(
        hand_ids_and_uids(&opening.document).last(),
        Some(&("TEAR_ASUNDER".to_string(), Some(10)))
    );
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        "1f70a8e5d2af5fd26aa40e2a786bd4039d1522cfaddad42342e1a1893984576d"
    );
    admits(&opening, "full Ironclad Crossbow");

    for (character, card, digest) in [
        ("CHARACTER.NECROBINDER", "SNAP", "ca2f5a61"),
        ("CHARACTER.IRONCLAD", "TEAR_ASUNDER", "2bc1341b"),
    ] {
        let opening = open(&crossbow_case(character, false))
            .unwrap_or_else(|refusal| panic!("{character} Crossbow opens: {refusal}"));
        assert_eq!(
            hand_ids_and_uids(&opening.document)
                .last()
                .map(|(id, _)| id.as_str()),
            Some(card),
            "{character}"
        );
        let got = python_view(&opening.document).differential_digest();
        assert!(got.starts_with(digest), "{character}: {got}");
        admits(&opening, character);
    }

    for stream in ["combat_card_generation", "combat_orbs"] {
        let mut absent = crossbow_case("CHARACTER.IRONCLAD", true);
        absent["save"]["rng"]["rngs"]
            .as_object_mut()
            .expect("the save's streams are an object")
            .remove(stream);
        assert_eq!(
            open(&absent).unwrap_err(),
            OpeningRefusal::CombatStreamAbsent { stream },
            "{stream}"
        );
    }

    let big_hat = |character: &str, orbs: bool| {
        let mut case = owner_deck_with(character, &[]);
        if orbs {
            case = with_orb_stream(case);
        }
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BIG_HAT"}),
        );
        case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
            crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
                .collect(),
        );
        case
    };
    for orbs in [true, false] {
        let opening = open(&big_hat("CHARACTER.IRONCLAD", orbs))
            .unwrap_or_else(|refusal| panic!("an Ironclad Big Hat opens ({orbs}): {refusal}"));
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            "85ee82a27b059e95d97a8e7d3f9b1982724d2c443fde1e76183f2d82cfba7e0e",
            "{orbs}"
        );
        admits(&opening, "Ironclad Big Hat");
    }
    let refusal = open(&big_hat("CHARACTER.NECROBINDER", true)).expect_err("refuses");
    assert_eq!(refusal.class(), "engine_not_implemented");
    assert!(
        refusal
            .to_string()
            .contains("Big Hat owner Ethereal pool beyond Ironclad (#3264)"),
        "{refusal}"
    );
    assert_eq!(
        open(&big_hat("CHARACTER.NECROBINDER", false)).unwrap_err(),
        OpeningRefusal::CombatStreamAbsent {
            stream: "combat_orbs"
        }
    );
}

/// Crossbow's same-`AfterSideTurnStart` peers at admission (#2970).
///
/// The five peers whose bodies commute with Crossbow's (the IL is in
/// `engine::admission`'s comment above the gate) admit beside it, alone and
/// together, on a partial-profile Ironclad root: the 1KJJGR1GFZR6 shape, which
/// holds Brimstone, Happy Flower and Candelabra before its Crossbow and a
/// Lantern after it. Pael's Legion is the Necrobinder run's (WV9GZBQ50ZHH)
/// peer. Every other same-hook peer still refuses by the acquisition-order
/// name (Chandelier stands for them). Brimstone beside a reachable Arsenal,
/// whether the Arsenal is a card in the deck or an Arsenal power already
/// live, admits in either vouched order and refuses by its own name on an
/// unvouched inventory (#3381).
#[test]
fn crossbow_admits_its_commuting_turn_start_peers_and_names_the_rest() {
    let admit = |case: &Value| {
        let opening = open(case).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
        let document = &opening.document;
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document).unwrap();
        let state = crate::boundary::HotBoundary::from_canonical(document, &catalog).unwrap();
        crate::engine::admit(document, &state, &catalog).map_err(|refusal| {
            refusal
                .missing()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
    };
    let with = |character: &str, relics: &[&str], cards: &[&str]| {
        let mut case = crossbow_case(character, false);
        for id in relics {
            let mut row =
                serde_json::json!({"floor_added_to_deck": 1, "id": format!("RELIC.{id}")});
            // The flower's saved counter is run state the opening seeds.
            if *id == "HAPPY_FLOWER" {
                row["props"] = serde_json::json!({"ints": [{"name": "TurnsSeen", "value": 1}]});
            }
            relic(&mut case, row);
        }
        for id in cards {
            deck_row(
                &mut case,
                serde_json::json!({"floor_added_to_deck": 1, "id": format!("CARD.{id}")}),
            );
        }
        case
    };
    for peers in [
        &["BRIMSTONE"][..],
        &["CANDELABRA"],
        &["HAPPY_FLOWER"],
        &["LANTERN"],
        &["BRIMSTONE", "HAPPY_FLOWER", "CANDELABRA", "LANTERN"],
    ] {
        admit(&with("CHARACTER.IRONCLAD", peers, &[]))
            .unwrap_or_else(|missing| panic!("{peers:?}: {missing:?}"));
    }
    admit(&with("CHARACTER.NECROBINDER", &["PAELS_LEGION"], &[]))
        .unwrap_or_else(|missing| panic!("Pael's Legion: {missing:?}"));

    let order = "argument shape at Crossbow AfterSideTurnStart acquisition order";
    let missing = admit(&with("CHARACTER.IRONCLAD", &["CHANDELIER"], &[]))
        .expect_err("an unproven peer still refuses");
    assert!(missing.iter().any(|item| item == order), "{missing:?}");

    // Brimstone beside a reachable Arsenal (#3381): a vouched inventory
    // decides the pair's order in either direction (Crossbow first is the
    // engine's fixed order, Brimstone first moves Brimstone ahead,
    // `engine::relics::brimstone_leads_crossbow`); an unvouched one refuses
    // by the pair's name.
    let arsenal = "argument shape at Crossbow + Brimstone with Arsenal Strength order";
    // `case`'s facts with the inventory vouched or not, Crossbow and Brimstone
    // swapped when `brimstone_first`.
    let ordered = |case: &Value, brimstone_first: bool, vouched: bool| {
        let mut facts = entry_facts(case);
        let relics = &mut facts.relic_entry.relics_entering;
        let position = |relics: &[String], id: &str| relics.iter().position(|r| r == id).unwrap();
        let crossbow = position(relics, "RELIC.CROSSBOW");
        let brimstone = position(relics, "RELIC.BRIMSTONE");
        assert!(crossbow < brimstone, "the fixture appends Brimstone last");
        if brimstone_first {
            relics.swap(crossbow, brimstone);
        }
        facts.relic_entry.dispatch_ordered = vouched;
        facts
    };
    let admit_facts = |facts: &crate::entry::document::EntryDocument| {
        let opening = open_facts(facts).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
        let document = &opening.document;
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document).unwrap();
        let state = crate::boundary::HotBoundary::from_canonical(document, &catalog).unwrap();
        crate::engine::admit(document, &state, &catalog).map_err(|refusal| {
            refusal
                .missing()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
    };
    let arsenal_deck = with("CHARACTER.REGENT", &["BRIMSTONE"], &["ARSENAL"]);
    for brimstone_first in [false, true] {
        admit_facts(&ordered(&arsenal_deck, brimstone_first, true)).unwrap_or_else(|missing| {
            panic!("vouched, Brimstone first {brimstone_first}: {missing:?}")
        });
        let missing = admit_facts(&ordered(&arsenal_deck, brimstone_first, false))
            .expect_err("an unvouched inventory beside a reachable Arsenal refuses");
        assert!(missing.iter().any(|item| item == arsenal), "{missing:?}");
    }
    // Without Brimstone the same Arsenal deck admits: the fence is the pair.
    admit(&with("CHARACTER.REGENT", &[], &["ARSENAL"]))
        .unwrap_or_else(|missing| panic!("Arsenal alone: {missing:?}"));
    // No Arsenal card, but an Arsenal power already live on the player.
    let brimstone_deck = with("CHARACTER.REGENT", &["BRIMSTONE"], &[]);
    for vouched in [true, false] {
        let opening = open_facts(&ordered(&brimstone_deck, false, vouched)).unwrap();
        let catalog =
            crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
        let mut state =
            crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
        crate::engine::admit(&opening.document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("Regent Brimstone: {refusal}"));
        state.powers.set(
            crate::ids::PowerId::Arsenal,
            crate::powers::SlotWire::Int,
            1,
        );
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[crate::ids::PowerId::Arsenal])
        );
        let admitted = crate::engine::admit(&opening.document, &state, &catalog);
        if vouched {
            admitted.unwrap_or_else(|refusal| panic!("vouched live Arsenal: {refusal}"));
        } else {
            let refusal = admitted.expect_err("an unvouched live Arsenal refuses beside Brimstone");
            assert!(
                refusal.missing().any(|item| item.to_string() == arsenal),
                "{refusal:?}"
            );
        }
    }

    // The closure is the OWNER's pool: a Necrobinder catalog does not hold
    // the Regent Attacks a Regent owner would draw, so that state refuses.
    let opening = open(&crossbow_case("CHARACTER.NECROBINDER", false)).unwrap();
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    let mut state =
        crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
    state.reward_card_pool = Some(crate::catalog::RewardPool::Regent);
    let refusal = crate::engine::admit(&opening.document, &state, &catalog)
        .expect_err("another owner's pool is not in the closure");
    assert!(
        refusal
            .missing()
            .any(|item| item.to_string() == "argument shape at Crossbow generation closure"),
        "{refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// Toolbox, Sling of Courage, and Silent/Defect owner pools (#2827 item B)
// ---------------------------------------------------------------------------
//
// The Coach's synthetic roots reach all three. Every digest below was printed
// by the oracle with the recipe above `empty_belt_case`, on `empty_belt_case`
// with the mutation [`coach_case`] names.

/// The empty-belt fixture as `character` (when given), holding `relics` and
/// `cards` (both appended), under the fully unlocked profile (every epoch of
/// `boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS`) when `full_profile`, with a
/// recorded `combat_orbs` stream at counter 0 when `orbs`.
///
/// Mutation: `p['relics'].append({'floor_added_to_deck': 1, 'id': r})`,
/// `p['deck'].append({'floor_added_to_deck': 1, 'id': c})`,
/// `p['unlock_state']['unlocked_epochs'] = ['EPOCH.' + e for e in EPOCHS]`, and
/// `save['rng']['rngs']['combat_orbs'] = RunRngSet(seed).rngs['CombatOrbs']`'s
/// counter-0 words.
fn coach_case(
    character: Option<&str>,
    relics: &[&str],
    cards: &[&str],
    full_profile: bool,
    orbs: bool,
) -> Value {
    let mut case = empty_belt_case();
    let player = &mut case["save"]["players"][0];
    if let Some(character) = character {
        player["character_id"] = Value::from(character);
    }
    for id in relics {
        player["relics"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"floor_added_to_deck": 1, "id": id}));
    }
    for id in cards {
        player["deck"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"floor_added_to_deck": 1, "id": id}));
    }
    if full_profile {
        player["unlock_state"]["unlocked_epochs"] = Value::Array(
            crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS
                .iter()
                .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
                .collect(),
        );
    }
    if orbs {
        let seed = case["save"]["rng"]["seed"].as_str().unwrap().to_string();
        let words = crate::rng::run_stream_at_zero(&seed, "combat_orbs").words;
        case["save"]["rng"]["rngs"]["combat_orbs"] = serde_json::json!({
            "counter": 0, "s0": words[0], "s1": words[1], "s2": words[2], "s3": words[3]});
    }
    case
}

fn with_node_type(mut case: Value, node_type: &str) -> Value {
    case["node_type"] = Value::from(node_type);
    case
}

/// `RELIC.TOOLBOX`'s turn-1 choice opens where the oracle roots it, and the
/// pending three-card Colorless choice is the oracle's byte for byte.
///
/// The body is `engine::relics::before_hand_draw`'s turn-1 Toolbox pause; the
/// opening reaches it through `engine::deal_opening_hand`. The three owners
/// the oracle roots it for under a full profile each open to its digest.
#[test]
fn toolbox_opens_and_matches_the_oracle() {
    for (character, digest) in [
        (
            None,
            "490c5aaa0d6541720f6e0901865bf3f66bb1ba948983db68208d80f894d70591",
        ),
        (
            Some("CHARACTER.SILENT"),
            "4f091b9d3bb88ea45065a5e842a2dac6673c2fdc08ac014918b0af497d8c0756",
        ),
        (
            Some("CHARACTER.REGENT"),
            "268ba6ce10174bb206dcc027925b1e5c35fc55ce6d712665e7e7b3e4deeb818a",
        ),
    ] {
        let case = coach_case(character, &["RELIC.TOOLBOX"], &[], true, true);
        let opening =
            open(&case).unwrap_or_else(|err| panic!("{character:?} Toolbox opens: {err}"));
        // Toolbox's full-profile Colorless closure reaches Voltaic for every
        // owner here.
        assert_eq!(
            without_generated_voltaic_seed(&opening.document, true).differential_digest(),
            digest,
            "{character:?}"
        );
    }

    // #3392: a turn start that parks on a SetupPlayerTurn choice took
    // native's opening checksum at that pause, and never reaches the
    // synchronous pre-AutoPre boundary: a recording opening reports nothing,
    // so the census compares the parked root itself.
    let case = coach_case(None, &["RELIC.TOOLBOX"], &[], true, true);
    let recorded = build_opening(
        &entry_facts(&case),
        &OpeningOptions {
            record_native_checkpoints: true,
            ..OpeningOptions::default()
        },
    )
    .unwrap_or_else(|err| panic!("Toolbox opens: {err}"));
    assert!(
        recorded.document.player.contains_key("pending"),
        "the opening parks"
    );
    assert_eq!(recorded.native_checkpoints, Some(Vec::new()));
}

/// #3404: a deferring deal whose turn start parks before the hand draw
/// refuses rather than publish a pile the fixup never reached. The opening
/// never pairs Toolbox with a Draw-order relic (their own gates refuse the
/// pair), so the engine entry is driven directly on Toolbox's pre-hook state.
#[test]
fn a_deferring_deal_that_parks_before_the_hand_draw_refuses() {
    let case = coach_case(None, &["RELIC.TOOLBOX"], &[], true, true);
    let built = pre_hook(&case).expect("the Toolbox pre-hook builds");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&built.document)
        .expect("the pre-hook builds a catalog");
    let mut state = crate::boundary::HotBoundary::from_canonical(&built.document, &catalog)
        .expect("the pre-hook hydrates");
    assert_eq!(
        crate::engine::deal_opening_hand_deferring_turn_one_fixup(
            &mut state,
            &catalog,
            &mut Vec::new()
        ),
        Err(crate::engine::EngineRefusal::MalformedArgs(
            "deferred turn-one fixup before the hand draw"
        ))
    );
}

/// Toolbox's gates refuse by name.
///
/// Oracle messages: *"Toolbox requires the explicitly fully-unlocked owner
/// card pool"* with no profile (the oracle also refused a partial one; #3325
/// admits it, see [`toolbox_offers_the_recorded_colorless_pool_for_every_owner`]); *"TOOLBOX with
/// same-BeforeHandDraw relic peers ['PENDULUM']"* (and `['POLLINOUS_CORE']`);
/// *"CombatOrbGeneration counter unknown for this fight (consumers:
/// generation-relic card pools)"* with no `combat_orbs` stream.
#[test]
fn toolbox_refuses_where_the_oracle_does() {
    {
        let refusal = open(&coach_case(None, &["RELIC.TOOLBOX"], &[], false, true))
            .expect_err("no profile refuses");
        assert_eq!(refusal.class(), "generation_pool_partial_unlock");
    }
    for peer in ["RELIC.PENDULUM", "RELIC.POLLINOUS_CORE"] {
        assert_eq!(
            open(&coach_case(None, &["RELIC.TOOLBOX", peer], &[], true, true)).unwrap_err(),
            OpeningRefusal::RelicCombinationRefused {
                relics: vec!["RELIC.TOOLBOX".to_string(), peer.to_string()],
                reason: "Toolbox's same-BeforeHandDraw relic order is unrecorded",
            },
            "{peer}"
        );
    }
    assert_eq!(
        open(&coach_case(None, &["RELIC.TOOLBOX"], &[], true, false)).unwrap_err(),
        OpeningRefusal::CombatStreamAbsent {
            stream: "combat_orbs"
        }
    );
    // No recorded owner: the oracle's "requires a recorded owner
    // CharacterCardPool".
    let mut ownerless = coach_case(None, &["RELIC.TOOLBOX"], &[], true, true);
    ownerless["save"]["players"][0]
        .as_object_mut()
        .unwrap()
        .remove("character_id");
    assert_eq!(
        open(&ownerless).unwrap_err().class(),
        "generation_pool_unmodeled"
    );
}

/// Toolbox's three options are character-blind and follow the recorded
/// Colorless profile (#3325).
///
/// `Toolbox/<BeforeHandDraw>d__4::MoveNext` (v0.111.0 RVA `0x332adc`) loads
/// the static `CardPool<ColorlessCardPool>` (IL_005e) and the owner's
/// `UnlockState` (IL_0063-IL_0069) into `GetUnlockedCards` (IL_007e) before
/// `GetDistinctForCombat(.., 3, CombatCardGeneration)` (IL_00a8). So on one
/// seed every owner class is offered the SAME three cards, all from the
/// fully-unlocked Colorless pool; and a recorded profile hiding
/// `COLORLESS5_EPOCH` (and, since the oracle's Ironclad conjunct is gone,
/// `IRONCLAD2_EPOCH`) opens on that profile's 47-card pool.
#[test]
fn toolbox_offers_the_recorded_colorless_pool_for_every_owner() {
    use crate::steps::neutral::colorless_generation_pool;
    let full = colorless_generation_pool(None);
    let ids = |options: Vec<String>| -> Vec<crate::ids::CardId> {
        options
            .iter()
            .map(|option| crate::ids::CardId::from_str(option).unwrap())
            .collect()
    };
    let reference = ids(generation_options(
        &open(&coach_case(None, &["RELIC.TOOLBOX"], &[], true, true))
            .expect("the default owner opens")
            .document,
    ));
    assert_eq!(reference.len(), 3);
    assert!(
        reference.iter().all(|id| full.contains(id)),
        "{reference:?}"
    );
    for character in [
        "CHARACTER.IRONCLAD",
        "CHARACTER.SILENT",
        "CHARACTER.DEFECT",
        "CHARACTER.NECROBINDER",
        "CHARACTER.REGENT",
    ] {
        let opening = open(&coach_case(
            Some(character),
            &["RELIC.TOOLBOX"],
            &[],
            true,
            true,
        ))
        .unwrap_or_else(|err| panic!("{character} Toolbox opens: {err}"));
        assert_eq!(
            ids(generation_options(&opening.document)),
            reference,
            "{character}"
        );
    }

    let hidden = ["COLORLESS5_EPOCH", "IRONCLAD2_EPOCH"];
    let partial_epochs: Vec<&'static str> = crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS
        .iter()
        .copied()
        .filter(|epoch| !hidden.contains(epoch))
        .collect();
    let partial_pool = colorless_generation_pool(Some(&partial_epochs));
    assert_eq!(partial_pool.len(), 47);
    for character in [None, Some("CHARACTER.SILENT"), Some("CHARACTER.REGENT")] {
        let mut case = coach_case(character, &["RELIC.TOOLBOX"], &[], true, true);
        case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
            partial_epochs
                .iter()
                .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
                .collect(),
        );
        let opening = open(&case)
            .unwrap_or_else(|err| panic!("{character:?} partial-profile Toolbox opens: {err}"));
        let options = ids(generation_options(&opening.document));
        assert_eq!(options.len(), 3);
        assert!(
            options.iter().all(|id| partial_pool.contains(id)),
            "{character:?} {options:?}"
        );
        assert_ne!(
            options, reference,
            "{character:?}: the shuffle is over 47 rows"
        );
    }
}

/// `RELIC.SLING_OF_COURAGE` applies two Strength on an elite node only, in the
/// oracle's room-entry order, and counts in the Ruined Helmet gate.
///
/// Oracle: *"RUINED_HELMET with distinct AfterRoomEntered Strength sources
/// [('RELIC.VAJRA', 1), ('RELIC.SLING_OF_COURAGE', 2)]"*; *"SLING_OF_COURAGE
/// room type provenance missing"* without a node type.
#[test]
fn sling_of_courage_opens_and_matches_the_oracle() {
    let sling = |relics: &[&str], node_type: &str| {
        with_node_type(coach_case(None, relics, &[], false, false), node_type)
    };
    for (relics, node_type, strength, digest) in [
        (
            &["RELIC.SLING_OF_COURAGE"][..],
            "elite",
            Some(2),
            "4d7892e05e49449c8125b8d6077b22a1bbccf76703796ce13f0133fe5cab2ed9",
        ),
        (
            &["RELIC.SLING_OF_COURAGE"][..],
            "monster",
            None,
            "cbb57858079dde20a6978ac3d9d810216b77ab926af57647f80e2a55d09406dd",
        ),
        (
            &["RELIC.SLING_OF_COURAGE"][..],
            "boss",
            None,
            "0dacce91380ad5e7c1b83d14a6af370555ef2acc2932a2e09a227ca76f9eba55",
        ),
        (
            &["RELIC.SLING_OF_COURAGE", "RELIC.VAJRA"][..],
            "elite",
            Some(3),
            "b8099bbad7ce8f72f7c9b8db7424229d060b9bb7b8824fdb2d7083d711f15494",
        ),
        // A lone Sling doubled by Ruined Helmet: one source, so no order.
        (
            &["RELIC.SLING_OF_COURAGE", "RELIC.RUINED_HELMET"][..],
            "elite",
            Some(4),
            "a1044fe600911ec49e9a7893cb938466d5d314f2351f5be7a74c31527d50769b",
        ),
    ] {
        let opening = open(&sling(relics, node_type))
            .unwrap_or_else(|err| panic!("{relics:?} at {node_type} opens: {err}"));
        let hero_strength = opening
            .document
            .player
            .get("strength")
            .and_then(Value::as_i64);
        assert_eq!(hero_strength, strength, "{relics:?} at {node_type}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relics:?} at {node_type}"
        );
    }
    assert_eq!(
        open(&sling(
            &[
                "RELIC.SLING_OF_COURAGE",
                "RELIC.VAJRA",
                "RELIC.RUINED_HELMET"
            ],
            "elite"
        ))
        .unwrap_err()
        .class(),
        "ruined_helmet_strength_order"
    );
    let mut untyped = empty_belt_case();
    relic(
        &mut untyped,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.SLING_OF_COURAGE"}),
    );
    let mut facts = entry_facts(&untyped);
    facts.node_type = None;
    assert_eq!(
        build_opening(&facts, &OpeningOptions::default()).unwrap_err(),
        OpeningRefusal::SlingOfCourageRoomTypeUnknown
    );
}

/// Silent and Defect owner-pool generators open and match the oracle
/// (#2827 item B), as the Necrobinder ones do: the owner pool is read at play
/// time, so the gate is the whole question and the documents are the oracle's.
/// The Splash rows record `combat_orbs`; without it the oracle refuses and the
/// opening roots, which is the pre-existing #2931 absent-stream gap every
/// owner shares.
#[test]
fn silent_and_defect_generators_open_and_match_the_oracle() {
    for (character, card, orbs, digest) in [
        (
            "CHARACTER.SILENT",
            "CARD.JACKPOT",
            false,
            "95f41d568c14864cfff73222a24d1b924c5b1366f18cf9104aac0a020d1ecb9f",
        ),
        (
            "CHARACTER.SILENT",
            "CARD.CALAMITY",
            false,
            "23b2c575ae39275063f2cd372bf6cb28bcddda1cbe7dfa3176e4038705c074af",
        ),
        (
            "CHARACTER.SILENT",
            "CARD.SPLASH",
            true,
            "23c65914cb3a92af9103a5a38d88182e3983f390fbf6e7cdf8177bea03bcd0d1",
        ),
        (
            "CHARACTER.DEFECT",
            "CARD.DISCOVERY",
            false,
            "449ccd7841055603255f2ae02aba1391d3ed1535cef4e288c1bb767d6894c211",
        ),
        (
            "CHARACTER.DEFECT",
            "CARD.JACK_OF_ALL_TRADES",
            false,
            "be09e678cc15a68b480f3268a4d1e1070d4e68616f9a2bedd7a5cb300ce9bf3d",
        ),
        (
            "CHARACTER.DEFECT",
            "CARD.SPLASH",
            true,
            "a51068e8d2fbd5b50a1af9784887fb6045934f665a937134115712b3d02d528e",
        ),
    ] {
        let opening = open(&coach_case(Some(character), &[], &[card], true, orbs))
            .unwrap_or_else(|err| panic!("{character} {card} opens: {err}"));
        // Discovery draws the Defect pool, which holds Voltaic.
        assert_eq!(
            without_generated_voltaic_seed(&opening.document, card == "CARD.DISCOVERY")
                .differential_digest(),
            digest,
            "{character} {card}"
        );
    }
    // The profile gate reaches them as it reaches every owner. Since #3336 a
    // recorded partial profile opens (the oracle refused it on the Ironclad
    // epochs: "Jackpot requires the explicitly fully-unlocked owner card
    // pool"), and only an unrecorded one refuses.
    let mut partial = coach_case(
        Some("CHARACTER.SILENT"),
        &[],
        &["CARD.JACKPOT"],
        false,
        false,
    );
    partial["save"]["players"][0]["unlock_state"]["unlocked_epochs"] =
        serde_json::json!(["EPOCH.SILENT2_EPOCH"]);
    open(&partial).expect("a recorded partial profile opens (#3336)");
    let unrecorded = coach_case(
        Some("CHARACTER.SILENT"),
        &[],
        &["CARD.JACKPOT"],
        false,
        false,
    );
    assert_eq!(
        open(&unrecorded).unwrap_err().class(),
        "generation_pool_partial_unlock"
    );
}

/// `RELIC.MEAT_ON_THE_BONE` and `RELIC.POCKETWATCH` open and match the oracle
/// (#2827 item B): both window bodies are inert on turn one (the IL is cited
/// above `OPENING_WINDOW_RELIC_BODIES`). Pocketwatch beside Bag of Preparation
/// pins that turn one draws only the Bag's two extra cards.
#[test]
fn meat_on_the_bone_and_pocketwatch_open_and_match_the_oracle() {
    for (relics, hand, digest) in [
        (
            &["RELIC.MEAT_ON_THE_BONE"][..],
            5,
            "8e5e5e45ea46dd85ad3b6bdf1dedc78ad8fb3f90f3c9f69db3dadc68ebc90c23",
        ),
        (
            &["RELIC.POCKETWATCH"][..],
            5,
            "98e0977f613dca49795beac35a656bfe5e8c662d309acc60d966de6755ef11fe",
        ),
        (
            &["RELIC.POCKETWATCH", "RELIC.BAG_OF_PREPARATION"][..],
            7,
            "b6baf41b930ec6d023f6347a70119827f739f332b3a581ad7544c40b9d6674ff",
        ),
    ] {
        let opening = open(&coach_case(None, relics, &[], false, false))
            .unwrap_or_else(|err| panic!("{relics:?} opens: {err}"));
        assert_eq!(hand_size(&opening.document), hand, "{relics:?}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relics:?}"
        );
    }
}

/// An unobservable `CLONE` enchantment leaves the combat payload, as in the
/// oracle, and stays where Mystic Lighter or the Thieving Hopper can observe it
/// (#2827 item B; `retains_clone_enchantment` carries the IL).
///
/// Before this, a Clone deck opened with the enchantment on its copies and
/// `exact_piles` set, and the engine refused the document at load (*"entry
/// needs enchantment CLONE"*) where the oracle's document loads. Mutation:
/// `p['deck'].append({'floor_added_to_deck': 1, 'id': 'CARD.STRIKE_IRONCLAD',
/// 'enchantment': {'id': 'ENCHANTMENT.CLONE', 'amount': 4}})` (two Bash rows
/// for the pair case), on `empty_belt_case`, plus the relic or the encounter
/// named.
#[test]
fn a_clone_enchantment_is_kept_only_where_it_is_observable() {
    let clone_row = |id: &str| {
        serde_json::json!({"floor_added_to_deck": 1, "id": id,
                           "enchantment": {"id": "ENCHANTMENT.CLONE", "amount": 4}})
    };
    let with_rows = |rows: &[Value]| {
        let mut case = empty_belt_case();
        for row in rows {
            case["save"]["players"][0]["deck"]
                .as_array_mut()
                .unwrap()
                .push(row.clone());
        }
        case
    };
    let strike = with_rows(&[clone_row("CARD.STRIKE_IRONCLAD")]);
    let mut lighter = strike.clone();
    relic(
        &mut lighter,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.MYSTIC_LIGHTER"}),
    );
    let mut hopper = strike.clone();
    hopper["encounter_id"] = Value::from("ENCOUNTER.THIEVING_HOPPER_WEAK");
    for (name, case, kept, digest) in [
        (
            "alone",
            strike,
            false,
            "14dab5f91f95b02e5937fa756952285e5c4f7eb316b0eea9b6ef6fdbc37cf3dd",
        ),
        (
            "two Bash",
            with_rows(&[clone_row("CARD.BASH"), clone_row("CARD.BASH")]),
            false,
            "c60131544ca1ee594f94f2c4c95eb8bf2e1a913a983e8b238106c802459c8599",
        ),
        (
            "Mystic Lighter",
            lighter,
            true,
            "99cc920ea03fcf9314d185601d58f911914cc501bce241e61f8c0bf0532d9d40",
        ),
        (
            "Thieving Hopper",
            hopper,
            true,
            "f2a969466a8fb73fb2d9a37381c5dfdb6ef7cdac01d015b8e1d22b746b353905",
        ),
    ] {
        let opening = open(&case).unwrap_or_else(|err| panic!("{name} opens: {err}"));
        let clones = opening
            .document
            .piles
            .values()
            .flatten()
            .filter(|card| {
                card.enchantment
                    .as_ref()
                    .and_then(|value| value.get(0))
                    .and_then(Value::as_str)
                    == Some("CLONE")
            })
            .count();
        assert_eq!(clones > 0, kept, "{name}");
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{name}"
        );
    }
}

/// `RELIC.UNSETTLING_LAMP` opens armed and matches the oracle (#2827 item B).
///
/// `lamp` is seeded from ownership (`PER_FIGHT_RELIC_STATE_SEEDED` carries the
/// `BeforeCombatStart` IL), and a turn-1 relic debuff does not trigger it,
/// because the doubling arms only for a card-sourced power: Bag of Marbles'
/// Vulnerable lands undoubled, before or after the lamp in inventory order.
#[test]
fn unsettling_lamp_opens_armed_and_matches_the_oracle() {
    for (relics, digest) in [
        (
            &["RELIC.UNSETTLING_LAMP"][..],
            "763821f0fb5c8c9167889b1281ee9db861bf4634df2178b11f9e49ea1ac97881",
        ),
        (
            &["RELIC.UNSETTLING_LAMP", "RELIC.BAG_OF_MARBLES"][..],
            "38656be3bbceb12a2904a546c5483fba29381339c352bc486ce673dc6a2b5d5a",
        ),
        (
            &["RELIC.BAG_OF_MARBLES", "RELIC.UNSETTLING_LAMP"][..],
            "42194c4dae3d8f09dc13c22db345d10310b4a8659a50818629add1ab7f838240",
        ),
    ] {
        let opening = open(&coach_case(None, relics, &[], false, false))
            .unwrap_or_else(|err| panic!("{relics:?} opens: {err}"));
        assert_eq!(
            opening.document.player.get("lamp"),
            Some(&Value::Bool(true)),
            "{relics:?}"
        );
        assert_eq!(
            python_view(&opening.document).differential_digest(),
            digest,
            "{relics:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// RELIC.FENCING_MANUAL's turn-1 Forge (#3090)
// ---------------------------------------------------------------------------
//
// The Python oracle that printed the digests above was deleted (#2827 F3), so
// these witnesses assert the Forge's effects structurally against the
// untouched control, and pin the Rust document's digest only as a
// change-detector. Certification is the eval census against the game's own
// `.mcr` checksums (#1282 D1-D4), recorded on the PR.

/// Fencing Manual Forges 10 on the first turn-start walk.
///
/// `FencingManual/<AfterSideTurnStart>d__6::MoveNext` (`0x324a80`,
/// `ForgeCmd::Forge` at `IL_006c`); the body is
/// `engine::relics::fencing_manual_after_side_turn_start`. The deck holds no
/// Sovereign Blade, so the Forge generates the L0 Blade into the Hand's bottom
/// with the first uid past the entering deck and grows it by 10: the
/// control's hand plus one card whose payload says 20, and one generated-card
/// history tick. The opening's own identity-allocation check
/// (`check_card_identity_allocation`) passing is part of the witness: the
/// Blade is the single fresh uid and it is the Hand's tail.
#[test]
fn a_fencing_manual_save_forges_a_twenty_damage_blade_into_the_hand() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.FENCING_MANUAL")).expect("Fencing Manual opens");
    let control_hand = control.document.piles["hand"].clone();
    let forged_hand = opening.document.piles["hand"].clone();
    assert!(card_named(&control.document, "SOVEREIGN_BLADE").is_empty());
    assert_eq!(forged_hand.len(), control_hand.len() + 1);
    let blade = forged_hand.last().expect("the hand has a tail");
    assert_eq!(blade.id, "SOVEREIGN_BLADE");
    assert_eq!(blade.upgrade, 0);
    let deck = control.document.player["next_card_uid"].as_u64().unwrap();
    assert_eq!(blade.uid, Some(deck));
    assert_eq!(
        serde_json::to_value(blade).unwrap()["sovereign_blade"],
        serde_json::json!(["SOVEREIGN_BLADE", 20, 1])
    );
    assert_eq!(card_named(&opening.document, "SOVEREIGN_BLADE").len(), 1);
    assert_eq!(
        opening.document.player["next_card_uid"],
        Value::from(deck + 1)
    );
    assert_eq!(
        opening.document.player["owner_generated_cards_combat"],
        Value::from(1)
    );
    // Everything the deal put in Hand before the Forge is the control's.
    let ids = |cards: &[crate::canonical::CanonicalCardV2]| {
        cards
            .iter()
            .map(|card| (card.id.clone(), card.uid))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&forged_hand[..control_hand.len()]), ids(&control_hand));
    // The Forge draws no stream.
    assert_eq!(opening.document.rng, control.document.rng);
    assert_eq!(
        python_view(&opening.document).differential_digest(),
        FENCING_MANUAL_DIGEST,
        "Rust change-detector, not an oracle value"
    );
}

/// The Rust document digest for
/// [`a_fencing_manual_save_forges_a_twenty_damage_blade_into_the_hand`],
/// recorded 2026-09-25 (#3090) as a change-detector.
const FENCING_MANUAL_DIGEST: &str =
    "c9aa36621b0520a1389caefc2970a4f4b9272323c21a5167447dd50b1b6bcd70";

/// The catalog interns the Blade only while the body can still fire (#3090,
/// `boundary.rs` beside Bellows): on the pre-deal document it is present
/// (this Ironclad deck has no Forge card of its own, so nothing else would),
/// and at turn 2, past the `TurnNumber <= 1` guard, it is not.
#[test]
fn the_catalog_interns_the_fencing_manual_blade_only_before_the_walk() {
    let blade = crate::catalog::CardIdentity {
        id: crate::ids::CardId::SovereignBlade,
        upgrade: 0,
        enchantment: None,
    };
    let control = pre_hook(&empty_belt_case()).expect("the control builds");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&control.document)
        .expect("catalog builds");
    assert!(catalog.atom(&blade).is_none(), "control");

    let built = pre_hook(&with_relic("RELIC.FENCING_MANUAL")).expect("the pre-hook builds");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&built.document)
        .expect("catalog builds");
    assert!(catalog.atom(&blade).is_some(), "turn 1, no Blade yet");
    assert!(catalog.is_reachable(blade));

    let mut later = built.document.clone();
    later.player.insert("turn".to_string(), Value::from(2));
    let catalog =
        crate::boundary::HotBoundary::catalog_from_canonical(&later).expect("catalog builds");
    assert!(catalog.atom(&blade).is_none(), "turn 2");

    // Past the walk the Blade is in a pile, and the gate adds nothing: the
    // post-opening document's catalog is the one its piles alone produce.
    let opened = open(&with_relic("RELIC.FENCING_MANUAL")).expect("Fencing Manual opens");
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opened.document)
        .expect("catalog builds");
    assert!(catalog.atom(&blade).is_some(), "the dealt Blade");
}

/// Every Fencing Manual same-hook arm, by direct call.
///
/// Crossbow, Big Hat and Infused Core refuse at any order (Infused Core is
/// gated earlier by `TURN_ONE_ORB_RELICS`, so only a direct call reaches it).
/// Orange Dough is admitted exactly when the inventory is vouched for and
/// every Dough precedes every Manual; each way that can fail refuses.
#[test]
fn every_fencing_manual_turn_start_order_arm_refuses_by_name() {
    const FM: &str = "RELIC.FENCING_MANUAL";
    const OD: &str = "RELIC.ORANGE_DOUGH";
    let check = |inventory: &[&str], ordered: bool| {
        let relics: std::collections::BTreeSet<&str> = inventory.iter().copied().collect();
        let entering: Vec<String> = inventory.iter().map(ToString::to_string).collect();
        super::fencing_manual_turn_start_peers_are_ordered(&relics, &entering, ordered)
            .err()
            .map(|refusal| match refusal {
                OpeningRefusal::RelicCombinationRefused { relics, .. } => relics,
                other => panic!("wrong refusal {other:?}"),
            })
    };
    for peer in ["RELIC.BIG_HAT", "RELIC.CROSSBOW", "RELIC.INFUSED_CORE"] {
        for ordered in [false, true] {
            for inventory in [[peer, FM], [FM, peer]] {
                assert_eq!(
                    check(&inventory, ordered),
                    Some(vec![FM.to_string(), peer.to_string()]),
                    "{inventory:?} ordered={ordered}"
                );
            }
        }
        assert_eq!(check(&[peer], true), None, "{peer} without Fencing Manual");
    }
    // Orange Dough: the one admitted order, and every refused one.
    assert_eq!(check(&[OD, "RELIC.LANTERN", FM], true), None);
    let refused = Some(vec![FM.to_string(), OD.to_string()]);
    assert_eq!(check(&[FM, OD], true), refused, "Manual first");
    assert_eq!(check(&[OD, FM], false), refused, "order not vouched for");
    assert_eq!(
        check(&[OD, FM, OD], true),
        refused,
        "a Dough after the Manual"
    );
    assert_eq!(check(&[OD], true), None, "Dough without Fencing Manual");
    // A vouched Dough-first order does not excuse a second, unorderable peer.
    assert_eq!(
        check(&[OD, "RELIC.CROSSBOW", FM], true),
        Some(vec![FM.to_string(), "RELIC.CROSSBOW".to_string()])
    );
    // Same-hook peers whose writes commute with the Forge are admitted.
    for peer in [
        "RELIC.BRIMSTONE",
        "RELIC.LANTERN",
        "RELIC.BREAD",
        "RELIC.SYMBIOTIC_VIRUS",
    ] {
        for ordered in [false, true] {
            assert_eq!(check(&[FM, peer], ordered), None, "{peer}");
        }
    }
    assert_eq!(check(&[FM], false), None);
}

/// Orange Dough acquired AFTER Fencing Manual refuses through the opening:
/// native would Forge before the Dough, and this port runs the Dough first.
#[test]
fn fencing_manual_before_orange_dough_refuses_through_the_opening() {
    let mut case = with_relic("RELIC.FENCING_MANUAL");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.ORANGE_DOUGH"}),
    );
    let refusal = open(&case).unwrap_err();
    assert_eq!(refusal.class(), "relic_combination_refused");
    assert!(
        refusal.to_string().contains("RELIC.FENCING_MANUAL")
            && refusal.to_string().contains("RELIC.ORANGE_DOUGH"),
        "{refusal}"
    );
}

/// Orange Dough acquired BEFORE Fencing Manual passes this gate on the
/// vouched fixture save, and the opening goes on to the next ownership check:
/// the Dough's generation-stream gate (`generation_relic_streams_are_recorded`),
/// which this synthetic save, recording no `combat_orbs`, fails. The real
/// fights that pass it are measured by the eval census (PR body).
#[test]
fn orange_dough_before_fencing_manual_passes_the_order_gate() {
    let mut case = with_relic("RELIC.ORANGE_DOUGH");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.FENCING_MANUAL"}),
    );
    assert!(entry_facts(&case).relic_entry.dispatch_ordered);
    let refusal = open(&case).unwrap_err();
    assert_eq!(refusal.class(), "combat_stream_absent", "{refusal}");
    assert!(refusal.to_string().contains("combat_orbs"), "{refusal}");
}

/// Retiring the turn-1 gate makes the Symbiotic Virus arm that names Fencing
/// Manual reachable through `open`: Symbiotic Virus + Cracked Core + Fencing
/// Manual at this Ironclad's zero base slots refuses by that arm's name.
#[test]
fn fencing_manual_with_symbiotic_virus_and_cracked_core_refuses_like_the_oracle() {
    let mut case = with_relic("RELIC.SYMBIOTIC_VIRUS");
    for peer in ["RELIC.CRACKED_CORE", "RELIC.FENCING_MANUAL"] {
        relic(
            &mut case,
            serde_json::json!({"floor_added_to_deck": 1, "id": peer}),
        );
    }
    let refusal = open(&case).unwrap_err();
    assert_eq!(refusal.class(), "symbiotic_virus_turn_start_order");
    assert!(
        refusal.to_string().contains("RELIC.FENCING_MANUAL")
            && refusal.to_string().contains("RELIC.CRACKED_CORE"),
        "{refusal}"
    );
}

/// Fencing Manual beside a commuting late-group peer opens with both effects.
/// Brimstone runs after the Forge in the same group and writes Strength only,
/// so the monsters are Brimstone's alone and the Blade is Fencing Manual's.
#[test]
fn fencing_manual_with_brimstone_opens_with_both_effects() {
    let mut case = with_relic("RELIC.FENCING_MANUAL");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BRIMSTONE"}),
    );
    let opening = open(&case).expect("Fencing Manual with Brimstone opens");
    assert_eq!(card_named(&opening.document, "SOVEREIGN_BLADE").len(), 1);
    let brimstone = open(&with_relic("RELIC.BRIMSTONE")).expect("Brimstone opens");
    assert_eq!(opening.document.monsters, brimstone.document.monsters);
}

/// #3100: `RELIC.PEN_NIB` beside every replay source the retired
/// `pen_nib_replay_semantics` gate named now opens, with the saved counter
/// seeded unchanged.
///
/// The gate refused because nobody had read whether Pen Nib's
/// BeforeCardPlayed re-fires on a replay. It does, once per generated body:
/// every source (Duplicator's Duplication, Glam, Spiral, One-Two Punch, Echo
/// Form, Sword Sage's `BaseReplayCount`) feeds the one frozen play count that
/// `OnPlayWrapper` loops over, and `engine::relics::before_card_played_hand`
/// carries the IL. The one residual, a replayable Sovereign Blade, refuses by
/// name at engine admission, which the opening does not duplicate. Each row
/// mutates [`full_profile_case`] (so the Duplicator belt is exact) by
/// appending Pen Nib with `AttacksPlayed = 3` plus the named source.
#[test]
fn pen_nib_beside_every_replay_source_opens_with_its_saved_counter() {
    fn strike_with(enchantment: &str) -> Value {
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "CARD.STRIKE_IRONCLAD",
            "enchantment": {"id": enchantment, "amount": 1}})
    }
    let card = |id: &str| serde_json::json!({"floor_added_to_deck": 1, "id": id});
    for (name, deck, potions) in [
        ("control: no replay source", None, vec![]),
        ("Duplicator", None, vec![potion_row("POTION.DUPLICATOR", 0)]),
        ("Glam", Some(strike_with("ENCHANTMENT.GLAM")), vec![]),
        ("Spiral", Some(strike_with("ENCHANTMENT.SPIRAL")), vec![]),
        ("One-Two Punch", Some(card("CARD.ONE_TWO_PUNCH")), vec![]),
        ("Echo Form", Some(card("CARD.ECHO_FORM")), vec![]),
        ("Sword Sage", Some(card("CARD.SWORD_SAGE")), vec![]),
    ] {
        let mut case = full_profile_case();
        relic(
            &mut case,
            serde_json::json!({
                "floor_added_to_deck": 1, "id": "RELIC.PEN_NIB",
                "props": {"ints": [{"name": "AttacksPlayed", "value": 3}]}}),
        );
        if let Some(row) = deck {
            deck_row(&mut case, row);
        }
        case["save"]["players"][0]["potions"] = Value::Array(potions);
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(
            opening.document.player.get("pen_nib"),
            Some(&Value::from(3)),
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Owner-pool generators draw the RECORDED profile's pool (#2560, #2946)
// ---------------------------------------------------------------------------
//
// Every witness below is a Rust-opened root (save bytes in, canonical document
// out, `catalog_from_canonical`, `from_canonical`, `admit`) whose profile HIDES
// at least one of the owner's own gating epochs, so the recorded profile's
// pool is strictly smaller than the fully-unlocked one and a fully-unlocked
// fallback would be caught. The expected cards are computed here from the
// pre-play `CombatCardGeneration` stream with the native `Rng` port directly:
// `GetDistinctForCombat` is one complete shuffle (`CardFactory` `0x112878`),
// `GetForCombat` one `NextInt(count)` per card (`0x1162e8`).

/// The `DEFECT7_EPOCH` rows of the Defect pool (`DefectCardPool::
/// FilterThroughEpochs` `0xf1944`), which [`owner_deck_with`]'s profile hides.
const DEFECT7_ROWS: [crate::ids::CardId; 3] = [
    crate::ids::CardId::HelixDrill,
    crate::ids::CardId::Scavenge,
    crate::ids::CardId::Turbo,
];

/// Open `case`, move one copy of each of `into_hand` to the Hand's tail (the
/// shuffle decides which pile a card lands in, which is not what these
/// witnesses are about), set energy to 9, and hydrate.
fn owner_pool_root(
    case: &Value,
    into_hand: &[&str],
) -> (
    crate::canonical::CanonicalStateV2,
    crate::catalog::Catalog,
    crate::hot::HotState,
) {
    let opening = open(case).unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    let mut document = opening.document.clone();
    let mut piles: Value = serde_json::to_value(&document.piles).expect("the piles serialize");
    for wanted in into_hand {
        let in_hand = piles["hand"]
            .as_array()
            .is_some_and(|hand| hand.iter().any(|card| card["id"] == *wanted));
        if in_hand {
            continue;
        }
        let mut moved = None;
        for name in ["draw", "discard"] {
            let Some(pile) = piles.get_mut(name).and_then(Value::as_array_mut) else {
                continue;
            };
            if let Some(index) = pile.iter().position(|card| card["id"] == *wanted) {
                moved = Some(pile.remove(index));
                break;
            }
        }
        piles["hand"]
            .as_array_mut()
            .expect("a hand")
            .push(moved.unwrap_or_else(|| panic!("{wanted} is in the deck")));
    }
    document.piles = serde_json::from_value(piles).expect("the piles round-trip");
    document.player.insert("energy".to_string(), Value::from(9));
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document)
        .expect("the Rust-opened root builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog)
        .expect("the Rust-opened root hydrates");
    (document, catalog, state)
}

/// `case` with the profile set to every epoch this build declares except
/// `hidden` (the `EPOCH.`-prefixed save spelling).
fn with_hidden_epochs(mut case: Value, hidden: &[&str]) -> Value {
    case["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = Value::Array(
        crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .filter(|epoch| !hidden.contains(epoch))
            .map(|epoch| Value::from(format!("EPOCH.{epoch}")))
            .collect(),
    );
    case
}

fn hand_card_uid(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    id: crate::ids::CardId,
) -> u32 {
    state
        .piles
        .get(crate::hot::PileId::Hand)
        .as_slice()
        .iter()
        .find(|card| catalog.spec(card.atom).unwrap().identity.id == id)
        .unwrap_or_else(|| panic!("{id:?} is in hand"))
        .uid
}

fn hand_card_ids(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
) -> Vec<crate::ids::CardId> {
    state
        .piles
        .get(crate::hot::PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| catalog.spec(card.atom).unwrap().identity.id)
        .collect()
}

fn generation_rng(state: &crate::hot::HotState) -> crate::rng::Xoshiro256StarStar {
    let live = state.rng.get(crate::hot::RngStream::Generation);
    crate::rng::Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    }
}

fn play(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    uid: u32,
    target: Option<u8>,
) -> Result<crate::hot::HotState, crate::engine::EngineRefusal> {
    crate::engine::apply_action_into(
        state,
        catalog,
        &crate::engine::Action::Play {
            uid,
            target,
            selection: crate::engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
}

/// The first `count` cards of one native `GetDistinctForCombat` shuffle.
fn distinct_prefix(
    rng: &mut crate::rng::Xoshiro256StarStar,
    pool: &[crate::ids::CardId],
    count: usize,
) -> Vec<crate::ids::CardId> {
    let mut shuffled = pool.to_vec();
    rng.shuffle(&mut shuffled).expect("a bounded shuffle");
    shuffled.truncate(count);
    shuffled
}

/// `count` native `GetForCombat` samples, with replacement.
fn samples(
    rng: &mut crate::rng::Xoshiro256StarStar,
    pool: &[crate::ids::CardId],
    count: usize,
) -> Vec<crate::ids::CardId> {
    (0..count)
        .map(|_| pool[usize::try_from(rng.next_bounded(pool.len() as i32).unwrap()).unwrap()])
        .collect()
}

/// Discovery on a Defect root that hides `DEFECT7_EPOCH` (#2946).
///
/// `Discovery/<OnPlay>d__4::MoveNext` `0x399254` reads `Owner.Character.
/// CardPool` (IL_003e-IL_0043) through `GetUnlockedCards(Owner.UnlockState,
/// ..)` (IL_0063) and offers the first three of one `GetDistinctForCombat`
/// shuffle (IL_007e). The offer is that shuffle of the RECORDED profile's
/// pool, which lacks the three `DEFECT7_EPOCH` rows; the fully-unlocked
/// control (every epoch revealed) offers from the complete pool.
#[test]
fn discovery_offers_the_recorded_profile_owner_pool() {
    use crate::ids::CardId;
    for (hidden, partial) in [(&["DEFECT7_EPOCH"][..], true), (&[][..], false)] {
        let case = with_hidden_epochs(owner_deck_with("CHARACTER.DEFECT", &["DISCOVERY"]), hidden);
        let (document, catalog, state) = owner_pool_root(&case, &["DISCOVERY"]);
        assert_eq!(catalog.splash_unlock_epochs().is_some(), partial);
        assert_eq!(state.fully_unlocked_card_pool_epochs, !partial);
        assert!(!document.player.contains_key("entropy_card_pool"));
        crate::engine::admit(&document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("hidden={hidden:?}: {refusal}"));
        let pool = catalog.owner_generation_pool(crate::catalog::RewardPool::Defect);
        let full =
            crate::steps::neutral::owner_generation_pool(crate::catalog::RewardPool::Defect, None);
        let gated = DEFECT7_ROWS
            .iter()
            .filter(|id| full.contains(id))
            .collect::<Vec<_>>();
        assert!(!gated.is_empty(), "DEFECT7 gates a Discovery row");
        assert_eq!(gated.iter().all(|id| !pool.contains(id)), partial);
        assert_eq!(pool.len() < full.len(), partial);

        let expected = distinct_prefix(&mut generation_rng(&state), &pool, 3);
        let played = play(
            &state,
            &catalog,
            hand_card_uid(&state, &catalog, CardId::Discovery),
            None,
        )
        .expect("Discovery plays");
        let wire = crate::boundary::HotBoundary::to_canonical(&played, &catalog);
        assert_eq!(wire.player["pending"][0], "discovery_select");
        let offered: Vec<CardId> = wire.player["pending"][3]
            .as_array()
            .expect("the options")
            .iter()
            .map(|option| CardId::from_str(option[0].as_str().unwrap()).unwrap())
            .collect();
        assert_eq!(offered, expected, "hidden={hidden:?}");
    }
}

/// Jackpot on a Defect root that hides `DEFECT7_EPOCH` (#2946).
///
/// `Jackpot/<OnPlay>d__3::MoveNext` `0x3a80d4`: the owner pool through
/// `GetUnlockedCards(Owner.UnlockState, ..)` (IL_010b), the canonical-zero-cost
/// `Where` (IL_012f), three `GetForCombat` samples (IL_0159). `TURBO` is a
/// zero-cost `DEFECT7_EPOCH` row, so the recorded pool is one card shorter and
/// the sample indices land on different cards than a fully-unlocked draw would.
#[test]
fn jackpot_samples_the_recorded_profile_zero_cost_pool() {
    use crate::ids::CardId;
    for (hidden, partial) in [(&["DEFECT7_EPOCH"][..], true), (&[][..], false)] {
        let case = with_hidden_epochs(owner_deck_with("CHARACTER.DEFECT", &["JACKPOT"]), hidden);
        let (document, catalog, state) = owner_pool_root(&case, &["JACKPOT"]);
        crate::engine::admit(&document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("hidden={hidden:?}: {refusal}"));
        let pool = catalog.owner_zero_cost_pool(crate::catalog::RewardPool::Defect);
        let full =
            crate::steps::neutral::owner_zero_cost_pool(crate::catalog::RewardPool::Defect, None);
        assert!(full.contains(&CardId::Turbo));
        assert_eq!(!pool.contains(&CardId::Turbo), partial);
        let expected = samples(&mut generation_rng(&state), &pool, 3);
        let played = play(
            &state,
            &catalog,
            hand_card_uid(&state, &catalog, CardId::Jackpot),
            Some(0),
        )
        .expect("Jackpot plays");
        let hand = hand_card_ids(&played, &catalog);
        assert_eq!(hand[hand.len() - 3..], expected[..], "hidden={hidden:?}");
    }
}

/// Distraction and White Noise draw the owner's one-type pool under the
/// recorded profile (#2560).
///
/// Distraction (`0x399690` IL_002c-IL_0051, Skill predicate `0x399682`) and
/// White Noise (`0x3c74ac` IL_00aa-IL_00cf, Power predicate `0x3c749e`) take
/// the first card of one `GetDistinctForCombat` shuffle. Before #2560
/// Distraction was Silent-only (its frozen table was the Silent Skill list),
/// so a Defect Distraction refused whatever its profile. This profile hides
/// `DEFECT5_EPOCH` and `DEFECT7_EPOCH`: `SCAVENGE`/`TURBO` leave the Skill pool
/// and `SMOKESTACK` the Power pool.
#[test]
fn distraction_and_white_noise_draw_the_recorded_profile_owner_type_pool() {
    use crate::content_tables::CardType;
    use crate::ids::CardId;
    for (card, card_type, gated) in [
        ("DISTRACTION", CardType::Skill, CardId::Scavenge),
        ("WHITE_NOISE", CardType::Power, CardId::Smokestack),
    ] {
        for (hidden, partial) in [
            (&["DEFECT5_EPOCH", "DEFECT7_EPOCH"][..], true),
            (&[][..], false),
        ] {
            let case = with_hidden_epochs(owner_deck_with("CHARACTER.DEFECT", &[card]), hidden);
            let (document, catalog, state) = owner_pool_root(&case, &[card]);
            crate::engine::admit(&document, &state, &catalog)
                .unwrap_or_else(|refusal| panic!("{card} hidden={hidden:?}: {refusal}"));
            let pool =
                catalog.owner_type_generation_pool(crate::catalog::RewardPool::Defect, card_type);
            assert_eq!(!pool.contains(&gated), partial, "{card}");
            let expected = distinct_prefix(&mut generation_rng(&state), &pool, 1);
            let id = CardId::from_str(card).unwrap();
            let played = play(&state, &catalog, hand_card_uid(&state, &catalog, id), None)
                .unwrap_or_else(|refusal| panic!("{card} plays: {refusal:?}"));
            assert_eq!(
                hand_card_ids(&played, &catalog).last(),
                expected.first(),
                "{card} hidden={hidden:?}"
            );
        }
    }
}

/// Metamorphosis and Calamity sample the recorded profile's owner Attack pool
/// (#2560).
///
/// `Metamorphosis/<OnPlay>d__5::MoveNext` `0x3ac1e8` (owner pool IL_002c-IL_0051,
/// Attack `Where` IL_0075, `GetForCombat` IL_009f) and `CalamityPower/
/// <AfterCardPlayed>d__7::MoveNext` `0x3368bc` (owner pool IL_005a-IL_0089,
/// `GetForCombat` IL_00d2). `HELIX_DRILL` is a `DEFECT7_EPOCH` Attack, so it
/// leaves both pools on this profile.
#[test]
fn metamorphosis_and_calamity_sample_the_recorded_profile_attack_pool() {
    use crate::ids::CardId;
    // Two separate roots: one deck holding both is refused by name as
    // `Metamorphosis generation ordering`, which is not this witness.
    let case = owner_deck_with("CHARACTER.DEFECT", &["METAMORPHOSIS"]);
    let (document, catalog, state) = owner_pool_root(&case, &["METAMORPHOSIS"]);
    crate::engine::admit(&document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("Metamorphosis: {refusal}"));
    let pool = catalog.metamorphosis_attack_pool(crate::catalog::RewardPool::Defect);
    assert!(!pool.contains(&CardId::HelixDrill));
    assert!(
        crate::steps::neutral::metamorphosis_owner_attack_pool(
            crate::catalog::RewardPool::Defect,
            None
        )
        .contains(&CardId::HelixDrill)
    );
    let expected = samples(&mut generation_rng(&state), &pool, 3);
    let played = play(
        &state,
        &catalog,
        hand_card_uid(&state, &catalog, CardId::Metamorphosis),
        None,
    )
    .expect("Metamorphosis plays");
    // The results land at random Draw positions (#3243), in allocation order.
    let first_generated = state.next_card_uid;
    let mut generated: Vec<_> = played
        .piles
        .get(crate::hot::PileId::Draw)
        .as_slice()
        .iter()
        .filter(|card| card.uid >= first_generated)
        .map(|card| (card.uid, catalog.spec(card.atom).unwrap().identity.id))
        .collect();
    generated.sort_unstable_by_key(|(uid, _)| *uid);
    assert_eq!(
        generated.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
        expected
    );

    let case = owner_deck_with("CHARACTER.DEFECT", &["CALAMITY", "STRIKE_DEFECT"]);
    let (document, catalog, state) = owner_pool_root(&case, &["CALAMITY", "STRIKE_DEFECT"]);
    crate::engine::admit(&document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("Calamity: {refusal}"));
    let pool = catalog.calamity_attack_pool(crate::catalog::RewardPool::Defect);
    assert!(!pool.contains(&CardId::HelixDrill));
    assert!(
        crate::steps::neutral::calamity_owner_attack_pool(crate::catalog::RewardPool::Defect, None)
            .contains(&CardId::HelixDrill)
    );
    let armed = play(
        &state,
        &catalog,
        hand_card_uid(&state, &catalog, CardId::Calamity),
        None,
    )
    .expect("Calamity plays");
    let expected = samples(&mut generation_rng(&armed), &pool, 1);
    let struck = play(
        &armed,
        &catalog,
        hand_card_uid(&armed, &catalog, CardId::StrikeDefect),
        Some(0),
    )
    .expect("a Strike plays into Calamity");
    assert_eq!(hand_card_ids(&struck, &catalog).last(), expected.first());
}

/// The Colorless rows `COLORLESS5_EPOCH` gates (`ColorlessCardPool::
/// FilterThroughEpochs` `0xf13f4`), which the witnesses below hide.
const COLORLESS5_ROWS: [crate::ids::CardId; 3] = [
    crate::ids::CardId::Anointed,
    crate::ids::CardId::Calamity,
    crate::ids::CardId::Splash,
];

/// [`owner_pool_root`] for a Colorless generator: the named card in hand, nine
/// energy, and seven Stars so Quasar's fixed two-Star cost is payable.
fn colorless_root(
    character: &str,
    card: &str,
    hidden: &[&str],
) -> (
    crate::canonical::CanonicalStateV2,
    crate::catalog::Catalog,
    crate::hot::HotState,
) {
    let case = with_hidden_epochs(owner_deck_with(character, &[card]), hidden);
    let (mut document, _, _) = owner_pool_root(&case, &[card]);
    document.player.insert("stars".to_string(), Value::from(7));
    // The Colorless pool reaches Entropy, whose closure reaches Black Hole and
    // the Regent Star writers, so admission needs the (empty, turn-1)
    // AfterEnergyReset acquisition order as the explicit fact a real
    // checkpoint carries (#2669), or it refuses the terminal order by name.
    document.player.insert(
        "after_energy_reset_order".to_string(),
        Value::Array(Vec::new()),
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document)
        .expect("the Rust-opened root builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog)
        .expect("the Rust-opened root hydrates");
    (document, catalog, state)
}

/// The five Colorless generators draw the RECORDED profile's Colorless pool
/// (the Colorless half of #2560).
///
/// Every body loads `CardPool<ColorlessCardPool>` through
/// `GetUnlockedCards(Owner.UnlockState, ..)` and hands it to one
/// `GetDistinctForCombat` shuffle: Quasar (`0x3b4e48` IL_004d, three offered
/// at IL_0068), Bundle of Joy (`0x390188` IL_003e, `Cards` = 3 at IL_0068),
/// Manifest Authority (`0x3ab860` IL_00c2, one at IL_00dd), Spectrum Shift's
/// listener (`0x345978` IL_005e, `Amount` at IL_007e) and Jack of All Trades
/// (`0x3a7ecc` IL_0046, minus itself at IL_006a, `Cards` = 1 at IL_0094). The
/// profile hides `COLORLESS5_EPOCH`, so `ANOINTED`, `CALAMITY` and `SPLASH`
/// leave the pool and every later index lands on a different card than the
/// fully-unlocked shuffle would; the control reveals every epoch and draws the
/// frozen constants. Each expected card is computed from the pre-play
/// `CombatCardGeneration` stream with the native `Rng` port. Bundle of Joy and
/// Jack also run on a Defect owner: neither body reads the owner's class, and
/// Bundle of Joy used to refuse every non-Regent owner.
#[test]
fn colorless_generators_draw_the_recorded_profile_colorless_pool() {
    use crate::ids::CardId;
    use crate::steps::neutral::{colorless_generation_pool, jack_of_all_trades_pool};
    let full = colorless_generation_pool(None);
    assert_eq!(
        full[..],
        crate::content_tables::REGENT_COLORLESS_GENERATION_POOL_V1101[..]
    );
    assert_eq!(
        jack_of_all_trades_pool(None)[..],
        crate::content_tables::JACK_OF_ALL_TRADES_POOL_V1091[..]
    );
    for (hidden, partial) in [(&["COLORLESS5_EPOCH"][..], true), (&[][..], false)] {
        for (character, card, count) in [
            ("CHARACTER.IRONCLAD", "QUASAR", 3),
            ("CHARACTER.NECROBINDER", "BUNDLE_OF_JOY", 3),
            ("CHARACTER.SILENT", "BUNDLE_OF_JOY", 3),
            ("CHARACTER.IRONCLAD", "MANIFEST_AUTHORITY", 1),
            ("CHARACTER.NECROBINDER", "JACK_OF_ALL_TRADES", 1),
            ("CHARACTER.SILENT", "JACK_OF_ALL_TRADES", 1),
        ] {
            let label = format!("{card} on {character}, hidden={hidden:?}");
            let (document, catalog, state) = colorless_root(character, card, hidden);
            assert_eq!(catalog.splash_unlock_epochs().is_some(), partial, "{label}");
            crate::engine::admit(&document, &state, &catalog)
                .unwrap_or_else(|refusal| panic!("{label}: {refusal}"));
            let id = CardId::from_str(card).unwrap();
            let pool = if id == CardId::JackOfAllTrades {
                catalog.jack_of_all_trades_pool()
            } else {
                catalog.colorless_generation_pool()
            };
            assert_eq!(
                COLORLESS5_ROWS.iter().all(|row| !pool.contains(row)),
                partial,
                "{label}"
            );
            assert_eq!(
                pool.len() + 3 * usize::from(partial),
                full.len() - usize::from(id == CardId::JackOfAllTrades),
                "{label}"
            );
            let expected = distinct_prefix(&mut generation_rng(&state), &pool, count);
            let played = play(&state, &catalog, hand_card_uid(&state, &catalog, id), None)
                .unwrap_or_else(|refusal| panic!("{label} plays: {refusal:?}"));
            if id == CardId::Quasar {
                let wire = crate::boundary::HotBoundary::to_canonical(&played, &catalog);
                assert_eq!(wire.player["pending"][0], "quasar_select", "{label}");
                let offered: Vec<CardId> = wire.player["pending"][3]
                    .as_array()
                    .expect("the options")
                    .iter()
                    .map(|option| CardId::from_str(option[0].as_str().unwrap()).unwrap())
                    .collect();
                assert_eq!(offered, expected, "{label}");
                // The pending decoder reads the same pool back.
                let reloaded = crate::boundary::HotBoundary::from_canonical(&wire, &catalog)
                    .unwrap_or_else(|refusal| panic!("{label} decodes: {refusal:?}"));
                assert_eq!(reloaded, played, "{label}");
                // `selection::quasar_candidates` accepts the recorded pool's
                // options and mints the chosen one.
                let chosen = crate::engine::apply_action_into(
                    &reloaded,
                    &catalog,
                    &crate::engine::Action::Select {
                        answer: crate::engine::SelectionAnswer::OptionIndex(1),
                    },
                    &mut Vec::new(),
                )
                .unwrap_or_else(|refusal| panic!("{label} resolves: {refusal:?}"));
                assert!(chosen.pending.is_none(), "{label}");
                assert_eq!(
                    hand_card_ids(&chosen, &catalog).last(),
                    Some(&expected[1]),
                    "{label}"
                );
            } else {
                let hand = hand_card_ids(&played, &catalog);
                assert_eq!(hand[hand.len() - count..], expected[..], "{label}");
            }
            assert_eq!(
                generation_rng(&played).counter,
                {
                    let mut rng = generation_rng(&state);
                    let mut shuffled = pool.clone();
                    rng.shuffle(&mut shuffled).unwrap();
                    rng.counter
                },
                "{label}: one complete shuffle of the recorded pool"
            );
        }
    }
}

/// Spectrum Shift's BeforeHandDraw listener draws the recorded profile's
/// Colorless pool (#2560): `SpectrumShiftPower/<BeforeHandDraw>d__4::MoveNext`
/// `0x345978` reads `GetUnlockedCards` at IL_005e and takes `Amount` of one
/// `GetDistinctForCombat` shuffle at IL_007e. The root holds Spectrum Shift, so
/// its closure interns the pool; the listener's selection is then called on
/// the admitted state directly.
#[test]
fn spectrum_shift_listener_draws_the_recorded_profile_colorless_pool() {
    for (hidden, partial) in [(&["COLORLESS5_EPOCH"][..], true), (&[][..], false)] {
        let (document, catalog, state) =
            colorless_root("CHARACTER.IRONCLAD", "SPECTRUM_SHIFT", hidden);
        crate::engine::admit(&document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("hidden={hidden:?}: {refusal}"));
        let pool = catalog.colorless_generation_pool();
        assert_eq!(
            COLORLESS5_ROWS.iter().all(|row| !pool.contains(row)),
            partial
        );
        let expected = distinct_prefix(&mut generation_rng(&state), &pool, 2);
        let mut live = state.clone();
        let selected = crate::engine::cards::select_spectrum_shift_cards(&mut live, &catalog, 2)
            .expect("the listener selects");
        assert_eq!(
            selected
                .iter()
                .map(|identity| identity.id)
                .collect::<Vec<_>>(),
            expected,
            "hidden={hidden:?}"
        );
    }
}

/// The profile the generation-potion witnesses hide: `DEFECT2_EPOCH` gates a
/// Defect Attack (`NULL`) and two Powers (`CONSUMING_SHADOW`, `LOOP`),
/// `DEFECT7_EPOCH` an Attack (`HELIX_DRILL`) and two Skills (`SCAVENGE`,
/// `TURBO`) (`DefectCardPool::FilterThroughEpochs` `0xf1944`), and
/// `COLORLESS5_EPOCH` three Colorless rows. So each of the four pools a
/// generation potion reads is strictly smaller than its fully-unlocked one.
const GENERATION_POTION_HIDDEN: [&str; 3] = ["DEFECT2_EPOCH", "DEFECT7_EPOCH", "COLORLESS5_EPOCH"];

/// A `character` root on `hidden`'s profile holding `potion` in belt slot 0.
fn generation_potion_root(
    character: &str,
    potion: &str,
    hidden: &[&str],
) -> (
    crate::canonical::CanonicalStateV2,
    crate::catalog::Catalog,
    crate::hot::HotState,
) {
    let mut case = with_hidden_epochs(owner_deck_with(character, &[]), hidden);
    case["save"]["players"][0]["potions"] =
        Value::Array(vec![potion_row(&format!("POTION.{potion}"), 0)]);
    let (mut document, _, _) = owner_pool_root(&case, &[]);
    // As in `colorless_root`: the Colorless pool's closure reaches Entropy,
    // Black Hole and the Regent Star writers, so admission needs the
    // (empty, turn-1) AfterEnergyReset acquisition order a real checkpoint
    // carries (#2669).
    document.player.insert(
        "after_energy_reset_order".to_string(),
        Value::Array(Vec::new()),
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document)
        .expect("the Rust-opened root builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog)
        .expect("the Rust-opened root hydrates");
    (document, catalog, state)
}

fn use_potion_zero(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
) -> Result<crate::hot::HotState, crate::engine::EngineRefusal> {
    crate::engine::apply_action_into(
        state,
        catalog,
        &crate::engine::Action::UsePotion {
            slot: 0,
            target: None,
        },
        &mut Vec::new(),
    )
}

/// The four choose-a-card generation potions draw the RECORDED profile's pool
/// with the native Generation RNG (#3141).
///
/// `AttackPotion` / `SkillPotion` / `PowerPotion` `<OnUse>d__6::MoveNext`
/// (`0x34bdb4` / `0x3502bc` / `0x34fb58`) read `target.Player.Character.
/// CardPool` through `GetUnlockedCards(UnlockState, ..)` (IL_005b), keep one
/// `CardType` (`Where` IL_007f) and offer the first three of one
/// `GetDistinctForCombat` shuffle (IL_0095); `ColorlessPotion` (`0x34c784`)
/// does the same over `CardPool<ColorlessCardPool>` (IL_003f/IL_0055/IL_006b)
/// with no `Where`. On the partial profile every pool loses its hidden rows,
/// so the offer, the pending decoder's accepted options and the RNG advance
/// are those of the SMALLER pool; the control reveals every epoch and draws
/// the fully-unlocked pool. Expected cards come from the pre-use Generation
/// stream through the native `Rng` port.
#[test]
fn generation_potions_draw_the_recorded_profile_pool() {
    use crate::ids::{CardId, PotionId};
    for (hidden, partial) in [(&GENERATION_POTION_HIDDEN[..], true), (&[][..], false)] {
        for potion in [
            PotionId::AttackPotion,
            PotionId::SkillPotion,
            PotionId::PowerPotion,
            PotionId::ColorlessPotion,
        ] {
            let label = format!("{potion:?} hidden={hidden:?}");
            let (document, catalog, state) =
                generation_potion_root("CHARACTER.DEFECT", potion.as_str(), hidden);
            assert_eq!(catalog.splash_unlock_epochs().is_some(), partial, "{label}");
            crate::engine::admit(&document, &state, &catalog)
                .unwrap_or_else(|refusal| panic!("{label}: {refusal}"));
            let owner = Some(crate::catalog::RewardPool::Defect);
            let full = crate::engine::potions::generation_choice_pool(potion, owner, None).unwrap();
            let pool = crate::engine::potions::generation_choice_pool(
                potion,
                owner,
                catalog.splash_unlock_epochs(),
            )
            .unwrap();
            assert_eq!(
                crate::engine::potions::fight_generation_choice_pool(&state, &catalog, potion),
                Some(pool.clone()),
                "{label}"
            );
            if partial {
                assert!(pool.len() < full.len(), "{label}: the profile hides rows");
                assert!(pool.iter().all(|id| full.contains(id)), "{label}");
            } else {
                assert_eq!(pool, full, "{label}");
            }
            let expected = distinct_prefix(&mut generation_rng(&state), &pool, 3);
            let used = use_potion_zero(&state, &catalog)
                .unwrap_or_else(|refusal| panic!("{label} uses: {refusal:?}"));
            assert_eq!(
                generation_rng(&used).counter,
                {
                    let mut rng = generation_rng(&state);
                    let mut shuffled = pool.clone();
                    rng.shuffle(&mut shuffled).unwrap();
                    rng.counter
                },
                "{label}: one complete shuffle of the recorded pool"
            );
            let wire = crate::boundary::HotBoundary::to_canonical(&used, &catalog);
            assert_eq!(
                wire.player["pending"][0], "generation_potion_select",
                "{label}"
            );
            let offered: Vec<CardId> = wire.player["pending"][2]
                .as_array()
                .expect("the options")
                .iter()
                .map(|option| CardId::from_str(option[0].as_str().unwrap()).unwrap())
                .collect();
            assert_eq!(offered, expected, "{label}");
            // The pending decoder reads the same pool back.
            let reloaded = crate::boundary::HotBoundary::from_canonical(&wire, &catalog)
                .unwrap_or_else(|refusal| panic!("{label} decodes: {refusal:?}"));
            assert_eq!(reloaded, used, "{label}");
            assert!(
                crate::engine::potions::generation_pending_is_exact(&reloaded, &catalog),
                "{label}"
            );
            let chosen = crate::engine::apply_action_into(
                &reloaded,
                &catalog,
                &crate::engine::Action::Select {
                    answer: crate::engine::SelectionAnswer::OptionIndex(1),
                },
                &mut Vec::new(),
            )
            .unwrap_or_else(|refusal| panic!("{label} resolves: {refusal:?}"));
            assert!(chosen.pending.is_none(), "{label}");
            assert_eq!(
                hand_card_ids(&chosen, &catalog).last(),
                Some(&expected[1]),
                "{label}"
            );
            if partial {
                // A forged option the recorded profile hides is not an exact
                // pending screen, even though the catalog interns it (the
                // closure walk keeps the fully-unlocked superset).
                let hidden_row = full
                    .iter()
                    .copied()
                    .find(|id| !pool.contains(id))
                    .expect("a hidden row");
                let hidden_atom = catalog
                    .atom(&crate::catalog::CardIdentity {
                        id: hidden_row,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .expect("the hidden row is interned");
                let options: Vec<_> = used
                    .pending
                    .as_deref()
                    .and_then(|pending| pending.generation_potion_record(&used.frames))
                    .expect("a generation screen")
                    .generation_options()
                    .collect();
                let reframed = |options: Vec<crate::catalog::CardAtom>| {
                    let mut state = used.clone();
                    state
                        .frames
                        .replace_top_potion_finish(&crate::hot::PotionFinishRecord {
                            name: potion,
                            stage: crate::frame::PotionFinishStage::Effect,
                            body_stage: crate::hot::PotionBodyStage::GenerationSelecting,
                            current_uid: None,
                            aux: 0,
                            candidates: Vec::new(),
                            generation_options: options,
                        })
                        .expect("the top frame is the potion");
                    state
                };
                // Control: the recorded options re-framed are still exact.
                assert!(
                    crate::engine::potions::generation_pending_is_exact(
                        &reframed(options.clone()),
                        &catalog
                    ),
                    "{label}"
                );
                let mut forged_options = options;
                forged_options[0] = hidden_atom;
                assert!(
                    !crate::engine::potions::generation_pending_is_exact(
                        &reframed(forged_options),
                        &catalog
                    ),
                    "{label}: {hidden_row:?} is outside the recorded pool"
                );
                // And the boundary refuses the same forged screen on the wire.
                let mut forged = wire.clone();
                forged.player.get_mut("pending").unwrap()[2][0][0] =
                    Value::from(hidden_row.as_str());
                assert!(
                    crate::boundary::HotBoundary::from_canonical(&forged, &catalog).is_err(),
                    "{label}"
                );
            }
        }
    }
}

/// Orobic Acid draws one card from each of the recorded profile's Attack,
/// Skill and Power pools (#3141).
///
/// `OrobicAcid/<OnUse>d__6::MoveNext` `0x34f050` makes the three owner-pool
/// queries in that order (`GetUnlockedCards` IL_005b / IL_00bd / IL_011f,
/// `Where` predicates `0x34f02e` Attack / `0x34f039` Skill / `0x34f044`
/// Power), each `GetDistinctForCombat(.., 1, ..)` (IL_0095 / IL_00f7 /
/// IL_0159) on the one Generation stream, and adds all three to the hand.
#[test]
fn orobic_acid_draws_the_recorded_profile_pools() {
    use crate::ids::PotionId;
    for (hidden, partial) in [(&GENERATION_POTION_HIDDEN[..], true), (&[][..], false)] {
        let label = format!("hidden={hidden:?}");
        let (document, catalog, state) =
            generation_potion_root("CHARACTER.DEFECT", "OROBIC_ACID", hidden);
        crate::engine::admit(&document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("{label}: {refusal}"));
        let mut rng = generation_rng(&state);
        let mut expected = Vec::new();
        for potion in [
            PotionId::AttackPotion,
            PotionId::SkillPotion,
            PotionId::PowerPotion,
        ] {
            let pool =
                crate::engine::potions::fight_generation_choice_pool(&state, &catalog, potion)
                    .unwrap();
            let full = crate::engine::potions::generation_choice_pool(
                potion,
                Some(crate::catalog::RewardPool::Defect),
                None,
            )
            .unwrap();
            assert_eq!(pool.len() < full.len(), partial, "{label} {potion:?}");
            expected.extend(distinct_prefix(&mut rng, &pool, 1));
        }
        let used = use_potion_zero(&state, &catalog)
            .unwrap_or_else(|refusal| panic!("{label} uses: {refusal:?}"));
        let hand = hand_card_ids(&used, &catalog);
        assert_eq!(hand[hand.len() - 3..], expected[..], "{label}");
        assert_eq!(generation_rng(&used).counter, rng.counter, "{label}");
    }
}

/// A `character` root on `hidden`'s profile holding `potion` in belt slot 0
/// with `cards` added to its deck.
fn generation_potion_root_with(
    character: &str,
    potion: &str,
    cards: &[&str],
    hidden: &[&str],
) -> (
    crate::canonical::CanonicalStateV2,
    crate::catalog::Catalog,
    crate::hot::HotState,
) {
    let mut case = with_hidden_epochs(owner_deck_with(character, cards), hidden);
    case["save"]["players"][0]["potions"] =
        Value::Array(vec![potion_row(&format!("POTION.{potion}"), 0)]);
    let (mut document, _, _) = owner_pool_root(&case, &[]);
    document.player.insert(
        "after_energy_reset_order".to_string(),
        Value::Array(Vec::new()),
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document)
        .expect("the Rust-opened root builds a catalog");
    let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog)
        .expect("the Rust-opened root hydrates");
    (document, catalog, state)
}

const GENERATION_POTION_CHARACTERS: [&str; 5] = [
    "CHARACTER.IRONCLAD",
    "CHARACTER.SILENT",
    "CHARACTER.DEFECT",
    "CHARACTER.NECROBINDER",
    "CHARACTER.REGENT",
];

/// Every row a generation potion can offer is an L0 atom of the root's
/// catalog, for every owner, on a partial and on the full profile.
///
/// The bodies draw `fight_generation_choice_pool` (the recorded profile's
/// `GetUnlockedCards` + `Where` + `FilterForCombat` projection; `0x3502bc`
/// IL_005b/IL_007f/IL_0095 for Skill, the siblings cited on
/// `engine::potions::generation_choice_pool`), and the boundary interns the
/// fully-unlocked superset of each, so the offer can never name an
/// un-interned identity. Orobic Acid draws all three character pools.
#[test]
fn generation_potion_closures_intern_every_recorded_pool_row() {
    use crate::ids::PotionId;
    for character in GENERATION_POTION_CHARACTERS {
        for (hidden, label_profile) in [
            (&GENERATION_POTION_HIDDEN[..], "partial"),
            (&[][..], "full"),
        ] {
            for (potion, pools) in [
                (PotionId::AttackPotion, &[PotionId::AttackPotion][..]),
                (PotionId::SkillPotion, &[PotionId::SkillPotion][..]),
                (PotionId::PowerPotion, &[PotionId::PowerPotion][..]),
                (PotionId::ColorlessPotion, &[PotionId::ColorlessPotion][..]),
                (
                    PotionId::OrobicAcid,
                    &[
                        PotionId::AttackPotion,
                        PotionId::SkillPotion,
                        PotionId::PowerPotion,
                    ][..],
                ),
            ] {
                let label = format!("{character} {potion:?} {label_profile}");
                let (_, catalog, state) =
                    generation_potion_root(character, potion.as_str(), hidden);
                for &pool_potion in pools {
                    let pool = crate::engine::potions::fight_generation_choice_pool(
                        &state,
                        &catalog,
                        pool_potion,
                    )
                    .unwrap_or_else(|| panic!("{label}: an owner"));
                    assert!(!pool.is_empty(), "{label} {pool_potion:?}");
                    for id in pool {
                        assert!(
                            catalog
                                .atom(&crate::catalog::CardIdentity {
                                    id,
                                    upgrade: 0,
                                    enchantment: None,
                                })
                                .is_some(),
                            "{label}: {pool_potion:?} row {id:?} is not interned"
                        );
                    }
                }
            }
        }
    }
}

/// Census `DXLGWV6KZ1BF` node 16: a Skill Potion drunk before White Noise
/// moves the Generation stream White Noise's seeded one-row preview was read
/// from, so the preview no longer names the card the play mints (Coolant).
///
/// Both draw the live `CombatCardGeneration` stream at use/play time
/// (`SkillPotion/<OnUse>d__6` `0x3502bc` IL_0090/IL_0095,
/// `WhiteNoise/<OnPlay>d__3` `0x3c74ac` IL_0109/IL_010e;
/// `Distraction/<OnPlay>d__5` `0x399690` IL_008b/IL_0090). Beside a reachable
/// generation potion the boundary interns the owner's COMPLETE one-type pool
/// instead of the prefix, on a partial and on the full profile; a
/// non-generation potion (the control) keeps the one-row preview. End to end,
/// Skill Potion then White Noise mints the native post-potion pick.
#[test]
fn a_generation_potion_closes_white_noise_and_distraction_completely() {
    use crate::content_tables::CardType;
    use crate::ids::CardId;
    let l0 = |id| crate::catalog::CardIdentity {
        id,
        upgrade: 0,
        enchantment: None,
    };
    for (character, card, card_type) in [
        ("CHARACTER.DEFECT", "WHITE_NOISE", CardType::Power),
        ("CHARACTER.SILENT", "DISTRACTION", CardType::Skill),
    ] {
        for hidden in [&GENERATION_POTION_HIDDEN[..], &[][..]] {
            let label = format!("{character} {card} hidden={hidden:?}");
            let missing = |potion: &str| {
                let (_, catalog, state) =
                    generation_potion_root_with(character, potion, &[card], hidden);
                let owner = state.reward_card_pool.expect("an owner");
                catalog
                    .owner_type_generation_pool(owner, card_type)
                    .into_iter()
                    .filter(|id| catalog.atom(&l0(*id)).is_none())
                    .collect::<Vec<_>>()
            };
            for potion in [
                "SKILL_POTION",
                "ATTACK_POTION",
                "COLORLESS_POTION",
                "OROBIC_ACID",
            ] {
                assert_eq!(
                    missing(potion),
                    Vec::<CardId>::new(),
                    "{label} beside {potion}"
                );
            }
            assert!(
                !missing("BLOCK_POTION").is_empty(),
                "{label}: without a generation potion the preview stays a prefix"
            );
        }
    }

    // End to end: Skill Potion (option 0), then White Noise.
    let (document, catalog, state) = generation_potion_root_with(
        "CHARACTER.DEFECT",
        "SKILL_POTION",
        &["WHITE_NOISE"],
        &GENERATION_POTION_HIDDEN,
    );
    crate::engine::admit(&document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("admits: {refusal}"));
    let used = use_potion_zero(&state, &catalog).expect("the potion resolves");
    let mut chosen = crate::engine::apply_action_into(
        &used,
        &catalog,
        &crate::engine::Action::Select {
            answer: crate::engine::SelectionAnswer::OptionIndex(0),
        },
        &mut Vec::new(),
    )
    .expect("the pick resolves");
    assert_ne!(
        generation_rng(&chosen).counter,
        generation_rng(&state).counter,
        "the potion moved the Generation stream"
    );
    let white_noise = catalog.atom(&l0(CardId::WhiteNoise)).expect("White Noise");
    let uid = crate::hot::PileId::ALL
        .into_iter()
        .find_map(|pile| {
            let cards = chosen.piles.get(pile).as_slice();
            cards
                .iter()
                .position(|card| card.atom == white_noise)
                .map(|index| (pile, index))
        })
        .map(|(pile, index)| {
            let card = chosen.piles.get_mut(pile).make_mut().remove(index);
            chosen
                .piles
                .get_mut(crate::hot::PileId::Hand)
                .make_mut()
                .push(card);
            card.uid
        })
        .expect("White Noise is in the deck");
    chosen.energy = 3;
    let owner = chosen.reward_card_pool.unwrap();
    let pool = catalog.owner_type_generation_pool(owner, CardType::Power);
    let expected = distinct_prefix(&mut generation_rng(&chosen), &pool, 1)[0];
    let played = play(&chosen, &catalog, uid, None)
        .unwrap_or_else(|refusal| panic!("White Noise plays: {refusal:?}"));
    assert_eq!(hand_card_ids(&played, &catalog).last(), Some(&expected));
}

/// The other seeded previews close completely beside a generation potion
/// too: Infernal Blade (`0x3a7164` IL_008b/IL_0090, the frozen Attack pool at
/// L0), Bundle of Joy (`0x390188` IL_0063/IL_0068, the Colorless pool at L0)
/// and Abundance (`0x3888ec` IL_0092/IL_0097, the owner's Power pool at L1).
/// Each draws the live Generation stream at play time, as every generation
/// potion does at use time. A non-generation potion (the control) keeps each
/// one-shuffle prefix. Jack of All Trades deliberately keeps its prefix (see
/// `boundary::intern_generation_preview`).
#[test]
fn a_generation_potion_closes_every_seeded_preview_completely() {
    use crate::ids::CardId;
    let abundance_defect = crate::content_tables::ABUNDANCE_POWER_POOLS_V1101
        .iter()
        .find_map(|(name, pool)| (*name == "Defect").then_some(pool.to_vec()))
        .unwrap();
    type Pool = fn(&crate::catalog::Catalog) -> Vec<CardId>;
    let cases: [(&str, &str, Pool, u8); 3] = [
        (
            "CHARACTER.IRONCLAD",
            "INFERNAL_BLADE",
            |_| crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109.to_vec(),
            0,
        ),
        (
            "CHARACTER.DEFECT",
            "BUNDLE_OF_JOY",
            |catalog| catalog.colorless_generation_pool(),
            0,
        ),
        ("CHARACTER.DEFECT", "ABUNDANCE", |_| Vec::new(), 1),
    ];
    for (character, card, pool_of, upgrade) in cases {
        let missing = |potion: &str| {
            let (_, catalog, _) = generation_potion_root_with(character, potion, &[card], &[]);
            let pool = if card == "ABUNDANCE" {
                abundance_defect.clone()
            } else {
                pool_of(&catalog)
            };
            assert!(pool.len() > 4, "{card}: a pool larger than any prefix");
            pool.into_iter()
                .filter(|id| {
                    catalog
                        .atom(&crate::catalog::CardIdentity {
                            id: *id,
                            upgrade,
                            enchantment: None,
                        })
                        .is_none()
                })
                .collect::<Vec<_>>()
        };
        for potion in [
            "SKILL_POTION",
            "POWER_POTION",
            "COLORLESS_POTION",
            "OROBIC_ACID",
        ] {
            assert_eq!(
                missing(potion),
                Vec::<CardId>::new(),
                "{card} beside {potion}"
            );
        }
        assert!(
            !missing("BLOCK_POTION").is_empty(),
            "{card}: without a generation potion the preview stays a prefix"
        );
    }
}

/// A document that records NO profile still refuses every generation potion
/// by name, at admission and at use (#3141): the pools are profile-filtered,
/// so an unrecorded profile has no exact pool. Entropic Brew, which can
/// procure any of the four, refuses under its own name.
#[test]
fn an_unrecorded_profile_refuses_the_generation_potions_by_name() {
    for (potion, missing) in [
        ("ATTACK_POTION", "generation potion pool provenance"),
        ("SKILL_POTION", "generation potion pool provenance"),
        ("POWER_POTION", "generation potion pool provenance"),
        ("COLORLESS_POTION", "generation potion pool provenance"),
        ("OROBIC_ACID", "Orobic generation provenance"),
        ("ENTROPIC_BREW", "Entropic generation-potion provenance"),
    ] {
        let (mut document, _, _) =
            generation_potion_root("CHARACTER.DEFECT", potion, &GENERATION_POTION_HIDDEN);
        assert!(document.player.remove("splash_unlock_epochs").is_some());
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert!(catalog.splash_unlock_epochs().is_none());
        assert!(!state.fully_unlocked_card_pool_epochs);
        let refusal =
            crate::engine::admit(&document, &state, &catalog).expect_err("no recorded profile");
        let text = refusal.to_string();
        assert!(text.contains(missing), "{potion}: {text}");
        assert!(
            !text.contains("Part D potion prerequisites"),
            "{potion}: the named prerequisite replaces the bundled name: {text}"
        );
        assert!(
            use_potion_zero(&state, &catalog).is_err(),
            "{potion}: the use refuses too"
        );
    }
}

/// Entropic Brew on a partial profile now admits (#3141): its procurement
/// gate requires each generation potion it can procure to have exact pool
/// provenance, which a recorded partial profile now gives. The three corpus
/// fights this freed (`fe3c8051b05a7ded`, `f8b9e283ddd0bc9c`,
/// `f4d10c8afe9771b3`) are Ironclad, and so is this root (a Defect owner's
/// factory pool reaches a potion body outside the modeled set and refuses
/// "owner potion generation body closure", which is not this witness); the
/// profile hides `COLORLESS5_EPOCH`, so its Colorless Potion pool is filtered.
#[test]
fn entropic_brew_admits_on_a_recorded_partial_profile() {
    let (document, catalog, state) = generation_potion_root(
        "CHARACTER.IRONCLAD",
        "ENTROPIC_BREW",
        &GENERATION_POTION_HIDDEN,
    );
    assert!(catalog.splash_unlock_epochs().is_some());
    crate::engine::admit(&document, &state, &catalog).unwrap_or_else(|refusal| panic!("{refusal}"));
}

/// The opening hands its root the AfterEnergyReset ledger (#3687).
///
/// The reported fight is a fully-unlocked Silent holding Entropic Brew
/// (seanb/J8JL0SC7A2JX floor 31). Entropic Brew can procure Colorless Potion,
/// whose pool reaches Entropy, whose five-class transform closure reaches
/// Black Hole, the Regent Star writers and an energy-next-turn peer: the
/// three legs of admission's `terminal star/orb AfterEnergyReset order` gate,
/// all from the catalog, with no listener live.
///
/// The opened root admits because it records the entry's empty acquisition
/// order. The same root without the field is the pre-#3687 production root,
/// and it still refuses by name: an arbitrary checkpoint that does not say
/// its order is not an entry.
#[test]
fn the_opened_root_carries_the_after_energy_reset_ledger() {
    const ORDER: &str = "after_energy_reset_order";
    let mut case = owner_deck_with("CHARACTER.SILENT", &[]);
    case["save"]["players"][0]["potions"] =
        Value::Array(vec![potion_row("POTION.ENTROPIC_BREW", 0)]);
    let pre_hook = pre_hook(&case).expect("the pre-hook opening builds");
    assert_eq!(
        pre_hook.document.player.get(ORDER),
        Some(&Value::Array(Vec::new()))
    );
    let opening = open_with_reset_ledger(&case).expect("opens");
    let document = &opening.document;
    assert_eq!(document.player.get(ORDER), Some(&Value::Array(Vec::new())));
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(document, &catalog).unwrap();
    assert_eq!(state.fanouts.after_energy_reset_order(), Some(&[][..]));
    assert!(state.fanouts.after_energy_reset_order_is_explicit());
    crate::engine::admit(document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the opened root admits: {refusal}"));
    assert_eq!(
        &crate::boundary::HotBoundary::to_canonical(&state, &catalog),
        document
    );

    let mut legacy = document.clone();
    assert!(legacy.player.remove(ORDER).is_some());
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&legacy).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(&legacy, &catalog).unwrap();
    assert_eq!(state.fanouts.after_energy_reset_order(), None);
    let refusal = crate::engine::admit(&legacy, &state, &catalog).expect_err("no recorded order");
    assert!(
        refusal.contains(crate::engine::admission::MissingCapability::ArgumentShape(
            "terminal star/orb AfterEnergyReset order"
        )),
        "{refusal}"
    );
}

/// Abundance offers the recorded profile's owner-Power pool (#3285), on both
/// profiles, and a partial-profile pending screen round-trips the boundary.
///
/// `Abundance/<OnPlay>d__6::MoveNext` `0x3888ec`: `Owner.Character.CardPool`
/// through `GetUnlockedCards(Owner.UnlockState, ..)` (IL_0027-IL_0058), the
/// Power `Where` (IL_005d-IL_007c, predicate `0x3888de`), then the first three
/// of one `GetDistinctForCombat` shuffle (IL_0081-IL_0097), each upgraded
/// (`CardCmd::Upgrade` IL_00b8). This profile hides `DEFECT5_EPOCH` and
/// `DEFECT7_EPOCH`, which removes `SMOKESTACK` from the Defect Power pool, so
/// the partial root shuffles a shorter pool than the frozen
/// `ABUNDANCE_POWER_POOLS_V1101` row; the fully-unlocked control shuffles
/// exactly that row. Until #3285 the partial root refused by name
/// (`PartialUnlockProfile(ABUNDANCE)`).
///
/// The partial arm then round-trips the suspended screen:
/// `to_canonical` -> a fresh `catalog_from_canonical` + `from_canonical` (the
/// pending decoder checks each option against the recorded pool) ->
/// `to_canonical` is the identity, and the rebuilt state resolves the screen
/// to the chosen upgraded Power.
#[test]
fn abundance_offers_the_recorded_profile_owner_power_pool() {
    use crate::content_tables::CardType;
    use crate::ids::CardId;
    let frozen = crate::content_tables::ABUNDANCE_POWER_POOLS_V1101
        .iter()
        .find_map(|(name, pool)| (*name == "Defect").then_some(pool.to_vec()))
        .unwrap();
    assert!(frozen.contains(&CardId::Smokestack));
    for (hidden, partial) in [
        (&["DEFECT5_EPOCH", "DEFECT7_EPOCH"][..], true),
        (&[][..], false),
    ] {
        let case = with_hidden_epochs(owner_deck_with("CHARACTER.DEFECT", &["ABUNDANCE"]), hidden);
        let (document, catalog, state) = owner_pool_root(&case, &["ABUNDANCE"]);
        assert_eq!(catalog.splash_unlock_epochs().is_some(), partial);
        assert_eq!(state.fully_unlocked_card_pool_epochs, !partial);
        crate::engine::admit(&document, &state, &catalog)
            .unwrap_or_else(|refusal| panic!("hidden={hidden:?}: {refusal}"));
        let pool =
            catalog.owner_type_generation_pool(crate::catalog::RewardPool::Defect, CardType::Power);
        assert_eq!(!pool.contains(&CardId::Smokestack), partial);
        assert_eq!(pool == frozen, !partial, "hidden={hidden:?}");

        let expected = distinct_prefix(&mut generation_rng(&state), &pool, 3);
        let played = play(
            &state,
            &catalog,
            hand_card_uid(&state, &catalog, CardId::Abundance),
            None,
        )
        .unwrap_or_else(|refusal| panic!("Abundance plays: {refusal:?}"));
        let wire = crate::boundary::HotBoundary::to_canonical(&played, &catalog);
        assert_eq!(wire.player["pending"][0], "abundance_select");
        let offered: Vec<(CardId, i64)> = wire.player["pending"][3]
            .as_array()
            .expect("the options")
            .iter()
            .map(|option| {
                (
                    CardId::from_str(option[0].as_str().unwrap()).unwrap(),
                    option[1].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            offered,
            expected.iter().map(|id| (*id, 1)).collect::<Vec<_>>(),
            "hidden={hidden:?}"
        );
        if !partial {
            continue;
        }
        let rebuilt_catalog = crate::boundary::HotBoundary::catalog_from_canonical(&wire)
            .expect("the suspended partial-profile screen builds a catalog");
        let rebuilt = crate::boundary::HotBoundary::from_canonical(&wire, &rebuilt_catalog)
            .expect("the suspended partial-profile screen hydrates");
        assert!(rebuilt_catalog.splash_unlock_epochs().is_some());
        assert_eq!(
            crate::boundary::HotBoundary::to_canonical(&rebuilt, &rebuilt_catalog),
            wire
        );
        crate::engine::admit(&wire, &rebuilt, &rebuilt_catalog)
            .unwrap_or_else(|refusal| panic!("the suspended screen admits: {refusal}"));
        let resolved = crate::engine::apply_action_into(
            &rebuilt,
            &rebuilt_catalog,
            &crate::engine::Action::Select {
                answer: crate::engine::SelectionAnswer::OptionIndex(1),
            },
            &mut Vec::new(),
        )
        .expect("the rebuilt screen resolves");
        let generated = *resolved
            .piles
            .get(crate::hot::PileId::Hand)
            .as_slice()
            .last()
            .unwrap();
        assert_eq!(
            rebuilt_catalog.spec(generated.atom).unwrap().identity,
            crate::catalog::CardIdentity {
                id: expected[1],
                upgrade: 1,
                enchantment: None,
            }
        );
    }
}

/// A document that records NO profile still refuses every widened generator
/// by name, at admission and at the body (#2560).
///
/// Deleting `player.splash_unlock_epochs` leaves neither the fully-unlocked
/// flag nor a catalog profile, so `unlock_profile_is_recorded` is false.
#[test]
fn an_unrecorded_profile_still_refuses_the_owner_pool_generators_by_name() {
    use crate::ids::CardId;
    for (card, missing) in [
        ("DISCOVERY", "Discovery generation provenance"),
        ("JACKPOT", "jackpot generation provenance"),
        ("DISTRACTION", "distraction generation provenance"),
        ("WHITE_NOISE", "white noise generation provenance"),
        // The Colorless half of #2560: the recorded-profile Colorless pool
        // needs a recorded profile just the same.
        (
            "JACK_OF_ALL_TRADES",
            "jack of all trades generation provenance",
        ),
        ("BUNDLE_OF_JOY", "bundle of joy generation provenance"),
        (
            "MANIFEST_AUTHORITY",
            "manifest authority generation provenance",
        ),
    ] {
        let case = owner_deck_with("CHARACTER.DEFECT", &[card]);
        let (mut document, _, _) = owner_pool_root(&case, &[card]);
        assert!(document.player.remove("splash_unlock_epochs").is_some());
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = crate::boundary::HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert!(catalog.splash_unlock_epochs().is_none());
        assert!(!state.fully_unlocked_card_pool_epochs);
        let refusal =
            crate::engine::admit(&document, &state, &catalog).expect_err("no recorded profile");
        assert!(refusal.to_string().contains(missing), "{card}: {refusal}");
        let id = CardId::from_str(card).unwrap();
        let target = (card == "JACKPOT").then_some(0);
        assert!(
            play(
                &state,
                &catalog,
                hand_card_uid(&state, &catalog, id),
                target
            )
            .is_err(),
            "{card}: the body refuses too"
        );
    }
}

/// Vexing Puzzlebox's Necrobinder root now admits (#2946): the relic reads its
/// OWNER's pool (`VexingPuzzlebox/<AfterPlayerTurnStart>d__2::MoveNext`
/// `0x333de0` IL_005d-IL_0082), and admission checks the Necrobinder closure
/// (`boundary::vexing_puzzlebox_owner_closure_is_exact`) instead of the
/// Ironclad Stoke closure. The profile hides `DEFECT7_EPOCH`, which gates no
/// Necrobinder row.
#[test]
fn a_necrobinder_vexing_puzzlebox_root_admits_on_its_recorded_profile() {
    let (document, catalog, state) = owner_pool_root(&puzzlebox_case("CHARACTER.NECROBINDER"), &[]);
    assert!(catalog.splash_unlock_epochs().is_some());
    crate::engine::admit(&document, &state, &catalog).unwrap_or_else(|refusal| panic!("{refusal}"));
    assert!(crate::boundary::vexing_puzzlebox_owner_closure_is_exact(
        &catalog,
        crate::catalog::RewardPool::Necrobinder
    ));
}

// ---------------------------------------------------------------------------
// #3162: Jeweled Mask, Funerary Mask, Radiant Pearl, Big Mushroom, Tea of
// Discourtesy, Planisphere and Pantograph
// ---------------------------------------------------------------------------
//
// The frozen Python oracle that printed the parity digests above is deleted
// (#2827), so these witnesses pin the IL's observable effect piecewise. The
// certification witness is the eval census: every corpus capture holding one
// of these relics that now roots matches its own `.mcr` opening checkpoint
// (tabulated in the #3162 invariant walk).

fn open_facts(entry: &crate::entry::document::EntryDocument) -> Result<Opening, OpeningRefusal> {
    build_opening(entry, &OpeningOptions::default())
}

fn pile_ids_and_uids(
    document: &crate::canonical::CanonicalStateV2,
    pile: &str,
) -> Vec<(String, Option<u64>)> {
    document.piles[pile]
        .iter()
        .map(|card| (card.id.clone(), card.uid))
        .collect()
}

/// `JeweledMask/<BeforeHandDraw>d__2::MoveNext` (`0x3272f4`) moves the one
/// deck Power from Draw to the Hand's bottom before the deal, so the Hand is
/// that Power followed by a five-card prefix draw of the rest, and the uid
/// check accepts it only through `jeweled_mask_moved_order`.
#[test]
fn a_jeweled_mask_save_lifts_its_power_to_the_hand_before_the_deal() {
    let control = open(&with_deck_front(&[deck_card("INFLAME")])).expect("the control opens");
    let mut case = with_deck_front(&[deck_card("INFLAME")]);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JEWELED_MASK"}),
    );
    let opening = open(&case).expect("Jeweled Mask opens");
    let hand = pile_ids_and_uids(&opening.document, "hand");
    assert_eq!(hand.len(), 6, "{hand:?}");
    assert_eq!(hand[0].0, "INFLAME", "{hand:?}");
    let control_cards: Vec<_> = pile_ids_and_uids(&control.document, "hand")
        .into_iter()
        .chain(pile_ids_and_uids(&control.document, "draw"))
        .filter(|card| card.0 != "INFLAME")
        .collect();
    let dealt: Vec<_> = hand[1..].iter().map(|card| card.0.clone()).collect();
    let prefix: Vec<_> = control_cards[..5]
        .iter()
        .map(|card| card.0.clone())
        .collect();
    assert_eq!(
        dealt, prefix,
        "the deal is a prefix of the pile without the Power"
    );
    assert_eq!(
        opening.document.rng["sel"].counter,
        control.document.rng["sel"].counter + 1,
        "one NextItem draw"
    );
}

/// #3176: the lifted Power is free on the canonical root, not only inside the
/// opening's hot state. `CardModel::SetToFreeThisTurn` (`0x7d3c9`, called at
/// `JeweledMask/<BeforeHandDraw>d__2::MoveNext` `0x3272f4` `IL_00f3`) appends
/// an Energy `ThisTurnOrPlayed` Set-0 row (`CardEnergyCost::
/// SetThisTurnOrUntilPlayed` `0x11e1d6`) and a temporary Star row; nothing
/// clears them before the turn ends or the card is played. Before the fix the
/// rows were live in the engine and absent from the emitted document, so a
/// reloaded root charged Demon Form's full 3 and the recorded line ran out of
/// Energy (the census's `NetPlayCardAction is not legal`).
#[test]
fn a_jeweled_mask_power_stays_free_through_the_canonical_root() {
    let mut case = with_deck_front(&[deck_card("DEMON_FORM")]);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JEWELED_MASK"}),
    );
    let opening = open(&case).expect("Jeweled Mask opens");
    let power = opening.document.piles["hand"][0].clone();
    assert_eq!(power.id, "DEMON_FORM");
    let physical = power
        .physical_state
        .as_ref()
        .expect("the free rows project as slot 7");
    assert_eq!(
        physical,
        &serde_json::json!([
            "PHYSICAL_CARD_STATE",
            [[1, 0, 6, false]],
            0,
            false,
            [[0, true, true]]
        ]),
        "one Energy ThisTurnOrPlayed Set-0 row and one temporary Star row"
    );
    // The round trip is the identity, and the hydrated card is still free.
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .expect("the root builds a catalog");
    let mut state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
        .expect("the root hydrates");
    assert_eq!(
        crate::boundary::HotBoundary::to_canonical(&state, &catalog),
        opening.document
    );
    let uid = u32::try_from(power.uid.expect("a deck uid")).expect("a small uid");
    assert_eq!(
        state
            .card_states
            .get(uid)
            .free_star_cost_this_turn_or_played_rows,
        1
    );
    // Free, so it plays on zero Energy and leaves the Energy untouched.
    state.energy = 0;
    let play = crate::engine::Action::Play {
        uid,
        target: None,
        selection: crate::engine::SelectionRef::NONE,
    };
    assert!(
        crate::engine::legal_actions(&state, &catalog).contains(&play),
        "the lifted Demon Form is playable on zero Energy"
    );
    let played = crate::engine::apply_action_into(&state, &catalog, &play, &mut Vec::new())
        .expect("the free Power plays");
    assert_eq!(played.energy, 0, "it cost nothing");
    assert!(
        played.powers.value(crate::ids::PowerId::DemonForm) > 0,
        "Demon Form resolved"
    );
}

/// #3178 x #3176 cross-slice witness: a `ROYALLY_APPROVED` Attack beside a
/// Power under Jeweled Mask. The two opening moves compose without touching
/// each other. Jeweled Mask's `BeforeHandDraw` (`0x3272f4`) runs first, on
/// the shuffled pile (#3404: `SetupPlayerTurn` `0x3f6c6c` IL_0177 precedes
/// the fixup), and filters Powers only (`<>c::<BeforeHandDraw>b__2_1`
/// `0x3272e1`); `RoyallyApproved::CanEnchantCardType` `0xd6290` keeps the
/// enchantment off Powers, so it lifts Inflame out of the pile. The Innate
/// fixup (`<>c::<SetupPlayerTurn>b__102_1` `0x3f256f`) then lifts the
/// enchanted Anger to the pile front, numbered 0 by the fixed-pile stamping. The Hand is Inflame followed by a prefix deal that opens with Anger,
/// and the opening's uid check accepts that order through
/// `jeweled_mask_moved_order`.
#[test]
fn a_royally_approved_card_and_a_jeweled_mask_power_compose_at_the_opening() {
    let mut case = with_deck_front(&[
        stone_row("ANGER", 0, Some(("ROYALLY_APPROVED", 1))),
        deck_card("INFLAME"),
    ]);
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JEWELED_MASK"}),
    );
    let opening = open(&case).expect("Jeweled Mask with a Royally Approved card opens");
    let hand = pile_ids_and_uids(&opening.document, "hand");
    assert_eq!(hand.len(), 6, "{hand:?}");
    assert_eq!(
        hand[0].0, "INFLAME",
        "the lifted Power heads the Hand: {hand:?}"
    );
    assert_eq!(
        hand[1],
        ("ANGER".to_string(), Some(0)),
        "the Innate front is dealt first and keeps uid 0"
    );
    assert_eq!(
        opening.document.player.get("innate_min_draw"),
        Some(&Value::from(1))
    );
    assert_eq!(
        opening.document.piles["hand"][1].enchantment,
        Some(serde_json::json!(["ROYALLY_APPROVED", 1]))
    );
}

/// `jeweled_mask_moved_order` is the identity unless Jeweled Mask is owned
/// and the Hand's head is a deck Power; then that uid moves to the front.
#[test]
fn the_jeweled_mask_reorder_moves_only_a_deck_power_hand_head() {
    use super::jeweled_mask_moved_order;
    let opened = open(&with_deck_front(&[deck_card("INFLAME")]))
        .expect("the control opens")
        .document;
    let expected: Vec<Option<u64>> = (0..11).map(Some).collect();
    let owned = ["RELIC.JEWELED_MASK".to_string()];
    let mut lifted = opened.clone();
    {
        let hand = lifted.piles.get_mut("hand").expect("hand");
        hand[0].id = "INFLAME".to_string();
        hand[0].upgrade = 0;
        hand[0].uid = Some(7);
    }
    assert_eq!(
        jeweled_mask_moved_order(&lifted, expected.clone(), &[]),
        expected,
        "unowned"
    );
    let mut want = expected.clone();
    let seven = want.remove(7);
    want.insert(0, seven);
    assert_eq!(
        jeweled_mask_moved_order(&lifted, expected.clone(), &owned),
        want
    );
    let mut not_power = lifted.clone();
    not_power.piles.get_mut("hand").expect("hand")[0].id = "STRIKE_IRONCLAD".to_string();
    assert_eq!(
        jeweled_mask_moved_order(&not_power, expected.clone(), &owned),
        expected,
        "a non-Power head"
    );
    let mut fresh = lifted;
    fresh.piles.get_mut("hand").expect("hand")[0].uid = Some(42);
    assert_eq!(
        jeweled_mask_moved_order(&fresh, expected.clone(), &owned),
        expected,
        "a head that is not a deck uid"
    );
}

/// `RadiantPearl/<BeforeHandDraw>d__6::MoveNext` (`0x32f10c`) generates one
/// Luminesce into Hand before the deal: it heads the Hand with the first uid
/// past the deck, and the deal follows it unchanged.
#[test]
fn a_radiant_pearl_save_heads_the_hand_with_one_fresh_luminesce() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.RADIANT_PEARL")).expect("Radiant Pearl opens");
    let hand = pile_ids_and_uids(&opening.document, "hand");
    let deck = (hand.len() - 1 + opening.document.piles["draw"].len()) as u64;
    assert_eq!(hand[0], ("LUMINESCE".to_string(), Some(deck)), "{hand:?}");
    assert_eq!(hand[1..], pile_ids_and_uids(&control.document, "hand")[..]);
    assert_eq!(
        opening.document.player.get("next_card_uid"),
        Some(&Value::from(deck + 1))
    );
}

/// `FuneraryMask/<BeforeHandDraw>d__6::MoveNext` (`0x325000`) inserts three
/// fresh Souls at random Draw positions before the deal, with the next three
/// uids in creation order; the deal can reach them.
#[test]
fn a_funerary_mask_save_shuffles_three_souls_into_draw_before_the_deal() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.FUNERARY_MASK")).expect("Funerary Mask opens");
    let deck = (hand_size(&control.document) + control.document.piles["draw"].len()) as u64;
    let mut souls: Vec<u64> = opening
        .document
        .piles
        .values()
        .flatten()
        .filter(|card| card.id == "SOUL")
        .filter_map(|card| card.uid)
        .collect();
    souls.sort_unstable();
    assert_eq!(souls, [deck, deck + 1, deck + 2]);
    assert_eq!(hand_size(&opening.document), 5);
    assert_eq!(
        opening.document.rng["rng"].counter,
        control.document.rng["rng"].counter + 3,
        "one Shuffle-stream draw per Soul"
    );
}

/// #3404: with a turn-one Draw-order relic owned, the pre-hook pile stays in
/// shuffled order and the Innate fixup runs in the engine after the relic
/// (`CombatManager/<SetupPlayerTurn>d__102::MoveNext` `0x3f6c6c`:
/// `BeforeHandDraw` IL_0177, the fixup IL_02b8-IL_0360).
///
/// Rows: `CARD.WRITHE` beside Funerary Mask. The shuffle leaves Writhe at
/// pile position 4 (`an_innate_card_the_shuffle_left_mid_pile_...`). Its
/// pre-hook row stays there, carrying the uid the fixed pile numbers it
/// with (0): sorting the pre-hook pile by uid is exactly the relic-less
/// control's build-time-fixed pile. After the deal Writhe heads the Hand
/// with uid 0, and removing it and the three Souls leaves the deck in the
/// shuffled relative order. The census witness is RAB8SE1H26ZH n47, whose
/// opening now matches its native card piles.
#[test]
fn a_funerary_mask_inserts_its_souls_before_the_innate_fixup() {
    let control_case = with_deck_front(&[deck_card("WRITHE")]);
    let mut case = control_case.clone();
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.FUNERARY_MASK"}),
    );
    let control = pre_hook(&control_case).expect("the control pre-hook builds");
    let built = pre_hook(&case).expect("the Funerary Mask pre-hook builds");
    assert_eq!(built.pile, control.pile, "the same shuffle");
    let draw = &built.document.piles["draw"];
    assert_eq!(
        (draw[4].id.as_str(), draw[4].uid),
        ("WRITHE", Some(0)),
        "the pre-hook pile keeps the shuffled order"
    );
    let mut by_uid = draw.clone();
    by_uid.sort_by_key(|card| card.uid);
    assert_eq!(by_uid, control.document.piles["draw"]);
    assert_eq!(control.document.piles["draw"][0].id, "WRITHE");

    let opening = open(&case).expect("Funerary Mask with an Innate card opens");
    assert_eq!(
        pile_ids_and_uids(&opening.document, "hand")[0],
        ("WRITHE".to_string(), Some(0))
    );
    assert_eq!(
        opening.document.player.get("innate_min_draw"),
        Some(&Value::from(1))
    );
    let dealt_deck: Vec<Option<u64>> = opening.document.piles["hand"]
        .iter()
        .chain(&opening.document.piles["draw"])
        .filter(|card| card.id != "SOUL" && card.id != "WRITHE")
        .map(|card| card.uid)
        .collect();
    let shuffled_deck: Vec<Option<u64>> = draw
        .iter()
        .filter(|card| card.id != "WRITHE")
        .map(|card| card.uid)
        .collect();
    assert_eq!(dealt_deck, shuffled_deck);
}

/// #3404: Jeweled Mask lifts an Innate Power out of the shuffled Draw before
/// the deferred fixup, so native clamps the hand draw to the Innate cards
/// left in Draw (`SetupPlayerTurn` `0x3f6c6c` IL_038b-IL_0395), not to the
/// deck's count. With the only Innate card lifted the clamp is the plain hand
/// draw either way and the opening admits; with more Innate cards than the
/// hand draw, the two clamps differ and the opening refuses by name.
#[test]
fn a_jeweled_mask_innate_power_lift_clamps_the_deferred_draw() {
    let mut one = with_deck_front(&[stone_row("AGGRESSION", 1, None)]);
    relic(
        &mut one,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JEWELED_MASK"}),
    );
    let opening = open(&one).expect("one lifted Innate Power opens");
    assert_eq!(opening.document.piles["hand"][0].id, "AGGRESSION");
    assert_eq!(hand_size(&opening.document), 6);
    assert_eq!(
        opening.document.player.get("innate_min_draw"),
        Some(&Value::from(1))
    );

    let mut six = with_deck_front(&stone_rows("AGGRESSION", 1, 6));
    relic(
        &mut six,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.JEWELED_MASK"}),
    );
    let refusal = open(&six).expect_err("the two clamps differ");
    assert!(
        format!("{refusal:?}").contains("deferred turn-one Innate draw clamp"),
        "{refusal:?}"
    );
}

/// #3404: which openings defer the fixup. One of the three turn-one
/// Draw-order relics, and none of the three owners whose uid stamping has its
/// own contract (Thieving Hopper, Ghost Seed, Tea of Discourtesy).
#[test]
fn only_a_draw_order_relic_without_a_stamping_owner_defers_the_fixup() {
    use super::defers_turn_one_fixup;
    use std::collections::BTreeSet;
    let set = |relics: &[&'static str]| relics.iter().copied().collect::<BTreeSet<&str>>();
    for relic in [
        "RELIC.FUNERARY_MASK",
        "RELIC.BLESSED_ANTLER",
        "RELIC.JEWELED_MASK",
    ] {
        assert!(defers_turn_one_fixup(&set(&[relic]), false), "{relic}");
        assert!(
            !defers_turn_one_fixup(&set(&[relic]), true),
            "{relic} hopper"
        );
        for owner in ["RELIC.GHOST_SEED", "RELIC.TEA_OF_DISCOURTESY"] {
            assert!(
                !defers_turn_one_fixup(&set(&[relic, owner]), false),
                "{relic} {owner}"
            );
        }
    }
    assert!(!defers_turn_one_fixup(
        &set(&["RELIC.RADIANT_PEARL"]),
        false
    ));
    assert!(!defers_turn_one_fixup(&set(&[]), false));
}

/// `BigMushroom::ModifyHandDraw` (`0x90aa8`) takes two from the turn-1 draw;
/// its `AfterRoomEntered` is a visual scale, so nothing else moves.
#[test]
fn a_big_mushroom_save_deals_three() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.BIG_MUSHROOM")).expect("Big Mushroom opens");
    assert_eq!(hand_size(&opening.document), 3);
    assert_eq!(
        pile_ids_and_uids(&opening.document, "hand")[..],
        pile_ids_and_uids(&control.document, "hand")[..3]
    );
    assert_eq!(opening.document.rng, control.document.rng);
}

/// Any two of the turn-1 `BeforeHandDraw` Draw/Hand card movers refuse by
/// name (`refuse_unordered_hand_draw_card_peers`); Ninja Scroll still refuses
/// by its own name first, from the body table.
#[test]
fn every_pair_of_hand_draw_card_movers_refuses() {
    let movers = [
        "RELIC.FUNERARY_MASK",
        "RELIC.JEWELED_MASK",
        "RELIC.RADIANT_PEARL",
    ];
    for (index, first) in movers.iter().enumerate() {
        for second in &movers[index + 1..] {
            match open(&with_relics(first, second)) {
                Err(OpeningRefusal::RelicCombinationRefused { relics, .. }) => {
                    assert_eq!(relics, [first.to_string(), second.to_string()]);
                }
                other => panic!("{first} + {second}: {:?}", other.err()),
            }
        }
        match open(&with_relics(first, "RELIC.NINJA_SCROLL")) {
            Err(OpeningRefusal::RoomEntryRelicNotModeled { relic }) => {
                assert_eq!(relic, "RELIC.NINJA_SCROLL");
            }
            other => panic!("{first} + Ninja Scroll: {:?}", other.err()),
        }
    }
    let set = |relics: &[&'static str]| relics.iter().copied().collect();
    assert!(
        super::refuse_unordered_hand_draw_card_peers(&set(&[
            "RELIC.JEWELED_MASK",
            "RELIC.NINJA_SCROLL"
        ]))
        .is_err()
    );
    assert!(
        super::refuse_unordered_hand_draw_card_peers(&set(&[
            "RELIC.JEWELED_MASK",
            "RELIC.PENDULUM",
            "RELIC.POLLINOUS_CORE"
        ]))
        .is_ok()
    );
}

/// The uid check's pre-deal Hand-head arm refuses a Luminesce that is not the
/// Hand's head, or that `next_card_uid` does not account for.
#[test]
fn the_uid_check_holds_radiant_pearls_luminesce_to_the_hand_head() {
    use super::{PreDeal, check_card_identity_allocation, expected_identity_order};
    let case = with_relic("RELIC.RADIANT_PEARL");
    let built = pre_hook(&case).expect("the pre-hook opening builds");
    let opened = open(&case).expect("Radiant Pearl opens").document;
    let expected = expected_identity_order(&built.document, &[], None);
    let head = PreDeal {
        hand_head: 1,
        ..PreDeal::default()
    };
    assert_eq!(
        check_card_identity_allocation(&opened, &expected, head),
        Ok(())
    );
    let refuses = |document: &crate::canonical::CanonicalStateV2| {
        matches!(
            check_card_identity_allocation(document, &expected, head),
            Err(OpeningRefusal::CardIdentityAllocationDiverged { .. })
        )
    };
    let mut swapped = opened.clone();
    swapped.piles.get_mut("hand").expect("hand").swap(0, 1);
    assert!(refuses(&swapped), "a Luminesce behind a dealt card refuses");
    let mut undercounted = opened.clone();
    undercounted
        .player
        .insert("next_card_uid".to_string(), Value::from(10));
    assert!(
        refuses(&undercounted),
        "next_card_uid short of the insert refuses"
    );
    assert!(
        matches!(
            check_card_identity_allocation(&opened, &expected, 0),
            Err(OpeningRefusal::CardIdentityAllocationDiverged { .. })
        ),
        "an unannounced Hand head refuses"
    );
}

/// `tea_of_discourtesy_spent`: a spent Tea (`CombatsLeft` 0) is inert and
/// opens with the counter projected; a charged one refuses as the body table
/// did; an absent or out-of-domain counter refuses as not exact.
#[test]
fn tea_of_discourtesy_opens_spent_and_refuses_charged_or_unknown() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let spent = open(&with_relic_counter(
        "RELIC.TEA_OF_DISCOURTESY",
        "CombatsLeft",
        0,
    ))
    .expect("a spent Tea opens");
    assert_eq!(
        spent.document.player.get("tea_discourtesy_combats_left"),
        Some(&Value::from(0))
    );
    assert_eq!(spent.document.piles, control.document.piles);
    assert!(matches!(
        open(&with_relic_counter("RELIC.TEA_OF_DISCOURTESY", "CombatsLeft", 1)),
        Err(OpeningRefusal::RoomEntryRelicNotModeled { relic })
            if relic == "RELIC.TEA_OF_DISCOURTESY"
    ));
    for case in [
        with_relic_counter("RELIC.TEA_OF_DISCOURTESY", "CombatsLeft", 2),
        with_relic("RELIC.TEA_OF_DISCOURTESY"),
    ] {
        assert!(matches!(
            open(&case),
            Err(OpeningRefusal::RelicCounterNotExact {
                relic: "RELIC.TEA_OF_DISCOURTESY",
                ..
            })
        ));
    }
}

/// `Planisphere/<AfterRoomEntered>d__5::MoveNext` (`0x32e318`) heals only on
/// an `unknown` map point: every other point opens exactly as the control.
/// On an `unknown` point whose fight is the act's next scheduled normal
/// encounter (the point's first room) it heals 5, clamped; any other
/// `unknown` fight, or a point the save does not name, refuses by name.
#[test]
fn planisphere_is_inert_off_the_unknown_point_and_heals_its_first_room() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let mut entry = entry_facts(&with_relic("RELIC.PLANISPHERE"));
    let max_hp = entry.max_hp_entering;
    entry.hp_entering = max_hp - 20;
    let hp = |opening: &Opening| opening.document.player.get("hp").and_then(Value::as_i64);
    for point in ["monster", "elite", "boss", "shop", "rest_site"] {
        entry.map_point_type = Some(point.to_string());
        let opening = open_facts(&entry).unwrap_or_else(|refusal| panic!("{point}: {refusal}"));
        assert_eq!(hp(&opening), Some(max_hp - 20), "{point}");
        assert_eq!(opening.document.piles, control.document.piles);
    }
    entry.map_point_type = Some("unknown".to_string());
    entry.next_normal_encounter = Some(entry.encounter_id.clone());
    let healed = open_facts(&entry).expect("a first-room ? fight opens");
    assert_eq!(hp(&healed), Some(max_hp - 15));
    assert_eq!(healed.document.piles, control.document.piles);
    entry.hp_entering = max_hp - 2;
    assert_eq!(
        hp(&open_facts(&entry).expect("opens")),
        Some(max_hp),
        "clamped"
    );
    let refused = Some(OpeningRefusal::RoomEntryHealUnmodeled {
        relic: "RELIC.PLANISPHERE",
    });
    for (point, next) in [
        (Some("unknown"), Some("ENCOUNTER.NIBBITS_WEAK")),
        (Some("unknown"), None),
        (None, None),
    ] {
        entry.map_point_type = point.map(str::to_string);
        entry.next_normal_encounter = next.map(str::to_string);
        assert_eq!(open_facts(&entry).err(), refused, "{point:?} {next:?}");
    }
    // No named point: a boss room is the boss point, so inert; any other
    // room kind could be a `?` point's, so it refuses.
    entry.map_point_type = None;
    entry.next_normal_encounter = None;
    entry.node_type = Some("boss".to_string());
    entry.hp_entering = max_hp - 20;
    assert_eq!(
        hp(&open_facts(&entry).expect("a pointless boss room opens")),
        Some(max_hp - 20)
    );
    entry.node_type = Some("elite".to_string());
    assert_eq!(open_facts(&entry).err(), refused, "a pointless elite room");
    // Unowned, a first-room `?` fight does not heal.
    let mut unowned = entry_facts(&empty_belt_case());
    unowned.hp_entering = max_hp - 20;
    unowned.map_point_type = Some("unknown".to_string());
    unowned.next_normal_encounter = Some(unowned.encounter_id.clone());
    assert_eq!(hp(&open_facts(&unowned).expect("opens")), Some(max_hp - 20));
}

/// `Pantograph/<BeforeCombatStart>d__5::MoveNext` (`0x32d694`) heals 25 in a
/// boss room, clamped to max HP, and nowhere else.
#[test]
fn pantograph_heals_twenty_five_in_a_boss_room_only() {
    let mut entry = entry_facts(&with_relic("RELIC.PANTOGRAPH"));
    let max_hp = entry.max_hp_entering;
    entry.hp_entering = max_hp - 40;
    let hp = |entry: &crate::entry::document::EntryDocument| {
        open_facts(entry)
            .expect("the opening builds")
            .document
            .player
            .get("hp")
            .and_then(Value::as_i64)
    };
    assert_eq!(
        hp(&entry),
        Some(max_hp - 40),
        "a monster room does not heal"
    );
    entry.node_type = Some("boss".to_string());
    assert_eq!(hp(&entry), Some(max_hp - 15));
    entry.hp_entering = max_hp - 10;
    assert_eq!(hp(&entry), Some(max_hp), "clamped to max HP");
    let mut unowned = entry_facts(&empty_belt_case());
    unowned.node_type = Some("boss".to_string());
    unowned.hp_entering = max_hp - 40;
    assert_eq!(hp(&unowned), Some(max_hp - 40), "unowned");
}

/// The opening document's player Strength (0 when the field is absent).
fn opening_strength(opening: &Opening) -> i64 {
    opening
        .document
        .player
        .get("strength")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn opening_hp(opening: &Opening) -> i64 {
    opening
        .document
        .player
        .get("hp")
        .and_then(Value::as_i64)
        .expect("the opening writes HP")
}

/// #3044: `RedSkull/<AfterRoomEntered>d__11::MoveNext` (`0x32f6bc`) runs
/// `ModifyStrengthIfNecessary` on entering the combat room, so an owner at or
/// below half HP opens with a real `+3` Strength, and one above opens with
/// none.
#[test]
fn red_skull_opens_with_three_strength_at_or_below_half_hp() {
    let mut entry = entry_facts(&with_relic("RELIC.RED_SKULL"));
    let max_hp = entry.max_hp_entering;
    entry.hp_entering = max_hp / 2;
    let held = open_facts(&entry).expect("a half-HP Red Skull opens");
    assert_eq!(opening_strength(&held), 3);
    entry.hp_entering = max_hp / 2 + 1;
    let above = open_facts(&entry).expect("an above-half Red Skull opens");
    assert_eq!(opening_strength(&above), 0);
    let mut unowned = entry_facts(&empty_belt_case());
    unowned.hp_entering = max_hp / 2;
    assert_eq!(opening_strength(&open_facts(&unowned).expect("opens")), 0);
}

/// Planisphere's room-entry heal and Red Skull share the acquisition-ordered
/// `AfterRoomEntered` walk, and the heal cannot re-run Red Skull (combat is
/// not yet in progress). A crossing heal is exact with Planisphere first, or
/// with Red Skull first when a turn-one Blood Vial heal re-runs it; it refuses
/// otherwise. A heal that does not cross is order-free.
#[test]
fn planisphere_and_red_skull_open_in_their_vouched_order() {
    let mut case = with_relics("RELIC.RED_SKULL", "RELIC.PLANISPHERE");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.BLOOD_VIAL"}),
    );
    let mut entry = entry_facts(&case);
    entry.map_point_type = Some("unknown".to_string());
    entry.next_normal_encounter = Some(entry.encounter_id.clone());
    let max_hp = entry.max_hp_entering;
    // At half HP the Planisphere heal of 5 crosses the threshold.
    entry.hp_entering = max_hp / 2;
    entry.relic_entry.dispatch_ordered = true;
    let order = |first: &str, second: &str, vial: bool| {
        let mut relics = vec![first.to_string(), second.to_string()];
        if vial {
            relics.push("RELIC.BLOOD_VIAL".to_string());
        }
        relics
    };
    let refused = || {
        Some(OpeningRefusal::RelicCombinationRefused {
            relics: vec![
                "RELIC.PLANISPHERE".to_string(),
                "RELIC.RED_SKULL".to_string(),
            ],
            reason: "Planisphere's heal crosses Red Skull's threshold in an order no turn-one heal resolves",
        })
    };

    // Planisphere first: Red Skull reads the healed HP and does nothing.
    entry.relic_entry.relics_entering = order("RELIC.PLANISPHERE", "RELIC.RED_SKULL", true);
    let healed_first = open_facts(&entry).expect("Planisphere-first opens");
    assert_eq!(opening_strength(&healed_first), 0);
    assert_eq!(opening_hp(&healed_first), max_hp / 2 + 5 + 2);

    // Red Skull first: +3 at half HP, stale above it until the Blood Vial
    // heal re-runs Red Skull and removes it (the MFKRHBXBVZ7V node 21 shape).
    entry.relic_entry.relics_entering = order("RELIC.RED_SKULL", "RELIC.PLANISPHERE", true);
    let skull_first = open_facts(&entry).expect("Red Skull-first with Blood Vial opens");
    assert_eq!(opening_strength(&skull_first), 0);
    assert_eq!(opening_hp(&skull_first), max_hp / 2 + 5 + 2);

    // ... and nothing re-runs it without a turn-one heal.
    let mut no_vial = entry_facts(&with_relics("RELIC.RED_SKULL", "RELIC.PLANISPHERE"));
    no_vial.map_point_type = entry.map_point_type.clone();
    no_vial.next_normal_encounter = entry.next_normal_encounter.clone();
    no_vial.hp_entering = entry.hp_entering;
    no_vial.relic_entry.dispatch_ordered = true;
    no_vial.relic_entry.relics_entering = order("RELIC.RED_SKULL", "RELIC.PLANISPHERE", false);
    assert_eq!(open_facts(&no_vial).err(), refused());

    // An unvouched inventory names no order.
    entry.relic_entry.dispatch_ordered = false;
    assert_eq!(open_facts(&entry).err(), refused());

    // A heal that stays at or below half leaves either order the same.
    entry.hp_entering = max_hp / 2 - 10;
    let low = open_facts(&entry).expect("a non-crossing heal opens unvouched");
    assert_eq!(opening_strength(&low), 3);
}

/// Pantograph's `BeforeCombatStart` heal runs after `IsInProgress` is set
/// (`<StartCombatInternal>d__98` `0x3f71b0` IL_020f before IL_023b), so it
/// re-runs Red Skull: a heal past half HP removes the room-entry Strength.
#[test]
fn pantographs_boss_heal_removes_red_skulls_strength_past_half() {
    let mut entry = entry_facts(&with_relics("RELIC.RED_SKULL", "RELIC.PANTOGRAPH"));
    entry.node_type = Some("boss".to_string());
    let max_hp = entry.max_hp_entering;
    entry.hp_entering = max_hp / 2 - 10;
    let lifted = open_facts(&entry).expect("opens");
    assert_eq!(opening_hp(&lifted), max_hp / 2 + 15);
    assert_eq!(opening_strength(&lifted), 0);
    entry.hp_entering = max_hp / 2 - 30;
    let held = open_facts(&entry).expect("opens");
    assert_eq!(opening_hp(&held), max_hp / 2 - 5);
    assert_eq!(opening_strength(&held), 3);
}

/// Red Skull's room-entry `+3` is a room-entry Strength source like Vajra's:
/// Ruined Helmet doubles it, and a peer of a different amount refuses on the
/// unrecorded order.
#[test]
fn red_skull_is_a_ruined_helmet_room_entry_strength_source() {
    let mut entry = entry_facts(&with_relics("RELIC.RED_SKULL", "RELIC.RUINED_HELMET"));
    let max_hp = entry.max_hp_entering;
    entry.hp_entering = max_hp / 2;
    assert_eq!(opening_strength(&open_facts(&entry).expect("opens")), 6);
    entry.hp_entering = max_hp / 2 + 1;
    assert_eq!(opening_strength(&open_facts(&entry).expect("opens")), 0);

    let mut case = with_relics("RELIC.RED_SKULL", "RELIC.RUINED_HELMET");
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.VAJRA"}),
    );
    let mut vajra = entry_facts(&case);
    vajra.hp_entering = max_hp / 2;
    assert!(matches!(
        open_facts(&vajra),
        Err(OpeningRefusal::RuinedHelmetStrengthOrder { .. })
    ));
    vajra.hp_entering = max_hp / 2 + 1;
    assert_eq!(opening_strength(&open_facts(&vajra).expect("opens")), 2);
}

/// #3336: every character pool is gated by exactly that character's own
/// `2/5/7` epochs, read from the generated `CHARACTER_CARD_POOL_ROWS_V1101`
/// (the per-row `unlock_epoch` of each `<Character>CardPool::FilterThroughEpochs`).
/// For Ironclad that is the save-level `fully_unlocked_card_pool` answer's
/// three epochs, which is what lets
/// `opening::owner_card_pool_epochs_revealed` answer Ironclad with that field.
#[test]
fn owner_card_pool_gating_epochs_are_each_owners_own_three() {
    for (pool, rows) in crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101.iter() {
        let mut gates: Vec<&str> = rows.iter().filter_map(|(_, gate)| *gate).collect();
        gates.sort_unstable();
        gates.dedup();
        let expected: Vec<String> = [2, 5, 7]
            .into_iter()
            .map(|n| format!("{pool}{n}_EPOCH"))
            .collect();
        assert_eq!(gates, expected, "{pool}");
    }
}

/// #3336: `owner_card_pool_epochs_revealed` answers each owner from that
/// owner's own epochs and nobody else's.
#[test]
fn owner_card_pool_epochs_revealed_reads_only_the_owners_own_epochs() {
    use crate::entry::opening::owner_card_pool_epochs_revealed;
    for character in [
        "CHARACTER.IRONCLAD",
        "CHARACTER.SILENT",
        "CHARACTER.REGENT",
        "CHARACTER.NECROBINDER",
        "CHARACTER.DEFECT",
    ] {
        let name = character.trim_start_matches("CHARACTER.");
        let own = format!("{name}7_EPOCH");
        let other = if name == "IRONCLAD" {
            "SILENT7_EPOCH"
        } else {
            "IRONCLAD7_EPOCH"
        };
        let facts = |hidden: &[&str]| {
            entry_facts(&with_hidden_epochs(owner_deck_with(character, &[]), hidden))
        };
        assert!(owner_card_pool_epochs_revealed(
            &facts(&[]),
            Some(character)
        ));
        assert!(
            owner_card_pool_epochs_revealed(&facts(&[other, "COLORLESS5_EPOCH"]), Some(character)),
            "{character}: another pool's epoch cannot move this pool"
        );
        assert!(
            !owner_card_pool_epochs_revealed(&facts(&[own.as_str()]), Some(character)),
            "{character}: its own gating epoch hidden"
        );
        let mut unrecorded = owner_deck_with(character, &[]);
        unrecorded["save"]["players"][0]["unlock_state"]["unlocked_epochs"] = serde_json::json!([]);
        assert!(!owner_card_pool_epochs_revealed(
            &entry_facts(&unrecorded),
            Some(character)
        ));
    }
    assert!(!owner_card_pool_epochs_revealed(
        &entry_facts(&with_hidden_epochs(case(), &[])),
        None
    ));
}

/// #3336: the owner-pool and Colorless generators gate on a RECORDED profile,
/// not on the Ironclad 2/5/7 epochs the oracle tested for every owner.
///
/// Each of Discovery (`0x399254` IL_003e-IL_0063), Jackpot (`0x3a80d4`
/// IL_00e6-IL_010b), Calamity (`CalamityPower` `0x3368bc` IL_005a-IL_0089) and
/// Stoke (`0x3bef44` IL_0194-IL_01b9) reads `Owner.Character.CardPool` under
/// `Owner.UnlockState`; Jack of All Trades (`0x3a7ecc` IL_0026-IL_0046) reads
/// `CardPool<ColorlessCardPool>` under it. So for every non-Ironclad owner:
///
/// * a profile hiding `IRONCLAD7_EPOCH` and `COLORLESS5_EPOCH` now OPENS (it
///   refused `generation_pool_partial_unlock` before) and is ADMITTED, the
///   owner pool at its fully-unlocked projection (no own epoch hidden) and
///   the Colorless pool without its COLORLESS5 rows;
/// * Stoke, which the oracle's `ironclad_only` refused for any other owner
///   (`generation_pool_unmodeled`), opens and admits for each;
/// * an unrecorded profile still refuses each by name.
#[test]
fn owner_pool_generators_gate_on_a_recorded_profile_not_the_ironclad_epochs() {
    use crate::catalog::RewardPool;
    for (character, owner) in [
        ("CHARACTER.SILENT", RewardPool::Silent),
        ("CHARACTER.DEFECT", RewardPool::Defect),
        ("CHARACTER.NECROBINDER", RewardPool::Necrobinder),
        ("CHARACTER.REGENT", RewardPool::Regent),
    ] {
        for card in [
            "JACKPOT",
            "DISCOVERY",
            "CALAMITY",
            "JACK_OF_ALL_TRADES",
            "STOKE",
        ] {
            let case = with_hidden_epochs(
                owner_deck_with(character, &[card]),
                &["IRONCLAD7_EPOCH", "COLORLESS5_EPOCH"],
            );
            let opening =
                open(&case).unwrap_or_else(|refusal| panic!("{character} {card}: {refusal}"));
            let catalog =
                crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
            let state =
                crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
            let admitted = crate::engine::admit(&opening.document, &state, &catalog)
                .err()
                .map(|refusal| refusal.to_string());
            // The admission verdict is the fully revealed control's: the
            // Ironclad epoch changes nothing past the opening. On this Ironclad
            // fixture deck the Regent Jackpot / Discovery / Stoke roots refuse
            // at admission for reasons no profile moves (the Quasar closure,
            // the star/orb `AfterEnergyReset` order), exactly as their fully
            // revealed controls did before #3336; every other root admits.
            let control = open(&with_hidden_epochs(
                owner_deck_with(character, &[card]),
                &[],
            ))
            .unwrap_or_else(|refusal| panic!("{character} {card} control: {refusal}"));
            let control_catalog =
                crate::boundary::HotBoundary::catalog_from_canonical(&control.document).unwrap();
            let control_state =
                crate::boundary::HotBoundary::from_canonical(&control.document, &control_catalog)
                    .unwrap();
            let control_admitted =
                crate::engine::admit(&control.document, &control_state, &control_catalog)
                    .err()
                    .map(|refusal| refusal.to_string());
            assert_eq!(admitted, control_admitted, "{character} {card}");
            if owner != RewardPool::Regent || matches!(card, "CALAMITY" | "JACK_OF_ALL_TRADES") {
                assert_eq!(admitted, None, "{character} {card} admits");
            }
            assert_eq!(
                catalog.owner_generation_pool(owner),
                crate::steps::neutral::owner_generation_pool(owner, None),
                "{character} {card}: an Ironclad epoch cannot move the owner pool"
            );
            assert!(
                catalog.colorless_generation_pool().len()
                    < crate::steps::neutral::colorless_generation_pool(None).len(),
                "{character} {card}: the hidden Colorless epoch drops its rows"
            );

            let mut unrecorded = owner_deck_with(character, &[card]);
            unrecorded["save"]["players"][0]["unlock_state"]["unlocked_epochs"] =
                serde_json::json!([]);
            match open(&unrecorded) {
                Err(OpeningRefusal::GenerationPoolPartialUnlock { source, .. }) => {
                    assert_eq!(source, card, "{character}");
                }
                other => panic!("{character} {card}: {other:?}"),
            }
        }
    }
}

/// #3336: Infernal Blade is the one owner-pool source whose engine pool is
/// frozen (`INFERNAL_BLADE_ATTACK_POOL_V109`, the fully-unlocked Ironclad
/// read of `InfernalBlade/<OnPlay>d__3::MoveNext` `0x3a7164` IL_0027-IL_0051),
/// so it alone still needs the Ironclad pool fully revealed, and the refusal
/// names it alone beside a source the recorded profile satisfies. A profile
/// hiding only another pool's epoch opens, writes `entropy_card_pool`
/// `ironclad`, and admits.
#[test]
fn infernal_blade_alone_still_needs_the_ironclad_pool_revealed() {
    let hidden = with_hidden_epochs(
        owner_deck_with("CHARACTER.IRONCLAD", &["INFERNAL_BLADE", "DISCOVERY"]),
        &["IRONCLAD7_EPOCH"],
    );
    match open(&hidden) {
        Err(OpeningRefusal::GenerationPoolPartialUnlock { source, .. }) => {
            assert_eq!(source, "INFERNAL_BLADE");
        }
        other => panic!("{other:?}"),
    }
    open(&with_hidden_epochs(
        owner_deck_with("CHARACTER.IRONCLAD", &["DISCOVERY"]),
        &["IRONCLAD7_EPOCH"],
    ))
    .expect("Discovery alone opens on the recorded profile");

    let other = with_hidden_epochs(
        owner_deck_with("CHARACTER.IRONCLAD", &["INFERNAL_BLADE", "DISCOVERY"]),
        &["DEFECT7_EPOCH", "COLORLESS5_EPOCH"],
    );
    let opening = open(&other).expect("another pool's epoch cannot refuse Infernal Blade");
    assert_eq!(
        opening.document.player.get("entropy_card_pool"),
        Some(&Value::from("ironclad"))
    );
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
    crate::engine::admit(&opening.document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("{refusal}"));
}

/// #3336: `entropy_card_pool` is written when the OWNER's own pool is fully
/// revealed. The oracle wrote it under the Ironclad 2/5/7 epochs for a Regent
/// too; `RegentCardPool::FilterThroughEpochs` (`0xf28e0`) never reads them.
///
/// * Regent, `IRONCLAD7_EPOCH` hidden: written (the oracle omitted it).
/// * Regent, `REGENT7_EPOCH` hidden: omitted (the oracle wrote it).
/// * Ironclad: unchanged, written exactly when the Ironclad epochs are
///   revealed, whatever another pool's epoch.
/// * Silent: never written.
#[test]
fn entropy_card_pool_follows_the_owners_own_pool_epochs() {
    let entropy = |character: &str, hidden: &[&str]| {
        open(&with_hidden_epochs(owner_deck_with(character, &[]), hidden))
            .unwrap_or_else(|refusal| panic!("{character} {hidden:?}: {refusal}"))
            .document
            .player
            .get("entropy_card_pool")
            .cloned()
    };
    let regent = Some(Value::from("regent"));
    let ironclad = Some(Value::from("ironclad"));
    assert_eq!(entropy("CHARACTER.REGENT", &["IRONCLAD7_EPOCH"]), regent);
    assert_eq!(entropy("CHARACTER.REGENT", &["REGENT7_EPOCH"]), None);
    assert_eq!(entropy("CHARACTER.REGENT", &[]), regent);
    assert_eq!(entropy("CHARACTER.IRONCLAD", &["IRONCLAD7_EPOCH"]), None);
    assert_eq!(entropy("CHARACTER.IRONCLAD", &["REGENT7_EPOCH"]), ironclad);
    assert_eq!(entropy("CHARACTER.SILENT", &[]), None);
}

fn with_whispering_earring(mut case: Value) -> Value {
    relic(
        &mut case,
        serde_json::json!({"floor_added_to_deck": 1, "id": "RELIC.WHISPERING_EARRING"}),
    );
    case
}

fn with_deck(rows: &[Value]) -> Value {
    let mut case = empty_belt_case();
    case["save"]["players"][0]["deck"] = Value::from(rows.to_vec());
    case
}

fn root_state(
    document: &crate::canonical::CanonicalStateV2,
) -> (crate::hot::HotState, crate::catalog::Catalog) {
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(document).unwrap();
    let state = crate::boundary::HotBoundary::from_canonical(document, &catalog).unwrap();
    (state, catalog)
}

/// Whispering Earring AutoPlays the dealt hand in turn one's AutoPre phase,
/// and never again (#3414).
///
/// `WhisperingEarring/<AfterAutoPrePlayPhaseEnteredLate>d__8::MoveNext` RVA
/// `0x333fcc` leaves at IL_005a unless `TurnNumber <= 1` (IL_0047-IL_0058).
/// The opened root has played cards (the control opens with none), still
/// admits (the admission wall guards only a loop still ahead,
/// `engine::turn::whispering_earring_loop_is_ahead`), and the recorded
/// pre-AutoPre checkpoint (#3392) holds the hand unplayed. Ending turn one
/// starts turn two with nothing auto-played.
#[test]
fn whispering_earring_auto_plays_the_hand_on_turn_one_only() {
    let played = |document: &crate::canonical::CanonicalStateV2| {
        ["discard", "exhaust"]
            .into_iter()
            .filter_map(|pile| document.piles.get(pile))
            .map(Vec::len)
            .sum::<usize>()
    };
    let control = open(&empty_belt_case()).expect("the control opens");
    assert_eq!(played(&control.document), 0);

    let case = with_whispering_earring(empty_belt_case());
    let recording = OpeningOptions {
        record_native_checkpoints: true,
        ..OpeningOptions::default()
    };
    let opening = build_opening(&entry_facts(&case), &recording)
        .unwrap_or_else(|refusal| panic!("opens: {refusal}"));
    let document = &opening.document;
    assert!(played(document) > 0, "turn one's hand was AutoPlayed");
    let (state, catalog) = root_state(document);
    assert!(!crate::engine::turn::whispering_earring_loop_is_ahead(
        &state
    ));
    crate::engine::admit(document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the post-AutoPre root admits: {refusal}"));

    let checkpoints = opening.native_checkpoints.expect("recorded");
    let before = checkpoints[0].state.as_ref().expect("projects");
    assert_eq!(played(before), 0, "the checkpoint precedes the loop");
    assert_eq!(
        before.piles["hand"].len(),
        control.document.piles["hand"].len()
    );

    let next = crate::engine::apply_action(&state, &catalog, &crate::engine::Action::EndTurn)
        .unwrap()
        .state;
    assert_eq!(next.turn, 2);
    assert!(next.pending.is_none());
    assert_eq!(next.history.manual_card_plays_finished_this_turn, 0);
    assert!(
        !next.piles.get(crate::hot::PileId::Hand).is_empty(),
        "turn two's hand is the player's"
    );
}

/// The Earring's children are still bounded by name (#3414): an X-cost card
/// has no modeled Earring energy, so a hand of Whirlwinds refuses.
#[test]
fn whispering_earring_refuses_an_x_cost_child_by_name() {
    let case = with_whispering_earring(with_deck(&vec![deck_card("WHIRLWIND"); 5]));
    let refusal = open(&case).unwrap_err().to_string();
    assert!(
        refusal.contains("Whispering Earring playable child"),
        "{refusal}"
    );
}

/// A selecting child resolves inline under the Earring's pushed
/// `VakuuCardSelector` (#3414, `engine::selection::VakuuSelectorScope`):
/// Glimmer's Hand put-back and Cosmic Indifference's Discard pick. Each deck
/// opens with the selecting card played and nothing pending, where it would
/// otherwise have refused as a suspending AutoPlay child.
#[test]
fn whispering_earring_resolves_glimmer_and_cosmic_indifference_selections() {
    for selecting in ["GLIMMER", "COSMIC_INDIFFERENCE"] {
        let mut deck = vec![deck_card("STRIKE_IRONCLAD"); 8];
        deck.insert(0, deck_card(selecting));
        let case = with_whispering_earring(with_deck(&deck));
        let opening = open(&case).unwrap_or_else(|refusal| panic!("{selecting}: {refusal}"));
        let document = &opening.document;
        let (state, _) = root_state(document);
        assert!(state.pending.is_none(), "{selecting}");
        assert!(
            document.piles["discard"]
                .iter()
                .any(|card| card.id == selecting),
            "{selecting} was AutoPlayed"
        );
    }
}

/// The admission wall over the Earring's children guards only a loop still
/// ahead (#3414). The same root with a reachable X-cost card admits past
/// turn one's AutoPre, and refuses by name when set back into it.
#[test]
fn whispering_earring_admission_wall_guards_only_a_loop_still_ahead() {
    let opening = open(&with_whispering_earring(empty_belt_case())).expect("opens");
    let mut document = opening.document;
    let next = document.player["next_card_uid"].as_u64().unwrap();
    document
        .piles
        .get_mut("draw")
        .unwrap()
        .push(crate::canonical::CanonicalCardV2 {
            id: "WHIRLWIND".to_string(),
            upgrade: 0,
            uid: Some(next),
            pick: false,
            enchantment: None,
            local_keywords: Vec::new(),
            transient_keywords: Vec::new(),
            enchantment_state: None,
            sovereign_blade: None,
            physical_state: None,
            extra: Vec::new(),
        });
    document
        .player
        .insert("next_card_uid".to_string(), Value::from(next + 1));
    let (mut state, catalog) = root_state(&document);
    crate::engine::admit(&document, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the loop is behind: {refusal}"));

    state.player_phase = crate::engine::turn::PHASE_AUTO_PRE;
    let refusal = crate::engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal.missing().any(|missing| missing
            == crate::engine::admission::MissingCapability::ArgumentShape(
                "Whispering Earring bounded live Hand loop"
            )),
        "{refusal}"
    );
}

/// The uid-allocation check's Whispering Earring branch (#3414) admits the
/// opened root and still refuses what the loop cannot produce: fresh uids
/// out of allocation order in the hand, and a dealt uid carried twice.
#[test]
fn whispering_earring_allocation_check_refuses_what_the_loop_cannot_produce() {
    use super::{PreDeal, check_card_identity_allocation, expected_identity_order};
    let case = with_whispering_earring(empty_belt_case());
    let built = pre_hook(&case).expect("the pre-hook builds");
    let relics = entry_facts(&case).relic_entry.relics_entering;
    let expected = expected_identity_order(&built.document, &relics, None);
    let earring = PreDeal {
        whispering_earring: true,
        ..PreDeal::default()
    };
    let opened = open(&case).expect("opens").document;
    assert_eq!(
        check_card_identity_allocation(&opened, &expected, earring),
        Ok(())
    );

    let mut unordered = opened.clone();
    let next = unordered.player["next_card_uid"].as_u64().unwrap();
    let template = unordered.piles["draw"][0].clone();
    for uid in [next + 1, next] {
        let mut fresh = template.clone();
        fresh.uid = Some(uid);
        unordered.piles.get_mut("hand").unwrap().push(fresh);
    }
    unordered
        .player
        .insert("next_card_uid".to_string(), Value::from(next + 2));
    assert!(matches!(
        check_card_identity_allocation(&unordered, &expected, earring),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.contains("not in allocation order")
    ));

    let mut twice = opened.clone();
    let dealt = twice.piles["draw"][0].clone();
    twice.piles.get_mut("draw").unwrap().push(dealt);
    assert!(matches!(
        check_card_identity_allocation(&twice, &expected, earring),
        Err(OpeningRefusal::CardIdentityAllocationDiverged { detail })
            if detail.contains("twice")
    ));
}

// ---------------------------------------------------------------------------
// Ring of the Drake (#2758)
// ---------------------------------------------------------------------------
//
// Left `OPENING_WINDOW_RELIC_BODIES` on 2026-10-02. No frozen-oracle digest
// exists for it (the Python simulator was deleted before this retirement) and
// the capture corpus holds no fight that owns it, so these witnesses assert
// the opened document and the engine's next three hand draws directly against
// the IL.

/// The player turn and Hand size the engine reaches on each of the next
/// `turns` player turns, ending the turn from the opened document each time.
fn hand_sizes_after_end_turns(opening: &Opening, turns: usize) -> Vec<(i64, usize)> {
    let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&opening.document)
        .expect("the opened document builds its catalog");
    let mut state = crate::boundary::HotBoundary::from_canonical(&opening.document, &catalog)
        .expect("the opened document decodes");
    let mut dealt = Vec::new();
    for _ in 0..turns {
        state = crate::engine::apply_action(&state, &catalog, &crate::engine::Action::EndTurn)
            .expect("the turn ends and the next hand is dealt")
            .state;
        let document = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog)
            .expect("the next turn projects");
        dealt.push((i64::from(state.turn), hand_size(&document)));
    }
    dealt
}

/// `RingOfTheDrake::ModifyHandDraw` (v0.111.0 RVA `0x9a6d0`) adds
/// `DynamicVars.Cards` (`CardsVar(2)`, `get_CanonicalVars` `0x9a6a6`
/// `IL_0009`-`IL_000a`) unless `TurnNumber > DynamicVars["Turns"]` (`3`,
/// `IL_0012`-`IL_0018`; the compare is `op_GreaterThan` at `IL_0041`). It
/// declares no other hook and no saved property, so the ten-card starter deck
/// deals seven on turns one to three and five on turn four.
///
/// The deal is the control's, two cards longer: same shuffle, same RNG
/// position, and the two extra cards are the next two the control would draw.
#[test]
fn a_ring_of_the_drake_save_draws_two_extra_cards_on_turns_one_to_three() {
    let control = open(&empty_belt_case()).expect("the control opens");
    let opening = open(&with_relic("RELIC.RING_OF_THE_DRAKE")).expect("Ring of the Drake opens");
    assert_eq!(hand_size(&control.document), 5);
    assert_eq!(hand_size(&opening.document), 7);
    assert_eq!(
        pile_ids_and_uids(&opening.document, "hand")[..5],
        pile_ids_and_uids(&control.document, "hand")[..]
    );
    // One prefix deal of the same shuffled order: the ring's two extra cards
    // and its draw pile are, together, the control's draw pile.
    let mut ring_cards = pile_ids_and_uids(&opening.document, "hand")[5..].to_vec();
    ring_cards.extend(pile_ids_and_uids(&opening.document, "draw"));
    ring_cards.sort();
    let mut control_cards = pile_ids_and_uids(&control.document, "draw");
    control_cards.sort();
    assert_eq!(ring_cards, control_cards);
    assert_eq!(opening.document.rng, control.document.rng);
    assert_eq!(
        opening.document.player.get("ring_of_the_drake"),
        Some(&Value::from(true))
    );
    assert_ne!(
        python_view(&opening.document).differential_digest(),
        EMPTY_BELT_DIGEST
    );

    assert_eq!(
        hand_sizes_after_end_turns(&opening, 3),
        [(2, 7), (3, 7), (4, 5)],
        "+2 while TurnNumber <= 3, then the base five"
    );
    assert_eq!(
        hand_sizes_after_end_turns(&control, 3),
        [(2, 5), (3, 5), (4, 5)]
    );
    admits_as_loaded(&opening.document).expect("the Ring of the Drake root admits");
}

/// Ring of the Drake beside the other `ModifyHandDraw` rows the opening
/// admits: each listener returns `draw ± its own CanonicalVar`, so they sum
/// in any order. A fold that made one row exclusive (the #2755 guarded-arm
/// shape) fails every line here.
#[test]
fn ring_of_the_drake_sums_with_every_other_hand_draw_row() {
    // (peer, turn-1 Hand, then turns two to four)
    for (peer, first, later) in [
        // Big Mushroom: -2 on turn one only.
        ("RELIC.BIG_MUSHROOM", 5, [7, 7, 5]),
        // The two bag relics: +2 on turn one only.
        ("RELIC.BAG_OF_PREPARATION", 9, [7, 7, 5]),
        ("RELIC.RING_OF_THE_SNAKE", 9, [7, 7, 5]),
    ] {
        for case in [
            with_relics("RELIC.RING_OF_THE_DRAKE", peer),
            with_relics(peer, "RELIC.RING_OF_THE_DRAKE"),
        ] {
            let opening = open(&case).unwrap_or_else(|err| panic!("{peer} opens: {err}"));
            assert_eq!(hand_size(&opening.document), first, "{peer}");
            let dealt: Vec<usize> = hand_sizes_after_end_turns(&opening, 3)
                .into_iter()
                .map(|(_, hand)| hand)
                .collect();
            assert_eq!(dealt, later, "{peer}");
        }
    }
}

/// Every relic still in the body gate refuses by its own name beside Ring of
/// the Drake, in either save order: retiring the ring opens none of them.
#[test]
fn ring_of_the_drake_beside_a_still_gated_relic_refuses_by_the_peers_name() {
    for peer in super::OPENING_WINDOW_RELIC_BODIES {
        for case in [
            with_relics("RELIC.RING_OF_THE_DRAKE", peer),
            with_relics(peer, "RELIC.RING_OF_THE_DRAKE"),
        ] {
            match open(&case) {
                Err(OpeningRefusal::RoomEntryRelicNotModeled { relic }) => {
                    assert_eq!(relic, peer);
                }
                other => panic!("{peer}: {:?}", other.err()),
            }
        }
    }
    assert!(!super::OPENING_WINDOW_RELIC_BODIES.contains(&"RELIC.RING_OF_THE_DRAKE"));
}

// ---------------------------------------------------------------------------
// Fur Coat (#2526)
// ---------------------------------------------------------------------------
//
// The entry now proves whether the current room is one of Fur Coat's marks
// (`entry::relics::fur_coat_membership`). An unmarked room passes the body
// gate, because both of the relic's combat hooks leave before any write; a
// marked room and an unprovable one still refuse. The fixture save stands on
// the monster point `(3, 1)` of act 0, the only point its saved map holds. No
// capture in the corpus owns the relic, so these witnesses assert the opened
// document against the control's.

fn with_fur_coat(act: i64, set: bool, cols: &[i64], rows: &[i64]) -> Value {
    let mut case = empty_belt_case();
    relic(
        &mut case,
        serde_json::json!({
            "floor_added_to_deck": 1, "id": "RELIC.FUR_COAT",
            "props": {
                "ints": [{"name": "FurCoatActIndex", "value": act}],
                "int_arrays": [{"name": "FurCoatCoordCols", "value": cols},
                               {"name": "FurCoatCoordRows", "value": rows}],
                "bools": [{"name": "FurCoatCoordsSet", "value": set}]}}),
    );
    case
}

fn refuses_fur_coat(case: &Value) {
    match open(case) {
        Err(OpeningRefusal::RoomEntryRelicNotModeled { relic }) => {
            assert_eq!(relic, "RELIC.FUR_COAT");
        }
        other => panic!("expected the Fur Coat refusal, got {:?}", other.err()),
    }
}

/// `FurCoat/<BeforeCombatStart>d__26::MoveNext` (v0.111.0 RVA `0x325360`)
/// leaves at `IL_0047` when `GetMarkedCoords` does not hold the current map
/// point, before its `SetCurrentHp` loop (`IL_00c0`-`IL_00c7`), so an
/// unmarked room's opening is the control's with one more relic in the
/// inventory: same roster HP, same piles, same RNG, and the engine's
/// `fur_coat_active` flag left clear so the mid-fight half stays off too.
#[test]
fn a_fur_coat_save_in_an_unmarked_room_opens_as_the_fight_without_it() {
    let control = open(&empty_belt_case()).expect("the control opens");
    for case in [
        // Its own act, marks kept by `AddMarkedRooms`, none of them here.
        with_fur_coat(0, true, &[], &[]),
        // Another act's marks: no act test in the reader, and none match.
        with_fur_coat(2, true, &[1, 3, 0], &[3, 9, 1]),
        // Never marked, outside the act that would mark it.
        with_fur_coat(-1, false, &[], &[]),
    ] {
        assert_eq!(entry_facts(&case).relic_entry.fur_coat_active, Some(false));
        let opening = open(&case).expect("an unmarked Fur Coat room opens");
        let document = &opening.document;
        assert!(!document.monsters.is_empty());
        assert_eq!(document.monsters, control.document.monsters);
        assert_eq!(document.piles, control.document.piles);
        assert_eq!(document.rng, control.document.rng);
        assert_eq!(document.continuations, control.document.continuations);
        assert_ne!(
            document.player.get("fur_coat_active"),
            Some(&Value::from(true))
        );
        assert_ne!(document.differential_digest(), EMPTY_BELT_DIGEST);
        admits_as_loaded(document).expect("the unmarked Fur Coat root admits");
    }
}

/// A marked room keeps refusing: the opening has no body for the initial
/// roster (#3634). The reader has no act test, so another act's mark on this
/// coordinate is a marked room too.
#[test]
fn a_fur_coat_save_in_a_marked_room_still_refuses() {
    for case in [
        with_fur_coat(0, true, &[3], &[1]),
        with_fur_coat(2, true, &[6, 3], &[11, 1]),
    ] {
        assert_eq!(entry_facts(&case).relic_entry.fur_coat_active, Some(true));
        refuses_fur_coat(&case);
    }
}

/// Unprovable membership is never read as "unmarked".
#[test]
fn a_fur_coat_save_with_unprovable_membership_refuses() {
    let mut partial = with_fur_coat(2, true, &[1], &[3]);
    partial["save"]["players"][0]["relics"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["props"]
        .as_object_mut()
        .unwrap()
        .remove("ints");
    let mut off_map = with_fur_coat(2, true, &[1], &[3]);
    off_map["save"]["visited_map_coords"] =
        serde_json::json!([{"col": 3, "row": 0}, {"col": 5, "row": 1}]);
    for case in [
        // No saved properties at all.
        with_relic("RELIC.FUR_COAT"),
        partial,
        // Its own act with no marks set, or with a mark that is not one of
        // this map's combat points: a rebuilt map re-rolls both.
        with_fur_coat(0, false, &[], &[]),
        with_fur_coat(0, true, &[1], &[3]),
        with_fur_coat(0, true, &[3, 1], &[1, 3]),
        // Unequal coordinate arrays.
        with_fur_coat(2, true, &[1, 3], &[3]),
        // A current coordinate the saved map does not hold.
        off_map,
    ] {
        assert_eq!(entry_facts(&case).relic_entry.fur_coat_active, None);
        refuses_fur_coat(&case);
    }
}
