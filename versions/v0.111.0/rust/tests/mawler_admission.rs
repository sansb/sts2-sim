//! #2453: retained capture admission is proved by actual replay, not a cap alone.
use serde_json::{Value, json};
use std::collections::BTreeSet;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::{self, Action};
use sts_sim::exact_dfs::ExactDfsStatus;
use sts_sim::exact_solve_v1::{
    ExactSolveActionV1, ExactSolveRefusalCodeV1, PROTOCOL_V1, solve_value,
};

fn row() -> Value {
    serde_json::from_str::<Value>(include_str!("../fixtures/exact_solve_corpus_v1.json"))
        .unwrap()["rows"].as_array().unwrap().iter()
        .find(|row| row["id"] == "capture-6p96t755cnz3-mawler-deadline").unwrap().clone()
}

fn assert_capture_has_no_epoch_filtered_generation(
    document: &CanonicalStateV2,
    catalog: &sts_sim::catalog::Catalog,
) {
    // Complete immutable closure, not just the visible hand. The only minted
    // descendant is fixed Slimed from the slime bodies. None of these exact
    // registry identities, the two relics, or Powdered Demise selects a pool.
    let closure: BTreeSet<_> = catalog
        .specs()
        .map(|spec| spec.identity.id.as_str())
        .collect();
    assert!(
        closure.is_subset(&BTreeSet::from([
            "ASCENDERS_BANE",
            "BASH",
            "BREAKTHROUGH",
            "DEFEND_IRONCLAD",
            "STRIKE_IRONCLAD",
            "THUNDERCLAP",
            "TWIN_STRIKE",
            "SLIMED",
        ])),
        "unexpected capture closure: {closure:?}"
    );
    assert_eq!(
        document.player["relics_entering"],
        json!(["RELIC.BURNING_BLOOD", "RELIC.GOLDEN_PEARL"])
    );
    assert!(
        document
            .player
            .get("potions")
            .is_none_or(|belt| *belt == json!(["POWDERED_DEMISE"]))
    );
}

#[test]
fn retained_mawler_replays_the_recorded_line_with_lossless_entry_and_terminal_digests() {
    let row = row();
    let document: CanonicalStateV2 = serde_json::from_value(row["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    assert_capture_has_no_epoch_filtered_generation(&document, &catalog);
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    assert!(
        !state.fully_unlocked_card_pool_epochs,
        "capture lacks DEFECT4..7"
    );
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(
        !engine::legal_actions(&state, &catalog)
            .iter()
            .any(|action| matches!(action, Action::Play { uid: 0, .. })),
        "native Bane is unplayable"
    );
    let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert_eq!(projected.differential_digest(), row["entry_digest"]);
    assert_eq!(
        projected.player["splash_unlock_epochs"],
        document.player["splash_unlock_epochs"]
    );
    assert_eq!(
        projected.player["splash_unlock_epochs"]
            .as_array()
            .unwrap()
            .len(),
        53
    );
    let mut events = Vec::new();
    let mut ended = false;
    for wire in row["python"]["action_line"].as_array().unwrap() {
        let wire: ExactSolveActionV1 = serde_json::from_value(wire.clone()).unwrap();
        let action: Action = wire.try_into().unwrap();
        let first_end = !ended && matches!(action, Action::EndTurn);
        state = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
        if first_end {
            let after = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert!(
                after.piles["exhaust"]
                    .iter()
                    .any(|card| card.uid == Some(0))
            );
            ended = true;
        }
    }
    assert!(ended);
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .differential_digest(),
        row["python"]["final_state_digest"]
    );
    assert_eq!(state.hp, 24);
}

#[test]
fn retained_provenance_admits_any_normalized_profile_and_refuses_malformed_ones() {
    let mut request = json!({"protocol": PROTOCOL_V1, "entry": row()["entry"],
        "max_turns": 5, "deadline_ms": 0});
    let response = solve_value(request.clone());
    assert_eq!(
        response.status,
        ExactDfsStatus::Deadline,
        "{:?}",
        response.refusal
    );
    // #2453 refused a Splash here because the capture profile was partial.
    // #2469 derives the pool from that profile, so the same entry is now
    // admitted and its owner-excluded pool is strictly smaller than the
    // fully-unlocked one (the capture lacks DEFECT4..7).
    request["entry"]["piles"]["hand"][0]["id"] = json!("SPLASH");
    let response = solve_value(request.clone());
    assert_eq!(
        response.status,
        ExactDfsStatus::Deadline,
        "{:?}",
        response.refusal
    );
    let document: CanonicalStateV2 = serde_json::from_value(request["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(
        !state.fully_unlocked_card_pool_epochs,
        "a derivable profile must never be promoted to full unlocks"
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .player["splash_unlock_epochs"],
        document.player["splash_unlock_epochs"]
    );
    // Malformed tuples still refuse at the canonical boundary, mirroring
    // Python's I5 checks in `_derive_splash_attack_pool` L16164.
    for mutation in [
        "duplicate",
        "unknown",
        "reordered",
        "prefixed",
        "empty_name",
    ] {
        let mut entry = row()["entry"].clone();
        let epochs = entry["player"]["splash_unlock_epochs"]
            .as_array_mut()
            .unwrap();
        match mutation {
            "duplicate" => epochs.push(epochs[0].clone()),
            "unknown" => epochs[0] = json!("UNKNOWN_EPOCH"),
            "reordered" => epochs.swap(0, 1),
            "prefixed" => epochs[0] = json!(format!("EPOCH.{}", epochs[0].as_str().unwrap())),
            "empty_name" => epochs[0] = json!(""),
            _ => unreachable!(),
        }
        let response = solve_value(json!({"protocol": PROTOCOL_V1, "entry": entry,
            "max_turns": 5, "deadline_ms": 0}));
        assert_eq!(
            response.refusal.unwrap().code,
            ExactSolveRefusalCodeV1::CanonicalBoundaryRefused,
            "{mutation}"
        );
    }
    // A shorter but still normalized profile is a different capture, not a
    // malformed one: it must load and replay rather than refuse (#2469).
    let mut entry = row()["entry"].clone();
    entry["player"]["splash_unlock_epochs"]
        .as_array_mut()
        .unwrap()
        .pop();
    let document: CanonicalStateV2 = serde_json::from_value(entry.clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    assert!(!state.fully_unlocked_card_pool_epochs);
    engine::admit(&document, &state, &catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .player["splash_unlock_epochs"],
        document.player["splash_unlock_epochs"],
        "the shortened tuple round-trips verbatim"
    );
    assert_eq!(
        solve_value(json!({"protocol": PROTOCOL_V1, "entry": entry,
            "max_turns": 5, "deadline_ms": 0}))
        .status,
        ExactDfsStatus::Deadline
    );
}

#[test]
fn direct_review_and_all_four_slime_bodies_replay_against_python_digests() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/mawler_admission_v1.json")).unwrap();
    for row in fixture["rows"].as_array().unwrap() {
        let document: CanonicalStateV2 = serde_json::from_value(row["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        if row["id"] == "direct-6p96-fight-0" {
            assert_capture_has_no_epoch_filtered_generation(&document, &catalog);
        }
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        engine::admit(&document, &state, &catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&state, &catalog)
                .unwrap()
                .differential_digest(),
            row["entry_digest"]
        );
        let response = solve_value(json!({"protocol": PROTOCOL_V1, "entry": row["entry"],
            "max_turns": 5, "deadline_ms": 0}));
        assert_eq!(
            response.status,
            ExactDfsStatus::Deadline,
            "{}: {:?}",
            row["id"],
            response.refusal
        );
        let mut events = Vec::new();
        for (turn, digest) in row["end_turn_digests"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            state =
                engine::apply_action_into(&state, &catalog, &Action::EndTurn, &mut events).unwrap();
            assert_eq!(
                HotBoundary::try_to_canonical(&state, &catalog)
                    .unwrap()
                    .differential_digest(),
                digest.as_str().unwrap(),
                "{} turn {}",
                row["id"],
                turn + 1
            );
        }
    }
}

#[test]
fn recorded_mawler_seed_is_replayed_and_retained_at_the_adapter_deadline() {
    let row = row();
    let response = solve_value(json!({"protocol": PROTOCOL_V1, "entry": row["entry"],
        "max_turns": 5, "deadline_ms": 0, "seed": {
            "actions": row["python"]["action_line"], "objective": row["python"]["objective"],
            "final_digest": row["python"]["final_state_digest"]}}));
    assert_eq!(
        response.status,
        ExactDfsStatus::Deadline,
        "{:?}",
        response.refusal
    );
    let solution = response
        .solution
        .expect("the recorded line is an achieved incumbent");
    assert_eq!(solution.final_digest, row["python"]["final_state_digest"]);
    assert_eq!(solution.objective[1], 24);
}

#[test]
fn cold_capture_provenance_cannot_be_dropped_or_promoted_to_full_unlocks() {
    let mut document: CanonicalStateV2 = serde_json::from_value(row()["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    state.fully_unlocked_card_pool_epochs = true;
    assert!(HotBoundary::try_to_canonical(&state, &catalog).is_err());
    document.player.remove("splash_unlock_epochs");
    assert!(HotBoundary::from_canonical(&document, &catalog).is_err());
    let no_profile_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let original = serde_json::from_value(row()["entry"].clone()).unwrap();
    assert!(HotBoundary::from_canonical(&original, &no_profile_catalog).is_err());
}
