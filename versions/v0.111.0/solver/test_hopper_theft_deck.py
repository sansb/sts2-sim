"""
Same-node removal boundary in the .run deck reconstruction (#827).

The stamp semantics (CardPileCmd.Add writes FloorAddedToDeck =
IRunState.TotalFloor at ADD time, re-adds RESTAMP) make a Thieving
Hopper theft look like this in the .run:

  TQM88QFMHSQR node 18 (THIEVING_HOPPER_WEAK, fight 9, floor 19):
    cards_removed: [COLOSSUS, floor_added 7]   <- the theft, mid-fight
    cards_gained:  [COLOSSUS]                  <- the return at fight end
  final deck: COLOSSUS restamped floor 19, array position moved to the
  re-add slot — so the pre-theft fights' membership AND order live only
  in the removal record.

Both reconstruction paths treated a removal at the fight's OWN node as
pre-fight (`j < floor` / `j <= i`), silently dropping the stolen card
from that fight's entry deck — and the restamp additionally hid it from
every fight since its REAL acquisition (node 6). Fight-node rewards only
ADD, so a same-node removal is in-combat (or a post-fight reward-relic
transform) and the fight began with the card.

Ground truth here: the committed entry SAVES are the game's own deck
arrays, and the hopper fight's recorded play log is reproduced
position-for-position by the run-only reconstruction only when the
boundary is right. HQPAXCBS6P's node-19 Taunt removal is the same
signature on the certified CardSel run (also a Thieving Hopper).

Run: python3 -m pytest versions/v0.111.0/solver/test_hopper_theft_deck.py

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))

from relay_parser import parse_run  # noqa: E402
from replay_fight import entry_deck, load_combats  # noqa: E402

TD = pathlib.Path(__file__).parent / "testdata"
RUN = TD / "TQM88QFMHSQR.run"

# fight index -> committed entry-save fixture (the game's deck array)
SAVE_ORACLES = {2: "sludge", 3: "colony", 6: "eel", 7: "gardeners",
                9: "hopper"}


def _save_ids(tag):
    save = json.loads((TD / f"TQM88QFMHSQR_{tag}_entry.save").read_text())
    return [c["id"] for c in save["players"][0]["deck"]]


def test_run_only_entry_decks_equal_the_game_deck_arrays():
    run = load_combats(str(RUN))
    for fight_idx, tag in SAVE_ORACLES.items():
        fight = run["combats"][fight_idx]
        ids = [c["id"] for c in entry_deck(run["deck"], fight["floor"],
                                           run["removals"])]
        assert ids == _save_ids(tag), (fight_idx, tag)


def test_stolen_colossus_is_back_in_the_hopper_fight():
    # Membership AND position: the save array has it at index 17, and the
    # no-identical-sibling re-insertion (after the last stamp <= 7 card)
    # lands exactly there. The restamped array copy (floor 19) must not
    # double it.
    run = load_combats(str(RUN))
    fight = run["combats"][9]
    assert fight["encounter"] == "ENCOUNTER.THIEVING_HOPPER_WEAK"
    assert fight["floor"] == 19
    ids = [c["id"] for c in entry_deck(run["deck"], fight["floor"],
                                       run["removals"])]
    assert ids.count("CARD.COLOSSUS") == 1
    assert ids.index("CARD.COLOSSUS") == _save_ids("hopper").index(
        "CARD.COLOSSUS") == 17


def test_hqpaxcbs6p_taunt_survives_into_its_own_theft_fight():
    # The certified CardSel run has the same signature: TAUNT removed at
    # node 19, itself a THIEVING_HOPPER_WEAK fight. Both reconstruction
    # paths must keep it in that fight's entry deck (it was previously
    # silently excluded; no pinned hand touched this fight).
    run = load_combats(str(TD / "HQPAXCBS6P.run"))
    idx = next(k for k, c in enumerate(run["combats"])
               if c["node_index"] == 19)
    fight = run["combats"][idx]
    assert fight["encounter"] == "ENCOUNTER.THIEVING_HOPPER_WEAK"
    ids = [c["id"] for c in entry_deck(run["deck"], fight["floor"],
                                       run["removals"])]
    assert "CARD.TAUNT" in ids
    parsed = parse_run(str(TD / "HQPAXCBS6P.run"))
    assert "CARD.TAUNT" in [c["id"]
                            for c in parsed.fights[idx].deck_entering]
    # This hopper also returned its loot (cards_gained TAUNT at node 19,
    # final-array copy restamped floor 20): fights AFTER the theft carry
    # exactly ONE copy — the restamped array entry, with the removal
    # record correctly not re-inserted alongside it.
    later = next(c for c in run["combats"] if c["node_index"] > 19)
    later_ids = [c["id"] for c in entry_deck(run["deck"], later["floor"],
                                             run["removals"])]
    assert later_ids.count("CARD.TAUNT") == 1
    # ...and the theft fight itself carries exactly one too (the removal
    # record, floor 13; the floor-20 array copy is excluded there).
    assert ids.count("CARD.TAUNT") == 1
