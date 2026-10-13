# Headless engine harness (spike, #275)

Loads the game's own `sts2.dll` into a bare CoreCLR (no Godot engine, no game
process) and drives real combat through the game's own code. Proven end to end
on 2026-07-16: constructed an Ironclad run + CultistsNormal fight and played a
Strike through the real `PlayCardAction` pipeline — energy 3→2, hand 5→4,
Calcified Cultist 38→32 hp, full `CombatHistory` entries. Whole run (CLR boot +
ModelDb.Init + combat + play): **0.84 s**; post-boot steps are milliseconds.

This is the executable-source-of-truth oracle for #138-style conformance
testing: for any (fight state, action) ask the real engine what happens and
diff against the model (the Rust crate; until #2827 deleted it, the Python
`combat_sim.py`). It is a **local dev tool** — `sts2.dll` cannot be
redistributed, so nothing here may become a prod dependency.

## Setup (one-time)

```sh
# .NET 9 runtime, local dir (no system install; ~90MB)
curl -sSL https://dot.net/v1/dotnet-install.sh | bash /dev/stdin \
    --channel 9.0 --runtime dotnet --install-dir ./dotnet
python3 -m venv venv && venv/bin/pip install pythonnet
```

`host.py` expects `dotnet/` next to it and finds the game at the standard
Steam path. `spike.runtimeconfig.json` is gitignored but static; `host.py`
writes it on first import if missing (#329), so a fresh clone/worktree needs
only the dotnet runtime + venv above.

## Run the demo

```sh
venv/bin/python demo_play_strike.py
```

## How it works (load-bearing discoveries)

- STS2 is Godot 4 + plain .NET 9. `MegaCrit.Sts2.Core.*` is a clean model
  layer: `CombatState`/`Player`/`Creature` extend `Object`, all content extends
  `Models.*`. The view lives in `Core.Nodes.*` (Godot types) behind a command
  layer.
- **Mega Crit ships their test seams in the release DLL** and we use them:
  `TestMode.TurnOnInternal()` (gates `Cmd.Wait` etc. to no-op),
  `SaveManager.MockInstanceForTesting` + `Saves.Test.MockGodotFileIo`,
  `RunState.CreateForTest`, `RunManager.SetUpTest`, `Player.CreateForNewRun`,
  `AbstractModel.MutableClone`, `TestRngInjector` (unused so far).
- Content comes from `ModelDb.Init()` (requires `ModManager.State =
  Initialized` first); never construct model classes directly.
- The remaining Godot native calls are Harmony-patched at runtime (0Harmony
  ships with the game; prefixes are emitted via Reflection.Emit from Python).
  Current shim surface, found empirically + by a `.cctor` census:

  | Patch | Why |
  |---|---|
  | `GD.Print*/PushError/PushWarning` | logging (see log tap below) |
  | `OS.GetCmdlineArgs/HasFeature/GetUserDataDir` | Logger cctor, paths |
  | `StringName`/`NodePath` ctors + `op_Implicit` | engine string interning, used by many `.cctor`s |
  | `LocString.GetFormattedText/get_Text/ToString` | loc tables not loaded; `Creature.LogName` etc. |
  | `NHitStop.HitStopTask` | damage hit-pause vfx awaits engine time |
  | `PeerInputSynchronizer.GetTicksMsec`, `Godot.Time.GetTicksMsec` | `Godot.Time` cctor needs ClassDB |
  | `TalkCmd.Play`, `ThinkCmd.Play` | speech/thought bubbles (monster Incantation moves speak) |
  | `ConsoleLogPrinter.Print` → stdout | surfaces the game's own logs/errors (`[GAME] …`) |

- Combat bootstrap order (see demo): ModelDb.Init → Player.CreateForNewRun →
  RunState.CreateForTest → CombatState(encounter.MutableClone(), …) →
  RunManager.Instance.SetUpTest (Instance FIRST — the getter lazily creates) →
  player Creature ctor + AddPlayer + encounter.GenerateMonstersWithSlots +
  CreateCreature/AttachCreature/AddCreature → CombatManager.SetUpCombat →
  StartCombatInternal → drain ActionExecutor → SetupPlayerTurn(player, ctx) →
  SetPhaseForAllPlayers(Play) → enqueue actions via
  `ActionQueueSynchronizer.EnqueueAction` and pump `ActionExecutor`.
- Vector2/Color math is managed (safe); anything hitting ClassDB/NativeFuncs
  segfaults as a *call to 0x0* — diagnose with the crash report in
  `~/Library/Logs/DiagnosticReports` and the FirstChanceException tap in the
  demo, then extend the shim.

## Enemy turn (#277, proven)

`demo_enemy_turn.py` continues the Strike demo through a full turn cycle:
`EndPlayerTurnAction` → `ReadyToBeginEnemyTurnAction` (pass a real
`Func<Task>` no-op, NOT null — its ToString logs it) → enemy phase runs the
real AI (`Creature.TakeTurn` → `MonsterModel.PerformMove`; both cultists
performed their canonical Incantations, `PowerReceived RITUAL_POWER +2/+5`)
→ back to Player side, round 2.

**Driver protocol:** the harness is the "view" — per round it must call
`CombatManager.SetupPlayerTurn(player, HookPlayerChoiceContext)` to deal the
hand and reset energy, then `SetPhaseForAllPlayers(Play)` before card plays.
(CORRECTION from an earlier draft: hands do NOT persist across turns — the
certified sim + conformance drift show the real end-turn pipeline discards
the hand. The persistence we first observed was an artifact of this manual
driver protocol skipping the end-turn discard phase — see the conformance
section.)

## State injection (#278, proven)

`demo_state_injection.py` loads a REAL captured `current_run.save`
(`sim/v0.111.0/python/testdata/8DVXPWUWRY_gardeners_entry.save`) through the game's own
deserializers and reproduces the pinned fight setup:

- `SaveManager.FromJson<SerializableRun>(json)` → `RunState.FromSerializable`
  restores everything: deck (acquisition order), relics, potions, hp/gold,
  and — load-bearing for the oracle — **exact RNG stream counters**
  (shuffle 110, niche 5, sel 2, ai 4 verified against the fixture).
- Wire the singletons before rolling anything:
  `RunManager.Instance.set_State(run)` AND
  `set_AscensionManager(new AscensionManager(run.AscensionLevel))` —
  monster HP ranges go through `AscensionHelper.GetValueIfAscension`, which
  reads `RunManager.Instance`. (Symptom of forgetting: every HP exactly one
  low — A8+ gardener range is 27–32 vs 26–31.)
- Result: engine rolled monster HP **[28, 31, 29, 30]** — identical to the
  live fight and to `combat_sim.py`'s prediction (`test_live_captures.py`).
  Three-way conformance (engine = live game = sim) on real mid-run state:
  the first #279 datapoint.

## Conformance runner (#279, working — one driver gap left)

`conformance.py` replays a captured .mcr through the engine from its entry
save. The .mcr events ARE the engine's own Net* actions, so they feed back
nearly raw (`PlayCardAction(player, NetCombatCard(idx), ModelId, targetId)`,
`UsePotionAction(player, potionIndex, ...)`). Hard-won mapping facts:

- `.mcr target_id` = creation-order `Creature.CombatId`; the player creature
  must be added FIRST (id 0) so monsters get 1..N. Do NOT call
  `AttachCreature` after `CreateCreature` (it double-increments the id
  counter) — `AddCreature` only.
- Boot via SetUpTest-minus-InitializeNewRun (that last step is new-run-only:
  "Grab bag was already populated" guard): set_State → InitializeShared →
  InitializeRunLobby → CombatStateSynchronizer.IsDisabled = true.
- New shims this round: `Engine.GetMainLoop` → uninitialized `SceneTree`
  shell + `SceneTree.get_Root` → uninitialized `Window` +
  `Node.GetProcessDeltaTime` → 1e9 (CardPileCmd.Shuffle's animation pacing
  has no headless guard and dereferences the tree).

**End-turn fix (#279, landed)**: the driver no longer runs any manual turn
protocol. Setting `LocalContext.NetId` (static, set at login in-game) to the
loaded player's NetId makes the engine's OWN pipeline run end-to-end:
EndPlayerTurnAction → EndPlayerTurnPhaseOne (hand discard, ethereal exhaust)
→ enemy turn → StartTurn deal. Verified self-consistent on v0.109: 5 dealt →
5 discarded → fresh 5, energy reset, correct enemy damage. The old manual
`SetupPlayerTurn`/`SetPhaseForAllPlayers`/RBET calls are GONE — if the
pipeline stalls, fix the wiring; don't reintroduce them (they race the
engine and skip the discard).

## Game v0.109 (2026-07-16) — patch-churn notes

- `ModelDb.Init` gained an optional `Type[] injectedModelTypes` param
  (exact-arity reflection must pass null).
- `NCard.FindOnTable` (new scene query in OnPlayWrapper's multi-play anim)
  → shimmed.
- Save schema v18→v19: the game's migration runs in the harness, BUT it
  zeroes RNG stream counters (v19 serializes full xoshiro state that the
  migration doesn't synthesize). `_restore_rng_counters` fast-forwards each
  stream by the fixture's recorded counter — valid because the ALGORITHM is
  unchanged (MegaRandom = xoshiro256** + splitmix64 = sts2_rng.py's model).
- **BREAKING for the solver**: per-stream SEEDING changed —
  `Rng(seed, name)` now seeds from `runSeed64 + GetDeterministicHashCode(name)`
  (was a different 32-bit derivation). Same save + counters now deals
  different hands than v0.108. All v0.108 captures/pins can no longer be
  engine-replayed for identity, and sts2_rng.py predicts wrong streams for
  v0.109 runs. Tracked in its own issue.

## Experiment kit — `run_experiment()` (#307)

`experiment.py` is a thin utility on top of the driver: build a fresh run from
scratch (character + deck + relics + potions), start an encounter, play a
**single turn** of actions, and get back a structured report — no live capture
needed. It answers "what does the engine actually do here?" for #138 wave
sessions and hook-order disputes.

```python
from experiment import run_experiment
report = run_experiment(
    character="Ironclad", ascension=10,
    deck=["CARD.STRIKE_IRONCLAD"] * 8,      # [] keeps the character's starting deck
    relics=["RELIC.KUNAI"], potions=[],
    encounter="ENCOUNTER.CULTISTS_NORMAL", seed="TESTSEED",
    line=[("play", 0, 1), ("play", 0, 1), ("end",)],
)
```

CLI (each run is a fresh CLR process — one CLR per process):

```sh
venv/bin/python experiment.py --demo          # built-in Ironclad/Cultists demo
venv/bin/python experiment.py spec.json        # {character,ascension,deck,relics,potions,encounter,seed,line}
```

**`line` grammar** — a list of actions:
- `("play", hand_index, target_or_None)` — `hand_index` is the CURRENT hand
  index (the hand reindexes after each play, so index 0 twice plays the first
  two cards). `target` is a creation-order **CombatId**: player is 0, enemies
  are 1..N (same convention as `.mcr` target_id / `conformance.py`).
- `("potion", potion_index, target_or_None)`
- `("end",)` — end the turn; lets the engine run its own end-turn → enemy-turn
  → next-deal pipeline so you can observe the enemy phase.

**SINGLE-TURN ONLY.** At most one `("end",)`, and nothing may follow it. Playing
cards then a single `("end",)` to watch the end-turn/enemy phase is fine;
continuing into a second player turn raises `LineError`. Multi-turn is only an
approximation until the end-turn-discard work lands (#309) and must not be
silently offered — the kit refuses it rather than return a wrong answer (I5).

**Report** (dict; JSON-dumpable) includes: the live `build` stamp (read from
`release_info.json` at runtime — every report is provenance-tagged), the ordered
`combat_history` (CombatHistory entries, exact event order), per-action
`rng_counter_delta` for all 12 `RunRngSet` streams (read via the `_counter`
field), per-action and net `hp_block_delta` per creature (keyed by CombatId),
player `energy` before/after, and the opening hand / initial + final states.
Cite results as `(build version, seed, action line)`.

**Turn-start note (#344 — REPLACES an earlier stale claim):** post-#308,
`<StartCombatInternal>d__105::MoveNext` fires the opening
`CombatManager.StartTurn` ITSELF (v0.109 IL_0344-0346) — the earlier claim
that it "doesn't auto-fire headlessly" is wrong on current builds. The kit
therefore calls NO StartTurn of its own: it sets `LocalContext.NetId` BEFORE
combat setup (required — without it the engine's own StartTurn skips the
hand deal) and just pumps the executor until the engine's opening StartTurn
lands in `Phase == Play`. The kit's old explicit second `StartTurn` re-ran
the whole `AfterSideTurnStart` broadcast, so every turn-1
player-side-turn-start hook fired TWICE (observed: Diamond Diadem block 40
instead of 20, the full fire sequence duplicated in CombatHistory). And as
ever: NEVER the banned manual `SetupPlayerTurn`/`SetPhaseForAllPlayers`/RBET
protocol — it races the engine and skips the discard. `InitializeNewRun` is
deliberately skipped (it would roll the `UpFront` map stream a constructed
combat never touches), keeping the RNG deltas pure.

**#344 cite-trust boundary:** turn-1 player-side-turn-start harness cites
are trustworthy only from the #344 fix forward (the B-harnesskit PR,
2026-07-17 — first cite: `test_relic_grant_fires_turn1_hooks_exactly_once`).
Audit of every PRIOR harness cite (independently re-checked against walks
48/51/52/53/56/57/58 + the ENCOUNTER_MECHANICS.md harness cites): NONE was
turn-1-start-hook-exposed — #338's summon/redirect runs were potion-action
+ enemy-phase observations, #340's Orbit traces were action-driven with no
start-hook listener in any deck, #328's guards observed the enemy phase,
#325/CW34's Demon Form cite is a ROUND-2 fire (the engine end-turn pipeline
ran round-2+ starts exactly once even pre-fix), and walk 58's Diadem
conclusions were drawn only from double-fire-independent observations (it
DOCUMENTS the double-fire). Seed-pinned hand/damage/RNG numbers are
unaffected either way (the old boot never double-dealt: the engine's own
fire ran pre-NetId, broadcasting the hooks but SKIPPING the hand deal, and
only the kit's second fire dealt — one deal, one combat-start shuffle in
both worlds; all pre-fix seed-pinned tests pass unchanged post-fix).

**Relic/potion grants (#343/#330):** run creation populates starting relics
and the one-shot `Player::PopulateRelics` throws "already populated" ever
after, so `_grant_relics` grants extras via the incremental
`Player::AddRelicInternal(MutableClone, -1, false)` (0x2c3208) — the exact
twin of the #331 `AddPotionInternal` potion fix.

**Swallowed-play guard (#339):** the engine wraps card OnPlay tasks in
`TaskHelper.LogTaskExceptions` — an exception inside a card body is logged
to `[GAME]` and SWALLOWED, the action still reports Finished, and the report
would silently show zero effect. The kit raises `SwallowedPlayError` when a
play leaves a `CardPlayStartedEntry` without its `CardPlayFinishedEntry`.
The known instance is FIXED: `SaveManager.Instance.PrefsSave` was null
headless and any card reading `PrefsSave.FastMode` mid-play (Whirlwind's
VFX-delay branch — the whole multi-hit/AoE "stall" of #339) NRE'd; `boot()`
now installs the game's own `InitPrefsDataForTest` seam (0x3f592).

**Local-only.** Needs the game install + the harness venv/dotnet (see Setup).
Never CI, never prod. `test_experiment.py` is triple-gated
(`pytest.importorskip("pythonnet")` + `RUN_HARNESS=1` + game/dotnet existence)
so a plain `python3 -m pytest solver/ -q` skips it everywhere; run it
deliberately with `RUN_HARNESS=1 venv/bin/python -m pytest
python/harness/test_experiment.py -q`.

## Game v0.109.1 validation (2026-07-26)

The #625 point-release gate archived build v0.109.1 (commit `c8c577f6`,
`sts2.dll` sha256
`2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f`)
and booted it with the documented local .NET 9 / pythonnet harness. The
built-in `experiment.py --demo` completed a fresh Ironclad/Cultists run:
ModelDb and combat setup succeeded, two Strikes dealt 6 damage each, and the
engine's own end-turn → enemy-turn → turn-2 deal pipeline completed.

`probe_rng_seeding.py` also completed against the live v0.109.1 engine. All
70 existing v0.109.0 engine-pinned hash/run/player comparisons matched
exactly, including the legacy `old`-prefix path. Full old/new managed
comparison found all 43,794 parsed CIL bodies byte-identical, so no harness
shim or RNG-model change was needed.

## Game v0.110.1 validation (2026-07-31)

Batch 202 / #793 archived build v0.110.1 (commit `db5d3552`, `sts2.dll`
sha256 `5a8fb7eb62510a86fd03653b9210cd8f67b511b632331a1f174042de39c92bd9`)
and reran the local harness against a materially changed combat ABI.

Two compatibility seams changed:

- `RunManager.InitializeRunLobby` RVA `0x50464` gained a third
  `IEnumerable<RunLobbyPlayer>` argument. IL `000d-0017` tests
  `service.Type.IsMultiplayer`; the false branch jumps to IL `0045` without
  reading arg3, whose sole read is multiplayer-only at IL `0023`. The
  single-player harness therefore passes null on the exact native solo path.
- `CombatManager.StartCombatInternal` now accepts the `CombatTurnState`
  that `SetUpCombat` RVA `0x139268` constructs at IL `001f-0025` and stores in
  `_turnState` at IL `0026-0028`. StartCombatInternal RVA `0x139510` captures
  that one argument at IL `0020-0023`. Its nested
  `<StartCombatInternal>d__96::MoveNext` RVA `0x3f4234` awaits turn endings,
  switches sides, and loops while `turnState.IsLive` at IL `05da-05e5`; the
  task sets its result only after that loop at IL `060c-0621`. Waiting for it
  before submitting player actions therefore deadlocks by contract.
  `host.start_combat_internal` waits for the historical zero-arg one-shot
  task, but on v0.110.1 passes the exact private `_turnState`, retains the
  returned task, and lets the engine drive turns concurrently.

With those adaptive seams, a bounded fresh-process `experiment.py --demo`
completed in 1.5 seconds: boot/run/combat setup succeeded, two Strikes dealt
6 each, the enemy phase ran, player turn 2 dealt, energy reset to 3, and the
end-turn reshuffle advanced Shuffle by 4. The live harness pytest pins exact
build `v0.110.1` and the two-Strike damage/history boundary.

The RNG probe matched all 267 scalar hash/run/player observations, and the
Encounter probe matched all 42 per-fight state/counter/roster observations.
Those results admit v0.110.1 to the existing RNG scheme only. Combat solving
stays globally fail-closed under #794 until its stale modeled surfaces are
re-read and remodeled.

## Game v0.111.0 validation (2026-08-13)

Batch 293 / #1214 archived build v0.111.0 (commit `41cef1ea`, `sts2.dll`
sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
and reran every supported local-oracle probe.

The new single-player service constructs `PeerVersionInfo.LocalDefault`
before boot completes. That initializes `PlatformUtil`, whose zero-argument
Null and Steam strategy constructors enter Godot/Steam native state that bare
CoreCLR does not own. The harness now skips exactly those two constructors;
the managed initializer still installs the resulting strategy shells and the
null branch supplies its constant platform value. `NGame.GetGameVersion` is
replaced with the exact version already loaded from `release_info.json`,
avoiding its headless-unsafe ClassDB lookup. `NullPlatformUtilStrategy` and
`SteamPlatformUtilStrategy` are the exact constructor owners. These are boot
shims only, not solver behavior substitutes.

With those seams, a fresh-process `experiment.py --demo` completed through
player turn 2: two Strikes dealt 6 damage each, the enemy Incantation phase
ran, energy reset to 3, and the end-turn reshuffle advanced Shuffle by 4.
The RNG probe matched all 267 scalar observations, the Encounter probe matched
all 42 observations, and both checked card/potion pool censuses regenerated
behavior-identically with only their v0.111.0 provenance changing. The same
boot seam let `tools/extract_values.py` enumerate the exact distinct v0.111.0
viewer table (596 cards, 221 relics, 48 potions, 5 enchantments, 596 colors).
Batch 298 completed the shared-combat audit after Batches 294-297 closed the
card, encounter, RNG/relic, and Inky children. Exact v0.111.0 combat is now
admitted; the harness still establishes the local-oracle and independently
enumerated RNG/census/value surfaces rather than substituting for solver
behavior.

## Character-card pool census — `probe_card_pools.py` (#667)

Splash composes its candidate list from `UnlockState.CharacterCardPools`,
so a handwritten set cannot attest its RNG-significant order, unlock epochs,
single-player filtering, or current-build exclusions. This probe boots
`ModelDb`, asks the real five `CardPoolModel` instances for `AllCards` and
`GetUnlockedCards` at each character epoch, and records every row plus the
exact fully-unlocked solo-Ironclad Splash projection.

```sh
venv/bin/python probe_card_pools.py \
  --output ../card_pool_census.json
```

The checked artifact is local-oracle output from v0.111.0/41cef1ea and is
consumed by DLL-free `test_card_pool_census.py`. Re-run it on every game
build change. A reachable refused leaf without a durable GitHub blocker makes
the probe fail instead of emitting an incomplete admission set.

## Alchemize potion-generation census — `probe_potion_generation.py` (#678/#2056)

Alchemize composes its options from the owner's unlocked potion pool followed
by the Shared pool, filters `CanBeGeneratedInCombat`, then performs a
rarity roll and a within-rarity selection on `CombatPotionGeneration`.
Pool order, live property overrides, float32 thresholds, and sparse belt
slots are all exactness inputs; a handwritten list is not adequate evidence.

This probe records the fully unlocked solo-Ironclad pool and rarity
partitions, 20 deterministic live-engine generation vectors, and actual
`TryToProcure(slot=-1)` results for a sparse, last-empty, and full belt:

```sh
venv/bin/python probe_potion_generation.py \
  --output ../potion_pool_census.json
```

The #2056-refreshed v0.111.0/41cef1ea artifact is consumed by DLL-free
`test_potion_pool_census.py`. Re-run it on every build change. A reachable
refused potion without a durable issue route makes the probe fail. The probe
does not itself model Alchemize or change solver admission. Its current RVAs,
metadata tokens, and normalized CIL hashes attest the two pool providers,
unlock epochs, factory/filter/rarity path, and both RNG draws; R52 Part A
retains that proof while refusing every occupied public belt.

## Per-fight Encounter stream — `probe_encounter_stream.py` (#637)

The 12 named run streams are covered by `probe_rng_seeding.py`, but
`EncounterModel::GenerateMonstersWithSlots` builds an **ad-hoc** per-fight
`Rng` that no save persists — so nothing on the save side can falsify its
seed derivation, and the six roster builders in
`sim/v0.111.0/python/content/encounters/` silently stayed on the v0.108 scheme after the
v0.109 seeding change. This probe is the oracle for that stream:
`MutableClone()` -> `GenerateMonstersWithSlots(run)` on a
`RunState.CreateForTest` run, then reads `_rng._counter` and
`_rng._random`'s xoshiro quadruple by reflection, so the seed the engine
used can be SOLVED for rather than merely checked against one candidate.

Two facts it depends on and encodes:

- private fields of BASE types are invisible to `Type.GetField` — walk
  `BaseType` (`_rng` lives on `EncounterModel`, the instance is a subclass
  like `SlimesWeak`).
- `RunState::get_TotalFloor` = `Sum(MapPointHistory, act => act.Count)` over
  a `List<List<MapPointHistoryEntry>>` (one inner list per act), so a
  NONZERO TotalFloor comes from appending one inner list of N entries.
  Without that the probe only ever measures floor 0 and the floor addend
  stays inferred from the IL.

42 (seed, encounter, TotalFloor) cases against v0.109.1 uniquely selected
`uint64(run_set_seed + TotalFloor + XxHash64(Id.Entry))`; the vectors are
pinned in `sim/v0.111.0/python/test_encounter_stream_seeding.py`. **Re-run on every game
version bump**, like `probe_rng_seeding.py`.

## Real waits — `real_waits.py`, `one_thread.py` (#3931)

The harness turns `TestMode` on, so `NonInteractiveMode.IsActive` (RVA
`0x10a381`) is true. Five things differ from a game someone is playing, and
each removes a point where that game gives the main loop back:

| What | Headless | A played game |
|---|---|---|
| `Cmd/<Wait>d__1` RVA `0x3e8d28` | returns at once (IL_0019-0020) | awaits a SceneTree timer (IL_006e-0096), unless `seconds <= 0` (IL_0025-0032), fast mode is Instant (IL_0037-0050) or the combat is ending and `ignoreCombatEnd` is false (IL_0055-0069) |
| `Cmd/<CustomScaledWait>d__3` RVA `0x3e8b64` | returns at once (IL_0020-0027) | Instant returns (IL_002c-003e); otherwise `Cmd.Wait` of the Standard or the Fast seconds (IL_005c-0107) |
| `ActionExecutor/<ExecuteActions>d__28` RVA `0x3d4310` | awaits `action.Execute()` itself (IL_012c-01ac) | starts it and polls once a `ProcessFrame` until it completes or pauses (IL_01b8-0288) |
| `CardPileCmd.GetTweenForCardsChangingPiles` RVA `0x1307f4` | no tween (IL_0019-0027) | a tween the caller awaits: `CardPileCmd/<Add>d__10` IL_073e-075c, `<Shuffle>d__22` RVA `0x3e4b74` IL_0363-0368 |
| the thread an `await` resumes on | the thread pool: the harness has no `SynchronizationContext` | the main thread, a frame later (Godot's context) |

A point where one flow waits and nothing else can run changes nothing. Order
can differ only where the game runs two flows at once. The claim this section
first carried, that a `CardSelectCmd` selection on a hook context is the only
such place, was wrong: `DoTurnEndCards` is another and needs no selection.

### Where combat code runs two flows at once

The census below is a scan of the whole assembly's IL (51,602 method bodies)
for every primitive that can start or join a second flow, then a read of each
hit outside the UI node classes: `Task.WhenAll` (21 sites, 13 outside UI),
`Task.WhenAny` (4), `Task.Yield` (7, one in combat), `Task.Delay` (96, 14
outside UI), `ContinueWith` (2), `TaskHelper.RunSafely` (417), every direct or
virtual call of an async method whose task is stored in a local or a field or
dropped instead of awaited (76), every `ldftn` of an async method (473, all
but 35 a monster's move or an event's option), every delegate `Invoke` whose
result is stored, dropped, added to a list or passed to `RunSafely`, every
`TaskCompletionSource` constructed (34), and `ToSignal` / `CallDeferred`
outside UI. The assembly has no `async void` method, no `Task.Run` and no
`ConfigureAwait`.

**Sites where combat state can depend on the order.**

| Site | What runs beside what | The trigger to recognise |
|---|---|---|
| `CombatManager/<DoTurnEndCards>d__132` RVA `0x3f3a68` | Per turn-end card three tasks are started and kept in locals: `AddTurnEndCardToPlayPileWithDelay` (IL_0052; `d__133` RVA `0x3f270c`: `Cmd.Wait(delay)`, then `CardPileCmd.Add` to the Play pile), `ResolveTurnEndCardEffects` (IL_0097; `d__134` RVA `0x3f64bc`: awaits the previous card's effect and this card's move, then `OnTurnEndInHandWrapper`, then Exhaust or the result pile) and `TweenTurnEndCardToResultPile` (IL_00a7); one `WhenAll` at the end (IL_012f). The effects are chained and run one at a time. The moves are not: card *i+1* leaves the Hand on a timer (the delay grows by 0.8 s, 0.4 s on Fast, 0.3 s for another player's card, times `1 - i/(i+3)`, IL_00b9-0106) while card *i*'s effect is still resolving. Headless every wait is zero, so the order is move 0, effect 0, move 1, effect 1, … | Two or more cards with `HasTurnEndInHandEffect` in the Hand at a turn end: Bad Luck, Beckon, Burn, Debt, Decay, Doubt, Infection, Regret, Shame, Toxic, Wither. What can read the difference: an effect that counts the Hand (Regret), and any listener on a card changing piles or on the damage of an earlier card |
| `Hook/<BeforeSideTurnEnd>d__80` RVA `0x3d34f4` (three passes), `<AfterSideTurnEnd>d__81` RVA `0x3d11c0` (two), `<BeforeFlush>d__33` RVA `0x3d2acc` (two) | Each listener gets its own `HookPlayerChoiceContext` and the walk awaits `AssignTaskAndWaitForPauseOrCompletion`. A selection inside the listener releases the walk (below); the later listeners of the pass then run beside the listener's remainder, and the pass ends on `WhenAll` of every context's `WaitForCompletion` (e.g. `d__81` IL_0124-016c) | A `CardSelectCmd` selection (`FromHand`, `FromHandForUpgrade`, `FromCombatPile`, `FromSimpleGrid`, `FromChooseACardScreen`: the five callers of `SignalPlayerChoiceBegun`) reached from a side-turn-end or flush listener, directly or through a card it auto-plays or draws, with another listener after it in the same pass |
| `Hook/<AfterDeath>d__28` RVA `0x3cd984`, `<AfterDiedToDoom>d__30` RVA `0x3cdb4c`, `<BeforeSideTurnStart>d__74` RVA `0x3d39a8` | The same release, but these walks never call `WaitForCompletion`: the listener's remainder runs beside the later listeners **and** beside everything the caller does after the walk returns | A selection reached from one of these listeners (Gremlin Horn's draw into a selecting Strike is the committed case). Inside a card play or a potion the remainder is held until that action finishes (the signal's second await, below), which fixes the order; a death outside any action (the enemy turn, turn-end damage) has nothing holding it |
| `CombatManager/<StartTurn>d__100` RVA `0x3f781c`: the hand-draw context once it is assigned `RunAutoPrePlayPhase` (IL_0b1e-0b2e; `d__101` RVA `0x3f670c` first awaits the `SetupPlayerTurn` task, IL_0032, then runs `Hook.AfterAutoPrePlayPhaseEntered`, IL_011c), and `OrbQueue.AfterTurnStart` (IL_08a0-08c5) | Neither context is ever passed to `WaitForCompletion`. Once a selection has released it, `StartTurn` goes on to the next player, `CheckWinCondition` (IL_0bd2), the executor's `Unpause` (IL_0c5d) and the rest of the turn start beside the listener's remainder | A selection during the hand draw (`SetupPlayerTurn`, `d__102` RVA `0x3f6c6c`: a `BeforeHandDraw` relic such as Toolbox, a drawn card that is auto-played, an `AfterPlayerTurnStart` listener at IL_0451), or in an `AfterAutoPrePlayPhaseEntered` listener (Whispering Earring, History Course, Mayhem, Imbued, Bombardment, through the cards they play), or in an orb's turn-start passive. On the opening turn the executor is paused until IL_0c5d, so the remainder cannot start before `StartTurn`'s tail has run: the order is fixed there. With a prompt the player holds the remainder in every mode |
| `RollingBoulderPower/<AfterPlayerTurnStart>d__8` RVA `0x342bdc` | Under `TestMode` one `DoDamage` over the hittable enemies (IL_0077-009a). In a played game the boulder's node calls a lambda per creature it reaches (`<>c__DisplayClass8_0::<AfterPlayerTurnStart>b__0` RVA `0x342bb2`: `DoDamage` for that one creature, the task added to a list; connected at IL_0137-0148) and the listener awaits the node's signal and then `WhenAll` (IL_01f0) | Rolling Boulder with two or more hittable enemies. The harness cannot take the played-game branch at all (no nodes); its traces are the `TestMode` branch and say nothing about the order or the overlap of the per-creature damage |
| Multiplayer only: `CombatManager/<EndPlayerTurnPhaseOneInternal>d__130` RVA `0x3f4a4c` (IL_020c-0241 and IL_0567-0594, awaited at IL_0338 and IL_0660), `<EndPlayerTurnPhaseTwoInternal>d__139` RVA `0x3f54e8` (IL_016b-0193, IL_0264), `KnowledgeDemon/<CurseOfKnowledgeMove>d__38` RVA `0x3607a8` (IL_010a-0130), `BranchingPlayerChoiceContext/<SignalPlayerChoiceBegun>d__15` RVA `0x3d5eb0` (IL_022e, IL_024d; reached from `CardModel/<OnPlayWrapper>d__339` IL_0641-0669, `PotionModel/<OnUseWrapper>d__75` IL_03a2 and `CardPileCmd.DrawWithoutBlockingOnOtherPlayers` RVA `0x13142c`, which Huddle Up calls) | One context or task a player, joined after the loop over the players. With one player the loop has one element and the join follows it directly: one flow | More than one player. Not reachable from these probes |

**Sites that are one flow, or touch no combat state.**

| Site | Why |
|---|---|
| `CombatManager/<StartTurn>d__100` IL_069f-06d4: the hand draw, from its selection to the point `StartTurn` assigns it | The walk awaits `WaitForPauseOrCompletionWithoutAssigningTask` (`d__35` RVA `0x3d6aa0`), which waits on `_pausedBeforeTaskAssignedCompletionSource`. A selection completes that source before anything else (`<SignalPlayerChoiceBegun>d__37` IL_01d4-01d9) and then waits for a task that is assigned only at IL_0b29. So `StartTurn` is released at the signal in every mode, headless too, and `Hook.AfterSideTurnStart` (IL_07bf, awaited) and the orbs run to their end while the listener stands still: a fixed order, which is why a Toolbox pick records the same in every mode. What follows the assignment is the `StartTurn` row of the first table |
| `CombatManager/<AfterAllPlayersReadyToBeginEnemyTurn>d__137` RVA `0x3f283c` IL_008e-0097 | `await Task.Yield()`: one flow, a frame later. Headless without a context it moved the rest of the turn onto a pool thread (`one_thread.py`) |
| `ActionExecutor/<WaitForUnpause>d__31` RVA `0x3d4880` | A poll while the executor is paused: `Task.Delay(50)` under `TestMode` (IL_0038-0046), a `ProcessFrame` otherwise (IL_009d-00b1). Paused from `StartCombatInternal` (`d__98` RVA `0x3f71b0` IL_00d7) to the opening `StartTurn` (IL_0c5d) only |
| `GameAction/<Execute>d__45` RVA `0x3d4c28` IL_0095-009a, IL_014b-0168; `ActionExecutor.ActionQueueChanged` RVA `0x10b93e` IL_001b-0020; `CombatManager.AfterCombatRoomLoaded` RVA `0x135aa0` IL_0025-0033 | The executor and the turn loop are started with `RunSafely` and are the two long-lived flows of a combat. A player's actions run in the executor while the turn loop waits for the turn to end (`<WaitUntilQueueIsEmptyOrWaitingOnNonPlayerDrivenAction>d__128` RVA `0x3f9008`), and the turn loop's own work (turn end, enemy turn, turn start) runs with the queue empty except for the hook actions its listeners enqueue, which are the rows above |
| `ActionQueueSynchronizer.GetHookActionForId` RVA `0x110df8` IL_005e-0074 | A `ContinueWith` on a hook action's `ExecutionStartedTask` that calls `HookActionStarted`: bookkeeping of the pending-hook list, for a hook action another peer announced |
| `CombatState/<GetCreatureAsync>d__60` RVA `0x3f927c` IL_0079-0084, IL_00e8-0108 | Returns at once when the creature exists; otherwise `WhenAny` of a timer (`<GodotTimerTask>d__75` RVA `0x3f952c`) and `CreaturesChanged`. Awaited in line by `PlayCardAction` and `UsePotionAction` |
| `CreatureCmd/<Damage>d__12` RVA `0x3e96c8` IL_0830-08b6; `CardCmd/<Transform>d__13` RVA `0x3e0ae0` IL_0751-08ec; `DoomPower.StartDoomAnim` RVA `0xa1ad8` IL_0156-015b | `WhenAll` over hurt animations, over card previews (under `TestMode.IsOff`), over the death animation that becomes `DeathAnimationTask`: nodes only |
| `AttackCommand.BeforeDamage` delegates (`Hyperbeam/<<OnPlay>b__5_0>d` RVA `0x3a6254`, `SweepingBeam/<<OnPlay>b__3_0>d` RVA `0x3c0fd4`) and `CreatureCmd`'s Stun wrapper (RVA `0x461f58`) | Async lambdas: a VFX node and a `Cmd.Wait`, invoked from `AttackCommand/<Execute>d__90` (RVA `0x3f19c0` IL_0664-066c); the Stun wrapper defers an `AddChildSafely` and awaits the stun it was given |
| `TaskHelper.RunSafely` in combat models and commands | Display only, each read at its callee: every relic's `DoActivateVisuals` (21 sites, e.g. `IronClub/<AfterCardPlayed>d__19` RVA `0x326f78` IL_0078-007d: `IsActivating`, `Flash`, a wait), `OnEnqueuePlayVfx` (`CardModel.EnqueueManualPlay` RVA `0x7de0f`, `CardCmd/<AutoPlay>d__0` IL_0532-0537), `NCreature.RefreshIntents` (`Creature.PrepareForNextTurn` RVA `0x11d85c` IL_0046-004b after `RollMove` at IL_002a; `MonsterModel.SetMoveImmediate` RVA `0x825f0`), `InfestedPower.RevealWrigglersAfterDeathAnim` and `StockPower.RevealReplacementAfterDeathAnim` (a wait, then `SetNodeVisible`; the creatures were added before), `ReattachPower.DoFadeOutOnAllSegments` RVA `0xa6674`, the exhaust and preview VFX lambdas of `CardPileCmd` and `CardCmd`, `CombatStateTracker.NotifyCombatStateChanged` |
| `FoulPotion/<OnUse>d__10` RVA `0x34ddd0` IL_0291 | `WhenAll` over merchants' reactions: the branch outside combat |

**What this census does not prove.** It finds flows by the primitives that
start them. A task handed to a delegate type the scan does not name, or stored
through a generic call the disassembly prints without its target, would be
missed; the `Invoke` and `ldftn` passes are there to catch those and found
only what is listed. "Display only" was decided by reading each callee, not
by running it. Godot signals that call back into game code (the Rolling
Boulder row is one) are found only where the connecting code is in a
non-UI class: a node class that changes combat state from an animation
callback is outside this scan. And whether a site in the first table changes
an end state for a given root is not answered here: that is what a recording
in more than one mode is for.

### The hook context, step by step

The walk calls the listener, then
`HookPlayerChoiceContext/<AssignTaskAndWaitForPauseOrCompletion>d__34` (RVA
`0x3d62fc`), which stores the listener's task (IL_0021-0043), completes
`_taskAssignedCompletionSource` (IL_0049-004e) and awaits
`WhenAny(task, _pausedCompletionSource)` (IL_0054-0070).

A selection inside the listener calls `<SignalPlayerChoiceBegun>d__37` (RVA
`0x3d64f4`), which in order:

1. makes the hook action and completes
   `_pausedBeforeTaskAssignedCompletionSource` (IL_01b3-01d9), which only
   `StartTurn`'s hand-draw walk waits on;
2. if the task is not assigned yet, awaits `_taskAssignedCompletionSource`
   (IL_01df-021b);
3. enqueues the hook action (IL_0283-028e) and completes
   `_pausedCompletionSource` (IL_0294-0299);
4. awaits the hook action's `ExecutionStartedTask` (IL_029e-02d3): the
   listener goes on only once the executor has started that action. With the
   executor idle it starts inside the enqueue and this await is already
   complete. Inside a card play or a potion the executor is busy with that
   action, and the listener's remainder waits until it finishes;
5. pauses the action for the choice (IL_02ff-0310). The options are read
   after the signal returns.

Three cases follow.

- **The selection is the first thing the listener awaits.** Step 2 waits; the
  walk assigns the task, and steps 3 to 5 run inside that `SetResult`, with
  nobody awaiting `_pausedCompletionSource` yet. The options are read, a
  selection that needs no prompt resolves, and the listener runs on. In a
  headless trace it runs to its end, because nothing after the selection
  waits. In a played game it runs to its **first await after the selection**
  (a `Cmd.Wait`, a card tween), and there the walk reaches its `WhenAny`,
  finds the pause already set and goes on to the next listener. So this
  listener forks too, later than the next case and after the options were
  read.
- **The listener waited before the selection** (any wait, any tween). The
  walk is already in `WhenAny` when step 3 completes the pause, and its
  continuation runs inside that `SetResult`, before the options are read.
- **`StartTurn`'s hand-draw walk.** Released at step 1 in every mode; the
  listener stays at step 2 until `StartTurn` assigns it, after the side's
  turn-start listeners and the orbs. One order up to there. From the
  assignment on it is the first case: steps 3 to 5 run inside the `SetResult`
  and `StartTurn`'s tail is what the remainder runs beside.

After a release the walk does not start every later listener at once. It
calls the next one and awaits *its* `AssignTaskAndWaitForPauseOrCompletion`,
so the later listeners run one after another, each to its end or to its own
selection. The race is between the released listener's remainder and that
sequence, and each side advances when the other waits.

**With a prompt** the player holds the listener, the later listeners finish
while the prompt is open, and the two orders leave the same state behind.
They can still differ in what the prompt offers: in the second case the next
listener's work up to its first wait runs before the options are read, and
headless after them.

**With no prompt** nothing holds the listener, and its remainder interleaves
with the later listeners by how long each wait takes.
`PowerCmd/<Remove>d__8` (RVA `0x3f09bc`) removes the power (IL_0030) and then
waits 0.2 / 0.4 s (IL_0035-0049) before `AfterRemoved` (IL_00af-00b4), so a
Rage that ends at the side's end is gone before a Hellraiser play the same
pass makes later, which the headless trace shows the other way round (the
6 HP of `d3_seeker_one_rage`, #3920). Past each side's first wait it is a
race between timers and tweens.

### What the mode does

`STS2_WAITS=real[:normal|fast|instant][:always][:after]` (and
`STS2_WAITS_FPS`, default 60) on `probe_relic_hook_choice.py` or the crate's
`tests/native/probe.py`. Unset, `Cmd.Wait` returns at once as it always did
and the committed traces re-record (byte for byte, or on every key the
committed file has where the probe's snapshot has since gained one).

- `Cmd.Wait` and `Cmd.CustomScaledWait` are prefixed and keep the game's own
  gates in its order; a wait it would really take becomes a timer with its
  duration. Real waits start at the scenario's first `snap` (or a
  `["real_waits"]` step): the setup before the root runs headless.
- The fast mode is the whole game's, not only the two waits':
  `SaveManager.Instance.PrefsSave.FastMode` is set to it, because
  `DoTurnEndCards` (IL_00de-00f7), `TalkCmd.Play` (RVA `0x133fe8`) and the
  OnPlay of Flash of Steel, Neutralize, Slice, Suppress and Whirlwind read
  the setting themselves. `Cmd.Wait`'s Instant gate sits behind
  `NGame.Instance != null` (IL_0037-003c); the harness has no `NGame`, a
  played game always does, and the prefix applies the gate as a played game
  would.
- A pump starts the executor if it is idle and runs a frame clock to the
  next expiry. A timer loses `1/fps` a frame and fires below zero; timers
  that run out in one frame fire in creation order; one made while another
  fires waits for a later frame. The continuation runs inline on the pumping
  thread, as a SceneTree timer's `timeout` does. A continuation the game
  posted (`Task.Yield`) takes one frame and runs after that frame's timers;
  that order within a frame is this module's choice, not a measurement of
  the engine.
- Every snapshot is taken with no timer pending and nothing posted, and so is
  the end of the run: a combat that has ended still has waits to run out
  before the dead monster is gone and the play counts are reset.
- `always` makes a hook context's first signal wait one frame when its task
  is not assigned yet: it turns the first case above into the second. It
  stands in for a tween **before** the selection.
- `after` makes a hook context's `SignalPlayerChoiceEnded`
  (`d__38` RVA `0x3d6864`) complete one frame late: the listener yields right
  after its selection, so the walk goes on to the next listener before the
  listener does anything with what was chosen. It stands in for a tween
  **after** the selection, which is where the first case forks in a played
  game, and which neither the timers nor `always` can show when the
  listener's next await is a tween and not a `Cmd.Wait`.
- Each of the two is wrong for a listener that has no await at that point
  (in a played game that one does run straight through), so they are further
  recordings to compare, never the default.

### One thread (`one_thread.py`)

Both probes install a `SynchronizationContext` right after the boot, in every
mode, and run what the game posts to it from their own loop. Before this the
harness had none, so every `await` that did not complete inline resumed on
the thread pool: `Task.Yield()` at the start of the enemy turn
(`<AfterAllPlayersReadyToBeginEnemyTurn>d__137` IL_008e) moved the rest of
the turn end, the enemy turn and the next turn start to a pool thread, and
`Task.Delay(50)` in `WaitForUnpause` did the same during the opening. The
probes slept between pumps to let that thread finish, and a snapshot taken
too early was a truncated one. With the context the game runs on the probe's
thread only, the sleeps are gone (`idle` waits for a `Task.Delay` to post, and
returns at once when something already has), and the real-waits timer list
has one writer. A wait created on any other thread is listed under
`off_thread_waits` and the run is refused.

Nothing else in the harness installs the context: `demo_enemy_turn.py`,
`conformance.py` and the other scripts block on `GetResult()` for tasks that
cross the `Task.Yield`, which under a context would wait for ever. They run
as they always have. `_Fight` uses the context when a probe has installed it
(`_Fight.posted`) and sleeps as before when none has.

Two real-time waits remain and are not on the frame clock: `Task.Delay(50)`
in `WaitForUnpause` (the opening turn only) and the timeout of
`GetCreatureAsync` (only when a target does not exist yet). A run that
crosses one while two timers are pending is not reproducible to the frame.
`posts` in the report counts every continuation that came through the
context.

### Reading a trace

Each run ends with a `waits` entry: the mode, the count of waits,
`forced_yields`, `max_pending`, under `concurrent` every moment two or more
gameplay timers were pending with the frames that created them (a relic's
flash is left out), under `hook_signals` every selection signalled on a hook
context (`source` is the listener's model, null for `CombatManager`'s own
contexts; `walk_waiting` says the walk was already waiting, the second case
above), `posts`, `off_thread_waits`, `stalled` and `pending_at_end`.

Not emulated: the card tweens and every node animation (no scene tree
exists), the executor's frame poll, and any branch the game takes only with
`TestMode` off (Rolling Boulder's). Instant fast mode therefore records like
headless unless `always` or `after` is given, though in a played game on
Instant the tweens still yield.

- No `hook_signals`, `max_pending <= 1`: one flow at a time. The order does
  not depend on any duration; if the trace equals the headless one, the
  headless trace was right.
- A `hook_signals` entry: a second flow could start there. Record `real`,
  `real:always`, `real:after` and `real:always:after`. If all four equal the
  headless trace, no yield at either point changes anything this probe
  snapshots. If they differ, read the listener's IL for the await that is
  really there (a Draw has a tween before a selection it leads to; a
  selection that opens the listener has none before it), and take the
  recording that matches.
- `max_pending >= 2`: two flows raced on timers (the turn-end cards do this
  with no selection at all). The clock gave one order; a tween on either side
  could give another. Neither this trace nor the headless one is evidence for
  the order of the racing effects. If they commute, say so from the
  `concurrent` sites; if not, the engine refuses the root until a capture of
  a played game settles it (`.mcr` checksums are the authority).

Scenarios that are not one of the probe's own can be given in a file:
`STS2_PROBE_SPECS=FILE` adds `{name: spec}` entries to
`probe_relic_hook_choice.py`. Besides the probe's own setup steps (`hp`, `power`,
`mpower`) a spec may use `["pblock", N]` (Block on the player) and
`["kill", CREATURE_ID]`. Two specs that isolate one site each:

```json
{"turn_end_three": {"character": "Ironclad", "relics": [],
   "deck": ["BURN", "REGRET", "DECAY", "DEFEND_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD",
            "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "BASH"],
   "script": [["energy", 3],
              ["arrange", {"hand": ["BURN", "REGRET", "DECAY", "DEFEND_IRONCLAD"],
                           "draw": ["STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD",
                                    "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "BASH"], "discard": []}],
              ["snap", "root"], ["play", "@END", null], ["snap", "end"]]},
 "constrict_seeker_rage": {"character": "Ironclad",
   "encounter": "ENCOUNTER.SLITHERING_STRANGLER_NORMAL", "seed": "RV3920A",
   "relics": ["CENTENNIAL_PUZZLE", "LUNAR_PASTRY"],
   "deck": ["DEFEND_IRONCLAD", "SEEKER_STRIKE", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD",
            "STRIKE_IRONCLAD", "BASH", "TRUE_GRIT", "SHRUG_IT_OFF"],
   "script": [["energy", 3],
              ["arrange", {"hand": [], "draw": ["DEFEND_IRONCLAD", "SEEKER_STRIKE", "STRIKE_IRONCLAD",
                           "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "BASH", "TRUE_GRIT",
                           "SHRUG_IT_OFF"], "discard": []}],
              ["pblock", 60], ["kill", 1], ["power", "HellraiserPower", 1], ["play", "@END", null],
              ["arrange", {"hand": ["DEFEND_IRONCLAD"], "draw": ["SEEKER_STRIKE", "STRIKE_IRONCLAD"],
                           "discard": ["STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "BASH",
                                       "TRUE_GRIT", "SHRUG_IT_OFF"]}],
              ["power", "RagePower", 3],
              ["snap", "root"], ["play", "@END", null], ["snap", "end"]]}}
```

`turn_end_three` (no answers) has no selection and no relic: three turn-end
cards. Its report lists the staggered moves under `concurrent` with
`max_pending` 3, and its trace equals the headless one in `real` and
`real:fast`: none of the eleven turn-end effects reads the Hand while it runs
(Regret's count is taken in `BeforeSideTurnEnd`, RVA `0xe9638` IL_003b-004a).

`constrict_seeker_rage` (answers `0,0`) is #3920's `d3_seeker_one_rage`: at
the side's end Constrict's listener damages the player, Centennial Puzzle
draws, Hellraiser auto-plays the drawn Seeker Strike, and its one-candidate
selection is signalled on the listener's hook context with Rage's
`AfterSideTurnEnd` still to come. The player's HP at the end is 71 headless
and 65 wherever the walk goes on before the Strike's attack:

| Mode | HP | `hook_signals[0]` | Why |
|---|---:|---|---|
| unset | 71 | | nothing waits |
| `real`, `real:fast` | 65 | `walk_waiting: true` | the Draw's waits come before the selection; `max_pending` 2 |
| `real:instant` | 71 | `walk_waiting: false` | no wait is taken on Instant, so the timers show nothing |
| `real:instant:always` | 65 | `walk_waiting: false` | a yield before the selection |
| `real:instant:after` | 65 | `walk_waiting: false` | a yield after it, with none before: the fork at the listener's first await after the selection |

The last row is the case `after` exists for. The committed
`horn_seeker_short` is the other unprompted selection on a hook context
(Gremlin Horn's `AfterDeath` Draw into a Seeker Strike with one candidate),
and records the same in every mode.

### Stalls and children that throw

A run that stops making progress is never written as a trace. A probe step
that times out, a turn end that does not reach the next Play phase
(`_Fight.end_turn`), a drain that runs out of pumps with timers pending, a
task that never completes or a wait made off the pump thread prints
`STALLED <report>` (the reason under `stalled`, the timers under
`pending_at_end`) and exits 3.

`drive` plays every answer sequence in its own process and finds the end of
the options by the selector throwing on an index past the last one. That is
the only thing a missing trace may mean. `probe_drive.read_child` returns
None for exactly that exception (`ArgumentOutOfRangeException` out of
`Enumerable.ElementAt`), accepts the one the close of a combat always logs
(`EndCombatInternal` has no room to hand off to), and raises on anything
else: a `STALLED` line or exit 3, any other exception the game logged, or no
`NAT` line. Before, any logged exception read as "no such option" and
`drive` could write a scenario with `runs: []` and exit 0.

## Known gaps / next steps (tracked on #275)

- Pin-based .mcr certification needs v0.109-native captures + the sts2_rng
  seeding update; the runner itself is ready.
- ARCHIVE GAME DLLS: keep a copy of each version's sts2.dll before Steam
  auto-updates — v0.108 is already gone and its pins are unverifiable
  against a live engine.
- `RequestEnqueue` (net loopback path) crashed pre-shim; direct
  `EnqueueAction` is used instead. Revisit with the full shim.
- Patch churn: every game update changes the DLL; keep the shim census script
  handy (it scans all `.cctor`s for Godot MemberRefs).
