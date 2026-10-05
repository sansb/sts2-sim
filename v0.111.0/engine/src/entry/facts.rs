//! `sts-sim entry --facts`: an `sts-sim-entry-v1` document as the input
//! (#2827 item B).
//!
//! # What it is for
//!
//! Every other entry input is a run the game wrote: a schema-20 `.save`, or a
//! capture's embedded run mapped onto the save surface. The Coach card-offer
//! review has no such run. Its roots are **synthetic** fight entries: the
//! decision-time deck and relics, full HP, an empty belt, a benchmark
//! encounter at a fixed node, a paired per-sample seed with every stream at
//! counter 0, and a fully unlocked profile. Until this module those roots were
//! built by frozen Python (`combat_sim.start_combat` on a hand-built entry
//! dict). This input lets the caller hand Rust the **entry facts themselves**,
//! in the exact wire vocabulary the save path already emits
//! ([`EntryDocument::to_json`]), and run the Rust opening on them.
//!
//! # The document
//!
//! [`EntryDocument::to_json`] plus one key, `streams`: every recorded run
//! stream's counter **and** its four xoshiro words, in the save's own
//! `rng.rngs` spelling (`{"shuffle": {"counter": 0, "s0": …, "s3": …}, …}`).
//! The opening reads stream state, never a seeding scheme (`entry/counters.rs`),
//! so the facts carry state too.
//!
//! # Validation: the save path's rules, stated on the facts
//!
//! The save path derives each fact from raw save bytes and can only ever
//! produce a well-formed one. A facts document is those derived facts written
//! down by someone else, so this parser refuses, by name, every value the
//! save path could not have produced:
//!
//! * the key surface is exact at every level — an untaught, missing or
//!   wrong-typed key is [`EntryRefusal::UnknownFactsField`] (the save path's
//!   `deny_unknown_fields`);
//! * ids are already canonical (`CARD.X`, never a bare `X` to be repaired);
//! * per-instance `props` ride only on the three cards `entry/deck.rs` carries
//!   them for; an enchantment carries its amount and the amount its
//!   enchantment;
//! * `potions_entering` is exactly the slot ids, and `relic_counters` / the
//!   Tea Set flags name only owned relics, with the value shapes
//!   `entry/relics.rs` produces (an integer, Lizard Tail's or Maw Bank's
//!   boolean, or an object of integers);
//! * `fur_coat_active` is set only beside an owned `RELIC.FUR_COAT`, the one
//!   fight the save path answers it for (#2526), and `entry_ambiguous` is
//!   false, because a save never proves it;
//! * the epoch set is normalized exactly as `unlocks::card_pool_unlocked_epochs`
//!   normalizes it;
//! * stream names are `RunRngType` members, the six required combat streams
//!   are present (`entry/counters.rs`), `counters` is exactly the streams'
//!   counters, and — the one check a save does not get — each stream's words
//!   are the state `RunRngSet` derives from the seed at that counter.
//!
//! The three unlock answers (`unlocked_card_pool_epochs`,
//! `fully_unlocked_card_pool`, `fully_unlocked_potion_pool`) are independent
//! facts here, as they are independent `start_combat` arguments: a save
//! derives the two booleans from its epoch list, and a synthetic profile
//! states them.
//!
//! # Why the stream words are checked against the seed
//!
//! A save's words are the game's own serialized state; a facts document's
//! words are someone's claim. The check makes the facts document no less
//! authoritative than the seed it names: `RunRngSet::CreateRng` (RVA
//! `0x4df00`) seeds each stream as `Seed + hash(snake_case(name))`
//! ([`crate::rng::run_stream_at_zero`]), and a stream at counter `n` has made
//! exactly `n` generator draws — every `Rng` draw method increments `_counter`
//! and makes one `MegaRandom` draw (`Rng::NextInt` RVA `0x5eb26`,
//! `IL_0001`-`IL_0016`), which is `sts2_rng.Rng.fast_forward`. The same
//! derivation agreed with 3,092 of 3,092 schema-20 saves
//! (`entry/counters.rs`). It is a cross-check, not a source: the opening still
//! reads the recorded words.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::catalog::GameBuild;
use crate::entry::belt::{Belt, PotionSlot};
use crate::entry::canonical_model_id;
use crate::entry::counters::REQUIRED_COMBAT_STREAMS;
use crate::entry::deck::{DeckEntry, PER_INSTANCE_PROP_CARDS};
use crate::entry::document::{ENTRY_SCHEMA_V1, EntryDocument};
use crate::entry::refusal::EntryRefusal;
use crate::entry::relics::RelicEntry;
use crate::entry::save::{RUN_RNG_STREAMS, SerializedRng};

/// The save schema the entry facts describe. `entry/save.rs` admits only 20.
const FACTS_SAVE_SCHEMA_VERSION: u32 = 20;

/// The furthest counter the seed cross-check walks. The corpus's largest
/// recorded run-stream counter is in the low thousands; a counter beyond this
/// is refused by name rather than walked.
pub const MAX_VERIFIED_STREAM_COUNTER: u64 = 1 << 20;

const TOP_LEVEL_KEYS: [&str; 9] = [
    "schema",
    "game_build",
    "save_schema_version",
    "seed",
    "character",
    "entry",
    "counters",
    "streams",
    "unlocks",
];

const ENTRY_KEYS: [&str; 18] = [
    "encounter_id",
    "node_index",
    "node_type",
    "deck_entering",
    "relics_entering",
    "potions_entering",
    "max_potion_slot_count",
    "potion_slots_entering",
    "hp_entering",
    "max_hp_entering",
    "gold_entering",
    "ascension",
    "relic_counters",
    "tea_set_charged",
    "fake_tea_set_charged",
    "fur_coat_active",
    "relics_entering_dispatch_ordered",
    "entry_ambiguous",
];

const UNLOCK_KEYS: [&str; 3] = [
    "unlocked_card_pool_epochs",
    "fully_unlocked_card_pool",
    "fully_unlocked_potion_pool",
];

fn unknown(detail: impl Into<String>) -> EntryRefusal {
    EntryRefusal::UnknownFactsField(detail.into())
}

fn inconsistent(field: &'static str, detail: impl Into<String>) -> EntryRefusal {
    EntryRefusal::FactsInconsistent {
        field,
        detail: detail.into(),
    }
}

/// The object at `path`, with exactly `required` keys plus any of `optional`.
fn exact_object<'a>(
    value: &'a Value,
    path: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a Map<String, Value>, EntryRefusal> {
    let object = value
        .as_object()
        .ok_or_else(|| unknown(format!("{path} is not an object")))?;
    for key in object.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(unknown(format!("{path}.{key}")));
        }
    }
    for key in required {
        if !object.contains_key(*key) {
            return Err(unknown(format!("{path}.{key} is absent")));
        }
    }
    Ok(object)
}

fn string<'a>(value: &'a Value, path: &str) -> Result<&'a str, EntryRefusal> {
    value
        .as_str()
        .ok_or_else(|| unknown(format!("{path} is not a string")))
}

fn optional_string(value: &Value, path: &str) -> Result<Option<String>, EntryRefusal> {
    match value {
        Value::Null => Ok(None),
        other => string(other, path).map(|text| Some(text.to_string())),
    }
}

fn integer(value: &Value, path: &str) -> Result<i64, EntryRefusal> {
    value
        .as_i64()
        .ok_or_else(|| unknown(format!("{path} is not an integer")))
}

fn optional_integer(value: &Value, path: &str) -> Result<Option<i64>, EntryRefusal> {
    match value {
        Value::Null => Ok(None),
        other => integer(other, path).map(Some),
    }
}

fn boolean(value: &Value, path: &str) -> Result<bool, EntryRefusal> {
    value
        .as_bool()
        .ok_or_else(|| unknown(format!("{path} is not a boolean")))
}

fn optional_bool(value: &Value, path: &str) -> Result<Option<bool>, EntryRefusal> {
    match value {
        Value::Null => Ok(None),
        other => boolean(other, path).map(Some),
    }
}

fn array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>, EntryRefusal> {
    value
        .as_array()
        .ok_or_else(|| unknown(format!("{path} is not an array")))
}

/// An id the save path would have emitted: already canonical in `category`.
fn canonical_id(value: &Value, path: &str, category: &'static str) -> Result<String, EntryRefusal> {
    let text = string(value, path)?;
    let canonical = canonical_model_id(Some(text), category)?;
    if canonical != text {
        return Err(EntryRefusal::MalformedModelId {
            category,
            value: text.to_string(),
        });
    }
    Ok(canonical)
}

/// Parse and validate one entry-facts document for `game_build`.
pub fn parse(text: &str, game_build: GameBuild) -> Result<EntryDocument, EntryRefusal> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| EntryRefusal::MalformedFactsJson(error.to_string()))?;
    let top = exact_object(
        &value,
        "facts",
        &TOP_LEVEL_KEYS,
        &["opening", "map_point_type", "next_normal_encounter"],
    )?;

    let schema = string(&top["schema"], "schema")?;
    if schema != ENTRY_SCHEMA_V1 {
        return Err(EntryRefusal::UnsupportedFactsSchema(schema.to_string()));
    }
    let facts_build = string(&top["game_build"], "game_build")?;
    if facts_build != game_build.as_str() {
        return Err(EntryRefusal::FactsBuildMismatch {
            facts: facts_build.to_string(),
            requested: game_build.as_str().to_string(),
        });
    }
    let raw_schema_version = top["save_schema_version"]
        .as_u64()
        .ok_or_else(|| unknown("save_schema_version is not an unsigned integer"))?;
    let save_schema_version = u32::try_from(raw_schema_version).unwrap_or(u32::MAX);
    if save_schema_version != FACTS_SAVE_SCHEMA_VERSION {
        return Err(EntryRefusal::UnsupportedSaveSchema(save_schema_version));
    }
    let seed = string(&top["seed"], "seed")?.to_string();
    let character = match &top["character"] {
        Value::Null => None,
        other => Some(canonical_id(other, "character", "CHARACTER")?),
    };
    if let Some(opening) = top.get("opening") {
        opening_block(opening)?;
    }

    let entry = exact_object(&top["entry"], "entry", &ENTRY_KEYS, &[])?;
    let encounter_id = canonical_id(&entry["encounter_id"], "entry.encounter_id", "ENCOUNTER")?;
    // #3248: an event-started combat is entered after the event's option ran.
    // A facts document carries no run modifiers and no mark saying which side
    // of the event its player is, so the save path's allow-list
    // (`entry/event_combat.rs`) cannot be applied to it.
    if crate::entry::event_combat::event_combat(&encounter_id)?.is_some() {
        return Err(EntryRefusal::EventCombatFactsUnvouched {
            encounter: encounter_id,
        });
    }
    let node_index = integer(&entry["node_index"], "entry.node_index")?;
    if node_index < 0 {
        return Err(inconsistent(
            "entry.node_index",
            format!("{node_index} is negative"),
        ));
    }
    let node_type = optional_string(&entry["node_type"], "entry.node_type")?;
    if node_type.as_deref() == Some("") {
        return Err(inconsistent("entry.node_type", "is the empty string"));
    }
    let deck_entering = deck(&entry["deck_entering"])?;
    let belt = belt(entry)?;
    let relic_entry = relics(entry)?;
    // `entry_json` always writes `false`: a save is the node-entry snapshot.
    if boolean(&entry["entry_ambiguous"], "entry.entry_ambiguous")? {
        return Err(inconsistent(
            "entry.entry_ambiguous",
            "is true; a save is the node-entry snapshot and never writes it",
        ));
    }

    let streams = streams(&top["streams"], &seed)?;
    let counters = counters(&top["counters"], &streams)?;
    let unlocks = exact_object(&top["unlocks"], "unlocks", &UNLOCK_KEYS, &[])?;

    Ok(EntryDocument {
        game_build,
        save_schema_version,
        seed,
        character,
        encounter_id,
        node_type,
        node_index,
        deck_entering,
        relic_entry,
        belt,
        hp_entering: integer(&entry["hp_entering"], "entry.hp_entering")?,
        max_hp_entering: integer(&entry["max_hp_entering"], "entry.max_hp_entering")?,
        gold_entering: integer(&entry["gold_entering"], "entry.gold_entering")?,
        ascension: optional_integer(&entry["ascension"], "entry.ascension")?,
        unlocked_card_pool_epochs: epochs(&unlocks["unlocked_card_pool_epochs"])?,
        fully_unlocked_card_pool: optional_bool(
            &unlocks["fully_unlocked_card_pool"],
            "unlocks.fully_unlocked_card_pool",
        )?,
        fully_unlocked_potion_pool: optional_bool(
            &unlocks["fully_unlocked_potion_pool"],
            "unlocks.fully_unlocked_potion_pool",
        )?,
        counters,
        streams,
        map_point_type: map_point_type(top.get("map_point_type"))?,
        next_normal_encounter: match top.get("next_normal_encounter") {
            None | Some(Value::Null) => None,
            Some(value) => Some(canonical_id(value, "next_normal_encounter", "ENCOUNTER")?),
        },
    })
}

/// The optional off-wire `map_point_type` ([`EntryDocument::map_point_type`]):
/// absent or `null` is `None`, and a present one must be a `MAP_POINT_TYPES`
/// spelling, since only [`crate::entry::node::map_point_type`] writes it.
fn map_point_type(value: Option<&Value>) -> Result<Option<String>, EntryRefusal> {
    let Some(kind) = value
        .map(|value| optional_string(value, "map_point_type"))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    if !crate::entry::save::MAP_POINT_TYPES.contains(&kind.as_str()) {
        return Err(inconsistent(
            "map_point_type",
            format!("{kind:?} is not a MapPointType spelling"),
        ));
    }
    Ok(Some(kind))
}

/// The `opening` block [`EntryDocument::to_json`] writes: an entry document
/// never carries a built or spliced opening.
fn opening_block(value: &Value) -> Result<(), EntryRefusal> {
    let block = exact_object(
        value,
        "opening",
        &["built", "mcr_splice_applied"],
        &["detail", "refusal_class"],
    )?;
    if boolean(&block["built"], "opening.built")?
        || boolean(&block["mcr_splice_applied"], "opening.mcr_splice_applied")?
    {
        return Err(inconsistent(
            "opening",
            "an sts-sim-entry-v1 document never carries a built or spliced opening",
        ));
    }
    for key in ["detail", "refusal_class"] {
        if let Some(text) = block.get(key) {
            string(text, &format!("opening.{key}"))?;
        }
    }
    Ok(())
}

fn deck(value: &Value) -> Result<Vec<DeckEntry>, EntryRefusal> {
    let mut rows = Vec::new();
    for (index, row) in array(value, "entry.deck_entering")?.iter().enumerate() {
        let path = format!("entry.deck_entering[{index}]");
        let object = exact_object(
            row,
            &path,
            &["id", "upgrade_level"],
            &["props", "enchantment", "enchant_amount"],
        )?;
        let id = canonical_id(&object["id"], &format!("{path}.id"), "CARD")?;
        let props = match object.get("props") {
            None => None,
            Some(props) => {
                if !PER_INSTANCE_PROP_CARDS.contains(&id.as_str()) {
                    return Err(inconsistent(
                        "entry.deck_entering",
                        format!(
                            "{path} ({id}) carries props; only the per-instance cards \
                             {PER_INSTANCE_PROP_CARDS:?} do"
                        ),
                    ));
                }
                Some(
                    props
                        .as_object()
                        .ok_or_else(|| unknown(format!("{path}.props is not an object")))?
                        .clone(),
                )
            }
        };
        let (enchantment, enchant_amount) =
            match (object.get("enchantment"), object.get("enchant_amount")) {
                (None, None) => (None, None),
                (Some(enchantment), Some(amount)) => (
                    Some(canonical_id(
                        enchantment,
                        &format!("{path}.enchantment"),
                        "ENCHANTMENT",
                    )?),
                    Some(integer(amount, &format!("{path}.enchant_amount"))?),
                ),
                _ => {
                    return Err(inconsistent(
                        "entry.deck_entering",
                        format!(
                            "{path} carries one of enchantment/enchant_amount without the other"
                        ),
                    ));
                }
            };
        rows.push(DeckEntry {
            id,
            upgrade_level: integer(&object["upgrade_level"], &format!("{path}.upgrade_level"))?,
            props,
            enchantment,
            enchant_amount,
        });
    }
    Ok(rows)
}

fn belt(entry: &Map<String, Value>) -> Result<Belt, EntryRefusal> {
    let mut slots = Vec::new();
    for (index, row) in array(
        &entry["potion_slots_entering"],
        "entry.potion_slots_entering",
    )?
    .iter()
    .enumerate()
    {
        let path = format!("entry.potion_slots_entering[{index}]");
        let object = exact_object(row, &path, &["id", "slot_index"], &[])?;
        slots.push(PotionSlot {
            id: canonical_id(&object["id"], &format!("{path}.id"), "POTION")?,
            slot_index: optional_integer(&object["slot_index"], &format!("{path}.slot_index"))?,
        });
    }
    let belt = Belt {
        slots,
        max_potion_slot_count: optional_integer(
            &entry["max_potion_slot_count"],
            "entry.max_potion_slot_count",
        )?,
    };
    let listed = array(&entry["potions_entering"], "entry.potions_entering")?
        .iter()
        .enumerate()
        .map(|(index, id)| {
            string(id, &format!("entry.potions_entering[{index}]")).map(str::to_string)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if listed != belt.potions_entering() {
        return Err(inconsistent(
            "entry.potions_entering",
            "is not exactly the ids of potion_slots_entering, in order",
        ));
    }
    Ok(belt)
}

fn relics(entry: &Map<String, Value>) -> Result<RelicEntry, EntryRefusal> {
    let relics_entering = array(&entry["relics_entering"], "entry.relics_entering")?
        .iter()
        .enumerate()
        .map(|(index, id)| canonical_id(id, &format!("entry.relics_entering[{index}]"), "RELIC"))
        .collect::<Result<Vec<_>, _>>()?;
    let owns = |relic: &str| relics_entering.iter().any(|id| id == relic);

    // `entry_json` writes `relic_counters or None`: an empty bag is `null`.
    let relic_counters = match &entry["relic_counters"] {
        Value::Null => BTreeMap::new(),
        Value::Object(bag) if bag.is_empty() => {
            return Err(inconsistent(
                "entry.relic_counters",
                "is an empty object; the save path writes an empty bag as null",
            ));
        }
        Value::Object(bag) => {
            let mut counters = BTreeMap::new();
            for (relic, value) in bag {
                if !owns(relic) {
                    return Err(inconsistent(
                        "entry.relic_counters",
                        format!("names {relic}, which is not in relics_entering"),
                    ));
                }
                if !relic_counter_shape(relic, value) {
                    return Err(inconsistent(
                        "entry.relic_counters",
                        format!(
                            "{relic} holds {value}; the save path writes an integer, \
                             Lizard Tail's or Maw Bank's boolean, or an object of integers"
                        ),
                    ));
                }
                counters.insert(relic.clone(), value.clone());
            }
            counters
        }
        _ => return Err(unknown("entry.relic_counters is not an object or null")),
    };

    let tea = |field: &'static str, key: &str, relic: &str| -> Result<Option<bool>, EntryRefusal> {
        let flag = optional_bool(&entry[key], field)?;
        if flag.is_some() && !owns(relic) {
            return Err(inconsistent(
                field,
                format!("is set and {relic} is not in relics_entering"),
            ));
        }
        Ok(flag)
    };
    let tea_set_charged = tea(
        "entry.tea_set_charged",
        "tea_set_charged",
        "RELIC.VENERABLE_TEA_SET",
    )?;
    let fake_tea_set_charged = tea(
        "entry.fake_tea_set_charged",
        "fake_tea_set_charged",
        "RELIC.FAKE_VENERABLE_TEA_SET",
    )?;
    // Null beside an owner is "unprovable", which the opening refuses (#2526).
    let fur_coat_active = tea("entry.fur_coat_active", "fur_coat_active", "RELIC.FUR_COAT")?;
    Ok(RelicEntry {
        relics_entering,
        relic_counters,
        tea_set_charged,
        fake_tea_set_charged,
        fur_coat_active,
        dispatch_ordered: boolean(
            &entry["relics_entering_dispatch_ordered"],
            "entry.relics_entering_dispatch_ordered",
        )?,
    })
}

/// The value shapes `entry/relics.rs` can put in `relic_counters`.
fn relic_counter_shape(relic: &str, value: &Value) -> bool {
    match value {
        Value::Number(number) => number.is_i64() || number.is_u64(),
        Value::Bool(_) => crate::entry::relics::is_counter_bool_relic(relic),
        Value::Object(bag) => {
            !bag.is_empty() && bag.values().all(|value| value.is_i64() || value.is_u64())
        }
        _ => false,
    }
}

fn streams(value: &Value, seed: &str) -> Result<BTreeMap<String, SerializedRng>, EntryRefusal> {
    let object = value
        .as_object()
        .ok_or_else(|| unknown("streams is not an object"))?;
    let mut streams = BTreeMap::new();
    for (name, state) in object {
        if !RUN_RNG_STREAMS.contains(&name.as_str()) {
            return Err(EntryRefusal::UnknownRngStream(name.clone()));
        }
        let state: SerializedRng = serde_json::from_value(state.clone())
            .map_err(|error| unknown(format!("streams.{name}: {error}")))?;
        verify_against_seed(seed, name, &state)?;
        streams.insert(name.clone(), state);
    }
    for stream in REQUIRED_COMBAT_STREAMS {
        if !streams.contains_key(stream) {
            return Err(EntryRefusal::MissingRngStream(stream));
        }
    }
    Ok(streams)
}

/// The recorded words are the state `RunRngSet` derives at the recorded
/// counter (see the module comment).
fn verify_against_seed(seed: &str, name: &str, state: &SerializedRng) -> Result<(), EntryRefusal> {
    if state.counter > MAX_VERIFIED_STREAM_COUNTER {
        return Err(EntryRefusal::FactsStreamCounterUnverifiable {
            stream: name.to_string(),
            counter: state.counter,
        });
    }
    let mut derived = crate::rng::run_stream_at_zero(seed, name);
    for _ in 0..state.counter {
        derived.next_u64();
    }
    if derived.words != [state.s0, state.s1, state.s2, state.s3] {
        return Err(EntryRefusal::FactsStreamDisagreesWithSeed {
            stream: name.to_string(),
        });
    }
    Ok(())
}

fn counters(
    value: &Value,
    streams: &BTreeMap<String, SerializedRng>,
) -> Result<BTreeMap<String, u64>, EntryRefusal> {
    let object = value
        .as_object()
        .ok_or_else(|| unknown("counters is not an object"))?;
    let mut counters = BTreeMap::new();
    for (name, counter) in object {
        let counter = counter
            .as_u64()
            .ok_or_else(|| unknown(format!("counters.{name} is not an unsigned integer")))?;
        counters.insert(name.clone(), counter);
    }
    let recorded: BTreeMap<String, u64> = streams
        .iter()
        .map(|(name, state)| (name.clone(), state.counter))
        .collect();
    if counters != recorded {
        return Err(inconsistent(
            "counters",
            "is not exactly the streams' counters (the save path's stream_counters)",
        ));
    }
    Ok(counters)
}

/// `unlocks::card_pool_unlocked_epochs`'s normal form: `EPOCH.`-stripped,
/// non-empty, sorted and deduplicated.
fn epochs(value: &Value) -> Result<Option<Vec<String>>, EntryRefusal> {
    let Value::Array(items) = value else {
        return match value {
            Value::Null => Ok(None),
            _ => Err(unknown(
                "unlocks.unlocked_card_pool_epochs is not an array or null",
            )),
        };
    };
    let names = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            string(item, &format!("unlocks.unlocked_card_pool_epochs[{index}]")).map(str::to_string)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let normal = names
        .iter()
        .all(|name| !name.is_empty() && !name.starts_with("EPOCH."))
        && names.windows(2).all(|pair| pair[0] < pair[1]);
    if !normal {
        return Err(inconsistent(
            "unlocks.unlocked_card_pool_epochs",
            "is not EPOCH.-stripped, non-empty, sorted and deduplicated",
        ));
    }
    Ok(Some(names))
}

impl EntryDocument {
    /// The facts-input wire form: [`EntryDocument::to_json`] plus `streams`,
    /// and `map_point_type` / `next_normal_encounter` when the save named them.
    pub fn facts_json(&self) -> Value {
        let mut value = self.to_json();
        if let Some(kind) = &self.map_point_type {
            value
                .as_object_mut()
                .expect("an entry document")
                .insert("map_point_type".to_string(), Value::String(kind.clone()));
        }
        if let Some(encounter) = &self.next_normal_encounter {
            value.as_object_mut().expect("an entry document").insert(
                "next_normal_encounter".to_string(),
                Value::String(encounter.clone()),
            );
        }
        value.as_object_mut().expect("an entry document").insert(
            "streams".to_string(),
            Value::Object(
                self.streams
                    .iter()
                    .map(|(name, state)| {
                        let mut row = Map::new();
                        row.insert("counter".to_string(), Value::from(state.counter));
                        row.insert("s0".to_string(), Value::from(state.s0));
                        row.insert("s1".to_string(), Value::from(state.s1));
                        row.insert("s2".to_string(), Value::from(state.s2));
                        row.insert("s3".to_string(), Value::from(state.s3));
                        (name.clone(), Value::Object(row))
                    })
                    .collect(),
            ),
        );
        value
    }
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
