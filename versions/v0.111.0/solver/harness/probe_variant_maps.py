"""Ground-truth probe (#1868/#1869): real-engine SpoilsActMap and
Big Game Hunter regenerated maps.

For each seed: constructs a test RunState, then
  - spoils: `new SpoilsActMap(state, null)` with CurrentActIndex = 1
    (the exact object SpoilsMap::ModifyGeneratedMap installs), and
  - bgh: StandardActMap.CreateFor(state, false) for each act, passed
    through the REAL BigGameHunter relic model's ModifyGeneratedMap hook.
Dumps every resulting graph as JSON; the committed fixture
testdata/map_gen_variant_maps_v0111.json is produced by this probe and
pinned by test_map_gen.py.

Usage: cd versions/v0.111.0/solver/harness &&
       venv/bin/python probe_variant_maps.py out.json
"""
import json
import sys
from pathlib import Path

HARNESS = Path(__file__).resolve().parent
sys.path.insert(0, str(HARNESS))

import experiment as E  # noqa: E402
from experiment import find, generic_list, _method, ALL  # noqa: E402
from host import call, call_static, prop  # noqa: E402
from System import Array, Object, Enum, Int32, Boolean  # noqa: E402

E._boot_and_init()

SEEDS = [
    ("MAPTESTAAA", "underdocks", 0), ("MAPTESTBBB", "underdocks", 10),
    ("MAPTESTCCC", "overgrowth", 0), ("MAPTESTDDD", "overgrowth", 10),
    ("TQM88QFMHSQR", "underdocks", 10), ("ZPJHU3WSH2", "underdocks", 1),
    ("SEEDMAP111", "underdocks", 5), ("SEEDMAP222", "overgrowth", 15),
    ("HQPAXCBS6P", "overgrowth", 20), ("VARMAP0001", "underdocks", 0),
    ("VARMAP0002", "overgrowth", 1), ("VARMAP0003", "underdocks", 20),
]
ACT_SETS = {
    "underdocks": ["Underdocks", "Hive", "Glory"],
    "overgrowth": ["Overgrowth", "Hive", "Glory"],
}

Player_t = find("Player")
RunState_t = find("RunState")
RM = find("RunManager")
SAM = find("StandardActMap")
SpoilsAM = find("SpoilsActMap")
cft = [m for m in RunState_t.GetMethods(ALL)
       if str(m.Name) == "CreateForTest"][0]
create_for = [m for m in SAM.GetMethods(ALL)
              if str(m.Name) == "CreateFor"][0]
spoils_ctor = [c for c in SpoilsAM.GetConstructors(ALL) if not c.IsStatic][0]
ModelDb = E.asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
BGH = find("BigGameHunter")
bgh_hook = [m for m in BGH.GetMethods(ALL)
            if str(m.Name) == "ModifyGeneratedMap"][0]


def coord_of(p):
    c = p.GetType().GetField("coord", ALL).GetValue(p)
    t = c.GetType()
    return [int(t.GetField("col", ALL).GetValue(c)),
            int(t.GetField("row", ALL).GetValue(c))]


def dump_point(p):
    return {"coord": coord_of(p), "type": int(prop(p, "PointType")),
            "can_modify": bool(prop(p, "CanBeModified")),
            "children": [coord_of(c) for c in prop(p, "Children")]}


def dump_map(amap):
    out = {
        "points": [dump_point(p) for p in call(amap, "GetAllMapPoints")],
        "boss": dump_point(prop(amap, "BossMapPoint")),
        "start": dump_point(prop(amap, "StartingMapPoint")),
        "start_coords": [
            coord_of(p) for p in
            find("ActMap").GetField("startMapPoints", ALL).GetValue(amap)],
    }
    sb = prop(amap, "SecondBossMapPoint")
    if sb is not None:
        out["second_boss"] = dump_point(sb)
    return out


out = []
for seed, act_set, ascension in SEEDS:
    player = E._make_player("Ironclad")
    acts = ACT_SETS[act_set]
    act_models = generic_list(
        find("ActModel"),
        [call_static(ModelDb, "Get", find(a)) for a in acts])
    run = cft.Invoke(None, Array[Object]([
        generic_list(Player_t, [player]), act_models,
        generic_list(find("ModifierModel")),
        Enum.Parse(find("GameMode"), "Standard"),
        Int32(int(ascension)), seed]))
    rmi = call_static(RM, "get_Instance")
    _method(RM, "set_State").Invoke(rmi, Array[Object]([run]))
    am_ctor = [c for c in find("AscensionManager").GetConstructors(ALL)
               if not c.IsStatic
               and str(c.GetParameters()[0].ParameterType.Name) == "Int32"][0]
    _method(RM, "set_AscensionManager").Invoke(
        rmi, Array[Object]([am_ctor.Invoke(
            Array[Object]([Int32(int(ascension))]))]))

    # -- spoils: act index 1 only
    _method(RunState_t, "set_CurrentActIndex").Invoke(
        run, Array[Object]([Int32(1)]))
    smap = spoils_ctor.Invoke(Array[Object]([run, None]))
    d = dump_map(smap)
    d.update(seed=seed, kind="spoils", act_index=1,
             act_id=str(prop(prop(run, "Act"), "Id")), ascension=ascension)
    out.append(d)
    print(f"done {seed} spoils {len(d['points'])} pts", file=sys.stderr)

    # -- bgh: real relic hook over a fresh standard map, per act
    bgh_model = call_static(ModelDb, "Get", BGH)
    for idx in range(3):
        _method(RunState_t, "set_CurrentActIndex").Invoke(
            run, Array[Object]([Int32(idx)]))
        base = create_for.Invoke(None, Array[Object]([run, False]))
        regen = bgh_hook.Invoke(bgh_model, Array[Object](
            [run, base, Int32(idx)]))
        d = dump_map(regen)
        d.update(seed=seed, kind="bgh", act_index=idx,
                 act_id=str(prop(prop(run, "Act"), "Id")),
                 ascension=ascension)
        out.append(d)
        print(f"done {seed} bgh act{idx + 1} {len(d['points'])} pts",
              file=sys.stderr)

json.dump(out, open(sys.argv[1], "w"), separators=(",", ":"))
print(f"wrote {len(out)} maps", file=sys.stderr)
