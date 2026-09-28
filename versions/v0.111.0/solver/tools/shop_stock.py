#!/usr/bin/env python3
"""Predict merchant (shop) relic stock from a current_run.save, PRE-entry.

IL read: v0.110.1 (build db5d3552), RVA cites in ENCOUNTER_MECHANICS.md
"Merchant shop inventory generation". Shop stock is generated once, at
room entry (`MerchantRoom::EnterInternal` 0x5d5c4 -> per player
`MerchantInventory::CreateForNormalMerchant` 0x11fb80), so a save
captured at shop-node arrival predicts the stock exactly.

The three relic slots are [RollRarity, RollRarity, Shop]:

- `RelicFactory::RollRarity` 0x116b41/0x116b54 draws the player
  **Rewards** stream: NextFloat(1.0) < 0.5 -> Common(2),
  < 0.83 -> Uncommon(3), else Rare(4). The third slot is hardcoded
  RelicRarity.Shop(5) (`PopulateRelicEntries` 0x11fd2c).
- Before those two draws, inventory card population consumes exactly
  12 Rewards draws: 5 character cards x (rarity-odds roll 0x65cb0 +
  upgrade roll 0x116848) + 2 colorless cards x (upgrade roll only) —
  validated live on seed TQM88QFMHSQR (draws 13/14 gave the observed
  Uncommon/Uncommon).
- Each slot then pulls `RelicGrabBag::PullFromBack` 0x4eb5c: scan the
  save's per-rarity `relic_id_lists` from the TAIL (last index) down,
  take the first relic passing `IsAllowedInShops` (shops are the only
  tail-pullers; rewards/events pull from the front). Before scanning,
  `RemoveDisallowedRelicsFromDeques` 0x4edac drops IsAllowed==false
  relics from every deque; a deque with no eligible relic downgrades
  Common->Uncommon->Rare->stop, Shop->Common (`GetAvailableDeque`
  0x4ecfc switch), and full exhaustion yields the Circlet fallback
  (0x1169ff). The player bag never refreshes (_refreshAllowed=false,
  parameterless ctor at `Player::CreateForNewRun` 0x11a548).

The Shops stream is also consumed during generation (sale index, card
picks, all prices, potions) but never for relic IDENTITY; relic stock
depends only on the two Rewards draws and bag order.

Usage:
  python3 shop_stock.py SAVE_PATH [--player N]
"""
from __future__ import annotations

import json
import struct
import sys

import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from sts2_rng import MegaRandom  # noqa: E402

# Save schema this model was read+validated against (v0.110.1 capture).
KNOWN_SCHEMA_VERSIONS = {20}

# get_IsAllowedInShops == false overrides (base 0x87c9a returns true):
# AmethystAubergine 0x934ba, BowlerHat 0x94c7e, LuckyFysh 0x99e9a,
# OldCoin 0x9b20e, TheCourier 0x9fe12.
SHOP_DISALLOWED = frozenset({
    "RELIC.AMETHYST_AUBERGINE",
    "RELIC.BOWLER_HAT",
    "RELIC.LUCKY_FYSH",
    "RELIC.OLD_COIN",
    "RELIC.THE_COURIER",
})

# IsAllowed == IsBeforeAct3TreasureChest 0x87e5c:
# TotalFloor < (players > 1 ? 38 : 41).
EGG_FAMILY = frozenset({
    "RELIC.AMETHYST_AUBERGINE",
    "RELIC.BOOK_OF_FIVE_RINGS",
    "RELIC.BOWLER_HAT",
    "RELIC.DRAGON_FRUIT",
    "RELIC.FROZEN_EGG",
    "RELIC.GIRYA",
    "RELIC.JUZU_BRACELET",
    "RELIC.LASTING_CANDY",
    "RELIC.LUCKY_FYSH",
    "RELIC.MEAL_TICKET",
    "RELIC.MOLTEN_EGG",
    "RELIC.OLD_COIN",
    "RELIC.PLANISPHERE",
    "RELIC.SHOVEL",
    "RELIC.TOXIC_EGG",
    "RELIC.WHITE_BEAST_STATUE",
    "RELIC.WHITE_STAR",
})

# IsAllowed player-count gates: MassiveScroll 0x99fca (MP only),
# SilverCrucible 0x9ee8d + WingedBoots 0xa18b2 (SP only).
MP_ONLY = frozenset({"RELIC.MASSIVE_SCROLL"})
SP_ONLY = frozenset({"RELIC.SILVER_CRUCIBLE", "RELIC.WINGED_BOOTS"})

# Rewards draws consumed by card population before the relic rarity
# rolls: 5 character entries x 2 + 2 colorless entries x 1
# (MerchantInventory::.cctor 0x11fe5c arrays).
CARD_REWARDS_DRAWS = 5 * 2 + 2 * 1

RARITY_NAMES = {2: "common", 3: "uncommon", 4: "rare", 5: "shop"}
# GetAvailableDeque 0x4ecfc downgrade switch (rarity -> next rarity;
# 0 = give up -> Circlet fallback).
DOWNGRADE = {2: 3, 3: 4, 4: 0, 5: 2}

FALLBACK_RELIC = "RELIC.CIRCLET"


def _f32(x: float) -> float:
    return struct.unpack("f", struct.pack("f", x))[0]


# RollRarity 0x116b54 thresholds compare float32s.
_T_COMMON = _f32(0.5)
_T_UNCOMMON = _f32(0.8299999833106995)


def _mega_from_state(state: dict) -> MegaRandom:
    """MegaRandom at the save's recorded xoshiro state (the serialized
    s0..s3 are the CURRENT state; no fast-forward needed)."""
    mr = MegaRandom.__new__(MegaRandom)
    mr.s0 = state["s0"]
    mr.s1 = state["s1"]
    mr.s2 = state["s2"]
    mr.s3 = state["s3"]
    return mr


def total_floor(save: dict) -> int:
    """IRunState.TotalFloor at shop entry == 1-based index of the node
    being entered (the same node_index+1 rule the combat entry uses)."""
    return len(save["visited_map_coords"])


def relic_is_allowed(relic_id: str, floor: int, player_count: int) -> bool:
    """RelicModel.IsAllowed(runState) as of v0.110.1 (base 0x87e48
    true; overrides enumerated in module constants)."""
    if relic_id in EGG_FAMILY:
        return floor < (38 if player_count > 1 else 41)
    if relic_id in MP_ONLY:
        return player_count > 1
    if relic_id in SP_ONLY:
        return player_count == 1
    return True


def roll_rarity(mega: MegaRandom) -> int:
    """RelicFactory::RollRarity 0x116b54: one Rewards NextFloat(1.0)."""
    f = _f32(mega.next_double())
    if f < _T_COMMON:
        return 2
    if f < _T_UNCOMMON:
        return 3
    return 4


def pull_from_back(deques: dict[int, list[str]], rarity: int,
                   floor: int, player_count: int) -> tuple[str, int]:
    """RelicGrabBag::PullFromBack 0x4eb5c on save-order deques.

    Mutates `deques` exactly as the game does: IsAllowed==false relics
    are removed from every deque first (0x4edac), the pulled relic is
    RemoveAt'd. Returns (relic_id, rarity_pulled_from); rarity 0 with
    FALLBACK_RELIC when everything is exhausted."""
    for lst in deques.values():
        lst[:] = [r for r in lst
                  if relic_is_allowed(r, floor, player_count)]
    cur = rarity
    while cur:
        deque = deques.get(cur, [])
        # DequeHasAnyRelics with the shop predicate decides downgrade
        if any(r not in SHOP_DISALLOWED for r in deque):
            for i in range(len(deque) - 1, -1, -1):
                if deque[i] not in SHOP_DISALLOWED:
                    return deque.pop(i), cur
        cur = DOWNGRADE[cur]
    return FALLBACK_RELIC, 0


def predict_shop_relics(save: dict, player_index: int = 0) -> list[dict]:
    """The three relic slots the shop will stock when entered, in
    display order: [rolled, rolled, shop-pool]."""
    schema = save.get("schema_version")
    if schema not in KNOWN_SCHEMA_VERSIONS:
        raise NotImplementedError(
            f"save schema_version {schema!r} not validated for shop "
            f"stock prediction (known: {sorted(KNOWN_SCHEMA_VERSIONS)}); "
            "re-verify the IL before trusting predictions "
            "(SOLVER_INVARIANTS.md I5)")
    player = save["players"][player_index]
    player_count = len(save["players"])
    floor = total_floor(save)

    mega = _mega_from_state(player["rng"]["rngs"]["rewards"])
    for _ in range(CARD_REWARDS_DRAWS):
        mega.next_ulong()
    rarities = [roll_rarity(mega), roll_rarity(mega), 5]

    lists = player["relic_grab_bag"]["relic_id_lists"]
    name_to_num = {v: k for k, v in RARITY_NAMES.items()}
    deques = {name_to_num[name]: list(ids)
              for name, ids in lists.items()}

    slots = []
    for slot, rarity in enumerate(rarities):
        relic, pulled_from = pull_from_back(
            deques, rarity, floor, player_count)
        slots.append({
            "slot": slot,
            "rolled_rarity": RARITY_NAMES.get(rarity, str(rarity)),
            "pulled_rarity": RARITY_NAMES.get(pulled_from, "fallback"),
            "relic_id": relic,
        })
    return slots


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        raise SystemExit(2)
    player_index = 0
    if "--player" in sys.argv:
        player_index = int(sys.argv[sys.argv.index("--player") + 1])
    save = json.load(open(args[0]))
    for s in predict_shop_relics(save, player_index):
        note = ("" if s["rolled_rarity"] == s["pulled_rarity"]
                else f" (rolled {s['rolled_rarity']})")
        print(f"slot {s['slot']}: {s['relic_id']}  "
              f"[{s['pulled_rarity']}]{note}")


if __name__ == "__main__":
    main()
