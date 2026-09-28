//! The review-provenance **entry projection**, and what it cannot answer.
//!
//! # What this input is
//!
//! `versions/v0.111.0/solver/REVIEW_PROVENANCE_PAYLOAD.md` specifies the v2
//! uploader payload: each captured fight carries its `.mcr` bytes plus at most
//! two `entry_candidates`, each holding an `entry` object projected from a
//! neighbouring save. It is deliberately a **projection**, not a save — v1's
//! timestamp heuristic labelled one projection authoritative and the live
//! smoke test found it post-combat in 6 of 7 fights, so v2 carries unresolved
//! candidates and no entry-state assertion at all.
//!
//! This module exists for one acceptance question on #2511: the
//! `7UEE3Y1SPJCY` floor-33 fight (The Insatiable, Defect, ascension 9 — the
//! motivating case in #1282's DECIDED comment) must either root from its
//! save-projected entry or **refuse by a name that identifies the missing
//! fact**. The floor-33 start save itself sits behind an owner-token download,
//! so the projection is the input that is actually in hand.
//!
//! # The answer, measured
//!
//! The projection carries the character, the deck (ids, upgrade levels,
//! enchantments), the relic inventory in acquisition order with its saved
//! properties, the unlock epochs, and all twelve run RNG streams with counters
//! **and** words. It does not carry the player's HP, max HP or gold, any
//! potion or belt size, the run seed, the visited-coordinate/act-history pair
//! the global node index comes from, or the act room lists the encounter comes
//! from. So the refusal is not "Rust cannot build this" — it is "this payload
//! records fewer facts than a fight root needs", and [`missing_facts`] names
//! all of them so the uploader contract can be extended deliberately.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{ADMITTED_SAVE_SCHEMA, SerializedRng};

/// One `entry_candidates[].entry` projection.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEntry {
    #[serde(default)]
    pub save_schema_version: Option<u32>,
    #[serde(default)]
    pub run_rng: BTreeMap<String, SerializedRng>,
    #[serde(default)]
    pub shared_relic_grab_bag: Option<Value>,
    #[serde(default)]
    pub players: Option<Vec<ProvenancePlayer>>,
}

/// One projected player.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProvenancePlayer {
    #[serde(default)]
    pub net_id: Option<i64>,
    #[serde(default)]
    pub character_id: Option<String>,
    #[serde(default)]
    pub deck: Vec<ProvenanceCard>,
    #[serde(default)]
    pub relics: Vec<ProvenanceRelic>,
    #[serde(default)]
    pub unlock_state: Option<Value>,
    #[serde(default)]
    pub relic_grab_bag: Option<Value>,
}

/// One projected deck row.
///
/// `copy_index` is the capture-local identity of the array element, and the
/// upgrade level is spelled `upgrade_level` here — the decoded-MCR key — not
/// `current_upgrade_level` as a save writes it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceCard {
    #[serde(default)]
    pub copy_index: Option<i64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub upgrade_level: Option<i64>,
    #[serde(default)]
    pub enchantment: Option<Value>,
    #[serde(default)]
    pub floor_added_to_deck: Option<i64>,
}

/// One projected relic row. `acquisition_index` makes the order contract
/// explicit rather than implied by array position.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceRelic {
    #[serde(default)]
    pub acquisition_index: Option<i64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub props: Option<Map<String, Value>>,
    #[serde(default)]
    pub floor_added_to_deck: Option<i64>,
}

/// Every entry fact a fight root needs that this projection does not record,
/// in the order the builder would reach them.
///
/// A fact appears here only when the projection genuinely cannot supply it;
/// facts it does carry (character, deck, relic inventory and properties,
/// unlock epochs, run stream counters and words) are absent from the list.
pub fn missing_facts(entry: &ProvenanceEntry) -> Vec<&'static str> {
    let mut missing = vec![
        // The global node index, which feeds `total_floor` and therefore the
        // per-fight Encounter stream.
        "visited_map_coords",
        "map_point_history",
        // The node's kind and the act's encounter schedule.
        "acts[].saved_map",
        "acts[].rooms",
        // The run seed every stream derivation and cross-check is keyed on.
        "rng.seed",
    ];
    // The player facts a combat root cannot start without.
    let player = entry
        .players
        .as_deref()
        .and_then(<[ProvenancePlayer]>::first);
    if player.is_some() {
        missing.push("players[].current_hp");
        missing.push("players[].max_hp");
        missing.push("players[].gold");
        missing.push("players[].max_potion_slot_count");
        missing.push("players[].potions");
    }
    missing
}

/// Parse one projection and report what it cannot answer.
///
/// Always a refusal in this slice, and deliberately a *typed* one: I13's
/// "not yet" rather than "never", because a different input — a full save —
/// answers every fact [`missing_facts`] names.
pub fn build(text: &str) -> Result<ProvenanceEntry, EntryRefusal> {
    let entry: ProvenanceEntry = serde_json::from_str(text)
        .map_err(|error| EntryRefusal::UnknownSaveField(error.to_string()))?;
    match entry.save_schema_version {
        Some(ADMITTED_SAVE_SCHEMA) => {}
        Some(other) => return Err(EntryRefusal::UnsupportedProvenanceSchema(other)),
        None => {
            return Err(EntryRefusal::ProvenanceEntryIncomplete {
                fact: "save_schema_version",
            });
        }
    }
    match entry.players.as_deref() {
        Some([_]) => {}
        Some(players) => return Err(EntryRefusal::MultiplayerSave(players.len())),
        None => {
            return Err(EntryRefusal::ProvenanceEntryIncomplete { fact: "players" });
        }
    }
    Ok(entry)
}

/// The refusal a projection earns: the first fact it cannot supply.
pub fn refusal_for(entry: &ProvenanceEntry) -> Option<EntryRefusal> {
    missing_facts(entry)
        .first()
        .map(|fact| EntryRefusal::ProvenanceEntryIncomplete { fact })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTION: &str = r#"{
        "save_schema_version": 20,
        "run_rng": {"shuffle": {"counter": 649, "s0": 1, "s1": 2, "s2": 3, "s3": 4}},
        "shared_relic_grab_bag": {"relic_id_lists": {"common": []}},
        "players": [{
            "net_id": 1,
            "character_id": "CHARACTER.DEFECT",
            "deck": [{"copy_index": 0, "id": "CARD.DEFEND_DEFECT", "upgrade_level": 0,
                      "enchantment": null, "floor_added_to_deck": 1}],
            "relics": [{"acquisition_index": 0, "id": "RELIC.CRACKED_CORE",
                        "floor_added_to_deck": 1}],
            "unlock_state": {"unlocked_epochs": [], "encounters_seen": [],
                             "number_of_runs": 0},
            "relic_grab_bag": {"relic_id_lists": {}}
        }]
    }"#;

    #[test]
    fn the_documented_v2_projection_shape_parses() {
        let entry = build(PROJECTION).unwrap();
        assert_eq!(entry.run_rng["shuffle"].counter, 649);
        let player = &entry.players.as_ref().unwrap()[0];
        assert_eq!(player.character_id.as_deref(), Some("CHARACTER.DEFECT"));
        assert_eq!(player.deck[0].upgrade_level, Some(0));
    }

    #[test]
    fn it_refuses_by_naming_the_facts_it_cannot_supply() {
        let entry = build(PROJECTION).unwrap();
        let missing = missing_facts(&entry);
        assert!(missing.contains(&"players[].current_hp"));
        assert!(missing.contains(&"rng.seed"));
        assert!(missing.contains(&"acts[].rooms"));
        // Facts the projection DOES carry must not be listed.
        assert!(!missing.iter().any(|fact| fact.contains("character_id")));
        assert!(!missing.iter().any(|fact| fact.contains("relics")));
        assert_eq!(
            refusal_for(&entry),
            Some(EntryRefusal::ProvenanceEntryIncomplete {
                fact: "visited_map_coords"
            })
        );
    }

    #[test]
    fn an_untaught_projection_key_refuses_rather_than_being_dropped() {
        let text = PROJECTION.replace(r#""net_id": 1,"#, r#""net_id": 1, "soul_power": 3,"#);
        assert_eq!(build(&text).unwrap_err().class(), "unknown_save_field");
    }

    #[test]
    fn an_older_projection_schema_refuses_as_a_schema() {
        let text = PROJECTION.replace(
            r#""save_schema_version": 20"#,
            r#""save_schema_version": 19"#,
        );
        assert_eq!(
            build(&text),
            Err(EntryRefusal::UnsupportedProvenanceSchema(19))
        );
    }
}
