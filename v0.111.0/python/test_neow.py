"""Neow blessing option generation (#626).

The engine fixture in `testdata/neow_options_v109.json` was produced by
`sim/v0.111.0/python/harness/probe_neow.py`, which calls the real
`Neow::GenerateInitialOptions` in sts2.dll v0.109.1. Regenerate it with that
tool if the game build changes; it needs the local game install, so these
tests replay the recorded oracle instead of re-running the engine.
"""
import json
from pathlib import Path

import pytest

from neow import (CURSE_EXCLUSIONS, CURSE_OPTIONS, LARGE_CAPSULE,
                  POSITIVE_OPTIONS, event_rng, generate_neow_options,
                  options_from_save)

FIXTURE = Path(__file__).parent / "testdata" / "neow_options_v109.json"

with FIXTURE.open() as _fh:
    _DATA = json.load(_fh)

EPOCHS = frozenset(_DATA["unlocked_epochs"])
RECORDS = _DATA["records"]

# The certified capture: ~/sts2-captures/2026-07-26/084144.724_save_6ccd86c1
# .save, whose follow-up save (085337.192_save_8b453cf1.save, same seed) shows
# the player holding RELIC.LARGE_CAPSULE -- the curse this seed offers.
CERTIFIED_SEED = "LPTMBBTY7FQY"


def _predict(record):
    return generate_neow_options(
        record["seed"], unlocked_epochs=EPOCHS,
        scroll_boxes_allowed=record["scroll_boxes_allowed"])


@pytest.mark.parametrize("record", RECORDS, ids=lambda r: r["seed"])
def test_matches_engine_oracle(record):
    """Every option, in order, and the exact event-stream draw count."""
    got = _predict(record)
    assert list(got.all_options) == record["options"]
    assert got.draws == record["draws"]


@pytest.mark.parametrize("record", RECORDS, ids=lambda r: r["seed"])
def test_event_seed_matches_engine(record):
    """The dedicated event Rng seed is run seed + slot + hash("NEOW")."""
    rng = event_rng(record["seed"])
    assert rng.counter == 0
    assert rng.seed == record["event_seed"]


def test_certified_capture_curse_is_large_capsule():
    """Live-game ground truth: the player who ran this seed came out of Neow
    holding RELIC.LARGE_CAPSULE, so that must be the curse offered."""
    record = next(r for r in RECORDS if r["seed"] == CERTIFIED_SEED)
    got = _predict(record)
    assert got.curse == LARGE_CAPSULE
    assert got.all_options == ("RELIC.NEW_LEAF", "RELIC.KALEIDOSCOPE",
                               LARGE_CAPSULE)


def test_large_capsule_curse_skips_the_lava_rock_flip():
    """Neow::GenerateInitialOptions IL_0184: an isinst LargeCapsule test
    branches past the Lava Rock / Small Capsule NextBool, so neither ever
    appears alongside a Large Capsule curse."""
    seen = 0
    for record in RECORDS:
        if record["options"][2] != LARGE_CAPSULE:
            continue
        seen += 1
        assert "RELIC.LAVA_ROCK" not in record["options"]
        assert "RELIC.SMALL_CAPSULE" not in record["options"]
    assert seen, "fixture no longer covers the Large Capsule branch"


@pytest.mark.parametrize("curse,excluded", sorted(CURSE_EXCLUSIONS.items()))
def test_curse_excludes_its_counterpart(curse, excluded):
    """b__35_1..b__35_6: a drawn curse strikes its thematic positive."""
    for record in RECORDS:
        if record["options"][2] != curse:
            continue
        for relic in excluded:
            assert relic not in record["options"][:2]


def test_every_curse_is_covered_by_the_fixture():
    """Guard against a fixture regeneration that silently loses branches."""
    offered = {r["options"][2] for r in RECORDS}
    assert offered == set(CURSE_OPTIONS)


def test_single_player_pool_filtering():
    """MassiveScroll needs >1 player; WingedBoots/SilverCrucible need ==1."""
    for record in RECORDS:  # the whole fixture is single-player
        assert "RELIC.MASSIVE_SCROLL" not in record["options"]


def test_multiplayer_flips_the_player_count_gates():
    """With >1 player WingedBoots and SilverCrucible drop out of their pools
    (MassiveScroll becomes eligible, though it need not land in the top two)."""
    for record in RECORDS:
        got = generate_neow_options(
            record["seed"], player_count=2, unlocked_epochs=EPOCHS,
            scroll_boxes_allowed=record["scroll_boxes_allowed"])
        assert "RELIC.WINGED_BOOTS" not in got.all_options
        assert got.curse != "RELIC.SILVER_CRUCIBLE"


def test_kaleidoscope_gated_on_all_characters_unlocked():
    """Kaleidoscope::IsAllowedAtNeow (0x241858) requires all five character
    card pools, i.e. every character epoch revealed."""
    partial = EPOCHS - {"DEFECT1_EPOCH"}
    for record in RECORDS:
        got = generate_neow_options(
            record["seed"], unlocked_epochs=partial,
            scroll_boxes_allowed=record["scroll_boxes_allowed"])
        assert "RELIC.KALEIDOSCOPE" not in got.all_options


def test_scroll_boxes_requires_an_explicit_verdict():
    """I5: CanGenerateBundles needs card-pool data the save does not carry,
    so the generator refuses rather than guessing."""
    with pytest.raises(NotImplementedError, match="ScrollBoxes"):
        generate_neow_options(CERTIFIED_SEED, unlocked_epochs=EPOCHS)


def test_modifiers_refuse_rather_than_approximate():
    with pytest.raises(NotImplementedError, match="modifier"):
        generate_neow_options(CERTIFIED_SEED, unlocked_epochs=EPOCHS,
                              scroll_boxes_allowed=True,
                              modifiers=("MODIFIER.SOMETHING",))


def test_pre_v109_builds_refuse():
    with pytest.raises(NotImplementedError):
        generate_neow_options(CERTIFIED_SEED, unlocked_epochs=EPOCHS,
                              scroll_boxes_allowed=True, build="v0.108.0")


def test_pools_match_the_il_arrays():
    """Pool sizes are load-bearing: UnstableShuffle draws len-1 times, so a
    dropped entry silently shifts every downstream draw."""
    assert len(CURSE_OPTIONS) == 10
    assert len(POSITIVE_OPTIONS) == 14
    assert len(set(CURSE_OPTIONS) & set(POSITIVE_OPTIONS)) == 0


def test_options_from_save_round_trip():
    save = {
        "rng": {"seed": CERTIFIED_SEED},
        "modifiers": [],
        "players": [{"unlock_state": {"unlocked_epochs": sorted(EPOCHS)}}],
    }
    got = options_from_save(save, scroll_boxes_allowed=True)
    assert got.curse == LARGE_CAPSULE
