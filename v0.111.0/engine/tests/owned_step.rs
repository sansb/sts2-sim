//! `engine::apply_action_owned` against `engine::apply_action_into` (#3976).
//!
//! The owned step writes the state in place and drops its predecessor, so it
//! is exact only if every step it takes equals the borrowing step from the
//! same state. These tests walk every eval fixture twice, once in each form,
//! and compare after every action: the fixture's human line (certified
//! against the game's checksums), then seeded uniform-random walks, which
//! reach states no human line does.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action, Event, LegalActionBuffer};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hot::HotState;

/// Random-walk actions per fixture and seed: long enough to cross several
/// turns and reshuffles, short enough to keep a debug run in seconds.
const WALK_ACTIONS: usize = 80;
const WALK_SEEDS: [u64; 2] = [0x5354_5332_4f57_4e31, 0x5354_5332_4f57_4e32];

fn fights() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights")
}

/// Every fixture the engine admits, as (id, catalog, root, human line).
fn admitted_fixtures() -> Vec<(String, Catalog, HotState, Vec<Action>)> {
    let mut ids: Vec<String> = std::fs::read_dir(fights())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            let dir = fights().join(&id);
            let document: CanonicalStateV2 =
                serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).ok()?)
                    .unwrap();
            let line: Value =
                serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).ok()?).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).ok()?;
            let root = HotBoundary::from_canonical(&document, &catalog).ok()?;
            engine::admit(&document, &root, &catalog).ok()?;
            let actions = line["actions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|wire| {
                    let wire: ExactSolveActionV1 = serde_json::from_value(wire.clone()).unwrap();
                    Action::try_from(wire).unwrap()
                })
                .collect();
            Some((id, catalog, root, actions))
        })
        .collect()
}

/// What one paired step did, for the corpus-level tallies.
#[derive(Default)]
struct Tally {
    steps: usize,
    rebuilds: usize,
    refusals: usize,
}

fn replay(root: &HotState, catalog: &Catalog, line: &[Action]) -> HotState {
    let mut events = Vec::new();
    line.iter().fold(root.clone(), |state, action| {
        engine::apply_action_into(&state, catalog, action, &mut events).unwrap()
    })
}

/// Two chains walked side by side from the same root: one stepped by the
/// borrowing form, one by the owned form. The owned chain is never cloned
/// after the root, so from its first write on its copy-on-write components are
/// uniquely owned and the owned step really mutates in place, as bench and
/// search run it.
struct Chains {
    borrowed: HotState,
    owned: HotState,
}

/// Apply `action` to both chains (each reached from `root` by `line`) and
/// assert the two agree on the result and the events. Returns `false` on a
/// refusal, which both forms must agree on too; the owned chain is then gone.
fn paired_step(
    id: &str,
    catalog: &Catalog,
    root: &HotState,
    line: &[Action],
    chains: &mut Chains,
    action: &Action,
    tally: &mut Tally,
) -> bool {
    let mut borrowed_events: Vec<Event> = Vec::new();
    let borrowed =
        engine::apply_action_into(&chains.borrowed, catalog, action, &mut borrowed_events);
    let rebuilt = Cell::new(false);
    let mut owned_events: Vec<Event> = Vec::new();
    let owned_input = std::mem::replace(&mut chains.owned, root.clone());
    let owned = engine::apply_action_owned(owned_input, catalog, action, &mut owned_events, || {
        rebuilt.set(true);
        let predecessor = replay(root, catalog, line);
        assert!(
            predecessor == chains.borrowed,
            "{id}: the rebuilt predecessor differs at step {}",
            line.len()
        );
        predecessor
    });
    let at = format!("{id} step {} {action:?}", line.len());
    assert_eq!(owned, borrowed, "{at}: owned and borrowed results differ");
    assert_eq!(owned_events, borrowed_events, "{at}: events differ");
    tally.steps += 1;
    tally.rebuilds += usize::from(rebuilt.get());
    tally.refusals += usize::from(borrowed.is_err());
    match (owned, borrowed) {
        (Ok(owned), Ok(borrowed)) => {
            *chains = Chains { borrowed, owned };
            true
        }
        _ => false,
    }
}

/// SplitMix64, as the bench's policy: not a game RNG.
fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut word = *seed;
    word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    word ^ (word >> 31)
}

#[test]
fn owned_steps_equal_borrowed_steps_on_every_eval_fixture() {
    let fixtures = admitted_fixtures();
    assert!(
        fixtures.len() > 500,
        "only {} eval fixtures admitted",
        fixtures.len()
    );
    let mut human = Tally::default();
    let mut random = Tally::default();
    let mut legal = LegalActionBuffer::new();
    for (id, catalog, root, actions) in &fixtures {
        let mut chains = Chains {
            borrowed: root.clone(),
            owned: root.clone(),
        };
        for (index, action) in actions.iter().enumerate() {
            assert!(
                paired_step(
                    id,
                    catalog,
                    root,
                    &actions[..index],
                    &mut chains,
                    action,
                    &mut human,
                ),
                "{id}: the certified human line refused at step {index}"
            );
        }
        for seed in WALK_SEEDS {
            let mut rng = seed ^ u64::from_str_radix(&id[1..], 16).unwrap();
            let mut chains = Chains {
                borrowed: root.clone(),
                owned: root.clone(),
            };
            let mut line = Vec::new();
            while line.len() < WALK_ACTIONS && !chains.borrowed.history.over {
                let actions = engine::legal_actions_into(&chains.borrowed, catalog, &mut legal);
                if actions.is_empty() {
                    break;
                }
                let action = actions[(next(&mut rng) % actions.len() as u64) as usize];
                if !paired_step(id, catalog, root, &line, &mut chains, &action, &mut random) {
                    break;
                }
                line.push(action);
            }
        }
    }
    eprintln!(
        "owned_step: {} fixtures; human {} steps, {} rebuilds; random {} steps, {} rebuilds, \
         {} refusals",
        fixtures.len(),
        human.steps,
        human.rebuilds,
        random.steps,
        random.rebuilds,
        random.refusals
    );
    assert!(
        human.rebuilds + random.rebuilds > 0,
        "no step reached the parked-selection rebuild; the witness is vacuous"
    );
}
