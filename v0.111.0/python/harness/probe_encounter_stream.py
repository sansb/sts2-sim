"""Ground-truth probe for #637: the PER-FIGHT Encounter stream's seed.

`EncounterModel::GenerateMonstersWithSlots` builds an ad-hoc `Rng` per
fight that is NOT one of the 12 persisted run streams, so — unlike
`live_coach.verify_stream_seeding` — its derivation cannot be checked
against a save. This asks the REAL engine instead. Per
(run seed, encounter, TotalFloor) it reports:

  1. RunState.Rng.Seed            the RunRngSet SET seed (UInt64)
  2. RunState.TotalFloor          the floor addend actually used
  3. EncounterModel.Id.Entry      the hashed string
  4. GetDeterministicHashCode(Entry) and ...Old(Entry)
  5. the encounter Rng's `_counter` and `_random`'s xoshiro quadruple
     right after GenerateMonstersWithSlots — enough to SOLVE for the
     seed the engine used rather than merely check one candidate
  6. the rolled roster in MonstersWithSlots (= add = slot) order

`RunState::get_TotalFloor` (v0.109.1 RVA 0x5134e) =
`Sum(MapPointHistory, act => act.Count)` over a
`List<List<MapPointHistoryEntry>>`, so a nonzero TotalFloor is produced by
appending one inner list of N entries. Without that the probe would only
ever measure floor 0 and the floor addend would stay inferred.

Output is JSON on stdout after an `@@JSON@@` marker; the vectors are pinned
in `sim/v0.111.0/python/test_encounter_stream_seeding.py`. Local-only dev tool (sts2.dll
cannot be redistributed) — run with the harness venv python, cwd = this
dir, and re-run it on every game version bump.
"""
import json
import sys
import time
from pathlib import Path

HARNESS = Path(__file__).resolve().parent
sys.path.insert(0, str(HARNESS.parent))
sys.path.insert(0, str(HARNESS))

import experiment as X  # noqa: E402  (starts CoreCLR on import)
import host  # noqa: E402
from host import ALL, call, call_static, prop  # noqa: E402

import System  # noqa: E402
from System import Array, Enum, Int32, Int64, Object, String  # noqa: E402

asm = host.asm
find = X.find
StringHelper = host.T("MegaCrit.Sts2.Core.Helpers.StringHelper")

SEEDS_AND_FLOORS = (("7XDBEBWZ1REL", 0), ("ZPJHU3WSH2", 0),
                    ("7XDBEBWZ1REL", 3), ("7XDBEBWZ1REL", 8),
                    ("ZPJHU3WSH2", 8), ("ZPJHU3WSH2", 30),
                    ("HQPAXCBS6P", 1))

# Every encounter whose GenerateMonsters draws from the Encounter stream.
ENCOUNTERS = ("ENCOUNTER.SLIMES_WEAK", "ENCOUNTER.BOWLBUGS_WEAK",
              "ENCOUNTER.TWO_TAILED_RATS_NORMAL", "ENCOUNTER.SLIMES_NORMAL",
              "ENCOUNTER.SCROLLS_OF_BITING_NORMAL",
              "ENCOUNTER.DECIMILLIPEDE_ELITE")


def invoke_static(t, name, *args):
    ms = [m for m in t.GetMethods(ALL)
          if str(m.Name) == name and m.GetParameters().Length == len(args)]
    assert ms, f"{name} not found"
    return ms[0].Invoke(None, Array[Object](list(args)))


def hash_new(s):
    return int(invoke_static(StringHelper, "GetDeterministicHashCode",
                             String(s)))


def hash_old(s):
    return int(invoke_static(StringHelper, "GetDeterministicHashCodeOld",
                             String(s)))


def _fieldinfo(t, name):
    """GetField does not see PRIVATE fields of base types -- walk up."""
    while t is not None:
        f = t.GetField(name, ALL)
        if f is not None:
            return f
        t = t.BaseType
    return None


def _field(obj, name):
    f = _fieldinfo(obj.GetType(), name)
    if f is None:
        raise KeyError(f"no field {name} on {obj.GetType().FullName}")
    return f.GetValue(obj)


def _all_fields(t):
    out = []
    while t is not None:
        out.extend(t.GetFields(ALL))
        t = t.BaseType
    return out


def _list_add(lst, item):
    lst.GetType().GetMethod("Add").Invoke(lst, Array[Object]([item]))


def make_run(seed, ascension=10, acts=("Glory",)):
    """A fresh single-player run, wired like experiment._Fight but stopping
    short of any CombatState (GenerateMonstersWithSlots needs only the run
    plus RunManager.AscensionManager for the HP ranges)."""
    player = X._make_player("Ironclad")
    Player_t = find("Player")
    RunState_t = find("RunState")
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    cft = [m for m in RunState_t.GetMethods(ALL)
           if str(m.Name) == "CreateForTest"][0]
    act_models = X.generic_list(
        find("ActModel"),
        [call_static(ModelDb, "Get", find(a)) for a in acts])
    run = cft.Invoke(None, Array[Object]([
        X.generic_list(Player_t, [player]),
        act_models,
        X.generic_list(find("ModifierModel")),
        Enum.Parse(find("GameMode"), "Standard"),
        Int32(int(ascension)), seed]))

    RM = find("RunManager")
    rmi = call_static(RM, "get_Instance")
    svc = [c for c in find("NetSingleplayerGameService").GetConstructors(ALL)
           if not c.IsStatic][0].Invoke(Array[Object]([]))
    peer_ctor = [c for c in find("PeerInputSynchronizer").GetConstructors(ALL)
                 if not c.IsStatic][0]
    peer = peer_ctor.Invoke(
        Array[Object]([svc] * peer_ctor.GetParameters().Length))
    X._method(RM, "set_State").Invoke(rmi, Array[Object]([run]))
    X._method(RM, "InitializeShared").Invoke(rmi, Array[Object](
        [svc, peer, False, None, Int64(int(time.time())), Int64(0), Int64(0),
         Int32(0)]))
    X.host.initialize_run_lobby(rmi, svc, run)
    css = prop(rmi, "CombatStateSynchronizer")
    X._method(css.GetType(), "set_IsDisabled").Invoke(
        css, Array[Object]([True]))
    if prop(rmi, "AscensionManager") is None:
        am_ctor = [c for c in find("AscensionManager").GetConstructors(ALL)
                   if not c.IsStatic
                   and str(c.GetParameters()[0].ParameterType.Name) == "Int32"
                   ][0]
        am = am_ctor.Invoke(Array[Object]([Int32(int(ascension))]))
        X._method(RM, "set_AscensionManager").Invoke(rmi, Array[Object]([am]))
    return run


def set_total_floor(run, n):
    """TotalFloor = Sum(MapPointHistory, act => act.Count) and
    `_mapPointHistory` is a List<List<MapPointHistoryEntry>> (one inner list
    per act), so appending one inner list of n entries gives TotalFloor n."""
    if n == 0:
        return int(prop(run, "TotalFloor"))
    fld = _fieldinfo(run.GetType(), "_mapPointHistory")
    hist = fld.GetValue(run)
    inner_t = hist.GetType().GetGenericArguments()[0]
    entry_t = inner_t.GetGenericArguments()[0]
    inner = System.Activator.CreateInstance(inner_t)
    ctor = sorted((c for c in entry_t.GetConstructors(ALL) if not c.IsStatic),
                  key=lambda c: c.GetParameters().Length)[0]
    for _ in range(n):
        _list_add(inner, ctor.Invoke(
            Array[Object]([None] * ctor.GetParameters().Length)))
    _list_add(hist, inner)
    return int(prop(run, "TotalFloor"))


def probe(run, encounter_id):
    enc = call(X._model_by_id(encounter_id, "EncounterModel"), "MutableClone")
    entry = str(prop(prop(enc, "Id"), "Entry"))
    total_floor = int(prop(run, "TotalFloor"))
    set_seed = int(prop(prop(run, "Rng"), "Seed"))
    call(enc, "GenerateMonstersWithSlots", run)
    rng = _field(enc, "_rng")
    mr = _field(rng, "_random")
    state = {str(f.Name): int(f.GetValue(mr))
             for f in _all_fields(mr.GetType())
             if not f.IsStatic and str(f.FieldType.Name) == "UInt64"}
    roster = []
    for item in prop(enc, "MonstersWithSlots"):
        it = item.GetType()
        roster.append({
            "monster": str(prop(it.GetField("Item1").GetValue(item), "Id")),
            "slot": str(it.GetField("Item2").GetValue(item)),
        })
    return {
        "encounter_id": encounter_id,
        "entry": entry,
        "total_floor": total_floor,
        "set_seed": set_seed,
        "hash_new_entry": hash_new(entry),
        "hash_old_entry": hash_old(entry),
        "enc_rng_counter": int(_field(rng, "_counter")),
        "enc_rng_state": state,
        "roster": roster,
    }


def main():
    X._boot_and_init()
    out = {"build": json.load(open(str(host.GAME / "../release_info.json"))),
           "probes": []}
    for seed, floor in SEEDS_AND_FLOORS:
        run = make_run(seed)
        got = set_total_floor(run, floor)
        assert got == floor, f"TotalFloor {got} != requested {floor}"
        for encounter_id in ENCOUNTERS:
            out["probes"].append(dict(seed=seed, **probe(run, encounter_id)))
    print("@@JSON@@")
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
