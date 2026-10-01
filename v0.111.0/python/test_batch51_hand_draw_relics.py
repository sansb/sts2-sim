"""Batch 51 exact pins for hand-draw relics.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from relay_parser import (  # noqa: E402
    FightState, RunSummary, _date_cross_combat_relic_state,
)


def test_parser_seeds_turn_counters_only_before_the_first_owned_combat():
    def fight(node, relics):
        return FightState(
            node_index=node, node_type="monster", encounter_id="E",
            monster_ids=[], hp_entering=1, max_hp_entering=1,
            gold_entering=0, deck_entering=[], relics_entering=relics,
            potions_entering=[], damage_taken=0, hp_healed=0,
            turns_taken=1, potions_used=[], hp_after=1)

    def node(turns):
        return {
            "map_point_type": "monster",
            "rooms": [{"room_type": "monster", "turns_taken": turns}],
            "player_stats": [{}],
        }

    relics = ["RELIC.HAPPY_FLOWER", "RELIC.PENDULUM",
              "RELIC.POLLINOUS_CORE"]
    player = {"relics": [
        {"id": relic, "floor_added_to_deck": 0} for relic in relics]}
    summary = RunSummary(
        seed="S", build_id="b", schema_version=1, character="c",
        ascension=0, win=True, killed_by=None)
    # #3076: before the relics' first owned combat each enters at 0; after
    # one, the last turn's tick is unknown and every counter refuses by name.
    summary.fights = [fight(0, relics), fight(2, relics)]
    _date_cross_combat_relic_state(summary, [node(3), node(4), node(1)],
                                   player)
    first, later = summary.fights
    assert first.relic_counters == {rid: 0 for rid in relics}
    assert later.relic_counters == {}
    for rid in relics:
        assert any(f"{rid} turn counter unseedable after a prior owned "
                   "combat" in c for c in later.caveats)


@pytest.mark.parametrize("missing_turns", [True, False])
def test_parser_refuses_pollinous_missing_or_acquisition_ambiguous_seed(
        missing_turns):
    relic = "RELIC.POLLINOUS_CORE"
    fight = FightState(
        node_index=1, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1, gold_entering=0,
        deck_entering=[], relics_entering=[relic], potions_entering=[],
        damage_taken=0, hp_healed=0, turns_taken=1, potions_used=[],
        hp_after=1)
    summary = RunSummary(
        seed="S", build_id="b", schema_version=1, character="c",
        ascension=0, win=True, killed_by=None)
    summary.fights = [fight]
    turns = None if missing_turns else 2
    prior_type = "monster" if missing_turns else "unknown"
    history = [
        {"map_point_type": prior_type,
         "rooms": [{"room_type": "monster", "turns_taken": turns}],
         "player_stats": [{}]},
        {"map_point_type": "monster",
         "rooms": [{"room_type": "monster", "turns_taken": 1}],
         "player_stats": [{}]},
    ]
    # Missing-history case: owned before the prior combat. Ambiguous case:
    # obtained on the same event node that hosted the prior combat.
    floor = 0 if missing_turns else 1
    player = {"relics": [{"id": relic, "floor_added_to_deck": floor}]}
    _date_cross_combat_relic_state(summary, history, player)
    assert relic not in fight.relic_counters
    assert any("turn counter unseedable" in caveat
               for caveat in fight.caveats)
