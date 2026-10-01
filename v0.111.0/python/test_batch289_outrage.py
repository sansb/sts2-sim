"""#138 Batch 289 / #1205: exact Outrage teammate-clone fan-out.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_current_build_nested_move_next_and_clone_helpers_are_cited():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text().lower()
    for rva in ("0xea8d6", "0xea8e3", "0xea8e6", "0xea8fc",
                "0xea94f", "0x3ae538"):
        assert rva in mechanics
