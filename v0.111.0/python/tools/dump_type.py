#!/usr/bin/env python3
"""Dump full IL disassembly of every method of the named TypeDef(s) in sts2.dll.

Usage: python3 dump_type.py MegaRandom Rng RunRngSet
"""
import sys
from pathlib import Path

try:
    # dnfile/dncil are only installed under /usr/local/bin/python3.12 (see
    # sim/v0.111.0/python/tools/dump_type.py's usage note and the repo
    # CLAUDE.md's IL-tools section). Guard the import so this module — and
    # in particular resolve_nested_targets(), the pure filter logic below —
    # stays importable under the plain pytest interpreter, which lacks them.
    import dnfile
    from dncil.cil.body import CilMethodBody
    from dncil.cil.body.reader import CilMethodBodyReaderBase
    from dncil.cil.error import MethodBodyFormatError
except ImportError:
    dnfile = None
    CilMethodBody = None
    CilMethodBodyReaderBase = object
    MethodBodyFormatError = Exception

DLL = Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                     "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                     "data_sts2_macos_arm64/sts2.dll")


def resolve_nested_targets(targets, type_names, nested_of, out=None):
    """Expand 'Enclosing/Nested' and 'Enclosing/' query targets.

    `targets` is the raw set of command-line target strings (a mix of plain
    type names and 'Enclosing/Nested' or 'Enclosing/' forms). `type_names` is
    a list of TypeName strings indexed by (1-based TypeDef rid - 1).
    `nested_of` maps a nested type's 1-based rid to its enclosing type's
    1-based rid, per the NestedClass table. `out`, if given, is called with
    each "Enclosing/Nested" listing line for a bare 'Enclosing/' query.

    Returns (remaining_targets, qualified_rids): remaining_targets is the
    input set with every '/'-form entry removed, and qualified_rids is the
    set of 1-based TypeDef rids that satisfied an 'Enclosing/Nested' query —
    matched via the full enclosing chain, not by nested name alone, so two
    unrelated enclosing classes sharing a nested name (e.g. two different
    cards both compiling an `<OnPlay>d__3`) never cross-match.
    """
    targets = set(targets)
    qualified = set()
    for tgt in [t for t in targets if "/" in t]:
        targets.discard(tgt)
        enc_name, _, nest_name = tgt.partition("/")
        for rid, enc_rid in nested_of.items():
            if type_names[enc_rid - 1] != enc_name:
                continue
            nested = type_names[rid - 1]
            if not nest_name:
                if out is not None:
                    out(f"{enc_name}/{nested}")
            elif nest_name in nested:
                qualified.add(rid)
    return targets, qualified


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
    if dnfile is None:
        sys.exit(
            "error: dnfile/dncil are not installed under this interpreter; "
            "run with /usr/local/bin/python3.12 (see CLAUDE.md's IL-tools note)"
        )
    targets = set(sys.argv[1:])
    pe = dnfile.dnPE(str(DLL))
    raw = DLL.read_bytes()
    md = pe.net.mdtables

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
                    cls_name = (str(tns) + "." if tns else "") + (str(tn) if tn else "?")
                return f"{cls_name}::{row.Name}"
            if table == 0x06:  # MethodDef
                row = md.MethodDef.rows[rid - 1]
                return f"MethodDef::{row.Name}"
            if table == 0x04:  # Field
                row = md.Field.rows[rid - 1]
                return f"Field::{row.Name}"
            if table == 0x01:  # TypeRef
                row = md.TypeRef.rows[rid - 1]
                return f"{row.TypeNamespace}.{row.TypeName}"
            if table == 0x02:  # TypeDef
                row = md.TypeDef.rows[rid - 1]
                return f"{row.TypeNamespace}.{row.TypeName}"
            if table == 0x70:  # user string
                return repr(pe.net.user_strings.get(rid).value)
        except Exception as e:
            return f"tok(0x{val:08x} err {e})"
        return f"tok(0x{val:08x})"

    # "Enclosing/<Nested>d__N" targets (async/iterator state machines: the
    # real body of an async hook lives in <MethodName>d__N::MoveNext, and
    # those nested names are duplicated across classes — qualify via the
    # NestedClass table). "Enclosing/" alone lists that type's nested types.
    # See resolve_nested_targets() for why this matches the full enclosing
    # chain (by TypeDef rid) rather than the nested name alone.
    nested_of = {}                               # nested rid -> enclosing rid
    for row in md.NestedClass:
        nested_of[row.NestedClass.row_index] = row.EnclosingClass.row_index
    type_names = [str(t.TypeName) for t in md.TypeDef]
    targets, qualified = resolve_nested_targets(targets, type_names, nested_of, out=print)

    for idx, t in enumerate(md.TypeDef):
        name = str(t.TypeName)
        if name not in targets and (idx + 1) not in qualified:
            continue
        ns = str(t.TypeNamespace)
        print(f"\n{'='*80}\nTYPE {ns}.{name}")
        for f in t.FieldList:
            print(f"  field: {f.row.Name}")
        for m in t.MethodList:
            mname = str(m.row.Name)
            rva = m.row.Rva
            print(f"\n--- METHOD {name}::{mname} (RVA 0x{rva:x}) ---")
            if not rva:
                print("  <no body>")
                continue
            off = pe.get_offset_from_rva(rva)
            try:
                body = CilMethodBody(RawReader(raw, off))
            except MethodBodyFormatError as e:
                print(f"  <body parse error: {e}>")
                continue
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


if __name__ == "__main__":
    main()
