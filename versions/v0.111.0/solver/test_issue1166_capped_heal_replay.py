"""#1166: a capped post-combat heal is settled by an exact recorded replay.

`.run` records HP after Burning Blood's post-combat heal. When that heal ends
at the max-HP cap, the combat-end HP is only known to lie in a band; the Rust
review path now accepts the recorded replay's combat-end HP when it lands in
that band, and keeps the observed_final_hp_ambiguous refusal otherwise.
"""
import json
from types import SimpleNamespace

import pytest

import review_summary
import rust_replay
import rust_review as review
from review_summary import ReviewConfig, ReviewRefusal


def fight(**overrides):
    values = dict(hp_entering=68, hp_after=80, max_hp_entering=80, hp_healed=12,
                  relics_entering=("RELIC.BURNING_BLOOD",), potions_used=[],
                  turns_taken=4, node_index=34, encounter_id="ENCOUNTER.X",
                  node_type="monster")
    values.update(overrides)
    return SimpleNamespace(**values)


@pytest.mark.parametrize("overrides,band", [
    ({}, (74, 80)),
    ({"relics_entering": ("RELIC.BLACK_BLOOD",)}, (68, 80)),
    # Chosen Cheese's max-HP gain is removed first; the cap moved with it.
    ({"relics_entering": ("RELIC.BURNING_BLOOD", "RELIC.CHOSEN_CHEESE"),
      "hp_after": 81}, (74, 80)),
    ({"hp_after": 79}, None),                        # below the cap: exact
    ({"hp_after": 0}, None),                         # a loss has no heal
    ({"relics_entering": ()}, None),                 # no ordinary heal
    ({"max_hp_entering": 4, "hp_after": 4}, (1, 4)),  # never below 1 HP
])
def test_capped_heal_band(overrides, band):
    assert review.capped_heal_band(fight(**overrides)) == band


@pytest.mark.parametrize("won,hp,expected", [
    (True, 74, True), (True, 80, True), (True, 73, False), (True, 81, False),
    (False, 77, False)])
def test_recorded_outcome_matches_a_band(won, hp, expected):
    actual = {"won": True, "final_hp": None, "final_hp_range": (74, 80)}
    assert rust_replay.recorded_outcome_matches(actual, won, hp) is expected


def test_recorded_outcome_matches_an_exact_value_unchanged():
    actual = {"won": True, "final_hp": 55}
    assert rust_replay.recorded_outcome_matches(actual, True, 55)
    assert not rust_replay.recorded_outcome_matches(actual, True, 56)
    assert not rust_replay.recorded_outcome_matches(actual, False, 55)


def generate(tmp_path, monkeypatch, the_fight, *, replay, witness):
    root = {"player": {"hp": the_fight.hp_entering}}
    run = SimpleNamespace(build_id="v0.111.0", seed="SEED", fights=[the_fight])
    monkeypatch.setattr(review, "build_replay_root",
                        lambda *args: (run, the_fight, root))
    digest = review.canonical_document.differential_digest(root)
    monkeypatch.setattr(review.subprocess, "run", lambda *a, **kw: SimpleNamespace(
        returncode=0, stdout=json.dumps({"entry_digest": digest, "best": {
            "won": True, "combat_hp": 60, "turn": 5, "actions": [],
            "final_digest": "end"}})))
    monkeypatch.setattr(review, "replay_line", lambda *a: [])
    monkeypatch.setattr(review.review, "simulator_document", lambda *a: {})
    calls = []

    def recorded(binary, entry, capture, actual):
        calls.append(dict(actual))
        if isinstance(witness, Exception):
            raise witness
        if not rust_replay.recorded_outcome_matches(actual, True, witness):
            raise ValueError("Rust recorded replay outcome differs from recorded result")
        return {"combat_hp": witness, "native_checkpoints": 9, "actions": [],
                "won": True, "final_digest": "d"}

    monkeypatch.setattr(review, "recorded_witness", recorded)
    config = ReviewConfig(rust_exact_solver_binary=tmp_path / "solver",
                          recorded_replay=replay)
    return review.generate(tmp_path / "run.json", 0, None, config), calls


def test_replay_inside_the_band_settles_the_capped_heal(tmp_path, monkeypatch):
    document, calls = generate(tmp_path, monkeypatch, fight(), replay={}, witness=80)
    assert document["status"] == "ok"
    assert document["actual"]["final_hp"] == 80
    assert document["actual"]["hp_lost"] == -12
    assert "final_hp_range" not in document["actual"]
    assert document["metadata"]["actual_final_hp_source"] == {
        "source": "recorded_replay", "reason": "capped_postcombat_heal",
        "band": [74, 80]}
    assert document["metadata"]["recorded_replay"]["status"] == "complete"
    assert len(calls) == 1 and calls[0]["final_hp_range"] == (74, 80)


@pytest.mark.parametrize("witness", [73, ValueError("recorded inputs end early")])
def test_replay_outside_the_band_or_failing_keeps_the_refusal(tmp_path, monkeypatch, witness):
    with pytest.raises(ReviewRefusal) as refusal:
        generate(tmp_path, monkeypatch, fight(), replay={}, witness=witness)
    assert refusal.value.reason == "observed_final_hp_ambiguous"


def test_without_a_recorded_replay_the_refusal_is_unchanged(tmp_path, monkeypatch):
    with pytest.raises(ReviewRefusal) as refusal:
        generate(tmp_path, monkeypatch, fight(), replay=None, witness=80)
    assert refusal.value.reason == "observed_final_hp_ambiguous"


def test_healing_below_the_uncapped_heal_still_refuses(tmp_path, monkeypatch):
    # The other ambiguous class is not a capped endpoint; no band applies.
    the_fight = fight(hp_after=70, hp_healed=2)
    with pytest.raises(ReviewRefusal, match="smaller than the uncapped"):
        review_summary.actual_outcome(the_fight)
    with pytest.raises(ReviewRefusal):
        generate(tmp_path, monkeypatch, the_fight, replay={}, witness=70)


def test_an_exact_outcome_does_not_consult_the_band(tmp_path, monkeypatch):
    document, calls = generate(tmp_path, monkeypatch, fight(hp_after=79), replay={}, witness=73)
    assert document["status"] == "ok"
    assert document["actual"]["final_hp"] == 73
    assert "actual_final_hp_source" not in document["metadata"]
    assert "final_hp_range" not in calls[0]
