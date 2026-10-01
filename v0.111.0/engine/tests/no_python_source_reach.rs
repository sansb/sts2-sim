//! No crate code or test may compile against, or shell out to, a Python source
//! (#2999, under #2827).
//!
//! #2827 item F deletes the frozen Python simulator (`combat_sim.py`,
//! `solve_fight.py` and the `content/` package). A crate file that
//! `include_str!`s one of those files stops the whole target compiling once it
//! is gone, not just the one test. #2998 added exactly that reach
//! (`run_counters/tests.rs` read `solver/solve_fight.py`), and the #3000
//! review found it only by moving the simulator away by hand. #2827 item D had
//! already moved the cargo tests that shelled out to `python3` onto frozen
//! data. This guard makes both shapes a named failure at the PR that
//! reintroduces them:
//!
//! * an `include_str!` / `include_bytes!` whose path literal ends in `.py`;
//! * a `Command::new("python…")`.
//!
//! JSON tables under `solver/` are not Python sources and stay allowed. Freeze
//! the Python half as crate data instead (`tools/frozen_oracle_data.py` pins
//! it).

use std::path::{Path, PathBuf};

const ROOTS: [&str; 4] = ["src", "tests", "examples", "benches"];

/// Every `.rs` file under the crate's source roots, plus `build.rs`.
fn rust_files() -> Vec<PathBuf> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = ROOTS
        .iter()
        .map(|root| crate_dir.join(root))
        .filter(|path| path.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let build = crate_dir.join("build.rs");
    if build.is_file() {
        out.push(build);
    }
    out.sort();
    out
}

/// The Python reaches in one file's text, as `(line, description)`.
fn python_reaches(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for macro_name in ["include_str!", "include_bytes!"] {
        let mut from = 0;
        while let Some(at) = text[from..].find(macro_name) {
            let start = from + at + macro_name.len();
            from = start;
            // The argument: skip whitespace and the opening parenthesis, then
            // read the first string literal, which may start on a later line.
            let rest = text[start..].trim_start();
            let Some(rest) = rest.strip_prefix('(') else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('"') else {
                continue;
            };
            let Some(end) = rest.find('"') else {
                continue;
            };
            let literal = &rest[..end];
            if literal.ends_with(".py") {
                let line = text[..start].matches('\n').count() + 1;
                found.push((line, format!("{macro_name}(\"{literal}\")")));
            }
        }
    }
    let spawn = "Command::new(";
    let mut from = 0;
    while let Some(at) = text[from..].find(spawn) {
        let start = from + at + spawn.len();
        from = start;
        let rest = text[start..].trim_start();
        if let Some(program) = rest.strip_prefix('"') {
            let program = &program[..program.find('"').unwrap_or(program.len())];
            if program.starts_with("python") {
                let line = text[..start].matches('\n').count() + 1;
                found.push((line, format!("{spawn}\"{program}\")")));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_crate_file_reaches_a_python_source() {
    let this = Path::new(file!()).file_name().unwrap().to_owned();
    let files = rust_files();
    assert!(
        files.len() > 100,
        "the walk found only {} files",
        files.len()
    );
    let mut offenders = Vec::new();
    for path in files {
        if path.file_name() == Some(this.as_os_str()) {
            continue; // the controls below spell the patterns out
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (line, what) in python_reaches(&text) {
            offenders.push(format!("{}:{line}: {what}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "crate code reaches a Python source; freeze the Python half as crate \
         data instead (tools/frozen_oracle_data.py):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scanner_finds_each_shape_it_forbids() {
    let reaches = python_reaches(concat!(
        "let a = include_str!(\"../../../python/solve_fight.py\");\n",
        "let b = include_bytes!(\n    \"../python/combat_sim.py\"\n);\n",
        "std::process::Command::new(\"python3\").arg(\"x\");\n",
        "Command::new( \"python3.12\" );\n",
    ));
    assert_eq!(
        reaches,
        vec![
            (
                1,
                "include_str!(\"../../../python/solve_fight.py\")".to_owned()
            ),
            (2, "include_bytes!(\"../python/combat_sim.py\")".to_owned()),
            (5, "Command::new(\"python3\")".to_owned()),
            (6, "Command::new(\"python3.12\")".to_owned()),
        ]
    );
}

#[test]
fn the_scanner_allows_data_files_and_other_programs() {
    assert!(
        python_reaches(concat!(
            "include_str!(\"../../../python/card_templates_raw.json\");\n",
            "include_str!(\"boundary.rs\");\n",
            "Command::new(\"cargo\");\n",
            "Command::new(binary);\n",
            "// python3 tools/gen_roster_pins.py --relabel\n",
        ))
        .is_empty()
    );
}
