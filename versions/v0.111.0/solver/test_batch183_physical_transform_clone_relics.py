"""Issue #755 / Batch 183: physical transform/clone relic continuations.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json

from relay_parser import parse_run


CLAWS = "RELIC.CLAWS"


def _synthetic_claws_run(path):
    def stats(**events):
        result = {
            "current_hp": 80, "damage_taken": 0, "hp_healed": 0,
            "max_hp": 80, "max_hp_gained": 0, "max_hp_lost": 0,
            "current_gold": 99, "gold_gained": 0, "gold_lost": 0,
            "gold_spent": 0, "gold_stolen": 0,
        }
        result.update(events)
        return result

    def node(kind, room, **events):
        return {
            "map_point_type": kind, "rooms": [room],
            "player_stats": [stats(**events)],
        }

    enchantment = {"id": "ENCHANTMENT.SHARP", "amount": 2}
    claw = {
        "id": "CARD.CLAW", "floor_added_to_deck": 1,
        "current_upgrade_level": 1, "enchantment": enchantment,
    }
    maul = {
        "id": "CARD.MAUL", "floor_added_to_deck": 3,
        "current_upgrade_level": 1, "enchantment": enchantment,
    }
    run = {
        "seed": "BATCH183-CLAWS", "build_id": "v0.109.1",
        "schema_version": 1, "ascension": 0, "win": False,
        "killed_by_encounter": "NONE.NONE",
        "map_point_history": [[
            node("ancient", {"room_type": "ancient"}),
            node("monster", {
                "room_type": "monster", "turns_taken": 1,
                "model_id": "ENCOUNTER.SLIMES_WEAK",
                "monster_ids": ["MONSTER.SLIME", "MONSTER.SLIME"],
            }),
            node("unknown", {"room_type": "event"},
                 cards_transformed=[{
                     "original_card": claw, "final_card": maul,
                 }]),
            node("monster", {
                "room_type": "monster", "turns_taken": 1,
                "model_id": "ENCOUNTER.NIBBITS_WEAK",
                "monster_ids": ["MONSTER.NIBBIT"],
            }),
        ]],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": [maul],
            "relics": [{"id": CLAWS, "floor_added_to_deck": 3}],
        }],
    }
    path.write_text(json.dumps(run))
    return path


def test_claws_logged_transform_preserves_upgrade_enchantment_and_provenance(
        tmp_path):
    summary = parse_run(str(_synthetic_claws_run(
        tmp_path / "claws.run")))
    before, after = summary.fights

    assert before.deck_entering == [{
        "id": "CARD.CLAW", "upgrade_level": 1, "floor_added": 1,
        "enchantment": "ENCHANTMENT.SHARP", "enchant_amount": 2,
    }]
    assert after.deck_entering == [{
        "id": "CARD.MAUL", "upgrade_level": 1, "floor_added": 3,
        "enchantment": "ENCHANTMENT.SHARP", "enchant_amount": 2,
    }]
    assert CLAWS not in before.relics_entering
    assert CLAWS in after.relics_entering
    assert not any(
        "no recorded gain" in caveat or "unknown floor" in caveat
        for caveat in summary.global_caveats)
