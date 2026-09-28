# Issue #2444: performance-floor recovery findings (2026-09-14)

This record explains the proposed replacement floors. The CI-runner artifact
is the authority for their values; the local component and memory profiles
below diagnose costs and do not set or widen any floor.

## What the red floor means

The CI checkpoints establish cumulative regression, independently of the later
fixture edit. They do not by themselves establish a cause or justify accepting
the cost. On the same self-hosted lane, the observed checkpoints were:

| interval ending at | observed throughput | observed peak RSS | reading |
| --- | ---: | ---: | --- |
| `49c0f490` (2026-09-02) | 1.833 M transitions/s | 4.260 MB | last known green before this series |
| `7e6f39f3` (2026-09-02, #1955) | 1.729 M/s | 4.325 MB | first RSS-only breach; interval includes #1953/#1955 |
| `323d75a1` (2026-09-03, #1977) | 1.536 M/s | 4.293 MB | first throughput breach; interval spans #1973/#1975/#1977 |
| `79034c24` (2026-09-04, #2062) | 1.428 M/s | 4.719 MB | later cumulative expansion |
| `1bdc5797` (2026-09-06, #2133) | 1.179 M/s | 4.915 MB | ordered side-start coverage added |
| `571f8b4b` (2026-09-14, #2423) | 1.046 M/s | 5.243 MB | current pre-recovery observation |

These are interval endpoints, not isolated per-commit proofs: each interval can
contain more than one merge. The component and memory measurements below
supply the missing attribution requested in the independent review of
`74ed4a233e07a594fe48ee1793560dfab687975f`.

## Work-identity separation

The original floor evidence records entry SHA-256
`6a5f557d8ae8b3966aeea58c623d2bdb6fdda9069ec3f1a4a05af32049f16ee2`,
checksum `5d4f992689b53874`, and 4,078,800 transitions.  Commit `72ec06ab`
(#2273, 2026-09-11) added `RELIC.BURNING_BLOOD` to the evolving canonical
projection fixture, changing that file's SHA to
`ad2447f1b5f353640db18ecb3c955416c0efa1fd790ddc29c060184882bba973`.
The performance drop predates that edit.

The benchmark now embeds
`fixtures/bench_ironclad_toadpoles_uniform_v1.json`, a byte-for-byte copy of
the reviewed pre-#2273 entry.  Its Rust SHA assertion and the performance tool's
artifact hash both name the original bytes.  Current local runs again report
the original entry SHA, checksum, and transition count, so fixture drift is no
longer mixed into an engine-cost comparison.  A change to those bytes requires
a new workload name and a separately reviewed floor.

## Narrow recovery and local checkpoint

The recovery keeps the exact listener-order validation but takes its existing
empty-state result directly after validating the relevant metadata.  The three
affected paths are player turn start, side turn start, and side turn end;
malformed live members still take the full validation path and are covered by
unit tests.

On this developer ARM64 host, five full, byte-identical runs after the combined
change measured 1.013--1.054 M transitions/s (median 1.036 M/s) and
4.751--4.784 MB RSS.  The preceding single-guard checkpoint was 0.940--0.969
M/s (median 0.954 M/s), an approximately 9% median recovery (roughly 9--11%
across local checkpoint comparisons).  These numbers are diagnostic only:
they did not run inside the CI-runner floor-setting process and do not
authorize a floor change. The controlled interval comparison below supersedes
them for attribution.

## Reproducible component and process-memory profile

Run from the repository root on macOS:

```sh
python3 versions/v0.111.0/rust/tools/profile_intervals.py --repeat 5 \
  --output versions/v0.111.0/rust/benchmarks/2026-09-14-issue-2444-component-profile.json
python3 versions/v0.111.0/rust/tools/profile_rss_origin.py \
  --output versions/v0.111.0/rust/benchmarks/2026-09-14-issue-2444-rss-origin.json
```

The tools create dedicated temporary branches and sparse worktrees, hold the
host-wide **shared** workload lock while compiling, and the **exclusive** lock
for each complete measurement series. Each series alternates chronological
and reverse sweeps to avoid assigning all later-machine drift to later
commits. All observations, including slower ones, are retained. The raw JSON
contains full source SHAs and tree IDs, original benchmark and Cargo.lock
hashes, release manifests, exact build/run commands, compiler output, binary
hashes, source diffs, `/usr/bin/time -l` output, and complete `sample`/`vmmap`
output. Rust is Homebrew **1.97.1 (8bab26f4f68e0e26f0bb7960be334d5b520ea452)**,
LLVM 22.1.8, on an Apple M5 Max running macOS 26.6.2. All builds retain their
source revision's release settings; all seven September checkpoints use thin
LTO, one codegen unit, and panic-abort.

Every measured run, including observed processes, asserts the original
fixture SHA above, seed `6004515678751904326`, 240,000 trajectories, a 64-action
cap, 2,000 warmup trajectories, **4,078,800 transitions**, **14,332,026 legal
actions**, checksum `5d4f992689b53874`, and 240,000 terminal completions with
zero refusals or other stops. At `571f8b4b`, the temporary copy's evolving
fixture is restored to the pinned original bytes; the raw source diff records
this explicitly. Production sources, the accepted CI benchmark/tool/fixture
hashes, semantic gates, and floor-setting evidence remain unchanged.

The component build adds clocks and allocator telemetry **only in temporary
worktrees**. Its non-overlapping boundaries are:

- `trajectory_clone`: the initial `entry.clone()` once per trajectory; it
  excludes every clone performed inside action application.
- `legal_actions_into`: one public enumeration call per transition, including
  any validation it performs, ending before policy selection/checksum work.
- `apply_action_into_inclusive`: the complete public application call,
  including its state clone, COW container copies, validation, nested engine
  operations and event writes. Do not add inner functions' sampled costs to it.
- `replace_and_drop_predecessor`: assigning the returned state and dropping
  its predecessor, after application has returned. This substantial destructor
  cost would be incorrectly hidden if all work outside the application call
  were labeled "clone".

Warmup is excluded from all four buckets. Their sum is below total loop time;
the remainder includes policy/checksum work, terminal-state destruction,
counter updates, clock setup and allocator snapshots. Clock boundaries include
the elapsed-time read. An independently reported one-million-call empty-timer
calibration exposes that overhead; no noisy per-call estimate is subtracted.
The component build also adds atomic live-allocation bookkeeping, so its
absolute timings must not be presented as the uninstrumented throughput.
Allocation counts/bytes sum exactly across the four buckets to the original
loop counters. Five separate plain-release runs provide aggregate timing and
RSS; independent 2-second, 1-ms `sample` observations of those plain binaries
provide named call stacks without injecting engine timers.

Live heap means **currently requested Rust allocator bytes**, measured after
warmup, at the largest point in the measured loop, and immediately after the
loop, before JSON construction. It excludes allocator metadata, fragmentation,
stack, mapped code and shared libraries. It is supporting evidence, never an
RSS estimate. RSS is separately measured by `/usr/bin/time -l`; `vmmap`
observations run in separate processes so debugger overhead cannot contaminate
the five timing/RSS observations. `size -m` records mapped image/section sizes,
not a claim that every mapped byte is resident.

### Component results and interval attribution

These are medians of five runs. Component columns are **nanoseconds per
transition**, calculated for each run as its bucket's total nanoseconds divided
by 4,078,800, then medianed. The clone column amortizes 240,000 initial clones;
each individual clone takes about 24–25 ns including the approximately
14.4–14.8 ns empty-timer boundary. Plain-loop time is from the separate
uninstrumented binary and must not be summed with component columns.

| source | plain loop ns/transition | legal enumeration | inclusive apply | initial clone | predecessor drop | plain peak RSS bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `49c0f490` | 620.74 | 50.14 | 485.17 | 1.473 | 110.21 | 3,751,936 |
| `7e6f39f3` | 635.54 | 48.38 | 489.75 | 1.475 | 105.52 | 3,833,856 |
| `323d75a1` | 708.66 | 56.08 | 556.20 | 1.426 | 108.65 | 3,932,160 |
| `79034c24` | 834.76 | 91.90 | 649.16 | 1.441 | 108.22 | 4,276,224 |
| `1bdc5797` | 905.78 | 89.42 | 706.49 | 1.451 | 114.36 | 4,341,760 |
| `571f8b4b` | 1,024.25 | 98.44 | 813.73 | 1.444 | 107.61 | 4,751,360 |
| `74ed4a23` | 931.74 | 98.41 | 739.72 | 1.424 | 109.03 | 4,767,744 |

The control has real noise: plain-loop ranges are 617.35–649.29 ns/transition
at `49c0f490` and 917.29–1,027.64 at the candidate. All five observations are
retained, including the candidate's slow run; these are diagnostic medians,
not a replacement best-of-N floor. CPU samples contain 1,688–1,699 main-thread
observations per checkpoint. Named sample counts below are inclusive stack
counts; a child and its parent overlap and must never be added together.

- **`49c0f490 → 7e6f39f3`:** component costs and allocation traffic are
  effectively unchanged; the small plain timing step overlaps run variation.
  This profile does **not** reproduce a strong component-level throughput
  regression or prove that the historical 5.7% throughput drop was necessary.
  The interval's exact Foregone Conclusion/Demonic Shield work adds
  `before_hand_draw_turn_reachability`/`run_foregone_before_hand_draw` and
  rollback-capable card branches in `play_card_with_work`; those rare bodies
  do not run on this fixture. The plain text segment grows only 16,384 bytes.
  The RSS step has unchanged 4,573-byte live/7,919-byte peak requested heap;
  its separate `vmmap` snapshot shows +32 KiB resident text and +64 KiB malloc
  zone residency. This is small image/page/allocator occupancy growth, not
  evidence of a larger combat-state clone. It does not independently justify
  any blanket floor relaxation.
- **`7e6f39f3 → 323d75a1`:** the first clear measured cost increase is inside
  application: **+66.45 ns/transition**, versus +7.70 in enumeration and
  essentially flat cloning/destruction. `end_player_turn` and
  `begin_player_turn` now authenticate the player-start and Tools/Tyranny
  choice ledgers; `call_local_ethereal_provenance_is_exact` enters both the
  enumerator and application for Call of the Void. Plain samples first expose
  these validators (15 player-start, 12 hand-choice and 10 Ethereal-provenance
  inclusive observations), plus 26 in `CardStates::get`. The interval also
  introduces the instanced Bomb/order checks and new turn-start work. These
  functions enforce exact order/ownership and refusal on malformed shared
  state even when a particular card body is inactive. Their aggregate cost
  is measured; the sparse samples do not allocate every nanosecond among
  #1973/#1975/#1977 or the other merges in this interval.
- **`323d75a1 → 79034c24`:** both public paths increase: **+92.96 ns** in
  application and **+35.82 ns** in enumeration. The latter is consistent with
  the expanded `can_play` path (Sloth private-state and Chains of Binding
  checks), card-state lookups and the authoritative potion-belt enumeration.
  Inclusive `can_play` samples rise 111/1,690 → 184/1,691 and
  `CardStates::get` samples 26 → 38; `aeonglass_reachable` adds 27 observations
  after the Aeonglass lifecycle enters shared paths. The interval also adds
  Entropy, monster affliction/death/turn hooks and potion boundary machinery.
  Those are named sources of new dispatch/validation work and a 360,448-byte
  plain text-segment increase; the data do not isolate a single guilty merge.
- **`79034c24 → 1bdc5797`:** enumeration is flat while application adds
  **57.33 ns/transition**. Full acquisition-ordered side-start dispatch and
  Clarity/peer coverage now authenticate `HotState::after_side_turn_start_power_order`
  at turn entry and dispatch; that function appears with 32 inclusive samples.
  The interval also adds ordered hit/exhaust listeners and resumable potion
  hooks. `CardStateStore` grows 32 → 40 bytes while the entry clone remains
  flat. The intended cost is shared listener membership/order validation and
  dispatch, not larger per-trajectory state copying.
- **`1bdc5797 → 571f8b4b`:** application adds **107.24 ns/transition** while
  enumeration adds only 9.02. The new instanced side-end and card-play ledgers
  and relic hooks are visible in `after_side_turn_end_power_order_is_exact`
  (99 inclusive samples), `capture_after_side_turn_end_power_record` (40),
  `finish_player_turn_start_after_before_hand_draw_relics` (123) and
  `relics::after_card_played_hand` (14). Ledger validation preserves physical
  listener identity, acquisition order and late-failure atomicity; the relic
  dispatch gates preserve the newly modeled content. The plain text segment
  adds another 393,216 bytes. Allocation bytes actually **fall** from 868.702
  to 858.466 per transition, so cumulative allocation volume cannot explain
  this CPU or RSS rise.

**Eliminated overhead is distinguishable from the remaining cost.** From
`571f8b4b` to the candidate, inclusive application falls **74.01
ns/transition (9.1%)**, while enumeration is unchanged and clone/destruction
remain flat. Plain-loop medians improve from 1,024.25 to 931.74 ns/transition,
about **9.9% more transitions/second**. In the plain samples, side-end order
validation drops **99 → 10**, side-start order reconstruction **29 → 1**,
and player-start validation **19 → 7**. These are precisely the three
empty-listener shortcuts in this PR; they avoid scanning/reconstructing an
already authenticated empty order. Allocation traffic and live-heap peaks
are identical before and after the recovery. This is a CPU optimization;
it does not claim to shrink process RSS.

The residual candidate cost is concentrated in the shared application path:
roughly 740 diagnostic ns/transition, plus 98 in enumeration and 109 in
predecessor destruction, against only 1.4 in the initial clone. Remaining
plain stacks include `play_card_with_work_inner`, `can_play`, turn-start
continuation/dispatch functions, `HotPile::make_mut`, `HotState::monsters_mut`,
`Arc<FanoutState/ColdFanoutState>::make_mut`, and the system allocator/free
paths. Those perform the enlarged exact engine's validation, mutation and
ownership work. Enumeration and the initial clone still allocate **zero**;
all measured allocation traffic is inside application (13.2161 → 13.2315
allocations/transition across September, only +0.12%).

These measurements establish where the cost lies and that the targeted empty
scans were avoidable; they do **not** prove this implementation globally
minimal or rule out future optimizations of dispatch, COW copying or code
layout. Accepting the replacement floor means accepting the measured larger
exact engine after this recovery, with retained semantic/refusal tests and
unchanged margins. It does not assert that every historical timing delta was
necessary or that untested fast paths may bypass the remaining validators.

### Whole-process RSS: the baseline predates the reported regression interval

The old 4,276,224-byte ceiling was 150% of **2,850,816 bytes measured on
2026-08-19**, at `b5b1ddd558cd98a0b84c6cad6a9bbabb84965211`. The August 26
artifact changed only the allocated-byte ceiling; it retained the August 19
RSS and throughput limits. Thus the proposed **81.0% ceiling increase is not
an 81.0% September-2-to-September-14 memory regression**: the September anchor
was already near the old ceiling.

To cover that earlier growth, `2026-09-14-issue-2444-rss-origin.json` records a
second, original-source comparison. There is **no source instrumentation or
fixture edit** in these three worktrees. Both plain release and the original
`allocation-counting` feature are built and run five times. This reproduces
the floor's allocator configuration, separately from the component profiler's
extra telemetry. All sizes below are bytes; RSS values are medians, with full
ranges/raw outputs in the artifact.

| original source | plain RSS | original allocation-counting RSS | allocation-counting `__TEXT` segment |
| --- | ---: | ---: | ---: |
| `b5b1ddd5` (August floor source) | 2,883,584 | 2,867,200 | 933,888 |
| `49c0f490` (September anchor) | 3,768,320 | 3,866,624 | 2,211,840 |
| `74ed4a23` (recovery candidate) | 4,751,360 | 4,816,896 | 3,424,256 |

In the corresponding original allocation-counting `vmmap` snapshots,
`__TEXT` **resident** size rises from 6,572 KiB to 8,364 KiB, an increase of
**1,835,008 bytes**, versus the independently measured RSS increase of
**1,949,696 bytes**. `__DATA_CONST` residency adds 49,152 bytes; the malloc
zone's resident memory adds only 16,384 bytes (704 to 720 KiB), allocator
metadata stays at 336 KiB, and resident stack stays at 48 KiB. This locates the
large increase in executable/constant pages, consistent with expanded engine
functions, canonical conversion/admission code and content tables. It does
not attribute megabytes to cloned combat state. These are separate-process
snapshots, include shared-image regions and page rounding, and are therefore
supporting attribution rather than an exact additive RSS accounting equation.

The September component runs give a stronger check against retained-heap
growth: the post-warmup requested live heap rises only **4,573 → 5,005 bytes**;
the measured-loop peak rises **7,919 → 8,525 bytes**. `HotState` remains 224
bytes, `HotMonster` 96, `HotCard` 8, `HotHistory` 52, `FanoutState` 312 and
`FrameStore` 48 at every September checkpoint. The added cold state is real:
`ColdFanoutState` appears at 80 bytes by `323d75a1`, grows to 88 and then 184
with the later side-end/card-play ledgers and relic state; `CardStateStore`
appears at 32 and grows to 40 bytes. These cold COW structures explain small
retained/allocated-byte changes, not the megabyte RSS increase. A cumulative
3.5 GB of requested allocation traffic is explicitly **not** 3.5 GB of live
memory.

The local original-allocation candidate range is **4,784,128–4,833,280 bytes**,
below the accepted CI-runner observation of **5,160,960 bytes**. This profile
does not assign that approximately 0.34 MB process/environment difference to
a particular structure or use it to replace the CI RSS measurement. What it
establishes is a large, reproducible image-residency increase with essentially
flat live heap and hot-state sizes. The exact CI value, and its unchanged 50%
margin, still come only from the already reviewed runner artifact.

## Evidence acquisition and final baseline

At the attempted rebaseline time this host had load averages 6.76 / 6.20 /
6.57 and an unrelated slow-pin pytest process using a full core.  The advisory
workload lock cannot make a non-participating busy host quiet.  No evidence
artifact was generated and no floors were changed.

The evidence was generated inside the same CI runner that enforces the floor:

```sh
gh workflow run rust-port-perf.yml \
  --ref codex/issue-2444-perf-floor-recovery \
  -f repeat=25 -f rebaseline=true
```

The downloaded dated JSON and resulting `floors.json` candidate keep the
best-of-N statistic and all current margins unchanged. The CI-runner artifact,
not a local timing pass, is the authority for the new values, subject to
independent acceptance of the intended cost described here.

Run [34896897363](https://github.com/sansb/StsHistoryViewer/actions/runs/34896897363)
performed that CI-runner rebaseline at commit `b02332c4`. Every pass retained
the original entry SHA, checksum, and 4,078,800-transition identity. The best
of 25 was 1.184 M transitions/s and the median was 1.155 M/s. Although the
first three passes began under elevated startup load, the five consecutive
best-of-5 batches were 1.180 / 1.169 / 1.184 / 1.176 / 1.165 M/s, only 1.6%
apart. That supports retaining the existing 15% throughput tolerance and sets
the floor at 1.006 M/s.

The allocation-counting pass measured 5,160,960 bytes peak RSS, 13.2315
allocations/transition, and 858.466 allocated bytes/transition. Applying the
unchanged 50% RSS and 10% allocation margins produces ceilings of 7,741,440
bytes, 14.55 allocations/transition, and 944.3 allocated bytes/transition.
The exact raw observations and environment are committed in
`2026-09-14-perf-floor-baseline-v1.json`.
