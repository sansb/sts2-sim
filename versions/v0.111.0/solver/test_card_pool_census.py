"""#667: checked current-build character-card pool census for Splash."""
import hashlib
import json
from pathlib import Path

from attestations import attested


ROOT = Path(__file__).parent
CENSUS_PATH = ROOT / "card_pool_census.json"
PROBE_PATH = ROOT / "harness" / "probe_card_pools.py"
CARDS_CENSUS_PATH = ROOT / "cards_census.json"
COVERAGE_PATH = ROOT / "coverage_matrix.json"

EXPECTED_CENSUS_SHA256 = attested("card_pool_census.json")
EXPECTED_PROBE_SHA256 = attested("harness/probe_card_pools.py")

with CENSUS_PATH.open() as _fh:
    CENSUS = json.load(_fh)
with CARDS_CENSUS_PATH.open() as _fh:
    CARDS_CENSUS = json.load(_fh)
with COVERAGE_PATH.open() as _fh:
    COVERAGE = json.load(_fh)


def test_card_pool_census_and_probe_are_exactly_attested():
    """Pool order is RNG-significant, so partial row assertions are unsafe."""
    assert hashlib.sha256(CENSUS_PATH.read_bytes()).hexdigest() == \
        EXPECTED_CENSUS_SHA256
    assert hashlib.sha256(PROBE_PATH.read_bytes()).hexdigest() == \
        EXPECTED_PROBE_SHA256


def test_card_pool_census_has_current_build_provenance():
    assert CENSUS["schema"] == 1
    assert CENSUS["game_build"] == {
        "version": "v0.111.0",
        "commit": "41cef1ea",
        "sts2_dll_sha256":
            "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4",
    }
    assert CENSUS["source"]["kind"] == "headless-engine-oracle"
    assert CENSUS["source"]["probe"] == \
        "versions/v0.111.0/solver/harness/probe_card_pools.py"
    assert CENSUS["source"]["rvas"]["splash_on_play"] == "0x3bba6c"


def test_all_character_pool_order_sizes_indices_and_unlock_epochs():
    expected_order = [
        "IRONCLAD", "SILENT", "REGENT", "NECROBINDER", "DEFECT"]
    expected_sizes = {
        "IRONCLAD": 90,
        "SILENT": 91,
        "REGENT": 91,
        "NECROBINDER": 91,
        "DEFECT": 91,
    }
    assert CENSUS["character_pool_order"] == expected_order
    for character in expected_order:
        rows = CENSUS["pools"][character]["cards"]
        epochs = set(CENSUS["character_unlock_epochs"][character])
        assert len(rows) == expected_sizes[character]
        assert [row["index"] for row in rows] == list(range(len(rows)))
        assert len({row["id"] for row in rows}) == len(rows)
        assert {row["unlock_epoch"] for row in rows} <= epochs | {None}
    # #674 corrected the older constructor scanner.  Keep this whole-pool
    # comparison strict so a future census/live-engine disagreement cannot
    # silently reappear.
    type_mismatches = {
        row["id"]: (CARDS_CENSUS[row["id"]]["type"].title(), row["type"])
        for character in expected_order
        for row in CENSUS["pools"][character]["cards"]
        if CARDS_CENSUS[row["id"]]["type"].title() != row["type"]
    }
    assert type_mismatches == {}


def test_solo_ironclad_splash_pool_cardinality_order_and_routes():
    splash = CENSUS["splash_from_solo_ironclad"]
    cards = splash["cards"]
    assert splash[
        "owner_pool_removed_when_character_pool_count_gt_one"] == "IRONCLAD"
    assert not {row["character"] for row in cards} & {"IRONCLAD"}
    assert splash["filtered_pool_size"] == len(cards) == 112
    assert splash["combat_card_generation_draws"] == 111
    assert [
        (character, sum(row["character"] == character for row in cards))
        for character in ("SILENT", "REGENT", "NECROBINDER", "DEFECT")
    ] == [
        ("SILENT", 23),
        ("REGENT", 29),
        ("NECROBINDER", 32),
        ("DEFECT", 28),
    ]

    refused = {
        row["id"]: row["blocker_issues"]
        for row in cards if row["status"] == "refused"
    }
    assert refused == {}
    assert sum(row["status"] == "modeled" for row in cards) == 112
    assert all(
        bool(row["blocker_issues"]) == (row["status"] == "refused")
        for row in cards)
    assert all(
        COVERAGE["categories"]["card"][row["id"]]["status"] == row["status"]
        for row in cards)


def test_single_character_edge_retains_the_owner_pool():
    """Splash's count<=1 branch skips Remove(owner), rather than going empty."""
    edge = CENSUS["splash_from_solo_ironclad"]["single_pool_edge"]
    assert edge["owner_pool_retained"] is True
    assert edge["filtered_pool_size"] == len(edge["cards"]) == 33
    assert edge["combat_card_generation_draws"] == 32
    assert edge["cards"][0] == "CARD.ANGER"
    assert edge["cards"][-1] == "CARD.WHIRLWIND"


def test_the_hunt_is_refused_but_not_reachable_from_splash():
    excluded = {
        row["id"]: row
        for row in CENSUS["splash_from_solo_ironclad"][
            "excluded_attacks"]
    }
    assert excluded["CARD.THE_HUNT"]["reasons"] == [
        "CanBeGeneratedInCombat=false"]
    assert "CARD.THE_HUNT" not in {
        row["id"]
        for row in CENSUS["splash_from_solo_ironclad"]["cards"]
    }
