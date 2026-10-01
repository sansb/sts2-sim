//! `sts-sim entry`: build one fight's entry facts from a save on disk.
//!
//! ```text
//! sts-sim entry --build vX.Y.Z --save PATH [--encounter ID] [--node-type KIND]
//!                              [--opening [--native-checkpoints]] [--mcr PATH]
//! sts-sim entry --build vX.Y.Z --capture-run PATH [--encounter ID] [--node-type KIND]
//!                              [--opening [--native-checkpoints]] [--mcr PATH]
//! sts-sim entry --build vX.Y.Z --provenance-entry PATH
//! sts-sim entry --build vX.Y.Z --facts PATH [--opening [--native-checkpoints]]
//! ```
//!
//! `--native-checkpoints` (#3392) wraps a built, unspliced opening as
//! `sts-sim-opening-checkpoints-v1`: the unchanged root under `state`, and the
//! native checkpoints the deal passed under `native_checkpoints`. It needs
//! `--opening` and refuses beside `--mcr`. An opening that refuses answers
//! exactly as it does without the flag.
//!
//! `--facts` takes an `sts-sim-entry-v1` entry-facts document plus its
//! `streams` (`entry/facts.rs`, #2827 item B): the facts themselves, for a
//! caller with no run to read them from (the Coach's synthetic benchmark
//! roots). The document names its own encounter and node, so `--encounter`,
//! `--node-type` and `--mcr` refuse beside it. Without `--opening` the answer
//! is the validated document as `--save` would print it.
//!
//! `--capture-run` takes a replay capture's embedded run as
//! `mcr_parser.decode(...)["run"]` spells it, and maps it onto the save
//! surface exactly (`entry/capture.rs`, #2972). Everything after the parse is
//! the `--save` path unchanged.
//!
//! # `--opening` is opt-in, and the schema says which half ran
//!
//! Without it the subcommand emits `sts-sim-entry-v1` — the `build_entry`
//! argument bundle — exactly as it did before #2528, so the census's
//! `--root-with rust` measurement is unchanged. With it the subcommand runs
//! the run-RNG opening too and emits **`sts-sim-canonical-v2`**, a
//! post-`start_combat` state whose digest is directly comparable with the
//! oracle's save-rooted document. When the opening refuses, the answer is the
//! entry document with `opening.built = false` and the named refusal beside
//! it: `sts-sim-entry-v1` is never reinterpreted to mean "post-opening".
//!
//! `--mcr` implies `--opening`: the opening-checksum splice overwrites the
//! nine RNG streams of an already-opened combat, so without an opening there
//! is nothing to overwrite.
//!
//! # `--build` has no default, deliberately
//!
//! I11 records the precedent: `sts2_rng.seeding_scheme` was admitted by
//! *range* until #1265 forced it to be an enumeration, and the live Python
//! `build=` default (`GAME_BUILD_V0_108`) is still the open loose end tracked
//! by #1272. A fifth call site with an implicit build would widen exactly the
//! hole #1265 closed, so this one refuses without `--build`.
//!
//! # Exit status
//!
//! A *flag* refusal is stderr and exit 2, the binary's argv contract
//! (PORT_PLAN D6). An *entry* refusal is a normal answer: the refusal document
//! on stdout and exit 0, exactly as `diff-serve` answers a refused load with a
//! refusal line rather than a transport failure. What happened is in the JSON,
//! not in the status.

use std::fmt;
use std::path::PathBuf;

use crate::catalog::GameBuild;
use crate::entry::opening::OpeningOptions;
use crate::entry::opening::mcr::McrOpeningChecksum;
use crate::entry::refusal::EntryRefusal;
use crate::entry::{EntryInput, EntryRequest, document, provenance};

const USAGE: &str = "usage: sts-sim entry --build vX.Y.Z ((--save PATH | --capture-run PATH) \
                     [--encounter ID] [--node-type KIND] [--opening [--native-checkpoints]] \
                     [--mcr PATH] | --provenance-entry PATH | \
                     --facts PATH [--opening [--native-checkpoints]])";

/// The argv-shaped refusals of this subcommand.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryCliRefusal {
    /// A flag outside the admitted set.
    UnknownFlag(String),
    /// A flag that takes a value was given none.
    MissingValue(String),
    /// A flag was given twice; the second value would silently win.
    RepeatedFlag(String),
    /// No input was named.
    MissingInput,
    /// More than one input was named; they are different schemas.
    ConflictingInputs,
    /// `--build` was omitted, or named a build outside the admitted set.
    UnadmittedBuild(Option<String>),
    /// A flag that only applies to a save was given with a projection.
    FlagRequiresSave(String),
    /// `--native-checkpoints` without `--opening`, or beside `--mcr` (#3392).
    NativeCheckpointsNeedAnUnsplicedOpening,
    /// The input file could not be read.
    Unreadable { path: String, detail: String },
}

impl fmt::Display for EntryCliRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(f, "refusal: entry: unknown flag {flag:?}; {USAGE}"),
            Self::MissingValue(flag) => {
                write!(f, "refusal: entry: flag {flag:?} takes a value; {USAGE}")
            }
            Self::RepeatedFlag(flag) => {
                write!(f, "refusal: entry: flag {flag:?} was given twice; {USAGE}")
            }
            Self::MissingInput => write!(
                f,
                "refusal: entry: one of --save, --capture-run, --provenance-entry or \
                 --facts is required; {USAGE}"
            ),
            Self::ConflictingInputs => write!(
                f,
                "refusal: entry: --save, --capture-run, --provenance-entry and --facts are \
                 different input schemas; give exactly one; {USAGE}"
            ),
            Self::UnadmittedBuild(None) => write!(
                f,
                "refusal: entry: --build is required and has no default (SOLVER_INVARIANTS.md \
                 I11); {USAGE}"
            ),
            Self::UnadmittedBuild(Some(build)) => write!(
                f,
                "refusal: entry: unadmitted game build {build:?}; the admitted set is \
                 sim/builds.json and grows forward only"
            ),
            Self::FlagRequiresSave(flag) => write!(
                f,
                "refusal: entry: flag {flag:?} applies to --save or --capture-run only; {USAGE}"
            ),
            Self::NativeCheckpointsNeedAnUnsplicedOpening => write!(
                f,
                "refusal: entry: --native-checkpoints needs --opening and refuses beside --mcr; \
                 {USAGE}"
            ),
            Self::Unreadable { path, detail } => {
                write!(f, "refusal: entry: cannot read {path:?}: {detail}")
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct Config {
    save: Option<PathBuf>,
    capture_run: Option<PathBuf>,
    provenance_entry: Option<PathBuf>,
    facts: Option<PathBuf>,
    encounter: Option<String>,
    node_type: Option<String>,
    mcr: Option<PathBuf>,
    build: Option<String>,
    opening: bool,
    native_checkpoints: bool,
}

fn take<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), EntryCliRefusal> {
    if slot.is_some() {
        return Err(EntryCliRefusal::RepeatedFlag(flag.to_string()));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Config, EntryCliRefusal> {
    let mut config = Config::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = || -> Result<String, EntryCliRefusal> {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| EntryCliRefusal::MissingValue(flag.to_string()))
        };
        match flag {
            "--save" => take(&mut config.save, flag, PathBuf::from(value()?))?,
            "--capture-run" => take(&mut config.capture_run, flag, PathBuf::from(value()?))?,
            "--provenance-entry" => {
                take(&mut config.provenance_entry, flag, PathBuf::from(value()?))?;
            }
            "--facts" => take(&mut config.facts, flag, PathBuf::from(value()?))?,
            "--encounter" => take(&mut config.encounter, flag, value()?)?,
            "--node-type" => take(&mut config.node_type, flag, value()?)?,
            "--mcr" => take(&mut config.mcr, flag, PathBuf::from(value()?))?,
            "--build" => take(&mut config.build, flag, value()?)?,
            // The two flags that take no value, so they advance by one.
            "--opening" | "--native-checkpoints" => {
                let slot = if flag == "--opening" {
                    &mut config.opening
                } else {
                    &mut config.native_checkpoints
                };
                if *slot {
                    return Err(EntryCliRefusal::RepeatedFlag(flag.to_string()));
                }
                *slot = true;
                index += 1;
                continue;
            }
            other => return Err(EntryCliRefusal::UnknownFlag(other.to_string())),
        }
        index += 2;
    }
    Ok(config)
}

fn read(path: &PathBuf) -> Result<String, EntryCliRefusal> {
    std::fs::read_to_string(path).map_err(|error| EntryCliRefusal::Unreadable {
        path: path.display().to_string(),
        detail: error.to_string(),
    })
}

/// Run the subcommand, returning the JSON line to print.
pub fn main(args: &[String]) -> Result<String, EntryCliRefusal> {
    let config = parse_args(args)?;
    let build = match config.build.as_deref() {
        None => return Err(EntryCliRefusal::UnadmittedBuild(None)),
        Some(name) => GameBuild::from_str(name)
            .ok_or_else(|| EntryCliRefusal::UnadmittedBuild(Some(name.to_string())))?,
    };
    let named = [
        config.save.is_some(),
        config.capture_run.is_some(),
        config.provenance_entry.is_some(),
        config.facts.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if named > 1 {
        return Err(EntryCliRefusal::ConflictingInputs);
    }
    if config.native_checkpoints && (!config.opening || config.mcr.is_some()) {
        return Err(EntryCliRefusal::NativeCheckpointsNeedAnUnsplicedOpening);
    }
    let options = |mcr_first_checksum| OpeningOptions {
        mcr_first_checksum,
        record_native_checkpoints: config.native_checkpoints,
    };
    if let Some(path) = &config.facts {
        for (flag, present) in [
            ("--encounter", config.encounter.is_some()),
            ("--node-type", config.node_type.is_some()),
            ("--mcr", config.mcr.is_some()),
        ] {
            if present {
                return Err(EntryCliRefusal::FlagRequiresSave(flag.to_string()));
            }
        }
        let value = facts_outcome(&read(path)?, build, config.opening.then(|| options(None)));
        return Ok(serde_json::to_string(&value).expect("an entry document serializes"));
    }
    // At most one input is named from here on.
    let run_input = match (&config.save, &config.capture_run) {
        (Some(path), _) => Some((path, false)),
        (_, Some(path)) => Some((path, true)),
        _ => None,
    };
    let value = match (run_input, &config.provenance_entry) {
        (None, None) => return Err(EntryCliRefusal::MissingInput),
        (Some((path, capture)), _) => {
            let text = read(path)?;
            let request = EntryRequest {
                input: if capture {
                    EntryInput::CaptureRun(&text)
                } else {
                    EntryInput::Save(&text)
                },
                encounter_id: config.encounter.as_deref(),
                node_type: config.node_type.as_deref(),
                game_build: build,
                mcr_splice: config.mcr.is_some(),
            };
            if config.opening || config.mcr.is_some() {
                let checksum = match &config.mcr {
                    None => None,
                    Some(path) => Some(first_checksum(&read(path)?)?),
                };
                crate::entry::build_root_with(&request, &options(checksum.as_ref())).to_json()
            } else {
                crate::entry::build(&request).to_json()
            }
        }
        (None, Some(path)) => {
            for (flag, present) in [
                ("--encounter", config.encounter.is_some()),
                ("--node-type", config.node_type.is_some()),
                ("--mcr", config.mcr.is_some()),
                ("--opening", config.opening),
            ] {
                if present {
                    return Err(EntryCliRefusal::FlagRequiresSave(flag.to_string()));
                }
            }
            provenance_outcome(&read(path)?)
        }
    };
    Ok(serde_json::to_string(&value).expect("an entry document serializes"))
}

/// Read the capture's **first** checksum out of a decoded `.mcr` payload.
///
/// The `.mcr` decoder is not ported (§A5): `--mcr` takes a JSON file holding
/// the decoded capture, and the splice needs only `checksums[0]`. A payload
/// with no checksums is an argv-level refusal, because the caller asked for a
/// splice the file cannot supply.
fn first_checksum(text: &str) -> Result<McrOpeningChecksum, EntryCliRefusal> {
    let payload: serde_json::Value =
        serde_json::from_str(text).map_err(|error| EntryCliRefusal::Unreadable {
            path: "--mcr".to_string(),
            detail: error.to_string(),
        })?;
    let first = payload
        .get("checksums")
        .and_then(|checksums| checksums.get(0))
        .ok_or_else(|| EntryCliRefusal::Unreadable {
            path: "--mcr".to_string(),
            detail: "payload carries no checksums[0]".to_string(),
        })?;
    McrOpeningChecksum::from_first_checksum(first).map_err(|refusal| EntryCliRefusal::Unreadable {
        path: "--mcr".to_string(),
        detail: refusal.to_string(),
    })
}

/// An entry-facts document: validated, and opened when `--opening` asks.
fn facts_outcome(
    text: &str,
    build: GameBuild,
    opening: Option<OpeningOptions<'_>>,
) -> serde_json::Value {
    match (crate::entry::facts::parse(text, build), opening) {
        (Err(refusal), _) => document::refusal_json(&refusal),
        (Ok(document), Some(options)) => {
            crate::entry::root_from_document_with(document, &options).to_json()
        }
        (Ok(document), None) => document.to_json(),
    }
}

/// A projection always refuses in this slice; the document names every fact it
/// could not supply so the uploader contract can be extended deliberately.
fn provenance_outcome(text: &str) -> serde_json::Value {
    let (refusal, missing) = match provenance::build(text) {
        Err(refusal) => (refusal, Vec::new()),
        Ok(entry) => {
            let missing = provenance::missing_facts(&entry);
            let refusal = provenance::refusal_for(&entry).unwrap_or(
                // Unreachable while any fact is missing; kept exhaustive
                // rather than unwrapped so a future richer payload does not
                // panic here.
                EntryRefusal::ProvenanceEntryIncomplete {
                    fact: "save_schema_version",
                },
            );
            (refusal, missing)
        }
    };
    let mut value = document::refusal_json(&refusal);
    if !missing.is_empty() {
        value.as_object_mut().expect("a refusal document").insert(
            "missing".to_string(),
            serde_json::Value::Array(
                missing
                    .into_iter()
                    .map(|fact| serde_json::Value::String(fact.to_string()))
                    .collect(),
            ),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn the_build_has_no_default() {
        assert_eq!(
            main(&argv(&["--save", "/nonexistent"])),
            Err(EntryCliRefusal::UnadmittedBuild(None))
        );
    }

    #[test]
    fn an_unadmitted_build_refuses_by_name() {
        assert_eq!(
            main(&argv(&["--build", "v0.110.1", "--save", "/nonexistent"])),
            Err(EntryCliRefusal::UnadmittedBuild(Some(
                "v0.110.1".to_string()
            )))
        );
    }

    #[test]
    fn exactly_one_input_is_required() {
        assert_eq!(
            main(&argv(&["--build", "v0.111.0"])),
            Err(EntryCliRefusal::MissingInput)
        );
        assert_eq!(
            main(&argv(&[
                "--build",
                "v0.111.0",
                "--save",
                "a",
                "--provenance-entry",
                "b"
            ])),
            Err(EntryCliRefusal::ConflictingInputs)
        );
        for other in ["--save", "--provenance-entry"] {
            assert_eq!(
                main(&argv(&[
                    "--build",
                    "v0.111.0",
                    "--capture-run",
                    "a",
                    other,
                    "b"
                ])),
                Err(EntryCliRefusal::ConflictingInputs),
                "{other}"
            );
        }
    }

    #[test]
    fn a_capture_run_answers_exactly_what_its_first_save_answers() {
        // The corpus pair of `entry/capture.rs`'s tests, through argv.
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/capture_run_pair_v1");
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{fixtures}/pair.json")).unwrap(),
        )
        .unwrap();
        let answer = |flag: &str, file: &str| {
            main(&argv(&[
                "--build",
                "v0.111.0",
                flag,
                &format!("{fixtures}/{file}"),
                "--encounter",
                meta["encounter_id"].as_str().unwrap(),
                "--node-type",
                meta["node_type"].as_str().unwrap(),
                "--opening",
            ]))
            .unwrap()
        };
        let from_capture = answer("--capture-run", "capture_run.json");
        assert_eq!(from_capture, answer("--save", "save.json"));
        // Since #3322 this fight's Chaos Mad Science roots, so `--opening`
        // answers the canonical opening document (with the card's Tinker
        // row) rather than falling back to the `sts-sim-entry-v1` entry.
        assert!(from_capture.contains("\"game_build\":\"v0.111.0\""));
        assert!(from_capture.contains("[\"MAD_SCIENCE_TINKER\",2,6]"));
    }

    /// #3392: `--native-checkpoints` wraps the unchanged root beside the
    /// opening's recorded checkpoints, and needs an unspliced opening.
    #[test]
    fn native_checkpoints_wrap_the_unchanged_opening() {
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/capture_run_pair_v1");
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{fixtures}/pair.json")).unwrap(),
        )
        .unwrap();
        let save = format!("{fixtures}/save.json");
        let run = |extra: &[&str]| {
            let mut args = vec![
                "--build",
                "v0.111.0",
                "--save",
                &save,
                "--encounter",
                meta["encounter_id"].as_str().unwrap(),
                "--node-type",
                meta["node_type"].as_str().unwrap(),
            ];
            args.extend_from_slice(extra);
            main(&argv(&args))
        };
        let plain: serde_json::Value = serde_json::from_str(&run(&["--opening"]).unwrap()).unwrap();
        let wrapped: serde_json::Value =
            serde_json::from_str(&run(&["--opening", "--native-checkpoints"]).unwrap()).unwrap();
        assert_eq!(
            wrapped["schema"],
            serde_json::Value::from(crate::entry::OPENING_CHECKPOINTS_SCHEMA_V1)
        );
        assert_eq!(
            wrapped["state"], plain,
            "the root is byte for byte unchanged"
        );
        let recorded = wrapped["native_checkpoints"].as_array().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["kind"], "after_player_turn_start");
        assert_eq!(recorded[0]["state"]["schema"], plain["schema"]);
        assert!(recorded[0].get("refusal").is_none());

        for extra in [
            &["--native-checkpoints"][..],
            &["--opening", "--native-checkpoints", "--mcr", "x"][..],
        ] {
            assert_eq!(
                run(extra),
                Err(EntryCliRefusal::NativeCheckpointsNeedAnUnsplicedOpening),
                "{extra:?}"
            );
        }
        assert_eq!(
            parse_args(&argv(&["--native-checkpoints", "--native-checkpoints"])),
            Err(EntryCliRefusal::RepeatedFlag(
                "--native-checkpoints".to_string()
            ))
        );
    }

    #[test]
    fn a_capture_run_takes_the_save_only_flags() {
        // --encounter, --node-type and --opening are accepted with
        // --capture-run; the unreadable path is what refuses.
        let refusal = main(&argv(&[
            "--build",
            "v0.111.0",
            "--capture-run",
            "/definitely/not/here.json",
            "--encounter",
            "ENCOUNTER.X",
            "--node-type",
            "monster",
            "--opening",
        ]))
        .unwrap_err();
        assert!(matches!(refusal, EntryCliRefusal::Unreadable { .. }));
    }

    /// `--facts` on the corpus pair's own entry facts answers exactly what
    /// `--save` answers, with and without `--opening`; the save-only flags
    /// refuse beside it; and a facts refusal is a document, not an exit.
    #[test]
    fn a_facts_document_answers_exactly_what_its_save_answers() {
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/capture_run_pair_v1");
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{fixtures}/pair.json")).unwrap(),
        )
        .unwrap();
        let (encounter, node_type) = (
            meta["encounter_id"].as_str().unwrap(),
            meta["node_type"].as_str().unwrap(),
        );
        let save_text = std::fs::read_to_string(format!("{fixtures}/save.json")).unwrap();
        let document = match crate::entry::build(&EntryRequest {
            input: EntryInput::Save(&save_text),
            encounter_id: Some(encounter),
            node_type: Some(node_type),
            game_build: GameBuild::V0_111_0,
            mcr_splice: false,
        }) {
            crate::entry::EntryOutcome::Built(document) => document,
            other => panic!("the pair's save builds: {other:?}"),
        };
        let path = std::env::temp_dir().join(format!(
            "sts-sim-entry-facts-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, document.facts_json().to_string()).unwrap();
        let facts = path.to_str().unwrap();
        let save = format!("{fixtures}/save.json");
        let run = |extra: &[&str]| main(&argv(&[&["--build", "v0.111.0"], extra].concat()));
        let from_save = |opening: bool| {
            let mut args = vec![
                "--save",
                &save,
                "--encounter",
                encounter,
                "--node-type",
                node_type,
            ];
            if opening {
                args.push("--opening");
            }
            run(&args).unwrap()
        };
        assert_eq!(run(&["--facts", facts]).unwrap(), from_save(false));
        assert_eq!(
            run(&["--facts", facts, "--opening"]).unwrap(),
            from_save(true)
        );
        for flag in ["--encounter", "--node-type", "--mcr"] {
            assert_eq!(
                run(&["--facts", facts, flag, "x"]),
                Err(EntryCliRefusal::FlagRequiresSave(flag.to_string())),
                "{flag}"
            );
        }
        assert_eq!(
            run(&["--facts", facts, "--save", &save]),
            Err(EntryCliRefusal::ConflictingInputs)
        );
        std::fs::write(&path, "{").unwrap();
        let refused: serde_json::Value =
            serde_json::from_str(&run(&["--facts", facts, "--opening"]).unwrap()).unwrap();
        assert_eq!(
            refused["refusal_class"],
            serde_json::Value::from("malformed_facts_json")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unknown_repeated_and_valueless_flags_refuse() {
        assert_eq!(
            parse_args(&argv(&["--seed", "x"])),
            Err(EntryCliRefusal::UnknownFlag("--seed".to_string()))
        );
        assert_eq!(
            parse_args(&argv(&["--save"])),
            Err(EntryCliRefusal::MissingValue("--save".to_string()))
        );
        assert_eq!(
            parse_args(&argv(&["--save", "a", "--save", "b"])),
            Err(EntryCliRefusal::RepeatedFlag("--save".to_string()))
        );
    }

    #[test]
    fn save_only_flags_refuse_against_a_projection() {
        assert_eq!(
            main(&argv(&[
                "--build",
                "v0.111.0",
                "--provenance-entry",
                "a",
                "--encounter",
                "ENCOUNTER.MAWLER_NORMAL"
            ])),
            Err(EntryCliRefusal::FlagRequiresSave("--encounter".to_string()))
        );
    }

    #[test]
    fn an_unreadable_input_refuses_with_its_path() {
        let refusal = main(&argv(&[
            "--build",
            "v0.111.0",
            "--save",
            "/definitely/not/here.save",
        ]))
        .unwrap_err();
        assert!(refusal.to_string().contains("/definitely/not/here.save"));
    }

    #[test]
    fn a_projection_answers_with_a_refusal_document_and_its_missing_facts() {
        let text = r#"{"save_schema_version": 20, "run_rng": {},
                       "players": [{"character_id": "CHARACTER.DEFECT"}]}"#;
        let value = provenance_outcome(text);
        assert_eq!(
            value["refusal"]["kind"],
            serde_json::Value::from("entry_not_buildable")
        );
        assert_eq!(
            value["refusal_class"],
            serde_json::Value::from("provenance_entry_incomplete")
        );
        let missing = value["missing"].as_array().unwrap();
        assert!(missing.iter().any(|fact| fact == "players[].current_hp"));
    }
}
