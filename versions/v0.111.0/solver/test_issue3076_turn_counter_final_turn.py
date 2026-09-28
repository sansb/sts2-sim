"""#3076: a combat won during a player turn start does not tick turn counters.

Every hook walk goes through ``Hook.IterateCombatHookListeners``, which yields
nothing once ``CombatManager.IsOverOrEnding`` (RVA 0x3d3bc0 IL_0028-IL_0042).
Happy Flower ticks in AfterSideTurnStart, dispatched late in
``<StartTurn>d__100`` (IL_07bf); Pendulum and Pollinous Core tick in
BeforeHandDraw inside SetupPlayerTurn. When the last enemy dies during a turn
start, before that walk, ``turns_taken`` (the TurnNumber at combat end) counts
the turn but the relic never sees it. The .run does not record where a combat
ended, so ``relay_parser`` seeds these counters only before the relic's first
owned combat.

The witness tables were read from the uploader's ``.mcr`` captures on Sean's
Mac (2026-09-25), not committed: for every combat after the relic was picked,
the ``.run``'s ``turns_taken``, the capture's entry ``TurnsSeen``, and whether
the capture's last native checkpoint is "After player turn start" with every
enemy dead. The next combat's entry value is the previous one plus the ticks.
"""

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from relay_parser import (  # noqa: E402
    _COUNTER_RELIC_MODS, FightState, RunSummary,
    _date_cross_combat_relic_state,
)

FLOWER = "RELIC.HAPPY_FLOWER"
TURN_COUNTERS = tuple(_COUNTER_RELIC_MODS)

# (floor, map_point_type, turns_taken, TurnsSeen at entry,
#  ended with every enemy dead at the "After player turn start" checkpoint)
# A10 Regent UE7YGG9XC3ZB, Happy Flower picked on floor 20.
UE7YGG9XC3ZB = (
    (21, "unknown", 6, 0, False),     # Mysterious Knight event combat
    (23, "monster", 7, 0, False),
    (24, "elite", 8, 1, True),        # Infested Prisms: 8 turns, 7 ticks
    (27, "monster", 8, 2, False),
    (29, "elite", 6, 1, False),
    (30, "monster", 4, 1, False),
    (31, "monster", 4, 2, False),
    (33, "boss", 6, 0, False),
    (35, "monster", 4, 0, False),
    (36, "monster", 5, 1, False),
    (40, "elite", 3, 0, False),
    (42, "elite", 6, 0, False),
    (43, "monster", 5, 0, False),
    (45, "monster", 4, 2, False),
    (46, "elite", 5, 0, False),
    (48, "boss", 6, 2, False),
    (49, "boss", 8, 2, None),         # final fight: no later entry value
)
# NA4SLXKS9923, Happy Flower picked on floor 27: two more skipped last turns.
NA4SLXKS9923 = (
    (28, "elite", 1, 0, False),
    (30, "monster", 4, 1, True),
    (31, "elite", 5, 1, False),
    (33, "boss", 7, 0, False),
    (35, "monster", 4, 1, False),
    (36, "monster", 2, 2, False),
    (40, "elite", 5, 1, False),
    (43, "monster", 4, 0, False),
    (45, "elite", 4, 1, True),
    (46, "monster", 4, 1, False),
    (48, "boss", 7, 2, False),
    (49, "boss", 5, 0, None),
)


def _replay(table, modulus, *, skip_turn_start_wins):
    """Entry values implied by the table's first entry and turns_taken."""
    value = table[0][3]
    implied = [value]
    for _floor, _kind, turns, _seen, turn_start_win in table[:-1]:
        ticks = turns - (1 if skip_turn_start_wins and turn_start_win else 0)
        value = (value + ticks) % modulus
        implied.append(value)
    return implied


@pytest.mark.parametrize("table", [UE7YGG9XC3ZB, NA4SLXKS9923],
                         ids=["UE7YGG9XC3ZB", "NA4SLXKS9923"])
def test_witness_ticks_every_turn_except_a_turn_start_win(table):
    captured = [row[3] for row in table]
    modulus = _COUNTER_RELIC_MODS[FLOWER]
    assert _replay(table, modulus, skip_turn_start_wins=True) == captured
    # The retired model (one tick per turns_taken) drifts at the first
    # turn-start win and never recovers.
    assert _replay(table, modulus, skip_turn_start_wins=False) != captured


def _fight(node, relics):
    return FightState(
        node_index=node, node_type="monster", encounter_id="E",
        monster_ids=[], hp_entering=1, max_hp_entering=1, gold_entering=0,
        deck_entering=[], relics_entering=list(relics), potions_entering=[],
        damage_taken=0, hp_healed=0, turns_taken=1, potions_used=[],
        hp_after=1)


def _history(combats, length):
    """Combat nodes at floor - 1; every other node a combat-free rest site."""
    history = [{"map_point_type": "rest_site", "rooms": [],
                "player_stats": [{}]} for _ in range(length)]
    for floor, kind, turns in combats:
        room_type = "monster" if kind == "unknown" else kind
        history[floor - 1] = {
            "map_point_type": kind,
            "rooms": [{"room_type": room_type, "turns_taken": turns}],
            "player_stats": [{}]}
    return history


def _summary(fights):
    summary = RunSummary(
        seed="S", build_id="b", schema_version=1, character="c",
        ascension=0, win=True, killed_by=None)
    summary.fights = fights
    return summary


@pytest.mark.parametrize("table,picked", [(UE7YGG9XC3ZB, 20),
                                          (NA4SLXKS9923, 27)],
                         ids=["UE7YGG9XC3ZB", "NA4SLXKS9923"])
def test_parser_never_claims_a_value_the_witness_contradicts(table, picked):
    history = _history([row[:3] for row in table], table[-1][0])
    player = {"relics": [{"id": FLOWER, "floor_added_to_deck": picked}]}
    fights = [_fight(floor - 1, [FLOWER]) for floor, kind, *_ in table
              if kind != "unknown"]
    _date_cross_combat_relic_state(_summary(fights), history, player)
    seen = {floor - 1: row_seen for floor, _k, _t, row_seen, _w in table}
    claimed = {f.node_index: f.relic_counters[FLOWER]
               for f in fights if FLOWER in f.relic_counters}
    # Only a fight with no prior owned combat is claimed, and it is right.
    first_owned_combat = table[0][0] - 1
    assert set(claimed) <= {first_owned_combat}
    assert all(seen[node] == value for node, value in claimed.items())
    for f in fights:
        if f.node_index > first_owned_combat:
            assert any("HAPPY_FLOWER turn counter unseedable after a prior "
                       "owned combat" in c for c in f.caveats)


@pytest.mark.parametrize("relic", TURN_COUNTERS)
def test_first_owned_combat_enters_at_zero(relic):
    # Picked on floor 2, after the floor-1 combat it never saw.
    history = _history([(1, "monster", 5), (3, "monster", 2)], 3)
    player = {"relics": [{"id": relic, "floor_added_to_deck": 2}]}
    fight = _fight(2, [relic])
    _date_cross_combat_relic_state(_summary([fight]), history, player)
    assert fight.relic_counters == {relic: 0}
    assert not fight.caveats


@pytest.mark.parametrize("relic", TURN_COUNTERS)
def test_any_prior_owned_combat_refuses_by_name(relic):
    # One owned 3-turn combat: the relic ticked 3 or 2 times.
    history = _history([(1, "monster", 3), (2, "monster", 1)], 2)
    player = {"relics": [{"id": relic, "floor_added_to_deck": 0}]}
    fight = _fight(1, [relic])
    _date_cross_combat_relic_state(_summary([fight]), history, player)
    assert relic not in fight.relic_counters
    assert fight.caveats == [
        f"{relic} turn counter unseedable after a prior owned combat (a "
        "combat won during a player turn start does not tick; the .run "
        "omits where combat ended)"]


@pytest.mark.parametrize("relic", TURN_COUNTERS)
def test_event_combat_grant_keeps_its_own_refusal(relic):
    # Granted on the event node that hosted the combat: order unknown, and
    # that refusal is named before the owned-combat one.
    history = _history([(1, "unknown", 3), (2, "monster", 1)], 2)
    player = {"relics": [{"id": relic, "floor_added_to_deck": 1}]}
    fight = _fight(1, [relic])
    _date_cross_combat_relic_state(_summary([fight]), history, player)
    assert relic not in fight.relic_counters
    assert fight.caveats == [
        f"{relic} turn counter unseedable (relic obtained at a "
        "combat-bearing event node)"]
