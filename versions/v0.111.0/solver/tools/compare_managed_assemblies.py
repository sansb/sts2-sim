#!/usr/bin/env python3
"""Compare two managed assemblies by semantic metadata and normalized CIL.

Metadata tokens and RVAs are build-local implementation details.  A raw byte
or token comparison therefore reports most rebuilt methods as changed even
when their behavior is identical.  This tool resolves types, fields, methods,
member references, generic instantiations, strings, and branch destinations
to stable semantic names before hashing a method body.

The JSON report is intended for game-version impact gates.  It records the
complete TypeDef/Field/MethodDef/RVA universe, parse failures, and exact
added/removed/changed identities.  It does not decide that a changed body is
safe; every reported behavioral delta still needs contextual review.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError


class RawReader(CilMethodBodyReaderBase):
    def __init__(self, data: bytes, offset: int):
        self.data = data
        self.base = offset
        self.i = 0

    def read(self, size: int) -> bytes:
        result = self.data[self.base + self.i:self.base + self.i + size]
        self.i += size
        return result

    def tell(self) -> int:
        return self.i

    def seek(self, offset: int) -> int:
        self.i = offset
        return self.i


class BlobReader:
    def __init__(self, value) -> None:
        self.data = bytes(value.value if hasattr(value, "value") else value)
        self.i = 0

    def byte(self) -> int:
        if self.i >= len(self.data):
            raise ValueError("truncated signature")
        result = self.data[self.i]
        self.i += 1
        return result

    def compressed_uint(self) -> int:
        first = self.byte()
        if first < 0x80:
            return first
        if first < 0xC0:
            return ((first & 0x3F) << 8) | self.byte()
        if first < 0xE0:
            return ((first & 0x1F) << 24) | (self.byte() << 16) | \
                (self.byte() << 8) | self.byte()
        raise ValueError(f"invalid compressed integer prefix {first:#x}")

    def compressed_int(self) -> int:
        start = self.i
        unsigned = self.compressed_uint()
        consumed = self.i - start
        bits = {1: 7, 2: 14, 4: 29}[consumed]
        value = unsigned >> 1
        if unsigned & 1:
            value -= 1 << (bits - 1)
        return value

    def remaining_hex(self) -> str:
        result = self.data[self.i:].hex()
        self.i = len(self.data)
        return result


class AssemblyInventory:
    PRIMITIVES = {
        0x01: "void", 0x02: "bool", 0x03: "char", 0x04: "int8",
        0x05: "uint8", 0x06: "int16", 0x07: "uint16", 0x08: "int32",
        0x09: "uint32", 0x0A: "int64", 0x0B: "uint64",
        0x0C: "float32", 0x0D: "float64", 0x0E: "string",
        0x16: "typedref", 0x18: "native-int", 0x19: "native-uint",
        0x1C: "object",
    }
    TOKEN_OPS = {
        "box", "call", "callvirt", "castclass", "constrained.",
        "initobj", "isinst", "ldelema", "ldfld", "ldflda", "ldftn",
        "ldsfld", "ldsflda", "ldstr", "ldtoken", "newarr", "newobj",
        "sizeof", "stfld", "stsfld", "unbox", "unbox.any",
    }

    def __init__(self, path: Path) -> None:
        self.path = path
        self.pe = dnfile.dnPE(str(path))
        self.raw = path.read_bytes()
        self.md = self.pe.net.mdtables
        self.enclosing: dict[int, int] = {}
        nested = getattr(self.md, "NestedClass", None)
        if nested:
            for row in nested.rows:
                self.enclosing[row.NestedClass.row_index] = \
                    row.EnclosingClass.row_index
        self.method_owner: dict[int, int] = {}
        self.field_owner: dict[int, int] = {}
        for type_rid, row in enumerate(self.md.TypeDef.rows, 1):
            for method in row.MethodList or ():
                self.method_owner[method.row_index] = type_rid
            for field in row.FieldList or ():
                self.field_owner[field.row_index] = type_rid
        self._typedef_names: dict[int, str] = {}
        self._typeref_names: dict[int, str] = {}
        self._field_keys: dict[int, str] = {}
        self._method_keys: dict[int, str] = {}

    @staticmethod
    def _flags(row) -> int:
        value = getattr(getattr(row, "struct", None), "Flags", 0)
        try:
            return int(value)
        except (TypeError, ValueError):
            return int(getattr(value, "value", 0))

    def typedef_name(self, rid: int) -> str:
        if rid in self._typedef_names:
            return self._typedef_names[rid]
        row = self.md.TypeDef.rows[rid - 1]
        name = str(row.TypeName)
        parent = self.enclosing.get(rid)
        if parent is not None:
            result = f"{self.typedef_name(parent)}/{name}"
        else:
            namespace = str(row.TypeNamespace)
            result = f"{namespace}.{name}" if namespace else name
        self._typedef_names[rid] = result
        return result

    def typeref_name(self, rid: int) -> str:
        if rid in self._typeref_names:
            return self._typeref_names[rid]
        row = self.md.TypeRef.rows[rid - 1]
        name = str(row.TypeName)
        scope = getattr(row, "ResolutionScope", None)
        if scope is not None and getattr(scope, "table", None) is not None \
                and scope.table.name == "TypeRef":
            result = f"{self.typeref_name(scope.row_index)}/{name}"
        else:
            namespace = str(row.TypeNamespace)
            result = f"{namespace}.{name}" if namespace else name
        self._typeref_names[rid] = result
        return result

    def table_index_name(self, index) -> str:
        if index is None or getattr(index, "table", None) is None:
            return "<nil>"
        table = index.table.name
        rid = index.row_index
        if table == "TypeDef":
            return self.typedef_name(rid)
        if table == "TypeRef":
            return self.typeref_name(rid)
        if table == "TypeSpec":
            return self.typespec_name(rid)
        if table == "MethodDef":
            return self.method_key(rid)
        if table == "MemberRef":
            return self.memberref_name(rid)
        if table == "ModuleRef":
            return f"module:{index.row.Name}"
        return f"{table}:{rid}"

    def typedef_or_ref(self, coded: int) -> str:
        tag, rid = coded & 0x03, coded >> 2
        if not rid:
            return "<nil-type>"
        if tag == 0:
            return self.typedef_name(rid)
        if tag == 1:
            return self.typeref_name(rid)
        if tag == 2:
            return self.typespec_name(rid)
        return f"invalid-typedeforref:{coded}"

    def typespec_name(self, rid: int) -> str:
        reader = BlobReader(self.md.TypeSpec.rows[rid - 1].Signature)
        return self.parse_type(reader)

    def parse_type(self, reader: BlobReader) -> str:
        element = reader.byte()
        if element in self.PRIMITIVES:
            return self.PRIMITIVES[element]
        if element == 0x0F:
            return f"ptr<{self.parse_type(reader)}>"
        if element == 0x10:
            return f"byref<{self.parse_type(reader)}>"
        if element in (0x11, 0x12):
            kind = "valuetype" if element == 0x11 else "class"
            return f"{kind}<{self.typedef_or_ref(reader.compressed_uint())}>"
        if element == 0x13:
            return f"var<{reader.compressed_uint()}>"
        if element == 0x14:
            item = self.parse_type(reader)
            rank = reader.compressed_uint()
            sizes = [reader.compressed_uint()
                     for _ in range(reader.compressed_uint())]
            lowers = [reader.compressed_int()
                      for _ in range(reader.compressed_uint())]
            return f"array<{item};rank={rank};sizes={sizes};lowers={lowers}>"
        if element == 0x15:
            kind_element = reader.byte()
            if kind_element not in (0x11, 0x12):
                raise ValueError(
                    f"genericinst has invalid kind {kind_element:#x}")
            base = self.typedef_or_ref(reader.compressed_uint())
            args = [self.parse_type(reader)
                    for _ in range(reader.compressed_uint())]
            return f"generic<{base};{','.join(args)}>"
        if element == 0x1B:
            return f"fnptr<{self.parse_method_signature(reader)}>"
        if element == 0x1D:
            return f"szarray<{self.parse_type(reader)}>"
        if element == 0x1E:
            return f"mvar<{reader.compressed_uint()}>"
        if element in (0x1F, 0x20):
            kind = "modreq" if element == 0x1F else "modopt"
            modifier = self.typedef_or_ref(reader.compressed_uint())
            return f"{kind}<{modifier};{self.parse_type(reader)}>"
        if element == 0x41:
            return "sentinel"
        if element == 0x45:
            return f"pinned<{self.parse_type(reader)}>"
        raise ValueError(
            f"unsupported signature element {element:#x} at {reader.i - 1}")

    def parse_method_signature(self, reader: BlobReader) -> str:
        callconv = reader.byte()
        generic_count = reader.compressed_uint() if callconv & 0x10 else 0
        parameter_count = reader.compressed_uint()
        result = self.parse_type(reader)
        parameters = []
        while len([p for p in parameters if p != "sentinel"]) \
                < parameter_count:
            parameters.append(self.parse_type(reader))
        return (f"cc={callconv:#x};g={generic_count};"
                f"ret={result};args=({','.join(parameters)})")

    def signature(self, value) -> str:
        reader = BlobReader(value)
        try:
            first = reader.data[0]
            if first == 0x06:
                reader.byte()
                result = f"field:{self.parse_type(reader)}"
            elif first == 0x07:
                reader.byte()
                locals_ = [self.parse_type(reader)
                           for _ in range(reader.compressed_uint())]
                result = f"locals:({','.join(locals_)})"
            elif first == 0x0A:
                reader.byte()
                args = [self.parse_type(reader)
                        for _ in range(reader.compressed_uint())]
                result = f"inst:({','.join(args)})"
            elif (first & 0x0F) == 0x08:
                result = f"property:{self.parse_method_signature(reader)}"
            else:
                result = self.parse_method_signature(reader)
            if reader.i != len(reader.data):
                result += f";trailing={reader.remaining_hex()}"
            return result
        except (IndexError, ValueError) as error:
            return f"raw:{reader.data.hex()};parse_error={error}"

    def field_key(self, rid: int) -> str:
        if rid not in self._field_keys:
            row = self.md.Field.rows[rid - 1]
            owner = self.typedef_name(self.field_owner[rid])
            self._field_keys[rid] = \
                f"{owner}::{row.Name} [{self.signature(row.Signature)}]"
        return self._field_keys[rid]

    def method_key(self, rid: int) -> str:
        if rid not in self._method_keys:
            row = self.md.MethodDef.rows[rid - 1]
            owner = self.typedef_name(self.method_owner[rid])
            self._method_keys[rid] = \
                f"{owner}::{row.Name} [{self.signature(row.Signature)}]"
        return self._method_keys[rid]

    def memberref_name(self, rid: int) -> str:
        row = self.md.MemberRef.rows[rid - 1]
        return (f"{self.table_index_name(row.Class)}::{row.Name} "
                f"[{self.signature(row.Signature)}]")

    def token_name(self, token) -> str:
        value = int(getattr(token, "value", token))
        table, rid = (value >> 24) & 0xFF, value & 0xFFFFFF
        try:
            if table == 0x01:
                return self.typeref_name(rid)
            if table == 0x02:
                return self.typedef_name(rid)
            if table == 0x04:
                return self.field_key(rid)
            if table == 0x06:
                return self.method_key(rid)
            if table == 0x0A:
                return self.memberref_name(rid)
            if table == 0x11:
                row = self.md.StandAloneSig.rows[rid - 1]
                return self.signature(row.Signature)
            if table == 0x1B:
                return self.typespec_name(rid)
            if table == 0x2B:
                row = self.md.MethodSpec.rows[rid - 1]
                return (f"{self.table_index_name(row.Method)} "
                        f"[{self.signature(row.Instantiation)}]")
            if table == 0x70:
                return repr(self.pe.net.user_strings.get(rid).value)
        except (AttributeError, IndexError, KeyError, TypeError, ValueError) \
                as error:
            return f"token:{value:#010x}:resolve_error={error}"
        return f"token:{value:#010x}"

    def normalize_operand(self, opcode: str, operand, offsets: dict) -> object:
        if operand is None:
            return None
        if isinstance(operand, (list, tuple)):
            return [self.normalize_operand(opcode, value, offsets)
                    for value in operand]
        if opcode in self.TOKEN_OPS:
            return self.token_name(operand)
        target_offset = getattr(operand, "offset", None)
        if target_offset is not None:
            return {"target": offsets.get(int(target_offset), int(target_offset))}
        value = getattr(operand, "value", operand)
        if opcode.startswith(("br", "leave")) or opcode in ("switch",):
            if isinstance(value, int):
                return {"target": offsets.get(value, value)}
        if isinstance(value, bytes):
            return {"bytes": value.hex()}
        if isinstance(value, (bool, float, int, str)):
            return value
        return str(value)

    def body(self, rid: int) -> tuple[str | None, str | None]:
        row = self.md.MethodDef.rows[rid - 1]
        if not row.Rva:
            return None, None
        try:
            parsed = CilMethodBody(
                RawReader(self.raw, self.pe.get_offset_from_rva(row.Rva)))
            offsets = {int(instruction.offset): index
                       for index, instruction in enumerate(parsed.instructions)}
            normalized = [
                [instruction.opcode.name,
                 self.normalize_operand(
                     instruction.opcode.name, instruction.operand, offsets)]
                for instruction in parsed.instructions
            ]
            payload = json.dumps(
                normalized, sort_keys=True, separators=(",", ":"))
            return hashlib.sha256(payload.encode()).hexdigest(), None
        except (MethodBodyFormatError, ValueError, TypeError, IndexError) \
                as error:
            return None, f"{type(error).__name__}: {error}"

    def inventory(self) -> dict:
        types = {}
        for rid, row in enumerate(self.md.TypeDef.rows, 1):
            extends = self.table_index_name(row.Extends) \
                if getattr(row, "Extends", None) is not None else "<nil>"
            types[self.typedef_name(rid)] = {
                "flags": self._flags(row), "extends": extends}

        fields = {
            self.field_key(rid): {"flags": self._flags(row)}
            for rid, row in enumerate(self.md.Field.rows, 1)
        }
        methods = {
            self.method_key(rid): {"flags": self._flags(row)}
            for rid, row in enumerate(self.md.MethodDef.rows, 1)
        }
        bodies = {}
        parse_errors = {}
        for rid, row in enumerate(self.md.MethodDef.rows, 1):
            if not row.Rva:
                continue
            digest, error = self.body(rid)
            key = self.method_key(rid)
            if error is not None:
                parse_errors[key] = error
            else:
                bodies[key] = digest

        return {
            "path": str(self.path),
            "sha256": hashlib.sha256(self.raw).hexdigest(),
            "mvid": str(self.md.Module.rows[0].Mvid),
            "counts": {
                "typedef": len(self.md.TypeDef.rows),
                "field": len(self.md.Field.rows),
                "methoddef": len(self.md.MethodDef.rows),
                "rva_methods": sum(bool(row.Rva)
                                   for row in self.md.MethodDef.rows),
                "parsed_bodies": len(bodies),
                "parse_errors": len(parse_errors),
            },
            "types": types,
            "fields": fields,
            "methods": methods,
            "bodies": bodies,
            "parse_error_details": parse_errors,
        }


def delta(old: dict, new: dict, key: str) -> dict:
    before, after = old[key], new[key]
    shared = before.keys() & after.keys()
    return {
        "removed": sorted(before.keys() - after.keys()),
        "added": sorted(after.keys() - before.keys()),
        "changed": sorted(name for name in shared
                          if before[name] != after[name]),
    }


def summarize(change: dict) -> dict:
    return {kind: len(values) for kind, values in change.items()}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("old", type=Path)
    parser.add_argument("new", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    old = AssemblyInventory(args.old).inventory()
    new = AssemblyInventory(args.new).inventory()
    changes = {key: delta(old, new, key)
               for key in ("types", "fields", "methods", "bodies")}
    report = {
        "old": {key: old[key] for key in
                ("path", "sha256", "mvid", "counts",
                 "parse_error_details")},
        "new": {key: new[key] for key in
                ("path", "sha256", "mvid", "counts",
                 "parse_error_details")},
        "summary": {key: summarize(value)
                    for key, value in changes.items()},
        "delta": changes,
    }
    encoded = json.dumps(report, indent=1, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
