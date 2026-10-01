//! #2949: an engine refusal ends one search playout, never the search.
//!
//! The fixture is eval fight `f98c2d87a6b5e4eb` (seed LYE3ZK9FYKKV node 37,
//! Axebots, A10 Ironclad), where both explored searches still prune a
//! `ContinuationNotModeled` refusal. Both methods must still return a
//! replayed winning line and report the refusal they pruned. (Earlier
//! witnesses: KD13JGCDPB3U fight 3 stopped refusing once #2950 modeled
//! Fiend Fire, fight 0, `search_refused_branch_toadpoles.json`, once
//! #2655 modeled the retained dead Thorns receiver (`thorns_dead_receiver.rs`
//! keeps that fixture as the positive witness), `ffc19a45c017c3bf`
//! (Corpse Slugs) once #3197 let Throwing Axe and Echo Form replay a Power
//! body, and `f1e17a89e32a62dc` (Terror Eel) once #3197 let Reboot shuffle
//! over duplicate `(id, upgrade)` keys.)
use serde_json::Value;
use std::process::Command;

fn search(method: &str, seed: &str) -> Value {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_axebots.json"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_sts-sim"))
        .args([
            "search", entry, method, seed, "600", "1.414", "0.05", "300", "20",
        ])
        .output()
        .expect("run sts-sim search");
    assert!(
        output.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("search JSON")
}

#[test]
fn refused_branches_are_pruned_not_fatal() {
    for (method, seed) in [("uct", "7"), ("random", "16")] {
        let result = search(method, seed);
        let refusals = result["refusals"].as_u64().unwrap();
        assert!(
            refusals > 0,
            "{method}/{seed} no longer reaches a refusal; pick a new witness entry (#2949)"
        );
        assert_eq!(result["playouts"], 300);
        assert!(result["refused_playouts"].as_u64().unwrap() >= 1);
        let first = &result["first_refusal"];
        assert_eq!(first["status"], "refused");
        assert!(first["detail"].as_str().is_some_and(|d| !d.is_empty()));
        assert!(!first["actions"].as_array().unwrap().is_empty());
        // The retained line was replayed from the entry without a refusal.
        assert_eq!(result["best"]["won"], true, "{method}/{seed}");
    }
}
