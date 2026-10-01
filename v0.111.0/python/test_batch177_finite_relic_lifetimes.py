"""#138 Batch 177 / walk 20: exact finite relic lifetimes.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import copy
from pathlib import Path

from mcr_parser import decode
from relay_parser import (
    FightState, RunSummary, _date_cross_combat_relic_state,
)
from tools.live_coach import build_entry


HERE = Path(__file__).parent
BONE = "RELIC.BONE_TEA"
EMBER = "RELIC.EMBER_TEA"
REAL_TEA = "RELIC.VENERABLE_TEA_SET"
FAKE_TEA = "RELIC.FAKE_VENERABLE_TEA_SET"
PUMPKIN = "RELIC.PUMPKIN_CANDLE"
DISCOURTESY = "RELIC.TEA_OF_DISCOURTESY"
RELICS = {BONE, EMBER, FAKE_TEA, PUMPKIN, DISCOURTESY}


def _fight(node, relics=RELICS):
    return FightState(
        node_index=node, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1,
        gold_entering=0, deck_entering=[], relics_entering=list(relics),
        potions_entering=[], damage_taken=0, hp_healed=0,
        turns_taken=1, potions_used=[], hp_after=1)


def _node(kind, *choices, event_combat=False):
    rooms = [{"room_type": kind}]
    if event_combat:
        rooms = [
            {"room_type": "event"},
            {"room_type": "monster", "turns_taken": 1},
        ]
    elif kind in ("monster", "elite", "boss"):
        rooms[0]["turns_taken"] = 1
    return {
        "map_point_type": "unknown" if event_combat else kind,
        "rooms": rooms,
        "player_stats": [{"rest_site_choices": list(choices)}],
    }


def _live_save(relics, *, deck=None, potions=None):
    return {
        "visited_map_coords": [[0, 0]],
        "ascension": 10,  # RunState.AscensionLevel (#2539)
        "players": [{
            "deck": list(deck or [{
                "id": "CARD.BASH", "current_upgrade_level": 0}]),
            "relics": list(relics),
            "potions": list(potions or []),
            "current_hp": 50, "max_hp": 50, "gold": 0,
        }],
    }


def test_relay_reconstructs_consumption_recharge_and_noncombat_noops():
    # Starting relics: five owned combats exhaust Ember and Pumpkin. A later
    # KINDLE revives Pumpkin to five; the next fight consumes one. Fake Tea
    # is charged entering that next fight and cleared entering the following.
    history = [
        _node("ancient"),
        _node("monster"), _node("monster"), _node("monster"),
        _node("monster"), _node("monster"), _node("monster"),
        _node("rest_site", "KINDLE"),
        _node("monster"), _node("monster"),
    ]
    summary = RunSummary(
        "S", "b", 1, "c", 0, True, None,
        fights=[_fight(1), _fight(6), _fight(8), _fight(9)])
    player = {
        "relics": [
            {"id": rid, "floor_added_to_deck": 1} for rid in RELICS]}
    _date_cross_combat_relic_state(summary, history, player)
    f1, f6, f8, f9 = summary.fights
    assert [f1.relic_counters[r] for r in (BONE, EMBER, DISCOURTESY)] \
        == [1, 5, 1]
    assert [f6.relic_counters[r] for r in (BONE, EMBER, DISCOURTESY)] \
        == [0, 0, 0]
    assert [f.relic_counters[PUMPKIN] for f in (f1, f6, f8, f9)] \
        == [5, 0, 5, 4]
    assert f8.fake_tea_set_charged is True
    assert f9.fake_tea_set_charged is False


def test_event_acquisition_ambiguity_refuses_until_candidates_converge():
    history = [
        _node("ancient"),
        _node("event", event_combat=True),
        _node("monster"), _node("monster"), _node("monster"),
        _node("monster"), _node("monster"), _node("monster"),
    ]
    player = {
        "relics": [
            {"id": rid, "floor_added_to_deck": 2}
            for rid in (BONE, EMBER, PUMPKIN, DISCOURTESY)]}
    early = _fight(2, {BONE, EMBER, PUMPKIN, DISCOURTESY})
    late = _fight(7, {BONE, EMBER, PUMPKIN, DISCOURTESY})
    summary = RunSummary(
        "S", "b", 1, "c", 0, True, None, fights=[early, late])
    _date_cross_combat_relic_state(summary, history, player)
    assert not ({BONE, EMBER, PUMPKIN, DISCOURTESY}
                & set(early.relic_counters))
    assert late.relic_counters[BONE] == 0
    assert late.relic_counters[EMBER] == 0
    assert late.relic_counters[DISCOURTESY] == 0
    assert late.relic_counters[PUMPKIN] == 0


def test_live_save_exact_typed_properties_seed_every_entry_field():
    relics = [
        {"id": BONE, "props": {
            "ints": [{"name": "CombatsLeft", "value": 1}]}},
        {"id": EMBER, "props": {
            "ints": [{"name": "CombatsLeft", "value": 4}]}},
        {"id": PUMPKIN, "props": {
            "ints": [{"name": "KindleCount", "value": 9}]}},
        {"id": DISCOURTESY, "props": {
            "ints": [{"name": "CombatsLeft", "value": 0}]}},
        {"id": REAL_TEA, "props": {"bools": [{
            "name": "GainEnergyInNextCombat", "value": True}]}},
        {"id": FAKE_TEA, "props": {"bools": [{
            "name": "GainEnergyInNextCombat", "value": False}]}},
    ]
    save = {
        "visited_map_coords": [[0, 0]],
        "ascension": 10,  # RunState.AscensionLevel (#2539)
        "players": [{
            "deck": [{
                "id": "CARD.BASH", "current_upgrade_level": 1,
            }],
            "relics": relics, "potions": [],
            "current_hp": 50, "max_hp": 50, "gold": 0,
        }],
    }
    entry = build_entry(save, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert entry["deck_entering"] == [{
        "id": "CARD.BASH", "upgrade_level": 1}]
    assert entry["relic_counters"] == {
        BONE: 1, EMBER: 4, PUMPKIN: 9, DISCOURTESY: 0}
    assert entry["tea_set_charged"] is True
    assert entry["fake_tea_set_charged"] is False


def test_real_decoded_mcr_fixture_normalizes_ids_and_upgraded_card():
    run = decode(HERE / "testdata/latest.mcr")["run"]
    entry = build_entry(run, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert all(row["id"].startswith("CARD.")
               for row in entry["deck_entering"])
    assert all(rid.startswith("RELIC.")
               for rid in entry["relics_entering"])
    assert all(pid.startswith("POTION.")
               for pid in entry["potions_entering"])
    blood_wall = next(
        row for row in entry["deck_entering"]
        if row["id"] == "CARD.BLOOD_WALL")
    assert blood_wall["upgrade_level"] == 1


def test_scoped_grouped_props_allow_omitted_inherited_false_defaults():
    props = {
        "ints": [{"name": "CombatsLeft", "value": 3}],
        "bools": [
            {"name": "IsWax", "value": False},
            {"name": "IsMelted", "value": False},
        ],
    }
    save = _live_save([{"id": EMBER, "props": props}])
    before = copy.deepcopy(save)
    entry = build_entry(save, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert entry["relic_counters"] == {EMBER: 3}
    assert save == before
    # A decoded flat row may legitimately omit both inherited false fields;
    # the adapter accepts that omission without synthesizing either name.
    flat = _live_save([{
        "id": "FAKE_VENERABLE_TEA_SET",
        "props": {"GainEnergyInNextCombat": False},
    }])
    flat_entry = build_entry(
        flat, "ENCOUNTER.TERROR_EEL_ELITE", "elite")
    assert flat_entry["fake_tea_set_charged"] is False
    assert set(flat["players"][0]["relics"][0]["props"]) == {
        "GainEnergyInNextCombat"}
