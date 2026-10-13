"""What a replay capture itself records, read without the Python simulator.

Two facts every recorded-line reader needs, moved here VERBATIM (#2999) out of
modules that import `combat_sim` at load time:

* the capture's own deal — combat-card instance k is the k-th card of the
  entry deck after the combat-start `Shuffle` draw (`first_cycle_from_replay`,
  `first_cycle_rows_from_replay`, formerly in `mcr_replay`);
* the nine RNG rows a native full-state checkpoint carries
  (`_REPRESENTED_RNG_STREAMS`, `_rng_tuple`, `UnsupportedStateProjection`,
  formerly in `tools/mcr_validate`).

`mcr_replay` and `mcr_validate` import these names back, so every existing
reader gets the identical objects. Only the stdlib, `relay_parser` and
`sts2_rng` are imported here: this is the surface the Rust-rooted eval census
(`sim/v0.111.0/engine/tools/eval_suite.py`) and the production recorded
replay (`rust_replay`) share once the simulator is deleted (#2827 item F).
"""

from __future__ import annotations

from relay_parser import require_single_player
from sts2_rng import Rng, RunRngSet

_REPRESENTED_RNG_STREAMS = {
    "rng": "Shuffle",
    "niche": "Niche",
    "ai": "MonsterAi",
    "sel": "CombatCardSelection",
    "generation": "CombatCardGeneration",
    "potion_generation": "CombatPotionGeneration",
    "targets": "CombatTargets",
    "combat_orbs": "CombatOrbs",
    "energy_costs": "CombatEnergyCosts",
}


class UnsupportedStateProjection(NotImplementedError):
    """A checksum field has no exact solver projection (I5)."""


def _rng_tuple(rng_doc: dict, stream: str) -> tuple:
    try:
        words = rng_doc["states"][stream]
        counter = rng_doc["counters"][stream]
    except (KeyError, TypeError):
        raise UnsupportedStateProjection(
            f"RNG stream {stream!r} lacks counter+xoshiro state") from None
    if (not isinstance(words, list) or len(words) != 4
            or any(type(word) is not int or not 0 <= word < 1 << 64
                   for word in words)
            or type(counter) is not int or counter < 0):
        raise UnsupportedStateProjection(
            f"RNG stream {stream!r} has malformed counter+xoshiro state")
    return tuple(words) + (counter,)


def _saved_int(card: dict, name: str):
    """One saved integer property of a deck row, in either spelling: the
    decoder's flat `{name: value}` or a save's `{"ints": [{name, value}]}`."""
    props = card.get("props") or {}
    if name in props:
        return props[name]
    for row in props.get("ints") or ():
        if isinstance(row, dict) and row.get("name") == name:
            return row.get("value")
    return None


def combat_deck(deck: list, root: dict | None) -> list:
    """The entry snapshot's deck as the combat copies were made from it.

    The snapshot is written on moving onto the node, before the room is
    entered. `Cards.Dowsing::BeforeRoomEntered` (v0.111.0 DLL `9cb4f1ad`,
    `Dowsing/<BeforeRoomEntered>d__15::MoveNext` RVA `0x399e18`) counts the
    first room of a `?` point, and when the saved `RoomsEntered` 4 becomes 5
    it replaces the deck card with an Abundance (`TransformTo<Abundance>`,
    IL_0099-00a1) that `CardCmd/<Transform>d__13` (RVA `0x3e0ae0`) adds at
    the deck's END (`AddInternal(replacement, -1, false)`, IL_03ca-03d8). The
    combat deck is cloned afterwards, so combat-card instance k is row k of
    THAT deck (#3791).

    Whether this entry was the counting room is the Rust opening's decision
    (`entry::opening::dowsing_before_room_entered`), and it is read off the
    root rather than decided a second time here: the saved deck holds
    Dowsing rows one room short, and the root holds that many fewer Dowsing
    cards. Every instance is still compared with the root's own card
    afterwards, so a reading the root does not bear out refuses the deal.
    Without a root, or when nothing transformed, the deck is returned as is.
    """
    if root is None:
        return deck
    is_dowsing = lambda card: card["id"].removeprefix("CARD.") == "DOWSING"
    due = [row for row, card in enumerate(deck)
           if is_dowsing(card) and _saved_int(card, "RoomsEntered") == 4]
    if not due:
        return deck
    saved = sum(1 for card in deck if is_dowsing(card))
    held = sum(1 for pile in root["piles"].values() for card in pile
               if card.get("id") == "DOWSING")
    if held != saved - len(due):
        return deck
    return ([card for row, card in enumerate(deck) if row not in due]
            + [{"id": "CARD.ABUNDANCE", "upgrade_level": 0} for _ in due])


def first_cycle_from_replay(replay: dict, root: dict | None = None) -> list:
    """Combat-card instance order from the MCR's own entry snapshot.

    `root`, when given, is the Rust root the cycle will be matched against;
    see `combat_deck` for the one thing it changes.
    """

    player = require_single_player(replay["run"], "MCR replay")
    ids = [(card["id"].removeprefix("CARD."),
            card.get("upgrade_level") or
            card.get("current_upgrade_level") or 0)
           for card in combat_deck(player["deck"], root)]
    return [ids[row] for row in first_cycle_rows_from_replay(replay)]


def first_cycle_rows_from_replay(replay: dict) -> list[int]:
    """Master-deck row of each combat-card instance: instance k is row [k].

    ``Rng.shuffle`` is a Fisher-Yates whose swaps depend only on the list
    length, so shuffling the row ordinals applies exactly the permutation
    ``first_cycle_from_replay`` applies to the card ids. Roots that number
    physical cards by master-deck row (Thieving Hopper's DeckVersion uids,
    #2805) need this map rather than instance-index uids.
    """

    run = replay["run"]
    player = require_single_player(run, "MCR replay")
    rows = list(range(len(player["deck"])))
    build = replay["version"]
    rng = Rng(
        RunRngSet(run["rng"]["seed"], build=build)["Shuffle"].seed,
        counter=run["rng"]["counters"]["Shuffle"], build=build)
    rng.shuffle(rows)
    return rows
