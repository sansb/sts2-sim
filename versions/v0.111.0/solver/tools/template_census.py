#!/usr/bin/env python3
"""Template census for the auto-translation accelerant (#58).

For every card class in sts2.dll, dump what the accelerant needs to
decide whether the card is template-simple:
  - hooks:    gameplay methods overridden beyond the declarative /
              cosmetic whitelist (anything here = manual read required)
  - keywords: canonical keywords (decoded ints -> names)
  - calls:    deduped call targets reachable from the card type and its
              nested async state machines, EXCLUDING the declarative /
              cosmetic methods themselves (so OnUpgrade's Decimal math,
              hover tips, portraits etc. don't pollute the effect set).
              MethodSpec calls resolve generic args: PowerCmd::Apply<X>.

Output: JSON {CARD.ID: {class, cost, type, hooks, keywords, calls}}.
Classification against the safe vocabulary happens in
versions/v0.111.0/solver/accelerant_report.py, not here.

Usage: template_census.py > ../template_census.json
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
from tools.census_cards import (  # noqa: E402
    card_model_ctor_constants, constructor_cost_type)

DLL = Path(os.environ.get(
    "STS2_DLL",
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                   "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                   "data_sts2_macos_arm64/sts2.dll"),
))

CARD_NS = "MegaCrit.Sts2.Core.Models.Cards"
TYPE_NAMES = {1: "attack", 2: "skill", 3: "power", 5: "curse", 6: "quest",
              4: "status"}

# Declarative or cosmetic methods: their bodies never touch combat state,
# so they are neither hooks nor sources of effect calls. OnPlay is handled
# specially (it IS the behavior). Everything else = a gameplay hook.
DECLARATIVE = {
    ".ctor", "OnUpgrade", "AfterDowngraded",
    "get_CanonicalVars", "get_CanonicalKeywords", "get_CanonicalTags",
    "get_ExtraHoverTips", "get_ExtraRunAssetPaths", "get_MaxUpgradeLevel",
    "get_MultiplayerConstraint", "get_CanonicalStarCost",
    "get_CanBeGeneratedInCombat", "get_CanBeGeneratedByModifiers",
    "get_ShouldGlowGoldInternal", "get_ShouldGlowRedInternal",
    "get_VisualCardPool", "get_GainsBlock", "get_TargetType",
    "get_PortraitPath", "get_AllPortraitPaths", "GetPortraitPath",
    "GetPortraitFilename", "OnEnqueuePlayVfx",
}


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

    type_ns, type_methods, method_owner = {}, {}, {}
    for ti, t in enumerate(md.TypeDef.rows):
        type_ns[ti] = (str(t.TypeNamespace), str(t.TypeName))
        ms = []
        for m in (t.MethodList or []):
            ms.append((str(m.row.Name), m.row))
            method_owner[m.row_index] = ti
        type_methods[ti] = ms

    enclosing = {}
    nc = getattr(md, "NestedClass", None)
    if nc:
        for row in nc.rows:
            enclosing[row.NestedClass.row_index - 1] = \
                row.EnclosingClass.row_index - 1

    memberref = {}
    for i, r in enumerate(md.MemberRef.rows):
        cls = r.Class
        parent = ""
        if cls.table and cls.table.name == "TypeRef":
            parent = str(md.TypeRef.rows[cls.row_index - 1].TypeName)
        memberref[i + 1] = f"{parent}::{r.Name}"

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

    def read_compressed(blob, i):
        b = blob[i]
        if b < 0x80:
            return b, i + 1
        if b < 0xC0:
            return ((b & 0x3F) << 8) | blob[i + 1], i + 2
        return ((b & 0x1F) << 24) | (blob[i + 1] << 16) \
            | (blob[i + 2] << 8) | blob[i + 3], i + 4

    def generic_args(blob):
        out, i = [], 0
        if not blob or blob[0] != 0x0A:
            return out
        n, i = read_compressed(blob, 1)
        for _ in range(n):
            while i < len(blob) and blob[i] not in (0x11, 0x12):
                i += 1
            if i >= len(blob):
                break
            i += 1
            coded, i = read_compressed(blob, i)
            row, tbl = coded >> 2, coded & 3
            if tbl == 0 and row - 1 < len(md.TypeDef.rows):
                out.append(str(md.TypeDef.rows[row - 1].TypeName))
            elif tbl == 1 and row - 1 < len(md.TypeRef.rows):
                out.append(str(md.TypeRef.rows[row - 1].TypeName))
            else:
                out.append(f"typespec{row}")
        return out

    methodspec = {}
    ms_table = getattr(md, "MethodSpec", None)
    if ms_table:
        for i, r in enumerate(ms_table.rows):
            meth = r.Method
            try:
                blob = r.Instantiation.value_bytes()
            except Exception:
                blob = b""
            args = generic_args(blob)
            if meth.table and meth.table.name == "MethodDef":
                methodspec[i + 1] = ("mdef", meth.row_index, args)
            else:
                methodspec[i + 1] = (
                    "mref", memberref.get(meth.row_index, "?"), args)

    def body_of(row):
        if not row.Rva:
            return None
        try:
            return CilMethodBody(
                RawReader(raw, pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return None

    def nested_of(ti):
        seen, added = {ti}, True
        while added:
            added = False
            for n, e in enclosing.items():
                if e in seen and n not in seen:
                    seen.add(n)
                    added = True
        return seen

    def calls_in(row):
        b = body_of(row)
        if b is None:
            return []
        out = []
        for insn in b.instructions:
            if insn.opcode.name not in ("call", "callvirt", "newobj"):
                continue
            val = getattr(insn.operand, "value", None)
            if val is None:
                continue
            table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
            if table == 0x06:
                oi = method_owner.get(rid)
                out.append(f"{type_ns[oi][1] if oi is not None else '?'}::"
                           f"{md.MethodDef.rows[rid - 1].Name}")
            elif table == 0x0A:
                out.append(memberref.get(rid, f"mref{rid}"))
            elif table == 0x2B:
                spec = methodspec.get(rid)
                if spec is None:
                    continue
                kind, ref, args = spec
                if kind == "mdef":
                    oi = method_owner.get(ref)
                    base = (f"{type_ns[oi][1]}::"
                            f"{md.MethodDef.rows[ref - 1].Name}"
                            if oi is not None else f"mdef{ref}")
                else:
                    base = ref
                out.append(f"{base}<{','.join(args)}>")
        return out

    # keyword decode (same approach as census_cards.py, reused inline)
    keyword_names = {}
    for ti, t in enumerate(md.TypeDef.rows):
        if str(t.TypeName) == "CardKeyword":
            idx = {f.row_index: str(f.row.Name) for f in (t.FieldList or [])}
            for c in md.Constant.rows:
                p = c.Parent
                if p.table and p.table.name == "Field" \
                        and p.row_index in idx and idx[p.row_index] != "value__":
                    keyword_names[int.from_bytes(
                        c.Value.value_bytes()[:4], "little")] = \
                        idx[p.row_index]
    field_rva = {}
    fr = getattr(md, "FieldRva", None)
    if fr:
        for row in fr.rows:
            field_rva[row.Field.row_index] = row.Rva

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

    def keyword_values(row):
        b = body_of(row)
        if b is None:
            return []
        insns = list(b.instructions)
        if not any(i.opcode.name == "newarr" for i in insns):
            return [v for v in (_ldc_val(i) for i in insns) if v is not None]
        n_expected = 0
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
        out = []
        for i, insn in enumerate(insns):
            if insn.opcode.name.startswith("stelem") and i >= 2:
                v = _ldc_val(insns[i - 1])
                if v is not None:
                    out.append(v)
        return out

    def const_ints(row):
        b = body_of(row)
        return [] if b is None else \
            [v for v in (_ldc_val(i) for i in b.instructions)
             if v is not None]

    # async state machines belong to a specific source method: map each
    # nested type to the method whose name is embedded in '<Name>d__N'
    def sm_source(nested_name):
        if nested_name.startswith("<") and ">" in nested_name:
            return nested_name[1:nested_name.index(">")]
        return None

    out = {}
    for ti, (ns, name) in type_ns.items():
        if ns != CARD_NS or name.startswith("<"):
            continue
        card_id = "CARD." + snake_case(name).upper()
        info = {"class": name, "hooks": [], "keywords": [], "calls": []}
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
        calls = set()
        for t2 in nested_of(ti):
            nested_name = type_ns[t2][1]
            src = sm_source(nested_name) if t2 != ti else None
            for mname, row in type_methods[t2]:
                if t2 == ti:
                    if mname in DECLARATIVE:
                        continue
                    base = mname.split(">")[0].lstrip("<")
                    if mname.startswith("<") and base in DECLARATIVE:
                        continue                 # lambda of a declarative
                    if mname != "OnPlay" and not mname.startswith("<OnPlay>"):
                        if not mname.startswith("get_Has"):
                            info["hooks"].append(mname)
                else:
                    if src in DECLARATIVE:
                        continue
                    if src is not None and src != "OnPlay":
                        # state machine of a hook: hook already recorded
                        # via the stub on the card type; still collect
                        # calls so complexity is visible
                        pass
                calls.update(calls_in(row))
        info["hooks"] = sorted(set(info["hooks"]))
        info["calls"] = sorted(calls)
        out[card_id] = info

    print(json.dumps(out, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
