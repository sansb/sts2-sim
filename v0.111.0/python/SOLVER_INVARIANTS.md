# Solver invariants registry (#58)

Every entry is an assumption the solver/sim leans on for **correctness**,
with its executable guard where one exists. Rule for the card/relic ladder:
**when modeling a new card family, walk this list before merging** — the
NOT_YET incident (2026-07-10) is the template: the "final HP ≤ current HP"
pruning assumption was silently false the moment healing existed, and no
pinned fight could have caught it.

Guards live in `test_combat_sim.py` unless noted.

> **Frozen record since 2026-09-25 (#2827 item F).** The Python simulator
> (`combat_sim.py`, `solve_fight.py`, `content/`) was hard-deleted together
> with the test modules that guarded it. `test_combat_sim.py` survives only
> as `test_run_parser_contracts.py`, which keeps its run-parser, census and
> `dotnet_sort` pins. Guard test names below that lived in a
> deleted module are historical. A Guards field names only files that still
> exist; a deleted module is written without backticks and marked "deleted
> #2827". The Rust crate's cargo tests and `///` IL citations, and the `.mcr`
> certification census (`sim/v0.111.0/engine/tools/eval_suite.py
> census`), carry these invariants now. Walk this registry as before; new
> entries cite Rust guards.

## I1 — Modeled cards and potions match the IL census
Costs, power/Exhaust/Ethereal flags and draw behavior of every entry in
`combat_sim.CARDS` must agree with `cards_census.json` (regenerate with
`tools/census_cards.py` on game patches). Catches transcription errors and
missed keywords at modeling time.
Potion side (#120 D4, 2026-07-14): every modeled potion constant must
agree with `potions_census.json` (`tools/census_potions.py`), which
parses `get_CanonicalVars` **structurally** — array-size and
element-index opcodes are consumed by the grammar, the exact class of
misread that produced the Radiant Tincture incident (the `DynamicVar[]`
size `ldc.i4.2` hand-read as EnergyVar = 2, caught only against the
in-game tooltip). The census also carries each potion's English tooltip
template from the Godot pack; every `{Placeholder}` must resolve to an
IL var.
**Guards:** `test_modeled_cards_agree_with_il_census` (also asserts
`Card.heal` ≡ its steps — no dual source of truth),
`test_modeled_potions_agree_with_il_census`,
`test_potion_tooltips_resolve_against_census`.

## I2 — The memo key covers all mutable state
A field of `State`/`Monster` missing from `key()` = wrong transpositions =
silently wrong optima. Keys are **auto-derived from dataclass fields**;
`State._KEY_CONSTANTS` is the explicit opt-OUT list for per-fight constants.
Adding a field without touching anything is safe (worst case: fewer memo
hits). Only auditing needed: anything added to `_KEY_CONSTANTS` must truly
be immutable within a fight (threat example: a relic that grows thorns
mid-fight would make `thorns` mutable).
Batch 170's direct multiplayer boundary follows the same rule: ordered player
keys/liveness/orb queues, target-keyed Imitation Learning amounts, exact
source-to-clone records, and a pending teammate CardPlay callback are ordinary
keyed fields. Batch 268 adds each represented teammate's permanent and
temporary Strength/Dexterity projections plus the explicit Power-observer
boundary to that same immutable keyed row. A pending callback, transient clone
correlation, stat amount, or observer surface therefore cannot merge with a
quiescent or differently targeted state.
**Guard:** structural (by construction).

## I3 — Rust pruning must upper-bound the achievable objective

Rust is the sole exact-search authority. A prune may discard a state only
when its ceiling is a true upper bound on the full lexicographic objective:
victory, final HP after admitted victory-time effects, potions kept, and fewer
turns. Remaining heals, max-HP growth, repeated/non-Exhausting heal sources,
banked percentage heals, and outside-combat victory listeners must either be
bounded conservatively or make the Rust boundary refuse that entry.
Over-estimating a ceiling is safe but slower; under-estimating it can erase the
optimum and is never acceptable.

Meat on the Bone is the current explicit victory-time seam. It heals 12 HP,
capped at max HP, only when a winning state is at or below 50% max HP. Chosen
Cheese composes an earlier max-HP/current-HP mutation and remains fail-closed
with Meat until that ordering is admitted end to end. Feed and Fruit Juice are
max-HP sources, not ordinary heals; Blood Potion pricing must use the projected
post-growth max HP. Future healing or max-HP mechanics require the same audit.

The retired Python `final_hp_ceiling` and `heal_potential` helpers are not a
fallback or a test oracle. Rust unit tests, the frozen authority corpus,
adversarial exact-DFS checks, and cross-language trajectory certification own
this contract.

## I4 — Rust exact completion proves the horizon optimum

For horizon H, the Rust DFS must enumerate every admitted line that terminates
within H turns, modulo sound memoization and pruning. An `exact` result proves
the best lexicographic objective in that space. `deadline` and `cancelled` may
return only an achieved, replayable line and never claim optimality. A typed
`refused` result is final for that request.

Python submits one canonical state document, invokes `sts-sim exact-solve`
once, and replays any returned UID-anchored action line only for boundary
verification and rendering. It never enumerates search nodes, resumes an
incomplete Rust result, or falls back after a refusal. Rust owns legal-action
enumeration, transition, memoization, alpha seeding, deadline handling, and
line reconstruction. Unknown protocol/status/refusal values fail closed.

Any deliberate action-space narrowing must be represented in the canonical
entry and disclosed beside the verdict. Unmodeled potions remain inert only
under their declared conditional-optimum contract; strict mode refuses them.
Unsafe multiplayer/pet targeting and multiplayer-only card contexts remain
typed refusals rather than approximations.

**Guards:** Rust `exact_dfs`, `exact_solve_v1`, `solo_v1_authority`, and
Mawler-admission tests; Python `test_review_summary.py` (the
`rust_exact_solve` wire, parser, transport and typed-refusal contracts
live there — it is the only test module that imports it),
test_exact_solve_corpus.py (deleted #2827), `test_python_exact_search_retired.py`, and the
post-submit Rust/Python trajectory certifications (deleted #2827).

## I5 — Unmodeled interactions fail loudly, never approximate
The product's premise is exactness; a guessed mechanic is worse than a
crash. Power replay sources are admitted only as exact source/body
combinations. BaseReplayCount is a generic replay witness over every
registered Power body and composes with Duplicator, eligible Echo Form, and
Throwing Axe. Batch 267 adds SignalBoostPower as the second generic witness:
its owner-Power modifier contributes a constant +1, its changed-listener
callback decrements one stack before the body loop, and it composes with the
same hook sources and BaseReplayCount across the exhaustive registered-body
census. Signal Boost mixed with a fresh enchantment replay refuses atomically
because that mutable enchantment `OnPlay` lifecycle is source-specific.
Without either generic witness, the narrow source/body table remains: W172
`FAN_OF_KNIVES` L0/L1 with Duplicator and/or eligible Echo Form; the four
pre-existing exact Glam-only bodies for Machine Learning L0, Demesne L0, Pyre
L1, and Friendship L0; W199 Pale Blue Dot L0/L1 with fresh Glam only; and
Sneaky L0 with Duplicator only through its private Sly-discard AutoPlay entry.
Every other or mixed Power replay combination raises `NotImplementedError`;
Duplicator has stable refusal precedence when no generic witness is present.
Unknown card ids / upgrade levels
(`NotImplementedError` at `start_combat` naming the card — until
2026-07-14 these leaked as a bare KeyError from `heal_potential`
mid-solve; the FNP incident), unknown relics/enchantments
(`NotImplementedError` at `start_combat`; unknown POTIONS are the declared
exception — see I8/#634 — and fail loudly at use instead of at entry).
When adding cards, prefer a
raise over a "probably fine" translation for anything not IL-read.
The ordinary combat State has exactly one *actionable* player. Raw `.run`,
live-save, replay, and MCR adapters must reject a `players` collection whose
length is not one before selecting any player; direct potion actions must
likewise reject unrepresentable other-player targets before belt removal or
effect dispatch. Batch 170 adds only direct caller-supplied teammate player /
creature projections: no teammate pile, turn, choice, resource spend, or
action is inferred. A summoned Osty is a pet, not another player, and remains
governed by each content-specific ally-target audit.

Ignition can use a teammate queue only while its selected-player Channel
bootstraps or enqueues without eviction and the teammate has no represented
orb hook. A full queue refuses before published mutation because evoking its
front orb can touch unrepresented teammate block/energy and cross-player
listeners. Imitation Learning's teammate path is a blocking externally
observed Before/After callback; owner death closes it without an After hook or
stack consumption. Its local path refuses another applicable same-event
player power because acquisition order around the awaited nested AutoPlay is
not recorded. Native `Dictionary.Add` source-identity collisions refuse too.
Coordinate and Fade extend only that direct selected-player projection. Their
positive TemporaryStrength/TemporaryDexterity wrappers stack exact amounts on
the selected live player and remove those same temporary aggregates at the
shared player-side end. A represented teammate Power application/removal
observer, malformed stat row, or dead teammate retaining a temporary amount
refuses. Local application or expiry with one of the represented suspendable
AfterPowerAmountChanged listeners also refuses rather than skipping the nested
Strength/Dexterity notification. Teammate turns, resources, piles, attacks,
and block actions remain outside the solver rather than being inferred.
**Guards:** `test_duplicating_a_power_fails_loudly`, the Fan/Glam/generic
Power replay and atomicity pins in `test_batch70_generated_card_loops.py`,
the exhaustive registered-body and source-composition pins in
test_batch267_signal_boost.py (deleted #2827), `test_unknown_card_refused_at_start_combat`, and
`test_multiplayer_run_save_and_forged_oil_target_fail_closed`.

## I6 — RNG stream consumption is exact per line
Shuffle: combat-start Fisher-Yates (n−1 draws), reshuffle = sort + F-Y
(size−1). Modeled explicit Shuffle consumers must likewise pin their complete
pool and draw count. Catastrophe freshly StableShuffles its non-Unplayable
live Draw copy on every iteration; an empty preferred pool consumes zero,
then the independently materialized all-Draw fallback consumes size−1.
Its loop bound is also live: AutoPlayed Apotheosis can upgrade the active
source from two iterations to three before the next bound check.
Beat Down instead freezes one live Discard snapshot filtered to playable
Attacks, fully StableShuffles all `n` candidates (`n-1` draws), then awaits
the first 3/4 exact physical references serially. Its Cards count is frozen
after the shuffle. Each AnyEnemy child consumes one CombatTargets draw in
Beat Down and receives that explicit target, so AutoPlay does not draw twice;
a terminal child suppresses the complete remaining frozen suffix.
Batch 170/268 AnyAlly AutoPlay likewise consumes exactly one CombatTargets draw
over `combatState.Allies.Where(c != null && c.IsAlive && c.IsPlayer
&& c != card.Owner.Creature)` (`CardCmd.AutoPlay` predicate `0x4283f0`):
live teammate players only, excluding self, dead players, and every pet.
`NextItem` consumes that one draw even for a singleton teammate pool.
Manual AnyAlly choices consume none.
Niche: exactly one draw per creature created. **Threats:** cards/relics on
the census fork lists
(draws/adds/shuffles flags; `CardPileCmd.Add` random-position inserts
consume Shuffle), mid-fight creature spawns (one Niche draw each; the
.run's `monster_ids` roster under-counts them —
`solve_fight.NICHE_MIDFIGHT_SPAWNERS` caveats fights after a spawner),
Niche-consuming on-obtain relics (`solve_fight.NICHE_ONOBTAIN_RELICS`
warns).
**Guards:** `test_eel_entry_state` / `test_gardeners_entry_state` /
`test_byrdonis_entry_state_7ma0py7ad4` (in-game verified permutation model +
HP rolls); census flags cross-checked by I1.

## I7 — Damage arithmetic matches the decimal pipeline
`(base + Σadditive) × Πmultiplicative`, block-absorb then HP-loss with
truncating decimal→int casts; all modeled multipliers are dyadic so floats
are exact. **Threat:** any future non-dyadic multiplier (e.g. ×1.1 or /3)
breaks float-exactness — switch those paths to Fraction.
**Guard:** `test_truncation_matches_decimal_casts`.

## I8 — Entry state covers everything that can change the fight
The sim refuses (NotImplementedError) any fight whose entry state contains
things it does not model: **enchanted deck cards** (Glam changes play
count, Nimble changes block — invisible before 2026-07-10 because the
parser dropped the `enchantment` field), **relics outside KNOWN_RELICS**
(each relic must be IL-read as either combat-relevant-and-modeled or
verified inert). Silently ignoring either produced a provably wrong verdict
(solver "optimum" 51 HP lost vs 40 observed on 7MA0PY7AD4 fight 3 — the
deck's Glam Bully and Nimble Defend were dropped).
**Potions outside KNOWN_POTIONS are the one entry-state item that does NOT
refuse (#634).** They are carried in `State.inert_potions`: they occupy
their belt slots (so Belt Buckle's empty-belt check still sees a full belt)
and no action can play them. That is exact — a potion that is never drunk
changes nothing — and it costs only the counterfactual, so the verdict
degrades to a CONDITIONAL optimum (I4) rather than a wrong one. Silently
DROPPING them would not be exact, and refusing the whole fight over one was
overkill: potions persist across fights, so a single unmodeled potion
refused every remaining solve in the run (seed 7XDBEBWZ1REL, floor 9
Colorless Potion, 2026-07-26 live session). Three surfaces stay fail-closed:
an unmodeled potion the record says was USED, any line that tries to drink
one (`apply_action`), and `strict_potions=True` for pin verification.
**Guard:** the refusal branches in `start_combat`; scans in
coverage_report.py (deleted #2827) should count all four dimensions (an unmodeled potion
still blocks the unconditional claim, so the ledger still counts it);
`test_unmodeled_potion_is_inert_not_playable`,
`test_unmodeled_potion_strict_and_used_still_refuse`,
`test_belt_buckle_sees_an_unmodeled_potion_in_the_belt`.

## I9 — Reshuffle equal-key order is the game's exact introsort over true pile order
CardModel.CompareTo ties on (id, upgrade); the game breaks ties with .NET
9's `GenericArraySortHelper` introsort over the discard's LIVE order
(`dotnet_sort.py`, IL-ported swap-for-swap — see ENCOUNTER_MECHANICS.md
Shuffles). This only matters when an (id, upgrade) group's members are
distinguishable (mixed enchantment payloads, or a Glam copy among
group-mates); those fights run `exact_piles`: hand/discard keep true game
order, `key()` stops canonicalizing them, and plays enumerate WHICH
identical copy leaves the hand (flush order is a hidden player choice).
**Threats:** result-location routing can add the played instance to Draw Top
(CardPilePosition 2 = list index 0), while ordinary Discard Bottom remains
append; a replay series resolves and inserts the physical card only once.
Feral additionally routes an eligible zero-energy Attack to Hand Top; a full
Hand redirect preserves Top and therefore prepends it to Discard. Make It So
moves exact off-pile instances to Hand Bottom from a fresh Late snapshot.
Master Planner's permanent combat-local Sly and Midnight's ordered local-cost
rows can make equal siblings diverge by immutable UID. Canonical Sly,
turn-transient `SlyThisTurn`, and permanent local Sly must all use the same
effective predicate, while cleanup removes only the transient dimension.
The Bolas/Thrumming Hatchet BeforeHandDraw family also makes equal siblings
diverge: completed-play history belongs to one exact physical UID, and the
frozen card-listener order can move only that object. Their current/previous
history and frozen listener tuples are keyed; while populated, legal
play/exhaust actions retain UID rather than collapsing payload-equal cards.
Beat Down likewise freezes exact object references after the CompareTo sort
and full Shuffle. Its keyed `(uid, frozen payload)` batch locates a still-live
UID in any pile before each child, observing intervening mutations; only a
fully removed object falls back to the captured native reference payload.
Scrape instead freezes the exact UID order returned by one completed Draw,
then re-locates every UID across all five piles before eagerly evaluating its
live zero/X-cost predicate. Free* cost hooks apply only to a UID currently in
Hand or Play; Corruption and physical local-cost rows retain their ordinary
pile-independent semantics. The selected ordered UID batch is moved serially
to Discard before its keyed delayed-Sly continuation begins.
Imitation Learning also keys exact physical identity: BeforeCardPlayed maps
the source Power UID to a fresh transient clone UID and frozen payload, and
native `Dictionary.Add` collision semantics forbid observing the same source
identity twice. Its outer CardPlay parks in a keyed `after_imitation` frame
after body/enchantment/history but before later player/relic/enemy hooks;
the nested AutoPlay can therefore block and resume without replaying either
side of the listener boundary. The clone record is removed with its
target-keyed power at zero, matching the native power-owned dictionary
lifetime.
Successive top inserts therefore reverse play order, and a redirected
auto-play can become the next draw-top flip in the same multi-flip command.
Any new
player-ordered multi-discard (Gambler's Brew is refused for exact_piles
fights) needs its selection order enumerated; a BCL bump in a game patch
(runtimeconfig != 9.0.x) invalidates the port.
**Guards:** `test_dotnet_sort_small_partitions_insertion_stable`,
`test_dotnet_sort_large_partition_hand_traced` (independent hand trace of
the dumped IL), `test_exact_reshuffle_sorts_the_live_discard_order`,
`test_exact_piles_discard_records_play_then_flush_order`,
`test_exact_piles_enumerates_identical_copy_positions`, and the focused
test_result_location.py (deleted #2827) exact-top/Chaos/replay pins.

## I10 — A predicted Shuffle counter is only exact if prior fights are add-free
Monsters that add cards mid-combat (slimes' Slimed, Phrog/Wriggler
Infections, Entomancer Dazes, … — `solve_fight.CARD_AFFLICTING_MONSTERS`,
IL census) change PRIOR fights' reshuffle sizes by unrecorded amounts, so
`replay_fight`'s counter becomes a baseline+k hypothesis space
(`counter_hypotheses.py` methodology). The 7MA0PY7AD4 fight-3 lesson
(2026-07-10): the "exact" counter 70 was really 70+k, k ∈ {1..3} Slimed
from the fight-2 slimes — the wrong counter produced a solver "optimum"
of 44 vs 40 observed, an I8-style provable wrongness that first looked
like a combat-model bug.
Same class, relic-side (2026-07-12): GREMLIN_HORN in a PRIOR fight drew
one card per enemy death at unrecorded times (including the fight-ending
kill — AfterDeath fires before any win check), shifting that fight's
reshuffle boundaries.
The complete modeled draw-source audit (2026-07-19) extends that rule to
prior ownership of BAG_OF_PREPARATION, RING_OF_THE_SNAKE, PAELS_BLOOD,
CENTENNIAL_PUZZLE, GREMLIN_HORN, PENDULUM, GAME_PIECE, or POCKETWATCH, and
prior recorded use of SWIFT_POTION, GAMBLERS_BREW, CURE_ALL, CLARITY,
DISTILLED_CHAOS, or GLOWWATER_POTION. Later modeled draw sources extend the
relic inventory with BIG_MUSHROOM, BOOMING_CONCH, FIDDLE, POLLINOUS_CORE,
RING_OF_THE_DRAKE, and SNECKO_EYE, and the potion inventory with SNECKO_OIL.
Pile-changing potion selections DROPLET_OF_PRECOGNITION and LIQUID_MEMORIES
also make prior reshuffle boundaries ambiguous despite drawing no RNG: the
`.run` omits their selected physical card and exact use time.
FAKE_SNECKO_EYE does not modify hand draw and therefore is intentionally
absent here. FERAL, MAKE_IT_SO, and MASTER_PLANNER also join the conservative
prior-fight Shuffle caveat inventory because their unrecorded plays can change
pile membership or delayed Sly AutoPlay boundaries. The `.run` does not preserve exact
trigger/play timing or selection/flip bodies, so each makes the predicted
Shuffle counter a hypothesis. BIIIG_HUG retains its separate generated-Soot
and random-insertion caveat.
The same conservative inventory includes CLEANSE, PHOTON_CUT,
SECRET_TECHNIQUE, and SECRET_WEAPON because their selected exact instances
leave, enter, or reorder the ordinary draw/discard cycle at unrecorded play
times.
PURITY likewise removes an ordered optional set of exact Hand cards from the
cycle at an unrecorded prior-fight play/selection point, so later reshuffle
membership and boundaries are hypotheses even though its body consumes no RNG.
SEEKER_STRIKE likewise StableShuffles the complete live Draw pile on the
CombatCardSelection stream and moves one exact shortlisted object out of Draw.
The `.run` preserves neither play time nor chosen identity, so both that stream
counter and all later Shuffle membership/boundaries become hypotheses.
CATASTROPHE likewise repeatedly StableShuffles freshly materialized live Draw
copies on the Shuffle stream and direct-AutoPlays exact selected objects out
of that cycle. The `.run` preserves neither play timing, selected identities,
nested child effects, nor whether an AutoPlayed Apotheosis extended an L0
body to a third iteration, so later Shuffle positions and cycle membership
are hypotheses.
BEAT_DOWN fully StableShuffles a live Discard Attack pool and serially
AutoPlays its frozen exact prefix at an unrecorded point; both Shuffle
consumption and resulting cycle membership/order are therefore hypotheses.
BOLAS and THRUMMING_HATCHET likewise return the exact prior-turn physical
card before the next Hand draw. The `.run` omits that exact play and return
timing, so later flush/reshuffle membership and order are hypotheses.
WELL_LAID_PLANS and prior ownership of RINGING_TRIANGLE or RUNIC_PYRAMID
likewise join the conservative inventory: their pure `ShouldFlush=false`
listeners retain the live hand at unrecorded prior-fight turn ends, changing
cycle membership and later reshuffle boundaries without consuming RNG at the
listener itself.
FAN_OF_KNIVES and UP_MY_SLEEVE generate Shivs which can redirect to or flush
into Discard, while FIGHT_THROUGH inserts Wounds there directly. Their exact
prior-fight play/flush timing is likewise absent, so all three join the same
conservative inventory despite consuming no Shuffle draw in their own body.
COMPACT likewise bulk-transforms every live Discard Status into a fresh Fuel;
the `.run` omits the play boundary, exact Status identities, and later Fuel
plays/exhausts, so prior ownership makes later cycle membership and reshuffle
boundaries hypotheses even though Compact itself consumes no Shuffle RNG.
Burning Sticks likewise creates one exact clone of the first owned Skill
exhausted in a prior fight; the `.run` does not preserve that trigger or the
clone's Hand-versus-Discard insertion timing, so prior ownership joins the
relic draw-cycle caveat inventory.
Hellraiser likewise joins the card inventory: after its unrecorded power
application, every drawn Strike is direct-AutoPlayed and Pommel/Minion Strike
can draw recursively. The `.run` records neither the trigger boundary nor the
resulting exact cycle membership, so later Shuffle boundaries are hypotheses.
Scrape likewise joins the card inventory: each unrecorded play draws a live
returned prefix, evaluates zero/X cost only after every draw hook completes,
then moves that ordered physical subset through Discard and delayed Sly
AutoPlay. The `.run` preserves neither the play boundary nor those identities,
so later cycle membership and reshuffle boundaries are hypotheses.
**Guard:** `shuffle_counter_caveats` in `solve_fight` (printed on every
solve); `test_slime_affliction_caveat`,
`test_gremlin_horn_prior_fight_shuffle_caveat`, and the exact inventory plus
per-source pins in test_batch49_turn_history_relics.py (deleted #2827) and
`test_batch70_generated_card_loops.py`.

## I11 — Combat semantics are admitted per exact game build
A run is simulated only under the build it was recorded on, and only if that
exact build is admitted. I5 refuses what is *unmodeled*; this refuses what is
*mis-modeled for that version* — every mechanic modeled, just not as that build
behaved. Nothing raised for that case before #1258, so a v0.104 run was
reviewed with v0.104 RNG and v0.111 combat semantics and rendered as an
ordinary certified card.

`sim/builds.json` is the sole admission authority. Matching is by
exact build string: no ranges, no `latest` fallback, no default. A build absent
from the registry, or present in any state other than `admitted`, refuses.

Enforcement is at the **review boundary** — `review_summary.load_fight_context`,
where a real run enters the product — and surfaces as a structured
`unadmitted_game_build` refusal card rather than a crash. It is deliberately
not (yet) inside `start_combat`: the engine is reached by ~460 test call sites
built on the `fight_states` pins, which are `build_id: v0.108.0` and can never
be admitted because that DLL is gone. Moving the check inward is #1272's job,
once those pins are recaptured on an admitted build; until then an engine-level
check would refuse the suite that proves the engine correct. A bundle also
refuses a build it does not itself model, so a dispatcher mis-route fails at
the callee rather than succeeding quietly.

Admitting a build is a deliberate act requiring all four criteria in
`sim/meta/SIM_VERSIONING.md`: the archived DLL pinned by sha256, a completed
version-impact triage, a green harness oracle corpus, and regenerated
build-owned data artifacts. The admitted set grows forward only — a build whose
DLL was never archived can never be admitted, which is why v0.108.0 and
everything at or below v0.107.0 are permanently unreviewable at exactness.

The precedent this generalises: `sts2_rng.seeding_scheme` was version-aware
while `combat_sim` was not. It admitted by *range* (`<= (0, 108)`,
`== (0, 109)`) until #1265, extrapolating a derivation verified at v0.108.0
across nine minor versions and 100% of the local run corpus. It is now an
enumeration, `_VERIFIED_SEEDING`, holding exactly the five builds whose
seeding was checked against their own DLL. Both admission surfaces now refuse
by default and admit only on recorded evidence.

What remains of #1265 is the `build=` argument's default
(`GAME_BUILD_V0_108`), which is a silent assumption at every call site that
does not pass one. Removing it is blocked with the rest of the engine-side
work behind #1272.

## I12 — A published document records the simulator that produced it
A review document states what it was produced by, or it cannot be withdrawn
when the producer is corrected. I11 stops a *new* answer being wrong for the
build; I12 is what lets an *old* answer be found and invalidated once we learn
it was.

Before #1267 the only producer fields were the schema version, the worker's
generator configuration, and `metadata.solver_build` — which held the run's
game build under a name that read like the solver's. Nothing referred to the
simulator, so correcting combat semantics under a fixed game build left every
stored document looking current: the freshness check compared a document
against a producer identity that did not exist.

Every document — card, benchmark-only card, and **refusal** — carries
`metadata.simulator`: `game_build` plus the archived artifact it was verified
against (`game_commit`, `dll_sha256`, read from the admission registry, never
restated), `sim_revision` and `bundle_digest`, `parser_revision`, and
`pipeline_revision`, plus `rust_exact_solver`: the native executable's resolved
path, availability state, and sha256. Refusals are included because a refusal
reason is itself a claim the simulator makes; modelling the mechanic changes
the answer.

`bundle_digest` is the load-bearing field and is **computed, not declared** —
a sha256 over the bundle's semantic surface, recomputed on every run, so any
edit to the engine, `content/**`, a census or a template invalidates stored
documents whether or not anybody remembered to bump anything. The surface is
fail-closed the useful way: a file nobody classified counts as semantic, so an
oversight over-invalidates rather than leaving a stale document looking
current. `test_sim_identity.py` derives the review CLI's real import closure
and fails naming anything the digest misses — it caught `tools/live_coach.py`,
which `mcr_replay` imports for entry reconstruction, on its first run.

The executable hash is independently load-bearing now that Rust admission,
transitions, and exact search produce the answer. Replacing `sts-sim` at the
same configured path changes producer identity even when the tracked Python
bundle is untouched. Missing and non-executable candidates are explicit
states, so installation and permission changes also invalidate freshness.

Changes to the **shared** pipeline (search, estimator, parser envelope,
producer correctness) declare a compatibility class in
`sim/meta/pipeline_revisions.json`: `equivalent`, `quality_only`, or
`correctness_invalidating`. An entry without one refuses to load. Left
inferrable, `quality_only` becomes the default because it is the cheap one and
invalidation quietly stops firing. A `correctness_invalidating` change revokes
every lower revision across **every** build — the pipeline is not frozen per
build, so a false optimum it published is false wherever it was published.

The worker folds this identity into `generator_config_hash`, so invalidation
is automatic rather than a backfill, and verifies per document that the CLI
stamped the identity the worker keyed on — a bundle that changes under a
running worker stops it instead of storing documents whose freshness hash does
not match their own contents. Contract: `REVIEW_PIPELINE_SCHEMA_V7.md`.

Scope limit while #1275 is open: search and the review producer still live
inside the bundle, so `bundle_digest` covers them today. The hoist must add a
`pipeline_digest` over the shared layer, or this narrows to fidelity alone.

Identity also carries `admitted_builds_digest`, a hash of every build's state
in the registry (#1268). Admission decides whether a fight yields a card, a
deferral or a terminal refusal, so the registry produces the answer as surely
as the engine does. Only states are hashed — rewording an `evidence` string
must not invalidate a document.

## I13 — A refusal distinguishes "not yet" from "never"
A build whose DLL was archived can still be admitted at explicit cost; a build
whose DLL was never archived can never be, because its combat semantics can
never be verified against anything (#309 — Steam deletes old builds on
update). Those are different answers and are recorded as different document
statuses: `deferred` and `refused`.

**The line is whether the evidence survives, not how old the build is.**
v0.108.0 is *newer* than v0.107.1 and permanently unreviewable, because Steam
deleted its DLL while v0.107.1's was archived. Any rule phrased as a version
comparison gets this backwards, so `admission.is_deferrable` reads registry
state and never parses a version.

Both statuses are stored, because a stored answer is what stops the worker
re-solving the same fight on every scan. What separates them is what happens
next: a deferral makes no claim about the fight and regenerates when its build
is admitted — automatically, because `admitted_builds_digest` is inside
producer identity (I12), so admitting a build restales every document rather
than requiring a backfill anybody has to remember. `revoked` is terminal by
definition; a corrected successor is a new revision, not a reinstatement.

Consumer-facing wording is part of the invariant, not decoration. A deferral
must not be rendered as "not reviewable" (claiming we never will) nor as
"coming soon" (claiming we will) — `pending` is a registry state and
explicitly not a queue promise. Contract: `REVIEW_PIPELINE_SCHEMA_V8.md`.

## Invariant walks

Every mechanic-adding change records a walk of the registry above. Walks
live as ONE FILE PER WALK in [`invariant-walks/`](invariant-walks/),
named `YYYY-MM-DD-slug.md`, where the slug carries the issue (e.g.
`2026-08-25-issue1613-canplay-census.md`) — parallel sessions create
different files, so walk records can never merge-conflict (this file's
append-only log was the repo's most common conflict source before the
2026-07-15 split). Read them in filename order for the modeling history;
start a new walk by copying the shape of the latest.

**Do not add a within-day number.** Walks carried a `NN` sequence until
#1584. It was a shared counter that concurrent lanes allocated
optimistically, so two PRs writing a walk on the same day collided by
construction — moving the conflict out of git and into a test, where
whichever PR merged second paid a full ~10 minute gate cycle to discover
a filename clash. The slug already disambiguates, and the filesystem
already forbids two files with one name. The 523 legacy numbered walks
remain valid and keep their reservations checked.

## Ladder process

The card/relic modeling ladder should track per family: IL read done →
steps translated → census cross-check green (I1 auto) → **invariant walk
(a new file in `invariant-walks/`) done** → pinned test if the family
adds a mechanic class.
