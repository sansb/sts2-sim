"""#138 Batch 133 / #614: exact Infernal Blade generation admission.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from tools.live_coach import (infernal_blade_pool_fully_unlocked,
                              player_character_id)


def test_batch133_live_unlock_epoch_proof_is_explicit():
    epochs = [
        "IRONCLAD2_EPOCH", "EPOCH.IRONCLAD5_EPOCH", "IRONCLAD7_EPOCH"]
    player = {"unlock_state": {"unlocked_epochs": epochs}}
    assert infernal_blade_pool_fully_unlocked(player) is True
    assert infernal_blade_pool_fully_unlocked({
        "unlock_state": {"unlocked_epochs": epochs[:-1]}}) is False
    assert infernal_blade_pool_fully_unlocked({}) is None
    assert player_character_id({
        "character_id": "CHARACTER.IRONCLAD"}) == "CHARACTER.IRONCLAD"
    assert player_character_id({"character": "IRONCLAD"}) == \
        "CHARACTER.IRONCLAD"
