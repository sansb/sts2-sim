//! The potion belt entering the fight.
//!
//! Oracle: the potion block of `live_coach.build_entry`.
//!
//! This is the class the save closes outright. Python's `.run` adapters had to
//! *infer* which belt slot a potion occupied and refused when a full-belt
//! replacement made the inventory ambiguous; a schema-20 save records
//! `max_potion_slot_count` on the player (3,092 of 3,092 corpus saves) and
//! `slot_index` on every potion row (3,642 of 3,642), so the belt is read, not
//! reconstructed.
//!
//! Native authority: `Saves.Runs.SerializablePotion` (`Id`, `SlotIndex`) and
//! `SerializablePlayer.MaxPotionSlotCount`, v0.111.0 `sts2.dll` sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.

use serde_json::{Map, Value};

use crate::entry::canonical_model_id;
use crate::entry::refusal::EntryRefusal;
use crate::entry::save::SerializedPlayer;

/// One occupied belt slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PotionSlot {
    pub id: String,
    /// `None` only if the save omitted `slot_index`, which no corpus save
    /// does. It is carried through rather than defaulted, because a guessed
    /// slot is exactly the ambiguity this field exists to remove.
    pub slot_index: Option<i64>,
}

impl PotionSlot {
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("id".to_string(), Value::String(self.id.clone()));
        row.insert(
            "slot_index".to_string(),
            self.slot_index.map_or(Value::Null, Value::from),
        );
        Value::Object(row)
    }
}

/// The belt: occupied slots in save order, plus the recorded belt size.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Belt {
    pub slots: Vec<PotionSlot>,
    /// `MaxPotionSlotCount`. `None` when the save omits it; the oracle's
    /// `start_combat` then materializes no empty belt rather than guessing a
    /// size.
    pub max_potion_slot_count: Option<i64>,
}

impl Belt {
    /// Just the potion ids, in save order — `potions_entering`.
    pub fn potions_entering(&self) -> Vec<String> {
        self.slots.iter().map(|slot| slot.id.clone()).collect()
    }
}

/// Read the belt off the save's player.
pub fn belt(player: &SerializedPlayer) -> Result<Belt, EntryRefusal> {
    let mut slots = Vec::with_capacity(player.potions.len());
    for potion in &player.potions {
        slots.push(PotionSlot {
            id: canonical_model_id(potion.id.as_deref(), "POTION")?,
            slot_index: potion.slot_index,
        });
    }
    Ok(Belt {
        slots,
        max_potion_slot_count: player.max_potion_slot_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::save::SerializedRun;

    fn player(body: &str) -> SerializedPlayer {
        let text = format!(
            r#"{{"schema_version": 20, "rng": {{"seed": "S", "rngs": {{}}}},
                 "players": [{body}]}}"#
        );
        SerializedRun::parse(&text)
            .unwrap()
            .single_player()
            .unwrap()
            .clone()
    }

    #[test]
    fn slots_and_size_come_straight_off_the_save() {
        let belt = belt(&player(
            r#"{"max_potion_slot_count": 3,
                "potions": [{"id": "POTION.FYSH_OIL", "slot_index": 0},
                            {"id": "POTION.SPEED_POTION", "slot_index": 2}]}"#,
        ))
        .unwrap();
        assert_eq!(belt.max_potion_slot_count, Some(3));
        assert_eq!(
            belt.potions_entering(),
            ["POTION.FYSH_OIL", "POTION.SPEED_POTION"]
        );
        assert_eq!(belt.slots[1].slot_index, Some(2));
    }

    #[test]
    fn an_empty_belt_is_empty_and_still_records_its_size() {
        let belt = belt(&player(r#"{"max_potion_slot_count": 2}"#)).unwrap();
        assert!(belt.slots.is_empty());
        assert_eq!(belt.max_potion_slot_count, Some(2));
    }

    #[test]
    fn an_absent_belt_size_stays_absent_rather_than_defaulting_to_zero() {
        let belt = belt(&player(r#"{"potions": []}"#)).unwrap();
        assert_eq!(belt.max_potion_slot_count, None);
    }

    #[test]
    fn a_wrong_category_potion_id_refuses() {
        let refusal = belt(&player(r#"{"potions": [{"id": "CARD.BASH"}]}"#)).unwrap_err();
        assert_eq!(refusal.class(), "malformed_model_id");
    }
}
