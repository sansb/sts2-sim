"""Organic-capture reconciliation for Corpse Slugs #146 and Knights #199.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path

from attestations import attested


HERE = Path(__file__).parent


def test_ravenous_is_censused_as_a_non_debuff_for_rend():
    census = json.loads((HERE / "target_power_census.json").read_text())
    row = next(row for row in census["excluded_non_debuff"]
               if row["power"] == "RavenousPower")
    assert row == {
        "power": "RavenousPower",
        "representation": "Monster.ravenous on CORPSE_SLUG",
        "presence": (
            "one innate Amount-5 instance while the owner is live; sibling "
            "deaths grant Strength without changing the instance"),
        "current_type": 1,
        "type_rva": "0xa639f",
        "stack_type": 1,
        "stack_type_rva": "0xa63a2",
    }
    assert attested("target_power_census.json")
