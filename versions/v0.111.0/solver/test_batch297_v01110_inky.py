"""#138 Batch 297 / #1222: v0.111.0 Inky remodel pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_current_docs_close_the_complete_version_impact_gate():
    streams = (HERE / "STREAM_CONSUMERS.md").read_text()
    findings = (HERE / "RNG_FINDINGS.md").read_text()
    assert "globally fail-closed under #1217" not in streams
    assert "globally refused under #1217" not in findings
    assert "#1222" not in streams and "#1222" not in findings
