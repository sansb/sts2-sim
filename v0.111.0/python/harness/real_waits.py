"""Real waits for the native probes: `Cmd.Wait` as a timer, not a no-op (#3931).

The harness runs with `TestMode` on, so `NonInteractiveMode.IsActive` (RVA
`0x10a381`) is true and `Cmd/<Wait>d__1` (RVA `0x3e8d28`) returns at once
(IL_0019-0020). In a game someone is playing it awaits a SceneTree timer
(IL_006e-0096). Where one flow awaits one thing at a time that makes no
difference to the order things happen in. It matters where the game runs two
flows at once; README.md, "Real waits", lists every such place found in the
IL, which of them can change combat state, and what this emulation does and
does not reproduce.

What it does. `Cmd.Wait` (the three-argument overload every other one calls)
and `Cmd.CustomScaledWait` are Harmony-prefixed. Each wait the game would
really take becomes a pending task with a duration; `pump` advances a virtual
frame clock to the next expiry and completes the timers that ran out, oldest
first, on this thread, so the continuation runs inline the way a SceneTree
timer's `timeout` does. The gates the game applies itself are kept, in its
order: `seconds <= 0` (IL_0025-0032), Instant fast mode (IL_0037-0050), the
combat ending unless `ignoreCombatEnd` (IL_0055-0069); and for
`CustomScaledWait` (`<CustomScaledWait>d__3` RVA `0x3e8b64`) Instant
(IL_002c-003e), the combat ending (IL_0043-0057), then the Fast or the
Standard seconds by fast mode (IL_005c-0107).

The Instant gate of `Cmd.Wait` sits behind `NGame.Instance != null`
(IL_0037-003c). The harness has no `NGame`, so the game's own code would skip
the gate here; a played game always has one, and `_wait` applies the gate as
a played game does.

The fast mode is also written to `SaveManager.Instance.PrefsSave.FastMode`,
because the two waits are not its only readers: `DoTurnEndCards` (RVA
`0x3f3a68` IL_00de-00f7) spaces the turn-end cards by it, and `TalkCmd.Play`
(RVA `0x133fe8`) and the OnPlay of Flash of Steel, Neutralize, Slice,
Suppress and Whirlwind read it directly.

`always` adds a yield: the first `SignalPlayerChoiceBegun` of a
`HookPlayerChoiceContext` whose task is not assigned yet waits one frame
first. `after` adds another: `SignalPlayerChoiceEnded` on a hook context
completes one frame late. They stand in for the awaits this module does not
emulate (the card tweens of `CardPileCmd`, node animations) before and after
a listener's selection. Each is wrong for a listener that has no such await
there, so they are further recordings to compare, not the default.

What it reports. `report()` counts the waits and lists every moment two or
more timers were pending at once, with the game frames that created them.
With one timer pending at a time the order is forced and durations do not
matter. With more, two flows were racing and the order came from the
durations: this module resolves that by its clock (frame length `1/fps`,
timers in creation order within a frame), which is a model of the engine's
timer list, not a recording of it. `DoActivateVisuals` timers (a relic's
flash, started fire-and-forget) are listed apart: they touch no combat state.
`hook_signals` lists every selection signalled on a hook context, with
whether the walk was already waiting for it: each one is a point where a
second flow can start whether or not two timers were ever pending together.

One thread. `install` puts the game on the probe's thread in every mode,
headless too (`one_thread.PumpContext`): `pump` runs what the game posted,
and `idle` replaces the sleeps the probes used to need. A wait created on any
other thread is listed under `off_thread_waits` and refuses the run.

Without `install`, `OFF.pump` is the plain `ExecuteActions` call the probes
always made and nothing else changes.
"""

FAST_MODES = ("normal", "fast", "instant")
# `PrefsSave.FastMode` as the game numbers it: `Cmd.Wait` compares with 3
# (IL_004d), `DoTurnEndCards` with 2 (IL_00ed), and `CustomScaledWait`
# switches 1 to the Standard seconds, 2 to the Fast ones, 3 to no wait and
# throws on 0 (IL_006d).
FAST_MODE_VALUE = {"normal": 1, "fast": 2, "instant": 3}
LISTENER_YIELDS = ("timers", "always", "after", "always+after")
# A wait whose creating stack holds one of these is display only: the relic
# flashes on a fire-and-forget task (`TaskHelper.RunSafely`), e.g.
# `IronClub/<AfterCardPlayed>d__19` RVA `0x326f78` IL_0078-007d.
VISUAL_ONLY = ("DoActivateVisuals", "RevealWrigglersAfterDeathAnim",
               "RevealReplacementAfterDeathAnim", "OnEnqueuePlayVfx", "PlayPowerCardFlyVfx")
WAIT_TIMEOUT_S = 120.0


def _one_thread():
    """Put the game on the calling thread; `_Fight` pumps the same queue."""
    import experiment
    import one_thread
    posted = one_thread.PumpContext().install()
    experiment._Fight.posted = posted
    return posted


class Off:
    """No emulation of waits: `Cmd.Wait` returns at once, as the harness always ran."""

    active = False
    posted = None             # `one_thread.PumpContext` once `install` ran

    def pump(self, ex):
        from experiment import call
        call(ex, "ExecuteActions")
        return self.posted.run_posted() if self.posted is not None else 0

    def idle(self, seconds):
        """Let real time pass for a `Task.Delay`: a sleep, or a wait for its post."""
        if self.posted is None:
            import time
            time.sleep(seconds)
        else:
            self.posted.idle(seconds)

    def drain(self, ex, limit=0):
        return 0

    def wait(self, ex, task):
        from experiment import call
        if self.posted is not None:
            # A blocking GetResult would never run a posted continuation.
            import time
            t0 = time.time()
            while not bool(task.IsCompleted):
                call(ex, "Unpause")
                if not self.pump(ex):
                    self.posted.idle(0.01)
                if time.time() - t0 > WAIT_TIMEOUT_S:
                    raise SystemExit(f"a task the probe started never completed ({WAIT_TIMEOUT_S:.0f} s)")
        call(call(task, "GetAwaiter"), "GetResult")

    def install(self):
        """The probes call this right after the boot: one thread for the game."""
        self.posted = _one_thread()

    def begin(self, fight):
        pass

    def stalled_on(self, what):
        """Headless keeps its old behaviour: the caller reports its own timeout state."""

    def report(self):
        return None


OFF = Off()


class Stalled(SystemExit):
    """A real-waits run that stopped making progress. Exit status 3; never a trace."""


class RealWaits:
    active = True

    def __init__(self, fast_mode="normal", fps=60.0, listener_yield="timers"):
        import threading
        assert fast_mode in FAST_MODES, fast_mode
        assert listener_yield in LISTENER_YIELDS, listener_yield
        self.fast_mode = fast_mode
        self.delta = 1.0 / float(fps)
        self.fps = float(fps)
        self.listener_yield = listener_yield
        self.timers = []          # [left, tcs, seconds, site, serial]
        self.count = 0
        self.frames = 0
        self.max_pending = 0
        self.concurrent = []
        self.forced_yields = 0
        self.enabled = False      # until `begin`: a scenario's setup runs as it always did
        self._keep = []           # delegates the CLR must not collect
        self._yielded = []
        self._in_ended = False
        self.stalled = None
        self.posted = None        # `one_thread.PumpContext` once `install` ran
        self.pump_thread = None
        self.off_thread_waits = []
        self.hook_signals = []
        self._lock = threading.Lock()   # the timer list, should a wait ever come from another thread

    # ---- the game's own gates, then a timer --------------------------------
    def _wait(self, seconds, ignore_combat_end):
        from System.Threading.Tasks import Task
        if not self.enabled or float(seconds) <= 0.0 or self.fast_mode == "instant":
            return Task.CompletedTask
        if not ignore_combat_end:
            if self._combat_ending():
                return Task.CompletedTask
        return self._timer(float(seconds))

    def _scaled(self, fast_seconds, standard_seconds, ignore_combat_end):
        from System.Threading.Tasks import Task
        if not self.enabled or self.fast_mode == "instant":
            return Task.CompletedTask
        if not ignore_combat_end:
            if self._combat_ending():
                return Task.CompletedTask
        seconds = fast_seconds if self.fast_mode == "fast" else standard_seconds
        return self._wait(seconds, ignore_combat_end)

    def _combat_ending(self):
        from experiment import call_static, find, prop
        cm = call_static(find("CombatManager"), "get_Instance")
        return cm is not None and bool(prop(cm, "IsEnding"))

    def _site(self):
        import System
        frames = []
        for line in str(System.Environment.StackTrace).splitlines():
            line = line.strip()
            if "MegaCrit." not in line or ".Cmd." in line and ("Wait" in line):
                continue
            name = line.split("(")[0].replace("at ", "").strip()
            name = name.replace(".MoveNext", "").split(".")
            frames.append(".".join(name[-2:]))
            if len(frames) == 4:
                break
        return frames

    def _thread(self):
        import System
        return int(System.Environment.CurrentManagedThreadId)

    def _timer(self, seconds):
        from System.Threading.Tasks import TaskCompletionSource
        tcs = TaskCompletionSource()
        site = self._site()
        with self._lock:
            self.count += 1
            if self.pump_thread is not None and self._thread() != self.pump_thread:
                # Game code left the pump thread: the clock no longer orders it.
                self.off_thread_waits.append(dict(serial=self.count, site=site))
            self.timers.append([seconds, tcs, seconds, site, self.count])
        return tcs.Task

    # ---- install -----------------------------------------------------------
    def install(self):
        """One thread, the fast mode game-wide, the two waits and the hook context's signals."""
        self.posted = _one_thread()
        self.pump_thread = self.posted.thread
        self._set_fast_mode()
        self._install_waits()
        self._install_signal_begun()
        if "after" in self.listener_yield:
            self._install_signal_ended()

    def _set_fast_mode(self):
        """`PrefsSave.FastMode`, for the game code that reads it without going through a wait."""
        import System
        from experiment import ALL, call_static, find, prop
        prefs = prop(call_static(find("SaveManager"), "get_Instance"), "PrefsSave")
        setter = [m for m in prefs.GetType().GetMethods(ALL) if str(m.Name) == "set_FastMode"][0]
        enum_t = setter.GetParameters()[0].ParameterType
        value = System.Enum.ToObject(enum_t, System.Int32(FAST_MODE_VALUE[self.fast_mode]))
        setter.Invoke(prefs, System.Array[System.Object]([value]))
        got = System.Convert.ToInt32(prop(prefs, "FastMode"))
        assert int(got) == FAST_MODE_VALUE[self.fast_mode], got

    def _install_waits(self):
        import clr
        import System
        import host
        from experiment import ALL, find
        from System import Array, Boolean, Func, Object, Single, Type
        from System.Reflection import (CallingConventions, FieldAttributes,
                                       MethodAttributes, ParameterAttributes)
        from System.Reflection.Emit import OpCodes
        from System.Threading.Tasks import Task

        task_t = clr.GetClrType(Task)
        none = getattr(ParameterAttributes, "None")
        host._shim_n += 1
        tb = host._mod.DefineType(f"RealWaits{host._shim_n}")

        def prefix(name, func_t, arg_types, arg_names):
            field = tb.DefineField(name + "Hook", func_t, FieldAttributes.Public | FieldAttributes.Static)
            mb = tb.DefineMethod(
                name, MethodAttributes.Public | MethodAttributes.Static, CallingConventions.Standard,
                clr.GetClrType(Boolean), Array[Type]([task_t.MakeByRefType()] + arg_types))
            mb.DefineParameter(1, none, "__result")
            for at, arg in enumerate(arg_names):
                mb.DefineParameter(at + 2, none, arg)
            il = mb.GetILGenerator()
            il.Emit(OpCodes.Ldarg_0)
            il.Emit(OpCodes.Ldsfld, field)
            for at in range(len(arg_names)):
                il.Emit(OpCodes.Ldarg, at + 1)
            il.EmitCall(OpCodes.Callvirt, func_t.GetMethod("Invoke"), None)
            il.Emit(OpCodes.Stind_Ref)
            il.Emit(OpCodes.Ldc_I4_0)
            il.Emit(OpCodes.Ret)

        single, boolean = clr.GetClrType(Single), clr.GetClrType(Boolean)
        wait_f = clr.GetClrType(Func[Single, Boolean, Task])
        scaled_f = clr.GetClrType(Func[Single, Single, Boolean, Task])
        prefix("Wait", wait_f, [single, boolean], ["seconds", "ignoreCombatEnd"])
        prefix("Scaled", scaled_f, [single, single, boolean],
               ["fastSeconds", "standardSeconds", "ignoreCombatEnd"])
        made = tb.CreateType()
        wait_d = Func[Single, Boolean, Task](lambda s, i: self._wait(float(s), bool(i)))
        scaled_d = Func[Single, Single, Boolean, Task](
            lambda f, s, i: self._scaled(float(f), float(s), bool(i)))
        self._keep += [wait_d, scaled_d]
        made.GetField("WaitHook").SetValue(None, wait_d)
        made.GetField("ScaledHook").SetValue(None, scaled_d)

        patched = []
        for m in find("Cmd").GetMethods(ALL):
            if str(m.Name) == "Wait" and m.GetParameters().Length == 3:
                pre = made.GetMethod("Wait")
            elif str(m.Name) == "CustomScaledWait":
                pre = made.GetMethod("Scaled")
            else:
                continue
            hm = System.Activator.CreateInstance(host.HarmonyMethod, Array[Object]([pre]))
            host._patch.Invoke(host._h, Array[Object]([m, hm, None, None, None]))
            patched.append(str(m.Name))
        assert sorted(patched) == ["CustomScaledWait", "Wait"], patched

    def _signal_prefix(self, name, target, hook):
        """Harmony-prefix a `Task`-returning method of the hook context with `hook(ctx, args)`.

        `hook` returns the task to hand back instead, or None to run the game's own method.
        """
        import clr
        import System
        import host
        from System import Array, Boolean, Func, Object, Type
        from System.Reflection import (CallingConventions, FieldAttributes,
                                       MethodAttributes, ParameterAttributes)
        from System.Reflection.Emit import OpCodes
        from System.Threading.Tasks import Task

        task_t = clr.GetClrType(Task)
        obj_t = clr.GetClrType(Object)
        args_t = clr.GetClrType(Array[Object])
        none = getattr(ParameterAttributes, "None")
        func_t = clr.GetClrType(Func[Object, Array[Object], Task])
        host._shim_n += 1
        tb = host._mod.DefineType(f"{name}{host._shim_n}")
        field = tb.DefineField("Hook", func_t, FieldAttributes.Public | FieldAttributes.Static)
        mb = tb.DefineMethod(
            "Prefix", MethodAttributes.Public | MethodAttributes.Static, CallingConventions.Standard,
            clr.GetClrType(Boolean), Array[Type]([task_t.MakeByRefType(), obj_t, args_t]))
        mb.DefineParameter(1, none, "__result")
        mb.DefineParameter(2, none, "__instance")
        mb.DefineParameter(3, none, "__args")
        il = mb.GetILGenerator()
        local = il.DeclareLocal(task_t)
        run_original = il.DefineLabel()
        il.Emit(OpCodes.Ldsfld, field)
        il.Emit(OpCodes.Ldarg_1)
        il.Emit(OpCodes.Ldarg_2)
        il.EmitCall(OpCodes.Callvirt, func_t.GetMethod("Invoke"), None)
        il.Emit(OpCodes.Stloc, local)
        il.Emit(OpCodes.Ldloc, local)
        il.Emit(OpCodes.Brfalse, run_original)
        il.Emit(OpCodes.Ldarg_0)
        il.Emit(OpCodes.Ldloc, local)
        il.Emit(OpCodes.Stind_Ref)
        il.Emit(OpCodes.Ldc_I4_0)
        il.Emit(OpCodes.Ret)
        il.MarkLabel(run_original)
        il.Emit(OpCodes.Ldc_I4_1)
        il.Emit(OpCodes.Ret)
        made = tb.CreateType()
        hook_d = Func[Object, Array[Object], Task](hook)
        self._keep.append(hook_d)
        made.GetField("Hook").SetValue(None, hook_d)
        hm = System.Activator.CreateInstance(host.HarmonyMethod, Array[Object]([made.GetMethod("Prefix")]))
        host._patch.Invoke(host._h, Array[Object]([target, hm, None, None, None]))

    def _install_signal_begun(self):
        """Record every selection signalled on a hook context; `always` makes the first wait a frame.

        Headless, a listener reaches `SignalPlayerChoiceBegun` before the walk
        has its task, and the signal awaits `_taskAssignedCompletionSource`
        (`<SignalPlayerChoiceBegun>d__37` RVA `0x3d64f4` IL_01df-021b), so the
        walk never sees the pause before the listener runs on. Under `always`
        the prefix returns a task that resumes the original on the next frame,
        when the walk has assigned the task and is awaiting
        `WhenAny(task, paused)`.
        """
        from experiment import ALL, find
        from System import Array, Func, Object
        from System.Threading.Tasks import Task, TaskContinuationOptions, TaskExtensions

        ctx_t = find("HookPlayerChoiceContext")
        target = [m for m in ctx_t.GetMethods(ALL) if str(m.Name) == "SignalPlayerChoiceBegun"][0]
        task_prop = ctx_t.GetProperty("Task", ALL)
        source_prop = ctx_t.GetProperty("Source", ALL)
        action_field = ctx_t.GetField("_gameAction", ALL)

        def hook(ctx, args):
            if not self.enabled:
                return None
            resumed = any(seen is ctx or seen.Equals(ctx) for seen in self._yielded)
            assigned = task_prop.GetValue(ctx) is not None
            if not resumed:
                source = source_prop.GetValue(ctx)
                self.hook_signals.append(dict(
                    source=str(source.GetType().Name) if source is not None else None,
                    walk_waiting=assigned, waits_before=self.count, frame=self.frames))
            if ("always" not in self.listener_yield or assigned or resumed
                    or action_field.GetValue(ctx) is not None):
                return None                       # the game's own path
            self._yielded.append(ctx)             # once a context: some are never assigned a task
            self.forced_yields += 1
            gate = self._timer(0.0)               # runs out on the next frame
            kept = Array[Object](list(args))
            resume = Func[Task, Task](lambda _t: target.Invoke(ctx, kept))
            self._keep.append(resume)
            chained = gate.ContinueWith[Task](resume, TaskContinuationOptions.ExecuteSynchronously)
            return TaskExtensions.Unwrap(chained)

        self._signal_prefix("RealWaitsBegun", target, hook)

    def _install_signal_ended(self):
        """`after`: a hook context's `SignalPlayerChoiceEnded` completes one frame late.

        A listener whose selection is the first thing it does runs straight
        through the signal in every mode, and the walk is released only at the
        listener's first await after it
        (`<AssignTaskAndWaitForPauseOrCompletion>d__34` RVA `0x3d62fc` reaches
        its `WhenAny` at IL_0054-0070 with `_pausedCompletionSource` already
        complete). When that await is a `Cmd.Wait` the timers reproduce it.
        When it is a card tween they do not; this yield stands in for it, at
        the earliest point one could be.
        """
        from experiment import ALL, find
        from System import Array, Object
        from System.Threading.Tasks import Task

        ctx_t = find("HookPlayerChoiceContext")
        target = [m for m in ctx_t.GetMethods(ALL) if str(m.Name) == "SignalPlayerChoiceEnded"][0]

        def hook(ctx, args):
            if not self.enabled or self._in_ended:
                return None
            self._in_ended = True
            try:
                original = target.Invoke(ctx, Array[Object](list(args)))
            finally:
                self._in_ended = False
            self.forced_yields += 1
            return Task.WhenAll(Array[Task]([original, self._timer(0.0)]))

        self._signal_prefix("RealWaitsEnded", target, hook)

    # ---- the clock ---------------------------------------------------------
    def _note_concurrent(self):
        live = [t for t in self.timers if not any(v in ".".join(t[3]) for v in VISUAL_ONLY)]
        self.max_pending = max(self.max_pending, len(live))
        if len(live) >= 2 and len(self.concurrent) < 40:
            self.concurrent.append(dict(
                frame=self.frames,
                timers=[dict(serial=t[4], seconds=round(t[2], 4), left=round(t[0], 4), site=t[3])
                        for t in live]))

    def advance(self, frames=None):
        """Run the clock to the next expiry (or `frames` frames); complete what ran out. Returns how many."""
        with self._lock:
            if not self.timers:
                return 0
            self._note_concurrent()
            # SceneTree timers lose `delta` a frame and fire once below zero.
            need = frames if frames is not None else min(int(t[0] / self.delta) + 1 for t in self.timers)
            self.frames += need
            due, rest = [], []
            for t in self.timers:
                t[0] -= need * self.delta
                (due if t[0] < 0.0 else rest).append(t)
            self.timers[:] = rest     # timers made while these fire wait for a later frame
        for t in due:
            t[1].SetResult()
        return len(due)

    def pump(self, ex):
        """One probe pump: start the executor if it is idle, then let time pass.

        An `ExecuteActions` already in flight (suspended on a wait) is not
        re-entered; only its timer fires. A continuation the game posted
        (`Task.Yield`, a `Task.Delay` that ran out) runs on the next frame, as
        it does under Godot's own context: with one waiting, a single frame
        passes for the timers, then it runs.
        """
        from experiment import call, prop
        if not bool(prop(ex, "IsRunning")):
            call(ex, "ExecuteActions")
        if self.posted is not None and self.posted.pending:
            if self.timers:
                n = self.advance(frames=1)
            else:
                n, self.frames = 0, self.frames + 1
            return n + self.posted.run_posted()
        return self.advance()

    def idle(self, seconds):
        """Real time, for a `Task.Delay` only: returns at once when something is already posted."""
        if self.posted is None:
            import time
            time.sleep(seconds)
        else:
            self.posted.idle(seconds)

    def stalled_on(self, what):
        """Stop the run: print `STALLED <report>` and exit 3, so no truncated trace is written.

        A probe step that timed out, a drain that ran out of pumps with timers
        still pending, a task that never completed, a wait made off the pump
        thread. `drive` raises on the line.
        """
        import json
        self.stalled = what
        print("STALLED " + json.dumps(self.report()), flush=True)
        raise Stalled(3)

    def _busy(self):
        return bool(self.timers) or (self.posted is not None and self.posted.pending)

    def drain(self, ex, limit=2000):
        """Pump until no timer is pending and nothing is posted (the end of a combat still has waits)."""
        from experiment import call
        n = 0
        while self._busy() and n < limit:
            call(ex, "Unpause")
            self.pump(ex)
            n += 1
        if self._busy():
            self.stalled_on(f"drain: {len(self.timers)} timers still pending after {limit} pumps")
        if self.off_thread_waits:
            self.stalled_on(f"{len(self.off_thread_waits)} waits were created off the pump thread")
        return n

    def wait(self, ex, task):
        """Block on a task the probe started itself, letting timers fire meanwhile."""
        from experiment import call
        import time
        t0 = time.time()
        while not bool(task.IsCompleted):
            call(ex, "Unpause")
            if not self.pump(ex):
                # Nothing to fire and nothing posted: only a `Task.Delay` can finish it.
                self.idle(0.01)
                if time.time() - t0 > WAIT_TIMEOUT_S:
                    self.stalled_on(f"wait: a task the probe started never completed ({WAIT_TIMEOUT_S:.0f} s)")
        call(call(task, "GetAwaiter"), "GetResult")

    def begin(self, fight):
        """Real waits from here on: the scenario's root. Setup before it ran headless."""
        self.enabled = True
        fight.waits = self

    def report(self):
        return dict(mode="real", fast_mode=self.fast_mode, fps=self.fps,
                    listener_yield=self.listener_yield, waits=self.count, frames=self.frames,
                    forced_yields=self.forced_yields, max_pending=self.max_pending,
                    concurrent=self.concurrent, hook_signals=self.hook_signals,
                    posts=self.posted.posts if self.posted is not None else None,
                    off_thread_waits=self.off_thread_waits, stalled=self.stalled,
                    pending_at_end=[dict(serial=t[4], seconds=round(t[2], 4), left=round(t[0], 4),
                                         site=t[3]) for t in self.timers])


def from_env():
    """The mode the probes run in: `STS2_WAITS` unset or `off` is the old behaviour.

    `STS2_WAITS=real[:normal|fast|instant][:always][:after]`, and
    `STS2_WAITS_FPS`. An environment variable because `drive` re-runs itself,
    one CLR process a run, and the child must be in the parent's mode.
    """
    import os
    spec = os.environ.get("STS2_WAITS", "off")
    if spec in ("", "off"):
        return OFF
    parts = spec.split(":")
    if parts[0] != "real":
        raise SystemExit(f"STS2_WAITS={spec!r}: expected off or real[:normal|fast|instant][:always][:after]")
    fast_mode = next((p for p in parts[1:] if p in FAST_MODES), "normal")
    yields = [p for p in ("always", "after") if p in parts[1:]]
    extra = [p for p in parts[1:] if p not in FAST_MODES and p not in ("always", "after")]
    if extra:
        raise SystemExit(f"STS2_WAITS={spec!r}: unknown part {extra}")
    return RealWaits(fast_mode=fast_mode, fps=float(os.environ.get("STS2_WAITS_FPS", "60")),
                     listener_yield="+".join(yields) or "timers")
