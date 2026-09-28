# Review provenance payload v2 (#1032, #1059)

## Scope and compatibility

The RelayTheSpire uploader may attach an optional `review_provenance` object
to a completed run. This is an evidence tier above the existing `.run` data,
not a new upload requirement. A missing, partial, malformed, or unsupported
review payload must always degrade to the existing card tiers; the run itself
remains valid and uploadable forever.

This document specifies the mod-side v2 producer contract. Server
validation/storage is a separate `area:infra` follow-up. Solver consumption
is future work. The first mod PR does not change any solver Python.

Version 1 was merged but never shipped. Its timestamp heuristic labeled one
save projection as authoritative `entry`, but the pre-release live smoke test
showed that projection was post-combat in 6 of 7 fights. Version 2 removes
that false guarantee and carries unresolved neighboring candidates instead.

## Transport

For a schema-1 single upload, `review_provenance` is adjacent to `run`:

```json
{
  "schema": 1,
  "run": {},
  "review_provenance": { "schema_version": 2, "fights": [] },
  "provenance": {}
}
```

For a schema-2 batch, it belongs to the corresponding run entry:

```json
{
  "schema": 2,
  "runs": [
    { "run": {}, "review_provenance": { "schema_version": 2, "fights": [] } },
    { "run": {} }
  ],
  "provenance": {}
}
```

Per-entry placement prevents evidence from being associated with the wrong
run in a mixed batch. Absence is the only missing-data signal; producers do
not serialize capture errors. The existing endpoint currently ignores the
unknown optional member, so mod-side bundling does not change upload success,
but no evidence is stored until the infra follow-up lands.

## Version 2 shape

```json
{
  "schema_version": 2,
  "fights": [{
    "capture_index": 0,
    "archived_at_utc": "2026-08-09T00:00:00.0000000Z",
    "mcr": {
      "encoding": "base64",
      "byte_count": 29346,
      "file_last_write_utc": "2026-08-09T00:00:01.0000000Z",
      "sha256": "lowercase hex",
      "data": "base64 bytes"
    },
    "entry_candidates": [{
      "observation": "previous",
      "save_last_write_utc": "2026-08-09T00:00:00.0000000Z",
      "save_byte_count": 68412,
      "entry": {
        "save_schema_version": 20,
        "run_rng": {
          "shuffle": { "counter": 46, "s0": 0, "s1": 0, "s2": 0, "s3": 0 },
          "combat_card_selection": { "counter": 0, "s0": 0, "s1": 0, "s2": 0, "s3": 0 }
        },
        "shared_relic_grab_bag": {},
        "players": [{
          "net_id": 1,
          "character_id": "CHARACTER.IRONCLAD",
          "deck": [{
            "copy_index": 0,
            "id": "CARD.STRIKE_IRONCLAD",
            "upgrade_level": 0,
            "enchantment": null,
            "floor_added_to_deck": 1
          }],
          "relics": [{
            "acquisition_index": 0,
            "id": "RELIC.JOSS_PAPER",
            "props": { "CardsExhausted": 2, "EtherealCount": 0 },
            "floor_added_to_deck": 4
          }],
          "unlock_state": {
            "unlocked_epochs": [],
            "encounters_seen": [],
            "number_of_runs": 0
          },
          "relic_grab_bag": {}
        }]
      }
    }]
  }]
}
```

`schema_version` and `fights` are required when the object is present. Every
fight member is independently optional so one failed projection does not
discard valid replay bytes. `fights` is ordered by archive time.

`entry_candidates` contains at most the two neighboring distinct run-counter
epochs observed for the same run, ordered oldest to newest. Repeated saves in
one epoch update its projection without evicting the preceding epoch; this
keeps multiple reward-screen writes from displacing the usual mid-fight
candidate. `observation` is `previous` or `latest`; it describes observation
order only and is never an entry-state assertion. File stamps help diagnose
ordering but are not proof of combat phase. A locally persisted, unshipped v1
record is rewritten on bundle as one `legacy_unverified` candidate so its
exact replay remains useful without preserving v1's false `entry` label.
An already queued v1 bundle cannot recover the neighboring projection and is
therefore omitted from the wire; its normal run payload remains uploadable.

Consumers **MUST** compare each candidate's complete run RNG stream-to-counter
map with the replay's combat-start run-counter map. A candidate is usable as
entry state only when those maps are equal. On exactly one match, consumers may
use that candidate's entire entry projection. On zero or multiple matches,
consumers **MUST** flag the record and treat all save-derived fields as missing;
replay-derived evidence remains usable. Consumers must never select a candidate
by timestamp or array position.

Within a counter-matched candidate, the save arrays are authoritative and
order-sensitive:

- `deck[]` is the physical-copy snapshot at fight entry. `copy_index` is the
  capture-local identity of that array element; omitted save upgrade levels
  normalize to zero, while enchantment id/amount is preserved.
- `relics[]` retains inventory order. Schema >= 19 saves vouch that this is
  acquisition and same-hook dispatch order; `acquisition_index` makes the
  contract explicit. Saved `props` and floor metadata are preserved.
- `run_rng` retains every stream the save supplies, not an allowlist. The
  counter is always retained; schema >= 19 xoshiro state words are retained
  when present.
- `unlock_state`, player `relic_grab_bag`, and
  `shared_relic_grab_bag` preserve pool eligibility and remaining relic-pool
  state without copying the whole save.

Paths, account ids, tokens, and the raw `current_run.save` are never included.

## Evidence-to-refusal map

The public v2 card schema has stable degraded reason codes for copy assignment
and six future RNG streams. Other exactness failures currently share the
top-level `simulation_refusal` class and are distinguished by the solver
message. This table names both levels rather than inventing new reason codes.

| v2 evidence | Refusal/degradation retired |
|---|---|
| `fights[].mcr` exact bytes | Retires `solver_below_observed_outcome` when capped search is seeded with the achieved action line. Enables Phase 2 decision review and action-boundary replay certification. The MCR combat-start snapshot also resolves `simulation_refusal` cases whose message identifies event-node entry ordering, because it records the actual entry side of the event/fight boundary. |
| replay counters plus matched `entry_candidates[].entry.run_rng` | Retires `combat_card_selection_counter_unknown`, `combat_targets_counter_unknown`, `combat_energy_costs_counter_unknown`, `combat_card_generation_counter_unknown`, `combat_potion_generation_counter_unknown`, and `combat_orb_generation_counter_unknown`. The same evidence retires any Shuffle, MonsterAi, Niche, or later `*_counter_unknown` refusal and the Stone Cracker pre-seam refusal (entry consumes CombatCardSelection). |
| matched `entry_candidates[].entry.players[].deck[]` | Retires `upgrade_copy_assignment_ambiguous`, `enchant_copy_assignment_ambiguous`, and `simulation_refusal` messages for mutable Goopy amount or generated-card physical identity when the copy exists at entry. |
| matched `entry_candidates[].entry.players[].relics[].props` | Retires `simulation_refusal` messages for Joss Paper persistent state and future saved-property equivalents. |
| matched ordered `entry_candidates[].entry.players[].relics[]` | Retires `simulation_refusal` messages for same-hook relic acquisition/dispatch order. |
| matched `entry_candidates[].entry.players[].unlock_state` plus relic grab bags | Retires the missing unlock assertion / `assumed_fully_unlocked` dependency for card and potion pools, and supplies exact epoch/relic-pool eligibility instead of an A10 inference. |

The MCR start snapshot deliberately overlaps the JSON projection. The
projection stays useful when a new game build has no regenerated MCR net-id
table yet, and it preserves save-native enchantment amount and ordered
per-copy state in a directly reviewable form.

## Capture mechanics

The game keeps only `replays/latest.mcr`, overwritten at every combat end,
and deletes `saves/current_run.save` at run end. The mod therefore observes
both files on its existing low-priority background worker, following
`versions/v0.111.0/solver/tools/mcr_watch.py`:

1. Baseline an existing replay seen at launch; it belongs to an earlier fight
   and is not re-archived. If the replay is absent at launch, its first
   appearance is the profile's first completed combat and is captured.
2. On each stable save change, parse and retain the two neighboring distinct
   run-counter epochs, updating the latest projection on same-epoch writes. A
   mid-fight save still reports that fight's entering counters.
3. On each stable replay content change, archive both same-run neighboring
   save projections, oldest to newest, with observation labels and file stamps.
   Do not label either projection as entry state.
4. Persist the fight atomically under the mod's own data directory, keyed by
   a one-way digest of run seed. Attach matching-seed captures when the `.run`
   enters the existing disk queue.

The game exposes no read-only atomic save/replay pair or safe fight-lifecycle
hook. The #1059 live smoke test established that it normally writes the
post-combat save before `latest.mcr` (6 of 7 observed fights), often in
different polls. Timestamp selection is therefore unsound for counters,
deck, relics, persistent props, and reward-sensitive pool state. Keeping both
neighbors preserves the usual mid-fight entry save without falsely blessing
the following post-combat save; the binding counter-match rule above is the
only supported disambiguation.

No new combat lifecycle or game-object hook is used. Capture only reads
files and never calls or mutates combat objects, RNGs, saves, or game
behavior. Run-end hooks continue to enqueue strings only.

## Limits and retention

Committed fixtures are roughly 3–66 KiB per replay. Raw entry saves are
48–120 KiB, while each projection is normally a few KiB; base64 adds about
33%. Two bounded projections add tens to hundreds of KiB across a run. A
50-fight run is expected to remain within the 8 MiB run cap.

- maximum raw replay: 128 KiB per fight;
- maximum bundled provenance: 8 MiB per run;
- local unbundled retention: 30 days and 256 MiB globally;
- batch byte accounting includes review provenance and splits before the
  uploader's request budget.

Crossing a limit drops oldest or excess evidence only. It never deletes,
holds, or suppresses a normal run queue item.

## Failure contract

Every discovery, stable read, JSON parse, hash, projection, persistence,
lookup, bundle, and cleanup boundary is best effort and logs a bounded warning
on failure. Capture code cannot throw into game hooks or upload code. A capture
fault must not delay the game, prevent a queue write, poison a batch, move a
valid run to `failed/`, or become a player-visible error. The only product
effect is a lower review-card tier for the missing fight evidence.
