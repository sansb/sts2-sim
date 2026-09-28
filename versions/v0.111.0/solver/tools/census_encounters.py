#!/usr/bin/env python3
"""Encounter census for the solver derisk (#58) / coverage swarm (#138).

Enumerates EVERY class in the Encounters namespace (and .Mocks), keyed by
the run-file id form (ENCOUNTER.<UPPER_SNAKE of the class name> — the same
id the .run/.save files and the Rust encounter registry match on).
Each entry is classified:

    kind = combat | event | deprecated | mock
    pool = weak | normal | elite | boss | null   (from the class-name suffix)

For combat encounters we additionally extract, as before:
- monster composition (newobj Monsters.* in the encounter body)
- per monster: HP getters (ascension-aware constants), RandomBranchState
  count (random AI), Shuffle-stream card effects (CardPileCmd/CardFactory/
  CardCmd calls AND generic MethodSpec card insertions —
  AddToCombatAndPreview<T>, ICombatState.CreateCard<T>, ModelDb.Card<T>,
  ModelDb.Affliction<T>; the type arg names the card, #200), summon calls,
  move-machine size, powers applied, and card effects HOSTED INSIDE the
  powers the monster applies (#218 — e.g. PainfulStabsPower's Wound add,
  PersonalHivePower's Dazed add live in the power body, not the monster's)

Two scan gaps closed in #218/#225:
- powers_applied now also catches NON-generic PowerCmd::Apply sites (#225):
  a monster that builds a power via ModelDb.Power<T> and applies it with a
  plain PowerCmd::Apply (AEONGLASS's WitheringPresencePower) was invisible
  to the generic-only `Apply<T>` scan.
- power_card_effects (#218): for every power a monster applies, the power's
  own methods (and its nested async hook bodies) are walked for the same
  card-effect shapes; the card add is attributed to the monster's row,
  tagged with the hosting power. This is the recurring power-hosted blind
  spot (Entomancer PersonalHive, SpectralKnight Hex, TestSubject
  PainfulStabs).

NOTE on act/class metadata: the DLL does NOT encode an encounter's act on
the encounter class (the pool/act assignment lives in map-generation data,
and per ENCOUNTER_CENSUS.md act was *measured* from run-history floor
distributions, not read from the class). So `act` is intentionally absent
here — the pool suffix is the only tier signal the class name carries.

Output: JSON {ENCOUNTER.ID: {...}} on stdout, sorted. Regenerate the
checked-in census with:  python3 tools/census_encounters.py > encounters_census.json
(#138 extends this from the original 24 Elite/Boss-only rows.)
"""
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
    pe = dnfile.dnPE(str(DLL))
    raw = DLL.read_bytes()
    md = pe.net.mdtables
    n_types = len(md.TypeDef.rows)

    type_ns = {}
    type_methods = {}       # ti -> [(name, row)]
    method_owner = {}
    for ti, t in enumerate(md.TypeDef.rows):
        type_ns[ti] = (str(t.TypeNamespace), str(t.TypeName))
        ms = []
        for m in (t.MethodList or []):
            ms.append((str(m.row.Name), m.row))
            method_owner[m.row_index] = ti
        type_methods[ti] = ms

    # nested -> enclosing (async bodies live in nested types)
    enclosing = {}
    nc = getattr(md, "NestedClass", None)
    if nc:
        for row in nc.rows:
            enclosing[row.NestedClass.row_index - 1] = \
                row.EnclosingClass.row_index - 1

    def root_of(ti):
        while ti in enclosing:
            ti = enclosing[ti]
        return ti

    name_to_ti = {}
    for ti, (ns, name) in type_ns.items():
        name_to_ti[(ns, name)] = ti

    MON_NS = "MegaCrit.Sts2.Core.Models.Monsters"
    ENC_NS = "MegaCrit.Sts2.Core.Models.Encounters"

    def body_of(row):
        if not row.Rva:
            return None
        try:
            return CilMethodBody(RawReader(raw, pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return None

    def typedefref_name(coded):
        table, rid = coded & 0x03, coded >> 2
        if table == 0 and rid:
            return type_ns[rid - 1][1]
        if table == 1 and rid:
            return str(md.TypeRef.rows[rid - 1].TypeName)
        return "?"

    def decode_inst(blob):
        """MethodSpec instantiation blob -> list of type-arg names."""
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
                if et in (0x12, 0x11):
                    return typedefref_name(compressed())
                if et == 0x15:
                    base = ty()
                    n = compressed()
                    return f"{base}<{', '.join(ty() for _ in range(n))}>"
                if et == 0x1D:
                    return ty() + "[]"
                return f"et{et:#x}"

            if u8() != 0x0A:
                return []
            n = compressed()
            return [ty() for _ in range(n)]
        except Exception:
            return []

    def calls_in(row):
        b = body_of(row)
        if b is None:
            return []
        out = []
        for insn in b.instructions:
            if insn.opcode.name in ("call", "callvirt", "newobj", "ldftn") \
                    and insn.operand is not None:
                val = getattr(insn.operand, "value", None)
                if val is None:
                    continue
                table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
                if table == 0x06:
                    ti = method_owner.get(rid)
                    if ti is not None:
                        out.append((type_ns[ti][0], type_ns[ti][1],
                                    str(md.MethodDef.rows[rid - 1].Name),
                                    insn.opcode.name))
                elif table == 0x0A:
                    r = md.MemberRef.rows[rid - 1]
                    cls = r.Class.row
                    out.append(("", str(getattr(cls, "TypeName", "?")),
                                str(r.Name), insn.opcode.name))
                elif table == 0x2B:
                    r = md.MethodSpec.rows[rid - 1]
                    mrow = r.Method.row
                    nm = str(getattr(mrow, "Name", "?"))
                    # declaring class of the generic method (MethodDefOrRef:
                    # MemberRef rows carry .Class; MethodDef rows resolve
                    # via method_owner). Needed by card_effects (#200) —
                    # without it AddToCombatAndPreview<T> is invisible.
                    cls = getattr(getattr(mrow, "Class", None), "row", None)
                    if cls is not None:
                        owner = str(getattr(cls, "TypeName", "?"))
                    else:
                        ti2 = method_owner.get(
                            getattr(r.Method, "row_index", None))
                        owner = type_ns[ti2][1] if ti2 is not None else "?"
                    for arg in decode_inst(r.Instantiation):
                        out.append(("spec", arg, nm, insn.opcode.name, owner))
        return out

    def const_ints(row):
        b = body_of(row)
        if b is None:
            return []
        out = []
        for insn in b.instructions:
            nm = insn.opcode.name
            if nm.startswith("ldc.i4"):
                if insn.operand is not None:
                    out.append(int(insn.operand))
                elif nm == "ldc.i4.m1":
                    out.append(-1)
                elif nm[-1].isdigit():
                    out.append(int(nm[-1]))
        return out

    # ---- collect per-type call info incl. nested (async move bodies) ----
    def all_calls_of_type(ti):
        calls = []
        stack = [ti] + [n for n, e in enclosing.items() if e == ti]
        # include transitively nested
        added = True
        seen = set(stack)
        while added:
            added = False
            for n, e in enclosing.items():
                if e in seen and n not in seen:
                    seen.add(n)
                    stack.append(n)
                    added = True
        for t2 in stack:
            for name, row in type_methods[t2]:
                calls.extend(calls_in(row))
        return calls

    # Card effects come in two shapes (#200): plain calls on the card
    # command classes, and GENERIC (MethodSpec) calls whose type arg IS
    # the card — CardPileCmd.AddToCombatAndPreview<Slimed>(...) is how
    # slimes/Mytes/Chompers/MechaKnight insert status cards, and
    # ICombatState.CreateCard<T> / ModelDb.Card<T> name the card behind
    # otherwise-opaque adds. The pre-#200 scan only saw the first shape.
    CARD_CLASSES = ("CardPileCmd", "CardFactory", "CardCmd")

    def card_effects_of(calls):
        return sorted(
            {f"{c[1]}::{c[2]}" for c in calls
             if c[0] != "spec" and c[1] in CARD_CLASSES
             and c[2] not in ("get_Card",)} |
            {f"{c[4]}::{c[2]}<{c[1]}>" for c in calls
             if c[0] == "spec" and (
                 c[4] in CARD_CLASSES
                 or (c[4] == "ICombatState" and c[2] == "CreateCard")
                 or (c[4] == "ModelDb" and c[2] in ("Card", "Affliction")))})

    POWER_NS = "MegaCrit.Sts2.Core.Models.Powers"

    def power_card_effects_of(power_names):
        # #218: monsters host card adds inside the POWERS they apply, not
        # only in their own move bodies — PainfulStabsPower's Wound add,
        # PersonalHivePower's Dazed add (nested async <Hook>d__N::MoveNext,
        # walked transitively by all_calls_of_type), HexPower's Hexed
        # affliction. Walk each applied power's TypeDef with the same
        # card-effect extraction and attribute the result to the monster,
        # tagged with the hosting power for provenance (these fire on the
        # power's hook, so they are semantically distinct from a move's
        # own unconditional add — kept in a separate field, not merged
        # into card_effects).
        out = []
        for p in sorted(set(power_names)):
            pti = name_to_ti.get((POWER_NS, p))
            if pti is None:
                continue
            for e in card_effects_of(all_calls_of_type(pti)):
                out.append(f"{p}: {e}")
        return sorted(out)

    def analyze_monster(ti):
        ns, name = type_ns[ti]
        info = {"class": name}
        for mname, row in type_methods[ti]:
            if mname == "get_MinInitialHp":
                info["hp_min_consts"] = const_ints(row)
            elif mname == "get_MaxInitialHp":
                c = const_ints(row)
                if c:
                    info["hp_max_consts"] = c
        calls = all_calls_of_type(ti)
        info["random_branches"] = sum(
            1 for c in calls if c[1] == "RandomBranchState" and
            c[3] == "newobj")
        info["card_effects"] = card_effects_of(calls)
        info["summons"] = sorted({
            f"{c[1]}::{c[2]}" for c in calls
            if "Spawn" in c[2] or c[2] == "AddCreatureToCombat"
            or "Summon" in c[2]})
        # powers_applied: generic PowerCmd.Apply<T> (type arg = power) as
        # before, PLUS non-generic PowerCmd::Apply sites (#225). A monster
        # that builds a mutable power with ModelDb.Power<T> and applies it
        # via a plain (non-generic) PowerCmd::Apply — AEONGLASS's
        # WitheringPresencePower — is invisible to the generic-only scan.
        # ModelDb.Power<T>/Affliction<T> is only emitted in this
        # construct-then-apply shape, so when a non-generic Apply is present
        # its type arg names the applied power.
        has_nongeneric_apply = any(
            c[0] != "spec" and c[2] == "Apply" for c in calls)
        powers = {c[1] for c in calls if c[0] == "spec" and c[2] == "Apply"}
        if has_nongeneric_apply:
            powers |= {c[1] for c in calls if c[0] == "spec"
                       and c[4] == "ModelDb"
                       and c[2] in ("Power", "Affliction")}
        info["powers_applied"] = sorted(powers)
        # Emitted only when non-empty (like hp_max_consts): a bare [] on
        # every monster would swamp the census diff, the I1 cross-check
        # signal. Presence of the key == this monster hosts card effects
        # inside a power it applies.
        pce = power_card_effects_of(powers)
        if pce:
            info["power_card_effects"] = pce
        info["move_states"] = sum(
            1 for c in calls if c[1] == "MoveState" and c[3] == "newobj")
        return info

    # Every class in Encounters + Encounters.Mocks, classified and keyed
    # by the run-file id form. Monster composition is extracted for combat
    # encounters only (event/deprecated/mock encounters don't feed the
    # solver's make_monsters and their bodies are structurally different).
    POOL_SUFFIXES = (("Elite", "elite"), ("Boss", "boss"),
                     ("Normal", "normal"), ("Weak", "weak"))

    def classify(ns, name):
        if ns.endswith(".Mocks") or name.startswith("Mock"):
            return "mock"
        if name == "DeprecatedEncounter" or name.startswith("Deprecated"):
            return "deprecated"
        if "Event" in name:                      # *EventEncounter + V1/V2/V3
            return "event"
        return "combat"

    def pool_of(name):
        for suf, val in POOL_SUFFIXES:
            if name.endswith(suf):
                return val
        return None

    result = {}
    monster_cache = {}
    for ti, (ns, name) in sorted(type_ns.items(), key=lambda kv: kv[1][1]):
        if not ns.startswith(ENC_NS):
            continue
        kind = classify(ns, name)
        enc_id = "ENCOUNTER." + snake_case(name).upper()
        entry = {"class": name, "kind": kind, "pool": pool_of(name)}
        if kind == "combat":
            calls = all_calls_of_type(ti)
            monsters = []
            for c in calls:
                if (MON_NS, c[1]) in name_to_ti and (
                        (c[3] == "newobj" and c[2] == ".ctor")
                        or (c[0] == "spec"
                            and c[2] in ("Monster", "AddMonster"))):
                    if c[1] not in monsters:
                        monsters.append(c[1])
            for m in monsters:
                if m not in monster_cache:
                    monster_cache[m] = analyze_monster(name_to_ti[(MON_NS, m)])
            entry["monsters"] = monsters
        result[enc_id] = entry

    print(json.dumps({"encounters": result, "monsters": monster_cache},
                     indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
