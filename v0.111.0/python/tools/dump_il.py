#!/usr/bin/env python3
"""Enhanced IL dumper for sts2.dll: dumps every method of the named TypeDef(s)
AND their compiler-generated nested types (async move bodies live in
`<MoveName>d__N::MoveNext`). Resolves MethodDef/Field tokens to
DeclaringType::Name so intent ctors and cross-type calls are readable.

Usage: python3 dump_il.py TerrorEel SkulkingColony
       python3 dump_il.py --method RollMove          # dump methods by name
"""
import os
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError

DLL = Path(os.environ.get(
    "STS2_DLL",
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                   "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                   "data_sts2_macos_arm64/sts2.dll"),
))


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
    method_mode = "--method" in sys.argv
    targets = [a for a in sys.argv[1:] if not a.startswith("--")]
    pe = dnfile.dnPE(str(DLL))
    raw = DLL.read_bytes()
    md = pe.net.mdtables

    n_types = len(md.TypeDef.rows)
    n_methods = len(md.MethodDef.rows)
    n_fields = len(md.Field.rows)

    # method/field index (1-based) -> declaring TypeDef index (0-based).
    # dnfile materializes MethodList/FieldList as lists of MDTableIndex.
    method_owner = [None] * (n_methods + 2)
    field_owner = [None] * (n_fields + 2)
    type_methods = {}   # ti -> [(row_index, row), ...]
    for ti, t in enumerate(md.TypeDef.rows):
        ms = []
        for m in (t.MethodList or []):
            method_owner[m.row_index] = ti
            ms.append((m.row_index, m.row))
        type_methods[ti] = ms
        for f in (t.FieldList or []):
            field_owner[f.row_index] = ti

    # nested type -> enclosing type (both 0-based TypeDef indices)
    enclosing = {}
    nc = getattr(md, "NestedClass", None)
    if nc:
        for row in nc.rows:
            enclosing[row.NestedClass.row_index - 1] = \
                row.EnclosingClass.row_index - 1

    def type_display(ti):
        parts = []
        while ti is not None:
            t = md.TypeDef.rows[ti]
            parts.append(str(t.TypeName))
            ti = enclosing.get(ti)
        return "/".join(reversed(parts))

    ELEMENT_NAMES = {0x02: "bool", 0x08: "int32", 0x0c: "float32",
                     0x0e: "string", 0x1c: "object"}

    def typedefref_name(coded):
        table, rid = coded & 0x03, coded >> 2
        if table == 0 and rid:      # TypeDef
            return type_display(rid - 1)
        if table == 1 and rid:      # TypeRef
            r = md.TypeRef.rows[rid - 1]
            return str(r.TypeName)
        return f"tdr({table},{rid})"

    def decode_inst(blob):
        """Decode a MethodSpec instantiation blob -> list of type names."""
        try:
            data = bytes(blob.value if hasattr(blob, "value") else blob)
            pos = [0]

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
                if et in (0x12, 0x11):          # CLASS / VALUETYPE
                    return typedefref_name(compressed())
                if et == 0x15:                  # GENERICINST
                    base = ty()
                    n = compressed()
                    return f"{base}<{', '.join(ty() for _ in range(n))}>"
                if et == 0x1D:                  # SZARRAY
                    return ty() + "[]"
                return ELEMENT_NAMES.get(et, f"et{et:#x}")

            if u8() != 0x0A:                    # GENRICINST marker
                return ["?"]
            n = compressed()
            return [ty() for _ in range(n)]
        except Exception as e:
            return [f"?err:{e}"]

    def resolve_token(tok):
        val = getattr(tok, "value", tok)
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        try:
            if table == 0x0A:  # MemberRef
                row = md.MemberRef.rows[rid - 1]
                cls = row.Class.row
                cls_name = ""
                if cls is not None:
                    tn = getattr(cls, "TypeName", None)
                    tns = getattr(cls, "TypeNamespace", None)
                    cls_name = (str(tns) + "." if tns else "") + \
                        (str(tn) if tn else "?")
                return f"{cls_name}::{row.Name}"
            if table == 0x06:  # MethodDef
                row = md.MethodDef.rows[rid - 1]
                owner = method_owner[rid]
                return f"{type_display(owner)}::{row.Name}"
            if table == 0x04:  # Field
                row = md.Field.rows[rid - 1]
                owner = field_owner[rid]
                return f"{type_display(owner)}::{row.Name}"
            if table == 0x01:
                row = md.TypeRef.rows[rid - 1]
                return f"{row.TypeNamespace}.{row.TypeName}"
            if table == 0x02:
                return type_display(rid - 1)
            if table == 0x2B:  # MethodSpec
                row = md.MethodSpec.rows[rid - 1]
                inner = row.Method.row
                nm = getattr(inner, "Name", "?")
                args = decode_inst(row.Instantiation)
                return f"spec:{nm}<{', '.join(args)}>"
            if table == 0x70:
                return repr(pe.net.user_strings.get(rid).value)
        except Exception as e:
            return f"tok(0x{val:08x} err {e})"
        return f"tok(0x{val:08x})"

    def dump_method(ti, m):
        name = type_display(ti)
        mname = str(m.Name)
        rva = m.Rva
        print(f"\n--- {name}::{mname} (RVA 0x{rva:x}) ---")
        if not rva:
            print("  <no body>")
            return
        off = pe.get_offset_from_rva(rva)
        try:
            body = CilMethodBody(RawReader(raw, off))
        except MethodBodyFormatError as e:
            print(f"  <body parse error: {e}>")
            return
        for insn in body.instructions:
            op = insn.opcode.name
            operand = insn.operand
            txt = ""
            if operand is not None:
                if op in ("call", "callvirt", "newobj", "ldfld", "stfld",
                          "ldsfld", "stsfld", "ldstr", "ldtoken", "box",
                          "unbox.any", "castclass", "isinst", "ldftn",
                          "newarr", "initobj", "ldflda", "constrained."):
                    txt = resolve_token(operand)
                else:
                    txt = str(operand)
            print(f"  IL_{insn.offset:04x}: {op:14s} {txt}")

    if method_mode:
        for mi, m in enumerate(md.MethodDef.rows, 1):
            if any(t in str(m.Name) for t in targets):
                dump_method(method_owner[mi], m)
        return

    # collect target type indices + all (transitively) nested types
    roots = {ti for ti in range(n_types)
             if str(md.TypeDef.rows[ti].TypeName) in targets}
    selected = set(roots)
    changed = True
    while changed:
        changed = False
        for nested, encl in enclosing.items():
            if encl in selected and nested not in selected:
                selected.add(nested)
                changed = True

    for ti in sorted(selected):
        t = md.TypeDef.rows[ti]
        print(f"\n{'='*80}\nTYPE {type_display(ti)}")
        if ti in roots:
            for f in (t.FieldList or []):
                print(f"  field: {f.row.Name}")
        for _mi, mrow in type_methods[ti]:
            dump_method(ti, mrow)


if __name__ == "__main__":
    main()
