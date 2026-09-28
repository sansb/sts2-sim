"""Fast contracts for the schema-v2 provenance consumer (#1068)."""

from __future__ import annotations

import base64
import json
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import review_provenance as provenance  # noqa: E402
from relay_parser import FightState, RunSummary  # noqa: E402


HERE = pathlib.Path(__file__).parent


def _fight(*, node=1, encounter="ENCOUNTER.SLIMES_WEAK",
           node_type="monster", monsters=None):
    return FightState(
        node_index=node, node_type=node_type, encounter_id=encounter,
        monster_ids=monsters or ["MONSTER.TWIG_SLIME_S"],
        hp_entering=80, max_hp_entering=80, gold_entering=99,
        deck_entering=[], relics_entering=[], potions_entering=[],
        damage_taken=0, hp_healed=0, turns_taken=1, potions_used=[],
        hp_after=80)


def _run(*fights):
    return RunSummary(
        seed="SEED", build_id="v0.110.1", schema_version=20,
        character="CHARACTER.IRONCLAD", ascension=10, win=True,
        killed_by=None, fights=list(fights))


def _record(candidates=(), capture_index=0):
    return {
        "capture_index": capture_index,
        "mcr": {"encoding": "base64", "data": base64.b64encode(b"x").decode()},
        "entry_candidates": list(candidates),
    }


def _candidate(observation, **counters):
    return {
        "observation": observation,
        "entry": {"run_rng": {
            name: {"counter": value, "s0": 0, "s1": 0, "s2": 0, "s3": 0}
            for name, value in counters.items()}},
    }


def _replay(*, encounter="SLIMES_WEAK", monsters=("TWIG_SLIME_S",),
            counters=None):
    return {
        "run": {
            "rng": {"seed": "SEED", "counters": counters or {
                "Shuffle": 9, "CombatCardSelection": 4}},
            "start_time": 10,
            "current_act_index": 0,
            "visited_map_coords": [[0, 0], [0, 1]],
            "map_point_history": [[{}]],
            "acts": [{"rooms": {
                "normal_encounter_ids": [encounter],
                "normal_encounters_visited": 0,
                "elite_encounter_ids": [], "elites_visited": 0,
                "bosses_visited": 0, "boss_id": None,
                "second_boss_id": None,
            }}],
        },
        "checksums": [{"full_state": {"creatures": [
            {"monster_id": None},
            *({"monster_id": item} for item in monsters),
        ]}}],
    }


def test_snake_pascal_mapping_is_nonvacuous_and_binding(monkeypatch):
    replay = {"Shuffle": 9, "CombatCardSelection": 4}
    candidate = _candidate(
        "previous", shuffle=9, combat_card_selection=4)
    mapped = provenance.candidate_counter_map(candidate)
    shared = set(mapped) & set(replay)

    assert len(shared) > 0, "resolution must compare real shared streams"
    assert mapped == replay
    assert provenance.resolve_entry_candidate(
        [candidate], replay).tier == "provenance_resolved"

    monkeypatch.setattr(provenance, "snake_to_pascal", lambda name: name)
    broken = provenance.resolve_entry_candidate([candidate], replay)
    assert broken.tier == "provenance_unresolved"


def test_resolution_requires_one_complete_map_and_rejects_legacy():
    replay = {"Shuffle": 9, "CombatCardSelection": 4}
    complete = _candidate(
        "previous", shuffle=9, combat_card_selection=4)
    partial = _candidate("previous", shuffle=9)
    duplicate = _candidate(
        "latest", shuffle=9, combat_card_selection=4)
    legacy = _candidate(
        "legacy_unverified", shuffle=9, combat_card_selection=4)

    assert provenance.resolve_entry_candidate(
        [partial], replay).tier == "provenance_unresolved"
    assert provenance.resolve_entry_candidate(
        [complete, duplicate], replay).tier == "provenance_unresolved"
    assert provenance.resolve_entry_candidate(
        [legacy], replay).tier == "provenance_legacy"


def test_real_replay_maps_by_history_depth_and_encounter_not_bundle_order():
    run_path = HERE / "testdata" / "8DVXPWUWRY.run"
    mcr_path = HERE / "testdata" / "8DVXPWUWRY_tunneler_win37.mcr"
    raw = json.loads(run_path.read_text())
    record = {
        "capture_index": 91,
        "mcr": {
            "encoding": "base64",
            "data": base64.b64encode(mcr_path.read_bytes()).decode(),
        },
        "entry_candidates": [],
    }

    analyzed = provenance.analyze_bundle(
        raw, {"schema_version": 2, "fights": [record]})
    enriched = [
        index for index, fight in enumerate(analyzed.fights)
        if fight.counter_overrides]

    assert enriched == [9]
    assert analyzed.fights[9].tier == "provenance_unresolved"
    assert dict(analyzed.fights[9].counter_overrides)["shuffle"] == 294
    assert [(item.reason, item.capture_index, item.fight_index)
            for item in analyzed.issues] == [
                ("candidate_unresolved", 91, 9)]


def test_ambiguous_unmatched_and_retried_records():
    same_a = _fight()
    same_b = _fight()
    run = _run(same_a, same_b)
    raw = {"start_time": 10}
    bundle = {"schema_version": 2, "fights": [_record()]}
    ambiguous = provenance.analyze_bundle(
        raw, bundle, decoder=lambda _data: _replay(), parsed_run=run)
    assert [issue.reason for issue in ambiguous.issues] == [
        "association_ambiguous"]
    assert all(not item.counter_overrides for item in ambiguous.fights)

    unmatched = provenance.analyze_bundle(
        raw, bundle,
        decoder=lambda _data: _replay(encounter="NIBBITS_WEAK"),
        parsed_run=_run(_fight()))
    assert [issue.reason for issue in unmatched.issues] == [
        "association_unmatched"]

    candidates = [_candidate(
        "previous", shuffle=9, combat_card_selection=4)]
    duplicates = provenance.analyze_bundle(
        raw, {"schema_version": 2, "fights": [
            _record(candidates, 1), _record(candidates, 2)]},
        decoder=lambda _data: _replay(), parsed_run=_run(_fight()))
    assert [issue.reason for issue in duplicates.issues] == [
        "superseded_fight_capture"]
    assert duplicates.fights[0].counter_overrides
    assert duplicates.fights[0].capture_index == 2
    assert duplicates.fights[0].capture_count == 2


def test_event_association_uses_exact_depth_and_replay_monster_identity():
    fight = _fight(
        node_type="event",
        encounter="ENCOUNTER.SLIMED_BERSERKER_NORMAL",
        monsters=["MONSTER.SLIMED_BERSERKER", "MONSTER.WRIGGLER"])
    candidates = [_candidate(
        "previous", shuffle=9, combat_card_selection=4)]
    analyzed = provenance.analyze_bundle(
        {"start_time": 10},
        {"schema_version": 2, "fights": [_record(candidates)]},
        decoder=lambda _data: _replay(monsters=("SLIMED_BERSERKER",)),
        parsed_run=_run(fight))

    assert analyzed.resolved_fights == (0,)
    assert analyzed.fights[0].tier == "provenance_resolved"
    assert analyzed.fights[0].entry == candidates[0]["entry"]
    assert analyzed.issues == ()


def test_unresolved_and_legacy_records_still_use_replay_counters():
    raw = {"start_time": 10}
    replay = _replay()
    unresolved = provenance.analyze_bundle(
        raw, {"schema_version": 2, "fights": [_record()]},
        decoder=lambda _data: replay, parsed_run=_run(_fight()))
    assert unresolved.fights[0].tier == "provenance_unresolved"
    assert dict(unresolved.fights[0].counter_overrides) == {
        "combat_card_selection": 4, "shuffle": 9}

    legacy = provenance.analyze_bundle(
        raw, {"schema_version": 2, "fights": [_record([
            _candidate("legacy_unverified", shuffle=9,
                       combat_card_selection=4)])]},
        decoder=lambda _data: replay, parsed_run=_run(_fight()))
    assert legacy.fights[0].tier == "provenance_legacy"
    assert dict(legacy.fights[0].counter_overrides) == {
        "combat_card_selection": 4, "shuffle": 9}
    assert unresolved.fights[0].entry is None
    assert legacy.fights[0].entry is None


def test_latest_capture_wins_even_if_array_reordered_or_its_entry_is_unresolved():
    old = _record([_candidate("previous", shuffle=9, combat_card_selection=4)], 1)
    latest = _record([], 2)
    result = provenance.analyze_bundle(
        {"start_time": 10}, {"schema_version": 2, "fights": [latest, old]},
        decoder=lambda _: _replay(), parsed_run=_run(_fight()))
    chosen = result.fights[0]
    assert chosen.capture_index == 2 and chosen.capture_count == 2
    assert chosen.replay is not None
    assert chosen.entry is None  # never borrow an earlier attempt's entry
    assert chosen.tier == "provenance_unresolved"


def test_retry_order_must_be_explicit_and_unambiguous():
    for records in ([_record([], 2), _record([], 2)],
                    [_record([], 2), {"mcr": _record()["mcr"]}]):
        result = provenance.analyze_bundle(
            {"start_time": 10}, {"schema_version": 2, "fights": records},
            decoder=lambda _: _replay(), parsed_run=_run(_fight()))
        assert result.fights[0].replay is None
        assert all(i.reason == "duplicate_fight_record" for i in result.issues)


def test_retries_retain_byte_derived_ids_and_do_not_mix_entry_provenance():
    import hashlib
    first = _record([_candidate("latest", shuffle=9, combat_card_selection=4)], 2)
    second = _record([_candidate("latest", shuffle=12, combat_card_selection=4)], 3)
    second["mcr"]["data"] = base64.b64encode(b"new attempt").decode()
    old_replay = _replay(counters={"Shuffle": 9, "CombatCardSelection": 4})
    new_replay = _replay(counters={"Shuffle": 12, "CombatCardSelection": 4})
    result = provenance.analyze_bundle(
        {"start_time": 10}, {"schema_version": 2, "fights": [second, first]},
        decoder=lambda data: old_replay if data == b"x" else new_replay,
        parsed_run=_run(_fight()))
    fight = result.fights[0]
    assert [c.id for c in fight.captures] == [hashlib.sha256(b).hexdigest() for b in (b"x", b"new attempt")]
    assert [c.capture_index for c in fight.captures] == [2, 3]
    assert fight.replay is new_replay
    assert dict(fight.counter_overrides)["shuffle"] == 12


# --- #3373: a branch run's pre-branch fights carry the SOURCE run's captures.

BRANCH_START, SOURCE_START, GRAND_START = 300, 200, 100


def _replay_at(depth, start_time, *, seed="SEED"):
    """A capture at 0-based history depth ``depth`` under ``start_time``."""
    replay = _replay()
    replay["run"]["rng"]["seed"] = seed
    replay["run"]["start_time"] = start_time
    replay["run"]["map_point_history"] = [[{}] * depth]
    replay["run"]["visited_map_coords"] = [[0, i] for i in range(depth + 1)]
    return replay


def _branch_analysis(depth, capture_start, *, lineage, seed="SEED"):
    candidates = [_candidate("previous", shuffle=9, combat_card_selection=4)]
    return provenance.analyze_bundle(
        {"start_time": BRANCH_START},
        {"schema_version": 2, "fights": [_record(candidates)]},
        decoder=lambda _: _replay_at(depth, capture_start, seed=seed),
        parsed_run=_run(_fight(node=depth)), lineage=lineage)


def _reasons(result):
    return [issue.reason for issue in result.issues]


def test_branch_accepts_source_capture_below_and_at_the_branch_floor():
    # Branched on the rewards screen after floor 17: depths 0..16 were the
    # source's, and depth 16 IS floor 17 -- the branch point's own fight.
    for depth in (1, 16):
        result = _branch_analysis(
            depth, SOURCE_START, lineage=[(SOURCE_START, 17)])
        assert result.resolved_fights == (0,), depth
        assert result.fights[0].tier == "provenance_resolved"
        assert result.issues == ()


def test_branch_rejects_source_capture_above_the_branch_floor():
    # Depth 17 (floor 18) was played by the branch itself: a source capture
    # there belongs to a different history.
    for depth in (17, 30):
        result = _branch_analysis(
            depth, SOURCE_START, lineage=[(SOURCE_START, 17)])
        assert _reasons(result) == ["run_identity_mismatch"], depth
        assert result.fights[0].tier == "provenance_unresolved"
        assert result.fights[0].replay is None


def test_branch_own_captures_are_unchanged_and_unbounded():
    result = _branch_analysis(20, BRANCH_START, lineage=[(SOURCE_START, 17)])
    assert result.resolved_fights == (0,)
    result = _branch_analysis(3, BRANCH_START, lineage=[(SOURCE_START, 17)])
    assert result.resolved_fights == (0,)


def test_source_start_time_with_the_wrong_seed_is_still_a_mismatch():
    result = _branch_analysis(
        5, SOURCE_START, lineage=[(SOURCE_START, 17)], seed="OTHER")
    assert _reasons(result) == ["run_identity_mismatch"]


def test_non_branch_run_rejects_every_foreign_start_time_as_before():
    for lineage in ((), []):
        result = _branch_analysis(1, SOURCE_START, lineage=lineage)
        assert _reasons(result) == ["run_identity_mismatch"]
    # An unrelated start_time is not rescued by an unrelated lineage.
    result = _branch_analysis(1, 999, lineage=[(SOURCE_START, 17)])
    assert _reasons(result) == ["run_identity_mismatch"]


def test_branch_of_branch_bounds_by_the_nearest_link_first():
    # C (this run) branched from B after floor 10; B branched from A after
    # floor 17. C inherits only B's floors <= 10, so an A capture is genuine
    # for C only below min(17, 10) = 10, and a B capture below 10.
    lineage = [(SOURCE_START, 10), (GRAND_START, 17)]
    for start in (SOURCE_START, GRAND_START):
        assert _branch_analysis(
            9, start, lineage=lineage).resolved_fights == (0,)
        assert _reasons(_branch_analysis(10, start, lineage=lineage)) == [
            "run_identity_mismatch"]
    # A nearer, looser link never widens a further, tighter one.
    lineage = [(SOURCE_START, 17), (GRAND_START, 5)]
    assert _branch_analysis(
        4, GRAND_START, lineage=lineage).resolved_fights == (0,)
    assert _reasons(_branch_analysis(5, GRAND_START, lineage=lineage)) == [
        "run_identity_mismatch"]
    assert _branch_analysis(
        16, SOURCE_START, lineage=lineage).resolved_fights == (0,)


def test_malformed_lineage_refuses_rather_than_widening():
    for lineage in ("x", [(SOURCE_START,)], [(SOURCE_START, 0)],
                    [(str(SOURCE_START), 17)], [(SOURCE_START, True)],
                    {"a": 1}):
        with pytest.raises(ValueError):
            _branch_analysis(1, SOURCE_START, lineage=lineage)


def test_capture_identity_admitted_is_the_association_rule():
    lineage = [(SOURCE_START, 17)]
    admitted = provenance.capture_identity_admitted
    assert admitted(BRANCH_START, BRANCH_START, 40, lineage)
    assert admitted(SOURCE_START, BRANCH_START, 16, lineage)
    assert not admitted(SOURCE_START, BRANCH_START, 17, lineage)
    assert not admitted(SOURCE_START, BRANCH_START, 1, ())
    assert not admitted(999, BRANCH_START, 1, lineage)
    assert provenance.parse_lineage([[SOURCE_START, 17]]) == (
        (SOURCE_START, 17),)
    for bad in ({"links": []}, [[SOURCE_START, 0]], [["1", 2]]):
        with pytest.raises(ValueError):
            provenance.parse_lineage(bad)
