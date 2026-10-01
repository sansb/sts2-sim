//! Deterministic, single-threaded exact DFS for the solo-v1 Rust authority.
//!
//! This is a library boundary, deliberately not a product command.  It uses
//! the hot engine for transitions, but takes a canonical document at its
//! public authority entry so solo policy, canonical/hot conversion, and the
//! engine admission walk all happen before any search work.  A future product
//! cutover and its versioned serialization boundary must remain an explicit
//! separate decision; these Rust library types are not a wire protocol.
//!
//! The memo key is the SHA-256 of the requested horizon and the state's
//! derived `Hash` ([`MemoKey`], #3522).  `HotState` is the whole mutable
//! state and the engine is deterministic in `(state, catalog)`, so two states
//! share a key only if they are equal or SHA-256 collides.  Until #3522 the
//! digest was over the canonical projection bytes instead; building that
//! projection cost 20-33 us per node, 4-22x a transition, on every #3420 panel
//! fight.  Debug builds still project every node, keep the projection's
//! refusal, and assert that both keys partition the visited states
//! identically.  Until #3470 the key was the
//! projection bytes themselves, which made incorrect merging impossible by
//! construction. That key was measured as the whole of solve memory: on the
//! three heaviest certified-fixture exact solves, the memo took 225-424 MB,
//! against 6.8-12.5 MB hashed. Solutions and node counts were identical, and
//! solve time was 6-17% higher. The browser engine cannot carry the unhashed
//! memo. The trade was
//! Sean's (2026-09-29): a false merge now needs a SHA-256 collision, about
//! `n^2 / 2^257` for `n` memoized states, so about 1e-59 even at a billion
//! entries. That is far below the rate of undetected hardware error.

#[cfg(target_arch = "wasm32")]
use crate::wasm_clock::Instant;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// Optional cap on the memo table's accounted bytes (#3470). Past it, no
    /// further subtree is memoized, and the search continues without them.
    /// The memo is a pure cache, so a capped solve returns exactly the result
    /// an uncapped one would. It costs only recomputation, which the
    /// deadline bounds, and it can cost a lot: memoization is post-order, so
    /// a full memo keeps deep subtrees and refuses the shallow ones worth
    /// the most. Solve memory is essentially the memo (a deadline solve with
    /// full-projection keys measured 777 MB peak RSS with memo on and 6.5 MB
    /// with it off), so this is the backstop that keeps a browser solve
    /// inside wasm32's 4 GB (and a phone tab's much smaller) ceiling. Hashed
    /// keys ([`MemoKey`]) are what keep it from binding. `None` is unlimited,
    /// the prior behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo_budget_bytes: Option<u64>,
}

impl Default for ExactDfsConfig {
    fn default() -> Self {
        Self {
            max_turns: 12,
            deadline: None,
            memo: true,
            memo_budget_bytes: None,
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
    /// The memo's accounted bytes at stop (see [`memo_entry_bytes`]).
    pub memo_bytes: u64,
    /// Whether `memo_budget_bytes` stopped at least one memo insert. The
    /// result is still exact when `status` says so; see
    /// [`ExactDfsConfig::memo_budget_bytes`].
    pub memo_budget_reached: bool,
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
}

/// A winning line the search achieved, before its terminal digest is known.
///
/// The search carries no digests (#3420). Projecting every kept terminal
/// state was 40-66% of search time on fights whose search finds wins, and
/// every such projection was of a distinct state. The digest never orders
/// values (`visit` compares objective, then presentation), and it is a
/// function of the root and the line, because the engine is deterministic and
/// the search produced the line by applying exactly those actions. So
/// [`certify`] replays the one reported line and projects its final state
/// once. A caller's seed arrives already certified.
struct Achieved {
    actions: Vec<Action>,
    objective: ExactObjective,
    final_digest: Option<String>,
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
    };

    fn achieved_with_prefix(&self, prefix: &[Action]) -> Option<Achieved> {
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
            Achieved {
                actions,
                objective: self.objective,
                final_digest: None,
            }
        })
    }
}

/// The memo key: SHA-256 over the horizon and the state's derived `Hash` (see
/// the module docs for why a digest, and why of the hot state).
type MemoKey = [u8; 32];

/// SHA-256 over the horizon and `state`'s derived `Hash` (#3522).
fn structural_memo_key(max_turns: i16, state: &HotState) -> MemoKey {
    use std::hash::{Hash, Hasher};
    let mut writer = Sha256Writer::new();
    writer.write(&max_turns.to_be_bytes());
    state.hash(&mut writer);
    writer.finish_key()
}

/// Streams a value's derived `Hash` into SHA-256 through a staging buffer, so
/// each of the many small field writes is a copy rather than a compression
/// call. Integers hash in native byte order and `usize` at native width, so a
/// key is stable within one process, which is all a per-solve memo needs.
struct Sha256Writer {
    digest: Sha256,
    staged: [u8; 256],
    len: usize,
}

impl Sha256Writer {
    fn new() -> Self {
        Self {
            digest: Sha256::new(),
            staged: [0; 256],
            len: 0,
        }
    }

    fn finish_key(mut self) -> MemoKey {
        self.digest.update(&self.staged[..self.len]);
        self.digest.finalize().into()
    }
}

impl std::hash::Hasher for Sha256Writer {
    fn write(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > self.staged.len() {
            self.digest.update(&self.staged[..self.len]);
            self.len = 0;
            if bytes.len() > self.staged.len() {
                self.digest.update(bytes);
                return;
            }
        }
        self.staged[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    /// Never called: derived `Hash` impls only write. The key is the full
    /// 256-bit digest from [`Sha256Writer::finish_key`].
    fn finish(&self) -> u64 {
        unreachable!("Sha256Writer yields its key through finish_key")
    }
}

/// Heap bytes one memo entry owns beyond its table slot. The key is inline
/// in the slot, so this is only the one [`LineNode`] allocation a win's line
/// adds, counted with an `Arc` header. Line tails are shared with other
/// entries and are not charged again. Allocator overhead
/// is not counted, so the accounting is a lower bound on real memory.
fn memo_entry_bytes(value: &NodeValue) -> u64 {
    if value.line.is_some() {
        (std::mem::size_of::<LineNode>() + 2 * std::mem::size_of::<usize>()) as u64
    } else {
        0
    }
}

/// Bytes of a memo table with `capacity` usable slots. The std `HashMap`
/// keeps at most 7/8 of its buckets full, and each bucket holds one
/// `(key, value)` pair plus one control byte.
fn memo_table_bytes(capacity: usize) -> u64 {
    if capacity == 0 {
        return 0;
    }
    let buckets = (capacity.saturating_mul(8) / 7)
        .max(capacity + 1)
        .next_power_of_two();
    let slot = std::mem::size_of::<(MemoKey, NodeValue)>() + 1;
    (buckets as u64).saturating_mul(slot as u64)
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
    memo: HashMap<MemoKey, NodeValue>,
    /// Heap bytes owned by memo entries (their line nodes), excluding the
    /// table; see [`memo_entry_bytes`].
    memo_heap_bytes: u64,
    active: HashSet<MemoKey>,
    /// Debug-only cross-check of the structural key against the canonical
    /// projection key it replaced (#3522): each structural key must name one
    /// projection, and each projection one structural key.
    #[cfg(debug_assertions)]
    projection_of: HashMap<MemoKey, MemoKey>,
    #[cfg(debug_assertions)]
    structure_of: HashMap<MemoKey, MemoKey>,
    path: Vec<Action>,
    best_achieved: Option<Achieved>,
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
            memo_heap_bytes: 0,
            active: HashSet::new(),
            #[cfg(debug_assertions)]
            projection_of: HashMap::new(),
            #[cfg(debug_assertions)]
            structure_of: HashMap::new(),
            path: Vec::new(),
            alpha_final_hp,
            battleworn,
            best_achieved: seed.map(|seed| Achieved {
                actions: seed.actions,
                objective: seed.objective,
                final_digest: Some(seed.final_digest),
            }),
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
        if !self.active.insert(key) {
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
            self.memoize(key, &best);
        }
        Ok(best)
    }

    /// Insert a completed subtree unless it would take the memo past its
    /// budget. Skipping is always sound: the memo only ever returns a value
    /// the search would recompute identically.
    fn memoize(&mut self, key: MemoKey, value: &NodeValue) {
        let entry = memo_entry_bytes(value);
        let table = if self.memo.len() == self.memo.capacity() {
            // This insert rehashes into a table about twice the size; charge
            // the table that will exist, not the one that does.
            memo_table_bytes(self.memo.capacity().saturating_mul(2).max(3))
        } else {
            memo_table_bytes(self.memo.capacity())
        };
        let projected = self
            .memo_heap_bytes
            .saturating_add(entry)
            .saturating_add(table);
        if self
            .config
            .memo_budget_bytes
            .is_some_and(|budget| projected > budget)
        {
            self.telemetry.memo_budget_reached = true;
            return;
        }
        self.memo_heap_bytes += entry;
        self.memo.insert(key, value.clone());
    }

    /// The accounted memo size: entry heap plus the table itself.
    fn memo_bytes(&self) -> u64 {
        self.memo_heap_bytes
            .saturating_add(memo_table_bytes(self.memo.capacity()))
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
        let objective = terminal_objective(state, self.catalog, self.battleworn)
            .map_err(VisitFailure::Refusal)?
            .expect("winning terminal state has an objective");
        // Debug builds project every winning leaf as before, so the test suite
        // keeps an unchosen leaf's projection refusal (#3420).
        #[cfg(debug_assertions)]
        terminal_digest(state, self.catalog).map_err(VisitFailure::Refusal)?;
        let value = NodeValue {
            objective,
            line: None,
        };
        if self
            .best_achieved
            .as_ref()
            .is_none_or(|best| objective > best.objective)
        {
            self.best_achieved = value.achieved_with_prefix(&self.path);
        }
        if objective.event_reward {
            self.alpha_final_hp = self.alpha_final_hp.max(objective.final_hp);
            self.telemetry.alpha_final_hp = self.alpha_final_hp;
        }
        Ok(Some(value))
    }

    fn memo_key(&mut self, state: &HotState) -> Result<MemoKey, VisitFailure> {
        let key = structural_memo_key(self.config.max_turns, state);
        #[cfg(debug_assertions)]
        self.check_key_partition(state, key)?;
        Ok(key)
    }

    /// The canonical projection key (#3516), kept in debug builds so the test
    /// suite projects every node as before, keeps that projection's refusal,
    /// and checks the structural key against it.
    #[cfg(debug_assertions)]
    fn check_key_partition(&mut self, state: &HotState, key: MemoKey) -> Result<(), VisitFailure> {
        let document = HotBoundary::try_to_canonical(state, self.catalog)
            .map_err(|refusal| VisitFailure::Refusal(ExactDfsRefusal::MemoProjection(refusal)))?;
        let mut hasher = Sha256::new();
        hasher.update(self.config.max_turns.to_be_bytes());
        hasher.update(document.canonical_json().as_bytes());
        let projected: MemoKey = hasher.finalize().into();
        let projection = *self.projection_of.entry(key).or_insert(projected);
        assert_eq!(projection, projected, "one structural key, two projections");
        let structure = *self.structure_of.entry(projected).or_insert(key);
        assert_eq!(
            structure, key,
            "one projection, two structural keys: the hot key split a projection class"
        );
        Ok(())
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
    dfs.telemetry.memo_bytes = dfs.memo_bytes();
    // A caller's seed is an achieved fallback for an *incomplete* search,
    // not an extra branch within this invocation's horizon.  In particular,
    // a valid turn-4 seed must not turn an exact max-turn-0 loss into an
    // exact win.  The complete result therefore comes only from `outcome`;
    // `best_achieved` (which includes the seed) is retained for deadline/
    // cancellation paths below.
    let (status, achieved) = match outcome {
        Ok(value) => (ExactDfsStatus::Exact, value.achieved_with_prefix(&[])),
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
    let best = match achieved
        .map(|achieved| certify(&root, catalog, achieved))
        .transpose()
    {
        Ok(best) => best,
        Err(refusal) => {
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
    let Some(objective) = terminal_objective(&state, catalog, BattlewornObjective::of_root(root))?
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
    if terminal_digest(&state, catalog)? != seed.final_digest {
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
) -> Result<Option<ExactObjective>, ExactDfsRefusal> {
    if !state.history.over || state.hp <= 0 || state.monsters.iter().any(|monster| monster.hp > 0) {
        return Ok(None);
    }
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
    Ok(Some(ExactObjective {
        won: true,
        event_reward: !battleworn.timed_out(state),
        final_hp,
        potions_kept,
        negative_turns: state
            .turn
            .checked_neg()
            .expect("exact-solve entry rejects negative turns"),
    }))
}

/// Replay an achieved line from the root and project its final state: the one
/// digest a solve reports (see [`Achieved`]).
fn certify(
    root: &HotState,
    catalog: &Catalog,
    achieved: Achieved,
) -> Result<ExactSolution, ExactDfsRefusal> {
    let final_digest = match achieved.final_digest {
        Some(digest) => digest,
        None => {
            let mut state = root.clone();
            let mut events = Vec::new();
            for action in &achieved.actions {
                state = engine::apply_action_into(&state, catalog, action, &mut events)
                    .map_err(ExactDfsRefusal::Transition)?;
            }
            terminal_digest(&state, catalog)?
        }
    };
    Ok(ExactSolution {
        actions: achieved.actions,
        objective: achieved.objective,
        final_digest,
    })
}

/// The canonical `differential_digest` of a winning terminal state: the
/// `final_digest` a solution reports. Meat on the Bone's post-terminal heal is
/// in the objective, not in this digest (see [`terminal_objective`]).
fn terminal_digest(state: &HotState, catalog: &Catalog) -> Result<String, ExactDfsRefusal> {
    Ok(HotBoundary::try_to_canonical(state, catalog)
        .map_err(ExactDfsRefusal::MemoProjection)?
        .differential_digest())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::PowerId;
    use crate::powers::SlotWire;
    use std::hash::Hasher;

    const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    fn root() -> (Catalog, HotState) {
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (catalog, state)
    }

    #[test]
    fn sha256_writer_matches_one_shot_sha256_across_staging_boundaries() {
        let bytes: Vec<u8> = (0..2000_u32).map(|i| (i * 31 % 251) as u8).collect();
        let expected: MemoKey = Sha256::digest(&bytes).into();
        for chunk in [1, 7, 64, 255, 256, 257, 511, 2000] {
            let mut writer = Sha256Writer::new();
            for part in bytes.chunks(chunk) {
                writer.write(part);
            }
            assert_eq!(writer.finish_key(), expected, "chunk {chunk}");
        }
        // Fill the stage exactly, overflow it, write one byte, then one
        // larger than the stage.
        let mut writer = Sha256Writer::new();
        for range in [0..256, 256..1000, 1000..1001, 1001..2000] {
            writer.write(&bytes[range]);
        }
        assert_eq!(writer.finish_key(), expected);
    }

    #[test]
    fn the_structural_key_separates_every_changed_field_and_the_horizon() {
        let (_, state) = root();
        let key = structural_memo_key(3, &state);
        assert_eq!(structural_memo_key(3, &state.clone()), key);
        assert_ne!(structural_memo_key(4, &state), key, "the horizon is keyed");

        let mut hp = state.clone();
        hp.hp -= 1;
        let mut power = state.clone();
        power.powers.set(PowerId::Strength, SlotWire::Int, 1);
        let mut pile = state.clone();
        let hand = pile.piles.get(crate::hot::PileId::Hand).clone();
        pile.piles.set(crate::hot::PileId::Discard, hand);
        let changed = [hp, power, pile];
        for (index, changed) in changed.iter().enumerate() {
            assert_ne!(changed, &state, "case {index} changes the state");
            assert_ne!(structural_memo_key(3, changed), key, "case {index}");
        }
    }

    #[test]
    #[should_panic(expected = "one projection, two structural keys")]
    #[cfg(debug_assertions)]
    fn the_debug_cross_check_rejects_a_split_projection_class() {
        let (catalog, state) = root();
        let cancellation = ExactCancellation::default();
        let mut dfs = Dfs::new(
            &catalog,
            ExactDfsConfig::default(),
            &cancellation,
            Instant::now(),
            None,
            0,
            BattlewornObjective::of_root(&state),
        );
        let key = structural_memo_key(dfs.config.max_turns, &state);
        assert!(dfs.check_key_partition(&state, key).is_ok());
        let mut other = key;
        other[0] ^= 1;
        let _ = dfs.check_key_partition(&state, other);
    }
}
