//! The `Shuffle` counter entering a fight, replayed from the raw `.run`.
//!
//! A port of `replay_fight.load_combats`, `entry_deck`, `fight_consumption`,
//! `cycle_leavers` and the counter half of `predict`. The model, as that
//! module states it:
//!
//! * the entry deck is the final deck array filtered to
//!   `floor_added_to_deck < floor`, plus the cards the per-node
//!   `cards_removed` / `cards_transformed` logs say were purged later,
//!   re-inserted after their last same-`(id, floor)` sibling;
//! * each prior fight shuffles its entry deck once (`n - 1` draws;
//!   `CardPile::RandomizeOrderInternal` `0x11ea84` is one `UnstableShuffle`,
//!   see [`crate::entry::opening::shuffle`]), draws five a turn for
//!   `turns_taken` turns, and reshuffles its discard (`len - 1` draws) whenever
//!   the draw pile runs out. Ascender's Bane leaves the cycle at the end of
//!   the turn it is drawn; every other card is discarded.
//!
//! Plays, exhausts, retention and draw effects are unrecorded, so a deck that
//! holds any card whose play can change the cycle makes the counter a
//! baseline (the `cycle_leavers` caveat), as does an event-node combat.
//!
//! # The seed (the one departure)
//!
//! The replay runs on the `Shuffle` stream as v0.111.0 seeds it:
//! [`crate::rng::run_stream_at_zero`]`(seed, "shuffle")`, i.e.
//! `RunRngSet::.ctor` (RVA `0x4de0c`) storing the XxHash64
//! `GetDeterministicHashCode` of the seed string (IL_0060-IL_0067; the `"old"`
//! prefix escape at IL_0024-IL_0059), then `get_Shuffle` (`0x4dda3`,
//! `GetRng(1)`) through `CreateRng` (`0x4df00`). The Python predictor passed
//! no build and so replayed the v0.108 seeding; see the module doc of
//! [`super`].

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::RunCountersRefusal;
use crate::rng::{Xoshiro256StarStar, run_stream_at_zero};

/// `replay_fight.ETHEREAL_UNPLAYABLE`.
const ETHEREAL_UNPLAYABLE: &str = "CARD.ASCENDERS_BANE";
/// `replay_fight.DRAWS_PER_TURN`.
const DRAWS_PER_TURN: usize = 5;

/// One combat node, as `load_combats` lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Combat {
    pub node_index: usize,
    pub floor: i64,
    /// `room.get("model_id", "?")`.
    pub encounter: String,
    /// `room.get("turns_taken", -1)`; `None` when the key holds `null`.
    pub turns: Option<i64>,
    /// `map_point_type == "unknown"`.
    pub event: bool,
}

/// A card in an entry deck: the deck array's rows always carry a floor; a
/// re-inserted removal row may not.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Card {
    id: String,
    floor: Option<i64>,
}

/// A removal event: `(node index, removed row)`.
#[derive(Clone, Debug)]
struct Removal {
    node: usize,
    id: Option<String>,
    /// `rc.get("floor_added_to_deck", 1)`, with the key's presence kept.
    floor: Option<i64>,
}

impl Removal {
    fn floor_or_one(&self) -> i64 {
        self.floor.unwrap_or(1)
    }
}

/// What `load_combats` returns, minus the seed string (kept alongside).
#[derive(Clone, Debug)]
pub struct Combats {
    pub seed: String,
    deck: Vec<Card>,
    pub combats: Vec<Combat>,
    removals: Vec<Removal>,
}

fn run_malformed(path: impl Into<String>, detail: impl Into<String>) -> RunCountersRefusal {
    RunCountersRefusal::MalformedHistory {
        path: path.into(),
        detail: detail.into(),
    }
}

fn run_integer(value: &Value, path: &str) -> Result<i64, RunCountersRefusal> {
    if value.is_boolean() {
        return Err(run_malformed(path, "expected an integer, found a boolean"));
    }
    value
        .as_i64()
        .ok_or_else(|| run_malformed(path, "expected an integer"))
}

/// Python truthiness of a JSON value, for `room.get("turns_taken")`.
fn python_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

const COMBAT_ROOM_TYPES: [&str; 3] = ["monster", "elite", "boss"];

fn removal(row: &Value, node: usize, path: &str) -> Result<Removal, RunCountersRefusal> {
    let object = row
        .as_object()
        .ok_or_else(|| run_malformed(path, "expected an object"))?;
    let id = match object.get("id") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| run_malformed(format!("{path}.id"), "expected a string"))?
                .to_owned(),
        ),
    };
    let floor = match object.get("floor_added_to_deck") {
        None => None,
        Some(value) => Some(run_integer(value, &format!("{path}.floor_added_to_deck"))?),
    };
    Ok(Removal { node, id, floor })
}

/// `replay_fight.load_combats`, over the run already checked single-player.
pub fn load_combats(run: &Value) -> Result<Combats, RunCountersRefusal> {
    let seed = run
        .get("seed")
        .and_then(Value::as_str)
        .ok_or_else(|| run_malformed("$.run.seed", "expected a string"))?
        .to_owned();
    let deck_rows = run["players"][0]
        .get("deck")
        .and_then(Value::as_array)
        .ok_or_else(|| run_malformed("$.run.players[0].deck", "expected a list"))?;
    let mut deck = Vec::with_capacity(deck_rows.len());
    for (i, row) in deck_rows.iter().enumerate() {
        let path = format!("$.run.players[0].deck[{i}]");
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| run_malformed(format!("{path}.id"), "expected a string"))?;
        let floor = run_integer(
            row.get("floor_added_to_deck")
                .ok_or_else(|| run_malformed(format!("{path}.floor_added_to_deck"), "missing"))?,
            &format!("{path}.floor_added_to_deck"),
        )?;
        deck.push(Card {
            id: id.to_owned(),
            floor: Some(floor),
        });
    }

    let acts = run
        .get("map_point_history")
        .and_then(Value::as_array)
        .ok_or_else(|| run_malformed("$.run.map_point_history", "expected a list"))?;
    let mut nodes = Vec::new();
    for (a, act) in acts.iter().enumerate() {
        let points = act.as_array().ok_or_else(|| {
            run_malformed(format!("$.run.map_point_history[{a}]"), "expected a list")
        })?;
        nodes.extend(points.iter());
    }

    let mut combats = Vec::new();
    let mut removals = Vec::new();
    for (i, pt) in nodes.iter().enumerate() {
        let path = format!("$.run.nodes[{i}]");
        let stats = pt
            .get("player_stats")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(Value::as_object)
            .ok_or_else(|| {
                run_malformed(format!("{path}.player_stats[0]"), "expected an object")
            })?;
        for key in ["cards_removed", "cards_transformed"] {
            let Some(rows) = stats.get(key) else { continue };
            let rows = rows
                .as_array()
                .ok_or_else(|| run_malformed(format!("{path}.{key}"), "expected a list"))?;
            for (r, row) in rows.iter().enumerate() {
                let row_path = format!("{path}.{key}[{r}]");
                let removed = if key == "cards_transformed" {
                    row.get("original_card").ok_or_else(|| {
                        run_malformed(format!("{row_path}.original_card"), "missing")
                    })?
                } else {
                    row
                };
                removals.push(removal(removed, i, &row_path)?);
            }
        }
        let rooms: Vec<&Value> = match pt.get("rooms") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(rows)) => rows.iter().collect(),
            Some(_) => return Err(run_malformed(format!("{path}.rooms"), "expected a list")),
        };
        let room = rooms.iter().copied().find(|r| {
            r.get("room_type")
                .and_then(Value::as_str)
                .is_some_and(|t| COMBAT_ROOM_TYPES.contains(&t))
        });
        let point_type = pt
            .get("map_point_type")
            .and_then(Value::as_str)
            .ok_or_else(|| run_malformed(format!("{path}.map_point_type"), "expected a string"))?;
        let is_combat = COMBAT_ROOM_TYPES.contains(&point_type)
            || room.is_some_and(|r| python_truthy(r.get("turns_taken")));
        if !is_combat {
            continue;
        }
        let (encounter, turns) = match room {
            None => ("?".to_owned(), Some(-1)),
            Some(room) => {
                let encounter = match room.get("model_id") {
                    None => "?".to_owned(),
                    Some(value) => value
                        .as_str()
                        .ok_or_else(|| {
                            run_malformed(format!("{path}.model_id"), "expected a string")
                        })?
                        .to_owned(),
                };
                let turns = match room.get("turns_taken") {
                    None => Some(-1),
                    Some(Value::Null) => None,
                    Some(value) => Some(run_integer(value, &format!("{path}.turns_taken"))?),
                };
                (encounter, turns)
            }
        };
        combats.push(Combat {
            node_index: i,
            floor: i as i64 + 1,
            encounter,
            turns,
            event: point_type == "unknown",
        });
    }
    Ok(Combats {
        seed,
        deck,
        combats,
        removals,
    })
}

/// The card at `index`'s floor, where Python would index
/// `c["floor_added_to_deck"]` on it.
fn floor_of(card: &Card, node: usize) -> Result<i64, RunCountersRefusal> {
    card.floor
        .ok_or(RunCountersRefusal::RemovalRowWithoutFloor { node })
}

/// `replay_fight.entry_deck`: the ids of the deck entering `floor`.
fn entry_deck(combats: &Combats, floor: i64) -> Result<Vec<String>, RunCountersRefusal> {
    let mut cards: Vec<Card> = combats
        .deck
        .iter()
        .filter(|c| c.floor.is_some_and(|f| f < floor))
        .cloned()
        .collect();
    for rc in &combats.removals {
        // Present iff the removal node is at or after the fight's node: a
        // removal logged at the fight's own node happened during its combat.
        if (rc.node as i64) < floor - 1 || rc.floor_or_one() >= floor {
            continue;
        }
        let rc_id = rc
            .id
            .clone()
            .ok_or_else(|| run_malformed(format!("removal at node {}", rc.node), "missing id"))?;
        let rc_floor = rc.floor_or_one();
        let mut pos = 0;
        for (k, c) in cards.iter().enumerate() {
            // Python builds the `(id, floor)` tuple before comparing, so a
            // floorless card raises even when the ids already differ.
            let c_floor = floor_of(c, rc.node)?;
            if c.id == rc_id && c_floor == rc_floor {
                pos = k + 1;
            }
        }
        if pos == 0 {
            let bane = cards.iter().position(|c| c.id == ETHEREAL_UNPLAYABLE);
            match bane {
                Some(bane) if rc_floor == 1 => pos = bane,
                _ => {
                    for (k, c) in cards.iter().enumerate() {
                        if floor_of(c, rc.node)? <= rc_floor {
                            pos = k + 1;
                        }
                    }
                }
            }
        }
        cards.insert(
            pos,
            Card {
                id: rc_id,
                floor: rc.floor,
            },
        );
    }
    Ok(cards.into_iter().map(|c| c.id).collect())
}

/// `replay_fight.fight_consumption`: advance `rng` past one prior fight.
fn fight_consumption(deck_ids: &[String], turns: i64, rng: &mut Xoshiro256StarStar) {
    let shuffle = |rng: &mut Xoshiro256StarStar, pile: &mut Vec<&str>| {
        rng.shuffle(pile)
            .expect("a pile length always fits the shuffle bound");
    };
    let mut pile: Vec<&str> = deck_ids.iter().map(String::as_str).collect();
    shuffle(rng, &mut pile);
    pile.reverse(); // draw from the front == pop from the end
    let mut discard: Vec<&str> = Vec::new();
    for _turn in 0..turns.max(0) {
        let mut drawn = Vec::with_capacity(DRAWS_PER_TURN);
        for _ in 0..DRAWS_PER_TURN {
            if pile.is_empty() {
                if discard.is_empty() {
                    break; // tiny cycle: nothing to reshuffle
                }
                shuffle(rng, &mut discard);
                pile = discard.iter().rev().copied().collect();
                discard.clear();
            }
            drawn.push(pile.pop().expect("the pile was just refilled"));
        }
        for card in drawn {
            if card != ETHEREAL_UNPLAYABLE {
                discard.push(card);
            }
        }
    }
}

/// The census fields `cycle_leavers` reads, from `data/cards_census.json`
/// (a byte copy of the solver's `cards_census.json`, pinned in the tests).
#[derive(Debug)]
struct CensusSpec {
    power: bool,
    draws: bool,
    adds_cards: bool,
    shuffles: bool,
    exhausts_other: bool,
    keywords: Vec<String>,
}

fn census() -> &'static HashMap<String, CensusSpec> {
    static CENSUS: OnceLock<HashMap<String, CensusSpec>> = OnceLock::new();
    CENSUS.get_or_init(|| {
        let raw: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../../data/cards_census.json"))
                .expect("the checked-in census is a JSON object");
        raw.into_iter()
            .map(|(id, spec)| {
                let flag = |key: &str| spec.get(key).and_then(Value::as_bool).unwrap_or(false);
                let keywords = spec
                    .get("keywords")
                    .and_then(Value::as_array)
                    .map(|k| {
                        k.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let power = spec.get("type").and_then(Value::as_str) == Some("power");
                (
                    id,
                    CensusSpec {
                        power,
                        draws: flag("draws"),
                        adds_cards: flag("adds_cards"),
                        shuffles: flag("shuffles"),
                        exhausts_other: flag("exhausts_other"),
                        keywords,
                    },
                )
            })
            .collect()
    })
}

/// `replay_fight.cycle_leavers`: the deck's cards whose play (or non-play) can
/// change reshuffle sizes by an unrecorded amount, sorted.
fn cycle_leavers(deck_ids: &[String]) -> Vec<String> {
    let census = census();
    let mut out: Vec<String> = Vec::new();
    for id in deck_ids {
        if id == ETHEREAL_UNPLAYABLE {
            continue;
        }
        let leaver = match census.get(id) {
            None => Some(format!("{id} (not in cards_census)")),
            Some(spec) => {
                let keyword = spec
                    .keywords
                    .iter()
                    .any(|k| matches!(k.as_str(), "Exhaust" | "Ethereal" | "Retain"));
                (spec.power
                    || spec.draws
                    || spec.adds_cards
                    || spec.shuffles
                    || spec.exhausts_other
                    || keyword)
                    .then(|| id.clone())
            }
        };
        if let Some(leaver) = leaver {
            out.push(leaver);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The `Shuffle` half of the answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShufflePrediction {
    pub counter: u64,
    pub caveats: Vec<String>,
}

/// The counter half of `replay_fight.predict`.
pub fn predict(
    combats: &Combats,
    fight_index: usize,
) -> Result<ShufflePrediction, RunCountersRefusal> {
    let list = &combats.combats;
    if fight_index >= list.len() {
        return Err(RunCountersRefusal::FightIndexOutOfRange {
            index: fight_index,
            fights: list.len(),
        });
    }
    let mut rng = run_stream_at_zero(&combats.seed, "shuffle");
    let mut caveats = Vec::new();
    for (k, fight) in list.iter().enumerate().take(fight_index) {
        let ids = entry_deck(combats, fight.floor)?;
        let turns = match fight.turns {
            Some(turns) if turns >= 0 => turns,
            _ => return Err(RunCountersRefusal::PriorFightTurnsUnknown { fight: k }),
        };
        fight_consumption(&ids, turns, &mut rng);
        let leavers = cycle_leavers(&ids);
        if !leavers.is_empty() {
            let names: Vec<String> = leavers.iter().map(|c| c.replace("CARD.", "")).collect();
            caveats.push(format!(
                "fight {k} ({}): deck held cards that can leave/alter the cycle unrecorded \
                 ({}) — the counter is a baseline, not exact",
                fight.encounter,
                names.join(", ")
            ));
        }
        if fight.event {
            caveats.push(format!(
                "fight {k} ({}) is an event-node combat: cards the event granted BEFORE its \
                 fight (ordering unrecorded) would shift its deck size and reshuffle \
                 boundaries — counter is a baseline",
                fight.encounter
            ));
        }
    }
    // The target's own entry deck is built too, as Python built it, so a row it
    // would have raised on refuses here rather than being skipped.
    entry_deck(combats, list[fight_index].floor)?;
    Ok(ShufflePrediction {
        counter: rng.counter,
        caveats,
    })
}

#[cfg(test)]
pub(super) mod test_support {
    pub fn entry_deck_ids(run: &serde_json::Value, floor: i64) -> Vec<String> {
        let combats = super::load_combats(run).unwrap();
        super::entry_deck(&combats, floor).unwrap()
    }

    pub fn cycle_leavers(ids: &[&str]) -> Vec<String> {
        let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        super::cycle_leavers(&ids)
    }
}
