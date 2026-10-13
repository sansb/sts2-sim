"""The real-waits clock of the native probes, without the game (#3931).

`harness/real_waits.py` makes `Cmd.Wait` a timer when a probe is run with
`STS2_WAITS=real`. Patching the game needs the CLR and the DLL, which no CI
lane has, so what is pinned here is the part that is plain Python: the mode
string, the game's own gates on a wait (against a stub of
`System.Threading.Tasks`), the frame clock that decides which timer runs out
first, the concurrency report, the stall paths, how `drive` reads a child
process, and that the default is the old behaviour. The module must import
with no CLR in the process: the probes import it before they boot one.
"""
import importlib.util
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"


def _load(name):
    spec = importlib.util.spec_from_file_location(f"{name}_under_test", HARNESS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _module():
    return _load("real_waits")


class _Tcs:
    """Stands in for a TaskCompletionSource: records the order timers complete in."""

    def __init__(self, log, name, then=None):
        self.log, self.name, self.then = log, name, then

    def SetResult(self):
        self.log.append(self.name)
        if self.then:
            self.then()


def _add(waits, log, name, seconds, site=("Some.Flow",), then=None):
    waits.count += 1
    waits.timers.append([seconds, _Tcs(log, name, then), seconds, list(site), waits.count])


def test_module_imports_without_a_clr():
    module = _module()
    assert module.OFF.active is False
    assert module.OFF.report() is None
    assert module.OFF.posted is None, "one thread only once a probe installs it"


def test_default_mode_is_off(monkeypatch):
    module = _module()
    monkeypatch.delenv("STS2_WAITS", raising=False)
    assert module.from_env() is module.OFF
    monkeypatch.setenv("STS2_WAITS", "off")
    assert module.from_env() is module.OFF


@pytest.mark.parametrize("spec, fast_mode, listener_yield", [
    ("real", "normal", "timers"),
    ("real:fast", "fast", "timers"),
    ("real:instant:always", "instant", "always"),
    ("real:always", "normal", "always"),
    ("real:after", "normal", "after"),
    ("real:fast:after:always", "fast", "always+after"),
])
def test_mode_string(monkeypatch, spec, fast_mode, listener_yield):
    module = _module()
    monkeypatch.setenv("STS2_WAITS", spec)
    monkeypatch.setenv("STS2_WAITS_FPS", "30")
    waits = module.from_env()
    assert (waits.fast_mode, waits.listener_yield, waits.fps) == (fast_mode, listener_yield, 30.0)
    assert waits.enabled is False, "real waits start at the scenario's root, not at boot"


@pytest.mark.parametrize("spec", ["on", "real:slow", "fast"])
def test_bad_mode_string_is_refused(monkeypatch, spec):
    module = _module()
    monkeypatch.setenv("STS2_WAITS", spec)
    with pytest.raises(SystemExit):
        module.from_env()


# ---- the game's own gates, with no CLR: `_wait` and `_scaled` ---------------
class _Done:
    """Stands in for `Task.CompletedTask`."""


class _Pending:
    def __init__(self):
        self.Task = self


def _stub_clr(monkeypatch):
    """`System.Threading.Tasks` and `System.Environment`, as far as the gates use them."""
    import sys
    import types
    system = types.ModuleType("System")
    system.Environment = types.SimpleNamespace(StackTrace="", CurrentManagedThreadId=7)
    threading = types.ModuleType("System.Threading")
    tasks = types.ModuleType("System.Threading.Tasks")
    tasks.Task = types.SimpleNamespace(CompletedTask=_Done)
    tasks.TaskCompletionSource = _Pending
    system.Threading, threading.Tasks = threading, tasks
    for name, module in (("System", system), ("System.Threading", threading),
                         ("System.Threading.Tasks", tasks)):
        monkeypatch.setitem(sys.modules, name, module)


def _gated(monkeypatch, fast_mode, ending=False):
    module = _module()
    _stub_clr(monkeypatch)
    waits = module.RealWaits(fast_mode=fast_mode)
    waits.enabled = True
    waits._combat_ending = lambda: ending
    return waits


def _seconds(waits):
    return [t[2] for t in waits.timers]


@pytest.mark.parametrize("fast_mode", ["normal", "fast"])
def test_a_wait_the_game_would_take_becomes_a_timer_of_its_length(monkeypatch, fast_mode):
    waits = _gated(monkeypatch, fast_mode)
    assert waits._wait(0.25, False) is not _Done
    assert _seconds(waits) == [0.25], "Cmd.Wait takes its seconds as given in Normal and in Fast"


def test_instant_fast_mode_takes_no_wait_at_all(monkeypatch):
    """`<Wait>d__1` IL_0037-0050 and `<CustomScaledWait>d__3` IL_002c-003e."""
    waits = _gated(monkeypatch, "instant")
    assert waits._wait(0.25, False) is _Done
    assert waits._wait(0.25, True) is _Done
    assert waits._scaled(0.1, 0.3, False) is _Done
    assert waits.timers == [] and waits.count == 0


def test_a_wait_of_no_length_and_a_wait_before_the_root_take_no_time(monkeypatch):
    waits = _gated(monkeypatch, "normal")
    assert waits._wait(0.0, False) is _Done and waits._wait(-1.0, True) is _Done
    waits.enabled = False
    assert waits._wait(0.25, False) is _Done and waits._scaled(0.1, 0.3, False) is _Done
    assert waits.timers == []


def test_the_scaled_wait_takes_the_fast_seconds_only_in_fast_mode(monkeypatch):
    """`<CustomScaledWait>d__3` IL_006d: 1 is the Standard seconds, 2 the Fast ones."""
    normal = _gated(monkeypatch, "normal")
    normal._scaled(0.1, 0.3, False)
    assert _seconds(normal) == [0.3]
    fast = _gated(monkeypatch, "fast")
    fast._scaled(0.1, 0.3, False)
    assert _seconds(fast) == [0.1]


def test_a_combat_that_is_ending_skips_the_wait_unless_told_to_ignore_it(monkeypatch):
    """`<Wait>d__1` IL_0055-0069: `ignoreCombatEnd` true is the one that still waits."""
    ending = _gated(monkeypatch, "normal", ending=True)
    assert ending._wait(0.25, False) is _Done
    assert ending._scaled(0.1, 0.3, False) is _Done
    assert ending.timers == []
    assert ending._wait(0.25, True) is not _Done
    assert ending._scaled(0.1, 0.3, True) is not _Done
    assert _seconds(ending) == [0.25, 0.3]
    live = _gated(monkeypatch, "normal", ending=False)
    assert live._wait(0.25, False) is not _Done and _seconds(live) == [0.25]


def test_the_fast_mode_is_the_number_the_game_compares_with():
    """`Cmd.Wait` IL_004d compares with 3, `DoTurnEndCards` IL_00ed with 2."""
    module = _module()
    assert module.FAST_MODE_VALUE == {"normal": 1, "fast": 2, "instant": 3}


# ---- the clock -------------------------------------------------------------
def test_shorter_timer_fires_first_whatever_the_creation_order():
    module = _module()
    waits = module.RealWaits()
    log = []
    _add(waits, log, "long", 0.4)
    _add(waits, log, "short", 0.3)
    assert waits.advance() == 1 and log == ["short"]
    assert waits.frames == 19, "0.3 s loses 1/60 a frame and fires once below zero"
    assert waits.advance() == 1 and log == ["short", "long"]
    assert waits.advance() == 0
    report = waits.report()
    assert report["max_pending"] == 2
    assert [t["seconds"] for t in report["concurrent"][0]["timers"]] == [0.4, 0.3]


def test_timers_that_run_out_in_one_frame_fire_in_creation_order():
    module = _module()
    waits = module.RealWaits()
    log = []
    _add(waits, log, "first", 0.25)
    _add(waits, log, "second", 0.25)
    assert waits.advance() == 2 and log == ["first", "second"]


def test_a_timer_made_while_another_fires_waits_for_a_later_frame():
    module = _module()
    waits = module.RealWaits()
    log = []
    _add(waits, log, "outer", 0.1, then=lambda: _add(waits, log, "inner", 0.0001))
    assert waits.advance() == 1 and log == ["outer"]
    assert len(waits.timers) == 1
    assert waits.advance() == 1 and log == ["outer", "inner"]


def test_one_timer_at_a_time_is_not_reported_as_concurrent():
    module = _module()
    waits = module.RealWaits(fast_mode="fast")
    log = []
    for name in ("a", "b", "c"):
        _add(waits, log, name, 0.2)
        waits.advance()
    report = waits.report()
    assert report["max_pending"] == 1 and report["concurrent"] == []
    assert report["mode"] == "real" and report["fast_mode"] == "fast" and report["waits"] == 3


def test_a_relic_flash_beside_a_gameplay_wait_is_not_a_race():
    module = _module()
    waits = module.RealWaits()
    log = []
    _add(waits, log, "flash", 1.0, site=("IronClub.DoActivateVisuals",))
    _add(waits, log, "draw", 0.2, site=("CardPileCmd.Draw",))
    waits.advance()
    assert waits.report()["max_pending"] == 1 and waits.report()["concurrent"] == []


def test_the_harness_probe_takes_the_mode_from_the_environment_only():
    """`drive` re-runs itself a process a run; the child must be in the parent's mode."""
    text = (HARNESS / "probe_relic_hook_choice.py").read_text()
    assert "waits = real_waits.from_env()" in text
    assert 'call(f.ex, "ExecuteActions")' not in text, "a pump that real waits cannot see"
    assert "time.sleep" not in text, "the game runs on the probe's thread: nothing to sleep for"


# ---- one thread ------------------------------------------------------------
class _FakeExecutor:
    IsRunning = True


def _fake_experiment(monkeypatch):
    """`pump`, `drain` and `wait` reach the game through two helpers; stand in for them."""
    import sys
    import types
    fake = types.ModuleType("experiment")
    fake.call = lambda obj, name, *args: None
    fake.prop = lambda obj, name: getattr(obj, name)
    monkeypatch.setitem(sys.modules, "experiment", fake)


class _Posted:
    """Stands in for `one_thread.PumpContext`: one continuation waiting to run."""

    def __init__(self, log):
        self.log, self.pending, self.posts = log, True, 1

    def run_posted(self):
        self.log.append("posted")
        self.pending = False
        return 1

    def idle(self, seconds):
        self.log.append("idle")


def test_a_posted_continuation_takes_one_frame_and_runs_after_that_frames_timers(monkeypatch):
    """`Task.Yield` under a context resumes a frame later, not when the next timer runs out."""
    module = _module()
    _fake_experiment(monkeypatch)
    waits = module.RealWaits()
    log = []
    waits.posted = _Posted(log)
    _add(waits, log, "long", 0.4)
    _add(waits, log, "this frame", 0.01)
    _add(waits, log, "next frame", waits.delta)   # at zero after one frame: fires below it, a frame on
    assert waits.pump(_FakeExecutor()) == 2
    assert log == ["this frame", "posted"] and waits.frames == 1
    assert waits.pump(_FakeExecutor()) == 1 and log[-1] == "next frame" and waits.frames == 2
    waits.posted.pending = True
    assert waits.pump(_FakeExecutor()) == 1 and log[-1] == "posted", "one frame, not a jump to the long timer"
    assert waits.frames == 3
    assert waits.pump(_FakeExecutor()) == 1 and log[-1] == "long"


def test_a_drain_runs_what_was_posted_even_with_no_timer_pending(monkeypatch):
    module = _module()
    _fake_experiment(monkeypatch)
    waits = module.RealWaits()
    log = []
    waits.posted = _Posted(log)
    assert waits.drain(_FakeExecutor()) == 1 and log == ["posted"]


def test_headless_pumps_run_what_was_posted_and_idle_does_not_sleep(monkeypatch):
    module = _module()
    _fake_experiment(monkeypatch)
    monkeypatch.setattr("time.sleep", lambda _s: pytest.fail("a sleep with the game on one thread"))
    off = module.Off()
    log = []
    off.posted = _Posted(log)
    assert off.pump(_FakeExecutor()) == 1
    off.idle(0.02)
    assert log == ["posted", "idle"]


def test_a_wait_made_off_the_pump_thread_is_reported_and_refuses_the_run(monkeypatch, capsys):
    import json
    module = _module()
    _stub_clr(monkeypatch)
    _fake_experiment(monkeypatch)
    waits = module.RealWaits()
    waits.enabled = True
    waits._combat_ending = lambda: False
    waits.pump_thread = 7
    waits._wait(0.2, False)
    assert waits.off_thread_waits == []
    waits.pump_thread = 1                     # the stub's thread is 7: this wait is off it
    waits._wait(0.2, False)
    assert [w["serial"] for w in waits.off_thread_waits] == [2]
    for t in waits.timers:
        t[1].SetResult = lambda: None
    with pytest.raises(module.Stalled):
        waits.drain(_FakeExecutor())
    report = json.loads(capsys.readouterr().out.split("STALLED ", 1)[1])
    assert report["stalled"] == "1 waits were created off the pump thread"


# ---- a run that stops making progress is reported, never written as a trace ----
def test_a_drain_that_runs_out_of_pumps_stalls_with_the_pending_timers(monkeypatch, capsys):
    import json
    module = _module()
    _fake_experiment(monkeypatch)
    waits = module.RealWaits()
    log = []

    def again():
        _add(waits, log, "again", 0.1, site=("Loop.Forever",), then=again)

    again()
    with pytest.raises(module.Stalled) as stop:
        waits.drain(_FakeExecutor(), limit=5)
    assert stop.value.code == 3
    line = next(l for l in capsys.readouterr().out.splitlines() if l.startswith("STALLED "))
    report = json.loads(line[len("STALLED "):])
    assert report["stalled"].startswith("drain: 1 timers still pending after 5 pumps")
    assert [t["site"] for t in report["pending_at_end"]] == [["Loop.Forever"]]


def test_a_task_that_never_completes_stalls_instead_of_blocking(monkeypatch, capsys):
    module = _module()
    _fake_experiment(monkeypatch)
    monkeypatch.setattr("time.sleep", lambda _s: None)
    monkeypatch.setattr(module, "WAIT_TIMEOUT_S", -1.0)

    class _Task:
        IsCompleted = False

    waits = module.RealWaits()
    with pytest.raises(module.Stalled):
        waits.wait(_FakeExecutor(), _Task())
    assert "STALLED " in capsys.readouterr().out
    assert waits.report()["stalled"].startswith("wait: a task the probe started never completed")


def test_a_step_timeout_is_a_stall_in_real_waits_and_nothing_new_headless(capsys):
    module = _module()
    assert module.OFF.stalled_on("step timed out") is None, "headless keeps returning its timeout state"
    waits = module.RealWaits()
    with pytest.raises(module.Stalled):
        waits.stalled_on("step timed out with the action in state Executing")
    assert '"stalled": "step timed out' in capsys.readouterr().out


def test_a_clean_run_reports_no_stall_and_nothing_pending(monkeypatch):
    module = _module()
    _fake_experiment(monkeypatch)
    monkeypatch.setattr("time.sleep", lambda _s: None)
    waits = module.RealWaits()
    log = []
    _add(waits, log, "only", 0.2)
    assert waits.drain(_FakeExecutor()) == 1
    report = waits.report()
    assert report["stalled"] is None and report["pending_at_end"] == []
    assert report["off_thread_waits"] == [] and report["hook_signals"] == []


def test_the_turn_end_of_the_kit_stalls_when_it_runs_out_of_time_under_real_waits():
    """`_Fight.end_turn` used to return after its 30 s with no stall and no sign of it."""
    text = (HARNESS / "experiment.py").read_text()
    body = text[text.index("    def end_turn(self"):text.index("    def history(self")]
    assert "self.waits.stalled_on(" in body and "the turn end timed out" in body
    assert "time.sleep" not in body


# ---- `drive`: a child that is not a clean trace is never "no such option" ----
NAT = 'NAT [{"label": "root"}, {"label": "play:done"}]'
SELECTOR = ["[GAME] System.ArgumentOutOfRangeException: Specified argument was out of the range of valid values.",
            "   at System.Linq.ThrowHelper.ThrowArgumentOutOfRangeException(ExceptionArgument argument)",
            "   at System.Linq.Enumerable.ElementAt[TSource](IEnumerable`1 source, Int32 index)"]
EXECUTOR = ["[GAME] System.InvalidOperationException: An attempt was made to transition a task to a final state",
            "   at System.Threading.Tasks.TaskCompletionSource`1.SetException(Exception exception)",
            "   at MegaCrit.Sts2.Core.GameActions.ActionExecutor.ExecuteActions()"]
COMBAT_END = ["[GAME] System.NullReferenceException: Object reference not set to an instance of an object.",
              "   at MegaCrit.Sts2.Core.Combat.CombatManager.EndCombatInternal(CombatTurnState turnState)"]


def test_a_clean_child_is_its_trace_and_a_combat_that_closed_is_clean():
    drive = _load("probe_drive")
    assert drive.read_child("s", ["0"], [NAT], 0) == [{"label": "root"}, {"label": "play:done"}]
    assert drive.read_child("s", ["0"], COMBAT_END + [NAT], 0)[-1]["label"] == "play:done"


def test_an_answer_past_the_last_option_is_none_even_when_the_run_then_stalls():
    drive = _load("probe_drive")
    assert drive.read_child("s", ["9"], SELECTOR + [NAT], 0) is None
    assert drive.read_child("s", ["9"], SELECTOR + ['STALLED {"stalled": "step timed out"}'], 3) is None


@pytest.mark.parametrize("lines, returncode, says", [
    (['STALLED {"stalled": "drain: 1 timers still pending"}'], 3, "STALLED"),
    ([], 3, "['0']: exit 3"),
    ([NAT], 3, "['0']: exit 3"),
    ([NAT, 'STALLED {"stalled": "x"}'], 0, "STALLED"),
    (EXECUTOR + [NAT], 0, "the game threw"),
    (EXECUTOR, 0, "the game threw"),
    (["Traceback (most recent call last):", "AssertionError: opening turn never reached Play"], 1, "no trace"),
    ([], 0, "no trace"),
])
def test_a_child_that_stalled_threw_or_printed_nothing_raises(lines, returncode, says):
    drive = _load("probe_drive")
    with pytest.raises(SystemExit) as stop:
        drive.read_child("scenario", ["0"], lines, returncode)
    assert says in str(stop.value.code) and "scenario" in str(stop.value.code)


def _probe_with_children(monkeypatch, children):
    """The relic probe itself, its child processes replaced by `children[answers] -> (lines, returncode)`."""
    import subprocess
    import sys
    import types
    monkeypatch.setenv("STS2_HARNESS_DIR", str(HARNESS))
    monkeypatch.delitem(sys.modules, "probe_drive", raising=False)
    monkeypatch.setattr(sys, "path", list(sys.path))
    probe = _load("probe_relic_hook_choice")
    asked = []

    def run(argv, capture_output, text):
        assert argv[2] == "run" and argv[3] == "some_scenario"
        asked.append(argv[4])
        lines, returncode = children[argv[4]]
        return types.SimpleNamespace(stdout="\n".join(lines) + "\n", stderr="", returncode=returncode)

    monkeypatch.setattr(subprocess, "run", run)
    monkeypatch.setattr(probe, "scenarios", lambda: {"some_scenario": {"script": []}})
    return probe, asked


def _parked(answer):
    return 'NAT [{"label": "play:parked-0"}, {"label": "play:done", "answer": "%s"}]' % answer


def test_drive_enumerates_the_options_up_to_the_one_the_selector_refuses(monkeypatch):
    probe, asked = _probe_with_children(monkeypatch, {
        "0": ([_parked(0)], 0), "1": (COMBAT_END + [_parked(1)], 0), "2": (SELECTOR + [NAT], 0)})
    out = probe.drive("some_scenario")
    assert [run["answers"] for run in out["runs"]] == [["0"], ["1"]]
    assert asked == ["0", "1", "2"]


@pytest.mark.parametrize("second, says", [
    ((['STALLED {"stalled": "step timed out"}'], 3), "STALLED"),
    (([], 3), "exit 3"),
    ((EXECUTOR + [_parked(1)], 0), "the game threw"),
    (([], 1), "no trace"),
])
def test_drive_refuses_a_child_that_stalled_or_threw(monkeypatch, second, says):
    """Read as "no such option", any of these left `drive` writing the scenario with runs missing."""
    probe, asked = _probe_with_children(monkeypatch, {"0": ([_parked(0)], 0), "1": second})
    with pytest.raises(SystemExit) as stop:
        probe.drive("some_scenario")
    assert says in str(stop.value.code) and asked == ["0", "1"]


def test_both_probes_hand_the_parent_half_of_drive_to_the_shared_module():
    """The crate's probe is outside what this suite may read; the harness one is checked here.

    Both hold the same three functions (`_drive`, `_run_child`, `drive`), whose
    only logic is the calls into `probe_drive` exercised above.
    """
    text = (HARNESS / "probe_relic_hook_choice.py").read_text()
    body = text[text.index("def _drive():"):text.index("def main(argv):")]
    assert "subprocess" not in body and "Exception" not in body
    assert "_drive().run_child(Path(__file__).resolve(), name, answers)" in body
    assert "_drive().explore(lambda answers: _run_child(name, answers))" in body
