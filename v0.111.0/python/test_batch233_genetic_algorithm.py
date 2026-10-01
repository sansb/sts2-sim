"""#138 Batch 233 / #722: exact Genetic Algorithm persistent Block.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import relay_parser
from tools.live_coach import build_entry


def _props(growth):
    return {
        "ints": [
            {"name": "CurrentBlock", "value": 1 + growth},
            {"name": "IncreasedBlock", "value": growth},
        ],
    }


def test_live_save_and_historical_run_props_preserve_i5_boundary():
    props = _props(8)
    save = {
        "players": [{
            "deck": [{
                "id": "CARD.GENETIC_ALGORITHM",
                "current_upgrade_level": 1,
                "props": props,
            }],
            "relics": [], "potions": [], "current_hp": 70, "max_hp": 80,
            "gold": 12,
        }],
        "visited_map_coords": [[0, 0]],
    }
    entry = build_entry(save, "ENCOUNTER.TEST", "monster")
    assert entry["deck_entering"][0]["props"] == props

    endpoint = {
        "id": "CARD.GENETIC_ALGORITHM", "floor_added_to_deck": 1,
        "props": props,
    }
    first, ambiguous = \
        relay_parser._genetic_algorithm_historical_entry_props(
            endpoint, 2, [2, 5, 8])
    assert first == _props(0) and not ambiguous
    later, ambiguous = \
        relay_parser._genetic_algorithm_historical_entry_props(
            endpoint, 5, [2, 5, 8])
    assert later == props and ambiguous
    zero = dict(endpoint, props=_props(0))
    exact, ambiguous = \
        relay_parser._genetic_algorithm_historical_entry_props(
            zero, 8, [2, 5, 8])
    assert exact == _props(0) and not ambiguous
