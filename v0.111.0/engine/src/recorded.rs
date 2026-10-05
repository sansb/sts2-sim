//! The player's recorded line, resolved to engine actions (#3578 slice B1).
//!
//! A decoded combat replay ([`crate::mcr::decode`]) records what the player
//! did as the GAME names it: a card by its combat-card index, a target by its
//! creation ordinal, a choice by display index or physical card. This module
//! resolves every recorded input to the one engine action it denotes, with no
//! search, applies it, and reports the line: the wire actions, the engine's
//! differential digest after each, and the terminal state.
//!
//! It is a port of `replay_recorded_line` in `tools/eval_suite.py`, the
//! measurement form of the production review's `rust_replay.recorded_witness`
//! (`python/rust_replay.py`), and it changes no engine semantics: every
//! transition, option enumerator and legality answer is the engine's existing
//! one. The identity rules and the native facts they rest on are documented
//! where the Python states them (issue numbers are kept here so each rule can
//! be traced); the IL citations for the choose-a-card screens and Knowledge
//! Demon's curse are on `_CHOOSE_A_CARD_OPTIONS` and
//! `_knowledge_demon_curse_options` in `rust_replay.py`. No IL was re-read for
//! this port. Its acceptance bar is the eval fixtures: the same actions, step
//! digests and terminal the census wrote to each `human_line.json`
//! (`tools/recorded_parity.py`).
//!
//! NOT ported here (slice B2): the per-action comparison against the capture's
//! own native checkpoints (`rust_replay.NativeChecks`), and with it the
//! truncated-capture classification. A line that stops before the engine's
//! combat ends is `combat_not_complete`.
//!
//! An input with no exact counterpart is a named divergence, never a guess
//! (I5).

use crate::boundary::HotBoundary;
use crate::canonical::CanonicalStateV2;
use crate::catalog::Catalog;
use crate::engine::{self, Action, SelectionAnswer};
use crate::exact_solve_v1::{ExactSolveActionV1, ExactSolveSelectionV1};
use crate::hot::HotState;
use crate::rng::Xoshiro256StarStar;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The recorded inputs a line may carry (`RECORDED_INPUT_BUDGET`).
const RECORDED_INPUT_BUDGET: usize = 2000;
/// Search nodes the deal reading may visit before it refuses
/// (`DEAL_SEARCH_BUDGET`).
const DEAL_SEARCH_BUDGET: usize = 100_000;

/// One recorded input with no exact engine counterpart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiverged {
    /// The stable class: `entry_deal`, `card_identity`, `not_legal`, ...
    pub check: &'static str,
    pub detail: String,
    /// Actions applied before the divergence.
    pub step: usize,
    /// The engine's turn there.
    pub turn: i64,
}

/// A resolved line.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedLine {
    pub actions: Vec<ExactSolveActionV1>,
    /// The engine's differential digest after each action.
    pub step_digests: Vec<String>,
    /// `{hp, turn, over, won}`, the fixture tree's `terminal` shape.
    pub terminal: Value,
    /// Plays whose recorded uid was not the representative `legal` offers.
    pub nonrepresentative_uids: usize,
}

/// Limits a caller with a CPU budget puts on one line (#3578 slice C).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordedOptions {
    /// Refuse a physical-card selection, as `selection_budget`, when the
    /// engine offers more answers than this. `None` is no limit.
    ///
    /// `engine::recorded_selection_answer` decodes every offered answer. For
    /// a card-play selection that is one enumeration (#3581: it used to be
    /// one per answer, and a 14,000-answer surface took minutes), but the
    /// work still grows with the answer count, and a Function's budget is
    /// small.
    pub max_selection_answers: Option<usize>,
}

type Deal = Result<Vec<u32>, String>;

fn turn_of(state: &Value) -> i64 {
    state["player"]["turn"].as_i64().unwrap_or(1)
}

fn cards_of<'a>(state: &'a Value, pile: &str) -> &'a [Value] {
    state["piles"][pile].as_array().map_or(&[], Vec::as_slice)
}

fn all_cards(state: &Value) -> impl Iterator<Item = (&String, &Value)> {
    state["piles"]
        .as_object()
        .into_iter()
        .flat_map(|piles| piles.iter())
        .flat_map(|(name, pile)| {
            pile.as_array()
                .into_iter()
                .flatten()
                .map(move |c| (name, c))
        })
}

fn uid_of(card: &Value) -> Option<u32> {
    card["uid"].as_u64().and_then(|uid| u32::try_from(uid).ok())
}

fn has_relic(root: &Value, relic: &str) -> bool {
    root["player"]["relics_entering"]
        .as_array()
        .is_some_and(|relics| relics.iter().any(|r| r == relic))
}

/// One recorded deck row: `(id, upgrade, enchantment)`.
#[derive(Debug, Clone)]
struct Instance {
    id: String,
    upgrade: i64,
    enchantment: Option<(String, i64)>,
}

/// A root card in a deal queue; `None` is a vanished card's placeholder.
type Queued<'a> = (u32, Option<&'a Value>);

/// Whether a root card IS the recorded deck card, by identity. The id must
/// agree; the upgrade may only have grown (an opening that upgrades the dealt
/// hand changes the payload, never the physical card). Deliberately never the
/// enchantment: a root's card does not carry every enchantment the capture
/// records (#3329).
fn same_card(card: Option<&Value>, recorded: &Instance) -> bool {
    let Some(card) = card else { return true };
    card["id"].as_str() == Some(recorded.id.as_str())
        && card["upgrade"].as_i64().unwrap_or(0) >= recorded.upgrade
}

/// Whether a root card's own enchantment matches the recorded one. Used only
/// to break an id/upgrade tie across queue heads (#3329).
fn same_enchantment(card: Option<&Value>, recorded: &Option<(String, i64)>) -> bool {
    let enchantment = card
        .map(|card| &card["enchantment"])
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty());
    match (enchantment, recorded) {
        (None, None) => true,
        (Some(e), Some((id, level))) => {
            e.len() == 2 && e[0].as_str() == Some(id.as_str()) && e[1].as_i64() == Some(*level)
        }
        _ => false,
    }
}

fn is_imbued(card: &Value) -> bool {
    card["enchantment"]
        .as_array()
        .and_then(|e| e.first())
        .is_some_and(|id| id == "IMBUED")
}

/// Master-deck row of each combat-card instance: instance k is row `[k]`
/// (`mcr_native.first_cycle_rows_from_replay`). The game's combat-start
/// shuffle is a Fisher-Yates whose swaps depend only on the list length, so
/// shuffling the row ordinals applies the deal's permutation. The Shuffle
/// stream is taken as the capture RECORDS it (state words and counter), never
/// derived from the seed.
fn first_cycle_rows(replay: &Value) -> Result<(Vec<usize>, &Vec<Value>), String> {
    let players = replay["run"]["players"]
        .as_array()
        .ok_or("MCR replay has no players")?;
    if players.len() != 1 {
        return Err(format!(
            "MCR replay has {} players; one is supported",
            players.len()
        ));
    }
    let deck = players[0]["deck"]
        .as_array()
        .ok_or("MCR replay player has no deck")?;
    let rng = &replay["run"]["rng"];
    let words = rng["states"]["Shuffle"].as_array().filter(|w| w.len() == 4);
    let counter = rng["counters"]["Shuffle"].as_u64();
    let (Some(words), Some(counter)) = (words, counter) else {
        return Err("MCR replay carries no Shuffle stream state".to_owned());
    };
    let mut state = [0u64; 4];
    for (slot, word) in state.iter_mut().zip(words) {
        *slot = word
            .as_u64()
            .ok_or("MCR replay Shuffle state is malformed")?;
    }
    let mut rng = Xoshiro256StarStar {
        words: state,
        counter,
    };
    let mut rows: Vec<usize> = (0..deck.len()).collect();
    rng.shuffle(&mut rows).map_err(str::to_owned)?;
    Ok((rows, deck))
}

fn first_cycle_instances(replay: &Value) -> Result<(Vec<usize>, Vec<Instance>), String> {
    let (rows, deck) = first_cycle_rows(replay)?;
    let instances = rows
        .iter()
        .map(|&row| {
            let card = &deck[row];
            let id = card["id"].as_str().unwrap_or("");
            let upgrade = [&card["upgrade_level"], &card["current_upgrade_level"]]
                .into_iter()
                .filter_map(Value::as_i64)
                .find(|&level| level != 0)
                .unwrap_or(0);
            let enchantment = card["enchantment"].as_object().map(|found| {
                (
                    found
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    found.get("level").and_then(Value::as_i64).unwrap_or(0),
                )
            });
            Instance {
                id: id.strip_prefix("CARD.").unwrap_or(id).to_owned(),
                upgrade,
                enchantment,
            }
        })
        .collect();
    Ok((rows, instances))
}

/// Every way (at most two) to read `instances` off the queue heads
/// (`_interleavings`). A card that could be the head of two queues is first
/// narrowed by enchantment; if more than one head is left, each is tried, and
/// the caller refuses unless exactly one reading completes (#3152, #3329).
fn interleavings(
    queues: &[Vec<Queued<'_>>],
    instances: &[Instance],
) -> Result<Vec<Vec<u32>>, String> {
    struct Walk<'a, 'b> {
        queues: &'a [Vec<Queued<'b>>],
        instances: &'a [Instance],
        heads: Vec<usize>,
        uids: Vec<u32>,
        solutions: Vec<Vec<u32>>,
        visited: usize,
    }
    impl Walk<'_, '_> {
        fn walk(&mut self, index: usize) -> Result<(), String> {
            self.visited += 1;
            if self.visited > DEAL_SEARCH_BUDGET {
                return Err("recorded initial physical card identities differ".to_owned());
            }
            if index == self.instances.len() {
                self.solutions.push(self.uids.clone());
                return Ok(());
            }
            let recorded = &self.instances[index];
            let mut matched: Vec<usize> = (0..self.queues.len())
                .filter(|&q| {
                    self.queues[q]
                        .get(self.heads[q])
                        .is_some_and(|(_, card)| same_card(*card, recorded))
                })
                .collect();
            if matched.len() > 1 {
                let narrowed: Vec<usize> = matched
                    .iter()
                    .copied()
                    .filter(|&q| {
                        same_enchantment(self.queues[q][self.heads[q]].1, &recorded.enchantment)
                    })
                    .collect();
                if narrowed.len() == 1 {
                    matched = narrowed;
                }
            }
            for q in matched {
                self.uids.push(self.queues[q][self.heads[q]].0);
                self.heads[q] += 1;
                self.walk(index + 1)?;
                self.heads[q] -= 1;
                self.uids.pop();
                if self.solutions.len() > 1 {
                    return Ok(());
                }
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        queues,
        instances,
        heads: vec![0; queues.len()],
        uids: Vec::new(),
        solutions: Vec::new(),
        visited: 0,
    };
    walk.walk(0)?;
    Ok(walk.solutions)
}

/// The uid of each recorded instance, undoing SetupPlayerTurn's rewrite
/// (`_dealt_uids`). The engine numbers the dealt deck in its post-opening pile
/// order (Hand, then Draw), after every IMBUED card moved to the Draw bottom
/// and every remaining Innate card to the front in reversed relative order;
/// the capture's first cycle is the raw shuffle order (#2552). The three
/// subsequences each keep their relative order, so reading the recorded cycle
/// off their heads reconstructs the raw order, and it must be the only
/// complete reading.
fn dealt_uids(root: &Value, instances: &[Instance]) -> Deal {
    let deck_size = instances.len();
    let in_deck = |card: &Value| uid_of(card).is_some_and(|uid| (uid as usize) < deck_size);
    let mut dealt: Vec<Queued<'_>> = cards_of(root, "hand")
        .iter()
        .chain(cards_of(root, "draw"))
        .filter_map(|card| Some((uid_of(card)?, Some(card))))
        .collect();
    // Turn one's AutoPre auto-plays an IMBUED deck card out of the Draw
    // bottom (#3381), so the root holds it in its result pile: put it back at
    // the end of the Draw, in uid order.
    let mut imbued_played: Vec<Queued<'_>> = all_cards(root)
        .filter(|(name, card)| {
            *name != "hand" && *name != "draw" && is_imbued(card) && in_deck(card)
        })
        .filter_map(|(_, card)| Some((uid_of(card)?, Some(card))))
        .collect();
    imbued_played.sort_by_key(|(uid, _)| *uid);
    dealt.extend(imbued_played);
    let innate = match &root["player"]["innate_min_draw"] {
        Value::Null => 0,
        value => value
            .as_u64()
            .map(|n| n as usize)
            .filter(|&n| n <= dealt.len())
            .ok_or("Rust root innate_min_draw is out of range")?,
    };
    if has_relic(root, "RELIC.WHISPERING_EARRING") {
        // Whispering Earring's turn-one loop AutoPlays live Hand cards after
        // the deal (#3414), and a played Power leaves every pile. The deck
        // was numbered in the post-rewrite layout, so the deck cards in uid
        // order ARE that layout; a uid in no pile holds its place as a
        // placeholder that matches whatever the cycle records there.
        let present: BTreeMap<u32, &Value> = all_cards(root)
            .filter(|(_, card)| in_deck(card))
            .filter_map(|(_, card)| Some((uid_of(card)?, card)))
            .collect();
        dealt = (0..deck_size as u32)
            .map(|uid| (uid, present.get(&uid).copied()))
            .collect();
    } else if has_relic(root, "RELIC.JEWELED_MASK") {
        // Jeweled Mask's turn-1 BeforeHandDraw moves one deck Power from
        // mid-Draw to the Hand AFTER the deck was numbered, keeping its uid
        // (#3170): the deck cards in uid order are the layout before the lift.
        dealt.retain(|(uid, _)| (*uid as usize) < deck_size);
        dealt.sort_by_key(|(uid, _)| *uid);
    }
    let innate = innate.min(dealt.len());
    let deck_card = |card: &Queued<'_>| (card.0 as usize) < deck_size;
    let front: Vec<Queued<'_>> = dealt[..innate]
        .iter()
        .rev()
        .copied()
        .filter(deck_card)
        .collect();
    let tail: Vec<Queued<'_>> = dealt[innate..].iter().copied().filter(deck_card).collect();
    let mut seen: Vec<u32> = front.iter().chain(&tail).map(|(uid, _)| *uid).collect();
    seen.sort_unstable();
    if seen != (0..deck_size as u32).collect::<Vec<_>>() {
        return Err("recorded deck size differs from the Rust deal".to_owned());
    }
    let imbued = |card: &Queued<'_>| card.1.is_some_and(is_imbued);
    let mut queues = vec![front];
    queues.push(tail.iter().filter(|c| !imbued(c)).copied().collect());
    if tail.iter().any(imbued) {
        queues.push(tail.iter().filter(|c| imbued(c)).copied().collect());
    }
    let mut solutions = interleavings(&queues, instances)?;
    if solutions.len() != 1 {
        return Err("recorded initial physical card identities differ".to_owned());
    }
    Ok(solutions.remove(0))
}

/// Whether a Thieving Hopper root's uids past the deck are exactly the cards
/// its opening created (`_opening_created_uids_are_exact`).
fn opening_created_uids_are_exact(root: &Value, deck_size: usize) -> bool {
    let Some(next_uid) = root["player"]["next_card_uid"].as_u64().map(|n| n as usize) else {
        return false;
    };
    if next_uid < deck_size {
        return false;
    }
    let mut created: Vec<usize> = all_cards(root)
        .filter_map(|(_, card)| uid_of(card).map(|uid| uid as usize))
        .filter(|&uid| uid >= deck_size)
        .collect();
    created.sort_unstable();
    created == (deck_size..next_uid).collect::<Vec<_>>()
}

/// `combat_card_index -> physical uid`, authenticated against the deal
/// (`_deal_uid_map`). A Thieving Hopper root keeps each deck card's
/// master-deck row as its uid (#2805) and says so by carrying
/// `hopper_master_deck`.
fn deal(root: &Value, replay: &Value) -> Deal {
    let (rows, instances) = first_cycle_instances(replay)?;
    if let Some(master) = root["player"].get("hopper_master_deck") {
        let mut master_rows: Vec<u64> = master
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get(0).and_then(Value::as_u64))
            .collect();
        master_rows.sort_unstable();
        if master_rows != (0..rows.len() as u64).collect::<Vec<_>>()
            || !opening_created_uids_are_exact(root, rows.len())
        {
            return Err("recorded master-deck rows differ from the Rust root".to_owned());
        }
        let cards: BTreeMap<u32, &Value> = all_cards(root)
            .filter_map(|(_, card)| Some((uid_of(card)?, card)))
            .collect();
        for (index, recorded) in instances.iter().enumerate() {
            let held = u32::try_from(rows[index])
                .ok()
                .and_then(|uid| cards.get(&uid));
            if !held.is_some_and(|card| same_card(Some(card), recorded)) {
                return Err("recorded initial physical card identities differ".to_owned());
            }
        }
        return Ok(rows.into_iter().map(|row| row as u32).collect());
    }
    dealt_uids(root, &instances)
}

/// Past the entry deck, instances are creation-ordered and so is the
/// allocator: the j-th card created in combat is uid `len(deck) + j`.
fn uid_map(deck: &[u32], index: &Value) -> Result<u32, String> {
    let index = index
        .as_u64()
        .and_then(|index| u32::try_from(index).ok())
        .ok_or("invalid recorded physical card index")?;
    Ok(deck.get(index as usize).copied().unwrap_or(index))
}

struct Line {
    catalog: Catalog,
    state: HotState,
    events: Vec<engine::Event>,
}

impl Line {
    fn project(&self) -> Result<(CanonicalStateV2, Value), String> {
        let document =
            HotBoundary::try_to_canonical(&self.state, &self.catalog).map_err(|e| e.to_string())?;
        let value = serde_json::to_value(&document).map_err(|e| e.to_string())?;
        Ok((document, value))
    }

    fn legal(&self) -> Vec<ExactSolveActionV1> {
        engine::legal_actions(&self.state, &self.catalog)
            .into_iter()
            .map(Into::into)
            .collect()
    }
}

/// Whether a recorded play names an action `legal` offers
/// (`_play_is_offered`). `legal` offers ONE representative uid per group of
/// payload-identical Hand cards, while `apply` accepts the physical copy the
/// player clicked (#2473). The replay must apply that exact copy, so a
/// recorded uid absent from the list is accepted only when an offered play
/// differs from it solely by a Hand uid whose projected card is identical
/// apart from `uid`.
fn play_is_offered(
    before: &Value,
    legal: &[ExactSolveActionV1],
    wire: &ExactSolveActionV1,
) -> bool {
    if legal.contains(wire) {
        return true;
    }
    let ExactSolveActionV1::Play {
        uid,
        target,
        selection,
    } = wire
    else {
        return false;
    };
    let hand: BTreeMap<u32, Value> = cards_of(before, "hand")
        .iter()
        .filter_map(|card| {
            let mut payload = card.clone();
            payload.as_object_mut()?.remove("uid");
            Some((uid_of(card)?, payload))
        })
        .collect();
    let same = |offered: u32, recorded: u32| {
        offered == recorded
            || matches!((hand.get(&offered), hand.get(&recorded)), (Some(a), Some(b)) if a == b)
    };
    legal.iter().any(|offered| match offered {
        ExactSolveActionV1::Play {
            uid: o_uid,
            target: o_target,
            selection: o_selection,
        } => {
            o_target == target
                && same(*o_uid, *uid)
                && match (o_selection, selection) {
                    (None, None) => true,
                    (Some(o), Some(r)) => same(*o, *r),
                    _ => false,
                }
        }
        _ => false,
    })
}

fn option_index(index: u32) -> ExactSolveActionV1 {
    ExactSolveActionV1::Select {
        answer: ExactSolveSelectionV1::OptionIndex { index },
    }
}

fn select(answer: SelectionAnswer) -> ExactSolveActionV1 {
    Action::Select { answer }.into()
}

/// The pending slot holding a choose-a-card screen's offered cards, and
/// whether the screen can be skipped (`_CHOOSE_A_CARD_OPTIONS`,
/// `_CHOOSE_A_CARD_SKIPPABLE`; the IL is cited there).
fn choose_a_card(kind: &str) -> Option<(usize, bool)> {
    match kind {
        "discovery_select" | "splash_select" | "quasar_select" => Some((3, true)),
        "abundance_select" => Some((3, false)),
        "generation_potion_select" => Some((2, true)),
        _ => None,
    }
}

fn one_display_index(result: &Value) -> Result<i64, String> {
    match result["indexes"].as_array().map(Vec::as_slice) {
        Some([index]) => index
            .as_i64()
            .ok_or_else(|| "recorded selection needs one display index".to_owned()),
        _ => Err("recorded selection needs one display index".to_owned()),
    }
}

/// One recorded `PlayerChoice`, resolved (`rust_replay._selection`, and
/// `_multi_card_selection` for more than one physical card).
fn selection(
    line: &Line,
    before: &Value,
    result: &Value,
    deck: &[u32],
    options: &RecordedOptions,
) -> Result<ExactSolveActionV1, (&'static str, String)> {
    selection_answer(line, before, result, deck, options).map_err(|detail| {
        let check = if detail.starts_with(OVER_BUDGET) {
            "selection_budget"
        } else {
            "selection"
        };
        (check, detail)
    })
}

/// The detail a selection over [`RecordedOptions::max_selection_answers`]
/// starts with; [`selection`] names it `selection_budget`.
const OVER_BUDGET: &str = "recorded selection is over the answer budget";

fn selection_answer(
    line: &Line,
    before: &Value,
    result: &Value,
    deck: &[u32],
    options: &RecordedOptions,
) -> Result<ExactSolveActionV1, String> {
    const NO_ANSWER: &str = "recorded selection has no unique supported Rust answer";
    let legal = line.legal();
    let pending = before["player"]["pending"]
        .as_array()
        .filter(|p| !p.is_empty());
    let kind = result["type"].as_str().unwrap_or("");
    // Knowledge Demon's curse is the monster's modal, parked on the enemy
    // turn: the player's own `pending` is empty (#3156). canSkip=false.
    let curse = before["player"]["enemy_pending"]
        .as_array()
        .filter(|enemy| pending.is_none() && enemy.first().is_some_and(|tag| tag == "KD_CURSE"));
    if let Some(enemy) = curse {
        let options = enemy
            .get(1)
            .and_then(Value::as_array)
            .ok_or("Knowledge Demon curse choice has no option list")?;
        if kind != "Index" {
            return Err("recorded Knowledge Demon curse choice is not an Index".to_owned());
        }
        let index = one_display_index(result)?;
        if index < 0 || index as usize >= options.len() {
            return Err("recorded Knowledge Demon curse index outside the offered set".to_owned());
        }
        let wire = option_index(index as u32);
        return if legal.contains(&wire) {
            Ok(wire)
        } else {
            Err("recorded Knowledge Demon curse choice is not offered by Rust".to_owned())
        };
    }
    let pending_kind = pending.and_then(|p| p[0].as_str()).unwrap_or("");
    if kind == "Index"
        && (choose_a_card(pending_kind).is_some() || pending_kind == "generation_relic_select")
    {
        let mut index = one_display_index(result)?;
        if let Some((slot, skippable)) = choose_a_card(pending_kind) {
            let options = pending
                .and_then(|p| p.get(slot))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if index == -1 && skippable {
                // A skip is recorded as [-1]; Rust offers it as ordinal len(options).
                index = options as i64;
            } else if index < 0 || index as usize >= options {
                return Err("recorded selection index outside the offered cards".to_owned());
            }
        }
        let wire = u32::try_from(index)
            .map(option_index)
            .map_err(|_| NO_ANSWER.to_owned())?;
        if legal.contains(&wire) {
            return Ok(wire);
        }
        return Err(NO_ANSWER.to_owned());
    }
    if kind == "CombatCard" {
        let indexes = result["combat_card_indexes"]
            .as_array()
            .filter(|indexes| !indexes.is_empty())
            .ok_or("recorded selection needs one physical card")?;
        let uids = indexes
            .iter()
            .map(|index| uid_map(deck, index))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(budget) = options.max_selection_answers {
            let offered = legal
                .iter()
                .filter(|action| matches!(action, ExactSolveActionV1::Select { .. }))
                .count();
            if offered > budget {
                return Err(format!(
                    "{OVER_BUDGET}: {offered} answers offered, {budget} allowed"
                ));
            }
        }
        // The engine runs the rule in-process (`recorded_selection_answer`,
        // #3125): each offered answer decoded through the transition's own
        // enumerator, kept only if it names exactly these uids in this order
        // and applies, refused when two do. It also names Ashwater's and
        // Gambler's Brew's ordered answers, which `legal` does not offer.
        return match engine::recorded_selection_answer(&line.state, &line.catalog, &uids) {
            Ok(Some(answer)) => Ok(select(answer)),
            Ok(None) => Err(NO_ANSWER.to_owned()),
            Err(error) => Err(error.to_string()),
        };
    }
    Err(NO_ANSWER.to_owned())
}

/// Resolve every recorded input of `replay` (a decoded `.mcr`) to an engine
/// action from `entry` and apply it, in order.
///
/// # Errors
///
/// [`LineDiverged`] when an input cannot be reproduced exactly, the root is
/// refused, or the inputs end before the engine's combat does.
pub fn recorded_line(
    entry: &CanonicalStateV2,
    replay: &Value,
) -> Result<RecordedLine, LineDiverged> {
    recorded_line_with(entry, replay, &RecordedOptions::default())
}

/// [`recorded_line`] under a caller's limits.
///
/// # Errors
///
/// As [`recorded_line`], plus `selection_budget` when a selection offers more
/// answers than `options` allows.
pub fn recorded_line_with(
    entry: &CanonicalStateV2,
    replay: &Value,
    options: &RecordedOptions,
) -> Result<RecordedLine, LineDiverged> {
    let root = serde_json::to_value(entry).expect("canonical state serializes");
    let diverged_at = |check: &'static str, detail: String, step: usize, turn: i64| LineDiverged {
        check,
        detail: detail.chars().take(220).collect(),
        step,
        turn,
    };
    let root_turn = turn_of(&root);
    let events = replay["events"]
        .as_array()
        .filter(|e| !e.is_empty() && e.len() <= RECORDED_INPUT_BUDGET);
    let Some(events) = events else {
        return Err(diverged_at(
            "recorded_input_budget",
            "recorded input count outside the replay budget".to_owned(),
            0,
            root_turn,
        ));
    };
    let deck =
        deal(&root, replay).map_err(|detail| diverged_at("entry_deal", detail, 0, root_turn))?;
    let refused = |detail: String| diverged_at("root_refused", detail, 0, root_turn);
    entry
        .validate_schema()
        .map_err(|e| refused(format!("{e:?}")))?;
    let catalog = HotBoundary::catalog_from_canonical(entry).map_err(|e| refused(e.to_string()))?;
    let state = HotBoundary::from_canonical(entry, &catalog).map_err(|e| refused(e.to_string()))?;
    engine::admit(entry, &state, &catalog).map_err(|e| refused(e.to_string()))?;
    let mut line = Line {
        catalog,
        state,
        events: Vec::new(),
    };

    let mut actions: Vec<ExactSolveActionV1> = Vec::new();
    let mut digests = Vec::new();
    let mut nonrepresentative = 0;
    let mut consumed = BTreeSet::new();
    let mut before = root;
    for (index, event) in events.iter().enumerate() {
        if consumed.contains(&index) {
            continue;
        }
        let diverged = |check: &'static str, detail: String| {
            diverged_at(check, detail, actions.len(), turn_of(&before))
        };
        let action = &event["action"];
        let kind = action["type"].as_str().unwrap_or("");
        let event_type = event["event_type"].as_str().unwrap_or("");
        let is_input = matches!(
            kind,
            "NetPlayCardAction" | "NetUsePotionAction" | "NetEndPlayerTurnAction"
        );
        if before["player"]["over"] == true {
            if is_input {
                return Err(diverged("premature_combat_end", kind.to_owned()));
            }
            continue;
        }
        let wire = if matches!(kind, "NetPlayCardAction" | "NetUsePotionAction") {
            let target = match (action["target_id"].as_u64(), &action["target_player_id"]) {
                (Some(tid), player) if tid != 0 && !player.is_u64() => {
                    // A target is the monster's CREATION ordinal, never its slot.
                    let matches: Vec<usize> = before["monsters"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .filter(|(_, m)| m["uid"].as_u64().unwrap_or(0) == tid - 1)
                        .map(|(i, _)| i)
                        .collect();
                    match matches.as_slice() {
                        [slot] => u8::try_from(*slot).ok(),
                        _ => None,
                    }
                    .map(Some)
                    .ok_or_else(|| {
                        diverged(
                            "target_identity",
                            format!("recorded target {tid} at input {index}"),
                        )
                    })?
                }
                _ => None,
            };
            let mut wire = if kind == "NetPlayCardAction" {
                let uid = uid_map(&deck, &action["combat_card_index"])
                    .map_err(|detail| diverged("card_identity", detail))?;
                let recorded = action["card_id"].as_str().unwrap_or("");
                let recorded = recorded.strip_prefix("CARD.").unwrap_or(recorded);
                let held = cards_of(&before, "hand")
                    .iter()
                    .find(|card| uid_of(card) == Some(uid));
                if !held.is_some_and(|card| card["id"].as_str() == Some(recorded)) {
                    return Err(diverged(
                        "card_identity",
                        format!("recorded card identity mismatch at input {index}"),
                    ));
                }
                ExactSolveActionV1::Play {
                    uid,
                    target,
                    selection: None,
                }
            } else {
                let slot = action["potion_index"]
                    .as_u64()
                    .and_then(|slot| u8::try_from(slot).ok());
                let Some(slot) = slot else {
                    return Err(diverged(
                        "not_legal",
                        format!("{kind} is not legal at input {index}"),
                    ));
                };
                ExactSolveActionV1::Potion { slot, target }
            };
            let legal = line.legal();
            let mut offered = if kind == "NetPlayCardAction" {
                play_is_offered(&before, &legal, &wire)
            } else {
                legal.contains(&wire)
            };
            if !offered && kind == "NetPlayCardAction" {
                // An immediate one-card choice belongs to the play wire
                // (Burning Pact); consume only its adjacent recorded choice.
                let following = events.get(index + 1);
                let selected = following
                    .filter(|next| next["event_type"] == "PlayerChoice")
                    .and_then(|next| next["result"]["combat_card_indexes"].as_array())
                    .filter(|selected| selected.len() == 1);
                if let (Some(selected), ExactSolveActionV1::Play { uid, target, .. }) =
                    (selected, &wire)
                {
                    let picked = uid_map(&deck, &selected[0])
                        .map_err(|detail| diverged("card_identity", detail))?;
                    let combined = ExactSolveActionV1::Play {
                        uid: *uid,
                        target: *target,
                        selection: Some(picked),
                    };
                    if play_is_offered(&before, &legal, &combined) {
                        wire = combined;
                        consumed.insert(index + 1);
                        offered = true;
                    }
                }
            }
            if !offered {
                return Err(diverged(
                    "not_legal",
                    format!("{kind} is not legal at input {index}"),
                ));
            }
            if kind == "NetPlayCardAction" && !legal.contains(&wire) {
                nonrepresentative += 1;
            }
            wire
        } else if kind == "NetEndPlayerTurnAction" {
            if action["turn_number"].as_i64() != Some(turn_of(&before)) {
                return Err(diverged(
                    "turn_number",
                    format!(
                        "recorded turn {} at engine turn {}",
                        action["turn_number"],
                        turn_of(&before)
                    ),
                ));
            }
            ExactSolveActionV1::End
        } else if event_type == "PlayerChoice" {
            selection(&line, &before, &event["result"], &deck, options)
                .map_err(|(check, detail)| diverged(check, detail))?
        } else if matches!(event_type, "HookAction" | "ResumeAction")
            || kind == "NetReadyToBeginEnemyTurnAction"
        {
            continue;
        } else {
            return Err(diverged(
                "unsupported_event",
                format!("{event_type}/{kind} at input {index}"),
            ));
        };
        let action: Action = wire
            .clone()
            .try_into()
            .map_err(|detail: String| diverged("rust_apply_refused", detail))?;
        let next = engine::apply_action_into(&line.state, &line.catalog, &action, &mut line.events)
            .map_err(|error| diverged("rust_apply_refused", error.to_string()))?;
        line.state = next;
        let (document, value) = line
            .project()
            .map_err(|detail| diverged("rust_apply_refused", detail))?;
        before = value;
        actions.push(wire);
        digests.push(document.differential_digest());
    }
    if before["player"]["over"] != true {
        return Err(diverged_at(
            "combat_not_complete",
            "recorded inputs ended before the engine's combat".to_owned(),
            actions.len(),
            turn_of(&before),
        ));
    }
    let hp = before["player"]["hp"].as_i64().unwrap_or(0);
    Ok(RecordedLine {
        actions,
        step_digests: digests,
        terminal: json!({"hp": hp, "turn": turn_of(&before), "over": true, "won": hp > 0}),
        nonrepresentative_uids: nonrepresentative,
    })
}

/// What a capture says about WHICH fight it records, for a caller that has to
/// pair captures with a run's fights (`review_provenance.analyze_bundle`'s
/// association inputs): the run's seed and start time, the 0-based history
/// depth of the node being fought, the encounter the act's pools select for
/// each node type, and the monsters in the first native checkpoint.
///
/// `history_depth` is null, with `history_depth_issue` naming why, when the
/// capture's own map history is malformed or inconsistent (`_history_depth`).
#[must_use]
pub fn capture_summary(replay: &Value) -> Value {
    let run = &replay["run"];
    let depth = history_depth(run);
    let act = run["current_act_index"].as_u64().unwrap_or(u64::MAX) as usize;
    let rooms = &run["acts"][act]["rooms"];
    let named = |id: Option<&str>| {
        id.filter(|id| !id.is_empty())
            .map_or(Value::Null, |id| json!(format!("ENCOUNTER.{id}")))
    };
    let selected = |ids: &str, visited: &str| {
        named(
            rooms[visited]
                .as_u64()
                .and_then(|index| rooms[ids].get(index as usize))
                .and_then(Value::as_str),
        )
    };
    let boss = named(match rooms["bosses_visited"].as_u64() {
        Some(0) => rooms["boss_id"].as_str(),
        Some(1) => rooms["second_boss_id"].as_str(),
        _ => None,
    });
    let monsters: Vec<Value> = replay["checksums"][0]["full_state"]["creatures"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|creature| creature["monster_id"].as_str())
        .map(|id| json!(id.strip_prefix("MONSTER.").unwrap_or(id)))
        .collect();
    json!({
        "version": replay["version"],
        "seed": run["rng"]["seed"],
        "start_time": run["start_time"],
        "players": run["players"].as_array().map_or(0, Vec::len),
        "history_depth": depth.as_ref().ok(),
        "history_depth_issue": depth.as_ref().err(),
        "selected_encounter": {
            "monster": selected("normal_encounter_ids", "normal_encounters_visited"),
            "elite": selected("elite_encounter_ids", "elites_visited"),
            "boss": boss,
        },
        "monsters": monsters,
        "events": replay["events"].as_array().map_or(0, Vec::len),
    })
}

/// The 0-based node index a capture was taken at (`_history_depth`): the
/// number of completed history entries, which must equal the prior acts'
/// entries plus the current act's visited coordinates, less the node being
/// fought.
fn history_depth(run: &Value) -> Result<usize, &'static str> {
    const INVALID: &str = "history_depth_invalid";
    let history = run["map_point_history"].as_array().ok_or(INVALID)?;
    let acts: Vec<usize> = history
        .iter()
        .map(|act| act.as_array().map(Vec::len))
        .collect::<Option<_>>()
        .ok_or(INVALID)?;
    let depth: usize = acts.iter().sum();
    let act_index = match &run["current_act_index"] {
        Value::Null => 0,
        value => value.as_u64().ok_or(INVALID)? as usize,
    };
    let visited = run["visited_map_coords"]
        .as_array()
        .filter(|visited| !visited.is_empty())
        .ok_or(INVALID)?;
    let prior: usize = acts.iter().take(act_index).sum();
    if depth != prior + visited.len() - 1 {
        return Err("history_depth_inconsistent");
    }
    Ok(depth)
}
