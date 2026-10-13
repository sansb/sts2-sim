"""The run parser's per-id upgrade ledger (#3746).

Each case is a shape the uploader archive rejected as a run-versus-save
``deck_upgrade`` conflict: the save at that fight is the witness named in the
docstring, and the synthetic run reproduces only the events that decide it.
"""

from __future__ import annotations

import json
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import relay_parser  # noqa: E402
from relay_parser import parse_run  # noqa: E402


def _stats(**updates):
    stats = {
        "current_hp": 80, "damage_taken": 0, "hp_healed": 0,
        "max_hp": 80, "max_hp_gained": 0, "max_hp_lost": 0,
        "current_gold": 99, "gold_gained": 0, "gold_lost": 0,
        "gold_spent": 0, "gold_stolen": 0,
    }
    stats.update(updates)
    return stats


def _node(node_type, *, rooms=None, **stats):
    if rooms is None:
        rooms = [node_type]
    built = []
    for room_type in rooms:
        room = {"room_type": room_type}
        if room_type in ("monster", "elite", "boss"):
            room.update({
                "turns_taken": 2,
                "model_id": "ENCOUNTER.NIBBITS_WEAK",
                "monster_ids": ["MONSTER.NIBBIT"],
            })
        built.append(room)
    return {
        "map_point_type": node_type,
        "rooms": built,
        "player_stats": [_stats(**stats)],
    }


def _card(card_id, floor=1, level=0):
    return {"id": card_id, "floor_added_to_deck": floor,
            "current_upgrade_level": level}


def _picked(card_id, floor, level=0):
    """A picked reward: `.run` stamps the card with its floor and level."""
    return {"card_choices": [
        {"was_picked": True,
         "card": {"id": card_id, "floor_added_to_deck": floor,
                  "current_upgrade_level": level}}]}


def _parse(tmp_path, history, deck, relics=()):
    run = {
        "seed": "ISSUE3746", "build_id": "v0.111.0", "schema_version": 9,
        "ascension": 10, "win": True, "killed_by_encounter": "NONE.NONE",
        "map_point_history": [history],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": deck, "relics": list(relics),
        }],
    }
    path = tmp_path / "issue3746.run"
    path.write_text(json.dumps(run))
    return parse_run(str(path))


def _copies(fight, card_id):
    return [(row["upgrade_level"], bool(row.get("upgrade_ambiguous")))
            for row in fight.deck_entering if row["id"] == card_id]


def test_combat_nodes_own_upgrade_follows_the_entry(tmp_path):
    """A relic reward's upgrade is logged at the elite it was won from.

    J8JL0SC7A2JX node 30: Whetstone upgraded Neutralize after the elite; the
    save entering that elite holds it at level 0.
    """
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("elite", upgraded_cards=["CARD.NEUTRALIZE"]),
        _node("monster"),
    ], [_card("CARD.NEUTRALIZE", level=1)])

    assert _copies(run.fights[0], "CARD.NEUTRALIZE") == [(0, False)]
    assert _copies(run.fights[1], "CARD.NEUTRALIZE") == [(1, False)]


def test_event_rooms_own_upgrade_is_unordered_against_its_combat(tmp_path):
    """K9VGN7Y1ZTE8 node 35 (Battleworn Dummy): `.run` does not say whether
    the event upgraded All for One before or after its combat."""
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("unknown", rooms=["event", "monster"],
              upgraded_cards=["CARD.ALL_FOR_ONE"]),
        _node("monster"),
    ], [_card("CARD.ALL_FOR_ONE", level=1)])

    assert _copies(run.fights[0], "CARD.ALL_FOR_ONE") == [(1, True)]
    assert any("CARD.ALL_FOR_ONE cannot be assigned" in caveat
               for caveat in run.fights[0].caveats)
    assert _copies(run.fights[1], "CARD.ALL_FOR_ONE") == [(1, False)]
    assert relay_parser.own_node_precedes_entry(
        {"rooms": [{"room_type": "event"}, {"room_type": "monster"}]}) is None
    assert relay_parser.own_node_precedes_entry(
        {"rooms": [{"room_type": "monster"}]}) is False
    assert relay_parser.own_node_precedes_entry({}) is False


def test_later_copys_upgrade_is_not_undone_on_the_earlier_copy(tmp_path):
    """NA4SLXKS9923 node 7: Inferno smithed at node 6, a second Inferno
    bought at node 19 and upgraded at node 38. The save holds the first at
    level 1; the old single-copy rule undid the second copy's upgrade on it.
    """
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster", **_picked("CARD.INFERNO", 2)),
        _node("rest_site", upgraded_cards=["CARD.INFERNO"]),
        _node("monster"),
        _node("shop", cards_gained=[{"id": "CARD.INFERNO"}]),
        _node("rest_site", upgraded_cards=["CARD.INFERNO"]),
        _node("monster"),
    ], [_card("CARD.INFERNO", floor=2, level=1),
        _card("CARD.INFERNO", floor=5, level=1)])

    assert _copies(run.fights[0], "CARD.INFERNO") == []
    assert _copies(run.fights[1], "CARD.INFERNO") == [(1, False)]
    assert _copies(run.fights[2], "CARD.INFERNO") == [
        (1, False), (1, False)]


def test_acquisition_level_assigns_a_later_upgrade_to_the_other_copy(
        tmp_path):
    """QGKT78ER4DY8 node 4: Sword Boomerang picked already upgraded, a plain
    copy bought later and smithed. The picked copy was level 1 throughout."""
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster", **_picked("CARD.SWORD_BOOMERANG", 2, level=1)),
        _node("monster"),
        _node("shop", cards_gained=[
            {"id": "CARD.SWORD_BOOMERANG", "current_upgrade_level": 0}]),
        _node("monster"),
        _node("rest_site", upgraded_cards=["CARD.SWORD_BOOMERANG"]),
    ], [_card("CARD.SWORD_BOOMERANG", floor=2, level=1),
        _card("CARD.SWORD_BOOMERANG", floor=4, level=1)])

    assert _copies(run.fights[1], "CARD.SWORD_BOOMERANG") == [(1, False)]
    assert _copies(run.fights[2], "CARD.SWORD_BOOMERANG") == [
        (1, False), (0, False)]


def test_downgrade_is_a_dated_level_event(tmp_path):
    """QMTZUN2HGAQP node 34: Scrape picked upgraded and downgraded by
    Reflections at node 38; the final deck's level 0 is not its level before.
    8M2X57B83DNY node 23: Bloodletting downgraded and re-upgraded at one node.
    """
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster", card_choices=(
            _picked("CARD.SCRAPE", 2, level=1)["card_choices"]
            + _picked("CARD.BLOODLETTING", 2, level=1)["card_choices"])),
        _node("monster"),
        _node("unknown", rooms=["event"],
              upgraded_cards=["CARD.BLOODLETTING"],
              downgraded_cards=["CARD.SCRAPE", "CARD.BLOODLETTING"]),
        _node("monster"),
    ], [_card("CARD.SCRAPE", floor=2, level=0),
        _card("CARD.BLOODLETTING", floor=2, level=1)])

    assert _copies(run.fights[1], "CARD.SCRAPE") == [(1, False)]
    assert _copies(run.fights[1], "CARD.BLOODLETTING") == [(1, False)]
    assert _copies(run.fights[2], "CARD.SCRAPE") == [(0, False)]
    assert _copies(run.fights[2], "CARD.BLOODLETTING") == [(1, False)]


def test_removed_copys_endpoint_bounds_which_events_reach_it(tmp_path):
    """8M2X57B83DNY node 16: Uppercut smithed at node 15, stolen and returned
    at node 19 (a removal and a regain), a second copy smithed at node 31.
    The stolen endpoint left the deck before the second Smith."""
    stolen = _card("CARD.UPPERCUT", floor=2, level=1)
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster", **_picked("CARD.UPPERCUT", 2)),
        _node("rest_site", upgraded_cards=["CARD.UPPERCUT"]),
        _node("monster"),
        _node("monster", cards_removed=[stolen],
              cards_gained=[{"id": "CARD.UPPERCUT",
                             "current_upgrade_level": 1}]),
        _node("monster", **_picked("CARD.UPPERCUT", 6)),
        _node("rest_site", upgraded_cards=["CARD.UPPERCUT"]),
        _node("monster"),
    ], [_card("CARD.UPPERCUT", floor=5, level=1),
        _card("CARD.UPPERCUT", floor=6, level=1)])

    assert _copies(run.fights[1], "CARD.UPPERCUT") == [(1, False)]
    assert _copies(run.fights[2], "CARD.UPPERCUT") == [(1, False)]
    assert _copies(run.fights[3], "CARD.UPPERCUT") == [(1, False)]
    assert _copies(run.fights[4], "CARD.UPPERCUT") == [
        (1, False), (1, False)]


def test_duplicate_starters_before_any_upgrade_are_exact(tmp_path):
    """Every copy is provably level 0 before the id's first upgrade, and
    after the last one the endpoint stands; only between two upgrades of
    interchangeable copies is the upgraded slot unknown."""
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster"),
        _node("rest_site", upgraded_cards=["CARD.STRIKE_IRONCLAD"]),
        _node("monster"),
        _node("rest_site", upgraded_cards=["CARD.STRIKE_IRONCLAD"]),
        _node("monster"),
    ], [_card("CARD.STRIKE_IRONCLAD", level=1),
        _card("CARD.STRIKE_IRONCLAD", level=1),
        _card("CARD.STRIKE_IRONCLAD", level=0)])

    assert _copies(run.fights[0], "CARD.STRIKE_IRONCLAD") == [
        (0, False)] * 3
    assert _copies(run.fights[1], "CARD.STRIKE_IRONCLAD") == [
        (1, True), (1, True), (0, True)]
    assert any(
        "1 later upgrade(s) of CARD.STRIKE_IRONCLAD cannot be assigned"
        in caveat for caveat in run.fights[1].caveats)
    assert _copies(run.fights[2], "CARD.STRIKE_IRONCLAD") == [
        (1, False), (1, False), (0, False)]


def test_unexplained_ledger_flags_instead_of_guessing(tmp_path):
    """An endpoint no assignment reproduces (a level-0 copy with an upgrade
    logged and nothing to undo it) is flagged, not approximated."""
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("monster", **_picked("CARD.BASH", 2)),
        _node("monster"),
        _node("rest_site", upgraded_cards=["CARD.BASH"]),
    ], [_card("CARD.BASH", floor=2, level=0)])

    assert _copies(run.fights[1], "CARD.BASH") == [(0, True)]


def _row(alive_from=0, alive_to=None, start=0, end=0, present=True):
    return {"alive_from": alive_from, "alive_to": alive_to, "start": start,
            "end": end, "present": present}


def test_ledger_enumeration_contract():
    ledger = relay_parser._ledger_entry_levels
    # One copy, one later upgrade: level 0 at an earlier fight.
    assert ledger([_row(end=1)], [(3, 1)], 2, True) == (0,)
    # The fight's own node, ordered: the upgrade follows the entry.
    assert ledger([_row(end=1)], [(2, 1)], 2, True) == (0,)
    # The fight's own node, unordered: both entries are possible.
    assert ledger([_row(end=1)], [(2, 1)], 2, False) is None
    # An unrecorded acquisition level is enumerated, not assumed: the
    # endpoint and the one upgrade still force level 0 before it.
    assert ledger([_row(start=None, end=1)], [(3, 1)], 2, True) == (0,)
    # ... and with no event to pin it, a downgrade proves it started at 1.
    assert ledger([_row(start=None, end=0)], [(3, -1)], 2, True) == (1,)
    # A copy acquired after the event's node cannot have received it.
    assert ledger(
        [_row(end=1), _row(alive_from=5, end=0, present=False)],
        [(3, 1)], 2, True) == (0,)
    # A copy removed before the event's node cannot have received it.
    assert ledger(
        [_row(end=1), _row(alive_to=2, end=1, start=1, present=False)],
        [(3, 1)], 1, True) == (0,)
    # Absent copies that fork only among themselves leave the entry exact.
    assert ledger(
        [_row(end=1, start=1),
         _row(alive_from=4, end=1, present=False),
         _row(alive_from=4, end=0, present=False)],
        [(5, 1)], 2, True) == (1,)
    # No level passes CardModel.MaxUpgradeLevel.
    assert ledger([_row(end=2)], [(3, 1), (4, 1)], 2, True) is None
    assert ledger([_row(end=1)], [(3, 1), (4, 1)], 2, True) is None


def test_ledger_refuses_to_enumerate_too_many_unknown_starts():
    rows = [_row(start=None, end=0)
            for _ in range(relay_parser._LEDGER_UNKNOWN_START_LIMIT + 1)]
    assert relay_parser._ledger_entry_levels(
        rows, [(3, 1)], 2, True) is None
    rows.pop()
    rows[0]["end"] = 1
    assert relay_parser._ledger_entry_levels(
        rows, [(3, 1)], 2, True) == (0,) * len(rows)


def _byrdpip_run(tmp_path, swoop_level, *, removed=False):
    """An Egg Byrdpip turned into a Swoop at node 3, then a later Smith."""
    swoop = _card("CARD.BYRD_SWOOP", floor=2, level=swoop_level)
    history = [
        _node("ancient"),
        _node("monster", cards_gained=[{"id": "CARD.BYRDONIS_EGG"}]),
        _node("monster"),
        _node("treasure"),
        _node("monster"),
        _node("rest_site", upgraded_cards=["CARD.BYRD_SWOOP"]),
        _node("monster", cards_removed=[swoop] if removed else []),
        _node("monster"),
    ]
    return _parse(
        tmp_path, history, [] if removed else [swoop],
        relics=[{"id": "RELIC.BYRDPIP", "floor_added_to_deck": 4}])


@pytest.mark.parametrize("removed", [False, True])
def test_byrdpip_swoop_is_a_ledger_copy_only_from_byrdpips_node(
        tmp_path, removed):
    """Byrdpip's automatic Swoop is created at level 1 with no upgrade
    event, so a logged Swoop upgrade cannot be its level: the ledger has no
    assignment and flags the copy. Before Byrdpip the row is the Egg."""
    run = _byrdpip_run(tmp_path, 1, removed=removed)

    assert _copies(run.fights[0], "CARD.BYRD_SWOOP") == []
    assert _copies(run.fights[1], "CARD.BYRDONIS_EGG") == [(0, False)]
    assert _copies(run.fights[2], "CARD.BYRD_SWOOP") == [(1, True)]
    assert _copies(run.fights[3], "CARD.BYRD_SWOOP") == [(1, False)]


@pytest.mark.parametrize("removed", [False, True])
def test_malformed_byrdpip_swoop_has_no_known_start(tmp_path, removed):
    """A non-canonical automatic Swoop endpoint is already flagged; the
    ledger enumerates its start instead of assuming level 1."""
    run = _byrdpip_run(tmp_path, 0, removed=removed)

    assert all(flagged for _level, flagged
               in _copies(run.fights[2], "CARD.BYRD_SWOOP"))


def test_transform_final_card_carries_its_acquisition_level(tmp_path):
    """K9VGN7Y1ZTE8: Pandora's Box made two level-0 Glassworks (the
    transform record serializes each final card), and one was upgraded
    later. Both were level 0 at every fight before that upgrade."""
    glasswork = _card("CARD.GLASSWORK", floor=2)
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("ancient", cards_transformed=[
            {"original_card": _card("CARD.STRIKE_IRONCLAD", level=1),
             "final_card": glasswork},
            {"original_card": _card("CARD.DEFEND_IRONCLAD"),
             "final_card": glasswork}]),
        _node("monster"),
        _node("elite", upgraded_cards=["CARD.GLASSWORK"]),
        _node("monster"),
    ], [_card("CARD.GLASSWORK", floor=2, level=0),
        _card("CARD.GLASSWORK", floor=2, level=1)])

    assert _copies(run.fights[0], "CARD.GLASSWORK") == [
        (0, False), (0, False)]
    assert _copies(run.fights[1], "CARD.GLASSWORK") == [
        (0, False), (0, False)]
    assert _copies(run.fights[2], "CARD.GLASSWORK") == [
        (0, False), (1, False)]


def test_undated_upgrade_leaves_acquisition_levels_unknown(tmp_path):
    """A second Bash ends upgraded with no event to explain it, so no copy's
    acquisition level is trusted. The dated own-node upgrade can still only
    have reached the copy that existed, which the ledger undoes; every row
    keeps the unknown-floor flag (#1119)."""
    run = _parse(tmp_path, [
        _node("ancient"),
        _node("elite", upgraded_cards=["CARD.BASH"]),
        _node("monster"),
        _node("shop", cards_gained=[{"id": "CARD.BASH"}]),
        _node("monster"),
    ], [_card("CARD.BASH", level=1), _card("CARD.BASH", floor=4, level=1)])

    assert _copies(run.fights[0], "CARD.BASH") == [(0, True)]
    assert _copies(run.fights[1], "CARD.BASH") == [(1, True)]
    assert _copies(run.fights[2], "CARD.BASH") == [(1, True), (1, True)]
