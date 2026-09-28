//! Static guard for #2472: no engine state write may live inside a
//! `debug_assert!` argument.
//!
//! `[profile.release]` leaves Cargo's `debug-assertions = false` default in
//! place, and the macro expands to *nothing* — argument included — when they
//! are off. So `debug_assert!(state.fanouts.set_kusarigama(next))` performs
//! the write in every `cargo test` build and silently performs none in
//! `target/release/sts-sim`, which is the binary `STS_SIM_EXACT_SOLVER` and
//! every census/eval driver run. #2472 found 42 such writes; the corpus census
//! saw six counter-relic digest divergences and one Tender Goop refusal from
//! them, and no lane could see it because every lane builds dev.
//!
//! `clippy::debug_assert_with_mut_call` (enabled as `deny` in `Cargo.toml`) is
//! the other half of the guard, but it is not sufficient on its own: measured
//! on the unfixed tree it flagged 13 of the 42 sites — only those whose
//! receiver is a direct `&mut` binding (`state.set_kunai(..)`,
//! `monster.set_waterfall_steam_eruption_damage(..)`). It saw through none of
//! the 29 written as `state.fanouts.set_x(..)` / `ctx.state.fanouts.set_x(..)`,
//! which is the majority shape in this crate. This test covers the field-
//! projection shape by deriving the mutating-method set from the crate's own
//! declarations rather than matching a name pattern.
//!
//! The behavioural half of the guard is the `assertless` profile (see
//! `Cargo.toml`): dev codegen with `debug-assertions = false`. On the unfixed
//! tree `cargo test --profile assertless --lib` failed 39 existing unit tests.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One `debug_assert*!` invocation: where it is, and its argument text.
#[derive(Debug)]
struct Site {
    file: String,
    line: usize,
    argument: String,
}

/// Strip comments, string and char literals so the scanners below cannot be
/// fooled by a `debug_assert!` spelled inside a doc comment or a message.
///
/// Replacement is length-preserving (each removed byte becomes a space, and
/// newlines are kept) so byte offsets still map to source lines.
fn blank_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            out[i] = b'\n';
            i += 1;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let mut depth = 1;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'\n' {
                    out[i] = b'\n';
                }
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if c == b'r' && matches!(bytes.get(i + 1), Some(&b'"') | Some(&b'#')) {
            // Raw string: r"", r#".."#, r##".."##, …
            let mut hashes = 0;
            let mut j = i + 1;
            while bytes.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) != Some(&b'"') {
                out[i] = c;
                i += 1;
                continue;
            }
            j += 1;
            while j < bytes.len() {
                if bytes[j] == b'\n' {
                    out[j] = b'\n';
                }
                if bytes[j] == b'"' {
                    let closing = (1..=hashes).all(|k| bytes.get(j + k) == Some(&b'#'));
                    if closing {
                        j += hashes + 1;
                        break;
                    }
                }
                j += 1;
            }
            i = j;
        } else if c == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\n' {
                    out[i] = b'\n';
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if c == b'\'' && bytes.get(i + 1) == Some(&b'\\') {
            // Escaped char literal; a lifetime can never start with a backslash.
            i += 2;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1;
        } else if c == b'\'' && bytes.get(i + 2) == Some(&b'\'') {
            // Plain char literal `'x'`. A lifetime is `'a` followed by a
            // non-quote, so this cannot swallow one.
            i += 3;
        } else {
            out[i] = c;
            i += 1;
        }
    }
    String::from_utf8(out).expect("length-preserving ASCII blanking keeps UTF-8 validity")
}

/// The byte offset just past the `(` that opens `open`'s delimiter, and the
/// offset of its matching `)`.
fn balanced(source: &str, open: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let start = source[open..].find('(')? + open;
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start + 1, offset));
                }
            }
            _ => {}
        }
    }
    None
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

/// Every `debug_assert!` / `debug_assert_eq!` / `debug_assert_ne!` argument in
/// `source`, with comments and literals already blanked.
fn debug_assert_sites(file: &str, source: &str) -> Vec<Site> {
    let clean = blank_comments_and_literals(source);
    let mut sites = Vec::new();
    let mut search = 0;
    while let Some(found) = clean[search..].find("debug_assert") {
        let at = search + found;
        search = at + "debug_assert".len();
        // Must be a whole identifier: `fn my_debug_assert(` is not this macro.
        if at > 0 && {
            let prev = clean.as_bytes()[at - 1];
            prev == b'_' || prev.is_ascii_alphanumeric()
        } {
            continue;
        }
        let rest = &clean[at..];
        let Some(bang) = rest.find('!') else { continue };
        let name = &rest["debug_assert".len()..bang];
        if !matches!(name, "" | "_eq" | "_ne") {
            continue;
        }
        // Only a macro call, never `debug_assert!` inside another identifier.
        if rest[bang + 1..].trim_start().starts_with('(')
            && let Some((from, to)) = balanced(&clean, at)
        {
            sites.push(Site {
                file: file.to_string(),
                line: line_of(&clean, at),
                argument: clean[from..to].to_string(),
            });
            search = to;
        }
    }
    sites
}

/// Every method in `source` declared with a `&mut self` receiver.
///
/// Derived, not pattern-matched: the defect is not "a name starting with
/// `set_`", it is "a call that mutates". A new mutating method is covered the
/// day it is declared.
fn mutating_method_names(source: &str) -> BTreeSet<String> {
    let clean = blank_comments_and_literals(source);
    let mut names = BTreeSet::new();
    let mut search = 0;
    while let Some(found) = clean[search..].find("fn ") {
        let at = search + found;
        search = at + 3;
        if at > 0 && {
            let prev = clean.as_bytes()[at - 1];
            prev == b'_' || prev.is_ascii_alphanumeric()
        } {
            continue;
        }
        let after = &clean[at + 3..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some((from, to)) = balanced(&clean, at + 3) else {
            continue;
        };
        let params = &clean[from..to];
        let first = params.split(',').next().unwrap_or_default();
        let normalized = first.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.starts_with("&mut self") || normalized.starts_with("mut self") {
            names.insert(name);
        }
        search = to;
    }
    names
}

/// Call names appearing in an expression: the identifier before each `(`.
fn called_names(expression: &str) -> BTreeSet<String> {
    let bytes = expression.as_bytes();
    let mut names = BTreeSet::new();
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte != b'(' {
            continue;
        }
        let head = expression[..offset].trim_end();
        let name: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !name.is_empty() && !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            names.insert(name);
        }
    }
    names
}

fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("the crate source tree is readable");
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                into.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    assert!(
        files.len() > 20,
        "expected the whole src/ tree, got {files:?}"
    );
    files
}

/// Sites this guard tolerates. **It must stay empty.** A site belongs in the
/// source as `let ok = …; debug_assert!(ok);`, or — where the `false` return
/// means the incoming document is unrepresentable rather than that the engine
/// computed an impossible value — as a typed refusal. It never belongs here.
const ALLOWLIST: &[(&str, u32)] = &[];

#[test]
fn no_debug_assert_argument_mutates_engine_state() {
    let files = source_files();
    let sources: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            let name = path
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
                .expect("every scanned file is under src/")
                .to_string_lossy()
                .into_owned();
            (
                name,
                std::fs::read_to_string(path).expect("readable source"),
            )
        })
        .collect();

    let mut mutating = BTreeSet::new();
    for (_, source) in &sources {
        mutating.extend(mutating_method_names(source));
    }
    assert!(
        mutating.contains("set_kusarigama") && mutating.contains("set_tender_state"),
        "the receiver derivation must find the #2472 setters"
    );

    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for (name, source) in &sources {
        for site in debug_assert_sites(name, source) {
            scanned += 1;
            if ALLOWLIST.contains(&(site.file.as_str(), site.line as u32)) {
                continue;
            }
            let called = called_names(&site.argument);
            let mutates: Vec<&String> = called.intersection(&mutating).collect();
            if !mutates.is_empty() {
                violations.push(format!(
                    "{}:{} calls {:?} inside debug_assert!: `{}`",
                    site.file, site.line, mutates, site.argument
                ));
            }
        }
    }
    assert!(
        scanned > 50,
        "the scanner found only {scanned} debug_assert sites; it is not looking at the crate"
    );
    assert!(
        violations.is_empty(),
        "a release build drops these writes (#2472). Hoist each into a `let`:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_allowlist_is_empty() {
    assert!(
        ALLOWLIST.is_empty(),
        "#2472's contract is an empty allowlist; {} entries remain",
        ALLOWLIST.len()
    );
}

/// The scanner must be able to FAIL. A guard that has never been shown to
/// detect its own defect is not evidence (PORT_PLAN §4's `--self-test`
/// discipline, applied to a static check).
#[test]
fn the_scanner_detects_the_defect_shapes_and_ignores_pure_ones() {
    let declarations = r#"
        impl Fanouts {
            pub(crate) fn set_kusarigama(&mut self, value: u8) -> bool { true }
            pub(crate) fn kusarigama(&self) -> u8 { 0 }
            pub(crate) fn records_are_exact(&self) -> bool { true }
        }
        fn helper_is_exact(state: &HotState) -> bool { true }
    "#;
    let mutating = mutating_method_names(declarations);
    assert!(mutating.contains("set_kusarigama"));
    assert!(!mutating.contains("kusarigama"));
    assert!(!mutating.contains("records_are_exact"));
    assert!(!mutating.contains("helper_is_exact"));

    // The direct-receiver shape clippy catches.
    let direct = "fn f() { debug_assert!(state.set_kusarigama(next)); }";
    // The field-projection shape clippy does NOT catch — the #2472 majority.
    let projected = "fn f() { debug_assert!(state.fanouts.set_kusarigama(next)); }";
    // Wrapped in an outer expression, and spread over several lines.
    let wrapped = "fn f() {\n    debug_assert_eq!(\n        ctx.state.fanouts.set_kusarigama(n),\n        true\n    );\n}";
    for (label, source) in [
        ("direct", direct),
        ("projected", projected),
        ("wrapped", wrapped),
    ] {
        let sites = debug_assert_sites("synthetic.rs", source);
        assert_eq!(sites.len(), 1, "{label}: one site");
        assert!(
            called_names(&sites[0].argument).contains("set_kusarigama"),
            "{label}: the mutating call must be seen in `{}`",
            sites[0].argument
        );
    }

    // The fixed shape, and genuinely pure arguments, must all pass.
    let clean = concat!(
        "fn f() {\n",
        "    let written = state.fanouts.set_kusarigama(next);\n",
        "    debug_assert!(written);\n",
        "    debug_assert!(state.fanouts.kusarigama() < 3);\n",
        "    debug_assert!(helper_is_exact(state));\n",
        "    debug_assert_eq!(physical.len(), 6);\n",
        "}",
    );
    let mutating_in_clean: Vec<_> = debug_assert_sites("synthetic.rs", clean)
        .iter()
        .filter(|site| !called_names(&site.argument).is_disjoint(&mutating))
        .map(|site| site.argument.clone())
        .collect();
    assert!(
        mutating_in_clean.is_empty(),
        "the fixed shape must not be flagged: {mutating_in_clean:?}"
    );

    // A macro name mentioned in a comment or a string is not a site.
    let decoys = concat!(
        "fn f() {\n",
        "    // debug_assert!(state.fanouts.set_kusarigama(next));\n",
        "    let message = \"debug_assert!(state.fanouts.set_kusarigama(next))\";\n",
        "    /* debug_assert!(state.fanouts.set_kusarigama(next)); */\n",
        "}",
    );
    assert!(
        debug_assert_sites("synthetic.rs", decoys).is_empty(),
        "comments and string literals must not register as sites"
    );

    // A function whose NAME contains the macro's name is not a site either.
    let lookalike = "fn my_debug_assert_helper() { other.set_kusarigama(1); }";
    assert!(debug_assert_sites("synthetic.rs", lookalike).is_empty());
}
