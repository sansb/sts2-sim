//! Run-history RNG counter prediction for the legacy review path (#2827 C1).
//!
//! When a fight has neither floor saves nor an `.mcr` capture, nothing records
//! the run-lifetime RNG counters entering it. The legacy review path
//! (`review_summary.load_fight_context`) predicted them from the `.run` history
//! in Python: `replay_fight.predict` for the `Shuffle` stream and the
//! `solve_fight` accounting functions for the others. This module is that
//! prediction, ported so the Python simulator can be deleted (#2827 item F).
//!
//! # Inputs: the parser's projection, not a second parser
//!
//! The `.run` parsing layer (`relay_parser.parse_run`) is not ported: it is the
//! projection layer that forks forward per build. The input is an
//! `sts-sim-run-history-v1` document ([`history`]) carrying
//!
//! * `run`: the raw `.run` JSON, which the `Shuffle` predictor reads directly,
//!   exactly as `replay_fight.load_combats` did; and
//! * `fights`: the per-fight fields the other predictors read from
//!   `relay_parser.FightState` (`node_index`, `encounter_id`, `monster_ids`,
//!   `turns_taken`, `relics_entering`, `potions_used`, and the `deck_entering`
//!   rows' `id` / `upgrade_level` / `enchantment` / `upgrade_ambiguous`).
//!
//! The two sides index fights independently (`load_combats` and `parse_run`
//! each enumerate combat nodes). The Python path silently assumed they agree;
//! here every prior and target fight's node index is compared and a
//! disagreement is the named refusal `combat_enumeration_disagrees`.
//!
//! # Outputs: every counter says how much it is worth
//!
//! [`predict`] answers one [`StreamPrediction`] per stream plus the flat caveat
//! list in the order `load_fight_context` concatenated it. A stream's `status`
//! is derived, never asserted:
//!
//! * `exact`: a value and no caveat;
//! * `baseline`: a value the caveats say may be short of the truth (the
//!   `Shuffle` and `Niche` predictions under unrecorded prior-fight events);
//! * `unknown`: no value (the accounting proved it cannot know);
//! * `assumed`: `MonsterAi` after the run's first combat, which the legacy
//!   path never predicted. It entered at `start_combat`'s `ai_counter=0`
//!   default; the value is carried so the caller can reproduce that, and
//!   labelled so nothing reads it as a claim. Entering the first combat it is
//!   `exact` 0 ([`monster_ai_entering`]);
//! * `not_predicted`: `CombatPotionGeneration`, which the legacy path left
//!   `None` so every consumer refused in `start_combat`.
//!
//! # Where this departs from the Python it replaces
//!
//! One place, the `Shuffle` stream's seed ([`shuffle`]). `replay_fight.predict`
//! built `RunRngSet(seed)` and `Rng(rs["Shuffle"].seed)` without a build, so
//! both took `sts2_rng`'s v0.108 default: a djb2 run seed and stream hash added
//! at 32 bits, truncated to 32 bits before seeding xoshiro. v0.111.0's
//! `RunRngSet::.ctor` (RVA `0x4de0c`, IL_0060-IL_0067) stores
//! `StringHelper::GetDeterministicHashCode` (XxHash64) as the 64-bit `Seed`,
//! and `get_Shuffle` (`0x4dda3`) is `GetRng(1)`, built by `CreateRng`
//! (`0x4df00`) as [`crate::rng::run_stream_at_zero`]. The counter depends on
//! the order only through when an Ascender's Bane is drawn and leaves the cycle,
//! so the two agree on most fights and disagree exactly where that timing
//! differs. The walk `solver/invariant-walks/2026-09-25-issue2827-run-counters.md`
//! measures both against the recorded counters of captured start saves.
//!
//! One formatting difference is deliberate: `niche_counter_entering` rendered
//! its on-obtain relic set with Python's `set` repr, whose order follows the
//! per-process string hash. This module renders the same set sorted.

pub mod cli;
pub mod history;
pub mod shuffle;
pub mod streams;

#[cfg(test)]
mod tests;

use std::fmt;

use serde_json::{Value, json};

use crate::catalog::GameBuild;
use history::RunHistory;

/// The output schema name.
pub const SCHEMA: &str = "sts-sim-run-counters-v1";

/// Why no prediction was made. Every variant is a place where the Python path
/// raised (or, for the enumeration check, silently assumed) rather than
/// answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunCountersRefusal {
    /// The input was not JSON.
    MalformedJson(String),
    /// `schema` is not `sts-sim-run-history-v1`.
    SchemaMismatch(Option<String>),
    /// The run was recorded on a build other than the one requested.
    RunBuildMismatch {
        requested: String,
        recorded: Option<String>,
    },
    /// The run does not have exactly one player (`require_single_player`).
    MultiplayerRun(String),
    /// A field has the wrong shape, where Python would have raised
    /// `KeyError`/`TypeError`/`IndexError`.
    MalformedHistory { path: String, detail: String },
    /// The fight index is outside the run's combats.
    FightIndexOutOfRange { index: usize, fights: usize },
    /// `load_combats` and the parser projection disagree on which node a
    /// fight index names.
    CombatEnumerationDisagrees {
        fight: usize,
        run_node: Option<i64>,
        parsed_node: i64,
    },
    /// A prior fight's `turns_taken` is unrecorded (`replay_fight.predict`'s
    /// `"turns unknown; pass --turns"` exit).
    PriorFightTurnsUnknown { fight: usize },
    /// A `cards_removed` / `cards_transformed` row re-inserted into an entry
    /// deck has no `floor_added_to_deck`, and a later removal would have to
    /// compare against it (`entry_deck` raised `KeyError`).
    RemovalRowWithoutFloor { node: usize },
}

impl RunCountersRefusal {
    /// The stable machine name.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MalformedJson(_) => "malformed_json",
            Self::SchemaMismatch(_) => "history_schema_mismatch",
            Self::RunBuildMismatch { .. } => "run_build_mismatch",
            Self::MultiplayerRun(_) => "multiplayer_run",
            Self::MalformedHistory { .. } => "malformed_history",
            Self::FightIndexOutOfRange { .. } => "fight_index_out_of_range",
            Self::CombatEnumerationDisagrees { .. } => "combat_enumeration_disagrees",
            Self::PriorFightTurnsUnknown { .. } => "prior_fight_turns_unknown",
            Self::RemovalRowWithoutFloor { .. } => "removal_row_without_floor",
        }
    }
}

impl fmt::Display for RunCountersRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refusal: run-counters: {}: ", self.code())?;
        match self {
            Self::MalformedJson(detail) => write!(f, "{detail}"),
            Self::SchemaMismatch(found) => {
                write!(f, "expected schema {:?}, found {found:?}", history::SCHEMA)
            }
            Self::RunBuildMismatch {
                requested,
                recorded,
            } => write!(
                f,
                "requested build {requested:?} but the run records {recorded:?}; \
                 the counter streams are seeded per build"
            ),
            Self::MultiplayerRun(count) => write!(
                f,
                "the run has {count} players; the prediction reads one player's deck \
                 and refuses multiplayer rather than selecting players[0] (I5)"
            ),
            Self::MalformedHistory { path, detail } => write!(f, "{path}: {detail}"),
            Self::FightIndexOutOfRange { index, fights } => {
                write!(f, "fight index {index} is outside 0..{fights}")
            }
            Self::CombatEnumerationDisagrees {
                fight,
                run_node,
                parsed_node,
            } => write!(
                f,
                "fight {fight} is node {run_node:?} in the raw run's combat list but node \
                 {parsed_node} in the parsed fights"
            ),
            Self::PriorFightTurnsUnknown { fight } => write!(
                f,
                "fight {fight}'s turns_taken is unrecorded, so its Shuffle draws cannot be \
                 replayed"
            ),
            Self::RemovalRowWithoutFloor { node } => write!(
                f,
                "a removal row re-inserted at node {node} has no floor_added_to_deck to \
                 compare a later removal against"
            ),
        }
    }
}

/// How much one predicted counter is worth; see the module doc.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Exact,
    Baseline,
    Unknown,
    Assumed,
    NotPredicted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Baseline => "baseline",
            Self::Unknown => "unknown",
            Self::Assumed => "assumed",
            Self::NotPredicted => "not_predicted",
        }
    }

    /// The status a predictor's `(value, caveats)` pair earns.
    fn derived(value: Option<u64>, caveats: &[String]) -> Self {
        match (value, caveats.is_empty()) {
            (None, _) => Self::Unknown,
            (Some(_), true) => Self::Exact,
            (Some(_), false) => Self::Baseline,
        }
    }
}

/// `MonsterAi` entering fight `fight_index`: exact 0 entering the run's first
/// combat, `assumed` 0 after it (#2827 C2).
///
/// `RunRngSet::.ctor` (v0.111.0 RVA `0x4de0c`) creates every run stream at
/// counter 0 through `CreateRng` (`0x4df00`), and nothing outside combat
/// draws `MonsterAi`. `RunRngSet::get_MonsterAi` (`0x4dde2`), the stream's
/// only accessor, has exactly three callers in the DLL, all combat code:
///
/// * `MonsterModel::RollMove` (`0x825c8`, `IL_0015`), itself called only from
///   `Creature::PrepareForNextTurn` (`0x11d85c`, `IL_002a`) and
///   `CombatManager/<AfterCreatureAdded>d__113::MoveNext` (`0x3f2dfc`,
///   `IL_00cb`);
/// * `FlutterPower/<AfterDamageReceived>d__10::MoveNext` (`0x33a620`,
///   `IL_016b`);
/// * `Fabricator/<SpawnBot>d__25::MoveNext` (`0x35a80c`, `IL_005c`).
///
/// Re-derived for this change with a call-site scan of the archived DLL. So
/// the stream is untouched until the first combat, and all 36 captured
/// first-combat start saves record 0. After a combat the count depends on
/// unrecorded monster rolls, and the value stays `assumed`.
fn monster_ai_entering(fight_index: usize, caveats: Vec<String>) -> StreamPrediction {
    StreamPrediction {
        stream: "monster_ai",
        value: Some(0),
        status: if fight_index == 0 {
            Status::Exact
        } else {
            Status::Assumed
        },
        caveats,
    }
}

/// One stream's prediction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPrediction {
    /// The save's snake-case stream key (`RunRngType` through `SnakeCase`).
    pub stream: &'static str,
    pub value: Option<u64>,
    pub status: Status,
    pub caveats: Vec<String>,
}

impl StreamPrediction {
    fn derived(stream: &'static str, value: Option<u64>, caveats: Vec<String>) -> Self {
        Self {
            stream,
            value,
            status: Status::derived(value, &caveats),
            caveats,
        }
    }
}

/// The whole answer for one fight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prediction {
    pub fight_index: usize,
    pub encounter_id: String,
    pub floor: i64,
    /// In the order `load_fight_context` concatenated the caveats: the
    /// `Shuffle` replay, the prior-fight `Shuffle` sources, `Niche`,
    /// `MonsterAi`, `CombatCardSelection`, `CombatTargets`,
    /// `CombatEnergyCosts`, `CombatCardGeneration`, `CombatOrbs`.
    pub streams: Vec<StreamPrediction>,
}

impl Prediction {
    /// Every caveat, flattened in stream order.
    pub fn caveats(&self) -> Vec<String> {
        self.streams
            .iter()
            .flat_map(|s| s.caveats.iter().cloned())
            .collect()
    }

    pub fn to_json(&self, build: GameBuild) -> Value {
        let counters: serde_json::Map<String, Value> = self
            .streams
            .iter()
            .map(|s| {
                (
                    s.stream.to_owned(),
                    json!({"value": s.value, "status": s.status.as_str(), "caveats": s.caveats}),
                )
            })
            .collect();
        json!({
            "schema": SCHEMA,
            "build": build.as_str(),
            "fight_index": self.fight_index,
            "encounter_id": self.encounter_id,
            "floor": self.floor,
            "counters": counters,
            "caveats": self.caveats(),
        })
    }
}

/// Predict every counter entering `fight_index`.
pub fn predict(
    text: &str,
    build: GameBuild,
    fight_index: usize,
) -> Result<Prediction, RunCountersRefusal> {
    let history = RunHistory::parse(text, build)?;
    predict_history(&history, fight_index)
}

/// [`predict`] over an already-parsed history.
pub fn predict_history(
    history: &RunHistory,
    fight_index: usize,
) -> Result<Prediction, RunCountersRefusal> {
    let fights = &history.fights;
    if fight_index >= fights.len() {
        return Err(RunCountersRefusal::FightIndexOutOfRange {
            index: fight_index,
            fights: fights.len(),
        });
    }
    let combats = shuffle::load_combats(&history.run)?;
    for (k, fight) in fights.iter().enumerate().take(fight_index + 1) {
        let run_node = combats.combats.get(k).map(|c| c.node_index as i64);
        if run_node != Some(fight.node_index) {
            return Err(RunCountersRefusal::CombatEnumerationDisagrees {
                fight: k,
                run_node,
                parsed_node: fight.node_index,
            });
        }
    }
    let shuffle = shuffle::predict(&combats, fight_index)?;
    let floor = combats.combats[fight_index].floor;

    let mut shuffle_caveats = shuffle.caveats;
    shuffle_caveats.extend(streams::shuffle_counter_caveats(fights, fight_index));
    let (niche, niche_caveats) = streams::niche_counter_entering(fights, fight_index);
    let monster_ai_caveats = streams::monsterai_caveats(fights, fight_index);
    let (selection, selection_caveats) = streams::card_sel_counter_entering(fights, fight_index);
    let (targets, targets_caveats) = streams::combat_targets_counter_entering(fights, fight_index);
    let (energy, energy_caveats) = streams::energy_costs_counter_entering(fights, fight_index);
    let (generation, generation_caveats) =
        streams::combat_card_generation_counter_entering(fight_index);
    let (orbs, orbs_caveats) = streams::combat_orb_generation_counter_entering(fights, fight_index);

    let streams = vec![
        StreamPrediction::derived("shuffle", Some(shuffle.counter), shuffle_caveats),
        StreamPrediction::derived("niche", Some(niche), niche_caveats),
        monster_ai_entering(fight_index, monster_ai_caveats),
        StreamPrediction::derived("combat_card_selection", selection, selection_caveats),
        StreamPrediction::derived("combat_targets", targets, targets_caveats),
        StreamPrediction::derived("combat_energy_costs", energy, energy_caveats),
        StreamPrediction::derived("combat_card_generation", generation, generation_caveats),
        StreamPrediction::derived("combat_orbs", orbs, orbs_caveats),
        StreamPrediction {
            stream: "combat_potion_generation",
            value: None,
            status: Status::NotPredicted,
            caveats: Vec::new(),
        },
    ];
    Ok(Prediction {
        fight_index,
        encounter_id: fights[fight_index].encounter_id.clone(),
        floor,
        streams,
    })
}
