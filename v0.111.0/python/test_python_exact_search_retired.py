"""Static cutover guard: Python must not regain exact-search authority."""

from __future__ import annotations

import ast
import pathlib


HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[1]  # `sim/`, or the sts2-sim repo
VERSION = HERE.parent
RETIRED_TOP_LEVEL = frozenset({
    "SolveDeadline",
    "_beam_rollout",
    "_canon_key",
    "_legal_actions",
    "final_hp_ceiling",
    "greedy_rollout",
    "heal_potential",
    "outcome",
    "solve",
})


def _bound_names(target: ast.expr) -> set[str]:
    if isinstance(target, ast.Name):
        return {target.id}
    if isinstance(target, (ast.Tuple, ast.List)):
        return set().union(*(_bound_names(item) for item in target.elts))
    return set()


def _top_level_names(tree: ast.Module) -> set[str]:
    names = set()
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            names.add(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                names.update(_bound_names(target))
        elif isinstance(node, ast.AnnAssign):
            names.update(_bound_names(node.target))
    return names


def _retired_references(tree: ast.Module) -> list[int]:
    module_aliases = set()
    offenders = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for imported in node.names:
                if imported.name == "solve_fight":
                    module_aliases.add(imported.asname or imported.name)
        elif isinstance(node, ast.ImportFrom) and node.module == "solve_fight":
            for imported in node.names:
                if imported.name in RETIRED_TOP_LEVEL:
                    offenders.append(node.lineno)
    for node in ast.walk(tree):
        if (isinstance(node, ast.Attribute)
                and node.attr in RETIRED_TOP_LEVEL
                and isinstance(node.value, ast.Name)
                and node.value.id in module_aliases):
            offenders.append(node.lineno)
    return offenders


def test_python_exact_search_entry_points_are_absent():
    # #2827 item F hard-deleted the Python simulator: the module that once
    # held the retired entry points is gone, not merely emptied of them.
    for name in ("solve_fight.py", "combat_sim.py", "content"):
        assert not (HERE / name).exists(), name
    assert not (VERSION / "rust/tools/generate_exact_solve_corpus.py").exists()


def test_no_python_module_calls_the_retired_solver():
    offenders = []
    for path in sorted(REPO.rglob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        offenders.extend(
            f"{path.relative_to(REPO)}:{line}"
            for line in _retired_references(tree))
    assert not offenders, f"retired Python solver references remain: {offenders}"


def test_guard_detects_assignment_import_alias_and_callback_forms():
    assignment = ast.parse(
        "solve = lambda state: max(map(solve, state.children))\n")
    assert "solve" in _top_level_names(assignment)

    direct_callback = ast.parse(
        "from solve_fight import solve as search\nconsume(search)\n")
    assert _retired_references(direct_callback)

    module_callback = ast.parse(
        "import solve_fight as sf\nconsume(sf.solve)\n")
    assert _retired_references(module_callback)


def test_operator_and_review_paths_name_the_rust_boundary():
    # #2827 item F1: every review roots, searches and replays in Rust through
    # `rust_review.search_outcomes`, and the live coach solves its Rust root
    # through `rust_exact_solve.solve_document`. The Python-state
    # `solve_state` callers (the retired producer, `infoset_sample`) are gone.
    callers = {
        HERE / "rust_review.py": "def search_outcomes(",
        HERE / "rust_legacy_review.py": "rust_review.search_outcomes(",
        HERE / "tools/live_coach.py": "rust_exact_solve.solve_document(",
    }
    for path, call in callers.items():
        source = path.read_text(encoding="utf-8")
        assert call in source, path
    assert "def solve_state(" not in (
        HERE / "rust_exact_solve.py").read_text(encoding="utf-8")
