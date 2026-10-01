// The reference host for the sts-sim browser engine (#3470; ABI in
// ../src/lib.rs). Works in a browser, a Web Worker and node.
//
// TEXT RULE: pass canonical documents and requests as strings and keep them
// strings. Never JSON.parse and re-stringify a document: the engine keeps
// numbers' exact text (serde_json arbitrary_precision) and a JS round trip
// changes it. This module parses only small responses, for convenience, and
// returns documents (`projectText`) and solve responses (`solveText`) as text.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** Instantiate from bytes (ArrayBuffer/typed array), a Response, or a URL. */
export async function loadEngine(source) {
  const imports = { env: { sts_now_ms: () => performance.now() } };
  let instance;
  if (typeof source === 'string' || source instanceof URL) {
    source = await fetch(source);
  }
  if (typeof Response !== 'undefined' && source instanceof Response) {
    ({ instance } = await WebAssembly.instantiateStreaming(source, imports));
  } else {
    ({ instance } = await WebAssembly.instantiate(source, imports));
  }
  return new Engine(instance.exports);
}

export class EngineRefusal extends Error {
  constructor(refusal) {
    super(`${refusal.code}: ${refusal.detail}`);
    this.code = refusal.code;
    this.refusal = refusal;
  }
}

export class Engine {
  constructor(exports) {
    this.x = exports;
    if (exports.sts_api_version() !== 1) {
      throw new Error(`unsupported engine API ${exports.sts_api_version()}`);
    }
  }

  #read(len) {
    return decoder.decode(new Uint8Array(this.x.memory.buffer, this.x.sts_out_ptr(), len));
  }

  #send(fn, text, ...leading) {
    const bytes = encoder.encode(text);
    const ptr = this.x.sts_alloc(bytes.length);
    new Uint8Array(this.x.memory.buffer, ptr, bytes.length).set(bytes);
    return this.#read(fn(...leading, ptr, bytes.length));
  }

  static #ok(text) {
    const value = JSON.parse(text);
    if (value.refusal) throw new EngineRefusal(value.refusal);
    return value.ok;
  }

  /** Load a canonical document (text). Returns {state, digest, over}. */
  load(documentText) {
    return Engine.#ok(this.#send(this.x.sts_load, documentText));
  }

  /** Legal actions from a state id: ExactSolveActionV1 objects. */
  legal(state) {
    return Engine.#ok(this.#read(this.x.sts_legal(state))).actions;
  }

  /** Apply an action (object or text) to a state. Returns a NEW state id:
   *  {state, digest, over, events}. The old id stays valid: undo/branch. */
  apply(state, action) {
    const text = typeof action === 'string' ? action : JSON.stringify(action);
    return Engine.#ok(this.#send(this.x.sts_apply, text, state));
  }

  /** The canonical document as text, to pass to load() or a solve entry. */
  projectText(state) {
    const text = this.#read(this.x.sts_project_text(state));
    if (text.startsWith('{"refusal"')) throw new EngineRefusal(JSON.parse(text).refusal);
    return text;
  }

  /** {document, digest} parsed, for display only (see TEXT RULE). */
  project(state) {
    return Engine.#ok(this.#read(this.x.sts_project(state)));
  }

  intents(state) {
    return Engine.#ok(this.#read(this.x.sts_intents(state))).intents;
  }

  drop(state) {
    return Engine.#ok(this.#read(this.x.sts_drop(state)));
  }

  liveStates() {
    return this.x.sts_live_states();
  }

  /** An sts-sim-exact-solve-v1 request (text) -> response text. */
  solveText(requestText) {
    return this.#send(this.x.sts_solve, requestText);
  }

  /** Solve from a live state without re-serializing its document in JS. */
  solveState(state, { maxTurns, deadlineMs, memoBudgetBytes } = {}) {
    const options = { protocol: 'sts-sim-exact-solve-v1', max_turns: maxTurns, memo: true };
    if (deadlineMs !== undefined) options.deadline_ms = deadlineMs;
    if (memoBudgetBytes !== undefined) options.memo_budget_bytes = memoBudgetBytes;
    // Splice the document text in verbatim: never parse it (TEXT RULE).
    const head = JSON.stringify(options).slice(0, -1);
    return this.solveText(`${head},"entry":${this.projectText(state)}}`);
  }

  /** A bounded search from a live state (#3419): the review worker's UCT
   *  (`mode` "uct") or random playouts ("random") for `seconds`, out to
   *  `maxTurns`. Returns the report `sts-sim search` prints, parsed. Its
   *  `best` is an achieved line (null when no playout ended), never an
   *  optimum. */
  searchState(state, { mode, seed, seconds, maxTurns, maxPlayouts } = {}) {
    const options = { mode, seed, seconds, max_turns: maxTurns };
    if (maxPlayouts !== undefined) options.max_playouts = maxPlayouts;
    const head = JSON.stringify(options).slice(0, -1);
    return Engine.#ok(this.#send(this.x.sts_search, `${head},"entry":${this.projectText(state)}}`));
  }

  /** Build a fight from a save (#3471). `request` fields: build (e.g.
   *  "v0.111.0"), save or capture_run (the file's TEXT), optional encounter
   *  and node_type, opening, native_checkpoints. Returns the response text,
   *  byte-identical to `sts-sim entry`'s stdout: an `sts-sim-canonical-v2`
   *  document when an opening was built, else an entry or refusal document. */
  entryText(request) {
    return this.#send(this.x.sts_entry, JSON.stringify(request));
  }

  /** From a dropped save to a live state: builds the opening and loads it.
   *  Returns {state, digest, over, document} on success, where document is the
   *  root as TEXT; throws EngineRefusal naming why the save cannot be rooted. */
  loadSave(saveText, { build = 'v0.111.0', encounter, nodeType } = {}) {
    const request = { build, save: saveText, opening: true };
    if (encounter !== undefined) request.encounter = encounter;
    if (nodeType !== undefined) request.node_type = nodeType;
    const text = this.entryText(request);
    // Peek at the schema without re-serializing the document (TEXT RULE).
    const head = JSON.parse(text);
    if (head.schema === 'sts-sim-canonical-v2') {
      return { ...this.load(text), document: text };
    }
    // The three ways a save does not root, each with a stable code:
    if (head.refusal_class) {
      // the save could not be read as an entry (`refusal_class`, e.g.
      // `unsupported_save_schema`);
      throw new EngineRefusal({ code: head.refusal_class, detail: head.refusal?.detail ?? '' });
    }
    if (head.opening && head.opening.built === false) {
      // the entry was built but its opening refused (an unmodeled mechanic);
      throw new EngineRefusal({ code: head.opening.refusal_class, detail: head.opening.detail ?? '' });
    }
    // or the request itself was refused (unadmitted build, ...).
    throw new EngineRefusal(head.refusal ?? { code: 'entry_refused', detail: text.slice(0, 200) });
  }

  /** {entry, actions} (text) -> {final_digest}, or throws EngineRefusal. */
  replayText(requestText) {
    return Engine.#ok(this.#send(this.x.sts_replay, requestText));
  }

  /** Bytes of linear memory; it only grows (terminate the Worker to free). */
  memoryBytes() {
    return this.x.memory.buffer.byteLength;
  }
}
