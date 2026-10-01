"""The attestation manifest is current and well-formed (#713).

This is THE change-detector for shared protected artifacts: editing any
of them turns this test red until the manifest is refreshed deliberately
(tools/refresh_attestations.py) and the one-file diff is reviewed.
Historical batch tests reference these values via attestations.attested()
instead of embedding their own copies.
"""

import re

import attestations


def test_manifest_is_current_and_well_formed():
    manifest = attestations.manifest()
    assert manifest, "empty attestation manifest"
    stale = {}
    for name, expected in sorted(manifest.items()):
        assert re.fullmatch(r"[0-9a-f]{64}", expected), (name, expected)
        live = attestations.live_sha256(name)
        if live != expected:
            stale[name] = (expected[:12], live[:12])
    assert not stale, (
        f"stale attestation manifest entries {stale!r} — refresh "
        "deliberately with tools/refresh_attestations.py and review the "
        "one-file diff (SOLVER_INVARIANTS.md I5)")


def test_unknown_name_fails_closed():
    import pytest
    with pytest.raises(NotImplementedError, match="manifest entry"):
        attestations.attested("nonexistent-artifact.json")


def test_current_global_coverage_totals_single_home():
    """The ONE explicit global-totals pin (#713).

    Historical batch tests no longer embed coverage totals; when a batch
    moves the frontier it updates exactly this assertion (plus the
    regenerated matrix, whose freshness test_coverage_matrix.py owns).
    """
    import json
    import pathlib
    matrix = json.loads(
        (pathlib.Path(__file__).parent / "coverage_matrix.json")
        .read_text())
    totals = {
        status: sum(category["status"].get(status, 0)
                    for category in matrix["summary"].values())
        for status in ("certified", "modeled", "refused")
    }
    assert totals == {"certified": 9, "modeled": 1060, "refused": 0}
    assert totals["certified"] + totals["modeled"] == 1069
    assert sum(category["eligible_ids"]
               for category in matrix["summary"].values()) == 1069
    assert matrix["summary"]["encounter"] == {
        "by_pool": {
            "boss": {"certified": 2, "modeled": 10},
            "elite": {"certified": 4, "modeled": 8},
            "event": {"modeled": 7},
            "normal": {"certified": 1, "modeled": 40},
            "unknown": {"modeled": 1},
            "weak": {"certified": 2, "modeled": 13},
        },
        "eligible_ids": 88,
        "excluded_ids": 11,
        "status": {"certified": 9, "modeled": 79},
        "total_ids": 99,
    }
    assert matrix["summary"]["card"]["status"] == {
        "modeled": 596}
