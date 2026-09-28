# Fight-review schema v5: stable generated-line addresses (#1219, #1108)

Schema v5 is the v4 contract plus the producer-owned identity and claim fields
needed by decision URLs and previews. The v4 replay gate, unresolved-world
classification, status vocabulary, and trust metadata are unchanged.

## True-state solve identities

Every present true-state generated line carries a producer-authored `line_id`
slug. The headline actual-state solve is:

```json
{
  "best_actual_seed": {
    "line_id": "best",
    "line": []
  }
}
```

`best` is the well-known semantic id for `/solve/best/{turn}/{k}`. Future
families such as potion-holdback or quest variants must choose semantic slugs
at the producer; consumers must never synthesize a slug from array position.
The field is absent when no generated line exists.

Sample lines remain a separate family. `benchmark.outcomes[]` stays in stable
world-index order within one document, and every item keeps its explicit,
zero-based `world` discriminator. Regeneration may replace the sampled worlds;
that claim-as-of-now behavior is deliberate.

## Producer-owned claim wording

Every solved outcome (`best_actual_seed` and `benchmark.outcomes[]`) carries:

```json
"claim": {"kind": "exact", "display": "exact"}
```

or, for a capped achieved line:

```json
"claim": {"kind": "achieved", "display": "achieved (lower bound)"}
```

Decision previews and UI copy use `claim.display` verbatim. They must not infer
"best" or "optimal" from the presence of a line. The existing
`derived.skill_gap.display` remains the producer-owned source for exact versus
`at least N` skill wording.

## Stable fight floor

Every non-refusal document carries `fight.floor`, the one-based game floor
(`node_index + 1`). This is the path coordinate used by fight-review URLs;
clients must not substitute the ordinal `fight_index`.

## Compatibility and regeneration

The production worker requires schema 5. Its generator identity includes the
schema version, so every stored review regenerates through the normal worker
loop after deployment. Schema 2–4 documents remain readable by compatible
frontends, but they do not expose generated-line decision URLs because they
lack producer-authored ids and claim copy.
