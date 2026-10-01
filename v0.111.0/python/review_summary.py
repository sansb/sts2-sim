"""Shared review-document vocabulary for the Rust review producers.

Every review is now built in Rust (#2827): the floor-snapshot and
replay-capture paths by ``rust_review``, and the ``.run``-only path by
``rust_legacy_review``. This module keeps what they share, all of it
simulator-free: the CLI configuration (``ReviewConfig``), the refusal type and
document, build admission, the recorded-outcome projection
(``actual_outcome``) and the producer identity (``simulator_document``).

The retired Python producer that used to live here (``combat_sim`` roots,
``infoset_sample`` benchmark worlds, the Python recorded-line seed and its
renderer) was deleted by #2827 item F1. Its root builder, which the frozen
simulator's own tests still read, moved to ``python_fight_context`` and goes
with the simulator.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import time
from dataclasses import dataclass
from typing import Any, Sequence

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import sim_identity  # noqa: E402
from admission import (admitted_builds, build_state,  # noqa: E402
                       is_deferrable, require_admitted)
from relay_parser import FightState, RunSummary  # noqa: E402


SCHEMA_VERSION = 1
POST_COMBAT_RELIC_HEALS = {
    "RELIC.BURNING_BLOOD": 6,
    "RELIC.BLACK_BLOOD": 12,
}
# Post-combat max-HP gains that also heal the same amount (#2961). Chosen
# Cheese's `<AfterCombatEnd>d__4::MoveNext` (v0.111.0 RVA 0x321df8, IL_003e)
# calls `CreatureCmd::GainMaxHp(owner, MaxHp)` unconditionally, and
# `<GainMaxHp>d__22::MoveNext` (RVA 0x3eb2f0) raises MaxHp (IL_005b) and then
# heals the same amount (IL_010b). The cap rises with the heal, so the
# contribution is always exact.
POST_COMBAT_MAX_HP_GAINS = {
    "RELIC.CHOSEN_CHEESE": 1,
}
DEFAULT_K = 20
DEFAULT_SAMPLING_SEED = "review-pipeline-v1"
DEFAULT_HORIZON = 12
DEFAULT_ACTUAL_DEADLINE = 5.0
DEFAULT_BENCHMARK_DEADLINE = 2.0
HERE = pathlib.Path(__file__).parent


@dataclass(frozen=True)
class ReviewConfig:
    k: int = DEFAULT_K
    sampling_seed: str = DEFAULT_SAMPLING_SEED
    horizon: int = DEFAULT_HORIZON
    actual_deadline: float | None = DEFAULT_ACTUAL_DEADLINE
    benchmark_deadline: float | None = DEFAULT_BENCHMARK_DEADLINE
    fully_unlocked_card_pool: bool = False
    shuffle_counter: int | None = None
    selection_counter: int | None = None
    targets_counter: int | None = None
    energy_costs_counter: int | None = None
    generation_counter: int | None = None
    potion_generation_counter: int | None = None
    orb_generation_counter: int | None = None
    recorded_replay: dict[str, Any] | None = None
    line_replay_status_path: pathlib.Path | None = None
    # Optional deployment override.  Without one, the review CLI discovers
    # the configured STS_SIM_EXACT_SOLVER/release artifact.  Rust builds the
    # root, searches and replays; its typed refusals are the review's.
    rust_exact_solver_binary: pathlib.Path | None = None
    # Potion holdback (the player's own restraint, not a game rule): keep at
    # least `potion_hold_count` of the entry belt's drinkable potions, and
    # never drink the entry belt's potions in `potion_hold_slots`. The
    # defaults admit every drink.
    potion_hold_count: int = 0
    potion_hold_slots: tuple[int, ...] = ()

    def potion_holdback_requested(self) -> bool:
        return self.potion_hold_count > 0 or bool(self.potion_hold_slots)

    def validate(self) -> None:
        if type(self.k) is not int or self.k <= 0:
            raise ValueError(f"k must be a positive integer, got {self.k!r}")
        if not isinstance(self.sampling_seed, str) or not self.sampling_seed:
            raise ValueError("sampling_seed must be a nonempty string")
        if type(self.horizon) is not int or self.horizon <= 0:
            raise ValueError(
                f"horizon must be a positive integer, got {self.horizon!r}")
        for name, value in (
                ("actual_deadline", self.actual_deadline),
                ("benchmark_deadline", self.benchmark_deadline)):
            if value is not None and (
                    not isinstance(value, (int, float)) or value <= 0):
                raise ValueError(f"{name} must be positive or null, got {value!r}")
        for name, value in self.counter_overrides().items():
            if type(value) is not int or value < 0:
                raise ValueError(
                    f"{name} must be a non-negative integer, got {value!r}")
        if self.recorded_replay is not None and not isinstance(
                self.recorded_replay, dict):
            raise ValueError("recorded_replay must be an object or null")
        if self.line_replay_status_path is not None and not isinstance(
                self.line_replay_status_path, pathlib.Path):
            raise ValueError(
                "line_replay_status_path must be a pathlib.Path or null")
        if self.rust_exact_solver_binary is not None and not isinstance(
                self.rust_exact_solver_binary, pathlib.Path):
            raise ValueError(
                "rust_exact_solver_binary must be a pathlib.Path or null")
        if type(self.potion_hold_count) is not int or self.potion_hold_count < 0:
            raise ValueError(
                "potion_hold_count must be a non-negative integer, "
                f"got {self.potion_hold_count!r}")
        slots = self.potion_hold_slots
        if (not isinstance(slots, tuple)
                or any(type(s) is not int or not 0 <= s < 64 for s in slots)
                or len(set(slots)) != len(slots)):
            raise ValueError(
                "potion_hold_slots must be a tuple of distinct slots in 0..63, "
                f"got {slots!r}")

    def counter_overrides(self) -> dict[str, int]:
        return {
            name: value for name, value in (
                ("shuffle", self.shuffle_counter),
                ("combat_card_selection", self.selection_counter),
                ("combat_targets", self.targets_counter),
                ("combat_energy_costs", self.energy_costs_counter),
                ("combat_card_generation", self.generation_counter),
                ("combat_potion_generation", self.potion_generation_counter),
                ("combat_orbs", self.orb_generation_counter),
            ) if value is not None
        }


class ReviewRefusal(Exception):
    """A deliberate, machine-classified refusal to emit a partial card.

    ``deferred`` distinguishes "not yet" from "no" (#1268). Both stop a card
    being emitted and both are stored as completed results — a stored answer
    is what stops the worker re-solving the same fight every scan — but a
    deferral makes no claim about the fight and is expected to be revisited,
    while a refusal is this bundle's final word.
    """

    def __init__(self, reason: str, message: str, *, deferred: bool = False,
                 **details: Any):
        super().__init__(message)
        self.reason = reason
        self.message = message
        self.deferred = deferred
        self.details = details


def require_review_admission(run: RunSummary) -> None:
    """Refuse (or defer) a run whose build this bundle does not admit.

    Shared by the Rust legacy path (`rust_legacy_review.generate`, #2827 C2)
    and the frozen Python context loader the simulator's tests still read
    (`python_fight_context`), so both answer "not yet" and "no" identically.
    """

    # Admission (SOLVER_INVARIANTS.md I11, #1258). This is the boundary where
    # a real run enters the product, and it is deliberately here rather than
    # in the engine: the frozen Python engine was reached by ~460 test call
    # sites built on the v0.108 fight_states pins, which this bundle could
    # never admit. Those pins and that engine were deleted (#2827 item F); an
    # engine-level check is #1272's job.
    #
    # A refusal is the correct answer for every build we have not verified.
    # The failure it prevents is silent: I5 catches the *unmodeled*, but an
    # older run is *mis-modeled* -- every mechanic modeled, none of them as
    # that build behaved -- so the review rendered a confident number for a
    # fight that did not play that way.
    try:
        require_admitted(run.build_id)
    except NotImplementedError as exc:
        # "Not yet" and "no" are different answers, and the line between them
        # is whether the evidence still exists — not how old the build is
        # (#1268). A `pending` build has an archived DLL, so its semantics
        # can still be verified and this fight becomes reviewable if that
        # bundle is ever built. A build absent from the registry never had
        # its DLL archived (#309), so no future work can make it reviewable.
        state = build_state(run.build_id)
        deferred = is_deferrable(run.build_id)
        message = str(exc) if not deferred else (
            f"game build {run.build_id} is archived but not yet admitted "
            f"(registry state {state!r}); this fight has no result yet "
            "rather than no result ever, and is regenerated if that build "
            "is admitted (sim/meta/SIM_VERSIONING.md — the admitted set grows "
            "forward only)")
        raise ReviewRefusal(
            "game_build_pending_admission" if deferred
            else "unadmitted_game_build",
            message, deferred=deferred,
            game_build=run.build_id,
            registry_state=state,
            admitted_builds=list(admitted_builds())) from exc


def generate_review_document(
        run_path: str | pathlib.Path, fight_index: int, *,
        config: ReviewConfig = ReviewConfig()) -> dict:
    """Return one complete card or refusal for a ``.run``-only fight.

    A review is built entirely in Rust (`rust_legacy_review.generate`,
    #2827 C2): the root from the ``.run`` facts and the Rust counter
    prediction, then Rust search and Rust replay. The classify-only
    eligibility seam and the Python producer behind it were deleted by #2827
    item F1 with their only reader, the refusal census.
    """

    import rust_legacy_review
    return rust_legacy_review.generate(run_path, fight_index, config)


def actual_outcome(fight: FightState) -> dict:
    """Project the recorded result onto the solver's combat-end HP seam.

    ``.run`` HP is recorded after the Ironclad starter relic's ordinary
    post-combat heal, while ``solve`` compares combat-ending HP (plus its
    separately modeled Meat on the Bone early-victory projection). Burning
    Blood heals 6 and its Black Blood upgrade heals 12. Below max HP that
    contribution is exact; a max-HP endpoint does not reveal how much of the
    capped heal landed, so that composition refuses instead of guessing.
    Chosen Cheese's post-combat max-HP gain heals its amount too and is
    removed first; it raises the cap with it, so it is never ambiguous.
    """

    won = fight.hp_after > 0
    final_hp = fight.hp_after
    hp_healed = fight.hp_healed
    if won:
        # Removed first: the max-HP gain lifts the cap by what it heals, so
        # it commutes with the capped heals below and is never ambiguous.
        max_hp_gain = sum(
            amount for relic, amount in POST_COMBAT_MAX_HP_GAINS.items()
            if relic in fight.relics_entering)
        final_hp -= max_hp_gain
        hp_healed -= max_hp_gain
    ordinary_postcombat_heal = sum(
        amount for relic, amount in POST_COMBAT_RELIC_HEALS.items()
        if relic in fight.relics_entering)
    if won and ordinary_postcombat_heal:
        if final_hp == fight.max_hp_entering:
            raise ReviewRefusal(
                "observed_final_hp_ambiguous",
                "an ordinary post-combat relic heal ended at the max-HP "
                "cap, so .run does not separate combat-ending HP from the "
                "capped heal",
                hp_after=fight.hp_after,
                max_hp=fight.max_hp_entering,
                hp_healed=fight.hp_healed)
        if hp_healed < ordinary_postcombat_heal:
            raise ReviewRefusal(
                "observed_final_hp_ambiguous",
                "recorded healing is smaller than the uncapped ordinary "
                "post-combat relic heal",
                hp_healed=fight.hp_healed,
                postcombat_heal=ordinary_postcombat_heal)
        final_hp -= ordinary_postcombat_heal
    return {
        "won": won,
        "entry_hp": fight.hp_entering,
        "final_hp": final_hp,
        "hp_lost": fight.hp_entering - final_hp,
        "potions_used": {
            "count": len(fight.potions_used),
            "names": list(fight.potions_used),
        },
        "turns": fight.turns_taken,
    }


def _write_line_replay_divergence(
        path: pathlib.Path | None, turn: int | None, check: str) -> None:
    """Write the worker-only diagnostic side channel, never card data."""

    if path is None:
        return
    try:
        path.write_text(json.dumps({"turn": turn, "check": check}))
    except OSError:
        pass


def simulator_document(
        run_build: str | None = None,
        rust_exact_solver_binary: pathlib.Path | None = None) -> dict:
    """The producer's identity, and the proof it was routed correctly.

    ``run_build`` is checked rather than trusted (SIM_VERSIONING.md, "verify,
    do not route"). Admission has already run by the time a document is built,
    so a disagreement here is a dispatcher mis-route, and the only safe thing
    to do with one is refuse to stamp a document that would look ordinary.
    """

    identity = sim_identity.identity(rust_exact_solver_binary)
    if run_build is not None and run_build != identity["game_build"]:
        raise NotImplementedError(
            f"run build {run_build} reached the {identity['game_build']} "
            "bundle; a document must never record semantics from a build the "
            "fight did not play on (SOLVER_INVARIANTS.md I11/I12)")
    return identity


def _refusal_document(identity: dict, config: ReviewConfig,
                      refusal: ReviewRefusal, started: float) -> dict:
    # A deferral is a completed result with no claim in it, so it is stored
    # like a refusal but is deliberately a different status: consumers must
    # not tell a user "not reviewable" about a fight we expect to review
    # later, and a query must be able to find the population to regenerate.
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "deferred" if refusal.deferred else "refused",
        "fight": identity,
        ("deferral" if refusal.deferred else "refusal"): {
            "reason": refusal.reason,
            "message": refusal.message,
            "details": refusal.details,
        },
        "metadata": {
            "k": config.k,
            "sampling_seed": config.sampling_seed,
            "horizon": config.horizon,
            "deadlines_seconds": {
                "best_actual_seed": config.actual_deadline,
                "benchmark_world": config.benchmark_deadline,
            },
            "elapsed_seconds": {"total": time.monotonic() - started},
            # A refusal is a completed result, and its REASON is a claim the
            # simulator makes: "this bundle does not model that". Modelling
            # the mechanic, or admitting the build, changes the answer, so a
            # stored refusal needs the same identity as a card or it never
            # regenerates. Deliberately unvalidated against the run's build —
            # `unadmitted_game_build` is precisely the case where they differ,
            # and it records the run's build in `refusal.details`.
            "simulator": simulator_document(
                rust_exact_solver_binary=config.rust_exact_solver_binary),
            "counter_overrides": config.counter_overrides(),
        },
    }


def _deadline(value: str) -> float | None:
    if value.lower() in {"none", "null", "unlimited"}:
        return None
    parsed = float(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("deadline must be positive or 'none'")
    return parsed


def load_recorded_replay(
        path: pathlib.Path | None, status_path: pathlib.Path | None
        ) -> dict | None:
    if path is None:
        return None
    try:
        replay = json.loads(path.read_text())
        if not isinstance(replay, dict):
            raise TypeError("decoded replay is not an object")
        return replay
    except (OSError, json.JSONDecodeError, TypeError):
        _write_line_replay_divergence(
            status_path, None, "recorded_replay_decode")
        return None


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="emit one Phase 1a fight-review card JSON document")
    parser.add_argument("run", type=pathlib.Path)
    parser.add_argument("fight_index", type=int)
    parser.add_argument("-k", type=int, default=DEFAULT_K)
    parser.add_argument("--sampling-seed", default=DEFAULT_SAMPLING_SEED)
    parser.add_argument("--horizon", type=int, default=DEFAULT_HORIZON)
    parser.add_argument(
        "--actual-deadline", type=_deadline,
        default=DEFAULT_ACTUAL_DEADLINE,
        help="seconds, or 'none' for an exact unbounded solve")
    parser.add_argument(
        "--benchmark-deadline", type=_deadline,
        default=DEFAULT_BENCHMARK_DEADLINE,
        help="seconds per sampled world, or 'none'")
    parser.add_argument("--fully-unlocked-card-pool", action="store_true")
    parser.add_argument("--counter", type=int, dest="shuffle_counter")
    parser.add_argument("--sel-counter", type=int, dest="selection_counter")
    parser.add_argument("--targets-counter", type=int)
    parser.add_argument("--energy-costs-counter", type=int)
    parser.add_argument("--generation-counter", type=int)
    parser.add_argument("--potion-generation-counter", type=int)
    parser.add_argument("--orb-generation-counter", type=int)
    parser.add_argument("--recorded-replay", type=pathlib.Path)
    parser.add_argument("--line-replay-status", type=pathlib.Path)
    parser.add_argument("--out", type=pathlib.Path)
    parser.add_argument(
        "--no-print", action="store_true",
        help="write only --out; invalid without --out")
    args = parser.parse_args(argv)
    if args.no_print and args.out is None:
        parser.error("--no-print requires --out")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
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
        recorded_replay=load_recorded_replay(
            args.recorded_replay, args.line_replay_status),
        line_replay_status_path=args.line_replay_status)
    document = generate_review_document(
        args.run, args.fight_index, config=config)
    rendered = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if args.out is not None:
        args.out.write_text(rendered)
    if not args.no_print:
        sys.stdout.write(rendered)
    return 0 if document["status"] == "ok" else 2


if __name__ == "__main__":
    raise SystemExit(main())
