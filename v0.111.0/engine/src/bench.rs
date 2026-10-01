//! `sts-sim bench`: the measured workload behind the post-submit performance
//! floors (PORT_PLAN §6/§7).
//!
//! # What it measures, and why it lives here
//!
//! The floors lane needs one number that moves when the engine gets slower:
//! **transitions per second** over a search-shaped workload. "Search-shaped"
//! means the loop a UCT playout actually runs — enumerate the legal actions,
//! pick one, clone-and-apply — not `apply_action_into` in isolation. A
//! regression that doubles `legal_actions` cost is exactly the kind of creep
//! §7 exists to catch, so enumeration is inside the timed window.
//!
//! This module is **binary-local and engine-external**. It calls only the
//! public library surface `diff_serve` calls ([`HotBoundary`],
//! [`engine::admit`], [`engine::legal_actions`], [`engine::apply_action_into`])
//! and adds nothing to `sts_sim`'s API. A benchmark that needed a private hook
//! into the engine would be measuring a different engine than the one that
//! ships.
//!
//! # The workload is a synthetic stand-in for #1283
//!
//! PORT_PLAN §6 specifies throughput floors **per eval-suite fight**. The eval
//! suite is #1283 and does not exist yet, so the workload here is a
//! checked-in, byte-pinned canonical entry replayed under a seeded
//! uniform-random policy —
//! a deterministic stand-in with the right *shape*. When #1283 lands, each eval
//! fight becomes another [`Workload`] row and another floor entry in
//! `benchmarks/floors.json`; nothing else about the lane changes. The synthetic
//! row is kept afterwards as the fast smoke case.
//!
//! # Determinism
//!
//! Everything except the clock is deterministic:
//!
//! - the entry document is embedded in the binary (`include_str!`), so a run
//!   cannot pick up a stale or edited fixture from the filesystem, and its
//!   sha256 is reported;
//! - the policy is a seeded SplitMix64 whose per-trajectory stream is derived
//!   from `(seed, trajectory index)`, so trajectory *i* is the same sequence
//!   regardless of how many trajectories ran;
//! - the run reports a `checksum` folded over every action it applied. Two runs
//!   of the same workload that report different checksums did different work,
//!   and comparing their throughput would be meaningless — `perf_floor.py`
//!   asserts the checksums agree across its repeats.
//!
//! # Output
//!
//! One `sts-sim-bench/v1` JSON object on stdout. Under
//! `--features allocation-counting` the `allocation` block carries measured
//! allocations and bytes per transition (the process allocator counts them);
//! in a plain build the block reports `"instrumented": false` and nulls, rather
//! than zeros that could be mistaken for a measurement.
//!
//! Note the two allocation windows are deliberately different: the cargo-test
//! pin (`allocations_per_transition_stay_under_the_ceiling`) brackets
//! `apply_action_into` alone, while this one brackets the whole playout loop
//! and therefore also pays for `legal_actions` enumeration and the canonical
//! entry state's per-trajectory clone. This number is the larger of the two by
//! construction; `perf_floor.py` cross-checks that it stays above the cargo pin
//! rather than expecting them to match.

use std::fmt;
use std::time::Instant;

use sha2::{Digest, Sha256};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action};
use sts_sim::hot::HotState;

/// Schema tag of the JSON this subcommand writes. Bumped, never reinterpreted.
pub const SCHEMA_V1: &str = "sts-sim-bench/v1";

const USAGE: &str = "usage: sts-sim bench [--workload NAME] [--trajectories N] \
                     [--actions M] [--seed S] [--warmup T] [--list]";

/// The byte-pinned entry for the v1 synthetic workload.
///
/// This is deliberately distinct from the evolving canonical projection
/// fixture. A performance floor needs a fixed input: schema/projection coverage
/// may add a semantically inert field to the ordinary fixture without silently
/// changing the workload against which this floor is compared. The SHA is
/// asserted below against the original reviewed floor evidence.
const IRONCLAD_TOADPOLES: &str =
    include_str!("../fixtures/bench_ironclad_toadpoles_uniform_v1.json");

/// SHA-256 of `ironclad_toadpoles_uniform_v1` as originally reviewed.
///
/// Changing v1's bytes requires a new workload name and a separately reviewed
/// floor; this assertion prevents an unrelated canonical-fixture update from
/// invalidating the comparison.
#[cfg(test)]
const IRONCLAD_TOADPOLES_UNIFORM_V1_SHA256: &str =
    "6a5f557d8ae8b3966aeea58c623d2bdb6fdda9069ec3f1a4a05af32049f16ee2";

/// One named, fully specified benchmark workload.
///
/// A workload is identified by name in `benchmarks/floors.json`; changing any
/// field here changes what the floor means, so a field change and a floor
/// change travel together in one reviewed diff.
#[derive(Copy, Clone, Debug)]
pub struct Workload {
    /// Stable identifier, also the floors.json key.
    pub name: &'static str,
    /// The canonical v2 entry document, embedded at compile time.
    pub entry: &'static str,
    /// How many independent playouts to run.
    pub trajectories: u32,
    /// Maximum actions applied per playout before it is cut short.
    pub actions: u32,
    /// Policy seed.
    pub seed: u64,
    /// Playouts run before the clock starts, to warm caches and the allocator.
    pub warmup: u32,
}

/// Every workload this build can run.
///
/// One row today (the #1283 stand-in). Eval fights append rows; they do not
/// replace this one.
pub const WORKLOADS: &[Workload] = &[Workload {
    name: "ironclad_toadpoles_uniform_v1",
    entry: IRONCLAD_TOADPOLES,
    // Sized so a release run's timed window lands around two seconds on the
    // mac-solver class of machine (measured 2026-08-18: ~2.0 M transitions/s,
    // ~17 transitions per complete playout). Long enough that scheduler
    // hiccups average out and process startup is noise; short enough that
    // best-of-5 plus an allocation-counting pass fits the lane's 20-minute
    // timeout many times over. `actions` is a safety cap, not a budget the
    // workload spends: every playout of this entry reaches a terminal state
    // well inside it, which is what makes complete-playouts/s meaningful.
    trajectories: 240_000,
    actions: 64,
    seed: 0x5354_5332_5045_5246,
    warmup: 2_000,
}];

/// Typed refusal surface of `bench`, mirroring the binary's D6 discipline: a
/// bad invocation is a named refusal with a nonzero exit, never a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchRefusal {
    /// A flag outside the admitted set.
    UnknownFlag(String),
    /// A flag that takes a value was given none.
    MissingValue(String),
    /// A flag's value did not parse as the integer it requires.
    MalformedValue {
        /// The flag.
        flag: String,
        /// What was given.
        value: String,
    },
    /// `--workload` named something [`WORKLOADS`] does not carry.
    UnknownWorkload(String),
    /// A zero trajectory or action budget: it would measure nothing.
    EmptyBudget(String),
    /// The embedded entry did not survive parsing, validation, the canonical ⇄
    /// hot boundary, or the admission walk.
    EntryRejected(String),
}

impl fmt::Display for BenchRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(f, "refusal: bench: unknown flag {flag:?}; {USAGE}"),
            Self::MissingValue(flag) => {
                write!(f, "refusal: bench: {flag} requires a value; {USAGE}")
            }
            Self::MalformedValue { flag, value } => write!(
                f,
                "refusal: bench: {flag} expects an integer, got {value:?}; {USAGE}"
            ),
            Self::UnknownWorkload(name) => write!(
                f,
                "refusal: bench: unknown workload {name:?}; known: {}",
                WORKLOADS
                    .iter()
                    .map(|workload| workload.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::EmptyBudget(flag) => {
                write!(f, "refusal: bench: {flag} must be greater than zero")
            }
            Self::EntryRejected(detail) => {
                write!(f, "refusal: bench: entry rejected: {detail}")
            }
        }
    }
}

/// Deterministic policy RNG.
///
/// SplitMix64. This is **not** a game RNG and must never be confused with one —
/// it only decides which legal action the benchmark's uniform policy takes, and
/// its modulo reduction carries the usual negligible bias, which is irrelevant
/// to a throughput measurement. The game's parity RNG lives in `sts_sim::rng`
/// and is driven by the engine, not by this module.
struct PolicyRng(u64);

impl PolicyRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        word ^ (word >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// Rolling checksum over the actions a run applied.
///
/// FNV-1a over a stable encoding of every applied action, so "did these two
/// runs do the same work" is one integer comparison. It is a work identity, not
/// a state digest — the canonical digest is `diff-serve`'s job.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct WorkChecksum(u64);

impl WorkChecksum {
    const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn fold(&mut self, word: u64) {
        for byte in word.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    /// Fold one applied action as a stable `(tag, uid, target, selection)` encoding.
    fn fold_action(&mut self, action: &Action) {
        match action {
            Action::Play {
                uid,
                target,
                selection,
            } => {
                let word = (1u64 << 56)
                    | (u64::from(*uid) << 16)
                    | u64::from(target.map_or(255, u16::from));
                self.fold(word);
                self.fold(selection.get().map_or(u64::MAX, u64::from));
            }
            Action::UsePotion { slot, target } => {
                self.fold(
                    (2u64 << 56)
                        | (u64::from(*slot) << 16)
                        | u64::from(target.map_or(255, u16::from)),
                );
            }
            Action::EndTurn => self.fold(3u64 << 56),
            Action::Select { answer } => match answer {
                sts_sim::engine::SelectionAnswer::CardUid(uid) => {
                    self.fold((4u64 << 56) | u64::from(*uid))
                }
                sts_sim::engine::SelectionAnswer::OptionIndex(index) => {
                    self.fold((5u64 << 56) | u64::from(*index))
                }
            },
        }
    }
}

/// How each playout stopped. Reported so a green floor cannot hide a workload
/// that quietly stopped exercising the engine.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct StopTally {
    /// Combat ended (`history.over`).
    terminal: u32,
    /// The per-playout action budget ran out.
    budget: u32,
    /// The engine offered no legal action.
    no_legal_actions: u32,
    /// `apply_action_into` refused — an unmodeled path in this slice.
    refused: u32,
}

/// Everything one timed pass produced.
struct Measurement {
    transitions: u64,
    enumerations: u64,
    legal_actions_seen: u64,
    stops: StopTally,
    checksum: WorkChecksum,
    wall_nanos: u128,
    allocations: u64,
    allocated_bytes: u64,
}

/// Parsed invocation.
struct Config {
    workload: Workload,
    list: bool,
}

fn parse_u64(flag: &str, value: Option<&String>) -> Result<u64, BenchRefusal> {
    let value = value.ok_or_else(|| BenchRefusal::MissingValue(flag.to_string()))?;
    value
        .parse::<u64>()
        .map_err(|_| BenchRefusal::MalformedValue {
            flag: flag.to_string(),
            value: value.clone(),
        })
}

fn parse_u32(flag: &str, value: Option<&String>) -> Result<u32, BenchRefusal> {
    let parsed = parse_u64(flag, value)?;
    u32::try_from(parsed).map_err(|_| BenchRefusal::MalformedValue {
        flag: flag.to_string(),
        value: parsed.to_string(),
    })
}

fn parse_args(args: &[String]) -> Result<Config, BenchRefusal> {
    let mut workload = WORKLOADS[0];
    let mut list = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--list" => {
                list = true;
                index += 1;
            }
            "--workload" => {
                let name = args
                    .get(index + 1)
                    .ok_or_else(|| BenchRefusal::MissingValue(flag.to_string()))?;
                workload = *WORKLOADS
                    .iter()
                    .find(|candidate| candidate.name == name.as_str())
                    .ok_or_else(|| BenchRefusal::UnknownWorkload(name.clone()))?;
                index += 2;
            }
            "--trajectories" => {
                workload.trajectories = parse_u32(flag, args.get(index + 1))?;
                index += 2;
            }
            "--actions" => {
                workload.actions = parse_u32(flag, args.get(index + 1))?;
                index += 2;
            }
            "--warmup" => {
                workload.warmup = parse_u32(flag, args.get(index + 1))?;
                index += 2;
            }
            "--seed" => {
                workload.seed = parse_u64(flag, args.get(index + 1))?;
                index += 2;
            }
            other => return Err(BenchRefusal::UnknownFlag(other.to_string())),
        }
    }
    if workload.trajectories == 0 {
        return Err(BenchRefusal::EmptyBudget("--trajectories".to_string()));
    }
    if workload.actions == 0 {
        return Err(BenchRefusal::EmptyBudget("--actions".to_string()));
    }
    Ok(Config { workload, list })
}

/// Admit the workload's entry: parse, validate, cross the boundary, and run the
/// engine's admission walk — the same four gates `diff-serve`'s `load` runs, in
/// the same order, so a benchmark can never measure a state the differential
/// would refuse.
fn admit(entry: &str) -> Result<(HotState, Catalog), BenchRefusal> {
    let document: CanonicalStateV2 = serde_json::from_str(entry).map_err(|error| {
        BenchRefusal::EntryRejected(format!("not a canonical v2 document: {error}"))
    })?;
    document
        .validate_schema()
        .map_err(|refusal| BenchRefusal::EntryRejected(refusal.to_string()))?;
    let catalog = HotBoundary::catalog_from_canonical(&document)
        .map_err(|refusal| BenchRefusal::EntryRejected(refusal.to_string()))?;
    let state = HotBoundary::from_canonical(&document, &catalog)
        .map_err(|refusal| BenchRefusal::EntryRejected(refusal.to_string()))?;
    engine::admit(&document, &state, &catalog)
        .map_err(|refusal| BenchRefusal::EntryRejected(refusal.to_string()))?;
    Ok((state, catalog))
}

/// Per-trajectory policy stream, derived so trajectory *i* is independent of how
/// many trajectories the run does.
fn trajectory_seed(seed: u64, trajectory: u32) -> u64 {
    seed ^ u64::from(trajectory).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Run one playout, accumulating into the measurement counters.
///
/// Kept out of the timed loop's body only for readability — it is a plain call,
/// and the release profile (`lto = "thin"`, `codegen-units = 1`) inlines across
/// it as it would inside a search.
fn play_one(
    entry: &HotState,
    catalog: &Catalog,
    seed: u64,
    max_actions: u32,
    legal_actions: &mut engine::LegalActionBuffer,
    events: &mut Vec<engine::Event>,
    measurement: &mut Measurement,
) {
    let mut rng = PolicyRng::new(seed);
    let mut state = entry.clone();
    for _ in 0..max_actions {
        if state.history.over {
            measurement.stops.terminal += 1;
            return;
        }
        let actions = engine::legal_actions_into(&state, catalog, legal_actions);
        measurement.enumerations += 1;
        measurement.legal_actions_seen += actions.len() as u64;
        if actions.is_empty() {
            measurement.stops.no_legal_actions += 1;
            return;
        }
        let action = actions[rng.below(actions.len())];
        measurement.checksum.fold_action(&action);
        match engine::apply_action_into(&state, catalog, &action, events) {
            Ok(next) => {
                state = next;
                measurement.transitions += 1;
            }
            Err(_) => {
                // An unmodeled path in this slice. It ends the playout rather
                // than aborting the run: refusal is a first-class engine
                // answer (D6), and the tally makes its share visible.
                measurement.stops.refused += 1;
                return;
            }
        }
    }
    measurement.stops.budget += 1;
}

/// Run a workload and return its measurement.
fn measure(workload: &Workload, entry: &HotState, catalog: &Catalog) -> Measurement {
    // Caller-owned enumeration scratch is the engine/search API: one buffer
    // per worker, reused across every state and trajectory.
    let mut legal_actions = engine::LegalActionBuffer::new();
    let mut events: Vec<engine::Event> = Vec::new();
    let mut discard = Measurement {
        transitions: 0,
        enumerations: 0,
        legal_actions_seen: 0,
        stops: StopTally::default(),
        checksum: WorkChecksum::new(),
        wall_nanos: 0,
        allocations: 0,
        allocated_bytes: 0,
    };
    for trajectory in 0..workload.warmup {
        play_one(
            entry,
            catalog,
            trajectory_seed(workload.seed, trajectory),
            workload.actions,
            &mut legal_actions,
            &mut events,
            &mut discard,
        );
    }

    let mut measurement = Measurement {
        transitions: 0,
        enumerations: 0,
        legal_actions_seen: 0,
        stops: StopTally::default(),
        checksum: WorkChecksum::new(),
        wall_nanos: 0,
        allocations: 0,
        allocated_bytes: 0,
    };
    let (allocations_before, bytes_before) = sts_sim::allocation::snapshot();
    let started = Instant::now();
    for trajectory in 0..workload.trajectories {
        play_one(
            entry,
            catalog,
            trajectory_seed(workload.seed, trajectory),
            workload.actions,
            &mut legal_actions,
            &mut events,
            &mut measurement,
        );
    }
    measurement.wall_nanos = started.elapsed().as_nanos();
    let (allocations_after, bytes_after) = sts_sim::allocation::snapshot();
    measurement.allocations = allocations_after.saturating_sub(allocations_before);
    measurement.allocated_bytes = bytes_after.saturating_sub(bytes_before);
    measurement
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn per_transition(total: u64, transitions: u64) -> Option<f64> {
    (transitions > 0).then(|| total as f64 / transitions as f64)
}

fn report(workload: &Workload, measurement: &Measurement) -> serde_json::Value {
    let seconds = measurement.wall_nanos as f64 / 1e9;
    let instrumented = sts_sim::allocation::instrumented();
    let allocation = if instrumented {
        serde_json::json!({
            "instrumented": true,
            "allocations": measurement.allocations,
            "allocated_bytes": measurement.allocated_bytes,
            "allocations_per_transition":
                per_transition(measurement.allocations, measurement.transitions),
            "allocated_bytes_per_transition":
                per_transition(measurement.allocated_bytes, measurement.transitions),
        })
    } else {
        // Nulls, not zeros: a plain build did not measure zero allocations, it
        // did not measure allocations.
        serde_json::json!({
            "instrumented": false,
            "allocations": serde_json::Value::Null,
            "allocated_bytes": serde_json::Value::Null,
            "allocations_per_transition": serde_json::Value::Null,
            "allocated_bytes_per_transition": serde_json::Value::Null,
        })
    };
    serde_json::json!({
        "schema": SCHEMA_V1,
        "workload": workload.name,
        "crate_version": env!("CARGO_PKG_VERSION"),
        "entry_sha256": sha256_hex(workload.entry.as_bytes()),
        "config": {
            "trajectories": workload.trajectories,
            "actions": workload.actions,
            "seed": workload.seed,
            "warmup": workload.warmup,
        },
        "work": {
            "transitions": measurement.transitions,
            "legal_action_enumerations": measurement.enumerations,
            "legal_actions_seen": measurement.legal_actions_seen,
            "mean_branching_factor":
                per_transition(measurement.legal_actions_seen, measurement.enumerations),
            "checksum": format!("{:016x}", measurement.checksum.0),
            "stops": {
                "terminal": measurement.stops.terminal,
                "budget": measurement.stops.budget,
                "no_legal_actions": measurement.stops.no_legal_actions,
                "refused": measurement.stops.refused,
            },
        },
        "timing": {
            "wall_nanos": measurement.wall_nanos,
            "wall_seconds": seconds,
            "transitions_per_second":
                (seconds > 0.0).then(|| measurement.transitions as f64 / seconds),
            // PORT_PLAN §6 names complete playouts alongside transitions. It is
            // reported, not floored: on a workload whose playouts all reach a
            // terminal state it moves in lockstep with transitions/s, so
            // flooring both would only double the false-positive rate.
            "complete_playouts_per_second":
                (seconds > 0.0).then(|| f64::from(measurement.stops.terminal) / seconds),
            "nanos_per_transition":
                per_transition(measurement.wall_nanos as u64, measurement.transitions),
        },
        "allocation": allocation,
    })
}

fn list() -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA_V1,
        "workloads": WORKLOADS.iter().map(|workload| serde_json::json!({
            "name": workload.name,
            "entry_sha256": sha256_hex(workload.entry.as_bytes()),
            "trajectories": workload.trajectories,
            "actions": workload.actions,
            "seed": workload.seed,
            "warmup": workload.warmup,
        })).collect::<Vec<_>>(),
    })
}

/// Entry point `main` dispatches to. Returns the JSON line to print.
pub fn main(args: &[String]) -> Result<String, BenchRefusal> {
    let config = parse_args(args)?;
    if config.list {
        return Ok(list().to_string());
    }
    let (entry, catalog) = admit(config.workload.entry)?;
    let measurement = measure(&config.workload, &entry, &catalog);
    Ok(report(&config.workload, &measurement).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sts_sim::engine::SelectionRef;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    /// A tiny budget, so the tests measure the harness rather than spending a
    /// benchmark's worth of time inside `cargo test`.
    fn tiny() -> Vec<String> {
        argv(&["--trajectories", "8", "--actions", "6", "--warmup", "0"])
    }

    fn run_tiny() -> serde_json::Value {
        serde_json::from_str(&main(&tiny()).unwrap()).unwrap()
    }

    #[test]
    fn the_embedded_entry_is_admitted() {
        // If the fixture ever stops being admissible the benchmark must say so
        // loudly rather than reporting a fast zero-transition run.
        admit(IRONCLAD_TOADPOLES).expect("the checked-in fixture is admitted");
    }

    #[test]
    fn v1_entry_bytes_are_pinned_to_the_reviewed_floor() {
        assert_eq!(
            sha256_hex(IRONCLAD_TOADPOLES.as_bytes()),
            IRONCLAD_TOADPOLES_UNIFORM_V1_SHA256,
            "changing v1 requires a new workload name and reviewed floor"
        );
    }

    #[test]
    fn a_run_reports_the_schema_and_does_real_work() {
        let report = run_tiny();
        assert_eq!(report["schema"], SCHEMA_V1);
        assert_eq!(report["workload"], WORKLOADS[0].name);
        assert!(report["work"]["transitions"].as_u64().unwrap() > 0);
        assert!(report["timing"]["transitions_per_second"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn every_playout_is_accounted_for_by_exactly_one_stop_reason() {
        let report = run_tiny();
        let stops = &report["work"]["stops"];
        let total: u64 = ["terminal", "budget", "no_legal_actions", "refused"]
            .iter()
            .map(|key| stops[*key].as_u64().unwrap())
            .sum();
        assert_eq!(total, 8);
    }

    #[test]
    fn the_workload_is_deterministic_apart_from_the_clock() {
        let first = run_tiny();
        let second = run_tiny();
        assert_eq!(first["work"], second["work"]);
        assert_eq!(first["entry_sha256"], second["entry_sha256"]);
    }

    #[test]
    fn trajectory_streams_do_not_depend_on_the_trajectory_count() {
        // The prefix property `perf_floor.py`'s checksum comparison relies on:
        // trajectory i is the same playout in a short run and a long one.
        let short: serde_json::Value = serde_json::from_str(
            &main(&argv(&[
                "--trajectories",
                "4",
                "--actions",
                "6",
                "--warmup",
                "0",
            ]))
            .unwrap(),
        )
        .unwrap();
        let long: serde_json::Value = serde_json::from_str(&main(&tiny()).unwrap()).unwrap();
        assert!(
            long["work"]["transitions"].as_u64().unwrap()
                >= short["work"]["transitions"].as_u64().unwrap()
        );
        assert_ne!(short["work"]["checksum"], long["work"]["checksum"]);
    }

    #[test]
    fn a_different_seed_is_a_different_workload() {
        let mut other = tiny();
        other.extend(argv(&["--seed", "99"]));
        let rerun: serde_json::Value = serde_json::from_str(&main(&other).unwrap()).unwrap();
        assert_ne!(rerun["work"]["checksum"], run_tiny()["work"]["checksum"]);
    }

    #[test]
    fn the_allocation_block_never_reports_an_unmeasured_zero() {
        let report = run_tiny();
        let allocation = &report["allocation"];
        assert_eq!(
            allocation["instrumented"].as_bool().unwrap(),
            sts_sim::allocation::instrumented()
        );
        if allocation["instrumented"].as_bool().unwrap() {
            assert!(allocation["allocations_per_transition"].as_f64().unwrap() > 0.0);
        } else {
            assert!(allocation["allocations"].is_null());
            assert!(allocation["allocations_per_transition"].is_null());
        }
    }

    #[test]
    fn list_names_every_workload() {
        let listed: serde_json::Value =
            serde_json::from_str(&main(&argv(&["--list"])).unwrap()).unwrap();
        let names: Vec<&str> = listed["workloads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            WORKLOADS
                .iter()
                .map(|workload| workload.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn workload_names_are_unique() {
        // floors.json is keyed by name; two rows sharing one would silently
        // make one of them unfloored.
        let mut names: Vec<&str> = WORKLOADS.iter().map(|workload| workload.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn unknown_flags_and_workloads_refuse() {
        assert_eq!(
            main(&argv(&["--fast"])),
            Err(BenchRefusal::UnknownFlag("--fast".to_string()))
        );
        assert_eq!(
            main(&argv(&["--workload", "queen"])),
            Err(BenchRefusal::UnknownWorkload("queen".to_string()))
        );
        assert_eq!(
            main(&argv(&["--workload"])),
            Err(BenchRefusal::MissingValue("--workload".to_string()))
        );
        assert_eq!(
            main(&argv(&["--trajectories", "many"])),
            Err(BenchRefusal::MalformedValue {
                flag: "--trajectories".to_string(),
                value: "many".to_string(),
            })
        );
        assert_eq!(
            main(&argv(&["--trajectories", "0"])),
            Err(BenchRefusal::EmptyBudget("--trajectories".to_string()))
        );
        assert_eq!(
            main(&argv(&["--actions", "0"])),
            Err(BenchRefusal::EmptyBudget("--actions".to_string()))
        );
    }

    #[test]
    fn a_rejected_entry_refuses_instead_of_measuring_nothing() {
        assert!(matches!(
            admit("{\"schema\":\"nonsense\"}"),
            Err(BenchRefusal::EntryRejected(_))
        ));
    }

    #[test]
    fn the_checksum_separates_action_shapes() {
        let mut play = WorkChecksum::new();
        play.fold_action(&Action::Play {
            uid: 3,
            target: Some(1),
            selection: SelectionRef::new(Some(2)),
        });
        let mut untargeted = WorkChecksum::new();
        untargeted.fold_action(&Action::Play {
            uid: 3,
            target: None,
            selection: SelectionRef::new(Some(2)),
        });
        let mut unselected = WorkChecksum::new();
        unselected.fold_action(&Action::Play {
            uid: 3,
            target: Some(1),
            selection: SelectionRef::NONE,
        });
        let mut end = WorkChecksum::new();
        end.fold_action(&Action::EndTurn);
        assert_ne!(play, untargeted);
        assert_ne!(play, unselected);
        assert_ne!(play, end);
        assert_ne!(untargeted, end);
    }
}
