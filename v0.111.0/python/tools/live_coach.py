#!/usr/bin/env python3
"""Build a live combat state in Rust and request one Rust exact solve.

The Python side keeps save parsing, game-build/RNG verification, caveat
reporting and narration. The combat-entry root is the Rust opening
(``sts-sim entry --save --opening``, the same root the review worker builds,
#2827), and exact search, memoization, deadlines, objective ordering and
action selection belong to ``sts-sim exact-solve``. The solved line is
replayed in Rust (``diff-serve``) and must reach the digest the solver
claimed before it is narrated.

The save-adapter helpers below (``build_entry``, ``verify_stream_seeding``,
``current_node``, the unlock-state readers) are the documented oracle of the
Rust entry crate (``engine/src/entry/**`` cites them by name), and the review
CLI imports them too. Nothing here imports the frozen Python simulator (#2827
item F1).

Usage:
    python3 -u tools/live_coach.py SAVE [--encounter ID]
        [--max-turns N | --horizon N] [--deadline SECONDS]
        [--alpha FINAL_HP] [--quiet] [--dry] [--build vX.Y.Z]
        [--no-unmodeled-potions]

Legacy Python-only rollout, potion-bank, potion-margin, and holdback flags are
rejected explicitly instead of silently changing the Rust objective.
"""
from __future__ import annotations

import json
import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

import sts2_rng  # noqa: E402
from relay_parser import require_single_player  # noqa: E402

INSTALLED_RELEASE_INFO = pathlib.Path.home() / (
    "Library/Application Support/Steam/steamapps/common/"
    "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/release_info.json")


def installed_game_build() -> str:
    """The build string of the CURRENTLY INSTALLED game.

    A live save is by definition recorded under the installed build, so the
    per-stream seeding scheme must come from there. The deleted Python
    simulator's RunRngSet(seed) calls omitted build= and therefore defaulted to
    GAME_BUILD_V0_108 (deliberately, to keep pre-#309 .run pins valid) —
    which silently derives every stream with the 32-bit djb2 scheme and
    produces wrong monster-HP/shuffle rolls on a v0.109 save.
    """
    return json.loads(INSTALLED_RELEASE_INFO.read_text())["version"]


def verify_stream_seeding(save: dict, build: str) -> list[str]:
    """Check derived stream states against the ones the save records.

    Save schema >= 19 stores each stream's xoshiro state (s0..s3), so the
    seeding scheme is now directly falsifiable instead of merely assumed.
    Returns the names of streams whose derived state disagrees.
    """
    rngs = save["rng"].get("rngs")
    if not rngs:
        return []
    counters = {"".join(p.capitalize() for p in k.split("_")): v["counter"]
                for k, v in rngs.items()}
    run_set = sts2_rng.RunRngSet(save["rng"]["seed"], counters, build=build)
    bad = []
    for key, state in rngs.items():
        name = "".join(p.capitalize() for p in key.split("_"))
        rng = run_set.rngs.get(name)
        if rng is None:
            continue
        m = rng._random
        if (m.s0, m.s1, m.s2, m.s3) != (state["s0"], state["s1"],
                                        state["s2"], state["s3"]):
            bad.append(key)
    return bad

STREAMS = {"shuffle": "Shuffle", "niche": "Niche", "monster_ai": "MonsterAi",
           "combat_card_selection": "CombatCardSelection",
           "combat_targets": "CombatTargets",
           "combat_energy_costs": "CombatEnergyCosts",
           "combat_card_generation": "CombatCardGeneration",
           "combat_potion_generation": "CombatPotionGeneration",
           "combat_orbs": "CombatOrbGeneration"}

_INFERNAL_BLADE_IRONCLAD_EPOCHS = frozenset({
    "IRONCLAD2_EPOCH", "IRONCLAD5_EPOCH", "IRONCLAD7_EPOCH"})
_ALCHEMIZE_IRONCLAD_EPOCHS = frozenset({
    "IRONCLAD4_EPOCH", "POTION1_EPOCH", "POTION2_EPOCH"})


def infernal_blade_pool_fully_unlocked(player: dict) -> bool | None:
    """Whether the save proves the full Infernal Blade/Stoke Ironclad pool."""
    unlock_state = player.get("unlock_state")
    if not isinstance(unlock_state, dict):
        return None
    epochs = unlock_state.get("unlocked_epochs")
    if not isinstance(epochs, list):
        return None
    normalized = {
        str(epoch).removeprefix("EPOCH.") for epoch in epochs}
    return _INFERNAL_BLADE_IRONCLAD_EPOCHS <= normalized


def card_pool_unlocked_epochs(player: dict) -> tuple[str, ...] | None:
    """Return the save's normalized exact card-pool UnlockState proof."""
    unlock_state = player.get("unlock_state")
    if not isinstance(unlock_state, dict):
        return None
    epochs = unlock_state.get("unlocked_epochs")
    if (not isinstance(epochs, list)
            or any(not isinstance(epoch, str) or not epoch
                   for epoch in epochs)):
        return None
    return tuple(sorted({
        epoch.removeprefix("EPOCH.") for epoch in epochs
    }))


def potion_pool_fully_unlocked(player: dict) -> bool | None:
    """Whether the save proves #678's complete solo-Ironclad potion pool."""
    epochs = card_pool_unlocked_epochs(player)
    return (
        None if epochs is None
        else _ALCHEMIZE_IRONCLAD_EPOCHS <= set(epochs))


def player_character_id(player: dict) -> str | None:
    """Normalize JSON-save and decoded-MCR character fields."""
    value = player.get("character_id", player.get("character"))
    if not isinstance(value, str):
        return None
    return value if value.startswith("CHARACTER.") else f"CHARACTER.{value}"


def load_save(path):
    return json.load(open(path))


def current_node(save):
    """Node index in the run's GLOBAL numbering.

    visited_map_coords resets at each act transition, so its length only
    gives the PER-ACT index; prior acts' node lists stay behind in
    map_point_history (one completed list per finished act). The entry
    derives the per-fight Encounter stream's total_floor from this value
    (+1), so the per-act shortfall silently reseeded every act-2+
    random-starter encounter — the #826 MonsterAi chain drift: the
    session-7 SCROLLS_OF_BITING_NORMAL fight rolled starter 0 instead of
    the game's 2 and its CHEW draw vanished. Same formula as
    mcr_validate.global_node_index."""
    act = save.get("current_act_index") or 0
    prior = (sum(len(a) for a in save["map_point_history"][:act])
             if act else 0)
    return prior + len(save["visited_map_coords"]) - 1


def node_type(save):
    """Map point type of the current node, from the saved act map."""
    m = save["acts"][save["current_act_index"]].get("saved_map")
    cur = save["visited_map_coords"][-1]
    if not m:
        return None
    for pt in m["points"]:
        if pt["coord"] == cur:
            # JSON uses the enum name; normalize to relay_parser style
            t = str(pt.get("point_type") or pt.get("type") or "")
            return t.lower()
    return None


def next_encounter(save, kind):
    r = save["acts"][save["current_act_index"]]["rooms"]
    if kind == "elite":
        return r["elite_encounter_ids"][r["elite_encounters_visited"]]
    if kind == "boss":
        return r["boss_id"]
    return r["normal_encounter_ids"][r["normal_encounters_visited"]]


_SCOPED_RELIC_SAVED_PROPERTIES = {
    "RELIC.BONE_TEA": ("ints", "CombatsLeft", int),
    "RELIC.EMBER_TEA": ("ints", "CombatsLeft", int),
    "RELIC.PUMPKIN_CANDLE": ("ints", "KindleCount", int),
    "RELIC.TEA_OF_DISCOURTESY": ("ints", "CombatsLeft", int),
    "RELIC.VENERABLE_TEA_SET": (
        "bools", "GainEnergyInNextCombat", bool),
    "RELIC.FAKE_VENERABLE_TEA_SET": (
        "bools", "GainEnergyInNextCombat", bool),
    "RELIC.LIZARD_TAIL": ("bools", "WasUsed", bool),
    # #2924: the persistent counters the entry seeds. v0.111.0 sts2.dll
    # (sha 9cb4f1ad): each type declares exactly one [SavedProperty] with the
    # parameterless ctor (CustomAttribute blob 01000000 — always serialized,
    # the same form as BoneTea.CombatsLeft) on an int32 property (sig
    # 28 00 08), beside RelicModel's inherited IsWax/IsMelted, whose ctor
    # argument omits them at false. The decoded MCR save carries these as a
    # flat map ({"TurnsSeen": 2}) that the grouped-only path below ignored.
    "RELIC.HAPPY_FLOWER": ("ints", "TurnsSeen", int),
    "RELIC.FAKE_HAPPY_FLOWER": ("ints", "TurnsSeen", int),
    "RELIC.PENDULUM": ("ints", "TurnsSeen", int),
    "RELIC.POLLINOUS_CORE": ("ints", "TurnsSeen", int),
    "RELIC.NUNCHAKU": ("ints", "AttacksPlayed", int),
    "RELIC.PEN_NIB": ("ints", "AttacksPlayed", int),
    "RELIC.GALACTIC_DUST": ("ints", "StarsSpent", int),
    "RELIC.IRON_CLUB": ("ints", "CardsPlayed", int),
    "RELIC.TUNING_FORK": ("ints", "SkillsPlayed", int),
    "RELIC.GIRYA": ("ints", "TimesLifted", int),
}
_INHERITED_FALSE_RELIC_PROPERTIES = frozenset({"IsWax", "IsMelted"})
_GROUPED_SAVED_PROPERTY_KEYS = frozenset({
    "ints", "int_arrays", "bools", "model_ids", "cards", "card_arrays",
    "strings",
})


def _canonical_model_id(value, category):
    """Normalize one JSON or decoded-MCR model entry to its full ID."""
    if not isinstance(value, str) or not value:
        raise NotImplementedError(
            f"invalid {category} model-entry id {value!r}")
    prefix = f"{category}."
    if value.startswith(prefix) and value.count(".") == 1:
        return value
    if "." not in value:
        return prefix + value
    raise NotImplementedError(
        f"{category} model-entry id has the wrong category: {value!r}")


def _card_upgrade_level(card):
    """Read the mutually exclusive JSON-save or decoded-MCR upgrade key."""
    has_json = "current_upgrade_level" in card
    has_mcr = "upgrade_level" in card
    if has_json and has_mcr:
        raise NotImplementedError(
            "card row mixes current_upgrade_level and upgrade_level")
    value = (
        card["current_upgrade_level"] if has_json
        else card["upgrade_level"] if has_mcr
        else 0)
    if type(value) is not int:
        raise NotImplementedError(
            f"card upgrade level is not an exact integer: {value!r}")
    return value


def _strict_scoped_relic_property(relic_id, props):
    """Return ``(valid, value)`` for one scoped relic's SavedProperties.

    JSON saves retain typed groups as name/value rows. The MCR decoder emits
    the same serialized properties as one flat name/value map. Only the
    relic's required property and the two inherited false default flags are
    legal. The inherited flags may be omitted and are never synthesized.
    """
    group, required_name, required_type = \
        _SCOPED_RELIC_SAVED_PROPERTIES[relic_id]
    if not isinstance(props, dict) or not props:
        return False, None

    grouped = bool(set(props) & _GROUPED_SAVED_PROPERTY_KEYS)
    values = {}
    if grouped:
        allowed_groups = {group}
        if group != "bools":
            allowed_groups.add("bools")
        if not set(props) <= allowed_groups:
            return False, None
        for prop_group, rows in props.items():
            if (not isinstance(rows, list) or not rows
                    or any(not isinstance(row, dict)
                           or set(row) != {"name", "value"}
                           for row in rows)):
                return False, None
            for row in rows:
                name = row["name"]
                value = row["value"]
                if not isinstance(name, str) or name in values:
                    return False, None
                if name == required_name:
                    if (prop_group != group
                            or type(value) is not required_type):
                        return False, None
                elif name in _INHERITED_FALSE_RELIC_PROPERTIES:
                    if prop_group != "bools" or value is not False:
                        return False, None
                else:
                    return False, None
                values[name] = value
    else:
        if not set(props) <= (
                {required_name} | _INHERITED_FALSE_RELIC_PROPERTIES):
            return False, None
        values = dict(props)
        for name, value in values.items():
            if name == required_name:
                if type(value) is not required_type:
                    return False, None
            elif value is not False:
                return False, None

    if required_name not in values:
        return False, None
    return True, values[required_name]


def _strict_joss_paper_properties(props):
    """Decode room-entry Joss Paper properties, including native defaults.

    v0.111.0 JossPaper.CardsExhausted uses SaveIfNotTypeDefault; zero is
    omitted by SavedProperties::FromInternal. JossPaper::AfterCombatEnd
    (RVA 0x95573) clears EtherealCount. Neither omitted zero is unknown.
    Explicit values still pass through to the entry's range/phase checks.
    """
    required = {"CardsExhausted", "EtherealCount"}
    if props is None:
        props = {}
    if not isinstance(props, dict):
        return None
    grouped = bool(set(props) & _GROUPED_SAVED_PROPERTY_KEYS)
    if grouped:
        if not set(props) <= {"ints", "bools"}:
            return None
        ints = props.get("ints", [])
        bools = props.get("bools", [])
        if (not isinstance(ints, list)
                or not isinstance(bools, list)
                or any(not isinstance(row, dict)
                       or set(row) != {"name", "value"}
                       or not isinstance(row["name"], str)
                       for row in ints + bools)):
            return None
        values = {}
        for row in ints:
            if (row["name"] in values or row["name"] not in required
                    or type(row["value"]) is not int):
                return None
            values[row["name"]] = row["value"]
        flags = set()
        for row in bools:
            if (row["name"] in flags
                    or row["name"] not in _INHERITED_FALSE_RELIC_PROPERTIES
                    or row["value"] is not False):
                return None
            flags.add(row["name"])
    else:
        if not set(props) <= (required | _INHERITED_FALSE_RELIC_PROPERTIES):
            return None
        values = {name: props[name] for name in required if name in props}
        if any(type(value) is not int for value in values.values()):
            return None
        if any(props[name] is not False
               for name in set(props) - required):
            return None
    return {name: values.get(name, 0) for name in required}


def _strict_fur_coat_membership(props, current_coord):
    """Return exact current-coordinate membership, or None if unprovable.

    Fur Coat's map writer persists the acquisition act, parallel coordinate
    arrays, and a completion latch. JSON saves retain typed property groups;
    decoded MCR snapshots flatten the same properties. Accept only the whole
    native shape, including inherited false defaults, so a partial/mixed
    payload can never silently seed combat behavior.
    """
    required_groups = {
        "FurCoatActIndex": "ints",
        "FurCoatCoordCols": "int_arrays",
        "FurCoatCoordRows": "int_arrays",
        "FurCoatCoordsSet": "bools",
    }
    if (not isinstance(props, dict) or not props
            or not isinstance(current_coord, list)
            or len(current_coord) != 2
            or any(type(value) is not int or not 0 <= value <= 255
                   for value in current_coord)):
        return None
    grouped = bool(set(props) & _GROUPED_SAVED_PROPERTY_KEYS)
    values = {}
    if grouped:
        if not set(props) <= set(required_groups.values()):
            return None
        for group, rows in props.items():
            if (not isinstance(rows, list)
                    or any(not isinstance(row, dict)
                           or set(row) != {"name", "value"}
                           for row in rows)):
                return None
            for row in rows:
                name, value = row["name"], row["value"]
                if not isinstance(name, str) or name in values:
                    return None
                if name in required_groups:
                    if group != required_groups[name]:
                        return None
                elif name in _INHERITED_FALSE_RELIC_PROPERTIES:
                    if group != "bools" or value is not False:
                        return None
                else:
                    return None
                values[name] = value
    else:
        allowed = set(required_groups) | _INHERITED_FALSE_RELIC_PROPERTIES
        if not set(props) <= allowed:
            return None
        values = dict(props)
        if any(values[name] is not False
               for name in set(values) & _INHERITED_FALSE_RELIC_PROPERTIES):
            return None
    if not set(required_groups) <= set(values):
        return None
    if (type(values["FurCoatActIndex"]) is not int
            or values["FurCoatActIndex"] < 0
            or type(values["FurCoatCoordsSet"]) is not bool):
        return None
    cols = values["FurCoatCoordCols"]
    rows = values["FurCoatCoordRows"]
    if (not isinstance(cols, list) or not isinstance(rows, list)
            or len(cols) != len(rows)
            or any(type(value) is not int or not 0 <= value <= 255
                   for value in cols + rows)):
        return None
    if values["FurCoatCoordsSet"]:
        if len(cols) != 7 or len(set(zip(cols, rows))) != 7:
            return None
    elif cols or rows:
        return None
    return tuple(current_coord) in set(zip(cols, rows))


def build_entry(save, encounter_id, ntype):
    p = require_single_player(save, "live save")
    deck = []
    for c in p["deck"]:
        card_id = _canonical_model_id(c.get("id"), "CARD")
        e = {"id": card_id, "upgrade_level": _card_upgrade_level(c)}
        if card_id in {
                "CARD.THE_SCYTHE", "CARD.GENETIC_ALGORITHM",
                "CARD.MAD_SCIENCE"} \
                and "props" in c:
            # Current-build saves persist both mutable DynamicVar fields.
            # The entry validates exact names/types and each card's
            # Current == native base + Increased relationship.
            e["props"] = c["props"]
        ench = c.get("enchantment")
        if ench:
            e["enchantment"] = _canonical_model_id(
                ench.get("id"), "ENCHANTMENT")
            # Current-build saves store the enchantment strength as
            # "amount" (89SJD17KYUEH: SHARP 3 read as 1 shorted every
            # Gunk Up hit by 2 pre-multiplier — the 9-HP Amalgam drift
            # on the first Queen certification capture). "level" is the
            # historical key; amountless enchantments default to 1.
            e["enchant_amount"] = ench.get(
                "amount", ench.get("level", 1))
        deck.append(e)
    relic_counters = {}
    tea_set_charged = None
    fake_tea_set_charged = None
    fur_coat_active = None
    relic_ids = []
    for r in p["relics"]:
        relic_id = _canonical_model_id(r.get("id"), "RELIC")
        relic_ids.append(relic_id)
        # JSON saves group props by type; decoded MCR SavedProperties is a
        # flat name->value map. Scoped finite/Tea relics use a strict,
        # fail-closed validator across the complete property shape.
        props = r.get("props") or {}
        if relic_id == "RELIC.FUR_COAT":
            coords = save.get("visited_map_coords")
            current_coord = coords[-1] if isinstance(coords, list) and coords \
                else None
            fur_coat_active = _strict_fur_coat_membership(
                props, current_coord)
            continue
        if relic_id == "RELIC.JOSS_PAPER":
            values = _strict_joss_paper_properties(r.get("props"))
            if values is not None:
                relic_counters[relic_id] = values
            continue
        if relic_id in _SCOPED_RELIC_SAVED_PROPERTIES:
            valid, value = _strict_scoped_relic_property(relic_id, props)
            if valid:
                group, _name, _kind = \
                    _SCOPED_RELIC_SAVED_PROPERTIES[relic_id]
                if group == "ints" or relic_id == "RELIC.LIZARD_TAIL":
                    relic_counters[relic_id] = value
                elif relic_id == "RELIC.VENERABLE_TEA_SET":
                    tea_set_charged = value
                else:
                    fake_tea_set_charged = value
            continue

        # Preserve the established grouped JSON adapter for unrelated relic
        # counters. Decoded flat props outside the reviewed scope remain
        # ignored rather than gaining new combat semantics here.
        ints = props.get("ints") or []
        if len(ints) == 1:
            relic_counters[relic_id] = ints[0]["value"]
        elif ints:
            relic_counters[relic_id] = {
                e["name"]: e["value"] for e in ints}
    potion_rows = []
    for q in p.get("potions", []):
        potion_rows.append({
            "id": _canonical_model_id(q.get("id"), "POTION"),
            "slot_index": q.get("slot_index", q.get("slot")),
        })
    max_potion_slots = p.get(
        "max_potion_slot_count", p.get("max_potion_slots"))
    return {
        "encounter_id": encounter_id,
        "node_index": current_node(save),
        "node_type": ntype or "monster",
        "deck_entering": deck,
        "relics_entering": relic_ids,
        "potions_entering": [row["id"] for row in potion_rows],
        "max_potion_slot_count": max_potion_slots,
        "potion_slots_entering": potion_rows,
        "hp_entering": p["current_hp"],
        "max_hp_entering": p["max_hp"],
        "gold_entering": p["gold"],
        # RunState.AscensionLevel: the entry selects every tiered monster
        # constant by it and refuses a missing/non-int level (#2539).
        "ascension": save.get("ascension"),
        "relic_counters": relic_counters or None,
        "tea_set_charged": tea_set_charged,
        "fake_tea_set_charged": fake_tea_set_charged,
        "fur_coat_active": fur_coat_active,
        # players[N].relics is the serialized Player._relics list itself
        # (ToSerializable 0x11a804 order-preserving projection,
        # PopulateRelics 0x11b3c0 appends on load), which is exactly the
        # same-hook relic dispatch order (IterateHookListeners d__69
        # 0x3f6700 enumerates it by ascending index). Vouch for it only on
        # schema >= 19 saves, where that list order was verified end-to-end
        # against a live run (issue #808, seed TQM88QFMHSQR).
        "relics_entering_dispatch_ordered":
            "rngs" in (save.get("rng") or {}),
        "entry_ambiguous": False,
    }


def stream_counters(save: dict) -> dict:
    """Per-stream RNG counters, across both save schemas.

    schema <= 18: rng.counters = {stream: counter}
    schema >= 19: rng.rngs = {stream: {counter, s0..s3}} — the xoshiro
    state is now stored per stream; only the counter is read here, since
    the entry re-derives the states from rng.seed.
    """
    rng = save["rng"]
    if "counters" in rng:
        return dict(rng["counters"])
    if "rngs" in rng:
        return {k: v["counter"] for k, v in rng["rngs"].items()}
    raise NotImplementedError(
        f"save rng block has neither 'counters' (schema <= 18) nor 'rngs' "
        f"(schema >= 19); keys = {sorted(rng)}")


def solution_summary(*, label: str, elapsed: float, nodes: int,
                     hp_entering: int, objective: tuple[int, int, int, int],
                     observed_damage: int | None) -> str:
    """Render only quantities authenticated by the Rust objective.

    Final belt size is exact even when Alchemize/Entropic Brew gained a
    potion. Entry-minus-final belt size is not a consumption count and can be
    negative, so live coaching deliberately reports potions kept.
    """
    final_hp = objective[1]
    turns = -objective[3]
    return (
        f"\n{label} ({elapsed:.1f}s, {nodes} states): win with "
        f"{hp_entering - final_hp} HP lost ({final_hp} remaining), "
        f"{objective[2]} potion(s) kept, {turns} turns "
        f"(vs observed {observed_damage} HP lost)"
    )


def potion_caveat(root: dict) -> str | None:
    """The narrowed potion action space a solved line must declare (I4).

    The frozen Python helper (`combat_sim.unmodeled_potion_caveat`, #634,
    #679, #875) listed exactly which reachable potions made a line
    conditional. The Rust root carries what it holds inert
    (`player.inert_potions`) and whether generation is strict
    (`player.strict_potions`); the rest is stated as its condition rather than
    enumerated, which over-discloses rather than under-discloses.
    """
    player = root.get("player") or {}
    parts = []
    inert = sorted(player.get("inert_potions") or ())
    if inert:
        parts.append(
            f"{len(inert)} unmodeled potion(s) held ({', '.join(inert)}); "
            "the line is optimal among lines that do not use them, and may "
            "be beaten by a line that does")
    if not player.get("strict_potions"):
        parts.append(
            "a potion generated mid-fight that the engine does not model is "
            "held inert, so any line through one is optimal only among lines "
            "that do not use it (--no-unmodeled-potions refuses instead)")
    if any(slot is not None for slot in player.get("potion_slots") or ()):
        parts.append(
            "once a local pet is live, AnyAlly potion actions the engine does "
            "not whitelist are omitted, so the line is optimal only among "
            "plays that do not drink them after that point")
    return "; ".join(parts) if parts else None


def rust_opening(binary: pathlib.Path, save_path: pathlib.Path, build: str,
                 encounter: str | None, ntype: str | None) -> dict:
    """`sts-sim entry --save --opening`: the review worker's own root.

    The Rust entry reads the save (the port of `build_entry`,
    `stream_counters`, `current_node`, `node_type` and `next_encounter`
    above), runs the combat opening, and answers a canonical document or a
    named refusal. The entry's empty AfterEnergyReset order is proposed the
    way the floor-snapshot review does it (`rust_review.build_root`).
    """
    import subprocess

    import rust_review
    argv = [str(binary), "entry", "--build", build, "--save", str(save_path),
            "--opening"]
    if encounter is not None:
        argv += ["--encounter", encounter]
    if ntype is not None:
        argv += ["--node-type", ntype]
    completed = subprocess.run(argv, text=True, capture_output=True,
                               timeout=60, check=False)
    if completed.returncode:
        raise NotImplementedError(
            "Rust opening CLI failed: " + completed.stderr[-500:])
    document = json.loads(completed.stdout)
    if document.get("schema") != "sts-sim-canonical-v2":
        opening = document.get("opening") or {}
        refusal = document.get("refusal") or {}
        kind = (opening.get("refusal_class") or document.get("refusal_class")
                or "entry_refused")
        detail = opening.get("detail") or refusal.get("detail") or ""
        raise NotImplementedError(
            f"Rust opening refused: {kind}: {detail[:300]} "
            "(SOLVER_INVARIANTS.md I5 — no line without an exact root)")
    return rust_review._with_entry_reset_witness(binary, document)


def narrate_rust_line(binary: pathlib.Path, root: dict,
                      actions, final_digest: str) -> list[str]:
    """Replay the solved line in Rust and render it turn by turn.

    Every action is applied through `diff-serve` from the same root the
    solver searched; the replay must end on the digest the solver claimed.
    """
    import canonical_document
    import rust_review
    from rust_replay import RustReplay, selection_details

    rows = []
    with RustReplay(binary, root) as session:
        before = root
        turn = None
        for wire in actions:
            player = before["player"]
            if player.get("turn", 1) != turn:
                turn = player.get("turn", 1)
                visible = rust_review.snapshot(before)
                rows.append(
                    f"turn {turn}: hp {visible['hp']} block {visible['block']} "
                    f"energy {visible['energy']} hand {visible['hand']}")
            applied = session.ask({"cmd": "apply", "action": wire,
                                   "describe_selection": wire["kind"] == "select"})
            after = session.ask({"cmd": "project"})["state"]
            if wire["kind"] == "play":
                card = next(c for c in before["piles"]["hand"]
                            if c["uid"] == wire["uid"])
                text = f"  play {rust_review.card_name(card)}"
                if wire.get("target") is not None:
                    target = before["monsters"][wire["target"]]
                    text += f" -> {target['kind']} ({target.get('hp', 0)} hp)"
            elif wire["kind"] == "potion":
                text = f"  potion {player['potion_slots'][wire['slot']]}"
                if wire.get("target") is not None:
                    target = before["monsters"][wire["target"]]
                    text += f" -> {target['kind']} ({target.get('hp', 0)} hp)"
            elif wire["kind"] == "select":
                details = selection_details(
                    before, wire, after, applied.get("selected_uids"))
                text = f"  select {details.get('cards') or details or wire['answer']}"
            else:
                text = "  end turn"
            rows.append(f"{text}  => hp {after['player'].get('hp', 0)}")
            before = after
    if canonical_document.differential_digest(before) != final_digest:
        raise NotImplementedError(
            "Rust replay of the solved line did not reach its claimed digest")
    return rows


def main():
    import rust_exact_solve

    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    save_path = pathlib.Path(args[0])
    save = load_save(save_path)
    seed = save["rng"]["seed"]
    counters = {STREAMS[k]: v for k, v in stream_counters(save).items()
                if k in STREAMS}
    build = (sys.argv[sys.argv.index("--build") + 1]
             if "--build" in sys.argv else installed_game_build())
    bad = verify_stream_seeding(save, build)
    if bad:
        raise NotImplementedError(
            f"stream seeding for build {build} does not reproduce the "
            f"states recorded in the save: {bad} "
            "(SOLVER_INVARIANTS.md I5 — refusing to solve off unverified "
            "RNG; re-verify seeding_scheme against this build's sts2.dll)")
    print(f"build {build}: stream seeding verified against save state "
          f"({len(save['rng'].get('rngs', {}))} streams)")
    ntype = node_type(save)
    enc = None
    if "--encounter" in sys.argv:
        enc = sys.argv[sys.argv.index("--encounter") + 1]
    else:
        kind = ("elite" if ntype == "elite"
                else "boss" if ntype == "boss" else "monster")
        enc = next_encounter(save, kind)
    entry = build_entry(save, enc, ntype)

    print(f"seed {seed} node {entry['node_index']} ({ntype}) vs {enc}")
    print(f"hp {entry['hp_entering']}/{entry['max_hp_entering']}, "
          f"deck {len(entry['deck_entering'])}, "
          f"potions {entry['potions_entering']}")
    print("recorded counters:", counters)
    retired = sorted(flag for flag in (
        "--hold-potions", "--no-rollout", "--no-potion-bank",
        "--potion-bank", "--potion-margins", "--no-potion-margins",
    ) if flag in sys.argv)
    if retired:
        raise NotImplementedError(
            "Python-only solver options retired with the Rust cutover: "
            + ", ".join(retired))
    binary = rust_exact_solve.default_binary()
    if binary is None:
        raise rust_exact_solve.RustExactSolveError(
            "Rust sts-sim binary is required for live coaching "
            "(cargo build --release --bin sts-sim, or STS_SIM_EXACT_SOLVER)")
    root = rust_opening(binary, save_path, build, enc, ntype or "monster")
    if "--no-unmodeled-potions" in sys.argv:
        inert = root["player"].get("inert_potions") or []
        if inert:
            raise NotImplementedError(
                f"unmodeled potion(s) held: {sorted(inert)} "
                "(--no-unmodeled-potions; SOLVER_INVARIANTS.md I5)")
        root["player"]["strict_potions"] = True
    # (#634) an unmodeled potion does not end the coaching session; it sits
    # inert in its belt slot and the verdict below is a conditional optimum
    caveat = potion_caveat(root)
    if caveat:
        print(f"! CAVEAT: {caveat}")
    print("turn-1 hand: "
          f"{[card['id'] for card in root['piles'].get('hand', [])]}")
    if "--dry" in sys.argv:
        print("dry run ok (state constructed)")
        return
    max_turns = 10
    if "--max-turns" in sys.argv:
        max_turns = int(sys.argv[sys.argv.index("--max-turns") + 1])
    if "--horizon" in sys.argv:
        max_turns = int(sys.argv[sys.argv.index("--horizon") + 1])
    deadline = None
    if "--deadline" in sys.argv:
        deadline = float(sys.argv[sys.argv.index("--deadline") + 1])
    alpha0 = 0
    if "--alpha" in sys.argv:
        alpha0 = int(sys.argv[sys.argv.index("--alpha") + 1])

    started = time.monotonic()
    result = rust_exact_solve.solve_document(
        root, horizon=max_turns, deadline=deadline, binary=binary,
        alpha_final_hp=alpha0)
    elapsed = time.monotonic() - started
    solution = result.solution
    if solution is None:
        if result.incomplete:
            print(f"search found no winning line within horizon {max_turns} "
                  f"({elapsed:.1f}s) — result UNKNOWN, search incomplete")
        else:
            print(f"no winning line exists within horizon {max_turns} "
                  f"({elapsed:.1f}s; Rust search complete)")
        if caveat:
            print(f"! CAVEAT: {caveat}")
        return

    rows = narrate_rust_line(binary, root, solution.actions,
                             solution.final_digest)
    label = ("EXACT" if result.exact
             else "BEST FOUND (deadline hit — NOT proven optimal)")
    print(solution_summary(
        label=label, elapsed=elapsed, nodes=result.nodes,
        hp_entering=entry["hp_entering"], objective=solution.objective,
        observed_damage=entry.get("damage_taken")))
    if caveat:
        print(f"! CAVEAT: {caveat}")
    if "--quiet" not in sys.argv:
        print()
        for row in rows:
            print(row)


if __name__ == "__main__":
    main()
