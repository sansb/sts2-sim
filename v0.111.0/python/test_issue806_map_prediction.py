"""Issue #806: exact map-layer Rest and Whetstone predictions."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "tools"))

import map_prediction as prediction  # noqa: E402


def _save(*, deck=None, hp=42, max_hp=87, relics=None, modifiers=None,
          niche=None):
    return {
        "schema_version": 20,
        "modifiers": [] if modifiers is None else modifiers,
        "rng": {"rngs": {"niche": niche or {
            "counter": 23, "s0": 1, "s1": 2, "s2": 3, "s3": 4,
        }}},
        "players": [{
            "current_hp": hp,
            "max_hp": max_hp,
            "deck": [] if deck is None else deck,
            "relics": ([{"id": "RELIC.BURNING_BLOOD"}]
                       if relics is None else relics),
        }],
    }


def _rest(save):
    return prediction.predict_rest(
        save, build=prediction.GAME_BUILD,
        provenance=prediction.REST_PROVENANCE,
        relics_census={
            "RELIC.BURNING_BLOOD": {}, "RELIC.REGAL_PILLOW": {}})


def _whetstone(save, *, census=None, max_upgrades=None):
    return prediction.predict_whetstone(
        save, build=prediction.GAME_BUILD,
        provenance=prediction.WHETSTONE_PROVENANCE,
        cards_census=(
            {"CARD.STRIKE_IRONCLAD": {"type": "attack"}}
            if census is None else census),
        max_upgrades=(
            {"CARD.STRIKE_IRONCLAD": 1}
            if max_upgrades is None else max_upgrades))


def test_current_dll_evidence_and_canonical_metadata_are_exhaustive():
    assert prediction.IL_EVIDENCE == (
        ("HealRestSiteOption.GetBaseHealAmount", 0x1154EA),
        ("HealRestSiteOption.GetHealAmount", 0x11533A),
        ("HealRestSiteOption.ExecuteRestSiteHeal.MoveNext", 0x3D841C),
        ("Creature.HealInternal", 0x11D6DC),
        ("Creature.SetCurrentHpInternal", 0x11D734),
        ("Whetstone.AfterObtained", 0x9DE24),
        ("Whetstone.AfterObtained.predicate", 0x333F98),
        ("ListExtensions.StableShuffle", 0x1131A4),
        ("ListExtensions.UnstableShuffle", 0x1131E4),
    )
    census = json.loads((HERE / "cards_census.json").read_text())
    maximums = json.loads((HERE / "card_templates.json").read_text())[
        "max_upgrade"]
    assert set(census) == set(maximums)
    assert {row["type"] for row in census.values()} == {
        "attack", "skill", "power", "status", "curse", "quest"}
    assert census["CARD.SPOILS_MAP"]["type"] == "quest"
    assert all(type(value) is int and value >= 0
               for value in maximums.values())


def test_rest_decimal_request_truncates_only_at_hp_assignment():
    save = _save(hp=42, max_hp=87)
    before = copy.deepcopy(save)
    result = _rest(save)
    assert result == {
        "build": "v0.111.0",
        "player_index": 0,
        "requested_heal_decimal": "26.1",
        "hp_gained": 26,
        "hp_after": 68,
    }
    assert save == before


@pytest.mark.parametrize(
    ("hp", "max_hp", "requested", "gained", "after"),
    ((21, 80, "24.0", 24, 45),
     (80, 87, "26.1", 7, 87),
     (1, 1, "0.3", 0, 1)))
def test_rest_clamp_and_fractional_edges(hp, max_hp, requested, gained, after):
    result = _rest(_save(hp=hp, max_hp=max_hp))
    assert (result["requested_heal_decimal"], result["hp_gained"],
            result["hp_after"]) == (requested, gained, after)


def test_rest_refuses_regal_pillow_and_night_terrors_atomically():
    pillow = _save(relics=[{"id": "RELIC.REGAL_PILLOW"}])
    before = copy.deepcopy(pillow)
    with pytest.raises(NotImplementedError, match="Regal Pillow"):
        _rest(pillow)
    assert pillow == before

    terrors = _save(modifiers=["MODIFIER.NIGHT_TERRORS"])
    before = copy.deepcopy(terrors)
    with pytest.raises(NotImplementedError, match="Night Terrors"):
        _rest(terrors)
    assert terrors == before


@pytest.mark.parametrize(
    ("field", "value", "match"),
    (("schema_version", 19, "schema 20"),
     ("schema_version", True, "schema 20")))
def test_rest_refuses_noncanonical_schema(field, value, match):
    save = _save()
    save[field] = value
    with pytest.raises(NotImplementedError, match=match):
        _rest(save)


def test_rest_refuses_unknown_relic_provenance_and_bad_build_or_capture():
    unknown = _save(relics=[{"id": "RELIC.MODDED_PILLOW"}])
    with pytest.raises(NotImplementedError, match="hook provenance"):
        _rest(unknown)
    with pytest.raises(NotImplementedError, match="exact build"):
        prediction.predict_rest(
            _save(), build="v0.110.1",
            provenance=prediction.REST_PROVENANCE,
            relics_census={"RELIC.BURNING_BLOOD": {}})
    with pytest.raises(NotImplementedError, match="pre-rest-choice-save"):
        prediction.predict_rest(
            _save(), build=prediction.GAME_BUILD,
            provenance="historical-run",
            relics_census={"RELIC.BURNING_BLOOD": {}})


def test_whetstone_preserves_exact_identity_through_dotnet_sort_and_shuffle():
    # Seventeen equal CardModel.CompareTo keys cross .NET's insertion-sort
    # threshold.  Its introsort tie permutation is
    # [0,14,13,12,11,10,9,15,8,6,5,4,3,2,1,7,16], not Python's stable order;
    # Niche state (1,2,3,4) then selects serialized physical rows 9 and 15.
    deck = [
        {"id": "CARD.STRIKE_IRONCLAD", "props": {"token": index}}
        for index in range(17)]
    save = _save(deck=deck)
    before = copy.deepcopy(save)
    result = _whetstone(save)
    assert result["eligible_count"] == 17
    assert result["niche_draws"] == 16
    assert result["niche_counter_before"] == 23
    assert result["niche_counter_after"] == 39
    assert result["niche_state_after"] == {
        "counter": 39,
        "s0": 3416911464729273103,
        "s1": 8788105025338247321,
        "s2": 6251941031793736903,
        "s3": 14442664351366069179,
    }
    assert result["target_deck_indexes"] == [9, 15]
    assert [row["props"]["token"] for row in result["deck_after"]] == \
        list(range(17))
    assert [index for index, row in enumerate(result["deck_after"])
            if row.get("current_upgrade_level") == 1] == [9, 15]
    assert save == before


def test_whetstone_filters_attack_and_is_upgradable_before_full_shuffle():
    deck = [
        {"id": "CARD.STRIKE_IRONCLAD"},
        {"id": "CARD.DEFEND_IRONCLAD"},
        {"id": "CARD.BASH", "current_upgrade_level": 1},
        {"id": "CARD.SPOILS_MAP"},
    ]
    result = _whetstone(
        _save(deck=deck),
        census={
            "CARD.STRIKE_IRONCLAD": {"type": "attack"},
            "CARD.DEFEND_IRONCLAD": {"type": "skill"},
            "CARD.BASH": {"type": "attack"},
            "CARD.SPOILS_MAP": {"type": "quest"},
        },
        max_upgrades={
            "CARD.STRIKE_IRONCLAD": 1,
            "CARD.DEFEND_IRONCLAD": 1,
            "CARD.BASH": 1,
            # Keep this artificially upgradable so CardType alone proves the
            # authoritative Quest row remains Whetstone-ineligible.
            "CARD.SPOILS_MAP": 1,
        })
    assert result["eligible_count"] == 1
    assert result["niche_draws"] == 0
    assert result["target_deck_indexes"] == [0]
    assert result["deck_after"][1:] == deck[1:]


def test_whetstone_takes_two_only_after_consuming_complete_pool_schedule():
    deck = [
        {"id": "CARD.STRIKE_IRONCLAD", "props": {"token": index}}
        for index in range(4)]
    result = _whetstone(_save(deck=deck))
    assert result["eligible_count"] == 4
    assert len(result["targets"]) == 2
    assert result["niche_draws"] == 3
    assert result["niche_counter_after"] - result["niche_counter_before"] == 3


@pytest.mark.parametrize(
    "mutation",
    (lambda save: save.update(schema_version=19),
     lambda save: save.update(rng=[]),
     lambda save: save["rng"].update(rngs=[]),
     lambda save: save["rng"]["rngs"].pop("niche"),
     lambda save: save["rng"]["rngs"]["niche"].update(extra=0),
     lambda save: save["rng"]["rngs"]["niche"].update(counter=True),
     lambda save: save["rng"]["rngs"]["niche"].update(s0=-1),
     lambda save: save["players"][0]["deck"][0].update(upgrade_level=0)))
def test_whetstone_refuses_forged_boundary_without_mutation(mutation):
    save = _save(deck=[{"id": "CARD.STRIKE_IRONCLAD"}])
    mutation(save)
    before = copy.deepcopy(save)
    with pytest.raises(NotImplementedError):
        _whetstone(save)
    assert save == before


def test_whetstone_refuses_unknown_card_metadata_and_provenance():
    save = _save(deck=[{"id": "CARD.MODDED_ATTACK"}])
    before = copy.deepcopy(save)
    with pytest.raises(NotImplementedError, match="CardType"):
        _whetstone(save)
    assert save == before
    with pytest.raises(NotImplementedError, match="pre-whetstone-pickup-save"):
        prediction.predict_whetstone(
            _save(deck=[]), build=prediction.GAME_BUILD,
            provenance="post-pickup-save", cards_census={}, max_upgrades={})


def test_whetstone_empty_pool_consumes_nothing_and_preserves_deck_bytes():
    deck = [{"id": "CARD.DEFEND_IRONCLAD", "floor_added_to_deck": 1}]
    result = _whetstone(
        _save(deck=deck),
        census={"CARD.DEFEND_IRONCLAD": {"type": "skill"}},
        max_upgrades={"CARD.DEFEND_IRONCLAD": 1})
    assert result["eligible_count"] == 0
    assert result["niche_draws"] == 0
    assert result["target_deck_indexes"] == []
    assert result["deck_after"] == deck
