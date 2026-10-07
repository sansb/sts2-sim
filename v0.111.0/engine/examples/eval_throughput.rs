//! Random-playout throughput for each eval fight, with the same timed loop as
//! `sts-sim bench`: list the legal actions, select one at random, copy the
//! state, apply the action.
//!
//! usage: eval_throughput FIGHTS_DIR [MIN_SECONDS]
//!
//! Prints one JSON line per fight whose `entry.canonical.json` the engine
//! admits, then one summary line.
use serde_json::json;
use std::{env, fs, path::Path, time::Instant};
use sts_sim::{
    boundary::HotBoundary, canonical::CanonicalStateV2, catalog::Catalog, engine, hot::HotState,
};

const SEED: u64 = 0x5354_5332_5045_5246;
const MAX_ACTIONS: u32 = 4096;
const WARMUP: u32 = 50;
const BATCH: u32 = 50;

struct PolicyRng(u64);

impl PolicyRng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((word ^ (word >> 31)) % bound as u64) as usize
    }
}

#[derive(Default)]
struct Tally {
    transitions: u64,
    legal_actions_seen: u64,
    terminal: u64,
    refused: u64,
    other: u64,
}

fn admit(path: &Path) -> Result<(HotState, Catalog), String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document: CanonicalStateV2 =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    document
        .validate_schema()
        .map_err(|refusal| refusal.to_string())?;
    let catalog =
        HotBoundary::catalog_from_canonical(&document).map_err(|refusal| refusal.to_string())?;
    let state =
        HotBoundary::from_canonical(&document, &catalog).map_err(|refusal| refusal.to_string())?;
    engine::admit(&document, &state, &catalog).map_err(|refusal| refusal.to_string())?;
    Ok((state, catalog))
}

fn play_one(
    entry: &HotState,
    catalog: &Catalog,
    trajectory: u32,
    legal_actions: &mut engine::LegalActionBuffer,
    events: &mut Vec<engine::Event>,
    tally: &mut Tally,
) {
    let mut rng = PolicyRng(SEED ^ u64::from(trajectory).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut state = entry.clone();
    for _ in 0..MAX_ACTIONS {
        if state.history.over {
            tally.terminal += 1;
            return;
        }
        let actions = engine::legal_actions_into(&state, catalog, legal_actions);
        tally.legal_actions_seen += actions.len() as u64;
        if actions.is_empty() {
            tally.other += 1;
            return;
        }
        let action = actions[rng.below(actions.len())];
        match engine::apply_action_into(&state, catalog, &action, events) {
            Ok(next) => {
                state = next;
                tally.transitions += 1;
            }
            Err(_) => {
                tally.refused += 1;
                return;
            }
        }
    }
    tally.other += 1;
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let min_seconds: f64 = args.get(2).map_or(0.25, |value| value.parse().unwrap());
    let mut fights: Vec<_> = fs::read_dir(&args[1])
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    fights.sort();
    let mut rates = Vec::new();
    let mut not_admitted = 0u32;
    for fight in &fights {
        let Ok((entry, catalog)) = admit(&fight.join("entry.canonical.json")) else {
            not_admitted += 1;
            continue;
        };
        let mut legal_actions = engine::LegalActionBuffer::new();
        let mut events = Vec::new();
        let mut discard = Tally::default();
        for trajectory in 0..WARMUP {
            play_one(
                &entry,
                &catalog,
                trajectory,
                &mut legal_actions,
                &mut events,
                &mut discard,
            );
        }
        let mut tally = Tally::default();
        let mut playouts = 0u32;
        let started = Instant::now();
        while started.elapsed().as_secs_f64() < min_seconds {
            for _ in 0..BATCH {
                play_one(
                    &entry,
                    &catalog,
                    playouts,
                    &mut legal_actions,
                    &mut events,
                    &mut tally,
                );
                playouts += 1;
            }
        }
        let seconds = started.elapsed().as_secs_f64();
        if tally.transitions == 0 {
            not_admitted += 1;
            continue;
        }
        let rate = tally.transitions as f64 / seconds;
        rates.push(rate);
        println!(
            "{}",
            json!({
                "fight": fight.file_name().unwrap().to_string_lossy(),
                "transitions_per_second": rate,
                "playouts_per_second": f64::from(playouts) / seconds,
                "transitions_per_playout": tally.transitions as f64 / f64::from(playouts),
                "mean_branching_factor":
                    tally.legal_actions_seen as f64 / (tally.transitions + tally.refused) as f64,
                "playouts": playouts,
                "terminal": tally.terminal,
                "refused": tally.refused,
                "other": tally.other,
            })
        );
    }
    rates.sort_by(f64::total_cmp);
    let at = |fraction: f64| rates[((rates.len() - 1) as f64 * fraction).round() as usize];
    println!(
        "{}",
        json!({
            "summary": true,
            "fights_measured": rates.len(),
            "fights_not_admitted": not_admitted,
            "min": at(0.0), "p10": at(0.1), "p25": at(0.25), "median": at(0.5),
            "p75": at(0.75), "p90": at(0.9), "max": at(1.0),
        })
    );
}
