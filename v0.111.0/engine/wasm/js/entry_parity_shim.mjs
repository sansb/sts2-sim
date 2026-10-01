// #3471: a drop-in `sts-sim` for `eval_suite.py census`/`verify` whose
// `entry` subcommand is answered by the wasm module AND the native binary,
// byte-compared. The wasm answer is what the census consumes, so a census run
// through this shim measures the wasm root builder end to end; every
// comparison and both timings go to $STS_ENTRY_PARITY_LOG (JSONL). Every other
// subcommand (`diff-serve`, ...) execs the native binary.
//
//   STS_WASM=module.wasm STS_NATIVE=target/release/sts-sim \
//     STS_ENTRY_PARITY_LOG=log.jsonl node entry_parity_shim.mjs entry --build ...
import { appendFileSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { loadEngine } from './sts_engine.mjs';

const [, , subcommand, ...rest] = process.argv;
const native = process.env.STS_NATIVE;
if (subcommand !== 'entry') {
  const child = spawnSync(native, [subcommand, ...rest], { stdio: 'inherit' });
  process.exit(child.status ?? 1);
}

const flags = {};
for (let i = 0; i < rest.length; i++) {
  const flag = rest[i];
  if (flag === '--opening' || flag === '--native-checkpoints') flags[flag] = true;
  else flags[flag] = rest[++i];
}
const known = new Set(['--build', '--save', '--capture-run', '--encounter', '--node-type',
  '--opening', '--native-checkpoints']);
const unknown = Object.keys(flags).filter((flag) => !known.has(flag));

const t0 = performance.now();
const nativeRun = spawnSync(native, ['entry', ...rest], { encoding: 'utf8', maxBuffer: 1 << 28 });
const nativeMs = performance.now() - t0;

let wasmOut = null;
let wasmMs = null;
let instantiateMs = null;
if (unknown.length === 0) {
  const t1 = performance.now();
  const engine = await loadEngine(readFileSync(process.env.STS_WASM));
  instantiateMs = performance.now() - t1;
  const request = { build: flags['--build'] };
  if (flags['--save']) request.save = readFileSync(flags['--save'], 'utf8');
  if (flags['--capture-run']) request.capture_run = readFileSync(flags['--capture-run'], 'utf8');
  if (flags['--encounter']) request.encounter = flags['--encounter'];
  if (flags['--node-type']) request.node_type = flags['--node-type'];
  if (flags['--opening']) request.opening = true;
  if (flags['--native-checkpoints']) request.native_checkpoints = true;
  const t2 = performance.now();
  wasmOut = engine.entryText(request);
  wasmMs = performance.now() - t2;
}

const nativeOut = nativeRun.stdout.replace(/\n$/, '');
const record = {
  argv: rest,
  native_status: nativeRun.status,
  native_ms: nativeMs,
  wasm_ms: wasmMs,
  instantiate_ms: instantiateMs,
  unknown_flags: unknown,
  equal: wasmOut === nativeOut,
  bytes: nativeOut.length,
};
if (!record.equal) {
  record.native_head = nativeOut.slice(0, 300);
  record.wasm_head = (wasmOut ?? '').slice(0, 300);
}
if (process.env.STS_ENTRY_PARITY_LOG) {
  appendFileSync(process.env.STS_ENTRY_PARITY_LOG, JSON.stringify(record) + '\n');
}
// Argv refusals (exit 2) and anything the shim cannot map stay native.
if (nativeRun.status !== 0 || wasmOut === null) {
  process.stderr.write(nativeRun.stderr);
  process.stdout.write(nativeRun.stdout);
  process.exit(nativeRun.status ?? 1);
}
process.stdout.write(wasmOut + '\n');
