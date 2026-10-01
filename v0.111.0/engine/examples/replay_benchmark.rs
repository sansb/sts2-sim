//! Fixed-trajectory engine throughput, with no search or serialization per step.
use serde_json::json;
use std::{env, fs, time::Instant};
use sts_sim::{
    boundary::HotBoundary, canonical::CanonicalStateV2, engine, exact_solve_v1::ExactSolveActionV1,
    solo_v1::admit_exact_solve,
};

fn main() {
    let args: Vec<_> = env::args().collect();
    let doc: CanonicalStateV2 =
        serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let wire: Vec<ExactSolveActionV1> =
        serde_json::from_str(&fs::read_to_string(&args[2]).unwrap()).unwrap();
    let actions: Vec<engine::Action> = wire.into_iter().map(|a| a.try_into().unwrap()).collect();
    let repeats: u64 = args[3].parse().unwrap();
    assert!(repeats > 0);
    admit_exact_solve(&doc).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let root = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &root, &catalog).unwrap();
    let mut events = vec![];
    let mut state = root.clone();
    let started = Instant::now();
    for _ in 0..repeats {
        state = root.clone();
        for action in &actions {
            state = engine::apply_action_into(&state, &catalog, action, &mut events).unwrap();
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "{}",
        json!({"repeats":repeats,"transitions":repeats * actions.len() as u64,
        "seconds":elapsed,"transitions_per_second":repeats as f64 * actions.len() as f64 / elapsed,
        "final_digest":HotBoundary::try_to_canonical(&state,&catalog).unwrap().differential_digest()})
    );
}
