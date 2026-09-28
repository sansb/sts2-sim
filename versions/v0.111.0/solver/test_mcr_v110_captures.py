"""
v0.110.1 live-capture pins — issue #807 (revive the .mcr decoder after
the v0.110.1 bump).

Fixtures from the 2026-07-31 coached session on seed TQM88QFMHSQR
(Ironclad A10 Standard, the first fully-coached act-boss-kill run),
harvested by tools/mcr_watch.py:

  testdata/TQM88QFMHSQR_sludge_entry.save     entering node 5
      (SLUDGE_SPINNER_WEAK, fight 2): shuffle 46, niche 4, ai 0, hp 47
  testdata/TQM88QFMHSQR_sludge_win.mcr        the executed win (10 plays)
  testdata/TQM88QFMHSQR_colony_entry.save     entering node 6
      (SKULKING_COLONY_ELITE, fight 3): shuffle 75, niche 5, ai 3, hp 47
  testdata/TQM88QFMHSQR_colony_win.mcr        the win, including the
      Colorless Potion 1-of-3 Index PlayerChoice (the generated FINESSE
      is later played as instance 17, one past the 17-card deck)
  testdata/TQM88QFMHSQR_eel_entry.save        entering node 11
      (TERROR_EEL_ELITE, fight 6): shuffle 168, ai 3, 20-card deck
  testdata/TQM88QFMHSQR_eel_win.mcr           the win (#824): the
      Colorless Potion generates SEEKER_STRIKE (played as instance 20),
      whose two frame_select draw-shortlist choices are recorded as
      CombatCard PlayerChoices (instances 13 and 12)
  testdata/TQM88QFMHSQR_gardeners_entry.save  entering node 13
      (PHANTASMAL_GARDENERS_ELITE, fight 7): shuffle 220, niche 18
  testdata/TQM88QFMHSQR_gardeners_win.mcr     the win, including an
      enemy-targeted Powdered Demise use (target_id 4, creation-ordinal
      scheme, target_player_id null)
  testdata/TQM88QFMHSQR_fabricator_entry.save entering node 38 (act 3,
      FABRICATOR_NORMAL, fight 18)
  testdata/TQM88QFMHSQR_fabricator_win.mcr    the win (#825): kills the
      spawned Zapbot mid-fight, exercising the dead-bot power-state
      validation at every later end_player_turn
  testdata/TQM88QFMHSQR_scrolls3_entry.save   entering node 42 (act 3,
      SCROLLS_OF_BITING_NORMAL, fight 19): per-act index 9 — the
      global-vs-per-act node regression fixture (#826)
  testdata/TQM88QFMHSQR_scrolls3_win.mcr      the win: starter roll 2
      under the correct total_floor, one post-CHEW MonsterAi draw

What these pin:

1. The regenerated v0.110.1 net-id tables reproduce the modelIdHash the
   GAME wrote into every 2026-07-31 replay header (0xea293d3f) — the
   #639 reconstruction of ModelIdSerializationCache.Init holds on
   v0.110.1 unchanged; only the tables were stale (issue #807's
   "reconstruction itself is stale" premise was the pre-#639 state).
2. Full decodes consume every bit; the v0.109.1 wire schema carried
   over to v0.110.1 unchanged.
3. The decoded run-scope rng set (counter AND xoshiro s0..s3 per
   stream) equals the paired current_run.save — an external oracle:
   nothing in the .mcr decode path produced those numbers.
4. The recorded entering Shuffle counter reproduces every recorded play
   position-for-position under the v0.110.1 seeding scheme (the .mcr
   header's version string MUST reach sts2_rng: the default build is
   the v0.108 scheme, which scrambles every v0.109+ permutation —
   exactly how mcr_validate's play-log check silently rotted).
5. Full input-log replays through combat_sim reproduce the game's own
   recorded outcomes (the checksum section's final player HP), through
   two newly-transcribed replay paths: the generation-potion Index
   choice and the enemy-targeted potion.

Run: python3 -m pytest versions/v0.111.0/solver/test_mcr_v110_captures.py

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

# Ground truth: the modelIdHash v0.110.1 (db5d3552) actually wrote into
# its replay headers — read from the 2026-07-31 session captures. Do NOT
# relax this to match a rebuilt mcr_tables.json: agreement of both sides
# on a wrong value is exactly the failure mode #639 existed to catch.
HASH_V110 = 0xEA293D3F
HASH_V109 = 0xFC40A98D

FIXTURES = {
    "sludge": ("ENCOUNTER.SLUDGE_SPINNER_WEAK", "monster"),
    "colony": ("ENCOUNTER.SKULKING_COLONY_ELITE", "elite"),
    "eel": ("ENCOUNTER.TERROR_EEL_ELITE", "elite"),
    "gardeners": ("ENCOUNTER.PHANTASMAL_GARDENERS_ELITE", "elite"),
}


def _save(tag):
    return json.loads((TD / f"TQM88QFMHSQR_{tag}_entry.save").read_text())


def _mcr(tag):
    return TD / f"TQM88QFMHSQR_{tag}_win.mcr"


def test_tables_match_the_hash_the_game_wrote():
    # v0.110.1 is served by its FROZEN copy since the current tables moved
    # to v0.111.0 (test_mcr_v111_captures pins the current file); the
    # frozen v0.109.1 copy serves both v0.109 point releases
    # (CIL-identical per the #625 check).
    frozen = mcr_parser.tables_for_version("v0.110.1")
    assert frozen["game_version"] == "v0.110.1"
    assert frozen["model_id_hash"] == HASH_V110
    assert frozen is not mcr_parser.load_tables(mcr_parser.TABLES_PATH)
    for v109 in ("v0.109.0", "v0.109.1"):
        assert mcr_parser.tables_for_version(v109)["model_id_hash"] \
            == HASH_V109
    assert HASH_V110 != HASH_V109


def test_v110_full_decode_consumes_every_bit():
    # decode() raises on hash mismatch, unconsumed bits, or nonzero
    # padding — reaching the asserts means all three checks passed.
    for tag in FIXTURES:
        replay = mcr_parser.decode(_mcr(tag))
        assert replay["version"] == "v0.110.1"
        assert replay["git_commit"] == "db5d3552"
        assert replay["model_id_hash"] == HASH_V110


def test_decoded_rng_set_matches_the_paired_save():
    # External oracle: the .mcr's combat-start rng snapshot must equal
    # the current_run.save captured entering the same node — counter AND
    # xoshiro state words, every run-scope stream, plus the seed.
    for tag in FIXTURES:
        save_rngs = _save(tag)["rng"]["rngs"]
        decoded = mcr_parser.decode(_mcr(tag))["run"]["rng"]
        assert decoded["seed"] == _save(tag)["rng"]["seed"] \
            == "TQM88QFMHSQR"
        for stream, counter in decoded["counters"].items():
            recorded = save_rngs[snake_case(stream)]
            assert counter == recorded["counter"], (tag, stream)
            assert tuple(decoded["states"][stream]) \
                == tuple(recorded[k] for k in ("s0", "s1", "s2", "s3")), \
                (tag, stream)
