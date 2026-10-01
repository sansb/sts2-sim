//! Build a fight's entry facts from the game's own save bytes (#2511,
//! #1282 D2).
//!
//! # What this module is for
//!
//! Before this module the crate had **no filesystem input at all**: every root
//! it ever ran was a `CanonicalStateV2` that Python built, so Python's entry
//! refusals were Rust's ceiling — 318 of 513 corpus captures ever reached the
//! engine. #1282 D2 moves that boundary: *"Rust builds its own root from the
//! save / upload payload. Python's entry refusals stop gating Rust; Rust
//! carries its own typed I5 refusals at that boundary; the census measures
//! admission from 513, not 318."*
//!
//! # The two halves, and the two schemas
//!
//! The oracle's `combat_sim.start_combat` splits in two at the line where the
//! run RNG is first constructed. Everything **above** that line is pure entry
//! construction — the deck, the relic inventory and its persistent counters,
//! the belt, the unlock profile, the node and its encounter, the recorded
//! stream counters. Everything **below** it is the run-RNG opening: the
//! shuffle, monster creation with rolled HP, the room-entry and before-combat
//! relic hooks, and the first `begin_player_turn`.
//!
//! This module owns the first half (#2511 E1–E3) and [`opening`] owns the
//! second (#2528 E4a). They emit **different schemas**, and that is the whole
//! contract: [`build`] answers `sts-sim-entry-v1`, exactly the argument bundle
//! `mcr_replay.start_from_save` hands `start_combat`, so the census can root a
//! fight from Rust's entry facts and Python's opening. [`build_root`] answers
//! `sts-sim-canonical-v2` when the opening builds — a post-`start_combat`
//! state whose digest is directly comparable with the oracle's — and the
//! entry document with a named opening refusal when it does not.
//!
//! `sts-sim-entry-v1` is never reinterpreted to mean "post-opening" (I12): a
//! reader can always tell which half produced what it is holding.
//!
//! # I5 in this module
//!
//! Rust does not inherit Python's refusals, and does not inherit its
//! admissions either. Every fact is re-derived from the save or refused by a
//! Rust name from [`refusal::EntryRefusal`]. Where the oracle answers
//! "unprovable" (an unreadable relic property bag, an absent unlock profile)
//! this module answers unprovable too — that is an entry fact, not a refusal,
//! and `start_combat`'s own gates decide whether the fight is runnable.

pub mod belt;
pub mod capture;
pub mod cli;
pub mod counters;
pub mod deck;
pub mod document;
pub mod event_combat;
pub mod facts;
#[cfg(test)]
mod fixture_tests;
pub mod node;
pub mod opening;
pub mod props;
pub mod provenance;
pub mod refusal;
pub mod relics;
pub mod save;
pub mod unlocks;

use serde_json::Value;

use crate::catalog::GameBuild;
use crate::entry::document::EntryDocument;
use crate::entry::refusal::EntryRefusal;
use crate::entry::save::SerializedRun;

/// The bytes a fight root is built from.
///
/// Both carry one `Saves.SerializableRun`. A capture run is mapped onto the
/// save surface by [`capture::parse`] before any reader sees it, so every
/// entry fact below is read from the same typed [`SerializedRun`] either way.
#[derive(Clone, Copy, Debug)]
pub enum EntryInput<'a> {
    /// A schema-20 `.save` file's JSON.
    Save(&'a str),
    /// A replay capture's decoded `CombatReplay.run` (`mcr_parser` JSON).
    CaptureRun(&'a str),
}

impl EntryInput<'_> {
    fn parse(self) -> Result<SerializedRun, EntryRefusal> {
        match self {
            Self::Save(text) => SerializedRun::parse(text),
            Self::CaptureRun(text) => capture::parse(text),
        }
    }
}

/// One request to build a fight root.
#[derive(Clone, Copy, Debug)]
pub struct EntryRequest<'a> {
    /// The run the fight is entered from.
    pub input: EntryInput<'a>,
    /// The encounter to enter. When absent it is derived from the act's room
    /// lists, which only combat nodes carry.
    pub encounter_id: Option<&'a str>,
    /// The node kind. When absent it is read from the act's saved map.
    pub node_type: Option<&'a str>,
    /// The build the save was recorded under. **No default** (I11): a fifth
    /// call site with an implicit build would reopen the hole #1265 closed.
    pub game_build: GameBuild,
    /// Whether an opening-checksum RNG splice was requested.
    pub mcr_splice: bool,
}

/// The result of one request.
#[derive(Clone, Debug, PartialEq)]
pub enum EntryOutcome {
    Built(Box<EntryDocument>),
    Refused(EntryRefusal),
}

impl EntryOutcome {
    /// The wire form: the document, or `{"refusal": {...}}`.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Built(document) => document.to_json(),
            Self::Refused(refusal) => document::refusal_json(refusal),
        }
    }

    pub fn refusal(&self) -> Option<&EntryRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Built(_) => None,
        }
    }
}

/// The one public entry point: save bytes in, entry document or typed refusal
/// out.
pub fn build(request: &EntryRequest<'_>) -> EntryOutcome {
    match build_document(request) {
        Ok(document) => EntryOutcome::Built(Box::new(document)),
        Err(refusal) => EntryOutcome::Refused(refusal),
    }
}

/// The wire schema of a **spliced** opening.
///
/// Bumped, never reinterpreted, and deliberately distinct from
/// `sts-sim-canonical-v2`: a document whose nine RNG streams were overwritten
/// from a capture's first checksum is not the document the opening computed,
/// and the pre-splice streams it carries are the sharp regression net that
/// would otherwise be masked (§A5 obligation 3).
pub const SPLICED_OPENING_SCHEMA_V1: &str = "sts-sim-spliced-opening-v1";

/// The wire schema of an unspliced opening **with the native checkpoints its
/// deal passed** (`entry --opening --native-checkpoints`, #3392).
///
/// Opt-in and deliberately a wrapper, never extra keys on the bare document:
/// the root under `state` stays byte for byte the `sts-sim-canonical-v2`
/// document `--opening` prints, so its digest is unchanged. Beside it,
/// `native_checkpoints` lists each boundary as `{"kind","state"}` (a `null`
/// `state` carries the boundary's `refusal`). The one opening boundary is
/// `after_player_turn_start`: native's opening checksum, taken before turn
/// one's `RunAutoPrePlayPhase` (`engine::native_checkpoint`).
pub const OPENING_CHECKPOINTS_SCHEMA_V1: &str = "sts-sim-opening-checkpoints-v1";

/// The result of rooting one fight all the way through the opening.
#[derive(Clone, Debug, PartialEq)]
pub enum RootOutcome {
    /// Entry facts **and** the opening: a post-`start_combat` state, in the
    /// wire contract both engines share.
    Opened(Box<opening::Opening>),
    /// Entry facts built, opening refused by name. The entry document is still
    /// the honest answer for what *was* computed, and its `opening` block
    /// carries the refusal.
    EntryOnly(Box<EntryDocument>, opening::refusal::OpeningRefusal),
    /// The entry facts themselves could not be built.
    Refused(EntryRefusal),
}

impl RootOutcome {
    /// The wire form.
    ///
    /// **The schema says which half ran, and whether it was spliced**
    /// (I12, §A6, §A5 obligation 2). Three shapes, never overlapping:
    ///
    /// | shape | schema |
    /// |---|---|
    /// | opening built, no splice | `sts-sim-canonical-v2` |
    /// | opening built, `--mcr` splice applied | `sts-sim-spliced-opening-v1`, with the state under `state` |
    /// | entry facts only | `sts-sim-entry-v1`, `opening.built = false` |
    /// | opening built, `--native-checkpoints` | `sts-sim-opening-checkpoints-v1`, the state under `state` beside `native_checkpoints` (#3392) |
    ///
    /// The unspliced case is the **bare** canonical document, byte for byte,
    /// because its digest is what acceptance compares with the oracle's. The
    /// spliced case is deliberately a different schema rather than the same
    /// document with a flag: `mcr_validate.checksum_opening_state` overwrites
    /// all nine RNG streams *after* the opening has run, so the two are
    /// different documents and a reader must not be able to mistake one for
    /// the other. `sts-sim-entry-v1` is never reinterpreted to mean
    /// "post-opening".
    pub fn to_json(&self) -> Value {
        match self {
            Self::Opened(opening) => {
                let state =
                    serde_json::to_value(&opening.document).expect("a canonical state serializes");
                if let Some(recorded) = &opening.native_checkpoints {
                    // The CLI refuses `--native-checkpoints` beside `--mcr`;
                    // a spliced opening never records.
                    assert!(
                        !opening.mcr_splice_applied,
                        "a spliced opening records no native checkpoints"
                    );
                    let mut wrapper = serde_json::Map::new();
                    wrapper.insert(
                        "schema".to_string(),
                        Value::String(OPENING_CHECKPOINTS_SCHEMA_V1.to_string()),
                    );
                    wrapper.insert("state".to_string(), state);
                    wrapper.insert(
                        "native_checkpoints".to_string(),
                        Value::Array(recorded.iter().map(opening_checkpoint_json).collect()),
                    );
                    return Value::Object(wrapper);
                }
                if !opening.mcr_splice_applied {
                    return state;
                }
                let mut wrapper = serde_json::Map::new();
                wrapper.insert(
                    "schema".to_string(),
                    Value::String(SPLICED_OPENING_SCHEMA_V1.to_string()),
                );
                wrapper.insert("mcr_splice_applied".to_string(), Value::Bool(true));
                wrapper.insert(
                    "pre_splice_rng".to_string(),
                    serde_json::to_value(&opening.pre_splice_rng).expect("stream states serialize"),
                );
                wrapper.insert("state".to_string(), state);
                Value::Object(wrapper)
            }
            Self::EntryOnly(document, refusal) => {
                let mut value = document.to_json();
                if let Some(object) = value.as_object_mut() {
                    let mut block = serde_json::Map::new();
                    block.insert("built".to_string(), Value::Bool(false));
                    block.insert("mcr_splice_applied".to_string(), Value::Bool(false));
                    block.insert("refusal_class".to_string(), Value::String(refusal.class()));
                    block.insert("detail".to_string(), Value::String(refusal.to_string()));
                    object.insert("opening".to_string(), Value::Object(block));
                }
                value
            }
            Self::Refused(refusal) => document::refusal_json(refusal),
        }
    }
}

/// One recorded opening checkpoint on the wire (#3392): `{"kind","state"}`,
/// or a `null` `state` with the boundary's `refusal`, so a reader fails that
/// checkpoint by name rather than silently skipping it.
fn opening_checkpoint_json(checkpoint: &opening::OpeningNativeCheckpoint) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert(
        "kind".to_string(),
        Value::String(checkpoint.kind.as_str().to_string()),
    );
    match &checkpoint.state {
        Ok(document) => {
            entry.insert(
                "state".to_string(),
                serde_json::to_value(document).expect("a canonical state serializes"),
            );
        }
        Err(refusal) => {
            entry.insert("state".to_string(), Value::Null);
            entry.insert("refusal".to_string(), Value::String(refusal.clone()));
        }
    }
    Value::Object(entry)
}

/// Root one fight from its save: entry facts, then the run-RNG opening.
///
/// This is the #1282 D2 boundary at full width. [`build`] stops at the entry
/// facts and is what `--root-with rust` measures today; this continues into
/// the opening and emits the post-`start_combat` state.
pub fn build_root(
    request: &EntryRequest<'_>,
    mcr_first_checksum: Option<&opening::mcr::McrOpeningChecksum>,
) -> RootOutcome {
    match build_document(request) {
        Ok(document) => root_from_document(document, mcr_first_checksum),
        Err(refusal) => RootOutcome::Refused(refusal),
    }
}

/// The opening on entry facts already in hand: those [`build_document`] read
/// from a run, or those [`facts::parse`] validated from an entry-facts
/// document.
pub fn root_from_document(
    document: EntryDocument,
    mcr_first_checksum: Option<&opening::mcr::McrOpeningChecksum>,
) -> RootOutcome {
    root_from_document_with(
        document,
        &opening::OpeningOptions {
            mcr_first_checksum,
            record_native_checkpoints: false,
        },
    )
}

/// [`build_root`] under explicit [`opening::OpeningOptions`] (#3392).
pub fn build_root_with(
    request: &EntryRequest<'_>,
    options: &opening::OpeningOptions<'_>,
) -> RootOutcome {
    match build_document(request) {
        Ok(document) => root_from_document_with(document, options),
        Err(refusal) => RootOutcome::Refused(refusal),
    }
}

/// [`root_from_document`] under explicit [`opening::OpeningOptions`] (#3392).
pub fn root_from_document_with(
    document: EntryDocument,
    options: &opening::OpeningOptions<'_>,
) -> RootOutcome {
    match opening::build(&document, options) {
        Ok(opened) => RootOutcome::Opened(Box::new(opened)),
        Err(refusal) => RootOutcome::EntryOnly(Box::new(document), refusal),
    }
}

fn build_document(request: &EntryRequest<'_>) -> Result<EntryDocument, EntryRefusal> {
    let run = request.input.parse()?;
    let player = run.single_player()?;

    let node_index = node::current_node(&run)?;
    let derived_kind = match request.node_type {
        Some(_) => None,
        None => node::node_type(&run)?,
    };
    let kind: Option<String> = request
        .node_type
        .map(str::to_string)
        .or(derived_kind.clone());
    let encounter_id = match request.encounter_id {
        Some(id) => id.to_string(),
        None => {
            let kind = kind.clone().ok_or(EntryRefusal::NoEncounterForNode {
                node_type: "<no saved map>".to_string(),
            })?;
            node::next_encounter(&run, &kind)?
        }
    };

    let deck_entering = deck::deck_entering(player)?;
    let relic_entry = relics::relic_entry(&run, player)?;
    let belt = belt::belt(player)?;
    let streams = counters::run_streams(&run)?;
    let hp_saved = require(player.current_hp, "player", "current_hp")?;
    let max_hp_entering = require(player.max_hp, "player", "max_hp")?;
    // #3248: an event-started combat is entered after the event's option ran,
    // and the save predates the event. Admitted only on an IL-read option
    // chain whose pre-combat effects are modeled; refused by name otherwise.
    let modifiers: Vec<String> = run
        .modifiers
        .iter()
        .map(|modifier| modifier.id.clone().unwrap_or_else(|| "<no id>".to_string()))
        .collect();
    let hp_entering = event_combat::hp_entering(
        &encounter_id,
        &event_combat::EventRootFacts {
            relics: &relic_entry.relics_entering,
            deck: &deck_entering,
            modifiers: &modifiers,
            hp: hp_saved,
            max_hp: max_hp_entering,
        },
    )?;

    Ok(EntryDocument {
        game_build: request.game_build,
        save_schema_version: run.schema_version.unwrap_or_default(),
        seed: streams.seed.clone(),
        character: unlocks::player_character_id(player),
        encounter_id,
        // `build_entry` writes `ntype or "monster"`: an unnamed node enters as
        // a monster room, which is the shape every non-boss/elite fight takes.
        node_type: kind.filter(|kind| !kind.is_empty()),
        node_index,
        deck_entering,
        relic_entry,
        belt,
        hp_entering,
        max_hp_entering,
        gold_entering: require(player.gold, "player", "gold")?,
        // `build_entry` writes `save.get("ascension")`: absent is `None`, and
        // the opening (like `start_combat`) refuses it rather than guess.
        ascension: run.ascension,
        unlocked_card_pool_epochs: unlocks::card_pool_unlocked_epochs(player),
        fully_unlocked_card_pool: unlocks::infernal_blade_pool_fully_unlocked(player),
        fully_unlocked_potion_pool: unlocks::potion_pool_fully_unlocked(player),
        counters: streams.counters(),
        streams: streams.streams.clone(),
        map_point_type: node::map_point_type(&run),
        next_normal_encounter: node::next_encounter(&run, "monster").ok(),
    })
}

fn require<T>(
    value: Option<T>,
    owner: &'static str,
    field: &'static str,
) -> Result<T, EntryRefusal> {
    value.ok_or(EntryRefusal::MissingSaveField { owner, field })
}

/// Normalize one JSON model entry to its full model id.
///
/// Oracle: `live_coach._canonical_model_id`. An id already carrying exactly
/// its own category is returned unchanged, a bare name gains the category, and
/// anything else — an empty string, a second dot, another category's prefix —
/// is a refusal rather than a repair.
pub(crate) fn canonical_model_id(
    value: Option<&str>,
    category: &'static str,
) -> Result<String, EntryRefusal> {
    let malformed = |value: &str| EntryRefusal::MalformedModelId {
        category,
        value: value.to_string(),
    };
    let value = value.ok_or_else(|| malformed(""))?;
    if value.is_empty() {
        return Err(malformed(value));
    }
    let prefix = format!("{category}.");
    if value.starts_with(&prefix) && value.matches('.').count() == 1 {
        return Ok(value.to_string());
    }
    if !value.contains('.') {
        return Ok(format!("{prefix}{value}"));
    }
    Err(malformed(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_normalize_or_refuse() {
        assert_eq!(
            canonical_model_id(Some("CARD.BASH"), "CARD").unwrap(),
            "CARD.BASH"
        );
        assert_eq!(
            canonical_model_id(Some("BASH"), "CARD").unwrap(),
            "CARD.BASH"
        );
        assert!(canonical_model_id(Some("RELIC.VAJRA"), "CARD").is_err());
        assert!(canonical_model_id(Some("CARD.A.B"), "CARD").is_err());
        assert!(canonical_model_id(Some(""), "CARD").is_err());
        assert!(canonical_model_id(None, "CARD").is_err());
    }

    #[test]
    fn the_root_builder_carries_the_opening_refusal_beside_the_entry_facts() {
        // A malformed save never reaches the opening at all, so the refusal
        // is still the entry's.
        let outcome = build_root(
            &EntryRequest {
                input: EntryInput::Save("{}"),
                encounter_id: None,
                node_type: None,
                game_build: GameBuild::V0_111_0,
                mcr_splice: true,
            },
            None,
        );
        assert!(matches!(outcome, RootOutcome::Refused(_)));
        let value = outcome.to_json();
        assert_eq!(value["refusal"]["kind"], Value::from("entry_not_buildable"));
    }

    #[test]
    fn a_built_entry_whose_opening_refuses_keeps_the_v1_schema() {
        // #2528 replaced `McrSpliceWithoutOpening` with a real splice step, so
        // the schema — not a refusal variant — is what says which half ran.
        let outcome = RootOutcome::EntryOnly(
            Box::new(document::EntryDocument {
                game_build: GameBuild::V0_111_0,
                save_schema_version: 20,
                seed: "ZPJHU3WSH2".to_string(),
                character: None,
                encounter_id: "ENCOUNTER.TOADPOLES_WEAK".to_string(),
                node_type: None,
                node_index: 0,
                deck_entering: Vec::new(),
                relic_entry: relics::RelicEntry::default(),
                belt: belt::Belt::default(),
                hp_entering: 75,
                max_hp_entering: 75,
                gold_entering: 0,
                ascension: Some(10),
                unlocked_card_pool_epochs: None,
                fully_unlocked_card_pool: None,
                fully_unlocked_potion_pool: None,
                counters: std::collections::BTreeMap::new(),
                streams: std::collections::BTreeMap::new(),
                map_point_type: None,
                next_normal_encounter: None,
            }),
            opening::refusal::OpeningRefusal::StoneCrackerUpgradeLadderUnknown {
                card: "X+0".to_string(),
            },
        );
        let value = outcome.to_json();
        assert_eq!(value["schema"], Value::from(document::ENTRY_SCHEMA_V1));
        assert_eq!(value["opening"]["built"], Value::Bool(false));
        assert_eq!(
            value["opening"]["refusal_class"],
            Value::from("stone_cracker_upgrade_ladder_unknown")
        );
    }
}
