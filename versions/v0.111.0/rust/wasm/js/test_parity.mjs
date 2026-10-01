// Parity of the BUILT browser engine (#3470): run with
//   node js/test_parity.mjs path/to/sts_sim_wasm.wasm
// Every certified eval fixture's human line is replayed through the module
// and each step digest is compared with the fixture's; then undo/branch,
// refusals, solve and replay are checked. Mirrors tests/api.rs, which runs
// the same API natively. Exits nonzero on any failure.

import { readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadEngine, EngineRefusal } from './sts_engine.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const evalDir = join(here, '..', '..', '..', 'eval');
const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error('usage: node test_parity.mjs MODULE.wasm');
  process.exit(2);
}

const failures = [];
const check = (condition, message) => {
  if (!condition) failures.push(message);
};
const refusalCode = (fn) => {
  try {
    fn();
  } catch (error) {
    if (error instanceof EngineRefusal) return error.code;
    throw error;
  }
  return null;
};

const engine = await loadEngine(readFileSync(wasmPath));
const manifest = JSON.parse(readFileSync(join(evalDir, 'manifest.json'), 'utf8'));
const fixtures = [];
for (const fight of manifest.fights) {
  const dir = join(evalDir, 'fights', fight.id);
  if (!existsSync(join(dir, 'entry.canonical.json')) || !existsSync(join(dir, 'human_line.json'))) {
    continue;
  }
  fixtures.push({
    id: fight.id,
    // Entry stays TEXT (see sts_engine.mjs). The line holds only small
    // integers and digests, so parsing it is safe.
    entry: readFileSync(join(dir, 'entry.canonical.json'), 'utf8'),
    line: JSON.parse(readFileSync(join(dir, 'human_line.json'), 'utf8')),
  });
}

// 1. Every certified human line, every step digest.
let steps = 0;
const started = performance.now();
for (const { id, entry, line } of fixtures) {
  const root = engine.load(entry);
  check(root.digest === line.entry_digest, `${id}: root digest`);
  let current = root.state;
  let over = root.over;
  line.actions.forEach((action, index) => {
    const next = engine.apply(current, action);
    check(next.digest === line.step_digests[index], `${id}: step ${index} digest`);
    engine.drop(current);
    current = next.state;
    over = next.over;
    steps += 1;
  });
  check(over === line.terminal.over, `${id}: terminal over`);
  engine.drop(current);
}
check(engine.liveStates() === 0, 'states leaked after the replay pass');
const replayMs = performance.now() - started;

// 2. Undo and branch: B from the root after A equals B from a fresh root.
let branched = 0;
for (const { id, entry } of fixtures.slice(0, 40)) {
  const root = engine.load(entry).state;
  const actions = engine.legal(root);
  if (actions.length < 2) continue;
  const a = engine.apply(root, actions[0]);
  const b = engine.apply(root, actions[1]);
  const freshB = engine.apply(engine.load(entry).state, actions[1]);
  check(b.digest === freshB.digest, `${id}: branch after undo`);
  check(engine.project(a.state).digest === a.digest, `${id}: A unchanged by B`);
  branched += 1;
}
check(branched >= 20, `only ${branched} fixtures had two root actions`);

// 3. Named refusals.
check(refusalCode(() => engine.load('{not json')) === 'malformed_entry', 'malformed entry');
const first = fixtures[0];
const root = engine.load(first.entry).state;
check(refusalCode(() => engine.legal(root + 1_000_000)) === 'unknown_state', 'unknown state');
check(refusalCode(() => engine.apply(root, { kind: 'fly' })) === 'malformed_action', 'malformed action');
check(refusalCode(() => engine.apply(root, { kind: 'play', uid: 999999 })) === 'action_refused',
  'refused action');

// 4. Solve from a live state, and replay-check its line (text all the way).
const response = JSON.parse(engine.solveState(root, { maxTurns: 1, memoBudgetBytes: 1 << 26 }));
check(['exact', 'refused'].includes(response.status), `solve status ${response.status}`);
if (response.solution) {
  const replayed = engine.replayText(
    `{"entry":${engine.projectText(root)},"actions":${JSON.stringify(response.solution.actions)}}`);
  check(replayed.final_digest === response.solution.final_digest, 'solved line replays');
}
check(typeof response.telemetry?.memo_bytes === 'number', 'solve telemetry carries memo_bytes');

// 5. From a save to a fight (#3471): the committed save/capture pair roots
// through `loadSave`, matching the root built with the pair's explicit
// encounter and node; a save the engine cannot read is refused by name.
const pairDir = join(here, '..', '..', 'fixtures', 'capture_run_pair_v1');
const pair = JSON.parse(readFileSync(join(pairDir, 'pair.json'), 'utf8'));
const saveText = readFileSync(join(pairDir, 'save.json'), 'utf8');
const fromSave = engine.loadSave(saveText);
const explicit = engine.load(engine.entryText({
  build: 'v0.111.0', save: saveText, encounter: pair.encounter_id,
  node_type: pair.node_type, opening: true,
}));
check(fromSave.digest === explicit.digest, 'loadSave root matches the explicit-encounter root');
check(engine.legal(fromSave.state).length > 0, 'a rooted save has legal actions');
check(refusalCode(() => engine.loadSave('{"schema_version": 3}')) === 'unsupported_save_schema',
  'an unreadable save is refused by name');
check(refusalCode(() => engine.loadSave(saveText, { build: 'v0.110.1' })) === 'unadmitted_build',
  'an unadmitted build is refused by name');

const summary = {
  fixtures: fixtures.length,
  steps,
  replay_ms: Math.round(replayMs),
  branched,
  solve_status: response.status,
  save_root: fromSave.digest.slice(0, 12),
  memory_bytes: engine.memoryBytes(),
  failures: failures.length,
};
console.log(`wasm engine parity: ${JSON.stringify(summary)}`);
if (fixtures.length < 500 || steps < 10_000) failures.push('fixture tree unexpectedly small');
if (failures.length) {
  for (const failure of failures.slice(0, 20)) console.error(`FAIL ${failure}`);
  process.exit(1);
}
