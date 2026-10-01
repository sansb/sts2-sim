"""#138 Batch 160 / #679: exact Alchemize generation and procurement.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path

from tools.live_coach import (build_entry, potion_pool_fully_unlocked,
                              stream_counters)


HERE = Path(__file__).parent


def test_batch160_live_coach_reports_the_root_helper_contract():
    live_source = (HERE / "tools/live_coach.py").read_text()
    # #2827 item F1: the live coach roots in Rust, so the caveat is read off
    # the Rust root (`live_coach.potion_caveat`) rather than this helper; its
    # own witnesses are in `test_live_coach.py`.
    call = "caveat = potion_caveat(root)"
    assert call in live_source
    assert 'print(f"! CAVEAT: {caveat}")' in live_source
    # Rust owns the search. The Python operator shell still declares the
    # narrowed potion action space next to both solved and no-solution output.
    assert "rust_exact_solve.solve_document(" in live_source
    assert live_source.count('print(f"! CAVEAT: {caveat}")') >= 3


def test_batch160_live_entry_and_schema19_counter_provenance():
    save = json.loads(
        (HERE / "testdata/8DVXPWUWRY_fog_entry.save").read_text())
    player = save["players"][0]
    player["deck"] = [{
        "id": "CARD.ALCHEMIZE", "current_upgrade_level": 0}]
    player["potions"] = [{
        "id": "POTION.STRENGTH_POTION", "slot_index": 2}]
    player["max_potion_slot_count"] = 4
    player["unlock_state"] = {"unlocked_epochs": [
        "EPOCH.IRONCLAD4_EPOCH", "EPOCH.POTION1_EPOCH",
        "EPOCH.POTION2_EPOCH"]}
    entry = build_entry(save, "ENCOUNTER.BATCH160", "monster")
    assert entry["max_potion_slot_count"] == 4
    assert entry["potion_slots_entering"] == [{
        "id": "POTION.STRENGTH_POTION", "slot_index": 2}]
    assert potion_pool_fully_unlocked(player) is True

    counters = save["rng"].pop("counters")
    save["rng"]["rngs"] = {
        name: {"counter": counter, "s0": 1, "s1": 2, "s2": 3, "s3": 4}
        for name, counter in counters.items()
    }
    save["rng"]["rngs"]["combat_potion_generation"] = {
        "counter": 37, "s0": 1, "s1": 2, "s2": 3, "s3": 4}
    assert stream_counters(save)["combat_potion_generation"] == 37
