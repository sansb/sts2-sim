//! Decoder for the game's combat replay file (`replays/latest.mcr`), the
//! recording of one fight's inputs that the companion mod uploads (#3578).
//!
//! The game writes one `CombatReplay` per combat through a bit-packing
//! serializer (`MegaCrit.Sts2.Core.Multiplayer.Serialization.PacketWriter`):
//! values go LSB-first into a little-endian bitstream. This is a port of the
//! Python decoder (`sim/v0.111.0/python/mcr_parser.py`), whose schema is a
//! transcription of each type's `Serialize`/`Deserialize` IL in `sts2.dll`;
//! `MCR_FORMAT.md` beside it is the format record, and the RVAs cited below
//! are the ones that file and the Python comments carry. No serializer was
//! re-read for this port: it changes no claim about the format, and its
//! acceptance bar is the same decoded document as the Python decoder on every
//! v0.111.0 replay (`tools/mcr_parity.py`, and the digests pinned in
//! `tests/mcr_decode.rs`).
//!
//! **One build.** Model ids are stored as compact per-build "net ids", so a
//! replay decodes only against the tables of the build that wrote it
//! (`data/mcr_tables.v0.111.0.json`, a byte copy of the Python tables). A file
//! whose `modelIdHash` is not that build's is refused by name, never guessed
//! at. The pre-v0.109 schema branches of the Python decoder are not carried.
//!
//! The output is the Python decoder's dictionary as JSON: same keys, same
//! values. Two Python behaviours are kept on purpose because consumers were
//! written against them: a negative count in a plain (non-`list_of`) list
//! reads as an empty list, and an unknown enum value reads as `"Name?N"`.

use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const TABLES_JSON: &str = include_str!("../data/mcr_tables.v0.111.0.json");

/// A replay that cannot be decoded, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McrError {
    /// `unsupported_replay_build`, `truncated_replay` or `malformed_replay`.
    pub code: &'static str,
    pub detail: String,
}

impl std::fmt::Display for McrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

type Res<T> = Result<T, McrError>;

fn malformed<T>(detail: impl Into<String>) -> Res<T> {
    Err(McrError {
        code: "malformed_replay",
        detail: detail.into(),
    })
}

struct EnumTable {
    bits: u32,
    values: BTreeMap<i64, String>,
}

struct Tables {
    game_version: String,
    game_commit: String,
    model_id_hash: u32,
    categories: Vec<String>,
    category_bits: u32,
    entries: Vec<String>,
    entry_bits: u32,
    epochs: Vec<String>,
    epoch_bits: u32,
    property_names: Vec<String>,
    property_bits: u32,
    enums: BTreeMap<String, EnumTable>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let raw: Value = serde_json::from_str(TABLES_JSON).expect("mcr tables are JSON");
        let strings = |key: &str| -> Vec<String> {
            raw[key]
                .as_array()
                .expect("mcr table list")
                .iter()
                .map(|v| v.as_str().expect("mcr table name").to_owned())
                .collect()
        };
        let bits = |key: &str| raw[key].as_u64().expect("mcr table width") as u32;
        let enums = raw["enums"]
            .as_object()
            .expect("mcr enums")
            .iter()
            .map(|(name, spec)| {
                let values = spec["values"]
                    .as_object()
                    .expect("mcr enum values")
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.parse::<i64>().expect("mcr enum key"),
                            v.as_str().expect("mcr enum name").to_owned(),
                        )
                    })
                    .collect();
                (
                    name.clone(),
                    EnumTable {
                        bits: spec["bits"].as_u64().expect("mcr enum width") as u32,
                        values,
                    },
                )
            })
            .collect();
        Tables {
            game_version: raw["game_version"].as_str().unwrap_or("").to_owned(),
            game_commit: raw["game_commit"].as_str().unwrap_or("").to_owned(),
            model_id_hash: raw["model_id_hash"].as_u64().expect("mcr model id hash") as u32,
            categories: strings("categories"),
            category_bits: bits("category_bits"),
            entries: strings("entries"),
            entry_bits: bits("entry_bits"),
            epochs: strings("epochs"),
            epoch_bits: bits("epoch_bits"),
            property_names: strings("property_names"),
            property_bits: bits("property_bits"),
            enums,
        }
    })
}

/// The build these tables decode, e.g. `v0.111.0`.
#[must_use]
pub fn tables_game_version() -> &'static str {
    &tables().game_version
}

/// `NetTypeCache` assigns byte ids to `INetAction` implementations sorted by
/// class name (ordinal); `INetActionSubtypes` lists exactly these eleven.
const NET_ACTION_TYPES: [&str; 11] = [
    "NetConsoleCmdGameAction",
    "NetDiscardPotionGameAction",
    "NetEndPlayerTurnAction",
    "NetMoveToMapCoordAction",
    "NetPickRelicAction",
    "NetPlayCardAction",
    "NetReadyToBeginEnemyTurnAction",
    "NetUndoEndPlayerTurnAction",
    "NetUsePotionAction",
    "NetVoteForMapCoordAction",
    "NetVoteToMoveToNextActAction",
];

/// LSB-first bitstream over bytes (`PacketReader` semantics).
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    fn read_bits(&mut self, n: u32) -> Res<u64> {
        let n = n as usize;
        if n > self.bits_left() {
            return Err(McrError {
                code: "truncated_replay",
                detail: format!(
                    "read past end: want {n} bits at bit {}, {} left",
                    self.pos,
                    self.bits_left()
                ),
            });
        }
        let mut value = 0u64;
        let mut p = self.pos;
        for i in 0..n {
            value |= u64::from((self.data[p >> 3] >> (p & 7)) & 1) << i;
            p += 1;
        }
        self.pos = p;
        Ok(value)
    }

    fn u(&mut self, bits: u32) -> Res<u64> {
        self.read_bits(bits)
    }

    /// `PacketReader.ReadInt`: the value goes through an Int32 round trip, so
    /// only a full 32-bit read can be negative.
    fn i(&mut self, bits: u32) -> Res<i64> {
        let v = self.read_bits(bits)?;
        Ok(if bits == 32 && v >= 1 << 31 {
            v as i64 - (1i64 << 32)
        } else {
            v as i64
        })
    }

    fn i64(&mut self) -> Res<i64> {
        Ok(self.read_bits(64)? as i64)
    }

    fn boolean(&mut self) -> Res<bool> {
        Ok(self.read_bits(1)? == 1)
    }

    fn f32(&mut self) -> Res<Value> {
        let value = f32::from_bits(self.read_bits(32)? as u32);
        match Number::from_f64(f64::from(value)) {
            Some(number) => Ok(Value::Number(number)),
            None => malformed(format!("non-finite float at bit {}", self.pos)),
        }
    }

    fn string(&mut self) -> Res<String> {
        let n = self.i(32)?;
        if n < 0 || n as usize > self.bits_left() / 8 {
            return malformed(format!("bad string length {n} at bit {}", self.pos));
        }
        let mut bytes = Vec::with_capacity(n as usize);
        for _ in 0..n {
            bytes.push(self.read_bits(8)? as u8);
        }
        String::from_utf8(bytes).or_else(|_| malformed("a string is not UTF-8"))
    }
}

struct Decoder<'a> {
    r: BitReader<'a>,
    t: &'static Tables,
}

fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

impl Decoder<'_> {
    // --- primitives -------------------------------------------------------
    fn enum_raw(&mut self, name: &str) -> Res<(i64, String)> {
        let table = &self.t.enums[name];
        let v = self.r.i(table.bits)?;
        let label = table
            .values
            .get(&v)
            .cloned()
            .unwrap_or_else(|| format!("{name}?{v}"));
        Ok((v, label))
    }

    fn enum_name(&mut self, name: &str) -> Res<Value> {
        Ok(Value::String(self.enum_raw(name)?.1))
    }

    fn lookup(list: &'static [String], index: i64, what: &str) -> Res<&'static str> {
        usize::try_from(index)
            .ok()
            .and_then(|i| list.get(i))
            .map(String::as_str)
            .ok_or_else(|| McrError {
                code: "malformed_replay",
                detail: format!("{what} net id {index} is outside this build's table"),
            })
    }

    fn model_entry_str(&mut self) -> Res<&'static str> {
        let index = self.r.i(self.t.entry_bits)?;
        Self::lookup(&self.t.entries, index, "model entry")
    }

    fn model_entry(&mut self) -> Res<Value> {
        Ok(Value::String(self.model_entry_str()?.to_owned()))
    }

    fn full_model_id(&mut self) -> Res<Value> {
        let cat = self.r.i(self.t.category_bits)?;
        let cat = Self::lookup(&self.t.categories, cat, "category")?;
        let ent = self.model_entry_str()?;
        Ok(Value::String(format!("{cat}.{ent}")))
    }

    fn epoch_id(&mut self) -> Res<Value> {
        let index = self.r.i(self.t.epoch_bits)?;
        Ok(Value::String(
            Self::lookup(&self.t.epochs, index, "epoch")?.to_owned(),
        ))
    }

    fn property_name(&mut self) -> Res<String> {
        let index = self.r.i(self.t.property_bits)?;
        Ok(Self::lookup(&self.t.property_names, index, "property name")?.to_owned())
    }

    /// A plain Python `[f() for _ in range(n)]`: a negative count is an empty
    /// list, and an absurd one runs until the stream ends.
    fn repeat(&mut self, count: i64, mut f: impl FnMut(&mut Self) -> Res<Value>) -> Res<Value> {
        let mut out = Vec::new();
        for _ in 0..count.max(0) {
            out.push(f(self)?);
        }
        Ok(Value::Array(out))
    }

    fn repeat32(&mut self, f: impl FnMut(&mut Self) -> Res<Value>) -> Res<Value> {
        let count = self.r.i(32)?;
        self.repeat(count, f)
    }

    /// The Python `list_of`: the count is checked before anything is read.
    fn list_of(&mut self, bits: u32, f: impl FnMut(&mut Self) -> Res<Value>) -> Res<Value> {
        let n = self.r.i(bits)?;
        if !(0..=1_000_000).contains(&n) {
            return malformed(format!("implausible list count {n} at bit {}", self.r.pos));
        }
        self.repeat(n, f)
    }

    fn list(&mut self, f: impl FnMut(&mut Self) -> Res<Value>) -> Res<Value> {
        self.list_of(32, f)
    }

    fn model_entry_list(&mut self) -> Res<Value> {
        self.repeat32(Self::model_entry)
    }

    fn full_model_id_list(&mut self) -> Res<Value> {
        self.repeat32(Self::full_model_id)
    }

    fn optional(&mut self, f: impl FnOnce(&mut Self) -> Res<Value>) -> Res<Value> {
        if self.r.boolean()? {
            f(self)
        } else {
            Ok(Value::Null)
        }
    }

    fn int(&mut self, bits: u32) -> Res<Value> {
        Ok(Value::from(self.r.i(bits)?))
    }

    fn uint(&mut self, bits: u32) -> Res<Value> {
        Ok(Value::from(self.r.u(bits)?))
    }

    fn boolean(&mut self) -> Res<Value> {
        Ok(Value::Bool(self.r.boolean()?))
    }

    fn string(&mut self) -> Res<Value> {
        Ok(Value::String(self.r.string()?))
    }

    // --- top level ----------------------------------------------------------
    fn combat_replay(&mut self) -> Res<Value> {
        let version = self.r.string()?;
        let git_commit = self.r.string()?;
        let model_id_hash = self.r.u(32)? as u32;
        if model_id_hash != self.t.model_id_hash {
            return Err(McrError {
                code: "unsupported_replay_build",
                detail: format!(
                    "modelIdHash mismatch: file was written by {version} ({git_commit}) with \
                     0x{model_id_hash:08x}, tables are for {} ({}) with 0x{:08x}",
                    self.t.game_version, self.t.game_commit, self.t.model_id_hash
                ),
            });
        }
        let choice_ids = self.repeat32(|d| d.uint(32))?;
        let reward_ids = self.repeat32(|d| d.int(32))?;
        let next_action_id = self.uint(32)?;
        let next_checksum_id = self.uint(32)?;
        let next_hook_id = self.uint(32)?;
        let run = self.serializable_run()?;
        let events = self.list(Self::replay_event)?;
        let checksums = self.list(Self::checksum_data)?;
        // The writer pads the final byte with zero bits.
        let left = self.r.bits_left();
        if left >= 8 || (left > 0 && self.r.read_bits(left as u32)? != 0) {
            return malformed(format!("{left} unconsumed bits after decode"));
        }
        Ok(obj(vec![
            ("version", Value::String(version)),
            ("git_commit", Value::String(git_commit)),
            ("model_id_hash", Value::from(model_id_hash)),
            ("choice_ids", choice_ids),
            ("reward_ids", reward_ids),
            ("next_action_id", next_action_id),
            ("next_checksum_id", next_checksum_id),
            ("next_hook_id", next_hook_id),
            ("run", run),
            ("events", events),
            ("checksums", checksums),
        ]))
    }

    // --- run snapshot -------------------------------------------------------
    fn serializable_run(&mut self) -> Res<Value> {
        let mut out = Map::new();
        out.insert("schema_version".into(), self.int(32)?);
        out.insert("acts".into(), self.list(Self::act_model)?);
        out.insert("modifiers".into(), self.list(Self::modifier)?);
        let daily = self.optional(|d| Ok(Value::from(d.r.i64()?)))?;
        out.insert("daily_time".into(), daily);
        out.insert("game_mode".into(), self.enum_name("GameMode")?);
        out.insert("current_act_index".into(), self.int(4)?);
        out.insert("events_seen".into(), self.model_entry_list()?);
        let room = self.optional(Self::room)?;
        out.insert("pre_finished_room".into(), room);
        let odds = obj(vec![
            ("card_reward_odds", self.r.f32()?),
            ("potion_reward_odds", self.r.f32()?),
            ("relic_reward_odds", self.r.f32()?),
            ("gold_reward_odds", self.r.f32()?),
        ]);
        out.insert("run_odds".into(), odds);
        out.insert("players".into(), self.list(Self::player)?);
        out.insert("rng".into(), self.run_rng_set()?);
        out.insert("shared_relic_grab_bag".into(), self.relic_grab_bag()?);
        out.insert("visited_map_coords".into(), self.list(Self::map_coord)?);
        let history = self.list(|d| d.list(Self::map_point_history_entry))?;
        out.insert("map_point_history".into(), history);
        out.insert("save_time".into(), Value::from(self.r.i64()?));
        out.insert("start_time".into(), Value::from(self.r.i64()?));
        out.insert("run_time".into(), Value::from(self.r.i64()?));
        out.insert("win_time".into(), Value::from(self.r.i64()?));
        out.insert("ascension".into(), self.int(8)?);
        let drawings = self.optional(Self::map_drawings)?;
        out.insert("map_drawings".into(), drawings);
        let extra = obj(vec![
            ("started_with_neow", self.boolean()?),
            ("test_subject_kills", self.int(32)?),
            ("freed_repy", self.boolean()?),
        ]);
        out.insert("extra_fields".into(), extra);
        out.insert("num_reloads".into(), self.int(32)?);
        Ok(Value::Object(out))
    }

    fn act_model(&mut self) -> Res<Value> {
        let id = self.model_entry()?;
        let rooms = self.room_set()?;
        let saved_map = self.optional(Self::act_map)?;
        Ok(obj(vec![
            ("id", id),
            ("rooms", rooms),
            ("saved_map", saved_map),
        ]))
    }

    fn modifier(&mut self) -> Res<Value> {
        let id = self.model_entry()?;
        let props = self.optional(Self::saved_properties)?;
        Ok(obj(vec![("id", id), ("props", props)]))
    }

    fn room_set(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("event_ids", self.model_entry_list()?),
            ("events_visited", self.int(32)?),
            ("normal_encounter_ids", self.model_entry_list()?),
            ("normal_encounters_visited", self.int(32)?),
            ("elite_encounter_ids", self.model_entry_list()?),
            ("elites_visited", self.int(32)?),
            ("bosses_visited", self.int(32)?),
            ("boss_id", self.optional(Self::model_entry)?),
            ("second_boss_id", self.optional(Self::model_entry)?),
            ("ancient_id", self.optional(Self::model_entry)?),
        ]))
    }

    fn act_map(&mut self) -> Res<Value> {
        let grid_width = self.int(8)?;
        let grid_height = self.int(8)?;
        let boss_point = self.map_point()?;
        let starting_point = self.map_point()?;
        let second_boss_point = self.optional(Self::map_point)?;
        let points = self.list_of(16, Self::map_point)?;
        let n = self.r.i(8)?;
        let starts = self.repeat(n, Self::map_coord)?;
        Ok(obj(vec![
            ("grid_width", grid_width),
            ("grid_height", grid_height),
            ("boss_point", boss_point),
            ("starting_point", starting_point),
            ("second_boss_point", second_boss_point),
            ("points", points),
            ("start_map_point_coords", starts),
        ]))
    }

    /// `SerializableMapPoint::Serialize` (v0.111.0 RVA 0x40d80) writes Coord
    /// first (IL_0013), then PointType as an 8-bit int (IL_0020); Deserialize
    /// (RVA 0x40e1c) reads them in that order (#2988).
    fn map_point(&mut self) -> Res<Value> {
        let coord = self.map_coord()?;
        let pt = self.r.i(8)?; // MapPointType written as a plain int8 here
        let point_type = match self.t.enums["MapPointType"].values.get(&pt) {
            Some(name) => Value::String(name.clone()),
            None => Value::from(pt),
        };
        let can_be_modified = self.boolean()?;
        let n = self.r.i(8)?;
        let child_coords = self.repeat(n, Self::map_coord)?;
        Ok(obj(vec![
            ("coord", coord),
            ("point_type", point_type),
            ("can_be_modified", can_be_modified),
            ("child_coords", child_coords),
        ]))
    }

    fn map_coord(&mut self) -> Res<Value> {
        // col, row
        Ok(Value::Array(vec![self.uint(8)?, self.uint(8)?]))
    }

    fn room(&mut self) -> Res<Value> {
        let room_type = self.int(32)?;
        let encounter_id = self.optional(Self::model_entry)?;
        let event_id = self.optional(Self::model_entry)?;
        let gold_proportion = self.r.f32()?;
        let is_pre_finished = self.boolean()?;
        let mut extra_rewards = Map::new();
        for _ in 0..self.r.i(32)?.max(0) {
            let key = self.r.u(64)?.to_string();
            let rewards = self.list(Self::reward)?;
            extra_rewards.insert(key, rewards);
        }
        let parent_event_id = self.optional(Self::model_entry)?;
        let should_resume_parent_event = self.boolean()?;
        let mut encounter_state = Map::new();
        for _ in 0..self.r.i(32)?.max(0) {
            let key = self.r.string()?;
            let value = self.string()?;
            encounter_state.insert(key, value);
        }
        Ok(obj(vec![
            ("room_type", room_type),
            ("encounter_id", encounter_id),
            ("event_id", event_id),
            ("gold_proportion", gold_proportion),
            ("is_pre_finished", is_pre_finished),
            ("extra_rewards", Value::Object(extra_rewards)),
            ("parent_event_id", parent_event_id),
            ("should_resume_parent_event", should_resume_parent_event),
            ("encounter_state", Value::Object(encounter_state)),
        ]))
    }

    fn reward(&mut self) -> Res<Value> {
        let mut out = Map::new();
        let reward_type = self.r.i(32)?;
        out.insert("reward_type".into(), Value::from(reward_type));
        out.insert("predetermined_model_id".into(), self.full_model_id()?);
        if reward_type == 6 {
            // A special card reward embeds the card.
            out.insert("special_card".into(), self.card()?);
        }
        out.insert("gold_amount".into(), self.int(32)?);
        out.insert("was_gold_stolen_back".into(), self.boolean()?);
        out.insert("source".into(), self.enum_name("CardCreationSource")?);
        out.insert("rarity_odds".into(), self.enum_name("CardRarityOddsType")?);
        out.insert("card_pool_ids".into(), self.model_entry_list()?);
        out.insert("option_count".into(), self.int(32)?);
        out.insert(
            "custom_description_encounter_source_id".into(),
            self.model_entry()?,
        );
        Ok(Value::Object(out))
    }

    fn player(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("net_id", self.uint(64)?),
            ("character", self.model_entry()?),
            ("current_hp", self.int(32)?),
            ("max_hp", self.int(32)?),
            ("max_energy", self.int(16)?),
            ("max_potion_slots", self.int(8)?),
            ("gold", self.int(32)?),
            ("base_orb_slots", self.int(16)?),
            ("deck", self.list(Self::card)?),
            ("relics", self.list(Self::relic)?),
            ("potions", self.list(Self::potion)?),
            ("player_rng", self.player_rng_set()?),
            (
                "odds",
                obj(vec![
                    ("card_rarity_odds_value", self.r.f32()?),
                    ("potion_reward_odds_value", self.r.f32()?),
                ]),
            ),
            ("relic_grab_bag", self.relic_grab_bag()?),
            (
                "extra_fields",
                obj(vec![
                    ("card_shop_removals_used", self.int(32)?),
                    ("wongo_points", self.int(32)?),
                    ("cccombo_badge_unlocked", self.boolean()?),
                    ("damage_dealt", self.int(32)?),
                    ("debuffs_applied", self.int(32)?),
                ]),
            ),
            (
                "unlock_state",
                obj(vec![
                    ("unlocked_epochs", self.repeat32(Self::epoch_id)?),
                    ("encounters_seen", self.model_entry_list()?),
                    ("number_of_runs", self.int(16)?),
                ]),
            ),
            ("discovered_cards", self.full_model_id_list()?),
            ("discovered_enemies", self.full_model_id_list()?),
            ("discovered_epochs", self.repeat32(Self::epoch_id)?),
            ("discovered_potions", self.full_model_id_list()?),
            ("discovered_relics", self.full_model_id_list()?),
        ]))
    }

    fn card(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("id", self.model_entry()?),
            ("upgrade_level", self.int(8)?),
            ("enchantment", self.optional(Self::enchantment)?),
            ("props", self.optional(Self::saved_properties)?),
            ("floor_added_to_deck", self.optional(|d| d.int(8))?),
        ]))
    }

    fn enchantment(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("id", self.model_entry()?),
            ("level", self.int(8)?),
            ("props", self.optional(Self::saved_properties)?),
        ]))
    }

    fn relic(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("id", self.model_entry()?),
            ("props", self.optional(Self::saved_properties)?),
            ("floor_added", self.optional(|d| d.int(8))?),
        ]))
    }

    fn potion(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("id", self.model_entry()?),
            ("slot", self.int(4)?),
        ]))
    }

    /// One rng set. Since v0.109 each stream is `SerializableRng`: a counter
    /// (int32) and the four xoshiro256** state words (uint64 each), RVA
    /// 0x408fc.
    fn rng_set(&mut self, enum_name: &str, seed: Value) -> Res<Value> {
        let mut counters = Map::new();
        let mut states = Map::new();
        for _ in 0..self.r.i(8)? {
            let name = self.enum_raw(enum_name)?.1;
            let counter = self.int(32)?;
            let mut words = Vec::with_capacity(4);
            for _ in 0..4 {
                words.push(self.uint(64)?);
            }
            counters.insert(name.clone(), counter);
            states.insert(name, Value::Array(words));
        }
        let mut out = Map::new();
        out.insert("seed".into(), seed);
        out.insert("counters".into(), Value::Object(counters));
        if !states.is_empty() {
            out.insert("states".into(), Value::Object(states));
        }
        Ok(Value::Object(out))
    }

    fn run_rng_set(&mut self) -> Res<Value> {
        let seed = self.string()?;
        self.rng_set("RunRngType", seed)
    }

    /// The player seed is a uint64 since v0.109 (`SerializablePlayerRngSet`,
    /// RVA 0x4042c).
    fn player_rng_set(&mut self) -> Res<Value> {
        let seed = self.uint(64)?;
        self.rng_set("PlayerRngType", seed)
    }

    fn relic_grab_bag(&mut self) -> Res<Value> {
        let mut out = Map::new();
        for _ in 0..self.r.i(32)?.max(0) {
            let rarity = self.enum_raw("RelicRarity")?.1;
            let ids = self.model_entry_list()?;
            out.insert(rarity, ids);
        }
        Ok(Value::Object(out))
    }

    /// `SavedProperties.Serialize` writes the property name BEFORE the value
    /// for every group, seven groups in this order.
    fn saved_properties(&mut self) -> Res<Value> {
        let mut out = Map::new();
        let mut group = |d: &mut Self, value: fn(&mut Self) -> Res<Value>| -> Res<()> {
            if d.r.boolean()? {
                for _ in 0..d.r.i(8)? {
                    let name = d.property_name()?;
                    let v = value(d)?;
                    out.insert(name, v);
                }
            }
            Ok(())
        };
        group(self, |d| d.int(32))?; // ints
        group(self, |d| d.repeat32(|d| d.int(32)))?; // int arrays
        group(self, Self::boolean)?; // bools
        group(self, Self::full_model_id)?; // model ids
        group(self, Self::card)?; // cards
        group(self, |d| d.repeat32(Self::card))?; // card arrays
        group(self, Self::string)?; // strings
        Ok(Value::Object(out))
    }

    // --- map history ----------------------------------------------------------
    fn map_point_history_entry(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("map_point_type", self.enum_name("MapPointType")?),
            ("rooms", self.list(Self::map_point_room_history)?),
            ("players", self.list(Self::player_map_point_history)?),
        ]))
    }

    fn map_point_room_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("room_type", self.enum_name("RoomType")?),
            ("model_id", self.optional(Self::full_model_id)?),
            ("monster_ids", self.model_entry_list()?),
            ("turns_taken", self.int(32)?),
        ]))
    }

    fn player_map_point_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("player_id", self.uint(64)?),
            ("gold_gained", self.int(32)?),
            ("gold_spent", self.int(32)?),
            ("gold_lost", self.int(32)?),
            ("gold_stolen", self.int(32)?),
            ("stolen_loot", self.int(32)?),
            ("current_gold", self.int(32)?),
            ("current_hp", self.int(32)?),
            ("max_hp", self.int(32)?),
            ("damage_taken", self.int(32)?),
            ("hp_healed", self.int(32)?),
            ("max_hp_gained", self.int(32)?),
            ("max_hp_lost", self.int(32)?),
            ("event_choices", self.list(Self::event_option_history)?),
            ("ancient_choices", self.list(Self::ancient_choice_history)?),
            ("cards_gained", self.list(Self::card)?),
            ("card_choices", self.list(Self::card_choice_history)?),
            ("relic_choices", self.list(Self::model_choice_history)?),
            ("potion_choices", self.list(Self::model_choice_history)?),
            ("potions_discarded", self.model_entry_list()?),
            ("potions_used", self.model_entry_list()?),
            ("cards_removed", self.list(Self::card)?),
            ("relics_removed", self.model_entry_list()?),
            (
                "cards_enchanted",
                self.list(Self::card_enchantment_history)?,
            ),
            (
                "cards_transformed",
                self.list(Self::card_transformation_history)?,
            ),
            ("cards_upgraded", self.model_entry_list()?),
            ("cards_downgraded", self.model_entry_list()?),
            ("event_choices_2", self.list(Self::event_option_history)?),
            ("rest_site_choices", self.repeat32(Self::string)?),
            ("bought_relics", self.model_entry_list()?),
            ("bought_potions", self.model_entry_list()?),
            ("bought_colorless", self.model_entry_list()?),
            ("completed_quests", self.model_entry_list()?),
            ("is_affected_by_fur_coat", self.boolean()?),
        ]))
    }

    fn event_option_history(&mut self) -> Res<Value> {
        let mut out = Map::new();
        out.insert("title_loc_table".into(), self.string()?);
        out.insert("title_loc_key".into(), self.string()?);
        if self.r.boolean()? {
            let mut variables = Map::new();
            for _ in 0..self.r.i(32)?.max(0) {
                let key = self.r.string()?;
                let value = self.dynamic_var()?;
                variables.insert(key, value);
            }
            out.insert("variables".into(), Value::Object(variables));
        }
        Ok(Value::Object(out))
    }

    /// `SerializableDynamicVar.Serialize` switches on the 1-based type:
    /// 1 BaseDynamic and 3 Decimal are a float32; 2 DynamicString and 4 String
    /// a string; 5 Bool a bool.
    fn dynamic_var(&mut self) -> Res<Value> {
        let (index, name) = self.enum_raw("DynamicVarType")?;
        match index {
            1 | 3 => self.r.f32(),
            2 | 4 => self.string(),
            5 => self.boolean(),
            _ => malformed(format!("bad DynamicVarType {name}")),
        }
    }

    fn ancient_choice_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("a", self.string()?),
            ("b", self.string()?),
            ("flag", self.boolean()?),
        ]))
    }

    fn card_choice_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("card", self.card()?),
            ("picked", self.boolean()?),
        ]))
    }

    fn model_choice_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("id", self.model_entry()?),
            ("picked", self.boolean()?),
        ]))
    }

    fn card_enchantment_history(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("card", self.card()?),
            ("enchantment_id", self.model_entry()?),
        ]))
    }

    fn card_transformation_history(&mut self) -> Res<Value> {
        Ok(obj(vec![("from", self.card()?), ("to", self.card()?)]))
    }

    fn map_drawings(&mut self) -> Res<Value> {
        self.list(|d| {
            let player_id = d.uint(64)?;
            let lines = d.list(|_| {
                // SerializableMapDrawingLine: bool + count(16) + points.
                malformed("map drawing decode not implemented (no sample file exercises it)")
            })?;
            Ok(obj(vec![("player_id", player_id), ("lines", lines)]))
        })
    }

    // --- events ---------------------------------------------------------------
    fn replay_event(&mut self) -> Res<Value> {
        let etype = self.r.i(3)?;
        let mut out = Map::new();
        match etype {
            1 => {
                out.insert("event_type".into(), "GameAction".into());
                out.insert("player_id".into(), self.uint(64)?);
                let action_id = self.r.u(8)?;
                out.insert("action".into(), self.net_action(action_id)?);
            }
            2 => {
                out.insert("event_type".into(), "HookAction".into());
                out.insert("player_id".into(), self.uint(64)?);
                out.insert("hook_id".into(), self.uint(32)?);
                out.insert("game_action_type".into(), self.enum_name("GameActionType")?);
            }
            3 => {
                out.insert("event_type".into(), "ResumeAction".into());
                out.insert("action_id".into(), self.uint(32)?);
            }
            4 => {
                out.insert("event_type".into(), "PlayerChoice".into());
                out.insert("player_id".into(), self.uint(64)?);
                out.insert("choice_id".into(), self.uint(32)?);
                out.insert("result".into(), self.player_choice_result()?);
            }
            _ => return malformed(format!("bad CombatReplayEventType {etype}")),
        }
        Ok(Value::Object(out))
    }

    fn net_action(&mut self, action_id: u64) -> Res<Value> {
        let Some(name) = usize::try_from(action_id)
            .ok()
            .and_then(|i| NET_ACTION_TYPES.get(i))
        else {
            return malformed(format!("unknown net action id {action_id}"));
        };
        let mut out = Map::new();
        out.insert("type".into(), Value::String((*name).to_owned()));
        match *name {
            "NetConsoleCmdGameAction" => {
                out.insert("cmd".into(), self.string()?);
                out.insert("in_combat".into(), self.boolean()?);
            }
            "NetDiscardPotionGameAction" => {
                out.insert("potion_slot_index".into(), self.uint(4)?);
                out.insert("was_enqueued_in_combat".into(), self.boolean()?);
            }
            "NetEndPlayerTurnAction" | "NetUndoEndPlayerTurnAction" => {
                out.insert("turn_number".into(), self.int(16)?);
            }
            "NetMoveToMapCoordAction" => {
                out.insert("destination".into(), self.map_coord()?);
            }
            "NetPickRelicAction" => {
                let index = self.optional(|d| d.int(8))?;
                out.insert("relic_index".into(), index);
            }
            "NetPlayCardAction" => {
                out.insert("combat_card_index".into(), self.uint(16)?);
                out.insert("card_id".into(), self.model_entry()?);
                let target = self.optional(|d| d.uint(6))?;
                out.insert("target_id".into(), target);
            }
            "NetUsePotionAction" => {
                out.insert("potion_index".into(), self.uint(4)?);
                out.insert("enqueued_in_combat".into(), self.boolean()?);
                let target = self.optional(|d| d.uint(6))?;
                out.insert("target_id".into(), target);
                let player = self.optional(|d| d.uint(64))?;
                out.insert("target_player_id".into(), player);
            }
            "NetVoteForMapCoordAction" => {
                let act_index = self.int(4)?;
                let coord = self.optional(Self::map_coord)?;
                out.insert(
                    "source".into(),
                    obj(vec![("act_index", act_index), ("coord", coord)]),
                );
                let destination = self.optional(|d| {
                    let count = d.int(4)?;
                    let coord = d.map_coord()?;
                    Ok(obj(vec![("map_generation_count", count), ("coord", coord)]))
                })?;
                out.insert("destination".into(), destination);
            }
            "NetVoteToMoveToNextActAction" => {
                out.insert("current_act_index".into(), self.int(32)?);
            }
            // NetReadyToBeginEnemyTurnAction carries nothing.
            _ => {}
        }
        Ok(Value::Object(out))
    }

    fn player_choice_result(&mut self) -> Res<Value> {
        let ctype = self.enum_raw("PlayerChoiceType")?.1;
        let mut out = Map::new();
        out.insert("type".into(), Value::String(ctype.clone()));
        match ctype.as_str() {
            "CanonicalCard" => {
                out.insert("cards".into(), self.repeat32(Self::model_entry)?);
            }
            "CombatCard" => {
                out.insert("combat_card_indexes".into(), self.list(|d| d.uint(16))?);
            }
            "DeckCard" => {
                out.insert("deck_indexes".into(), self.list(|d| d.uint(16))?);
            }
            "MutableCard" => {
                out.insert("cards".into(), self.list(Self::card)?);
                let owner = self.optional(|d| d.uint(64))?;
                out.insert("owner".into(), owner);
            }
            "Player" => {
                let player = self.optional(|d| d.uint(64))?;
                out.insert("player_id".into(), player);
            }
            "Index" => {
                out.insert("indexes".into(), self.repeat32(|d| d.int(32))?);
            }
            _ => return malformed(format!("bad PlayerChoiceType {ctype}")),
        }
        Ok(Value::Object(out))
    }

    // --- checksums ------------------------------------------------------------
    fn checksum_data(&mut self) -> Res<Value> {
        let id = self.uint(32)?;
        let value = self.uint(32)?;
        Ok(obj(vec![
            ("checksum", obj(vec![("id", id), ("value", value)])),
            ("context", self.string()?),
            ("full_state", self.net_full_combat_state()?),
        ]))
    }

    fn net_full_combat_state(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("creatures", self.list(Self::creature_state)?),
            ("players", self.list(Self::combat_player_state)?),
            ("rng", self.run_rng_set()?),
            ("next_choice_ids", self.repeat32(|d| d.uint(32))?),
            ("next_reward_ids", self.repeat32(|d| d.int(32))?),
            ("last_executed_action_id", self.optional(|d| d.uint(32))?),
            ("last_executed_hook_id", self.optional(|d| d.uint(32))?),
        ]))
    }

    /// `CreatureState.Serialize`, RVA 0x42282c (v0.109.1; nested in
    /// `MegaCrit.Sts2.Core.Entities.Multiplayer` with the states below).
    fn creature_state(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("monster_id", self.optional(Self::model_entry)?),
            ("player_id", self.optional(|d| d.uint(64))?),
            ("current_hp", self.int(32)?),
            ("max_hp", self.int(32)?),
            ("block", self.int(32)?),
            // PowerState.Serialize, RVA 0x422991
            (
                "powers",
                self.list(|d| Ok(obj(vec![("id", d.model_entry()?), ("amount", d.int(32)?)])))?,
            ),
        ]))
    }

    /// `PlayerState.Serialize`, RVA 0x422a68.
    fn combat_player_state(&mut self) -> Res<Value> {
        Ok(obj(vec![
            ("player_id", self.uint(64)?),
            ("character", self.model_entry()?),
            ("turn_number", self.int(32)?),
            ("phase", self.enum_name("PlayerTurnPhase")?),
            ("energy", self.int(32)?),
            ("stars", self.int(32)?),
            ("max_potion_count", self.int(32)?),
            ("gold", self.int(32)?),
            ("piles", self.list(Self::combat_pile_state)?),
            // PotionState is a bare model entry; RelicState is one
            // SerializableRelic (RVAs 0x422f3c / 0x422f58).
            ("potions", self.list(Self::model_entry)?),
            ("relics", self.list(Self::relic)?),
            // OrbState.Serialize, RVA 0x4229c9
            (
                "orbs",
                self.list(|d| {
                    Ok(obj(vec![
                        ("id", d.model_entry()?),
                        ("passive", d.int(16)?),
                        ("evoke", d.int(16)?),
                    ]))
                })?,
            ),
            ("player_rng", self.player_rng_set()?),
            ("relic_grab_bag", self.relic_grab_bag()?),
        ]))
    }

    /// `CombatPileState.Serialize`, RVA 0x422c27: pileType is a raw int32, not
    /// written through `WriteEnum`.
    fn combat_pile_state(&mut self) -> Res<Value> {
        let pile = self.r.i(32)?;
        let Some(table) = self.t.enums.get("PileType") else {
            return malformed(
                "tables have no PileType enum, so a combat-state pile cannot be labelled",
            );
        };
        let label = table
            .values
            .get(&pile)
            .cloned()
            .unwrap_or_else(|| format!("PileType?{pile}"));
        Ok(obj(vec![
            ("pile_type", Value::String(label)),
            ("cards", self.list(Self::card_state)?),
        ]))
    }

    /// `CardState.Serialize`, RVA 0x422ce0.
    fn card_state(&mut self) -> Res<Value> {
        let mut out = Map::new();
        out.insert("card".into(), self.card()?);
        if self.r.boolean()? {
            out.insert("affliction".into(), self.model_entry()?);
            out.insert("affliction_count".into(), self.int(32)?);
        } else {
            out.insert("affliction".into(), Value::Null);
        }
        let cost = self.optional(|d| d.int(32))?;
        out.insert("energy_cost".into(), cost);
        // The keyword count is 3 bits wide.
        let keywords = self.optional(|d| {
            let n = d.r.i(3)?;
            d.repeat(n, |d| d.enum_name("CardKeyword"))
        })?;
        out.insert("keywords".into(), keywords);
        Ok(Value::Object(out))
    }
}

/// Decode one `.mcr` file's bytes into the Python decoder's document.
///
/// # Errors
///
/// `unsupported_replay_build` when the file was not written by the build
/// these tables are for; `truncated_replay` when the stream ends early;
/// `malformed_replay` for anything else that does not decode.
pub fn decode(data: &[u8]) -> Result<Value, McrError> {
    Decoder {
        r: BitReader { data, pos: 0 },
        t: tables(),
    }
    .combat_replay()
}
