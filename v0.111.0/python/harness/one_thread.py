"""The game on one thread: a SynchronizationContext the probe pumps (#3931).

The harness has no SynchronizationContext of its own, so an `await` that does
not complete inline resumes on the thread pool. Two awaits in combat code do
that in every headless run, and everything after them then runs beside the
thread that is driving the probe:

- `CombatManager/<AfterAllPlayersReadyToBeginEnemyTurn>d__137` (RVA `0x3f283c`)
  awaits `Task.Yield()` at IL_008e-0097: the second phase of the player's turn
  end, the whole enemy turn and the next turn start resume on a pool thread;
- `ActionExecutor/<WaitForUnpause>d__31` (RVA `0x3d4880`) awaits
  `Task.Delay(50)` under `TestMode` (IL_0038-0046) while the executor is
  paused, which it is from `StartCombatInternal` until the opening
  `StartTurn` unpauses it.

`CombatState/<GodotTimerTask>d__75` (RVA `0x3f952c`, `Task.Delay` under
`TestMode`, reached from `GetCreatureAsync` only when a target does not exist
yet) and `TestCardSelector/<GetSelectedCards>d__20` (RVA `0x2ffedc`,
`Task.Delay` at IL_006e only when the test asked it to block) are the other
two a probe can reach.

In a played game Godot installs its own context and every one of these
resumes on the main thread, a frame later. `PumpContext` does the same for a
probe: `Post` puts the continuation on a queue, and the probe runs the queue
from its own loop. The thread that boots the CLR is then the only thread game
code runs on, a probe's snapshot can no longer be taken while a pool thread
is still inside the enemy turn, and no loop needs a sleep to let that thread
finish. `idle` is what replaces the sleeps: it returns as soon as something
is posted, and after its timeout otherwise (a `Task.Delay` is real time).

Nothing installs this by itself. A script that blocks on `GetResult()` for a
task that crosses one of the awaits above would wait for ever under it, and
several of the older harness scripts do; they keep running as they always
have. The two native probes install it in every mode.
"""


class PumpContext:
    def __init__(self):
        import clr
        import host
        from System import Array, Boolean, Int32, Object, Type
        from System.Collections.Concurrent import ConcurrentQueue
        from System.Collections.Generic import KeyValuePair
        from System.Reflection import CallingConventions, FieldAttributes, MethodAttributes
        from System.Reflection.Emit import OpCodes
        from System.Threading import ManualResetEventSlim, SendOrPostCallback, SynchronizationContext

        item_t = clr.GetClrType(KeyValuePair[SendOrPostCallback, Object])
        queue_t = clr.GetClrType(ConcurrentQueue[KeyValuePair[SendOrPostCallback, Object]])
        event_t = clr.GetClrType(ManualResetEventSlim)
        callback_t = clr.GetClrType(SendOrPostCallback)
        object_t = clr.GetClrType(Object)
        int_t = clr.GetClrType(Int32)
        base_t = clr.GetClrType(SynchronizationContext)

        host._shim_n += 1
        tb = host._mod.DefineType(f"PumpContext{host._shim_n}", _public_class(), base_t)
        queue_f = tb.DefineField("Queue", queue_t, FieldAttributes.Public)
        signal_f = tb.DefineField("Signal", event_t, FieldAttributes.Public)
        posts_f = tb.DefineField("Posts", int_t, FieldAttributes.Public)
        tb.DefineDefaultConstructor(MethodAttributes.Public)
        virtual = MethodAttributes.Public | MethodAttributes.Virtual | MethodAttributes.HideBySig

        # public override void Post(SendOrPostCallback d, object state)
        post = tb.DefineMethod("Post", virtual, CallingConventions.HasThis, None,
                               Array[Type]([callback_t, object_t]))
        il = post.GetILGenerator()
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldfld, queue_f)
        il.Emit(OpCodes.Ldarg_1)
        il.Emit(OpCodes.Ldarg_2)
        il.Emit(OpCodes.Newobj, item_t.GetConstructor(Array[Type]([callback_t, object_t])))
        il.EmitCall(OpCodes.Callvirt, queue_t.GetMethod("Enqueue"), None)
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldfld, posts_f)
        il.Emit(OpCodes.Ldc_I4_1)
        il.Emit(OpCodes.Add)
        il.Emit(OpCodes.Stfld, posts_f)
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldfld, signal_f)
        il.EmitCall(OpCodes.Callvirt, event_t.GetMethod("Set"), None)
        il.Emit(OpCodes.Ret)

        # public override SynchronizationContext CreateCopy() => this
        copy = tb.DefineMethod("CreateCopy", virtual, CallingConventions.HasThis, base_t, Array[Type]([]))
        il = copy.GetILGenerator()
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ret)

        # public bool RunOne(): run the oldest posted continuation on the calling thread
        run = tb.DefineMethod("RunOne", MethodAttributes.Public | MethodAttributes.HideBySig,
                              CallingConventions.HasThis, clr.GetClrType(Boolean), Array[Type]([]))
        il = run.GetILGenerator()
        item = il.DeclareLocal(item_t)
        found = il.DefineLabel()
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldfld, queue_f)
        il.Emit(OpCodes.Ldloca, item)
        il.EmitCall(OpCodes.Callvirt, queue_t.GetMethod("TryDequeue"), None)
        il.Emit(OpCodes.Brtrue, found)
        il.Emit(OpCodes.Ldc_I4_0)
        il.Emit(OpCodes.Ret)
        il.MarkLabel(found)
        il.Emit(OpCodes.Ldloca, item)
        il.EmitCall(OpCodes.Call, item_t.GetProperty("Key").GetGetMethod(), None)
        il.Emit(OpCodes.Ldloca, item)
        il.EmitCall(OpCodes.Call, item_t.GetProperty("Value").GetGetMethod(), None)
        il.EmitCall(OpCodes.Callvirt, callback_t.GetMethod("Invoke"), None)
        il.Emit(OpCodes.Ldc_I4_1)
        il.Emit(OpCodes.Ret)

        made = tb.CreateType()
        self._type = made
        self.context = made.GetConstructor(Array[Type]([])).Invoke(None)
        self.queue = ConcurrentQueue[KeyValuePair[SendOrPostCallback, Object]]()
        self.signal = ManualResetEventSlim(False)
        made.GetField("Queue").SetValue(self.context, self.queue)
        made.GetField("Signal").SetValue(self.context, self.signal)
        self._posts = made.GetField("Posts")
        self._run_one = made.GetMethod("RunOne")
        self.thread = None
        self.ran = 0

    def install(self):
        """Make the calling thread the game's thread. Call it once, right after the boot."""
        import System
        from System.Threading import SynchronizationContext
        SynchronizationContext.SetSynchronizationContext(self.context)
        self.thread = int(System.Environment.CurrentManagedThreadId)
        return self

    @property
    def pending(self):
        return not bool(self.queue.IsEmpty)

    @property
    def posts(self):
        """How many continuations were posted so far (each one is a point the game gave the loop back)."""
        return int(self._posts.GetValue(self.context))

    def run_posted(self, limit=100000):
        """Run every posted continuation, those posted meanwhile too. Returns how many."""
        n = 0
        self.signal.Reset()
        while n < limit and bool(self._run_one.Invoke(self.context, None)):
            n += 1
        self.ran += n
        return n

    def idle(self, seconds):
        """Wait for a post from another thread (a `Task.Delay` running out), at most `seconds`."""
        if self.pending:
            return True
        return bool(self.signal.Wait(max(1, int(seconds * 1000))))


def _public_class():
    from System.Reflection import TypeAttributes
    return TypeAttributes.Public | TypeAttributes.Class
