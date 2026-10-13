"""The parent half of a native probe's `drive`: run a child, read it, enumerate the answers (#3931).

Both native probes (`probe_relic_hook_choice.py` here and the Rust crate's
`tests/native/probe.py`) play one answer sequence a CLR process and explore
the options by trying index 0, 1, 2 … until the game refuses one. That makes
"this child gave no trace" the signal for "no such option", and it must mean
nothing else: a child that stalled, or threw anywhere but in the selector,
used to read the same way, and `drive` then wrote a scenario with runs
missing and exited 0. Everything here is plain Python with no CLR, so the
fast suite pins it (`test_harness_real_waits.py`).
"""
import json
import subprocess
import sys


def read_child(name, answers, lines, returncode):
    """What one probe child printed: its trace, or None for an answer past the last option.

    Anything else that is not a clean trace raises, so `drive` can never write
    a scenario with runs missing: a stalled run (`STALLED`, exit 3), an
    exception the game logged, or no `NAT` line at all. Two exceptions are
    expected. An index past the last option throws in the selector
    (`Enumerable.ElementAt`, reached from `TestCardSelector`), which is how
    `drive` finds the end of the options; the action then never finishes, so
    under real waits a `STALLED` line follows and is not the news. And the
    close of a combat throws in `EndCombatInternal`, which has no run or room
    to hand off to in the harness.
    """
    thrown = [i for i, line in enumerate(lines) if line.startswith("[GAME]") and "Exception" in line]
    for i in thrown:
        if "ArgumentOutOfRangeException" in lines[i] and any(
                "Enumerable.ElementAt" in later for later in lines[i + 1:i + 7]):
            return None
    stalled = next((line for line in lines if line.startswith("STALLED ")), None)
    if stalled is not None or returncode == 3:
        raise SystemExit(f"{name} {answers}: {stalled or 'exit 3'}")
    for i in thrown:
        if any("EndCombatInternal" in later for later in lines[i:i + 3]):
            continue
        raise SystemExit(f"{name} {answers}: the game threw, so the run is not a trace: "
                         + " | ".join(line.strip() for line in lines[i:i + 4]))
    trace = next((json.loads(line[4:]) for line in lines if line.startswith("NAT ")), None)
    if trace is None:
        raise SystemExit(f"{name} {answers}: no trace (exit {returncode}): " + " | ".join(lines[-4:]))
    return trace


def run_child(script, name, answers):
    """Play one answer sequence in its own process (`SCRIPT run NAME ANSWERS`) and read it."""
    proc = subprocess.run(
        [sys.executable, str(script), "run", name, ",".join(answers) or "-"],
        capture_output=True, text=True)
    return read_child(name, answers, (proc.stdout + proc.stderr).splitlines(), proc.returncode)


def explore(child):
    """Every answer sequence the game accepts, as `[{answers, trace}]`. `child(answers)` plays one."""
    runs = []

    def walk(prefix):
        for option in range(12):
            answers = prefix + [str(option)]
            trace = child(answers)
            if trace is None:
                # An index past the last option throws inside the selector.
                break
            if trace[-1]["label"] == "unanswered":
                walk(answers)
                continue
            parks = sum(":parked-" in entry["label"] for entry in trace)
            if parks < len(answers):
                # The last answer was never asked for: the prefix is a run.
                if option == 0:
                    runs.append(dict(answers=prefix, trace=trace))
                return
            runs.append(dict(answers=answers, trace=trace))

    walk([])
    return runs
