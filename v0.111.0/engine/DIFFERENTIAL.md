# The trajectory differential — the wave gate

> **Retired (#2999, under #2827).** `tools/differential.py`, its `smoke/`
> configs, the divergence-ticket sink, the smoke-pairing and must-cover
> audits and `rust-port-nightly.yml` were deleted with the Python simulator
> they compared against. Rust is certified against the game's own `.mcr`
> checksums instead (`tools/eval_suite.py census`). The engine side of the
> protocol stays: `sts-sim diff-serve` (`src/diff_serve.rs`), spoken to by
> `tools/diff_serve_client.py`. This document is the record of the gate as
> it ran; the commands below no longer exist.

`tools/differential.py` is the artifact every port PR gates on (PORT_PLAN §4,
#1291). It synthesizes entry states in Python, asks the Rust engine to admit
them, and plays seeded uniform-random trajectories through **both** engines in
lockstep, comparing at every step:

* the **ordered legal-action list** (exact, ordered — order is contract, not
  heuristic);
* the **full canonical projection** (`sts-sim-canonical-v2` digest every step,
  full bytes at the first and last step of each trajectory);
* **refusal parity** both directions (D6): where Python raises
  `NotImplementedError`, Rust must refuse the same action, and vice versa;
* **terminal agreement**.

Every entry is constructed through `combat_sim.start_combat` with
`build=GAME_BUILD_V0_111_0` explicitly, and every projected state is
canary-checked for the two v0.111.0 fingerprints (`regalite_block_amount == 4`,
`inky_attack_damage == 0`) — the `combat_sim` default is v0.108, which changes
both the seeding scheme and build-branched constants, so an oracle on the wrong
build cannot produce green results.

## The two lanes

| lane | where | scale | role |
|---|---|---|---|
| `--smoke` | hosted ubuntu, every port PR (`rust-port.yml`, `continue-on-error` since 2026-08-24) | every config at its configured scale, bounded and deterministic | the per-PR parity report |
| `--nightly --scale N` | four `mac-solver` matrix jobs, **on demand only** (`workflow_dispatch` on `rust-port-nightly.yml`) | N× entries and trajectories, 2× line length, partitioned by config | the dragnet |

**Red in neither lane is a halt any more.** The smoke step still runs on every
port PR and still prints every divergence with its replay command, but it no
longer fails the `rust port` check (2026-08-24), and a red nightly lane is a
filed note rather than stop-the-line. Since the 2026-08-23 decision record on
#1282, parity is the destination rather than an invariant held at every commit,
so a divergence becomes a durable ticket for the validation phase instead of
stopping the line: record it with its `--replay` command and keep merging. (A
red *build*, lint or test on `main` is still stop-the-line — that is a broken
tree, not a parity gap.)

The nightly lane also stopped firing on its own (#1548): the daily scale-20
cron and the per-merge scale-5 push run are both retired, and the workflow
survives as the vehicle for that validation phase — one deliberate deep run
against a mostly-complete sim beats many shallow ones against a moving target.

### Nightly config sharding

The nightly workflow starts four matrix jobs. Each calls the same driver with
paired, zero-based shard arguments:

```bash
python3 tools/differential.py --nightly --scale 20 \
  --shard-count 4 --shard-index 0
```

The partition is automatic and deterministic. The driver estimates each
config's relative worst-case work from its checked-in declaration:

```text
seeds × (core entries + probe entries) × trajectories × max actions
```

It then assigns the heaviest remaining config to the least-loaded shard, with
filename and shard-index tie-breaks. Nightly scaling applies the same factors
to every config, so it does not change the relative weights. The four shards
cover every selected config exactly once, refuse empty shards, and do not need
a shared filename map. In particular, #1407's derived cross-family config will
join the partition as soon as its config file lands.

Each job prints its selected config count, estimated load, final status, and
wall time. Compare the four `SHARD … wall time` lines after coverage changes;
if the slowest is roughly 2× the fastest, rebalance the estimator or introduce
a justified explicit map. The differential self-test pins complete coverage,
no overlap, deterministic membership, current declared-load skew below 2×,
and refusal of incomplete, empty, or out-of-range shard arguments.

At the 2026-08-20 coverage level, unsharded scale 5 takes about 57 minutes.
Scale 20 is 16× that work because both entries and trajectories scale, which
projects to about 228 minutes per balanced shard. The workflow gives each
shard an inner 480-minute deadline (more than 2× headroom) and a later outer
runner guard. The inner deadline exits nonzero and prints `SHARD TIMEOUT`, so
exhausting the coverage budget makes the matrix red instead of leaving a
cancelled-at-ceiling run.

### The cross-family config

`smoke/cross_family.json` uses the separate
`sts-sim-cross-config-v1` schema because listing `core`/`probes` would defeat
its purpose. The file declares only fixed seeds and runtime bounds. At the
start of the config run, the driver derives its pools mechanically:

1. read `StepKind` family ownership and each family's `IMPLEMENTED` registry
   through the same parser as `build_coverage_badge.py`, and require that their
   union equals `diff-serve`'s live capability manifest;
2. sweep every identity and upgrade in the Python `CARDS` registry through an
   ordinary Python `start_combat` and the Rust load boundary, partitioning
   Python-buildable rows into currently admitted identities and named refused
   near-misses;
3. sweep every `SUPPORTED_ENCOUNTERS` identity through that same boundary.

The mixed deck has one **distinct** admitted anchor for every implemented
family, plus a few admitted cards from the global pool. Consequently every
implemented family is represented and every unordered family pair occurs in
every mixed entry; this is a stronger, more compact stratification than one
fight per pair. Identities rotate deterministically with `(manifest, config,
seed, entry index)`, and encounters rotate in registry order. The entry count
automatically grows to at least the number of admitted encounters, so every
admitted encounter is exercised without adding its name to the config.

Admission is composition-dependent: two identities that load separately can
still require an unimplemented interaction when packed together. The driver
therefore treats the base and its paired one-card near-miss as one candidate:
it builds both through the Python oracle, requires Rust to admit the base, and
requires Python to build the near-miss. A Python-unbuildable base or near-miss
has no oracle, while a named Rust refusal of the base is whole-entry admission
guidance; only those cases deterministically repack all family anchors,
fillers, swap positions, and refused-card choices. Once Python builds the
near-miss, Rust admitting it or returning anything other than the exact
source-derived missing set is an immediate divergence, never a reason to try a
different card. It never filters or allowlists a card or pair, and every
candidate retains all families and the entry's rotated encounter. Named
candidate refusals and retry counts remain visible in the report; exhausting
the bounded search is a loud refusal with the complete reason counts, never a
silently dropped entry.

Every admitted mixed entry has a paired **near-miss** whose document differs
by exactly one card swap. That replacement was refused in isolation during
the same runtime sweep. The mixed document must refuse with the same stable
ordered missing-capability list after rebinding it to the candidate's unique
live physical row. If a live Aggression source makes successive Attack upgrade
rows reachable, their exact `(CardId, upgrade)` classifications are composed
from the same refused/admitted census; no reason comes from the near-miss
response. Discovery-time card-instance UIDs are replaced mechanically by that
one candidate's exact live UID when its projected physical state is
non-default. Zero or multiple live candidate matches, a missing or duplicated
census classification, an admission, an unjustified refusal, or a different
placement-aware missing set is red. This pins the D6 boundary rather than
merely sampling the interior of the implemented surface.

The self-test checks manifest/registry equality, deterministic pool derivation,
non-empty admitted/refused card pools, an admitted carrier for every
implemented family, exact pair coverage and complete encounter rotation in
the admitted base entries, deterministic repacking after a synthetic
near-miss composition refusal, derived-pool-only card participation, the
exhausted-search diagnostic, the one-card near-miss shape/refusal, and
synthetic pins for unchanged and Attack-upgraded candidate rows, physical UID
rebinding, zero/multiple candidate refusal, and red-on-omitted/invented Rust
reasons as well as Rust admission disagreements.
No card, encounter, family, composition exclusion, or refusal identity is
checked into the cross config, so a later port slice joins the lane without
editing a shared file.

Smoke runs the declared bounded entry/trajectory count. Nightly applies the
ordinary `--scale N` rule: entries and trajectories both multiply by `N`, and
the maximum line length doubles. Automatic weighted sharding discovers this
config like every other `smoke/*.json`; no workflow filename map or invocation
flag is needed.

## Adding your family's smoke config (wave PRs)

Drop **one new file**, `smoke/<family>.json`. Configs are discovered by glob
and never share a file, so two wave PRs cannot conflict here — the same
property `content/` relies on. Do not edit anyone else's config.

```jsonc
{
  "schema": "sts-sim-smoke-config-v1",
  "family": "silent_uncommon",          // MUST equal the file name
  "description": "…",
  "character": "CHARACTER.IRONCLAD",
  // Opt in only for generation cards whose native pool provenance requires
  // it. Regent configs also receive the exact character + Colorless epochs.
  "fully_unlocked_card_pool": false,

  // Optional, and mutually exclusive with the flag above (#2469): a normalized
  // PARTIAL unlock profile, sorted and deduplicated, passed verbatim to
  // `start_combat(unlocked_card_pool_epochs=…)`. Use it only for a body
  // Python itself derives from an arbitrary profile — today that is Splash
  // alone; every other generator still demands the exact fully-unlocked set
  // at its use site and would refuse rather than compare anything.
  "unlocked_card_pool_epochs": [],

  // Content this family asserts is INSIDE its implemented surface. Entries
  // built only from these pools MUST be admitted and MUST play identically;
  // a refusal here is red whatever it names (refusal parity, direction 1).
  "core": {
    "encounters": ["ENCOUNTER.TOADPOLES_WEAK"],
    "cards": [{"id": "CARD.STRIKE_IRONCLAD", "upgrade": 0}],
    "relics": ["RELIC.BURNING_BLOOD"]
  },

  // Optional. Content expected to be OUTSIDE it. Each probe entry carries
  // exactly one probe element; if Rust refuses, the refusal must be justified
  // against the capability manifest, and if Rust admits it, the entry is
  // played like any other.
  "probes": {
    "encounters": [], "cards": [], "relics": []
  },

  "deck": {"min_cards": 9, "max_cards": 13},
  "hp": {"min": 45, "max": 80},
  "entries": {"core": 6, "probe": 3},   // per seed
  "trajectories": 8,                    // K per admitted entry
  "max_actions": 60,
  "seeds": ["ZPJHU3WSH2", "…"],         // fixed: the smoke lane is deterministic

  // The claim the gate checks. Every name here must be exercised by some
  // trajectory, or the run is red.
  "must_cover": {
    "steps": ["attack"], "moves": [], "powers": [], "relics": [], "hooks": []
  }
}
```

`must_cover` is the point of the exercise: **"green because untested" is not
green.** Coverage is derived, not asserted — the Rust engine records which step
kinds, move kinds, powers, relics and hooks actually ran, at the dispatch sites
themselves (`src/coverage.rs`), and the driver reads it back per trajectory
through `diff-serve`'s `coverage` command. List every kind your PR implements.

### Auditing meaningful pairings

Dispatch coverage cannot by itself distinguish a conditional term that ran
with a meaningful input from the same term's zero branch. Run the report-first
pairing audit after the binary is built:

```bash
python3 tools/smoke_pairing_audit.py --self-test --output /tmp/pairing-audit.md
```

The all-config report is generated on demand rather than checked in. The hosted
Rust-port gate publishes its exact-head copy in the job summary; local callers
can choose any path with `--output`, or omit it to print the report to stdout.
This avoids making every executable-surface PR edit one shared derived file.
The tool mechanically rebuilds its graph from all discovered configs, the live
`diff-serve` manifest and load boundary, the authoritative runtime registry
which generates `content_tables.rs`, and the current Rust dispatch bodies. It
has no per-kind power-provider allowlist. Power writes and readers, including
listener power-to-power transforms, are derived from their exact dispatch/match
arms; non-zero initial monster power slots are read from exact live-admitted
projected encounter states and intersected with the manifest. Cross-boundary
state resources are derived from the current `HotState` and `CardSpec` schemas,
direct source access, exact helper resolution, helper mutation frontiers,
admitted card rows, and admitted initial states. An unresolved ambiguous helper
is a derivation error, not an empty closure. Consequently a newly
implemented/admitted reader, writer, card, move, or encounter changes the audit
at the same time as the executable surface changes, without adding a resource
name or provider regex.

For each `must_cover` claim the report names every current power read, whether
a reachable writer exists in that config's admitted `core`/`probes` cards and
encounters, and the provider evidence. Exact power-slot and scalar-state
self-writes are subtracted. Aggregate collection read/modify/write is not: a
replace/remove/reorder does not supply the pre-existing orb, card, listener, or
RNG state it consumes, and the claiming kind itself is excluded from that
collection provider evidence. Claims for which the structural census derives
no external dependency are retained explicitly as `UNCHECKED`; the report
gives separate denominators for declared claims, externally-dependent claims,
dependency checks, paired checks, unpaired checks, and unchecked claims.
Claimed powers are also paired with a reachable reader/event. Cross-boundary
state needs a composition, not two independent ingredients: the legacy-card
identity check is emitted for every config and reports an injector without a
delayed-death carrier as unpaired. This is the distinction that reproduces
#1451 even though `elite.json` contains Phrog injection and ordinary attacks
independently.

The `sts-sim-cross-config-v1` schema has no explicit pools or `must_cover`
claims. It is reported separately using its runtime-derived admitted pools;
its family-pair and encounter-rotation coverage remains pinned by
`differential.py --self-test`.

Issue #1455 deliberately makes this audit **report-only**: existing findings
do not redden CI before the backlog is triaged. The hosted Rust-port workflow
runs the derivation and `--self-test` after building `sts-sim`; a derivation or
parser error or failed synthetic self-test is red, while reported `UNPAIRED`
backlog remains green. Generate a local report deliberately with the command
above; never weaken a `must_cover` claim or remove a pin to make the report
smaller.

The production graph contains no content-identity provider table. Historical
regression fixtures in the audit self-test are selected mechanically by their
current admitted writer sets and initial projected powers (for example the
#1462 disposable widening), with uniqueness required. They prove a known gap
closes through the ordinary derivation and fail closed if the witness becomes
ambiguous or stops admitting. Those acceptance fixtures are not cross-config
exclusions, refusal allowlists, or production provider rules. The cross-family
config itself still checks in no card, encounter, family, composition, or
refusal identity.

The interlock that keeps this honest: a config that under-claims `must_cover`
gets caught by review, and a config that shrinks its `core` pools to dodge a
defect cannot then exercise the kinds it claims. The two assertions have to be
satisfied together.

Budget: keep your config's smoke scale to a couple of seconds. The whole lane
is the sum of every family's config.

## Running it

```bash
cd sim/v0.111.0/engine
cargo build --locked --bin sts-sim          # the driver needs the binary

python3 tools/differential.py --self-test   # prove the gate can fail
python3 tools/differential.py --smoke       # the PR lane
python3 tools/differential.py --config smoke/shared.json --keep-going
python3 tools/differential.py --nightly --scale 20
python3 tools/differential.py --nightly --scale 20 --shard-count 4 --shard-index 0
python3 tools/differential.py --smoke --assert-covered steps:attack moves:spit_attack
python3 tools/differential.py --nightly --keep-going --tickets /tmp/divergences.jsonl
```

## Replaying a divergence

Every divergence prints a **replay ticket** (`sts-sim-differential-divergence-v1`)
naming the config, the seed, the entry index and digest, the trajectory, the
step, and the exact action prefix. Entries are a pure function of
`(config, seed, index, kind)`, so those four scalars rebuild the entry
byte-for-byte:

```bash
python3 tools/differential.py --replay '<ticket json>'   # or a file, or -
```

### Getting the ticket out of a sharded run

`--tickets PATH` writes every ticket to `PATH` as JSONL — one ticket per line,
the same object `--replay` takes — flushed as each config finishes.

The nightly workflow passes it and uploads the file per shard
(`divergences-shard-N`), because **Actions withholds a matrix job's logs until
the whole run finishes**: before #1546 a shard that failed early was
undiagnosable until its slowest sibling finished — 17–49 minutes at scale 5,
hours at scale 20. An artifact is readable as soon as its own job ends.

```bash
gh run download RUN_ID -n divergences-shard-3
python3 tools/differential.py --replay "$(head -1 divergences-shard-3.jsonl)"
```

Two properties are deliberate. The file is written **incrementally**, so a
shard killed at its deadline still hands over what it already found; and an
**empty file is not a missing file** — empty means the shard ran clean, absent
means the sink was never wired up, which is why the upload uses
`if-no-files-found: warn`.

Replay re-runs the trajectory verbosely — every step's chosen action and legal
list, then a field-level diff of the two canonical documents at the diverging
step. It exits nonzero if the divergence recurs, so it doubles as a
regression check while you fix it.

If the ticket's `entry_digest` no longer matches, the config or the oracle
moved since the ticket was written; replay says so rather than comparing
something else.

## The self-test

`--self-test` is what makes a green run mean anything. It injects known faults
and asserts the differential reports them:

1. a **wrong action** applied Python-side at a chosen step → must produce a
   divergence at that step, with a prefix that replays cleanly on the
   unperturbed pair;
2. a **flipped projected field** → must produce a state-digest divergence at
   that step, with the field named in the field-level diff;
3. a `must_cover` claim naming a kind no trajectory exercised → must be red.

It runs in the PR lane *before* the real smoke run, and in the nightly lane
too. It uses its own self-contained config, so it does not depend on whatever
the checked-in smoke configs happen to say today.

## What the differential found on its first runs (2026-08-18)

The gate is not decorative: running it over R0.5's engine core immediately
produced four divergence classes, two fixed in #1291 and two filed:

| finding | status |
|---|---|
| `player_side_active` was not modeled, so any fight ending **inside the enemy phase** (the player dying to a monster attack or thorns) projected wrongly | fixed in #1291 (`hot.rs`/`boundary.rs`/`turn.rs`, Python `begin_player_turn` L41998 / `_finish_side_switch_after_disintegration` L48433) |
| card programs kept running after the **owner died mid-card** — a Bash whose thorns retaliation killed the player still applied its `vulnerable` step | fixed in #1291 (`play.rs`, Python `_run_steps_inner` L60167) |
| `zero_energy_attack_plays_started_this_turn` is not tracked, so any **0-cost attack** card diverges the moment it is played | fixed at engine slice 2 (#1367, `play.rs`/`hot.rs`/`boundary.rs`, Python `_advance_card_play_frame` L51216); BULLY and WHIRLWIND are in `smoke/ironclad_uncommon.json`'s core pool and exercise it |
| a long all-skill deck diverged on draw/reshuffle order with one extra finished play in Python | still open on #1314, with its reproduction |

Both were cases where **admission is too permissive**: the engine admits
content it cannot play exactly, which is an I5 violation in the admission gate
rather than a hole in a step body. Slice 2 found a third of the same class and
closed it the other way — STOMP's step body ports in two lines, but the card is
a pile listener whose cost mutates on any attack play, so the gate now refuses
every card `start_combat` gives a slot-7 payload (`steps/ironclad_uncommon.rs`
records the measured divergence beside the stub). The honest interim for what
is left is that a config's `core` pool stays on the surface the engine
reproduces exactly, and the gap is named here and on its issue rather than
being silently avoided.

Slice 2's own two finds, both caught the moment a new mechanic reached the
board rather than by review:

| finding | status |
|---|---|
| an ordinary monster death clears **Poison** (and its instance identity) with Doom/Demise/Oblivion/Strangle/Hang, while Vulnerable, Thorns, Weak and Strength survive it — the death cascade only cleared the acquisition order | fixed in the same PR (`engine/damage.rs`, Python `_finish_monster_death` L22957). Reported as `monsters[1].poison: python='<absent>' rust=6` on a poison tick that landed a killing blow |
| a Poison *source* is what makes a Poison *consumer* mean anything: with only `poison_if_poisoned` and `mirage_total_poison_block_exact` claimed, both kinds were exercised and both could only run their zero branches | fixed by porting the plain `poison` template step and putting DEADLY_POISON in the core pool. `must_cover` cannot catch this class — the kind *is* exercised — so it is a config-design rule, not a gate one |

## Run-lifetime stream counters in entry synthesis (#1356)

`start_combat` refuses to guess how many draws a run-shared stream had already
issued before a fight, and raises instead. For a captured fight that is right;
for a **synthesized** entry there is no history behind it — it is a run's first
fight — so `build_state` passes zero for each explicitly
(`SYNTHESIZED_STREAM_COUNTERS`). Before this, any deck holding a random-target
card made the *Python oracle* refuse, the entry was counted as
`unmodeled in Python` and skipped, and the differential could say nothing about
that kind in either direction — not refusal parity, not a played trajectory.

The counter is an input to *building* the state; both engines then work from
the state that was built, so its value cannot make a red run green. Measured on
`smoke/silent_uncommon.json`: `unmodeled in Python` went 3 → 0, and the
`BOUNCING_FLASK` probe now reaches Rust and refuses with a named reason
(`card step kind "poison_random_serial"`).
