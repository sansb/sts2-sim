//! Information-set MCTS from a canonical entry (#3436).
//!
//! ```text
//! cargo run --release --example ismcts -- search ENTRY.json hidden|clairvoyant ITERATIONS SEED
//! cargo run --release --example ismcts -- play ENTRY.json ITERATIONS WORLDS SEED
//! ```
//!
//! `search` runs one search from the entry and prints each root action's
//! estimate. `play` is the closed-loop evaluation: it samples `WORLDS` true
//! worlds from the entry's information set (common random numbers: world `i`
//! is the same for every policy) and plays each to the end, re-planning at
//! every decision, once with the hidden-information search and once with the
//! clairvoyant one on the same budget. World 0 is the fixture's true world. The gap between the two is what the
//! hidden information is worth to this search. Every number is an estimate.
//! `ISMCTS_THREADS=N` runs N root-parallel trees per search.

use std::time::Instant;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hidden::{self, DrawKnowledge, PolicyRng};
use sts_sim::hot::HotState;
use sts_sim::ismcts::{self, Information, IsmctsParams, IsmctsResult};

/// Root-parallel trees per search, from `ISMCTS_THREADS` (default 1).
fn threads() -> usize {
    std::env::var("ISMCTS_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn load(path: &str) -> (HotState, Catalog) {
    let document: CanonicalStateV2 =
        serde_json::from_slice(&std::fs::read(path).expect("entry readable"))
            .expect("canonical entry");
    let catalog = HotBoundary::catalog_from_canonical(&document).expect("catalog");
    let root = HotBoundary::from_canonical(&document, &catalog).expect("hot root");
    engine::admit(&document, &root, &catalog).expect("admitted");
    (root, catalog)
}

fn wire(action: Action) -> Value {
    serde_json::to_value(ExactSolveActionV1::from(action)).expect("wire action")
}

fn report(result: &IsmctsResult) -> Value {
    json!({
        "recommended": result.recommended.map(wire),
        "iterations": result.iterations,
        "transitions": result.transitions,
        "nodes": result.nodes,
        "receipt_nodes": result.receipt_nodes,
        "refusals": result.refusals,
        "root": result.root.iter().map(|stat| json!({
            "action": wire(stat.action),
            "visits": stat.visits,
            // A visit average under exploration and its sample interval,
            // not a value estimate (#3993).
            "rollout_mean": stat.mean,
            "rollout_mean_ci95": stat.ci95,
            "win_rate": stat.win_rate,
            "mean_final_hp": stat.mean_final_hp,
        })).collect::<Vec<_>>(),
    })
}

/// Play one true world to the end under one information mode.
fn play_world(
    world: &HotState,
    catalog: &Catalog,
    information: Information,
    iterations: u64,
    seed: u64,
) -> Value {
    let mut state = world.clone();
    let mut know = DrawKnowledge::none();
    let mut events = Vec::new();
    let mut decisions = 0u64;
    let mut transitions = 0u64;
    let started = Instant::now();
    while !state.history.over && state.hp > 0 && decisions < 400 {
        let mut params = IsmctsParams::new(information, iterations, seed ^ decisions);
        params.threads = threads();
        let result = match ismcts::search(&state, &know, catalog, &params) {
            Ok(result) => result,
            Err(refusal) => return json!({"status": "refused", "detail": format!("{refusal:?}")}),
        };
        transitions += result.transitions;
        let Some(action) = result.recommended else {
            return json!({"status": "refused", "detail": "no recommended action", "refusals": result.refusals});
        };
        let next = match engine::apply_action_into(&state, catalog, &action, &mut events) {
            Ok(next) => next,
            Err(error) => return json!({"status": "refused", "detail": format!("{error:?}")}),
        };
        know = match hidden::observe_step(catalog, &state, &know, &next, &events) {
            Ok(know) => know,
            Err(refusal) => return json!({"status": "refused", "detail": format!("{refusal:?}")}),
        };
        state = next;
        decisions += 1;
    }
    let won = state.history.over && state.hp > 0 && state.monsters.iter().all(|m| m.hp <= 0);
    json!({
        "status": "played",
        "won": won,
        "final_hp": if won { state.hp } else { 0 },
        "turns": state.turn,
        "decisions": decisions,
        "transitions": transitions,
        "seconds": started.elapsed().as_secs_f64(),
    })
}

fn summarize(outcomes: &[Value]) -> Value {
    let played: Vec<&Value> = outcomes
        .iter()
        .filter(|o| o["status"] == "played")
        .collect();
    let n = played.len() as f64;
    let hp: Vec<f64> = played
        .iter()
        .map(|o| o["final_hp"].as_f64().unwrap())
        .collect();
    let mean = hp.iter().sum::<f64>() / n.max(1.0);
    let variance = hp.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    json!({
        "played": played.len(),
        "refused": outcomes.len() - played.len(),
        "win_rate": played.iter().filter(|o| o["won"] == true).count() as f64 / n.max(1.0),
        "mean_final_hp": mean,
        "ci95_final_hp": 1.96 * (variance / n.max(1.0)).sqrt(),
        "seconds": played.iter().map(|o| o["seconds"].as_f64().unwrap()).sum::<f64>(),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: ismcts search ENTRY hidden|clairvoyant ITERATIONS SEED\n       \
                 ismcts play ENTRY ITERATIONS WORLDS SEED";
    match args.first().map(String::as_str) {
        Some("search") if args.len() == 5 => {
            let (root, catalog) = load(&args[1]);
            let information = match args[2].as_str() {
                "hidden" => Information::Hidden,
                "clairvoyant" => Information::Clairvoyant,
                _ => panic!("{usage}"),
            };
            let mut params = IsmctsParams::new(
                information,
                args[3].parse().expect(usage),
                args[4].parse().expect(usage),
            );
            params.threads = threads();
            let started = Instant::now();
            let result = ismcts::search(&root, &DrawKnowledge::none(), &catalog, &params)
                .expect("root knowledge");
            let mut out = report(&result);
            out["seconds"] = json!(started.elapsed().as_secs_f64());
            out["label"] = json!("estimate");
            println!("{out}");
        }
        Some("play") if args.len() == 5 => {
            let (root, catalog) = load(&args[1]);
            let iterations: u64 = args[2].parse().expect(usage);
            let worlds: u64 = args[3].parse().expect(usage);
            let seed: u64 = args[4].parse().expect(usage);
            let mut hidden_outcomes = Vec::new();
            let mut clairvoyant_outcomes = Vec::new();
            for index in 0..worlds {
                // World `index` is the same for both policies. World 0 is the
                // fixture's true world, the one the human played.
                let world = if index == 0 {
                    root.clone()
                } else {
                    hidden::sample_world(
                        &root,
                        &DrawKnowledge::none(),
                        &mut PolicyRng(0x574f_524c_4400_0000 ^ index),
                    )
                };
                let h = play_world(&world, &catalog, Information::Hidden, iterations, seed);
                let c = play_world(&world, &catalog, Information::Clairvoyant, iterations, seed);
                eprintln!("world {index}: hidden {h} clairvoyant {c}");
                hidden_outcomes.push(h);
                clairvoyant_outcomes.push(c);
            }
            println!(
                "{}",
                json!({
                    "label": "estimate",
                    "iterations_per_decision": iterations,
                    "worlds": worlds,
                    "hidden": summarize(&hidden_outcomes),
                    "clairvoyant": summarize(&clairvoyant_outcomes),
                    "true_world": {"hidden": hidden_outcomes[0], "clairvoyant": clairvoyant_outcomes[0]},
                })
            );
        }
        _ => panic!("{usage}"),
    }
}
