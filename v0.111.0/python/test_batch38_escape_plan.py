"""Batch38 exact content pins for Escape Plan (#138).

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))


HERE = pathlib.Path(__file__).parent


def test_escape_plan_census_targeting_and_refusal_provenance():
    census = json.loads((HERE / "cards_census.json").read_text())
    metadata = json.loads((HERE.parents[1] / "data" / "cards.json").read_text())
    raw = json.loads((HERE / "card_templates_raw.json").read_text())
    composed = json.loads((HERE / "card_templates.json").read_text())
    cid = "CARD.ESCAPE_PLAN"
    assert metadata[cid] == {"rarity": "uncommon", "type": "skill"}
    assert census[cid]["cost"] == 0 and census[cid]["type"] == "skill"
    assert census[cid]["draws"] and not census[cid]["adds_cards"]
    for templates in (raw, composed):
        assert templates["max_upgrade"][cid] == 1
        assert templates["refused"][cid] == \
            "FirstOrDefault on non-selection"
        assert cid not in templates["cards"]
        target = templates["targeting"][cid]
        assert target["constructor_rva"] == "0xe271b"
        assert target["effective_target_type"]["name"] == "Self"
        assert target["multiplayer_constraint"]["name"] == "None"
