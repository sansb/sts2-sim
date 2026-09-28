"""#138 Batch 87: Shackling Potion, Rainbow Ring, and Pael's Tears (W189).

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))


def test_batch87_raw_relic_refusals_remain_unchanged():
    refused = json.loads((HERE / "relic_templates.json").read_text())[
        "refused"]
    assert refused["RELIC.RAINBOW_RING"] == "ldarg 3 unmapped"
    assert refused["RELIC.PAELS_TEARS"] == "ldarg 3 unmapped"
