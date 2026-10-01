# STS2 .mcr combat replay format (v0.108.0, updated for v0.109.1/v0.110.1)

> ## ⚠️ Shelved — read this before building on it
>
> This work is **archived for reference and is not a productive route for
> the product** (decision: July 2026, see issue #66; #61 and #28 were
> closed wontfix). The game keeps **only `latest.mcr`, overwritten every
> combat** — there is no on-disk history to upload post-hoc. Capturing
> fights therefore requires software running on the player's machine while
> they play (a watcher app), which is the same install friction as a
> Workshop mod with none of a mod's discoverability — and it clashes with
> the project's cold-start, post-hoc `.run`-upload distribution model
> (no brand recognition yet, many players on Steam Deck where sideloaded
> watchers are impractical). The decoder also needs its net-id tables
> regenerated every game patch.
>
> Everything below remains true and validated; treat it as a research
> artifact. If local capture ever becomes viable (i.e. we ship a Workshop
> mod), start from issue #66.
>
> **Dev-side exception (issue #118, 2026-07-14):** the format IS in
> active use as a validation harness over Sean's own local gameplay —
> `tools/mcr_watch.py` archives the files, `tools/mcr_validate.py`
> checks counter accounting and shuffle replay against them. That work
> also corrected two claims below, one of which was itself withdrawn in
> #639; see "Counter caveat".

Reverse-engineered July 9 2026 from the IL of
`MegaCrit.Sts2.Core.Multiplayer.Replay.*` and
`MegaCrit.Sts2.Core.Multiplayer.Serialization.*` in sts2.dll (game
v0.108.0, commit 58694f64). Decoder: `mcr_parser.py`; net-id tables:
`tools/build_mcr_tables.py` → `mcr_tables.json`. Re-derived for game
v0.109.1 (commit c8c577f6) on 2026-07-26 (#639); the deltas are called out
per section and in "v0.108 → v0.109 wire changes". Everything below is
validated against real files: `testdata/latest.mcr` (v0.108.0) and
`testdata/7XDBEBWZ1REL_byrdonis_v109.mcr` (v0.109.1) both decode with zero
unconsumed bits and a matching `modelIdHash`, as do all 25 captures from
the 2026-07-26 live session.

**v0.110.1 (commit db5d3552, checked 2026-08-01, #807):** the wire schema
is UNCHANGED from v0.109.1 — regenerating the net-id tables against the
archived v0.110.1 DLL reproduces the header hash every 2026-07-31 capture
carries (`0xea293d3f`), and all 32 captures from that session decode with
zero unconsumed bits under the v0.109 schema branch. The #639 hash-recipe
reconstruction holds; only the tables were stale. Table files are now
per-build: `mcr_tables.json` tracks the CURRENT install, and frozen copies
(`mcr_tables_v0.108.0.json`, `mcr_tables_v0.109.1.json` — the latter also
serves CIL-identical v0.109.0) decode older captures (see
`TABLES_BY_VERSION` in `mcr_parser.py`). Pinned fixtures:
`testdata/TQM88QFMHSQR_{sludge,colony,gardeners}_win.mcr` with paired
entry saves (`test_mcr_v110_captures.py`), covering the Colorless-Potion
1-of-3 Index `PlayerChoice` and an enemy-targeted potion
(`NetUsePotionAction.target_id` uses the same creation-ordinal scheme as
card plays; a self/ally use records `target_player_id` instead).

**v0.111.0 (commit 41cef1ea, checked 2026-09-04, #1283 groundwork):** the
wire schema is again UNCHANGED from v0.109.1 — regenerating the net-id
tables against the archived v0.111.0 DLL reproduces the header hash every
capture since 2026-08-14 carries (`0x5d828510`), and all 540 v0.111.0
captures on the dev machine decode with zero unconsumed bits under the
v0.109 schema branch. Only the tables were stale (the same failure as #807),
and for three weeks every current-build replay refused with a modelIdHash
mismatch while #1272 recorded the v0.111 capture evidence as absent.
`mcr_tables.json` now serves v0.111.0 and the previous file is frozen as
`mcr_tables_v0.110.1.json`. Pinned fixture:
`testdata/6P96T755CNZ3_mawler_{entry.save,win.mcr}` — the slow-pin fight of
`test_v0111_ingame_validation.py`, now with the human's exact 17-action line
and 13 native per-action checksums agreeing with the simulator
(`test_mcr_v111_captures.py`).

## What it is, operationally

- The game writes **one file, `profile1/replays/latest.mcr`, overwritten
  every combat** (no other `.mcr` path exists in the assembly; the string
  only appears elsewhere as a file-picker filter). A watcher app must
  harvest on file change to capture every fight.
- The write happens at combat **END** (`RunManager::EndCombatInternal`
  → `WriteReplay`, also on `StateDiverged` and `CleanUp`), with the
  snapshot recorded at combat **start** (`EnterMapPointInternal` →
  `CombatReplayWriter::RecordInitialState(RunManager::ToSave(null))`,
  RVA 0x21502c) — so one file holds the entering state plus the whole
  fight's inputs.
- It is a **CombatReplay**: a full run-state snapshot at combat start
  (deck, relics, HP, gold, map, history, and — see "Counter caveat" — the
  full rng set) plus the per-combat **input log** (cards played with
  targets, potions, end-turns) recorded for the multiplayer
  desync/replay system.
- The game can load these files in-engine ("Loaded replay. Game version:
  …" strings in the loader), so any writer we ever build has a free
  validation oracle.
- The events list contains **player inputs only** (`GameAction` entries),
  not engine outcomes — the engine re-simulates deterministically from the
  snapshot + inputs. Draw order etc. must come from RNG replay (see
  `sts2_rng.py`).
- `NetPlayCardAction.combatCardIndex` is a **stable per-instance id
  assigned in first-cycle draw order**: instance k is the k-th card
  dealt from the combat-start shuffle, and it keeps that id across
  turns and reshuffles (verified on a 36-event, 7-turn boss log —
  replayed cards reuse their id; ids ≥ deck size are mid-combat
  additions). Every recorded play therefore asserts one position of
  the predicted opening permutation — `tools/mcr_validate.py` uses
  this as an automatic per-fight Shuffle pin.
- For checksum-bearing captures with a paired entry save,
  `tools/mcr_validate.py` also treats the first `After player turn start`
  snapshot as the external action-entry carrier. It installs all nine RNG
  streams represented by `combat_sim.State` from the snapshot's exact
  counter plus four xoshiro words, compares the complete ordered opening
  Hand and Draw, then compares each recorded Play/Use-Potion result with the
  bijectively matched `finished action execution ...` snapshot. The exact
  projection covers all five ordered piles (including serialized local
  energy cost), player/live-monster HP/max-HP/block and supported power
  amounts, player turn/phase/resources, and the same nine RNG streams. A
  recognized card modifier, native power row, generated-PowerId field,
  creature shape, or RNG carrier outside that projection is an explicit I5
  refusal, not an omitted comparison.
  Empty-checksum v0.108 captures retain the older play-index path unchanged
  (`test_mcr_v110_captures.py`, issue #649).

## Counter caveat (2026-07-14) — WITHDRAWN 2026-07-26 (#639): it was our bug

**The `.mcr` snapshot always carried the full, cumulative rng set.** The
"caveat" — that a replay holds only `UpFront` + player `Rewards` with
non-run-cumulative values — was a bug in `mcr_parser.py`, not a property of
the game. `run_rng_set` did

```python
counters[self.enum("RunRngType")] = r.i(32)   # WRONG
```

and Python evaluates the value expression before the subscript, so the
32-bit counter was read where the 5-bit stream id lives and vice versa. The
bit total per entry is the same either way (5 + 32), so every file still
decoded with zero unconsumed bits — into garbage name/value pairs, of which
`{"UpFront": 11}` was one. `SavedProperties` had the identical mistake
(names paired with the wrong values). Both are fixed; both classes of read
now bind the name to a local first.

Evidence for the withdrawal, all on the committed v0.108.0 fixtures:

- `HYHM8WP1E5_boss.mcr` decodes to all 12 run streams and 3 player streams
  that **equal `HYHM8WP1E5_boss_entry.save`** — the very `current_run.save`
  distillation the contradiction was built on — counter for counter, plus
  both seeds (pinned by `test_v108_counters_match_the_paired_json_save`).
- `latest.mcr` (ZPJHU3WSH2 fight 2) decodes `Shuffle: 18`, the value the
  recorded inputs independently proved.
- Across seed 8DVXPWUWRY the counters **advance per fight** exactly as the
  live saves did — Shuffle 21 → 48 → 110 → 144 → 172 → 207 → 294, Niche
  1 → 2 → 5 → 9 → 10 → 13 → 17 — while repeated captures of one fight
  (the gardeners/tunneler retries) share one entering value, which is what
  a per-fight ENTERING snapshot must look like. The 2026-07-14 conclusion
  that the dict was "frozen run-start dead weight" is retracted: it was
  reading the same garbage every time because the garbage came from the
  stream-id bits, which barely change.

Consequences:

- Everything the 2026-07-14 divergence trace ruled out (two RunManager
  instances, anonymization, a ToSave branch, serializer entry-dropping, mod
  patching) was ruled out correctly, and there was never anything left to
  explain. `RunManager::ToSave` (RVA 0x4ede0) → `RunRngSet::ToSerializable`
  (0x5059c) does dump every stream, and the wire form does carry them.
- **A `.mcr` is a valid entering-counter carrier on every build.**
  `current_run.save` is still the finer-grained record (it is written
  continuously, not once per combat) and remains the source `live_coach.py`
  uses, but a replay no longer needs it to establish the entering state.
- v0.109 goes further: each stream also carries its four xoshiro256** state
  words, so the entering position is exact rather than counter-inferred.
  Verified byte-for-byte against the JSON save written two minutes before
  the same fight (seed 7XDBEBWZ1REL, `111629.527_save_1d565e24.save`): all
  15 streams agree on counter and on s0..s3, run seed `7XDBEBWZ1REL`,
  player seed `5758252670655643806`.

Unchanged and still true: a save written mid-fight shows the fight's
ENTERING values (combat consumption merges back into the run-scope set only
at combat end; observed live), `current_run.save` is deleted when the run
ends, and `tools/mcr_watch.py` archives it on every change alongside
`latest.mcr`.

One claim from the old section survives on its own merits: **Neow's option
generation consumes zero `player:Rewards` draws** — every event rolls on its
own `Rng`, seeded per event and starting at counter 0
(`EventModel/<BeginEvent>d__28::MoveNext`, RVA 0x366380; see
`STREAM_CONSUMERS.md` → "Per-event streams" and `sim/v0.111.0/python/neow.py`). It was
recorded as a correction (#626) to a gloss on the garbled numbers; the
underlying IL read stands.

## Bitstream primitives (PacketWriter/PacketReader)

LSB-first bitstream: values are written low-bit-first into the low bits
of each byte. Multi-byte values are little-endian before packing, so a
byte-aligned 32-bit field looks like a plain LE uint in a hex dump.

| primitive | encoding |
|---|---|
| `WriteInt/WriteUInt(v, bits)` | low `bits` bits of the LE representation; only a full 32/64-bit read is sign-extended |
| `WriteBool` | 1 bit |
| `WriteByte(v, 8)` | 8 bits |
| `WriteString` | int32 UTF-8 byte count, then the bytes |
| `WriteFloat` (unquantized) | raw IEEE-754 single, 32 bits |
| `WriteEnum<T>` | `ceil(log2(maxEnumValue) + 1)` bits (max **value**, not member count — `MaxEnumValueCache`) |
| `WriteList<T>(bits)` | count as `WriteInt(count, bits)` (always 32 in this format), then each item's `Serialize` |
| nullable / null-object | 1 guard bit, then the value if set |

### Model-id net ids

`ModelIdSerializationCache` maps model ids to dense ints:

1. Take every type in the source-generated `AbstractModelSubtypes` list
   (1650 types in v0.108.0). ModelId = category + entry, where category =
   `Slugify(name of ancestor class directly below AbstractModel)` minus a
   trailing `_MODEL` (e.g. `CardModel` → `CARD`), and entry =
   `Slugify(class name)` (`SkulkingColony` → `SKULKING_COLONY`).
   Slugify inserts `_` before every capital preceded by an alphanumeric,
   then uppercases.
2. Sort by ModelId, ordinal compare on category then entry. (Mods sort
   after base-game content; gameplay-affecting content first.)
3. Net ids are assigned in order of first appearance — **after seeding
   both maps with the `NONE` sentinel at id 0**.
4. Wire widths: `ceil(log2(count))` → v0.108.0: 20 categories = 5 bits,
   1648 entries = **11 bits**, 57 epochs = 6 bits. v0.109.1: 20 / 1654
   (still 11 bits) / 57, and 47 property names (6 bits).
5. `modelIdHash` = XxHash32 over UTF-8 bytes appended as a side effect of
   the net-id assignment. The game logs it on boot
   (`ModelIdSerializationCache initialized. … Hash: …`) and stores it in
   every replay header; `build_mcr_tables.py` reproduces it exactly, which
   proves the table. **The recipe changed between v0.108 and v0.109**
   (issue #639):

   | build | appended, in order | value |
   |---|---|---|
   | v0.108.0 | (category, entry) per sorted model; each epoch id; the three max-id counts as int32 LE | `0x75EE2DEF` |
   | v0.109.x | (category, entry) per sorted model; then each NEW `[SavedProperty]` name, model types in the same sorted order; then each epoch id. **No counts.** | `0xFC40A98D` |

   The v0.109 order is exactly `Init`'s own three passes (RVA 0x2139d4):
   the model loop appends category+entry for every item, the second loop
   calls `CachePropertiesForType(type, hasher, buffer)` which appends each
   property name the first time it gets a net id, and the epoch loop
   appends every epoch id. `GetCurrentHashAsUInt32` is called right after
   the epoch loop, before the bit sizes are computed — hence no counts.

`WriteModelEntry` writes just the entry id (11 bits); `WriteFullModelId`
writes category + entry; `WriteEpochId` writes an epoch id (6 bits).
`SavedProperties` property names use the same trick over `[SavedProperty]`
attributes (47 names in v0.109.1, 6 bits, in ModelId-then-(order, name)
order). Reflection semantics matter for that set: `GetProperties` is called
with `Instance | Public | NonPublic`, so static properties are excluded and
a base class's *private* properties are visible only on their declaring
type.

### Tables are per game build

`mcr_tables.json` always tracks the CURRENT build; replays from older
builds decode against a frozen copy, keyed by the version string in the
header (`mcr_parser.TABLES_BY_VERSION`). At a version bump:

1. copy `mcr_tables.json` to `mcr_tables_<old version>.json`,
2. register that file in `TABLES_BY_VERSION`,
3. regenerate `mcr_tables.json` from the new sts2.dll.

Skipping step 1 orphans every committed replay from the old build — and
old tables cannot be rebuilt once Steam deletes the DLL, which is why
`mcr_tables_v0.108.0.json` is frozen rather than regenerable.

## Top-level layout (CombatReplay)

```
string   version            e.g. "v0.108.0"
string   gitCommit          e.g. "58694f64"
uint32   modelIdHash        must match your tables (see above)
int32 n + n × uint32        choiceIds
int32 n + n × int32         rewardIds
uint32   nextActionId
uint32   nextChecksumId
uint32   nextHookId
SerializableRun              full run snapshot at combat start
int32 n + n × CombatReplayEvent
int32 n + n × ReplayChecksumData
zero padding to byte boundary
```

### SerializableRun (the snapshot)

In order: schemaVersion int32; acts `List<SerializableActModel>`;
modifiers; nullable dailyTime int64; GameMode enum; currentActIndex
int4; eventsSeen entry-list; nullable preFinishedRoom; 4 × float32 run
odds; players `List<SerializablePlayer>`; **SerializableRunRngSet** (seed
string + int8 count × [RunRngType enum(5 bits) + SerializableRng]; in
v0.108 the entry was a bare int32 counter — see "wire changes"); shared
relic grab bag; visited MapCoords (2×8 bits each); map-point history
(int32 n × `List<MapPointHistoryEntry>`); save/start/run/win times int64;
ascension int8; nullable map drawings; extra fields (bool startedWithNeow,
int32 testSubjectKills, bool freedRepy); numReloads int32.

`SerializablePlayer`: netId u64, character entry, hp/maxHp int32, energy
int16, potion slots int8, gold int32, orb slots int16, deck/relics/
potions lists, **SerializablePlayerRngSet** (seed — uint32 in v0.108,
uint64 in v0.109 — + entries keyed by PlayerRngType enum(2 bits)), odds
2 × float32, relic grab bag, extra
fields, unlock state, 4 × discovered full-model-id lists + epochs.

`SerializableCard`: entry id, upgradeLevel int8, nullable enchantment,
nullable SavedProperties, nullable floorAddedToDeck int8 — the same
(id, floor_added) identity relay_parser.py uses.

Full field-by-field detail is the decoder itself (`mcr_parser.py`), which
is a 1:1 transcription of the IL.

### CombatReplayEvent

`eventType` is written as a **3-bit int**:

| type | payload |
|---|---|
| 1 GameAction | playerId u64, action-type byte, action payload |
| 2 HookAction | playerId u64, hookId u32, GameActionType enum(3) |
| 3 ResumeAction | actionId u32 |
| 4 PlayerChoice | playerId u64, choiceId u32, NetPlayerChoiceResult |

Action-type bytes index the 11 `INetAction` implementations sorted by
class name (`NetTypeCache`): 0 ConsoleCmd, 1 DiscardPotion, 2 EndPlayerTurn,
3 MoveToMapCoord, 4 PickRelic, **5 PlayCard**, 6 ReadyToBeginEnemyTurn,
7 UndoEndPlayerTurn, 8 UsePotion, 9 VoteForMapCoord, 10 VoteToMoveToNextAct.

`NetPlayCardAction`: combatCardIndex u16, card entry id (11 bits),
nullable target u6. `NetUsePotionAction`: potionIndex u4, bool
enqueuedInCombat, nullable target u6, nullable targetPlayer u64.

`NetPlayerChoiceResult` switches on PlayerChoiceType enum(4):
CanonicalCard → card model list; CombatCard → u16 combat indexes;
DeckCard → u16 deck indexes; MutableCard → SerializableCard list +
nullable owner u64; Player → nullable u64; Index → int32 list.

### ReplayChecksumData

NetChecksumData (id u32 + checksum u32), context string, and a full
`NetFullCombatState`. The v0.108 samples all carry an EMPTY checksum list;
every v0.109 capture carries one snapshot per engine action (~1.4 per
recorded event, contexts like `After player turn start` and
`finished action execution PlayCardAction card: CARD.BASH …`), and they
dominate the file size. Transcribed from the IL 2026-07-26 (#639):

```
NetFullCombatState (0x2c5eb8)
  int32 n × CreatureState        (0x42282c)
  int32 n × PlayerState          (0x422a68)
  SerializableRunRngSet          rng at that instant
  int32 n × uint32  nextChoiceIds
  int32 n × int32   nextRewardIds
  nullable uint32   lastExecutedActionId
  nullable uint32   lastExecutedHookId

CreatureState   nullable monster entry, nullable playerId u64,
                currentHp/maxHp/block int32, int32 n × PowerState
PowerState      power entry + amount int32                (0x422991)
PlayerState     playerId u64, character entry, turnNumber int32,
                PlayerTurnPhase enum, energy/stars/maxPotionCount/gold
                int32, int32 n × CombatPileState, int32 n × potion entry,
                int32 n × SerializableRelic, int32 n × OrbState,
                SerializablePlayerRngSet, SerializableRelicGrabBag
CombatPileState PileType as a RAW int32 (not WriteEnum) + int32 n ×
                CardState                                 (0x422c95)
CardState       SerializableCard, nullable (affliction entry +
                afflictionCount int32), nullable energyCost int32,
                nullable (3-bit count × CardKeyword enum)  (0x422ce0)
OrbState        orb entry + passive int16 + evoke int16    (0x422a35)
```

This is the richest validation surface in the file: the engine's own hand
and **draw-pile order** at every action boundary, plus monster HP, block,
and power stacks — the exact evidence #636 needed when a predicted hand
disagreed with the player's screen.

### v0.108 → v0.109 wire changes

The decoder branches on the header version (`McrDecoder.v`):

- **rng streams.** v0.108 wrote a bare int32 counter per stream. v0.109
  writes `SerializableRng` = counter int32 + the four xoshiro256** state
  words as uint64 (0x408fc), in both `SerializableRunRngSet` (0x458a0) and
  `SerializablePlayerRngSet` (0x4042c). Decoded as `counters` (unchanged
  shape) plus a new `states` dict.
- **player rng seed.** uint32 in v0.108, uint64 in v0.109.
- **checksums.** Present and non-empty in v0.109 (see above).

## Regenerating the tables after a game patch

```
python3 sim/v0.111.0/python/tools/build_mcr_tables.py
```

reads the installed sts2.dll and rewrites `sim/v0.111.0/python/mcr_tables.json` (freeze
the outgoing one first — see "Tables are per game build"). If a patch
changes the model set, decode() fails fast with a modelIdHash mismatch
rather than mislabeling cards. `tools/dump_serializer.py` (IL dumper that
resolves generic tokens) is the tool for re-deriving schema changes.

Regenerating is necessary but **not sufficient**: the hash RECIPE can
change too, and then a rebuilt table agrees with itself while disagreeing
with the game (v0.108's tables said `0x75EE2DEF`, a v0.109 rebuild with the
old recipe said `0x6893E9D8`, and the game wrote `0xFC40A98D` — #639). So
the check that matters is a real recorded replay, which is why
`test_mcr_parser.py` pins the hash literal each build wrote and decodes one
committed replay per build.

## What this unlocks (issue #61; counter claim corrected twice — 2026-07-14, then withdrawn 2026-07-26)

- **RNG-replay validation is automatic**, from the input log AND the
  snapshot counters: each recorded play pins one position of the opening
  permutation, and the snapshot's rng set supplies the per-stream entering
  counters (plus, on v0.109, the exact xoshiro state). The 2026-07-14
  correction that only `current_run.save` could supply them is withdrawn —
  see "Counter caveat".
- **Per-turn action data without a mod** (#57): exact cards played with
  targets and potion timing. A watcher app that uploads latest.mcr on
  change (alongside .run) gets everything "here's where your line
  diverged" needs.
- **Per-card damage attribution (#28)**: events + validated RNG replay +
  a combat sim = exact attribution, still no mod required.
