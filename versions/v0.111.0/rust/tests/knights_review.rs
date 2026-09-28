//! Captured floor starts from Y3NULJSNND7N, plus search-discovered continuations.
use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;

#[test]
fn stampede_headbutt_finishes_before_next_turn_mittens_choice() {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../../eval/search/knights-review-v1/floor46.canonical.json"
    ))
    .unwrap();
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(include_str!(
        "../../eval/search/knights-review-v1/stampede-headbutt.json"
    ))
    .unwrap();
    let mut catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    for (i, wire) in actions.into_iter().enumerate() {
        let action: Action = wire.try_into().unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new())
            .unwrap_or_else(|e| panic!("step {i}: {e:?}"));
        let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        catalog = HotBoundary::catalog_from_canonical(&projected).unwrap();
        state = HotBoundary::from_canonical(&projected, &catalog).unwrap();
        engine::admit(&projected, &state, &catalog).unwrap();
        if i == 8 {
            assert_eq!(state.turn, 1);
            assert_eq!(state.pending.as_ref().unwrap().frame_uid, 41);
            assert_eq!(state.frames.len(), 4);
        }
    }
    assert_eq!(state.turn, 2);
    assert!(
        state.frames.is_empty(),
        "completed end-turn root is retired"
    );
    let doc =
        serde_json::to_value(HotBoundary::try_to_canonical(&state, &catalog).unwrap()).unwrap();
    assert_eq!(doc["player"]["pending"][0], "toasty_mittens_select");
    assert!(!engine::legal_actions(&state, &catalog).is_empty());
    // A second public action consumes the new independent relic selection.
    let choice = engine::legal_actions(&state, &catalog)[0];
    let next = engine::apply_action_into(&state, &catalog, &choice, &mut Vec::new()).unwrap();
    assert!(next.pending.is_none());
    assert!(next.frames.is_empty());
}

#[test]
fn knights_capture_matches_every_completed_native_action() {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../../eval/search/knights-review-v1/floor42.canonical.json"
    ))
    .unwrap();
    let steps: Vec<Value> = serde_json::from_str(include_str!(
        "../../eval/search/knights-review-v1/floor42-recorded-line.json"
    ))
    .unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        entry
    );
    let mut events = Vec::new();
    let mut checked = 0;
    for (i, step) in steps.iter().enumerate() {
        let wire: ExactSolveActionV1 = serde_json::from_value(step["action"].clone()).unwrap();
        let action: Action = wire.try_into().unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
        let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        // Choices must survive the same persistence boundary used by the site.
        let reloaded_catalog = HotBoundary::catalog_from_canonical(&projected).unwrap();
        let restored = HotBoundary::from_canonical(&projected, &reloaded_catalog).unwrap();
        if !state.history.over {
            engine::admit(&projected, &restored, &reloaded_catalog).unwrap();
        }
        assert_eq!(
            HotBoundary::try_to_canonical(&restored, &reloaded_catalog).unwrap(),
            projected,
            "step {i} boundary roundtrip"
        );
        assert_eq!(
            projected.differential_digest(),
            step["digest_after"],
            "step {i}"
        );
        if let Some(native) = step.get("native") {
            checked += 1;
            assert_eq!(
                json!({"hp":state.hp,"max_hp":state.max_hp,"block":state.block,
                "energy":state.energy,"turn":state.turn}),
                native["player"],
                "step {i}"
            );
            let monsters: Vec<_> = state
                .monsters
                .iter()
                .filter(|m| m.hp > 0)
                .map(|m| json!([m.kind.as_str(), m.hp, m.max_hp, m.block]))
                .collect();
            assert_eq!(json!(monsters), native["monsters"], "step {i}");
            let doc = serde_json::to_value(&projected).unwrap();
            assert_eq!(doc["rng"], native["rng"], "step {i} RNG");
            for (key, expected) in native["relic_counters"].as_object().unwrap() {
                assert_eq!(
                    doc["player"].get(key).cloned().unwrap_or(json!(0)),
                    *expected,
                    "step {i} {key}"
                );
            }
            for (pile, expected) in native["piles"].as_object().unwrap() {
                let cards: Vec<_> = doc["piles"][pile]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| json!([c["id"], c.get("upgrade").cloned().unwrap_or(json!(0))]))
                    .collect();
                assert_eq!(json!(cards), *expected, "step {i} {pile}");
            }
        }
    }
    assert_eq!(checked, 26);
    assert_eq!((state.turn, state.hp), (5, 38));
    assert!(state.history.over);
}

#[test]
fn all_three_captured_fights_have_replayable_winning_witnesses() {
    for (floor, entry, line) in [
        (
            42,
            include_str!("../../eval/search/knights-review-v1/floor42.canonical.json"),
            include_str!("../../eval/search/knights-review-v1/floor42-solver-line.json"),
        ),
        (
            43,
            include_str!("../../eval/search/knights-review-v1/floor43.canonical.json"),
            include_str!("../../eval/search/knights-review-v1/floor43-solver-line.json"),
        ),
        (
            46,
            include_str!("../../eval/search/knights-review-v1/floor46.canonical.json"),
            include_str!("../../eval/search/knights-review-v1/floor46-solver-line.json"),
        ),
    ] {
        let entry: CanonicalStateV2 = serde_json::from_str(entry).unwrap();
        let line: Value = serde_json::from_str(line).unwrap();
        let mut catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
        let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
        engine::admit(&entry, &state, &catalog).unwrap();
        for (i, wire) in line["actions"].as_array().unwrap().iter().enumerate() {
            let wire: ExactSolveActionV1 = serde_json::from_value(wire.clone()).unwrap();
            let action: Action = wire.try_into().unwrap();
            state = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new())
                .unwrap_or_else(|e| panic!("floor {floor}, step {i}: {e:?}"));
            let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            catalog = HotBoundary::catalog_from_canonical(&projected).unwrap();
            state = HotBoundary::from_canonical(&projected, &catalog).unwrap();
            if !state.history.over {
                engine::admit(&projected, &state, &catalog)
                    .unwrap_or_else(|e| panic!("floor {floor}, step {i}: {e:?}"));
            }
        }
        assert!(state.history.over && state.hp > 0);
        assert_eq!(json!(state.hp), line["hp"]);
        assert_eq!(json!(state.turn), line["turn"]);
        assert_eq!(
            HotBoundary::try_to_canonical(&state, &catalog)
                .unwrap()
                .differential_digest(),
            line["digest"]
        );
    }
}
