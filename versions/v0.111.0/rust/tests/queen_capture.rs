//! Complete Queen recording: relic choices, Bound AutoPlay and Inferno/Horn deaths.
use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;

#[test]
fn queen_capture_matches_every_completed_native_action() {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../../eval/search/queen-capture-v1/entry.canonical.json"
    ))
    .unwrap();
    let steps: Vec<Value> = serde_json::from_str(include_str!(
        "../../eval/search/queen-capture-v1/recorded-line.json"
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
        // This historical digest predates the explicit native hook-active
        // carrier. Validate its new value separately, then retain the exact
        // old digest over every pre-existing field (including native data).
        // The completed capture's only actual player death is its last step.
        let mut legacy_shape = projected.clone();
        assert_eq!(
            legacy_shape.player.remove("player_hooks_deactivated"),
            if i + 1 == steps.len() {
                Some(json!(true))
            } else {
                None
            },
            "step {i} native hook deactivation"
        );
        assert_eq!(
            legacy_shape.differential_digest(),
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
    assert_eq!(checked, 28);
    assert_eq!((state.turn, state.hp), (5, 0));
    assert!(state.history.over);
}

#[test]
fn queen_horn_hellraiser_autoplay_resumes_inside_amalgam_death() {
    let regression: Value = serde_json::from_str(include_str!(
        "../../eval/search/queen-capture-v1/horn-hellraiser-regression.json"
    ))
    .unwrap();
    let entry: CanonicalStateV2 = serde_json::from_value(regression["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    let action: Action = serde_json::from_value::<ExactSolveActionV1>(regression["action"].clone())
        .unwrap()
        .try_into()
        .unwrap();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap();
    assert!(next.monsters[0].hp <= 0);
    assert!(next.monsters[1].hp < state.monsters[1].hp);
    let doc = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let reloaded = HotBoundary::from_canonical(&doc, &reloaded_catalog).unwrap();
    if !next.history.over {
        engine::admit(&doc, &reloaded, &reloaded_catalog).unwrap();
    }
}
