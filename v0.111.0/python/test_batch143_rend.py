"""#138 Batch 143 / #623: exact Rend live target-power cardinality.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path

from attestations import attested


HERE = Path(__file__).parent


def test_batch143_checked_census_source_and_exact_partition():
    raw_census = (HERE / "target_power_census.json").read_bytes()
    # Protect every RVA, presence/cardinality rule, representation, and
    # exclusion row. Any census edit must be reviewed with this exact pin.
    assert attested("target_power_census.json")
    census = json.loads(raw_census)
    assert census["_source"] == {
        "game_version": "v0.109.1",
        "game_commit": "c8c577f6",
        "sts2_dll_sha256":
            "2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f",
        "power_type_fallback_rva": "0x22fe14",
        "rend_predicate_rva": "0x2958a1",
        "rend_multiplier_rva": "0x4000c2",
        "note": (
            "One row per represented enemy-owned or player-applied-to-enemy "
            "PowerModel family. Type/stack values are PowerType/"
            "PowerStackType enum values. Instance cardinality is one unless "
            "noted."),
    }
    counted = {row["power"] for row in census["counted_projection"]}
    assert counted == {
        "StrengthPower", "WeakPower", "VulnerablePower", "HangPower",
        "PoisonPower", "DoomPower", "OblivionPower", "StranglePower",
        "DemisePower",
        "ShrinkPower",
        "ConquerorPower", "DebilitatePower", "SicEmPower", "KnockdownPower",
        "ShriekPower", "SlowPower", "ImbalancedPower", "PlowPower",
    }
    assert len(counted) == len(census["counted_projection"]) == 18
    temporary = census["excluded_temporary"]
    assert len(temporary) == 1 and temporary[0]["temporary"] is True
    assert set(temporary[0]["concrete_represented_types"]) == {
        "ManglePower", "DarkShacklesPower", "EnfeeblingTouchPower",
        "CrushUnderPower", "DyingStarPower", "PiercingWailPower",
    }
    excluded = {row["power"] for row in census["excluded_non_debuff"]}
    assert {
        "ArtifactPower", "HardToKillPower", "IllusionPower",
        "InfestedPower", "IntangiblePower", "MinionPower",
        "PaperCutsPower", "ReattachPower", "RitualPower", "ThornsPower",
    } <= excluded
