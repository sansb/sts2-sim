#!/usr/bin/env python3
"""Record the live v0.111.0 Alchemize/potion-boundary contract (#2056).

This local-only oracle probe loads the installed ``sts2.dll`` through the
documented headless harness.  It records the fully unlocked solo-Ironclad
potion pools in their RNG-significant order, the in-combat generation filter
and rarity partitions, deterministic generation vectors, and ``slot=-1``
procurement behavior for a sparse and then full belt.

The checked JSON is consumed by ordinary DLL-free tests.  Neither CI nor the
solver imports the game assembly.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import host
from host import ALL, asm, call, call_static, prop
import System
from System import Activator, Array, Enum, Int32, Object, String, UInt64

import experiment


ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "coverage_matrix.json"

FULL_UNLOCK_EPOCHS = (
    "IRONCLAD4_EPOCH",
    "POTION1_EPOCH",
    "POTION2_EPOCH",
)

# Durable routes verified on GitHub when #678 was implemented.  A reachable
# refused row without a route is a generator failure, never silent admission.
BLOCKER_ISSUES = {
    "POTION.ATTACK_POTION": (228,),
    "POTION.BOTTLED_POTENTIAL": (231,),
    "POTION.COLORLESS_POTION": (228,),
    "POTION.ENTROPIC_BREW": (680,),
    "POTION.GIGANTIFICATION_POTION": (252,),
    "POTION.MAZALETHS_GIFT": (256,),
    "POTION.OROBIC_ACID": (228,),
    "POTION.POWDERED_DEMISE": (681,),
    "POTION.POWER_POTION": (228,),
    "POTION.SKILL_POTION": (228,),
    "POTION.SOLDIERS_STEW": (677,),
}

GENERATION_SEED = "ALCHEMIZE_CENSUS_V01110"
GENERATION_SAMPLES = 20


def _find(name):
    hits = [t for t in asm.GetTypes() if str(t.Name) == name]
    if not hits:
        raise KeyError(name)
    return hits[0]


def _generic_list(elem_t, items=()):
    list_t = System.Type.GetType("System.Collections.Generic.List`1") \
        .MakeGenericType(Array[System.Type]([elem_t]))
    result = Activator.CreateInstance(list_t)
    add = list_t.GetMethod("Add")
    for item in items:
        add.Invoke(result, Array[Object]([item]))
    return result


def _unlock_state():
    unlock_t = _find("UnlockState")
    ctor = next(c for c in unlock_t.GetConstructors(ALL)
                if c.GetParameters().Length == 3)
    return ctor.Invoke(Array[Object]([
        _generic_list(
            String, [String(epoch) for epoch in FULL_UNLOCK_EPOCHS]),
        _generic_list(_find("ModelId"), []),
        Int32(1000),
    ]))


def _player(unlock):
    model_db = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    character = call_static(model_db, "Get", _find("Ironclad"))
    create = next(
        method for method in _find("Player").GetMethods(ALL)
        if str(method.Name) == "CreateForNewRun"
        and method.GetParameters().Length == 3)
    return create.Invoke(
        None, Array[Object]([character, unlock, UInt64(0)]))


def _method(type_, name, nargs):
    return next(
        method for method in type_.GetMethods(ALL)
        if str(method.Name) == name
        and method.GetParameters().Length == nargs)


def _field(obj, name):
    type_ = obj.GetType()
    while type_ is not None:
        field = type_.GetField(name, ALL)
        if field is not None:
            return field.GetValue(obj)
        type_ = type_.BaseType
    raise AttributeError(name)


def _potion_id(potion):
    return str(prop(potion, "Id"))


def _potion_row(potion, index, *, pool):
    return {
        "index": index,
        "id": _potion_id(potion),
        "class": str(potion.GetType().Name),
        "pool": pool,
        "rarity": str(prop(potion, "Rarity")),
        "can_be_generated_in_combat":
            bool(prop(potion, "CanBeGeneratedInCombat")),
    }


def _slots(player):
    return [
        None if potion is None else _potion_id(potion)
        for potion in prop(player, "PotionSlots")
    ]


def _counter(rng):
    return int(_field(rng, "_counter"))


def _mutable_potion(potion_id):
    return call(experiment._model_by_id(potion_id, "PotionModel"),
                "MutableClone")


def _procure(potion_id, player):
    potion = _mutable_potion(potion_id)
    method = _method(_find("PotionCmd"), "TryToProcure", 3)
    before = _slots(player)
    task = method.Invoke(
        None, Array[Object]([potion, player, Int32(-1)]))
    result = call(call(task, "GetAwaiter"), "GetResult")
    return {
        "potion": potion_id,
        "slot_argument": -1,
        "before": before,
        "after": _slots(player),
        "success": bool(_field(result, "success")),
        "failure_reason": str(_field(result, "failureReason")),
    }


def build_census():
    experiment._boot_and_init()
    unlock = _unlock_state()
    player = _player(unlock)

    model_db = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    character_pool = prop(prop(player, "Character"), "PotionPool")
    shared_pool = call_static(model_db, "Get", _find("SharedPotionPool"))
    get_unlocked = _method(
        character_pool.GetType().BaseType, "GetUnlockedPotions", 1)
    character = list(get_unlocked.Invoke(
        character_pool, Array[Object]([unlock])))
    shared = list(get_unlocked.Invoke(
        shared_pool, Array[Object]([unlock])))

    rows = [
        *(_potion_row(potion, index, pool="IRONCLAD")
          for index, potion in enumerate(character)),
        *(_potion_row(
            potion, len(character) + index, pool="SHARED")
          for index, potion in enumerate(shared)),
    ]
    reachable = [
        row for row in rows if row["can_be_generated_in_combat"]]

    with MATRIX.open() as fh:
        coverage = json.load(fh)["categories"]["potion"]
    unrouted = []
    reachable_rows = []
    for row in reachable:
        entry = coverage[row["id"]]
        blockers = (
            list(BLOCKER_ISSUES.get(row["id"], ()))
            if entry["status"] == "refused" else [])
        if entry["status"] == "refused" and not blockers:
            unrouted.append(row["id"])
        reachable_rows.append({
            **row,
            "status": entry["status"],
            "blocker_issues": blockers,
        })
    if unrouted:
        raise RuntimeError(
            "reachable refused Alchemize leaves have no durable blocker "
            "route: " + ", ".join(unrouted))

    rarity_partitions = {
        rarity: [row["id"] for row in reachable_rows
                 if row["rarity"] == rarity]
        for rarity in ("Common", "Uncommon", "Rare")
    }
    assert sum(map(len, rarity_partitions.values())) == len(reachable_rows)

    # One live run supplies the real named stream and a real mutable player.
    # Slot 1 is deliberately occupied before combat; Alchemize's slot=-1
    # path must fill the first null (slot 0), then the remaining null (slot
    # 2), then fail without mutation once the belt is full.
    add_potion = _method(_find("Player"), "AddPotionInternal", 3)
    setup = add_potion.Invoke(
        player, Array[Object]([
            _mutable_potion("POTION.STRENGTH_POTION"),
            Int32(1), False]))
    assert bool(_field(setup, "success"))
    fight = experiment._Fight(
        player, "Ironclad", 0, ["Glory"],
        "ENCOUNTER.CULTISTS_NORMAL", GENERATION_SEED)
    rng = prop(prop(fight.run, "Rng"), "CombatPotionGeneration")
    factory = _method(_find("PotionFactory"),
                      "CreateRandomPotionInCombat", 3)
    generation = []
    for _ in range(GENERATION_SAMPLES):
        before = _counter(rng)
        potion = factory.Invoke(
            None, Array[Object]([player, rng, None]))
        generation.append({
            "counter_before": before,
            "counter_after": _counter(rng),
            "id": _potion_id(potion),
            "rarity": str(prop(potion, "Rarity")),
        })

    sparse = _procure("POTION.WEAK_POTION", player)
    last_empty = _procure("POTION.SPEED_POTION", player)
    full = _procure("POTION.BLOCK_POTION", player)

    release = json.loads((host.GAME.parent / "release_info.json").read_text())
    dll = host.GAME / "sts2.dll"
    return {
        "schema": 1,
        "game_build": {
            "version": release["version"],
            "commit": release["commit"],
            "sts2_dll_sha256":
                hashlib.sha256(dll.read_bytes()).hexdigest(),
        },
        "source": {
            "kind": "headless-engine-oracle",
            "probe": (
                "sim/v0.111.0/python/harness/"
                "probe_potion_generation.py"),
            "rvas": {
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
            },
            "metadata_tokens": {
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
            },
            "normalized_cil_sha256": {
                "ironclad_pool_generate_all": "a86528d6403d348cb199b028d6c7fff622a63734063e52f3cf6918c191cf5ede",
                "ironclad_pool_get_unlocked": "ae589e1d5439fe36a044ecce0282eb7c750904b0f58248bd213903b97477884f",
                "shared_pool_generate_all": "b372b253e271e1c97acf4f32740a47cfc60e9c1129e74fe24bd63dc901001bb8",
                "shared_pool_get_unlocked": "081a36478405fd55d3a82baa53893ab08d3e8ecbce26a09aa1c1a872d014946f",
                "potion_factory_get_options": "ac371b800e5786dfa5533df1190afacdfbd027c1bfdea338edac624a51b46d81",
                "create_random_potion_in_combat": "9d55da723e28bb7debdc75322ea54e71a302c8ab591dfc84d8cb98b9b3384a44",
                "create_random_potions": "407b30c9eb79054dc04b15edd87e162a3ce4a0fa56ba3eb7443116ecef8cda63",
                "can_generate_filter": "9e8b60cc5d3724543640a20a58dde43716c3b91e0d89fcbc5589607d6d5472fb",
                "rarity_filter": "5b0d32e25da04c3175204a62f3ea98db37a911bd4032c5649c3c0fcce53846a4",
                "rng_next_float": "a75281a592f23a967f1c1c278a738b93a4a0eebbba7cd9fe21cc108dbd036ff6",
                "rng_next_float_range": "8009a1c76c825b4728a73f4e71b393756639f934e3a040f9b285a8e6df36a33c",
                "rng_next_item": "62b05b86c4103dae460e69fd59748aba894f3739ea6b2eb5cb26bb5beb848512",
            },
        },
        "fully_unlocked_solo_ironclad": {
            "unlock_epochs": list(FULL_UNLOCK_EPOCHS),
            "pool_order": ["IRONCLAD", "SHARED"],
            "pool_sizes": {
                "IRONCLAD": len(character),
                "SHARED": len(shared),
            },
            "all_options": rows,
            "filter": "PotionModel.CanBeGeneratedInCombat",
            "reachable_count": len(reachable_rows),
            "reachable": reachable_rows,
            "rarity_partitions": rarity_partitions,
        },
        "generation": {
            "rarity_roll": {
                "draw": "Rng.NextFloat(1.0)",
                "rare_when_lte": 0.10000000149011612,
                "uncommon_when_lte": 0.3499999940395355,
                "otherwise": "Common",
            },
            "within_rarity_roll": (
                "Rng.NextItem(materialized reachable rarity partition)"),
            "draws_per_generated_potion": 2,
            "without_replacement_within_one_CreateRandomPotions_call": True,
            "samples": {
                "seed": GENERATION_SEED,
                "calls": generation,
            },
        },
        "procurement": {
            "constructed_ascension": 0,
            "probe_max_potion_count": int(prop(player, "MaxPotionCount")),
            "slot_minus_one_rule": "first null slot; fail if none",
            "sparse_first_null": sparse,
            "last_empty_slot": last_empty,
            "full_belt_failure": full,
            "hooks": {
                "pre_insert_veto": "Hook.ShouldProcurePotion",
                "post_success_only": "Hook.AfterPotionProcured",
                "sole_current_pre_insert_override": "RELIC.SOZU",
                "sole_current_post_success_override": "RELIC.BELT_BUCKLE",
            },
            "history": (
                "on success only, append ModelChoiceHistoryEntry(id, true) "
                "when CurrentMapPointHistoryEntry is non-null"),
        },
        "solver_contract_gap": {
            "required_keyed_state": [
                "fixed-length ordered potion slots including nulls",
                "modeled versus inert identity per occupied slot",
                "CombatPotionGeneration counter",
                "Belt Buckle applied latch when owned",
            ],
            "required_entry_evidence": [
                "exact max_potion_slot_count",
                "each potion's save slot_index",
                "recorded CombatPotionGeneration counter",
                "fully-unlocked Ironclad potion-pool proof",
            ],
            "run_history_limit": (
                ".run potion ids do not preserve slot_index after use or "
                "discard; refuse Alchemize unless exact slot layout comes "
                "from a save/replay-backed entry"),
            "atomic_refusal": (
                "validate pool, counter, slot topology, generated leaf, "
                "Sozu/Belt Buckle hook support, and ending gates before RNG "
                "or belt mutation"),
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    text = json.dumps(build_census(), indent=1) + "\n"
    if args.output:
        args.output.write_text(text)
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
