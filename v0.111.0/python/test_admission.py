"""Exact-build admission (#1258, I11)."""

import json
import pathlib

import pytest

import admission
from admission import BUNDLE_BUILD, admitted_builds, require_admitted


HERE = pathlib.Path(__file__).parent
REGISTRY = HERE.parents[1] / "builds.json"


def test_registry_is_the_authority_and_admits_only_this_bundles_build():
    rows = json.loads(REGISTRY.read_text())["builds"]
    admitted = [b for b, r in rows.items() if r["state"] == "admitted"]
    assert admitted == [BUNDLE_BUILD] == list(admitted_builds())
    # Admission is evidence-bearing, not a bare allowlist: the DLL is pinned
    # by sha256 because a depot can be rebuilt under an unchanged version
    # string (sim/meta/SIM_VERSIONING.md).
    row = rows[BUNDLE_BUILD]
    assert len(row["dll_sha256"]) == 64
    assert row["game_commit"] and row["evidence"]


@pytest.mark.parametrize("build", [
    "v0.104.0",    # the local corpus: no archived DLL, never admittable
    "v0.108.0",    # the retired fight_states pins' build; DLL deleted by Steam (#309)
    "v0.110.1",    # archived, modelled by the Rust kernel, NOT admitted here
    "v0.112.0",    # a future build must refuse until it is verified
])
def test_unadmitted_builds_refuse(build):
    with pytest.raises(NotImplementedError, match="not admitted"):
        require_admitted(build)


@pytest.mark.parametrize("build", ["", "   ", None, 111])
def test_a_missing_or_malformed_build_refuses_rather_than_defaulting(build):
    """No default. A caller that does not know its build cannot simulate."""
    with pytest.raises(NotImplementedError, match="explicit game build"):
        require_admitted(build)


def test_a_bundle_refuses_a_build_it_does_not_model(monkeypatch):
    """Admitted elsewhere is not admitted here.

    Once a second bundle exists this is what makes a dispatcher mis-route
    fail at the callee — the only place that can know which build it models.
    """
    monkeypatch.setattr(admission, "_registry", lambda: {
        BUNDLE_BUILD: {"state": "admitted"},
        "v0.112.0": {"state": "admitted"},
    })
    assert require_admitted(BUNDLE_BUILD) == BUNDLE_BUILD
    with pytest.raises(NotImplementedError, match="routed to the"):
        require_admitted("v0.112.0")


def test_a_pending_bundle_is_not_admitted(monkeypatch):
    """Copying a predecessor must never itself enable a build."""
    monkeypatch.setattr(admission, "_registry", lambda: {
        BUNDLE_BUILD: {"state": "pending"}})
    with pytest.raises(NotImplementedError, match="registry state 'pending'"):
        require_admitted(BUNDLE_BUILD)


def test_registry_states_split_not_yet_from_never():
    """The line is whether the evidence survives, not how old the build is.

    v0.108.0 is NEWER than v0.107.1 and permanently unreviewable, because
    Steam deleted its DLL (#309) while v0.107.1's was archived. Any rule
    phrased as a version comparison gets this backwards.
    """
    assert admission.build_state(BUNDLE_BUILD) == "admitted"
    for build in ("v0.110.1", "v0.109.1", "v0.109.0", "v0.107.1"):
        assert admission.build_state(build) == "pending", build
        assert admission.is_deferrable(build), build
    for build in ("v0.108.0", "v0.107.0", "v0.104.0", "v0.99.1", "v0.112.0"):
        assert admission.build_state(build) == "unknown", build
        assert not admission.is_deferrable(build), build
    # Admitted is not deferrable: there is nothing to wait for.
    assert not admission.is_deferrable(BUNDLE_BUILD)


def test_the_registry_digest_ignores_prose_but_tracks_states(monkeypatch):
    """Editing an `evidence` string must not invalidate a single document;
    changing what a build is allowed to do must invalidate all of them."""

    base = {BUNDLE_BUILD: {"state": "admitted", "evidence": "original"}}
    monkeypatch.setattr(admission, "_registry", lambda: base)
    baseline = admission.registry_digest()

    monkeypatch.setattr(admission, "_registry", lambda: {
        BUNDLE_BUILD: {"state": "admitted", "evidence": "reworded"}})
    assert admission.registry_digest() == baseline

    monkeypatch.setattr(admission, "_registry", lambda: {
        BUNDLE_BUILD: {"state": "admitted", "evidence": "original"},
        "v0.110.1": {"state": "pending"}})
    promoted = admission.registry_digest()
    assert promoted != baseline

    monkeypatch.setattr(admission, "_registry", lambda: {
        BUNDLE_BUILD: {"state": "admitted", "evidence": "original"},
        "v0.110.1": {"state": "admitted"}})
    assert admission.registry_digest() not in (baseline, promoted)
