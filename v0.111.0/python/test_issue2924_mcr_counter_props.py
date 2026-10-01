"""#2924: decoded-MCR flat props seed the persistent counter relics.

A replay-backed review builds its opening from the .mcr's own save, whose
SavedProperties decode as a flat map (``{"TurnsSeen": 2}``). The save adapter
read flat props only for the teas, Tea Sets, Pumpkin Candle and Lizard Tail,
so LYE3ZK9FYKKV floors 12-17 refused with ``PENDULUM counter unseeded``.

Property names and types are v0.111.0 sts2.dll [SavedProperty] members; see
the citation at ``_SCOPED_RELIC_SAVED_PROPERTIES``.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import pytest

from tools.live_coach import build_entry


COUNTERS = {
    "HAPPY_FLOWER": "TurnsSeen",
    "FAKE_HAPPY_FLOWER": "TurnsSeen",
    "PENDULUM": "TurnsSeen",
    "POLLINOUS_CORE": "TurnsSeen",
    "NUNCHAKU": "AttacksPlayed",
    "PEN_NIB": "AttacksPlayed",
    "GALACTIC_DUST": "StarsSpent",
    "IRON_CLUB": "CardsPlayed",
    "TUNING_FORK": "SkillsPlayed",
    "GIRYA": "TimesLifted",
}


def _save(relic):
    return {
        "visited_map_coords": [[0, 0]],
        "ascension": 10,
        "players": [{
            "deck": [{"id": "CARD.BASH", "current_upgrade_level": 0}],
            "relics": [relic], "potions": [],
            "current_hp": 50, "max_hp": 50, "gold": 0,
        }],
    }


def _entry(relic):
    return build_entry(_save(relic), "ENCOUNTER.TERROR_EEL_ELITE", "elite")


@pytest.mark.parametrize("short,name", sorted(COUNTERS.items()))
@pytest.mark.parametrize("shape", ("flat", "flat_wax", "grouped",
                                   "grouped_wax"))
def test_flat_and_grouped_props_seed_the_same_counter(short, name, shape):
    props = {
        "flat": {name: 2},
        "flat_wax": {name: 2, "IsWax": False, "IsMelted": False},
        "grouped": {"ints": [{"name": name, "value": 2}]},
        "grouped_wax": {
            "ints": [{"name": name, "value": 2}],
            "bools": [{"name": "IsWax", "value": False}],
        },
    }[shape]
    relic_id = short if shape.startswith("flat") else f"RELIC.{short}"
    entry = _entry({"id": relic_id, "props": props})
    assert entry["relic_counters"] == {f"RELIC.{short}": 2}
