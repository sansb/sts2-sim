"""#138 Batch 291 / #1209: exact Legion of Bone Osty fan-out.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_current_build_nested_move_next_and_summon_helpers_are_cited():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text().lower()
    for rva in ("0xe7b03", "0xe7b10", "0xe7b44", "0xe7b93",
                "0x3a6f38", "0x3eb7fc", "0x3e8b6c"):
        assert rva in mechanics
