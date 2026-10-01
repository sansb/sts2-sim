"""Simulator identity stamped on every review document (#1267, I12).

The bug this exists to kill: a review document did not record what produced
it, so it could not be invalidated when the producer changed. Its only
identity fields were the run's own build (under ``metadata.solver_build``, a
name that reads like the solver's), the schema version, and the worker's
generator configuration. Correcting the *simulator* for the same game build
therefore left every stored document looking current — the freshness check had
nothing to compare against.

Four revisions, per ``sim/meta/SIM_VERSIONING.md``:

``game_build`` plus game artifact identity
    Which build's semantics this bundle models, and the archived DLL those
    semantics were verified against. Verified, not routed: the commit and
    sha256 come from the admission registry, so a document records the exact
    artifact rather than a version string a depot could rebuild under.

``sim_revision`` + ``bundle_digest``
    Build-specific game and world-model fidelity. ``sim_revision`` is the
    declared, human-meaningful era marker; the digest is the mechanical one
    and is the part that actually protects the invariant. It is recomputed
    from the live files on every run, so it cannot go stale, and any semantic
    edit — an entry adapter, a census, a template — changes it without
    anybody remembering to bump anything. (The Python engine and
    ``content/**`` it once covered were deleted in #2827 item F; the engine
    is the Rust executable identified below.)

``parser_revision``
    Parser and entry reconstruction. These change the state handed to the
    simulator, so they are inside the exactness envelope even though they are
    not combat semantics.

``pipeline_revision``
    Shared search, estimator, and producer correctness, declared in
    ``sim/meta/pipeline_revisions.json`` with an explicit compatibility class.

``rust_exact_solver``
    The resolved path, availability state, and sha256 of the native executable
    that admits, transitions, and searches the fight. Deployments replace that
    file in place, so its bytes are identity independently of the Python bundle.

**What is automatic today and what is not.** ``bundle_digest`` currently
covers the pipeline too, because #1275 has not yet hoisted the shared layer
out of the bundle — search, the estimator, and the review producer are still
physically inside ``sim/v0.111.0/python``. The declared
``pipeline_revision`` is therefore belt-and-braces right now and becomes
load-bearing the moment that hoist lands. **#1275 must add a `pipeline_digest`
over the hoisted layer**, or the automatic half of this protection silently
narrows to fidelity alone. The separately hashed Rust executable is already
automatic and must stay separate: it can be rebuilt at an unchanged path
without changing any tracked Python source.

Surface selection is deliberately fail-closed the other way from most
allowlists: everything tracked in the bundle counts as semantic *unless*
excluded below. A file nobody classified therefore over-invalidates (wasted
worker time) instead of under-invalidating (a stale document that looks
current). ``test_sim_identity.py`` derives the review CLI's real import
closure and asserts the digest covers it, so the exclusion list cannot quietly
drop something the production path imports.
"""

from __future__ import annotations

import fnmatch
import functools
import hashlib
import json
import os
import pathlib

from admission import BUNDLE_BUILD, registry_digest

# Declared era markers. Neither is the safety mechanism — ``bundle_digest``
# is — so a forgotten bump costs legibility, not correctness. Bump when a
# change is worth naming to a human reading stored documents.
SIM_REVISION = 1
PARSER_REVISION = 1

HERE = pathlib.Path(__file__).resolve().parent
_SIM_ROOT = HERE.parents[1]
_ADMITTED_BUILDS = _SIM_ROOT / "builds.json"
_PIPELINE_REVISIONS = _SIM_ROOT / "meta" / "pipeline_revisions.json"

# Directories that hold no semantics the production review path reads.
_EXCLUDED_DIRS = {
    "testdata": "test inputs",
    "tools": "developer tooling, except the files named below",
    "harness": "the DLL oracle harness, not imported by the review CLI",
    "profiles": "profiling artifacts",
    "__pycache__": "build output",
}

# Files inside an excluded directory that the production path really imports.
# This list is not trusted to be complete on anyone's word:
# ``test_sim_identity.py`` derives the review CLI's actual import closure and
# fails naming whatever is missing. live_coach is here because it was caught
# that way — the review CLI imports its entry builder (`build_entry`, via
# ``review_summary_v2`` and ``rust_review``), which puts it squarely inside
# `parser_revision`, however much it reads like a tool.
_INCLUDED_FILES = {
    pathlib.PurePath("tools/live_coach.py"):
        "the review CLI imports build_entry from it — entry reconstruction",
}

# Individual files, each with the reason it does not change an answer. Keyed
# by full bundle-relative path, not basename: a name is excused here on the
# strength of what that particular file is, and a future `content/.../err.txt`
# has not earned the same excuse.
_EXCLUDED_FILES = {
    pathlib.PurePath(name): reason for name, reason in {
        "attestations.json": "test pin manifest",
        "attestations.py": "test pin manifest reader",
        "coverage_matrix.json":
            "frozen Python coverage record, read by harness probes and tests",
        "err.txt": "stray tooling output",
        "relic_err.txt": "stray tooling output",
    }.items()
}

_EXCLUDED_GLOBS = {
    "test_*.py": "tests",
    "*.md": "documentation",
    "*.pyc": "build output",
    ".DS_Store": "filesystem noise",
}

_COMPATIBILITY_CLASSES = frozenset(
    ("equivalent", "quality_only", "correctness_invalidating"))


def _excluded(relative: pathlib.PurePath) -> bool:
    if relative in _INCLUDED_FILES:
        return False
    if set(relative.parts[:-1]) & set(_EXCLUDED_DIRS):
        return True
    if relative in _EXCLUDED_FILES:
        return True
    return any(fnmatch.fnmatch(relative.name, glob) for glob in _EXCLUDED_GLOBS)


def semantic_files() -> tuple[pathlib.PurePath, ...]:
    """Every bundle file whose content can change an answer, sorted.

    Walks the filesystem rather than the git index: the production worker
    must be able to compute this without shelling out to git, and an
    untracked stray in the bundle changing the digest is the safe direction.
    """
    return tuple(sorted(
        (path.relative_to(HERE) for path in HERE.rglob("*")
         if path.is_file() and not _excluded(path.relative_to(HERE))),
        key=str))


@functools.cache
def bundle_digest() -> str:
    """sha256 over the bundle's semantic surface, path-sensitive.

    Names are hashed alongside contents so that moving a file — which can
    change what imports resolve to — is a digest change, and so that two
    files swapping contents cannot collide.
    """
    digest = hashlib.sha256()
    for relative in semantic_files():
        digest.update(str(relative).encode())
        digest.update(b"\0")
        digest.update(hashlib.sha256((HERE / relative).read_bytes()).digest())
        digest.update(b"\0")
    return digest.hexdigest()


def exact_solver_artifact_identity(
        binary: pathlib.Path | None = None) -> dict:
    """Identity of the configured native answer-producing artifact.

    The executable path alone is not identity: deployments replace the file
    in place.  Missing/non-executable candidates remain explicit states so a
    later install, chmod, path override, or rebuild changes worker freshness.
    """
    if binary is None:
        configured = os.environ.get("STS_SIM_EXACT_SOLVER")
        binary = (pathlib.Path(configured).expanduser() if configured else
                  HERE.parent / "engine" / "target" / "release" / "sts-sim")
    path = binary.expanduser().resolve()
    base = {"path": str(path)}
    if not path.is_file():
        return {**base, "state": "absent"}
    artifact_sha256 = hashlib.sha256(path.read_bytes()).hexdigest()
    if not os.access(path, os.X_OK):
        return {**base, "state": "non_executable",
                "sha256": artifact_sha256}
    return {**base, "state": "present", "sha256": artifact_sha256}


def game_artifact_identity() -> dict:
    """The archived artifact this bundle's semantics were verified against.

    Read from the admission registry rather than restated here: a second
    copy of the sha256 is a second thing to drift.
    """
    row = json.loads(_ADMITTED_BUILDS.read_text())["builds"].get(BUNDLE_BUILD)
    if not isinstance(row, dict):
        raise NotImplementedError(
            f"{BUNDLE_BUILD} has no entry in {_ADMITTED_BUILDS.name}; this "
            "bundle cannot state what artifact it models "
            "(SOLVER_INVARIANTS.md I12)")
    identity = {"game_build": BUNDLE_BUILD}
    for field in ("game_commit", "dll_sha256"):
        value = row.get(field)
        if not isinstance(value, str) or not value.strip():
            raise NotImplementedError(
                f"{BUNDLE_BUILD}'s admission entry has no {field}; a document "
                "may not claim artifact identity it does not have "
                "(SOLVER_INVARIANTS.md I12)")
        identity[field] = value
    return identity


def _pipeline_registry() -> dict:
    registry = json.loads(_PIPELINE_REVISIONS.read_text())
    revisions = registry.get("revisions")
    if not isinstance(revisions, list) or not revisions:
        raise NotImplementedError(
            "sim/meta/pipeline_revisions.json declares no revisions")
    expected = 1
    for entry in revisions:
        if not isinstance(entry, dict) or entry.get("revision") != expected:
            raise NotImplementedError(
                "sim/meta/pipeline_revisions.json must declare revisions "
                f"consecutively from 1; expected {expected}")
        # The whole point of the field: a change that does not say what it
        # did to prior answers cannot ship. Left inferrable, `quality_only`
        # becomes the default because it is the cheap one, and invalidation
        # quietly stops firing (SIM_VERSIONING.md).
        if entry.get("compatibility") not in _COMPATIBILITY_CLASSES:
            raise NotImplementedError(
                f"pipeline revision {expected} declares no reviewed "
                f"compatibility class; one of {sorted(_COMPATIBILITY_CLASSES)} "
                "must be stated on the change itself")
        expected += 1
    if registry.get("current") != revisions[-1]["revision"]:
        raise NotImplementedError(
            "sim/meta/pipeline_revisions.json's `current` must name its last "
            "declared revision")
    return registry


def pipeline_revision() -> int:
    return int(_pipeline_registry()["current"])


def minimum_valid_pipeline_revision() -> int:
    """The oldest pipeline revision whose documents may still be believed.

    A ``correctness_invalidating`` change revokes every revision below it
    **across every build** — the pipeline is shared, so a false optimum it
    published is false wherever it was published. This is the query side of
    invalidation; the worker's freshness check regenerates on any declared
    change at all, including the cheap ones.
    """
    registry = _pipeline_registry()
    revoking = [
        entry["revision"] for entry in registry["revisions"]
        if entry["compatibility"] == "correctness_invalidating"]
    return max(revoking) if revoking else 1


def identity(exact_solver_binary: pathlib.Path | None = None) -> dict:
    """The full simulator identity block stamped into a review document."""
    return {
        **game_artifact_identity(),
        "bundle": f"sim/{BUNDLE_BUILD}/python",
        "sim_revision": SIM_REVISION,
        "bundle_digest": bundle_digest(),
        "parser_revision": PARSER_REVISION,
        "pipeline_revision": pipeline_revision(),
        # Admission decides card vs deferral vs terminal refusal, so the
        # registry produces the answer as surely as the engine does (#1268).
        # Including it is what makes a deferred document regenerate when its
        # build is finally admitted, instead of waiting for a backfill nobody
        # remembers to run.
        "admitted_builds_digest": registry_digest(),
        # Rust admission, transitions, and exact search now produce the
        # answer. Rebuilding the same path must invalidate stored documents.
        "rust_exact_solver": exact_solver_artifact_identity(
            exact_solver_binary),
    }


def digest(block: dict | None = None) -> str:
    """A single hash of the identity block, for the worker's freshness key."""
    encoded = json.dumps(
        identity() if block is None else block,
        sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    return hashlib.sha256(encoded).hexdigest()


if __name__ == "__main__":  # pragma: no cover - operator convenience
    print(json.dumps(identity(), indent=2, sort_keys=True))
