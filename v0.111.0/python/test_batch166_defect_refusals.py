"""Batch 166 / #731 current-build refusal pins for the Defect slice.

The complete card bodies and dependent power hooks were re-read from the
installed v0.109.1/c8c577f6 ARM64 sts2.dll (sha256
2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f):

* Chaos/<OnPlay>d__5::MoveNext 0x3dabec
* ConsumingShadow/<OnPlay>d__5::MoveNext 0x3dd6e8 and
  ConsumingShadowPower/<AfterSideTurnEnd>d__4::MoveNext 0x380b88
* Ignition/<OnPlay>d__7::MoveNext 0x3efc10
* ImitationLearning/<OnPlay>d__7::MoveNext 0x3efdac,
  ImitationLearningPower::BeforeCardPlayed 0x24fce4, and
  ImitationLearningPower/<AfterCardPlayed>d__16::MoveNext 0x38645c

All four cross engine or multiplayer-state boundaries outside this
content-only track. These pins prevent partial fixed-orb, bare-channel, or
local-player approximations from silently entering the executable table.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path


HERE = Path(__file__).parent
CARD_IDS = (
    "CARD.CHAOS",
    "CARD.CONSUMING_SHADOW",
    "CARD.IGNITION",
    "CARD.IMITATION_LEARNING",
)


def _load(name):
    return json.loads((HERE / name).read_text())


def test_batch166_build_and_defect_pool_rows_are_pinned():
    census = _load("card_pool_census.json")
    assert (census["game_build"]["version"],
            census["game_build"]["commit"]) == ("v0.111.0", "41cef1ea")
    rows = {
        row["id"]: row for row in census["pools"]["DEFECT"]["cards"]
        if row["id"] in CARD_IDS
    }
    assert set(rows) == set(CARD_IDS)
    assert {
        cid: (
            row["type"], row["rarity"], row["multiplayer_constraint"],
            row["can_be_generated_in_combat"])
        for cid, row in rows.items()
    } == {
        "CARD.CHAOS": ("Skill", "Uncommon", "None", True),
        "CARD.CONSUMING_SHADOW": ("Power", "Rare", "None", True),
        "CARD.IGNITION": ("Skill", "Uncommon", "MultiplayerOnly", True),
        "CARD.IMITATION_LEARNING":
            ("Skill", "Rare", "MultiplayerOnly", True),
    }


def test_batch166_nested_consumers_and_multiplayer_targets_stay_visible():
    census = _load("template_census.json")
    expected_calls = {
        "CARD.CHAOS": {
            "RunRngSet::get_CombatOrbGeneration",
            "OrbModel::GetRandomOrb",
            "OrbCmd::Channel",
        },
        "CARD.CONSUMING_SHADOW": {
            "OrbCmd::Channel<DarkOrb>",
            "PowerCmd::Apply<ConsumingShadowPower>",
        },
        "CARD.IGNITION": {
            "CardPlay::get_Target",
            "Creature::get_Player",
            "OrbCmd::Channel<PlasmaOrb>",
        },
        "CARD.IMITATION_LEARNING": {
            "CardPlay::get_Target",
            "Creature::get_Powers",
            "Enumerable::OfType<ImitationLearningPower>",
            "ImitationLearningPower::set_PlayerTarget",
            "PowerCmd::Apply<ImitationLearningPower>",
        },
    }
    for cid, calls in expected_calls.items():
        assert calls <= set(census[cid]["calls"])

    targeting = _load("card_templates_raw.json")["targeting"]
    for cid in ("CARD.CHAOS", "CARD.CONSUMING_SHADOW"):
        assert targeting[cid]["effective_target_type"]["name"] == "Self"
        assert targeting[cid]["multiplayer_constraint"]["name"] == "None"
    for cid in ("CARD.IGNITION", "CARD.IMITATION_LEARNING"):
        assert targeting[cid]["effective_target_type"]["name"] == "AnyAlly"
        assert targeting[cid]["multiplayer_constraint"]["name"] == \
            "MultiplayerOnly"
