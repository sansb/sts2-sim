"""#138 Batch 149 / #333: outgoing multiplier and debuff seam.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path


HERE = Path(__file__).parent


def test_batch149_target_power_census_partition_rows():
    census = json.loads((HERE / "target_power_census.json").read_text())
    rows = {row["power"]: row for row in census["counted_projection"]}
    assert len(rows) == 18
    assert rows["StranglePower"]["representation"] == "Monster.strangle"
    assert rows["DebilitatePower"] == {
        "power": "DebilitatePower",
        "representation": "Monster.debilitate",
        "presence": "amount > 0; decremented and removed at zero",
        "current_type": 2,
        "cardinality":
            "one Amount-stacking singleton; Amount is duration only",
        "temporary": False,
        "type_rva": "0x24d641",
        "stack_type": 1,
        "stack_type_rva": "0x24d644",
    }
    assert rows["KnockdownPower"] == {
        "power": "KnockdownPower",
        "representation":
            "Monster.knockdown tuple of (amount, applier identity)",
        "presence": (
            "one tuple row per live application; every row removes at "
            "owner side end"),
        "current_type": 2,
        "cardinality": (
            "InstanceType 1 creates a distinct listener for every "
            "application"),
        "temporary": False,
        "type_rva": "0xa44cd",
        "stack_type": 1,
        "stack_type_rva": "0xa44d0",
        "instance_type": 1,
        "instance_type_rva": "0xa44d3",
    }
    assert rows["SicEmPower"] == {
        "power": "SicEmPower",
        "representation": "Monster.sic_em",
        "presence": (
            "amount > 0; whole instance removed at owner side end, death, "
            "or creature replacement"),
        "current_type": 2,
        "cardinality": (
            "one Amount-stacking singleton; Amount is the summon/growth "
            "value for every qualifying Osty DamageResult"),
        "temporary": False,
        "type_rva": "0x2538fc",
        "stack_type": 1,
        "stack_type_rva": "0x2538ff",
    }
