# Review refusal census (#1034)

> **Retired (#2827 item F1).** `tools/refusal_census.py` and the classify-only
> seam it read were deleted with the Python review producer; the snapshot
> below and `tools/refusal_census.json` are kept as the dated record.

Measured on 2026-08-27 from base `36c10e7d0365998f67b4e6d62f39cb59fb46c97f`. This is an aggregate-only local measurement: no run filename, seed, user, deck, action, path, or refusal prose is retained.

**Headline: every one of the 5,991 classified fights stops at the build-admission gate. This is a version-frontier census, not a current-build content-refusal distribution; first-blocker masking makes every later content or modeling gap invisible here.**

## Authority and limitations

The classifier follows `review_summary_v2` through parsing, build admission, exact entry reconstruction, its benchmark-only retry, and recorded-length validation, then stops before replay, sampling, or search. It therefore measures the context-load frontier, not a promise that a later reachable action will not refuse.

Only the first blocker is visible. A refused fight can contain additional masked blockers, and an eligible context can encounter a later I5 refusal during a real solve.

The classify-only seam lives in the review producer and therefore participates in its bundle digest and worker-restart contract. The aggregate snapshot lives under excluded `solver/tools/`, so later measurement-only refreshes do not change simulator identity.

## Corpus

- Input runs: 518
- Parsed runs: 517
- Parse failures: 1
- Classified fights: 5991

### Context-load outcomes

| outcome | fights |
|---|---:|
| `deferred` | 1076 |
| `refused` | 4915 |

### By build

| build | outcome | fights | runs |
|---|---|---:|---:|
| `v0.102.0` | `refused` | 56 | 3 |
| `v0.103.0` | `refused` | 461 | 45 |
| `v0.103.2` | `refused` | 453 | 34 |
| `v0.104.0` | `refused` | 627 | 46 |
| `v0.105.0` | `refused` | 74 | 6 |
| `v0.105.1` | `refused` | 200 | 12 |
| `v0.106.1` | `refused` | 498 | 42 |
| `v0.107.0` | `refused` | 1518 | 130 |
| `v0.107.1` | `deferred` | 892 | 78 |
| `v0.108.0` | `refused` | 735 | 69 |
| `v0.109.0` | `deferred` | 41 | 3 |
| `v0.109.1` | `deferred` | 49 | 5 |
| `v0.110.1` | `deferred` | 94 | 7 |
| `v0.99.1` | `refused` | 293 | 29 |

### Normalized blocker classes

| class | fights | runs |
|---|---:|---:|
| `unadmitted_game_build` | 4915 | 416 |
| `game_build_pending_admission` | 1076 | 93 |

### Triage dispositions

| disposition | fights |
|---|---:|
| `untriaged` | 5991 |

The machine-readable snapshot retains all 924 build/encounter/outcome rows. The human report intentionally summarizes them rather than repeating the full admission table.

Every parsed fight stops at the build-admission gate. This 518-run historical corpus therefore measures the version frontier, not current-build content/modeling gaps; those later classes are completely masked in this snapshot.
No row is promoted to `fix`, `relax`, or `provenance`: the admission frontier supplies no content-level evidence from which to derive one of those dispositions.

## Provenance-aware comparison

- Paired context classifications: 0
- Sampled full dual solves: 0
- Unavailable sanitized records: 1
  - `sanitized_pair_build_not_admitted`: 1
- No current-build sanitized provenance pair exists, so the census reports no provenance improvement and performs zero full dual solves. The historical v0.110.1 fixture is parser evidence, not authority for the admitted v0.111.0 bundle.

### Context eligibility and trust by mode

| mode | provenance tier | outcome | trust tier | fights |
|---|---|---|---|---:|
| — | — | — | — | 0 |

### Sampled full-card coarse differences

| dimension | changed | unchanged |
|---|---:|---:|
| `status` | 0 | 0 |
| `trust_tier` | 0 | 0 |
| `benchmark_hp_loss_band` | 0 | 0 |
| `skill_gap_availability` | 0 | 0 |

Only changed/unchanged counts leave memory. Status, trust tier, the benchmark HP-loss p10–p90 band, and skill-gap availability are compared; card bodies, values, and identities are not stored.

## Product-weighted frontier

Unavailable. Issue #1110 remains an unimplemented proposal and there is no authorized coarse aggregate. This census does not query production storage or derive users from local paths. Unique users, full-review yield lost, and cumulative top-N share remain null until #1110 supplies an aggregate that excludes raw decks, actions, seeds, review prose, and row identifiers.

## Cadence

Re-run the private local sweep after changes to review eligibility, build admission, provenance consumption, or modeled coverage. CI runs mutation-sensitive public-fixture/seam tests and verifies this Markdown renders byte-for-byte from the checked-in JSON; it cannot honestly regenerate a private corpus it does not possess.

Manual refresh:

```text
python3 sim/v0.111.0/python/tools/refusal_census.py --history HISTORY_DIR --write
```
