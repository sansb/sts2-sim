"""A resolved save projection, checked against the `.run` and its capture.

An uploaded fight comes with a save projection (the candidate entry the
uploader resolved) and the capture's own embedded run. Before the capture run
roots a fight (`rust_review.replay_capture_run`, which the eval suite's upload
import calls), the projection must be complete and must agree with every
unambiguous `.run` fact: ``provenance_entry_for_fight`` is that gate.

These checks were part of the fight-review CLI (`review_summary_v2.py`) until
the Python review path was deleted on 2026-10-11; they moved here unchanged.
"""

from __future__ import annotations

import copy
import json
import pathlib
import sys
from collections import Counter
from dataclasses import asdict, dataclass
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).parent))
sys.path.insert(0, str(pathlib.Path(__file__).parent / "tools"))

import relay_parser as relay  # noqa: E402
# The save/capture entry adapter (`build_entry` and the unlock-state readers),
# which `live_coach` owns.
import live_coach  # noqa: E402
from relay_parser import FightState  # noqa: E402


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

    player = _with_cards_added_after_the_save(
        fight, _complete_projected_player(projection), replay)
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


def _with_cards_added_after_the_save(
        fight: FightState, player: dict[str, Any],
        replay: dict[str, Any] | None) -> dict[str, Any]:
    """Append the cards the capture shows were gained after the save.

    The resolved save candidate is the save the uploader last saw before the
    capture changed, matched to the capture by RNG counters. Taking a card
    reward moves no counter, so that save can be the one written while the
    previous node's reward was still on screen: one card short of the deck
    the fight started with (#3756, STGS76WZ6WFA node 20: 25 cards saved 70 s
    before a capture whose own snapshot, and the `.run`, both hold 26).

    The capture's snapshot is the game's own statement of that deck, taken
    as the map point is entered: `RunManager/<EnterMapPointInternal>d__192
    ::MoveNext` (RVA 0x30bb3c) passes `RunManager::ToSave` (IL_01e8) to
    `CombatReplayWriter::RecordInitialState` (IL_01ed; RVA 0x69928), which
    keeps it as the replay's run (IL_00da), and `CombatReplay::Serialize`
    (RVA 0x69248) writes it (IL_00f9). v0.111.0 sts2.dll, sha256
    9cb4f1ad...

    So when all of these hold the missing rows are taken from the snapshot,
    and every later check runs on the completed deck as it would on a save
    written a moment later:

    * the snapshot's card ids equal the `.run` deck's, as multisets (two
      records of the entry deck agree, and the save disagrees with both);
    * the save's deck is a leading run of the snapshot's, id for id (the
      game appends a gained card; a removal or transform breaks this);
    * each missing row is a bare card: no enchantment, no saved props, not
      an id whose persistent fields the gate checks.

    Otherwise the player is returned untouched and `deck_membership` (or the
    check that applies) refuses as before.
    """

    if replay is None:
        return player
    save_deck = player["deck"]
    save_ids = [row.get("id") for row in save_deck]
    run_ids = Counter(row.get("id") for row in fight.deck_entering)
    if Counter(save_ids) == run_ids:
        return player
    try:
        snapshot = relay.require_single_player(
            replay["run"], "MCR replay")["deck"]
    except (KeyError, TypeError, ValueError, NotImplementedError):
        return player
    if not isinstance(snapshot, list) or any(
            not isinstance(row, dict) or not isinstance(row.get("id"), str)
            for row in snapshot):
        return player
    snapshot_ids = [
        "CARD." + row["id"].removeprefix("CARD.") for row in snapshot]
    if Counter(snapshot_ids) != run_ids or \
            len(snapshot_ids) <= len(save_ids) or \
            snapshot_ids[:len(save_ids)] != save_ids:
        return player
    added = []
    for index in range(len(save_ids), len(snapshot)):
        row = snapshot[index]
        if type(row.get("upgrade_level")) is not int or \
                row["upgrade_level"] < 0 or \
                row.get("enchantment") is not None or \
                row.get("props") is not None or \
                snapshot_ids[index] in _PERSISTENT_CARD_IDS:
            return player
        added.append({
            "id": snapshot_ids[index], "copy_index": index,
            "enchantment": None, "upgrade_level": row["upgrade_level"]})
    return {**player, "deck": [*save_deck, *added]}


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
                not isinstance(relic.get("props"), dict) and \
                not _joss_paper_props_omitted(relic):
            raise ProvenanceEntryRejected(
                "provenance_entry_incomplete", "relic_persistent_props")
    return player


def _joss_paper_props_omitted(relic: dict[str, Any]) -> bool:
    """A Joss Paper row with no ``props`` is complete: both fields are zero.

    v0.111.0 sts2.dll: of JossPaper's two counters only ``CardsExhausted``
    carries ``[SavedProperty]``, and its attribute blob is
    ``01 00 02 00 00 00 00 00`` — ``SerializationCondition`` 2,
    ``SaveIfNotTypeDefault`` — so the game omits it at zero and the relic
    serializes with no ``props`` key at all. ``EtherealCount`` is never
    saved; ``JossPaper::AfterCombatEnd`` (RVA 0x95573) clears it. The save
    adapter decodes the omission the same way
    (``live_coach._strict_joss_paper_properties``), and
    ``_check_adapted_conflicts`` still compares the decoded zeros with the
    `.run` counters. Any other persistent relic saves unconditionally, so
    its missing ``props`` stays incomplete.
    """
    return relic["id"] == "RELIC.JOSS_PAPER" and relic.get("props") is None


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
        # A conflict needs two claims. The run parser writes ``props`` on
        # every copy of the ids whose persistent fields it reconstructs
        # (``_PERSISTENT_CARD_IDS``, even when the value is unknown) and on
        # no other card, so a run side with no ``props`` key says nothing:
        # the save's Spoils Map ``SpoilsActIndex``, Guilty ``CombatsSeen``
        # or Dowsing ``RoomsEntered`` (each ``[SavedProperty]`` AlwaysSave
        # in v0.111.0 sts2.dll) is then the only evidence, not a
        # disagreement. The capture carries the same fields to the engine.
        checks_props = any("props" in row for row in run_rows)
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

