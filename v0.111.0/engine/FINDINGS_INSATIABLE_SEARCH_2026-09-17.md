# Insatiable search experiment — 2026-09-17

Sean's floor-33 fight has winning lines in the current Rust simulator. Best observed: **55 HP, turn 11, 55 actions**, UCT policy seed 2. It replays to the identical final digest through a fresh Rust process. A separate **43 HP, turn 14** line also matches the frozen Python simulator after all 90 actions. These are achieved simulator wins, not an optimality claim or an independently played in-game counterfactual.

Context: [fight](https://relaythespire.com/seanb/7UEE3Y1SPJCY/0/floor/33), [handoff #2558](https://github.com/sansb/StsHistoryViewer/issues/2558), [research #1278](https://github.com/sansb/StsHistoryViewer/issues/1278). Engine base: `1617eec6`. No engine or admission changes.

## Matched-budget pilot

Four policy seeds per method; 30 seconds each, sequentially on the same Mac. Both methods use the same weighted-random rollout: every non-EndTurn legal action has weight 1; EndTurn has weight 0.05. This is **not blind uniform sampling**, and has no card-specific priority table. UCT uses UCB1 with exploration 1.414, one randomly chosen unexpanded action per simulation, mean backup, no transpositions, and a 250,000-node cap. The tree cap was never reached. The random baseline has no tree.

Wins receive `0.5 + 0.5 * HP/maxHP`; other leaves receive `-0.5 - 0.5 * remainingEnemyHP/initialEnemyHP`, clipped to the respective bands. Best-line ranking uses raw combat HP then fewer turns; it does not claim the production potion-retention or post-combat-healing objective. Policy RNG is separate from all gameplay RNG streams. Horizon is inclusive turn 20 and 256 decisions; all eight runs had **zero cutoffs and zero transition refusals**. Every result remains `incomplete`.

| Method | Seed | Playouts | Transitions incl. replay | Win visits | Best HP | Turn | First win: playout / time |
|---|---:|---:|---:|---:|---:|---:|---|
| random | 1 | 70,980 | 3,566,088 | 967 | 46 | 14 | 5 / 0.003s |
| random | 2 | 69,693 | 3,507,631 | 936 | 54 | 11 | 50 / 0.025s |
| random | 3 | 71,139 | 3,580,825 | 990 | 43 | 14 | 88 / 0.038s |
| random | 4 | 66,752 | 3,360,841 | 995 | 50 | 11 | 62 / 0.030s |
| uct | 1 | 79,771 | 3,287,091 | 587 | 43 | 14 | 27 / 0.016s |
| uct | 2 | 70,250 | 2,893,643 | 497 | 55 | 11 | 60 / 0.031s |
| uct | 3 | 79,639 | 3,279,502 | 547 | 44 | 11 | 72 / 0.032s |
| uct | 4 | 70,154 | 2,891,023 | 507 | 44 | 14 | 54 / 0.026s |

All eight runs find a win in fewer than 100 playouts. Weighted random best HP: 46, 54, 43, 50 (mean 48.25); UCT: 43, 55, 44, 44 (mean 46.5). This small pilot provides **no evidence that UCT improves on the rollout baseline here**. Win visits are visits, not distinct lines. Shared-host timings are observations, not isolated throughput guarantees. First-win times exclude loading/admission. Transitions include tree traversal and final retained-line replay.

The DFS three-turn timeout in #2558 concerned exhaustive exploration of a short horizon; it did not measure how difficult finding any full-fight win would be. These runs show that distinction matters on this fight. This is one fight and four seeds, so it says nothing conclusive about UCT across the eval suite. The late correction in #1278 also matters: more budget let the old Queen UCT beat its human line; re-rooting was not demonstrated superior at matched budget.

## Winning line: 55 HP

Execute in order; end each turn except the lethal last turn. HP/enemy HP below are after the turn closes (or after the lethal action). All attacks target the sole boss. The JSON action sequence retains physical identity for duplicate cards and selection ordinals; it is the reproducible reference.

| Turn | Actions | Your HP | Boss HP |
|---|---|---:|---:|
| 1 | Loop → Turbo → Boost Away → Boot Sequence → Clarity potion → Scavenge | 66 | 326 |
| 2 | Claw → Zap → Frantic Escape → Defend Defect → Frantic Escape → Ball Lightning+ | 66 | 301 |
| 3 | Coolheaded → Glacier+ → Go For The Eyes | 60 | 274 |
| 4 | Jack Of All Trades+ → Rolling Boulder → Chill | 60 | 269 |
| 5 | Hologram → return Frantic Escape (uid 35) → Boost Away → Frantic Escape → Lightning Rod → FTL | 57 | 254 |
| 6 | Coolant+ → Equilibrium | 57 | 236 |
| 7 | Shatter → Lightning Rod+ → Go For The Eyes+ → Scavenge+ → exhaust Iteration (uid 11) → Claw+ | 57 | 164 |
| 8 | Ice Lance+ → Frantic Escape → Frantic Escape | 55 | 95 |
| 9 | Coolheaded+ → Frantic Escape → FTL+ → Zap+ → Boost Away+ | 55 | 50 |
| 10 | Glacier → Defend Defect+ → Turbo+ → Boost Away+ | 55 | 1 |
| 11 | Ball Lightning+ | 55 | 0 |

The recorded human line ended on turn 9 at 0 HP with 111 boss HP remaining. This winning line keeps 57 HP through turn 7 and reaches 55 HP on turn 8, then takes no further damage. It uses a generated Rolling Boulder on turn 4 and postpones Shatter until turn 7. These describe the successful trajectory; they are not isolated causal ablations. The line may include redundant actions and is not proven optimal.

## What is verified, and what is not

- The reconstructed opening RNG words/counters match the native recording with zero pre-splice drift. Rust admits the canonical root and matches its entry digest.
- The 44-action human line matches Rust/Python after every action. Native recorded HP, player block, monster HP, energy and turn match at all 33 captured card/potion completion checkpoints.
- All eight retained wins replay through a fresh Rust `diff-serve` process with identical final digest. The 43-HP UCT seed-1 line additionally matches Python at every action.
- The 55-HP line's advisory Python replay reached action 23 (Hologram selection), then exceeded a 45-second process-group limit. It has Rust replay verification; it does not have completed Python lockstep verification. The earlier random seed-1 advisory replay was stopped after a prolonged stall. Neither timeout is a simulation-divergence finding.
- **Full native checksum certification is incomplete.** `checksum_opening_state` installs recorded RNG; despite the external handoff script's printed “opening checksum PASS”, this function does not perform a complete comparison. The full native validator refuses all 33 action checkpoints because it lacks projections for Loop, Coolant, next-turn energy and Clarity. It also flags Feral+'s serialized local-cost carrier (`1` versus absent). The card template prices Feral+ at 1 already; this may be a representation mismatch, but that is not resolved by this experiment. The raw differences/refusals are retained unchanged in `native-validation.json`.

Consequently the strongest statement is **winning lines in the admitted Rust model**, with a 43-HP independent Python parity witness and limited native corroboration of the recorded human trajectory. Do not describe this as full native per-action certification. No counterfactual line has been executed in the actual game.

## Reproduce

From the repository root:

```sh
cargo build --release --locked --manifest-path sim/v0.111.0/engine/Cargo.toml --bin sts-sim --example search_experiment

# Reproduce exactly the recorded seed-2 search count; 120s is a safety cap.
sim/v0.111.0/engine/target/release/examples/search_experiment   sim/v0.111.0/engine/benchmarks/2026-09-17-insatiable-search/entry.json   uct 2 120 1.414 0.05 70250

# Omit the last argument for a wall-time-only experiment. Replace uct with
# random for the baseline; replace 0.05 with 1 for uniform rollouts.
```

`tools/search_experiment.py prepare` takes `--save`, `--mcr`, `--encounter`, `--kind` and `--out`, roots the fight, and records its human/native validation evidence. Its `verify` command also takes `--result`; default verification includes advisory Python lockstep, while `--rust-only` records precisely that narrower check. Raw captures remain outside the repository.

Validation: release build and release Clippy with warnings denied pass; both policies reproduce identical outcomes, transition counts and win visits across repeated fixed-seed 1,000-playout invocations. The final runner reproduces the full seed-2 result at 70,250 playouts. No production search code changed.

## Next research

1. Resolve the native-validator gaps separately from search tuning; preserve the failing evidence instead of weakening a check.
2. Use this fight as a positive smoke case, then compare the same generic policies on a fixed, diverse subset of the existing eval corpus. Include a uniform-policy ablation and equal-transition as well as equal-time budgets. Do not select only fights a new policy happens to win.
3. Measure first-win reliability and best-HP curves over fixed seeds. If discovery is weak on harder fights, compare nested rollouts/beam search; if discovery is easy but refinement stalls, compare max/mean backup and incumbent-guided refinement. There is no current reason to abandon UCT, or to credit it for discovery that the baseline already achieves.

Artifacts: [raw results and replay evidence](benchmarks/2026-09-17-insatiable-search/), [55-HP machine line](benchmarks/2026-09-17-insatiable-search/uct-2.json), [43-HP Python/Rust replay](benchmarks/2026-09-17-insatiable-search/uct-1-replay.json), [experiment runner](examples/search_experiment.rs).
