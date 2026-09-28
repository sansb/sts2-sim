# Fight-review schema v3: per-sample solver lines (#1089)

Schema v3 is the v2 contract (`REVIEW_PIPELINE_SCHEMA_V2.md`) plus exactly
one addition: every solved-outcome object — `best_actual_seed` and each
entry of `benchmark.outcomes[]` — carries a `line` field, the turn-grouped
play log of the solver's winning line for that world. Nothing else moved:
statuses, refusal reasons, degraded reasons, derived chips, and the
metadata block are byte-compatible with v2 apart from
`schema_version: 3`.

Producer: `review_summary.line_document()` serializes the same
exact-engine replay `solve_state` already performed. Presentation follows
the live-coach conventions: card names via `solve_fight._cname`, monster
targets via `solve_fight._mon_label` (observable names only — the #118
lesson), event text mirroring `solve_fight.narrate`. Keep
`review_summary._event_text` in sync with `narrate` when a new engine
event lands.

## The `line` field

`null` in exactly three cases:

- **Losses** — no winning line exists within the horizon; the solver's
  non-winning search prefix is deliberately not stored.
- **Bare recorded-floor substitutions** (`observed_outcome_floor`) — the
  stored numbers are the player's recorded result, which no known line
  achieves because `.run` files record no player actions. An associated MCR
  that constructively replays to the same combat-end HP supplies the real
  line instead and stamps `seeded_by: "recorded_line"`.
- Historical v1/v2 documents read by a v3-aware consumer (absent key).

Otherwise it is a non-empty array of turn groups:

```json
"line": [
  {
    "turn": 1,
    "hp": 57,
    "block": 0,
    "hand": ["DEFEND_IRONCLAD", "STRIKE_IRONCLAD+", "IMPERVIOUS", "CINDER", "BASH+"],
    "actions": [
      {"kind": "play", "card": "DEFEND_IRONCLAD"},
      {"kind": "play", "card": "CINDER", "target": "41hp SLUDGE_SPINNER",
       "events": ["CINDER hits for 18 (enemy 23)", "exhaust STRIKE_IRONCLAD"]},
      {"kind": "end", "events": ["enemy OIL_SPRAY hits for 0 (hp 57, block 1)"]}
    ]
  }
]
```

Per turn group: `turn` (int, the engine's turn counter at group start),
`hp`/`block` (player, at group start), `hand` (visible hand, display
names), `actions` (ordered).

Per action: `kind` ∈ `play | potion | select | end`.

- `play`: `card` (display name); `target` (observable monster label) when
  targeted; `exhaust` (display name) for exhaust-cost plays.
- `potion`: `potion` (raw id, e.g. `SPEED_POTION`); `target` or `cards`
  when the potion takes one.
- `select`: `cards` (display names; may be empty).
- `end`: no extra fields.

Any action may carry `events`: an ordered list of display strings from the
engine's per-action log (hits, enemy moves, exhausts, reshuffles). Events
are presentation text, not a stable machine surface — decision-level
tooling (Phase 2) should consume the structured `kind/card/target` fields
and re-derive anything else from the engine.

## Semantics and honesty rules (unchanged from v2, restated)

- A line from a `deadline_hit` world is an **achieved** line — replayable
  and real, but not proven optimal; regeneration can produce a different
  line for that world. UI copy must not call a capped world's line "the
  best line".
- Benchmark lines are the **solver's** play. When
  `best_actual_seed.seeded_by == "recorded_line"`, the actual-world solve
  started from the player's constructively replayed MCR line; the returned
  line may be that seed or a reviewer-found improvement. Bare `.run` files
  still record no in-fight actions. UI copy must describe that provenance
  without claiming the line is certainly one source or the other.
- All v2 UI suppression rules apply unchanged.

## Compatibility and rollout

- **DB**: no migration. `supabase/migrations/0023` checks
  `schema_version > 0` plus document/column consistency; both hold for 3.
- **Frontend**: `src/reviewcard.js` accepts schema 2 and 3; v2 documents
  simply render no line affordances. Ship the frontend before or with the
  producer bump (Pages deploys on merge; the worker only produces v3 after
  its host pulls and restarts).
- **Regeneration**: the worker's `GeneratorConfig.identity()` includes
  `schema_version`, so bumping `review_worker.SCHEMA_VERSION` changes
  `generator_config_hash` and the worker regenerates every stored row on
  its next pass. No manual backfill step.
- **Size** (measured, `8DVXPWUWRY.run` fight 1, production config K=20):
  34.9 KB compact JSON with lines vs 5.5 KB without — ~6× growth,
  comfortably within jsonb norms for owner-only rows. Revisit only if a
  corpus fight lands far outside this envelope.

## v1

`review_summary.py` (schema v1) shares `outcome_document` and therefore
also emits `line`; its `SCHEMA_VERSION` stays 1 and
`REVIEW_PIPELINE_PHASE1A.md` remains the field authority for everything
else. v1 output is a development surface only — nothing stores it.
