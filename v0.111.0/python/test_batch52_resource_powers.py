"""Batch 52 exact pins for persistent draw/max-energy resource powers.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))


HERE = pathlib.Path(__file__).parent


def test_three_raw_templates_leave_only_the_reviewed_ledger():
    raw = json.loads((HERE / "card_templates_raw.json").read_text())
    checked = json.loads((HERE / "card_templates.json").read_text())
    ledger = json.loads((HERE / "card_refusal_ledger.json").read_text())
    expected = {
        "CARD.MACHINE_LEARNING": {
            "0": [["power", "MachineLearningPower", "self", 1]],
            "1": [["power", "MachineLearningPower", "self", 1]],
        },
        "CARD.DEMESNE": {
            "0": [["power", "DemesnePower", "self", 1]],
            "1": [["power", "DemesnePower", "self", 1]],
        },
        "CARD.PYRE": {
            "0": [["power", "PyrePower", "self", 1]],
            "1": [["power", "PyrePower", "self", 2]],
        },
    }
    for cid, levels in expected.items():
        assert raw["cards"][cid]["levels"] == levels
        assert checked["cards"][cid] == raw["cards"][cid]
        assert cid not in ledger["explicit_refusals"]
        assert cid not in checked["refused"]
    # Batch 69 retires Danse Macabre and Spirit of Ash; W173 retires
    # Infinite Blades; W197 retires Juggling; W198 retires Biased Cognition
    # and Fasten; W199 retires Pale Blue Dot and Wraith Form after their
    # exact hooks land; W229 retires Drum of Battle; Batch 128 retires
    # Colossus after its exact incoming-damage hook lands; Batch 129
    # retires Cruelty after its exact Vulnerable-multiplier hook lands;
    # Batch 130 retires Corruption after both owned-Skill hooks land;
    # Batch 131 retires Vicious after its exact amount-change hook lands;
    # Batch 134 retires Hellraiser after its exact Early hook lands; Batch
    # 144 retires Calamity after its exact generation power lands; Batch 147
    # retires Entropy after its exact turn-start transform lands; Batch 149
    # retires Debilitate, Knockdown, and Tracking; Batch 151 retires Arsenal;
    # Batch 152 retires Rocket Punch; Batch 156 retires Strangle; Batch 162
    # retires Stratagem after its exact AfterShuffle selection lands; Batch
    # 169 retires Underworld through its exact teammate boundary; Batch 222
    # retires Hello World and Sentry Mode after their exact BeforeHandDraw
    # generation continuations land; Batch 223 retires Pillar of Creation and
    # Smokestack after their exact generated-card listeners land; Batch 224
    # retires Shroud and Sleight of Flesh after their exact shared
    # AfterPowerAmountChanged listener walk lands; Batch 226 retires Pagestorm
    # and Iteration after the recursive AfterCardDrawn cursor lands; Batch 227
    # retires Trash to Treasure after its random-Channel continuation lands;
    # Batch 231 retires Creative AI after its recursive serial generation;
    # Batch 244 retires Calcify, Haunt, and Lethality after their exact
    # damage hooks land; Batch 245 retires Serpent Form and Shadowmeld after
    # their exact pending-damage and block-multiplier hooks land; Batch 246
    # retires Tag Team after its exact enemy play-count hook lands; Batch 254
    # retires Spectrum Shift after its exact Regent owner-pool closure; Batch
    # 267 retires Signal Boost after its exact generic Power replay; Batch 268
    # retires Coordinate and Fade behind exact hand admission; Batch 269
    # retires Blaze behind the same reviewed raw quarantine; Batch 271
    # retires Flanking and Intercept behind exact multiplayer power
    # projections (Intercept remains excluded from raw composition); Batch
    # 277 retires Beacon of Hope behind its exact multiplayer block hook;
    # Batch 290 retires Concoct behind exact selected-player ownership; Batch
    # 299 retires Hammer Time behind exact serial multiplayer Forge fan-out;
    # issue #1751 (PR #1818) retires Kingly Kick and Pinpoint after their
    # exact physical cost listener lifecycles land.
    assert len(ledger["explicit_refusals"]) == 14
