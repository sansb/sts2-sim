"""Batch 182 / #763: persistent Iron Club and Tuning Fork counters.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from relay_parser import (
    FightState, RunSummary, _date_cross_combat_relic_state,
)


IRON = "RELIC.IRON_CLUB"
FORK = "RELIC.TUNING_FORK"


def _fight(node):
    return FightState(
        node_index=node, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1,
        gold_entering=0, deck_entering=[],
        relics_entering=[IRON, FORK], potions_entering=[],
        damage_taken=0, hp_healed=0, turns_taken=1,
        potions_used=[], hp_after=1)


def _node(kind="monster"):
    return {
        "map_point_type": kind,
        "rooms": [{
            "room_type": kind,
            **({"turns_taken": 1} if kind == "monster" else {}),
        }],
        "player_stats": [{"rest_site_choices": []}],
    }


def test_relay_seeds_first_owned_fight_and_refuses_cross_fight_guess():
    summary = RunSummary(
        "S", "b", 1, "c", 0, True, None,
        fights=[_fight(1), _fight(2)])
    player = {"relics": [
        {"id": IRON, "floor_added_to_deck": 1},
        {"id": FORK, "floor_added_to_deck": 1},
    ]}

    _date_cross_combat_relic_state(
        summary, [_node("ancient"), _node(), _node()], player)

    assert summary.fights[0].relic_counters == {IRON: 0, FORK: 0}
    assert not ({IRON, FORK} & summary.fights[1].relic_counters.keys())
    caveats = " ".join(summary.fights[1].caveats)
    assert "card-play remainder unseedable" in caveats
    assert "Skill-play counter unseedable" in caveats
