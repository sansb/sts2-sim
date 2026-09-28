"""Spike probe 6: construct Player + RunState + CombatState, start combat, play Strike."""
import host
from host import T, call, call_static, prop, ALL
from System import Activator, Array, Object, Enum, UInt64, Int32, Boolean, String
import System

sm = host.boot()
print("[1] boot OK", flush=True)
asm = host.asm
MM = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
MMState = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState")
print("   ModManagerState values:", list(Enum.GetNames(MMState)), flush=True)
done_state = None
for n in Enum.GetNames(MMState):
    if str(n) in ("Initialized", "Finished", "Done", "Ready"):
        done_state = Enum.Parse(MMState, n)
assert done_state is not None, "no terminal ModManagerState found"
call_static(MM, "set_State", done_state)
ModelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
call_static(ModelDb, "Init")
print("[1b] ModelDb.Init OK; models:", flush=True)

def find(simple):
    for t in asm.GetTypes():
        if str(t.Name) == simple:
            return t
    raise KeyError(simple)

def new(t, *args):
    for c in t.GetConstructors(ALL):
        if not c.IsStatic and c.GetParameters().Length == len(args):
            return c.Invoke(Array[Object](list(args)))
    raise RuntimeError(f"no ctor/{len(args)} on {t}")

def generic_list(elem_t, items=()):
    lt = System.Type.GetType("System.Collections.Generic.List`1").MakeGenericType(elem_t)
    lst = Activator.CreateInstance(lt)
    add = lt.GetMethod("Add")
    for it in items:
        add.Invoke(lst, Array[Object]([it]))
    return lst

def step(n, msg):
    print(f"[{n}] {msg}", flush=True)

# ---- player ----
def model(simple):
    return call_static(ModelDb, "Get", find(simple))

ironclad = model("Ironclad")
US = find("UnlockState")
us = None
for c in US.GetConstructors(ALL):
    ps = c.GetParameters()
    if ps.Length == 3:
        def tn(p):
            t = p.ParameterType
            if t.IsGenericType:
                return f"{t.Name}<{','.join(str(a.Name) for a in t.GetGenericArguments())}>"
            return str(t.Name)
        print("   US 3-arg:", [f"{tn(p)} {p.Name}" for p in ps], flush=True)
        args = []
        for p in ps:
            t = p.ParameterType
            if t.IsGenericType:
                args.append(generic_list(t.GetGenericArguments()[0]))
            else:
                args.append(Int32(0))
        us = c.Invoke(Array[Object](args))
assert us is not None
rgb = new(find("RelicGrabBag"))
ModelId = find("ModelId")
Player_t = find("Player")
cfnr = [m for m in Player_t.GetMethods(ALL) if str(m.Name) == "CreateForNewRun"
        and m.GetParameters().Length == 3][0]
player = cfnr.Invoke(None, Array[Object]([ironclad, us, UInt64(0)]))
step(2, f"player OK: {player}")

# ---- run state ----
GameMode = find("GameMode")
standard = Enum.Parse(GameMode, "Standard")
glory = model("Glory")
RunState_t = find("RunState")
cft = [m for m in RunState_t.GetMethods(ALL) if str(m.Name) == "CreateForTest"][0]
run = cft.Invoke(None, Array[Object]([
    generic_list(Player_t, [player]),
    generic_list(find("ActModel"), [glory]),
    generic_list(find("ModifierModel")),
    standard, Int32(0), "SPIKETEST"]))
step(3, f"run state OK: {run}")

# ---- give the player a deck: 5 strikes ----
# discover deck-ish members on Player
for p in Player_t.GetProperties(ALL):
    n = str(p.Name)
    if any(k in n.lower() for k in ("deck", "card", "hand", "draw", "discard")):
        print("   Player prop:", str(p.PropertyType.Name), n, flush=True)
for m in Player_t.GetMethods(ALL):
    n = str(m.Name)
    if "Card" in n or "Deck" in n:
        print("   Player method:", n, [str(pp.ParameterType.Name) for pp in m.GetParameters()], flush=True)
step(4, "deck discovery done")

# ---- starting deck ----
deck = prop(player, "Deck")
deck_cards = list(prop(deck, "Cards"))
step(5, f"starting deck: {len(deck_cards)} cards: {[str(c) for c in deck_cards[:12]]}")

# ---- encounter + combat state ----
cultists = call(model("CultistsNormal"), "MutableClone")
CombatState_t = find("CombatState")
cs = new(CombatState_t, cultists, run, generic_list(find("ModifierModel")),
         generic_list(find("BadgeModel")), None)
step(6, f"combat state OK: {cs}")

# ---- run manager (owns ActionExecutor) ----
RM = find("RunManager")
rm = call_static(RM, "get_Instance")
NetSP = find("NetSingleplayerGameService")
for c in NetSP.GetConstructors(ALL):
    print("   NetSP ctor:", [f"{p.ParameterType.Name} {p.Name}" for p in c.GetParameters()], flush=True)
svc = new(NetSP, *([None] * [c.GetParameters().Length for c in NetSP.GetConstructors(ALL) if not c.IsStatic][0]))
sut = [m for m in RM.GetMethods(ALL) if str(m.Name) == "SetUpTest"][0]
print("   SetUpTest:", [f"{p.ParameterType.Name} {p.Name}" for p in sut.GetParameters()], flush=True)
sut.Invoke(rm, Array[Object]([run, svc, False, False]))
step(6.5, f"RunManager.SetUpTest OK; Instance={call_static(RM, 'get_Instance')}")

# ---- combat manager: set up + start ----
CM = find("CombatManager")
cm = call_static(CM, "get_Instance")
step(7, f"combat manager: {cm}")
# ---- creatures: player + encounter monsters ----
Creature_t = find("Creature")
cctor = [c for c in Creature_t.GetConstructors(ALL)
         if not c.IsStatic and c.GetParameters().Length == 3
         and str(c.GetParameters()[0].ParameterType.Name) == "Player"][0]
pcreature = cctor.Invoke(Array[Object]([player, Int32(80), Int32(80)]))
call(cs, "AddPlayer", player)
CombatSide = find("CombatSide")
print("   CombatSide values:", list(Enum.GetNames(CombatSide)), flush=True)
enemy_side = Enum.Parse(CombatSide, [n for n in Enum.GetNames(CombatSide) if "nem" in str(n)][0])
call(cultists, "GenerateMonstersWithSlots", run)
mws = prop(cultists, "MonstersWithSlots")
for item in mws:
    it = item.GetType()
    print("   mws item:", it, [str(p.Name) for p in it.GetProperties(ALL)],
          [str(f.Name) for f in it.GetFields(ALL)], flush=True)
    break
for item in mws:
    it = item.GetType()
    mon = it.GetField("Item1").GetValue(item)
    slot = it.GetField("Item2").GetValue(item)
    print("   spawning:", mon, "slot", slot, flush=True)
    cr = call(cs, "CreateCreature", mon, enemy_side, slot)
    if not any(True for _ in prop(cs, "Enemies")) or cr not in list(prop(cs, "Enemies")):
        call(cs, "AttachCreature", cr)
        call(cs, "AddCreature", cr)
    print("   enemies now:", len(list(prop(cs, "Enemies"))), flush=True)
def cinfo(c):
    try:
        m = prop(c, "Monster")
        label = str(m) if m is not None else "player"
    except Exception:
        label = "player"
    return f"{label} hp={prop(c,'CurrentHp')}/{prop(c,'MaxHp')} block={prop(c,'Block')}"

step(7.5, f"creatures: {[cinfo(c) for c in prop(cs, 'Creatures')]}")

rng_set = prop(run, "Rng")
print("   RunRngSet props:", [str(p.Name) for p in rng_set.GetType().GetProperties(ALL)], flush=True)
shuffle_rng = None
for cand in ("Shuffle", "CombatCard", "Combat", "Card"):
    try:
        shuffle_rng = prop(rng_set, cand)
        print(f"   using rng stream: {cand}", flush=True)
        break
    except AttributeError:
        continue
call(cm, "SetUpCombat", cs)
step(8, "SetUpCombat OK")
turn_loop_task = host.start_combat_internal(cm)
step(9, "StartCombatInternal started")
print("   IsInProgress:", prop(cm, "IsInProgress"), "IsStarting:", prop(cm, "IsStarting"),
      "IsEnding:", prop(cm, "IsEnding"), "CurrentSide:", prop(cs, "CurrentSide"),
      "Round:", prop(cs, "RoundNumber"), flush=True)

# ---- inspect state ----
creatures = prop(cs, "Creatures")
for c in creatures:
    print("   creature:", cinfo(c), "side", prop(c, "Side"), flush=True)
pcs = prop(player, "PlayerCombatState")
print("   PlayerCombatState props:", [str(p.Name) for p in pcs.GetType().GetProperties(ALL)], flush=True)
hand = None
for pname in ("Hand", "HandPile"):
    try:
        hand = prop(pcs, pname)
        break
    except AttributeError:
        pass
print("   draw pile:", len(list(prop(prop(pcs, "DrawPile"), "Cards"))), "cards", flush=True)
hand_cards = list(prop(hand, "Cards"))
if not hand_cards:
    print("   hand empty -> calling SetupPlayerTurn manually", flush=True)
    spt = [m for m in cm.GetType().GetMethods(ALL) if str(m.Name) == "SetupPlayerTurn"][0]
    HPC = find("HookPlayerChoiceContext")
    GAType = find("GameActionType")
    print("   GameActionType values:", list(Enum.GetNames(GAType))[:12], flush=True)
    ga_none = Enum.Parse(GAType, list(Enum.GetNames(GAType))[0])
    hpc_ctor = [c for c in HPC.GetConstructors(ALL)
                if str(c.GetParameters()[0].ParameterType.Name) == "Player"][0]
    ctx = hpc_ctor.Invoke(Array[Object]([player, UInt64(0), ga_none]))
    t2 = spt.Invoke(cm, Array[Object]([player, ctx]))
    if t2 is not None and "Task" in str(t2.GetType().Name):
        call(call(t2, "GetAwaiter"), "GetResult")
    hand_cards = list(prop(hand, "Cards"))
print("   hand:", [str(c) for c in hand_cards], flush=True)
try:
    print("   energy:", prop(pcs, "Energy"), flush=True)
except AttributeError:
    print("   energy prop not named Energy", flush=True)
step(10, "state inspected")

from System import AppDomain
def _fce(sender, args):
    ex2 = args.Exception
    if "NullReference" in str(ex2.GetType().Name):
        print("[FCE]", str(ex2.GetType().Name), str(ex2.StackTrace)[:2000], flush=True)
AppDomain.CurrentDomain.FirstChanceException += _fce

# ---- drain pending setup actions, advance phase to Play ----
rmi0 = call_static(RM, "get_Instance")
ex0 = prop(rmi0, "ActionExecutor")
call(ex0, "Unpause")
pump0 = call(ex0, "ExecuteActions")
dr = call(ex0, "FinishedExecutingActions")
call(call(dr, "GetAwaiter"), "GetResult")
hand_cards = list(prop(hand, "Cards"))
print("   after drain: phase:", prop(pcs, "Phase"), "energy:", prop(pcs, "Energy"),
      "hand:", [str(c) for c in hand_cards], flush=True)
PTP = find("PlayerTurnPhase")
print("   PlayerTurnPhase values:", list(Enum.GetNames(PTP)), flush=True)
if str(prop(pcs, "Phase")) != "Play":
    play_phase = Enum.Parse(PTP, "Play")
    call(cm, "SetPhaseForAllPlayers", play_phase)
    print("   phase forced to:", prop(pcs, "Phase"), flush=True)

# ---- play a Strike at the Calcified Cultist ----
strike = next(c for c in hand_cards if "STRIKE" in str(c))
target = next(c for c in prop(cs, "Enemies") if "CALCIFIED" in cinfo(c))
before = prop(target, "CurrentHp")
PCA = find("PlayCardAction")
pca_ctor = [c for c in PCA.GetConstructors(ALL)
            if not c.IsStatic and c.GetParameters().Length == 2][0]
action = pca_ctor.Invoke(Array[Object]([strike, target]))
step(11, f"PlayCardAction constructed for {strike} -> {cinfo(target)}")
print("   action.Player:", prop(action, "Player"), flush=True)
pile = prop(strike, "Pile")
print("   strike.Pile:", prop(pile, "Type") if pile else None, flush=True)
for gate in ("CanPlay", "IsValidTarget"):
    ms = [m for m in strike.GetType().GetMethods(ALL) if str(m.Name) == gate]
    for m in ms:
        ps = m.GetParameters()
        print(f"   {gate} sig:", [f"{p.ParameterType.Name} {p.Name}" for p in ps], flush=True)
        try:
            args = []
            for p in ps:
                tn2 = str(p.ParameterType.Name)
                args.append(target if tn2 == "Creature" else (player if tn2 == "Player" else None))
            print(f"   {gate} ->", m.Invoke(strike, Array[Object](args) if args else None), flush=True)
        except Exception as e2:
            print(f"   {gate} ERR", repr(e2)[:120], flush=True)
rmi = call_static(RM, "get_Instance")
print("   rmi ok", flush=True)
aqs = prop(rmi, "ActionQueueSynchronizer")
print("   aqs:", aqs, flush=True)
ex = prop(rmi, "ActionExecutor")
print("   ex:", ex, flush=True)
try:
    call(aqs, "EnqueueAction", action, UInt64(0))
    print("   direct EnqueueAction OK", flush=True)
except Exception as e:
    print("   EnqueueAction failed:", repr(e)[:200], flush=True)
    call(aqs, "RequestEnqueue", action)
print("   action state after enqueue:", prop(action, "State"), flush=True)
call(ex, "Unpause")
pump = call(ex, "ExecuteActions")
print("   pump started; IsRunning:", prop(ex, "IsRunning"), flush=True)
import time
for i in range(50):
    st = str(prop(action, "State"))
    if st in ("Finished", "Cancelled"):
        break
    time.sleep(0.1)
print("   action state:", prop(action, "State"), flush=True)
after = prop(target, "CurrentHp")
step(12, f"STRIKE PLAYED: target hp {before} -> {after} (expected -6)")
print("   hand now:", [str(c) for c in prop(hand, "Cards")], flush=True)
print("   energy now:", prop(pcs, "Energy"), "phase:", prop(pcs, "Phase"),
      "turn:", prop(pcs, "TurnNumber"), flush=True)
print("   discard:", [str(c) for c in prop(prop(pcs, "DiscardPile"), "Cards")], flush=True)
print("   player hp:", prop(prop(player, "Creature"), "CurrentHp"), flush=True)
hist = prop(cm, "History")
print("   history type:", hist.GetType() if hist else None, flush=True)
try:
    entries = list(prop(hist, "Entries"))
except Exception:
    try:
        entries = list(hist)
    except Exception:
        entries = []
for e in entries[-12:]:
    et = e.GetType()
    fields = {str(f.Name): f.GetValue(e) for f in et.GetFields(ALL)}
    print("   hist:", str(et.Name), {k: str(v)[:40] for k, v in list(fields.items())[:6]}, flush=True)
