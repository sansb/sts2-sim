#!/usr/bin/env python3
"""Generate the sts-sim content ids and static tables.

Design authority: ``versions/v0.111.0/rust/PORT_PLAN.md`` §3 (D2, D4) and §5.
Issue: #1286 (part of #1282); the ``--source`` switch is #2496 (P1 of the
2026-09-15 decision-record addendum on #1282).

This tool imports the authoritative ``combat_sim`` registry at
``versions/v0.111.0/solver/`` and emits two checked-in Rust files:

* ``src/ids.rs``            — dense ``#[repr(u16)]`` enums + the string tables
                              that form the *only* string boundary (D2);
* ``src/content_tables.rs`` — card rows, monster loops / ``*_MOVES`` tables,
                              template-relic steps, generation pools.

Determinism contract
--------------------
Every set-derived sequence is sorted; dict-derived sequences either keep the
Python insertion order (where that order is semantically load-bearing — move
tables, card step lists) or are sorted by their generated enum ordinal (where
they are looked up by key). Nothing in the output depends on ``PYTHONHASHSEED``,
the wall clock, the checkout path, or the interpreter version. Rerunning the
generator on an unchanged tree therefore reproduces byte-identical files, which
is what the ``rust port`` workflow's freshness step asserts.

Exactness contract (SOLVER_INVARIANTS.md I5)
--------------------------------------------
Nothing is guessed and nothing is silently dropped. A row whose shape or whose
values cannot be transcribed faithfully is emitted as an explicit
``// REFUSED: <reason>`` comment plus a row in the generated
``REFUSED_CONTENT`` table.

Where the facts come from (``--source``)
----------------------------------------
``--source python`` is the behaviour above: every fact comes from the
registry.

``--source dll`` re-sources the *content* half — content identity, per-card
rarity/type/target/cost/keywords/tags, and the unlock-epoch universe — from
``sts2.dll`` itself, through ``dll_content.py``. The *modeling* half (step
programs, move tables, template-relic rules, dispatch, the op-language
vocabularies) has no DLL analogue and stays on the registry; every such fact
is named in ``dll_content.MODELING_ANNEX`` rather than silently retained, and
``dll_content.py --ledger`` prints the whole boundary.

This exists because the addendum's D3 freezes ``combat_sim`` at v0.111.0 while
D4 forks only the crate: a forked crate regenerating from a frozen registry
would reproduce v0.111.0 content on a new build. Under ``--source dll`` the
identity axes are *reconciled* rather than merely preferred — a card the
assembly has and the registry does not (or vice versa) raises, because on a
new build that difference is the porting work.

``--source manifest`` (#2515) replays the DLL facts from the committed
``data/dll_content.<build>.json`` with **no assembly and no dnfile**. It is not
a third opinion about content: ``--source dll`` *writes* that manifest and then
generates from it, so the two are the same facts and the manifest is the DLL
read made reviewable, diffable and hermetic. This exists because the ``rust
port`` workflow's codegen freshness step re-derives in CI, where there is
neither an archive nor ``dnfile`` — and on a forked crate under D3 the frozen
registry would be the *wrong* source, so before the manifest that step had no
usable source at all.

Both sources produce byte-identical output on v0.111.0; that is the acceptance
test (``tools/test_dll_content_source.py``), and it is what makes the DLL path
trustworthy on the next build. The manifest path is pinned by the same test one
hop further out: manifest-generated files must equal the committed ones, and a
DLL-written manifest must equal the committed manifest.

``--source auto`` is the default since 2026-09-16, when Sean approved D1-D4 and
the Python registry stopped being the surface of record. It resolves in this
order:

``dll``
    the archived v0.111.0 assembly **and** its IL reader are both on this host;
``manifest``
    otherwise, when the committed manifest for this build is present — which
    is the normal CI case, and is authoritative rather than a downgrade, since
    those facts came out of the assembly;
``python``
    only when neither is available, and then with a loud note on stderr naming
    exactly what was missing.

A fall back to the frozen registry is the one outcome that must never be
silent. Every run — resolved or explicit — prints ``content source: <kind>``
with the assembly, manifest and sha256 it actually read, so no generated
artifact exists without its provenance on the record.

Usage::

    python3 versions/v0.111.0/rust/tools/generate_content.py           # write
    python3 versions/v0.111.0/rust/tools/generate_content.py --check   # verify
    python3 versions/v0.111.0/rust/tools/generate_content.py \\
        --source dll --check      # force the assembly; fail if unreadable,
                                  # and check the manifest it would write
    python3 versions/v0.111.0/rust/tools/generate_content.py \\
        --source manifest --check # force the committed manifest (what CI runs)
    python3 versions/v0.111.0/rust/tools/generate_content.py \\
        --source python --check   # force the frozen v0.111.0 registry

Where the modeling annex comes from (``--registry``, #2827 item D)
------------------------------------------------------------------
``frozen`` (the only registry since #2827 item F) replays
``data/python_registry.v0.111.0.json``: the frozen v0.111.0 Python registry
*as this generator reads it*, recorded before the simulator's deletion through
a proxy that noted every attribute read and every source-reading census call
(``registry_snapshot.py``). Item F deleted the simulator and with it the
``--registry python`` import and the ``--record-registry`` recorder, so the
snapshot is frozen data. Rust-owned modeling additions go in
``apply_rust_overlays``, never into the snapshot. The source-reading census
extractors below still exist: the frozen registry replays their recorded
answers, and the ``rust port`` controls exercise them against synthetic
sources.

Standard library only on the ``python`` and ``manifest`` paths, and
``--source auto`` needs nothing more than that to resolve to either. Reading
the assembly additionally needs ``dnfile``/``dncil`` and a readable
``sts2.dll``; on this Mac that means ``python3.12``, since the rotating
``python3`` is 3.14 without dnfile — under plain ``python3`` the default
therefore resolves to ``manifest`` and says so.
"""

from __future__ import annotations

import argparse
import ast
import collections
import dataclasses
import decimal
import glob
import json
import os
import pathlib
import re
import sys

TOOL = "versions/v0.111.0/rust/tools/generate_content.py"

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
BUILD_DIR = HERE.parents[2]
REPO_ROOT = HERE.parents[4]
SOLVER_DIR = BUILD_DIR / "solver"

if str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))
import dll_content  # noqa: E402  (sibling tool; stdlib-only until --source dll)


#: The registry the current run reads: the
#: ``registry_snapshot.FrozenRegistry`` replayed from the committed snapshot
#: (or, in a control, a synthetic namespace).
#: Source-reading censuses consult it (through ``frozen_census``) rather than
#: importing the Python tree themselves.
ACTIVE_REGISTRY = None


def apply_rust_overlays(registry):
    """Rust-owned modeling additions; never mutate the frozen Python engine.

    These are applied on top of the frozen snapshot, so a new Rust-owned fact
    is added here, never recorded into the snapshot (#2827 items D and F).
    """
    import dataclasses

    # v111 CalculatedGamble.OnPlay 0x390e00 discards Hand and draws its
    # count at both levels. OnUpgrade 0xda903 adds Retain (keyword 5);
    # canonical Exhaust (0xda8ad) and zero cost (0xda8a0) remain intact.
    cards = dict(registry.CARDS)
    cards[("CALCULATED_GAMBLE", 1)] = dataclasses.replace(
        cards[("CALCULATED_GAMBLE", 0)], retain=True)
    # #3036: the frozen registry gave Misery+ Exhaust. v111 Misery::OnUpgrade
    # RVA 0xe5c23 raises Damage by 2, then IL_0018 ldc.i4.5 -> AddKeyword
    # (IL_0019): CardKeyword 5 is Retain (the enum's field constants:
    # None 0, Exhaust 1, Ethereal 2, Innate 3, Unplayable 4, Retain 5).
    # Misery declares no CanonicalKeywords, so L0 carries none and L1 carries
    # Retain only. The MiseryExact step (damage 9) is unchanged.
    misery_plus = cards[("MISERY", 1)]
    assert misery_plus.exhausts and not misery_plus.retain, "Misery+ overlay"
    cards[("MISERY", 1)] = dataclasses.replace(
        misery_plus, exhausts=False, retain=True)
    # #3022: the frozen registry gave Sculpting Strike Snap's select step
    # (a copy error; the Python oracle carried it too). v111
    # SculptingStrike.<OnPlay>d__7::MoveNext RVA 0x3b8cec builds a
    # one-element CardKeyword[] holding 2 (Ethereal) at IL_0187 and passes it
    # to CardCmd.ApplyKeyword at IL_0189; its FromHand filter
    # <>c::<OnPlay>b__7_0 RVA 0x3b8cda admits only cards whose
    # GetKeywordsWithSources(2) (LocalKeywords: canonical plus added) lacks
    # Ethereal. Snap (<OnPlay>d__9 RVA 0x3bc99c IL_01ac ldc.i4.5) is the
    # Retain writer the rows were copied from. The attack step is unchanged.
    for upgrade in (0, 1):
        row = cards[("SCULPTING_STRIKE", upgrade)]
        steps = tuple(
            ("select", "hand", 1, 1, "without_ethereal_keyword",
             "apply_permanent_ethereal")
            if step[0] == "select" else step
            for step in row.steps)
        assert steps != tuple(row.steps), "Sculpting Strike select overlay"
        cards[("SCULPTING_STRIKE", upgrade)] = dataclasses.replace(
            row, steps=type(row.steps)(steps))
    # #3147: the frozen registry carried Cascade's raw constructor argument
    # (Cascade::.ctor RVA 0xdab73 IL_0002 ldc.i4.m1 -> canonicalEnergyCost)
    # as its cost. The live cost is CardEnergyCost._base, and v111
    # CardEnergyCost::.ctor RVA 0x11e002 branches on get_CostsX (IL_0022,
    # brtrue IL_0027) to store ldc.i4.0 (IL_002c) into Canonical, then
    # copies Canonical into _base (IL_0039). Cascade::get_HasEnergyCostX
    # (RVA 0xdab80) returns true, so its canonical cost is 0 at both levels,
    # as every other X-cost card (Whirlwind) already carries.
    for upgrade in (0, 1):
        row = cards[("CASCADE", upgrade)]
        assert row.cost == -1 and row.x_cost, "Cascade cost overlay"
        cards[("CASCADE", upgrade)] = dataclasses.replace(row, cost=0)
    # #3151: the frozen registry gave Cosmic Indifference Hologram's
    # destination (Hand, Bottom). v111 CosmicIndifference.<OnPlay>d__5::
    # MoveNext RVA 0x3950c0 selects one Discard card (IL_00c8 ldc.i4.3 ->
    # GetPile, FromCombatPile IL_00db), then, while the card is still in Draw
    # or Discard (IL_018d-IL_0193), IL_01a5-IL_01ab calls
    # CardPileCmd.Add(card, ldc.i4.1, ldc.i4.2, null, false): PileType 1 is
    # Draw and CardPilePosition 2 is Top (the enums' field constants:
    # PileType None 0, Draw 1, Hand 2, Discard 3; CardPilePosition None 0,
    # Bottom 1, Top 2, Random 3). Headbutt already carries that operation.
    for upgrade in (0, 1):
        row = cards[("COSMIC_INDIFFERENCE", upgrade)]
        steps = tuple(
            ("select", "discard", 1, 1, None, ("move", "draw", "top"))
            if step[0] == "select" else step
            for step in row.steps)
        assert steps != tuple(row.steps), "Cosmic Indifference select overlay"
        cards[("COSMIC_INDIFFERENCE", upgrade)] = dataclasses.replace(
            row, steps=type(row.steps)(steps))
    registry.CARDS = cards
    # #3159: Self-Forming Clay applies its own power, not BlockNextTurn.
    # v111 SelfFormingClay.<AfterDamageReceived>d__7::MoveNext RVA 0x330ae4
    # IL_008f calls PowerCmd.Apply<T> through MethodSpec 0x2b002689, whose
    # instantiation blob is TypeDef 1078 = SelfFormingClayPower. The bare
    # field name `self_forming_clay` is already the relic-ownership flag, so
    # the power's state field takes the `*_power` spelling (`hex_power`).
    registry.MODELED_POWER_FIELDS = frozenset(
        registry.MODELED_POWER_FIELDS | {"self_forming_clay_power"})
    return registry


#: Vocabulary the overlays above add to the modeled axes. The frozen
#: snapshot keys each source-reading census on its full ``axes`` argument
#: (``registry_snapshot.census_key``), so a word no recording ever saw would
#: turn every replay into ``FrozenRegistryMiss``. The two censuses keyed on
#: ``axes`` (``solo_unplayable_census`` and ``multiplayer_gate_contract``)
#: read only ``axes["CardId"]``, which no overlay touches; they are replayed
#: under the axes as recorded, via ``recorded_axes``.
RUST_OVERLAY_VOCABULARY: dict[str, frozenset[str]] = {
    # #3022 Sculpting Strike.
    "FilterMode": frozenset({"without_ethereal_keyword"}),
    "StepWord": frozenset({"without_ethereal_keyword",
                           "apply_permanent_ethereal"}),
    # #3159 Self-Forming Clay.
    "PowerId": frozenset({"self_forming_clay_power"}),
    # #3322 Mad Science's Chaos rider (`RUST_STEP_KIND_FAMILIES`).
    "StepKind": frozenset({"mad_science_chaos_exact"}),
}

#: Rust-owned step kinds that no frozen ``CARDS`` row carries, with the family
#: module that owns each body. Only a generated row outside ``CARDS`` can use
#: one: ``card_step_families`` replays the frozen census for every recorded
#: kind, and these are appended to its answer.
#:
#: * ``mad_science_chaos_exact`` (#3322): Mad Science's Chaos rider, emitted
#:   only into ``MAD_SCIENCE_VARIANT_ROWS``. Mad Science is an Event card with
#:   no character, so its body lives with the other owner-pool generators in
#:   ``neutral`` (Discovery, Jackpot).
RUST_STEP_KIND_FAMILIES: dict[str, str] = {
    "mad_science_chaos_exact": "neutral",
}


def recorded_axes(axes):
    """``axes`` without ``RUST_OVERLAY_VOCABULARY``: the axes the frozen
    snapshot's censuses were recorded under. Refuses a stale entry, so the
    strip cannot outlive the overlay that needs it."""
    out = dict(axes)
    for axis, words in RUST_OVERLAY_VOCABULARY.items():
        if axis not in out:
            continue
        missing = words - set(out[axis])
        if missing:
            raise SystemExit(
                f"RUST_OVERLAY_VOCABULARY names {sorted(missing)!r} on {axis}, "
                "but no overlay emits it; drop the stale entry")
        out[axis] = [word for word in out[axis] if word not in words]
    return out


def load_registry(kind="frozen"):
    """The modeling registry, with the Rust-owned overlays applied.

    ``frozen`` replays ``data/python_registry.v0.111.0.json``. It is the only
    kind: the live ``python`` import went with the simulator (#2827 item F).
    """
    global ACTIVE_REGISTRY
    import registry_snapshot  # noqa: PLC0415 - sibling tool

    if kind != "frozen":
        raise ValueError(
            f"unknown registry {kind!r}: only the frozen snapshot remains "
            "(the Python simulator was deleted, #2827 item F)")
    ACTIVE_REGISTRY = apply_rust_overlays(registry_snapshot.load_snapshot())
    return ACTIVE_REGISTRY


def _live_registry():
    """The live namespace a source-reading census walks (never a snapshot)."""
    import registry_snapshot  # noqa: PLC0415

    registry = ACTIVE_REGISTRY
    if registry is None:
        raise registry_snapshot.FrozenRegistryMiss(
            "a source-reading census ran with no registry loaded; the live "
            "Python registry was deleted (#2827 item F)")
    if isinstance(registry, registry_snapshot.RecordingRegistry):
        return registry.raw()
    if isinstance(registry, registry_snapshot.FrozenRegistry):
        raise registry_snapshot.FrozenRegistryMiss(
            "a source-reading census ran against the frozen registry")
    return registry


def _census(fn):
    """Replay a source-reading census from the frozen registry.

    The census bodies below read the Python simulator's source
    (``combat_sim.py``, ``content/``), which #2827 item F deleted. Under the
    frozen registry — the only one left — each call replays its recorded
    answer and the body never runs. The bodies stay as the specification the
    fork-forward restatement follows (#2999: a crate whose manifest moves an
    axis gets ``FrozenRegistryMiss`` naming the census, and those censuses
    then need a Rust- or manifest-owned source). Two of them,
    ``solo_unplayable_census`` and ``multiplayer_gate_contract``, still run
    against synthetic sources in ``test_solo_unplayable_census.py``.
    """
    import registry_snapshot  # noqa: PLC0415

    return registry_snapshot.frozen_census(fn)


# ---------------------------------------------------------------------------
# Refusal ledger
# ---------------------------------------------------------------------------

REFUSALS: list[tuple[str, str]] = []

# `Monster(...)` construction sites whose kind is a runtime value. These are
# NOT refusals: no content row is dropped, because the kinds that flow into
# them come from the move/order tables that the other MonsterKind sources
# already enumerate. They are published so the coverage argument is auditable
# rather than asserted.
DYNAMIC_SITES: list[tuple[str, str]] = []


def refuse(site: str, reason: str) -> None:
    REFUSALS.append((site, reason))


# ---------------------------------------------------------------------------
# Rust identifier helpers
# ---------------------------------------------------------------------------

_RUST_KEYWORDS = frozenset("""
as break const continue crate dyn else enum extern false fn for if impl in let
loop match mod move mut pub ref return self Self static struct super trait true
type unsafe use where while async await box do final macro override priv typeof
unsized virtual yield try
""".split())


def camel(name: str) -> str:
    """``SOME_ID`` / ``some_kind`` / ``RELIC.X`` -> ``SomeId`` / ``SomeKind``."""
    parts = [p for p in re.split(r"[^A-Za-z0-9]+", name) if p]
    out = "".join(p[:1].upper() + p[1:].lower() for p in parts)
    if not out or not out[0].isalpha():
        out = "V" + out
    if out in _RUST_KEYWORDS:
        out = out + "_"
    return out


def rust_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


# ---------------------------------------------------------------------------
# Enum axis extraction
# ---------------------------------------------------------------------------


@_census
def monster_kinds(cs) -> list[str]:
    """Every monster kind that can exist in a fight.

    Extraction source (documented in the generated header):

    1. every ``Monster(...)`` construction site across ``combat_sim.py`` and
       ``content/encounters/*.py``, with the first positional argument
       resolved through ``combat_sim``'s module globals — and, for the loop
       spawns that build a roster from an order table, through the enclosing
       ``for`` statement's iterable;
    2. ``LOOPS`` keys (kinds with a deterministic move machine);
    3. ``_RANDOM_MOVES`` keys and ``FABRICATOR_BOT_MOVES`` keys (kinds whose
       machine is a random branch, and the fabricated minion kinds).

    Sources 2 and 3 cover the kinds that only ever appear from a mid-fight
    spawn, which no builder constructs directly.
    """
    g = vars(cs)
    kinds: set[str] = set()

    files = [SOLVER_DIR / "combat_sim.py"]
    files += [pathlib.Path(p) for p in sorted(
        glob.glob(str(SOLVER_DIR / "content" / "**" / "*.py"), recursive=True))]

    for path in files:
        tree = ast.parse(path.read_text(), filename=str(path))
        # Parent links, so an unresolved loop variable can be traced back to
        # the `for` statement that binds it.
        parent: dict[int, ast.AST] = {}
        for node in ast.walk(tree):
            for child in ast.iter_child_nodes(node):
                parent[id(child)] = node
        for node in ast.walk(tree):
            if not (isinstance(node, ast.Call)
                    and isinstance(node.func, ast.Name)
                    and node.func.id == "Monster" and node.args):
                continue
            arg = node.args[0]
            if isinstance(arg, ast.Constant) and isinstance(arg.value, str):
                kinds.add(arg.value)
                continue
            if isinstance(arg, ast.Name) and isinstance(g.get(arg.id), str):
                kinds.add(g[arg.id])
                continue
            if isinstance(arg, ast.Name):
                resolved = _resolve_loop_var(arg.id, node, parent, g)
                if resolved is not None:
                    kinds.update(resolved)
                    continue
            # Cite the enclosing function, not the line number: a line-anchored
            # citation reddens the freshness check on ANY solver edit above it
            # in a 61k-line file (#1301 — #1295's -14 lines broke main with
            # zero semantic change). Function names survive ordinary edits.
            DYNAMIC_SITES.append((
                f"{path.name}:{_enclosing_function(node, parent)}",
                f"Monster({ast.unparse(arg)}, ...) — kind is a runtime value; "
                f"its values flow from the move/order tables enumerated by "
                f"the LOOPS / _RANDOM_MOVES / FABRICATOR_BOT_MOVES sources"))

    kinds |= set(cs.LOOPS)
    kinds |= set(cs._RANDOM_MOVES)
    kinds |= set(cs.FABRICATOR_BOT_MOVES)
    return sorted(kinds)


def _enclosing_function(node, parent):
    """Name of the nearest enclosing def, or ``module`` at module level."""
    cur = node
    while id(cur) in parent:
        cur = parent[id(cur)]
        if isinstance(cur, (ast.FunctionDef, ast.AsyncFunctionDef)):
            return cur.name
    return "module"


def _resolve_loop_var(name, node, parent, g):
    """Resolve ``for <name> in <global>`` / ``enumerate(<global>)`` bindings."""
    cur = node
    while id(cur) in parent:
        cur = parent[id(cur)]
        if not isinstance(cur, ast.For):
            continue
        target = cur.target
        binds = False
        if isinstance(target, ast.Name) and target.id == name:
            binds = True
        elif isinstance(target, ast.Tuple):
            binds = any(isinstance(e, ast.Name) and e.id == name
                        for e in target.elts)
        if not binds:
            continue
        it = cur.iter
        if (isinstance(it, ast.Call) and isinstance(it.func, ast.Name)
                and it.func.id == "enumerate" and it.args):
            it = it.args[0]
        if isinstance(it, ast.Name):
            value = g.get(it.id)
            if (isinstance(value, (tuple, list, frozenset, set))
                    and value and all(isinstance(v, str) for v in value)):
                return set(value)
        return None
    return None


def step_kinds(cs) -> list[str]:
    out: set[str] = set()
    for table in (cs.CARDS, cs.TEMPLATE_CARDS):
        for card in table.values():
            for step in card.steps:
                if step:
                    out.add(step[0])
    # Rust-owned kinds no frozen row carries (`RUST_STEP_KIND_FAMILIES`).
    out |= set(RUST_STEP_KIND_FAMILIES)
    return sorted(out)


def move_tables(cs):
    """Classify every ``*_MOVES`` global (plus ``LOOPS``) into a known shape.

    Shapes, all four of which occur in the current build:

    * ``A`` tuple of ``(name, kind, args[, repeats])``      -> ``[Move]``
    * ``B`` dict ``name -> (kind, args)``                   -> ``[Move]``
    * ``C`` dict ``monster_kind -> (name, kind, args)``     -> ``[(kind, Move)]``
    * ``D`` dict ``monster_kind -> <shape A or B>``         -> ``[(kind, [Move])]``
    """
    names = ["LOOPS"] + sorted(n for n in dir(cs) if n.endswith("_MOVES"))
    out = []
    for name in names:
        value = getattr(cs, name)
        shape = classify_move_table(value)
        if shape is None:
            refuse(f"move_table:{name}",
                   "unrecognised move-table shape; not one of the four "
                   "transcribable shapes (A/B/C/D)")
            continue
        out.append((name, shape, value))
    return out


def _is_move_entry_a(e):
    return (isinstance(e, tuple) and len(e) in (3, 4)
            and isinstance(e[0], str) and isinstance(e[1], str)
            and isinstance(e[2], tuple))


def _is_move_entry_b(v):
    return (isinstance(v, tuple) and len(v) == 2
            and isinstance(v[0], str) and isinstance(v[1], tuple))


def _is_move_entry_c(v):
    return (isinstance(v, tuple) and len(v) == 3
            and isinstance(v[0], str) and isinstance(v[1], str)
            and isinstance(v[2], tuple))


def classify_move_table(value):
    if isinstance(value, tuple):
        return "A" if value and all(_is_move_entry_a(e) for e in value) else None
    if isinstance(value, dict) and value:
        vals = list(value.values())
        if all(_is_move_entry_b(v) for v in vals):
            return "B"
        if all(_is_move_entry_c(v) for v in vals):
            return "C"
        if all(classify_move_table(v) in ("A", "B") for v in vals):
            return "D"
    return None


def move_kinds(cs, tables) -> list[str]:
    out: set[str] = set()

    def scan(shape, value):
        if shape == "A":
            for e in value:
                out.add(e[1])
        elif shape == "B":
            for v in value.values():
                out.add(v[0])
        elif shape == "C":
            for v in value.values():
                out.add(v[1])
        elif shape == "D":
            for v in value.values():
                scan(classify_move_table(v), v)

    for _name, shape, value in tables:
        scan(shape, value)
    return sorted(out)


def power_ids(cs) -> list[str]:
    # combat_sim owns this complete source-derived field census so validators
    # and code generation cannot grow two subtly different Power vocabularies.
    return sorted(cs.MODELED_POWER_FIELDS)


def select_ops(cs) -> list[str]:
    return sorted({op if isinstance(op, str) else op[0]
                   for _pile, op in cs._SELECT_OPS})


# The `select` step's Python tuple is
# ``("select", pile, min, max, filter, effect)``, so the filter is
# ``Step.args[3]`` once the leading kind is stripped. Position-addressed, like
# every other typed-argument rule here — never inferred from the value.
SELECT_FILTER_ARG = 3


def filter_modes(cs) -> list[str]:
    """The `select` step's filter vocabulary, as the registry uses it.

    The admission-time step compiler resolves this position to a `FilterMode`
    so the play path never compares the text (PORT_PLAN.md D2 + the #1297
    coordinator guidance). `None` (no filter) is not a mode.
    """
    out: set[str] = set()
    for table in (cs.CARDS, cs.TEMPLATE_CARDS):
        for card in table.values():
            for step in card.steps:
                if not step or step[0] != "select":
                    continue
                args = step[1:]
                if len(args) > SELECT_FILTER_ARG:
                    value = args[SELECT_FILTER_ARG]
                    if isinstance(value, str):
                        out.add(value)
                    elif value is not None:
                        refuse("step:select.filter",
                               f"filter argument {value!r} is not a string")
    return sorted(out)


def step_words(tables_src: str) -> list[str]:
    """Every opaque string literal the generated tables still carry.

    Read back out of the emitted `content_tables.rs` rather than re-derived
    from the registry, so the vocabulary is exactly what the admission-time
    compiler can meet: a value the generator already resolved to a typed `Arg`
    never reaches it as text.
    """
    return sorted(set(re.findall(r'Arg::S\("((?:[^"\\]|\\.)*)"\)', tables_src)))


# ---------------------------------------------------------------------------
# Typed-argument resolution
# ---------------------------------------------------------------------------
#
# Step/move arguments are heterogeneous Python tuples. Typing them by set
# membership would be a guess: `conqueror` is both a PowerId and a
# `forge_family_exact` sub-mode selector, and `discard` is both a pile name and
# a select verb. So the mapping is an explicit, position-addressed allowlist,
# and every mapped value is checked against its target table at generation time
# — a miss refuses the row rather than falling back to a string.
#
# "card_key" means the argument is a nested ``(card_id, upgrade)`` tuple.

CARD_STEP_ARG_TYPES: dict[tuple[str, int], str] = {
    ("generate_fixed_status", 0): "card_key",
    ("power_all_serial", 0): "power",
    ("select", 4): "select_op_or_opaque",
}

MOVE_ARG_TYPES: dict[tuple[str, int], str] = {
    ("add_status_discard", 0): "card_key",
    ("add_status_hand", 0): "card_key",
    ("attack_pile_inject", 2): "card_key",
    ("pile_inject", 0): "card_key",
    ("summon_illusion", 0): "monster_kind",
}

RELIC_EFFECT_ARG_TYPES: dict[tuple[str, int], str] = {
    ("power_self", 0): "power",
    ("power_all", 0): "power",
}


class ArgWriter:
    """Renders a Python argument value as a Rust ``Arg`` literal."""

    def __init__(self, axes, tiers=None):
        self.axes = axes
        self.tiers = tiers

    def render(self, value, site, typed=None):
        """Return the Rust literal, or raise ``Untranscribable``."""
        if self.tiers is not None:
            tiered = self.tiers.render(value, site)
            if tiered is not None:
                return tiered
        if typed == "card_key":
            return self._card_key(value, site)
        if typed == "power":
            return self._member("Power", "PowerId", value, site)
        if typed == "monster_kind":
            return self._member("Monster", "MonsterKind", value, site)
        if typed == "select_op_or_opaque":
            return self._select_op(value, site)
        return self._plain(value, site)

    # -- typed forms --------------------------------------------------

    def _card_key(self, value, site):
        if (isinstance(value, tuple) and len(value) == 2
                and isinstance(value[0], str) and isinstance(value[1], int)
                and not isinstance(value[1], bool)):
            return (f"Arg::List(&[{self._member('Card', 'CardId', value[0], site)}"
                    f", Arg::I({value[1]})])")
        raise Untranscribable(
            f"{site}: expected a (card_id, upgrade) key, got {value!r}")

    def _member(self, ctor, enum, name, site):
        table = self.axes[enum]
        if name not in table:
            raise Untranscribable(
                f"{site}: {name!r} is not a member of {enum}")
        return f"Arg::{ctor}({enum}::{camel(name)})"

    def _select_op(self, value, site):
        ops = self.axes["SelectOp"]
        if isinstance(value, str):
            if value in ops:
                return f"Arg::Select(SelectOp::{camel(value)})"
            return f"Arg::S({rust_str(value)})"
        if isinstance(value, tuple) and value and value[0] in ops:
            inner = [f"Arg::Select(SelectOp::{camel(value[0])})"]
            inner += [self._plain(v, site) for v in value[1:]]
            return "Arg::List(&[" + ", ".join(inner) + "])"
        return self._plain(value, site)

    # -- plain forms --------------------------------------------------

    def _plain(self, value, site):
        if value is None:
            return "Arg::Nil"
        if isinstance(value, bool):
            return f"Arg::B({'true' if value else 'false'})"
        if isinstance(value, int):
            return f"Arg::I({value})"
        if isinstance(value, str):
            return f"Arg::S({rust_str(value)})"
        if isinstance(value, (tuple, list)):
            inner = ", ".join(self._plain(v, site) for v in value)
            return f"Arg::List(&[{inner}])"
        raise Untranscribable(
            f"{site}: argument of type {type(value).__name__} is not a "
            "transcribable literal")

    def args(self, values, site, typing=None, kind=None):
        out = []
        for i, v in enumerate(values):
            typed = typing.get((kind, i)) if typing else None
            out.append(self.render(v, f"{site}[{i}]", typed))
        return "&[" + ", ".join(out) + "]"


class Untranscribable(Exception):
    pass


class MoveConstantJoin:
    """The assembly's tiered move constants, joined onto the move tables.

    #2828. `move_constant_sites` declares, by name, which table argument each
    `GetValueIfAscension` constant is; the manifest supplies the triple. This
    class is the whole join, and it fails codegen (never a refusal row) on:

    * a manifest row of a modeled kind that nothing accounts for;
    * a declared name the manifest does not carry;
    * a joined site whose registry value is not that exact `AscensionTier`
      (an untiered int included), or an `AscensionTier` at a site that is not
      joined;
    * a joined site the tables never reach (a stale declaration).
    """

    def __init__(self, cs, axes, src):
        import move_constant_sites as sites  # noqa: PLC0415 - tools-local

        self.sites = dict(sites.MOVE_CONSTANT_SITES)
        self.engine = dict(sites.ENGINE_MOVE_CONSTANTS)
        self.roster = dict(sites.ROSTER_CONSTANTS)
        self.inline_names = dict(sites.INLINE_CONSTANT_NAMES)
        self.tier_type = cs.AscensionTier
        facts = src.monster_move_constants(cs)
        self.rows = {}
        problems = []
        for kind in axes["MonsterKind"]:
            entries = AGGREGATE_MONSTER_CLASSES.get(kind, (kind,))
            present = [entry for entry in entries if entry in facts]
            if not present:
                continue
            refused = [facts[e]["refused"] for e in present
                       if "refused" in facts[e]]
            if refused:
                problems.append(f"{kind}: the manifest refuses it: "
                                f"{refused[0]}")
                continue
            first = facts[present[0]]
            for entry in present[1:]:
                if facts[entry] != first:
                    problems.append(f"{kind}: the classes behind it disagree "
                                    f"({present[0]} vs {entry})")
            for key, row in first.items():
                self.rows[f"{kind}.{key}"] = row
        accounted = (set(self.sites.values()) | set(self.engine)
                     | set(self.roster) | set(sites.NOT_MOVE_CONSTANTS))
        for name in sorted(set(self.rows) - accounted):
            problems.append(f"{name}: a tiered constant of a modeled kind "
                            "that move_constant_sites does not account for")
        for name in sorted(accounted - set(self.rows)):
            problems.append(f"{name}: declared in move_constant_sites but "
                            "absent from the manifest")
        if problems:
            raise SystemExit(f"{TOOL}: move constants: " + "; ".join(problems))
        self.hit = set()

    def const_name(self, name):
        kind, key = name.split(".", 1)
        key = self.inline_names.get(name, key)
        return f"{kind}_{snake(key).upper()}"

    def consumed(self):
        """Every constant the generated tables, the engine or a roster
        builder read."""
        return sorted(set(self.sites.values()) | set(self.engine)
                      | set(self.roster))

    def render(self, value, site):
        """`Arg::Tier(...)` for a joined site; None for any other value."""
        key = site.removeprefix("moves:")
        name = self.sites.get(key)
        tiered = type(value) is self.tier_type
        if name is None:
            if tiered:
                raise SystemExit(
                    f"{TOOL}: {key} holds {value!r} but move_constant_sites "
                    "does not say which native constant it is")
            return None
        row = self.rows[name]
        native = (row["gate"], row["at_or_above"], row["below"])
        mine = ((value.gate, value.at_or_above, value.below)
                if tiered else None)
        if mine != native:
            raise SystemExit(
                f"{TOOL}: {key} is {name}, {native} on the assembly, but the "
                f"registry holds {value!r}. The assembly is the authority; "
                "fix the registry literal")
        self.hit.add(key)
        return f"Arg::Tier(move_constants::{self.const_name(name)})"

    def check_complete(self):
        missed = sorted(set(self.sites) - self.hit)
        if missed:
            raise SystemExit(
                f"{TOOL}: move_constant_sites names table arguments the "
                f"generated tables never reached: {missed}")

    def emit(self):
        out = ["",
               "/// Ascension-tiered monster move constants (#2828), read "
               "from the assembly",
               "/// (`dll_content.DllFacts.monster_move_constants`) and "
               "joined onto the move",
               "/// tables by name (`tools/move_constant_sites.py`). The "
               "catalog selects the",
               "/// fight's tier ([`crate::encounters::tier`]); a body that "
               "reads one directly",
               "/// selects it the same way.",
               "#[rustfmt::skip]",
               "pub mod move_constants {",
               "    use super::AscensionTier;"]
        for name in self.consumed():
            row = self.rows[name]
            kind, key = name.split(".", 1)
            if "@" in key:
                where = f"`{row['declared_by']}/{key}` (RVA `{row['rva']}`)"
            else:
                where = (f"`{row['declared_by']}::get_{key}` "
                         f"(RVA `{row['rva']}`)")
            out.append(f"    /// {kind}: {where}, {row['shape']}.")
            tier = _tier_literal(row)
            out.append(f"    pub const {self.const_name(name)}: AscensionTier "
                       f"= {tier};")
        out.append("}")
        out.append("")
        return "\n".join(out)


# ---------------------------------------------------------------------------
# ids.rs
# ---------------------------------------------------------------------------

ENUM_SOURCES = {
    "CardId": "combat_sim.CARDS keys, id component (upgrade is row data)",
    "PowerId": "_TEMPLATE_POWER_STEPS values, _RELIC_POWER_SELF/_RELIC_POWER_ALL "
               "values, the count_target_powers_for_rend field enumerations "
               "(_REND_TARGET_POWER_NONNEGATIVE_FIELDS / _..._BOOLEAN_FIELDS), "
               "and the hand-authored engine-owned player-power fields",
    "RelicId": "combat_sim.KNOWN_RELICS (hand-modeled | CENSUS_INERT_RELICS | "
               "TEMPLATE_RELICS); identical to the relics_census.json id set",
    "PotionId": "combat_sim.KNOWN_POTIONS",
    "MonsterKind": "Monster(...) construction sites across combat_sim.py and "
                   "content/encounters/*.py, plus LOOPS / _RANDOM_MOVES / "
                   "FABRICATOR_BOT_MOVES keys",
    "EncounterId": "combat_sim.SUPPORTED_ENCOUNTERS (== ENCOUNTER_BUILDERS keys)",
    "StepKind": "distinct steps-tuple kinds across CARDS and TEMPLATE_CARDS",
    "MoveKind": "distinct move kinds across LOOPS and every *_MOVES table",
    "EnchantmentId": "combat_sim.KNOWN_ENCHANTMENTS",
    "SelectOp": "combat_sim._SELECT_OPS verbs",
    "FilterMode": "the select step's filter argument (steps[4] in Python, "
                  "Step.args[3] here) across CARDS and TEMPLATE_CARDS",
    "StepWord": "every string literal the generated content tables still "
                "carry as Arg::S — the opaque sub-mode / pile / filter / "
                "counter-source vocabulary the admission-time step compiler "
                "interns so no dispatch below the canonical boundary compares "
                "text (D2)",
}

IDS_PREAMBLE = """\
//! Generated content id enums — DO NOT EDIT BY HAND.
//!
//! Regenerate with:
//!
//! ```text
//! python3 {tool}
//! ```
//!
//! Source of truth: the assembled `combat_sim` registry at
//! `versions/v0.111.0/solver/`. Freshness is CI-enforced by the `rust port`
//! workflow (regenerate + `git diff --exit-code`).
//!
//! PORT_PLAN.md D2: ids are interned everywhere and strings exist **only** at
//! this boundary. The `NAMES` tables below and the canonical (de)serialization
//! that reads them are the only place a content string may appear; dispatch
//! and predicates below the canonical boundary use these enums.
//!
//! Every enum is `#[repr(u16)]` with explicit discriminants `0..COUNT`,
//! declared in ascending name order, so `as_str` is an array index and
//! `from_str` is a binary search over an already-sorted `NAMES` table.
//!
//! Extraction source per enum:
//!
{sources}

#![allow(clippy::enum_variant_names)]
"""

TABLES_PREAMBLE = """\
//! Generated content tables — DO NOT EDIT BY HAND.
//!
//! Regenerate with:
//!
//! ```text
//! python3 {tool}
//! ```
//!
//! Source of truth: the assembled `combat_sim` registry at
//! `versions/v0.111.0/solver/`. Freshness is CI-enforced by the `rust port`
//! workflow (regenerate + `git diff --exit-code`).
//!
//! PORT_PLAN.md D4: card `steps` rows and monster moves are heterogeneous
//! tuples in Python; they are transcribed here as `Step {{ kind, args }}` /
//! `Move {{ name, kind, args, repeats }}` with `Arg` a small union. This is a
//! **faithful transcription only** — per-kind argument-shape validation is the
//! admission gate's job (R0.6, #1290), never the generator's.
//!
//! String arguments are resolved to typed ids only through an explicit,
//! position-addressed allowlist in the generator, never by set membership:
//! `conqueror` is both a `PowerId` and a `forge_family_exact` sub-mode, and
//! `discard` is both a pile name and a select verb, so membership inference
//! would be a guess. Everything outside the allowlist stays `Arg::S`.
//!
//! Relic-template hook / condition / effect verbs are still `&'static str`
//! here: they are a 21-symbol vocabulary with no enum axis in #1286's scope.
//! `TEMPLATE_RELIC_HOOKS` / `_COND_VERBS` / `_EFFECT_VERBS` publish that
//! vocabulary so interning them stays mechanical.

#![allow(clippy::type_complexity)]

use crate::ids::{{
    CardId, EnchantmentId, EncounterId, MonsterKind, MoveKind, PotionId, PowerId, RelicId,
    SelectOp, StepKind,
}};
"""


def emit_enum(name: str, members: list[str], source: str) -> str:
    variants = [camel(m) for m in members]
    dup = [v for v, n in collections.Counter(variants).items() if n > 1]
    if dup:
        raise SystemExit(
            f"{name}: Rust identifier collision after case folding: {dup}")
    lines = []
    lines.append(f"/// {name} — {len(members)} variants.")
    lines.append("///")
    for chunk in wrap_doc(f"Extraction source: {source}."):
        lines.append(f"/// {chunk}")
    lines.append("#[repr(u16)]")
    lines.append("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, "
                 "PartialOrd, Ord)]")
    lines.append(f"pub enum {name} {{")
    for i, v in enumerate(variants):
        lines.append(f"    {v} = {i},")
    lines.append("}")
    lines.append("")
    lines.append("#[allow(clippy::should_implement_trait)]")
    lines.append(f"impl {name} {{")
    lines.append("    /// Number of variants.")
    lines.append(f"    pub const COUNT: usize = {len(members)};")
    lines.append("")
    lines.append("    /// Canonical names, ascending — the binary-search table.")
    lines.append("    #[rustfmt::skip]")
    lines.append("    pub const NAMES: [&str; Self::COUNT] = [")
    for line in pack(", ".join(rust_str(m) for m in members) + ",", 8):
        lines.append(line)
    lines.append("    ];")
    lines.append("")
    lines.append("    /// Every variant, in discriminant order.")
    lines.append("    #[rustfmt::skip]")
    lines.append(f"    pub const ALL: [{name}; Self::COUNT] = [")
    for line in pack(", ".join(f"{name}::{v}" for v in variants) + ",", 8):
        lines.append(line)
    lines.append("    ];")
    lines.append("")
    lines.append("    /// The canonical name of this variant.")
    lines.append("    pub const fn as_str(&self) -> &'static str {")
    lines.append("        Self::NAMES[*self as usize]")
    lines.append("    }")
    lines.append("")
    lines.append("    /// Parse a canonical name. `O(log COUNT)`, no allocation.")
    lines.append("    pub fn from_str(s: &str) -> Option<Self> {")
    lines.append("        match Self::NAMES.binary_search(&s) {")
    lines.append("            Ok(i) => Some(Self::ALL[i]),")
    lines.append("            Err(_) => None,")
    lines.append("        }")
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def wrap_doc(text: str, width: int = 72) -> list[str]:
    words, out, cur = text.split(), [], ""
    for w in words:
        if cur and len(cur) + 1 + len(w) > width:
            out.append(cur)
            cur = w
        else:
            cur = f"{cur} {w}" if cur else w
    if cur:
        out.append(cur)
    return out


def pack(payload: str, indent: int, width: int = 96) -> list[str]:
    """Wrap a comma-separated literal list into indented lines."""
    pad = " " * indent
    out, cur = [], pad
    for tok in payload.split(" "):
        if not tok:
            continue
        if len(cur) + 1 + len(tok) > width and cur != pad:
            out.append(cur)
            cur = pad + tok
        else:
            cur = cur + (" " if cur != pad else "") + tok
    if cur.strip():
        out.append(cur)
    return out


def build_ids(cs, axes, order):
    sources = "\n".join(
        "//! " + line
        for name in order
        for line in [f"* `{name}` ({len(axes[name])}): {ENUM_SOURCES[name]}"])
    parts = [IDS_PREAMBLE.format(tool=TOOL, sources=sources)]
    for name in order:
        parts.append(emit_enum(name, axes[name], ENUM_SOURCES[name]))
    parts.append(ids_tests(axes, order))
    return "\n".join(parts)


def ids_tests(axes, order):
    lines = ["#[cfg(test)]",
             "#[rustfmt::skip]",
             "mod tests {", "    use super::*;", ""]
    lines.append("    /// Pinned magnitudes. A content change that moves one of")
    lines.append("    /// these is a real event: update the pin deliberately in")
    lines.append("    /// the same PR that regenerates the tables.")
    lines.append("    #[test]")
    lines.append("    fn enum_counts_are_pinned() {")
    for name in order:
        lines.append(f"        assert_eq!({name}::COUNT, {len(axes[name])});")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn names_are_sorted_and_unique() {")
    for name in order:
        lines.append(f"        assert!({name}::NAMES.windows(2)"
                     f".all(|w| w[0] < w[1]));")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn from_str_round_trips_every_variant() {")
    for name in order:
        lines.append(f"        for (i, v) in {name}::ALL.iter().enumerate() {{")
        lines.append(f"            assert_eq!(*v as usize, i);")
        lines.append(f"            assert_eq!({name}::from_str(v.as_str()),"
                     f" Some(*v));")
        lines.append("        }")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn from_str_rejects_unknown_names() {")
    for name in order:
        lines.append(f"        assert_eq!({name}::from_str(\"\"), None);")
        lines.append(f"        assert_eq!({name}::from_str"
                     f"(\"__not_a_real_id__\"), None);")
    lines.append("    }")
    lines.append("}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# content_tables.rs
# ---------------------------------------------------------------------------

TABLE_TYPES = """
/// One transcribed argument of a card step, monster move, or relic-template
/// effect. `List` preserves Python's nested tuples verbatim.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    /// An integer literal.
    I(i64),
    /// A boolean literal.
    B(bool),
    /// An opaque content string (a sub-mode selector, pile name, or filter).
    S(&'static str),
    /// Python `None`.
    Nil,
    /// A nested tuple.
    List(&'static [Arg]),
    /// A resolved card id.
    Card(CardId),
    /// A resolved power id.
    Power(PowerId),
    /// A resolved select verb.
    Select(SelectOp),
    /// A resolved monster kind.
    Monster(MonsterKind),
    /// An ascension-tiered monster move constant, read from the assembly
    /// (`move_constants`, #2828). The catalog compiles it to the fight's tier
    /// with `encounters::tier`; nothing reads it untiered.
    Tier(AscensionTier),
}

/// One entry of a card's `steps` tuple.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Step {
    /// The dispatch kind (`steps[0]` in Python).
    pub kind: StepKind,
    /// The remaining tuple elements, in order.
    pub args: &'static [Arg],
}

/// Current-build `CardType` (native enum values 1 through 6).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CardType {
    /// Native Attack card type.
    Attack = 1,
    /// Native Skill card type.
    Skill = 2,
    /// Native Power card type.
    Power = 3,
    /// A status card; Hidden Gem admits this as its fallback class.
    Status = 4,
    /// A curse card.
    Curse = 5,
    /// A quest card.
    Quest = 6,
}

impl CardType {
    /// All six current-build enum values in native order.
    pub const ALL: [Self; 6] = [
        Self::Attack,
        Self::Skill,
        Self::Power,
        Self::Status,
        Self::Curse,
        Self::Quest,
    ];
}

/// A card row: one `(id, upgrade)` key of the assembled `CARDS` registry.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CardRow {
    /// The card id (the enum axis).
    pub id: CardId,
    /// Upgrade level (the second half of the `CARDS` key).
    pub upgrade: u8,
    /// `Card.name`, including the `+` suffix on upgraded rows.
    pub name: &'static str,
    /// Canonical card rarity from `_CARD_RARITY_BY_ID`.
    pub rarity: CardRarity,
    /// Energy cost.
    pub cost: i64,
    /// The step program.
    pub steps: &'static [Step],
    /// Generation pool tag, when the row carries one.
    pub pool: Option<&'static str>,
    /// Star cost (`-1` when the card has none).
    pub star_cost: i64,
    /// Whether the star cost is X.
    pub star_x: bool,
    /// Whether the card is playable at all.
    pub playable: bool,
    /// Named play condition, when the row carries one.
    pub play_condition: Option<&'static str>,
    /// Ethereal.
    pub ethereal: bool,
    /// Energy lost when the card is drawn.
    pub on_draw_energy_loss: i64,
    /// Exact current-build native card type.
    pub card_type: CardType,
    /// Nimble's native powered-BlockVar owner predicate.
    pub nimble_eligible: bool,
    /// Card type: power.
    pub is_power: bool,
    /// Card type: skill.
    pub is_skill: bool,
    /// Carries `CardTag.Strike`.
    pub strike_tag: bool,
    /// All tags, ascending.
    pub tags: &'static [&'static str],
    /// Exhausts on play.
    pub exhausts: bool,
    /// Requires a target.
    pub targeted: bool,
    /// Target type name.
    pub target_type: &'static str,
    /// Heal amount.
    pub heal: i64,
    /// End-of-turn self damage.
    pub turn_end_dmg: i64,
    /// End-of-turn self HP loss.
    pub turn_end_hp_loss: i64,
    /// End-of-turn self Weak.
    pub turn_end_weak: i64,
    /// End-of-turn self Frail.
    pub turn_end_frail: i64,
    /// End-of-turn HP loss fires from hand.
    pub turn_end_hp_loss_hand: bool,
    /// Exact native Status card type (not Curse).
    pub is_status: bool,
    /// Status or curse.
    pub is_status_curse: bool,
    /// Runs a select.
    pub selects: bool,
    /// Innate.
    pub innate: bool,
    /// X-cost.
    pub x_cost: bool,
    /// Retain.
    pub retain: bool,
    /// Sly.
    pub sly: bool,
}

/// The deterministic successor encoded on a loop entry.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Repeats {
    /// The Python tuple had no fourth element, so the successor is the next
    /// loop position with wraparound.
    Absent,
    /// A fixed successor loop position (despite this generated type's legacy
    /// name).
    Fixed(i64),
    /// A conditional successor spec, transcribed verbatim.
    Conditional(&'static [Arg]),
}

/// One monster move: a loop entry or a `*_MOVES` row.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    /// The move name as the sim and the game's move machine spell it.
    pub name: &'static str,
    /// The dispatch kind.
    pub kind: MoveKind,
    /// The move's argument tuple, in order.
    pub args: &'static [Arg],
    /// Loop successor, when the source tuple carried one.
    pub repeats: Repeats,
}

/// One condition or effect of a relic-template hook rule.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Op {
    /// The verb (see `TEMPLATE_RELIC_COND_VERBS` / `_EFFECT_VERBS`).
    pub verb: &'static str,
    /// The verb's arguments, in order.
    pub args: &'static [Arg],
}

/// One `(conditions, effects)` rule of a relic-template hook.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelicRule {
    /// All conditions must hold.
    pub conds: &'static [Op],
    /// Effects applied in order when they do.
    pub effects: &'static [Op],
}

/// One hook of a template relic.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelicHook {
    /// The hook name (see `TEMPLATE_RELIC_HOOKS`).
    pub hook: &'static str,
    /// The hook's rules, in order.
    pub rules: &'static [RelicRule],
}

/// Coarse partition of the canonical 299-relic census.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RelicClassification {
    /// Non-combat, shop, map, or out-of-combat relic with zero combat effects.
    CensusInert,
    /// Pure template relic (in TEMPLATE_RELICS, not hand-authored active).
    TemplateOnly,
    /// Combat-active relic with hand-authored Python/Rust hook modeling.
    HandAuthoredActive,
}

/// One row of the 299-relic canonical inventory coverage ledger.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelicLedgerRow {
    /// Canonical RelicId enum variant.
    pub id: RelicId,
    /// Wire/content name in combat_sim.KNOWN_RELICS.
    pub name: &'static str,
    /// Partition classification across the 299 known relics.
    pub classification: RelicClassification,
    /// Whether this relic has raw template rules in relic_templates.json (34 total).
    pub in_raw_templates: bool,
    /// Whether this relic is an effective template relic in combat_sim.TEMPLATE_RELICS (18 total).
    pub in_effective_templates: bool,
    /// Whether this relic has compiled rules in TEMPLATE_RELIC_STEPS (18 total).
    pub compiled_template: bool,
    /// Whether this relic has full executable combat semantics in this build.
    pub implemented: bool,
    /// Whether this relic can be accepted into a valid canonical inventory.
    pub inventory_represented: bool,
    /// Whether a single-relic probe is admitted by the simulation admission gate.
    pub exactly_admitted: bool,
}
"""

LOOKUPS = """
/// The card row for `(id, upgrade)`. `O(log n)`, no allocation.
pub fn card_row(id: CardId, upgrade: u8) -> Option<&'static CardRow> {
    CARD_ROWS
        .binary_search_by_key(&(id as u16, upgrade), |r| (r.id as u16, r.upgrade))
        .ok()
        .map(|i| &CARD_ROWS[i])
}

/// Every upgrade level of `id`, ascending.
pub fn card_rows(id: CardId) -> &'static [CardRow] {
    let key = id as u16;
    let lo = CARD_ROWS.partition_point(|r| (r.id as u16) < key);
    let hi = CARD_ROWS.partition_point(|r| (r.id as u16) <= key);
    &CARD_ROWS[lo..hi]
}

/// The deterministic move loop of `kind`, when it has one.
pub fn monster_loop(kind: MonsterKind) -> Option<&'static [Move]> {
    lookup_kind(&LOOPS, kind)
}

/// The random-AI move set of `kind`, when it has one.
pub fn random_moves(kind: MonsterKind) -> Option<&'static [Move]> {
    lookup_kind(&RANDOM_MOVES, kind)
}

/// The template-hook program of `relic`, when it is a template relic.
pub fn template_relic(relic: RelicId) -> Option<&'static [RelicHook]> {
    TEMPLATE_RELIC_STEPS
        .binary_search_by_key(&(relic as u16), |(r, _)| *r as u16)
        .ok()
        .map(|i| TEMPLATE_RELIC_STEPS[i].1)
}

fn lookup_kind(
    table: &'static [(MonsterKind, &'static [Move])],
    kind: MonsterKind,
) -> Option<&'static [Move]> {
    table
        .binary_search_by_key(&(kind as u16), |(k, _)| *k as u16)
        .ok()
        .map(|i| table[i].1)
}

/// Coverage ledger row for `relic`. O(1), no allocation.
pub const fn relic_ledger_row(relic: RelicId) -> &'static RelicLedgerRow {
    &RELIC_LEDGER[relic as usize]
}
"""


def build_tables(cs, axes, tables, src):
    tiers = MoveConstantJoin(cs, axes, src)
    aw = ArgWriter(axes, tiers)
    parts = [TABLES_PREAMBLE.format(tool=TOOL), TABLE_TYPES]
    parts.append(emit_card_rows(cs, aw, axes, src))
    parts.append(emit_native_unplayable(cs, axes, src))
    parts.append(emit_mad_science_variants(cs, aw, axes, src))
    parts.append(emit_move_tables(cs, aw, axes, tables))
    tiers.check_complete()
    parts.append(emit_template_relics(cs, aw, axes))
    parts.append(emit_relic_ledger(cs, axes))
    pools_src, pool_count = emit_pools(cs, axes, src)
    parts.append(pools_src)
    parts.append(emit_solo_unplayable(cs, axes))
    parts.append(emit_splash_epochs(cs, axes, src))
    parts.append(emit_monster_models(cs, axes, src))
    parts.append(emit_monster_ctor_ints(cs, axes, src))
    parts.append(tiers.emit())
    parts.append(emit_encounter_pool_constants(cs, src))
    parts.append(emit_encounter_rng_draws(src))
    parts.append(emit_encounter_roster_builders(cs, axes))
    parts.append(emit_character_pools(cs, axes, src))
    parts.append(emit_potion_pools(cs, axes, src))
    parts.append(emit_misc(cs, axes))
    parts.append(LOOKUPS)
    parts.append(emit_refusals())
    parts.append(tables_tests(cs, aw, pool_count))
    return "\n".join(parts)


def emit_native_unplayable(cs, axes, src):
    """Emit the exact CardKeyword.Unplayable metadata census."""
    card_index = {name: i for i, name in enumerate(axes["CardId"])}
    keys = sorted(
        src.native_unplayable_keys(cs),
        key=lambda key: (card_index[key[0]], key[1]),
    )
    unknown = set(keys) - set(cs.CARDS)
    if unknown:
        raise SystemExit(
            "native Unplayable census names missing CARDS rows: "
            + ", ".join(f"{cid}+{upgrade}" for cid, upgrade in sorted(unknown))
        )
    lines = [
        "",
        "/// Exact v0.111.0 `CardKeyword.Unplayable` metadata rows.",
        "///",
        "/// This is deliberately not `!CardRow::playable`: dynamic/native",
        "/// CanPlay refusals such as Sloth do not carry keyword 4.",
        "#[rustfmt::skip]",
        f"pub static NATIVE_UNPLAYABLE_CARD_ROWS: [(CardId, u8); {len(keys)}] = [",
    ]
    lines.extend(
        f"    (CardId::{camel(cid)}, {upgrade})," for cid, upgrade in keys
    )
    lines.extend([
        "];",
        "",
        "/// Whether one exact registry row carries native Unplayable.",
        "pub fn card_has_native_unplayable_keyword(id: CardId, upgrade: u8) -> bool {",
        "    NATIVE_UNPLAYABLE_CARD_ROWS",
        "        .binary_search_by_key(&(id as u16, upgrade), |(card, level)| {",
        "            (*card as u16, *level)",
        "        })",
        "        .is_ok()",
        "}",
        "",
    ])
    return "\n".join(lines)


def emit_card_rows(cs, aw, axes, src):
    card_index = {name: i for i, name in enumerate(axes["CardId"])}
    rows = []
    refused_here = []
    for (cid, upgrade), card in sorted(
            cs.CARDS.items(), key=lambda kv: (card_index[kv[0][0]], kv[0][1])):
        site = f"card:{cid}+{upgrade}"
        try:
            if not isinstance(upgrade, int) or isinstance(upgrade, bool):
                raise Untranscribable(f"{site}: upgrade is not an int")
            steps = []
            for si, step in enumerate(card.steps):
                if not step:
                    raise Untranscribable(f"{site}: empty step tuple at {si}")
                args = aw.args(step[1:], f"{site}.steps[{si}]",
                               CARD_STEP_ARG_TYPES, step[0])
                steps.append(
                    f"Step {{ kind: StepKind::{camel(step[0])}, args: {args} }}")
            body = render_card_row(cs, cid, upgrade, card, steps, src)
        except Untranscribable as exc:
            refuse(site, str(exc))
            refused_here.append((site, str(exc)))
            continue
        rows.append(body)

    rarities = src.card_rarity_vocabulary(cs)
    out = ["", "/// Canonical card-rarity vocabulary derived from"
                " `_CARD_RARITY_BY_ID`.",
           "#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]",
           "#[repr(u8)]",
           "pub enum CardRarity {"]
    out.extend(f"    {camel(rarity)}," for rarity in rarities)
    out.extend(["}", "", "impl CardRarity {",
                f"    pub const ALL: [Self; {len(rarities)}] = ["])
    out.extend(f"        Self::{camel(rarity)}," for rarity in rarities)
    out.extend(["    ];", "", "    pub const fn as_str(self) -> &'static str {",
                "        match self {"])
    out.extend(
        f"            Self::{camel(rarity)} => {rust_str(rarity)},"
        for rarity in rarities
    )
    out.extend(["        }", "    }", "}", "",
           "/// Every `(CardId, upgrade)` row of the assembled `CARDS`"
                " registry,",
           "/// ascending by `(id, upgrade)` so `card_row` can binary-search it.",
           "#[rustfmt::skip]",
           f"pub static CARD_ROWS: [CardRow; {len(rows)}] = ["])
    for site, reason in refused_here:
        out.append(f"    // REFUSED: {site} — {reason}")
    out.extend(rows)
    out.append("];")
    out.append("")
    return "\n".join(out)


#: Mad Science's per-variant body (#2942), the modeling half of the variant
#: rows: which op-language steps one saved (type, rider) pair plays, each
#: amount named by the `get_CanonicalVars` var it reads. The numbers come from
#: `data/canonical_vars.v0.111.0.json` (the harness's DLL read, #3027) and the
#: legal (type, rider) domain from the DLL manifest (`mad_science_variants`),
#: so nothing here is a number. v0.111.0 IL, sts2.dll sha256 9cb4f1ad…:
#:
#: * `MadScience/<OnPlay>d__51::MoveNext` RVA 0x3aaf08 switches on
#:   `TinkerTimeType - 1` (IL_005c-IL_0065): Attack -> ExecuteAttack
#:   (IL_007b-IL_0093), Skill -> ExecuteSkill (IL_00f2-IL_00f9), Power ->
#:   ExecutePower (IL_0158-IL_015f), anything else throws (IL_01be). It then
#:   runs `ExecuteRider` (IL_01e4-IL_01fc) only for rider 1 or 3..6
#:   (IL_01c4-IL_01e2: `rider == 1 || rider - 3 <= 3` unsigned).
#: * `<ExecuteAttack>d__52::MoveNext` RVA 0x3aa530: one `DamageCmd.Attack` of
#:   the Damage var's BaseValue (IL_0042-IL_0052) with hit count ViolenceHits
#:   when rider == 2 (Violence) and 1 otherwise (IL_0020-IL_0041, IL_0058),
#:   `Targeting(target)` (IL_006f).
#: * `<ExecuteSkill>d__53::MoveNext` RVA 0x3aae28: `CreatureCmd.GainBlock` of
#:   the Block var with the CardPlay (IL_001d-IL_003a).
#: * `<ExecutePower>d__54::MoveNext` RVA 0x3aa660: a cosmetic `Cast`
#:   animation (IL_0034-IL_0054), then a switch on `rider - 7`
#:   (IL_00ae-IL_00b8): Expertise applies StrengthPower ExpertiseStrength
#:   (IL_00ce-IL_0101) then DexterityPower ExpertiseDexterity
#:   (IL_015f-IL_0192), Curious applies CuriousPower CuriousReduction
#:   (IL_01f5-IL_0228), Improvement applies ImprovementPower 1
#:   (IL_0288-IL_02ab).
#: * `<ExecuteRider>d__57::MoveNext` RVA 0x3aa9c4 switches on `rider - 1`
#:   (IL_0039-IL_0042): Sapping applies WeakPower SappingWeak
#:   (IL_0065-IL_0092) then VulnerablePower SappingVulnerable
#:   (IL_00f1-IL_011e) to the target, Choking applies StranglePower
#:   ChokingDamage to the target (IL_0182-IL_01af), Energized is
#:   `PlayerCmd.GainEnergy(EnergizedEnergy)` (IL_0212-IL_0232), Wisdom is
#:   `CardPileCmd.Draw(WisdomCards)` (IL_0295-IL_02bb), and Chaos generates
#:   one card of the owner's unlocked pool (IL_031e-IL_0395).
#:
#: `None` is a body this crate has not ported: the variant keeps its identity
#: and refuses by name when a fight holds it (never a nearest-row guess).
MAD_SCIENCE_TYPE_BODY = {
    "Attack": ("attack",),
    "Skill": ("block", "Block"),
    "Power": None,
}
MAD_SCIENCE_RIDER_BODY = {
    "Sapping": (("weak", "SappingWeak"), ("vulnerable", "SappingVulnerable")),
    "Violence": (),
    "Choking": (("strangle", "ChokingDamage"),),
    "Energized": (("energy", "EnergizedEnergy"),),
    "Wisdom": (("draw", "WisdomCards"),),
    # #3322: Chaos is one zero-argument body (`mad_science_chaos_exact`),
    # the owner-pool `GetDistinctForCombat` draw at IL_031e-IL_0395.
    "Chaos": (("mad_science_chaos_exact", None),),
    "Expertise": (("strength", "ExpertiseStrength"),
                  ("dexterity", "ExpertiseDexterity")),
    "Curious": None,
    "Improvement": None,
}
MAD_SCIENCE_UNMODELED = {
    "Curious": "CuriousPower (the Power-card cost reduction) is not modeled",
    "Improvement": ("ImprovementPower (the after-combat upgrade) is not "
                    "modeled"),
}
CANONICAL_VARS_MANIFEST = RUST_DIR / "data" / "canonical_vars.v0.111.0.json"


def _mad_science_vars(upgrade):
    """`CARD.MAD_SCIENCE`'s canonical vars at one level, as exact ints."""
    manifest = json.loads(CANONICAL_VARS_MANIFEST.read_text())
    level = manifest["cards"]["CARD.MAD_SCIENCE"]["levels"][upgrade]
    out = {}
    for name, var in level["vars"].items():
        value = decimal.Decimal(var["value"])
        if value != value.to_integral_value():
            raise SystemExit(f"Mad Science var {name} is not integral: {value}")
        out[name] = int(value)
    return out


def emit_mad_science_variants(cs, aw, axes, src):
    """`MAD_SCIENCE_VARIANT_ROWS`: every saved Tinker Time variant (#2942)."""
    facts = src.mad_science_variants(cs)
    variants = facts["variants"]
    legal = [(row["type"], row["rider"]) for row in variants]
    if sorted(MAD_SCIENCE_RIDER_BODY) != sorted({r for _t, r in legal}) or \
            sorted(MAD_SCIENCE_TYPE_BODY) != sorted({t for t, _r in legal}):
        raise SystemExit(
            "the Mad Science body annex and the DLL's legal variant domain "
            f"disagree: {legal!r}")
    rows = []
    for upgrade in (0, 1):
        placeholder = cs.CARDS[("MAD_SCIENCE", upgrade)]
        values = _mad_science_vars(upgrade)
        for variant in variants:
            card_type, rider = variant["type"], variant["rider"]
            head = MAD_SCIENCE_TYPE_BODY[card_type]
            tail = MAD_SCIENCE_RIDER_BODY[rider]
            site = f"card:MAD_SCIENCE+{upgrade}/{card_type}/{rider}"
            lines = [
                "    MadScienceVariantRow {",
                f"        tinker_type: {variant['type_value']},"
                f" rider: {variant['rider_value']},"
                f" rider_name: {rust_str(rider)},",
            ]
            if tail is None:
                lines.append("        row: None,")
                lines.append(
                    f"        unmodeled: Some("
                    f"{rust_str(MAD_SCIENCE_UNMODELED[rider])}),")
                lines.append("    },")
                rows.append("\n".join(lines))
                continue
            program = []
            if head is not None and head[0] == "attack":
                hits = values["ViolenceHits"] if rider == "Violence" else 1
                program.append(("attack", values["Damage"], hits))
            elif head is not None:
                program.append((head[0], values[head[1]]))
            program.extend((kind,) if var is None else (kind, values[var])
                           for kind, var in tail)
            steps = []
            for si, step in enumerate(program):
                args = aw.args(step[1:], f"{site}.steps[{si}]",
                               CARD_STEP_ARG_TYPES, step[0])
                steps.append(
                    f"Step {{ kind: StepKind::{camel(step[0])}, args: {args} }}")
            card = dataclasses.replace(
                placeholder,
                targeted=variant["target_type"] == "AnyEnemy",
                is_power=card_type == "Power",
                is_skill=card_type == "Skill",
            )
            body = render_card_row(
                cs, "MAD_SCIENCE", upgrade, card, steps, src,
                card_type=card_type.lower(),
                target_type=variant["target_type"],
                nimble_eligible=variant["gains_block"])
            body_lines = body.split("\n")
            assert body_lines[0] == "    CardRow {" and body_lines[-1] == "    },"
            lines.append("        row: Some(CardRow {")
            lines.extend("    " + line for line in body_lines[1:-1])
            lines.append("        }),")
            lines.append("        unmodeled: None,")
            lines.append("    },")
            rows.append("\n".join(lines))
    il = facts["il"]
    out = [
        "",
        "/// One saved Tinker Time variant of Mad Science (#2942).",
        "///",
        "/// `MadScience` saves exactly `TinkerTimeType` and `TinkerTimeRider`",
        "/// (`[SavedProperty]` orders 0 and 2), and together they pick the",
        "/// card's type, target and body. The legal domain is the three",
        "/// `RiderEffect[]` blobs of `TinkerTime::ChooseRiderEffect`",
        f"/// {il['TinkerTime::ChooseRiderEffect']}, the type/target/Nimble",
        "/// columns are `MadScience::get_TargetType`",
        f"/// {il['MadScience::get_TargetType']} and `get_GainsBlock`",
        f"/// {il['MadScience::get_GainsBlock']}, and the bodies are",
        "/// `generate_content.MAD_SCIENCE_RIDER_BODY` over the canonical vars.",
        "#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]",
        "pub struct MadScienceVariantRow {",
        "    /// Native `CardType` value saved as `TinkerTimeType`.",
        "    pub tinker_type: u8,",
        "    /// Native `TinkerTime.RiderEffect` value saved as `TinkerTimeRider`.",
        "    pub rider: u8,",
        "    /// The rider's native `RiderEffect` name.",
        "    pub rider_name: &'static str,",
        "    /// The card this variant is at this row's level, or `None` when",
        "    /// its body is not ported.",
        "    pub row: Option<CardRow>,",
        "    /// Why `row` is `None`.",
        "    pub unmodeled: Option<&'static str>,",
        "}",
        "",
        "/// Every legal `(TinkerTimeType, TinkerTimeRider)` pair at both levels,",
        "/// level-major, in `ChooseRiderEffect`'s native order.",
        "#[rustfmt::skip]",
        f"pub static MAD_SCIENCE_VARIANT_ROWS: [MadScienceVariantRow; "
        f"{len(rows)}] = [",
    ]
    out.extend(rows)
    out.extend([
        "];",
        "",
        "/// The variant row for one saved pair at one level, when it is legal.",
        "pub fn mad_science_variant_row(",
        "    tinker_type: u8,",
        "    rider: u8,",
        "    upgrade: u8,",
        ") -> Option<&'static MadScienceVariantRow> {",
        f"    let per_level = MAD_SCIENCE_VARIANT_ROWS.len() / 2;",
        "    let level = usize::from(upgrade);",
        "    MAD_SCIENCE_VARIANT_ROWS",
        "        .get(level * per_level..(level + 1) * per_level)?",
        "        .iter()",
        "        .find(|row| row.tinker_type == tinker_type && row.rider == rider)",
        "}",
        "",
        "/// Whether `row` is one of the generated rows: a `CARD_ROWS` entry or",
        "/// a ported Mad Science variant row. The provenance predicate generic",
        "/// consumers use in place of `card_row(id, upgrade) == Some(row)`,",
        "/// which no variant row can satisfy.",
        "pub fn is_generated_card_row(row: &CardRow) -> bool {",
        "    card_row(row.id, row.upgrade) == Some(row)",
        "        || (row.id == CardId::MadScience",
        "            && MAD_SCIENCE_VARIANT_ROWS",
        "                .iter()",
        "                .any(|variant| variant.row.as_ref() == Some(row)))",
        "}",
        "",
    ])
    return "\n".join(out)


def render_card_row(cs, cid, upgrade, card, steps, src, *, card_type=None,
                    target_type=None, nimble_eligible=None):
    def b(v):
        return "true" if v else "false"

    def opt(v):
        return "None" if v is None else f"Some({rust_str(v)})"

    if card_type is None:
        card_type = src.card_type(cs, cid)
    if target_type is None:
        target_type = src.card_target_type(cs, cid, card)
    if nimble_eligible is None:
        nimble_eligible = cid in cs.NIMBLE_ELIGIBLE_CARD_IDS
    kw = {column: src.card_keyword(cs, cid, upgrade, card, column)
          for column in dll_content.KEYWORD_COLUMNS}
    tags = ", ".join(rust_str(t) for t in src.card_tags(cs, cid, card))
    lines = [
        "    CardRow {",
        f"        id: CardId::{camel(cid)}, upgrade: {upgrade},"
        f" name: {rust_str(card.name)},"
        f" cost: {src.card_cost(cs, cid, upgrade, card)},",
        f"        rarity: CardRarity::{camel(src.card_rarity(cs, cid))},",
    ]
    if steps:
        lines.append("        steps: &[")
        for s in steps:
            lines.append(f"            {s},")
        lines.append("        ],")
    else:
        lines.append("        steps: &[],")
    lines.append(
        f"        pool: {opt(card.pool)}, star_cost: {card.star_cost},"
        f" star_x: {b(card.star_x)}, playable: {b(card.playable)},")
    lines.append(
        f"        play_condition: {opt(card.play_condition)},"
        f" ethereal: {b(kw['ethereal'])},"
        f" on_draw_energy_loss: {card.on_draw_energy_loss},")
    lines.append(
        f"        card_type: CardType::{camel(card_type)},")
    lines.append(
        f"        nimble_eligible: {b(nimble_eligible)},")
    lines.append(
        f"        is_power: {b(card.is_power)}, is_skill: {b(card.is_skill)},"
        f" strike_tag: {b(src.card_strike_tag(cs, cid, card))},"
        f" tags: &[{tags}],")
    lines.append(
        f"        exhausts: {b(kw['exhausts'])},"
        f" targeted: {b(card.targeted)},"
        f" target_type: {rust_str(target_type)},"
        f" heal: {card.heal},")
    lines.append(
        f"        turn_end_dmg: {card.turn_end_dmg},"
        f" turn_end_hp_loss: {card.turn_end_hp_loss},"
        f" turn_end_weak: {card.turn_end_weak},"
        f" turn_end_frail: {card.turn_end_frail},")
    lines.append(
        f"        turn_end_hp_loss_hand: {b(card.turn_end_hp_loss_hand)},"
        f" is_status: {b(card_type == 'status')},"
        f" is_status_curse: {b(card.is_status_curse)},"
        f" selects: {b(card.selects)}, innate: {b(kw['innate'])},")
    lines.append(
        f"        x_cost: {b(card.x_cost)}, retain: {b(kw['retain'])},"
        f" sly: {b(kw['sly'])},")
    lines.append("    },")
    return "\n".join(lines)


def render_move(aw, name, kind, args, repeats, site):
    rendered = aw.args(args, site, MOVE_ARG_TYPES, kind)
    if repeats is None:
        rep = "Repeats::Absent"
    elif isinstance(repeats, bool):
        raise Untranscribable(f"{site}: repeats is a bool")
    elif isinstance(repeats, int):
        rep = f"Repeats::Fixed({repeats})"
    elif isinstance(repeats, tuple):
        rep = f"Repeats::Conditional({aw.args(repeats, site + '.repeats')})"
    else:
        raise Untranscribable(
            f"{site}: repeats has type {type(repeats).__name__}")
    return (f"Move {{ name: {rust_str(name)}, kind: MoveKind::{camel(kind)},"
            f" args: {rendered}, repeats: {rep} }}")


def _moves_of(aw, shape, value, site):
    out = []
    if shape == "A":
        for e in value:
            out.append(render_move(aw, e[0], e[1], e[2],
                                   e[3] if len(e) > 3 else None,
                                   f"{site}.{e[0]}"))
    elif shape == "B":
        for name, v in value.items():
            out.append(render_move(aw, name, v[0], v[1], None,
                                   f"{site}.{name}"))
    else:
        raise Untranscribable(f"{site}: not a move-list shape")
    return out


def emit_move_tables(cs, aw, axes, tables):
    kinds = set(axes["MonsterKind"])
    kind_index = {k: i for i, k in enumerate(axes["MonsterKind"])}
    out = [""]
    for name, shape, value in tables:
        rust_name = name.lstrip("_")
        site = f"moves:{name}"
        try:
            if shape in ("A", "B"):
                moves = _moves_of(aw, shape, value, site)
                out.append(f"/// `combat_sim.{name}` (shape {shape}), in "
                           f"Python order.")
                out.append("#[rustfmt::skip]")
                out.append(f"pub static {rust_name}: [Move; {len(moves)}] = [")
                out.extend(f"    {m}," for m in moves)
                out.append("];")
                out.append("")
            elif shape == "C":
                rows = []
                for mk, v in sorted(value.items(),
                                    key=lambda kv: kind_index.get(kv[0], -1)):
                    if mk not in kinds:
                        raise Untranscribable(
                            f"{site}: key {mk!r} is not a MonsterKind")
                    move = render_move(aw, v[0], v[1], v[2], None,
                                       f"{site}.{mk}")
                    rows.append(f"    (MonsterKind::{camel(mk)}, {move}),")
                out.append(f"/// `combat_sim.{name}` (shape C), by monster kind.")
                out.append("#[rustfmt::skip]")
                out.append(f"pub static {rust_name}: "
                           f"[(MonsterKind, Move); {len(rows)}] = [")
                out.extend(rows)
                out.append("];")
                out.append("")
            elif shape == "D":
                rows = []
                for mk, sub in sorted(value.items(),
                                      key=lambda kv: kind_index.get(kv[0], -1)):
                    if mk not in kinds:
                        raise Untranscribable(
                            f"{site}: key {mk!r} is not a MonsterKind")
                    moves = _moves_of(aw, classify_move_table(sub), sub,
                                      f"{site}.{mk}")
                    rows.append(f"    (MonsterKind::{camel(mk)}, &[")
                    rows.extend(f"        {m}," for m in moves)
                    rows.append("    ]),")
                out.append(f"/// `combat_sim.{name}` (shape D), ascending by "
                           f"`MonsterKind`.")
                out.append("#[rustfmt::skip]")
                out.append(
                    f"pub static {rust_name}: "
                    f"[(MonsterKind, &[Move]); {len(value)}] = [")
                out.extend(rows)
                out.append("];")
                out.append("")
        except Untranscribable as exc:
            refuse(site, str(exc))
            out.append(f"// REFUSED: {site} — {exc}")
            out.append("")
    return "\n".join(out)


def emit_template_relics(cs, aw, axes):
    relic_index = {r: i for i, r in enumerate(axes["RelicId"])}
    hooks, conds, effs = set(), set(), set()
    rows = []
    for rid, hookmap in sorted(cs.TEMPLATE_RELICS.items(),
                               key=lambda kv: relic_index.get(kv[0], -1)):
        site = f"template_relic:{rid}"
        try:
            if rid not in relic_index:
                raise Untranscribable(f"{site}: not a RelicId")
            body = [f"    (RelicId::{camel(rid)}, &["]
            for hook, rules in sorted(hookmap.items()):
                hooks.add(hook)
                body.append(f"        RelicHook {{ hook: {rust_str(hook)},"
                            f" rules: &[")
                for ri, (cs_, es) in enumerate(rules):
                    cond_lits = []
                    for c in cs_:
                        conds.add(c[0])
                        cond_lits.append(
                            f"Op {{ verb: {rust_str(c[0])}, args: "
                            f"{aw.args(c[1:], f'{site}.{hook}[{ri}].cond')} }}")
                    eff_lits = []
                    for e in es:
                        effs.add(e[0])
                        eff_lits.append(
                            f"Op {{ verb: {rust_str(e[0])}, args: "
                            f"{aw.args(e[1:], f'{site}.{hook}[{ri}].eff', RELIC_EFFECT_ARG_TYPES, e[0])} }}")
                    body.append(
                        f"            RelicRule {{ conds: "
                        f"&[{', '.join(cond_lits)}], effects: "
                        f"&[{', '.join(eff_lits)}] }},")
                body.append("        ] },")
            body.append("    ]),")
            rows.append("\n".join(body))
        except Untranscribable as exc:
            refuse(site, str(exc))
            rows.append(f"    // REFUSED: {site} — {exc}")
    out = ["",
           "/// `combat_sim.TEMPLATE_RELICS`, ascending by `RelicId`.",
           "#[rustfmt::skip]",
           f"pub static TEMPLATE_RELIC_STEPS: "
           f"[(RelicId, &[RelicHook]); {len(rows)}] = ["]
    out.extend(rows)
    out.append("];")
    out.append("")
    for const, values, doc in (
            ("TEMPLATE_RELIC_HOOKS", sorted(hooks),
             "Distinct relic-template hook names."),
            ("TEMPLATE_RELIC_COND_VERBS", sorted(conds),
             "Distinct relic-template condition verbs."),
            ("TEMPLATE_RELIC_EFFECT_VERBS", sorted(effs),
             "Distinct relic-template effect verbs.")):
        out.append(f"/// {doc}")
        out.append("#[rustfmt::skip]")
        out.append(f"pub static {const}: [&str; {len(values)}] = [")
        out.extend(pack(", ".join(rust_str(v) for v in values) + ",", 4))
        out.append("];")
        out.append("")
    return "\n".join(out)


def emit_relic_ledger(cs, axes):
    raw_path = SOLVER_DIR / "relic_templates.json"
    if raw_path.exists():
        with open(raw_path) as f:
            raw_templates = set(json.load(f)["relics"].keys())
    else:
        raw_templates = set()

    known = set(cs.KNOWN_RELICS)
    inert = set(cs.CENSUS_INERT_RELICS)
    template_only = set(cs.TEMPLATE_RELICS.keys())
    hand_active = set(cs._HAND_KNOWN_RELICS) - inert

    assert len(known) == 299, f"expected 299 known relics, got {len(known)}"
    assert len(inert) == 117, f"expected 117 inert relics, got {len(inert)}"
    assert len(template_only) == 18, f"expected 18 template relics, got {len(template_only)}"
    assert len(hand_active) == 164, f"expected 164 hand-active relics, got {len(hand_active)}"
    assert inert.isdisjoint(template_only)
    assert inert.isdisjoint(hand_active)
    assert template_only.isdisjoint(hand_active)
    assert inert | template_only | hand_active == known

    overlap = sorted(raw_templates & hand_active)
    assert len(overlap) == 16, f"expected 16 raw template overlap relics, got {len(overlap)}"
    assert raw_templates == set(overlap) | template_only

    effective_templates_sorted = sorted(template_only)
    hand_active_sorted = sorted(hand_active)

    # Relic batches 1-9: all generated Python template programs,
    # pre-entry hand models whose effects are already represented in canonical
    # v2, and the compact live combat folds. Top and Lamp predate the batches
    # and retain their bespoke engine seams.
    implemented_set = set(template_only) | {
        "RELIC.AKABEKO",
        "RELIC.ANCHOR",
        "RELIC.ART_OF_WAR",
        "RELIC.BAG_OF_MARBLES",
        "RELIC.BAG_OF_PREPARATION",
        "RELIC.BEATING_REMNANT",
        "RELIC.BELLOWS",
        "RELIC.BELT_BUCKLE",
        "RELIC.BLESSED_ANTLER",
        "RELIC.BIG_HAT",
        "RELIC.BIG_MUSHROOM",
        "RELIC.BIIIG_HUG",
        "RELIC.BLOOD_VIAL",
        "RELIC.BONE_FLUTE",
        "RELIC.BONE_TEA",
        "RELIC.BOOKMARK",
        "RELIC.BOOK_REPAIR_KNIFE",
        "RELIC.BOOK_OF_FIVE_RINGS",
        "RELIC.BOOMING_CONCH",
        "RELIC.BOUND_PHYLACTERY",
        "RELIC.BREAD",
        "RELIC.BRILLIANT_SCARF",
        "RELIC.BRIMSTONE",
        "RELIC.BRONZE_SCALES",
        "RELIC.BURNING_STICKS",
        "RELIC.BYRDPIP",
        "RELIC.CANDELABRA",
        "RELIC.CHANDELIER",
        "RELIC.CHARONS_ASHES",
        "RELIC.CHEMICAL_X",
        "RELIC.CENTENNIAL_PUZZLE",
        "RELIC.CHOICES_PARADOX",
        "RELIC.CLAWS",
        "RELIC.CLOAK_CLASP",
        "RELIC.CRACKED_CORE",
        "RELIC.CROSSBOW",
        "RELIC.DAUGHTER_OF_THE_WIND",
        "RELIC.DELICATE_FROND",
        "RELIC.DEMON_TONGUE",
        "RELIC.EMBER_TEA",
        "RELIC.EMOTION_CHIP",
        "RELIC.ETERNAL_FEATHER",
        "RELIC.FAKE_ORICHALCUM",
        "RELIC.FAKE_SNECKO_EYE",
        "RELIC.FAKE_STRIKE_DUMMY",
        "RELIC.FAKE_VENERABLE_TEA_SET",
        "RELIC.FESTIVE_POPPER",
        "RELIC.FENCING_MANUAL",
        "RELIC.FAKE_HAPPY_FLOWER",
        "RELIC.FIDDLE",
        "RELIC.FORGOTTEN_SOUL",
        "RELIC.FUNERARY_MASK",
        "RELIC.FUR_COAT",
        "RELIC.GALACTIC_DUST",
        "RELIC.GAMBLING_CHIP",
        "RELIC.GAME_PIECE",
        "RELIC.GHOST_SEED",
        "RELIC.GIRYA",
        "RELIC.GOLD_PLATED_CABLES",
        "RELIC.GORGET",
        "RELIC.GREMLIN_HORN",
        "RELIC.HAND_DRILL",
        "RELIC.HELICAL_DART",
        "RELIC.HORN_CLEAT",
        "RELIC.HISTORY_COURSE",
        "RELIC.HAPPY_FLOWER",
        "RELIC.ICE_CREAM",
        "RELIC.INFUSED_CORE",
        "RELIC.INTIMIDATING_HELMET",
        "RELIC.IRON_CLUB",
        "RELIC.IVORY_TILE",
        "RELIC.JEWELED_MASK",
        "RELIC.JOSS_PAPER",
        "RELIC.KUNAI",
        "RELIC.KUSARIGAMA",
        "RELIC.LANTERN",
        "RELIC.LARGE_CAPSULE",
        "RELIC.LAVA_LAMP",
        "RELIC.LETTER_OPENER",
        "RELIC.LIZARD_TAIL",
        "RELIC.LOST_WISP",
        "RELIC.MEAL_TICKET",
        "RELIC.MEAT_ON_THE_BONE",
        "RELIC.MINIATURE_CANNON",
        "RELIC.MINI_REGENT",
        "RELIC.METRONOME",
        "RELIC.MR_STRUGGLES",
        "RELIC.MUMMIFIED_HAND",
        "RELIC.MUSIC_BOX",
        "RELIC.MYSTIC_LIGHTER",
        "RELIC.NINJA_SCROLL",
        "RELIC.NUNCHAKU",
        "RELIC.ODDLY_SMOOTH_STONE",
        "RELIC.ORANGE_DOUGH",
        "RELIC.ORICHALCUM",
        "RELIC.ORNAMENTAL_FAN",
        "RELIC.PAELS_FLESH",
        "RELIC.PAELS_EYE",
        "RELIC.PAELS_LEGION",
        "RELIC.PAELS_TEARS",
        "RELIC.PANTOGRAPH",
        "RELIC.PAPER_KRANE",
        "RELIC.PAPER_PHROG",
        "RELIC.PARRYING_SHIELD",
        "RELIC.PEN_NIB",
        "RELIC.PENDULUM",
        "RELIC.PETRIFIED_TOAD",
        "RELIC.PHILOSOPHERS_STONE",
        "RELIC.PHYLACTERY_UNBOUND",
        "RELIC.PERMAFROST",
        "RELIC.PLANISPHERE",
        "RELIC.POCKETWATCH",
        "RELIC.POLLINOUS_CORE",
        "RELIC.POWER_CELL",
        "RELIC.PUMPKIN_CANDLE",
        "RELIC.RADIANT_PEARL",
        "RELIC.RAINBOW_RING",
        "RELIC.RAZOR_TOOTH",
        "RELIC.RED_MASK",
        "RELIC.RED_SKULL",
        "RELIC.REGALITE",
        "RELIC.REPTILE_TRINKET",
        "RELIC.RINGING_TRIANGLE",
        "RELIC.RING_OF_THE_SNAKE",
        "RELIC.RING_OF_THE_DRAKE",
        "RELIC.RIPPLE_BASIN",
        "RELIC.ROYAL_POISON",
        "RELIC.RUINED_HELMET",
        "RELIC.RUNIC_CAPACITOR",
        "RELIC.RUNIC_PYRAMID",
        "RELIC.SCREAMING_FLAGON",
        "RELIC.SEAL_OF_GOLD",
        "RELIC.SELF_FORMING_CLAY",
        "RELIC.SHURIKEN",
        "RELIC.SLING_OF_COURAGE",
        "RELIC.SNECKO_EYE",
        "RELIC.SNECKO_SKULL",
        "RELIC.SPIKED_GAUNTLETS",
        "RELIC.STONE_CALENDAR",
        "RELIC.STONE_CRACKER",
        "RELIC.STRIKE_DUMMY",
        "RELIC.STURDY_CLAMP",
        "RELIC.SYMBIOTIC_VIRUS",
        "RELIC.TEA_OF_DISCOURTESY",
        "RELIC.THE_ABACUS",
        "RELIC.THE_BOOT",
        "RELIC.THROWING_AXE",
        "RELIC.TINGSHA",
        "RELIC.TOASTY_MITTENS",
        "RELIC.TOOLBOX",
        "RELIC.TOUGH_BANDAGES",
        "RELIC.TUNGSTEN_ROD",
        "RELIC.TUNING_FORK",
        "RELIC.TWISTED_FUNNEL",
        "RELIC.UNCEASING_TOP",
        "RELIC.UNDYING_SIGIL",
        "RELIC.UNSETTLING_LAMP",
        "RELIC.VAMBRACE",
        "RELIC.VAJRA",
        "RELIC.VELVET_CHOKER",
        "RELIC.VENERABLE_TEA_SET",
        "RELIC.VEXING_PUZZLEBOX",
        "RELIC.VITRUVIAN_MINION",
        "RELIC.WHISPERING_EARRING",
    }
    assert len(implemented_set) == 182
    assert implemented_set - template_only <= hand_active

    rows = []
    admitted_count = 0
    for r in axes["RelicId"]:
        rid_variant = f"RelicId::{camel(r)}"
        r_name = rust_str(r)
        in_raw = r in raw_templates
        in_eff = r in template_only
        compiled_tmpl = in_eff
        impl_ = r in implemented_set

        if r in inert:
            classification = "RelicClassification::CensusInert"
            admitted = True
        elif r in template_only:
            classification = "RelicClassification::TemplateOnly"
            admitted = impl_
        elif r in hand_active:
            classification = "RelicClassification::HandAuthoredActive"
            admitted = impl_
        else:
            raise SystemExit(f"unclassified relic: {r}")

        if admitted:
            admitted_count += 1

        rows.append(
            f"    RelicLedgerRow {{\n"
            f"        id: {rid_variant},\n"
            f"        name: {r_name},\n"
            f"        classification: {classification},\n"
            f"        in_raw_templates: {'true' if in_raw else 'false'},\n"
            f"        in_effective_templates: {'true' if in_eff else 'false'},\n"
            f"        compiled_template: {'true' if compiled_tmpl else 'false'},\n"
            f"        implemented: {'true' if impl_ else 'false'},\n"
            f"        inventory_represented: true,\n"
            f"        exactly_admitted: {'true' if admitted else 'false'},\n"
            f"    }},"
        )

    expected_admitted_count = len(inert) + len(implemented_set)
    assert admitted_count == expected_admitted_count, (
        f"expected {expected_admitted_count} admitted relics, got {admitted_count}"
    )

    out = [
        "",
        "/// Canonical 299-relic coverage ledger, ascending by `RelicId`.",
        "///",
        "/// Disjoint partition: 117 CensusInert + 18 TemplateOnly + 164 HandAuthoredActive = 299.",
        f"/// Admission partition: 117 CensusInert + {len(implemented_set)} Implemented = "
        f"{expected_admitted_count} admitted; {len(known) - expected_admitted_count} refused.",
        "#[rustfmt::skip]",
        f"pub static RELIC_LEDGER: [RelicLedgerRow; {len(rows)}] = [",
    ]
    out.extend(rows)
    out.append("];")
    out.append("")

    out.append(
        "/// The 16 combat-active relics whose template rules in `relic_templates.json`\n"
        "/// are shadowed by hand-authored modeling in `combat_sim._HAND_KNOWN_RELICS`."
    )
    out.append("#[rustfmt::skip]")
    out.append(f"pub static RAW_TEMPLATE_OVERLAP_RELICS: [RelicId; {len(overlap)}] = [")
    out.extend(pack(", ".join(f"RelicId::{camel(r)}" for r in overlap) + ",", 4))
    out.append("];")
    out.append("")

    out.append(
        "/// The 18 combat-active relics whose rules come solely from `combat_sim.TEMPLATE_RELICS`."
    )
    out.append("#[rustfmt::skip]")
    out.append(f"pub static EFFECTIVE_TEMPLATE_RELICS: [RelicId; {len(effective_templates_sorted)}] = [")
    out.extend(pack(", ".join(f"RelicId::{camel(r)}" for r in effective_templates_sorted) + ",", 4))
    out.append("];")
    out.append("")

    out.append(
        "/// The 164 combat-active relics with hand-authored modeling in `combat_sim._HAND_KNOWN_RELICS`."
    )
    out.append("#[rustfmt::skip]")
    out.append(f"pub static HAND_AUTHORED_ACTIVE_RELICS: [RelicId; {len(hand_active_sorted)}] = [")
    out.extend(pack(", ".join(f"RelicId::{camel(r)}" for r in hand_active_sorted) + ",", 4))
    out.append("];")
    out.append("")

    return "\n".join(out)


def emit_pools(cs, axes, src):
    cards = set(axes["CardId"])
    out = ["", "// --- generation pools "
                "-----------------------------------------------", ""]
    flat: list[str] = []
    # Every pool-named module global except the census blobs, which are
    # schema'd JSON rather than content and are read by the census tests, not by
    # generation (the `CENSUS` in their names is what excludes them). Anything that survives the name filter but is not a card-id
    # container is refused by name rather than silently skipped.
    for name in sorted(n for n in dir(cs)
                       if "POOL" in n and "CENSUS" not in n):
        value = getattr(cs, name)
        if name == "ABUNDANCE_POWER_POOLS_V1101":
            # Abundance/<OnPlay>d__6::MoveNext (v0.111.0 RVA 0x3888ec)
            # narrows to Powers, then GetDistinctForCombat (0x112878) runs
            # FilterForCombat; its predicate (0x3d7042) excludes Basic,
            # Ancient and Event. Preserve the native character-pool order.
            value = {
                owner.title(): tuple(card for card, _epoch in pool
                             if src.card_type(cs, card) == "power"
                             and src.card_rarity(cs, card) not in
                             ("basic", "ancient", "event")
                             and src.card_multiplayer_constraint(cs, card) == "None"
                             and src.card_can_be_generated_in_combat(cs, card))
                for owner, pool in src.character_card_pools(cs).items()
            }
        rust_name = name.lstrip("_")
        site = f"pool:{name}"
        if isinstance(value, (tuple, list, frozenset, set)):
            members = sorted(value) if isinstance(value, (frozenset, set)) \
                else list(value)
            bad = [m for m in members
                   if not isinstance(m, str) or m not in cards]
            if bad:
                refuse(site, f"pool members are not CardIds: {bad[:4]!r}")
                out.append(f"// REFUSED: {site} — members are not CardIds")
                out.append("")
                continue
            out.append(f"/// `combat_sim.{name}`.")
            out.append("#[rustfmt::skip]")
            out.append(f"pub static {rust_name}: [CardId; {len(members)}] = [")
            if members:
                out.extend(pack(", ".join(f"CardId::{camel(m)}"
                                          for m in members) + ",", 4))
            out.append("];")
            out.append("")
            flat.append(rust_name)
            continue
        if isinstance(value, dict):
            ok = all(isinstance(k, str)
                     and isinstance(v, (tuple, list))
                     and all(isinstance(m, str) and m in cards for m in v)
                     for k, v in value.items())
            if not ok:
                refuse(site, "keyed pool values are not tuples of CardIds")
                out.append(f"// REFUSED: {site} — values are not CardId tuples")
                out.append("")
                continue
            if name == "ABUNDANCE_POWER_POOLS_V1101":
                out.append("/// Native Abundance Power candidates after CardFactory.FilterForCombat, ascending by owner.")
            else:
                out.append(f"/// `combat_sim.{name}`, ascending by key.")
            out.append("#[rustfmt::skip]")
            out.append(f"pub static {rust_name}: "
                       f"[(&str, &[CardId]); "
                       f"{len(value)}] = [")
            for k in sorted(value):
                out.append(f"    ({rust_str(k)}, &[")
                if value[k]:
                    out.extend(pack(", ".join(f"CardId::{camel(m)}"
                                              for m in value[k]) + ",", 8))
                out.append("    ]),")
            out.append("];")
            out.append("")
            continue
        refuse(site, f"unsupported pool container {type(value).__name__}")
        out.append(f"// REFUSED: {site} — unsupported container")
        out.append("")

    out.append("/// Index of the flat card-id generation pools above.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static GENERATION_POOLS: "
               f"[(&str, &[CardId]); {len(flat)}] = [")
    for n in flat:
        out.append(f"    ({rust_str(n)}, &{n}),")
    out.append("];")
    out.append("")
    return "\n".join(out), len(flat)



# ---------------------------------------------------------------------------
# The `_card_can_play` identity gates that collapse in the solo boundary
# ---------------------------------------------------------------------------


class _SoloGateShapeChanged(Exception):
    """A Python multiplayer gate no longer has the shape we understand."""


def _named_function(tree, name):
    """Return the unique function named ``name`` or fail closed."""
    matches = [
        node
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == name
    ]
    if len(matches) != 1:
        raise _SoloGateShapeChanged(
            f"found {len(matches)} functions named {name}, expected 1"
        )
    return matches[0]


def _card_identity_test(node):
    """``card[0] in {...}`` / ``card[0] == "X"`` -> the named identities."""
    if not isinstance(node, ast.Compare) or len(node.ops) != 1:
        return None
    left, op, right = node.left, node.ops[0], node.comparators[0]
    if not (isinstance(left, ast.Subscript)
            and isinstance(left.value, ast.Name) and left.value.id == "card"
            and isinstance(left.slice, ast.Constant) and left.slice.value == 0):
        return None
    if isinstance(op, ast.In) and isinstance(right, (ast.Set, ast.Tuple, ast.List)):
        names = [e.value for e in right.elts
                 if isinstance(e, ast.Constant) and isinstance(e.value, str)]
        if len(names) != len(right.elts):
            raise _SoloGateShapeChanged(
                "a card-identity membership set holds a non-string element")
        return names
    if isinstance(op, ast.Eq) and isinstance(right, ast.Constant) \
            and isinstance(right.value, str):
        return [right.value]
    return None


def _is_live_state(node) -> bool:
    """The bare live `State` parameter, not a clone/probe/other creature.

    Both collapses below are sound only for the state the caller is actually
    ruling on. `len(_living_player_keys(probe)) <= 1` over a hypothetical, or
    `not other.teammate_present` over a different creature, are different
    claims -- accepting them would emit a table whose doc comment asserts a
    provenance it does not have.
    """
    return isinstance(node, ast.Name) and node.id == "s"


def _solo_constant_condition(node):
    """Name an exact live solo predicate that Rust evaluates independently.

    The generated identity sets are static, but these predicates are not:
    synthesized multiplayer entries make both relations observable.  The
    receiver and argument arity are therefore checked, not just the callee
    name, so a predicate about a probe cannot be relabelled as one about the
    state being played.
    """
    # `len(_living_player_keys(s)) <= 1` -- solo is always 1, and 0 when the
    # player is dead, so the comparison holds either way.
    if (isinstance(node, ast.Compare) and len(node.ops) == 1
            and isinstance(node.ops[0], ast.LtE)
            and isinstance(node.left, ast.Call)
            and isinstance(node.left.func, ast.Name)
            and node.left.func.id == "len"
            and not node.left.keywords
            and len(node.left.args) == 1
            and isinstance(node.left.args[0], ast.Call)
            and isinstance(node.left.args[0].func, ast.Name)
            and node.left.args[0].func.id == "_living_player_keys"
            and not node.left.args[0].keywords
            and len(node.left.args[0].args) == 1
            and _is_live_state(node.left.args[0].args[0])
            and isinstance(node.comparators[0], ast.Constant)
            and node.comparators[0].value == 1):
        return "len(_living_player_keys(s)) <= 1"
    # `not s.teammate_present` -- the field is inadmissible, so always False.
    if (isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not)
            and isinstance(node.operand, ast.Attribute)
            and node.operand.attr == "teammate_present"
            and _is_live_state(node.operand.value)):
        return "not s.teammate_present"
    return None


def _is_card_zero_compare(node) -> bool:
    """Any comparison whose left side is `card[0]`, whatever the operator."""
    return (isinstance(node, ast.Compare)
            and isinstance(node.left, ast.Subscript)
            and isinstance(node.left.value, ast.Name)
            and node.left.value.id == "card"
            and isinstance(node.left.slice, ast.Constant)
            and node.left.slice.value == 0)


# Return-expression operands that mention `card[0]` but are NOT
# solo-collapsing identity gates. Keyed by exact `ast.unparse` text so a
# rewrite re-opens the classification instead of being silently reabsorbed;
# `unparse` normalises formatting and quoting, so only a semantic edit breaks
# the match. Anything here must be justified, because everything else that
# mentions `card[0]` now raises.
_NON_GATE_CARD_OPERANDS = frozenset({
    # Enthralled is a hand-wide lockout, not an identity gate: the test is
    # `!=`, and its partner is a live Hand scan rather than a multiplayer
    # condition, so it does not collapse to a constant here.
    "card[0] != 'ENTHRALLED' and any((candidate[0] == 'ENTHRALLED'"
    " for candidate in s.hand))",
})


def _classify_solo_gate(operand):
    """`(condition, identities)` if this operand is a solo-collapsing gate.

    Conjunct order is not assumed: Python may legitimately write the live
    condition first. What is required is exactly two conjuncts, one an
    identity test and the other a recognised solo constant.
    """
    if not (isinstance(operand, ast.BoolOp) and isinstance(operand.op, ast.And)
            and len(operand.values) == 2):
        return None
    first, second = operand.values
    for identity_node, condition_node in ((first, second), (second, first)):
        identities = _card_identity_test(identity_node)
        if identities is None:
            continue
        condition = _solo_constant_condition(condition_node)
        if condition is not None:
            return condition, identities
    return None


@_census
def solo_unplayable_census(cs, axes):
    """Card identities `_card_can_play` (frozen Python) always rejects when solo.

    Returns ``{provenance: [CardId names]}``.

    This reads the *returned disjunction* only, never the whole function body:
    `_card_can_play` opens with `card[0] == "TUTOR"` / `card[0] == "HAMMER_TIME"`
    guards that call multiplayer entry-spec validators under
    `len(_living_player_keys(s)) > 1`. Those are the OPPOSITE polarity -- they
    are unreachable when solo -- and sweeping them in here would refuse two
    cards Python happily plays. Restricting to the return expression excludes
    them structurally rather than by name.
    """
    path = SOLVER_DIR / "combat_sim.py"
    tree = ast.parse(path.read_text(), filename=str(path))
    fn = _named_function(tree, "_card_can_play")
    returns = [n for n in ast.walk(fn) if isinstance(n, ast.Return)]
    if len(returns) != 1:
        raise _SoloGateShapeChanged(
            f"_card_can_play has {len(returns)} return statements, expected 1")
    value = returns[0].value
    if not (isinstance(value, ast.UnaryOp) and isinstance(value.op, ast.Not)):
        raise _SoloGateShapeChanged("_card_can_play no longer returns `not (...)`")
    disjunction = value.operand
    if not (isinstance(disjunction, ast.BoolOp)
            and isinstance(disjunction.op, ast.Or)):
        raise _SoloGateShapeChanged("the refusal expression is no longer an `or` chain")

    cards = set(axes["CardId"])
    found: dict[str, list[str]] = {}
    for operand in disjunction.values:
        classified = _classify_solo_gate(operand)
        if classified is not None:
            condition, identities = classified
            unknown = [c for c in identities if c not in cards]
            if unknown:
                raise _SoloGateShapeChanged(
                    f"gate names identities outside the CardId axis: {unknown!r}")
            found.setdefault(condition, []).extend(identities)
            continue
        # FAIL CLOSED. Any operand that mentions a card identity either
        # classifies completely above or stops codegen here. Skipping it would
        # drop a widened gate while the existing membership pin stayed green,
        # which is precisely the silent widening this table exists to catch.
        if (any(_is_card_zero_compare(node) for node in ast.walk(operand))
                and ast.unparse(operand) not in _NON_GATE_CARD_OPERANDS):
            raise _SoloGateShapeChanged(
                "a `_card_can_play` return operand names a card identity but "
                "does not classify as `<identity test> and <solo constant>`: "
                f"{ast.unparse(operand)[:240]!r}. Classify it, or add it to "
                "_NON_GATE_CARD_OPERANDS with a reason, before regenerating.")
    if not found:
        raise _SoloGateShapeChanged("no solo-collapsing identity gate found")
    return {cond: sorted(set(names)) for cond, names in found.items()}


# Python deliberately offers Largesse through `_card_can_play`, then refuses
# it at `_apply_action_impl`'s MultiplayerOnly gate.  Keep the generated
# CanPlay census truthful while making that exceptional source relationship a
# reviewed, fail-closed contract (#1688).
_APPLY_ONLY_MULTIPLAYER_CARDS = frozenset({"LARGESSE"})


def _is_not_implemented_raise(node) -> bool:
    """Whether ``node`` is one direct ``raise NotImplementedError(...)``."""
    return (
        isinstance(node, ast.Raise)
        and isinstance(node.exc, ast.Call)
        and isinstance(node.exc.func, ast.Name)
        and node.exc.func.id == "NotImplementedError"
    )


def _apply_multiplayer_cards(tree, axes):
    """Extract `_apply_action_impl`'s pre-body live-player refusal identities.

    The direct-statement and first-child checks are intentional.  A matching
    set elsewhere in the function, or behind some earlier branch-local work,
    is not evidence for the same application-time refusal point.
    """
    fn = _named_function(tree, "_apply_action_impl")
    candidates = []
    for statement in fn.body:
        if not isinstance(statement, ast.If) or not statement.body:
            continue
        live_gate = statement.body[0]
        if not (
            isinstance(live_gate, ast.If)
            and _solo_constant_condition(live_gate.test)
            == "len(_living_player_keys(s)) <= 1"
        ):
            continue
        if (
            statement.orelse
            or live_gate.orelse
            or len(live_gate.body) != 1
            or not _is_not_implemented_raise(live_gate.body[0])
        ):
            raise _SoloGateShapeChanged(
                "the `_apply_action_impl` multiplayer gate no longer begins "
                "with one direct NotImplementedError refusal"
            )
        identities = _card_identity_test(statement.test)
        if identities is None:
            raise _SoloGateShapeChanged(
                "the `_apply_action_impl` multiplayer refusal is no longer "
                "guarded by `card[0] in {...}`"
            )
        candidates.append(identities)
    if len(candidates) != 1:
        raise _SoloGateShapeChanged(
            "found "
            f"{len(candidates)} `_apply_action_impl` live-player refusal gates, "
            "expected 1"
        )
    cards = set(axes["CardId"])
    unknown = sorted(set(candidates[0]) - cards)
    if unknown:
        raise _SoloGateShapeChanged(
            f"apply gate names identities outside the CardId axis: {unknown!r}"
        )
    # Match Python set semantics. The source currently repeats OUTRAGE, which
    # is inert; membership and the directional delta are the load-bearing
    # facts rather than literal occurrence count.
    return sorted(set(candidates[0]))


@_census
def multiplayer_gate_contract(cs, axes):
    """Return both Python multiplayer sets after checking their named delta."""
    can_play_census = solo_unplayable_census(cs, axes)
    can_play = set(can_play_census["len(_living_player_keys(s)) <= 1"])
    path = SOLVER_DIR / "combat_sim.py"
    tree = ast.parse(path.read_text(), filename=str(path))
    apply = set(_apply_multiplayer_cards(tree, axes))
    apply_only = apply - can_play
    can_play_only = can_play - apply
    if apply_only != _APPLY_ONLY_MULTIPLAYER_CARDS or can_play_only:
        raise _SoloGateShapeChanged(
            "Python multiplayer gates changed: expected apply-only "
            f"{sorted(_APPLY_ONLY_MULTIPLAYER_CARDS)!r} and no can-play-only "
            f"identities, found apply-only {sorted(apply_only)!r} and "
            f"can-play-only {sorted(can_play_only)!r}"
        )
    return {
        "can_play_census": can_play_census,
        "apply": sorted(apply),
        "apply_only": sorted(apply_only),
        "can_play_only": sorted(can_play_only),
    }


_SOLO_GATE_CONSTS = {
    "len(_living_player_keys(s)) <= 1": (
        "MULTIPLAYER_ONLY_CANPLAY_CARDS",
        "Card identities `_card_can_play` (frozen Python, deleted #2827) refuses while the living\n"
        "/// player count is one. The generated identity set is static; the Rust\n"
        "/// play path evaluates this live condition against the admitted party.\n"
        "/// The generator also parses `_apply_action_impl` and refuses unless\n"
        "/// its application-time set is exactly this set plus `LARGESSE`. Python\n"
        "/// deliberately offers Largesse through CanPlay, then refuses its solo\n"
        "/// application, so Largesse does not belong in this CanPlay table.\n"
        "///\n"
        "/// **This is not `STOKE_POOL_EXCLUSIONS_V109[\"multiplayer_only\"]`.**\n"
        "/// That one is a five-member generation-pool exclusion set; this is the\n"
        "/// playability census. They overlap on four names and disagree on the\n"
        "/// rest, so reaching for the wrong one produces a plausible fix that\n"
        "/// does not fix anything (#1613).",
    ),
    "not s.teammate_present": (
        "TEAMMATE_REQUIRED_CANPLAY_CARDS",
        "Card identities `_card_can_play` (frozen Python, deleted #2827) refuses while no teammate is\n"
        "/// present. This remains a separate Python gate from the living-player\n"
        "/// census above, and the Rust play path evaluates its live relation flag\n"
        "/// independently.",
    ),
}


def emit_solo_unplayable(cs, axes):
    census = multiplayer_gate_contract(cs, recorded_axes(axes))["can_play_census"]
    out = ["", "// --- solo-unplayable CanPlay census "
                "----------------------------------", ""]
    emitted = []
    for condition, members in sorted(census.items()):
        if condition not in _SOLO_GATE_CONSTS:
            raise _SoloGateShapeChanged(f"no Rust constant assigned to {condition!r}")
        const, doc = _SOLO_GATE_CONSTS[condition]
        out.append(f"/// {doc}")
        out.append("///")
        out.append(f"/// Python source condition: `{condition}`.")
        out.append("#[rustfmt::skip]")
        out.append(f"pub static {const}: [CardId; {len(members)}] = [")
        out.extend(pack(", ".join(f"CardId::{camel(m)}" for m in members) + ",", 4))
        out.append("];")
        out.append("")
        emitted.append(const)
    out.append("/// Every card identity that is unplayable in the admitted solo")
    out.append("/// boundary, whatever the Python gate that makes it so.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SOLO_UNPLAYABLE_GATES: [(&str, &[CardId]); "
               f"{len(emitted)}] = [")
    for const in emitted:
        out.append(f"    ({rust_str(const)}, &{const}),")
    out.append("];")
    out.append("")
    return "\n".join(out)


#: Modeled `MonsterKind`s the assembly spells as several `MONSTER` classes.
#: Pinned rather than inferred from a name prefix: a prefix rule would sweep
#: `BOWLBUG_EGG`/`BOWLBUG_NECTAR`/`BOWLBUG_ROCK`/`BOWLBUG_SILK` — four
#: independent modeled kinds — under a `BOWLBUG` aggregate that does not
#: exist. Every class behind an aggregate must agree on every fact; a
#: disagreement is a refusal, never a pick.
AGGREGATE_MONSTER_CLASSES = {
    "DECIMILLIPEDE_SEGMENT": ("DECIMILLIPEDE_SEGMENT_FRONT",
                              "DECIMILLIPEDE_SEGMENT_MIDDLE",
                              "DECIMILLIPEDE_SEGMENT_BACK"),
}

#: Pool constants that restate a fact the assembly also carries. They are
#: *checked* here — gate and both tiers (#2539) — so a Python builder
#: constant that drifts away from the build it models fails codegen instead
#: of shipping. Key -> (MONSTER entry, what). Each is a
#: `combat_sim.AscensionTier` (or a band of two) and so is not emitted: the
#: Rust builders read the same fact from `MONSTER_MODELS`.
POOL_CONSTANT_DLL_CHECKS = {
    ("elite", "SEGMENT_HP"): ("DECIMILLIPEDE_SEGMENT", "hp_range"),
    ("elite", "PHROG_HP"): ("PHROG_PARASITE", "hp_range"),
    ("elite", "SKITTISH"): ("PHANTASMAL_GARDENER", "power:SkittishPower"),
    ("elite", "FLAIL_KNIGHT_HP"): ("FLAIL_KNIGHT", "hp_fixed"),
    ("elite", "SPECTRAL_KNIGHT_HP"): ("SPECTRAL_KNIGHT", "hp_fixed"),
    ("elite", "MAGI_KNIGHT_HP"): ("MAGI_KNIGHT", "hp_fixed"),
    ("elite", "MECHA_KNIGHT_HP"): ("MECHA_KNIGHT", "hp_fixed"),
}


def _tier_literal(tier):
    gate = "None" if tier["gate"] is None else f"Some({tier['gate']})"
    return (f"AscensionTier {{ gate: {gate}, "
            f"at_or_above: {tier['at_or_above']}, "
            f"below: {tier['below']} }}")


def _monster_model_rows(cs, axes, src):
    """`MonsterKind` -> the assembly's model facts, or a refusal string."""
    hp_facts = src.monster_initial_hp(cs)
    power_facts = src.monster_initial_powers(cs)
    rows = []
    for kind in axes["MonsterKind"]:
        entries = AGGREGATE_MONSTER_CLASSES.get(kind, (kind,))
        present = [entry for entry in entries if entry in hp_facts]
        if not present:
            rows.append((kind, None, f"no MONSTER class named {entries[0]}"))
            continue
        refused = [hp_facts[e]["refused"] for e in present
                   if "refused" in hp_facts[e]]
        if refused:
            rows.append((kind, None, refused[0]))
            continue
        model = {
            "min_initial_hp": hp_facts[present[0]]["min_initial_hp"],
            "max_initial_hp": hp_facts[present[0]]["max_initial_hp"],
            "initial_powers": power_facts[present[0]]["initial_powers"],
        }
        disagree = [e for e in present[1:]
                    if hp_facts[e]["min_initial_hp"] !=
                    model["min_initial_hp"]
                    or hp_facts[e]["max_initial_hp"] !=
                    model["max_initial_hp"]
                    or power_facts[e]["initial_powers"] !=
                    model["initial_powers"]]
        if disagree:
            rows.append((kind, None,
                         f"the classes behind {kind} disagree: {disagree}"))
            continue
        rows.append((kind, model, None))
    return rows


def emit_monster_models(cs, axes, src):
    """Emit `MONSTER_MODELS` — per-kind initial HP and spawn-time powers.

    DLL-sourced (`dll_content.DLL_SOURCED_FACTS`): `MonsterModel`'s
    `get_MinInitialHp`/`get_MaxInitialHp` and the `Apply<XPower>` call sites of
    each class's `<AfterAddedToRoom>d__N::MoveNext`. This is the table the
    `encounters/` roster builders roll HP and spawn-time power amounts from,
    so that no HP number is ever typed into a Rust file by hand (PORT_PLAN
    §5).

    A class the reader could not decode exactly becomes a refusal row rather
    than a guess (I5). A builder that needs one fails at the boundary, by
    name.
    """
    rows = _monster_model_rows(cs, axes, src)
    refused = [(kind, why) for kind, model, why in rows if model is None]
    out = ["",
           "/// One ascension-tiered native constant.",
           "///",
           "/// `AscensionHelper::GetValueIfAscension(gate, atOrAbove, "
           "below)` is the",
           "/// single shape this build spells a tiered monster constant in; "
           "`gate: None`",
           "/// is an untiered `ldc; ret`, where both fields hold the same "
           "value.",
           "#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]",
           "pub struct AscensionTier {",
           "    /// The ascension at or above which `at_or_above` applies.",
           "    pub gate: Option<u8>,",
           "    /// The value at or above `gate`.",
           "    pub at_or_above: i64,",
           "    /// The value below `gate`.",
           "    pub below: i64,",
           "}",
           "",
           "/// One monster class's initial HP band and spawn-time powers.",
           "#[derive(Copy, Clone, Debug, PartialEq, Eq)]",
           "pub struct MonsterModel {",
           "    /// `get_MinInitialHp`, or `None` when the row is refused.",
           "    pub min_initial_hp: Option<AscensionTier>,",
           "    /// `get_MaxInitialHp`, or `None` when the row is refused.",
           "    pub max_initial_hp: Option<AscensionTier>,",
           "    /// `Apply<XPower>` at `AfterAddedToRoom`, in body order.",
           "    pub initial_powers: &'static [(&'static str, "
           "AscensionTier)],",
           "    /// Why this row carries no facts; empty when it does.",
           "    pub refused: &'static str,",
           "}",
           "",
           "/// Native model facts per `MonsterKind`, indexed by the enum.",
           f"/// {len(rows) - len(refused)} of {len(rows)} kinds are read "
           "exactly; the rest",
           "/// carry their refusal reason and no facts.",
           "#[rustfmt::skip]",
           f"pub static MONSTER_MODELS: [MonsterModel; {len(rows)}] = ["]
    for kind, model, why in rows:
        out.append(f"    // {kind}")
        if model is None:
            out.append("    MonsterModel { min_initial_hp: None, "
                       "max_initial_hp: None, initial_powers: &[],")
            out.append(f"        refused: {rust_str(why)} }},")
            continue
        powers = ", ".join(
            f"({rust_str(name)}, {_tier_literal(tier)})"
            for name, tier in model["initial_powers"])
        out.append("    MonsterModel {")
        out.append("        min_initial_hp: Some("
                   f"{_tier_literal(model['min_initial_hp'])}),")
        out.append("        max_initial_hp: Some("
                   f"{_tier_literal(model['max_initial_hp'])}),")
        out.append(f"        initial_powers: &[{powers}],")
        out.append('        refused: "",')
        out.append("    },")
    out.append("];")
    out.append("")
    return "\n".join(out)


def emit_monster_ctor_ints(cs, axes, src):
    """Emit `MONSTER_CTOR_INTS` — integer field initialisers per kind.

    DLL-sourced (`dll_content.DLL_SOURCED_FACTS["monster.ctor_ints"]`): the
    `ldarg.0; ldc.i4*; stfld` prologue of each monster class's `.ctor`. Only
    kinds on the `MonsterKind` axis whose constructor stores at least one
    integer literal have a row, in axis order, so `monster_ctor_int` can
    binary-search it. A roster builder reads constructor-initialised roster
    state (`TwoTailedRat._turnsUntilSummonable`, `combat_sim.Monster.tus`)
    from here rather than typing the number in (PORT_PLAN §5).
    """
    facts = src.monster_ctor_ints(cs)
    rows = []
    for kind in axes["MonsterKind"]:
        entry = AGGREGATE_MONSTER_CLASSES.get(kind, (kind,))[0]
        stores = facts.get(entry)
        if stores:
            rows.append((kind, sorted(stores.items())))
    out = ["",
           "/// Integer field initialisers each monster class's `.ctor` "
           "stores before its",
           "/// base constructor call, as `(field, value)` sorted by field. "
           "Ascending by",
           "/// `MonsterKind`; only kinds with such a store have a row.",
           "#[rustfmt::skip]",
           "pub static MONSTER_CTOR_INTS: "
           f"[(MonsterKind, &[(&str, i64)]); {len(rows)}] = ["]
    for kind, stores in rows:
        body = ", ".join(f"({rust_str(name)}, {value})"
                         for name, value in stores)
        out.append(f"    (MonsterKind::{camel(kind)}, &[{body}]),")
    out += [
        "];",
        "",
        "/// The integer `kind`'s `.ctor` initialises `field` to, when "
        "`MONSTER_CTOR_INTS`",
        "/// carries it.",
        "pub fn monster_ctor_int(kind: MonsterKind, field: &str) -> "
        "Option<i64> {",
        "    let index = MONSTER_CTOR_INTS",
        "        .binary_search_by_key(&(kind as u16), |(k, _)| *k as u16)",
        "        .ok()?;",
        "    MONSTER_CTOR_INTS[index]",
        "        .1",
        "        .iter()",
        "        .find(|(name, _)| *name == field)",
        "        .map(|(_, value)| *value)",
        "}",
        "",
    ]
    return "\n".join(out)


def _pool_constants(cs):
    """`{pool: {name: (values, is_scalar)}}` for the encounter pool modules.

    The mechanical rule, applied to every `content/encounters/<pool>.py`: a
    module-level name bound to an `int` (not a `bool`) or to a flat tuple of
    `int`s. That captures the per-slot starter tables the roster builders
    read, and captures nothing else — the rotation `*_LOOP` tables are tuples
    of tuples, the kind names are strings, and an ascension-tiered constant
    is a `combat_sim.AscensionTier` (or a band of two), whose value depends on
    the fight (#2539); `MONSTER_MODELS` carries those for Rust.
    """
    return {pool: {name: value for name, (value, tiered) in names.items()
                   if not tiered}
            for pool, names in _pool_module_names(cs).items()}


@_census
def _pool_module_names(cs):
    """`{pool: {name: (value, tiered)}}`: every flat int / int tuple, and
    every `AscensionTier` / tuple of them, bound at a pool module's top."""
    import importlib  # noqa: PLC0415 - local to the generator's Python path

    tier_type = cs.AscensionTier
    pools = importlib.import_module("content.encounters").POOLS
    out = {}
    for pool in pools:
        module = importlib.import_module(f"content.encounters.{pool}")
        found = {}
        for name in sorted(vars(module)):
            if name.startswith("_") or name.upper() != name:
                continue
            value = getattr(module, name)
            if isinstance(value, bool):
                continue
            if isinstance(value, int):
                found[name] = (((value,), True), False)
            elif (isinstance(value, tuple) and value
                  and all(isinstance(v, int) and not isinstance(v, bool)
                          for v in value)):
                found[name] = ((tuple(value), False), False)
            elif type(value) is tier_type or (
                    isinstance(value, tuple) and value
                    and all(type(v) is tier_type for v in value)):
                found[name] = (value, True)
        if found:
            out[pool] = found
    return out


def _tier_fact(tier):
    """A `combat_sim.AscensionTier` in the DLL reader's fact shape."""
    return {"gate": tier.gate, "at_or_above": tier.at_or_above,
            "below": tier.below}


def emit_encounter_pool_constants(cs, src):
    """Emit the encounter pools' flat integer constants as typed Rust consts.

    These are the facts a static assembly read cannot supply as a table — the
    per-slot starter orders live in `GenerateMoveStateMachine` branch
    structure (`dll_content.MODELING_ANNEX`). Lifting them from the Python
    module rather than retyping them is what keeps PORT_PLAN §5's "never a
    hand-transcribed number" true for the procedural builders too.

    Where a constant restates something the assembly *does* carry, it is
    checked against the assembly here rather than merely emitted.
    """
    pools = _pool_constants(cs)
    names = _pool_module_names(cs)
    hp_facts = src.monster_initial_hp(cs)
    power_facts = src.monster_initial_powers(cs)
    for (pool, name), (entry, what) in sorted(
            POOL_CONSTANT_DLL_CHECKS.items()):
        value, tiered = names.get(pool, {}).get(name, (None, False))
        if not tiered:
            raise SystemExit(
                f"{TOOL}: content/encounters/{pool}.py has no AscensionTier "
                f"constant {name}, but POOL_CONSTANT_DLL_CHECKS pins one")
        tiers = value if isinstance(value, tuple) else (value,)
        mine = tuple(_tier_fact(tier) for tier in tiers)
        entries = AGGREGATE_MONSTER_CLASSES.get(entry, (entry,))
        if what in ("hp_range", "hp_fixed"):
            band = hp_facts[entries[0]]
            native = (band["min_initial_hp"], band["max_initial_hp"])
            if what == "hp_fixed":
                if native[0] != native[1]:
                    raise SystemExit(
                        f"{TOOL}: {entries[0]} has the HP band {native}, but "
                        f"content/encounters/{pool}.py {name} is a single "
                        "value")
                native = native[:1]
        else:
            power = what.split(":", 1)[1]
            native = tuple(
                tier
                for pname, tier in power_facts[entries[0]]["initial_powers"]
                if pname == power)
        if mine != native:
            raise SystemExit(
                f"{TOOL}: content/encounters/{pool}.py {name} = "
                f"{mine}, but {entries[0]} says {native} on the "
                "assembly this build is certified against. One of the two is "
                "stale; neither may be resolved here")
    out = ["",
           "/// Flat integer constants lifted from the "
           "`content/encounters/*.py`",
           "/// pool modules — the per-slot starter orders the roster "
           "builders read.",
           "/// Generated, never hand-transcribed (PORT_PLAN §5). "
           "Ascension-tiered HP",
           "/// and spawn amounts are not lifted: they depend on the fight "
           "(#2539), and",
           "/// the builders read them from `MONSTER_MODELS`.",
           "///",
           "/// The starter tables record IL *branch structure* "
           "(`GenerateMoveStateMachine`),",
           "/// which a static table read cannot supply; see "
           "`dll_content.MODELING_ANNEX`.",
           "/// Anything that also exists as an assembly fact is checked "
           "against it at",
           "/// codegen time (`POOL_CONSTANT_DLL_CHECKS`).",
           "pub mod encounter_pool_constants {"]
    for pool in sorted(pools):
        out.append(f"    /// `content/encounters/{pool}.py`.")
        out.append(f"    pub mod {pool} {{")
        for name in sorted(pools[pool]):
            values, scalar = pools[pool][name]
            if scalar:
                out.append(f"        pub const {name}: i64 = {values[0]};")
            else:
                body = ", ".join(str(v) for v in values)
                out.append(f"        pub const {name}: [i64; {len(values)}] "
                           f"= [{body}];")
        out.append("    }")
    out.append("}")
    out.append("")
    return "\n".join(out)


def emit_encounter_rng_draws(src):
    """Emit `ENCOUNTER_RNG_DRAWS` — literal per-fight Encounter-stream bounds.

    DLL-sourced (`dll_content.DLL_SOURCED_FACTS["encounter.rng_draws"]`): each
    `Rng::NextInt` an ENCOUNTER class's own `GenerateMonsters` draws from
    `EncounterModel::get_Rng`, in body order, as `[lo, hi)`. The roster
    builders that spend a numeric draw on that stream read their bounds here,
    so a bound such as `PunchOffEventEncounter`'s `NextInt(2, 10)` is never
    typed into a Rust file (PORT_PLAN §5). A class whose bounds are not
    literals carries its refusal reason and no draws (I5).
    """
    rows = src.encounter_rng_draws(None)
    out = ["",
           "/// One ENCOUNTER class's literal `Rng::NextInt` bounds on the "
           "per-fight",
           "/// Encounter stream, in `GenerateMonsters` body order.",
           "#[derive(Copy, Clone, Debug, PartialEq, Eq)]",
           "pub struct EncounterRngDraws {",
           "    /// The ModelId entry of the ENCOUNTER class.",
           "    pub entry: &'static str,",
           "    /// `[lo, hi)` per draw — `NextInt(max)` is `(0, max)`.",
           "    pub draws: &'static [(i32, i32)],",
           "    /// Why the bounds could not be read; empty when they were.",
           "    pub refused: &'static str,",
           "}",
           "",
           "/// Every ENCOUNTER class whose own `GenerateMonsters` spends a "
           "`NextInt`",
           "/// on `EncounterModel::get_Rng`, ascending by entry. Read out of "
           "the",
           "/// assembly, never hand-transcribed.",
           "#[rustfmt::skip]",
           "pub static ENCOUNTER_RNG_DRAWS: "
           f"[EncounterRngDraws; {len(rows)}] = ["]
    for entry in sorted(rows):
        row = rows[entry]
        if "refused" in row:
            out.append(f"    EncounterRngDraws {{ entry: {rust_str(entry)}, "
                       "draws: &[], refused: "
                       f"{rust_str(row['refused'])} }},")
            continue
        draws = ", ".join(f"({lo}, {hi})" for lo, hi in row["draws"])
        out.append(f"    EncounterRngDraws {{ entry: {rust_str(entry)}, "
                   f'draws: &[{draws}], refused: "" }},')
    out.append("];")
    out.append("")
    return "\n".join(out)


#: Where the E4b roster builders live, and the name one must have to be
#: dispatched: `ENCOUNTER.SKULKING_COLONY` -> `build_skulking_colony`.
ENCOUNTERS_DIR = RUST_DIR / "src" / "encounters"
ROSTER_BUILDER_RE = re.compile(r"^pub fn (build_[a-z0-9_]+)\(", re.M)


def emit_encounter_roster_builders(cs, axes):
    """Emit `ENCOUNTER_ROSTER_BUILDERS` — the E4b dispatch table.

    Derived by *looking*: every `content/encounters` registration key whose
    Rust counterpart `build_<key lowercased>` is defined under
    `src/encounters/`. That is what makes the family wave PRs additive — each
    family adds its own module and regenerates, rather than hand-editing one
    shared registry (PORT_PLAN §4, and the E4b per-family contract).

    `make_monsters` dispatches by **substring**, so the key is not the wire
    id: `ENCOUNTER.DECIMILLIPEDE_ELITE` is built by the builder registered
    under `DECIMILLIPEDE`. The `EncounterId` axis is the registered keys, so
    the mapping here is one-to-one and the substring rule lives in the engine
    half (#2528), not in this table.
    """
    if not ENCOUNTERS_DIR.is_dir():
        return ""
    defined = {}
    for path in sorted(ENCOUNTERS_DIR.glob("*.rs")):
        if path.name == "mod.rs":
            continue
        for name in ROSTER_BUILDER_RE.findall(path.read_text()):
            if name in defined:
                raise SystemExit(
                    f"{TOOL}: {name} is defined in both "
                    f"{defined[name]} and {path.name}")
            defined[name] = path.stem
    rows = []
    for key in cs.SUPPORTED_ENCOUNTERS:
        fn = "build_" + key.rsplit(".", 1)[-1].lower()
        if fn in defined:
            rows.append((key, defined.pop(fn), fn))
    if defined:
        raise SystemExit(
            f"{TOOL}: src/encounters/ defines {sorted(defined)}, which no "
            "registered encounter key dispatches to")
    # Ascending by enum discriminant, which is the `EncounterId` axis order —
    # what `binary_search_by_key` below relies on.
    order = {key: index for index, key in enumerate(axes["EncounterId"])}
    rows.sort(key=lambda row: order[row[0]])
    out = ["",
           "/// Every `EncounterId` the crate can build a roster for, "
           "ascending by",
           f"/// variant — {len(rows)} of {len(axes['EncounterId'])} today. "
           "The row set is",
           "/// derived by looking for `build_<registered key>` under "
           "`src/encounters/`,",
           "/// so a family wave PR adds its module and regenerates rather "
           "than",
           "/// hand-editing one shared registry.",
           "///",
           "/// `RosterBuilder` and the lookup live in `crate::encounters`; "
           "the engine",
           "/// half — `make_monsters`' substring dispatch, the "
           "`_EncounterCtx`",
           "/// equivalent and the `HotMonster` admission edge — is #2528's.",
           "#[rustfmt::skip]",
           "pub static ENCOUNTER_ROSTER_BUILDERS: "
           f"[(EncounterId, crate::encounters::RosterBuilder); "
           f"{len(rows)}] = ["]
    for key, module, fn in rows:
        out.append(f"    (EncounterId::{camel(key)}, "
                   f"crate::encounters::{module}::{fn}),")
    out.append("];")
    out.append("")
    return "\n".join(out)


def emit_splash_epochs(cs, axes, src):
    """Emit the unlock-epoch universe and the Splash pool census (#2469).

    Three sources, all current-build and all checked rather than trusted:

    * the unlock-epoch universe — ``EpochModel.AllEpochs``. Under
      ``--source python`` this is read from ``mcr_tables.json["epochs"]``,
      which ``solver/tools/build_mcr_tables.py`` derived from the archived
      ``sts2.dll``; under ``--source dll`` it is read from the assembly
      directly and reconciled against that file (#2496). Either way it is the
      closed universe a projected ``player.splash_unlock_epochs`` tuple may
      draw from.
    * ``combat_sim._CARD_POOL_CENSUS["character_pool_order"]`` and
      ``["character_unlock_epochs"]`` — what ``_derive_splash_attack_pool``
      tests for character-pool inclusion.
    * ``["pools"][character]["cards"]`` — the per-character rows, carrying the
      per-row ``unlock_epoch`` the same derivation filters on.

    The four *static* row predicates (``type``, ``rarity``,
    ``multiplayer_constraint``, ``can_be_generated_in_combat``) are applied
    here, at codegen time, because they cannot vary with the profile; only the
    epoch predicate is left for the engine. Python applies all five in one
    pass, and ``continue`` order does not change the resulting set, so the
    split is exact. The row-shape checks ``_derive_splash_attack_pool`` makes
    at runtime (dense ``index``, ``CARD.`` prefix) are made here instead, and
    a drift raises rather than silently dropping a row (I5).
    """
    cards = set(axes["CardId"])
    census = cs._CARD_POOL_CENSUS
    build = cs._CURRENT_POOL_CENSUS_BUILD
    universe, tables = src.epoch_universe(cs, SOLVER_DIR)
    if (tables.get("game_version") != build["version"]
            or tables.get("game_commit") != build["commit"]
            or build["sts2_dll_sha256"] not in (
                tables.get("game_version_hint") or "")):
        raise SystemExit(
            "mcr_tables.json provenance does not match "
            "combat_sim._CURRENT_POOL_CENSUS_BUILD: "
            f"{tables.get('game_version')!r}/{tables.get('game_commit')!r}")
    if (not isinstance(universe, list)
            or any(not isinstance(e, str) or not e or e.startswith("EPOCH.")
                   for e in universe)
            or sorted(set(universe)) != universe):
        raise SystemExit(
            f"the unlock-epoch universe is not normalized: {universe!r}")
    order = census["character_pool_order"]
    if order != ["IRONCLAD", "SILENT", "REGENT", "NECROBINDER", "DEFECT"]:
        raise SystemExit(f"card-pool census order drifted: {order!r}")
    unlock_epochs = census["character_unlock_epochs"]
    firsts = []
    for character in order:
        epochs = unlock_epochs.get(character)
        if (not isinstance(epochs, list) or not epochs
                or any(e not in universe for e in epochs)):
            raise SystemExit(
                f"character unlock epochs drifted for {character}: "
                f"{epochs!r}")
        firsts.append(epochs[0])

    rows_by_character = []
    for character in order:
        rows = census.get("pools", {}).get(character, {}).get("cards")
        if not isinstance(rows, list):
            raise SystemExit(f"card-pool census lacks {character} rows")
        selected = []
        for index, row in enumerate(rows):
            if (not isinstance(row, dict)
                    or row.get("index") != index
                    or not isinstance(row.get("id"), str)
                    or not row["id"].startswith("CARD.")):
                raise SystemExit(
                    f"malformed Splash census row {character}/{index}: "
                    f"{row!r}")
            unlock_epoch = row.get("unlock_epoch")
            if unlock_epoch is not None and unlock_epoch not in universe:
                raise SystemExit(
                    f"Splash census row {character}/{index} carries an epoch "
                    f"outside the build universe: {unlock_epoch!r}")
            if (row.get("multiplayer_constraint") != "None"
                    or row.get("type") != "Attack"
                    or row.get("can_be_generated_in_combat") is not True
                    or row.get("rarity") not in {"Common", "Uncommon", "Rare"}):
                continue
            card_id = row["id"].removeprefix("CARD.")
            if card_id not in cards:
                raise SystemExit(
                    f"Splash census row {character}/{index} is not a known "
                    f"CardId: {card_id!r}")
            selected.append((card_id, unlock_epoch))
        rows_by_character.append((character, selected))

    out = ["", "// --- unlock epochs and the Splash character pool census "
                "-------------------", ""]
    out.append("/// `EpochModel.AllEpochs` for the current build, as extracted")
    out.append("/// by `versions/v0.111.0/solver/tools/build_mcr_tables.py` into")
    out.append("/// `versions/v0.111.0/solver/mcr_tables.json[\"epochs\"]`,")
    out.append("/// ascending.")
    out.append("///")
    out.append("/// This is the closed universe a projected")
    out.append("/// `player.splash_unlock_epochs` tuple may draw from; an")
    out.append("/// unrecognized string is refused rather than tolerated (I5).")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static UNLOCK_EPOCH_UNIVERSE_V1101: "
               f"[&str; {len(universe)}] = [")
    out.extend(pack(", ".join(rust_str(e) for e in universe) + ",", 4))
    out.append("];")
    out.append("")
    out.append("/// `combat_sim._CARD_POOL_CENSUS[\"character_pool_order\"]` —")
    out.append("/// the native `CharacterCardPools` order that")
    out.append("/// `_derive_splash_attack_pool` preserves after the owner")
    out.append("/// removal. NOT sorted: this order is load-bearing.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SPLASH_CHARACTER_POOL_ORDER_V1101: "
               f"[&str; {len(order)}] = [")
    out.extend(pack(", ".join(rust_str(c) for c in order) + ",", 4))
    out.append("];")
    out.append("")
    out.append("/// `character_unlock_epochs[character][0]`, in")
    out.append("/// `SPLASH_CHARACTER_POOL_ORDER_V1101` order. This single")
    out.append("/// epoch is the membership test `_derive_splash_attack_pool`")
    out.append("/// makes when deciding whether a non-owner character pool")
    out.append("/// joins the concatenation.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SPLASH_CHARACTER_FIRST_UNLOCK_EPOCH_V1101: "
               f"[&str; {len(firsts)}] = [")
    out.extend(pack(", ".join(rust_str(e) for e in firsts) + ",", 4))
    out.append("];")
    out.append("")
    out.append("/// Splash-eligible rows per character, in census row order,")
    out.append("/// each paired with the row's `unlock_epoch` (`None` = the")
    out.append("/// row is unconditionally unlocked).")
    out.append("///")
    out.append("/// Rows are pre-filtered by the four profile-independent")
    out.append("/// `_derive_splash_attack_pool` predicates —")
    out.append("/// `multiplayer_constraint == \"None\"`, `type == \"Attack\"`,")
    out.append("/// `can_be_generated_in_combat`, and a")
    out.append("/// Common/Uncommon/Rare rarity. Only the epoch predicate")
    out.append("/// varies with the profile, so only it is left to the engine.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SPLASH_CHARACTER_ATTACK_ROWS_V1101: "
               f"[(&str, &[(CardId, Option<&str>)]); "
               f"{len(rows_by_character)}] = [")
    for character, selected in rows_by_character:
        out.append(f"    ({rust_str(character)}, &[")
        if selected:
            out.extend(pack(", ".join(
                f"(CardId::{camel(card_id)}, "
                + ("None" if epoch is None else f"Some({rust_str(epoch)})")
                + ")" for card_id, epoch in selected) + ",", 8))
        out.append("    ]),")
    out.append("];")
    out.append("")
    return "\n".join(out)


def emit_potion_pools(cs, axes, src):
    """Native owner-first, shared-second factory order, before rarity filtering."""
    pools = src.character_potion_pools(cs)
    shared = src.shared_potion_pool(cs)
    universe, _ = src.epoch_universe(cs, SOLVER_DIR)
    known = set(axes["PotionId"])
    out = ["", "/// PotionFactory::GetPotionOptions (v111 RVA 0x112fba): owner pool",
           "/// followed by SharedPotionPool, preserving native order. Each row",
           "/// carries rarity and CanBeGeneratedInCombat; these fully-unlocked",
           "/// tables may only be used with an authenticated complete profile.",
           "#[rustfmt::skip]",
           "pub static CHARACTER_POTION_POOL_ROWS: [(&str, &[(PotionId, u8, bool)]); 5] = ["]
    for character in src.character_pool_order(cs):
        own = pools[character]
        if own["epoch"] not in universe:
            raise SystemExit(f"unknown potion epoch: {own!r}")
        rows = [(p, own["epoch"]) for p in own["potions"]] + shared
        if len({p for p, _ in rows}) != len(rows):
            raise SystemExit(f"duplicate potion in {character} pool")
        out.append(f"    ({rust_str(character)}, &[")
        for potion, epoch in rows:
            if potion not in known or (epoch is not None and epoch not in universe):
                raise SystemExit(f"unknown potion/epoch: {potion}/{epoch}")
            rarity = {"Common": 1, "Uncommon": 2, "Rare": 3}.get(src.potion_rarity(cs, potion))
            if rarity not in (1, 2, 3):
                raise SystemExit(f"unsupported potion rarity: {potion}/{rarity}")
            combat = str(src.potion_can_be_generated_in_combat(cs, potion)).lower()
            out.append(f"        (PotionId::{camel(potion)}, {rarity}, {combat}),")
        out.append("    ]),")
    out.append("];\n")
    # #3343: the unlock epoch gating each row above, row for row. Additive
    # rather than a widened tuple, so every existing reader of
    # `CHARACTER_POTION_POOL_ROWS` is untouched.
    out += ["/// The unlock epoch gating each `CHARACTER_POTION_POOL_ROWS` row, in",
            "/// the same order (#3343). An owner row is gated whole by its pool's",
            "/// `<Character>PotionPool::GetUnlockedPotions` single `IsEpochRevealed`",
            "/// test (e.g. Ironclad `0xadd38`); a shared row by",
            "/// `SharedPotionPool::GetUnlockedPotions` `0xadfb4`'s per-epoch",
            "/// removals; `None` is ungated.",
            "#[rustfmt::skip]",
            "pub static CHARACTER_POTION_POOL_ROW_EPOCHS: "
            "[(&str, &[Option<&str>]); 5] = ["]
    for character in src.character_pool_order(cs):
        own = pools[character]
        rows = [(p, own["epoch"]) for p in own["potions"]] + shared
        out.append(f"    ({rust_str(character)}, &[")
        for potion, epoch in rows:
            out.append("        " + ("None" if epoch is None
                                     else f"Some({rust_str(epoch)})") + ",")
        out.append("    ]),")
    out.append("];\n")
    return "\n".join(out)


def emit_character_pools(cs, axes, src):
    """Emit the five generation card pools, whole (#2542).

    `emit_splash_epochs` above emits the same rows narrowed to Splash's
    predicate — Attack, Common/Uncommon/Rare — because Splash was the only
    generator that had ever needed a profile-derived pool. Every other
    generator reached for a frozen `*_POOL_V110*` constant instead, which is
    why Necrobinder, Defect and Silent had no pool at all and 79 real fights
    refused with `requires an explicit <Character> owner`.

    This table is the *whole* pool per character, so every generator's
    projection is derivable from it:

    * membership and order are `<Character>CardPool::GenerateAllCards`, the
      literal `ModelDb.Card<T>()` array. The order is RNG-significant — a
      full `UnstableShuffle` or an `Rng.NextItem` index over this exact
      sequence is what the game does — so it is preserved, never sorted.
    * each row's `unlock_epoch` is the epoch whose `Cards` list
      `FilterThroughEpochs` removes when that epoch is not revealed, or
      `None` for an unconditionally unlocked row.

    Two predicates are applied here, at codegen time, because they are
    profile-independent AND universal — every combat generation path in the
    game runs `CardFactory.FilterForPlayerCount` and `FilterForCombat`:
    `MultiplayerConstraint == None` and `CanBeGeneratedInCombat`. The
    rarity/type/cost predicates are *not* applied, because they differ per
    generator (Abundance takes Powers excluding Basic/Ancient/Event;
    the generation potions take Common/Uncommon/Rare only), and the engine
    has every row's rarity, type and cost in `CARD_ROWS` already.
    """
    cards = set(axes["CardId"])
    order = src.character_pool_order(cs)
    unlock_epochs = src.character_unlock_epochs(cs)
    pools = src.character_card_pools(cs)
    universe, _tables = src.epoch_universe(cs, SOLVER_DIR)
    if sorted(pools) != sorted(order) or sorted(unlock_epochs) != sorted(order):
        raise SystemExit(
            f"the character pools, order and epochs disagree: {sorted(pools)!r}"
            f"/{order!r}/{sorted(unlock_epochs)!r}")

    rows_by_character = []
    membership_by_character = []
    for character in order:
        epochs = unlock_epochs[character]
        if not epochs or any(epoch not in universe for epoch in epochs):
            raise SystemExit(
                f"{character} unlock epochs are outside the build universe: "
                f"{epochs!r}")
        selected = []
        members = []
        for card_id, epoch in pools[character]:
            if card_id not in cards:
                raise SystemExit(
                    f"{character} pool row {card_id!r} is not a known CardId")
            if epoch is not None and epoch not in universe:
                raise SystemExit(
                    f"{character} pool row {card_id!r} carries an epoch "
                    f"outside the build universe: {epoch!r}")
            members.append(card_id)
            if (src.card_multiplayer_constraint(cs, card_id) != "None"
                    or not src.card_can_be_generated_in_combat(cs, card_id)):
                continue
            selected.append((card_id, epoch))
        if len(selected) != len(set(card for card, _ in selected)):
            raise SystemExit(f"{character} pool repeats a card")
        if len(members) != len(set(members)):
            raise SystemExit(f"{character} pool repeats a card")
        rows_by_character.append((character, selected))
        membership_by_character.append((character, members))

    # `CardModel::get_Pool` is `ModelDb.AllCardPools.FirstOrDefault(p =>
    # p.AllCardIds.Contains(id))`, so a card may belong to at most one pool
    # without the answer depending on `AllCardPools` order. The five character
    # pools and the Colorless pool must therefore be pairwise disjoint for the
    # membership table below to BE that lookup; the generator proves it rather
    # than the engine assuming it.
    seen = {}
    colorless_ids = [card_id for card_id, _epoch in src.colorless_card_pool(cs)]
    # #2734: the six other `ModelDb.AllSharedCardPools` members join the
    # disjointness proof, so `get_Pool` over the WHOLE `AllCardPools` list is
    # order-independent, not just over the six pools read before them.
    shared_pools = src.shared_card_pools(cs)
    for character, members in list(membership_by_character) + [
            ("COLORLESS", colorless_ids)] + [
            (pool, [row[0] for row in rows]) for pool, rows in shared_pools]:
        for card_id in members:
            if card_id in seen:
                raise SystemExit(
                    f"{card_id!r} is in both the {seen[card_id]} and "
                    f"{character} pools, so `CardModel::get_Pool` depends on "
                    f"`ModelDb.AllCardPools` order")
            seen[card_id] = character

    out = ["", "// --- the five character generation card pools (#2542) "
                "-------------", ""]
    out.append("/// `ModelDb.AllCharacters` order — the order")
    out.append("/// `UnlockState::get_CharacterCardPools` preserves, and the")
    out.append("/// key order of the two tables below. NOT sorted.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static CHARACTER_POOL_ORDER_V1101: "
               f"[&str; {len(order)}] = [")
    out.extend(pack(", ".join(rust_str(c) for c in order) + ",", 4))
    out.append("];")
    out.append("")
    out.append("/// Every unlock epoch belonging to each character, ascending.")
    out.append("///")
    out.append("/// The union across all five is the \"fully unlocked")
    out.append("/// character epochs\" set every legacy generator required")
    out.append("/// outright; a recorded profile that is a strict subset now")
    out.append("/// derives a smaller pool from")
    out.append("/// `CHARACTER_CARD_POOL_ROWS_V1101` instead of refusing.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static CHARACTER_UNLOCK_EPOCHS_V1101: "
               f"[(&str, &[&str]); {len(order)}] = [")
    for character in order:
        members = ", ".join(rust_str(e) for e in unlock_epochs[character])
        out.append(f"    ({rust_str(character)}, &[{members}]),")
    out.append("];")
    out.append("")
    out.append("/// Each character pool's complete MEMBERSHIP, in native")
    out.append("/// `<Character>CardPool::GenerateAllCards` order.")
    out.append("///")
    out.append("/// This is `CardPoolModel::get_AllCardIds` (RVA `0x7e4dc` =")
    out.append("/// `get_AllCards().Select(Id).ToHashSet()`, and `get_AllCards`")
    out.append("/// RVA `0x7e4a6` is `GenerateAllCards`), which is the set")
    out.append("/// `CardModel::get_Pool` (RVA `0x7c878` IL_001b–IL_002d)")
    out.append("/// searches to answer WHICH POOL A CARD BELONGS TO. Nothing")
    out.append("/// is filtered: `Basic`, non-`CanBeGeneratedInCombat` and")
    out.append("/// `MultiplayerConstraint != None` rows are all members, and")
    out.append("/// `CardFactory::GetDefaultTransformationOptions` (RVA")
    out.append("/// `0x112960` IL_0041–IL_0047) reads `original.Pool` for")
    out.append("/// every non-Quest, non-`{Ancient, Event, Token}` original —")
    out.append("/// the generable/constraint/epoch predicates apply to the")
    out.append("/// CANDIDATES it then draws (`GetUnlockedCards` at")
    out.append("/// IL_005f, `GetFilteredTransformationOptions` RVA")
    out.append("/// `0x112a30` IL_005f–IL_0087 and IL_00a0–IL_00b6), never to")
    out.append("/// the original. Use `CHARACTER_CARD_POOL_ROWS_V1101` below")
    out.append("/// for candidates and this table for origins.")
    out.append("///")
    out.append("/// The generator proves these five and the Colorless pool are")
    out.append("/// pairwise disjoint, so the lookup does not depend on")
    out.append("/// `ModelDb.AllCardPools` order.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static CHARACTER_CARD_POOL_MEMBERSHIP_V1101: "
               f"[(&str, &[CardId]); {len(membership_by_character)}] = [")
    for character, members in membership_by_character:
        out.append(f"    ({rust_str(character)}, &[")
        if members:
            out.extend(pack(", ".join(
                f"CardId::{camel(card_id)}" for card_id in members) + ",", 8))
        out.append("    ]),")
    out.append("];")
    out.append("")
    out.append("/// Each character's complete combat-generation card pool, in")
    out.append("/// native `GenerateAllCards` order, paired with the row's")
    out.append("/// `unlock_epoch` (`None` = unconditionally unlocked).")
    out.append("///")
    out.append("/// Pre-filtered by the two universal profile-independent")
    out.append("/// predicates only — `MultiplayerConstraint == None`")
    out.append("/// (`CardFactory::FilterForPlayerCount`) and")
    out.append("/// `CanBeGeneratedInCombat` (`FilterForCombat`). Rarity,")
    out.append("/// type and cost are per-generator and live on `CARD_ROWS`.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static CHARACTER_CARD_POOL_ROWS_V1101: "
               f"[(&str, &[(CardId, Option<&str>)]); "
               f"{len(rows_by_character)}] = [")
    for character, selected in rows_by_character:
        out.append(f"    ({rust_str(character)}, &[")
        if selected:
            out.extend(pack(", ".join(
                f"(CardId::{camel(card_id)}, "
                + ("None" if epoch is None else f"Some({rust_str(epoch)})")
                + ")" for card_id, epoch in selected) + ",", 8))
        out.append("    ]),")
    out.append("];")
    out.append("")

    # --- the one Colorless pool (#2512) ---------------------------------
    colorless = src.colorless_card_pool(cs)
    selected = []
    for card_id, epoch in colorless:
        if card_id not in cards:
            raise SystemExit(
                f"Colorless pool row {card_id!r} is not a known CardId")
        if epoch is not None and epoch not in universe:
            raise SystemExit(
                f"Colorless pool row {card_id!r} carries an epoch outside "
                f"the build universe: {epoch!r}")
        if not src.card_can_be_generated_in_combat(cs, card_id):
            continue
        selected.append(
            (card_id, epoch,
             src.card_multiplayer_constraint(cs, card_id) == "None"))
    if len(selected) != len(set(card for card, _e, _s in selected)):
        raise SystemExit("the Colorless pool repeats a card")
    out.append("/// The Colorless generation pool, in native")
    out.append("/// `ColorlessCardPool::GenerateAllCards` 0xf11a0 order,")
    out.append("/// paired with the row's `unlock_epoch` "
               "(`FilterThroughEpochs`")
    out.append("/// 0xf13f4, `None` = unconditionally unlocked) and whether")
    out.append("/// a SOLO game keeps it.")
    out.append("///")
    out.append("/// Unlike the character pools this table keeps the")
    out.append("/// multiplayer rows and flags them, because Largesse reads")
    out.append("/// the pool WITH them while every solo generator reads it")
    out.append("/// without: `CardPoolModel::GetUnlockedCards` 0x7e54c")
    out.append("/// removes `MultiplayerConstraint == 2` only when")
    out.append("/// `playerCount == 1`. `CanBeGeneratedInCombat` IS applied")
    out.append("/// here - `GetDistinctForCombat` 0x2be804 drops those three")
    out.append("/// rows on every path.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static COLORLESS_CARD_POOL_ROWS_V1101: "
               f"[(CardId, Option<&str>, bool); {len(selected)}] = [")
    out.extend(pack(", ".join(
        f"(CardId::{camel(card_id)}, "
        + ("None" if epoch is None else f"Some({rust_str(epoch)})")
        + f", {'true' if solo else 'false'})"
        for card_id, epoch, solo in selected) + ",", 4))
    out.append("];")
    out.append("")

    # --- the six other shared pools (#2734) -----------------------------
    for pool, rows in shared_pools:
        for card_id, _constraint, _in_combat in rows:
            if card_id not in cards:
                raise SystemExit(
                    f"{pool} pool row {card_id!r} is not a known CardId")
    out.append("/// The six non-Colorless `ModelDb::get_AllSharedCardPools`")
    out.append("/// (RVA `0x80e58`) members, in that literal array order, each")
    out.append("/// with its complete MEMBERSHIP in native `GenerateAllCards`")
    out.append("/// order (#2734). `CardModel::get_Pool` (RVA `0x7c878`)")
    out.append("/// searches these after the character pools, so this is where")
    out.append("/// a Shiv, a Regret or a Burn BELONGS. None of the six")
    out.append("/// overrides `FilterThroughEpochs`, so no row has an epoch.")
    out.append("/// The generator proves them disjoint from every other pool.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SHARED_CARD_POOL_MEMBERSHIP_V1110: "
               f"[(&str, &[CardId]); {len(shared_pools)}] = [")
    for pool, rows in shared_pools:
        out.append(f"    ({rust_str(pool)}, &[")
        out.extend(pack(", ".join(
            f"CardId::{camel(row[0])}" for row in rows) + ",", 8))
        out.append("    ]),")
    out.append("];")
    out.append("")
    out.append("/// `ColorlessCardPool::GenerateAllCards` 0xf11a0's complete")
    out.append("/// MEMBERSHIP, native order (#2734): the `get_Pool` answer for")
    out.append("/// a Colorless ORIGIN, which `COLORLESS_CARD_POOL_ROWS_V1101`")
    out.append("/// cannot give because it drops the rows that are not")
    out.append("/// `CanBeGeneratedInCombat` — legal origins all the same.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static COLORLESS_CARD_POOL_MEMBERSHIP_V1110: "
               f"[CardId; {len(colorless_ids)}] = [")
    out.extend(pack(", ".join(
        f"CardId::{camel(card_id)}" for card_id in colorless_ids) + ",", 4))
    out.append("];")
    out.append("")
    out.append("/// `SHARED_CARD_POOL_MEMBERSHIP_V1110` pre-filtered the way")
    out.append("/// `CHARACTER_CARD_POOL_ROWS_V1101` is: `MultiplayerConstraint")
    out.append("/// == None` (the solo `GetUnlockedCards` / `FilterForPlayerCount`")
    out.append("/// removal) and `CanBeGeneratedInCombat`. Native order. Rarity")
    out.append("/// is per-consumer and lives on `CARD_ROWS`.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub static SHARED_CARD_POOL_ROWS_V1110: "
               f"[(&str, &[CardId]); {len(shared_pools)}] = [")
    for pool, rows in shared_pools:
        kept = [card_id for card_id, constraint, in_combat in rows
                if constraint == "None" and in_combat]
        out.append(f"    ({rust_str(pool)}, &[")
        if kept:
            out.extend(pack(", ".join(
                f"CardId::{camel(card_id)}" for card_id in kept) + ",", 8))
        out.append("    ]),")
    out.append("];")
    out.append("")
    return "\n".join(out)


def emit_misc(cs, axes):
    inert = sorted(cs.CENSUS_INERT_RELICS)
    out = ["",
           "/// `combat_sim.CENSUS_INERT_RELICS` — known relic ids with no",
           "/// combat-active behaviour, ascending.",
           "#[rustfmt::skip]",
           f"pub static CENSUS_INERT_RELICS: [RelicId; {len(inert)}] = ["]
    out.extend(pack(", ".join(f"RelicId::{camel(r)}" for r in inert) + ",", 4))
    out.append("];")
    out.append("")
    encounters = axes["EncounterId"]
    out.append("/// `combat_sim.SUPPORTED_ENCOUNTERS`, in Python registration")
    out.append("/// order — the order `ENCOUNTER_BUILDERS` is matched in.")
    out.append("#[rustfmt::skip]")
    order = [e for e in cs.SUPPORTED_ENCOUNTERS]
    assert sorted(order) == encounters
    out.append(f"pub static ENCOUNTER_MATCH_ORDER: "
               f"[EncounterId; {len(order)}] = [")
    out.extend(pack(", ".join(f"EncounterId::{camel(e)}"
                              for e in order) + ",", 4))
    out.append("];")
    out.append("")
    for const, enum, values, doc in (
            ("KNOWN_POTIONS", "PotionId", axes["PotionId"],
             "`combat_sim.KNOWN_POTIONS`, ascending."),
            ("KNOWN_ENCHANTMENTS", "EnchantmentId", axes["EnchantmentId"],
             "`combat_sim.KNOWN_ENCHANTMENTS`, ascending.")):
        out.append(f"/// {doc}")
        out.append("#[rustfmt::skip]")
        out.append(f"pub static {const}: [{enum}; {len(values)}] = [")
        out.extend(pack(", ".join(f"{enum}::{camel(v)}"
                                  for v in values) + ",", 4))
        out.append("];")
        out.append("")
    return "\n".join(out)


def emit_refusals():
    dyn = sorted(DYNAMIC_SITES)
    out = ["",
           "/// `Monster(...)` construction sites whose kind is a runtime",
           "/// value, as `(site, note)`. Not refusals: `MonsterKind` is a",
           "/// union of several sources and every kind reaching these sites",
           "/// is enumerated by the move-table sources. Published so the",
           "/// coverage argument is auditable.",
           "#[rustfmt::skip]",
           f"pub static DYNAMIC_MONSTER_SITES: [(&str, &str); {len(dyn)}] = ["]
    for site, note in dyn:
        out.append(f"    ({rust_str(site)}, {rust_str(note)}),")
    out.append("];")
    out.append("")
    rows = sorted(REFUSALS)
    out += [""]
    out += [
           "/// Registry rows the generator refused to transcribe, as",
           "/// `(site, reason)`. Empty is the healthy state; a non-empty",
           "/// table is an explicit I5 refusal, never a silent drop.",
           "#[rustfmt::skip]",
           f"pub static REFUSED_CONTENT: "
           f"[(&str, &str); {len(rows)}] = ["]
    for site, reason in rows:
        out.append(f"    ({rust_str(site)}, {rust_str(reason)}),")
    out.append("];")
    out.append("")
    return "\n".join(out)


def _args_assert(aw, expr, values, site, typing=None, kind=None):
    rendered = aw.args(values, site, typing, kind)
    if rendered == "&[]":
        return f"assert!({expr}.is_empty());"
    return f"assert_eq!({expr}, {rendered});"


def tables_tests(cs, aw, pool_count):
    """Spot checks pinned against values read out of the registry right now."""
    checks = []
    sculpt = cs.CARDS.get(("SCULPTING_STRIKE", 0))
    if sculpt is not None:
        body = ["let row = card_row(CardId::SculptingStrike, 0).unwrap();",
                f"assert_eq!(row.cost, {sculpt.cost});",
                f"assert_eq!(row.name, {rust_str(sculpt.name)});",
                f"assert_eq!(row.steps.len(), {len(sculpt.steps)});",
                ("assert!(row.strike_tag);" if sculpt.strike_tag
                 else "assert!(!row.strike_tag);")]
        for i, step in enumerate(sculpt.steps):
            body.append(f"assert_eq!(row.steps[{i}].kind, "
                        f"StepKind::{camel(step[0])});")
            body.append(_args_assert(
                aw, f"row.steps[{i}].args", step[1:],
                f"test:SCULPTING_STRIKE.steps[{i}]",
                CARD_STEP_ARG_TYPES, step[0]))
        checks.append(("sculpting_strike_row_matches_registry", body))
    eel = cs.LOOPS.get("TERROR_EEL")
    if eel is not None:
        body = ["let moves = monster_loop(MonsterKind::TerrorEel).unwrap();",
                f"assert_eq!(moves.len(), {len(eel)});"]
        for i, e in enumerate(eel):
            body.append(f"assert_eq!(moves[{i}].name, {rust_str(e[0])});")
            body.append(f"assert_eq!(moves[{i}].kind, MoveKind::{camel(e[1])});")
            # The table's own site spelling, so a tiered argument renders
            # through the same `MoveConstantJoin` row the table did.
            body.append(_args_assert(aw, f"moves[{i}].args", e[2],
                                     f"moves:LOOPS.TERROR_EEL.{e[0]}",
                                     MOVE_ARG_TYPES, e[1]))
            body.append(f"assert_eq!(moves[{i}].repeats, Repeats::"
                        + ("Absent" if len(e) < 4 else f"Fixed({e[3]})") + ");")
        checks.append(("terror_eel_loop_matches_registry", body))
    slime = cs.LEAF_SLIME_S_MOVES.get("GOOP_MOVE")
    if slime is not None:
        body = ["let moves = random_moves(MonsterKind::LeafSlimeS).unwrap();",
                "let goop = moves",
                "    .iter()",
                "    .find(|m| m.name == \"GOOP_MOVE\")",
                "    .expect(\"GOOP_MOVE\");",
                f"assert_eq!(goop.kind, MoveKind::{camel(slime[0])});",
                _args_assert(aw, "goop.args", slime[1],
                             "test:LEAF_SLIME_S.GOOP_MOVE",
                             MOVE_ARG_TYPES, slime[0])]
        checks.append(("leaf_slime_goop_move_is_a_typed_card_key", body))

    lines = ["#[cfg(test)]",
             "#[rustfmt::skip]",
             "mod tests {", "    use super::*;", ""]
    lines.append("    #[test]")
    lines.append("    fn card_rows_are_sorted_and_complete() {")
    lines.append(f"        assert_eq!(CARD_ROWS.len(), {len(cs.CARDS)});")
    lines.append("        assert!(CARD_ROWS")
    lines.append("            .windows(2)")
    lines.append("            .all(|w| (w[0].id as u16, w[0].upgrade)")
    lines.append("                < (w[1].id as u16, w[1].upgrade)));")
    lines.append("        for row in CARD_ROWS.iter() {")
    lines.append("            assert_eq!(card_row(row.id, row.upgrade), Some(row));")
    lines.append("        }")
    lines.append("    }")
    lines.append("")
    type_counts = collections.Counter(
        cs._CARD_TYPE_BY_ID[cid] for cid, _upgrade in cs.CARDS)
    lines.append("    #[test]")
    lines.append("    fn card_type_metadata_has_the_complete_six_way_census() {")
    lines.append("        assert_eq!(CardType::ALL.len(), 6);")
    for card_type in ("attack", "skill", "power", "status", "curse", "quest"):
        lines.append(
            "        assert_eq!(CARD_ROWS.iter().filter(|row| "
            f"row.card_type == CardType::{camel(card_type)}).count(), "
            f"{type_counts[card_type]});")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn native_unplayable_rows_are_exact_and_distinct_from_can_play() {")
    lines.append(f"        assert_eq!(NATIVE_UNPLAYABLE_CARD_ROWS.len(), "
                 f"{len(cs._NATIVE_UNPLAYABLE_CARD_KEYS)});")
    lines.append("        assert!(NATIVE_UNPLAYABLE_CARD_ROWS.windows(2).all(|w| ")
    lines.append("            (w[0].0 as u16, w[0].1) < (w[1].0 as u16, w[1].1)));")
    lines.append("        for &(id, upgrade) in NATIVE_UNPLAYABLE_CARD_ROWS.iter() {")
    lines.append("            assert!(card_has_native_unplayable_keyword(id, upgrade));")
    lines.append("        }")
    lines.append("        assert!(card_has_native_unplayable_keyword(CardId::Dazed, 0));")
    lines.append("        assert!(card_has_native_unplayable_keyword(CardId::Normality, 0));")
    lines.append("        assert!(card_has_native_unplayable_keyword(CardId::SpoilsMap, 0));")
    lines.append("        assert!(!card_has_native_unplayable_keyword(CardId::Sloth, 0));")
    lines.append("        assert!(!card_has_native_unplayable_keyword(CardId::Enthralled, 0));")
    lines.append("    }")
    lines.append("")
    rarity_counts = collections.Counter(cs._CARD_RARITY_BY_ID.values())
    lines.append("    #[test]")
    lines.append("    fn card_rarity_metadata_has_the_complete_derived_census() {")
    lines.append(f"        assert_eq!(CardRarity::ALL.len(), {len(rarity_counts)});")
    for rarity in sorted(rarity_counts):
        lines.append(
            "        assert_eq!(CardId::ALL.iter().filter(|id| "
            "card_rows(**id)[0].rarity == "
            f"CardRarity::{camel(rarity)}).count(), {rarity_counts[rarity]});"
        )
        lines.append(
            f"        assert_eq!(CardRarity::{camel(rarity)}.as_str(),"
            f" {rust_str(rarity)});"
        )
    lines.append("        for id in CardId::ALL {")
    lines.append("            let rows = card_rows(id);")
    lines.append("            assert!(!rows.is_empty(), \"{}\", id.as_str());")
    lines.append("            assert!(rows.iter().all(|row| row.rarity == rows[0].rarity),")
    lines.append("                \"rarity drifted across upgrades for {}\", id.as_str());")
    lines.append("        }")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn every_card_id_has_at_least_one_row() {")
    lines.append("        for id in CardId::ALL.iter() {")
    lines.append("            assert!(!card_rows(*id).is_empty(), \"{}\", id.as_str());")
    lines.append("        }")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn keyed_tables_are_sorted_for_binary_search() {")
    lines.append("        assert!(LOOPS.windows(2).all(|w| w[0].0 < w[1].0));")
    lines.append("        assert!(RANDOM_MOVES.windows(2).all(|w| w[0].0 < w[1].0));")
    lines.append("        assert!(FABRICATOR_BOT_MOVES")
    lines.append("            .windows(2)")
    lines.append("            .all(|w| w[0].0 < w[1].0));")
    lines.append("        assert!(TEMPLATE_RELIC_STEPS")
    lines.append("            .windows(2)")
    lines.append("            .all(|w| w[0].0 < w[1].0));")
    lines.append("    }")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn generation_pools_are_pinned() {")
    lines.append(f"        assert_eq!(GENERATION_POOLS.len(), {pool_count});")
    lines.append("        for (name, pool) in GENERATION_POOLS.iter() {")
    lines.append("            assert!(!name.is_empty());")
    lines.append("            assert!(pool.iter().all(|c| "
                 "CardId::from_str(c.as_str()) == Some(*c)));")
    lines.append("        }")
    lines.append("    }")
    lines.append("")
    lines.append("    /// I5: a non-empty refusal table is a real event. This")
    lines.append("    /// pins the count so a new refusal cannot land silently.")
    lines.append("    #[test]")
    lines.append("    fn refusal_count_is_pinned() {")
    lines.append(f"        assert_eq!(REFUSED_CONTENT.len(), {len(REFUSALS)});")
    lines.append(f"        assert_eq!(DYNAMIC_MONSTER_SITES.len(), "
                 f"{len(DYNAMIC_SITES)});")
    lines.append("    }")
    lines.append("    #[test]")
    lines.append("    fn relic_ledger_partition_is_exact() {")
    lines.append("        assert_eq!(RELIC_LEDGER.len(), 299);")
    lines.append("        assert_eq!(CENSUS_INERT_RELICS.len(), 117);")
    lines.append("        assert_eq!(EFFECTIVE_TEMPLATE_RELICS.len(), 18);")
    lines.append("        assert_eq!(HAND_AUTHORED_ACTIVE_RELICS.len(), 164);")
    lines.append("        assert_eq!(RAW_TEMPLATE_OVERLAP_RELICS.len(), 16);")
    lines.append("")
    lines.append("        let mut inert_count = 0;")
    lines.append("        let mut template_only_count = 0;")
    lines.append("        let mut hand_active_count = 0;")
    lines.append("        let mut admitted_count = 0;")
    lines.append("        let mut implemented_count = 0;")
    lines.append("        let mut compiled_template_count = 0;")
    lines.append("        let mut raw_template_count = 0;")
    lines.append("")
    lines.append("        for (idx, row) in RELIC_LEDGER.iter().enumerate() {")
    lines.append("            assert_eq!(row.id as usize, idx);")
    lines.append("            assert_eq!(relic_ledger_row(row.id), row);")
    lines.append("            assert_eq!(row.id.as_str(), row.name);")
    lines.append("            assert!(row.inventory_represented);")
    lines.append("")
    lines.append("            if row.in_raw_templates {")
    lines.append("                raw_template_count += 1;")
    lines.append("            }")
    lines.append("            if row.compiled_template {")
    lines.append("                compiled_template_count += 1;")
    lines.append("            }")
    lines.append("            if row.implemented {")
    lines.append("                implemented_count += 1;")
    lines.append("            }")
    lines.append("            if row.exactly_admitted {")
    lines.append("                admitted_count += 1;")
    lines.append("            }")
    lines.append("")
    lines.append("            match row.classification {")
    lines.append("                RelicClassification::CensusInert => {")
    lines.append("                    inert_count += 1;")
    lines.append("                    assert!(CENSUS_INERT_RELICS.binary_search(&row.id).is_ok());")
    lines.append("                    assert!(!row.in_effective_templates);")
    lines.append("                    assert!(!row.compiled_template);")
    lines.append("                    assert!(!row.implemented);")
    lines.append("                    assert!(row.exactly_admitted);")
    lines.append("                }")
    lines.append("                RelicClassification::TemplateOnly => {")
    lines.append("                    template_only_count += 1;")
    lines.append("                    assert!(EFFECTIVE_TEMPLATE_RELICS.binary_search(&row.id).is_ok());")
    lines.append("                    assert!(row.in_raw_templates);")
    lines.append("                    assert!(row.in_effective_templates);")
    lines.append("                    assert!(row.compiled_template);")
    lines.append("                    assert_eq!(row.implemented, row.exactly_admitted);")
    lines.append("                }")
    lines.append("                RelicClassification::HandAuthoredActive => {")
    lines.append("                    hand_active_count += 1;")
    lines.append("                    assert!(HAND_AUTHORED_ACTIVE_RELICS.binary_search(&row.id).is_ok());")
    lines.append("                    assert!(!row.in_effective_templates);")
    lines.append("                    assert!(!row.compiled_template);")
    lines.append("                    if row.implemented {")
    lines.append("                        assert!(row.exactly_admitted);")
    lines.append("                    } else {")
    lines.append("                        assert!(!row.exactly_admitted);")
    lines.append("                    }")
    lines.append("                }")
    lines.append("            }")
    lines.append("        }")
    lines.append("")
    lines.append("        assert_eq!(inert_count, 117);")
    lines.append("        assert_eq!(template_only_count, 18);")
    lines.append("        assert_eq!(hand_active_count, 164);")
    lines.append("        assert_eq!(raw_template_count, 34);")
    lines.append("        assert_eq!(compiled_template_count, 18);")
    lines.append("        assert_eq!(implemented_count, 182);")
    lines.append("        assert_eq!(admitted_count, 299);")
    lines.append("    }")
    for name, body in checks:
        lines.append("")
        lines.append("    #[test]")
        lines.append(f"    fn {name}() {{")
        lines.extend(f"        {b}" for b in body)
        lines.append("    }")
    lines.append("}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# Dispatch scaffolds (D4): src/steps/** and src/moves/**
# ---------------------------------------------------------------------------
#
# PORT_PLAN.md D4: the complete dispatch surface is generated once, and a wave
# PR fills stub bodies in ONE per-family file. Two file classes come out of
# here, with different ownership:
#
# * ``mod.rs`` (per tree) is GENERATED AND OWNED BY THIS TOOL. It carries the
#   whole ``match``, the kind -> family index, and the per-family implemented
#   registries the capability manifest is derived from. A wave PR never edits
#   it, so no two wave PRs can conflict in it.
# * ``<family>.rs`` files are GENERATED ONCE, THEN OWNED BY HAND. The tool
#   creates a family file when it is absent and APPENDS a refusing stub for a
#   kind the file does not carry yet; it never rewrites, reorders, or deletes
#   an existing body. That is what lets a wave PR replace a stub body in place
#   while the freshness check still guarantees every dispatch target exists.
#
# The family split is DERIVED, never hand-assigned: a card step kind belongs to
# the ``content/cards/*.py`` family whose cards use it, and a monster move kind
# to the ``content/encounters/*.py`` pool whose monsters use it. A kind used by
# more than one family belongs to ``shared``.

CARD_FAMILY_SHARED = "shared"
CARD_FAMILY_TEMPLATES = "templates"
MOVE_FAMILY_SPAWNED = "spawned"


def snake(name: str) -> str:
    """``attack_all`` / ``AfterBlockCleared`` -> a Rust snake_case identifier."""
    spaced = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name)
    out = re.sub(r"[^A-Za-z0-9]+", "_", spaced).strip("_").lower()
    if not out or out[0].isdigit():
        out = "k_" + out
    if out in _RUST_KEYWORDS:
        out = out + "_"
    return out


@_census
def dispatch_sites(cs) -> dict[str, list[str]]:
    """``kind`` -> the ``combat_sim`` functions that compare against it.

    A stub's doc comment cites the Python branch it stands for. The citation is
    a FUNCTION NAME, never a line number: #1301 showed a line-anchored citation
    reddens the freshness check on any unrelated edit above it in a 61k-line
    file. Every ``x == "kind"`` / ``x in ("a", "b")`` comparison in
    ``combat_sim.py`` is indexed here, so a kind whose dispatch moved is
    re-cited by regeneration rather than by hand.
    """
    path = SOLVER_DIR / "combat_sim.py"
    tree = ast.parse(path.read_text(), filename=str(path))
    parent: dict[int, ast.AST] = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parent[id(child)] = node
    out: dict[str, set[str]] = collections.defaultdict(set)
    for node in ast.walk(tree):
        if not isinstance(node, ast.Compare):
            continue
        if not any(isinstance(op, (ast.Eq, ast.In)) for op in node.ops):
            continue
        where = _enclosing_function(node, parent)
        for comparator in node.comparators:
            literals = []
            if isinstance(comparator, ast.Constant):
                literals = [comparator]
            elif isinstance(comparator, (ast.Tuple, ast.List, ast.Set)):
                literals = comparator.elts
            for literal in literals:
                if isinstance(literal, ast.Constant) and isinstance(literal.value, str):
                    out[literal.value].add(where)
    return {kind: sorted(sites) for kind, sites in out.items()}


def _collect_card_family_steps(fam):
    """Every ``steps`` list a card family module registers."""
    import importlib
    module = importlib.import_module(f"content.cards.{fam}")
    rows: list[list] = []

    def add(key, **kw):
        if "steps" not in kw:
            refuse(f"card_family:{fam}", f"add({key!r}) passes no steps= keyword")
            return
        rows.append(kw["steps"] or [])

    module.register(add)
    return rows


@_census
def card_step_families(cs, kinds) -> dict[str, str]:
    """``StepKind`` -> the family module that owns its body."""
    import importlib
    families = importlib.import_module("content.cards").FAMILIES
    used: dict[str, set[str]] = collections.defaultdict(set)
    for fam in families:
        for steps in _collect_card_family_steps(fam):
            for step in steps:
                if step:
                    used[step[0]].add(fam)
    for card in cs.TEMPLATE_CARDS.values():
        for step in card.steps:
            if step:
                used[step[0]].add(CARD_FAMILY_TEMPLATES)
    out = {}
    for kind in kinds:
        owners = used.get(kind, set())
        if len(owners) == 1:
            out[kind] = sorted(owners)[0]
        else:
            # No owner is unreachable (the kind axis is derived from the same
            # two tables); it lands in `shared` for the same reason a
            # multi-owner kind does — no single family may claim it.
            out[kind] = CARD_FAMILY_SHARED
    return out


@_census
def monster_kind_pools() -> dict[str, set[str]]:
    """``MonsterKind`` -> the encounter pool modules that put it in a fight.

    Two sources, both mechanical: the ``loops={kind: loop}`` registrations a
    pool module makes, and its ``Monster(...)`` construction sites (constant,
    module-global, or ``for`` -bound, resolved exactly as `monster_kinds`
    resolves them).
    """
    import importlib
    cs = _live_registry()
    g = vars(cs)
    pools = importlib.import_module("content.encounters").POOLS
    out: dict[str, set[str]] = collections.defaultdict(set)

    class _Reg:
        def __init__(self):
            self.loops = {}

        def encounter(self, id_substring, builder, loops=None):
            if loops:
                self.loops.update(loops)

    for pool in pools:
        module = importlib.import_module(f"content.encounters.{pool}")
        registry = _Reg()
        module.register(registry)
        for kind in registry.loops:
            out[kind].add(pool)

        path = SOLVER_DIR / "content" / "encounters" / f"{pool}.py"
        tree = ast.parse(path.read_text(), filename=str(path))
        parent: dict[int, ast.AST] = {}
        for node in ast.walk(tree):
            for child in ast.iter_child_nodes(node):
                parent[id(child)] = node
        for node in ast.walk(tree):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "Monster" and node.args):
                continue
            arg = node.args[0]
            if isinstance(arg, ast.Constant) and isinstance(arg.value, str):
                out[arg.value].add(pool)
            elif isinstance(arg, ast.Name) and isinstance(g.get(arg.id), str):
                out[g[arg.id]].add(pool)
            elif isinstance(arg, ast.Name):
                resolved = _resolve_loop_var(arg.id, node, parent, g)
                for kind in resolved or ():
                    out[kind].add(pool)
    return out


@_census
def move_kind_families(cs, tables, kinds) -> dict[str, str]:
    """``MoveKind`` -> the encounter pool module that owns its body.

    A kind reachable from more than one pool is ``shared``; a kind whose only
    table belongs to a monster no pool module constructs (a mid-fight spawn or
    a dynamically built roster) is ``spawned``.
    """
    g = vars(cs)
    pools = monster_kind_pools()
    used: dict[str, set[str]] = collections.defaultdict(set)

    def owners_of(monster_kind):
        return set(pools.get(monster_kind) or ()) or {MOVE_FAMILY_SPAWNED}

    def kinds_in(shape, value):
        if shape == "A":
            return [entry[1] for entry in value]
        if shape == "B":
            return [entry[0] for entry in value.values()]
        if shape == "C":
            return [entry[1] for entry in value.values()]
        return []

    for name, shape, value in tables:
        if shape in ("C", "D"):
            for monster_kind, entry in value.items():
                if shape == "C":
                    found = [entry[1]]
                else:
                    found = kinds_in(classify_move_table(entry), entry)
                for kind in found:
                    used[kind] |= owners_of(monster_kind)
            continue
        # A single monster's own table, named `<MONSTER_KIND>_MOVES`.
        base = name[:-len("_MOVES")] if name.endswith("_MOVES") else None
        monster_kind = g.get(base) if base and isinstance(g.get(base), str) else None
        owners = owners_of(monster_kind) if monster_kind else {MOVE_FAMILY_SPAWNED}
        for kind in kinds_in(shape, value):
            used[kind] |= owners

    out = {}
    for kind in kinds:
        owners = used.get(kind, set())
        if len(owners) > 1:
            owners = owners - {MOVE_FAMILY_SPAWNED}
        out[kind] = sorted(owners)[0] if len(owners) == 1 else CARD_FAMILY_SHARED
    return out


DISPATCH_MOD_PREAMBLE = """\
//! Generated {what} dispatch — DO NOT EDIT BY HAND.
//!
//! Regenerate with:
//!
//! ```text
//! python3 {tool}
//! ```
//!
//! PORT_PLAN.md D4. This file is the **complete** dispatch surface: one arm
//! per {enum} variant ({count} of them), each delegating to a body in the
//! per-family module that owns it. It is generated once and never edited by a
//! wave PR, so two wave PRs cannot conflict here; a wave PR replaces a stub
//! body in its own `{dir}/<family>.rs` and flips nothing shared.
//!
//! The family split is derived, not assigned:
{split}
//!
//! Freshness (this file, and the existence of every dispatch target) is
//! CI-enforced by the `rust port` workflow's generator `--check` step.
"""

FAMILY_FILE_PREAMBLE = """\
//! {title} — GENERATED ONCE, THEN OWNED BY HAND.
//!
//! PORT_PLAN.md D4. `{tool}`
//! created this file with a refusing stub per kind and will APPEND a stub for
//! any kind that later joins this family, but it never rewrites, reorders, or
//! removes what is already here: the bodies are hand-written ports and this
//! file is the wave PR's private edit surface.
//!
//! Filling a stub is a three-line contract:
//!
//! 1. replace the `Err(...)` body with the port of the cited Python branch;
//! 2. add the kind to [`IMPLEMENTED`] — the capability manifest and the
//!    admission gate are both derived from it (D6), so an unlisted body is
//!    unreachable. The source-derived family-triage gate also rejects a listed
//!    body that directly names its own `*KindNotModeled` refusal; focused crate
//!    tests remain the runtime evidence;
//! 3. leave the signature alone; [`super`]'s generated `match` calls it.
"""


def emit_dispatch(kind_enum, kinds, family_of, sites, what, dir_name,
                  ctx_type, refusal_variant, split_note, cite):
    """Render the generated `mod.rs` plus one stub body per kind.

    Returns ``(mod_src, {family: {kind: stub_src}})``.
    """
    families = sorted(set(family_of.values()))
    lines = [DISPATCH_MOD_PREAMBLE.format(
        what=what, tool=TOOL, enum=kind_enum, count=len(kinds), dir=dir_name,
        split="\n".join("//! " + line for line in wrap_doc(split_note)))]
    lines.append("")
    for family in families:
        lines.append(f"pub mod {family};")
    lines.append("")
    lines.append(f"use crate::engine::{{EngineRefusal, {ctx_type}}};")
    lines.append(f"use crate::ids::{kind_enum};")
    lines.append("")
    lines.append(f"/// Every {kind_enum}, with the family module that owns its body.")
    lines.append("///")
    lines.append("/// Published so the completeness/uniqueness contract test can be")
    lines.append("/// derived rather than hand-listed.")
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static FAMILY_OF: [({kind_enum}, &str); {len(kinds)}] = [")
    payload = " ".join(
        f"({kind_enum}::{camel(kind)}, {rust_str(family_of[kind])})," for kind in kinds)
    lines.extend(pack(payload, 4))
    lines.append("];")
    lines.append("")
    lines.append("/// Each family's own implemented-kind registry.")
    lines.append("///")
    lines.append("/// D6: the capability manifest is DERIVED from these, never")
    lines.append("/// transcribed. A family file owns its entry; this table only")
    lines.append("/// concatenates them.")
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static FAMILIES: [(&str, &[{kind_enum}]); {len(families)}] = [")
    payload = " ".join(
        f"({rust_str(family)}, {family}::IMPLEMENTED)," for family in families)
    lines.extend(pack(payload, 4))
    lines.append("];")
    lines.append("")
    lines.append(f"/// Whether some family implements `kind`.")
    lines.append("///")
    lines.append("/// Boundary-time work (the admission gate and the manifest), never a")
    lines.append("/// per-node path: it walks the family registries.")
    lines.append(f"pub fn is_implemented(kind: {kind_enum}) -> bool {{")
    lines.append("    FAMILIES.iter().any(|(_, kinds)| kinds.contains(&kind))")
    lines.append("}")
    lines.append("")
    lines.append(f"/// Dispatch one {what} to its family body.")
    lines.append(f"pub fn apply_{dir_name[:-1]}(kind: {kind_enum}, "
                 f"ctx: &mut {ctx_type}<'_>) -> Result<(), EngineRefusal> {{")
    lines.append("    match kind {")
    for kind in kinds:
        # rustfmt's own arm layout: inline while it fits in 100 columns, the
        # braced form when it does not. Emitting what rustfmt would emit is
        # what keeps `cargo fmt --check` and the generator freshness check
        # green at the same time.
        arm = (f"        {kind_enum}::{camel(kind)} => "
               f"{family_of[kind]}::{snake(kind)}(ctx),")
        if len(arm) <= 100:
            lines.append(arm)
        else:
            lines.append(f"        {kind_enum}::{camel(kind)} => {{")
            lines.append(f"            {family_of[kind]}::{snake(kind)}(ctx)")
            lines.append("        }")
    lines.append("    }")
    lines.append("}")
    lines.append("")

    stubs: dict[str, dict[str, str]] = {family: {} for family in families}
    for kind in kinds:
        where = sites.get(kind) or []
        if where:
            citation = (f"Python: `{cite}` dispatch; the kind is read in "
                        + ", ".join(f"`{fn}`" for fn in where[:4])
                        + (", …" if len(where) > 4 else "") + ".")
        else:
            citation = (f"Python: no `== {kind!r}` comparison in combat_sim.py — "
                        f"the branch is reached by table, not by name.")
        body = [f"/// `{kind}` — not modeled.", "///"]
        body += [f"/// {line}" for line in wrap_doc(citation)]
        body.append(f"pub(crate) fn {snake(kind)}("
                    f"ctx: &mut {ctx_type}<'_>) -> Result<(), EngineRefusal> {{")
        body.append("    let _ = ctx;")
        body.append(f"    Err(EngineRefusal::{refusal_variant}"
                    f"({kind_enum}::{camel(kind)}))")
        body.append("}")
        stubs[family_of[kind]][kind] = "\n".join(body) + "\n"
    return "\n".join(lines), stubs


def family_file_header(kind_enum, ctx_type, family, title):
    lines = [FAMILY_FILE_PREAMBLE.format(title=title, tool=TOOL)]
    lines.append("")
    lines.append(f"use super::{ctx_type};")
    lines.append("use crate::engine::EngineRefusal;")
    lines.append(f"use crate::ids::{kind_enum};")
    lines.append("")
    lines.append(f"/// The {kind_enum}s this family implements.")
    lines.append("///")
    lines.append("/// Add a kind here in the same diff that fills its body — this")
    lines.append("/// slice is the manifest's and the admission gate's only source of")
    lines.append("/// truth for what this family can do (D6).")
    lines.append(f"pub const IMPLEMENTED: &[{kind_enum}] = &[];")
    lines.append("")
    lines.append("")
    return "\n".join(lines)


def write_dispatch_tree(root, mod_src, stubs, kind_enum, ctx_type, title_of,
                        check, stale, created):
    """Write/verify one dispatch tree. Returns nothing; appends to `stale`."""
    mod_path = root / "mod.rs"
    current = mod_path.read_text() if mod_path.exists() else None
    if current != mod_src:
        if check:
            stale.append(mod_path)
        else:
            root.mkdir(parents=True, exist_ok=True)
            mod_path.write_text(mod_src)
    for family, bodies in sorted(stubs.items()):
        path = root / f"{family}.rs"
        text = path.read_text() if path.exists() else None
        if text is None:
            fresh = family_file_header(kind_enum, ctx_type, family,
                                       title_of(family))
            fresh += "\n".join(bodies[kind] for kind in sorted(bodies))
            if check:
                stale.append(path)
                continue
            root.mkdir(parents=True, exist_ok=True)
            path.write_text(normalize(fresh))
            created.append(path)
            continue
        missing = [kind for kind in sorted(bodies)
                   if f"fn {snake(kind)}(" not in text]
        if not missing:
            continue
        if check:
            stale.append(path)
            continue
        # Append-only: an owned file gains the stubs it lacks and keeps every
        # body it already carries.
        path.write_text(normalize(
            text.rstrip("\n") + "\n\n"
            + "\n".join(bodies[kind] for kind in missing)))
        created.append(path)


def build_dispatch(cs, axes, tables):
    """Both dispatch trees, as ``(root, mod_src, stubs, enum, ctx, title_of)``."""
    sites = dispatch_sites(cs)
    step_family = dict(card_step_families(cs, recorded_axes(axes)["StepKind"]))
    for kind, family in RUST_STEP_KIND_FAMILIES.items():
        assert kind not in step_family, f"{kind} is already a frozen step kind"
        step_family[kind] = family
    move_family = move_kind_families(cs, tables, axes["MoveKind"])

    step_mod, step_stubs = emit_dispatch(
        "StepKind", axes["StepKind"], step_family, sites,
        what="card-step", dir_name="steps", ctx_type="StepCtx",
        refusal_variant="StepKindNotModeled",
        split_note=("a kind belongs to the `content/cards/*.py` family whose "
                    "cards use it, `templates` when only auto-translated "
                    "`card_templates.json` rows use it, and `shared` when more "
                    "than one family does."),
        cite="_run_steps_inner")
    move_mod, move_stubs = emit_dispatch(
        "MoveKind", axes["MoveKind"], move_family, sites,
        what="monster-move", dir_name="moves", ctx_type="MoveCtx",
        refusal_variant="MoveKindNotModeled",
        split_note=("a kind belongs to the `content/encounters/*.py` pool whose "
                    "monsters use it, `spawned` when its only monster is one no "
                    "pool builder constructs, and `shared` when more than one "
                    "pool does."),
        cite="monster_act")

    def step_title(family):
        if family == CARD_FAMILY_SHARED:
            return "Card-step bodies used by more than one content family"
        if family == CARD_FAMILY_TEMPLATES:
            return ("Card-step bodies used only by auto-translated "
                    "`card_templates.json` rows")
        return f"Card-step bodies for the `content/cards/{family}.py` family"

    def move_title(family):
        if family == CARD_FAMILY_SHARED:
            return "Monster-move bodies used by more than one encounter pool"
        if family == MOVE_FAMILY_SPAWNED:
            return ("Monster-move bodies for monsters no encounter builder "
                    "constructs directly")
        return (f"Monster-move bodies for the "
                f"`content/encounters/{family}.py` pool")

    return [
        (RUST_DIR / "src" / "steps", normalize(step_mod), step_stubs,
         "StepKind", "StepCtx", step_title, step_family),
        (RUST_DIR / "src" / "moves", normalize(move_mod), move_stubs,
         "MoveKind", "MoveCtx", move_title, move_family),
    ]


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------

def normalize(src: str) -> str:
    """Make the emitted source `cargo fmt --check`-stable.

    The emitters concatenate independently-terminated sections, which can
    produce runs of blank lines and trailing whitespace that rustfmt would
    otherwise rewrite — turning the CI freshness check into a formatting
    argument. Collapsing them here keeps `cargo fmt --check` and the
    regenerate-and-diff check green at the same time.
    """
    lines = [line.rstrip() for line in src.split("\n")]
    out: list[str] = []
    for line in lines:
        if not line and out and not out[-1]:
            continue
        out.append(line)
    while out and not out[-1]:
        out.pop()
    return "\n".join(out) + "\n"


ENUM_ORDER = ["CardId", "EnchantmentId", "EncounterId", "FilterMode",
              "MonsterKind", "MoveKind", "PotionId", "PowerId", "RelicId",
              "SelectOp", "StepKind", "StepWord"]


def build_axes(cs, tables, src):
    kinds = monster_kinds(cs)
    return {
        # Content identity: `src` decides whether these follow the registry
        # or the assembly (#2496).
        "CardId": src.card_ids(cs),
        "EnchantmentId": src.enchantment_ids(cs),
        "PotionId": src.potion_ids(cs),
        "RelicId": src.relic_ids(cs),
        # Modeled axes: no DLL analogue, see dll_content.MODELING_ANNEX.
        "FilterMode": filter_modes(cs),
        "EncounterId": sorted(cs.SUPPORTED_ENCOUNTERS),
        "MonsterKind": kinds,
        "MoveKind": move_kinds(cs, tables),
        "PowerId": power_ids(cs),
        "SelectOp": select_ops(cs),
        "StepKind": step_kinds(cs),
    }


def resolve_source(kind, dll_path=None, manifest=None):
    """Resolve ``--source`` into ``(kind, source, note)``.

    Only ``auto`` resolves; an explicit ``python``/``dll``/``manifest`` is
    built as asked, so ``--source dll`` still *fails* when the assembly is
    unreadable rather than quietly producing registry output.

    The ladder is ``dll`` -> ``manifest`` -> ``python``. Landing on
    ``manifest`` is NOT a fallback and carries no note: those facts were read
    out of the assembly and committed, so the content half still follows the
    build (#2515). Only ``python`` is a downgrade, because under D3 the
    registry is frozen at v0.111.0 — so the note is non-empty exactly then,
    and the caller must print it. A silent downgrade to the frozen registry is
    the one outcome this must never have (D3, #1282).

    Each source is read once, here, and returned rather than re-derived by the
    caller.
    """
    if kind != "auto":
        return kind, dll_content.make_source(kind, dll_path,
                                             manifest=manifest), ""
    missing = []
    archived = dll_content.archived_certified_dll()
    if archived is None:
        missing.append(
            f"no archived {dll_content.CERTIFIED_BUILD} assembly on this host "
            "(solver/dll-archive/ is gitignored and lives only in the MAIN "
            "checkout)")
    else:
        try:
            facts = dll_content.DllFacts(archived, require_certified=True)
            return "dll", dll_content.DllContentSource(facts), ""
        except dll_content.DllUnavailable as exc:
            missing.append(f"{archived} is present but unreadable here: {exc}")
    try:
        return "manifest", dll_content.make_source(
            "manifest", manifest=manifest, require_certified=True), ""
    except dll_content.ManifestUnavailable as exc:
        missing.append(str(exc))
    return "python", dll_content.make_source("python"), "; ".join(missing)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true",
                    help="verify the checked-in files are up to date; do not "
                         "write")
    ap.add_argument("--source",
                    choices=("auto", "python", "dll", "manifest"),
                    default="auto",
                    help="where the content facts come from. `auto` (the "
                         "default) resolves dll -> manifest -> python, and "
                         "only the last carries a loud note; `dll` reads "
                         "sts2.dll for everything in "
                         "dll_content.DLL_SOURCED_FACTS, writes the manifest "
                         "and generates from it (#2496, #2515); `manifest` "
                         "replays the committed manifest with no assembly and "
                         "no dnfile, which is what CI runs; `python` reads "
                         "the frozen registry snapshot. The declared "
                         "modeling annex comes from the registry on every "
                         "path")
    ap.add_argument("--dll", default=None,
                    help="with --source dll, the assembly to read (default: "
                         "the archived certified build, else $STS2_DLL, else "
                         "the Steam install)")
    ap.add_argument("--manifest", default=None,
                    help="the content manifest to read (--source manifest) or "
                         "write (--source dll); default: "
                         "versions/<build>/rust/data/dll_content.<build>.json, "
                         "or that basename under --out-dir")
    ap.add_argument("--registry", choices=("frozen",),
                    default="frozen",
                    help="where the modeling annex comes from: the committed "
                         "data/python_registry.v0.111.0.json, the only "
                         "registry since the Python simulator's deletion "
                         "(#2827 item F)")
    ap.add_argument("--out-dir", default=None,
                    help="write the generated files under this directory "
                         "instead of the crate, for a regenerate-and-diff "
                         "check that never touches the working tree")
    args = ap.parse_args(argv)

    if args.dll and args.source != "dll":
        ap.error("--dll is only meaningful with --source dll")
    if args.manifest and args.source not in ("dll", "manifest"):
        ap.error("--manifest is only meaningful with --source dll (which "
                 "writes it) or --source manifest (which reads it)")
    source, src, note = resolve_source(args.source, args.dll, args.manifest)
    if note:
        print(f"NOTE: --source auto fell back to `python`: {note}",
              file=sys.stderr)
        print("      the generated tables describe the frozen v0.111.0 "
              "registry, not an assembly. Re-run with a readable archive "
              "under python3.12, or commit a content manifest, for "
              "DLL-sourced output (#1282 D3, #2515).",
              file=sys.stderr)

    # `--source dll` never hands its facts straight to the generator: it
    # serializes every one of them into the committed manifest and generates
    # from THAT (#2515). Two things follow, and both are the point. The
    # manifest is proven complete, because a fact it failed to carry would
    # break the DLL path too rather than only CI's. And the byte-for-byte
    # acceptance test reaches one hop further out: DLL -> manifest -> tables
    # equals the committed tables.
    provenance = src.describe()
    manifest_target, manifest_bytes = None, None
    if source == "dll":
        manifest = dll_content.build_manifest(src.facts)
        if manifest["build"] != dll_content.CERTIFIED_BUILD:
            raise SystemExit(
                f"the assembly is build {manifest['build']}, but this crate "
                f"is keyed to {dll_content.CERTIFIED_BUILD}. Fork the crate "
                "before regenerating its content (VERSION_BUMP_RUNBOOK.md "
                "step 10), rather than writing another build's facts into "
                "this one's manifest")
        name = f"dll_content.{manifest['build']}.json"
        if args.manifest:
            manifest_target = pathlib.Path(args.manifest)
        elif args.out_dir:
            manifest_target = pathlib.Path(args.out_dir) / name
        else:
            manifest_target = dll_content.default_manifest_path()
        manifest_bytes = dll_content.manifest_text(manifest)
        src = dll_content.ManifestContentSource(
            dll_content.ManifestFacts(manifest_target, payload=manifest))
        provenance = f"{provenance} -> {src.describe()}"

    cs = load_registry(args.registry)
    tables = move_tables(cs)
    axes = build_axes(cs, tables, src)

    # The tables come first: `StepWord` is the vocabulary they still carry as
    # `Arg::S` after typed-argument resolution, so it is read back out of the
    # emitted source rather than guessed alongside it.
    tables_src = normalize(build_tables(cs, axes, tables, src))
    axes["StepWord"] = step_words(tables_src)
    for mode in axes["FilterMode"]:
        if mode not in axes["StepWord"]:
            raise SystemExit(
                f"FilterMode member {mode!r} is not in the emitted table "
                "vocabulary; the select filter position was typed away")
    ids_src = normalize(build_ids(cs, axes, ENUM_ORDER))

    # `--out-dir` writes the two generated files somewhere else so a caller
    # can regenerate and diff without ever touching the working tree. The
    # dispatch pass is then forced into check mode: its output is a pure
    # function of the axes, which `--out-dir` does not change, so verifying it
    # is right and writing it into a scratch directory would be meaningless.
    out_dir = pathlib.Path(args.out_dir) if args.out_dir else RUST_DIR / "src"
    if args.out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)
    targets = {
        out_dir / "ids.rs": ids_src,
        out_dir / "content_tables.rs": tables_src,
    }
    if manifest_target is not None:
        targets[manifest_target] = manifest_bytes
        if not args.check or args.out_dir:
            manifest_target.parent.mkdir(parents=True, exist_ok=True)

    stale = []
    for path, generated in targets.items():
        current = path.read_text() if path.exists() else None
        if current == generated:
            continue
        if args.check and not args.out_dir:
            stale.append(path)
        else:
            path.write_text(generated)

    created = []
    trees = build_dispatch(cs, axes, tables)
    for root, mod_src, stubs, kind_enum, ctx_type, title_of, _family_of in trees:
        write_dispatch_tree(root, mod_src, stubs, kind_enum, ctx_type,
                            title_of, args.check or bool(args.out_dir),
                            stale, created)

    counts = ", ".join(f"{n}={len(axes[n])}" for n in ENUM_ORDER)
    print(f"content source: {source} — {provenance}", file=sys.stderr)
    print(f"modeling registry: {args.registry}", file=sys.stderr)
    families = "; ".join(
        f"{root.name}: " + ", ".join(
            f"{fam}={n}" for fam, n in sorted(
                collections.Counter(family_of.values()).items()))
        for root, _m, _s, _k, _c, _t, family_of in trees)
    if args.check and not args.out_dir:
        if stale:
            for p in stale:
                print(f"STALE: {os.path.relpath(p, REPO_ROOT)}", file=sys.stderr)
            print("re-run: python3 " + TOOL, file=sys.stderr)
            return 1
        print(f"up to date ({counts}; refusals={len(REFUSALS)})")
        print(f"dispatch families — {families}")
        return 0
    if args.out_dir and stale:
        for p in stale:
            print(f"STALE: {os.path.relpath(p, REPO_ROOT)}", file=sys.stderr)
        return 1
    print(f"wrote {len(targets)} files + {len(trees)} dispatch trees "
          f"({len(created)} family files created/extended) "
          f"({counts}; refusals={len(REFUSALS)})")
    print(f"dispatch families — {families}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
