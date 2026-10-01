"""Batch 165 / #718 refusal provenance for four Silent cards.

Historical native anchors (v0.109.1 / c8c577f6, ARM64 sts2.dll SHA-256
2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f):

- BladeSymphony/<OnPlay>d__7::MoveNext 0x3d608c enumerates alive player
  teammates through GetTeammatesOf at IL_00ad-00e2, then creates two L0
  Shivs for each teammate at IL_0122-021d. The solo State has no teammate
  piles or multiplayer action lifecycle; this remains routed to #431.
- KnifeTrap/<OnPlay>d__6::MoveNext 0x3f1b74 snapshots all Shiv-tag cards
  in Exhaust at IL_0032-006c, upgrades each exact instance when the source
  is upgraded at IL_0091-009c, and serially direct-AutoPlays the frozen
  target-bound list at IL_00a1-0124. Batch 168 admits that exact body while
  preserving the translator refusal as provenance.
- ShadowStep was rechecked by Batch 235 against v0.110.1/db5d3552. Its
  `<OnPlay>d__3::MoveNext` current 0x3b818c discards the complete live
  Hand at IL_0024-0090, then applies ShadowStepPower at IL_0095-010f.
  ShadowStepPower/<AfterSideTurnStart>d__4::MoveNext current 0x341d98 applies
  DoubleDamagePower and removes itself. DoubleDamagePower 0xa53bc doubles
  owner/pet powered attacks, and its side-end body 0x3373e8 decrements it.
  Batch 235 admits this exact body while retaining the translator refusal.
- TheHunt/<OnPlay>d__9::MoveNext 0x40bd44 snapshots the target's complete
  ShouldOwnerDeathTriggerFatal conjunction, performs its 10/15 attack, and
  on WasTargetKilled adds a CardReward to CombatRoom.ExtraRewards before
  applying TheHuntPower (IL_0145-0254). Combat-created reward mutation is
  outside the represented combat state.

The raw translator stops earlier than these semantic boundaries. These
tests preserve both that generated provenance and the reviewed I5 decision:
the raw failures stay visible even after reviewed hand models supersede them.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import json
from pathlib import Path


HERE = Path(__file__).parent
CARD_IDS = (
    "CARD.BLADE_SYMPHONY",
    "CARD.KNIFE_TRAP",
    "CARD.SHADOW_STEP",
    "CARD.THE_HUNT",
)
TARGETING = {
    "CARD.BLADE_SYMPHONY": (
        "0xdca32", "AllAllies", "MultiplayerOnly"),
    "CARD.KNIFE_TRAP": ("0xe75e8", "AnyEnemy", "None"),
    "CARD.SHADOW_STEP": ("0xee63f", "Self", "None"),
    "CARD.THE_HUNT": ("0xf217c", "AnyEnemy", "None"),
}
CALLS = {
    "CARD.BLADE_SYMPHONY": {
        "ICombatState::GetTeammatesOf", "Shiv::CreateInHand"},
    "CARD.KNIFE_TRAP": {
        "CardPile::get_Cards", "CardModel::get_Tags",
        "CardCmd::Upgrade", "CardCmd::AutoPlay"},
    "CARD.SHADOW_STEP": {
        "CardPile::get_Cards", "CardCmd::Discard",
        "PowerCmd::Apply<ShadowStepPower>"},
    "CARD.THE_HUNT": {
        "PowerModel::ShouldOwnerDeathTriggerFatal",
        "DamageResult::get_WasTargetKilled",
        "CombatRoom::AddExtraReward", "PowerCmd::Apply<TheHuntPower>"},
}


def _load(name):
    return json.loads((HERE / name).read_text())


def test_batch165_build_and_silent_pool_rows_are_pinned():
    census = _load("card_pool_census.json")
    assert (census["game_build"]["version"],
            census["game_build"]["commit"]) == ("v0.111.0", "41cef1ea")
    rows = {
        row["id"]: row for row in census["pools"]["SILENT"]["cards"]
        if row["id"] in CARD_IDS
    }
    assert set(rows) == set(CARD_IDS)
    assert {
        cid: (
            row["type"], row["rarity"], row["multiplayer_constraint"],
            row["can_be_generated_in_combat"])
        for cid, row in rows.items()
    } == {
        "CARD.BLADE_SYMPHONY":
            ("Skill", "Uncommon", "MultiplayerOnly", True),
        "CARD.KNIFE_TRAP": ("Skill", "Rare", "None", True),
        "CARD.SHADOW_STEP": ("Skill", "Rare", "None", True),
        "CARD.THE_HUNT": ("Attack", "Rare", "None", False),
    }


def test_batch165_targeting_and_current_body_callsets_stay_visible():
    raw = _load("card_templates_raw.json")
    census = _load("template_census.json")

    for cid, (ctor, target, constraint) in TARGETING.items():
        metadata = raw["targeting"][cid]
        assert metadata["constructor_rva"] == ctor
        assert metadata["effective_target_type"]["name"] == target
        assert metadata["multiplayer_constraint"]["name"] == constraint
        assert raw["max_upgrade"][cid] == 1
        assert CALLS[cid] <= set(census[cid]["calls"])


def test_batch165_the_hunt_stays_out_of_combat_generation():
    excluded = {
        row["id"]: row for row in
        _load("card_pool_census.json")["splash_from_solo_ironclad"][
            "excluded_attacks"]
    }
    assert excluded["CARD.THE_HUNT"]["reasons"] == [
        "CanBeGeneratedInCombat=false"]
