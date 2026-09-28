"""ENCOUNTER.QUEEN_BOSS certification: the 89SJD17KYUEH death fight.

First live Queen capture (2026-08-04, v0.110.1/db5d3552, Ironclad A10,
Sean's modded-profile run). The recorded input log replays through the
sim with EVERY play-level checksum exact — player HP and the full
living-monster (kind, hp, block) rows on all 21 plays — through the
Amalgam kill, a live Whistle stun on the Amalgam, Primal Force /
Infernal Blade mid-combat generation, and the lethal enemy turn.

The capture also pinned two harness fixes that ride in the same batch:

- live_coach save conversion read the enchantment strength from the
  historical "level" key; current-build saves store "amount". SHARP 3
  on the upgraded Gunk Up entered the sim as SHARP 1 and shorted each
  of its three hits by 2 pre-multiplier (the 9-HP Amalgam drift).
- mcr_replay mapped mid-combat adds only through the Soul Fysh BECKON
  pad; the generic mapping pairs combat_card_index k >= deck size with
  the sim card whose uid is (allocator base + k - deck size), both
  being creation-ordered.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))
import mcr_parser  # noqa: E402

TD = pathlib.Path(__file__).parent / "testdata"
ENTRY_SAVE = TD / "89SJD17KYUEH_queen_entry.save"
DEATH_MCR = TD / "89SJD17KYUEH_queen_death.mcr"
ENCOUNTER = "ENCOUNTER.QUEEN_BOSS"


def _save():
    return json.loads(ENTRY_SAVE.read_text())


def test_entry_conversion_reads_current_build_enchant_amount():
    """SHARP strength lives in save key "amount" (historical: "level")."""
    from live_coach import build_entry
    entry = build_entry(_save(), ENCOUNTER, "boss")
    gunk = next(c for c in entry["deck_entering"]
                if c["id"] == "CARD.GUNK_UP")
    assert gunk["upgrade_level"] == 1
    assert gunk["enchantment"] == "ENCHANTMENT.SHARP"
    assert gunk["enchant_amount"] == 3


def test_live_whistle_stun_and_generated_adds_replayed():
    """The log's own contents pin the Batch 230 mechanics live.

    WHISTLE is played at instance 19 on target_id 1 (creation ordinal 0
    = the Amalgam); instances 36-38 are Primal Force Giant Rocks and 39
    is the Infernal Blade Ashen Strike — mid-combat adds resolved by
    the creation-order uid mapping (deck size 34).
    """
    replay = mcr_parser.decode(DEATH_MCR)
    plays = [(e["action"]["combat_card_index"], e["action"]["card_id"],
              e["action"].get("target_id"))
             for e in replay["events"]
             if e.get("action", {}).get("type") == "NetPlayCardAction"]
    assert (19, "WHISTLE", 1) in plays
    assert len(_save()["players"][0]["deck"]) == 34
    adds = [(k, cid) for (k, cid, _) in plays if k >= 34]
    assert adds == [(36, "GIANT_ROCK"), (37, "GIANT_ROCK"),
                    (38, "GIANT_ROCK"), (39, "ASHEN_STRIKE")]
