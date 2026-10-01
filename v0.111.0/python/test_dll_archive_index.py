"""The DLL archive index is the record that makes builds admittable (#1269).

The index exists because a missed archive is the one loss in this project that
cannot be undone: without a build's DLL there is no IL re-verification and no
harness oracle, so that build can never be admitted (I11). v0.108.0 is the
standing proof -- gone, and with it any possibility of admitting the build both
ground-truth pins were recorded on.
"""

import json
import pathlib

import pytest

from admission import admitted_builds

ROOT = pathlib.Path(__file__).resolve().parents[2]
INDEX = ROOT / "dll-archive" / "index.json"
TOOL = ROOT / "dll-archive" / "archive_build.py"


def _index() -> dict:
    return json.loads(INDEX.read_text())


def test_index_exists_and_is_machine_readable():
    """Prose in the README cannot be checked by a scheduled job."""
    assert INDEX.exists(), "the archive index is what --check reads"
    assert _index()["schema"] == "sts-dll-archive-index/v1"


def test_every_archived_build_is_pinned_by_sha_and_dated():
    """Identity is (version, sha256): a depot can be rebuilt under an
    unchanged version string, and that is a different build.

    The dates are not decoration -- #1271's straddling-run detector needs a
    release date per build to prove that a run which started before its own
    tagged build existed must have crossed a version boundary.
    """
    for build, row in _index()["builds"].items():
        assert len(row["dll_sha256"]) == 64, build
        assert row["archived_at"], build
        assert row["built_at"], f"{build} has no release date"
        assert row["commit"], build


def test_every_admitted_build_is_archived():
    """The load-bearing direction. Admission claims we can re-verify a build
    against its DLL; if it is not in the archive, that claim is empty."""
    archived = set(_index()["builds"])
    missing = [b for b in admitted_builds() if b not in archived]
    assert not missing, (
        f"admitted but unarchived: {missing} — admission promises the DLL is "
        "available for re-verification (SOLVER_INVARIANTS.md I11)")


def test_the_lost_build_is_recorded_as_lost():
    """v0.108.0 must never quietly reappear as archivable."""
    assert "v0.108.0" not in _index()["builds"]
    assert "v0.108" in (ROOT / "dll-archive" / "README.md").read_text()


@pytest.mark.parametrize("flag", ["--check", "--archive"])
def test_the_tool_offers_both_modes(flag):
    assert flag in TOOL.read_text()


def test_check_refuses_rather_than_assuming_when_there_is_no_install():
    """On a machine without the game, --check must not report success."""
    src = TOOL.read_text()
    assert "no Steam install at" in src
    assert "SystemExit" in src
