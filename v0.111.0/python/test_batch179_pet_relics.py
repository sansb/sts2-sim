"""Issue #753 / Batch 179: exact deterministic pet relics.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json

from relay_parser import parse_run


BYRDPIP = "RELIC.BYRDPIP"


def _synthetic_byrdpip_run(path):
    def stats(hp=80):
        return {
            "current_hp": hp, "damage_taken": 0, "hp_healed": 0,
            "max_hp": 80, "max_hp_gained": 0, "max_hp_lost": 0,
            "current_gold": 99, "gold_gained": 0, "gold_lost": 0,
            "gold_spent": 0, "gold_stolen": 0,
        }

    def node(kind, rooms):
        return {
            "map_point_type": kind, "rooms": rooms,
            "player_stats": [stats()],
        }

    run = {
        "seed": "BATCH179", "build_id": "v0.109.1",
        "schema_version": 1, "ascension": 0, "win": False,
        "killed_by_encounter": "NONE.NONE",
        "map_point_history": [[
            node("ancient", [{"room_type": "ancient"}]),
            node("monster", [{
                "room_type": "monster", "turns_taken": 1,
                "model_id": "ENCOUNTER.SLIMES_WEAK",
                "monster_ids": ["MONSTER.SLIME", "MONSTER.SLIME"],
            }]),
            node("unknown", [{"room_type": "event"}]),
            node("monster", [{
                "room_type": "monster", "turns_taken": 1,
                "model_id": "ENCOUNTER.NIBBITS_WEAK",
                "monster_ids": ["MONSTER.NIBBIT"],
            }]),
        ]],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": [{
                "id": "CARD.BYRD_SWOOP", "floor_added_to_deck": 1,
                "current_upgrade_level": 1,
            }],
            "relics": [{
                "id": BYRDPIP, "floor_added_to_deck": 3,
            }],
        }],
    }
    path.write_text(json.dumps(run))
    return path


def test_run_reconstruction_reverses_unlogged_byrdpip_transform(tmp_path):
    summary = parse_run(str(_synthetic_byrdpip_run(
        tmp_path / "byrdpip.run")))
    assert [fight.deck_entering for fight in summary.fights] == [
        [{
            "id": "CARD.BYRDONIS_EGG", "upgrade_level": 0,
            "floor_added": 1, "enchantment": None,
        }],
        [{
            "id": "CARD.BYRD_SWOOP", "upgrade_level": 1,
            "floor_added": 1, "enchantment": None,
        }],
    ]
    assert BYRDPIP not in summary.fights[0].relics_entering
    assert BYRDPIP in summary.fights[1].relics_entering
    assert not any("unknown floor" in caveat
                   for caveat in summary.global_caveats)
