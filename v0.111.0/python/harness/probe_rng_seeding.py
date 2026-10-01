"""Ground-truth probe for #309: v0.109 per-stream RNG seeding.

Asks the REAL engine (sts2.dll in bare CoreCLR) for:
  1. StringHelper.GetDeterministicHashCode(s) for stream names + seed strings
  2. StringHelper.GetDeterministicHashCodeOld(s) for the same
  3. RunRngSet("<seed>") fresh-boot per-stream state: first NextInt(100) draws
  4. RunRngSet("old<seed>") — the legacy-hash escape path
  5. PlayerRngSet(hash + slot) first draws

Output is JSON on stdout for diffing against the sts2_rng.py model.
Local-only dev tool; run with the harness venv python, cwd = this dir.
"""
import json

import host  # noqa: F401  (boots CoreCLR + shims)
from host import T, ALL

from System import Array, Object, UInt64, Int32, String

StringHelper = T("MegaCrit.Sts2.Core.Helpers.StringHelper")
RunRngSet = T("MegaCrit.Sts2.Core.Runs.RunRngSet")
PlayerRngSet = T("MegaCrit.Sts2.Core.Random.PlayerRngSet")
Rng = T("MegaCrit.Sts2.Core.Random.Rng")

release = json.load(open(
    str(host.GAME / "../release_info.json")))

def invoke_static(t, name, *args):
    ms = [m for m in t.GetMethods(ALL)
          if str(m.Name) == name and m.GetParameters().Length == len(args)]
    assert ms, f"{name} not found"
    return ms[0].Invoke(None, Array[Object](list(args)))

def hash_new(s):
    return int(invoke_static(StringHelper, "GetDeterministicHashCode", String(s)))

def hash_old(s):
    return int(invoke_static(StringHelper, "GetDeterministicHashCodeOld", String(s)))

STREAMS = ["up_front", "shuffle", "unknown_map_point", "combat_card_generation",
           "combat_potion_generation", "combat_card_selection",
           "combat_energy_costs", "combat_targets", "monster_ai", "niche",
           "combat_orbs", "treasure_room_relics"]
PLAYER_STREAMS = ["rewards", "shops", "transformations"]

SEEDS = ["TESTBATCH0", "ZPJHU3WSH2", "8DVXPWUWRY", "HQPAXCBS6P"]

out = {"build": release, "hash_new": {}, "hash_old": {}}

for s in STREAMS + PLAYER_STREAMS + SEEDS + ["", "a", "abc",
                                             "oldZPJHU3WSH2"]:
    out["hash_new"][s] = hash_new(s)
    out["hash_old"][s] = hash_old(s)

def rng_first_draws(rng, n=3):
    m = [mm for mm in Rng.GetMethods(ALL)
         if str(mm.Name) == "NextInt" and mm.GetParameters().Length == 1][0]
    return [int(m.Invoke(rng, Array[Object]([Int32(100)]))) for _ in range(n)]

def runset_draws(seed_string):
    ctor = RunRngSet.GetConstructors(ALL)[0]
    rs = ctor.Invoke(Array[Object]([String(seed_string)]))
    seed_prop = RunRngSet.GetProperty("Seed", ALL)
    res = {"set_seed": int(seed_prop.GetValue(rs)), "streams": {}}
    get_rng = [m for m in RunRngSet.GetMethods(ALL)
               if str(m.Name) == "GetRng"][0]
    enum_t = T("MegaCrit.Sts2.Core.Entities.Rngs.RunRngType")
    from System import Enum
    for i, name in enumerate(STREAMS):
        ev = Enum.ToObject(enum_t, i)
        rng = get_rng.Invoke(rs, Array[Object]([ev]))
        res["streams"][name] = rng_first_draws(rng)
    return res

out["run_sets"] = {s: runset_draws(s) for s in SEEDS}
out["run_sets"]["oldZPJHU3WSH2"] = runset_draws("oldZPJHU3WSH2")

def playerset_draws(seed_string, slot):
    ctor = PlayerRngSet.GetConstructors(ALL)[0]
    seed = UInt64((hash_new(seed_string) + slot) & 0xFFFFFFFFFFFFFFFF)
    ps = ctor.Invoke(Array[Object]([seed]))
    get_rng = [m for m in PlayerRngSet.GetMethods(ALL)
               if str(m.Name) == "GetRng"][0]
    enum_t = T("MegaCrit.Sts2.Core.Entities.Rngs.PlayerRngType")
    from System import Enum
    res = {}
    for i, name in enumerate(PLAYER_STREAMS):
        ev = Enum.ToObject(enum_t, i)
        rng = get_rng.Invoke(ps, Array[Object]([ev]))
        res[name] = rng_first_draws(rng)
    return res

out["player_sets"] = {f"{s}/slot{sl}": playerset_draws(s, sl)
                      for s in SEEDS[:2] for sl in (0, 1)}

print(json.dumps(out, indent=1))
