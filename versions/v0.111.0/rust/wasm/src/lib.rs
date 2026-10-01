//! The browser engine (#3470): `sts-sim` behind a handle-based C ABI.
//!
//! A page loads a canonical fight document once, then steps, undoes and
//! branches by **state id**. `apply` never mutates: it returns a new id and
//! leaves the old one valid, so undo is "go back to the old id" and a branch
//! is "apply something else to it". Both are cheap because `HotState` is
//! copy-on-write (#3412 measured a clone at about 15 ns). `solve` and
//! `replay` speak the existing `sts-sim-exact-solve-v1` wire, so a browser
//! solve and a server check (#3507) use the same request/response shape as
//! the review worker.
//!
//! **Text in, text out.** Every request and response is UTF-8 JSON text, and
//! all parsing happens here. A host must pass canonical documents through as
//! text and never `JSON.parse`/`stringify` them: `serde_json`'s
//! `arbitrary_precision` keeps numbers' exact text, which a JS round trip does
//! not, and the #3412 spike's first host broke every step digest that way.
//!
//! **ABI.** The host copies a request into memory from `sts_alloc(len)`; the
//! call takes ownership of that buffer and frees it. Every call that answers
//! returns the response's byte length, and the response is read from
//! `sts_out_ptr()` before the next call overwrites it. One import is
//! required, `env.sts_now_ms` (monotonic milliseconds, e.g.
//! `performance.now()`), for the exact DFS deadline (`sts_sim::wasm_clock`).
//! `js/sts_engine.mjs` is the reference host.
//!
//! Responses are `{"ok": {...}}` or `{"refusal": {"code": ..., "detail": ...}}`,
//! except `solve`, whose response is an `ExactSolveResponseV1` verbatim, and
//! `entry`, whose response is exactly what `sts-sim entry` prints.
//!
//! **From a save to a fight (#3471).** `entry` builds a fight root from the
//! game's own `current_run.save` (or a capture's embedded run) with the same
//! library call as `sts-sim entry --opening`: the post-`start_combat`
//! `sts-sim-canonical-v2` document, or the entry refusal by name. That
//! document is `load`'s input, so a page can go from a dropped save to a
//! solvable fight without a server (#3411).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::{self, ExactSolveActionV1};
use sts_sim::hot::HotState;

/// Bumped on any incompatible change to an export or a response shape.
pub const API_VERSION: u32 = 1;

/// A live state and the fight catalog it was loaded under.
struct Held {
    catalog: Rc<Catalog>,
    state: HotState,
}

/// Every live state, by id. Ids are never reused within one instance.
#[derive(Default)]
pub struct Engine {
    states: HashMap<u32, Held>,
    next_id: u32,
    events: Vec<engine::Event>,
}

fn refusal(code: &str, detail: impl ToString) -> String {
    json!({ "refusal": { "code": code, "detail": detail.to_string() } }).to_string()
}

fn ok(payload: Value) -> String {
    json!({ "ok": payload }).to_string()
}

fn unknown_state(id: u32) -> String {
    refusal("unknown_state", format!("no live state {id}"))
}

impl Engine {
    fn insert(&mut self, held: Held) -> u32 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("fewer than 2^32 states per instance");
        self.states.insert(id, held);
        id
    }

    fn digest(held: &Held) -> Result<String, String> {
        HotBoundary::try_to_canonical(&held.state, &held.catalog)
            .map(|document| document.differential_digest())
            .map_err(|error| refusal("projection_refused", error))
    }

    /// Load a canonical v2 document through the same gates as `diff-serve`'s
    /// `load`: schema, catalog, boundary, and the engine admission walk.
    pub fn load(&mut self, document: &str) -> String {
        let document: CanonicalStateV2 = match serde_json::from_str(document) {
            Ok(document) => document,
            Err(error) => return refusal("malformed_entry", error),
        };
        if let Err(error) = document.validate_schema() {
            return refusal("malformed_entry", format!("{error:?}"));
        }
        let catalog = match HotBoundary::catalog_from_canonical(&document) {
            Ok(catalog) => catalog,
            Err(error) => return refusal("unrepresentable_state", error),
        };
        let state = match HotBoundary::from_canonical(&document, &catalog) {
            Ok(state) => state,
            Err(error) => return refusal("unrepresentable_state", error),
        };
        if let Err(error) = engine::admit(&document, &state, &catalog) {
            let missing: Vec<String> = error.missing().map(|item| item.to_string()).collect();
            return json!({ "refusal": {
                "code": "not_admitted",
                "detail": error.to_string(),
                "missing": missing,
            } })
            .to_string();
        }
        let held = Held {
            catalog: Rc::new(catalog),
            state,
        };
        let digest = match Self::digest(&held) {
            Ok(digest) => digest,
            Err(response) => return response,
        };
        let over = held.state.history.over;
        let id = self.insert(held);
        ok(json!({ "state": id, "digest": digest, "over": over }))
    }

    /// The legal actions from a state, on the `ExactSolveActionV1` wire.
    pub fn legal(&self, id: u32) -> String {
        let Some(held) = self.states.get(&id) else {
            return unknown_state(id);
        };
        let actions: Vec<ExactSolveActionV1> = engine::legal_actions(&held.state, &held.catalog)
            .iter()
            .copied()
            .map(ExactSolveActionV1::from)
            .collect();
        ok(json!({ "actions": actions }))
    }

    /// Apply one action to a state, returning a **new** state id. The input
    /// state stays live: that is undo and branching.
    pub fn apply(&mut self, id: u32, action: &str) -> String {
        let action: Action = match serde_json::from_str::<ExactSolveActionV1>(action)
            .map_err(|error| error.to_string())
            .and_then(Action::try_from)
        {
            Ok(action) => action,
            Err(detail) => return refusal("malformed_action", detail),
        };
        let Some(held) = self.states.get(&id) else {
            return unknown_state(id);
        };
        self.events.clear();
        let next = match engine::apply_action_into(
            &held.state,
            &held.catalog,
            &action,
            &mut self.events,
        ) {
            Ok(next) => next,
            Err(error) => return refusal("action_refused", error),
        };
        let held = Held {
            catalog: Rc::clone(&held.catalog),
            state: next,
        };
        let digest = match Self::digest(&held) {
            Ok(digest) => digest,
            Err(response) => return response,
        };
        let over = held.state.history.over;
        let events = self.events.len();
        let id = self.insert(held);
        ok(json!({ "state": id, "digest": digest, "over": over, "events": events }))
    }

    /// The state's canonical v2 document and digest. `document` is embedded
    /// as JSON; a host that needs it as text (to pass back into `load` or a
    /// solve request) should use [`Engine::project_text`].
    pub fn project(&self, id: u32) -> String {
        let Some(held) = self.states.get(&id) else {
            return unknown_state(id);
        };
        match HotBoundary::try_to_canonical(&held.state, &held.catalog) {
            Ok(document) => ok(json!({
                "document": serde_json::to_value(&document).expect("canonical state serializes"),
                "digest": document.differential_digest(),
            })),
            Err(error) => refusal("projection_refused", error),
        }
    }

    /// The state's canonical v2 document as bare text, the form `load` and
    /// the `entry` of a solve request take. On failure, a refusal object.
    pub fn project_text(&self, id: u32) -> String {
        let Some(held) = self.states.get(&id) else {
            return unknown_state(id);
        };
        match HotBoundary::try_to_canonical(&held.state, &held.catalog) {
            Ok(document) => serde_json::to_string(&document).expect("canonical state serializes"),
            Err(error) => refusal("projection_refused", error),
        }
    }

    /// Each monster's displayed intent (presentation only), as `diff-serve`'s
    /// `intents`: damage fields only where the engine proves them exact.
    pub fn intents(&self, id: u32) -> String {
        let Some(held) = self.states.get(&id) else {
            return unknown_state(id);
        };
        let intents: Vec<Value> = engine::turn::monster_intents(&held.state, &held.catalog)
            .into_iter()
            .map(|intent| {
                let mut entry = json!({ "uid": intent.uid, "intent": intent.intent });
                if let (Some(damage), Some(hits)) = (intent.damage, intent.hits) {
                    entry["intent_damage"] = json!(damage);
                    entry["intent_hits"] = json!(hits);
                }
                entry
            })
            .collect();
        ok(json!({ "intents": intents }))
    }

    /// Release a state. Other states, including ones derived from it, stay.
    pub fn drop_state(&mut self, id: u32) -> String {
        match self.states.remove(&id) {
            Some(_) => ok(json!({ "dropped": id })),
            None => unknown_state(id),
        }
    }

    /// How many states are live (for a host's leak checks).
    #[must_use]
    pub fn live_states(&self) -> usize {
        self.states.len()
    }
}

/// One `sts-sim-exact-solve-v1` request, answered exactly as `sts-sim
/// exact-solve` answers it (including `memo_budget_bytes`, #3470).
#[must_use]
pub fn solve(request: &str) -> String {
    let mut out = Vec::new();
    exact_solve_v1::serve(request.as_bytes(), &mut out).expect("writing to a Vec cannot fail");
    let mut text = String::from_utf8(out).expect("serde_json writes UTF-8");
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayRequest {
    entry: CanonicalStateV2,
    actions: Vec<ExactSolveActionV1>,
}

/// Replay a claimed line from a root and report its terminal digest: the
/// primitive a server uses to check a client's solve (#3507). It goes through
/// the same solo/boundary/admission gates as `solve`
/// (`exact_solve_v1::replay_actions`). Comparing the digest and objective
/// with the claim is the caller's job.
#[must_use]
pub fn replay(request: &str) -> String {
    let request: ReplayRequest = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => return refusal("malformed_request", error),
    };
    match exact_solve_v1::replay_actions(&request.entry, &request.actions) {
        Ok(final_digest) => ok(json!({ "final_digest": final_digest })),
        Err(error) => json!({ "refusal": error }).to_string(),
    }
}

/// One `entry` request: the `sts-sim entry` flags the census uses, with the
/// file's **text** in place of its path. Absent `opening` builds only the
/// `sts-sim-entry-v1` facts, as the CLI does without `--opening`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryRequest {
    build: String,
    #[serde(default)]
    save: Option<String>,
    #[serde(default)]
    capture_run: Option<String>,
    #[serde(default)]
    encounter: Option<String>,
    #[serde(default)]
    node_type: Option<String>,
    #[serde(default)]
    opening: bool,
    #[serde(default)]
    native_checkpoints: bool,
}

/// Build one fight's entry from a save's text: byte for byte what
/// `sts-sim entry --build B (--save|--capture-run) PATH [--encounter E]
/// [--node-type K] [--opening [--native-checkpoints]]` prints
/// (`entry::cli::main`, minus the file read). An argv-shaped mistake (no or
/// unadmitted build, both or neither input, checkpoints without an opening)
/// is a `{"refusal": ...}`, as the CLI's exit-2 cases are. An *entry* refusal
/// is the CLI's normal refusal document.
#[must_use]
pub fn entry(request: &str) -> String {
    use sts_sim::catalog::GameBuild;
    use sts_sim::entry::opening::OpeningOptions;
    use sts_sim::entry::{EntryInput, EntryRequest as Request};

    let request: EntryRequest = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => return refusal("malformed_request", error),
    };
    let Some(build) = GameBuild::from_str(&request.build) else {
        return refusal(
            "unadmitted_build",
            format!(
                "unadmitted game build {:?}; the admitted set is \
                 versions/admitted_builds.json and grows forward only",
                request.build
            ),
        );
    };
    let input = match (&request.save, &request.capture_run) {
        (Some(text), None) => EntryInput::Save(text),
        (None, Some(text)) => EntryInput::CaptureRun(text),
        (None, None) => return refusal("missing_input", "one of save or capture_run is required"),
        (Some(_), Some(_)) => {
            return refusal(
                "conflicting_inputs",
                "save and capture_run are different input schemas; give exactly one",
            );
        }
    };
    if request.native_checkpoints && !request.opening {
        return refusal(
            "native_checkpoints_need_an_opening",
            "native_checkpoints needs opening",
        );
    }
    let entry_request = Request {
        input,
        encounter_id: request.encounter.as_deref(),
        node_type: request.node_type.as_deref(),
        game_build: build,
        mcr_splice: false,
    };
    let value = if request.opening {
        let options = OpeningOptions {
            mcr_first_checksum: None,
            record_native_checkpoints: request.native_checkpoints,
        };
        sts_sim::entry::build_root_with(&entry_request, &options).to_json()
    } else {
        sts_sim::entry::build(&entry_request).to_json()
    };
    serde_json::to_string(&value).expect("an entry document serializes")
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn answer(text: String) -> u32 {
    OUT.with(|out| {
        let mut out = out.borrow_mut();
        *out = text.into_bytes();
        u32::try_from(out.len()).expect("responses are under 4 GiB")
    })
}

/// Take ownership of a request buffer from [`sts_alloc`].
fn take(ptr: *mut u8, len: u32) -> String {
    // SAFETY: `ptr` came from `sts_alloc(len)`, the host filled `len` bytes,
    // and this call is the buffer's single owner from here on.
    let bytes = unsafe { Vec::from_raw_parts(ptr, len as usize, len as usize) };
    String::from_utf8(bytes).unwrap_or_default()
}

/// Allocate `len` bytes for one request. The next call that takes a request
/// takes ownership of it.
#[unsafe(no_mangle)]
pub extern "C" fn sts_alloc(len: u32) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len as usize);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Where the last response starts; its length was the call's return value.
#[unsafe(no_mangle)]
pub extern "C" fn sts_out_ptr() -> *const u8 {
    OUT.with(|out| out.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_api_version() -> u32 {
    API_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_load(ptr: *mut u8, len: u32) -> u32 {
    let document = take(ptr, len);
    answer(ENGINE.with(|engine| engine.borrow_mut().load(&document)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_legal(id: u32) -> u32 {
    answer(ENGINE.with(|engine| engine.borrow().legal(id)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_apply(id: u32, ptr: *mut u8, len: u32) -> u32 {
    let action = take(ptr, len);
    answer(ENGINE.with(|engine| engine.borrow_mut().apply(id, &action)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_project(id: u32) -> u32 {
    answer(ENGINE.with(|engine| engine.borrow().project(id)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_project_text(id: u32) -> u32 {
    answer(ENGINE.with(|engine| engine.borrow().project_text(id)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_intents(id: u32) -> u32 {
    answer(ENGINE.with(|engine| engine.borrow().intents(id)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_drop(id: u32) -> u32 {
    answer(ENGINE.with(|engine| engine.borrow_mut().drop_state(id)))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_live_states() -> u32 {
    ENGINE.with(|engine| u32::try_from(engine.borrow().live_states()).unwrap_or(u32::MAX))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_solve(ptr: *mut u8, len: u32) -> u32 {
    let request = take(ptr, len);
    answer(solve(&request))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_entry(ptr: *mut u8, len: u32) -> u32 {
    let request = take(ptr, len);
    answer(entry(&request))
}

#[unsafe(no_mangle)]
pub extern "C" fn sts_replay(ptr: *mut u8, len: u32) -> u32 {
    let request = take(ptr, len);
    answer(replay(&request))
}
