"""
Decode pin for the issue-#118 harness's first Standard-mode capture
(HYHM8WP1E5, harvested 2026-07-13/14): `testdata/HYHM8WP1E5_boss.mcr`, the
Act-1 boss fight's combat-start snapshot and full 36-event input log.

This module was `test_mcr_validate.py`, whose other pins replayed the capture
through the Python simulator's counter accounting (`tools/mcr_validate.py`).
#2827 item F deleted both; the decoder pin below never reached them.
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))

import mcr_parser  # noqa: E402

TD = pathlib.Path(__file__).parent / "testdata"
MCR = TD / "HYHM8WP1E5_boss.mcr"


def test_boss_mcr_full_decode_and_dynamic_vars():
    replay = mcr_parser.decode(MCR)     # raises on any unconsumed bit
    assert replay["version"] == "v0.108.0"
    run = replay["run"]
    assert run["rng"]["seed"] == "HYHM8WP1E5"
    assert run["game_mode"] == "Standard"
    assert len(replay["events"]) == 36
    # the SUNKEN_STATUE event choice exercises both dynamic-var branches
    # the first sample file never hit (float/string were swapped before)
    for act in run["map_point_history"]:
        for node in act:
            for ps in node["players"]:
                for choice in ps["event_choices"]:
                    if "SUNKEN_STATUE" in choice["title_loc_key"]:
                        v = choice["variables"]
                        assert v["Relic"] == "Sword of Stone"
                        assert v["HpLoss"] == 7.0
                        return
    raise AssertionError("SUNKEN_STATUE choice not found in history")
