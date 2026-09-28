#!/usr/bin/env python3
"""Dump IL of every MethodDef whose name matches any argument substring,
printing the declaring type. Usage: dump_method.py GetDeterministicHashCode SnakeCase
"""
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase

DLL = Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                     "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                     "data_sts2_macos_arm64/sts2.dll")


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

    def tokname(tok):
        val = getattr(tok, "value", tok)
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        try:
            if table == 0x0A:
                row = md.MemberRef.rows[rid - 1]
                cls = row.Class.row
                cn = ""
                if cls is not None:
                    cn = f"{getattr(cls, 'TypeNamespace', '')}.{getattr(cls, 'TypeName', '')}"
                return f"{cn}::{row.Name}"
            if table == 0x06:
                return f"MethodDef::{md.MethodDef.rows[rid - 1].Name}"
            if table == 0x04:
                return f"Field::{md.Field.rows[rid - 1].Name}"
            if table == 0x70:
                return repr(pe.net.user_strings.get(rid).value)
        except Exception:
            pass
        return f"tok(0x{val:08x})"

    for t in md.TypeDef:
        for m in t.MethodList:
            mname = str(m.row.Name)
            if not any(x in mname for x in targets):
                continue
            print(f"\n--- {t.TypeNamespace}.{t.TypeName}::{mname} "
                  f"(RVA 0x{m.row.Rva:x}) ---")
            if not m.row.Rva:
                print("  <no body>")
                continue
            off = pe.get_offset_from_rva(m.row.Rva)
            try:
                body = CilMethodBody(RawReader(raw, off))
            except Exception as e:
                print(f"  <parse error {e}>")
                continue
            for insn in body.instructions:
                op = insn.opcode.name
                operand = insn.operand
                txt = ""
                if operand is not None:
                    if hasattr(operand, "table") or op in (
                            "call", "callvirt", "newobj", "ldstr",
                            "ldfld", "stfld", "ldsfld", "stsfld"):
                        txt = tokname(operand)
                    else:
                        txt = str(operand)
                print(f"  IL_{insn.offset:04x}: {op:14s} {txt}")


if __name__ == "__main__":
    main()
