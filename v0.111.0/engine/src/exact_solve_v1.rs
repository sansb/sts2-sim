//! Stable, coarse JSON boundary for the solo-v1 exact solver (#2449).
//!
//! `exact_dfs` deliberately exposes efficient Rust library types, not an ABI.
//! This module is the one versioned boundary a Python review may cross for a
//! solve: it receives the complete canonical entry and returns the complete
//! outcome and replay line.  Search nodes never cross this boundary.
//!
//! The response's `refusal.code` is the stable machine identity.  `detail`
//! is diagnostic evidence only and callers must not branch on its wording.

use std::io::{Read, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::boundary::HotBoundary;
use crate::canonical::CanonicalStateV2;
use crate::engine::{self, Action, SelectionAnswer, SelectionRef};
use crate::exact_dfs::{
    ExactCancellation, ExactDfsConfig, ExactDfsRefusal, ExactDfsResult, ExactDfsStatus,
    ExactObjective, ExactSolution,
};
use crate::solo_v1::{SoloV1Refusal, admit_exact_solve};

/// Identifier for this request/response schema.  It is bumped, never
/// reinterpreted; a caller that does not recognise it must refuse locally.
pub const PROTOCOL_V1: &str = "sts-sim-exact-solve-v1";

fn default_memo() -> bool {
    true
}

/// One complete exact-solve request.  The canonical entry makes this a
/// single, coarse boundary crossing rather than a language call per node.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactSolveRequestV1 {
    pub protocol: String,
    pub entry: CanonicalStateV2,
    pub max_turns: i16,
    /// Python's observed final-HP lower bound. It is deliberately distinct
    /// from `seed`: no action line is attached, so it may inform future safe
    /// pruning but can never become an achieved response by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_final_hp: Option<i32>,
    /// Optional complete-invocation wall cap. `0` is valid and returns an
    /// incomplete `deadline` result rather than inventing a line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default = "default_memo")]
    pub memo: bool,
    /// Optional memo byte budget (#3470). Past it the solve stops memoizing
    /// and continues, so the answer is unchanged and only speed is lost. See
    /// `ExactDfsConfig::memo_budget_bytes`. Omitted means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo_budget_bytes: Option<u64>,
    /// A fully replayable achieved incumbent.  It is not a blind alpha value:
    /// Rust replays it before accepting it as a deadline fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<ExactSolveSolutionV1>,
}

/// Stable wire representation of a replay action.
///
/// `select.answer` keeps uid and option-index selection domains distinct, so
/// a payload-equal selected card can never be substituted for its physical
/// identity during a Python review replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExactSolveActionV1 {
    Play {
        uid: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selection: Option<u32>,
    },
    Select {
        answer: ExactSolveSelectionV1,
    },
    Potion {
        slot: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<u8>,
    },
    End,
}

/// The two non-interchangeable selection identities on the stable wire.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExactSolveSelectionV1 {
    CardUid { uid: u32 },
    OptionIndex { index: u32 },
}

impl From<Action> for ExactSolveActionV1 {
    fn from(action: Action) -> Self {
        match action {
            Action::Play {
                uid,
                target,
                selection,
            } => Self::Play {
                uid,
                target,
                selection: selection.get(),
            },
            Action::Select { answer } => Self::Select {
                answer: match answer {
                    SelectionAnswer::CardUid(uid) => ExactSolveSelectionV1::CardUid { uid },
                    SelectionAnswer::OptionIndex(index) => {
                        ExactSolveSelectionV1::OptionIndex { index }
                    }
                },
            },
            Action::UsePotion { slot, target } => Self::Potion { slot, target },
            Action::EndTurn => Self::End,
        }
    }
}

impl TryFrom<ExactSolveActionV1> for Action {
    type Error = String;

    fn try_from(action: ExactSolveActionV1) -> Result<Self, Self::Error> {
        match action {
            ExactSolveActionV1::Play {
                uid,
                target,
                selection,
            } => {
                // `SelectionRef` uses uid + 1 to niche-pack `None`.  Never
                // let an untrusted wire uid reach its internal `expect`.
                if selection == Some(u32::MAX) {
                    return Err("selected card uid u32::MAX overflows uid + 1".to_owned());
                }
                Ok(Self::Play {
                    uid,
                    target,
                    selection: SelectionRef::new(selection),
                })
            }
            ExactSolveActionV1::Select { answer } => Ok(Self::Select {
                answer: match answer {
                    ExactSolveSelectionV1::CardUid { uid } => {
                        if uid == u32::MAX {
                            return Err("selected card uid u32::MAX overflows uid + 1".to_owned());
                        }
                        SelectionAnswer::CardUid(uid)
                    }
                    ExactSolveSelectionV1::OptionIndex { index } => {
                        SelectionAnswer::OptionIndex(index)
                    }
                },
            }),
            ExactSolveActionV1::Potion { slot, target } => Ok(Self::UsePotion { slot, target }),
            ExactSolveActionV1::End => Ok(Self::EndTurn),
        }
    }
}

/// The stable typed refusal vocabulary.  This is intentionally flatter than
/// the DFS's internal enum: callers can make safe product decisions without
/// importing engine or boundary implementation types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactSolveRefusalCodeV1 {
    MalformedRequest,
    UnsupportedProtocol,
    UnsupportedSchema,
    UnsupportedGameBuild,
    DocumentCarriesRefusal,
    MultiplayerAllies,
    MultiplayerPlayerOrder,
    TeammatePresent,
    TeammatePowerCardPending,
    TeammateDamagePending,
    TeammateDamageOwnerKey,
    CanonicalBoundaryRefused,
    EngineAdmissionRefused,
    EngineTransitionRefused,
    MemoProjectionRefused,
    CycleDetected,
    NoLegalActions,
    VictoryScoreCombinationNotModeled,
    InvalidHorizon,
    InvalidTurn,
    InvalidAlphaFloor,
    InvalidSeed,
}

/// One stable refusal.  `detail` remains human/debug evidence, not identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactSolveRefusalV1 {
    pub code: ExactSolveRefusalCodeV1,
    pub detail: String,
}

/// A terminal achieved solution.  It is present for exact wins and may be
/// present for an incomplete search only after the entire line replayed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactSolveSolutionV1 {
    pub actions: Vec<ExactSolveActionV1>,
    pub objective: [i64; 4],
    pub final_digest: String,
}

/// Stable response to one complete solve request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactSolveResponseV1 {
    pub protocol: String,
    pub status: ExactDfsStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solution: Option<ExactSolveSolutionV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ExactSolveRefusalV1>,
    pub telemetry: ExactSolveTelemetryV1,
}

/// Deliberately small, versioned telemetry contract.  It reports whole-solve,
/// current-thread allocation counters (not a process-wide approximation).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactSolveTelemetryV1 {
    pub elapsed_ns: u128,
    pub nodes: u64,
    pub transitions: u64,
    pub allocation_instrumented: bool,
    pub allocations: u64,
    pub allocated_bytes: u64,
    pub alpha_final_hp: i32,
    /// The memo's accounted bytes at stop (#3470). The browser client reports
    /// it, and budgets are tuned from it.
    #[serde(default)]
    pub memo_bytes: u64,
    /// The memo budget stopped at least one insert. It is never a statement
    /// about exactness; `status` is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub memo_budget_reached: bool,
}

/// Decode and run one request.  All invalid input becomes a response, never a
/// panic or a partial answer.
#[must_use]
pub fn solve_value(value: Value) -> ExactSolveResponseV1 {
    let request: ExactSolveRequestV1 = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(error) => return protocol_refusal(ExactSolveRefusalCodeV1::MalformedRequest, error),
    };
    if request.protocol != PROTOCOL_V1 {
        return protocol_refusal(
            ExactSolveRefusalCodeV1::UnsupportedProtocol,
            format!("expected {PROTOCOL_V1:?}, got {:?}", request.protocol),
        );
    }
    if request.alpha_final_hp.is_some_and(|value| value < 0) {
        return protocol_refusal(
            ExactSolveRefusalCodeV1::InvalidAlphaFloor,
            "alpha_final_hp must be non-negative",
        );
    }
    let seed = match request.seed.as_ref().map(solution_into_dfs) {
        Some(Ok(seed)) => Some(seed),
        Some(Err(refusal)) => return response_for_refusal(refusal),
        None => None,
    };
    let result = crate::exact_dfs::solve_seeded(
        &request.entry,
        ExactDfsConfig {
            max_turns: request.max_turns,
            deadline: request.deadline_ms.map(Duration::from_millis),
            memo: request.memo,
            memo_budget_bytes: request.memo_budget_bytes,
        },
        &ExactCancellation::default(),
        seed.as_ref(),
        request.alpha_final_hp.unwrap_or(0),
    );
    result.into()
}

/// Run the one-shot JSON protocol over stdin/stdout.  Exactly one JSON value
/// is read and exactly one JSON response is written.
pub fn serve<R: Read, W: Write>(mut input: R, output: &mut W) -> std::io::Result<()> {
    let mut body = String::new();
    input.read_to_string(&mut body)?;
    let value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(error) => {
            let response = protocol_refusal(ExactSolveRefusalCodeV1::MalformedRequest, error);
            serde_json::to_writer(&mut *output, &response)?;
            writeln!(output)?;
            return Ok(());
        }
    };
    serde_json::to_writer(&mut *output, &solve_value(value))?;
    writeln!(output)
}

/// Replay an adapter action line through the same solo/boundary/admission
/// gates used by `solve`.  This is an adapter validation primitive for review
/// integration tests; it is not a node-by-node product protocol.
pub fn replay_actions(
    entry: &CanonicalStateV2,
    actions: &[ExactSolveActionV1],
) -> Result<String, ExactSolveRefusalV1> {
    admit_exact_solve(entry).map_err(solo_refusal)?;
    let catalog = HotBoundary::catalog_from_canonical(entry).map_err(boundary_refusal)?;
    let mut state = HotBoundary::from_canonical(entry, &catalog).map_err(boundary_refusal)?;
    engine::admit(entry, &state, &catalog).map_err(|error| ExactSolveRefusalV1 {
        code: ExactSolveRefusalCodeV1::EngineAdmissionRefused,
        detail: error.to_string(),
    })?;
    let mut events = Vec::new();
    for action in actions.iter().cloned() {
        let action = action.try_into().map_err(|detail| ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::MalformedRequest,
            detail,
        })?;
        state =
            engine::apply_action_into(&state, &catalog, &action, &mut events).map_err(|error| {
                ExactSolveRefusalV1 {
                    code: ExactSolveRefusalCodeV1::EngineTransitionRefused,
                    detail: error.to_string(),
                }
            })?;
    }
    HotBoundary::try_to_canonical(&state, &catalog)
        .map(|state| state.differential_digest())
        .map_err(boundary_refusal)
}

impl From<ExactDfsResult> for ExactSolveResponseV1 {
    fn from(result: ExactDfsResult) -> Self {
        let solution = result.best.map(|solution| ExactSolveSolutionV1 {
            actions: solution.actions.into_iter().map(Into::into).collect(),
            objective: [
                i64::from(solution.objective.won),
                i64::from(solution.objective.final_hp),
                i64::from(solution.objective.potions_kept),
                i64::from(solution.objective.negative_turns),
            ],
            final_digest: solution.final_digest,
        });
        let refusal = result.refusal.map(refusal_from_dfs);
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            status: result.status,
            solution,
            refusal,
            telemetry: ExactSolveTelemetryV1 {
                elapsed_ns: result.telemetry.elapsed_ns,
                nodes: result.telemetry.nodes,
                transitions: result.telemetry.transitions,
                allocation_instrumented: result.telemetry.allocation_instrumented,
                allocations: result.telemetry.allocations,
                allocated_bytes: result.telemetry.allocated_bytes,
                alpha_final_hp: result.telemetry.alpha_final_hp,
                memo_bytes: result.telemetry.memo_bytes,
                memo_budget_reached: result.telemetry.memo_budget_reached,
            },
        }
    }
}

fn protocol_refusal(code: ExactSolveRefusalCodeV1, detail: impl ToString) -> ExactSolveResponseV1 {
    ExactSolveResponseV1 {
        protocol: PROTOCOL_V1.to_owned(),
        status: ExactDfsStatus::Refused,
        solution: None,
        refusal: Some(ExactSolveRefusalV1 {
            code,
            detail: detail.to_string(),
        }),
        telemetry: ExactSolveTelemetryV1 {
            elapsed_ns: 0,
            nodes: 0,
            transitions: 0,
            allocation_instrumented: crate::allocation::instrumented(),
            allocations: 0,
            allocated_bytes: 0,
            alpha_final_hp: 0,
            memo_bytes: 0,
            memo_budget_reached: false,
        },
    }
}

fn response_for_refusal(refusal: ExactDfsRefusal) -> ExactSolveResponseV1 {
    ExactSolveResponseV1 {
        protocol: PROTOCOL_V1.to_owned(),
        status: ExactDfsStatus::Refused,
        solution: None,
        refusal: Some(refusal_from_dfs(refusal)),
        telemetry: ExactSolveTelemetryV1 {
            elapsed_ns: 0,
            nodes: 0,
            transitions: 0,
            allocation_instrumented: crate::allocation::instrumented(),
            allocations: 0,
            allocated_bytes: 0,
            alpha_final_hp: 0,
            memo_bytes: 0,
            memo_budget_reached: false,
        },
    }
}

fn solution_into_dfs(seed: &ExactSolveSolutionV1) -> Result<ExactSolution, ExactDfsRefusal> {
    let [won, final_hp, potions_kept, negative_turns] = seed.objective;
    let potions_kept = u8::try_from(potions_kept)
        .map_err(|_| ExactDfsRefusal::InvalidSeed("seed potion count is outside u8".to_owned()))?;
    let negative_turns = i16::try_from(negative_turns)
        .map_err(|_| ExactDfsRefusal::InvalidSeed("seed turn count is outside i16".to_owned()))?;
    if won != 1 {
        return Err(ExactDfsRefusal::InvalidSeed(
            "seed objective won field must be integer 1".to_owned(),
        ));
    }
    let final_hp = i32::try_from(final_hp)
        .map_err(|_| ExactDfsRefusal::InvalidSeed("seed final HP is outside i32".to_owned()))?;
    let actions = seed
        .actions
        .iter()
        .cloned()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, String>>()
        .map_err(ExactDfsRefusal::InvalidSeed)?;
    Ok(ExactSolution {
        actions,
        objective: ExactObjective {
            won: true,
            // The v1 wire has no slot for it; seed validation replays the
            // line and keeps the replayed tier (#3369).
            event_reward: true,
            final_hp,
            potions_kept,
            negative_turns,
        },
        final_digest: seed.final_digest.clone(),
    })
}

fn solo_refusal(error: SoloV1Refusal) -> ExactSolveRefusalV1 {
    ExactSolveRefusalV1 {
        code: match error {
            SoloV1Refusal::UnsupportedSchema => ExactSolveRefusalCodeV1::UnsupportedSchema,
            SoloV1Refusal::UnsupportedGameBuild => ExactSolveRefusalCodeV1::UnsupportedGameBuild,
            SoloV1Refusal::DocumentCarriesRefusal => {
                ExactSolveRefusalCodeV1::DocumentCarriesRefusal
            }
            SoloV1Refusal::MultiplayerAllies => ExactSolveRefusalCodeV1::MultiplayerAllies,
            SoloV1Refusal::MultiplayerPlayerOrder => {
                ExactSolveRefusalCodeV1::MultiplayerPlayerOrder
            }
            SoloV1Refusal::TeammatePresent => ExactSolveRefusalCodeV1::TeammatePresent,
            SoloV1Refusal::TeammatePowerCardPending => {
                ExactSolveRefusalCodeV1::TeammatePowerCardPending
            }
            SoloV1Refusal::TeammateDamagePending => ExactSolveRefusalCodeV1::TeammateDamagePending,
            SoloV1Refusal::TeammateDamageOwnerKey => {
                ExactSolveRefusalCodeV1::TeammateDamageOwnerKey
            }
        },
        detail: error.to_string(),
    }
}

fn boundary_refusal(error: crate::boundary::BoundaryRefusal) -> ExactSolveRefusalV1 {
    ExactSolveRefusalV1 {
        code: ExactSolveRefusalCodeV1::CanonicalBoundaryRefused,
        detail: error.to_string(),
    }
}

fn refusal_from_dfs(error: ExactDfsRefusal) -> ExactSolveRefusalV1 {
    match error {
        ExactDfsRefusal::Solo(error) => solo_refusal(error),
        ExactDfsRefusal::Boundary(error) => boundary_refusal(error),
        ExactDfsRefusal::Admission(error) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::EngineAdmissionRefused,
            detail: error.to_string(),
        },
        ExactDfsRefusal::Transition(error) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::EngineTransitionRefused,
            detail: error.to_string(),
        },
        ExactDfsRefusal::MemoProjection(error) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::MemoProjectionRefused,
            detail: error.to_string(),
        },
        ExactDfsRefusal::CycleDetected => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::CycleDetected,
            detail: "DFS revisited an active state".to_owned(),
        },
        ExactDfsRefusal::NoLegalActions => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::NoLegalActions,
            detail: "admitted nonterminal state exposed no legal action".to_owned(),
        },
        ExactDfsRefusal::VictoryScoreCombinationNotModeled => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::VictoryScoreCombinationNotModeled,
            detail: "Meat on the Bone and Chosen Cheese are not jointly modeled".to_owned(),
        },
        ExactDfsRefusal::InvalidHorizon(value) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::InvalidHorizon,
            detail: format!("max_turns must be non-negative, got {value}"),
        },
        ExactDfsRefusal::InvalidTurn(value) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::InvalidTurn,
            detail: format!("turn must be non-negative, got {value}"),
        },
        ExactDfsRefusal::InvalidAlphaFloor(value) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::InvalidAlphaFloor,
            detail: format!("alpha_final_hp must be non-negative, got {value}"),
        },
        ExactDfsRefusal::InvalidSeed(detail) => ExactSolveRefusalV1 {
            code: ExactSolveRefusalCodeV1::InvalidSeed,
            detail,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const CORPUS: &str = include_str!("../fixtures/exact_solve_corpus_v1.json");

    fn request() -> Value {
        let entry = serde_json::from_str::<Value>(CORPUS).unwrap()["rows"][0]["entry"].clone();
        json!({
            "protocol": PROTOCOL_V1,
            "entry": entry,
            "max_turns": 4,
            "memo": true,
        })
    }

    #[test]
    fn memo_budget_is_an_optional_request_field_with_additive_telemetry() {
        // #3470: omitted means unlimited, and the unbudgeted wire stays as
        // it was apart from the additive `memo_bytes`. A one-turn horizon
        // keeps it cheap: this pins the wire, not the search.
        let request = || {
            let mut request = request();
            request["max_turns"] = json!(1);
            request
        };
        let plain = solve_value(request());
        assert_eq!(plain.status, ExactDfsStatus::Exact);
        assert!(plain.telemetry.memo_bytes > 0);
        assert!(!plain.telemetry.memo_budget_reached);
        let wire = serde_json::to_value(&plain).unwrap();
        assert!(wire["telemetry"].get("memo_budget_reached").is_none());

        // One byte short of the uncapped size: only the last inserts are
        // skipped, which keeps this fast while still exercising the cap.
        let budget = plain.telemetry.memo_bytes - 1;
        let mut budgeted = request();
        budgeted["memo_budget_bytes"] = json!(budget);
        let capped = solve_value(budgeted);
        assert_eq!(capped.status, ExactDfsStatus::Exact);
        assert_eq!(capped.solution, plain.solution);
        assert!(capped.telemetry.memo_budget_reached);
        assert!(capped.telemetry.memo_bytes <= budget);
        let wire = serde_json::to_value(&capped).unwrap();
        assert_eq!(wire["telemetry"]["memo_budget_reached"], json!(true));

        // A response written before these fields existed still decodes.
        let mut old = serde_json::to_value(&plain).unwrap();
        let telemetry = old["telemetry"].as_object_mut().unwrap();
        telemetry.remove("memo_bytes");
        let decoded: ExactSolveResponseV1 = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.telemetry.memo_bytes, 0);

        let mut negative = request();
        negative["memo_budget_bytes"] = json!(-1);
        assert_eq!(solve_value(negative).status, ExactDfsStatus::Refused);
    }

    #[test]
    fn versioned_boundary_returns_a_replayable_exact_solution() {
        let response = solve_value(request());
        assert_eq!(response.protocol, PROTOCOL_V1);
        assert_eq!(
            response.status,
            ExactDfsStatus::Exact,
            "{:?}",
            response.refusal
        );
        let solution = response.solution.expect("corpus has a winning solution");
        let entry = serde_json::from_value::<ExactSolveRequestV1>(request())
            .unwrap()
            .entry;
        assert_eq!(
            replay_actions(&entry, &solution.actions).unwrap(),
            solution.final_digest
        );
        assert!(solution.actions.iter().all(|action| matches!(
            action,
            ExactSolveActionV1::Play { .. } | ExactSolveActionV1::End
        )));
    }

    #[test]
    fn action_wire_round_trips_selected_card_and_potion_identity() {
        let actions = [
            ExactSolveActionV1::Play {
                uid: 7,
                target: Some(1),
                selection: Some(12),
            },
            ExactSolveActionV1::Select {
                answer: ExactSolveSelectionV1::CardUid { uid: 12 },
            },
            ExactSolveActionV1::Select {
                answer: ExactSolveSelectionV1::OptionIndex { index: 3 },
            },
            ExactSolveActionV1::Potion {
                slot: 2,
                target: Some(0),
            },
        ];
        for action in actions {
            let encoded = serde_json::to_value(&action).unwrap();
            let decoded: ExactSolveActionV1 = serde_json::from_value(encoded).unwrap();
            assert_eq!(
                Action::try_from(decoded).unwrap(),
                Action::try_from(action).unwrap()
            );
        }
    }

    #[test]
    fn bad_protocol_and_solo_refusal_are_stable_codes() {
        let mut bad_protocol = request();
        bad_protocol["protocol"] = json!("future-protocol");
        assert_eq!(
            solve_value(bad_protocol).refusal.unwrap().code,
            ExactSolveRefusalCodeV1::UnsupportedProtocol
        );
        let mut multiplayer = request();
        multiplayer["entry"]["player"]["teammate_present"] = json!(false);
        assert_eq!(
            solve_value(multiplayer).refusal.unwrap().code,
            ExactSolveRefusalCodeV1::TeammatePresent
        );
        let mut negative_turn = request();
        negative_turn["entry"]["player"]["turn"] = json!(-1);
        assert_eq!(
            solve_value(negative_turn).refusal.unwrap().code,
            ExactSolveRefusalCodeV1::InvalidTurn
        );
    }

    #[test]
    fn verified_constructive_seed_survives_a_deadline_without_becoming_exact() {
        let initial = solve_value(request());
        let seed = initial.solution.expect("corpus seed");
        let mut seeded = request();
        seeded["deadline_ms"] = json!(0);
        seeded["seed"] = serde_json::to_value(&seed).unwrap();
        let result = solve_value(seeded);
        assert_eq!(result.status, ExactDfsStatus::Deadline);
        assert_eq!(result.solution, Some(seed));
        assert!(result.refusal.is_none());
    }

    #[test]
    fn seed_outside_horizon_is_an_incomplete_fallback_not_an_exact_win() {
        let seed = solve_value(request())
            .solution
            .expect("turn-four corpus seed");
        assert_eq!(seed.objective[3], -4);
        let mut constrained = request();
        constrained["max_turns"] = json!(0);
        constrained["seed"] = serde_json::to_value(seed).unwrap();
        let result = solve_value(constrained);
        assert_eq!(result.status, ExactDfsStatus::Exact);
        assert!(result.solution.is_none());
        assert!(result.refusal.is_none());
    }

    #[test]
    fn overflowing_selection_uid_is_an_invalid_seed_not_a_panic() {
        let seed = solve_value(request()).solution.expect("corpus seed");
        let mut malformed = request();
        let mut wire_seed = serde_json::to_value(seed).unwrap();
        wire_seed["actions"][0]["selection"] = json!(u32::MAX);
        malformed["seed"] = wire_seed;
        assert_eq!(
            solve_value(malformed).refusal.unwrap().code,
            ExactSolveRefusalCodeV1::InvalidSeed
        );
    }

    #[test]
    fn retained_solo_review_corpus_enumerates_adapter_admission_frontier() {
        // This is the cutover census, not merely solo-policy admission. A zero
        // cap keeps it cheap while still proving canonical boundary + whole-
        // fight engine admission. Every retained solo row must cross both.
        let corpus: Value = serde_json::from_str(CORPUS).unwrap();
        for row in corpus["rows"].as_array().unwrap() {
            let response = solve_value(json!({
                "protocol": PROTOCOL_V1,
                "entry": row["entry"],
                "max_turns": 4,
                "deadline_ms": 0,
            }));
            match row["id"].as_str() {
                Some(
                    "synthetic-toadpoles-exact"
                    | "self-check-toadpoles-exact"
                    | "capture-6p96t755cnz3-mawler-deadline",
                ) => {
                    assert_eq!(
                        response.status,
                        ExactDfsStatus::Deadline,
                        "{:?}",
                        response.refusal
                    );
                    assert!(response.refusal.is_none());
                }
                other => panic!("uncensused exact-solve row {other:?}"),
            }
        }
    }
}
