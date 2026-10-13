"""Fast contracts for the resolved-save projection checks.

`provenance_entry_for_fight` gates the capture run the eval suite's upload
import writes (`rust_review.replay_capture_run`). These tests moved here from
`test_review_summary_v2.py` when the review CLI was deleted (2026-10-11).
"""

import copy
import json
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import review_provenance as provenance  # noqa: E402
import provenance_entry as review  # noqa: E402
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



def _joss_swapped_entry():
    _path, run, original = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    projection = copy.deepcopy(original)
    old_relic = fight.relics_entering[2]
    fight.relics_entering[2] = "RELIC.JOSS_PAPER"
    fight.relic_counters.pop(old_relic, None)
    fight.relic_counters["RELIC.JOSS_PAPER"] = {
        "CardsExhausted": 0, "EtherealCount": 0}
    relic = projection["players"][0]["relics"][2]
    relic["id"] = "RELIC.JOSS_PAPER"
    relic.pop("props", None)
    return run, fight, projection, relic


def test_joss_paper_saved_without_props_is_complete_at_zero():
    """#3746: the game omits Joss Paper's only saved counter at zero
    (``SaveIfNotTypeDefault``), so the relic row carries no ``props``.
    XP0KJLT0K4NL node 26 and 17 more archive fights were refused for it."""
    run, fight, projection, relic = _joss_swapped_entry()

    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character)
    assert entry["relic_counters"]["RELIC.JOSS_PAPER"] == {
        "CardsExhausted": 0, "EtherealCount": 0}

    relic["props"] = None
    review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character)

    # The decoded zero is still a claim: a `.run` remainder disagrees by name.
    fight.relic_counters["RELIC.JOSS_PAPER"] = {
        "CardsExhausted": 3, "EtherealCount": 0}
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_conflict", "relic_props")


def test_missing_persistent_props_stay_incomplete_outside_joss_paper():
    run, fight, projection, relic = _joss_swapped_entry()

    relic["props"] = []                          # present, but not a mapping
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert raised.value.check == "relic_persistent_props"

    relic.pop("props")
    relic["id"] = "RELIC.FUR_COAT"               # saved unconditionally
    fight.relics_entering[2] = "RELIC.FUR_COAT"
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_incomplete", "relic_persistent_props")


def test_save_only_card_props_are_evidence_not_a_conflict():
    """#3746: the parser reconstructs no field of Spoils Map, Guilty or
    Dowsing, so their saved props have nothing on the run side to
    contradict (1J10MCU2JZ8V node 4 and 64 more archive fights). A card
    whose props the parser does claim still conflicts by name."""
    _path, run, original = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    projection = copy.deepcopy(original)
    save_card = next(row for row in projection["players"][0]["deck"]
                     if row["id"] == "CARD.STAMPEDE")
    run_card = next(row for row in fight.deck_entering
                    if row["id"] == "CARD.STAMPEDE")
    save_card["id"] = run_card["id"] = "CARD.SPOILS_MAP"
    save_card["props"] = {"ints": [{"name": "SpoilsActIndex", "value": 1}]}

    review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character)

    save_card["id"] = run_card["id"] = "CARD.GENETIC_ALGORITHM"
    save_card["props"] = {"ints": [
        {"name": "CurrentBlock", "value": 7},
        {"name": "IncreasedBlock", "value": 6}]}
    run_card["props"] = {"ints": [
        {"name": "CurrentBlock", "value": 6},
        {"name": "IncreasedBlock", "value": 5}]}
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_conflict", "deck_props")

    # An unknown run-side value is still a claim the parser made.
    run_card["props"] = None
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert raised.value.check == "deck_props"


# --- #3756: Fur Coat's mark count, and a save one reward behind -------------

_FUR_MARKS = [(6, 8), (5, 11), (3, 12), (4, 3), (6, 10), (4, 8), (4, 6),
              (6, 11)]


def _fur_props(marks, *, coords_set=True, flat=False):
    cols = [col for col, _row in marks]
    rows = [row for _col, row in marks]
    if flat:
        return {"FurCoatActIndex": 2, "FurCoatCoordCols": cols,
                "FurCoatCoordRows": rows, "FurCoatCoordsSet": coords_set}
    return {
        "ints": [{"name": "FurCoatActIndex", "value": 2}],
        "bools": [{"name": "FurCoatCoordsSet", "value": coords_set}],
        "int_arrays": [{"name": "FurCoatCoordCols", "value": cols},
                       {"name": "FurCoatCoordRows", "value": rows}],
    }


@pytest.mark.parametrize("flat", [False, True])
def test_fur_coat_marks_eight_fights_in_this_build(flat):
    """#3756: `FurCoat::get_CanonicalVars` (RVA 0x93ee2) is `Combats` 8 in
    v0.111.0; the adapter still demanded the 7 of v0.109.1 and answered
    "unprovable" for every Fur Coat this build writes (MW6WGHZ9XF82 and
    D6P38T5JU3LW, 19 uploaded fights)."""
    member = review.live_coach._strict_fur_coat_membership
    assert review.live_coach.FUR_COAT_COMBATS == len(_FUR_MARKS) == 8
    props = _fur_props(_FUR_MARKS, flat=flat)
    for col, row in _FUR_MARKS:
        assert member(props, [col, row]) is True
    assert member(props, [4, 1]) is False
    # `Take(Combats)` of what the act has left: fewer is a whole answer too.
    assert member(_fur_props(_FUR_MARKS[:7], flat=flat), [4, 6]) is True
    assert member(_fur_props(_FUR_MARKS[:1], flat=flat), [4, 6]) is False
    assert member(_fur_props([], flat=flat), [4, 6]) is False


@pytest.mark.parametrize("flat", [False, True])
def test_fur_coat_marks_the_writer_cannot_produce_stay_unprovable(flat):
    member = review.live_coach._strict_fur_coat_membership
    # More than `Combats` marks.
    assert member(_fur_props([*_FUR_MARKS, (0, 1)], flat=flat),
                  [6, 8]) is None
    # One map point taken twice.
    assert member(_fur_props([*_FUR_MARKS[:7], _FUR_MARKS[0]], flat=flat),
                  [6, 8]) is None
    # Coordinates without the latch.
    assert member(_fur_props(_FUR_MARKS, coords_set=False, flat=flat),
                  [6, 8]) is None
    assert member(_fur_props([], coords_set=False, flat=flat),
                  [6, 8]) is False


def _fur_coat_entry(marks, coord):
    _path, run, original = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    projection = copy.deepcopy(original)
    old_relic = fight.relics_entering[2]
    fight.relics_entering[2] = "RELIC.FUR_COAT"
    fight.relic_counters.pop(old_relic, None)
    relic = projection["players"][0]["relics"][2]
    relic["id"] = "RELIC.FUR_COAT"
    relic["props"] = _fur_props(marks)
    replay = {"run": {
        "visited_map_coords": [[0, 0], list(coord)],
        "players": [{"character": "IRONCLAD", "deck": [
            {"id": row["id"].removeprefix("CARD."),
             "upgrade_level": row["upgrade_level"],
             "enchantment": None, "props": None}
            for row in projection["players"][0]["deck"]]}]}}
    return run, fight, projection, replay


def test_the_entry_gate_passes_a_fur_coat_with_eight_marks():
    run, fight, projection, replay = _fur_coat_entry(_FUR_MARKS, (4, 8))
    fight.fur_coat_active = True
    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character, replay=replay)
    assert entry["fur_coat_active"] is True

    # The `.run`'s own bit is still a claim: an unmarked room disagrees.
    run, fight, projection, replay = _fur_coat_entry(_FUR_MARKS, (4, 1))
    fight.fur_coat_active = True
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character,
            replay=replay)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_conflict", "fur_coat_props")

    # Nine marks: no v0.111.0 writer made this latch.
    run, fight, projection, replay = _fur_coat_entry(
        [*_FUR_MARKS, (0, 1)], (4, 8))
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character,
            replay=replay)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_incomplete", "fur_coat_props")


def _save_one_reward_behind():
    """The save candidate without its last card; the capture snapshot and
    the `.run` deck both with it."""
    _path, run, original = _real_resolved_entry()
    fight = copy.deepcopy(run.fights[2])
    projection = copy.deepcopy(original)
    deck = projection["players"][0]["deck"]
    snapshot = [
        {"id": row["id"].removeprefix("CARD."),
         "upgrade_level": row["upgrade_level"],
         "enchantment": None, "props": None,
         "floor_added_to_deck": row.get("floor_added_to_deck")}
        for row in deck]
    last = deck.pop()
    replay = {"run": {"players": [{
        "character": "IRONCLAD", "deck": snapshot}]}}
    return run, fight, projection, replay, last


def test_a_save_written_before_a_card_reward_is_completed_from_the_capture():
    """#3756, STGS76WZ6WFA node 20: the resolved save was written while the
    previous node's reward was still on screen, so it is one card short of
    the deck the capture's own snapshot and the `.run` both record."""
    run, fight, projection, replay, last = _save_one_reward_behind()
    before = copy.deepcopy(projection)

    entry, _pools = review.provenance_entry_for_fight(
        fight, projection, expected_character=run.character, replay=replay)

    assert [row["id"] for row in entry["deck_entering"]] == [
        "CARD." + row["id"] for row in replay["run"]["players"][0]["deck"]]
    assert entry["deck_entering"][-1]["id"] == last["id"]
    assert entry["deck_entering"][-1]["upgrade_level"] == last["upgrade_level"]
    assert projection == before, "the projection itself is not edited"


def _drop_snapshot_card(replay, _fight, _last):
    replay["run"]["players"][0]["deck"].pop()


def _move_gained_card_to_the_front(replay, _fight, _last):
    deck = replay["run"]["players"][0]["deck"]
    deck.insert(0, deck.pop())


def _unleveled(replay, _fight, _last):
    replay["run"]["players"][0]["deck"][-1]["upgrade_level"] = None


def _enchanted(replay, _fight, _last):
    replay["run"]["players"][0]["deck"][-1]["enchantment"] = {
        "id": "SHARP", "level": 2, "props": None}


def _with_props(replay, _fight, _last):
    replay["run"]["players"][0]["deck"][-1]["props"] = {"SpoilsActIndex": 1}


def _persistent_id(replay, fight, last):
    replay["run"]["players"][0]["deck"][-1]["id"] = "THE_SCYTHE"
    row = next(row for row in reversed(fight.deck_entering)
               if row["id"] == last["id"])
    row["id"] = "CARD.THE_SCYTHE"


def _second_player(replay, _fight, _last):
    replay["run"]["players"].append({"character": "SILENT", "deck": []})


def _deck_is_not_a_list(replay, _fight, _last):
    replay["run"]["players"][0]["deck"] = {"cards": []}


def _row_without_an_id(replay, _fight, _last):
    replay["run"]["players"][0]["deck"][0] = {"upgrade_level": 0}


@pytest.mark.parametrize("edit", [
    _drop_snapshot_card,             # the capture does not back the `.run`
    _move_gained_card_to_the_front,  # not a card appended after the save
    _unleveled,
    _enchanted,
    _with_props,
    _persistent_id,
    _second_player,
    _deck_is_not_a_list,
    _row_without_an_id,
], ids=lambda edit: edit.__name__.strip("_"))
def test_a_short_save_the_capture_does_not_settle_stays_refused(edit):
    run, fight, projection, replay, last = _save_one_reward_behind()
    edit(replay, fight, last)
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character,
            replay=replay)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_conflict", "deck_membership")


def test_a_short_save_without_a_capture_stays_refused():
    run, fight, projection, _replay, _last = _save_one_reward_behind()
    with pytest.raises(review.ProvenanceEntryRejected) as raised:
        review.provenance_entry_for_fight(
            fight, projection, expected_character=run.character)
    assert (raised.value.reason, raised.value.check) == (
        "provenance_entry_conflict", "deck_membership")
