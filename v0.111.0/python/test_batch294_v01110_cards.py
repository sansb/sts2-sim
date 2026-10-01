"""#138 Batch 294 / #1215: exact v0.111.0 card remodel pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path


HERE = Path(__file__).parent
BUILD = "v0.111.0"


def test_historical_generated_snapshots_are_unchanged_and_current_values_exact():
    census = json.loads((HERE / "cards_census.json").read_text())
    templates = json.loads((HERE / "card_templates.json").read_text())
    values = json.loads((HERE.parents[1] / "data" / "game_values.json").read_text())
    assert census["CARD.EXPECT_A_FIGHT"]["cost"] == 2
    assert census["CARD.REND"]["cost"] == 2
    assert census["CARD.TIMES_UP"]["keywords"] == ["Exhaust"]
    assert templates["cards"]["CARD.SHROUD"]["levels"] == {
        "0": [["power", "ShroudPower", "self", 2]],
        "1": [["power", "ShroudPower", "self", 3]],
    }
    current = values["builds"][BUILD]["cards"]
    assert current["CARD.ALIGNMENT"]["stars"] == [2, 2]
    assert current["CARD.HYPERBEAM"]["vars"]["Damage"] == [24, 30]
    assert current["CARD.SHROUD"]["vars"]["Block"] == [3, 4]
