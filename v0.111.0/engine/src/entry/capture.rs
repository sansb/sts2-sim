//! A replay capture's embedded run (`CombatReplay.run`), mapped exactly onto
//! the schema-20 save surface (#2972, #2827 item A).
//!
//! # What this input is
//!
//! A `.mcr` capture embeds the same `Saves.SerializableRun` object a `.save`
//! file holds, but written by a different serializer. The `.save` is
//! `System.Text.Json` with snake-case names, converters and
//! `SerializationCondition` omissions ([`crate::entry::save`]). The capture is
//! the bit-packed `PacketWriter` path: each type's `Serialize(PacketWriter)`
//! writes its properties positionally, with no names at all. The `.mcr` decoder
//! is not ported (§A5); `sim/v0.111.0/python/mcr_parser.py`
//! (`McrDecoder.serializable_run`) transcribes those `Serialize` bodies and
//! **chooses its own key names**. This module parses that decoded JSON with
//! `deny_unknown_fields`, so an untaught key is a named refusal, and maps each
//! field onto [`SerializedRun`] by **position in the IL**, not by the
//! decoder's label.
//!
//! The mapping is exact rather than guessed field by field because every
//! correspondence below was read off the matching `Deserialize(PacketReader)`
//! body in v0.111.0 `sts2.dll` (sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`), which
//! names the setter each positional read feeds and, for model ids, the model
//! type whose category the bare entry belongs to:
//!
//! | decoder key | native property | IL |
//! |---|---|---|
//! | `run_odds.card_reward_odds` | `SerializableRunOddsSet.UnknownMapPointMonsterOddsValue` | `SerializableRunOddsSet::Serialize` RVA 0x42010, first `WriteFloat` (IL_001c) |
//! | `run_odds.potion_reward_odds` | `…UnknownMapPointEliteOddsValue` | same, IL_0031 |
//! | `run_odds.relic_reward_odds` | `…UnknownMapPointTreasureOddsValue` | same, IL_0046 |
//! | `run_odds.gold_reward_odds` | `…UnknownMapPointShopOddsValue` | same, IL_005b |
//! | `rooms.elites_visited` | `SerializableRoomSet.EliteEncountersVisited` | `SerializableRoomSet::Deserialize` RVA 0x41f1c IL_0055 |
//! | `rooms.bosses_visited` | `…BossEncountersVisited` | same, IL_0063 |
//! | `saved_map.grid_width` / `grid_height` | `SerializableActMap.GridWidth` / `GridHeight` | `SerializableActMap::Deserialize` RVA 0x40594 IL_0014 / IL_0021 |
//! | `saved_map.boss_point` / `starting_point` / `second_boss_point` | `BossPoint` / `StartingPoint` / `SecondBossPoint` | same, IL_002d / IL_0039 / IL_004d |
//! | `saved_map.start_map_point_coords` | `StartMapPointCoords` | same, IL_0097 |
//! | `point.coord` / `point.point_type` | `SerializableMapPoint.Coord`, then `PointType` (`MapPointType`, written as an int) | `SerializableMapPoint::Deserialize` RVA 0x40e1c IL_0013 / IL_0020 (the decoder read these in the other order until the #2988 review; the corpus pair's whole-map comparison is the witness) |
//! | `point.can_be_modified` / `child_coords` | `CanBeModified` / `ChildCoords` | same, IL_002c / IL_0044 |
//! | `[col, row]` | `MapCoord.col`, `MapCoord.row` | `MapCoord::Serialize` RVA 0xf8d72: `col` IL_0003, then `row` IL_0011 |
//! | `player.character` | `SerializablePlayer.CharacterId` (`CharacterModel`) | `SerializablePlayer::Deserialize` RVA 0x41290 IL_001c–0021 |
//! | `player.max_potion_slots` / `base_orb_slots` | `MaxPotionSlotCount` / `BaseOrbSlotCount` | same, IL_0058 / IL_0074 |
//! | `player.player_rng` | `SerializablePlayer.Rng` | same, IL_00aa |
//! | `card.upgrade_level` | `SerializableCard.CurrentUpgradeLevel` | `SerializableCard::Deserialize` RVA 0x40998 IL_0020 |
//! | `enchantment.level` | `SerializableEnchantment.Amount` | `SerializableEnchantment::Deserialize` RVA 0x40b98 IL_0020 |
//! | `relic.floor_added` | `SerializableRelic.FloorAddedToDeck` | `SerializableRelic::Deserialize` RVA 0x416f4 IL_0045 |
//! | `potion.slot` | `SerializablePotion.SlotIndex` | `SerializablePotion::Deserialize` RVA 0x41624 IL_0015 |
//!
//! Every other decoder key already spells the save's JSON name. The model-id
//! categories are the `ReadModelIdAssumingType<T>` / `ReadModelIdListAssumingType<T>`
//! type arguments at the same sites: `EventModel` (`events_seen`, `event_ids`),
//! `EncounterModel` (encounter lists, `boss_id`, `second_boss_id`,
//! `encounters_seen`), `AncientEventModel` (`ancient_id`, category `EVENT`),
//! `ActModel`, `ModifierModel`, `CardModel`, `EnchantmentModel`,
//! `RelicModel` (relics and both grab bags), `PotionModel`, and
//! `CharacterModel`. The `discovered_*` lists are `ReadFullModelIdList`, so
//! they already carry their category.
//!
//! # `SavedProperties`: the one place the decoder loses a fact
//!
//! `SavedProperties::Serialize` (RVA 0x3f9b8) writes seven typed groups, and
//! the save keeps them apart (`{"ints": [{name, value}], …}`, see
//! [`crate::entry::props`]). The decoder flattens them to `{name: value}`,
//! which erases the group. A string could be a `strings` or a `model_ids` row,
//! and an empty list could be `int_arrays` or `card_arrays`. The group is
//! recovered from the **property's declared type**, not from the value: each
//! `[SavedProperty]` property's signature, read over the same
//! `CachePropertiesForType` set `tools/build_mcr_tables.py` walks for the
//! property-name net ids (47 names on v0.111.0). [`property_group`] is that
//! table. A name outside it is [`EntryRefusal::CapturePropertyUnknown`], and a
//! value whose JSON type contradicts the declared type is
//! [`EntryRefusal::CapturePropertyType`]. Neither is guessed.
//!
//! # What is carried opaquely
//!
//! `map_point_history` is read only for its per-act lengths
//! ([`crate::entry::node::current_node`]), so its rows are carried as the
//! decoder wrote them, exactly as the save's rows are carried untyped.
//! `map_drawings` is a converter-encoded blob in the save and a list in the
//! capture. No entry fact reads either one. `pre_finished_room` is null on
//! every corpus capture; a non-null one is [`EntryRefusal::CaptureFieldUnmapped`]
//! rather than a mapping that no capture has exercised.
//!
//! # Absence
//!
//! The binary writer has no `SerializationCondition`: every property is
//! written, including the ones the JSON save omits at their defaults. The
//! mapping therefore produces `Some(default)` where a save would have no key.
//! Every entry read site treats an absent default and a present default the
//! same (for example `current_upgrade_level.unwrap_or(0)`), except
//! `ascension`. `build_entry` writes `save.get("ascension")`, and the opening
//! refuses `None`. There, the capture's explicit `0` is the recorded fact.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::entry::canonical_model_id;
use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{
    ADMITTED_SAVE_SCHEMA, Coord, RUN_RNG_STREAMS, SerializedAct, SerializedActMap, SerializedCard,
    SerializedEnchantment, SerializedExtraPlayerFields, SerializedExtraRunFields,
    SerializedMapPoint, SerializedModifier, SerializedPlayer, SerializedPlayerOdds,
    SerializedPlayerRngSet, SerializedPotion, SerializedRelic, SerializedRelicGrabBag,
    SerializedRng, SerializedRoomSet, SerializedRun, SerializedRunOdds, SerializedRunRngSet,
    SerializedUnlockState,
};

/// `Entities.Rngs.RunRngType` member names, in declaration order. The decoder
/// keys its stream maps by these; the save keys them by the snake-cased
/// [`RUN_RNG_STREAMS`] at the same index (`SnakeCaseJsonStringEnumConverter`).
pub const RUN_RNG_TYPE_NAMES: [&str; 12] = [
    "UpFront",
    "Shuffle",
    "UnknownMapPoint",
    "CombatCardGeneration",
    "CombatPotionGeneration",
    "CombatCardSelection",
    "CombatEnergyCosts",
    "CombatTargets",
    "MonsterAi",
    "Niche",
    "CombatOrbs",
    "TreasureRoomRelics",
];

/// `Entities.Rngs.PlayerRngType`, and its snake-cased save spelling.
const PLAYER_RNG_TYPES: [(&str, &str); 3] = [
    ("Rewards", "rewards"),
    ("Shops", "shops"),
    ("Transformations", "transformations"),
];

/// `Map.MapPointType`, and its snake-cased save spelling.
const MAP_POINT_TYPE_NAMES: [(&str, &str); 9] = [
    ("Unassigned", "unassigned"),
    ("Unknown", "unknown"),
    ("Shop", "shop"),
    ("Treasure", "treasure"),
    ("RestSite", "rest_site"),
    ("Monster", "monster"),
    ("Elite", "elite"),
    ("Boss", "boss"),
    ("Ancient", "ancient"),
];

/// `Runs.GameMode`, and its snake-cased save spelling.
const GAME_MODE_NAMES: [(&str, &str); 4] = [
    ("None", "none"),
    ("Standard", "standard"),
    ("Daily", "daily"),
    ("Custom", "custom"),
];

/// `Entities.Relics.RelicRarity`, and its snake-cased save spelling.
const RELIC_RARITY_NAMES: [(&str, &str); 8] = [
    ("None", "none"),
    ("Starter", "starter"),
    ("Common", "common"),
    ("Uncommon", "uncommon"),
    ("Rare", "rare"),
    ("Shop", "shop"),
    ("Event", "event"),
    ("Ancient", "ancient"),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRng {
    seed: Value,
    counters: BTreeMap<String, i64>,
    states: BTreeMap<String, [u64; 4]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureEnchantment {
    id: String,
    level: i64,
    props: Option<OrderedProps>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureCard {
    id: String,
    upgrade_level: i64,
    enchantment: Option<CaptureEnchantment>,
    props: Option<OrderedProps>,
    floor_added_to_deck: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRelic {
    id: String,
    props: Option<OrderedProps>,
    floor_added: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePotion {
    id: String,
    slot: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePlayerOdds {
    card_rarity_odds_value: f64,
    potion_reward_odds_value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureExtraPlayerFields {
    card_shop_removals_used: i64,
    wongo_points: i64,
    /// The decoder's spelling (three `c`s); the save's is the assembly's
    /// four-`c` literal ([`SerializedExtraPlayerFields`]).
    cccombo_badge_unlocked: bool,
    damage_dealt: i64,
    debuffs_applied: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureUnlockState {
    unlocked_epochs: Vec<String>,
    encounters_seen: Vec<String>,
    number_of_runs: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePlayer {
    net_id: u64,
    character: String,
    current_hp: i64,
    max_hp: i64,
    max_energy: i64,
    max_potion_slots: i64,
    gold: i64,
    base_orb_slots: i64,
    deck: Vec<CaptureCard>,
    relics: Vec<CaptureRelic>,
    potions: Vec<CapturePotion>,
    player_rng: CaptureRng,
    odds: CapturePlayerOdds,
    relic_grab_bag: BTreeMap<String, Vec<String>>,
    extra_fields: CaptureExtraPlayerFields,
    unlock_state: CaptureUnlockState,
    discovered_cards: Vec<String>,
    discovered_enemies: Vec<String>,
    discovered_epochs: Vec<String>,
    discovered_potions: Vec<String>,
    discovered_relics: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureMapPoint {
    coord: [i64; 2],
    point_type: String,
    can_be_modified: bool,
    child_coords: Vec<[i64; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureActMap {
    grid_width: i64,
    grid_height: i64,
    boss_point: CaptureMapPoint,
    starting_point: CaptureMapPoint,
    second_boss_point: Option<CaptureMapPoint>,
    points: Vec<CaptureMapPoint>,
    start_map_point_coords: Vec<[i64; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRoomSet {
    event_ids: Vec<String>,
    events_visited: usize,
    normal_encounter_ids: Vec<String>,
    normal_encounters_visited: usize,
    elite_encounter_ids: Vec<String>,
    elites_visited: usize,
    bosses_visited: usize,
    boss_id: Option<String>,
    second_boss_id: Option<String>,
    ancient_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureAct {
    id: String,
    rooms: CaptureRoomSet,
    saved_map: Option<CaptureActMap>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureModifier {
    id: String,
    props: Option<OrderedProps>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRunOdds {
    card_reward_odds: f64,
    potion_reward_odds: f64,
    relic_reward_odds: f64,
    gold_reward_odds: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureExtraRunFields {
    started_with_neow: bool,
    test_subject_kills: i64,
    freed_repy: bool,
}

/// `CombatReplay.run` as `mcr_parser.McrDecoder.serializable_run` spells it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRun {
    schema_version: u32,
    acts: Vec<CaptureAct>,
    modifiers: Vec<CaptureModifier>,
    daily_time: Option<i64>,
    game_mode: String,
    current_act_index: usize,
    events_seen: Vec<String>,
    pre_finished_room: Option<Value>,
    run_odds: CaptureRunOdds,
    players: Vec<CapturePlayer>,
    rng: CaptureRng,
    shared_relic_grab_bag: BTreeMap<String, Vec<String>>,
    visited_map_coords: Vec<[i64; 2]>,
    map_point_history: Vec<Value>,
    save_time: i64,
    start_time: i64,
    run_time: i64,
    win_time: i64,
    ascension: i64,
    map_drawings: Option<Value>,
    extra_fields: CaptureExtraRunFields,
    num_reloads: i64,
}

/// Just the schema, parsed before anything else is read.
#[derive(Debug, Deserialize)]
struct SchemaProbe {
    #[serde(default)]
    schema_version: Option<u32>,
}

/// One decoded property value.
#[derive(Debug)]
enum PropValue {
    /// A scalar, string or int array.
    Plain(Value),
    /// A `SerializableCard` (`StarterCard`, and `AncientCard` where it is one).
    Card(Box<CaptureCard>),
    /// A `List<SerializableCard>` (`SerializableCards`).
    Cards(Vec<CaptureCard>),
}

/// A decoded `{name: value}` property bag, **in the decoder's order**.
///
/// `serde_json::Map` here is sorted by key (the crate does not enable
/// `preserve_order`, and must not: canonical documents serialize through it),
/// so parsing the bag as a map would reorder rows the writer emitted in list
/// order. Mad Science's `TinkerTimeType` / `TinkerTimeRider` pair is the
/// corpus witness: the save lists `TinkerTimeType` first, and a sorted map
/// puts `TinkerTimeRider` first. The visitor below reads the entries as they
/// arrive, and parses the card-typed names straight into [`CaptureCard`], so
/// a nested card's own bag keeps its order too.
#[derive(Debug)]
struct OrderedProps(Vec<(String, PropValue)>);

impl<'de> Deserialize<'de> for OrderedProps {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> serde::de::Visitor<'de> for Entries {
            type Value = OrderedProps;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a decoded SavedProperties object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<OrderedProps, A::Error> {
                let mut entries = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    let value = match name.as_str() {
                        "StarterCard" => PropValue::Card(map.next_value()?),
                        "SerializableCards" => PropValue::Cards(map.next_value()?),
                        "AncientCard" => map.next_value::<CardOrModelId>()?.0,
                        _ => PropValue::Plain(map.next_value()?),
                    };
                    entries.push((name, value));
                }
                Ok(OrderedProps(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

/// `AncientCard`, declared as a `ModelId` on one model and a
/// `SerializableCard` on another: a string is the first, an object the second.
struct CardOrModelId(PropValue);

impl<'de> Deserialize<'de> for CardOrModelId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Either;
        impl<'de> serde::de::Visitor<'de> for Either {
            type Value = CardOrModelId;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a model id string or a card object")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<CardOrModelId, E> {
                Ok(CardOrModelId(PropValue::Plain(Value::String(
                    value.to_string(),
                ))))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<CardOrModelId, A::Error> {
                let card =
                    CaptureCard::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(CardOrModelId(PropValue::Card(Box::new(card))))
            }
        }
        deserializer.deserialize_any(Either)
    }
}

/// The `SavedProperties` group a `[SavedProperty]` property's declared type
/// selects, keyed by the property name the decoder writes.
///
/// Read from each property's signature over the v0.111.0
/// `CachePropertiesForType` set (the 47 names of `mcr_tables.json`
/// `property_names`): `int` and the two enum-typed properties
/// (`TinkerTimeType: CardType`, `TinkerTimeRider: RiderEffect`) are `ints`,
/// `bool` is `bools`, `string` is `strings`, `ModelId` is `model_ids`,
/// `SerializableCard` is `cards`, `int[]` is `int_arrays`, and
/// `List<SerializableCard>` is `card_arrays`. `AncientCard` is declared twice,
/// as `ModelId` on one model and `SerializableCard` on another; the decoded
/// value's JSON type separates those two, and nothing else needs to.
///
/// A nested card has already been parsed as a [`CaptureCard`] by
/// [`OrderedProps`], which reads the card-typed names that way, so only the
/// plain values are type-checked here.
fn property_group(
    owner: &str,
    name: &str,
    value: &PropValue,
) -> Result<&'static str, EntryRefusal> {
    let group = match name {
        "AttacksPlayed"
        | "CardsAdded"
        | "CardsExhausted"
        | "CardsPlayed"
        | "CombatRewardsSeen"
        | "CombatsFinished"
        | "CombatsLeft"
        | "CombatsSeen"
        | "CurrentBlock"
        | "CurrentDamage"
        | "ElitesDefeated"
        | "FurCoatActIndex"
        | "GoldenPathAct"
        | "IncreasedBlock"
        | "IncreasedDamage"
        | "KindleCount"
        | "RewardsSacrificed"
        | "RoomsEntered"
        | "SkillsPlayed"
        | "SpoilsActIndex"
        | "StarsSpent"
        | "TimesLifted"
        | "TimesUsed"
        | "TinkerTimeRider"
        | "TinkerTimeType"
        | "TreasureRoomsEntered"
        | "TurnsSeen" => "ints",
        "FurCoatCoordsSet"
        | "GainEnergyInNextCombat"
        | "GaveRelic"
        | "HasItemBeenBought"
        | "HasTriggered"
        | "IsMelted"
        | "IsUsed"
        | "IsWax"
        | "TookDamageThisCombat"
        | "WasUsed" => "bools",
        "Skin" => "strings",
        "CharacterId" | "CharacterModel" | "StarterRelic" | "UpgradedRelic" => "model_ids",
        "StarterCard" => "cards",
        "AncientCard" if matches!(value, PropValue::Card(_)) => "cards",
        "AncientCard" => "model_ids",
        "FurCoatCoordCols" | "FurCoatCoordRows" => "int_arrays",
        "SerializableCards" => "card_arrays",
        _ => {
            return Err(EntryRefusal::CapturePropertyUnknown {
                owner: owner.to_string(),
                name: name.to_string(),
            });
        }
    };
    let fits = match (group, value) {
        ("cards", PropValue::Card(_)) | ("card_arrays", PropValue::Cards(_)) => true,
        ("ints", PropValue::Plain(value)) => value.is_i64() || value.is_u64(),
        ("bools", PropValue::Plain(value)) => value.is_boolean(),
        ("strings" | "model_ids", PropValue::Plain(value)) => value.is_string(),
        ("int_arrays", PropValue::Plain(value)) => value
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item.is_i64() || item.is_u64())),
        _ => false,
    };
    if !fits {
        return Err(EntryRefusal::CapturePropertyType {
            owner: owner.to_string(),
            name: name.to_string(),
            group,
        });
    }
    Ok(group)
}

/// Regroup one decoded property bag into the save's `{group: [{name, value}]}`.
///
/// Rows keep the decoder's order, which is the writer's order within each
/// group: `SavedProperties::Serialize` (RVA 0x3f9b8) enumerates each group's
/// own list, the same list the JSON writer emits (IL_003b for `ints`). A
/// nested card (the `cards` and `card_arrays` groups) is mapped with
/// [`card_json`], so its own ids and properties take the save's spelling too.
fn regroup(owner: &str, props: &OrderedProps) -> Result<Map<String, Value>, EntryRefusal> {
    let mut groups: Map<String, Value> = Map::new();
    for (name, value) in &props.0 {
        let group = property_group(owner, name, value)?;
        let value = match value {
            PropValue::Card(card) => card_json(card)?,
            PropValue::Cards(cards) => {
                Value::Array(cards.iter().map(card_json).collect::<Result<_, _>>()?)
            }
            PropValue::Plain(value) => value.clone(),
        };
        let mut row = Map::new();
        row.insert("name".to_string(), Value::String(name.clone()));
        row.insert("value".to_string(), value);
        groups
            .entry(group.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("every group is a list")
            .push(Value::Object(row));
    }
    Ok(groups)
}

fn optional_props(
    owner: &str,
    props: Option<&OrderedProps>,
) -> Result<Option<Map<String, Value>>, EntryRefusal> {
    props.map(|props| regroup(owner, props)).transpose()
}

fn card(card: &CaptureCard) -> Result<SerializedCard, EntryRefusal> {
    let id = canonical_model_id(Some(&card.id), "CARD")?;
    let enchantment = match &card.enchantment {
        None => None,
        Some(enchantment) => Some(SerializedEnchantment {
            id: canonical_model_id(Some(&enchantment.id), "ENCHANTMENT")?,
            amount: Some(enchantment.level),
            props: optional_props(&id, enchantment.props.as_ref())?,
        }),
    };
    Ok(SerializedCard {
        props: optional_props(&id, card.props.as_ref())?,
        id: Some(id),
        current_upgrade_level: Some(card.upgrade_level),
        enchantment,
        floor_added_to_deck: card.floor_added_to_deck,
    })
}

/// A nested card as the save's JSON writes it.
///
/// A nested card reaches the entry only as a raw property value, which no
/// entry fact reads. The save's own nested cards omit a zero upgrade level
/// and every null, so this does too, and a corpus comparison sees the same
/// value either way.
fn card_json(capture: &CaptureCard) -> Result<Value, EntryRefusal> {
    let mapped = card(capture)?;
    let mut out = Map::new();
    out.insert("id".to_string(), Value::from(mapped.id));
    if let Some(level) = mapped.current_upgrade_level.filter(|level| *level != 0) {
        out.insert("current_upgrade_level".to_string(), Value::from(level));
    }
    if let Some(enchantment) = mapped.enchantment {
        let mut row = Map::new();
        row.insert("id".to_string(), Value::from(enchantment.id));
        row.insert("amount".to_string(), Value::from(enchantment.amount));
        if let Some(props) = enchantment.props {
            row.insert("props".to_string(), Value::Object(props));
        }
        out.insert("enchantment".to_string(), Value::Object(row));
    }
    if let Some(props) = mapped.props {
        out.insert("props".to_string(), Value::Object(props));
    }
    if let Some(floor) = mapped.floor_added_to_deck {
        out.insert("floor_added_to_deck".to_string(), Value::from(floor));
    }
    Ok(Value::Object(out))
}

fn ids(values: &[String], category: &'static str) -> Result<Vec<String>, EntryRefusal> {
    values
        .iter()
        .map(|value| canonical_model_id(Some(value), category))
        .collect()
}

fn optional_id(
    value: Option<&String>,
    category: &'static str,
) -> Result<Option<String>, EntryRefusal> {
    value
        .map(|value| canonical_model_id(Some(value), category))
        .transpose()
}

fn enum_name(
    table: &[(&'static str, &'static str)],
    enumeration: &'static str,
    value: &str,
) -> Result<&'static str, EntryRefusal> {
    table
        .iter()
        .find(|(native, _)| *native == value)
        .map(|(_, snake)| *snake)
        .ok_or_else(|| EntryRefusal::CaptureEnumUnknown {
            enumeration,
            value: value.to_string(),
        })
}

fn coord([col, row]: [i64; 2]) -> Coord {
    Coord { col, row }
}

fn map_point(point: &CaptureMapPoint) -> Result<SerializedMapPoint, EntryRefusal> {
    Ok(SerializedMapPoint {
        coord: Some(coord(point.coord)),
        point_type: Some(
            enum_name(&MAP_POINT_TYPE_NAMES, "MapPointType", &point.point_type)?.to_string(),
        ),
        can_modify: Some(point.can_be_modified),
        children: point.child_coords.iter().copied().map(coord).collect(),
    })
}

fn act(capture: &CaptureAct) -> Result<SerializedAct, EntryRefusal> {
    let rooms = &capture.rooms;
    let saved_map = match &capture.saved_map {
        None => None,
        Some(map) => Some(SerializedActMap {
            points: map.points.iter().map(map_point).collect::<Result<_, _>>()?,
            boss: Some(map_point(&map.boss_point)?),
            second_boss: map.second_boss_point.as_ref().map(map_point).transpose()?,
            start: Some(map_point(&map.starting_point)?),
            start_coords: map
                .start_map_point_coords
                .iter()
                .copied()
                .map(coord)
                .collect(),
            width: Some(map.grid_width),
            height: Some(map.grid_height),
        }),
    };
    Ok(SerializedAct {
        id: Some(canonical_model_id(Some(&capture.id), "ACT")?),
        rooms: SerializedRoomSet {
            event_ids: ids(&rooms.event_ids, "EVENT")?,
            events_visited: rooms.events_visited,
            normal_encounter_ids: ids(&rooms.normal_encounter_ids, "ENCOUNTER")?,
            normal_encounters_visited: rooms.normal_encounters_visited,
            elite_encounter_ids: ids(&rooms.elite_encounter_ids, "ENCOUNTER")?,
            elite_encounters_visited: rooms.elites_visited,
            boss_encounters_visited: rooms.bosses_visited,
            boss_id: optional_id(rooms.boss_id.as_ref(), "ENCOUNTER")?,
            second_boss_id: optional_id(rooms.second_boss_id.as_ref(), "ENCOUNTER")?,
            ancient_id: optional_id(rooms.ancient_id.as_ref(), "EVENT")?,
        },
        saved_map,
    })
}

/// One stream set: the decoder's parallel `counters`/`states` maps, joined.
///
/// v0.109+ writes each stream as `SerializableRng` (counter plus four state
/// words, RVA 0x3d198), so a counter without its state words, or the
/// reverse, is a malformed capture rather than a stream to fill in.
fn streams(
    rng: &CaptureRng,
    names: &[(&'static str, &'static str)],
    enumeration: &'static str,
) -> Result<BTreeMap<String, SerializedRng>, EntryRefusal> {
    let mut out = BTreeMap::new();
    for (native, counter) in &rng.counters {
        let snake = enum_name(names, enumeration, native)?;
        let [s0, s1, s2, s3] =
            *rng.states
                .get(native)
                .ok_or_else(|| EntryRefusal::CaptureRngStateMissing {
                    stream: native.clone(),
                })?;
        let counter = u64::try_from(*counter).map_err(|_| {
            EntryRefusal::UnknownCaptureField(format!("rng counter {native} is negative"))
        })?;
        out.insert(
            snake.to_string(),
            SerializedRng {
                counter,
                s0,
                s1,
                s2,
                s3,
            },
        );
    }
    if let Some(native) = rng
        .states
        .keys()
        .find(|key| !rng.counters.contains_key(*key))
    {
        return Err(EntryRefusal::CaptureRngStateMissing {
            stream: native.clone(),
        });
    }
    Ok(out)
}

fn grab_bag(bag: &BTreeMap<String, Vec<String>>) -> Result<SerializedRelicGrabBag, EntryRefusal> {
    let mut relic_id_lists = BTreeMap::new();
    for (rarity, relics) in bag {
        relic_id_lists.insert(
            enum_name(&RELIC_RARITY_NAMES, "RelicRarity", rarity)?.to_string(),
            ids(relics, "RELIC")?,
        );
    }
    Ok(SerializedRelicGrabBag { relic_id_lists })
}

/// The shortest decimal that round-trips the `float` the writer stored.
///
/// `PacketWriter::WriteFloat` stores an `f32`; the decoder widens it to a
/// double. The JSON save writes the same `float` in its shortest form, so the
/// value is narrowed back to `f32` and re-read at that precision.
fn float(value: f64) -> f64 {
    let narrow = value as f32;
    narrow
        .to_string()
        .parse()
        .expect("an f32's shortest decimal form parses as an f64")
}

fn player(capture: &CapturePlayer) -> Result<SerializedPlayer, EntryRefusal> {
    let player_names: Vec<(&'static str, &'static str)> = PLAYER_RNG_TYPES.to_vec();
    let seed = capture.player_rng.seed.as_u64().ok_or_else(|| {
        EntryRefusal::UnknownCaptureField("player_rng.seed is not an unsigned integer".into())
    })?;
    let net_id = i64::try_from(capture.net_id)
        .map_err(|_| EntryRefusal::UnknownCaptureField("player net_id exceeds i64".into()))?;
    Ok(SerializedPlayer {
        character_id: Some(canonical_model_id(Some(&capture.character), "CHARACTER")?),
        current_hp: Some(capture.current_hp),
        max_hp: Some(capture.max_hp),
        max_energy: Some(capture.max_energy),
        max_potion_slot_count: Some(capture.max_potion_slots),
        gold: Some(capture.gold),
        base_orb_slot_count: Some(capture.base_orb_slots),
        net_id: Some(net_id),
        deck: capture.deck.iter().map(card).collect::<Result<_, _>>()?,
        relics: capture
            .relics
            .iter()
            .map(|relic| {
                let id = canonical_model_id(Some(&relic.id), "RELIC")?;
                Ok(SerializedRelic {
                    props: optional_props(&id, relic.props.as_ref())?,
                    id: Some(id),
                    floor_added_to_deck: relic.floor_added,
                })
            })
            .collect::<Result<_, EntryRefusal>>()?,
        potions: capture
            .potions
            .iter()
            .map(|potion| {
                Ok(SerializedPotion {
                    id: Some(canonical_model_id(Some(&potion.id), "POTION")?),
                    slot_index: Some(potion.slot),
                })
            })
            .collect::<Result<_, EntryRefusal>>()?,
        rng: Some(SerializedPlayerRngSet {
            seed: Some(seed),
            rngs: streams(&capture.player_rng, &player_names, "PlayerRngType")?,
        }),
        odds: Some(SerializedPlayerOdds {
            card_rarity_odds_value: Some(float(capture.odds.card_rarity_odds_value)),
            potion_reward_odds_value: Some(float(capture.odds.potion_reward_odds_value)),
        }),
        relic_grab_bag: Some(grab_bag(&capture.relic_grab_bag)?),
        extra_fields: Some(SerializedExtraPlayerFields {
            card_shop_removals_used: Some(capture.extra_fields.card_shop_removals_used),
            wongo_points: Some(capture.extra_fields.wongo_points),
            cccombo_badge_unlocked: Some(capture.extra_fields.cccombo_badge_unlocked),
            damage_dealt: Some(capture.extra_fields.damage_dealt),
            debuffs_applied: Some(capture.extra_fields.debuffs_applied),
        }),
        unlock_state: Some(SerializedUnlockState {
            unlocked_epochs: Some(capture.unlock_state.unlocked_epochs.clone()),
            encounters_seen: Some(ids(&capture.unlock_state.encounters_seen, "ENCOUNTER")?),
            number_of_runs: Some(capture.unlock_state.number_of_runs),
        }),
        discovered_cards: capture.discovered_cards.clone(),
        discovered_enemies: capture.discovered_enemies.clone(),
        discovered_epochs: capture.discovered_epochs.clone(),
        discovered_potions: capture.discovered_potions.clone(),
        discovered_relics: capture.discovered_relics.clone(),
    })
}

/// Parse a decoded capture run and map it onto the schema-20 save surface.
pub fn parse(text: &str) -> Result<SerializedRun, EntryRefusal> {
    // Schema first, on its own, exactly as `SerializedRun::parse` decides it.
    let probe: SchemaProbe = serde_json::from_str(text)
        .map_err(|error| EntryRefusal::MalformedSaveJson(error.to_string()))?;
    match probe.schema_version {
        Some(ADMITTED_SAVE_SCHEMA) => {}
        Some(other) => return Err(EntryRefusal::UnsupportedSaveSchema(other)),
        None => {
            return Err(EntryRefusal::MissingSaveField {
                owner: "run",
                field: "schema_version",
            });
        }
    }
    // Straight from the text, never through a `Value`: a `Value` object is a
    // sorted map here, and would reorder the property rows (`OrderedProps`).
    let capture: CaptureRun = serde_json::from_str(text)
        .map_err(|error| EntryRefusal::UnknownCaptureField(error.to_string()))?;
    if capture.pre_finished_room.is_some() {
        return Err(EntryRefusal::CaptureFieldUnmapped {
            field: "pre_finished_room",
        });
    }
    let run_names: Vec<(&'static str, &'static str)> = RUN_RNG_TYPE_NAMES
        .iter()
        .copied()
        .zip(RUN_RNG_STREAMS.iter().copied())
        .collect();
    let seed = capture
        .rng
        .seed
        .as_str()
        .ok_or_else(|| EntryRefusal::UnknownCaptureField("rng.seed is not a string".into()))?
        .to_string();
    Ok(SerializedRun {
        schema_version: Some(capture.schema_version),
        acts: capture.acts.iter().map(act).collect::<Result<_, _>>()?,
        modifiers: capture
            .modifiers
            .iter()
            .map(|modifier| {
                let id = canonical_model_id(Some(&modifier.id), "MODIFIER")?;
                Ok(SerializedModifier {
                    props: optional_props(&id, modifier.props.as_ref())?,
                    id: Some(id),
                })
            })
            .collect::<Result<_, EntryRefusal>>()?,
        daily_time: capture.daily_time,
        current_act_index: Some(capture.current_act_index),
        events_seen: ids(&capture.events_seen, "EVENT")?,
        pre_finished_room: None,
        odds: Some(SerializedRunOdds {
            unknown_map_point_monster_odds_value: Some(float(capture.run_odds.card_reward_odds)),
            unknown_map_point_elite_odds_value: Some(float(capture.run_odds.potion_reward_odds)),
            unknown_map_point_treasure_odds_value: Some(float(capture.run_odds.relic_reward_odds)),
            unknown_map_point_shop_odds_value: Some(float(capture.run_odds.gold_reward_odds)),
        }),
        shared_relic_grab_bag: Some(grab_bag(&capture.shared_relic_grab_bag)?),
        players: Some(
            capture
                .players
                .iter()
                .map(player)
                .collect::<Result<_, _>>()?,
        ),
        rng: Some(SerializedRunRngSet {
            seed,
            rngs: Some(streams(&capture.rng, &run_names, "RunRngType")?),
        }),
        visited_map_coords: capture
            .visited_map_coords
            .iter()
            .copied()
            .map(coord)
            .collect(),
        map_point_history: Some(capture.map_point_history),
        save_time: Some(capture.save_time),
        start_time: Some(capture.start_time),
        run_time: Some(capture.run_time),
        win_time: Some(capture.win_time),
        ascension: Some(capture.ascension),
        num_reloads: Some(capture.num_reloads),
        // Not a `SerializableRun` field the binary writer carries.
        platform_type: None,
        map_drawings: capture.map_drawings,
        extra_fields: Some(SerializedExtraRunFields {
            started_with_neow: Some(capture.extra_fields.started_with_neow),
            test_subject_kills: Some(capture.extra_fields.test_subject_kills),
            freed_repy: Some(capture.extra_fields.freed_repy),
        }),
        game_mode: Some(enum_name(&GAME_MODE_NAMES, "GameMode", &capture.game_mode)?.to_string()),
    })
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
