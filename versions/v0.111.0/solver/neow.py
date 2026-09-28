#!/usr/bin/env python3
"""Neow blessing (EVENT.NEOW) option generation.

Predicts the three options Neow offers at the start of a run from a
run-start save, exactly as `Neow::GenerateInitialOptions` computes them.

Every branch below is transcribed from a v0.109.1 sts2.dll IL read; see
versions/v0.111.0/solver/ENCOUNTER_MECHANICS.md ("Neow blessing option generation") for the
RVA citations. Per I5, anything that cannot be read exactly raises
NotImplementedError rather than being approximated.

Key correction to the pre-existing folklore: option generation does NOT
consume the player `Rewards` stream. `EventModel.Rng` is a dedicated
per-event Rng constructed in `EventModel/<BeginEvent>d__28::MoveNext`
(RVA 0x366380) as

    new Rng(RunState.Rng.Seed
            + (IsShared ? 0 : playerSlotIndex)
            + StringHelper.GetDeterministicHashCode(Id.Entry))

so it starts at counter 0 and depends only on the run seed, the player's
slot index, and the literal event entry name "NEOW". That makes the
offered blessings predictable before the event runs, with no dependence
on how much of any other stream has already been consumed.
"""
from __future__ import annotations

from dataclasses import dataclass

from sts2_rng import (GAME_BUILD_V0_109, MASK64, Rng,
                      deterministic_hash_code_v109, seeding_scheme)

# ---------------------------------------------------------------------------
# Option pools — Neow::get_CurseOptions / get_PositiveOptions and the six
# singleton getters. Order is the literal array-init order in the IL, which
# is what UnstableShuffle permutes, so it must be preserved exactly.
# ---------------------------------------------------------------------------

# Neow::get_CurseOptions (RVA 0x276250) — 10 entries.
CURSE_OPTIONS: tuple[str, ...] = (
    "RELIC.CURSED_PEARL",
    "RELIC.DOWSING_ROD",
    "RELIC.HEFTY_TABLET",
    "RELIC.LARGE_CAPSULE",
    "RELIC.LEAFY_POULTICE",
    "RELIC.NEOWS_BONES",
    "RELIC.NEOWS_SACRIFICE",
    "RELIC.PRECARIOUS_SHEARS",
    "RELIC.SILKEN_TRESS",
    "RELIC.SILVER_CRUCIBLE",
)

# Neow::get_PositiveOptions (RVA 0x2760bc) — 14 entries.
POSITIVE_OPTIONS: tuple[str, ...] = (
    "RELIC.ARCANE_SCROLL",
    "RELIC.BOOMING_CONCH",
    "RELIC.FISHING_ROD",
    "RELIC.GOLDEN_PEARL",
    "RELIC.KALEIDOSCOPE",
    "RELIC.LEAD_PAPERWEIGHT",
    "RELIC.LOST_COFFER",
    "RELIC.MASSIVE_SCROLL",
    "RELIC.NEOWS_TORMENT",
    "RELIC.NEW_LEAF",
    "RELIC.PHIAL_HOLSTER",
    "RELIC.PRECISE_SCISSORS",
    "RELIC.SCROLL_BOXES",
    "RELIC.WINGED_BOOTS",
)

# The three coin-flip pairs appended to the positive pool. Each is one
# Rng.NextBool: True picks the first member, False the second.
LAVA_ROCK = "RELIC.LAVA_ROCK"
SMALL_CAPSULE = "RELIC.SMALL_CAPSULE"
NUTRITIOUS_OYSTER = "RELIC.NUTRITIOUS_OYSTER"
STONE_HUMIDIFIER = "RELIC.STONE_HUMIDIFIER"
NEOWS_TALISMAN = "RELIC.NEOWS_TALISMAN"
POMANDER = "RELIC.POMANDER"

# Neow::GenerateInitialOptions (RVA 0x2766e0), predicates b__35_1..b__35_6:
# drawing a given curse strikes its thematic counterpart from the positives.
CURSE_EXCLUSIONS: dict[str, tuple[str, ...]] = {
    "RELIC.CURSED_PEARL": ("RELIC.GOLDEN_PEARL",),          # b__35_1
    "RELIC.HEFTY_TABLET": ("RELIC.ARCANE_SCROLL",),         # b__35_2
    "RELIC.LEAFY_POULTICE": ("RELIC.NEW_LEAF",),            # b__35_3
    "RELIC.PRECARIOUS_SHEARS": ("RELIC.PRECISE_SCISSORS",),  # b__35_4
    # b__35_5 and b__35_6 are BOTH inside the NeowsSacrifice branch.
    "RELIC.NEOWS_SACRIFICE": ("RELIC.PHIAL_HOLSTER", "RELIC.LOST_COFFER"),
}

# Drawing Large Capsule skips the Lava Rock / Small Capsule coin flip
# entirely — one fewer draw off the event stream.
LARGE_CAPSULE = "RELIC.LARGE_CAPSULE"

# The five characters ModelDb::get_AllCharacters returns (RVA 0x22d276) and
# the epoch that reveals each, per UnlockState::get_Characters (RVA 0x114b0).
# Ironclad has no gate — it is always present.
CHARACTER_UNLOCK_EPOCHS: tuple[str, ...] = (
    "SILENT1_EPOCH",
    "REGENT1_EPOCH",
    "NECROBINDER1_EPOCH",
    "DEFECT1_EPOCH",
)
ALL_CHARACTER_COUNT = 5


@dataclass(frozen=True)
class NeowOptions:
    """The three options Neow offers, in the order GenerateInitialOptions
    returns them: two positives (the first two of the shuffled pool), then
    the curse."""

    positives: tuple[str, str]
    curse: str
    draws: int  # event-stream draws consumed producing this result

    @property
    def all_options(self) -> tuple[str, str, str]:
        return (self.positives[0], self.positives[1], self.curse)

    def __str__(self) -> str:
        return (f"{self.positives[0]} | {self.positives[1]} "
                f"| {self.curse} (curse)")


def event_rng(seed_string: str, *, slot_index: int = 0,
              entry: str = "NEOW",
              build: str = GAME_BUILD_V0_109) -> Rng:
    """The dedicated per-event Rng, at counter 0.

    EventModel/<BeginEvent>d__28::MoveNext (RVA 0x366380):
        new Rng(RunState.Rng.Seed + slotIndex + hash(Id.Entry))
    EventModel::get_IsShared (RVA 0x22c1b9) returns false and Neow does not
    override it, so the slot index is always added.
    Rng::.ctor(ulong) (RVA 0x61b81) seeds MegaRandom directly, counter 0.
    """
    if seeding_scheme(build) != "v109":
        raise NotImplementedError(
            f"Neow option generation has only been IL-read against v0.109 "
            f"(sts2.dll v0.109.1); build {build!r} uses the "
            f"{seeding_scheme(build)!r} seeding scheme and must be "
            f"re-verified before its options can be trusted")
    run_seed = deterministic_hash_code_v109(seed_string)
    seed = (run_seed + slot_index
            + deterministic_hash_code_v109(entry)) & MASK64
    return Rng(seed, 0, build=build)


def _is_allowed_at_neow(relic_id: str, *, player_count: int,
                        unlocked_epochs: frozenset[str],
                        scroll_boxes_allowed: bool | None) -> bool:
    """RelicModel::IsAllowedAtNeow (RVA 0x2309e7) = IsAllowed(RunState),
    plus the two relics that override IsAllowedAtNeow itself.

    RelicModel::IsAllowed (RVA 0x2309e4) is `return true`; only three
    relics in the Neow pools override it, all on player count.
    """
    # SilverCrucible::IsAllowed (0x24794d), WingedBoots::IsAllowed (0x24a372)
    if relic_id in ("RELIC.SILVER_CRUCIBLE", "RELIC.WINGED_BOOTS"):
        return player_count == 1
    # MassiveScroll::IsAllowed (0x242b2e)
    if relic_id == "RELIC.MASSIVE_SCROLL":
        return player_count > 1
    # Kaleidoscope::IsAllowedAtNeow (0x241858): base && every character's
    # card pool is unlocked, i.e. UnlockState.Characters covers all five.
    if relic_id == "RELIC.KALEIDOSCOPE":
        unlocked_chars = 1 + sum(  # Ironclad is ungated
            1 for epoch in CHARACTER_UNLOCK_EPOCHS
            if epoch in unlocked_epochs)
        return unlocked_chars == ALL_CHARACTER_COUNT
    # ScrollBoxes::IsAllowedAtNeow (0x246e96) = CanGenerateBundles(player)
    # && base. CanGenerateBundles (0x246ef0) needs the character's
    # epoch-filtered, multiplayer-constrained card pool to hold >= 4 cards
    # of rarity 2 and >= 2 of rarity 3. That pool is not recorded in the
    # save and reconstructing it needs the full per-character card table
    # (the #138 census), so this one must be supplied by the caller.
    if relic_id == "RELIC.SCROLL_BOXES":
        if scroll_boxes_allowed is None:
            raise NotImplementedError(
                "ScrollBoxes' Neow eligibility depends on "
                "ScrollBoxes::CanGenerateBundles (RVA 0x246ef0): the "
                "character's unlocked card pool must hold >= 4 rarity-2 and "
                ">= 2 rarity-3 cards. The save does not record the card "
                "pool, so pass scroll_boxes_allowed=True/False explicitly "
                "once it is known for the account being predicted.")
        return scroll_boxes_allowed
    return True


def generate_neow_options(seed_string: str, *,
                          slot_index: int = 0,
                          player_count: int = 1,
                          unlocked_epochs: frozenset[str] | set[str]
                          | list[str] | tuple[str, ...] = (),
                          scroll_boxes_allowed: bool | None = None,
                          modifiers: tuple = (),
                          build: str = GAME_BUILD_V0_109) -> NeowOptions:
    """Reproduce Neow::GenerateInitialOptions (RVA 0x2766e0).

    Draw order off the dedicated event stream:
      1. NextItem over the allowed curse pool                     (1 draw)
      2. Lava Rock / Small Capsule NextBool  -- SKIPPED entirely
         when the curse drawn is Large Capsule                (0 or 1 draw)
      3. Nutritious Oyster / Stone Humidifier NextBool            (1 draw)
      4. Neow's Talisman / Pomander NextBool                      (1 draw)
      5. UnstableShuffle over the whole positive pool     (len - 1 draws)
    then Take(2) from the shuffled positives and append the curse.
    """
    if modifiers:
        # GenerateInitialOptions branches to an entirely different path when
        # RunState.Modifiers is non-empty: it offers ModifierModel-generated
        # options instead and consumes no RNG at all. Not modeled.
        raise NotImplementedError(
            "run modifiers are set: Neow::GenerateInitialOptions takes the "
            "ModifierModel::GenerateNeowOption path (IL_0248+), which offers "
            "modifier options instead of blessings and is not modeled")

    epochs = frozenset(unlocked_epochs)
    rng = event_rng(seed_string, slot_index=slot_index, build=build)
    start = rng.counter

    def allowed(relic_id: str) -> bool:
        return _is_allowed_at_neow(
            relic_id, player_count=player_count, unlocked_epochs=epochs,
            scroll_boxes_allowed=scroll_boxes_allowed)

    # curses.RemoveAll(b__35_0) then Rng.NextItem(curses)
    curses = [r for r in CURSE_OPTIONS if allowed(r)]
    if not curses:
        raise NotImplementedError(
            "every curse option was filtered out; Rng.NextItem would return "
            "null and the event's behaviour past that point is unread")
    curse = rng.next_item(curses)

    # positives start unfiltered; the curse-specific exclusions run first.
    positives = list(POSITIVE_OPTIONS)
    for excluded in CURSE_EXCLUSIONS.get(curse, ()):
        positives = [r for r in positives if r != excluded]

    # Three appended coin flips. The first is skipped when the curse is
    # Large Capsule (IL_0184 `isinst LargeCapsule` / `brtrue`).
    if curse != LARGE_CAPSULE:
        positives.append(LAVA_ROCK if rng.next_bool() else SMALL_CAPSULE)
    positives.append(
        NUTRITIOUS_OYSTER if rng.next_bool() else STONE_HUMIDIFIER)
    positives.append(NEOWS_TALISMAN if rng.next_bool() else POMANDER)

    # positives.RemoveAll(b__35_7) — runs AFTER the appends, so the coin-flip
    # relics are subject to the same Neow-eligibility filter.
    positives = [r for r in positives if allowed(r)]
    if len(positives) < 2:
        raise NotImplementedError(
            f"only {len(positives)} positive option(s) survived filtering; "
            f"Take(2) would yield a short list and the UI's behaviour there "
            f"is unread")

    rng.shuffle(positives)  # ListExtensions::UnstableShuffle (RVA 0x2bf170)
    chosen = positives[:2]

    return NeowOptions(positives=(chosen[0], chosen[1]), curse=curse,
                       draws=rng.counter - start)


def options_from_save(save: dict, *,
                      scroll_boxes_allowed: bool | None = None,
                      slot_index: int = 0,
                      build: str = GAME_BUILD_V0_109) -> NeowOptions:
    """Convenience wrapper: pull every input out of a parsed run-start save."""
    player = save["players"][slot_index]
    unlock = player.get("unlock_state") or {}
    return generate_neow_options(
        save["rng"]["seed"],
        slot_index=slot_index,
        player_count=len(save["players"]),
        unlocked_epochs=frozenset(unlock.get("unlocked_epochs") or ()),
        scroll_boxes_allowed=scroll_boxes_allowed,
        modifiers=tuple(save.get("modifiers") or ()),
        build=build,
    )


if __name__ == "__main__":
    import json
    import sys

    if len(sys.argv) < 2:
        sys.exit("usage: neow.py SAVE.json [--scroll-boxes 0|1]")
    with open(sys.argv[1]) as fh:
        save_data = json.load(fh)
    sb: bool | None = None
    if "--scroll-boxes" in sys.argv:
        sb = bool(int(sys.argv[sys.argv.index("--scroll-boxes") + 1]))
    result = options_from_save(save_data, scroll_boxes_allowed=sb)
    print(f"seed {save_data['rng']['seed']}")
    print(f"  {result}")
    print(f"  ({result.draws} event-stream draws)")
