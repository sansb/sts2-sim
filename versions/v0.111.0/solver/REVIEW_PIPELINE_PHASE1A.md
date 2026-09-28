# Fight-review summary pipeline Phase 1a (#1019)

## Purpose and invocation

`review_summary.py` is the measurement-first producer for the Phase 1b fight
review card. It reads one `.run` plus a zero-based fight index and emits one
JSON document to stdout, `--out`, or both:

```text
python3 versions/v0.111.0/solver/review_summary.py RUN.run FIGHT_INDEX \
  -k 20 --sampling-seed review-pipeline-v1 --horizon 12 \
  --actual-deadline 5 --benchmark-deadline 2 \
  --out card.json
```

Use `none` for an uncapped deadline. The counter override flags expose the
Python provenance layer's stream inputs: `--counter`, `--sel-counter`,
`--targets-counter`, `--energy-costs-counter`, `--generation-counter`,
`--potion-generation-counter`, and `--orb-generation-counter`. They are for
exact counters recovered from a save/MCR or a tested hypothesis; the CLI
never invents one when `.run`
accounting returns unknown. `--fully-unlocked-card-pool` is likewise an
explicit assertion, not a default.

The CLI exits 0 for a complete card and 2 for a structured refusal. A refusal
is still valid JSON on stdout/`--out`; no partial result fields are emitted.

## Evaluation contract

- **Actual** is read from the recorded fight and projected onto the solver's
  combat-ending HP seam. `.run` endpoints occur after Burning Blood's known
  +6 or its Black Blood upgrade's +12 post-combat heal; that fixed heal is
  removed below the max-HP cap. A capped max-HP endpoint cannot reveal how
  much landed and refuses rather than guessing. HP lost is entry HP minus this
  combat-ending final HP, not blindly the raw damage counter, so other
  represented healing remains visible. The Phase 1a post-combat HP
  composition table is `{RELIC.BURNING_BLOOD: +6, RELIC.BLACK_BLOOD: +12}`.
  The modeled-mechanic inventory was verified for
  solver build `v0.108.0`: no other modeled mechanic adjusts recorded
  post-combat HP. A future relic/mechanic batch that changes this inventory
  must update this named invariant before its fights are review-card eligible.
- **Best on actual seed** calls the unchanged clairvoyant solver on the true
  entry root. `alpha0` is seeded with the recorded final HP for every recorded
  win. A bare `.run` uses the recorded result as the universal achieved
  fallback if a deadline expires before search rediscovers an equally good
  line. When associated provenance replays the recorded line to the same
  combat-end HP, its full achieved score and action line replace that numeric
  hint as the rollout incumbent and deadline fallback. An uncapped/exact solve
  below the bare observation refuses as `solver_below_observed_outcome`; below
  a constructively replayed line it refuses as `solver_below_replayed_line`.
- **Benchmark distribution** captures the simulator state immediately before
  the opening turn, when Hand is empty and the complete combat deck is in
  Draw. `infoset_sample.sample_worlds` faithfully shuffles that full hidden
  deck and refreshes future named streams; the simulator's real
  `begin_player_turn` then performs the opening draw and all draw-time hooks.
  Each resulting world is solved clairvoyantly. This is the deliberately
  optimistic **best play per sampled shuffle** benchmark, not an honest fixed
  information-set policy.
- A capped solve returns a replayable achieved line, not an optimum proof.
  Best-on-seed final HP is therefore a lower bound and its skill gap is
  rendered `at least N`. Across benchmark worlds, capped final HP is
  downward-biased, win rate is a lower bound, and loss risk is an upper bound.
- Solver losses are horizon-qualified even when the search is exact: a loss
  means that no winning line was found/proved within the configured absolute
  turn horizon, not that the shuffle is unwinnable at every length.
- The complete order of the pre-opening Draw pile is hidden. Encounter setup
  already materialized by the faithful `.run` adapter is conditioned/fixed.
  The Phase 0 caveat remains: revisit this boundary if a modeled mechanic
  reveals or fixes draw-pile positions.
- The trust tier is read from the encounter row in
  `coverage_matrix.json`. Only an unambiguous `certified` row emits
  `certified`; missing, refused, untouched, malformed, or otherwise ambiguous
  rows emit `modeled`.

Phase 1c should add a small public pre-opening-draw seam to `combat_sim`, then
retire both Phase 1a's temporary `begin_player_turn` replacement and its call
to private `_normalize_card_identities`.

## JSON schema v1 — Phase 1b UI contract

All documents contain:

| path | type | meaning |
|---|---|---|
| `schema_version` | integer | Always `1` for this contract. |
| `status` | string | `ok` or `refused`. |
| `fight.run_file` | string | Basename only; local absolute paths are not serialized. |
| `fight.fight_index` | integer | Zero-based index in the parsed run. |
| `fight.run_seed` | string | Present after the run/fight loads. |
| `fight.encounter_id` | string | Present after the run/fight loads. |
| `fight.node_type` | string | `monster`, `elite`, `boss`, or `event`, when loaded. |

An `ok` document also contains:

### `actual`

| field | type | meaning |
|---|---|---|
| `won` | boolean | Whether recorded final HP is positive. |
| `entry_hp` | integer | HP entering the fight. |
| `final_hp` | integer | Recorded combat-ending HP on the solver's scoring seam, before Burning Blood. |
| `hp_lost` | integer | `entry_hp - final_hp`; may be negative. |
| `potions_used.count` | integer | Number of recorded potion-use entries. |
| `potions_used.names` | array of strings | Recorded potion IDs, preserving the run record. |
| `turns` | integer | Recorded `turns_taken`. |

### `best_actual_seed`

| field | type | meaning |
|---|---|---|
| `won` | boolean | Whether an achieved winning line is available. |
| `final_hp` | integer or null | Solver/observed-fallback final HP for a win. |
| `hp_lost` | integer or null | Entry HP minus `final_hp`. |
| `potions_used` | integer or null | Explicit potion actions in the solved line; the observed fallback uses the recorded count. |
| `turns` | integer or null | Terminal turn for a winning line. |
| `exact` | boolean | True only when the search completed before its deadline. |
| `bound` | string | `exact` or `achieved_lower_bound`. |
| `deadline_hit` | boolean | Whether the solver reported deadline expiry. |
| `states` | integer | Search nodes reported by the Rust exact-solve boundary. |

### `benchmark`

| field | type | meaning |
|---|---|---|
| `world_count` | integer | K sampled worlds. |
| `wins`, `losses` | integer | Achieved world outcomes within `metadata.horizon`; a loss does not claim the fight is unwinnable beyond that horizon. |
| `win_rate`, `loss_risk` | rate object | Achieved rates over all K worlds within `metadata.horizon`; every reported loss means "no winning line within the horizon." |
| `winning_worlds` | integer | Count used by the three metric distributions. |
| `hp_lost` | distribution or null | Winning-world entry HP minus final HP. |
| `potions_used` | distribution or null | Winning-world explicit potion-action count. |
| `turns` | distribution or null | Winning-world terminal turns. |
| `exact` | boolean | True only when all K world solves are exact. |
| `distribution_kind` | string | `exact` or `achieved_lower_bound_biased`. |
| `exact_worlds` | integer | World solves that completed exactly. |
| `deadline_worlds` | integer | K minus `exact_worlds`. |
| `outcomes` | array | One outcome object per sampled world, in stable world-index order. |

Each `outcomes[]` object has `world` plus every `best_actual_seed` field. A
loss has null `final_hp`, `hp_lost`, `potions_used`, and `turns`, because the
existing solver proves/optimizes wins but deliberately does not rank losing
terminal states.

A **distribution** has `count`, exact-rational `mean`, exact-rational
`median`, and integer `p10`/`p90`. Percentiles use Phase 0's nearest-rank
definition and are achieved sampled values, never interpolations. A **rate
object** has exact-rational `value` plus `kind`: win rate is `exact` or
`lower_bound`; loss risk is `exact` or `upper_bound`. Every exact rational is
`{"numerator": integer, "denominator": positive integer}` in lowest terms.

Phase 1b must render both rate fields with the horizon qualifier. In
particular, it must not translate `loss_risk` into “N% of shuffles lose this
fight”; the supported claim is “no winning line within H turns in N% of
sampled shuffles,” where H is `metadata.horizon`. This applies even when every
world solve has `kind: exact`, because exactness is relative to that horizon.

### `derived`

| path | type | meaning |
|---|---|---|
| `skill_gap.available` | boolean | False when no winning actual-seed line/fallback exists. |
| `skill_gap.hp_lost` | integer or null | Actual HP lost minus best achieved HP lost, equivalently best final HP minus actual final HP. |
| `skill_gap.exact` | boolean | Mirrors actual-seed solve exactness. |
| `skill_gap.kind` | string | `exact`, `lower_bound`, or `unavailable`. |
| `skill_gap.display` | string or null | `N` when exact; `at least N` when capped. This is the safe chip text. |
| `luck_percentile.available` | boolean | True when K is nonzero (all valid cards). |
| `luck_percentile.percentile` | exact rational | Exact midrank percentile from 0 to 100; higher means luckier. |
| `luck_percentile.worse_worlds` | integer | Sampled benchmarks strictly worse than the actual seed benchmark. |
| `luck_percentile.tied_worlds` | integer | Equal-HP wins, or loss/loss ties. |
| `luck_percentile.world_count` | integer | K. |
| `luck_percentile.exact` | boolean | Actual-seed and all sampled solves completed exactly. |
| `luck_percentile.kind` | string | `exact` or deadline-qualified `achieved`. |

Luck compares wins by HP lost only (lower is better); every win outranks every
loss. Losses tie because losing endpoints are not ranked. Ties receive half
credit: `100 * (worse + ties/2) / K`.

### `metadata`

| path | type | meaning |
|---|---|---|
| `k` | integer | Requested sampled-world count. |
| `sampling_seed` | string | Experiment seed expanded by Phase 0's faithful named-stream machinery. |
| `horizon` | integer | Absolute maximum fight turn searched. |
| `deadlines_seconds.best_actual_seed` | number or null | Whole actual-seed solve deadline. |
| `deadlines_seconds.benchmark_world` | number or null | Per-world benchmark deadline. |
| `elapsed_seconds.best_actual_seed` | number | Measured wall time. |
| `elapsed_seconds.benchmark` | number | All K sampling/opening/solve wall time. |
| `elapsed_seconds.total` | number | Full card wall time including loading. |
| `solver_build` | string | Run build passed to faithful RNG and entry reconstruction. |
| `trust_tier` | string | `certified` or `modeled`. |
| `benchmark_name` | string | `best play per sampled shuffle`. |
| `entry_caveats` | array of strings | Read-only counter-accounting caveats surfaced by the existing loader. |
| `fully_unlocked_card_pool` | boolean | Whether the caller made that explicit assertion. |
| `counter_overrides` | object | Only explicit named counter overrides supplied by the caller. |

### Structured refusal

A `refused` document omits `actual`, `best_actual_seed`, `benchmark`, and
`derived`. It contains `refusal.reason` (stable machine code),
`refusal.message` (crisp human detail), `refusal.details` (reason-specific
values), and requested K/seed/horizon/deadlines/counter overrides plus total
elapsed time under `metadata`. Current reason codes are:

- `invalid_run`
- `fight_index_out_of_range`
- `simulation_refusal`
- `recorded_length_unknown`
- `recorded_length_exceeds_horizon`
- `entry_sampling_seam_unavailable`
- `solver_below_observed_outcome`
- `solver_below_replayed_line`
- `observed_final_hp_ambiguous`

## Fixture cards

The checked-in Eel, Colony, and Sludge cards under `versions/v0.111.0/solver/testdata/` are real
outputs from `8DVXPWUWRY.run`, using exact stream-counter overrides recovered
from the matching entry saves. They are UI fixtures, not exact-search pins;
their metadata and per-world flags disclose every deadline-limited solve.

## Measurements

Measurement host: Sean's local Mac, 2026-08-08. Wall time is sequential and
includes sampling plus solving unless noted. All capped results are achieved
lines, not optimum proofs.

### Testdata fights

All five rows use the real `8DVXPWUWRY.run` fight and exact Shuffle / Card
Selection overrides recovered from the matching entry save. K=10, horizon 12,
5-second actual-seed deadline, 2-second benchmark-world deadline, experiment
seed `phase1a-fixtures-2026-08-08`:

| fight | total | actual solve | actual exact | benchmark exact | wins | HP-lost mean / median / p10 / p90 | achieved luck |
|---|---:|---:|---:|---:|---:|---|---:|
| Sludge | 33.86 s | 6.56 s | yes | 2/10 | 10/10 | 1.1 / 0 / 0 / 2 | 60th |
| Eel | 46.62 s | 5.12 s | no | 0/10 | 10/10 | 25.8 / 22.5 / 15 / 36 | 100th |
| Colony | 35.56 s | 5.06 s | no | 0/10 | 10/10 | 33.8 / 32.5 / 23 / 38 | 100th |
| Fog | 33.22 s | 6.51 s | no | 0/10 | 10/10 | 3.7 / 3.5 / 0 / 9 | 30th |
| Gardeners | 56.86 s | 7.18 s | no | 0/10 | 10/10 | 19.7 / 18.5 / 10 / 28 | 100th |

The deadline is checked at the solver's existing boundaries, so measured wall
time can exceed the nominal cap. Gardeners is tractable only as a capped card
in this pass. Eel K=10 uncapped was stopped after 600 seconds without a
document. Colony K=10 uncapped at the shorter valid horizon 8 completed in
651.54 seconds. Sludge K=10 uncapped completed in 219.06 seconds.

### K/deadline grid and deadline bias

Sludge grid, horizon 12, experiment seed `phase1a-grid-2026-08-08`. The
actual-seed deadline is 5 seconds in capped rows and uncapped in the `none`
row. Bands are winning-world HP lost:

| K | world deadline | total | exact worlds | mean / median / p10 / p90 | achieved luck |
|---:|---:|---:|---:|---|---:|
| 10 | 2 s | 31.94 s | 1/10 | 1.2 / 0 / 0 / 0 | 55th |
| 10 | 5 s | 55.88 s | 5/10 | 1.2 / 0 / 0 / 0 | 55th |
| 10 | none | 219.06 s | 10/10 | 1.2 / 0 / 0 / 0 | 55th exact |
| 20 | 2 s | 56.88 s | 5/20 | 1.5 / 0 / 0 / 7 | 60th |
| 20 | 5 s | 84.73 s | 12/20 | 1.5 / 0 / 0 / 7 | 60th |
| 40 | 2 s | 110.22 s | 11/40 | 2.225 / 0 / 0 / 9 | 66.25th |
| 40 | 5 s | 193.62 s | 19/40 | 2.225 / 0 / 0 / 9 | 66.25th |

Uncapped K=20/K=40 was not tractable enough to justify another roughly 7/14
minutes after the all-exact K=10 result. Sludge's greedy achieved lines happen
to equal its exact metrics for the first ten worlds, so deadline changes only
the proof/exactness labels there. K itself is not stabilized at 10: p90 moves
0 → 7 → 9 and luck moves 55 → 60 → 66.25 across K=10/20/40. Median stays 0.

The required nonzero deadline-bias exhibit is Colony, using identical K=10
worlds at horizon 8 (`phase1a-bias-colony-2026-08-08`):

| world deadline | total | exact worlds | HP-lost mean / median / p10 / p90 | luck | actual-seed gap |
|---:|---:|---:|---|---:|---|
| 2 s | 38.69 s | 0/10 | 31.2 / 30.5 / 21 / 38 | achieved 100th | at least 19 HP |
| none | 651.54 s | 10/10 | 11.1 / 11.5 / 6 / 16 | exact 40th | exact 20 HP |

Thus the cap depressed median achievable final HP by 19 (equivalently inflated
median HP lost by 19), moved both p10 and p90 by 15/22 HP, and shifted the
player from an apparently luckiest-sampled 100th percentile to the exact 40th
percentile. This is why a capped band must never be rendered as an optimum
distribution even when every world has an achieved win.

### Local-corpus sample and refusal rate

The local corpus contained 355 `.run` files. The deterministic stratified
sample selected 20 filename-sorted runs at evenly spaced corpus quantiles;
within them it cycled fight positions 0%, 25%, 50%, 75%, 100% through each
run. No unlock or RNG-counter assertion was supplied.

| result | fights | rate / cost |
|---|---:|---|
| complete K=10 cards | 2 | 10%; 29.43 s and 31.32 s |
| structured refusals | 18 | 90%; 17 entry refusals completed together in under 0.3 s, one reachable solve refusal in 0.03 s |

The 17 entry refusals were: five ambiguous upgrade timings, four unknown RNG
counters, four missing explicit fully-unlocked-pool assertions, two event-node
entry-order ambiguities, one ambiguous enchant timing, and one unsupported
encounter. One apparently admissible Punch Construct fight then reached an
I5 refusal for an enchanted/cost-overridden X-cost interaction. Across the
whole sample, measured compute was about 61 seconds, or about 3 seconds per
attempted fight, because refusals fail fast. For product capacity planning,
the more relevant supported-fight cost is the 29–31 seconds above; the 90%
refusal rate says the `.run`-only provenance surface, not just the 63 currently
refused coverage rows, is Phase 1c's largest eligibility constraint.

### Recommended production defaults

Recommend **K=20, 2 seconds per benchmark world, 5 seconds for the actual
seed, horizon 12**, always offline/asynchronous and cached. This is a pilot
default, not a claim that K=20 bands are fully converged: K=10 is visibly too
noisy in the Sludge tail, while K=40 doubles cost and still leaves most hard
worlds unproven. Keep the schema's bound labels visible and allow a background
K=40 refinement later without changing schema v1.

Measured at that default:

| fight | total | benchmark exact |
|---|---:|---:|
| Sludge | 56.88 s | 5/20 |
| Eel | 81.49 s | 0/20 |
| Colony | 66.72 s | 0/20 |
| Fog | 70.14 s | 1/20 |
| Gardeners | 86.35 s | 0/20 |

Mean is 72.32 seconds per supported fight (range 56.88–86.35). A fully
supported 20-fight run is therefore about **24.1 minutes sequentially**;
using the observed range gives roughly 19.0–28.8 minutes. Independent worlds
can be parallelized by Phase 1c, but Phase 1a reports only measured sequential
cost and does not assume a worker count. With the corpus sample's current 10%
eligibility, most attempted fights refuse before paying that compute cost.

Gold is intentionally absent from schema v1. It is reserved as a future field
after the fight-level HP/potion/turn contract and compute placement settle.
