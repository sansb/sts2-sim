"""Ground-truth probe (#1846): real-engine StandardActMap dumps.

Constructs a test RunState per seed, points CurrentActIndex at each act,
calls the exact fresh-map call site RunManager::GenerateMap uses —
StandardActMap.CreateFor(state, false) — and dumps every resulting graph
as JSON. The committed fixture
testdata/map_gen_engine_maps_v0111.json was produced by this probe on
build v0.111.0 (28 seeds x 3 acts, ascensions 0-20, both act-1 families);
map_gen.py must reproduce it byte-exactly (test_map_gen.py).

Usage (local-only, needs the game install + harness venv/dotnet):
    cd sim/v0.111.0/python/harness
    venv/bin/python probe_act_map.py out.json
"""
import json
import sys
from pathlib import Path

HARNESS = Path(__file__).resolve().parent
sys.path.insert(0, str(HARNESS))

import experiment as E  # noqa: E402  (boots CoreCLR via host)
from experiment import find, generic_list, _method, ALL  # noqa: E402
from host import call, call_static, prop  # noqa: E402
from System import Array, Object, Enum, Int32  # noqa: E402

E._boot_and_init()

ACT_SETS = {
    "underdocks": ["Underdocks", "Hive", "Glory"],
    "overgrowth": ["Overgrowth", "Hive", "Glory"],
}

SEEDS = [
    ("MAPTESTAAA", "underdocks", 0),
    ("MAPTESTBBB", "underdocks", 10),
    ("MAPTESTCCC", "overgrowth", 0),
    ("MAPTESTDDD", "overgrowth", 10),
    ("ZPJHU3WSH2", "underdocks", 1),
    ("HQPAXCBS6P", "overgrowth", 20),
    ("SEEDMAP111", "underdocks", 5),
    ("SEEDMAP222", "overgrowth", 15),
    ("HBRPOIG8F1", "underdocks", 0),
    ("CBFNO6B9M8", "overgrowth", 1),
    ("0O2RAK1VRJ", "underdocks", 5),
    ("NVGFYGWWQC", "overgrowth", 10),
    ("38HYF9SXME", "underdocks", 20),
    ("COSFOGYR3X", "overgrowth", 0),
    ("KXWNREK8PK", "underdocks", 1),
    ("3YR9OUDOCU", "overgrowth", 5),
    ("ZRENUN5Z3J", "underdocks", 10),
    ("QIP98Q1ZXO", "overgrowth", 20),
    ("I65FDHJK1E", "underdocks", 0),
    ("YY37Q9AH8R", "overgrowth", 1),
    ("VHS1K3AQ6L", "underdocks", 5),
    ("6GT6MJXK87", "overgrowth", 10),
    ("AU5BHXTPDP", "underdocks", 20),
    ("FF5E8II49K", "overgrowth", 0),
    ("Q71N8MTZX2", "underdocks", 1),
    ("72HPOEVB9O", "overgrowth", 5),
    ("OAEDOECVE6", "underdocks", 10),
    ("PR5N8I4P40", "overgrowth", 20),
]

Player_t = find("Player")
RunState_t = find("RunState")
RM = find("RunManager")
SAM = find("StandardActMap")
cft = [m for m in RunState_t.GetMethods(ALL)
       if str(m.Name) == "CreateForTest"][0]
create_for = [m for m in SAM.GetMethods(ALL)
              if str(m.Name) == "CreateFor"][0]
ModelDb = E.asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")


def coord_of(p):
    c = p.GetType().GetField("coord", ALL).GetValue(p)
    t = c.GetType()
    return [int(t.GetField("col", ALL).GetValue(c)),
            int(t.GetField("row", ALL).GetValue(c))]


def dump_point(p):
    return {
        "coord": coord_of(p),
        "type": int(prop(p, "PointType")),
        "can_modify": bool(prop(p, "CanBeModified")),
        "children": [coord_of(c) for c in prop(p, "Children")],
    }


out = []
for seed, act_set, ascension in SEEDS:
    player = E._make_player("Ironclad")
    acts = ACT_SETS[act_set]
    act_models = generic_list(
        find("ActModel"),
        [call_static(ModelDb, "Get", find(a)) for a in acts])
    run = cft.Invoke(None, Array[Object]([
        generic_list(Player_t, [player]),
        act_models,
        generic_list(find("ModifierModel")),
        Enum.Parse(find("GameMode"), "Standard"),
        Int32(int(ascension)), seed]))
    rmi = call_static(RM, "get_Instance")
    _method(RM, "set_State").Invoke(rmi, Array[Object]([run]))
    am_ctor = [c for c in find("AscensionManager").GetConstructors(ALL)
               if not c.IsStatic
               and str(c.GetParameters()[0].ParameterType.Name) == "Int32"][0]
    am = am_ctor.Invoke(Array[Object]([Int32(int(ascension))]))
    _method(RM, "set_AscensionManager").Invoke(rmi, Array[Object]([am]))
    for idx in range(3):
        _method(RunState_t, "set_CurrentActIndex").Invoke(
            run, Array[Object]([Int32(idx)]))
        amap = create_for.Invoke(None, Array[Object]([run, False]))
        pts = [dump_point(p) for p in call(amap, "GetAllMapPoints")]
        entry = {
            "seed": seed, "act_index": idx,
            "act_id": str(prop(prop(run, "Act"), "Id")),
            "ascension": ascension,
            "boss": dump_point(prop(amap, "BossMapPoint")),
            "start": dump_point(prop(amap, "StartingMapPoint")),
            "points": pts,
            "start_coords": [
                coord_of(p) for p in
                amap.GetType().GetField("startMapPoints", ALL).GetValue(amap)],
        }
        sb = prop(amap, "SecondBossMapPoint")
        if sb is not None:
            entry["second_boss"] = dump_point(sb)
        out.append(entry)
        print(f"done {seed} act{idx+1} {entry['act_id']} "
              f"{len(pts)} pts", file=sys.stderr, flush=True)

json.dump(out, open(sys.argv[1], "w"), separators=(",", ":"))
print(f"wrote {len(out)} maps", file=sys.stderr)
