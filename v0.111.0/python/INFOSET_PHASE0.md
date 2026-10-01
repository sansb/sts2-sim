# Information-set sampler Phase 0 (#1019)

> **Retired (#2827 item F1).** `infoset_sample.py` and
> `infoset_phase0_exhibits.py` were deleted with the Python review producer
> they fed; no review samples benchmark worlds any more (benchmark-only cards
> were dropped, #3004). This page is kept as the dated Phase 0 record.

## Prototype

`infoset_sample.py` wraps the deterministic simulator and the one-call Rust
exact-solve adapter; it does not change either one. A caller loads the normal
fight-entry `State`, replays the recorded player-action prefix with
`replay_observed_prefix`, and passes that decision state to `sample_worlds`.

Each world keeps the complete observed state fixed: seen piles, current hand,
HP, block, energy, potions, monster HP/status, and the currently telegraphed
intents. The remaining Draw-pile multiset is permuted by the existing
`sts2_rng.Rng.shuffle`, and that same fresh Shuffle state drives later
reshuffles and random-position inserts. Fresh named `RunRngSet` states drive
future Niche, CombatCardSelection (random discard/exhaust),
CombatCardGeneration, CombatPotionGeneration, CombatTargets,
CombatEnergyCosts, and CombatOrbs outcomes. MonsterAi is replaced only when
the current fight contains a modeled random-AI consumer; deterministic enemy
scripts keep the recorded stream state.

Sampling is reproducible from an experiment seed. It never uses Python's
`random` module or a hand-written shuffle. Simulator and loader refusals
propagate unchanged.

For each named candidate, the prototype forces its action sequence in the
same K worlds and calls the Rust clairvoyant solver for the continuation.
Ranking is win rate, then mean final HP, potions kept, and fewer turns.
Means and medians are exact `Fraction` values internally; p10/p90 are
nearest-rank empirical outcomes (no interpolated imaginary world). HP and
potion distributions contain winning worlds; win rate always contains all K.

### Information-set assumptions

The entire order of the remaining Draw pile is treated as hidden. This is
valid while no modeled mechanic reveals or fixes draw-pile positions; Phase 1
must revisit the conditioning boundary if such a mechanic lands. The current
modeled set has random-position inserts but no such reveal/fixed-position
effect.

Reproduce the pocket exhibits with:

```text
python3 sim/v0.111.0/python/infoset_phase0_exhibits.py eel -k 40 --max-turns 8 --deadline 0.5
python3 sim/v0.111.0/python/infoset_phase0_exhibits.py colony -k 40 --max-turns 8 --deadline 0.5
```

The experiment seed for every table below is `phase0-2026-08-08`.

## Eel T3

The recorded prefix reaches #120 C1's exact state: player 59 HP, Eel 122 HP
with visible Crash 24, hand Impervious/Bash+/Defend/Strike/Strike, 3 energy.
The actual-seed columns are the exact clairvoyant results documented in #120
and re-solved locally without hitting a 30-second deadline.

| sampled rank | forced T3 turn | actual-seed final HP | K=40 final HP mean / median / p10 / p90 | wins | exact before 0.5s deadline |
|---:|---|---:|---|---:|---:|
| 1 | Impervious -> Strike | 57 | 55.65 / 57 / 47 / 59 | 40/40 | 6/40 |
| 2 | Impervious -> Defend | 57 | 52.15 / 55 / 47 / 59 | 40/40 | 10/40 |
| 3 | Defend -> Strike -> Strike | 45 | 42.75 / 43 / 33 / 45 | 40/40 | 0/40 |
| 4 | Bash+ -> Strike | 35 | 35 / 35 / 35 / 35 | 40/40 | 0/40 |

All lines kept zero potions (the Speed Potion is part of the observed T1
prefix). The robust information-set result preserves C1's important
conclusion: the Impervious turns dominate the non-Impervious turns. The
within-family Strike-over-Defend ordering is **provisional**: most of those
K=40 solves hit the deadline, so their achieved HP values are per-world lower
bounds. Different subtree hardness and rollout-seed quality can bias those
bounds systematically by candidate, making the 55.65-versus-52.15 gap
untrustworthy until both candidates are run all-exact. The visible-state
argument still explains why Strike could have value in other worlds
(Impervious already walls Crash while six damage can matter), but the capped
measurement does not establish that tie-break.

A longer K=10 check with a 5-second per-solve ceiling produced the same order
(means 56.8, 54.3, 44.2, 35); 26/40 candidate/world solves completed exact.
Every deadline fallback in both runs is an achieved, replayable solver line,
but not a proof of that world's optimum.

## Colony T2 / C2 answer

The recorded T1 prefix reaches 67 HP with visible Zoom, hand
Bash+/Strike/Feel No Pain/Ascender's Bane/Bludgeon, and 3 energy. The
13-HP-loss clairvoyant line plays Bash+ and Feel No Pain, ends the turn, and
deliberately takes Zoom; the recorded 33-HP-loss line instead spent T2 on
Bludgeon. Complete T2 turns are forced below, so the named decision cannot
fuse a different within-turn continuation.

| sampled rank | forced T2 turn | actual-seed final HP with optimal continuation | K=40 final HP mean / median / p10 / p90 | wins | exact before 0.5s deadline |
|---:|---|---:|---|---:|---:|
| 1 | Bash+ -> Feel No Pain -> end (eat Zoom) | 54 | 52.52 / 54 / 47 / 54 | 40/40 | 29/40 |
| 2 | Feel No Pain -> Strike -> end | 53 | 49.92 / 49 / 44 / 54 | 40/40 | 10/40 |
| 3 | Bash+ -> Strike -> end | 51 | 49.10 / 51 / 42 / 51 | 40/40 | 24/40 |
| 4 | Bludgeon -> end (recorded T2) | 51 | 48.77 / 50 / 42 / 51 | 40/40 | 20/40 |
| 5 | End turn | 46 | 41.35 / 41 / 36 / 45 | 40/40 | 0/40 |

All lines won and kept zero potions. A K=10 validation with a 5-second
per-world ceiling completed all 50 solves exactly and gave the same ranking:
eat-Zoom mean 52.4 HP versus Bludgeon mean 47.4 HP.

**C2 answer:** the eat-Zoom setup is not clairvoyance-dependent in direction;
it remains the sampled information-set winner, by 3.75 mean HP over the
recorded Bludgeon turn at K=40 (5 HP in the all-exact K=10 check). The famous
20-HP whole-line gap should not be attributed entirely to T2, however. On the
actual seed, forcing only T2 and then re-solving optimally yields 54 versus 51
HP, a 3-HP decision gap. The other 17 HP of the recorded 34-versus-optimal-54
outcome came from later recorded decisions.

## Latency versus K

Wall time for the five-candidate Colony table on this host. Horizon is turn 8
and the deadline is 0.5 seconds per candidate/world solve. The solver checks
deadlines at its existing coarse-grained boundaries, so wall time is slightly
more than `candidates * K * deadline` in some runs.

| K | candidate/world solves | wall time | seconds per solve |
|---:|---:|---:|---:|
| 5 | 25 | 13.81 s | 0.55 |
| 10 | 50 | 26.98 s | 0.54 |
| 20 | 100 | 53.24 s | 0.53 |
| 40 | 200 | 105.66 s | 0.53 |

The K=40 Eel table (four candidates, 160 solves under the same caps) took
110.73 seconds. The all-exact Colony K=10 validation used a 5-second ceiling
and took 80.73 seconds. Runtime is effectively linear in K in this sequential
prototype. Phase 1 should cache worlds/results and can parallelize independent
world solves without an engine change.

## Strategy fusion

No confirmed strategy-fusion misranking appeared in these two exhibits. Eel's
robust result is only that the Impervious family stays dominant; its capped
within-family ordering is provisional for the deadline-bias reason above.
Colony's sampled winner agrees with the actual-seed optimum and with the
defensive setup intuition.

The distributions are still optimistic by construction: after the forced
decision, the Rust exact solver sees each world's complete future and can tailor every
later play to its draw order. In particular, the reported bands are
best-clairvoyant-continuation bands, not an honest fixed-policy EV. The Phase-0
evidence therefore does not prove strategy fusion absent in general; it says
only that it did not reverse either pocket exhibit. An honest EV-policy
benchmark or true chance-node evaluator remains the v2 check if broader
decision samples expose a disagreement.
