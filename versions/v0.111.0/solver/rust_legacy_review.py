"""The legacy (``.run``-only) review on the Rust root, search and replay.

#2827 item C2. A fight with neither floor saves nor a resolved save projection
used to be reviewed by the frozen Python simulator: ``replay_fight.predict``
and the ``solve_fight.*_counter_entering`` accounting predicted the entering
RNG counters, ``combat_sim.start_combat`` built the root, Rust searched it, and
``rust_exact_solve.replay_python_actions`` replayed the line back through
Python. This module replaces every one of those steps:

1. ``sts-sim run-counters`` predicts the counters (#2827 C1,
   ``rust_run_counters``);
2. this module writes the fight's ``.run`` facts plus those counters as an
   ``sts-sim-entry-v1`` document and ``sts-sim entry --facts --opening``
   builds the root (the Coach's input, #2995);
3. ``rust_review.search_outcomes`` runs the same bounded Rust search, and
   replays each witness line in Rust, that the floor-snapshot and
   replay-capture paths use (#2973, #2988).

Nothing here imports ``combat_sim``, ``solve_fight`` or ``mcr_replay``, directly
or through the modules it uses (#2827 item F1 deleted ``infoset_sample`` and
``rust_exact_solve.replay_python_actions`` with the Python producer).

# Counters the history cannot know

``run-counters`` labels each stream ``exact``, ``baseline`` (a value the
caveats say may be short), ``unknown``, ``assumed`` (MonsterAi) or
``not_predicted`` (CombatPotionGeneration). The legacy path used the value of
the first two and disclosed the caveats; that contract is kept, and the caveat
list travels in ``metadata.entry_caveats`` exactly as before.

The last three are never fed to the opening as facts. The Python path passed
``ai_counter=0`` into ``start_combat`` for every fight, and C1 measured that
assumption against 540 captured start saves: it held on 161. Each such stream
instead enters at a **placeholder** (counter 0), and every result built on the
root is required to leave it there. Every ``Rng`` draw increments ``_counter``
(``Rng::NextInt`` RVA ``0x5eb26`` IL_0001-IL_0016, the same fact
``entry/facts.rs`` relies on), so a stream whose counter never moved was never
drawn and its true state cannot have changed anything:

* the opening must not move it (else ``<stream>_counter_unknown``);
* a witness line whose Rust replay moves it is not published; if no witness
  survives, the review refuses with that stream's name.

A counter the worker resolved from a capture (``counter_overrides``) or that
a capture's embedded run records is an observation, not a prediction, and is
used as exact.
"""

from __future__ import annotations

import copy
import json
import pathlib
import subprocess
import tempfile
import time
from dataclasses import asdict
from typing import Any

import relay_parser
import rust_run_counters
import sts2_rng

ENTRY_FACTS_SCHEMA = "sts-sim-entry-v1"
BUILD = "v0.111.0"

# The nine combat streams: `run-counters` / save name, then the
# `sts-sim-canonical-v2` `rng` key it projects to (`entry/counters.rs`
# `COMBAT_STREAMS`).
COMBAT_STREAMS = (
    ("shuffle", "rng"),
    ("niche", "niche"),
    ("monster_ai", "ai"),
    ("combat_card_selection", "sel"),
    ("combat_card_generation", "generation"),
    ("combat_targets", "targets"),
    ("combat_energy_costs", "energy_costs"),
    ("combat_orbs", "combat_orbs"),
    ("combat_potion_generation", "potion_generation"),
)
_CANONICAL_KEY = dict(COMBAT_STREAMS)
# The `run-counters` statuses whose value the legacy path used as the entering
# counter. `baseline` carries caveats, which are disclosed.
USED_STATUSES = frozenset({"exact", "baseline"})
PLACEHOLDER_COUNTER = 0
# `live_coach.build_entry` keeps saved per-instance `props` on these three
# cards only (`entry/deck.rs` PER_INSTANCE_PROP_CARDS).
PER_INSTANCE_PROP_CARDS = frozenset({
    "CARD.THE_SCYTHE", "CARD.GENETIC_ALGORITHM", "CARD.MAD_SCIENCE"})


class LegacyRootRefusal(ValueError):
    """A named reason the legacy root cannot be built exactly."""

    def __init__(self, reason: str, message: str, **details: Any):
        super().__init__(message)
        self.reason = reason
        self.message = message
        self.details = details


class UnknownStreamConsumed(ValueError):
    """A result drew a stream whose entering counter is not known."""

    def __init__(self, stream: str):
        super().__init__(
            f"{stream} counter unknown: the .run history does not determine "
            "it, and this result draws from it (SOLVER_INVARIANTS.md I5 — "
            "refusing to guess)")
        self.stream = stream


def _snake_to_pascal(name: str) -> str:
    return "".join(part.capitalize() for part in name.split("_"))


def capture_counters(replay: dict | None) -> dict[str, int]:
    """Every combat-stream counter a capture's embedded run records.

    The worker has already associated the capture with this fight (seed,
    start time, node and encounter; `review_provenance.analyze_bundle`), and
    the embedded run is the combat-entry state, so these are observations.
    Only well-formed non-negative integers are taken; anything else is left
    to the prediction.
    """
    if not isinstance(replay, dict):
        return {}
    run = replay.get("run")
    counters = (run.get("rng") or {}).get("counters") if isinstance(run, dict) else None
    if not isinstance(counters, dict):
        return {}
    observed = {}
    for name, _key in COMBAT_STREAMS:
        # The capture spells the orb stream `CombatOrbs` (rust_review).
        value = counters.get(_snake_to_pascal(name))
        if type(value) is int and value >= 0:
            observed[name] = value
    return observed


def resolve_counters(prediction: dict, overrides: dict[str, int],
                     observed: dict[str, int]) -> dict[str, dict]:
    """Each combat stream's entering counter and where it came from.

    Returns ``{stream: {"value": int | None, "source": str, "status": str}}``;
    ``value`` is None exactly when the stream enters at a placeholder.
    """
    resolved = {}
    for name, _key in COMBAT_STREAMS:
        row = prediction["counters"].get(name)
        if row is None:
            raise LegacyRootRefusal(
                "run_counters_incomplete",
                f"sts-sim run-counters answered no {name} row")
        if name in overrides:
            resolved[name] = {"value": overrides[name], "source": "override",
                              "status": "exact"}
        elif name in observed:
            resolved[name] = {"value": observed[name], "source": "capture",
                              "status": "exact"}
        elif row["status"] in USED_STATUSES and type(row["value"]) is int:
            resolved[name] = {"value": row["value"], "source": "prediction",
                              "status": row["status"]}
        else:
            resolved[name] = {"value": None, "source": "placeholder",
                              "status": row["status"]}
    return resolved


def _streams(seed: str, counters: dict[str, int]) -> dict[str, dict]:
    """Each stream's state at its counter, as a save's `rng.rngs` spells it.

    `sts2_rng.RunRngSet` is the oracle `live_coach.verify_stream_seeding`
    checks saves with; Rust re-derives every state from the seed and refuses a
    document whose words disagree (`entry/facts.rs`).
    """
    run_set = sts2_rng.RunRngSet(
        seed, {_snake_to_pascal(name): value for name, value in counters.items()},
        build=BUILD)
    streams = {}
    for name, value in counters.items():
        rng = run_set.rngs[_snake_to_pascal(name)]
        if rng.counter != value:
            raise LegacyRootRefusal(
                "stream_state_unavailable",
                f"{name} could not be advanced to counter {value}")
        words = rng._random
        streams[name] = {"counter": value, "s0": words.s0, "s1": words.s1,
                         "s2": words.s2, "s3": words.s3}
    return streams


def _deck_row(row: dict) -> dict:
    """One `.run` entry deck row in `build_entry`'s vocabulary."""
    if row.get("props_ambiguous"):
        raise LegacyRootRefusal(
            "entry_props_ambiguous",
            f"{row['id']} mutable props entering this fight are ambiguous: "
            "the historical .run endpoint cannot date prior-fight growth "
            "(SOLVER_INVARIANTS.md I5/I8 — refusing to guess)")
    projected = {"id": row["id"], "upgrade_level": row["upgrade_level"]}
    if "props" in row and row["id"] in PER_INSTANCE_PROP_CARDS:
        projected["props"] = copy.deepcopy(row["props"])
    if row.get("enchantment"):
        projected["enchantment"] = row["enchantment"]
        # `start_combat` read `c.get("enchant_amount", 1)`.
        projected["enchant_amount"] = row.get("enchant_amount", 1)
    return projected


def entry_preconditions(entry: dict) -> None:
    """The `.run` entry refusals ``start_combat`` raised before the deck.

    Checked first, in ``start_combat``'s order, so a fight that is both an
    event-node entry and a copy-ambiguous one keeps the old refusal.
    """
    if entry.get("entry_ambiguous"):
        raise LegacyRootRefusal(
            "entry_ambiguous",
            "event-node fight: entry state is the node-entry snapshot, "
            "pre-combat event effects unordered "
            "(SOLVER_INVARIANTS.md I5 — refusing to guess)")
    if entry.get("potion_entry_ambiguous"):
        raise LegacyRootRefusal(
            "potion_entry_ambiguous",
            "potion inventory entering this fight is ambiguous because a "
            "full-belt replacement slot or potion source is unrecorded "
            "(SOLVER_INVARIANTS.md I5 — refusing to guess)")


def entry_facts(run: relay_parser.RunSummary, entry: dict,
                counters: dict[str, dict], *,
                fully_unlocked_card_pool: bool,
                unlocked_card_pool_epochs: list[str] | None = None) -> dict:
    """The `sts-sim-entry-v1` facts of one `.run` fight (pure data).

    ``entry`` is ``asdict(FightState)``, after copy-only canonicalization.
    Every field is the one ``start_combat`` read from the same dict; the
    `.run` records no potion slot or belt capacity, so only an empty belt is
    written (a fight entering with potions refuses by name).
    """
    entry_preconditions(entry)
    for row in entry["deck_entering"]:
        if row.get("upgrade_ambiguous") or row.get("enchant_ambiguous"):
            raise LegacyRootRefusal(
                "copy_assignment_ambiguous",
                f"{row['id']} copy assignment is ambiguous at entry")
    relics = list(entry["relics_entering"])
    if "RELIC.FUR_COAT" in relics:
        # `.run` dates Fur Coat membership exactly, but the entry-facts
        # vocabulary is the save's, which never proves it (#2526).
        raise LegacyRootRefusal(
            "fur_coat_membership_unrepresentable",
            "Fur Coat membership is a .run fact the Rust entry facts do not "
            "carry (entry/facts.rs, #2526)")
    potions = list(entry["potions_entering"])
    if potions:
        # The parser tracks the belt as a multiset (`relay_parser`, the
        # full-belt replacement enumeration), so neither a slot index nor the
        # capacity at this fight is recorded; the Rust belt is the sparse
        # slot list and the boundary refuses a dense list without it
        # (`boundary.rs` `hydrate_potion_belt`). The Python root carried the
        # dense list alone, which that same boundary refused.
        raise LegacyRootRefusal(
            "potion_belt_slots_unrecorded",
            f"{len(potions)} potion(s) enter this fight and the .run records "
            "no belt slot or capacity for them "
            "(SOLVER_INVARIANTS.md I5 — refusing to guess)",
            potions=potions)
    placeholders = {name for name, row in counters.items() if row["value"] is None}
    values = {name: (PLACEHOLDER_COUNTER if name in placeholders else row["value"])
              for name, row in counters.items()}
    streams = _streams(run.seed, values)
    return {
        "schema": ENTRY_FACTS_SCHEMA, "game_build": BUILD,
        "save_schema_version": 20, "seed": run.seed,
        "character": run.character,
        "entry": {
            "encounter_id": entry["encounter_id"],
            "node_index": entry["node_index"],
            "node_type": entry["node_type"],
            "deck_entering": [_deck_row(row) for row in entry["deck_entering"]],
            "relics_entering": relics,
            "potions_entering": potions,
            "max_potion_slot_count": None,
            "potion_slots_entering": [],
            "hp_entering": entry["hp_entering"],
            "max_hp_entering": entry["max_hp_entering"],
            "gold_entering": entry["gold_entering"],
            "ascension": run.ascension,
            "relic_counters": dict(entry["relic_counters"]) or None,
            "tea_set_charged": entry["tea_set_charged"],
            "fake_tea_set_charged": entry["fake_tea_set_charged"],
            "fur_coat_active": None,
            # `.run` reconstruction retains no relic order.
            "relics_entering_dispatch_ordered": False,
            "entry_ambiguous": False,
        },
        "counters": {name: state["counter"] for name, state in streams.items()},
        "streams": streams,
        "unlocks": {
            "unlocked_card_pool_epochs": unlocked_card_pool_epochs,
            "fully_unlocked_card_pool": fully_unlocked_card_pool,
            # `load_fight_context` passed only the card-pool answer;
            # `start_combat`'s potion pool stayed unknown.
            "fully_unlocked_potion_pool": None,
        },
    }


def rust_opening(binary: pathlib.Path, facts: dict) -> dict:
    """`sts-sim entry --facts --opening`; a refusal raises by name."""
    with tempfile.TemporaryDirectory(prefix="sts-legacy-facts-") as directory:
        path = pathlib.Path(directory) / "facts.json"
        path.write_text(json.dumps(facts))
        completed = subprocess.run(
            [str(binary), "entry", "--build", BUILD, "--facts", str(path),
             "--opening"],
            text=True, capture_output=True, timeout=60, check=False)
    if completed.returncode:
        raise LegacyRootRefusal(
            "rust_opening_cli_failed",
            "Rust opening CLI failed: " + completed.stderr[-500:])
    document = json.loads(completed.stdout)
    if document.get("schema") != "sts-sim-canonical-v2":
        opening = document.get("opening") or {}
        refusal = document.get("refusal") or {}
        kind = (opening.get("refusal_class") or document.get("refusal_class")
                or "entry_refused")
        detail = opening.get("detail") or refusal.get("detail") or ""
        raise LegacyRootRefusal(
            "rust_opening_refused",
            f"Rust opening refused: {kind}: {detail[:300]}",
            refusal_class=kind)
    return document


def placeholder_check(placeholders: dict[str, int]):
    """A check that every placeholder stream is still at its counter.

    The check is handed to ``rust_review.search_outcomes`` as its
    ``final_check``; ``check.withholds`` names the exception that withholds
    a line rather than failing the review.
    """
    keys = {stream: _CANONICAL_KEY[stream] for stream in placeholders}

    def check(state: dict) -> None:
        rng = state.get("rng") or {}
        for stream, counter in sorted(placeholders.items()):
            live = rng.get(keys[stream])
            # The canonical document elides a stream only when it is the
            # all-zero default, which no seeded placeholder is; an absent
            # stream therefore fails the check rather than passing it.
            if not isinstance(live, dict) or live.get("counter") != counter:
                raise UnknownStreamConsumed(stream)
    check.withholds = UnknownStreamConsumed
    return check


def build_root(binary: pathlib.Path, run: relay_parser.RunSummary,
               entry: dict, prediction: dict, *,
               overrides: dict[str, int], replay: dict | None,
               fully_unlocked_card_pool: bool,
               unlocked_card_pool_epochs: list[str] | None = None
               ) -> tuple[dict, dict[str, dict], dict[str, int]]:
    """The Rust opening for one `.run` fight, and its unknown streams.

    Returns ``(root, counters, placeholders)``. ``root`` carries the entry's
    empty AfterEnergyReset order where the boundary accepts it
    (``rust_review._with_entry_reset_witness``, the floor-snapshot rule).
    """
    import rust_review
    counters = resolve_counters(prediction, overrides, capture_counters(replay))
    facts = entry_facts(
        run, entry, counters,
        fully_unlocked_card_pool=fully_unlocked_card_pool,
        unlocked_card_pool_epochs=unlocked_card_pool_epochs)
    placeholders = {name: PLACEHOLDER_COUNTER
                    for name, row in counters.items() if row["value"] is None}
    root = rust_opening(binary, facts)
    try:
        placeholder_check(placeholders)(root)
    except UnknownStreamConsumed as exc:
        raise LegacyRootRefusal(
            f"{exc.stream}_counter_unknown",
            f"the combat opening draws {exc.stream}, whose entering counter "
            "the .run history does not determine "
            "(SOLVER_INVARIANTS.md I5 — refusing to guess)",
            stream=exc.stream, status=counters[exc.stream]["status"]) from exc
    return rust_review._with_entry_reset_witness(binary, root), counters, placeholders


def fight_entry(run_path: pathlib.Path, run: relay_parser.RunSummary,
                fight_index: int) -> tuple[dict, list[str]]:
    """``asdict(FightState)`` after the copy-only canonicalization.

    ``review_summary_v2.canonicalize_entry_multiset`` is pure data: it
    resolves duplicate-copy flags to one canonical multiset or refuses. A
    resolution still leaves which *physical* copy carries the upgrade or
    enchantment unknown, which the draw order depends on, so the old path
    only benchmarked those fights (``degraded_reasons``).
    """
    fight = run.fights[fight_index]
    if not any(row.get("upgrade_ambiguous") or row.get("enchant_ambiguous")
               for row in fight.deck_entering):
        return asdict(fight), []
    import review_summary_v2
    raw = json.loads(pathlib.Path(run_path).read_text())
    return review_summary_v2.canonicalize_entry_multiset(fight, raw)


def history_prediction(binary: pathlib.Path, run_path: pathlib.Path,
                       fight_index: int, build: str) -> dict:
    """`sts-sim run-counters` for one fight; a refusal raises by name."""
    try:
        history = rust_run_counters.load_history(run_path)
        return rust_run_counters.predict(binary, history, fight_index, build)
    except rust_run_counters.RunCountersRefusal as exc:
        raise LegacyRootRefusal(
            f"run_counters_{exc.code}", exc.detail) from exc


def counters_metadata(counters: dict[str, dict]) -> dict[str, dict]:
    return {name: {"value": row["value"], "source": row["source"],
                   "status": row["status"]}
            for name, row in sorted(counters.items())}


# The old path's degraded-reason names for an unknown counter the opening
# consumes (the retired Python `review_summary_v2._COUNTER_FAILURES`).
_UNKNOWN_COUNTER_REASONS = {
    "combat_potion_generation": "combat_potion_generation_counter_unknown",
    "combat_card_generation": "combat_card_generation_counter_unknown",
    "combat_orbs": "combat_orb_generation_counter_unknown",
    "combat_card_selection": "combat_card_selection_counter_unknown",
    "combat_targets": "combat_targets_counter_unknown",
    "combat_energy_costs": "combat_energy_costs_counter_unknown",
    "monster_ai": "monster_ai_counter_unknown",
}
ENTRY_STATE_SOURCE = "run_history_prediction"
OPENING_ADAPTER = "rust_entry_facts_opening"


def _refusal(cause: str, message: str, **details: Any):
    """A ``simulation_refusal`` naming its exact cause.

    The top-level reason stays the vocabulary the site and the worker already
    know (`src/fightreviews.mjs` REFUSAL_REASONS); ``details.cause`` is the
    machine-readable name of what could not be built exactly.
    """
    import review_summary as phase1
    return phase1.ReviewRefusal("simulation_refusal", message, cause=cause,
                                **details)


def _stream_refusal(stream: str, where: str, status: str | None,
                    **details: Any):
    return _refusal(
        _UNKNOWN_COUNTER_REASONS.get(stream, f"{stream}_counter_unknown"),
        f"{stream} counter unknown: the .run history does not determine it "
        f"and {where} draws from it (SOLVER_INVARIANTS.md I5 — refusing to "
        "guess)",
        stream=stream, prediction_status=status, **details)


def _write_line_status(path: pathlib.Path | None, check: str) -> None:
    """The worker-only line-replay diagnostic side channel."""
    if path is None:
        return
    try:
        path.write_text(json.dumps({"turn": None, "check": check}))
    except OSError:
        pass


def generate(run_path: str | pathlib.Path, fight_index: int, config: Any, *,
             assumed_fully_unlocked: bool | None = None) -> dict:
    """One review document for a ``.run``-only fight, built entirely in Rust.

    ``config`` is a ``review_summary.ReviewConfig``. A card is the Rust
    review shape (``rust_review.generate``: ``best_actual_seed``,
    ``alternative_actual_seeds``, achieved-lower-bound claims) plus the
    legacy path's disclosure fields (``entry_caveats``,
    ``assumed_fully_unlocked``, ``counter_overrides``, ``degraded_reasons``).
    A refusal or deferral is the legacy refusal document, unchanged in shape.
    """
    import review_summary as phase1
    import review_summary_v2 as v2
    import rust_review
    from rust_exact_solve import canonical_document, default_binary

    config.validate()
    started = time.monotonic()
    path = pathlib.Path(run_path)
    identity: dict[str, Any] = {"run_file": path.name, "fight_index": fight_index}
    assumed = False

    def refused(exc) -> dict:
        document = phase1._refusal_document(identity, config, exc, started)
        document["metadata"]["entry_state_source"] = ENTRY_STATE_SOURCE
        return v2._upgrade_document(document, assumed_fully_unlocked=assumed)

    try:
        try:
            run = relay_parser.parse_run(str(path))
        except NotImplementedError as exc:
            # A parse-level I5 refusal (a multiplayer run).
            raise phase1.ReviewRefusal("simulation_refusal", str(exc)) from exc
        except (OSError, json.JSONDecodeError, KeyError, TypeError,
                ValueError) as exc:
            raise phase1.ReviewRefusal("invalid_run", str(exc)) from exc
        phase1.require_review_admission(run)
        if type(fight_index) is not int or not 0 <= fight_index < len(run.fights):
            raise phase1.ReviewRefusal(
                "fight_index_out_of_range",
                f"fight index {fight_index!r} is outside "
                f"0..{len(run.fights) - 1}",
                fight_count=len(run.fights))
        fight = run.fights[fight_index]
        identity.update({"run_seed": run.seed,
                         "encounter_id": fight.encounter_id,
                         "node_type": fight.node_type,
                         "floor": fight.node_index + 1})
        assumed = (assumed_fully_unlocked if assumed_fully_unlocked is not None
                   else (not config.fully_unlocked_card_pool
                         and type(run.ascension) is int
                         and run.ascension >= v2.ASCENSION_UNLOCK_DEFAULT))
        fully_unlocked = config.fully_unlocked_card_pool or assumed
        v2._validate_recorded_length(fight, config.horizon)
        actual = phase1.actual_outcome(fight)
        binary = config.rust_exact_solver_binary or default_binary()
        if binary is None:
            raise _refusal("rust_binary_unavailable",
                           "Rust solver executable is unavailable")
        try:
            prediction = history_prediction(binary, path, fight_index,
                                            run.build_id)
            entry_preconditions(asdict(fight))
            entry, degraded = fight_entry(path, run, fight_index)
            if degraded:
                raise _refusal(
                    "benchmark_only_unavailable",
                    "the entry is determined only as a card multiset (which "
                    "copy carries the upgrade or enchantment is unrecorded, "
                    "and the draw order depends on it), so only a sampled "
                    "benchmark could be reviewed; that benchmark ran on the "
                    "retired Python simulator and is not ported (#3004) "
                    "(SOLVER_INVARIANTS.md I5)",
                    degraded_reasons=degraded)
            root, counters, placeholders = build_root(
                binary, run, entry, prediction,
                overrides=config.counter_overrides(),
                replay=config.recorded_replay,
                fully_unlocked_card_pool=fully_unlocked)
        except LegacyRootRefusal as exc:
            if "stream" in exc.details:
                raise _stream_refusal(
                    exc.details["stream"], "the combat opening",
                    exc.details.get("status")) from exc
            raise _refusal(exc.reason, exc.message, **exc.details) from exc

        check = placeholder_check(placeholders)
        withheld: list[dict] = []
        outcomes, diagnostics = rust_review.search_outcomes(
            binary, root, config, final_check=check, withheld=withheld)
        if not outcomes:
            if withheld:
                stream = withheld[0]["stream"]
                raise _stream_refusal(
                    stream, "every witness line the search found",
                    counters[stream]["status"], withheld_lines=withheld)
            first = next((d["first_refusal"] for d in diagnostics
                          if d.get("first_refusal")), None)
            if first is not None:
                # Every playout that reached a terminal crossed a refusal.
                raise _refusal("rust_search_refused",
                               "Rust search refused: " + json.dumps(first))
            raise _refusal("search_budget_exhausted",
                           "search budget reached without a terminal witness")

        document = {
            "schema_version": v2.SCHEMA_VERSION, "status": "ok",
            "fight": identity, "actual": actual,
            "best_actual_seed": outcomes[0],
            "alternative_actual_seeds": outcomes[1:],
            "degraded_reasons": [],
            "metadata": {
                "game_build": run.build_id,
                "simulator": phase1.simulator_document(run.build_id, binary),
                "trust_tier": "modeled",
                "entry_state_source": ENTRY_STATE_SOURCE,
                "opening_validation": "not_checked",
                "opening_adapter": OPENING_ADAPTER,
                "search_engine": "rust", "replay_engine": "rust",
                "entry_digest": canonical_document.differential_digest(root),
                "objective": ("win, then combat HP; potion usage is reported "
                              "separately"),
                "horizon": config.horizon,
                "deadlines_seconds": {
                    "best_actual_seed": config.actual_deadline},
                "searches": diagnostics,
                "entry_caveats": list(prediction.get("caveats", [])),
                "entry_counters": counters_metadata(counters),
                "unverified_streams": sorted(placeholders),
                "withheld_lines": withheld,
                "fully_unlocked_card_pool": config.fully_unlocked_card_pool,
                "assumed_fully_unlocked": assumed,
                "counter_overrides": config.counter_overrides(),
                "native_full_checksum": "not_checked",
                "in_game_verification": "pending",
            },
        }
        if config.recorded_replay is not None:
            # The old path required a supplied capture to replay to the
            # recorded outcome before it published anything built on the
            # predicted root (`replay_recorded_line(required=True)`): a
            # capture that does not reproduce proves the root wrong.
            try:
                witness = rust_review.recorded_witness(
                    binary, root, config.recorded_replay, actual)
                document["recorded_line"] = rust_review.replay_line(
                    binary, root, witness, check)
            except (ValueError, KeyError, IndexError, TypeError,
                    NotImplementedError, OSError,
                    subprocess.SubprocessError) as exc:
                _write_line_status(
                    config.line_replay_status_path,
                    f"rust_recorded_replay:{type(exc).__name__}")
                raise phase1.ReviewRefusal(
                    "recorded_line_replay_failed",
                    "recorded observed line does not replay from the "
                    "predicted fight entry; refusing to publish a review of "
                    "a state that is known to differ from the real fight",
                    check=str(exc)[:300]) from exc
            document["metadata"]["opening_validation"] = "recorded_replay"
            document["metadata"]["recorded_replay"] = {
                "status": "complete", "engine": "rust",
                "final_digest": witness["final_digest"],
                "validation": "all_inputs_and_recorded_outcome",
                "native_checkpoints": witness["native_checkpoints"]}
        rust_review.record_potion_belt(document, root, config)
        document["metadata"]["elapsed_seconds"] = time.monotonic() - started
        return document
    except phase1.ReviewRefusal as exc:
        return refused(exc)
    except (NotImplementedError, ValueError, KeyError, TypeError, OSError,
            subprocess.SubprocessError) as exc:
        return refused(phase1.ReviewRefusal("simulation_refusal", str(exc)))


__all__ = [
    "COMBAT_STREAMS", "LegacyRootRefusal", "UnknownStreamConsumed",
    "build_root", "capture_counters", "counters_metadata", "entry_facts",
    "entry_preconditions", "fight_entry", "generate", "history_prediction",
    "placeholder_check",
    "resolve_counters", "rust_opening",
]

