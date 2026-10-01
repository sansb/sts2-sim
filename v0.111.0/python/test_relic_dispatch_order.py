"""#808: same-hook relic dispatch order recorded by schema>=19 saves.

v0.110.1 Hook/<AfterCardPlayed>d__16::MoveNext 0x3ca4d0 awaits each model of
the CombatState/<IterateHookListeners>d__69::MoveNext 0x3f6700 snapshot,
which enumerates Player.Relics (_relics) by ascending index — same-hook
relics fire in inventory-list order. The save's players[N].relics is that
list itself (ToSerializable 0x11a804 / PopulateRelics 0x11b3c0), so
live_coach vouches for the entry's relic order via
relics_entering_dispatch_ordered, and the deleted Python start_combat admitted AfterCardPlayed
peers of Iron Club/Tuning Fork exactly when every peer precedes both
counters — the order the sim's fixed counter-relic suffix implements.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from tools.live_coach import build_entry


FORK = "RELIC.TUNING_FORK"
GAME_PIECE = "RELIC.GAME_PIECE"


def _save(relics, rng):
    save = {
        "visited_map_coords": [[0, 0]],
        "players": [{
            "deck": [{"id": "CARD.BASH", "current_upgrade_level": 0}],
            "relics": [{"id": rid} for rid in relics],
            "potions": [], "max_potion_slot_count": 2,
            "current_hp": 50, "max_hp": 50, "gold": 7,
        }],
    }
    if rng is not None:
        save["rng"] = rng
    return save


def test_live_coach_vouches_only_for_schema19_save_lists():
    relics = ("RELIC.BURNING_BLOOD", GAME_PIECE, FORK)
    schema19 = _save(relics, {"seed": "X", "rngs": {
        "Shuffle": {"counter": 0, "s0": 1, "s1": 2, "s2": 3, "s3": 4}}})
    entry = build_entry(schema19, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert entry["relics_entering_dispatch_ordered"] is True
    assert entry["relics_entering"] == list(relics)

    schema18 = _save(relics, {"seed": "X", "counters": {"Shuffle": 0}})
    entry = build_entry(schema18, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert entry["relics_entering_dispatch_ordered"] is False

    no_rng = _save(relics, None)
    entry = build_entry(no_rng, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert entry["relics_entering_dispatch_ordered"] is False
