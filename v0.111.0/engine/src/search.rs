//! Bounded search runner; achieved results only, never optimality claims.
//! Usage: search_experiment ENTRY.json uct|random SEED SECONDS [C] [END_WEIGHT] [PLAYOUTS] [HORIZON]
//!        [--max-potions K] [--hold-slot SLOT]...
//! Search-policy randomness is independent of every game RNG stream.
//!
//! A potion holdback restricts the player's own choices, never the game's:
//! `--max-potions K` stops offering `UsePotion` once the line has drunk K
//! potions (a potion gained mid-fight counts when drunk), and each
//! `--hold-slot SLOT` never offers the entry belt's potion in that slot. A
//! held slot stays occupied, so no gained potion can land in it. The tree is
//! keyed by path, so a count over the line is exact at every node.
//!
//! An engine refusal ends only the playout that reached it (D6, as in
//! `bench.rs`): the refused action never enters the tree, the playout backs up
//! the worst reward, and the first refusal is reported. A state whose
//! legal-action enumeration refuses (#2985) is pruned the same way. A retained line is
//! replayed from the entry below, so an achieved result never crosses one.
//!
//! The search itself is [`search_document`], a library call over a canonical
//! document, so the browser engine runs the same UCT the review worker does
//! (#3419). [`run`] is the command line around it.
#[cfg(target_arch = "wasm32")]
use crate::wasm_clock::Instant;
use crate::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, BattlewornObjective, LegalActionBuffer},
    exact_solve_v1::ExactSolveActionV1,
    hot::HotState,
    solo_v1::admit_exact_solve,
};
use serde_json::{Value, json};
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (((z ^ (z >> 31)) >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn index(&mut self, n: usize) -> usize {
        (self.unit() * n as f64) as usize
    }
}

#[derive(Default)]
struct Node {
    visits: u64,
    value: f64,
    children: Vec<(Action, usize)>,
    untried: Option<Vec<Action>>,
    /// At least one of this node's actions was refused and pruned.
    refused: bool,
}

fn won(s: &HotState) -> bool {
    s.history.over && s.hp > 0 && s.monsters.iter().all(|m| m.hp <= 0)
}
fn enemy_hp(s: &HotState) -> i32 {
    s.monsters.iter().map(|m| m.hp.max(0)).sum()
}
fn stopped(s: &HotState, depth: usize, horizon: i16) -> bool {
    s.history.over || s.hp <= 0 || s.turn > horizon || depth >= 256
}
/// Playout reward. A win is in `(0.5, 1]` by HP fraction and every other
/// leaf in `[-1, -0.5]`. A Battleworn Dummy timeout (#3369) is a win that
/// forfeits the event's reward, so it takes the band between them,
/// `(0, 0.25]` by HP fraction: below any kill at any HP, above any loss.
fn reward(s: &HotState, initial_enemy_hp: i32, battleworn: BattlewornObjective) -> f64 {
    let hp_fraction = (f64::from(s.hp) / f64::from(s.max_hp.max(1))).clamp(0., 1.);
    if battleworn.timed_out(s) {
        0.25 * hp_fraction
    } else if won(s) {
        0.5 + 0.5 * hp_fraction
    } else {
        -0.5 - 0.5 * (f64::from(enemy_hp(s)) / f64::from(initial_enemy_hp.max(1))).clamp(0., 1.)
    }
}
/// Retained-line order: wins first, then (#3369) a kill over a Battleworn
/// timeout at any HP, then HP and fewer turns; other leaves by enemy HP
/// removed. The reward flag is true for every win outside a Battleworn fight,
/// so those order exactly as before.
fn ranking(s: &HotState, battleworn: BattlewornObjective) -> (bool, bool, i32, i32, i16) {
    if won(s) {
        (true, !battleworn.timed_out(s), s.hp, 0, -s.turn)
    } else {
        (false, false, -enemy_hp(s), s.hp, -s.turn)
    }
}
/// The player's potion holdback. The default admits every drink.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PotionLimit {
    max_drinks: Option<u32>,
    /// Bit `i` holds belt slot `i` of the entry.
    held_slots: u64,
}
impl PotionLimit {
    fn is_unlimited(&self) -> bool {
        self.max_drinks.is_none() && self.held_slots == 0
    }
    fn allows(&self, action: &Action, line: &[Action]) -> bool {
        let Action::UsePotion { slot, .. } = action else {
            return true;
        };
        if self.held_slots & (1u64 << u32::from(*slot).min(63)) != 0 {
            return false;
        }
        self.max_drinks.is_none_or(|max| {
            let drunk = line
                .iter()
                .filter(|a| matches!(a, Action::UsePotion { .. }))
                .count();
            drunk < max as usize
        })
    }
    fn held_slot_list(&self) -> Vec<u8> {
        (0..64u8)
            .filter(|i| self.held_slots & (1u64 << i) != 0)
            .collect()
    }
    fn to_json(self) -> Value {
        json!({"max_drinks": self.max_drinks, "held_slots": self.held_slot_list()})
    }
}

/// Split `--max-potions K` and repeated `--hold-slot SLOT` out of the argv,
/// returning the positional arguments that remain.
fn parse_potion_limit(args: &[String]) -> Result<(Vec<String>, PotionLimit), String> {
    let mut positional = vec![];
    let mut limit = PotionLimit::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--max-potions" => {
                if limit.max_drinks.is_some() {
                    return Err("--max-potions given twice".into());
                }
                let value = it.next().ok_or("--max-potions needs a count")?;
                limit.max_drinks = Some(value.parse().map_err(|_| "invalid --max-potions")?);
            }
            "--hold-slot" => {
                let value = it.next().ok_or("--hold-slot needs a slot")?;
                let slot: u8 = value.parse().map_err(|_| "invalid --hold-slot")?;
                if slot >= 64 {
                    return Err("--hold-slot must be below 64".into());
                }
                if limit.held_slots & (1u64 << slot) != 0 {
                    return Err(format!("--hold-slot {slot} given twice"));
                }
                limit.held_slots |= 1u64 << slot;
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            _ => positional.push(arg.clone()),
        }
    }
    Ok((positional, limit))
}

/// A held slot must name a potion the entry's belt holds; holding an empty or
/// absent slot is a malformed request, not a vacuous constraint.
fn check_held_slots(limit: &PotionLimit, slots: Option<&Value>) -> Result<(), String> {
    let slots = slots
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for slot in limit.held_slot_list() {
        if slots.get(usize::from(slot)).is_none_or(Value::is_null) {
            return Err(format!(
                "--hold-slot {slot}: the entry belt has no potion there"
            ));
        }
    }
    Ok(())
}

fn wire(line: &[Action]) -> Vec<ExactSolveActionV1> {
    line.iter().copied().map(Into::into).collect()
}

struct Search {
    catalog: Catalog,
    events: Vec<engine::Event>,
    legal: LegalActionBuffer,
    rng: Rng,
    transitions: u64,
    end_weight: f64,
    refusals: u64,
    first_refusal: Option<Value>,
    potion_limit: PotionLimit,
}
impl Search {
    fn apply(&mut self, s: &HotState, a: &Action, line: &[Action]) -> Result<HotState, Value> {
        self.transitions += 1;
        engine::apply_action_into(s, &self.catalog, a, &mut self.events).map_err(|e| {
            let mut witness = wire(line);
            witness.push((*a).into());
            json!({"status":"refused", "detail":format!("{e:?}"), "actions":witness})
        })
    }
    /// Apply an action already applied once from this same state (a tree
    /// edge, or the retained line). Transitions are deterministic, so a
    /// refusal here is a defect, not an unmodeled branch.
    fn reapply(&mut self, s: &HotState, a: &Action, line: &[Action]) -> Result<HotState, String> {
        self.apply(s, a, line).map_err(|e| e.to_string())
    }
    fn note_refusal(&mut self, refusal: Value) {
        self.refusals += 1;
        self.first_refusal.get_or_insert(refusal);
    }
    /// Enumerate a live state's actions. An enumeration refusal (#2985) is
    /// reported like a refused transition, with the line that reached it;
    /// a nonterminal state never comes back as an empty list. The potion
    /// holdback filters only `UsePotion`, and `EndTurn` is always legal, so
    /// a filtered list is never empty either.
    fn actions(&mut self, s: &HotState, line: &[Action]) -> Result<Vec<Action>, Value> {
        let limit = self.potion_limit;
        engine::legal_actions_checked(s, &self.catalog, &mut self.legal)
            .map(|actions| {
                if limit.is_unlimited() {
                    actions.to_vec()
                } else {
                    actions
                        .iter()
                        .copied()
                        .filter(|a| limit.allows(a, line))
                        .collect()
                }
            })
            .map_err(
                |e| json!({"status":"refused", "detail":format!("{e:?}"), "actions":wire(line)}),
            )
    }
    fn rollout_action(&mut self, actions: &[Action]) -> Action {
        let weight = |a: &Action| {
            if matches!(a, Action::EndTurn) {
                self.end_weight
            } else {
                1.0
            }
        };
        let total: f64 = actions.iter().map(weight).sum();
        let mut choice = self.rng.unit() * total;
        for a in actions {
            choice -= weight(a);
            if choice < 0. {
                return *a;
            }
        }
        *actions.last().unwrap()
    }
}

/// One bounded search's policy and budget: the command line's positional
/// arguments after the entry.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchParams {
    /// `"uct"` or `"random"`.
    pub mode: String,
    /// Search-policy randomness; independent of every game RNG stream.
    pub seed: u64,
    pub seconds: f64,
    pub exploration: f64,
    pub end_weight: f64,
    pub max_playouts: u64,
    pub horizon: i16,
}

impl SearchParams {
    /// The command line's defaults for everything it makes optional.
    #[must_use]
    pub fn new(mode: &str, seed: u64, seconds: f64) -> Self {
        Self {
            mode: mode.to_owned(),
            seed,
            seconds,
            exploration: 1.414,
            end_weight: 0.05,
            max_playouts: u64::MAX,
            horizon: 20,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !["uct", "random"].contains(&self.mode.as_str()) {
            return Err("unknown mode".into());
        }
        if !(1..=100).contains(&self.horizon) {
            return Err("horizon must be 1..100".into());
        }
        if !self.seconds.is_finite()
            || self.seconds <= 0.
            || !self.exploration.is_finite()
            || self.exploration < 0.
            || !self.end_weight.is_finite()
            || self.end_weight <= 0.
        {
            return Err("invalid budget or policy".into());
        }
        Ok(())
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (args, potion_limit) = parse_potion_limit(args)?;
    let args = &args[..];
    if args.len() < 5 {
        return Err("usage: search_experiment ENTRY.json uct|random SEED SECONDS [C=1.414] [END_WEIGHT=0.05] [PLAYOUTS]".into());
    }
    if !["uct", "random"].contains(&args[2].as_str()) {
        return Err("unknown mode".into());
    }
    let mut params = SearchParams::new(
        &args[2],
        args[3].parse().map_err(|_| "invalid seed")?,
        args[4].parse().map_err(|_| "invalid seconds")?,
    );
    if let Some(c) = args.get(5) {
        params.exploration = c.parse().map_err(|_| "invalid C")?;
    }
    if let Some(weight) = args.get(6) {
        params.end_weight = weight.parse().map_err(|_| "invalid weight")?;
    }
    if let Some(playouts) = args.get(7) {
        params.max_playouts = playouts.parse().map_err(|_| "invalid playouts")?;
    }
    if let Some(horizon) = args.get(8) {
        params.horizon = horizon.parse().map_err(|_| "invalid horizon")?;
    }
    params.validate()?;
    let document: CanonicalStateV2 =
        serde_json::from_str(&fs::read_to_string(&args[1]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    println!("{}", search_with(&document, &params, potion_limit, true)?);
    Ok(())
}

/// Search one admitted fight for `params.seconds` and return the report the
/// command line prints. Its `best` is an achieved line replayed from the
/// entry, never an optimality claim; it is `null` when no playout ended.
pub fn search_document(
    document: &CanonicalStateV2,
    params: &SearchParams,
) -> Result<Value, String> {
    search_with(document, params, PotionLimit::default(), false)
}

/// `trace` writes each improvement to stderr as it is found, as the command
/// line always has.
fn search_with(
    document: &CanonicalStateV2,
    params: &SearchParams,
    potion_limit: PotionLimit,
    trace: bool,
) -> Result<Value, String> {
    params.validate()?;
    let SearchParams {
        mode,
        seed,
        seconds,
        exploration: c,
        end_weight,
        max_playouts,
        horizon,
    } = params.clone();
    admit_exact_solve(document).map_err(|e| format!("{e:?}"))?;
    check_held_slots(&potion_limit, document.player.get("potion_slots"))?;
    let catalog = HotBoundary::catalog_from_canonical(document).map_err(|e| format!("{e:?}"))?;
    let root = HotBoundary::from_canonical(document, &catalog).map_err(|e| format!("{e:?}"))?;
    engine::admit(document, &root, &catalog).map_err(|e| format!("{e:?}"))?;
    let battleworn = BattlewornObjective::of_root(&root);
    let mut search = Search {
        catalog,
        events: vec![],
        legal: LegalActionBuffer::new(),
        rng: Rng(seed),
        transitions: 0,
        end_weight,
        refusals: 0,
        first_refusal: None,
        potion_limit,
    };
    let mut tree = vec![Node::default()];
    let (mut playouts, mut wins, mut deaths, mut cutoffs, mut refused_playouts) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut best: Option<(HotState, Vec<Action>)> = None;
    let mut improvements = vec![];
    let started = Instant::now();
    while started.elapsed().as_secs_f64() < seconds && playouts < max_playouts {
        let mut state = root.clone();
        let mut line = vec![];
        let mut path = vec![0];
        let mut refused = false;
        if mode == "uct" {
            loop {
                if stopped(&state, line.len(), horizon) {
                    break;
                }
                let n = *path.last().unwrap();
                if tree[n].untried.is_none() {
                    tree[n].untried = Some(match search.actions(&state, &line) {
                        Ok(actions) => actions,
                        Err(refusal) => {
                            // The state itself refused: a node with no
                            // actions and the refused mark, i.e. a dead end.
                            search.note_refusal(refusal);
                            tree[n].refused = true;
                            vec![]
                        }
                    });
                }
                let available = tree[n].untried.as_ref().unwrap().len();
                if available > 0 {
                    // Cap allocation; roll out from frontier once the cap is reached.
                    if tree.len() >= 250_000 {
                        break;
                    }
                    let a = tree[n]
                        .untried
                        .as_mut()
                        .unwrap()
                        .swap_remove(search.rng.index(available));
                    match search.apply(&state, &a, &line) {
                        Ok(next) => state = next,
                        Err(refusal) => {
                            // Pruned: the action never becomes a child.
                            search.note_refusal(refusal);
                            tree[n].refused = true;
                            continue;
                        }
                    }
                    line.push(a);
                    let child = tree.len();
                    tree.push(Node::default());
                    tree[n].children.push((a, child));
                    path.push(child);
                    break;
                }
                if tree[n].children.is_empty() && tree[n].refused {
                    // Every action here was refused: a dead end.
                    refused = true;
                    break;
                }
                let log_n = (tree[n].visits.max(1) as f64).ln();
                let &(a, child) = tree[n]
                    .children
                    .iter()
                    .max_by(|(_, x), (_, y)| {
                        let ucb = |i: usize| {
                            let child = &tree[i];
                            child.value / child.visits as f64
                                + c * (log_n / child.visits as f64).sqrt()
                        };
                        ucb(*x).total_cmp(&ucb(*y))
                    })
                    .ok_or("no legal actions in nonterminal state")?;
                state = search.reapply(&state, &a, &line)?;
                line.push(a);
                path.push(child);
            }
        }
        while !refused && !stopped(&state, line.len(), horizon) {
            let actions = match search.actions(&state, &line) {
                Ok(actions) => actions,
                Err(refusal) => {
                    search.note_refusal(refusal);
                    refused = true;
                    break;
                }
            };
            let a = search.rollout_action(&actions);
            match search.apply(&state, &a, &line) {
                Ok(next) => state = next,
                Err(refusal) => {
                    search.note_refusal(refusal);
                    refused = true;
                }
            }
            line.push(a);
        }
        playouts += 1;
        if refused {
            // Worst reward: steer UCT away without claiming an outcome.
            refused_playouts += 1;
            if mode == "uct" {
                for n in path {
                    tree[n].visits += 1;
                    tree[n].value -= 1.0;
                }
            }
            continue;
        }
        if won(&state) {
            wins += 1;
        } else if state.history.over || state.hp <= 0 {
            deaths += 1;
        } else {
            cutoffs += 1;
        }
        let value = reward(&state, enemy_hp(&root), battleworn);
        if mode == "uct" {
            for n in path {
                tree[n].visits += 1;
                tree[n].value += value;
            }
        }
        // Never retain a horizon leaf as an achieved loss/win.
        if (state.history.over || state.hp <= 0)
            && best
                .as_ref()
                .is_none_or(|(s, _)| ranking(&state, battleworn) > ranking(s, battleworn))
        {
            improvements.push(json!({"seconds":started.elapsed().as_secs_f64(), "playout":playouts, "won":won(&state), "battleworn_timeout":battleworn.timed_out(&state), "hp":state.hp, "enemy_hp":enemy_hp(&state), "turn":state.turn}));
            if trace {
                eprintln!("{}", improvements.last().unwrap());
            }
            best = Some((state, line));
        }
    }
    let achieved = if let Some((best_state, actions)) = best {
        // Independently replay the retained trajectory from the entry, and
        // re-check the holdback on the line that is reported.
        let mut replay = root.clone();
        for (i, a) in actions.iter().enumerate() {
            if !potion_limit.allows(a, &actions[..i]) {
                return Err("retained line breaks the potion holdback".into());
            }
            replay = search.reapply(&replay, a, &actions[..i])?;
        }
        if replay != best_state {
            return Err("retained line replay mismatch".into());
        }
        let projected = HotBoundary::try_to_canonical(&replay, &search.catalog)
            .map_err(|e| format!("{e:?}"))?;
        Some(
            json!({"won":won(&replay), "battleworn_timeout":battleworn.timed_out(&replay), "combat_hp":replay.hp, "enemy_hp":enemy_hp(&replay), "turn":replay.turn, "actions":wire(&actions), "final_digest":projected.differential_digest()}),
        )
    } else {
        None
    };
    let mut report = json!({"status":"incomplete", "mode":mode, "seed":seed, "budget_seconds":seconds,
        "elapsed_seconds":started.elapsed().as_secs_f64(), "exploration":c, "end_weight":end_weight,
        "max_turns":horizon, "max_actions":256, "max_playouts":max_playouts, "max_tree_nodes":250_000, "entry_digest":document.differential_digest(),
        "playouts":playouts, "win_visits":wins, "deaths":deaths, "cutoffs":cutoffs,
        "refused_playouts":refused_playouts, "refusals":search.refusals, "first_refusal":search.first_refusal,
        "transitions_including_replay":search.transitions, "tree_nodes":tree.len(), "improvements":improvements, "best":achieved});
    // Only a holdback run names its limit, so a default run's report is
    // byte-identical to the pre-holdback one.
    if !potion_limit.is_unlimited() {
        report["potion_limit"] = potion_limit.to_json();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hot::HotMonster, ids::MonsterKind, powers::SlotWire};

    fn battleworn_root() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 80;
        let mut dummy = HotMonster::new(MonsterKind::BattleFriendV2, 150);
        dummy.max_hp = 150;
        dummy
            .powers
            .set(crate::ids::PowerId::BattlewornTimeLimit, SlotWire::Int, 1);
        state.monsters_mut().push(dummy);
        state
    }

    fn killed(root: &HotState, hp: i32) -> HotState {
        let mut state = root.clone();
        state.monsters_mut()[0].hp = 0;
        state.history.over = true;
        state.hp = hp;
        state
    }

    fn timed_out(root: &HotState, hp: i32) -> HotState {
        let mut state = root.clone();
        state.monsters_mut().clear();
        state.history.over = true;
        state.hp = hp;
        state
    }

    /// #3369: a kill outranks a timeout at equal HP and at lower HP, in both
    /// the retained-line ranking and the playout reward; a timeout still
    /// outranks a loss.
    #[test]
    fn a_battleworn_kill_outranks_a_timeout_in_ranking_and_reward() {
        let root = battleworn_root();
        let goal = BattlewornObjective::of_root(&root);
        let initial = enemy_hp(&root);
        let (kill_equal, kill_low, timeout_high, timeout_equal) = (
            killed(&root, 60),
            killed(&root, 1),
            timed_out(&root, 80),
            timed_out(&root, 60),
        );
        assert!(ranking(&kill_equal, goal) > ranking(&timeout_equal, goal));
        assert!(ranking(&kill_low, goal) > ranking(&timeout_high, goal));
        assert!(reward(&kill_equal, initial, goal) > reward(&timeout_equal, initial, goal));
        assert!(reward(&kill_low, initial, goal) > reward(&timeout_high, initial, goal));

        let mut lost = root.clone();
        lost.hp = 0;
        lost.history.over = true;
        assert!(ranking(&timed_out(&root, 1), goal) > ranking(&lost, goal));
        assert!(reward(&timed_out(&root, 1), initial, goal) > reward(&lost, initial, goal));
        assert!(reward(&timed_out(&root, 1), initial, goal) > 0.);
    }

    /// #3369: outside a Battleworn fight an emptied roster is the same win it
    /// was, so ranking and reward are unchanged.
    #[test]
    fn non_battleworn_scores_are_unchanged() {
        let mut root = HotState::at_defaults();
        root.hp = 60;
        root.max_hp = 80;
        root.monsters_mut()
            .push(HotMonster::new(MonsterKind::ThievingHopper, 80));
        let goal = BattlewornObjective::of_root(&root);
        let mut fled = root.clone();
        fled.monsters_mut().clear();
        fled.history.over = true;
        assert_eq!(ranking(&fled, goal), (true, true, 60, 0, -fled.turn));
        assert_eq!(reward(&fled, 80, goal), 0.5 + 0.5 * 60. / 80.);
    }
    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }
    fn drink(slot: u8) -> Action {
        Action::UsePotion { slot, target: None }
    }

    /// The holdback flags leave the positional grammar intact wherever they
    /// appear, and a malformed flag refuses rather than being ignored.
    #[test]
    fn potion_limit_flags_parse_around_positionals() {
        let (positional, limit) = parse_potion_limit(&argv(&[
            "search",
            "--hold-slot",
            "2",
            "e.json",
            "uct",
            "--max-potions",
            "1",
            "7",
            "3",
            "--hold-slot",
            "0",
        ]))
        .unwrap();
        assert_eq!(positional, argv(&["search", "e.json", "uct", "7", "3"]));
        assert_eq!(limit.max_drinks, Some(1));
        assert_eq!(limit.held_slot_list(), vec![0, 2]);
        assert_eq!(
            parse_potion_limit(&argv(&["search", "e.json"])).unwrap().1,
            PotionLimit::default()
        );
        for bad in [
            &["--max-potions"][..],
            &["--max-potions", "-1"],
            &["--max-potions", "1", "--max-potions", "2"],
            &["--hold-slot", "64"],
            &["--hold-slot", "1", "--hold-slot", "1"],
            &["--hold-potions"],
        ] {
            assert!(parse_potion_limit(&argv(bad)).is_err(), "{bad:?}");
        }
    }

    /// A drink budget counts every drink on the line, a held slot is never
    /// offered, and neither ever filters a non-potion action.
    #[test]
    fn potion_limit_filters_only_over_budget_or_held_drinks() {
        let limit = PotionLimit {
            max_drinks: Some(1),
            held_slots: 1 << 2,
        };
        assert!(limit.allows(&drink(0), &[]));
        assert!(!limit.allows(&drink(2), &[]));
        assert!(!limit.allows(&drink(1), &[Action::EndTurn, drink(0)]));
        assert!(limit.allows(&Action::EndTurn, &[drink(0), drink(1)]));
        let none = PotionLimit {
            max_drinks: Some(0),
            held_slots: 0,
        };
        assert!(!none.allows(&drink(0), &[]));
        assert!(PotionLimit::default().allows(&drink(5), &[drink(0), drink(1), drink(2)]));
    }

    /// Holding an empty or absent belt slot is a malformed request.
    #[test]
    fn a_held_slot_must_hold_a_potion_at_entry() {
        let belt = json!(["FirePotion", null, "BlockPotion"]);
        let hold = |slot: u8| PotionLimit {
            max_drinks: None,
            held_slots: 1 << slot,
        };
        assert!(check_held_slots(&hold(0), Some(&belt)).is_ok());
        assert!(check_held_slots(&hold(2), Some(&belt)).is_ok());
        assert!(check_held_slots(&hold(1), Some(&belt)).is_err());
        assert!(check_held_slots(&hold(3), Some(&belt)).is_err());
        assert!(check_held_slots(&hold(0), None).is_err());
        assert!(check_held_slots(&PotionLimit::default(), None).is_ok());
    }
}
