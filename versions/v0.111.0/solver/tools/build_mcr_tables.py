#!/usr/bin/env python3
"""Build the net-id lookup tables the .mcr replay decoder needs.

The game's PacketWriter serializes ModelIds (cards, relics, encounters, ...)
as compact integer "net ids" assigned by ModelIdSerializationCache.Init
(v0.109.1 RVA 0x2139d4). Reconstruction, step for step:

1. `ModelDb.All` — one instance per ModelId in `_contentById`, which
   `ModelDb.Init` (0x22ccf8) fills by walking `AbstractModelSubtypes.All`
   (a generated array of 1656 ldtoken'd TypeDefs) in order and doing
   `_contentById[GetId(type)] = Activator.CreateInstance(type)`, so a
   repeated ModelId keeps only the LAST type.
2. `ModelDb.GetId` (0x22cf29) = (category, entry) where category =
   `ModelId.SlugifyCategory(GetCategoryType(t).Name)` — the ancestor whose
   BaseType is AbstractModel, slugified, trailing "_MODEL" stripped — and
   entry = `StringHelper.Slugify(t.Name)`.
3. `ContentSorter<ModelId>.Sort(types, ModelDb.GetId, true)` (0x213678)
   sorts by mod-affects-gameplay, then `ModelId.CompareTo` (Ordinal on
   category, then entry), then mod id, then `Type.FullName` (Ordinal).
4. Category/entry net ids are handed out in that order, first appearance
   wins, both maps pre-seeded at net id 0 with `ModelId.none` = NONE/NONE.
5. Epoch net ids: `ContentSorter<string>.Sort(EpochModel.AllEpochs,
   EpochModel.GetId, true)`, id = the string literal each subclass's
   get_Id returns; first appearance wins (no NONE pre-seed).
6. SavedProperty net ids: `CachePropertiesForType` (0x213f88) per model
   type in the same sorted order — instance (public+non-public) properties
   carrying [SavedProperty], sorted by `CompareProperties` (order, then
   name Ordinal); first appearance wins.

`modelIdHash`, stored in every replay header, is an XxHash32 over the UTF-8
bytes appended in exactly that order:

    for each sorted model:      category, entry     (every item, dupes too)
    for each sorted model:      each NEW property name
    for each sorted epoch:      epoch id            (every item, dupes too)

and nothing else — in particular NOT the max-id counts. **This changed
between v0.108.0 and v0.109.0**: the v0.108-era tables checked in on
2026-07-09 matched a real v0.108 replay with the counts appended and the
property names absent (see MCR_FORMAT.md), so the recipe above is v0.109+.
The hash is what lets us verify the reconstruction byte-for-byte, and the
game version that wrote a replay is in its header, so tables and replay
must come from the same build.

Output: versions/v0.111.0/solver/mcr_tables.json with entries/categories/epochs/property names
(net id = index), wire bit sizes, enum tables, and the expected hash.

Usage: python3 build_mcr_tables.py [-o ../mcr_tables.json] [--dll PATH]
"""
import argparse
import hashlib
import json
import math
import struct
import sys
from pathlib import Path

import dnfile
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBase

DLL = Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                     "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                     "data_sts2_macos_arm64/sts2.dll")

# Enums the .mcr serializer writes with WriteEnum / ReadEnum (wire width =
# ceil(log2(maxValue) + 1)) plus enums useful for labeling decoded ints.
ENUMS = [
    "GameMode", "RunRngType", "PlayerRngType", "RelicRarity", "MapPointType",
    "RoomType", "CardCreationSource", "CardRarityOddsType", "PlayerChoiceType",
    "GameActionType", "CombatReplayEventType", "PlayerTurnPhase",
    "DynamicVarType",
    # NetFullCombatState (the checksum section): CardState.keywords goes
    # through WriteEnum, CombatPileState.pileType is a raw int32.
    "CardKeyword", "PileType",
]


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


def slugify(name: str) -> str:
    """StringHelper.Slugify: camel-case split, upper, no special chars.

    The game's CamelCaseRegex is ([A-Za-z0-9]|\\G(?!^))([A-Z]) -> $1_$2,
    which (because \\G chains through consecutive capitals) is equivalent to
    inserting '_' before every capital preceded by an alphanumeric char.
    """
    out = []
    for i, c in enumerate(name.strip()):
        if i > 0 and c.isupper() and name[i - 1].isalnum() \
                and name[i - 1].isascii():
            out.append("_")
        out.append(c)
    s = "".join(out).upper()
    s = "".join("_" if ch.isspace() else ch for ch in s)
    return "".join(ch for ch in s if ch in
                   "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_")


def xxhash32(datas, seed=0):
    """XxHash32 over a sequence of byte chunks (streaming, like the game's
    XxHash32.Append calls)."""
    P1, P2, P3, P4, P5 = (2654435761, 2246822519, 3266489917,
                          668265263, 374761393)
    M = 0xFFFFFFFF

    def rotl(x, r):
        return ((x << r) | (x >> (32 - r))) & M

    data = b"".join(datas)
    n = len(data)
    i = 0
    if n >= 16:
        v1 = (seed + P1 + P2) & M
        v2 = (seed + P2) & M
        v3 = seed & M
        v4 = (seed - P1) & M
        while i <= n - 16:
            for j, v in enumerate((v1, v2, v3, v4)):
                lane = struct.unpack_from("<I", data, i + 4 * j)[0]
                v = (v + lane * P2) & M
                v = rotl(v, 13)
                v = (v * P1) & M
                if j == 0:
                    v1 = v
                elif j == 1:
                    v2 = v
                elif j == 2:
                    v3 = v
                else:
                    v4 = v
            i += 16
        h = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & M
    else:
        h = (seed + P5) & M
    h = (h + n) & M
    while i <= n - 4:
        h = (h + struct.unpack_from("<I", data, i)[0] * P3) & M
        h = (rotl(h, 17) * P4) & M
        i += 4
    while i < n:
        h = (h + data[i] * P5) & M
        h = (rotl(h, 11) * P1) & M
        i += 1
    h ^= h >> 15
    h = (h * P2) & M
    h ^= h >> 13
    h = (h * P3) & M
    h ^= h >> 16
    return h


class Dll:
    def __init__(self, dll_path=DLL):
        dll_path = Path(dll_path)
        self.path = dll_path
        self.pe = dnfile.dnPE(str(dll_path))
        self.raw = dll_path.read_bytes()
        self.md = self.pe.net.mdtables
        self.typedefs = list(self.md.TypeDef)
        self.typedef_by_name = {}
        for idx, t in enumerate(self.typedefs):
            self.typedef_by_name.setdefault(str(t.TypeName), []).append(idx)
        # method owner map (row id -> typedef index)
        self.method_owner = {}
        for idx, t in enumerate(self.typedefs):
            for m in t.MethodList:
                self.method_owner[id(m.row)] = idx
        # nested type -> enclosing type (0-based typedef indices)
        self.enclosing = {r.NestedClass.row_index - 1:
                          r.EnclosingClass.row_index - 1
                          for r in self.md.NestedClass.rows}

    def full_name(self, rid):
        """Type.FullName: 'Namespace.Outer+Inner' (the sort tiebreaker)."""
        idx = rid - 1
        parts = [str(self.typedefs[idx].TypeName)]
        while idx in self.enclosing:
            idx = self.enclosing[idx]
            parts.append(str(self.typedefs[idx].TypeName))
        ns = str(self.typedefs[idx].TypeNamespace)
        name = "+".join(reversed(parts))
        return f"{ns}.{name}" if ns else name

    def body(self, method_row):
        off = self.pe.get_offset_from_rva(method_row.Rva)
        return CilMethodBody(RawReader(self.raw, off))

    def ldtoken_typedefs(self, type_name, method_name):
        """All TypeDef rids loaded via ldtoken in the named method."""
        rids = []
        for idx in self.typedef_by_name.get(type_name, []):
            t = self.typedefs[idx]
            for m in t.MethodList:
                if str(m.row.Name) != method_name or not m.row.Rva:
                    continue
                for insn in self.body(m.row).instructions:
                    if insn.opcode.name == "ldtoken":
                        val = getattr(insn.operand, "value", insn.operand)
                        if (val >> 24) & 0xFF == 0x02:  # TypeDef
                            rids.append(val & 0xFFFFFF)
        return rids

    def base_typedef_rid(self, rid):
        """TypeDef rid of the base class, or None if base is a TypeRef
        (e.g. System.Object) or absent."""
        row = self.typedefs[rid - 1]
        ext = row.Extends
        base = getattr(ext, "row", None)
        if base is None:
            return None
        # Extends is a coded index: TypeDef, TypeRef or TypeSpec
        if type(base).__name__ == "TypeDefRow":
            for i, t in enumerate(self.typedefs):
                if t is base:
                    return i + 1
            return None
        return None  # TypeRef/TypeSpec: outside this assembly

    def type_name(self, rid):
        t = self.typedefs[rid - 1]
        return str(t.TypeName), str(t.TypeNamespace)

    def first_ldstr(self, type_rid, method_name):
        t = self.typedefs[type_rid - 1]
        for m in t.MethodList:
            if str(m.row.Name) == method_name and m.row.Rva:
                for insn in self.body(m.row).instructions:
                    if insn.opcode.name == "ldstr":
                        val = getattr(insn.operand, "value", insn.operand)
                        return self.pe.net.user_strings.get(
                            val & 0xFFFFFF).value
        return None

    def enum_table(self, name):
        """(members_by_value, wire_bits) for an enum TypeDef."""
        for idx in self.typedef_by_name.get(name, []):
            t = self.typedefs[idx]
            field_rows = {id(f.row): str(f.row.Name) for f in t.FieldList
                          if str(f.row.Name) != "value__"}
            if not field_rows:
                continue
            values = {}
            for c in self.md.Constant.rows:
                p = c.Parent.row
                if p is not None and id(p) in field_rows:
                    blob = (c.Value.value_bytes()
                            if hasattr(c.Value, "value_bytes")
                            else c.Value.raw_data)
                    values[struct.unpack("<i", bytes(blob)[:4])[0]] = \
                        field_rows[id(p)]
            max_val = max(values)
            # MaxEnumValueCache.Get -> max VALUE; bits = ceil(log2(max)+1)
            bits = math.ceil(math.log2(max_val) + 1) if max_val > 0 else 1
            return values, bits
        raise KeyError(name)

    def saved_properties(self, type_rid):
        """[(order, name)] of the [SavedProperty] properties that
        `type.GetProperties(Instance | Public | NonPublic)` returns, sorted
        the way `ModelIdSerializationCache.CompareProperties` sorts them
        (order ascending, then name Ordinal).

        Reflection semantics that matter: static properties are excluded
        (BindingFlags.Instance), and a base class's *private* properties are
        not inherited, so they are only visible on their declaring type.
        """
        out = []
        rid = type_rid
        depth = 0
        seen_names = set()
        while rid is not None:
            t = self.typedefs[rid - 1]
            for prop_row, attrs in self._props_with_attrs(t):
                name = str(prop_row.Name)
                if name in seen_names:
                    continue  # hidden/overridden: the derived one wins
                if not self._reflection_visible(prop_row, inherited=depth > 0):
                    continue
                for order in attrs:
                    seen_names.add(name)
                    out.append((order, name))
            rid = self.base_typedef_rid(rid)
            depth += 1
        out.sort(key=lambda p: (p[0], p[1]))
        return out

    def _reflection_visible(self, prop_row, inherited):
        flags = self._prop_accessor_flags().get(id(prop_row), [])
        if not flags:
            return True
        if all(f.mdStatic for f in flags):
            return False
        if inherited and all(f.mdPrivate for f in flags):
            return False
        return True

    def _prop_accessor_flags(self):
        """PropertyRow -> [accessor MethodDef flags], via MethodSemantics."""
        if not hasattr(self, "_prop_acc_cache"):
            cache = {}
            for r in self.md.MethodSemantics.rows:
                assoc = r.Association.row
                if assoc is None or type(assoc).__name__ != "PropertyRow":
                    continue
                cache.setdefault(id(assoc), []).append(r.Method.row.Flags)
            self._prop_acc_cache = cache
        return self._prop_acc_cache

    def _props_with_attrs(self, typedef):
        if not hasattr(self, "_prop_attr_cache"):
            self._build_prop_attr_cache()
        return self._prop_attr_cache.get(id(typedef), [])

    def _build_prop_attr_cache(self):
        # Map PropertyRow -> [SavedPropertyAttribute order args]
        md = self.md
        prop_attr = {}
        for ca in md.CustomAttribute.rows:
            parent = ca.Parent.row
            if parent is None or type(parent).__name__ != "PropertyRow":
                continue
            ctor = ca.Type.row
            cls = getattr(ctor, "Class", None)
            cls_name = ""
            if cls is not None and cls.row is not None:
                cls_name = str(getattr(cls.row, "TypeName", ""))
            else:
                owner = self.method_owner.get(id(ctor))
                if owner is not None:
                    cls_name = str(self.typedefs[owner].TypeName)
            if cls_name != "SavedPropertyAttribute":
                continue
            blob = bytes(ca.Value.value_bytes()
                         if hasattr(ca.Value, "value_bytes")
                         else ca.Value.raw_data)
            # prolog 0x0001, then fixed args; ctor may take (int order) or ()
            order = 0
            if len(blob) >= 6:
                order = struct.unpack_from("<i", blob, 2)[0]
            prop_attr.setdefault(id(parent), []).append(order)
        # PropertyMap: TypeDef -> property list (dnfile materializes
        # PropertyList as a list of Property rows)
        self._prop_attr_cache = {}
        for r in md.PropertyMap.rows:
            td = r.Parent.row
            if td is None:
                continue
            plist = []
            for p in r.PropertyList:
                prow = getattr(p, "row", p)
                if id(prow) in prop_attr:
                    plist.append((prow, prop_attr[id(prow)]))
            if plist:
                self._prop_attr_cache[id(td)] = plist


def model_items(dll):
    """The sorted (category, entry, type_rid, full_name) list Init walks."""
    model_rids = dll.ldtoken_typedefs("AbstractModelSubtypes", ".cctor")
    if not model_rids:
        sys.exit("no model types found — AbstractModelSubtypes missing?")
    by_id = {}
    for rid in model_rids:
        name, _ = dll.type_name(rid)
        # category type: ancestor whose base is AbstractModel
        cat_rid = rid
        while True:
            base = dll.base_typedef_rid(cat_rid)
            if base is None:
                break
            base_name, _ = dll.type_name(base)
            if base_name == "AbstractModel":
                break
            cat_rid = base
        cat_name, _ = dll.type_name(cat_rid)
        category = slugify(cat_name)
        if category.endswith("_MODEL"):
            category = category[:-len("_MODEL")]
        entry = slugify(name)
        # ModelDb.Init: _contentById[id] = instance, walking the generated
        # array in order, so a repeated ModelId keeps only the last type.
        by_id[(category, entry)] = (category, entry, rid, dll.full_name(rid))
    collapsed = len(model_rids) - len(by_id)
    # ContentSorter: ModelId.CompareTo (Ordinal category, then entry), ties
    # broken by Type.FullName (Ordinal). No mods, so the mod keys are equal.
    return sorted(by_id.values(), key=lambda x: (x[0], x[1], x[3])), collapsed


def epoch_items(dll):
    """The sorted epoch ids Init walks (EpochModel.AllEpochs)."""
    # Init reads EpochModel.AllEpochs, the array EpochModel..cctor ldtokens.
    epoch_rids = dll.ldtoken_typedefs("EpochModel", ".cctor")
    if not epoch_rids:
        sys.exit("no epoch types found — EpochModel..cctor missing?")
    epochs = []
    for rid in epoch_rids:
        eid = dll.first_ldstr(rid, "get_Id")
        if eid is None:
            # I5: never guess an id we cannot read out of the IL.
            raise NotImplementedError(
                f"epoch type {dll.type_name(rid)[0]} has no literal get_Id — "
                "EpochModel.GetId is a dictionary built from instances, so a "
                "computed id needs a new extraction path here")
        epochs.append((eid, dll.full_name(rid)))
    # ContentSorter<string> sorts with string.CompareTo, i.e. the CURRENT
    # CULTURE, then Type.FullName (Ordinal). Ordinal reproduces it for the
    # present all-uppercase-ASCII id set; the modelIdHash pin is what would
    # catch a set where the two orders diverge.
    return sorted(epochs, key=lambda e: (e[0], e[1]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-o", "--output",
                    default=str(Path(__file__).resolve().parent.parent /
                                "mcr_tables.json"))
    ap.add_argument("--dll", default=str(DLL),
                    help="sts2.dll to read (default: the Steam install)")
    args = ap.parse_args()

    dll = Dll(args.dll)
    items, collapsed = model_items(dll)
    epochs = epoch_items(dll)

    # Everything below mirrors ModelIdSerializationCache.Init's own order,
    # because the XxHash32 is fed as a side effect of assigning net ids.
    chunks = []

    # --- model net ids ---
    # ..cctor pre-seeds both maps with ModelId.none (NONE/NONE) at net id 0
    # (the epoch and property maps are not pre-seeded).
    categories, entries = ["NONE"], ["NONE"]
    cat_map, entry_map = {"NONE": 0}, {"NONE": 0}
    for cat, ent, _rid, _fn in items:
        if cat not in cat_map:
            cat_map[cat] = len(categories)
            categories.append(cat)
        if ent not in entry_map:
            entry_map[ent] = len(entries)
            entries.append(ent)
        # appended for every item, including ids already seen
        chunks.append(cat.encode())
        chunks.append(ent.encode())

    # --- SavedProperty property-name net ids (same pass in the game) ---
    prop_names, prop_seen = [], set()
    for _cat, _ent, rid, _fn in items:
        for _order, pname in dll.saved_properties(rid):
            if pname not in prop_seen:
                prop_seen.add(pname)
                prop_names.append(pname)
                chunks.append(pname.encode())

    # --- epoch net ids ---
    epoch_ids = []
    epoch_seen = set()
    for eid, _fn in epochs:
        if eid not in epoch_seen:
            epoch_seen.add(eid)
            epoch_ids.append(eid)
        chunks.append(eid.encode())  # appended even for a repeated id

    # --- hash (must equal CombatReplay.modelIdHash) ---
    model_hash = xxhash32(chunks)

    def bit_size(count):
        # Mathf.CeilToInt(Math.Log2(count))
        return math.ceil(math.log2(count)) if count > 1 else 0

    # --- enums ---
    enums = {}
    for name in ENUMS:
        values, bits = dll.enum_table(name)
        enums[name] = {"bits": bits,
                       "values": {str(k): v for k, v in values.items()}}

    release = dll.path.parent.parent / "release_info.json"
    info = json.loads(release.read_text()) if release.exists() else {}
    # machine-independent provenance (absolute paths differ per checkout)
    digest = hashlib.sha256(dll.raw).hexdigest()

    out = {
        "game_version": info.get("version", "unknown"),
        "game_commit": info.get("commit", "unknown"),
        "game_version_hint": (f"built by tools/build_mcr_tables.py from "
                              f"{dll.path.name} sha256 {digest}"),
        "model_id_hash": model_hash,
        "categories": categories,
        "category_bits": bit_size(len(categories)),
        "entries": entries,
        "entry_bits": bit_size(len(entries)),
        "epochs": epoch_ids,
        "epoch_bits": bit_size(len(epoch_ids)),
        "property_names": prop_names,
        "property_bits": bit_size(len(prop_names)),
        "enums": enums,
    }
    Path(args.output).write_text(json.dumps(out, indent=1))
    print(f"game {out['game_version']} ({out['game_commit']})")
    print(f"models: {len(items)} ids -> {len(entries)} entries "
          f"({out['entry_bits']} bits), {len(categories)} categories "
          f"({out['category_bits']} bits)"
          + (f", {collapsed} duplicate ModelIds collapsed" if collapsed
             else ""))
    print(f"epochs: {len(epoch_ids)} ({out['epoch_bits']} bits)")
    print(f"saved properties: {len(prop_names)} ({out['property_bits']} bits)")
    print(f"modelIdHash = 0x{model_hash:08x}")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
