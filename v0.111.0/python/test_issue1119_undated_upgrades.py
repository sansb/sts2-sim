"""Regression coverage for unknown-floor historical card upgrades."""

from __future__ import annotations

import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from relay_parser import parse_run  # noqa: E402


def _stats(*, rest_choices=()):
    return {
        "current_hp": 80,
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
        "rest_site_choices": list(rest_choices),
    }


def _node(node_type, *, rest_choices=()):
    combat = node_type == "monster"
    room = {"room_type": node_type}
    if combat:
        room.update({
            "turns_taken": 2,
            "model_id": "ENCOUNTER.NIBBITS_WEAK",
            "monster_ids": ["MONSTER.NIBBIT"],
        })
    return {
        "map_point_type": node_type,
        "rooms": [room],
        "player_stats": [_stats(rest_choices=rest_choices)],
    }


def test_unknown_floor_upgrade_flags_only_rows_before_last_possible_date(
        tmp_path):
    """A final level is approximate before an unassigned Smith, exact after."""
    run = {
        "seed": "ISSUE1119",
        "build_id": "v0.110.1",
        "schema_version": 9,
        "ascension": 10,
        "win": True,
        "killed_by_encounter": "NONE.NONE",
        "map_point_history": [[
            _node("ancient"),
            _node("monster"),
            # Deliberately omit upgraded_cards: the endpoint proves an
            # upgrade occurred, while this Smith is only its latest possible
            # historical date.
            _node("rest_site", rest_choices=("SMITH",)),
            _node("monster"),
        ]],
        "players": [{
            "character": "IRONCLAD",
            "max_potion_slot_count": 2,
            "deck": [{
                "id": "CARD.STRIKE_IRONCLAD",
                "floor_added_to_deck": 1,
                "current_upgrade_level": 1,
            }],
            "relics": [],
        }],
    }
    path = tmp_path / "issue1119.run"
    path.write_text(json.dumps(run))

    first, second = parse_run(str(path)).fights
    first_strike = first.deck_entering[0]
    second_strike = second.deck_entering[0]

    assert first_strike["upgrade_level"] == 1
    assert first_strike["upgrade_ambiguous"] is True
    assert any("final endpoint upgrade levels may not yet apply" in caveat
               for caveat in first.caveats)
    assert second_strike["upgrade_level"] == 1
    assert "upgrade_ambiguous" not in second_strike
