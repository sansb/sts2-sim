//! Monster-move bodies for the `content/encounters/normal.py` pool — GENERATED ONCE, THEN OWNED BY HAND.
//!
//! PORT_PLAN.md D4. `versions/v0.111.0/rust/tools/generate_content.py`
//! created this file with a refusing stub per kind and will APPEND a stub for
//! any kind that later joins this family, but it never rewrites, reorders, or
//! removes what is already here: the bodies are hand-written ports and this
//! file is the wave PR's private edit surface.
//!
//! Filling a stub is a three-line contract:
//!
//! 1. replace the `Err(...)` body with the port of the cited Python branch;
//! 2. add the kind to [`IMPLEMENTED`] — the capability manifest and the
//!    admission gate are both derived from it (D6), so an unlisted body is
//!    unreachable. The source-derived family-triage gate also rejects a listed
//!    body that directly names its own `*KindNotModeled` refusal; focused crate
//!    tests remain the runtime evidence;
//! 3. leave the signature alone; [`super`]'s generated `match` calls it.
//!
//! # R2 status (#1560): 36 of 36 ported
//!
//! R2's batches add exact command/lifecycle shapes for Myte status
//! injection, Forgotten's Dexterity-scaled attack, Guardbot team block, Gas
//! Bomb's attack-then-unblockable self-damage, Living Fog's Gas Bomb spawn,
//! Ovicopter's retained-egg attack/lay/hatch machine, The Obscura's illusion
//! opener plus complete random AI, and Fogmog/Scroll of Biting's exact
//! in-handler AI branches with their Dazed/Paper Cuts closures, plus Owl
//! Magistrate's exact Soar/Verdict lifecycle, the Lost/Forgotten pair's exact
//! owner-scoped Possess debit/restore lifecycle, and Fabricator's exact
//! conditional AI, bot spawns, High Voltage, and Dazed injection, plus Louse
//! Progenitor's Curl Up lifecycle and exact three-move cycle, plus the
//! Two-Tailed Rat's complete summon/AI/roster closure. Batch J adds Frog
//! Knight's Plating/one-shot charge branch and Axebot's Stock replacement
//! machine. R43 adds Gremlin Merc's exact Thievery/Surprise/Heist lineage and
//! Fat Gremlin Escape. R44 closes the family with Thieving Hopper's exact
//! theft, Flutter, Swipe-return, stun-follow-up, and Escape lifecycle.

use super::MoveCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::move_constants as mc;
use crate::engine::cards::{inject_legacy_bottom, inject_legacy_draw_random};
use crate::engine::damage::{
    apply_player_duration_affliction, monster_attack_player_hopper_transient,
    monster_attack_player_with_catalog, note_power,
};
use crate::engine::{EngineRefusal, Subject};
use crate::hot::PileId;
use crate::ids::{CardId, MonsterKind, MoveKind, PowerId, StepWord};
use crate::powers::SlotWire;

/// The MoveKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[MoveKind] = &[
    MoveKind::AddStatusHand,
    MoveKind::AttackCharge,
    MoveKind::AttackDexterity,
    MoveKind::AttackScrollBranch,
    MoveKind::AttackSmoggy,
    MoveKind::AttackSpawnAggro,
    MoveKind::AttackSteal,
    MoveKind::AttackStrengthBranch,
    MoveKind::AttackTangle,
    MoveKind::AttackVulnerableDeferredOvicopter,
    MoveKind::AxebotBootup,
    MoveKind::Bloat,
    MoveKind::BuffTeamBlock,
    MoveKind::Escape,
    MoveKind::Explode,
    MoveKind::Fabricate,
    MoveKind::HatchToughEgg,
    MoveKind::Haunt,
    MoveKind::HopperEscape,
    MoveKind::HopperFlutter,
    MoveKind::HopperThievery,
    MoveKind::LayToughEggs,
    MoveKind::LouseCurlGrow,
    MoveKind::LousePounce,
    MoveKind::LouseWeb,
    MoveKind::NoiseStatus,
    MoveKind::PileInject,
    MoveKind::PossessSpeed,
    MoveKind::PossessStrength,
    MoveKind::Soar,
    MoveKind::SummonIllusion,
    MoveKind::SummonRat,
    MoveKind::Verdict,
    MoveKind::VulnPlayer,
    MoveKind::WeakPlayer,
    MoveKind::WeakPlayerStrength,
];

/// Every kind this file owns a body for, ascending — the list the escalation
/// tests walk. `super::FAMILY_OF` is the generated authority; this mirrors the
/// `normal` rows of it, and a test below pins the two against each other so a
/// kind appended by the generator cannot slip past the interlock.
#[cfg(test)]
const OWNED: [MoveKind; 36] = [
    MoveKind::AddStatusHand,
    MoveKind::AttackCharge,
    MoveKind::AttackDexterity,
    MoveKind::AttackScrollBranch,
    MoveKind::AttackSmoggy,
    MoveKind::AttackSpawnAggro,
    MoveKind::AttackSteal,
    MoveKind::AttackStrengthBranch,
    MoveKind::AttackTangle,
    MoveKind::AttackVulnerableDeferredOvicopter,
    MoveKind::AxebotBootup,
    MoveKind::Bloat,
    MoveKind::BuffTeamBlock,
    MoveKind::Escape,
    MoveKind::Explode,
    MoveKind::Fabricate,
    MoveKind::HatchToughEgg,
    MoveKind::Haunt,
    MoveKind::HopperEscape,
    MoveKind::HopperFlutter,
    MoveKind::HopperThievery,
    MoveKind::LayToughEggs,
    MoveKind::LouseCurlGrow,
    MoveKind::LousePounce,
    MoveKind::LouseWeb,
    MoveKind::NoiseStatus,
    MoveKind::PileInject,
    MoveKind::PossessSpeed,
    MoveKind::PossessStrength,
    MoveKind::Soar,
    MoveKind::SummonIllusion,
    MoveKind::SummonRat,
    MoveKind::Verdict,
    MoveKind::VulnPlayer,
    MoveKind::WeakPlayer,
    MoveKind::WeakPlayerStrength,
];

/// `add_status_hand` — Myte TOXIC (#191).
///
/// Python: `monster_act` (frozen, deleted #2827) — one `_pile_inject` to (hand, bottom), which
/// [`pile_inject`] below already ports through [`inject_legacy_bottom`]
/// (`MAX_CARDS_IN_HAND` overflow redirect included). The nested generated
/// identity is compiled and authenticated before the exact Hand/Bottom add;
/// Toxic's independent keyword gate remains visible at admission.
pub(crate) fn add_status_hand(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::List(identity), CompiledArg::I(count)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("add_status_hand"));
    };
    let [CompiledArg::Card(id), CompiledArg::I(upgrade)] = ctx.catalog.args(*identity) else {
        return Err(EngineRefusal::MalformedArgs("add_status_hand identity"));
    };
    let upgrade = u8::try_from(*upgrade)
        .map_err(|_| EngineRefusal::MalformedArgs("add_status_hand upgrade"))?;
    let count = usize::try_from(*count)
        .map_err(|_| EngineRefusal::CounterOverflow("add_status_hand count"))?;
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::Myte
        || (*id, upgrade, count) != (CardId::Toxic, 0, 2)
    {
        return Err(EngineRefusal::MalformedArgs("add_status_hand owner/row"));
    }
    inject_legacy_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: *id,
            upgrade,
            enchantment: None,
        },
        count,
        PileId::Hand,
    )
}

/// `attack_charge` — Frog Knight BEETLE_CHARGE.
///
/// Python: `monster_act` (frozen, deleted #2827) — set the one-shot charged latch, then attack.
///
/// The owner-disjoint hot latch is written before the attack, including on a
/// lethal result. Frog Knight's `hp_below_half_once` successor later reads
/// that latch after BUFF_STRENGTH; the monster-side Plating listener runs
/// independently before the ACT walk.
///
/// #2828: the damage is `FrogKnight::get_BeetleChargeDamage` RVA `0xb5345`,
/// `GetValueIfAscension(9, 40, 35)` at the fight's tier.
pub(crate) fn attack_charge(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_charge"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::FROG_KNIGHT_BEETLE_CHARGE_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_charge"));
    }
    let owner = &ctx.state.monsters[ctx.actor];
    if owner.kind != MonsterKind::FrogKnight || owner.louse_curled() || owner.loop_pos != 3 {
        return Err(EngineRefusal::MalformedArgs("attack_charge owner/state"));
    }
    // `louse_curled` is an owner-disjoint hot latch. The boundary publishes
    // it as `beetle_charged` only for Frog Knight and as `louse_curled` only
    // for Louse Progenitor, so the shared bit cannot cross the two lifecycles.
    ctx.state.monsters_mut()[ctx.actor].set_louse_curled(true);
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)
}

/// `attack_dexterity` — The Forgotten DREAD.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack for the ascension base plus the
/// owner's own live Dexterity. `TheForgotten::get_DreadDamage` RVA `0xc1368`
/// IL_000c-IL_0026 is `GetValueIfAscension(9, 15, 13)` plus the owner's
/// `DexterityPower` amount; the row carries the tier (#2828).
///
/// The branch authenticates its two operands, snapshots the actor's live
/// monster Dexterity into the base, then enters the ordinary attack fold.
/// The possess-roster validator remains an independent entry refusal.
pub(crate) fn attack_dexterity(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_dexterity"));
    };
    if *damage < 0 || *hits < 1 {
        return Err(EngineRefusal::MalformedArgs("attack_dexterity"));
    }
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::TheForgotten
        || (*damage, *hits) != (ctx.tier(mc::THE_FORGOTTEN_DREAD_DAMAGE), 1)
    {
        return Err(EngineRefusal::MalformedArgs("attack_dexterity owner/row"));
    }
    let dexterity = i64::from(
        ctx.state.monsters[ctx.actor]
            .powers
            .value(PowerId::Dexterity),
    );
    let damage = damage
        .checked_add(dexterity)
        .ok_or(EngineRefusal::CounterOverflow("attack_dexterity damage"))?;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, *hits, ctx.events)
}

/// `attack_scroll_branch` — Scroll of Biting CHEW.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, then roll the follow-up branch.
///
/// The powered two-hit attack completes first. CHEW's `FollowUpState` is the
/// `rand` RandomBranchState (`ScrollOfBiting::GenerateMoveStateMachine` RVA
/// `0xbcde4` IL_00a1-IL_00a4), and native traverses it only when the machine
/// next rolls: `MonsterModel/<PerformMove>d__105::MoveNext` RVA `0x31d794`
/// IL_017f-IL_0184 ends a move with `MonsterMoveStateMachine::OnMovePerformed`
/// RVA `0x78e88` alone, and the roll is `Creature::PrepareForNextTurn` RVA
/// `0x11d85c` IL_002a (`MonsterModel::RollMove` RVA `0x825c8`), reached from
/// `CombatManager/<StartTurn>d__100::MoveNext` RVA `0x3f781c` IL_036b-IL_03a0
/// over `CombatState.Enemies` on the next player side. So a surviving Scroll
/// only records the pending traversal here (#3304); `prepare_random_ai`
/// spends its one MonsterAi draw (`roll_scroll_of_biting_branch`, #3026) if
/// the Scroll is still alive then. A Scroll that dies later in this enemy
/// phase (Doom at BeforeSideTurnEnd) never draws. `monster_act`'s ordinary
/// tail advance is suppressed: the loop stays on CHEW until that roll.
pub(crate) fn attack_scroll_branch(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_scroll_branch"));
    };
    // `ScrollOfBiting::get_ChewDamage` RVA `0xbcd28`, (9, 6, 5) (#2828).
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::ScrollOfBiting
        || ctx.state.monsters[ctx.actor].loop_pos != 1
        || (*damage, *hits) != (ctx.tier(mc::SCROLL_OF_BITING_CHEW_DAMAGE), 2)
    {
        return Err(EngineRefusal::MalformedArgs(
            "attack_scroll_branch owner/row",
        ));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        ctx.state.monsters_mut()[ctx.actor].set_scroll_branch_pending(true);
    }
    Ok(())
}

/// `("attack_smoggy", damage, hits, amount)` — Living Fog ADVANCED_GAS:
/// attack, then stack fresh Smoggy while the actor and combat remain live.
/// The damage is `LivingFog::get_AdvancedGasDamage` RVA `0xb88db`,
/// `GetValueIfAscension(9, 9, 8)` at the fight's tier (#2828).
pub(crate) fn attack_smoggy(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_smoggy"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::LIVING_FOG_ADVANCED_GAS_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_smoggy"));
    }
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::LivingFog {
        return Err(EngineRefusal::MalformedArgs("attack_smoggy owner"));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::Smoggy,
            PowerId::SmoggyFresh,
            1,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `attack_spawn_aggro` — Fabricator FABRICATING_STRIKE_MOVE.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, then spawn one aggro bot.
///
/// Exact row: authenticate `(damage, 1)`, validate the complete Fabricator
/// command boundary, attack, then re-resolve the live owner by uid before one
/// aggressive bot spawn. Frozen Python `monster_act` (deleted #2827).
///
/// The damage is `Fabricator::get_FabricatingStrikeDamage` RVA `0xb3ba1`,
/// `GetValueIfAscension(9, 21, 18)` at the fight's tier (#2828).
///
/// Spawn semantics are `_spawn_fabricator_bot` (frozen Python, deleted #2827).
pub(crate) fn attack_spawn_aggro(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_spawn_aggro"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::FABRICATOR_FABRICATING_STRIKE_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_spawn_aggro"));
    }
    let owner = &ctx.state.monsters[ctx.actor];
    if owner.kind != MonsterKind::Fabricator {
        return Err(EngineRefusal::MalformedArgs("attack_spawn_aggro owner/row"));
    }
    let owner_uid = owner.uid;
    crate::engine::monsters::require_fabricator_state(ctx.state, owner_uid)?;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    crate::engine::monsters::spawn_fabricator_bot(
        ctx.state,
        owner_uid,
        &crate::engine::monsters::FABRICATOR_AGGRO_BOTS,
    )
}

/// `attack_steal` — Gremlin Merc GIMME / DOUBLE_SMASH / HEHE.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, then the owner's Thievery debit,
/// then the row's optional player Weak or owner Strength suffix.
///
/// Current-build authority: `sts2.dll` SHA-256 `9cb4f1ad…12b4`;
/// GremlinMerc Gimme/DoubleSmash/Hehe wrappers RVAs
/// `0xb606c`/`0xb60b8`/`0xb6104`, nested bodies
/// `0x35dd94`/`0x35db00`/`0x35e044`, and Thievery Steal
/// `0xaa058` -> `0x3497fc`. The nested Steal body debits Gold before adding
/// the same amount to its instance DynamicVar. The three damages are gated at
/// **A8**, not A9: `get_GimmeDamage` `0xb5f1e` (8, 8, 7), `get_DoubleSmashDamage`
/// `0xb5f2b` (8, 7, 6) and `get_HeheDamage` `0xb5f38` (8, 9, 8), at the
/// fight's tier (#2828).
pub(crate) fn attack_steal(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(weak),
        CompiledArg::I(strength),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_steal"));
    };
    let row = (*damage, *hits, *weak, *strength);
    if ![
        (ctx.tier(mc::GREMLIN_MERC_GIMME_DAMAGE), 2, 0, 0),
        (ctx.tier(mc::GREMLIN_MERC_DOUBLE_SMASH_DAMAGE), 2, 2, 0),
        (ctx.tier(mc::GREMLIN_MERC_HEHE_DAMAGE), 1, 0, 2),
    ]
    .contains(&row)
        || !crate::engine::monsters::gremlin_merc_state_is_valid(ctx.state)
        || ctx.actor != 0
        || ctx.state.monsters[0].kind != MonsterKind::GremlinMerc
        || ctx.state.monsters[0].hp <= 0
        || ctx.state.gold < 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Gremlin Merc attack/Thievery entry",
        ));
    }
    let owner_uid = ctx.state.monsters[0].uid;
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if ctx.state.history.over || ctx.state.hp <= 0 {
        return Ok(());
    }
    let owner = ctx
        .state
        .monsters
        .iter()
        .position(|monster| monster.uid == owner_uid);
    if let Some(index) = owner.filter(|index| ctx.state.monsters[*index].hp > 0) {
        let stolen = ctx.state.gold.min(20);
        let planned_stolen = ctx.state.monsters[index]
            .powers
            .value(PowerId::StolenGold)
            .checked_add(stolen)
            .ok_or(EngineRefusal::CounterOverflow("Gremlin Merc StolenGold"))?;
        ctx.state.gold -= stolen;
        ctx.state.monsters_mut()[index].powers.set(
            PowerId::StolenGold,
            SlotWire::Int,
            planned_stolen,
        );
        note_power(
            ctx.events,
            Subject::Monster(owner_uid),
            PowerId::StolenGold,
            planned_stolen,
        );
    }
    if *weak > 0 && !ctx.state.history.over && ctx.state.hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerWeak,
            PowerId::PlayerWeakFresh,
            i32::try_from(*weak).expect("authenticated Gremlin Merc Weak"),
            true,
            ctx.events,
        )?;
    }
    if *strength > 0
        && let Some(index) = ctx
            .state
            .monsters
            .iter()
            .position(|monster| monster.uid == owner_uid && monster.hp > 0)
    {
        let planned = crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[index],
            i32::try_from(*strength).expect("authenticated Gremlin Merc Strength"),
            "Gremlin Merc Hehe Strength",
        )?;
        // `owner_uid` is `ctx.actor` (the entry gate above pins the Merc at
        // slot 0 and `ctx.actor == 0`), so this is the ordinary enemy
        // self-buff: applier is the acting creature.
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        crate::engine::damage::write_monster_self_strength(
            &mut ctx.state.monsters_mut()[index],
            planned,
            upkeep,
        );
        note_power(
            ctx.events,
            Subject::Monster(owner_uid),
            PowerId::Strength,
            planned,
        );
    }
    Ok(())
}

/// `attack_strength_branch` — Fogmog SWIPE_MOVE.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, self Strength, then roll the branch.
///
/// The attack completes first, then a surviving Fogmog gains one Strength and
/// consumes one MonsterAi draw at this exact in-handler point. The binary32
/// 0.4/0.6 branch parks the loop on SWIPE_RANDOM or HEADBUTT; the ordinary
/// `monster_act` tail advance is suppressed because this body prepared its
/// own successor. Fogmog's exact illusion spawn landed with Batch B.
pub(crate) fn attack_strength_branch(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(strength),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_strength_branch"));
    };
    // `Fogmog::get_SwipeDamage` RVA `0xb4c9b`, (9, 9, 8) (#2828).
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::Fogmog
        || ctx.state.monsters[ctx.actor].loop_pos != 1
        || (*damage, *hits, *strength) != (ctx.tier(mc::FOGMOG_SWIPE_DAMAGE), 1, 1)
    {
        return Err(EngineRefusal::MalformedArgs(
            "attack_strength_branch owner/row",
        ));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let planned_strength = crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[ctx.actor],
            1,
            "attack_strength_branch Strength",
        )?;
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let uid = monster.uid;
        crate::engine::damage::write_monster_self_strength(monster, planned_strength, upkeep);
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Strength,
            planned_strength,
        );
        crate::engine::turn::roll_fogmog_branch(ctx.state, ctx.actor)?;
    }
    Ok(())
}

/// `("attack_tangle", damage, hits, amount)` — attack, then apply the unique
/// Tangled instance. A live restack is deliberately typed and refused. The
/// damage is `VineShambler::get_GraspingVinesDamage` RVA `0xc4a3f`,
/// `GetValueIfAscension(9, 9, 8)` at the fight's tier (#2828).
pub(crate) fn attack_tangle(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_tangle"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::VINE_SHAMBLER_GRASPING_VINES_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_tangle"));
    }
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::VineShambler {
        return Err(EngineRefusal::MalformedArgs("attack_tangle owner"));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        if ctx.state.powers.value(PowerId::Tangled) > 0 {
            return Err(EngineRefusal::PowerRestackNotModeled(PowerId::Tangled));
        }
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            ctx.state,
            PowerId::Tangled,
            0,
            1,
        )?;
        ctx.state.powers.set(PowerId::Tangled, SlotWire::Int, 1);
        note_power(ctx.events, Subject::Player, PowerId::Tangled, 1);
    }
    Ok(())
}

/// `attack_vulnerable_deferred_ovicopter` — Ovicopter TENDERIZER.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, player Vulnerable with the fresh
/// latch, then park the loop on the summon branch instead of advancing.
///
/// The attack and fresh Vulnerable finish before the loop is parked on the
/// interior SUMMON_BRANCH sentinel. `engine::turn` suppresses the ordinary
/// tail advance and resolves that sentinel only after the complete enemy
/// side-end death/duration walk. The damage is
/// `Ovicopter::get_TenderizerDamage` RVA `0xbaa5d`,
/// `GetValueIfAscension(9, 8, 7)` at the fight's tier (#2828).
pub(crate) fn attack_vulnerable_deferred_ovicopter(
    ctx: &mut MoveCtx<'_>,
) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs(
            "attack_vulnerable_deferred_ovicopter",
        ));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::OVICOPTER_TENDERIZER_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs(
            "attack_vulnerable_deferred_ovicopter",
        ));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::Ovicopter {
        return Err(EngineRefusal::MalformedArgs(
            "attack_vulnerable_deferred_ovicopter owner",
        ));
    }
    let uid = ctx.state.monsters[ctx.actor].uid;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    let Some(index) = ctx
        .state
        .monsters
        .iter()
        .position(|monster| monster.uid == uid)
    else {
        return Ok(());
    };
    if !ctx.state.history.over && ctx.state.monsters[index].hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerVuln,
            PowerId::PlayerVulnFresh,
            2,
            true,
            ctx.events,
        )?;
        ctx.state.monsters_mut()[index].loop_pos = crate::engine::monsters::OVICOPTER_BRANCH_POS;
    }
    Ok(())
}

/// `axebot_bootup` — Axebot BOOT_UP.
///
/// Python: `monster_act` (frozen, deleted #2827) — Block, then Strength scaled by the
/// post-decrement respawn stock.
///
/// Stock's death listener validates the singleton retained owner, decrements
/// before replacement, consumes exactly one Niche HP draw, publishes a fresh
/// uid in slot zero, and re-enters at BOOT_UP. Therefore the live post-respawn
/// Stock amount here is exactly 1 then 0, yielding Strength 4 then 8 after the
/// Block command. Axebot's loop also carries `attack_weak_frail`, whose body
/// is `moves/shared.rs`'s.
///
/// #2828: `Axebot::get_BootUpBlock` RVA `0xaed63` is
/// `GetValueIfAscension(9, 15, 10)` and `get_BootUpStrGain` RVA `0xaed7d` is
/// `(9, 4, 3)`; `<BootUpMove>d__36::MoveNext` RVA `0x352eb0` gains the block,
/// then applies `StrGain * (2 - Stock)`, both at the fight's tier.
pub(crate) fn axebot_bootup(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(block), CompiledArg::I(gain)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("axebot_bootup"));
    };
    let (block, gain) = (*block, *gain);
    if (block, gain)
        != (
            ctx.tier(mc::AXEBOT_BOOT_UP_BLOCK),
            ctx.tier(mc::AXEBOT_BOOT_UP_STR_GAIN),
        )
    {
        return Err(EngineRefusal::MalformedArgs("axebot_bootup"));
    }
    let block = i32::try_from(block).map_err(|_| EngineRefusal::MalformedArgs("axebot_bootup"))?;
    let gain = i32::try_from(gain).map_err(|_| EngineRefusal::MalformedArgs("axebot_bootup"))?;
    let owner = &ctx.state.monsters[ctx.actor];
    let stock = owner.powers.value(PowerId::Stock);
    if owner.kind != MonsterKind::Axebot
        || owner.loop_pos != 2
        || !matches!(stock, 0 | 1)
        || owner.hp <= 0
    {
        return Err(EngineRefusal::MalformedArgs("axebot_bootup owner/state"));
    }
    let strength = gain
        .checked_mul(2 - stock)
        .ok_or(EngineRefusal::CounterOverflow("axebot_bootup Strength"))?;
    let updated_block = owner
        .block
        .checked_add(block)
        .ok_or(EngineRefusal::CounterOverflow("axebot_bootup block"))?;
    let updated_strength = crate::engine::damage::checked_monster_strength_successor(
        owner,
        strength,
        "axebot_bootup Strength",
    )?;
    let uid = owner.uid;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let owner = &mut ctx.state.monsters_mut()[ctx.actor];
    owner.block = updated_block;
    crate::engine::damage::write_monster_self_strength(owner, updated_strength, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Strength,
        updated_strength,
    );
    Ok(())
}

/// `bloat` — Living Fog BLOAT.
///
/// Python: `monster_act` (frozen, deleted #2827) — spawn a Gas Bomb, then attack.
///
/// `_spawn_gasbomb` publishes the first free fixed bomb slot and creation uid
/// after exactly one Niche draw. Because slot order inserts the bomb before
/// its owner, the attack re-resolves the Living Fog by uid. The damage is
/// `LivingFog::get_BloatDamage` RVA `0xb88e7`, `GetValueIfAscension(9, 6, 5)`
/// at the fight's tier (#2828).
pub(crate) fn bloat(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("bloat"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::LIVING_FOG_BLOAT_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("bloat"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::LivingFog {
        return Err(EngineRefusal::MalformedArgs("bloat owner"));
    }
    let uid = ctx.state.monsters[ctx.actor].uid;
    crate::engine::monsters::spawn_gas_bomb(ctx.state, uid)?;
    let Some(index) = ctx
        .state
        .monsters
        .iter()
        .position(|monster| monster.uid == uid)
    else {
        return Ok(());
    };
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, index, damage, 1, ctx.events)
}

/// `buff_team_block` — Guardbot GUARD_MOVE: 15 Block to each Fabricator.
///
/// v0.111.0 (`sts2.dll` SHA-256 `9cb4f1ad…12b4`, #3145):
/// `<GuardMove>d__8::MoveNext` (RVA `0x35e3b0`) reads
/// `CombatState.Enemies` (`IL_009e`-`IL_00a4`), filters it with
/// `<>c::<GuardMove>b__8_0` (RVA `0x35e2ba`: `get_Monster`, then `isinst`
/// TypeDef `0x0200050b` = `MegaCrit.Sts2.Core.Models.Monsters.Fabricator`)
/// through `Enumerable.Where` + `ToList` (`IL_00c8`/`IL_00cd`), then awaits
/// `CreatureCmd.GainBlock(member, 15m, ValueProp 4, null, false)` for each
/// (`IL_00f3`-`IL_00ff`). So the Block goes to Fabricators only: never to
/// the Guardbot itself, nor to any other bot. `Enemies` holds no removed
/// corpse ([`crate::engine::monsters::in_native_enemies`], which refuses by
/// name on a represented death veto), so a dead Fabricator gains nothing.
///
/// The recipient uids are snapshotted first (the `ToList`); each is then
/// re-resolved before its grant, and the loop stops once combat is over.
pub(crate) fn buff_team_block(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(raw)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("buff_team_block"));
    };
    let amount: i32 = (*raw)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("buff_team_block block"))?;
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("buff_team_block"));
    }
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::Guardbot || amount != 15 {
        return Err(EngineRefusal::MalformedArgs("buff_team_block owner/row"));
    }
    let mut recipients: Vec<u32> = Vec::new();
    for monster in ctx.state.monsters.iter() {
        if monster.kind == crate::ids::MonsterKind::Fabricator
            && crate::engine::monsters::in_native_enemies(monster)?
        {
            recipients.push(monster.uid);
        }
    }
    for uid in recipients {
        if ctx.state.history.over {
            break;
        }
        let Some(index) = ctx
            .state
            .monsters
            .iter()
            .position(|monster| monster.uid == uid && monster.hp > 0)
        else {
            continue;
        };
        let monster = &mut ctx.state.monsters_mut()[index];
        monster.block = monster
            .block
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
    }
    Ok(())
}

/// `escape` — Fat Gremlin FLEE.
///
/// Python: `monster_act` (frozen, deleted #2827) — validate the Heist/Merc reward projection,
/// drop the creature from the roster, and end combat if nobody is left.
///
/// Current-build authority: `sts2.dll` SHA-256 `9cb4f1ad…12b4`; Fat Gremlin
/// Flee wrapper RVA `0xb45c8`, nested body `0x35b240`. Its sole command is
/// `CreatureCmd.Escape(owner, true)`: removal without Before/AfterDeath or
/// Heist reward, followed by terminal recomputation.
pub(crate) fn escape(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || !crate::engine::monsters::gremlin_merc_state_is_valid(ctx.state)
        || ctx.state.monsters.get(ctx.actor).is_none_or(|monster| {
            monster.kind != MonsterKind::FatGremlin
                || monster.hp <= 0
                || monster.slot != 2
                || Some(monster.uid) != crate::engine::monsters::gremlin_merc_fat_uid(ctx.state)
                || monster.spawn_noop
        })
    {
        return Err(EngineRefusal::MalformedArgs("Fat Gremlin Escape entry"));
    }
    let uid = ctx.state.monsters[ctx.actor].uid;
    let mut next = ctx.state.clone();
    let index = next
        .monsters
        .iter()
        .position(|monster| monster.uid == uid)
        .ok_or(EngineRefusal::MalformedArgs("Fat Gremlin Escape identity"))?;
    next.monsters_mut().remove(index);
    if !next.any_monster_alive() {
        next.history.over = true;
    }
    if !crate::engine::monsters::gremlin_merc_state_is_valid(&next) {
        return Err(EngineRefusal::MalformedArgs("Fat Gremlin Escape result"));
    }
    let ended = next.history.over && !ctx.state.history.over;
    *ctx.state = next;
    if ended {
        ctx.events
            .push(crate::engine::Event::CombatOver { player_won: true });
    }
    Ok(())
}

/// `explode` — Gas Bomb EXPLODE.
///
/// Python: `monster_act` (frozen, deleted #2827) — attack, then an unblockable null-dealer
/// self-kill through the full kill pipeline.
///
/// After the attack, a surviving bomb receives unblockable, unpowered damage
/// equal to live HP plus Block through the ordinary monster death pipeline.
/// Gas Bomb spawning remains an independent entry refusal.
pub(crate) fn explode(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("explode"));
    };
    if *damage < 0 || *hits < 1 {
        return Err(EngineRefusal::MalformedArgs("explode"));
    }
    // `GasBomb::get_ExplodeDamage` RVA `0xb5a90`, (9, 9, 8) (#2828).
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::GasBomb
        || (*damage, *hits) != (ctx.tier(mc::GAS_BOMB_EXPLODE_DAMAGE), 1)
    {
        return Err(EngineRefusal::MalformedArgs("explode owner/row"));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if ctx.state.monsters[ctx.actor].hp > 0 {
        let amount = i64::from(ctx.state.monsters[ctx.actor].hp)
            .checked_add(i64::from(ctx.state.monsters[ctx.actor].block))
            .ok_or(EngineRefusal::CounterOverflow("explode self-kill"))?;
        // `GasBomb/<ExplodeMove>d__20::MoveNext` RVA `0x35d1b0` (v0.111.0 DLL
        // 9cb4f1ad…) awaits the attack, then `CreatureCmd.Kill(Creature,
        // false)` (IL_00ce-IL_00d4), whose `Hook.AfterDeath` walk reaches
        // Gremlin Horn. The self-kill keeps the move's catalog for that Draw
        // (#3172), as Waterfall Giant's Explode does (#3166).
        crate::engine::damage::damage_monster_after_catalog_auth(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            crate::decimal::DotNetDecimal::from_i64(amount),
            false,
            false,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `fabricate` — Fabricator FABRICATE_MOVE.
///
/// Python `monster_act` (frozen, deleted #2827): spawn a defense bot, then, if the owner is
/// still live and combat has not ended, an aggro bot.
///
/// Exact body validates the complete command boundary once, then performs the
/// serial defensive and aggressive `SpawnBot` commands. Each helper
/// re-resolves the immutable owner uid because the first child can insert
/// before the Fabricator in slot order.
///
/// Python `_spawn_fabricator_bot` (frozen, deleted #2827) owns each child lifecycle.
pub(crate) fn fabricate(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() || ctx.state.monsters[ctx.actor].kind != MonsterKind::Fabricator {
        return Err(EngineRefusal::MalformedArgs("fabricate owner/row"));
    }
    let owner_uid = ctx.state.monsters[ctx.actor].uid;
    crate::engine::monsters::require_fabricator_state(ctx.state, owner_uid)?;
    crate::engine::monsters::spawn_fabricator_bot(
        ctx.state,
        owner_uid,
        &crate::engine::monsters::FABRICATOR_DEFENSE_BOTS,
    )?;
    crate::engine::monsters::spawn_fabricator_bot(
        ctx.state,
        owner_uid,
        &crate::engine::monsters::FABRICATOR_AGGRO_BOTS,
    )
}

/// `hatch_tough_egg` — Tough Egg HATCH_MOVE.
///
/// Python: `monster_act` (frozen, deleted #2827) — `_hatch_tough_egg` replaces the egg with its
/// hatchling in place.
///
/// The retained creature keeps slot, uid, Block, and MinionPower; every other
/// represented power is cleared before one Niche draw chooses 20..23 HP and
/// the loop moves permanently to NIBBLE.
pub(crate) fn hatch_tough_egg(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() || ctx.state.monsters[ctx.actor].kind != MonsterKind::ToughEgg {
        return Err(EngineRefusal::MalformedArgs("hatch_tough_egg owner/row"));
    }
    let uid = ctx.state.monsters[ctx.actor].uid;
    crate::engine::monsters::hatch_tough_egg(ctx.state, uid)
}

/// `("haunt", weak, dazed)` — fresh player Weak, then legacy Dazed cards at
/// Discard/Bottom. The encounter remains refused by its independent fixed
/// successor and Ethereal Dazed admission gaps.
pub(crate) fn haunt(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(3), CompiledArg::I(5)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("haunt"));
    };
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::HauntedShip {
        return Err(EngineRefusal::MalformedArgs("haunt owner"));
    }
    apply_player_duration_affliction(
        ctx.state,
        PowerId::PlayerWeak,
        PowerId::PlayerWeakFresh,
        3,
        true,
        ctx.events,
    )?;
    inject_legacy_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: CardId::Dazed,
            upgrade: 0,
            enchantment: None,
        },
        5,
        PileId::Discard,
    )
}

/// `hopper_escape` — Thieving Hopper ESCAPE.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Escape wrapper RVA `0xc2434`, nested body `0x370e30`. The owner leaves
/// combat without death/Swipe return; an empty living roster ends combat.
/// Python: `monster_act` (frozen, deleted #2827) — drop the creature from the roster, carrying
/// its stolen card out of the fight, and end combat if nobody is left.
pub(crate) fn hopper_escape(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.actor != 0
        || !crate::engine::monsters::thieving_hopper_state_is_valid(ctx.state)
        || ctx.state.monsters[0].hp <= 0
        || ctx.state.monsters[0].loop_pos != 4
    {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper Escape entry"));
    }
    let mut next = ctx.state.clone();
    next.monsters_mut().clear();
    next.history.over = true;
    if !crate::engine::monsters::thieving_hopper_state_is_valid(&next) {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper Escape result",
        ));
    }
    *ctx.state = next;
    ctx.events
        .push(crate::engine::Event::CombatOver { player_won: true });
    Ok(())
}

/// `hopper_flutter` — Thieving Hopper FLUTTER.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Flutter wrapper RVA `0xc23f0`, nested body `0x371034`; it installs exact
/// amount five once. The powered-damage reader owns all later decrements.
/// Python: `monster_act` (frozen, deleted #2827) — apply Flutter to the owner, refusing a
/// restack.
pub(crate) fn hopper_flutter(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(raw)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper Flutter row"));
    };
    if (*raw != i64::from(crate::engine::monsters::THIEVING_HOPPER_FLUTTER))
        || ctx.actor != 0
        || !crate::engine::monsters::thieving_hopper_state_is_valid(ctx.state)
        || ctx.state.monsters[0].hp <= 0
        || ctx.state.monsters[0].powers.value(PowerId::Flutter) != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper Flutter entry",
        ));
    }
    ctx.state.monsters_mut()[0].powers.set(
        PowerId::Flutter,
        SlotWire::Int,
        crate::engine::monsters::THIEVING_HOPPER_FLUTTER,
    );
    note_power(
        ctx.events,
        Subject::Monster(ctx.state.monsters[0].uid),
        PowerId::Flutter,
        crate::engine::monsters::THIEVING_HOPPER_FLUTTER,
    );
    Ok(())
}

/// `hopper_thievery` — Thieving Hopper THIEVERY.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Thievery wrapper RVA `0xc231c`, nested body `0x371428`; Swipe's Steal
/// wrapper/body are `0xa8fa8`/`0x347bbc`. The complete Draw+Discard
/// DeckVersion-filtered selection and persistent removal precede the attack.
/// Python: `monster_act` (frozen, deleted #2827) — `_hopper_theft` first, then the attack.
/// The damage is `ThievingHopper::get_TheftDamage` RVA `0xc21b9`,
/// `GetValueIfAscension(9, 19, 17)` at the fight's tier (#2828).
pub(crate) fn hopper_thievery(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper Thievery row"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::THIEVING_HOPPER_THEFT_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper Thievery row"));
    }
    if ctx.actor != 0
        || !crate::engine::monsters::thieving_hopper_state_is_valid(ctx.state)
        || ctx.state.monsters[0].hp <= 0
        || ctx.state.monsters[0].loop_pos != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper Thievery entry",
        ));
    }
    let mut next = ctx.state.clone();
    let mut emitted = Vec::new();
    crate::engine::monsters::hopper_steal_one_card(&mut next, ctx.catalog, 0)?;
    monster_attack_player_hopper_transient(&mut next, ctx.catalog, 0, damage, 1, &mut emitted)?;
    *ctx.state = next;
    ctx.events.extend(emitted);
    Ok(())
}

/// `lay_tough_eggs` — Ovicopter LAY_EGGS_MOVE.
///
/// Python: `monster_act` (frozen, deleted #2827) — spawn the fixed number of Tough Eggs.
///
/// Three serial `_spawn_tough_egg` commands fill the highest free named slot,
/// allocate contiguous creation uids, and consume one Niche draw apiece. A
/// full roster is an exact zero-RNG no-op for each remaining command.
pub(crate) fn lay_tough_eggs(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("lay_tough_eggs"));
    };
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::Ovicopter {
        return Err(EngineRefusal::MalformedArgs("lay_tough_eggs owner"));
    }
    let uid = ctx.state.monsters[ctx.actor].uid;
    for _ in 0..3 {
        crate::engine::monsters::spawn_tough_egg(ctx.state, uid)?;
    }
    Ok(())
}

/// `louse_curl_grow` — Louse Progenitor CURL_AND_GROW.
///
/// Python: `monster_act` (frozen, deleted #2827) — Block, Strength, then set the curled flag,
/// in that order, behind `_validate_curl_up_state`.
///
/// Curl Up itself is a monster power; `_curled` is independent private owner
/// state and deliberately is not represented as the generated
/// [`PowerId::LouseCurled`] vocabulary token. This body authenticates the
/// unique native row and performs Block, Strength, then `_curled = True`.
///
/// #2828: `LouseProgenitor::get_CurlBlock` RVA `0xb8e8b` is
/// `GetValueIfAscension(8, 18, 14)` — the same getter the spawn-time CurlUp
/// reads, gated at A8 — and `get_GrowStrength` RVA `0xb8e97` is `(9, 7, 5)`;
/// `<CurlAndGrowMove>d__32::MoveNext` reads both, at the fight's tier.
pub(crate) fn louse_curl_grow(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(block), CompiledArg::I(grow)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("louse_curl_grow"));
    };
    let (block, grow) = (*block, *grow);
    if (block, grow)
        != (
            ctx.tier(mc::LOUSE_PROGENITOR_CURL_BLOCK),
            ctx.tier(mc::LOUSE_PROGENITOR_GROW_STRENGTH),
        )
    {
        return Err(EngineRefusal::MalformedArgs("louse_curl_grow"));
    }
    let block =
        i32::try_from(block).map_err(|_| EngineRefusal::MalformedArgs("louse_curl_grow"))?;
    let grow = i32::try_from(grow).map_err(|_| EngineRefusal::MalformedArgs("louse_curl_grow"))?;
    let monster = &ctx.state.monsters[ctx.actor];
    if monster.kind != MonsterKind::LouseProgenitor {
        return Err(EngineRefusal::MalformedArgs("louse_curl_grow owner"));
    }
    let block = monster
        .block
        .checked_add(block)
        .ok_or(EngineRefusal::CounterOverflow("louse_curl_grow block"))?;
    let strength = crate::engine::damage::checked_monster_strength_successor(
        monster,
        grow,
        "louse_curl_grow Strength",
    )?;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    monster.block = block;
    crate::engine::damage::write_monster_self_strength(monster, strength, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(monster.uid),
        PowerId::Strength,
        strength,
    );
    monster.set_louse_curled(true);
    Ok(())
}

/// `louse_pounce` — Louse Progenitor POUNCE.
///
/// Python: `monster_act` (frozen, deleted #2827) — clear the curled flag, then attack.
///
/// The private `_curled` flag clears before the ordinary powered monster
/// attack, including a player-lethal attack. The damage is
/// `LouseProgenitor::get_PounceDamage` RVA `0xb8e7e`,
/// `GetValueIfAscension(9, 16, 14)` at the fight's tier (#2828).
pub(crate) fn louse_pounce(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("louse_pounce"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::LOUSE_PROGENITOR_POUNCE_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("louse_pounce"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::LouseProgenitor {
        return Err(EngineRefusal::MalformedArgs("louse_pounce owner"));
    }
    ctx.state.monsters_mut()[ctx.actor].set_louse_curled(false);
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)
}

/// `louse_web` — Louse Progenitor WEB_CANNON.
///
/// Python: `monster_act` (frozen, deleted #2827) — clear the curled flag, attack, then stack
/// player Frail with its fresh latch.
///
/// The private `_curled` flag clears first. The attack completes next; only a
/// surviving combat reaches the fresh two-turn player Frail application. The
/// damage is `LouseProgenitor::get_WebDamage` RVA `0xb8e71`,
/// `GetValueIfAscension(9, 10, 9)` at the fight's tier (#2828).
pub(crate) fn louse_web(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("louse_web"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::LOUSE_PROGENITOR_WEB_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("louse_web"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::LouseProgenitor {
        return Err(EngineRefusal::MalformedArgs("louse_web owner"));
    }
    let owner_uid = ctx.state.monsters[ctx.actor].uid;
    ctx.state.monsters_mut()[ctx.actor].set_louse_curled(false);
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over
        && ctx
            .state
            .monsters
            .iter()
            .any(|monster| monster.uid == owner_uid && monster.hp > 0)
    {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerFrail,
            PowerId::PlayerFrailFresh,
            2,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `noise_status` — Noisebot NOISE_MOVE.
///
/// Python `monster_act` (frozen, deleted #2827): one Dazed to the discard end, then a second
/// to a random draw-pile position.
///
/// Exact body inserts one legacy Dazed+0 at Discard/Bottom, then one at a
/// random Draw position. Only the latter spends one Shuffle draw, and its
/// insertion sees the live pile after the first serial command.
///
/// Python `_insert_draw_random` (frozen, deleted #2827) owns the RNG/insertion path.
pub(crate) fn noise_status(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.state.monsters[ctx.actor].kind != MonsterKind::Noisebot
        || !crate::engine::monsters::fabricator_state_is_valid(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("noise_status owner/state"));
    }
    let identity = CardIdentity {
        id: CardId::Dazed,
        upgrade: 0,
        enchantment: None,
    };
    if ctx.catalog.atom(&identity).is_none() {
        return Err(EngineRefusal::UnknownMintIdentity(identity));
    }
    inject_legacy_bottom(ctx.state, ctx.catalog, identity, 1, PileId::Discard)?;
    inject_legacy_draw_random(ctx.state, ctx.catalog, identity, 1)
}

/// `("pile_inject", (card, upgrade), count, pile, position)` — the generalized
/// monster status-card inject: Chomper SCREECH, Eye With Teeth DISTRACT, and
/// Slimed Berserker VOMIT_ICHOR.
///
/// Python: `monster_act` (frozen, deleted #2827) delegating to `_pile_inject`, which
/// is one primitive over `CardPileCmd.AddToCombatAndPreview` with an
/// IL-verified destination table and a refusal for anything outside it.
///
/// The two admitted destinations are ported here through the same
/// [`inject_legacy_bottom`] the Phrog's `add_status` and the Wriggler's
/// `wriggle` use:
///
/// * **(discard, bottom)** appends the copies to the discard end, with no
///   Shuffle draw. Every compiled row in this build takes this branch.
/// * **(hand, bottom)** appends to the player's hand end, redirecting each
///   copy that would exceed `MAX_CARDS_IN_HAND` to the discard — the per-card
///   `isFullHandAdd` check, which the shared primitive already implements.
///
/// (draw, random) is Python's third verified destination and refuses here: it
/// spends one Shuffle-stream draw per card, and no ported path advances that
/// stream. Python raises on every other pair, so the refusal shapes match.
///
/// Like its siblings this body appends compact legacy entries and deliberately
/// does not advance `next_card_uid`; the next identity-sensitive turn-start
/// pass allocates their physical uids in all-piles order.
///
/// No entry can reach this body yet — see the module header's reachability
/// note. The claim still pays for itself: it is what leaves ArtifactPower and
/// the Ethereal status-card keywords as the *only* named gaps on the Chomper
/// encounters, which `smoke/normal.json` probes.
pub(crate) fn pile_inject(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::List(identity),
        CompiledArg::I(count),
        CompiledArg::Pile(pile),
        CompiledArg::Word(position),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("pile_inject"));
    };
    let destination = match (*pile, *position) {
        (PileId::Discard, StepWord::Bottom) => PileId::Discard,
        (PileId::Hand, StepWord::Bottom) => PileId::Hand,
        _ => return Err(EngineRefusal::MalformedArgs("pile_inject destination")),
    };
    let [CompiledArg::Card(id), CompiledArg::I(upgrade)] = ctx.catalog.args(*identity) else {
        return Err(EngineRefusal::MalformedArgs("pile_inject identity"));
    };
    let upgrade =
        u8::try_from(*upgrade).map_err(|_| EngineRefusal::MalformedArgs("pile_inject upgrade"))?;
    let count =
        usize::try_from(*count).map_err(|_| EngineRefusal::MalformedArgs("pile_inject count"))?;
    inject_legacy_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: *id,
            upgrade,
            enchantment: None,
        },
        count,
        destination,
    )
}

/// `possess_speed` — The Forgotten MIASMA.
///
/// Python: `monster_act` (frozen, deleted #2827) — player Dexterity down through the exact
/// Possess applier, the ledger debit, then owner Block and owner Dexterity,
/// each independently observing the ending and owner liveness.
///
/// The admitted boundary is the native fixed solo roster. Its owner ledger is
/// a complete one-row dictionary keyed by the implicit local player zero;
/// relics (including Gremlin Horn and Ruined Helmet) and multiplayer are
/// independently refused at admission.
///
/// The owner Block is a powered monster-move Block, so it reads the owner's
/// live Dexterity from BEFORE this move's own Dexterity gain (#3214).
/// `TheForgotten/<MiasmaMove>d__12::MoveNext` (RVA `0x36f800`) orders
/// `Apply<DexterityPower>(-steal)` on the player (IL_00aa-IL_00c9), then
/// `CreatureCmd::GainBlock(owner, 8m, ValueProp 8 = Move, null, false)`
/// (IL_0124-IL_0133), then `Apply<DexterityPower>(+steal)` on the owner
/// (IL_0191-IL_01af). `Hook::ModifyBlock` (RVA `0x104c30`) folds every
/// listener's `ModifyBlockAdditive`, then every `ModifyBlockMultiplicative`,
/// and returns `Math.Max(0, amount)` (IL_0100-IL_0106).
/// `DexterityPower::ModifyBlockAdditive` (RVA `0xa17b8`, IL_002a-IL_0052)
/// adds the owner's Amount when the recipient is the owner and
/// `IsPoweredCardOrMonsterMoveBlock` (RVA `0xd70b`: Move set, Unpowered
/// clear) holds, which it does for props 8. So a second MIASMA over
/// Dexterity 2 gains 10 Block (fd41bbfb8f5a0529 step 24).
///
/// The other listeners that can move a no-card monster-move Block on its own
/// recipient are owner-held Frail (`FrailPower` RVA `0xa28ff`, not a
/// monster-representable power), `NoBlockPower` (`0xa4edf`),
/// `ShadowmeldPower` (`0xa74fc`) and `FastenPower` (`0xa22c0`); an owner
/// carrying any of the last three refuses by name. The card-gated relic and
/// enchantment listeners read a null card and are identity, and
/// `MultiplayerScalingModel` (`0x8fb24`) is identity solo.
pub(crate) fn possess_speed(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(steal), CompiledArg::I(block)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("possess_speed"));
    };
    // `TheForgotten::get_DebilitatingSmogDexStealAmount` RVA `0xc1390`,
    // (9, 2, 2) (#2828); the Block 8 is untiered.
    if (*steal, *block)
        != (
            ctx.tier(mc::THE_FORGOTTEN_DEBILITATING_SMOG_DEX_STEAL_AMOUNT),
            8,
        )
    {
        return Err(EngineRefusal::MalformedArgs("possess_speed row"));
    }
    crate::engine::monsters::require_possess_roster(
        ctx.state,
        ctx.actor,
        MonsterKind::TheForgotten,
    )?;
    if ctx.state.history.over || ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    let steal: i32 = (*steal)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("possess_speed amount"))?;
    let block: i32 = (*block)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("possess_speed block"))?;
    let player_dexterity = ctx
        .state
        .powers
        .value(PowerId::Dexterity)
        .checked_sub(steal)
        .ok_or(EngineRefusal::CounterOverflow("player Dexterity"))?;
    let owner = &ctx.state.monsters[ctx.actor];
    if [PowerId::NoBlock, PowerId::Shadowmeld, PowerId::Fasten]
        .into_iter()
        .any(|power| owner.powers.value(power) != 0)
    {
        return Err(EngineRefusal::MalformedArgs(
            "possess_speed owner block modifier",
        ));
    }
    let debit = owner
        .possess_speed_debit
        .checked_sub(steal)
        .ok_or(EngineRefusal::CounterOverflow("Possess Speed debit"))?;
    // Hook::ModifyBlock: additive owner Dexterity (pre-gain), floored at 0.
    // `possess_roster_is_valid` already refuses negative Forgotten
    // Dexterity, so the floor is the literal port, not a reachable branch.
    let block = block
        .checked_add(owner.powers.value(PowerId::Dexterity))
        .ok_or(EngineRefusal::CounterOverflow("Possess Speed block"))?
        .max(0);
    let owner_block = owner
        .block
        .checked_add(block)
        .ok_or(EngineRefusal::CounterOverflow("Possess Speed block"))?;
    let owner_dexterity = owner
        .powers
        .value(PowerId::Dexterity)
        .checked_add(steal)
        .ok_or(EngineRefusal::CounterOverflow("Possess Speed Dexterity"))?;
    let uid = owner.uid;

    ctx.state
        .powers
        .set(PowerId::Dexterity, SlotWire::Int, player_dexterity);
    note_power(
        ctx.events,
        Subject::Player,
        PowerId::Dexterity,
        player_dexterity,
    );
    let owner = &mut ctx.state.monsters_mut()[ctx.actor];
    owner.possess_speed_debit = debit;
    owner.block = owner_block;
    owner
        .powers
        .set(PowerId::Dexterity, SlotWire::Int, owner_dexterity);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Dexterity,
        owner_dexterity,
    );
    Ok(())
}

/// `possess_strength` — The Lost DEBILITATING_SMOG.
///
/// Python: `monster_act` (frozen, deleted #2827) — player Strength down through the exact
/// Possess applier, the ledger debit, then owner Strength up.
///
/// The source-specific negative `StrengthPower` result is recorded before the
/// later owner buff, so later aggregate changes can never be mistaken for the
/// exact debit restored on owner death.
pub(crate) fn possess_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(steal)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("possess_strength"));
    };
    // `TheLost::get_DebilitatingSmogStrengthStealAmount` RVA `0xc1aa9`,
    // (9, 2, 2) (#2828).
    if *steal != ctx.tier(mc::THE_LOST_DEBILITATING_SMOG_STRENGTH_STEAL_AMOUNT) {
        return Err(EngineRefusal::MalformedArgs("possess_strength row"));
    }
    crate::engine::monsters::require_possess_roster(ctx.state, ctx.actor, MonsterKind::TheLost)?;
    if ctx.state.history.over || ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    let steal: i32 = (*steal)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("possess_strength amount"))?;
    let player_strength = ctx
        .state
        .powers
        .value(PowerId::Strength)
        .checked_sub(steal)
        .ok_or(EngineRefusal::CounterOverflow("player Strength"))?;
    let owner = &ctx.state.monsters[ctx.actor];
    let debit = owner
        .possess_strength_debit
        .checked_sub(steal)
        .ok_or(EngineRefusal::CounterOverflow("Possess Strength debit"))?;
    let owner_strength = crate::engine::damage::checked_monster_strength_successor(
        owner,
        steal,
        "Possess Strength owner",
    )?;
    let uid = owner.uid;

    ctx.state
        .powers
        .set(PowerId::Strength, SlotWire::Int, player_strength);
    note_power(
        ctx.events,
        Subject::Player,
        PowerId::Strength,
        player_strength,
    );
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let owner = &mut ctx.state.monsters_mut()[ctx.actor];
    owner.possess_strength_debit = debit;
    crate::engine::damage::write_monster_self_strength(owner, owner_strength, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Strength,
        owner_strength,
    );
    Ok(())
}

/// `soar` — Owl Magistrate JUDICIAL_FLIGHT.
///
/// Python: `monster_act` (frozen, deleted #2827) — set the flying flag, no attack.
///
/// The bool power is both the canonical flying flag and the owner-local
/// SoarPower instance. [`crate::engine::damage::player_attack`] reads it in the
/// common multiplicative fold and halves only powered incoming attacks.
pub(crate) fn soar(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("soar"));
    };
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::OwlMagistrate {
        return Err(EngineRefusal::MalformedArgs("soar owner"));
    }
    if ctx.state.history.over || ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    monster.powers.set(PowerId::Soar, SlotWire::Bool, 1);
    note_power(ctx.events, Subject::Monster(monster.uid), PowerId::Soar, 1);
    Ok(())
}

/// `summon_illusion` — The Obscura / Fogmog opener.
///
/// Python: `monster_act` (frozen, deleted #2827) — `_spawn_illusion` adds the illusions.
///
/// The generated owner/illusion pair is authenticated before one fixed-range
/// Niche draw and a slot-0/next-uid insertion. The kind-derived secondary
/// keeps its retained corpse and spends its next action fully healing through
/// the shared illusion revive machine.
pub(crate) fn summon_illusion(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::Monster(kind), CompiledArg::I(hp)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("summon_illusion"));
    };
    let uid = ctx.state.monsters[ctx.actor].uid;
    let hp: i32 = (*hp)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("summon_illusion HP"))?;
    crate::engine::monsters::spawn_illusion(ctx.state, uid, *kind, hp)
}

/// `summon_rat` — Two-Tailed Rat CALL_FOR_BACKUP.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_spawn_rat`. The move adds
/// one rat through the exact five-slot roster lifecycle: last free rat slot,
/// one unique-HP Niche draw, fresh creation uid, one immediate MonsterAi
/// starter roll, then the all-rat CallForBackupCount synchronization. A full
/// roster is an exact zero-mutation command.
pub(crate) fn summon_rat(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() || ctx.state.monsters[ctx.actor].kind != MonsterKind::TwoTailedRat {
        return Err(EngineRefusal::MalformedArgs("summon_rat owner/row"));
    }
    let owner_uid = ctx.state.monsters[ctx.actor].uid;
    crate::engine::monsters::spawn_rat(ctx.state, owner_uid)
}

/// `verdict` — Owl Magistrate VERDICT.
///
/// Python: `monster_act` (frozen, deleted #2827) — clear the flying flag, attack, then player
/// Vulnerable with the fresh latch and Soar removed.
///
/// The canonical flying flag is cleared before the attack. A surviving actor
/// then applies fresh Vulnerable 4 and publishes the Soar removal; a terminal
/// attack skips both suffix events while retaining the already-cleared flag.
/// The damage is `OwlMagistrate::get_VerdictDamage` RVA `0xbaed0`,
/// `GetValueIfAscension(9, 36, 33)` at the fight's tier (#2828).
pub(crate) fn verdict(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(4)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("verdict"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::OWL_MAGISTRATE_VERDICT_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("verdict"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::OwlMagistrate {
        return Err(EngineRefusal::MalformedArgs("verdict owner"));
    }
    if ctx.state.history.over || ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    ctx.state.monsters_mut()[ctx.actor]
        .powers
        .set(PowerId::Soar, SlotWire::Bool, 0);
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerVuln,
            PowerId::PlayerVulnFresh,
            4,
            true,
            ctx.events,
        )?;
        note_power(
            ctx.events,
            Subject::Monster(ctx.state.monsters[ctx.actor].uid),
            PowerId::Soar,
            0,
        );
    }
    Ok(())
}

/// `("vuln_player", amount)` — fresh enemy-applied player Vulnerable.
pub(crate) fn vuln_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("vuln_player"));
    };
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("vuln_player Vulnerable"))?;
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("vuln_player"));
    }
    if !matches!(
        (ctx.state.monsters[ctx.actor].kind, amount),
        (crate::ids::MonsterKind::Flyconid, 2) | (crate::ids::MonsterKind::Mawler, 3)
    ) {
        return Err(EngineRefusal::MalformedArgs("vuln_player owner"));
    }
    apply_player_duration_affliction(
        ctx.state,
        PowerId::PlayerVuln,
        PowerId::PlayerVulnFresh,
        amount,
        true,
        ctx.events,
    )
}

/// `("weak_player", amount)` — fresh enemy-applied player Weak.
pub(crate) fn weak_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("weak_player"));
    };
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::BowlbugSilk {
        return Err(EngineRefusal::MalformedArgs("weak_player owner"));
    }
    apply_player_duration_affliction(
        ctx.state,
        PowerId::PlayerWeak,
        PowerId::PlayerWeakFresh,
        1,
        true,
        ctx.events,
    )
}

/// `("weak_player_strength", weak, strength)` — fresh player Weak, then
/// Strength on the live owner.
pub(crate) fn weak_player_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(3), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("weak_player_strength"));
    };
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::SlimedBerserker {
        return Err(EngineRefusal::MalformedArgs("weak_player_strength owner"));
    }
    let updated_strength = if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        Some(crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[ctx.actor],
            3,
            "weak_player_strength monster Strength",
        )?)
    } else {
        None
    };
    apply_player_duration_affliction(
        ctx.state,
        PowerId::PlayerWeak,
        PowerId::PlayerWeakFresh,
        3,
        true,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let updated = updated_strength.ok_or(EngineRefusal::MalformedArgs(
            "weak_player_strength Strength plan",
        ))?;
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        crate::engine::damage::write_monster_self_strength(monster, updated, upkeep);
        note_power(
            ctx.events,
            Subject::Monster(monster.uid),
            PowerId::Strength,
            updated,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CatalogBuilder, CompiledMove};
    use crate::engine::admission::{AdmissionRefusal, MissingCapability, admit};
    use crate::ids::{CardId, MonsterKind};
    use serde_json::Value;

    /// The R0.5 entry document every test here starts from.
    const FIXTURE: &str = include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    /// Admit the fixture with its single monster swapped for `kind`.
    ///
    /// The measurement the doc comments above quote: the entry-level question
    /// an operator holding that encounter would actually read, rather than a
    /// peek at one internal predicate.
    fn admit_with_monster(kind: MonsterKind) -> Result<(), AdmissionRefusal> {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert("kind".to_owned(), Value::from(kind.as_str()));
        // Flyconid became an admitted random-AI machine in #2481 slot 3, so the
        // generic singleton swap now has to hand it a live machine: its steady
        // RAND is reached from any concrete state, and SMASH's cooldown is the
        // shortest, so a one-entry SMASH log is the smallest legal projection.
        if kind == MonsterKind::Flyconid {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("SMASH_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("SMASH_MOVE")]),
            );
        }
        if kind == MonsterKind::TwoTailedRat {
            let template = document.monsters[0].clone();
            document.monsters.clear();
            for (position, (hp, starter)) in
                [(18, "SCRATCH"), (19, "DISEASE_BITE"), (20, "SCREECH")]
                    .into_iter()
                    .enumerate()
            {
                let mut rat = template.clone();
                rat.insert("hp".to_owned(), Value::from(hp));
                rat.insert("max_hp".to_owned(), Value::from(hp));
                rat.remove("loop_pos");
                rat.insert("next_move".to_owned(), Value::from(starter));
                rat.insert(
                    "move_log".to_owned(),
                    Value::Array(vec![Value::from(starter)]),
                );
                rat.insert("tus".to_owned(), Value::from(2));
                if position > 0 {
                    rat.insert("slot".to_owned(), Value::from(position as i64));
                    rat.insert("uid".to_owned(), Value::from(position as i64));
                }
                document.monsters.push(rat);
            }
        }
        if matches!(kind, MonsterKind::TheLost | MonsterKind::TheForgotten) {
            let mut lost = document.monsters[0].clone();
            lost.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::TheLost.as_str()),
            );
            lost.insert(
                "hp".to_owned(),
                Value::from(crate::engine::monsters::THE_LOST_HP),
            );
            lost.insert(
                "max_hp".to_owned(),
                Value::from(crate::engine::monsters::THE_LOST_HP),
            );
            lost.remove("slot");
            lost.remove("uid");
            lost.remove("loop_pos");
            let mut forgotten = lost.clone();
            forgotten.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::TheForgotten.as_str()),
            );
            forgotten.insert(
                "hp".to_owned(),
                Value::from(crate::engine::monsters::THE_FORGOTTEN_HP),
            );
            forgotten.insert(
                "max_hp".to_owned(),
                Value::from(crate::engine::monsters::THE_FORGOTTEN_HP),
            );
            forgotten.insert("slot".to_owned(), Value::from(1));
            forgotten.insert("uid".to_owned(), Value::from(1));
            document.monsters = vec![lost, forgotten];
        }
        if matches!(kind, MonsterKind::Ovicopter | MonsterKind::ToughEgg) {
            let mut parent = document.monsters[0].clone();
            parent.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::Ovicopter.as_str()),
            );
            parent.insert("slot".to_owned(), Value::from(5));
            parent.remove("uid");
            parent.remove("loop_pos");
            document.monsters.clear();
            if kind == MonsterKind::ToughEgg {
                let mut egg = parent.clone();
                egg.insert(
                    "kind".to_owned(),
                    Value::from(MonsterKind::ToughEgg.as_str()),
                );
                egg.insert("hp".to_owned(), Value::from(15));
                egg.insert("max_hp".to_owned(), Value::from(15));
                egg.insert("slot".to_owned(), Value::from(4));
                egg.insert("uid".to_owned(), Value::from(1));
                egg.insert(PowerId::HatchPower.as_str().to_owned(), Value::from(2));
                egg.insert(PowerId::Secondary.as_str().to_owned(), Value::from(true));
                document.monsters.push(egg);
            }
            document.monsters.push(parent);
        }
        if kind == MonsterKind::Mawler {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("CLAW_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("CLAW_MOVE")]),
            );
        }
        if kind == MonsterKind::TheObscura {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("ILLUSION_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("ILLUSION_MOVE")]),
            );
        }
        if kind == MonsterKind::Fabricator {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("FABRICATE_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("FABRICATE_MOVE")]),
            );
        }
        if kind == MonsterKind::FrogKnight {
            document.monsters.truncate(1);
            document.monsters[0].insert("hp".to_owned(), Value::from(199));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(199));
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert(PowerId::Mplating.as_str().to_owned(), Value::from(19));
        }
        if kind == MonsterKind::Axebot {
            document.monsters.truncate(1);
            document.monsters[0].insert("hp".to_owned(), Value::from(76));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(76));
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert(PowerId::Stock.as_str().to_owned(), Value::from(2));
        }
        if kind == MonsterKind::GremlinMerc {
            document.monsters.truncate(1);
            document.monsters[0].insert("hp".to_owned(), Value::from(51));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(51));
            document.monsters[0].remove("loop_pos");
        }
        if kind == MonsterKind::FatGremlin {
            let mut merc = document.monsters[0].clone();
            merc.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::GremlinMerc.as_str()),
            );
            merc.insert("hp".to_owned(), Value::from(0));
            merc.insert("max_hp".to_owned(), Value::from(51));
            merc.remove("loop_pos");

            let mut sneaky = merc.clone();
            sneaky.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::SneakyGremlin.as_str()),
            );
            sneaky.insert("hp".to_owned(), Value::from(11));
            sneaky.insert("max_hp".to_owned(), Value::from(11));
            sneaky.insert("slot".to_owned(), Value::from(1));
            sneaky.insert("uid".to_owned(), Value::from(2));
            sneaky.insert("spawn_noop".to_owned(), Value::from(true));
            sneaky.remove("loop_pos");

            let mut fat = sneaky.clone();
            fat.insert(
                "kind".to_owned(),
                Value::from(MonsterKind::FatGremlin.as_str()),
            );
            fat.insert("hp".to_owned(), Value::from(14));
            fat.insert("max_hp".to_owned(), Value::from(14));
            fat.insert("slot".to_owned(), Value::from(2));
            fat.insert("uid".to_owned(), Value::from(1));

            document.monsters = vec![merc, sneaky, fat];
            document.player.insert(
                "gremlin_merc_gold_proportion_halves".to_owned(),
                Value::from(2),
            );
        }
        if matches!(
            kind,
            MonsterKind::Guardbot
                | MonsterKind::Noisebot
                | MonsterKind::Stabbot
                | MonsterKind::Zapbot
        ) {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert(PowerId::Secondary.as_str().to_owned(), Value::from(true));
            if kind == MonsterKind::Zapbot {
                document.monsters[0]
                    .insert(PowerId::HighVoltage.as_str().to_owned(), Value::from(2));
            }
        }
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog)
    }

    fn state_for(monster: MonsterKind) -> (CanonicalStateV2, crate::catalog::Catalog) {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters.truncate(1);
        document.monsters[0].insert("kind".to_owned(), Value::from(monster.as_str()));
        document.monsters[0].remove("loop_pos");
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        (document, catalog)
    }

    fn row(
        catalog: &crate::catalog::Catalog,
        monster: MonsterKind,
        kind: MoveKind,
    ) -> CompiledMove {
        catalog
            .moves(monster)
            .iter()
            .find(|entry| entry.kind == kind)
            .copied()
            .expect("the native monster loop carries this row")
    }

    fn possess_state() -> (crate::hot::HotState, crate::catalog::Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TheLost).unwrap();
        builder.intern_monster(MonsterKind::TheForgotten).unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 80;
        let mut lost =
            crate::hot::HotMonster::new(MonsterKind::TheLost, crate::engine::monsters::THE_LOST_HP);
        lost.max_hp = crate::engine::monsters::THE_LOST_HP;
        let mut forgotten = crate::hot::HotMonster::new(
            MonsterKind::TheForgotten,
            crate::engine::monsters::THE_FORGOTTEN_HP,
        );
        forgotten.max_hp = crate::engine::monsters::THE_FORGOTTEN_HP;
        forgotten.slot = 1;
        forgotten.uid = 1;
        state.monsters_mut().extend([lost, forgotten]);
        (state, catalog)
    }

    fn fabricator_state(intent: u8) -> (crate::hot::HotState, crate::catalog::Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Fabricator).unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 200;
        state.max_hp = 200;
        let mut owner = crate::hot::HotMonster::new(MonsterKind::Fabricator, 155);
        owner.max_hp = 155;
        owner.slot = 2;
        assert!(owner.random_ai.set_next(Some(intent)));
        assert!(owner.random_ai.set_log(&[intent]));
        state.monsters_mut().push(owner);
        (state, catalog)
    }

    fn batch_j_state(
        kind: MonsterKind,
        loop_pos: i32,
    ) -> (crate::hot::HotState, crate::catalog::Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(kind).unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let hp = if kind == MonsterKind::FrogKnight {
            199
        } else {
            86
        };
        let mut owner = crate::hot::HotMonster::new(kind, hp);
        owner.max_hp = hp;
        owner.loop_pos = loop_pos;
        state.monsters_mut().push(owner);
        (state, catalog)
    }

    fn apply_batch_j(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        kind: MoveKind,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let entry = row(catalog, state.monsters[0].kind, kind);
        let mut ctx = MoveCtx {
            state,
            catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events,
        };
        match kind {
            MoveKind::AttackCharge => attack_charge(&mut ctx),
            MoveKind::AxebotBootup => axebot_bootup(&mut ctx),
            _ => unreachable!(),
        }
    }

    #[test]
    fn batch_j_has_exactly_two_generated_rows() {
        const BATCH_J: &[MoveKind] = &[MoveKind::AttackCharge, MoveKind::AxebotBootup];
        let rows: Vec<(MonsterKind, MoveKind)> = owned_rows()
            .into_iter()
            .filter_map(|(monster, row)| BATCH_J.contains(&row.kind).then_some((monster, row.kind)))
            .collect();
        assert_eq!(
            rows,
            [
                (MonsterKind::Axebot, MoveKind::AxebotBootup),
                (MonsterKind::FrogKnight, MoveKind::AttackCharge),
            ]
        );
        assert!(BATCH_J.iter().all(|kind| IMPLEMENTED.contains(kind)));
        assert_eq!(admit_with_monster(MonsterKind::FrogKnight), Ok(()));
        assert_eq!(admit_with_monster(MonsterKind::Axebot), Ok(()));
    }

    #[test]
    fn frog_charge_latches_before_normal_and_lethal_damage() {
        for hp in [100, 1] {
            let (mut state, catalog) = batch_j_state(MonsterKind::FrogKnight, 3);
            state.hp = hp;
            state.max_hp = hp;
            apply_batch_j(
                &mut state,
                &catalog,
                MoveKind::AttackCharge,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(state.monsters[0].louse_curled());
            assert_eq!(state.hp, (hp - 40).max(0));
            assert_eq!(state.history.over, hp <= 40);
        }

        let (mut malformed, catalog) = batch_j_state(MonsterKind::FrogKnight, 3);
        malformed.monsters_mut()[0].set_louse_curled(true);
        let before = malformed.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_batch_j(
                &mut malformed,
                &catalog,
                MoveKind::AttackCharge,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("attack_charge owner/state"))
        );
        assert_eq!(malformed, before);
        assert!(events.is_empty());
    }

    #[test]
    fn axebot_bootup_reads_live_stock_and_refuses_unrespawned_stock() {
        for (stock, expected_strength) in [(1, 4), (0, 8)] {
            let (mut state, catalog) = batch_j_state(MonsterKind::Axebot, 2);
            if stock > 0 {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Stock, SlotWire::Int, stock);
            }
            let mut events = Vec::new();
            apply_batch_j(&mut state, &catalog, MoveKind::AxebotBootup, &mut events).unwrap();
            assert_eq!(state.monsters[0].block, 15);
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                expected_strength
            );
            assert!(events.contains(&crate::engine::Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Strength,
                amount: expected_strength,
            }));
        }

        let (mut malformed, catalog) = batch_j_state(MonsterKind::Axebot, 2);
        malformed.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, 2);
        let before = malformed.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_batch_j(
                &mut malformed,
                &catalog,
                MoveKind::AxebotBootup,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("axebot_bootup owner/state"))
        );
        assert_eq!(malformed, before);
        assert!(events.is_empty());
    }

    fn apply_louse(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        kind: MoveKind,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let entry = row(catalog, MonsterKind::LouseProgenitor, kind);
        let mut ctx = MoveCtx {
            state,
            catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events,
        };
        match kind {
            MoveKind::LouseCurlGrow => louse_curl_grow(&mut ctx),
            MoveKind::LousePounce => louse_pounce(&mut ctx),
            MoveKind::LouseWeb => louse_web(&mut ctx),
            _ => unreachable!(),
        }
    }

    #[test]
    fn louse_batch_has_exactly_three_rows_and_one_new_admission() {
        const BATCH_H: &[MoveKind] = &[
            MoveKind::LouseCurlGrow,
            MoveKind::LousePounce,
            MoveKind::LouseWeb,
        ];
        let rows: Vec<(MonsterKind, MoveKind)> = owned_rows()
            .into_iter()
            .filter_map(|(monster, row)| BATCH_H.contains(&row.kind).then_some((monster, row.kind)))
            .collect();
        assert_eq!(
            rows,
            [
                (MonsterKind::LouseProgenitor, MoveKind::LouseWeb),
                (MonsterKind::LouseProgenitor, MoveKind::LouseCurlGrow),
                (MonsterKind::LouseProgenitor, MoveKind::LousePounce),
            ]
        );
        assert!(BATCH_H.iter().all(|kind| IMPLEMENTED.contains(kind)));
        assert_eq!(admit_with_monster(MonsterKind::LouseProgenitor), Ok(()));
    }

    #[test]
    fn louse_three_move_cycle_preserves_native_order_and_lethal_short_circuit() {
        let (document, catalog) = state_for(MonsterKind::LouseProgenitor);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        state.max_hp = 100;
        let mut events = Vec::new();

        apply_louse(&mut state, &catalog, MoveKind::LouseCurlGrow, &mut events).unwrap();
        assert_eq!(state.monsters[0].block, 18);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 7);
        assert!(state.monsters[0].louse_curled());

        apply_louse(&mut state, &catalog, MoveKind::LouseWeb, &mut events).unwrap();
        assert_eq!(state.hp, 83, "Web reads the Strength gained by Curl/Grow");
        assert!(!state.monsters[0].louse_curled());
        assert_eq!(state.powers.value(PowerId::PlayerFrail), 2);
        assert_eq!(state.powers.value(PowerId::PlayerFrailFresh), 1);

        state.monsters_mut()[0].set_louse_curled(true);
        apply_louse(&mut state, &catalog, MoveKind::LousePounce, &mut events).unwrap();
        assert_eq!(state.hp, 60, "Pounce reads the same live Strength");
        assert!(!state.monsters[0].louse_curled());

        let mut lethal = HotBoundary::from_canonical(&document, &catalog).unwrap();
        lethal.hp = 1;
        lethal.max_hp = 1;
        lethal.monsters_mut()[0].set_louse_curled(true);
        apply_louse(&mut lethal, &catalog, MoveKind::LouseWeb, &mut Vec::new()).unwrap();
        assert!(lethal.history.over);
        assert!(!lethal.monsters[0].louse_curled());
        assert_eq!(lethal.powers.value(PowerId::PlayerFrail), 0);
    }

    fn apply_possess(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        actor: usize,
        kind: MoveKind,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let entry = row(catalog, state.monsters[actor].kind, kind);
        let mut ctx = MoveCtx {
            state,
            catalog,
            actor,
            args: catalog.args(entry.args),
            events,
        };
        match kind {
            MoveKind::PossessStrength => possess_strength(&mut ctx),
            MoveKind::PossessSpeed => possess_speed(&mut ctx),
            _ => unreachable!(),
        }
    }

    #[test]
    fn possess_moves_record_only_the_source_debit_before_owner_results() {
        let (mut state, catalog) = possess_state();
        let mut events = Vec::new();

        apply_possess(
            &mut state,
            &catalog,
            0,
            MoveKind::PossessStrength,
            &mut events,
        )
        .unwrap();
        apply_possess(
            &mut state,
            &catalog,
            0,
            MoveKind::PossessStrength,
            &mut events,
        )
        .unwrap();
        apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events).unwrap();

        assert_eq!(state.powers.value(PowerId::Strength), -4);
        assert_eq!(state.monsters[0].possess_strength_debit, -4);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 4);
        assert_eq!(state.powers.value(PowerId::Dexterity), -2);
        assert_eq!(state.monsters[1].possess_speed_debit, -2);
        assert_eq!(state.monsters[1].block, 8);
        assert_eq!(state.monsters[1].powers.value(PowerId::Dexterity), 2);
        assert_eq!(
            events,
            [
                crate::engine::Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: -2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: -4,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 4,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Dexterity,
                    amount: -2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(1),
                    power: PowerId::Dexterity,
                    amount: 2,
                },
            ]
        );

        let before = state.clone();
        state.monsters_mut()[1].uid = 9;
        assert_eq!(
            apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events,),
            Err(EngineRefusal::MalformedArgs(
                "TheLostAndForgottenNormal roster lifecycle"
            ))
        );
        state.monsters_mut()[1].uid = 1;
        assert_eq!(state, before);

        for (actor, args, body) in [
            (0, vec![CompiledArg::I(3)], MoveKind::PossessStrength),
            (
                1,
                vec![CompiledArg::I(2), CompiledArg::I(9)],
                MoveKind::PossessSpeed,
            ),
        ] {
            let (mut state, catalog) = possess_state();
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor,
                args: &args,
                events: &mut events,
            };
            let result = match body {
                MoveKind::PossessStrength => possess_strength(&mut ctx),
                MoveKind::PossessSpeed => possess_speed(&mut ctx),
                _ => unreachable!(),
            };
            assert!(matches!(result, Err(EngineRefusal::MalformedArgs(_))));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    /// #3214: MIASMA's `GainBlock(8, Move)` reads the owner's PRE-gain
    /// Dexterity through `DexterityPower::ModifyBlockAdditive` (RVA
    /// `0xa17b8`) and `Hook::ModifyBlock` floors at 0 (RVA `0x104c30`,
    /// IL_0100-IL_0106). fd41bbfb8f5a0529: 8 then 8 + 2 = 18 total.
    #[test]
    fn miasma_block_reads_owner_pre_gain_dexterity_and_floors_at_zero() {
        let (mut state, catalog) = possess_state();
        let mut events = Vec::new();
        apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events).unwrap();
        assert_eq!(state.monsters[1].block, 8);
        assert_eq!(state.monsters[1].powers.value(PowerId::Dexterity), 2);
        state.monsters_mut()[1].block = 0;
        apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events).unwrap();
        assert_eq!(state.monsters[1].block, 10);
        assert_eq!(state.monsters[1].powers.value(PowerId::Dexterity), 4);

        // The native floor is unreachable on the admitted roster: negative
        // Forgotten Dexterity already refuses as a roster lifecycle.
        let (mut state, catalog) = possess_state();
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, -11);
        let before = state.clone();
        assert_eq!(
            apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "TheLostAndForgottenNormal roster lifecycle"
            ))
        );
        assert_eq!(state, before);

        // An owner-held Block-modifying listener refuses by name, atomically.
        for power in [PowerId::NoBlock, PowerId::Shadowmeld, PowerId::Fasten] {
            let (mut state, catalog) = possess_state();
            state.monsters_mut()[1].powers.set(power, SlotWire::Int, 1);
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                apply_possess(&mut state, &catalog, 1, MoveKind::PossessSpeed, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "possess_speed owner block modifier"
                ))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn fabricator_moves_pin_spawn_attack_and_noise_command_order() {
        let (mut state, catalog) =
            fabricator_state(crate::engine::monsters::FABRICATOR_FABRICATE_INDEX);
        let entry = row(&catalog, MonsterKind::Fabricator, MoveKind::Fabricate);
        let ai_before = state.rng.get(crate::hot::RngStream::Ai).counter;
        let niche_before = state.rng.get(crate::hot::RngStream::Niche).counter;
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        fabricate(&mut ctx).unwrap();
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Ai).counter,
            ai_before + 2
        );
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Niche).counter,
            niche_before + 2
        );
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::Guardbot, 0, 1),
                (MonsterKind::Zapbot, 1, 2),
                (MonsterKind::Fabricator, 2, 0),
            ]
        );
        assert_eq!(
            state
                .monsters
                .iter()
                .find(|monster| monster.uid == 0)
                .unwrap()
                .last_spawned,
            Some(MonsterKind::Zapbot)
        );

        let (mut strike, catalog) =
            fabricator_state(crate::engine::monsters::FABRICATOR_STRIKE_INDEX);
        let entry = row(
            &catalog,
            MonsterKind::Fabricator,
            MoveKind::AttackSpawnAggro,
        );
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut strike,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        attack_spawn_aggro(&mut ctx).unwrap();
        assert_eq!(strike.hp, 179);
        assert_eq!(strike.monsters.len(), 2);
        assert_eq!(strike.monsters[0].kind, MonsterKind::Zapbot);

        let mut noise = crate::hot::HotState::at_defaults();
        let mut bot = crate::hot::HotMonster::new(MonsterKind::Noisebot, 19);
        bot.max_hp = 19;
        bot.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        noise.monsters_mut().push(bot);
        let entry = row(&catalog, MonsterKind::Noisebot, MoveKind::NoiseStatus);
        let shuffle_before = noise.rng.get(crate::hot::RngStream::Rng).counter;
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut noise,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        noise_status(&mut ctx).unwrap();
        let dazed = catalog
            .atom(&CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        assert_eq!(noise.piles.get(PileId::Discard).as_slice()[0].atom, dazed);
        assert_eq!(noise.piles.get(PileId::Draw).as_slice()[0].atom, dazed);
        assert_eq!(
            noise.rng.get(crate::hot::RngStream::Rng).counter,
            shuffle_before + 1
        );
    }

    #[test]
    fn lethal_fabricating_strike_short_circuits_before_spawn() {
        let (mut state, catalog) =
            fabricator_state(crate::engine::monsters::FABRICATOR_STRIKE_INDEX);
        state.hp = 1;
        let entry = row(
            &catalog,
            MonsterKind::Fabricator,
            MoveKind::AttackSpawnAggro,
        );
        let ai_before = state.rng.get(crate::hot::RngStream::Ai);
        let niche_before = state.rng.get(crate::hot::RngStream::Niche);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        attack_spawn_aggro(&mut ctx).unwrap();
        assert!(state.history.over);
        assert_eq!(state.monsters.len(), 1);
        assert_eq!(state.monsters[0].last_spawned, None);
        assert_eq!(state.rng.get(crate::hot::RngStream::Ai), ai_before);
        assert_eq!(state.rng.get(crate::hot::RngStream::Niche), niche_before);
    }

    #[test]
    fn fabricator_batch_has_exactly_three_generated_carriers_and_operands() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Fabricator).unwrap();
        let catalog = builder.build();
        let mut carriers = Vec::new();
        for monster in [MonsterKind::Fabricator, MonsterKind::Noisebot] {
            for entry in catalog.moves(monster) {
                if matches!(
                    entry.kind,
                    MoveKind::AttackSpawnAggro | MoveKind::Fabricate | MoveKind::NoiseStatus
                ) {
                    carriers.push((monster, entry.kind, catalog.args(entry.args).to_vec()));
                }
            }
        }
        assert_eq!(
            carriers,
            [
                (MonsterKind::Fabricator, MoveKind::Fabricate, vec![]),
                (
                    MonsterKind::Fabricator,
                    MoveKind::AttackSpawnAggro,
                    vec![CompiledArg::I(21), CompiledArg::I(1)],
                ),
                (MonsterKind::Noisebot, MoveKind::NoiseStatus, vec![]),
            ]
        );
        assert!(
            carriers
                .iter()
                .all(|(_, kind, _)| IMPLEMENTED.contains(kind))
        );
    }

    #[test]
    fn fabricator_move_preconditions_refuse_without_mutation() {
        let (mut state, catalog) =
            fabricator_state(crate::engine::monsters::FABRICATOR_STRIKE_INDEX);
        state.monsters_mut()[0].last_spawned = Some(MonsterKind::Guardbot);
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &[CompiledArg::I(21), CompiledArg::I(1)],
            events: &mut events,
        };
        assert_eq!(
            attack_spawn_aggro(&mut ctx),
            Err(EngineRefusal::MalformedArgs("Fabricator bot/AI lifecycle"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// Every compiled loop row in the build that uses one of this family's
    /// kinds, with the monster that carries it.
    fn owned_rows() -> Vec<(MonsterKind, CompiledMove)> {
        let mut builder = CatalogBuilder::new();
        for kind in MonsterKind::ALL {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let mut rows = Vec::new();
        for monster in MonsterKind::ALL {
            for entry in catalog.moves(monster) {
                if OWNED.contains(&entry.kind) {
                    rows.push((monster, *entry));
                }
            }
        }
        // Every kind this file owns is reachable from some monster's move
        // table, or the generator's family split and the content tables
        // disagree. Kinds carried by an unallowlisted random-AI table remain
        // visible through the source-table half of this total census.
        for kind in OWNED {
            let in_loop = rows.iter().any(|(_, entry)| entry.kind == kind);
            let in_random_ai = MonsterKind::ALL.iter().any(|monster| {
                crate::content_tables::random_moves(*monster)
                    .is_some_and(|table| table.iter().any(|entry| entry.kind == kind))
            });
            assert!(
                in_loop || in_random_ai,
                "no monster move table uses {:?}",
                kind.as_str()
            );
        }
        rows
    }

    #[test]
    fn rat_table_has_exactly_four_rows_and_one_new_body() {
        let table = crate::content_tables::random_moves(MonsterKind::TwoTailedRat)
            .expect("generated Two-Tailed Rat table");
        assert_eq!(table.len(), 4);
        assert_eq!(
            table
                .iter()
                .map(|entry| (entry.name, entry.kind, entry.args))
                .collect::<Vec<_>>(),
            [
                (
                    "SCRATCH",
                    MoveKind::Attack,
                    &[
                        crate::content_tables::Arg::Tier(mc::TWO_TAILED_RAT_SCRATCH_DAMAGE),
                        crate::content_tables::Arg::I(1)
                    ][..],
                ),
                (
                    "DISEASE_BITE",
                    MoveKind::Attack,
                    &[
                        crate::content_tables::Arg::Tier(mc::TWO_TAILED_RAT_DISEASE_BITE_DAMAGE),
                        crate::content_tables::Arg::I(1)
                    ][..],
                ),
                (
                    "SCREECH",
                    MoveKind::FrailPlayer,
                    &[crate::content_tables::Arg::I(1)][..],
                ),
                ("CALL_FOR_BACKUP", MoveKind::SummonRat, &[][..]),
            ]
        );
        assert!(crate::moves::shared::IMPLEMENTED.contains(&MoveKind::Attack));
        assert!(crate::moves::shared::IMPLEMENTED.contains(&MoveKind::FrailPlayer));
        assert!(IMPLEMENTED.contains(&MoveKind::SummonRat));
    }

    /// [`OWNED`] is the walk order for the escalation tests below, so it has
    /// to be ascending and duplicate-free for them to be total over it — and
    /// nothing may be claimed in [`IMPLEMENTED`] that this file does not own a
    /// body for.
    #[test]
    fn the_owned_list_is_ascending_and_covers_every_claim() {
        assert!(OWNED.windows(2).all(|pair| pair[0] < pair[1]));
        for kind in IMPLEMENTED {
            assert!(
                OWNED.contains(kind),
                "normal claims {:?}, which is not one of its own stubs",
                kind.as_str()
            );
        }
    }

    /// A kind this family claims must never make its own monster's entry
    /// refuse for the *shape* of the row that kind carries — if it does, the
    /// claim admits nothing and only changes which refusal an operator reads
    /// (#1366: wave kinds' shapes are body-owned, so the entry-level question
    /// is the honest one).
    #[test]
    fn a_claimed_kind_is_never_refused_for_its_argument_shape() {
        for (monster, entry) in owned_rows() {
            if !IMPLEMENTED.contains(&entry.kind) {
                continue;
            }
            // Hopper's three newly claimed rows deliberately require the
            // cold DeckVersion sidecar and exact EA/Flutter cadence; the
            // generic singleton swap cannot construct that public root.
            // Dedicated Hopper tests exercise those rows below.
            if monster == MonsterKind::ThievingHopper {
                continue;
            }
            let Err(refusal) = admit_with_monster(monster) else {
                continue;
            };
            assert!(
                !refusal
                    .missing()
                    .any(|item| matches!(item, MissingCapability::ArgumentShape(_))),
                "normal claims {:?}, but {:?}'s entry refuses on an argument \
                 shape ({refusal}), so every entry carrying it refuses instead \
                 of playing",
                entry.kind.as_str(),
                monster.as_str()
            );
        }
    }

    /// The escalation record, measured rather than narrated: for every kind
    /// this family has NOT claimed, the entry-level refusal for its own
    /// monster must name it. Prose rots; this cannot.
    #[test]
    fn the_gate_names_every_escalated_kind_on_its_own_monster() {
        let rows = owned_rows();
        let mut measured: Vec<MoveKind> = Vec::new();
        for (monster, entry) in rows.iter().copied() {
            if IMPLEMENTED.contains(&entry.kind) {
                continue;
            }
            measured.push(entry.kind);
            let refusal = admit_with_monster(monster).expect_err(
                "an unclaimed kind cannot be admitted: the gate walks every \
                 loop entry of every monster in the entry",
            );
            assert!(
                refusal.contains(MissingCapability::MoveKind(entry.kind)),
                "{:?} carries {:?}, which this family has not claimed, so the \
                 gate must name it; it said {refusal}",
                monster.as_str(),
                entry.kind.as_str()
            );
        }
        // Total over the family, not merely non-vacuous.
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            assert!(
                measured.contains(&kind) || !rows.iter().any(|(_, entry)| entry.kind == kind),
                "{:?} sits in a compiled loop but was not measured",
                kind.as_str()
            );
        }
    }

    /// Two of this family's escalations have a second half the walk above
    /// cannot see: they are unreachable regardless of this file, because their
    /// only carriers are random-AI machines the catalog never compiles a loop
    /// for. The gate refuses those monsters before it reads any move table.
    #[test]
    fn a_random_ai_only_kind_is_refused_before_its_table_is_read() {
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let carriers: Vec<MonsterKind> = MonsterKind::ALL
                .into_iter()
                .filter(|monster| {
                    crate::content_tables::random_moves(*monster)
                        .is_some_and(|table| table.iter().any(|entry| entry.kind == kind))
                })
                .collect();
            assert!(
                !carriers.is_empty() || owned_rows().iter().any(|(_, entry)| entry.kind == kind),
                "{:?} is in no move table at all",
                kind.as_str()
            );
            for monster in carriers {
                let refusal =
                    admit_with_monster(monster).expect_err("a random-AI monster is never admitted");
                assert!(
                    refusal.contains(MissingCapability::MonsterAi(monster)),
                    "{:?} reaches {:?} only through its random-AI table, so the \
                     gate must refuse on the AI; it said {refusal}",
                    monster.as_str(),
                    kind.as_str()
                );
            }
        }
    }

    /// An unclaimed kind's body must actually refuse, by its own name. The
    /// manifest is derived from [`IMPLEMENTED`] (D6), so a body filled without
    /// being listed would be unreachable *and* invisible, and one refusing
    /// under a different kind would misdirect the operator reading it.
    #[test]
    fn every_unclaimed_kind_refuses_by_its_own_name() {
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let mut events = Vec::new();

        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &[],
                events: &mut events,
            };
            assert_eq!(
                super::super::apply_move(kind, &mut ctx),
                Err(EngineRefusal::MoveKindNotModeled(kind)),
                "{:?} is unclaimed, so its body must refuse under its own kind",
                kind.as_str()
            );
        }
        // A refusing dispatch touches nothing.
        assert!(events.is_empty());
    }

    #[test]
    fn real_normal_affliction_rows_write_their_exact_state() {
        for (monster, kind, power, amount) in [
            (
                MonsterKind::LivingFog,
                MoveKind::AttackSmoggy,
                PowerId::Smoggy,
                1,
            ),
            (
                MonsterKind::BowlbugSilk,
                MoveKind::WeakPlayer,
                PowerId::PlayerWeak,
                1,
            ),
            (
                MonsterKind::SlimedBerserker,
                MoveKind::WeakPlayerStrength,
                PowerId::PlayerWeak,
                3,
            ),
        ] {
            let (document, catalog) = state_for(monster);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            let entry = row(&catalog, monster, kind);
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut events,
            };
            super::super::apply_move(kind, &mut ctx).unwrap();
            assert_eq!(state.powers.value(power), amount);
            let fresh = if power == PowerId::Smoggy {
                PowerId::SmoggyFresh
            } else {
                PowerId::PlayerWeakFresh
            };
            assert_eq!(state.powers.value(fresh), 1);
            if monster == MonsterKind::SlimedBerserker {
                assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 3);
            }
        }
    }

    #[test]
    fn lifecycle_batch_has_exactly_six_generated_carriers() {
        const BATCH: &[MoveKind] = &[
            MoveKind::AttackVulnerableDeferredOvicopter,
            MoveKind::Bloat,
            MoveKind::HatchToughEgg,
            MoveKind::LayToughEggs,
            MoveKind::SummonIllusion,
        ];
        let rows: Vec<(MonsterKind, MoveKind)> = MonsterKind::ALL
            .into_iter()
            .flat_map(|monster| {
                crate::content_tables::monster_loop(monster)
                    .or_else(|| crate::content_tables::random_moves(monster))
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(move |entry| {
                        BATCH.contains(&entry.kind).then_some((monster, entry.kind))
                    })
            })
            .collect();
        assert_eq!(
            rows,
            [
                (MonsterKind::Fogmog, MoveKind::SummonIllusion,),
                (MonsterKind::LivingFog, MoveKind::Bloat),
                (MonsterKind::Ovicopter, MoveKind::LayToughEggs,),
                (
                    MonsterKind::Ovicopter,
                    MoveKind::AttackVulnerableDeferredOvicopter,
                ),
                (MonsterKind::TheObscura, MoveKind::SummonIllusion,),
                (MonsterKind::ToughEgg, MoveKind::HatchToughEgg),
            ]
        );
        assert!(BATCH.iter().all(|kind| IMPLEMENTED.contains(kind)));
    }

    #[test]
    fn deterministic_branch_batch_has_two_rows_and_two_new_admissions() {
        const BATCH: &[MoveKind] = &[MoveKind::AttackScrollBranch, MoveKind::AttackStrengthBranch];
        let rows: Vec<(MonsterKind, MoveKind)> = owned_rows()
            .into_iter()
            .filter_map(|(monster, row)| BATCH.contains(&row.kind).then_some((monster, row.kind)))
            .collect();
        assert_eq!(
            rows,
            [
                (MonsterKind::Fogmog, MoveKind::AttackStrengthBranch),
                (MonsterKind::ScrollOfBiting, MoveKind::AttackScrollBranch,),
            ]
        );
        assert!(BATCH.iter().all(|kind| IMPLEMENTED.contains(kind)));

        assert_eq!(admit_with_monster(MonsterKind::Fogmog), Ok(()));
        assert_eq!(admit_with_monster(MonsterKind::ScrollOfBiting), Ok(()));
    }

    #[test]
    fn owl_lifecycle_has_exactly_two_rows_and_one_new_admission() {
        const BATCH: &[MoveKind] = &[MoveKind::Soar, MoveKind::Verdict];
        let rows: Vec<(MonsterKind, MoveKind)> = owned_rows()
            .into_iter()
            .filter_map(|(monster, row)| BATCH.contains(&row.kind).then_some((monster, row.kind)))
            .collect();
        assert_eq!(
            rows,
            [
                (MonsterKind::OwlMagistrate, MoveKind::Soar),
                (MonsterKind::OwlMagistrate, MoveKind::Verdict),
            ]
        );
        assert!(BATCH.iter().all(|kind| IMPLEMENTED.contains(kind)));
        assert_eq!(admit_with_monster(MonsterKind::OwlMagistrate), Ok(()));
    }

    #[test]
    fn owl_soar_and_verdict_preserve_power_attack_and_suffix_order() {
        let (document, catalog) = state_for(MonsterKind::OwlMagistrate);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        let mut events = Vec::new();

        let soar_row = row(&catalog, MonsterKind::OwlMagistrate, MoveKind::Soar);
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(soar_row.args),
            events: &mut events,
        };
        soar(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Soar), 1);

        let verdict_row = row(&catalog, MonsterKind::OwlMagistrate, MoveKind::Verdict);
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(verdict_row.args),
            events: &mut events,
        };
        verdict(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Soar), 0);
        assert_eq!(state.hp, 64);
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 4);
        assert_eq!(state.powers.value(PowerId::PlayerVulnFresh), 1);

        let power_events: Vec<(PowerId, i32)> = events
            .iter()
            .filter_map(|event| match event {
                crate::engine::Event::PowerChanged { power, amount, .. } => Some((*power, *amount)),
                _ => None,
            })
            .collect();
        assert_eq!(
            power_events,
            [
                (PowerId::Soar, 1),
                (PowerId::PlayerVuln, 4),
                (PowerId::PlayerVulnFresh, 1),
                (PowerId::Soar, 0),
            ]
        );

        // VERDICT clears Soar before the attack and a lethal attack never
        // reaches the Vulnerable suffix.
        state.history.over = false;
        state.hp = 1;
        state.powers.set(PowerId::PlayerVuln, SlotWire::Int, 0);
        state
            .powers
            .set(PowerId::PlayerVulnFresh, SlotWire::Bool, 0);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Soar, SlotWire::Bool, 1);
        events.clear();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(verdict_row.args),
            events: &mut events,
        };
        verdict(&mut ctx).unwrap();
        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Soar), 0);
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 0);
        assert_eq!(state.powers.value(PowerId::PlayerVulnFresh), 0);
        assert!(!events.iter().any(|event| matches!(
            event,
            crate::engine::Event::PowerChanged {
                power: PowerId::Soar,
                amount: 0,
                ..
            }
        )));
    }

    #[test]
    fn owl_move_owner_and_operand_sensitivity_refuse_before_mutation() {
        let catalog = CatalogBuilder::new().build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state
            .monsters_mut()
            .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 20));
        let before = state.clone();
        let mut events = Vec::new();

        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &[],
            events: &mut events,
        };
        assert_eq!(
            soar(&mut ctx),
            Err(EngineRefusal::MalformedArgs("soar owner"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.monsters_mut()[0].kind = MonsterKind::OwlMagistrate;
        let before = state.clone();
        let wrong = [CompiledArg::I(35), CompiledArg::I(1), CompiledArg::I(4)];
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &wrong,
            events: &mut events,
        };
        assert_eq!(
            verdict(&mut ctx),
            Err(EngineRefusal::MalformedArgs("verdict"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn fogmog_dazed_admits_reachable_dark_embrace_deferred_draw() {
        for active_at_entry in [false, true] {
            let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
            document.monsters[0]
                .insert("kind".to_owned(), Value::from(MonsterKind::Fogmog.as_str()));
            if active_at_entry {
                document
                    .player
                    .insert("dark_embrace".to_owned(), Value::from(1));
                document.player.insert(
                    "after_card_exhausted_power_order".to_owned(),
                    serde_json::json!(["dark_embrace"]),
                );
                document.player.insert(
                    "after_side_turn_end_power_order".to_owned(),
                    serde_json::json!([["dark_embrace", 0]]),
                );
                document.player.insert(
                    "next_after_side_turn_end_power_uid".to_owned(),
                    Value::from(1),
                );
            } else {
                document
                    .player
                    .insert("next_card_uid".to_owned(), Value::from(11));
                let mut dark_embrace = document.piles["hand"][0].clone();
                dark_embrace.id = CardId::DarkEmbrace.as_str().to_owned();
                dark_embrace.upgrade = 0;
                dark_embrace.uid = Some(10);
                document.piles.get_mut("hand").unwrap().push(dark_embrace);
            }
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
        }
    }

    #[test]
    fn lifecycle_move_bodies_preserve_owner_identity_and_spawn_order() {
        let mut builder = CatalogBuilder::new();
        for kind in [
            MonsterKind::LivingFog,
            MonsterKind::Ovicopter,
            MonsterKind::TheObscura,
        ] {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();

        let mut ovicopter = crate::hot::HotState::at_defaults();
        ovicopter.hp = 100;
        let mut parent = crate::hot::HotMonster::new(MonsterKind::Ovicopter, 126);
        parent.slot = 5;
        ovicopter.monsters_mut().push(parent);
        let lay = row(&catalog, MonsterKind::Ovicopter, MoveKind::LayToughEggs);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut ovicopter,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(lay.args),
            events: &mut events,
        };
        lay_tough_eggs(&mut ctx).unwrap();
        assert_eq!(
            ovicopter
                .monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::ToughEgg, 2, 3),
                (MonsterKind::ToughEgg, 3, 2),
                (MonsterKind::ToughEgg, 4, 1),
                (MonsterKind::Ovicopter, 5, 0),
            ]
        );
        let egg = ovicopter
            .monsters_mut()
            .iter_mut()
            .find(|monster| monster.uid == 1)
            .unwrap();
        egg.powers.set(PowerId::HatchPower, SlotWire::Int, 1);
        let egg_index = ovicopter
            .monsters
            .iter()
            .position(|monster| monster.uid == 1)
            .unwrap();
        let hatch = row(&catalog, MonsterKind::ToughEgg, MoveKind::HatchToughEgg);
        let mut ctx = MoveCtx {
            state: &mut ovicopter,
            catalog: &catalog,
            actor: egg_index,
            args: catalog.args(hatch.args),
            events: &mut events,
        };
        hatch_tough_egg(&mut ctx).unwrap();
        let egg = ovicopter
            .monsters
            .iter()
            .find(|monster| monster.uid == 1)
            .unwrap();
        assert!((20..=23).contains(&egg.hp));
        assert_eq!(egg.slot, 4);
        assert_eq!(egg.loop_pos, 1);
        assert_eq!(egg.powers.value(PowerId::IsHatched), 1);

        let parent_index = ovicopter
            .monsters
            .iter()
            .position(|monster| monster.uid == 0)
            .unwrap();
        let tenderizer = row(
            &catalog,
            MonsterKind::Ovicopter,
            MoveKind::AttackVulnerableDeferredOvicopter,
        );
        let mut ctx = MoveCtx {
            state: &mut ovicopter,
            catalog: &catalog,
            actor: parent_index,
            args: catalog.args(tenderizer.args),
            events: &mut events,
        };
        attack_vulnerable_deferred_ovicopter(&mut ctx).unwrap();
        assert_eq!(ovicopter.hp, 92);
        assert_eq!(ovicopter.powers.value(PowerId::PlayerVuln), 2);
        let parent = ovicopter
            .monsters
            .iter()
            .find(|monster| monster.uid == 0)
            .unwrap();
        assert_eq!(
            parent.loop_pos,
            crate::engine::monsters::OVICOPTER_BRANCH_POS
        );

        let mut fog = crate::hot::HotState::at_defaults();
        fog.hp = 100;
        fog.monsters_mut()
            .push(crate::hot::HotMonster::new(MonsterKind::LivingFog, 100));
        let bloat_row = row(&catalog, MonsterKind::LivingFog, MoveKind::Bloat);
        let mut ctx = MoveCtx {
            state: &mut fog,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(bloat_row.args),
            events: &mut events,
        };
        bloat(&mut ctx).unwrap();
        assert_eq!(fog.hp, 94);
        assert_eq!(fog.monsters[0].kind, MonsterKind::LivingFog);
        assert_eq!(fog.monsters[1].kind, MonsterKind::GasBomb);

        let mut obscura = crate::hot::HotState::at_defaults();
        let mut owner = crate::hot::HotMonster::new(MonsterKind::TheObscura, 129);
        owner.slot = 1;
        obscura.monsters_mut().push(owner);
        let summon_args = [
            CompiledArg::Monster(MonsterKind::Parafright),
            CompiledArg::I(21),
        ];
        let mut ctx = MoveCtx {
            state: &mut obscura,
            catalog: &catalog,
            actor: 0,
            args: &summon_args,
            events: &mut events,
        };
        summon_illusion(&mut ctx).unwrap();
        assert_eq!(
            obscura
                .monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid, monster.hp))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::Parafright, 0, 1, 21),
                (MonsterKind::TheObscura, 1, 0, 129),
            ]
        );
    }

    #[test]
    fn vine_tangled_is_unique_and_haunt_adds_exactly_five_dazed() {
        let (document, catalog) = state_for(MonsterKind::VineShambler);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let entry = row(&catalog, MonsterKind::VineShambler, MoveKind::AttackTangle);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        attack_tangle(&mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::Tangled), 1);
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        assert_eq!(
            attack_tangle(&mut ctx),
            Err(EngineRefusal::PowerRestackNotModeled(PowerId::Tangled))
        );

        let (document, catalog) = state_for(MonsterKind::HauntedShip);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let entry = row(&catalog, MonsterKind::HauntedShip, MoveKind::Haunt);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        haunt(&mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::PlayerWeak), 3);
        assert_eq!(state.powers.value(PowerId::PlayerWeakFresh), 1);
        let dazed = catalog
            .atom(&CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            })
            .expect("Haunt pre-interns its implicit leaf");
        assert_eq!(state.piles.get(PileId::Discard).len(), 5);
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .all(|card| card.atom == dazed)
        );
    }

    #[test]
    fn random_ai_vulnerable_body_and_its_complete_mawler_carrier_are_exact() {
        let (document, catalog) = state_for(MonsterKind::Mawler);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let args = [CompiledArg::I(3)];
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        vuln_player(&mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 3);
        assert_eq!(state.powers.value(PowerId::PlayerVulnFresh), 1);
        assert_eq!(admit_with_monster(MonsterKind::Mawler), Ok(()));
    }

    /// Mechanical sweep of every complete loop touched by this family's
    /// claims. Slice 6 adds Slimed Berserker to the two prior `pile_inject`
    /// completions; its encounter remains independently gated by Slimed's
    /// card-keyword lifecycle.
    #[test]
    fn the_claims_complete_exactly_three_monster_loops() {
        let mut builder = CatalogBuilder::new();
        for kind in MonsterKind::ALL {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let mut completed: Vec<&'static str> = Vec::new();
        for monster in MonsterKind::ALL {
            let moves = catalog.moves(monster);
            if moves.is_empty() {
                continue;
            }
            let carries = moves.iter().any(|entry| entry.kind == MoveKind::PileInject);
            let all_implemented = moves
                .iter()
                .all(|entry| super::super::is_implemented(entry.kind));
            if carries && all_implemented {
                completed.push(monster.as_str());
            }
        }
        assert_eq!(completed, ["CHOMPER", "EYE_WITH_TEETH", "SLIMED_BERSERKER"]);
    }

    /// The Chomper row, played: three Dazed appended to the discard end, no
    /// draw touched, and no physical uid allocated (the turn-start
    /// normalization pass owns that).
    #[test]
    fn the_chomper_row_appends_three_dazed_to_the_discard_end() {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert(
            "kind".to_owned(),
            Value::from(MonsterKind::Chomper.as_str()),
        );
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();

        let before_discard = state.piles.get(PileId::Discard).len();
        let before_draw = state.piles.get(PileId::Draw).len();
        let before_uid = state.next_card_uid;
        let entry = catalog
            .moves(MonsterKind::Chomper)
            .iter()
            .find(|entry| entry.kind == MoveKind::PileInject)
            .copied()
            .expect("the Chomper loop carries the inject row");

        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        pile_inject(&mut ctx).expect("the discard/bottom shape is implemented");

        assert_eq!(state.piles.get(PileId::Discard).len(), before_discard + 3);
        assert_eq!(state.piles.get(PileId::Draw).len(), before_draw);
        assert_eq!(state.next_card_uid, before_uid);
        let dazed = catalog
            .atom(&CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            })
            .expect("the inject row's identity is pre-interned");
        for card in state.piles.get(PileId::Discard).as_slice()[before_discard..].iter() {
            assert_eq!(card.atom, dazed);
        }
    }

    /// The hand destination's per-card overflow redirect, and the refusal for
    /// every destination outside the two ported ones. Both start from the real
    /// Chomper row so the identity under test is one the catalog interned.
    #[test]
    fn the_hand_destination_redirects_overflow_and_other_piles_refuse() {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert(
            "kind".to_owned(),
            Value::from(MonsterKind::Chomper.as_str()),
        );
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let entry = catalog
            .moves(MonsterKind::Chomper)
            .iter()
            .find(|entry| entry.kind == MoveKind::PileInject)
            .copied()
            .expect("the Chomper loop carries the inject row");
        let row: Vec<CompiledArg> = catalog.args(entry.args).to_vec();

        let hand = state.piles.get(PileId::Hand).len();
        let discard = state.piles.get(PileId::Discard).len();

        // The third verified Python destination refuses here: it would spend
        // one Shuffle-stream draw per card.
        let mut elsewhere = row.clone();
        elsewhere[2] = CompiledArg::Pile(PileId::Draw);
        elsewhere[3] = CompiledArg::Word(StepWord::Random);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &elsewhere,
            events: &mut events,
        };
        assert_eq!(
            pile_inject(&mut ctx),
            Err(EngineRefusal::MalformedArgs("pile_inject destination"))
        );
        // Nothing moved on the refusal.
        assert_eq!(state.piles.get(PileId::Hand).len(), hand);
        assert_eq!(state.piles.get(PileId::Discard).len(), discard);

        // The hand destination fills the hand and redirects the overflow.
        let room = crate::engine::draw::MAX_CARDS_IN_HAND - hand;
        let mut to_hand = row;
        to_hand[1] = CompiledArg::I((room + 2) as i64);
        to_hand[2] = CompiledArg::Pile(PileId::Hand);
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &to_hand,
            events: &mut events,
        };
        pile_inject(&mut ctx).expect("the hand destination is implemented");
        assert_eq!(
            state.piles.get(PileId::Hand).len(),
            crate::engine::draw::MAX_CARDS_IN_HAND
        );
        assert_eq!(state.piles.get(PileId::Discard).len(), discard + 2);
    }

    #[test]
    fn r2_normal_bodies_pin_hand_dexterity_team_block_and_self_kill() {
        let (document, catalog) = state_for(MonsterKind::Myte);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let hand_before = state.piles.get(PileId::Hand).len();
        let entry = row(&catalog, MonsterKind::Myte, MoveKind::AddStatusHand);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        add_status_hand(&mut ctx).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), hand_before + 2);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .skip(hand_before)
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect::<Vec<_>>(),
            vec![CardId::Toxic; 2]
        );

        let (document, catalog) = state_for(MonsterKind::TheForgotten);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, 4);
        let entry = row(
            &catalog,
            MonsterKind::TheForgotten,
            MoveKind::AttackDexterity,
        );
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        attack_dexterity(&mut ctx).unwrap();
        assert_eq!(state.hp, 81);

        let catalog = CatalogBuilder::new().build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        let mut guard = crate::hot::HotMonster::new(MonsterKind::Guardbot, 20);
        guard.uid = 2;
        let mut bomb = crate::hot::HotMonster::new(MonsterKind::GasBomb, 10);
        bomb.uid = 4;
        bomb.block = 18;
        let mut ally = crate::hot::HotMonster::new(MonsterKind::LivingFog, 20);
        ally.uid = 7;
        state.monsters_mut().extend([guard, bomb, ally]);
        let mut events = Vec::new();
        let args = [CompiledArg::I(15)];
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        // No Fabricator on the side: GUARD_MOVE's `isinst Fabricator` filter
        // leaves no recipient (#3145).
        buff_team_block(&mut ctx).unwrap();
        assert_eq!(
            state.monsters.iter().map(|m| m.block).collect::<Vec<_>>(),
            [0, 18, 0]
        );

        let args = [CompiledArg::I(9), CompiledArg::I(1)];
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &args,
            events: &mut events,
        };
        explode(&mut ctx).unwrap();
        assert_eq!(state.hp, 91);
        assert_eq!(state.monsters[1].hp, -18);
        assert_eq!(state.monsters[1].block, 18);
        assert!(!state.history.over);
    }

    /// #3172: Gas Bomb's Explode self-kill with Gremlin Horn owned.
    /// `GasBomb/<ExplodeMove>d__20::MoveNext` RVA `0x35d1b0` awaits the
    /// attack, then `CreatureCmd.Kill(Creature, false)` (IL_00ce-IL_00d4),
    /// whose AfterDeath walk reaches Gremlin Horn. The move keeps its catalog
    /// through that death, so the Horn grants energy and a card beside the
    /// surviving Guardbot instead of refusing "Gremlin Horn death catalog".
    #[test]
    fn gas_bomb_explode_self_kill_keeps_the_catalog_for_gremlin_horn() {
        use crate::catalog::CardIdentity;
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, PileId};
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: crate::ids::CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state.fanouts.set_gremlin_horn_owned(true);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 0,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.next_card_uid = 1;
        let mut guard = crate::hot::HotMonster::new(MonsterKind::Guardbot, 20);
        guard.uid = 2;
        let mut bomb = crate::hot::HotMonster::new(MonsterKind::GasBomb, 10);
        bomb.uid = 4;
        state.monsters_mut().extend([guard, bomb]);
        let energy = state.energy;
        let args = [CompiledArg::I(9), CompiledArg::I(1)];
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &args,
            events: &mut events,
        };
        explode(&mut ctx).unwrap();
        assert_eq!(state.hp, 91);
        assert!(state.monsters[1].hp <= 0);
        assert!(!state.history.over);
        assert_eq!(state.energy, energy + 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert!(state.piles.get(PileId::Draw).is_empty());
    }

    /// #3145 (census witness `f08d5c27ac8e5859`, step 22): GUARD_MOVE's
    /// `<GuardMove>b__8_0` keeps only `isinst Fabricator` members of
    /// `Enemies`, so the live Fabricator gains 15 Block while the Guardbot
    /// itself, a live Zapbot and a dead Noisebot gain none.
    #[test]
    fn guard_move_blocks_only_the_fabricator() {
        use crate::hot::{HotMonster, HotState};
        let catalog = CatalogBuilder::new().build();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        let mut dead = HotMonster::new(MonsterKind::Noisebot, 0);
        dead.max_hp = 23;
        dead.uid = 1;
        let mut guard = HotMonster::new(MonsterKind::Guardbot, 11);
        guard.max_hp = 21;
        guard.uid = 3;
        let mut zap = HotMonster::new(MonsterKind::Zapbot, 5);
        zap.max_hp = 20;
        zap.slot = 1;
        zap.uid = 2;
        let mut fabricator = HotMonster::new(MonsterKind::Fabricator, 67);
        fabricator.max_hp = 155;
        fabricator.slot = 2;
        fabricator.uid = 0;
        state.monsters_mut().extend([dead, guard, zap, fabricator]);
        let args = [CompiledArg::I(15)];
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &args,
            events: &mut events,
        };
        buff_team_block(&mut ctx).unwrap();
        assert_eq!(
            state.monsters.iter().map(|m| m.block).collect::<Vec<_>>(),
            [0, 0, 0, 15]
        );

        // A dead Fabricator has left `Enemies`: nobody gains Block.
        state.monsters_mut()[3].hp = 0;
        state.monsters_mut()[3].block = 0;
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &args,
            events: &mut events,
        };
        buff_team_block(&mut ctx).unwrap();
        assert!(state.monsters.iter().all(|m| m.block == 0));

        // A Fabricator corpse under a represented keep-on-death veto would
        // still be in `Enemies`; the move refuses by name instead of guessing.
        state.monsters_mut()[3]
            .powers
            .set(PowerId::Adaptable, SlotWire::Int, 1);
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            buff_team_block(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "spawn seam: corpse kept in Enemies by a death veto"
            ))
        );
    }

    /// #2850: `TrackerRubyRaider::GenerateMoveStateMachine` (v111 RVA
    /// `0xc3474`) starts on TRACK_MOVE (IL_0085 `ldloc.1`), sets
    /// TRACK.FollowUpState = HOUNDS (IL_0069–IL_006b) and
    /// HOUNDS.FollowUpState = HOUNDS (IL_0070–IL_0072). So TRACK fires once
    /// (`<TrackMove>d__12::MoveNext` RVA `0x372cec` IL_00a9–IL_00b7:
    /// `Apply<FrailPower>(targets, 2, …)`) and HOUNDS then repeats forever
    /// (`<HoundsMove>d__13::MoveNext` RVA `0x372bd4` IL_0021–IL_0070:
    /// `Attack(HoundsDamage).WithHitCount(HoundsRepeat)`, `get_HoundsDamage`
    /// RVA `0xc3458` = 1, `get_HoundsRepeat` RVA `0xc3463` =
    /// `GetValueIfAscension(9, 9, 8)`). Driven through the real enemy turn so
    /// the generic tail's `Repeats::Fixed(1)` successor is what is measured.
    #[test]
    fn tracker_ruby_raider_tracks_once_then_hounds_repeats() {
        assert_eq!(admit_with_monster(MonsterKind::TrackerRubyRaider), Ok(()));
        for (ascension, hits) in [(10_i64, 9), (8, 8)] {
            let (mut document, _) = state_for(MonsterKind::TrackerRubyRaider);
            document.monsters[0].insert("hp".to_owned(), Value::from(24));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(24));
            if ascension != 10 {
                // A10 is the wire default and must stay omitted.
                document
                    .player
                    .insert("ascension".to_owned(), Value::from(ascension));
            }
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
            state.hp = 200;
            state.max_hp = 200;
            let mut events = Vec::new();

            // Turn 1: TRACK_MOVE — Frail 2, no damage, successor HOUNDS.
            state.block = 0;
            crate::engine::turn::end_player_turn(&mut state, &catalog, &mut events).unwrap();
            assert_eq!(state.hp, 200, "A{ascension}: TRACK deals no damage");
            assert!(
                state.powers.value(PowerId::PlayerFrail) > 0,
                "A{ascension}: TRACK applies Frail"
            );
            assert_eq!(state.monsters[0].loop_pos, 1, "TRACK -> HOUNDS");

            // Turns 2 and 3: HOUNDS_MOVE follows itself.
            for turn in 2..=3 {
                state.block = 0;
                let before = state.hp;
                crate::engine::turn::end_player_turn(&mut state, &catalog, &mut events).unwrap();
                assert_eq!(
                    before - state.hp,
                    hits,
                    "A{ascension} turn {turn}: HOUNDS is 1 x {hits}"
                );
                assert_eq!(state.monsters[0].loop_pos, 1, "HOUNDS -> HOUNDS");
            }
        }
    }
}
