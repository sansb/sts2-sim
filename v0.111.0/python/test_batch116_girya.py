"""#138 Batch 116 / W218: exact Girya lift reconstruction.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from relay_parser import (
    FightState, RunSummary, _date_cross_combat_relic_state,
)


GIRYA = "RELIC.GIRYA"


def _fight(node):
    return FightState(
        node_index=node, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1,
        gold_entering=0, deck_entering=[], relics_entering=[GIRYA],
        potions_entering=[], damage_taken=0, hp_healed=0,
        turns_taken=1, potions_used=[], hp_after=1)


def _node(kind, *choices):
    room_type = "monster" if kind == "unknown" else kind
    return {
        "map_point_type": kind,
        "rooms": [{"room_type": room_type, "turns_taken": 0}],
        "player_stats": [{"rest_site_choices": list(choices)}],
    }


def test_relay_history_counts_only_prior_owned_lift_choices():
    # Girya is obtained on global node 1 (floor 2), so that node's synthetic
    # LIFT is excluded by the strict acquisition-floor convention. Later
    # SMITH/HEAL choices are disjoint, and future lifts do not leak backward.
    history = [
        _node("ancient"),
        _node("rest_site", "LIFT"),
        _node("rest_site", "LIFT"),
        _node("rest_site", "SMITH"),
        _node("monster"),
        _node("rest_site", "LIFT"),
        _node("monster"),
        _node("rest_site", "HEAL", "LIFT"),
        _node("monster"),
    ]
    summary = RunSummary(
        "S", "b", 1, "c", 0, True, None,
        fights=[_fight(4), _fight(6), _fight(8)])
    player = {
        "relics": [{"id": GIRYA, "floor_added_to_deck": 2}]}
    _date_cross_combat_relic_state(summary, history, player)
    assert [f.relic_counters[GIRYA] for f in summary.fights] == [1, 2, 3]
    assert all(not f.caveats for f in summary.fights)
