# Solver research development set — 2026-09-17

This extends the existing eval tree rather than creating a second captured-fight
corpus. The main manifest now holds 202 fixtures (176 recorded lines and 26
refusals); 175 have the historical Rust/Python-lockstep `certified` tag.
That tag is not full native certification. Search results below are model
results; none of the new counterfactual lines has yet been verified in game.

## Frozen experiments and results

**These manifests predate the #2915 re-seed**, which limited the fixture tree
to A9/A10 and rebuilt it from the Rust census. They still name fixtures that
re-seed removed. The results stay here as a record. To re-run a manifest,
check out the fixture tree from the parent of the #2915 merge. New search
suites select from the current tree, and its `node:boss` / `node:elite` tags
pick out the hard fights.

All manifests record selection policy, game build, search seeds and budgets.
Results retain every best action witness, root/final digests, improvement
history, playout counts, node counts, cutoffs, failures and binary hash.
`replay-validation.json` records separate-process Rust replay checks.

- `pilot-v1.json`: eight fights selected before running the matrix: the
  longest available recorded death for each character where present,
  otherwise a long recorded line, another boss, two short controls, and
  Insatiable. Three search seeds, random and UCT, two seconds each.
- `pilot-v1-results.json`: all 48 searches found wins. Five of the eight
  fights saturated at the same HP for both methods; Insatiable favored random
  at this budget, while Entomancer and Ceremonial Beast favored UCT.
- `pilot-v1-equal-playouts.json`: same matrix with 1,000 playouts each, all
  completed below the two-second ceiling. Random won 22/24, UCT 24/24.
  Equal playouts isolate sample efficiency, not equal transition work.
- `mining-v1.json`: all 175 lockstep-tagged fixtures, random seed 1, 0.3 seconds.
  165 wins, two completed no-win searches, eight unsupported/failed searches.
  Retained witnesses exceed recorded final HP or rescue a recorded loss in
  115 of 175 cases; the designated A10 Ironclad comparison subset has 81
  such cases out of 116 screened (110 completed, six refused). These are raw
  combat-HP comparisons, with unrestricted potion use, not established
  player-skill or full-run-value improvements.
- `challenge-v1.json`: the two no-win cases from mining, five seconds per
  method and three seeds. Defect A1 Entomancer `fc53621e544a3773` still has
  zero wins in six searches. Ironclad A10 Terror Eel `ff048525e2583950` yields
  1-HP wins in 3/3 UCT searches and 1/3 random searches. Both recorded human
  lines died on turn one. No-win is not an impossibility result.
- `stress-v1.json`: explicitly synthetic Insatiable states with post-opening
  HP changed to 33, 16 or 8. RNG, deck and all other modeled state stay fixed.
  These are stress tests, not captured fights or evidence that a real run can
  reach those roots. They are outside native and human-comparison denominators.

The synthetic states expose a known-feasible search miss. At 33 HP the
short searches won 5/6 times, retaining 5–9 HP. At 16 and 8 HP they found no
wins in 6/6 searches each. Independently replaying the original 55-HP witness
from the altered roots wins with **22 HP from 33**, and **5 HP from 16**;
it dies from 8. `stress-known-witnesses.json` preserves those replays.
Thus the 16-HP case is demonstrably solvable in the model, while both short
search policies miss it. The 8-HP case remains unresolved. No search was
seeded with those witnesses.

Selected two-second results (best HP for each of three search seeds):

| Fight | Random | UCT |
|---|---|---|
| Insatiable, Defect A9 | 37 / 44 / 41 | 34 / 35 / 31 |
| Entomancer, Ironclad A10 | 7 / 7 / 7 | 12 / 14 / 12 |
| Ceremonial Beast, Ironclad A10 | 58 / 51 / 45 | 58 / 53 / 56 |

At 1,000 playouts, Entomancer is a clearer discriminator: random loses twice
and wins with 1 HP once; UCT wins with 4, 4 and 3 HP. This is a reason to keep
UCT in the research portfolio, not enough seeds or independent fights to
claim a general advantage. Random search remains an essential baseline.

The search implementation and objective are fixed by
[`search_experiment.rs`](../../rust/examples/search_experiment.rs): exact
captured RNG, single-root UCB1, mean backup, uniform non-end legal actions,
End Turn weight 0.05, 20-turn/256-action horizon and 250,000-node cap. There is
no proof of exhaustiveness. Potion expenditure and post-combat healing are
not part of its raw combat-HP objective. The human line does not seed search.

## Does Rust throughput explain the wins?

`throughput-insatiable.json` measures the same 44 recorded actions in both
engines, with actions pre-resolved and serialization outside the timed loop.
Three batches: Python roughly 1,900 transitions/s; Rust roughly
90,600–103,100/s; median rate ratio **53.47×**. A later run of the checked-in
driver measured 54.36×. This is a shared-host observation on one trajectory,
not a universal engine speedup or an end-to-end solver speedup.

Throughput clearly changes the affordable search budget. It does not by itself
explain why Insatiable is easy for these policies: every initial search found
its first win in fewer than 100 playouts. Our rollout policy's strong preference
for playing over ending a turn, fixture-specific structure, and old search
policy/coverage differences are alternative contributors. The original
30-second experiment did not show a UCT advantage on this fight. We have not
ported the identical search runner back to frozen Python, so do not attribute
all historical solver improvement to the language change.

## Unsupported counterfactuals

Root admission plus the recorded human trajectory did not cover these paths:

| Fixture | Search refusal |
|---|---|
| f3939bb49c5afa08 | ContinuationNotModeled |
| f75ba52dc713643b | Thrash damage candidate |
| fb064ff8b20d0bed | Foul Potion player share after a combat-ending enemy share |
| fc56f6e7f04ab583 | monster forced follow-up owner/state |
| fc5e053094d551c0 | random AI weights |
| fea2a7ad8ee352e9 | ContinuationNotModeled |
| ff58b542499979bf | Hopper Swipe death entry |
| ffc19a45c017c3bf | ContinuationNotModeled |

The full action prefixes and verbatim errors are in `mining-v1-results.json`.
A failed search contributes no successful result to this report, even if it
printed an earlier improvement before refusing. Preserve these cases in the
coverage queue; do not count them as losses, silently prune the offending
moves, or widen admission to make a search pass. No simulator mechanics were
changed in this work.

## Reproduce

From the repository root:

```sh
cargo build --locked --release --manifest-path versions/v0.111.0/rust/Cargo.toml --bin sts-sim --example search_experiment --example replay_benchmark
python3 versions/v0.111.0/rust/tools/search_suite.py --manifest versions/v0.111.0/eval/search/pilot-v1.json --out /tmp/pilot.json
python3 versions/v0.111.0/rust/tools/search_suite.py --manifest versions/v0.111.0/eval/search/pilot-v1.json --playouts 1000 --out /tmp/equal-playouts.json
python3 versions/v0.111.0/rust/tools/search_suite.py --manifest versions/v0.111.0/eval/search/challenge-v1.json --out /tmp/challenge.json
python3 versions/v0.111.0/rust/tools/search_suite.py --manifest versions/v0.111.0/eval/search/stress-v1.json --out /tmp/stress.json
python3 versions/v0.111.0/rust/tools/verify_search_suite.py /tmp/pilot.json /tmp/equal-playouts.json /tmp/challenge.json /tmp/stress.json --out /tmp/validation.json
```

Three tools behind the committed data were retired by #2999 (under #2827),
because each rooted or replayed through the Python simulator that #2827 item
F deletes. Their outputs stay as frozen data, sha256-pinned by
`versions/v0.111.0/rust/tools/frozen_oracle_data.py`, and the tools are in git
history before #2999:

* `generate_search_stress.py` wrote `stress-v1.json`,
  `stress-known-witnesses.json` and `generated/insatiable-hp{33,16,8}.json`;
* `search_experiment.py prepare` rooted the Insatiable fight in Python and
  wrote `../../rust/benchmarks/2026-09-17-insatiable-search/`'s `entry.json`,
  `human-replay.json` and `native-validation.json`. Its Rust-only `verify`
  now lives in `verify_search_suite.py`, and it returns the same result on
  every committed witness;
* `replay_throughput.py` wrote `throughput-insatiable.json`, a
  Python-vs-Rust transition timing.

Wall-time runs reproduce the experiment, not an identical playout count.
Use `--playouts` with enough wall time for deterministic fixed-work checks.
All data here is development data. The challenge cases were selected because
random search failed: they must not be presented as an unbiased algorithm
ranking or repurposed as a held-out benchmark.

## Next experiment decisions

Keep the broad corpus and a separate small challenge list. Split future
captures by run/player before tuning so adjacent fights cannot leak between
development and held-out sets. Include successful human play, deaths, long
fights, every character, narrow selections, potion decisions, generated cards,
healing, and counterfactual refusals. Preserve easy controls too.

Compare random, current UCT, heuristic rollout UCT, and turn-level beam search
at equal wall time and equal transition work. Use paired seeds, more than
three replications, time-to-first-win, best-HP-over-time curves, potion cost,
regret against best known feasible witnesses, and coverage as separate axes.
Keep loss and cutoff value shaping fixed while comparing tree policies.
Archive action witnesses whenever a new best is reached. Establish ground
truth optima only for deliberately small exhaustible controls.

## Ovicopter partial recording

`ovicopter-partial-v1/` retains an A10 Ironclad opening and 20 recorded
inputs ending during turn 6. It is deliberately **not** a terminal human
line. Fourteen native checkpoints cover HP/max HP/block, energy/turn, living
monster identity/HP/block, ordered card identities/upgrades in all piles,
and all nine RNG streams. The Rust `ovicopter_capture` integration test
checks these directly. Full power/affliction/cost serialization is outside
this fixture's certificate. It catches egg-slot reuse after deaths and
Armaments selection reordering; the frozen Python engine still diverges at
those points. Search-generated lines require separate in-game verification.

## Queen complete recording

`queen-capture-v1/` retains the floor-48 Queen opening for Y3NULJSNND7N
and the complete five-turn human loss. Its Rust integration test checks 28
completed native action checkpoints: HP/max HP/block, energy/turn, living
monster identities/HP/block, ordered card identities/upgrades, and all nine
RNG streams. It also roundtrips every suspended choice through the canonical
boundary. This is a development regression fixture, not held-out evaluation.
Full power/affliction/cost serialization and counterfactual winning lines
still need independent verification.
