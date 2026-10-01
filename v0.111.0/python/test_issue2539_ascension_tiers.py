"""#2539 part 1: monster HP and spawn-time amounts follow the fight's ascension.

Native (v0.111.0 `sts2.dll` sha256 9cb4f1ad...): every tiered monster
constant is `AscensionHelper::GetValueIfAscension(gate, atOrAbove, below)`
(RVA 0x106c4c), which is `atOrAbove` iff `AscensionManager::HasLevel(gate)`
(0x11fa83: `!(_level < gate)`, i.e. `>=`). These tests pin that selection on
the oracle's opening, its mid-fight spawns and its roster validators, and
pin the IL-read tiers of the rows the DLL content manifest refuses. The
check of every roster builder against that manifest (`MONSTER_MODELS`'s
source) lives in `engine/tools/test_dll_content_source.py`
(`EveryRosterTierIsTheManifests`), because the manifest is outside the fast
gate's trigger set.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import pathlib
import sys

from tools.live_coach import build_entry

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent / "engine" / "tools"))


def test_build_entry_reads_the_saves_ascension():
    save = {
        "visited_map_coords": [[0, 0]], "ascension": 7,
        "players": [{"deck": [], "relics": [], "potions": [],
                     "current_hp": 50, "max_hp": 50, "gold": 0}],
    }
    assert build_entry(save, "ENCOUNTER.X", "monster")["ascension"] == 7
    del save["ascension"]
    assert build_entry(save, "ENCOUNTER.X", "monster")["ascension"] is None
