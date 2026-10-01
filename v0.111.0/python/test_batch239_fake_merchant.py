"""#138 Batch 239 / #870: reachable Fake Merchant event combat.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_batch239_docs_and_capture_correction_are_durable():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text()
    assert "Batch 239 / #870" in mechanics
    assert "165748.350_mcr_845b9bec.mcr" in mechanics
    assert "0x37a050" in mechanics and "0x3589bc" in mechanics
    assert "2026-08-05-10-batch239-fake-merchant.md" in mechanics
