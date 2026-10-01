"""Regression coverage for full-belt potion replacement reconstruction.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys
from collections import Counter


HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from relay_parser import parse_run  # noqa: E402


def _stats(*, hp=80, used=(), choices=()):
    return {
        "current_hp": hp,
        "damage_taken": 0,
        "hp_healed": 0,
        "max_hp": 80,
        "max_hp_gained": 0,
        "max_hp_lost": 0,
        "current_gold": 99,
        "gold_gained": 0,
        "gold_lost": 0,
        "gold_spent": 0,
        "gold_stolen": 0,
        "potion_used": list(used),
        "potion_choices": [
            {"was_picked": True, "choice": choice}
            for choice in choices
        ],
    }


def _node(index, *, node_type="unknown", used=(), choices=()):
    combat = node_type in {"monster", "elite", "boss"}
    room = {"room_type": node_type if combat else "unknown"}
    if combat:
        room.update({
            "turns_taken": 2,
            "model_id": "ENCOUNTER.NIBBITS_WEAK",
            "monster_ids": ["MONSTER.NIBBIT"],
        })
    return {
        "map_point_type": node_type,
        "rooms": [room],
        "player_stats": [_stats(used=used, choices=choices)],
    }


def _write_run(tmp_path, nodes):
    run = {
        "seed": "ISSUE1118",
        "build_id": "v0.110.1",
        "schema_version": 9,
        "ascension": 10,
        "win": True,
        "killed_by_encounter": "NONE.NONE",
        "map_point_history": [nodes],
        "players": [{
            "character": "IRONCLAD",
            "max_potion_slot_count": 2,
            "deck": [{
                "id": "CARD.STRIKE_IRONCLAD",
                "floor_added_to_deck": 1,
                "current_upgrade_level": 0,
            }],
            "relics": [],
        }],
    }
    path = tmp_path / "issue1118.run"
    path.write_text(json.dumps(run))
    return path


def test_later_uses_resolve_two_full_belt_replacements(tmp_path):
    """The #1117 shape resolves to Radiant Tincture + Heart of Iron.

    Each picked potion at capacity branches across both replacement slots.
    A later fight's two recorded uses then select the only compatible belt.
    """
    skill = "POTION.SKILL_POTION"
    vulnerable = "POTION.VULNERABLE_POTION"
    energy = "POTION.ENERGY_POTION"
    beetle = "POTION.BEETLE_JUICE"
    heart = "POTION.HEART_OF_IRON"
    radiant = "POTION.RADIANT_TINCTURE"
    nodes = [
        _node(0, choices=(skill, vulnerable)),
        _node(1, node_type="monster", choices=(energy,)),
        _node(2, node_type="boss", used=(energy, skill)),
        _node(3, choices=(beetle,)),
        _node(4, choices=(heart,)),
        _node(5, choices=(radiant,)),
        _node(6, node_type="boss", used=(radiant, heart)),
    ]

    summary = parse_run(str(_write_run(tmp_path, nodes)))
    first, replacement_resolved, final = summary.fights

    assert first.potions_entering == [skill, vulnerable]
    assert not first.potion_entry_ambiguous
    assert replacement_resolved.potions_entering == [skill, energy]
    assert not replacement_resolved.potion_entry_ambiguous
    assert final.potions_entering == [radiant, heart]
    assert not final.potion_entry_ambiguous
    for fight in summary.fights:
        assert not (Counter(fight.potions_used)
                    - Counter(fight.potions_entering))
