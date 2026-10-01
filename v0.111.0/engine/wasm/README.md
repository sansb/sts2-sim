# sts-sim-wasm: the browser engine (#3470)

The certified `sts-sim` engine, compiled to `wasm32-unknown-unknown` behind a
small handle-based C ABI, so a page can step a fight, undo, branch and run
exact solves locally. `src/lib.rs` documents the ABI. `js/sts_engine.mjs` is
the reference host, and it works in a page, a Web Worker, or node.

```js
import { loadEngine } from './sts_engine.mjs';
const engine = await loadEngine('/engines/<fingerprint>/sts_sim_wasm.wasm');
const root = engine.load(documentText);          // {state, digest, over}
const legal = engine.legal(root.state);          // ExactSolveActionV1[]
const next = engine.apply(root.state, legal[0]); // NEW id; root.state stays valid
const solve = JSON.parse(engine.solveState(next.state,
  { maxTurns: 2, deadlineMs: 5000, memoBudgetBytes: 256 << 20 }));
```

`searchState` runs the bounded search the review worker runs
(`sts_sim::search`, `sts-sim search`): UCT or random playouts for a number of
seconds. The site's solve (`src/solvecore.mjs`) is that search and nothing
else, with the worker's settings (#3419). The exact search's best-so-far line
at a deadline is weak, because it has only walked the first few branches.

```js
const report = engine.searchState(next.state, { mode: 'uct', seed: 2, seconds: 5, maxTurns: 20 });
report.best; // { won, combat_hp, turn, actions, final_digest } or null
```

## Rules for hosts

- **Text rule.** Keep canonical documents and requests as strings. Never
  `JSON.parse` a document and `stringify` it back: the engine keeps numbers'
  exact text (`arbitrary_precision`), and a JS round trip broke every step
  digest in the #3412 spike. `projectText` and `solveState` keep documents as
  text.
- **One Worker per solve, terminated after.** wasm linear memory never
  shrinks. `memo_budget_bytes` bounds the memo, and with hashed memo keys
  (#3516) a 64 MiB budget did not bind on any certified exact fixture.
- **A solve result is a claim.** Only `status: "exact"` is a proven optimum.
  A `deadline` result is a best-so-far line. A client result that is stored
  or shared is re-checked on the server with `replay` (#3507).
- **Per-build modules.** Each game build's module is served at an immutable
  fingerprinted URL (#3506). The fingerprint is the module's sha256, which
  `tools/wasm_build.py` prints.

## Build and check

```bash
python3 sim/v0.111.0/engine/tools/wasm_build.py
```

This is the `rust port` step `wasm32 library build`. It needs the pinned
rustup toolchain with the wasm32 target, `wasm-opt` (binaryen) and `node`,
and it refuses with the install command when any of them is missing. It
writes `target/wasm32-rustup/sts_sim_wasm.wasm`, then replays every certified
eval fixture through that module and compares every step digest.

`Cargo.lock` must pin exactly the engine crate's versions (the build refuses
on drift). After the engine's lockfile moves, run
`cp ../Cargo.lock Cargo.lock` here, then any cargo command without `--locked`.

## From a save (#3471)

`entry` builds a fight from the game's own save text, with the same library
call as `sts-sim entry --opening`. `loadSave` in `js/sts_engine.mjs` goes
straight from a dropped `current_run.save` to a live, solvable state (#3411).

```js
const root = engine.loadSave(await file.text());   // encounter and node read from the save
```

A save that can't be rooted throws an `EngineRefusal` with a stable code:
`refusal_class` for an unreadable entry (e.g. `unsupported_save_schema`),
the opening's `refusal_class` for an unmodeled mechanic, or the request's
own code (e.g. `unadmitted_build`).

Measured 2026-09-30 for #3471 on this Mac:

- **Parity:** the capture-corpus census run through `js/sts-sim-entry-parity`
  answered every `entry` call with wasm while byte-comparing it against the
  native binary. All **1,201/1,201** calls were byte-identical. The census
  rows and summary equal a native run (1,206 fights, 1,199 rooted, 1,199/1,199
  opening checkpoints matching, 1,196 certified). `tests/api.rs` pins byte
  equality with `entry::cli::main` for the committed save/capture pair in
  every mode, and `js/test_parity.mjs` checks `loadSave` in CI.
- **Latency** (`examples/entry_bench.rs` against `js/entry_bench.mjs`, the same
  1,201 requests, best of 3, two interleaved rounds): wasm p50 0.60 ms, p99
  8.1 ms, max 14.5 ms, which is 1.18–1.35× native. In Chrome, the first
  `loadSave` in a fresh page took 114 ms (lazy compilation); a one-turn exact
  solve from it took 259 ms.
- **Size:** against `main`'s module, `entry` adds 601 KB raw (+17%) and
  165 KB brotli. The module is 4.14 MB raw and 1.16 MB brotli.
