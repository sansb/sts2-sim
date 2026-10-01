#!/usr/bin/env python3
"""Card template translator — the #58 accelerant.

Auto-translates every card whose declarative surface and OnPlay IL fall
entirely inside a known-safe shape into the simulator's step vocabulary.
The exactness guarantee (SOLVER_INVARIANTS.md I5) is preserved by
construction: translation is a strict whitelist-only symbolic execution
of the IL — every opcode, every call target, and every branch must have
table-defined semantics, and any surprise (unknown call, data-dependent
branch, unexpected stack shape) REFUSES the card with a recorded reason
instead of guessing.

Per card it extracts:
  - ctor consts        -> cost / card type
  - get_CanonicalVars  -> named DynamicVars: base value (Decimal ctor)
                          and props (DamageVar/BlockVar ctor arg)
  - OnUpgrade          -> linear per-level var deltas (UpgradeValueBy)
  - get_MaxUpgradeLevel-> const override (default 1)
  - get_CanonicalKeywords -> decoded keyword names
  - get_CanonicalTags  -> decoded tag names (Strike etc.)
  - OnPlay <OnPlay>d__N::MoveNext -> ordered effect steps.  The async
    state machine is walked on its completed-synchronously path: the
    state field is the const -1 set in OnPlay, and every
    TaskAwaiter::get_IsCompleted test is resolved TRUE, which jumps
    straight to the matching GetResult join.  Anything that branches on
    real data refuses.

Step vocabulary emitted (amounts are ints or {"var": name} refs):
  ["attack", dmg, hits]      DamageCmd::Attack ... Targeting(target)
  ["attack_all", dmg, hits]  ... TargetingAllOpponents
  ["block", n]               CreatureCmd::GainBlock on own creature
  ["hp_loss", n]             CreatureCmd::Damage self, props==14
  ["energy", n]              PlayerCmd::GainEnergy on own player
  ["draw", n]                CardPileCmd::Draw
  ["heal", n]                CreatureCmd::Heal on own creature
  ["power", P, tgt, n]       PowerCmd::Apply<P>, tgt in enemy|self
  ["summon", "OSTY", n]      OstyCmd::Summon(player, n, source) — the SUM1
        ally primitive (v0.109 <Summon>d__0). Emitted for EVERY Necrobinder
        summon card; the sim gates which reach CARDS behind its own audited
        allowlist (_SUMMON_CARDS_MODELED), so unaudited summon cards refuse.

Tier-2 selection vocabulary (2026-07-11, IL reads in
ENCOUNTER_MECHANICS.md "Card selection commands"). A card may make at
most ONE in-play card selection; the null-check branch on the selected
card (FirstOrDefault) is the single fork the interpreter explores both
ways, and the two leaves must differ exactly by the sel_* consumer steps
(the game skips only the consumer when nothing was selectable):
  ["select", pile, min, max, filter]
        CardSelectCmd::FromHand / FromHandForDiscard (same semantics —
        the discard variant only sets a gold-glow cosmetic, RVA
        0x42d408) / FromHandForUpgrade (filter="upgradable", min=max=1)
        / FromCombatPile.  pile in draw|hand|discard|exhaust; min/max
        from the CardSelectorPrefs ctor ((prompt, n) -> n,n;
        (prompt, min, max)).  Auto-select rule (FromHand d__28 RVA
        0x42cf20, FromCombatPile d__20 RVA 0x42b844): if the filtered
        pile has <= min cards, ALL of them are taken with no player
        choice; otherwise the player picks min..max.
  ["sel_exhaust"]            CardCmd::Exhaust(ctx, sel, 0, 0)
  ["sel_discard"]            CardCmd::Discard(ctx, sel)
  ["sel_upgrade"]            CardCmd::Upgrade(sel, 1)
  ["sel_move", pile, pos]    CardPileCmd::Add(sel, pile, pos, ...)
                             (Headbutt: chosen discard card -> draw top)

Bodies that branch on CardModel::get_IsUpgraded run once per upgrade
level with the getter resolved to that level's constant, so a card can
parse at one level and refuse at another (True Grit: the unupgraded
path exhausts a RANDOM hand card via the CombatCardSelection stream ->
refused; the upgraded path is a player choice -> parsed).  Refused
levels are recorded in "refused_levels"; a card with no parsing level
refuses entirely.

Output JSON: {"cards": {CARD.ID: {..., "levels": {"0": steps, ...}}},
              "refused": {CARD.ID: reason},
              "targeting": {CARD.ID: exact target/constraint metadata}}.
The targeting map is universal: it covers translator-refused cards too.
Which powers / step kinds the simulator actually accepts is decided in
the engine (the Python combat_sim.py until #2827 deleted it; the Rust crate
now), not here — this file only proves the IL shape.

Regeneration:
  card_templates.py --raw --output ../card_templates_raw.json
  card_templates.py --compose --output ../card_templates.json

The raw step needs the pinned game DLL.  The compose step is intentionally
DLL-free and is what normal pytest freshness coverage exercises.
"""
import argparse
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
from template_targeting import (  # noqa: E402
    MULTIPLAYER_CONSTRAINT_NAMES, TARGET_TYPE_NAMES,
    template_target_refusal, validate_targeting_map)

SOLVER = Path(__file__).resolve().parent.parent
DLL = Path(os.environ.get(
    "STS2_DLL",
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                   "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                   "data_sts2_macos_arm64/sts2.dll"),
))
RAW_DEFAULT = SOLVER / "card_templates_raw.json"
LEDGER_DEFAULT = SOLVER / "card_refusal_ledger.json"

CARD_NS = "MegaCrit.Sts2.Core.Models.Cards"
TYPE_NAMES = {1: "attack", 2: "skill", 3: "power", 4: "status", 5: "curse",
              6: "quest"}


class Refuse(Exception):
    pass


class SelFork(Refuse):
    """Raised on a branch over the selected-card null check;
    Interp.run_forked explores both paths. Subclasses Refuse so any
    context that does not fork (plain run(), the relic translator's
    explorer) records it as an ordinary refusal instead of crashing."""

    def __init__(self, target, selected_on_taken):
        super().__init__("selection fork outside run_forked")
        self.target = target
        self.selected_on_taken = selected_on_taken


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


def read_compressed(data, i):
    b = data[i]
    if b < 0x80:
        return b, i + 1
    if (b & 0xC0) == 0x80:
        return ((b & 0x3F) << 8) | data[i + 1], i + 2
    return ((b & 0x1F) << 24) | (data[i + 1] << 16) | (data[i + 2] << 8) \
        | data[i + 3], i + 4


class Meta:
    """sts2.dll metadata with call-name resolution and signature arity."""

    def __init__(self):
        self.pe = dnfile.dnPE(str(DLL))
        self.raw = DLL.read_bytes()
        md = self.md = self.pe.net.mdtables

        self.n_methods = len(md.MethodDef.rows)
        self.method_owner = {}
        self.type_methods = {}       # ti -> {name: [rows]}
        self.type_name = {}
        for ti, t in enumerate(md.TypeDef.rows):
            self.type_name[ti] = (str(t.TypeNamespace), str(t.TypeName))
            byname = {}
            for m in (t.MethodList or []):
                self.method_owner[m.row_index] = ti
                byname.setdefault(str(m.row.Name), []).append(m.row)
            self.type_methods[ti] = byname

        self.enclosing = {}
        nc = getattr(md, "NestedClass", None)
        if nc:
            for row in nc.rows:
                self.enclosing[row.NestedClass.row_index - 1] = \
                    row.EnclosingClass.row_index - 1

        self.typedef_by_name = {}
        for ti, (ns, name) in self.type_name.items():
            self.typedef_by_name.setdefault(name, ti)

        self.field_rva = {}
        fr = getattr(md, "FieldRva", None)
        if fr:
            for row in fr.rows:
                self.field_rva[row.Field.row_index] = row.Rva

    # -- signature decoding (ECMA-335 II.23.2) ------------------------------

    def _sig_of(self, blob):
        data = bytes(blob.value_bytes())
        cc, i = data[0], 1
        if cc & 0x10:                                  # GENERIC
            _, i = read_compressed(data, i)
        nparams, i = read_compressed(data, i)
        # return type: skip custom mods, then check VOID
        j = i
        while data[j] in (0x1F, 0x20):                 # CMOD_REQD/OPT
            j += 1
            _, j = read_compressed(data, j)
        void = data[j] == 0x01
        hasthis = bool(cc & 0x20)
        return nparams, hasthis, void

    def sig_memberref(self, rid):
        return self._sig_of(self.md.MemberRef.rows[rid - 1].Signature)

    def sig_methoddef(self, rid):
        return self._sig_of(self.md.MethodDef.rows[rid - 1].Signature)

    # -- type / call name resolution ----------------------------------------

    def _typespec_base(self, rid):
        """Base type name of a TypeSpec (generic instantiations keep the
        `N suffix, drop the args): Task`1, TaskAwaiter`1, PowerVar`1."""
        data = bytes(self.md.TypeSpec.rows[rid - 1].Signature.value_bytes())

        def ty(i):
            et = data[i]
            i += 1
            if et in (0x11, 0x12):                    # VALUETYPE / CLASS
                coded, i = read_compressed(data, i)
                tbl, row = coded & 3, coded >> 2
                if tbl == 0:
                    return self.type_name[row - 1][1], i
                if tbl == 1:
                    return str(self.md.TypeRef.rows[row - 1].TypeName), i
                return "?", i
            if et == 0x15:                            # GENERICINST
                base, i = ty(i)
                n, i = read_compressed(data, i)
                for _ in range(n):
                    _, i = ty(i)
                return base, i
            if et == 0x1D:                            # SZARRAY
                base, i = ty(i)
                return base + "[]", i
            if et in (0x13, 0x1E):                    # VAR / MVAR
                _, i = read_compressed(data, i)
                return "T", i
            return f"et{et:#x}", i

        try:
            return ty(0)[0]
        except Exception:
            return "?"

    def memberref_name(self, rid):
        return self.memberref_info(rid)[0]

    def memberref_info(self, rid):
        """-> ('Class::Name', class_generic_args) — generic args come from
        a TypeSpec parent (PowerVar`1<VulnerablePower>::.ctor)."""
        row = self.md.MemberRef.rows[rid - 1]
        cls = row.Class
        parent, gargs = "?", []
        if cls.table:
            if cls.table.name == "TypeRef":
                parent = str(self.md.TypeRef.rows[cls.row_index - 1].TypeName)
            elif cls.table.name == "TypeSpec":
                parent, gargs = self._typespec_info(cls.row_index)
            elif cls.table.name == "TypeDef":
                parent = self.type_name[cls.row_index - 1][1]
        return f"{parent}::{row.Name}", gargs

    def _typespec_info(self, rid):
        """(base name, generic args) of a TypeSpec."""
        data = bytes(self.md.TypeSpec.rows[rid - 1].Signature.value_bytes())
        try:
            et = data[0]
            if et == 0x15:                            # GENERICINST
                base, i = self._ts_ty(data, 1)
                n, i = read_compressed(data, i)
                args = []
                for _ in range(n):
                    nm, i = self._ts_ty(data, i)
                    args.append(nm)
                return base, args
            return self._ts_ty(data, 0)[0], []
        except Exception:
            return self._typespec_base(rid), []

    def methoddef_name(self, rid):
        oi = self.method_owner.get(rid)
        owner = self.type_name[oi][1] if oi is not None else "?"
        return f"{owner}::{self.md.MethodDef.rows[rid - 1].Name}"

    def methodspec(self, rid):
        """-> (base_name, generic_args, arity_info)"""
        row = self.md.MethodSpec.rows[rid - 1]
        meth = row.Method
        data = bytes(row.Instantiation.value_bytes())
        args, i = [], 0
        if data and data[0] == 0x0A:
            n, i = read_compressed(data, 1)
            for _ in range(n):
                nm, i = self._ts_ty(data, i)
                args.append(nm)
        if meth.table.name == "MethodDef":
            return (self.methoddef_name(meth.row_index), args,
                    self.sig_methoddef(meth.row_index))
        return (self.memberref_name(meth.row_index), args,
                self.sig_memberref(meth.row_index))

    def _ts_ty(self, data, i):
        et = data[i]
        i += 1
        if et in (0x11, 0x12):
            coded, i = read_compressed(data, i)
            tbl, row = coded & 3, coded >> 2
            if tbl == 0:
                return self.type_name[row - 1][1], i
            if tbl == 1:
                return str(self.md.TypeRef.rows[row - 1].TypeName), i
            return "?", i
        if et == 0x15:
            base, i = self._ts_ty(data, i)
            n, i = read_compressed(data, i)
            inner = []
            for _ in range(n):
                nm, i = self._ts_ty(data, i)
                inner.append(nm)
            return f"{base}<{','.join(inner)}>", i
        if et == 0x1D:
            base, i = self._ts_ty(data, i)
            return base + "[]", i
        simple = {0x02: "bool", 0x08: "int32", 0x0E: "string",
                  0x0C: "float32", 0x0D: "float64", 0x1C: "object"}
        if et in simple:
            return simple[et], i
        if et in (0x13, 0x1E):
            n, i = read_compressed(data, i)
            return f"T{n}", i
        raise Refuse(f"typespec element {et:#x}")

    def body_of(self, row):
        if not row.Rva:
            return None
        try:
            return CilMethodBody(
                RawReader(self.raw, self.pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return None

    def user_string(self, rid):
        try:
            return str(self.pe.net.user_strings.get(rid).value)
        except Exception:
            return f"str{rid}"

    def nested_named(self, ti, prefix):
        """Nested TypeDef of ti whose name starts with prefix."""
        for n, e in self.enclosing.items():
            if e == ti and self.type_name[n][1].startswith(prefix):
                return n
        return None

    def enum_names(self, enum_type_name):
        """value -> field name for an enum TypeDef (via Constant table)."""
        ti = self.typedef_by_name.get(enum_type_name)
        if ti is None:
            return {}
        t = self.md.TypeDef.rows[ti]
        idx = {f.row_index: str(f.row.Name) for f in (t.FieldList or [])}
        out = {}
        for c in self.md.Constant.rows:
            p = c.Parent
            if p.table and p.table.name == "Field" and p.row_index in idx \
                    and idx[p.row_index] != "value__":
                out[int.from_bytes(c.Value.value_bytes()[:4], "little",
                                   signed=True)] = idx[p.row_index]
        return out


# ---------------------------------------------------------------------------
# Symbolic values (plain tuples):
#   ("int", n) ("dec", n) ("str", s) ("null",) ("this",) ("cardplay",)
#   ("ctx",) ("target",) ("owner",) ("player",) ("varset",) ("var", k)
#   ("varval", k) ("ref", ("loc"|"fld", key)) ("task", payload)
#   ("awaiter", payload) ("iscompleted",) ("cmd", dict) ("opaque", tag)
#   ("varlist", [entries])  (CanonicalVars accumulation)
# Tier-2 selection values:
#   ("prompt",)              a LocString selection prompt (cosmetic)
#   ("selprefs", mn, mx)     CardSelectorPrefs (amounts int or var refs)
#   ("pileobj", n)           PileTypeExtensions::GetPile result
#   ("selection",)           FromHand/FromCombatPile task payload
#   ("selcard",)             FirstOrDefault / FromHandForUpgrade result;
#                            branching on it forks the execution
# ---------------------------------------------------------------------------

LDC = {"ldc.i4.m1": -1, "ldc.i4.0": 0, "ldc.i4.1": 1, "ldc.i4.2": 2,
       "ldc.i4.3": 3, "ldc.i4.4": 4, "ldc.i4.5": 5, "ldc.i4.6": 6,
       "ldc.i4.7": 7, "ldc.i4.8": 8}

# Var classes with a compiled-in default name (ldstr in their ctor) are
# discovered at runtime by VarNames below; PowerVar`1 keys by its generic
# argument's type name (PowerVar`1::.ctor: typeof(T).Name).


class Interp:
    """Strict symbolic executor for one method body."""

    MAX_STEPS = 5000

    def __init__(self, meta: Meta, tr: "Translator", body, args):
        self.meta = meta
        self.tr = tr
        self.insns = list(body.instructions)
        self.by_off = {ins.offset: i for i, ins in enumerate(self.insns)}
        self.args = args               # ldarg.N -> symbolic value
        self.locals = {}
        self.fields = {}               # state-machine instance fields
        self.stack = []
        self.effects = []
        self.visited = set()

    # -- helpers -------------------------------------------------------------

    def pop(self, n=1):
        if len(self.stack) < n:
            raise Refuse("stack underflow")
        if n == 1:
            return self.stack.pop()
        vals = self.stack[-n:]
        del self.stack[-n:]
        return vals

    def deref(self, v):
        if v[0] == "ref":
            kind, key = v[1]
            store = self.locals if kind == "loc" else self.fields
            return store.get(key, ("opaque", "uninit"))
        return v

    def setref(self, v, val):
        kind, key = v[1]
        (self.locals if kind == "loc" else self.fields)[key] = val

    # -- main loop -------------------------------------------------------------

    def run(self):
        i = 0
        steps = 0
        while True:
            steps += 1
            if steps > self.MAX_STEPS:
                raise Refuse("instruction budget exceeded (loop?)")
            if i >= len(self.insns):
                raise Refuse("fell off method end")
            ins = self.insns[i]
            key = (ins.offset, tuple(sorted(self.locals.keys())))
            if ins.offset in self.visited:
                raise Refuse(f"revisited IL_{ins.offset:04x} (loop)")
            self.visited.add(ins.offset)
            jump = self.step(ins)
            if jump == "ret":
                return
            if jump is None:
                i += 1
            else:
                if jump not in self.by_off:
                    raise Refuse(f"branch to unknown offset {jump}")
                i = self.by_off[jump]

    def _snapshot(self):
        return (list(self.stack), dict(self.locals), dict(self.fields),
                [list(e) for e in self.effects], set(self.visited))

    def _restore(self, snap):
        stack, locs, flds, effs, vis = snap
        self.stack = list(stack)
        self.locals = dict(locs)
        self.fields = dict(flds)
        self.effects = [list(e) for e in effs]
        self.visited = set(vis)

    def run_forked(self):
        """run() + the ONE fork this interpreter knows: a branch over the
        selected-card null check. Returns [(selected|None, effects)] —
        one leaf when no selection branch exists, two otherwise."""
        leaves = []

        def explore(ip, selected):
            steps = 0
            while True:
                steps += 1
                if steps > self.MAX_STEPS:
                    raise Refuse("instruction budget exceeded (loop?)")
                if ip >= len(self.insns):
                    raise Refuse("fell off method end")
                ins = self.insns[ip]
                if ins.offset in self.visited:
                    raise Refuse(f"revisited IL_{ins.offset:04x} (loop)")
                self.visited.add(ins.offset)
                try:
                    jump = self.step(ins)
                except SelFork as f:
                    if selected is not None:
                        raise Refuse("nested selection forks")
                    snap = self._snapshot()
                    explore(self.by_off[f.target], f.selected_on_taken)
                    self._restore(snap)
                    explore(ip + 1, not f.selected_on_taken)
                    return
                if jump == "ret":
                    leaves.append((selected, [list(e)
                                              for e in self.effects]))
                    return
                if jump is None:
                    ip += 1
                else:
                    if jump not in self.by_off:
                        raise Refuse(f"branch to unknown offset {jump}")
                    ip = self.by_off[jump]

        explore(0, None)
        return leaves

    def step(self, ins):
        op = ins.opcode.name
        operand = ins.operand

        if op in LDC:
            self.stack.append(("int", LDC[op]))
        elif op in ("ldc.i4.s", "ldc.i4"):
            self.stack.append(("int", int(operand)))
        elif op in ("ldc.r4", "ldc.r8"):
            self.stack.append(("float", float(operand)))
        elif op == "ldstr":
            val = getattr(operand, "value", None)
            rid = val & 0xFFFFFF if val is not None else None
            self.stack.append(
                ("str", self.meta.user_string(rid) if rid else "?"))
        elif op == "ldnull":
            self.stack.append(("null",))
        elif op == "nop":
            pass
        elif op == "dup":
            self.stack.append(self.stack[-1])
        elif op == "pop":
            self.pop()
        elif op.startswith("stloc"):
            idx = int(op[-1]) if op[-1].isdigit() else int(str(operand)
                                                           .split("(")[-1]
                                                           .rstrip(")"), 0)
            self.locals[self.loc_idx(op, operand)] = self.pop()
        elif op.startswith("ldloca"):
            self.stack.append(("ref", ("loc", self.loc_idx(op, operand))))
        elif op.startswith("ldloc"):
            idx = self.loc_idx(op, operand)
            if idx not in self.locals:
                raise Refuse(f"read of unset local {idx}")
            self.stack.append(self.locals[idx])
        elif op.startswith("ldarg"):
            idx = self.arg_idx(op, operand)
            if idx not in self.args:
                raise Refuse(f"ldarg {idx} unmapped")
            self.stack.append(self.args[idx])
        elif op in ("ldfld", "ldflda", "stfld", "ldsfld", "stsfld"):
            return self.do_field(op, operand)
        elif op == "ldftn":
            # only reachable as the operand of a compiler-generated
            # delegate ctor (cached-lambda VFX pattern); the delegate is
            # only accepted by cosmetic With* consumers downstream
            self.stack.append(("opaque", "fnptr"))
        elif op in ("call", "callvirt", "newobj"):
            return self.do_call(op, operand)
        elif op == "castclass" or op == "box" or op == "unbox.any":
            pass                                        # value unchanged
        elif op == "initobj":
            self.pop()
        elif op == "newarr":
            self.stack.append(("varlist", []))
        elif op == "stelem.ref":
            val, _idx = self.pop(), self.pop()
            arr = self.pop()
            if arr[0] != "varlist":
                raise Refuse("stelem on non-array")
            arr[1].append(val)
            # array object is aliased via dup; mutation is in-place
        elif op in ("br", "br.s", "leave", "leave.s"):
            return int(operand)
        elif op in ("brtrue", "brtrue.s", "brfalse", "brfalse.s"):
            v = self.deref(self.pop())
            if v[0] == "selcard":
                # null check on the selected card: fork (taken on brtrue
                # means a card WAS selected; on brfalse it means none)
                raise SelFork(int(operand), op.startswith("brtrue"))
            if v == ("iscompleted",):
                truth = True
            elif v[0] in ("int",):
                truth = v[1] != 0
            elif v[0] == "null":
                truth = False
            else:
                raise Refuse(f"data-dependent branch on {v[0]} "
                             f"at IL_{ins.offset:04x}")
            taken = truth if op.startswith("brtrue") else not truth
            return int(operand) if taken else None
        elif op in ("beq", "beq.s", "bne.un", "bne.un.s"):
            b, a = self.pop(), self.pop()
            a, b = self.deref(a), self.deref(b)
            if a[0] != "int" or b[0] != "int":
                raise Refuse(f"data-dependent compare at "
                             f"IL_{ins.offset:04x}")
            eq = a[1] == b[1]
            taken = eq if op.startswith("beq") else not eq
            return int(operand) if taken else None
        elif op == "switch":
            v = self.deref(self.pop())
            if v[0] != "int":
                raise Refuse("data-dependent switch")
            targets = list(operand) if isinstance(operand, (list, tuple)) \
                else [operand]
            if 0 <= v[1] < len(targets):
                return int(targets[v[1]])
            return None                                # default: fall through
        elif op == "ret":
            return "ret"
        else:
            raise Refuse(f"opcode {op} at IL_{ins.offset:04x}")
        return None

    @staticmethod
    def loc_idx(op, operand):
        if op[-1].isdigit() and "." in op and not op.endswith(".s"):
            return int(op[-1])
        # dncil Local operand prints like local(0x0002); use its index
        s = str(operand)
        if "(" in s:
            return int(s.split("(")[-1].rstrip(")"), 0)
        return int(s)

    @staticmethod
    def arg_idx(op, operand):
        if op[-1].isdigit() and not op.endswith(".s"):
            return int(op[-1])
        s = str(operand)
        if "(" in s:
            return int(s.split("(")[-1].rstrip(")"), 0)
        return int(s)

    # -- fields ---------------------------------------------------------------

    def field_name(self, operand):
        val = getattr(operand, "value", None)
        if val is None:
            raise Refuse("field token unreadable")
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        if table == 0x04:
            return str(self.meta.md.Field.rows[rid - 1].Name)
        if table == 0x0A:
            return self.meta.memberref_name(rid)
        raise Refuse(f"field table {table:#x}")

    def do_field(self, op, operand):
        name = self.field_name(operand)
        short = name.split("::")[-1]
        if op == "stfld":
            val, obj = self.pop(), self.pop()
            self.fields[short] = val
        elif op == "ldfld":
            self.pop()
            if short == "<>1__state":
                self.stack.append(("int", -1))
            elif short in self.fields:
                self.stack.append(self.fields[short])
            elif short == "<>4__this":
                self.stack.append(("this",))
            elif short == "cardPlay":
                self.stack.append(("cardplay",))
            elif short == "choiceContext":
                self.stack.append(("ctx",))
            else:
                raise Refuse(f"ldfld {name}")
        elif op == "ldflda":
            self.pop()
            self.stack.append(("ref", ("fld", short)))
        elif op == "ldsfld":
            if name == "Decimal::One":
                self.stack.append(("dec", 1))
            elif name == "Decimal::Zero":
                self.stack.append(("dec", 0))
            elif name == "Decimal::MinusOne":
                self.stack.append(("dec", -1))
            elif short.startswith("<>9__"):
                # compiler-generated lambda cache: resolve to null so the
                # `dup; brtrue` fast path falls through to the fresh
                # ldftn/newobj construction (semantically identical)
                self.stack.append(("null",))
            elif short == "<>9":
                self.stack.append(("opaque", "lambda_host"))
            else:
                raise Refuse(f"ldsfld {name}")
        elif op == "stsfld":
            if not short.startswith("<>9__"):
                raise Refuse(f"stsfld {name}")
            self.pop()                       # lambda-cache write: benign
        return None

    # -- calls ------------------------------------------------------------------

    def resolve_call(self, operand):
        """-> (name, generic_args, nparams, hasthis, void)"""
        val = getattr(operand, "value", None)
        if val is None:
            raise Refuse("call token unreadable")
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        if table == 0x06:
            nparams, hasthis, void = self.meta.sig_methoddef(rid)
            return self.meta.methoddef_name(rid), [], nparams, hasthis, void
        if table == 0x0A:
            nparams, hasthis, void = self.meta.sig_memberref(rid)
            name, gargs = self.meta.memberref_info(rid)
            return name, gargs, nparams, hasthis, void
        if table == 0x2B:
            name, args, (nparams, hasthis, void) = self.meta.methodspec(rid)
            return name, args, nparams, hasthis, void
        raise Refuse(f"call table {table:#x}")

    def do_call(self, op, operand):
        name, gargs, nparams, hasthis, void = self.resolve_call(operand)
        if op == "newobj":
            ctor_args = self.pop(nparams) if nparams else []
            if not isinstance(ctor_args, list):
                ctor_args = [ctor_args]
            self.stack.append(self.tr.new_object(self, name, gargs,
                                                 ctor_args))
            return None
        n = nparams + (1 if hasthis else 0)
        argv = self.pop(n) if n else []
        if not isinstance(argv, list):
            argv = [argv]
        result = self.tr.call(self, name, gargs, argv, void)
        if result is not None:
            self.stack.append(result)
        return None


# ---------------------------------------------------------------------------
# Call semantics (the whitelist). Anything not matched refuses.
# ---------------------------------------------------------------------------

# calls that never touch combat state; result (if non-void) is opaque or
# the builder itself for With* chains
COSMETIC_CLASSES = {"VfxCmd", "SfxCmd"}
COSMETIC_CALLS = {
    "CreatureCmd::TriggerAnim",
    "Player::get_Character",
    "Ironclad::GetHeavyAttackDelay", "Ironclad::GetHeavyAnim",
    "Ironclad::GetHeavyAnimIfApplicable",
    "Ironclad::GetHeavyAttackDelayIfApplicable",
    "Necrobinder::GetSummonAnimIfApplicable",
    "Necrobinder::GetSummonDelayIfApplicable",
}
ATTACK_COSMETIC = {
    "AttackCommand::WithHitFx", "AttackCommand::WithHitSfx",
    "AttackCommand::WithHitVfxNode", "AttackCommand::WithHitVfxSpawnedAtBase",
    "AttackCommand::WithAttackerAnim", "AttackCommand::WithScreenShake",
    "AttackCommand::WithAttackerFx",
}
ASYNC_PLUMBING = {
    "AsyncTaskMethodBuilder::SetResult",
    "AsyncTaskMethodBuilder::SetException",
    "AsyncTaskMethodBuilder::SetStateMachine",
    "AsyncTaskMethodBuilder::Start",
    "AsyncTaskMethodBuilder::AwaitUnsafeOnCompleted",
}


class Translator:
    def __init__(self, meta: Meta):
        self.meta = meta
        self.varset_keys = self._varset_getter_keys()
        self.var_class_names = self._var_default_names()
        self.keyword_names = meta.enum_names("CardKeyword")
        self.tag_names = meta.enum_names("CardTag")
        self.level = 0            # upgrade level of the current OnPlay
        #                           pass (resolves get_IsUpgraded)

    # -- one-time metadata maps ---------------------------------------------

    def _varset_getter_keys(self):
        """DynamicVarSet::get_X -> its ldstr dictionary key."""
        ti = self.meta.typedef_by_name["DynamicVarSet"]
        out = {}
        for mname, rows in self.meta.type_methods[ti].items():
            if not mname.startswith("get_"):
                continue
            body = self.meta.body_of(rows[0])
            if body is None:
                continue
            for ins in body.instructions:
                if ins.opcode.name == "ldstr":
                    val = getattr(ins.operand, "value", None)
                    if val is not None:
                        out[mname] = self.meta.user_string(val & 0xFFFFFF)
                    break
        return out

    def _var_default_names(self):
        """XVar class -> default Name (the ldstr its ctor passes to
        DynamicVar::.ctor)."""
        out = {}
        for ti, (ns, name) in self.meta.type_name.items():
            if not name.endswith("Var") or "DynamicVars" not in ns:
                continue
            for row in self.meta.type_methods[ti].get(".ctor", []):
                body = self.meta.body_of(row)
                if body is None:
                    continue
                for ins in body.instructions:
                    if ins.opcode.name == "ldstr":
                        val = getattr(ins.operand, "value", None)
                        if val is not None:
                            out.setdefault(
                                name, self.meta.user_string(val & 0xFFFFFF))
                        break
        return out

    # -- newobj semantics ------------------------------------------------------

    def new_object(self, interp, name, gargs, args):
        cls = name.split("::")[0]
        if name == "Decimal::.ctor" or name == "System.Decimal::.ctor":
            if len(args) == 1 and args[0][0] == "int":
                return ("dec", args[0][1])
            raise Refuse(f"Decimal ctor args {args}")
        if len(args) == 2 and args[1] == ("opaque", "fnptr"):
            # delegate over a compiler-generated lambda (VFX callbacks);
            # only cosmetic With* consumers accept it downstream
            return ("opaque", "delegate")
        if cls == "LocString":
            return ("prompt",)
        if cls == "CardSelectorPrefs":
            return self._selprefs(args)
        if cls.startswith("<>z__ReadOnly"):
            # C# collection-expression wrappers: pass the payload through
            entries = []
            for a in args:
                if a[0] == "varlist":
                    entries.extend(a[1])
                else:
                    entries.append(a)
            return ("varlist", entries)
        if cls.endswith("Var") or cls.startswith("PowerVar"):
            return self._new_var(cls, gargs, args)
        if cls == "DynamicVarSet":
            # ctor over an array or a single var: return the collected list
            entries = []
            for a in args:
                if a[0] == "varlist":
                    entries.extend(a[1])
                elif a[0] == "varobj":
                    entries.append(a)
                else:
                    raise Refuse(f"DynamicVarSet ctor arg {a[0]}")
            return ("varlist", entries)
        raise Refuse(f"newobj {name}")

    def _new_var(self, cls, gargs, args):
        # shapes: (val) | (val, props:int) | (name:str, val[, props:int])
        # where val is a Decimal or a plain int32 (CardsVar/EnergyVar...)
        name = None
        props = None
        vals = list(args)
        if vals and vals[0][0] == "str":
            name = vals.pop(0)[1]
        if len(vals) == 2 and vals[1][0] == "int":
            props = vals.pop()[1]
        if len(vals) != 1 or vals[0][0] not in ("dec", "int"):
            raise Refuse(f"var ctor {cls} args {args}")
        if name is None:
            if cls.startswith("PowerVar"):
                if len(gargs) != 1:
                    raise Refuse("PowerVar without generic arg")
                name = gargs[0]
            else:
                name = self.var_class_names.get(cls)
                if name is None:
                    raise Refuse(f"no default name for {cls}")
        return ("varobj", {"class": cls, "key": name, "base": vals[0][1],
                           "props": props})

    @staticmethod
    def _selprefs(args):
        """CardSelectorPrefs ctors (RVA 0x2e1d58 / 0x2e1d7c):
        (prompt, n) -> Min=Max=n; (prompt, min, max)."""
        if not args or args[0] != ("prompt",):
            raise Refuse(f"selprefs ctor args {args}")
        amounts = args[1:]
        for a in amounts:
            if a[0] not in ("int", "var", "varval", "dec"):
                raise Refuse(f"selprefs amount {a[0]}")
        if len(amounts) == 1:
            return ("selprefs", amounts[0], amounts[0])
        if len(amounts) == 2:
            return ("selprefs", amounts[0], amounts[1])
        raise Refuse(f"selprefs arity {len(args)}")

    # -- call semantics ----------------------------------------------------------

    def call(self, interp: Interp, name, gargs, argv, void):
        cls, meth = name.split("::", 1) if "::" in name else ("", name)
        base = f"{cls.split('.')[-1]}::{meth}"
        deref = interp.deref

        # ---- universal plumbing
        if base == "Object::.ctor":
            return None
        if base == "ArgumentNullException::ThrowIfNull":
            return None
        if base.startswith("AsyncTaskMethodBuilder::"):
            suffix = base.split("::")[1]
            if f"AsyncTaskMethodBuilder::{suffix}" in ASYNC_PLUMBING:
                return None
            if suffix == "Create":
                return ("opaque", "builder")
            if suffix == "get_Task":
                return ("task", None)
            raise Refuse(f"builder call {base}")
        if meth == "Start" and name.startswith("AsyncTaskMethodBuilder"):
            return None
        if base.endswith("::GetAwaiter") and cls.split(".")[-1] in (
                "Task", "Task`1"):
            task = deref(argv[0])
            payload = task[1] if task[0] == "task" else None
            return ("awaiter", payload)
        if meth == "get_IsCompleted" and "TaskAwaiter" in cls:
            return ("iscompleted",)
        if meth == "GetResult" and "TaskAwaiter" in cls:
            aw = deref(argv[0])
            payload = aw[1] if aw[0] == "awaiter" else None
            if void:
                return None
            return payload if payload is not None else ("opaque", "result")
        if base == "Task::get_CompletedTask":
            return ("task", None)
        if base == "Decimal::op_Implicit":
            v = deref(argv[0])
            if v[0] == "int":
                return ("dec", v[1])
            if v[0] == "varval":
                return v
            raise Refuse(f"op_Implicit on {v[0]}")

        # ---- cosmetics
        bare_cls = cls.split(".")[-1]
        if bare_cls in COSMETIC_CLASSES or \
                (bare_cls.startswith("N") and "Vfx" in bare_cls):
            # Godot scene-node vfx classes (NFireBurningVfx etc.)
            return None if void else ("task", None)
        if base in COSMETIC_CALLS:
            if base == "CreatureCmd::TriggerAnim":
                return ("task", None)
            return None if void else ("opaque", "cosmetic")
        if base.startswith("CharacterModel::get_") and \
                meth.endswith("AnimDelay"):
            return ("opaque", "cosmetic")
        if base == "NCombatRoom::get_Instance":
            # Godot scene node behind a null guard; the null path is a
            # real game path (headless), so the guarded region can only
            # hold VFX — resolve to null and take the skip path.
            return ("null",)
        if base in ATTACK_COSMETIC:
            cmd = deref(argv[0])
            if cmd[0] != "cmd":
                raise Refuse(f"{base} on non-command")
            return cmd

        # ---- context getters
        if base == "CardModel::get_DynamicVars":
            return ("varset",)
        if base == "CardModel::get_Owner":
            return ("owner",)
        if base == "Player::get_Creature":
            if deref(argv[0]) != ("owner",):
                raise Refuse("get_Creature on non-owner")
            return ("player",)
        if base == "CardPlay::get_Target":
            return ("target",)
        if base == "CardModel::get_CombatState":
            # only consumed by TargetingAllOpponents; any other use refuses
            return ("combatstate",)
        if base == "DynamicVarSet::get_Item":
            key = deref(argv[1])
            if key[0] != "str":
                raise Refuse("var indexer with non-const key")
            return ("var", key[1])
        if base.startswith("DynamicVarSet::get_"):
            key = self.varset_keys.get(meth)
            if key is None:
                raise Refuse(f"unknown varset getter {meth}")
            return ("var", key)
        if base in ("DynamicVar::get_BaseValue", "DynamicVar::get_IntValue"):
            # IntValue is (int)BaseValue; declared bases are int-valued
            v = deref(argv[0])
            if v[0] != "var":
                raise Refuse("BaseValue of non-var")
            return ("varval", v[1])

        # ---- card selection (tier-2; RVAs in ENCOUNTER_MECHANICS.md)
        if cls.split(".")[-1] == "CardSelectorPrefs":
            if meth.startswith("get_") and meth.endswith("SelectionPrompt"):
                return ("prompt",)
            if meth == ".ctor":
                # struct ctor through `ldloca; call .ctor`
                if not argv or argv[0][0] != "ref":
                    raise Refuse("selprefs ctor without ref this")
                interp.setref(argv[0],
                              self._selprefs([deref(a) for a in argv[1:]]))
                return None
            raise Refuse(f"CardSelectorPrefs call {meth}")
        if meth == "get_SelectionScreenPrompt":
            return ("prompt",)
        if base == "PileTypeExtensions::GetPile":
            argv = [deref(a) for a in argv]
            if len(argv) != 2 or argv[0][0] != "int" \
                    or argv[1] not in (("owner",), ("player",)):
                raise Refuse(f"GetPile args {argv}")
            return ("pileobj", argv[0][1])
        if base in ("CardSelectCmd::FromHand",
                    "CardSelectCmd::FromHandForDiscard"):
            argv = [deref(a) for a in argv]
            # (ctx, player, prefs, filter, source)
            if len(argv) != 5 or argv[0] != ("ctx",) \
                    or argv[1] != ("owner",) or argv[2][0] != "selprefs" \
                    or argv[3] != ("null",) or argv[4] != ("this",):
                raise Refuse(f"{base.split('::')[1]} shape {argv}")
            interp.effects.append(["select", "hand",
                                   self.amount(argv[2][1]),
                                   self.amount(argv[2][2]), None])
            return ("task", ("selection",))
        if base == "CardSelectCmd::FromHandForUpgrade":
            argv = [deref(a) for a in argv]
            # (ctx, player, source); filters IsUpgradable, min=max=1,
            # auto-picks at <=1 candidate (d__30 RVA 0x42d510)
            if len(argv) != 3 or argv[0] != ("ctx",) \
                    or argv[1] != ("owner",) or argv[2] != ("this",):
                raise Refuse(f"FromHandForUpgrade shape {argv}")
            interp.effects.append(["select", "hand", 1, 1, "upgradable"])
            return ("task", ("selcard",))
        if base == "CardSelectCmd::FromCombatPile":
            argv = [deref(a) for a in argv]
            # (ctx, pile, player, prefs[, filter])
            if len(argv) not in (4, 5) or argv[0] != ("ctx",) \
                    or argv[1][0] != "pileobj" \
                    or argv[2] not in (("owner",), ("player",)) \
                    or argv[3][0] != "selprefs":
                raise Refuse(f"FromCombatPile shape {argv}")
            if len(argv) == 5 and argv[4] != ("null",):
                raise Refuse("FromCombatPile with filter")
            pname = {1: "draw", 2: "hand", 3: "discard",
                     4: "exhaust"}.get(argv[1][1])
            if pname is None:
                raise Refuse(f"FromCombatPile pile {argv[1][1]}")
            interp.effects.append(["select", pname,
                                   self.amount(argv[3][1]),
                                   self.amount(argv[3][2]), None])
            return ("task", ("selection",))
        if meth == "FirstOrDefault" and gargs == ["CardModel"]:
            if deref(argv[0])[0] != "selection":
                raise Refuse("FirstOrDefault on non-selection")
            return ("selcard",)
        if base == "CardCmd::Exhaust":
            argv = [deref(a) for a in argv]
            # (ctx, card, causedByEthereal, skipVisuals)
            if len(argv) != 4 or argv[0] != ("ctx",) \
                    or argv[1][0] != "selcard" \
                    or argv[2][0] != "int" or argv[3][0] != "int":
                raise Refuse(f"Exhaust shape {argv}")
            interp.effects.append(["sel_exhaust"])
            return ("task", None)
        if base == "CardCmd::Discard":
            argv = [deref(a) for a in argv]
            if len(argv) != 2 or argv[0] != ("ctx",):
                raise Refuse(f"Discard shape {argv}")
            if argv[1][0] == "selcard":
                interp.effects.append(["sel_discard"])
                return ("task", None)
            if argv[1][0] == "selection":
                # plural overload (d__3 RVA 0x4256d4) on the whole
                # selection: no null-check fork — an empty selection is
                # a natural no-op (DiscardAndDraw iterates it)
                interp.effects.append(["sel_discard_all"])
                return ("task", None)
            raise Refuse(f"Discard on {argv[1][0]}")
        if base == "CardCmd::Upgrade":
            argv = [deref(a) for a in argv]
            # single-card overload (RVA 0x2d91bf): (card, levels)
            if len(argv) != 2 or argv[0][0] != "selcard" \
                    or argv[1] != ("int", 1):
                raise Refuse(f"Upgrade shape {argv}")
            interp.effects.append(["sel_upgrade"])
            return None if void else ("task", None)
        if base == "CardPileCmd::Add":
            argv = [deref(a) for a in argv]
            if not argv or argv[0][0] != "selcard":
                raise Refuse(f"PileCmd::Add on {argv[0][0] if argv else '?'}")
            for extra in argv[3:]:
                if extra[0] not in ("int", "null"):
                    raise Refuse(f"PileCmd::Add extra arg {extra[0]}")
            if argv[1][0] != "int" or argv[2][0] != "int":
                raise Refuse("PileCmd::Add non-const pile/pos")
            pname = {1: "draw", 2: "hand", 3: "discard",
                     4: "exhaust"}.get(argv[1][1])
            pos = {1: "bottom", 2: "top"}.get(argv[2][1])
            if pname is None or pos is None:
                raise Refuse(f"PileCmd::Add pile {argv[1][1]} "
                             f"pos {argv[2][1]}")
            interp.effects.append(["sel_move", pname, pos])
            return None if void else ("task", None)
        if base == "CardModel::get_IsUpgraded":
            # CurrentUpgradeLevel > 0 (RVA 0x227d1e) — resolved to the
            # level of the current per-level pass
            return ("int", 1 if self.level > 0 else 0)

        # ---- effect commands
        if base == "DamageCmd::Attack":
            amount = deref(argv[0])
            return ("cmd", {"kind": "attack", "dmg": amount,
                            "hits": ("int", 1), "target": None})
        if base == "AttackCommand::FromCard":
            cmd = deref(argv[0])
            if cmd[0] != "cmd" or deref(argv[1]) != ("this",):
                raise Refuse("FromCard shape")
            return cmd
        if base == "AttackCommand::Targeting":
            cmd = deref(argv[0])
            if cmd[0] != "cmd" or deref(argv[1]) != ("target",):
                raise Refuse("Targeting shape")
            cmd[1]["target"] = "enemy"
            return cmd
        if base == "AttackCommand::TargetingAllOpponents":
            cmd = deref(argv[0])
            if cmd[0] != "cmd" or \
                    deref(argv[1]) not in (("combatstate",), ("ctx",)):
                raise Refuse("TargetingAllOpponents shape")
            cmd[1]["target"] = "all"
            return cmd
        if base == "AttackCommand::WithHitCount":
            cmd = deref(argv[0])
            if cmd[0] != "cmd":
                raise Refuse("WithHitCount shape")
            cmd[1]["hits"] = deref(argv[1])
            return cmd
        if base == "AttackCommand::Execute":
            cmd = deref(argv[0])
            if cmd[0] != "cmd" or deref(argv[1]) != ("ctx",):
                raise Refuse("Execute shape")
            d = cmd[1]
            if d["target"] not in ("enemy", "all"):
                raise Refuse("attack without target")
            kind = "attack" if d["target"] == "enemy" else "attack_all"
            interp.effects.append(
                [kind, self.amount(d["dmg"]), self.amount(d["hits"])])
            return ("task", ("opaque", "cmdresult"))
        if base == "CreatureCmd::GainBlock":
            argv = [deref(a) for a in argv]
            if argv[0] != ("player",):
                raise Refuse(f"GainBlock on {argv[0]}")
            if argv[1][0] not in ("var", "varval", "dec"):
                raise Refuse(f"GainBlock amount {argv[1][0]}")
            interp.effects.append(["block", self.amount(argv[1])])
            return ("task", ("opaque", "dec"))
        if base == "CreatureCmd::Damage":
            argv = [deref(a) for a in argv]
            # (ctx, creature, amount, props, sourceModel, cardPlay)
            if len(argv) != 6 or argv[0] != ("ctx",):
                raise Refuse(f"CreatureCmd::Damage arity {len(argv)}")
            if argv[1] != ("player",):
                raise Refuse(f"Damage on {argv[1]}")
            if argv[3] != ("int", 14):
                raise Refuse(f"Damage props {argv[3]}")
            interp.effects.append(["hp_loss", self.amount(argv[2])])
            return ("task", ("opaque", "dec"))
        if base == "CreatureCmd::Heal":
            argv = [deref(a) for a in argv]
            if argv[0] != ("player",):
                raise Refuse(f"Heal on {argv[0]}")
            amounts = [a for a in argv[1:] if a[0] in ("var", "varval",
                                                       "dec")]
            if len(amounts) != 1:
                raise Refuse(f"Heal amount shape {argv}")
            interp.effects.append(["heal", self.amount(amounts[0])])
            return ("task", ("opaque", "dec"))
        if base == "PlayerCmd::GainEnergy":
            argv = [deref(a) for a in argv]
            amounts = [a for a in argv if a[0] in ("var", "varval", "dec")]
            if len(amounts) != 1:
                raise Refuse(f"GainEnergy amount shape {argv}")
            interp.effects.append(["energy", self.amount(amounts[0])])
            return None if void else ("task", None)
        if base == "PlayerCmd::GainStars":
            argv = [deref(a) for a in argv]
            amounts = [a for a in argv if a[0] in ("var", "varval", "dec")]
            if len(amounts) != 1:
                raise Refuse(f"GainStars amount shape {argv}")
            # The second semantic argument is the owning Player. Cards use
            # CardModel.Owner; a future target-dependent source must not be
            # silently translated through this owner-only step.
            players = [a for a in argv if a in (("owner",), ("player",))]
            if not players:
                raise Refuse(f"GainStars player shape {argv}")
            interp.effects.append(["stars", self.amount(amounts[0])])
            return None if void else ("task", None)
        if base == "CardPileCmd::Draw":
            argv = [deref(a) for a in argv]
            # (ctx, amount, owner, flag)
            if len(argv) != 4 or argv[0] != ("ctx",) \
                    or argv[2] not in (("owner",), ("player",)) \
                    or argv[3][0] != "int":
                raise Refuse(f"Draw shape {argv}")
            if argv[1][0] not in ("var", "varval", "dec"):
                raise Refuse(f"Draw amount {argv[1]}")
            interp.effects.append(["draw", self.amount(argv[1])])
            return None if void else ("task", None)
        if base.startswith("PowerCmd::Apply"):
            argv = [deref(a) for a in argv]
            if len(gargs) != 1:
                raise Refuse("Apply without power type")
            # (ctx, targetCreature, amount, applierCreature, source, flag)
            if len(argv) != 6 or argv[0] != ("ctx",):
                raise Refuse(f"Apply arity {len(argv)}")
            tgt = argv[1]
            if tgt == ("target",):
                tk = "enemy"
            elif tgt == ("player",):
                tk = "self"
            else:
                raise Refuse(f"Apply target {tgt}")
            if argv[3] != ("player",):
                raise Refuse(f"Apply applier {argv[3]}")
            if argv[4] != ("this",):
                raise Refuse(f"Apply source {argv[4]}")
            interp.effects.append(
                ["power", gargs[0], tk, self.amount(argv[2])])
            return ("task", ("opaque", "power"))
        if base == "OstyCmd::Summon":
            argv = [deref(a) for a in argv]
            # (choiceContext, player, amount, source) — summon the Osty pet
            # for the owning player with `amount` max-HP/heal (SUM1
            # summon_ally). v0.109 <Summon>d__0::MoveNext 0x437308; the
            # player arg is the card owner's Player (get_Owner) or the
            # picked target's Player, both the player in solo. Emitting the
            # step is engine-gated: the sim only models the SUM2 summon
            # cards (NecroMastery); other summon cards stay refused there.
            if len(argv) != 4 or argv[0] != ("ctx",) \
                    or argv[1] not in (("owner",), ("player",)) \
                    or argv[2][0] not in ("var", "varval", "dec") \
                    or argv[3] != ("this",):
                raise Refuse(f"Summon shape {argv}")
            interp.effects.append(["summon", "OSTY", self.amount(argv[2])])
            return ("task", ("opaque", "summon"))

        raise Refuse(f"call {name}")

    @staticmethod
    def amount(v):
        if v[0] in ("int", "dec"):
            return v[1]
        if v[0] in ("var", "varval"):
            return {"var": v[1]}
        raise Refuse(f"unresolvable amount {v}")


# ---------------------------------------------------------------------------
# Per-card extraction
# ---------------------------------------------------------------------------

class CardTranslator:
    def __init__(self):
        self.meta = Meta()
        self.tr = Translator(self.meta)

    def method(self, ti, name):
        rows = self.meta.type_methods[ti].get(name, [])
        return rows[0] if rows else None

    def run_body(self, row, args):
        body = self.meta.body_of(row)
        if body is None:
            raise Refuse("no body")
        it = Interp(self.meta, self.tr, body, args)
        it.run()
        return it

    # every extraction below raises Refuse on any off-template shape

    def card_model_ctor(self, ti):
        """Return the exact five constants passed to CardModel::.ctor.

        Some constructors initialize fields before the base call (SpoilsMap
        is the current example), so indexing all constants from the start of
        the body is not sound. The CardModel arguments are the final five
        constants immediately preceding its unique call.
        """
        row = self.method(ti, ".ctor")
        if row is None:
            raise Refuse("no ctor")
        body = self.meta.body_of(row)
        calls = []
        for i, ins in enumerate(body.instructions):
            if ins.opcode.name in ("call", "callvirt") \
                    and self._tokname(ins.operand) == "CardModel::.ctor":
                calls.append(i)
        if len(calls) != 1:
            raise Refuse(f"CardModel ctor call shape ({len(calls)})")
        values = []
        for ins in body.instructions[:calls[0]]:
            nm = ins.opcode.name
            if nm in LDC:
                values.append(LDC[nm])
            elif nm in ("ldc.i4.s", "ldc.i4"):
                values.append(int(ins.operand))
        if len(values) < 5:
            raise Refuse(f"CardModel ctor args ({len(values)} constants)")
        return row, values[-5:]

    @staticmethod
    def _enum_value(value, names, label):
        name = names.get(value)
        if name is None:
            raise Refuse(f"unknown {label} value {value}")
        return {"name": name, "value": value}

    def targeting_metadata(self, ti):
        ctor, args = self.card_model_ctor(ti)
        constructor = self._enum_value(args[-2], TARGET_TYPE_NAMES,
                                       "TargetType")

        target_getter = self.method(ti, "get_TargetType")
        if target_getter is None:
            effective = {"kind": "static", **constructor,
                         "source": "constructor"}
        else:
            try:
                value = self.const_getter(ti, "get_TargetType", None)
            except Refuse:
                effective = {"getter_rva": hex(target_getter.Rva),
                             "kind": "dynamic", "source": "getter"}
            else:
                effective = {
                    "getter_rva": hex(target_getter.Rva), "kind": "static",
                    **self._enum_value(value, TARGET_TYPE_NAMES, "TargetType"),
                    "source": "getter",
                }

        constraint_getter = self.method(ti, "get_MultiplayerConstraint")
        if constraint_getter is None:
            constraint = {
                "kind": "static", "name": "None", "source": "inherited",
                "value": 0,
            }
        else:
            try:
                value = self.const_getter(
                    ti, "get_MultiplayerConstraint", None)
            except Refuse:
                constraint = {"getter_rva": hex(constraint_getter.Rva),
                              "kind": "dynamic", "source": "getter"}
            else:
                constraint = {
                    "getter_rva": hex(constraint_getter.Rva),
                    "kind": "static",
                    **self._enum_value(value, MULTIPLAYER_CONSTRAINT_NAMES,
                                       "CardMultiplayerConstraint"),
                    "source": "getter",
                }
        return {
            "constructor_rva": hex(ctor.Rva),
            "constructor_target_type": constructor,
            "effective_target_type": effective,
            "multiplayer_constraint": constraint,
        }

    def canonical_vars(self, ti):
        row = self.method(ti, "get_CanonicalVars")
        if row is None:
            return {}
        it = self.run_body(row, {0: ("this",)})
        # find the returned varlist: the interpreter ends at ret with the
        # value on the stack
        entries = []
        for v in it.stack:
            if v[0] == "varlist":
                entries.extend(e for e in v[1])
            elif v[0] == "varobj":
                entries.append(v)
        # varobj values may also be nested in varlist entries
        out = {}
        for e in entries:
            if e[0] != "varobj":
                raise Refuse(f"non-var in CanonicalVars: {e[0]}")
            d = e[1]
            if d["key"] in out:
                raise Refuse(f"duplicate var {d['key']}")
            out[d["key"]] = {"class": d["class"], "base": d["base"],
                             "props": d["props"]}
        return out

    def upgrade_deltas(self, ti):
        """OnUpgrade -> ({var_key_or_EnergyCost: per-level delta},
        kw_added, kw_removed). Refuses non-linear bodies."""
        row = self.method(ti, "OnUpgrade")
        if row is None:
            return {}, [], []
        body = self.meta.body_of(row)
        if body is None:
            return {}, [], []
        deltas, kw_add, kw_rem = {}, [], []
        pending_var, pending_val, pending_str = None, None, None
        for ins in body.instructions:
            nm = ins.opcode.name
            if nm in ("nop", "ret", "ldarg.0"):
                continue
            if nm in LDC:
                pending_val = LDC[nm]
                continue
            if nm in ("ldc.i4.s", "ldc.i4"):
                pending_val = int(ins.operand)
                continue
            if nm == "ldstr":
                val = getattr(ins.operand, "value", None)
                if val is None:
                    raise Refuse("OnUpgrade ldstr unreadable")
                pending_str = self.meta.user_string(val & 0xFFFFFF)
                continue
            if nm == "ldsfld":
                fname = self._tokname(ins.operand)
                if fname.endswith("::One"):
                    pending_val = 1
                elif fname.endswith("::MinusOne"):
                    pending_val = -1
                else:
                    raise Refuse(f"OnUpgrade ldsfld {fname}")
                continue
            if nm == "neg":
                if pending_val is None:
                    raise Refuse("neg without value")
                pending_val = -pending_val
                continue
            if nm in ("call", "callvirt", "newobj"):
                cname = self._tokname(ins.operand)
                short = cname.split("::")[-1]
                if short in ("get_DynamicVars", "get_EnergyCost"):
                    if short == "get_EnergyCost":
                        pending_var = "EnergyCost"
                    continue
                if cname == "DynamicVarSet::get_Item":
                    if pending_str is None:
                        raise Refuse("OnUpgrade indexer without key")
                    pending_var, pending_str = pending_str, None
                    continue
                if cname.startswith("DynamicVarSet::get_"):
                    key = self.tr.varset_keys.get(
                        "get_" + cname.split("get_")[-1])
                    if key is None:
                        raise Refuse(f"OnUpgrade unknown getter {cname}")
                    pending_var = key
                    continue
                if short == ".ctor" and "Decimal" in cname:
                    continue                     # value already captured
                if short in ("UpgradeValueBy", "UpgradeBy"):
                    # UpgradeValueBy(Decimal) on a var;
                    # CardEnergyCost::UpgradeBy(int) on EnergyCost
                    if pending_var is None or pending_val is None:
                        raise Refuse(f"{short} without var/value")
                    if short == "UpgradeBy" and pending_var != "EnergyCost":
                        raise Refuse("UpgradeBy on non-cost")
                    deltas[pending_var] = deltas.get(pending_var, 0) \
                        + pending_val
                    pending_var, pending_val = None, None
                    continue
                if short in ("AddKeyword", "RemoveKeyword"):
                    if pending_val is None:
                        raise Refuse(f"{short} without keyword const")
                    kw = self.tr.keyword_names.get(pending_val)
                    if kw is None:
                        raise Refuse(f"{short} unknown keyword "
                                     f"{pending_val}")
                    (kw_add if short == "AddKeyword" else kw_rem).append(kw)
                    pending_val = None
                    continue
                raise Refuse(f"OnUpgrade call {cname}")
            raise Refuse(f"OnUpgrade opcode {nm}")
        return deltas, kw_add, kw_rem

    def _tokname(self, operand):
        val = getattr(operand, "value", None)
        if val is None:
            raise Refuse("token unreadable")
        table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
        if table == 0x06:
            return self.meta.methoddef_name(rid)
        if table == 0x0A:
            return self.meta.memberref_name(rid)
        if table == 0x04:
            owner = None
            return f"Field::{self.meta.md.Field.rows[rid - 1].Name}"
        if table == 0x2B:
            return self.meta.methodspec(rid)[0]
        raise Refuse(f"token table {table:#x}")

    def const_getter(self, ti, name, default):
        row = self.method(ti, name)
        if row is None:
            return default
        body = self.meta.body_of(row)
        consts = []
        for ins in body.instructions:
            nm = ins.opcode.name
            if nm in LDC:
                consts.append(LDC[nm])
            elif nm in ("ldc.i4.s", "ldc.i4"):
                consts.append(int(ins.operand))
            elif nm in ("nop", "ret"):
                pass
            else:
                raise Refuse(f"{name} opcode {nm}")
        if len(consts) != 1:
            raise Refuse(f"{name} shape")
        return consts[0]

    def enum_list(self, ti, getter, names):
        """get_CanonicalKeywords / get_CanonicalTags -> decoded names.
        Same decoder as tools/template_census.py keyword_values (validated
        over all 593 cards): no-newarr bodies put only values on the
        stack; newarr bodies use either an RVA-initialized blob (ldtoken)
        or stelem stores whose value is the const right before."""
        row = self.method(ti, getter)
        if row is None:
            return []
        body = self.meta.body_of(row)
        if body is None:
            return []
        insns = list(body.instructions)

        def ldc_val(ins):
            nm = ins.opcode.name
            if nm in LDC:
                return LDC[nm]
            if nm in ("ldc.i4.s", "ldc.i4"):
                return int(ins.operand)
            return None

        if not any(i.opcode.name == "newarr" for i in insns):
            vals = [v for v in (ldc_val(i) for i in insns) if v is not None]
        else:
            vals = None
            n_expected = 0
            for i, ins in enumerate(insns):
                if ldc_val(ins) is not None and i + 1 < len(insns) \
                        and insns[i + 1].opcode.name == "newarr":
                    n_expected = ldc_val(ins)
                if ins.opcode.name == "ldtoken" and ins.operand is not None:
                    tok = ins.operand.value
                    if (tok >> 24) & 0xFF == 0x04:
                        rva = self.meta.field_rva.get(tok & 0xFFFFFF)
                        if rva and n_expected:
                            off = self.meta.pe.get_offset_from_rva(rva)
                            raw = self.meta.raw
                            vals = [int.from_bytes(
                                raw[off + 4 * k:off + 4 * k + 4], "little")
                                for k in range(n_expected)]
            if vals is None:
                vals = []
                for i, ins in enumerate(insns):
                    if ins.opcode.name.startswith("stelem") and i >= 1:
                        v = ldc_val(insns[i - 1])
                        if v is not None:
                            vals.append(v)
        out = set()
        for v in vals:
            if v not in names:
                raise Refuse(f"{getter} unknown enum value {v}")
            out.add(names[v])
        return sorted(out)

    SEL_CONSUMERS = ("sel_exhaust", "sel_discard", "sel_upgrade",
                     "sel_move")

    def onplay_steps(self, ti, level):
        row = self.method(ti, "OnPlay")
        if row is None:
            return []                            # base CardModel: no-op
        sm = self.meta.nested_named(ti, "<OnPlay>")
        if sm is None:
            raise Refuse("OnPlay without async state machine")
        mn = self.method(sm, "MoveNext")
        if mn is None:
            raise Refuse("no MoveNext")
        body = self.meta.body_of(mn)
        if body is None:
            raise Refuse("no body")
        self.tr.level = level
        it = Interp(self.meta, self.tr, body, {0: ("smthis",)})
        leaves = it.run_forked()
        return self._merge_leaves(leaves)

    def _merge_leaves(self, leaves):
        """One leaf: straight-line body, no selection allowed to linger.
        Two leaves (the selected-card null-check fork): the selected
        path must equal the empty path plus the sel_* consumer steps —
        the game skips exactly the consumer when nothing was selected."""
        if len(leaves) == 1:
            effects = leaves[0][1]
            if any(e[0] in self.SEL_CONSUMERS for e in effects):
                raise Refuse("selection without a null-check fork")
            selects = [i for i, e in enumerate(effects)
                       if e[0] == "select"]
            alls = [i for i, e in enumerate(effects)
                    if e[0] == "sel_discard_all"]
            if selects or alls:
                # whole-selection consumers need no fork (empty
                # selection is a no-op); require exactly one, after
                # its select
                if len(selects) != 1 or len(alls) != 1 \
                        or alls[0] < selects[0]:
                    raise Refuse("unconsumed selection")
            return effects
        by = {flag: eff for flag, eff in leaves}
        if len(leaves) != 2 or set(by) != {True, False}:
            raise Refuse(f"unmergeable fork leaves ({len(leaves)})")
        selected, skipped = by[True], by[False]
        stripped = [e for e in selected if e[0] not in self.SEL_CONSUMERS]
        if stripped != skipped:
            raise Refuse("fork paths differ beyond selection consumers")
        consumers = [e for e in selected if e[0] in self.SEL_CONSUMERS]
        if len(consumers) != 1:
            raise Refuse(f"{len(consumers)} selection consumers")
        selects = [i for i, e in enumerate(selected)
                   if e[0] == "select"]
        if len(selects) != 1:
            raise Refuse(f"{len(selects)} select steps")
        consumer_idx = next(i for i, e in enumerate(selected)
                            if e[0] in self.SEL_CONSUMERS)
        if consumer_idx < selects[0]:
            raise Refuse("consumer precedes its selection")
        return selected

    def translate(self, ti):
        _ctor, consts = self.card_model_ctor(ti)
        cost, ctype = consts[0], TYPE_NAMES.get(consts[1])
        if ctype is None:
            raise Refuse(f"card type {consts[1]}")
        vars_ = self.canonical_vars(ti)
        deltas, kw_add, kw_rem = self.upgrade_deltas(ti)
        max_up = self.const_getter(ti, "get_MaxUpgradeLevel", 1)
        keywords = self.enum_list(ti, "get_CanonicalKeywords",
                                  self.tr.keyword_names)
        tags = self.enum_list(ti, "get_CanonicalTags", self.tr.tag_names)

        for k in deltas:
            if k == "EnergyCost":
                continue
            if k not in vars_:
                raise Refuse(f"upgrade of undeclared var {k}")

        def resolve(step, level):
            out = [step[0]]
            for a in step[1:]:
                if isinstance(a, dict):
                    key = a["var"]
                    if key not in vars_:
                        raise Refuse(f"step reads undeclared var {key}")
                    v = vars_[key]["base"] + level * deltas.get(key, 0)
                    out.append(v)
                elif isinstance(a, (int, str)) or a is None:
                    out.append(a)
                else:
                    raise Refuse(f"unresolved step arg {a}")
            return out

        levels, cost_by_level, kw_by_level = {}, {}, {}
        refused_levels = {}
        for lv in range(max_up + 1):
            cost_by_level[str(lv)] = cost + lv * deltas.get("EnergyCost", 0)
            kws = set(keywords)
            if lv > 0:
                kws |= set(kw_add)
                kws -= set(kw_rem)
            kw_by_level[str(lv)] = sorted(kws)
            try:
                steps = self.onplay_steps(ti, lv)
                levels[str(lv)] = [resolve(s, lv) for s in steps]
            except Refuse as e:
                refused_levels[str(lv)] = str(e)
        if not levels:
            raise Refuse(refused_levels[str(0)])
        return {
            "class": self.meta.type_name[ti][1],
            "cost": cost, "type": ctype,
            "max_upgrade": max_up,
            "keywords_by_level": kw_by_level, "tags": tags,
            "vars": vars_,
            "upgrade": deltas,
            "cost_by_level": cost_by_level,
            "levels": levels,
            "refused_levels": refused_levels,
        }

    def run(self):
        cards, refused, max_up, targeting = {}, {}, {}, {}
        for ti, (ns, name) in self.meta.type_name.items():
            if ns != CARD_NS or name.startswith("<"):
                continue
            cid = "CARD." + snake_case(name).upper()
            # Universal target/eligibility evidence, including cards whose
            # OnPlay translator refuses. Failure is fatal: a partial map
            # would recreate the metadata blind spot fixed by #439.
            targeting[cid] = self.targeting_metadata(ti)
            try:
                # universal map (even for refused cards): the sim needs
                # the game's MaxUpgradeLevel to evaluate IsUpgradable
                # for upgrade-selection candidates
                max_up[cid] = self.const_getter(ti, "get_MaxUpgradeLevel",
                                                1)
            except Refuse:
                pass                    # non-const override: sim refuses
            try:
                cards[cid] = self.translate(ti)
            except Refuse as e:
                refused[cid] = str(e)
            except Exception as e:                  # pragma: no cover
                refused[cid] = f"internal: {type(e).__name__}: {e}"
        return cards, refused, max_up, targeting


def raw_templates() -> dict:
    ct = CardTranslator()
    cards, refused, max_up, targeting = ct.run()
    return {"cards": cards, "refused": refused, "max_upgrade": max_up,
            "targeting": targeting}


def _load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except FileNotFoundError as e:
        raise ValueError(f"missing required input: {path}") from e
    except json.JSONDecodeError as e:
        raise ValueError(f"invalid JSON in {path}: {e}") from e


def canonical_json(data: dict) -> str:
    """The checked artifact format; clean regeneration is byte-stable."""
    return json.dumps(data, indent=1, sort_keys=True) + "\n"


def compose_templates(raw: dict, ledger: dict, census: dict) -> dict:
    """Compose raw translation with reviewed, translator-invisible I5 work.

    The ledger is never inferred from a previous composed output. Raw
    refusals are retained save for a dedicated reviewed exclusion; explicit
    refusals are unioned in and override an otherwise translatable template.
    """
    expected_raw = {"cards", "refused", "max_upgrade", "targeting"}
    if set(raw) != expected_raw:
        raise ValueError(f"raw templates keys must be {sorted(expected_raw)}")
    expected_ledger = {"explicit_refusals", "template_admission_exclusions",
                       "template_exclusions", "translator_refusal_exclusions"}
    if set(ledger) != expected_ledger:
        raise ValueError(f"ledger keys must be {sorted(expected_ledger)}")
    cards, raw_refused = raw["cards"], raw["refused"]
    explicit = ledger["explicit_refusals"]
    admission_exclusions = ledger["template_admission_exclusions"]
    template_exclusions = ledger["template_exclusions"]
    exclusions = ledger["translator_refusal_exclusions"]
    validate_targeting_map(raw["targeting"], census)
    if not all(isinstance(x, dict) for x in (
            cards, raw_refused, explicit, admission_exclusions,
            template_exclusions, exclusions)):
        raise ValueError("template cards/refusals and ledger sections are maps")
    for label, entries in (("explicit_refusals", explicit),
                           ("template_admission_exclusions",
                            admission_exclusions),
                           ("template_exclusions", template_exclusions),
                           ("translator_refusal_exclusions", exclusions)):
        unknown = set(entries) - set(census)
        empty = {cid for cid, reason in entries.items()
                 if not isinstance(reason, str) or not reason.strip()}
        if unknown or empty:
            raise ValueError(f"{label} invalid: unknown={sorted(unknown)} "
                             f"empty={sorted(empty)}")
    overlap = set(explicit) & set(raw_refused)
    if overlap:
        raise ValueError("explicit refusals duplicate raw translator refusals: "
                         f"{sorted(overlap)}")
    bad_exclusions = set(exclusions) - set(raw_refused)
    if bad_exclusions:
        raise ValueError("translator refusal exclusions must name current raw "
                         f"refusals: {sorted(bad_exclusions)}")
    conflict = set(exclusions) & (set(explicit) | set(cards)
                                   | set(admission_exclusions)
                                   | set(template_exclusions))
    if conflict:
        raise ValueError("translator refusal exclusions conflict with a live "
                         f"card or explicit refusal: {sorted(conflict)}")
    invalid_template_exclusions = ((set(template_exclusions) - set(explicit))
                                   | (set(template_exclusions) - set(cards)))
    if invalid_template_exclusions:
        raise ValueError("template exclusions must be explicit refusals with "
                         f"a current raw template: {sorted(invalid_template_exclusions)}")
    invalid_admission_exclusions = set(admission_exclusions) - set(cards)
    if invalid_admission_exclusions:
        raise ValueError("template admission exclusions must name a current "
                         "raw template: "
                         f"{sorted(invalid_admission_exclusions)}")
    admission_conflict = set(admission_exclusions) & (
        set(explicit) | set(raw_refused) | set(template_exclusions))
    if admission_conflict:
        raise ValueError("template admission exclusions conflict with a "
                         "refusal or metadata exclusion: "
                         f"{sorted(admission_conflict)}")
    unsafe_targets = {
        cid: template_target_refusal(raw["targeting"][cid], spec["levels"])
        for cid, spec in cards.items()
    }
    unsafe_targets = {cid: reason for cid, reason in unsafe_targets.items()
                      if reason is not None}
    unreviewed_unsafe = (set(unsafe_targets) - set(explicit)
                         - set(admission_exclusions))
    if unreviewed_unsafe:
        raise ValueError("unsafe translated target requires a "
                         "reviewed refusal or hand-model admission exclusion: "
                         f"{sorted(unreviewed_unsafe)}")
    refused = {cid: reason for cid, reason in raw_refused.items()
               if cid not in exclusions}
    # I5 is monotonic: composition only adds reviewed refusals.  A raw
    # refusal may disappear only through its separately reviewed exclusion.
    refused.update(explicit)
    # Refused cards keep their independently verified constructor/template
    # metadata. The engine makes the admission decision from `refused`.
    # Metadata-only exclusions remove refused templates, while admission
    # exclusions quarantine unsafe raw bodies owned by exact hand models.
    composed_cards = {
        cid: spec for cid, spec in cards.items()
        if cid not in template_exclusions and cid not in admission_exclusions}
    return {"cards": composed_cards, "refused": refused,
            "max_upgrade": raw["max_upgrade"],
            "targeting": raw["targeting"]}


def _write(path: Path, data: dict) -> None:
    path.write_text(canonical_json(data))


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Generate raw card templates or compose the reviewed I5 ledger.")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--raw", action="store_true",
                       help="translate the installed pinned DLL (requires the DLL)")
    modes.add_argument("--compose", action="store_true",
                       help="compose a checked raw snapshot and ledger (DLL-free)")
    parser.add_argument("--raw-file", type=Path, default=RAW_DEFAULT,
                        help="raw snapshot input/output (default: %(default)s)")
    parser.add_argument("--ledger", type=Path, default=LEDGER_DEFAULT,
                        help="reviewed explicit-refusal source (default: %(default)s)")
    parser.add_argument("--output", type=Path,
                        help="output path; stdout when omitted")
    args = parser.parse_args(argv)
    if args.raw:
        data = raw_templates()
        print(f"translated {len(data['cards'])} / "
              f"{len(data['cards']) + len(data['refused'])}", file=sys.stderr)
    elif args.compose:
        data = compose_templates(_load_json(args.raw_file),
                                 _load_json(args.ledger),
                                 _load_json(SOLVER / "cards_census.json"))
    else:
        parser.error("choose --raw or --compose")
    if args.output:
        _write(args.output, data)
    else:
        sys.stdout.write(canonical_json(data))


if __name__ == "__main__":
    main()
