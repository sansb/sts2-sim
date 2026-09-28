# Fight-review schema v7: the document says what produced it (#1267, #1258)

Schema v7 is the v6 contract plus producer identity. The v4 replay gate,
unresolved-world classification, status vocabulary, trust metadata, v5
generated-line addresses and v6 recorded line are all unchanged.

## The problem v7 fixes

A v6 document could not be invalidated. Its producer fields were the schema
version, the worker's generator configuration, and `metadata.solver_build` —
which despite the name held `run.build_id`, the **game** build. Nothing in a
document referred to the simulator, so correcting combat semantics for a fixed
game build left every stored document looking current, and the worker's
freshness check had nothing to compare against.

That matters because of the #1258 defect: a review of an older run under
current combat semantics is confidently wrong, and I5 does not catch it — I5
refuses the *unmodeled*, while this is *mis-modeled for that version*.

## `metadata.simulator`

Every document — card, benchmark-only card, **and refusal** — carries:

```json
"metadata": {
  "game_build": "v0.111.0",
  "simulator": {
    "game_build": "v0.111.0",
    "game_commit": "41cef1ea",
    "dll_sha256": "9cb4f1ad…",
    "bundle": "versions/v0.111.0/solver",
    "sim_revision": 1,
    "bundle_digest": "…",
    "parser_revision": 1,
    "pipeline_revision": 1
  }
}
```

`metadata.game_build` is the **run's** build, renamed from `solver_build`.
`metadata.simulator.game_build` is the build the **bundle models**. Admission
(I11) makes them equal on any non-refusal document, and the producer asserts
it rather than assuming it: a disagreement raises instead of stamping a
document that would look ordinary. They differ only on a refusal, which is
exactly the `unadmitted_game_build` case — the run's own build is in
`refusal.details` there.

Refusals carry identity because a refusal *reason* is a claim the simulator
makes ("this bundle does not model that"). Modelling the mechanic, or
admitting the build, changes the answer, so a refusal that cannot be
invalidated never regenerates.

Field by field:

| field | what changes it | who declares it |
| --- | --- | --- |
| `game_commit`, `dll_sha256` | a new admitted build | `versions/admitted_builds.json` |
| `bundle_digest` | any byte of the bundle's semantic surface | **nobody — computed at run time** |
| `sim_revision` | a named era of world-model fidelity | `sim_identity.SIM_REVISION` |
| `parser_revision` | a named era of parsing / entry reconstruction | `sim_identity.PARSER_REVISION` |
| `pipeline_revision` | a declared change to shared search/estimator/producer | `solver/pipeline_revisions.json` |

`bundle_digest` is the load-bearing one. The declared revisions are legible
era markers whose forgetting costs legibility; the digest is recomputed from
the live files on every run, so a semantic edit invalidates documents whether
or not anybody remembered anything.

## Sampling is separated from fidelity

`metadata.sampling` states how hard we looked, so that a reader comparing two
documents can tell "we looked harder" from "the answer changed":

```json
"sampling": {
  "k": 20, "sampling_seed": "review-pipeline-v1",
  "worlds": 20, "worlds_won": 17, "worlds_unresolved": 2,
  "win_rate_interval": {
    "kind": "clopper_pearson", "confidence": 0.95,
    "low": 0.622, "high": 0.968, "basis": "win_rate_lower_bound"
  }
}
```

Two different uncertainties, with different cures. **Sampling** uncertainty is
finite K, and the interval is the exact (Clopper–Pearson) binomial interval —
exact in the coverage sense, which matters at K=20 and at the degenerate
0-of-K and K-of-K where a normal approximation collapses to a point and lies.
**Resolution** uncertainty is worlds whose solve hit its deadline; those are
counted as non-wins, so with any of them present `basis` is
`win_rate_lower_bound` and the interval bounds a lower bound rather than the
win rate itself. `metadata.sampling` is `null` on a refusal and on any
document with no sampled worlds.

The existing `benchmark.win_rate` / `loss_risk` `kind` fields are unchanged
and remain the per-rate exactness statement.

## Invalidation

Two mechanisms, deliberately different.

**Automatic, by hash.** The worker asks the bundle for its identity once at
startup (`review_summary_v2.py --simulator-identity`) and folds it into
`generator_config_hash`. Any change to any of the five identity components
changes that hash, so the existing freshness check regenerates every affected
document with no backfill and no hand-written query. This fires on *any*
declared change including a `quality_only` one: regenerating costs idle worker
time, whereas trusting a misclassification costs a false answer.

**By query, for a human.** The identity is in the document body, so any
component can be selected on:

```sql
select count(*) from public.fight_review_documents
where document->'metadata'->'simulator'->>'bundle_digest'
      is distinct from '<current>';
```

`sim_identity.minimum_valid_pipeline_revision()` gives the floor below which a
stored `pipeline_revision` is revoked outright rather than merely stale.

## Compatibility classification

Every change to the shared pipeline declares one of `equivalent`,
`quality_only`, or `correctness_invalidating` in
`solver/pipeline_revisions.json`, argued in the PR body. A registry entry
without a declared class refuses to load — left inferrable, `quality_only`
becomes the default because it is the cheap one, and invalidation quietly
stops firing. `correctness_invalidating` revokes every lower revision **across
every build**: the pipeline is shared, so a false optimum it published is
false wherever it was published.

Note the scope limit while #1275 is open: search, the estimator and the review
producer still live *inside* the bundle, so `bundle_digest` currently covers
them too. When #1275 hoists the shared layer out, it must add a
`pipeline_digest` over the hoisted files, or the automatic half of this
protection silently narrows to fidelity alone.

## The worker's staleness detector

The review CLI is read fresh from disk per fight while the worker process is
long-lived, so a merge can split them — the 2026-08-10 incident, where a
pre-bump worker validated v2 against a CLI emitting v3. v7 makes that loud:

- at startup the worker refuses a CLI whose reported `schema_version` is not
  the one it was built for, naming the `launchctl kickstart` remedy;
- per document it compares the stamped `metadata.simulator` with the identity
  it keyed freshness on, and fails the fight if they differ. Three consecutive
  failures stop the worker, which is the correct end state for a process
  running against a bundle that changed underneath it.

## Compatibility and regeneration

The production worker requires schema 7. Its generator identity includes the
schema version, so every stored review regenerates through the normal worker
loop after deployment. Schema 2–6 documents remain readable by compatible
frontends; they record no producer identity and are the population #1268
purges.

The database column `fight_review_documents.solver_build` keeps its historical
name in v7 — renaming it is a coordinated migration-then-merge-then-restart
deploy, tracked separately. The document field is already `metadata.game_build`.
