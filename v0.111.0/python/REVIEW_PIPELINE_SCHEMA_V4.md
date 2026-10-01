# Fight-review schema v4: replay gate and unresolved worlds (#1121)

Schema v4 is the v3 contract plus an honest, producer-owned classification
for capped non-wins and a trust gate for recorded provenance lines.

## Outcome classification

Every `best_actual_seed` and `benchmark.outcomes[]` object has `result`:

- `win`: a complete winning line was achieved, even if search later capped;
- `loss`: an exact search proved no win within the turn horizon;
- `unresolved`: search capped without finding a win.

At benchmark level, `wins`, `losses`, and `unresolved` are disjoint and sum to
`world_count`. `losses` therefore means proven losses only. `loss_risk.value`
is the conservative ceiling `(losses + unresolved) / world_count` whenever
unresolved worlds exist, with `kind: "upper_bound"`; its `proven_losses` and
`unresolved` fields expose the two numerator components directly.

Existing exactness fields remain: a capped win is a proven win but its HP,
potion, and turn optimum is still an achieved lower bound, not exact.

## Recorded-line trust gate

When a resolved provenance bundle supplies the player's recorded action line,
the producer must replay it from the modeled entry and match the recorded
combat outcome and ending HP before emitting any benchmark. Divergence yields
the structured refusal `recorded_line_replay_failed` plus the existing
worker-only diagnostic (`turn`, `check`). A run without recorded action
provenance is unchanged and may still produce a review.

Knowledge Demon's blocking curse selection is represented in a stored line as
`{"kind":"select","choice":"MIND_ROT"}` (or the recorded alternative).

## Compatibility and regeneration

The frontend accepts schemas 2–4 and derives the old classification only for
v2/v3 documents. The production worker requires schema 4; the schema-version
change updates its generator identity so every stored review is regenerated.

## Recorded input log

An associated decoded MCR capture adds optional `recorded_log`, independent of
the card status. It is a turn-grouped list using the existing `line` action
vocabulary (`play`, `potion`, `select`, and `end`), but contains only recorded input
facts. In particular it deliberately omits simulated hands, HP, block,
targets, and action-event annotations. That lets an owner see their captured
fight even when the solver refuses it or the replay diverges, without implying
that any part of the log is solver output.

`review-provenance-consumer-v5` regenerates provenance-bearing rows to add this
field; it does not add a new table, migration, or read surface.
