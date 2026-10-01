"""#138 Batch 266 / #1075: exact Mad Science Tinker Time variants.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path

from relay_parser import (_persistent_card_historical_entry_props,
                          parse_run)


HERE = Path(__file__).parent


def test_batch266_historical_and_live_adapters_preserve_tinker_props(monkeypatch):
    props = {"ints": [
        {"name": "TinkerTimeType", "value": 3},
        {"name": "TinkerTimeRider", "value": 7},
    ]}
    card = {"id": "CARD.MAD_SCIENCE", "props": props}
    assert _persistent_card_historical_entry_props(card, 5, [5]) == \
        (props, False)

    # The full live-entry builder has unrelated run fields; pin its deck
    # adapter through a minimal structurally complete save fixture.
    save = json.loads((HERE / "testdata" / "TQM88QFMHSQR.run").read_text())
    player = save["players"][0]
    mad = next(c for c in player["deck"] if c["id"] == "CARD.MAD_SCIENCE")
    assert mad["props"] == props
    parsed = parse_run(str(HERE / "testdata" / "TQM88QFMHSQR.run"))
    historical = [
        c for fight in parsed.fights for c in fight.deck_entering
        if c["id"] == "CARD.MAD_SCIENCE"]
    assert len(historical) == 2
    assert all(card["props"] == props for card in historical)
