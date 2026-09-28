"""One-call adapter for Rust's versioned exact-solve boundary (#2449).

The caller hands over one complete ``sts-sim-canonical-v2`` document (built by
the Rust opening, ``sts-sim entry ... --opening``), the adapter invokes ``sts-sim
exact-solve`` once, and the returned UID-anchored line is replayed in Rust
(``rust_review.replay_line``) before anything renders it. It deliberately
does not expose legal/apply calls: a DFS that crossed the process boundary per
node would be both slow and semantically easier to split-brain.

The Rust response's ``refusal.code`` is stable identity. Diagnostic text is
never parsed. Every refusal remains a hard, typed boundary result; there is
no Python search fallback.

Nothing here imports the frozen Python simulator (#2827 item F1). The
``combat_sim.State`` projection, the Python constructive seed and the Python
line re-resolution (``solve_state``, ``replay_python_actions``,
``_canonical_wire_action``) were deleted with their last callers: the retired
Python review producer and ``infoset_sample``.
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
from dataclasses import dataclass
from typing import Any

_RUST_TOOLS = pathlib.Path(__file__).parent.parent / "rust" / "tools"
if str(_RUST_TOOLS) not in sys.path:
    sys.path.insert(0, str(_RUST_TOOLS))
# The simulator-free document layer (digest, schema constants), #2827 item D.
import canonical_document  # noqa: E402


PROTOCOL_V1 = "sts-sim-exact-solve-v1"

# This is the v1 protocol's complete machine-readable refusal vocabulary.
# Treating an unfamiliar code as a normal fallback would let a newer or
# corrupted binary change exact-review authority without a Python change.
REFUSAL_CODES_V1 = frozenset({
    "malformed_request", "unsupported_protocol", "unsupported_schema",
    "unsupported_game_build", "document_carries_refusal",
    "multiplayer_allies", "multiplayer_player_order", "teammate_present",
    "teammate_power_card_pending", "teammate_damage_pending",
    "teammate_damage_owner_key", "canonical_boundary_refused",
    "engine_admission_refused", "engine_transition_refused",
    "memo_projection_refused", "cycle_detected", "no_legal_actions",
    "victory_score_combination_not_modeled", "invalid_horizon",
    "invalid_turn", "invalid_alpha_floor", "invalid_seed",
})

# #2754: a zero-millisecond budget does not ask for a one-second search -- it
# asks for NO search.  Rust compares `started.elapsed() >= duration` at its
# first node, with the clock started before admission, so `deadline_ms: 0`
# returns an incomplete `deadline` result immediately and only JSON parse,
# canonical admission, catalog/boundary construction and constructive-seed
# replay ever execute (`rust/src/exact_dfs.rs` `Dfs::visit`/`Dfs::new`,
# `rust/src/exact_solve_v1.rs` `deadline_ms`).  Deriving the transport wall
# clock from that zero budget gave `infoset_sample._validate_terminal_candidate`
# a 1.0 s cap on the DEBUG binary, which a co-resident local cargo battery
# starved into `TimeoutExpired` and reddened the gating `rust port` step on
# `main`.  The call is milliseconds on an idle box, so a floor generous enough
# to ride out contention costs nothing on the happy path while still failing
# closed on a wedged subprocess.  Positive budgets keep `deadline + 1.0`.
AUTHENTICATION_TRANSPORT_FLOOR_S = 30.0


def _wire_deadline_ms(deadline: float) -> int:
    """Round one caller cap DOWN to the wire's millisecond resolution."""
    return int(deadline * 1000)


def _transport_timeout(deadline: float | None) -> float | None:
    """Wall cap for one request, given the budget Rust will actually receive.

    A zero-millisecond budget is an authentication rather than a timed search,
    so it gets `AUTHENTICATION_TRANSPORT_FLOOR_S` instead of `budget + slack`.
    Sub-millisecond positive caps resolve here too, because they reach Rust as
    that same zero budget; keying on the wire value rather than on the float
    keeps the transport cap and the solver's budget from disagreeing.
    """
    if deadline is None:
        return None
    if _wire_deadline_ms(deadline) == 0:
        return AUTHENTICATION_TRANSPORT_FLOOR_S
    return max(1.0, deadline + 1.0)


def default_binary() -> pathlib.Path | None:
    """Return the deployed exact-solve CLI, without guessing a PATH entry.

    Production pins an absolute release binary with ``STS_SIM_EXACT_SOLVER``;
    the checked-in release target is the developer/CI fallback.  A missing
    binary is explicit (`None`) so callers fail closed rather than silently
    selecting a second search implementation.
    """
    configured = os.environ.get("STS_SIM_EXACT_SOLVER")
    candidates = ([pathlib.Path(configured).expanduser()] if configured else [
        pathlib.Path(__file__).parent.parent / "rust" / "target" / "release" / "sts-sim",
    ])
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate.resolve()
    return None


class RustExactSolveError(NotImplementedError):
    """The configured Rust boundary was malformed or cannot be replayed."""


class RustExactSolveRefusal(RustExactSolveError):
    """A stable Rust refusal code that callers must surface fail-closed."""

    def __init__(self, code: str, detail: str):
        super().__init__(f"Rust exact solve refused {code}: {detail}")
        self.code = code
        self.detail = detail


@dataclass(frozen=True)
class RustExactSolution:
    actions: tuple[dict[str, Any], ...]
    objective: tuple[int, int, int, int]
    final_digest: str


@dataclass(frozen=True)
class RustExactResult:
    status: str
    solution: RustExactSolution | None
    nodes: int
    transitions: int

    @property
    def exact(self) -> bool:
        return self.status == "exact"

    @property
    def incomplete(self) -> bool:
        return self.status in {"deadline", "cancelled"}


def solve_document(entry: dict[str, Any], *, horizon: int,
                   deadline: float | None, binary: pathlib.Path,
                   alpha_final_hp: int = 0) -> RustExactResult:
    """Solve one canonical document with exactly one subprocess request.

    ``deadline`` is rounded *down* to milliseconds: rounding up would claim a
    stronger caller cap than was requested.  A positive sub-millisecond cap
    becomes zero, which is safely more conservative.  The subprocess wall cap
    is a separate, looser fail-closed net -- see ``_transport_timeout``; a
    zero budget is an authentication, not a one-second search.
    """

    if not isinstance(binary, pathlib.Path):
        raise RustExactSolveError("Rust exact solver binary must be a pathlib.Path")
    if not isinstance(entry, dict) or entry.get("schema") != canonical_document.SCHEMA:
        raise RustExactSolveError(
            f"Rust exact solver entry must be a {canonical_document.SCHEMA} document")
    if type(horizon) is not int or horizon < 0:
        raise RustExactSolveError(f"Rust exact solver horizon is invalid: {horizon!r}")
    if type(alpha_final_hp) is not int or alpha_final_hp < 0:
        raise RustExactSolveError(
            f"Rust exact solver alpha final HP is invalid: {alpha_final_hp!r}")
    if deadline is not None and (
            not isinstance(deadline, (int, float)) or deadline < 0):
        raise RustExactSolveError(f"Rust exact solver deadline is invalid: {deadline!r}")

    request: dict[str, Any] = {
        "protocol": PROTOCOL_V1,
        "entry": entry,
        "max_turns": horizon,
        "alpha_final_hp": alpha_final_hp,
        "memo": True,
    }
    if deadline is not None:
        request["deadline_ms"] = _wire_deadline_ms(deadline)
    try:
        completed = subprocess.run(
            [str(binary), "exact-solve"], input=json.dumps(request),
            text=True, capture_output=True, check=False,
            timeout=_transport_timeout(deadline))
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RustExactSolveError(
            f"Rust exact solver transport failed: {type(exc).__name__}") from exc
    if completed.returncode:
        raise RustExactSolveError(
            f"Rust exact solver exited {completed.returncode}: "
            f"{completed.stderr.strip()}")
    try:
        response = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RustExactSolveError("Rust exact solver emitted invalid JSON") from exc
    return _parse_response(response)


def _parse_response(response: Any) -> RustExactResult:
    if not isinstance(response, dict) or response.get("protocol") != PROTOCOL_V1:
        raise RustExactSolveError("Rust exact solver response has an unknown protocol")
    status = response.get("status")
    if status not in {"exact", "deadline", "cancelled", "refused"}:
        raise RustExactSolveError(f"Rust exact solver status is invalid: {status!r}")
    telemetry = response.get("telemetry")
    if not isinstance(telemetry, dict):
        raise RustExactSolveError("Rust exact solver omitted telemetry")
    nodes = telemetry.get("nodes")
    transitions = telemetry.get("transitions")
    if type(nodes) is not int or nodes < 0 or type(transitions) is not int or transitions < 0:
        raise RustExactSolveError("Rust exact solver telemetry is invalid")
    raw_solution = response.get("solution")
    refusal = response.get("refusal")
    if status == "refused":
        if raw_solution is not None:
            raise RustExactSolveError(
                "Rust refused exact solve response also carried a solution")
        if (not isinstance(refusal, dict) or not isinstance(refusal.get("code"), str)
                or not isinstance(refusal.get("detail"), str)):
            raise RustExactSolveError("Rust exact solver refusal has no stable code")
        if refusal["code"] not in REFUSAL_CODES_V1:
            raise RustExactSolveError(
                f"Rust exact solver refusal code is unknown: {refusal['code']!r}")
        raise RustExactSolveRefusal(refusal["code"], refusal["detail"])
    if refusal is not None:
        raise RustExactSolveError(
            f"Rust {status} exact solve response also carried a refusal")
    if raw_solution is None:
        return RustExactResult(status, None, nodes, transitions)
    if not isinstance(raw_solution, dict):
        raise RustExactSolveError("Rust exact solver solution is invalid")
    actions = raw_solution.get("actions")
    objective = raw_solution.get("objective")
    digest = raw_solution.get("final_digest")
    if (not isinstance(actions, list) or not all(isinstance(action, dict)
            for action in actions) or not isinstance(objective, list)
            or len(objective) != 4 or any(type(value) is not int for value in objective)
            or not isinstance(digest, str)):
        raise RustExactSolveError("Rust exact solver solution shape is invalid")
    if objective[0] != 1:
        raise RustExactSolveError(
            "Rust exact solver solution must be a winning terminal line")
    return RustExactResult(
        status, RustExactSolution(tuple(actions), tuple(objective), digest),
        nodes, transitions)
