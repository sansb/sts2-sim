"""The live coach on the Rust root (#2827 item F1).

`tools/live_coach.py` used to build its root with the frozen Python
`start_combat`, solve a projection of it, and narrate by replaying the line
through Python. It now takes the review worker's root (`sts-sim entry --save
--opening`), solves that canonical document with one `exact-solve` request, and
replays the line in Rust before narrating it. These pin the disclosure, the
flags, and the end-to-end path on a real v0.111.0 save with no simulator
module importable.
"""

import importlib.abc
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "tools"))

import live_coach  # noqa: E402
import rust_exact_solve  # noqa: E402

BINARY = HERE.parent / "rust" / "target" / "release" / "sts-sim"
MAWLER_SAVE = HERE / "testdata" / "6P96T755CNZ3_mawler_entry.save"
SIMULATOR_MODULES = frozenset({
    "combat_sim", "solve_fight", "content", "mcr_replay", "project_state",
    "python_fight_context"})


def test_live_coach_reports_final_belt_not_negative_net_consumption():
    summary = live_coach.solution_summary(
        label="EXACT", elapsed=0.25, nodes=7, hp_entering=64,
        objective=(1, 60, 2, -3), observed_damage=9)

    assert "2 potion(s) kept" in summary
    assert "potion(s) used" not in summary
    assert "-1 potion" not in summary


def test_potion_caveat_declares_every_narrowing_the_root_carries():
    held = live_coach.potion_caveat({"player": {
        "inert_potions": ["SHIP_IN_A_BOTTLE"],
        "potion_slots": ["SHIP_IN_A_BOTTLE", None]}})
    assert "1 unmodeled potion(s) held (SHIP_IN_A_BOTTLE)" in held
    assert "generated mid-fight" in held
    assert "local pet" in held

    strict_empty = live_coach.potion_caveat({"player": {
        "strict_potions": True, "potion_slots": [None, None]}})
    assert strict_empty is None
    # Non-strict mode always discloses the generation condition, even with an
    # empty belt: over-disclosure, never under (I4).
    assert "generated mid-fight" in live_coach.potion_caveat({"player": {}})


def _fake_session(monkeypatch, root, argv):
    save = {"rng": {"seed": "SEED", "counters": {"shuffle": 3}}}
    monkeypatch.setattr(live_coach, "load_save", lambda _path: save)
    monkeypatch.setattr(live_coach, "verify_stream_seeding",
                        lambda *_args: [])
    monkeypatch.setattr(live_coach, "node_type", lambda _save: "elite")
    monkeypatch.setattr(live_coach, "next_encounter",
                        lambda *_args: "ENCOUNTER.TEST")
    monkeypatch.setattr(live_coach, "build_entry", lambda *_args: {
        "node_index": 1, "hp_entering": 50, "max_hp_entering": 60,
        "deck_entering": [], "potions_entering": []})
    monkeypatch.setattr(rust_exact_solve, "default_binary",
                        lambda: pathlib.Path("/fake/sts-sim"))
    opened = []
    monkeypatch.setattr(
        live_coach, "rust_opening",
        lambda binary, path, build, encounter, ntype: opened.append(
            (binary, path, build, encounter, ntype)) or root)
    monkeypatch.setattr(sys, "argv", ["live_coach.py", *argv])
    return opened


def test_dry_run_roots_in_rust_and_discloses_the_caveat(monkeypatch, capsys):
    root = {"player": {"inert_potions": ["X_POTION"]},
            "piles": {"hand": [{"id": "BASH", "uid": 0}]}}
    opened = _fake_session(monkeypatch, root,
                           ["fake.save", "--dry", "--build", "v0.111.0"])
    monkeypatch.setattr(
        rust_exact_solve, "solve_document",
        lambda *a, **k: pytest.fail("a dry run must not solve"))
    live_coach.main()
    out = capsys.readouterr().out
    assert opened == [(pathlib.Path("/fake/sts-sim"), pathlib.Path("fake.save"),
                       "v0.111.0", "ENCOUNTER.TEST", "elite")]
    assert "! CAVEAT: 1 unmodeled potion(s) held (X_POTION)" in out
    assert "turn-1 hand: ['BASH']" in out
    assert "dry run ok" in out


def test_strict_flag_refuses_a_held_unmodeled_potion(monkeypatch):
    root = {"player": {"inert_potions": ["X_POTION"]}, "piles": {}}
    _fake_session(monkeypatch, root, [
        "fake.save", "--dry", "--build", "v0.111.0", "--no-unmodeled-potions"])
    with pytest.raises(NotImplementedError, match="X_POTION"):
        live_coach.main()


def test_strict_flag_marks_the_root_strict(monkeypatch, capsys):
    root = {"player": {}, "piles": {}}
    _fake_session(monkeypatch, root, [
        "fake.save", "--dry", "--build", "v0.111.0", "--no-unmodeled-potions"])
    live_coach.main()
    assert root["player"]["strict_potions"] is True
    assert "CAVEAT" not in capsys.readouterr().out


def test_retired_python_flags_still_refuse(monkeypatch):
    _fake_session(monkeypatch, {"player": {}, "piles": {}},
                  ["fake.save", "--build", "v0.111.0", "--no-rollout"])
    with pytest.raises(NotImplementedError, match="--no-rollout"):
        live_coach.main()


def test_a_solved_line_is_replayed_in_rust_before_it_is_narrated(
        monkeypatch, capsys):
    root = {"schema": "sts-sim-canonical-v2", "player": {}, "piles": {}}
    _fake_session(monkeypatch, root, [
        "fake.save", "--build", "v0.111.0", "--horizon", "4",
        "--deadline", "2", "--alpha", "7"])
    solution = rust_exact_solve.RustExactSolution(
        ({"kind": "end"},), (1, 41, 0, -2), "digest")
    requests = []
    monkeypatch.setattr(
        rust_exact_solve, "solve_document",
        lambda entry, **kwargs: requests.append((entry, kwargs)) or
        rust_exact_solve.RustExactResult("exact", solution, 11, 10))
    replayed = []
    monkeypatch.setattr(
        live_coach, "narrate_rust_line",
        lambda binary, entry, actions, digest: replayed.append(
            (entry, actions, digest)) or ["  end turn  => hp 41"])
    live_coach.main()
    out = capsys.readouterr().out
    assert requests == [(root, {"horizon": 4, "deadline": 2.0,
                                "binary": pathlib.Path("/fake/sts-sim"),
                                "alpha_final_hp": 7})]
    assert replayed == [(root, ({"kind": "end"},), "digest")]
    assert "EXACT" in out and "win with 9 HP lost (41 remaining)" in out
    assert "  end turn  => hp 41" in out


def test_the_live_coach_imports_no_simulator_module():
    import ast
    tree = ast.parse((HERE / "tools" / "live_coach.py").read_text())
    imported = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            imported.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            imported.add(node.module.split(".")[0])
    assert not imported & SIMULATOR_MODULES, imported & SIMULATOR_MODULES


@pytest.mark.skipif(not BINARY.exists(),
                    reason="build versions/v0.111.0/rust/target/release/sts-sim")
def test_real_save_coaches_end_to_end_without_the_simulator(
        monkeypatch, capsys):
    """A real v0.111.0 Mawler entry save: Rust root, Rust solve, Rust replay."""

    class Refuse(importlib.abc.MetaPathFinder):
        def find_spec(self, name, path=None, target=None):
            if name.split(".")[0] in SIMULATOR_MODULES:
                raise ImportError(f"the live coach imported {name}")
            return None

    for name in list(sys.modules):
        if name.split(".")[0] in SIMULATOR_MODULES:
            monkeypatch.delitem(sys.modules, name)
    monkeypatch.setattr(sys, "meta_path", [Refuse(), *sys.meta_path])
    monkeypatch.setenv("STS_SIM_EXACT_SOLVER", str(BINARY))
    monkeypatch.setattr(sys, "argv", [
        "live_coach.py", str(MAWLER_SAVE), "--build", "v0.111.0",
        "--horizon", "12", "--deadline", "1"])
    live_coach.main()
    out = capsys.readouterr().out
    assert "vs ENCOUNTER.MAWLER_NORMAL" in out
    assert "win with" in out
    assert "turn 1: hp 45" in out
    assert out.count("! CAVEAT:") == 2


def test_narration_rejects_a_line_that_misses_its_claimed_digest(monkeypatch):
    import rust_replay

    class Session:
        def __init__(self, binary, entry):
            self.state = entry

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

        def ask(self, request):
            return {"digest": "x"} if request["cmd"] == "apply" else {
                "state": self.state}

    monkeypatch.setattr(rust_replay, "RustReplay", Session)
    root = {"player": {"turn": 1, "hp": 5}, "piles": {"hand": []},
            "monsters": []}
    with pytest.raises(NotImplementedError, match="claimed digest"):
        live_coach.narrate_rust_line(
            pathlib.Path("/fake"), root, ({"kind": "end"},), "not-the-digest")
