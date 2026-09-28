"""Spike probe 8 (#278): state injection — load a real current_run.save into the engine."""
import sys
from pathlib import Path

import host
from host import T, call, call_static, prop, ALL
from System import Activator, Array, Object, Enum, UInt64, Int32, String
import System

SAVE = Path("/Users/seanbloomfield/Documents/StsHistoryViewer/.claude/worktrees/"
            "serene-hellman-af0679/versions/v0.111.0/solver/testdata/8DVXPWUWRY_eel_entry.save")

sm = host.boot()
asm = host.asm

def find(simple):
    for t in asm.GetTypes():
        if str(t.Name) == simple:
            return t
    raise KeyError(simple)

ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
MM = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
MMState = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState")
call_static(MM, "set_State", Enum.Parse(MMState, "Initialized"))
call_static(ModelDb, "Init")
print("[1] boot + ModelDb OK", flush=True)

# ---- deserialize via the game's own reader ----
SR = find("SerializableRun")
from_json = [m for m in T("MegaCrit.Sts2.Core.Saves.SaveManager").GetMethods(ALL)
             if str(m.Name) == "FromJson"][0].MakeGenericMethod(SR)
result = from_json.Invoke(None, Array[Object]([SAVE.read_text()]))
print("[2] FromJson result:", result.GetType(),
      {str(p.Name): str(p.GetValue(result))[:60] for p in result.GetType().GetProperties(ALL)},
      flush=True)
sr = prop(result, "SaveData")
print("[3] SerializableRun OK; schema:", prop(sr, "SchemaVersion") if any(str(p.Name)=='SchemaVersion' for p in sr.GetType().GetProperties(ALL)) else "?", flush=True)

# ---- engine RunState from the save ----
RS = find("RunState")
fs = [m for m in RS.GetMethods(ALL) if str(m.Name) == "FromSerializable"][0]
run = fs.Invoke(None, Array[Object]([sr]))
print("[4] RunState.FromSerializable OK", flush=True)

players = list(prop(run, "Players"))
pl = players[0]
pc = prop(pl, "Creature")
deck_cards = list(prop(prop(pl, "Deck"), "Cards"))
relics = list(prop(pl, "Relics"))
print("[5] player:", f"hp={prop(pc,'CurrentHp')}/{prop(pc,'MaxHp')}",
      f"gold={prop(pl,'Gold')}", f"deck={len(deck_cards)}", f"relics={len(relics)}", flush=True)
print("   deck:", sorted(str(c).split(" ")[0] for c in deck_cards), flush=True)
print("   relics:", [str(r).split(" ")[0] for r in relics], flush=True)
print("   potions:", [str(p2) for p2 in prop(pl, "Potions")], flush=True)
print("   act:", prop(run, "CurrentActIndex"), "floor:", prop(run, "ActFloor"),
      "coord:", prop(run, "CurrentMapCoord"), flush=True)
rng = prop(run, "Rng")
print("   rng seed:", prop(rng, "StringSeed"), flush=True)

# ---- the next room should be the Terror Eel elite ----
room = prop(run, "CurrentRoom")
print("[6] current room:", room, flush=True)
try:
    base = prop(run, "BaseRoom")
    print("   base room:", base, flush=True)
except Exception as e:
    print("   base room err:", repr(e)[:80], flush=True)
print("\nPROBE 8 STAGE 1 DONE", flush=True)

# ==== STAGE 2: gardeners entry — engine must roll monster HP [28, 31, 29, 30] ====
SAVE2 = SAVE.parent / "8DVXPWUWRY_gardeners_entry.save"
result2 = from_json.Invoke(None, Array[Object]([SAVE2.read_text()]))
sr2 = prop(result2, "SaveData")
run2 = fs.Invoke(None, Array[Object]([sr2]))
print("[7] gardeners run loaded", flush=True)

rng2 = prop(run2, "Rng")
counters = {}
for name in ("Shuffle", "Niche", "CombatCardSelection", "MonsterAi"):
    stream = prop(rng2, name)
    for p2 in stream.GetType().GetProperties(ALL):
        if str(p2.Name) in ("Counter", "Count", "DrawCount"):
            counters[name] = p2.GetValue(stream)
print("[8] engine rng counters:", counters, "(save says shuffle 110, niche 5, sel 2, ai 4)", flush=True)

RM = find("RunManager")
rmi = call_static(RM, "get_Instance")
[m for m in RM.GetMethods(ALL) if str(m.Name) == "set_State"][0].Invoke(rmi, Array[Object]([run2]))
AM = find("AscensionManager")
am = [c for c in AM.GetConstructors(ALL) if not c.IsStatic
      and str(c.GetParameters()[0].ParameterType.Name) == "Int32"][0].Invoke(
      Array[Object]([Int32(int(prop(run2, "AscensionLevel")))]))
[m for m in RM.GetMethods(ALL) if str(m.Name) == "set_AscensionManager"][0].Invoke(rmi, Array[Object]([am]))
print("   RunManager wired; IsInProgress:", prop(rmi, "IsInProgress"),
      "HasAscension(8):", [m for m in RM.GetMethods(ALL) if str(m.Name) == "HasAscension"][0].Invoke(rmi, Array[Object]([Int32(8)])), flush=True)
enc = call(call_static(ModelDb, "Get", find("PhantasmalGardenersElite")), "MutableClone") \
    if asm.GetType("MegaCrit.Sts2.Core.Models.Encounters.PhantasmalGardenersElite") or True else None
print("[9] encounter:", enc, flush=True)

CombatState_t = find("CombatState")

def generic_list(elem_t, items=()):
    lt = System.Type.GetType("System.Collections.Generic.List`1").MakeGenericType(elem_t)
    lst = Activator.CreateInstance(lt)
    add = lt.GetMethod("Add")
    for it in items:
        add.Invoke(lst, Array[Object]([it]))
    return lst

mods2 = prop(run2, "Modifiers")
badges2 = prop(run2, "BadgeModels")
mps2 = prop(run2, "MultiplayerScalingModel")
print("   run modifiers:", [str(m) for m in mods2] if mods2 else mods2,
      "badges:", [str(b) for b in badges2] if badges2 else badges2,
      "mpScaling:", mps2, flush=True)
cs2 = [c for c in CombatState_t.GetConstructors(ALL) if not c.IsStatic][0].Invoke(
    Array[Object]([enc, run2, mods2, badges2, mps2]))
pl2 = list(prop(run2, "Players"))[0]
call(cs2, "AddPlayer", pl2)
CombatSide = find("CombatSide")
enemy_side = Enum.Parse(CombatSide, "Enemy")
call(enc, "GenerateMonstersWithSlots", run2)
hps = []
for item in prop(enc, "MonstersWithSlots"):
    it = item.GetType()
    mon = it.GetField("Item1").GetValue(item)
    slot = it.GetField("Item2").GetValue(item)
    cr = call(cs2, "CreateCreature", mon, enemy_side, slot)
    if cr not in list(prop(cs2, "Enemies")):
        call(cs2, "AttachCreature", cr)
        call(cs2, "AddCreature", cr)
    hps.append(int(prop(cr, "CurrentHp")))
print("[10] ENGINE MONSTER HP:", hps, "— pinned live+sim: [28, 31, 29, 30]", flush=True)
print("MATCH!" if hps == [28, 31, 29, 30] else "MISMATCH", flush=True)
