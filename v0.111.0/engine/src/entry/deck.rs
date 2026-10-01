//! The deck a fight is entered with, one row per physical copy.
//!
//! Oracle: the deck loop of `live_coach.build_entry`, plus
//! `live_coach._card_upgrade_level` and `live_coach._canonical_model_id`.
//!
//! The save's `deck` array is the physical-copy snapshot in save-array order,
//! and that order is load bearing twice over: the opening shuffle permutes
//! *this* list, and the replay deal maps input-log instance `k` to the k-th
//! card of the first shuffle cycle over it.

use serde_json::{Map, Value};

use crate::entry::canonical_model_id;
use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{SerializedCard, SerializedPlayer};

/// The three cards whose saved per-instance properties are carried into the
/// fight verbatim.
///
/// `start_combat` validates the exact names and types and each card's
/// `Current == native base + Increased` relationship, so the bytes are passed
/// through unexamined here rather than re-derived: reshaping them would change
/// what the oracle is handed.
pub const PER_INSTANCE_PROP_CARDS: [&str; 3] = [
    "CARD.THE_SCYTHE",
    "CARD.GENETIC_ALGORITHM",
    "CARD.MAD_SCIENCE",
];

/// One entering deck row, in `build_entry`'s exact key vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckEntry {
    pub id: String,
    pub upgrade_level: i64,
    /// Present only for [`PER_INSTANCE_PROP_CARDS`], and only when the save
    /// row carried a `props` object at all.
    pub props: Option<Map<String, Value>>,
    pub enchantment: Option<String>,
    pub enchant_amount: Option<i64>,
}

impl DeckEntry {
    /// The row as `build_entry` spells it, for digest-comparable output.
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("id".to_string(), Value::String(self.id.clone()));
        row.insert("upgrade_level".to_string(), Value::from(self.upgrade_level));
        if let Some(props) = &self.props {
            row.insert("props".to_string(), Value::Object(props.clone()));
        }
        if let Some(enchantment) = &self.enchantment {
            row.insert(
                "enchantment".to_string(),
                Value::String(enchantment.clone()),
            );
            row.insert(
                "enchant_amount".to_string(),
                self.enchant_amount.map_or(Value::from(1), Value::from),
            );
        }
        Value::Object(row)
    }
}

/// The card's upgrade level.
///
/// Oracle: `live_coach._card_upgrade_level`. A save writes
/// `current_upgrade_level` and omits it at zero; the decoded-MCR form writes
/// `upgrade_level`, which the typed save parser does not accept, so the
/// oracle's "row mixes both keys" refusal is structurally unreachable here.
/// A non-integer level is rejected by the parser as an untaught value shape,
/// where the oracle raises.
fn upgrade_level(card: &SerializedCard) -> i64 {
    card.current_upgrade_level.unwrap_or(0)
}

/// Every entering deck row, in save-array order.
pub fn deck_entering(player: &SerializedPlayer) -> Result<Vec<DeckEntry>, EntryRefusal> {
    let mut rows = Vec::with_capacity(player.deck.len());
    for card in &player.deck {
        let id = canonical_model_id(card.id.as_deref(), "CARD")?;
        let props = if PER_INSTANCE_PROP_CARDS.contains(&id.as_str()) {
            card.props.clone()
        } else {
            None
        };
        let (enchantment, enchant_amount) = match &card.enchantment {
            Some(enchantment) => (
                Some(canonical_model_id(
                    Some(enchantment.id.as_str()),
                    "ENCHANTMENT",
                )?),
                // Current-build saves store the strength as `amount`; an
                // amountless enchantment defaults to 1 (89SJD17KYUEH: SHARP 3
                // read as 1 shorted every Gunk Up hit by 2 pre-multiplier).
                Some(enchantment.amount.unwrap_or(1)),
            ),
            None => (None, None),
        };
        rows.push(DeckEntry {
            id,
            upgrade_level: upgrade_level(card),
            props,
            enchantment,
            enchant_amount,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::save::SerializedRun;

    fn player(deck: &str) -> SerializedPlayer {
        let text = format!(
            r#"{{"schema_version": 20, "rng": {{"seed": "S", "rngs": {{}}}},
                 "players": [{{"deck": {deck}}}]}}"#
        );
        SerializedRun::parse(&text)
            .unwrap()
            .single_player()
            .unwrap()
            .clone()
    }

    #[test]
    fn a_bare_row_carries_id_and_zero_upgrade() {
        let rows = deck_entering(&player(r#"[{"id": "CARD.STRIKE_IRONCLAD"}]"#)).unwrap();
        assert_eq!(rows[0].upgrade_level, 0);
        assert_eq!(rows[0].to_json()["upgrade_level"], Value::from(0));
        assert!(rows[0].to_json().get("props").is_none());
    }

    #[test]
    fn an_unprefixed_id_gains_its_category() {
        let rows = deck_entering(&player(r#"[{"id": "STRIKE_IRONCLAD"}]"#)).unwrap();
        assert_eq!(rows[0].id, "CARD.STRIKE_IRONCLAD");
    }

    #[test]
    fn a_wrong_category_prefix_refuses() {
        let refusal = deck_entering(&player(r#"[{"id": "RELIC.VAJRA"}]"#)).unwrap_err();
        assert_eq!(refusal.class(), "malformed_model_id");
    }

    #[test]
    fn only_the_three_per_instance_cards_carry_props_through() {
        let scythe = deck_entering(&player(
            r#"[{"id": "CARD.THE_SCYTHE",
                 "props": {"ints": [{"name": "CurrentDamage", "value": 9}]}}]"#,
        ))
        .unwrap();
        assert!(scythe[0].props.is_some());
        let other = deck_entering(&player(
            r#"[{"id": "CARD.SPOILS",
                 "props": {"ints": [{"name": "SpoilsActIndex", "value": 1}]}}]"#,
        ))
        .unwrap();
        assert!(other[0].props.is_none());
    }

    #[test]
    fn enchantment_strength_comes_from_amount_and_defaults_to_one() {
        let sharp = deck_entering(&player(
            r#"[{"id": "CARD.BASH",
                 "enchantment": {"id": "ENCHANTMENT.SHARP", "amount": 3}}]"#,
        ))
        .unwrap();
        assert_eq!(sharp[0].enchant_amount, Some(3));
        assert_eq!(sharp[0].to_json()["enchant_amount"], Value::from(3));
        let bare = deck_entering(&player(
            r#"[{"id": "CARD.BASH", "enchantment": {"id": "ENCHANTMENT.SHARP"}}]"#,
        ))
        .unwrap();
        assert_eq!(bare[0].enchant_amount, Some(1));
    }
}
