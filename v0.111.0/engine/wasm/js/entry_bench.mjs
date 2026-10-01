// #3471: time `entry` through the wasm module, per request (best of N).
//   node entry_bench.mjs MODULE.wasm [reps] < requests.jsonl > times.jsonl
// Request lines are {"request": {...}}; the save text inside is a JSON
// string, which a JS round trip preserves exactly.
import { readFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { loadEngine } from './sts_engine.mjs';

const engine = await loadEngine(readFileSync(process.argv[2]));
const reps = Number(process.argv[3] ?? 3);
for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
  if (!line.trim()) continue;
  const { request } = JSON.parse(line);
  let best = Infinity;
  let bytes = 0;
  for (let i = 0; i < reps; i++) {
    const start = performance.now();
    const answer = engine.entryText(request);
    best = Math.min(best, performance.now() - start);
    bytes = answer.length;
  }
  process.stdout.write(JSON.stringify({ ms: best, bytes }) + '\n');
}
