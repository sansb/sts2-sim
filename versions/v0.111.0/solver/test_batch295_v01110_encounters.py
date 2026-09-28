"""#138 Batch 295 / #1216: exact v0.111.0 encounter remodel pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path


HERE = Path(__file__).parent


def test_historical_census_and_current_impact_delta_have_distinct_roles():
    census = json.loads((HERE / "encounters_census.json").read_text())[
        "monsters"]
    assert census["Entomancer"]["hp_min_consts"] == [8, 155, 145]
    assert census["Exoskeleton"]["hp_min_consts"] == [8, 25, 24]
    assert census["Exoskeleton"]["hp_max_consts"] == [8, 29, 28]
    impact = json.loads((HERE.parents[2] / "solver" / "version-impact/v0.111.0.json").read_text())
    delta = impact["like_for_like_artifact_delta"]["encounters"]
    assert len(delta) == 8
    assert delta[0].startswith("AXEBOT ONE_TWO")
    assert delta[-1].startswith("MECHA_KNIGHT Flamethrower")


def test_current_build_docs_drop_completed_children_from_pending_boundary():
    paths = (
        HERE / "RNG_FINDINGS.md", HERE / "STREAM_CONSUMERS.md",
        HERE / "harness/README.md",
        # dll-archive is cross-build and stays outside the bundle
        HERE.parents[2] / "solver" / "dll-archive/README.md")
    for path in paths:
        text = path.read_text()
        current = text[text.index("v0.111.0"):]
        normalized = " ".join(current.split())
        assert "v0.111.0 combat is" in normalized
        assert "admitted" in normalized
        assert "under #1217 and #1222" not in normalized
        assert "under #1217/#1222" not in normalized
        assert "#1215-#1218" not in current
    assert "#1215/#1216" in (HERE.parents[2] / "solver" / "dll-archive/README.md").read_text()
