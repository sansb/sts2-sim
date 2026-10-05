//! Monster-move bodies for the `content/encounters/boss.py` pool — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # R42 status (#1976): 25 of 27 ported, 2 escalated
//!
//! R2 adds exact Ceremonial Beast, Vantom, Knowledge Demon, Matriarch, Test
//! Subject, Soul Fysh, The Insatiable, and Queen command shapes. The two
//! remaining refusals retain named prerequisites for Aeonglass card mapping
//! and Knowledge Demon blocking enemy choices; inline tests pin each refusal
//! and each new body.

use super::MoveCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::move_constants as mc;
use crate::content_tables::{Arg, Repeats};
use crate::decimal::DotNetDecimal;
use crate::engine::cards::{
    inject_generated_null_bottom, inject_generated_null_random, inject_legacy_bottom,
    preflight_generated_null_card_batch,
};
use crate::engine::damage::{
    apply_owner_strength, apply_player_duration_affliction,
    monster_attack_player_positive_results_with_catalog, monster_attack_player_with_catalog,
    note_power,
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
pub const IMPLEMENTED: &[MoveKind] = &[
    MoveKind::AeonglassIntensity,
    MoveKind::AttackBeckon,
    MoveKind::AttackWounds,
    MoveKind::BeastCry,
    MoveKind::BeastStamp,
    MoveKind::Beckon,
    MoveKind::Fade,
    MoveKind::InsatiableLiquify,
    MoveKind::KdCurse,
    MoveKind::Ponder,
    MoveKind::QueenBurnBright,
    MoveKind::QueenPuppetStrings,
    MoveKind::QueenYoureMine,
    MoveKind::Scream,
    MoveKind::SoulSiphon,
    MoveKind::TestSubjectBurningGrowl,
    MoveKind::TestSubjectMultiClaw,
    MoveKind::TestSubjectRespawn,
    MoveKind::TestSubjectSkullBash,
    MoveKind::WaterfallAbout,
    MoveKind::WaterfallExplode,
    MoveKind::WaterfallPressureGun,
    MoveKind::WaterfallPressureUp,
    MoveKind::WaterfallPressurize,
    MoveKind::WaterfallRam,
    MoveKind::WaterfallSiphon,
    MoveKind::WaterfallStomp,
];

fn require_waterfall_owner(ctx: &MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::WaterfallGiant {
        return Err(EngineRefusal::MalformedArgs("waterfall owner"));
    }
    Ok(())
}

fn add_waterfall_pressure(ctx: &mut MoveCtx<'_>, amount: i32) -> Result<(), EngineRefusal> {
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let pressure = monster
        .powers
        .value(PowerId::SteamPressure)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("steam pressure"))?;
    let buildup = monster
        .pressure_buildup_idx()
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("pressure buildup index"))?;
    let uid = monster.uid;
    monster
        .powers
        .set(PowerId::SteamPressure, SlotWire::Int, pressure);
    let stored = monster.set_pressure_buildup_idx(buildup);
    debug_assert!(stored);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::SteamPressure,
        pressure,
    );
    Ok(())
}

/// Every kind owned by this family, ascending. Tests below derive the carrier
/// census from the generated catalog and pin every unclaimed body to its own
/// typed refusal, so this escalation record cannot silently lose a new stub.
#[cfg(test)]
const OWNED: [MoveKind; 27] = [
    MoveKind::AeonglassIntensity,
    MoveKind::AttackBeckon,
    MoveKind::AttackWounds,
    MoveKind::BeastCry,
    MoveKind::BeastStamp,
    MoveKind::Beckon,
    MoveKind::Fade,
    MoveKind::InsatiableLiquify,
    MoveKind::KdCurse,
    MoveKind::Ponder,
    MoveKind::QueenBurnBright,
    MoveKind::QueenPuppetStrings,
    MoveKind::QueenYoureMine,
    MoveKind::Scream,
    MoveKind::SoulSiphon,
    MoveKind::TestSubjectBurningGrowl,
    MoveKind::TestSubjectMultiClaw,
    MoveKind::TestSubjectRespawn,
    MoveKind::TestSubjectSkullBash,
    MoveKind::WaterfallAbout,
    MoveKind::WaterfallExplode,
    MoveKind::WaterfallPressureGun,
    MoveKind::WaterfallPressureUp,
    MoveKind::WaterfallPressurize,
    MoveKind::WaterfallRam,
    MoveKind::WaterfallSiphon,
    MoveKind::WaterfallStomp,
];

/// `("aeonglass_intensity", strength, withers)` — Increasing Intensity.
///
/// #2828: both arguments are tiered. `Aeonglass::get_IncreasingIntensityBaseStrength`
/// RVA `0xae809` is `GetValueIfAscension(9, 4, 3)` (read through
/// `get_IncreasingIntensityTotalStrength` RVA `0xae84d`, base + additional) and
/// `get_WitherAmount` RVA `0xae814` is `(9, 2, 1)`, the count of generated
/// Withers. Below A9 the body gains `3 + old_additional` and generates one.
///
/// Pre-R48 escalation citation map (historical statement superseded):
/// Python: `monster_act` (frozen, deleted #2827) -> `_aeonglass_intensity`. It validates
///
/// Current v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// move outer/body RVAs `0xaea34`/`0x3529ec`, generated callback/matcher
/// `0xaea80`/`0xaeaac`, AdditionalStrength get/set `0xae81f`/`0xae827`, and
/// WitherUpgradeCount get/set `0xae836`/`0xae83e`. Python `monster_act` (frozen, deleted #2827) dispatches to `_aeonglass_intensity`: it visits physical
/// Withers in Hand/Draw/Discard/Exhaust/Play order, increments the private
/// fake-upgrade counter, awaits two singular discard-bottom generated-card
/// commands, gains `4 + old_additional` Strength while live, and increments
/// additional Strength even after terminal work. The complete transaction is
/// rehearsed so a later refusal cannot publish only one generated child.
///
/// # No Wither child is terminal, and Regalite is inert (#3297)
///
/// The Withers' creator is null: `<IncreasingIntensityMove>d__36::MoveNext`
/// RVA `0x3529ec` IL_013d-IL_014c calls `AddToCombatAndPreview<Wither>(targets,
/// Discard, WitherAmount, null, Bottom)` (`ldnull` at IL_014a), and
/// `CardPileCmd/<AddToCombatAndPreview>d__27::MoveNext` RVA `0x3e334c`
/// IL_00c4-IL_00d5 forwards it to one awaited singular
/// `AddGeneratedCardToCombat` per card. That command raises three hooks
/// (`<AddGeneratedCardsToCombat>d__6` RVA `0x3e2f0c` IL_0155 and IL_01d8,
/// `<Add>d__10` RVA `0x3e1ba4` IL_0659 and IL_0887). This is every override of
/// each in the v0.111.0 DLL, read for a null-creator Status entering Discard:
///
/// - `AfterCardGeneratedForCombat`, eight overrides. Six leave on a null
///   creator before any mutation or await: Regalite RVA `0x32f988`
///   IL_0020-IL_0026, Arsenal `0x334ec8` IL_001d-IL_0023, Pillar of Creation
///   `0x340488` IL_001d-IL_0023, Smokestack `0x3455e8` IL_0033-IL_0039,
///   Soulbound `0x345824` IL_0020-IL_0026 and Trash to Treasure `0x349f90`
///   IL_0033-IL_0039. `RocketPunch::AfterCardGeneratedForCombat` RVA `0xe9d44`
///   returns unless `creator == Owner` (IL_000c-IL_0013). The eighth is
///   Aeonglass's own, RVA `0xaea80`: a synchronous `MatchWitherToUpgradeCount`
///   (IL_001c-IL_001e), modeled as the fake-upgrade growth.
/// - `AfterCardEnteredCombat`, fifteen overrides, none of which deals damage or
///   ends combat: Ghost Seed, Phantom Blades and Hexed touch keywords or
///   afflictions; Galvanic, Hex, Ringing, Smoggy, Tangled and Vital Spark
///   afflict the entering card; Sword Sage, Banshee's Cry, Flatten, Midnight,
///   Pinpoint and Stomp rewrite that card's own replay count or cost.
/// - `AfterCardChangedPiles`, six overrides outside the two `Mock*` test
///   powers. Bing Bong (`0x31fbf0`
///   IL_0021-IL_003f), Book of Five Rings (`0x320238` IL_0050-IL_006e),
///   Darkstone Periapt (`0x322664` IL_0021-IL_003f), Lucky Fysh (`0x329f6c`
///   IL_0021-IL_003f) and Hoarder (`0x3777f4` IL_0029-IL_0047) leave unless the
///   card's pile is the Deck (`PileType` 6), and
///   `SovereignBlade::AfterCardChangedPiles` (`0xec068`) returns unless the
///   card is itself. `AfterCardChangedPilesLate` has one override,
///   `SoulFysh` (`0xbed1c`), a monster this one-boss fight never holds.
///
/// So no first child can end combat or kill the boss, and the crate agrees:
/// `inject_aeonglass_wither` writes no creature HP and never sets
/// `history.over`. The "Aeonglass terminal first generated child" refusal that
/// stood after each child was unreachable and is gone. The "Aeonglass
/// suspendable generated listener" refusal of Regalite is gone with it:
/// Regalite's only await (`GainBlock`, IL_006d) lies behind the null-creator
/// gate.
///
/// # An owner-created Wither (#3606)
///
/// A Wither the player creates (a clone) is a different relation: the
/// owner-gated listeners above pass their creator gate and answer it in the
/// same walk as Aeonglass's listener. `CombatState/<IterateHookListeners>d__69`
/// RVA `0x3f9720` lists the owner's powers, relics and cards before any
/// enemy's powers and model, so Aeonglass is asked last, and only while it is
/// still in combat (`CombatState::Contains` RVA `0x137564` IL_01b2-IL_01c1).
/// `engine::cards::after_local_card_generated_inner` carries the full read
/// and runs that order; the Regalite pairing #3297 kept refused is admitted.
pub(crate) fn aeonglass_intensity(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    fn apply(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
        let [CompiledArg::I(strength), CompiledArg::I(withers)] = ctx.args else {
            return Err(EngineRefusal::MalformedArgs("aeonglass_intensity"));
        };
        let (strength, withers) = (*strength, *withers);
        if (strength, withers)
            != (
                ctx.tier(mc::AEONGLASS_INCREASING_INTENSITY_BASE_STRENGTH),
                ctx.tier(mc::AEONGLASS_WITHER_AMOUNT),
            )
        {
            return Err(EngineRefusal::MalformedArgs("aeonglass_intensity"));
        }
        if ctx.actor != 0
            || ctx.state.monsters[0].loop_pos != 2
            || !crate::engine::cards::aeonglass_state_is_exact(ctx.state, ctx.catalog)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass Increasing Intensity owner/state",
            ));
        }
        let old_additional = ctx.state.monsters[0].aeonglass_additional_strength();
        let old_upgrades = ctx.state.monsters[0].aeonglass_wither_upgrade_count();
        let next_upgrades = old_upgrades
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("wither_upgrade_count"))?;
        let next_additional = old_additional
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("additional_strength"))?;
        let strength_delta = i64::from(old_additional)
            .checked_add(strength)
            .and_then(|delta| i32::try_from(delta).ok())
            .ok_or(EngineRefusal::CounterOverflow(
                "Aeonglass Intensity Strength",
            ))?;
        let planned_strength = crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[0],
            strength_delta,
            "Aeonglass Intensity Strength",
        )?;
        let children = u32::try_from(withers)
            .map_err(|_| EngineRefusal::MalformedArgs("aeonglass_intensity"))?;
        ctx.state
            .next_generated_hook_uid
            .checked_add(
                i32::try_from(children)
                    .map_err(|_| EngineRefusal::MalformedArgs("aeonglass_intensity"))?,
            )
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        ctx.state
            .next_card_uid
            .checked_add(children)
            .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
        for pile in crate::engine::cards::AEONGLASS_CARD_PILES {
            for card in ctx.state.piles.get(pile).as_slice() {
                let spec = ctx
                    .catalog
                    .spec(card.atom)
                    .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                if matches!(spec.identity.id, CardId::Wither) {
                    ctx.state
                        .card_states
                        .get(card.uid)
                        .damage_growth
                        .checked_add(3)
                        .ok_or(EngineRefusal::CounterOverflow("Wither fake upgrade"))?;
                }
            }
        }

        for pile in crate::engine::cards::AEONGLASS_CARD_PILES {
            let cards = ctx.state.piles.get(pile).as_slice().to_vec();
            for card in cards {
                let spec = ctx
                    .catalog
                    .spec(card.atom)
                    .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                if matches!(spec.identity.id, CardId::Wither) {
                    ctx.state
                        .card_states
                        .add_damage_growth(card.uid, 3)
                        .ok_or(EngineRefusal::CounterOverflow("Wither fake upgrade"))?;
                }
            }
        }
        // `_map_combat_cards(..., force_exact=True)` (frozen Python `_aeonglass_intensity`, deleted #2827) promotes exact piles whether or not a
        // Wither was mapped; the Wither children below then require it
        // (#2957, `engine::cards::aeonglass_pile_mode_is_exact`).
        ctx.state.exact_piles = true;
        let stored = ctx.state.monsters_mut()[0].set_aeonglass_wither_upgrade_count(next_upgrades);
        debug_assert!(stored);
        // One singular null-creator command per Wither. No child can end
        // combat or kill the boss (the listener census above), so every child
        // enters and the Strength gain below always runs on a live boss.
        for _ in 0..children {
            crate::engine::cards::inject_aeonglass_wither(
                ctx.state,
                ctx.catalog,
                PileId::Discard,
                ctx.events,
            )?;
        }
        if !ctx.state.history.over && ctx.state.monsters[0].hp > 0 {
            let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
            let owner = &mut ctx.state.monsters_mut()[0];
            crate::engine::damage::write_monster_self_strength(owner, planned_strength, upkeep);
            note_power(
                ctx.events,
                Subject::Monster(owner.uid),
                PowerId::Strength,
                planned_strength,
            );
        }
        let stored = ctx.state.monsters_mut()[0].set_aeonglass_additional_strength(next_additional);
        debug_assert!(stored);
        Ok(())
    }

    let mut probe_state = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply(&mut MoveCtx {
        state: &mut probe_state,
        catalog: ctx.catalog,
        actor: ctx.actor,
        args: ctx.args,
        events: &mut probe_events,
    })?;
    apply(ctx)
}

/// `("attack_beckon", damage, hits, count)`.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_finish_monster_move_after_attack`'s
/// resumed suffix. Attack, then while owner and combat remain live
/// generate `count` Beckons at discard bottom.
///
/// Authenticate Soul Fysh's GAZE boundary, attack once for `SoulFysh::get_GazeDamage`
/// (RVA `0xbecbe`, `GetValueIfAscension(9, 8, 7)` at the fight's tier, #2828),
/// then append one exact generated Beckon+0 to Discard/Bottom only while both
/// combat and owner remain live. A terminal attack never begins the generated
/// command.
///
/// The Beckon's creator is null (#3256): `SoulFysh/<GazeMove>d__38::MoveNext`
/// RVA `0x36bba0` passes `ldnull` as `AddGeneratedCardToCombat`'s creator
/// (IL_0132-IL_0137, `(card, Discard, null, Bottom)`). So it records no
/// owner-created history (`Supermassive`'s filter `<>c__DisplayClass2_0::
/// <get_CanonicalVars>b__1` RVA `0x3c0aa8` counts only `Creator == Owner`)
/// and no owner-gated generated listener answers.
/// [`preflight_generated_null_card_batch`] is the uid/epoch wall, and since
/// #3297 it checks no owner history and no Arsenal or Pillar arithmetic.
pub(crate) fn attack_beckon(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_beckon"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::SOUL_FYSH_GAZE_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_beckon"));
    }
    if !crate::engine::monsters::soul_fysh_move_state_is_valid(ctx.state, ctx.actor, 2, 0) {
        return Err(EngineRefusal::MalformedArgs("attack_beckon owner/state"));
    }
    let identity = CardIdentity {
        id: CardId::Beckon,
        upgrade: 0,
        enchantment: None,
    };
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        preflight_generated_null_card_batch(ctx.state, ctx.catalog, identity, 1)?;
        inject_generated_null_bottom(ctx.state, ctx.catalog, identity, 1, ctx.events)?;
    }
    Ok(())
}

/// `("attack_wounds", damage, hits, count)`.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_finish_monster_move_after_attack`'s
/// resumed suffix. It authenticates Vantom's exact Dismember row,
/// attacks, then appends `count` Wounds to discard bottom while owner and
/// combat remain live.
///
/// Authenticate Vantom's exact row, await the attack, then append three L0
/// Wounds to Discard/Bottom while combat and owner remain live. The catalog
/// interns this row's implicit fixed identity. Slippery and Wound keyword
/// admission remain separate refusals. The damage is `Vantom::get_DismemberDamage`
/// RVA `0xc4570`, `GetValueIfAscension(9, 30, 26)` at the fight's tier (#2828).
pub(crate) fn attack_wounds(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("attack_wounds"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::VANTOM_DISMEMBER_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("attack_wounds"));
    }
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::Vantom {
        return Err(EngineRefusal::MalformedArgs("attack_wounds owner"));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        inject_legacy_bottom(
            ctx.state,
            ctx.catalog,
            CardIdentity {
                id: CardId::Wound,
                upgrade: 0,
                enchantment: None,
            },
            3,
            PileId::Discard,
        )?;
    }
    Ok(())
}

/// `("beast_cry",)` — apply Ringing to the player and every owner card.
///
/// Python: `monster_act` (frozen, deleted #2827) -> `_apply_ringing_power`. The empty-argument
/// move applies Ringing to the player through the exact affliction-power
/// overlap validator.
///
pub(crate) fn beast_cry(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("beast_cry"));
    };
    if ctx.actor != 0
        || !crate::engine::monsters::ceremonial_beast_state_is_valid(ctx.state)
        || ctx.state.monsters[ctx.actor].loop_pos != 2
    {
        return Err(EngineRefusal::MalformedArgs("beast_cry owner/state"));
    }
    crate::engine::cards::apply_ringing_power(ctx.state)
}

/// `("beast_stamp", threshold)` — install Ceremonial Beast's Plow threshold.
///
/// Python: `monster_act` (frozen, deleted #2827). It authenticates a fresh Ceremonial Beast and
/// installs the private Plow threshold exactly once. The threshold is
/// `CeremonialBeast::get_PlowAmount` RVA `0xb0c33`,
/// `GetValueIfAscension(9, 160, 150)` at the fight's tier (#2828).
///
pub(crate) fn beast_stamp(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(threshold)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("beast_stamp"));
    };
    let threshold = *threshold;
    if threshold != ctx.tier(mc::CEREMONIAL_BEAST_PLOW_AMOUNT) {
        return Err(EngineRefusal::MalformedArgs("beast_stamp"));
    }
    let threshold =
        i32::try_from(threshold).map_err(|_| EngineRefusal::MalformedArgs("beast_stamp"))?;
    if ctx.actor != 0
        || !crate::engine::monsters::ceremonial_beast_state_is_valid(ctx.state)
        || ctx.state.monsters[ctx.actor].loop_pos != 0
    {
        return Err(EngineRefusal::MalformedArgs("beast_stamp owner/state"));
    }
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    monster
        .powers
        .set(PowerId::PlowThreshold, SlotWire::Int, threshold);
    note_power(
        ctx.events,
        Subject::Monster(monster.uid),
        PowerId::PlowThreshold,
        threshold,
    );
    Ok(())
}

/// `("beckon", 2)`.
///
/// Python: `monster_act` (frozen, deleted #2827) -> `_add_beckons`. It creates one
/// Beckon at a random draw-pile position (one Shuffle-stream draw) and then
/// one at discard bottom, each through generated-card entry semantics.
///
/// The two commands are distinct authenticated transactions: first one fresh
/// Beckon+0 at a Shuffle-selected Draw index, then one fresh Beckon+0 at
/// Discard/Bottom. Uids and listener epochs follow that order exactly.
///
/// Both creators are null (#3256): `SoulFysh/<BeckonMove>d__37::MoveNext` RVA
/// `0x36b4b8` passes `ldnull` to `AddGeneratedCardToCombat` for the Draw/Random
/// card (IL_0213-IL_0218) and the Discard/Bottom card (IL_02ab-IL_02b0). No
/// owner-created history advances and no owner-gated listener answers, so the
/// two-card preflight checks only the uid, epoch and Strike counters the two
/// null commands write (#3297, [`preflight_generated_null_card_batch`]).
pub(crate) fn beckon(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("beckon"));
    };
    if !crate::engine::monsters::soul_fysh_move_state_is_valid(ctx.state, ctx.actor, 0, 0) {
        return Err(EngineRefusal::MalformedArgs("beckon owner/state"));
    }
    let identity = CardIdentity {
        id: CardId::Beckon,
        upgrade: 0,
        enchantment: None,
    };
    preflight_generated_null_card_batch(ctx.state, ctx.catalog, identity, 2)?;
    inject_generated_null_random(ctx.state, ctx.catalog, identity, PileId::Draw, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        inject_generated_null_bottom(ctx.state, ctx.catalog, identity, 1, ctx.events)?;
    }
    Ok(())
}

/// `("fade", amount)`.
///
/// Python: `monster_act` (frozen, deleted #2827). Before stacking Intangible it conditionally
/// rewrites the Demise-vs-Intangible same-owner listener order, then applies
/// the amount.
///
/// A fresh Intangible singleton acquired after an existing Demise records
/// Demise-first order. Re-stacking live Intangible preserves the existing
/// listener order.
pub(crate) fn fade(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("fade"));
    };
    if !crate::engine::monsters::soul_fysh_move_state_is_valid(ctx.state, ctx.actor, 3, 0) {
        return Err(EngineRefusal::MalformedArgs("fade owner/state"));
    }
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    if monster.powers.value(PowerId::Demise) > 0 && monster.powers.value(PowerId::Intangible) == 0 {
        monster.set_demise_after_intangible(false);
    }
    monster.powers.set(PowerId::Intangible, SlotWire::Int, 2);
    note_power(
        ctx.events,
        Subject::Monster(monster.uid),
        PowerId::Intangible,
        2,
    );
    Ok(())
}

/// `("insatiable_liquify",)` — Sandpit, then six serial Frantic Escapes.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_insatiable_liquify`. The full operation is simulated on a cloned state before
/// the live Sandpit write, so a later uid/RNG/generated-listener overflow
/// cannot leave a partial command. Each generated copy is a distinct
/// null-creator transaction at Random position: three Draw, then three
/// Discard, stopping exactly when a generated listener ends combat.
pub(crate) fn insatiable_liquify(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("insatiable_liquify"));
    };

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        actor: usize,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        if !crate::engine::monsters::insatiable_liquify_state_is_valid(state, actor) {
            return Err(EngineRefusal::MalformedArgs(
                "The Insatiable Liquify owner/state",
            ));
        }
        let identity = CardIdentity {
            id: CardId::FranticEscape,
            upgrade: 0,
            enchantment: None,
        };
        if catalog.atom(&identity).is_none() {
            return Err(EngineRefusal::UnknownMintIdentity(identity));
        }
        let uid = state.monsters[actor].uid;
        state.monsters_mut()[actor]
            .powers
            .set(PowerId::Sandpit, SlotWire::Int, 4);
        note_power(events, Subject::Monster(uid), PowerId::Sandpit, 4);
        for destination in [
            PileId::Draw,
            PileId::Draw,
            PileId::Draw,
            PileId::Discard,
            PileId::Discard,
            PileId::Discard,
        ] {
            inject_generated_null_random(state, catalog, identity, destination, events)?;
        }
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(&mut probe, ctx.catalog, ctx.actor, &mut Vec::new())?;
    apply(ctx.state, ctx.catalog, ctx.actor, ctx.events)
}

/// `("kd_curse",)` — exact Knowledge Demon blocking curse choice.
///
/// Current v0.111.0 arm64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// Knowledge Demon move/choice wrappers are RVAs `0xb79bc`, `0xb7b08`, and
/// `0xb7b54`; their awaited bodies are `0x3607a8` and `0x3605e0`. The four
/// chosen bodies are `0x399420`, `0x3acc64`, `0x3bc54c`, and `0x3c6ecc`.
/// Counter access is `0xb7926`/`0xb792e`.
/// Python: `monster_act` (frozen, deleted #2827), completed by `_kd_curse_choice`. It
/// suspends the enemy phase on the exact tiered one-of-two choice, applies
/// Disintegration/Mind Rot/Sloth/Waste Away,
/// increment the private counter, and resume the authenticated cursor.
pub(crate) fn kd_curse(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.state.pending.is_some()
        || !ctx.state.frames.is_empty()
        || ctx.state.monsters.len() != 1
        || ctx.actor != 0
    {
        return Err(EngineRefusal::MalformedArgs("kd_curse"));
    }
    let owner = &ctx.state.monsters[ctx.actor];
    let counter = owner
        .knowledge_demon_curse_counter()
        .filter(|counter| *counter < 3)
        .ok_or(EngineRefusal::MalformedArgs(
            "Knowledge Demon curse counter",
        ))?;
    if Some(owner.max_hp)
        != crate::engine::monsters::native_fixed_hp(ctx.state, MonsterKind::KnowledgeDemon)
        || owner.loop_pos != 0
        || owner.hp <= 0
        || counter > 2
    {
        return Err(EngineRefusal::MalformedArgs("Knowledge Demon curse owner"));
    }
    let uid = owner.uid;
    let record = crate::hot::EnemyPhaseRecord {
        actor_uid: uid,
        stage: 0,
        remaining_uids: Vec::new(),
    };
    let frame_record = ctx
        .state
        .frames
        .push_enemy_phase(&record)
        .ok_or(EngineRefusal::CounterOverflow("enemy phase record"))?;
    ctx.state.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
        frame_uid: uid,
        frame_record,
    }));
    Ok(())
}

/// `("ponder", damage, hits, heal, strength)`.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_finish_monster_move_after_attack`'s
/// resumed suffix. Attack, then while owner and combat remain live
/// clamp-heal the owner and apply Strength.
///
/// Authenticate the Knowledge Demon row, await the attack, clamp-heal by 30,
/// then apply Strength while combat and owner remain live. Its blocking
/// curse choice and conditional successor remain separate refusals. #2828:
/// `KnowledgeDemon::get_PonderDamage` RVA `0xb7964` is
/// `GetValueIfAscension(9, 13, 11)` and `get_PonderStrength` RVA `0xb797d` is
/// `(9, 3, 2)`, both at the fight's tier; the heal is untiered.
pub(crate) fn ponder(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(1),
        CompiledArg::I(30),
        CompiledArg::I(strength),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("ponder"));
    };
    let (damage, strength) = (*damage, *strength);
    if (damage, strength)
        != (
            ctx.tier(mc::KNOWLEDGE_DEMON_PONDER_DAMAGE),
            ctx.tier(mc::KNOWLEDGE_DEMON_PONDER_STRENGTH),
        )
    {
        return Err(EngineRefusal::MalformedArgs("ponder"));
    }
    let strength = i32::try_from(strength).map_err(|_| EngineRefusal::MalformedArgs("ponder"))?;
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::KnowledgeDemon {
        return Err(EngineRefusal::MalformedArgs("ponder owner"));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let monster = &ctx.state.monsters[ctx.actor];
        let hp = monster
            .hp
            .checked_add(30)
            .ok_or(EngineRefusal::CounterOverflow("ponder heal"))?
            .min(monster.max_hp);
        let planned_strength = crate::engine::damage::checked_monster_strength_successor(
            monster,
            strength,
            "ponder Strength",
        )?;
        let uid = monster.uid;
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        monster.hp = hp;
        crate::engine::damage::write_monster_self_strength(monster, planned_strength, upkeep);
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Strength,
            planned_strength,
        );
    }
    Ok(())
}

/// `("queen_burn_bright", strength, block)`.
///
/// Python: `monster_act` (frozen, deleted #2827). It snapshots living teammates excluding the
/// Queen, applies Strength serially, then unconditionally attempts Queen
/// Block. A retained move after the Amalgam dies has a separately validated
/// empty-snapshot route and clears its private latch.
///
/// The shared Queen/Amalgam lifecycle authenticates the fixed roster, death
/// and retained-Burn latches, and both conditional-successor branches before
/// this body is reachable.
pub(crate) fn queen_burn_bright(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    queen_burn_bright_apply(ctx)
}

#[cfg_attr(not(test), allow(dead_code))]
fn queen_burn_bright_row_is_exact(ctx: &MoveCtx<'_>) -> bool {
    queen_amalgam_machine_is_exact(ctx.catalog)
}

pub(crate) fn queen_amalgam_machine_is_exact(catalog: &crate::catalog::Catalog) -> bool {
    let [strong_tackle, tackle_two, beam, tackle_three, tackle_four] =
        catalog.moves(MonsterKind::TorchHeadAmalgam)
    else {
        return false;
    };
    // #2828: each damage is TorchHeadAmalgam's tiered getter at the
    // catalog's tier (StrongTackle 0xc2c1e, Tackle 0xc2c2b, SoulBeam 0xc2c45,
    // WeakTackle 0xc2c38); the hit counts are untiered.
    let at = |tier| CompiledArg::I(catalog.tier(tier));
    let amalgam_machine = [
        (
            strong_tackle,
            [
                at(mc::TORCH_HEAD_AMALGAM_STRONG_TACKLE_DAMAGE),
                CompiledArg::I(1),
            ],
        ),
        (
            tackle_two,
            [at(mc::TORCH_HEAD_AMALGAM_TACKLE_DAMAGE), CompiledArg::I(1)],
        ),
        (
            beam,
            [
                at(mc::TORCH_HEAD_AMALGAM_SOUL_BEAM_DAMAGE),
                CompiledArg::I(3),
            ],
        ),
        (
            tackle_three,
            [
                at(mc::TORCH_HEAD_AMALGAM_WEAK_TACKLE_DAMAGE),
                CompiledArg::I(1),
            ],
        ),
        (
            tackle_four,
            [
                at(mc::TORCH_HEAD_AMALGAM_WEAK_TACKLE_DAMAGE),
                CompiledArg::I(1),
            ],
        ),
    ];
    if amalgam_machine
        .iter()
        .any(|(row, args)| row.kind != MoveKind::Attack || catalog.args(row.args) != *args)
        || strong_tackle.repeats != Repeats::Absent
        || tackle_two.repeats != Repeats::Absent
        || beam.repeats != Repeats::Absent
        || tackle_three.repeats != Repeats::Absent
        || tackle_four.repeats != Repeats::Fixed(2)
    {
        return false;
    }

    let mut rows = catalog
        .moves(MonsterKind::Queen)
        .iter()
        .filter(|entry| entry.kind == MoveKind::QueenBurnBright);
    let Some(row) = rows.next() else {
        return false;
    };
    if rows.next().is_some()
        || catalog.args(row.args) != [at(mc::QUEEN_BURN_BRIGHT_STRENGTH), CompiledArg::I(20)]
    {
        return false;
    }
    if !matches!(
        row.repeats,
        Repeats::Conditional(args)
            if args == [Arg::S("queen_amalgam_alive"), Arg::I(2), Arg::I(3)]
    ) {
        return false;
    }
    let [puppet, mine, burn, off_with_head, execution, enrage] = catalog.moves(MonsterKind::Queen)
    else {
        return false;
    };
    puppet.kind == MoveKind::QueenPuppetStrings
        && catalog.args(puppet.args) == [CompiledArg::I(3)]
        && puppet.repeats == Repeats::Absent
        && mine.kind == MoveKind::QueenYoureMine
        && catalog.args(mine.args) == [CompiledArg::I(99)]
        && matches!(mine.repeats, Repeats::Conditional(args) if args == [Arg::S("queen_amalgam_alive"), Arg::I(2), Arg::I(3)])
        && burn == row
        && off_with_head.kind == MoveKind::Attack
        && catalog.args(off_with_head.args)
            == [at(mc::QUEEN_OFF_WITH_YOUR_HEAD_DAMAGE), CompiledArg::I(5)]
        && off_with_head.repeats == Repeats::Absent
        && execution.kind == MoveKind::Attack
        && catalog.args(execution.args) == [at(mc::QUEEN_EXECUTION_DAMAGE), CompiledArg::I(1)]
        && execution.repeats == Repeats::Absent
        && enrage.kind == MoveKind::BuffStrength
        && catalog.args(enrage.args) == [CompiledArg::I(2)]
        && enrage.repeats == Repeats::Fixed(3)
}

// Current SHA-256
// 9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4:
// CreatureCmd/<GainBlock>d__18::MoveNext RVA 0x3eaec0, IL_0101-IL_0139
// enters Hook.ModifyBlock; MultiplayerScalingModel's multiplicative body is
// RVA 0x8fb24, IL_000c-IL_0075. Creature::GainBlockInternal RVA 0x11d64c,
// IL_0024-IL_004a computes Decimal(current + amount), Math.Min with
// 999,999,999, explicit Int32, then set_Block.
const QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP: i32 = crate::engine::monsters::MONSTER_BLOCK_CAP;

fn queen_burn_bright_listener_projection_is_exact(state: &crate::hot::HotState) -> bool {
    [
        PowerId::Vicious,
        PowerId::Shroud,
        PowerId::SleightOfFlesh,
        PowerId::SwordSage,
    ]
    .into_iter()
    .all(|power| {
        state
            .powers
            .get(power)
            .is_none_or(|slot| slot.wire == SlotWire::Int && slot.value > 0)
    }) && crate::engine::damage::player_type_one_listener_order_is_exact(state)
}

fn queen_burn_bright_block_listener_projection_is_exact(state: &crate::hot::HotState) -> bool {
    let juggernaut_active = match state.powers.get(PowerId::Juggernaut) {
        None => false,
        Some(slot) if slot.wire == SlotWire::Int && slot.value > 0 => true,
        Some(_) => return false,
    };
    let beacon_active = match state.powers.get(PowerId::BeaconOfHope) {
        None => false,
        Some(slot) if slot.wire == SlotWire::Int && slot.value == 1 => true,
        Some(_) => return false,
    };
    let order = state.fanouts.after_block_gained_order();
    let active_len = usize::from(juggernaut_active) + usize::from(beacon_active);
    order.len() == active_len
        && (!juggernaut_active || order.contains(&PowerId::Juggernaut))
        && (!beacon_active || order.contains(&PowerId::BeaconOfHope))
        && order.iter().all(|power| match power {
            PowerId::Juggernaut => juggernaut_active,
            PowerId::BeaconOfHope => beacon_active,
            _ => false,
        })
}

/// Exact Queen Burn Bright body shared by ordinary and retained-empty routes.
#[cfg_attr(not(test), allow(dead_code))]
fn queen_burn_bright_apply(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.args
        != [
            CompiledArg::I(ctx.tier(mc::QUEEN_BURN_BRIGHT_STRENGTH)),
            CompiledArg::I(20),
        ]
        || !queen_burn_bright_row_is_exact(ctx)
        || ctx.actor != 1
        || ctx
            .state
            .monsters
            .get(ctx.actor)
            .is_none_or(|monster| monster.loop_pos != 2)
        || !crate::engine::monsters::queen_roster_state_is_valid(ctx.state)
        || ctx.state.multiplayer_ally_key != 0
        || ctx.state.monsters.iter().any(|monster| {
            monster
                .powers
                .get(PowerId::Strength)
                .is_some_and(|slot| slot.wire != SlotWire::Int || slot.value == 0)
        })
        || !queen_burn_bright_listener_projection_is_exact(ctx.state)
        || !queen_burn_bright_block_listener_projection_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation"));
    }
    let teammate_uids: Vec<u32> = ctx
        .state
        .monsters
        .iter()
        .filter(|monster| monster.kind != MonsterKind::Queen && monster.hp > 0)
        .map(|monster| monster.uid)
        .collect();
    let queen_uid = ctx.state.monsters[1].uid;

    fn apply(
        state: &mut crate::hot::HotState,
        teammate_uids: &[u32],
        queen_uid: u32,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let live_teammates: Vec<u32> = state
            .monsters
            .iter()
            .filter(|monster| monster.kind != MonsterKind::Queen && monster.hp > 0)
            .map(|monster| monster.uid)
            .collect();
        if live_teammates != teammate_uids
            || state.monsters.get(1).map(|monster| monster.uid) != Some(queen_uid)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Queen Burn Bright frozen roster",
            ));
        }
        let strength_plan: Vec<(u32, i32)> = teammate_uids
            .iter()
            .filter_map(|uid| {
                state
                    .monsters
                    .iter()
                    .find(|monster| monster.uid == *uid && monster.hp > 0)
                    .map(|monster| {
                        crate::engine::damage::checked_monster_strength_successor(
                            monster,
                            1,
                            "Queen Burn Bright Amalgam Strength",
                        )
                        .map(|updated| (*uid, updated))
                    })
            })
            .collect::<Result<_, _>>()?;
        if state.monsters[1].queen_burn_bright_retained() {
            state.monsters_mut()[1].set_queen_burn_bright_retained(false);
        }
        for (teammate_uid, planned_strength) in &strength_plan {
            if state.history.over {
                break;
            }
            let Some(target) = state
                .monsters
                .iter()
                .position(|monster| monster.uid == *teammate_uid && monster.hp > 0)
            else {
                continue;
            };
            let upkeep = state.fanouts.misery_attachment_upkeep();
            let monster = &mut state.monsters_mut()[target];
            // The Queen buffs a TEAMMATE, so the applier is the Queen's own
            // creature, not the recipient's (`<DebilitatingSmogMove>d__13`
            // `0x3703a4` IL_00ab-IL_00c5 is the same shape).
            crate::engine::damage::write_monster_strength(
                monster,
                *planned_strength,
                crate::hot::Applier::Monster(queen_uid),
                upkeep,
            );
            note_power(
                events,
                Subject::Monster(monster.uid),
                PowerId::Strength,
                *planned_strength,
            );
            if !queen_burn_bright_listener_projection_is_exact(state) {
                return Err(EngineRefusal::MalformedArgs(
                    "Queen Burn Bright after-power-amount-changed suffix",
                ));
            }
        }
        if !state.history.over
            && let Some(owner) = state
                .monsters
                .iter()
                .position(|monster| monster.uid == queen_uid && monster.hp > 0)
        {
            let queen = &mut state.monsters_mut()[owner];
            queen.block = (i64::from(queen.block) + 20)
                .min(i64::from(QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP))
                as i32;
            if !queen_burn_bright_block_listener_projection_is_exact(state) {
                return Err(EngineRefusal::MalformedArgs(
                    "Queen Burn Bright after-block-gained suffix",
                ));
            }
        }
        Ok(())
    }

    // Native walks the frozen AfterBlockGained order after the trailing Block.
    // Its exact current listeners are body-inert for Queen, but rehearse the
    // complete awaited body so any late suffix refusal remains atomic.
    let mut probe = ctx.state.clone();
    apply(&mut probe, &teammate_uids, queen_uid, &mut Vec::new())?;
    apply(ctx.state, &teammate_uids, queen_uid, ctx.events)
}

#[cfg(test)]
fn queen_burn_bright_living_foundation(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    queen_burn_bright_apply(ctx)
}

/// `("queen_puppet_strings", 3)` — Queen PUPPET STRINGS.
///
/// Current-build authority: v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`;
/// `PuppetStringsMove` RVA `0xbc58c`, `MoveNext` RVA `0x3678b0`, Chains
/// access/init `0xa034b/0xa034e/0xa0351`, and its draw/play/side-end callbacks
/// `0xa0360/0xa03ac/0xa040b/0xa0440`.
///
/// Python: `monster_act` (frozen, deleted #2827). It validates the fixed Queen roster and the
/// card-affliction overlap state, then applies a fresh Chains of Binding
/// singleton to the player. The current branch also pins both
/// acquisition-ordered listener carriers before installing Chains(3).
/// The shared draw/play/
/// side-end foundations own Bound creation, latch, denial, and five-pile
/// cleanup; this body rehearses registration and power publication atomically.
///
/// **Non-exact piles promote here (#3404)**, exactly as Hex's application
/// does (#2959, [`crate::engine::cards::apply_hex_power`]). `exact_piles` is
/// the solver's pile-identity promotion, not a native field, and nothing
/// Chains of Binding does reads a card-group ordering property: its
/// `<AfterCardDrawn>d__8::MoveNext` (RVA `0x336cec`) afflicts the one drawn
/// `card` object (`Affliction<Bound>` / `CanAfflict`, IL_005a-IL_0065) after
/// counting this turn's `CardAfflictedEntry` rows (IL_0071-IL_0091) against
/// `Amount` (IL_0097-IL_0099); `BeforeCardPlayed` (`0xa03ac`) and
/// `ShouldPlay` (`0xa040b`) read the played card's own `Affliction` and the
/// power's `boundCardPlayed` latch; `BeforeSideTurnEnd` (`0xa0440`) clears
/// every card's affliction over `PlayerCombatState.AllCards`
/// (IL_004a-IL_0077), an order-free walk. Which card is drawn is the
/// physical Draw order, which the hot piles carry in either mode. So a root
/// opened without a tied deck group (no prior promotion) is admitted at
/// `loop_pos` 0 and the Chains writer promotes before any Bound can exist;
/// every later Chains gate ([`crate::engine::cards::chains_draw_state_is_exact`])
/// keeps requiring the promoted piles.
pub(crate) fn queen_puppet_strings(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("queen_puppet_strings"));
    };
    if ctx.actor != 1
        || !crate::engine::monsters::queen_roster_state_is_valid(ctx.state)
        || ctx.state.monsters[1].loop_pos != 0
        || ctx.state.multiplayer_ally_key != 0
        || ctx.state.powers.value(PowerId::ChainsOfBinding) != 0
        || ctx.state.powers.value(PowerId::HexPower) != 0
        || ctx.state.ringing()
        || ctx.state.bound_afflictions_this_turn() != 0
        || ctx.state.bound_card_played()
        || !crate::engine::cards::hex_card_affliction_application_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("Queen Puppet state"));
    }
    fn apply(
        state: &mut crate::hot::HotState,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        if state.history.over || state.monsters[1].hp <= 0 {
            return Ok(());
        }
        if !state
            .fanouts
            .register_after_card_drawn(PowerId::ChainsOfBinding)
            || !state
                .fanouts
                .register_before_side_turn_end(PowerId::ChainsOfBinding)
        {
            return Err(EngineRefusal::MalformedArgs("Chains listener capacity"));
        }
        state.exact_piles = true;
        state.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
        note_power(events, Subject::Player, PowerId::ChainsOfBinding, 3);
        Ok(())
    }
    let mut probe = ctx.state.clone();
    apply(&mut probe, &mut Vec::new())?;
    apply(ctx.state, ctx.events)
}

/// `("queen_youre_mine", 99)` — after validating the fixed Queen/Amalgam
/// roster, apply fresh Frail, Weak, then Vulnerable in native serial order.
pub(crate) fn queen_youre_mine(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(99)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("queen_youre_mine"));
    };
    if ctx.actor != 1 || !crate::engine::monsters::queen_roster_state_is_valid(ctx.state) {
        return Err(EngineRefusal::MalformedArgs("queen roster"));
    }
    let queen = &ctx.state.monsters[1];
    if ctx.state.history.over || queen.hp <= 0 {
        return Ok(());
    }
    for (power, fresh) in [
        (PowerId::PlayerFrail, PowerId::PlayerFrailFresh),
        (PowerId::PlayerWeak, PowerId::PlayerWeakFresh),
        (PowerId::PlayerVuln, PowerId::PlayerVulnFresh),
    ] {
        apply_player_duration_affliction(ctx.state, power, fresh, 99, true, ctx.events)?;
    }
    Ok(())
}

/// `("scream", damage, hits, vulnerable)` — attack, then apply fresh player
/// Vulnerable while Soul Fysh and combat remain live. The damage is
/// `SoulFysh::get_ScreamDamage` RVA `0xbecb1`, `GetValueIfAscension(9, 15, 13)`
/// at the fight's tier (#2828).
pub(crate) fn scream(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("scream"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::SOUL_FYSH_SCREAM_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("scream"));
    }
    if !crate::engine::monsters::soul_fysh_move_state_is_valid(ctx.state, ctx.actor, 4, 1) {
        return Err(EngineRefusal::MalformedArgs("scream owner/state"));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerVuln,
            PowerId::PlayerVulnFresh,
            3,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `("soul_siphon", -2, -2, 2)`.
///
/// Python: `monster_act` (frozen, deleted #2827). It serially applies player Strength -2,
/// player Dexterity -2, then owner Strength +2, with an independent ending /
/// recipient-liveness check on every command.
///
/// Authenticate the Matriarch row and execute the three independently gated
/// power commands in native order. Plating and Asleep remain separate entry
/// refusals.
pub(crate) fn soul_siphon(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(-2), CompiledArg::I(-2), CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("soul_siphon"));
    };
    if ctx.state.monsters[ctx.actor].kind != MonsterKind::LagavulinMatriarch {
        return Err(EngineRefusal::MalformedArgs("soul_siphon owner"));
    }
    let owner_strength = if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        Some(crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[ctx.actor],
            2,
            "soul_siphon Strength",
        )?)
    } else {
        None
    };
    if !ctx.state.history.over && ctx.state.hp > 0 {
        apply_owner_strength(ctx.state, -2, ctx.events)?;
    }
    if !ctx.state.history.over && ctx.state.hp > 0 {
        let updated = ctx
            .state
            .powers
            .value(PowerId::Dexterity)
            .checked_sub(2)
            .ok_or(EngineRefusal::CounterOverflow("soul_siphon Dexterity"))?;
        ctx.state
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, updated);
        note_power(ctx.events, Subject::Player, PowerId::Dexterity, updated);
    }
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let updated =
            owner_strength.ok_or(EngineRefusal::MalformedArgs("soul_siphon Strength plan"))?;
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

/// `("test_subject_burning_growl", burns, strength)`.
///
/// Python: `monster_act` (frozen, deleted #2827). It serially creates discard-bottom Burns,
/// awaiting the complete generated-card hook chain after each, then rechecks
/// the reviving owner before applying Strength. #2828: `TestSubject::
/// get_BurningGrowlBurnCount` RVA `0xc01bc` is `GetValueIfAscension(9, 5, 3)`
/// and `get_BurningGrowlStrengthGain` RVA `0xc01c7` is `(9, 3, 2)`, both at the
/// fight's tier.
///
pub(crate) fn test_subject_burning_growl(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    fn apply(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
        let [CompiledArg::I(burns), CompiledArg::I(strength)] = ctx.args else {
            return Err(EngineRefusal::MalformedArgs("test_subject_burning_growl"));
        };
        let (burns, strength) = (*burns, *strength);
        if (burns, strength)
            != (
                ctx.tier(mc::TEST_SUBJECT_BURNING_GROWL_BURN_COUNT),
                ctx.tier(mc::TEST_SUBJECT_BURNING_GROWL_STRENGTH_GAIN),
            )
        {
            return Err(EngineRefusal::MalformedArgs("test_subject_burning_growl"));
        }
        let burns = usize::try_from(burns)
            .map_err(|_| EngineRefusal::MalformedArgs("test_subject_burning_growl"))?;
        let strength = i32::try_from(strength)
            .map_err(|_| EngineRefusal::MalformedArgs("test_subject_burning_growl"))?;
        if !crate::engine::monsters::test_subject_live_move_state_is_valid(
            ctx.state, ctx.actor, 2, 6,
        ) {
            return Err(EngineRefusal::MalformedArgs(
                "test_subject_burning_growl owner/state",
            ));
        }
        let planned_strength = crate::engine::damage::checked_monster_strength_successor(
            &ctx.state.monsters[ctx.actor],
            strength,
            "test_subject_burning_growl Strength",
        )?;
        inject_generated_null_bottom(
            ctx.state,
            ctx.catalog,
            CardIdentity {
                id: CardId::Burn,
                upgrade: 0,
                enchantment: None,
            },
            burns,
            ctx.events,
        )?;
        if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
            let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
            let owner = &mut ctx.state.monsters_mut()[ctx.actor];
            crate::engine::damage::write_monster_self_strength(owner, planned_strength, upkeep);
            note_power(
                ctx.events,
                Subject::Monster(owner.uid),
                PowerId::Strength,
                planned_strength,
            );
        }
        Ok(())
    }

    let mut probe_state = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply(&mut MoveCtx {
        state: &mut probe_state,
        catalog: ctx.catalog,
        actor: ctx.actor,
        args: ctx.args,
        events: &mut probe_events,
    })?;
    apply(ctx)
}

/// `("test_subject_multi_claw", damage, base_hits)`.
///
/// Python: `monster_act` (frozen, deleted #2827), resumed by
/// `_finish_monster_move_after_attack` and validated by
/// `_advance_monster_move_frame`. It attacks for
/// `base_hits + extra_multi_claw_count`, lets
/// Painful Stabs create one Wound per hit, and increments the signed Int32
/// counter after the awaited attack even for a retained Adaptable corpse.
///
pub(crate) fn test_subject_multi_claw(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    fn apply(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
        // #2828: `TestSubject::get_MultiClawDamage` RVA `0xc019b` is
        // `GetValueIfAscension(9, 11, 10)`; the base hit count is untiered.
        let [CompiledArg::I(damage), CompiledArg::I(3)] = ctx.args else {
            return Err(EngineRefusal::MalformedArgs("test_subject_multi_claw"));
        };
        let damage = *damage;
        if damage != ctx.tier(mc::TEST_SUBJECT_MULTI_CLAW_DAMAGE) {
            return Err(EngineRefusal::MalformedArgs("test_subject_multi_claw"));
        }
        if !crate::engine::monsters::test_subject_live_move_state_is_valid(
            ctx.state, ctx.actor, 1, 3,
        ) {
            return Err(EngineRefusal::MalformedArgs(
                "test_subject_multi_claw owner/state",
            ));
        }
        let extra = ctx.state.monsters[ctx.actor].test_subject_extra_multi_claw_count();
        let next = extra.checked_add(1).ok_or(EngineRefusal::CounterOverflow(
            "test subject extra multi claw count",
        ))?;
        let hits = i64::from(extra)
            .checked_add(3)
            .ok_or(EngineRefusal::CounterOverflow(
                "test subject multi claw hits",
            ))?;
        let positive = monster_attack_player_positive_results_with_catalog(
            ctx.state,
            ctx.catalog,
            ctx.actor,
            damage,
            hits,
            ctx.events,
        )?;
        if !ctx.state.history.over {
            let count: usize = positive
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("Painful Stabs Wounds"))?;
            inject_generated_null_bottom(
                ctx.state,
                ctx.catalog,
                CardIdentity {
                    id: CardId::Wound,
                    upgrade: 0,
                    enchantment: None,
                },
                count,
                ctx.events,
            )?;
        }
        let stored =
            ctx.state.monsters_mut()[ctx.actor].set_test_subject_extra_multi_claw_count(next);
        debug_assert!(stored);
        Ok(())
    }

    let mut probe_state = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply(&mut MoveCtx {
        state: &mut probe_state,
        catalog: ctx.catalog,
        actor: ctx.actor,
        args: ctx.args,
        events: &mut probe_events,
    })?;
    apply(ctx)
}

/// `("test_subject_respawn",)`.
///
/// Python: the dead-owner pre-dispatch route at `monster_act` (frozen Python, deleted #2827) calls
/// `_test_subject_respawn`; reaching `monster_act`'s ordinary living
/// dispatch arm is an invariant failure. Respawn advances the private counter,
/// clears the revive latch, replaces HP/max HP and form powers, and jumps to
/// the next form's loop position.
///
/// This in-place form transition is reached only by the sequenced dead-owner
/// dispatch. Death retention and all companion moves enter admission in the
/// same unit, so a live loop can never invoke RESPawn directly.
pub(crate) fn test_subject_respawn(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("test_subject_respawn"));
    };
    if ctx.actor != 0
        || !crate::engine::monsters::test_subject_state_is_valid(ctx.state)
        || ctx.state.monsters[0].hp > 0
        || !ctx.state.monsters[0].test_subject_adaptable_reviving()
        || ctx.state.monsters[0].powers.value(PowerId::Adaptable) != 1
    {
        return Err(EngineRefusal::MalformedArgs(
            "test_subject_respawn owner/state",
        ));
    }
    let old_form = ctx.state.monsters[0].test_subject_respawns();
    // `<RespawnMove>d__74` revives at `get_SecondFormHp` / `get_ThirdFormHp`
    // (IL_021d / IL_02fc), each `GetValueIfAscension(8, …)` (#2539).
    let (new_form, loop_pos) = match old_form {
        0 => (1, 3),
        1 => (2, 4),
        _ => {
            return Err(EngineRefusal::MalformedArgs("test_subject_respawn form"));
        }
    };
    let hp = crate::engine::monsters::test_subject_form_hp(ctx.state, usize::from(new_form))
        .ok_or(EngineRefusal::MalformedArgs("test_subject_respawn form"))?;

    let owner = &mut ctx.state.monsters_mut()[0];
    let stored = owner.set_test_subject_respawns(new_form);
    debug_assert!(stored);
    owner.set_test_subject_adaptable_reviving(false);
    owner.max_hp = hp;
    owner.hp = hp;
    owner.loop_pos = loop_pos;
    if new_form == 1 {
        owner.powers.set(PowerId::PainfulStabs, SlotWire::Int, 1);
        note_power(
            ctx.events,
            Subject::Monster(owner.uid),
            PowerId::PainfulStabs,
            1,
        );
    } else {
        owner.powers.set(PowerId::Nemesis, SlotWire::Int, 1);
        owner.powers.set(PowerId::Adaptable, SlotWire::Int, 0);
        owner.powers.set(PowerId::PainfulStabs, SlotWire::Int, 0);
        for (power, amount) in [
            (PowerId::Nemesis, 1),
            (PowerId::Adaptable, 0),
            (PowerId::PainfulStabs, 0),
        ] {
            note_power(ctx.events, Subject::Monster(owner.uid), power, amount);
        }
    }
    debug_assert!(crate::engine::monsters::test_subject_state_is_valid(
        ctx.state
    ));
    Ok(())
}

/// `("test_subject_skull_bash", damage, hits, vulnerable)`.
///
/// Python: `monster_act` (frozen, deleted #2827) and `_finish_monster_move_after_attack`'s
/// resumed suffix. It validates the frozen Test Subject form,
/// attacks, then applies fresh player Vulnerable if the player remains alive.
/// Unlike ordinary attack suffixes, this branch does not require the actor to
/// remain alive after the attack.
///
/// The exact form/respawn validator authenticates the fixed successor before
/// damage; fresh Vulnerable is then applied after the attack even if Thorns
/// retained the owner corpse, provided the player and combat remain live.
pub(crate) fn test_subject_skull_bash(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    // #2828: `TestSubject::get_SkullBashDamage` RVA `0xc018e` is
    // `GetValueIfAscension(9, 16, 14)`, at the fight's tier.
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("test_subject_skull_bash"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::TEST_SUBJECT_SKULL_BASH_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("test_subject_skull_bash"));
    }
    if !crate::engine::monsters::test_subject_live_move_state_is_valid(ctx.state, ctx.actor, 0, 2) {
        return Err(EngineRefusal::MalformedArgs(
            "test_subject_skull_bash owner/state",
        ));
    }
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerVuln,
            PowerId::PlayerVulnFresh,
            1,
            true,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `("waterfall_about",)` — freeze Steam Pressure for EXPLODE.
///
/// Python: `monster_act` (frozen, deleted #2827). It snapshots Steam Pressure into the private
/// eruption-damage slot, removes the pressure, and parks the loop on EXPLODE.
///
pub(crate) fn waterfall_about(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_about"));
    };
    require_waterfall_owner(ctx)?;
    let monster = &mut ctx.state.monsters_mut()[ctx.actor];
    let pressure = monster.powers.value(PowerId::SteamPressure);
    let uid = monster.uid;
    let written = monster.set_waterfall_steam_eruption_damage(pressure);
    debug_assert!(written);
    monster.powers.set(PowerId::SteamPressure, SlotWire::Int, 0);
    let stored = monster.set_pressure_buildup_idx(7);
    debug_assert!(stored);
    note_power(ctx.events, Subject::Monster(uid), PowerId::SteamPressure, 0);
    Ok(())
}

/// `("waterfall_explode",)` — attack for the frozen amount, then self-kill.
///
/// Python: `monster_act` (frozen, deleted #2827). It attacks for the snapped eruption damage,
/// then force-kills the still-live owner through the unblocked monster death
/// cascade and remains parked on EXPLODE.
///
pub(crate) fn waterfall_explode(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_explode"));
    };
    require_waterfall_owner(ctx)?;
    let damage = i64::from(ctx.state.monsters[ctx.actor].waterfall_steam_eruption_damage());
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    // Player death does not cancel the enclosing native move body: the
    // awaited AttackCommand completes with `false`, then Kill(false) still
    // runs. Only a retaliation that left this owner genuinely dead skips it.
    if ctx.state.monsters[ctx.actor].hp <= 0 {
        return Ok(());
    }
    let monster = &ctx.state.monsters[ctx.actor];
    let self_kill = i64::from(monster.hp) + i64::from(monster.block);
    // The self-kill's death runs `Hook.AfterDeath`, so it keeps the move's
    // catalog for Gremlin Horn's Draw (#3166).
    crate::engine::damage::damage_monster_after_catalog_auth(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        DotNetDecimal::from_i64(self_kill),
        false,
        false,
        ctx.events,
    )?;
    Ok(())
}

/// `("waterfall_pressure_gun", 5, 3)` — attack with and grow the live gun.
///
/// Python: `monster_act` (frozen, deleted #2827). It attacks with the private live gun-damage
/// value, then grows that value, Steam Pressure, and the buildup index while
/// owner and combat remain live.
///
pub(crate) fn waterfall_pressure_gun(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(5), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_pressure_gun"));
    };
    require_waterfall_owner(ctx)?;
    let damage = i64::from(ctx.state.monsters[ctx.actor].pressure_gun_damage);
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let updated = ctx.state.monsters[ctx.actor]
            .pressure_gun_damage
            .checked_add(5)
            .ok_or(EngineRefusal::CounterOverflow("pressure gun damage"))?;
        ctx.state.monsters_mut()[ctx.actor].pressure_gun_damage = updated;
        add_waterfall_pressure(ctx, 3)?;
    }
    Ok(())
}

/// `("waterfall_pressure_up", damage, 3)` — attack, then build pressure.
///
/// Python: `monster_act` (frozen, deleted #2827). It attacks, then increments Steam Pressure
/// and its private buildup index while owner and combat remain live; its fixed
/// successor jumps back to STOMP. The damage is
/// `WaterfallGiant::get_PressureUpDamage` RVA `0xc4d48`,
/// `GetValueIfAscension(9, 14, 13)` at the fight's tier (#2828).
///
pub(crate) fn waterfall_pressure_up(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_pressure_up"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::WATERFALL_GIANT_PRESSURE_UP_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("waterfall_pressure_up"));
    }
    require_waterfall_owner(ctx)?;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        add_waterfall_pressure(ctx, 3)?;
    }
    Ok(())
}

/// `("waterfall_pressurize", amount)` — build pressure without a liveness
/// tail.
///
/// Python: `monster_act` (frozen, deleted #2827). It adds Steam Pressure and increments the
/// buildup index with no liveness gate. The amount is
/// `WaterfallGiant::get_PressurizeAmount` RVA `0xc4d21`,
/// `GetValueIfAscension(9, 20, 15)` at the fight's tier (#2828).
///
pub(crate) fn waterfall_pressurize(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_pressurize"));
    };
    let amount = *amount;
    if amount != ctx.tier(mc::WATERFALL_GIANT_PRESSURIZE_AMOUNT) {
        return Err(EngineRefusal::MalformedArgs("waterfall_pressurize"));
    }
    let amount =
        i32::try_from(amount).map_err(|_| EngineRefusal::MalformedArgs("waterfall_pressurize"))?;
    require_waterfall_owner(ctx)?;
    add_waterfall_pressure(ctx, amount)
}

/// `("waterfall_ram", damage, 3)` — attack, then build pressure.
///
/// Python: `monster_act` (frozen, deleted #2827). It attacks, then increments Steam Pressure
/// and the buildup index while owner and combat remain live. The damage is
/// `WaterfallGiant::get_RamDamage` RVA `0xc4d3b`,
/// `GetValueIfAscension(9, 11, 10)` at the fight's tier (#2828).
///
pub(crate) fn waterfall_ram(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_ram"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::WATERFALL_GIANT_RAM_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("waterfall_ram"));
    }
    require_waterfall_owner(ctx)?;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        add_waterfall_pressure(ctx, 3)?;
    }
    Ok(())
}

/// `("waterfall_siphon", heal, 3)` — clamp-heal, then build pressure.
///
/// Python: `monster_act` (frozen, deleted #2827). While owner and combat remain live it clamps
/// a self-heal, then increments Steam Pressure and the buildup index. The heal
/// is `WaterfallGiant::get_SiphonHeal` RVA `0xc4d15`,
/// `GetValueIfAscension(8, 15, 10)` — gated at A8, not A9 — at the fight's
/// tier (#2828).
///
pub(crate) fn waterfall_siphon(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(heal), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_siphon"));
    };
    let heal = *heal;
    if heal != ctx.tier(mc::WATERFALL_GIANT_SIPHON_HEAL) {
        return Err(EngineRefusal::MalformedArgs("waterfall_siphon"));
    }
    let heal = i32::try_from(heal).map_err(|_| EngineRefusal::MalformedArgs("waterfall_siphon"))?;
    require_waterfall_owner(ctx)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        monster.hp = monster
            .hp
            .checked_add(heal)
            .ok_or(EngineRefusal::CounterOverflow("waterfall siphon heal"))?
            .min(monster.max_hp);
        add_waterfall_pressure(ctx, 3)?;
    }
    Ok(())
}

/// `("waterfall_stomp", damage, 1, 3)` — attack, fresh Weak, then pressure.
///
/// Python: `monster_act` (frozen, deleted #2827). It attacks, applies fresh player Weak, then
/// increments Steam Pressure and the buildup index while owner and combat
/// remain live. The damage is `WaterfallGiant::get_StompDamage` RVA `0xc4d2e`,
/// `GetValueIfAscension(9, 16, 15)` at the fight's tier (#2828).
///
pub(crate) fn waterfall_stomp(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(1), CompiledArg::I(3)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("waterfall_stomp"));
    };
    let damage = *damage;
    if damage != ctx.tier(mc::WATERFALL_GIANT_STOMP_DAMAGE) {
        return Err(EngineRefusal::MalformedArgs("waterfall_stomp"));
    }
    require_waterfall_owner(ctx)?;
    monster_attack_player_with_catalog(ctx.state, ctx.catalog, ctx.actor, damage, 1, ctx.events)?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        apply_player_duration_affliction(
            ctx.state,
            PowerId::PlayerWeak,
            PowerId::PlayerWeakFresh,
            1,
            true,
            ctx.events,
        )?;
        add_waterfall_pressure(ctx, 3)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CatalogBuilder, CompiledMove};
    use crate::engine::admission::{AdmissionRefusal, MissingCapability, admit};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, RngStream};
    use crate::ids::MonsterKind;
    use serde_json::Value;

    const FIXTURE: &str = include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    fn admit_with_monster(kind: MonsterKind) -> Result<(), AdmissionRefusal> {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert("kind".to_owned(), Value::from(kind.as_str()));
        if kind == MonsterKind::WaterfallGiant {
            document.monsters[0].insert("hp".to_owned(), Value::from(250));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(250));
            document.monsters[0].insert("pressure_gun_damage".to_owned(), Value::from(23));
        }
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog)
    }

    /// Mechanically enumerate every generated loop row owned by this file.
    fn owned_rows() -> Vec<(MonsterKind, CompiledMove)> {
        let mut builder = CatalogBuilder::new();
        for kind in MonsterKind::ALL {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let rows: Vec<_> = MonsterKind::ALL
            .into_iter()
            .flat_map(|monster| {
                catalog
                    .moves(monster)
                    .iter()
                    .copied()
                    .filter(|entry| OWNED.contains(&entry.kind))
                    .map(move |entry| (monster, entry))
            })
            .collect();
        for kind in OWNED {
            assert!(
                rows.iter().any(|(_, entry)| entry.kind == kind),
                "no monster move table uses {:?}",
                kind.as_str()
            );
        }
        rows
    }

    fn soul_fysh_state(loop_pos: i32, intangible: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let mut owner = HotMonster::new(MonsterKind::SoulFysh, 221);
        owner.max_hp = 221;
        owner.loop_pos = loop_pos;
        if intangible != 0 {
            owner
                .powers
                .set(PowerId::Intangible, SlotWire::Int, intangible);
        }
        state.monsters_mut().push(owner);
        state
    }

    fn aeonglass_fixture(loop_pos: i32, counter: i32) -> (crate::catalog::Catalog, HotState) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Aeonglass).unwrap();
        builder
            .intern_reachable(CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 300;
        state.max_hp = 300;
        state.exact_piles = true;
        let mut boss = HotMonster::new(MonsterKind::Aeonglass, 535);
        boss.max_hp = 535;
        boss.loop_pos = loop_pos;
        boss.powers.set(PowerId::Artifact, SlotWire::Int, 3);
        assert!(boss.set_aeonglass_additional_strength(counter));
        assert!(boss.set_aeonglass_wither_upgrade_count(counter));
        state.monsters_mut().push(boss);
        assert!(state.fanouts.set_withering_cards_left(6));
        (catalog, state)
    }

    fn push_wither(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        pile: PileId,
        uid: u32,
        growth: i32,
    ) {
        let atom = catalog
            .atom(&CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        state.piles.get_mut(pile).make_mut().push(HotCard {
            uid,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.next_card_uid = state.next_card_uid.max(uid + 1);
        if growth != 0 {
            assert_eq!(
                state.card_states.add_damage_growth(uid, growth),
                Some(growth)
            );
        }
    }

    fn insatiable_state() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let mut owner = HotMonster::new(
            MonsterKind::TheInsatiable,
            crate::engine::monsters::THE_INSATIABLE_HP,
        );
        owner.max_hp = crate::engine::monsters::THE_INSATIABLE_HP;
        state.monsters_mut().push(owner);
        state
    }

    fn queen_burn_bright_fixture() -> (HotState, crate::catalog::Catalog, CompiledMove) {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_monster(MonsterKind::TorchHeadAmalgam)
            .unwrap();
        builder.intern_monster(MonsterKind::Queen).unwrap();
        let catalog = builder.build();
        let row = catalog
            .moves(MonsterKind::Queen)
            .iter()
            .find(|row| row.kind == MoveKind::QueenBurnBright)
            .copied()
            .unwrap();

        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, 211);
        amalgam.max_hp = 211;
        amalgam.loop_pos = 2;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = HotMonster::new(MonsterKind::Queen, 419);
        queen.max_hp = 419;
        queen.loop_pos = 2;
        queen.slot = 1;
        queen.uid = 1;
        state.monsters_mut().extend([amalgam, queen]);
        (state, catalog, row)
    }

    fn run_queen_burn_bright_foundation(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        actor: usize,
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        queen_burn_bright_living_foundation(&mut MoveCtx {
            state,
            catalog,
            actor,
            args,
            events,
        })
    }

    #[test]
    fn queen_puppet_strings_requires_its_exact_row_and_preserves_ordinary_cards() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_monster(MonsterKind::TorchHeadAmalgam)
            .unwrap();
        builder.intern_monster(MonsterKind::Queen).unwrap();
        let strike = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let row = catalog
            .moves(MonsterKind::Queen)
            .iter()
            .find(|row| row.kind == MoveKind::QueenPuppetStrings)
            .copied()
            .unwrap();
        let (mut state, _, _) = queen_burn_bright_fixture();
        state.monsters_mut()[0].loop_pos = 0;
        state.monsters_mut()[1].loop_pos = 0;
        state.exact_piles = true;
        state.next_card_uid = 1;
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(crate::hot::HotCard {
                uid: 0,
                atom: strike,
                flags: 0,
            });

        queen_puppet_strings(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(state.powers.value(PowerId::ChainsOfBinding), 3);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].flags, 0);

        let existing = [
            PowerId::Speedster,
            PowerId::CorrosiveWave,
            PowerId::Automation,
            PowerId::Cacophony,
            PowerId::Pagestorm,
            PowerId::Iteration,
        ];
        let (mut full_order, _, _) = queen_burn_bright_fixture();
        full_order.monsters_mut()[0].loop_pos = 0;
        full_order.monsters_mut()[1].loop_pos = 0;
        full_order.exact_piles = true;
        assert!(full_order.fanouts.set_after_card_drawn_order(&existing));
        queen_puppet_strings(&mut MoveCtx {
            state: &mut full_order,
            catalog: &catalog,
            actor: 1,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(
            full_order.fanouts.after_card_drawn_order(),
            &[
                PowerId::Speedster,
                PowerId::CorrosiveWave,
                PowerId::Automation,
                PowerId::Cacophony,
                PowerId::Pagestorm,
                PowerId::Iteration,
                PowerId::ChainsOfBinding,
            ]
        );

        let (mut at_capacity, _, _) = queen_burn_bright_fixture();
        at_capacity.monsters_mut()[0].loop_pos = 0;
        at_capacity.monsters_mut()[1].loop_pos = 0;
        at_capacity.exact_piles = true;
        assert!(at_capacity.fanouts.set_after_card_drawn_order(&[
            PowerId::Speedster,
            PowerId::CorrosiveWave,
            PowerId::Automation,
            PowerId::Cacophony,
            PowerId::Pagestorm,
            PowerId::Iteration,
            PowerId::Speedster,
        ]));
        let before_capacity = at_capacity.clone();
        let mut capacity_events = vec![crate::engine::Event::TurnBegan { turn: 91 }];
        let before_events = capacity_events.clone();
        assert_eq!(
            queen_puppet_strings(&mut MoveCtx {
                state: &mut at_capacity,
                catalog: &catalog,
                actor: 1,
                args: catalog.args(row.args),
                events: &mut capacity_events,
            }),
            Err(EngineRefusal::MalformedArgs("Chains listener capacity"))
        );
        assert_eq!(at_capacity, before_capacity);
        assert_eq!(capacity_events, before_events);

        let (mut wrong_row, _, _) = queen_burn_bright_fixture();
        wrong_row.monsters_mut()[0].loop_pos = 1;
        wrong_row.monsters_mut()[1].loop_pos = 1;
        wrong_row.exact_piles = true;
        let before = wrong_row.clone();
        assert_eq!(
            queen_puppet_strings(&mut MoveCtx {
                state: &mut wrong_row,
                catalog: &catalog,
                actor: 1,
                args: catalog.args(row.args),
                events: &mut Vec::new(),
            }),
            Err(EngineRefusal::MalformedArgs("Queen Puppet state"))
        );
        assert_eq!(wrong_row, before);
    }

    /// #3404 — `QueenPuppetStrings` on non-exact piles is the promotion, as Hex's
    /// application is (#2959): Chains lands and the piles become exact, with
    /// no card touched. A refused application (listener capacity) promotes
    /// nothing.
    #[test]
    fn queen_puppet_strings_on_non_exact_piles_promotes_with_the_chains_write() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_monster(MonsterKind::TorchHeadAmalgam)
            .unwrap();
        builder.intern_monster(MonsterKind::Queen).unwrap();
        let strike = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let row = catalog
            .moves(MonsterKind::Queen)
            .iter()
            .find(|row| row.kind == MoveKind::QueenPuppetStrings)
            .copied()
            .unwrap();
        let fresh = || {
            let (mut state, _, _) = queen_burn_bright_fixture();
            state.monsters_mut()[0].loop_pos = 0;
            state.monsters_mut()[1].loop_pos = 0;
            state.next_card_uid = 2;
            for uid in 0..2 {
                state
                    .piles
                    .get_mut(PileId::Draw)
                    .make_mut()
                    .push(crate::hot::HotCard {
                        uid,
                        atom: strike,
                        flags: 0,
                    });
            }
            assert!(!state.exact_piles);
            state
        };
        let run = |state: &mut HotState| {
            queen_puppet_strings(&mut MoveCtx {
                state,
                catalog: &catalog,
                actor: 1,
                args: catalog.args(row.args),
                events: &mut Vec::new(),
            })
        };

        let mut state = fresh();
        let draw_before = state.piles.get(PileId::Draw).as_slice().to_vec();
        run(&mut state).unwrap();
        assert!(state.exact_piles);
        assert_eq!(state.powers.value(PowerId::ChainsOfBinding), 3);
        assert_eq!(state.piles.get(PileId::Draw).as_slice(), draw_before);

        let mut at_capacity = fresh();
        assert!(at_capacity.fanouts.set_after_card_drawn_order(&[
            PowerId::Speedster,
            PowerId::CorrosiveWave,
            PowerId::Automation,
            PowerId::Cacophony,
            PowerId::Pagestorm,
            PowerId::Iteration,
            PowerId::Speedster,
        ]));
        let before = at_capacity.clone();
        assert_eq!(
            run(&mut at_capacity),
            Err(EngineRefusal::MalformedArgs("Chains listener capacity"))
        );
        assert_eq!(at_capacity, before);
        assert!(!at_capacity.exact_piles);
    }

    fn retained_test_subject(form: u8) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let (max_hp, painful) = match form {
            0 => (crate::engine::monsters::TEST_SUBJECT_FIRST_HP, 0),
            1 => (crate::engine::monsters::TEST_SUBJECT_SECOND_HP, 1),
            _ => unreachable!(),
        };
        let mut owner = HotMonster::new(MonsterKind::TestSubject, 0);
        owner.max_hp = max_hp;
        assert!(owner.set_test_subject_respawns(form));
        owner.set_test_subject_adaptable_reviving(true);
        owner.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
        owner
            .powers
            .set(PowerId::PainfulStabs, SlotWire::Int, painful);
        state.monsters_mut().push(owner);
        state
    }

    fn live_test_subject(form: u8, loop_pos: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let (hp, adaptable, painful, nemesis, enrage) = match form {
            0 => (
                crate::engine::monsters::TEST_SUBJECT_FIRST_HP,
                1,
                0,
                0,
                crate::engine::monsters::TEST_SUBJECT_ENRAGE,
            ),
            1 => (crate::engine::monsters::TEST_SUBJECT_SECOND_HP, 1, 1, 0, 0),
            2 => (crate::engine::monsters::TEST_SUBJECT_THIRD_HP, 0, 0, 1, 0),
            _ => unreachable!(),
        };
        let mut owner = HotMonster::new(MonsterKind::TestSubject, hp);
        owner.max_hp = hp;
        owner.loop_pos = loop_pos;
        assert!(owner.set_test_subject_respawns(form));
        for (power, amount) in [
            (PowerId::Adaptable, adaptable),
            (PowerId::PainfulStabs, painful),
            (PowerId::Nemesis, nemesis),
            (PowerId::Enrage, enrage),
        ] {
            owner.powers.set(power, SlotWire::Int, amount);
        }
        state.monsters_mut().push(owner);
        state
    }

    #[test]
    fn the_owned_list_is_ascending_complete_and_covers_every_claim() {
        assert_eq!(OWNED.len(), 27);
        assert!(OWNED.windows(2).all(|pair| pair[0] < pair[1]));
        for kind in IMPLEMENTED {
            assert!(OWNED.contains(kind));
        }
    }

    #[test]
    fn test_subject_respawn_advances_both_retained_forms_in_native_order() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TestSubject).unwrap();
        let catalog = builder.build();
        let entry = catalog.moves(MonsterKind::TestSubject)[0];

        let mut state = retained_test_subject(0);
        let mut events = Vec::new();
        test_subject_respawn(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        })
        .unwrap();
        let owner = &state.monsters[0];
        assert_eq!(owner.test_subject_respawns(), 1);
        assert!(!owner.test_subject_adaptable_reviving());
        assert_eq!(owner.hp, crate::engine::monsters::TEST_SUBJECT_SECOND_HP);
        assert_eq!(
            owner.max_hp,
            crate::engine::monsters::TEST_SUBJECT_SECOND_HP
        );
        assert_eq!(owner.loop_pos, 3);
        assert_eq!(owner.powers.value(PowerId::Adaptable), 1);
        assert_eq!(owner.powers.value(PowerId::PainfulStabs), 1);
        assert_eq!(owner.powers.value(PowerId::Nemesis), 0);
        assert_eq!(
            events,
            [crate::engine::Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::PainfulStabs,
                amount: 1,
            }]
        );

        state.monsters_mut()[0].hp = 0;
        state.monsters_mut()[0].loop_pos = 0;
        state.monsters_mut()[0].set_test_subject_adaptable_reviving(true);
        events.clear();
        test_subject_respawn(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        })
        .unwrap();
        let owner = &state.monsters[0];
        assert_eq!(owner.test_subject_respawns(), 2);
        assert!(!owner.test_subject_adaptable_reviving());
        assert_eq!(owner.hp, crate::engine::monsters::TEST_SUBJECT_THIRD_HP);
        assert_eq!(owner.max_hp, crate::engine::monsters::TEST_SUBJECT_THIRD_HP);
        assert_eq!(owner.loop_pos, 4);
        assert_eq!(owner.powers.value(PowerId::Adaptable), 0);
        assert_eq!(owner.powers.value(PowerId::PainfulStabs), 0);
        assert_eq!(owner.powers.value(PowerId::Nemesis), 1);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    crate::engine::Event::PowerChanged { power, amount, .. } => {
                        Some((*power, *amount))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [
                (PowerId::Nemesis, 1),
                (PowerId::Adaptable, 0),
                (PowerId::PainfulStabs, 0),
            ]
        );
    }

    #[test]
    fn test_subject_respawn_refuses_before_mutating_an_unlatched_shape() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TestSubject).unwrap();
        let catalog = builder.build();
        let entry = catalog.moves(MonsterKind::TestSubject)[0];
        let mut state = retained_test_subject(0);
        state.monsters_mut()[0].set_test_subject_adaptable_reviving(false);
        let before = state.clone();
        let mut events = Vec::new();
        assert!(
            test_subject_respawn(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut events,
            })
            .is_err()
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn test_subject_generated_moves_pin_serial_transactions_and_frozen_tails() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TestSubject).unwrap();
        let catalog = builder.build();

        let growl = catalog
            .moves(MonsterKind::TestSubject)
            .iter()
            .find(|entry| entry.kind == MoveKind::TestSubjectBurningGrowl)
            .copied()
            .unwrap();
        let mut state = live_test_subject(2, 6);
        state.next_card_uid = 40;
        state.next_generated_hook_uid = 70;
        test_subject_burning_growl(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(growl.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(state.next_card_uid, 45);
        assert_eq!(state.next_generated_hook_uid, 75);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 3);
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| (card.uid, catalog.spec(card.atom).unwrap().row.id))
                .collect::<Vec<_>>(),
            (40..45).map(|uid| (uid, CardId::Burn)).collect::<Vec<_>>()
        );

        let claw = catalog
            .moves(MonsterKind::TestSubject)
            .iter()
            .find(|entry| entry.kind == MoveKind::TestSubjectMultiClaw)
            .copied()
            .unwrap();
        let mut state = live_test_subject(1, 3);
        assert!(state.monsters_mut()[0].set_test_subject_extra_multi_claw_count(2));
        test_subject_multi_claw(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(claw.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(state.hp, 45);
        assert_eq!(state.monsters[0].test_subject_extra_multi_claw_count(), 3);
        assert_eq!(state.piles.get(PileId::Discard).len(), 5);
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .all(|card| matches!(
                    catalog.spec(card.atom).map(|spec| spec.row.id),
                    Some(CardId::Wound)
                ))
        );

        let mut owner_lethal = live_test_subject(1, 3);
        owner_lethal.monsters_mut()[0].hp = 1;
        owner_lethal.powers.set(PowerId::Thorns, SlotWire::Int, 1);
        test_subject_multi_claw(&mut MoveCtx {
            state: &mut owner_lethal,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(claw.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(owner_lethal.monsters[0].test_subject_adaptable_reviving());
        assert_eq!(owner_lethal.hp, 89, "the frozen first hit still lands");
        assert_eq!(owner_lethal.piles.get(PileId::Discard).len(), 1);
        assert_eq!(
            owner_lethal.monsters[0].test_subject_extra_multi_claw_count(),
            1
        );

        let mut player_lethal = live_test_subject(1, 3);
        player_lethal.hp = 1;
        test_subject_multi_claw(&mut MoveCtx {
            state: &mut player_lethal,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(claw.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(player_lethal.history.over);
        assert!(player_lethal.piles.get(PileId::Discard).is_empty());
        assert_eq!(
            player_lethal.monsters[0].test_subject_extra_multi_claw_count(),
            1
        );

        let mut overflow = live_test_subject(1, 3);
        assert!(overflow.monsters_mut()[0].set_test_subject_extra_multi_claw_count(i32::MAX));
        let before = overflow.clone();
        assert!(
            test_subject_multi_claw(&mut MoveCtx {
                state: &mut overflow,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(claw.args),
                events: &mut Vec::new(),
            })
            .is_err()
        );
        assert_eq!(overflow, before);
    }

    /// #2927 (run LYE3ZK9FYKKV floor 48): a player-Thorns kill of a retained
    /// Test Subject form inside its own attack reaches Gremlin Horn's
    /// AfterDeath (`GremlinHorn/<AfterDeath>d__6::MoveNext` RVA `0x326170`:
    /// side check IL_0024–IL_003f, GainEnergy IL_0062, Draw IL_00d9 — no
    /// reviving exemption). Both Test Subject attacks now carry the catalog:
    /// Horn grants one energy and one draw, the corpse latches for RESPAWN,
    /// and each move's own tail still runs.
    #[test]
    fn test_subject_attacks_carry_the_catalog_through_a_thorns_kill_with_gremlin_horn() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TestSubject).unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let row = |kind| {
            catalog
                .moves(MonsterKind::TestSubject)
                .iter()
                .find(|entry| entry.kind == kind)
                .copied()
                .unwrap()
        };
        let arm = |state: &mut HotState| {
            state.monsters_mut()[0].hp = 1;
            state.powers.set(PowerId::Thorns, SlotWire::Int, 1);
            state.fanouts.set_gremlin_horn_owned(true);
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 0,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state.next_card_uid = state.next_card_uid.max(1);
        };
        let drawn_strike = |state: &HotState| {
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().row.id)
                .collect::<Vec<_>>()
                == [CardId::StrikeIronclad]
                && state.piles.get(PileId::Draw).is_empty()
        };

        // Multi Claw (form 1): Horn fires on the first hit's retaliation
        // kill, the later claws are cancelled, and Painful Stabs still
        // injects one Wound for the one positive result.
        let claw = row(MoveKind::TestSubjectMultiClaw);
        let mut state = live_test_subject(1, 3);
        arm(&mut state);
        let energy = state.energy;
        test_subject_multi_claw(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(claw.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(!state.history.over);
        assert!(state.monsters[0].test_subject_adaptable_reviving());
        assert_eq!(state.hp, 89, "the frozen first hit still lands");
        assert_eq!(state.energy, energy + 1);
        assert!(drawn_strike(&state));
        assert_eq!(state.piles.get(PileId::Discard).len(), 1);
        assert_eq!(state.monsters[0].test_subject_extra_multi_claw_count(), 1);

        // The latched corpse then takes its RESPAWN turn normally.
        let respawn = row(MoveKind::TestSubjectRespawn);
        test_subject_respawn(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(respawn.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(state.monsters[0].test_subject_respawns(), 2);
        assert_eq!(
            state.monsters[0].hp,
            crate::engine::monsters::TEST_SUBJECT_THIRD_HP
        );

        // Skull Bash (form 0): same Horn grant, then its Vulnerable tail.
        let bash = row(MoveKind::TestSubjectSkullBash);
        let mut state = live_test_subject(0, 2);
        arm(&mut state);
        let energy = state.energy;
        test_subject_skull_bash(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(bash.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(!state.history.over);
        assert!(state.monsters[0].test_subject_adaptable_reviving());
        assert_eq!(state.energy, energy + 1);
        assert!(drawn_strike(&state));
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 1);
    }

    /// This is the mechanical carrier sweep used for the PR admission audit.
    /// These are the ten exact loops touched by this family; Queen retains
    /// independent named refusals while Soul Fysh and Test Subject are now
    /// closed as complete encounter lifecycles.
    #[test]
    fn the_generated_carrier_census_is_exact() {
        let carriers: BTreeSet<_> = owned_rows()
            .into_iter()
            .map(|(monster, _)| monster.as_str())
            .collect();
        assert_eq!(
            carriers,
            BTreeSet::from([
                "AEONGLASS",
                "CEREMONIAL_BEAST",
                "KNOWLEDGE_DEMON",
                "LAGAVULIN_MATRIARCH",
                "QUEEN",
                "SOUL_FYSH",
                "TEST_SUBJECT",
                "THE_INSATIABLE",
                "VANTOM",
                "WATERFALL_GIANT",
            ])
        );
    }

    #[test]
    fn aeonglass_intensity_maps_native_pile_order_and_generates_two_serial_withers() {
        let (catalog, mut state) = aeonglass_fixture(2, 2);
        for (uid, pile, growth) in [
            (0, PileId::Hand, 0),
            (1, PileId::Draw, 3),
            (2, PileId::Discard, 6),
            (3, PileId::Exhaust, 9),
            (4, PileId::Play, 12),
        ] {
            push_wither(&mut state, &catalog, pile, uid, growth);
        }
        state.card_states.append_local_cost_modifier(
            0,
            crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Set,
                amount: 1,
                expiration: crate::hot::LocalCostExpiration::ThisCombat,
                reduce_only: true,
            },
        );
        state
            .card_states
            .set_to_free_this_turn(0, -1)
            .expect("one exact Star row");
        let row = catalog
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        let mut events = Vec::new();
        aeonglass_intensity(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut events,
        })
        .unwrap();

        assert_eq!(
            [0, 1, 2, 3, 4].map(|uid| state.card_states.get(uid).damage_growth),
            [3, 6, 9, 12, 15]
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| state.card_states.get(card.uid).damage_growth)
                .collect::<Vec<_>>(),
            [9, 9, 9]
        );
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_generated_hook_uid, 2);
        assert_eq!(state.monsters[0].aeonglass_wither_upgrade_count(), 3);
        assert_eq!(state.monsters[0].aeonglass_additional_strength(), 3);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 6);
        let mapped = state.card_states.get(0);
        assert_eq!(mapped.local_cost_modifiers.as_slice().len(), 1);
        assert_eq!(mapped.free_star_cost_this_turn_or_played_rows, 1);
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));
    }

    /// #2957: a fight the oracle opened without exact piles reaches its first
    /// Increasing Intensity with the flag off. The AllCards map promotes it
    /// (`_map_combat_cards(..., force_exact=True)`, frozen Python `_aeonglass_intensity`, deleted #2827)
    /// before either Wither child is generated, and the result is the same
    /// exact state an exact-piles fight reaches.
    #[test]
    fn aeonglass_first_intensity_promotes_exact_piles_like_the_oracle() {
        let (catalog, mut state) = aeonglass_fixture(2, 0);
        state.exact_piles = false;
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));
        let row = catalog
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        aeonglass_intensity(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(state.exact_piles);
        assert_eq!(state.piles.get(PileId::Discard).as_slice().len(), 2);
        assert_eq!(state.monsters[0].aeonglass_wither_upgrade_count(), 1);
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));
    }

    #[test]
    fn aeonglass_intensity_null_withers_never_fire_smokestack() {
        // #3256: Aeonglass's Withers are null-creator cards, and Smokestack
        // leaves on a null creator (`SmokestackPower/<AfterCardGenerated
        // ForCombat>d__4` RVA `0x3455e8` IL_0033-IL_004e). A 5-HP boss
        // survives both children. #3297 removed the terminal-first-child
        // refusal this state could never reach.
        let (catalog, mut state) = aeonglass_fixture(2, 0);
        state.monsters_mut()[0].hp = 5;
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 6);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        let row = catalog
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        aeonglass_intensity(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 5);
        assert_eq!(state.piles.get(PileId::Discard).as_slice().len(), 2);
    }

    fn aeonglass_regalite_fixture(
        loop_pos: i32,
        counter: i32,
    ) -> (crate::catalog::Catalog, HotState) {
        let (_, state) = aeonglass_fixture(loop_pos, counter);
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Aeonglass).unwrap();
        builder
            .intern_reachable(CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicRegalite])
            .unwrap();
        (builder.build(), state)
    }

    fn run_aeonglass_intensity(catalog: &crate::catalog::Catalog, state: &mut HotState) {
        let row = catalog
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        aeonglass_intensity(&mut MoveCtx {
            state,
            catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
    }

    /// #3297 item 2: Regalite beside Aeonglass. Both Wither writers pass a
    /// null creator (Intensity `0x3529ec` IL_014a, Withering Presence
    /// `0x34af14` IL_0103) and `Regalite/<AfterCardGeneratedForCombat>d__8`
    /// RVA `0x32f988` leaves on it at IL_0020-IL_0026, before
    /// `set_UsedThisTurn` and `GainBlock`. Until #3297 both paths refused.
    #[test]
    fn aeonglass_withers_are_admitted_with_regalite_and_never_fire_it() {
        let (catalog, mut state) = aeonglass_regalite_fixture(2, 0);
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));
        run_aeonglass_intensity(&catalog, &mut state);
        assert!(!state.piles.get(PileId::Discard).is_empty());
        assert_eq!(
            state.piles.get(PileId::Discard).len(),
            usize::try_from(state.next_generated_hook_uid).unwrap()
        );
        assert_eq!(state.block, 0);
        assert!(!state.fanouts.regalite_used_this_turn());
        assert_eq!(state.monsters[0].aeonglass_additional_strength(), 1);
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));

        let (catalog, mut state) = aeonglass_regalite_fixture(0, 2);
        state.fanouts.set_withering_cards_left(1);
        crate::engine::cards::withering_after_card_played(
            &mut state,
            &catalog,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(
            state
                .card_states
                .get(state.piles.get(PileId::Hand).as_slice()[0].uid)
                .damage_growth,
            6
        );
        assert_eq!(state.fanouts.withering_cards_left(), 6);
        assert_eq!(state.block, 0);
        assert!(!state.fanouts.regalite_used_this_turn());
        assert!(crate::engine::cards::aeonglass_state_is_exact(
            &state, &catalog
        ));
    }

    /// #3297 item 1: no Wither child is terminal. With every admitted
    /// generated-card listener armed at once (Smokestack far above the boss's
    /// 1 HP, Arsenal, Pillar of Creation, Trash to Treasure, and Regalite),
    /// each null-creator child still enters and none of them answers, so the
    /// removed "terminal first generated child" branch had no state to reach.
    #[test]
    fn aeonglass_wither_children_are_never_terminal_under_every_generated_listener() {
        let (catalog, mut state) = aeonglass_regalite_fixture(2, 0);
        state.monsters_mut()[0].hp = 1;
        let order = [
            PowerId::Smokestack,
            PowerId::Arsenal,
            PowerId::PillarOfCreation,
            PowerId::TrashToTreasure,
        ];
        for power in order {
            state.powers.set(power, SlotWire::Int, 99);
        }
        assert!(state.fanouts.set_local_generated_power_order(&order));
        let before = state.clone();
        run_aeonglass_intensity(&catalog, &mut state);
        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 1);
        assert!(state.next_generated_hook_uid > before.next_generated_hook_uid);
        assert_eq!(
            state.piles.get(PileId::Discard).len(),
            usize::try_from(state.next_generated_hook_uid - before.next_generated_hook_uid)
                .unwrap()
        );
        // No owner-gated listener answered a null creator.
        assert_eq!(state.powers.value(PowerId::Strength), 0);
        assert_eq!(state.block, 0);
        assert_eq!(state.rng, before.rng);
        assert!(!state.fanouts.regalite_used_this_turn());
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        // The move's own suffix ran on the live boss.
        assert!(state.monsters[0].powers.value(PowerId::Strength) > 0);
        assert_eq!(state.monsters[0].aeonglass_additional_strength(), 1);
    }

    #[test]
    fn aeonglass_intensity_forged_roots_and_overflows_refuse_atomically() {
        let (catalog, base) = aeonglass_fixture(2, 0);
        let row = catalog
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        let mut forged = Vec::new();
        let mut multiplayer = base.clone();
        multiplayer.multiplayer_ally_key = 1;
        forged.push(multiplayer);
        let mut wrong_slot = base.clone();
        wrong_slot.monsters_mut()[0].slot = 1;
        forged.push(wrong_slot);
        let mut wrong_uid = base.clone();
        wrong_uid.monsters_mut()[0].uid = 1;
        forged.push(wrong_uid);
        let mut wrong_max_hp = base.clone();
        wrong_max_hp.monsters_mut()[0].max_hp -= 1;
        forged.push(wrong_max_hp);
        let mut wrong_loop = base.clone();
        wrong_loop.monsters_mut()[0].loop_pos = 3;
        forged.push(wrong_loop);
        let mut malformed_artifact = base.clone();
        malformed_artifact.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Bool, 1);
        forged.push(malformed_artifact);
        let mut counter_gap = base.clone();
        assert!(counter_gap.monsters_mut()[0].set_aeonglass_wither_upgrade_count(1));
        forged.push(counter_gap);
        let mut zero_countdown = base.clone();
        assert!(zero_countdown.fanouts.set_withering_cards_left(0));
        forged.push(zero_countdown);
        let mut alien_roster = base.clone();
        alien_roster
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        forged.push(alien_roster);

        for (case, mut state) in forged.into_iter().enumerate() {
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 48 }];
            let events_before = events.clone();
            assert!(
                aeonglass_intensity(&mut MoveCtx {
                    state: &mut state,
                    catalog: &catalog,
                    actor: 0,
                    args: catalog.args(row.args),
                    events: &mut events,
                })
                .is_err(),
                "forged case {case} admitted"
            );
            assert_eq!(state, before, "forged case {case} mutated state");
            assert_eq!(events, events_before, "forged case {case} emitted events");
        }

        for (case, mut state) in [
            {
                let mut state = base.clone();
                assert!(state.monsters_mut()[0].set_aeonglass_additional_strength(i32::MAX));
                assert!(state.monsters_mut()[0].set_aeonglass_wither_upgrade_count(i32::MAX));
                state
            },
            {
                let mut state = base.clone();
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Strength, SlotWire::Int, i32::MAX);
                state
            },
            {
                let mut state = base.clone();
                state.next_card_uid = u32::MAX;
                state
            },
            {
                let mut state = base.clone();
                state.next_generated_hook_uid = i32::MAX;
                state
            },
        ]
        .into_iter()
        .enumerate()
        {
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 48 }];
            let events_before = events.clone();
            assert!(
                aeonglass_intensity(&mut MoveCtx {
                    state: &mut state,
                    catalog: &catalog,
                    actor: 0,
                    args: catalog.args(row.args),
                    events: &mut events,
                })
                .is_err(),
                "overflow case {case} admitted"
            );
            assert_eq!(state, before, "overflow case {case} mutated state");
            assert_eq!(events, events_before, "overflow case {case} emitted events");
        }

        let mut incomplete_builder = CatalogBuilder::new();
        incomplete_builder
            .intern_monster(MonsterKind::Aeonglass)
            .unwrap();
        let incomplete = incomplete_builder.build();
        let incomplete_row = incomplete
            .moves(MonsterKind::Aeonglass)
            .iter()
            .find(|row| row.kind == MoveKind::AeonglassIntensity)
            .copied()
            .unwrap();
        let mut state = base.clone();
        let before = state.clone();
        let mut events = Vec::new();
        assert!(
            aeonglass_intensity(&mut MoveCtx {
                state: &mut state,
                catalog: &incomplete,
                actor: 0,
                args: incomplete.args(incomplete_row.args),
                events: &mut events,
            })
            .is_err()
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn withering_zero_adds_one_null_created_card_without_firing_smokestack() {
        let (catalog, mut state) = aeonglass_fixture(0, 2);
        state.monsters_mut()[0].hp = 5;
        state.fanouts.set_withering_cards_left(1);
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 6);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        let generation_before = state.rng.get(RngStream::Generation);
        let rng_before = state.rng.get(RngStream::Rng);
        crate::engine::cards::withering_after_card_played(
            &mut state,
            &catalog,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        // #3256: the null-creator Wither leaves Smokestack silent.
        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 5);
        assert_eq!(state.fanouts.withering_cards_left(), 6);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        let generated = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(state.card_states.get(generated.uid).damage_growth, 6);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_generated_hook_uid, 1);
        assert_eq!(state.rng.get(RngStream::Generation), generation_before);
        assert_eq!(state.rng.get(RngStream::Rng), rng_before);
    }

    #[test]
    fn withering_full_hand_redirects_the_same_fresh_uid_to_discard() {
        let (catalog, mut state) = aeonglass_fixture(0, 0);
        state.fanouts.set_withering_cards_left(1);
        for uid in 0..10 {
            push_wither(&mut state, &catalog, PileId::Hand, uid, 0);
        }
        crate::engine::cards::withering_after_card_played(
            &mut state,
            &catalog,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Discard).len(), 1);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 10);
        assert_eq!(state.fanouts.withering_cards_left(), 6);
    }

    #[test]
    fn generated_waterfall_rows_are_exact_and_the_complete_loop_admits() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::WaterfallGiant).unwrap();
        let catalog = builder.build();
        let rows = catalog.moves(MonsterKind::WaterfallGiant);
        assert_eq!(rows.len(), 8);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                MoveKind::WaterfallPressurize,
                MoveKind::WaterfallStomp,
                MoveKind::WaterfallRam,
                MoveKind::WaterfallSiphon,
                MoveKind::WaterfallPressureGun,
                MoveKind::WaterfallPressureUp,
                MoveKind::WaterfallAbout,
                MoveKind::WaterfallExplode,
            ]
        );
        assert!(rows.iter().all(|row| IMPLEMENTED.contains(&row.kind)));
        assert_eq!(admit_with_monster(MonsterKind::WaterfallGiant), Ok(()));
    }

    #[test]
    fn generated_ceremonial_beast_rows_and_new_move_bodies_are_exact() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_monster(MonsterKind::CeremonialBeast)
            .unwrap();
        let catalog = builder.build();
        let rows = catalog.moves(MonsterKind::CeremonialBeast);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                MoveKind::BeastStamp,
                MoveKind::AttackStrength,
                MoveKind::BeastCry,
                MoveKind::Attack,
                MoveKind::AttackStrength,
            ]
        );
        assert_eq!(catalog.args(rows[0].args), [CompiledArg::I(160)]);
        assert!(catalog.args(rows[2].args).is_empty());
        assert!(
            rows.iter()
                .all(|row| crate::moves::is_implemented(row.kind))
        );

        let mut state = HotState::at_defaults();
        let mut beast = HotMonster::new(MonsterKind::CeremonialBeast, 262);
        beast.max_hp = 262;
        state.monsters_mut().push(beast);
        let mut events = Vec::new();

        let mut stamp = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(rows[0].args),
            events: &mut events,
        };
        beast_stamp(&mut stamp).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::PlowThreshold), 160);

        state.monsters_mut()[0].loop_pos = 2;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::PlowThreshold, SlotWire::Int, 0);
        let mut cry = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(rows[2].args),
            events: &mut events,
        };
        beast_cry(&mut cry).unwrap();
        assert!(state.ringing() && state.exact_piles);
    }

    /// Every escalation must remain visible at admission on its own carrier.
    #[test]
    fn the_gate_names_every_escalated_kind_on_its_own_monster() {
        let rows = owned_rows();
        let mut measured = BTreeSet::new();
        for (monster, entry) in rows {
            if IMPLEMENTED.contains(&entry.kind) {
                continue;
            }
            measured.insert(entry.kind);
            let refusal = admit_with_monster(monster)
                .expect_err("an unclaimed kind cannot be admitted through its carrier loop");
            assert!(
                refusal.contains(MissingCapability::MoveKind(entry.kind)),
                "{:?} carries {:?}, but its refusal was {refusal}",
                monster.as_str(),
                entry.kind.as_str()
            );
        }
        assert_eq!(
            measured,
            OWNED
                .into_iter()
                .filter(|kind| !IMPLEMENTED.contains(kind))
                .collect()
        );
    }

    /// A documented escalation that accidentally receives a body but not a
    /// manifest entry would otherwise be unreachable and invisible.
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
                Err(EngineRefusal::MoveKindNotModeled(kind))
            );
        }
        assert!(events.is_empty());
    }

    #[test]
    fn soul_fysh_scream_attacks_then_applies_fresh_vulnerable() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let entry = catalog
            .moves(MonsterKind::SoulFysh)
            .iter()
            .find(|entry| entry.kind == MoveKind::Scream)
            .copied()
            .unwrap();
        let mut state = soul_fysh_state(4, 1);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };

        scream(&mut ctx).unwrap();

        assert_eq!(state.hp, 85);
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 3);
        assert_eq!(state.powers.value(PowerId::PlayerVulnFresh), 1);
    }

    #[test]
    fn generated_soul_fysh_loop_is_exact_and_fully_claimed() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let rows = catalog.moves(MonsterKind::SoulFysh);
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                MoveKind::Beckon,
                MoveKind::Attack,
                MoveKind::AttackBeckon,
                MoveKind::Fade,
                MoveKind::Scream,
            ]
        );
        assert!(
            rows.iter()
                .all(|row| crate::moves::is_implemented(row.kind))
        );
        assert_eq!(catalog.args(rows[0].args), [CompiledArg::I(2)]);
        assert_eq!(
            catalog.args(rows[2].args),
            [CompiledArg::I(8), CompiledArg::I(1), CompiledArg::I(1)]
        );
        assert_eq!(catalog.args(rows[3].args), [CompiledArg::I(2)]);
        assert_eq!(
            catalog.args(rows[4].args),
            [CompiledArg::I(15), CompiledArg::I(1), CompiledArg::I(3)]
        );
    }

    /// Soul Fysh `DE_GAS` carries the v0.111.0 A9+ tier (#2807).
    ///
    /// `SoulFysh::get_DeGasDamage` RVA `0xbeca4` IL_0001-IL_0007 is
    /// `GetValueIfAscension(9, 18, 16)` on v0.111.0 (DLL SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`);
    /// v0.110.1's getter (RVA `0xc2220`) was `(9, 17, 16)`, and the table kept
    /// that 17 across the bump. `<DeGasMove>d__39::MoveNext` RVA `0x36b910`
    /// IL_001d-IL_0028 is a single `DamageCmd::Attack(get_DeGasDamage())`
    /// with no `WithHitCount`, so the row is `(18, 1)`.
    ///
    /// `LOOPS` carries both tiers (#2828) and the catalog compiles the
    /// fight's: 18 at A9+ and 16 below, so the A8 capture `9U27KAL35CJY`
    /// (HP already at its A8+ 221) takes 16.
    #[test]
    fn soul_fysh_de_gas_is_the_v0111_a9_tier() {
        for (ascension, damage) in [(8, 16), (10, 18)] {
            let mut builder = CatalogBuilder::new();
            assert!(builder.set_ascension(ascension));
            builder.intern_monster(MonsterKind::SoulFysh).unwrap();
            let catalog = builder.build();
            let row = catalog.moves(MonsterKind::SoulFysh)[1];
            assert_eq!(row.kind, MoveKind::Attack);
            assert_eq!(
                catalog.args(row.args),
                [CompiledArg::I(damage), CompiledArg::I(1)]
            );

            let mut state = soul_fysh_state(1, 0);
            assert!(state.fanouts.set_ascension(ascension));
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(row.args),
                events: &mut events,
            };
            super::super::apply_move(row.kind, &mut ctx).unwrap();
            assert_eq!(state.hp, 100 - i32::try_from(damage).unwrap());
        }

        // At the state's default tier (A10) a 211 roster is the wrong tier
        // and refuses; at A7 it is the native roster and admits (#2539).
        let mut below = soul_fysh_state(1, 0);
        below.monsters_mut()[0].max_hp = 211;
        below.monsters_mut()[0].hp = 211;
        assert!(!crate::engine::monsters::soul_fysh_state_is_valid(&below));
        assert!(below.fanouts.set_ascension(7));
        assert!(crate::engine::monsters::soul_fysh_state_is_valid(&below));
    }

    #[test]
    fn generated_insatiable_loop_is_exact_and_liquify_orders_six_null_transactions() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TheInsatiable).unwrap();
        let catalog = builder.build();
        let rows = catalog.moves(MonsterKind::TheInsatiable);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                MoveKind::InsatiableLiquify,
                MoveKind::Attack,
                MoveKind::Attack,
                MoveKind::BuffStrength,
                MoveKind::Attack,
            ]
        );
        assert!(
            rows.iter()
                .all(|row| crate::moves::is_implemented(row.kind))
        );
        let frantic = CardIdentity {
            id: CardId::FranticEscape,
            upgrade: 0,
            enchantment: None,
        };
        assert!(catalog.atom(&frantic).is_some());

        let mut state = insatiable_state();
        state.next_card_uid = 40;
        state.next_generated_hook_uid = 50;
        let rng_before = state.rng.get(RngStream::Rng).counter;
        let history_before = state.history.owner_generated_cards_combat;
        let mut events = Vec::new();
        insatiable_liquify(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(rows[0].args),
            events: &mut events,
        })
        .unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Sandpit), 4);
        assert_eq!(state.rng.get(RngStream::Rng).counter, rng_before + 6);
        assert_eq!(state.next_card_uid, 46);
        assert_eq!(state.next_generated_hook_uid, 56);
        assert_eq!(state.history.owner_generated_cards_combat, history_before);
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert_eq!(state.piles.get(PileId::Discard).len(), 3);
        let mut uids = state
            .piles
            .get(PileId::Draw)
            .as_slice()
            .iter()
            .chain(state.piles.get(PileId::Discard).as_slice())
            .map(|card| {
                assert_eq!(catalog.spec(card.atom).unwrap().identity, frantic);
                card.uid
            })
            .collect::<Vec<_>>();
        uids.sort_unstable();
        assert_eq!(uids, [40, 41, 42, 43, 44, 45]);
        assert!(matches!(
            events.first(),
            Some(crate::engine::Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Sandpit,
                amount: 4
            })
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::engine::Event::CardResolved { .. }))
                .count(),
            6
        );
    }

    #[test]
    fn liquify_null_frantic_escapes_never_fire_smokestack() {
        // #3256: `TheInsatiable/<LiquifyMove>d__35::MoveNext` RVA `0x36fb70`
        // passes `ldnull` as each Frantic Escape's creator (IL_0308), and
        // Smokestack leaves on a null creator (RVA `0x3455e8`
        // IL_0033-IL_004e): a 1-HP Insatiable survives all six.
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TheInsatiable).unwrap();
        let catalog = builder.build();
        let row = catalog.moves(MonsterKind::TheInsatiable)[0];
        let mut state = insatiable_state();
        state.monsters_mut()[0].hp = 1;
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        state.next_card_uid = 40;
        state.next_generated_hook_uid = 50;
        let rng_before = state.rng.get(RngStream::Rng).counter;

        insatiable_liquify(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();

        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 1);
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert_eq!(state.piles.get(PileId::Discard).len(), 3);
        assert_eq!(state.next_card_uid, 46);
        assert_eq!(state.next_generated_hook_uid, 56);
        assert_eq!(state.rng.get(RngStream::Rng).counter, rng_before + 6);
        assert_eq!(state.monsters[0].powers.value(PowerId::Sandpit), 4);
    }

    #[test]
    fn liquify_preflights_the_complete_six_card_suffix_atomically() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TheInsatiable).unwrap();
        let catalog = builder.build();
        let row = catalog.moves(MonsterKind::TheInsatiable)[0];
        let mut state = insatiable_state();
        state.next_card_uid = u32::MAX - 3;
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            insatiable_liquify(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(row.args),
                events: &mut events,
            }),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        // #3256: Pillar of Creation never answers the null creator, so Block
        // one short of its cap is neither a refusal nor moved.
        let mut quiet_pillar = insatiable_state();
        quiet_pillar.block = 999_999_998;
        quiet_pillar
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        assert!(
            quiet_pillar
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        insatiable_liquify(&mut MoveCtx {
            state: &mut quiet_pillar,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(row.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(quiet_pillar.block, 999_999_998);
        assert_eq!(quiet_pillar.piles.get(PileId::Discard).len(), 3);
    }

    #[test]
    fn beckon_publishes_random_draw_then_discard_as_separate_transactions() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let entry = catalog.moves(MonsterKind::SoulFysh)[0];
        let mut state = soul_fysh_state(0, 0);
        state.next_card_uid = 40;
        state.next_generated_hook_uid = 50;
        let rng_before = state.rng.get(RngStream::Rng).counter;
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };

        beckon(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Rng).counter, rng_before + 1);
        assert_eq!(state.next_card_uid, 42);
        assert_eq!(state.next_generated_hook_uid, 52);
        // #3256: both Beckons are null-creator cards, never owner history.
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 40);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 41);
        assert_eq!(
            events,
            [
                crate::engine::Event::CardResolved {
                    uid: 40,
                    pile: PileId::Draw
                },
                crate::engine::Event::CardResolved {
                    uid: 41,
                    pile: PileId::Discard
                },
            ]
        );

        let mut overflow = soul_fysh_state(0, 0);
        overflow.next_card_uid = u32::MAX - 1;
        let before = overflow.clone();
        let mut overflow_events = Vec::new();
        assert_eq!(
            beckon(&mut MoveCtx {
                state: &mut overflow,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut overflow_events,
            }),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(overflow, before);
        assert!(overflow_events.is_empty());
    }

    /// #3297 item 3: the Beckon preflight is null-specific. Owner history at
    /// its ceiling, Arsenal at the Strength ceiling and Pillar of Creation at
    /// the Block ceiling all refused before, though no null-creator Beckon
    /// reaches any of them (`ArsenalPower/<…>d__6` RVA `0x334ec8` and
    /// `PillarOfCreationPower/<…>d__6` RVA `0x340488` leave at
    /// IL_001d-IL_0023). The counters the null commands do write still refuse
    /// by name, and Beckon's two-card batch refuses atomically.
    #[test]
    fn beckon_preflight_checks_only_what_a_null_creator_card_writes() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let moves = catalog.moves(MonsterKind::SoulFysh);

        for (index, loop_pos) in [(0usize, 0), (2, 2)] {
            let body = if index == 0 { beckon } else { attack_beckon };
            let cards: u32 = if index == 0 { 2 } else { 1 };

            let mut history = soul_fysh_state(loop_pos, 0);
            history.history.owner_generated_cards_combat = i32::MAX;
            body(&mut MoveCtx {
                state: &mut history,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(moves[index].args),
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(history.history.owner_generated_cards_combat, i32::MAX);
            assert_eq!(history.next_card_uid, cards, "move {index}");

            let mut arsenal = soul_fysh_state(loop_pos, 0);
            arsenal.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
            arsenal
                .powers
                .set(PowerId::Strength, SlotWire::Int, i32::MAX);
            assert!(
                arsenal
                    .fanouts
                    .set_local_generated_power_order(&[PowerId::Arsenal])
            );
            body(&mut MoveCtx {
                state: &mut arsenal,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(moves[index].args),
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(arsenal.powers.value(PowerId::Strength), i32::MAX);
            assert_eq!(arsenal.next_card_uid, cards, "move {index}");

            let mut epoch = soul_fysh_state(loop_pos, 0);
            epoch.next_generated_hook_uid = i32::MAX - i32::try_from(cards).unwrap() + 1;
            let mut uid = soul_fysh_state(loop_pos, 0);
            uid.next_card_uid = u32::MAX - cards + 1;
            for (name, mut state) in [("next_generated_hook_uid", epoch), ("next_card_uid", uid)] {
                let before = state.clone();
                let mut events = Vec::new();
                assert_eq!(
                    body(&mut MoveCtx {
                        state: &mut state,
                        catalog: &catalog,
                        actor: 0,
                        args: catalog.args(moves[index].args),
                        events: &mut events,
                    }),
                    Err(EngineRefusal::CounterOverflow(name)),
                    "move {index}"
                );
                // Gaze's attack lands before its Beckon is preflighted, so
                // only Beckon's own two-card batch is all-or-nothing here.
                if index == 0 {
                    assert_eq!(state, before);
                    assert!(events.is_empty());
                }
            }
        }

        let mut pillar = soul_fysh_state(0, 0);
        pillar.block = 999_999_999;
        pillar
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        assert!(
            pillar
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        beckon(&mut MoveCtx {
            state: &mut pillar,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(moves[0].args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert_eq!(pillar.block, 999_999_999);
        assert_eq!(pillar.next_card_uid, 2);
    }

    /// #3256: both Soul Fysh Beckon moves pass a null creator
    /// (`<BeckonMove>d__37` RVA `0x36b4b8` IL_0216/IL_02ae,
    /// `<GazeMove>d__38` RVA `0x36bba0` IL_0135), so a Smokestack never
    /// answers a Beckon and neither Beckon counts as owner history.
    #[test]
    fn beckons_are_null_created_and_never_fire_smokestack() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let moves = catalog.moves(MonsterKind::SoulFysh);
        for (index, loop_pos) in [(0, 0), (2, 2)] {
            let mut state = soul_fysh_state(loop_pos, 0);
            state.powers.set(PowerId::Smokestack, SlotWire::Int, 5);
            assert!(
                state
                    .fanouts
                    .set_local_generated_power_order(&[PowerId::Smokestack])
            );
            let hp_before = state.monsters[0].hp;
            let body = if index == 0 { beckon } else { attack_beckon };
            body(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(moves[index].args),
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(state.monsters[0].hp, hp_before, "move {index}");
            assert_eq!(state.history.owner_generated_cards_combat, 0);
            assert!(!state.piles.get(PileId::Discard).is_empty());
        }
    }

    #[test]
    fn attack_beckon_generates_only_after_a_live_attack_suffix() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let entry = catalog.moves(MonsterKind::SoulFysh)[2];

        let mut normal = soul_fysh_state(2, 0);
        let mut events = Vec::new();
        attack_beckon(&mut MoveCtx {
            state: &mut normal,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        })
        .unwrap();
        assert_eq!(normal.hp, 92);
        assert_eq!(normal.piles.get(PileId::Discard).len(), 1);
        assert_eq!(normal.history.owner_generated_cards_combat, 0);

        let mut lethal_player = soul_fysh_state(2, 0);
        lethal_player.hp = 1;
        attack_beckon(&mut MoveCtx {
            state: &mut lethal_player,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(lethal_player.history.over);
        assert!(lethal_player.piles.get(PileId::Discard).is_empty());
        assert_eq!(lethal_player.next_card_uid, 0);

        let mut lethal_owner = soul_fysh_state(2, 0);
        lethal_owner.monsters_mut()[0].hp = 1;
        lethal_owner
            .powers
            .set(PowerId::FlameBarrier, SlotWire::Int, 10);
        attack_beckon(&mut MoveCtx {
            state: &mut lethal_owner,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(lethal_owner.monsters[0].hp <= 0);
        assert!(lethal_owner.piles.get(PileId::Discard).is_empty());
        assert_eq!(lethal_owner.next_card_uid, 0);
    }

    #[test]
    fn fade_installs_only_the_exact_reachable_intangible_state() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        let entry = catalog.moves(MonsterKind::SoulFysh)[3];
        let mut state = soul_fysh_state(3, 0);
        let mut events = Vec::new();
        fade(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        })
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Intangible), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::engine::Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Intangible,
                amount: 2,
            }
        )));

        state.monsters_mut()[0]
            .powers
            .set(PowerId::Demise, SlotWire::Int, 1);
        let before = state.clone();
        assert!(
            fade(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut Vec::new(),
            })
            .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn r2_boss_suffixes_pin_cards_heal_stats_and_vulnerability() {
        let mut builder = CatalogBuilder::new();
        for kind in [
            MonsterKind::Vantom,
            MonsterKind::KnowledgeDemon,
            MonsterKind::LagavulinMatriarch,
            MonsterKind::TestSubject,
        ] {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 200;
        state.max_hp = 200;
        for (uid, kind) in [
            MonsterKind::Vantom,
            MonsterKind::KnowledgeDemon,
            MonsterKind::LagavulinMatriarch,
        ]
        .into_iter()
        .enumerate()
        {
            let mut monster = HotMonster::new(kind, 50);
            monster.uid = uid as u32;
            monster.max_hp = 100;
            state.monsters_mut().push(monster);
        }
        let mut events = Vec::new();
        for (actor, kind) in [
            MoveKind::AttackWounds,
            MoveKind::Ponder,
            MoveKind::SoulSiphon,
        ]
        .into_iter()
        .enumerate()
        {
            let owner = state.monsters[actor].kind;
            let entry = catalog
                .moves(owner)
                .iter()
                .find(|entry| entry.kind == kind)
                .copied()
                .unwrap();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor,
                args: catalog.args(entry.args),
                events: &mut events,
            };
            super::super::apply_move(kind, &mut ctx).unwrap();
        }
        assert_eq!(state.hp, 157);
        assert_eq!(state.piles.get(PileId::Discard).len(), 3);
        assert_eq!(state.monsters[1].hp, 80);
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), 3);
        assert_eq!(state.powers.value(PowerId::Strength), -2);
        assert_eq!(state.powers.value(PowerId::Dexterity), -2);
        assert_eq!(state.monsters[2].powers.value(PowerId::Strength), 2);

        let mut subject = HotState::at_defaults();
        subject.hp = 200;
        subject.max_hp = 200;
        let mut owner = HotMonster::new(
            MonsterKind::TestSubject,
            crate::engine::monsters::TEST_SUBJECT_FIRST_HP,
        );
        owner.max_hp = crate::engine::monsters::TEST_SUBJECT_FIRST_HP;
        owner.loop_pos = 2;
        owner.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
        owner.powers.set(
            PowerId::Enrage,
            SlotWire::Int,
            crate::engine::monsters::TEST_SUBJECT_ENRAGE,
        );
        subject.monsters_mut().push(owner);
        let entry = catalog
            .moves(MonsterKind::TestSubject)
            .iter()
            .find(|entry| entry.kind == MoveKind::TestSubjectSkullBash)
            .copied()
            .unwrap();
        super::super::apply_move(
            MoveKind::TestSubjectSkullBash,
            &mut MoveCtx {
                state: &mut subject,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(subject.hp, 184);
        assert_eq!(subject.powers.value(PowerId::PlayerVuln), 1);
        assert_eq!(subject.powers.value(PowerId::PlayerVulnFresh), 1);
    }

    #[test]
    fn queen_youre_mine_validates_roster_then_applies_three_fresh_durations() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_monster(MonsterKind::TorchHeadAmalgam)
            .unwrap();
        builder.intern_monster(MonsterKind::Queen).unwrap();
        let catalog = builder.build();
        let entry = catalog
            .moves(MonsterKind::Queen)
            .iter()
            .find(|entry| entry.kind == MoveKind::QueenYoureMine)
            .copied()
            .unwrap();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        let mut amalgam = crate::hot::HotMonster::new(MonsterKind::TorchHeadAmalgam, 211);
        amalgam.max_hp = 211;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = crate::hot::HotMonster::new(MonsterKind::Queen, 419);
        queen.max_hp = 419;
        queen.slot = 1;
        queen.uid = 1;
        state.monsters_mut().extend([amalgam, queen]);
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: catalog.args(entry.args),
            events: &mut events,
        };

        queen_youre_mine(&mut ctx).unwrap();

        for (power, fresh) in [
            (PowerId::PlayerFrail, PowerId::PlayerFrailFresh),
            (PowerId::PlayerWeak, PowerId::PlayerWeakFresh),
            (PowerId::PlayerVuln, PowerId::PlayerVulnFresh),
        ] {
            assert_eq!(state.powers.value(power), 99);
            assert_eq!(state.powers.value(fresh), 1);
        }
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    crate::engine::Event::PowerChanged { power, .. } => Some(*power),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [
                PowerId::PlayerFrail,
                PowerId::PlayerFrailFresh,
                PowerId::PlayerWeak,
                PowerId::PlayerWeakFresh,
                PowerId::PlayerVuln,
                PowerId::PlayerVulnFresh,
            ]
        );

        state.monsters_mut()[0].max_hp = 210;
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        assert_eq!(
            queen_youre_mine(&mut ctx),
            Err(EngineRefusal::MalformedArgs("queen roster"))
        );
    }

    #[test]
    fn queen_burn_bright_row_metadata_and_public_dispatch_are_exact() {
        let (mut state, catalog, row) = queen_burn_bright_fixture();
        let rows: Vec<_> = MonsterKind::ALL
            .into_iter()
            .flat_map(|owner| {
                catalog
                    .moves(owner)
                    .iter()
                    .filter(|entry| entry.kind == MoveKind::QueenBurnBright)
                    .map(move |entry| (owner, *entry))
            })
            .collect();
        assert_eq!(rows, [(MonsterKind::Queen, row)]);
        assert_eq!(
            catalog.args(row.args),
            [CompiledArg::I(1), CompiledArg::I(20)]
        );
        assert_eq!(
            row.repeats,
            Repeats::Conditional(&[Arg::S("queen_amalgam_alive"), Arg::I(2), Arg::I(3),])
        );
        assert!(IMPLEMENTED.contains(&MoveKind::QueenBurnBright));
        assert!(
            crate::engine::capability_manifest()
                .moves
                .contains(&MoveKind::QueenBurnBright)
        );

        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 71 }];
        let before_events = events.clone();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: catalog.args(row.args),
            events: &mut events,
        };
        queen_burn_bright(&mut ctx).unwrap();
        assert_ne!(*ctx.state, before);
        assert_ne!(*ctx.events, before_events);
    }

    #[test]
    fn queen_burn_bright_living_foundation_stacks_amalgam_strength_then_queen_block() {
        let (mut state, catalog, row) = queen_burn_bright_fixture();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, 5);
        state.monsters_mut()[1].block = 7;
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Strength, SlotWire::Int, -2);
        let mut events = Vec::new();

        for _ in 0..2 {
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            )
            .unwrap();
        }

        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 7);
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), -2);
        assert_eq!(state.monsters[1].block, 47);
        assert_eq!(
            events,
            [
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 6,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 7,
                },
            ]
        );
    }

    #[test]
    fn queen_burn_bright_living_foundation_accepts_every_exact_listener_permutation() {
        for order in [
            &[][..],
            &[PowerId::Shroud][..],
            &[PowerId::SleightOfFlesh][..],
            &[PowerId::Shroud, PowerId::SleightOfFlesh][..],
            &[PowerId::SleightOfFlesh, PowerId::Shroud][..],
            &[
                PowerId::Vicious,
                PowerId::Shroud,
                PowerId::SleightOfFlesh,
                PowerId::SwordSage,
            ][..],
        ] {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            if order.contains(&PowerId::Shroud) {
                state.powers.set(PowerId::Shroud, SlotWire::Int, 2);
            }
            if order.contains(&PowerId::SleightOfFlesh) {
                state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 3);
            }
            if order.contains(&PowerId::Vicious) {
                state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
            }
            if order.contains(&PowerId::SwordSage) {
                state.powers.set(PowerId::SwordSage, SlotWire::Int, 1);
            }
            assert!(state.fanouts.set_after_power_amount_changed_order(order));
            let mut events = Vec::new();
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
            assert_eq!(state.monsters[1].block, 20);
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_rejects_listener_projection_drift_atomically() {
        for malformed in 0..12 {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            let order: &[PowerId] = match malformed {
                0 => {
                    state.powers.set(PowerId::Shroud, SlotWire::Int, 2);
                    &[]
                }
                1 => {
                    state.powers.set(PowerId::Shroud, SlotWire::Int, 2);
                    &[PowerId::Shroud, PowerId::Shroud]
                }
                2 => &[PowerId::SwordSage],
                3 => {
                    state.powers.set(PowerId::Shroud, SlotWire::Bool, 1);
                    &[PowerId::Shroud]
                }
                4 => {
                    state.powers.set(PowerId::Shroud, SlotWire::Int, -1);
                    &[]
                }
                5 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::Shroud,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                    &[]
                }
                6 => {
                    state.powers.set(PowerId::SleightOfFlesh, SlotWire::Bool, 1);
                    &[PowerId::SleightOfFlesh]
                }
                7 => {
                    state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, -1);
                    &[]
                }
                8 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::SleightOfFlesh,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                    &[]
                }
                9 => {
                    state.powers.set(PowerId::Vicious, SlotWire::Bool, 1);
                    &[PowerId::Vicious]
                }
                10 => {
                    state.powers.set(PowerId::Shroud, SlotWire::Int, 2);
                    state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 3);
                    &[PowerId::Shroud]
                }
                11 => {
                    state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 3);
                    &[PowerId::SleightOfFlesh, PowerId::SleightOfFlesh]
                }
                _ => unreachable!(),
            };
            assert!(state.fanouts.set_after_power_amount_changed_order(order));
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 77 }];
            let before_events = events.clone();
            assert_eq!(
                run_queen_burn_bright_foundation(
                    &mut state,
                    &catalog,
                    1,
                    catalog.args(row.args),
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation")),
                "malformed listener projection {malformed}"
            );
            assert_eq!(state, before, "malformed listener projection {malformed}");
            assert_eq!(
                events, before_events,
                "malformed listener projection {malformed}"
            );
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_rejects_roster_machine_and_wire_drift_atomically() {
        for malformed in 0..24 {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            let mut actor = 1;
            match malformed {
                0 => state.monsters_mut()[0].hp = 0,
                1 => state.monsters_mut()[1].hp = 0,
                2 => state.monsters_mut()[0].hp = 212,
                3 => state.monsters_mut()[1].hp = 420,
                4 => state.monsters_mut()[0].block = -1,
                5 => state.monsters_mut()[1].block = -1,
                6 => actor = 0,
                7 => state.monsters_mut()[1].loop_pos = 1,
                8 => state.monsters_mut()[0].loop_pos = 5,
                9 => state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Secondary, SlotWire::Bool, 0),
                10 => state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Secondary, SlotWire::Int, 1),
                11 => state.monsters_mut()[1]
                    .powers
                    .set(PowerId::Secondary, SlotWire::Bool, 1),
                12 => state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Strength, SlotWire::Bool, 1),
                13 => {
                    state.monsters_mut()[1].powers =
                        crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                            key: PowerId::Strength,
                            wire: SlotWire::Int,
                            value: 0,
                        }])
                        .unwrap();
                }
                14 => {
                    state.monsters_mut()[0].powers = crate::powers::Slots::from_unsorted(vec![
                        crate::powers::Slot {
                            key: PowerId::Secondary,
                            wire: SlotWire::Bool,
                            value: 1,
                        },
                        crate::powers::Slot {
                            key: PowerId::Strength,
                            wire: SlotWire::Int,
                            value: 0,
                        },
                    ])
                    .unwrap();
                }
                15 => state.monsters_mut()[1]
                    .powers
                    .set(PowerId::Strength, SlotWire::Bool, 1),
                16 => state.monsters_mut()[1].override_state = crate::hot::MonsterOverride::Stunned,
                17 => {
                    state.monsters_mut()[0].forced_follow_up =
                        crate::hot::MonsterFollowUp::BeastCry;
                }
                18 => assert!(state.monsters_mut()[1].random_ai.set_next(Some(0))),
                19 => state.monsters_mut()[0].spawn_noop = true,
                20 => state.monsters_mut()[1].revive_stage = 1,
                21 => state.monsters_mut()[0].uid = 9,
                22 => state.monsters_mut()[0].block = QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP + 1,
                23 => state.monsters_mut()[1].block = QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP + 1,
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 72 }];
            let before_events = events.clone();
            assert_eq!(
                run_queen_burn_bright_foundation(
                    &mut state,
                    &catalog,
                    actor,
                    catalog.args(row.args),
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation")),
                "malformed case {malformed}"
            );
            assert_eq!(state, before, "malformed case {malformed}");
            assert_eq!(events, before_events, "malformed case {malformed}");
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_preserves_ordinary_survivor_powers() {
        for (owner, power) in [
            (0, PowerId::Weak),
            (1, PowerId::Weak),
            (0, PowerId::Oblivion),
            (1, PowerId::Oblivion),
        ] {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            state.monsters_mut()[owner]
                .powers
                .set(power, SlotWire::Int, 1);
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 80 }];
            let before_events = events.clone();
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            )
            .unwrap();
            assert_ne!(state, before, "owner {owner} survivor power {power:?}");
            assert_ne!(
                events, before_events,
                "owner {owner} survivor power {power:?}"
            );
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_refuses_multiplayer_scaling_atomically() {
        let (mut state, catalog, row) = queen_burn_bright_fixture();
        state.multiplayer_ally_key = 1;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 81 }];
        let before_events = events.clone();
        assert_eq!(
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn queen_burn_bright_living_foundation_refuses_dead_retained_proxies_and_missing_program() {
        for shape in 0..4 {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            match shape {
                0 => state.monsters_mut()[0].hp = 0,
                1 => {
                    state.monsters_mut()[0].hp = 0;
                    state.monsters_mut()[1].override_state = crate::hot::MonsterOverride::Stunned;
                }
                2 => {
                    state.monsters_mut()[0].hp = 0;
                    state.monsters_mut()[1].loop_pos = 3;
                }
                3 => {
                    state.monsters_mut().remove(0);
                }
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 73 }];
            let before_events = events.clone();
            assert!(
                run_queen_burn_bright_foundation(
                    &mut state,
                    &catalog,
                    1,
                    catalog.args(row.args),
                    &mut events,
                )
                .is_err()
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        for args in [
            &[CompiledArg::I(2), CompiledArg::I(20)][..],
            &[CompiledArg::I(1), CompiledArg::I(19)][..],
            &[CompiledArg::I(1)][..],
        ] {
            let (mut state, catalog, _row) = queen_burn_bright_fixture();
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 74 }];
            let before_events = events.clone();
            assert_eq!(
                run_queen_burn_bright_foundation(&mut state, &catalog, 1, args, &mut events,),
                Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        for only_kind in [MonsterKind::TorchHeadAmalgam, MonsterKind::Queen] {
            let (mut state, _catalog, _row) = queen_burn_bright_fixture();
            let mut builder = CatalogBuilder::new();
            builder.intern_monster(only_kind).unwrap();
            let incomplete = builder.build();
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 75 }];
            let before_events = events.clone();
            assert_eq!(
                run_queen_burn_bright_foundation(
                    &mut state,
                    &incomplete,
                    1,
                    &[CompiledArg::I(1), CompiledArg::I(20)],
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_accepts_every_exact_block_listener_permutation() {
        for order in [
            &[][..],
            &[PowerId::Juggernaut][..],
            &[PowerId::BeaconOfHope][..],
            &[PowerId::Juggernaut, PowerId::BeaconOfHope][..],
            &[PowerId::BeaconOfHope, PowerId::Juggernaut][..],
        ] {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            if order.contains(&PowerId::Juggernaut) {
                state.powers.set(PowerId::Juggernaut, SlotWire::Int, 4);
            }
            if order.contains(&PowerId::BeaconOfHope) {
                state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, 1);
            }
            assert!(state.fanouts.set_after_block_gained_order(order));
            let mut events = Vec::new();
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
            assert_eq!(state.monsters[1].block, 20);
        }
    }

    #[test]
    fn queen_burn_bright_living_foundation_rejects_block_listener_projection_drift_atomically() {
        for malformed in 0..13 {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            let order: &[PowerId] = match malformed {
                0 => {
                    state.powers.set(PowerId::Juggernaut, SlotWire::Int, 4);
                    &[]
                }
                1 => {
                    state.powers.set(PowerId::Juggernaut, SlotWire::Int, 4);
                    &[PowerId::Juggernaut, PowerId::Juggernaut]
                }
                2 => &[PowerId::Strength],
                3 => {
                    state.powers.set(PowerId::Juggernaut, SlotWire::Bool, 1);
                    &[PowerId::Juggernaut]
                }
                4 => {
                    state.powers.set(PowerId::Juggernaut, SlotWire::Int, -1);
                    &[]
                }
                5 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::Juggernaut,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                    &[]
                }
                6 => {
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Bool, 1);
                    &[PowerId::BeaconOfHope]
                }
                7 => {
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, -1);
                    &[]
                }
                8 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::BeaconOfHope,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                    &[]
                }
                9 => {
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, 2);
                    &[PowerId::BeaconOfHope]
                }
                10 => {
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, 1);
                    &[]
                }
                11 => {
                    state.powers.set(PowerId::Juggernaut, SlotWire::Int, 4);
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, 1);
                    &[PowerId::Juggernaut]
                }
                12 => {
                    state.powers.set(PowerId::BeaconOfHope, SlotWire::Int, 1);
                    &[PowerId::BeaconOfHope, PowerId::BeaconOfHope]
                }
                _ => unreachable!(),
            };
            assert!(state.fanouts.set_after_block_gained_order(order));
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 78 }];
            let before_events = events.clone();
            assert_eq!(
                run_queen_burn_bright_foundation(
                    &mut state,
                    &catalog,
                    1,
                    catalog.args(row.args),
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("Queen Burn Bright foundation")),
                "malformed block listener projection {malformed}"
            );
            assert_eq!(
                state, before,
                "malformed block listener projection {malformed}"
            );
            assert_eq!(
                events, before_events,
                "malformed block listener projection {malformed}"
            );
        }
    }

    #[test]
    fn queen_burn_bright_strength_overflow_rolls_back_and_block_saturates_at_native_cap() {
        let (mut overflow, catalog, row) = queen_burn_bright_fixture();
        overflow.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX);
        let before = overflow.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 79 }];
        let before_events = events.clone();
        assert_eq!(
            run_queen_burn_bright_foundation(
                &mut overflow,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "Queen Burn Bright Amalgam Strength"
            ))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);

        for (initial, expected) in [
            (
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP - 21,
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP - 1,
            ),
            (
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP - 10,
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP,
            ),
            (
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP,
                QUEEN_BURN_BRIGHT_NATIVE_BLOCK_CAP,
            ),
        ] {
            let (mut state, catalog, row) = queen_burn_bright_fixture();
            state.monsters_mut()[1].block = initial;
            let mut events = Vec::new();
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
            assert_eq!(state.monsters[1].block, expected);
        }
    }

    #[test]
    fn queen_burn_bright_refuses_an_unrepresentable_temporary_strength_successor() {
        let (mut state, catalog, row) = queen_burn_bright_fixture();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX - 1);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -1);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 82 }];
        let before_events = events.clone();

        assert_eq!(
            run_queen_burn_bright_foundation(
                &mut state,
                &catalog,
                1,
                catalog.args(row.args),
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn queen_burn_bright_terminal_entry_runs_no_nested_command() {
        let (mut state, catalog, row) = queen_burn_bright_fixture();
        state.history.over = true;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 76 }];
        let before_events = events.clone();
        run_queen_burn_bright_foundation(
            &mut state,
            &catalog,
            1,
            catalog.args(row.args),
            &mut events,
        )
        .unwrap();
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn waterfall_move_bodies_preserve_the_pressure_rotation_and_terminal_self_kill() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::WaterfallGiant).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 1_000;
        state.max_hp = 1_000;
        let mut monster = HotMonster::new(MonsterKind::WaterfallGiant, 250);
        monster.max_hp = 250;
        monster.pressure_gun_damage = 23;
        state.monsters_mut().push(monster);
        let mut events = Vec::new();

        let run = |kind: MoveKind, state: &mut HotState, events: &mut Vec<crate::engine::Event>| {
            let entry = catalog
                .moves(MonsterKind::WaterfallGiant)
                .iter()
                .find(|entry| entry.kind == kind)
                .copied()
                .unwrap();
            let mut ctx = MoveCtx {
                state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events,
            };
            super::super::apply_move(kind, &mut ctx).unwrap();
        };

        run(MoveKind::WaterfallPressurize, &mut state, &mut events);
        assert_eq!(state.monsters[0].powers.value(PowerId::SteamPressure), 20);
        assert_eq!(state.monsters[0].pressure_buildup_idx(), 1);

        run(MoveKind::WaterfallStomp, &mut state, &mut events);
        assert_eq!(state.powers.value(PowerId::PlayerWeak), 1);
        assert_eq!(state.powers.value(PowerId::PlayerWeakFresh), 1);
        run(MoveKind::WaterfallRam, &mut state, &mut events);
        state.monsters_mut()[0].hp = 200;
        run(MoveKind::WaterfallSiphon, &mut state, &mut events);
        assert_eq!(state.monsters[0].hp, 215);
        run(MoveKind::WaterfallPressureGun, &mut state, &mut events);
        assert_eq!(state.monsters[0].pressure_gun_damage, 28);
        run(MoveKind::WaterfallPressureUp, &mut state, &mut events);
        assert_eq!(state.monsters[0].powers.value(PowerId::SteamPressure), 35);
        assert_eq!(state.monsters[0].pressure_buildup_idx(), 6);

        state.monsters_mut()[0].set_is_about_to_blow(true);
        run(MoveKind::WaterfallAbout, &mut state, &mut events);
        assert_eq!(state.monsters[0].waterfall_steam_eruption_damage(), 35);
        assert_eq!(state.monsters[0].powers.value(PowerId::SteamPressure), 0);
        assert_eq!(state.monsters[0].pressure_buildup_idx(), 7);
        state.hp = 1;
        run(MoveKind::WaterfallExplode, &mut state, &mut events);
        assert_eq!(state.hp, 0);
        assert!(state.monsters[0].hp <= 0);
        assert!(state.history.over);
    }

    /// #3166 (f15842d1c9bcca83): Waterfall Giant's Explode self-kill with
    /// Gremlin Horn owned. `<ExplodeMove>d__76::MoveNext` RVA `0x3756dc`
    /// awaits the attack, then `CreatureCmd.Kill(Creature, false)` (IL_00c0),
    /// whose AfterDeath walk reaches Gremlin Horn. The move keeps its
    /// catalog through that death instead of refusing "Gremlin Horn death
    /// catalog"; the lone boss's death ends the combat, so the ending gate
    /// suppresses the Horn's energy and draw.
    #[test]
    fn waterfall_explode_self_kill_keeps_the_catalog_for_gremlin_horn() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::WaterfallGiant).unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 1_000;
        state.max_hp = 1_000;
        state.fanouts.set_gremlin_horn_owned(true);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 0,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.next_card_uid = 1;
        let mut monster = HotMonster::new(MonsterKind::WaterfallGiant, 250);
        monster.max_hp = 250;
        monster.set_is_about_to_blow(true);
        assert!(monster.set_waterfall_steam_eruption_damage(35));
        state.monsters_mut().push(monster);
        let entry = catalog
            .moves(MonsterKind::WaterfallGiant)
            .iter()
            .find(|entry| entry.kind == MoveKind::WaterfallExplode)
            .copied()
            .unwrap();
        let energy = state.energy;
        super::super::apply_move(
            MoveKind::WaterfallExplode,
            &mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(state.hp, 965);
        assert!(state.monsters[0].hp <= 0);
        assert!(state.history.over);
        assert_eq!(state.energy, energy);
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
    }

    /// #2828: every Waterfall Giant body computes from its own tiered row, and
    /// the gates differ. `get_SiphonHeal` RVA `0xc4d15` is
    /// `GetValueIfAscension(8, 15, 10)`; `get_PressurizeAmount` `0xc4d21`
    /// (9, 20, 15), `get_StompDamage` `0xc4d2e` (9, 16, 15), `get_RamDamage`
    /// `0xc4d3b` (9, 11, 10) and `get_PressureUpDamage` `0xc4d48` (9, 14, 13)
    /// are gated at A9. So A8 heals the A8+ 15 while pressurizing and hitting
    /// at the below-A9 tier, and A7 takes the lower tier of all five.
    #[test]
    fn waterfall_bodies_follow_each_constants_own_gate() {
        for (ascension, pressurize, stomp, ram, heal, pressure_up) in [
            (7, 15, 15, 10, 10, 13),
            (8, 15, 15, 10, 15, 13),
            (9, 20, 16, 11, 15, 14),
        ] {
            let mut builder = CatalogBuilder::new();
            assert!(builder.set_ascension(ascension));
            builder.intern_monster(MonsterKind::WaterfallGiant).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            assert!(state.fanouts.set_ascension(ascension));
            state.hp = 1_000;
            state.max_hp = 1_000;
            let mut monster = HotMonster::new(MonsterKind::WaterfallGiant, 250);
            monster.max_hp = 250;
            monster.pressure_gun_damage = 23;
            state.monsters_mut().push(monster);
            let mut events = Vec::new();
            let mut run = |kind: MoveKind, state: &mut HotState| {
                let entry = catalog
                    .moves(MonsterKind::WaterfallGiant)
                    .iter()
                    .find(|entry| entry.kind == kind)
                    .copied()
                    .unwrap();
                let hp = state.hp;
                super::super::apply_move(
                    kind,
                    &mut MoveCtx {
                        state,
                        catalog: &catalog,
                        actor: 0,
                        args: catalog.args(entry.args),
                        events: &mut events,
                    },
                )
                .unwrap();
                hp - state.hp
            };
            assert_eq!(run(MoveKind::WaterfallPressurize, &mut state), 0);
            assert_eq!(
                state.monsters[0].powers.value(PowerId::SteamPressure),
                pressurize,
                "A{ascension}"
            );
            assert_eq!(
                run(MoveKind::WaterfallStomp, &mut state),
                stomp,
                "A{ascension}"
            );
            // Weak from STOMP applies to the player, not to the Giant's hits.
            assert_eq!(run(MoveKind::WaterfallRam, &mut state), ram, "A{ascension}");
            state.monsters_mut()[0].hp = 200;
            run(MoveKind::WaterfallSiphon, &mut state);
            assert_eq!(state.monsters[0].hp, 200 + heal, "A{ascension}");
            assert_eq!(
                run(MoveKind::WaterfallPressureUp, &mut state),
                pressure_up,
                "A{ascension}"
            );

            // A row compiled at another tier is refused by its own body, not
            // run with the wrong constant.
            let mut forged = state.clone();
            let entry = catalog
                .moves(MonsterKind::WaterfallGiant)
                .iter()
                .find(|entry| entry.kind == MoveKind::WaterfallPressurize)
                .copied()
                .unwrap();
            let other = [CompiledArg::I(if pressurize == 20 { 15 } else { 20 })];
            assert_eq!(
                super::super::apply_move(
                    entry.kind,
                    &mut MoveCtx {
                        state: &mut forged,
                        catalog: &catalog,
                        actor: 0,
                        args: &other,
                        events: &mut Vec::new(),
                    },
                ),
                Err(EngineRefusal::MalformedArgs("waterfall_pressurize"))
            );
        }
    }

    /// #2828: Increasing Intensity below A9 generates one Wither and gains
    /// `3 + old_additional` Strength: `get_WitherAmount` RVA `0xae814` is
    /// `GetValueIfAscension(9, 2, 1)` and `get_IncreasingIntensityBaseStrength`
    /// RVA `0xae809` is `(9, 4, 3)`. The boss HP gate is A8 (535/512), so at
    /// A8 the roster is the A8+ one while the move is the below-A9 one.
    #[test]
    fn aeonglass_intensity_below_a9_generates_one_wither_and_gains_three() {
        for (ascension, withers, gain) in [(8, 1, 3), (9, 2, 4)] {
            let mut builder = CatalogBuilder::new();
            assert!(builder.set_ascension(ascension));
            builder.intern_monster(MonsterKind::Aeonglass).unwrap();
            builder
                .intern_reachable(CardIdentity {
                    id: CardId::Wither,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let (_, mut state) = aeonglass_fixture(2, 2);
            assert!(state.fanouts.set_ascension(ascension));
            let row = catalog
                .moves(MonsterKind::Aeonglass)
                .iter()
                .find(|row| row.kind == MoveKind::AeonglassIntensity)
                .copied()
                .unwrap();
            assert_eq!(
                catalog.args(row.args),
                [CompiledArg::I(gain), CompiledArg::I(withers)]
            );
            aeonglass_intensity(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(row.args),
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(
                state.piles.get(PileId::Discard).as_slice().len(),
                usize::try_from(withers).unwrap(),
                "A{ascension}"
            );
            assert_eq!(
                state.next_generated_hook_uid,
                i32::try_from(withers).unwrap()
            );
            // Old additional Strength 2: gain + 2.
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                i32::try_from(gain).unwrap() + 2,
                "A{ascension}"
            );
            assert_eq!(state.monsters[0].aeonglass_additional_strength(), 3);
            assert!(crate::engine::cards::aeonglass_state_is_exact(
                &state, &catalog
            ));
        }
    }

    /// #2828: Test Subject Multi-Claw attacks from its row, whose damage is
    /// `TestSubject::get_MultiClawDamage` RVA `0xc019b`,
    /// `GetValueIfAscension(9, 11, 10)`, with `3 + extra` hits — the hit
    /// count grows in the body and is not tiered.
    #[test]
    fn test_subject_multi_claw_hits_at_its_rows_tier() {
        for (ascension, damage) in [(8, 10), (9, 11)] {
            let mut builder = CatalogBuilder::new();
            assert!(builder.set_ascension(ascension));
            builder.intern_monster(MonsterKind::TestSubject).unwrap();
            let catalog = builder.build();
            let row = catalog
                .moves(MonsterKind::TestSubject)
                .iter()
                .find(|row| row.kind == MoveKind::TestSubjectMultiClaw)
                .copied()
                .unwrap();
            assert_eq!(
                catalog.args(row.args),
                [CompiledArg::I(damage), CompiledArg::I(3)]
            );
            let mut state = live_test_subject(1, 3);
            assert!(state.fanouts.set_ascension(ascension));
            let hp = state.hp;
            test_subject_multi_claw(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(row.args),
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(
                hp - state.hp,
                3 * i32::try_from(damage).unwrap(),
                "A{ascension}"
            );
        }
    }
}
