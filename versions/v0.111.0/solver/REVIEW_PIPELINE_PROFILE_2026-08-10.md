# Production fight-review profile (#1112)

> **Retired (#2827 item F1).** `tools/profile_review_pipeline.py` and
> `tools/profile_review_concurrency.py` profiled the Python review producer
> and were deleted with it; this page and `profiles/*.json` are the dated
> record.

Date: 2026-08-10  
Base: `d8ab850c`  
Scope: observation only. `combat_sim.py`, search behaviour, card documents,
and worker behaviour are untouched.

## Decision

**Do not use this measurement to park or schedule the Rust rewrite yet.**
The host has useful raw CPU capacity, but four simultaneous deadline-bound
reviews reduce exact benchmark worlds per card. More importantly, the current
worker is intentionally single-flight and has no atomic claim, so this is
*host-capacity evidence*, not a deployable four-worker recommendation. The
hard Eel capture is also quality/refusal-bound (`ok_benchmark_only`, zero
exact sampled worlds), so #1034/#1036/#880 remain higher-value work than a
core rewrite.

This updates #1105's go/no-go rule with current evidence: the port's engine
target is now identified by an exact-search profile, but pilot-economics and
parallel-throughput conclusions remain unproven. Revisit the decision after a
safe multi-worker claim/lease design has been measured for queue age, quality,
and cost; re-open the smallest representative Rust spike if those measurements
or live-coach latency show a true compute constraint.

## Reproducible commands

The two checked-in tools are observers. The first records normal production
wall time using only very small phase timers; the second starts independent
processes at each requested worker count. `--cprofile` and `--tracemalloc`
are intentionally opt-in because both change deadline-shaped timings.

```text
python3 solver/tools/profile_review_pipeline.py \
  solver/testdata/85920V7XQFSN.run 2 \
  --provenance-bundle solver/testdata/85920V7XQFSN_fight2_provenance.json \
  --out solver/profiles/2026-08-10-nibbets-provenance-control.json

python3 solver/tools/profile_review_concurrency.py \
  solver/testdata/85920V7XQFSN.run 2 \
  --provenance-bundle solver/testdata/85920V7XQFSN_fight2_provenance.json \
  --levels 1,2,4 --trials 3 \
  --out solver/profiles/2026-08-10-nibbets-concurrency.json

# Exact-search function sample; unbounded deadlines preserve the control's
# exactness rather than profiling a deadline-capped greedy rollout.
python3 solver/tools/profile_review_pipeline.py \
  solver/testdata/85920V7XQFSN.run 2 -k 1 \
  --provenance-bundle solver/testdata/85920V7XQFSN_fight2_provenance.json \
  --actual-deadline none --benchmark-deadline none --cprofile --top 200 \
  --out solver/profiles/2026-08-10-nibbets-provenance-cprofile-exact-k1.json

# Allocation-only sample: deadline-shaped and not used for function ranking.
python3 solver/tools/profile_review_pipeline.py \
  solver/testdata/85920V7XQFSN.run 2 -k 1 \
  --provenance-bundle solver/testdata/85920V7XQFSN_fight2_provenance.json \
  --cprofile --tracemalloc --top 200 \
  --out solver/profiles/2026-08-10-nibbets-provenance-cprofile-k1.json
```

All control captures use the worker configuration: K=20, horizon 12,
5-second actual deadline, 2-second per-world deadline, and sampling seed
`review-pipeline-v1`. The provenance capture resolves and loads the recorded
entry, replays the recorded line, and then solves the recorded and 20 sampled
worlds.

## Production captures

| capture | status | wall | solved states | solve throughput | exact solves |
|---|---|---:|---:|---:|---:|
| Nibbets, full provenance | `ok` | 46.03 s | 147,223 | 3,201 nodes/s | 7 / 21 |
| Sludge, ordinary run path | `ok` | 44.65 s | 229,408 | 5,140 nodes/s | 8 / 21 |
| Terror Eel, hard/mostly-capped | `ok_benchmark_only` | 56.83 s | 20 DFS states | 0.4 nodes/s | 0 / 20 |

The Eel state's low DFS count is not a speed claim: every world reaches the
deadline during the greedy rollout before a DFS pass completes. Its result is
therefore evidence of a difficult, capped/reduced-quality review, not of a
fast search. It is retained specifically because the worker has to handle
that production failure mode.

On Nibbets, phase timers assign 45.99 of 46.03 seconds to the actual and
sampled solves. Provenance decode/resolution took 20 ms, resolved-entry/world
construction 3 ms, recorded-line replay 4 ms, world sampling 2 ms, and the
remaining loading/document work 4 ms. Sludge similarly spent 44.64 of 44.65
seconds in solve calls. Entry/provenance loading and line serialization are
not viable Rust-port targets for end-to-end throughput.

The exact-search function sample is deliberately K=1 and uncapped: both
solves are exact, with 19,821 DFS states in 16.16 solve seconds. Its
cumulative timings overlap and are not additive, but they identify the hot
path: `apply_action` accounts for 14.32 cumulative seconds over 24,650 calls;
`State.key()` accounts for 0.60 seconds over 16,537 calls; and legal-action
enumeration accounts for 0.51 seconds over 9,930 calls. This supports an
engine-transition target without conflating exact search with deadline-capped
rollout. The separate deadline-shaped allocation sample has a 4.89 MiB traced
Python peak; it is not used to rank exact-search functions or estimate native
allocation.

## Parallel throughput

Same provenance-backed Nibbets review, three independent trials at each level:

| workers | mean wall | raw reviews/s | exact-world-equivalent reviews/s | actual-exact reviews/s | exact / deadline benchmark worlds per trial | CPU parallelism |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 50.75 s | 0.0198 | 0.00594 | 0.0198 | 6 / 14 | 0.99 |
| 2 | 46.17 s | 0.0433 | 0.01300 | 0.0433 | 12 / 28 | 1.98 |
| 4 | 53.15 s | 0.0753 | 0.01636 | 0.0753 | 17.3 / 62.7 | 3.90 |

The actual-seed solve stayed exact in every child and the headline result was
stable within each trial. Benchmark quality did not: the mean exact worlds per
card fell from 6.0 at one worker to 4.3 at four. Raw review rate rises 3.80x,
but exact-world-equivalent rate rises only 2.75x. This is useful evidence that
the host has CPU capacity without memory saturation; it is not evidence that
the present worker can safely or beneficially run four processes. Its
single-flight lock and lack of an atomic database claim intentionally prevent
that deployment today. A future safe-concurrency design must repeat this
quality-adjusted measurement with queue-level metrics.

## PyO3 boundary study

No Rust code belongs in this issue. The cProfile sample nevertheless gives a
useful boundary-size warning:

| candidate boundary | observed Python granularity | transfer-inclusive estimate | implication |
|---|---:|---:|---|
| per-state `legal_actions` + `apply_action` | 9,930 enumerations and 24,650 transitions in two exact solves | At 5–25 µs for each Python↔Rust call that copies/decodes a state/action payload, 34,580 calls add 173–865 ms before Rust work; a two-way enumerate/step interface doubles that envelope. | It can only win if the Rust transition itself is substantially faster; do not treat a state-step microbenchmark that borrows Python objects as end-to-end evidence. |
| batched expansion / core+search loop | one transfer per expanded frontier rather than per action | The same 5–25 µs copy/decode envelope is paid once per batch, while action lists and successor states stay Rust-side. Even 100 batches cost only 0.5–2.5 ms. | This is the only plausible boundary for a 5x+ outcome if the port is reopened. |

The 5–25 µs range is an explicit planning envelope, not a PyO3 benchmark. It
includes serialization/copy and result decoding conceptually; the project has
no Rust payload yet, so claiming a measured cross-language number would be
false precision. Any reopened spike must measure both payload ownership modes
with the real canonical state/RNG representation, and compare them against
native/MCR evidence from #649/#1036—not merely Rust versus Python.

## Speedup thresholds and next action

- **2x end-to-end:** the exact profile makes an engine-core experiment worth
  measuring, but does not predict a result until the real transfer benchmark
  and canonical action/RNG differential tests exist.
- **5x end-to-end:** likely requires batched expansion at minimum; per-state
  calls leave too much Python search/transfer work in the loop.
- **10x end-to-end:** likely requires the engine **and** search/key/memo/
  rollout loop to be Rust-side, with Python limited to document and worker I/O.

For the present pilot, keep the single-flight worker, track queue age,
per-review cost, exact-world rate, and refusal/degradation rate, and
prioritize the hard fight's evidence/coverage causes. Do not raise worker
concurrency until a safe claim/lease implementation exists and passes the
quality-adjusted measurement above.
