//! The `sts-sim-run-history-v1` input: the raw `.run` plus the parser's fights.
//!
//! The fight rows carry only what the counter accounting reads from
//! `relay_parser.FightState`. Every field is typed here, so a row Python would
//! have tripped over (`KeyError`, `TypeError`) is a named refusal rather than a
//! default.

use serde_json::Value;

use super::RunCountersRefusal;
use crate::catalog::GameBuild;

/// The input schema name.
pub const SCHEMA: &str = "sts-sim-run-history-v1";

/// One `deck_entering` row, as far as the accounting reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckRow {
    /// The full model id, `CARD.` prefix included.
    pub id: String,
    pub upgrade_level: i64,
    /// The full enchantment id (`ENCHANTMENT.` prefix), or none.
    pub enchantment: Option<String>,
    /// `relay_parser`'s `upgrade_ambiguous` flag (absent means false).
    pub upgrade_ambiguous: bool,
}

impl DeckRow {
    /// `e["id"].replace("CARD.", "")`.
    pub fn bare_id(&self) -> String {
        self.id.replace("CARD.", "")
    }

    /// `(e.get("enchantment") or "").replace("ENCHANTMENT.", "")`.
    pub fn bare_enchantment(&self) -> String {
        self.enchantment
            .as_deref()
            .unwrap_or("")
            .replace("ENCHANTMENT.", "")
    }
}

/// One parsed fight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fight {
    pub node_index: i64,
    pub encounter_id: String,
    pub monster_ids: Vec<String>,
    pub turns_taken: i64,
    pub relics_entering: Vec<String>,
    pub potions_used: Vec<String>,
    pub deck_entering: Vec<DeckRow>,
}

/// The validated document.
#[derive(Clone, Debug)]
pub struct RunHistory {
    pub run: Value,
    pub fights: Vec<Fight>,
}

fn history_malformed(path: impl Into<String>, detail: impl Into<String>) -> RunCountersRefusal {
    RunCountersRefusal::MalformedHistory {
        path: path.into(),
        detail: detail.into(),
    }
}

fn history_string(value: &Value, path: &str) -> Result<String, RunCountersRefusal> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| history_malformed(path, "expected a string"))
}

fn history_integer(value: &Value, path: &str) -> Result<i64, RunCountersRefusal> {
    if value.is_boolean() {
        return Err(history_malformed(
            path,
            "expected an integer, found a boolean",
        ));
    }
    value
        .as_i64()
        .ok_or_else(|| history_malformed(path, "expected an integer"))
}

fn history_strings(value: &Value, path: &str) -> Result<Vec<String>, RunCountersRefusal> {
    value
        .as_array()
        .ok_or_else(|| history_malformed(path, "expected a list"))?
        .iter()
        .enumerate()
        .map(|(i, item)| history_string(item, &format!("{path}[{i}]")))
        .collect()
}

fn history_field<'a>(
    row: &'a Value,
    key: &str,
    path: &str,
) -> Result<&'a Value, RunCountersRefusal> {
    row.get(key)
        .ok_or_else(|| history_malformed(format!("{path}.{key}"), "missing"))
}

const FIGHT_KEYS: [&str; 7] = [
    "node_index",
    "encounter_id",
    "monster_ids",
    "turns_taken",
    "relics_entering",
    "potions_used",
    "deck_entering",
];
const DECK_KEYS: [&str; 4] = ["id", "upgrade_level", "enchantment", "upgrade_ambiguous"];

fn only_keys(row: &Value, keys: &[&str], path: &str) -> Result<(), RunCountersRefusal> {
    let object = row
        .as_object()
        .ok_or_else(|| history_malformed(path, "expected an object"))?;
    if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(history_malformed(format!("{path}.{key}"), "untaught key"));
    }
    Ok(())
}

fn deck_row(row: &Value, path: &str) -> Result<DeckRow, RunCountersRefusal> {
    only_keys(row, &DECK_KEYS, path)?;
    let enchantment = match row.get("enchantment") {
        None | Some(Value::Null) => None,
        Some(value) => Some(history_string(value, &format!("{path}.enchantment"))?),
    };
    let upgrade_ambiguous = match row.get("upgrade_ambiguous") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| {
            history_malformed(format!("{path}.upgrade_ambiguous"), "expected a boolean")
        })?,
    };
    Ok(DeckRow {
        id: history_string(history_field(row, "id", path)?, &format!("{path}.id"))?,
        upgrade_level: history_integer(
            history_field(row, "upgrade_level", path)?,
            &format!("{path}.upgrade_level"),
        )?,
        enchantment,
        upgrade_ambiguous,
    })
}

fn fight(row: &Value, path: &str) -> Result<Fight, RunCountersRefusal> {
    only_keys(row, &FIGHT_KEYS, path)?;
    let deck = history_field(row, "deck_entering", path)?
        .as_array()
        .ok_or_else(|| history_malformed(format!("{path}.deck_entering"), "expected a list"))?
        .iter()
        .enumerate()
        .map(|(i, card)| deck_row(card, &format!("{path}.deck_entering[{i}]")))
        .collect::<Result<_, _>>()?;
    Ok(Fight {
        node_index: history_integer(
            history_field(row, "node_index", path)?,
            &format!("{path}.node_index"),
        )?,
        encounter_id: history_string(
            history_field(row, "encounter_id", path)?,
            &format!("{path}.encounter_id"),
        )?,
        monster_ids: history_strings(
            history_field(row, "monster_ids", path)?,
            &format!("{path}.monster_ids"),
        )?,
        turns_taken: history_integer(
            history_field(row, "turns_taken", path)?,
            &format!("{path}.turns_taken"),
        )?,
        relics_entering: history_strings(
            history_field(row, "relics_entering", path)?,
            &format!("{path}.relics_entering"),
        )?,
        potions_used: history_strings(
            history_field(row, "potions_used", path)?,
            &format!("{path}.potions_used"),
        )?,
        deck_entering: deck,
    })
}

impl RunHistory {
    /// Parse and validate a document against the requested build.
    pub fn parse(text: &str, build: GameBuild) -> Result<Self, RunCountersRefusal> {
        let document: Value = serde_json::from_str(text)
            .map_err(|error| RunCountersRefusal::MalformedJson(error.to_string()))?;
        Self::from_value(document, build)
    }

    pub fn from_value(mut document: Value, build: GameBuild) -> Result<Self, RunCountersRefusal> {
        only_keys(&document, &["schema", "run", "fights"], "$")?;
        let schema = document.get("schema").and_then(Value::as_str);
        if schema != Some(SCHEMA) {
            return Err(RunCountersRefusal::SchemaMismatch(
                schema.map(str::to_owned),
            ));
        }
        let run = document
            .get_mut("run")
            .map(Value::take)
            .ok_or_else(|| history_malformed("$.run", "missing"))?;
        if !run.is_object() {
            return Err(history_malformed("$.run", "expected an object"));
        }
        let recorded = run.get("build_id").and_then(Value::as_str);
        if recorded != Some(build.as_str()) {
            return Err(RunCountersRefusal::RunBuildMismatch {
                requested: build.as_str().to_owned(),
                recorded: recorded.map(str::to_owned),
            });
        }
        // `relay_parser.require_single_player`, which `load_combats` calls.
        match run.get("players").and_then(Value::as_array) {
            Some(players) if players.len() == 1 => {}
            Some(players) => {
                return Err(RunCountersRefusal::MultiplayerRun(
                    players.len().to_string(),
                ));
            }
            None => return Err(RunCountersRefusal::MultiplayerRun("missing".to_owned())),
        }
        let fights = document
            .get("fights")
            .ok_or_else(|| history_malformed("$.fights", "missing"))?
            .as_array()
            .ok_or_else(|| history_malformed("$.fights", "expected a list"))?
            .iter()
            .enumerate()
            .map(|(i, row)| fight(row, &format!("$.fights[{i}]")))
            .collect::<Result<_, _>>()?;
        Ok(Self { run, fights })
    }
}
