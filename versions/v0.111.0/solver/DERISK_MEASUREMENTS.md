# Derisk measurements: player-side coverage + counter-ambiguity growth (#58)

Measured 2026-07-10 over the local run history
(`~/Library/Application Support/SlayTheSpire2/.../saves/history`) by
`versions/v0.111.0/solver/coverage_report.py`, using the card-behavior census
(`versions/v0.111.0/solver/cards_census.json`, from `versions/v0.111.0/solver/tools/census_cards.py`).

**Dataset:** 477 .run files → 447 parseable schema-9 runs (30 skipped:
schema-8 / unreadable), 46 victories, **1804 elite/boss fights** (435 in
victories), 44 runs on the replay-exact build v0.108.0.

## 1. Player-side coverage (the big unknown — now quantified)

What the current solver models: 12 cards, 4 relics, 3 potions, 2 encounters.

- **Encounters:** the two modeled elites already cover **12.9%** of all
  elite/boss fights encountered. The seven unmodeled Act-1 elites
  (Decimillipede 126, Byrdonis 124, Entomancer 122, BygoneEffigy 118,
  Gardeners 116, InfestedPrisms 114, PhrogParasite 113 fights) + the two
  modeled ones ≈ **59% of all elite/boss fights are Act-1 elites**.
  Modeling all 9 is the highest-leverage encounter work.
  [CORRECTION 2026-07-10 (Sean): "Act-1" was a mislabel — floor
  distributions put Decimillipede/Entomancer/InfestedPrisms in **Act 2**
  (floors 24-33). The nine encounters and the 59% fight share are
  unchanged; they span Acts 1+2. See ENCOUNTER_CENSUS.md correction 5.]
  (Old builds reference removed content — e.g. DOORMAKER_BOSS exists only
  ≤ v0.104 — so encounter coverage is per-build; the census matches
  v0.108.0.)
- **Cards are the long pole.** 447 distinct unmodeled cards appear across
  fights (332 within victories). Fully-card-covered fights today: 0.3%.
  Frequency-ordered modeling curve (all elite/boss fights):
  top-100 → 29.6%, top-200 → 59.3%, top-400 → 95.7%.
  **Act-1 fights only** (the launch scope, 1006 fights):
  top-50 → 25%, top-100 → 47.5%, top-150 → 68%, top-200 → 80%.
  Victory runs are the worst case (deepest decks): top-200 → 44%, full
  coverage needs essentially the whole obtainable pool (~400).
- The top of the frequency list is cheap: TREMBLE / GREED / CLUMSY are
  curses (mostly unplayable — trivial to model), then Ironclad/Silent/Regent
  commons. A **~150-card ladder ≈ 2/3 of Act-1 elite fights** reviewable.
- **Relics: 251 distinct unmodeled** (204 in victories) — the second-largest
  surface, same ladder approach applies (top of list: Lantern, Anchor,
  BagOfPreparation, OddlySmoothStone — mostly simple, but each combat relic
  must be exact).
- **Potions: ~30 distinct** — small surface, mostly one-line effects.

**Strategy implication:** full "review any fight of any run" needs
near-complete pool modeling (bulk extraction pipeline territory — the card
census already automates cost/type/keywords; effect semantics are the manual
part). But the launch cut is well-defined: 9 Act-1 encounters + ~150 cards +
~40 relics ≈ two-thirds of every Act-1 elite fight ever played in this
history, with honest "not yet reviewable" for the rest (coverage-ledger
model).

## 2. Shuffle-counter ambiguity growth (the Act-3 worry — answered)

304 of 593 cards are counter-forking (114 powers, 101 self-exhaust, draw /
exhaust-other / add-card effects). Monte-Carlo over unrecorded play
scenarios (200 samples/run, 44 replay-exact runs; real Shuffle RNG;
card-adding effects ignored → counts are lower bounds):

| fight index | mean distinct counters | max |
|---|---|---|
| 0 | 1.0 | 1 |
| 4 | 5.1 | 20 |
| 8 | 10.4 | 22 |
| 13 | 19.9 | 45 |
| 19–22 (Act 3 depth) | ~30–36 | 47 |

**Growth is roughly linear in fight index, not exponential.** Even allowing
for sampling undercount and ignored card-adds, Act-3 fights look like
dozens-to-low-hundreds of candidate counters — each candidate solvable in
seconds, before applying the self-consistency pruning (filter hypotheses
against each prior fight's recorded turns/damage) that should cut deeply.
The "state space explodes and we're left with nothing" scenario is ruled
out; the realistic cost is "solve N hypothesis variants per late fight and
present ranges."

Caveats: draw amounts approximated (2) in the scenario sim; card-adding
cards (56 exist) not simulated; per-scenario feasibility uses real draws
(a card can only be played on a turn it was actually in hand under that
scenario's own history).
