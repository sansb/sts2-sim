"""The .mcr decoder on v0.111.0, and the first certified fight on the admitted
build (#1283 eval-suite groundwork; #1272's missing capture artifact).

`mcr_tables.json` tracks the CURRENT install and was last regenerated for
v0.110.1, so every v0.111.0 replay written since 2026-08-14 refused with a
modelIdHash mismatch — 540 of them on this machine, across all five
characters, sitting unread while #1272 recorded the v0.111 capture evidence
as absent. Regenerating the tables against the archived v0.111.0 DLL
(`sim/dll-archive/v0.111.0/`, #1221) reproduces the header hash the game
wrote, `0x5d828510`, with no bitstream change.

The pinned fixture is the Mawler fight of seed 6P96T755CNZ3 — the same run
and the same fight `test_v0111_ingame_validation.py` exhausts as the v0.111.0
slow pin. That pin knows the human's outcome only from the `.run` (5 turns,
21 damage); this capture supplies the human's exact 17-action line, and the
native per-action checksums certify the simulator's state after each of
them. It is the first fight on an admitted build with all three: a `.run`,
an entry save, and a checksum-bearing replay.

Run: solver/tools/pytest_lane.sh -- sim/v0.111.0/python/test_mcr_v111_captures.py

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))
sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))

import mcr_parser  # noqa: E402
from sts2_rng import snake_case  # noqa: E402

TD = pathlib.Path(__file__).parent / "testdata"

# Ground truth: the modelIdHash v0.111.0 (41cef1ea) actually wrote into its
# replay headers — read from every capture of the 2026-08-14..24 sessions.
# Do NOT relax this to match a rebuilt mcr_tables.json: agreement of both
# sides on a wrong value is exactly the failure mode #639 existed to catch.
HASH_V111 = 0x5D828510
HASH_V110 = 0xEA293D3F

SEED = "6P96T755CNZ3"
SAVE = TD / f"{SEED}_mawler_entry.save"
MCR = TD / f"{SEED}_mawler_win.mcr"


def _save():
    return json.loads(SAVE.read_text())


def test_current_tables_are_v0111_and_match_the_hash_the_game_wrote():
    current = mcr_parser.load_tables(mcr_parser.TABLES_PATH)
    assert current["game_version"] == "v0.111.0"
    assert current["model_id_hash"] == HASH_V111
    # v0.111.0 has no frozen copy: it IS the current install, so it resolves
    # through the default path. v0.110.1 moved to its frozen copy.
    assert mcr_parser.tables_for_version("v0.111.0") is current
    assert mcr_parser.tables_for_version("v0.110.1")["model_id_hash"] \
        == HASH_V110
    assert HASH_V111 != HASH_V110


def test_v111_full_decode_consumes_every_bit():
    # decode() raises on hash mismatch, unconsumed bits, or nonzero
    # padding — reaching the asserts means all three checks passed.
    replay = mcr_parser.decode(MCR)
    assert replay["version"] == "v0.111.0"
    assert replay["git_commit"] == "41cef1ea"
    assert replay["model_id_hash"] == HASH_V111
    assert len(replay["events"]) == 21
    assert len(replay["checksums"]) == 34


def test_decoded_rng_set_matches_the_paired_save():
    # External oracle: the .mcr's combat-start rng snapshot must equal the
    # save captured entering the same node — counter AND xoshiro state
    # words, every run-scope stream, plus the seed.
    save = _save()
    decoded = mcr_parser.decode(MCR)["run"]["rng"]
    assert decoded["seed"] == save["rng"]["seed"] == SEED
    for stream, counter in decoded["counters"].items():
        recorded = save["rng"]["rngs"][snake_case(stream)]
        assert counter == recorded["counter"], stream
        assert tuple(decoded["states"][stream]) \
            == tuple(recorded[k] for k in ("s0", "s1", "s2", "s3")), stream
