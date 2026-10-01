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


def first_cycle_from_replay(replay: dict) -> list:
    """Combat-card instance order from the MCR's own entry snapshot."""

    player = require_single_player(replay["run"], "MCR replay")
    ids = [(card["id"].removeprefix("CARD."),
            card.get("upgrade_level") or
            card.get("current_upgrade_level") or 0)
           for card in player["deck"]]
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
