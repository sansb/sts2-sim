# STS2 RNG — reverse-engineering findings (build v0.108.0)

**Date:** 2026-07-08.
**Source:** IL disassembly of the shipped `sts2.dll`, build v0.108.0
(commit `58694f64`, built 2026-07-02) — the macOS Steam install. Disassembly
scripts in `versions/v0.111.0/solver/tools/`.

## Headline answers (handoff task 1)

1. **The current build does NOT use C# `System.Random`.** The June 2026 RNG
   overhaul replaced it with a custom class `MegaCrit.Sts2.Core.Random.MegaRandom`
   — a textbook **xoshiro256\*\*** generator seeded by four successive outputs of
   the canonical **splitmix64** routine (constants match the Blackman/Vigna
   reference exactly).
2. **The sample run (`fight_states.json`, seed `ZPJHU3WSH2`, v0.108.0) is
   post-overhaul.** The installed build and the sample .run file are the *same
   build*, so the port in `sts2_rng.py` applies to it directly. Pre-overhaul
   reverse engineering (sts2-rng-fix repo, tck.mn blog) describes the OLD
   System.Random-based system — useful for architecture, wrong for bit-level
   replay of new runs. Old .run files (`build_id` < the ~June 19 patch) would
   need the old RNG; not worth supporting for launch.

## Architecture (unchanged by the overhaul)

The stream layout survived the overhaul — only the underlying generator and
seed-derivation hash changed from what the community documented.

```
Rng                      wrapper: int32 seed + call counter
 └─ MegaRandom           xoshiro256**, seeded via splitmix64((ulong)seed)

RunRngSet                one per run  — seed = hash(seedString)
PlayerRngSet             one per player — seed = hash(seedString) + slotIndex
 └─ per stream:          Rng(setSeed + hash(snake_case(streamEnumName)))
```

- `hash` = `StringHelper.GetDeterministicHashCode`: the well-known .NET
  "deterministic string hash" (two-lane shift-add-xor, int32 wraparound,
  `h1 + h2 * 1566083941`, init `0x15051505`).
- `snake_case` via regex `([A-Za-z0-9]|\G(?!^))([A-Z])` → `$1_$2`, lowercased.
- Seed → ulong conversion uses CIL `conv.u8` = **zero-extension** of the int32
  bit pattern (negative seeds map to `0x00000000_FFFFFFFF`-style values).

### Run streams (`RunRngType`, index order)

| # | enum | string hashed |
|---|------|----------------|
| 0 | UpFront | `up_front` |
| 1 | Shuffle | `shuffle` |
| 2 | UnknownMapPoint | `unknown_map_point` |
| 3 | CombatCardGeneration | `combat_card_generation` |
| 4 | CombatPotionGeneration | `combat_potion_generation` |
| 5 | CombatCardSelection | `combat_card_selection` |
| 6 | CombatEnergyCosts | `combat_energy_costs` |
| 7 | CombatTargets | `combat_targets` |
| 8 | MonsterAi | `monster_ai` |
| 9 | Niche | `niche` |
| 10 | CombatOrbs | `combat_orbs` |
| 11 | TreasureRoomRelics | `treasure_room_relics` |

### Player streams (`PlayerRngType`)

Rewards (`rewards`), Shops (`shops`), Transformations (`transformations`).

There is also a 4-arg `Rng(IRunContext, something-with-.Entry, offset, counter)`
ctor: seed = `RunState.Rng.Seed + playerSlotIndex + hash(arg.Entry) + offset` —
apparently for ad-hoc per-entity RNGs (not yet traced; see follow-ups).

## Draw semantics (what the solver must reproduce)

Every public `Rng` method consumes **exactly one** 64-bit xoshiro output and
increments the counter by one. Save/load stores per-stream counters and
fast-forwards a fresh Rng by replaying `NextInt()` counter times — so
**"RNG state entering fight N" ≡ per-stream call counts**, as hoped.

- `NextDouble()` = `(next_u64 >> 11) * 2⁻⁵³`
- `NextInt(max)` = `(int)(NextDouble() * max)` — *floor-of-double*, not
  rejection sampling; max exclusive
- `NextInt(min, max)` = `min + (int)(NextDouble() * (max-min))`; throws if
  min ≥ max
- `NextBool()` = `NextInt-style draw, Next(2) == 0`
- `NextFloat/NextUnsignedInt` — same one-draw pattern, see `sts2_rng.py`
- `NextGaussian*` — Box-Muller; consumes **two** draws per attempt, retries
  until in `[0,1]` (mean-relative), so counter delta is variable
- `Shuffle(list)` — Fisher-Yates from the top: `for i = n-1..1: swap(i,
  NextInt(i+1))` → n−1 draws for n items
- `NextItem(list)` — one draw, **except empty list returns default with zero
  draws** (counter-relevant edge case)
- `Rng.Chaotic` — static Rng seeded from wall-clock Unix time: used for
  anything intentionally non-deterministic; never replayable, and that's fine

## What's in this directory

- `sts2_rng.py` — pure-Python port (MegaRandom, Rng, RunRngSet, PlayerRngSet).
  `python3 sts2_rng.py [seed]` prints derived stream seeds.
- `test_sts2_rng.py` — 12 tests; algorithm layer validated against an
  independent C implementation of the canonical reference generators
  (`test_vectors.json`).
- `test_vectors.json` — generated vectors (states, raw outputs, doubles,
  bounded ints, string hashes).

## Confidence & open verification items

Validated: the Python port bit-matches the canonical xoshiro256**/splitmix64
reference and the documented .NET hash, and matches my reading of the IL.

**Not yet validated against the live game.** Remaining risks: misread IL
(e.g. an overload subtlety), wrong stream consumed by a given game system, or
draw-order assumptions. Ground truth requires handoff tasks 3/5 (in-game
determinism spot-check + opening-hand replication for one encounter), or a
tiny Harmony mod logging `(stream, counter, value)` triples.

## Build v0.109.0 (2026-07-16): per-stream seed derivation changed (#309)

**Source:** IL disassembly of `sts2.dll` build v0.109.0 (commit `c12f634d`,
archived at `solver/dll-archive/v0.109.0/`), re-read for this section — plus
a headless-engine verification run (below). Modeled in `sts2_rng.py` behind
`build=` (default remains the v0.108 scheme so all pre-#309 pins hold).

What changed vs v0.108 (algorithm did NOT change — MegaRandom's
xoshiro256**/splitmix64 and every draw conversion are byte-identical in IL;
v0.109 adds `_incrDouble`/`_incrFloat` fields that no Rng draw path uses):

- `StringHelper.GetDeterministicHashCode` (RVA 0x2b6ba8) is now
  **`System.IO.Hashing.XxHash64.HashToUInt64(UTF8(s), seed: 0)`** returning
  `uint64`. The old two-lane djb2 int32 hash survives as
  `GetDeterministicHashCodeOld` (RVA 0x2b6c38), byte-identical to v0.108.
- `RunRngSet::.ctor` (RVA 0x50f70): `Seed = GetDeterministicHashCode(seed
  string)` — EXCEPT seed strings starting with `"old"`, which use
  `(uint64)GetDeterministicHashCodeOld(rest)` (CIL `conv.u8` zero-extension)
  as a legacy escape hatch.
- `RunRngSet::CreateRng` (RVA 0x5100c) -> `Rng::.ctor(UInt64, string)`
  (RVA 0x61be9): `stream_seed = Seed + GetDeterministicHashCode(
  SnakeCase(enum.ToString()))`, **64-bit wrapping add, no int32
  truncation**; the full uint64 goes into splitmix64.
- `Player::InitializeSeed` (RVA 0x2c2bda): `player_set_seed =
  GetDeterministicHashCode(seed string) + (long)slot_index` — 64-bit, and
  NO `"old"` escape on the player path.
- Stream enum NAMES are unchanged (`RunRngType` still declares
  `CombatOrbs`; only the C# getter was renamed `CombatOrbGeneration`).
  `SnakeCase` (RVA 0x2b6ac3) is unchanged.
- New in v0.109, not needed for the sets but worth knowing:
  `Rng::.ctor(Player, ModelId, offset)` (RVA 0x61bb5) derives a per-model
  stream as `RunRngSet.Seed + slot + hash(ModelId.Entry) + offset`.
- Save schema v18->v19: v19 serializes full xoshiro state
  (`SerializableRng {counter, state0..3}`, `Rng::LoadFromSerializable`
  RVA 0x61bf9); the v18->v19 migration zeroes counters.

**Engine verification** (build v0.109.0 commit c12f634d, fresh boot,
`versions/v0.111.0/solver/harness/probe_rng_seeding.py`): the Python model bit-matches the
real engine's `GetDeterministicHashCode`/`GetDeterministicHashCodeOld` on
all 15 stream names + 4 seed strings + edge strings, and the first three
`NextInt(100)` draws of every run stream (12) and player stream (3, slots
0 and 1) for seeds `TESTBATCH0`, `ZPJHU3WSH2`, `8DVXPWUWRY`, `HQPAXCBS6P`,
and `oldZPJHU3WSH2`. Pinned in `test_sts2_rng.py` (v0.109 section).

**Version rule:** `sts2_rng` refuses builds > v0.109
(`NotImplementedError`) until their seeding is re-verified — rerun the
probe against the new DLL and extend `seeding_scheme` on each game update.

### The per-fight Encounter stream also moved (#637)

The 12 named run streams are not the whole story. `EncounterModel::
GenerateMonstersWithSlots` (v0.109.1 RVA 0x22bc0c) lazily builds an
**ad-hoc** `Rng` per fight, outside every set, and it changed with the same
patch:

```
v0.108: enc_seed = int32 (run_set_seed + TotalFloor + djb2(Id.Entry))
v0.109: enc_seed = uint64(run_set_seed + TotalFloor + XxHash64(Id.Entry))
```

IL, `GenerateMonstersWithSlots` IL_002d-0054: `RunRngSet::get_Seed`
(MethodDef rid 3288) returns **u8**; `IRunState::get_TotalFloor` (rid 2899)
returns i4 and is widened by `conv.i8`; both adds are the unchecked 64-bit
`add`; `StringHelper::GetDeterministicHashCode` (rid 32749) returns **u8**
(the djb2 `GetDeterministicHashCodeOld`, rid 32750, is NOT called here);
and `newobj Rng::.ctor` resolves to rid 4166 = `Rng(UInt64)` (RVA 0x61b81),
which passes the sum to `MegaRandom(UInt64)` untruncated. So all three
addends and the seed itself are 64-bit — the v0.108 derivation was wrong on
the run-seed hash, the entry hash, and the wrap width simultaneously.

**Why it needed its own verification pass.** This stream is not persisted:
save schema >= 19 records `rng.rngs[stream] = {counter, s0..s3}` for the run
streams only, which is what lets `live_coach.verify_stream_seeding` falsify
their derivation (#636). The per-fight stream has no recorded state, so it
was invisible to that check and silently stayed on v0.108 semantics in the
six roster builders under `versions/v0.111.0/solver/content/encounters/`.

**Engine verification** (build v0.109.1 commit c8c577f6, headless harness):
`EncounterModel.MutableClone()` -> `GenerateMonstersWithSlots(run)` on a
`RunState.CreateForTest` run, then `_rng._counter` and `_rng._random`'s
xoshiro quadruple read by reflection, for 6 encounters x 7
(seed, TotalFloor) combinations = **42 cases**. Solving those states back
to a seed picks out the formula above uniquely: the pre-#637 hybrid
(v0.109 set seed + djb2 + int32 truncation), the same hybrid at uint64,
the pure v0.108 derivation, and an int32-truncated XxHash64 variant are all
rejected in all 42 cases. `RunState::get_TotalFloor` (RVA 0x5134e) =
`Sum(MapPointHistory, act => act.Count)` over a
`List<List<MapPointHistoryEntry>>`, so the probe reached TotalFloor N by
appending one inner list of N entries — floors 0, 1, 3, 8 and 30 are
covered, i.e. the floor addend is measured rather than inferred.

Modeled as `Rng.for_encounter(set_seed, total_floor, entry, build=)` and
reached only through `_EncounterCtx.encounter_rng()`; pinned in
`test_encounter_stream_seeding.py`. The v0.108 route is unchanged and
remains the default, so the in-game v0.108 roster pins (e.g. 7MA0PY7AD4
fight 2 at TotalFloor 5) still reproduce.

## Build v0.109.1 (2026-07-26): point-release validation (#625)

Build v0.109.1 (commit `c8c577f6`) was archived before validation; its
`sts2.dll` sha256 is
`2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f`
(v0.109.0:
`06c78d946ca70658e85abb28f6dc2ee0a023a4467faf0708ff542180fe5f4c82`).
The two 204-file release trees have the same paths and bytes everywhere
except `sts2.dll`. A full managed comparison found no type, field, method
signature, metadata-table-count, or parsed CIL-body delta across 9,627 types,
41,144 fields, 50,854 methods, and 43,794 method bodies. The assembly MVID
changed, but managed behavior did not.

The RNG-specific `Rng`, `RunRngSet`, `PlayerRngSet`, and `StringHelper` dumps
are byte-identical. `probe_rng_seeding.py` against the live v0.109.1 engine
matched all 70 existing v0.109.0 engine-pinned hash/run/player comparisons,
including the `old`-prefix seed path. Therefore v0.109.1 remains on the
existing `v109` seeding route with no algorithm or vector change.

As an independent content/schema guard, the cards, relics, potions,
encounters, enchantments, and raw card-template regenerations were
byte-identical between the archived builds. A relic-template regeneration
exposed five stale committed refusal rows, but both archived builds generated
the same new output; that pre-existing artifact drift is not a v0.109.1
change and is intentionally excluded from #625.

## Build v0.110.1 (2026-07-31): RNG exact, solver models parked (#793)

Build v0.110.1 (commit `db5d3552`) was archived before validation; its
`sts2.dll` sha256 is
`5a8fb7eb62510a86fd03653b9210cd8f67b511b632331a1f174042de39c92bd9`.
This is not a metadata-only point release: the complete managed comparison
found 1,013 signature/token-normalized changed bodies plus substantial added
and removed types/methods. The changed combat/content surfaces are tracked by
#794 and children #795-#798.

RNG was independently re-established rather than inferred from that broader
diff. A fresh `probe_rng_seeding.py` run against the installed v0.110.1 engine
matched the existing v109 algorithm in all 267 scalar observations: string
hashes, run-set seeds, all 12 named run streams, player-set seeds, and all
three player streams, including the legacy `old`-prefix path. A fresh
`probe_encounter_stream.py` run matched all 42 archived
`(seed, encounter, TotalFloor)` state/counter/roster observations. The
complete named-stream call-site scan also retained the same 162 unique raw
scanner rows. Those row identities are intentionally qualified: the scanner
prints only the immediate declaring type, method, and target getter, so common
nested state-machine names are lossy without their enclosing owner. Manual
owner recovery for all four textual deltas found compiler state-machine
renumberings only: Shuffle, AutoPlayFromDrawPile, one AfterCardPlayed caller,
and Abundance OnPlay `d__5 -> d__6` (CombatCardGeneration read remains at
v0.110.1 IL `0092`).

Therefore the exact build string `v0.110.1` maps to the existing `v109` RNG
scheme. Other v0.110 patch strings still refuse until separately verified.
This admission was deliberately narrower than combat admission at the gate:
`start_combat` rejected v0.110.1 until the stale-model umbrella #794 closed.
The completed remodel later admitted combat. Exact RNG was necessary but not
sufficient evidence for that exact solve.

## Build v0.111.0 (2026-08-13): seeding exact, one new consumer (#1214)

Build v0.111.0 (commit `41cef1ea`) was archived before impact work. Its
`sts2.dll` sha256 is
`9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
The complete normalized managed comparison found substantial combat/content
change, so RNG was reproved rather than inherited from v0.110.1.

Fresh engine probes matched the v109 derivation in all **267** scalar
hash/run/player observations and all **42** independent Encounter-stream
state/counter/roster observations. The 21 pinned encounter projections also
matched the model exactly. Accordingly only exact build string `v0.111.0`
joins the `v109` seeding route; neighboring future builds still refuse.

The complete named-stream getter scan changed from 162 to 163 unique rows.
Owner recovery found one real new consumer rather than a state-machine rename:
`BeautifulBracelet.AfterObtained` now reads `RunState.Rng.Niche` to shuffle
the complete eligible acquisition-time deck before taking four and applying
Swift. Batch 296 / #1218 pins the exact `max(0, n - 1)` consumption and keeps
ordinary `.run` prediction explicitly caveated because that acquisition-time
pool is not recorded. Batches 294-298 completed the card, encounter,
RNG/relic, Inky, and shared-combat remodels. v0.111.0 combat is now admitted;
the shared-combat delta adds no RNG consumer.
