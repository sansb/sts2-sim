//! The R0.5 slice's parity evidence: replay one scripted line in Rust and
//! compare it, step by step, against pins generated from the Python oracle.
//!
//! `tools/gen_slice_pins.py` plays the same line through `combat_sim` and
//! writes `fixtures/slice_line_v1.json`: per step, the ordered legal-action
//! list and the sha256 of the successor state's canonical projection. This
//! test loads the R0.3 fixture through the R0.4 boundary, admits it, and
//! replays the line, asserting after every action that
//!
//! * the Rust legal-action list matches Python's **exactly and in order** —
//!   enumeration order is part of the differential contract (PORT_PLAN §4),
//!   not an implementation detail; and
//! * the canonical digest of the Rust successor equals Python's.
//!
//! A digest match is a whole-document match: `differential_digest` hashes the
//! same sorted, compact bytes `project_state.py` produces, so agreeing on it
//! means agreeing on every projected `State` and `Monster` field, every pile's
//! order, every card uid, and every RNG stream's words and counter.
//!
//! The line is 15 actions long, crosses two reshuffle boundaries (the draw
//! pile empties at the ends of turns 2 and 4), and ends at a terminal state
//! with both TOADPOLEs dead — so it exercises the whole slice: energy, the
//! attack and block steps, Vulnerable from BASH, monster attacks against
//! block, Thorns retaliation, monster death, and the win transition.

use serde_json::Value;
use std::collections::BTreeSet;

use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{
    Action, EngineRefusal, SelectionRef, admit, apply_action_into, legal_actions,
};
use sts_sim::hot::HotState;

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");
const PINS: &str = include_str!("../fixtures/slice_line_v1.json");

/// Bumped, never reinterpreted — see `tools/gen_slice_pins.py` (in git
/// history before #2999, which deleted it; the pins are frozen data).
const PIN_SCHEMA: &str = "sts-sim-slice-line-v1";

fn load() -> (HotState, Catalog) {
    let document: CanonicalStateV2 =
        serde_json::from_str(FIXTURE).expect("the fixture parses as canonical v2");
    let catalog =
        HotBoundary::catalog_from_canonical(&document).expect("the fixture builds a catalog");
    let state =
        HotBoundary::from_canonical(&document, &catalog).expect("the fixture builds a hot state");
    admit(&document, &state, &catalog).expect("the fixture is inside the slice");
    (state, catalog)
}

fn digest(state: &HotState, catalog: &Catalog) -> String {
    HotBoundary::try_to_canonical(state, catalog)
        .unwrap()
        .differential_digest()
}

/// Decode one pinned action from the generator's encoding.
fn decode(value: &Value) -> Action {
    match value["kind"].as_str().expect("every pin names a kind") {
        "end" => Action::EndTurn,
        "play" => Action::Play {
            uid: value["uid"].as_u64().expect("a play pin carries a uid") as u32,
            target: value
                .get("target")
                .and_then(Value::as_u64)
                .map(|index| index as u8),
            selection: SelectionRef::new(
                value
                    .get("selection")
                    .and_then(Value::as_u64)
                    .map(|uid| uid as u32),
            ),
        },
        other => panic!("unknown pinned action kind {other:?}"),
    }
}

fn pins() -> Value {
    let pins: Value = serde_json::from_str(PINS).expect("the pins parse");
    assert_eq!(pins["schema"], PIN_SCHEMA);
    assert_eq!(pins["game_build"], "v0.111.0");
    pins
}

#[test]
fn the_scripted_line_matches_the_python_oracle_step_for_step() {
    let pins = pins();
    let (mut state, catalog) = load();
    assert_eq!(
        digest(&state, &catalog),
        pins["entry_digest"].as_str().unwrap(),
        "the entry state itself must round-trip before any action"
    );

    let steps = pins["steps"].as_array().expect("the pins carry steps");
    assert!(
        steps.len() >= 8,
        "the slice's parity evidence needs at least eight pinned states, got {}",
        steps.len()
    );

    let mut events = Vec::with_capacity(64);
    for (index, step) in steps.iter().enumerate() {
        let expected: Vec<Action> = step["legal"]
            .as_array()
            .expect("every step pins its legal-action list")
            .iter()
            .map(decode)
            .collect();
        assert_eq!(
            legal_actions(&state, &catalog),
            expected,
            "legal actions diverged at step {index}"
        );

        let action = decode(&step["action"]);
        state = apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("step {index} ({action:?}) refused: {refusal}"));
        assert_eq!(
            digest(&state, &catalog),
            step["digest"].as_str().unwrap(),
            "canonical projection diverged at step {index} after {action:?}"
        );
    }

    // The line must end at a terminal state, and a terminal state must behave
    // like one.
    assert!(
        steps.last().unwrap()["over"].as_bool().unwrap(),
        "the pinned line must reach a terminal state"
    );
    assert!(legal_actions(&state, &catalog).is_empty());
    assert_eq!(
        apply_action_into(&state, &catalog, &Action::EndTurn, &mut events).unwrap_err(),
        EngineRefusal::CombatOver
    );
}

#[test]
fn the_pinned_line_crosses_a_reshuffle_boundary() {
    // A reshuffle is the only thing in the slice that consumes RNG, so the
    // `rng` stream's counter advancing is exactly the evidence that draw
    // parity — not just damage arithmetic — is under test. Two boundaries are
    // crossed; the counters are read off the projections the digests pin.
    let pins = pins();
    let (mut state, catalog) = load();
    let mut counters: BTreeSet<u64> = BTreeSet::new();
    let mut events = Vec::new();
    counters.insert(rng_counter(&state, &catalog));
    for step in pins["steps"].as_array().unwrap() {
        state = apply_action_into(&state, &catalog, &decode(&step["action"]), &mut events).unwrap();
        counters.insert(rng_counter(&state, &catalog));
    }
    assert!(
        counters.len() >= 3,
        "expected at least two reshuffles, saw rng counters {counters:?}"
    );
}

fn rng_counter(state: &HotState, catalog: &Catalog) -> u64 {
    HotBoundary::try_to_canonical(state, catalog).unwrap().rng["rng"].counter
}

#[test]
fn every_pinned_action_is_one_the_engine_would_have_offered() {
    // The oracle's policy is "always the first legal action". Replaying it
    // only proves parity if the action it names is in the Rust list at that
    // step — otherwise a matching digest could come from a different move.
    let pins = pins();
    let (mut state, catalog) = load();
    let mut events = Vec::new();
    for (index, step) in pins["steps"].as_array().unwrap().iter().enumerate() {
        let action = decode(&step["action"]);
        let offered = legal_actions(&state, &catalog);
        assert_eq!(
            offered.first(),
            Some(&action),
            "step {index} replayed an action the engine did not enumerate first"
        );
        state = apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    }
}
