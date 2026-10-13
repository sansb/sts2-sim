"""Recorded-capture association contracts for #1068 slice 2.

The Python recorded-line seed, its annotated line and the review that
published them are gone (#2827 item F1, then the review path itself on
2026-10-11). The recorded line is resolved and replayed in Rust
(`engine/src/recorded.rs`). What stays here is the provenance half: an
associated capture is kept at every candidate tier.
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
