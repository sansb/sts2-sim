"""Batch 165 / #719 current-build pins for the original Defect slice.

Nested async bodies were re-read from the installed v0.109.1/c8c577f6
ARM64 sts2.dll (sha256 2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f):

* Compact/<OnPlay>d__7::MoveNext 0x3dc9b8
* GeneticAlgorithm/<OnPlay>d__16::MoveNext 0x3ea3ac
* Hibernate/<OnPlay>d__7::MoveNext 0x3ee334 and
  HibernatePower/<AfterPlayerTurnStart>d__6::MoveNext 0x385d60
* OneForAll/<OnPlay>d__5::MoveNext 0x3f952c and
  OneForAllPower::ModifyDamageAdditive 0x2512e4

Compact was retired from refusal by Batch 232 after the required bulk
Transform engine work. Genetic Algorithm was retired by Batch 233 after its
physical-card/master-deck growth relation was modeled. One for All was retired
by Batch 276 after its multiplayer amount and zero-energy modifier were
modeled. Hibernate was retired by Batch 278 after its exact additive duration,
turn-start decrement, source authentication, and Frost-channel body landed.
These pins still make a partial approximation fail loudly.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
import pathlib


HERE = pathlib.Path(__file__).parent


def _load(name):
    return json.loads((HERE / name).read_text())


def test_batch165_defect_refusals_keep_their_exact_consumer_metadata():
    census = _load("template_census.json")
    expected_calls = {
        # Block 6/7, then snapshot every transformable Status in Discard and
        # transform each exact instance to a fresh Fuel (upgraded iff source).
        # Gain the physical instance's CurrentBlock, then increase both that
        # combat copy and its exact master-deck DeckVersion by 3/4.
        "CARD.GENETIC_ALGORITHM": {
            "CardModel::get_DeckVersion",
            "GeneticAlgorithm::BuffFromPlay",
        },
        # Multiplayer-only Self card: apply a duration tied to this exact
        # CardPlay.Card, then serially channel 2/3 Frost orbs.
        "CARD.HIBERNATE": {
            "CardPlay::get_Card", "OrbCmd::Channel<FrostOrb>",
            "PowerCmd::Apply<HibernatePower>",
        },
        # Multiplayer-only AllAllies power: enumerate every player and apply
        # the owner-keyed zero-energy powered-attack modifier.
        "CARD.ONE_FOR_ALL": {
            "ICombatState::get_Players",
            "PowerCmd::Apply<OneForAllPower>",
        },
    }
    for cid, calls in expected_calls.items():
        assert calls <= set(census[cid]["calls"])

    assert {
        "CardPile::get_Cards", "CardCmd::Transform",
        "ICombatState::CreateCard<Fuel>",
    } <= set(census["CARD.COMPACT"]["calls"])

    targeting = _load("card_templates_raw.json")["targeting"]
    assert targeting["CARD.COMPACT"]["effective_target_type"]["name"] == \
        "Self"
    assert targeting["CARD.GENETIC_ALGORITHM"][
        "effective_target_type"]["name"] == "Self"
    assert targeting["CARD.HIBERNATE"]["multiplayer_constraint"]["name"] == \
        "MultiplayerOnly"
    assert targeting["CARD.ONE_FOR_ALL"]["effective_target_type"]["name"] == \
        "AllAllies"
    assert targeting["CARD.ONE_FOR_ALL"][
        "multiplayer_constraint"]["name"] == "MultiplayerOnly"
