#!/usr/bin/env python3
"""Exact v0.111.0 map-layer Rest and Whetstone predictions.

Authority is the installed/archive-identical v0.111.0 ``sts2.dll``
(SHA-256 ``9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4``).
The cited CIL and full contract live in ``ENCOUNTER_MECHANICS.md`` under
"Map-layer Rest and Whetstone prediction".

This tool deliberately accepts only an explicitly identified, current-build
save captured immediately before the predicted action.  A historical run,
post-pickup save, inferred Niche counter, or adjacent game build is not a
substitute for that provenance.

Usage:
  python3 tools/map_prediction.py SAVE --build v0.111.0 \
      --provenance pre-rest-choice-save --rest [--player N]
  python3 tools/map_prediction.py SAVE --build v0.111.0 \
      --provenance pre-whetstone-pickup-save --whetstone [--player N]
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import sys
from typing import Any

SOLVER = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SOLVER))

from dotnet_sort import dotnet_list_sort  # noqa: E402
from sts2_rng import MegaRandom  # noqa: E402

GAME_BUILD = "v0.111.0"
SAVE_SCHEMA = 20
REST_PROVENANCE = "pre-rest-choice-save"
WHETSTONE_PROVENANCE = "pre-whetstone-pickup-save"
IL_EVIDENCE = (
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

_CARDS_CENSUS = SOLVER / "cards_census.json"
_CARD_TEMPLATES = SOLVER / "card_templates.json"
_RELICS_CENSUS = SOLVER / "relics_census.json"
_RNG_STATE_KEYS = frozenset({"counter", "s0", "s1", "s2", "s3"})
_U64_MAX = (1 << 64) - 1


def _refuse(message: str) -> None:
    raise NotImplementedError(
        f"{message} (SOLVER_INVARIANTS.md I5; issue #806)")


def _require_snapshot(save: dict[str, Any], *, build: str,
                      provenance: str, expected_provenance: str) -> None:
    if build != GAME_BUILD:
        _refuse(
            f"map prediction requires exact build {GAME_BUILD}, got {build!r}")
    if provenance != expected_provenance:
        _refuse(
            f"map prediction requires provenance {expected_provenance!r}, "
            f"got {provenance!r}")
    if not isinstance(save, dict) or save.get("schema_version") != SAVE_SCHEMA:
        schema = save.get("schema_version") if isinstance(save, dict) else None
        _refuse(
            f"map prediction requires save schema {SAVE_SCHEMA}, got {schema!r}")
    if not isinstance(save.get("players"), list) or not save["players"]:
        _refuse("map prediction requires a nonempty exact players array")


def _player(save: dict[str, Any], player_index: int) -> dict[str, Any]:
    if type(player_index) is not int or not 0 <= player_index < len(save["players"]):
        _refuse(f"player index {player_index!r} is outside the saved roster")
    player = save["players"][player_index]
    if not isinstance(player, dict):
        _refuse(f"player {player_index} is not a structured save row")
    return player


def _current_relic_ids(player: dict[str, Any],
                       relics_census: dict[str, Any]) -> tuple[str, ...]:
    rows = player.get("relics")
    if not isinstance(rows, list):
        _refuse("Rest prediction requires the exact saved relic array")
    ids = []
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("id"), str):
            _refuse(f"Rest prediction found malformed relic row {row!r}")
        relic_id = row["id"]
        if relic_id not in relics_census:
            _refuse(
                f"Rest prediction lacks current-build hook provenance for "
                f"{relic_id!r}")
        ids.append(relic_id)
    return tuple(ids)


def predict_rest(save: dict[str, Any], *, build: str, provenance: str,
                 player_index: int = 0,
                 relics_census: dict[str, Any] | None = None) -> dict[str, Any]:
    """Return the exact ordinary Rest heal request and resulting HP.

    ``HealRestSiteOption.GetBaseHealAmount`` produces Decimal(MaxHp) * 0.3.
    ``Creature.SetCurrentHpInternal`` clamps to MaxHp and explicitly converts
    the resulting Decimal to int, truncating the positive fractional tail.

    Regal Pillow and any run modifier are refused.  The current DLL has two
    heal-amount overrides (Regal Pillow and Night Terrors), but composing
    those listeners is outside this intentionally ordinary-Rest seam.
    """
    _require_snapshot(
        save, build=build, provenance=provenance,
        expected_provenance=REST_PROVENANCE)
    player = _player(save, player_index)
    max_hp = player.get("max_hp")
    current_hp = player.get("current_hp")
    if (type(max_hp) is not int or type(current_hp) is not int
            or max_hp <= 0 or not 0 <= current_hp <= max_hp):
        _refuse(
            f"Rest prediction requires exact integral HP, got "
            f"{current_hp!r}/{max_hp!r}")

    if relics_census is None:
        relics_census = json.loads(_RELICS_CENSUS.read_text())
    relic_ids = _current_relic_ids(player, relics_census)
    if "RELIC.REGAL_PILLOW" in relic_ids:
        _refuse(
            "Rest prediction does not compose Regal Pillow's +15 heal hook")
    modifiers = save.get("modifiers")
    if not isinstance(modifiers, list):
        _refuse("Rest prediction requires the exact saved modifier array")
    if modifiers:
        names = []
        for row in modifiers:
            if isinstance(row, str):
                names.append(row)
            elif isinstance(row, dict) and isinstance(row.get("id"), str):
                names.append(row["id"])
            else:
                names.append(repr(row))
        detail = ("Night Terrors" if any(
            name in {"NIGHT_TERRORS", "MODIFIER.NIGHT_TERRORS"}
            for name in names) else "nonempty modifier composition")
        _refuse(f"Rest prediction does not compose {detail}: {names!r}")

    # Decimal(MaxHp) * Decimal("0.3") is exact.  The requested value retains
    # one decimal place; SetCurrentHpInternal's Decimal->int conversion then
    # truncates after adding it to integral current HP.
    tenths = max_hp * 3
    requested = f"{tenths // 10}.{tenths % 10}"
    hp_after = min(max_hp, (current_hp * 10 + tenths) // 10)
    return {
        "build": GAME_BUILD,
        "player_index": player_index,
        "requested_heal_decimal": requested,
        "hp_gained": hp_after - current_hp,
        "hp_after": hp_after,
    }


def _card_level(row: dict[str, Any]) -> int:
    if "upgrade_level" in row:
        _refuse(
            "Whetstone requires save key current_upgrade_level, not "
            "historical/replay key upgrade_level")
    value = row.get("current_upgrade_level", 0)
    if type(value) is not int or value < 0:
        _refuse(f"Whetstone found malformed current_upgrade_level {value!r}")
    return value


def _niche_random(save: dict[str, Any]) -> tuple[MegaRandom, int]:
    rng = save.get("rng")
    if not isinstance(rng, dict):
        _refuse("Whetstone requires the exact serialized RNG container")
    rngs = rng.get("rngs")
    if not isinstance(rngs, dict) or "niche" not in rngs:
        _refuse("Whetstone requires the exact serialized Niche RNG state")
    state = rngs["niche"]
    if not isinstance(state, dict) or set(state) != _RNG_STATE_KEYS:
        _refuse(
            f"Whetstone Niche state must contain exactly "
            f"{sorted(_RNG_STATE_KEYS)}, got {state!r}")
    counter = state["counter"]
    if type(counter) is not int or counter < 0:
        _refuse(f"Whetstone Niche counter is not exact: {counter!r}")
    words = []
    for key in ("s0", "s1", "s2", "s3"):
        value = state[key]
        if type(value) is not int or not 0 <= value <= _U64_MAX:
            _refuse(f"Whetstone Niche {key} is not a uint64: {value!r}")
        words.append(value)
    random = MegaRandom.__new__(MegaRandom)
    random.s0, random.s1, random.s2, random.s3 = words
    return random, counter


def _shuffle(items: list[Any], random: MegaRandom) -> int:
    draws = 0
    for index in range(len(items) - 1, 0, -1):
        # Rng.NextInt(index + 1) is exactly int(NextDouble() * bound).
        selected = int(random.next_double() * (index + 1))
        items[selected], items[index] = items[index], items[selected]
        draws += 1
    return draws


def predict_whetstone(
        save: dict[str, Any], *, build: str, provenance: str,
        player_index: int = 0,
        cards_census: dict[str, Any] | None = None,
        max_upgrades: dict[str, Any] | None = None) -> dict[str, Any]:
    """Predict Whetstone targets and return an upgraded deck copy.

    ``target_deck_indexes`` are physical identities: positions in the exact
    serialized pre-pickup deck.  Sorting and shuffling operate on references
    to those indexed rows; the returned deck remains in its original order,
    with only each selected row's ``current_upgrade_level`` incremented.
    """
    _require_snapshot(
        save, build=build, provenance=provenance,
        expected_provenance=WHETSTONE_PROVENANCE)
    player = _player(save, player_index)
    deck = player.get("deck")
    if not isinstance(deck, list):
        _refuse("Whetstone requires the exact ordered serialized deck")
    if cards_census is None:
        cards_census = json.loads(_CARDS_CENSUS.read_text())
    if max_upgrades is None:
        max_upgrades = json.loads(_CARD_TEMPLATES.read_text())["max_upgrade"]

    eligible = []
    for index, row in enumerate(deck):
        if not isinstance(row, dict) or not isinstance(row.get("id"), str):
            _refuse(f"Whetstone found malformed deck row {index}: {row!r}")
        card_id = row["id"]
        metadata = cards_census.get(card_id)
        max_level = max_upgrades.get(card_id)
        if not isinstance(metadata, dict) or metadata.get("type") not in {
                -1, "attack", "skill", "power", "status", "curse",
                "quest"}:
            _refuse(
                f"Whetstone lacks current-build CardType for deck row "
                f"{index} {card_id!r}")
        if type(max_level) is not int or max_level < 0:
            _refuse(
                f"Whetstone lacks current-build MaxUpgradeLevel for deck "
                f"row {index} {card_id!r}")
        level = _card_level(row)
        if level > max_level:
            _refuse(
                f"Whetstone deck row {index} exceeds MaxUpgradeLevel: "
                f"{card_id!r} {level}>{max_level}")
        if metadata["type"] == "attack" and level < max_level:
            eligible.append((index, row, level))

    random, counter_before = _niche_random(save)
    # CardModel.CompareTo compares ModelId then current upgrade level.  All
    # candidates are CardModel, so the common "CARD." category is irrelevant;
    # current built-in entries are ASCII and their exact census spelling is
    # the ordinal comparison key.  The existing port retains .NET 9 tie order.
    shuffled = dotnet_list_sort(
        eligible, key=lambda item: (item[1]["id"], item[2]))
    draws = _shuffle(shuffled, random)
    selected = shuffled[:2]

    upgraded_deck = copy.deepcopy(deck)
    targets = []
    for index, row, level in selected:
        upgraded_deck[index]["current_upgrade_level"] = level + 1
        targets.append({
            "deck_index": index,
            "card_id": row["id"],
            "upgrade_level_before": level,
            "upgrade_level_after": level + 1,
        })
    return {
        "build": GAME_BUILD,
        "player_index": player_index,
        "eligible_count": len(eligible),
        "niche_draws": draws,
        "niche_counter_before": counter_before,
        "niche_counter_after": counter_before + draws,
        "niche_state_after": {
            "counter": counter_before + draws,
            "s0": random.s0,
            "s1": random.s1,
            "s2": random.s2,
            "s3": random.s3,
        },
        "target_deck_indexes": [row["deck_index"] for row in targets],
        "targets": targets,
        "deck_after": upgraded_deck,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("save", type=Path)
    parser.add_argument("--build", required=True)
    parser.add_argument("--provenance", required=True)
    parser.add_argument("--player", type=int, default=0)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--rest", action="store_true")
    mode.add_argument("--whetstone", action="store_true")
    args = parser.parse_args()
    save = json.loads(args.save.read_text())
    if args.rest:
        result = predict_rest(
            save, build=args.build, provenance=args.provenance,
            player_index=args.player)
    else:
        result = predict_whetstone(
            save, build=args.build, provenance=args.provenance,
            player_index=args.player)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
