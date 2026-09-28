# STS2 RNG stream consumers — complete call-site map (through build v0.111.0)

Every method in `sts2.dll` that reads an RNG stream getter, found by scanning
all method bodies for calls to `RunRngSet::get_*` / `PlayerRngSet::get_*`
(script: `versions/v0.111.0/solver/tools/`; 2026-07-08). This is the authoritative list of what
can advance each stream's counter — anything not listed here cannot desync a
replay of that stream.

Point-release provenance: #625 compared v0.109.0 with build v0.109.1
(`c8c577f6`) on 2026-07-26. All 43,794 parsed CIL bodies were byte-identical,
with no managed type, field, or method-signature delta, so this call-site map
is unchanged through v0.109.1.

Version-impact provenance: #793 repeated the complete getter call-site scan
against archived v0.109.1 and installed/archive v0.110.1. Both produced the
same 162 unique raw scanner rows. This identity is lossy: the scanner prints
the immediate declaring type, method, and target getter, so repeated nested
state-machine names do not themselves identify the enclosing content owner.
Manual owner recovery for the four removed/add row pairs found compiler
state-machine renumberings only: `Shuffle d__20 -> d__22`,
`AutoPlayFromDrawPile d__21 -> d__23`, `AfterCardPlayed d__7 -> d__13`, and
Abundance `OnPlay d__5 -> d__6`; Abundance's CombatCardGeneration getter is
still read at v0.110.1 IL `0092`. No stream gained or lost a semantic
consumer, so the map below was current through exact build v0.110.1. The
broader combat/content model stayed fail-closed until #794 later closed.

Version-impact provenance: Batch 293 / #1214 repeated the exhaustive scan
against archived v0.110.1 and installed/archive v0.111.0/41cef1ea. Raw rows
grew from 162 to 163. The sole new semantic row is
`BeautifulBracelet::AfterObtained -> RunRngSet::get_Niche`; the other streams
retain their prior consumer census. Exact seed/state probes remained unchanged
(267 scalar plus 42 Encounter observations). Batch 296 / #1218 pins Bracelet's
full eligible-pool shuffle and durable acquisition-time `.run` boundary.
Batches 294-298 completed the card, encounter, RNG/relic, Inky, and shared-
combat remodels. v0.111.0 combat is admitted; the final shared-combat delta
adds no stream consumer.

## Shuffle (7 sites) — the replay-critical stream

| site | consumption |
|---|---|
| `CombatManager::SetUpCombat` → `Player::PopulateCombatState` → `CardPile::RandomizeOrderInternal` | combat start: deck cloned **in deck-array order** into the draw pile, then one Fisher-Yates = **n−1 draws** |
| `CardPileCmd.Shuffle` | reshuffle when draw pile empties (and shuffle effects): **size−1 draws**; has `ModifyShuffleOrder`/`AfterShuffle` relic hooks |
| `CardPileCmd.Add` | random-position insert ("shuffle a card into the draw pile"): **1 draw** (`NextInt(count)`) |
| `Cards.BeatDown::OnPlay`, `Cards.Catastrophe::OnPlay`, `Cards.Uproar::OnPlay`, `Powers.StampedePower` | card-specific extra consumption |

Fetch, Right Hand Hand, and Snap consume no RNG directly and are not
CombatTargets consumers: their Osty attacks use fixed targets. They still
join the prior-fight Shuffle uncertainty set because unrecorded Fetch draws,
Right Hand Hand Discard-to-Hand returns, and Snap's permanent Retain mutation
can change later reshuffle membership, size, or boundaries.

Fan of Knives, Fight Through, and Up My Sleeve likewise consume no stream
directly and are not CombatTargets consumers. Their fixed generated
Shivs/Wounds can enter or later flush into Discard, so unrecorded prior-fight
plays change later reshuffle membership and boundaries; all three are in
`solve_fight.PRIOR_FIGHT_SHUFFLE_CARDS`.

## MonsterAi (3 sites)

`MonsterModel::RollMove` (the per-turn intent roll), `Monsters.Fabricator::SpawnBot`, `Powers.FlutterPower::AfterDamageReceived`.

Flutter does not roll a random move. Its current-build listener decrements
only after positive unblocked powered-attack damage; at zero it reads the
owner's `StateLog.Last`, calls that state's deterministic `GetNextState`, and
stuns to the returned state. Batch 219's Hopper projection therefore consumes
no MonsterAi draw when Flutter reaches zero: HAT's next state is the fixed NAB.

## CombatEnergyCosts (3 sites)

`Enchantments.Slither`, `Potions.SneckoOil`, `Powers.ConfusedPower` — all
`NextEnergyCost`. Slither is modeled with one `NextInt(4)` after each actual
draw of its exact card into Hand. Snecko Oil and the Snecko/Fake Snecko Eye
ConfusedPower sources are also modeled: Confused rolls once for each drawn
card whose canonical cost is nonnegative (including X), while Oil awaits an
ordinary Draw(7), re-reads the complete live Hand, and rolls once for each
non-X card whose unmodified base cost is nonnegative. Co-owned Eyes still
install one behaviorally identical Confused listener. Their prior-fight
presence/use is included in `solve_fight.energy_costs_counter_entering`.
The run-lifetime entering counter is exactly zero only across a consumer-free
prefix; otherwise it is unknown and a current Slither, either Eye, or Snecko
Oil refuses unless the caller supplies `--energy-costs-counter`. Captured
saves and live coach restore the recorded `combat_energy_costs` counter
directly. Fights without a
cost-randomizing effect remain untouched.

## CombatCardGeneration (33 sites)

`AfflictionModel::PickRandomTargets` plus cards/potions/relics/powers that
generate cards mid-combat: BundleOfJoy, Discovery, Distraction, InfernalBlade,
JackOfAllTrades, Jackpot, Largesse, MadScience, ManifestAuthority,
Metamorphosis, Quasar, Splash, Stoke, WhiteNoise; ThievingHopper;
Attack/Colorless/Power/Skill potions, CosmicConcoction, OrobicAcid;
CalamityPower, CallOfTheVoidPower, CreativeAiPower, HelloWorldPower,
SpectrumShiftPower; relics BigHat, ChoicesParadox, Crossbow, OrangeDough,
Toolbox, VexingPuzzlebox.

Thieving Hopper filters the solo player's live Draw then Discard piles to
cards with a non-null DeckVersion, chooses the first nonempty rarity-priority
pool (Uncommon; Common/Rare/Event; Basic/Curse; Ancient-or-Imbued), falling
back to the complete ordered eligible list when every predicate is empty,
and calls one `NextItem` on CombatCardGeneration. Only a truly empty eligible
list consumes zero; a Status/Token/Quest-only eligible list consumes one.
The v0.110.1 TQM88QFMHSQR capture pins entering counter 98 selecting the
physical Colossus row and advancing to 99.

## CombatCardSelection (16 sites) — all IL-read 2026-07-11 (#58)

`CardPileCmd.AutoPlayFromDrawPile`; cards Anointed, Cinder, DrainPower,
HiddenGem, SeekerStrike, Thrash, TrueGrit; powers Aggression, Entropy,
Improvement; relics Bookmark, JeweledMask, MummifiedHand, PowerCell,
StoneCracker.

Run-lifetime counter stream (like Shuffle, unlike per-fight Encounter).
Full per-consumer semantics in ENCOUNTER_MECHANICS.md
"CombatCardSelection stream". Key accounting facts:

- Consumption only happens when a consumer is PRESENT — the entering
  counter is exactly 0 for a fight with no consumer in any prior
  fight's deck/relics/potion uses (most fights).
- **Stone Cracker is exactly accountable across fights**: one firing
  per prior combat room, draws = max(0, upgradable_deck_cards − 1) —
  a pure function of the recorded deck (its upgrades hit combat
  clones, never the master deck).
- Consumer CARDS (incl. TrueGrit at level 0 only; TrueGrit+ is a
  FromHand selection) consume per unrecorded PLAY → prior-fight deck
  presence voids exactness (`solve_fight.card_sel_counter_entering`
  returns None + caveats; a current fight holding a consumer then
  refuses in start_combat).
- AutoPlayFromDrawPile callers (token-resolved): MayhemPower,
  DistilledChaos (potion — uses recorded per fight), Cascade, Havoc,
  IAmInvincible — but **every caller passes CardPilePosition Top**, so
  the machinery's NextItem branch is dead code today and auto-players
  consume nothing themselves (only the auto-played card's own OnPlay
  does; live-validated on the HQPAXCBS6P run). ImprovementPower comes
  from the Tinker Time event's rider on a MadScience card (unrecorded)
  and consumes AFTER combat ends — MadScience presence is the
  detectable proxy.
- Card-GENERATION sources (this file's CombatCardGeneration census)
  can create-and-play a direct consumer (a generated Cinder) — their
  presence/use in a prior fight voids cardsel exactness as well
  (`CARDSEL_GENERATION_*` in solve_fight).
- Modeled in the sim: Cinder / True Grit L0 (`exhaust_random` step),
  Seeker Strike (Batch 140 / #646), Entropy (Batch 147 / #413), Stone
  Cracker (start_combat), and the Batch 64 relic family Bookmark, Jeweled
  Mask, Mummified Hand, and Power Cell. Bookmark/Jeweled/Mummified use
  `NextItem` (empty 0, nonempty 1); Power Cell StableShuffles its complete
  filtered pool (`max(0,n-1)`) before `Take(2)`. Seeker Strike likewise
  StableShuffles a sorted copy of its complete live Draw pile
  (`max(0,n-1)`), takes three exact identities, then exposes those identities
  in live Draw order for an exact-one move to Hand. Entropy consumes one
  `NextItem` per transformed exact Hand object, serially, over that original
  card's exact filtered transformation pool. Empty Hand consumes zero.
  Current fights require the exact entering counter. Their unrecorded
  prior-fight events still void exactness and return None+caveats; explicit
  CLI/save/live/MCR counters remain authoritative. Other consumers retain
  their recorded refusals.

## CombatTargets (16 sites) — all IL-read 2026-07-13 (#58)

`AttackCommand::Execute` (random-target attacks), `CardCmd::AutoPlay`; cards
BeatDown, BouncingFlask, TheBall; LightningOrb; powers Cacophony, Countdown,
Haunt, Juggernaut, SerpentForm; relics ForgottenSoul, Kusarigama,
ParryingShield, Tingsha, WhisperingEarring.

Run-lifetime counter stream (like Shuffle/CombatCardSelection); every
"random enemy" pick is `Rng::NextItem` over `ICombatState.HittableEnemies`
(= enemies in slot order where `IsHittable`: alive && ShouldAllowHitting —
for modeled encounters hittable == alive, since ReattachPower's reviving
window is entirely dead). Full per-consumer semantics in
ENCOUNTER_MECHANICS.md "CombatTargets stream". Key accounting facts:

- Consumption only happens when a consumer is present — entering counter
  exactly 0 for a fight with no consumer in any prior fight (most fights).
  There is NO exactly-accountable consumer (no Stone Cracker analogue).
- **Random-target attacks roll once PER HIT** (the NextItem sits inside
  Execute's per-hit loop; validTargets recomputed per hit, dead enemies
  drop out, duplicates-disallowed filters already-hit ones). The count==1
  shortcut exists only on the NON-random path — **single-enemy fights
  still consume**. Player/pet attackers only (the RNG comes from
  `Attacker.Player ?? Attacker.PetOwner`). Real cards (TargetingRandom-
  Opponents callers): FlakCannon, Ricochet, RipAndTear, Stardust,
  SweepingGaze, SwordBoomerang, Volley.
- **The auto-players consume HERE** even though they consume nothing on
  CombatCardSelection: `AutoPlayFromDrawPile` calls `CardCmd::AutoPlay`
  with a NULL target, and AutoPlay rolls `NextItem(HittableEnemies)` for
  any enemy-targeted (TargetType 2) card — Havoc / Cascade /
  IAmInvincible / MayhemPower / DistilledChaos void CombatTargets
  exactness while staying exact for CardSelection.
- **Master Planner is an indirect current/prior consumer**: it permanently
  gives the exact completed Skill Sly, and a later DiscardAndDraw AutoPlays
  that Skill with a null target. An enemy-targeted Skill therefore consumes
  one `CombatTargets` draw through the existing `CardCmd::AutoPlay` site.
  The current deck gate and prior-fight inventory conservatively include the
  power card, using Hand Trick's transient-Sly precedent.
- **Hellraiser is an indirect current/prior consumer**: its sole
  `AfterCardDrawnEarly` listener direct-AutoPlays every exact drawn
  CardTag.Strike. All currently admitted Strike-tag cards are AnyEnemy, so
  each admitted AutoPlay consumes one `CombatTargets` draw before the body;
  Pommel/Minion Strike can synchronously draw and re-enter the same listener.
  The current deck gate therefore requires the entering counter, and prior
  ownership joins the conservative Shuffle caveat because unrecorded trigger
  timing changes draw-cycle membership and recursive draw boundaries.
- LightningOrb Passive/Evoke damage is a null-target roll per trigger;
  every Lightning channeler/granter (Zap, BallLightning, Tempest,
  TeslaCoil, Voltaic, Rainbow; Thunder/Storm/LightningRod powers;
  CrackedCore/InfusedCore relics; Chaos-style random channels) is a
  consumer.
- TheBall and WhisperingEarring's own GetTarget roll draw from
  OTHER-PLAYER pools — empty in single-player, and NextItem on an empty
  pool consumes nothing.
- Modeled direct consumers include **Bouncing Flask** (one fresh roll per
  reached Poison application), **Sweeping Gaze** (one powered Osty hit),
  **Kusarigama** (every 3rd attack card in a turn -> 6 damage to a rolled
  enemy), and **Parrying Shield** (own side-turn end with block >= 10 -> 6
  damage to a rolled enemy), plus the modeled random-attack and Lightning
  families documented in `ENCOUNTER_MECHANICS.md`. Refused consumers remain
  visible through the card/relic censuses; prior-fight consumer presence
  voids the entering counter (`solve_fight.combat_targets_counter_entering`,
  `--targets-counter` to enumerate).

## CombatPotionGeneration (5) / CombatOrbGeneration (2)

Alchemize, EntropicBrew, AlchemicalCoffer, DelicateFrond, PhialHolster /
Chaos, TrashToTreasurePower. Batch 227 models the latter stream as keyed
`State.combat_orbs`, preserving `OrbModel::_validOrbs` order
Lightning/Frost/Dark/Plasma/Glass and one `NextItem` per live callback-loop
iteration. `solve_fight.combat_orb_generation_counter_entering` proves zero
only across a prior-fight prefix free of both direct consumers and every
card/relic/potion route in the shared generation censuses (including
transitive sources such as White Noise); otherwise
`--orb-generation-counter` supplies the explicit hypothesis. Captured saves
and MCR replay use the serialized `combat_orbs` counter directly.

## Niche (18 sites)

`CombatState::CreateCreature` (monster HP rolls etc.), CursedRun modifier,
ToughEgg hatch, and on-obtain relic effects: Astrolabe, **BeautifulBracelet
(new in v0.111.0)**, FishingRod, FragrantMushroom, Kaleidoscope, NeowsBones,
NewLeaf, PandorasBox, RoyalStamp, SandCastle, SereTalon, WarHammer, WarPaint,
Whetstone, WingCharm. Beautiful Bracelet filters the owner's eligible deck
cards, materializes the complete native-order pool, runs backwards
Fisher-Yates (`UnstableShuffle`), and only then takes the first four before
applying Swift 2. A pool of size `n` consumes exactly `max(0, n - 1)` Niche
draws, including pools of four or fewer. Captured save/MCR stream counters
remain exact. Ordinary `.run` history does not retain the eligible deck at
the relic's acquisition node, so `solve_fight` names the consumer and marks
its predicted counter a baseline rather than silently omitting the offset.

Whetstone's `AfterObtained` **0x9de24** is another exact Niche
`StableShuffle`: it copies the owner's Deck cards whose `CardType` is Attack
and `IsUpgradable` is true, sorts by `CardModel.CompareTo` (ID then current
upgrade level), runs the complete backward Fisher-Yates, and only then takes
the first two physical rows. An eligible pool of size `n` therefore consumes
exactly `max(0, n - 1)` Niche draws even though at most two cards are changed.
`tools/map_prediction.py --whetstone` requires the serialized pre-pickup
Niche state and deck, and preserves deck-index identity through the temporary
sort/shuffle; ordinary `.run` history cannot supply that provenance.

## UpFront (5) / UnknownMapPoint (3) / TreasureRoomRelics (1)

Map & room generation (`RunManager::GenerateRooms`, `InitializeNewRun`,
`InitializeSavedRun`), event-node resolution (`RunState::*`), treasure relics
(`RunManager::InitializeShared`).

## player:Rewards (17 sites)

`CardFactory::CreateForReward` / `RollForUpgrade`, `RelicFactory::RollRarity`,
`GoldReward::Populate`, `PotionReward::Populate`, `PlayerOddsSet` (pity
odds), events BattlewornDummy, EndlessConveyor, PotionCourier,
TheLegendsWereTrue, Wellspring; relics DustyTome, NeowsBones, PaelsTooth,
ScrollBoxes; multiplayer treasure sync.

## Per-fight Encounter stream — NOT in either named set (#58, #637)

`EncounterModel.Rng` is a throwaway per-fight stream, built lazily in
`EncounterModel::GenerateMonstersWithSlots` (v0.109.1 RVA 0x22bc0c) as

```
new Rng(RunState.Rng.Seed + (long)RunState.TotalFloor
        + StringHelper.GetDeterministicHashCode(Id.Entry))
```

`Rng::.ctor(ulong)` (RVA 0x61b81) seeds MegaRandom directly, so like the
per-event streams it **starts at counter 0 every fight** and needs no
cross-fight counter accounting. `TotalFloor` (RVA 0x5134e) =
`Sum(MapPointHistory, act => act.Count)` = the 1-indexed global floor, the
same numbering as `floor_added_to_deck` (= `node_index + 1`).

Consumers — always in `GenerateMonsters`, before any Niche HP roll:

| encounter | draws |
|---|---|
| `SlimesWeak` | 3 (`NextItem` small ×2 — the 1-item second still draws — then medium) |
| `SlimesNormal` | 1 `NextBool` (which small goes in slot 2) |
| `BowlbugsWeak` | 1 `NextItem` over `[BowlbugEgg, BowlbugNectar]` |
| `TwoTailedRatsNormal` | 1 `NextInt(3)` starter-move offset |
| `ScrollsOfBiting` (weak + normal) | 1 `NextInt(3)` starter-move offset |
| `DecimillipedeElite` | 1 `NextInt(3)` starter-move offset |

**Build-dependent seeding, and it is not falsifiable from a save.** Save
schema >= 19 persists `rng.rngs[stream] = {counter, s0..s3}` for the 12 run
streams only, so `live_coach.verify_stream_seeding` cannot see this stream —
which is how the six roster builders silently stayed on the v0.108
derivation after the #309 seeding change. Both routes now go through
`Rng.for_encounter`:

```
v0.108: int32 (run_set_seed + TotalFloor + djb2(Id.Entry))
v0.109: uint64(run_set_seed + TotalFloor + XxHash64(Id.Entry))
```

verified against the real engine for 42 (seed, encounter, TotalFloor) cases
(#637; see RNG_FINDINGS.md and `test_encounter_stream_seeding.py`). Because
the rolls choose ROSTERS and starter-move offsets, a wrong derivation
changes which monsters a solve is run against — not just a damage number.

## Per-event streams — NOT in either named set (2026-07-26, #626)

`EventModel.Rng` is **not** one of the 12 run streams or the 3 player
streams: every event gets its own throwaway `Rng`, built in
`EventModel/<BeginEvent>d__28::MoveNext` (RVA 0x366380) as

```
new Rng(RunState.Rng.Seed
        + (IsShared ? 0 : playerSlotIndex)
        + StringHelper.GetDeterministicHashCode(Id.Entry))
```

`Rng::.ctor(ulong)` (RVA 0x61b81) seeds MegaRandom directly, so the stream
**starts at counter 0 every time** and depends only on the run seed, the
player's slot, and the event's entry name. `EventModel::get_IsShared`
(RVA 0x22c1b9) returns false and Neow does not override it.

Consequence: event option generation cannot desync any tracked stream, and
conversely nothing that happens earlier in a run can change what an event
rolls. Modeled for Neow in `versions/v0.111.0/solver/neow.py`.

**Corrects an earlier inference.** `MCR_FORMAT.md` read the frozen .mcr
`{Rewards: 10}` dict as "~10 Rewards of Neow generation". Neow's option
generation consumes **zero** `player:Rewards` draws — it is entirely on the
dedicated `EVENT.NEOW` stream. Whatever produced that frozen 10 is
something else (the Rewards entries below are the real consumers).

## player:Shops (7 sites)

MerchantInventory Populate* + per-entry CalcCost + `CardFactory::CreateForMerchant`.

## player:Transformations (2 sites)

Relics Claws, LeafyPoultice.

## Replay-relevant conclusions

- For fights without card-generating/adding or cycle-membership effects,
  **Shuffle consumption depends only on entry deck size and turn count**
  (plus ethereal exhausts, whose timing we know because we replicate the
  permutation). `ShouldFlush=false` sources — Well-Laid Plans, Ringing
  Triangle on turn 1, Runic Pyramid, and Stable Serum's existing
  RetainHandPower — consume no stream directly but retain cards that would
  otherwise re-enter the draw cycle. Current-fight timing is exact;
  Well-Laid Plans and the two relics therefore join the conservative
  prior-fight Shuffle inventory because `.run` omits those turn-end piles.
  Fan of Knives / Up My Sleeve generated Shivs and Fight Through generated
  Wounds join for the same unrecorded prior-fight cycle-membership reason.
  Implemented in `replay_fight.py` / `solve_fight.shuffle_counter_caveats`.
- Toadpoles and Seapunk (the sample run's first two encounters) have no
  card-adding moves (verified: their move sets are damage/buff only).
- Reward/shop/event generation never touches combat streams, so nothing
  between fights desyncs the Shuffle counter.
- `MonsterAi::RollMove` is the intent stream. Verified model (see
  `monster_ai.py`): rolls happen once per creature when added to combat and
  once per creature per turn (`Creature::PrepareForNextTurn`), but a roll
  only CONSUMES a draw when the monster's move graph traverses a
  `RandomBranchState` (1 `NextFloat` per node). `MoveState` follow-ups and
  `ConditionalBranchState` (first condition > 0 wins) are draw-free.
  **Exactly 21 monsters have random nodes:** DecimillipedeSegment,
  Exoskeleton, Fabricator, FakeMerchantMonster, FlailKnight, Flyconid,
  Fogmog, FossilStalker, HunterKiller, Inklet, LeafSlimeS, Mawler,
  PhrogParasite, ScrollOfBiting, SlitheringStrangler, SludgeSpinner,
  SoulNexus, SpectralKnight, TheObscura, TwigSlimeM, TwoTailedRat.
  Everything else — including Toadpole, Seapunk, and the Act 1 elites
  TerrorEel / PhantasmalGardeners / SkulkingColony — has fully
  deterministic intents given fight state.

## CombatCardGeneration — the card-generation stream (2026-07-10, Sean's Splash/Discovery question)

Card-generation effects (Discovery, Splash, Infernal Blade, attack/skill/
power potions, …) do NOT touch Shuffle/Niche/MonsterAi. They draw from a
dedicated named run stream, **`RunRngSet.CombatCardGeneration`** — counter-
based like the others.

**Draw pattern** (Discovery/Splash, `CardFactory::GetDistinctForCombat`):
filter the pool (`CanBeGeneratedInCombat && rarity ∉ {1,5,6}`, Distinct,
FilterForPlayerCount) → `TakeRandom` = ToList → **UnstableShuffle (one full
Fisher-Yates = poolSize−1 draws)** → take the first 3. So each play
consumes a constant number of draws for a fixed recorded UnlockState (its
filtered pool size − 1), regardless of which option is picked. The chosen
card goes to the HAND (pile 2) free-this-turn — no Shuffle draws. Splash
projects the unlocked character pools in native order and removes its
owner's pool only when more than one character pool is present; upgraded
Splash upgrades all three options before the choice.

**Exact consumer census** (token-level scan, 33 top-level types):
AfflictionModel(PickRandomTargets), AttackPotion, BigHat, BundleOfJoy,
CalamityPower, CallOfTheVoidPower, ChoicesParadox, ColorlessPotion,
CosmicConcoction, CreativeAiPower, Crossbow, Discovery, Distraction,
HelloWorldPower, InfernalBlade, JackOfAllTrades, Jackpot, Largesse,
MadScience, ManifestAuthority, Metamorphosis, OrangeDough, OrobicAcid,
PowerPotion, Quasar, SkillPotion, SpectrumShiftPower, Splash, Stoke,
ThievingHopper (monster move), Toolbox, VexingPuzzlebox, WhiteNoise.
NB: an earlier name-based owner resolution wrongly implicated every card —
`<OnPlay>d__5` is a common compiler-generated name; the token scan above
is authoritative. Gambler's Brew / Fire Potion are NOT consumers.

**Solver implications:**
- Within a fight, generation is deterministic given the entering counter:
  the k-th generation play's 3 options depend only on the SEQUENCE of
  generation plays before it (each advances the counter by its own
  constant), not on timing — no chance nodes, just a 3-way choice.
- Cross-fight: unrecorded generation plays in earlier fights shift the
  counter by known constants — the same hypothesis-enumeration +
  self-consistency-pruning structure as the Shuffle counter, one more
  dimension.

**Implemented bounded slices:** Batch 133 / #614 made Infernal Blade carry
an explicit `CombatCardGeneration` state and consumes the exact 32 draws from
the frozen 33-card, fully-unlocked solo Ironclad Attack pool. Live save/MCR
entry uses the recorded counter and proves the three relevant Ironclad epochs.
The `.run` path infers zero only for fight 0; later fights require an explicit
counter override because the payload omits both this stream and hidden
Affliction consumers. Batch 135 / #618 adds Stoke: one exact `NextItem` draw
with replacement per card in its frozen post-source Hand snapshot, over the
fixed ordered 78-card fully-unlocked solo Ironclad pool.

Batch 159 / #676 adds canonical-live Splash. Live saves and decoded MCR state
pass the complete recorded `UnlockState.unlocked_epochs` set into combat
entry. The engine derives the exact native pool from the checked
`card_pool_census.json`, validates every reachable L0/L1 leaf before
constructing RNG state, and refuses missing or malformed provenance. Three
attested projections are pinned:

- fully unlocked solo Ironclad: 112 cards / 111 draws;
- Ironclad-only one-pool edge: 33 cards / 32 draws;
- Sean's current live profile (Defect epochs 1–2 only): 109 cards / 108
  draws, excluding Barrage, Flak Cannon, and Helix Drill.

The raw `.run` payload has no unlock state, so it continues to refuse Splash;
the old `--fully-unlocked-card-pool` switch is deliberately not treated as a
substitute for recorded epochs. Splash's inaccessible non-null
`MockGeneratedCard` test-hook branch has no installed-assembly caller and is
not exposed as production state. Other generation sources in the census
remain refused under #228.

## CombatCardGeneration — Jack of All Trades (#686 / Batch 163)

Jack of All Trades (`<OnPlay>d__6::MoveNext` 0x3f1124) consumes the stream
through `IEnumerableExtensions.TakeRandom` 0x2bf119 →
`ListExtensions.UnstableShuffle` 0x2bf170 over the materialized
GetDistinctForCombat projection of the fully unlocked solo Colorless pool:
after the runtime-type self-exclusion (b__6_0 0x3f1116) and the
CanBeGeneratedInCombat / rarity filters, the list is exactly 49 entries, so
**every L0 or L1 body consumes exactly 48 draws** — the Cards count changes
how many leading shuffled entries materialize, not the shuffle cost. Live
oracle JACK_CENSUS_V1091: counter 0→48 (L0) and 48→96 (L1). Each generated
entry is created fresh at L0 (Jack+ does not upgrade them) and takes a
separately awaited singular `AddGeneratedCardToCombat(Hand, Bottom)`
(overflow → Discard/Bottom) with no Shuffle-stream draws. Entry requires the
explicit fully-unlocked solo-Ironclad profile, recorded card-pool unlock
epochs (Splash is a reachable generated leaf), and the exact entering
counter.

## CombatCardGeneration — Spectrum Shift / Regent owner pools (#268 / Batch 254)

Spectrum Shift's exact nested BeforeHandDraw body **0x343524** calls
`GetDistinctForCombat` over the owner's unlocked Colorless pool. The frozen
v0.110.1 solo projection has 50 entries in native order, so every trigger
performs one complete 49-draw Fisher-Yates shuffle and takes the first
`min(Amount, 50)` distinct L0 cards. Stacking changes the materialized prefix,
not the draw count. The results enter through one resumable plural
Hand/Bottom command and do not touch Shuffle.

Because those 50 leaves include owner-sensitive generators, the closure also
freezes the Regent `CharacterCardPool` projections used by the generated card
under the same CombatCardGeneration stream: Discovery/Entropy use 79 cards,
Calamity uses 29 Attacks, and Jackpot uses 17 canonical-zero non-X cards.
Splash derives its pool from recorded unlock epochs in native character-pool
order and excludes Regent only when another pool is present. Missing Regent
or Colorless epochs, owner mismatch, or a cross-owner Entropy original refuses
before a stream draw. Infernal Blade and Stoke remain fixed Ironclad-only
consumers and likewise refuse before spend/draw under a Regent owner.

## CombatPotionGeneration — Alchemize (#679 / Batch 160)

Alchemize uses the named run stream `CombatPotionGeneration`, independently
of CombatCardGeneration, Shuffle, and the potion body's own later streams.
Its canonical body calls `CreateRandomPotionInCombat`, which consumes exactly
two draws in this order:

1. `NextFloat(1)` selects Rare at `<= float32(0.1)`, Uncommon at
   `<= float32(0.35)`, otherwise Common.
2. `NextItem` consumes one draw over the chosen stable rarity partition,
   including a singleton partition.

The checked v0.111.0 `potion_pool_census.json` pins the fully unlocked solo
Ironclad reachable pool at 45 identities: Common 16, Uncommon 15, Rare 14.
The stream advances by two even when `TryToProcure(-1)` subsequently fails
because Sozu vetoes or the fixed-size belt is full. An already-ending combat
suppresses the body before either draw.

Live schema-19 saves carry the cumulative
`rng.rngs.combat_potion_generation.counter`; earlier saves use
`rng.counters`. Both live coaching and decoded MCR replay pass that recorded
counter and the replay's recorded build into `start_combat`. Historical
`.run` records carry neither the counter nor exact slot topology/unlock
evidence and therefore refuse Alchemize.

Procurement fills the first null belt slot. The ordered fixed-length slot
tuple is authoritative keyed state; modeled and inert dense tuples are
validated compatibility views. Generated unmodeled potions remain inert
under #634 (or refuse atomically under strict mode), while generated modeled
potions immediately enter the exact slot-distinct action space. Sozu is the
sole native pre-insert veto; fail-closed model guards are described below.
Belt Buckle's post-success hook removes its
empty-belt Dexterity and clears the latch, allowing exact reapplication when
the last potion is later used.

R52A / #2056 carries that provenance through the Rust boundary without
executing a potion body. The fixed-length nullable `potion_slots` tuple is
authoritative; `potions` must be its exact ordered non-null mirror, while
unknown `inert_potions` refuse. A nonempty topology requires the immutable
fully-unlocked/current-build proof and retains Sozu, Belt Buckle, its applied
latch, and strict-mode facts. Flags without a topology refuse rather than
inventing a capacity. Occupied public belts remain outside Part A admission,
so no `CombatPotionGeneration` draw or body mutation is newly reachable.

Thieving Hopper makes persistent DeckVersion UIDs outcome-relevant. While
that master-deck projection is active, the still-payload-only selectors of
Gambler's Brew, Ashwater, and Touch of Insanity refuse at entry, procurement,
and forged runtime state. Generated procurement runs on a clone first, so a
generated one of those three refuses before the real CombatPotionGeneration
counter, belt slot, or Belt Buckle latch changes. This is distinct from the
native Sozu/full-belt failed-procurement path, which still consumes its draws.

## CombatPotionGeneration — Delicate Frond (#774 / Batch 187)

Delicate Frond uses the same rarity-roll/item-roll primitive as Alchemize,
but calls the out-of-combat factory at ordinary combat entry. Its attested
fully unlocked solo Ironclad pool contains 48 identities, partitioned
16 Common / 16 Uncommon / 16 Rare. It does not apply
`CanBeGeneratedInCombat`, so Fairy in a Bottle, Fruit Juice, and Regen
Potion are present in addition to Alchemize's 45 leaves.

The relic always generates before attempting procurement. A non-Sozu belt
with `n > 0` null slots consumes exactly `2n` draws and fills each first-null
slot; a full belt or Sozu consumes exactly two draws before the first attempt
fails. Each loop iteration is a fresh one-potion factory call, so duplicate
identities are legal. Successful insertion runs the shared
AfterPotionProcured response, including Belt Buckle's Dexterity removal.
Petrified Toad's later fixed Potion Shaped Rock attempt consumes no stream.

Admission requires the exact entering `CombatPotionGeneration` counter,
v0.111.0 build provenance, fully unlocked solo Ironclad potion-pool
provenance, exact positive belt capacity, and sparse slot identities.
Historical `.run` records still lack those inputs and remain refused.
Because the out-of-combat pool can generate combat-card-generation and
target-selection potions (and Snecko Oil), the conservative prior-fight
accounting also requires the existing CombatCardGeneration, CombatTargets,
and CombatEnergyCosts counters. The stream constructor retains the full
v0.109 64-bit seed when materializing CombatPotionGeneration and the
reachable CombatCardGeneration, CombatCardSelection, CombatTargets,
CombatEnergyCosts, and MonsterAi keyed states.

## Shuffle — Bottled Potential (#231 final capture reconciliation)

Bottled Potential owns one explicit `CardPileCmd.Shuffle` after moving its
complete frozen Hand to Draw/Bottom. Shuffle snapshots live Discard followed
by live Draw, performs the ordinary StableShuffle (exact .NET List.Sort then
backwards Fisher-Yates), empties Discard, raises AfterShuffle, and only then
draws five. A combined pile of size `n` therefore consumes `max(0, n - 1)`
draws on the named run Shuffle stream regardless of the five-card prefix.

The v0.110.1 live MCR pin starts at counter 332 with 27 combined cards and
ends the shuffle at 358. Its full post-use Hand and Draw order matches the
solver. Distinguishable compare-equal payloads remain refused; ordinary
equal-payload copies preserve the captured quotient. Since `.run` records a
potion use but not its exact time or live pile membership, any prior fight
that used `POTION.BOTTLED_POTENTIAL` makes later predicted Shuffle counters
hypotheses and is surfaced by `shuffle_counter_caveats`.
