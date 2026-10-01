#!/usr/bin/env python3
"""Scan every method body in sts2.dll for call/callvirt/ldftn whose resolved
target name contains any given substring; print DeclaringType::Method per hit.

Usage: python3 scan_calls.py ModifyDamageAdditive NextEnergyCost
"""
import os
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError

DLL = Path(os.environ["STS2_DLL"]) if os.environ.get("STS2_DLL") else \
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
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

    # target name -> set of MethodDef/MemberRef token values
    tok_names = {}
    for rid, row in enumerate(md.MethodDef.rows, 1):
        nm = str(row.Name)
        if any(t in nm for t in targets):
            tok_names[0x06000000 | rid] = nm
    for rid, row in enumerate(md.MemberRef.rows, 1):
        nm = str(row.Name)
        if any(t in nm for t in targets):
            tok_names[0x0A000000 | rid] = nm

    owner = {}
    for ti, t in enumerate(md.TypeDef.rows):
        for m in (t.MethodList or []):
            owner[m.row_index] = str(t.TypeName)

    call_ops = {"call", "callvirt", "ldftn", "newobj"}
    for mi, m in enumerate(md.MethodDef.rows, 1):
        if not m.Rva:
            continue
        off = pe.get_offset_from_rva(m.Rva)
        try:
            body = CilMethodBody(RawReader(raw, off))
        except MethodBodyFormatError:
            continue
        for insn in body.instructions:
            if insn.opcode.name in call_ops and insn.operand is not None:
                val = getattr(insn.operand, "value", None)
                if val in tok_names:
                    print(f"{owner.get(mi, '?')}::{m.Name} -> "
                          f"{tok_names[val]}")


if __name__ == "__main__":
    main()
