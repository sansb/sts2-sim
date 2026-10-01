//! The browser engine API (#3470), exercised natively. `js/test_parity.mjs`
//! runs the same checks through the built wasm module.

use std::path::PathBuf;

use serde_json::{Value, json};
use sts_sim_wasm::{Engine, replay, solve};

/// The certified eval fixture tree beside this crate's build directory.
fn eval_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval")
}

/// Rooted fixtures with a recorded human line: (id, entry text, line).
fn fixtures() -> Vec<(String, String, Value)> {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(eval_dir().join("manifest.json")).expect("eval manifest"),
    )
    .expect("manifest parses");
    manifest["fights"]
        .as_array()
        .expect("fights")
        .iter()
        .filter_map(|fight| {
            let id = fight["id"].as_str()?.to_owned();
            let dir = eval_dir().join("fights").join(&id);
            let entry = std::fs::read_to_string(dir.join("entry.canonical.json")).ok()?;
            let line = std::fs::read_to_string(dir.join("human_line.json")).ok()?;
            Some((
                id,
                entry,
                serde_json::from_str(&line).expect("human line parses"),
            ))
        })
        .collect()
}

fn ok(response: &str) -> Value {
    let value: Value = serde_json::from_str(response).expect("response is JSON");
    assert!(value.get("ok").is_some(), "expected ok, got {response}");
    value["ok"].clone()
}

fn refusal_code(response: &str) -> String {
    let value: Value = serde_json::from_str(response).expect("response is JSON");
    value["refusal"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal, got {response}"))
        .to_owned()
}

fn state(response: &str) -> u32 {
    ok(response)["state"].as_u64().expect("state id") as u32
}

#[test]
fn every_certified_human_line_replays_with_its_step_digests() {
    let fixtures = fixtures();
    assert!(fixtures.len() > 500, "found {} fixtures", fixtures.len());
    let mut engine = Engine::default();
    let mut steps = 0;
    for (fid, entry, line) in &fixtures {
        let loaded = ok(&engine.load(entry));
        assert_eq!(loaded["digest"], line["entry_digest"], "{fid} root digest");
        let mut current = loaded["state"].as_u64().unwrap() as u32;
        let mut over = loaded["over"].clone();
        let digests = line["step_digests"].as_array().expect("step digests");
        for (index, action) in line["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .enumerate()
        {
            let next = ok(&engine.apply(current, &action.to_string()));
            assert_eq!(next["digest"], digests[index], "{fid} step {index}");
            engine.drop_state(current);
            current = next["state"].as_u64().unwrap() as u32;
            over = next["over"].clone();
            steps += 1;
        }
        assert_eq!(over, line["terminal"]["over"], "{fid} terminal");
        engine.drop_state(current);
    }
    assert_eq!(engine.live_states(), 0, "every state was dropped");
    assert!(steps > 10_000, "{steps} steps");
}

#[test]
fn undo_and_branch_are_digest_equal_to_a_fresh_replay() {
    // For a fixture with at least two legal actions at the root: apply A,
    // go back to the root id (undo), apply B. B's state must equal B applied
    // to a freshly loaded root, and A's state must be untouched by B.
    let mut engine = Engine::default();
    let mut checked = 0;
    for (fid, entry, _) in fixtures().iter().take(40) {
        let root = state(&engine.load(entry));
        let actions = ok(&engine.legal(root))["actions"]
            .as_array()
            .unwrap()
            .clone();
        if actions.len() < 2 {
            continue;
        }
        let a = ok(&engine.apply(root, &actions[0].to_string()));
        let b = ok(&engine.apply(root, &actions[1].to_string()));
        let fresh_root = state(&engine.load(entry));
        let fresh_b = ok(&engine.apply(fresh_root, &actions[1].to_string()));
        assert_eq!(b["digest"], fresh_b["digest"], "{fid}: branch after undo");
        let a_again = ok(&engine.project(a["state"].as_u64().unwrap() as u32));
        assert_eq!(a_again["digest"], a["digest"], "{fid}: A unchanged by B");
        assert_eq!(
            ok(&engine.project(root))["digest"],
            ok(&engine.project(fresh_root))["digest"]
        );
        checked += 1;
    }
    assert!(
        checked >= 20,
        "only {checked} fixtures had two root actions"
    );
}

#[test]
fn project_text_loads_back_to_the_same_state() {
    let (_, entry, line) = fixtures().into_iter().next().unwrap();
    let mut engine = Engine::default();
    let root = state(&engine.load(&entry));
    let first = line["actions"][0].to_string();
    let next = ok(&engine.apply(root, &first));
    let text = engine.project_text(next["state"].as_u64().unwrap() as u32);
    let reloaded = ok(&engine.load(&text));
    assert_eq!(reloaded["digest"], next["digest"]);
}

#[test]
fn refusals_are_named_and_leave_state_untouched() {
    let (_, entry, _) = fixtures().into_iter().next().unwrap();
    let mut engine = Engine::default();
    assert_eq!(refusal_code(&engine.load("{not json")), "malformed_entry");
    assert_eq!(refusal_code(&engine.load("{}")), "malformed_entry");
    let root = state(&engine.load(&entry));
    assert_eq!(refusal_code(&engine.legal(root + 99)), "unknown_state");
    assert_eq!(
        refusal_code(&engine.apply(root, r#"{"kind":"fly"}"#)),
        "malformed_action"
    );
    // A uid no card has: well-formed, but the engine refuses it.
    assert_eq!(
        refusal_code(&engine.apply(root, r#"{"kind":"play","uid":999999}"#)),
        "action_refused"
    );
    assert_eq!(engine.live_states(), 1, "refused applies create no state");
    ok(&engine.drop_state(root));
    assert_eq!(refusal_code(&engine.drop_state(root)), "unknown_state");
    assert_eq!(refusal_code(&engine.project(root)), "unknown_state");
}

#[test]
fn intents_answer_for_a_loaded_state() {
    let (_, entry, _) = fixtures().into_iter().next().unwrap();
    let mut engine = Engine::default();
    let root = state(&engine.load(&entry));
    let intents = ok(&engine.intents(root));
    assert!(!intents["intents"].as_array().unwrap().is_empty());
}

/// The exact-solve corpus's authority row and its solution, via `solve`.
fn corpus_solution() -> (Value, Value) {
    let corpus: Value =
        serde_json::from_str(include_str!("../../fixtures/exact_solve_corpus_v1.json"))
            .expect("corpus parses");
    let entry = corpus["rows"][0]["entry"].clone();
    let request = json!({
        "protocol": "sts-sim-exact-solve-v1",
        "entry": entry,
        "max_turns": 4,
        "memo": true,
    });
    let response: Value = serde_json::from_str(&solve(&request.to_string())).unwrap();
    assert_eq!(response["status"], "exact", "{response}");
    (entry, response["solution"].clone())
}

#[test]
fn solve_is_the_exact_solve_v1_wire_and_replay_checks_a_claimed_line() {
    let (entry, solution) = corpus_solution();
    // A genuine line replays to its claimed digest.
    let genuine = json!({ "entry": entry, "actions": solution["actions"] });
    assert_eq!(
        ok(&replay(&genuine.to_string()))["final_digest"],
        solution["final_digest"]
    );

    // Tampering is caught by digest or refused outright, never accepted.
    let actions = solution["actions"].as_array().unwrap().clone();
    let digest_of = |actions: &[Value]| -> Option<Value> {
        let response: Value = serde_json::from_str(&replay(
            &json!({ "entry": entry, "actions": actions }).to_string(),
        ))
        .unwrap();
        response.get("ok").map(|ok| ok["final_digest"].clone())
    };
    let claimed = Some(solution["final_digest"].clone());
    assert_ne!(
        digest_of(&actions[..actions.len() - 1]),
        claimed,
        "one action short"
    );
    let mut extra = actions.clone();
    extra.push(json!({ "kind": "end" }));
    assert_ne!(digest_of(&extra), claimed, "one action too many");
    let mut wrong = actions.clone();
    wrong[0] = json!({ "kind": "end" });
    assert_ne!(digest_of(&wrong), claimed, "a different first action");

    assert_eq!(refusal_code(&replay("{}")), "malformed_request");
    // The solve wire refuses malformed input as a response, never a panic.
    let malformed: Value = serde_json::from_str(&solve("{not json")).unwrap();
    assert_eq!(malformed["status"], "refused");
}

#[test]
fn solve_honours_the_memo_budget() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../fixtures/exact_solve_corpus_v1.json")).unwrap();
    let request = json!({
        "protocol": "sts-sim-exact-solve-v1",
        "entry": corpus["rows"][0]["entry"],
        "max_turns": 1,
        "memo": true,
        "memo_budget_bytes": 0,
    });
    let response: Value = serde_json::from_str(&solve(&request.to_string())).unwrap();
    assert_eq!(response["status"], "exact");
    assert_eq!(response["telemetry"]["memo_budget_reached"], json!(true));
    assert_eq!(response["telemetry"]["memo_bytes"], json!(0));
}

/// #3471: `entry` answers byte for byte what `sts-sim entry` prints, for the
/// committed save/capture pair, in every mode the census uses.
#[test]
fn entry_is_byte_identical_to_the_cli() {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/capture_run_pair_v1");
    let meta: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("pair.json")).unwrap())
            .unwrap();
    let encounter = meta["encounter_id"].as_str().unwrap();
    let node_type = meta["node_type"].as_str().unwrap();
    let mut rooted = 0;
    for (flag, field, file) in [
        ("--save", "save", "save.json"),
        ("--capture-run", "capture_run", "capture_run.json"),
    ] {
        let path = fixtures.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        for (opening, checkpoints) in [(false, false), (true, false), (true, true)] {
            let mut argv: Vec<String> = [
                "--build",
                "v0.111.0",
                flag,
                path.to_str().unwrap(),
                "--encounter",
                encounter,
                "--node-type",
                node_type,
            ]
            .iter()
            .map(|item| (*item).to_owned())
            .collect();
            if opening {
                argv.push("--opening".to_owned());
            }
            if checkpoints {
                argv.push("--native-checkpoints".to_owned());
            }
            let cli = sts_sim::entry::cli::main(&argv).expect("the CLI accepts these flags");
            let mut request = json!({
                "build": "v0.111.0",
                field: text,
                "encounter": encounter,
                "node_type": node_type,
            });
            if opening {
                request["opening"] = json!(true);
            }
            if checkpoints {
                request["native_checkpoints"] = json!(true);
            }
            let wasm_api = sts_sim_wasm::entry(&request.to_string());
            assert_eq!(
                wasm_api, cli,
                "{flag} opening={opening} checkpoints={checkpoints}"
            );
            if opening && !checkpoints {
                // The rooted document is load's input, verbatim.
                let mut engine = Engine::default();
                ok(&engine.load(&wasm_api));
                rooted += 1;
            }
        }
    }
    assert_eq!(rooted, 2, "both inputs root and load");
}

#[test]
fn entry_refuses_argv_shaped_mistakes_by_name() {
    let save = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/capture_run_pair_v1/save.json"),
    )
    .unwrap();
    for (request, code) in [
        (json!({ "save": save }), "malformed_request"),
        (
            json!({ "build": "v0.110.1", "save": save }),
            "unadmitted_build",
        ),
        (json!({ "build": "v0.111.0" }), "missing_input"),
        (
            json!({ "build": "v0.111.0", "save": save, "capture_run": save }),
            "conflicting_inputs",
        ),
        (
            json!({ "build": "v0.111.0", "save": save, "native_checkpoints": true }),
            "native_checkpoints_need_an_opening",
        ),
        (
            json!({ "build": "v0.111.0", "save": save, "mcr": "x" }),
            "malformed_request",
        ),
    ] {
        assert_eq!(
            refusal_code(&sts_sim_wasm::entry(&request.to_string())),
            code,
            "{request}"
        );
    }
    // A save that is not a save is an entry refusal document, not a panic.
    let answer: Value = serde_json::from_str(&sts_sim_wasm::entry(
        r#"{"build":"v0.111.0","save":"{}","opening":true}"#,
    ))
    .unwrap();
    assert!(
        answer.get("refusal").is_some() || answer.get("opening").is_some(),
        "{answer}"
    );
}
