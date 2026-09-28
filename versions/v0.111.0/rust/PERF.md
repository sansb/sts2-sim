# Performance floors for `sts-sim` (PORT_PLAN §6/§7)

The port's performance discipline is split by **noise**, not by importance:

| when | what | why there |
|---|---|---|
| compile time | hot-type size assertions (`src/hot.rs`, `src/frame.rs`) | free, exact |
| PR time, hosted ubuntu | allocations-per-transition pin, string-compare contract test, differential smoke | deterministic — zero benchmark noise, so it can gate |
| **post-submit, mac-solver** | **throughput floors, peak-RSS ceilings, whole-loop allocation ceilings** | **real-hardware measurements move with the machine; they must never gate a PR** |

This file documents the third row: `.github/workflows/rust-port-perf.yml`,
`tools/perf_floor.py`, `src/bench.rs`, and `benchmarks/floors.json`.

## How the lane works

`sts-sim bench` replays a **fixed, seeded, deterministic workload** and prints
one `sts-sim-bench/v1` JSON object:

- the entry is the checked-in, byte-pinned workload fixture, embedded in the binary with
  `include_str!` and reported by sha256, so a run cannot silently pick up an
  edited file;
- the policy is a SplitMix64 that picks uniformly among the ordered legal
  actions. Trajectory *i*'s stream is derived from `(seed, i)`, so it does not
  depend on how many trajectories the run does;
- the timed window is **search-shaped**: enumerate legal actions, pick one,
  clone-and-apply. Enumeration is deliberately inside it — a regression that
  doubles `legal_actions` cost is exactly the creep §7 exists to catch;
- every run reports a `checksum` folded over the actions it applied. Two runs
  with different checksums did different work, and their throughput numbers are
  not comparable.

`tools/perf_floor.py` drives it:

1. builds plain release, hashes the binary;
2. runs the bench `--repeat` times (default 5) and asserts **all repeats did
   identical work** (same entry hash, checksum, transition count) — otherwise it
   refuses to issue a verdict at all;
3. takes **best-of-N** throughput. Best-of-N remains the statistic of record
   while the serialized lane accumulates controlled evidence; changing it or
   its repeat count without that evidence would combine two variables in one
   fix. Residual host/OS noise is still predominantly one-sided, so the maximum
   remains the closest sample to what the machine can do;
4. builds `--features allocation-counting`, verifies the binary actually
   changed, and runs one instrumented pass under `/usr/bin/time -l` for peak
   RSS and allocations/bytes per transition;
5. compares everything against `benchmarks/floors.json`.

The measured release profile uses `panic = "abort"`. The CLI's admitted
failure surface is typed `Result` refusal, and no shipped caller catches a
panic, so release unwinding cannot recover an admitted workload. Omitting its
panic-unwinding landing pads and most unwind metadata reduces the resident
release image; dev/test retain unwinding for invariant tests that deliberately
use `catch_unwind`.

Issue #1924 measured that build choice in one exclusive-lane, exact-workload
pair. The unwind control reproduced the CI allocation binary byte-for-byte
(`92543a167b98523b33ddbcead596c6281ccc6d5dce210bff5ece606e9f242567`)
and used 3,948,544 B peak RSS; the abort candidate used 3,833,856 B, **114,688
B (seven 16 KiB pages) less**, while the work checksum, 4,078,800 transitions,
53,905,710 allocations and 3,384,057,399 allocated bytes were identical. The
candidate was also 321,792 B smaller on disk. No floor or ceiling moved.

Exit codes: **0** green, **1** floor breach, **2** refusal (bad invocation,
broken environment, nondeterministic workload). The workflow fails on both
nonzero cases; the printed report says which.

## One physical Mac, one cooperative workload lane

Four Actions runner services share the same physical Mac. Separate GitHub
Actions concurrency groups do not prevent a perf run in one workflow from
overlapping a nightly shard, solver gate, or DLL check in another. Conversely,
putting all of those jobs in one shared Actions group would serialize the four
nightly shards and unrelated solver jobs that are safe to run together.

`solver/tools/mac_solver_workload_lock.py` is the host-wide reader/writer protocol:

- the perf command holds the **exclusive** resource lock across both builds,
  all timed repeats, and the instrumented pass;
- the nightly build/self-test/shard, fast-gate dependency/collection/test,
  slow-pin dependency/collection/test, and DLL archive check each hold a
  **shared** resource lock while doing work;
- every acquisition passes through an exclusive turnstile, and a waiting perf
  writer attempts the resource with `LOCK_NB` and *keeps* the turnstile while it
  backs off. Later correctness jobs therefore queue behind it rather than
  overtaking it;
- the lock lives at a fixed path under `/tmp`, outside every runner workdir.
  The wrapper replaces itself with the command while leaving only the resource
  descriptor inherited, so normal exit, failure, cancellation, signal, and
  crash all release it in the kernel. There is no background process or stale
  marker cleanup and every command's original exit status/signal is preserved.

This is deliberately a **cooperative** protocol. `SELF_HOSTED_WORKFLOWS` in the
contract test is an intentionally transcribed change-detector: it pins every
workflow containing `runs-on: [self-hosted, solver]`, plus every named command
step and its shared/exclusive/unlocked classification. Renaming or adding a step
therefore requires a deliberate census refresh; it is not a generated manifest.
Ad-hoc local processes cannot be discovered or suspended safely; operators
still avoid launching a manual benchmark or heavy local build during the perf
lane.

If an exclusive perf job arrives while all four scale-20 shards hold shared
locks, it takes the turnstile and holds it across each one-second backoff.
Later shards, solver gates, and DLL checks queue behind it rather than
overtaking it: this is deliberate **writer priority**, adopted in #1599. It
replaced an earlier reader-liveness policy that starved the writer outright --
six consecutive perf verdicts were lost to the acquisition budget with no
commit measured, because the lane was never idle at the instant the writer
looked.

The cost to readers is one drain interval, once per measurement. Only the
holders that already own the resource gate the writer; the longest of those is
a fuzz shard at ~19 minutes, and the exclusive command itself holds for ~40
seconds. Perf still exits nonzero with a contention error if it cannot acquire
within the workflow's explicit 120-minute budget
(`--acquire-timeout-seconds 7200`), inside `rust-port-perf.yml`'s 150-minute
outer timeout; the wrapper's own default, used only when no budget is passed,
remains 480 minutes. Fixed turnstile-then-resource order still prevents
deadlock -- a reader releases the turnstile before it returns the resource, so
no holder ever waits on the turnstile while owning the resource, and every
timeout or cancellation path closes both descriptors -- and a successful
exclusive acquisition still excludes every reader.

Run it locally through the same host-wide lane as CI:

```
python3 solver/tools/mac_solver_workload_lock.py \
  --acquire-timeout-seconds 28800 exclusive -- \
  python3 versions/v0.111.0/rust/tools/perf_floor.py
```

Add `--trajectories N` to the inner command for a fast local sanity check. That makes the run
**advisory**: it prints numbers and issues no verdict, because a shrunken
workload is not the workload the floors describe.

## The floors, and where they came from

`benchmarks/floors.json` is keyed by workload name. Each entry carries the
measured baseline, the tolerance applied to it, the derived floor/ceilings, and
— the load-bearing field — `evidence`, naming the artifact in `benchmarks/`
that the numbers came from.

Baseline of record: `benchmarks/2026-09-14-perf-floor-baseline-v1.json`,
measured on the runner Mac itself (Apple M5 Max, 18 logical CPUs, macOS) after
the complete v0.111.0 semantic expansion. It contains 25 timed passes under
the host-wide exclusive workload lane.

| metric | baseline | tolerance | floor / ceiling |
|---|---|---|---|
| transitions/s | 1,184,076 (best of 25) | −15% | **1,006,464** |
| peak RSS | 5,160,960 B | +50% | **7,741,440 B** |
| allocations/transition | 13.2315 | +10% | **14.55** |
| allocated bytes/transition | 858.466 | +10% | **944.3** |

### Why those tolerances

**Throughput, −15%.** The first three of the 25 passes began while the recorded
one-minute load average was falling from 10.14 and make the raw individual-run
spread look large (26.4%). The statistic the lane actually compares is
best-of-5. Splitting those same observations into five consecutive batches
gives bests of 1,180,255 / 1,169,156 / 1,184,076 / 1,176,399 / 1,165,395 per
second: a **best-of-5 spread of 1.6%**. The unchanged 15% margin is therefore
roughly nine times the observed statistic noise, and the worst batch best
clears the new floor by 15.8%. That is what a *creep detector* needs:
essentially no false positives on a runner that also hosts the solver gate and
the pytest lane, while a 15%-scale regression trips on the merge that caused
it and repeated smaller regressions trip once they accumulate past it.

Be honest about what this does not do: **a single-merge 5% regression will not
reliably trip a wall-clock floor on a shared host.** That is not a gap in the
tolerance, it is why §7 divides the labour the way it does — the structural
causes of small regressions (an owned `String` in an event, a clone in a hit
loop, a type that outgrew its budget) are caught exactly and noise-free at PR
time by the allocation pin and the size assertions. This lane catches what
those cannot: algorithmic cost that allocates nothing.

The baseline was produced under host-wide workload serialization. Repeat count,
statistic, tolerance, or baseline can still be reconsidered only in a separate
reviewed change with a fresh evidence artifact; serialization is not permission
to move a floor silently.

**Peak RSS, +50%.** The tolerance is retained as headroom for content tables and
hot-type fields that later slices legitimately add to the process. The new
5.16 MB baseline records the complete v0.111.0 listener/relic surface; a real
leak or unbounded per-transition retention still blows through the 7.74 MB
ceiling quickly.

**Allocations, +10%.** The evidence reports 53,968,550 allocations and
3,501,511,080 allocated bytes for the deterministic workload. The tolerance is
deliberately small so structural creep shows up. Note this is the *whole
playout loop*, not the PR-time
pin's window: `perf_floor.py` cross-checks that the bench number stays above the
cargo-test per-apply ceiling (a bench number **below** it would mean the
counter stopped counting), and reports that as an advisory rather than a gate,
because `src/engine/mod.rs` belongs to the engine slices and a rename there must
not stop the line.

## How to read a breach

A red run prints, per workload, a PASS/FAIL line per metric with the measured
value, the limit, and the percentage margin, then the observed distribution
(best / median / worst / spread over N runs).

Work through it in this order:

1. **Is it a refusal (exit 2) rather than a breach (exit 1)?** Then no
   measurement was trusted: the workload stopped being deterministic, the
   allocation-counting build did not take effect, or `/usr/bin/time -l` gave
   nothing. Fix the tool or the environment; there is no performance claim to
   make either way.
2. **Is there a `[DRIFT]` line?** The workload's checksum or transition count
   moved since the baseline — a slice changed the action space, which is normal
   during the wave. Drift is *never* a breach on its own, but it does mean the
   throughput comparison is approximate; refresh the baseline at the next
   convenient reviewed rebaseline.
3. **Which metric failed, and by how much?** Confirm the log says the exclusive
   mac-solver lane was acquired. A throughput failure just past −15% on a run
   whose spread is wide is worth one controlled rerun before acting. A failure
   at −30% with a tight spread, or any allocation-ceiling failure (that metric
   is exact), is real.
4. **Which merge?** The lane runs per push to `main`, so the previous green run
   brackets it to a single commit.

**A red run is stop-the-line.** Fix or revert before merging anything else — a
stack of merges on top of a red floor destroys the per-merge attribution that
is the whole reason this runs on push rather than nightly only.

## How to rebaseline legitimately

Floors move **only** in a reviewed diff that names fresh evidence. Never widen
a floor to turn a red run green.

Generate timing candidates on the same self-hosted CI runner that enforces the
floor. macOS scheduling priority makes an otherwise locked local shell
systematically slower than that runner, so local timing is diagnostic and must
not set a CI floor.

```
gh workflow run rust-port-perf.yml --ref <candidate-ref> \
  -f repeat=25 -f rebaseline=true
```

The run uploads `rust-port-perf-rebaseline-<sha>`, containing the candidate
`floors.json` and `benchmarks/YYYY-MM-DD-perf-floor-baseline-v1.json`.
The evidence records environment, rustc, load averages at both ends, artifact
hashes, both binary hashes, and **every run's number, not just the best**.
Download both candidates into the review branch and inspect them before
committing; the workflow never pushes or changes repository contents itself.

For tool development only, the equivalent clean-tree command is
`perf_floor.py --rebaseline --repeat 25 --write-floors` under the exclusive
workload lock. Its timing output is not CI evidence.

The tool refuses `--rebaseline` from a dirty tree (the artifact would name a SHA
that does not describe what was measured). `--allow-dirty` overrides it, and
then the PR body has to say why.

The PR carrying a floor change states, in prose: what changed, why the new
number is correct rather than a regression being accommodated, and which
evidence artifact backs it. A floor that moved without that is a floor nobody
can audit — which is the same failure as having no floor.

## Relationship to #1283

PORT_PLAN §6 specifies throughput floors **per eval-suite fight**. That suite is
#1283 and does not exist yet, so the single workload here —
`ironclad_toadpoles_uniform_v1` — is a **synthetic stand-in with the right
shape**: a byte-pinned canonical entry, real engine transitions, a deterministic
policy, sized to a ~2-second timed window.

What changes when #1283 lands: each eval fight becomes another `Workload` row in
`src/bench.rs` and another entry in `benchmarks/floors.json`, measured and
floored the same way. Nothing about the tool, the workflow, or this document
changes shape. The synthetic row stays as the fast smoke case, and per-fight
floors — short fights, long boss fights, the exactly-solvable ones — are what
finally make "no performance regressions" a per-encounter claim rather than a
whole-engine average.

Until then, read the number for what it is: one Ironclad opening against two
Toadpoles, played to a terminal state 240,000 times.
