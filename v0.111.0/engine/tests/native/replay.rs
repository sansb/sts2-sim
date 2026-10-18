//! Replay a recorded native run on the engine and compare step by step.
//!
//! A `*.native.json` beside this file is what the game itself did
//! (`probe.py`, the archived v0.111.0 `sts2.dll`, headless). [`compare`] opens
//! the composition sweep's synthetic root for the same character and
//! encounter, gives it the recorded root's piles, Energy, monster HP and RNG
//! streams, plays the scenario's script through the engine's public actions,
//! and compares a [`View`] after every step.
//!
//! The view is what both sides can state: the player's HP, Block and Energy,
//! every living enemy's HP and Block, a living pet's HP, the four piles by
//! card label in the game's order, and each combat RNG stream's counter. The
//! caller adds named powers (`Powers`): a native power class beside the
//! engine's player or monster field that carries its amount.

use serde_json::Value;
use sts_sim::{
    boundary::HotBoundary,
    canonical::{CanonicalCardV2, CanonicalStateV2},
    catalog::{Catalog, GameBuild},
    engine::{self, Action, SelectionRef},
    entry::{self, EntryInput, EntryRequest, RootOutcome},
    hot::HotState,
};

use super::sweep;

/// Native power classes to compare, each beside the engine field holding its
/// amount: on the player's field bag, and on each monster's.
#[derive(Clone, Copy, Default)]
pub struct Powers {
    pub player: &'static [(&'static str, &'static str)],
    pub monster: &'static [(&'static str, &'static str)],
}

#[derive(Debug, PartialEq, Eq)]
pub struct View {
    pub label: String,
    pub hp: i64,
    pub block: i64,
    pub energy: i64,
    /// `(hp, block, compared powers)` per living enemy, in creature order.
    pub monsters: Vec<(i64, i64, Vec<i64>)>,
    pub pet_hp: Option<i64>,
    pub player_powers: Vec<i64>,
    pub hand: Vec<String>,
    pub draw: Vec<String>,
    pub discard: Vec<String>,
    pub exhaust: Vec<String>,
    pub rng: Vec<(String, u64)>,
    pub over: bool,
}

const STREAMS: [&str; 9] = [
    "ai",
    "combat_orbs",
    "energy_costs",
    "generation",
    "niche",
    "potion_generation",
    "rng",
    "sel",
    "targets",
];

fn labels(cards: &Value) -> Vec<String> {
    cards
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label.as_str().unwrap().to_string())
        .collect()
}

fn native_view(entry: &Value, powers: Powers) -> View {
    let snap = &entry["snap"];
    let amount = |list: &Value, class: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|power| power[0] == class)
            .map_or(0, |power| power[1].as_i64().unwrap())
    };
    let creatures = snap["creatures"].as_array().unwrap();
    View {
        label: entry["label"].as_str().unwrap().to_string(),
        hp: snap["hp"].as_i64().unwrap(),
        block: snap["block"].as_i64().unwrap(),
        energy: snap["energy"].as_i64().unwrap(),
        monsters: creatures
            .iter()
            .filter(|creature| creature["name"] != "PLAYER" && creature["pet"] == false)
            .filter(|creature| creature["dead"] == false)
            .map(|creature| {
                (
                    creature["hp"].as_i64().unwrap(),
                    creature["block"].as_i64().unwrap(),
                    powers
                        .monster
                        .iter()
                        .map(|(class, _)| amount(&creature["powers"], class))
                        .collect(),
                )
            })
            .collect(),
        pet_hp: creatures
            .iter()
            .find(|creature| creature["pet"] == true && creature["dead"] == false)
            .map(|creature| creature["hp"].as_i64().unwrap()),
        player_powers: powers
            .player
            .iter()
            .map(|(class, _)| amount(&snap["powers"], class))
            .collect(),
        hand: labels(&snap["piles"]["Hand"]),
        draw: labels(&snap["piles"]["DrawPile"]),
        discard: labels(&snap["piles"]["DiscardPile"]),
        exhaust: labels(&snap["piles"]["ExhaustPile"]),
        rng: STREAMS
            .iter()
            .map(|stream| {
                (
                    (*stream).to_string(),
                    snap["rng"][*stream]["counter"].as_u64().unwrap(),
                )
            })
            .collect(),
        over: snap["over"].as_bool().unwrap(),
    }
}

/// The probe's card label: the id, a `+` per upgrade, `@ENCHANTMENT`.
fn card_label(card: &CanonicalCardV2) -> String {
    let enchantment = card
        .enchantment
        .as_ref()
        .and_then(|enchantment| enchantment.get(0))
        .and_then(Value::as_str)
        // A spent Glam is still the card's Glam enchantment on the game.
        .map(|name| format!("@{}", name.trim_end_matches("_USED")))
        .unwrap_or_default();
    format!(
        "{}{}{enchantment}",
        card.id,
        "+".repeat(card.upgrade as usize)
    )
}

fn engine_view(label: &str, state: &HotState, catalog: &Catalog, powers: Powers) -> View {
    let doc = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let int = |bag: &sts_sim::canonical::CanonicalEntityV2, key: &str| {
        bag.get(key).and_then(Value::as_i64).unwrap_or(0)
    };
    let pile = |name: &str| {
        doc.piles
            .get(name)
            .map(|cards| cards.iter().map(card_label).collect())
            .unwrap_or_default()
    };
    View {
        label: label.to_string(),
        hp: int(&doc.player, "hp"),
        block: int(&doc.player, "block"),
        // The document omits a field at its default, and Energy's is 3.
        energy: doc
            .player
            .get("energy")
            .and_then(Value::as_i64)
            .unwrap_or(3),
        monsters: doc
            .monsters
            .iter()
            .filter(|monster| int(monster, "hp") > 0)
            .map(|monster| {
                (
                    int(monster, "hp"),
                    int(monster, "block"),
                    powers
                        .monster
                        .iter()
                        .map(|(_, field)| int(monster, field))
                        .collect(),
                )
            })
            .collect(),
        // `ally` is `["OSTY", hp, max_hp]` while Osty lives.
        pet_hp: doc
            .player
            .get("ally")
            .and_then(|ally| ally.get(1))
            .and_then(Value::as_i64)
            .filter(|hp| *hp > 0),
        player_powers: powers
            .player
            .iter()
            .map(|(_, field)| int(&doc.player, field))
            .collect(),
        hand: pile("hand"),
        draw: pile("draw"),
        discard: pile("discard"),
        exhaust: pile("exhaust"),
        rng: STREAMS
            .iter()
            .map(|stream| {
                (
                    (*stream).to_string(),
                    doc.rng.get(*stream).map_or(0, |rng| rng.counter),
                )
            })
            .collect(),
        over: state.history.over,
    }
}

fn character_name(native: &str) -> String {
    native.to_ascii_uppercase()
}

/// The engine's root for a recorded scenario, and its catalog.
pub fn root(recorded: &Value) -> (HotState, Catalog) {
    let spec = &recorded["spec"];
    let character = sweep::Character::named(&character_name(spec["character"].as_str().unwrap()))
        .expect("a sweep character");
    let encounter = spec["encounter"].as_str().unwrap();
    let deck: Vec<String> = labels(&spec["deck"])
        .iter()
        .map(|label| {
            // `ID`, a `+` per upgrade, `@ENCHANTMENT:amount`; the sweep
            // names the same card `ENCHANTMENT@ID+LEVEL`.
            let (card, enchantment) = label.split_once('@').unwrap_or((label, ""));
            let id = card.trim_end_matches('+');
            let level = card.len() - id.len();
            match enchantment.split(':').next().unwrap() {
                "" => format!("{id}+{level}"),
                enchantment => format!("{enchantment}@{id}+{level}"),
            }
        })
        .collect();
    // The sweep's root already owns the character's starter relic.
    let relics: Vec<String> = labels(&spec["relics"])
        .into_iter()
        .filter(|relic| format!("RELIC.{relic}") != character.starter_relic)
        .collect();
    let root = sweep::custom_root(&deck, &relics, &[], Some(encounter)).unwrap();
    let save = sweep::save_json(character, &root);
    let encounter_id = format!("ENCOUNTER.{encounter}");
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: Some(&encounter_id),
        node_type: Some("monster"),
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let RootOutcome::Opened(opening) = entry::build_root(&request, None) else {
        panic!("the synthetic root opens");
    };
    let mut doc: CanonicalStateV2 = opening.document.clone();
    let native = &recorded["runs"][0]["trace"][0]["snap"];

    // The recorded root's piles, taken card by card from the opened deck.
    let mut pool: Vec<CanonicalCardV2> = ["hand", "draw", "discard"]
        .iter()
        .flat_map(|pile| doc.piles.get(*pile).cloned().unwrap_or_default())
        .collect();
    for (name, native_pile) in [
        ("hand", "Hand"),
        ("draw", "DrawPile"),
        ("discard", "DiscardPile"),
    ] {
        let cards: Vec<CanonicalCardV2> = labels(&native["piles"][native_pile])
            .iter()
            .map(|label| {
                let at = pool
                    .iter()
                    .position(|card| card_label(card) == *label)
                    .unwrap_or_else(|| panic!("the opened deck holds {label}"));
                pool.remove(at)
            })
            .collect();
        doc.piles.insert(name.to_string(), cards);
    }
    doc.player
        .insert("energy".to_string(), native["energy"].clone());
    doc.player.insert("hp".to_string(), native["hp"].clone());
    let enemies: Vec<&Value> = native["creatures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|creature| creature["name"] != "PLAYER" && creature["pet"] == false)
        .collect();
    assert_eq!(enemies.len(), doc.monsters.len());
    for (monster, enemy) in doc.monsters.iter_mut().zip(enemies) {
        monster.insert("hp".to_string(), enemy["hp"].clone());
        monster.insert("max_hp".to_string(), enemy["max_hp"].clone());
    }
    for stream in STREAMS {
        let Some(rng) = doc.rng.get_mut(stream) else {
            continue;
        };
        let recorded = &native["rng"][stream];
        rng.counter = recorded["counter"].as_u64().unwrap();
        for (word, native_word) in rng
            .words
            .iter_mut()
            .zip(recorded["words"].as_array().unwrap())
        {
            *word = native_word[1].as_u64().unwrap();
        }
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

/// Every state on the line is a document the engine takes back: it
/// projects, loads under a catalog built from that document alone, is
/// admitted, and projects to the same document again.
fn reload(state: &HotState, catalog: &Catalog, index: usize) {
    let doc = HotBoundary::try_to_canonical(state, catalog).unwrap();
    if state.history.over {
        return;
    }
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&doc)
        .unwrap_or_else(|refusal| panic!("step {index}: catalog {refusal:?}"));
    let reloaded = HotBoundary::from_canonical(&doc, &reloaded_catalog)
        .unwrap_or_else(|refusal| panic!("step {index}: reload {refusal:?}"));
    engine::admit(&doc, &reloaded, &reloaded_catalog)
        .unwrap_or_else(|refusal| panic!("step {index}: not admitted: {refusal}"));
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog).unwrap(),
        doc,
        "step {index}: the reloaded state projects differently"
    );
}

/// The engine's index for a native CombatId: its place among the enemies in
/// creature order.
fn target_index(recorded: &Value, combat_id: i64) -> usize {
    recorded["runs"][0]["trace"][0]["snap"]["creatures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|creature| creature["name"] != "PLAYER" && creature["pet"] == false)
        .position(|creature| creature["id"] == combat_id)
        .expect("a recorded enemy")
}

/// Both sides' views after each scripted step: `(native, engine)`.
///
/// A play must be offered by `legal_actions`; the comparison is the caller's
/// (`assert_eq!` on the pair), so a test can first assert what the recording
/// itself shows.
pub fn views(recorded: &str, powers: Powers) -> (Vec<View>, Vec<View>) {
    let recorded: Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(recorded["build"], "v0.111.0");
    let trace = recorded["runs"][0]["trace"].as_array().unwrap();
    let native: Vec<View> = trace[1..]
        .iter()
        .map(|entry| {
            assert_eq!(entry["state"], "Finished", "{}", entry["label"]);
            native_view(entry, powers)
        })
        .collect();
    let (mut state, catalog) = root(&recorded);
    let mut ours = Vec::new();
    let mut step = 0;
    for (index, line) in recorded["spec"]["script"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let action = match line[0].as_str().unwrap() {
            "play" => {
                let label = line[1].as_str().unwrap();
                let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
                let uid = doc.piles["hand"]
                    .iter()
                    .find(|card| card_label(card) == label)
                    .and_then(|card| card.uid)
                    .unwrap_or_else(|| panic!("{label} is in the Hand"))
                    as u32;
                let action = Action::Play {
                    uid,
                    target: line
                        .get(2)
                        .and_then(Value::as_i64)
                        .map(|id| target_index(&recorded, id) as u8),
                    selection: SelectionRef::NONE,
                };
                assert!(
                    engine::legal_actions(&state, &catalog).contains(&action),
                    "step {index}: {label} is offered"
                );
                action
            }
            "end" => Action::EndTurn,
            _ => continue,
        };
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap_or_else(|refusal| panic!("step {index}: {refusal:?}"))
            .state;
        ours.push(engine_view(&native[step].label, &state, &catalog, powers));
        reload(&state, &catalog, index);
        step += 1;
    }
    assert_eq!(step, native.len());
    (native, ours)
}

/// Assert the engine does what the recording shows, step by step.
pub fn compare(recorded: &str, powers: Powers) -> Vec<View> {
    let (native, ours) = views(recorded, powers);
    for (theirs, ours) in native.iter().zip(&ours) {
        assert_eq!(ours, theirs);
    }
    native
}
