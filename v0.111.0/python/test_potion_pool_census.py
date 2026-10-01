"""#2056: checked current-build Alchemize potion pool/RNG/belt census.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""
import hashlib
import json
from pathlib import Path

from sts2_rng import RunRngSet

from attestations import attested


ROOT = Path(__file__).parent
CENSUS_PATH = ROOT / "potion_pool_census.json"
PROBE_PATH = ROOT / "harness" / "probe_potion_generation.py"
# Frozen with the Python simulator (#2827): the probe still reads its statuses.
COVERAGE_PATH = ROOT / "coverage_matrix.json"

EXPECTED_CENSUS_SHA256 = attested("potion_pool_census.json")
EXPECTED_PROBE_SHA256 = attested("harness/probe_potion_generation.py")

with CENSUS_PATH.open() as _fh:
    CENSUS = json.load(_fh)
with COVERAGE_PATH.open() as _fh:
    COVERAGE = json.load(_fh)


def test_potion_pool_census_and_probe_are_exactly_attested():
    """Pool order and generator code are RNG-significant: pin them whole."""
    assert hashlib.sha256(CENSUS_PATH.read_bytes()).hexdigest() == \
        EXPECTED_CENSUS_SHA256
    assert hashlib.sha256(PROBE_PATH.read_bytes()).hexdigest() == \
        EXPECTED_PROBE_SHA256


def test_potion_pool_census_has_current_build_provenance():
    assert CENSUS["schema"] == 1
    assert CENSUS["game_build"] == {
        "version": "v0.111.0",
        "commit": "41cef1ea",
        "sts2_dll_sha256":
            "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4",
    }
    assert CENSUS["source"]["kind"] == "headless-engine-oracle"
    assert CENSUS["source"]["probe"] == \
        "sim/v0.111.0/python/harness/probe_potion_generation.py"
    assert CENSUS["source"]["rvas"] == {
        "alchemize_on_play": "0xd7988",
        "alchemize_on_play_move_next": "0x3897c8",
        "ironclad_pool_generate_all": "0xadd30",
        "ironclad_pool_get_unlocked": "0xadd38",
        "shared_pool_generate_all": "0xade0c",
        "shared_pool_get_unlocked": "0xadfb4",
        "potion_factory_get_options": "0x112fba",
        "create_random_potion_in_combat": "0x112edc",
        "create_random_potions": "0x112f2c",
        "can_generate_filter": "0x3d71eb",
        "potion_can_be_generated_in_combat": "0x83120",
        "rarity_filter": "0x3d71fb",
        "rng_next_float": "0x5ec45",
        "rng_next_float_range": "0x5ec53",
        "rng_next_item": "0x5ee74",
        "try_to_procure": "0x1336e8",
        "try_to_procure_move_next": "0x3ef588",
        "player_add_potion_internal": "0x117670",
        "hook_should_procure_potion": "0x106820",
        "hook_after_potion_procured": "0x104470",
        "potion_is_valid_target": "0x8328c",
        "potion_on_use_wrapper": "0x83344",
        "potion_on_use_wrapper_move_next": "0x31dd20",
        "use_potion_action_execute": "0x10e078",
        "use_potion_action_execute_move_next": "0x3d58ec",
    }
    assert CENSUS["source"]["metadata_tokens"] == {
        "ironclad_pool_generate_all": "0x06002c54",
        "ironclad_pool_get_unlocked": "0x06002c55",
        "shared_pool_generate_all": "0x06002c66",
        "shared_pool_get_unlocked": "0x06002c67",
        "create_random_potion_in_combat": "0x06004f1a",
        "create_random_potions": "0x06004f1b",
        "potion_factory_get_options": "0x06004f1c",
        "can_generate_filter": "0x0600b016",
        "potion_can_be_generated_in_combat": "0x06001b28",
        "rarity_filter": "0x0600b018",
        "rng_next_float": "0x06000ff1",
        "rng_next_float_range": "0x06000ff2",
        "rng_next_item": "0x06000ff8",
        "ironclad4_epoch_potions": "0x060002c4",
        "potion1_epoch": "0x06000328",
        "potion2_epoch": "0x06000330",
    }
    assert CENSUS["source"]["normalized_cil_sha256"] == {
        "ironclad_pool_generate_all":
            "a86528d6403d348cb199b028d6c7fff622a63734063e52f3cf6918c191cf5ede",
        "ironclad_pool_get_unlocked":
            "ae589e1d5439fe36a044ecce0282eb7c750904b0f58248bd213903b97477884f",
        "shared_pool_generate_all":
            "b372b253e271e1c97acf4f32740a47cfc60e9c1129e74fe24bd63dc901001bb8",
        "shared_pool_get_unlocked":
            "081a36478405fd55d3a82baa53893ab08d3e8ecbce26a09aa1c1a872d014946f",
        "potion_factory_get_options":
            "ac371b800e5786dfa5533df1190afacdfbd027c1bfdea338edac624a51b46d81",
        "create_random_potion_in_combat":
            "9d55da723e28bb7debdc75322ea54e71a302c8ab591dfc84d8cb98b9b3384a44",
        "create_random_potions":
            "407b30c9eb79054dc04b15edd87e162a3ce4a0fa56ba3eb7443116ecef8cda63",
        "can_generate_filter":
            "9e8b60cc5d3724543640a20a58dde43716c3b91e0d89fcbc5589607d6d5472fb",
        "rarity_filter":
            "5b0d32e25da04c3175204a62f3ea98db37a911bd4032c5649c3c0fcce53846a4",
        "rng_next_float":
            "a75281a592f23a967f1c1c278a738b93a4a0eebbba7cd9fe21cc108dbd036ff6",
        "rng_next_float_range":
            "8009a1c76c825b4728a73f4e71b393756639f934e3a040f9b285a8e6df36a33c",
        "rng_next_item":
            "62b05b86c4103dae460e69fd59748aba894f3739ea6b2eb5cb26bb5beb848512",
    }


def test_solo_ironclad_pool_order_filters_and_rarity_partitions():
    pool = CENSUS["fully_unlocked_solo_ironclad"]
    rows = pool["all_options"]
    reachable = pool["reachable"]
    assert pool["unlock_epochs"] == [
        "IRONCLAD4_EPOCH", "POTION1_EPOCH", "POTION2_EPOCH"]
    assert pool["pool_order"] == ["IRONCLAD", "SHARED"]
    assert pool["pool_sizes"] == {"IRONCLAD": 3, "SHARED": 45}
    assert len(rows) == 48
    assert [row["index"] for row in rows] == list(range(48))
    assert [row["pool"] for row in rows[:3]] == ["IRONCLAD"] * 3
    assert [row["pool"] for row in rows[3:]] == ["SHARED"] * 45
    assert len({row["id"] for row in rows}) == len(rows)

    excluded = [
        row["id"] for row in rows
        if not row["can_be_generated_in_combat"]
    ]
    assert excluded == [
        "POTION.FAIRY_IN_A_BOTTLE",
        "POTION.FRUIT_JUICE",
        "POTION.REGEN_POTION",
    ]
    assert pool["reachable_count"] == len(reachable) == 45
    assert [row["id"] for row in reachable] == [
        row["id"] for row in rows
        if row["can_be_generated_in_combat"]
    ]

    partitions = pool["rarity_partitions"]
    assert {rarity: len(ids) for rarity, ids in partitions.items()} == {
        "Common": 16, "Uncommon": 15, "Rare": 14}
    assert partitions == {
        rarity: [row["id"] for row in reachable
                 if row["rarity"] == rarity]
        for rarity in ("Common", "Uncommon", "Rare")
    }


def test_every_reachable_refused_leaf_has_a_durable_blocker():
    rows = CENSUS["fully_unlocked_solo_ironclad"]["reachable"]
    refused = {
        row["id"]: row["blocker_issues"]
        for row in rows if row["status"] == "refused"
    }
    # The Bottled Potential MCR retired the final reachable potion refusal.
    assert refused == {}
    assert sum(row["status"] == "modeled" for row in rows) == 45
    assert all(
        row["status"] != "refused" or row["blocker_issues"]
        for row in rows)
    assert all(
        COVERAGE["categories"]["potion"][row["id"]]["status"] ==
        row["status"]
        for row in rows)


def test_live_generation_vectors_match_the_solver_rng_exactly():
    generation = CENSUS["generation"]
    assert generation["rarity_roll"] == {
        "draw": "Rng.NextFloat(1.0)",
        "rare_when_lte": 0.10000000149011612,
        "uncommon_when_lte": 0.3499999940395355,
        "otherwise": "Common",
    }
    assert generation["draws_per_generated_potion"] == 2
    assert generation[
        "without_replacement_within_one_CreateRandomPotions_call"] is True

    partitions = CENSUS[
        "fully_unlocked_solo_ironclad"]["rarity_partitions"]
    samples = generation["samples"]
    rng = RunRngSet(
        samples["seed"], build="v0.111.0")["CombatPotionGeneration"]
    for call in samples["calls"]:
        assert rng.counter == call["counter_before"]
        roll = rng.next_float(1.0)
        rarity = (
            "Rare" if roll <= 0.10000000149011612
            else "Uncommon" if roll <= 0.3499999940395355
            else "Common"
        )
        assert rarity == call["rarity"]
        assert rng.next_item(partitions[rarity]) == call["id"]
        assert rng.counter == call["counter_after"]
    assert len(samples["calls"]) == 20
    assert any(
        first["id"] == second["id"]
        for i, first in enumerate(samples["calls"])
        for second in samples["calls"][i + 1:])


def test_live_procurement_vectors_pin_sparse_and_full_belts():
    procurement = CENSUS["procurement"]
    assert procurement["constructed_ascension"] == 0
    assert procurement["probe_max_potion_count"] == 3
    assert procurement["slot_minus_one_rule"] == \
        "first null slot; fail if none"
    assert procurement["sparse_first_null"] == {
        "potion": "POTION.WEAK_POTION",
        "slot_argument": -1,
        "before": [None, "POTION.STRENGTH_POTION", None],
        "after": [
            "POTION.WEAK_POTION", "POTION.STRENGTH_POTION", None],
        "success": True,
        "failure_reason": "None",
    }
    assert procurement["last_empty_slot"] == {
        "potion": "POTION.SPEED_POTION",
        "slot_argument": -1,
        "before": [
            "POTION.WEAK_POTION", "POTION.STRENGTH_POTION", None],
        "after": [
            "POTION.WEAK_POTION", "POTION.STRENGTH_POTION",
            "POTION.SPEED_POTION"],
        "success": True,
        "failure_reason": "None",
    }
    assert procurement["full_belt_failure"] == {
        "potion": "POTION.BLOCK_POTION",
        "slot_argument": -1,
        "before": [
            "POTION.WEAK_POTION", "POTION.STRENGTH_POTION",
            "POTION.SPEED_POTION"],
        "after": [
            "POTION.WEAK_POTION", "POTION.STRENGTH_POTION",
            "POTION.SPEED_POTION"],
        "success": False,
        "failure_reason": "TooFull",
    }
    assert procurement["hooks"] == {
        "pre_insert_veto": "Hook.ShouldProcurePotion",
        "post_success_only": "Hook.AfterPotionProcured",
        "sole_current_pre_insert_override": "RELIC.SOZU",
        "sole_current_post_success_override": "RELIC.BELT_BUCKLE",
    }


def test_fail_closed_solver_contract_preserves_slots_and_provenance():
    gap = CENSUS["solver_contract_gap"]
    assert gap["required_keyed_state"] == [
        "fixed-length ordered potion slots including nulls",
        "modeled versus inert identity per occupied slot",
        "CombatPotionGeneration counter",
        "Belt Buckle applied latch when owned",
    ]
    assert gap["required_entry_evidence"] == [
        "exact max_potion_slot_count",
        "each potion's save slot_index",
        "recorded CombatPotionGeneration counter",
        "fully-unlocked Ironclad potion-pool proof",
    ]
    assert ".run potion ids do not preserve slot_index" in \
        gap["run_history_limit"]
    assert "before RNG or belt mutation" in gap["atomic_refusal"]

    # A checked save proves why the dense tuple is insufficient: one held
    # potion can occupy slot 1 while slot 0 is empty.
    save = json.loads(
        (ROOT / "testdata" / "DSRNCBLEBL_phrog_entry.save").read_text())
    player = save["players"][0]
    assert player["max_potion_slot_count"] == 2
    assert player["potions"] == [{
        "id": "POTION.EXPLOSIVE_AMPOULE", "slot_index": 1}]
    assert save["rng"]["counters"]["combat_potion_generation"] == 0
