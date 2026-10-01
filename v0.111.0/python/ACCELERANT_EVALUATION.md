# Accelerant evaluation: bulk coverage measurements (#58, 2026-07-10)

Question from the last session: can auto-translating IL-template-simple
cards "cover half the ladder without touching the exactness guarantee"?
Short answer: **the accelerant is real (43% of all cards are
template-simple), but no partial-breadth strategy moves fight coverage —
the four dimensions are jointly binding, and meaningful coverage requires
the whole catalog.** The catalog is bounded and mechanical, though, and
this session's censuses sized every piece.

Tools built: `tools/template_census.py` (+ `template_census.json`),
`tools/census_relics.py` (+ `relics_census.json`),
`tools/card_callsets.py`, `accelerant_report.py`, and the four-dimension
ledger + greedy ladder in `coverage_report.py`.

## 1. The four-dimension ledger (448 runs, 1815 elite/boss fights, 170 replay-exact)

Per-fight blockers are computed exactly as `start_combat` refuses them:
unmodeled (card, upgrade) pairs, non-whitelisted relics, unmodeled
potions, enchanted cards, unsupported encounters.

| dimension | fights with dim clear | median blockers/fight |
|---|---|---|
| cards | 0.7% | ~7 distinct unmodeled cards |
| relics | 10.1% (2.2% before the inert census) | ~6 |
| potions | 12.5% | 2 |
| enchantments | 41.5% | 1 |
| encounter | 59.0% | 0 |

**No dimension is individually binding.** A greedy fights-unlocked-per-
item ladder is nearly flat: 200 optimally-chosen items reach only 52% of
replay-exact fights; the top-40 picks unlock 9 fights. There is no
20-item or 150-item shortcut — DERISK_MEASUREMENTS' "~150 cards ≈ 68%"
was a cards-only number and does not survive the joint constraint.

## 2. Universe sizes (all observed in the local history)

| dimension | distinct in history | total in game |
|---|---|---|
| cards | 459 | 593 classes |
| relics | 256 | 298 classes |
| potions | 60 | — |
| enchantments | 20 | — |
| encounters left | — | 15 (3 Act-3 elites + 12 bosses) |

Everything is bounded. The program is whole-catalog modeling, delivered
by pipelines rather than per-item hand reads.

## 3. Card accelerant measurement

Every card's behavior surface is highly regular: numbers are declarative
(`get_CanonicalVars` Decimal ctors + `OnUpgrade` `UpgradeValueBy`),
behavior is an `OnPlay` async body whose commands come from a small
vocabulary. Classifying all 593 cards by their OnPlay call closure
(generic `PowerCmd::Apply<X>` args resolved via MethodSpec blobs):

| category | cards | meaning |
|---|---|---|
| simple_now | 84 | safe vocabulary only, powers already modeled |
| simple_needs_power | 158 | safe vocabulary + unmodeled power(s) |
| inert | 11 | unplayable, no behavior (curses/statuses) |
| complex_calls | 241 | calls outside the vocabulary |
| hooks | 76 | non-OnPlay gameplay hooks |
| keyword | 17 | Innate/Retain/etc. not simulated |
| xcost | 5 | `ResolveEnergyXValue` |

Safe vocabulary today: attack (incl. hit-count/all-targets), block,
draw, energy, hp-loss, apply-power. **253/593 (43%) are auto-
translatable** once their ~90 distinct powers are modeled (powers are
the hand-unit: one ShriekPower-sized IL read each; EnergyNextTurnPower,
RetainHandPower, PlatingPower, PoisonPower top the list).

The complex bucket decomposes into a *second vocabulary tier* — command
families the sim can extend exactly, each unlocking a card family:
`CardCmd::Exhaust` (13 cards), `CardPileCmd::AddGeneratedCardToCombat`
(26), `CardCmd::Upgrade` (19), `CardCmd::Discard` + select-from-hand
commands (~25), deterministic state reads (pile contents, combat
history — the sim knows both). Card generation (`CombatCardGeneration`
stream, 33 consumers) stays blocked on pool + UnlockState extraction as
documented in STREAM_CONSUMERS.md.

## 4. Relic census (landed this session)

Hook-name classification over all 298 relic classes (inheritance-aware):
a relic is combat-inert when every hook fires strictly outside the
combat window — post-combat effects need no modeling because entry state
is read from the .run. Result: **111 inert relics whitelisted into
`KNOWN_RELICS`** (relic dimension 2.2% → 10.1%).

Exactness catch worth remembering: `AfterRoomEntered` fires on entering
a combat room, *before* the fight — **BronzeScales applies its
ThornsPower there**, not in any combat hook. The census body-scans that
hook's closure for combat commands; the scan caught 15 relics
(Vajra, Girya, Oddly Smooth Stone, Philosopher's Stone…) that hook-name
classification alone would have wrongly whitelisted. Unknown hook names
default to combat-active. Pinned by `test_relic_census_sanity`.

The ~187 combat-active relics are mostly one-hook one-liners (Lantern:
`AfterSideTurnStart`; Anchor: `BeforeCombatStart`; Akabeko, Happy
Flower, Blood Vial, Bag of Marbles…) — the same template approach
applies to relic hooks and is likely *more* tractable than cards.

## 5. Cumulative program scenarios

What the full history would look like as each program phase lands
(each row includes all rows above it):

| scenario | all fights | replay-exact |
|---|---|---|
| S1 today (ledger + inert-relic census) | 0.2% | 2.4% |
| S2 + card accelerant (253 cards + their powers) | 0.4% | 2.9% |
| S3 + all 60 potions | 0.8% | 2.9% |
| S4 + all 20 enchantments | 1.6% | 5.3% |
| S5 + ALL cards (complex hand-modeled too) | 9.8% | 11.8% |
| S6 + ALL relics | 59.0% | 62.9% |
| S7 + ALL encounters | 100% | 100% |

The jump only happens when cards AND relics are both essentially
complete (S6); the residual 37% is the encounter dimension (Act-3
elites + bosses). Partial strategies flatline below ~12%.

## 6. Recommended build order

1. **Enchantment layer** (20 items; already promoted last session — top
   of the replay-exact greedy ladder: Sharp, Glam, Nimble, Tezcatara's
   Ember are picks 1/2/3/21).
2. **Active-relic templates**: hand-model the ~30 most frequent one-hook
   relics, then template the rest by hook shape.
3. **All potions** (60, mostly safe-vocabulary effects).
4. **Card accelerant build-out**: template translator for the 253 +
   tier-2 command extensions + the powers ladder.
5. **Act-3 elites + bosses** (the final 37%, incl. the two random-AI
   fights and the two counter-forking bosses).

The exactness guarantee is untouched throughout: auto-translation only
ever *adds* modeled items whose IL matches known-safe shapes; everything
else keeps refusing (I5), and every new mechanism class still gets an
invariant walk + pinned test.
