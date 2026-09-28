#!/usr/bin/env python3
"""Potion census for the coverage pipeline (#58, roadmap #120 D4).

For every potion class in sts2.dll, extract the CanonicalVars — the
declarative (name, value) pairs the game substitutes into the potion's
tooltip — by parsing the get_CanonicalVars IL structurally, plus the
English tooltip template from the Godot pack's localization files.

Structural parsing is the point: the Radiant Tincture misread
(2026-07-14) came from eyeballing raw IL — the `ldc.i4.2` that is the
DynamicVar[] ARRAY SIZE was read as EnergyVar(2). Here array sizes and
element indexes are consumed by the array grammar and can never leak
into a var value.

get_CanonicalVars grammar (every potion in the shipped DLL fits it):

  single var:   <payload> newobj XVar::.ctor
                newobj <>z__ReadOnlySingleElementList`1::.ctor  ret
  var array:    ldc.i4.N  newarr DynamicVar
                { dup  ldc.i4.<idx>  <payload>  newobj XVar::.ctor
                  stelem.ref }*N
                newobj <>z__ReadOnlyArray`1::.ctor  ret
  payload:      [ldstr <name>]  value  [newobj Decimal::.ctor]  [props...]
                where value = ldc.i4* int or ldsfld Decimal::One (= 1)

Var naming (matches the {Placeholder} keys in the tooltips):
  DynamicVar(name, v)      -> the ldstr name       (e.g. HealPercent)
  PowerVar`1<XPower>(v)    -> the power class name (e.g. RadiancePower)
  XVar(v)                  -> class minus "Var"    (e.g. Energy, Cards)

Output: JSON {POTION.ID: {class, vars: {name: value},
var_props: {name: [ints]} (extra ctor int args, e.g. DamageVar
props=4 nonCardUnpowered — only when present), tooltip_eng}}.

Usage: census_potions.py > ../potions_census.json
"""
import json
import os
import re
import struct
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase
from dncil.cil.error import MethodBodyFormatError

sys.path.insert(0, str(Path(__file__).parent.parent))
from sts2_rng import snake_case  # noqa: E402

RESOURCES = Path.home() / ("Library/Application Support/Steam/steamapps/"
                           "common/Slay the Spire 2/SlayTheSpire2.app/"
                           "Contents/Resources")
DLL = Path(os.environ.get(
    "STS2_DLL", RESOURCES / "data_sts2_macos_arm64/sts2.dll"))
PCK = RESOURCES / "Slay the Spire 2.pck"

POTION_NS = "MegaCrit.Sts2.Core.Models.Potions"


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


def read_pck_potion_tooltips():
    """localization/eng/potions.json out of the Godot pack (GDPC v3;
    file offsets are file_base-relative when flags bit 1 is set)."""
    f = open(PCK, "rb")
    magic = f.read(4)
    ver, _, _, _, flags = struct.unpack("<5I", f.read(20))
    if magic != b"GDPC" or ver != 3:
        raise NotImplementedError(f"pck format {magic} v{ver}: only GDPC "
                                  "v3 is handled — re-derive the layout")
    file_base, dir_off = struct.unpack("<QQ", f.read(16))
    f.seek(dir_off)
    nfiles, = struct.unpack("<I", f.read(4))
    for _ in range(nfiles):
        ln, = struct.unpack("<I", f.read(4))
        name = f.read(ln).rstrip(b"\0").decode()
        off, size = struct.unpack("<QQ", f.read(16))
        f.read(16)                                   # md5
        fl, = struct.unpack("<I", f.read(4))
        if name == "res://localization/eng/potions.json" \
                or name == "localization/eng/potions.json":
            if fl != 0:
                raise NotImplementedError(
                    f"potions.json pck flags {fl}: encrypted/compressed "
                    "entries not handled")
            f.seek(off + (file_base if flags & 2 else 0))
            return json.loads(f.read(size))
    raise FileNotFoundError("localization/eng/potions.json not in pck")


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

    typedef_name = {i + 1: str(r.TypeName)
                    for i, r in enumerate(md.TypeDef.rows)}
    typeref_name = {i + 1: str(r.TypeName)
                    for i, r in enumerate(md.TypeRef.rows)}

    def decode_typespec(idx):
        """GENERICINST TypeSpec -> 'Base<Arg,...>' (enough to read
        PowerVar`1<RadiancePower>; anything else returns ?typespec)."""
        blob = md.TypeSpec.rows[idx - 1].Signature.value_bytes()

        def comp(j):                       # ECMA-335 compressed uint
            b0 = blob[j]
            if b0 < 0x80:
                return b0, j + 1
            if b0 < 0xC0:
                return ((b0 & 0x3F) << 8) | blob[j + 1], j + 2
            return ((b0 & 0x1F) << 24) | (blob[j + 1] << 16) | \
                (blob[j + 2] << 8) | blob[j + 3], j + 4

        def tok_name(coded):
            tab, rid = coded & 3, coded >> 2
            return (typedef_name if tab == 0 else typeref_name).get(
                rid, f"?tok{coded}")

        if not blob or blob[0] != 0x15:    # GENERICINST
            return "?typespec"
        coded, i = comp(2)                 # skip GENERICINST + CLASS
        base = tok_name(coded)
        argc, i = comp(i)
        args = []
        for _ in range(argc):
            et, i = blob[i], i + 1
            if et in (0x11, 0x12):         # VALUETYPE / CLASS
                coded, i = comp(i)
                args.append(tok_name(coded))
            else:
                args.append(f"?et{et:02x}")
        return f"{base}<{','.join(args)}>"

    def ctor_owner(val):
        """newobj operand -> declaring type name (generics decoded)."""
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        if table == 0x06:                  # MethodDef
            oi = method_owner.get(rid)
            return type_ns[oi][1] if oi is not None else "?"
        if table == 0x0A:                  # MemberRef
            cls = md.MemberRef.rows[rid - 1].Class
            if cls.table and cls.table.name == "TypeRef":
                return typeref_name.get(cls.row_index, "?")
            if cls.table and cls.table.name == "TypeSpec":
                return decode_typespec(cls.row_index)
            if cls.table and cls.table.name == "TypeDef":
                return typedef_name.get(cls.row_index, "?")
        return "?"

    def field_ref_name(val):
        """ldsfld operand -> 'Parent::Field' (MemberRef only)."""
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        if table == 0x0A:
            r = md.MemberRef.rows[rid - 1]
            cls = r.Class
            parent = typeref_name.get(cls.row_index, "?") \
                if cls.table and cls.table.name == "TypeRef" else "?"
            return f"{parent}::{r.Name}"
        return "?"

    us = pe.net.user_strings

    def ldc_val(insn):
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

    def body_of(row):
        if not row.Rva:
            return None
        try:
            return CilMethodBody(
                RawReader(raw, pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return None

    def parse_canonical_vars(row, cls_name):
        """-> (vars {name: value}, props {name: [ints]}); raises on any
        instruction the grammar doesn't cover (I5: refuse, don't skim)."""
        insns = list(body_of(row).instructions)
        vars_out, props_out = {}, {}
        in_array = False
        ints, strs, dec_at = [], [], None   # payload accumulator
        skip_next_int = False               # element index after dup

        def finalize(owner):
            name = None
            if owner == "DynamicVar":
                if len(strs) != 1:
                    raise NotImplementedError(
                        f"{cls_name}: DynamicVar without a single ldstr "
                        "name")
                name = strs[0]
            elif owner.startswith("PowerVar`1<"):
                name = owner[len("PowerVar`1<"):-1]
            elif owner.endswith("Var"):
                name = owner[:-len("Var")]
            else:
                raise NotImplementedError(
                    f"{cls_name}: unrecognized var ctor {owner}")
            if not ints:
                raise NotImplementedError(
                    f"{cls_name}: var {name} with no value constant")
            vi = dec_at if dec_at is not None else 0
            if name in vars_out:
                raise NotImplementedError(
                    f"{cls_name}: duplicate var name {name}")
            vars_out[name] = ints[vi]
            extra = ints[:vi] + ints[vi + 1:]
            if extra:
                props_out[name] = extra

        for insn in insns:
            nm = insn.opcode.name
            v = ldc_val(insn)
            if v is not None:
                if skip_next_int:
                    skip_next_int = False   # array element index
                else:
                    ints.append(v)
                continue
            if nm == "newarr":
                # the int before newarr is the ARRAY SIZE (the Radiant
                # Tincture trap): drop it, enter element mode
                ints.pop()
                in_array = True
                continue
            if nm == "dup" and in_array:
                skip_next_int = True
                continue
            if nm == "ldstr":
                tok = getattr(insn.operand, "value", insn.operand)
                strs.append(us.get(tok & 0xFFFFFF).value)
                continue
            if nm == "ldsfld":
                fr = field_ref_name(insn.operand.value)
                if fr == "Decimal::One":
                    ints.append(1)
                    continue
                raise NotImplementedError(
                    f"{cls_name}: unhandled ldsfld {fr} in CanonicalVars")
            if nm == "newobj":
                owner = ctor_owner(insn.operand.value)
                if owner == "Decimal":
                    if not ints:
                        raise NotImplementedError(
                            f"{cls_name}: Decimal ctor with no int")
                    dec_at = len(ints) - 1  # the decimal-wrapped VALUE
                    continue
                if owner.startswith("<>z__ReadOnly"):
                    continue                # list wrapper
                finalize(owner)
                ints, strs, dec_at = [], [], None
                continue
            if nm in ("stelem.ref", "ret", "ldarg.0", "nop"):
                continue
            raise NotImplementedError(
                f"{cls_name}: unhandled opcode {nm} in get_CanonicalVars")
        return vars_out, props_out

    tooltips = read_pck_potion_tooltips()

    out = {}
    for ti, (ns, name) in type_ns.items():
        if ns != POTION_NS or name.startswith("<"):
            continue
        pid = snake_case(name).upper()
        info = {"class": name, "vars": {}}
        for mname, row in type_methods[ti]:
            if mname == "get_CanonicalVars":
                v, p = parse_canonical_vars(row, name)
                info["vars"] = v
                if p:
                    info["var_props"] = p
        tip = tooltips.get(f"{pid}.description")
        info["tooltip_eng"] = tip
        if tip is None:
            print(f"note: {pid} has no eng tooltip", file=sys.stderr)
        out["POTION." + pid] = info

    # tooltip cross-check at generation time too (the test re-runs it on
    # the checked-in JSON): every {Placeholder} must be a census var
    unresolved = {}
    for pid, info in out.items():
        for ph in re.findall(r"\{([A-Za-z][A-Za-z0-9]*)",
                             info["tooltip_eng"] or ""):
            if ph not in info["vars"] and ph not in TOOLTIP_PSEUDO_VARS:
                unresolved.setdefault(pid, []).append(ph)
    if unresolved:
        print(f"TOOLTIP VARS WITHOUT IL VALUES: {unresolved}",
              file=sys.stderr)

    print(json.dumps(out, indent=1, sort_keys=True))


# display-only helpers the tooltip engine injects; not CanonicalVars
TOOLTIP_PSEUDO_VARS = {"energyPrefix"}


if __name__ == "__main__":
    main()
