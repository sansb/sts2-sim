#!/usr/bin/env python3
"""Record the live current-build character-card pools used by Splash (#667).

This is a local oracle probe.  It loads the installed ``sts2.dll`` through
the documented headless harness, asks the real ``CardPoolModel`` instances
for their ordered cards at each unlock epoch, and records the exact filters
used by combat generation.  The checked JSON is consumed by ordinary
DLL-free tests; neither CI nor the solver imports the game assembly.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import host
from host import ALL, asm, call_static, prop
import System
from System import Activator, Array, Enum, Int32, Object, String


ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "coverage_matrix.json"

CHARACTER_EPOCHS = {
    "IRONCLAD": tuple(f"IRONCLAD{i}_EPOCH" for i in range(2, 8)),
    "SILENT": tuple(f"SILENT{i}_EPOCH" for i in range(1, 8)),
    "REGENT": tuple(f"REGENT{i}_EPOCH" for i in range(1, 8)),
    "NECROBINDER": tuple(f"NECROBINDER{i}_EPOCH" for i in range(1, 8)),
    "DEFECT": tuple(f"DEFECT{i}_EPOCH" for i in range(1, 8)),
}

# Durable routes verified on GitHub when #667 was created.  A row with a
# refused status but no route is a generator failure, not an invitation to
# silently admit the leaf.
BLOCKER_ISSUES = {
    "CARD.FLAK_CANNON": (668,),
    "CARD.MISERY": (670,),
    "CARD.SCRAPE": (671,),
    "CARD.SIC_EM": (672,),
    "CARD.STRANGLE": (383,),
    "CARD.THE_SCYTHE": (673,),
}


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


def _unlock_state(epochs):
    unlock_t = _find("UnlockState")
    ctor = next(c for c in unlock_t.GetConstructors(ALL)
                if c.GetParameters().Length == 3)
    return ctor.Invoke(Array[Object]([
        _generic_list(String, [String(e) for e in epochs]),
        _generic_list(_find("ModelId"), []),
        Int32(1000),
    ]))


def _get_unlocked(pool, unlock, solo_constraint):
    method = next(m for m in pool.GetType().GetMethods(ALL)
                  if str(m.Name) == "GetUnlockedCards"
                  and m.GetParameters().Length == 2)
    return list(method.Invoke(
        pool, Array[Object]([unlock, solo_constraint])))


def _enum_name(value):
    return str(value)


def _card_id(card):
    return str(prop(card, "Id"))


def _pool_character(pool):
    name = str(pool.GetType().Name)
    assert name.endswith("CardPool")
    return name[:-8].upper()


def _boot():
    host.boot()
    mod_manager = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManager")
    state_t = asm.GetType("MegaCrit.Sts2.Core.Modding.ModManagerState")
    call_static(mod_manager, "set_State", Enum.Parse(state_t, "Initialized"))
    model_db = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")
    init = next(m for m in model_db.GetMethods(ALL)
                if str(m.Name) == "Init")
    init.Invoke(None, Array[Object]([None] * init.GetParameters().Length))
    return model_db


def build_census():
    model_db = _boot()
    all_characters = list(call_static(model_db, "get_AllCharacters"))

    all_epochs = tuple(
        epoch
        for character in ("IRONCLAD", "SILENT", "REGENT", "NECROBINDER",
                          "DEFECT")
        for epoch in CHARACTER_EPOCHS[character]
    )
    full_unlock = _unlock_state(all_epochs)
    character_pools = list(prop(full_unlock, "CharacterCardPools"))

    get_unlocked = next(
        m for m in character_pools[0].GetType().BaseType.GetMethods(ALL)
        if str(m.Name) == "GetUnlockedCards"
        and m.GetParameters().Length == 2)
    constraint_t = get_unlocked.GetParameters()[1].ParameterType
    # RunState.CardMultiplayerConstraint is the mode-specific exclusion:
    # SingleplayerOnly (2) removes MultiplayerOnly (1) cards.
    solo_constraint = Enum.Parse(constraint_t, "SingleplayerOnly")
    no_constraint = Enum.Parse(constraint_t, "None")

    pool_by_character = {
        _pool_character(pool): pool for pool in character_pools
    }
    character_order = [_pool_character(pool) for pool in character_pools]
    assert character_order == [
        str(prop(character, "Id")).split(".", 1)[1]
        for character in all_characters
    ]

    pools = {}
    for character in character_order:
        pool = pool_by_character[character]
        all_cards = list(prop(pool, "AllCards"))
        epoch_sets = [()]
        current = []
        for epoch in CHARACTER_EPOCHS[character]:
            current = [*current, epoch]
            epoch_sets.append(tuple(current))

        first_epoch = {}
        for epochs in epoch_sets:
            unlocked = _get_unlocked(
                pool, _unlock_state(epochs), no_constraint)
            for card in unlocked:
                first_epoch.setdefault(
                    _card_id(card), epochs[-1] if epochs else None)

        rows = []
        for index, card in enumerate(all_cards):
            card_id = _card_id(card)
            assert card_id in first_epoch
            rows.append({
                "index": index,
                "id": card_id,
                "class": str(card.GetType().Name),
                "unlock_epoch": first_epoch[card_id],
                "type": _enum_name(prop(card, "Type")),
                "rarity": _enum_name(prop(card, "Rarity")),
                "multiplayer_constraint": _enum_name(
                    prop(card, "MultiplayerConstraint")),
                "can_be_generated_in_combat": bool(
                    prop(card, "CanBeGeneratedInCombat")),
            })
        pools[character] = {
            "pool_type": str(pool.GetType().FullName),
            "generate_all_cards_rva": {
                "IRONCLAD": "0xf5614",
                "SILENT": "0xf636c",
                "REGENT": "0xf5f54",
                "NECROBINDER": "0xf5adc",
                "DEFECT": "0xf4fb8",
            }[character],
            "filter_through_epochs_rva": {
                "IRONCLAD": "0xf594c",
                "SILENT": "0xf66ac",
                "REGENT": "0xf6294",
                "NECROBINDER": "0xf5e1c",
                "DEFECT": "0xf52f8",
            }[character],
            "cards": rows,
        }

    with MATRIX.open() as fh:
        matrix = json.load(fh)
    coverage = matrix["categories"]["card"]

    owner = "IRONCLAD"

    def combat_attack_eligible(card):
        return (
            _enum_name(prop(card, "Type")) == "Attack"
            and bool(prop(card, "CanBeGeneratedInCombat"))
            and _enum_name(prop(card, "Rarity"))
            not in {"Basic", "Ancient", "Event"}
        )

    splash_rows = []
    excluded_attacks = []
    unrouted = []
    for character in character_order:
        if character == owner:
            continue
        pool = pool_by_character[character]
        solo_ids = {
            _card_id(card)
            for card in _get_unlocked(pool, full_unlock, solo_constraint)
        }
        for card in prop(pool, "AllCards"):
            if _enum_name(prop(card, "Type")) != "Attack":
                continue
            card_id = _card_id(card)
            reasons = []
            if card_id not in solo_ids:
                reasons.append("not in fully-unlocked single-player pool")
            if not bool(prop(card, "CanBeGeneratedInCombat")):
                reasons.append("CanBeGeneratedInCombat=false")
            rarity = _enum_name(prop(card, "Rarity"))
            if rarity in {"Basic", "Ancient", "Event"}:
                reasons.append(f"rarity={rarity}")
            if reasons:
                excluded_attacks.append({
                    "id": card_id,
                    "character": character,
                    "reasons": reasons,
                })
                continue
            assert combat_attack_eligible(card)
            entry = coverage[card_id]
            status = entry["status"]
            blockers = (list(BLOCKER_ISSUES.get(card_id, ()))
                        if status == "refused" else [])
            if status == "refused" and not blockers:
                unrouted.append(card_id)
            splash_rows.append({
                "id": card_id,
                "character": character,
                "status": status,
                "blocker_issues": blockers,
            })
    if unrouted:
        raise RuntimeError(
            "reachable refused Splash leaves have no durable blocker route: "
            + ", ".join(unrouted))

    release = json.loads((host.GAME.parent / "release_info.json").read_text())
    dll = host.GAME / "sts2.dll"
    single_pool_cards = [
        _card_id(card)
        for card in _get_unlocked(
            pool_by_character[owner], full_unlock, solo_constraint)
        if combat_attack_eligible(card)
    ]
    return {
        "schema": 1,
        "game_build": {
            "version": release["version"],
            "commit": release["commit"],
            "sts2_dll_sha256": hashlib.sha256(dll.read_bytes()).hexdigest(),
        },
        "source": {
            "kind": "headless-engine-oracle",
            "probe": "python/harness/probe_card_pools.py",
            "rvas": {
                "unlock_state_character_card_pools": "0x12064",
                "card_pool_get_unlocked_cards": "0x81e64",
                "card_factory_filter_for_player_count": "0x115f1c",
                "card_factory_filter_for_combat": "0x11634a",
                "splash_on_play": "0x3bba6c",
            },
        },
        "character_pool_order": character_order,
        "character_unlock_epochs": CHARACTER_EPOCHS,
        "pools": pools,
        "splash_from_solo_ironclad": {
            "owner_pool_removed_when_character_pool_count_gt_one": owner,
            "single_pool_edge": {
                "owner_pool_retained": True,
                "cards": single_pool_cards,
                "filtered_pool_size": len(single_pool_cards),
                "combat_card_generation_draws":
                    max(len(single_pool_cards) - 1, 0),
            },
            "filter_order": [
                "UnlockState.CharacterCardPools",
                "remove owner CardPool",
                "CardPoolModel.GetUnlockedCards(single-player)",
                "CardType.Attack",
                "CardFactory.FilterForPlayerCount",
                "CardFactory.FilterForCombat",
                "Distinct",
            ],
            "filtered_pool_size": len(splash_rows),
            "combat_card_generation_draws": max(len(splash_rows) - 1, 0),
            "cards": splash_rows,
            "excluded_attacks": excluded_attacks,
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
