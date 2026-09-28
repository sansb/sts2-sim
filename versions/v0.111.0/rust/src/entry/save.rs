//! The schema-20 run save, parsed into typed Rust with
//! `#[serde(deny_unknown_fields)]` everywhere.
//!
//! # Native authority
//!
//! Every struct below mirrors one save-writer type in v0.111.0 `sts2.dll`
//! (sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
//! `release_info.json` commit `41cef1ea`), read with the TypeDef/FieldList walk
//! `tools/dump_type.py` performs — the property backing fields name the JSON
//! keys one-to-one except where a converter renames them, which is called out
//! per field below:
//!
//! | Rust | IL type (`MegaCrit.Sts2.Core.…`) | fields |
//! |---|---|---|
//! | [`SerializedRun`] | `Saves.SerializableRun` | 23 |
//! | [`SerializedPlayer`] | `Saves.Runs.SerializablePlayer` | 21 |
//! | [`SerializedCard`] | `Saves.Runs.SerializableCard` | 5 (**no `Owner`**) |
//! | [`SerializedRelic`] | `Saves.Runs.SerializableRelic` | 3 |
//! | [`SerializedPotion`] | `Saves.Runs.SerializablePotion` | 2 |
//! | [`SerializedEnchantment`] | `Saves.Runs.SerializableEnchantment` | 3 |
//! | [`SerializedRng`] | `Saves.SerializableRng` | 5 (`counter`, `state0..3`) |
//! | [`SerializedRunRngSet`] | `Saves.Runs.SerializableRunRngSet` | `Seed`, `Rngs` |
//! | [`SerializedPlayerRngSet`] | `Saves.SerializablePlayerRngSet` | `Seed`, `Rngs` |
//! | [`SerializedUnlockState`] | `Unlocks.SerializableUnlockState` | 3 |
//! | [`SerializedAct`] | `Saves.Runs.SerializableActModel` | 3 |
//! | [`SerializedRoomSet`] | `Saves.Runs.SerializableRoomSet` | 10 |
//! | [`SerializedActMap`] | `Saves.Runs.SerializableActMap` | 7 |
//! | [`SerializedMapPoint`] | `Saves.Runs.SerializableMapPoint` | 4 |
//! | [`SerializedRoom`] | `Saves.Runs.SerializableRoom` | 9 |
//! | [`SerializedRelicGrabBag`] | `Saves.Runs.SerializableRelicGrabBag` | 1 |
//! | [`SerializedRunOdds`] | `Saves.Runs.SerializableRunOddsSet` | 4 |
//! | [`SerializedPlayerOdds`] | `Saves.Runs.SerializablePlayerOddsSet` | 2 |
//! | [`SerializedExtraRunFields`] | `Saves.Runs.SerializableExtraRunFields` | 3 |
//! | [`SerializedExtraPlayerFields`] | `Saves.SerializableExtraPlayerFields` | 5 |
//!
//! The run's stream keys are `Entities.Rngs.RunRngType` (12 members) and the
//! player's are `Entities.Rngs.PlayerRngType` (3), both reaching JSON through
//! `SnakeCaseJsonStringEnumConverter`. Map point kinds are `Map.MapPointType`
//! (9 members).
//!
//! # Why absence is not an error, and unknown keys are
//!
//! `Saves.Runs.SerializationCondition` gives each property one of
//! `AlwaysSave`, `SaveIfNotPropertyDefault`, `SaveIfNotTypeDefault`,
//! `SaveIfNotCollectionEmptyOrNull`, so a field sitting at its default is
//! legitimately **absent** from the file. Every field here is therefore
//! defaulted or optional, and a field the *builder reads* is refused by name
//! at its read site rather than at parse time.
//!
//! The opposite direction is the I8 guard the invariant walk asks for: an
//! untaught key is a parse error (`deny_unknown_fields`), never a silently
//! dropped fact. That is the failure mode I8 was written for — the parser that
//! dropped `enchantment`.
//!
//! # Fields recorded but deliberately not read
//!
//! `map_point_history` is read only for its per-act **lengths** (the global
//! node index); its rows are free-form per-room statistics with hundreds of
//! ad-hoc keys and localisation tables, so they stay [`Value`]. `map_drawings`
//! reaches JSON through `MapDrawing.SerializableMapDrawingsJsonConverter` as an
//! opaque encoded string. `extra_rewards` and `encounter_state` inside
//! `pre_finished_room`, and the grab-bag rarity lists, are recorded run state
//! that no combat entry fact depends on. Each is typed as loosely as it is
//! read and no more.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::entry::refusal::EntryRefusal;

/// The only admitted save schema. Schema 19 stores the same per-stream xoshiro
/// state, but `Migrations.SerializableRuns.SerializableRunV19ToV20` is what
/// defines 20's field surface, and the surface is what
/// `deny_unknown_fields` pins.
pub const ADMITTED_SAVE_SCHEMA: u32 = 20;

/// `MegaCrit.Sts2.Core.Map.MapPointType`, snake_cased by the serializer.
pub const MAP_POINT_TYPES: [&str; 9] = [
    "unassigned",
    "unknown",
    "shop",
    "treasure",
    "rest_site",
    "monster",
    "elite",
    "boss",
    "ancient",
];

/// `MegaCrit.Sts2.Core.Entities.Rngs.RunRngType`, snake_cased by the
/// serializer, in IL declaration order.
pub const RUN_RNG_STREAMS: [&str; 12] = [
    "up_front",
    "shuffle",
    "unknown_map_point",
    "combat_card_generation",
    "combat_potion_generation",
    "combat_card_selection",
    "combat_energy_costs",
    "combat_targets",
    "monster_ai",
    "niche",
    "combat_orbs",
    "treasure_room_relics",
];

/// One `(counter, s0..s3)` xoshiro256** stream, exactly as the save records it.
///
/// `Saves.SerializableRng` is a C# record with fields `counter`,
/// `state0`..`state3`; the serializer writes the states as `s0`..`s3`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRng {
    pub counter: u64,
    pub s0: u64,
    pub s1: u64,
    pub s2: u64,
    pub s3: u64,
}

/// `Saves.Runs.SerializableRunRngSet`: the run seed plus every run stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRunRngSet {
    pub seed: String,
    /// `Option` rather than a defaulted map on purpose. The oracle's
    /// `relics_entering_dispatch_ordered` is literally
    /// `"rngs" in (save.get("rng") or {})` — **key presence**, its proxy for
    /// schema >= 19 — and `stream_counters` refuses a block carrying neither
    /// `counters` nor `rngs`. A defaulted map cannot tell an absent key from an
    /// empty one, so both of those questions would be answered on a guess.
    #[serde(default)]
    pub rngs: Option<BTreeMap<String, SerializedRng>>,
}

/// `Saves.SerializablePlayerRngSet`: the per-player derived seed and streams.
///
/// Not projected into combat state — `rewards`, `shops` and `transformations`
/// are run state — but parsed so `deny_unknown_fields` still covers it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedPlayerRngSet {
    /// The player's derived seed is a 64-bit hash, not a signed counter: the
    /// corpus spans 1009204982354447093 .. 18358128862073348558, which is
    /// outside `i64`. (The *run* seed is the player-visible seed string.)
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub rngs: BTreeMap<String, SerializedRng>,
}

/// `Saves.Runs.SerializableEnchantment`.
///
/// The strength reaches JSON as `amount`. `level` — the historical `.run`
/// spelling — is deliberately *not* accepted: the save writer has no such
/// field, so a key spelled that way is an untaught fact, not a synonym.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedEnchantment {
    pub id: String,
    #[serde(default)]
    pub amount: Option<i64>,
    #[serde(default)]
    pub props: Option<Map<String, Value>>,
}

/// `Saves.Runs.SerializableCard`.
///
/// **There is no `Owner` field.** The IL walk over the TypeDef's FieldList
/// returns exactly `Id`, `CurrentUpgradeLevel`, `Enchantment`, `Props`,
/// `FloorAddedToDeck`. Card *owner* provenance is therefore the one entry fact
/// a schema-20 save cannot answer, and no amount of parsing recovers it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedCard {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub current_upgrade_level: Option<i64>,
    #[serde(default)]
    pub enchantment: Option<SerializedEnchantment>,
    /// Raw, so the entry document can hand the oracle's `start_combat` the
    /// same bytes the save carried. Validated by `entry/props.rs` at read.
    #[serde(default)]
    pub props: Option<Map<String, Value>>,
    #[serde(default)]
    pub floor_added_to_deck: Option<i64>,
}

/// `Saves.Runs.SerializableRelic`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRelic {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub props: Option<Map<String, Value>>,
    #[serde(default)]
    pub floor_added_to_deck: Option<i64>,
}

/// `Saves.Runs.SerializablePotion`.
///
/// `slot_index` is what closes the potion-belt ambiguity class: the belt slot
/// a potion occupies is recorded, not inferred.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedPotion {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub slot_index: Option<i64>,
}

/// `Unlocks.SerializableUnlockState`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedUnlockState {
    #[serde(default)]
    pub unlocked_epochs: Option<Vec<String>>,
    #[serde(default)]
    pub encounters_seen: Option<Vec<String>>,
    #[serde(default)]
    pub number_of_runs: Option<i64>,
}

/// `Saves.Runs.SerializableRelicGrabBag`: rarity → remaining relic ids.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRelicGrabBag {
    #[serde(default)]
    pub relic_id_lists: BTreeMap<String, Vec<String>>,
}

/// `Saves.Runs.SerializablePlayerOddsSet`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedPlayerOdds {
    #[serde(default)]
    pub card_rarity_odds_value: Option<f64>,
    #[serde(default)]
    pub potion_reward_odds_value: Option<f64>,
}

/// `Saves.Runs.SerializableRunOddsSet`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRunOdds {
    #[serde(default)]
    pub unknown_map_point_monster_odds_value: Option<f64>,
    #[serde(default)]
    pub unknown_map_point_elite_odds_value: Option<f64>,
    #[serde(default)]
    pub unknown_map_point_treasure_odds_value: Option<f64>,
    #[serde(default)]
    pub unknown_map_point_shop_odds_value: Option<f64>,
}

/// `Saves.Runs.SerializableExtraRunFields`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedExtraRunFields {
    #[serde(default)]
    pub started_with_neow: Option<bool>,
    #[serde(default)]
    pub test_subject_kills: Option<i64>,
    #[serde(default)]
    pub freed_repy: Option<bool>,
}

/// `Saves.SerializableExtraPlayerFields`.
///
/// One field here is why the taught surface was measured against the corpus
/// instead of snake-cased off the property names. The IL property is
/// `CccomboBadgeUnlocked` (three `c`s, backing field
/// `<CccomboBadgeUnlocked>k__BackingField`, accessors
/// `get_`/`set_CccomboBadgeUnlocked`), but the assembly also carries the
/// string literal **`ccccombo_badge_unlocked`** — four `c`s — which is the
/// name an explicit JSON property attribute puts on the wire, and the name 50
/// corpus saves actually use. Snake-casing the property would have produced a
/// key no save has.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedExtraPlayerFields {
    #[serde(default)]
    pub card_shop_removals_used: Option<i64>,
    #[serde(default)]
    pub wongo_points: Option<i64>,
    #[serde(default, rename = "ccccombo_badge_unlocked")]
    pub cccombo_badge_unlocked: Option<bool>,
    #[serde(default)]
    pub damage_dealt: Option<i64>,
    #[serde(default)]
    pub debuffs_applied: Option<i64>,
}

/// A map grid coordinate.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Coord {
    pub col: i64,
    pub row: i64,
}

/// `Saves.Runs.SerializableMapPoint`.
///
/// `PointType` reaches JSON as `type`, `CanBeModified` as `can_modify`, and
/// `ChildCoords` as `children`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedMapPoint {
    #[serde(default)]
    pub coord: Option<Coord>,
    #[serde(rename = "type", default)]
    pub point_type: Option<String>,
    #[serde(default)]
    pub can_modify: Option<bool>,
    #[serde(default)]
    pub children: Vec<Coord>,
}

/// `Saves.Runs.SerializableActMap`.
///
/// `Points` → `points`, `BossPoint` → `boss`, `SecondBossPoint` →
/// `second_boss`, `StartingPoint` → `start`, `StartMapPointCoords` →
/// `start_coords`, `GridWidth`/`GridHeight` → `width`/`height`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedActMap {
    #[serde(default)]
    pub points: Vec<SerializedMapPoint>,
    #[serde(default)]
    pub boss: Option<SerializedMapPoint>,
    #[serde(default)]
    pub second_boss: Option<SerializedMapPoint>,
    #[serde(default)]
    pub start: Option<SerializedMapPoint>,
    #[serde(default)]
    pub start_coords: Vec<Coord>,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
}

/// `Saves.Runs.SerializableRoomSet`: an act's encounter/event schedule.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRoomSet {
    #[serde(default)]
    pub event_ids: Vec<String>,
    #[serde(default)]
    pub events_visited: usize,
    #[serde(default)]
    pub normal_encounter_ids: Vec<String>,
    #[serde(default)]
    pub normal_encounters_visited: usize,
    #[serde(default)]
    pub elite_encounter_ids: Vec<String>,
    #[serde(default)]
    pub elite_encounters_visited: usize,
    #[serde(default)]
    pub boss_encounters_visited: usize,
    #[serde(default)]
    pub boss_id: Option<String>,
    #[serde(default)]
    pub second_boss_id: Option<String>,
    #[serde(default)]
    pub ancient_id: Option<String>,
}

/// `Saves.Runs.SerializableActModel`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedAct {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub rooms: SerializedRoomSet,
    #[serde(default)]
    pub saved_map: Option<SerializedActMap>,
}

/// `Saves.Runs.SerializableRoom`: the pre-finished room record.
///
/// `GoldProportion` reaches JSON as `reward_proportion`. `extra_rewards`
/// (`SerializableReward`) and `encounter_state` are recorded run state this
/// builder does not read.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRoom {
    #[serde(default)]
    pub room_type: Option<String>,
    #[serde(default)]
    pub encounter_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub is_pre_finished: Option<bool>,
    #[serde(default)]
    pub reward_proportion: Option<i64>,
    #[serde(default)]
    pub extra_rewards: Option<Value>,
    #[serde(default)]
    pub parent_event_id: Option<String>,
    #[serde(default)]
    pub should_resume_parent_event: Option<bool>,
    #[serde(default)]
    pub encounter_state: Option<Value>,
}

/// `Saves.Runs.SerializableModifier`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedModifier {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub props: Option<Map<String, Value>>,
}

/// `Saves.Runs.SerializablePlayer`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedPlayer {
    #[serde(default)]
    pub character_id: Option<String>,
    #[serde(default)]
    pub current_hp: Option<i64>,
    #[serde(default)]
    pub max_hp: Option<i64>,
    #[serde(default)]
    pub max_energy: Option<i64>,
    #[serde(default)]
    pub max_potion_slot_count: Option<i64>,
    #[serde(default)]
    pub gold: Option<i64>,
    #[serde(default)]
    pub base_orb_slot_count: Option<i64>,
    #[serde(default)]
    pub net_id: Option<i64>,
    #[serde(default)]
    pub deck: Vec<SerializedCard>,
    #[serde(default)]
    pub relics: Vec<SerializedRelic>,
    #[serde(default)]
    pub potions: Vec<SerializedPotion>,
    #[serde(default)]
    pub rng: Option<SerializedPlayerRngSet>,
    #[serde(default)]
    pub odds: Option<SerializedPlayerOdds>,
    #[serde(default)]
    pub relic_grab_bag: Option<SerializedRelicGrabBag>,
    #[serde(default)]
    pub extra_fields: Option<SerializedExtraPlayerFields>,
    #[serde(default)]
    pub unlock_state: Option<SerializedUnlockState>,
    #[serde(default)]
    pub discovered_cards: Vec<String>,
    #[serde(default)]
    pub discovered_enemies: Vec<String>,
    #[serde(default)]
    pub discovered_epochs: Vec<String>,
    #[serde(default)]
    pub discovered_potions: Vec<String>,
    #[serde(default)]
    pub discovered_relics: Vec<String>,
}

/// `Saves.SerializableRun`: the whole schema-20 run save.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SerializedRun {
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub acts: Vec<SerializedAct>,
    #[serde(default)]
    pub modifiers: Vec<SerializedModifier>,
    #[serde(default)]
    pub daily_time: Option<i64>,
    #[serde(default)]
    pub current_act_index: Option<usize>,
    #[serde(default)]
    pub events_seen: Vec<String>,
    #[serde(default)]
    pub pre_finished_room: Option<SerializedRoom>,
    #[serde(default)]
    pub odds: Option<SerializedRunOdds>,
    #[serde(default)]
    pub shared_relic_grab_bag: Option<SerializedRelicGrabBag>,
    #[serde(default)]
    pub players: Option<Vec<SerializedPlayer>>,
    #[serde(default)]
    pub rng: Option<SerializedRunRngSet>,
    #[serde(default)]
    pub visited_map_coords: Vec<Coord>,
    /// Read only for `len()` per act; see the module note.
    #[serde(default)]
    pub map_point_history: Option<Vec<Value>>,
    #[serde(default)]
    pub save_time: Option<i64>,
    #[serde(default)]
    pub start_time: Option<i64>,
    #[serde(default)]
    pub run_time: Option<i64>,
    #[serde(default)]
    pub win_time: Option<i64>,
    #[serde(default)]
    pub ascension: Option<i64>,
    #[serde(default)]
    pub num_reloads: Option<i64>,
    #[serde(default)]
    pub platform_type: Option<String>,
    /// Converter-encoded blob; see the module note.
    #[serde(default)]
    pub map_drawings: Option<Value>,
    #[serde(default)]
    pub extra_fields: Option<SerializedExtraRunFields>,
    #[serde(default)]
    pub game_mode: Option<String>,
}

/// Just enough of a save to decide whether its schema is admitted.
///
/// Parsed first, and on its own, because the field surface
/// `deny_unknown_fields` pins is schema 20's. Deciding admission from a full
/// parse would report a schema-16 save as an untaught key rather than as an
/// unadmitted schema — and I11 requires the build/schema decision to precede
/// reading any field.
#[derive(Debug, Deserialize)]
struct SchemaProbe {
    #[serde(default)]
    schema_version: Option<u32>,
}

impl SerializedRun {
    /// Parse one save's bytes, schema first.
    pub fn parse(text: &str) -> Result<Self, EntryRefusal> {
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
        serde_json::from_str(text)
            .map_err(|error| EntryRefusal::UnknownSaveField(error.to_string()))
    }

    /// The sole player, or the multiplayer refusal.
    ///
    /// Mirrors `relay_parser.require_single_player`: every raw run/save adapter
    /// must refuse rather than silently select `players[0]`, because one
    /// `State` cannot represent another player's piles, powers, potion targets
    /// or RNG ownership.
    pub fn single_player(&self) -> Result<&SerializedPlayer, EntryRefusal> {
        match self.players.as_deref() {
            Some([player]) => Ok(player),
            Some(players) => Err(EntryRefusal::MultiplayerSave(players.len())),
            None => Err(EntryRefusal::MissingSaveField {
                owner: "run",
                field: "players",
            }),
        }
    }

    /// The run RNG block, or the refusal naming it.
    pub fn run_rng(&self) -> Result<&SerializedRunRngSet, EntryRefusal> {
        self.rng.as_ref().ok_or(EntryRefusal::MissingSaveField {
            owner: "run",
            field: "rng",
        })
    }

    /// `current_act_index`, defaulting to act 0 exactly as the oracle's
    /// `current_node` does with `save.get("current_act_index") or 0`.
    pub fn act_index(&self) -> usize {
        self.current_act_index.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "schema_version": 20,
        "current_act_index": 0,
        "visited_map_coords": [{"col": 3, "row": 0}],
        "rng": {"seed": "ZPJHU3WSH2", "rngs": {}},
        "players": [{"character_id": "CHARACTER.IRONCLAD", "current_hp": 75,
                     "max_hp": 75, "gold": 99, "max_potion_slot_count": 2,
                     "deck": [], "relics": [], "potions": []}]
    }"#;

    #[test]
    fn a_minimal_schema_20_save_parses() {
        let run = SerializedRun::parse(MINIMAL).unwrap();
        assert_eq!(run.schema_version, Some(20));
        assert_eq!(run.single_player().unwrap().gold, Some(99));
        assert_eq!(run.run_rng().unwrap().seed, "ZPJHU3WSH2");
    }

    #[test]
    fn an_untaught_key_refuses_rather_than_dropping_the_fact() {
        // The I8 guard: this is the failure mode that dropped `enchantment`.
        let text = MINIMAL.replace(r#""max_hp": 75,"#, r#""max_hp": 75, "soul_power": 3,"#);
        let refusal = SerializedRun::parse(&text).unwrap_err();
        assert_eq!(refusal.class(), "unknown_save_field");
        assert!(refusal.to_string().contains("soul_power"));
    }

    #[test]
    fn an_older_schema_refuses_as_a_schema_not_as_an_untaught_key() {
        let text = MINIMAL.replace(r#""schema_version": 20"#, r#""schema_version": 16"#);
        assert_eq!(
            SerializedRun::parse(&text),
            Err(EntryRefusal::UnsupportedSaveSchema(16))
        );
    }

    #[test]
    fn multiplayer_refuses_by_count() {
        let text = MINIMAL.replace(
            r#""potions": []}]"#,
            r#""potions": []}, {"character_id": "CHARACTER.SILENT"}]"#,
        );
        assert_eq!(
            SerializedRun::parse(&text).unwrap().single_player(),
            Err(EntryRefusal::MultiplayerSave(2))
        );
    }

    #[test]
    fn the_stream_vocabularies_match_the_il_member_counts() {
        assert_eq!(RUN_RNG_STREAMS.len(), 12);
        assert_eq!(MAP_POINT_TYPES.len(), 9);
        let mut sorted: Vec<&str> = RUN_RNG_STREAMS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), RUN_RNG_STREAMS.len());
    }

    #[test]
    fn the_combo_badge_key_is_the_assemblys_literal_not_a_snake_cased_property() {
        // The IL property is CccomboBadgeUnlocked; the wire name in the DLL's
        // string heap — and in 50 corpus saves — is ccccombo_badge_unlocked.
        let text = MINIMAL.replace(
            r#""max_hp": 75,"#,
            r#""max_hp": 75, "extra_fields": {"ccccombo_badge_unlocked": true},"#,
        );
        let run = SerializedRun::parse(&text).unwrap();
        assert_eq!(
            run.single_player().unwrap().extra_fields,
            Some(SerializedExtraPlayerFields {
                cccombo_badge_unlocked: Some(true),
                ..SerializedExtraPlayerFields::default()
            })
        );
        let wrong = MINIMAL.replace(
            r#""max_hp": 75,"#,
            r#""max_hp": 75, "extra_fields": {"cccombo_badge_unlocked": true},"#,
        );
        assert_eq!(
            SerializedRun::parse(&wrong).unwrap_err().class(),
            "unknown_save_field"
        );
    }

    #[test]
    fn an_enchantment_level_key_is_untaught_not_a_synonym() {
        let text = MINIMAL.replace(
            r#""deck": [],"#,
            r#""deck": [{"id": "CARD.BASH", "enchantment": {"id": "ENCHANTMENT.SHARP", "level": 3}}],"#,
        );
        assert_eq!(
            SerializedRun::parse(&text).unwrap_err().class(),
            "unknown_save_field"
        );
    }
}
