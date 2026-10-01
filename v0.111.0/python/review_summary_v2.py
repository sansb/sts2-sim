"""The fight-review CLI (schema v8) the review worker runs.

Every review is built in Rust (#2827): with floor saves, or a resolved save
projection plus its capture, by ``rust_review``; otherwise, from the ``.run``
alone, by ``rust_legacy_review``. This module is the CLI and the entry
checks both share: the resolved-save projection's completeness and conflict
checks (``provenance_entry_for_fight``) and the ``.run`` copy-only
canonicalization (``canonicalize_entry_multiset``).

It imports nothing from the frozen Python simulator (#2827 item F1). The
retired Python producer (``combat_sim`` roots, the ``infoset_sample``
benchmark-only card, the classify-only eligibility seam) was deleted with its
last readers. Benchmark-only cards are not produced: those fights keep a
named refusal (Sean, 2026-09-25, #3004). Stored ``ok_benchmark_only``
documents stay readable by the site and the worker.
"""

from __future__ import annotations

import argparse
import copy
import itertools
import json
import pathlib
import sys
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from typing import Any, Sequence

sys.path.insert(0, str(pathlib.Path(__file__).parent))
sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))

import relay_parser as relay  # noqa: E402
import review_summary as phase1  # noqa: E402
import rust_exact_solve  # noqa: E402
import sim_identity  # noqa: E402
# The save/capture entry adapter (`build_entry` and the unlock-state readers),
# the same functions `mcr_replay` imports; `live_coach` is simulator-free.
import live_coach  # noqa: E402
from relay_parser import FightState  # noqa: E402


# 5 (#1219/#1108): true-state generated lines carry stable producer-authored
# identifiers, and every solved outcome carries producer-owned claim wording.
# See sim/v0.111.0/python/REVIEW_PIPELINE_SCHEMA_V5.md.
# 6 (#1241): `recorded_line` — the player's OWN line with the simulator's
# annotations, present only when the capture replayed to the recorded outcome
# exactly. Purely additive: `recorded_log` keeps its sim-free contract and
# stays the fallback, because it must survive refusals that this field cannot.
# 7 (#1267): `metadata.simulator` — the identity of what produced the
# document, on cards AND refusals, so a stored document can be invalidated by
# query when the simulator changes under a fixed game build.
# `metadata.solver_build` is renamed `metadata.game_build`; it always held the
# run's game build, never the solver's. See REVIEW_PIPELINE_SCHEMA_V7.md.
# 8 (#1268): the `deferred` status — a run on an archived-but-not-yet-
# admitted build has no result YET rather than no result ever, and says so
# instead of borrowing the refusal vocabulary. See
# REVIEW_PIPELINE_SCHEMA_V8.md.
SCHEMA_VERSION = 8

# Statuses that carry no card. The CLI exits 2 for these: a caller asking
# for a review did not get one, whether the answer was "no" or "not yet".
NO_CARD_STATUSES = frozenset(("refused", "deferred"))
ASCENSION_UNLOCK_DEFAULT = 10
ReviewConfig = phase1.ReviewConfig

UPGRADE_COPY_REASON = "upgrade_copy_assignment_ambiguous"
ENCHANT_COPY_REASON = "enchant_copy_assignment_ambiguous"


def generate_review_document(
        run_path: str | pathlib.Path, fight_index: int, *,
        config: ReviewConfig = ReviewConfig()) -> dict:
    """Return a card or refusal for a fight reviewed from its ``.run`` alone.

    The review of a fight with neither floor saves nor a capture-backed
    resolved save (those take `rust_review`, see ``main``) is built entirely
    in Rust by `rust_legacy_review.generate` (#2827 C2): the `.run` facts
    plus the Rust counter prediction root it, and Rust searches and replays
    it.
    """

    import rust_legacy_review
    return rust_legacy_review.generate(run_path, fight_index, config)


_PERSISTENT_CARD_IDS = frozenset({
    "CARD.THE_SCYTHE", "CARD.GENETIC_ALGORITHM", "CARD.MAD_SCIENCE",
})


_PERSISTENT_RELIC_IDS = frozenset({
    "RELIC.FUR_COAT", "RELIC.JOSS_PAPER",
    *relay._FINITE_COMBAT_RELIC_SEEDS,
    *relay._COUNTER_RELIC_MODS,
    *relay._ZERO_ONLY_COUNTER_RELICS,
    relay._GIRYA, relay._PUMPKIN_CANDLE, relay._TEA_SET,
    relay._FAKE_TEA_SET, relay._LIZARD_TAIL,
})


@dataclass(frozen=True)
class ProvenancePoolEvidence:
    fully_unlocked_card_pool: bool
    fully_unlocked_potion_pool: bool
    unlocked_card_pool_epochs: tuple[str, ...]


class ProvenanceEntryRejected(ValueError):
    """A resolved save projection was incomplete or conflicted with .run."""

    def __init__(self, reason: str, check: str):
        super().__init__(check)
        self.reason = reason
        self.check = check


def provenance_entry_for_fight(
        fight: FightState, projection: Any, *,
        expected_character: str | None = None,
        replay: dict[str, Any] | None = None
        ) -> tuple[dict, ProvenancePoolEvidence]:
    """Adapt one projected save through the save adapter (`live_coach.build_entry`).

    Conflict authority is field-specific: captured values may replace a run
    field explicitly marked ambiguous, while every other unambiguous run fact
    must still agree.  Only after those independent checks pass are the
    deck/relic portions overlaid on the otherwise unchanged run entry.
    """

    player = _complete_projected_player(projection)
    _check_entry_conflicts(fight, player, expected_character)
    epochs = live_coach.card_pool_unlocked_epochs(player)
    card_pool = live_coach.infernal_blade_pool_fully_unlocked(player)
    potion_pool = live_coach.potion_pool_fully_unlocked(player)
    if epochs is None or card_pool is None or potion_pool is None:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "unlock_state")

    current_coord = _replay_current_coord(replay)
    if "RELIC.FUR_COAT" in fight.relics_entering and \
            current_coord is None:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "fur_coat_coord")
    visited = [[0, index] for index in range(fight.node_index + 1)]
    if current_coord is not None:
        visited[-1] = current_coord
    save = {
        "players": [{
            **copy.deepcopy(player),
            "current_hp": fight.hp_entering,
            "max_hp": fight.max_hp_entering,
            "gold": fight.gold_entering,
            "potions": [],
        }],
        "rng": {"rngs": copy.deepcopy(projection["run_rng"])},
        "current_act_index": 0,
        "map_point_history": [],
        "visited_map_coords": visited,
    }
    try:
        adapted = live_coach.build_entry(
            save, fight.encounter_id, fight.node_type)
    except (KeyError, TypeError, ValueError, NotImplementedError) as exc:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete",
            f"save_adapter:{type(exc).__name__}") from exc
    _overlay_recorded_entry_upgrades(
        fight, adapted, replay, expected_character=expected_character)
    _check_adapted_conflicts(fight, adapted)

    entry = asdict(fight)
    for key in (
            "deck_entering", "relics_entering", "relic_counters",
            "tea_set_charged", "fake_tea_set_charged",
            "fur_coat_active", "relics_entering_dispatch_ordered"):
        entry[key] = copy.deepcopy(adapted[key])
    return entry, ProvenancePoolEvidence(
        card_pool, potion_pool, epochs)


def _overlay_recorded_entry_upgrades(
        fight: FightState, adapted: dict, replay: dict[str, Any] | None, *,
        expected_character: str | None) -> None:
    """Resolve run-ambiguous upgrade fields from the combat replay snapshot.

    The replay embeds the game's run snapshot at combat entry. It is stronger
    evidence for a field used by the recorded inputs than the separately
    projected save: #1117's projection reported Pyre/Whirlwind at level 0,
    while the embedded entry snapshot reported level 1 and the recorded
    energy budget is legal only at level 1. All unambiguous fields remain
    strict conflicts, and exact deck order must agree before any overlay.
    """

    if replay is None:
        return
    try:
        replay_run = replay["run"]
        replay_player = relay.require_single_player(
            replay_run, "MCR replay")
        replay_deck = replay_player["deck"]
    except (KeyError, TypeError, ValueError, NotImplementedError) as exc:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete",
            f"recorded_replay_entry:{type(exc).__name__}") from exc
    if not isinstance(replay_deck, list) or any(
            not isinstance(row, dict)
            or not isinstance(row.get("id"), str)
            or type(row.get("upgrade_level")) is not int
            or row["upgrade_level"] < 0
            for row in replay_deck):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "recorded_replay_deck")
    replay_character = live_coach.player_character_id(replay_player)
    if (expected_character is not None
            and replay_character != expected_character):
        raise ProvenanceEntryRejected(
            "provenance_entry_conflict", "recorded_replay_character")

    exact_deck = adapted["deck_entering"]
    replay_ids = [row["id"].removeprefix("CARD.") for row in replay_deck]
    exact_ids = [row["id"].removeprefix("CARD.") for row in exact_deck]
    if replay_ids != exact_ids:
        raise ProvenanceEntryRejected(
            "provenance_entry_conflict", "recorded_replay_deck_order")

    ambiguous_ids = {
        row["id"].removeprefix("CARD.")
        for row in fight.deck_entering
        if row.get("upgrade_ambiguous")
    }
    for exact, recorded in zip(exact_deck, replay_deck):
        card_id = exact["id"].removeprefix("CARD.")
        recorded_level = recorded["upgrade_level"]
        if card_id in ambiguous_ids:
            exact["upgrade_level"] = recorded_level
        elif exact["upgrade_level"] != recorded_level:
            raise ProvenanceEntryRejected(
                "provenance_entry_conflict",
                "recorded_replay_deck_upgrade")


def _complete_projected_player(projection: Any) -> dict[str, Any]:
    if not isinstance(projection, dict) or \
            type(projection.get("save_schema_version")) is not int or \
            projection["save_schema_version"] < 19 or \
            not isinstance(projection.get("run_rng"), dict) or \
            not projection["run_rng"] or \
            not isinstance(projection.get("shared_relic_grab_bag"), dict):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "entry_envelope")
    players = projection.get("players")
    if not isinstance(players, list) or len(players) != 1 or \
            not isinstance(players[0], dict):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "single_player")
    player = players[0]
    if type(player.get("net_id")) is not int or player["net_id"] < 0 or \
            not isinstance(player.get("character_id"), str) or \
            not player["character_id"] or \
            not isinstance(player.get("unlock_state"), dict) or \
            not isinstance(player.get("relic_grab_bag"), dict):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "player_envelope")
    unlock = player["unlock_state"]
    if not isinstance(unlock.get("unlocked_epochs"), list) or \
            any(not isinstance(item, str) or not item
                for item in unlock["unlocked_epochs"]) or \
            not isinstance(unlock.get("encounters_seen"), list) or \
            any(not isinstance(item, str) or not item
                for item in unlock["encounters_seen"]) or \
            type(unlock.get("number_of_runs")) is not int or \
            unlock["number_of_runs"] < 0:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "unlock_state")
    _validate_grab_bag(
        projection["shared_relic_grab_bag"], "shared_relic_grab_bag")
    _validate_grab_bag(player["relic_grab_bag"], "relic_grab_bag")
    _validate_indexed_rows(player.get("deck"), "copy_index", "deck")
    _validate_indexed_rows(
        player.get("relics"), "acquisition_index", "relics")
    for card in player["deck"]:
        if not isinstance(card.get("id"), str) or not card["id"] or \
                type(card.get("upgrade_level")) is not int or \
                card["upgrade_level"] < 0 or "enchantment" not in card:
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "deck_card")
        enchantment = card["enchantment"]
        if enchantment is not None and (
                not isinstance(enchantment, dict)
                or not isinstance(enchantment.get("id"), str)
                or not enchantment["id"]
                or type(enchantment.get(
                    "amount", enchantment.get("level", 1))) is not int):
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "deck_enchantment")
        if card["id"] in _PERSISTENT_CARD_IDS and \
                not isinstance(card.get("props"), dict):
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "deck_persistent_props")
    for relic in player["relics"]:
        if not isinstance(relic.get("id"), str) or not relic["id"]:
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "relic_id")
        if relic["id"] in _PERSISTENT_RELIC_IDS and \
                not isinstance(relic.get("props"), dict):
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "relic_persistent_props")
    return player


def _validate_grab_bag(value: dict[str, Any], check: str) -> None:
    if not value:
        return
    lists = value.get("relic_id_lists")
    if set(value) != {"relic_id_lists"} or not isinstance(lists, dict) or \
            any(not isinstance(pool, list) or any(
                not isinstance(relic_id, str) or not relic_id
                for relic_id in pool) for pool in lists.values()):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", check)


def _validate_indexed_rows(rows: Any, index_key: str, check: str) -> None:
    if not isinstance(rows, list) or any(
            not isinstance(row, dict) or row.get(index_key) != index
            for index, row in enumerate(rows)):
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", check)


def _check_entry_conflicts(
        fight: FightState, player: dict[str, Any],
        expected_character: str | None) -> None:
    save_character = live_coach.player_character_id(player)
    if save_character is None:
        raise ProvenanceEntryRejected(
            "provenance_entry_incomplete", "character_id")
    if expected_character is not None and save_character != expected_character:
        raise ProvenanceEntryRejected(
            "provenance_entry_conflict", "character_id")

    run_deck = fight.deck_entering
    save_deck = player["deck"]
    if Counter(row.get("id") for row in run_deck) != \
            Counter(row.get("id") for row in save_deck):
        raise ProvenanceEntryRejected(
            "provenance_entry_conflict", "deck_membership")
    for card_id in sorted({row["id"] for row in run_deck}):
        run_rows = [row for row in run_deck if row["id"] == card_id]
        save_rows = [row for row in save_deck if row["id"] == card_id]
        upgrade_matches = (
            sorted(row["upgrade_level"] for row in run_rows)
            == sorted(row["upgrade_level"] for row in save_rows))
        if (not any(row.get("upgrade_ambiguous") for row in run_rows)
                and not upgrade_matches):
            raise ProvenanceEntryRejected(
                "provenance_entry_conflict", "deck_upgrade")
        run_enchants = Counter(
            json.dumps(
                (row.get("enchantment"),
                 row.get("enchant_amount", 1)),
                separators=(",", ":"))
            for row in run_rows)
        save_enchants = Counter(
            json.dumps(_projected_enchantment(row),
                       separators=(",", ":"))
            for row in save_rows)
        if (not any(row.get("enchant_ambiguous") for row in run_rows)
                and run_enchants != save_enchants):
            raise ProvenanceEntryRejected(
                "provenance_entry_conflict", "deck_enchantment")
        checks_props = any(
            "props" in row for row in run_rows + save_rows)
        if (not any(row.get("props_ambiguous") for row in run_rows)
                and checks_props):
            run_props = sorted(
                json.dumps(row.get("props"), sort_keys=True)
                for row in run_rows)
            save_props = sorted(
                json.dumps(row.get("props"), sort_keys=True)
                for row in save_rows)
            if run_props != save_props:
                raise ProvenanceEntryRejected(
                    "provenance_entry_conflict", "deck_props")

    save_relics = [row["id"] for row in player["relics"]]
    if Counter(fight.relics_entering) != Counter(save_relics):
        raise ProvenanceEntryRejected(
            "provenance_entry_conflict", "relic_membership")


def _check_adapted_conflicts(fight: FightState, adapted: dict) -> None:
    saved_counters = adapted.get("relic_counters") or {}
    for relic_id, run_value in fight.relic_counters.items():
        if relic_id not in saved_counters:
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "relic_props")
        if saved_counters[relic_id] != run_value:
            raise ProvenanceEntryRejected(
                "provenance_entry_conflict", "relic_props")
    for field in ("tea_set_charged", "fake_tea_set_charged"):
        run_value = getattr(fight, field)
        saved_value = adapted.get(field)
        if run_value is not None:
            if saved_value is None:
                raise ProvenanceEntryRejected(
                    "provenance_entry_incomplete", "relic_props")
            if run_value != saved_value:
                raise ProvenanceEntryRejected(
                    "provenance_entry_conflict", "relic_props")
    if "RELIC.FUR_COAT" in fight.relics_entering:
        saved_value = adapted.get("fur_coat_active")
        if saved_value is None:
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "fur_coat_props")
        if fight.fur_coat_active is not None and \
                saved_value != fight.fur_coat_active:
            raise ProvenanceEntryRejected(
                "provenance_entry_conflict", "fur_coat_props")


def _replay_current_coord(replay: Any) -> list[int] | None:
    try:
        coord = replay["run"]["visited_map_coords"][-1]
    except (KeyError, IndexError, TypeError):
        return None
    if not isinstance(coord, list) or len(coord) != 2 or any(
            type(value) is not int for value in coord):
        return None
    return list(coord)


def _projected_enchantment(card: dict[str, Any]) -> tuple[Any, int]:
    enchantment = card.get("enchantment")
    if enchantment is None:
        return None, 1
    return enchantment.get("id"), enchantment.get(
        "amount", enchantment.get("level", 1))


def _write_provenance_entry_rejection(
        path: pathlib.Path | None, reason: str, check: str) -> None:
    if path is None:
        return
    try:
        path.write_text(json.dumps({"reason": reason, "check": check}))
    except OSError:
        pass


def load_provenance_entry(
        path: pathlib.Path | None,
        status_path: pathlib.Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        _write_provenance_entry_rejection(
            status_path, "provenance_entry_incomplete", "transport_json")
        return None
    if not isinstance(value, dict):
        _write_provenance_entry_rejection(
            status_path, "provenance_entry_incomplete", "transport_shape")
        return None
    return value


def canonicalize_entry_multiset(
        fight: FightState, raw_run: dict) -> tuple[dict, list[str]]:
    """Resolve copy-only flags to one canonical exact entry multiset.

    Every concrete assignment consistent with the dated upgrade/enchantment
    events is enumerated per card id.  The fallback is legal only when all
    assignments produce one identical multiset of simulator-visible card
    attributes.  Physical rows are then assigned that multiset in canonical
    order; a genuine multiset fork refuses.
    """

    entry = asdict(fight)
    deck = entry["deck_entering"]
    ambiguous_ids = {
        row["id"] for row in deck
        if row.get("upgrade_ambiguous") or row.get("enchant_ambiguous")
    }
    if not ambiguous_ids:
        return entry, []

    history = _history(raw_run)
    late_upgrades = Counter(
        card_id
        for node_index, point in enumerate(history)
        if node_index > fight.node_index
        for card_id in point["player_stats"][0].get("upgraded_cards", []))
    enchant_constraints = _enchant_constraints(
        fight, raw_run, history)
    reasons = []

    for card_id in sorted(ambiguous_ids):
        positions = [
            index for index, row in enumerate(deck)
            if row["id"] == card_id]
        rows = [copy.deepcopy(deck[index]) for index in positions]
        upgrade_flagged = any(
            row.get("upgrade_ambiguous") for row in rows)
        decrement_count = late_upgrades[card_id] if upgrade_flagged else 0
        if upgrade_flagged and decrement_count <= 0:
            raise phase1.ReviewRefusal(
                "simulation_refusal",
                f"upgrade timing ambiguous for "
                f"{card_id.removeprefix('CARD.')} "
                "(not a dated duplicate-copy assignment; refusing to "
                "guess the entry multiset)")
        if upgrade_flagged:
            _validate_upgrade_copy_inventory(
                card_id, rows, fight, raw_run, history)

        local = {position: index for index, position in enumerate(positions)}
        constraints = []
        for indices, active_count in enchant_constraints.get(card_id, ()):
            try:
                constraints.append((
                    tuple(local[index] for index in indices), active_count))
            except KeyError as exc:
                raise phase1.ReviewRefusal(
                    "simulation_refusal",
                    f"enchant timing ambiguous for "
                    f"{card_id.removeprefix('CARD.')} (copy inventory "
                    "does not match the fight entry)") from exc

        candidates = _concrete_assignments(
            rows, decrement_count, constraints)
        multiset = None
        canonical_payloads = None
        for candidate in candidates:
            payloads = tuple(sorted(
                (_sim_payload(row) for row in candidate),
                key=_payload_key))
            signature = tuple(_payload_key(payload) for payload in payloads)
            if multiset is None:
                multiset = signature
                canonical_payloads = payloads
            elif signature != multiset:
                raise phase1.ReviewRefusal(
                    "simulation_refusal",
                    f"copy assignment forks the entry multiset for "
                    f"{card_id.removeprefix('CARD.')} "
                    "(SOLVER_INVARIANTS.md I5 — refusing to guess)")
        if canonical_payloads is None:
            raise phase1.ReviewRefusal(
                "simulation_refusal",
                f"copy assignment constraints are inconsistent for "
                f"{card_id.removeprefix('CARD.')} "
                "(SOLVER_INVARIANTS.md I5 — refusing to guess)")

        for position, payload in zip(positions, canonical_payloads):
            destination = deck[position]
            for key in (
                    "id", "upgrade_level", "enchantment",
                    "enchant_amount", "props"):
                destination.pop(key, None)
            destination.update(copy.deepcopy(payload))
            destination.pop("upgrade_ambiguous", None)
            destination.pop("enchant_ambiguous", None)

        if upgrade_flagged:
            reasons.append(UPGRADE_COPY_REASON)
        if constraints:
            reasons.append(ENCHANT_COPY_REASON)

    return entry, _stable_unique(reasons)


def _validate_upgrade_copy_inventory(
        card_id: str, entry_rows: list[dict], fight: FightState,
        raw_run: dict, history: list[dict]) -> None:
    """Prove later upgrades could only land on copies present at entry."""

    records = [
        record for record in _endpoint_card_records(raw_run, history)
        if record[1].get("id") == card_id]
    fight_floor = fight.node_index + 1

    def present_at_fight(record):
        removed_at, card = record
        return (card.get("floor_added_to_deck", 1) < fight_floor
                and (removed_at is None or removed_at >= fight.node_index))

    entering = [record for record in records if present_at_fight(record)]
    raw_levels = sorted(
        card.get("current_upgrade_level", 0) for _removed, card in entering)
    parsed_levels = sorted(row["upgrade_level"] for row in entry_rows)
    if len(entering) != len(entry_rows) or raw_levels != parsed_levels:
        raise phase1.ReviewRefusal(
            "simulation_refusal",
            f"upgrade timing ambiguous for "
            f"{card_id.removeprefix('CARD.')} (endpoint copy inventory "
            "does not match the fight entry; refusing to guess)")

    late_event_nodes = [
        node_index
        for node_index, point in enumerate(history)
        if node_index > fight.node_index
        for upgraded in point["player_stats"][0].get("upgraded_cards", [])
        if upgraded == card_id]
    for event_node in late_event_nodes:
        event_floor = event_node + 1
        for record in records:
            if present_at_fight(record):
                continue
            removed_at, card = record
            available = (
                card.get("floor_added_to_deck", 1) < event_floor
                and (removed_at is None or removed_at >= event_node))
            if available and card.get("current_upgrade_level", 0) > 0:
                raise phase1.ReviewRefusal(
                    "simulation_refusal",
                    f"upgrade timing ambiguous for "
                    f"{card_id.removeprefix('CARD.')} (a copy outside the "
                    "fight entry was eligible for a later upgrade, so the "
                    "entry multiset can fork; refusing to guess)")


def _concrete_assignments(rows: list[dict], decrement_count: int,
                          enchant_constraints: list[tuple[tuple[int, ...],
                                                          int]]):
    levels = tuple(row["upgrade_level"] for row in rows)
    for decrements in _bounded_compositions(levels, decrement_count):
        upgraded = copy.deepcopy(rows)
        for index, decrement in enumerate(decrements):
            upgraded[index]["upgrade_level"] -= decrement
        partial = [upgraded]
        for indices, active_count in enchant_constraints:
            if not 0 <= active_count <= len(indices):
                partial = []
                break
            next_partial = []
            for selected in itertools.combinations(indices, active_count):
                selected = set(selected)
                for candidate in partial:
                    changed = copy.deepcopy(candidate)
                    for index in indices:
                        if index not in selected:
                            changed[index]["enchantment"] = None
                            changed[index].pop("enchant_amount", None)
                    next_partial.append(changed)
            partial = next_partial
        for candidate in partial:
            for row in candidate:
                row.pop("upgrade_ambiguous", None)
                row.pop("enchant_ambiguous", None)
            yield candidate


def _bounded_compositions(limits: tuple[int, ...], total: int):
    if total < 0:
        return

    def visit(index: int, remaining: int, prefix: tuple[int, ...]):
        if index == len(limits):
            if remaining == 0:
                yield prefix
            return
        for value in range(min(limits[index], remaining) + 1):
            yield from visit(
                index + 1, remaining - value, prefix + (value,))

    yield from visit(0, total, ())


def _enchant_constraints(fight: FightState, raw_run: dict,
                         history: list[dict]):
    deck = fight.deck_entering
    flagged = defaultdict(list)
    for index, row in enumerate(deck):
        if not row.get("enchant_ambiguous"):
            continue
        enchantment = row.get("enchantment")
        if enchantment == "ENCHANTMENT.GOOPY":
            raise phase1.ReviewRefusal(
                "simulation_refusal",
                f"enchant timing ambiguous for GOOPY on "
                f"{row['id'].removeprefix('CARD.')} "
                "(the mutable amount forks the entry multiset; refusing "
                "to guess)")
        key = (row["id"], row.get("floor_added", 1), enchantment,
               row.get("enchant_amount", 1))
        flagged[key].append(index)

    if not flagged:
        return {}

    records = _endpoint_card_records(raw_run, history)
    endpoint_cards = [card for _removed, card in records]
    removed = [
        (node_index, card) for node_index, card in records
        if node_index is not None]

    events = defaultdict(list)
    for node_index, point in enumerate(history):
        for event in point["player_stats"][0].get("cards_enchanted", []):
            card = event.get("card", {})
            enchantment = card.get("enchantment") or {}
            enchantment_id = event.get("enchantment") or \
                enchantment.get("id")
            key = (
                card.get("id"), card.get("floor_added_to_deck", 1),
                enchantment_id, enchantment.get("amount", 1))
            events[key].append(node_index)

    constraints = defaultdict(list)
    for key, indices in flagged.items():
        card_id, floor_added, enchantment_id, amount = key
        base_key = key[:3]
        if any(event_key[:3] == base_key and event_key[3] != amount
               for event_key in events):
            raise phase1.ReviewRefusal(
                "simulation_refusal",
                f"enchant timing ambiguous for "
                f"{enchantment_id.removeprefix('ENCHANTMENT.')} on "
                f"{card_id.removeprefix('CARD.')} (dated and endpoint "
                "amounts differ, so the entry multiset can fork; refusing "
                "to guess)")
        endpoint_count = sum(
            _raw_enchant_key(card) == key for card in endpoint_cards)
        matching_events = events.get(key, [])
        arrived_enchanted = endpoint_count - len(matching_events)
        removed_before = sum(
            node_index < fight.node_index and _raw_enchant_key(card) == key
            for node_index, card in removed)
        if (endpoint_count != len(indices) or arrived_enchanted < 0
                or removed_before):
            raise phase1.ReviewRefusal(
                "simulation_refusal",
                f"enchant timing ambiguous for "
                f"{enchantment_id.removeprefix('ENCHANTMENT.')} on "
                f"{card_id.removeprefix('CARD.')} (the surviving-copy "
                "inventory can fork the entry multiset; refusing to guess)")
        active_count = arrived_enchanted + sum(
            node_index < fight.node_index
            for node_index in matching_events)
        constraints[card_id].append((tuple(indices), active_count))
    return constraints


def _endpoint_card_records(raw_run: dict, history: list[dict]):
    records = [(None, card) for card in raw_run["players"][0]["deck"]]
    for node_index, point in enumerate(history):
        stats = point["player_stats"][0]
        records.extend(
            (node_index, card)
            for card in stats.get("cards_removed", []))
        records.extend(
            (node_index, transform["original_card"])
            for transform in stats.get("cards_transformed", []))
    return records


def _raw_enchant_key(card: dict):
    enchantment = card.get("enchantment")
    if not isinstance(enchantment, dict):
        return None
    return (
        card.get("id"), card.get("floor_added_to_deck", 1),
        enchantment.get("id"), enchantment.get("amount", 1))


def _sim_payload(row: dict) -> dict:
    payload = {
        "id": row["id"],
        "upgrade_level": row["upgrade_level"],
        "enchantment": row.get("enchantment"),
    }
    if payload["enchantment"] is not None:
        payload["enchant_amount"] = row.get("enchant_amount", 1)
    if "props" in row:
        payload["props"] = copy.deepcopy(row["props"])
    return payload


def _payload_key(payload: dict) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def _upgrade_document(document: dict, *,
                      assumed_fully_unlocked: bool) -> dict:
    upgraded = copy.deepcopy(document)
    upgraded["schema_version"] = SCHEMA_VERSION
    if upgraded.get("status") != "eligible_benchmark_only":
        upgraded["degraded_reasons"] = []
    best = upgraded.get("best_actual_seed")
    if isinstance(best, dict) and isinstance(best.get("line"), list) \
            and best["line"]:
        # The headline actual-state solve is the well-known semantic family.
        # Future holdback/quest variants must supply their own producer slugs;
        # the consumer must never invent array-position identifiers.
        best["line_id"] = "best"
    upgraded.setdefault("metadata", {})["assumed_fully_unlocked"] = \
        assumed_fully_unlocked
    return upgraded


def _validate_recorded_length(fight: FightState, horizon: int) -> None:
    if type(fight.turns_taken) is not int or fight.turns_taken < 0:
        raise phase1.ReviewRefusal(
            "recorded_length_unknown",
            "recorded fight length is absent or invalid",
            turns=fight.turns_taken)
    if fight.turns_taken > horizon:
        raise phase1.ReviewRefusal(
            "recorded_length_exceeds_horizon",
            f"recorded fight took {fight.turns_taken} turns, "
            f"exceeding horizon {horizon}",
            recorded_turns=fight.turns_taken, horizon=horizon)


def _history(raw_run: dict) -> list[dict]:
    return [point for act in raw_run["map_point_history"] for point in act]


def _stable_unique(values: Sequence[str]) -> list[str]:
    return list(dict.fromkeys(values))


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="emit one schema-v2 fight-review card JSON document")
    parser.add_argument("run", type=pathlib.Path, nargs="?")
    parser.add_argument("fight_index", type=int, nargs="?")
    parser.add_argument(
        "--simulator-identity", action="store_true",
        help="print this bundle's producer identity and exit; the worker "
             "asks the bundle rather than recomputing it (#1267)")
    parser.add_argument("-k", type=int, default=phase1.DEFAULT_K)
    parser.add_argument(
        "--sampling-seed", default=phase1.DEFAULT_SAMPLING_SEED)
    parser.add_argument("--horizon", type=int, default=phase1.DEFAULT_HORIZON)
    parser.add_argument(
        "--actual-deadline", type=phase1._deadline,
        default=phase1.DEFAULT_ACTUAL_DEADLINE,
        help="seconds, or 'none' for an exact unbounded solve")
    parser.add_argument(
        "--benchmark-deadline", type=phase1._deadline,
        default=phase1.DEFAULT_BENCHMARK_DEADLINE,
        help="seconds per sampled world, or 'none'")
    parser.add_argument(
        "--fully-unlocked-card-pool", action="store_true",
        help="assert fully unlocked card and potion pools below A10")
    parser.add_argument("--counter", type=int, dest="shuffle_counter")
    parser.add_argument("--sel-counter", type=int, dest="selection_counter")
    parser.add_argument("--targets-counter", type=int)
    parser.add_argument("--energy-costs-counter", type=int)
    parser.add_argument("--generation-counter", type=int)
    parser.add_argument("--potion-generation-counter", type=int)
    parser.add_argument("--orb-generation-counter", type=int)
    parser.add_argument(
        "--hold-potions", type=int, default=0, dest="potion_hold_count",
        help="solve for lines that keep at least this many of the entry "
             "belt's drinkable potions undrunk")
    parser.add_argument(
        "--hold-slot", type=int, action="append", default=[],
        dest="potion_hold_slots",
        help="never drink the entry belt's potion in this slot (repeatable)")
    parser.add_argument(
        "--rust-exact-solver", type=pathlib.Path,
        default=rust_exact_solve.default_binary(),
        help="absolute sts-sim exact-solve binary; defaults to the deployed "
             "STS_SIM_EXACT_SOLVER/release artifact when present")
    parser.add_argument("--recorded-replay", type=pathlib.Path)
    parser.add_argument("--line-replay-status", type=pathlib.Path)
    parser.add_argument("--floor-saves", type=pathlib.Path, help="worker-verified captured start snapshots for Rust search")
    parser.add_argument(
        "--branch-lineage", default="[]",
        help="a branch run's ancestry as JSON [[source_start_time, "
             "source_floor], ...], nearest first (#3373); admits the recorded "
             "replay's source-run start_time below the branch floor")
    parser.add_argument("--provenance-entry", type=pathlib.Path)
    parser.add_argument("--provenance-entry-status", type=pathlib.Path)
    parser.add_argument("--out", type=pathlib.Path)
    parser.add_argument(
        "--no-print", action="store_true",
        help="write only --out; invalid without --out")
    args = parser.parse_args(argv)
    if args.no_print and args.out is None:
        parser.error("--no-print requires --out")
    if not args.simulator_identity and (
            args.run is None or args.fight_index is None):
        parser.error("run and fight_index are required")
    import review_provenance
    try:
        args.branch_lineage = review_provenance.parse_lineage(
            json.loads(args.branch_lineage))
    except ValueError as exc:  # JSONDecodeError is a ValueError
        parser.error(f"--branch-lineage: {exc}")
    return args


def simulator_identity_document(
        rust_exact_solver_binary: pathlib.Path | None = None) -> dict:
    """What the worker keys document freshness on (#1267).

    The bundle is asked rather than re-derived: a worker that computed the
    digest itself would be a second implementation of the semantic surface,
    free to disagree with the one that actually stamps documents.
    """

    return {
        "schema_version": SCHEMA_VERSION,
        "simulator": sim_identity.identity(rust_exact_solver_binary),
    }


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    if args.simulator_identity:
        sys.stdout.write(json.dumps(
            simulator_identity_document(args.rust_exact_solver),
            indent=2, sort_keys=True) + "\n")
        return 0
    config = ReviewConfig(
        k=args.k,
        sampling_seed=args.sampling_seed,
        horizon=args.horizon,
        actual_deadline=args.actual_deadline,
        benchmark_deadline=args.benchmark_deadline,
        fully_unlocked_card_pool=args.fully_unlocked_card_pool,
        shuffle_counter=args.shuffle_counter,
        selection_counter=args.selection_counter,
        targets_counter=args.targets_counter,
        energy_costs_counter=args.energy_costs_counter,
        generation_counter=args.generation_counter,
        potion_generation_counter=args.potion_generation_counter,
        orb_generation_counter=args.orb_generation_counter,
        potion_hold_count=args.potion_hold_count,
        potion_hold_slots=tuple(args.potion_hold_slots),
        rust_exact_solver_binary=args.rust_exact_solver,
        recorded_replay=phase1.load_recorded_replay(
            args.recorded_replay, args.line_replay_status),
        line_replay_status_path=args.line_replay_status)
    # Read (and, on a transport failure, reported to the worker through
    # --provenance-entry-status) on every path, as before #2827 item F1.
    provenance_entry = load_provenance_entry(
        args.provenance_entry, args.provenance_entry_status)
    if args.floor_saves is not None or (
            args.provenance_entry is not None and config.recorded_replay is not None):
        import rust_review
        document = rust_review.generate(args.run, args.fight_index,
                                        json.loads(args.floor_saves.read_text()) if args.floor_saves else None, config,
                                        provenance_entry=provenance_entry,
                                        branch_lineage=args.branch_lineage)
    else:
        # The `.run`-only review roots from the `.run` facts; a resolved save
        # projection without its capture is not used for the root.
        document = generate_review_document(
            args.run, args.fight_index, config=config)
    rendered = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if args.out is not None:
        args.out.write_text(rendered)
    if not args.no_print:
        sys.stdout.write(rendered)
    return 2 if document["status"] in NO_CARD_STATUSES else 0


if __name__ == "__main__":
    raise SystemExit(main())
