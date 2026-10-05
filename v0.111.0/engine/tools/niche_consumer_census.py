#!/usr/bin/env python3
"""Whole-DLL census of everything that draws the run's ``Niche`` RNG stream.

The run-history counter prediction (``src/run_counters/streams.rs``) counts
one ``Niche`` draw per recorded monster and must know every *other* consumer,
because each one makes the predicted counter a baseline instead of an exact
claim (#2997: the registry it inherited was not a census, and an Ovicopter's
eggs went uncounted). This tool is the census. It reads ``sts2.dll`` and
writes ``fixtures/niche_consumer_census_v1.json``; the cargo test
``the_niche_registries_cover_the_whole_dll_census`` holds the registries to
that file.

How the scan works
------------------

Start from the one accessor, ``RunRngSet::get_Niche``. Collect every method
whose body calls (``call`` / ``callvirt`` / ``newobj`` / ``ldftn`` /
``ldvirtftn``) a method already in the set. A caller that is a *wrapper*
(``WRAPPERS`` below: it only forwards the draw to its own callers) joins the
set and the walk continues from it; any other caller is a *site* and is
written to the census. Three things keep the walk honest:

* an async method's body lives in its compiler-generated state machine
  (``Type/<Method>d__N::MoveNext``), so a ``MoveNext`` that calls into the set
  stands for its kickoff method, found through the kickoff's
  ``AsyncTaskMethodBuilder.Start<StateMachine>`` instantiation. That is what
  tells the two creating ``CreatureCmd::Add`` overloads from the third, which
  only attaches a creature somebody else created;
* an interface method stands for its implementation (``ICombatState`` for
  ``CombatState``), through the ``InterfaceImpl`` table;
* generic call sites are ``MethodSpec`` tokens (``CreatureCmd::Add<ToughEgg>``)
  and resolve to the generic method they instantiate.

Every site's outermost declaring type must be classified in ``CLASSIFIED``.
An unclassified type is an error, not a default: that is how a new build's
extra caller fails loudly.

Two derived facts are checked rather than trusted:

* the events with the combat layout (``get_LayoutType`` returning 1), whose
  encounter ``EventCombatSynchronizer::InitializeForEvent`` creates on entry;
* which monsters apply each spawning power, since the registry is keyed by
  the monster the ``.run`` records, not by the power.

Usage
-----

    STS2_DLL=/path/to/sts2.dll python3.12 tools/niche_consumer_census.py --check
    STS2_DLL=/path/to/sts2.dll python3.12 tools/niche_consumer_census.py --write

``--check`` (the default) regenerates the census and diffs it against the
committed fixture. It needs ``dnfile`` and ``dncil`` (``python3.12`` on the
maintainer's machine) and the game assembly, which is not in the repository,
so no CI lane runs it: run it by hand whenever the crate is forked forward to
a new build, before trusting the copied registries.

One fact the scan does not derive is read from IL by hand and recorded in the
Rust doc comments: ``CombatState::CreateCreature`` draws only for an
enemy-side creature, which is why ``PlayerCmd::AddPet`` (it passes the
player's own side) is classified ``player_side``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve()
FIXTURE = HERE.parents[1] / "fixtures" / "niche_consumer_census_v1.json"
SCHEMA = "sts-sim-niche-consumer-census-v1"

#: The accessor every draw goes through.
ROOT = ("RunRngSet", "get_Niche")

#: Methods that only forward the draw to their callers.
WRAPPERS = {
    ("CombatState", "CreateCreature"),
    ("CreatureCmd", "Add"),
    ("ToughEgg", "Hatch"),
}

#: Outermost declaring type of a site -> (class, registry key).
#:
#: * ``setup``: the encounter's own roster, already counted from ``monster_ids``.
#: * ``midfight``: a creation (or ToughEgg's hatch roll) after setup; the key
#:   is the ``NICHE_MIDFIGHT_SPAWNERS`` row. A power is keyed by ``POWER_OWNERS``.
#: * ``midfight_recorded``: a creation after setup that ``monster_ids`` does
#:   count: one creature of each of two ids the encounter never sets up, once
#:   per fight (see the Rust doc comment). No registry row.
#: * ``layout_event``: creation when a combat-layout event is entered.
#: * ``relic`` / ``modifier``: a relic's or run modifier's own hook.
#: * ``player_side``: reaches ``CreateCreature`` with the player's side, so no draw.
#: * ``test_only``: a mock monster no encounter uses.
CLASSIFIED: dict[str, tuple[str, str | None]] = {
    "CombatRoom": ("setup", None),
    "EventCombatSynchronizer": ("layout_event", None),
    "PlayerCmd": ("player_side", None),
    "MockAttackAndSummonMinionMonster": ("test_only", None),
    "LivingFog": ("midfight", "LIVING_FOG"),
    "TwoTailedRat": ("midfight", "TWO_TAILED_RAT"),
    "InfestedPower": ("midfight", "PHROG_PARASITE"),
    "StockPower": ("midfight", "AXEBOT"),
    "Ovicopter": ("midfight", "OVICOPTER"),
    "ToughEgg": ("midfight", "OVICOPTER"),
    "Fogmog": ("midfight", "FOGMOG"),
    "TheObscura": ("midfight", "THE_OBSCURA"),
    "SurprisePower": ("midfight_recorded", "GREMLIN_MERC"),
    "Fabricator": ("midfight", "FABRICATOR"),
    "CursedRun": ("modifier", "MODIFIER.CURSED_RUN"),
    "Astrolabe": ("relic", "RELIC.ASTROLABE"),
    "BeautifulBracelet": ("relic", "RELIC.BEAUTIFUL_BRACELET"),
    "DistinguishedCape": ("relic", "RELIC.DISTINGUISHED_CAPE"),
    "FishingRod": ("relic", "RELIC.FISHING_ROD"),
    "FragrantMushroom": ("relic", "RELIC.FRAGRANT_MUSHROOM"),
    "Kaleidoscope": ("relic", "RELIC.KALEIDOSCOPE"),
    "NeowsBones": ("relic", "RELIC.NEOWS_BONES"),
    "NewLeaf": ("relic", "RELIC.NEW_LEAF"),
    "PandorasBox": ("relic", "RELIC.PANDORAS_BOX"),
    "RoyalStamp": ("relic", "RELIC.ROYAL_STAMP"),
    "SandCastle": ("relic", "RELIC.SAND_CASTLE"),
    "WarHammer": ("relic", "RELIC.WAR_HAMMER"),
    "WarPaint": ("relic", "RELIC.WAR_PAINT"),
    "Whetstone": ("relic", "RELIC.WHETSTONE"),
    "WingCharm": ("relic", "RELIC.WING_CHARM"),
}

#: Spawning power -> the only monster types whose code may name it.
POWER_OWNERS: dict[str, list[str]] = {
    "InfestedPower": ["PhrogParasite"],
    "StockPower": ["Axebot"],
    "SurprisePower": ["GremlinMerc"],
}

#: Event type -> model id, for the events ``get_LayoutType`` gives the combat
#: layout (1).
COMBAT_LAYOUT_EVENTS: dict[str, str] = {
    "PunchOff": "EVENT.PUNCH_OFF",
    "TheArchitect": "EVENT.THE_ARCHITECT",
    "TheLanternKey": "EVENT.THE_LANTERN_KEY",
}

CALL_OPS = {"call", "callvirt", "newobj", "ldftn", "ldvirtftn"}


class CensusError(Exception):
    """The DLL no longer matches what the classification table expects."""


def scan(dll: pathlib.Path) -> dict:
    import dnfile
    from dncil.cil.body import CilMethodBody
    from dncil.cil.body.reader import CilMethodBodyReaderBase
    from dncil.cil.error import MethodBodyFormatError

    class RawReader(CilMethodBodyReaderBase):
        def __init__(self, data, off):
            self.data, self.base, self.i = data, off, 0

        def read(self, n):
            b = self.data[self.base + self.i:self.base + self.i + n]
            self.i += n
            return b

        def tell(self):
            return self.i

        def seek(self, x):
            self.i = x
            return self.i

    raw = dll.read_bytes()
    pe = dnfile.dnPE(str(dll))
    md = pe.net.mdtables

    owner: dict[int, int] = {}              # method rid -> TypeDef index
    for ti, t in enumerate(md.TypeDef.rows):
        for m in (t.MethodList or []):
            owner[m.row_index] = ti
    enclosing: dict[int, int] = {}
    for row in md.NestedClass.rows:
        enclosing[row.NestedClass.row_index - 1] = \
            row.EnclosingClass.row_index - 1

    def type_name(ti: int) -> str:
        parts = []
        cur = ti
        while cur is not None:
            parts.append(str(md.TypeDef.rows[cur].TypeName))
            cur = enclosing.get(cur)
        return "/".join(reversed(parts))

    def outermost(ti: int) -> str:
        while ti in enclosing:
            ti = enclosing[ti]
        return str(md.TypeDef.rows[ti].TypeName)

    def method_name(rid: int) -> str:
        return str(md.MethodDef.rows[rid - 1].Name)

    def spec_typedefs(blob) -> list[int]:
        """TypeDef indices named by a MethodSpec instantiation blob."""
        data = bytes(blob.value if hasattr(blob, "value") else blob)
        pos = [0]
        found: list[int] = []

        def u8():
            pos[0] += 1
            return data[pos[0] - 1]

        def compressed():
            b = u8()
            if b < 0x80:
                return b
            if (b & 0xC0) == 0x80:
                return ((b & 0x3F) << 8) | u8()
            return ((b & 0x1F) << 24) | (u8() << 16) | (u8() << 8) | u8()

        def ty():
            et = u8()
            if et in (0x11, 0x12):                  # VALUETYPE / CLASS
                coded = compressed()
                if coded & 0x03 == 0 and coded >> 2:
                    found.append((coded >> 2) - 1)
            elif et == 0x15:                        # GENERICINST
                ty()
                for _ in range(compressed()):
                    ty()
            elif et in (0x1D, 0x0F, 0x10):          # SZARRAY / PTR / BYREF
                ty()
            elif et in (0x13, 0x1E):                # VAR / MVAR
                compressed()

        try:
            if u8() == 0x0A:
                for _ in range(compressed()):
                    ty()
        except IndexError:
            pass
        return found

    # Every call edge: (caller rid, il offset, callee MethodDef rid or None,
    # TypeDefs the call's generic instantiation names).
    edges: list[tuple[int, int, int | None, list[int]]] = []
    for rid, m in enumerate(md.MethodDef.rows, 1):
        if not m.Rva:
            continue
        try:
            body = CilMethodBody(
                RawReader(raw, pe.get_offset_from_rva(m.Rva)))
        except MethodBodyFormatError as error:
            raise CensusError(
                f"cannot parse the body of {type_name(owner[rid])}::"
                f"{m.Name}: {error}") from error
        for insn in body.instructions:
            if insn.opcode.name not in CALL_OPS or insn.operand is None:
                continue
            token = getattr(insn.operand, "value", None)
            if token is None:
                continue
            table, target = (token >> 24) & 0xFF, token & 0xFFFFFF
            callee, inst = None, []
            if table == 0x06:
                callee = target
            elif table == 0x2B:
                spec = md.MethodSpec.rows[target - 1]
                inst = spec_typedefs(spec.Instantiation)
                if spec.Method.table.name == "MethodDef":
                    callee = spec.Method.row_index
            edges.append((rid, insn.offset, callee, inst))

    # State machine MoveNext -> its kickoff method.
    move_next: dict[int, int] = {}          # SM TypeDef index -> MoveNext rid
    for ti, t in enumerate(md.TypeDef.rows):
        if ti in enclosing and "d__" in str(t.TypeName):
            for m in (t.MethodList or []):
                if str(m.row.Name) == "MoveNext":
                    move_next[ti] = m.row_index
    kickoff: dict[int, int] = {}            # MoveNext rid -> kickoff rid
    for caller, _offset, _callee, inst in edges:
        for ti in inst:
            if ti in move_next and enclosing.get(ti) == owner[caller]:
                kickoff[move_next[ti]] = caller

    # Interface method -> implementing methods, by name, through InterfaceImpl.
    implemented: dict[int, list[int]] = {}  # impl rid -> interface rids
    methods_of: dict[int, list[int]] = {}
    for rid, ti in owner.items():
        methods_of.setdefault(ti, []).append(rid)
    for row in md.InterfaceImpl.rows:
        if row.Interface.table.name != "TypeDef":
            continue
        cls, iface = row.Class.row_index - 1, row.Interface.row_index - 1
        for impl in methods_of.get(cls, []):
            for decl in methods_of.get(iface, []):
                if method_name(impl) == method_name(decl):
                    implemented.setdefault(impl, []).append(decl)

    def named(pair: tuple[str, str]) -> list[int]:
        return [rid for rid, ti in owner.items()
                if type_name(ti) == pair[0] and method_name(rid) == pair[1]]

    roots = named(ROOT)
    if len(roots) != 1:
        raise CensusError(f"expected one {ROOT[0]}::{ROOT[1]}, found {len(roots)}")

    def logical(rid: int) -> int:
        return kickoff.get(rid, rid)

    reached: set[int] = set()
    frontier = list(roots)
    sites: list[dict] = []
    seen_sites: set[tuple[int, int]] = set()
    while frontier:
        target = frontier.pop()
        if target in reached:
            continue
        reached.add(target)
        frontier.extend(implemented.get(target, []))
        for caller, offset, callee, _inst in edges:
            if callee != target:
                continue
            who = logical(caller)
            pair = (type_name(owner[who]), method_name(who))
            if who in reached or caller in reached:
                continue
            if pair in WRAPPERS or pair == ROOT:
                frontier.append(who)
                continue
            if (caller, offset) in seen_sites:
                continue
            seen_sites.add((caller, offset))
            sites.append({
                "type": type_name(owner[caller]),
                "method": method_name(caller),
                "rva": f"0x{md.MethodDef.rows[caller - 1].Rva:x}",
                "il": f"IL_{offset:04x}",
                "calls": f"{type_name(owner[target])}::{method_name(target)}",
                "_outer": outermost(owner[caller]),
            })
    unclassified = sorted({s["_outer"] for s in sites} - set(CLASSIFIED))
    if unclassified:
        raise CensusError(
            "unclassified Niche consumer(s): " + ", ".join(unclassified)
            + ". Read each one's IL, add it to CLASSIFIED here and to the "
              "matching registry in src/run_counters/streams.rs.")
    unused = sorted(set(CLASSIFIED) - {s["_outer"] for s in sites})
    if unused:
        raise CensusError(
            "classified type(s) with no Niche site in this DLL: "
            + ", ".join(unused))
    for site in sites:
        cls, key = CLASSIFIED[site.pop("_outer")]
        site["class"] = cls
        site["key"] = key
    sites.sort(key=lambda s: (s["class"], s["key"] or "", s["type"],
                              s["method"], s["il"]))

    # Events with the combat layout: get_LayoutType is `ldc.i4.1; ret`.
    layout = []
    for rid, m in enumerate(md.MethodDef.rows, 1):
        if str(m.Name) != "get_LayoutType" or not m.Rva:
            continue
        body = CilMethodBody(RawReader(raw, pe.get_offset_from_rva(m.Rva)))
        ops = [i.opcode.name for i in body.instructions
               if i.opcode.name != "nop"]
        if ops == ["ldc.i4.1", "ret"]:
            layout.append((type_name(owner[rid]), f"0x{m.Rva:x}"))
    if sorted(name for name, _ in layout) != sorted(COMBAT_LAYOUT_EVENTS):
        raise CensusError(
            "combat-layout events are "
            f"{sorted(name for name, _ in layout)}, expected "
            f"{sorted(COMBAT_LAYOUT_EVENTS)}")

    # Who names each spawning power in a generic call (PowerCmd.Apply<P>,
    # ModelDb.Power<P>, ...).
    power_index = {str(t.TypeName): ti
                   for ti, t in enumerate(md.TypeDef.rows)
                   if ti not in enclosing}
    appliers: dict[str, list[dict]] = {}
    for power, expected in POWER_OWNERS.items():
        rows = []
        for caller, offset, _callee, inst in edges:
            if power_index[power] in inst \
                    and outermost(owner[caller]) != power:
                rows.append({
                    "type": type_name(owner[caller]),
                    "method": method_name(caller),
                    "rva": f"0x{md.MethodDef.rows[caller - 1].Rva:x}",
                    "il": f"IL_{offset:04x}",
                    "_outer": outermost(owner[caller]),
                })
        got = sorted({r.pop("_outer") for r in rows})
        if got != sorted(expected):
            raise CensusError(
                f"{power} is named by {got}, expected {sorted(expected)}")
        appliers[power] = sorted(rows, key=lambda r: (r["type"], r["il"]))

    return {
        "schema": SCHEMA,
        "dll_sha256": hashlib.sha256(raw).hexdigest(),
        "root": f"{ROOT[0]}::{ROOT[1]}",
        "wrappers": sorted(
            f"{type_name(owner[rid])}::{method_name(rid)} "
            f"(0x{md.MethodDef.rows[rid - 1].Rva:x})"
            for rid in reached if md.MethodDef.rows[rid - 1].Rva),
        "sites": sites,
        "combat_layout_events": [
            {"type": name, "get_LayoutType": rva,
             "id": COMBAT_LAYOUT_EVENTS[name]}
            for name, rva in sorted(layout)],
        "spawning_power_appliers": appliers,
    }


def render(census: dict) -> str:
    return json.dumps(census, indent=2, sort_keys=True) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--dll", default=os.environ.get("STS2_DLL"),
                        help="path to sts2.dll (default: $STS2_DLL)")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true",
                      help="diff against the committed fixture (default)")
    mode.add_argument("--write", action="store_true",
                      help="rewrite the committed fixture")
    args = parser.parse_args(argv)
    if not args.dll:
        print("niche_consumer_census: pass --dll or set STS2_DLL",
              file=sys.stderr)
        return 2
    try:
        text = render(scan(pathlib.Path(args.dll)))
    except CensusError as error:
        print(f"niche_consumer_census: {error}", file=sys.stderr)
        return 1
    if args.write:
        FIXTURE.write_text(text, encoding="utf-8")
        print(f"wrote {FIXTURE}")
        return 0
    committed = FIXTURE.read_text(encoding="utf-8") if FIXTURE.exists() else ""
    if committed != text:
        print("niche_consumer_census: the committed census is stale for this "
              "DLL; re-run with --write and reconcile the registries in "
              "src/run_counters/streams.rs", file=sys.stderr)
        return 1
    print("niche consumer census: fresh")
    return 0


if __name__ == "__main__":
    sys.exit(main())
