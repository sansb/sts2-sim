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

// 4b. The review worker's bounded search from the same state (#3419): both
// modes answer with the CLI report, and a found line replays to its digest.
for (const mode of ['uct', 'random']) {
  const report = engine.searchState(root, { mode, seed: 2, seconds: 600, maxTurns: 12, maxPlayouts: 200 });
  check(report.mode === mode && report.playouts === 200, `${mode} search ran its playouts`);
  if (report.best) {
    const replayed = engine.replayText(
      `{"entry":${engine.projectText(root)},"actions":${JSON.stringify(report.best.actions)}}`);
    check(replayed.final_digest === report.best.final_digest, `${mode} line replays`);
  }
}

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

// The player's recorded line, from a capture's bytes (#3578). Two committed
// captures pair with an eval fixture.
const testdata = join(here, '..', '..', '..', 'python', 'testdata');
let recorded = 0;
for (const [capture, id] of [
  ['6P96T755CNZ3_mawler_win.mcr', 'f04442cd475cdc72'],
  ['YLVVPKPH1MTW_f33_the_insatiable.mcr', 'fc78829c88941121'],
]) {
  const mcr = readFileSync(join(testdata, capture));
  const dir = join(evalDir, 'fights', id);
  const expected = JSON.parse(readFileSync(join(dir, 'human_line.json'), 'utf8'));
  const line = engine.recordedLine(readFileSync(join(dir, 'entry.canonical.json'), 'utf8'), mcr,
    { maxSelectionAnswers: 2048 });
  // Compared as values: the fixture and the engine order an action's keys differently.
  const canon = (value) => (Array.isArray(value) ? value.map(canon)
    : value && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canon(value[key])]))
      : value);
  for (const key of ['actions', 'step_digests', 'terminal']) {
    check(JSON.stringify(canon(line[key])) === JSON.stringify(canon(expected[key])), `${capture}: recorded ${key}`);
  }
  const about = engine.captureSummary(mcr);
  check(about.version === 'v0.111.0' && Number.isInteger(about.history_depth) && about.monsters.length > 0,
    `${capture}: capture summary`);
  recorded += 1;
}
check(refusalCode(() => engine.captureSummary(readFileSync(join(testdata, 'latest.mcr'))))
  === 'unsupported_replay_build', 'another build\'s capture is refused by name');
check(refusalCode(() => engine.recordedLine('{}', new Uint8Array(0))) === 'malformed_entry',
  'a malformed entry is refused before the capture is read');

// Which cards a selection picked (the review's wording): a non-select names none.
{
  const probe = engine.load(fixtures[0].entry).state;
  check(engine.selectedUids(probe, { kind: 'end' }) === null, 'a non-select names no selection');
  engine.drop(probe);
}

const summary = {
  recorded,
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
