#!/usr/bin/env python3
"""The `.mcr` certification census and the v0.111.0 eval fixture set.

Coupled to #1283 (eval suite), #2048 (the fixture set), #1282 (Rust is the
engine authority: correctness is lockstep against the capture's own
per-action checksums, measured on the release binary) and #2999 (this tool
without the Python simulator).

Every fight is rooted, replayed and certified by the Rust engine alone:

  .mcr decode -> dedupe to the last snapshot per (seed, start_time, global
  node) -> pair with the FIRST save at that key -> `sts-sim entry --save
  --opening` (Rust's own opening) -> the capture's opening checkpoint ->
  `sts-sim diff-serve` load -> the recorded human line, resolved input by
  input to Rust's own wire actions -> every completed-action native checkpoint
  the capture carries.

Nothing here imports the Python simulator (`combat_sim`, `solve_fight`, the
content tree). The recorded-input resolution is the production review's own
(`python/rust_replay.py`, #2988): one resolver, reused, so the census and the
review cannot disagree about what a recorded input means.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional, Tuple

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
VERSION_DIR = RUST_DIR.parent
ROOT_DIR = VERSION_DIR.parent  # `sim/`, or the sts2-sim repo
SOLVER_DIR = VERSION_DIR / "python"
TOOLS_DIR = RUST_DIR / "tools"

sys.path.insert(0, str(SOLVER_DIR))
sys.path.insert(0, str(TOOLS_DIR))

# Simulator-free by construction (#2827 item D, #2999): the document layer
# (digest, build id), the `.mcr` byte decoder, the RNG, the capture's own deal
# and checkpoint rows, and the production recorded-input resolver.
import canonical_document  # noqa: E402
import mcr_native  # noqa: E402
import mcr_parser  # noqa: E402
import rust_replay  # noqa: E402
from sts2_rng import RUN_RNG_STREAMS, snake_case  # noqa: E402

DEFAULT_BINARY = RUST_DIR / "target" / "debug" / "sts-sim"
RELEASE_BINARY = RUST_DIR / "target" / "release" / "sts-sim"


def find_engine_binary(override: Optional[str] = None) -> pathlib.Path:
    if override:
        path = pathlib.Path(override).resolve()
        if not path.is_file():
            raise FileNotFoundError(f"specified binary not found: {path}")
        return path
    if RELEASE_BINARY.is_file():
        return RELEASE_BINARY
    if DEFAULT_BINARY.is_file():
        return DEFAULT_BINARY
    raise FileNotFoundError(
        f"sts-sim binary not found at {RELEASE_BINARY} or {DEFAULT_BINARY}. "
        "Run `cargo build --release --bin sts-sim` first."
    )


# ---------------------------------------------------------------------------
# Corpus census and the sim/v0.111.0/eval/ fixture set (#2048, #1283)
# ---------------------------------------------------------------------------
#
# Driver facts that cost a session to learn, kept here so the next reader does
# not re-derive them:
#
#   * Event-room fights carry map node type `unknown`. Their encounter comes
#     from the NEXT save at the same global node, under `pre_finished_room`.
#   * Rust `legal` lists ONE representative uid per group of physically
#     identical cards, but `apply` accepts the human's actual uid. The replay
#     applies the RECORDED uid and only records whether it was the group
#     representative (`rust_replay._play_is_offered`).
#   * The root is Rust's own opening, UNSPLICED. The capture's first checksum
#     ("After player turn start") is compared with it, never copied into it:
#     a splice would mask an opening that consumed the wrong number of draws
#     (I6, #2511), and certification is the comparison.

class EvalRefusal(NotImplementedError):
    """The fixture cannot be stated exactly, so nothing is written (I5)."""


class RustEntryCliError(RuntimeError):
    """`sts-sim entry` refused the ARGV, not the save (#2511).

    A refused *save* is a normal answer on stdout; this is the other kind —
    an unreadable file, an unadmitted `--build`, a binary that predates the
    subcommand — and it is a tool fault, never a census measurement.
    """


def _build_id() -> str:
    """The build this tool measures (the canonical document's stamp)."""
    return canonical_document.GAME_BUILD


BUILD_ID = _build_id()
CAPTURES_DEFAULT = pathlib.Path.home() / "sts2-captures"
EVAL_DIR = VERSION_DIR / "eval"
MANIFEST_SCHEMA = "sts-eval-manifest-v1"
PROVENANCE_SCHEMA = "sts-eval-provenance-v1"
HUMAN_LINE_SCHEMA = "sts-eval-human-line-v1"
REFUSAL_SCHEMA = "sts-eval-refusal-v1"

# Refusal-detail classes. Rust entry and opening refusals carry their own
# `refusal_class`; these name a Rust LOAD refusal's detail (boundary refusals
# name one field) and are the tags the committed fixture tree already uses.
_REFUSAL_CLASSES: Tuple[Tuple[str, str], ...] = (
    (r"^GLAM on power card", "glam_on_power"),
    (r"requires an explicit .* owner", "owner_provenance"),
    (r"requires (the exact )?recorded fully[- ]unlocked", "epoch_provenance"),
    (r"recorded dispatch order places", "relic_dispatch_order"),
    (r"with same-\w+ relic peers", "relic_peer_order"),
    (r"monster-node heal not modeled|boss-combat heal not modeled",
     "unmodeled_heal"),
    (r"^JOSS_PAPER requires", "joss_paper_exhaust_provenance"),
    (r"turn-start listener callback can suspend", "suspending_listener"),
    (r'field "splash_unlock_epochs" is unrepresentable', "rust_epoch_profile"),
    (r"^player: field|^monsters\[\d+\]: field|unmodeled field",
     "rust_unmodeled_field"),
)


_MISSING_AXES: Tuple[Tuple[str, str], ...] = (
    ("card keyword on", "card_keyword"),
    ("non-deterministic AI on", "monster_ai"),
    ("loop successor/branch on", "monster_loop"),
    ("per-instance state on", "per_instance_card_state"),
    ("enchantment ", "card_enchantment"),
    ("argument shape at", "argument_shape"),
    ("power ", "power"),
    ("relic ", "relic"),
    ("potion ", "potion"),
)


def missing_axis(item: str) -> str:
    """The axis one `MissingCapability` Display form belongs to."""
    for prefix, axis in _MISSING_AXES:
        if item.startswith(prefix):
            return axis
    return "other"


def rust_refusal_class(refusal: Dict[str, Any],
                       missing: List[str]) -> str:
    """One stable class for a Rust load refusal.

    A boundary (`unrepresentable_state`) refusal names exactly one field, so
    the detail classifies it. An admission refusal carries the FULL missing
    set, so the first axis it names classifies it and the census tallies all
    of them separately.
    """
    if refusal.get("kind") == "not_admitted" and missing:
        return missing_axis(missing[0])
    return refusal_class(refusal.get("detail") or "")


def refusal_class(detail: str) -> str:
    """One stable short name for a refusal detail (fixture tag + census row)."""
    for pattern, name in _REFUSAL_CLASSES:
        if re.search(pattern, detail or ""):
            return name
    return "other"


def fixture_id(seed: str, start_time: Any, node: int) -> str:
    """An opaque, deterministic fixture id.

    Consent for these captures is recorded on #2048; fixtures are keyed by an
    opaque id and never by a player, a file name, or a capture timestamp.
    """
    material = f"{seed}|{start_time}|{node}".encode("utf-8")
    return "f" + hashlib.sha256(material).hexdigest()[:15]


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


CANONICAL_SCHEMA = "sts-sim-canonical-v2"
#: The one root source (#2999). The frozen-Python `python` source and the
#: Rust-entry/Python-opening `rust` source were retired with the simulator;
#: the name is kept because every census JSON since #2693 records it.
ROOT_SOURCES = ("rust_opening",)

# ---------------------------------------------------------------------------
# The Rust-only provenance registry (#2693, #2751)
#
# The census no longer compares against a Python root. The registry and its
# type-strict path walk stay, and `test_eval_suite_hybrid_root.py` still pins
# the registry against `boundary.rs` in the `rust port` lane. Their last
# reader, `opening_census.py` (the Python-vs-Rust opening parity census), was
# retired by #2999, so #2827 item F can drop them with that test.
# ---------------------------------------------------------------------------

#: Boundary slots the **Rust** engine can emit and the frozen Python oracle
#: never can, because `combat_sim` has no counterpart for them at all.
#:
#: A REGISTRY, not an inference: a slot is listed here because
#: `sim/v0.111.0/engine/src/boundary.rs` says so in its own words, and
#: `derive_rust_only_slots` re-derives the set from that file so the two can
#: never drift apart silently (the #1432 *new-reader-invalidates-old-shortcut*
#: class). Today the set is `power_attachments` (#2693 S1-S3) and Scroll of
#: Biting's repeated-CHEW latch `scroll_chew_repeated` (#3026).
RUST_ONLY_PROVENANCE_SLOTS: Tuple[str, ...] = (
    "power_attachments", "scroll_chew_repeated")

BOUNDARY_RS = RUST_DIR / "src" / "boundary.rs"
#: `boundary.rs` marks such a slot with a `// Rust-only:` comment directly
#: above its `spec(...)` entry. The wire name is the spec's first string.
_RUST_ONLY_MARKER = re.compile(r"//\s*Rust-only\b")
_SPEC_WIRE_NAME = re.compile(r'spec\(\s*"([A-Za-z0-9_]+)"')


def derive_rust_only_slots(
        text: Optional[str] = None) -> Tuple[str, ...]:
    """The Rust-only wire slots `boundary.rs` itself declares.

    Read from the crate rather than restated, so `RUST_ONLY_PROVENANCE_SLOTS`
    is pinned against the file that decides the question. A `// Rust-only:`
    comment with no `spec(...)` after it is a malformed marker and raises,
    rather than being silently dropped.
    """
    if text is None:
        text = BOUNDARY_RS.read_text(encoding="utf-8")
    names: List[str] = []
    for marker in _RUST_ONLY_MARKER.finditer(text):
        spec = _SPEC_WIRE_NAME.search(text, marker.end())
        if spec is None:
            raise EvalRefusal(
                "boundary.rs marks a Rust-only slot with no spec() after it")
        names.append(spec.group(1))
    return tuple(sorted(set(names)))


def strip_rust_only_slots(
        value: Any,
        slots: Tuple[str, ...] = RUST_ONLY_PROVENANCE_SLOTS) -> Any:
    """`value` with every registered Rust-only key removed, at any depth.

    Keyed by wire NAME at any depth rather than by document path: the boundary
    tables own the name, and a name this distinctive cannot collide with an
    unrelated key. Values are never inspected — only the key is dropped.
    """
    if isinstance(value, dict):
        return {key: strip_rust_only_slots(item, slots)
                for key, item in value.items() if key not in slots}
    if isinstance(value, list):
        return [strip_rust_only_slots(item, slots) for item in value]
    return value


#: The wire-type name reported beside a path whose two sides disagree on TYPE.
#: Keyed by the exact Python type, never by `isinstance`, because `bool` is a
#: subclass of `int` and the whole point of this table is to keep them apart.
_WIRE_TYPE_NAMES: Dict[type, str] = {
    type(None): "null",
    bool: "bool",
    int: "int",
    float: "float",
    str: "str",
    list: "list",
    tuple: "tuple",
    dict: "dict",
}


def _wire_type_name(value: Any) -> str:
    """What this value is ON THE WIRE — a type name, never a value."""
    return _WIRE_TYPE_NAMES.get(type(value), type(value).__name__)


def _diff_paths(left: Any, right: Any, path: str = "") -> List[str]:
    """Dotted paths where two documents differ — paths only, never values.

    The comparison is TYPE-STRICT at every leaf (#2790): `canonical_json`, the
    bytes both engines digest, holds `1` and `true` apart even though Python's
    `==` does not. A TYPE pair is reported beside the path, in ARGUMENT order.
    """
    if isinstance(left, dict) and isinstance(right, dict):
        out: List[str] = []
        for key in sorted(set(left) | set(right)):
            if key not in left or key not in right:
                out.append(f"{path}.{key}" if path else key)
                continue
            out.extend(_diff_paths(left[key], right[key],
                                   f"{path}.{key}" if path else key))
        return out
    if isinstance(left, list) and isinstance(right, list):
        out = []
        if len(left) != len(right):
            out.append(f"{path}[]: length {len(left)} vs {len(right)}")
        for index in range(min(len(left), len(right))):
            out.extend(_diff_paths(left[index], right[index],
                                   f"{path}[{index}]"))
        return out
    if type(left) is not type(right):
        return [f"{path or '<root>'} "
                f"({_wire_type_name(left)} vs {_wire_type_name(right)})"]
    return [] if left == right else [path or "<root>"]


def type_strict_equal(left: Any, right: Any) -> bool:
    """`left == right` AND the same wire type at every leaf."""
    return not _diff_paths(left, right)


def opening_parity(python_document: Dict[str, Any],
                   rust_document: Dict[str, Any]) -> Tuple[bool, List[str]]:
    """The two-sided opening gate: `(agrees, differing_paths)`.

    Two roots agree when they are identical, TYPE-STRICTLY, except for
    `RUST_ONLY_PROVENANCE_SLOTS`. A reported type pair is `(python vs rust)`,
    this function's argument order. Its reader `opening_census.py` retired
    in #2999; the controls in `test_eval_suite_hybrid_root.py` still pin it.
    """
    paths = _diff_paths(strip_rust_only_slots(python_document),
                        strip_rust_only_slots(rust_document))
    return not paths, paths


# ---------------------------------------------------------------------------
# The root: Rust's own opening
# ---------------------------------------------------------------------------


def rust_opening_document(binary: pathlib.Path, save_path: pathlib.Path,
                          encounter: str, kind: str,
                          build: str, source: str = "--save",
                          native_checkpoints: bool = False) -> Dict[str, Any]:
    """`sts-sim entry --opening` on one save: the whole root, built by Rust.

    `source` is the input schema: `--save` for a game save, `--capture-run`
    for an imported upload's capture run (`entry_input`, #2915).

    Rust runs the entry AND the opening — the shuffle, deal, monster creation,
    room-entry and before-combat hooks and the first player turn — and what
    comes back is a `sts-sim-canonical-v2` document, byte-for-byte the thing
    `diff-serve` loads. An entry or opening refusal is a normal answer on
    stdout; a non-zero exit is an argv contract failure (`RustEntryCliError`).

    `native_checkpoints` adds `--native-checkpoints` (#3392): a built opening
    then comes back as `OPENING_CHECKPOINTS_SCHEMA`, split by
    `split_opening_checkpoints`.
    """
    argv = [str(binary), "entry", "--build", build, source, str(save_path),
            "--encounter", encounter, "--node-type", kind, "--opening"]
    if native_checkpoints:
        argv.append("--native-checkpoints")
    proc = subprocess.run(
        argv, capture_output=True, text=True, cwd=str(RUST_DIR), check=False)
    if proc.returncode != 0:
        raise RustEntryCliError(
            f"sts-sim entry --opening exited {proc.returncode}: "
            f"{proc.stderr.strip()[:200]}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RustEntryCliError(
            f"sts-sim entry --opening printed non-JSON: {exc}") from exc


#: The context of the capture's opening checkpoint (the game writes it after
#: the first player turn starts and before any input).
OPENING_CHECKPOINT = "After player turn start"

#: `sts-sim entry --opening --native-checkpoints` (#3392): the unchanged root
#: under `state`, beside the native checkpoints the opening's deal passed.
OPENING_CHECKPOINTS_SCHEMA = "sts-sim-opening-checkpoints-v1"
#: The one opening boundary: native's "After player turn start" checksum.
AFTER_PLAYER_TURN_START = "after_player_turn_start"


def split_opening_checkpoints(output: Dict[str, Any]
                              ) -> Tuple[Dict[str, Any], Optional[List[Any]]]:
    """`(root, native_checkpoints)` of one `entry --native-checkpoints` answer.

    A built opening is the wrapper; a refused entry or opening answers exactly
    as it does without the flag, so it is returned whole with `None`.
    """
    if output.get("schema") != OPENING_CHECKPOINTS_SCHEMA:
        return output, None
    return output["state"], output.get("native_checkpoints")


def opening_checkpoint_state(document: Dict[str, Any],
                             recorded: Optional[List[Any]]) -> Dict[str, Any]:
    """The Rust state the capture's opening checksum is compared against.

    Native writes "After player turn start" BEFORE turn one's
    `RunAutoPrePlayPhase` (`CombatManager/<StartTurn>d__100::MoveNext` RVA
    0x3f781c IL_096f/IL_0975, then IL_0b1e), but the root is post-AutoPre: a
    turn-one Imbued AutoPlay has already run in it (#3381). So the checksum is
    compared against the opening's recorded `after_player_turn_start` state
    (#3392). With nothing recorded -- `None`, recording not asked, or `[]`, a
    SetupPlayerTurn that paused on a choice, where native took the checksum at
    that pause and the root parks there too -- the root is the state compared.

    Nothing is skipped: an unknown kind, a kind recorded twice, a state that
    does not project, or a malformed report raises `ValueError`, which the
    caller records as the opening checkpoint's mismatch.
    """
    if recorded is None:
        return document
    if not isinstance(recorded, list):
        raise ValueError("the opening did not report its native checkpoints")
    states: Dict[str, Any] = {}
    for entry in recorded:
        kind = entry.get("kind") if isinstance(entry, dict) else None
        if kind != AFTER_PLAYER_TURN_START:
            raise ValueError(f"unknown opening native checkpoint kind {kind!r}")
        if kind in states:
            raise ValueError(f"opening native checkpoint {kind} recorded twice")
        if not isinstance(entry.get("state"), dict):
            raise ValueError(f"opening native checkpoint {kind} does not "
                             f"project: {entry.get('refusal')}")
        states[kind] = entry["state"]
    return states.get(AFTER_PLAYER_TURN_START, document)


def with_native_reset_order(document: Dict[str, Any],
                            replay: Dict[str, Any]) -> Tuple[Dict[str, Any], str]:
    """The root with the capture's own AfterEnergyReset order, and why.

    The production review root does the same (`rust_review._replay_opening`,
    #2669). The capture's opening checkpoint lists the hero's powers in
    `Creature.Powers` order, which is the order the AfterEnergyReset listeners
    fire in (`rust_replay.native_after_energy_reset_powers`, IL cited there).
    That order is recorded fact, not a proposal, so the root carries it as
    `player.after_energy_reset_order`. The order is explicit even when empty:
    no listener is live at the opening, and any later one is ordered by
    acquisition. Without it the admission gate refuses every root where Black
    Hole, a star source and another reset peer are all reachable. That
    refusal blocked every A9/A10 Regent boss capture (#3020).

    Nothing is spliced silently. The order is set only when the listener
    amounts Rust's opening built equal the checkpoint's; otherwise the root
    is returned unchanged and the status says why.
    """
    checksums = replay.get("checksums") or []
    first = checksums[0] if checksums else {}
    state = first.get("full_state") if first.get("context") == OPENING_CHECKPOINT else None
    if not state:
        return document, "no_opening_checkpoint"
    hero = next((c for c in state.get("creatures", ())
                 if c.get("player_id") is not None), None)
    if hero is None:
        return document, "no_opening_checkpoint"
    try:
        native = rust_replay.native_after_energy_reset_powers(hero)
    except ValueError:
        return document, "native_order_malformed"
    modeled = {name: document["player"].get(name, 0)
               for name in rust_replay.AFTER_ENERGY_RESET_NATIVE_IDS.values()}
    if {name: amount for name, amount in modeled.items() if amount} != dict(native):
        return document, "listener_amount_mismatch"
    order = [name for name, _ in native]
    existing = document["player"].get("after_energy_reset_order")
    if existing is not None:
        return document, ("native_checkpoint" if existing == order
                          else "rust_order_disagrees")
    player = dict(document["player"], after_energy_reset_order=order)
    return dict(document, player=player), "native_checkpoint"


def opening_checkpoint(document: Dict[str, Any],
                       replay: Dict[str, Any],
                       recorded: Optional[List[Any]] = None) -> Dict[str, Any]:
    """Compare Rust's opening with the capture's first native checkpoint.

    `recorded` is the opening's `native_checkpoints` report: the comparison
    is against `opening_checkpoint_state` (the pre-AutoPre state, #3392), and
    a malformed report is the checkpoint's mismatch, never a skip.

    The comparison is the production review's (`rust_replay.
    check_native_snapshot`, through `check_native_checkpoint` for the pet):
    player resources, the live monster roster, every pile's card ids and
    upgrades, and all nine combat RNG streams. It is a
    COMPARISON — the root is never spliced — so an opening that consumed the
    wrong draws shows here instead of being overwritten (I6).

    The per-stream RNG drift is recorded on its own as well, with the same
    meaning the Python census gave `opening_rng_drift` /
    `opening_counter_drift` before the splice (#2511): which streams' words or
    counters differ, and which counters.
    """
    checksums = replay.get("checksums") or []
    first = checksums[0] if checksums else {}
    native = first.get("full_state")
    if first.get("context") != OPENING_CHECKPOINT or not isinstance(native, dict):
        return {"checksummed": False}
    out: Dict[str, Any] = {"checksummed": True}
    report_error: Optional[str] = None
    try:
        document = opening_checkpoint_state(document, recorded)
    except ValueError as exc:
        report_error = f"{type(exc).__name__}: {exc}"[:220]
    drift: List[str] = []
    counter_drift: List[str] = []
    rng = document.get("rng") or {}
    for field, stream in mcr_native._REPRESENTED_RNG_STREAMS.items():
        try:
            recorded = mcr_native._rng_tuple(native.get("rng"), stream)
        except NotImplementedError:
            drift.append(field)
            counter_drift.append(field)
            continue
        modeled = rng.get(field) or {}
        if list(recorded[:4]) != modeled.get("words") \
                or recorded[4] != modeled.get("counter"):
            drift.append(field)
        if recorded[4] != modeled.get("counter"):
            counter_drift.append(field)
    out["opening_rng_drift"] = sorted(drift)
    out["opening_counter_drift"] = sorted(counter_drift)
    out["opening_counters_match"] = not counter_drift
    out["opening_power_mismatches"] = power_mismatch_rows(document, native)
    if report_error is not None:
        out["opening_checkpoint"] = "mismatch"
        out["opening_checkpoint_detail"] = report_error
        return out
    try:
        check_native_checkpoint(document, native)
        out["opening_checkpoint"] = "match"
    except (ValueError, NotImplementedError, KeyError, TypeError,
            IndexError) as exc:
        out["opening_checkpoint"] = "mismatch"
        out["opening_checkpoint_detail"] = f"{type(exc).__name__}: {exc}"[:220]
    return out


# ---------------------------------------------------------------------------
# The engine session and the recorded human line
# ---------------------------------------------------------------------------


class EngineSession:
    """One `sts-sim diff-serve` process.

    `ask` has the production review's contract (`rust_replay.RustReplay.ask`):
    the `ok` payload, or `ValueError("Rust replay refused: ...")`. The shared
    resolver `rust_replay._selection` depends on exactly that, so the census
    passes this object where the review passes its own session.
    """

    def __init__(self, binary: pathlib.Path):
        self.process = subprocess.Popen(
            [str(binary), "diff-serve"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, cwd=str(RUST_DIR))
        greeting = self.process.stdout.readline()
        try:
            protocol = json.loads(greeting).get("protocol")
        except json.JSONDecodeError:
            protocol = None
        if protocol != "diff-serve-v1":
            self.close()
            raise RuntimeError(
                f"sts-sim diff-serve greeted {greeting.strip()[:80]!r}")

    def request(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        self.process.stdin.write(json.dumps(payload) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("sts-sim diff-serve closed stdout")
        return json.loads(line)

    def load(self, document: Dict[str, Any]) -> Dict[str, Any]:
        return self.request({"cmd": "load", "entry": document})

    def ask(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        response = self.request(payload)
        if "ok" not in response:
            raise ValueError(f"Rust replay refused: {response}")
        return response["ok"]

    def close(self) -> None:
        try:
            self.process.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
            self.process.stdin.flush()
        except (BrokenPipeError, OSError, ValueError):
            pass
        self.process.terminate()
        self.process.wait()


#: Relic pets: player-side creatures a relic summons that the canonical
#: document carries as the relic, not as a creature. v0.111.0 `sts2.dll`
#: (sha256 9cb4f1ad…): `Byrdpip::SummonPet` (`<SummonPet>d__18::MoveNext`
#: RVA 0x3211e4, IL_001d-IL_0023) and `PaelsLegion::SummonPet`
#: (`<SummonPet>d__38::MoveNext` RVA 0x32cc88, IL_0023) both call
#: `PlayerCmd::AddPet<T>(Owner)`, so the creature is the player's
#: pet (`Creature::get_PetOwner`, `Byrdpip::SetupSkins` IL_0015-IL_001a). Each
#: monster model fixes the creature's whole observable state: MinInitialHp =
#: MaxInitialHp = 9999 (`Byrdpip::get_MinInitialHp` RVA 0xb08b8 /
#: `get_MaxInitialHp` 0xb08bf; `PaelsLegion` 0xbb33f / 0xbb346), no health bar
#: (`get_IsHealthBarVisible` ldc.i4.0, RVA 0xb08c6 / 0xbb34d), and a move state
#: machine of the single self-looping `NOTHING_MOVE` (`GenerateMoveStateMachine`
#: RVA 0xb091c / 0xbb3a0).
RELIC_PET_MONSTER_IDS = ("BYRDPIP", "PAELS_LEGION")
RELIC_PET_HP = 9999


def check_native_checkpoint(state: Dict[str, Any],
                            native: Dict[str, Any]) -> None:
    """`rust_replay.check_native_snapshot`, with the player's pets compared too.

    The production check lists every native creature carrying a `monster_id`
    as a monster, and a player-side pet is one. Unmapped, every checkpoint of
    a fight with a pet would read as a monster-roster divergence whatever the
    engine did, so each pet is compared with what the canonical document says
    about it and then removed before the production check runs on the rest:

    * Necrobinder's Osty (`monster_id` `OSTY`, no `player_id`) is the player's
      `ally` slot — `["OSTY", hp, max_hp]` while it lives, absent (null is
      elided) otherwise (`boundary.rs` `PlayerSlot::Ally`, `pet.rs`).
    * A relic pet (`RELIC_PET_MONSTER_IDS`) must still be in the state its
      monster model fixes — 9999/9999 HP and no Block. The document has no
      creature for it to compare against, so that fixed state is the claim.

    A comparison, never a substitution; no other creature is exempted.

    Not compared (#3335): Osty's `block` and `powers`, because the `ally`
    slot carries only `[kind, hp, max_hp]` — Rust's state has no pet Block or
    pet power list to compare them with; and a relic pet's `powers`, which
    the canonical document likewise does not represent.
    """
    creatures = native.get("creatures")
    if not isinstance(creatures, list):
        raise ValueError("native checkpoint creatures are not a list")
    osty = [c for c in creatures if c.get("monster_id") == "OSTY"]
    if len(osty) > 1:
        raise ValueError("native checkpoint carries more than one Osty")
    for pet in osty:
        expected = (["OSTY", pet["current_hp"], pet["max_hp"]]
                    if pet["current_hp"] > 0 else None)
        if state["player"].get("ally") != expected:
            raise ValueError("recorded replay differs from native Osty")
    relic_pets = [c for c in creatures
                  if c.get("monster_id") in RELIC_PET_MONSTER_IDS]
    for pet in relic_pets:
        if (pet["current_hp"], pet["max_hp"], pet["block"]) != (
                RELIC_PET_HP, RELIC_PET_HP, 0):
            raise ValueError(
                f"native relic pet {pet['monster_id']} left its fixed state")
    pets = osty + relic_pets
    if pets:
        native = dict(native, creatures=[
            c for c in creatures if not any(c is pet for pet in pets)])
    rust_replay.check_native_snapshot(state, native)


def power_mismatch_rows(state: Dict[str, Any],
                        native: Dict[str, Any]) -> List[Dict[str, Any]]:
    """The checkpoint's player-power disagreements as typed rows (#3029).

    `rust_replay.native_player_power_mismatches`, each as
    `{"class", "power", "native", "rust"}`. A checkpoint whose hero power
    list cannot be read is one `malformed_power_list` row, never a skip.
    """
    try:
        return [m.as_row() for m in
                rust_replay.native_player_power_mismatches(state, native)]
    except (ValueError, KeyError, TypeError) as exc:
        return [{"class": "malformed_power_list", "power": None,
                 "native": None, "rust": f"{exc}"[:120]}]


class _CensusChecks(rust_replay.NativeChecks):
    """`rust_replay.NativeChecks`, comparing through `check_native_checkpoint`.

    `completed` is the production body with the one call swapped, so the
    checkpoint cursor, the action-identity match and the pending skip stay
    the review's own. Each compared checkpoint's player powers are also
    compared (#3029) and the FIRST disagreeing checkpoint's rows kept in
    `power_mismatch`; that comparison never raises, so it cannot change the
    certification verdict by itself.
    """

    power_mismatch: Optional[List[Dict[str, Any]]] = None

    def completed(self, state):
        if self.owner is None or state['player'].get('pending'):
            return
        if self.checks:
            if self.cursor >= len(self.checks):
                raise ValueError('recorded completed-action checkpoint missing')
            checkpoint = self.checks[self.cursor]
            if not all(part in checkpoint['context'] for part in self.owner):
                raise ValueError('recorded checkpoint action identity mismatch')
            self.cursor += 1
            if checkpoint.get('full_state'):
                if self.power_mismatch is None:
                    rows = power_mismatch_rows(state, checkpoint['full_state'])
                    if rows:
                        self.power_mismatch = rows
                check_native_checkpoint(state, checkpoint['full_state'])
                self.validated += 1
        self.owner = None


class _RestoringSession:
    """The session `rust_replay._selection` sees, with `load` made exact.

    The shared resolver trial-applies each candidate answer and restores the
    pending state between trials by LOADING its projection. The engine
    refuses to load some suspended states (`player: field "pending" is
    unrepresentable`), which the census would report as a divergence the
    fight does not have. So a load of the current state is answered by
    re-loading the root and re-applying the line so far — the same state,
    rebuilt through the engine's own transitions rather than round-tripped
    through the boundary. Every other request passes through unchanged.
    """

    def __init__(self, session: "EngineSession", root: Dict[str, Any],
                 actions: List[Dict[str, Any]], current: Dict[str, Any]):
        self.session, self.root = session, root
        self.actions, self.current = actions, current

    def restore(self) -> None:
        self.session.ask({"cmd": "load", "entry": self.root})
        for action in self.actions:
            self.session.ask({"cmd": "apply", "action": action})

    def ask(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        if payload.get("cmd") == "load" and payload.get("entry") is self.current:
            self.restore()
            return {"digest": None}
        return self.session.ask(payload)


def _multi_card_selection(session: _RestoringSession, before: Dict[str, Any],
                          uids: List[int]) -> Dict[str, Any]:
    """A recorded PlayerChoice naming MORE than one physical card.

    The production resolver takes one card (`rust_replay._selection`). The
    same rule, for k cards: the unique legal `select` whose answer the
    engine itself describes (`describe_selection`, `engine::
    selected_card_uids`) as exactly the recorded uids, in the recorded order.

    The engine applies that rule in-process (`resolve_selection`,
    `engine::recorded_selection_answer`, #3125): an ordered pick surface
    such as Gambling Chip's (326 answers for a 5-card hand) or a 3-of-8
    frame pick (336) is far past any per-answer wire budget. It also names
    Ashwater's and Gambler's Brew's ordered answers, which `legal` does not
    offer (#2524). A null answer is no match; two matches refuse in Rust.
    """
    resolved = session.ask({"cmd": "resolve_selection", "uids": uids})
    if resolved.get("action") is None:
        raise ValueError("recorded selection has no unique supported Rust answer")
    return resolved["action"]


class LineDiverged(Exception):
    """One recorded input has no exact Rust counterpart.

    `check` is the stable class the census tallies (`diverged:<check>`);
    `detail` is verbatim for the row. `step` counts the actions applied
    before the divergence.

    `checkpoint_mismatch` is the first native completed-action checkpoint
    the line disagreed with BEFORE it diverged (`{"step", "detail"}`, or
    None), and `native_checkpoints` how many agreed before that. A line can
    be wrong against the game several inputs before an input stops
    resolving (#3025); the divergence must not hide that earlier answer.
    `power_mismatch` is the first checkpoint whose player powers disagreed
    (`{"step", "mismatches"}`, #3029), carried the same way.
    """

    def __init__(self, step: int, turn: Optional[int], check: str,
                 detail: str,
                 checkpoint_mismatch: Optional[Dict[str, Any]] = None,
                 native_checkpoints: int = 0,
                 power_mismatch: Optional[Dict[str, Any]] = None):
        super().__init__(detail)
        self.power_mismatch = power_mismatch
        self.step = step
        self.turn = turn
        self.check = check
        self.detail = detail
        self.checkpoint_mismatch = checkpoint_mismatch
        self.native_checkpoints = native_checkpoints


class CaptureTruncated(LineDiverged):
    """The capture stops while the combat it records is still live (#3277).

    An uploaded capture can be a snapshot of `latest.mcr` taken mid-fight.
    Rust applied every recorded input, agreed with every completed-action
    checkpoint, and the capture's own evidence says the fight had not ended
    where the file does (`_capture_ends_mid_combat`), so the missing end is
    the capture's, not the engine's. A LineDiverged subclass so every
    caller that only knows divergences still stops; the census names it
    `human = truncated`, never certified. `shape` names which evidence.
    """

    shape: Optional[str] = None


#: The two truncation shapes `_capture_ends_mid_combat` names.
TRUNCATED_AFTER_COMPLETED_ACTION = "after_completed_action"
TRUNCATED_INSIDE_PLAYER_DECISION = "inside_player_decision"


def _native_combat_live(checkpoint: Dict[str, Any]) -> bool:
    """A living player and a living enemy in one native checkpoint.

    Pets (Osty, relic pets) are the player's, not enemies.
    """
    creatures = (checkpoint.get("full_state") or {}).get("creatures")
    if not isinstance(creatures, list):
        return False
    pets = ("OSTY",) + RELIC_PET_MONSTER_IDS
    players = [c for c in creatures if c.get("player_id") is not None
               and c.get("monster_id") is None]
    enemies = [c for c in creatures if c.get("monster_id")
               and c.get("player_id") is None
               and c.get("monster_id") not in pets]
    return (bool(players) and all(c.get("current_hp", 0) > 0 for c in players)
            and any(c.get("current_hp", 0) > 0 for c in enemies))


def _capture_ends_mid_combat(replay: Dict[str, Any], checks: Any,
                             state: Dict[str, Any],
                             final_compare_is_last_input: bool
                             ) -> Optional[str]:
    """Which truncation shape the capture shows, or None when it shows none.

    Called only when the inputs ran out with Rust's combat live, and `checks`
    (the line's `NativeChecks`) is intact: no checkpoint disagreed. Both
    shapes need every completed-action checkpoint consumed and the
    capture's FINAL checksum, of any context, to show a living player and a
    living enemy. Then exactly one of:

    * `after_completed_action`: that final checksum IS the last
      completed-action checkpoint (`finished action execution`, nothing
      after it), and it was compared, and agreed, against Rust's state after
      the LAST applied input (`final_compare_is_last_input`: no end turn and
      no staged Void Form/AutoPost state after it). Natively the game was
      waiting on the next player input, which the file does not carry.
    * `inside_player_decision`: the last recorded input is a card play or
      potion that Rust leaves `pending` a player choice, and its own
      completed-action checkpoint is absent. The play itself shows natively
      that the combat was live when it was made, and the absent checkpoint
      shows the file stops before the action finished. Rust has not claimed
      the combat went on past it either. If the game had finished the action
      without a choice, its checkpoint would be in the file, left
      unconsumed, and this shape would not apply.

    Anything else is not claimed: a final checkpoint that shows every enemy
    or the player dead, a turn-boundary record after the last compared
    checkpoint, or a last input that is an end turn. Those rows stay
    `combat_not_complete`.
    """
    checksums = replay.get("checksums") or []
    recorded = getattr(checks, "checks", None) or []
    if not checksums or getattr(checks, "cursor", None) != len(recorded) \
            or not _native_combat_live(checksums[-1]):
        return None
    last = checksums[-1]
    if recorded and last is recorded[-1] and final_compare_is_last_input \
            and str(last.get("context", "")).startswith(
                "finished action execution "):
        return TRUNCATED_AFTER_COMPLETED_ACTION
    owner = getattr(checks, "owner", None) or ()
    if state["player"].get("pending") and owner and owner[0] in (
            "PlayCardAction card:", "UsePotionAction "):
        return TRUNCATED_INSIDE_PLAYER_DECISION
    return None


#: The recorded inputs a line consists of. After the engine reports combat
#: over, any of these still in the log is a premature end; everything else
#: (hooks, resumes, the enemy-turn ready marker) carries no player decision.
_RECORDED_INPUTS = frozenset({
    "NetPlayCardAction", "NetUsePotionAction", "NetEndPlayerTurnAction"})
_NO_DECISION_EVENTS = frozenset({"HookAction", "ResumeAction"})
#: What a native checkpoint comparison raises; recorded, never re-raised.
_CHECKPOINT_ERRORS = (ValueError, NotImplementedError, KeyError, TypeError,
                      IndexError)
RECORDED_INPUT_BUDGET = 2000


_WIRE_KEY_ORDER = ("kind", "uid", "slot", "target", "selection", "answer",
                   "index", "card", "choice")


def _wire_in_fixture_order(wire: Dict[str, Any]) -> Dict[str, Any]:
    """The same wire action with its keys in the fixture tree's order.

    Equal as a value either way; the order only keeps a re-seeded
    `human_line.json` byte-stable (`kind` first, as the stable v1 wire has
    always been written there) whichever resolver produced the wire.
    """
    def ordered(value: Dict[str, Any]) -> Dict[str, Any]:
        rank = {key: index for index, key in enumerate(_WIRE_KEY_ORDER)}
        return {key: (ordered(value[key]) if isinstance(value[key], dict)
                      else value[key])
                for key in sorted(value, key=lambda k: (rank.get(k, 99), k))}
    return ordered(wire)


def _turn(state: Dict[str, Any]) -> int:
    return state["player"].get("turn", 1)


def _terminal(state: Dict[str, Any]) -> Dict[str, Any]:
    """The line's end state, in the fixture's `terminal` shape."""
    player = state["player"]
    hp = player.get("hp", 0)
    over = bool(player.get("over"))
    return {"hp": hp, "turn": _turn(state), "over": over,
            "won": bool(over and hp > 0)}


def _same_card(card: Dict[str, Any], recorded) -> bool:
    """Whether a root card IS the recorded deck card, by identity.

    The id must agree. The upgrade may only have grown: an opening that
    upgrades cards (a combat-start upgrade of the dealt hand) changes the
    payload, never the physical card, and the native checkpoints compare the
    live upgrade levels pile by pile afterwards.

    Deliberately id/upgrade only, never enchantment: a root's own card does
    not carry every enchantment the capture's deck records (v0.111.0's
    canonical entry drops CLONE — a purely cosmetic "this is a duplicate"
    marker with no gameplay hook — while the capture still reports it, #3329
    census run on seed 7U6NANVCTHUE nodes 29/32/34). Requiring exact
    enchantment agreement here would refuse every one of those otherwise
    unambiguous MIND_BLAST+CLONE cards. `_dealt_uids` instead calls
    `_same_enchantment` itself, and only as a tiebreaker once id/upgrade
    alone already leaves more than one candidate.
    """
    if card.get("vanished"):
        return True
    return (card["id"] == recorded[0]
            and card.get("upgrade", 0) >= (recorded[1] or 0))


def _same_enchantment(card: Dict[str, Any], recorded_enchantment) -> bool:
    """Whether a root card's own enchantment matches the recorded one.

    `recorded_enchantment` is `(id, level)` or `None`, the third element
    `_first_cycle_instances` reports per recorded deck row. Used ONLY to
    break an id/upgrade tie in `_dealt_uids` (#3329): a Royally Approved copy
    of a card is a physically distinct card from a plain copy of the same id
    at the same upgrade (Queen fight f5549cb3085a3c2b's `BATTLE_TRANCE+0` is
    the next card in both the Innate queue's enchanted copy and the plain
    queue — id/upgrade alone ties them, and `_same_card` deliberately never
    resolves this, since not every root-modeled tie's enchantment survives
    to the wire; see `_same_card`'s docstring). Restricting this comparison
    to a tiebreaker, rather than folding it into every candidate's identity
    check, is what keeps CLONE-enchanted (root-silent) decks unaffected: an
    unambiguous id/upgrade match never reaches this function at all.
    """
    card_enchantment = card.get("enchantment")
    if recorded_enchantment is None:
        return not card_enchantment
    return (bool(card_enchantment)
            and tuple(card_enchantment) == tuple(recorded_enchantment))


def _dealt_uids(root: Dict[str, Any], instances: list) -> List[int]:
    """The uid of each recorded instance, undoing SetupPlayerTurn's rewrite.

    Rust numbers the dealt deck in its post-opening pile order (Hand, then
    Draw), and SetupPlayerTurn has already moved every IMBUED card to the
    Draw BOTTOM and every remaining Innate card to the FRONT, in reversed
    relative order. The capture's first cycle is the RAW shuffle order, so
    the two only line up position by position when no card moved (#2552).

    This is the frozen Python `mcr_replay._deal_uid_map` inversion restated
    over the canonical document: `player.innate_min_draw` is exactly the
    number of cards pulled to the front, so `dealt[:innate_min_draw]`
    reversed is those cards in their original relative order, and the IMBUED
    tail is read from each card's `enchantment`. Cards the opening created
    are left out: they are not in the shuffled deck the capture records. The three subsequences each
    keep their original relative order, so walking the recorded cycle
    against their heads reconstructs the raw order — exactly, because the
    walk must be the ONLY complete reading of the cycle (`_interleavings`):
    a head offered by two queues is decided by which choice completes, and
    refused when both do.
    """
    dealt = list(root["piles"].get("hand", [])) + list(
        root["piles"].get("draw", []))
    # Turn one's AutoPre phase auto-plays an IMBUED deck card out of the Draw
    # BOTTOM, where the rewrite put it (#3381,
    # `engine::play::autoplay_imbued_turn_one`), so the root holds it in its
    # result pile. Put it back at the end of the Draw, in uid order, to
    # rebuild the post-rewrite layout the inversion below undoes.
    dealt += sorted(
        (c for name, pile in root["piles"].items()
         if name not in ("hand", "draw") for c in pile
         if c.get("enchantment") and c["enchantment"][0] == "IMBUED"
         and c["uid"] < len(instances)),
        key=lambda c: c["uid"])
    innate = root["player"].get("innate_min_draw", 0)
    if type(innate) is not int or not 0 <= innate <= len(dealt):
        raise ValueError("Rust root innate_min_draw is out of range")
    # A card the opening CREATED (Gremlin Horn's Dazed, Orange Dough, Bag of
    # Preparation's draws of a generated card...) is not in the capture's
    # first cycle, which is the shuffled deck only. Created cards take uids
    # after the deck from the shared allocator, so the deck is exactly the
    # dealt cards below `len(instances)`; the created ones are matched later
    # by that allocator (#3089). The front/tail split is taken first, on the
    # dealt piles as Rust laid them out, and only then are created cards
    # dropped, so each queue keeps the deck cards' relative order.
    deck_size = len(instances)
    if "RELIC.WHISPERING_EARRING" in (root["player"].get("relics_entering") or ()):
        # Whispering Earring's turn-one AutoPre loop (#3414,
        # `engine::turn::whispering_earring_loop_is_ahead`) AutoPlays live
        # Hand cards after the deal, so deck cards leave `hand ++ draw` for
        # their result piles, and a played Power leaves every pile. The deck
        # was numbered in the post-rewrite layout, so the deck cards in uid
        # order ARE that layout (the Jeweled Mask reading below). A uid in no
        # pile is a vanished Power: it holds its place as a placeholder that
        # `_same_card` matches to whatever the cycle records there. Every
        # other position still matches exactly, the cycle is the whole deck,
        # and `_interleavings` still refuses a second reading, so the
        # placeholder's identity is forced; the native checkpoints compare
        # the loop's effects afterwards.
        present = {c["uid"]: c for pile in root["piles"].values() for c in pile
                   if c["uid"] < deck_size}
        dealt = [present.get(uid, {"uid": uid, "id": None, "vanished": True})
                 for uid in range(deck_size)]
    elif "RELIC.JEWELED_MASK" in (root["player"].get("relics_entering") or ()):
        # Jeweled Mask's turn-1 BeforeHandDraw (`JeweledMask/<BeforeHandDraw>
        # d__2::MoveNext`, v0.111.0 RVA 0x3272f4) moves one deck Power from
        # mid-Draw to the Hand AFTER the deck was numbered, and the moved card
        # keeps its uid (#3170). So the deck cards in uid order are the layout
        # before the lift, which is the layout the Innate/IMBUED inversion
        # below undoes. Only this relic takes the reorder; every other root
        # keeps the layout exactly as Rust laid it out.
        dealt = sorted((c for c in dealt if c["uid"] < deck_size),
                       key=lambda c: c["uid"])
    front = [c for c in reversed(dealt[:innate]) if c["uid"] < deck_size]
    tail = [c for c in dealt[innate:] if c["uid"] < deck_size]
    if sorted(c["uid"] for c in front + tail) != list(range(deck_size)):
        raise ValueError("recorded deck size differs from the Rust deal")
    queues = [front]
    imbued = [bool(c.get("enchantment")) and c["enchantment"][0] == "IMBUED"
              for c in tail]
    queues.append([c for c, flag in zip(tail, imbued) if not flag])
    if any(imbued):
        queues.append([c for c, flag in zip(tail, imbued) if flag])
    solutions = _interleavings(queues, instances)
    if len(solutions) != 1:
        raise ValueError("recorded initial physical card identities differ")
    return solutions[0]


#: Search nodes `_interleavings` may visit before it refuses a deal.
DEAL_SEARCH_BUDGET = 100_000


def _interleavings(queues: List[List[Dict[str, Any]]],
                   instances: list) -> List[List[int]]:
    """Every way (at most two) to read `instances` off the queue heads.

    Each recorded card must be the head of some queue (`_same_card`). When a
    card could be the head of two queues, this function first narrows by
    enchantment (#3329, below). If more than one head is still left, it tries
    each one. A tie is often settled a few cards later, when only one choice
    can read the whole cycle. For example, an Innate BIG_BANG+1 was upgraded
    from +0 at the opening, so it heads the Innate queue and matches a
    recorded +0 as well as the plain queue's +0 does. Only the choice that
    puts the plain +0 first reads the rest of the cycle (UE7YGG9XC3ZB node 29,
    #3152).

    Search is exhaustive. It stops once it has found a second complete
    reading, and the caller refuses unless there is exactly one. So a deal
    with one reading gets the uids a greedy walk would give, and a deal with
    two genuine readings still refuses. Going over `DEAL_SEARCH_BUDGET`
    also refuses.
    """
    solutions: List[List[int]] = []
    heads = [0] * len(queues)
    uids: List[int] = []
    visited = 0

    def walk(index: int) -> None:
        nonlocal visited
        visited += 1
        if visited > DEAL_SEARCH_BUDGET:
            raise ValueError("recorded initial physical card identities differ")
        if index == len(instances):
            solutions.append(list(uids))
            return
        recorded = instances[index]
        matched = [q for q, queue in enumerate(queues)
                   if heads[q] < len(queue)
                   and _same_card(queue[heads[q]], recorded)]
        if len(matched) > 1 and len(recorded) > 2:
            # An id/upgrade tie across queue heads (#3329): narrow by each
            # candidate's own enchantment, but only commit if that leaves
            # exactly one — a genuine physical tie (both candidates share
            # the same enchantment too) is left for the search, which
            # refuses it when both readings complete.
            narrowed = [q for q in matched
                        if _same_enchantment(queues[q][heads[q]], recorded[2])]
            if len(narrowed) == 1:
                matched = narrowed
        for q in matched:
            uids.append(queues[q][heads[q]]["uid"])
            heads[q] += 1
            walk(index + 1)
            heads[q] -= 1
            uids.pop()
            if len(solutions) > 1:
                return

    walk(0)
    return solutions


def _first_cycle_instances(replay: Dict[str, Any]) -> List[Tuple[Any, ...]]:
    """`mcr_native.first_cycle_from_replay`, extended with each card's own
    enchantment identity: `(id, upgrade, enchantment)` per recorded deck row,
    in first-cycle order.

    `mcr_native.first_cycle_from_replay` (shared with the production replay,
    `rust_replay.py`) reports only `(id, upgrade)`; the third element here is
    local to this harness (#3329) rather than a change to that shared
    function's return shape, which other consumers key on positionally.
    `enchantment` is `(id, level)` from the capture's own `card.enchantment`
    (`mcr_parser.McrDecoder.enchantment`), or `None` when the row carries
    none — the same identity `_same_card` now compares.
    """
    player = mcr_native.require_single_player(replay["run"], "MCR replay")
    deck = player["deck"]
    rows = mcr_native.first_cycle_rows_from_replay(replay)

    def enchantment(card: Dict[str, Any]) -> Optional[Tuple[str, int]]:
        found = card.get("enchantment")
        return (found["id"], found.get("level", 0)) if found else None

    return [(deck[row]["id"].removeprefix("CARD."),
             deck[row].get("upgrade_level")
             or deck[row].get("current_upgrade_level") or 0,
             enchantment(deck[row]))
            for row in rows]


def _opening_created_uids_are_exact(root: Dict[str, Any],
                                    deck_size: int) -> bool:
    """Whether a Thieving Hopper root's uids past the deck are exactly the
    cards its opening created.

    The Hopper form numbers the deck by master-deck row, `0..deck_size`
    (#2805). A card created during the opening (Vexing Puzzlebox's card,
    Y3NULJSNND7N node 21) takes the next allocator uid after the deck, the
    same numbering the capture gives it. So `next_card_uid` is `deck_size`
    plus the number of cards created. Every uid from `deck_size` up to
    `next_card_uid` must be held by exactly one root card, and no card may
    hold a uid outside the deck and that range. With nothing created, this
    is the old `next_card_uid == deck_size` check.
    """
    next_uid = root["player"].get("next_card_uid")
    if type(next_uid) is not int or next_uid < deck_size:
        return False
    created = sorted(c["uid"] for pile in root["piles"].values()
                     for c in pile if c["uid"] >= deck_size)
    return created == list(range(deck_size, next_uid))


def _deal_uid_map(root: Dict[str, Any], replay: Dict[str, Any]):
    """`combat_card_index -> physical uid`, authenticated against the deal.

    The capture's own first cycle (`_first_cycle_instances`) must name the
    card the Rust root holds at every deck uid before a uid is trusted — the
    production review's rule (`rust_replay.recorded_witness`), including its
    Thieving Hopper form where deck uids are master-deck rows (#2805), and
    extended by `_dealt_uids` to the Innate/IMBUED pile rewrite the
    production rule refuses.  Cards created in combat share the allocator
    after the deck either way.
    """
    instances = _first_cycle_instances(replay)
    cards = {c["uid"]: c for pile in root["piles"].values() for c in pile}
    if "hopper_master_deck" in root["player"]:
        rows = mcr_native.first_cycle_rows_from_replay(replay)
        master = root["player"]["hopper_master_deck"]
        if (sorted(row for row, _ in master) != list(range(len(rows)))
                or not _opening_created_uids_are_exact(root, len(rows))):
            raise ValueError(
                "recorded master-deck rows differ from the Rust root")
        for index, card in enumerate(instances):
            if rows[index] not in cards \
                    or not _same_card(cards[rows[index]], card):
                raise ValueError(
                    "recorded initial physical card identities differ")
        deck = list(rows)
    else:
        deck = _dealt_uids(root, instances)

    def uid_map(index: Any) -> int:
        if type(index) is not int or index < 0:
            raise ValueError("invalid recorded physical card index")
        # Past the entry deck, instances are creation-ordered, and so is the
        # allocator: the j-th card created in combat is uid len(deck) + j.
        return deck[index] if index < len(deck) else index
    return uid_map


def replay_recorded_line(session: EngineSession, root: Dict[str, Any],
                         replay: Dict[str, Any]) -> Dict[str, Any]:
    """Resolve every recorded input to a Rust action and apply it, in order.

    This is `rust_replay.recorded_witness` (the production review's replay,
    #2988) made into a MEASUREMENT: the same identity rules (the deal map,
    creation-ordinal targets, representative-uid fallback, adjacent
    one-card play choices, `_selection` for every PlayerChoice), but a
    divergence is returned by name instead of raised as a refusal, and every
    completed-action checkpoint the capture carries is compared as the line
    goes (`rust_replay.NativeChecks`). No outcome from the `.run` is needed:
    the census certifies the replay against the capture alone.

    Returns the wire actions, Rust's own `differential_digest` after each,
    the terminal state, and the checkpoint verdict. Raises `LineDiverged`
    when an input cannot be reproduced; the checkpoint verdict never raises,
    so a line can be exact while its state disagrees with the game.
    """
    events = replay.get("events")
    if not isinstance(events, list) or not events \
            or len(events) > RECORDED_INPUT_BUDGET:
        raise LineDiverged(0, _turn(root), "recorded_input_budget",
                           "recorded input count outside the replay budget")
    try:
        uid_map = _deal_uid_map(root, replay)
    except Exception as exc:  # noqa: BLE001 - every deal failure is one class
        raise LineDiverged(0, _turn(root), "entry_deal",
                           f"{type(exc).__name__}: {exc}"[:200]) from exc
    loaded = session.load(root)
    if "ok" not in loaded:
        raise LineDiverged(0, _turn(root), "root_refused",
                           json.dumps(loaded.get("refusal"))[:200])

    native: Optional[_CensusChecks] = _CensusChecks(replay)
    checkpoint: Dict[str, Any] = {"validated": 0, "mismatch": None,
                                  "powers": None}
    actions: List[Dict[str, Any]] = []
    digests: List[str] = []
    nonrepresentative = 0
    consumed: set = set()
    before = root
    # (step, compared against the post-apply state) of the last checkpoint
    # comparison, for the truncated-capture rule (#3277).
    last_compare: Optional[Tuple[int, bool]] = None

    def mismatch(exc: Exception) -> None:
        nonlocal native
        checkpoint["validated"] = native.validated
        checkpoint["mismatch"] = {"step": len(actions),
                                  "detail": f"{exc}"[:220]}
        native = None

    def note_powers(checks: _CensusChecks) -> None:
        powers = getattr(checks, "power_mismatch", None)
        if checkpoint["powers"] is None and powers is not None:
            checkpoint["powers"] = {"step": len(actions),
                                    "mismatches": powers}

    def diverged(check: str, detail: str) -> LineDiverged:
        # Carry any checkpoint the line already got wrong (#3025).
        validated = (native.validated if native is not None
                     else checkpoint["validated"])
        return LineDiverged(len(actions), _turn(before), check, detail[:220],
                            checkpoint_mismatch=checkpoint["mismatch"],
                            native_checkpoints=validated,
                            power_mismatch=checkpoint["powers"])

    for index, event in enumerate(events):
        if index in consumed:
            continue
        action = event.get("action", {}) or {}
        kind = action.get("type")
        if before["player"].get("over"):
            if kind in _RECORDED_INPUTS:
                raise diverged("premature_combat_end", kind)
            continue
        if native is not None:
            native.start(event)
        if kind in ("NetPlayCardAction", "NetUsePotionAction"):
            if kind == "NetPlayCardAction":
                try:
                    uid = uid_map(action["combat_card_index"])
                except (KeyError, ValueError) as exc:
                    raise diverged("card_identity", str(exc)) from exc
                card = next((c for c in before["piles"].get("hand", [])
                             if c["uid"] == uid), None)
                if card is None or card["id"] != str(
                        action.get("card_id", "")).removeprefix("CARD."):
                    raise diverged(
                        "card_identity",
                        f"recorded card identity mismatch at input {index}")
                wire: Dict[str, Any] = {"kind": "play", "uid": uid}
            else:
                wire = {"kind": "potion", "slot": action.get("potion_index")}
            tid = action.get("target_id")
            if tid and action.get("target_player_id") is None:
                targets = [i for i, m in enumerate(before["monsters"])
                           if m.get("uid", 0) == tid - 1]
                if len(targets) != 1:
                    raise diverged("target_identity",
                                   f"recorded target {tid} at input {index}")
                wire["target"] = targets[0]
            legal = session.ask({"cmd": "legal"})["actions"]
            if kind == "NetPlayCardAction":
                offered = rust_replay._play_is_offered(before, legal, wire)
            else:
                offered = wire in legal
            if not offered and kind == "NetPlayCardAction":
                # An immediate one-card choice belongs to the play wire
                # (Burning Pact); consume only its adjacent recorded choice.
                following = events[index + 1] if index + 1 < len(events) else {}
                selected = (following.get("result") or {}).get(
                    "combat_card_indexes", [])
                if following.get("event_type") == "PlayerChoice" \
                        and isinstance(selected, list) and len(selected) == 1:
                    try:
                        combined = dict(wire, selection=uid_map(selected[0]))
                    except ValueError as exc:
                        raise diverged("card_identity", str(exc)) from exc
                    if rust_replay._play_is_offered(before, legal, combined):
                        wire = combined
                        consumed.add(index + 1)
                        offered = True
            if not offered:
                raise diverged("not_legal",
                               f"{kind} is not legal at input {index}")
            if kind == "NetPlayCardAction" and wire not in legal:
                nonrepresentative += 1
        elif kind == "NetEndPlayerTurnAction":
            if action.get("turn_number") != _turn(before):
                raise diverged(
                    "turn_number",
                    f"recorded turn {action.get('turn_number')} at engine "
                    f"turn {_turn(before)}")
            wire = {"kind": "end"}
        elif event.get("event_type") == "PlayerChoice":
            result = event.get("result") or {}
            indexes = result.get("combat_card_indexes")
            restoring = _RestoringSession(session, root, actions, before)
            try:
                if result.get("type") == "CombatCard" \
                        and isinstance(indexes, list) and len(indexes) > 1:
                    wire = _multi_card_selection(
                        restoring, before, [uid_map(i) for i in indexes])
                else:
                    wire = rust_replay._selection(
                        restoring, before, result, uid_map)
            except ValueError as exc:
                raise diverged("selection", str(exc)) from exc
        elif event.get("event_type") in _NO_DECISION_EVENTS \
                or kind == "NetReadyToBeginEnemyTurnAction":
            # A HookAction may name the AutoPost dispatch checkpoint the
            # last apply ran through (#3242, `NativeChecks.no_decision`).
            if native is not None:
                checks = native
                cursor = getattr(checks, "cursor", None)
                try:
                    checks.no_decision(event)
                except _CHECKPOINT_ERRORS as exc:
                    mismatch(exc)
                if getattr(checks, "cursor", None) != cursor:
                    # Compared against a STAGED mid-apply state (#3242).
                    last_compare = (len(actions), False)
                note_powers(checks)
            continue
        else:
            raise diverged("unsupported_event",
                           f"{event.get('event_type')}/{kind} at input {index}")
        try:
            applied = session.ask({"cmd": "apply", "action": wire,
                                   "native_checkpoints": True})
        except ValueError as exc:
            raise diverged("rust_apply_refused", str(exc)) from exc
        before = session.ask({"cmd": "project"})["state"]
        actions.append(_wire_in_fixture_order(wire))
        digests.append(applied["digest"])
        if native is not None:
            # The apply may have run past a native checkpoint (#3242): the
            # wire's own checkpoint is then compared at that boundary.
            checks = native
            cursor = getattr(checks, "cursor", None)
            played = None
            try:
                played = checks.stage(applied)
                if wire["kind"] != "end":
                    checks.completed(played if played is not None else before)
            except _CHECKPOINT_ERRORS as exc:
                mismatch(exc)
            if getattr(checks, "cursor", None) != cursor:
                # Which step's checkpoint was compared, and whether against
                # the state after the whole apply (not a staged one).
                last_compare = (len(actions), played is None)
            note_powers(checks)
    if not before["player"].get("over"):
        # #3277: a capture archived mid-fight, proved by its own evidence.
        shape = None if native is None else _capture_ends_mid_combat(
            replay, native, before, last_compare == (len(actions), True))
        if shape is not None:
            truncated = CaptureTruncated(
                len(actions), _turn(before), "capture_truncated",
                f"capture ends mid-fight ({shape}): its final checkpoint "
                "shows a living enemy and player",
                checkpoint_mismatch=None, native_checkpoints=native.validated,
                power_mismatch=checkpoint["powers"])
            truncated.shape = shape
            raise truncated
        raise diverged("combat_not_complete",
                       "recorded inputs ended before the engine's combat")
    if native is not None:
        try:
            native.finish()
            checkpoint["validated"] = native.validated
        except ValueError as exc:
            mismatch(exc)
    return {
        "actions": actions,
        "step_digests": digests,
        "terminal": _terminal(before),
        "nonrepresentative_uids": nonrepresentative,
        "native_checkpoints": checkpoint["validated"],
        "checkpoint_mismatch": checkpoint["mismatch"],
        "power_mismatch": checkpoint["powers"],
    }


# ---------------------------------------------------------------------------
# Corpus discovery (capture pairing), stdlib only
#
# Verbatim from the simulator-importing modules they used to be read from
# (`tools/mcr_validate.py` capture scanning, `tools/live_coach.py` node
# helpers), so the census pairs exactly the fights it paired before #2999.
# ---------------------------------------------------------------------------

SNAKE_TO_STREAM = {snake_case(n): n for n in RUN_RNG_STREAMS}


def global_node_index(data: dict) -> int:
    """Node index in the .run's GLOBAL numbering for a save/.mcr snapshot.

    visited_map_coords resets at each act transition, so its length only
    gives the PER-ACT index. Prior acts' node lists stay behind in
    map_point_history (one completed list per finished act), so the
    global index is their summed length plus the per-act index. Act-0
    snapshots need no offset (distilled fixtures may omit the
    history)."""
    act = data.get("current_act_index") or 0
    prior = sum(len(a) for a in data["map_point_history"][:act]) if act else 0
    return prior + len(data["visited_map_coords"]) - 1


def rng_counters(rng_doc: dict) -> dict:
    """Per-stream counters from either save schema: < 19 stores
    rng.counters = {stream: int}; >= 19 stores rng.rngs =
    {stream: {counter, s0..s3}}."""
    if "counters" in rng_doc:
        return dict(rng_doc["counters"])
    return {k: v["counter"] for k, v in rng_doc["rngs"].items()}


def scan_saves(captures: pathlib.Path) -> list:
    """All parseable captured saves, in capture order (filename sort)."""
    out = []
    for path in sorted(captures.glob("*/*_save_*.save")):
        try:
            data = json.loads(path.read_text())
        except ValueError:
            continue                     # partial write caught mid-copy
        try:
            out.append({
                "path": path,
                "seed": data["rng"]["seed"],
                "start_time": data["start_time"],
                "node_index": global_node_index(data),
                "counters": {SNAKE_TO_STREAM.get(k, k): v
                             for k, v in rng_counters(data["rng"]).items()},
                # An imported upload's run (#2915) keeps the capture's own
                # `player_rng` spelling; `rng_counters` reads either shape.
                "player_counters": (rng_counters(
                    data["players"][0].get("rng")
                    or data["players"][0]["player_rng"])
                                    if data.get("players") else {}),
            })
        except (KeyError, IndexError):
            continue
    return out


#: A save's act map keeps its boss nodes OUTSIDE `points` (#3426): the act
#: boss under `saved_map.boss`, act 3's second boss under
#: `saved_map.second_boss`. Each names its encounter in its own `rooms` slot.
#: Verified over ~/sts2-captures on 2026-09-27: 212 saves sit at `boss` and
#: 19 at `second_boss`, and each of the 103 that a later save resolves
#: (`pre_finished_room.encounter_id`) agrees with this pairing.
BOSS_MAP_ROOMS = (("boss", "boss_id"), ("second_boss", "second_boss_id"))


def boss_room_slot(save) -> Optional[str]:
    """The `rooms` slot naming the current node's boss, or None off a boss."""
    m = save["acts"][save["current_act_index"]].get("saved_map")
    cur = save["visited_map_coords"][-1]
    for map_key, room_key in BOSS_MAP_ROOMS:
        point = (m or {}).get(map_key)
        if isinstance(point, dict) and point.get("coord") == cur:
            return room_key
    return None


def node_type(save):
    """Map point type of the current node, from the saved act map."""
    m = save["acts"][save["current_act_index"]].get("saved_map")
    cur = save["visited_map_coords"][-1]
    if not m:
        return None
    for pt in m["points"]:
        if pt["coord"] == cur:
            t = str(pt.get("point_type") or pt.get("type") or "")
            return t.lower()
    if boss_room_slot(save):
        return "boss"
    return None


def next_encounter(save, kind):
    r = save["acts"][save["current_act_index"]]["rooms"]
    if kind == "elite":
        return r["elite_encounter_ids"][r["elite_encounters_visited"]]
    if kind == "boss":
        slot = boss_room_slot(save)
        if slot is None:
            raise EvalRefusal("boss node is neither saved_map.boss nor "
                              "saved_map.second_boss")
        return r[slot]
    return r["normal_encounter_ids"][r["normal_encounters_visited"]]


def discover_fights(captures: pathlib.Path) -> Dict[str, Any]:
    """Every unique v0.111.0 fight in the corpus, paired with its entry save.

    A watcher copies `latest.mcr` on every write, so a fight appears many
    times; the LAST snapshot per (seed, start_time, global node) is the
    complete one. The FIRST save at that key is the node-entry save — later
    saves at the same node are mid-combat and break the deal map.
    """
    saves = scan_saves(captures)
    first_save: Dict[Tuple[Any, Any, int], pathlib.Path] = {}
    run_saves: Dict[Tuple[Any, Any], List[Dict[str, Any]]] = {}
    for save in saves:
        key = (save["seed"], save["start_time"], save["node_index"])
        first_save.setdefault(key, save["path"])
        run_saves.setdefault((save["seed"], save["start_time"]), []).append(save)

    fights: Dict[Tuple[Any, Any, int], Tuple[pathlib.Path, Dict[str, Any]]] = {}
    skipped: Dict[str, int] = {}
    for path in sorted(captures.glob("*/*_mcr_*.mcr")):
        try:
            replay = mcr_parser.decode(path)
        except Exception as exc:  # noqa: BLE001 - a corpus census reports these
            reason = f"{type(exc).__name__}: {str(exc)[:60]}"
            skipped[reason] = skipped.get(reason, 0) + 1
            continue
        if replay["version"] != BUILD_ID:
            reason = f"build {replay['version']}"
            skipped[reason] = skipped.get(reason, 0) + 1
            continue
        run = replay["run"]
        key = (run["rng"]["seed"], run["start_time"], global_node_index(run))
        fights[key] = (path, replay)
    return {"fights": fights, "first_save": first_save,
            "run_saves": run_saves, "skipped": skipped, "saves": len(saves)}


def upload_sidecar_path(save_path: pathlib.Path) -> pathlib.Path:
    """The sidecar `import-uploads` writes beside an upload's capture run."""
    return save_path.with_name(
        save_path.name.replace("_save_", "_upload_", 1)).with_suffix(".json")


def entry_input(save_path: pathlib.Path) -> str:
    """`sts-sim entry` input flag for one paired save (#2915).

    A game save is `--save`. An imported upload's "save" is the capture's own
    embedded run with the uploader's unlock state, which is exactly what prod
    reviews root with `--capture-run` (`rust_review.replay_capture_run`).
    """
    return "--capture-run" if upload_sidecar_path(save_path).is_file() else "--save"


def derive_encounter(save: Dict[str, Any], key, spath: pathlib.Path,
                     run_saves) -> Tuple[Optional[str], Optional[str]]:
    """(encounter id, node kind) for a fight's entry save.

    Combat nodes carry their encounter in the act's room lists. Event-room
    fights report map node type `unknown`; their encounter is recorded by the
    NEXT save at the same global node, under `pre_finished_room`.

    An imported upload (#2915) names its encounter in a sidecar instead: its
    capture run's map does not carry the room kind, and the `.run` the import
    resolved it against already names the fight.
    """
    sidecar = upload_sidecar_path(spath)
    if sidecar.is_file():
        recorded = json.loads(sidecar.read_text())
        return recorded["encounter"], recorded["node_type"]
    kind = node_type(save)
    if kind in ("monster", "elite", "boss"):
        return next_encounter(save, kind), kind
    for later in run_saves.get((key[0], key[1]), []):
        if str(later["path"]) <= str(spath) or later["node_index"] != key[2]:
            continue
        room = json.loads(later["path"].read_text()).get("pre_finished_room")
        if isinstance(room, dict) and room.get("encounter_id"):
            return room["encounter_id"], room.get("room_type") or "monster"
    return None, kind


# ---------------------------------------------------------------------------
# One census row
# ---------------------------------------------------------------------------


#: Whether a player-power disagreement (#3029) fails certification. OFF:
#: the census reports it (`power_check`, `player_powers`) beside an
#: unchanged `lockstep` verdict until the drift it finds is triaged; the
#: `--powers-verdict` flag turns it on for a run.
POWERS_VERDICT_DEFAULT = False


def census_row(key, capture: pathlib.Path, replay: Dict[str, Any],
               state: Dict[str, Any], session: EngineSession,
               binary: pathlib.Path,
               encounter_override: Optional[Tuple[str, str]] = None,
               powers_verdict: bool = POWERS_VERDICT_DEFAULT) -> Dict[str, Any]:
    """One fight, walked from capture to certification verdict.

    Stages, in pipeline order: capture pairing (`unpaired`, `no_encounter`),
    Rust's entry (`entry_refused`, `entry_error`), Rust's opening
    (`opening_refused`), then `rooted`. A rooted fight is compared with the
    capture's opening checkpoint, loaded (`rust`: admitted/refused), and —
    when admitted — its recorded human line is replayed (`human`:
    exact/diverged, or `truncated` for a capture archived mid-fight, #3277)
    and certified against every completed-action checkpoint
    (`lockstep`: `lockstep_ok`, `checkpoint_mismatch`, or
    `no_native_checkpoints` for a capture that predates them).
    """
    run = replay["run"]
    player = run["players"][0]
    row: Dict[str, Any] = {
        "id": fixture_id(key[0], key[1], key[2]),
        "seed": key[0],
        "node": key[2],
        "character": player.get("character") or player.get("character_id"),
        "ascension": run.get("ascension"),
        "deck": len(player["deck"]),
        "potions": sum(1 for p in player.get("potions", []) if p.get("id")),
        "relics": len(player.get("relics", [])),
        "capture_sha256": sha256_file(capture),
    }
    spath = state["first_save"].get(key)
    if spath is None:
        row["stage"] = "unpaired"
        return with_unusable_capture(row, replay)
    row["save_sha256"] = sha256_file(spath)
    save = json.loads(spath.read_text())
    try:
        inferred = derive_encounter(save, key, spath, state["run_saves"])
        if encounter_override and inferred[0] and inferred != encounter_override:
            raise EvalRefusal(f"explicit encounter {encounter_override} conflicts with {inferred}")
        encounter, kind = encounter_override or inferred
        if encounter_override:
            row["encounter_source"] = "explicit_capture_pair"
    except Exception as exc:  # noqa: BLE001
        row.update(stage="no_encounter", detail=f"{type(exc).__name__}: {exc}"[:160])
        return with_unusable_capture(row, replay)
    row["node_type"] = kind
    if not encounter:
        row["stage"] = "no_encounter"
        return with_unusable_capture(row, replay)
    row["encounter"] = encounter

    try:
        document, recorded = split_opening_checkpoints(rust_opening_document(
            binary, spath, encounter, kind, replay["version"],
            source=entry_input(spath), native_checkpoints=True))
    except RustEntryCliError as exc:
        row.update(stage="entry_error", detail=str(exc)[:220],
                   refusal_class="rust_entry_cli_error")
        return row
    if "refusal" in document:
        row.update(stage="entry_refused",
                   detail=(document["refusal"].get("detail") or "")[:220],
                   refusal_class=document.get("refusal_class", "other"))
        return row
    if document.get("schema") != CANONICAL_SCHEMA:
        opening = document.get("opening") or {}
        row.update(stage="opening_refused",
                   detail=(opening.get("detail") or "")[:220],
                   refusal_class=opening.get("refusal_class", "other"))
        return row
    row["stage"] = "rooted"
    row["root_source"] = "rust_opening"
    row.update(opening_checkpoint(document, replay, recorded))
    document, row["reset_order"] = with_native_reset_order(document, replay)
    row["entry_digest"] = canonical_document.differential_digest(document)
    row["_document"] = document

    response = session.load(document)
    if "ok" in response:
        row["rust"] = "admitted"
        row["entry_digest_match"] = response["ok"]["digest"] == row["entry_digest"]
    else:
        refusal = response.get("refusal", {})
        detail = refusal.get("detail") or ""
        # The FULL blocker set, not just the first one: an admission refusal
        # carries every missing capability it found, while a boundary
        # (`unrepresentable_state`) refusal names exactly one field by
        # construction.
        missing = list(response.get("missing", []))
        row.update(rust="refused", rust_kind=refusal.get("kind"),
                   rust_detail=detail[:220], rust_missing=missing,
                   rust_class=rust_refusal_class(refusal, missing))
        return row

    try:
        line = replay_recorded_line(session, document, replay)
    except CaptureTruncated as exc:
        row.update(truncated_row_fields(row, exc))
        row.update(power_check_fields(row, exc.power_mismatch))
        return row
    except LineDiverged as exc:
        row.update(human="diverged",
                   human_detail=f"t{exc.turn} {exc.check}: {exc.detail}"[:220],
                   human_class=f"diverged:{exc.check}",
                   human_step=exc.step)
        row.update(divergence_checkpoint_verdict(row, exc))
        row.update(power_check_fields(row, exc.power_mismatch))
        return row
    except Exception as exc:  # noqa: BLE001 - a transport fault is reported
        row.update(human="error",
                   human_detail=f"{type(exc).__name__}: {exc}"[:220],
                   human_class=f"error:{type(exc).__name__}")
        return row
    terminal = line["terminal"]
    row.update(human="exact", turns=terminal["turn"],
               action_count=len(line["actions"]), hp=terminal["hp"],
               over=terminal["over"], won=terminal["won"],
               nonrepresentative_uids=line["nonrepresentative_uids"],
               native_checkpoints=line["native_checkpoints"])
    row["_line"] = line

    row.update(power_check_fields(row, line["power_mismatch"]))
    row.update(certification_verdict(row, line, powers_verdict))
    return row


#: Why a capture can never be certified, whatever the engine does (#3421).
#: The census headline is certified / eligible, and eligible is every fight
#: less these. A reason is set only on positive evidence from the capture
#: itself. A real combat the harness fails to pair or name an encounter
#: for stays eligible, so the failure shows in the headline.
UNUSABLE_NO_COMBAT = "no_combat_in_capture"
UNUSABLE_TRUNCATED = "capture_truncated"


def capture_holds_no_combat(replay: Dict[str, Any]) -> bool:
    """True when the capture recorded no combat at all: no event, no checksum.

    A combat capture always carries both: the recorded actions, and the
    native checksum every completed action writes. A capture with neither
    is a watcher copy taken at a node that held no fight (#3421).
    """
    return not replay.get("events") and not replay.get("checksums")


def with_unusable_capture(row: Dict[str, Any],
                          replay: Dict[str, Any]) -> Dict[str, Any]:
    """A capture-pairing row, marked unusable when its capture holds no combat.

    Only `unpaired` and `no_encounter` rows come here. A pairing failure
    whose capture does hold a combat is left unmarked: it is a harness gap,
    and it stays in the eligible count (#3421).
    """
    if capture_holds_no_combat(replay):
        row["unusable"] = UNUSABLE_NO_COMBAT
    return row


def unusable_reason(row: Dict[str, Any]) -> Optional[str]:
    """Why this fight is out of the eligible count, or None when it is in.

    A truncated capture (#3277) is real but incomplete, so it can never
    certify. A row with an `unusable` mark holds no combat.
    """
    if row.get("human") == "truncated":
        return UNUSABLE_TRUNCATED
    return row.get("unusable")


def truncated_row_fields(row: Dict[str, Any],
                         exc: CaptureTruncated) -> Dict[str, Any]:
    """The census fields of a capture that stops mid-fight (#3277).

    `human = truncated`, `human_class = capture_truncated`: its own class,
    out of the engine-divergence bucket, and never a `lockstep` verdict — a
    partial line is not a certified fight. `prefix_lockstep` says what the
    checkpoints the capture does carry said: `prefix_agrees` when the
    opening and every completed-action checkpoint agreed (the truncation
    rule requires the latter), `checkpoint_mismatch` when the opening did
    not, `no_native_checkpoints` never (the rule needs a checkpoint).
    """
    out: Dict[str, Any] = {
        "human": "truncated", "human_class": "capture_truncated",
        "human_detail": f"t{exc.turn} {exc.check}: {exc.detail}"[:220],
        "human_step": exc.step,
        "truncation_shape": exc.shape,
        "native_checkpoints_before_truncation": exc.native_checkpoints,
    }
    if row.get("opening_checkpoint") != "match":
        out.update(prefix_lockstep="checkpoint_mismatch",
                   prefix_mismatch_step=0,
                   prefix_mismatch_detail=row.get("opening_checkpoint_detail"))
    else:
        out["prefix_lockstep"] = "prefix_agrees"
    return out


def power_check_fields(row: Dict[str, Any],
                       power_mismatch: Optional[Dict[str, Any]]
                       ) -> Dict[str, Any]:
    """The player-power comparison of a replayed line, as typed row fields.

    `power_check` is `match` when every compared checkpoint's hero powers
    agree with Rust's player, `mismatch` at the first that does not, and
    `no_native_checkpoints` for a capture without them. On a mismatch
    `power_mismatch_step` is that checkpoint's step (the opening is 0),
    `player_powers` every disagreeing row there (`{"class", "power",
    "native", "rust"}`), and `power_mismatch_class` / `power_mismatch_power`
    the first row's, for tallies. For a diverged line this covers the
    checkpoints before the divergence.
    """
    if not row.get("checksummed"):
        return {"power_check": "no_native_checkpoints"}
    if row.get("opening_power_mismatches"):
        step, rows = 0, row["opening_power_mismatches"]
    elif power_mismatch is not None:
        step, rows = power_mismatch["step"], power_mismatch["mismatches"]
    else:
        return {"power_check": "match"}
    return {"power_check": "mismatch", "power_mismatch_step": step,
            "power_mismatch_class": rows[0]["class"],
            "power_mismatch_power": rows[0]["power"],
            "player_powers": rows}


def _power_detail(row: Dict[str, Any]) -> str:
    first = row["player_powers"][0]
    if first["class"] in ("amount", "instances"):
        return (f"recorded replay differs from native player powers: "
                f"{first['power']} native {first['native']} "
                f"rust {first['rust']}")
    return (f"recorded replay cannot compare native player power "
            f"{first['power']} ({first['class']})")


def certification_verdict(row: Dict[str, Any],
                          line: Dict[str, Any],
                          powers_verdict: bool = POWERS_VERDICT_DEFAULT
                          ) -> Dict[str, Any]:
    """The lockstep fields for an exactly replayed line.

    Certified (`lockstep_ok`) means the opening AND every completed-action
    checkpoint agree with the game. A capture without checkpoints — or whose
    checksums hold no completed-action state, so nothing was compared — is
    measured and named `no_native_checkpoints`, never certified vacuously.
    With `powers_verdict`, a player-power disagreement (`row["power_check"]`)
    is a checkpoint mismatch too, at whichever step disagreed first.
    """
    if not row.get("checksummed"):
        return {"lockstep": "no_native_checkpoints", "lockstep_step": 0}
    power_step = (row.get("power_mismatch_step")
                  if powers_verdict and row.get("power_check") == "mismatch"
                  else None)
    if row.get("opening_checkpoint") != "match":
        return {"lockstep": "checkpoint_mismatch", "lockstep_step": 0,
                "lockstep_detail": row.get("opening_checkpoint_detail")}
    if power_step is not None and (
            line["checkpoint_mismatch"] is None
            or power_step < line["checkpoint_mismatch"]["step"]):
        return {"lockstep": "checkpoint_mismatch",
                "lockstep_step": power_step,
                "lockstep_detail": _power_detail(row)}
    if line["checkpoint_mismatch"] is not None:
        return {"lockstep": "checkpoint_mismatch",
                "lockstep_step": line["checkpoint_mismatch"]["step"],
                "lockstep_detail": line["checkpoint_mismatch"]["detail"]}
    if not line["native_checkpoints"]:
        return {"lockstep": "no_native_checkpoints", "lockstep_step": 0}
    return {"lockstep": "lockstep_ok", "lockstep_step": len(line["actions"])}


def checkpoint_mismatch_field(detail: Optional[str]) -> str:
    """The state field a checkpoint mismatch names, from its message.

    `rust_replay.check_native_snapshot` raises `recorded replay differs from
    native <field>` (`player powers: ...` names `player_powers`; a power it
    cannot compare is `power_not_projected` or
    `power_instances_not_projected`, #3029); anything else (a missing or misattributed checkpoint,
    a pet, an unmodeled RNG row) is `other`, never guessed.
    """
    text = detail or ""
    if "cannot compare native player power" in text:
        return ("power_instances_not_projected"
                if "power_instances_not_projected" in text
                else "power_not_projected")
    marker = "differs from native "
    if marker in text:
        return (text.split(marker, 1)[1].split(":", 1)[0].strip()
                .replace(" ", "_") or "other")
    if "checkpoint action identity" in text:
        return "action_identity"
    if "checkpoint missing" in text:
        return "checkpoint_missing"
    return "other"


def divergence_checkpoint_verdict(row: Dict[str, Any],
                                  exc: LineDiverged) -> Dict[str, Any]:
    """What the native checkpoints said about a line BEFORE it diverged (#3025).

    New fields beside the divergence, never a `lockstep` verdict: that name
    stays the certification of a completed line. `pre_divergence_lockstep`
    is `checkpoint_mismatch` when the opening or any completed-action
    checkpoint before the divergence disagreed with the game — step, field
    and detail of the first one — `no_mismatch_before_divergence` when every
    checkpoint reached agreed, and `no_native_checkpoints` for a capture
    without them. The same precedence as `certification_verdict`: the
    opening checkpoint is step 0.
    """
    if not row.get("checksummed"):
        return {"pre_divergence_lockstep": "no_native_checkpoints"}
    out: Dict[str, Any] = {
        "native_checkpoints_before_divergence": exc.native_checkpoints}
    if row.get("opening_checkpoint") != "match":
        step, detail = 0, row.get("opening_checkpoint_detail")
    elif exc.checkpoint_mismatch is not None:
        step = exc.checkpoint_mismatch["step"]
        detail = exc.checkpoint_mismatch["detail"]
    else:
        out["pre_divergence_lockstep"] = "no_mismatch_before_divergence"
        return out
    out.update(pre_divergence_lockstep="checkpoint_mismatch",
               pre_divergence_mismatch_step=step,
               pre_divergence_mismatch_field=checkpoint_mismatch_field(detail),
               pre_divergence_mismatch_detail=detail)
    return out


def run_census(captures: pathlib.Path, binary: pathlib.Path,
               progress: bool = False,
               powers_verdict: bool = POWERS_VERDICT_DEFAULT) -> Dict[str, Any]:
    """Walk the whole corpus and return rows plus an aggregate summary."""
    state = discover_fights(captures)
    session = EngineSession(binary)
    rows: List[Dict[str, Any]] = []
    started = time.time()
    try:
        for index, (key, (capture, replay)) in enumerate(
                sorted(state["fights"].items())):
            rows.append(census_row(key, capture, replay, state, session,
                                   binary, powers_verdict=powers_verdict))
            if progress and (index + 1) % 50 == 0:
                print(f"  {index + 1}/{len(state['fights'])} "
                      f"({time.time() - started:.0f}s)", file=sys.stderr,
                      flush=True)
    finally:
        session.close()
    return {
        "captures": str(captures),
        "build": BUILD_ID,
        "root_with": "rust_opening",
        "powers_verdict": powers_verdict,
        "saves_scanned": state["saves"],
        "captures_skipped": state["skipped"],
        "seconds": round(time.time() - started, 1),
        "rows": rows,
        "summary": census_summary(rows),
    }


def _tally(rows: List[Dict[str, Any]], key: str) -> Dict[str, int]:
    counts: Dict[str, int] = {}
    for row in rows:
        value = row.get(key)
        if value is None:
            continue
        counts[str(value)] = counts.get(str(value), 0) + 1
    return dict(sorted(counts.items(), key=lambda item: (-item[1], item[0])))


def census_summary(rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Aggregate one census."""
    rooted = [r for r in rows if r.get("stage") == "rooted"]
    admitted = [r for r in rooted if r.get("rust") == "admitted"]
    exact = [r for r in admitted if r.get("human") == "exact"]
    certified = [r for r in admitted if r.get("lockstep") == "lockstep_ok"]
    checksummed = [r for r in rooted if r.get("checksummed")]
    missing: Dict[str, int] = {}
    for row in rooted:
        for item in row.get("rust_missing", []) or []:
            missing[item] = missing.get(item, 0) + 1
    drifting: Dict[str, int] = {}
    for row in checksummed:
        for field in row.get("opening_counter_drift", []) or []:
            drifting[field] = drifting.get(field, 0) + 1
    excluded = _tally([{"reason": unusable_reason(r)} for r in rows], "reason")
    return {
        "fights": len(rows),
        # #3421: the headline is certified / eligible. Eligible is every
        # fight less the captures that can never certify, whose reasons are
        # counted in `excluded_unusable`.
        "certified": len(certified),
        "eligible": len(rows) - sum(excluded.values()),
        "excluded_unusable": excluded,
        "stages": _tally(rows, "stage"),
        "entry_refusal_classes": _tally(
            [r for r in rows if r.get("stage") in
             ("entry_refused", "entry_error")], "refusal_class"),
        "opening_refusal_classes": _tally(
            [r for r in rows if r.get("stage") == "opening_refused"],
            "refusal_class"),
        "rooted": len(rooted),
        "opening_checkpoints": _tally(rooted, "opening_checkpoint"),
        # I6: the unspliced opening's consumed counters against the capture's.
        "opening_counters_checked": len(checksummed),
        "opening_counters_match": sum(
            1 for r in checksummed if r.get("opening_counters_match")),
        "opening_counter_drift_streams": dict(
            sorted(drifting.items(), key=lambda item: (-item[1], item[0]))),
        "rust_admitted": len(admitted),
        "rust_refusal_classes": _tally(rooted, "rust_class"),
        "rust_missing_capabilities": dict(
            sorted(missing.items(), key=lambda item: (-item[1], item[0]))),
        "human": _tally(admitted, "human"),
        # A truncated capture (#3277) is counted apart, never as a divergence.
        "human_divergence_classes": _tally(
            [r for r in admitted if r.get("human") != "truncated"],
            "human_class"),
        "human_exact_characters": _tally(exact, "character"),
        "lockstep": _tally(admitted, "lockstep"),
        # #3025: a diverged line's checkpoints before the divergence.
        "lockstep_before_divergence": _tally(
            [r for r in admitted if r.get("human") == "diverged"],
            "pre_divergence_lockstep"),
        # #3277: captures archived mid-fight, out of the divergence bucket.
        "capture_truncated": sum(
            1 for r in admitted if r.get("human") == "truncated"),
        "capture_truncated_shapes": _tally(
            [r for r in admitted if r.get("human") == "truncated"],
            "truncation_shape"),
        "lockstep_before_truncation": _tally(
            [r for r in admitted if r.get("human") == "truncated"],
            "prefix_lockstep"),
        "checkpoint_mismatch_before_divergence_fields": _tally(
            [r for r in admitted
             if r.get("pre_divergence_lockstep") == "checkpoint_mismatch"],
            "pre_divergence_mismatch_field"),
        # Every fight a native checkpoint proves wrong, certifiable or not.
        "checkpoint_mismatch_known": sum(
            1 for r in admitted
            if r.get("lockstep") == "checkpoint_mismatch"
            or r.get("pre_divergence_lockstep") == "checkpoint_mismatch"),
        "native_checkpoints_validated": sum(
            r.get("native_checkpoints", 0) for r in exact),
        "lockstep_characters": _tally(certified, "character"),
        "lockstep_encounters": _tally(certified, "encounter"),
        # #3029: the player-power comparison, reported beside `lockstep`.
        "power_check": _tally(
            [r for r in admitted if r.get("human") == "exact"], "power_check"),
        "power_check_before_divergence": _tally(
            [r for r in admitted if r.get("human") == "diverged"],
            "power_check"),
        "power_mismatch_classes": _tally(admitted, "power_mismatch_class"),
        "power_mismatch_powers": _tally(admitted, "power_mismatch_power"),
        "lockstep_ok_with_power_mismatch": sum(
            1 for r in certified if r.get("power_check") == "mismatch"),
    }


def census_headline(summary: Dict[str, Any]) -> str:
    """`**certified / eligible: C / E** (P%)`, the census headline (#3421)."""
    certified = summary.get("certified", 0)
    eligible = summary.get("eligible", 0)
    share = f" ({100 * certified / eligible:.1f}%)" if eligible else ""
    return f"**certified / eligible: {certified} / {eligible}**{share}"


def census_excluded_line(summary: Dict[str, Any]) -> str:
    """The unusable captures left out of the eligible count, by reason."""
    excluded = summary.get("excluded_unusable") or {}
    reasons = ", ".join(f"`{name}` {count}"
                        for name, count in excluded.items())
    return (f"excluded as unusable captures: {sum(excluded.values())}"
            + (f" ({reasons})" if reasons else ""))


def census_markdown(census: Dict[str, Any]) -> str:
    """The standing "done per encounter" report, as a pasteable table."""
    summary = census["summary"]
    stages = summary["stages"]
    lines: List[str] = []
    lines.append(f"### Real-fight census — `{census['captures']}`, "
                 f"build {census['build']}")
    lines.append("")
    lines.append("Every root is Rust's own opening (`sts-sim entry --save "
                 "--opening`), unspliced; every recorded line is replayed by "
                 "Rust and certified against the capture's native "
                 "checkpoints (#1282, #2999).")
    lines.append("")
    lines.append(census_headline(summary))
    lines.append("")
    lines.append(census_excluded_line(summary))
    lines.append("")
    lines.append("| stage | fights |")
    lines.append("|---|---:|")
    lines.append(f"| unique {census['build']} fights | {summary['fights']} |")
    lines.append(f"| no encounter derivable / unpaired | "
                 f"{stages.get('no_encounter', 0) + stages.get('unpaired', 0)} |")
    lines.append(f"| Rust entry refusals | "
                 f"{stages.get('entry_refused', 0) + stages.get('entry_error', 0)} |")
    lines.append(f"| Rust opening refusals | "
                 f"{stages.get('opening_refused', 0)} |")
    lines.append(f"| Rust builds the root | {summary['rooted']} |")
    lines.append(f"| opening matches the capture's first checkpoint | "
                 f"{summary['opening_checkpoints'].get('match', 0)} / "
                 f"{summary['opening_counters_checked']} |")
    lines.append(f"| **Rust admits the root** | "
                 f"**{summary['rust_admitted']}** |")
    lines.append(f"| human line replays exactly through Rust | "
                 f"{summary['human'].get('exact', 0)} |")
    lines.append(f"| capture ends mid-fight (truncated, uncertified, "
                 f"#3277) | {summary.get('capture_truncated', 0)} |")
    lines.append(f"| **certified: every native checkpoint agrees** | "
                 f"**{summary['lockstep'].get('lockstep_ok', 0)}** |")
    lines.append(f"| completed-action checkpoints validated | "
                 f"{summary['native_checkpoints_validated']} |")
    lines.append(f"| certified, but player powers disagree (#3029) | "
                 f"{summary['lockstep_ok_with_power_mismatch']} |")
    lines.append(f"| native checkpoint mismatch (exact lines + before a "
                 f"divergence) | "
                 f"{summary['lockstep'].get('checkpoint_mismatch', 0)} + "
                 f"{summary['lockstep_before_divergence'].get('checkpoint_mismatch', 0)}"
                 f" = {summary['checkpoint_mismatch_known']} |")
    lines.append("")

    def table(title: str, counts: Dict[str, int], head: str) -> None:
        if not counts:
            return
        lines.append(f"**{title}**")
        lines.append("")
        lines.append(f"| {head} | fights |")
        lines.append("|---|---:|")
        for name, count in list(counts.items())[:12]:
            lines.append(f"| `{name}` | {count} |")
        lines.append("")

    table("Rust entry refusals", summary["entry_refusal_classes"], "class")
    table("Rust opening refusals", summary["opening_refusal_classes"], "class")
    table("Opening counter drift (unspliced)",
          summary["opening_counter_drift_streams"], "stream")
    table("Rust root refusals", summary["rust_refusal_classes"], "class")
    table("Rust missing capabilities (full blocker set, rooted fights)",
          summary["rust_missing_capabilities"], "missing")
    table("Human-line divergences", summary["human_divergence_classes"],
          "class")
    table("Lockstep verdicts", summary["lockstep"], "verdict")
    table("Checkpoints before a line divergence",
          summary["lockstep_before_divergence"], "verdict")
    table("Checkpoint mismatch before a divergence, by field",
          summary["checkpoint_mismatch_before_divergence_fields"], "field")
    table("Player-power mismatch classes (#3029)",
          summary["power_mismatch_classes"], "class")
    table("Player-power mismatches, first power",
          summary["power_mismatch_powers"], "power")
    table("Certified characters", summary["lockstep_characters"], "character")
    table("Certified encounters", summary["lockstep_encounters"], "encounter")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Fixtures: sim/v0.111.0/eval/
# ---------------------------------------------------------------------------


def categories_for(row: Dict[str, Any], kind: str) -> List[str]:
    """Category tags for one fixture (#2048, amended 2026-09-05).

    `human` — the baseline tag that licenses a "solver vs human" statement —
    goes only on fights whose ascension makes the comparison meaningful. That
    is A9/A10 for every character (Sean, 2026-09-23, #2915; it was A10
    Ironclad only while the other characters' captures were low-ascension).
    Every fixture carries its ascension, so a low-A gap can never be reported
    as a baseline.
    """
    tags = [f"multi:{row['character']}"]
    if row.get("node_type") in ("monster", "elite", "boss"):
        tags.append(f"node:{row['node_type']}")
    if kind == "refusal":
        facet = refusal_facet(row) or {}
        tags.append("refusal")
        if facet.get("surface"):
            tags.append(f"refusal:{facet['surface']}")
        if facet.get("class"):
            tags.append(f"class:{facet['class']}")
    else:
        turns = row.get("turns") or 0
        if turns <= 3:
            tags.append("short")
        if turns >= 5:
            tags.append("long")
        if row.get("potions"):
            tags.append("potions")
        if row.get("won") is False and row.get("over"):
            tags.append("death")
        if (row.get("ascension") or 0) >= SEED_MIN_ASCENSION:
            tags.append("human")
    if row.get("rust") == "admitted":
        tags.append("solvable")
    if row.get("lockstep") == "lockstep_ok":
        tags.append("certified")
    return sorted(set(tags))


#: The refusal surfaces a fixture can name, in pipeline order. The last four
#: `python-*` names are the frozen-oracle surfaces the tree seeded before #2999
#: records; nothing writes them any more.
REFUSAL_SURFACES = (
    "capture-pairing", "rust-entry", "rust-opening", "rust-load",
    "rust-line", "rust-lockstep",
)


def refusal_facet(row: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """The single refusal this fight is a fixture FOR.

    A fight can refuse on more than one surface; the earliest surface in the
    pipeline is the one that stops the fight, so that is the one the fixture
    records — and the census keeps the rest.
    """
    if row.get("stage") in ("no_encounter", "unpaired"):
        return {"surface": "capture-pairing", "class": row["stage"],
                "kind": None,
                "detail": row.get("detail")
                or f"{row['stage']} for map node type {row.get('node_type')!r}",
                "missing": []}
    if row.get("stage") in ("entry_refused", "entry_error"):
        return {"surface": "rust-entry", "class": row.get("refusal_class"),
                "kind": None, "detail": row.get("detail"), "missing": []}
    if row.get("stage") == "opening_refused":
        return {"surface": "rust-opening", "class": row.get("refusal_class"),
                "kind": None, "detail": row.get("detail"), "missing": []}
    if row.get("rust") == "refused":
        return {"surface": "rust-load", "class": row.get("rust_class"),
                "kind": row.get("rust_kind"), "detail": row.get("rust_detail"),
                "missing": row.get("rust_missing") or []}
    if row.get("human") == "truncated":
        # A capture archived mid-fight (#3277) refuses on no engine surface:
        # it is excluded from the seed set, neither a line nor a refusal.
        return None
    if row.get("human") in ("diverged", "error"):
        return {"surface": "rust-line", "class": row.get("human_class"),
                "kind": None, "detail": row.get("human_detail"), "missing": []}
    lockstep = row.get("lockstep")
    if lockstep and lockstep != "lockstep_ok":
        return {"surface": "rust-lockstep", "class": lockstep,
                "kind": None,
                "detail": row.get("lockstep_detail")
                or f"{lockstep} after {row.get('lockstep_step')} actions",
                "missing": []}
    return None


def fixture_documents(row: Dict[str, Any], kind: str) -> Dict[str, Any]:
    """The files one fixture owns, keyed by file name.

    No raw `.mcr` or `.save` bytes are ever written: a fixture carries the
    canonical entry, the sha256 provenance of the pair it came from, and the
    recorded line.
    """
    provenance = {
        "schema": PROVENANCE_SCHEMA,
        "build": BUILD_ID,
        "seed": row["seed"],
        "node": row["node"],
        "encounter": row.get("encounter"),
        "node_type": row.get("node_type"),
        "character": row["character"],
        "ascension": row["ascension"],
        "capture_sha256": row["capture_sha256"],
        "save_sha256": row.get("save_sha256"),
        "entry_digest": row.get("entry_digest"),
        # The capture carries the opening checkpoint the root was compared
        # with (never spliced into it since #2999).
        "checksummed": row.get("checksummed"),
    }
    if row.get("encounter_source"):
        provenance["encounter_source"] = row["encounter_source"]
    files: Dict[str, Any] = {"provenance.json": provenance}
    if row.get("human") == "truncated":
        raise EvalRefusal(
            f"{row['id']}: the capture stops mid-fight ({row.get('human_detail')}); "
            "a truncated capture is neither a line nor a refusal fixture (#3277)")
    if kind == "line":
        if not row.get("_line") or not row.get("_document"):
            raise EvalRefusal(f"{row['id']}: no exact human line to record")
        files["entry.canonical.json"] = row["_document"]
        line = row["_line"]
        files["human_line.json"] = {
            "schema": HUMAN_LINE_SCHEMA,
            "entry_digest": row["entry_digest"],
            "turns": row["turns"],
            # The canonical wire action list, plus the engine's own
            # `differential_digest` after each one: that per-step pin is what
            # lets the Rust replay test re-check a line with no capture on
            # disk.
            "actions": line["actions"],
            "step_digests": line["step_digests"],
            "terminal": line["terminal"],
        }
    else:
        facet = refusal_facet(row)
        if not facet or not facet.get("detail") or not facet.get("class"):
            raise EvalRefusal(
                f"{row['id']}: no refusal to record on any surface")
        files["refusal.json"] = dict(facet, schema=REFUSAL_SCHEMA,
                                     stage=row["stage"])
        if row.get("_document"):
            files["entry.canonical.json"] = row["_document"]
    return files


def manifest_entry(row: Dict[str, Any], kind: str,
                   files: Dict[str, Any]) -> Dict[str, Any]:
    entry = {
        "id": row["id"],
        "kind": kind,
        "categories": categories_for(row, kind),
        "files": sorted(files),
        "seed": row["seed"],
        "node": row["node"],
        "encounter": row.get("encounter"),
        "character": row["character"],
        "ascension": row["ascension"],
        "entry_digest": row.get("entry_digest"),
        "rust_root": row.get("rust"),
        "rust_refusal_class": row.get("rust_class"),
        "lockstep": row.get("lockstep"),
    }
    if kind == "line":
        entry.update(
            turns=row["turns"],
            actions=row["action_count"],
            terminal_hp=row["hp"],
            won=row["won"],
        )
    else:
        entry["refusal_class"] = files["refusal.json"]["class"]
        entry["refusal_surface"] = files["refusal.json"]["surface"]
        entry["refusal_detail"] = files["refusal.json"]["detail"]
    return entry


def write_fixture(row: Dict[str, Any], kind: str,
                  eval_dir: pathlib.Path) -> Dict[str, Any]:
    """Write one fixture directory and return its manifest entry.

    Refuses (I5) rather than writing a partial fixture: `fixture_documents`
    raises before anything is created.
    """
    files = fixture_documents(row, kind)
    entry = manifest_entry(row, kind, files)
    directory = eval_dir / "fights" / row["id"]
    directory.mkdir(parents=True, exist_ok=True)
    for name in sorted(directory.glob("*.json")):
        if name.name not in files:
            name.unlink()
    for name, document in files.items():
        (directory / name).write_text(
            json.dumps(document, indent=1, sort_keys=False) + "\n",
            encoding="utf-8")
    return entry


def write_manifest(entries: List[Dict[str, Any]], summary: Dict[str, Any],
                   eval_dir: pathlib.Path) -> Dict[str, Any]:
    categories: Dict[str, List[str]] = {}
    for entry in entries:
        for tag in entry["categories"]:
            categories.setdefault(tag, []).append(entry["id"])
    manifest = {
        "schema": MANIFEST_SCHEMA,
        "build": BUILD_ID,
        "generated_by": "sim/v0.111.0/engine/tools/eval_suite.py",
        "consent": (
            "Captures contributed by Sean and consenting friends (#2048, "
            "2026-09-05). Fixtures carry opaque ids and sha256 provenance; "
            "no usernames, file names, or raw capture bytes."),
        "census": summary,
        "categories": {name: sorted(ids)
                       for name, ids in sorted(categories.items())},
        "fights": sorted(entries, key=lambda entry: entry["id"]),
    }
    eval_dir.mkdir(parents=True, exist_ok=True)
    (eval_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n", encoding="utf-8")
    return manifest


SEED_LINE_TARGET = 27
#: The suite holds A9/A10 fights only (Sean, 2026-09-23, #2915): a low-A human
#: line is too easy a target to say anything about solver quality, and the
#: A0-A1 Defect set hid a review-search weakness a single A10 run exposed.
SEED_MIN_ASCENSION = 9
#: Certified fights are taken per (character, encounter) up to this many, so a
#: character with hundreds of A10 hallway captures cannot crowd out the bosses
#: and elites the breadth policy exists for.
SEED_CERTIFIED_PER_ENCOUNTER = 3


def _line_facets(row: Dict[str, Any]) -> List[str]:
    """The coverage axes one fixture contributes (breadth, not frequency)."""
    turns = row.get("turns") or 0
    facets = [
        f"character:{row['character']}",
        f"encounter:{row.get('encounter')}",
        f"node:{row.get('node_type')}",
        f"turns:{turns}",
        f"ascension:{row.get('ascension')}",
        f"rust:{row.get('rust')}",
        f"rust_class:{row.get('rust_class')}",
    ]
    if row.get("potions"):
        facets.append("potions")
    if row.get("won") is False:
        facets.append("death")
    return facets


def content_kinds(document: Optional[Dict[str, Any]]) -> set:
    """The card, enchantment, relic, potion and monster kinds a root holds.

    This is what a certified fixture certifies beyond its encounter, so the
    per-encounter cap never drops the only fixture exercising a kind (#2915).
    """
    document = document or {}
    kinds = set()
    for pile in (document.get("piles") or {}).values():
        for card in pile or ():
            kinds.add(f"card:{card.get('id')}")
            if card.get("enchantment"):
                kinds.add(f"enchantment:{card['enchantment']}")
    player = document.get("player") or {}
    kinds.update(f"relic:{relic}" for relic in player.get("relics_entering") or ())
    kinds.update(f"potion:{potion}" for potion in player.get("potions") or ()
                 if potion)
    kinds.update(f"monster:{monster.get('kind')}"
                 for monster in document.get("monsters") or ())
    return kinds


def select_line_rows(rooted: List[Dict[str, Any]],
                     target: int = SEED_LINE_TARGET,
                     prefer: frozenset = frozenset()) -> List[Dict[str, Any]]:
    """The seed set policy (#2048): breadth over frequency, never usage weight.

    In order, and deterministic at every step:

    1. Fights whose recorded line is certified (every native checkpoint
       agrees): every one already in the tree (`prefer`), then new ones up to
       `SEED_CERTIFIED_PER_ENCOUNTER` per (character, encounter), then any further certified fight whose root holds a card,
       enchantment, relic, potion or monster kind no fixture so far certifies
       (`content_kinds`).
    2. Every recorded death. They are rare, and a suite of wins would report a
       solver-versus-human gap the corpus cannot support.
    3. Quotas: every boss and elite encounter per character that the corpus
       can supply at all (certified first, #2915); at least five fights with
       potions in the belt; at least one per character and per node kind.
    4. A greedy fill to `target`, each step taking the candidate that adds the
       most unseen coverage facets (character, encounter, node kind, turn
       count, ascension, Rust admission class, potions, death), ties broken by
       the opaque id so the set is reproducible.
    """
    exact = [row for row in rooted
             if row.get("human") == "exact" and row.get("_line")]
    # Fixtures already in the tree come first wherever the policy has a
    # choice (the per-encounter cap, the quotas, the greedy fill), so a
    # re-seed replaces a fixture only when it must (#2915 follow-up: the
    # id-ordered cap swapped ten still-certified fixtures for lower-id twins).
    exact.sort(key=lambda row: (row["id"] not in prefer, row["id"]))
    chosen: Dict[str, Dict[str, Any]] = {}

    def take(row: Dict[str, Any]) -> None:
        chosen.setdefault(row["id"], row)

    def certified(row: Dict[str, Any]) -> bool:
        return row.get("lockstep") == "lockstep_ok"

    per_encounter: Dict[Tuple[str, str], int] = {}
    for row in exact:
        group = (row["character"], str(row.get("encounter")))
        # A committed certified fixture stays while it certifies; the cap
        # limits only what a re-seed adds, so the suite a solver is measured
        # on changes only when it must.
        if certified(row) and (row["id"] in prefer or per_encounter.get(
                group, 0) < SEED_CERTIFIED_PER_ENCOUNTER):
            per_encounter[group] = per_encounter.get(group, 0) + 1
            take(row)
    certified_kinds: set = set()
    for row in chosen.values():
        certified_kinds |= content_kinds(row.get("_document"))
    for row in exact:
        if certified(row) and row["id"] not in chosen:
            new_kinds = content_kinds(row.get("_document")) - certified_kinds
            if new_kinds:
                certified_kinds |= new_kinds
                take(row)
    for row in exact:
        if row.get("won") is False:
            take(row)

    def quota(predicate, minimum: int) -> None:
        have = sum(1 for row in chosen.values() if predicate(row))
        for row in exact:
            if have >= minimum:
                return
            if row["id"] in chosen or not predicate(row):
                continue
            take(row)
            have += 1

    for node in ("boss", "elite"):
        groups = sorted({(row["character"], str(row.get("encounter")))
                         for row in exact if row.get("node_type") == node})
        for character, encounter in groups:
            def same(row, c=character, e=encounter, n=node):
                return (row["character"] == c and row.get("node_type") == n
                        and str(row.get("encounter")) == e)
            quota(lambda row, same=same: same(row) and certified(row), 1)
            quota(same, 1)
    quota(lambda row: bool(row.get("potions")), 5)
    for character in sorted({row["character"] for row in exact}):
        quota(lambda row, c=character: row["character"] == c, 1)
    for node in sorted({str(row.get("node_type")) for row in exact}):
        quota(lambda row, n=node: str(row.get("node_type")) == n, 1)

    seen: set = set()
    for row in chosen.values():
        seen.update(_line_facets(row))
    while len(chosen) < target:
        best = None
        best_score = -1
        for row in exact:
            if row["id"] in chosen:
                continue
            score = len(set(_line_facets(row)) - seen)
            if score > best_score:
                best, best_score = row, score
        if best is None:
            break
        seen.update(_line_facets(best))
        take(best)
    return [chosen[key] for key in sorted(chosen)]


def select_seed_rows(rows: List[Dict[str, Any]],
                     prefer: frozenset = frozenset()) -> List[Tuple[Dict[str, Any], str]]:
    """Line fixtures by the policy above, plus one fixture per refusal class.

    Refusal classes are taken across every surface — capture pairing, Rust
    entry, opening, load, line replay and lockstep — because the eval set has
    to be able to say what the engine refuses as precisely as what it plays
    (#2048: the refusal-path fixtures).

    Both halves see only fights at or above `SEED_MIN_ASCENSION`.
    """
    chosen: Dict[str, Tuple[Dict[str, Any], str]] = {}
    rows = [row for row in rows
            if (row.get("ascension") or 0) >= SEED_MIN_ASCENSION]
    rooted = [row for row in rows if row.get("stage") == "rooted"]
    for row in select_line_rows(rooted, prefer=prefer):
        chosen[row["id"]] = (row, "line")

    refusals: Dict[str, Dict[str, Any]] = {}
    for row in sorted(rows, key=lambda r: r["id"]):
        if row["id"] in chosen:
            continue
        facet = refusal_facet(row)
        if not facet or not facet.get("class") or not facet.get("detail"):
            continue
        refusals.setdefault(f"{facet['surface']}/{facet['class']}", row)
    for row in refusals.values():
        chosen[row["id"]] = (row, "refusal")
    return [chosen[key] for key in sorted(chosen)]


def explicit_fixtures(eval_dir: pathlib.Path) -> Dict[str, Dict[str, Any]]:
    """Manifest entries `add` landed with an explicit encounter, by id.

    The census cannot reproduce these: their save carries no room kind, and
    the encounter came from capture/run evidence on the `add` command line
    (`encounter_source: explicit_capture_pair`). `seed` therefore keeps them
    as they are, subject to the same ascension floor, rather than deleting
    what it cannot re-derive (#2915: it had silently dropped Insatiable
    `f143c7e6993ed27c`, which the search suites name). Their node and
    baseline tags follow `categories_for`.
    """
    path = eval_dir / "manifest.json"
    if not path.is_file():
        return {}
    kept = {}
    for entry in json.loads(path.read_text())["fights"]:
        provenance = eval_dir / "fights" / entry["id"] / "provenance.json"
        if not provenance.is_file():
            continue
        recorded = json.loads(provenance.read_text())
        if (recorded.get("encounter_source") != "explicit_capture_pair"
                or (entry.get("ascension") or 0) < SEED_MIN_ASCENSION):
            continue
        tags = set(entry["categories"])
        if recorded.get("node_type") in ("monster", "elite", "boss"):
            tags.add(f"node:{recorded['node_type']}")
        if entry.get("kind") == "line":
            tags.add("human")
        kept[entry["id"]] = dict(entry, categories=sorted(tags))
    return kept


def seed_eval_set(census: Dict[str, Any],
                  eval_dir: pathlib.Path) -> Dict[str, Any]:
    explicit = explicit_fixtures(eval_dir)
    manifest_path = eval_dir / "manifest.json"
    prefer = frozenset(
        entry["id"] for entry in json.loads(manifest_path.read_text())["fights"]
    ) if manifest_path.is_file() else frozenset()
    selection = [(row, kind) for row, kind in select_seed_rows(census["rows"], prefer)
                 if row["id"] not in explicit]
    fights_dir = eval_dir / "fights"
    keep = {row["id"] for row, _ in selection} | set(explicit)
    if fights_dir.is_dir():
        for directory in sorted(fights_dir.iterdir()):
            if directory.is_dir() and directory.name not in keep:
                for path in sorted(directory.iterdir()):
                    path.unlink()
                directory.rmdir()
    entries = [write_fixture(row, kind, eval_dir) for row, kind in selection]
    entries.extend(explicit.values())
    manifest = write_manifest(entries, census["summary"], eval_dir)
    refresh_coverage(eval_dir, census["rows"])
    return manifest


def refresh_coverage(eval_dir: pathlib.Path,
                     rows: Optional[List[Dict[str, Any]]] = None) -> None:
    """Regenerate the committed coverage artifacts after a manifest write.

    Only for the committed tree: a scratch `--eval-dir` has no artifacts to
    keep fresh. `rows` is the census the tree was seeded from, which re-measures
    the grid's captured / not-captured split; without it (`add`) the committed
    split is carried forward (`eval/eval_coverage.py`).
    """
    if eval_dir.resolve() != EVAL_DIR.resolve():
        return
    sys.path.insert(0, str(EVAL_DIR))
    import eval_coverage  # noqa: PLC0415 - lives beside the fixtures it reads
    eval_coverage.write(rows)


def index_by_sha(captures: pathlib.Path,
                 patterns: Tuple[str, ...] = ("*/*.mcr", "*/*.save"),
                 ) -> Dict[str, pathlib.Path]:
    """`sha256 -> path` over the capture files a fixture's provenance names.

    Explicitly added captures may use descriptive names; identity is the
    SHA, not the watcher's filename convention. `verify` locates a fixture's
    pair through this one index (so did the roster-pin generator until #2999
    froze those pins, #2787).
    """
    by_sha: Dict[str, pathlib.Path] = {}
    for pattern in patterns:
        for path in sorted(captures.glob(pattern)):
            by_sha.setdefault(sha256_file(path), path)
    return by_sha


UPLOAD_ARCHIVE_DEFAULT = (pathlib.Path.home() / "Library" / "Application Support"
                          / "SlayTheSpire2" / "relay_the_spire_uploader"
                          / "review_provenance")
RUN_HISTORY_DEFAULT = (pathlib.Path.home() / "Library" / "Application Support"
                       / "SlayTheSpire2" / "steam")
UPLOADS_SUBDIR = "uploads"


def _upload_records(run_dir: pathlib.Path) -> List[Dict[str, Any]]:
    """One archived run's provenance records, in capture order.

    The uploader names archive files with a timestamp prefix and assigns
    `capture_index` in that sorted order (ReviewProvenanceCapture), which is
    what `analyze_bundle` needs to choose between save/quit retries.
    """
    records = []
    for index, path in enumerate(sorted(run_dir.glob("*.json"))):
        record = json.loads(path.read_text())
        record.setdefault("capture_index", index)
        records.append(record)
    return records


def import_uploads(archive: pathlib.Path, history: pathlib.Path,
                   captures: pathlib.Path) -> Dict[str, Any]:
    """Write the mod uploader's archived fights into the corpus (#2915).

    This is the `add --upload` source #2048 planned. Each fight the review
    pipeline would resolve (`review_provenance.analyze_bundle` over the
    archive plus the run's `.run`) is written under `captures/uploads/` as a
    watcher-shaped triple:

    * `_mcr_<sha>.mcr`, the capture's exact bytes;
    * `_save_<sha>.save`, the capture run `rust_review.replay_capture_run`
      builds, the same input prod reviews root with `--capture-run`;
    * `_upload_<sha>.json`, the `.run` fight's encounter. `derive_encounter`
      reads it, and its presence makes `entry_input` choose `--capture-run`.

    `census` and `seed` then treat an upload like any other capture, and
    `verify` finds it by sha256. Nothing is guessed: a fight is skipped, by
    name, when its provenance does not resolve, when the adapter rejects its
    entry projection, when it is an event combat, or when the watcher corpus
    already holds a capture of the same fight.
    """
    import review_provenance  # noqa: PLC0415 - the review pipeline's resolver
    import rust_review  # noqa: PLC0415

    out = captures / UPLOADS_SUBDIR
    runs: Dict[Tuple[Any, Any], pathlib.Path] = {}
    for path in sorted(history.glob("**/history/*.run")):
        try:
            raw = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        runs.setdefault((raw.get("seed"), raw.get("start_time")), path)
    watcher = {
        key for key, (path, _replay) in discover_fights(captures)["fights"].items()
        if path.parent != out}

    written: List[Dict[str, Any]] = []
    skipped: Dict[str, int] = {}

    def skip(reason: str) -> None:
        skipped[reason] = skipped.get(reason, 0) + 1

    for run_dir in sorted(p for p in archive.iterdir() if p.is_dir()):
        records = _upload_records(run_dir)
        if not records:
            continue
        identity = None
        for record in records:
            try:
                replay_run = review_provenance._decode_mcr(
                    review_provenance._mcr_bytes(record))["run"]
            except Exception:  # noqa: BLE001 - analyze_bundle names it below
                continue
            identity = (replay_run["rng"]["seed"], replay_run["start_time"])
            break
        run_path = runs.get(identity) if identity else None
        if run_path is None:
            skip("run_file_missing" if identity else "no_decodable_capture")
            continue
        provenance = review_provenance.analyze_bundle(
            json.loads(run_path.read_text()),
            {"schema_version": 2, "fights": records})
        for issue in provenance.issues:
            if issue.reason == "replay_decode_failed":
                skip(issue.reason)
        for index, fight in enumerate(provenance.fights):
            if fight.replay is None:
                skip("no_capture")
                continue
            if fight.tier != "provenance_resolved":
                skip(fight.tier)
                continue
            try:
                run, parsed, capture_run = rust_review.replay_capture_run(
                    run_path, index, fight.replay, fight.entry)
            except (ValueError, KeyError, TypeError, NotImplementedError) as exc:
                skip(f"translation_refused: {type(exc).__name__}")
                continue
            if parsed.node_type not in ("monster", "elite", "boss"):
                # Event combats need the census's later-save lookup, which an
                # upload has no saves for; the sidecar names room fights only.
                skip(f"node_type_{parsed.node_type}")
                continue
            node = global_node_index(capture_run)
            key = (capture_run["rng"]["seed"], capture_run["start_time"], node)
            if key in watcher:
                skip("watcher_capture_exists")
                continue
            capture = review_provenance._mcr_bytes(
                next(r for r in records
                     if r.get("capture_index") == fight.capture_index))
            digest = hashlib.sha256(capture).hexdigest()[:12]
            stem = f"{key[0]}-{node:03d}"
            out.mkdir(parents=True, exist_ok=True)
            (out / f"{stem}_mcr_{digest}.mcr").write_bytes(capture)
            save_path = out / f"{stem}_save_{digest}.save"
            save_path.write_text(json.dumps(capture_run, indent=1) + "\n",
                                 encoding="utf-8")
            upload_sidecar_path(save_path).write_text(json.dumps({
                "encounter": parsed.encounter_id,
                "node_type": parsed.node_type,
                "entry_input": "--capture-run",
                "source": "uploaded .run (#2915)"}, indent=1) + "\n",
                encoding="utf-8")
            written.append({"id": fixture_id(*key), "character": run.character,
                            "node_type": parsed.node_type,
                            "encounter": parsed.encounter_id})
    return {"archive": str(archive), "out": str(out), "written": written,
            "skipped": dict(sorted(skipped.items()))}


def verify_fixtures(eval_dir: pathlib.Path, captures: pathlib.Path,
                    binary: pathlib.Path) -> Dict[str, Any]:
    """Re-derive every rooted fixture from its provenance through Rust.

    The capture and entry save are located by the sha256 pair the fixture
    records — no file name, no player, nothing but the hashes — and walked
    through exactly the calls the census makes: Rust's opening, then the
    recorded line through Rust. A fixture whose canonical root, action line,
    step digests or terminal no longer reproduce is reported, never repaired.

    A fixture whose stored root no longer reproduces from Rust's opening is a
    `problem` like any other: since #2999 there is no second root builder to
    fall back to, and a tree seeded from the frozen Python root is exactly
    what this check must flag rather than excuse.
    """
    manifest = json.loads((eval_dir / "manifest.json").read_text())
    by_sha = index_by_sha(captures)

    checked = 0
    lines_checked = 0
    problems: List[str] = []
    session = EngineSession(binary)
    try:
        for entry in manifest["fights"]:
            directory = eval_dir / "fights" / entry["id"]
            provenance = json.loads((directory / "provenance.json").read_text())
            capture = by_sha.get(provenance["capture_sha256"])
            if capture is None:
                problems.append(f"{entry['id']}: capture not in {captures}")
                continue
            save_path = by_sha.get(provenance.get("save_sha256") or "")
            if save_path is None:
                # An unpaired-capture fixture records exactly that: there is
                # no entry save at its node, which is the refusal it pins.
                if provenance.get("save_sha256"):
                    problems.append(
                        f"{entry['id']}: entry save not in {captures}")
                continue
            if not (directory / "entry.canonical.json").is_file():
                continue
            checked += 1
            replay = mcr_parser.decode(capture)
            try:
                document = rust_opening_document(
                    binary, save_path, provenance["encounter"],
                    provenance["node_type"], replay["version"],
                    source=entry_input(save_path))
            except RustEntryCliError as exc:
                problems.append(f"{entry['id']}: {exc}")
                continue
            if document.get("schema") != CANONICAL_SCHEMA:
                problems.append(f"{entry['id']}: Rust no longer opens the root")
                continue
            document, _ = with_native_reset_order(document, replay)
            digest = canonical_document.differential_digest(document)
            if digest != provenance["entry_digest"]:
                problems.append(
                    f"{entry['id']}: root digest {digest} != recorded "
                    f"{provenance['entry_digest']}")
                continue
            line_path = directory / "human_line.json"
            if not line_path.is_file():
                continue
            lines_checked += 1
            recorded = json.loads(line_path.read_text())
            try:
                line = replay_recorded_line(session, document, replay)
            except LineDiverged as exc:
                problems.append(
                    f"{entry['id']}: recorded line diverged: "
                    f"{exc.check}: {exc.detail}")
                continue
            if line["actions"] != recorded["actions"]:
                problems.append(f"{entry['id']}: recorded action line changed")
            elif line["step_digests"] != recorded["step_digests"]:
                problems.append(f"{entry['id']}: recorded step digests changed")
            elif line["terminal"] != recorded["terminal"]:
                problems.append(f"{entry['id']}: recorded terminal state changed")
    finally:
        session.close()
    return {"fixtures": len(manifest["fights"]), "rooted_checked": checked,
            "lines_checked": lines_checked, "problems": problems}


#: Files whose content is compared as one value: a canonical root, and a
#: recorded line's per-action lists. A difference names the file or field,
#: without printing the value.
_BULK_FIELDS = frozenset({
    "entry.canonical.json", "human_line.json.actions",
    "human_line.json.step_digests", "human_line.json.terminal"})


def _label_value(value: Any) -> str:
    text = json.dumps(value, sort_keys=True)
    return text if len(text) <= 160 else text[:157] + "..."


def _field_differences(where: str, committed: Dict[str, Any],
                       current: Dict[str, Any]) -> List[str]:
    out = []
    for key in sorted(set(committed) | set(current)):
        name = f"{where}.{key}"
        if key not in current:
            out.append(f"{name}: committed {_label_value(committed[key])}, "
                       "not written from the census")
        elif key not in committed:
            out.append(f"{name}: missing, census says "
                       f"{_label_value(current[key])}")
        elif committed[key] != current[key]:
            if name in _BULK_FIELDS:
                out.append(f"{name}: changed")
            else:
                out.append(f"{name}: committed "
                           f"{_label_value(committed[key])} != census "
                           f"{_label_value(current[key])}")
    return out


def label_differences(entry: Dict[str, Any], files: Dict[str, Any],
                      row: Dict[str, Any], kind: str) -> List[str]:
    """Every way one committed fixture differs from what `add` writes (#3270).

    `entry` is the fixture's manifest entry and `files` its committed
    documents by file name; `row` and `kind` are `pair_row`'s for the same
    capture pair. The expected fixture comes from `fixture_documents` and
    `manifest_entry`, the functions `add` and `seed` write with, so there is
    one definition of what a fixture says: kind, categories, `rust_root`,
    `rust_refusal_class`, `lockstep`, `entry_digest`, the refusal's
    class/surface/detail, the turns/actions/HP/won summary, and every file.

    One kind choice is `seed`'s rather than `add`'s: `seed` records a fight
    whose line replays but whose checkpoints disagree as a `rust-lockstep`
    refusal. A committed refusal therefore stays a refusal while the census
    still names one; a fight that now certifies is a difference.
    """
    if (kind == "line" and entry.get("kind") == "refusal"
            and refusal_facet(row) is not None):
        kind = "refusal"
    try:
        expected_files = fixture_documents(row, kind)
    except EvalRefusal as exc:
        return [f"census row cannot be written as a {kind} fixture: {exc}"]
    expected = manifest_entry(row, kind, expected_files)
    out = _field_differences("manifest", entry, expected)
    for name in sorted(set(files) | set(expected_files)):
        if name not in expected_files:
            out.append(f"{name}: committed, not written from the census")
        elif name not in files:
            out.append(f"{name}: missing, the census writes it")
        elif name in _BULK_FIELDS:
            if files[name] != expected_files[name]:
                out.append(f"{name}: changed")
        else:
            out.extend(_field_differences(name, files[name],
                                          expected_files[name]))
    return out


def verify_labels(eval_dir: pathlib.Path, captures: pathlib.Path,
                  binary: pathlib.Path) -> Dict[str, Any]:
    """Compare every fixture with what `add` writes from the census (#3270).

    Unlike the digest walk in `verify_fixtures`, this covers every fixture:
    explicit-capture-pair fixtures (re-rooted with their recorded
    encounter, as `add --encounter` did), refusal fixtures with no
    `entry.canonical.json`, and unpaired captures. A difference is reported
    by fixture and field, never repaired: refresh with `add`.
    """
    manifest = json.loads((eval_dir / "manifest.json").read_text())
    by_sha = index_by_sha(captures)
    run_saves = scan_saves(captures)
    checked = 0
    differences: List[str] = []
    session = EngineSession(binary)
    try:
        for entry in manifest["fights"]:
            directory = eval_dir / "fights" / entry["id"]
            files = {path.name: json.loads(path.read_text())
                     for path in sorted(directory.glob("*.json"))}
            provenance = files.get("provenance.json") or {}
            capture = by_sha.get(provenance.get("capture_sha256") or "")
            if capture is None:
                differences.append(f"{entry['id']}: capture not in {captures}")
                continue
            save_path = None
            if provenance.get("save_sha256"):
                save_path = by_sha.get(provenance["save_sha256"])
                if save_path is None:
                    differences.append(
                        f"{entry['id']}: entry save not in {captures}")
                    continue
            override = None
            if provenance.get("encounter_source") == "explicit_capture_pair":
                override = (provenance.get("encounter"),
                            provenance.get("node_type"))
            try:
                row, kind = pair_row(capture, save_path, binary, session,
                                     encounter_override=override,
                                     run_saves=run_saves)
            except EvalRefusal as exc:
                differences.append(f"{entry['id']}: {exc}")
                continue
            checked += 1
            differences.extend(f"{entry['id']}: {text}" for text in
                               label_differences(entry, files, row, kind))
    finally:
        session.close()
    return {"fixtures": len(manifest["fights"]), "labels_checked": checked,
            "label_differences": differences}


def census_json(census: Dict[str, Any]) -> Dict[str, Any]:
    """The census without the in-memory documents the fixtures consume."""
    rows = [{key: value for key, value in row.items()
             if not key.startswith("_")} for row in census["rows"]]
    return dict(census, rows=rows)


def pair_row(capture: pathlib.Path, entry_save: Optional[pathlib.Path],
             binary: pathlib.Path, session: EngineSession, *,
             encounter_override: Optional[Tuple[str, str]] = None,
             run_saves: Optional[List[Dict[str, Any]]] = None,
             ) -> Tuple[Dict[str, Any], str]:
    """The census row and fixture kind `add` writes for one capture pair.

    This is the single definition of "what a fixture should say": `add`
    writes it, and `verify`'s label pass (#3270) compares every committed
    fixture with it. `entry_save=None` is a capture with no entry save at its
    node, the `unpaired` row `seed` records as a capture-pairing refusal;
    the `add` command line always names a save. `run_saves` is the capture
    corpus's parsed saves (`scan_saves`), scanned from the entry save's
    corpus directory when not supplied.
    """
    replay = mcr_parser.decode(capture)
    if replay["version"] != BUILD_ID:
        raise EvalRefusal(
            f"{capture.name} is build {replay['version']}, not {BUILD_ID}")
    run = replay["run"]
    key = (run["rng"]["seed"], run["start_time"], global_node_index(run))
    first_save: Dict[Tuple[Any, Any, int], pathlib.Path] = {}
    if entry_save is not None:
        save = json.loads(entry_save.read_text())
        save_key = (save["rng"]["seed"], save["start_time"],
                    global_node_index(save))
        if save_key != key:
            raise EvalRefusal(
                f"{entry_save.name} is at {save_key}, the capture at {key}: "
                "pair a capture with the FIRST save at its own "
                "(seed, start_time, node)")
        first_save[key] = entry_save
        if run_saves is None:
            run_saves = scan_saves(entry_save.parent.parent)
    state = {
        "first_save": first_save,
        "run_saves": {(key[0], key[1]): [
            item for item in run_saves or []
            if (item["seed"], item["start_time"]) == (key[0], key[1])]},
    }
    row = census_row(key, capture, replay, state, session, binary,
                     encounter_override=encounter_override)
    return row, ("line" if row.get("_line") else "refusal")


def add_fixture(capture: pathlib.Path, entry_save: pathlib.Path,
                binary: pathlib.Path, eval_dir: pathlib.Path, *,
                encounter_override: Optional[Tuple[str, str]] = None) -> Dict[str, Any]:
    """Write ONE fixture from a capture/entry-save pair (#2048 `add`).

    Refuses rather than writing a partial fixture: a pair whose root Rust
    cannot build or admit, or whose recorded line does not replay exactly,
    becomes a refusal fixture with its class and verbatim detail, never a
    half-written line fixture.
    """
    session = EngineSession(binary)
    try:
        row, kind = pair_row(capture, entry_save, binary, session,
                             encounter_override=encounter_override)
    finally:
        session.close()
    entry = write_fixture(row, kind, eval_dir)
    manifest_path = eval_dir / "manifest.json"
    entries: List[Dict[str, Any]] = []
    summary: Dict[str, Any] = {}
    if manifest_path.is_file():
        existing = json.loads(manifest_path.read_text())
        summary = existing.get("census", {})
        entries = [item for item in existing.get("fights", [])
                   if item["id"] != entry["id"]]
    entries.append(entry)
    write_manifest(entries, summary, eval_dir)
    refresh_coverage(eval_dir)
    return entry


# ---------------------------------------------------------------------------
# Self-test: no binary, no corpus
# ---------------------------------------------------------------------------


class _ScriptedSession:
    """A diff-serve stand-in for the self-test: a tiny deterministic fight.

    One monster (uid 0, 5 HP) and a hand of Strikes (uids 0..n). A `play` of
    any Strike deals 3; the player's turn advances on `end`; the fight is
    over when the monster dies. `legal` offers ONE representative Strike (the
    lowest uid), exactly as the engine dedupes payload-identical copies.
    """

    def __init__(self, hand: int = 3, monster_hp: int = 5,
                 refuse_apply: bool = False, refuse_load: bool = False,
                 native: Optional[Dict[int, Any]] = None):
        self.hand, self.monster_hp = hand, monster_hp
        self.refuse_apply, self.refuse_load = refuse_apply, refuse_load
        self.state: Optional[Dict[str, Any]] = None
        # apply ordinal -> the `native_checkpoints` that apply reports (#3242)
        self.native, self.applies = native or {}, 0

    def _document(self) -> Dict[str, Any]:
        return json.loads(json.dumps(self.state))

    def load(self, document: Dict[str, Any]) -> Dict[str, Any]:
        if self.refuse_load:
            return {"refusal": {"kind": "selftest", "detail": "x"}}
        self.state = json.loads(json.dumps(document))
        return {"ok": {"digest": canonical_document.differential_digest(
            self.state)}}

    def ask(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        cmd = payload["cmd"]
        if cmd == "project":
            return {"state": self._document()}
        if cmd == "legal":
            hand = self.state["piles"]["hand"]
            plays = [{"kind": "play", "uid": hand[0]["uid"], "target": 0}] \
                if hand else []
            return {"actions": plays + [{"kind": "end"}]}
        if cmd == "apply":
            if self.refuse_apply:
                raise ValueError("Rust replay refused: {'refusal': 'x'}")
            action = payload["action"]
            player, monster = self.state["player"], self.state["monsters"][0]
            if action["kind"] == "play":
                hand = self.state["piles"]["hand"]
                hand[:] = [c for c in hand if c["uid"] != action["uid"]]
                self.state["piles"]["discard"].append(
                    {"id": "STRIKE_IRONCLAD", "uid": action["uid"]})
                monster["hp"] = max(0, monster["hp"] - 3)
                if monster["hp"] == 0:
                    player["over"] = True
            else:
                player["turn"] = player.get("turn", 1) + 1
            out = {"digest": canonical_document.differential_digest(
                self.state)}
            if payload.get("native_checkpoints") is True:
                out["native_checkpoints"] = self.native.get(self.applies, [])
            self.applies += 1
            return out
        raise AssertionError(cmd)


def _scripted_root(hand: int = 3, monster_hp: int = 5) -> Dict[str, Any]:
    return {
        "schema": CANONICAL_SCHEMA,
        "player": {"hp": 70, "turn": 1},
        "monsters": [{"uid": 0, "kind": "SHRINKER_BEETLE", "hp": monster_hp,
                      "max_hp": monster_hp}],
        "piles": {"hand": [{"id": "STRIKE_IRONCLAD", "uid": uid}
                           for uid in range(hand)],
                  "draw": [], "discard": [], "exhaust": [], "play": []},
    }


def _scripted_replay(events: List[Dict[str, Any]],
                     hand: int = 3) -> Dict[str, Any]:
    """A decoded-capture stand-in whose deal is `hand` Strikes."""
    return {"version": BUILD_ID, "events": events, "checksums": [],
            "run": {"rng": {"seed": "SELFTEST", "counters": {"Shuffle": 0}},
                    "players": [{"deck": [{"id": "CARD.STRIKE_IRONCLAD"}]
                                 * hand}]}}


def _play(index: int, target: Optional[int] = 1) -> Dict[str, Any]:
    action = {"type": "NetPlayCardAction", "combat_card_index": index,
              "card_id": "STRIKE_IRONCLAD"}
    if target is not None:
        action["target_id"] = target
    return {"event_type": "GameAction", "action": action}


def _end(turn: int) -> Dict[str, Any]:
    return {"event_type": "GameAction",
            "action": {"type": "NetEndPlayerTurnAction", "turn_number": turn}}


def _expect_divergence(events, check: str, **session_args) -> LineDiverged:
    session = _ScriptedSession(**session_args)
    try:
        replay_recorded_line(session, _scripted_root(), _scripted_replay(events))
    except LineDiverged as exc:
        assert exc.check == check, (exc.check, exc.detail)
        return exc
    raise AssertionError(f"expected {check}")


def _native_pair() -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """A canonical state and a native checkpoint that agree on every field
    `rust_replay.check_native_snapshot` compares, with an Osty and a relic
    pet on the native side."""
    streams = mcr_native._REPRESENTED_RNG_STREAMS
    state = {
        "player": {"hp": 50, "max_hp": 70, "block": 3, "energy": 2,
                   "turn": 2, "ally": ["OSTY", 4, 9], "player_weak": 2,
                   "gold": 99, "potion_slots": [None, "FIRE_POTION", None],
                   "orbs": [["FROST", None]]},
        "monsters": [{"kind": "NIBBIT", "hp": 12, "max_hp": 44, "block": 0}],
        "piles": {"hand": [{"id": "STRIKE_NECROBINDER", "upgrade": 1,
                            "uid": 0}]},
        "rng": {field: {"words": [1, 2, 3, 4], "counter": 5}
                for field in streams},
    }
    native = {
        "players": [{"energy": 2, "turn_number": 2, "gold": 99, "stars": 0,
                     "max_potion_count": 3, "potions": ["FIRE_POTION"],
                     "orbs": [{"id": "FROST_ORB", "passive": 2, "evoke": 5}],
                     "piles": [
            {"pile_type": "Hand", "cards": [
                {"card": {"id": "STRIKE_NECROBINDER", "upgrade_level": 1}}]}]}],
        "creatures": [
            {"player_id": 1, "current_hp": 50, "max_hp": 70, "block": 3,
             "powers": [{"id": "WEAK_POWER", "amount": 2}]},
            {"monster_id": "OSTY", "current_hp": 4, "max_hp": 9, "block": 0},
            {"monster_id": "BYRDPIP", "current_hp": RELIC_PET_HP,
             "max_hp": RELIC_PET_HP, "block": 0},
            {"monster_id": "NIBBIT", "current_hp": 12, "max_hp": 44,
             "block": 0}],
        "rng": {"states": {stream: [1, 2, 3, 4] for stream in streams.values()},
                "counters": {stream: 5 for stream in streams.values()}},
    }
    return state, native


def _expect_checkpoint_refusal(state, native, words: str) -> None:
    try:
        check_native_checkpoint(state, native)
    except ValueError as exc:
        assert words in str(exc), exc
        return
    raise AssertionError(f"expected a checkpoint refusal naming {words!r}")


class _RecordingSession:
    """Records requests; answers `legal` with three generic select options
    whose described uids are [5, 6], [6, 5] and [5], and `resolve_selection`
    with the one option describing the recorded uids (null when none does)."""

    DESCRIBED = [[5, 6], [6, 5], [5]]

    def __init__(self):
        self.calls: List[Dict[str, Any]] = []

    def ask(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        self.calls.append(payload)
        if payload["cmd"] == "resolve_selection":
            index = next((i for i, uids in enumerate(self.DESCRIBED)
                          if uids == payload["uids"]), None)
            return {"action": None if index is None else {
                "kind": "select",
                "answer": {"kind": "option_index", "index": index}}}
        if payload["cmd"] == "legal":
            return {"actions": [
                {"kind": "select", "answer": {"kind": "option_index",
                                              "index": index}}
                for index in range(3)] + [{"kind": "end"}]}
        if payload.get("describe_selection"):
            index = payload["action"]["answer"]["index"]
            return {"digest": "d",
                    "selected_uids": [[5, 6], [6, 5], [5]][index]}
        return {"digest": "d"}


def _card(uid: int, name: str, upgrade: int = 0,
          enchantment: Optional[list] = None) -> Dict[str, Any]:
    out: Dict[str, Any] = {"uid": uid, "id": name, "upgrade": upgrade}
    if enchantment:
        out["enchantment"] = enchantment
    return out


def _self_test_opening_pre_auto_pre(state: Dict[str, Any],
                                    native: Dict[str, Any]) -> None:
    """#3392: the opening checksum is compared against the recorded
    pre-AutoPre state, never skipped. `state` agrees with `native`; the root
    below does not (turn one's AutoPre moved a card), so a match proves the
    recorded state was the one compared."""
    replay = {"checksums": [{"context": OPENING_CHECKPOINT,
                             "full_state": native}]}
    root = json.loads(json.dumps(state))
    moved = next(pile for pile in root["piles"].values() if pile)
    root["piles"].setdefault("exhaust", []).append(moved.pop())
    before = {"kind": AFTER_PLAYER_TURN_START, "state": state}

    out = opening_checkpoint(root, replay, [before])
    assert out["opening_checkpoint"] == "match", out
    # Nothing recorded compares the root itself: `None` (not asked) and `[]`
    # (a paused SetupPlayerTurn, whose checksum is the parked root's).
    for recorded in (None, []):
        out = opening_checkpoint(root, replay, recorded)
        assert out["opening_checkpoint"] == "mismatch", (recorded, out)
        assert "card piles" in out["opening_checkpoint_detail"], out
    assert opening_checkpoint(state, replay, [])["opening_checkpoint"] \
        == "match"

    # A malformed report is the checkpoint's mismatch, by name.
    for recorded, words in (
            ([before, before], "recorded twice"),
            ([dict(before, kind="void_form_end_turn_request")],
             "unknown opening native checkpoint kind"),
            ([{"kind": AFTER_PLAYER_TURN_START, "state": None,
               "refusal": "boundary says no"}], "boundary says no"),
            ({"kind": AFTER_PLAYER_TURN_START}, "did not report")):
        out = opening_checkpoint(state, replay, recorded)
        assert out["opening_checkpoint"] == "mismatch", (words, out)
        assert words in out["opening_checkpoint_detail"], (words, out)

    # The wrapper splits; a refusal answers exactly as without the flag.
    wrapper = {"schema": OPENING_CHECKPOINTS_SCHEMA, "state": root,
               "native_checkpoints": [before]}
    assert split_opening_checkpoints(wrapper) == (root, [before])
    refused = {"schema": "sts-sim-entry-v1", "opening": {"built": False}}
    assert split_opening_checkpoints(refused) == (refused, None)


def _self_test_spanned_checkpoints() -> None:
    """#3242: a native checkpoint INSIDE one apply is compared at its own
    boundary, never skipped. The staged state agrees with the checkpoint's
    `full_state`; the scripted projection does not, so a pass proves the
    staged state was the one compared."""
    agreeing, full_state = _native_pair()
    play0 = {"context": "finished action execution PlayCardAction card: "
             "STRIKE index: 0 targetid: 1"}
    play1 = {"context": "finished action execution PlayCardAction card: "
             "STRIKE index: 1 targetid: 1"}
    hook = {"context": "finished action execution GenericHookGameAction "
            "id 0 owner 1 source  last involved "}
    hook_event = {"event_type": "HookAction", "hook_id": 0}
    ready = {"event_type": "GameAction",
             "action": {"type": "NetReadyToBeginEnemyTurnAction"}}

    def run(events, checksums, native):
        replay = _scripted_replay(events)
        replay["checksums"] = checksums
        return replay_recorded_line(_ScriptedSession(native=native),
                                    _scripted_root(), replay)

    def staged(kind, state=agreeing):
        return [{"kind": kind, "state": state}]

    # Void Form: the play's own checkpoint precedes the EndTurn it requested.
    void = staged(rust_replay.VOID_FORM_END_TURN_REQUEST)
    line = run([_play(0), _play(1)],
               [dict(play0, full_state=full_state), play1], {0: void})
    assert line["checkpoint_mismatch"] is None, line
    assert line["native_checkpoints"] == 1, line
    line = run([_play(0), _play(1)],
               [dict(play0, full_state=full_state), play1], {})
    assert line["checkpoint_mismatch"]["step"] == 1, line

    # The AutoPost dispatch: the HookAction after the end names the staged
    # state. Without it the hook checkpoint is never consumed (the Stampede
    # failure); after the enemy-turn marker a HookAction is another hook's.
    events = [_play(0), _end(1), hook_event, ready, _play(1)]
    checksums = [play0, dict(hook, full_state=full_state), play1]
    auto_post = staged(rust_replay.AUTO_POST_HOOK_FINISHED)
    line = run(events, checksums, {1: auto_post})
    assert line["checkpoint_mismatch"] is None, line
    assert line["native_checkpoints"] == 1, line
    for order, native in (
            (events, {}),
            ([_play(0), _end(1), ready, hook_event, _play(1)],
             {1: auto_post})):
        line = run(order, checksums, native)
        assert "action identity" in line["checkpoint_mismatch"]["detail"], line
    # An unclaimed state (no listener yielded) is dropped, never compared.
    line = run([_play(0), _end(1), ready, _play(1)], [play0, play1],
               {1: auto_post})
    assert line["checkpoint_mismatch"] is None, line

    # Nothing is skipped: every malformed report is a named mismatch.
    for native, words in (
            ({0: void + void}, "recorded twice"),
            ({0: staged("mystery")}, "unknown native checkpoint kind"),
            ({0: [{"kind": rust_replay.VOID_FORM_END_TURN_REQUEST,
                   "state": None, "refusal": "boundary says no"}]},
             "boundary says no"),
            ({0: None}, "did not report")):
        line = run([_play(0), _play(1)], [play0, play1], native)
        assert words in line["checkpoint_mismatch"]["detail"], (words, line)


def _label_row(**fields) -> Dict[str, Any]:
    row = {"id": "f000000000000001", "seed": "SEED", "node": 7,
           "character": "IRONCLAD", "ascension": 10, "potions": 0,
           "capture_sha256": "c" * 64, "save_sha256": "s" * 64,
           "node_type": "boss", "encounter": "ENCOUNTER.X",
           "checksummed": True}
    row.update(fields)
    return row


def _certified_row(**fields) -> Dict[str, Any]:
    line = {"actions": [{"kind": "end"}], "step_digests": ["d1"],
            "terminal": {"hp": 50, "turn": 2, "over": True, "won": True}}
    return _label_row(**dict(
        dict(stage="rooted", rust="admitted", human="exact",
             lockstep="lockstep_ok", entry_digest="e" * 64,
             turns=2, action_count=1, hp=50, over=True, won=True,
             _document={"schema": CANONICAL_SCHEMA, "root": 1}, _line=line),
        **fields))


def _committed(row: Dict[str, Any], kind: str
               ) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """A fixture as `add` wrote it, round-tripped through its JSON files."""
    files = fixture_documents(row, kind)
    entry = manifest_entry(row, kind, files)
    return (json.loads(json.dumps(entry)),
            {name: json.loads(json.dumps(doc)) for name, doc in files.items()})


def _self_test_labels() -> None:
    """`verify`'s label pass (#3270): one definition of what a fixture says."""
    # In sync: a certified line, and seed's lockstep refusal of a fight
    # whose line replays but whose checkpoints disagree.
    row = _certified_row()
    assert label_differences(*_committed(row, "line"), row, "line") == []
    mismatch = _certified_row(lockstep="checkpoint_mismatch",
                              lockstep_detail="differs from native hp")
    assert label_differences(*_committed(mismatch, "refusal"), mismatch,
                             "line") == []
    # ... and once that fight certifies, the refusal is stale.
    stale = label_differences(*_committed(mismatch, "refusal"), row, "line")
    assert "manifest.kind: committed \"refusal\" != census \"line\"" in stale, stale

    # A stale class label: the root now refuses for a different reason.
    before = _label_row(stage="rooted", rust="refused", rust_class="power",
                        rust_kind="missing_capability", rust_detail="power X",
                        rust_missing=["power:X"], entry_digest="e" * 64,
                        _document={"schema": CANONICAL_SCHEMA})
    after = dict(before, rust_class="relic", rust_detail="relic Y",
                 rust_missing=["relic:Y"])
    names = {text.split(":", 1)[0]
             for text in label_differences(*_committed(before, "refusal"),
                                           after, "refusal")}
    assert names == {"manifest.rust_refusal_class", "manifest.refusal_class",
                     "manifest.refusal_detail", "manifest.categories",
                     "refusal.json.class", "refusal.json.detail",
                     "refusal.json.missing"}, names

    # A stale explicit-capture-pair opening refusal: no entry.canonical.json,
    # so the digest walk never reaches it, and the fight now certifies.
    explicit = dict(encounter_source="explicit_capture_pair")
    refused = _label_row(stage="opening_refused",
                         refusal_class="boundary_unrepresentable",
                         detail="field z", **explicit)
    entry, files = _committed(refused, "refusal")
    assert "entry.canonical.json" not in files
    now = _certified_row(**explicit)
    out = label_differences(entry, files, now, "line")
    for expected in ("manifest.kind: committed \"refusal\" != census \"line\"",
                     "manifest.lockstep: committed null != census "
                     "\"lockstep_ok\"",
                     "entry.canonical.json: missing, the census writes it",
                     "human_line.json: missing, the census writes it",
                     "refusal.json: committed, not written from the census",
                     "provenance.json.entry_digest: committed null != census "
                     f"\"{'e' * 64}\""):
        assert expected in out, (expected, out)
    # The same fixture, still refusing identically, is in sync.
    assert label_differences(entry, files, refused, "refusal") == []
    # A census row that cannot be written at all is named, not skipped.
    assert label_differences(entry, files, _label_row(stage="rooted"),
                             "refusal")[0].startswith(
        "census row cannot be written as a refusal fixture")
    # Bulk content is named without printing it.
    moved = _certified_row()
    moved["_line"] = dict(moved["_line"], step_digests=["d2"])
    assert label_differences(*_committed(row, "line"), moved, "line") == [
        "human_line.json.step_digests: changed"]


def _boss_save(cur: Dict[str, int], second: Optional[str]) -> Dict[str, Any]:
    """A minimal act save whose map carries both boss nodes (#3426)."""
    return {"current_act_index": 0, "visited_map_coords": [cur], "acts": [{
        "saved_map": {
            "points": [{"coord": {"col": 1, "row": 1}, "type": "monster"}],
            "boss": {"coord": {"col": 3, "row": 16}, "type": "boss"},
            "second_boss": ({"coord": {"col": 3, "row": 17}, "type": "boss"}
                            if second else None)},
        "rooms": {"boss_id": "ENCOUNTER.A_BOSS", "second_boss_id": second,
                  "normal_encounter_ids": ["ENCOUNTER.N"],
                  "normal_encounters_visited": 0}}]}


def _self_test_boss_nodes() -> None:
    """The act boss and the second boss resolve from their own map keys."""
    nowhere = pathlib.Path("no-such-save")
    boss = _boss_save({"col": 3, "row": 16}, "ENCOUNTER.B_BOSS")
    assert node_type(boss) == "boss" and boss_room_slot(boss) == "boss_id"
    assert derive_encounter(boss, ("s", 0, 16), nowhere, {}) == (
        "ENCOUNTER.A_BOSS", "boss")
    second = _boss_save({"col": 3, "row": 17}, "ENCOUNTER.B_BOSS")
    assert node_type(second) == "boss"
    assert boss_room_slot(second) == "second_boss_id"
    assert derive_encounter(second, ("s", 0, 17), nowhere, {}) == (
        "ENCOUNTER.B_BOSS", "boss")
    # An ordinary point still reads `points`; an absent second boss matches
    # nothing, and a boss kind off every boss node refuses by name.
    plain = _boss_save({"col": 1, "row": 1}, None)
    assert node_type(plain) == "monster" and boss_room_slot(plain) is None
    assert derive_encounter(plain, ("s", 0, 1), nowhere, {}) == (
        "ENCOUNTER.N", "monster")
    off_map = _boss_save({"col": 3, "row": 17}, None)
    assert node_type(off_map) is None and boss_room_slot(off_map) is None
    try:
        next_encounter(off_map, "boss")
    except EvalRefusal as exc:
        assert "saved_map.second_boss" in str(exc)
    else:
        raise AssertionError("a boss kind off every boss node must refuse")


def self_test() -> None:
    """Every census branch that needs no binary and no corpus."""
    # 1. The registry pin against boundary.rs, and the type-strict walk.
    assert derive_rust_only_slots() == tuple(sorted(
        RUST_ONLY_PROVENANCE_SLOTS)), (
        "RUST_ONLY_PROVENANCE_SLOTS and boundary.rs's own `// Rust-only:` "
        "markers disagree")
    assert strip_rust_only_slots(
        {"monsters": [{"hp": 3, "power_attachments": [{"power": "STRENGTH"}]}]}
    ) == {"monsters": [{"hp": 3}]}
    assert opening_parity({"monsters": [{"hp": 3}]}, {"monsters": [
        {"hp": 3, "power_attachments": [{"p": 1}]}]}) == (True, [])
    agrees, paths = opening_parity(
        {"monsters": [{"shriek": True}]}, {"monsters": [{"shriek": 1}]})
    assert agrees is False and paths == ["monsters[0].shriek (bool vs int)"]
    assert not type_strict_equal({"a": 0}, {"a": False})

    # 2. One root source; the retired ones are gone.
    assert ROOT_SOURCES == ("rust_opening",)

    # 3. The recorded line through a scripted engine. The deal check reads the
    # capture's shuffle through `sts2_rng`: three identical Strikes make every
    # permutation the same list, so uid k is instance k.
    session = _ScriptedSession()
    line = replay_recorded_line(session, _scripted_root(), _scripted_replay(
        [_play(1), _end(1),
         {"event_type": "GameAction",
          "action": {"type": "NetReadyToBeginEnemyTurnAction"}},
         _play(0)]))
    assert line["actions"] == [{"kind": "play", "uid": 1, "target": 0},
                               {"kind": "end"},
                               {"kind": "play", "uid": 0, "target": 0}], line
    # uid 1 is not the representative `legal` offers (uid 0 is), and is
    # applied as recorded.
    assert line["nonrepresentative_uids"] == 1, line
    assert len(line["step_digests"]) == 3
    assert line["terminal"] == {"hp": 70, "turn": 2, "over": True,
                                "won": True}, line
    assert line["checkpoint_mismatch"] is None
    assert line["native_checkpoints"] == 0

    _expect_divergence([_play(0), _end(2)], "turn_number")
    _expect_divergence([_play(0, target=7)], "target_identity")
    _expect_divergence([_play(5)], "card_identity")
    _expect_divergence([_play(0)], "combat_not_complete")
    _expect_divergence([_play(0), _play(1), _play(2)], "premature_combat_end")
    _expect_divergence([{"event_type": "Mystery"}], "unsupported_event")
    _expect_divergence([_play(0)], "rust_apply_refused", refuse_apply=True)
    _expect_divergence([_play(0)], "root_refused", refuse_load=True)
    _expect_divergence([], "recorded_input_budget")
    _expect_divergence([_play(0)] * (RECORDED_INPUT_BUDGET + 1),
                       "recorded_input_budget")
    # A targetless play of a card `legal` offers only with a target, and no
    # adjacent one-card choice to combine with, is not legal.
    _expect_divergence([_play(0, target=None)], "not_legal")
    _expect_divergence([{"event_type": "PlayerChoice",
                         "result": {"type": "Index", "indexes": [0]}}],
                       "selection")
    try:
        replay_recorded_line(_ScriptedSession(), _scripted_root(hand=2),
                             _scripted_replay([_play(0)], hand=3))
    except LineDiverged as exc:
        assert exc.check == "entry_deal", exc.check
    else:
        raise AssertionError("a deal the root does not hold must diverge")

    # A completed-action checkpoint that disagrees is recorded, not raised:
    # the line stays exact and names the step.
    replay = _scripted_replay([_play(0), _play(1)])
    replay["checksums"] = [{"context": "finished action execution "
                            "PlayCardAction card: STRIKE index: 0 targetid: 1",
                            "full_state": {"players": [], "creatures": []}},
                           {"context": "finished action execution "
                            "PlayCardAction card: STRIKE index: 1 targetid: 1"}]
    line = replay_recorded_line(_ScriptedSession(), _scripted_root(), replay)
    assert line["checkpoint_mismatch"]["step"] == 1, line
    assert "one player" in line["checkpoint_mismatch"]["detail"], line

    _self_test_spanned_checkpoints()

    # The player's pets are compared, then removed, never miscounted.
    state, native = _native_pair()
    check_native_checkpoint(state, native)
    dead = json.loads(json.dumps(native))
    dead["creatures"][1]["current_hp"] = 0
    state_without_ally = json.loads(json.dumps(state))
    del state_without_ally["player"]["ally"]
    check_native_checkpoint(state_without_ally, dead)
    _expect_checkpoint_refusal(state_without_ally, native, "native Osty")
    moved = json.loads(json.dumps(native))
    moved["creatures"][2]["current_hp"] = 9000
    _expect_checkpoint_refusal(state, moved, "relic pet BYRDPIP")
    two = json.loads(json.dumps(native))
    two["creatures"].append(dict(two["creatures"][1]))
    _expect_checkpoint_refusal(state, two, "more than one Osty")
    # Nothing else is exempted: an unknown creature is still a monster.
    stranger = json.loads(json.dumps(native))
    stranger["creatures"].append({"monster_id": "STRANGER", "current_hp": 1,
                                  "max_hp": 1, "block": 0})
    _expect_checkpoint_refusal(state, stranger, "native monsters")
    # Each player field native serializes and Rust carries exactly is its
    # own named mismatch (#3335).
    for slot, value, named, words in (
            ("gold", 98, "gold", "gold: native 99 rust 98"),
            ("stars", 1, "stars", "stars: native 0 rust 1"),
            ("potion_slots", [None, "FIRE_POTION"], "potion_slots",
             "potion_slots: native 3 rust 2"),
            ("potion_slots", [None, "BLOCK_POTION", None], "potions",
             "potions: native ['FIRE_POTION'] rust ['BLOCK_POTION']"),
            ("orbs", [["LIGHTNING", None]], "orbs",
             "orbs: native ['FROST'] rust ['LIGHTNING']")):
        drifted = json.loads(json.dumps(state))
        drifted["player"][slot] = value
        detail = "recorded replay differs from native " + words
        _expect_checkpoint_refusal(drifted, native, detail)
        assert checkpoint_mismatch_field(detail) == named, detail

    # The opening checkpoint compares and records drift; it never splices.
    replay = {"checksums": [{"context": OPENING_CHECKPOINT,
                             "full_state": native}]}
    assert opening_checkpoint(state, replay) == {
        "checksummed": True, "opening_rng_drift": [],
        "opening_counter_drift": [], "opening_counters_match": True,
        "opening_power_mismatches": [], "opening_checkpoint": "match"}
    # #3029: player powers are reported beside the verdict, never in it.
    ticked = json.loads(json.dumps(state))
    ticked["player"]["player_weak"] = 1
    out = opening_checkpoint(ticked, replay)
    assert out["opening_checkpoint"] == "match", out
    assert out["opening_power_mismatches"] == [
        {"class": "amount", "power": "WEAK_POWER", "native": 2,
         "rust": 1}], out
    powers = power_check_fields(dict(out), None)
    assert (powers["power_check"], powers["power_mismatch_step"]) == (
        "mismatch", 0), powers
    drifted = json.loads(json.dumps(state))
    drifted["rng"]["rng"]["counter"] = 6
    drifted["rng"]["niche"]["words"] = [9, 9, 9, 9]
    out = opening_checkpoint(drifted, replay)
    assert out["opening_rng_drift"] == ["niche", "rng"], out
    assert out["opening_counter_drift"] == ["rng"], out
    assert out["opening_checkpoint"] == "mismatch", out
    assert drifted["rng"]["rng"]["counter"] == 6, "the root was spliced"
    _self_test_opening_pre_auto_pre(state, native)

    # The deal: SetupPlayerTurn's Innate front block (reversed) and IMBUED
    # tail are undone, and a combat-start upgrade keeps the physical card.
    root = {"player": {"innate_min_draw": 2}, "piles": {
        "hand": [_card(0, "INNATE_B"), _card(1, "INNATE_A"),
                 _card(2, "S", 1)],
        "draw": [_card(3, "D"), _card(4, "I", 0, ["IMBUED", 1])]}}
    raw = [("I", 0), ("S", 0), ("INNATE_A", 0), ("D", 0), ("INNATE_B", 0)]
    assert _dealt_uids(root, raw) == [4, 2, 1, 3, 0]
    # #3381: turn one's AutoPre played the IMBUED card out of the Draw
    # bottom into Discard; the deal map is unchanged.
    played = json.loads(json.dumps(root))
    played["piles"]["discard"] = [played["piles"]["draw"].pop()]
    assert _dealt_uids(played, raw) == [4, 2, 1, 3, 0]
    for bad in ([("S", 1)] * 5, raw[:4], [("S", 2)] + raw[1:]):
        try:
            _dealt_uids(root, bad)
        except ValueError:
            continue
        raise AssertionError(f"deal {bad} must refuse")
    twins = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card(0, "X")], "draw": [_card(1, "X")]}}
    try:
        _dealt_uids(twins, [("X", 0), ("X", 0)])
    except ValueError:
        pass
    else:
        raise AssertionError("an ambiguous head must refuse, not guess")

    # #3329: an Innate/plain same-id, same-upgrade tie (Queen fight
    # f5549cb3085a3c2b's `BATTLE_TRANCE+0`) is resolved once `_same_card`
    # compares enchantments — the Royally Approved copy is a different
    # physical card from the plain one at the same id/upgrade.
    enchanted_tie = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card(0, "BATTLE_TRANCE", 0, ["ROYALLY_APPROVED", 1])],
        "draw": [_card(1, "BATTLE_TRANCE", 0)]}}
    assert _dealt_uids(enchanted_tie, [
        ("BATTLE_TRANCE", 0, None),
        ("BATTLE_TRANCE", 0, ("ROYALLY_APPROVED", 1)),
    ]) == [1, 0]
    # Without enchantment info in the recorded tuple (the pre-#3329 shape),
    # the same tie is still unresolvable and must still refuse, not guess.
    try:
        _dealt_uids(enchanted_tie, [("BATTLE_TRANCE", 0), ("BATTLE_TRANCE", 0)])
    except ValueError:
        pass
    else:
        raise AssertionError(
            "an id/upgrade-only tie without enchantment info must still refuse")
    # A GENUINE tie — same id, upgrade AND enchantment on both physical
    # cards — stays unresolvable even with enchantment identity available:
    # comparing enchantments narrows real ambiguity, it never manufactures a
    # distinction that is not there.
    true_twins = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card(0, "BATTLE_TRANCE", 0, ["ROYALLY_APPROVED", 1])],
        "draw": [_card(1, "BATTLE_TRANCE", 0, ["ROYALLY_APPROVED", 1])]}}
    try:
        _dealt_uids(true_twins, [
            ("BATTLE_TRANCE", 0, ("ROYALLY_APPROVED", 1)),
            ("BATTLE_TRANCE", 0, ("ROYALLY_APPROVED", 1)),
        ])
    except ValueError:
        pass
    else:
        raise AssertionError("a genuine physical tie must still refuse, not guess")

    # #3152 (UE7YGG9XC3ZB node 29): the Innate BIG_BANG was upgraded +0 -> +1
    # at the opening, so it and the plain BIG_BANG+0 both head a queue when
    # the first recorded BIG_BANG+0 arrives. Only the plain choice reads the
    # rest of the cycle, so the tie is settled by the search. Because the
    # recorded cycle has one reading, its uids are fixed by the recording.
    grown = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card(0, "BIG_BANG", 1), _card(1, "STRIKE"),
                 _card(2, "BIG_BANG")],
        "draw": [_card(3, "GLOW")]}}
    assert _dealt_uids(grown, [("STRIKE", 0), ("BIG_BANG", 0), ("GLOW", 0),
                               ("BIG_BANG", 0)]) == [1, 2, 3, 0]
    # Both readings complete when the plain copy is last: still refused.
    grown_tie = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card(0, "BIG_BANG", 1), _card(1, "STRIKE"),
                 _card(2, "BIG_BANG")], "draw": []}}
    try:
        _dealt_uids(grown_tie, [("STRIKE", 0), ("BIG_BANG", 0),
                                ("BIG_BANG", 0)])
    except ValueError:
        pass
    else:
        raise AssertionError("two complete readings must refuse, not guess")
    # A recorded +1 never reads a +0 root card, so no reading completes.
    try:
        _dealt_uids(grown, [("STRIKE", 0), ("BIG_BANG", 1), ("GLOW", 0),
                            ("BIG_BANG", 1)])
    except ValueError:
        pass
    else:
        raise AssertionError("a downgraded identity must refuse")

    # Y3NULJSNND7N node 21: a Hopper root's opening-created card takes the
    # allocator uid right after the master-deck rows.
    hopper = {"player": {"next_card_uid": 3}, "piles": {
        "hand": [_card(0, "A"), _card(2, "VICIOUS")], "draw": [_card(1, "B")]}}
    assert _opening_created_uids_are_exact(hopper, 2)
    assert _opening_created_uids_are_exact(
        {"player": {"next_card_uid": 2}, "piles": {
            "hand": [_card(0, "A"), _card(1, "B")]}}, 2)
    for bad_next, piles in (
            (4, hopper["piles"]),  # an allocated uid no card holds
            (2, hopper["piles"]),  # a card past the allocator
            (1, {"hand": [_card(0, "A")]}),  # the allocator behind the deck
            (3, {"hand": [_card(0, "A"), _card(2, "V"), _card(2, "W")],
                 "draw": [_card(1, "B")]})):  # a duplicated created uid
        assert not _opening_created_uids_are_exact(
            {"player": {"next_card_uid": bad_next}, "piles": piles}, 2), (
                bad_next, piles)

    # Exact restoration between trial selections, and the k-card rule.
    recording = _RecordingSession()
    current: Dict[str, Any] = {"player": {}}
    restoring = _RestoringSession(recording, {"root": True},
                                  [{"kind": "end"}], current)
    restoring.ask({"cmd": "load", "entry": current})
    assert recording.calls == [{"cmd": "load", "entry": {"root": True}},
                               {"cmd": "apply", "action": {"kind": "end"}}]
    chosen = _multi_card_selection(restoring, current, [6, 5])
    assert chosen["answer"]["index"] == 1, chosen
    try:
        _multi_card_selection(restoring, current, [7, 8])
    except ValueError:
        pass
    else:
        raise AssertionError("an unmatched k-card choice must diverge")
    assert list(_wire_in_fixture_order(
        {"answer": {"index": 1, "kind": "option_index"}, "kind": "select"}
    )) == ["kind", "answer"]
    assert list(_wire_in_fixture_order(
        {"answer": {"index": 1, "kind": "option_index"}, "kind": "select"}
    )["answer"]) == ["kind", "index"]

    # The verdict, branch by branch.
    exact = {"actions": [{"kind": "end"}] * 3, "checkpoint_mismatch": None,
             "native_checkpoints": 2}
    matched = {"checksummed": True, "opening_checkpoint": "match"}
    assert certification_verdict({}, exact)["lockstep"] == \
        "no_native_checkpoints"
    assert certification_verdict(
        {"checksummed": True, "opening_checkpoint": "mismatch",
         "opening_checkpoint_detail": "d"}, exact) == {
        "lockstep": "checkpoint_mismatch", "lockstep_step": 0,
        "lockstep_detail": "d"}
    assert certification_verdict(matched, dict(
        exact, checkpoint_mismatch={"step": 2, "detail": "x"})) == {
        "lockstep": "checkpoint_mismatch", "lockstep_step": 2,
        "lockstep_detail": "x"}
    assert certification_verdict(matched, dict(
        exact, native_checkpoints=0))["lockstep"] == "no_native_checkpoints"
    assert certification_verdict(matched, exact) == {
        "lockstep": "lockstep_ok", "lockstep_step": 3}

    # 4. A capture with no opening checkpoint is never compared.
    assert opening_checkpoint({}, {"checksums": []}) == {"checksummed": False}

    # 5. The certification verdict and its tallies.
    rows = [
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "lockstep_ok", "checksummed": True,
         "opening_checkpoint": "match", "opening_counters_match": True,
         "native_checkpoints": 4, "character": "IRONCLAD",
         "encounter": "ENCOUNTER.X"},
        {"stage": "rooted", "rust": "admitted", "human": "diverged",
         "human_class": "diverged:selection", "checksummed": True,
         "opening_checkpoint": "match", "opening_counters_match": False,
         "opening_counter_drift": ["rng"]},
        {"stage": "rooted", "rust": "refused", "rust_class": "power"},
        {"stage": "opening_refused", "refusal_class": "boundary_unrepresentable"},
        {"stage": "entry_refused", "refusal_class": "epoch_provenance"},
    ]
    summary = census_summary(rows)
    assert summary["rust_admitted"] == 2 and summary["rooted"] == 3
    assert summary["lockstep"] == {"lockstep_ok": 1}, summary
    assert summary["human"] == {"diverged": 1, "exact": 1}, summary
    assert summary["native_checkpoints_validated"] == 4
    assert summary["opening_counters_checked"] == 2
    assert summary["opening_counters_match"] == 1
    assert summary["opening_counter_drift_streams"] == {"rng": 1}
    assert summary["opening_refusal_classes"] == {"boundary_unrepresentable": 1}
    assert summary["entry_refusal_classes"] == {"epoch_provenance": 1}
    assert "Rust opening refusals" in census_markdown(
        {"captures": "c", "build": BUILD_ID, "summary": summary})
    # Nothing here is unusable: every fight is eligible (#3421).
    assert summary["certified"] == 1 and summary["eligible"] == len(rows)
    assert summary["excluded_unusable"] == {}

    # 5b. #3421: certified / eligible. Only captures proven unusable leave
    # the eligible count; a combat the harness failed to pair stays in it.
    empty = {"events": [], "checksums": []}
    fought = {"events": [{"event_type": "GameAction"}], "checksums": [{}]}
    assert capture_holds_no_combat(empty)
    assert not capture_holds_no_combat(fought)
    assert not capture_holds_no_combat({"events": [], "checksums": [{}]})
    assert with_unusable_capture({"stage": "no_encounter"}, empty) == {
        "stage": "no_encounter", "unusable": UNUSABLE_NO_COMBAT}
    assert with_unusable_capture({"stage": "no_encounter"}, fought) == {
        "stage": "no_encounter"}
    assert with_unusable_capture({"stage": "unpaired"}, fought) == {
        "stage": "unpaired"}
    eligible_rows = rows + [
        {"stage": "no_encounter", "unusable": UNUSABLE_NO_COMBAT},
        {"stage": "no_encounter"},        # a real fight the harness missed
        {"stage": "unpaired"},            # a real fight with no entry save
        {"stage": "rooted", "rust": "admitted", "human": "truncated"},
        {"stage": "rooted", "rust": "admitted", "human": "truncated"},
    ]
    assert unusable_reason(eligible_rows[-1]) == UNUSABLE_TRUNCATED
    assert unusable_reason(eligible_rows[-3]) is None
    split = census_summary(eligible_rows)
    assert split["fights"] == len(rows) + 5
    assert split["eligible"] == len(rows) + 2, split
    assert split["certified"] == 1
    assert split["excluded_unusable"] == {
        UNUSABLE_TRUNCATED: 2, UNUSABLE_NO_COMBAT: 1}, split
    # Every existing field is unchanged by the new rows' marks.
    assert split["stages"]["no_encounter"] == 2
    assert split["capture_truncated"] == 2
    report = census_markdown(
        {"captures": "c", "build": BUILD_ID, "summary": split})
    assert "**certified / eligible: 1 / 7** (14.3%)" in report, report
    assert ("excluded as unusable captures: 3 (`capture_truncated` 2, "
            "`no_combat_in_capture` 1)") in report, report
    assert census_headline({"certified": 0, "eligible": 0}) == \
        "**certified / eligible: 0 / 0**"

    # 6. Each surface a refusal fixture can name, earliest first.
    surfaces = [
        ({"stage": "unpaired"}, "capture-pairing"),
        ({"stage": "entry_refused", "refusal_class": "x", "detail": "d"},
         "rust-entry"),
        ({"stage": "opening_refused", "refusal_class": "x", "detail": "d"},
         "rust-opening"),
        ({"stage": "rooted", "rust": "refused", "rust_class": "x"},
         "rust-load"),
        ({"stage": "rooted", "rust": "admitted", "human": "diverged"},
         "rust-line"),
        ({"stage": "rooted", "rust": "admitted", "human": "exact",
          "lockstep": "checkpoint_mismatch"}, "rust-lockstep"),
    ]
    for row, surface in surfaces:
        assert refusal_facet(row)["surface"] == surface, (row, surface)
    assert [surface for _, surface in surfaces] == list(REFUSAL_SURFACES)
    assert refusal_facet({"stage": "rooted", "rust": "admitted",
                          "human": "exact", "lockstep": "lockstep_ok"}) is None

    _self_test_labels()
    _self_test_boss_nodes()

    print("self-test: registry and gate passed")
    print("self-test: recorded line decoder passed")
    print("self-test: certification tallies passed")
    print("self-test: all checks passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="Run self-tests and exit")
    parser.add_argument("--binary", type=str, help="Path to sts-sim binary (release)")
    parser.add_argument("--json", action="store_true", help="Output results as JSON")
    parser.add_argument(
        "command", nargs="?",
        choices=["census", "add", "seed", "verify", "import-uploads"],
        help="census: walk the capture corpus end to end (#2048); "
             "add: write one fixture from an --mcr/--save pair; "
             "import-uploads: write the mod uploader's archived fights "
             "into the corpus (#2915); "
             "seed: rebuild the fixture tree from a census; "
             "verify: re-derive every fixture from its provenance, and "
             "compare its labels with what `add` writes from the census.")
    parser.add_argument("--captures", type=str, default=str(CAPTURES_DEFAULT),
                        help="Capture corpus directory for census/seed")
    parser.add_argument("--eval-dir", type=str, default=str(EVAL_DIR),
                        help="Fixture tree written by add/seed")
    parser.add_argument("--census-json", type=str,
                        help="Write the census rows and summary here")
    parser.add_argument("--markdown", action="store_true",
                        help="census: print the pasteable summary table")
    parser.add_argument("--mcr", type=str, help="add: capture file")
    parser.add_argument("--entry-save", type=str,
                        help="add: the FIRST save at the capture's node")
    parser.add_argument("--encounter", help="add: explicit recorded encounter when the save has no map node type")
    parser.add_argument("--node-type", choices=("monster", "elite", "boss"),
                        help="add: node type paired with --encounter")
    parser.add_argument("--no-labels", dest="labels", action="store_false",
                        help="verify: skip the label pass, which compares "
                             "every fixture's labels and files with what "
                             "`add` writes from the current census, including "
                             "fixtures the digest walk skips (#3270)")
    parser.add_argument("--progress", action="store_true",
                        help="census: per-50-fight progress on stderr")
    parser.add_argument("--powers-verdict", action="store_true",
                        default=POWERS_VERDICT_DEFAULT,
                        help="census: a player-power disagreement fails "
                             "certification (default: reported only, #3029)")
    parser.add_argument("--upload-archive", type=str,
                        default=str(UPLOAD_ARCHIVE_DEFAULT),
                        help="import-uploads: the uploader's review_provenance directory")
    parser.add_argument("--run-history", type=str,
                        default=str(RUN_HISTORY_DEFAULT),
                        help="import-uploads: searched recursively for */history/*.run")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return
    if args.command is None:
        parser.error("name a command (census, add, seed, verify, "
                     "import-uploads) or --self-test")

    if args.command == "import-uploads":
        captures = pathlib.Path(args.captures).expanduser().resolve()
        if not captures.is_dir():
            sys.exit(f"capture corpus not found: {captures}")
        report = import_uploads(
            pathlib.Path(args.upload_archive).expanduser(),
            pathlib.Path(args.run_history).expanduser(), captures)
        print(json.dumps({**report, "written": len(report["written"])}, indent=1))
        return

    binary = find_engine_binary(args.binary)

    if args.command in ("census", "seed"):
        captures = pathlib.Path(args.captures).expanduser().resolve()
        if not captures.is_dir():
            sys.exit(f"capture corpus not found: {captures}")
        census = run_census(captures, binary, progress=args.progress,
                            powers_verdict=args.powers_verdict)
        if args.census_json:
            pathlib.Path(args.census_json).write_text(
                json.dumps(census_json(census), indent=1) + "\n",
                encoding="utf-8")
        if args.command == "seed":
            manifest = seed_eval_set(
                census, pathlib.Path(args.eval_dir).resolve())
            print(f"wrote {len(manifest['fights'])} fixtures to "
                  f"{args.eval_dir}")
        if args.markdown or not args.json:
            print(census_markdown(census))
        if args.json:
            print(json.dumps(census_json(census), indent=1))
        return

    if args.command == "verify":
        captures = pathlib.Path(args.captures).expanduser().resolve()
        if not captures.is_dir():
            sys.exit(f"capture corpus not found: {captures}")
        eval_dir = pathlib.Path(args.eval_dir).resolve()
        report = verify_fixtures(eval_dir, captures, binary)
        if args.labels:
            report.update(verify_labels(eval_dir, captures, binary))
        print(json.dumps(report, indent=1))
        sys.exit(1 if report["problems"] or report.get("label_differences")
                 else 0)

    if args.command == "add":
        if not args.mcr or not args.entry_save:
            sys.exit("add needs --mcr PATH and --entry-save PATH")
        if bool(args.encounter) != bool(args.node_type):
            sys.exit("--encounter and --node-type must be supplied together")
        entry = add_fixture(
            pathlib.Path(args.mcr).expanduser().resolve(),
            pathlib.Path(args.entry_save).expanduser().resolve(),
            binary, pathlib.Path(args.eval_dir).resolve(),
            encounter_override=(args.encounter, args.node_type) if args.encounter else None)
        print(json.dumps(entry, indent=1))
        return


if __name__ == "__main__":
    main()
