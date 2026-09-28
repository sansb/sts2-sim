"""Exact-build admission at the review boundary (#1258, I11)."""

import json
import pathlib

import pytest

import admission
from admission import BUNDLE_BUILD, admitted_builds, require_admitted
from relay_parser import parse_run
from review_summary import ReviewRefusal, require_review_admission


def load_fight_context(path, _fight_index):
    """The review boundary's admission check, as every producer runs it.

    Formerly `review_summary.load_fight_context`, whose Python root builder
    #2827 item F1 moved out of production; the admission half is shared by the
    Rust producers (`rust_legacy_review.generate`).
    """
    require_review_admission(parse_run(str(path)))


HERE = pathlib.Path(__file__).parent
# Any seed does: admission reads only the build. This one is the seed of the
# old fight_states.json pin run, deleted with the Python simulator (#2827).
SEED = "ZPJHU3WSH2"
REGISTRY = HERE.parents[1] / "admitted_builds.json"


def test_registry_is_the_authority_and_admits_only_this_bundles_build():
    rows = json.loads(REGISTRY.read_text())["builds"]
    admitted = [b for b, r in rows.items() if r["state"] == "admitted"]
    assert admitted == [BUNDLE_BUILD] == list(admitted_builds())
    # Admission is evidence-bearing, not a bare allowlist: the DLL is pinned
    # by sha256 because a depot can be rebuilt under an unchanged version
    # string (solver/SIM_VERSIONING.md).
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


def test_the_review_boundary_refuses_an_old_run_with_a_structured_reason(
        tmp_path):
    """The product-facing half: an old run yields a refusal card, not a number.

    Before #1258 this returned a confident review computed with v0.111
    combat semantics for a fight recorded on a different build.
    """
    raw = {
        "build_id": "v0.104.0", "seed": SEED, "schema_version": 9,
        "ascension": 0, "win": False, "killed_by_encounter": "",
        "killed_by_event": "NONE.NONE", "game_mode": "standard",
        "modifiers": [], "platform_type": "steam", "was_abandoned": False,
        "acts": [], "map_point_history": [], "run_time": 1, "start_time": 1,
        "players": [{"character": "CHARACTER.IRONCLAD", "id": 0, "deck": [],
                     "relics": [], "potions": [], "badges": [],
                     "max_potion_slot_count": 2}],
    }
    path = tmp_path / "old.run"
    path.write_text(json.dumps(raw))
    with pytest.raises(ReviewRefusal) as caught:
        load_fight_context(str(path), 0)
    assert caught.value.reason == "unadmitted_game_build"
    assert caught.value.details["game_build"] == "v0.104.0"
    assert caught.value.details["admitted_builds"] == [BUNDLE_BUILD]
    # v0.104.0's DLL was never archived, so no future work makes this fight
    # reviewable. Terminal, not deferred (#1268).
    assert caught.value.deferred is False
    assert caught.value.details["registry_state"] == "unknown"


def _raw_run(build):
    return {
        "build_id": build, "seed": SEED, "schema_version": 9,
        "ascension": 0, "win": False, "killed_by_encounter": "",
        "killed_by_event": "NONE.NONE", "game_mode": "standard",
        "modifiers": [], "platform_type": "steam", "was_abandoned": False,
        "acts": [], "map_point_history": [], "run_time": 1, "start_time": 1,
        "players": [{"character": "CHARACTER.IRONCLAD", "id": 0, "deck": [],
                     "relics": [], "potions": [], "badges": [],
                     "max_potion_slot_count": 2}],
    }


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


def test_a_pending_build_defers_instead_of_refusing(tmp_path):
    """"Not yet" is stored as its own answer, so a viewer is not told a fight
    is unreviewable when we expect to review it, and so the population can be
    found by query when the build is admitted."""

    path = tmp_path / "pending.run"
    path.write_text(json.dumps(_raw_run("v0.110.1")))
    with pytest.raises(ReviewRefusal) as caught:
        load_fight_context(str(path), 0)

    assert caught.value.deferred is True
    assert caught.value.reason == "game_build_pending_admission"
    assert caught.value.details["registry_state"] == "pending"
    assert caught.value.details["game_build"] == "v0.110.1"
    # The message must not read like a refusal.
    assert "not yet admitted" in caught.value.message
    assert "no result yet rather than no result ever" in caught.value.message


def test_a_pending_build_yields_a_deferred_document(tmp_path):
    from review_summary_v2 import generate_review_document

    path = tmp_path / "pending.run"
    path.write_text(json.dumps(_raw_run("v0.110.1")))
    document = generate_review_document(str(path), 0)

    assert document["status"] == "deferred"
    assert "refusal" not in document
    assert document["deferral"]["reason"] == "game_build_pending_admission"
    # A deferral is still a produced document and carries producer identity,
    # including the registry digest that makes it regenerate on admission.
    simulator = document["metadata"]["simulator"]
    assert simulator["game_build"] == BUNDLE_BUILD
    assert len(simulator["admitted_builds_digest"]) == 64


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
