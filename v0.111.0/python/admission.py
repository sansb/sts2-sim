"""Exact-build admission for this bundle (#1258, SOLVER_INVARIANTS.md I11).

The bug this exists to kill: I5 refuses what is *unmodeled*, but a run from an
older build is *mis-modeled* — every mechanic is modeled, just not as that
build behaved — so nothing raised and the review rendered as an ordinary
certified card. Reviewing a v0.104 run with v0.111 combat semantics produced a
confident number for a fight that did not play that way.

Two independent checks, both fail-closed:

1. the build must be `admitted` in ``sim/builds.json``, the sole
   admission authority. Exact string match only — no ranges, no "latest"
   fallback, no default. ``sts2_rng.seeding_scheme`` still admits by range and
   is the remaining instance of the shape this replaces (#1265).
2. the build must be the one THIS bundle models. A bundle that accepts a build
   it was not built for would make a dispatcher mis-route silently; here it
   fails at the callee, which is the only place that can actually know.
"""

from __future__ import annotations

import hashlib
import json
import pathlib

# The build this bundle models. Its directory name is a claim; this is the
# claim made executable.
BUNDLE_BUILD = "v0.111.0"

_REGISTRY_PATH = (
    pathlib.Path(__file__).resolve().parents[2] / "builds.json")


def _registry() -> dict:
    return json.loads(_REGISTRY_PATH.read_text())["builds"]


def admitted_builds() -> tuple[str, ...]:
    """Every build the registry currently admits."""
    return tuple(sorted(
        b for b, row in _registry().items() if row.get("state") == "admitted"))


def build_state(build: str) -> str:
    """The registry's state for ``build``: admitted, pending, revoked, unknown.

    ``unknown`` is not ignorance — it is the terminal answer. A build absent
    from the registry has no archived DLL, so its combat semantics can never
    be verified against anything (#309: Steam deletes old builds on update),
    and no amount of later work makes it reviewable.
    """
    if not isinstance(build, str) or not build.strip():
        return "unknown"
    row = _registry().get(build.strip())
    state = row.get("state") if isinstance(row, dict) else None
    return state if isinstance(state, str) and state else "unknown"


def is_deferrable(build: str) -> bool:
    """Whether refusing ``build`` today might be worth revisiting (#1268).

    The line is whether the evidence still exists, not how old the build is.
    A `pending` build has an archived DLL, so admission stays possible at
    explicit cost and a run on it is deferred rather than refused. Everything
    else is terminal — including `revoked`, which is terminal by definition
    (a corrected successor is a new revision, not a reinstatement).
    """
    return build_state(build) == "pending"


def registry_digest() -> str:
    """A hash of every build's state — part of producer identity (#1268).

    Admission decides whether a fight yields a card, a deferral, or a
    terminal refusal, so the registry is as much a producer of the answer as
    the engine is. Folding it into identity is what makes "eligible for
    generation when the build is admitted" true by construction rather than
    by somebody remembering to run a backfill. Only build states are hashed:
    editing an `evidence` string must not invalidate a single document.
    """
    states = {build: row.get("state") for build, row in _registry().items()}
    return hashlib.sha256(json.dumps(
        states, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def require_admitted(build: str) -> str:
    """Return ``build`` if this bundle may simulate it, else refuse.

    Raises ``NotImplementedError`` — the I5 refusal shape — rather than
    returning a degraded answer. A refusal here is the correct outcome for
    every build we have not verified, which is every build except this one.
    """
    if not isinstance(build, str) or not build.strip():
        raise NotImplementedError(
            f"combat requires an explicit game build, got {build!r} "
            "(SOLVER_INVARIANTS.md I11 — admission is by exact build)")
    build = build.strip()
    row = _registry().get(build)
    if row is None or row.get("state") != "admitted":
        state = "absent from the registry" if row is None else (
            f"registry state {row.get('state')!r}")
        raise NotImplementedError(
            f"game build {build} is not admitted for combat semantics "
            f"({state}). This bundle models {BUNDLE_BUILD}; simulating "
            f"{build} would apply {BUNDLE_BUILD} semantics to a fight that "
            "did not play that way. Admitted: "
            f"{', '.join(admitted_builds()) or 'none'} "
            "(SOLVER_INVARIANTS.md I11 — refusing to mis-model)")
    if build != BUNDLE_BUILD:
        raise NotImplementedError(
            f"game build {build} is admitted but routed to the "
            f"{BUNDLE_BUILD} bundle; the dispatcher must select that build's "
            "own bundle (SOLVER_INVARIANTS.md I11 — a bundle only simulates "
            "the build it models)")
    return build
