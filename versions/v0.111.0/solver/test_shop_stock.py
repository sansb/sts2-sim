"""Shop relic stock prediction (tools/shop_stock.py) — v0.110.1 IL read.

Pins the merchant relic-slot generation model against the live capture
from seed TQM88QFMHSQR (2026-07-31, v0.110.1 db5d3552): the first shop
offered Orichalcum + Parrying Shield (the last two entries of the
player uncommon grab-bag list) and Bread (last entry of the shop
list) — tail draws via RelicGrabBag::PullFromBack 0x4eb5c, rarities
Uncommon/Uncommon from Rewards draws 13/14 (12 card draws precede
them). RVA cites: ENCOUNTER_MECHANICS.md "Merchant shop inventory
generation".
"""
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "tools"))

import shop_stock  # noqa: E402

SAVE = HERE / "testdata" / "TQM88QFMHSQR_shop_entry.save"


def load_save():
    return json.load(open(SAVE))


def test_capture_pin_first_shop_stock():
    """Live-validated: predicted stock == observed in-game stock."""
    slots = shop_stock.predict_shop_relics(load_save())
    assert [s["relic_id"] for s in slots] == [
        "RELIC.ORICHALCUM", "RELIC.PARRYING_SHIELD", "RELIC.BREAD"]
    assert [s["rolled_rarity"] for s in slots] == [
        "uncommon", "uncommon", "shop"]
    assert [s["pulled_rarity"] for s in slots] == [
        "uncommon", "uncommon", "shop"]


def test_capture_pin_pulls_are_tail_draws():
    """The predicted relics are the TAILS of the save's bag lists
    (positions 32/31 of 32 uncommon, 26 of 26 shop) — the head stays
    untouched for future front-pulling reward draws."""
    save = load_save()
    lists = save["players"][0]["relic_grab_bag"]["relic_id_lists"]
    assert lists["uncommon"][-1] == "RELIC.ORICHALCUM"
    assert lists["uncommon"][-2] == "RELIC.PARRYING_SHIELD"
    assert lists["shop"][-1] == "RELIC.BREAD"
    assert len(lists["uncommon"]) == 32
    assert len(lists["shop"]) == 26


def test_rewards_draw_accounting():
    """Cards consume 5*2 + 2*1 = 12 Rewards draws before the two relic
    rarity rolls (MerchantInventory::.cctor 0x11fe5c arrays; odds roll
    0x65cb0 + upgrade roll 0x116848 per character card, upgrade only
    per colorless card)."""
    assert shop_stock.CARD_REWARDS_DRAWS == 12
    # And with the capture's recorded rewards state, draws 13/14 land
    # in [0.5, 0.83) — the two Uncommon slots seen in-game.
    save = load_save()
    mega = shop_stock._mega_from_state(
        save["players"][0]["rng"]["rngs"]["rewards"])
    for _ in range(shop_stock.CARD_REWARDS_DRAWS):
        mega.next_ulong()
    r1 = shop_stock.roll_rarity(mega)
    r2 = shop_stock.roll_rarity(mega)
    assert (r1, r2) == (3, 3)


def test_rarity_thresholds():
    """RollRarity 0x116b54: < 0.5 Common(2), < 0.83 Uncommon(3),
    else Rare(4)."""

    class FakeMega:
        def __init__(self, v):
            self.v = v

        def next_double(self):
            return self.v

    assert shop_stock.roll_rarity(FakeMega(0.0)) == 2
    assert shop_stock.roll_rarity(FakeMega(0.49999)) == 2
    assert shop_stock.roll_rarity(FakeMega(0.5)) == 3
    assert shop_stock.roll_rarity(FakeMega(0.8299)) == 3
    assert shop_stock.roll_rarity(FakeMega(0.83)) == 4
    assert shop_stock.roll_rarity(FakeMega(0.99)) == 4


def test_pull_skips_shop_disallowed_relics():
    """IsAllowedInShops==false relics (e.g. Old Coin 0x9b20e) are
    SKIPPED by the tail scan but stay in the deque."""
    deques = {4: ["RELIC.MANGO", "RELIC.ART_OF_WAR", "RELIC.OLD_COIN"]}
    relic, pulled = shop_stock.pull_from_back(deques, 4, floor=5,
                                              player_count=1)
    assert relic == "RELIC.ART_OF_WAR"
    assert pulled == 4
    assert deques[4] == ["RELIC.MANGO", "RELIC.OLD_COIN"]


def test_pull_downgrade_chain_shop_to_common():
    """GetAvailableDeque 0x4ecfc: an exhausted Shop deque downgrades
    Shop->Common (switch {2->3, 3->4, 4->stop, 5->2})."""
    deques = {5: [], 2: ["RELIC.LANTERN"], 3: [], 4: []}
    relic, pulled = shop_stock.pull_from_back(deques, 5, floor=5,
                                              player_count=1)
    assert relic == "RELIC.LANTERN"
    assert pulled == 2


def test_pull_exhausted_yields_circlet_fallback():
    """Everything empty -> RelicFactory fallback Circlet 0x1169ff."""
    deques = {2: [], 3: [], 4: [], 5: []}
    relic, pulled = shop_stock.pull_from_back(deques, 5, floor=5,
                                              player_count=1)
    assert relic == shop_stock.FALLBACK_RELIC
    assert pulled == 0


def test_egg_family_removed_at_floor_41_singleplayer():
    """RemoveDisallowedRelicsFromDeques 0x4edac drops relics whose
    IsAllowed is false: egg family allowed iff TotalFloor < 41 (SP)
    / 38 (MP) via IsBeforeAct3TreasureChest 0x87e5c. Removal is a
    deque MUTATION (persists), unlike the shop-predicate skip."""
    deques = {3: ["RELIC.PERMAFROST", "RELIC.FROZEN_EGG"]}
    relic, _ = shop_stock.pull_from_back(deques, 3, floor=40,
                                         player_count=1)
    assert relic == "RELIC.FROZEN_EGG"

    deques = {3: ["RELIC.PERMAFROST", "RELIC.FROZEN_EGG"]}
    relic, _ = shop_stock.pull_from_back(deques, 3, floor=41,
                                         player_count=1)
    assert relic == "RELIC.PERMAFROST"
    assert deques[3] == []

    deques = {3: ["RELIC.PERMAFROST", "RELIC.FROZEN_EGG"]}
    relic, _ = shop_stock.pull_from_back(deques, 3, floor=38,
                                         player_count=2)
    assert relic == "RELIC.PERMAFROST"


def test_unknown_schema_version_refuses():
    """I5: an unvalidated save schema raises NotImplementedError
    instead of guessing."""
    save = load_save()
    save["schema_version"] = 99
    try:
        shop_stock.predict_shop_relics(save)
    except NotImplementedError:
        pass
    else:
        raise AssertionError("expected NotImplementedError")


def test_sequential_pulls_share_deque_state():
    """Two same-rarity slots pull DIFFERENT relics: the first pull's
    RemoveAt persists into the second (observed live: slots 0/1 took
    uncommon positions 32 then 31)."""
    slots = shop_stock.predict_shop_relics(load_save())
    assert slots[0]["relic_id"] != slots[1]["relic_id"]
