"""Batch 165 / #720 Regent-slice refusal and retirement pins.

Nested async bodies were re-read from the installed v0.109.1/c8c577f6
ARM64 sts2.dll (sha256 2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f):

* Charge/<OnPlay>d__5::MoveNext 0x3dada4
* Glimmer/<OnPlay>d__4::MoveNext 0x3ea9d0
* Guards/<OnPlay>d__5::MoveNext 0x3ebc7c
* Largesse/<OnPlay>d__3::MoveNext 0x3f233c

Those addresses remain the original Batch165 discovery record. Charge and
Guards were subsequently hand-modeled, and Batch234 re-read Glimmer against
installed/archive-identical v0.110.1/db5d3552 at nested MoveNext 0x39f068.
Batch288 subsequently models Largesse's exact target-pool/source-owner split.
These pins prevent partial draw, transform, or local-player approximations
while checking each deliberate retirement.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib


HERE = pathlib.Path(__file__).parent
IDS = (
    "CARD.CHARGE",
    "CARD.GLIMMER",
    "CARD.GUARDS",
    "CARD.LARGESSE",
)


def _load(name):
    return json.loads((HERE / name).read_text())


def test_batch165_regent_refusals_keep_their_exact_consumer_metadata():
    census = _load("template_census.json")
    expected_calls = {
        # Select two exact Draw cards, then serially transform them to
        # MinionDiveBomb and upgrade each result iff Charge is upgraded.
        "CARD.CHARGE": {
            "CardSelectCmd::FromCombatPile",
            "CardCmd::TransformTo<MinionDiveBomb>",
            "CardCmd::Upgrade",
        },
        # Draw 3/4, then select one from the resulting live Hand and move that
        # exact instance to Draw/Top.
        "CARD.GLIMMER": {
            "CardPileCmd::Draw", "CardSelectCmd::FromHand",
            "CardPileCmd::Add",
        },
        # Select 0..all exact Hand cards and serially transform each to a
        # MinionSacrifice upgraded iff Guards is upgraded.
        "CARD.GUARDS": {
            "CardSelectCmd::FromHand",
            "ICombatState::CreateCard<MinionSacrifice>",
            "CardCmd::Transform",
        },
        # Multiplayer AnyAlly: target supplies unlock/constraint filtering,
        # owner supplies CombatCardGeneration RNG and receives the result.
        "CARD.LARGESSE": {
            "CardPoolModel::GetUnlockedCards",
            "CardFactory::GetDistinctForCombat",
            "RunRngSet::get_CombatCardGeneration",
            "CardPileCmd::AddGeneratedCardToCombat",
        },
    }
    for cid, calls in expected_calls.items():
        assert calls <= set(census[cid]["calls"])

    targeting = _load("card_templates_raw.json")["targeting"]
    for cid in ("CARD.CHARGE", "CARD.GLIMMER", "CARD.GUARDS"):
        assert targeting[cid]["effective_target_type"]["name"] == "Self"
        assert targeting[cid]["multiplayer_constraint"]["name"] == "None"
    assert targeting["CARD.LARGESSE"]["effective_target_type"]["name"] == \
        "AnyAlly"
    assert targeting["CARD.LARGESSE"]["multiplayer_constraint"]["name"] == \
        "MultiplayerOnly"


def test_batch165_regent_refusal_build_and_pool_provenance():
    pools = _load("card_pool_census.json")
    assert (pools["game_build"]["version"],
            pools["game_build"]["commit"]) == ("v0.111.0", "41cef1ea")
    regent = {
        entry["id"] for entry in pools["pools"]["REGENT"]["cards"]}
    assert set(IDS) <= regent
