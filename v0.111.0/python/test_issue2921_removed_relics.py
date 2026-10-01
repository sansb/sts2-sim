"""#2921: relics removed later in the run are owned by the fights before.

The final ``players[0].relics`` list omits a relic removed mid-run; the node
that removed it logs ``relics_removed``. LYE3ZK9FYKKV floor 17 refused with
``relic_membership`` because Touch of Orobas (floor 18) replaced Burning
Blood, and the parser dropped Burning Blood from every earlier fight.
"""

import json

import pytest

from relay_parser import _relic_spans, parse_run


BURNING = "RELIC.BURNING_BLOOD"
BLACK = "RELIC.BLACK_BLOOD"
OROBAS = "RELIC.TOUCH_OF_OROBAS"
PENDULUM = "RELIC.PENDULUM"
SWORD = "RELIC.SWORD_OF_STONE"
BLADE = "RELIC.BLADE_OF_INK"
ANCHOR = "RELIC.ANCHOR"


def _stats(*, picked=(), removed=()):
    stats = {
        "current_hp": 50, "damage_taken": 0, "hp_healed": 0,
        "max_hp": 50, "max_hp_gained": 0, "max_hp_lost": 0,
        "current_gold": 0, "gold_gained": 0, "gold_lost": 0,
        "gold_spent": 0, "gold_stolen": 0,
        "relic_choices": [
            {"choice": rid, "was_picked": True} for rid in picked],
    }
    if removed:
        stats["relics_removed"] = list(removed)
    return stats


def _combat(kind="monster", *, turns=1, **stats):
    return {
        "map_point_type": kind,
        "rooms": [{"room_type": kind, "model_id": "ENCOUNTER.X",
                   "monster_ids": [], "turns_taken": turns}],
        "player_stats": [_stats(**stats)],
    }


def _event(**stats):
    return {"map_point_type": "unknown",
            "rooms": [{"room_type": "event", "model_id": "EVENT.X"}],
            "player_stats": [_stats(**stats)]}


def _run(tmp_path, history, relics):
    run = {
        "seed": "ISSUE2921", "build_id": "v0.111.0",
        "schema_version": 19, "ascension": 0, "win": True,
        "killed_by_encounter": "", "map_point_history": [history],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": [{"id": "CARD.BASH", "current_upgrade_level": 0,
                      "floor_added_to_deck": 1}],
            "relics": [{"id": rid, "floor_added_to_deck": floor}
                       for rid, floor in relics],
        }],
    }
    path = tmp_path / "issue2921.run"
    path.write_text(json.dumps(run))
    return parse_run(str(path))


def test_touch_of_orobas_restores_the_starter_relic_before_its_floor(
        tmp_path):
    # Floor 1 start, floor 2 fight, floor 3 Orobas, floor 4 fight. The game
    # replaces Burning Blood in place, so Black Blood holds slot 0.
    history = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [_stats()]},
        _combat(),
        {"map_point_type": "ancient",
         "rooms": [{"room_type": "event", "model_id": "EVENT.OROBAS"}],
         "player_stats": [_stats(picked=(OROBAS, BLACK),
                                 removed=(BURNING,))]},
        _combat(),
    ]
    before, after = _run(
        tmp_path, history, [(BLACK, 3), (ANCHOR, 1), (OROBAS, 3)]).fights
    assert before.relics_entering == [BURNING, ANCHOR]
    assert after.relics_entering == [BLACK, ANCHOR, OROBAS]


def test_removal_at_a_combat_node_follows_that_nodes_fight(tmp_path):
    # Sword of Stone is picked at floor 2's reward and turns into Blade of
    # Ink after the floor-4 elite: owned for floors 3 and 4, not floor 5.
    history = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [_stats()]},
        _combat(picked=(SWORD,)),
        _combat(),
        _combat("elite", picked=(BLADE,), removed=(SWORD,)),
        _combat(),
    ]
    fights = _run(tmp_path, history, [(BLADE, 4)]).fights
    assert [f.relics_entering for f in fights] == [
        [], [SWORD], [SWORD], [BLADE]]


def test_removed_counter_relic_is_seeded_while_it_was_owned(tmp_path):
    # Pendulum picked at floor 2, ticks 2 turns at floor 3 and 2 more at
    # floor 4, and is traded away at floor 5. Before #2921 the floor-4 fight
    # neither owned it nor seeded TurnsSeen.
    history = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [_stats()]},
        _combat(picked=(PENDULUM,)),
        _combat(turns=2),
        _combat(turns=2),
        _event(picked=(ANCHOR,), removed=(PENDULUM,)),
        _combat(),
    ]
    fights = _run(tmp_path, history, [(ANCHOR, 5)]).fights
    assert fights[1].relics_entering == [PENDULUM]
    assert fights[1].relic_counters == {PENDULUM: 0}
    # the floor-3 combat is a prior owned combat: its last turn may not have
    # ticked (#3076), so floor 4 carries no exact TurnsSeen claim
    assert fights[2].relic_counters == {}
    assert any("PENDULUM turn counter unseedable" in c
               for c in fights[2].caveats)
    assert fights[3].relics_entering == [ANCHOR]
    assert PENDULUM not in fights[3].relic_counters


def test_spans_use_the_latest_pick_before_the_removal():
    history = [
        {"player_stats": [_stats()]},
        {"player_stats": [_stats(picked=(PENDULUM,))]},
        {"player_stats": [_stats(removed=(PENDULUM,))]},
        {"player_stats": [_stats(picked=(PENDULUM,))]},
    ]
    player = {"relics": [{"id": PENDULUM, "floor_added_to_deck": 4}]}
    assert _relic_spans(history, player) == [
        (PENDULUM, 2, 3), (PENDULUM, 4, None)]


def test_removal_without_a_distinct_acquisition_refuses():
    # A removed id that still sits in the final list at the only acquisition
    # floor available has no ownership span of its own (I5).
    history = [{"player_stats": [_stats(removed=(BURNING,))]}]
    player = {"relics": [{"id": BURNING, "floor_added_to_deck": 1}]}
    with pytest.raises(NotImplementedError, match="relic removal"):
        _relic_spans(history, player)
