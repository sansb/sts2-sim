"""Batch37 exact content pins for No Escape (#138).

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))


HERE = pathlib.Path(__file__).parent


def test_no_escape_census_targeting_and_refusal_provenance():
    census = json.loads((HERE / "cards_census.json").read_text())
    metadata = json.loads((HERE.parents[1] / "data" / "cards.json").read_text())
    raw = json.loads((HERE / "card_templates_raw.json").read_text())
    composed = json.loads((HERE / "card_templates.json").read_text())
    cid = "CARD.NO_ESCAPE"
    assert metadata[cid] == {"rarity": "uncommon", "type": "skill"}
    assert census[cid]["cost"] == 1 and census[cid]["type"] == "skill"
    refusal = "var ctor CalculatedVar args [('str', 'CalculatedDoom')]"
    for templates in (raw, composed):
        assert templates["max_upgrade"][cid] == 1
        assert templates["refused"][cid] == refusal
        assert cid not in templates["cards"]
        target = templates["targeting"][cid]
        assert target["constructor_rva"] == "0xe9f55"
        assert target["effective_target_type"]["name"] == "AnyEnemy"
        assert target["multiplayer_constraint"]["name"] == "None"
