# Review pipeline schema v2: benchmark-only cards (#1027)

## Purpose and invocation

`review_summary_v2.py` wraps the merged Phase 1a library. It emits the same
full card when exact-seed replay is available, and can emit an honest
benchmark-only card when the only missing information is physical-copy
assignment or a future run-lifetime RNG counter:

```text
python3 versions/v0.111.0/solver/review_summary_v2.py RUN.run FIGHT_INDEX \
  -k 20 --sampling-seed review-pipeline-v1 --horizon 12 \
  --actual-deadline 5 --benchmark-deadline 2 \
  --out card.json
```

The CLI exits 0 for `ok` and `ok_benchmark_only`, and 2 for `refused`.
`--out`, `--no-print`, deadline syntax, and counter overrides have the same
meaning as Phase 1a. `--potion-generation-counter` completes the six future
run-stream overrides consumed by provenance-bearing worker jobs. The legacy
`--fully-unlocked-card-pool` option asserts both fully unlocked card and potion
pools in this wrapper.

## Exactness boundary

The run-only, unresolved, legacy, and rejected-entry paths delegate to
`review_summary.generate_review_document` before trying any fallback. Apart
from schema additions and the A10 unlock default below, those fights retain
Phase 1a loading, solving, objective, bounds, distributions, and refusal
behavior. A complete, self-consistent resolved save entry instead enters the
same unmodified simulator through `mcr_replay`'s established save adapter;
only the evidence-backed entry and pool inputs differ.

The retry allowlist is deliberately narrow:

- **Upgrade copy assignment.** Dated later upgrade events determine how many
  upgrade levels must be removed from the endpoint copies. The wrapper
  enumerates every assignment consistent with those events. It proceeds only
  if every assignment has the same multiset of card id, upgrade, enchantment
  id/amount, and modeled persistent props.
- **Enchant copy assignment.** Static, dated non-Goopy enchant events are
  treated the same way. The wrapper determines the number of enchanted copies
  active at entry and enumerates which physical copies they could be.
  Different simulator-visible multisets refuse. Goopy's mutable amount always
  refuses when ambiguous; a post-combat endpoint cannot reconstruct its entry
  amount.
- **Unknown future RNG counter.** The wrapper may use a placeholder only to
  reach the pre-opening-draw seam. The captured stream tuple must retain the
  placeholder's exact counter, proving that the substituted stream was not
  consumed before the seam; any consumption refuses. As a redundant guard,
  the wrapper constructs the seam with placeholder values 0 and 1, then
  requires every non-hidden state field and the exact semantic Draw multiset
  to agree. The Phase 0 sampler replaces Draw order and every future hidden
  stream only after both checks. Stone Cracker, for example, refuses because
  its unknown CardSelection stream is consumed while upgrading a card before
  the seam.

Event-node entry ambiguity, removed-card ambiguity, mutable Goopy amount,
unsupported content, persistent relic state, and every other simulator
refusal remain refusals. Discovering a second refusal after canonicalizing the
first is also a refusal; the wrapper never treats “degraded” as permission to
approximate another mechanic.

## Ascension-gated unlock default

A recorded ascension of 10 or higher automatically supplies the product
assertion that the card and potion pools are fully unlocked. The card stamps
`metadata.assumed_fully_unlocked: true`. Below A10, no assumption is made;
the caller must pass `--fully-unlocked-card-pool`, and the metadata flag stays
false because the assertion was explicit rather than inferred.

This default supplies pool eligibility only. It does not suppress a later
counter, provenance, interaction-order, encounter, or mechanic refusal.

## Schema v2 — Phase 1b UI contract

Schema v2 inherits every field and numeric definition in
`REVIEW_PIPELINE_PHASE1A.md`, including exact rational numbers, nearest-rank
percentiles, deadline-bound labels, and the horizon-qualified meaning of a
loss. The following additions and status-specific presence rules are the v2
contract.

### Common fields

| path | type | meaning |
|---|---|---|
| `schema_version` | integer | Always `2`. |
| `status` | string | `ok`, `ok_benchmark_only`, or `refused`. |
| `degraded_reasons` | array of strings | Empty for `ok` and `refused`; one or more stable reason codes for `ok_benchmark_only`. |
| `metadata.assumed_fully_unlocked` | boolean | True only when recorded ascension >= 10 activated the automatic card/potion-pool assertion. |
| `metadata.provenance_tier` | string | Input tier for this card: `run_only`, `provenance_resolved`, `provenance_unresolved`, or `provenance_legacy`. |
| `metadata.provenance_resolved_fights` | array of integers | Sorted run fight indices whose save candidates uniquely matched the corresponding replay's complete combat-start counter map. |
| `metadata.entry_state_source` | string | Additively present as `resolved_save` when this fight used a complete, self-consistent resolved save projection for physical deck order/identity and ordered relic state. Omitted when that evidence was absent or rejected. |
| `metadata.unlock_pool_source` | string | Additively present as `resolved_save` when exact saved unlock epochs supplied card/potion pool eligibility, including when that evidence proves the pools are not fully unlocked. |

The provenance fields are additive. Documents generated before
the provenance consumer may omit them and remain valid schema-v2 documents.
New worker documents always carry both `metadata` fields; documents without a
reproduced line omit `best_actual_seed.seeded_by`. `provenance_unresolved` and
`provenance_legacy` prohibit every save-derived field for that fight, but a
successfully associated replay may still provide its self-contained
combat-start counter overrides and recorded action line.

`provenance_resolved` identifies a unique counter-matched candidate; it does
not by itself assert that slice-3 entry enrichment was consumed. Before using
the candidate, the consumer requires one schema-19+ player projection with
complete indexed deck/relic arrays and unlock/grab-bag state, and checks every
unambiguous `.run` deck/relic fact for agreement. On any missing required
field, it logs `provenance_entry_incomplete`; on disagreement, it logs
`provenance_entry_conflict`. Either result discards the whole save projection
for that fight and regenerates with slice-2 behavior. It never mixes a saved
deck with guessed relic or unlock state.

When admitted, the saved deck array is the physical deck in opening-shuffle
order, including exact upgrade, enchantment id/amount, and modeled mutable
card properties. The saved relic array supplies acquisition/dispatch order
and strict persistent properties. These facts remove copy-assignment
degradations and their corresponding Goopy/property/dispatch-order
`simulation_refusal` messages. Exact saved unlock epochs replace the A10
assumption, so `metadata.assumed_fully_unlocked` is false and
`metadata.unlock_pool_source` records the evidence even when
`metadata.fully_unlocked_card_pool` is false.

Stable `degraded_reasons` values are:

- `upgrade_copy_assignment_ambiguous`
- `enchant_copy_assignment_ambiguous`
- `combat_card_selection_counter_unknown`
- `combat_targets_counter_unknown`
- `combat_energy_costs_counter_unknown`
- `combat_card_generation_counter_unknown`
- `combat_potion_generation_counter_unknown`
- `combat_orb_generation_counter_unknown`

### `status: "ok"`

All Phase 1a `ok` fields remain present: `fight`, `actual`,
`best_actual_seed`, `benchmark`, `derived`, and `metadata`.
`degraded_reasons` is empty. `metadata.assumed_fully_unlocked` is the only
required new metadata field. `best_actual_seed.seeded_by` is additively
present as `recorded_line` only when the associated replay reproduced the
`.run` combat-end HP and its constructive action line seeded this solve.

### `status: "ok_benchmark_only"`

The document contains `fight`, `actual`, `benchmark`, `derived`,
`degraded_reasons`, and `metadata`. It deliberately omits
`best_actual_seed`: the recorded seed cannot be replayed exactly enough to
make that claim.

`actual` and `benchmark` have the Phase 1a shapes and meanings. Benchmark
worlds start from the exact canonical entry multiset and fresh faithful Phase
0 streams. `derived` retains stable chip-shaped objects but marks both claims
unavailable:

| path | value |
|---|---|
| `derived.skill_gap.available` | `false` |
| `derived.skill_gap.hp_lost` | `null` |
| `derived.skill_gap.exact` | `false` |
| `derived.skill_gap.kind` | `unavailable` |
| `derived.skill_gap.display` | `null` |
| `derived.luck_percentile.available` | `false` |
| `derived.luck_percentile.percentile` | `null` |
| `derived.luck_percentile.worse_worlds` | `null` |
| `derived.luck_percentile.tied_worlds` | `null` |
| `derived.luck_percentile.world_count` | requested K |
| `derived.luck_percentile.exact` | `false` |
| `derived.luck_percentile.kind` | `unavailable` |

`metadata.elapsed_seconds.best_actual_seed` is `null`, because that solve was
not run. The configured actual-seed deadline remains in
`metadata.deadlines_seconds.best_actual_seed` so configurations retain one
shape. All other metadata fields retain their Phase 1a definitions.

### `status: "refused"`

The Phase 1a structured-refusal shape applies. A provenance-seeded exact solve
that somehow falls below its replayed constructive score uses the distinct
`solver_below_replayed_line` reason rather than
`solver_below_observed_outcome`. Schema version, empty `degraded_reasons`, and
`metadata.assumed_fully_unlocked` are added. No partial actual or benchmark
claim is emitted.

## Required Phase 1b UI suppression

- On `ok_benchmark_only`, render the actual outcome and benchmark/loss-risk
  surfaces, plus an honesty-footer explanation derived from
  `degraded_reasons`. Do not render skill-gap or luck-percentile chips.
- On `ok`, render the luck chip only when
  `derived.luck_percentile.kind == "exact"`. Suppress it when the kind is
  `achieved`; Phase 1a measured a 60-point deadline-induced percentile shift
  on Colony.
- Every win-rate/loss-risk label must retain the configured horizon. A loss
  means no winning line within that horizon, not an unbounded proof that the
  shuffle loses.
- When `metadata.assumed_fully_unlocked` is true, disclose the A10 pool
  assumption in the honesty footer.

## Load-bearing invariance test

`test_real_copy_assignment_fixture_has_invariant_benchmark` uses
`8DVXPWUWRY.run` fight 0. Its later Strike upgrade can be assigned to
different physical starter copies. The test generates a benchmark-only card
under two such concrete endpoint assignments. For every K world it captures
the real `sample_worlds` output and directly asserts identical Draw order and
identical state for Shuffle, MonsterAi, and every future combat RNG stream;
complete benchmark-document equality is a secondary assertion. This makes
#120 C1's identical-copy shuffle-key premise executable independently of the
mocked solver result. A separate negative test pairs an ambiguous upgrade with
differing enchant attributes and proves that a real multiset fork refuses.

## Re-run of the Phase 1a 20-fight corpus sample

Measurement host: Sean's local Mac, 2026-08-09. The sample selection exactly
repeated Phase 1a: 355 filename-sorted `.run` files, 20 evenly spaced rounded
filename quantiles, cycling rounded fight positions 0%, 25%, 50%, 75%, and
100%. Configuration was K=10, horizon 12, 5-second actual-seed deadline,
2-second per-world benchmark deadline, and sampling seed
`phase1b-corpus-2026-08-09`. No manual unlock or RNG-counter assertion was
supplied.

| result | fights | rate | sequential wall time |
|---|---:|---:|---:|
| full `ok` | 2 | 10% | 41.42 s |
| `ok_benchmark_only` | 6 | 30% | 115.71 s |
| `refused` | 12 | 60% | 2.50 s |
| **eligible total** | **8** | **40%** | **157.13 s** |
| all attempts | 20 | 100% | 159.63 s |

The six benchmark-only cards were:

| run / fight | encounter | degraded reasons | exact worlds | cost |
|---|---|---|---:|---:|
| `1774674717.run` / 0 | Toadpoles Weak | upgrade copy assignment | 6/10 | 18.57 s |
| `1778134239.run` / 4 | Punch Construct | CombatTargets counter | 1/10 | 24.49 s |
| `1779683171.run` / 0 | Nibbits Weak | upgrade copy assignment | 10/10 | 9.42 s |
| `1780703734.run` / 3 | Living Fog | CardGeneration + CardSelection + Targets counters | 1/10 | 23.64 s |
| `1781370170.run` / 1 | Seapunk Weak | upgrade copy assignment | 4/10 | 16.65 s |
| `1781414909.run` / 2 | Toadpoles Weak | upgrade copy assignment | 0/10 | 22.94 s |

The 12 refusals were two event-node entry ambiguities; three unsupported
encounters (Corpse Slugs twice and Knights); and one each for mutable Goopy
amount, Joss Paper persistent state, same-hook relic acquisition order,
enchanted/cost-overridden X-cost behavior, generated Dazed physical identity,
Pantograph's boss heal, and an unknown CardSelection counter whose two
placeholder captures prove that the stream was consumed before the sampling
seam (Stone Cracker upgraded different card types).

The issue's planning estimate was approximately 16/20 eligible. Current-main
measurement is 8/20: eligibility still rises fourfold from Phase 1a's 2/20,
but removing the first refusal exposed independent I5 blockers that Phase 1a's
first-error taxonomy could not show. In particular, the four fights formerly
classified only as missing unlock assertions became one benchmark-only card
(Living Fog) and three refusals; none became a full card. Those secondary
failures are intentionally not widened into this issue.

At this K=10 configuration, an eligible card averaged 19.64 seconds in the
sample (20.71 seconds for a full card, 19.29 for benchmark-only). Refusals
remained effectively fail-fast except for one reachable 2.44-second refusal.
