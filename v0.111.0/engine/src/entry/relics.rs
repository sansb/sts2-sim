//! Relic entry state: the inventory, its dispatch-order vouch, and the saved
//! properties that persist across combats.
//!
//! Oracle: the relic loop of `live_coach.build_entry`, plus the three strict
//! adapters `live_coach._strict_scoped_relic_property`,
//! `live_coach._strict_joss_paper_properties` and
//! `live_coach._strict_fur_coat_membership`.
//!
//! # Fail-closed, not fail-quiet
//!
//! Every adapter here answers "proven" or "unprovable" over the **complete**
//! native property shape, never over a subset. A partial or mixed payload can
//! therefore never silently seed combat behaviour: it reaches `start_combat`
//! as an absent counter, and `start_combat`'s own exactness gates refuse the
//! fight. That division is deliberate and is why these adapters return an
//! option rather than an [`EntryRefusal`]: the *entry* is buildable; whether
//! the fight is runnable is the next stage's question.
//!
//! # Native authority
//!
//! `Saves.Runs.SerializableRelic` (`Id`, `Props`, `FloorAddedToDeck`) and
//! `Saves.Runs.SavedProperties` in v0.111.0 `sts2.dll` sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//! `players[N].relics` is the serialized `Player._relics` list itself
//! (`ToSerializable` 0x11a804 order-preserving projection, `PopulateRelics`
//! 0x11b3c0 appends on load), which is exactly the same-hook relic dispatch
//! order (`IterateHookListeners` d__69 0x3f6700 enumerates it by ascending
//! index).

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::entry::canonical_model_id;
use crate::entry::props::SavedProperties;
use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{SerializedPlayer, SerializedRun};

/// Which saved property each finite / Tea relic owns, and its group and type.
///
/// Group `"ints"` routes the value into `relic_counters`; the two Tea Set
/// booleans route into their own entry fields; Lizard Tail is a boolean that
/// nevertheless routes into `relic_counters`, because `start_combat` reads its
/// `WasUsed` there and requires an exact `bool`.
///
/// Maw Bank's `HasItemBeenBought` (#3328) has Lizard Tail's shape: a saved
/// `bool` the opening reads as an exact value from `relic_counters`
/// (`entry::opening`'s `maw_bank_room_entry_armed`). `MawBank` declares one
/// instance field, `_hasItemBeenBought`, behind that property
/// (`get_HasItemBeenBought` RVA `0x96947`). Its only writer of `true` is
/// `AfterItemPurchased` (`0x969bb`): `set_HasItemBeenBought(true)` at
/// `IL_002f`-`IL_0030`, after the owner test (`IL_0001`-`IL_0008`) and the
/// `goldSpent > 0` test (`IL_001e`-`IL_0020`). Nothing resets it.
const SCOPED_RELIC_SAVED_PROPERTIES: [(&str, &str, &str); 8] = [
    ("RELIC.BONE_TEA", "ints", "CombatsLeft"),
    ("RELIC.EMBER_TEA", "ints", "CombatsLeft"),
    ("RELIC.PUMPKIN_CANDLE", "ints", "KindleCount"),
    ("RELIC.TEA_OF_DISCOURTESY", "ints", "CombatsLeft"),
    ("RELIC.VENERABLE_TEA_SET", "bools", "GainEnergyInNextCombat"),
    (
        "RELIC.FAKE_VENERABLE_TEA_SET",
        "bools",
        "GainEnergyInNextCombat",
    ),
    ("RELIC.LIZARD_TAIL", "bools", "WasUsed"),
    ("RELIC.MAW_BANK", "bools", "HasItemBeenBought"),
];

/// The scoped `bools` relics whose saved value lives in `relic_counters`
/// rather than in an entry field of its own.
const COUNTER_BOOL_RELICS: [&str; 2] = ["RELIC.LIZARD_TAIL", "RELIC.MAW_BANK"];

/// Whether `relic` stores a saved `bool` in `relic_counters`. The facts
/// reader's shape check shares this list with the save path.
pub(crate) fn is_counter_bool_relic(relic: &str) -> bool {
    COUNTER_BOOL_RELICS.contains(&relic)
}

/// Base-class flags every relic inherits at `false`. They may be present at
/// `false` or omitted, and are never synthesized. A melted wax relic
/// (`IsMelted: true`) never reaches the per-relic adapters: `is_melted` drops
/// it from the inventory first.
const INHERITED_FALSE_RELIC_PROPERTIES: [&str; 2] = ["IsWax", "IsMelted"];

const JOSS_PAPER_REQUIRED: [&str; 2] = ["CardsExhausted", "EtherealCount"];

/// The relic half of the entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelicEntry {
    /// Inventory order, which on a schema >= 19 save is acquisition order.
    pub relics_entering: Vec<String>,
    /// Relic id → persistent counter, as `start_combat` reads it.
    pub relic_counters: BTreeMap<String, Value>,
    pub tea_set_charged: Option<bool>,
    pub fake_tea_set_charged: Option<bool>,
    pub fur_coat_active: Option<bool>,
    /// Whether `relics_entering` may be vouched for as same-hook dispatch
    /// order.
    pub dispatch_ordered: bool,
}

/// Build the relic half of the entry from the save's player.
pub fn relic_entry(
    run: &SerializedRun,
    player: &SerializedPlayer,
) -> Result<RelicEntry, EntryRefusal> {
    let mut entry = RelicEntry {
        // The oracle vouches for inventory order as dispatch order exactly
        // when the save carries per-stream RNG state, i.e. schema >= 19 —
        // spelled there as `"rngs" in (save.get("rng") or {})`. That is **key
        // presence**, not a non-empty map, which is why `SerializedRunRngSet`
        // keeps `rngs` optional. It was verified end-to-end against a live run
        // (#808, seed TQM88QFMHSQR).
        dispatch_ordered: run.rng.as_ref().is_some_and(|rng| rng.rngs.is_some()),
        ..RelicEntry::default()
    };
    let mut melted_whispering_earring = false;
    for relic in &player.relics {
        let id = canonical_model_id(relic.id.as_deref(), "RELIC")?;
        let props = match &relic.props {
            Some(raw) => SavedProperties::parse(&id, raw)?,
            None => SavedProperties::default(),
        };
        if is_melted(&id, &props)? {
            match MELTED_RELIC_READERS
                .iter()
                .find(|(relic_id, _)| *relic_id == id)
            {
                Some((_, reader)) => {
                    return Err(EntryRefusal::MeltedRelicStillRead { relic: id, reader });
                }
                None if id == "RELIC.WHISPERING_EARRING" => melted_whispering_earring = true,
                None => {}
            }
            continue;
        }
        entry.relics_entering.push(id.clone());
        if id == "RELIC.FUR_COAT" {
            entry.fur_coat_active = fur_coat_membership(run, &props);
            continue;
        }
        if id == "RELIC.JOSS_PAPER" {
            if let Some(values) = joss_paper_properties(&props) {
                entry.relic_counters.insert(id, Value::Object(values));
            }
            continue;
        }
        if let Some((_, group, name)) = SCOPED_RELIC_SAVED_PROPERTIES
            .iter()
            .find(|(relic_id, _, _)| *relic_id == id)
        {
            if let Some(value) = scoped_relic_property(&props, group, name) {
                if *group == "ints" || is_counter_bool_relic(&id) {
                    entry.relic_counters.insert(id, value);
                } else if id == "RELIC.VENERABLE_TEA_SET" {
                    entry.tea_set_charged = value.as_bool();
                } else {
                    entry.fake_tea_set_charged = value.as_bool();
                }
            }
            continue;
        }
        // The established grouped adapter for every unrelated relic counter.
        // Groups outside `ints` stay outside combat semantics here rather than
        // gaining new ones.
        let ints = props.group("ints");
        match ints.len() {
            0 => {}
            1 => {
                entry.relic_counters.insert(id, ints[0].value.clone());
            }
            _ => {
                let mut bag = Map::new();
                for row in ints {
                    bag.insert(row.name.clone(), row.value.clone());
                }
                entry.relic_counters.insert(id, Value::Object(bag));
            }
        }
    }
    if melted_whispering_earring
        && entry
            .relics_entering
            .iter()
            .any(|id| id == "RELIC.PAELS_EYE")
    {
        return Err(EntryRefusal::MeltedRelicStillRead {
            relic: "RELIC.WHISPERING_EARRING".to_string(),
            reader: "PaelsEye.AnyCardsPlayedThisTurn (RVA 0x98208 IL_002e) Relics.Any(is WhisperingEarring)",
        });
    }
    Ok(entry)
}

/// Melted relics native still reads outside the hook walk, and the reader.
///
/// Both are `Player.GetRelic<T>` calls (`Player.GetRelic` filters only by
/// type, `<GetRelic>b__146_0` RVA 0x3d91bf is a bare `isinst`), so a melted
/// copy still answers them. Whispering Earring's reader is conditional on an
/// unmelted Pael's Eye and is handled after the inventory walk.
const MELTED_RELIC_READERS: [(&str, &str); 2] = [
    (
        "RELIC.PAPER_PHROG",
        "VulnerablePower.ModifyDamageMultiplicative (RVA 0xaae4c IL_0051) GetRelic<PaperPhrog>",
    ),
    (
        "RELIC.PAPER_KRANE",
        "WeakPower.ModifyDamageMultiplicative (RVA 0xaafb8 IL_0053) GetRelic<PaperKrane>",
    ),
];

/// Whether a saved relic is a melted Toy Box wax relic, which native keeps in
/// `Player._relics` but never dispatches a hook to.
///
/// v0.111.0 IL:
/// - `ToyBox.AfterCombatEnd` (RVA 0x9cfb8) -> `<AfterCombatEnd>d__25.MoveNext`
///   (RVA 0x332f80) picks an unmelted wax relic (`<>c.<AfterCombatEnd>b__25_0`
///   RVA 0x332f66: `IsWax && !IsMelted`) and calls `RelicCmd.Melt` at IL_00a8,
///   whose `<Melt>d__4` (RVA 0x3f0bf4 IL_0027) calls
///   `Player.MeltRelicInternal`. That is the only melt path, and it runs after
///   combat ends, so `IsMelted` is constant for a whole fight.
/// - `Player.MeltRelicInternal` (RVA 0x11737c) throws unless `IsWax`
///   (IL_000d) and unless not yet melted (IL_0045), then sets `IsMelted`
///   (IL_00d6) **without** removing the relic from `_relics`.
/// - The combat hook walk `CombatState.<IterateHookListeners>d__69.MoveNext`
///   (RVA 0x3f9720) skips a relic whose `IsMelted` is set (IL_00de brtrue),
///   and the run walk `RunState.<IterateHookListeners>d__118.MoveNext`
///   (RVA 0x30e588) filters `Relics` through `<>c.<IterateHookListeners>b__118_0`
///   (RVA 0x30e474: `!IsMelted`) at IL_011f. A melted relic therefore owns no
///   hook, which is what dropping it from `relics_entering` models.
/// - The remaining `Player.Relics` / `GetRelic<T>` readers reachable in combat
///   are `MELTED_RELIC_READERS` and Pael's Eye's Whispering Earring check;
///   those refuse by name rather than drop.
///
/// A relic with no `IsMelted: true` row is not melted, whatever else its
/// flags say (the per-relic adapters own those shapes, as before). Once one
/// is present, the melted state must be exactly the native one — a single
/// boolean `IsMelted: true` and a single boolean `IsWax: true`, both in
/// `bools` — or the entry refuses: `IsMelted` without `IsWax` is unreachable
/// (`MeltRelicInternal` IL_000d throws).
fn is_melted(id: &str, props: &SavedProperties) -> Result<bool, EntryRefusal> {
    let flagged = props
        .rows()
        .any(|(_, row)| row.name == "IsMelted" && row.value == Value::Bool(true));
    if !flagged {
        return Ok(false);
    }
    let exactly_true = |name: &str| {
        let rows: Vec<_> = props.rows().filter(|(_, row)| row.name == name).collect();
        matches!(rows.as_slice(), [(group, row)] if *group == "bools" && row.value == Value::Bool(true))
    };
    if !exactly_true("IsMelted") {
        return Err(EntryRefusal::MeltedRelicFlags {
            relic: id.to_string(),
            detail: "IsMelted is repeated or outside the bools group",
        });
    }
    if !exactly_true("IsWax") {
        return Err(EntryRefusal::MeltedRelicFlags {
            relic: id.to_string(),
            detail: "IsMelted without exactly one IsWax: true",
        });
    }
    Ok(true)
}

/// `_strict_scoped_relic_property`, grouped form.
///
/// Only the relic's own required property and the two inherited `false` flags
/// are legal; the required property must sit in its declared group with its
/// declared type; no name may repeat; no group may be an empty list.
fn scoped_relic_property(props: &SavedProperties, group: &str, required: &str) -> Option<Value> {
    if props.is_empty() {
        return None;
    }
    for present in props.group_names() {
        if present != group && present != "bools" {
            return None;
        }
    }
    let mut values: BTreeMap<String, Value> = BTreeMap::new();
    for name in props.group_names() {
        let rows = props.group(name);
        if rows.is_empty() {
            return None;
        }
        for row in rows {
            if values.contains_key(&row.name) {
                return None;
            }
            if row.name == required {
                if name != group || !matches_group_type(group, &row.value) {
                    return None;
                }
            } else if INHERITED_FALSE_RELIC_PROPERTIES.contains(&row.name.as_str()) {
                if name != "bools" || row.value != Value::Bool(false) {
                    return None;
                }
            } else {
                return None;
            }
            values.insert(row.name.clone(), row.value.clone());
        }
    }
    values.get(required).cloned()
}

/// Whether a value has the exact JSON type its group declares.
///
/// `ints` rejects a boolean and `bools` rejects an integer, mirroring the
/// oracle's `type(value) is not required_type` (which, unlike `isinstance`,
/// does not accept `bool` as an `int`).
fn matches_group_type(group: &str, value: &Value) -> bool {
    match group {
        "ints" => value.is_i64() || value.is_u64(),
        "bools" => value.is_boolean(),
        _ => false,
    }
}

/// `_strict_joss_paper_properties`: room-entry values with native defaults.
/// v0.111.0 JossPaper.CardsExhausted is SaveIfNotTypeDefault: its zero
/// is omitted by SavedProperties::FromInternal. JossPaper::AfterCombatEnd
/// (RVA 0x95573) clears EtherealCount. Preserve explicit values for the
/// opening's range/phase validation; default only absent properties.
fn joss_paper_properties(props: &SavedProperties) -> Option<Map<String, Value>> {
    for present in props.group_names() {
        if present != "ints" && present != "bools" {
            return None;
        }
    }
    let ints = props.group("ints");
    let mut values = Map::new();
    for row in ints {
        if values.contains_key(&row.name)
            || !JOSS_PAPER_REQUIRED.contains(&row.name.as_str())
            || !matches_group_type("ints", &row.value)
        {
            return None;
        }
        values.insert(row.name.clone(), row.value.clone());
    }
    let mut flags = std::collections::BTreeSet::new();
    for row in props.group("bools") {
        if !INHERITED_FALSE_RELIC_PROPERTIES.contains(&row.name.as_str())
            || row.value != Value::Bool(false)
            || !flags.insert(&row.name)
        {
            return None;
        }
    }
    for name in JOSS_PAPER_REQUIRED {
        values.entry(name.to_string()).or_insert(Value::from(0));
    }
    Some(values)
}

/// `_strict_fur_coat_membership`, as it behaves on a JSON run save.
///
/// **Measured, not assumed:** the oracle's first gate is
/// `isinstance(current_coord, list) and len(current_coord) == 2`, and it is
/// handed `save["visited_map_coords"][-1]`. In a JSON run save every entry of
/// that array is a `{"col", "row"}` **object** — 34,174 of 34,174 coordinates
/// across the 3,092-save corpus — so the gate never opens and Fur Coat
/// membership is *always* unprovable from a save. The list-shaped coordinate
/// is the decoded-MCR form, which is a different adapter's input.
///
/// This port reproduces that rather than repairing it: entry parity with the
/// oracle is the acceptance contract, and a Rust builder that answered `true`
/// here would change the entry facts of every Fur Coat fight. The repair is
/// filed as #2526, and must move both engines in the same change.
fn fur_coat_membership(run: &SerializedRun, _props: &SavedProperties) -> Option<bool> {
    // The coordinate the oracle would receive. It is an object in every save,
    // so the shape gate below is the whole function; the props validation is
    // never reached in the oracle either, because it short-circuits first.
    let _current = run.visited_map_coords.last()?;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(relics: &str) -> SerializedRun {
        let text = format!(
            r#"{{"schema_version": 20,
                 "visited_map_coords": [{{"col": 1, "row": 2}}],
                 "rng": {{"seed": "S", "rngs": {{"shuffle": {{"counter": 0, "s0": 1, "s1": 2, "s2": 3, "s3": 4}}}}}},
                 "players": [{{"relics": {relics}}}]}}"#
        );
        SerializedRun::parse(&text).unwrap()
    }

    fn entry_of(relics: &str) -> RelicEntry {
        let run = run_with(relics);
        let player = run.single_player().unwrap().clone();
        relic_entry(&run, &player).unwrap()
    }

    #[test]
    fn inventory_order_is_preserved_and_vouched_on_a_schema_20_save() {
        let entry = entry_of(r#"[{"id": "RELIC.VAJRA"}, {"id": "RELIC.AKABEKO"}]"#);
        assert_eq!(entry.relics_entering, ["RELIC.VAJRA", "RELIC.AKABEKO"]);
        assert!(entry.dispatch_ordered);
    }

    #[test]
    fn an_empty_rngs_map_still_vouches_because_the_oracle_checks_the_key() {
        // `"rngs" in (save.get("rng") or {})` is true for `"rngs": {}`. The
        // fight is refused a step later for its missing streams, but the
        // dispatch-order fact must not disagree with the oracle in between.
        let run = SerializedRun::parse(
            r#"{"schema_version": 20,
                "visited_map_coords": [{"col": 1, "row": 2}],
                "rng": {"seed": "S", "rngs": {}},
                "players": [{"relics": [{"id": "RELIC.VAJRA"}]}]}"#,
        )
        .unwrap();
        let player = run.single_player().unwrap().clone();
        assert!(relic_entry(&run, &player).unwrap().dispatch_ordered);
    }

    #[test]
    fn a_single_int_property_becomes_a_bare_counter() {
        let entry = entry_of(
            r#"[{"id": "RELIC.BOOK_OF_FIVE_RINGS",
                 "props": {"ints": [{"name": "CardsAdded", "value": 30}]}}]"#,
        );
        assert_eq!(
            entry.relic_counters["RELIC.BOOK_OF_FIVE_RINGS"],
            Value::from(30)
        );
    }

    #[test]
    fn several_int_properties_become_a_named_bag() {
        let entry = entry_of(
            r#"[{"id": "RELIC.SOME_RELIC",
                 "props": {"ints": [{"name": "A", "value": 1}, {"name": "B", "value": 2}]}}]"#,
        );
        let bag = entry.relic_counters["RELIC.SOME_RELIC"]
            .as_object()
            .unwrap();
        assert_eq!(bag["A"], Value::from(1));
        assert_eq!(bag["B"], Value::from(2));
    }

    #[test]
    fn a_relic_with_only_non_int_groups_gets_no_counter() {
        let entry = entry_of(
            r#"[{"id": "RELIC.PANTOGRAPH", "props": {"strings": [{"name": "Skin", "value": "x"}]}}]"#,
        );
        assert!(entry.relic_counters.is_empty());
    }

    #[test]
    fn a_scoped_int_relic_routes_into_the_counters() {
        let entry = entry_of(
            r#"[{"id": "RELIC.EMBER_TEA",
                 "props": {"ints": [{"name": "CombatsLeft", "value": 3}],
                           "bools": [{"name": "IsWax", "value": false}]}}]"#,
        );
        assert_eq!(entry.relic_counters["RELIC.EMBER_TEA"], Value::from(3));
    }

    #[test]
    fn a_scoped_relic_with_a_stranger_property_is_unprovable() {
        let entry = entry_of(
            r#"[{"id": "RELIC.EMBER_TEA",
                 "props": {"ints": [{"name": "CombatsLeft", "value": 3},
                                    {"name": "Stranger", "value": 1}]}}]"#,
        );
        assert!(entry.relic_counters.is_empty());
    }

    #[test]
    fn a_scoped_relic_with_the_wrong_value_type_is_unprovable() {
        let entry = entry_of(
            r#"[{"id": "RELIC.EMBER_TEA",
                 "props": {"ints": [{"name": "CombatsLeft", "value": true}]}}]"#,
        );
        assert!(entry.relic_counters.is_empty());
    }

    #[test]
    fn the_two_tea_sets_route_to_their_own_fields() {
        let real = entry_of(
            r#"[{"id": "RELIC.VENERABLE_TEA_SET",
                 "props": {"bools": [{"name": "GainEnergyInNextCombat", "value": true}]}}]"#,
        );
        assert_eq!(real.tea_set_charged, Some(true));
        assert_eq!(real.fake_tea_set_charged, None);
        let fake = entry_of(
            r#"[{"id": "RELIC.FAKE_VENERABLE_TEA_SET",
                 "props": {"bools": [{"name": "GainEnergyInNextCombat", "value": false}]}}]"#,
        );
        assert_eq!(fake.fake_tea_set_charged, Some(false));
        assert_eq!(fake.tea_set_charged, None);
    }

    #[test]
    fn lizard_tail_is_a_bool_that_lives_in_the_counters() {
        let entry = entry_of(
            r#"[{"id": "RELIC.LIZARD_TAIL",
                 "props": {"bools": [{"name": "WasUsed", "value": false}]}}]"#,
        );
        assert_eq!(
            entry.relic_counters["RELIC.LIZARD_TAIL"],
            Value::Bool(false)
        );
    }

    /// Maw Bank's saved `HasItemBeenBought` reaches the counters as an exact
    /// bool, for both values (#3328). A stranger property or a non-bool value
    /// leaves it unprovable, which the opening refuses as
    /// `RelicCounterNotExact`.
    #[test]
    fn maw_bank_is_a_bool_that_lives_in_the_counters() {
        for (raw, bought) in [
            (
                r#"[{"id": "RELIC.MAW_BANK",
                     "props": {"bools": [{"name": "HasItemBeenBought", "value": false}]}}]"#,
                false,
            ),
            (
                r#"[{"id": "RELIC.MAW_BANK",
                     "props": {"bools": [{"name": "HasItemBeenBought", "value": true}]}}]"#,
                true,
            ),
        ] {
            assert_eq!(
                entry_of(raw).relic_counters["RELIC.MAW_BANK"],
                Value::Bool(bought)
            );
        }
        for unprovable in [
            r#"[{"id": "RELIC.MAW_BANK",
                 "props": {"bools": [{"name": "HasItemBeenBought", "value": false},
                                     {"name": "Stranger", "value": false}]}}]"#,
            r#"[{"id": "RELIC.MAW_BANK",
                 "props": {"bools": [{"name": "HasItemBeenBought", "value": 0}]}}]"#,
            r#"[{"id": "RELIC.MAW_BANK"}]"#,
        ] {
            assert!(
                entry_of(unprovable).relic_counters.is_empty(),
                "{unprovable}"
            );
        }
    }

    #[test]
    fn joss_paper_preserves_saved_counter_and_defaults_omitted_zeros() {
        let good = entry_of(
            r#"[{"id": "RELIC.JOSS_PAPER",
                 "props": {"ints": [{"name": "CardsExhausted", "value": 2},
                                    {"name": "EtherealCount", "value": 0}]}}]"#,
        );
        let bag = good.relic_counters["RELIC.JOSS_PAPER"].as_object().unwrap();
        assert_eq!(bag["CardsExhausted"], Value::from(2));
        assert_eq!(bag["EtherealCount"], Value::from(0));

        let partial = entry_of(
            r#"[{"id": "RELIC.JOSS_PAPER",
                 "props": {"ints": [{"name": "CardsExhausted", "value": 2}]}}]"#,
        );
        assert_eq!(partial.relic_counters, good.relic_counters);
        for relic in [
            r#"[{"id":"RELIC.JOSS_PAPER"}]"#,
            r#"[{"id":"RELIC.JOSS_PAPER","props":null}]"#,
            r#"[{"id":"RELIC.JOSS_PAPER","props":{}}]"#,
        ] {
            assert_eq!(
                entry_of(relic).relic_counters["RELIC.JOSS_PAPER"],
                serde_json::json!({"CardsExhausted": 0, "EtherealCount": 0})
            );
        }
    }

    #[test]
    fn joss_paper_rejects_a_non_false_inherited_flag() {
        let entry = entry_of(
            r#"[{"id": "RELIC.JOSS_PAPER",
                 "props": {"ints": [{"name": "CardsExhausted", "value": 2},
                                    {"name": "EtherealCount", "value": 0}],
                           "bools": [{"name": "IsWax", "value": true}]}}]"#,
        );
        assert!(entry.relic_counters.is_empty());
    }

    #[test]
    fn joss_paper_does_not_default_malformed_or_explicit_values() {
        for props in [
            serde_json::json!({"ints": [{"name": "CardsExhausted", "value": true}]}),
            serde_json::json!({"ints": [{"name": "CardsExhausted", "value": "0"}]}),
            serde_json::json!({"ints": [{"name": "Other", "value": 0}]}),
            serde_json::json!({"ints": [{"name": "CardsExhausted", "value": 1},
                                       {"name": "CardsExhausted", "value": 2}]}),
            serde_json::json!({"bools": [{"name": "IsWax", "value": false},
                                        {"name": "IsWax", "value": false}]}),
            serde_json::json!({"strings": []}),
        ] {
            let relics = serde_json::json!([{"id": "RELIC.JOSS_PAPER", "props": props}]);
            assert!(entry_of(&relics.to_string()).relic_counters.is_empty());
        }
        let explicit = entry_of(
            r#"[{"id":"RELIC.JOSS_PAPER","props":{"ints":[
                {"name":"CardsExhausted","value":5},
                {"name":"EtherealCount","value":1}]}}]"#,
        );
        assert_eq!(
            explicit.relic_counters["RELIC.JOSS_PAPER"],
            serde_json::json!({"CardsExhausted":5,"EtherealCount":1})
        );
    }

    #[test]
    fn fur_coat_membership_is_unprovable_from_any_json_save() {
        // Pins the measured oracle behaviour this port reproduces: the save's
        // coordinates are objects, and the oracle's gate wants a two-element
        // list. See `fur_coat_membership`.
        let entry = entry_of(
            r#"[{"id": "RELIC.FUR_COAT",
                 "props": {"ints": [{"name": "FurCoatActIndex", "value": 0}],
                           "int_arrays": [{"name": "FurCoatCoordCols", "value": []},
                                          {"name": "FurCoatCoordRows", "value": []}],
                           "bools": [{"name": "FurCoatCoordsSet", "value": false}]}}]"#,
        );
        assert_eq!(entry.fur_coat_active, None);
        assert!(entry.relic_counters.is_empty());
    }

    const MELTED: &str = r#"{"bools": [{"name": "IsWax", "value": true},
                                        {"name": "IsMelted", "value": true}]}"#;
    const WAX: &str = r#"{"bools": [{"name": "IsWax", "value": true},
                                     {"name": "IsMelted", "value": false}]}"#;

    fn refusal_of(relics: &str) -> EntryRefusal {
        let run = run_with(relics);
        let player = run.single_player().unwrap().clone();
        relic_entry(&run, &player).unwrap_err()
    }

    #[test]
    fn a_melted_wax_relic_owns_no_hook_and_an_unmelted_one_stays_active() {
        // #3165: f6aceb7447b4d471 carries a melted wax Red Skull that native
        // keeps in `_relics` but skips in both hook walks.
        let entry = entry_of(&format!(
            r#"[{{"id": "RELIC.VAJRA"}},
                {{"id": "RELIC.RED_SKULL", "props": {MELTED}}},
                {{"id": "RELIC.AKABEKO", "props": {WAX}}},
                {{"id": "RELIC.RED_SKULL", "props": {WAX}}}]"#
        ));
        assert_eq!(
            entry.relics_entering,
            ["RELIC.VAJRA", "RELIC.AKABEKO", "RELIC.RED_SKULL"]
        );
        let ids: Vec<crate::ids::RelicId> = entry
            .relics_entering
            .iter()
            .map(|id| crate::ids::RelicId::from_str(id).unwrap())
            .collect();
        let hooks = crate::hooks::HookTable::build(&ids).unwrap();
        assert!(hooks.owns(crate::ids::RelicId::RelicRedSkull));

        let melted_only = entry_of(&format!(
            r#"[{{"id": "RELIC.RED_SKULL", "props": {MELTED}}}]"#
        ));
        assert!(melted_only.relics_entering.is_empty());
        let hooks = crate::hooks::HookTable::build(&[]).unwrap();
        assert!(!hooks.owns(crate::ids::RelicId::RelicRedSkull));
    }

    #[test]
    fn a_melted_relic_contributes_no_counter() {
        let entry = entry_of(
            r#"[{"id": "RELIC.EMBER_TEA",
                 "props": {"ints": [{"name": "CombatsLeft", "value": 3}],
                           "bools": [{"name": "IsWax", "value": true},
                                     {"name": "IsMelted", "value": true}]}},
                {"id": "RELIC.BOOK_OF_FIVE_RINGS",
                 "props": {"ints": [{"name": "CardsAdded", "value": 30}],
                           "bools": [{"name": "IsWax", "value": true},
                                     {"name": "IsMelted", "value": true}]}}]"#,
        );
        assert!(entry.relics_entering.is_empty());
        assert!(entry.relic_counters.is_empty());
    }

    #[test]
    fn melted_relics_native_still_reads_refuse_by_name() {
        for id in ["RELIC.PAPER_PHROG", "RELIC.PAPER_KRANE"] {
            let refusal = refusal_of(&format!(r#"[{{"id": "{id}", "props": {MELTED}}}]"#));
            assert_eq!(refusal.class(), "melted_relic_still_read");
            assert!(refusal.to_string().contains(id));
        }
        // An unmelted wax copy is an ordinary owned relic.
        let entry = entry_of(&format!(
            r#"[{{"id": "RELIC.PAPER_PHROG", "props": {WAX}}}]"#
        ));
        assert_eq!(entry.relics_entering, ["RELIC.PAPER_PHROG"]);
    }

    #[test]
    fn a_melted_whispering_earring_refuses_only_beside_an_active_paels_eye() {
        for order in [
            format!(
                r#"[{{"id": "RELIC.PAELS_EYE"}}, {{"id": "RELIC.WHISPERING_EARRING", "props": {MELTED}}}]"#
            ),
            format!(
                r#"[{{"id": "RELIC.WHISPERING_EARRING", "props": {MELTED}}}, {{"id": "RELIC.PAELS_EYE"}}]"#
            ),
        ] {
            assert_eq!(refusal_of(&order).class(), "melted_relic_still_read");
        }
        let alone = entry_of(&format!(
            r#"[{{"id": "RELIC.WHISPERING_EARRING", "props": {MELTED}}}]"#
        ));
        assert!(alone.relics_entering.is_empty());
        let both_melted = entry_of(&format!(
            r#"[{{"id": "RELIC.PAELS_EYE", "props": {MELTED}}},
                {{"id": "RELIC.WHISPERING_EARRING", "props": {MELTED}}}]"#
        ));
        assert!(both_melted.relics_entering.is_empty());
    }

    #[test]
    fn unreachable_melted_flags_refuse() {
        for props in [
            r#"{"bools": [{"name": "IsMelted", "value": true}]}"#,
            r#"{"bools": [{"name": "IsWax", "value": false}, {"name": "IsMelted", "value": true}]}"#,
            r#"{"bools": [{"name": "IsWax", "value": true}, {"name": "IsWax", "value": true},
                          {"name": "IsMelted", "value": true}]}"#,
            r#"{"bools": [{"name": "IsWax", "value": true}, {"name": "IsMelted", "value": true},
                          {"name": "IsMelted", "value": true}]}"#,
            r#"{"bools": [{"name": "IsWax", "value": true}, {"name": "IsMelted", "value": true}],
                "ints": [{"name": "IsMelted", "value": 1}]}"#,
        ] {
            let refusal = refusal_of(&format!(
                r#"[{{"id": "RELIC.RED_SKULL", "props": {props}}}]"#
            ));
            assert_eq!(refusal.class(), "melted_relic_flags", "{props}");
        }
        // No `IsMelted: true` row leaves the relic owned, as before.
        let entry = entry_of(
            r#"[{"id": "RELIC.RED_SKULL",
                 "props": {"bools": [{"name": "IsMelted", "value": false},
                                     {"name": "IsMelted", "value": false}]}}]"#,
        );
        assert_eq!(entry.relics_entering, ["RELIC.RED_SKULL"]);
    }

    #[test]
    fn an_unknown_saved_property_group_refuses_by_name() {
        let run = run_with(r#"[{"id": "RELIC.VAJRA", "props": {"floats": []}}]"#);
        let player = run.single_player().unwrap().clone();
        assert_eq!(
            relic_entry(&run, &player).unwrap_err().class(),
            "saved_property_group_unhandled"
        );
    }
}
