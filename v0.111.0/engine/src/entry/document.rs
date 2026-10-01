//! The `sts-sim-entry-v1` wire document.
//!
//! # What it is
//!
//! Exactly the argument bundle `mcr_replay.start_from_save` hands
//! `combat_sim.start_combat`: the `build_entry` dictionary under `entry`, the
//! recorded stream counters under `counters`, the seed, the character, and the
//! three unlock-profile answers. Key-for-key with `build_entry`, so the census
//! can compare the two documents by value rather than by eye, and can feed
//! Rust's entry straight into the oracle's opening.
//!
//! # Why this is not `sts-sim-canonical-v2`
//!
//! A canonical v2 document is a *post-opening* combat state: shuffled piles,
//! a dealt hand, created monsters with rolled HP, fired room-entry hooks. This
//! slice builds none of that (see `entry/mod.rs`), so emitting a v2 document
//! would be a lie about what was computed. The schema is versioned instead:
//! when the opening lands, it emits `sts-sim-canonical-v2` and this schema
//! keeps its own meaning.
//!
//! # I6: the opening-checksum splice must never be silent
//!
//! `mcr_validate.checksum_opening_state` overwrites **all nine** RNG
//! attributes unconditionally from the capture's first checksum, *after* the
//! opening has run. So a document built with the splice and one built without
//! are different documents, and the splice masks RNG-counter drift in the
//! opening while leaving pile and monster drift visible — a simulator that
//! consumed the wrong number of Shuffle draws would still show a wrong hand,
//! but its counters would look right.
//!
//! Every document therefore carries an explicit `opening` block saying whether
//! an opening was built and whether a splice was applied. On **this** schema
//! `built` is always `false` by construction — a built opening emits
//! `sts-sim-canonical-v2` instead (`entry::RootOutcome`) — and when the
//! opening was attempted and refused, `entry::RootOutcome` adds the refusal's
//! class and detail to the same block. The flag can never be quietly
//! ignored.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::canonical::{Refusal, RefusalKind};
use crate::catalog::GameBuild;
use crate::entry::belt::Belt;
use crate::entry::deck::DeckEntry;
use crate::entry::refusal::EntryRefusal;
use crate::entry::relics::RelicEntry;
use crate::entry::save::SerializedRng;

/// Wire schema identifier. Bumped, never reinterpreted.
pub const ENTRY_SCHEMA_V1: &str = "sts-sim-entry-v1";

/// The refusal `site` every entry refusal carries.
pub const ENTRY_SITE: &str = "entry";

/// The entry facts of one fight.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryDocument {
    pub game_build: GameBuild,
    pub save_schema_version: u32,
    pub seed: String,
    pub character: Option<String>,
    pub encounter_id: String,
    /// `None` when the save's act map does not name the node's kind; the
    /// oracle's `build_entry` writes `ntype or "monster"`, so the document
    /// renders `"monster"` there.
    pub node_type: Option<String>,
    pub node_index: i64,
    pub deck_entering: Vec<DeckEntry>,
    pub relic_entry: RelicEntry,
    pub belt: Belt,
    pub hp_entering: i64,
    pub max_hp_entering: i64,
    pub gold_entering: i64,
    /// The save's `ascension` (`RunState.AscensionLevel`), verbatim: `None`
    /// when the save omits it. `build_entry` hands it to `start_combat` as
    /// `entry["ascension"]` (#2539), which refuses anything but an exact
    /// level; the opening does the same.
    pub ascension: Option<i64>,
    pub unlocked_card_pool_epochs: Option<Vec<String>>,
    pub fully_unlocked_card_pool: Option<bool>,
    pub fully_unlocked_potion_pool: Option<bool>,
    pub counters: BTreeMap<String, u64>,
    /// Every recorded run stream's full state — counter **and** its four
    /// xoshiro words.
    ///
    /// Not on the wire: `counters` is what `build_entry` hands the oracle, and
    /// bumping the entry schema to carry words would change what
    /// `--root-with rust` measures. The opening reads this instead of deriving
    /// the nine streams from a seeding scheme, which is what keeps
    /// `rng::stream_seed` uncalled (see `entry/counters.rs`).
    pub streams: BTreeMap<String, SerializedRng>,
    /// The current node's saved map point type, lowercased
    /// ([`crate::entry::node::map_point_type`]), read whether or not the
    /// caller supplied `node_type` (#3162). `None` when the save does not say.
    ///
    /// Not on the wire either, for the reason [`Self::streams`] gives; the
    /// facts input carries it as an optional top-level `map_point_type`.
    /// Read only by the opening's point-conditional relic gates (Planisphere).
    pub map_point_type: Option<String>,
    /// The encounter the act schedules next for a `monster` room
    /// (`normal_encounter_ids[normal_encounters_visited]`,
    /// [`crate::entry::node::next_encounter`]), read whether or not the
    /// caller supplied the encounter (#3162). `None` when the save's room set
    /// does not name one. Off the wire like [`Self::map_point_type`]; the facts
    /// input carries it as an optional top-level `next_normal_encounter`.
    ///
    /// An `unknown` (`?`) point whose fight IS this encounter resolved
    /// straight into its monster room, which is then the point's first room.
    pub next_normal_encounter: Option<String>,
}

fn optional_bool(value: Option<bool>) -> Value {
    value.map_or(Value::Null, Value::Bool)
}

impl EntryDocument {
    /// The `build_entry` dictionary, key for key.
    pub fn entry_json(&self) -> Value {
        let mut entry = Map::new();
        entry.insert(
            "encounter_id".to_string(),
            Value::String(self.encounter_id.clone()),
        );
        entry.insert("node_index".to_string(), Value::from(self.node_index));
        entry.insert(
            "node_type".to_string(),
            Value::String(
                self.node_type
                    .clone()
                    .unwrap_or_else(|| "monster".to_string()),
            ),
        );
        entry.insert(
            "deck_entering".to_string(),
            Value::Array(self.deck_entering.iter().map(DeckEntry::to_json).collect()),
        );
        entry.insert(
            "relics_entering".to_string(),
            Value::Array(
                self.relic_entry
                    .relics_entering
                    .iter()
                    .map(|id| Value::String(id.clone()))
                    .collect(),
            ),
        );
        entry.insert(
            "potions_entering".to_string(),
            Value::Array(
                self.belt
                    .potions_entering()
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        entry.insert(
            "max_potion_slot_count".to_string(),
            self.belt
                .max_potion_slot_count
                .map_or(Value::Null, Value::from),
        );
        entry.insert(
            "potion_slots_entering".to_string(),
            Value::Array(self.belt.slots.iter().map(|slot| slot.to_json()).collect()),
        );
        entry.insert("hp_entering".to_string(), Value::from(self.hp_entering));
        entry.insert(
            "max_hp_entering".to_string(),
            Value::from(self.max_hp_entering),
        );
        entry.insert("gold_entering".to_string(), Value::from(self.gold_entering));
        entry.insert(
            "ascension".to_string(),
            self.ascension.map_or(Value::Null, Value::from),
        );
        // `relic_counters or None`: an empty bag is absent, not empty.
        entry.insert(
            "relic_counters".to_string(),
            if self.relic_entry.relic_counters.is_empty() {
                Value::Null
            } else {
                Value::Object(
                    self.relic_entry
                        .relic_counters
                        .iter()
                        .map(|(id, value)| (id.clone(), value.clone()))
                        .collect(),
                )
            },
        );
        entry.insert(
            "tea_set_charged".to_string(),
            optional_bool(self.relic_entry.tea_set_charged),
        );
        entry.insert(
            "fake_tea_set_charged".to_string(),
            optional_bool(self.relic_entry.fake_tea_set_charged),
        );
        entry.insert(
            "fur_coat_active".to_string(),
            optional_bool(self.relic_entry.fur_coat_active),
        );
        entry.insert(
            "relics_entering_dispatch_ordered".to_string(),
            Value::Bool(self.relic_entry.dispatch_ordered),
        );
        // Always false from a save: the ambiguity it flags belongs to the
        // `.run` reconstruction, where an event node's own effects are not
        // ordered against the combat. A save is the node-entry snapshot.
        entry.insert("entry_ambiguous".to_string(), Value::Bool(false));
        Value::Object(entry)
    }

    /// The whole document.
    pub fn to_json(&self) -> Value {
        let mut document = Map::new();
        document.insert(
            "schema".to_string(),
            Value::String(ENTRY_SCHEMA_V1.to_string()),
        );
        document.insert(
            "game_build".to_string(),
            Value::String(self.game_build.as_str().to_string()),
        );
        document.insert(
            "save_schema_version".to_string(),
            Value::from(self.save_schema_version),
        );
        document.insert("seed".to_string(), Value::String(self.seed.clone()));
        document.insert(
            "character".to_string(),
            self.character.clone().map_or(Value::Null, Value::String),
        );
        document.insert("entry".to_string(), self.entry_json());
        document.insert(
            "counters".to_string(),
            Value::Object(
                self.counters
                    .iter()
                    .map(|(name, counter)| (name.clone(), Value::from(*counter)))
                    .collect(),
            ),
        );
        let mut unlocks = Map::new();
        unlocks.insert(
            "unlocked_card_pool_epochs".to_string(),
            self.unlocked_card_pool_epochs
                .clone()
                .map_or(Value::Null, |epochs| {
                    Value::Array(epochs.into_iter().map(Value::String).collect())
                }),
        );
        unlocks.insert(
            "fully_unlocked_card_pool".to_string(),
            optional_bool(self.fully_unlocked_card_pool),
        );
        unlocks.insert(
            "fully_unlocked_potion_pool".to_string(),
            optional_bool(self.fully_unlocked_potion_pool),
        );
        document.insert("unlocks".to_string(), Value::Object(unlocks));
        let mut opening = Map::new();
        opening.insert("built".to_string(), Value::Bool(false));
        opening.insert("mcr_splice_applied".to_string(), Value::Bool(false));
        opening.insert(
            "detail".to_string(),
            Value::String(
                "these are entry facts only; the run-RNG opening (shuffle, monster \
                 creation, room-entry and before-combat hooks, the first player turn) \
                 emits sts-sim-canonical-v2 when it builds"
                    .to_string(),
            ),
        );
        document.insert("opening".to_string(), Value::Object(opening));
        Value::Object(document)
    }
}

/// The wire form of a refusal: the canonical `site`/`kind`/`detail` triple,
/// plus the census class alongside it (not inside, so the object still parses
/// as a [`Refusal`]).
pub fn refusal_json(refusal: &EntryRefusal) -> Value {
    let canonical = Refusal::new(
        ENTRY_SITE,
        RefusalKind::EntryNotBuildable,
        refusal.to_string(),
    );
    let mut document = Map::new();
    document.insert(
        "refusal".to_string(),
        serde_json::to_value(&canonical).expect("a refusal serializes"),
    );
    document.insert(
        "refusal_class".to_string(),
        Value::String(refusal.class().to_string()),
    );
    Value::Object(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_document_parses_back_as_a_canonical_refusal() {
        let value = refusal_json(&EntryRefusal::UnsupportedSaveSchema(19));
        let refusal: Refusal = serde_json::from_value(value["refusal"].clone()).unwrap();
        assert_eq!(refusal.site, ENTRY_SITE);
        assert_eq!(refusal.kind, RefusalKind::EntryNotBuildable);
        assert!(refusal.detail.contains("19"));
        assert_eq!(
            value["refusal_class"],
            Value::from("unsupported_save_schema")
        );
    }

    #[test]
    fn the_opening_block_is_always_present_and_honest() {
        let document = EntryDocument {
            game_build: GameBuild::V0_111_0,
            save_schema_version: 20,
            seed: "ZPJHU3WSH2".to_string(),
            character: None,
            encounter_id: "ENCOUNTER.TOADPOLES_WEAK".to_string(),
            node_type: None,
            node_index: 0,
            deck_entering: Vec::new(),
            relic_entry: RelicEntry::default(),
            belt: Belt::default(),
            hp_entering: 75,
            max_hp_entering: 75,
            gold_entering: 99,
            ascension: Some(10),
            unlocked_card_pool_epochs: None,
            fully_unlocked_card_pool: None,
            fully_unlocked_potion_pool: None,
            counters: BTreeMap::new(),
            streams: BTreeMap::new(),
            map_point_type: None,
            next_normal_encounter: None,
        };
        let value = document.to_json();
        assert_eq!(value["schema"], Value::from(ENTRY_SCHEMA_V1));
        assert_eq!(value["opening"]["built"], Value::Bool(false));
        assert_eq!(value["opening"]["mcr_splice_applied"], Value::Bool(false));
        // `ntype or "monster"`.
        assert_eq!(value["entry"]["node_type"], Value::from("monster"));
        // An empty counter bag is absent, not `{}`.
        assert_eq!(value["entry"]["relic_counters"], Value::Null);
        assert_eq!(value["entry"]["entry_ambiguous"], Value::Bool(false));
        // `save.get("ascension")`, verbatim (#2539); absent is `null`, which
        // `start_combat` and the opening both refuse.
        assert_eq!(value["entry"]["ascension"], Value::from(10));
        let mut absent = document;
        absent.ascension = None;
        assert_eq!(absent.to_json()["entry"]["ascension"], Value::Null);
    }
}
