"""Consume schema-v2 mod provenance without weakening solver exactness.

The upload bundle deliberately has no fight indices.  A replay record earns
an association only when its combat-start run snapshot identifies one parsed
fight by BOTH global history depth and encounter identity.  Save-derived
state earns a resolution only when one non-legacy candidate's complete
snake_case counter map equals the replay's complete PascalCase map.
"""

from __future__ import annotations

import base64
import hashlib
import json
import pathlib
import tempfile
from collections import Counter, defaultdict
from dataclasses import dataclass
from typing import Any, Callable

from mcr_parser import McrDecoder
from relay_parser import FightState, RunSummary, parse_run


PROVENANCE_TIERS = frozenset((
    "run_only", "provenance_resolved", "provenance_unresolved",
    "provenance_legacy",
))

_COUNTER_OVERRIDES = {
    "Shuffle": "shuffle",
    "CombatCardSelection": "combat_card_selection",
    "CombatTargets": "combat_targets",
    "CombatEnergyCosts": "combat_energy_costs",
    "CombatCardGeneration": "combat_card_generation",
    "CombatPotionGeneration": "combat_potion_generation",
    "CombatOrbs": "combat_orbs",
}


@dataclass(frozen=True)
class ReplayCapture:
    id: str
    capture_index: int | None
    replay: dict[str, Any]


@dataclass(frozen=True)
class FightProvenance:
    tier: str
    counter_overrides: tuple[tuple[str, int], ...] = ()
    replay: dict[str, Any] | None = None
    entry: dict[str, Any] | None = None
    capture_index: int | None = None
    capture_count: int = 0
    captures: tuple[ReplayCapture, ...] = ()


@dataclass(frozen=True)
class ProvenanceIssue:
    reason: str
    capture_index: int | None = None
    fight_index: int | None = None


@dataclass(frozen=True)
class RunProvenance:
    fights: tuple[FightProvenance, ...]
    resolved_fights: tuple[int, ...]
    issues: tuple[ProvenanceIssue, ...]


@dataclass(frozen=True)
class CandidateResolution:
    tier: str
    counters: dict[str, int] | None
    entry: dict[str, Any] | None = None


def snake_to_pascal(name: str) -> str:
    """Map save stream names to the replay enum spelling (#1063)."""

    if not isinstance(name, str) or not name or any(
            not part for part in name.split("_")):
        raise ValueError("invalid snake_case RNG stream name")
    return "".join(part.capitalize() for part in name.split("_"))


def candidate_counter_map(candidate: Any) -> dict[str, int] | None:
    """Return one candidate's complete counter map in replay spelling."""

    if not isinstance(candidate, dict):
        return None
    entry = candidate.get("entry")
    run_rng = entry.get("run_rng") if isinstance(entry, dict) else None
    if not isinstance(run_rng, dict) or not run_rng:
        return None
    counters: dict[str, int] = {}
    try:
        for snake_name, state in run_rng.items():
            if not isinstance(state, dict):
                return None
            counter = state.get("counter")
            if type(counter) is not int or counter < 0:
                return None
            name = snake_to_pascal(snake_name)
            if name in counters:
                return None
            counters[name] = counter
    except ValueError:
        return None
    return counters


def resolve_entry_candidate(
        candidates: Any, replay_counters: dict[str, int]
        ) -> CandidateResolution:
    """Apply the binding full-map resolution contract.

    A legacy marker makes save-derived state unusable.  Otherwise exactly one
    complete map must equal the replay map; zero or multiple matches remain
    unresolved.  Partial/intersection-only agreement is never sufficient.
    """

    if not isinstance(candidates, list):
        return CandidateResolution("provenance_unresolved", None, None)
    if any(isinstance(item, dict) and
           item.get("observation") == "legacy_unverified"
           for item in candidates):
        return CandidateResolution("provenance_legacy", None, None)
    matches = []
    for candidate in candidates:
        counters = candidate_counter_map(candidate)
        if counters is not None and counters == replay_counters:
            matches.append((counters, candidate["entry"]))
    if len(matches) == 1:
        counters, entry = matches[0]
        return CandidateResolution("provenance_resolved", counters, entry)
    return CandidateResolution("provenance_unresolved", None, None)


def parse_run_data(run_data: dict[str, Any]) -> RunSummary:
    """Use relay_parser without introducing a second .run interpretation."""

    with tempfile.TemporaryDirectory(prefix="sts-review-provenance-") as temp:
        path = pathlib.Path(temp) / "input.run"
        path.write_text(json.dumps(run_data))
        return parse_run(str(path))


def analyze_bundle(
        run_data: dict[str, Any], bundle: Any, *,
        decoder: Callable[[bytes], dict[str, Any]] | None = None,
        parsed_run: RunSummary | None = None,
        fallback_fight_count: int = 0,
        lineage: Any = ()) -> RunProvenance:
    """Associate and resolve one bundle, returning no guessed state.

    ``lineage`` is a branch run's ancestry (#3313, #3373): its
    ``run_branches`` links as ``(source_start_time, source_floor)`` pairs,
    NEAREST FIRST. A replay-from-rewards-screen branch is allocated a fresh
    ``start_time`` by the server, but every floor up to and including the
    branch point was played under the SOURCE run, so those floors' captures
    carry the source's ``start_time``. See ``_accepted_start_times`` for the
    one extra identity this admits; the seed and every association and
    resolution check below apply to it unchanged.
    """

    issues: list[ProvenanceIssue] = []
    try:
        run = parsed_run or parse_run_data(run_data)
    except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError,
            NotImplementedError):
        issues.append(ProvenanceIssue("run_parse_failed"))
        return _unresolved(fallback_fight_count, issues)

    count = len(run.fights)
    if not isinstance(bundle, dict) or bundle.get("schema_version") != 2 or \
            not isinstance(bundle.get("fights"), list):
        issues.append(ProvenanceIssue("bundle_malformed"))
        return _unresolved(count, issues)

    accepted = _accepted_start_times(run_data.get("start_time"), lineage)
    decode = decoder or _decode_mcr
    associated: dict[
        int, list[tuple[dict[str, Any], dict[str, int], int | None,
                        dict[str, Any]]]
    ] = defaultdict(list)
    for ordinal, record in enumerate(bundle["fights"]):
        capture_index = _capture_index(record, ordinal)
        try:
            replay = decode(_mcr_bytes(record))
            replay_run = replay["run"]
            if replay_run.get("rng", {}).get("seed") != run.seed:
                raise _AssociationError("run_identity_mismatch")
            depth_bound = _start_time_depth_bound(
                replay_run.get("start_time"), run_data.get("start_time"),
                accepted)
            history_depth = _history_depth(replay_run)
            if depth_bound is not None and history_depth >= depth_bound:
                raise _AssociationError("run_identity_mismatch")
            candidates = [
                index for index, fight in enumerate(run.fights)
                if fight.node_index == history_depth
                and _encounter_matches(replay, fight)
            ]
            if len(candidates) != 1:
                reason = "association_ambiguous" if candidates else \
                    "association_unmatched"
                raise _AssociationError(reason)
            counters = _replay_counter_map(replay_run)
        except _AssociationError as exc:
            issues.append(ProvenanceIssue(exc.reason, capture_index))
            continue
        except Exception:  # noqa: BLE001 — one bad optional record is local
            issues.append(ProvenanceIssue("replay_decode_failed", capture_index))
            continue
        associated[candidates[0]].append(
            (record, counters, capture_index, replay))

    fights = [FightProvenance("provenance_unresolved") for _ in range(count)]
    resolved = []
    for fight_index, records in associated.items():
        # The uploader sorts timestamp-prefixed archive filenames, then assigns
        # capture_index in that chronological order (ReviewProvenanceCapture).
        # Save/quit retries are legitimate: use the latest captured attempt.
        # Require explicit unique indices when choosing between captures; an
        # array's incidental order or a tied index does not establish recency.
        if len(records) > 1:
            indices = [item[0].get("capture_index") for item in records]
            if any(type(i) is not int or i < 0 for i in indices) or len(set(indices)) != len(indices):
                for _record, _counters, capture_index, _replay in records:
                    issues.append(ProvenanceIssue(
                        "duplicate_fight_record", capture_index, fight_index))
                continue
            records.sort(key=lambda item: item[0]["capture_index"])
            for _record, _counters, capture_index, _replay in records[:-1]:
                issues.append(ProvenanceIssue(
                    "superseded_fight_capture", capture_index, fight_index))
        record, replay_counters, capture_index, replay = records[-1]
        resolution = resolve_entry_candidate(
            record.get("entry_candidates"), replay_counters)
        source_counters = resolution.counters or replay_counters
        overrides = tuple(sorted(
            (_COUNTER_OVERRIDES[name], source_counters[name])
            for name in _COUNTER_OVERRIDES if name in source_counters))
        # Keep each byte-distinct attempt for display. The newest capture still
        # owns solver entry resolution; older retries never supply its state.
        captures = {}
        for captured_record, _, captured_index, captured_replay in records:
            digest = hashlib.sha256(_mcr_bytes(captured_record)).hexdigest()
            captures[digest] = ReplayCapture(digest, captured_index, captured_replay)
        fights[fight_index] = FightProvenance(
            resolution.tier, overrides, replay, resolution.entry, capture_index,
            len(records), tuple(captures.values()))
        if resolution.tier == "provenance_resolved":
            resolved.append(fight_index)
        else:
            issues.append(ProvenanceIssue(
                "candidate_legacy" if resolution.tier ==
                "provenance_legacy" else "candidate_unresolved",
                capture_index, fight_index))
    return RunProvenance(
        tuple(fights), tuple(sorted(resolved)), tuple(issues))


def _accepted_start_times(
        own_start_time: Any, lineage: Any) -> dict[int, int]:
    """Map each ancestor ``start_time`` to the history depth it is bounded by.

    A link ``(source_start_time, source_floor)`` says the branch left its
    source on the rewards screen after floor ``source_floor``. History depth
    is the 0-based node index (floor ``depth + 1``), so the source's captures
    are genuine exactly for depths ``< source_floor`` — which includes the
    branch point's own fight, played before the branch existed.

    Branch-of-branch: walking nearest first, each further ancestor is bounded
    by the MINIMUM ``source_floor`` seen so far. The run C branched from B at
    floor f1 inherits only B's floors ``< f1``; within those, B inherited only
    A's floors ``< f0``; so an A capture is genuine for C only below
    ``min(f0, f1)``. The nearest link bounds first, and a further link can
    only narrow the bound, never widen it. (Nothing here also imposes a LOWER
    bound on an intermediate ancestor's captures, exactly as the run's own
    ``start_time`` is not lower-bounded: the association step still requires
    one parsed fight at that depth with that encounter.)

    The run's own ``start_time`` is never re-bounded by a link that names it,
    and a ``start_time`` repeated further up the chain keeps its first
    (nearest, tightest-so-far) bound. Malformed lineage raises ``ValueError``:
    the caller builds it, and an untrusted shape must not widen acceptance.
    """

    if isinstance(lineage, (str, bytes, dict)):
        raise ValueError("lineage must be a sequence of links")
    accepted: dict[int, int] = {}
    bound: int | None = None
    for link in lineage:
        if not isinstance(link, (tuple, list)) or len(link) != 2:
            raise ValueError("lineage link must be (source_start_time, source_floor)")
        source_start_time, source_floor = link
        if type(source_start_time) is not int or \
                type(source_floor) is not int or source_floor < 1:
            raise ValueError("lineage link must hold integers, floor >= 1")
        bound = source_floor if bound is None else min(bound, source_floor)
        if source_start_time == own_start_time and \
                type(own_start_time) is int:
            continue
        accepted.setdefault(source_start_time, bound)
    return accepted


def _start_time_depth_bound(
        capture_start_time: Any, own_start_time: Any,
        accepted: dict[int, int]) -> int | None:
    """None for the run's own identity, an exclusive depth bound for an
    ancestor's, and ``run_identity_mismatch`` for anything else."""

    if capture_start_time == own_start_time:
        return None
    if type(capture_start_time) is int and capture_start_time in accepted:
        return accepted[capture_start_time]
    raise _AssociationError("run_identity_mismatch")


def capture_identity_admitted(
        capture_start_time: Any, own_start_time: Any, history_depth: int,
        lineage: Any = ()) -> bool:
    """The association's run-identity rule, for a consumer re-checking it.

    ``rust_review.replay_capture_run`` re-verifies that the replay the worker
    associated belongs to the run; this is the same rule (own ``start_time``,
    or an ancestor's below its depth bound) so the two cannot drift. The seed
    is checked by the caller, as it is here.
    """

    try:
        bound = _start_time_depth_bound(
            capture_start_time, own_start_time,
            _accepted_start_times(own_start_time, lineage))
    except _AssociationError:
        return False
    return bound is None or history_depth < bound


def parse_lineage(value: Any) -> tuple[tuple[int, int], ...]:
    """A lineage from JSON (``[[source_start_time, source_floor], ...]``),
    validated by the same rule ``analyze_bundle`` applies; raises
    ``ValueError`` on any other shape."""

    if not isinstance(value, list):
        raise ValueError("lineage must be a JSON array of links")
    _accepted_start_times(None, value)
    return tuple((link[0], link[1]) for link in value)


class _AssociationError(ValueError):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


def _unresolved(count: int, issues: list[ProvenanceIssue]) -> RunProvenance:
    return RunProvenance(
        tuple(FightProvenance("provenance_unresolved")
              for _ in range(count)), (), tuple(issues))


def _capture_index(record: Any, ordinal: int) -> int | None:
    if isinstance(record, dict) and type(record.get("capture_index")) is int:
        return record["capture_index"]
    return ordinal


def _mcr_bytes(record: Any) -> bytes:
    mcr = record.get("mcr") if isinstance(record, dict) else None
    if not isinstance(mcr, dict) or mcr.get("encoding") != "base64" or \
            not isinstance(mcr.get("data"), str):
        raise _AssociationError("replay_missing")
    return base64.b64decode(mcr["data"], validate=True)


def _decode_mcr(data: bytes) -> dict[str, Any]:
    return McrDecoder(data).combat_replay()


def _history_depth(replay_run: dict[str, Any]) -> int:
    history = replay_run.get("map_point_history")
    if not isinstance(history, list) or any(
            not isinstance(act, list) for act in history):
        raise _AssociationError("history_depth_invalid")
    depth = sum(len(act) for act in history)
    act_index = replay_run.get("current_act_index") or 0
    visited = replay_run.get("visited_map_coords")
    if type(act_index) is not int or act_index < 0 or \
            not isinstance(visited, list) or not visited:
        raise _AssociationError("history_depth_invalid")
    prior = sum(len(act) for act in history[:act_index]) if act_index else 0
    if depth != prior + len(visited) - 1:
        raise _AssociationError("history_depth_inconsistent")
    return depth


def _replay_counter_map(replay_run: dict[str, Any]) -> dict[str, int]:
    counters = replay_run.get("rng", {}).get("counters")
    if not isinstance(counters, dict) or not counters or any(
            not isinstance(name, str) or type(value) is not int or value < 0
            for name, value in counters.items()):
        raise _AssociationError("replay_counters_invalid")
    return dict(counters)


def _encounter_matches(replay: dict[str, Any], fight: FightState) -> bool:
    selected = _selected_encounter_id(replay["run"], fight.node_type)
    if selected is not None:
        return selected == fight.encounter_id

    # Event combats are not selected from the ordinary encounter pools.  The
    # first combat checksum is the replay's only current-encounter identity.
    # Some encounter history lists include setup entities absent by the first
    # checksum (the validated Phrog/Wriggler case), so require the replay's
    # non-empty monster multiset to be a subset, never merely an overlap.
    replay_monsters = _replay_monsters(replay)
    expected = Counter(
        item.removeprefix("MONSTER.") for item in fight.monster_ids)
    return bool(replay_monsters) and not (replay_monsters - expected)


def _selected_encounter_id(
        replay_run: dict[str, Any], node_type: str) -> str | None:
    try:
        act = replay_run["acts"][replay_run["current_act_index"]]
        rooms = act["rooms"]
        if node_type == "monster":
            value = rooms["normal_encounter_ids"][
                rooms["normal_encounters_visited"]]
        elif node_type == "elite":
            value = rooms["elite_encounter_ids"][rooms["elites_visited"]]
        elif node_type == "boss":
            visited = rooms["bosses_visited"]
            value = rooms["boss_id"] if visited == 0 else \
                rooms["second_boss_id"] if visited == 1 else None
        else:
            return None
    except (KeyError, IndexError, TypeError):
        return None
    return f"ENCOUNTER.{value}" if isinstance(value, str) and value else None


def _replay_monsters(replay: dict[str, Any]) -> Counter[str]:
    try:
        creatures = replay["checksums"][0]["full_state"]["creatures"]
    except (KeyError, IndexError, TypeError):
        return Counter()
    return Counter(
        item["monster_id"].removeprefix("MONSTER.")
        for item in creatures
        if isinstance(item, dict) and isinstance(item.get("monster_id"), str))
