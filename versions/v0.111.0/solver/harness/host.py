"""STS2 headless host: loads sts2.dll in bare CoreCLR with a Godot shim.

Bootstrap order matters:
  1. start CoreCLR (game's target: net9.0)
  2. load sts2.dll + resolve deps from the game dir
  3. TestMode.TurnOnInternal()  -- MegaCrit's own headless switch
  4. Harmony-patch the GodotSharp natives that game code touches headlessly
  5. SaveManager(MockGodotFileIo) + MockInstanceForTesting
"""
import json
import os
from pathlib import Path

SPIKE = Path(__file__).parent
# STS2_GAME_DIR points at an alternate data_sts2_macos_arm64 dir (e.g. a
# solver/dll-archive/<version>/ snapshot) so probes can load archived builds;
# default is the live Steam install.
GAME = Path(os.environ["STS2_GAME_DIR"]) if os.environ.get("STS2_GAME_DIR") \
    else Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                        "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                        "data_sts2_macos_arm64")

# The runtimeconfig is static (game targets net9.0) but gitignored, so a fresh
# clone/worktree lacks it and CoreCLR fails with InvalidConfigFile (0x80008093).
# Write it on import if missing (#329).
_RUNTIMECONFIG = SPIKE / "spike.runtimeconfig.json"
if not _RUNTIMECONFIG.exists():
    _RUNTIMECONFIG.write_text(json.dumps({"runtimeOptions": {
        "tfm": "net9.0",
        "framework": {"name": "Microsoft.NETCore.App", "version": "9.0.0"},
        "rollForward": "LatestPatch",
    }}, indent=2) + "\n")

from pythonnet import set_runtime
from clr_loader import get_coreclr
set_runtime(get_coreclr(runtime_config=str(SPIKE / "spike.runtimeconfig.json"),
                        dotnet_root=str(SPIKE / "dotnet")))
import clr  # noqa: F401
from System.Reflection import Assembly, BindingFlags
from System.Runtime.Loader import AssemblyLoadContext
from System import Activator, Array, Object, Type, String, Boolean

def _resolving(ctx, name):
    p = GAME / (name.Name + ".dll")
    return ctx.LoadFromAssemblyPath(str(p)) if p.exists() else None

AssemblyLoadContext.Default.Resolving += _resolving
asm = Assembly.LoadFrom(str(GAME / "sts2.dll"))
godot_asm = Assembly.LoadFrom(str(GAME / "GodotSharp.dll"))
harmony_asm = Assembly.LoadFrom(str(GAME / "0Harmony.dll"))

ALL = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance


def T(name, from_asm=None):
    t = (from_asm or asm).GetType(name)
    if t is None:
        for a in (asm, godot_asm):
            for tt in a.GetTypes():
                if str(tt.FullName) == name or str(tt.Name) == name:
                    return tt
    assert t is not None, f"type not found: {name}"
    return t


def method(t, name, nargs=None):
    ms = [m for m in t.GetMethods(ALL) if str(m.Name) == name
          and (nargs is None or m.GetParameters().Length == nargs)]
    assert ms, f"no method {name} on {t}"
    return ms[0]


def call_static(t, name, *args):
    m = method(t, name, len(args))
    return m.Invoke(None, Array[Object](list(args)) if args else None)


def call(obj, name, *args):
    m = method(obj.GetType(), name, len(args))
    return m.Invoke(obj, Array[Object](list(args)) if args else None)


def prop(obj, name):
    t = obj.GetType()
    while t is not None:
        p = t.GetProperty(name, ALL)
        if p is not None:
            return p.GetValue(obj)
        t = t.BaseType
    raise AttributeError(name)


def initialize_run_lobby(run_manager, service, run):
    """Invoke the single-player lobby seam across the v0.109/v0.110 ABI.

    v0.110.1 added an optional initial-player sequence used only by the
    multiplayer branch.  A headless single-player service takes the same
    path as before and must pass null for that third argument.

    Current-build cite: v0.110.1/db5d3552 InitializeRunLobby RVA 0x50464,
    parameters (INetGameService, RunState, IEnumerable<RunLobbyPlayer>).
    IL_000d-0017 tests service.Type.IsMultiplayer; the false branch jumps to
    IL_0045 without reading arg3. The only arg3 read is multiplayer-only at
    IL_0023, so null is exact on the headless single-player path.
    """
    candidate = method(run_manager.GetType(), "InitializeRunLobby")
    nargs = candidate.GetParameters().Length
    assert nargs in (2, 3), f"unknown InitializeRunLobby arity: {nargs}"
    args = [service, run]
    if nargs == 3:
        args.append(None)
    return candidate.Invoke(run_manager, Array[Object](args))


def start_combat_internal(combat_manager):
    """Start combat across the one-shot v0.109 and turn-loop v0.110 ABIs.

    v0.109's task completes once the opening turn is ready, so this helper
    waits for it.  v0.110's task *is the combat-long turn loop* and remains
    pending while the player chooses actions; return that task to keep alive.

    Current-build cites (v0.110.1/db5d3552): SetUpCombat RVA 0x139268
    constructs CombatTurnState at IL_001f-0025 and stores `_turnState` at
    IL_0026-0028. StartCombatInternal RVA 0x139510 takes that exact
    CombatTurnState (wrapper capture IL_0020-0023). Its nested
    `<StartCombatInternal>d__96::MoveNext` RVA 0x3f4234 awaits turn ends and
    switches sides, then loops while turnState.IsLive at IL_05da-05e5; the
    task sets its result only after that loop at IL_060c-0621.
    """
    candidate = method(combat_manager.GetType(), "StartCombatInternal")
    nargs = candidate.GetParameters().Length
    assert nargs in (0, 1), f"unknown StartCombatInternal arity: {nargs}"
    if nargs == 0:
        task = candidate.Invoke(combat_manager, None)
        task.GetAwaiter().GetResult()
        return None
    owner = combat_manager.GetType()
    turn_state = None
    while owner is not None:
        field = owner.GetField("_turnState", ALL)
        if field is not None:
            turn_state = field.GetValue(combat_manager)
            break
        owner = owner.BaseType
    assert turn_state is not None, "SetUpCombat did not create _turnState"
    return candidate.Invoke(combat_manager, Array[Object]([turn_state]))


# ---------------------------------------------------------------- Godot shim
from System.Reflection.Emit import (AssemblyBuilder, AssemblyBuilderAccess,
                                    OpCodes)
from System.Reflection import (AssemblyName, MethodAttributes,
                               CallingConventions, ParameterAttributes)
PARAM_NONE = getattr(ParameterAttributes, "None")

_ab = AssemblyBuilder.DefineDynamicAssembly(AssemblyName("GodotShim"),
                                            AssemblyBuilderAccess.Run)
_mod = _ab.DefineDynamicModule("GodotShim")
_shim_n = 0


def _emit_skip_prefix(result_type=None, fill=None, uninit_type=None):
    """Build `static bool Prefix([ref T __result])` returning false (skip original)."""
    global _shim_n
    _shim_n += 1
    tb = _mod.DefineType(f"Shim{_shim_n}")
    if result_type is None:
        ptypes = Array[Type]([])
    else:
        ptypes = Array[Type]([result_type.MakeByRefType()])
    mb = tb.DefineMethod("Prefix",
                         MethodAttributes.Public | MethodAttributes.Static,
                         CallingConventions.Standard,
                         Boolean, ptypes)
    if result_type is not None:
        mb.DefineParameter(1, PARAM_NONE, "__result")
    il = mb.GetILGenerator()
    if result_type is not None:
        il.Emit(OpCodes.Ldarg_0)
        if fill == "empty_string_array":
            il.Emit(OpCodes.Ldc_I4_0)
            il.Emit.Overloads[OpCodes.Newarr.GetType(), Type](OpCodes.Newarr, clr.GetClrType(String))
            il.Emit(OpCodes.Stind_Ref)
        elif fill == "empty_string":
            il.Emit(OpCodes.Ldstr, "")
            il.Emit(OpCodes.Stind_Ref)
        elif isinstance(fill, tuple) and fill[:1] == ("literal_string",):
            il.Emit(OpCodes.Ldstr, fill[1])
            il.Emit(OpCodes.Stind_Ref)
        elif fill == "false":
            il.Emit(OpCodes.Ldc_I4_0)
            il.Emit(OpCodes.Stind_I1)
        elif fill == "zero_i4":
            il.Emit(OpCodes.Ldc_I4_0)
            il.Emit(OpCodes.Stind_I4)
        elif fill == "zero_i8":
            il.Emit(OpCodes.Ldc_I4_0)
            il.Emit(OpCodes.Conv_I8)
            il.Emit(OpCodes.Stind_I8)
        elif fill == "completed_task":
            from System.Threading.Tasks import Task as _Task
            get_ct = clr.GetClrType(_Task).GetMethod("get_CompletedTask")
            il.EmitCall(OpCodes.Call, get_ct, None)
            il.Emit(OpCodes.Stind_Ref)
        elif fill == "uninit":
            # __result = RuntimeHelpers.GetUninitializedObject(typeof(T)) —
            # a managed shell with no native side; only safe when callers
            # never invoke natives on it (they get patched too).
            from System.Runtime.CompilerServices import RuntimeHelpers as _RH
            il.Emit.Overloads[OpCodes.Ldtoken.GetType(), Type](OpCodes.Ldtoken, uninit_type or result_type)
            gtfh = clr.GetClrType(Type).GetMethod("GetTypeFromHandle")
            il.EmitCall(OpCodes.Call, gtfh, None)
            guo = clr.GetClrType(_RH).GetMethod("GetUninitializedObject")
            il.EmitCall(OpCodes.Call, guo, None)
            il.Emit.Overloads[OpCodes.Castclass.GetType(), Type](OpCodes.Castclass, uninit_type or result_type)
            il.Emit(OpCodes.Stind_Ref)
        elif fill == "big_r8":
            il.Emit.Overloads[OpCodes.Ldc_R8.GetType(), System.Double](OpCodes.Ldc_R8, 1e9)
            il.Emit(OpCodes.Stind_R8)
        elif fill == "null":
            il.Emit(OpCodes.Ldnull)
            il.Emit(OpCodes.Stind_Ref)
        else:
            raise ValueError(fill)
    il.Emit(OpCodes.Ldc_I4_0)   # return false -> skip original
    il.Emit(OpCodes.Ret)
    t = tb.CreateType()
    return t.GetMethod("Prefix")


Harmony = harmony_asm.GetType("HarmonyLib.Harmony")
HarmonyMethod = harmony_asm.GetType("HarmonyLib.HarmonyMethod")
_h = Activator.CreateInstance(Harmony, Array[Object](["sts2-headless-shim"]))
_patch = [m for m in Harmony.GetMethods(ALL)
          if str(m.Name) == "Patch" and m.GetParameters().Length == 5][0]


def patch_skip(t, method_name, result_type=None, fill=None, nargs=None):
    orig = method(t, method_name, nargs)
    pre = _emit_skip_prefix(result_type, fill)
    hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
    _patch.Invoke(_h, Array[Object]([orig, hm, None, None, None]))


import System
_string_array_t = System.Type.GetType("System.String[]")


def install_shim():
    GD = godot_asm.GetType("Godot.GD")
    OS = godot_asm.GetType("Godot.OS")
    for name in ("Print", "PrintErr", "PrintRich", "PushError", "PushWarning", "PrintRaw"):
        for m in GD.GetMethods(ALL):
            if str(m.Name) == name and m.DeclaringType == GD:
                pre = _emit_skip_prefix()
                hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
                _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))
    fills = {
        "GetCmdlineArgs": (_string_array_t, "empty_string_array"),
        "HasFeature": (clr.GetClrType(Boolean), "false"),
        "GetUserDataDir": (clr.GetClrType(String), "empty_string"),
    }
    for m in OS.GetMethods(ALL):
        n = str(m.Name)
        if n in fills:
            rt, fill = fills[n]
            pre = _emit_skip_prefix(rt, fill)
            hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
            _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))
    # StringName/NodePath intern strings in the engine; headless -> null native call.
    for tn in ("Godot.StringName", "Godot.NodePath"):
        SN = godot_asm.GetType(tn)
        for m in SN.GetMethods(ALL):
            if str(m.Name) == "op_Implicit":
                rt = m.ReturnType
                fill = "empty_string" if str(rt.Name) == "String" else "null"
                pre = _emit_skip_prefix(rt, fill)
                hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
                _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))
        for c in SN.GetConstructors(ALL):
            if c.IsStatic:
                continue
            pre = _emit_skip_prefix()
            hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
            _patch.Invoke(_h, Array[Object]([c, hm, None, None, None]))


def _emit_console_print_prefix():
    """static bool Prefix(string text) { Console.WriteLine(text); return false; }"""
    global _shim_n
    _shim_n += 1
    tb = _mod.DefineType(f"Shim{_shim_n}")
    mb = tb.DefineMethod("Prefix",
                         MethodAttributes.Public | MethodAttributes.Static,
                         CallingConventions.Standard,
                         Boolean, Array[Type]([clr.GetClrType(String)]))
    mb.DefineParameter(1, PARAM_NONE, "text")
    il = mb.GetILGenerator()
    console_wl = clr.GetClrType(System.Console).GetMethod(
        "WriteLine", Array[Type]([clr.GetClrType(String)]))
    il.Emit(OpCodes.Ldstr, "[GAME] ")
    il.Emit(OpCodes.Ldarg_0)
    concat = clr.GetClrType(String).GetMethod(
        "Concat", Array[Type]([clr.GetClrType(String), clr.GetClrType(String)]))
    il.EmitCall(OpCodes.Call, concat, None)
    il.EmitCall(OpCodes.Call, console_wl, None)
    il.Emit(OpCodes.Ldc_I4_0)
    il.Emit(OpCodes.Ret)
    t = tb.CreateType()
    return t.GetMethod("Prefix")


def _auto_fill(rt):
    n = str(rt.Name)
    if n == "Void":
        return None, None
    if n == "Task":
        return rt, "completed_task"
    if n == "String":
        return rt, "empty_string"
    if n == "Boolean":
        return rt, "false"
    if n == "Int32":
        return rt, "zero_i4"
    if n in ("UInt64", "Int64"):
        return rt, "zero_i8"
    return rt, "null"  # reference types


def _patch_skip_named(type_simple_name, method_name, rt=None, fill=None):
    T2 = None
    for t in asm.GetTypes():
        if str(t.Name) == type_simple_name:
            T2 = t
    for m in T2.GetMethods(ALL):
        if m.DeclaringType == T2 and str(m.Name) == method_name:
            mrt, mfill = (rt, fill) if rt is not None else _auto_fill(m.ReturnType)
            pre = _emit_skip_prefix(mrt, mfill)
            hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
            _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))


def _patch_skip_constructor(type_simple_name, nargs=0):
    """Skip a headless-unsafe instance constructor with an exact arity."""
    t = T(type_simple_name)
    ctors = [c for c in t.GetConstructors(ALL)
             if not c.IsStatic and c.GetParameters().Length == nargs]
    assert len(ctors) == 1, \
        f"expected one {type_simple_name} constructor/{nargs}, got {len(ctors)}"
    pre = _emit_skip_prefix()
    hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
    _patch.Invoke(_h, Array[Object]([ctors[0], hm, None, None, None]))


def install_game_shims():
    """Game-level methods whose bodies need engine state (timing/vfx juice)."""
    from System.Threading.Tasks import Task as _Task
    from System import UInt64 as _U64
    _patch_skip_named("NHitStop", "HitStopTask", clr.GetClrType(_Task), "completed_task")
    _patch_skip_named("PeerInputSynchronizer", "GetTicksMsec", clr.GetClrType(_U64), "zero_i8")
    _patch_skip_named("TalkCmd", "Play")
    _patch_skip_named("ThinkCmd", "Play")
    # v0.111.0: NetSingleplayerGameService constructs PeerVersionInfo.LocalDefault,
    # which initializes PlatformUtil before asking for the release version.
    # Both platform-strategy constructors enter native Godot/Steam state that
    # does not exist in bare CoreCLR.  LocalDefault only needs the null
    # strategy's constant PlatformBranch.None; skip both native constructors
    # so PlatformUtil's managed cctor can install harmless strategy shells.
    _patch_skip_constructor("NullPlatformUtilStrategy")
    _patch_skip_constructor("SteamPlatformUtilStrategy")
    # NGame.GetGameVersion otherwise enters ReleaseInfoManager/Godot.OS to
    # locate release_info.json and dereferences ClassDB.  The harness already
    # loads that exact release file for every report, so return its version
    # through this otherwise engine-only path.
    release = json.loads((GAME.parent / "release_info.json").read_text())
    _patch_skip_named(
        "NGame", "GetGameVersion", clr.GetClrType(String),
        ("literal_string", release["version"]))
    # v0.109: OnPlayWrapper queries the table for the visual card (multi-play
    # anim); scene lookups have no headless guard
    _patch_skip_named("NCard", "FindOnTable")
    GTime = godot_asm.GetType("Godot.Time")
    for m in GTime.GetMethods(ALL):
        if str(m.Name) == "GetTicksMsec":
            pre = _emit_skip_prefix(clr.GetClrType(_U64), "zero_i8")
            hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
            _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))
    # Frame-wait pacing paths (CardPileCmd.Shuffle) do
    # Engine.GetMainLoop().Root.GetProcessDeltaTime() with no headless guard.
    # Serve an uninitialized SceneTree/Window shell and a huge delta so the
    # "wait this frame?" comparison always says no.
    def _patch_one(t, name, rt, fill, uninit_type=None):
        for m in t.GetMethods(ALL):
            if str(m.Name) == name and m.DeclaringType == t:
                pre = _emit_skip_prefix(rt, fill, uninit_type)
                hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
                _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))

    GEngine = godot_asm.GetType("Godot.Engine")
    STree = godot_asm.GetType("Godot.SceneTree")
    GWindow = godot_asm.GetType("Godot.Window")
    GNode = godot_asm.GetType("Godot.Node")
    _patch_one(GEngine, "GetMainLoop", godot_asm.GetType("Godot.MainLoop"), "uninit", STree)
    _patch_one(STree, "get_Root", GWindow, "uninit")
    _patch_one(GNode, "GetProcessDeltaTime", clr.GetClrType(System.Double), "big_r8")


def install_loc_shim():
    """LocString text lookups need loc tables (not loaded headless) -> placeholder."""
    LS = None
    for t in asm.GetTypes():
        if str(t.Name) == "LocString":
            LS = t
    for m in LS.GetMethods(ALL):
        if m.DeclaringType == LS and str(m.ReturnType.Name) == "String" \
                and not m.IsStatic and str(m.Name) in ("GetFormattedText", "get_Text", "ToString"):
            pre = _emit_skip_prefix(clr.GetClrType(String), "empty_string")
            hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
            _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))


def install_log_tap():
    """Route the game's ConsoleLogPrinter.Print to stdout."""
    CLP = asm.GetType("MegaCrit.Sts2.Core.Logging.ConsoleLogPrinter")
    if CLP is None:
        for t in asm.GetTypes():
            if str(t.Name) == "ConsoleLogPrinter":
                CLP = t
    m = [m for m in CLP.GetMethods(ALL) if str(m.Name) == "Print"][0]
    pre = _emit_console_print_prefix()
    hm = Activator.CreateInstance(HarmonyMethod, Array[Object]([pre]))
    _patch.Invoke(_h, Array[Object]([m, hm, None, None, None]))


# ---------------------------------------------------------------- bootstrap
def boot():
    TestMode = T("MegaCrit.Sts2.Core.TestSupport.TestMode")
    call_static(TestMode, "TurnOnInternal")
    install_shim()
    install_loc_shim()
    install_game_shims()
    install_log_tap()
    SaveManager = T("MegaCrit.Sts2.Core.Saves.SaveManager")
    MockIo = T("MegaCrit.Sts2.Core.Saves.Test.MockGodotFileIo")
    save_dir = SPIKE / "mock_saves"
    save_dir.mkdir(exist_ok=True)
    mock_io = Activator.CreateInstance(MockIo, Array[Object]([str(save_dir)]))
    ctor2 = [c for c in SaveManager.GetConstructors(ALL)
             if c.GetParameters().Length == 2][0]
    sm = ctor2.Invoke(Array[Object]([mock_io, True]))
    call_static(SaveManager, "MockInstanceForTesting", sm)
    # SaveManager.Instance.PrefsSave is null until prefs are loaded; card
    # bodies read it MID-PLAY (e.g. Whirlwind's PrefsSave.FastMode VFX-delay
    # branch, IL_003f-0044) and NRE headlessly -- the async pipeline then
    # LOGS the exception and reports the play Finished with zero effect
    # (#339). MegaCrit's own test seam installs a default in-memory
    # PrefsSave with no file IO (InitPrefsDataForTest, 0x3f592).
    call(sm, "InitPrefsDataForTest")
    return sm


if __name__ == "__main__":
    sm = boot()
    print("boot OK; SaveManager.Instance =",
          call_static(T("MegaCrit.Sts2.Core.Saves.SaveManager"), "get_Instance"))
