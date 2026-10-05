//! Relic entry state: the inventory, its dispatch-order vouch, and the saved
//! properties that persist across combats.
//!
//! Oracle: the relic loop of `live_coach.build_entry`, plus the two strict
//! adapters `live_coach._strict_scoped_relic_property` and
//! `live_coach._strict_joss_paper_properties`. Fur Coat's membership is read
//! from the game's IL instead (#2526): the oracle's adapter for it never
//! answered on a save.
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
use crate::entry::save::{Coord, SerializedPlayer, SerializedRun};

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

/// Whether `FurCoat`'s combat bodies fire in the room this save stands in, or
/// `None` when the save cannot say (#2526).
///
/// IL read on the v0.111.0 DLL (sha256 `9cb4f1ad…`, `dump_il.py FurCoat`).
///
/// # The reader
///
/// Both combat bodies ask the same question. `<BeforeCombatStart>d__26`
/// (RVA `0x325360`) calls `GetMarkedCoords` (`IL_0020`-`IL_0021`), leaves when
/// it is null (`IL_0027`-`IL_0028`), and otherwise tests
/// `List<MapCoord>::Contains(Owner.RunState.CurrentMapPoint.coord)`
/// (`IL_002a`-`IL_0045`); `<AfterCreatureAddedToCombat>d__27` (`0x325238`)
/// repeats it at `IL_0033`-`IL_0058` behind its enemy-side test. Neither reads
/// `FurCoatActIndex` or `CurrentActIndex`: **the reader has no act test**, so
/// a coordinate marked in one act also answers in a later one.
///
/// * `GetMarkedCoords` (`0x94204`) is null exactly when `FurCoatCoordsSet` is
///   false (`IL_000c`-`IL_0015`); otherwise it pairs `FurCoatCoordCols[i]`
///   with `FurCoatCoordRows[i]` for `i < FurCoatCoordCols.Length`
///   (`IL_0020`-`IL_005a`), so a shorter `Rows` array throws and a longer one
///   is silently truncated. Unequal lengths are unprovable here.
/// * `RunState::get_CurrentMapPoint` (`0x4e1b4`) is
///   `Map.GetPoint(CurrentMapCoord)`, and `get_CurrentMapCoord` (`0x4e180`) is
///   `_visitedMapCoords.Last()` (`IL_0019`-`IL_0024`), null when the list is
///   empty. `ActMap::GetPoint` (`0xf89ac`) answers the boss, second boss and
///   starting points by coordinate and the grid otherwise, and can be null;
///   the reader dereferences `.coord` unguarded (`IL_003b`), so a current
///   coordinate the saved map does not hold is unprovable rather than
///   "unmarked".
/// * `MapCoord::Equals` (`0xf8dc2`) is `col == col && row == row`.
///
/// `visited_map_coords` is that `_visitedMapCoords` list: a JSON run save
/// writes each entry as a `{"col", "row"}` object and a capture as a
/// `[col, row]` pair, and both parse to [`Coord`], so the last entry is the
/// coordinate the reader compares in either form. (The deleted Python adapter
/// this function replaced demanded a two-element list of the JSON save and so
/// never answered. Measured 2026-10-02 over `~/sts2-captures`: 27,530 of the
/// 27,530 coordinates in its 3,400 JSON saves are objects, and 5,912 of 5,912
/// in its 693 uploaded captures are pairs.)
///
/// # The writer, and why the relic's own act is checked harder
///
/// `AddMarkedRooms` (`0x93f38`) is the only writer, reached from
/// `AfterObtained` (`0x93ef9`, which first sets `FurCoatActIndex` to the
/// current act) and from `ModifyGeneratedMapLate` (`0x93f2d`) whenever a map
/// is built. It returns at once when `CurrentActIndex != FurCoatActIndex`
/// (`IL_0019`-`IL_0037`). In the relic's own act it **re-rolls** the marks
/// when `GetMarkedCoords` is null or when any mark fails `<AddMarkedRooms>b__0`
/// (`0x3251f9`: on the map, and `PointType` `Monster` (5) or `Elite` (6)) —
/// `IL_0038`-`IL_005e` — taking `DynamicVars["Combats"]` fresh points and
/// setting `FurCoatCoordsSet` (`IL_0063`-`IL_0151`).
///
/// So a save taken in the relic's own act whose marks that test would reject,
/// or whose `FurCoatCoordsSet` is false, does not determine the fight: a map
/// rebuilt between the save and the combat re-rolls them. Those are
/// unprovable. In any other act nothing rewrites the latch and the saved
/// values are the answer.
///
/// # Strictness
///
/// The complete native shape or nothing: exactly `FurCoatActIndex` (`ints`),
/// `FurCoatCoordsSet` (`bools`, beside the two inherited flags at `false`) and
/// the two coordinate arrays (`int_arrays`), each once, every number a 32-bit
/// integer. Anything else is `None`, which `entry::opening` refuses; it is
/// never read as "inactive".
fn fur_coat_membership(run: &SerializedRun, props: &SavedProperties) -> Option<bool> {
    let latch = FurCoatLatch::read(props)?;
    let act = i64::try_from(run.current_act_index?).ok()?;
    let map = run.acts.get(run.act_index())?.saved_map.as_ref()?;
    let point_type = |coord: Coord| {
        map.points
            .iter()
            .chain(map.boss.iter())
            .chain(map.second_boss.iter())
            .chain(map.start.iter())
            .find(|point| point.coord == Some(coord))
            .map(|point| {
                point
                    .point_type
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
            })
    };
    if act == latch.act_index {
        // The relic's own act: a rebuilt map re-rolls a latch `AddMarkedRooms`
        // would reject, so only one it would keep is an answer.
        if !latch.coords_set {
            return None;
        }
        for mark in &latch.marks {
            let kind = point_type(*mark)?;
            if kind != "monster" && kind != "elite" {
                return None;
            }
        }
    }
    if !latch.coords_set {
        // `GetMarkedCoords` is null, and the reader leaves before it reads the
        // current map point.
        return Some(false);
    }
    let current = *run.visited_map_coords.last()?;
    point_type(current)?;
    Some(latch.marks.contains(&current))
}

/// Fur Coat's four saved properties, validated as one complete shape.
struct FurCoatLatch {
    act_index: i64,
    coords_set: bool,
    marks: Vec<Coord>,
}

impl FurCoatLatch {
    fn read(props: &SavedProperties) -> Option<Self> {
        if props
            .group_names()
            .iter()
            .any(|group| !["ints", "bools", "int_arrays"].contains(group))
        {
            return None;
        }
        let int32 = |value: &Value| {
            value
                .as_i64()
                .filter(|number| i32::try_from(*number).is_ok())
        };
        let [act] = props.group("ints") else {
            return None;
        };
        if act.name != "FurCoatActIndex" {
            return None;
        }
        let act_index = int32(&act.value)?;

        let mut coords_set = None;
        let mut inherited = std::collections::BTreeSet::new();
        for row in props.group("bools") {
            if row.name == "FurCoatCoordsSet" {
                if coords_set.replace(row.value.as_bool()?).is_some() {
                    return None;
                }
            } else if !INHERITED_FALSE_RELIC_PROPERTIES.contains(&row.name.as_str())
                || row.value != Value::Bool(false)
                || !inherited.insert(&row.name)
            {
                return None;
            }
        }
        let coords_set = coords_set?;

        let (mut cols, mut rows) = (None, None);
        for row in props.group("int_arrays") {
            let slot = match row.name.as_str() {
                "FurCoatCoordCols" => &mut cols,
                "FurCoatCoordRows" => &mut rows,
                _ => return None,
            };
            let values = row
                .value
                .as_array()?
                .iter()
                .map(int32)
                .collect::<Option<Vec<i64>>>()?;
            if slot.replace(values).is_some() {
                return None;
            }
        }
        let (cols, rows) = (cols?, rows?);
        if cols.len() != rows.len() {
            return None;
        }
        Some(Self {
            act_index,
            coords_set,
            marks: cols
                .into_iter()
                .zip(rows)
                .map(|(col, row)| Coord { col, row })
                .collect(),
        })
    }
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

    /// A save standing on `(1, 2)` of act `act`, whose map holds monster
    /// points at `(1, 2)`, `(3, 4)` and `(5, 6)`, an elite at `(0, 7)`, a rest
    /// site at `(2, 5)` and a boss at `(3, 15)`.
    fn fur_coat_run(act: usize, props: &serde_json::Value) -> serde_json::Value {
        let map = serde_json::json!({
            "points": [
                {"coord": {"col": 1, "row": 2}, "type": "monster"},
                {"coord": {"col": 3, "row": 4}, "type": "monster"},
                {"coord": {"col": 5, "row": 6}, "type": "monster"},
                {"coord": {"col": 0, "row": 7}, "type": "elite"},
                {"coord": {"col": 2, "row": 5}, "type": "rest_site"}],
            "boss": {"coord": {"col": 3, "row": 15}, "type": "boss"}});
        serde_json::json!({
            "schema_version": 20,
            "current_act_index": act,
            "acts": [{"saved_map": map}, {"saved_map": map}, {"saved_map": map}],
            "visited_map_coords": [{"col": 3, "row": 0}, {"col": 1, "row": 2}],
            "rng": {"seed": "S", "rngs": {}},
            "players": [{"relics": [{"id": "RELIC.FUR_COAT", "props": props}]}]})
    }

    fn fur_coat_props(act: i64, set: bool, cols: &[i64], rows: &[i64]) -> serde_json::Value {
        serde_json::json!({
            "ints": [{"name": "FurCoatActIndex", "value": act}],
            "int_arrays": [{"name": "FurCoatCoordCols", "value": cols},
                           {"name": "FurCoatCoordRows", "value": rows}],
            "bools": [{"name": "FurCoatCoordsSet", "value": set}]})
    }

    fn fur_coat_of(run: &serde_json::Value) -> Option<bool> {
        let run = SerializedRun::parse(&run.to_string()).unwrap();
        let player = run.single_player().unwrap().clone();
        let entry = relic_entry(&run, &player).unwrap();
        assert_eq!(entry.relics_entering, ["RELIC.FUR_COAT"]);
        assert!(entry.relic_counters.is_empty());
        entry.fur_coat_active
    }

    #[test]
    fn fur_coat_is_active_exactly_when_the_marks_hold_the_current_coordinate() {
        // The save stands on (1, 2). Marked there: active.
        let marked = fur_coat_props(2, true, &[3, 1, 0], &[4, 2, 7]);
        assert_eq!(fur_coat_of(&fur_coat_run(2, &marked)), Some(true));
        // Marked elsewhere: inactive. (The transposed (2, 1) and marks sharing
        // one axis are in the other-act test, where a mark need not be a
        // combat point of this map.)
        let elsewhere = fur_coat_props(2, true, &[3, 0], &[4, 7]);
        assert_eq!(fur_coat_of(&fur_coat_run(2, &elsewhere)), Some(false));
        let empty = fur_coat_props(2, true, &[], &[]);
        assert_eq!(fur_coat_of(&fur_coat_run(2, &empty)), Some(false));
        // The inherited flags may ride along at false.
        let mut flagged = marked.clone();
        flagged["bools"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "IsWax", "value": false}));
        assert_eq!(fur_coat_of(&fur_coat_run(2, &flagged)), Some(true));
    }

    #[test]
    fn fur_coat_in_another_act_answers_from_the_saved_marks_alone() {
        // The reader has no act test, and outside the relic's own act nothing
        // re-rolls the latch: the marks need not be combat points of this
        // act's map, or on it at all.
        let hit = fur_coat_props(0, true, &[6, 1, 2], &[13, 2, 5]);
        assert_eq!(fur_coat_of(&fur_coat_run(1, &hit)), Some(true));
        // Off the map, the transposed (2, 1), and one shared axis each way.
        let miss = fur_coat_props(0, true, &[6, 2, 1, 5], &[13, 1, 5, 2]);
        assert_eq!(fur_coat_of(&fur_coat_run(1, &miss)), Some(false));
        // Never marked, and not in the act that would mark it on a rebuild.
        let unset = fur_coat_props(-1, false, &[], &[]);
        assert_eq!(fur_coat_of(&fur_coat_run(1, &unset)), Some(false));
    }

    #[test]
    fn fur_coat_in_its_own_act_is_unprovable_when_a_rebuilt_map_would_re_roll_it() {
        // Unset in the relic's own act: `AddMarkedRooms` rolls on a rebuild.
        let unset = fur_coat_props(1, false, &[], &[]);
        assert_eq!(fur_coat_of(&fur_coat_run(1, &unset)), None);
        // A mark on a rest site, a boss, or no point at all fails
        // `<AddMarkedRooms>b__0`, whether or not the current room is marked.
        for (cols, rows) in [
            ([1, 2], [2, 5]),
            ([1, 3], [2, 15]),
            ([1, 6], [2, 13]),
            ([3, 2], [4, 5]),
        ] {
            let props = fur_coat_props(1, true, &cols, &rows);
            assert_eq!(fur_coat_of(&fur_coat_run(1, &props)), None, "{cols:?}");
        }
    }

    #[test]
    fn fur_coat_is_unprovable_without_the_current_map_point() {
        let props = fur_coat_props(0, true, &[1], &[2]);
        // No current act index, no saved map, no visited coordinate, and a
        // current coordinate the map does not hold.
        let mut run = fur_coat_run(1, &props);
        run.as_object_mut().unwrap().remove("current_act_index");
        assert_eq!(fur_coat_of(&run), None);
        let mut run = fur_coat_run(1, &props);
        run["acts"][1] = serde_json::json!({});
        assert_eq!(fur_coat_of(&run), None);
        let mut run = fur_coat_run(1, &props);
        run["visited_map_coords"] = serde_json::json!([]);
        assert_eq!(fur_coat_of(&run), None);
        let mut run = fur_coat_run(1, &props);
        run["visited_map_coords"] = serde_json::json!([{"col": 6, "row": 13}]);
        assert_eq!(fur_coat_of(&run), None);
        // An unset latch outside its own act never reads the map point.
        let unset = fur_coat_props(0, false, &[], &[]);
        let mut run = fur_coat_run(1, &unset);
        run["visited_map_coords"] = serde_json::json!([{"col": 6, "row": 13}]);
        assert_eq!(fur_coat_of(&run), Some(false));
    }

    #[test]
    fn fur_coat_malformed_properties_are_unprovable_never_inactive() {
        use serde_json::json;
        let good = fur_coat_props(0, true, &[3], &[4]);
        assert_eq!(fur_coat_of(&fur_coat_run(1, &good)), Some(false));
        let row = |name: &str, value: serde_json::Value| json!({"name": name, "value": value});
        let mut cases = vec![json!(null), json!({})];
        // A missing property, one at a time.
        for (group, name) in [
            ("ints", "FurCoatActIndex"),
            ("bools", "FurCoatCoordsSet"),
            ("int_arrays", "FurCoatCoordCols"),
            ("int_arrays", "FurCoatCoordRows"),
        ] {
            let mut props = good.clone();
            props[group]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["name"] != name);
            cases.push(props);
            let mut props = good.clone();
            let rows = props[group].as_array_mut().unwrap();
            let duplicate = rows.iter().find(|entry| entry["name"] == name).unwrap();
            rows.push(duplicate.clone());
            cases.push(props);
        }
        // Wrong types, out-of-range numbers, unequal lengths.
        for (group, name, value) in [
            ("ints", "FurCoatActIndex", json!("0")),
            ("ints", "FurCoatActIndex", json!(true)),
            ("ints", "FurCoatActIndex", json!(0.5)),
            ("ints", "FurCoatActIndex", json!(1_i64 << 31)),
            ("bools", "FurCoatCoordsSet", json!(1)),
            ("bools", "FurCoatCoordsSet", json!(null)),
            ("int_arrays", "FurCoatCoordCols", json!(3)),
            ("int_arrays", "FurCoatCoordCols", json!([true])),
            ("int_arrays", "FurCoatCoordCols", json!(["3"])),
            ("int_arrays", "FurCoatCoordCols", json!([1_i64 << 31])),
            ("int_arrays", "FurCoatCoordCols", json!([3, 1])),
            ("int_arrays", "FurCoatCoordRows", json!([])),
            ("int_arrays", "FurCoatCoordRows", json!([4, 2])),
        ] {
            let mut props = good.clone();
            let rows = props[group].as_array_mut().unwrap();
            let slot = rows.iter_mut().find(|entry| entry["name"] == name).unwrap();
            slot["value"] = value;
            cases.push(props);
        }
        // A stray row, a stray group, and an inherited flag that is not false.
        for (group, extra) in [
            ("ints", row("CombatsLeft", json!(1))),
            ("bools", row("WasUsed", json!(false))),
            ("bools", row("IsWax", json!(true))),
            ("int_arrays", row("Other", json!([]))),
        ] {
            let mut props = good.clone();
            props[group].as_array_mut().unwrap().push(extra);
            cases.push(props);
        }
        let mut doubled_flag = good.clone();
        for _ in 0..2 {
            doubled_flag["bools"]
                .as_array_mut()
                .unwrap()
                .push(row("IsWax", json!(false)));
        }
        cases.push(doubled_flag);
        let mut stray_group = good.clone();
        stray_group["strings"] = json!([]);
        cases.push(stray_group);
        for props in cases {
            assert_eq!(fur_coat_of(&fur_coat_run(1, &props)), None, "{props}");
        }
        // No `props` key at all is the same unprovable.
        let mut bare = fur_coat_run(1, &good);
        bare["players"][0]["relics"] = json!([{"id": "RELIC.FUR_COAT"}]);
        assert_eq!(fur_coat_of(&bare), None);
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
