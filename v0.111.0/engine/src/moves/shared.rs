//! Monster-move bodies used by more than one encounter pool — GENERATED ONCE, THEN OWNED BY HAND.
//!
//! PORT_PLAN.md D4. `sim/v0.111.0/engine/tools/generate_content.py`
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
//! # R2 status (#1560): all 19 ported
//!
//! R2 adds the pure scalar, Strength, status-pile, and Headbutt command shapes
//! independently of their carriers' remaining loop machinery. Batch E adds
//! Ritual's owner-scoped application and fresh latch; the enemy-side-end
//! reader lives in `engine::turn`. Inline tests pin every generated carrier
//! and exact operand.

use super::MoveCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::move_constants as mc;
use crate::engine::admission::move_args;
use crate::engine::cards::inject_legacy_bottom;
use crate::engine::damage::{
    apply_player_duration_affliction, monster_attack_player_aeonglass_transient,
    monster_attack_player_dampen_transient, monster_attack_player_hopper_transient,
    monster_attack_player_with_catalog, monster_attack_player_with_catalog_result, note_power,
};
use crate::engine::{EngineRefusal, Subject};
use crate::hot::PileId;
use crate::ids::{CardId, MonsterKind, MoveKind, PowerId};
use crate::powers::SlotWire;

/// The MoveKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// R0.5's three (#1289), moved here unchanged when R0.6 gave the dispatch its
/// generated home. All three are reachable from more than one encounter pool,
/// so the derived split puts them in `shared`.
pub const IMPLEMENTED: &[MoveKind] = &[
    MoveKind::AddStatusDiscard,
    MoveKind::Attack,
    MoveKind::AttackBlock,
    MoveKind::AttackFrail,
    MoveKind::AttackStrength,
    MoveKind::AttackWeakFrail,
    MoveKind::AttackWeakPlayer,
    MoveKind::Block,
    MoveKind::BlockStrength,
    MoveKind::BuffStrength,
    MoveKind::BuffThorns,
    MoveKind::Burrow,
    MoveKind::FrailPlayer,
    MoveKind::Headbutt,
    MoveKind::None,
    MoveKind::Ritual,
    MoveKind::Shrink,
    MoveKind::SpitAttack,
    MoveKind::Wriggle,
];

/// Every kind this file owns, ascending. The generated [`super::FAMILY_OF`]
/// table remains authoritative; this mirror is the test walk order for the
/// family's claim/refusal audit.
#[cfg(test)]
const OWNED: [MoveKind; 19] = [
    MoveKind::AddStatusDiscard,
    MoveKind::Attack,
    MoveKind::AttackBlock,
    MoveKind::AttackFrail,
    MoveKind::AttackStrength,
    MoveKind::AttackWeakFrail,
    MoveKind::AttackWeakPlayer,
    MoveKind::Block,
    MoveKind::BlockStrength,
    MoveKind::BuffStrength,
    MoveKind::BuffThorns,
    MoveKind::Burrow,
    MoveKind::FrailPlayer,
    MoveKind::Headbutt,
    MoveKind::None,
    MoveKind::Ritual,
    MoveKind::Shrink,
    MoveKind::SpitAttack,
    MoveKind::Wriggle,
];

/// `add_status_discard` — append a generated status to Discard/Bottom.
///
/// Python: `monster_act` (frozen, deleted #2827) delegates to `_pile_inject` with the compiled
/// card identity and count. This body performs that exact Discard/Bottom
/// append through the legacy-card normalizer; independent card-keyword and
/// random-AI admission refusals remain visible at entry.
pub(crate) fn add_status_discard(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::List(identity), CompiledArg::I(count)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("add_status_discard"));
    };
    let [CompiledArg::Card(id), CompiledArg::I(upgrade)] = ctx.catalog.args(*identity) else {
        return Err(EngineRefusal::MalformedArgs("add_status_discard identity"));
    };
    let upgrade = u8::try_from(*upgrade)
        .map_err(|_| EngineRefusal::MalformedArgs("add_status_discard upgrade"))?;
    let count = usize::try_from(*count)
        .map_err(|_| EngineRefusal::CounterOverflow("add_status_discard count"))?;
    if *id != CardId::Slimed
        || upgrade != 0
        || !matches!(count, 1 | 2)
        || !matches!(
            ctx.state.monsters[ctx.actor].kind,
            MonsterKind::LeafSlimeM | MonsterKind::LeafSlimeS | MonsterKind::TwigSlimeM
        )
    {
        return Err(EngineRefusal::MalformedArgs("add_status_discard owner/row"));
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
        PileId::Discard,
    )
}

/// `("attack", damage, hits)` — the plain monster attack.
///
/// Python: `monster_act`'s attack branch.
pub(crate) fn attack(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits, _) =
        move_args(MoveKind::Attack, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    if ctx
        .state
        .monsters
        .get(ctx.actor)
        .is_some_and(|monster| monster.kind == MonsterKind::ThievingHopper)
    {
        return monster_attack_player_hopper_transient(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            damage,
            hits,
            ctx.events,
        );
    }
    if ctx.state.card_states.dampen().is_some() {
        return monster_attack_player_dampen_transient(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            damage,
            hits,
            ctx.events,
        );
    }
    if ctx
        .state
        .monsters
        .get(ctx.actor)
        .is_some_and(|monster| monster.kind == MonsterKind::Aeonglass)
    {
        return monster_attack_player_aeonglass_transient(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            damage,
            hits,
            ctx.events,
        );
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, hits, ctx.events)
}

/// `("attack_block", damage, hits, block)` — attack, then gain Block while
/// the actor remains live and combat has not ended.
pub(crate) fn attack_block(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(block),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_block"));
    };
    if *damage < 0 || *hits < 1 || *block < 0 {
        return Err(EngineRefusal::MalformedArgs("attack_block"));
    }
    if ctx.state.card_states.dampen().is_some() {
        monster_attack_player_dampen_transient(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            *damage,
            *hits,
            ctx.events,
        )?;
    } else if ctx
        .state
        .monsters
        .get(ctx.actor)
        .is_some_and(|monster| monster.kind == MonsterKind::Aeonglass)
    {
        monster_attack_player_aeonglass_transient(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            *damage,
            *hits,
            ctx.events,
        )?;
    } else {
        monster_attack_player_with_catalog(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            *damage,
            *hits,
            ctx.events,
        )?;
    }
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let block: i32 = (*block)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("attack_block block"))?;
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        monster.block = monster
            .block
            .checked_add(block)
            .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
    }
    Ok(())
}

/// `("attack_frail", damage, hits, frail)` — attack, then enemy-applied
/// player Frail while the actor and combat remain live.
pub(crate) fn attack_frail(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(frail),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_frail"));
    };
    if *damage < 0 || *hits < 1 || *frail <= 0 {
        return Err(EngineRefusal::MalformedArgs("attack_frail"));
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
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerFrail,
            PowerId::PlayerFrailFresh,
            (*frail)
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("attack_frail Frail"))?,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `attack_strength` — attack, then apply owner Strength while live.
///
/// Python: `monster_act` (frozen, deleted #2827) and the resumed suffix in
/// `_finish_monster_move_after_attack`.
///
/// The suffix rechecks combat and actor liveness after the awaited attack,
/// then publishes the checked Strength change. Carrier-specific Shell,
/// spawning, and loop gaps remain independent admission refusals.
pub(crate) fn attack_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(amount),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_strength"));
    };
    if *damage < 0 || *hits < 1 || *amount < 0 {
        return Err(EngineRefusal::MalformedArgs("attack_strength"));
    }
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("attack_strength Strength"))?;
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        // Retaliation can mutate this owner before the awaited attack
        // returns. Ceremonial Beast's Plow threshold, in particular, clears
        // both Strength and TempStrength. Native's suffix reads that live
        // post-attack state rather than a pre-attack scalar snapshot.
        let updated = crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[ctx.actor],
            amount,
            "attack_strength Strength",
        )?;
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let uid = monster.uid;
        crate::engine::damage::write_monster_self_strength(monster, updated, upkeep);
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Strength,
            updated,
        );
    }
    Ok(())
}

/// `("attack_weak_frail", damage, hits, weak, frail)` — attack, then Weak
/// and Frail in that order, each marked fresh.
pub(crate) fn attack_weak_frail(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(weak),
        CompiledArg::I(frail),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_weak_frail"));
    };
    if *damage < 0 || *hits < 1 || *weak <= 0 || *frail <= 0 {
        return Err(EngineRefusal::MalformedArgs("attack_weak_frail"));
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
        for (power, fresh, raw, site) in [
            (
                PowerId::PlayerWeak,
                PowerId::PlayerWeakFresh,
                *weak,
                "attack_weak_frail Weak",
            ),
            (
                PowerId::PlayerFrail,
                PowerId::PlayerFrailFresh,
                *frail,
                "attack_weak_frail Frail",
            ),
        ] {
            apply_player_duration_affliction(
                ctx.state,
                power,
                fresh,
                raw.try_into()
                    .map_err(|_| EngineRefusal::CounterOverflow(site))?,
                true,
                ctx.events,
            )?;
        }
    }
    Ok(())
}

/// `("attack_weak_player", damage, hits, weak)` — attack, then enemy-applied
/// player Weak while the actor and combat remain live.
pub(crate) fn attack_weak_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(weak),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_weak_player"));
    };
    if *damage < 0 || *hits < 1 || *weak <= 0 {
        return Err(EngineRefusal::MalformedArgs("attack_weak_player"));
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
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerWeak,
            PowerId::PlayerWeakFresh,
            (*weak)
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("attack_weak_player Weak"))?,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `("block", amount)` — Crossbow Ruby Raider's RELOAD and Magi Knight's PREP.
/// MagiKnight::PrepMove (v111 MoveNext RVA 0x363490) awaits
/// CreatureCmd::GainBlock(get_PowerShieldBlock()), RVA `0xb9235`,
/// `GetValueIfAscension(8, 9, 5)` at the fight's tier (#2828).
/// Its fixed MAGIC_BOMB follow is admitted independently.
///
/// Python: `monster_act` (frozen, deleted #2827). The oracle deliberately fail-closes this
/// shared kind to the one native body reviewed for #1374.
pub(crate) fn block(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let (amount, _, _) =
        move_args(MoveKind::Block, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    let row = (ctx.state.monsters[ctx.actor].kind, amount);
    if row != (MonsterKind::CrossbowRubyRaider, 3)
        && row
            != (
                MonsterKind::MagiKnight,
                ctx.tier(mc::MAGI_KNIGHT_POWER_SHIELD_BLOCK),
            )
    {
        return Err(EngineRefusal::MalformedArgs("monster block owner"));
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("monster block"))?;
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    monster.block = monster
        .block
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
    Ok(())
}

/// `("block_strength", block, strength)` — gain Block, then apply Strength
/// to the actor.
///
/// Python: `monster_act` (frozen, deleted #2827). Both commands target the already-live actor;
/// neither can end combat, so the serial liveness gates are unobservable on
/// the admitted surface.
pub(crate) fn block_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(block), CompiledArg::I(strength)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("block_strength"));
    };
    if *block < 0 || *strength < 0 {
        return Err(EngineRefusal::MalformedArgs("block_strength"));
    }
    let block: i32 = (*block)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("block_strength block"))?;
    let strength: i32 = (*strength)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("block_strength strength"))?;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let updated_block = monster
        .block
        .checked_add(block)
        .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
    let updated_strength = crate::engine::damage::checked_monster_strength_successor(
        monster,
        strength,
        "monster strength",
    )?;
    let uid = monster.uid;
    monster.block = updated_block;
    crate::engine::damage::write_monster_self_strength(monster, updated_strength, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Strength,
        updated_strength,
    );
    Ok(())
}

/// `buff_strength` — apply checked owner Strength.
///
/// Python: `monster_act` (frozen, deleted #2827) applies the row amount to the actor's live
/// Strength instance. The R2 unit owns the shared gate, so the former
/// wave-local stale-fixture constraint no longer blocks this exact body.
pub(crate) fn buff_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("buff_strength"));
    };
    if *amount < 0 {
        return Err(EngineRefusal::MalformedArgs("buff_strength"));
    }
    add_monster_strength(ctx, *amount, "buff_strength Strength")
}

/// `("buff_thorns", amount)` — `Apply<ThornsPower>(amount)` on the actor.
///
/// Python: `monster_act`'s buff_thorns branch.
pub(crate) fn buff_thorns(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let (raw, _, _) =
        move_args(MoveKind::BuffThorns, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let amount: i32 = raw
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("thorns"))?;
    let updated = monster.powers.value(PowerId::Thorns) + amount;
    monster.powers.set(PowerId::Thorns, SlotWire::Int, updated);
    Ok(())
}

/// `("burrow", amount)` — Tunneler's BURROW.
///
/// Python: `monster_act` (frozen, deleted #2827). Apply Burrowed first, then gain Block; the
/// power keeps that Block across the owner's side start until a positive
/// block break removes it and installs the one-turn DIZZY override. The Block
/// is `Tunneler::get_BlockGain` RVA `0xc35de`, `GetValueIfAscension(8, 37, 32)`
/// at the fight's tier (#2828).
pub(crate) fn burrow(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let (amount, _, _) =
        move_args(MoveKind::Burrow, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::Tunneler
        || amount != ctx.tier(mc::TUNNELER_BLOCK_GAIN)
    {
        return Err(EngineRefusal::MalformedArgs("monster burrow owner"));
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("monster burrow block"))?;
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    monster.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
    monster.block = monster
        .block
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
    note_power(
        ctx.events,
        Subject::Monster(monster.uid),
        PowerId::Burrowed,
        1,
    );
    Ok(())
}

/// `("frail_player", amount)` — apply enemy-sourced Frail and mark it fresh.
pub(crate) fn frail_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("frail_player"));
    };
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("frail_player Frail"))?;
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("frail_player"));
    }
    apply_player_duration_affliction(
        ctx.state,
        PowerId::PlayerFrail,
        PowerId::PlayerFrailFresh,
        amount,
        true,
        ctx.events,
    )
}

/// `headbutt` — Bowlbug Rock's attack; the DIZZY response is the owner's power.
///
/// `BowlbugRock/<HeadbuttMove>d__24::MoveNext` 0x354048 builds and executes the
/// `AttackCommand` (IL_002e–IL_0063), awaits it, then at IL_00bf reads
/// `get_IsOffBalance` and at IL_00c7 awaits `BowlbugRock::Stun` (0xafddc ->
/// `<Stun>d__25::MoveNext` 0x3541c4 IL_009d–IL_00b0), which is
/// `CreatureCmd::Stun(Creature, /*stunMove*/ DizzyMove, /*nextMoveId*/ null)`.
/// `<DizzyMove>d__26::MoveNext` 0x353f70 IL_001e–IL_001f clears the latch.
/// Because `ForceCurrentState` (0x78f6b) never appends to `StateLog`, the
/// parked successor is the same HEADBUTT row — hence `loop_pos` 0 and no
/// `forced_follow_up`.
///
/// The latch itself is **not** set here: `ImbalancedPower/<AfterDamageGiven>d__4::MoveNext`
/// 0x33cd60 sets it, from the owner-side hook inside the damage command
/// ([`crate::engine::damage::monsters_after_damage_given`], #2647 slice A). The
/// setter `BowlbugRock::set_IsOffBalance` 0xafc54 has exactly those two
/// callers, and its backing field `_isOffBalance` has no other writer anywhere
/// in the assembly, so collapsing latch-plus-terminal-stun into the single
/// `Dizzy` state is exact for this move. This body keeps the `(damage, 1)` row
/// and kind pin that makes HEADBUTT the only way a `BowlbugRock` deals damage;
/// the damage is `BowlbugRock::get_HeadbuttDamage` RVA `0xafc3f`,
/// `GetValueIfAscension(9, 16, 15)` at the fight's tier (#2828).
///
/// `CreatureCmd/<Damage>d__12` 0x3e96c8 computes the result before Osty
/// interposition (IL0505–0554) and copies it to the original player result
/// (IL069f–06d3); unchanged player HP is therefore not its predicate. Frozen
/// Python `monster_act` (deleted #2827) / `_finish_monster_move_after_attack` use
/// unchanged HP; the native result replaces that shortcut without changing the
/// frozen Python model.
pub(crate) fn headbutt(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("headbutt"));
    };
    if *damage < 0 || *hits < 1 {
        return Err(EngineRefusal::MalformedArgs("headbutt"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::BowlbugRock
        || (*damage, *hits) != (ctx.tier(mc::BOWLBUG_ROCK_HEADBUTT_DAMAGE), 1)
    {
        return Err(EngineRefusal::MalformedArgs("headbutt owner/row"));
    }
    monster_attack_player_with_catalog_result(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    Ok(())
}

/// `none` — a no-op move with no arguments.
///
/// Python: `monster_act` (frozen, deleted #2827). The exact body accepts only the generated
/// empty argument row and performs no mutation; carrier-private state remains
/// independently admission-gated.
pub(crate) fn none(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("none"));
    }
    Ok(())
}

fn add_monster_strength(
    ctx: &mut MoveCtx<'_>,
    raw: i64,
    site: &'static str,
) -> Result<(), EngineRefusal> {
    let amount: i32 = raw
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow(site))?;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let updated = crate::engine::damage::checked_monster_strength_successor(monster, amount, site)?;
    let uid = monster.uid;
    crate::engine::damage::write_monster_self_strength(monster, updated, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Strength,
        updated,
    );
    Ok(())
}

/// `ritual` — apply Ritual to the actor and mark the application fresh.
///
/// Python: `monster_act` (frozen, deleted #2827). `Apply<RitualPower>(amount)` stacks
/// the owner's singleton and sets `WasJustAppliedByEnemy`; the corresponding
/// side-end reader is `engine::turn::tick_monster_ritual`.
pub(crate) fn ritual(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("ritual"));
    };
    let expected = match ctx.state.monsters[ctx.actor].kind {
        MonsterKind::CalcifiedCultist => 2,
        // `DampCultist::get_IncantationAmount` RVA `0xb27ed`,
        // `GetValueIfAscension(9, 6, 5)` at the fight's tier (#2828).
        MonsterKind::DampCultist => ctx.tier(mc::DAMP_CULTIST_INCANTATION_AMOUNT),
        MonsterKind::DevotedSculptor => 9,
        _ => return Err(EngineRefusal::MalformedArgs("ritual owner")),
    };
    if *amount != expected {
        return Err(EngineRefusal::MalformedArgs("ritual amount"));
    }
    if ctx.state.history.over || ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("ritual"))?;
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let was_active = monster.powers.value(PowerId::Ritual) > 0;
    let updated = monster
        .powers
        .value(PowerId::Ritual)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("ritual"))?;
    if !was_active && monster.powers.value(PowerId::Demise) > 0 {
        // Demise was already in the owner's listener list when this Ritual
        // singleton was created, so its side-end command must run first.
        // Restacks preserve the original relative order.
        monster.set_demise_before_ritual(true);
    }
    monster.powers.set(PowerId::Ritual, SlotWire::Int, updated);
    monster.ritual_fresh = true;
    note_power(
        ctx.events,
        Subject::Monster(monster.uid),
        PowerId::Ritual,
        updated,
    );
    Ok(())
}

/// `("shrink", 1)` — install the unique infinite Shrink instance.
///
/// The canonical slot is a presence sentinel rather than the native display
/// duration. Shrink is non-stackable and has no side-end tick.
pub(crate) fn shrink(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("shrink"));
    };
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::ShrinkerBeetle {
        return Err(EngineRefusal::MalformedArgs("shrink owner"));
    }
    if ctx.state.history.over {
        return Ok(());
    }
    if ctx.state.powers.value(PowerId::PlayerShrink) > 0 {
        return Err(EngineRefusal::PowerRestackNotModeled(PowerId::PlayerShrink));
    }
    ctx.state
        .powers
        .set(PowerId::PlayerShrink, SlotWire::Int, 1);
    note_power(ctx.events, Subject::Player, PowerId::PlayerShrink, 1);
    Ok(())
}

/// `("spit_attack", damage, hits, thorns_spent)`.
///
/// Python: `monster_act`'s spit_attack branch.
/// `Apply<ThornsPower>(-SpikenAmount)` runs **first**, then the multi-attack
/// (v0.110.1 `d__25` 0x3b7d48). The rotation guarantees a SPIKEN before every
/// SPIKE_SPIT, so the amount never goes negative; the clamp is here anyway,
/// because Python's `PowerCmd` removes an empty power.
pub(crate) fn spit_attack(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits, spent) =
        move_args(MoveKind::SpitAttack, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    {
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let spent: i32 = spent
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("thorns"))?;
        let updated = (monster.powers.value(PowerId::Thorns) - spent).max(0);
        monster.powers.set(PowerId::Thorns, SlotWire::Int, updated);
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, hits, ctx.events)
}

/// `wriggle` — Wriggler Strength plus one or more legacy Infections.
///
/// Python's body writes Strength first, then appends compact Infection tuples
/// to Discard/Bottom. Their physical uids are allocated by the next native
/// all-piles normalization pass.
pub(crate) fn wriggle(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(strength), CompiledArg::I(infections)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("wriggle"));
    };
    let strength: i32 = (*strength)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("wriggle strength"))?;
    let infections: usize = (*infections)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("wriggle infections"))?;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let updated = crate::engine::damage::checked_monster_strength_successor(
        monster,
        strength,
        "wriggle strength",
    )?;
    crate::engine::damage::write_monster_self_strength(monster, updated, upkeep);
    inject_legacy_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        },
        infections,
        PileId::Discard,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CatalogBuilder, CompiledMove};
    use crate::ids::MonsterKind;
    use serde_json::Value;

    const FIXTURE: &str = include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

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

    fn run_bowlbug_headbutt(state: &mut crate::hot::HotState, catalog: &crate::catalog::Catalog) {
        let entry = row(catalog, MonsterKind::BowlbugRock, MoveKind::Headbutt);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state,
            catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        headbutt(&mut ctx).unwrap();
    }

    #[test]
    fn bowlbug_headbutt_uses_the_pre_osty_fully_blocked_result() {
        for (name, block, osty, buffer, expected_dizzy, expected_hp) in [
            ("fully blocked", 16, None, 0, true, 100),
            ("partial block", 15, None, 0, false, 99),
            ("no block", 0, None, 0, false, 84),
            ("Osty absorbs all", 0, Some((20, 20)), 0, false, 100),
            ("Osty absorbs part", 0, Some((5, 5)), 0, false, 89),
            ("Block then Osty", 5, Some((20, 20)), 0, false, 100),
            // A live but unused pet: Block covers the whole hit, so the
            // pre-interposition flag is true and the pet never interposes.
            ("Osty alive but unused", 16, Some((20, 20)), 0, true, 100),
            ("Buffer without Block", 0, None, 1, false, 100),
            ("Buffer after partial Block", 5, None, 1, false, 100),
        ] {
            let (document, catalog) = state_for(MonsterKind::BowlbugRock);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            state.hp = 100;
            state.max_hp = 100;
            state.block = block;
            state.powers.set(PowerId::Buffer, SlotWire::Int, buffer);
            state.fanouts.set_osty(osty).unwrap();

            run_bowlbug_headbutt(&mut state, &catalog);

            assert_eq!(
                state.monsters[0].override_state == crate::hot::MonsterOverride::Dizzy,
                expected_dizzy,
                "{name}"
            );
            assert_eq!(state.hp, expected_hp, "{name}");
        }
    }

    #[test]
    fn bowlbug_headbutt_distinguishes_zero_damage_block_states() {
        for (name, block, expected) in [
            ("zero damage without Block", 0, false),
            ("zero damage with retained Block", 1, true),
        ] {
            let (document, catalog) = state_for(MonsterKind::BowlbugRock);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            state.block = block;
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Strength, SlotWire::Int, -16);

            run_bowlbug_headbutt(&mut state, &catalog);

            assert_eq!(
                state.monsters[0].override_state == crate::hot::MonsterOverride::Dizzy,
                expected,
                "{name}"
            );
        }
    }

    /// `state_for` at `ascension`, through the real boundary: the catalog
    /// compiles the move rows at the document's `player.ascension` (#2828).
    fn state_at(
        monster: MonsterKind,
        ascension: u8,
    ) -> (crate::hot::HotState, crate::catalog::Catalog) {
        let (mut document, _) = state_for(monster);
        if ascension != crate::encounters::MODELED_ASCENSION {
            document
                .player
                .insert("ascension".to_owned(), Value::from(ascension));
        }
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        assert_eq!(catalog.ascension(), ascension);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        state.max_hp = 100;
        (state, catalog)
    }

    fn run_row(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        monster: MonsterKind,
        name: &str,
    ) {
        let index = crate::content_tables::monster_loop(monster)
            .unwrap()
            .iter()
            .position(|entry| entry.name == name)
            .unwrap();
        let entry = catalog.moves(monster)[index];
        crate::moves::apply_move(
            entry.kind,
            &mut MoveCtx {
                state,
                catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut Vec::new(),
            },
        )
        .unwrap();
    }

    /// #2828 witnesses, one per kind of tiered constant, each on both sides
    /// of its own gate through the boundary. `HasLevel` RVA `0x11fa83` is
    /// `_level >= gate`, so A8 is at-or-above an A8 gate and below an A9 one.
    ///
    /// * damage and block in one row, with different gates: Nibbit SLICE is
    ///   `get_SliceDamage` `0xba37c` (9, 7, 6) then `get_SliceBlock`
    ///   `0xba372` (8, 6, 5);
    /// * a buff amount: Nibbit HISS, `get_HissStrengthGain` `0xba387` (9, 3, 2);
    /// * a multi-hit count: Tracker Ruby Raider HOUNDS, `get_HoundsDamage`
    ///   `0xc3458` (9, 1, 1) times `get_HoundsRepeat` `0xc3463` (9, 9, 8);
    /// * an owner-pinned body: Bowlbug Rock HEADBUTT, `get_HeadbuttDamage`
    ///   `0xafc3f` (9, 16, 15).
    #[test]
    fn tiered_rows_follow_each_constants_own_gate() {
        for (ascension, slice_damage, slice_block, hiss, hounds, headbutt) in [
            (7, 6, 5, 2, 8, 15),
            (8, 6, 6, 2, 8, 15),
            (9, 7, 6, 3, 9, 16),
            (10, 7, 6, 3, 9, 16),
        ] {
            let (mut state, catalog) = state_at(MonsterKind::Nibbit, ascension);
            run_row(&mut state, &catalog, MonsterKind::Nibbit, "SLICE");
            assert_eq!(100 - state.hp, slice_damage, "A{ascension} SLICE damage");
            assert_eq!(
                state.monsters[0].block, slice_block,
                "A{ascension} SLICE block"
            );

            let (mut state, catalog) = state_at(MonsterKind::Nibbit, ascension);
            run_row(&mut state, &catalog, MonsterKind::Nibbit, "HISS");
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                hiss,
                "A{ascension} HISS"
            );

            let (mut state, catalog) = state_at(MonsterKind::TrackerRubyRaider, ascension);
            run_row(
                &mut state,
                &catalog,
                MonsterKind::TrackerRubyRaider,
                "HOUNDS_MOVE",
            );
            assert_eq!(100 - state.hp, hounds, "A{ascension} HOUNDS hits");

            let (mut state, catalog) = state_at(MonsterKind::BowlbugRock, ascension);
            run_bowlbug_headbutt(&mut state, &catalog);
            assert_eq!(100 - state.hp, headbutt, "A{ascension} HEADBUTT");
        }
    }

    /// #2828: no body pins its own row at one tier. Every generated row that
    /// carries an `Arg::Tier` is run from the same bare state at A0 (below
    /// every gate) and at A10 (at-or-above every gate); the two may differ in
    /// the numbers dealt, never in whether the body accepts its row or in
    /// the refusal it names. A literal left pinned to the A9+ value refuses
    /// `MalformedArgs` at A0 only, and fails here by name.
    #[test]
    fn no_body_pins_its_tiered_row_to_one_tier() {
        let run = |kind: MonsterKind, index: usize, ascension: u8| {
            let mut builder = CatalogBuilder::new();
            assert!(builder.set_ascension(ascension));
            builder.intern_monster(kind).ok()?;
            let catalog = builder.build();
            let entry = *catalog.moves(kind).get(index)?;
            let mut state = crate::hot::HotState::at_defaults();
            assert!(state.fanouts.set_ascension(ascension));
            state.hp = 500;
            state.max_hp = 500;
            let mut monster = crate::hot::HotMonster::new(kind, 300);
            monster.max_hp = 300;
            monster.loop_pos = i32::try_from(index).unwrap();
            state.monsters_mut().push(monster);
            let result = crate::moves::apply_move(
                entry.kind,
                &mut MoveCtx {
                    state: &mut state,
                    catalog: &catalog,
                    actor: 0,
                    args: catalog.args(entry.args),
                    events: &mut Vec::new(),
                },
            );
            Some(result.err())
        };
        let mut checked = 0;
        let mut pinned = Vec::new();
        for (kind, rows) in crate::content_tables::LOOPS
            .iter()
            .chain(crate::content_tables::RANDOM_MOVES.iter())
        {
            for (index, row) in rows.iter().enumerate() {
                if !row
                    .args
                    .iter()
                    .any(|arg| matches!(arg, crate::content_tables::Arg::Tier(_)))
                {
                    continue;
                }
                let (Some(low), Some(high)) = (run(*kind, index, 0), run(*kind, index, 10)) else {
                    continue;
                };
                checked += 1;
                if low != high {
                    pinned.push(format!("{kind:?}.{}: A0 {low:?} vs A10 {high:?}", row.name));
                }
            }
        }
        assert!(pinned.is_empty(), "{checked} rows; pinned: {pinned:#?}");
        assert!(checked > 150, "{checked}");
    }

    #[test]
    fn bowlbug_headbutt_keeps_a_dead_dealer_out_of_dizzy() {
        let (document, catalog) = state_for(MonsterKind::BowlbugRock);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        state.block = 16;
        state.monsters_mut()[0].hp = 1;
        state.powers.set(PowerId::Thorns, SlotWire::Int, 1);

        run_bowlbug_headbutt(&mut state, &catalog);

        assert!(state.monsters[0].hp <= 0);
        assert_ne!(
            state.monsters[0].override_state,
            crate::hot::MonsterOverride::Dizzy
        );
    }

    #[test]
    fn bowlbug_dizzy_cold_round_trips_and_then_resumes_headbutt() {
        let (document, catalog) = state_for(MonsterKind::BowlbugRock);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 100;
        state.max_hp = 100;
        state.block = 16;
        state.powers.set(PowerId::Barricade, SlotWire::Int, 1);
        let state = crate::engine::apply_action(&state, &catalog, &crate::engine::Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(
            state.monsters[0].override_state,
            crate::hot::MonsterOverride::Dizzy
        );

        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt = HotBoundary::catalog_from_canonical(&document).unwrap();
        let reloaded = HotBoundary::from_canonical(&document, &rebuilt).unwrap();
        assert_eq!(
            crate::engine::admission::admit(&document, &reloaded, &rebuilt),
            Ok(())
        );

        let skipped =
            crate::engine::apply_action(&reloaded, &rebuilt, &crate::engine::Action::EndTurn)
                .unwrap()
                .state;
        assert_eq!(skipped.hp, 100);
        assert_eq!(
            skipped.monsters[0].override_state,
            crate::hot::MonsterOverride::None
        );

        let resumed =
            crate::engine::apply_action(&skipped, &rebuilt, &crate::engine::Action::EndTurn)
                .unwrap()
                .state;
        assert_eq!(resumed.hp, 84);
        assert_eq!(
            resumed.monsters[0].override_state,
            crate::hot::MonsterOverride::None
        );
    }

    #[test]
    fn the_owned_list_is_ascending_and_covers_every_claim() {
        assert!(OWNED.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(IMPLEMENTED.iter().all(|kind| OWNED.contains(kind)));
    }

    #[test]
    fn every_unclaimed_kind_refuses_by_its_own_name() {
        let (document, catalog) = state_for(MonsterKind::Toadpole);
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
                "{:?} must refuse under its own name",
                kind.as_str()
            );
        }
        assert!(events.is_empty());
    }

    #[test]
    fn ritual_generated_carriers_and_operands_are_exact() {
        let mut carriers = Vec::new();
        for monster in MonsterKind::ALL {
            let mut builder = CatalogBuilder::new();
            builder.intern_monster(monster).unwrap();
            let catalog = builder.build();
            for entry in catalog
                .moves(monster)
                .iter()
                .filter(|entry| entry.kind == MoveKind::Ritual)
            {
                let [CompiledArg::I(amount)] = catalog.args(entry.args) else {
                    panic!("Ritual carrier has a non-integer operand")
                };
                carriers.push((monster, i32::try_from(*amount).unwrap()));
            }
        }
        assert_eq!(
            carriers,
            [
                (MonsterKind::CalcifiedCultist, 2),
                (MonsterKind::DampCultist, 6),
                (MonsterKind::DevotedSculptor, 9),
            ]
        );

        for (monster, expected) in carriers {
            let (document, catalog) = state_for(monster);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            state.monsters_mut()[0].uid = 17;
            let entry = row(&catalog, monster, MoveKind::Ritual);
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut events,
            };
            ritual(&mut ctx).unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Ritual), expected);
            assert!(state.monsters[0].ritual_fresh);
            assert_eq!(
                events,
                [crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(17),
                    power: PowerId::Ritual,
                    amount: expected,
                }]
            );
        }
    }

    #[test]
    fn ritual_refuses_wrong_owner_or_amount_without_mutation() {
        for (monster, args, expected) in [
            (
                MonsterKind::Toadpole,
                [CompiledArg::I(2)],
                EngineRefusal::MalformedArgs("ritual owner"),
            ),
            (
                MonsterKind::CalcifiedCultist,
                [CompiledArg::I(3)],
                EngineRefusal::MalformedArgs("ritual amount"),
            ),
        ] {
            let (document, catalog) = state_for(monster);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &args,
                events: &mut events,
            };
            assert_eq!(ritual(&mut ctx), Err(expected));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn seapunk_bubble_burp_applies_block_before_strength() {
        let (document, catalog) = state_for(MonsterKind::Seapunk);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let entry = row(&catalog, MonsterKind::Seapunk, MoveKind::BlockStrength);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        block_strength(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].block, 8);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 2);
    }

    #[test]
    fn real_affliction_rows_apply_exact_amounts_and_fresh_latches() {
        for (monster, kind, expected) in [
            (MonsterKind::PunchConstruct, MoveKind::AttackFrail, (0, 1)),
            (
                MonsterKind::DecimillipedeSegment,
                MoveKind::AttackWeakPlayer,
                (1, 0),
            ),
            (MonsterKind::Axebot, MoveKind::AttackWeakFrail, (2, 2)),
            (MonsterKind::CorpseSlug, MoveKind::FrailPlayer, (0, 2)),
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
            assert_eq!(state.powers.value(PowerId::PlayerWeak), expected.0);
            assert_eq!(state.powers.value(PowerId::PlayerFrail), expected.1);
            assert_eq!(
                state.powers.value(PowerId::PlayerWeakFresh),
                i32::from(expected.0 > 0)
            );
            assert_eq!(
                state.powers.value(PowerId::PlayerFrailFresh),
                i32::from(expected.1 > 0)
            );
        }
    }

    #[test]
    fn shrink_is_unique_infinite_and_a_second_application_refuses() {
        let (document, catalog) = state_for(MonsterKind::ShrinkerBeetle);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let entry = row(&catalog, MonsterKind::ShrinkerBeetle, MoveKind::Shrink);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        shrink(&mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::PlayerShrink), 1);

        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        assert_eq!(
            shrink(&mut ctx),
            Err(EngineRefusal::PowerRestackNotModeled(PowerId::PlayerShrink))
        );
    }

    #[test]
    fn a_lethal_attack_never_runs_its_affliction_suffix() {
        let (document, catalog) = state_for(MonsterKind::PunchConstruct);
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        state.hp = 1;
        let entry = row(&catalog, MonsterKind::PunchConstruct, MoveKind::AttackFrail);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };

        attack_frail(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(state.powers.value(PowerId::PlayerFrail), 0);
        assert_eq!(state.powers.value(PowerId::PlayerFrailFresh), 0);
    }

    #[test]
    fn r2_claims_complete_the_expected_deterministic_loops() {
        let mut builder = CatalogBuilder::new();
        for kind in MonsterKind::ALL {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let mut completed = Vec::new();
        for monster in MonsterKind::ALL {
            let moves = catalog.moves(monster);
            if moves.is_empty()
                || !moves
                    .iter()
                    .any(|entry| entry.kind == MoveKind::BlockStrength)
                || !moves
                    .iter()
                    .all(|entry| super::super::is_implemented(entry.kind))
            {
                continue;
            }
            completed.push(monster.as_str());
        }
        assert_eq!(completed, ["MECHA_KNIGHT", "PUNCH_CONSTRUCT", "SEAPUNK"]);
    }

    #[test]
    fn r2_scalar_and_pile_bodies_execute_real_generated_rows() {
        for (monster, kind) in [
            (MonsterKind::LeafSlimeM, MoveKind::AddStatusDiscard),
            (MonsterKind::Myte, MoveKind::AttackStrength),
            (MonsterKind::Nibbit, MoveKind::BuffStrength),
            (MonsterKind::BowlbugRock, MoveKind::Headbutt),
            (MonsterKind::BattleFriendV1, MoveKind::None),
        ] {
            let (document, catalog) = state_for(monster);
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            state.hp = 100;
            if kind == MoveKind::Headbutt {
                state.block = 100;
            }
            let entry = row(&catalog, monster, kind);
            let before_discard = state.piles.get(PileId::Discard).len();
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut events,
            };
            super::super::apply_move(kind, &mut ctx).unwrap();
            match kind {
                MoveKind::AddStatusDiscard => {
                    assert_eq!(state.piles.get(PileId::Discard).len(), before_discard + 2)
                }
                MoveKind::AttackStrength | MoveKind::BuffStrength => {
                    assert!(state.monsters[0].powers.value(PowerId::Strength) > 0)
                }
                MoveKind::Headbutt => assert_eq!(
                    state.monsters[0].override_state,
                    crate::hot::MonsterOverride::Dizzy
                ),
                MoveKind::None => assert_eq!(state.hp, 100),
                _ => unreachable!(),
            }
        }
    }
}
