//! `SavedProperties`: the per-instance property bag a save writes beside a
//! card, a relic, an enchantment or a run modifier.
//!
//! # Native authority
//!
//! `MegaCrit.Sts2.Core.Saves.Runs.SavedProperties` (v0.111.0 `sts2.dll`,
//! sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
//! declares exactly **seven** fields, read off the TypeDef/FieldList walk:
//! `ints`, `bools`, `strings`, `intArrays`, `modelIds`, `cards`, `cardArrays`.
//! Each is a list of `{name, value}` rows, written through
//! `WritePropertyName` / `ReadPropertyName` and populated from
//! `SavedPropertyAttribute` / `SavedProperty\`1`. The JSON spellings are the
//! serializer's snake_case forms, which is why `intArrays` reaches the file as
//! `int_arrays` and `modelIds` as `model_ids`.
//!
//! Five of the seven appear in the local corpus (`ints`, `bools`, `strings`,
//! `cards`, `model_ids`); `int_arrays` and `card_arrays` are declared by the
//! IL but unobserved — Fur Coat's coordinate latch is an `int_arrays` pair, so
//! "unobserved" is not "unreachable". All seven are accepted here and an
//! eighth group is [`EntryRefusal::SavedPropertyGroupUnhandled`], never
//! skipped as empty (invariant walk, I5 commitment 3).
//!
//! # What this module validates, and what it deliberately does not
//!
//! It validates only what the Python oracle would *raise* on: the group name,
//! and that every row is exactly `{name, value}`. It leaves `value` untyped.
//! The oracle's strict adapters (`_strict_scoped_relic_property`,
//! `_strict_joss_paper_properties`, `_strict_fur_coat_membership`) check value
//! types themselves and answer "unprovable" rather than raising, so type
//! checking here would invent refusals Python does not have and break entry
//! parity for the fights they cover. Types are therefore checked at the read
//! sites in `entry/relics.rs`.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::entry::refusal::EntryRefusal;

/// The seven `SavedProperties` groups, in the IL's declaration order, spelled
/// as the save serializer writes them.
pub const SAVED_PROPERTY_GROUPS: [&str; 7] = [
    "ints",
    "bools",
    "strings",
    "int_arrays",
    "model_ids",
    "cards",
    "card_arrays",
];

/// One `{name, value}` row of one group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyRow {
    pub name: String,
    pub value: Value,
}

/// A validated property bag: group name → rows, in file order within a group.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SavedProperties {
    groups: BTreeMap<String, Vec<PropertyRow>>,
}

impl SavedProperties {
    /// Validate one raw `props` object.
    ///
    /// `owner` names the row the bag hangs off (`"RELIC.JOSS_PAPER"`,
    /// `"CARD.THE_SCYTHE"`) and appears in every refusal so a census row points
    /// at the instance, not just the shape.
    pub fn parse(owner: &str, raw: &Map<String, Value>) -> Result<Self, EntryRefusal> {
        let mut groups = BTreeMap::new();
        for (group, value) in raw {
            if !SAVED_PROPERTY_GROUPS.contains(&group.as_str()) {
                return Err(EntryRefusal::SavedPropertyGroupUnhandled {
                    owner: owner.to_string(),
                    group: group.clone(),
                });
            }
            let rows = value
                .as_array()
                .ok_or_else(|| EntryRefusal::SavedPropertyRowShape {
                    owner: owner.to_string(),
                    group: group.clone(),
                    detail: "group is not a list".to_string(),
                })?;
            let mut parsed = Vec::with_capacity(rows.len());
            for row in rows {
                let object =
                    row.as_object()
                        .ok_or_else(|| EntryRefusal::SavedPropertyRowShape {
                            owner: owner.to_string(),
                            group: group.clone(),
                            detail: "row is not an object".to_string(),
                        })?;
                if object.len() != 2
                    || !object.contains_key("name")
                    || !object.contains_key("value")
                {
                    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
                    keys.sort_unstable();
                    return Err(EntryRefusal::SavedPropertyRowShape {
                        owner: owner.to_string(),
                        group: group.clone(),
                        detail: format!("row keys are {keys:?}, not exactly [name, value]"),
                    });
                }
                let name =
                    object["name"]
                        .as_str()
                        .ok_or_else(|| EntryRefusal::SavedPropertyRowShape {
                            owner: owner.to_string(),
                            group: group.clone(),
                            detail: "row name is not a string".to_string(),
                        })?;
                parsed.push(PropertyRow {
                    name: name.to_string(),
                    value: object["value"].clone(),
                });
            }
            groups.insert(group.clone(), parsed);
        }
        Ok(Self { groups })
    }

    /// True when no group is present at all.
    ///
    /// The oracle's adapters all begin `if not isinstance(props, dict) or not
    /// props`, so an empty bag is "unprovable" everywhere, never "all
    /// defaults".
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The group names present, sorted.
    pub fn group_names(&self) -> Vec<&str> {
        self.groups.keys().map(String::as_str).collect()
    }

    /// The rows of one group, or an empty slice when the group is absent.
    pub fn group(&self, name: &str) -> &[PropertyRow] {
        self.groups.get(name).map_or(&[][..], Vec::as_slice)
    }

    /// Every row across every group, group name attached, in group order.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &PropertyRow)> {
        self.groups
            .iter()
            .flat_map(|(group, rows)| rows.iter().map(move |row| (group.as_str(), row)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> Map<String, Value> {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_seven_il_declared_groups_all_parse() {
        // int_arrays and card_arrays are unobserved in the corpus; the IL
        // declares them and Fur Coat's latch is an int_arrays pair, so an
        // unhandled-group refusal on either would be a real admission hole.
        let bag = raw(r#"{"ints": [{"name": "A", "value": 1}],
                "bools": [{"name": "B", "value": false}],
                "strings": [{"name": "C", "value": "x"}],
                "int_arrays": [{"name": "D", "value": [1, 2]}],
                "model_ids": [{"name": "E", "value": "CARD.BASH"}],
                "cards": [{"name": "F", "value": {"id": "CARD.BASH"}}],
                "card_arrays": [{"name": "G", "value": []}]}"#);
        let props = SavedProperties::parse("RELIC.TEST", &bag).unwrap();
        assert_eq!(props.group_names().len(), 7);
        assert_eq!(props.rows().count(), 7);
    }

    #[test]
    fn an_eighth_group_refuses_by_name() {
        let bag = raw(r#"{"floats": [{"name": "A", "value": 1.0}]}"#);
        assert_eq!(
            SavedProperties::parse("RELIC.TEST", &bag),
            Err(EntryRefusal::SavedPropertyGroupUnhandled {
                owner: "RELIC.TEST".to_string(),
                group: "floats".to_string(),
            })
        );
    }

    #[test]
    fn a_row_with_extra_keys_refuses() {
        let bag = raw(r#"{"ints": [{"name": "A", "value": 1, "extra": 2}]}"#);
        let refusal = SavedProperties::parse("RELIC.TEST", &bag).unwrap_err();
        assert_eq!(refusal.class(), "saved_property_row_shape");
        assert!(refusal.to_string().contains("not exactly"));
    }

    #[test]
    fn value_types_are_not_checked_here() {
        // Deliberate: the oracle's strict adapters answer "unprovable" on a
        // wrong-typed value instead of raising, so refusing here would break
        // entry parity rather than protect it.
        let bag = raw(r#"{"ints": [{"name": "A", "value": "not an int"}]}"#);
        assert!(SavedProperties::parse("RELIC.TEST", &bag).is_ok());
    }

    #[test]
    fn an_empty_bag_is_empty_not_defaulted() {
        let props = SavedProperties::parse("RELIC.TEST", &raw("{}")).unwrap();
        assert!(props.is_empty());
        assert!(props.group("ints").is_empty());
    }
}
