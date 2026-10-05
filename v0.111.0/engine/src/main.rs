//! `sts-sim` command-line entry point.
//!
//! Admitted subcommands: `version`; `diff-serve` — the persistent
//! line-delimited JSON protocol the trajectory differential drives
//! (PORT_PLAN §4, #1287); and `bench` — the seeded throughput workload behind
//! the post-submit performance floors (PORT_PLAN §6/§7); and `exact-solve` —
//! one versioned coarse request/response for review search (#2449); and
//! `entry` — one fight's entry facts built from the game's own save bytes
//! (#2511, the #1282 D2 authority flip). Every
//! other argv is a typed refusal with a nonzero exit, never a best-effort
//! guess (PORT_PLAN D6).

mod bench;
mod diff_serve;

use std::fmt;
use std::io::{BufReader, Write};
use sts_sim::search;

const USAGE: &str = "usage: sts-sim version | sts-sim diff-serve | sts-sim exact-solve | \
                     sts-sim bench [options] | sts-sim entry [options] | sts-sim run-counters [options] | sts-sim search ENTRY uct|random SEED SECONDS | \
                     sts-sim mcr-decode FILE.mcr | sts-sim recorded-line ENTRY.json FILE.mcr";

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static GLOBAL: sts_sim::allocation::CountingAllocator = sts_sim::allocation::CountingAllocator;

/// Typed refusal surface of the scaffold binary.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CliRefusal {
    /// No subcommand was given.
    MissingSubcommand,
    /// A subcommand outside the admitted set was given.
    UnimplementedSubcommand(String),
    /// A subcommand was given extra arguments it does not accept.
    UnexpectedArguments { subcommand: String, extra: usize },
}

impl fmt::Display for CliRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubcommand => {
                write!(f, "refusal: missing subcommand; {USAGE}")
            }
            Self::UnimplementedSubcommand(name) => {
                write!(f, "refusal: unimplemented subcommand {name:?}; {USAGE}")
            }
            Self::UnexpectedArguments { subcommand, extra } => write!(
                f,
                "refusal: subcommand {subcommand:?} takes no arguments ({extra} given); {USAGE}"
            ),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("search") {
        if let Err(error) = search::run(&args) {
            eprintln!("{error}");
            std::process::exit(2);
        }
        return;
    }
    // `diff-serve` streams for the life of the process rather than returning
    // one line, so it is dispatched before the single-shot `run` surface it
    // otherwise shares an argv contract with.
    if args.first().map(String::as_str) == Some("diff-serve") {
        if args.len() > 1 {
            eprintln!(
                "{}",
                CliRefusal::UnexpectedArguments {
                    subcommand: "diff-serve".to_string(),
                    extra: args.len() - 1,
                }
            );
            std::process::exit(2);
        }
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        if let Err(error) = diff_serve::serve(BufReader::new(std::io::stdin().lock()), &mut stdout)
        {
            let _ = stdout.flush();
            eprintln!("refusal: diff-serve transport failure: {error}");
            std::process::exit(2);
        }
        return;
    }
    // Exact solve consumes one complete canonical request and produces one
    // complete result.  It is deliberately not folded into diff-serve: no
    // review should make a language crossing for every DFS node.
    if args.first().map(String::as_str) == Some("exact-solve") {
        if args.len() > 1 {
            eprintln!(
                "{}",
                CliRefusal::UnexpectedArguments {
                    subcommand: "exact-solve".to_string(),
                    extra: args.len() - 1,
                }
            );
            std::process::exit(2);
        }
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        if let Err(error) = sts_sim::exact_solve_v1::serve(std::io::stdin().lock(), &mut stdout) {
            let _ = stdout.flush();
            eprintln!("refusal: exact-solve transport failure: {error}");
            std::process::exit(2);
        }
        return;
    }
    // `bench` owns its own flag grammar and refusal type, so like `diff-serve`
    // it is dispatched before the no-argument `run` surface.
    if args.first().map(String::as_str) == Some("bench") {
        match bench::main(&args[1..]) {
            Ok(report) => println!("{report}"),
            Err(refusal) => {
                eprintln!("{refusal}");
                std::process::exit(2);
            }
        }
        return;
    }
    // `entry` reads a file and owns its own flag grammar and refusal type, so
    // like `bench` it is dispatched before the no-argument `run` surface. A
    // refused *entry* is a normal answer on stdout (see `entry::cli`); only a
    // refused *argv* reaches stderr and exit 2.
    if args.first().map(String::as_str) == Some("entry") {
        match sts_sim::entry::cli::main(&args[1..]) {
            Ok(document) => println!("{document}"),
            Err(refusal) => {
                eprintln!("{refusal}");
                std::process::exit(2);
            }
        }
        return;
    }
    // `mcr-decode FILE` prints the decoded replay document (#3578). A file
    // that does not decode is a named refusal on stdout, as `entry`'s is.
    if args.first().map(String::as_str) == Some("mcr-decode") {
        let Some(path) = args.get(1).filter(|_| args.len() == 2) else {
            eprintln!("usage: sts-sim mcr-decode FILE.mcr");
            std::process::exit(2);
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("{path}: {error}");
                std::process::exit(2);
            }
        };
        match sts_sim::mcr::decode(&bytes) {
            Ok(document) => println!("{document}"),
            Err(error) => println!(
                "{}",
                serde_json::json!({"refusal": {"code": error.code, "detail": error.detail}})
            ),
        }
        return;
    }
    // `recorded-line ENTRY.json FILE.mcr` resolves the capture's recorded
    // inputs against the entry and prints the line (#3578). A capture that
    // does not decode, or an input with no exact counterpart, is a named
    // refusal on stdout.
    if args.first().map(String::as_str) == Some("recorded-line") {
        // An optional third argument is the selection answer budget
        // (`RecordedOptions::max_selection_answers`).
        let budget = match args.get(3).map(|text| text.parse::<usize>()) {
            None => Some(None),
            Some(Ok(budget)) => Some(Some(budget)),
            Some(Err(_)) => None,
        };
        let (Some(entry_path), Some(mcr_path), Some(budget)) =
            (args.get(1), args.get(2).filter(|_| args.len() <= 4), budget)
        else {
            eprintln!("usage: sts-sim recorded-line ENTRY.json FILE.mcr [MAX_SELECTION_ANSWERS]");
            std::process::exit(2);
        };
        let options = sts_sim::recorded::RecordedOptions {
            max_selection_answers: budget,
        };
        let read = |path: &String| {
            std::fs::read(path).unwrap_or_else(|error| {
                eprintln!("{path}: {error}");
                std::process::exit(2);
            })
        };
        let entry: sts_sim::canonical::CanonicalStateV2 =
            match serde_json::from_slice(&read(entry_path)) {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!("{entry_path}: not a canonical v2 document: {error}");
                    std::process::exit(2);
                }
            };
        let answer = match sts_sim::mcr::decode(&read(mcr_path)) {
            Err(error) => {
                serde_json::json!({"refusal": {"check": error.code, "detail": error.detail}})
            }
            Ok(replay) => match sts_sim::recorded::recorded_line_with(&entry, &replay, &options) {
                Ok(line) => serde_json::json!({"ok": {
                    "actions": line.actions,
                    "step_digests": line.step_digests,
                    "terminal": line.terminal,
                    "nonrepresentative_uids": line.nonrepresentative_uids,
                }}),
                Err(diverged) => serde_json::json!({"refusal": {
                    "check": diverged.check, "detail": diverged.detail,
                    "step": diverged.step, "turn": diverged.turn,
                }}),
            },
        };
        println!("{answer}");
        return;
    }
    if args.first().map(String::as_str) == Some("run-counters") {
        match sts_sim::run_counters::cli::main(&args[1..]) {
            Ok(document) => println!("{document}"),
            Err(refusal) => {
                eprintln!("{refusal}");
                std::process::exit(2);
            }
        }
        return;
    }
    match run(&args) {
        Ok(output) => println!("{output}"),
        Err(refusal) => {
            eprintln!("{refusal}");
            std::process::exit(2);
        }
    }
}

fn run(args: &[String]) -> Result<String, CliRefusal> {
    match args.split_first() {
        None => Err(CliRefusal::MissingSubcommand),
        Some((subcommand, rest)) if subcommand == "version" => {
            if rest.is_empty() {
                Ok(env!("CARGO_PKG_VERSION").to_string())
            } else {
                Err(CliRefusal::UnexpectedArguments {
                    subcommand: subcommand.clone(),
                    extra: rest.len(),
                })
            }
        }
        Some((subcommand, _)) => Err(CliRefusal::UnimplementedSubcommand(subcommand.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn version_prints_the_crate_version() {
        assert_eq!(run(&argv(&["version"])).unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn empty_argv_refuses() {
        assert_eq!(run(&argv(&[])), Err(CliRefusal::MissingSubcommand));
    }

    #[test]
    fn diff_serve_is_not_a_single_shot_subcommand() {
        // `main` intercepts it; `run` must not answer it with a line, or the
        // two surfaces would disagree about what the binary supports.
        assert_eq!(
            run(&argv(&["diff-serve"])),
            Err(CliRefusal::UnimplementedSubcommand(
                "diff-serve".to_string()
            ))
        );
    }

    #[test]
    fn bench_is_not_a_single_shot_subcommand() {
        // Same split as `diff-serve`: `main` intercepts it because it carries
        // its own flags, and `run` must not claim to answer it.
        assert_eq!(
            run(&argv(&["bench"])),
            Err(CliRefusal::UnimplementedSubcommand("bench".to_string()))
        );
    }

    #[test]
    fn exact_solve_is_not_a_single_shot_subcommand() {
        // `main` owns stdin/stdout because one JSON request is the whole
        // authority boundary; `run` must not claim a conflicting answer.
        assert_eq!(
            run(&argv(&["exact-solve"])),
            Err(CliRefusal::UnimplementedSubcommand(
                "exact-solve".to_string()
            ))
        );
    }

    #[test]
    fn entry_is_not_a_single_shot_subcommand() {
        // Same split as `bench`: `main` intercepts it because it carries its
        // own flags and reads a file, and `run` must not claim to answer it.
        assert_eq!(
            run(&argv(&["entry"])),
            Err(CliRefusal::UnimplementedSubcommand("entry".to_string()))
        );
    }

    #[test]
    fn the_usage_string_names_every_admitted_subcommand() {
        assert!(USAGE.contains("version"));
        assert!(USAGE.contains("diff-serve"));
        assert!(USAGE.contains("bench"));
        assert!(USAGE.contains("exact-solve"));
        assert!(USAGE.contains("entry"));
    }

    #[test]
    fn unknown_subcommand_refuses() {
        assert_eq!(
            run(&argv(&["search"])),
            Err(CliRefusal::UnimplementedSubcommand("search".to_string()))
        );
    }

    #[test]
    fn version_refuses_extra_arguments() {
        assert_eq!(
            run(&argv(&["version", "--json"])),
            Err(CliRefusal::UnexpectedArguments {
                subcommand: "version".to_string(),
                extra: 1,
            })
        );
    }
}
