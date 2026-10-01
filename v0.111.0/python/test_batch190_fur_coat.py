"""#138 Batch 190 / #758: Fur Coat exact marked-fight HP command.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json

from relay_parser import parse_run


FUR = "RELIC.FUR_COAT"


def _props(*, cols=None, rows=None, complete=True, flat=False):
    cols = [1, 2, 3, 4, 5, 6, 7] if cols is None else cols
    rows = [9, 8, 7, 6, 5, 4, 3] if rows is None else rows
    values = {
        "FurCoatActIndex": 0,
        "FurCoatCoordCols": cols,
        "FurCoatCoordRows": rows,
        "FurCoatCoordsSet": complete,
    }
    if flat:
        return values
    return {
        "ints": [{"name": "FurCoatActIndex", "value": 0}],
        "int_arrays": [
            {"name": "FurCoatCoordCols", "value": cols},
            {"name": "FurCoatCoordRows", "value": rows},
        ],
        "bools": [
            {"name": "FurCoatCoordsSet", "value": complete},
            {"name": "IsWax", "value": False},
        ],
    }


def test_completed_run_uses_each_fight_history_bit_not_endpoint_props(
        tmp_path):
    def stats(affected):
        return {
            "current_hp": 50, "damage_taken": 0, "hp_healed": 0,
            "max_hp": 50, "max_hp_gained": 0, "max_hp_lost": 0,
            "current_gold": 0, "gold_gained": 0, "gold_lost": 0,
            "gold_spent": 0, "gold_stolen": 0,
            "is_affected_by_fur_coat": affected,
        }

    room = {
        "room_type": "monster", "model_id": "ENCOUNTER.X",
        "monster_ids": [], "turns_taken": 1,
    }
    history = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [stats(False)]},
        {"map_point_type": "monster", "rooms": [room],
         "player_stats": [stats(True)]},
        {"map_point_type": "monster", "rooms": [room],
         "player_stats": [stats(False)]},
    ]
    run = {
        "seed": "BATCH190", "build_id": "v0.109.1",
        "schema_version": 19, "ascension": 0, "win": True,
        "killed_by_encounter": "", "map_point_history": [history],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": [{"id": "CARD.BASH", "current_upgrade_level": 0,
                      "floor_added_to_deck": 1}],
            # Deliberately contradictory endpoint props: historical adapter
            # must never backfill them across fights.
            "relics": [{"id": FUR, "floor_added_to_deck": 1,
                        "props": _props(flat=True)}],
        }],
    }
    path = tmp_path / "fur-coat.run"
    path.write_text(json.dumps(run))
    first, second = parse_run(str(path)).fights
    assert first.fur_coat_active is True
    assert second.fur_coat_active is False
