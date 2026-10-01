"""#138 Batch 288 / #1203: exact Largesse target-pool generation.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_current_build_nested_move_next_and_pool_helpers_are_cited():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text().lower()
    for rva in ("0xe795d", "0xe796a", "0xe79ea", "0x3a6a04",
                "0xf4b54", "0x81e64", "0x116290", "0x116bfc",
                "0x133f74", "0x3e064c"):
        assert rva in mechanics
