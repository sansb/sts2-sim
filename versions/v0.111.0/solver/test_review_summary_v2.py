"""Fast contracts for the review CLI and its shared entry checks.

The resolved-save projection checks (`provenance_entry_for_fight`), the
`.run` copy-only canonicalization and the CLI routing. The retired Python
producer's tests (benchmark-only cards, placeholder counters, the
classify-only seam) were deleted with it by #2827 item F1.
"""

import copy
import json
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import review_summary as phase1  # noqa: E402
import review_provenance as provenance  # noqa: E402
import review_summary_v2 as review  # noqa: E402
from relay_parser import FightState, parse_run  # noqa: E402


HERE = pathlib.Path(__file__).parent


def _fight(**updates):
    values = {
        "node_index": 1,
        "node_type": "monster",
        "encounter_id": "ENCOUNTER.SEAPUNK_WEAK",
        "monster_ids": ["MONSTER.SEAPUNK"],
        "hp_entering": 70,
        "max_hp_entering": 80,
        "gold_entering": 99,
        "deck_entering": [],
        "relics_entering": [],
        "potions_entering": [],
        "damage_taken": 3,
        "hp_healed": 0,
        "turns_taken": 4,
        "potions_used": [],
        "hp_after": 67,
    }
    values.update(updates)
    return FightState(**values)


def _full_v1_document():
    return {
        "schema_version": 1,
        "status": "ok",
        "fight": {"run_file": "sample.run", "fight_index": 0},
        "actual": {"won": True},
        "best_actual_seed": {"won": True},
        "benchmark": {"world_count": 2},
        "derived": {"skill_gap": {}, "luck_percentile": {}},
        "metadata": {
            "k": 2,
            "fully_unlocked_card_pool": False,
        },
    }


def _real_resolved_entry():
    run_path = HERE / "testdata" / "85920V7XQFSN.run"
    raw = json.loads(run_path.read_text())
    record = json.loads((
        HERE / "testdata" /
        "85920V7XQFSN_fight2_provenance.json").read_text())
    analyzed = provenance.analyze_bundle(
        raw, {"schema_version": 2, "fights": [record]})
    assert analyzed.resolved_fights == (2,)
    return run_path, parse_run(str(run_path)), analyzed.fights[2].entry


def test_real_resolved_save_entry_preserves_physical_and_relic_order():
    _path, run, projection = _real_resolved_entry()
    entry, pools = review.provenance_entry_for_fight(
        run.fights[2], projection, expected_character=run.character)
    player = projection["players"][0]

    assert [row["id"] for row in entry["deck_entering"]] == [
        row["id"] for row in player["deck"]]
    assert [row["upgrade_level"] for row in entry["deck_entering"]] == [
        row["upgrade_level"] for row in player["deck"]]
    assert entry["relics_entering"] == [
        row["id"] for row in player["relics"]]
    assert entry["relics_entering_dispatch_ordered"] is True
    assert pools.fully_unlocked_card_pool is True
    assert pools.fully_unlocked_potion_pool is True
    assert "IRONCLAD7_EPOCH" in pools.unlocked_card_pool_epochs


def test_ambiguous_run_copy_is_replaced_but_unambiguous_conflict_refuses():
    _path, run, projection = _real_resolved_entry()
    ambiguous = copy.deepcopy(run.fights[2])
    strike = next(row for row in ambiguous.deck_entering
                  if row["id"] == "CARD.STRIKE_IRONCLAD")
    strike["upgrade_level"] = 1
    strike["upgrade_ambiguous"] = True

    entry, _pools = review.provenance_entry_for_fight(
        ambiguous, projection, expected_character=run.character)
    assert all("upgrade_ambiguous" not in row
               for row in entry["deck_entering"])
    assert [row["upgrade_level"] for row in entry["deck_entering"]] == [
        row["upgrade_level"] for row in projection["players"][0]["deck"]]

    conflicting = copy.deepcopy(run.fights[2])
    next(row for row in conflicting.deck_entering
         if row["id"] == "CARD.STRIKE_IRONCLAD")["upgrade_level"] = 1
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            conflicting, projection, expected_character=run.character)
    assert raised.value.reason == "provenance_entry_conflict"
    assert raised.value.check == "deck_upgrade"


def test_capture_salvages_ambiguous_upgrade_but_not_unrelated_fields():
    """Capture authority is per field, not a blanket trust override."""
    _path, run, projection = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    strike = next(row for row in fight.deck_entering
                  if row["id"] == "CARD.STRIKE_IRONCLAD")
    saved_strike = next(row for row in projection["players"][0]["deck"]
                        if row["id"] == "CARD.STRIKE_IRONCLAD")
    strike["upgrade_level"] = saved_strike["upgrade_level"] + 1
    strike["upgrade_ambiguous"] = True

    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character)
    exact_strike = next(row for row in entry["deck_entering"]
                        if row["id"] == "CARD.STRIKE_IRONCLAD")
    assert exact_strike["upgrade_level"] == saved_strike["upgrade_level"]
    assert "upgrade_ambiguous" not in exact_strike

    conflicting = copy.deepcopy(projection)
    projected_strike = next(
        row for row in conflicting["players"][0]["deck"]
        if row["id"] == "CARD.STRIKE_IRONCLAD")
    projected_strike["enchantment"] = {
        "id": "ENCHANTMENT.GOOPY", "amount": 1}
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, conflicting, expected_character=run.character)
    assert raised.value.reason == "provenance_entry_conflict"
    assert raised.value.check == "deck_enchantment"


def test_recorded_entry_snapshot_wins_over_capture_for_ambiguous_upgrade():
    """The replay's combat-entry deck resolves a disputed saved projection."""
    _path, run, projection = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    strike_index = next(
        index for index, row in enumerate(fight.deck_entering)
        if row["id"] == "CARD.STRIKE_IRONCLAD")
    fight.deck_entering[strike_index]["upgrade_ambiguous"] = True
    replay_deck = [{
        "id": row["id"].removeprefix("CARD."),
        "upgrade_level": row["upgrade_level"],
    } for row in projection["players"][0]["deck"]]
    replay_deck[strike_index]["upgrade_level"] = 1
    replay = {"run": {"players": [{
        "character": "IRONCLAD", "deck": replay_deck,
    }]}}

    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character, replay=replay)

    assert entry["deck_entering"][strike_index]["upgrade_level"] == 1


def test_recorded_entry_snapshot_cannot_override_unambiguous_upgrade():
    _path, run, projection = _real_resolved_entry()
    replay_deck = [{
        "id": row["id"].removeprefix("CARD."),
        "upgrade_level": row["upgrade_level"],
    } for row in projection["players"][0]["deck"]]
    replay_deck[0]["upgrade_level"] += 1
    replay = {"run": {"players": [{
        "character": "IRONCLAD", "deck": replay_deck,
    }]}}

    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            run.fights[2], projection, expected_character=run.character,
            replay=replay)
    assert raised.value.reason == "provenance_entry_conflict"
    assert raised.value.check == "recorded_replay_deck_upgrade"


def test_save_resolves_goopy_card_props_and_joss_persistent_state():
    _path, run, original = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    projection = copy.deepcopy(original)
    save_deck = projection["players"][0]["deck"]

    save_deck[0]["enchantment"] = {
        "id": "ENCHANTMENT.GOOPY", "amount": 5}
    run_strike = next(row for row in fight.deck_entering
                      if row["id"] == "CARD.STRIKE_IRONCLAD")
    run_strike.update({
        "enchantment": "ENCHANTMENT.GOOPY", "enchant_amount": 99,
        "enchant_ambiguous": True,
    })

    save_genetic = next(row for row in save_deck
                        if row["id"] == "CARD.STAMPEDE")
    save_genetic["id"] = "CARD.GENETIC_ALGORITHM"
    save_genetic["props"] = {"ints": [
        {"name": "CurrentBlock", "value": 6},
        {"name": "IncreasedBlock", "value": 5},
    ]}
    run_genetic = next(row for row in fight.deck_entering
                       if row["id"] == "CARD.STAMPEDE")
    run_genetic.update({
        "id": "CARD.GENETIC_ALGORITHM", "props": None,
        "props_ambiguous": True,
    })

    save_relic = projection["players"][0]["relics"][2]
    old_relic = fight.relics_entering[2]
    save_relic["id"] = "RELIC.JOSS_PAPER"
    save_relic["props"] = {"ints": [
        {"name": "CardsExhausted", "value": 3},
        {"name": "EtherealCount", "value": 0},
    ]}
    fight.relics_entering[2] = "RELIC.JOSS_PAPER"
    fight.relic_counters.pop(old_relic, None)

    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character)

    assert entry["deck_entering"][0]["enchantment"] == \
        "ENCHANTMENT.GOOPY"
    assert entry["deck_entering"][0]["enchant_amount"] == 5
    exact_genetic = next(row for row in entry["deck_entering"]
                         if row["id"] == "CARD.GENETIC_ALGORITHM")
    assert exact_genetic["props"] == save_genetic["props"]
    assert "props_ambiguous" not in exact_genetic
    assert entry["relic_counters"]["RELIC.JOSS_PAPER"] == {
        "CardsExhausted": 3, "EtherealCount": 0}


def test_incomplete_resolved_entry_is_rejected_atomically():
    _path, run, projection = _real_resolved_entry()
    incomplete = copy.deepcopy(projection)
    incomplete["players"][0].pop("relic_grab_bag")

    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            run.fights[2], incomplete, expected_character=run.character)
    assert raised.value.reason == "provenance_entry_incomplete"
    assert raised.value.check == "player_envelope"


def test_schema_v5_assigns_the_headline_semantic_line_id():
    document = _full_v1_document()
    document["best_actual_seed"] = {
        "exact": True,
        "claim": {"kind": "exact", "display": "exact"},
        "line": [{"turn": 1, "actions": [{"kind": "end"}]}],
    }
    document["benchmark"] = {
        "outcomes": [{
            "world": 0,
            "exact": False,
            "claim": {
                "kind": "achieved", "display": "achieved (lower bound)"},
            "line": [{"turn": 1, "actions": [{"kind": "end"}]}],
        }],
    }

    upgraded = review._upgrade_document(
        document, assumed_fully_unlocked=False)

    assert upgraded["schema_version"] == review.SCHEMA_VERSION
    assert upgraded["best_actual_seed"]["line_id"] == "best"
    assert upgraded["benchmark"]["outcomes"][0]["world"] == 0
    assert "line_id" not in upgraded["benchmark"]["outcomes"][0]


def test_differing_duplicate_attributes_keep_multiset_fork_refusal():
    rows = [
        {"id": "CARD.BULLY", "upgrade_level": 1, "floor_added": 1,
         "enchantment": "ENCHANTMENT.CORRUPTED", "enchant_amount": 1,
         "upgrade_ambiguous": True},
        {"id": "CARD.BULLY", "upgrade_level": 1, "floor_added": 1,
         "enchantment": None, "upgrade_ambiguous": True},
    ]
    fight = _fight(deck_entering=rows)
    raw = {
        "players": [{"deck": [
            {"id": "CARD.BULLY", "current_upgrade_level": 1,
             "floor_added_to_deck": 1,
             "enchantment": {
                 "id": "ENCHANTMENT.CORRUPTED", "amount": 1}},
            {"id": "CARD.BULLY", "current_upgrade_level": 1,
             "floor_added_to_deck": 1},
        ]}],
        "map_point_history": [[
            {"player_stats": [{}]},
            {"player_stats": [{}]},
            {"player_stats": [{"upgraded_cards": ["CARD.BULLY"]}]},
        ]],
    }
    with pytest.raises(
            phase1.ReviewRefusal, match="forks the entry multiset"):
        review.canonicalize_entry_multiset(fight, raw)


def test_later_acquired_upgraded_copy_keeps_upgrade_multiset_refusal():
    rows = [
        {"id": "CARD.BULLY", "upgrade_level": 1, "floor_added": 1,
         "enchantment": None, "upgrade_ambiguous": True},
        {"id": "CARD.BULLY", "upgrade_level": 0, "floor_added": 1,
         "enchantment": None, "upgrade_ambiguous": True},
    ]
    fight = _fight(deck_entering=rows)
    raw = {
        "players": [{"deck": [
            {"id": "CARD.BULLY", "current_upgrade_level": 1,
             "floor_added_to_deck": 1},
            {"id": "CARD.BULLY", "current_upgrade_level": 0,
             "floor_added_to_deck": 1},
            {"id": "CARD.BULLY", "current_upgrade_level": 1,
             "floor_added_to_deck": 2},
        ]}],
        "map_point_history": [[
            {"player_stats": [{}]},
            {"player_stats": [{}]},
            {"player_stats": [{"upgraded_cards": ["CARD.BULLY"]}]},
        ]],
    }
    with pytest.raises(
            phase1.ReviewRefusal, match="outside the fight entry"):
        review.canonicalize_entry_multiset(fight, raw)


def test_non_goopy_enchant_copy_assignment_canonicalizes_exact_multiset():
    rows = [
        {"id": "CARD.DEFEND_IRONCLAD", "upgrade_level": 0,
         "floor_added": 1, "enchantment": "ENCHANTMENT.PERFECT_FIT",
         "enchant_amount": 1, "enchant_ambiguous": True},
        {"id": "CARD.DEFEND_IRONCLAD", "upgrade_level": 0,
         "floor_added": 1, "enchantment": "ENCHANTMENT.PERFECT_FIT",
         "enchant_amount": 1, "enchant_ambiguous": True},
    ]
    fight = _fight(node_index=1, deck_entering=rows)

    def raw_card():
        return {
            "id": "CARD.DEFEND_IRONCLAD", "floor_added_to_deck": 1,
            "current_upgrade_level": 0,
            "enchantment": {
                "id": "ENCHANTMENT.PERFECT_FIT", "amount": 1},
        }

    def event_point():
        return {"player_stats": [{"cards_enchanted": [{
            "card": raw_card(),
            "enchantment": "ENCHANTMENT.PERFECT_FIT",
        }]}]}

    raw = {
        "players": [{"deck": [raw_card(), raw_card()]}],
        "map_point_history": [[
            event_point(), {"player_stats": [{}]}, event_point()]],
    }
    entry, reasons = review.canonicalize_entry_multiset(fight, raw)
    canonical = entry["deck_entering"]

    assert reasons == [review.ENCHANT_COPY_REASON]
    assert sum(row["enchantment"] is not None for row in canonical) == 1
    assert all("enchant_ambiguous" not in row for row in canonical)


def test_non_goopy_enchant_amount_mismatch_keeps_multiset_refusal():
    row = {
        "id": "CARD.DEFEND_IRONCLAD", "upgrade_level": 0,
        "floor_added": 1, "enchantment": "ENCHANTMENT.PERFECT_FIT",
        "enchant_amount": 2, "enchant_ambiguous": True,
    }
    fight = _fight(node_index=1, deck_entering=[row])
    raw = {
        "players": [{"deck": [{
            "id": "CARD.DEFEND_IRONCLAD", "floor_added_to_deck": 1,
            "current_upgrade_level": 0,
            "enchantment": {
                "id": "ENCHANTMENT.PERFECT_FIT", "amount": 2},
        }]}],
        "map_point_history": [[
            {"player_stats": [{"cards_enchanted": [{
                "card": {
                    "id": "CARD.DEFEND_IRONCLAD",
                    "floor_added_to_deck": 1,
                    "enchantment": {
                        "id": "ENCHANTMENT.PERFECT_FIT", "amount": 1},
                },
                "enchantment": "ENCHANTMENT.PERFECT_FIT",
            }]}]},
            {"player_stats": [{}]},
        ]],
    }
    with pytest.raises(
            phase1.ReviewRefusal, match="dated and endpoint amounts differ"):
        review.canonicalize_entry_multiset(fight, raw)


def test_simulator_identity_document_tracks_explicit_rust_binary(tmp_path):
    binary = tmp_path / "sts-sim"
    binary.write_bytes(b"review-native-v1")
    binary.chmod(0o755)

    document = review.simulator_identity_document(binary)

    assert document["simulator"]["rust_exact_solver"] == \
        review.sim_identity.exact_solver_artifact_identity(binary)


def _cli_config(seen):
    def generate(*args, **kwargs):
        seen.append((args, kwargs))
        return {"schema_version": review.SCHEMA_VERSION, "status": "ok"}
    return generate


def test_cli_run_only_review_is_the_rust_legacy_path(monkeypatch, capsys):
    seen = []
    monkeypatch.setattr(review, "generate_review_document", _cli_config(seen))
    assert review.main([
        "sample.run", "0", "-k", "2",
        "--potion-generation-counter", "6"]) == 0
    assert json.loads(capsys.readouterr().out)["status"] == "ok"
    ((args, kwargs),) = seen
    assert args == (pathlib.Path("sample.run"), 0)
    assert set(kwargs) == {"config"}
    assert kwargs["config"].potion_generation_counter == 6


def test_cli_floor_saves_and_capture_route_to_the_rust_review(
        monkeypatch, tmp_path, capsys):
    import rust_review
    seen = []
    monkeypatch.setattr(
        rust_review, "generate",
        lambda run, index, saves, config, *, provenance_entry,
        branch_lineage: seen.append(
            (saves, provenance_entry, config.recorded_replay,
             branch_lineage)) or {
                "schema_version": 8, "status": "refused"})
    monkeypatch.setattr(
        review, "generate_review_document",
        lambda *a, **k: pytest.fail("a captured fight must not take the "
                                    ".run-only path"))
    saves = tmp_path / "saves.json"
    saves.write_text(json.dumps([{"id": "one"}]))
    assert review.main(["sample.run", "1", "--floor-saves", str(saves)]) == 2
    replay = tmp_path / "replay.json"
    replay.write_text(json.dumps({"events": []}))
    entry = tmp_path / "entry.json"
    entry.write_text(json.dumps({"players": []}))
    assert review.main([
        "sample.run", "1", "--recorded-replay", str(replay),
        "--provenance-entry", str(entry)]) == 2
    assert review.main([
        "sample.run", "1", "--recorded-replay", str(replay),
        "--provenance-entry", str(entry),
        "--branch-lineage", "[[1790483229, 17]]"]) == 2
    capsys.readouterr()
    assert seen == [([{"id": "one"}], None, None, ()),
                    (None, {"players": []}, {"events": []}, ()),
                    (None, {"players": []}, {"events": []},
                     ((1790483229, 17),))]


def test_cli_reports_a_bad_provenance_transport_on_the_run_only_path(
        monkeypatch, tmp_path, capsys):
    seen = []
    monkeypatch.setattr(review, "generate_review_document", _cli_config(seen))
    entry = tmp_path / "entry.json"
    entry.write_text("[")
    status = tmp_path / "entry-status.json"
    assert review.main([
        "sample.run", "0", "--provenance-entry", str(entry),
        "--provenance-entry-status", str(status)]) == 0
    capsys.readouterr()
    assert json.loads(status.read_text()) == {
        "reason": "provenance_entry_incomplete", "check": "transport_json"}
    assert len(seen) == 1


def test_cli_branch_lineage_is_parsed_by_the_association_rule(capsys):
    """#3373: the worker hands the CLI a branch run's ancestry as JSON."""
    assert review.parse_args(["sample.run", "0"]).branch_lineage == ()
    assert review.parse_args([
        "sample.run", "0", "--branch-lineage", "[[1790483229, 17]]",
    ]).branch_lineage == ((1790483229, 17),)
    for bad in ("{}", "[[1, 0]]", "not json", '[["1", 2]]'):
        with pytest.raises(SystemExit):
            review.parse_args(["sample.run", "0", "--branch-lineage", bad])
        assert "--branch-lineage" in capsys.readouterr().err


def test_cli_classify_only_seam_is_retired(capsys):
    with pytest.raises(SystemExit):
        review.parse_args(["sample.run", "0", "--classify-only"])
    assert "--classify-only" in capsys.readouterr().err

