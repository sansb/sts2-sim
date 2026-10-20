//! Synthetic witnesses for the information-set search (#3985).
//!
//! Every position is the Seapunk entry (`f156e43104f3752c`, Ironclad, turn 1)
//! with its piles, energy, player HP and monster HP edited, and its relics
//! removed so no relic moves HP after the fight. Seapunk's turn-1 attack is
//! measured at 13 damage, so at 8 or 12 HP any line that ends the turn
//! without the kill dies: every line in these positions resolves in turn 1.
//!
//! The witnesses live inside the crate because the exhaustive oracle scores
//! leaves with `search::reward`, the scale ISMCTS uses, which is
//! `pub(crate)`.
//!
//! # The oracle
//!
//! [`Oracle`] is exhaustive expectimax over the information sets: the root
//! belief is every order of the draw pile, equally likely (the distribution
//! `hidden::sample_world` draws from); a decision takes the max over the
//! actions of the information set; after each action the belief splits by
//! the next information-set key (`hidden::info_key`) and each part is
//! weighted by its share. The same belief over a single world is the
//! perfect-information value, which a PIMC loop averages.
//!
//! The search gives every sampled world fresh stream words; the oracle keeps
//! the root's. It is therefore exact only if no step's outcome depends on the
//! words, and it checks that at every step that consumes a stream: the step is
//! re-applied under three other word sets and must give the same observation,
//! or, for a step that ends the line, the same reward. In these positions the
//! consuming steps are Havoc (a `Targets` draw over the one monster) and
//! the deaths that end turn 1 (the monster's turn).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use super::{Information, IsmctsParams, IsmctsResult, search};
use crate::boundary::HotBoundary;
use crate::canonical::CanonicalStateV2;
use crate::catalog::Catalog;
use crate::engine::{self, Action, BattlewornObjective, LegalActionBuffer};
use crate::hidden::{self, DrawKnowledge, InfoKey, PolicyRng, STREAMS};
use crate::hot::{HotState, PileId};
use crate::search::{enemy_hp, reward, stopped, won};

struct Position {
    root: HotState,
    catalog: Catalog,
    /// Card id by uid: hand first, then the draw pile.
    names: Vec<&'static str>,
}

/// The Seapunk entry with `hand` and `draw` (top first) as its piles.
fn position(
    hand: &[&'static str],
    draw: &[&'static str],
    energy: i32,
    hp: i32,
    monster_hp: i32,
) -> Position {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights/f156e43104f3752c/entry.canonical.json");
    let mut doc: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let names: Vec<&'static str> = hand.iter().chain(draw).copied().collect();
    let cards = |ids: &[&str], first: usize| -> Value {
        ids.iter()
            .enumerate()
            .map(|(i, id)| json!({"id": id, "uid": first + i, "upgrade": 0}))
            .collect()
    };
    doc["piles"] = json!({
        "hand": cards(hand, 0),
        "draw": cards(draw, hand.len()),
        "discard": [],
    });
    let player = doc["player"].as_object_mut().unwrap();
    player.insert("next_card_uid".into(), json!(names.len()));
    player.insert("energy".into(), json!(energy));
    player.insert("hp".into(), json!(hp));
    player.insert("max_hp".into(), json!(hp));
    player.remove("relics_entering");
    player.remove("relics_entering_dispatch_ordered");
    doc["monsters"][0]["hp"] = json!(monster_hp);
    let document: CanonicalStateV2 = serde_json::from_value(doc).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let root = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &root, &catalog).unwrap();
    Position {
        root,
        catalog,
        names,
    }
}

impl Position {
    fn name(&self, action: &Action) -> &'static str {
        match action {
            Action::Play { uid, .. } => self.names[*uid as usize],
            Action::EndTurn => "EndTurn",
            _ => "other",
        }
    }

    fn top(&self, world: &HotState) -> &'static str {
        self.names[world.piles.get(PileId::Draw).as_slice()[0].uid as usize]
    }

    /// Every order of the draw pile, each once.
    fn worlds(&self) -> Vec<HotState> {
        let draw = self.root.piles.get(PileId::Draw).as_slice().to_vec();
        let mut order: Vec<usize> = (0..draw.len()).collect();
        let mut out = Vec::new();
        permute(&mut order, 0, &mut |order| {
            let mut world = self.root.clone();
            let pile = world.piles.get_mut(PileId::Draw).make_mut();
            for (slot, &i) in order.iter().enumerate() {
                pile[slot] = draw[i];
            }
            out.push(world);
        });
        out
    }

    /// The first world, in enumeration order, with `top` on top.
    fn world_with_top(&self, top: &str) -> HotState {
        self.worlds()
            .into_iter()
            .find(|world| self.top(world) == top)
            .unwrap()
    }

    fn search(&self, state: &HotState, information: Information, iterations: u64) -> IsmctsResult {
        search(
            state,
            &DrawKnowledge::none(),
            &self.catalog,
            &IsmctsParams::new(information, iterations, 1),
        )
        .unwrap()
    }

    fn recommended(&self, result: &IsmctsResult) -> &'static str {
        self.name(&result.recommended.unwrap())
    }

    /// Exact expectimax of each root action over every draw order.
    fn expectimax(&self) -> Vec<(&'static str, f64)> {
        self.values(&self.worlds())
    }

    /// The PIMC value of each root action: the mean over draw orders of its
    /// perfect-information value.
    fn pimc(&self) -> Vec<(&'static str, f64)> {
        let worlds = self.worlds();
        let mut total: Vec<(&'static str, f64)> = Vec::new();
        for world in &worlds {
            let perfect = self.values(std::slice::from_ref(world));
            for (i, (name, value)) in perfect.into_iter().enumerate() {
                if total.len() <= i {
                    total.push((name, 0.0));
                }
                total[i].1 += value / worlds.len() as f64;
            }
        }
        total
    }

    fn values(&self, worlds: &[HotState]) -> Vec<(&'static str, f64)> {
        let belief: Vec<_> = worlds
            .iter()
            .map(|world| (world.clone(), DrawKnowledge::none()))
            .collect();
        Oracle::new(&self.root, &self.catalog)
            .action_values(&belief, 0)
            .into_iter()
            .map(|(action, value)| (self.name(&action), value))
            .collect()
    }
}

fn permute(items: &mut [usize], k: usize, visit: &mut impl FnMut(&[usize])) {
    if k == items.len() {
        visit(items);
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permute(items, k + 1, visit);
        items.swap(k, i);
    }
}

fn value_of(values: &[(&str, f64)], name: &str) -> f64 {
    values.iter().find(|(n, _)| *n == name).unwrap().1
}

fn argmax<'a>(values: &[(&'a str, f64)]) -> &'a str {
    values.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0
}

/// Exhaustive expectimax over beliefs of equally likely worlds that share
/// one information set.
struct Oracle<'a> {
    catalog: &'a Catalog,
    initial_enemy_hp: i32,
    battleworn: BattlewornObjective,
    horizon: i16,
}

impl<'a> Oracle<'a> {
    fn new(root: &HotState, catalog: &'a Catalog) -> Self {
        Self {
            catalog,
            initial_enemy_hp: enemy_hp(root),
            battleworn: BattlewornObjective::of_root(root),
            horizon: IsmctsParams::new(Information::Hidden, 0, 0).horizon,
        }
    }

    /// Each action's value at the information set `belief`.
    fn action_values(
        &self,
        belief: &[(HotState, DrawKnowledge)],
        depth: usize,
    ) -> Vec<(Action, f64)> {
        let mut legal = LegalActionBuffer::new();
        let actions = engine::legal_actions_checked(&belief[0].0, self.catalog, &mut legal)
            .unwrap()
            .to_vec();
        let mut events = Vec::new();
        let mut values = Vec::new();
        for action in actions {
            let mut parts: BTreeMap<InfoKey, Vec<(HotState, DrawKnowledge)>> = BTreeMap::new();
            for (world, know) in belief {
                let offered =
                    engine::legal_actions_checked(world, self.catalog, &mut legal).unwrap();
                assert!(
                    offered.contains(&action),
                    "the action set depends on hidden state"
                );
                let next =
                    engine::apply_action_into(world, self.catalog, &action, &mut events).unwrap();
                let next_know =
                    hidden::observe_step(self.catalog, world, know, &next, &events).unwrap();
                if STREAMS
                    .iter()
                    .any(|&stream| next.rng.get(stream).counter != world.rng.get(stream).counter)
                {
                    self.check_word_independent(world, &action, &next, depth + 1);
                }
                parts
                    .entry(hidden::info_key(&next, &next_know))
                    .or_default()
                    .push((next, next_know));
            }
            let value = parts
                .values()
                .map(|part| part.len() as f64 / belief.len() as f64 * self.value(part, depth + 1))
                .sum();
            values.push((action, value));
        }
        values
    }

    fn value(&self, belief: &[(HotState, DrawKnowledge)], depth: usize) -> f64 {
        // Whether a line has stopped is visible: every world agrees.
        if stopped(&belief[0].0, depth, self.horizon) {
            return belief
                .iter()
                .map(|(world, _)| reward(world, self.initial_enemy_hp, self.battleworn))
                .sum::<f64>()
                / belief.len() as f64;
        }
        self.action_values(belief, depth)
            .into_iter()
            .map(|(_, value)| value)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// A stream-consuming step must not depend on the stream's words.
    fn check_word_independent(
        &self,
        world: &HotState,
        action: &Action,
        next: &HotState,
        depth: usize,
    ) {
        // Knowing the whole pile keeps its order: only the words change.
        let whole_pile = DrawKnowledge {
            top: world
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect(),
            bottom: Vec::new(),
        };
        let mut events = Vec::new();
        for salt in 1..=3 {
            let rewords = hidden::sample_world(world, &whole_pile, &mut PolicyRng(salt));
            let other =
                engine::apply_action_into(&rewords, self.catalog, action, &mut events).unwrap();
            if stopped(next, depth, self.horizon) {
                assert!(stopped(&other, depth, self.horizon));
                assert_eq!(
                    reward(next, self.initial_enemy_hp, self.battleworn),
                    reward(&other, self.initial_enemy_hp, self.battleworn),
                    "{action:?}: a terminal reward depends on stream words"
                );
            } else {
                assert!(
                    hidden::canonical_representative(next, &DrawKnowledge::none())
                        == hidden::canonical_representative(&other, &DrawKnowledge::none()),
                    "{action:?}: stream words moved the observation before the horizon"
                );
            }
        }
    }
}

/// Seapunk at 15 HP, 1 energy, 12/12 HP; hand Pommel Strike, Hemokinesis;
/// draw pile {Anger, Wound}.
///
/// - **Hedge:** Hemokinesis (15 damage, lose 2 HP) wins in every world at
///   10/12: reward 0.9167.
/// - **Gamble:** Pommel Strike (9 damage, draw 1) wins at 12/12 (reward 1)
///   when it draws Anger (6 damage, 0 cost). When it draws the Wound it
///   leaves Seapunk at 6 and the player dies to the 13-damage attack: −0.7.
///   Expected 0.15.
///
/// The clairvoyant search in the Anger-on-top world plays the gamble; the
/// hidden search plays the hedge in both worlds.
#[test]
fn a_sure_line_beats_a_coin_flip_only_clairvoyance_can_call() {
    let p = position(
        &["POMMEL_STRIKE", "HEMOKINESIS"],
        &["ANGER", "WOUND"],
        1,
        12,
        15,
    );
    let exact = p.expectimax();
    eprintln!("expectimax {exact:?}");
    assert!((value_of(&exact, "HEMOKINESIS") - (0.5 + 0.5 * 10.0 / 12.0)).abs() < 1e-12);
    assert!((value_of(&exact, "POMMEL_STRIKE") - 0.15).abs() < 1e-12);
    let anger = p.world_with_top("ANGER");
    let wound = p.world_with_top("WOUND");
    let clairvoyant_anger = p.search(&anger, Information::Clairvoyant, 4_000);
    let clairvoyant_wound = p.search(&wound, Information::Clairvoyant, 4_000);
    eprintln!(
        "clairvoyant: Anger on top {}, Wound on top {}",
        p.recommended(&clairvoyant_anger),
        p.recommended(&clairvoyant_wound)
    );
    assert_eq!(p.recommended(&clairvoyant_anger), "POMMEL_STRIKE");
    assert_eq!(p.recommended(&clairvoyant_wound), "HEMOKINESIS");
    for world in [&anger, &wound] {
        let hidden = p.search(world, Information::Hidden, 4_000);
        assert_eq!(
            p.recommended(&hidden),
            "HEMOKINESIS",
            "top {}",
            p.top(world)
        );
    }
}

fn fusion_position(draw: &[&'static str]) -> Position {
    position(&["HEMOKINESIS", "POMMEL_STRIKE", "HAVOC"], draw, 2, 8, 30)
}

/// Seapunk at 30 HP, 2 energy, 8/8 HP; hand Hemokinesis, Pommel Strike,
/// Havoc; draw pile {Anger, Bludgeon}. Bludgeon costs 3, so it lands only
/// through Havoc (play the top card).
///
/// - Hemokinesis first (15 damage, 6 HP left) leaves one energy and a blind
///   choice: Pommel Strike wins if Anger is on top (9 + 6), Havoc if Bludgeon
///   is (32). Every world has a winning follow-up, so PIMC (each world solved
///   with its draw order known, then averaged) values it as a sure win at
///   6/8: 0.875. Choosing without the order, it wins half the time: 0.1375.
///   That gap is strategy fusion.
/// - Pommel Strike first reveals the top card before anything is committed:
///   with Anger, Anger then Havoc on Bludgeon wins at 8/8; with Bludgeon in
///   hand, the best is Hemokinesis to 24 of 30 (−0.6). Exact value 0.2.
/// - Havoc first wins at 8/8 when Bludgeon is on top, else reaches 21
///   (−0.65): 0.175.
///
/// The hidden search picks Pommel Strike; the clairvoyant search picks Havoc
/// when Bludgeon is on top (and Pommel Strike when Anger is); PIMC picks
/// Hemokinesis.
/// Closed-loop, the hidden search's play scores the exact expectimax value.
#[test]
fn ismcts_does_not_fuse_strategies_where_pimc_does() {
    let p = fusion_position(&["ANGER", "BLUDGEON"]);
    let exact = p.expectimax();
    let pimc = p.pimc();
    eprintln!("expectimax {exact:?}\npimc {pimc:?}");
    assert_eq!(argmax(&exact), "POMMEL_STRIKE");
    assert!((value_of(&exact, "POMMEL_STRIKE") - 0.2).abs() < 1e-12);
    assert!((value_of(&exact, "HAVOC") - 0.175).abs() < 1e-12);
    assert!((value_of(&exact, "HEMOKINESIS") - 0.1375).abs() < 1e-12);
    assert_eq!(argmax(&pimc), "HEMOKINESIS");
    assert!((value_of(&pimc, "HEMOKINESIS") - 0.875).abs() < 1e-12);

    let anger = p.world_with_top("ANGER");
    let bludgeon = p.world_with_top("BLUDGEON");
    let clairvoyant_anger = p.search(&anger, Information::Clairvoyant, 8_000);
    let clairvoyant_bludgeon = p.search(&bludgeon, Information::Clairvoyant, 8_000);
    eprintln!(
        "clairvoyant: Anger on top {}, Bludgeon on top {}",
        p.recommended(&clairvoyant_anger),
        p.recommended(&clairvoyant_bludgeon)
    );
    assert_eq!(p.recommended(&clairvoyant_bludgeon), "HAVOC");
    assert_eq!(p.recommended(&clairvoyant_anger), "POMMEL_STRIKE");

    // The hidden search in the world where the clairvoyant one gambles. Its
    // statistics are the same in every world of the information set (the
    // invariance witness in `tests/ismcts.rs`). Pommel Strike's edge over
    // Havoc is 0.025, which takes a budget to resolve: at 4,000-20,000
    // iterations the most-visited root action varies with the seed between
    // the two, while Hemokinesis (the PIMC choice) is most visited in one
    // run of 25 (4,000 iterations); at 60,000 every seed tried picks Pommel
    // Strike.
    let hidden = p.search(&bludgeon, Information::Hidden, 60_000);
    eprintln!(
        "hidden: {:?}",
        hidden
            .root
            .iter()
            .map(|stat| (p.name(&stat.action), stat.visits, stat.mean))
            .collect::<Vec<_>>()
    );
    let first = hidden.recommended.unwrap();
    assert_eq!(p.name(&first), "POMMEL_STRIKE");

    // Closed loop: after the first decision, replan with the hidden search
    // at every decision. No line here places a card on the draw pile, so no
    // position becomes known and `DrawKnowledge::none()` is the player's
    // knowledge at every step.
    let horizon = IsmctsParams::new(Information::Hidden, 0, 0).horizon;
    let worlds = p.worlds();
    let mut total = 0.0;
    let mut events = Vec::new();
    for world in &worlds {
        let mut state = engine::apply_action_into(world, &p.catalog, &first, &mut events).unwrap();
        let mut depth = 1;
        while !stopped(&state, depth, horizon) {
            let action = p
                .search(&state, Information::Hidden, 4_000)
                .recommended
                .unwrap();
            state = engine::apply_action_into(&state, &p.catalog, &action, &mut events).unwrap();
            depth += 1;
        }
        let value = reward(
            &state,
            enemy_hp(&p.root),
            BattlewornObjective::of_root(&p.root),
        );
        eprintln!(
            "closed loop, top {}: won {} reward {value}",
            p.top(world),
            won(&state)
        );
        total += value / worlds.len() as f64;
    }
    assert!((total - 0.2).abs() < 1e-12, "closed-loop mean {total}");
}

/// At a large budget, the hidden search's root value for its best action
/// matches exhaustive expectimax, on the fusion position (2 unknown cards, 2
/// orders) and on the same hand over 4 unknown cards (Anger, Bludgeon, Wound,
/// Strike: 24 orders). The search also builds exactly the oracle's
/// information sets: 14 and 49 nodes.
///
/// The tolerance is the reported 95% interval **plus 0.01**, because that
/// interval is not calibrated for this comparison. A root mean is a visit
/// average, and UCB's exploration of worse lines below the root biases it
/// low; the interval treats visits as independent draws of a fixed policy
/// and does not cover the bias. Release runs, seeds 1-8 per budget, both
/// positions: the interval missed the oracle in 5 of 16 runs at 60,000
/// iterations (worst 0.1883 ± 0.0071 against 0.2), 6 of 16 at 100,000, 3 of
/// 16 at 200,000 and 3 of 16 at 400,000 (worst −0.0257 ± 0.0025 against
/// −0.0229). Every miss was low, never high, and the excess over the interval
/// was at most 0.0047 at 60,000. Seed 1 at 60,000 (this test) lands inside
/// the bare interval on both positions: 0.1956 ± 0.0070 and −0.0284 ±
/// 0.0065 against −0.0229.
#[test]
fn the_root_value_matches_exhaustive_expectimax() {
    for (draw, nodes) in [
        (&["ANGER", "BLUDGEON"][..], 14),
        (&["ANGER", "BLUDGEON", "WOUND", "STRIKE_IRONCLAD"][..], 49),
    ] {
        let p = fusion_position(draw);
        let exact = p.expectimax();
        let best = argmax(&exact);
        let oracle = value_of(&exact, best);
        let result = p.search(&p.root, Information::Hidden, 60_000);
        let stat = &result.root[0];
        eprintln!(
            "{draw:?}: expectimax {exact:?}; ismcts {} {:.4} ± {:.4}, {} nodes",
            p.name(&stat.action),
            stat.mean,
            stat.ci95,
            result.nodes
        );
        assert_eq!(p.name(&stat.action), best, "{draw:?}");
        assert!(
            (stat.mean - oracle).abs() <= stat.ci95 + 0.01,
            "{draw:?}: {} ± {} against the oracle's {oracle}",
            stat.mean,
            stat.ci95
        );
        assert_eq!(result.nodes, nodes, "{draw:?}");
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }
}
