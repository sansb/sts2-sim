"""
Run-parser, census and `dotnet_sort` pins that outlived the Python simulator.

This module was the Python simulator's test file (issue #55) until #2827 item
F deleted the simulator. What remains never reached it: the `.run` parser's
card/relic dating and counter seeding, the replay counter walk, the IL
censuses' self-consistency, the relic-template translator's frozen output,
and the hand-traced `dotnet_list_sort` pins (SOLVER_INVARIANTS.md I1, I6).

Run: solver/tools/pytest_lane.sh -- sim/v0.111.0/python/test_run_parser_contracts.py
"""

import json
import pathlib
import sys


sys.path.insert(0, str(pathlib.Path(__file__).parent))


POTIONS_CENSUS = json.load(open(pathlib.Path(__file__).parent /
                                "potions_census.json"))


def test_potion_tooltips_resolve_against_census():
    """D4 tooltip cross-check, recomputed from the checked-in JSON: every
    {Placeholder} in a potion's English tooltip must be one of that
    potion's IL CanonicalVars (the tooltip renders census values, so an
    unresolved placeholder means the census missed a var). energyPrefix
    is the tooltip engine's display pseudo-var."""
    import re
    for pid, info in POTIONS_CENSUS.items():
        for ph in re.findall(r"\{([A-Za-z][A-Za-z0-9]*)",
                             info["tooltip_eng"] or ""):
            assert ph in info["vars"] or ph == "energyPrefix", \
                f"{pid}: tooltip {{{ph}}} has no IL-census value"


def test_relic_templates_agree_with_hand_models():
    """The relic hook translator must reproduce every hand-modeled relic
    it covers exactly — hook, guards, and resolved amounts. Verified
    entry-by-entry against the hand models when frozen (2026-07-11)."""
    data = json.load(open(pathlib.Path(__file__).parent /
                          "relic_templates.json"))["relics"]
    expected = {
        "RELIC.ANCHOR": [("BeforeCombatStart", [], [["block", 10]])],
        "RELIC.LANTERN": [("AfterSideTurnStart",
                           [["own_side"], ["turn", "le", 1]],
                           [["energy", 1]])],
        "RELIC.BLOOD_VIAL": [("AfterPlayerTurnStartLate",
                              [["is_owner"], ["turn", "le", 1]],
                              [["heal", 2]])],
        "RELIC.FESTIVE_POPPER": [("AfterPlayerTurnStart",
                                  [["is_owner"], ["turn", "eq", 1]],
                                  [["damage_all", 9, 4]])],
        "RELIC.AKABEKO": [("AfterSideTurnStart",
                           [["own_side"], ["turn", "le", 1]],
                           [["power", "VigorPower", "self", 8]])],
        "RELIC.BAG_OF_MARBLES": [("BeforeSideTurnStart",
                                  [["own_side"], ["turn", "le", 1]],
                                  [["power_all", "VulnerablePower", 1]])],
        "RELIC.RED_MASK": [("BeforeSideTurnStart",
                            [["own_side"], ["turn", "le", 1]],
                            [["power_all", "WeakPower", 1]])],
        "RELIC.CANDELABRA": [("AfterSideTurnStart",
                              [["own_side"], ["turn", "eq", 2]],
                              [["energy", 2]])],
        "RELIC.CHANDELIER": [("AfterSideTurnStart",
                              [["own_side"], ["turn", "eq", 3]],
                              [["energy", 3]])],
        "RELIC.HORN_CLEAT": [("AfterBlockCleared",
                              [["is_owner"], ["turn", "eq", 2]],
                              [["block", 14]])],
        "RELIC.VAJRA": [("AfterRoomEntered",
                         [["room_type", "CombatRoom"]],
                         [["power", "StrengthPower", "self", 1]])],
        "RELIC.ODDLY_SMOOTH_STONE": [("AfterRoomEntered",
                                      [["room_type", "CombatRoom"]],
                                      [["power", "DexterityPower",
                                        "self", 1]])],
        "RELIC.BRONZE_SCALES": [("AfterRoomEntered",
                                 [["room_type", "CombatRoom"]],
                                 [["power", "ThornsPower", "self", 3]])],
        "RELIC.GORGET": [("AfterRoomEntered",
                          [["room_type", "CombatRoom"]],
                          [["power", "PlatingPower", "self", 4]])],
        "RELIC.BAG_OF_PREPARATION": [("ModifyHandDraw",
                                      [["is_owner"], ["turn", "le", 1]],
                                      [["draw_bonus", 2]])],
        "RELIC.RING_OF_THE_SNAKE": [("ModifyHandDraw",
                                     [["is_owner"], ["turn", "le", 1]],
                                     [["draw_bonus", 2]])],
    }
    for rid, exp in expected.items():
        t = data.get(rid)
        assert t is not None, f"{rid}: fell out of the template set"
        got = []
        for h in t["hooks"]:
            for leaf in h["guarded"]:
                steps = [[t["vars"][x["var"]]["base"]
                          if isinstance(x, dict) else x for x in st]
                         for st in leaf["steps"]]
                got.append((h["hook"], leaf["conds"], steps))
        assert got == exp, f"{rid}: {got} != {exp}"


# --------------------------------------------------------------------------
# The six remaining Act-1 elites (#58, 2026-07-10). Entry states are the
# real eel fight with the encounter swapped: same seed, same deck, same
# Shuffle counter — everything below is deterministic and hand-checked
# against the IL constants in ENCOUNTER_MECHANICS.md.
# --------------------------------------------------------------------------


def test_dotnet_sort_small_partitions_insertion_stable():
    """Top-level inputs of size 4-16 hit InsertionSort => equal keys keep
    input order; size 2 is a stable compare-swap; size 3 is .NET's
    UNSTABLE swap network — [B1, B2, A] comes out [A, B2, B1] because the
    (0,2) swap carries B1 across its equal twin. All pinned here because
    reshuffle tie order inherits every one of these quirks."""
    from dotnet_sort import dotnet_list_sort

    def sort_tagged(pairs):
        return dotnet_list_sort(pairs, key=lambda v: v[0])

    # 16 elements, two mixed groups: stable
    xs = [(k, i) for i, k in enumerate([3, 1, 3, 2, 3, 1, 2, 3,
                                        0, 3, 1, 2, 3, 0, 3, 3])]
    out = sort_tagged(xs)
    assert out == sorted(xs, key=lambda v: v[0])   # python sort is stable
    # size 2: stable
    assert sort_tagged([(1, "x"), (1, "y")]) == [(1, "x"), (1, "y")]
    # size 3 swap network: [B1, B2, A] -> [A, B2, B1]
    assert sort_tagged([(1, "B1"), (1, "B2"), (0, "A")]) == \
        [(0, "A"), (1, "B2"), (1, "B1")]
    # ...but [B1, A, B2] -> [A, B1, B2] (no crossing swap)
    assert sort_tagged([(1, "B1"), (0, "A"), (1, "B2")]) == \
        [(0, "A"), (1, "B1"), (1, "B2")]


def test_dotnet_sort_large_partition_hand_traced():
    """17 elements => one quicksort partition pass, then insertion sorts.
    Expected output derived BY HAND from the dumped GenericArraySortHelper
    IL (median-of-3 over indices 0/8/16, pivot parked at 15, the
    pre-increment/pre-decrement scan swaps (2,12) (4,11) (5,10) (6,8),
    final pivot swap (8,15), then stable insertion sorts of both sides) —
    independently of the Python port, so a transcription bug in either
    direction breaks this pin."""
    from dotnet_sort import dotnet_list_sort
    keys = [5, 3, 5, 1, 5, 9, 5, 0, 2, 8, 5, 4, 5, 7, 6, 5, 5]
    tags = {0: "a", 2: "b", 4: "c", 6: "d", 10: "e", 12: "f",
            15: "g", 16: "h"}
    xs = [(k, tags.get(i, "")) for i, k in enumerate(keys)]
    out = dotnet_list_sort(xs, key=lambda v: v[0])
    assert [k for k, _ in out] == sorted(keys)
    assert [t for k, t in out if k == 5] == \
        ["f", "e", "g", "a", "c", "b", "d", "h"]


def test_removed_card_and_upgrade_dating():
    """relay_parser must date cards_removed / upgraded_cards node events:
    a purged starting card was still in every earlier fight's deck (at its
    sibling's array slot), and a rest-site smith only upgrades fights
    after it. The gained-log heuristic alone is blind to removed starting
    cards (the 7MA0PY7AD4 Strike)."""
    import tempfile
    from relay_parser import parse_run

    def ps(**kw):
        base = dict(current_hp=50, damage_taken=0, hp_healed=0, max_hp=50,
                    max_hp_gained=0, max_hp_lost=0, current_gold=0,
                    gold_gained=0, gold_lost=0, gold_spent=0, gold_stolen=0)
        base.update(kw)
        return base

    deck = [
        {"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1},
        {"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1},
        {"id": "CARD.BASH", "floor_added_to_deck": 1,
         "current_upgrade_level": 1},
    ]
    fightroom = [{"model_id": "ENCOUNTER.X", "monster_ids": [],
                  "turns_taken": 1}]
    nodes = [
        {"map_point_type": "ancient", "rooms": [],
         "player_stats": [ps()]},
        {"map_point_type": "monster", "rooms": fightroom,
         "player_stats": [ps()]},
        {"map_point_type": "unknown", "rooms": [],
         "player_stats": [ps(cards_removed=[
             {"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1}])]},
        {"map_point_type": "rest_site", "rooms": [],
         "player_stats": [ps(upgraded_cards=["CARD.BASH"])]},
        {"map_point_type": "monster", "rooms": fightroom,
         "player_stats": [ps()]},
    ]
    run = {"seed": "TESTSEED", "build_id": "v0.108.0", "schema_version": 1,
           "ascension": 10, "win": True, "killed_by_encounter": "",
           "map_point_history": [nodes],
           "players": [{"character": "IRONCLAD", "deck": deck,
                        "relics": [], "max_potion_slot_count": 2}]}
    with tempfile.NamedTemporaryFile("w", suffix=".run") as f:
        json.dump(run, f)
        f.flush()
        s = parse_run(f.name)
    first, last = s.fights
    # fight 0: three Strikes (one purged later), BASH not yet upgraded
    assert [c["id"] for c in first.deck_entering].count(
        "CARD.STRIKE_IRONCLAD") == 3
    assert [c["upgrade_level"] for c in first.deck_entering
            if c["id"] == "CARD.BASH"] == [0]
    # the re-inserted Strike sits in the strike block, before BASH
    assert [c["id"].replace("CARD.", "")[0] for c in first.deck_entering] \
        == ["S", "S", "S", "B"]
    # fight 3 (after removal + smith): two Strikes, BASH+
    assert [c["id"] for c in last.deck_entering].count(
        "CARD.STRIKE_IRONCLAD") == 2
    assert [c["upgrade_level"] for c in last.deck_entering
            if c["id"] == "CARD.BASH"] == [1]


def test_parser_dates_cross_combat_relic_state():
    """Synthetic history: the Tea Set charge is set by a rest site the
    relic was owned at and consumed by ANY combat — including event-node
    combats, which also tick the Flower counter (they record turns_taken
    like normal combats; TurnNumber counts extra turns too, SwitchSides
    IL 0x2df690)."""
    from relay_parser import (FightState, RunSummary,
                              _date_cross_combat_relic_state)

    def fight(node, relics, turns):
        return FightState(
            node_index=node, node_type="monster", encounter_id="E",
            monster_ids=[], hp_entering=1, max_hp_entering=1,
            gold_entering=0, deck_entering=[], relics_entering=relics,
            potions_entering=[], damage_taken=0, hp_healed=0,
            turns_taken=turns, potions_used=[], hp_after=1)

    def node(t, turns=None, two_room=False):
        # single-room event combats carry room_type 'monster' (457 in the
        # local history); two-room event nodes are [event, monster] (51)
        room = {"room_type": "monster" if t == "unknown" else t}
        if turns is not None:
            room["turns_taken"] = turns
        rooms = ([{"room_type": "event", "turns_taken": 0}, room]
                 if two_room else [room])
        return {"map_point_type": t, "rooms": rooms, "player_stats": [{}]}

    relics = ["RELIC.VENERABLE_TEA_SET", "RELIC.HAPPY_FLOWER",
              "RELIC.GALACTIC_DUST"]
    player = {"relics": [{"id": r, "floor_added_to_deck": 1}
                         for r in relics]}   # starting relics (floor 1)
    # ...except the Flower, picked at the node-2 rest site (floor 3), so the
    # node-3 event combat is its only prior owned combat (#3076).
    player["relics"][1]["floor_added_to_deck"] = 3
    # nodes: 0 ancient | 1 fight(4 turns) | 2 rest | 3 TWO-ROOM event
    # combat(2 turns — the combat hides at rooms[1], the case the
    # original rooms[0] scan missed) | 4 fight
    history = [node("ancient"), node("monster", 4), node("rest_site"),
               node("unknown", 2, two_room=True), node("monster", 7)]
    summary = RunSummary(seed="S", build_id="b", schema_version=1,
                         character="c", ascension=0, win=True,
                         killed_by=None)
    summary.fights = [fight(1, relics, 4), fight(4, relics, 7)]
    _date_cross_combat_relic_state(summary, history, player)
    f0, f1 = summary.fights
    assert f0.tea_set_charged is False           # no rest before node 1
    assert f0.relic_counters["RELIC.HAPPY_FLOWER"] == 0
    # Dust starts from its CLR-default remainder only before the FIRST
    # owned combat.  The action history needed to advance it is absent.
    assert f0.relic_counters["RELIC.GALACTIC_DUST"] == 0
    # the event combat at node 3 CONSUMED the rest-site charge...
    assert f1.tea_set_charged is False
    # ...and it is an owned combat for the Flower, whose count it may or may
    # not have ticked on its last turn (#3076): no exact claim.
    assert "RELIC.HAPPY_FLOWER" not in f1.relic_counters
    assert any("HAPPY_FLOWER turn counter unseedable after a prior owned "
               "combat" in c for c in f1.caveats)
    assert "RELIC.GALACTIC_DUST" not in f1.relic_counters
    assert any("GALACTIC_DUST Stars-spend remainder unseedable" in c
               for c in f1.caveats)
    # without the event combat the charge survives and the count drops
    history2 = [node("ancient"), node("monster", 4), node("rest_site"),
                node("unknown"), node("monster", 7)]
    summary.fights = [fight(1, relics, 4), fight(4, relics, 7)]
    _date_cross_combat_relic_state(summary, history2, player)
    assert summary.fights[1].tea_set_charged is True
    # no owned combat before node 4 -> the Flower still enters at 0
    assert summary.fights[1].relic_counters["RELIC.HAPPY_FLOWER"] == 0
    assert "RELIC.GALACTIC_DUST" not in summary.fights[1].relic_counters
    assert any("GALACTIC_DUST Stars-spend remainder unseedable" in c
               for c in summary.fights[1].caveats)


def _synthetic_run(tmpdir):
    """Minimal .run with an ancient start, a two-room event combat, and a
    monster fight — the smallest structure parse_run accepts."""
    def stats(hp, dmg=0, healed=0):
        return {"current_hp": hp, "damage_taken": dmg, "hp_healed": healed,
                "max_hp": 80, "max_hp_gained": 0, "max_hp_lost": 0,
                "current_gold": 99, "gold_gained": 0, "gold_lost": 0,
                "gold_spent": 0, "gold_stolen": 0}

    def npt(t, rooms, ps):
        return {"map_point_type": t, "rooms": rooms, "player_stats": [ps]}

    run = {
        "seed": "TESTSEED00", "build_id": "vTEST", "schema_version": 1,
        "ascension": 0, "win": False, "killed_by_encounter": "NONE.NONE",
        "map_point_history": [[
            npt("ancient", [{"room_type": "ancient"}], stats(80)),
            npt("unknown",
                [{"room_type": "event", "model_id": "EVENT.DUMMY",
                  "turns_taken": 0},
                 {"room_type": "monster", "turns_taken": 2,
                  "model_id": "ENCOUNTER.SLIMES_WEAK",
                  "monster_ids": ["MONSTER.SLIME", "MONSTER.SLIME"]}],
                stats(75, dmg=5)),
            npt("monster",
                [{"room_type": "monster", "turns_taken": 3,
                  "model_id": "ENCOUNTER.NIBBITS_WEAK",
                  "monster_ids": ["MONSTER.NIBBIT"]}],
                stats(70, dmg=5)),
        ]],
        "players": [{
            "character": "IRONCLAD", "max_potion_slot_count": 2,
            "deck": [{"id": "CARD.STRIKE_IRONCLAD",
                      "floor_added_to_deck": 1,
                      "current_upgrade_level": 0}],
            "relics": [],
        }],
    }
    p = tmpdir / "synthetic.run"
    p.write_text(json.dumps(run))
    return str(p)


def test_replay_counter_walk_includes_event_combats(tmp_path):
    """The Shuffle stream is run-global: an event combat's combat-start
    shuffle consumes draws exactly like a fight node's. load_combats must
    list it (with the event flag caveated in predict's prior walk)."""
    from replay_fight import load_combats
    run = load_combats(_synthetic_run(tmp_path))
    assert [(c["node_index"], c["event"]) for c in run["combats"]] == \
        [(1, True), (2, False)]
    assert run["combats"][0]["encounter"] == "ENCOUNTER.SLIMES_WEAK"


def test_parser_seeds_attack_counters_only_at_zero():
    """Nunchaku/Pen Nib: no .run record of attack plays — the parser
    seeds 0 only when the relic has seen no prior owned combat."""
    from relay_parser import (FightState, RunSummary,
                              _date_cross_combat_relic_state)

    def fight(node, relics, turns):
        return FightState(
            node_index=node, node_type="monster", encounter_id="E",
            monster_ids=[], hp_entering=1, max_hp_entering=1,
            gold_entering=0, deck_entering=[], relics_entering=relics,
            potions_entering=[], damage_taken=0, hp_healed=0,
            turns_taken=turns, potions_used=[], hp_after=1)

    def node(t, turns=None):
        room = {"room_type": "monster" if t == "unknown" else t}
        if turns is not None:
            room["turns_taken"] = turns
        return {"map_point_type": t, "rooms": [room], "player_stats": [{}]}

    history = [node("ancient"), node("monster", 4), node("shop"),
               node("monster", 7)]
    summary = RunSummary(seed="S", build_id="b", schema_version=1,
                         character="c", ascension=0, win=True,
                         killed_by=None)
    # obtained at the shop (floor 3 = node 2): fight at node 1 predates
    # ownership -> seed 0 at node 3; owned-from-start -> unseedable
    player = {"relics": [{"id": "RELIC.NUNCHAKU",
                          "floor_added_to_deck": 3},
                         {"id": "RELIC.PEN_NIB",
                          "floor_added_to_deck": 1}]}
    relics = ["RELIC.NUNCHAKU", "RELIC.PEN_NIB"]
    summary.fights = [fight(3, relics, 7)]
    _date_cross_combat_relic_state(summary, history, player)
    f = summary.fights[0]
    assert f.relic_counters.get("RELIC.NUNCHAKU") == 0
    assert "RELIC.PEN_NIB" not in f.relic_counters
    assert any("PEN_NIB attack counter unseedable" in c for c in f.caveats)


def test_hqpaxcbs6p_live_pin_card_selection_and_hands():
    """First IN-GAME validation of the CombatCardSelection port (Sean's
    HQPAXCBS6P run, observed live 2026-07-11; .run 1783833699).

    Observed: fight 1 (node-3 corpse-slug event fight), turn-1 hand
    [Greed, Cinder, Defend, Defend, Strike], played Strike then Cinder,
    Cinder exhausted the LEFT Defend. Fight 5 (node-10 Terror Eel),
    turn-1 hand [Defend+, Cinder, Strike+, Strike, Defend], played
    Defend+ then Cinder, Cinder exhausted the Strike+.

    The hand replays also PROVED the .run deck reconstruction one card
    short in both fights (12 -> 13, 16 -> 17, matching the on-screen
    deck counters): an extra card X with no gain/removal record —
    X at array index 9, Bash's starter slot, satisfies both fights, and
    the final deck is missing Bash with no recorded removal (suspected
    unrecorded node-0 "ancient" event trade). X never surfaces in an
    observed hand, so its identity does not affect these pins."""
    from sts2_rng import Rng, RunRngSet
    rs = RunRngSet("HQPAXCBS6P")
    sel_seed = rs["CombatCardSelection"].seed

    # -- CombatCardSelection: the two observed exhausts --
    # fight 1, first consumer play of the run (counter 0): pool =
    # [Greed, Defend(left), Defend(right)] -> index 1, the left Defend
    assert Rng(sel_seed, counter=0).next_int(0, 3) == 1
    # fight 5: pool = [Strike+, Strike, Defend] -> observed index 0,
    # which the stream yields ONLY at counters 3-5 in the plausible
    # range (>= 1 fight-1 draw + unrecorded in-between Cinder plays)
    assert [c for c in range(9)
            if Rng(sel_seed, counter=c).next_int(0, 3) == 0] == [3, 4, 5]

    # -- Shuffle: observed turn-1 hands pin the counters AND the
    #    13th/17th card (parsed decks are unreachable) --
    S, D = "STRIKE_IRONCLAD", "DEFEND_IRONCLAD"
    seed = rs["Shuffle"].seed

    def first5(deck, counter):
        pile = list(deck)
        Rng(seed, counter=counter).shuffle(pile)
        return pile[:5]

    deck1_parsed = [S] * 5 + [D] * 4 + ["ASCENDERS_BANE", "GREED", "CINDER"]
    obs1 = ["GREED", "CINDER", D, D, S]        # all L0 at node 3
    assert all(first5(deck1_parsed, c) != obs1 for c in range(0, 61))
    deck1 = deck1_parsed[:9] + ["X"] + deck1_parsed[9:]     # Bash slot
    assert first5(deck1, 19) == obs1
    assert [c for c in range(0, 61) if first5(deck1, c) == obs1] == [19]

    tail = ["GREED", "CINDER", "STOMP", "BLOODLETTING", "HAVOC",
            "POMMEL_STRIKE"]
    obs5 = [(D, 1), ("CINDER", 0), (S, 1), (S, 0), (D, 0)]
    # exactly one Strike+ and one Defend+ at node 10 (the node-5 rest;
    # the second Defend+ dates to node 11 — the parser's 2x Defend+ for
    # this fight is its dating artifact). Copy assignment is unrecorded:
    # the pin quantifies over it.
    def decks5(with_x):
        for s_up in range(5):
            for d_up in range(4):
                base = ([(S, 1 if i == s_up else 0) for i in range(5)]
                        + [(D, 1 if i == d_up else 0) for i in range(4)]
                        + [("ASCENDERS_BANE", 0)] + [(c, 0) for c in tail])
                if with_x:
                    yield base[:9] + [("X", 0)] + base[9:]
                else:
                    yield base
    assert all(first5(d, c) != obs5
               for d in decks5(False) for c in range(85, 130))
    match = [(c, i) for i, d in enumerate(decks5(True))
             for c in range(85, 130) if first5(d, c) == obs5]
    assert match and all(c == 100 for c, _ in match)


def test_transform_dating_restores_original_card():
    """cards_transformed (Archaic Tooth: Bash -> Break, HQPAXCBS6P act-2
    entry) is a dated removal of the original card plus an ordinary
    floor-stamped add of the final card (#114). With the transform
    consumed and the starter-slot re-insertion rule, the parser + counter
    replay reproduce the LIVE-observed fight-1 opening hand end-to-end
    from the raw .run — zero free parameters."""
    from relay_parser import parse_run
    from replay_fight import predict
    path = str(pathlib.Path(__file__).parent / "testdata" / "HQPAXCBS6P.run")
    run = parse_run(path)
    d1 = [c["id"] for c in run.fights[1].deck_entering]
    d5 = [c["id"] for c in run.fights[5].deck_entering]
    assert len(d1) == 13 and d1[9] == "CARD.BASH"
    assert len(d5) == 17 and d5[9] == "CARD.BASH"
    d9 = [c["id"] for c in run.fights[9].deck_entering]     # post-transform
    assert "CARD.BASH" not in d9 and "CARD.BREAK" in d9
    # the re-inserted slot is still honestly flagged approximate
    assert any("BASH" in c for c in run.fights[1].caveats)
    # Break's appearance is explained by the transform — no global caveat
    assert not any("no recorded gain" in c for c in run.global_caveats)
    p = predict(path, 1)
    assert p["shuffle_counter_entering"] == 19               # pinned live
    assert p["predicted_turn1_hand"] == [
        "GREED", "CINDER", "DEFEND_IRONCLAD", "DEFEND_IRONCLAD",
        "STRIKE_IRONCLAD"]                                   # observed live


# --- EW2 encounter wave (#138): Spiny Toad + Overgrowth Crawlers ------------


def test_obscura_family_agrees_with_encounter_census():
    """I1: builders and modeled mechanics cover the checked-in v0.109 rows."""
    cen = json.load(open(pathlib.Path(__file__).with_name(
        "encounters_census.json")))
    encounters, monsters = cen["encounters"], cen["monsters"]
    assert encounters["ENCOUNTER.THE_OBSCURA_NORMAL"]["monsters"] == \
        ["TheObscura", "Parafright"]
    assert encounters["ENCOUNTER.FOGMOG_NORMAL"]["monsters"] == \
        ["Fogmog", "EyeWithTeeth"]
    assert (monsters["TheObscura"]["move_states"],
            monsters["TheObscura"]["random_branches"],
            monsters["TheObscura"]["hp_min_consts"]) == \
        (4, 1, [8, 129, 123])
    assert (monsters["Parafright"]["move_states"],
            monsters["Parafright"]["powers_applied"],
            monsters["Parafright"]["hp_min_consts"]) == \
        (1, ["IllusionPower"], [21])
    assert (monsters["Fogmog"]["move_states"],
            monsters["Fogmog"]["random_branches"],
            monsters["Fogmog"]["hp_min_consts"]) == \
        (4, 1, [8, 78, 74])
    assert (monsters["EyeWithTeeth"]["move_states"],
            monsters["EyeWithTeeth"]["powers_applied"],
            monsters["EyeWithTeeth"]["card_effects"]) == \
        (1, ["IllusionPower"],
         ["CardPileCmd::AddToCombatAndPreview<Dazed>"])


if __name__ == "__main__":
    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"PASS {name}")
            except AssertionError as e:
                failed += 1
                print(f"FAIL {name}: {e}")
    sys.exit(1 if failed else 0)
