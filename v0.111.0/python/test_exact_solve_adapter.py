"""Fast contracts for the exact-solve adapter (`rust_exact_solve`).

The wire, parser, transport and typed-refusal contracts of the one
Python-to-Rust exact-search crossing; SOLVER_INVARIANTS.md I4 names this
module as their guard. They lived in `test_review_summary.py` until the
Python review path was deleted (2026-10-11); the adapter stays because the
live coach solves through it.
"""

import json
import pathlib
import subprocess
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import rust_exact_solve  # noqa: E402


CANONICAL = {"schema": "sts-sim-canonical-v2", "player": {"hp": 60}}


def test_rust_response_parser_rejects_contradictory_or_unknown_messages():
    def response(**updates):
        base = {
            "protocol": rust_exact_solve.PROTOCOL_V1,
            "status": "exact",
            "solution": None,
            "refusal": None,
            "telemetry": {"nodes": 0, "transitions": 0},
        }
        base.update(updates)
        return base

    invalid = (
        response(refusal={"code": "invalid_seed", "detail": "contradiction"}),
        response(status="refused", solution={
            "actions": [], "objective": [1, 1, 0, -1], "final_digest": "x"},
            refusal={"code": "invalid_seed", "detail": "contradiction"}),
        response(status="refused", refusal={
            "code": "future_refusal", "detail": "unknown"}),
        response(solution={
            "actions": [], "objective": [0, 0, 0, 0], "final_digest": "x"}),
    )
    for wire in invalid:
        with pytest.raises(rust_exact_solve.RustExactSolveError):
            rust_exact_solve._parse_response(wire)


@pytest.mark.parametrize("code", (
    "invalid_seed", "malformed_request", "unsupported_protocol",
    "engine_transition_refused", "memo_projection_refused",
))
def test_rust_typed_refusals_are_raised_by_their_stable_code(code):
    with pytest.raises(rust_exact_solve.RustExactSolveRefusal) as raised:
        rust_exact_solve._parse_response({
            "protocol": rust_exact_solve.PROTOCOL_V1, "status": "refused",
            "solution": None, "refusal": {"code": code, "detail": "d"},
            "telemetry": {"nodes": 0, "transitions": 0}})
    assert raised.value.code == code


def test_winning_solution_parses_into_the_typed_result():
    result = rust_exact_solve._parse_response({
        "protocol": rust_exact_solve.PROTOCOL_V1, "status": "deadline",
        "solution": {"actions": [{"kind": "end"}],
                     "objective": [1, 44, 2, -3], "final_digest": "abc"},
        "refusal": None, "telemetry": {"nodes": 9, "transitions": 8}})
    assert result.incomplete and not result.exact
    assert result.solution.actions == ({"kind": "end"},)
    assert result.solution.objective == (1, 44, 2, -3)
    assert (result.nodes, result.transitions) == (9, 8)


def test_document_request_carries_alpha_horizon_and_no_python_seed(
        monkeypatch, tmp_path):
    seen = []

    def fake_run(argv, **kwargs):
        seen.append((argv, json.loads(kwargs["input"])))
        return subprocess.CompletedProcess(argv, 0, stdout=json.dumps({
            "protocol": rust_exact_solve.PROTOCOL_V1, "status": "exact",
            "solution": None, "refusal": None,
            "telemetry": {"nodes": 3, "transitions": 2}}), stderr="")

    monkeypatch.setattr(rust_exact_solve.subprocess, "run", fake_run)
    result = rust_exact_solve.solve_document(
        CANONICAL, horizon=7, deadline=None, binary=tmp_path / "sts-sim",
        alpha_final_hp=31)
    assert result.exact and result.solution is None
    ((argv, request),) = seen
    assert argv == [str(tmp_path / "sts-sim"), "exact-solve"]
    assert request == {
        "protocol": rust_exact_solve.PROTOCOL_V1, "entry": CANONICAL,
        "max_turns": 7, "alpha_final_hp": 31, "memo": True}


@pytest.mark.parametrize("entry,kwargs,match", (
    ({"player": {}}, {}, "canonical-v2 document"),
    (CANONICAL, {"horizon": -1}, "horizon is invalid"),
    (CANONICAL, {"alpha_final_hp": -2}, "alpha final HP is invalid"),
    (CANONICAL, {"deadline": -0.5}, "deadline is invalid"),
    (CANONICAL, {"binary": "sts-sim"}, "must be a pathlib.Path"),
))
def test_document_request_validation_fails_closed(
        monkeypatch, tmp_path, entry, kwargs, match):
    monkeypatch.setattr(
        rust_exact_solve.subprocess, "run",
        lambda *a, **k: pytest.fail("an invalid request must not be sent"))
    arguments = {"horizon": 3, "deadline": None,
                 "binary": tmp_path / "sts-sim", **kwargs}
    with pytest.raises(rust_exact_solve.RustExactSolveError, match=match):
        rust_exact_solve.solve_document(entry, **arguments)


def test_zero_budget_authentication_gets_a_transport_floor_not_one_second(
        monkeypatch, tmp_path):
    """#2754: a zero wire budget asks for no search, not a 1 s wall clock.

    The old `max(1.0, deadline + 1.0)` turned the zero budget into a 1.0 s
    subprocess cap, which a co-resident local cargo battery starved into
    `TimeoutExpired` and reddened `main`.  The wire budget stays zero -- the
    floor is a fail-closed transport net, not a wider search -- and positive
    budgets keep `deadline + 1.0` exactly.
    """
    seen = []

    def fake_run(argv, **kwargs):
        seen.append((json.loads(kwargs["input"]).get("deadline_ms"),
                     kwargs["timeout"]))
        return subprocess.CompletedProcess(argv, 0, stdout=json.dumps({
            "protocol": rust_exact_solve.PROTOCOL_V1, "status": "deadline",
            "solution": None, "refusal": None,
            "telemetry": {"nodes": 1, "transitions": 0}}), stderr="")

    monkeypatch.setattr(rust_exact_solve.subprocess, "run", fake_run)
    for deadline in (0, 0.0004, 2.5, 90.0, None):
        rust_exact_solve.solve_document(
            CANONICAL, horizon=5, deadline=deadline,
            binary=tmp_path / "sts-sim")

    floor = rust_exact_solve.AUTHENTICATION_TRANSPORT_FLOOR_S
    assert floor >= 30.0
    assert seen == [
        (0, floor),
        # A positive sub-millisecond cap reaches Rust as the same zero budget,
        # so it resolves to the floor too rather than to 1.0004 s.
        (0, floor),
        (2500, 3.5),
        (90000, 91.0),
        (None, None),
    ]


def test_transport_failures_are_typed_errors(monkeypatch, tmp_path):
    def exits(argv, **_kwargs):
        return subprocess.CompletedProcess(argv, 3, stdout="", stderr="boom")

    monkeypatch.setattr(rust_exact_solve.subprocess, "run", exits)
    with pytest.raises(rust_exact_solve.RustExactSolveError, match="exited 3"):
        rust_exact_solve.solve_document(
            CANONICAL, horizon=1, deadline=None, binary=tmp_path / "sts-sim")

    def garbage(argv, **_kwargs):
        return subprocess.CompletedProcess(argv, 0, stdout="{", stderr="")

    monkeypatch.setattr(rust_exact_solve.subprocess, "run", garbage)
    with pytest.raises(rust_exact_solve.RustExactSolveError, match="invalid JSON"):
        rust_exact_solve.solve_document(
            CANONICAL, horizon=1, deadline=None, binary=tmp_path / "sts-sim")
