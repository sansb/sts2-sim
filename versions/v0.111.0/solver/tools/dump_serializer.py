#!/usr/bin/env python3
"""Dump IL of Serialize/Deserialize methods with generic tokens resolved.

Unlike dump_type.py, this resolves MethodSpec (generic method
instantiation) and TypeSpec tokens, so calls like
`writer.Write<SerializableRun>(...)` show their type arguments — what you
need to walk a serialization graph (.mcr replay format, net packets).

Usage:
  dump_serializer.py TypeName [TypeName ...]   # dump all methods of type
  dump_serializer.py --serialize-only Type ... # only Serialize/Deserialize
  dump_serializer.py --enums EnumName ...      # enum members + wire bits
"""
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase

DLL = Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                     "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                     "data_sts2_macos_arm64/sts2.dll")

ELEMENT_NAMES = {
    0x01: "void", 0x02: "bool", 0x03: "char", 0x04: "int8", 0x05: "uint8",
    0x06: "int16", 0x07: "uint16", 0x08: "int32", 0x09: "uint32",
    0x0A: "int64", 0x0B: "uint64", 0x0C: "float32", 0x0D: "float64",
    0x0E: "string", 0x18: "intptr", 0x19: "uintptr", 0x1C: "object",
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


class Resolver:
    def __init__(self):
        self.pe = dnfile.dnPE(str(DLL))
        self.raw = DLL.read_bytes()
        self.md = self.pe.net.mdtables
        # TypeDef rid -> owning TypeDef rid for MethodDef lookup
        self._method_owner = {}
        for ti, t in enumerate(self.md.TypeDef, 1):
            for m in t.MethodList:
                self._method_owner[id(m.row)] = ti

    # --- compressed-int + type-sig parsing (ECMA-335 II.23.2) ---
    def _read_compressed(self, blob, i):
        b0 = blob[i]
        if b0 & 0x80 == 0:
            return b0, i + 1
        if b0 & 0xC0 == 0x80:
            return ((b0 & 0x3F) << 8) | blob[i + 1], i + 2
        return ((b0 & 0x1F) << 24) | (blob[i + 1] << 16) | \
            (blob[i + 2] << 8) | blob[i + 3], i + 4

    def _typedefref_name(self, coded):
        # coded TypeDefOrRef: tag in LOW 2 bits
        tag, rid = coded & 0x3, coded >> 2
        if tag == 0:  # TypeDef
            r = self.md.TypeDef.rows[rid - 1]
            return f"{r.TypeNamespace}.{r.TypeName}"
        if tag == 1:  # TypeRef
            r = self.md.TypeRef.rows[rid - 1]
            return f"{r.TypeNamespace}.{r.TypeName}"
        return f"typespec#{rid}"

    def parse_type_sig(self, blob, i=0):
        """Returns (human-name, next-index)."""
        et = blob[i]
        i += 1
        if et in ELEMENT_NAMES:
            return ELEMENT_NAMES[et], i
        if et in (0x11, 0x12):  # VALUETYPE / CLASS
            coded, i = self._read_compressed(blob, i)
            return self._typedefref_name(coded), i
        if et == 0x15:  # GENERICINST
            base, i = self.parse_type_sig(blob, i)
            n, i = self._read_compressed(blob, i)
            args = []
            for _ in range(n):
                a, i = self.parse_type_sig(blob, i)
                args.append(a)
            return f"{base}<{', '.join(args)}>", i
        if et == 0x1D:  # SZARRAY
            base, i = self.parse_type_sig(blob, i)
            return f"{base}[]", i
        if et == 0x13:  # VAR (class generic param)
            n, i = self._read_compressed(blob, i)
            return f"!{n}", i
        if et == 0x1E:  # MVAR
            n, i = self._read_compressed(blob, i)
            return f"!!{n}", i
        if et == 0x0F:  # PTR
            base, i = self.parse_type_sig(blob, i)
            return f"{base}*", i
        if et == 0x10:  # BYREF
            base, i = self.parse_type_sig(blob, i)
            return f"{base}&", i
        if et == 0x1F or et == 0x20:  # CMOD
            _, i = self._read_compressed(blob, i)
            return self.parse_type_sig(blob, i)
        return f"et0x{et:02x}", i

    def methodspec_name(self, rid):
        row = self.md.MethodSpec.rows[rid - 1]
        target = row.Method.row
        tname = ""
        if hasattr(target, "Name") and hasattr(target, "Class"):  # MemberRef
            cls = target.Class.row
            if cls is not None:
                tname = f"{getattr(cls, 'TypeNamespace', '')}." \
                        f"{getattr(cls, 'TypeName', '')}"
            mname = str(target.Name)
        else:  # MethodDef
            owner_rid = self._method_owner.get(id(target))
            if owner_rid:
                tr = self.md.TypeDef.rows[owner_rid - 1]
                tname = f"{tr.TypeNamespace}.{tr.TypeName}"
            mname = str(target.Name)
        blob = bytes(row.Instantiation.value_bytes()
                     if hasattr(row.Instantiation, "value_bytes")
                     else row.Instantiation.raw_data)
        # sig: 0x0A GENRICINST, count, types
        i = 0
        if blob and blob[0] == 0x0A:
            i = 1
            n, i = self._read_compressed(blob, i)
            args = []
            for _ in range(n):
                a, i = self.parse_type_sig(blob, i)
                args.append(a)
            return f"{tname}::{mname}<{', '.join(args)}>"
        return f"{tname}::{mname}<?>"

    def typespec_name(self, rid):
        row = self.md.TypeSpec.rows[rid - 1]
        blob = bytes(row.Signature.value_bytes()
                     if hasattr(row.Signature, "value_bytes")
                     else row.Signature.raw_data)
        name, _ = self.parse_type_sig(blob, 0)
        return name

    def resolve_token(self, tok):
        val = getattr(tok, "value", tok)
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        try:
            if table == 0x2B:
                return self.methodspec_name(rid)
            if table == 0x1B:
                return self.typespec_name(rid)
            if table == 0x0A:
                row = self.md.MemberRef.rows[rid - 1]
                cls = row.Class.row
                cn = ""
                if cls is not None:
                    tns = getattr(cls, "TypeNamespace", "")
                    tn = getattr(cls, "TypeName", "")
                    cn = f"{tns}.{tn}"
                return f"{cn}::{row.Name}"
            if table == 0x06:
                row = self.md.MethodDef.rows[rid - 1]
                owner = self._method_owner.get(id(row))
                if owner:
                    tr = self.md.TypeDef.rows[owner - 1]
                    return f"{tr.TypeName}::{row.Name}"
                return f"::{row.Name}"
            if table == 0x04:
                return f"Field::{self.md.Field.rows[rid - 1].Name}"
            if table == 0x01:
                row = self.md.TypeRef.rows[rid - 1]
                return f"{row.TypeNamespace}.{row.TypeName}"
            if table == 0x02:
                row = self.md.TypeDef.rows[rid - 1]
                return f"{row.TypeNamespace}.{row.TypeName}"
            if table == 0x70:
                return repr(self.pe.net.user_strings.get(rid).value)
        except Exception as e:
            return f"tok(0x{val:08x} err:{e})"
        return f"tok(0x{val:08x})"

    def find_typedefs(self, name):
        out = []
        for t in self.md.TypeDef:
            if str(t.TypeName) == name or \
                    f"{t.TypeNamespace}.{t.TypeName}" == name:
                out.append(t)
        return out

    def dump_method(self, m, only_serialize=False):
        mname = str(m.row.Name)
        rva = m.row.Rva
        lines = [f"--- {mname} (RVA 0x{rva:x}) ---"]
        if not rva:
            lines.append("  <no body>")
            return lines
        off = self.pe.get_offset_from_rva(rva)
        try:
            body = CilMethodBody(RawReader(self.raw, off))
        except Exception as e:
            lines.append(f"  <parse error {e}>")
            return lines
        for insn in body.instructions:
            op = insn.opcode.name
            operand = insn.operand
            txt = ""
            if operand is not None:
                if op in ("call", "callvirt", "newobj", "ldfld", "stfld",
                          "ldsfld", "stsfld", "ldstr", "ldtoken", "box",
                          "unbox.any", "castclass", "isinst", "ldftn",
                          "newarr", "initobj", "ldflda", "constrained.",
                          "ldobj", "stobj"):
                    txt = self.resolve_token(operand)
                else:
                    txt = str(operand)
            lines.append(f"  IL_{insn.offset:04x}: {op:14s} {txt}")
        return lines

    def dump_type(self, t, methods=None):
        ns, name = str(t.TypeNamespace), str(t.TypeName)
        print(f"\n{'=' * 80}\nTYPE {ns}.{name}")
        print("  fields:", ", ".join(str(f.row.Name) for f in t.FieldList))
        for m in t.MethodList:
            if methods and str(m.row.Name) not in methods:
                continue
            print()
            print("\n".join(self.dump_method(m)))

    def enum_info(self, t):
        """(members, wire bit width) for an enum TypeDef."""
        members = [str(f.row.Name) for f in t.FieldList
                   if str(f.row.Name) != "value__"]
        import math
        n = len(members)
        bits = math.ceil(math.log2(n) + 1) if n > 1 else 1
        return members, bits


def main():
    argv = sys.argv[1:]
    r = Resolver()
    if argv and argv[0] == "--enums":
        for name in argv[1:]:
            for t in r.find_typedefs(name):
                members, bits = r.enum_info(t)
                print(f"{t.TypeNamespace}.{t.TypeName}: {len(members)} "
                      f"members, {bits} wire bits")
                for i, m in enumerate(members):
                    print(f"  {i}: {m}")
        return
    serialize_only = "--serialize-only" in argv
    argv = [a for a in argv if a != "--serialize-only"]
    for name in argv:
        for t in r.find_typedefs(name):
            r.dump_type(t, methods={"Serialize", "Deserialize"}
                        if serialize_only else None)


if __name__ == "__main__":
    main()
