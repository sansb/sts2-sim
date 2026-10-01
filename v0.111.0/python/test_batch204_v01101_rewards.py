"""Batch 204 / #798: v0.110.1 reward-boundary remodel pins.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent


def test_reward_only_relic_hooks_remain_outside_combat_prediction():
    raw = (HERE / "relic_templates.json").read_text()
    assert '"RELIC.PAELS_WING": "unsupported hook ' \
        'TryModifyCardRewardAlternatives"' in raw
    assert "ModifyCardRewardCreationOptions" in \
        (HERE / "test_batch56_energy_relics.py").read_text()
