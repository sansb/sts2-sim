"""#138 Batch 217 / #196: exact Ovicopter and in-place Tough Egg hatch.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_issue1588_current_build_power_removal_and_identity_evidence():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text()
    for exact_fragment in (
        "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4",
        "`ToughEgg::HatchMove` outer/body are **0xc31ec/0x372874**",
        "at `0x00a9-0x00dd`",
        "at `0x00f1-0x0168`",
        "predicate **0x37249a**",
        "outer/body **0xc3230/0x3726d4**",
    ):
        assert exact_fragment in mechanics
