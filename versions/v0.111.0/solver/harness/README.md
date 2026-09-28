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
(`versions/v0.111.0/solver/testdata/8DVXPWUWRY_gardeners_entry.save`) through the game's own
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
solver/harness/test_experiment.py -q`.

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
`versions/v0.111.0/solver/content/encounters/` silently stayed on the v0.108 scheme after the
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
pinned in `versions/v0.111.0/solver/test_encounter_stream_seeding.py`. **Re-run on every game
version bump**, like `probe_rng_seeding.py`.

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
