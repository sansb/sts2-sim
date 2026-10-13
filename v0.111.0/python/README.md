# Solver-backed fight review

The solver reconstructs real Slay the Spire 2 fights and searches for better
legal lines. Its product goal is a post-hoc review experience: compare what
happened in an uploaded run with what was achievable from the same fight
state, draw order, enemy behavior, and available resources.

The product and research umbrella is
[#58](https://github.com/sansb/StsHistoryViewer/issues/58).

**The combat engine is the Rust crate** in [`../engine/`](../engine/). This
directory holds the Python layer around it: run parsing, RNG and map
reconstruction, `.mcr` decoding, the capture, replay and live-coach adapters
that drive `sts-sim`, the IL censuses and templates, and their tests. The frozen
v0.111.0 Python simulator (`combat_sim.py`, `solve_fight.py`, `content/`) and
the tests that existed only to exercise it were hard-deleted on 2026-09-25
(#2827 item F); `sim/meta/SIM_VERSIONING.md` carries the dated record.

## How a fight is replayed

1. `relay_parser.py` reconstructs fight-entry state from a `.run` history.
   Captured `.save` files (adapted by `tools/live_coach.build_entry`) and the
   mod's `.mcr` captures provide stronger snapshots when available.
2. `review_provenance.py` associates each uploaded capture with its fight and
   resolves the save projection that goes with it; `provenance_entry.py`
   checks that projection against the `.run` and the capture.
3. `rust_review.replay_capture_run` hands the capture's embedded run to
   `sts-sim entry --capture-run --opening`, which builds the fight's root in
   Rust: the real deck, relics, potions, enchantments, monsters, RNG state
   and the opening turn. `sts2_rng.py` recreates the game's named RNG streams
   to check the captured ones.
4. The recorded line is resolved and replayed in Rust (`../engine/src/recorded.rs`,
   which the site runs as wasm), and the certification census
   (`../engine/tools/eval_suite.py`) replays it step by step through
   `rust_replay.py`, comparing every native checkpoint.
5. `rust_exact_solve.py` hands a canonical document to `sts-sim exact-solve`
   for the live coach. Rust owns legal-action enumeration, transitions,
   memoization, pruning, deadlines, and objective ordering.

A Python review path used to sit on top of this: a CLI that searched each
fight and assembled a stored review document for a queue worker. The worker
was retired on 2026-10-06 and the path was deleted on 2026-10-11; the
`REVIEW_PIPELINE_*.md` files remain as the record of the stored document
schema, which the site still renders.

## Exactness and the trust model

The central rule is: **refuse rather than approximate**. If a combat-relevant
interaction has not been verified, the Rust entry, opening or engine refuses
it by name. A plausible guess can produce a confident but wrong coaching
verdict and is therefore worse than no verdict.

Certification is by the game's own per-action checksums: the `.mcr`
certification census (`../engine/tools/eval_suite.py census` over the
`../eval/` fixtures, on the release binary) replays each captured fight in
Rust and compares every native checkpoint.

Every implementation is build-specific. Check the installed game's
`release_info.json` before relying on an IL address or capture. On a version
bump, archive the DLL first according to
[`../../dll-archive/README.md`](../../dll-archive/README.md), then
follow `solver/VERSION_BUMP_RUNBOOK.md`.

## Data in this directory

- `card_pool_census.json` is the checked v0.111.0 character-pool order,
  unlock/filter metadata, and exact solo-Ironclad Splash reachability ledger.
  Its local headless-engine generator is `harness/probe_card_pools.py`; the
  Coach and the Rust crate read the checked JSON.
- `potion_pool_census.json` is the checked v0.111.0 solo-Ironclad Alchemize
  pool/order, rarity/RNG vectors, procurement behavior, and reachable-leaf
  blocker ledger. Its local oracle is `harness/probe_potion_generation.py`.
- `target_power_census.json` is the checked current-build projection from
  represented enemy power families to live instance cardinality.
- `cards_census.json`, `relics_census.json`, `potions_census.json`,
  `enchantments_census.json`, `encounters_census.json` and
  `template_census.json` are the IL censuses (`tools/census_*.py`); the
  frontend whitelist and the Rust crate read several of them.
- `card_templates_raw.json` is the pinned-DLL translator snapshot;
  `card_refusal_ledger.json` is the reviewed source for translator-invisible
  card refusals and deliberate translator exclusions. Together they compose
  `card_templates.json` without requiring the game DLL.
- `coverage_matrix.json` is the Python simulator's final coverage ledger,
  frozen when the simulator was deleted. The harness probes still read its
  statuses; the live coverage frontier is the Rust crate's
  (`../engine/COVERAGE.md`).
- `mcr_tables*.json` are the net-id tables `.mcr` decoding needs, per build.

To refresh card templates after a pinned-DLL card read:

```sh
# Requires the pinned sts2.dll.
python3 sim/v0.111.0/python/tools/card_templates.py --raw --output sim/v0.111.0/python/card_templates_raw.json
# DLL-free: validates and composes the raw snapshot with reviewed I5 ledger.
python3 sim/v0.111.0/python/tools/card_templates.py --compose --output sim/v0.111.0/python/card_templates.json
```

Never edit `card_templates.json` to record a manual refusal. Add the exact
reason to `card_refusal_ledger.json`; composition retains raw translator
refusals unless a narrowly documented exclusion applies.

## Code map

| Path | Responsibility |
|---|---|
| `relay_parser.py` | Parse `.run` histories and reconstruct per-fight entry state |
| `replay_fight.py` | Deck and shuffle-counter accounting over a `.run` (the Python predictor Rust `run_counters` replaced; kept for its tests) |
| `mcr_parser.py`, `mcr_native.py` | Decode combat replay files; read the capture's own deal and checkpoint RNG |
| `rust_replay.py` | The `sts-sim diff-serve` session and the recorded-input rules the census resolves a capture with: selections, inputs behind an open choice, native checkpoint comparison |
| `rust_review.py` | The capture run `sts-sim entry --capture-run` roots, the floor-save root's reset-order witness, and the card and state labels |
| `provenance_entry.py` | Completeness and conflict checks on a resolved save projection |
| `rust_exact_solve.py` | One-call, fail-closed adapter to the Rust exact solver over a canonical document |
| `sts2_rng.py`, `dotnet_sort.py` | Exact game RNG algorithms, stream seeding, counters, and .NET sort |
| `map_gen.py`, `neow.py`, `monster_ai.py` | Map, Neow and monster-AI stream reconstruction |
| `admission.py`, `review_provenance.py` | Build admission, and the association of uploaded captures with their fights |
| `tools/` | IL dumpers, censuses, template builders, `.mcr` tables and watcher, and the live coach |
| `harness/` | Local headless host for executing the real `sts2.dll` as an oracle |
| `testdata/` | Distilled run, save, and replay fixtures from live gameplay |

## Stable technical references

- [`SOLVER_INVARIANTS.md`](SOLVER_INVARIANTS.md) — correctness assumptions and
  their guards; a frozen record since #2827 item F. Every new mechanic still
  requires a new walk file under `solver/invariant-walks/`.
- [`ENCOUNTER_MECHANICS.md`](ENCOUNTER_MECHANICS.md) — the IL reads and
  verified mechanic behavior behind the deleted Python simulator, frozen; the
  Rust crate cites its own IL beside each ported function.
- [`RNG_FINDINGS.md`](RNG_FINDINGS.md) and
  [`STREAM_CONSUMERS.md`](STREAM_CONSUMERS.md) — RNG algorithms and stream
  consumption.
- [`MCR_FORMAT.md`](MCR_FORMAT.md) — decoded combat-replay format.
- [`harness/README.md`](harness/README.md) — setup and limits of the real-engine
  headless oracle.
- [`../../../CLAUDE.md`](../../../CLAUDE.md) — synchronization, issue claiming,
  area labels, and contribution rules for all AI contributors.

`solver/HANDOFF.md` is preserved as the historical July 8 feasibility brief,
and `solver/WAVE_PROMPT.md` / `solver/META_COORDINATOR_PROMPT.md` as the
record of the Python coverage waves. None is a current task list.

## Development workflow

Before changing solver behavior:

1. Sync with current `origin/main`, inspect open claims and recent merged pull
   requests, and select an issue matching the appropriate worker and area
   labels.
2. Verify the installed game build. Archive a new DLL version before Steam can
   replace it.
3. Read the relevant census rows, mechanics documentation, invariants, and
   recent invariant walks.
4. Re-derive the behavior from IL with `tools/dump_il.py`, `dump_type.py`,
   `dump_method.py`, and `scan_calls.py`. Async bodies often live in nested
   `MoveNext` types.
5. Model the complete verified interaction in the Rust crate, or record an
   explicit I5 refusal. Do not opportunistically fix work claimed by another
   issue.
6. Add tests and a new file under `solver/invariant-walks/`; never weaken a
   captured live-game certification.
7. Follow the current verification contract in `CLAUDE.md`: every local
   pytest run uses the host-wide lane wrapper, and the pre-merge evidence is
   the **"solver fast gate" CI check** green on the exact final candidate
   SHA (it triggers automatically on any PR touching the solver trigger
   set), plus `rust port` for crate changes. There is no post-submit solver
   lane since #2827 item F.

Lane status is ephemeral: an earlier "empty", "busy", or "available" report is
only a stale observation, never permission or a prerequisite. Invoke the
wrapper directly and let its atomic lock queue the run. No coordinator grant,
user confirmation, manual `ps` preflight, polling lease, or clearance queue is
required. Ordinary contention pauses only pytest; continue other valid work.
Escalate only concrete wrapper errors, a holder proven stale and unkillable, or
repeated wrapper timeouts.

```sh
# Focused or diagnostic pytest:
solver/tools/pytest_lane.sh -- versions/test_file.py -q

# Fallback only (the "solver fast gate" CI check is the normal pre-merge
# evidence; run locally to reproduce a red CI run or during a GitHub outage):
solver/tools/pytest_lane.sh --pr PR_NUMBER -- sim/*/python -q
```

Core tests require Python and `pytest`. IL and census tools additionally use
`dnfile` and `dncil`. The headless harness has a separate local .NET/Python
environment documented in its README and is never a production dependency.

## Product boundary

The current product strategy remains post-hoc and low-friction: users upload
the run files the game already creates. Developer-side watchers, `.mcr`
archives, and the headless DLL harness exist to improve verification; they do
not require players to install resident software. If product requirements
change, record that as an explicit decision rather than silently turning local
validation tooling into a user dependency.
