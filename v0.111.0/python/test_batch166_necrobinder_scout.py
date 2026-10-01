"""Batch 166 / #730 current-build refusal pins for the Necrobinder slice.

Complete outer methods and nested async bodies were re-read from the installed
v0.109.1/c8c577f6 ARM64 sts2.dll (sha256
2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f):

* Sacrifice 0x295fda; <OnPlay>d__9::MoveNext 0x4011b8
* Seance 0x296517; <OnPlay>d__7::MoveNext 0x402104
* Soulbound 0x297d26; <OnPlay>d__5::MoveNext 0x4061f0
* Transfigure 0x29b155; <OnPlay>d__9::MoveNext 0x40d56c

All four needed engine work outside this Necrobinder-shard-only track. These
raw provenance pins remain after later exact admissions.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib


HERE = pathlib.Path(__file__).parent
IDS = (
    "CARD.SACRIFICE",
    "CARD.SEANCE",
    "CARD.SOULBOUND",
    "CARD.TRANSFIGURE",
)


def _load(name):
    return json.loads((HERE / name).read_text())


def test_batch166_necrobinder_refusals_pin_complete_body_consumers():
    census = _load("template_census.json")
    expected_calls = {
        # Freeze block at 2 * live Osty MaxHp, kill Osty, then gain the
        # powered card-sourced frozen amount. Missing Osty returns up front.
        "CARD.SACRIFICE": {
            "Osty::CheckMissingWithAnim",
            "Creature::get_MaxHp",
            "CalculatedVar::Calculate",
            "CreatureCmd::Kill",
            "CreatureCmd::GainBlock",
        },
        # Select one exact pile card, materialize the selection order, then
        # serially transform the selected instance to Soul.
        "CARD.SEANCE": {
            "CardSelectCmd::FromCombatPile",
            "Enumerable::ToList<CardModel>",
            "CardCmd::TransformTo<Soul>",
        },
        # Multiplayer AnyAlly target: preserve the selected creature and
        # owner/applier identities when applying the non-inert power.
        "CARD.SOULBOUND": {
            "CardPlay::get_Target",
            "Creature::get_Player",
            "PowerCmd::Apply<SoulboundPower>",
        },
        # Select one exact Hand instance, conditionally add +1 combat cost,
        # and always increment that same instance's BaseReplayCount.
        "CARD.TRANSFIGURE": {
            "CardSelectCmd::FromHand",
            "CardEnergyCost::get_CostsX",
            "CardEnergyCost::GetWithModifiers",
            "CardEnergyCost::AddThisCombat",
            "CardModel::get_BaseReplayCount",
            "CardModel::set_BaseReplayCount",
        },
    }
    for cid, calls in expected_calls.items():
        assert calls <= set(census[cid]["calls"])


def test_batch166_necrobinder_target_and_keyword_boundaries():
    raw = _load("card_templates_raw.json")
    targeting = raw["targeting"]
    for cid in ("CARD.SACRIFICE", "CARD.SEANCE", "CARD.TRANSFIGURE"):
        assert targeting[cid]["effective_target_type"]["name"] == "Self"
        assert targeting[cid]["multiplayer_constraint"]["name"] == "None"
    assert targeting["CARD.SOULBOUND"]["effective_target_type"]["name"] == \
        "AnyAlly"
    assert targeting["CARD.SOULBOUND"]["multiplayer_constraint"]["name"] == \
        "MultiplayerOnly"

    cards = _load("cards_census.json")
    assert cards["CARD.SACRIFICE"]["keywords"] == ["Retain"]
    assert cards["CARD.SEANCE"]["keywords"] == ["Ethereal"]
    assert cards["CARD.SOULBOUND"].get("keywords", []) == []
    assert cards["CARD.TRANSFIGURE"]["keywords"] == ["Exhaust"]


def test_batch166_necrobinder_refusal_build_and_pool_provenance():
    pools = _load("card_pool_census.json")
    assert (pools["game_build"]["version"],
            pools["game_build"]["commit"]) == ("v0.111.0", "41cef1ea")
    necrobinder = {
        entry["id"] for entry in pools["pools"]["NECROBINDER"]["cards"]}
    assert set(IDS) <= necrobinder
