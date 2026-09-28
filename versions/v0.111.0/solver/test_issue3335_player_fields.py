"""#3335: native checkpoints compare the player's gold, stars, potion belt and orbs.

`rust_replay.check_native_snapshot` (the census and the production review)
compared hp, max_hp, block, energy, turn, monsters, piles and RNG, but no
other `PlayerState` field `NetFullCombatState::FromRun` writes. Each field
Rust carries exactly is now its own named mismatch, raised after the RNG so
an earlier-field divergence keeps its pre-#3335 detail.
"""
import copy

import pytest

import mcr_native
import rust_replay as replay


def _pair(native_player=None, rust_player=None):
    words = [1, 2, 3, 4]
    streams = mcr_native._REPRESENTED_RNG_STREAMS
    native = {
        "players": [dict({
            "energy": 3, "turn_number": 1, "piles": [], "gold": 120,
            "stars": 4, "max_potion_count": 3,
            "potions": ["FIRE_POTION", "BLOCK_POTION"],
            "orbs": [{"id": "LIGHTNING_ORB", "passive": 3, "evoke": 8},
                     {"id": "DARK_ORB", "passive": 6, "evoke": 12}],
            # Serialized, deliberately not compared (no exact Rust field).
            "phase": "Play", "character": "DEFECT", "player_id": 1,
            "relics": [{"id": "CRACKED_CORE", "props": None, "floor_added": 1}],
            "player_rng": {}, "relic_grab_bag": {},
        }, **(native_player or {}))],
        "creatures": [{"player_id": 1, "current_hp": 50, "max_hp": 80,
                       "block": 0, "powers": []}],
        "rng": {"states": {s: words for s in streams.values()},
                "counters": {s: 0 for s in streams.values()}},
    }
    state = {
        "player": dict({
            "hp": 50, "max_hp": 80, "gold": 120, "stars": 4,
            "potion_slots": ["FIRE_POTION", None, "BLOCK_POTION"],
            "orbs": [["LIGHTNING", None], ["DARK", 12]],
        }, **(rust_player or {})),
        "monsters": [], "piles": {},
        "rng": {attr: {"words": words, "counter": 0} for attr in streams},
    }
    return state, native


def _detail(state, native):
    with pytest.raises(ValueError) as caught:
        replay.check_native_snapshot(state, native)
    return str(caught.value)


def test_agreeing_checkpoint_passes():
    replay.check_native_snapshot(*_pair())


def test_uncompared_fields_do_not_fail():
    """Phase, orb passive/evoke, relics, character and the player-level RNG
    sets are serialized but have no exact Rust counterpart."""
    state, native = _pair(native_player={
        "phase": "End", "character": "IRONCLAD", "player_id": 7,
        "relics": [], "player_rng": {"seed": 1}, "relic_grab_bag": {"Rare": []},
        "orbs": [{"id": "LIGHTNING_ORB", "passive": 99, "evoke": 99},
                 {"id": "DARK_ORB", "passive": 0, "evoke": 0}]})
    replay.check_native_snapshot(state, native)


@pytest.mark.parametrize("native_player, rust_player, detail", [
    ({"gold": 35}, {"gold": 23},
     "recorded replay differs from native gold: native 35 rust 23"),
    # Rust elides a zero field; the default is 0, never a skip.
    ({"gold": 12}, {"gold": None},
     "recorded replay differs from native gold: native 12 rust 0"),
    ({"stars": 5}, {},
     "recorded replay differs from native stars: native 5 rust 4"),
    ({"max_potion_count": 2}, {},
     "recorded replay differs from native potion_slots: native 2 rust 3"),
    ({"potions": ["BLOCK_POTION", "FIRE_POTION"]}, {},
     "recorded replay differs from native potions: native "
     "['BLOCK_POTION', 'FIRE_POTION'] rust ['FIRE_POTION', 'BLOCK_POTION']"),
    ({"potions": ["FIRE_POTION"]}, {},
     "recorded replay differs from native potions: native "
     "['FIRE_POTION'] rust ['FIRE_POTION', 'BLOCK_POTION']"),
    ({"orbs": [{"id": "DARK_ORB", "passive": 6, "evoke": 12},
               {"id": "LIGHTNING_ORB", "passive": 3, "evoke": 8}]}, {},
     "recorded replay differs from native orbs: native "
     "['DARK', 'LIGHTNING'] rust ['LIGHTNING', 'DARK']"),
    # An orb id outside the kind table can never equal a Rust orb.
    ({"orbs": [{"id": "MYSTERY_ORB", "passive": 1, "evoke": 1},
               {"id": "DARK_ORB", "passive": 6, "evoke": 12}]}, {},
     "recorded replay differs from native orbs: native "
     "['MYSTERY_ORB', 'DARK'] rust ['LIGHTNING', 'DARK']"),
])
def test_each_field_is_a_named_mismatch(native_player, rust_player, detail):
    state, native = _pair(native_player, rust_player)
    state["player"] = {k: v for k, v in state["player"].items() if v is not None}
    assert _detail(state, native) == detail


def test_field_names_reach_the_census_tally():
    import pathlib
    import sys
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent
                           / "rust" / "tools"))
    import eval_suite
    for field in ("gold", "stars", "potion_slots", "potions", "orbs"):
        assert eval_suite.checkpoint_mismatch_field(
            f"ValueError: recorded replay differs from native {field}: "
            f"native 1 rust 2") == field


def test_earlier_fields_keep_their_detail():
    """The new comparison runs after the RNG, so a checkpoint that already
    disagreed elsewhere reports the same field it did before #3335."""
    state, native = _pair({"gold": 1})
    state["player"]["hp"] = 49
    assert _detail(state, native) == 'recorded replay differs from native player state'
    state, native = _pair({"gold": 1})
    state["rng"] = copy.deepcopy(state["rng"])
    next(iter(state["rng"].values()))["counter"] = 9
    assert _detail(state, native) == 'recorded replay differs from native RNG'


def test_a_native_checkpoint_without_gold_is_not_certified():
    state, native = _pair()
    del native["players"][0]["gold"]
    with pytest.raises(KeyError):
        replay.check_native_snapshot(state, native)
