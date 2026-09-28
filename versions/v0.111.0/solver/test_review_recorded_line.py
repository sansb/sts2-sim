"""Recorded-capture association contracts for #1068 slice 2.

The Python recorded-line seed and its annotated line (`replay_recorded_line`,
`line_document`, `_attach_recorded_line`) were retired with the Python review
producer (#2827 item F1). The recorded line is now resolved and replayed in
Rust (`rust_replay.recorded_witness`, `rust_review.replay_line`; see
`test_rust_replay.py`). What stays here is the provenance half: an associated
capture is kept at every candidate tier.
"""

from __future__ import annotations

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import review_provenance as provenance  # noqa: E402


TD = pathlib.Path(__file__).parent / "testdata"
RUN = TD / "85920V7XQFSN.run"
RECORD = TD / "85920V7XQFSN_fight2_provenance.json"


def test_associated_replay_is_retained_at_every_candidate_tier():
    raw = json.loads(RUN.read_text())
    record = json.loads(RECORD.read_text())
    for candidates, tier in (
        ([], "provenance_unresolved"),
        ([{"observation": "legacy_unverified"}], "provenance_legacy"),
    ):
        record["entry_candidates"] = candidates
        analyzed = provenance.analyze_bundle(
            raw, {"schema_version": 2, "fights": [record]})

        assert analyzed.fights[2].tier == tier
        assert analyzed.fights[2].replay is not None
