"""Conformance runner (#279): replay a captured .mcr line through the REAL
engine from its node-entry save, and report the exit state for diffing
against live pins and (until #2827 deleted it) combat_sim.

The .mcr events are recordings of the engine's own Net* actions, so the
driver feeds them back nearly raw:
  NetPlayCardAction   -> PlayCardAction(player, NetCombatCard(idx), ModelId, targetId)
  NetUsePotionAction  -> UsePotionAction(player, potionIndex, targetId, targetPlayerId, True)
  NetEndPlayerTurnAction -> EndPlayerTurnAction + ReadyToBeginEnemyTurnAction
                            + the per-round driver protocol (SetupPlayerTurn, Play phase)

Usage:
    venv/bin/python conformance.py <entry.save> <capture.mcr> <ENCOUNTER.ID>
"""
import json
import sys
import time
from pathlib import Path

HARNESS = Path(__file__).parent
SOLVER = HARNESS.parent
sys.path.insert(0, str(SOLVER))
sys.path.insert(0, str(SOLVER / "tools"))

import host  # noqa: E402  (starts CoreCLR)
from host import T, call, call_static, prop, ALL  # noqa: E402
import mcr_parser  # noqa: E402

from System import Activator, Array, Object, Enum, UInt64, UInt32, Int32, Func  # noqa: E402
from System.Threading.Tasks import Task  # noqa: E402
import System  # noqa: E402

asm = host.asm


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
    return ms[0]


def boot_and_load_run(save_path):
    """Boot the headless engine and load a current_run.save into a wired RunState."""
    host.boot()
    MM = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
    MMState = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState")
    call_static(MM, "set_State", Enum.Parse(MMState, "Initialized"))
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    # v0.109 added an optional Type[] injectedModelTypes param (mod injection)
    init = [m for m in ModelDb.GetMethods(ALL) if str(m.Name) == "Init"][0]
    init.Invoke(None, Array[Object]([None] * init.GetParameters().Length))

    SR = find("SerializableRun")
    from_json = _method(T("MegaCrit.Sts2.Core.Saves.SaveManager"), "FromJson").MakeGenericMethod(SR)
    result = from_json.Invoke(None, Array[Object]([Path(save_path).read_text()]))
    assert str(prop(result, "Status")) == "Success", str(prop(result, "ErrorMessage"))
    run = _method(find("RunState"), "FromSerializable").Invoke(
        None, Array[Object]([prop(result, "SaveData")]))

    # SetUpTest minus InitializeNewRun (which is new-run-only: it repopulates
    # grab bags / rooms the loaded run already has)
    import time as _time
    RM = find("RunManager")
    rmi = call_static(RM, "get_Instance")
    svc = [c for c in find("NetSingleplayerGameService").GetConstructors(ALL)
           if not c.IsStatic][0].Invoke(Array[Object]([]))
    peer = [c for c in find("PeerInputSynchronizer").GetConstructors(ALL)
            if not c.IsStatic][0]
    peer = peer.Invoke(Array[Object]([svc] * peer.GetParameters().Length))
    _method(RM, "set_State").Invoke(rmi, Array[Object]([run]))
    init_shared = _method(RM, "InitializeShared")
    from System import Int64
    init_shared.Invoke(rmi, Array[Object](
        [svc, peer, False, None, Int64(int(_time.time())), Int64(0), Int64(0), Int32(0)]))
    host.initialize_run_lobby(rmi, svc, run)
    css = prop(rmi, "CombatStateSynchronizer")
    _method(css.GetType(), "set_IsDisabled").Invoke(css, Array[Object]([True]))
    if prop(rmi, "AscensionManager") is None:
        am = [c for c in find("AscensionManager").GetConstructors(ALL) if not c.IsStatic
              and str(c.GetParameters()[0].ParameterType.Name) == "Int32"][0].Invoke(
            Array[Object]([Int32(int(prop(run, "AscensionLevel")))]))
        _method(RM, "set_AscensionManager").Invoke(rmi, Array[Object]([am]))
    # the engine resolves "the local player" via this static (set at login
    # in-game); without it GetMe() is null and the end-turn pipeline dies
    LC = find("LocalContext")
    net_id = UInt64(int(str(prop(list(prop(run, "Players"))[0], "NetId"))))
    _method(LC, "set_NetId").Invoke(None, Array[Object]([net_id]))
    _restore_rng_counters(run, save_path)
    return run, rmi


# v18 -> v19 save migration (game v0.109) zeroes rng stream counters: the new
# schema serializes full xoshiro state, and the migration doesn't synthesize
# it. The algorithm itself is unchanged (MegaRandom = xoshiro256** +
# splitmix64, exactly sts2_rng.py's model), so advancing each stream by the
# fixture's recorded counter reproduces the exact v0.108 state.
_STREAM_KEYS = {
    "up_front": "UpFront", "shuffle": "Shuffle",
    "unknown_map_point": "UnknownMapPoint",
    "combat_card_generation": "CombatCardGeneration",
    "combat_potion_generation": "CombatPotionGeneration",
    "combat_card_selection": "CombatCardSelection",
    "combat_energy_costs": "CombatEnergyCosts",
    "combat_targets": "CombatTargets", "monster_ai": "MonsterAi",
    "niche": "Niche", "combat_orbs": "CombatOrbGeneration",
    "treasure_room_relics": "TreasureRoomRelics",
}


def _restore_rng_counters(run, save_path):
    saved = json.loads(Path(save_path).read_text()).get("rng", {}).get("counters", {})
    rng = prop(run, "Rng")
    for key, propname in _STREAM_KEYS.items():
        want = int(saved.get(key, 0))
        if want == 0:
            continue
        stream = prop(rng, propname)
        fld = stream.GetType().GetField("_counter", ALL)
        have = int(fld.GetValue(stream))
        draw = [m for m in stream.GetType().GetMethods(ALL)
                if str(m.Name) == "NextDouble" and m.GetParameters().Length == 0][0]
        for _ in range(want - have):
            draw.Invoke(stream, None)
        now = int(fld.GetValue(stream))
        if now != want:
            raise AssertionError(f"rng restore {propname}: got {now}, want {want}")
        print(f"  [rng] {propname} fast-forwarded to {now}", flush=True)


class Fight:
    """Headless combat driver for a loaded run + encounter."""

    def __init__(self, run, rmi, encounter_id):
        self.run, self.rmi = run, rmi
        ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
        mid = _method(find("ModelId"), "Deserialize").Invoke(
            None, Array[Object]([encounter_id]))
        get_by_id = [m for m in ModelDb.GetMethods(ALL)
                     if str(m.Name) == "GetById" and m.IsGenericMethod][0] \
            .MakeGenericMethod(find("EncounterModel"))
        enc = get_by_id.Invoke(None, Array[Object]([mid]))
        self.enc = call(enc, "MutableClone")
        self.player = list(prop(run, "Players"))[0]
        cs_ctor = [c for c in find("CombatState").GetConstructors(ALL) if not c.IsStatic][0]
        self.cs = cs_ctor.Invoke(Array[Object](
            [self.enc, run, prop(run, "Modifiers"), prop(run, "BadgeModels"),
             prop(run, "MultiplayerScalingModel")]))
        # creation order matters: .mcr target ids are creation-order CombatIds
        # — player creature is id 0, monsters 1..N
        call(self.cs, "AddPlayer", self.player)
        enemy = Enum.Parse(find("CombatSide"), "Enemy")
        call(self.enc, "GenerateMonstersWithSlots", run)
        for item in prop(self.enc, "MonstersWithSlots"):
            it = item.GetType()
            cr = call(self.cs, "CreateCreature", it.GetField("Item1").GetValue(item),
                      enemy, it.GetField("Item2").GetValue(item))
            if cr not in list(prop(self.cs, "Enemies")):
                call(self.cs, "AddCreature", cr)
        for c in prop(self.cs, "Creatures"):
            mon = prop(c, "Monster")
            print(f"  creature CombatId={prop(c, 'CombatId')} "
                  f"{'player' if mon is None else str(mon)}", flush=True)

        self.cm = call_static(find("CombatManager"), "get_Instance")
        call(self.cm, "SetUpCombat", self.cs)
        self._turn_loop_task = host.start_combat_internal(self.cm)

        self.pcs = prop(self.player, "PlayerCombatState")
        self.aqs = prop(rmi, "ActionQueueSynchronizer")
        self.ex = prop(rmi, "ActionExecutor")
        self._drain()
        # with LocalContext.NetId set, the engine's own StartTurn deals the
        # opening hand; wait for it instead of manual SetupPlayerTurn
        import time as _t
        t0 = _t.time()
        while _t.time() - t0 < 15:
            if str(prop(self.pcs, "Phase")) == "Play":
                break
            call(self.ex, "Unpause")
            call(self.ex, "ExecuteActions")
            _t.sleep(0.05)
        assert str(prop(self.pcs, "Phase")) == "Play", \
            f"opening turn never reached Play: {prop(self.pcs, 'Phase')}"

    # -- driver protocol ---------------------------------------------------
    def _drain(self):
        call(self.ex, "Unpause")
        call(self.ex, "ExecuteActions")
        t = call(self.ex, "FinishedExecutingActions")
        call(call(t, "GetAwaiter"), "GetResult")

    # (the old manual _setup_turn -- SetupPlayerTurn + SetPhaseForAllPlayers --
    # was removed as DEAD CODE in the #344 hardening pass: it had no callers
    # and is the banned protocol; see README. Never reintroduce it.)

    def _run_action(self, action, timeout_s=10.0):
        call(self.aqs, "EnqueueAction", action,
             UInt64(int(str(prop(self.player, "NetId")))))
        call(self.ex, "Unpause")
        call(self.ex, "ExecuteActions")
        t0 = time.time()
        while time.time() - t0 < timeout_s:
            st = str(prop(action, "State"))
            if st in ("Finished", "Cancelled"):
                return st
            time.sleep(0.02)
        return f"timeout:{prop(action, 'State')}"

    @property
    def over(self):
        return bool(prop(self.cm, "IsOverOrEnding")) or not bool(prop(self.cm, "IsInProgress"))

    # -- .mcr event feed ---------------------------------------------------
    def play_card(self, combat_card_index, card_id, target_id):
        NCC = find("NetCombatCard")
        ncc = Activator.CreateInstance(NCC)
        setter = [m for m in NCC.GetMethods(ALL) if str(m.Name) == "set_CombatCardIndex"]
        if setter:
            setter[0].Invoke(ncc, Array[Object]([UInt32(combat_card_index)]))
        else:
            NCC.GetFields(ALL)[0].SetValue(ncc, UInt32(combat_card_index))
        mid = _method(find("ModelId"), "Deserialize").Invoke(
            None, Array[Object](["CARD." + card_id]))
        ctor = [c for c in find("PlayCardAction").GetConstructors(ALL)
                if not c.IsStatic and c.GetParameters().Length == 4][0]
        act = ctor.Invoke(Array[Object](
            [self.player, ncc, mid,
             UInt32(target_id) if target_id else None]))
        return self._run_action(act)

    def use_potion(self, potion_index, target_id, target_player_id):
        ctor = [c for c in find("UsePotionAction").GetConstructors(ALL)
                if not c.IsStatic and c.GetParameters().Length == 5][0]
        act = ctor.Invoke(Array[Object](
            [self.player, UInt32(potion_index),
             UInt32(target_id) if target_id else None,
             UInt64(target_player_id) if target_player_id else None, True]))
        return self._run_action(act)

    def end_turn(self, timeout_s=30.0):
        """Enqueue the end-turn and let the ENGINE drive its own pipeline:
        WaitForActionThenEndTurn -> EndPlayerTurnPhaseOneInternal (hand
        discard) -> enemy-turn hook action -> StartTurn -> per-player deal.
        Driving RBET/SetupPlayerTurn manually races that pipeline and skips
        the discard (the hp/shuffle drift of the first runner version)."""
        ept = [c for c in find("EndPlayerTurnAction").GetConstructors(ALL) if not c.IsStatic][0]
        st = self._run_action(ept.Invoke(Array[Object](
            [self.player, Int32(int(prop(self.pcs, "TurnNumber")))])))
        t0 = time.time()
        last = None
        while time.time() - t0 < timeout_s and not self.over:
            phase = str(prop(self.pcs, "Phase"))
            side = str(prop(self.cs, "CurrentSide"))
            hand_n = len(list(prop(prop(self.pcs, "Hand"), "Cards")))
            snap = (phase, side, hand_n)
            if snap != last:
                print(f"    [pump] side={side} phase={phase} hand={hand_n}", flush=True)
                last = snap
            if side == "Player" and phase == "Play":
                return st
            call(self.ex, "Unpause")
            call(self.ex, "ExecuteActions")
            time.sleep(0.05)
        if not self.over:
            print("    [end_turn] engine pipeline stalled at "
                  f"side={prop(self.cs, 'CurrentSide')} phase={prop(self.pcs, 'Phase')}",
                  flush=True)
        return st

    # -- reporting ----------------------------------------------------------
    def report(self):
        pc = prop(self.player, "Creature")
        rng = prop(self.run, "Rng")

        def counter(name):
            stream = prop(rng, name)
            fld = stream.GetType().GetField("_counter", ALL)
            if fld is not None:
                return int(fld.GetValue(stream))
            for p in stream.GetType().GetProperties(ALL):
                if str(p.Name) in ("Counter", "Count", "DrawCount"):
                    return int(p.GetValue(stream))
        return {
            "over": self.over,
            "player_hp": int(prop(pc, "CurrentHp")),
            "player_block": int(prop(pc, "Block")),
            "turn": int(prop(self.pcs, "TurnNumber")),
            "enemies_hp": [int(prop(c, "CurrentHp")) for c in prop(self.cs, "Enemies")],
            "rng": {"shuffle": counter("Shuffle"), "niche": counter("Niche"),
                    "sel": counter("CombatCardSelection"), "ai": counter("MonsterAi")},
        }


def replay_mcr(save_path, mcr_path, encounter_id, verbose=True):
    run, rmi = boot_and_load_run(save_path)
    fight = Fight(run, rmi, encounter_id)
    events = mcr_parser.decode(mcr_path)["events"]
    for e in events:
        if fight.over:
            break
        a = e.get("action", {})
        t = a.get("type")
        if t == "NetPlayCardAction":
            st = fight.play_card(a["combat_card_index"], a["card_id"], a.get("target_id"))
            if verbose:
                print(f"  play {a['card_id']}#{a['combat_card_index']} -> {st}", flush=True)
        elif t == "NetUsePotionAction":
            st = fight.use_potion(a["potion_index"], a.get("target_id"),
                                  a.get("target_player_id"))
            if verbose:
                print(f"  potion #{a['potion_index']} -> {st}", flush=True)
        elif t == "NetEndPlayerTurnAction":
            st = fight.end_turn()
            if verbose:
                print(f"  end turn -> {st} (round {prop(fight.cs, 'RoundNumber')})", flush=True)
    rep = fight.report()
    if verbose:
        print(json.dumps(rep, indent=1), flush=True)
    return rep


if __name__ == "__main__":
    replay_mcr(sys.argv[1], sys.argv[2], sys.argv[3])
