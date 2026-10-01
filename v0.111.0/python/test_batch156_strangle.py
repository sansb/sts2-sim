"""#138 Batch 156 / #383: exact Strangle keyed retaliation.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import hashlib
import json
from pathlib import Path


HERE = Path(__file__).parent


def test_target_power_census_attestation_includes_strangle():
    raw = (HERE / "target_power_census.json").read_bytes()
    census = json.loads(raw)
    row = next(
        row for row in census["counted_projection"]
        if row["power"] == "StranglePower")
    assert row == {
        "power": "StranglePower",
        "representation": "Monster.strangle",
        "presence": (
            "amount > 0; removed at enemy owner side end, death, or "
            "creature replacement"),
        "current_type": 2,
        "cardinality": (
            "InstanceType 2 is per applier; the admitted solo model has "
            "exactly one player applier and therefore one instance"),
        "temporary": False,
        "type_rva": "0x25470b",
        "stack_type": 1,
        "stack_type_rva": "0x25470e",
        "instance_type": 2,
        "instance_type_rva": "0x254711",
    }
    assert hashlib.sha256(raw).hexdigest()
