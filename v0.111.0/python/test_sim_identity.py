"""Contracts for the simulator identity stamped on review documents (#1267).

The defect being guarded: a document that cannot say what produced it cannot
be invalidated when the producer changes, so a corrected simulator leaves
every stored document looking current.
"""

import hashlib
import json
import pathlib
import subprocess
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import sim_identity  # noqa: E402
from admission import BUNDLE_BUILD  # noqa: E402


HERE = pathlib.Path(__file__).resolve().parent
SIM_ROOT = HERE.parents[1]


@pytest.fixture(autouse=True)
def _clear_digest_cache():
    sim_identity.bundle_digest.cache_clear()
    yield
    sim_identity.bundle_digest.cache_clear()


def _tracked_bundle_files():
    try:
        listed = subprocess.run(
            ["git", "ls-files", "--", str(HERE.relative_to(SIM_ROOT))],
            cwd=SIM_ROOT, text=True, capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):  # pragma: no cover
        pytest.skip("git is unavailable")
    prefix = str(HERE.relative_to(SIM_ROOT)) + "/"
    return {
        pathlib.PurePath(line[len(prefix):])
        for line in listed.splitlines() if line.startswith(prefix)}


def test_identity_names_the_artifact_admission_verified():
    """A document records the DLL, not just a version string.

    A depot can be rebuilt under an unchanged version string, so the string
    alone is not identity (SIM_VERSIONING.md, admission criterion 1). The
    values are read from the admission registry rather than restated in code,
    which is the only way the two cannot drift apart.
    """

    registry = json.loads(
        (SIM_ROOT / "builds.json").read_text())
    admitted = registry["builds"][BUNDLE_BUILD]
    identity = sim_identity.identity()

    assert identity["game_build"] == BUNDLE_BUILD
    assert identity["game_commit"] == admitted["game_commit"]
    assert identity["dll_sha256"] == admitted["dll_sha256"]
    assert identity["bundle"] == f"sim/{BUNDLE_BUILD}/python"
    assert admitted["state"] == "admitted"


def test_identity_is_json_serialisable_and_complete():
    identity = sim_identity.identity()
    assert json.loads(json.dumps(identity, sort_keys=True)) == identity
    assert set(identity) == {
        "game_build", "game_commit", "dll_sha256", "bundle",
        "sim_revision", "bundle_digest", "parser_revision",
        "pipeline_revision", "admitted_builds_digest", "rust_exact_solver"}
    assert len(identity["bundle_digest"]) == 64


def test_rust_exact_solver_identity_tracks_bytes_path_and_availability(
        tmp_path):
    missing = tmp_path / "missing-sts-sim"
    assert sim_identity.exact_solver_artifact_identity(missing) == {
        "path": str(missing.resolve()), "state": "absent"}

    first = tmp_path / "first-sts-sim"
    first.write_bytes(b"native-v1")
    first.chmod(0o755)
    baseline = sim_identity.exact_solver_artifact_identity(first)
    assert baseline == {
        "path": str(first.resolve()), "state": "present",
        "sha256": hashlib.sha256(b"native-v1").hexdigest(),
    }

    first.write_bytes(b"native-v2")
    rebuilt = sim_identity.exact_solver_artifact_identity(first)
    assert rebuilt["sha256"] != baseline["sha256"]

    second = tmp_path / "second-sts-sim"
    second.write_bytes(b"native-v2")
    second.chmod(0o755)
    overridden = sim_identity.exact_solver_artifact_identity(second)
    assert overridden["sha256"] == rebuilt["sha256"]
    assert overridden["path"] != rebuilt["path"]

    second.chmod(0o644)
    assert sim_identity.exact_solver_artifact_identity(second)["state"] == \
        "non_executable"


def test_bundle_digest_tracks_content_and_layout(tmp_path, monkeypatch):
    """The digest is what makes invalidation automatic, so it must actually
    depend on bytes AND on names — a rename can change what an import
    resolves to without changing any file's contents."""

    monkeypatch.setattr(sim_identity, "HERE", tmp_path)
    (tmp_path / "content").mkdir()
    (tmp_path / "combat_sim.py").write_text("A")
    (tmp_path / "content" / "cards.py").write_text("B")
    baseline = sim_identity.bundle_digest()

    sim_identity.bundle_digest.cache_clear()
    (tmp_path / "content" / "cards.py").write_text("B ")
    edited = sim_identity.bundle_digest()
    assert edited != baseline

    sim_identity.bundle_digest.cache_clear()
    (tmp_path / "content" / "cards.py").write_text("B")
    assert sim_identity.bundle_digest() == baseline

    sim_identity.bundle_digest.cache_clear()
    (tmp_path / "content" / "cards.py").rename(tmp_path / "content" / "x.py")
    assert sim_identity.bundle_digest() != baseline

    sim_identity.bundle_digest.cache_clear()
    (tmp_path / "content" / "x.py").rename(tmp_path / "content" / "cards.py")
    (tmp_path / "extra.json").write_text("{}")
    assert sim_identity.bundle_digest() != baseline


def test_digest_ignores_tests_docs_and_build_output(tmp_path, monkeypatch):
    monkeypatch.setattr(sim_identity, "HERE", tmp_path)
    (tmp_path / "combat_sim.py").write_text("A")
    baseline = sim_identity.bundle_digest()

    for name in ("test_thing.py", "NOTES.md", "combat_sim.pyc"):
        sim_identity.bundle_digest.cache_clear()
        (tmp_path / name).write_text("noise")
        assert sim_identity.bundle_digest() == baseline, name
    for directory in sim_identity._EXCLUDED_DIRS:
        sim_identity.bundle_digest.cache_clear()
        (tmp_path / directory).mkdir()
        (tmp_path / directory / "thing.py").write_text("noise")
        assert sim_identity.bundle_digest() == baseline, directory


def test_a_file_exclusion_excuses_that_file_and_not_its_name(
        tmp_path, monkeypatch):
    """`err.txt` at the bundle root is stray tooling output. A future
    `content/err.txt` has not earned the same excuse, so exclusions are keyed
    by path — the fail-open direction is the one that leaves a stale document
    looking current."""

    monkeypatch.setattr(sim_identity, "HERE", tmp_path)
    (tmp_path / "combat_sim.py").write_text("A")
    (tmp_path / "err.txt").write_text("noise")
    baseline = sim_identity.bundle_digest()

    sim_identity.bundle_digest.cache_clear()
    (tmp_path / "content").mkdir()
    (tmp_path / "content" / "err.txt").write_text("noise")
    assert sim_identity.bundle_digest() != baseline


def test_every_tracked_bundle_file_is_classified():
    """A new file is semantic unless somebody says otherwise.

    The exclusion lists are allowlists of *non*-semantic files, so a file
    nobody classified over-invalidates (wasted worker time) rather than
    under-invalidating (a stale document that looks current). This test only
    has to prove the two sets partition the bundle; it deliberately does not
    require anyone to touch it when a content file lands.
    """

    tracked = _tracked_bundle_files()
    assert tracked, "git listed no bundle files"
    covered = set(sim_identity.semantic_files())
    unclassified = {
        path for path in tracked - covered
        if not sim_identity._excluded(path)}
    assert not unclassified, sorted(map(str, unclassified))
    # An untracked file inside the bundle silently changes the digest this
    # machine computes, so the worker here would key freshness differently
    # from CI. Loud is better than mysterious.
    assert not covered - tracked, sorted(map(str, covered - tracked))


_IMPORT_CLOSURE_PROBE = """
import json, pathlib, sys
here = pathlib.Path(sys.argv[1]).resolve()
sys.path.insert(0, str(here))
import review_summary_v2  # noqa: F401
print(json.dumps(sorted(
    str(pathlib.Path(module.__file__).resolve().relative_to(here))
    for module in list(sys.modules.values())
    if getattr(module, "__file__", None)
    and pathlib.Path(module.__file__).resolve().is_relative_to(here))))
"""


def test_digest_covers_the_review_cli_import_closure():
    """Derived, not declared: whatever the production CLI actually imports
    from this bundle must be inside the digest. An exclusion that quietly
    drops a module the review path loads is the failure this catches.

    Run in a subprocess so the closure is the CLI's, not this test session's
    — pytest has already imported every test module in the bundle.
    """

    completed = subprocess.run(
        [sys.executable, "-c", _IMPORT_CLOSURE_PROBE, str(HERE)],
        text=True, capture_output=True, check=True)
    imported = {pathlib.PurePath(p) for p in json.loads(completed.stdout)}
    covered = set(sim_identity.semantic_files())

    assert imported, "no bundle modules imported; did the CLI move?"
    assert not imported - covered, sorted(map(str, imported - covered))


def _registry(tmp_path, monkeypatch, payload):
    path = tmp_path / "pipeline_revisions.json"
    path.write_text(json.dumps(payload))
    monkeypatch.setattr(sim_identity, "_PIPELINE_REVISIONS", path)
    return path


def test_pipeline_revision_refuses_an_undeclared_compatibility_class(
        tmp_path, monkeypatch):
    """Left inferrable, `quality_only` becomes the default because it is the
    cheap one, and invalidation quietly stops firing (SIM_VERSIONING.md)."""

    _registry(tmp_path, monkeypatch, {
        "current": 2,
        "revisions": [
            {"revision": 1, "compatibility": "correctness_invalidating"},
            {"revision": 2, "summary": "forgot to say"},
        ]})
    with pytest.raises(NotImplementedError, match="compatibility"):
        sim_identity.pipeline_revision()

    _registry(tmp_path, monkeypatch, {
        "current": 2,
        "revisions": [
            {"revision": 1, "compatibility": "correctness_invalidating"},
            {"revision": 2, "compatibility": "probably_fine"},
        ]})
    with pytest.raises(NotImplementedError, match="compatibility"):
        sim_identity.pipeline_revision()


def test_pipeline_registry_refuses_gaps_and_a_stale_current(
        tmp_path, monkeypatch):
    _registry(tmp_path, monkeypatch, {
        "current": 3,
        "revisions": [
            {"revision": 1, "compatibility": "equivalent"},
            {"revision": 3, "compatibility": "equivalent"},
        ]})
    with pytest.raises(NotImplementedError, match="consecutively"):
        sim_identity.pipeline_revision()

    _registry(tmp_path, monkeypatch, {
        "current": 1,
        "revisions": [
            {"revision": 1, "compatibility": "equivalent"},
            {"revision": 2, "compatibility": "quality_only"},
        ]})
    with pytest.raises(NotImplementedError, match="last declared revision"):
        sim_identity.pipeline_revision()


def test_correctness_invalidating_revokes_everything_below_it(
        tmp_path, monkeypatch):
    """The pipeline is shared, so a false optimum it published is false under
    every build — revocation is not per bundle."""

    _registry(tmp_path, monkeypatch, {
        "current": 4,
        "revisions": [
            {"revision": 1, "compatibility": "correctness_invalidating"},
            {"revision": 2, "compatibility": "quality_only"},
            {"revision": 3, "compatibility": "correctness_invalidating"},
            {"revision": 4, "compatibility": "equivalent"},
        ]})
    assert sim_identity.pipeline_revision() == 4
    assert sim_identity.minimum_valid_pipeline_revision() == 3

    _registry(tmp_path, monkeypatch, {
        "current": 2,
        "revisions": [
            {"revision": 1, "compatibility": "equivalent"},
            {"revision": 2, "compatibility": "quality_only"},
        ]})
    assert sim_identity.minimum_valid_pipeline_revision() == 1


def test_checked_in_pipeline_registry_validates():
    registry = json.loads((SIM_ROOT / "meta" / "pipeline_revisions.json").read_text())
    assert sim_identity.pipeline_revision() == registry["current"]
    assert set(registry["classes"]) == {
        "equivalent", "quality_only", "correctness_invalidating"}
    for entry in registry["revisions"]:
        assert entry["compatibility"] in registry["classes"]
        assert entry["summary"] and entry["detail"]


def test_identity_digest_is_stable_and_covers_every_field():
    baseline = sim_identity.digest()
    assert baseline == sim_identity.digest(sim_identity.identity())
    assert baseline == hashlib.sha256(json.dumps(
        sim_identity.identity(), sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()
    for field in sim_identity.identity():
        changed = sim_identity.identity()
        changed[field] = "different"
        assert sim_identity.digest(changed) != baseline, field
