//! Information-set Monte Carlo tree search (#3436).
//!
//! A fight solver that decides from what the player observes, not from the
//! true draw order and RNG. It is single-observer ISMCTS: each iteration
//! samples one world from the root's information set
//! ([`hidden::sample_world`]) and steps that world down a tree whose nodes are
//! **information sets** ([`hidden::info_key`]), choosing by UCB over the
//! statistics of the information set, never of the world. A world sampled at
//! the root and stepped forward is a draw from the true posterior over
//! everything hidden at every later node it reaches, so this needs no
//! per-edge resampling: the only hidden information in a solo fight is chance,
//! with no adversary holding private information.
//!
//! Strategy fusion — choosing differently in two worlds the player cannot tell
//! apart — is impossible by construction: every world of an information set
//! reaches the same node and reads the same statistics.
//!
//! # Honesty contract (#1136)
//!
//! The result is an **estimate** with a confidence interval. It is never
//! labelled exact or optimal: the tree's values are means over a rollout
//! policy, and a finite tree under-explores.
//!
//! # Clairvoyant mode
//!
//! [`Information::Clairvoyant`] runs the same search on the true state, keyed
//! by the full state, with no resampling. It is the like-for-like known-draws
//! baseline: the difference between the two modes on the same budget is the
//! value of the hidden information to this search.

use std::collections::{BTreeMap, HashMap};

use crate::catalog::Catalog;
use crate::engine::{self, Action, BattlewornObjective, LegalActionBuffer};
use std::hash::{Hash, Hasher};

use crate::hidden::{self, DrawKnowledge, HiddenRefusal, PolicyRng};
use crate::hot::HotState;
use crate::search::{enemy_hp, reward, stopped, won};

/// What the search may see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Information {
    /// Only the player's observation: draw order and stream words hidden.
    Hidden,
    /// The true state: the known-draws baseline.
    Clairvoyant,
}

/// One search's budget and policy.
#[derive(Clone, Debug)]
pub struct IsmctsParams {
    /// What the search may see.
    pub information: Information,
    /// Iterations to run.
    pub iterations: u64,
    /// UCB exploration constant over rewards in `[-1, 1]`.
    pub exploration: f64,
    /// Rollout weight of `EndTurn` relative to each other legal action.
    pub end_weight: f64,
    /// Turn past which a line is cut off (scored as a non-win).
    pub horizon: i16,
    /// Search-policy randomness; independent of every game stream.
    pub seed: u64,
    /// Independent root-parallel trees (each with its own seed and a share of
    /// the iterations), their root statistics summed. 1 searches one tree.
    /// Ignored on wasm32, which has no threads.
    pub threads: usize,
}

impl IsmctsParams {
    /// Defaults matching `search.rs`'s UCT, over `iterations`.
    #[must_use]
    pub fn new(information: Information, iterations: u64, seed: u64) -> Self {
        Self {
            information,
            iterations,
            exploration: 1.414,
            end_weight: 0.05,
            horizon: 30,
            seed,
            threads: 1,
        }
    }
}

/// One root action's statistics.
#[derive(Clone, Debug)]
pub struct RootStat {
    /// The action.
    pub action: Action,
    /// Iterations through it.
    pub visits: u64,
    /// Mean reward over the iterations that took this action
    /// (`search::reward` scale: wins in `(0.5, 1]`).
    ///
    /// This is a **visit average under UCB exploration**, not the action's
    /// value: the subtree below it keeps sampling worse lines, so it sits
    /// below the value of the best continuation (#3993). Use it to rank root
    /// actions; for the value of playing this way, use closed-loop play
    /// (`examples/ismcts.rs play`).
    pub mean: f64,
    /// Half-width of the 95% normal interval of [`RootStat::mean`] as a
    /// sample mean. It is **not** a confidence interval for the action's
    /// value: against an exhaustive expectimax oracle (#3985) it missed low in
    /// 3–6 of 16 runs at 60k–400k iterations (#3993).
    pub ci95: f64,
    /// Fraction of its iterations that ended in a win.
    pub win_rate: f64,
    /// Mean final HP over its iterations (0 for a loss).
    pub mean_final_hp: f64,
}

/// A finished search.
#[derive(Clone, Debug)]
pub struct IsmctsResult {
    /// Every root action that was tried, most visited first.
    pub root: Vec<RootStat>,
    /// The most visited root action; `None` when nothing was legal.
    pub recommended: Option<Action>,
    /// Iterations run.
    pub iterations: u64,
    /// Engine transitions applied, rollouts included.
    pub transitions: u64,
    /// Information-set nodes created.
    pub nodes: usize,
    /// Of those, nodes identified by their sanitized document because they
    /// carry an action-replay receipt.
    pub receipt_nodes: usize,
    /// Refusals met, by name, and how often. A refused edge is pruned.
    pub refusals: BTreeMap<String, u64>,
}

/// Why a search could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsmctsRefusal {
    /// The root's knowledge does not describe its draw pile.
    Root(HiddenRefusal),
}

#[derive(Default)]
struct Edge {
    visits: u64,
    value_sum: f64,
    value_sq: f64,
    wins: u64,
    final_hp_sum: f64,
    refused: bool,
}

struct Node {
    /// The information set's canonical representative (the true state in
    /// clairvoyant mode) and knowledge: a lookup hit is confirmed by `==` on
    /// both, so a hash collision can never merge two information sets.
    rep: NodeIdentity,
    know: DrawKnowledge,
    /// Legal actions, in the order first seen: the set every world of this
    /// information set must reproduce.
    actions: Vec<Action>,
    edges: Vec<Edge>,
    visits: u64,
}

/// Same multiset of actions, in any order. Enumeration order can follow the
/// hidden draw order (a selection from the draw pile lists its options in
/// pile order), so order is not compared.
///
/// Sets are usually small, but a combinatorial selection is not: one decision
/// in f5a68f3e1efbd383 offers 13,700 options, and this check runs on every
/// revisit of a hidden node. A pairwise scan there cost ~188M comparisons
/// per revisit and stalled a 6-thread search for hours. Identical order (the
/// usual case) is one linear pass; small sets keep the allocation-free scan;
/// anything else counts occurrences.
fn same_action_set(a: &[Action], b: &[Action]) -> bool {
    const PAIRWISE: usize = 32;
    if a.len() != b.len() {
        return false;
    }
    if a == b {
        return true;
    }
    if a.len() <= PAIRWISE {
        return a.iter().all(|action| {
            a.iter().filter(|x| *x == action).count() == b.iter().filter(|x| *x == action).count()
        });
    }
    let mut counts: HashMap<Action, isize> = HashMap::with_capacity(a.len());
    for action in a {
        *counts.entry(*action).or_default() += 1;
    }
    for action in b {
        match counts.get_mut(action) {
            Some(count) if *count > 0 => *count -= 1,
            _ => return false,
        }
    }
    true
}

/// A fast 64-bit hash for the node index. Collisions are harmless: every hit
/// is confirmed by full equality.
struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// What identifies a node: the hot canonical representative, or — for a
/// state carrying an action-replay receipt, whose embedded predecessor holds
/// the hidden draw order and words out of the representative's reach — the
/// sanitized document key ([`hidden::receipt_aware_key`]).
#[derive(Clone, PartialEq, Eq, Hash)]
enum NodeIdentity {
    Hot(HotState),
    Document(hidden::InfoKey),
    /// A state carrying a bottom receipt that lifts out: the rest of the state's
    /// representative, the receipt's action and answers, and the node the
    /// receipt's predecessor was (its `origin`, or [`ROOT_RECEIPT_ORIGIN`]
    /// for a receipt the root already carries). That node's identity *is*
    /// the predecessor's information set, so this needs no projection. A
    /// transposition reaching one information set through two origins is
    /// split in two: finer than the player's partition, never coarser.
    Receipt {
        rest: HotState,
        parts: hidden::ReceiptParts,
        origin: usize,
    },
}

/// The origin of a receipt the search root already carries.
const ROOT_RECEIPT_ORIGIN: usize = usize::MAX;

/// The node identity of `state` under `know`; `origin` is the node its
/// receipt's predecessor was, when it carries one.
fn representative(
    information: Information,
    state: &HotState,
    know: &DrawKnowledge,
    catalog: &Catalog,
    origin: Option<usize>,
) -> Result<NodeIdentity, HiddenRefusal> {
    if information == Information::Hidden {
        hidden::check_frozen_views(state, catalog)?;
    }
    Ok(match information {
        Information::Hidden if hidden::has_replay_receipt(state) => {
            match (origin, hidden::sole_receipt_parts(state, know)) {
                (Some(origin), Some((rest, parts))) => NodeIdentity::Receipt {
                    rest,
                    parts,
                    origin,
                },
                _ => NodeIdentity::Document(hidden::receipt_aware_key(state, know, catalog)?),
            }
        }
        Information::Hidden => NodeIdentity::Hot(hidden::canonical_representative(state, know)),
        Information::Clairvoyant => NodeIdentity::Hot(state.clone()),
    })
}

fn fast_hash(rep: &NodeIdentity, know: &DrawKnowledge) -> u64 {
    let mut hasher = FastHasher(0);
    rep.hash(&mut hasher);
    know.hash(&mut hasher);
    hasher.finish()
}

/// Information-set nodes, found by fast hash and confirmed by equality.
#[derive(Default)]
struct NodeIndex(HashMap<u64, Vec<usize>>);

impl NodeIndex {
    fn find(
        &self,
        nodes: &[Node],
        hash: u64,
        rep: &NodeIdentity,
        know: &DrawKnowledge,
    ) -> Option<usize> {
        self.0.get(&hash)?.iter().copied().find(|&id| {
            let node = &nodes[id];
            node.know == *know && node.rep == *rep
        })
    }
    fn insert(&mut self, hash: u64, id: usize) {
        self.0.entry(hash).or_default().push(id);
    }
}

fn note(refusals: &mut BTreeMap<String, u64>, name: String) {
    *refusals.entry(name).or_default() += 1;
}

/// The state after `line` from `start`: an owned step's rebuilt predecessor.
/// Every action already applied once from the same states, and transitions
/// are deterministic, so a refusal here is a defect.
fn replay(start: &HotState, catalog: &Catalog, line: &[Action]) -> HotState {
    let mut events = Vec::new();
    line.iter().fold(start.clone(), |state, action| {
        engine::apply_action_into(&state, catalog, action, &mut events)
            .expect("an applied line replays: transitions are deterministic")
    })
}

/// Weighted-random rollout choice: `EndTurn` at `end_weight`, the rest at 1.
fn rollout_action(actions: &[Action], end_weight: f64, rng: &mut PolicyRng) -> Action {
    let weight = |action: &Action| {
        if matches!(action, Action::EndTurn) {
            end_weight
        } else {
            1.0
        }
    };
    let total: f64 = actions.iter().map(weight).sum();
    let mut choice = rng.unit() * total;
    for action in actions {
        choice -= weight(action);
        if choice < 0.0 {
            return *action;
        }
    }
    *actions
        .last()
        .expect("a nonterminal state offers an action")
}

/// One root action's raw sums, as one tree accumulated them.
#[derive(Clone)]
struct RootSums {
    action: Action,
    visits: u64,
    value_sum: f64,
    value_sq: f64,
    wins: u64,
    final_hp_sum: f64,
}

/// Root statistics summed by action across trees, in first-seen order (the
/// final sort is stable, so that order breaks its ties).
///
/// A root can offer a combinatorial selection's hundreds of thousands of
/// answers. Matching each tree's row with a linear scan of the merged rows
/// was quadratic in that count, and stalled f5a68f3e1efbd383 for over half
/// an hour after its trees had finished; the index makes it linear.
#[derive(Default)]
struct RootMerge {
    rows: Vec<RootSums>,
    at: HashMap<Action, usize>,
}

impl RootMerge {
    fn add(&mut self, sums: RootSums) {
        match self.at.get(&sums.action) {
            Some(&at) => {
                let row = &mut self.rows[at];
                row.visits += sums.visits;
                row.value_sum += sums.value_sum;
                row.value_sq += sums.value_sq;
                row.wins += sums.wins;
                row.final_hp_sum += sums.final_hp_sum;
            }
            None => {
                self.at.insert(sums.action, self.rows.len());
                self.rows.push(sums);
            }
        }
    }
}

/// What one tree produced, before trees are merged.
struct TreeOutcome {
    root: Vec<RootSums>,
    iterations: u64,
    transitions: u64,
    nodes: usize,
    receipt_nodes: usize,
    refusals: BTreeMap<String, u64>,
}

/// Run one search from `root`, whose player knows `know` about its draw pile.
///
/// With `params.threads > 1` (not on wasm32) it runs that many independent
/// trees on scoped threads — root parallelism: each tree gets its own seed and
/// an equal share of the iterations — and sums their root statistics. The
/// result is deterministic for a fixed thread count.
///
/// # Errors
///
/// [`IsmctsRefusal::Root`] when `know` does not describe `root`.
pub fn search(
    root: &HotState,
    know: &DrawKnowledge,
    catalog: &Catalog,
    params: &IsmctsParams,
) -> Result<IsmctsResult, IsmctsRefusal> {
    let threads = if cfg!(target_arch = "wasm32") {
        1
    } else {
        params.threads.clamp(1, 256)
    };
    let outcomes: Vec<Result<TreeOutcome, IsmctsRefusal>> = if threads == 1 {
        vec![search_tree(root, know, catalog, params)]
    } else {
        let shares: Vec<IsmctsParams> = (0..threads as u64)
            .map(|index| {
                let mut share = params.clone();
                share.threads = 1;
                share.iterations = params.iterations / threads as u64
                    + u64::from(index < params.iterations % threads as u64);
                share.seed = hidden::PolicyRng(
                    params.seed ^ (index + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                )
                .next_u64();
                share
            })
            .collect();
        std::thread::scope(|scope| {
            let handles: Vec<_> = shares
                .iter()
                .map(|share| scope.spawn(move || search_tree(root, know, catalog, share)))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a search tree panicked"))
                .collect()
        })
    };
    let mut merged = RootMerge::default();
    let (mut iterations, mut transitions, mut nodes, mut receipt_nodes) = (0, 0, 0, 0);
    let mut refusals = BTreeMap::new();
    for outcome in outcomes {
        let outcome = outcome?;
        iterations += outcome.iterations;
        transitions += outcome.transitions;
        nodes += outcome.nodes;
        receipt_nodes += outcome.receipt_nodes;
        for (name, count) in outcome.refusals {
            *refusals.entry(name).or_default() += count;
        }
        for sums in outcome.root {
            merged.add(sums);
        }
    }
    let mut root_stats: Vec<RootStat> = merged
        .rows
        .iter()
        .filter(|sums| sums.visits > 0)
        .map(|sums| {
            let n = sums.visits as f64;
            let mean = sums.value_sum / n;
            let variance = (sums.value_sq / n - mean * mean).max(0.0);
            RootStat {
                action: sums.action,
                visits: sums.visits,
                mean,
                ci95: 1.96 * (variance / n).sqrt(),
                win_rate: sums.wins as f64 / n,
                mean_final_hp: sums.final_hp_sum / n,
            }
        })
        .collect();
    root_stats.sort_by(|a, b| b.visits.cmp(&a.visits).then(b.mean.total_cmp(&a.mean)));
    Ok(IsmctsResult {
        recommended: root_stats.first().map(|stat| stat.action),
        root: root_stats,
        iterations,
        transitions,
        nodes,
        receipt_nodes,
        refusals,
    })
}

fn search_tree(
    root: &HotState,
    know: &DrawKnowledge,
    catalog: &Catalog,
    params: &IsmctsParams,
) -> Result<TreeOutcome, IsmctsRefusal> {
    hidden::check_knowledge(root, know).map_err(IsmctsRefusal::Root)?;
    let mut rng = PolicyRng(params.seed ^ 0x4953_4d43_5453_0001);
    let battleworn = BattlewornObjective::of_root(root);
    let initial_enemy_hp = enemy_hp(root);
    let mut legal = LegalActionBuffer::new();
    let mut events = Vec::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut index = NodeIndex::default();
    let mut refusals = BTreeMap::new();
    let mut transitions = 0u64;
    if params.information == Information::Hidden {
        hidden::check_root(root).map_err(IsmctsRefusal::Root)?;
    }
    let root_origin = hidden::has_replay_receipt(root).then_some(ROOT_RECEIPT_ORIGIN);
    let root_rep = representative(params.information, root, know, catalog, root_origin)
        .map_err(IsmctsRefusal::Root)?;
    let root_hash = fast_hash(&root_rep, know);
    let mut line: Vec<Action> = Vec::new();
    let mut iterations = 0;
    while iterations < params.iterations {
        iterations += 1;
        let mut world = match params.information {
            Information::Hidden => hidden::sample_world(root, know, &mut rng),
            Information::Clairvoyant => root.clone(),
        };
        // The iteration's starting world and every action applied to it, so
        // an owned rollout step can rebuild its predecessor (#3976).
        let start = world.clone();
        line.clear();
        let mut world_know = know.clone();
        let mut rep = root_rep.clone();
        let mut hash = root_hash;
        let mut origin = root_origin;
        let mut path: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0usize;
        let mut aborted = false;
        // Selection and expansion.
        loop {
            if stopped(&world, depth, params.horizon) {
                break;
            }
            let (id, fresh) = match index.find(&nodes, hash, &rep, &world_know) {
                Some(id) => (id, false),
                None => {
                    let actions = match engine::legal_actions_checked(&world, catalog, &mut legal) {
                        Ok(actions) => actions.to_vec(),
                        Err(error) => {
                            note(&mut refusals, format!("enumeration: {error:?}"));
                            aborted = true;
                            break;
                        }
                    };
                    nodes.push(Node {
                        rep: rep.clone(),
                        know: world_know.clone(),
                        edges: actions.iter().map(|_| Edge::default()).collect(),
                        actions,
                        visits: 0,
                    });
                    index.insert(hash, nodes.len() - 1);
                    (nodes.len() - 1, true)
                }
            };
            if fresh && depth > 0 {
                // One new information set per iteration: roll out from it.
                break;
            }
            if params.information == Information::Hidden && !fresh {
                // Every world of an information set must offer the same set.
                match engine::legal_actions_checked(&world, catalog, &mut legal) {
                    Ok(actions) if same_action_set(actions, &nodes[id].actions) => {}
                    Ok(_) => {
                        note(
                            &mut refusals,
                            format!("{:?}", HiddenRefusal::ActionSetDependsOnHiddenState),
                        );
                        aborted = true;
                        break;
                    }
                    Err(error) => {
                        note(&mut refusals, format!("enumeration: {error:?}"));
                        aborted = true;
                        break;
                    }
                }
            }
            let node = &nodes[id];
            let open: Vec<usize> = (0..node.edges.len())
                .filter(|&e| !node.edges[e].refused)
                .collect();
            if open.is_empty() {
                note(&mut refusals, "every action refused".to_owned());
                aborted = true;
                break;
            }
            let untried: Vec<usize> = open
                .iter()
                .copied()
                .filter(|&e| node.edges[e].visits == 0)
                .collect();
            let chosen = if untried.is_empty() {
                let log_n = (node.visits.max(1) as f64).ln();
                *open
                    .iter()
                    .max_by(|&&a, &&b| {
                        let ucb = |e: usize| {
                            let edge = &node.edges[e];
                            edge.value_sum / edge.visits as f64
                                + params.exploration * (log_n / edge.visits as f64).sqrt()
                        };
                        ucb(a).total_cmp(&ucb(b))
                    })
                    .expect("open is nonempty")
            } else {
                untried[rng.below(untried.len())]
            };
            let action = node.actions[chosen];
            transitions += 1;
            // Offered by this world: a fresh node enumerated it here, a hidden
            // revisit checked the world offers the node's set, and a
            // clairvoyant node is keyed by the whole state.
            let next =
                match engine::apply_offered_action_into(&world, catalog, &action, &mut events) {
                    Ok(next) => next,
                    Err(error) => {
                        note(&mut refusals, format!("transition: {error:?}"));
                        nodes[id].edges[chosen].refused = true;
                        aborted = true;
                        break;
                    }
                };
            let next_know = match params.information {
                Information::Hidden => {
                    match hidden::observe_step(catalog, &world, &world_know, &next, &events) {
                        Ok(next_know) => next_know,
                        Err(refusal) => {
                            note(&mut refusals, format!("{refusal:?}"));
                            nodes[id].edges[chosen].refused = true;
                            aborted = true;
                            break;
                        }
                    }
                }
                Information::Clairvoyant => DrawKnowledge::none(),
            };
            // A receipt carried over from the parent keeps its origin; a new
            // one was minted from the parent, whose node is the origin.
            origin = if params.information == Information::Hidden
                && hidden::has_replay_receipt(&next)
            {
                let carried = origin.is_some()
                    && hidden::has_replay_receipt(&world)
                    && hidden::receipt_predecessor(&world) == hidden::receipt_predecessor(&next);
                if carried { origin } else { Some(id) }
            } else {
                None
            };
            path.push((id, chosen));
            line.push(action);
            world = next;
            world_know = next_know;
            depth += 1;
            rep = match representative(params.information, &world, &world_know, catalog, origin) {
                Ok(rep) => rep,
                Err(refusal) => {
                    note(&mut refusals, format!("{refusal:?}"));
                    nodes[id].edges[chosen].refused = true;
                    aborted = true;
                    break;
                }
            };
            hash = fast_hash(&rep, &world_know);
        }
        // Rollout.
        if !aborted {
            while !stopped(&world, depth, params.horizon) {
                let actions = match engine::legal_actions_checked(&world, catalog, &mut legal) {
                    Ok(actions) if !actions.is_empty() => actions,
                    Ok(_) => break,
                    Err(error) => {
                        note(&mut refusals, format!("rollout enumeration: {error:?}"));
                        aborted = true;
                        break;
                    }
                };
                let action = rollout_action(actions, params.end_weight, &mut rng);
                transitions += 1;
                // Rollouts never return to a predecessor: step in place.
                let stepped = engine::apply_offered_action_owned(
                    world,
                    catalog,
                    &action,
                    &mut events,
                    || replay(&start, catalog, &line),
                );
                match stepped {
                    Ok(next) => world = next,
                    Err(error) => {
                        note(&mut refusals, format!("rollout transition: {error:?}"));
                        // The refused step consumed the world; an aborted
                        // iteration never reads it again.
                        world = start.clone();
                        aborted = true;
                        break;
                    }
                }
                line.push(action);
                depth += 1;
            }
        }
        // A refused iteration scores the worst reward, as `search.rs` does:
        // it steers the tree away without claiming an outcome.
        let (value, win, final_hp) = if aborted {
            (-1.0, false, 0.0)
        } else {
            let win = won(&world);
            (
                reward(&world, initial_enemy_hp, battleworn),
                win,
                if win { f64::from(world.hp) } else { 0.0 },
            )
        };
        for &(id, e) in &path {
            let node = &mut nodes[id];
            node.visits += 1;
            let edge = &mut node.edges[e];
            edge.visits += 1;
            edge.value_sum += value;
            edge.value_sq += value * value;
            edge.wins += u64::from(win);
            edge.final_hp_sum += final_hp;
        }
    }
    let root = index
        .find(&nodes, root_hash, &root_rep, know)
        .map_or_else(Vec::new, |id| {
            let node = &nodes[id];
            node.actions
                .iter()
                .zip(&node.edges)
                .map(|(action, edge)| RootSums {
                    action: *action,
                    visits: edge.visits,
                    value_sum: edge.value_sum,
                    value_sq: edge.value_sq,
                    wins: edge.wins,
                    final_hp_sum: edge.final_hp_sum,
                })
                .collect()
        });
    Ok(TreeOutcome {
        root,
        iterations,
        transitions,
        nodes: nodes.len(),
        receipt_nodes: nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.rep,
                    NodeIdentity::Document(_) | NodeIdentity::Receipt { .. }
                )
            })
            .count(),
        refusals,
    })
}

#[cfg(test)]
#[path = "ismcts_witnesses.rs"]
mod witnesses;

#[cfg(test)]
mod tests {
    use super::{RootMerge, RootSums, same_action_set};
    use crate::engine::Action;
    use crate::engine::SelectionAnswer;

    /// `RootMerge` sums exactly as the pairwise scan it replaced, row for
    /// row and in the same first-seen order, over trees that list the same
    /// actions in different orders and some actions not at all.
    #[test]
    fn root_merge_matches_the_pairwise_scan() {
        let tree = |offset: u32, step: u32, n: u32| -> Vec<RootSums> {
            (0..n)
                .map(|i| {
                    let action = select((i * step + offset) % 5_000);
                    RootSums {
                        action,
                        visits: u64::from(i % 7),
                        value_sum: f64::from(i) * 0.25,
                        value_sq: f64::from(i % 11),
                        wins: u64::from(i % 3),
                        final_hp_sum: f64::from(i % 13),
                    }
                })
                .collect()
        };
        let trees = [tree(0, 1, 5_000), tree(17, 3, 5_000), tree(4_999, 7, 2_000)];
        let mut scan: Vec<RootSums> = Vec::new();
        let mut merge = RootMerge::default();
        for rows in &trees {
            for sums in rows {
                match scan.iter_mut().find(|row| row.action == sums.action) {
                    Some(row) => {
                        row.visits += sums.visits;
                        row.value_sum += sums.value_sum;
                        row.value_sq += sums.value_sq;
                        row.wins += sums.wins;
                        row.final_hp_sum += sums.final_hp_sum;
                    }
                    None => scan.push(sums.clone()),
                }
                merge.add(sums.clone());
            }
        }
        let key = |row: &RootSums| {
            (
                row.action,
                row.visits,
                row.value_sum.to_bits(),
                row.value_sq.to_bits(),
                row.wins,
                row.final_hp_sum.to_bits(),
            )
        };
        assert_eq!(
            merge.rows.iter().map(key).collect::<Vec<_>>(),
            scan.iter().map(key).collect::<Vec<_>>()
        );
    }

    fn select(index: u32) -> Action {
        Action::Select {
            answer: SelectionAnswer::OptionIndex(index),
        }
    }

    /// Both arms (pairwise below the cutoff, counting above it) compare
    /// multisets in any order, and the large arm stays linear: 20,000
    /// options reversed is one pass, not 400M comparisons.
    #[test]
    fn same_action_set_compares_multisets_at_every_size() {
        for n in [0_u32, 1, 5, 32, 33, 13_700, 20_000] {
            let a: Vec<Action> = (0..n).map(select).collect();
            let mut reversed = a.clone();
            reversed.reverse();
            assert!(same_action_set(&a, &a), "{n}: identical");
            assert!(same_action_set(&a, &reversed), "{n}: permuted");
            if n >= 2 {
                let mut swapped = reversed.clone();
                swapped[0] = select(n);
                assert!(!same_action_set(&a, &swapped), "{n}: one action differs");
                let mut doubled = reversed.clone();
                doubled[0] = doubled[1];
                assert!(!same_action_set(&a, &doubled), "{n}: a repeat replaces one");
                assert!(!same_action_set(&doubled, &a), "{n}: a repeat, either side");
            }
            assert!(!same_action_set(&a, &a[..a.len().saturating_sub(1)]) || n == 0);
        }
    }
}
