"""Issue #215 / Batch 178: exact death preventers and Steam Eruption.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from relay_parser import FightState, RunSummary, _date_cross_combat_relic_state
from tools.live_coach import build_entry


TAIL = "RELIC.LIZARD_TAIL"
WATERFALL = "ENCOUNTER.WATERFALL_GIANT_BOSS"


def _fight(node, relics=(TAIL,)):
    return FightState(
        node_index=node, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1,
        gold_entering=0, deck_entering=[], relics_entering=list(relics),
        potions_entering=[], damage_taken=0, hp_healed=0,
        turns_taken=1, potions_used=[], hp_after=1)


def _node(kind="monster"):
    return {
        "map_point_type": kind,
        "rooms": [{"room_type": kind, "turns_taken": 1}],
        "player_stats": [{"rest_site_choices": []}],
    }


def test_live_save_adapter_reads_exact_lizard_saved_property():
    save = {
        "visited_map_coords": [[0, 0]],
        "players": [{
            "deck": [{"id": "CARD.BASH", "current_upgrade_level": 0}],
            "relics": [{
                "id": TAIL,
                "props": {"bools": [{"name": "WasUsed", "value": True}]},
            }],
            "potions": [],
            "current_hp": 50, "max_hp": 50, "gold": 0,
        }],
    }
    entry = build_entry(save, WATERFALL, "boss")
    assert entry["relic_counters"] == {TAIL: True}


def test_run_history_only_seeds_fresh_lizard_and_refuses_later_fights():
    summary = RunSummary(
        seed="S", build_id="v0.109.1", schema_version=1,
        character="CHARACTER.IRONCLAD", ascension=10, win=True,
        killed_by=None, fights=[_fight(0), _fight(1)])
    history = [_node(), _node()]
    player = {"relics": [{"id": TAIL, "floor_added_to_deck": 0}]}
    _date_cross_combat_relic_state(summary, history, player)
    assert summary.fights[0].relic_counters[TAIL] is False
    assert TAIL not in summary.fights[1].relic_counters
    assert any("WasUsed unseedable" in caveat
               for caveat in summary.fights[1].caveats)
