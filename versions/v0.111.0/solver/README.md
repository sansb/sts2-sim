# Solver-backed fight review

The solver reconstructs real Slay the Spire 2 fights and searches for better
legal lines. Its product goal is a post-hoc review experience: compare what
happened in an uploaded run with what was achievable from the same fight
state, draw order, enemy behavior, and available resources.

The product and research umbrella is
[#58](https://github.com/sansb/StsHistoryViewer/issues/58).

**The combat engine is the Rust crate** in [`../rust/`](../rust/). This
directory holds the Python layer around it: run parsing, RNG and map
reconstruction, `.mcr` decoding, the review, replay and Coach adapters that
drive `sts-sim`, the IL censuses and templates, and their tests. The frozen
v0.111.0 Python simulator (`combat_sim.py`, `solve_fight.py`, `content/`) and
the tests that existed only to exercise it were hard-deleted on 2026-09-25
(#2827 item F); `solver/SIM_VERSIONING.md` carries the dated record.

## How a review works

1. `relay_parser.py` reconstructs fight-entry state from a `.run` history.
   Captured `.save` files (adapted by `tools/live_coach.build_entry`) and the
   mod's `.mcr` captures provide stronger snapshots when available.
2. `sts2_rng.py` recreates the game's named RNG streams for the relevant game
   build; `rust_run_counters.py` (Rust `sts-sim run-counters`) advances each
   stream to the fight on the `.run`-only path.
3. `sts-sim entry … --opening` builds the fight's root in Rust: the real deck,
   relics, potions, enchantments, monsters, RNG state and the opening turn.
4. `rust_review.py` / `rust_exact_solve.py` hand the canonical document to
   `sts-sim exact-solve`. Rust owns legal-action enumeration, transitions,
   memoization, pruning, deadlines, and objective ordering.
5. `rust_replay.py` replays the recorded line (from the `.mcr`) and the solved
   line through Rust, checking the capture's native checkpoints.
6. `review_summary_v2.py` assembles the review document the worker stores.

The `.run` format does not contain every in-combat decision or stream counter.
Later fights may therefore carry counter caveats. The review keeps those
caveats explicit rather than presenting one guessed history as fact.

## Exactness and the trust model

The central rule is: **refuse rather than approximate**. If a combat-relevant
interaction has not been verified, the Rust entry, opening or engine refuses
it by name. A plausible guess can produce a confident but wrong coaching
verdict and is therefore worse than no verdict.

Certification is by the game's own per-action checksums: the `.mcr`
certification census (`../rust/tools/eval_suite.py census` over the
`../eval/` fixtures, on the release binary) replays each captured fight in
Rust and compares every native checkpoint.

Every implementation is build-specific. Check the installed game's
`release_info.json` before relying on an IL address or capture. On a version
bump, archive the DLL first according to
[`../../../solver/dll-archive/README.md`](../../../solver/dll-archive/README.md), then
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
  (`../rust/COVERAGE.md`).
- `mcr_tables*.json` are the net-id tables `.mcr` decoding needs, per build.

To refresh card templates after a pinned-DLL card read:

```sh
# Requires the pinned sts2.dll.
python3 versions/v0.111.0/solver/tools/card_templates.py --raw --output versions/v0.111.0/solver/card_templates_raw.json
# DLL-free: validates and composes the raw snapshot with reviewed I5 ledger.
python3 versions/v0.111.0/solver/tools/card_templates.py --compose --output versions/v0.111.0/solver/card_templates.json
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
| `review_summary_v2.py`, `rust_review.py`, `rust_legacy_review.py`, `rust_replay.py` | The review CLI: Rust roots, Rust search, Rust replay of recorded and solved lines |
| `rust_exact_solve.py` | One-call, fail-closed adapter to the Rust exact solver over a canonical document |
| `rust_run_counters.py` | The `.run`-only path's entering RNG counters, from Rust |
| `sts2_rng.py`, `dotnet_sort.py` | Exact game RNG algorithms, stream seeding, counters, and .NET sort |
| `map_gen.py`, `neow.py`, `monster_ai.py` | Map, Neow and monster-AI stream reconstruction |
| `admission.py`, `sim_identity.py`, `review_provenance.py` | Build admission, producer identity, and review provenance |
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
solver/tools/pytest_lane.sh --pr PR_NUMBER -- versions/*/solver -q
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
