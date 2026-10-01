"""#138 Batch 123 / W225: Bing Bong census false-positive closure.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import hashlib
import json
from pathlib import Path

from attestations import attested
from tools.census_relics import (
    CLASS_INERT_METHODS, COMBAT_METHODS, INERT_METHODS,
)


HERE = Path(__file__).parent
BING_BONG = "RELIC.BING_BONG"


def _sha(name):
    return hashlib.sha256((HERE / name).read_bytes()).hexdigest()


def test_w225_class_scoped_census_boundary_and_frozen_provenance():
    assert "AfterCardChangedPiles" in COMBAT_METHODS
    assert "AfterCardChangedPiles" not in INERT_METHODS
    assert CLASS_INERT_METHODS == {
        "BingBong": {"AfterCardChangedPiles"},
        "DarkstonePeriapt": {"AfterCardChangedPiles"},
        "LuckyFysh": {"AfterCardChangedPiles"},
        "LoomingFruit": {"HasCornucopia"},
        "PaelsWing": {"OnSacrifice"},
    }

    census = json.loads((HERE / "relics_census.json").read_text())
    assert census[BING_BONG] == {
        "class": "BingBong",
        "combat_active": False,
        "hooks": [],
        "unknown_hooks": [],
    }
    raw = json.loads((HERE / "relic_templates.json").read_text())
    assert raw["refused"][BING_BONG] == \
        "unsupported hook AfterCardChangedPiles"

    assert _sha("relic_templates.json") == \
        attested("relic_templates.json")
    assert _sha("relics_census.json") == \
        attested("relics_census.json")
    assert _sha("tools/census_relics.py") == \
        attested("tools/census_relics.py")
