"""Spike probe 7 (#277): end the player turn, run the enemy phase headlessly."""
import time

exec(open(__file__.replace("demo_enemy_turn", "demo_play_strike")).read())  # full fight setup + Strike played (hp 38->32)

print("\n=== PROBE 7: enemy turn ===", flush=True)

def enqueue_and_pump(act, timeout_s=8.0):
    call(aqs, "EnqueueAction", act, UInt64(0))
    call(ex, "Unpause")
    call(ex, "ExecuteActions")
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        st = str(prop(act, "State"))
        if st in ("Finished", "Cancelled"):
            return st
        time.sleep(0.05)
    return f"timeout in state {prop(act, 'State')}"

player_c = prop(player, "Creature")
hp_before = prop(player_c, "CurrentHp")

EPT = find("EndPlayerTurnAction")
ept_ctor = [c for c in EPT.GetConstructors(ALL) if not c.IsStatic][0]
ept = ept_ctor.Invoke(Array[Object]([player, Int32(int(prop(pcs, "TurnNumber")))]))
print("[13] EndPlayerTurnAction ->", enqueue_and_pump(ept), flush=True)

# co-op protocol step 2: ready-to-begin-enemy-turn (null func = no mid-turn action)
RBET = find("ReadyToBeginEnemyTurnAction")
rbet_ctor = [c for c in RBET.GetConstructors(ALL) if not c.IsStatic][0]
from System import Func
from System.Threading.Tasks import Task as _Task
_noop = Func[_Task](lambda: _Task.CompletedTask)
rbet = rbet_ctor.Invoke(Array[Object]([player, _noop]))
print("[14] ReadyToBeginEnemyTurnAction ->", enqueue_and_pump(rbet), flush=True)

# give async enemy phase a moment, then keep pumping until back on Player side
for i in range(100):
    side = str(prop(cs, "CurrentSide"))
    rnd = prop(cs, "RoundNumber")
    if side == "Player" and rnd == 2:
        break
    call(ex, "Unpause")
    call(ex, "ExecuteActions")
    time.sleep(0.1)
print("[15] side:", prop(cs, "CurrentSide"), "round:", prop(cs, "RoundNumber"),
      "turn:", prop(pcs, "TurnNumber"), flush=True)

print("[16] player:", cinfo(player_c), f"(was hp={hp_before})", flush=True)
for c in prop(cs, "Enemies"):
    print("   enemy:", cinfo(c), flush=True)
print("   hand round 2:", [str(c) for c in prop(hand, "Cards")],
      "energy:", prop(pcs, "Energy"), flush=True)

print("\n   --- history tail ---", flush=True)
entries = list(prop(hist, "Entries")) if hasattr(hist, "Entries") else []
try:
    entries = list(prop(hist, "Entries"))
except Exception:
    entries = list(hist)
for e in entries[-18:]:
    et = e.GetType()
    fields = {str(f.Name).replace("<", "").replace(">k__BackingField", ""): str(f.GetValue(e))[:45]
              for f in et.GetFields(ALL)}
    print("   hist:", str(et.Name), fields, flush=True)
print("\nPROBE 7 DONE", flush=True)

# ---- round 2 player-turn setup (driver responsibility, same as round 1) ----
ctx2 = hpc_ctor.Invoke(Array[Object]([player, UInt64(0), ga_none]))
t3 = spt.Invoke(cm, Array[Object]([player, ctx2]))
call(call(t3, "GetAwaiter"), "GetResult")
print("[17] round 2 hand:", [str(c) for c in prop(hand, "Cards")],
      "energy:", prop(pcs, "Energy"), "phase:", prop(pcs, "Phase"), flush=True)
print("PROBE 7 ROUND 2 READY", flush=True)
