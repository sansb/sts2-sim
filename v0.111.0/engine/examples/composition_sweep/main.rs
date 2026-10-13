//! Composition sweep: synthetic fight roots through the crate's public path,
//! one JSON line per root (#3877).
//!
//! A measurement tool, not a gate. `tools/composition_sweep.py` builds this,
//! runs it in parallel shards and aggregates or compares the output; the
//! roots, stages and line shape are documented in `sweep.rs`.
//!
//! ```text
//! composition_sweep --mode single|pairs|enchant|chains [--character NAME|all]
//!     [--rollouts N] [--depth N] [--seed N] [--round-trips N] [--park-round-trips]
//!     [--only DIM[,DIM]] [--match TEXT]... [--shard I/N] [--limit N]
//!     [--out PATH] [--omit-ok] [--root-timeout SECONDS] [--in-process]
//! composition_sweep --mode custom --character NAME [--card [ENCH@]ID[+LEVEL]]...
//!     [--relic ID]... [--potion ID]... [--encounter ID] [--trace] [play flags]
//! ```
//!
//! `--trace` keeps each finding's whole action trail and the state it was met
//! in (its canonical document), for working on one root.
//!
//! `--match` keeps the roots whose item contains every given text. `--only`
//! keeps the named dimensions. `--shard I/N` keeps every Nth root; a root's
//! rollouts are seeded from its own name, so a slice, a shard and the full
//! sweep agree on every root they share.
//!
//! # A panic never ends the sweep
//!
//! The release profile aborts on panic, so the driver supervises a worker
//! (itself, re-run with `--worker`). The worker's panic hook writes the
//! `panic` line for the root in flight before the process dies; the
//! supervisor restarts the worker at the next root. A worker that dies with
//! no line (a stack overflow, a kill) is a `crash` line, and one that writes
//! nothing for `--root-timeout` seconds is killed and is a `timeout` line.
//! `--in-process` skips the supervisor: under an unwinding profile a panic is
//! caught and the sweep continues, under the release profile it ends it.
//!
//! `--fault panic@N|abort@N|hang@N` makes the worker fail at ordinal N, to
//! check the supervisor itself.
//!
//! Memory is bounded by one root: nothing is kept between roots but a count
//! per `(character, dimension, stage)`, written last as `summary` lines.

#[path = "sweep.rs"]
mod sweep;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};
use sweep::{CHARACTERS, Character, Mode, Play, Root};

const USAGE: &str = "usage: composition_sweep --mode single|pairs|enchant|chains|custom \
    [--character NAME|all] [--rollouts N] [--depth N] [--seed N] [--round-trips N] \
    [--only DIM[,DIM]] [--match TEXT]... [--shard I/N] [--limit N] [--out PATH] \
    [--omit-ok] [--root-timeout SECONDS] [--in-process] \
    [--card [ENCH@]ID[+LEVEL]]... [--relic ID]... [--potion ID]... [--encounter ID] [--trace]";

/// Which roots one invocation runs, in ordinal order.
struct Selection {
    mode: Option<Mode>,
    custom: Option<Root>,
    characters: Vec<Character>,
    only: Vec<String>,
    matches: Vec<String>,
    shard: (u64, u64),
    limit: Option<u64>,
}

impl Selection {
    /// Visit `(ordinal, character, root)` for every selected root at or after
    /// `start`. The ordinal counts roots after `--only`/`--match` and before
    /// `--shard`, so it names the same root in the supervisor and the worker.
    fn for_each(&self, start: u64, visit: &mut dyn FnMut(u64, Character, &Root) -> bool) {
        let mut ordinal = 0u64;
        let mut go = true;
        for character in &self.characters {
            let mut each = |root: Root| {
                if (!self.only.is_empty() && !self.only.iter().any(|dim| dim == root.dim))
                    || !self.matches.iter().all(|text| root.item.contains(text))
                {
                    return true;
                }
                let this = ordinal;
                ordinal += 1;
                if self.limit.is_some_and(|limit| this >= limit) {
                    go = false;
                    return false;
                }
                if this >= start && this % self.shard.1 == self.shard.0 {
                    go = visit(this, *character, &root);
                }
                go
            };
            match (&self.custom, self.mode) {
                (Some(root), _) => {
                    each(root.clone());
                }
                (None, Some(mode)) => sweep::for_each_root(mode, *character, &mut each),
                (None, None) => unreachable!("a selection names a mode or a custom root"),
            }
            if !go {
                return;
            }
        }
    }
}

struct Config {
    selection: Selection,
    play: Play,
    out: Option<String>,
    omit_ok: bool,
    root_timeout: Duration,
    in_process: bool,
    worker: bool,
    start: u64,
    fault: Option<(String, u64)>,
}

fn parse(args: &[String]) -> Result<Config, String> {
    let mut mode: Option<String> = None;
    let mut character = "all".to_string();
    let mut play = Play::default();
    let mut only = Vec::new();
    let mut matches = Vec::new();
    let mut shard = (0u64, 1u64);
    let mut limit = None;
    let mut out = None;
    let mut omit_ok = false;
    let mut root_timeout = 120u64;
    let mut in_process = false;
    let mut worker = false;
    let mut start = 0u64;
    let mut fault = None;
    let (mut cards, mut relics, mut potions) = (Vec::new(), Vec::new(), Vec::new());
    let mut encounter: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--omit-ok" => omit_ok = true,
            "--in-process" => in_process = true,
            "--worker" => worker = true,
            "--trace" => play.trace = true,
            "--park-round-trips" => play.park_round_trips = true,
            _ => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("flag {flag:?} takes a value"))?;
                let number = || {
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("flag {flag:?} takes a number, not {value:?}"))
                };
                match flag {
                    "--mode" => mode = Some(value.clone()),
                    "--character" => character = value.clone(),
                    "--rollouts" => play.rollouts = number()? as u32,
                    "--depth" => play.depth = number()? as u32,
                    "--seed" => play.seed = number()?,
                    "--round-trips" => play.round_trips = number()? as u32,
                    "--only" => only.extend(value.split(',').map(str::to_string)),
                    "--match" => matches.push(value.clone()),
                    "--shard" => {
                        let parsed = value.split_once('/').and_then(|(index, count)| {
                            Some((index.parse::<u64>().ok()?, count.parse::<u64>().ok()?))
                        });
                        shard = match parsed {
                            Some((index, count)) if index < count => (index, count),
                            _ => {
                                return Err(format!("--shard takes I/N with I < N, not {value:?}"));
                            }
                        };
                    }
                    "--limit" => limit = Some(number()?),
                    "--out" => out = Some(value.clone()),
                    "--root-timeout" => root_timeout = number()?,
                    "--start" => start = number()?,
                    "--fault" => {
                        fault = Some(
                            value
                                .split_once('@')
                                .and_then(|(kind, at)| Some((kind.to_string(), at.parse().ok()?)))
                                .ok_or_else(|| format!("--fault takes KIND@N, not {value:?}"))?,
                        );
                    }
                    "--card" => cards.push(value.clone()),
                    "--relic" => relics.push(value.clone()),
                    "--potion" => potions.push(value.clone()),
                    "--encounter" => encounter = Some(value.clone()),
                    _ => return Err(format!("unknown flag {flag:?}")),
                }
                index += 1;
            }
        }
        index += 1;
    }
    let characters = if character == "all" {
        CHARACTERS.to_vec()
    } else {
        vec![
            Character::named(&character)
                .ok_or_else(|| format!("unknown character {character:?}"))?,
        ]
    };
    let mode = mode.ok_or("--mode is required")?;
    let custom = if mode == "custom" {
        in_process = true;
        Some(sweep::custom_root(
            &cards,
            &relics,
            &potions,
            encounter.as_deref(),
        )?)
    } else if cards.len() + relics.len() + potions.len() > 0 || encounter.is_some() {
        return Err("--card, --relic, --potion and --encounter need --mode custom".to_string());
    } else {
        None
    };
    let mode = match custom {
        Some(_) => None,
        None => Some(Mode::named(&mode).ok_or_else(|| format!("unknown mode {mode:?}"))?),
    };
    Ok(Config {
        selection: Selection {
            mode,
            custom,
            characters,
            only,
            matches,
            shard,
            limit,
        },
        play,
        out,
        omit_ok,
        root_timeout: Duration::from_secs(root_timeout),
        in_process,
        worker,
        start,
        fault,
    })
}

/// The root in flight, as the top of the line a panic would write.
static IN_FLIGHT: Mutex<Option<Value>> = Mutex::new(None);
/// The panic line of a panic that unwound, for the worker loop to emit.
static PANICKED: Mutex<Option<Value>> = Mutex::new(None);

fn locked<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let Some(mut line) = locked(&IN_FLIGHT).take() else {
            default(info);
            return;
        };
        line["stage"] = json!("panic");
        line["detail"] = json!(info.to_string());
        if cfg!(panic = "unwind") {
            *locked(&PANICKED) = Some(line);
        } else {
            // The process is about to abort: this is the last word on the
            // root, and stdout is line-buffered, so it is on the pipe before
            // the supervisor sees the exit.
            println!("{line}");
        }
    }));
}

/// Run the selected roots from `start`, one line each.
fn work(config: &Config, start: u64, emit: &mut dyn FnMut(Value)) {
    let play = &config.play;
    config
        .selection
        .for_each(start, &mut |ordinal, character, root| {
            *locked(&IN_FLIGHT) = Some(sweep::outcome_json(ordinal, character, root, &[]));
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                match &config.fault {
                    Some((kind, at)) if *at == ordinal => match kind.as_str() {
                        "panic" => panic!("--fault panic@{ordinal}"),
                        "abort" => std::process::abort(),
                        _ => loop {
                            std::thread::sleep(Duration::from_secs(1));
                        },
                    },
                    _ => {}
                }
                sweep::run_root(character, root, play)
            }));
            *locked(&IN_FLIGHT) = None;
            match outcome {
                Ok(findings) => emit(sweep::outcome_json(ordinal, character, root, &findings)),
                Err(_) => {
                    if let Some(line) = locked(&PANICKED).take() {
                        emit(line);
                    }
                }
            }
            true
        });
}

/// Where lines go, and the counts the summary is written from.
struct Sink {
    out: Box<dyn Write>,
    omit_ok: bool,
    tally: BTreeMap<(String, String, String), u64>,
    /// The stage of the last line written, for the supervisor.
    last_stage: Option<String>,
}

impl Sink {
    fn open(config: &Config) -> Result<Self, String> {
        let out: Box<dyn Write> = match &config.out {
            Some(path) => Box::new(BufWriter::new(
                File::create(path).map_err(|error| format!("cannot write {path:?}: {error}"))?,
            )),
            None => Box::new(std::io::stdout()),
        };
        Ok(Self {
            out,
            omit_ok: config.omit_ok,
            tally: BTreeMap::new(),
            last_stage: None,
        })
    }

    fn line(&mut self, line: &Value) {
        let text = |key: &str| line[key].as_str().unwrap_or_default().to_string();
        let stage = text("stage");
        let ok = stage == "ok";
        self.last_stage = Some(stage.clone());
        *self
            .tally
            .entry((text("ch"), text("dim"), stage))
            .or_default() += 1;
        if !(ok && self.omit_ok) {
            writeln!(self.out, "{line}").expect("the output is writable");
        }
    }

    /// One `summary` line per `(character, dimension)`: how many roots ran and
    /// how many of each stage. With `--omit-ok` this is where the `ok` count
    /// lives.
    fn finish(mut self) {
        let mut groups: BTreeMap<(String, String), BTreeMap<String, u64>> = BTreeMap::new();
        for ((character, dim, stage), count) in &self.tally {
            groups
                .entry((character.clone(), dim.clone()))
                .or_default()
                .insert(stage.clone(), *count);
        }
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        for ((character, dim), stages) in groups {
            let roots: u64 = stages.values().sum();
            for (stage, count) in &stages {
                *totals.entry(stage.clone()).or_default() += count;
            }
            let line = json!({
                "summary": true, "ch": character, "dim": dim, "roots": roots, "stages": stages,
            });
            writeln!(self.out, "{line}").expect("the output is writable");
        }
        self.out.flush().expect("the output is writable");
        eprintln!("composition_sweep: {totals:?}");
    }
}

/// Supervise workers until every selected root has a line.
fn supervise(config: &Config, args: &[String], sink: &mut Sink) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut next = 0u64;
    loop {
        let mut child = Command::new(&program)
            .args(args)
            .args(["--worker", "--start", &next.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start a worker: {error}"))?;
        let stdout = child.stdout.take().expect("the worker's stdout is piped");
        let (sender, receiver) = mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        // How the worker stopped: `None` is an exit, `Some` a kill.
        let stopped = loop {
            match receiver.recv_timeout(config.root_timeout) {
                Ok(text) => {
                    let Ok(line) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if let Some(ordinal) = line["i"].as_u64() {
                        next = ordinal + 1;
                    }
                    sink.line(&line);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    break Some(format!(
                        "no outcome within {} seconds",
                        config.root_timeout.as_secs()
                    ));
                }
            }
        };
        let status = child.wait().map_err(|error| error.to_string())?;
        let _ = reader.join();
        // Lines the reader had queued when the worker was killed.
        for text in receiver.try_iter() {
            if let Ok(line) = serde_json::from_str::<Value>(&text) {
                if let Some(ordinal) = line["i"].as_u64() {
                    next = ordinal + 1;
                }
                sink.line(&line);
            }
        }
        if stopped.is_none() && status.success() {
            return Ok(());
        }
        // The worker died. If its hook wrote the root's panic line, `next` is
        // already past it. Otherwise the first selected root from `next` is
        // the one that was in flight, and gets a line here.
        let mut in_flight: Option<Value> = None;
        config
            .selection
            .for_each(next, &mut |ordinal, character, root| {
                in_flight = Some(sweep::outcome_json(ordinal, character, root, &[]));
                false
            });
        let Some(mut line) = in_flight else {
            return Ok(());
        };
        let panicked_last = sink.last_stage.as_deref() == Some("panic");
        if !(panicked_last && stopped.is_none()) {
            match stopped {
                Some(detail) => {
                    line["stage"] = json!("timeout");
                    line["detail"] = json!(detail);
                }
                None => {
                    line["stage"] = json!("crash");
                    line["detail"] = json!(format!("the worker died without a line: {status}"));
                }
            }
            next = line["i"].as_u64().expect("an outcome has an ordinal") + 1;
            sink.line(&line);
        }
        sink.last_stage = None;
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match parse(&args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("refusal: composition_sweep: {error}; {USAGE}");
            return ExitCode::from(2);
        }
    };
    install_panic_hook();
    if config.worker {
        work(&config, config.start, &mut |line| {
            println!("{line}");
        });
        return ExitCode::SUCCESS;
    }
    let mut sink = match Sink::open(&config) {
        Ok(sink) => sink,
        Err(error) => {
            eprintln!("refusal: composition_sweep: {error}");
            return ExitCode::from(2);
        }
    };
    if config.in_process {
        work(&config, 0, &mut |line| {
            sink.line(&line);
        });
    } else if let Err(error) = supervise(&config, &args, &mut sink) {
        eprintln!("refusal: composition_sweep: {error}");
        return ExitCode::from(1);
    }
    sink.finish();
    ExitCode::SUCCESS
}
