"""Ground-truth oracle for Neow blessing options (#626).

Boots the real sts2.dll headlessly, builds an Ironclad run for each requested
seed, and calls the game's own `Neow::GenerateInitialOptions`. Prints one JSON
record per seed so `versions/v0.111.0/solver/neow.py` can be diffed against the engine.

    solver/harness/venv/bin/python solver/harness/probe_neow.py \
        LPTMBBTY7FQY AAAAAAAAAA ...

With no seeds it uses the built-in sweep, which is chosen to exercise the
branches a single capture cannot: a Large Capsule curse (skips the Lava Rock /
Small Capsule coin flip) and curses that trigger the positive-pool exclusions.

LOCAL-ONLY: needs the game install + harness venv/dotnet (see README).
sts2.dll cannot be redistributed, so this never runs in CI.
"""
import json
import sys
from pathlib import Path

HARNESS = Path(__file__).parent
sys.path.insert(0, str(HARNESS))
sys.path.insert(0, str(HARNESS.parent))

import host  # noqa: E402  (starts CoreCLR on import)
from host import ALL, call, call_static, prop  # noqa: E402

import System  # noqa: E402
from System import Activator, Array, Enum, Int32, Object, String, UInt64  # noqa: E402

asm = host.asm

# A default sweep. The first is the certified capture (2026-07-26,
# 084144.724_save_6ccd86c1.save); the rest are arbitrary strings picked only to
# land on different curses so the exclusion/skip branches get exercised.
DEFAULT_SEEDS = ("LPTMBBTY7FQY", "AAAAAAAAAA", "BBBBBBBBBB", "CCCCCCCCCC",
                 "DDDDDDDDDD", "EEEEEEEEEE", "FFFFFFFFFF", "GGGGGGGGGG",
                 "HHHHHHHHHH", "IIIIIIIIII", "JJJJJJJJJJ", "KKKKKKKKKK")

# The unlock state of the certified capture: every epoch the account has.
# Kaleidoscope needs all five character epochs; ScrollBoxes needs a card pool
# deep enough for CanGenerateBundles.
FULL_EPOCHS = (
    "COLORLESS1_EPOCH", "RELIC1_EPOCH", "COLORLESS2_EPOCH", "SILENT6_EPOCH",
    "CUSTOM_AND_SEEDS_EPOCH", "ACT3_B_EPOCH", "RELIC2_EPOCH",
    "COLORLESS3_EPOCH", "IRONCLAD4_EPOCH", "IRONCLAD3_EPOCH", "ACT2_B_EPOCH",
    "COLORLESS4_EPOCH", "RELIC4_EPOCH", "EVENT3_EPOCH", "IRONCLAD2_EPOCH",
    "COLORLESS5_EPOCH", "SILENT4_EPOCH", "SILENT5_EPOCH", "IRONCLAD5_EPOCH",
    "POTION2_EPOCH", "SILENT2_EPOCH", "OROBAS_EPOCH", "EVENT1_EPOCH",
    "RELIC5_EPOCH", "REGENT6_EPOCH", "RELIC3_EPOCH", "REGENT1_EPOCH",
    "SILENT3_EPOCH", "IRONCLAD6_EPOCH", "REGENT5_EPOCH", "POTION1_EPOCH",
    "NEOW_EPOCH", "NECROBINDER2_EPOCH", "SILENT1_EPOCH", "REGENT2_EPOCH",
    "EVENT2_EPOCH", "DEFECT1_EPOCH", "NECROBINDER3_EPOCH", "UNDERDOCKS_EPOCH",
    "REGENT3_EPOCH", "DEFECT2_EPOCH", "NECROBINDER4_EPOCH", "REGENT4_EPOCH",
    "NECROBINDER5_EPOCH", "NECROBINDER6_EPOCH", "DARV_EPOCH",
    "NECROBINDER1_EPOCH", "REGENT7_EPOCH", "IRONCLAD7_EPOCH", "SILENT7_EPOCH",
    "DAILY_RUN_EPOCH", "NECROBINDER7_EPOCH",
)


def find(name):
    hits = [t for t in asm.GetTypes() if str(t.Name) == name]
    if not hits:
        raise KeyError(name)
    return hits[0]


def glist(elem_t, items=()):
    lt = System.Type.GetType("System.Collections.Generic.List`1") \
        .MakeGenericType(Array[System.Type]([elem_t]))
    inst = Activator.CreateInstance(lt)
    add = lt.GetMethod("Add")
    for item in items:
        add.Invoke(inst, Array[Object]([item]))
    return inst


def meth(t, name, nargs=None):
    return [m for m in t.GetMethods(ALL) if str(m.Name) == name
            and (nargs is None or m.GetParameters().Length == nargs)][0]


def boot():
    host.boot()
    MM = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
    call_static(MM, "set_State", Enum.Parse(
        asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState"),
        "Initialized"))
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    init = [m for m in ModelDb.GetMethods(ALL) if str(m.Name) == "Init"][0]
    init.Invoke(None, Array[Object]([None] * init.GetParameters().Length))

    # Headless cosmetics. Option IDENTITY is the RelicModel and is untouched by
    # any of these; they only stop loc/texture lookups from NREing while
    # EventOption.FromRelic builds each option's display shell.
    host._patch_skip_named("LocString", "Exists")
    host._patch_skip_named("LocString", "GetIfExists")
    rl = host.godot_asm.GetType("Godot.ResourceLoader")
    for m in rl.GetMethods(host.ALL):
        if str(m.Name) == "Exists":
            host.patch_skip(rl, "Exists", m.ReturnType, "false",
                            m.GetParameters().Length)
    # AssetCache has generic overloads Harmony cannot wrap; patching the
    # non-generic ones is enough to keep texture loads off the Godot path.
    for name in ("GetCompressedTexture2D", "GetAsset", "LoadAsset"):
        try:
            host._patch_skip_named("AssetCache", name)
        except Exception:  # noqa: BLE001 - generic defs are expected to fail
            pass


def probe(seed_string, *, ascension=10, character="Ironclad",
          epochs=FULL_EPOCHS, number_of_runs=551):
    ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    US = find("UnlockState")
    us = [c for c in US.GetConstructors(ALL)
          if c.GetParameters().Length == 3][0].Invoke(Array[Object]([
              glist(System.String, [String(e) for e in epochs]),
              glist(find("ModelId"), []), Int32(number_of_runs)]))

    Player_t = find("Player")
    cfnr = [m for m in Player_t.GetMethods(ALL)
            if str(m.Name) == "CreateForNewRun"
            and m.GetParameters().Length == 3][0]
    player = cfnr.Invoke(None, Array[Object]([
        call_static(ModelDb, "Get", find(character)), us, UInt64(0)]))

    RunState_t = find("RunState")
    cft = [m for m in RunState_t.GetMethods(ALL)
           if str(m.Name) == "CreateForTest"][0]
    run = cft.Invoke(None, Array[Object]([
        glist(Player_t, [player]),
        glist(find("ActModel"), [call_static(ModelDb, "Get",
                                             find("Underdocks"))]),
        glist(find("ModifierModel"), []),
        Enum.Parse(find("GameMode"), "Standard"), Int32(ascension),
        String(seed_string)]))

    RM = find("RunManager")
    rmi = call_static(RM, "get_Instance")
    meth(RM, "set_State", 1).Invoke(rmi, Array[Object]([run]))
    meth(RM, "set_AscensionManager", 1).Invoke(rmi, Array[Object]([
        Activator.CreateInstance(find("AscensionManager"),
                                 Array[Object]([Int32(ascension)]))]))

    Neow_t = find("Neow")
    neow = call(call_static(ModelDb, "Get", Neow_t), "MutableClone")
    EventModel_t = find("EventModel")
    meth(EventModel_t, "set_Owner", 1).Invoke(neow, Array[Object]([player]))

    # Rebuild EventModel/<BeginEvent>d__28::MoveNext's Rng (RVA 0x366380)
    # from the engine's OWN primitives rather than from sts2_rng.py, so the
    # comparison downstream stays honest.
    run_seed = int(prop(prop(run, "Rng"), "Seed"))
    slot = int(call(run, "GetPlayerSlotIndex", player))
    entry = str(prop(prop(neow, "Id"), "Entry"))
    hashed = int(meth(find("StringHelper"), "GetDeterministicHashCode", 1)
                 .Invoke(None, Array[Object]([String(entry)])))
    event_seed = (run_seed + slot + hashed) % (1 << 64)

    Rng_t = find("Rng")
    rng_ctor = [c for c in Rng_t.GetConstructors(ALL)
                if c.GetParameters().Length == 1
                and str(c.GetParameters()[0].ParameterType)
                == "System.UInt64"][0]
    rng = rng_ctor.Invoke(Array[Object]([UInt64(event_seed)]))
    meth(EventModel_t, "set_Rng", 1).Invoke(neow, Array[Object]([rng]))

    options = [str(prop(prop(o, "Relic"), "Id"))
               for o in meth(Neow_t, "GenerateInitialOptions", 0)
               .Invoke(neow, None)]
    counter = int(Rng_t.GetField("_counter", ALL).GetValue(rng))

    sb = find("ScrollBoxes")
    return {
        "seed": seed_string,
        "run_seed": run_seed,
        "slot": slot,
        "entry": entry,
        "entry_hash": hashed,
        "event_seed": event_seed,
        "options": options,
        "draws": counter,
        "scroll_boxes_allowed": bool(
            call(call_static(ModelDb, "Get", sb), "IsAllowedAtNeow", player)),
        "kaleidoscope_allowed": bool(
            call(call_static(ModelDb, "Get", find("Kaleidoscope")),
                 "IsAllowedAtNeow", player)),
    }


def main():
    seeds = sys.argv[1:] or list(DEFAULT_SEEDS)
    boot()
    records = [probe(s) for s in seeds]
    print(json.dumps(records, indent=2))


if __name__ == "__main__":
    main()
