"""Organic-capture reconciliation for Kaiser Crab issue #223.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path

from attestations import attested


HERE = Path(__file__).parent


def test_crab_powers_are_censused_as_type_one_rend_exclusions():
    census = json.loads((HERE / "target_power_census.json").read_text())
    rows = {row["power"]: row for row in census["excluded_non_debuff"]}
    assert rows["BackAttackLeftPower"] == {
        "power": "BackAttackLeftPower",
        "representation": "CRUSHER kind",
        "presence": "one immutable innate instance while the creature is live",
        "current_type": 1,
        "type_rva": "0x9fa83",
        "stack_type": 2,
        "stack_type_rva": "0x9fa86",
    }
    assert rows["BackAttackRightPower"] == {
        **rows["BackAttackLeftPower"],
        "power": "BackAttackRightPower",
        "representation": "ROCKET kind",
        "type_rva": "0x9fa91",
        "stack_type_rva": "0x9fa94",
    }
    assert rows["CrabRagePower"] == {
        "power": "CrabRagePower",
        "representation": "Monster.crab_rage on CRUSHER or ROCKET",
        "presence": (
            "one innate instance until owner death or a sibling actual death "
            "grants Strength 6 and Block 99, then removes it"),
        "current_type": 1,
        "type_rva": "0xa0dcb",
        "stack_type": 2,
        "stack_type_rva": "0xa0dce",
    }
    assert attested("target_power_census.json")
