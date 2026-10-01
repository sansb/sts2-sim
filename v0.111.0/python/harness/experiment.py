"""Harness experiment kit (#307): parameterized SINGLE-TURN ground-truth runs.

Ask the REAL engine what a mechanic does, from a CONSTRUCTED state (no live
capture needed). Build a fresh Ironclad/etc. run, override deck/relics/potions,
start an encounter, play a line of actions, and get back a structured report:
the CombatHistory event order, per-creature hp/block deltas, player energy, and
per-action RNG counter deltas for all 12 run streams.

    from experiment import run_experiment
    report = run_experiment(
        character="Ironclad", ascension=10,
        deck=["CARD.STRIKE_IRONCLAD"] * 8,
        relics=["RELIC.KUNAI"], potions=[],
        encounter="ENCOUNTER.CULTISTS_NORMAL", seed="TESTSEED",
        # hand indices are CURRENT-hand positions (the hand reindexes after
        # each play), so index 0 twice plays the first two cards
        line=[("play", 0, 1), ("play", 0, 1), ("end",)],
    )

SINGLE-TURN ONLY. A `line` is a list of actions:
    ("play", hand_index, target_combat_id_or_None)
    ("potion", potion_index, target_combat_id_or_None)
    ("end",)
Target ids are creation-order CombatIds: the player creature is 0, enemies are
1..N (same convention as .mcr target_id / conformance.py). At most ONE ("end",)
is allowed and nothing may follow it -- playing cards then a single ("end",) to
observe the end-turn/enemy phase is fine, but continuing into a second player
turn is REJECTED. Multi-turn fidelity waits on the end-turn-discard work (#309);
until then it would be an approximation and must not be silently offered.

LOCAL-ONLY: needs the game install + the harness venv/dotnet (see README).
Never CI, never prod -- sts2.dll cannot be redistributed.

Validated through build v0.110.1 (commit db5d3552). Cite format:
(build version, seed, action line).
"""
import json
import sys
import time
from pathlib import Path

HARNESS = Path(__file__).parent
SOLVER = HARNESS.parent
sys.path.insert(0, str(SOLVER))
sys.path.insert(0, str(SOLVER / "tools"))

import host  # noqa: E402  (starts CoreCLR on import)
from host import (call, call_static, initialize_run_lobby, prop,
                  start_combat_internal, ALL)  # noqa: E402

from System import Activator, Array, Object, Enum, UInt64, UInt32, Int32, Int64  # noqa: E402
import System  # noqa: E402

asm = host.asm

# The 12 RunRngSet streams, in RunRngSet declaration order.
RNG_STREAMS = (
    "UpFront", "Shuffle", "UnknownMapPoint", "CombatCardGeneration",
    "CombatPotionGeneration", "CombatCardSelection", "CombatEnergyCosts",
    "CombatTargets", "MonsterAi", "Niche", "CombatOrbGeneration",
    "TreasureRoomRelics",
)

# Standard Steam install path for release_info.json (build provenance).
_RELEASE_INFO = (Path.home() / "Library/Application Support/Steam/steamapps/"
                 "common/Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                 "release_info.json")


def build_version():
    """Read the live game build stamp so every report is provenance-tagged."""
    try:
        info = json.loads(_RELEASE_INFO.read_text())
        return {"version": info.get("version"), "commit": info.get("commit"),
                "main_assembly_hash": info.get("main_assembly_hash")}
    except Exception as e:  # pragma: no cover - defensive
        return {"version": None, "commit": None, "error": repr(e)}


def find(simple):
    for t in asm.GetTypes():
        if str(t.Name) == simple:
            return t
    raise KeyError(simple)


def generic_list(elem_t, items=()):
    lt = System.Type.GetType("System.Collections.Generic.List`1").MakeGenericType(elem_t)
    lst = Activator.CreateInstance(lt)
    add = lt.GetMethod("Add")
    for it in items:
        add.Invoke(lst, Array[Object]([it]))
    return lst


def _method(t, name, nargs=None, first_param=None):
    ms = [m for m in t.GetMethods(ALL) if str(m.Name) == name
          and (nargs is None or m.GetParameters().Length == nargs)
          and (first_param is None or
               (m.GetParameters().Length > 0 and
                str(m.GetParameters()[0].ParameterType.Name) == first_param))]
    if not ms:
        raise KeyError(f"no method {name} on {t.Name}")
    return ms[0]


def _deserialize_id(model_id):
    return _method(find("ModelId"), "Deserialize").Invoke(None, Array[Object]([model_id]))


def _model_by_id(model_id, model_type_name):
    """ModelDb.GetById<T>(ModelId) -> the shared model instance for an id."""
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    get_by_id = [m for m in ModelDb.GetMethods(ALL)
                 if str(m.Name) == "GetById" and m.IsGenericMethod][0]
    return get_by_id.MakeGenericMethod(find(model_type_name)).Invoke(
        None, Array[Object]([_deserialize_id(model_id)]))


# --------------------------------------------------------------------------
# line validation: SINGLE-TURN ONLY
# --------------------------------------------------------------------------
class LineError(ValueError):
    """The action line violates the single-turn contract."""


class SwallowedPlayError(RuntimeError):
    """A card play 'Finished' but its effects never resolved (#339).

    The engine's play pipeline wraps card OnPlay tasks in
    TaskHelper.LogTaskExceptions: an exception inside a card body (e.g. an
    NRE on a headless-null singleton) is LOGGED to the [GAME] console and
    SWALLOWED -- the action still reports Finished, energy is spent, and the
    report would silently read as zero effect. That is worse than a crash
    for a ground-truth oracle, so the kit checks after every play that its
    CardPlayStartedEntry got a matching CardPlayFinishedEntry and raises
    this instead of returning a clean report. Look for a '[GAME]
    System.<SomeException> ... at <Card>.OnPlay' stack in the run's output
    to find the swallowed exception."""


def _validate_line(line):
    if not isinstance(line, (list, tuple)):
        raise LineError(f"line must be a list of action tuples, got {type(line)}")
    seen_end = False
    for i, action in enumerate(line):
        if not isinstance(action, (list, tuple)) or len(action) == 0:
            raise LineError(f"action {i} is not a non-empty tuple: {action!r}")
        kind = action[0]
        if seen_end:
            raise LineError(
                f"action {i} ({kind!r}) follows an ('end',): the experiment kit is "
                "SINGLE-TURN ONLY. Playing cards then one final ('end',) to observe the "
                "end-turn/enemy phase is allowed, but continuing into a second player "
                "turn is not (multi-turn is approximate until #309 lands).")
        if kind == "end":
            seen_end = True
        elif kind == "play":
            if len(action) != 3:
                raise LineError(f"action {i}: ('play', hand_index, target_or_None), got {action!r}")
        elif kind == "potion":
            if len(action) != 3:
                raise LineError(f"action {i}: ('potion', potion_index, target_or_None), got {action!r}")
        else:
            raise LineError(f"action {i}: unknown action kind {kind!r} "
                            "(expected 'play', 'potion', or 'end')")


# --------------------------------------------------------------------------
# construction: fresh run + deck/relic/potion overrides
# --------------------------------------------------------------------------
def _boot_and_init():
    host.boot()
    MM = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
    MMState = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState")
    call_static(MM, "set_State", Enum.Parse(MMState, "Initialized"))
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    init = [m for m in ModelDb.GetMethods(ALL) if str(m.Name) == "Init"][0]
    # v0.109: Init gained an optional Type[] injectedModelTypes (pass nulls)
    init.Invoke(None, Array[Object]([None] * init.GetParameters().Length))


def _make_player(character):
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    char_model = call_static(ModelDb, "Get", find(character))
    US = find("UnlockState")
    us = None
    for c in US.GetConstructors(ALL):
        ps = c.GetParameters()
        if ps.Length == 3:
            args = []
            for p in ps:
                t = p.ParameterType
                args.append(generic_list(t.GetGenericArguments()[0]) if t.IsGenericType else Int32(0))
            us = c.Invoke(Array[Object](args))
    assert us is not None, "no 3-arg UnlockState ctor"
    Player_t = find("Player")
    cfnr = [m for m in Player_t.GetMethods(ALL) if str(m.Name) == "CreateForNewRun"
            and m.GetParameters().Length == 3][0]
    return cfnr.Invoke(None, Array[Object]([char_model, us, UInt64(0)]))


def _override_deck(player, deck):
    """Replace the character's starting deck with the given card ids (order kept)."""
    if not deck:
        return  # keep the character's default starting deck
    pile = prop(player, "Deck")
    clear = _method(pile.GetType(), "Clear", 1)
    clear.Invoke(pile, Array[Object]([False]))
    add = _method(pile.GetType(), "AddInternal", 3)
    for idx, card_id in enumerate(deck):
        model = _model_by_id(card_id, "CardModel")
        clone = call(model, "MutableClone")
        add.Invoke(pile, Array[Object]([clone, Int32(idx), False]))


def _grant_relics(player, relics):
    """Grant EXTRA relics on top of the character's starting relics.

    Run creation already populates starting relics, so the one-shot
    `Player::PopulateRelics` throws 'Relics have already been populated.'
    (v0.109 RVA 0x2c3934, guard at IL_0017-0023) for any later grant (#343).
    Use the incremental `Player::AddRelicInternal(model, index, silent)`
    instead (0x2c3208; index -1 = append, AssertMutable at IL_000d) -- the
    same per-relic path PopulateRelics itself runs (IL_0034-0038), and the
    exact twin of the #331 AddPotionInternal fix. silent=False matches what
    run creation passes (PopulateStartingRelics IL_007f `ldc.i4.0`).
    """
    if not relics:
        return
    Player_t = find("Player")
    add_internal = _method(Player_t, "AddRelicInternal", 3)
    for rid in relics:
        model = _model_by_id(rid, "RelicModel")
        clone = call(model, "MutableClone")
        add_internal.Invoke(player, Array[Object]([clone, Int32(-1), False]))


def _grant_potions(player, potions):
    if not potions:
        return
    Player_t = find("Player")
    add_internal = _method(Player_t, "AddPotionInternal", 3)
    for idx, pid in enumerate(potions):
        model = _model_by_id(pid, "PotionModel")
        model = call(model, "MutableClone")
        add_internal.Invoke(player, Array[Object]([model, Int32(idx), False]))


# --------------------------------------------------------------------------
# combat setup + engine-driven turn (NO manual SetupPlayerTurn/RBET protocol)
# --------------------------------------------------------------------------
def _rng_counters(run):
    """Read _counter (Int32) via reflection for all 12 RunRngSet streams."""
    rng = prop(run, "Rng")
    out = {}
    for name in RNG_STREAMS:
        stream = prop(rng, name)
        fld = stream.GetType().GetField("_counter", ALL)
        out[name] = int(fld.GetValue(stream))
    return out


def _creature_snapshot(cs):
    """Per-creature hp/block keyed by CombatId, with a readable label."""
    snap = {}
    for c in prop(cs, "Creatures"):
        cid = int(prop(c, "CombatId"))
        mon = prop(c, "Monster")
        label = "player" if mon is None else str(mon)
        snap[cid] = {"label": label, "hp": int(prop(c, "CurrentHp")),
                     "block": int(prop(c, "Block"))}
    return snap


class _Fight:
    """Fresh-constructed headless combat, driven by the engine's own pipeline."""

    def __init__(self, player, character, ascension, acts, encounter, seed):
        Player_t = find("Player")
        RunState_t = find("RunState")
        cft = [m for m in RunState_t.GetMethods(ALL) if str(m.Name) == "CreateForTest"][0]
        act_models = generic_list(find("ActModel"),
                                  [call_static(asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb"),
                                               "Get", find(a)) for a in acts])
        self.run = cft.Invoke(None, Array[Object]([
            generic_list(Player_t, [player]),
            act_models,
            generic_list(find("ModifierModel")),
            Enum.Parse(find("GameMode"), "Standard"),
            Int32(int(ascension)), seed]))
        self.player = player

        # RunManager owns the ActionExecutor / ActionQueueSynchronizer. Wire it
        # the same way conformance.boot_and_load_run does (SetUpTest-minus-
        # InitializeNewRun): set_State -> InitializeShared -> InitializeRunLobby
        # -> CombatStateSynchronizer.IsDisabled. InitializeNewRun is skipped on
        # purpose -- it repopulates map/grab-bag state (rolling the UpFront RNG
        # stream) that a single constructed combat never touches; skipping it
        # keeps the RNG streams clean so the per-action deltas are pure.
        RM = find("RunManager")
        rmi = call_static(RM, "get_Instance")
        svc = [c for c in find("NetSingleplayerGameService").GetConstructors(ALL)
               if not c.IsStatic][0].Invoke(Array[Object]([]))
        peer_ctor = [c for c in find("PeerInputSynchronizer").GetConstructors(ALL)
                     if not c.IsStatic][0]
        peer = peer_ctor.Invoke(Array[Object]([svc] * peer_ctor.GetParameters().Length))
        _method(RM, "set_State").Invoke(rmi, Array[Object]([self.run]))
        _method(RM, "InitializeShared").Invoke(rmi, Array[Object](
            [svc, peer, False, None, Int64(int(time.time())), Int64(0), Int64(0), Int32(0)]))
        initialize_run_lobby(rmi, svc, self.run)
        self.rmi = rmi
        # Headless has no network peer to confirm combat-state syncs; without
        # this the StartTurn pipeline stalls at phase=Start (never deals).
        css = prop(rmi, "CombatStateSynchronizer")
        _method(css.GetType(), "set_IsDisabled").Invoke(css, Array[Object]([True]))
        # A8+ monster HP ranges read RunManager.Instance.AscensionManager.
        if prop(rmi, "AscensionManager") is None:
            am = [c for c in find("AscensionManager").GetConstructors(ALL)
                  if not c.IsStatic and str(c.GetParameters()[0].ParameterType.Name) == "Int32"][0]
            am = am.Invoke(Array[Object]([Int32(int(ascension))]))
            _method(RM, "set_AscensionManager").Invoke(rmi, Array[Object]([am]))

        # Encounter + CombatState.
        enc_model = _model_by_id(encounter, "EncounterModel")
        self.enc = call(enc_model, "MutableClone")
        cs_ctor = [c for c in find("CombatState").GetConstructors(ALL) if not c.IsStatic][0]
        self.cs = cs_ctor.Invoke(Array[Object]([
            self.enc, self.run, prop(self.run, "Modifiers"), prop(self.run, "BadgeModels"),
            prop(self.run, "MultiplayerScalingModel")]))
        # Creation order pins CombatIds: player FIRST (id 0), monsters 1..N.
        call(self.cs, "AddPlayer", self.player)
        enemy_side = Enum.Parse(find("CombatSide"), "Enemy")
        call(self.enc, "GenerateMonstersWithSlots", self.run)
        for item in prop(self.enc, "MonstersWithSlots"):
            it = item.GetType()
            cr = call(self.cs, "CreateCreature", it.GetField("Item1").GetValue(item),
                      enemy_side, it.GetField("Item2").GetValue(item))
            if cr not in list(prop(self.cs, "Enemies")):
                call(self.cs, "AddCreature", cr)

        self.cm = call_static(find("CombatManager"), "get_Instance")

        # The engine resolves "the local player" via this static (set at login
        # in-game); the turn pipelines (#308) read it. Set it BEFORE combat
        # start: post-#308 the OPENING StartTurn fires inside
        # StartCombatInternal itself, so the wiring must already be in place.
        # (It previously worked pre-wiring only because CreateForNewRun's
        # NetId 0 happens to equal the LocalContext default.)
        LC = find("LocalContext")
        net_id = UInt64(int(str(prop(self.player, "NetId"))))
        _method(LC, "set_NetId").Invoke(None, Array[Object]([net_id]))

        call(self.cm, "SetUpCombat", self.cs)
        self._turn_loop_task = start_combat_internal(self.cm)

        self.pcs = prop(self.player, "PlayerCombatState")
        self.aqs = prop(rmi, "ActionQueueSynchronizer")
        self.ex = prop(rmi, "ActionExecutor")

        self._drain()
        # The OPENING player turn is fired by the ENGINE ITSELF:
        # <StartCombatInternal>d__105::MoveNext calls StartTurn(null) at
        # IL_0344-0346 (v0.109, post-#308) and awaits the full pipeline --
        # deal, energy reset, start-of-turn hooks/relics, Start -> Play.
        # Do NOT call StartTurn here a second time: that re-runs the whole
        # AfterSideTurnStart broadcast and every turn-1 player-side-turn-start
        # hook fires TWICE (#344 -- observed as Diamond Diadem block 40
        # instead of 20). And NEVER the banned manual
        # SetupPlayerTurn/SetPhaseForAllPlayers/RBET protocol (races the
        # engine; see README). All the driver does is pump the executor until
        # the engine's own opening StartTurn lands in Phase == Play.
        t0 = time.time()
        while time.time() - t0 < 15:
            if str(prop(self.pcs, "Phase")) == "Play":
                break
            call(self.ex, "Unpause")
            call(self.ex, "ExecuteActions")
            time.sleep(0.05)
        assert str(prop(self.pcs, "Phase")) == "Play", \
            f"opening turn never reached Play: {prop(self.pcs, 'Phase')}"

    def _drain(self):
        call(self.ex, "Unpause")
        call(self.ex, "ExecuteActions")
        t = call(self.ex, "FinishedExecutingActions")
        call(call(t, "GetAwaiter"), "GetResult")

    @property
    def over(self):
        return bool(prop(self.cm, "IsOverOrEnding")) or not bool(prop(self.cm, "IsInProgress"))

    def _creature_by_id(self, combat_id):
        if combat_id is None:
            return None
        for c in prop(self.cs, "Creatures"):
            if int(prop(c, "CombatId")) == int(combat_id):
                return c
        raise LineError(f"no creature with CombatId={combat_id}")

    def _hand(self):
        return list(prop(prop(self.pcs, "Hand"), "Cards"))

    def _run_action(self, action, timeout_s=15.0):
        call(self.aqs, "EnqueueAction", action, UInt64(int(str(prop(self.player, "NetId")))))
        call(self.ex, "Unpause")
        call(self.ex, "ExecuteActions")
        t0 = time.time()
        while time.time() - t0 < timeout_s:
            st = str(prop(action, "State"))
            if st in ("Finished", "Cancelled"):
                return st
            time.sleep(0.02)
        return f"timeout:{prop(action, 'State')}"

    def play(self, hand_index, target_id):
        hand = self._hand()
        if hand_index < 0 or hand_index >= len(hand):
            raise LineError(f"hand_index {hand_index} out of range "
                            f"(hand has {len(hand)} cards: {[str(c) for c in hand]})")
        card = hand[hand_index]
        target = self._creature_by_id(target_id)
        ctor = [c for c in find("PlayCardAction").GetConstructors(ALL)
                if not c.IsStatic and c.GetParameters().Length == 2][0]
        act = ctor.Invoke(Array[Object]([card, target]))
        result = {"card": str(card), "target_id": target_id, "state": self._run_action(act)}
        self._check_play_completed(str(card))
        return result

    def _check_play_completed(self, card_label):
        """Raise SwallowedPlayError if a play 'Finished' without finishing.

        Guard for the #339 class of failure: an exception inside the card's
        OnPlay task is logged-and-swallowed by TaskHelper.LogTaskExceptions,
        so the action reports Finished while the card's effects silently
        never happened. Signature: a CardPlayStartedEntry with no matching
        CardPlayFinishedEntry. Skipped when combat ended mid-play (a lethal
        play may legitimately not log its finish)."""
        if self.over:
            return
        started = finished = 0
        for e in prop(prop(self.cm, "History"), "Entries"):
            n = str(e.GetType().Name)
            if n == "CardPlayStartedEntry":
                started += 1
            elif n == "CardPlayFinishedEntry":
                finished += 1
        if started != finished:
            raise SwallowedPlayError(
                f"play of {card_label} reported Finished but CombatHistory has "
                f"{started} CardPlayStartedEntry vs {finished} CardPlayFinishedEntry "
                "-- the card's OnPlay task threw and the engine swallowed it "
                "(#339). Check the [GAME] log above for the real exception; the "
                "report would otherwise silently show zero effect.")

    def use_potion(self, potion_index, target_id):
        ctor = [c for c in find("UsePotionAction").GetConstructors(ALL)
                if not c.IsStatic and c.GetParameters().Length == 5][0]
        act = ctor.Invoke(Array[Object]([
            self.player, UInt32(potion_index),
            UInt32(target_id) if target_id is not None else None,
            None, True]))
        return {"potion_index": potion_index, "target_id": target_id,
                "state": self._run_action(act)}

    def end_turn(self, timeout_s=30.0):
        """Enqueue EndPlayerTurnAction and let the ENGINE drive its own pipeline
        (discard -> enemy turn -> next StartTurn deal). Never the manual RBET
        protocol -- that races the engine and skips the discard."""
        ept = [c for c in find("EndPlayerTurnAction").GetConstructors(ALL) if not c.IsStatic][0]
        st = self._run_action(ept.Invoke(Array[Object]([
            self.player, Int32(int(prop(self.pcs, "TurnNumber")))])))
        t0 = time.time()
        while time.time() - t0 < timeout_s and not self.over:
            side = str(prop(self.cs, "CurrentSide"))
            phase = str(prop(self.pcs, "Phase"))
            if side == "Player" and phase == "Play":
                break
            call(self.ex, "Unpause")
            call(self.ex, "ExecuteActions")
            time.sleep(0.05)
        return {"state": st, "over": self.over}

    def history(self):
        entries = []
        hist = prop(self.cm, "History")
        for e in prop(hist, "Entries"):
            et = e.GetType()
            actor = prop(e, "Actor")
            actor_label = None
            if actor is not None:
                mon = prop(actor, "Monster")
                actor_label = ("player" if mon is None else str(mon)) + \
                    f"#{int(prop(actor, 'CombatId'))}"
            try:
                text = (str(prop(e, "HumanReadableString") or "")
                        or str(prop(e, "Description") or ""))
            except Exception as exc:  # headless: some Descriptions NRE
                text = f"<unreadable: {type(exc).__name__}>"
            entries.append({
                "type": str(et.Name),
                "actor": actor_label,
                "round": int(prop(e, "RoundNumber")),
                "side": str(prop(e, "CurrentSide")),
                "text": text,
            })
        return entries


# --------------------------------------------------------------------------
# public entry point
# --------------------------------------------------------------------------
def run_experiment(character, ascension, deck, relics, potions, encounter, seed, line,
                   acts=("Glory",)):
    """Run one SINGLE-TURN experiment against the real engine; return a report.

    See the module docstring for the `line` grammar and the single-turn rule.
    Returns a dict: build version, initial/final state, per-action results with
    per-stream RNG counter deltas, net per-creature hp/block deltas, and the
    ordered CombatHistory.
    """
    _validate_line(line)  # fail fast BEFORE booting the CLR

    _boot_and_init()
    player = _make_player(character)
    _override_deck(player, deck)
    _grant_relics(player, relics)
    _grant_potions(player, potions)
    fight = _Fight(player, character, ascension, list(acts), encounter, seed)

    start_creatures = _creature_snapshot(fight.cs)
    start_rng = _rng_counters(fight.run)
    start_energy = int(prop(fight.pcs, "Energy"))
    opening_hand = [str(c) for c in fight._hand()]

    actions_report = []
    for action in line:
        kind = action[0]
        rng_before = _rng_counters(fight.run)
        cr_before = _creature_snapshot(fight.cs)
        energy_before = int(prop(fight.pcs, "Energy")) if not fight.over else None

        if kind == "play":
            result = fight.play(action[1], action[2])
        elif kind == "potion":
            result = fight.use_potion(action[1], action[2])
        elif kind == "end":
            result = fight.end_turn()
        else:  # unreachable (validated)
            raise LineError(kind)

        rng_after = _rng_counters(fight.run)
        cr_after = _creature_snapshot(fight.cs)
        rng_delta = {k: rng_after[k] - rng_before[k]
                     for k in RNG_STREAMS if rng_after[k] != rng_before[k]}
        hp_block_delta = {}
        for cid in cr_after:
            if cid in cr_before:
                dhp = cr_after[cid]["hp"] - cr_before[cid]["hp"]
                dblk = cr_after[cid]["block"] - cr_before[cid]["block"]
                if dhp or dblk:
                    hp_block_delta[cid] = {"label": cr_after[cid]["label"],
                                           "hp": dhp, "block": dblk}
        actions_report.append({
            "action": list(action),
            "result": result,
            "rng_counter_delta": rng_delta,
            "hp_block_delta": hp_block_delta,
            "energy_before": energy_before,
            "energy_after": int(prop(fight.pcs, "Energy")) if not fight.over else None,
        })

    final_creatures = _creature_snapshot(fight.cs)
    final_rng = _rng_counters(fight.run)
    net_creature_delta = {}
    for cid in final_creatures:
        if cid in start_creatures:
            net_creature_delta[cid] = {
                "label": final_creatures[cid]["label"],
                "hp": final_creatures[cid]["hp"] - start_creatures[cid]["hp"],
                "block": final_creatures[cid]["block"] - start_creatures[cid]["block"],
            }

    return {
        "build": build_version(),
        "cite": {"seed": seed, "line": [list(a) for a in line]},
        "character": character, "ascension": ascension, "encounter": encounter,
        "initial": {
            "creatures": start_creatures,
            "energy": start_energy,
            "hand": opening_hand,
            "rng_counters": start_rng,
        },
        "actions": actions_report,
        "final": {
            "over": fight.over,
            "creatures": final_creatures,
            "energy": int(prop(fight.pcs, "Energy")) if not fight.over else None,
            "turn": int(prop(fight.pcs, "TurnNumber")),
            "rng_counters": final_rng,
        },
        "net_creature_delta": net_creature_delta,
        "net_rng_delta": {k: final_rng[k] - start_rng[k]
                          for k in RNG_STREAMS if final_rng[k] != start_rng[k]},
        "combat_history": fight.history(),
    }


# --------------------------------------------------------------------------
# CLI: run a JSON spec file, or a small inline demo
# --------------------------------------------------------------------------
def _parse_line_json(raw):
    """JSON has no tuples; normalize [["play",5,1],["end"]] -> tuples."""
    return [tuple(a) for a in raw]


def main(argv=None):
    import argparse
    ap = argparse.ArgumentParser(
        description="Run a single-turn harness experiment (local-only; needs game+venv).")
    ap.add_argument("spec", nargs="?",
                    help="Path to a JSON spec file with keys: character, ascension, deck, "
                         "relics, potions, encounter, seed, line (and optional acts).")
    ap.add_argument("--demo", action="store_true",
                    help="Run the built-in Ironclad/Cultists demo instead of a spec file.")
    args = ap.parse_args(argv)

    if args.demo or not args.spec:
        spec = dict(
            character="Ironclad", ascension=0,
            deck=["CARD.STRIKE_IRONCLAD"] * 8, relics=[], potions=[],
            encounter="ENCOUNTER.CULTISTS_NORMAL", seed="TESTSEED",
            # hand reindexes after each play, so index 0 twice = two Strikes
            line=[("play", 0, 1), ("play", 0, 1), ("end",)],
        )
    else:
        raw = json.loads(Path(args.spec).read_text())
        raw["line"] = _parse_line_json(raw["line"])
        spec = raw

    report = run_experiment(**spec)
    print(json.dumps(report, indent=1), flush=True)
    return report


if __name__ == "__main__":
    main()
