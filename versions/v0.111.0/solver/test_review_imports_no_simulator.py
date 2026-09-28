"""Production review, replay, Coach and worker code imports no simulator (#2827 F1).

Each production module is imported in a fresh interpreter in which the frozen
Python simulator (`combat_sim`, `solve_fight`, `content`) and its adapters
(`mcr_replay`, `project_state`, `python_fight_context`) cannot be imported at
all. A module that reaches one of them, directly or through anything it
imports at load time, fails here by name. This is the precondition for
deleting the simulator (#2827 item F); the lazy imports inside functions are
exercised by the end-to-end tests (`test_rust_legacy_review.py`'s real-binary
review, `test_live_coach.py`, `test_relay_coach.py`) under the same ban.
"""

import json
import pathlib
import subprocess
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
BANNED = ("combat_sim", "solve_fight", "content", "mcr_replay",
          "project_state", "python_fight_context")

PRODUCTION = (
    # The review CLI the worker runs, and everything it roots, searches and
    # replays through.
    "versions/v0.111.0/solver/review_summary_v2.py",
    "versions/v0.111.0/solver/review_summary.py",
    "versions/v0.111.0/solver/rust_review.py",
    "versions/v0.111.0/solver/rust_legacy_review.py",
    "versions/v0.111.0/solver/rust_replay.py",
    "versions/v0.111.0/solver/rust_exact_solve.py",
    "versions/v0.111.0/solver/rust_run_counters.py",
    "versions/v0.111.0/solver/review_provenance.py",
    "versions/v0.111.0/solver/admission.py",
    "versions/v0.111.0/solver/sim_identity.py",
    "versions/v0.111.0/solver/mcr_native.py",
    "versions/v0.111.0/solver/mcr_parser.py",
    "versions/v0.111.0/solver/relay_parser.py",
    "versions/v0.111.0/solver/tools/live_coach.py",
    "versions/v0.111.0/rust/tools/canonical_document.py",
    # The review worker, the Coach and their tools.
)

_PROBE = r'''
import importlib.abc, importlib.util, json, os, sys
banned = set(json.loads(sys.argv[2]))
class Refuse(importlib.abc.MetaPathFinder):
    def find_spec(self, name, path=None, target=None):
        if name.split(".")[0] in banned:
            raise ImportError("simulator module imported: " + name)
        return None
sys.meta_path.insert(0, Refuse())
path = os.path.abspath(sys.argv[1])
sys.path.insert(0, os.path.dirname(path))
name = os.path.splitext(os.path.basename(path))[0]
spec = importlib.util.spec_from_file_location(name, path)
module = importlib.util.module_from_spec(spec)
sys.modules[name] = module
spec.loader.exec_module(module)
print(json.dumps(sorted(m for m in sys.modules if m.split(".")[0] in banned)))
'''


@pytest.mark.parametrize("relative", PRODUCTION)
def test_production_module_imports_without_the_simulator(relative):
    probe = subprocess.run(
        [sys.executable, "-c", _PROBE, str(REPO / relative),
         json.dumps(BANNED)],
        capture_output=True, text=True, cwd=str(REPO), check=False)
    assert probe.returncode == 0, probe.stderr[-2000:]
    assert json.loads(probe.stdout.splitlines()[-1]) == []


def test_the_review_cli_answers_its_identity_without_the_simulator():
    """What the worker asks the bundle first (`review_worker.py`)."""
    probe = subprocess.run(
        [sys.executable, "-c",
         _PROBE.replace('print(json.dumps(sorted(m for m in sys.modules if '
                        'm.split(".")[0] in banned)))',
                        "sys.exit(module.main(['--simulator-identity']))"),
         str(HERE / "review_summary_v2.py"), json.dumps(BANNED)],
        capture_output=True, text=True, cwd=str(REPO), check=False)
    assert probe.returncode == 0, probe.stderr[-2000:]
    identity = json.loads(probe.stdout)
    assert identity["schema_version"] == 8
    assert len(identity["simulator"]["bundle_digest"]) == 64


def test_the_frozen_python_root_is_kept_out_of_production():
    """No production module names a simulator module in any import statement.

    The load-time probe above cannot see an import inside a function body;
    this reads every one (`python_fight_context` is the simulator's own test
    surface, not a producer).
    """
    import ast
    for relative in PRODUCTION:
        tree = ast.parse((REPO / relative).read_text(encoding="utf-8"))
        imported = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                imported.update(a.name.split(".")[0] for a in node.names)
            elif isinstance(node, ast.ImportFrom) and node.module:
                imported.add(node.module.split(".")[0])
        assert not imported & set(BANNED), (relative, imported & set(BANNED))
