"""
Replay the draw RNG of any fight in a .run file — entry deck order,
Shuffle-stream counter accounting, and opening-hand prediction (issue #60).

Model (all verified against sts2.dll IL of build v0.108.0 and live ground
truth for fight 1; see RNG_FINDINGS.md):

  entry deck order  = the .run final-deck array (acquisition-ordered,
                      verified monotone in floor_added_to_deck) filtered to
                      floor_added_to_deck < fight floor, PLUS cards the
                      per-node cards_removed log says were purged later
                      (re-inserted at their sibling's slot — the final
                      array alone silently loses them; see the 7MA0PY7AD4
                      fifth Strike in SOLVER_INVARIANTS.md I10).

  Shuffle-stream consumers (complete list from IL):
    - combat start: deck cloned in order -> one Fisher-Yates = n-1 draws
    - reshuffle when the draw pile runs out: discard size - 1 draws
    - CardPileCmd.Add with random position (e.g. "shuffle a card into the
      draw pile"): 1 draw  [not modeled — warn if plausible]
    - BeatDown / Catastrophe / Uproar / StampedePower [not modeled — warn]

  Per-turn draws: 5 (verified turns 1-3 of fight 1). Ascender's Bane is
  ethereal+unplayable: it leaves the cycle at the end of the turn it is
  drawn. Playing cards does not change reshuffle sizes otherwise (only
  exhaust/power plays would, which we can't see in a .run — warned).

Counter accounting per fight is exact for "plain" decks (no draw effects,
no exhaust plays, no card-adding monsters). Prior fights whose MONSTERS
add cards mid-combat (slimes' Slimed, Phrog Infections, Entomancer Dazes,
… — the deleted solve_fight.CARD_AFFLICTING_MONSTERS, now Rust
run_counters' registry) make the predicted counter a BASELINE: each afflicted
card in a pre-reshuffle discard adds one unrecorded draw (the review prints
the caveat).

Usage:
    python3 replay_fight.py path/to/file.run FIGHT_INDEX [--turns N,N,...]
        FIGHT_INDEX: 0-based index into the run's combats
        --turns: override turns_taken of PRIOR fights (comma list, in combat
                 order) when predicting a fresh replay rather than the
                 recorded run.
"""

from __future__ import annotations

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from sts2_rng import Rng, RunRngSet  # noqa: E402
from relay_parser import require_single_player  # noqa: E402

ETHEREAL_UNPLAYABLE = {"CARD.ASCENDERS_BANE"}
DRAWS_PER_TURN = 5
HAND_SIZE = 5

_CENSUS_PATH = pathlib.Path(__file__).parent / "cards_census.json"
_census_cache: dict | None = None


def cycle_leavers(deck_ids: list[str]) -> list[str]:
    """Cards in a deck whose PLAY (or non-play) can change reshuffle sizes
    by an unrecorded amount: powers leave the cycle when played; Exhaust
    cards leave when played; Ethereal cards leave if drawn and unplayed
    (only the modeled unplayable ones are exact); Retain cards skip the
    end-of-turn discard; draws / adds_cards / shuffles / exhausts_other
    change consumption directly. Thrumming Hatchet's post-first-play
    hand-retention (BeforeHandDraw CardPileCmd::Add, IL-read 2026-07-14)
    is covered by its adds_cards census flag.

    The HYHM8WP1E5 live capture proved this matters: the recorded Shuffle
    counter entering the boss was 113 vs a 118 baseline, and even the
    hatchet-timing hypothesis set {115..118} missed it — INFLAME (power)
    and TREMBLE (Exhaust) plays were also unrecorded. A deck containing
    any of these makes later entering counters a hypothesis, not exact."""
    global _census_cache
    if _census_cache is None:
        _census_cache = json.load(open(_CENSUS_PATH))
    out = set()
    for cid in deck_ids:
        if cid in ETHEREAL_UNPLAYABLE:
            continue
        spec = _census_cache.get(cid)
        if spec is None:
            out.add(cid + " (not in cards_census)")
            continue
        kw = set(spec.get("keywords", []))
        if (spec.get("type") == "power" or spec.get("draws")
                or spec.get("adds_cards") or spec.get("shuffles")
                or spec.get("exhausts_other")
                or kw & {"Exhaust", "Ethereal", "Retain"}):
            out.add(cid)
    return sorted(out)


def load_combats(run_path: str) -> dict:
    data = json.load(open(run_path))
    deck = require_single_player(data, ".run replay")["deck"]
    nodes = [pt for act in data["map_point_history"] for pt in act]
    combats = []
    removals = []                # (node_index, removed card entry)
    for i, pt in enumerate(nodes):
        for c in pt["player_stats"][0].get("cards_removed", []):
            removals.append((i, c))
        # a transform (Archaic Tooth: Bash -> Break) is a dated removal
        # of the original; the final card is an ordinary deck entry
        # gated by its own floor stamp (issue #114)
        for t in pt["player_stats"][0].get("cards_transformed", []):
            removals.append((i, t["original_card"]))
        rooms = pt.get("rooms") or []
        # combat rooms are identified by their OWN room_type at any slot:
        # event ('unknown') nodes host real combats too, and those consume
        # the run-global Shuffle stream exactly like fight nodes — skipping
        # them silently corrupted counter propagation for any run with one
        room = next((r for r in rooms
                     if r.get("room_type") in ("monster", "elite", "boss")),
                    None)
        if pt["map_point_type"] in ("monster", "elite", "boss") or (
                room is not None and room.get("turns_taken")):
            room = room or {}
            combats.append({
                "node_index": i,
                "floor": i + 1,
                "encounter": room.get("model_id", "?"),
                "turns": room.get("turns_taken", -1),
                "event": pt["map_point_type"] == "unknown",
            })
    return {"seed": data["seed"], "deck": deck, "combats": combats,
            "removals": removals}


def entry_deck(deck: list[dict], floor: int, removals=()) -> list[dict]:
    """Deck array filtered to cards owned before this floor's fight,
    preserving array (acquisition) order. Cards recorded in a node's
    cards_removed were still in the deck for every EARLIER fight — the
    final array is missing them, so they are re-inserted after their last
    same-(id, floor) sibling (exact when identical copies survive)."""
    cards = [c for c in deck if c["floor_added_to_deck"] < floor]
    for j, rc in removals:
        # present iff the removal node j is AT or after the fight's node
        # (floor - 1): fight-node rewards only ADD, so a removal recorded
        # at a fight's own node happened DURING its combat — Thieving
        # Hopper theft (#827: TQM88QFMHSQR fight 9's Colossus, whose
        # return also RESTAMPED floor_added_to_deck to the theft floor;
        # HQPAXCBS6P's node-19 Taunt is the same signature). Event-node
        # combats stay ambiguous (the caller's event caveat covers them).
        if j < floor - 1 or rc.get("floor_added_to_deck", 1) >= floor:
            continue                             # removed before this fight
        pos = 0
        for k, c in enumerate(cards):
            if (c["id"], c["floor_added_to_deck"]) == \
                    (rc["id"], rc.get("floor_added_to_deck", 1)):
                pos = k + 1
        if pos == 0:
            # no identical sibling. Starter-floor cards belong in the
            # CHARACTER block, before Ascender's Bane — floor-1 event
            # grants (Greed) append after it, so "after the last
            # floor <= 1 card" lands starters wrong; the HQPAXCBS6P
            # hand pin proved the transformed Bash sat at the pre-Bane
            # slot (test_hqpaxcbs6p_live_pin_card_selection_and_hands).
            bane = next((k for k, c in enumerate(cards)
                         if c["id"] == "CARD.ASCENDERS_BANE"), None)
            if rc.get("floor_added_to_deck", 1) == 1 and bane is not None:
                pos = bane
            else:
                for k, c in enumerate(cards):
                    if c["floor_added_to_deck"] <= rc.get(
                            "floor_added_to_deck", 1):
                        pos = k + 1
        cards.insert(pos, rc)
    return cards


def fight_consumption(deck_ids: list[str], turns: int,
                      rng: Rng) -> tuple[int, list[str]]:
    """Simulate one combat's Shuffle-stream consumption using the REAL rng
    (so ethereal timing is exact). Returns (draws consumed, caveats).
    Advances `rng` past the whole fight."""
    caveats = []
    start = rng.counter
    pile = list(deck_ids)
    rng.shuffle(pile)                      # combat start: n-1 draws
    pile.reverse()                         # draw from front == pop from end
    discard: list[str] = []
    removed = 0
    for _turn in range(1, turns + 1):
        drawn = []
        for _ in range(DRAWS_PER_TURN):
            if not pile:
                if not discard:
                    break                  # tiny cycle: nothing to reshuffle
                rng.shuffle(discard)       # reshuffle: size-1 draws
                pile = discard[::-1]
                discard = []
            drawn.append(pile.pop())
        # end of turn: ethereal-unplayable cards leave the cycle
        for c in drawn:
            if c in ETHEREAL_UNPLAYABLE:
                removed += 1
            else:
                discard.append(c)
    return rng.counter - start, caveats


def predict(run_path: str, fight_index: int,
            turns_override: list[int] | None = None) -> dict:
    run = load_combats(run_path)
    combats = run["combats"]
    if not 0 <= fight_index < len(combats):
        raise SystemExit(f"fight index out of range (0..{len(combats)-1})")
    rs = RunRngSet(run["seed"])
    rng = Rng(rs["Shuffle"].seed)  # counter 0 at run start
    caveats: list[str] = []

    for k in range(fight_index):
        fight = combats[k]
        ids = [c["id"] for c in entry_deck(run["deck"], fight["floor"],
                                           run["removals"])]
        turns = (turns_override[k] if turns_override and k < len(turns_override)
                 else fight["turns"])
        if turns < 0:
            raise SystemExit(f"fight {k}: turns unknown; pass --turns")
        _, cv = fight_consumption(ids, turns, rng)
        caveats += [f"fight {k} ({fight['encounter']}): {c}" for c in cv]
        leavers = cycle_leavers(ids)
        if leavers:
            caveats.append(
                f"fight {k} ({fight['encounter']}): deck held cards that "
                f"can leave/alter the cycle unrecorded "
                f"({', '.join(c.replace('CARD.', '') for c in leavers)}) — "
                "the counter is a baseline, not exact")
        if fight.get("event"):
            caveats.append(
                f"fight {k} ({fight['encounter']}) is an event-node combat:"
                " cards the event granted BEFORE its fight (ordering"
                " unrecorded) would shift its deck size and reshuffle"
                " boundaries — counter is a baseline")

    target = combats[fight_index]
    ids = [c["id"] for c in entry_deck(run["deck"], target["floor"],
                                       run["removals"])]
    counter = rng.counter
    order = list(ids)
    rng.shuffle(order)
    short = [c.replace("CARD.", "") for c in order]
    return {
        "seed": run["seed"],
        "fight_index": fight_index,
        "encounter": target["encounter"],
        "floor": target["floor"],
        "shuffle_counter_entering": counter,
        "entry_deck": [c.replace("CARD.", "") for c in ids],
        "predicted_turn1_hand": short[:HAND_SIZE],
        "predicted_turn2_hand": short[HAND_SIZE:2 * HAND_SIZE],
        "full_first_cycle": short,
        "caveats": caveats,
    }


if __name__ == "__main__":
    path, idx = sys.argv[1], int(sys.argv[2])
    override = None
    if "--turns" in sys.argv:
        override = [int(x) for x in
                    sys.argv[sys.argv.index("--turns") + 1].split(",")]
    r = predict(path, idx, override)
    print(f"seed {r['seed']} — fight {r['fight_index']} "
          f"({r['encounter']}, floor {r['floor']})")
    print(f"  entry deck ({len(r['entry_deck'])}): {', '.join(r['entry_deck'])}")
    print(f"  Shuffle counter entering: {r['shuffle_counter_entering']}")
    print(f"  predicted turn-1 hand: {', '.join(r['predicted_turn1_hand'])}")
    print(f"  predicted turn-2 hand: {', '.join(r['predicted_turn2_hand'])}")
    for c in r["caveats"]:
        print(f"  ! {c}")
