//! Captured native checkpoints, including a second Lay after egg deaths.
use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;

#[test]
fn partial_ovicopter_capture_matches_every_retained_native_checkpoint() {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../../eval/search/ovicopter-partial-v1/entry.canonical.json"
    ))
    .unwrap();
    let steps: Vec<Value> = serde_json::from_str(include_str!(
        "../../eval/search/ovicopter-partial-v1/recorded-prefix.json"
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
    assert_eq!(checked, 14);
    assert_eq!((state.turn, state.hp, state.block), (6, 46, 16));
    assert!(!state.history.over, "a partial recording is not a loss");
}
