# Fight-review schema v8: "not yet" is not "no" (#1268, #1258)

Schema v8 is the v7 contract plus one new status. Everything else — the v4
replay gate, v5 generated-line addresses, v6 recorded line, v7 simulator
identity — is unchanged.

## The problem v8 fixes

A run on a build this bundle does not model refused. That is right when the
build can never be admitted, and wrong when it merely has not been yet, for
two separate reasons:

- it tells a viewer their fight is **not reviewable** when we expect to review
  it, which is a false statement about the product rather than about the game;
- it leaves **nothing to query**. A refusal is a completed result, so the
  worker never revisits it. Without a distinct status there is no way to find
  the population that should regenerate when a build is admitted.

Both matter at real scale: at the 2026-08-19 measurement, 702 of the 1,006
non-admitted cards were on builds whose DLL is archived.

## The line: evidence, not age

`versions/admitted_builds.json` is still the sole authority. A build is
`pending` when its DLL was archived — its combat semantics can still be
verified against something, so admission stays possible at explicit cost. A
build **absent** from the registry never had its DLL archived (#309 — Steam
deletes old builds on update), so no future work can make it reviewable.

This is emphatically **not** a version comparison. **v0.108.0 is newer than
v0.107.1 and permanently unreviewable**, because Steam deleted its DLL while
v0.107.1's was archived. Any rule phrased as "older than X" gets this exactly
backwards, which is why `admission.is_deferrable` reads registry state rather
than parsing a version.

| registry state | disposition | status |
| --- | --- | --- |
| `admitted` | reviewed | `ok` / `ok_benchmark_only` |
| `pending` | no result **yet** | **`deferred`** |
| absent | no result **ever** | `refused` |
| `revoked` | terminal by definition — a corrected successor is a new revision | `refused` |

## The document

```json
{
  "schema_version": 8,
  "status": "deferred",
  "fight": {"run_file": "…", "fight_index": 3},
  "deferral": {
    "reason": "game_build_pending_admission",
    "message": "…no result yet rather than no result ever…",
    "details": {
      "game_build": "v0.110.1",
      "registry_state": "pending",
      "admitted_builds": ["v0.111.0"]
    }
  },
  "metadata": {"simulator": {…}, "…": "…"}
}
```

`deferral` mirrors `refusal`'s shape under its own key, and carries a
machine-readable `reason` for the same reason a refusal does: a consumer must
be able to tell "not yet" from "no" without parsing prose. A deferred document
carries full `metadata.simulator` — it is a produced result, and it needs
identity precisely so it can be invalidated.

The terminal case keeps `reason: "unadmitted_game_build"` unchanged, so
documents already stored under v7 keep their meaning.

## What makes a deferral regenerate

`metadata.simulator` gains **`admitted_builds_digest`** — a sha256 over every
build's *state* in the registry. Admission decides whether a fight yields a
card, a deferral, or a terminal refusal, so the registry produces the answer
as surely as the engine does, and belongs in producer identity by exactly the
argument that put `bundle_digest` there.

The consequence is the point: admitting a build changes that digest, which
changes `generator_config_hash`, which makes every stored document stale and
regenerates the deferred population **by construction** — no backfill, no
query anybody has to remember to run.

Only build *states* are hashed. Rewording an `evidence` string must not
invalidate a single document.

## Consumers

- **Worker**: `deferred` joins `VALID_STATUSES`; `NO_CARD_STATUSES` is
  `{refused, deferred}` and drives both the CLI's exit code (2 — the caller
  asked for a review and did not get one) and the validation that a no-card
  document carries a machine-readable reason under its own key.
- **Database**: migration `0027` widens the `status` check. It **must be
  applied before the schema-8 worker merges** — that constraint is exactly
  what would reject a deferred upsert.
- **Frontend**: `renderFightReviewState` renders deferrals as
  "Not reviewed yet — this run is on an older game version the reviewer has
  not been built for." Deliberately neither "not reviewable" (claims we never
  will) nor "coming soon" (claims we will). The registry makes no promise, and
  `pending` is explicitly not a queue commitment.
  `reviewcard.render` still returns null, so a deferral never renders as a
  card.
- Schema 2–7 documents remain readable. v8 adds a status rather than changing
  one, but the version is bumped because the status vocabulary is part of the
  contract a consumer switches on.
