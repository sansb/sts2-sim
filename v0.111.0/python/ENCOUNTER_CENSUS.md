# Elite & boss encounter census — solver derisk sizing (#58)

Generated from sts2.dll build v0.108.0 by `sim/v0.111.0/python/tools/census_encounters.py`
(2026-07-09). Scope: every `*Elite` / `*Boss` encounter class (mocks
excluded). Monster composition = distinct monster classes referenced by the
encounter (spawn *counts* are call-sites, not exact — read per encounter when
modeling it). HP shown at A8+ / below-A8.

## Corrections from the Act-1 modeling pass (2026-07-10)

Per-encounter IL reads for the six remaining Act-1 elites found four census
errors — noted here so downstream sizing doesn't inherit them:

1. **DECIMILLIPEDE_ELITE is 3 segments, not 6** (Front/Middle/Back; the
   "6-segment body" came from call-site counting).
2. **PHROG_PARASITE has NO reachable random AI** — its RandomBranchState
   node exists but nothing transitions into it (follow-ups alternate
   INFECT ⇄ LASH directly). Random-AI monsters: 3, not 4 (FlailKnight,
   SpectralKnight, SoulNexus). Its "summon" is a death trigger
   (InfestedPower: 4 stunned Wrigglers when the phrog dies), not a move.
3. **ENTOMANCER adds cards to the player's piles mid-fight** —
   PersonalHivePower inserts Dazed into the DRAW pile at random positions
   (one Shuffle draw per card). The census scanned encounter/monster
   classes only, so power-driven pile interaction was missed; the "2
   encounters touch player piles" claim undercounts. PHROG_PARASITE also
   adds cards (Infection → discard Bottom, no Shuffle draw). **Scan powers
   too when re-running the census.**
4. **DecimillipedeSegment revives** (ReattachPower 25) — killing the fight
   requires all three segments dead simultaneously, and each revive
   consumes one MonsterAi draw. Segment max HP is post-processed to even
   values (odd rolls round up, collisions +2 with wraparound).

Bonus: the per-fight **Encounter stream** was decoded —
`Rng(runSeed + totalFloor + hash(entry))`, no cross-fight accounting —
which also makes the TwoTailedRat starter offset derivable (was fitted).

5. **Act assignment measured** (the census couldn't capture it; run-history
   floor distributions are unambiguous): **Act 1 elites** = TerrorEel,
   SkulkingColony, Gardeners, BygoneEffigy, Byrdonis, PhrogParasite
   (floors 6-15); **Act 2 elites** = Decimillipede, Entomancer,
   InfestedPrisms (floors 24-33); **Act 3 elites** = Knights, MechaKnight,
   SoulNexus (floors 39-48). DERISK_MEASUREMENTS' "nine Act-1 elites" was
   a mislabel — the nine span Acts 1+2 (its fight-share numbers are
   unaffected: they summed the same nine encounters).

## Headline numbers

- **24 encounters** (10 bosses, 14 elites... see table), **32 distinct monsters**.
- **28 of 32 monsters are intent-deterministic.** Random AI (RandomBranchState,
  1 node each — same shape as the validated TwoTailedRat model): FlailKnight +
  SpectralKnight (KNIGHTS_ELITE), PhrogParasite, SoulNexus. **Every boss is
  deterministic.**
- **HP ranges (Niche-stream rolls) exist on 6 of 32 monsters** — correction
  2026-07-10: the first revision of this doc claimed only Decimillipede,
  a summary-rendering bug (it ignored `hp_max_consts`). At A8+ the ranged
  monsters are: DecimillipedeSegment 46–52, PhantasmalGardener 27–32,
  PhrogParasite 66–68, Wriggler 18–22, KinFollower 62–63 (Byrdonis is
  ranged only below A8: 81–84). Niche accounting turned out trivial anyway:
  `Creature::SetUniqueMonsterHpValue` consumes **exactly one Niche draw per
  monster created** (even fixed-HP ones) and picks a max-HP value distinct
  from other monsters' within [Min, Max] — so the counter entering fight N
  is just the number of creatures created in prior combats (+ summons +
  on-obtain relic effects per STREAM_CONSUMERS).
- **2 encounters touch the player's card piles** (Shuffle-stream relevant,
  and a downstream counter-ambiguity source for fights after them):
  SOUL_FYSH_BOSS and THE_INSATIABLE_BOSS (`CardPileCmd::AddGeneratedCardToCombat`).
- **1 encounter summons**: PHROG_PARASITE_ELITE (Wriggler spawn; Wriggler can
  start stunned — see `Wriggler::get_StartStunned`).
- **~25 signature powers to model** across all encounters (each the size of a
  ShriekPower/HardenedShell read). This is the bounded, mechanical part.

## Encounter table

| encounter | monsters (HP A8+/below) | random AI | special |
|---|---|---|---|
| TERROR_EEL_ELITE | TerrorEel 150/140 | — | ✅ **done** (#55): Shriek, Vigor |
| SKULKING_COLONY_ELITE | SkulkingColony 80/75 | — | ✅ **done** (#55): HardenedShell |
| PHANTASMAL_GARDENERS_ELITE | PhantasmalGardener ×4 27–32/26–31 | — | ✅ **done**: Skittish, unique-HP rolls, multi-monster |
| BYGONE_EFFIGY_ELITE | BygoneEffigy 132/127 | — | Slow |
| BYRDONIS_ELITE | Byrdonis 90/81 | — | Territorial |
| DECIMILLIPEDE_ELITE | Segment Front/Middle/Back **46–52/40–46** | — | ⚠️ only HP range in the game → Niche stream; 6-segment body |
| ENTOMANCER_ELITE | Entomancer 155/145 | — | PersonalHive |
| INFESTED_PRISMS_ELITE | InfestedPrism 171/161 | — | VitalSpark |
| KNIGHTS_ELITE | FlailKnight 108/101, SpectralKnight 97/93, MagiKnight 89/82 | ⚠️ Flail + Spectral | Hex; 3 monsters |
| MECHA_KNIGHT_ELITE | MechaKnight 320/300 | — | Artifact |
| PHROG_PARASITE_ELITE | PhrogParasite 66/61, Wriggler 18/17 | ⚠️ PhrogParasite | ⚠️ summons Wrigglers; Infested |
| SOUL_NEXUS_ELITE | SoulNexus 254/234 | ⚠️ | — |
| AEONGLASS_BOSS | Aeonglass 535/512 | — | Artifact |
| CEREMONIAL_BEAST_BOSS | CeremonialBeast 262/252 | — | Plow/Ringing; stun-by-plow-removal mechanic |
| KAISER_CRAB_BOSS | Crusher 219/209 + Rocket 209/199 | — | BackAttack left/right, CrabRage, Surrounded — positional |
| KNOWLEDGE_DEMON_BOSS | KnowledgeDemon 399/379 | — | — |
| LAGAVULIN_MATRIARCH_BOSS | LagavulinMatriarch 233/222 | — | Asleep/Plating (STS1-style wake trigger) |
| QUEEN_BOSS | Queen 419/400 + TorchHeadAmalgam 211/199 | — | ChainsOfBinding, Minion |
| SOUL_FYSH_BOSS | SoulFysh 221/211 | — | ✅ **done** (#120 E1): adds cards to player piles (BECKON = 1 Shuffle draw for the draw-pile insert + 1 discard append; GAZE = 1 append); Intangible; boss log replay certified against the exit save |
| TEST_SUBJECT_BOSS | TestSubject 111/100 → 212/200 → 313/300 | — | ✅ **done** (#216): Adaptable retained-death/RESPAWN graph; Skill-gated Enrage; scaling Multi Claw + Painful Stabs Wounds; alternating Nemesis; Burns/Strength |
| THE_INSATIABLE_BOSS | TheInsatiable 341/321 | — | ✅ **done** (#217): Sandpit 4; six null-creator Frantic Escape random inserts (3 Draw + 3 Discard); exact force-kill countdown and card-local cost growth |
| THE_KIN_BOSS | KinPriest 199/190 + KinFollower ×? 62/58 | — | Minion swarm |
| VANTOM_BOSS | Vantom 183/173 | — | Slippery |
| WATERFALL_GIANT_BOSS | WaterfallGiant 250/240 | — | SteamEruption; 8 move states (largest deterministic machine) |

## Risk tiers for the derisk plan

1. **Trivial extensions of what's built** (deterministic, fixed HP, no pile
   interaction): Gardeners, BygoneEffigy, Byrdonis, Entomancer,
   InfestedPrisms, MechaKnight, Aeonglass, KnowledgeDemon, WaterfallGiant,
   Vantom, KinBoss, Queen, LagavulinMatriarch, CeremonialBeast, KaiserCrab —
   each is "read the move machine + its signature power, add constants".
   Multi-monster fights (Gardeners, Knights, KaiserCrab, Kin) need the sim's
   single-monster assumption generalized — one-time engineering.
2. **Random-AI intents** (validated pattern, needs per-monster weights):
   Knights, PhrogParasite, SoulNexus.
3. **New machinery**: Decimillipede (Niche HP rolls — needs
   `CombatState::CreateCreature` roll order + Niche counter accounting),
   PhrogParasite (summons), TestSubject (multi-form).
4. **Replay-model interaction**: SoulFysh & TheInsatiable add cards to the
   player's piles mid-fight — consumes Shuffle stream (random-position
   inserts) AND permanently forks the counter for *later* fights (they're
   bosses, so "later" = next act). Must be modeled before any fight
   downstream of them is reviewable.

## Census tool caveats

- `powers_applied` mixes self-buffs and player-debuffs; per-encounter reads
  disambiguate.
- Spawn counts are call-sites (e.g. Gardeners shows 5 `Monster<...>` sites
  for a 4-monster fight — one is likely conditional/scaling).
- Move damage constants are NOT extracted here — that's the per-encounter
  modeling pass (as done for the eel/colony in ENCOUNTER_MECHANICS.md).
- Act assignment isn't captured (lives in encounter pools); worth adding when
  prioritizing by act.
