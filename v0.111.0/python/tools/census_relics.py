#!/usr/bin/env python3
"""Relic census for the coverage pipeline (#58).

For every relic class in sts2.dll, list the hooks it overrides and
classify it combat-inert vs combat-active. A relic is inert for the
fight sim when every method it declares is out-of-combat (pickup,
shop, rest site, rewards, post-combat) or cosmetic — post-combat
effects (heal, gold) don't need modeling because each fight's entry
state is read from the .run file, not simulated forward.

Hook names NOT in either list are treated as combat-active
(conservative) and reported so the lists stay complete.

Output: JSON {RELIC.ID: {class, rarity, hooks, combat_active,
unknown_hooks}}.

Usage: census_relics.py > ../relics_census.json
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

RELIC_NS = "MegaCrit.Sts2.Core.Models.Relics"

# Declarative / cosmetic / out-of-combat: never touches an in-progress
# fight. get_*/set_* accessors are handled separately (state, not hooks).
INERT_METHODS = {
    ".ctor", ".cctor", "SetupForPlayer", "SetupForTests",
    "AfterObtained", "AfterCloned", "IsAllowed", "IsAllowedAtNeow",
    # NB: AfterRoomEntered is NOT here. It fires on entering a combat room
    # — BEFORE the fight — so its effects persist into combat (caught live:
    # BronzeScales applies its ThornsPower there, not in a combat hook).
    # It gets a body scan below: inert only if the closure touches no
    # combat command.
    "AfterCombatEnd", "AfterCombatVictory", "AfterCombatVictoryEarly",
    "WillHealOnCombatFinished",                   # post-combat: entry state
    "AfterModifyingRewards", "TryModifyRewards",  # comes from the .run
    "TryModifyRewardsLate", "GenerateRewards", "BeforeCombatRewardOffered",
    "GenerateRandomBundles", "CanGenerateBundles", "ShouldGenerateTreasure",
    "ShouldForcePotionReward", "AfterPotionProcured", "ShouldProcurePotion",
    "TryModifyCardRewardOptions", "TryModifyCardRewardOptionsLate",
    "TryModifyCardRewardAlternatives", "AfterModifyingCardRewardOptions",
    "ModifyCardRewardCreationOptions", "GetSelectedCardReward",
    "TryModifyRestSiteOptions", "ModifyExtraRestSiteHealText",
    "AfterRestSiteHeal", "ModifyRestSiteHealAmount",
    "TryModifyRestSiteHealRewards", "ShouldDisableRemainingRestSiteOptions",
    "ModifyMerchantCardCreationResults", "ModifyMerchantPrice",
    "AfterItemPurchased", "ShouldRefillMerchantEntry", "PurchaseEverything",
    "TryModifyCardBeingAddedToDeck", "ModifyGoldGained",
    "AfterModifyingGoldGained", "AfterGoldGained",
    "ModifyGeneratedMap", "ModifyGeneratedMapLate",
    "ModifyUnknownMapPointRoomTypes", "AddMarkedRooms", "GetMarkedCoords",
    "ShouldAllowFreeTravel",
    "GetValidRelics", "GetStarterRelic", "GetUpgradedStarterRelic",
    "GetTranscendenceStarterCard", "GetTranscendenceTransformedCard",
    "GetCardPool", "CardPoolFilter", "Filter",
    "EnchantCard", "EnchantValidCards",   # enchant-at-reward: deck state
    #                                       is read from the .run
    "TryModifyStarCost",
    "UpdateDisplay", "DoActivateVisuals", "RefreshCounter",
    "RefreshStatus", "UpdateCardList", "UpdateHoverTips",
    "DebugAddCard", "SetTestEnergyCostOverride",
    "ApplyTestEnergyCostOverrideToPower", "GetSkillsPlayedForTest",
    "CheckIfUsedUp",
}

# Methods whose names are combat-sensitive elsewhere but whose complete
# call closure is inert for one exact relic class. Keep these exemptions
# class-scoped: adding any of these names to INERT_METHODS would silently
# admit a future combat-active implementation on another relic.
CLASS_INERT_METHODS = {
    # v0.109.0 / c12f634d (#599): AfterCardChangedPiles reaches its
    # CardsToSkip/CloneCard/Add recursion only after exact Deck (6), owner,
    # and clonedBy-null gates. Combat represents and mutates only pile types
    # Draw through Play (1-5); later fight entries already contain the
    # resulting persistent deck clones and skip-set consequences.
    "BingBong": {"AfterCardChangedPiles"},
    # v0.109.0 / c12f634d (#596): AfterCardChangedPiles owner-gates a
    # Curse and activates only when its destination is PileType.Deck (6),
    # the persistent master deck. Combat represents and mutates only pile
    # types Draw through Play (1-5); later fight entries already contain the
    # resulting current/max-HP state.
    "DarkstonePeriapt": {"AfterCardChangedPiles"},
    # v0.109.0 / c12f634d (#594): AfterCardChangedPiles owner-gates and
    # activates only when the destination is PileType.Deck (6), the
    # persistent master deck. Combat represents and mutates only pile types
    # Draw through Play (1-5); later fight entries already contain the
    # resulting gold.
    "LuckyFysh": {"AfterCardChangedPiles"},
    # v0.109.0 / c12f634d (#588): HasCornucopia reads the save UniqueId
    # parity and its sole DLL caller is get_IconBaseName (presentation).
    # AfterObtained's max-HP pickup is already reflected in each fight entry.
    "LoomingFruit": {"HasCornucopia"},
    # v0.109.0 / c12f634d (#590): OnSacrifice is referenced only by this
    # relic's TryModifyCardRewardAlternatives. Its persistent counter and
    # every-second-sacrifice relic obtain occur in post-combat reward flow;
    # later fight entries already contain the resulting run state.
    "PaelsWing": {"OnSacrifice"},
}

COMBAT_METHODS = {
    "BeforeCombatStart", "BeforeCombatStartLate", "AfterCombatStart",
    "BeforeSideTurnStart", "AfterSideTurnStart",
    "BeforeSideTurnEnd", "BeforeSideTurnEndEarly",
    "BeforeSideTurnEndVeryEarly", "AfterSideTurnEnd",
    "AfterPlayerTurnStart", "AfterPlayerTurnStartLate",
    "BeforeCardPlayed", "AfterCardPlayed", "AfterCardPlayedLate",
    "BeforeHandDraw", "ModifyHandDraw", "ModifyHandDrawLate",
    "ModifyMaxEnergy", "AfterEnergyReset", "AfterEnergyResetLate",
    "AfterDamageReceived", "AfterDamageGiven", "AfterAttack",
    "AfterBlockCleared", "ShouldClearBlock", "AfterPreventingBlockClear",
    "ModifyBlockMultiplicative", "AfterModifyingBlockAmount",
    "ModifyDamageAdditive", "ModifyDamageMultiplicative",
    "ModifyHpLostAfterOsty", "AfterModifyingHpLostAfterOsty",
    "ModifyHpLostAfterOstyLate",
    "AfterCardExhausted", "AfterCardChangedPiles", "AfterShuffle",
    "AfterCardDiscarded", "AfterCardEnteredCombat",
    "AfterCardGeneratedForCombat", "AfterHandEmptied",
    "AfterPotionUsed", "AfterPotionDiscarded", "SummonPet",
    "AfterCreatureAddedToCombat", "AfterCurrentHpChanged", "AfterDeath",
    "AfterDiedToDoom", "AfterPreventingDeath", "AfterPreventingDraw",
    "ShouldDie", "ShouldDieLate", "ShouldDraw", "DrawIfThresholdMet",
    "ApplyPower", "ApplyDexterity", "RemoveDexterity",
    "ModifyStrengthIfNecessary", "ModifyVulnerableMultiplier",
    "ModifyWeakMultiplier", "ModifyXValue",
    "ModifyPowerAmountGivenAdditive", "ModifyPowerAmountGivenMultiplicative",
    "AfterModifyingPowerAmountGiven", "AfterModifyingPowerAmountReceived",
    "TryModifyPowerAmountReceived", "BeforePowerAmountChanged",
    "TryModifyEnergyCostInCombat", "TryModifyEnergyCostInCombatLate",
    "ShouldModifyCost", "ShouldPlayerResetEnergy",
    "ShouldTakeExtraTurn", "AfterTakingExtraTurn",
    "NotifyAttackPlayed", "NotifySkillPlayed", "AnyCardsPlayedThisTurn",
    "ModifyCardPlayCount", "AfterModifyingCardPlayCount",
    "AfterOrbChanneled", "ModifyOrbValue", "ModifyOrbPassiveTriggerCounts",
    "AfterModifyingOrbPassiveTriggerCount", "AfterFlush", "ShouldFlush",
    "AfterStarsSpent",
    "GetTarget", "ShouldPlay", "IsValidPhase", "CanAffect",
    "Grow", "Rekindle", "OnSacrifice", "CreateMaulFromOriginal",
    "HasCornucopia", "HasDoubledTemporaryPowerSource",
    "GetDefendForCharacter", "GetStrikeForCharacter",
    # v0.109 hook names (#315), all raised inside the combat pipeline
    # (build v0.109.0 c12f634d):
    # - AfterBlockBroken: CreatureCmd::Damage <Damage>d__12 0x432a14
    #   (per DamageResult with WasBlockBroken) and CreatureCmd::LoseBlock
    #   <LoseBlock>d__19 0x435b2c (block >0 -> <=0). Hand Drill applies
    #   VulnerablePower there.
    # - AfterAutoPrePlayPhaseEntered(-Late): Hook d__58, raised from
    #   CombatManager::RunAutoPrePlayPhase <...>d__107 (turn pipeline;
    #   HistoryCourse / WhisperingEarring auto-play cards there).
    # - AfterModifyingHandDraw: raised from CombatManager::SetupPlayerTurn
    #   <...>d__108 (turn-start hand draw; Pocketwatch / PollinousCore).
    # - GetSelectedCards: CardSelectCmd::FromHand / FromCombatPile
    #   <...>d__28/d__20 (in-combat card selection; VakuuCardSelector).
    "AfterBlockBroken",
    "AfterAutoPrePlayPhaseEntered", "AfterAutoPrePlayPhaseEnteredLate",
    "AfterModifyingHandDraw", "GetSelectedCards",
}


# Combat-effect calls: if AfterRoomEntered's body closure touches any of
# these, the relic mutates combat state before the fight -> active.
COMBAT_CALL_PREFIXES = ("PowerCmd::", "CreatureCmd::", "CardPileCmd::",
                        "CardCmd::", "DamageCmd::", "AttackCommand::",
                        "CardSelectCmd::", "PlayerCmd::GainEnergy",
                        "PlayerCmd::GainStars")
COSMETIC_CALLS = ("CreatureCmd::TriggerAnim",)


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

    method_owner = {}
    for ti, t in enumerate(md.TypeDef.rows):
        for m in (t.MethodList or []):
            method_owner[m.row_index] = ti

    memberref = {}
    for i, r in enumerate(md.MemberRef.rows):
        cls = r.Class
        parent = ""
        if cls.table and cls.table.name == "TypeRef":
            parent = str(md.TypeRef.rows[cls.row_index - 1].TypeName)
        memberref[i + 1] = f"{parent}::{r.Name}"

    enclosing = {}
    nc = getattr(md, "NestedClass", None)
    if nc:
        for row in nc.rows:
            enclosing[row.NestedClass.row_index - 1] = \
                row.EnclosingClass.row_index - 1

    def body_calls(row, tname_of):
        if not row.Rva:
            return []
        try:
            b = CilMethodBody(RawReader(raw, pe.get_offset_from_rva(row.Rva)))
        except MethodBodyFormatError:
            return []
        out = []
        for insn in b.instructions:
            if insn.opcode.name not in ("call", "callvirt", "newobj"):
                continue
            val = getattr(insn.operand, "value", None)
            if val is None:
                continue
            table, rid = (val >> 24) & 0xFF, val & 0xFFFFFF
            if table == 0x06:
                oi = method_owner.get(rid)
                out.append(f"{tname_of(oi)}::"
                           f"{md.MethodDef.rows[rid - 1].Name}")
            elif table == 0x0A:
                out.append(memberref.get(rid, ""))
            elif table == 0x2B:
                # generic instantiation (PowerCmd.Apply<X>): resolve the
                # base method name; the arg doesn't matter for this check
                spec = md.MethodSpec.rows[rid - 1].Method
                if spec.table and spec.table.name == "MethodDef":
                    oi = method_owner.get(spec.row_index)
                    out.append(f"{tname_of(oi)}::"
                               f"{md.MethodDef.rows[spec.row_index - 1].Name}")
                elif spec.table:
                    out.append(memberref.get(spec.row_index, ""))
        return out

    # First pass: per-TypeDef declared hook sets + Extends edges, so hooks
    # inherited from intermediate base classes count against the subclass.
    declared, extends, ns_name = {}, {}, {}
    room_entered_rows = {}          # ti -> [MethodDef rows to body-scan]
    for ti, t in enumerate(md.TypeDef.rows):
        type_name = str(t.TypeName)
        ns_name[ti] = (str(t.TypeNamespace), type_name)
        ext = t.Extends
        if ext and ext.table and ext.table.name == "TypeDef":
            extends[ti] = ext.row_index - 1
        hooks, unknown = set(), set()
        for m in (t.MethodList or []):
            mn = str(m.row.Name)
            if mn == "AfterRoomEntered":
                room_entered_rows.setdefault(ti, []).append(m.row)
                continue
            if (mn in INERT_METHODS
                    or mn in CLASS_INERT_METHODS.get(type_name, set())
                    or mn.startswith("<")):
                continue
            if mn.startswith("get_") or mn.startswith("set_"):
                continue        # properties: vars/counters/flags, no hook
            if mn in COMBAT_METHODS:
                hooks.add(mn)
            else:
                unknown.add(mn)
        declared[ti] = (hooks, unknown)

    def tname_of(ti):
        return ns_name[ti][1] if ti is not None else "?"

    # Body-scan AfterRoomEntered (the method + its async state machine,
    # nested types named <AfterRoomEntered>...): combat call -> active.
    for ti, rows in room_entered_rows.items():
        calls = []
        for row in rows:
            calls += body_calls(row, tname_of)
        for n, e in enclosing.items():
            if e == ti and ns_name[n][1].startswith("<AfterRoomEntered>"):
                for m in (md.TypeDef.rows[n].MethodList or []):
                    calls += body_calls(m.row, tname_of)
        hit = sorted({c for c in calls
                      if any(c.startswith(p) for p in COMBAT_CALL_PREFIXES)
                      and c not in COSMETIC_CALLS})
        if hit:
            declared[ti][0].add(f"AfterRoomEntered[{','.join(hit)}]")

    # RelicModel itself defines the virtual hook surface (empty defaults) —
    # only count hooks declared BELOW it in the chain.
    def chain_hooks(ti):
        hooks, unknown = set(), set()
        cur = ti
        while cur is not None:
            _, tn = ns_name[cur]
            if tn == "RelicModel":
                break
            h, u = declared[cur]
            hooks |= h
            unknown |= u
            cur = extends.get(cur)
        return hooks, unknown

    def is_relic_model(ti):
        """Namespace helpers are not relic census rows.

        v0.109.1's VakuuCardSelector lives in the Relics namespace but
        implements ICardSelector and derives directly from Object. Require
        the actual RelicModel base in the TypeDef inheritance chain.
        """
        cur = ti
        while cur is not None:
            if ns_name[cur][1] == "RelicModel":
                return True
            cur = extends.get(cur)
        return False

    out = {}
    unknown_global = set()
    for ti, (ns, name) in ns_name.items():
        if ns != RELIC_NS or name.startswith("<") or name == "RelicModel":
            continue
        if not is_relic_model(ti):
            continue
        hooks, unknown = chain_hooks(ti)
        unknown_global |= unknown
        relic_id = "RELIC." + snake_case(name).upper()
        out[relic_id] = {
            "class": name,
            "hooks": sorted(hooks),
            "unknown_hooks": sorted(unknown),
            "combat_active": bool(hooks or unknown),
        }

    if unknown_global:
        print(f"UNKNOWN HOOK NAMES (classify these!): "
              f"{sorted(unknown_global)}", file=sys.stderr)
    print(json.dumps(out, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
