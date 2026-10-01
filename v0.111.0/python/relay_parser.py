"""
Relay the Spire — run-file parser & fight-state reconstruction prototype.

Given a STS2 .run file (schema_version 9, tested on build v0.108.0), produce a
FightState for every combat node: the player's deck, relics, potions, HP, and
gold *entering* that fight, plus the encounter faced and observed outcome.

This is the input a seed-replay solver needs. Known reconstruction limits are
tracked explicitly in FightState.caveats rather than silently guessed.

Usage:
    python3 relay_parser.py path/to/file.run          # human-readable report
    python3 relay_parser.py path/to/file.run --json   # machine-readable
"""

from __future__ import annotations

import json
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field, asdict


# ---------------------------------------------------------------------------
# Known starting decks (verify against game data; extend per character).
# Ascender's Bane appears at A10+ — presence is ascension-dependent, so we
# treat floor-1 cards in the file as ground truth rather than hardcoding.
# ---------------------------------------------------------------------------

@dataclass
class FightState:
    """Everything a solver needs to replay one combat."""
    node_index: int
    node_type: str                      # monster | elite | boss | event
    encounter_id: str
    monster_ids: list[str]
    # --- entry state ---
    hp_entering: int
    max_hp_entering: int
    gold_entering: int
    deck_entering: list[dict]           # [{id, upgrade_level, floor_added}]
    relics_entering: list[str]
    potions_entering: list[str]         # best-effort, see caveats
    # --- observed outcome (what the player actually did) ---
    damage_taken: int
    hp_healed: int
    turns_taken: int
    potions_used: list[str]
    hp_after: int
    # Exact per-fight history bit written by FurCoat.BeforeCombatStart.
    # None means the field is absent/malformed; the entry (Rust since #2827;
    # the deleted Python start_combat before it) refuses an owned Fur Coat instead of backfilling from the endpoint relic props.
    fur_coat_active: bool | None = None
    # --- trust ---
    caveats: list[str] = field(default_factory=list)
    # Event-node fights only: the .run does not order the event's own
    # effects (hp loss/heal, cards granted) against the combat, so the
    # entry state below is the node-entry snapshot, not necessarily the
    # combat-start state. The entry refuses these (I5); the unlock
    # path is a per-event effect census keyed by the logged
    # event_choices (event id + option picked).
    entry_ambiguous: bool = False
    # A picked potion may replace an unrecorded slot when the belt is full.
    # The parser carries every belt consistent with those replacements and
    # narrows them with later potion-use evidence.  Multiple survivors mean
    # the inventory (and therefore potion indexes) is not safe to simulate.
    potion_entry_ambiguous: bool = False
    # --- cross-combat relic state (post-pass; see the notes there) ---
    # None = relic present but the charge could not be dated (sim refuses)
    tea_set_charged: bool | None = None
    fake_tea_set_charged: bool | None = None
    # {relic_id: persistent counter modulo that relic's exact period entering
    # this fight}; a missing key for an owned counter relic means
    # "unseedable" (sim refuses)
    relic_counters: dict = field(default_factory=dict)


@dataclass
class RunSummary:
    seed: str
    build_id: str
    schema_version: int
    character: str
    ascension: int
    win: bool
    killed_by: str | None
    fights: list[FightState] = field(default_factory=list)
    global_caveats: list[str] = field(default_factory=list)


def require_single_player(data: dict, source: str):
    """Return the sole player or fail closed on unsupported multiplayer.

    The solver has one player State and cannot represent another player's
    piles, powers, potion targets, or RNG ownership.  Every raw run/save
    adapter must call this before selecting ``players[0]``.
    """
    players = data.get("players")
    if not isinstance(players, list) or len(players) != 1:
        count = len(players) if isinstance(players, list) else "missing"
        raise NotImplementedError(
            f"{source} contains {count} players; solver entry supports "
            "exactly one player and refuses multiplayer rather than "
            "silently selecting players[0] (SOLVER_INVARIANTS.md I5)")
    return players[0]


def _the_scythe_endpoint_growth(card: dict) -> int | None:
    """Read only enough of a .run endpoint to apply monotone dating."""
    if card.get("id") != "CARD.THE_SCYTHE":
        return None
    props = card.get("props")
    ints = props.get("ints") if isinstance(props, dict) else None
    if (not isinstance(ints, list) or len(ints) != 2
            or any(not isinstance(row, dict)
                   or set(row) != {"name", "value"}
                   or not isinstance(row.get("name"), str)
                   or type(row.get("value")) is not int
                   for row in ints)):
        return None
    values = {row["name"]: row["value"] for row in ints}
    current = values.get("CurrentDamage")
    growth = values.get("IncreasedDamage")
    if (len(values) != 2
            or set(values) != {"CurrentDamage", "IncreasedDamage"}
            or type(current) is not int or type(growth) is not int
            or growth < 0 or current != 13 + growth):
        return None
    return growth


def _the_scythe_historical_entry_props(
        card: dict, floor_no: int,
        combat_floors: list[int]) -> tuple[dict | None, bool]:
    """Date a post-fight mutable endpoint without backfilling growth."""
    if card.get("id") != "CARD.THE_SCYTHE":
        return None, False
    growth = _the_scythe_endpoint_growth(card)
    if growth == 0:
        # The fields only increase. A zero final/removal endpoint proves
        # that every eligible earlier entry was also the native 13/0.
        return card.get("props"), False
    added = card.get("floor_added_to_deck", 1)
    first = next((floor for floor in combat_floors
                  if added < floor), None)
    if growth is not None and floor_no == first:
        # A fresh acquired/starter deck object has not yet had a combat body
        # that could grow it. Positive later endpoints do not backfill into
        # this first eligible fight.
        return {
            "ints": [
                {"name": "CurrentDamage", "value": 13},
                {"name": "IncreasedDamage", "value": 0},
            ],
        }, False
    # A positive endpoint proves that growth happened by the snapshot, but
    # .run omits the exact prior play/fight. Missing or malformed props are
    # likewise not safe to default.
    return card.get("props"), True


def _genetic_algorithm_endpoint_growth(card: dict) -> int | None:
    """Read a Genetic Algorithm endpoint's exact monotone mutable fields."""
    if card.get("id") != "CARD.GENETIC_ALGORITHM":
        return None
    props = card.get("props")
    ints = props.get("ints") if isinstance(props, dict) else None
    if (not isinstance(ints, list) or len(ints) != 2
            or any(not isinstance(row, dict)
                   or set(row) != {"name", "value"}
                   or not isinstance(row.get("name"), str)
                   or type(row.get("value")) is not int
                   for row in ints)):
        return None
    values = {row["name"]: row["value"] for row in ints}
    current = values.get("CurrentBlock")
    growth = values.get("IncreasedBlock")
    if (len(values) != 2
            or set(values) != {"CurrentBlock", "IncreasedBlock"}
            or type(current) is not int or type(growth) is not int
            or growth < 0 or current != 1 + growth):
        return None
    return growth


def _genetic_algorithm_historical_entry_props(
        card: dict, floor_no: int,
        combat_floors: list[int]) -> tuple[dict | None, bool]:
    """Date a post-fight Genetic Algorithm endpoint conservatively."""
    if card.get("id") != "CARD.GENETIC_ALGORITHM":
        return None, False
    growth = _genetic_algorithm_endpoint_growth(card)
    if growth == 0:
        return card.get("props"), False
    added = card.get("floor_added_to_deck", 1)
    first = next((floor for floor in combat_floors
                  if added < floor), None)
    if growth is not None and floor_no == first:
        return {
            "ints": [
                {"name": "CurrentBlock", "value": 1},
                {"name": "IncreasedBlock", "value": 0},
            ],
        }, False
    return card.get("props"), True


def _persistent_card_historical_entry_props(
        card: dict, floor_no: int,
        combat_floors: list[int]) -> tuple[dict | None, bool]:
    if card.get("id") == "CARD.THE_SCYTHE":
        return _the_scythe_historical_entry_props(
            card, floor_no, combat_floors)
    if card.get("id") == "CARD.GENETIC_ALGORITHM":
        return _genetic_algorithm_historical_entry_props(
            card, floor_no, combat_floors)
    if card.get("id") == "CARD.MAD_SCIENCE":
        # Tinker Time assigns these immutable fields at physical card
        # creation; unlike growing DynamicVars, the endpoint is the exact
        # value at every historical entry where that object existed.
        return card.get("props"), False
    return None, False


def parse_run(path: str) -> RunSummary:
    with open(path) as f:
        data = json.load(f)

    player = require_single_player(data, ".run")
    # map_point_history is one node-list PER ACT (verified against a 3-act
    # win: lens like [17, 16, 15], each act starting with an 'ancient' start
    # node). Concatenate acts; floor numbers are global and 1-indexed across
    # the whole run, every node type counts.
    history = [pt for act in data["map_point_history"] for pt in act]
    final_deck = player["deck"]
    relic_spans = _relic_spans(history, player)

    summary = RunSummary(
        seed=data["seed"],
        build_id=data["build_id"],
        schema_version=data["schema_version"],
        character=player["character"],
        ascension=data["ascension"],
        win=data["win"],
        killed_by=(data.get("killed_by_encounter")
                   if data.get("killed_by_encounter", "").upper() not in ("", "NONE.NONE")
                   else None),
    )

    # ---- Global validation: additions in final deck vs. cards_gained log ----
    # Every deck entry has floor_added_to_deck. Every combat/shop node logs
    # cards_gained. If a card was gained per the node log but is absent from
    # the final deck, it was removed/transformed later (invisible event).
    gained_log: Counter = Counter()
    for pt in history:
        ps = pt["player_stats"][0]
        for c in ps.get("cards_gained", []):
            gained_log[c["id"]] += 1
    # ---- Transform dating (cards_transformed per node) ----
    # Archaic Tooth-style effects swap a card for another. The record is
    # complete: {original_card, final_card}, each with id +
    # floor_added_to_deck. The FINAL card is an ordinary final-deck
    # entry gated by its own floor stamp; the ORIGINAL is an exactly
    # dated removal and reuses the removal re-insertion machinery below.
    # (Found via the HQPAXCBS6P hand-replay pin — issue #114: Bash was
    # provably in the deck for every pre-transform fight, yet absent
    # from the reconstruction because this field was never consumed.)
    transform_events: list = []                  # (node_index, original)
    transform_gained: Counter = Counter()
    transform_original: Counter = Counter()
    transform_final_cards: list[dict] = []
    transformed_swoop_floors: Counter = Counter()
    for i, pt in enumerate(history):
        ps = pt["player_stats"][0]
        for t in ps.get("cards_transformed", []):
            transform_events.append((i, t["original_card"]))
            transform_gained[t["final_card"]["id"]] += 1
            transform_original[t["original_card"]["id"]] += 1
            # The serialized final CardModel is an exact, dated endpoint.
            # Its upgrade can be inherited by an explicit replacement
            # (Claws' CreateMaulFromOriginal) without a separate
            # upgraded_cards event, so it is explained provenance rather
            # than an unknown-floor smith.
            transform_final_cards.append(t["final_card"])
            if t["final_card"]["id"] == "CARD.BYRD_SWOOP":
                transformed_swoop_floors[
                    t["final_card"].get("floor_added_to_deck", 1)] += 1

    # ---- Removal dating (cards_removed per node) ----
    # The final deck array is the END-of-run deck: a card removed mid-run
    # (purge event, transform) is absent from it, so pre-removal fights
    # must get it re-inserted. Each node's player_stats logs cards_removed
    # with the card's state at removal — exact dating. (The gained-log
    # heuristic below cannot see removed STARTING cards at all: the
    # 7MA0PY7AD4 lesson, where a floor-1 Strike purged at an event
    # silently shrank every earlier fight's deck by one.)
    # Build this before Byrdpip provenance classification: surviving final
    # endpoints and later-removed endpoints form one identity pool. Claiming
    # logged Swoops from final_deck first can otherwise swap a surviving
    # automatic transform with a removed logged transform on the same floor.
    removal_events: list = []                    # (node_index, card entry)
    for i, pt in enumerate(history):
        ps = pt["player_stats"][0]
        for c in ps.get("cards_removed", []):
            removal_events.append((i, c))
    # A transform's original card is a dated removal (same re-insertion
    # semantics: in the deck for fights after its floor_added and before
    # the transform node).
    removal_events.extend(transform_events)
    # Credit only transform upgrades that are guaranteed to belong to a
    # surviving endpoint. A later removed/transformed card with the same
    # id/floor may have been the logged replacement; count it against the
    # possible survivors. Equal ordinary copies make this deliberately
    # conservative (a false unknown-floor caveat is safer than hiding one).
    transform_levels_by_group: dict[tuple, list[int]] = {}
    for card in transform_final_cards:
        key = (card["id"], card.get("floor_added_to_deck", 1))
        transform_levels_by_group.setdefault(key, []).append(
            card.get("current_upgrade_level", 0))
    removed_by_group = Counter(
        (card["id"], card.get("floor_added_to_deck", 1))
        for _floor, card in removal_events)
    final_by_group = Counter(
        (card["id"], card.get("floor_added_to_deck", 1))
        for card in final_deck)
    transform_upgrade_levels: Counter = Counter()
    for key, levels in transform_levels_by_group.items():
        guaranteed_survivors = min(
            final_by_group[key],
            max(0, len(levels) - removed_by_group[key]))
        transform_upgrade_levels[key[0]] += sum(
            sorted(levels)[:guaranteed_survivors])

    # Byrdpip.AfterObtained transforms every then-live Byrdonis Egg into a
    # level-1 Byrd Swoop without emitting the ordinary cards_transformed run
    # ledger. Identify only those unlogged endpoints. This both keeps the
    # global gained/removed validation honest and lets historical fights
    # reconstruct the original Egg below.
    byrdpip_floor = next(
        (r.get("floor_added_to_deck", 1) for r in player["relics"]
         if r["id"] == "RELIC.BYRDPIP"), None)
    unclaimed_logged_swoops = transformed_swoop_floors.copy()
    byrdpip_auto_indexes = set()
    byrdpip_transform_ambiguous_indexes = set()
    byrdpip_transform_ambiguous_removal_indexes = set()
    if byrdpip_floor is not None:
        swoop_candidates_by_floor = {}
        for card_index, card in enumerate(final_deck):
            floor = card.get("floor_added_to_deck", 1)
            if (card["id"] == "CARD.BYRD_SWOOP"
                    and floor < byrdpip_floor):
                swoop_candidates_by_floor.setdefault(
                    floor, []).append(("final", card_index))
        for removal_index, (_event_index, card) in enumerate(removal_events):
            floor = card.get("floor_added_to_deck", 1)
            if (card.get("id") == "CARD.BYRD_SWOOP"
                    and floor < byrdpip_floor):
                swoop_candidates_by_floor.setdefault(
                    floor, []).append(("removed", removal_index))
        ambiguous_floors = set()
        for floor, candidates in swoop_candidates_by_floor.items():
            logged = min(
                len(candidates), transformed_swoop_floors[floor])
            if 0 < logged < len(candidates):
                ambiguous_floors.add(floor)
                byrdpip_transform_ambiguous_indexes.update(
                    index for location, index in candidates
                    if location == "final")
                byrdpip_transform_ambiguous_removal_indexes.update(
                    index for location, index in candidates
                    if location == "removed")
        if ambiguous_floors:
            summary.global_caveats.append(
                "Byrdpip logged versus automatic Swoop identity is "
                "ambiguous across surviving/removed same-floor endpoints: "
                f"{sorted(ambiguous_floors)}")
        for card_index, card in enumerate(final_deck):
            floor = card.get("floor_added_to_deck", 1)
            if (card["id"] != "CARD.BYRD_SWOOP"
                    or floor >= byrdpip_floor):
                continue
            if unclaimed_logged_swoops[floor] > 0:
                unclaimed_logged_swoops[floor] -= 1
            else:
                byrdpip_auto_indexes.add(card_index)
    byrdpip_malformed_indexes = {
        card_index for card_index in byrdpip_auto_indexes
        if final_deck[card_index].get("current_upgrade_level", 0) != 1
    }
    if byrdpip_malformed_indexes:
        summary.global_caveats.append(
            "Byrdpip automatic Swoop endpoint is not canonical level 1; "
            "historical card identity is ambiguous")

    def validation_id(card_index, card):
        return ("CARD.BYRDONIS_EGG"
                if card_index in byrdpip_auto_indexes else card["id"])

    final_ids = Counter(
        validation_id(card_index, card)
        for card_index, card in enumerate(final_deck))
    floor1_ids = Counter(
        validation_id(card_index, card)
        for card_index, card in enumerate(final_deck)
        if card.get("floor_added_to_deck") == 1)
    # cards in final deck not explained by floor-1, the gained log, or a
    # transform's final side
    unexplained = final_ids - floor1_ids - gained_log - transform_gained
    # cards gained per log but missing from final deck -> removed at some
    # point (transformed originals are exactly dated, not unknown)
    removed = (gained_log - (final_ids - floor1_ids)
               - transform_original)
    if unexplained:
        summary.global_caveats.append(
            f"final deck contains cards with no recorded gain event: {dict(unexplained)} "
            f"(event reward or transform not captured by cards_gained)")
    if removed:
        summary.global_caveats.append(
            f"cards were gained but are absent from final deck: {dict(removed)} "
            f"— removal floor unknown; pre-removal deck reconstructions may "
            f"include a card the player had already purged, or vice versa")

    # The final deck array is the END-of-run deck: a card removed mid-run
    # is absent from it, so pre-removal fights re-insert the early-built
    # removal_events above.
    byrdpip_auto_removal_indexes = set()
    unclaimed_removed_swoops = unclaimed_logged_swoops.copy()
    if byrdpip_floor is not None:
        for removal_index, (_event_index, card) in enumerate(removal_events):
            floor = card.get("floor_added_to_deck", 1)
            if (card.get("id") != "CARD.BYRD_SWOOP"
                    or floor >= byrdpip_floor):
                continue
            if unclaimed_removed_swoops[floor] > 0:
                unclaimed_removed_swoops[floor] -= 1
            else:
                byrdpip_auto_removal_indexes.add(removal_index)
    byrdpip_malformed_removal_indexes = {
        removal_index for removal_index in byrdpip_auto_removal_indexes
        if removal_events[removal_index][1].get(
            "current_upgrade_level", 0) != 1
    }

    # ---- Upgrade dating (upgraded_cards per node + pick levels) ----
    # card_choices record the level at pick time (badge/pre-upgraded
    # rewards); rest-site smiths log upgraded_cards by id. A fight at node
    # i sees an upgrade iff its event node < i. When the id has a single
    # copy the dating is exact; with several copies the ARRAY SLOT that
    # got upgraded is unknowable => flagged (upgrade level is in
    # CardModel.CompareTo, so slot assignment changes shuffle order).
    pick_level: dict[tuple, int] = {}
    smith_nodes: list[int] = []
    upgrade_events: list = []                    # (node_index, card_id)
    for i, pt in enumerate(history):
        ps = pt["player_stats"][0]
        for ch in ps.get("card_choices", []):
            if ch.get("was_picked"):
                card = ch["card"]
                key = (card["id"], card.get("floor_added_to_deck"))
                pick_level[key] = card.get("current_upgrade_level", 0)
        for rc in ps.get("rest_site_choices", []):
            if rc == "SMITH":
                smith_nodes.append(i)
        for cid in ps.get("upgraded_cards", []):
            upgrade_events.append((i, cid))
    dated_upgrades: Counter = Counter(cid for _, cid in upgrade_events)
    # per-id ledger: total final levels must be explained by pick levels
    # plus dated upgrade events; anything left has an unknown floor
    final_lvl: Counter = Counter()
    picked_lvl: Counter = Counter()
    for card_index, c in enumerate(final_deck):
        # Byrdpip's unlogged Egg -> level-1 Swoop replacement is a card
        # transform, not an upgrade event. Its canonical level must not be
        # diagnosed as an unknown-floor smith.
        final_lvl[c["id"]] += (
            0 if card_index in byrdpip_auto_indexes
            else c.get("current_upgrade_level", 0))
        picked_lvl[c["id"]] += pick_level.get(
            (c["id"], c.get("floor_added_to_deck")), 0)
    undated = {cid: (final_lvl[cid] - picked_lvl[cid]
                    - dated_upgrades[cid] - transform_upgrade_levels[cid])
               for cid in final_lvl
               if (final_lvl[cid] - picked_lvl[cid]
                   - dated_upgrades[cid]
                   - transform_upgrade_levels[cid]) > 0}
    if undated:
        summary.global_caveats.append(
            f"cards upgraded on an unknown floor ({len(smith_nodes)} SMITH rests "
            f"seen): {undated} — entry decks show final upgrade level as an "
            f"approximation")
    # A Smith node gives an upper bound even when its card-id detail is absent
    # or insufficient to explain the endpoint levels.  With no Smith evidence,
    # the only safe upper bound is the end of the run.  Historical rows at or
    # before this node must advertise that their final endpoint level may not
    # yet have been reached; later rows are exact at the final level.
    last_possible_undated_upgrade_node = (
        smith_nodes[-1] if smith_nodes else len(history) - 1)

    # ---- Enchant-event dating ----
    # Each node's player_stats logs cards_enchanted: [{card, enchantment}].
    # Event at node i => the enchantment exists for fights at nodes > i
    # (same strict-< convention as floor_added_to_deck). Cards enchanted
    # in the final deck with NO logged event arrived enchanted (e.g.
    # Silken Tress card rewards) => active from floor_added. When a
    # (card id, floor_added, ench) group has several copies whose events
    # happened at different floors, fights between the first and last
    # event can't know WHICH copy was already enchanted => those entries
    # are flagged enchant_ambiguous (the sim refuses, I5). Goopy's monotone
    # amount uses the event snapshot for its first eligible later fight and
    # the endpoint equality/ordering rules documented below.
    ench_events: dict[tuple, list] = {}          # key -> [(floor, amount)]
    for i, pt in enumerate(history):
        ps = pt["player_stats"][0]
        for ev in ps.get("cards_enchanted", []):
            card = ev.get("card", {})
            ench = (card.get("enchantment") or {})
            key = (card.get("id"), card.get("floor_added_to_deck", 1),
                   ev.get("enchantment") or ench.get("id"))
            ench_events.setdefault(key, []).append(
                (i + 1, ench.get("amount", 1)))
    ench_group_n: Counter = Counter()            # endpoint copies per key
    for c in final_deck:
        e = c.get("enchantment")
        if e:
            ench_group_n[(c["id"], c.get("floor_added_to_deck", 1),
                          e["id"])] += 1
    for _, c in removal_events:
        e = c.get("enchantment")
        if e:
            ench_group_n[(c["id"], c.get("floor_added_to_deck", 1),
                          e["id"])] += 1

    # Run-only Goopy provenance: final-deck and removal payloads are post-fight
    # endpoints, not authoritative entry snapshots. With no logged enchant
    # event, every positive historical Goopy amount is therefore ambiguous;
    # zero alone proves no growth by monotonicity.
    # A unique logged event supplies the exact amount for the first eligible
    # post-event fight. Later fights are exact only when the endpoint amount
    # still equals the event amount: Goopy is monotone, so equality proves it
    # never grew. Exact live/save/direct fight entries bypass relay_parser.
    combat_floors = []
    for j, point in enumerate(history):
        point_type = point["map_point_type"]
        point_rooms = point.get("rooms") or []
        combat_room = next((r for r in point_rooms
                            if r.get("room_type") in (
                                "monster", "elite", "boss")),
                           point_rooms[0] if point_rooms else {})
        event_combat = (point_type == "unknown"
                        and bool(combat_room.get("turns_taken")))
        if point_type in ("monster", "elite", "boss") or event_combat:
            combat_floors.append(j + 1)

    def first_eligible_fight_after(c, event_floor):
        added = c.get("floor_added_to_deck", 1)
        return next((floor for floor in combat_floors
                     if event_floor < floor and added < floor), None)

    def enchant_state(c, floor_no):
        """-> (ench_id | None, amount, ambiguous) for this card entering
        the fight at floor_no."""
        e = c.get("enchantment")
        if not e:
            return None, None, False
        key = (c["id"], c.get("floor_added_to_deck", 1), e["id"])
        amount = e.get("amount", 1)
        events = sorted(ench_events.get(key, []))
        floors = [f for f, _ in events]
        if not floors:                           # arrived enchanted
            # Goopy can only increase. A zero endpoint proves zero growth;
            # every positive post-fight endpoint lacks an entry snapshot.
            ambiguous = (e["id"] == "ENCHANTMENT.GOOPY" and amount > 0)
            return e["id"], amount, ambiguous
        amount_static = all(a == amount for _, a in events)
        if floor_no > max(floors) and amount_static:
            return e["id"], amount, False
        if floor_no <= min(floors) \
                and len(floors) >= ench_group_n[key]:
            return None, None, False             # not enchanted yet
        if (e["id"] == "ENCHANTMENT.GOOPY"
                and len(events) == 1 and ench_group_n[key] == 1):
            event_floor, event_amount = events[0]
            if floor_no <= event_floor:
                return None, None, False
            if (amount > event_amount
                    and floor_no == first_eligible_fight_after(
                        c, event_floor)):
                return e["id"], event_amount, False
        return e["id"], amount, True             # timing/amount ambiguous

    # ---- Walk nodes, maintain running state ----
    # Potions: a full-belt picked reward replaces one occupied slot, but the
    # .run does not say which one.  Carry every ordered belt consistent with
    # the history; later potion uses often collapse the alternatives exactly.
    # This avoids the old behavior of silently dropping full-belt picks.
    potion_belts: set[tuple[str, ...]] = {()}
    potion_history_unresolved = False
    max_slots = player.get("max_potion_slot_count", 2)

    prev_hp = None
    prev_max_hp = None
    prev_gold = None

    for i, pt in enumerate(history):
        ps = pt["player_stats"][0]
        ntype = pt["map_point_type"]
        rooms = pt.get("rooms") or []
        # The combat room is identified by its own room_type, not the
        # node type or slot: event ('unknown') nodes host real combats —
        # either as a single monster-typed room or as [event, monster]
        # two-room nodes (508 in the local history, 51 of them two-room).
        room = next((r for r in rooms
                     if r.get("room_type") in ("monster", "elite", "boss")),
                    rooms[0] if rooms else {})
        event_combat = (ntype == "unknown"
                        and bool(room.get("turns_taken")))

        used_potions = ps.get("potion_used", [])
        compatible_belts = {
            belt for belt in potion_belts
            if not (Counter(used_potions) - Counter(belt))
        }
        potion_use_conflict = bool(used_potions) and not compatible_belts
        if compatible_belts:
            # A use is positive evidence about the inventory entering this
            # node.  Narrow before emitting the FightState so a later fight
            # can resolve an earlier full-belt replacement.
            potion_belts = compatible_belts
        elif potion_use_conflict:
            potion_history_unresolved = True

        # Entry state = previous node's exit state; node 0 derives backwards.
        hp_entering = prev_hp if prev_hp is not None else (
            ps["current_hp"] + ps["damage_taken"] - ps["hp_healed"])
        max_hp_entering = prev_max_hp if prev_max_hp is not None else (
            ps["max_hp"] - ps["max_hp_gained"] + ps["max_hp_lost"])
        gold_entering = prev_gold if prev_gold is not None else (
            ps["current_gold"] - ps["gold_gained"]
            + ps["gold_lost"] + ps["gold_spent"] + ps["gold_stolen"])

        if ntype in ("monster", "elite", "boss") or event_combat:
            # floor_added_to_deck semantics (verified 2026-07-08 against the
            # full local run history, ~2600 pick events, zero exceptions):
            # floor N = global node index N-1 (1-indexed, all node types
            # count, incl. each act's 'ancient' start node; starting deck is
            # floor 1). A card picked from the floor-N combat reward gets
            # floor_added_to_deck = N — i.e. it was NOT in the deck during
            # the floor-N fight, so the entry filter is strictly <.
            floor_no = i + 1
            deck_entering = []
            ambiguous_ench = []
            ambiguous_props = []
            ambiguous_undated_upgrades = set()
            fight_caveats = []
            for card_index, c in enumerate(final_deck):
                if c["floor_added_to_deck"] >= floor_no:
                    continue
                ench_id, amount, ambiguous = enchant_state(c, floor_no)
                byrdpip_before_transform = (
                    card_index in byrdpip_auto_indexes
                    and byrdpip_floor is not None
                    and floor_no <= byrdpip_floor)
                entry = {"id": ("CARD.BYRDONIS_EGG"
                                if byrdpip_before_transform else c["id"]),
                         "upgrade_level": c.get("current_upgrade_level", 0),
                         "floor_added": c["floor_added_to_deck"],
                         "enchantment": ench_id}
                if (c["id"] in undated
                        and i <= last_possible_undated_upgrade_node):
                    entry["upgrade_ambiguous"] = True
                    ambiguous_undated_upgrades.add(c["id"])
                if (card_index in byrdpip_transform_ambiguous_indexes
                        and byrdpip_floor is not None
                        and floor_no <= byrdpip_floor):
                    entry["upgrade_ambiguous"] = True
                    fight_caveats.append(
                        "Byrdpip logged versus automatic Swoop identity is "
                        "ambiguous across surviving/removed same-floor "
                        "endpoints")
                if card_index in byrdpip_malformed_indexes:
                    entry["upgrade_ambiguous"] = True
                if byrdpip_before_transform:
                    # Byrdpip creates a fresh level-1 Byrd Swoop from the
                    # existing Egg. Historical entry snapshots at/before the
                    # acquisition floor must retain the unplayable level-0
                    # Egg. Enchanted endpoints lack enough transfer
                    # provenance to reverse exactly, so keep I5 explicit.
                    entry["upgrade_level"] = 0
                    if ench_id:
                        entry["enchant_ambiguous"] = True
                        ambiguous_ench.append((c["id"], ench_id))
                        fight_caveats.append(
                            "Byrdpip-transformed enchanted Byrd Swoop "
                            "cannot reconstruct the prior Egg enchantment")
                props, props_ambiguous = \
                    _persistent_card_historical_entry_props(
                        c, floor_no, combat_floors)
                if c["id"] in {
                        "CARD.THE_SCYTHE", "CARD.GENETIC_ALGORITHM",
                        "CARD.MAD_SCIENCE"}:
                    entry["props"] = props
                    if props_ambiguous:
                        entry["props_ambiguous"] = True
                        ambiguous_props.append(c["id"])
                if ench_id:
                    entry["enchant_amount"] = amount
                    if ambiguous:
                        entry["enchant_ambiguous"] = True
                        ambiguous_ench.append((c["id"], ench_id))
                deck_entering.append(entry)
            # cards removed at a LATER node were still in the deck here;
            # re-insert them at their save-array slot. The slot is exact
            # when an identical sibling (same id/floor/upgrade/ench)
            # survives — the removed copy sat in the same acquisition
            # batch; otherwise the batch position is a guess (flagged:
            # array order feeds the opening shuffle).
            for removal_index, (j, rc) in enumerate(removal_events):
                # j == i means the card left the deck AT this fight's own
                # node — in-combat removal (Thieving Hopper theft, #827)
                # or a post-fight reward-relic transform: either way the
                # fight itself began with the card, since fight-node
                # rewards only ADD. Event-node combats stay ambiguous
                # (their existing ordering caveat covers removals too).
                if j < i or rc.get("floor_added_to_deck", 1) >= floor_no:
                    continue
                re_ench, re_amount, re_ambiguous = enchant_state(
                    rc, floor_no)
                byrdpip_before_transform = (
                    removal_index in byrdpip_auto_removal_indexes
                    and byrdpip_floor is not None
                    and floor_no <= byrdpip_floor)
                entry = {"id": ("CARD.BYRDONIS_EGG"
                                if byrdpip_before_transform else rc["id"]),
                         "upgrade_level": rc.get("current_upgrade_level", 0),
                         "floor_added": rc.get("floor_added_to_deck", 1),
                         "enchantment": re_ench}
                if (rc["id"] in undated
                        and i <= last_possible_undated_upgrade_node):
                    entry["upgrade_ambiguous"] = True
                    ambiguous_undated_upgrades.add(rc["id"])
                if (removal_index in
                        byrdpip_transform_ambiguous_removal_indexes
                        and byrdpip_floor is not None
                        and floor_no <= byrdpip_floor):
                    entry["upgrade_ambiguous"] = True
                    fight_caveats.append(
                        "Byrdpip logged versus automatic Swoop identity is "
                        "ambiguous across surviving/removed same-floor "
                        "endpoints")
                if removal_index in byrdpip_malformed_removal_indexes:
                    entry["upgrade_ambiguous"] = True
                if byrdpip_before_transform:
                    entry["upgrade_level"] = 0
                    if re_ench:
                        entry["enchant_ambiguous"] = True
                        ambiguous_ench.append((rc["id"], re_ench))
                        fight_caveats.append(
                            "Byrdpip-transformed enchanted removed Byrd "
                            "Swoop cannot reconstruct the prior Egg "
                            "enchantment")
                props, props_ambiguous = \
                    _persistent_card_historical_entry_props(
                        rc, floor_no, combat_floors)
                if rc["id"] in {
                        "CARD.THE_SCYTHE", "CARD.GENETIC_ALGORITHM",
                        "CARD.MAD_SCIENCE"}:
                    entry["props"] = props
                    if props_ambiguous:
                        entry["props_ambiguous"] = True
                        ambiguous_props.append(rc["id"])
                if re_ench:
                    entry["enchant_amount"] = re_amount
                    if re_ambiguous:
                        entry["enchant_ambiguous"] = True
                        ambiguous_ench.append((rc["id"], re_ench))
                pos = None
                for k2, e2 in enumerate(deck_entering):
                    if (e2["id"], e2["floor_added"], e2["upgrade_level"],
                            e2["enchantment"]) == \
                            (entry["id"], entry["floor_added"],
                             entry["upgrade_level"], entry["enchantment"]):
                        pos = k2 + 1
                if pos is None:
                    # no identical sibling. Starter-floor cards belong in
                    # the CHARACTER block, before Ascender's Bane —
                    # floor-1 event grants (Greed) append after it, so
                    # "after the last floor <= 1 card" lands starters
                    # wrong; the HQPAXCBS6P hand pin proved the
                    # transformed Bash sat at the pre-Bane slot.
                    bane = next((k2 for k2, e2 in enumerate(deck_entering)
                                 if e2["id"] == "CARD.ASCENDERS_BANE"),
                                None)
                    if entry["floor_added"] == 1 and bane is not None:
                        pos = bane
                    else:
                        pos = 0
                        for k2, e2 in enumerate(deck_entering):
                            if e2["floor_added"] <= entry["floor_added"]:
                                pos = k2 + 1
                    fight_caveats.append(
                        f"removed card {rc['id']} re-inserted at an "
                        f"approximate array slot (no identical sibling) — "
                        f"opening-shuffle order may be off")
                deck_entering.insert(pos, entry)
            # upgrades dated to a LATER node hadn't happened yet: undo
            # them. Exact when the id has one copy here; with several the
            # upgraded SLOT is unknowable (level is in the reshuffle sort
            # key) — flag every copy upgrade_ambiguous.
            late_ups = Counter(cid for j, cid in upgrade_events if j > i)
            for cid, n in late_ups.items():
                copies = [e for e in deck_entering if e["id"] == cid]
                if len(copies) == 1:
                    copies[0]["upgrade_level"] = max(
                        0, copies[0]["upgrade_level"] - n)
                elif copies:
                    for e in copies:
                        e["upgrade_ambiguous"] = True
                    fight_caveats.append(
                        f"{n} later upgrade(s) of {cid} cannot be assigned "
                        f"to a copy — upgrade levels ambiguous")
            relics_entering = [
                rid for rid, acquired, removed in relic_spans
                if acquired < floor_no
                and (removed is None or floor_no <= removed)
            ]
            ordered_belts = sorted(potion_belts)
            representative_belt = list(ordered_belts[0])
            potion_entry_ambiguous = (
                potion_history_unresolved or len(ordered_belts) != 1)
            if Counter(used_potions) - Counter(representative_belt):
                # Preserve the invariant that every recorded use appears in
                # the emitted representative, even for malformed/incomplete
                # histories.  The ambiguity flag below makes this belt
                # non-simulatable; these names are evidence, not a guess.
                representative_belt = list(used_potions)
                potion_entry_ambiguous = True
            fight = FightState(
                node_index=i,
                node_type="event" if event_combat else ntype,
                encounter_id=room.get("model_id", "UNKNOWN"),
                monster_ids=room.get("monster_ids", []),
                hp_entering=hp_entering,
                max_hp_entering=max_hp_entering,
                gold_entering=gold_entering,
                deck_entering=deck_entering,
                relics_entering=relics_entering,
                potions_entering=representative_belt,
                damage_taken=ps["damage_taken"],
                hp_healed=ps["hp_healed"],
                turns_taken=room.get("turns_taken", -1),
                potions_used=used_potions,
                hp_after=ps["current_hp"],
                fur_coat_active=(
                    ps.get("is_affected_by_fur_coat")
                    if type(ps.get("is_affected_by_fur_coat")) is bool
                    else None),
                caveats=[],
                potion_entry_ambiguous=potion_entry_ambiguous,
            )
            if event_combat:
                event_room = next(
                    (r.get("model_id") for r in rooms
                     if r.get("room_type") == "event"), None)
                fight.entry_ambiguous = True
                fight.caveats.append(
                    "event-node fight"
                    + (f" ({event_room})" if event_room else "")
                    + ": the event's own effects (hp, cards, relics) are "
                    "unordered against the combat — entry state is the "
                    "node-entry snapshot, not necessarily combat-start")
            # Per-fight caveats
            fight.caveats.extend(fight_caveats)
            if potion_use_conflict:
                fight.caveats.append(
                    "recorded potion use is incompatible with every belt "
                    "reconstructable from prior potion choices; entry "
                    "inventory is unresolved")
            elif len(ordered_belts) != 1:
                rendered = [list(belt) for belt in ordered_belts]
                fight.caveats.append(
                    "full-belt potion replacement slot is unrecorded; "
                    f"entry inventory has {len(rendered)} candidates: "
                    f"{rendered}")
            missing_used = Counter(fight.potions_used) - Counter(
                fight.potions_entering)
            if missing_used:
                raise AssertionError(
                    "relay parser emitted potion uses absent from the "
                    f"entry belt at node {i}: {dict(missing_used)}")
            if ambiguous_ench:
                fight.caveats.append(
                    f"enchant timing/amount ambiguous entering this fight: "
                    f"{sorted(set(ambiguous_ench))}")
            if ambiguous_undated_upgrades:
                fight.caveats.append(
                    "final endpoint upgrade levels may not yet apply "
                    "entering this fight: "
                    f"{sorted(ambiguous_undated_upgrades)}")
            if ambiguous_props:
                fight.caveats.append(
                    "The Scythe mutable damage is a positive/malformed "
                    "post-fight endpoint whose prior-fight entry cannot be "
                    "dated exactly")
            if summary.global_caveats:
                fight.caveats.append("see global caveats (removals/upgrades)")
            summary.fights.append(fight)

        # ---- update running potion state ----
        if compatible_belts:
            remaining_belts = set()
            for belt in compatible_belts:
                remaining = list(belt)
                for used in used_potions:
                    remaining.remove(used)
                remaining_belts.add(tuple(remaining))
            potion_belts = remaining_belts
        elif potion_use_conflict:
            # The source history is incomplete.  Do not let later nodes look
            # exact merely because this node consumed an untracked potion.
            potion_belts = {()}
        for pc in ps.get("potion_choices", []):
            if not pc.get("was_picked"):
                continue
            choice = pc["choice"]
            next_belts = set()
            for belt in potion_belts:
                if len(belt) < max_slots:
                    next_belts.add(belt + (choice,))
                else:
                    # A picked choice at capacity proves a replacement.  The
                    # raw run omits its slot, so branch across every slot.
                    for slot in range(len(belt)):
                        replaced = list(belt)
                        replaced[slot] = choice
                        next_belts.add(tuple(replaced))
            if next_belts:
                potion_belts = next_belts
            else:
                potion_history_unresolved = True

        prev_hp = ps["current_hp"]
        prev_max_hp = ps["max_hp"]
        prev_gold = ps["current_gold"]

    _date_cross_combat_relic_state(summary, history, player, relic_spans)
    return summary


def _relic_spans(history, player):
    """Every relic the run owned, as ``(id, acquired_floor, removed_floor)``.

    The final ``player.relics`` list omits relics removed during the run
    (Touch of Orobas upgrading the starter relic, Relic Trader, Ranwid, Sword
    of Stone after its elites), but the node that removed one logs it in
    ``player_stats[0].relics_removed``. Its acquisition floor is the latest
    ``relic_choices`` pick of that id before the removal, or floor 1 when it
    was never picked (the starter relic). That ledger is complete: across the
    1,555-run local corpus every relic acquired after floor 1 has a pick at
    its ``floor_added_to_deck`` (7,813 of 7,813, 2026-09-23, #2921).

    A relic is owned entering the fight at floor F iff
    ``acquired < F <= removed``: a combat node's removal comes after its
    fight, and an event-combat entry is already the node-entry snapshot.
    Removed relics are placed before the first final relic acquired after
    them, which is the save's acquisition order for an in-place replacement
    such as Burning Blood -> Black Blood. A .run entry never claims a
    dispatch order (``relics_entering_dispatch_ordered`` stays False), so
    only membership is load-bearing."""
    spans = [(r["id"], r.get("floor_added_to_deck", 1), None)
             for r in player.get("relics", [])]
    picks = defaultdict(list)
    for node, pt in enumerate(history):
        floor = node + 1
        for stats in pt.get("player_stats", ())[:1]:
            for rid in stats.get("relics_removed") or ():
                acquired = max(
                    (f for f in picks[rid] if f < floor), default=1)
                if any(sid == rid and acq == acquired
                       for sid, acq, _removed in spans):
                    raise NotImplementedError(
                        f"relic removal at floor {floor} cannot be matched "
                        f"to a distinct acquisition of {rid}")
                span = (rid, acquired, floor)
                at = next(
                    (k for k, (_sid, acq, removed) in enumerate(spans)
                     if removed is None and acq > acquired), len(spans))
                spans.insert(at, span)
            for choice in stats.get("relic_choices", ()):
                if choice.get("was_picked"):
                    picks[choice["choice"]].append(floor)
    return spans


# Relics whose combat behavior depends on state carried BETWEEN combats.
_TEA_SET = "RELIC.VENERABLE_TEA_SET"
_FAKE_TEA_SET = "RELIC.FAKE_VENERABLE_TEA_SET"
_GIRYA = "RELIC.GIRYA"
_LIZARD_TAIL = "RELIC.LIZARD_TAIL"
_FINITE_COMBAT_RELIC_SEEDS = {
    "RELIC.BONE_TEA": 1,
    "RELIC.EMBER_TEA": 5,
    "RELIC.TEA_OF_DISCOURTESY": 1,
}
_PUMPKIN_CANDLE = "RELIC.PUMPKIN_CANDLE"
# Per-turn-start counter relics and their IL periods (`get_CanonicalVars`
# DynamicVar "Turns"). The .run can seed TurnsSeen exactly only before the
# relic's first owned combat (#3076, see _date_cross_combat_relic_state); the
# periods are kept as the membership set the review adapter reads.
_COUNTER_RELIC_MODS = {
    "RELIC.HAPPY_FLOWER": 3,
    "RELIC.FAKE_HAPPY_FLOWER": 5,
    "RELIC.PENDULUM": 3,
    "RELIC.POLLINOUS_CORE": 4,
}
# Card-play counters (Nunchaku, Pen Nib, Iron Club, Tuning Fork) and Galactic
# Dust's Stars-spend remainder persist across combats, but the .run records
# neither per-fight action history. Their seed is only known (0) when the
# relic has seen NO prior owned combat; a live save can supply exact props.
_ZERO_ONLY_COUNTER_RELICS = ("RELIC.NUNCHAKU", "RELIC.PEN_NIB",
                             "RELIC.GALACTIC_DUST", "RELIC.IRON_CLUB",
                             "RELIC.TUNING_FORK")


def _date_cross_combat_relic_state(summary, history, player,
                                  relic_spans=None):
    """Post-pass: date Tea Set and seed exact cross-combat relic state.

    Grounding (IL, 2026-07-11 session; turn counters re-read 2026-09-25):
    - TurnNumber increments in CombatManager.SwitchSides (RVA 0x2df690)
      on every enemy->player switch AND on every extra turn, and
      <EndCombatInternal>d__122::MoveNext (RVA 0x3f3e60) records the local
      player's TurnNumber as the room's TurnsTaken (IL_0059-IL_0063 read,
      IL_0311 set_TurnsTaken). So turns_taken == the number of player turn
      starts, the last one included.
    - Turn counters (#3076): turns_taken is NOT the number of counter ticks.
      Every hook dispatch walks Hook.IterateCombatHookListeners, whose
      iterator (<IterateCombatHookListeners>d__0::MoveNext, RVA 0x3d3bc0)
      yields nothing once CombatManager.IsOverOrEnding (IL_0028-IL_0042;
      IsEnding is true as soon as every enemy is dead, IsCombatEnding RVA
      0x135854). Happy Flower / Fake Happy Flower tick in AfterSideTurnStart
      (<AfterSideTurnStart>d__20 / d__21::MoveNext, RVA 0x326434 / 0x3241f4:
      TurnsSeen + 1 rem Turns at IL_003d-IL_005c; dump_type.py offsets),
      which <StartTurn>d__100::MoveNext (RVA 0x3f781c) dispatches at IL_07bf,
      AFTER BeforeSideTurnStart (IL_0203), the AfterTurnStart and
      AfterBlockCleared walks and SetupPlayerTurn (IL_06b9: energy reset,
      BeforeHandDraw, draw, AfterPlayerTurnStart). Pendulum and Pollinous
      Core tick in BeforeHandDraw (RVA 0x98fa0 / 0x9998c), dispatched inside
      SetupPlayerTurn (<SetupPlayerTurn>d__102 RVA 0x3f6c6c, IL_0177). A
      combat won during a player turn start, before the relic's hook walk
      begins, therefore counts that turn in turns_taken but never ticks it.
      The .run does not record where a combat ended, so every prior owned
      combat's last turn is a {0, 1} unknown. With any period >= 3 even one
      such combat leaves two candidates, so the counter is exact (0) only
      before the relic's first owned combat; later fights refuse by name.
      Witnesses: UE7YGG9XC3ZB floor 24 (8 turns, flower 1 -> 2) and
      NA4SLXKS9923 floors 30 and 45 (4 turns, 1 -> 1) all end with the last
      enemy dead at the "After player turn start" checkpoint; a Pendulum
      fight ending the same way (killed after BeforeHandDraw) still ticked.
    - EVENT ('unknown') nodes can host real combats (457 in the local
      history) — they tick the counters and consume Tea Set charges, and
      they record turns_taken like any combat, so they are counted here
      even though they are not FightState entries.
    - Both Tea Sets: AfterRoomEntered(RestSiteRoom) sets the charge (only
      if the relic is already owned — strict floor <, same convention as
      cards); the FIRST AfterEnergyReset of the next combat consumes it.
    - Bone Tea / Ember Tea / Tea of Discourtesy seed 1 / 5 / 1 and tick
      once per owned combat. An event-node acquisition can be on either
      side of that node's combat; retain both candidates until later
      definite combats make them converge.
    - Pumpkin Candle starts at 5 after acquisition, gains 5 per owned
      KINDLE rest choice, and loses 1 (floored at zero) per owned combat.
      Its event-node acquisition ambiguity is likewise carried as a set.
    - Girya: LiftRestSiteOption's logged OptionId is ``LIFT`` and its sole
      gameplay mutation increments TimesLifted once, capped by option
      availability at 3. Count prior owned LIFT choices exactly.
    - Lizard Tail's saved WasUsed bool is definitely false only before its
      first owned combat. A .run has no relic SavedProperties or action log,
      so later fights retain an explicit unseedable caveat; live saves carry
      the exact bool.
    - Joss Paper's persistent CardsExhausted remainder is likewise exactly
      zero only before its first owned combat. EtherealCount is reset by
      AfterCombatEnd, but both saved integers are required as one exact entry
      payload; later owned fights therefore refuse.
    - A relic obtained AT a combat-bearing event node has an unknowable
      order vs that node's fight -> the counter seed is ambiguous."""
    if relic_spans is None:
        relic_spans = _relic_spans(history, player)
    spans_by_id = defaultdict(list)
    for rid, acquired, removed in relic_spans:
        spans_by_id[rid].append((acquired, removed))

    def owned_at(rid, node):                     # strict <, floor = node+1
        return any(acquired < node + 1
                   and (removed is None or node + 1 <= removed)
                   for acquired, removed in spans_by_id[rid])

    def acquired_at(rid, node):
        return any(acquired == node + 1
                   for acquired, _removed in spans_by_id[rid])

    def acquired_before(rid, node):              # the span owning node's fight
        return max(acquired for acquired, removed in spans_by_id[rid]
                   if acquired < node + 1
                   and (removed is None or node + 1 <= removed))

    # every node that hosted a combat (fight nodes AND event combats).
    # Combat rooms are found by their own room_type, at ANY slot — the
    # 51 two-room event nodes ([event, monster]) hide their combat at
    # rooms[1], which the original rooms[0] scan missed.
    combat_nodes = []                            # (node_index, turns_taken)
    event_combat_nodes = set()                   # grant-vs-fight order
    #                                              unknown at these
    rest_nodes = []
    for j, pt in enumerate(history):
        rooms = pt.get("rooms") or []
        tt = next((r.get("turns_taken") for r in rooms
                   if r.get("room_type") in ("monster", "elite", "boss")
                   and r.get("turns_taken") is not None), None)
        if pt["map_point_type"] in ("monster", "elite", "boss"):
            combat_nodes.append((j, tt if tt is not None else -1))
        elif tt:                                 # event-node combat
            combat_nodes.append((j, tt))
            event_combat_nodes.add(j)
        if pt["map_point_type"] == "rest_site":
            rest_nodes.append(j)

    for f in summary.fights:
        i = f.node_index
        prior = [(j, tt) for j, tt in combat_nodes if j < i]
        if _TEA_SET in f.relics_entering:
            last_combat = max((j for j, _ in prior), default=-1)
            f.tea_set_charged = any(
                last_combat < j < i and owned_at(_TEA_SET, j)
                for j in rest_nodes)
        if _FAKE_TEA_SET in f.relics_entering:
            last_combat = max((j for j, _ in prior), default=-1)
            f.fake_tea_set_charged = any(
                last_combat < j < i and owned_at(_FAKE_TEA_SET, j)
                for j in rest_nodes)
        for rid, seed in _FINITE_COMBAT_RELIC_SEEDS.items():
            if rid not in f.relics_entering:
                continue
            definite = sum(owned_at(rid, j) for j, _ in prior)
            ambiguous = sum(
                acquired_at(rid, j)
                for j in event_combat_nodes if j < i)
            candidates = {
                max(0, seed - definite - extra)
                for extra in range(ambiguous + 1)
            }
            if len(candidates) == 1:
                f.relic_counters[rid] = candidates.pop()
            else:
                f.caveats.append(
                    f"{rid} CombatsLeft unseedable (relic obtained at a "
                    "combat-bearing event node)")
        if _PUMPKIN_CANDLE in f.relics_entering:
            acquired = acquired_before(_PUMPKIN_CANDLE, i)
            candidates = {5}
            combat_indexes = {j for j, _tt in combat_nodes}
            for j in range(i):
                if j + 1 < acquired:
                    continue
                if j + 1 == acquired:
                    if j in event_combat_nodes:
                        candidates |= {
                            max(0, value - 1) for value in candidates}
                    continue
                if j in combat_indexes:
                    candidates = {
                        max(0, value - 1) for value in candidates}
                if j in rest_nodes:
                    kindles = sum(
                        choice == "KINDLE"
                        for stats in history[j].get("player_stats", ())
                        for choice in stats.get("rest_site_choices", ()))
                    if kindles:
                        candidates = {
                            value + 5 * kindles for value in candidates}
            if len(candidates) == 1:
                f.relic_counters[_PUMPKIN_CANDLE] = candidates.pop()
            else:
                f.caveats.append(
                    f"{_PUMPKIN_CANDLE} KindleCount unseedable (relic "
                    "obtained at a combat-bearing event node)")
        if _GIRYA in f.relics_entering:
            f.relic_counters[_GIRYA] = sum(
                choice == "LIFT"
                for j in rest_nodes if j < i and owned_at(_GIRYA, j)
                for stats in history[j].get("player_stats", ())
                for choice in stats.get("rest_site_choices", ()))
        if _LIZARD_TAIL in f.relics_entering:
            could_have_fired = (
                any(owned_at(_LIZARD_TAIL, j) for j, _ in prior)
                or any(acquired_at(_LIZARD_TAIL, j)
                       for j in event_combat_nodes if j < i))
            if could_have_fired:
                f.caveats.append(
                    "RELIC.LIZARD_TAIL WasUsed unseedable after a prior "
                    "owned combat (.run omits SavedProperties)")
            else:
                f.relic_counters[_LIZARD_TAIL] = False
        if "RELIC.JOSS_PAPER" in f.relics_entering:
            prior_owned = any(
                owned_at("RELIC.JOSS_PAPER", j) for j, _ in prior)
            ambiguous_grant = any(
                acquired_at("RELIC.JOSS_PAPER", j)
                for j in event_combat_nodes if j < i)
            if prior_owned or ambiguous_grant:
                f.caveats.append(
                    "RELIC.JOSS_PAPER CardsExhausted remainder unseedable "
                    "after a prior owned combat (.run omits "
                    "SavedProperties)")
            else:
                f.relic_counters["RELIC.JOSS_PAPER"] = {
                    "CardsExhausted": 0, "EtherealCount": 0}
        for rid in _COUNTER_RELIC_MODS:
            if rid not in f.relics_entering:
                continue
            # combat-reward relics (floor == fight floor) are fine: the
            # reward comes after the fight, strict < already excludes it.
            # An EVENT node that both granted the relic and hosted a combat
            # has an unknowable order.
            if any(acquired_at(rid, j) for j in event_combat_nodes):
                f.caveats.append(
                    f"{rid} turn counter unseedable (relic obtained at a "
                    f"combat-bearing event node)")
                continue
            # #3076: each prior owned combat ticked turns_taken or
            # turns_taken - 1 times (a win during a turn start skips that
            # turn's hook walk), and the .run cannot say which.
            if any(owned_at(rid, j) for j, _ in prior):
                f.caveats.append(
                    f"{rid} turn counter unseedable after a prior owned "
                    f"combat (a combat won during a player turn start does "
                    f"not tick; the .run omits where combat ended)")
                continue
            f.relic_counters[rid] = 0
        for rid in _ZERO_ONLY_COUNTER_RELICS:
            if rid not in f.relics_entering:
                continue
            if any(owned_at(rid, j) for j, _ in prior) or any(
                    acquired_at(rid, j) for j in event_combat_nodes):
                if rid == "RELIC.GALACTIC_DUST":
                    f.caveats.append(
                        f"{rid} Stars-spend remainder unseedable (per-fight "
                        f"resource spends are not recorded)")
                elif rid == "RELIC.IRON_CLUB":
                    f.caveats.append(
                        f"{rid} card-play remainder unseedable (owned card "
                        f"plays in prior fights are not recorded)")
                elif rid == "RELIC.TUNING_FORK":
                    f.caveats.append(
                        f"{rid} Skill-play counter unseedable (owned Skill "
                        f"plays in prior fights are not recorded)")
                else:
                    f.caveats.append(
                        f"{rid} attack counter unseedable (attack plays in "
                        f"prior fights are not recorded)")
            else:
                f.relic_counters[rid] = 0


def report(s: RunSummary) -> str:
    lines = []
    lines.append(f"seed={s.seed}  build={s.build_id}  schema=v{s.schema_version}")
    lines.append(f"{s.character}  A{s.ascension}  "
                 f"{'WIN' if s.win else 'LOSS — ' + (s.killed_by or '?')}")
    if s.global_caveats:
        lines.append("\nGLOBAL CAVEATS:")
        for c in s.global_caveats:
            lines.append(f"  ! {c}")
    lines.append(f"\n{len(s.fights)} combats:")
    for f in s.fights:
        deck_n = len(f.deck_entering)
        lines.append(
            f"  [{f.node_index:2d}] {f.node_type:7s} {f.encounter_id:40s} "
            f"enter {f.hp_entering:3d}/{f.max_hp_entering} hp, {deck_n:2d} cards, "
            f"{len(f.relics_entering)} relics, pots={f.potions_entering or '[]'} "
            f"-> took {f.damage_taken:2d} dmg in {f.turns_taken} turns"
            + (f"  (used {f.potions_used})" if f.potions_used else ""))
        for c in f.caveats:
            lines.append(f"        ! {c}")
    return "\n".join(lines)


if __name__ == "__main__":
    path = sys.argv[1]
    s = parse_run(path)
    if "--json" in sys.argv:
        print(json.dumps(asdict(s), indent=1))
    else:
        print(report(s))
