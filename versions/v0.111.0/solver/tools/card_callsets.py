#!/usr/bin/env python3
"""Per-method internal call-sets for named card classes (accelerant recon).

For each card class named on argv (class-name substrings), print every
method it defines (including nested async state machines) with the
internal calls that method makes. Used to define the known-safe command
vocabulary for the IL-template auto-translation accelerant (#58).

Usage: card_callsets.py StrikeIronclad TwinStrike Armaments ...
"""
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError

DLL = Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                     "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                     "data_sts2_macos_arm64/sts2.dll")

CARD_NS = "MegaCrit.Sts2.Core.Models.Cards"


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
    targets = sys.argv[1:]
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

    # MemberRef names for external calls (System.Decimal etc.)
    memberref = {}
    for i, r in enumerate(md.MemberRef.rows):
        cls = r.Class
        parent = ""
        if cls.table and cls.table.name == "TypeRef":
            tr = md.TypeRef.rows[cls.row_index - 1]
            parent = str(tr.TypeName)
        memberref[i + 1] = f"{parent}::{r.Name}"

    def read_compressed(blob, i):
        b = blob[i]
        if b < 0x80:
            return b, i + 1
        if b < 0xC0:
            return ((b & 0x3F) << 8) | blob[i + 1], i + 2
        return ((b & 0x1F) << 24) | (blob[i + 1] << 16) \
            | (blob[i + 2] << 8) | blob[i + 3], i + 4

    def generic_args(blob):
        """Type names from a MethodSpec instantiation signature."""
        out, i = [], 0
        if not blob or blob[0] != 0x0A:
            return out
        n, i = read_compressed(blob, 1)
        for _ in range(n):
            while i < len(blob) and blob[i] not in (0x11, 0x12):
                i += 1                       # skip modifiers/other elements
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
            if meth.table and meth.table.name == "MethodDef":
                base = (f"{md.MethodDef.rows[meth.row_index - 1].Name}")
                owner_row = None  # resolved via method_owner at print time
                base_full = None
            else:
                base = str(md.MemberRef.rows[meth.row_index - 1].Name)
                base_full = memberref.get(meth.row_index, base)
            try:
                blob = r.Instantiation.value_bytes()
            except Exception:
                blob = b""
            args = generic_args(blob)
            if meth.table and meth.table.name == "MethodDef":
                methodspec[i + 1] = ("mdef", meth.row_index, args)
            else:
                methodspec[i + 1] = ("mref", base_full, args)

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

    for ti, (ns, name) in type_ns.items():
        if ns != CARD_NS or not any(t in name for t in targets):
            continue
        print(f"\n=== {name} ===")
        for t2 in sorted(nested_of(ti)):
            owner = type_ns[t2][1]
            for mname, row in type_methods[t2]:
                b = body_of(row)
                if b is None:
                    continue
                calls = []
                for insn in b.instructions:
                    if insn.opcode.name not in ("call", "callvirt", "newobj"):
                        continue
                    val = getattr(insn.operand, "value", None)
                    if val is None:
                        continue
                    table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
                    if table == 0x06:
                        oi = method_owner.get(rid)
                        tgt = (f"{type_ns[oi][1]}::"
                               f"{md.MethodDef.rows[rid - 1].Name}"
                               if oi is not None else f"mdef{rid}")
                    elif table == 0x0A:
                        tgt = memberref.get(rid, f"mref{rid}")
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
                        tgt = f"{base}<{','.join(args)}>"
                    else:
                        continue
                    calls.append(tgt)
                label = f"{owner}::{mname}" if t2 != ti else mname
                print(f"  {label}")
                for c in calls:
                    print(f"      {c}")


if __name__ == "__main__":
    main()
