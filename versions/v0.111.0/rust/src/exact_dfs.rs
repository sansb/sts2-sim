//! Deterministic, single-threaded exact DFS for the solo-v1 Rust authority.
//!
//! This is a library boundary, deliberately not a product command.  It uses
//! the hot engine for transitions, but takes a canonical document at its
//! public authority entry so solo policy, canonical/hot conversion, and the
//! engine admission walk all happen before any search work.  A future product
//! cutover and its versioned serialization boundary must remain an explicit
//! separate decision; these Rust library types are not a wire protocol.
//!
//! The first memo key is intentionally conservative: canonical projection
//! bytes plus the requested horizon.  Projection is much slower than a
//! purpose-built hot hash, but it contains every semantically observable
//! state field and makes incorrect state merging impossible by construction.
//! Optimize this only after a measured corpus says it matters.

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::boundary::{BoundaryRefusal, HotBoundary};
use crate::canonical::CanonicalStateV2;
use crate::catalog::Catalog;
use crate::engine::{
    self, Action, AdmissionRefusal, BattlewornObjective, EngineRefusal, LegalActionBuffer,
};
use crate::hot::HotState;
use crate::ids::RelicId;
use crate::solo_v1::{SoloV1Refusal, admit_exact_solve};

/// The exact-solver objective: win, event reward kept, final HP, potions
/// kept, fewer turns.
///
/// `Ord` is deliberately the authority comparison: a winning state is always
/// greater than a loss; among wins, one that keeps its event reward beats one
/// that forfeits it at any HP (#3369); then larger HP and potion count are
/// better, while `negative_turns` gives fewer turns a larger value.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ExactObjective {
    /// Whether every monster is dead while the player survived.
    pub won: bool,
    /// Whether a win kept its event reward. It is true for every win except a
    /// Battleworn Dummy timeout ([`crate::engine::BattlewornObjective`]),
    /// where the dummy escapes and the event pays nothing, and false for a
    /// loss. So a Battleworn kill outranks a Battleworn timeout at any HP,
    /// and every other fight orders exactly as before.
    pub event_reward: bool,
    /// Final player HP. Losses use the all-zero Python sentinel.
    pub final_hp: i32,
    /// Occupied potion slots at the terminal state.
    pub potions_kept: u8,
    /// Negated terminal turn, so fewer turns wins a lexicographic tie.
    pub negative_turns: i16,
}

impl ExactObjective {
    /// Python's `LOSS = (0, 0, 0, 0)`.
    pub const LOSS: Self = Self {
        won: false,
        event_reward: false,
        final_hp: 0,
        potions_kept: 0,
        negative_turns: 0,
    };
}

/// Parameters that affect a deterministic exact DFS invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExactDfsConfig {
    /// Python-compatible inclusive turn horizon. States beyond it lose.
    pub max_turns: i16,
    /// Optional wall-clock cap for the complete public invocation. Expiry
    /// returns an achieved, incomplete result.
    pub deadline: Option<Duration>,
    /// Enable the correctness-first canonical memo table.
    pub memo: bool,
}

impl Default for ExactDfsConfig {
    fn default() -> Self {
        Self {
            max_turns: 12,
            deadline: None,
            memo: true,
        }
    }
}

/// Cooperative cancellation shared by the caller and a running DFS.
#[derive(Clone)]
pub struct ExactCancellation {
    requested: Arc<AtomicBool>,
    /// A deterministic testing/review hook. Normal callers leave this at
    /// `u64::MAX` and use `cancel`; a bounded cooperative caller can prove
    /// incomplete-result behaviour without relying on scheduler timing.
    nodes_remaining: Arc<AtomicU64>,
}

impl Default for ExactCancellation {
    fn default() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            nodes_remaining: Arc::new(AtomicU64::new(u64::MAX)),
        }
    }
}

impl ExactCancellation {
    /// Ask a running search to stop at its next node boundary.
    pub fn cancel(&self) {
        self.requested.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }

    /// Request cancellation at the node boundary after `nodes` visits.
    ///
    /// This is primarily a deterministic review/test hook for the contract
    /// that cancellation is incomplete even after a real win was found; it
    /// never changes a returned line's replay requirements.
    pub fn cancel_after_nodes(&self, nodes: u64) {
        self.nodes_remaining.store(nodes, Ordering::Relaxed);
    }

    fn observe_node(&self) {
        let remaining = self.nodes_remaining.load(Ordering::Relaxed);
        if remaining == u64::MAX {
            return;
        }
        if remaining == 0 || self.nodes_remaining.fetch_sub(1, Ordering::Relaxed) <= 1 {
            self.cancel();
        }
    }
}

/// Whether a returned answer is proven exact, incomplete, or refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactDfsStatus {
    /// Every reachable decision within the requested horizon was searched.
    Exact,
    /// Wall time expired; `best` is achieved but not proven optimal.
    Deadline,
    /// The caller cancelled; `best` is achieved but not proven optimal.
    Cancelled,
    /// Policy, boundary, admission, or a transition refused before completion.
    Refused,
}

impl ExactDfsStatus {
    /// A deadline/cancelled result must never be presented as exact.
    #[must_use]
    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact)
    }
}

/// Typed reason exact search could not safely start or continue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactDfsRefusal {
    /// The document is outside the frozen solo-v1 authority boundary.
    Solo(SoloV1Refusal),
    /// The canonical document cannot cross into the hot state.
    Boundary(BoundaryRefusal),
    /// The hot engine's whole-fight admission walk refused the content closure.
    Admission(AdmissionRefusal),
    /// A supposedly admitted transition encountered an unmodeled/invalid path.
    Transition(EngineRefusal),
    /// A post-transition state could not safely be projected for the memo key.
    MemoProjection(BoundaryRefusal),
    /// Revisiting an active state would make DFS recursive rather than finite.
    CycleDetected,
    /// An admitted, nonterminal state exposed no legal action to the DFS.
    NoLegalActions,
    /// Python refuses the unported dual-relic victory-score ordering.
    VictoryScoreCombinationNotModeled,
    /// A negative horizon would silently change Python's search semantics.
    InvalidHorizon(i16),
    /// Combat turns are nonnegative; accepting a forged negative value would
    /// make the negated tie-break overflow or claim an impossible ordering.
    InvalidTurn(i16),
    /// A line-less observed HP lower bound cannot be negative.
    InvalidAlphaFloor(i32),
    /// A caller-provided achieved floor did not replay to its advertised
    /// objective/digest, so accepting it would fabricate a deadline result.
    InvalidSeed(String),
}

/// One fully replayable winning line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactSolution {
    /// Rust wire actions, including physical-card UIDs for every play.
    pub actions: Vec<Action>,
    /// The Python-compatible objective achieved by replaying `actions`.
    pub objective: ExactObjective,
    /// Canonical v2 digest of the terminal state.
    pub final_digest: String,
}

/// Search accounting. Counts include work done before an incomplete stop.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExactDfsTelemetry {
    /// Total elapsed monotonic time.
    pub elapsed_ns: u128,
    /// DFS nodes entered, including terminal/horizon leaves.
    pub nodes: u64,
    /// Terminal leaves evaluated.
    pub terminal_nodes: u64,
    /// Nonterminal leaves rejected for exceeding the turn horizon.
    pub horizon_nodes: u64,
    /// Engine transitions attempted.
    pub transitions: u64,
    /// Canonical-key memo lookups.
    pub memo_probes: u64,
    /// Successful memo lookups.
    pub memo_hits: u64,
    /// Memo lookups without a completed entry.
    pub memo_misses: u64,
    /// Number of completed subtrees retained in the memo.
    pub memo_entries: usize,
    /// Current-thread allocations during the complete solve invocation when
    /// the allocation-counting feature is on.
    pub allocations: u64,
    /// Current-thread requested allocation bytes during the complete solve
    /// invocation when instrumented.
    pub allocated_bytes: u64,
    /// Whether allocation counters were instrumented for this binary.
    pub allocation_instrumented: bool,
    /// Highest independently achieved/observed final-HP floor supplied to
    /// this solve. It is telemetry only until a certified ceiling can use it
    /// for strict-below pruning.
    pub alpha_final_hp: i32,
}

/// The library result. `best` is absent when no winning terminal was reached.
///
/// Under `Deadline` and `Cancelled`, a present `best` is always a fully
/// replayed terminal win; this API never reports a partially explored prefix
/// as an achieved line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactDfsResult {
    pub status: ExactDfsStatus,
    pub best: Option<ExactSolution>,
    pub refusal: Option<ExactDfsRefusal>,
    pub telemetry: ExactDfsTelemetry,
}

#[derive(Clone, Debug)]
struct NodeValue {
    objective: ExactObjective,
    line: Option<Arc<LineNode>>,
    final_digest: Option<Arc<str>>,
}

#[derive(Debug)]
struct LineNode {
    action: Action,
    next: Option<Arc<LineNode>>,
}

impl NodeValue {
    const LOSS: Self = Self {
        objective: ExactObjective::LOSS,
        line: None,
        final_digest: None,
    };

    fn solution_with_prefix(&self, prefix: &[Action]) -> Option<ExactSolution> {
        self.objective.won.then(|| {
            // Memo entries share immutable tails, retaining one node per
            // chosen edge rather than a copied Vec suffix per state.
            let mut actions = Vec::with_capacity(prefix.len());
            actions.extend_from_slice(prefix);
            let mut node = self.line.as_deref();
            while let Some(current) = node {
                actions.push(current.action);
                node = current.next.as_deref();
            }
            ExactSolution {
                actions,
                objective: self.objective,
                final_digest: self
                    .final_digest
                    .as_deref()
                    .expect("winning DFS values always carry terminal evidence")
                    .to_owned(),
            }
        })
    }
}

enum VisitFailure {
    Deadline,
    Cancelled,
    Refusal(ExactDfsRefusal),
}

struct Dfs<'a> {
    catalog: &'a Catalog,
    config: ExactDfsConfig,
    cancellation: &'a ExactCancellation,
    deadline: Option<(Instant, Duration)>,
    legal_actions: LegalActionBuffer,
    events: Vec<engine::Event>,
    memo: HashMap<Vec<u8>, NodeValue>,
    active: HashSet<Vec<u8>>,
    path: Vec<Action>,
    best_achieved: Option<ExactSolution>,
    /// Constructive seed/observed HP floor.  It never supplies a line by
    /// itself; a future certified ceiling may safely prune only when strictly
    /// below it, preserving equal-HP potion/turn ties. Only a win that keeps
    /// its event reward raises it: a Battleworn timeout's HP is no floor for
    /// the kills that outrank it (#3369).
    alpha_final_hp: i32,
    /// Whether the root is a Battleworn Dummy fight, read once (#3369).
    battleworn: BattlewornObjective,
    telemetry: ExactDfsTelemetry,
}

impl<'a> Dfs<'a> {
    fn new(
        catalog: &'a Catalog,
        config: ExactDfsConfig,
        cancellation: &'a ExactCancellation,
        started: Instant,
        seed: Option<ExactSolution>,
        alpha_final_hp: i32,
        battleworn: BattlewornObjective,
    ) -> Self {
        let alpha_final_hp = alpha_final_hp.max(
            seed.as_ref()
                .filter(|solution| solution.objective.won && solution.objective.event_reward)
                .map_or(0, |solution| solution.objective.final_hp),
        );
        Self {
            catalog,
            // Compare elapsed time instead of constructing an absolute future
            // Instant: Duration::MAX must not overflow into "no deadline".
            // `started` comes from the public entry, so boundary/admission
            // time counts against the caller's cap too.
            deadline: config.deadline.map(|duration| (started, duration)),
            config,
            cancellation,
            legal_actions: LegalActionBuffer::new(),
            events: Vec::new(),
            memo: HashMap::new(),
            active: HashSet::new(),
            path: Vec::new(),
            alpha_final_hp,
            battleworn,
            best_achieved: seed,
            telemetry: ExactDfsTelemetry {
                allocation_instrumented: crate::allocation::instrumented(),
                alpha_final_hp,
                ..ExactDfsTelemetry::default()
            },
        }
    }

    fn visit(&mut self, state: &HotState) -> Result<NodeValue, VisitFailure> {
        self.telemetry.nodes += 1;
        self.cancellation.observe_node();
        if self.cancellation.is_cancelled() {
            return Err(VisitFailure::Cancelled);
        }
        if self
            .deadline
            .is_some_and(|(started, duration)| started.elapsed() >= duration)
        {
            return Err(VisitFailure::Deadline);
        }

        if let Some(value) = self.terminal_value(state)? {
            return Ok(value);
        }
        if state.turn > self.config.max_turns {
            self.telemetry.horizon_nodes += 1;
            return Ok(NodeValue::LOSS);
        }

        let key = self.memo_key(state)?;
        if self.config.memo {
            self.telemetry.memo_probes += 1;
            if let Some(value) = self.memo.get(&key) {
                self.telemetry.memo_hits += 1;
                return Ok(value.clone());
            }
            self.telemetry.memo_misses += 1;
        }
        if !self.active.insert(key.clone()) {
            return Err(VisitFailure::Refusal(ExactDfsRefusal::CycleDetected));
        }

        let actions =
            engine::legal_actions_into(state, self.catalog, &mut self.legal_actions).to_vec();
        if actions.is_empty() {
            self.active.remove(&key);
            return Err(VisitFailure::Refusal(ExactDfsRefusal::NoLegalActions));
        }
        let mut best = NodeValue::LOSS;
        let mut best_presentation = None;
        for (order, action) in actions.into_iter().enumerate() {
            self.telemetry.transitions += 1;
            let child = engine::apply_action_into(state, self.catalog, &action, &mut self.events)
                .map_err(|refusal| {
                VisitFailure::Refusal(ExactDfsRefusal::Transition(refusal))
            })?;
            self.path.push(action);
            let child_value = self.visit(&child);
            self.path.pop();
            let child_value = child_value?;
            let candidate = NodeValue {
                objective: child_value.objective,
                line: child_value.objective.won.then(|| {
                    Arc::new(LineNode {
                        action,
                        next: child_value.line,
                    })
                }),
                final_digest: child_value.final_digest,
            };
            let presentation = self.presentation_key(state, &action, &child, order);
            if candidate.objective > best.objective
                || candidate.objective == best.objective
                    && best_presentation.is_none_or(|current| presentation < current)
            {
                best = candidate;
                best_presentation = Some(presentation);
            }
        }
        self.active.remove(&key);
        if self.config.memo {
            self.memo.insert(key, best.clone());
        }
        Ok(best)
    }

    /// Current v0.111.0 authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
    /// Meat on the Bone's 50/12 vars are at RVA `0x96aaa`, its inclusive
    /// decimal threshold is at RVA `0x96b74`, and the early-victory body at
    /// RVA `0x32a748` requires a living owner before applying the heal.
    fn terminal_value(&mut self, state: &HotState) -> Result<Option<NodeValue>, VisitFailure> {
        if !state.history.over {
            return Ok(None);
        }
        self.telemetry.terminal_nodes += 1;
        if state.hp <= 0 || state.monsters.iter().any(|monster| monster.hp > 0) {
            return Ok(Some(NodeValue::LOSS));
        }
        let (objective, final_digest) = terminal_objective(state, self.catalog, self.battleworn)
            .map_err(VisitFailure::Refusal)?
            .expect("winning terminal state has an objective");
        let value = NodeValue {
            objective,
            line: None,
            final_digest: Some(Arc::from(final_digest)),
        };
        if let Some(solution) = value.solution_with_prefix(&self.path)
            && self
                .best_achieved
                .as_ref()
                .is_none_or(|best| solution.objective > best.objective)
        {
            self.best_achieved = Some(solution);
        }
        if objective.event_reward {
            self.alpha_final_hp = self.alpha_final_hp.max(objective.final_hp);
            self.telemetry.alpha_final_hp = self.alpha_final_hp;
        }
        Ok(Some(value))
    }

    fn memo_key(&self, state: &HotState) -> Result<Vec<u8>, VisitFailure> {
        let document = HotBoundary::try_to_canonical(state, self.catalog)
            .map_err(|refusal| VisitFailure::Refusal(ExactDfsRefusal::MemoProjection(refusal)))?;
        let canonical = document.canonical_json();
        let mut key = Vec::with_capacity(canonical.len() + 2);
        key.extend_from_slice(&self.config.max_turns.to_be_bytes());
        key.extend_from_slice(canonical.as_bytes());
        Ok(key)
    }

    fn presentation_key(
        &self,
        state: &HotState,
        action: &Action,
        child: &HotState,
        order: usize,
    ) -> (u8, i64, usize) {
        let rank = match action {
            Action::EndTurn => 4,
            Action::UsePotion { .. } => 3,
            Action::Select { .. } => 0,
            Action::Play { uid, .. } => state
                .piles
                .get(crate::hot::PileId::Hand)
                .as_slice()
                .iter()
                .find(|card| card.uid == *uid)
                .and_then(|card| self.catalog.spec(card.atom))
                .map(|spec| {
                    let setup = spec.is_power
                        || self.catalog.steps(spec).iter().any(|step| {
                            matches!(
                                step.kind,
                                crate::ids::StepKind::Vulnerable | crate::ids::StepKind::Weak
                            )
                        });
                    if setup {
                        0
                    } else if spec.is_attack {
                        2
                    } else {
                        1
                    }
                })
                .unwrap_or(5),
        };
        let enemy_hp = child
            .monsters
            .iter()
            .filter(|monster| monster.hp > 0)
            .map(|monster| i64::from(monster.hp))
            .sum();
        (rank, enemy_hp, order)
    }
}

/// Solve a canonical solo-v1 state through all authority gates.
#[must_use]
pub fn solve(
    document: &CanonicalStateV2,
    config: ExactDfsConfig,
    cancellation: &ExactCancellation,
) -> ExactDfsResult {
    solve_seeded(document, config, cancellation, None, 0)
}

/// Solve with an already achieved, fully replayable floor.
///
/// This is the Rust equivalent of Python's paired `seed_score`/`seed_line`:
/// the line is replayed through the normal engine and must reproduce its
/// objective and canonical digest before it can survive a deadline.  It is
/// an incumbent, never an unverified alpha claim.
#[must_use]
pub fn solve_seeded(
    document: &CanonicalStateV2,
    config: ExactDfsConfig,
    cancellation: &ExactCancellation,
    seed: Option<&ExactSolution>,
    alpha_final_hp: i32,
) -> ExactDfsResult {
    let started = Instant::now();
    let (allocations_before, bytes_before) = crate::allocation::thread_snapshot();
    if let Err(refusal) = admit_exact_solve(document) {
        return refused(
            started,
            allocations_before,
            bytes_before,
            ExactDfsRefusal::Solo(refusal),
        );
    }
    let catalog = match HotBoundary::catalog_from_canonical(document) {
        Ok(catalog) => catalog,
        Err(refusal) => {
            return refused(
                started,
                allocations_before,
                bytes_before,
                ExactDfsRefusal::Boundary(refusal),
            );
        }
    };
    let root = match HotBoundary::from_canonical(document, &catalog) {
        Ok(state) => state,
        Err(refusal) => {
            return refused(
                started,
                allocations_before,
                bytes_before,
                ExactDfsRefusal::Boundary(refusal),
            );
        }
    };
    if root.turn < 0 {
        return refused(
            started,
            allocations_before,
            bytes_before,
            ExactDfsRefusal::InvalidTurn(root.turn),
        );
    }
    if let Err(refusal) = engine::admit(document, &root, &catalog) {
        return refused(
            started,
            allocations_before,
            bytes_before,
            ExactDfsRefusal::Admission(refusal),
        );
    }
    let seed = match seed {
        Some(seed) => match validate_seed(&root, &catalog, seed) {
            Ok(seed) => Some(seed),
            Err(refusal) => {
                return refused(started, allocations_before, bytes_before, refusal);
            }
        },
        None => None,
    };
    if alpha_final_hp < 0 {
        return refused(
            started,
            allocations_before,
            bytes_before,
            ExactDfsRefusal::InvalidAlphaFloor(alpha_final_hp),
        );
    }
    solve_hot_inner(
        root,
        &catalog,
        SolveInvocation {
            config,
            cancellation,
            started,
            allocations_before,
            bytes_before,
            seed,
            alpha_final_hp,
        },
    )
}

struct SolveInvocation<'a> {
    config: ExactDfsConfig,
    cancellation: &'a ExactCancellation,
    started: Instant,
    allocations_before: u64,
    bytes_before: u64,
    seed: Option<ExactSolution>,
    alpha_final_hp: i32,
}

fn solve_hot_inner(
    root: HotState,
    catalog: &Catalog,
    invocation: SolveInvocation<'_>,
) -> ExactDfsResult {
    if invocation.config.max_turns < 0 {
        return refused(
            invocation.started,
            invocation.allocations_before,
            invocation.bytes_before,
            ExactDfsRefusal::InvalidHorizon(invocation.config.max_turns),
        );
    }
    let mut dfs = Dfs::new(
        catalog,
        invocation.config,
        invocation.cancellation,
        invocation.started,
        invocation.seed,
        invocation.alpha_final_hp,
        BattlewornObjective::of_root(&root),
    );
    let outcome = dfs.visit(&root);
    dfs.telemetry.memo_entries = dfs.memo.len();
    // A caller's seed is an achieved fallback for an *incomplete* search,
    // not an extra branch within this invocation's horizon.  In particular,
    // a valid turn-4 seed must not turn an exact max-turn-0 loss into an
    // exact win.  The complete result therefore comes only from `outcome`;
    // `best_achieved` (which includes the seed) is retained for deadline/
    // cancellation paths below.
    let (status, best) = match outcome {
        Ok(value) => (ExactDfsStatus::Exact, value.solution_with_prefix(&[])),
        Err(VisitFailure::Deadline) => (ExactDfsStatus::Deadline, dfs.best_achieved),
        Err(VisitFailure::Cancelled) => (ExactDfsStatus::Cancelled, dfs.best_achieved),
        Err(VisitFailure::Refusal(refusal)) => {
            return finish(
                ExactDfsStatus::Refused,
                None,
                Some(refusal),
                dfs.telemetry,
                invocation.started,
                invocation.allocations_before,
                invocation.bytes_before,
            );
        }
    };
    finish(
        status,
        best,
        None,
        dfs.telemetry,
        invocation.started,
        invocation.allocations_before,
        invocation.bytes_before,
    )
}

fn validate_seed(
    root: &HotState,
    catalog: &Catalog,
    seed: &ExactSolution,
) -> Result<ExactSolution, ExactDfsRefusal> {
    if !seed.objective.won {
        return Err(ExactDfsRefusal::InvalidSeed(
            "seed objective must be a win".to_owned(),
        ));
    }
    let mut state = root.clone();
    let mut events = Vec::new();
    for action in &seed.actions {
        state =
            engine::apply_action_into(&state, catalog, action, &mut events).map_err(|error| {
                ExactDfsRefusal::InvalidSeed(format!("seed action did not replay: {error}"))
            })?;
    }
    let Some((objective, final_digest)) =
        terminal_objective(&state, catalog, BattlewornObjective::of_root(root))?
    else {
        return Err(ExactDfsRefusal::InvalidSeed(
            "seed line did not reach a winning terminal state".to_owned(),
        ));
    };
    // The v1 wire objective has no `event_reward` slot, so a seed read from
    // it cannot claim one: compare the wire-visible fields and keep the
    // replayed tier (#3369).
    let visible = |o: &ExactObjective| (o.won, o.final_hp, o.potions_kept, o.negative_turns);
    if visible(&objective) != visible(&seed.objective) {
        return Err(ExactDfsRefusal::InvalidSeed(format!(
            "seed objective {:?} replayed as {:?}",
            seed.objective, objective
        )));
    }
    if final_digest != seed.final_digest {
        return Err(ExactDfsRefusal::InvalidSeed(
            "seed terminal digest did not replay".to_owned(),
        ));
    }
    Ok(ExactSolution {
        objective,
        ..seed.clone()
    })
}

/// Current v0.111.0 authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// Meat on the Bone's 50/12 vars are at RVA `0x96aaa`, its inclusive decimal
/// threshold is at RVA `0x96b74`, and the early-victory body at RVA `0x32a748`
/// requires a living owner before applying the heal.
///
/// `battleworn` is the root's fight (#3369): a Battleworn Dummy timeout is a
/// win whose `event_reward` is false (see
/// [`crate::engine::BattlewornObjective`]).
fn terminal_objective(
    state: &HotState,
    catalog: &Catalog,
    battleworn: BattlewornObjective,
) -> Result<Option<(ExactObjective, String)>, ExactDfsRefusal> {
    if !state.history.over || state.hp <= 0 || state.monsters.iter().any(|monster| monster.hp > 0) {
        return Ok(None);
    }
    let final_digest = HotBoundary::try_to_canonical(state, catalog)
        .map_err(ExactDfsRefusal::MemoProjection)?
        .differential_digest();
    let potions_kept = state
        .fanouts
        .potion_slots()
        .iter()
        .filter(|slot| slot.is_some())
        .count()
        .try_into()
        .expect("potion belt is bounded by the hot representation");
    let owns_meat = catalog.hooks().owns(RelicId::RelicMeatOnTheBone);
    if owns_meat && catalog.hooks().owns(RelicId::RelicChosenCheese) {
        return Err(ExactDfsRefusal::VictoryScoreCombinationNotModeled);
    }
    // Python's `victory_final_hp`: Meat on the Bone heals after the combat
    // action state becomes terminal, so it belongs in the objective but must
    // not alter the canonical final-state digest.
    let threshold = (i64::from(state.max_hp) * 50 / 100) as i32;
    let final_hp = if owns_meat && state.hp <= threshold {
        state.hp.saturating_add(12).min(state.max_hp)
    } else {
        state.hp
    };
    Ok(Some((
        ExactObjective {
            won: true,
            event_reward: !battleworn.timed_out(state),
            final_hp,
            potions_kept,
            negative_turns: state
                .turn
                .checked_neg()
                .expect("exact-solve entry rejects negative turns"),
        },
        final_digest,
    )))
}

fn refused(
    started: Instant,
    allocations_before: u64,
    bytes_before: u64,
    refusal: ExactDfsRefusal,
) -> ExactDfsResult {
    finish(
        ExactDfsStatus::Refused,
        None,
        Some(refusal),
        ExactDfsTelemetry {
            allocation_instrumented: crate::allocation::instrumented(),
            ..ExactDfsTelemetry::default()
        },
        started,
        allocations_before,
        bytes_before,
    )
}

fn finish(
    status: ExactDfsStatus,
    best: Option<ExactSolution>,
    refusal: Option<ExactDfsRefusal>,
    mut telemetry: ExactDfsTelemetry,
    started: Instant,
    allocations_before: u64,
    bytes_before: u64,
) -> ExactDfsResult {
    let (allocations_after, bytes_after) = crate::allocation::thread_snapshot();
    telemetry.elapsed_ns = started.elapsed().as_nanos();
    telemetry.allocations = allocations_after.saturating_sub(allocations_before);
    telemetry.allocated_bytes = bytes_after.saturating_sub(bytes_before);
    ExactDfsResult {
        status,
        best,
        refusal,
        telemetry,
    }
}
