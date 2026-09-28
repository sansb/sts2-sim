//! `sts-sim run-counters --build vX.Y.Z --history PATH --fight N`
//!
//! Reads an `sts-sim-run-history-v1` document and prints the
//! `sts-sim-run-counters-v1` prediction for fight `N` (0-based, in the run's
//! combat order). `--build` has no default, as for `entry`: the counter
//! streams are seeded per build.

use std::fmt;

use super::{RunCountersRefusal, predict};
use crate::catalog::GameBuild;

const USAGE: &str = "usage: sts-sim run-counters --build vX.Y.Z --history PATH --fight N";

/// The argv-shaped refusals, plus the prediction's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunCountersCliRefusal {
    UnknownFlag(String),
    MissingValue(String),
    RepeatedFlag(String),
    MissingFlag(&'static str),
    BadFightIndex(String),
    UnadmittedBuild(Option<String>),
    Unreadable { path: String, detail: String },
    Prediction(RunCountersRefusal),
}

impl fmt::Display for RunCountersCliRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFlag(flag) => {
                write!(f, "refusal: run-counters: unknown flag {flag:?}; {USAGE}")
            }
            Self::MissingValue(flag) => {
                write!(
                    f,
                    "refusal: run-counters: flag {flag:?} takes a value; {USAGE}"
                )
            }
            Self::RepeatedFlag(flag) => {
                write!(
                    f,
                    "refusal: run-counters: flag {flag:?} was given twice; {USAGE}"
                )
            }
            Self::MissingFlag(flag) => {
                write!(f, "refusal: run-counters: {flag} is required; {USAGE}")
            }
            Self::BadFightIndex(value) => write!(
                f,
                "refusal: run-counters: --fight takes a non-negative integer, got {value:?}"
            ),
            Self::UnadmittedBuild(None) => write!(
                f,
                "refusal: run-counters: --build is required and has no default \
                 (SOLVER_INVARIANTS.md I11); {USAGE}"
            ),
            Self::UnadmittedBuild(Some(build)) => write!(
                f,
                "refusal: run-counters: unadmitted game build {build:?}; the admitted set is \
                 versions/admitted_builds.json and grows forward only"
            ),
            Self::Unreadable { path, detail } => {
                write!(f, "refusal: run-counters: cannot read {path:?}: {detail}")
            }
            Self::Prediction(refusal) => write!(f, "{refusal}"),
        }
    }
}

#[derive(Default)]
struct Config {
    build: Option<String>,
    history: Option<String>,
    fight: Option<String>,
}

fn parse_args(args: &[String]) -> Result<Config, RunCountersCliRefusal> {
    let mut config = Config::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let slot = match flag.as_str() {
            "--build" => &mut config.build,
            "--history" => &mut config.history,
            "--fight" => &mut config.fight,
            _ => return Err(RunCountersCliRefusal::UnknownFlag(flag.clone())),
        };
        let value = rest
            .next()
            .ok_or_else(|| RunCountersCliRefusal::MissingValue(flag.clone()))?;
        if slot.replace(value.clone()).is_some() {
            return Err(RunCountersCliRefusal::RepeatedFlag(flag.clone()));
        }
    }
    Ok(config)
}

/// Run the subcommand; the `Ok` string is the JSON document.
pub fn main(args: &[String]) -> Result<String, RunCountersCliRefusal> {
    let config = parse_args(args)?;
    let build = match config.build.as_deref() {
        None => return Err(RunCountersCliRefusal::UnadmittedBuild(None)),
        Some(name) => GameBuild::from_str(name)
            .ok_or_else(|| RunCountersCliRefusal::UnadmittedBuild(Some(name.to_owned())))?,
    };
    let path = config
        .history
        .ok_or(RunCountersCliRefusal::MissingFlag("--history"))?;
    let fight = config
        .fight
        .ok_or(RunCountersCliRefusal::MissingFlag("--fight"))?;
    let fight_index: usize = fight
        .parse()
        .map_err(|_| RunCountersCliRefusal::BadFightIndex(fight.clone()))?;
    let text =
        std::fs::read_to_string(&path).map_err(|error| RunCountersCliRefusal::Unreadable {
            path: path.clone(),
            detail: error.to_string(),
        })?;
    let prediction =
        predict(&text, build, fight_index).map_err(RunCountersCliRefusal::Prediction)?;
    Ok(prediction.to_json(build).to_string())
}
