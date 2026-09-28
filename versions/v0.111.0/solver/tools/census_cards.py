#!/usr/bin/env python3
"""Card-behavior census for the solver derisk (#58).

For every card class in sts2.dll, extract the flags that matter for
Shuffle-counter forking and solver coverage:
  - cost / type (from CardModel ctor: cost, type, rarity, target)
  - keywords (Exhaust / Ethereal / Innate / Unplayable / Retain), decoded
    from get_CanonicalKeywords' InitializeArray blobs
  - OnPlay behavior flags from call-scans (incl. nested async bodies):
    draws cards, exhausts other cards, adds cards to piles, direct
    Shuffle-stream consumption

Output: JSON {CARD.ID: {...}} on stdout.
"""
import json
import os
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError

sys.path.insert(0, str(Path(__file__).parent.parent))
from sts2_rng import snake_case  # noqa: E402

DLL = Path(os.environ.get(
    "STS2_DLL",
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                   "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                   "data_sts2_macos_arm64/sts2.dll"),
))

CARD_NS = "MegaCrit.Sts2.Core.Models.Cards"
TYPE_NAMES = {1: "attack", 2: "skill", 3: "power", 5: "curse", 6: "quest",
              4: "status"}

def _ldc_val(insn):
    nm = insn.opcode.name
    if not nm.startswith("ldc.i4"):
        return None
    if insn.operand is not None:
        return int(insn.operand)
    if nm == "ldc.i4.m1":
        return -1
    if nm[-1].isdigit():
        return int(nm[-1])
    return None


def card_model_ctor_constants(instructions, token_name):
    """Return the exact five integer arguments before CardModel::.ctor.

    Card constructors may initialize mutable instance fields first.  A full-
    body integer scan therefore has no positional meaning: SpoilsMap's leading
    ``-1`` field initializer is the current counterexample.  The unique base
    call is the fail-closed boundary already used by the template extractor.
    """
    calls = [
        i for i, insn in enumerate(instructions)
        if insn.opcode.name in ("call", "callvirt")
        and token_name(insn.operand) == "CardModel::.ctor"
    ]
    if len(calls) != 1:
        raise ValueError(f"CardModel ctor call shape ({len(calls)})")
    values = [
        value for value in (
            _ldc_val(insn) for insn in instructions[:calls[0]])
        if value is not None
    ]
    if len(values) < 5:
        raise ValueError(f"CardModel ctor args ({len(values)} constants)")
    return values[-5:]


def constructor_cost_type(constants):
    """Interpret one exact five-argument CardModel constructor slice."""
    if len(constants) != 5:
        raise ValueError(f"CardModel ctor slice ({len(constants)} constants)")
    card_type = TYPE_NAMES.get(constants[1])
    if card_type is None:
        raise ValueError(f"unknown CardType value {constants[1]}")
    return constants[0], card_type


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


def main():
    pe = dnfile.dnPE(str(DLL))
    raw = DLL.read_bytes()
    md = pe.net.mdtables

    type_ns = {}
    type_methods = {}
    method_owner = {}
    for ti, t in enumerate(md.TypeDef.rows):
        type_ns[ti] = (str(t.TypeNamespace), str(t.TypeName))
        ms = []
        for m in (t.MethodList or []):
            ms.append((str(m.row.Name), m.row))
            method_owner[m.row_index] = ti
        type_methods[ti] = ms

    memberref = {}
    for i, row in enumerate(md.MemberRef.rows, start=1):
        cls = row.Class
        parent = "?"
        if cls.table and cls.table.name == "TypeRef":
            parent = str(md.TypeRef.rows[cls.row_index - 1].TypeName)
        elif cls.table and cls.table.name == "TypeDef":
            parent = type_ns[cls.row_index - 1][1]
        memberref[i] = f"{parent}::{row.Name}"

    def token_name(operand):
        value = getattr(operand, "value", None)
        if value is None:
            return "?"
        table, rid = (value >> 24) & 0xFF, value & 0xFFFFFF
        if table == 0x06:
            owner = method_owner.get(rid)
            parent = type_ns[owner][1] if owner is not None else "?"
            return f"{parent}::{md.MethodDef.rows[rid - 1].Name}"
        if table == 0x0A:
            return memberref.get(rid, f"mref{rid}")
        return f"token{table:#x}:{rid}"

    enclosing = {}
    nc = getattr(md, "NestedClass", None)
    if nc:
        for row in nc.rows:
            enclosing[row.NestedClass.row_index - 1] = \
                row.EnclosingClass.row_index - 1

    # CardKeyword enum values (Constant heap items expose value_bytes())
    keyword_names = {}
    for ti, t in enumerate(md.TypeDef.rows):
        if str(t.TypeName) == "CardKeyword":
            idx = {f.row_index: str(f.row.Name) for f in (t.FieldList or [])}
            for c in md.Constant.rows:
                p = c.Parent
                if p.table and p.table.name == "Field" \
                        and p.row_index in idx \
                        and idx[p.row_index] != "value__":
                    val = int.from_bytes(c.Value.value_bytes()[:4], "little")
                    keyword_names[val] = idx[p.row_index]

    # field RVA data (for InitializeArray keyword blobs)
    field_rva = {}
    fr = getattr(md, "FieldRva", None)
    if fr:
        for row in fr.rows:
            field_rva[row.Field.row_index] = row.Rva

    def body_of(row):
        if not row.Rva:
            return None
        try:
            return CilMethodBody(
                RawReader(raw, pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return None

    def keyword_values(row):
        """get_CanonicalKeywords: either InitializeArray from an RVA blob,
        or the dup/ldc.i4 index/ldc.i4 value/stelem pattern for short
        arrays. Returns the keyword ints."""
        b = body_of(row)
        if b is None:
            return []
        insns = list(b.instructions)
        if not any(i.opcode.name == "newarr" for i in insns):
            # single/few keywords passed straight to a collection ctor or
            # Add() calls: every ldc.i4 in the method is a keyword value
            return [v for v in (_ldc_val(i) for i in insns) if v is not None]
        n_expected = 0
        # blob pattern
        for i, insn in enumerate(insns):
            nm = insn.opcode.name
            if nm.startswith("ldc.i4") and i + 1 < len(insns) \
                    and insns[i + 1].opcode.name == "newarr":
                n_expected = _ldc_val(insn)
            if nm == "ldtoken" and insn.operand is not None:
                val = insn.operand.value
                if (val >> 24) & 0xFF == 0x04:
                    rva = field_rva.get(val & 0xFFFFFF)
                    if rva and n_expected:
                        off = pe.get_offset_from_rva(rva)
                        return [int.from_bytes(
                            raw[off + 4 * i:off + 4 * i + 4], "little")
                            for i in range(n_expected)]
        # stelem pattern: ... ldc.i4 <idx>; ldc.i4 <val>; stelem.i4
        out = []
        for i, insn in enumerate(insns):
            if insn.opcode.name.startswith("stelem") and i >= 2:
                v = _ldc_val(insns[i - 1])
                if v is not None:
                    out.append(v)
        return out

    def calls_of(ti):
        calls = []
        seen = {ti}
        stack = [ti]
        added = True
        while added:
            added = False
            for n, e in enclosing.items():
                if e in seen and n not in seen:
                    seen.add(n)
                    stack.append(n)
                    added = True
        for t2 in stack:
            for name, row in type_methods[t2]:
                b = body_of(row)
                if b is None:
                    continue
                for insn in b.instructions:
                    if insn.opcode.name in ("call", "callvirt") \
                            and insn.operand is not None:
                        val = getattr(insn.operand, "value", None)
                        if val is None:
                            continue
                        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
                        if table == 0x06:
                            oi = method_owner.get(rid)
                            if oi is not None:
                                calls.append((type_ns[oi][1], str(
                                    md.MethodDef.rows[rid - 1].Name)))
        return calls

    out = {}
    for ti, (ns, name) in type_ns.items():
        if ns != CARD_NS:
            continue
        card_id = "CARD." + snake_case(name).upper()
        info = {"class": name}
        for mname, row in type_methods[ti]:
            if mname == ".ctor":
                body = body_of(row)
                if body is None:
                    raise ValueError(f"{name}: constructor body unreadable")
                constants = card_model_ctor_constants(
                    list(body.instructions), token_name)
                info["cost"], info["type"] = \
                    constructor_cost_type(constants)
            elif mname == "get_CanonicalKeywords":
                info["keywords"] = sorted(
                    keyword_names.get(v, f"kw{v}")
                    for v in keyword_values(row))
        calls = calls_of(ti)
        callset = {f"{a}::{b}" for a, b in calls}
        info["draws"] = any(
            s in callset for s in
            ("CardPileCmd::Draw",
             "CardPileCmd::DrawWithoutBlockingOnOtherPlayers"))
        info["exhausts_other"] = "CardCmd::Exhaust" in callset
        info["adds_cards"] = any(
            s in callset for s in
            ("CardPileCmd::Add", "CardPileCmd::AddGeneratedCardToCombat",
             "CardPileCmd::AddGeneratedCardsToCombat",
             "CardPileCmd::AddToCombatAndPreview"))
        info["shuffles"] = "CardPileCmd::Shuffle" in callset
        out[card_id] = info

    print(json.dumps(out, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
