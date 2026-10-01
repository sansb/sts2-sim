"""#138 Batch 296 / #1218: v0.111.0 RNG/relic remodel pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_current_docs_close_the_complete_version_impact_gate():
    streams = (HERE / "STREAM_CONSUMERS.md").read_text()
    findings = (HERE / "RNG_FINDINGS.md").read_text()
    assert "max(0, n - 1)" in streams
    assert "acquisition-time" in streams and "baseline" in streams
    assert "remains globally refused" not in findings
    assert "#1222" not in findings
    assert "#1217, #1218, and #1222" not in findings
