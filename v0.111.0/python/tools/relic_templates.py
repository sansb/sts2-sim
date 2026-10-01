#!/usr/bin/env python3
"""Relic hook template translator (#58 — the relic-dimension accelerant).

Reuses the card translator's metadata layer and strict symbolic
interpreter (tools/card_templates.py) and adds ONE capability: branches
whose condition is a *recognized predicate* fork the execution and tag
each leaf's effects with the guard set, instead of refusing. Everything
else keeps the whitelist discipline — unknown calls, unrecognized
branches, stateful relic fields (counters), and loops all refuse the
relic with a recorded reason (SOLVER_INVARIANTS.md I5).

Recognized predicates (all evaluable by the simulator at hook time):
  ("own_side",)          participants.Contains(Owner.Creature)
  ("turn", op, n)        Owner.PlayerCombatState.TurnNumber  op n
  ("is_owner",)          hook arg == Owner (sync value hooks)
  ("in_progress",)       CombatManager.IsInProgress-style gate

Hook bodies handled:
  - async gameplay hooks (<Hook>d__N::MoveNext) -> guarded effect steps
  - ModifyMaxEnergy / ModifyHandDraw sync modifiers -> guarded
    ("max_energy", n) / ("draw_bonus", n) when the body is exactly
    "guards -> return base + var" (Sozu 0x247dbd shape)

Effect steps: ["energy", n], ["block", n]  (relic block: NOT dexterity-
modified — the sim adds it raw), ["draw", n], ["heal", n],
["power", P, "self", n], ["power_all", P, n]  (single command on the
HittableEnemies collection), ["damage_all", n, props].

Output: relic_templates.json {"relics": {...}, "refused": {...}}.
Which hooks/steps the engine accepts is decided in the engine (the Python
combat_sim.py until #2827 deleted it; the Rust crate now).

Usage: relic_templates.py > ../relic_templates.json
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(Path(__file__).parent.parent))
from card_templates import (  # noqa: E402
    LDC, CardTranslator, Interp, Meta, Refuse, Translator)
from sts2_rng import snake_case  # noqa: E402

RELIC_NS = "MegaCrit.Sts2.Core.Models.Relics"

# gameplay hooks we attempt; every other overridden gameplay method
# refuses the relic (declarative/cosmetic ones are ignored)
ASYNC_HOOKS = {
    "BeforeCombatStart", "AfterRoomEntered",
    "AfterSideTurnStart", "BeforeSideTurnStart",
    "AfterPlayerTurnStart", "AfterPlayerTurnStartLate",
    "AfterEnergyReset", "BeforeSideTurnEnd", "AfterSideTurnEnd",
    "AfterBlockCleared", "BeforeHandDraw",
}
# sync (no state machine) hook argument layouts, by hook name; arg0 is
# always `this`
HOOK_ARGS = {
    "BeforeCombatStart": [],
    "AfterRoomEntered": [("room",)],
    "AfterSideTurnStart": [("participants",)],
    "BeforeSideTurnStart": [("participants",)],
    "AfterSideTurnEnd": [("participants",)],
    "BeforeSideTurnEnd": [("participants",)],
    "AfterPlayerTurnStart": [("harg", "player")],
    "AfterPlayerTurnStartLate": [("harg", "player")],
    "AfterEnergyReset": [("harg", "player")],
    "AfterBlockCleared": [("harg", "creature")],
    "BeforeHandDraw": [("harg", "player")],
}
SYNC_VALUE_HOOKS = {"ModifyMaxEnergy": "max_energy",
                    "ModifyHandDraw": "draw_bonus"}
# declarative / cosmetic relic methods (never gameplay). Property
# accessors (get_*/set_*) are skipped as DEFINITIONS — gameplay only
# happens in hook bodies, and a hook body touching real relic state
# still refuses through the unknown-field / unknown-call paths.
IGNORED_METHODS = {
    ".ctor", "get_CanonicalVars", "get_ExtraHoverTips", "get_Tier",
    "get_CanonicalTags", "get_MultiplayerConstraint", "get_AssetPath",
    "get_CanBeObtainedInCombat", "get_ShopPrice", "get_FlavorText",
    "OnObtained", "get_CanObtain",
    "AfterObtained",          # fires at obtain time, outside combat
    "UpdateDisplay", "DoActivateVisuals",   # relic UI (set_Status etc.)
}

# Exact per-relic methods whose current-build bodies were reviewed as
# out-of-combat-only.  Do not move these into IGNORED_METHODS: a same-named
# method on any other relic must remain fail-closed until independently
# audited.  Every entry is also classified inert by census_relics.py.
REVIEWED_NONCOMBAT_METHODS = {
    "RELIC.ECTOPLASM": frozenset({
        "ModifyGoldGained", "AfterModifyingGoldGained",
    }),
    "RELIC.PRISMATIC_GEM": frozenset({
        "ModifyCardRewardCreationOptions",
    }),
    "RELIC.SOZU": frozenset({"ShouldProcurePotion"}),
}

# Reviewed classifications that the raw translator cannot own by itself.
# Keep these here, beside generation, so a clean regeneration cannot turn an
# audited unsupported relic into `untouched` or re-refuse a hand model.
REVIEWED_REFUSALS = {
    # Class-scoped inert reviews still retain their frozen translator
    # provenance in the generated artifact.
    "RELIC.LOOMING_FRUIT": "unsupported hook HasCornucopia",
    "RELIC.PAELS_WING":
        "unsupported hook TryModifyCardRewardAlternatives",
    "RELIC.LUCKY_FYSH": "unsupported hook IsAllowed",
    "RELIC.DARKSTONE_PERIAPT":
        "unsupported hook AfterCardChangedPiles",
    "RELIC.BING_BONG": "unsupported hook AfterCardChangedPiles",
    # Deliberate strict refusal retained by Batch 174.
    "RELIC.VAKUU_CARD_SELECTOR": "unsupported hook GetSelectedCards",
}
REVIEWED_HAND_MODELS = {
    # Batch 189: exact fifth manual-play Energy/Star absolute-zero hooks.
    "RELIC.BRILLIANT_SCARF",
    # Exact AfterBlockBroken implementation is hand-modeled by #345.
    "RELIC.HAND_DRILL",
    # Stateful AfterStarsSpent consumers (#427): Galactic Dust carries its
    # counter across combats; Mini Regent has an owner-turn latch.
    "RELIC.GALACTIC_DUST",
    "RELIC.MINI_REGENT",
    # Exact post-batch AfterDiedToDoom fatal-predicate heal (#432).
    "RELIC.BOOK_REPAIR_KNIFE",
    # Batch 178: persistent one-shot ShouldDieLate + selected preventer heal.
    "RELIC.LIZARD_TAIL",
    # Batch 179: bespoke local-owner pet/summon and Paels block lifecycle.
    "RELIC.BOUND_PHYLACTERY",
    "RELIC.BYRDPIP",
    "RELIC.PAELS_LEGION",
    "RELIC.PHYLACTERY_UNBOUND",
    # Batch 181: bespoke draw callers with persistent counters/latches.
    "RELIC.JOSS_PAPER",
    "RELIC.UNCEASING_TOP",
    # Batch 182: persistent exact AfterCardPlayed counters with awaited
    # Draw/block suffixes.
    "RELIC.IRON_CLUB",
    "RELIC.TUNING_FORK",
    # Batch 183: Claws is an exactly logged AfterObtained-only transform;
    # Music Box is a physical Before/AfterCardPlayed clone continuation.
    "RELIC.CLAWS",
    "RELIC.MUSIC_BOX",
    # Batch 184: creator-gated generated-card listener with an awaited flat
    # block command and a resumable later-listener suffix.
    "RELIC.REGALITE",
    # Batch 185: exact victory-only threshold heal projected at the solver
    # objective boundary.
    "RELIC.MEAT_ON_THE_BONE",
}


class Fork(Exception):
    def __init__(self, cond, target):
        self.cond, self.target = cond, target


class RelicInterp(Interp):
    """Interp + predicate forking. run() explores every predicate
    branch both ways (bounded) and returns leaves:
    [(conds, effects, retval)]."""

    MAX_LEAVES = 16

    def __init__(self, meta, tr, body, args):
        super().__init__(meta, tr, body, args)
        self.conds = []

    def snapshot(self):
        import copy
        return (list(self.stack), dict(self.locals), dict(self.fields),
                [list(e) for e in self.effects], list(self.conds),
                set(self.visited))

    def restore(self, snap):
        stack, locs, flds, effs, conds, vis = snap
        self.stack = list(stack)
        self.locals = dict(locs)
        self.fields = dict(flds)
        self.effects = [list(e) for e in effs]
        self.conds = list(conds)
        self.visited = set(vis)

    def run_forked(self):
        leaves = []

        def explore(ip):
            steps = 0
            while True:
                steps += 1
                if steps > self.MAX_STEPS:
                    raise Refuse("instruction budget exceeded")
                if ip >= len(self.insns):
                    raise Refuse("fell off method end")
                ins = self.insns[ip]
                if ins.offset in self.visited:
                    raise Refuse(f"revisited IL_{ins.offset:04x} (loop)")
                self.visited.add(ins.offset)
                try:
                    jump = self.step(ins)
                except Fork as f:
                    if len(leaves) + 2 > self.MAX_LEAVES:
                        raise Refuse("too many predicate forks")
                    snap = self.snapshot()
                    # taken path: predicate true
                    self.conds.append(f.cond)
                    explore(self.by_off[f.target])
                    # fall-through: predicate false
                    self.restore(snap)
                    neg = f.cond[1] if f.cond[0] == "not" \
                        else ("not", f.cond)
                    self.conds.append(neg)
                    explore(ip + 1)
                    return
                if jump == "ret":
                    ret = self.stack[-1] if self.stack else None
                    leaves.append((list(self.conds),
                                   [list(e) for e in self.effects], ret))
                    return
                ip = self.by_off[jump] if jump is not None else ip + 1

        explore(0)
        return leaves

    # branch handling: predicates fork, everything else defers to the
    # strict base implementation
    def step(self, ins):
        op = ins.opcode.name
        if op == "isinst":
            v = self.deref(self.stack[-1]) if self.stack else None
            if v == ("room",):
                self.pop()
                tok = getattr(ins.operand, "value", 0)
                table, rid = (tok >> 24) & 0xFF, tok & 0xFFFFFF
                if table == 0x02:
                    tname = self.meta.type_name[rid - 1][1]
                elif table == 0x01:
                    tname = str(self.meta.md.TypeRef.rows[rid - 1]
                                .TypeName)
                else:
                    raise Refuse(f"isinst token table {table:#x}")
                # value doubles as the null-checked cast result; only
                # cosmetic consumers accept it beyond the branch
                self.stack.append(("pred", ("room_type", tname)))
                return None
            raise Refuse(f"isinst on {v[0] if v else 'empty'}")
        if op in ("brtrue", "brtrue.s", "brfalse", "brfalse.s"):
            v = self.deref(self.stack[-1]) if self.stack else None
            if v is not None and v[0] == "pred":
                self.pop()
                cond = v[1]
                if op.startswith("brfalse"):
                    cond = ("not", cond) if cond[0] != "not" else cond[1]
                raise Fork(cond, int(ins.operand))
        if op in ("beq", "beq.s", "bne.un", "bne.un.s", "blt", "blt.s",
                  "ble", "ble.s", "bgt", "bgt.s", "bge", "bge.s"):
            if len(self.stack) >= 2:
                b = self.deref(self.stack[-1])
                a = self.deref(self.stack[-2])
                cond = self.tr.compare_pred(a, b, op)
                if cond is not None:
                    self.pop(2)
                    raise Fork(cond, int(ins.operand))
        return super().step(ins)


def cond_json(c):
    """nested cond tuple -> JSON-friendly nested list"""
    return [cond_json(x) if isinstance(x, tuple) else x for x in c]


class RelicTranslator(Translator):
    """Relic-context call semantics on top of the card vocabulary."""

    CMP_EQ = {"beq", "beq.s"}
    CMP_NE = {"bne.un", "bne.un.s"}
    CMP_ORDER = {"blt": "lt", "blt.s": "lt", "ble": "le", "ble.s": "le",
                 "bgt": "gt", "bgt.s": "gt", "bge": "ge", "bge.s": "ge"}

    def compare_pred(self, a, b, op):
        """-> predicate for a comparison branch, or None (let the strict
        base evaluate consts / refuse)."""
        def is_owner_creature(v):
            return v in (("player",), ("owner",))
        pair = {a[0], b[0]}
        if "turnnum" in pair:
            other = b if a[0] == "turnnum" else a
            if other[0] != "int":
                raise Refuse(f"turn compare vs {other[0]}")
            n = other[1]
            if op in self.CMP_EQ:
                return ("turn", "eq", n)
            if op in self.CMP_NE:
                return ("not", ("turn", "eq", n))
            o = self.CMP_ORDER.get(op)
            if o is None:
                raise Refuse(f"turn compare op {op}")
            if a[0] != "turnnum":                # const OP turn -> flip
                o = {"lt": "gt", "le": "ge", "gt": "lt", "ge": "le"}[o]
            return ("turn", o, n)
        if a[0] == "harg" or b[0] == "harg":
            other = b if a[0] == "harg" else a
            if is_owner_creature(other) and op in self.CMP_EQ | self.CMP_NE:
                cond = ("is_owner",)
                return cond if op in self.CMP_EQ else ("not", cond)
            raise Refuse(f"harg compare vs {other[0]}")
        return None

    def new_object(self, interp, name, gargs, args):
        cls = name.split("::")[0]
        if cls == "ThrowingPlayerChoiceContext":
            return ("ctx",)
        return super().new_object(interp, name, gargs, args)

    def call(self, interp, name, gargs, argv, void):
        cls, meth = name.split("::", 1) if "::" in name else ("", name)
        base = f"{cls.split('.')[-1]}::{meth}"
        deref = interp.deref

        if base == "OstyCmd::Summon":
            # The shared card translator gained OstyCmd::Summon support
            # (SUM1 #338 / SUM2 #341), but only CARD summons carry the
            # per-card audit (_SUMMON_CARDS_MODELED allowlist in
            # combat_sim). No relic summon has been audited — keep the
            # relic-side refusal reason stable (Phylactery Unbound) so
            # the coverage matrix does not silently downgrade it from
            # refused to untouched (#315 regen).
            raise Refuse("call OstyCmd::Summon")

        if base == "Decimal::op_Addition":
            a, b = deref(argv[0]), deref(argv[1])
            base_v = [v for v in (a, b) if v == ("basevalue",)]
            amt = [v for v in (a, b) if v[0] in ("varval", "dec", "int")]
            if len(base_v) == 1 and len(amt) == 1:
                return ("sum_base", self.amount(amt[0]))
            raise Refuse(f"op_Addition on {a[0]}/{b[0]}")
        if base == "RelicModel::get_Owner":
            return ("owner",)
        if base == "RelicModel::get_DynamicVars":
            return ("varset",)
        if base in ("RelicModel::Flash", "RelicModel::.ctor"):
            return None if void else ("task", None)
        if meth in ("set_IsActivating", "UpdateDisplay",
                    "DoActivateVisuals", "set_Status",
                    "InvokeDisplayAmountChanged"):
            # relic display plumbing (verified: setter = AssertMutable +
            # backing store + UpdateDisplay -> set_Status/UI only)
            return None if void else ("opaque", "display")
        if base == "Player::get_PlayerCombatState":
            return ("pcs",)
        if base == "PlayerCombatState::get_TurnNumber":
            return ("turnnum",)
        if base == "CombatManager::get_IsInProgress":
            return ("pred", ("in_progress",))
        if meth == "Contains" and gargs == ["Creature"]:
            # participants.Contains(Owner.Creature)
            target = deref(argv[-1])
            src = deref(argv[0])
            if src == ("participants",) and target == ("player",):
                return ("pred", ("own_side",))
            raise Refuse(f"Contains on {src}/{target}")
        if base == "ICombatState::get_HittableEnemies":
            return ("all_enemies",)
        if base == "Creature::get_CombatState" or \
                base == "Player::get_CombatState" or \
                base == "RelicModel::get_CombatState":
            return ("combatstate",)

        # ---- effect commands with relic shapes
        if base == "CreatureCmd::GainBlock":
            argv = [deref(a) for a in argv]
            if argv[0] != ("player",):
                raise Refuse(f"GainBlock on {argv[0]}")
            amounts = [a for a in argv[1:] if a[0] in ("var", "varval",
                                                       "dec")]
            if len(amounts) != 1:
                raise Refuse(f"GainBlock shape {argv}")
            interp.effects.append(["block", self.amount(amounts[0])])
            return ("task", ("opaque", "dec"))
        if base == "PlayerCmd::GainStars":
            argv = [deref(a) for a in argv]
            amounts = [a for a in argv if a[0] in ("var", "varval", "dec")]
            if len(amounts) != 1 or not any(
                    a in (("player",), ("owner",)) for a in argv):
                raise Refuse(f"GainStars shape {argv}")
            interp.effects.append(["stars", self.amount(amounts[0])])
            return None if void else ("task", None)
        if base == "CardPileCmd::Draw":
            argv = [deref(a) for a in argv]
            amounts = [a for a in argv if a[0] in ("var", "varval", "dec")]
            if len(amounts) != 1 or not any(a in (("owner",), ("player",))
                                            for a in argv):
                raise Refuse(f"Draw shape {argv}")
            interp.effects.append(["draw", self.amount(amounts[0])])
            return None if void else ("task", None)
        if base == "CreatureCmd::Heal":
            argv = [deref(a) for a in argv]
            if argv[0] != ("player",):
                raise Refuse(f"Heal on {argv[0]}")
            amounts = [a for a in argv[1:] if a[0] in ("var", "varval",
                                                       "dec")]
            if len(amounts) != 1:
                raise Refuse(f"Heal shape {argv}")
            interp.effects.append(["heal", self.amount(amounts[0])])
            return ("task", ("opaque", "dec"))
        if base == "CreatureCmd::Damage":
            argv = [deref(a) for a in argv]
            targets = [a for a in argv if a[0] in ("all_enemies",)]
            amounts = [a for a in argv if a[0] in ("var", "varval", "dec")]
            props = [a for a in argv if a[0] == "int" and a[1] in (4, 6)]
            if len(targets) == 1 and len(amounts) == 1:
                if len(props) == 1:
                    p = props[0][1]
                elif amounts[0][0] == "var":
                    p = None      # DamageVar overload: props live in the
                    #               var; resolved (and gated to 4/6) from
                    #               CanonicalVars at emission time
                else:
                    raise Refuse(f"relic Damage props {argv}")
                interp.effects.append(
                    ["damage_all", self.amount(amounts[0]), p])
                return ("task", ("opaque", "results"))
            raise Refuse(f"relic Damage shape {argv}")
        if base.startswith("PowerCmd::Apply"):
            # (ctx, target, amount, applier, sourceModel, flag) — relics
            # pass null as sourceModel
            argv = [deref(a) for a in argv]
            if len(gargs) != 1:
                raise Refuse("Apply without power type")
            if len(argv) != 6 or argv[0] != ("ctx",):
                raise Refuse(f"relic Apply arity {len(argv)}")
            tgt, amt = argv[1], argv[2]
            if amt[0] not in ("var", "varval", "dec", "int"):
                raise Refuse(f"relic Apply amount {amt}")
            if tgt == ("player",):
                interp.effects.append(
                    ["power", gargs[0], "self", self.amount(amt)])
            elif tgt == ("all_enemies",):
                interp.effects.append(
                    ["power_all", gargs[0], self.amount(amt)])
            else:
                raise Refuse(f"relic Apply target {tgt}")
            return ("task", ("opaque", "power"))
        if base == "Player::get_Creature":
            # hook-arg players are the owner in single-player runs (an
            # is_owner guard usually precedes; the sim only replays
            # single-player .runs)
            if deref(argv[0])[0] in ("owner", "harg"):
                return ("player",)
            raise Refuse("get_Creature on non-owner")

        return super().call(interp, name, gargs, argv, void)


class RelicTemplater(CardTranslator):
    def __init__(self):
        self.meta = Meta()
        self.tr = RelicTranslator(self.meta)

    def hook_args(self, sm_ti):
        """Symbolic values for a hook state machine's argument fields,
        keyed by field name (assigned in do_field/ldfld via
        self.fields)."""
        # ldfld of the arg fields: participants / player / room ...
        # Interp resolves cardPlay/choiceContext specially; for relics we
        # preload the fields dict instead.
        names = {}
        t = self.meta.md.TypeDef.rows[sm_ti]
        for f in (t.FieldList or []):
            n = str(f.row.Name)
            if n.startswith("<>"):
                continue
            if n == "participants":
                names[n] = ("participants",)
            elif n in ("player", "owner"):
                names[n] = ("harg", n)
            elif n == "room":
                names[n] = ("room",)
            elif n == "choiceContext":
                names[n] = ("ctx",)
            elif n == "combatState":
                names[n] = ("combatstate",)
            elif n == "creature":
                names[n] = ("harg", "creature")
            else:
                raise Refuse(f"hook arg field {n}")
        return names

    def async_hook(self, ti, hook_name):
        sm = self.meta.nested_named(ti, f"<{hook_name}>")
        if sm is None:
            # synchronous hook: the method body IS the logic (returns
            # Task.CompletedTask); args per the hook's known layout
            layout = HOOK_ARGS.get(hook_name)
            if layout is None:
                raise Refuse(f"{hook_name}: no state machine")
            row = self.method(ti, hook_name)
            body = self.meta.body_of(row)
            if body is None:
                raise Refuse(f"{hook_name}: no body")
            args = {0: ("this",)}
            for i, v in enumerate(layout, start=1):
                args[i] = v
            it = RelicInterp(self.meta, self.tr, body, args)
            leaves = it.run_forked()
        else:
            mn = self.method(sm, "MoveNext")
            body = self.meta.body_of(mn)
            if body is None:
                raise Refuse(f"{hook_name}: no body")
            it = RelicInterp(self.meta, self.tr, body, {0: ("smthis",)})
            it.fields.update(self.hook_args(sm))
            leaves = it.run_forked()
        out = []
        for conds, effects, _ret in leaves:
            if effects:
                out.append({"conds": [cond_json(c) for c in conds],
                            "steps": effects})
        # effectless leaves are guard exits; leaves are pairwise disjoint
        # by construction (each is one full branch path), so the sim can
        # apply every satisfied leaf independently
        return out

    def sync_value_hook(self, ti, hook_name, kind):
        row = self.method(ti, hook_name)
        body = self.meta.body_of(row)
        if body is None:
            raise Refuse(f"{hook_name}: no body")
        it = RelicInterp(self.meta, self.tr, body,
                         {0: ("this",), 1: ("harg", 1), 2: ("basevalue",),
                          3: ("harg", 3)})
        leaves = it.run_forked()
        out = []
        for conds, effects, ret in leaves:
            if effects:
                raise Refuse(f"{hook_name}: side effects in modifier")
            ret = it.deref(ret) if ret else None
            if ret == ("basevalue",) or ret is None:
                continue
            if ret and ret[0] == "sum_base":
                out.append({"conds": [cond_json(c) for c in conds],
                            "steps": [[kind, ret[1]]]})
                continue
            raise Refuse(f"{hook_name}: return shape {ret}")
        return out

    def translate_relic(self, ti, info, rid):
        hooks_out = []
        byname = self.meta.type_methods[ti]
        reviewed_noncombat = REVIEWED_NONCOMBAT_METHODS.get(rid, frozenset())
        missing_reviewed = reviewed_noncombat - set(byname)
        if missing_reviewed:
            raise Refuse(
                f"reviewed noncombat methods missing: "
                f"{sorted(missing_reviewed)}")
        for mname in byname:
            if mname in IGNORED_METHODS or mname in reviewed_noncombat \
                    or mname.startswith("get_") \
                    or mname.startswith("set_"):
                continue
            if mname in SYNC_VALUE_HOOKS:
                hooks_out.append((mname,
                                  self.sync_value_hook(
                                      ti, mname, SYNC_VALUE_HOOKS[mname])))
            elif mname in ASYNC_HOOKS:
                hooks_out.append((mname, self.async_hook(ti, mname)))
            elif mname.startswith("<"):
                continue
            else:
                raise Refuse(f"unsupported hook {mname}")
        vars_ = self.canonical_vars(ti)
        # resolve damage_all props left on the DamageVar (None marker)
        for _h, guarded in hooks_out:
            for leaf in guarded:
                for st in leaf["steps"]:
                    if st[0] == "damage_all" and st[2] is None:
                        key = st[1]["var"] if isinstance(st[1], dict) \
                            else None
                        p = vars_.get(key, {}).get("props") if key else None
                        if p not in (4, 6):
                            raise Refuse(f"damage_all var props {p}")
                        st[2] = p
        return {
            "class": self.meta.type_name[ti][1],
            "vars": vars_,
            "hooks": [{"hook": h, "guarded": g} for h, g in hooks_out
                      if g],
        }

    def run_relics(self):
        census = json.load(open(Path(__file__).parent.parent
                                / "relics_census.json"))
        relics, refused = {}, {}
        for ti, (ns, name) in self.meta.type_name.items():
            if ns != RELIC_NS or name.startswith("<"):
                continue
            rid = "RELIC." + snake_case(name).upper()
            c = census.get(rid)
            if c is None or not c["combat_active"]:
                continue                        # inert: already whitelisted
            try:
                out = self.translate_relic(ti, c, rid)
                if not out["hooks"]:
                    refused[rid] = "no template-expressible hooks"
                else:
                    relics[rid] = out
            except Refuse as e:
                refused[rid] = str(e)
            except Exception as e:              # pragma: no cover
                refused[rid] = f"internal: {type(e).__name__}: {e}"
        for rid in REVIEWED_HAND_MODELS:
            relics.pop(rid, None)
            refused.pop(rid, None)
        for rid, reason in REVIEWED_REFUSALS.items():
            relics.pop(rid, None)
            refused[rid] = reason
        return relics, refused


def main():
    rt = RelicTemplater()
    relics, refused = rt.run_relics()
    print(json.dumps({"relics": relics, "refused": refused},
                     indent=1, sort_keys=True))
    print(f"templated {len(relics)} / {len(relics) + len(refused)} "
          f"combat-active relics", file=sys.stderr)


if __name__ == "__main__":
    main()
