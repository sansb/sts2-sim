"""#138 Batch 298 / #1217: v0.111.0 shared-combat admission pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_docs_record_admission_and_no_stale_global_refusal():
    docs = (
        (HERE / "ENCOUNTER_MECHANICS.md").read_text(),
        (HERE / "RNG_FINDINGS.md").read_text(),
        (HERE / "STREAM_CONSUMERS.md").read_text(),
        (HERE / "harness" / "README.md").read_text(),
        (HERE.parents[2] / "solver" / "dll-archive" / "README.md").read_text(),
    )
    assert "Batch 298" in docs[0] and "#1217" in docs[0]
    stale = (
        "globally refused under #1217",
        "globally fail-closed under #1217",
        "remains globally refused under #1217",
    )
    assert all(phrase not in text for text in docs for phrase in stale)
