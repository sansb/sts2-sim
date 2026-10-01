"""Batch 56 exact pins for the audited +1 max-energy relic trio.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import copy
import hashlib
import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))

from tools.census_relics import INERT_METHODS  # noqa: E402
from tools.relic_templates import (  # noqa: E402
    IGNORED_METHODS, REVIEWED_NONCOMBAT_METHODS,
)


HERE = pathlib.Path(__file__).parent
EXPECTED_NONCOMBAT = {
    "RELIC.ECTOPLASM": frozenset({
        "ModifyGoldGained", "AfterModifyingGoldGained",
    }),
    "RELIC.PRISMATIC_GEM": frozenset({
        "ModifyCardRewardCreationOptions",
    }),
    "RELIC.SOZU": frozenset({"ShouldProcurePotion"}),
}
BASE_RELIC_TEMPLATES_SHA256 = \
    "d42b9e91083fa4df916a1602f07dc07377994bd1377b5cf022296162644e6041"


def test_reviewed_noncombat_allowlist_is_exact_per_relic_and_census_inert():
    assert REVIEWED_NONCOMBAT_METHODS == EXPECTED_NONCOMBAT
    reviewed = set().union(*REVIEWED_NONCOMBAT_METHODS.values())
    assert reviewed <= INERT_METHODS
    assert reviewed.isdisjoint(IGNORED_METHODS)


def test_checked_template_delta_is_only_the_three_reviewed_rows():
    current = json.loads((HERE / "relic_templates.json").read_text())
    assert len(current["relics"]) == 34
    # Batches 178/179/181/182/183/184/185/186/187/189/190 moved Lizard Tail,
    # four pet relics, Joss Paper, Unceasing Top, Iron Club, Tuning Fork,
    # Claws, Music Box, Regalite, Meat on the Bone, Pael's Eye, Delicate
    # Frond, Petrified Toad, Brilliant Scarf, and Fur Coat into the reviewed
    # hand-model
    # registry. Restore those
    # later deltas before reconstructing Batch56's base.
    assert len(current["refused"]) == 132
    prior_reasons = {
        "RELIC.ECTOPLASM": "unsupported hook ModifyGoldGained",
        "RELIC.PRISMATIC_GEM":
            "unsupported hook ModifyCardRewardCreationOptions",
        "RELIC.SOZU": "unsupported hook ShouldProcurePotion",
    }
    reconstructed_base = copy.deepcopy(current)
    reconstructed_base["refused"]["RELIC.LIZARD_TAIL"] = (
        "unsupported hook ShouldDieLate")
    reconstructed_base["refused"].update({
        "RELIC.PAELS_EYE": "unsupported hook BeforeCardPlayed",
        "RELIC.BRILLIANT_SCARF":
            "unsupported hook TryModifyEnergyCostInCombatLate",
        "RELIC.FUR_COAT": "unsupported hook ModifyGeneratedMapLate",
        "RELIC.BOUND_PHYLACTERY": "call BoundPhylactery::SummonPet",
        "RELIC.BYRDPIP": "call Byrdpip::SummonPet",
        "RELIC.DELICATE_FROND": "call Player::get_HasOpenPotionSlots",
        "RELIC.JOSS_PAPER": "unsupported hook AfterCardExhausted",
        "RELIC.PAELS_LEGION": "call PaelsLegion::SummonPet",
        "RELIC.PHYLACTERY_UNBOUND": "call OstyCmd::Summon",
        "RELIC.UNCEASING_TOP": "unsupported hook AfterHandEmptied",
        "RELIC.IRON_CLUB": "unsupported hook AfterCardPlayed",
        "RELIC.TUNING_FORK": "unsupported hook NotifySkillPlayed",
        "RELIC.CLAWS": "unsupported hook CreateMaulFromOriginal",
        "RELIC.MUSIC_BOX": "unsupported hook BeforeCardPlayed",
        "RELIC.REGALITE":
            "unsupported hook AfterCardGeneratedForCombat",
        "RELIC.MEAT_ON_THE_BONE":
            "call MeatOnTheBone::WillHealOnCombatFinished",
        "RELIC.PETRIFIED_TOAD": "unsupported hook BeforeCombatStartLate",
    })
    for rid, reason in prior_reasons.items():
        reconstructed_base["relics"].pop(rid)
        reconstructed_base["refused"][rid] = reason
    encoded = (json.dumps(reconstructed_base, indent=1, sort_keys=True)
               + "\n").encode()
    assert hashlib.sha256(encoded).hexdigest() == \
        BASE_RELIC_TEMPLATES_SHA256
