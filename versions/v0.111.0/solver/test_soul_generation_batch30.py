"""Batch 30 pins for the exact Soul generation family.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))


HERE = pathlib.Path(__file__).parent


def _load(name):
    return json.loads((HERE / name).read_text())


def test_random_generation_hook_consumer_is_now_admitted():
    composed = _load("card_templates.json")
    relics = _load("relic_templates.json")
    assert "CARD.ARSENAL" in composed["cards"]
    assert "RELIC.REGALITE" not in relics["refused"]
    assert not {
        card_id: reason
        for card_id, reason in composed["refused"].items()
        if "AfterCardGeneratedForCombat" in reason
    }
    assert "CARD.TRASH_TO_TREASURE" in composed["cards"]
    assert {"CARD.PILLAR_OF_CREATION", "CARD.SMOKESTACK"} <= \
        composed["cards"].keys()
