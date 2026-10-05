//! The eval fixture that reached a stale ledger position (#3583), replayed
//! in whatever profile this test is built with.
//!
//! `wasm/tests/api.rs` replays every fixture, but `tools/wasm_build.py` runs
//! it with `--release`, where debug assertions are compiled out. This crate's
//! own `cargo test` is the dev-profile lane, so the one fixture known to
//! reach the branch is held here with its assertions on.
//!
//! Fixture `fc66b15822d20bd5`, step 29 (`play` uid 7 on target 0): monster 0
//! carries `[strength 1, monarchs_gaze_strength_down 1]` and the hit applies
//! one more Monarch's Gaze. Strength lands on exactly zero, its row closes,
//! and the wrapper behind it has to be found at its new position.
//! `engine::damage::write_monster_temp_strength_wrapper` carries the IL.

use std::path::Path;

use serde_json::Value;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;

const FIXTURE: &str = "fc66b15822d20bd5";
const STEP: usize = 29;

fn read(name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights")
        .join(FIXTURE);
    std::fs::read(dir.join(name)).unwrap()
}

#[test]
fn the_fixture_that_zeroes_strength_under_a_wrapper_replays_with_its_digests() {
    let entry: CanonicalStateV2 = serde_json::from_slice(&read("entry.canonical.json")).unwrap();
    let line: Value = serde_json::from_slice(&read("human_line.json")).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();

    let attachments = |state: &sts_sim::hot::HotState| {
        let document = HotBoundary::try_to_canonical(state, &catalog).unwrap();
        let document = serde_json::to_value(&document).unwrap();
        (
            document["monsters"][0]["power_attachments"].clone(),
            document["monsters"][0]["strength"].clone(),
            document,
        )
    };

    let actions = line["actions"].as_array().unwrap();
    let digests = line["step_digests"].as_array().unwrap();
    let mut events = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        if index == STEP {
            // The shape the fix is about, read off the projection so a
            // regenerated fixture that no longer reaches it fails here
            // instead of passing vacuously.
            let (rows, strength, _) = attachments(&state);
            assert_eq!(
                rows,
                serde_json::json!([
                    ["strength", "monster", 0, 1, 0],
                    ["monarchs_gaze_strength_down", "player", 0, 1, 0]
                ]),
            );
            assert_eq!(strength, serde_json::json!(1));
        }
        let wire: ExactSolveActionV1 = serde_json::from_value(action.clone()).unwrap();
        let action = Action::try_from(wire).unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|error| panic!("step {index}: {error}"));
        let digest = HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .differential_digest();
        assert_eq!(Value::String(digest), digests[index], "step {index}");
    }
    assert!(actions.len() > STEP);
}
