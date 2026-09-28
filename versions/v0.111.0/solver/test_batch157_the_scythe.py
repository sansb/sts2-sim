"""#138 Batch 157 / #673: exact The Scythe local/deck damage growth.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json

import relay_parser
from tools.live_coach import build_entry


def _props(growth):
    return {
        "ints": [
            {"name": "CurrentDamage", "value": 13 + growth},
            {"name": "IncreasedDamage", "value": growth},
        ],
    }


def test_live_coach_preserves_native_props_and_run_dating_is_conservative():
    props = _props(8)
    save = {
        "players": [{
            "deck": [{
                "id": "CARD.THE_SCYTHE",
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

    positive = {
        "id": "CARD.THE_SCYTHE", "floor_added_to_deck": 1,
        "props": props,
    }
    first, ambiguous = relay_parser._the_scythe_historical_entry_props(
        positive, 2, [2, 5, 8])
    assert first == _props(0) and not ambiguous
    later, ambiguous = relay_parser._the_scythe_historical_entry_props(
        positive, 5, [2, 5, 8])
    assert later == props and ambiguous

    zero = dict(positive, props=_props(0))
    exact, ambiguous = relay_parser._the_scythe_historical_entry_props(
        zero, 8, [2, 5, 8])
    assert exact == _props(0) and not ambiguous
    missing = dict(positive)
    missing.pop("props")
    exact, ambiguous = relay_parser._the_scythe_historical_entry_props(
        missing, 2, [2, 5, 8])
    assert exact is None and ambiguous

    # Removal snapshots use the same helper, and malformed/duplicate endpoint
    # fields never acquire the monotone-zero proof.
    removed = dict(positive, props={
        "ints": [
            {"name": "CurrentDamage", "value": 13},
            {"name": "IncreasedDamage", "value": 0},
            {"name": "IncreasedDamage", "value": 0},
        ],
    })
    exact, ambiguous = relay_parser._the_scythe_historical_entry_props(
        removed, 2, [2, 5, 8])
    assert exact == removed["props"] and ambiguous


def test_historical_duplicate_scythe_rows_are_dated_independently(tmp_path):
    def stats():
        return {
            "current_hp": 50, "damage_taken": 0, "hp_healed": 0,
            "max_hp": 50, "max_hp_gained": 0, "max_hp_lost": 0,
            "current_gold": 0, "gold_gained": 0, "gold_lost": 0,
            "gold_spent": 0, "gold_stolen": 0,
        }

    room = {
        "room_type": "monster", "model_id": "ENCOUNTER.X",
        "monster_ids": [], "turns_taken": 1,
    }
    history = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [stats()]},
        {"map_point_type": "monster", "rooms": [room],
         "player_stats": [stats()]},
        {"map_point_type": "monster", "rooms": [room],
         "player_stats": [stats()]},
    ]
    endpoint_rows = [
        {"id": "CARD.THE_SCYTHE", "floor_added_to_deck": 1,
         "props": _props(4)},
        {"id": "CARD.THE_SCYTHE", "floor_added_to_deck": 2,
         "props": _props(8)},
    ]
    run = {
        "seed": "BATCH157DUPLICATES", "build_id": "v0.109.1",
        "schema_version": 1, "ascension": 0, "win": True,
        "killed_by_encounter": "", "map_point_history": [history],
        "players": [{
            "character": "NECROBINDER", "deck": endpoint_rows,
            "relics": [], "max_potion_slot_count": 2,
        }],
    }
    path = tmp_path / "duplicate-scythes.run"
    path.write_text(json.dumps(run))

    first, second = relay_parser.parse_run(str(path)).fights
    assert len(first.deck_entering) == 1
    assert first.deck_entering[0]["props"] == _props(0)
    assert not first.deck_entering[0].get("props_ambiguous", False)

    assert len(second.deck_entering) == 2
    older, newer = second.deck_entering
    assert older["floor_added"] == 1 and older["props"] == _props(4)
    assert older["props_ambiguous"]
    assert newer["floor_added"] == 2 and newer["props"] == _props(0)
    assert not newer.get("props_ambiguous", False)
