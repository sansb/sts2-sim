//! Draw-blind lookahead policy for RelayTheSpire Coach offer reviews.
//!
//! At every decision the policy samples K "determinizations" of what it cannot
//! see — the draw pile is reshuffled and every game RNG stream is re-seeded
//! from the policy's own randomness — then searches the rest of the current
//! turn in each, through the enemy turn the engine resolves on `EndTurn`, and
//! scores the resulting state with a static evaluation. The first action with
//! the best average score is played on the real state, and the policy re-plans
//! from scratch after it, so a card drawn mid-turn is seen only once it is
//! actually in hand. Enemy intents are priced by simulation rather than by a
//! hand-written blocking rule.
//!
//! This is a heuristic baseline, not an optimality claim: the horizon is one
//! turn, so setup cards whose payoff lands later are undervalued. Measured on
//! YLVVPKPH1MTW floor 17 (1000 fights per cell) it cut Skip's Infested Prisms
//! death rate from 62% under `coach_policy` to 16%.
//!
//! Usage: coach_lookahead ROOTS.json [POLICY_SEED_OFFSET]
//! Env: COACH_THREADS (available cores - 2, at most 16), COACH_K (1),
//!      COACH_BUDGET (1500 nodes/decision), COACH_WHP (2.0: weight of one
//!      player HP against one enemy HP).
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::{env, fs};
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, BattlewornObjective, Event, LegalActionBuffer},
    hot::{HotState, PileId},
    ids::{CardId, PowerId},
    powers::SlotWire,
    solo_v1::admit_exact_solve,
};

#[path = "coach/common.rs"]
mod common;
use common::{Rng, determinize, resample};

const POLICY: &str = "draw-blind-lookahead-v1";

/// Player HP one extra energy per turn is worth, per remaining turn:
/// measured, not guessed. A free `pyre:1` at the start of each certified eval
/// fight (175 fights x 20 paired shuffles, research/relay-coach/power-values)
/// saved 1.38 ±0.18 HP per energy-turn (hallway 1.16, elite 1.95).
const ENERGY_TURN_HP: f64 = 1.4;
/// Enemy HP (plus block) this policy removes per turn, over the same fights
/// (hallway 16.4, elite 23.9): the remaining-turns estimate for [`eval`].
const ENEMY_HP_PER_TURN: f64 = 20.0;

#[derive(Clone, Copy)]
struct Params {
    k: usize,
    budget: usize,
    w_hp: f64,
    /// [`ENERGY_TURN_HP`]; `COACH_ENERGY_TURN_HP=0` restores the score that
    /// gave energy-per-turn powers no credit, for comparisons.
    energy_turn_hp: f64,
    /// Player turns searched per decision: 1 scores after this turn's enemy
    /// phase; 2 also plays out the next turn, so a setup card's payoff
    /// (Pyre's energy) is measured rather than guessed. Turn 2 is searched
    /// only from each first move's `top_m` best turn-1 positions (by the
    /// one-turn score), `budget2` nodes each: searching it from every turn-1
    /// leaf cost ~100x on crowded hands.
    horizon: u8,
    budget2: usize,
    top_m: usize,
}

fn won(s: &HotState) -> bool {
    s.history.over && s.hp > 0 && s.monsters.iter().all(|m| m.hp <= 0)
}

/// Static score of a state reached after the enemy turn (or a terminal).
///
/// A win scores higher the sooner it comes. Without that, once a win inside
/// the horizon is certain every move ties, and the two-turn search ended its
/// turn forever against a harmless last slime (SLIMES_WEAK hit the turn cap
/// in 68 of 3300 eval samples) because it could always "win next turn".
///
/// A Battleworn Dummy timeout (#3369) is a won combat that forfeits the
/// event's reward, so it scores `50_000 + 10 * hp - 5 * turn`: below every
/// kill (`100_000 + 10 * hp - 5 * turn`, and HP is far below 5_000) and
/// above every live position and loss. `battleworn` is read from the
/// decision's own live state; other fights score exactly as before.
fn eval(s: &HotState, p: &Params, battleworn: BattlewornObjective) -> f64 {
    let enemy: f64 = s
        .monsters
        .iter()
        .filter(|m| m.hp > 0)
        .map(|m| {
            let poison = f64::from(m.powers.value(PowerId::Poison).max(0));
            let poison_left = (poison * (poison + 1.0) / 2.0).min(f64::from(m.hp));
            f64::from(m.hp + m.block) - poison_left
                + 3.0 * f64::from(m.powers.value(PowerId::Strength))
                - 2.0 * f64::from(m.powers.value(PowerId::Vulnerable).clamp(0, 3))
                - 2.0 * f64::from(m.powers.value(PowerId::Weak).clamp(0, 3))
        })
        .sum();
    if battleworn.timed_out(s) {
        return 50_000.0 + 10.0 * f64::from(s.hp) - 5.0 * f64::from(s.turn);
    }
    if won(s) {
        return 100_000.0 + 10.0 * f64::from(s.hp) - 5.0 * f64::from(s.turn);
    }
    if s.hp <= 0 {
        return -100_000.0 - enemy;
    }
    let pw = |id| f64::from(s.powers.value(id));
    // Energy each future turn (Pyre), priced by the measured rate over the
    // turns this fight is likely to last. The search already charges the
    // card's energy and draw costs; this is only the effect once active.
    let turns_left = (enemy.max(0.0) / ENEMY_HP_PER_TURN).min(10.0);
    let energy = p.w_hp * p.energy_turn_hp * pw(PowerId::Pyre).max(0.0) * turns_left;
    p.w_hp * f64::from(s.hp) - enemy
        + energy
        + 4.0 * pw(PowerId::Strength)
        + 3.0 * pw(PowerId::Dexterity)
        + 1.5 * pw(PowerId::Plating)
        + 8.0 * pw(PowerId::DemonForm).min(3.0)
        + 2.0 * pw(PowerId::Barricade).min(1.0) * f64::from(s.block).min(30.0) / 10.0
        - 2.0 * pw(PowerId::Vulnerable).clamp(0.0, 3.0)
        - 2.0 * pw(PowerId::Weak).clamp(0.0, 3.0)
}

/// Collapse plays of interchangeable copies (same atom, same target) into one.
fn dedup(actions: &[Action], s: &HotState) -> Vec<Action> {
    let hand = s.piles.get(PileId::Hand).as_slice();
    let mut seen = HashSet::new();
    actions
        .iter()
        .copied()
        .filter(|a| match a {
            Action::Play {
                uid,
                target,
                selection,
            } => {
                let modified = s.card_states.as_slice().iter().any(|(u, _)| u == uid);
                match hand.iter().find(|c| c.uid == *uid) {
                    Some(card) if !modified => {
                        seen.insert((card.atom, *target, format!("{selection:?}")))
                    }
                    _ => true,
                }
            }
            _ => true,
        })
        .collect()
}

struct Planner<'a> {
    catalog: &'a Catalog,
    p: Params,
    battleworn: BattlewornObjective,
    turn: i16,
    /// Further player turns this search may still enter.
    turns_left: u8,
    /// While scoring a first move at horizon 2: its live turn-boundary
    /// positions with their one-turn scores, for turn-2 rescoring.
    leaves: Option<Vec<(f64, HotState)>>,
    nodes: usize,
    limit: usize,
    events: Vec<Event>,
}

impl Planner<'_> {
    fn apply(&mut self, s: &HotState, a: &Action) -> Option<HotState> {
        engine::apply_action_into(s, self.catalog, a, &mut self.events).ok()
    }

    fn done(&self, s: &HotState) -> bool {
        s.history.over || s.hp <= 0 || s.turn != self.turn
    }

    /// Best score reachable from `s` by the end of the horizon's last enemy phase.
    fn search(&mut self, s: &HotState) -> f64 {
        if self.done(s) {
            return self.boundary(s);
        }
        if self.nodes >= self.limit {
            return self.end_now(s);
        }
        let mut buffer = LegalActionBuffer::new();
        let actions = dedup(engine::legal_actions_into(s, self.catalog, &mut buffer), s);
        let mut best = f64::NEG_INFINITY;
        // Plays before EndTurn: DFS spends its budget on the lines that play cards.
        for a in actions
            .iter()
            .filter(|a| !matches!(a, Action::EndTurn))
            .chain(actions.iter().filter(|a| matches!(a, Action::EndTurn)))
        {
            if matches!(a, Action::UsePotion { .. }) {
                continue;
            }
            // The budget binds inside a node too: an ordered card selection
            // offers every permutation (thousands of answers), and checking
            // only on entry played each one out (HAUNTED_SHIP, eval fixture
            // f46cea139d24f54b: 167 of the suite's 169 seconds).
            if self.nodes >= self.limit && best.is_finite() {
                break;
            }
            self.nodes += 1;
            if let Some(child) = self.apply(s, a) {
                best = best.max(self.search(&child));
            }
        }
        if best.is_finite() {
            best
        } else {
            self.end_now(s)
        }
    }

    /// A turn boundary: record it for turn-2 rescoring (first pass of a
    /// horizon-2 decision), or score it / search on as the horizon allows.
    fn boundary(&mut self, s: &HotState) -> f64 {
        let value = eval(s, &self.p, self.battleworn);
        match &mut self.leaves {
            Some(leaves) if !(s.history.over || s.hp <= 0) => {
                leaves.push((value, s.clone()));
                value
            }
            Some(_) => value,
            None => self.next_turn(s),
        }
    }

    /// Search the next player turn from a turn boundary when the horizon
    /// allows; score it otherwise.
    fn next_turn(&mut self, s: &HotState) -> f64 {
        if s.history.over || s.hp <= 0 || self.turns_left == 0 || s.turn != self.turn + 1 {
            return eval(s, &self.p, self.battleworn);
        }
        let saved = (self.turn, self.turns_left, self.nodes, self.limit);
        self.turn = s.turn;
        self.turns_left -= 1;
        self.nodes = 0;
        self.limit = self.p.budget2;
        let value = self.search(s);
        (self.turn, self.turns_left, self.nodes, self.limit) = saved;
        value
    }

    /// Out of budget: end the turn (answering any pending choice with its first option).
    fn end_now(&mut self, s: &HotState) -> f64 {
        let mut s = s.clone();
        let mut buffer = LegalActionBuffer::new();
        for _ in 0..64 {
            if self.done(&s) {
                return self.boundary(&s);
            }
            let actions = engine::legal_actions_into(&s, self.catalog, &mut buffer);
            let a = actions
                .iter()
                .find(|a| matches!(a, Action::EndTurn))
                .or_else(|| actions.first())
                .copied();
            match a.and_then(|a| self.apply(&s, &a)) {
                Some(next) => s = next,
                None => break,
            }
        }
        -200_000.0
    }
}

fn choose(s: &HotState, catalog: &Catalog, p: Params, rng: &mut Rng) -> Option<Action> {
    let mut buffer = LegalActionBuffer::new();
    let legal: Vec<Action> = engine::legal_actions_into(s, catalog, &mut buffer)
        .iter()
        .copied()
        .filter(|a| !matches!(a, Action::UsePotion { .. }))
        .collect();
    let roots = dedup(&legal, s);
    if roots.len() <= 1 {
        return roots.first().copied();
    }
    // A live decision state still holds the dummy, so it names the fight.
    let battleworn = BattlewornObjective::of_root(s);
    let mut totals = vec![0.0; roots.len()];
    for _ in 0..p.k {
        let world = determinize(s, rng);
        let mut planner = Planner {
            catalog,
            p,
            battleworn,
            turn: s.turn,
            turns_left: p.horizon.saturating_sub(1),
            leaves: None,
            nodes: 0,
            limit: 0,
            events: vec![],
        };
        let share = p.budget / roots.len();
        for (i, a) in roots.iter().enumerate() {
            planner.limit = planner.nodes + share.max(1);
            let Some(child) = planner.apply(&world, a) else {
                totals[i] += -300_000.0;
                continue;
            };
            if planner.turns_left == 0 {
                totals[i] += planner.search(&child);
                continue;
            }
            // Horizon 2: the one-turn pass ranks this move's turn-1 lines;
            // the best few are played on through turn 2. A line that ends
            // the fight within turn 1 keeps its terminal score.
            planner.leaves = Some(vec![]);
            let one_turn = planner.search(&child);
            let mut leaves = planner.leaves.take().unwrap_or_default();
            leaves.sort_by(|a, b| b.0.total_cmp(&a.0));
            leaves.truncate(p.top_m.max(1));
            let (nodes, limit) = (planner.nodes, planner.limit);
            let deeper = leaves
                .iter()
                .map(|(_, leaf)| planner.next_turn(leaf))
                .fold(f64::NEG_INFINITY, f64::max);
            (planner.nodes, planner.limit) = (nodes, limit);
            let ended = one_turn.abs() >= 100_000.0;
            totals[i] += if deeper.is_finite() && !(ended && one_turn > deeper) {
                deeper
            } else {
                one_turn
            };
        }
    }
    let best = (0..roots.len())
        .max_by(|&a, &b| totals[a].total_cmp(&totals[b]))
        .unwrap();
    Some(roots[best])
}

fn effective_damage(before_hp: i32, unblocked: i32) -> i32 {
    before_hp.max(0).min(unblocked.max(0))
}

fn trial(
    doc: CanonicalStateV2,
    policy_seed: u64,
    resample_index: Option<u64>,
    p: Params,
) -> Result<Value, String> {
    admit_exact_solve(&doc).map_err(|e| format!("admission: {e:?}"))?;
    let catalog: Catalog =
        HotBoundary::catalog_from_canonical(&doc).map_err(|e| format!("catalog: {e:?}"))?;
    let mut state =
        HotBoundary::from_canonical(&doc, &catalog).map_err(|e| format!("root: {e:?}"))?;
    engine::admit(&doc, &state, &catalog).map_err(|e| format!("engine admission: {e:?}"))?;
    if let Some(index) = resample_index {
        state = resample(&state, index);
    }
    // Research switches for measuring what a card or power is worth
    // (research/relay-coach/power-values). Neither is set in production.
    //
    // COACH_INJECT=pyre:1,...  start the fight with these player powers active:
    // the value of the *effect*, with no card, energy or draw cost.
    if let Ok(spec) = env::var("COACH_INJECT") {
        for item in spec.split(',').filter(|x| !x.is_empty()) {
            let (name, amount) = item
                .split_once(':')
                .ok_or("COACH_INJECT wants name:amount")?;
            let id = PowerId::from_str(name).ok_or_else(|| format!("unknown power {name}"))?;
            let amount: i32 = amount
                .parse()
                .map_err(|_| format!("bad amount in {item}"))?;
            state
                .powers
                .set(id, SlotWire::Int, state.powers.value(id) + amount);
        }
    }
    // COACH_FORCE_T1=PYRE  the card (already in the deck) starts in the opening
    // hand, swapped for a random hand card, and is played first on turn 1: the
    // card's best case with its real energy cost and deck slot.
    let force = match env::var("COACH_FORCE_T1") {
        Ok(name) => {
            let id = CardId::from_str(&name).ok_or_else(|| format!("unknown card {name}"))?;
            let is_it = |c: &sts_sim::hot::HotCard| {
                catalog.spec(c.atom).is_some_and(|x| x.identity.id == id)
            };
            if !state.piles.get(PileId::Hand).as_slice().iter().any(is_it) {
                let draw = state.piles.get(PileId::Draw).as_slice();
                let at = draw
                    .iter()
                    .position(is_it)
                    .ok_or_else(|| format!("{name} is not in the draw pile"))?;
                let hand_len = state.piles.get(PileId::Hand).len();
                let mut pick = Rng(policy_seed ^ 0x464f_5243);
                let swap = pick.below(hand_len.max(1));
                let card = state.piles.get(PileId::Draw).as_slice()[at];
                let out = std::mem::replace(
                    &mut state.piles.get_mut(PileId::Hand).make_mut()[swap],
                    card,
                );
                state.piles.get_mut(PileId::Draw).make_mut()[at] = out;
            }
            Some(id)
        }
        Err(_) => None,
    };
    let starting_hp = state.hp;
    let mut rng = Rng(policy_seed);
    let mut events = vec![];
    let mut actions_taken = 0;
    let (mut damage, mut block_generated) = (0_i64, 0_i64);
    while !state.history.over && state.hp > 0 && state.turn <= 20 && actions_taken < 400 {
        let forced = force.filter(|_| state.turn == 1).and_then(|id| {
            let hand = state.piles.get(PileId::Hand).as_slice();
            let mut buffer = LegalActionBuffer::new();
            engine::legal_actions_into(&state, &catalog, &mut buffer)
                .iter()
                .copied()
                .find(|a| {
                    matches!(a, Action::Play { uid, .. } if hand.iter().any(|c| c.uid == *uid
                    && catalog.spec(c.atom).is_some_and(|x| x.identity.id == id)))
                })
        });
        let action = match forced {
            Some(a) => a,
            None => {
                choose(&state, &catalog, p, &mut rng).ok_or("no legal action in a live state")?
            }
        };
        let mut enemy_hp: HashMap<_, _> = state.monsters.iter().map(|m| (m.uid, m.hp)).collect();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .map_err(|e| format!("combat transition after {actions_taken} actions: {e:?}"))?;
        for event in &events {
            match event {
                Event::MonsterDamaged {
                    uid, unblocked, hp, ..
                } => {
                    if let Some(before) = enemy_hp.get(uid) {
                        damage += i64::from(effective_damage(*before, *unblocked));
                    }
                    enemy_hp.insert(*uid, *hp);
                }
                Event::PlayerBlockGained { amount, .. } => block_generated += i64::from(*amount),
                _ => {}
            }
        }
        actions_taken += 1;
    }
    let cutoff = !state.history.over && state.hp > 0;
    Ok(
        json!({"won":won(&state),"cutoff":cutoff,"hp_lost":starting_hp-state.hp.max(0),
        "remaining_hp":state.hp.max(0),"turn":state.turn,
        "damage_dealt":damage,"block_generated":block_generated}),
    )
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let p = Params {
        k: env_or("COACH_K", 1),
        budget: env_or("COACH_BUDGET", 1500),
        horizon: env_or("COACH_HORIZON", 1),
        budget2: env_or("COACH_BUDGET2", 150),
        top_m: env_or("COACH_TOPM", 3),
        w_hp: env_or("COACH_WHP", 2.0),
        energy_turn_hp: env_or("COACH_ENERGY_TURN_HP", ENERGY_TURN_HP),
    };
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let threads: usize = env_or("COACH_THREADS", cores.saturating_sub(2).clamp(1, 16));
    let result = (|| -> Result<Value, String> {
        if args.len() != 2 && args.len() != 3 {
            return Err("usage: coach_lookahead ROOTS.json [POLICY_SEED_OFFSET]".into());
        }
        let offset: u64 = args
            .get(2)
            .map_or(Ok(0), |x| x.parse())
            .map_err(|_| "invalid policy seed offset")?;
        let roots: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(&args[1]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let want_resample = env::var("COACH_RESAMPLE").is_ok_and(|v| v == "1");
        // Fights vary a lot in length: threads pull the next root from a
        // shared cursor instead of taking fixed chunks.
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut out: Vec<Option<Result<Value, String>>> = (0..roots.len()).map(|_| None).collect();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads.max(1))
                .map(|_| {
                    let (roots, next) = (&roots, &next);
                    scope.spawn(move || {
                        let mut done = vec![];
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(root) = roots.get(i) else { break };
                            let index = offset.wrapping_add(i as u64);
                            let r = serde_json::from_value(root.clone())
                                .map_err(|e| e.to_string())
                                .and_then(|doc| {
                                    trial(
                                        doc,
                                        0x4c4f4f4b_u64.wrapping_add(index),
                                        want_resample.then_some(index),
                                        p,
                                    )
                                });
                            done.push((i, r.map_err(|e| format!("root {i}: {e}"))));
                        }
                        done
                    })
                })
                .collect();
            for h in handles {
                for (i, r) in h.join().expect("worker panicked") {
                    out[i] = Some(r);
                }
            }
        });
        let trials = out
            .into_iter()
            .map(|r| r.unwrap())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            json!({"policy":POLICY,"k":p.k,"budget":p.budget,"w_hp":p.w_hp,
            "energy_turn_hp":p.energy_turn_hp,"horizon":p.horizon,"budget2":p.budget2,"top_m":p.top_m,"trials":trials}),
        )
    })();
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_draw_order_and_game_rng_cannot_change_policy_action() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let one = Params {
            k: 1,
            budget: 300,
            w_hp: 2.0,
            energy_turn_hp: ENERGY_TURN_HP,
            horizon: 1,
            budget2: 1,
            top_m: 1,
        };
        let two = Params {
            horizon: 2,
            budget2: 40,
            ..one
        };
        for (row, p) in corpus["rows"]
            .as_array()
            .unwrap()
            .iter()
            .take(4)
            .flat_map(|row| [(row, one), (row, two)])
        {
            let doc: CanonicalStateV2 = serde_json::from_value(row["entry"].clone()).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
            let mut hidden = state.clone();
            hidden.piles.get_mut(PileId::Draw).make_mut().reverse();
            for stream in common::STREAMS {
                let mut rng = hidden.rng.get(stream);
                rng.words = [
                    rng.words[3] ^ 0x55,
                    rng.words[0],
                    rng.words[1] | 1,
                    rng.words[2],
                ];
                hidden.rng.set(stream, rng);
            }
            assert_eq!(
                choose(&state, &catalog, p, &mut Rng(7)),
                choose(&hidden, &catalog, p, &mut Rng(7))
            );
        }
    }

    #[test]
    fn copies_of_one_card_are_searched_once() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let doc: CanonicalStateV2 =
            serde_json::from_value(corpus["rows"][0]["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let mut buffer = LegalActionBuffer::new();
        let legal = engine::legal_actions_into(&state, &catalog, &mut buffer).to_vec();
        let kept = dedup(&legal, &state);
        let hand = state.piles.get(PileId::Hand).as_slice();
        let distinct: HashSet<_> = legal
            .iter()
            .filter_map(|a| match a {
                Action::Play { uid, target, .. } => hand
                    .iter()
                    .find(|c| c.uid == *uid)
                    .map(|c| (c.atom, *target)),
                _ => None,
            })
            .collect();
        let plays = kept
            .iter()
            .filter(|a| matches!(a, Action::Play { .. }))
            .count();
        assert_eq!(plays, distinct.len());
        assert!(kept.contains(&Action::EndTurn) == legal.contains(&Action::EndTurn));
    }

    #[test]
    fn an_active_pyre_is_worth_more_the_longer_the_fight_has_left() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let doc: CanonicalStateV2 =
            serde_json::from_value(corpus["rows"][0]["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let plain = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let p = Params {
            k: 1,
            budget: 1,
            w_hp: 2.0,
            energy_turn_hp: ENERGY_TURN_HP,
            horizon: 1,
            budget2: 1,
            top_m: 1,
        };
        let mut pyre = plain.clone();
        pyre.powers.set(PowerId::Pyre, SlotWire::Int, 1);
        let gain = eval(&pyre, &p, BattlewornObjective::default())
            - eval(&plain, &p, BattlewornObjective::default());
        assert!(gain > 0.0);
        let mut nearly_dead = pyre.clone();
        let mut monsters = (*nearly_dead.monsters).clone();
        for m in &mut monsters {
            m.hp = m.hp.min(1);
            m.block = 0;
        }
        nearly_dead.monsters = std::sync::Arc::new(monsters);
        let mut plain_nearly_dead = nearly_dead.clone();
        plain_nearly_dead
            .powers
            .set(PowerId::Pyre, SlotWire::Int, 0);
        assert!(
            eval(&nearly_dead, &p, BattlewornObjective::default())
                - eval(&plain_nearly_dead, &p, BattlewornObjective::default())
                < gain
        );
        let off = Params {
            energy_turn_hp: 0.0,
            ..p
        };
        assert_eq!(
            eval(&pyre, &off, BattlewornObjective::default()),
            eval(&plain, &off, BattlewornObjective::default())
        );
    }

    #[test]
    fn a_sooner_win_scores_higher() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let doc: CanonicalStateV2 =
            serde_json::from_value(corpus["rows"][0]["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let mut won_now = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let mut monsters = (*won_now.monsters).clone();
        for m in &mut monsters {
            m.hp = 0;
        }
        won_now.monsters = std::sync::Arc::new(monsters);
        won_now.history.over = true;
        let mut won_later = won_now.clone();
        won_later.turn += 1;
        let p = Params {
            k: 1,
            budget: 1,
            w_hp: 2.0,
            energy_turn_hp: ENERGY_TURN_HP,
            horizon: 1,
            budget2: 1,
            top_m: 1,
        };
        assert!(
            eval(&won_now, &p, BattlewornObjective::default())
                > eval(&won_later, &p, BattlewornObjective::default())
        );
        // ...but never by more than an HP point: HP still decides first.
        let mut hurt = won_now.clone();
        hurt.hp -= 1;
        assert!(
            eval(&hurt, &p, BattlewornObjective::default())
                < eval(&won_later, &p, BattlewornObjective::default())
        );
    }

    /// #3369: in a Battleworn Dummy fight a kill outscores a timeout at
    /// equal HP and at lower HP, and a timeout outscores a live position and
    /// a death; outside it an emptied roster scores as the win it was.
    #[test]
    fn a_battleworn_kill_outscores_a_timeout() {
        use sts_sim::hot::HotMonster;
        use sts_sim::ids::MonsterKind;
        let p = Params {
            k: 1,
            budget: 1,
            w_hp: 2.0,
            energy_turn_hp: ENERGY_TURN_HP,
            horizon: 1,
            budget2: 1,
            top_m: 1,
        };
        let mut root = HotState::at_defaults();
        root.hp = 60;
        root.max_hp = 80;
        let mut dummy = HotMonster::new(MonsterKind::BattleFriendV2, 150);
        dummy.max_hp = 150;
        dummy
            .powers
            .set(PowerId::BattlewornTimeLimit, SlotWire::Int, 1);
        root.monsters_mut().push(dummy);
        let goal = BattlewornObjective::of_root(&root);
        let at = |hp: i32, kill: bool| {
            let mut s = root.clone();
            if kill {
                s.monsters_mut()[0].hp = 0;
            } else {
                s.monsters_mut().clear();
            }
            s.history.over = true;
            s.hp = hp;
            s
        };
        assert!(eval(&at(60, true), &p, goal) > eval(&at(60, false), &p, goal));
        assert!(eval(&at(1, true), &p, goal) > eval(&at(80, false), &p, goal));
        assert!(eval(&at(1, false), &p, goal) > eval(&root, &p, goal));
        let mut dead = root.clone();
        dead.hp = 0;
        assert!(eval(&at(1, false), &p, goal) > eval(&dead, &p, goal));

        let plain = BattlewornObjective::default();
        assert_eq!(
            eval(&at(60, false), &p, plain),
            100_000.0 + 600.0 - 5.0 * f64::from(root.turn)
        );
    }

    #[test]
    fn terminal_scores_order_win_over_survival_over_death() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let doc: CanonicalStateV2 =
            serde_json::from_value(corpus["rows"][0]["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let alive = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let p = Params {
            k: 1,
            budget: 1,
            w_hp: 2.0,
            energy_turn_hp: ENERGY_TURN_HP,
            horizon: 1,
            budget2: 1,
            top_m: 1,
        };
        let mut dead = alive.clone();
        dead.hp = 0;
        assert!(
            eval(&alive, &p, BattlewornObjective::default())
                > eval(&dead, &p, BattlewornObjective::default())
        );
        assert!(eval(&alive, &p, BattlewornObjective::default()) < 100_000.0);
    }
}
