//! Card-step bodies used by more than one content family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1339 + #1394 + #1469 + #1473 + #1363 + #1563 + #1561 + #1751 + #1960): 43 of 43 ported
//!
//! All 43 owned kinds have been triaged against the current engine. The first
//! pass ported the ordinary attack/block/draw/resource commands; later slices
//! added orb slots, player-power folds, serial all-target powers, Stars, and
//! this exact calculated-operand resolver, the generic generated selection
//! gate, the deferred-energy power, and the attack-command random/result/
//! context modes plus temporary Strength, and Volley's bounded Energy-X
//! random attack, followed by the exact four-row fatal-attack reward gate.
//! `heal` and `generate_fixed_status` additionally have no admissible source card, so
//! claiming either would be green only because `must_cover` cannot exercise
//! it.

use super::StepCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::engine::admission::step_args;
use crate::engine::cards::{
    inject_generated_bottom, inject_generated_record_before_ending_bottom,
    inject_generated_record_before_ending_draw_random,
};
use crate::engine::damage::{
    AttackContextMode, alive_targets, apply_card_monster_debuff,
    apply_card_monster_debuff_with_catalog, apply_card_monster_strength_delta,
    apply_monster_temp_strength_wrapper_after_type_two_gate, apply_owner_strength,
    card_monster_type_two_amount, gain_powered_card_block, note_power, player_attack_all_from_card,
    player_attack_context_from_card, player_attack_from_card, player_attack_random_from_card,
    player_attack_results_from_card,
};
use crate::engine::{EngineRefusal, Subject, fire_hook};
use crate::hooks::HookEvent;
use crate::hot::{MiseryToken, OrbKind, OrbResetPower, PileId};
use crate::ids::{CardId, PowerId, StepKind, StepWord};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// `attack`, `block` and `vulnerable` are R0.5's slice (#1289), moved here
/// unchanged when R0.6 gave the dispatch its generated home;
/// `generate_fixed_shivs` landed with the generation seam. The #1339 wave's
/// first pass adds the six whose primitives engine slices 1–3 supplied:
/// the multi-target attack command, the resolved Energy-X, the step-callable
/// Draw, the card self-damage pipeline, and the card-sourced monster debuff.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::AddOrbSlotsExact,
    StepKind::Attack,
    StepKind::AttackAll,
    StepKind::AttackAllTempStrengthSnapshot,
    StepKind::AttackContextResultExact,
    StepKind::AttackRandom,
    StepKind::AttackRandomX,
    StepKind::AttackResult,
    StepKind::AttackX,
    StepKind::Block,
    StepKind::Calcify,
    StepKind::CalculatedAttack,
    StepKind::CalculatedBlock,
    StepKind::CalculatedHits,
    StepKind::Channel,
    StepKind::Dexterity,
    StepKind::Draw,
    StepKind::Energy,
    StepKind::EnergyNextTurn,
    StepKind::FatalAttackRewardExact,
    StepKind::GenerateFixedShivs,
    StepKind::GenerateFixedStatus,
    StepKind::HauntPower,
    StepKind::Heal,
    StepKind::HpLoss,
    StepKind::Lethality,
    StepKind::PowerAllSerial,
    StepKind::Select,
    StepKind::SerpentForm,
    StepKind::Shroud,
    StepKind::SoulBody,
    StepKind::SpectrumShift,
    StepKind::TempStrength,
    StepKind::ToolsOfTheTrade,
    StepKind::Tyranny,
    StepKind::Shadowmeld,
    StepKind::Spinner,
    StepKind::Stars,
    StepKind::Strength,
    StepKind::Summon,
    StepKind::Vicious,
    StepKind::Vulnerable,
    StepKind::Weak,
];

/// This family's argument shapes, destructured once per body.
///
/// Since #1366 the gate holds no shape opinion on a wave kind: the body owns
/// its destructure and returns a typed refusal on surprise, which the
/// differential's `must_cover` makes visible before merge.
fn one_int(ctx: &StepCtx<'_>, site: &'static str) -> Result<i64, EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(value)] => Ok(*value),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn two_ints(ctx: &StepCtx<'_>, site: &'static str) -> Result<(i64, i64), EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(first), CompiledArg::I(second)] => Ok((*first, *second)),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

/// Capacitor/Modded exact current-capacity growth, capped at ten.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — behind a CAPACITOR/MODDED exact-body
/// allowlist, `add_orb_slots(s, step[1], log)`.
///
pub(crate) fn add_orb_slots_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "add_orb_slots_exact")?;
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Capacitor, 0) => 2,
        (CardId::Capacitor, 1) => 3,
        (CardId::Modded, 0 | 1) => 1,
        _ => return Err(EngineRefusal::MalformedArgs("add_orb_slots_exact")),
    };
    if amount != expected {
        return Err(EngineRefusal::MalformedArgs("add_orb_slots_exact"));
    }
    crate::engine::orbs::add_slots(ctx.state, amount)
}

/// `("attack", damage, hits)` — a powered attack on the chosen target.
///
/// Python: `_run_steps_inner`'s attack branch. `card_damage(raw)` is
/// `_powered_card_damage_base`: the base plus enchantment, Instinct and relic
/// additives, every one of which needs content the gate refuses, so the row's
/// own base stands.
pub(crate) fn attack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits) =
        step_args(StepKind::Attack, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// `("attack_all", damage, hits)` — one all-opponents attack command.
///
/// A `TargetingAllOpponents` `AttackCommand` re-resolves its receivers on
/// every hit: `AttackCommand.<Execute>d__90::MoveNext` (RVA `0x3f19c0`) calls
/// `GetPossibleTargets` (`0x1348ac`, `GetOpponentsOf` at IL_005c) at IL_0168
/// inside the per-hit loop and filters it by `IsAlive` into `validTargets`
/// (IL_0196). A creature spawned by an earlier hit's death is a receiver of
/// the later hits, and the loop stops once no opponent lives (#3023; the
/// frozen Python captured the roster once, which this corrects).
/// `card_damage` is `_powered_card_damage_base`: the enchantment, Instinct
/// and relic additives it folds are all refused content, so the row's base
/// stands.
pub(crate) fn attack_all(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits) = two_ints(ctx, "attack_all")?;
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        hits,
        ctx.events,
    )
}

/// Crush Under / Dying Star: freeze the living identities, run one all-enemy
/// attack, then apply negative temporary Strength to the surviving snapshots.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Crush Under / Dying Star: a
/// uid-frozen `(monster, uid)` snapshot before the all-opponents attack,
/// then `_apply_temp_strength_enemy` serially on the surviving identities.
///
/// Slice 5 supplied the complete hot representation: the negative wrapper is
/// stored as [`PowerId::TempStrength`], monster attacks read the paired
/// Strength delta, and enemy side end removes both. `TempStrengthEnemy` is the
/// generated argument marker only; it is never a canonical power slot.
pub(crate) fn attack_all_temp_strength_snapshot(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    let (damage, strength) = two_ints(ctx, "attack_all_temp_strength_snapshot")?;
    let exact = matches!(
        (
            ctx.spec.identity.id,
            ctx.spec.identity.upgrade,
            damage,
            strength
        ),
        (CardId::CrushUnder, 0, 8, 1)
            | (CardId::CrushUnder, 1, 9, 2)
            | (CardId::DyingStar, 0, 9, 9)
            | (CardId::DyingStar, 1, 11, 11)
    );
    if !exact {
        return Err(EngineRefusal::MalformedArgs(
            "attack_all_temp_strength_snapshot",
        ));
    }
    let snapshot: Vec<(usize, u32)> = alive_targets(ctx.state)
        .into_iter()
        .map(|target| (target, ctx.state.monsters[target].uid))
        .collect();
    let targets: Vec<usize> = snapshot.iter().map(|(target, _)| *target).collect();
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &targets,
        damage,
        1,
        ctx.events,
    )?;
    let amount: i32 = strength
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("temporary monster strength"))?;
    let model = temp_strength_enemy_model(ctx.spec.identity.id)?;
    for (target, uid) in snapshot {
        if ctx
            .state
            .monsters
            .get(target)
            .is_some_and(|monster| monster.uid == uid)
        {
            apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)?;
        }
    }
    Ok(())
}

/// The concrete `TemporaryStrengthPower` subclass a card-sourced enemy
/// temporary-Strength application attaches (#2693 S2).
///
/// Derived from `content_tables`, not transcribed: the six cards below are
/// exactly the ones whose generated steps name `StepKind::TempStrengthEnemy`,
/// `StepKind::AttackAllTempStrengthSnapshot` or
/// `PowerAllSerial(PowerId::TempStrengthEnemy)`, and each maps to the one
/// native class its own `OnPlay` body applies. The other two members of the
/// family are not card plays — `MonarchsGazeStrengthDownPower` comes from a
/// player power's `AfterDamageGiven`
/// (`MonarchsGazePower/<AfterDamageGiven>d__4::MoveNext` `0x33e544`) and
/// `ShacklingPotionPower` from a potion
/// (`ShacklingPotion/<OnUse>d__10::MoveNext` `0x34ffb8`), so neither reaches
/// this seam.
pub(crate) fn temp_strength_enemy_model(
    card: CardId,
) -> Result<crate::hot::AttachedPowerModel, EngineRefusal> {
    Ok(match card {
        // `CrushUnder/<OnPlay>d__6::MoveNext` `0x395cac` IL_01d5.
        CardId::CrushUnder => crate::hot::AttachedPowerModel::CrushUnder,
        // `DarkShackles/<OnPlay>d__8::MoveNext` `0x396954` IL_00e6.
        CardId::DarkShackles => crate::hot::AttachedPowerModel::DarkShackles,
        // `DyingStar/<OnPlay>d__10::MoveNext` `0x39ac44` IL_01af.
        CardId::DyingStar => crate::hot::AttachedPowerModel::DyingStar,
        // `EnfeeblingTouch/<OnPlay>d__8::MoveNext` `0x39bb64` IL_00e6.
        CardId::EnfeeblingTouch => crate::hot::AttachedPowerModel::EnfeeblingTouch,
        // `Mangle/<OnPlay>d__6::MoveNext` `0x3ab670` IL_013c.
        CardId::Mangle => crate::hot::AttachedPowerModel::Mangle,
        // `PiercingWail/<OnPlay>d__8::MoveNext` `0x3b253c` IL_00f3.
        CardId::PiercingWail => crate::hot::AttachedPowerModel::PiercingWail,
        _ => {
            return Err(EngineRefusal::MalformedArgs(
                "temporary monster Strength wrapper model",
            ));
        }
    })
}

/// One card-sourced negative TemporaryStrengthPower wrapper.
///
/// The outer visible Type-2 wrapper reaches Lamp and Artifact exactly once.
/// Only a successful wrapper writes its matching internal signed Strength;
/// the wrapper itself is temporary and does not notify
/// AfterPowerAmountChanged, while its nested Strength application does.
///
/// `model` names the concrete subclass (#2693 S2). It decides whether this
/// application stacks onto an instance the monster already carries or attaches
/// a second one at its own ledger position: every member takes the base
/// `PowerModel::get_InstanceType` `0x83751` = 0, so
/// `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058 looks the
/// instance up by model id alone. The applier — `card.Owner.Player.Creature`
/// at `DarkShackles/<OnPlay>d__8::MoveNext` `0x396954` IL_00d9-IL_00df, and
/// the same at every sibling — is the player, and
/// `TemporaryStrengthPower/<BeforeApplied>d__20::MoveNext` `0x348d20`
/// IL_003e-IL_004b forwards it to the nested `Apply<StrengthPower>`.
///
/// The wrapper is one `PowerCmd.Apply` (Piercing Wail `0x3b253c` IL_00f3, Dark
/// Shackles `0x396954`), and `<Apply>d__1`1` (`0x3ef988`) returns at `IsEnding`
/// (IL_0025-002a) before Artifact, Lamp or the nested Strength run. So the gate
/// is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn apply_temp_strength_enemy(
    state: &mut crate::hot::HotState,
    target: usize,
    model: crate::hot::AttachedPowerModel,
    amount: i32,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if crate::engine::damage::damage_combat_is_ending(state) || monster.hp <= 0 {
        return Ok(());
    }
    let checkpoint = state.clone();
    let event_checkpoint = events.len();
    let result = (|| {
        let Some(amount) = card_monster_type_two_amount(state, target, amount, events)? else {
            return Ok(());
        };
        apply_monster_temp_strength_wrapper_after_type_two_gate(
            state,
            // The card seam has no catalog to hand the nested Strength
            // application's listener walk, exactly as before #2693 S3; the
            // `Misery` clone does, which is why the parameter exists.
            None,
            target,
            model,
            amount,
            crate::hot::Applier::Player,
            events,
        )
    })();
    if result.is_err() {
        *state = checkpoint;
        events.truncate(event_checkpoint);
    }
    result
}

/// `attack_context_result_exact` — Echoing Slash / Omnislice.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Echoing Slash / Omnislice, behind a
/// W194 exact-body allowlist: one `player_attack` carrying a
/// `context_result_mode` that reshapes the command's targeting/result flow.
///
/// Python: `player_attack` (frozen, deleted #2827).
/// Echoing Slash refreshes the live all-enemy wave and adds one pending wave
/// per killed result; Omnislice spills the first result's total plus overkill
/// as blockable unpowered damage before the shared AfterAttack close. The
/// spill is one `CreatureCmd.Damage` batch whose card source is the Omnislice,
/// so each Thorns holder it enters retaliates (#3612,
/// `engine::damage::omnislice_spill` carries the IL).
pub(crate) fn attack_context_result_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (mode, damage) = match (
        ctx.spec.identity.id,
        ctx.spec.identity.upgrade,
        ctx.args,
        ctx.target,
    ) {
        (
            CardId::EchoingSlash,
            0,
            [
                CompiledArg::Word(StepWord::EchoingSlash),
                CompiledArg::I(10),
            ],
            None,
        ) => (AttackContextMode::EchoingSlash, 10),
        (
            CardId::EchoingSlash,
            1,
            [
                CompiledArg::Word(StepWord::EchoingSlash),
                CompiledArg::I(13),
            ],
            None,
        ) => (AttackContextMode::EchoingSlash, 13),
        (
            CardId::Omnislice,
            0,
            [CompiledArg::Word(StepWord::Omnislice), CompiledArg::I(8)],
            Some(target),
        ) => {
            let _ = target;
            (AttackContextMode::Omnislice, 8)
        }
        (
            CardId::Omnislice,
            1,
            [CompiledArg::Word(StepWord::Omnislice), CompiledArg::I(11)],
            Some(target),
        ) => {
            let _ = target;
            (AttackContextMode::Omnislice, 11)
        }
        _ => return Err(EngineRefusal::MalformedArgs("attack_context_result_exact")),
    };
    let one_target;
    let targets = if let Some(target) = ctx.target {
        one_target = [target];
        &one_target[..]
    } else {
        &[][..]
    };
    player_attack_context_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        targets,
        damage,
        mode,
        ctx.events,
    )
}

/// `attack_random` — one random-target powered attack command.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Ricochet / Rip and Tear / Sword
/// Boomerang: **one** attack command with per-hit random targeting
/// (`random_targets=True`), so Vigor/Pen Nib and Before/AfterAttack wrap the
/// whole hit series exactly once.
///
/// Python: `player_attack` (frozen, deleted #2827) and `_run_steps_inner`. The live pool and one `CombatTargets` draw are refreshed
/// per executed hit while Vigor and AfterAttack wrap the whole command.
/// Ricochet's exact native-Sly rows enter through the separately
/// registry-locked shared discard/AutoPlay lifecycle in `engine::draw`.
pub(crate) fn attack_random(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Ricochet, 0, [CompiledArg::I(3), CompiledArg::I(4)])
        | (CardId::RipAndTear, 1, [CompiledArg::I(3), CompiledArg::I(4)]) => (3, 4),
        (CardId::Ricochet, 1, [CompiledArg::I(3), CompiledArg::I(5)]) => (3, 5),
        (CardId::RipAndTear, 0, [CompiledArg::I(3), CompiledArg::I(3)]) => (3, 3),
        (CardId::SwordBoomerang, 0, [CompiledArg::I(3), CompiledArg::I(3)]) => (3, 3),
        (CardId::SwordBoomerang, 1, [CompiledArg::I(3), CompiledArg::I(4)]) => (3, 4),
        _ => return Err(EngineRefusal::MalformedArgs("attack_random")),
    };
    if ctx.target.is_some() {
        return Err(EngineRefusal::TargetMismatch { required: false });
    }
    player_attack_random_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        hits,
        ctx.events,
    )
}

/// `attack_random_x` — Volley's Energy-X random-target attack.
///
/// Current v0.111.0/41cef1ea IL: `Volley/<OnPlay>d__5::MoveNext`
/// (RVA 0x3c6b7c) builds one powered AttackCommand from Damage 10/14, uses
/// `ResolveEnergyXValue` as its hit count, then calls
/// `TargetingRandomOpponents(..., true)`. Python `_run_steps_inner` (frozen, deleted #2827)
/// uses the same live-pool/one-CombatTargets-draw-per-executed-hit command.
///
/// Volley resolves Energy-X while Stardust resolves Star-X. Current v0.111
/// `Volley/<OnPlay>d__5::MoveNext` RVA `0x3c6b7c` and
/// `Stardust/<OnPlay>d__5::MoveNext` RVA `0x3bee38` build the same one-command
/// random-opponent attack after resolving their distinct captured resource.
/// The play layer owns that capture and this body authenticates all four
/// canonical rows, so the public derived kind cannot authorize a forged
/// Energy-X/Star-X carrier or operand.
pub(crate) fn attack_random_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Volley, 0, [CompiledArg::I(10)]) => 10,
        (CardId::Volley, 1, [CompiledArg::I(14)]) => 14,
        (CardId::Stardust, 0, [CompiledArg::I(5)]) => 5,
        (CardId::Stardust, 1, [CompiledArg::I(7)]) => 7,
        _ => return Err(EngineRefusal::MalformedArgs("attack_random_x")),
    };
    if ctx.target.is_some() || ctx.x_value < 0 {
        return Err(EngineRefusal::MalformedArgs("attack_random_x"));
    }
    player_attack_random_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        ctx.x_value,
        ctx.events,
    )
}

/// `attack_result` — the four exact result-consuming card bodies.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Blight Strike / Fisticuffs /
/// Knockout Blow / Sunder: one awaited attack whose command-private Results
/// list (`result_sink`) feeds a doom-total, block-total-overkill,
/// stars-on-kill or energy-on-kill consumer.
///
/// Python: `damage_monster` (frozen, deleted #2827) publishes blocked, unblocked,
/// overkill, killed, and pre-death receiver identity before death handling;
/// `_run_steps_inner` consumes that private list for Doom,
/// powered Block, Stars, or Energy. Exact card/level/operand rows are
/// allowlisted here so the result channel cannot become a generic shortcut.
///
/// Sunder's reward is `PlayerCmd.GainEnergy` (`Sunder/<OnPlay>d__5` `0x3c06e4`
/// IL_014e), whose `<GainEnergy>d__3` (`0x3ee8a0`) returns at `IsEnding`
/// (IL_0035-003c). It follows a kill, and the engine latches `history.over`
/// at a killing blow that leaves no living primary, so `history.over` is that
/// gate here; an ending combat never reaches the kill (#3515).
pub(crate) fn attack_result(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let (damage, outcome, reward) =
        match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
            (
                CardId::BlightStrike,
                0,
                [
                    CompiledArg::I(8),
                    CompiledArg::Word(StepWord::DoomTotal),
                    CompiledArg::I(0),
                ],
            ) => (8, StepWord::DoomTotal, 0),
            (
                CardId::BlightStrike,
                1,
                [
                    CompiledArg::I(10),
                    CompiledArg::Word(StepWord::DoomTotal),
                    CompiledArg::I(0),
                ],
            ) => (10, StepWord::DoomTotal, 0),
            (
                CardId::Fisticuffs,
                0,
                [
                    CompiledArg::I(7),
                    CompiledArg::Word(StepWord::BlockTotalOverkill),
                    CompiledArg::I(0),
                ],
            ) => (7, StepWord::BlockTotalOverkill, 0),
            (
                CardId::Fisticuffs,
                1,
                [
                    CompiledArg::I(9),
                    CompiledArg::Word(StepWord::BlockTotalOverkill),
                    CompiledArg::I(0),
                ],
            ) => (9, StepWord::BlockTotalOverkill, 0),
            (
                CardId::KnockoutBlow,
                0,
                [
                    CompiledArg::I(30),
                    CompiledArg::Word(StepWord::StarsOnKill),
                    CompiledArg::I(5),
                ],
            ) => (30, StepWord::StarsOnKill, 5),
            (
                CardId::KnockoutBlow,
                1,
                [
                    CompiledArg::I(38),
                    CompiledArg::Word(StepWord::StarsOnKill),
                    CompiledArg::I(5),
                ],
            ) => (38, StepWord::StarsOnKill, 5),
            (
                CardId::Sunder,
                0,
                [
                    CompiledArg::I(26),
                    CompiledArg::Word(StepWord::EnergyOnKill),
                    CompiledArg::I(3),
                ],
            ) => (26, StepWord::EnergyOnKill, 3),
            (
                CardId::Sunder,
                1,
                [
                    CompiledArg::I(34),
                    CompiledArg::Word(StepWord::EnergyOnKill),
                    CompiledArg::I(3),
                ],
            ) => (34, StepWord::EnergyOnKill, 3),
            _ => return Err(EngineRefusal::MalformedArgs("attack_result")),
        };
    let target_uid = ctx
        .state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?
        .uid;
    let results = player_attack_results_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    match outcome {
        StepWord::DoomTotal => {
            let amount = results.iter().try_fold(0_i32, |sum, result| {
                sum.checked_add(result.total_damage)
                    .ok_or(EngineRefusal::CounterOverflow("attack result doom"))
            })?;
            if amount > 0
                && !ctx.state.history.over
                && ctx
                    .state
                    .monsters
                    .get(target)
                    .is_some_and(|monster| monster.hp > 0 && monster.uid == target_uid)
                && results
                    .iter()
                    .all(|result| result.target == target && result.receiver_uid == target_uid)
            {
                apply_card_monster_debuff(
                    ctx.state,
                    target,
                    PowerId::Doom,
                    MiseryToken::Doom,
                    amount,
                    ctx.events,
                )?;
            }
        }
        StepWord::BlockTotalOverkill => {
            let amount = results.iter().try_fold(0_i64, |sum, result| {
                sum.checked_add(i64::from(result.total_damage) + i64::from(result.overkill_damage))
                    .ok_or(EngineRefusal::CounterOverflow("attack result block"))
            })?;
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, amount, ctx.events)?;
        }
        StepWord::StarsOnKill if results.iter().any(|result| result.was_target_killed) => {
            crate::engine::play::gain_stars(ctx.state, ctx.catalog, reward, ctx.events)?;
        }
        StepWord::EnergyOnKill
            if !ctx.state.history.over && results.iter().any(|result| result.was_target_killed) =>
        {
            let reward: i16 = reward
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("attack result energy"))?;
            ctx.state.energy = ctx
                .state
                .energy
                .checked_add(reward)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
        }
        StepWord::StarsOnKill | StepWord::EnergyOnKill => {}
        _ => return Err(EngineRefusal::MalformedArgs("attack_result outcome")),
    }
    Ok(())
}

/// `("attack_x", damage)` — Skewer / Eradicate: X hits at the chosen target.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). One powered, targeted attack command
/// whose hit count is the play's captured X. The command still runs at
/// X = 0: `BeforeAttack` latches Vigor and `AfterAttack` consumes it after
/// the empty hit loop, which is why [`player_attack`] runs its outer loop
/// zero times rather than this body skipping the call.
pub(crate) fn attack_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = one_int(ctx, "attack_x")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        ctx.x_value,
        ctx.events,
    )
}

/// `("block", amount)` — powered player block.
///
/// Python: `_run_steps_inner`'s block branch, which routes through the same
/// gain helper that fires the `AfterBlockGained` fan-out (D5).
pub(crate) fn block(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (amount, _) = step_args(StepKind::Block, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, amount, ctx.events)?;
    fire_hook(
        ctx.catalog,
        HookEvent::AfterBlockGained,
        ctx.state,
        ctx.events,
    )
}

/// `calcify` — acquire the exact owner-side pet-attack modifier.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — an over-gated Intensity stack onto
/// the owner's CalcifyPower, whose dealer/owner/powered gates live at
/// `player_attack`.
///
/// The separate pet attack entry point is the only reader: ordinary player
/// card attacks therefore cannot observe this amount.
///
/// v0.111.0 `Calcify/<OnPlay>` RVA `0x390c7c` awaits `PowerCmd.Apply<CalcifyPower>` at IL_00d1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn calcify(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Calcify, 0, [CompiledArg::I(4)]) => 4,
        (CardId::Calcify, 1, [CompiledArg::I(6)]) => 6,
        _ => return Err(EngineRefusal::MalformedArgs("calcify")),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let updated = ctx
        .state
        .powers
        .value(PowerId::Calcify)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("calcify"))?;
    ctx.state
        .powers
        .set(PowerId::Calcify, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Calcify, updated);
    Ok(())
}

/// Evaluate one exact-card/source allowlisted calculated operand.
///
/// Python: `_calculated_card_amount` (frozen, deleted #2827) and `_calculated_card_hits`. The two registries contain 19 identities (both upgrades): fourteen
/// amount sources and five hit-count sources. Unknown identities, kinds,
/// sources, or argument shapes fail before reading mutable state (I5).
/// `calculated_history` deliberately does not call either resolver and remains
/// outside this slice.
fn calculated_step_is_complete(ctx: &StepCtx<'_>, kind: StepKind) -> bool {
    let [step] = ctx.catalog.steps(ctx.spec) else {
        return false;
    };
    step.kind == kind && ctx.catalog.args(step.args) == ctx.args
}

pub(crate) fn calculated_operand(ctx: &StepCtx<'_>, kind: StepKind) -> Result<i64, EngineRefusal> {
    let malformed = || EngineRefusal::MalformedArgs(kind.as_str());
    let id = ctx.spec.identity.id;
    let upgrade = ctx.spec.identity.upgrade;
    if upgrade > 1 {
        return Err(malformed());
    }

    if kind == StepKind::CalculatedHits {
        let [CompiledArg::I(damage), CompiledArg::Word(source)] = ctx.args else {
            return Err(malformed());
        };
        let expected = match id {
            CardId::Barrage => StepWord::OrbCount,
            CardId::Dismantle => StepWord::TargetVulnerableHits,
            CardId::Finisher => StepWord::FinishedAttackCount,
            CardId::Flechettes => StepWord::HandSkillCount,
            CardId::LunarBlast => StepWord::FinishedSkillCount,
            _ => return Err(malformed()),
        };
        if *source != expected
            || (id == CardId::Dismantle
                && (*damage != if upgrade == 0 { 8 } else { 10 }
                    || !calculated_step_is_complete(ctx, kind)))
        {
            return Err(malformed());
        }
        return match source {
            StepWord::OrbCount => i64::try_from(ctx.state.orbs.as_slice().len())
                .map_err(|_| EngineRefusal::CounterOverflow("calculated_hits orb_count")),
            StepWord::HandSkillCount => ctx
                .state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .try_fold(0_i64, |count, card| {
                    let spec = ctx
                        .catalog
                        .spec(card.atom)
                        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                    Ok(count + i64::from(spec.is_skill))
                }),
            StepWord::FinishedAttackCount => {
                Ok(i64::from(ctx.state.history.attack_plays_finished_this_turn))
            }
            StepWord::FinishedSkillCount => {
                Ok(i64::from(ctx.state.history.skill_plays_finished_this_turn))
            }
            StepWord::TargetVulnerableHits => Ok(i64::from(
                ctx.target
                    .and_then(|target| ctx.state.monsters.get(target))
                    .is_some_and(|monster| monster.powers.value(PowerId::Vuln) > 0),
            ) + 1),
            _ => Err(malformed()),
        };
    }

    let [
        CompiledArg::Word(source),
        CompiledArg::I(base),
        CompiledArg::I(extra),
    ] = ctx.args
    else {
        return Err(malformed());
    };
    let expected = match (id, kind) {
        (CardId::AshenStrike, StepKind::CalculatedAttack) => StepWord::ExhaustCount,
        (CardId::BodySlam, StepKind::CalculatedAttack) => StepWord::PlayerBlock,
        (CardId::DeathMarch, StepKind::CalculatedAttack) => StepWord::NonHandDrawsThisTurn,
        (CardId::GoldAxe, StepKind::CalculatedAttack) => StepWord::CardPlaysFinishedCombat,
        (CardId::MementoMori, StepKind::CalculatedAttack) => StepWord::DiscardedCardsThisTurn,
        (CardId::MindBlast, StepKind::CalculatedAttack) => StepWord::DrawCount,
        (CardId::Murder, StepKind::CalculatedAttack) => StepWord::CardsDrawnCombat,
        (CardId::PreciseCut, StepKind::CalculatedAttack) => StepWord::NegativeHandCount,
        (CardId::SoulStorm, StepKind::CalculatedAttack) => StepWord::ExhaustExactSoul,
        (CardId::Supermassive, StepKind::CalculatedAttack) => StepWord::OwnerGeneratedCardsCombat,
        (CardId::TimesUp, StepKind::CalculatedAttack) => StepWord::TargetDoom,
        (CardId::ExpectAFight, StepKind::CalculatedBlock) => StepWord::StrengthPowerAmount,
        (CardId::Stack, StepKind::CalculatedBlock) => StepWord::DiscardCount,
        (CardId::NoEscape, StepKind::CalculatedDoomExact) => StepWord::TargetDoomTens,
        _ => return Err(malformed()),
    };
    if *source != expected {
        return Err(malformed());
    }

    // These three Python rows additionally authenticate their complete body.
    let exact_special_body = match id {
        CardId::PreciseCut => (*base, *extra) == (if upgrade == 0 { 13 } else { 16 }, 2),
        CardId::ExpectAFight => {
            (*base, *extra)
                == (
                    if upgrade == 0 { 15 } else { 16 },
                    if upgrade == 0 { 5 } else { 8 },
                )
        }
        CardId::NoEscape => (*base, *extra) == (if upgrade == 0 { 10 } else { 15 }, 5),
        _ => true,
    };
    if !exact_special_body
        || (matches!(
            id,
            CardId::PreciseCut | CardId::ExpectAFight | CardId::NoEscape
        ) && !calculated_step_is_complete(ctx, kind))
    {
        return Err(malformed());
    }

    let pile_len = |pile| {
        i64::try_from(ctx.state.piles.get(pile).len())
            .map_err(|_| EngineRefusal::CounterOverflow("calculated operand pile length"))
    };
    let target_power = |power| {
        ctx.target
            .and_then(|target| ctx.state.monsters.get(target))
            .map_or(0_i64, |monster| i64::from(monster.powers.value(power)))
    };
    let multiplier = match source {
        StepWord::PlayerBlock => i64::from(ctx.state.block),
        StepWord::DrawCount => pile_len(PileId::Draw)?,
        StepWord::DiscardCount => pile_len(PileId::Discard)?,
        StepWord::StrengthPowerAmount => (i64::from(ctx.state.powers.value(PowerId::Strength))
            + i64::from(ctx.state.powers.value(PowerId::TempStrength)))
        .max(0),
        StepWord::ExhaustCount => pile_len(PileId::Exhaust)?,
        StepWord::ExhaustExactSoul => ctx
            .state
            .piles
            .get(PileId::Exhaust)
            .as_slice()
            .iter()
            .try_fold(0_i64, |count, card| {
                let spec = ctx
                    .catalog
                    .spec(card.atom)
                    .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                Ok::<i64, EngineRefusal>(
                    count + i64::from(matches!(spec.identity.id, CardId::Soul)),
                )
            })?,
        StepWord::TargetDoom => target_power(PowerId::Doom),
        StepWord::NonHandDrawsThisTurn => i64::from(ctx.state.history.non_hand_draws_this_turn),
        StepWord::CardPlaysFinishedCombat => {
            i64::from(ctx.state.history.card_plays_finished_combat)
        }
        StepWord::DiscardedCardsThisTurn => i64::from(ctx.state.history.discarded_cards_this_turn),
        StepWord::CardsDrawnCombat => i64::from(ctx.state.cards_drawn_combat),
        StepWord::OwnerGeneratedCardsCombat => {
            i64::from(ctx.state.history.owner_generated_cards_combat)
        }
        StepWord::NegativeHandCount => {
            pile_len(PileId::Hand)?
                .checked_neg()
                .ok_or(EngineRefusal::CounterOverflow(
                    "calculated negative hand count",
                ))?
        }
        StepWord::TargetDoomTens => target_power(PowerId::Doom).div_euclid(10),
        _ => return Err(malformed()),
    };
    extra
        .checked_mul(multiplier)
        .and_then(|scaled| base.checked_add(scaled))
        .ok_or(EngineRefusal::CounterOverflow("calculated operand"))
}

/// `calculated_attack` — one exact live calculated amount, then one powered
/// targeted attack command.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). AttackCommand's ending gate precedes
/// calculation; a completed fight therefore skips without reading the
/// operand.
pub(crate) fn calculated_attack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.state.history.over {
        return Ok(());
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let amount = calculated_operand(ctx, StepKind::CalculatedAttack)?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        amount,
        1,
        ctx.events,
    )
}

/// `calculated_block` — one exact live calculated amount, then one powered
/// player block command and its ordinary AfterBlockGained fan-out.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The ending gate precedes calculation.
pub(crate) fn calculated_block(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.state.history.over {
        return Ok(());
    }
    let amount = calculated_operand(ctx, StepKind::CalculatedBlock)?;
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, amount, ctx.events)?;
    fire_hook(
        ctx.catalog,
        HookEvent::AfterBlockGained,
        ctx.state,
        ctx.events,
    )
}

/// `calculated_hits` — one live calculated hit count, then one powered
/// targeted attack command.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The command deliberately runs even at
/// zero hits so its BeforeAttack/AfterAttack lifecycle remains observable.
pub(crate) fn calculated_hits(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.state.history.over {
        return Ok(());
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let damage = match ctx.args {
        [CompiledArg::I(damage), CompiledArg::Word(_)] => *damage,
        _ => return Err(EngineRefusal::MalformedArgs("calculated_hits")),
    };
    let hits = calculated_operand(ctx, StepKind::CalculatedHits)?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// `("channel", "LIGHTNING", count)` — sequential Lightning channels.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `channel(s, step[1], log)` per count,
/// with the zero-slot bootstrap and auto-evoke-oldest-when-full semantics.
///
/// Issue #1394 slice 4.2 admits only Lightning's argument shape. Each
/// iteration re-enters the command's terminal gate; a full-queue Lightning
/// evoke that becomes lethal still finishes that one channel before the next
/// loop iteration observes combat over.
fn channel_source_is_exact(ctx: &StepCtx<'_>, kind: OrbKind, count: i64) -> bool {
    let id = ctx.spec.identity.id;
    let upgrade = ctx.spec.identity.upgrade;
    matches!(
        (id, upgrade, kind, count),
        (
            CardId::Zap | CardId::BallLightning,
            0 | 1,
            OrbKind::Lightning,
            1
        ) | (
            CardId::Rainbow,
            0 | 1,
            OrbKind::Lightning | OrbKind::Frost | OrbKind::Dark,
            1
        ) | (CardId::Glacier, 0 | 1, OrbKind::Frost, 2)
            | (CardId::IceLance, 0 | 1, OrbKind::Frost, 3)
            | (CardId::MeteorStrike, 0 | 1, OrbKind::Plasma, 3)
            | (
                CardId::ColdSnap | CardId::Coolheaded | CardId::Hibernate,
                0 | 1,
                OrbKind::Frost,
                1
            )
            | (
                CardId::Darkness | CardId::Null | CardId::ShadowShield,
                0 | 1,
                OrbKind::Dark,
                1
            )
            | (CardId::Fusion, 0 | 1, OrbKind::Plasma, 1)
            | (CardId::Glasswork, 0 | 1, OrbKind::Glass, 1)
            | (CardId::Refract, 0 | 1, OrbKind::Glass, 2)
            | (CardId::Spinner, 1, OrbKind::Glass, 1)
    )
}

/// `("channel", orb, count)` — `count` `OrbCmd.Channel` commands.
///
/// Each command returns on `IsOverOrEnding` (`OrbCmd/<Channel>d__3::MoveNext`
/// RVA `0x3ed69c` IL_0029-0035; the generic `<Channel>d__2`1` `0x3ed5c4`
/// forwards to it at IL_0032). [`crate::engine::orbs::channel`] carries that
/// gate through the shared IsEnding projection, so once the combat is ending
/// every later iteration is a no-op; the `history.over` break below only
/// stops the loop early and matches native in the ending-but-not-over window
/// too (#3502).
pub(crate) fn channel(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (kind, count) =
        crate::engine::admission::channel_args(ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    if !channel_source_is_exact(ctx, kind, count) {
        return Err(EngineRefusal::MalformedArgs("channel"));
    }
    for _ in 0..count {
        if ctx.state.history.over {
            break;
        }
        crate::engine::orbs::channel(ctx.state, ctx.catalog, kind, ctx.events)?;
    }
    Ok(())
}

/// Mad Science's Expertise amounts (#2942): the Strength or Dexterity the
/// generated Expertise variant row applies, when `ctx.spec` is exactly that
/// row, else `None`.
///
/// `MadScience/<ExecutePower>d__54::MoveNext` (v0.111.0 RVA `0x3aa660`)
/// applies `StrengthPower` of the `ExpertiseStrength` var's BaseValue
/// (IL_00ce-IL_0101) and then `DexterityPower` of `ExpertiseDexterity`
/// (IL_015f-IL_0192), each `PowerCmd.Apply` to the owner's creature with the
/// card as source — the same command every allowlisted row here performs.
/// The amounts are the generated row's own arguments, which
/// `generate_content.py` reads from the canonical vars manifest and
/// `tests/canonical_vars_manifest.rs` pins by role, so no number is
/// restated here.
fn mad_science_expertise_amount(ctx: &StepCtx<'_>, kind: StepKind) -> Option<i64> {
    if !crate::catalog::is_mad_science_variant_program(ctx.spec, "Expertise") {
        return None;
    }
    let mut steps = ctx.spec.row.steps.iter().filter(|step| step.kind == kind);
    match (steps.next(), steps.next()) {
        (
            Some(crate::content_tables::Step {
                args: [crate::content_tables::Arg::I(amount)],
                ..
            }),
            None,
        ) => Some(*amount),
        _ => None,
    }
}

/// Exact additive owner-Dexterity command sources.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack onto the
/// owner's DexterityPower. The command has no combat-ending gate.
///
pub(crate) fn dexterity(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "dexterity")?;
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Abrasive, 0 | 1) => 1,
        (CardId::BulkUp | CardId::Footwork, 0) => 2,
        (CardId::BulkUp | CardId::Footwork, 1) => 3,
        (CardId::Prowess, 0) => 1,
        (CardId::Prowess, 1) => 2,
        (CardId::MadScience, _) => mad_science_expertise_amount(ctx, StepKind::Dexterity)
            .ok_or(EngineRefusal::MalformedArgs("dexterity"))?,
        _ => return Err(EngineRefusal::MalformedArgs("dexterity")),
    };
    if amount != expected {
        return Err(EngineRefusal::MalformedArgs("dexterity"));
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("dexterity"))?;
    // `PowerCmd.Apply<DexterityPower>` returns on `IsEnding`
    // (`<Apply>d__1`1` RVA `0x3ef988` IL_0020-0034). Bulk Up and Prowess apply
    // it after an earlier command that can end the combat (#3495).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let updated = ctx
        .state
        .powers
        .value(PowerId::Dexterity)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("dexterity"))?;
    ctx.state
        .powers
        .set(PowerId::Dexterity, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Dexterity, updated);
    Ok(())
}

/// `("draw", count)` — one Draw command from a card body.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `draw_cards(s, step[1], log,
/// caller="none")` — the plain command form: no caller-locals continuation,
/// no drawn-identity consumer. [`DrawSource::Command`] is the `fromHandDraw`
/// = false overload, which is what bumps `non_hand_draws_this_turn` per
/// moved card; hand-space, reshuffle and the `AfterCardDrawn` fan-out all
/// live inside [`draw_cards`].
pub(crate) fn draw(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = one_int(ctx, "draw")?;
    let count: usize = count
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("draw count"))?;
    crate::engine::play::draw_cardplay_no_result(ctx, count)
}

/// `("energy", amount)` — `PlayerCmd::GainEnergy`.
///
/// v0.111.0 `sts2.dll` (sha `9cb4f1ad…`) `PlayerCmd/<GainEnergy>d__3::MoveNext`
/// RVA `0x3ee8a0`, in order:
/// - IL_0019-002b: returns when `amount <= 0`.
/// - IL_0030-003c: returns on `CombatManager.IsEnding`. Bloodletting and
///   Offering gain after their HP loss, whose listeners (Inferno) can end the
///   combat (#3495).
/// - IL_0062: `Hook.ModifyEnergyGain` (`0x1053e8`) folds every combat hook
///   listener's `ModifyEnergyGain` (IL_002f); AfterModifyingEnergyGain
///   (IL_006e) then notifies the listeners that changed it.
/// - IL_00c8-00fa: gains `finalAmount` only when it is positive.
///
/// The DLL declares exactly one `ModifyEnergyGain` override besides the
/// identity `AbstractModel::ModifyEnergyGain` (`0x7a261`, IL_0001-0002):
/// `NoEnergyGainPower::ModifyEnergyGain` (`0xa4f9d`), which returns zero when
/// the gaining player is the power's owner (IL_0001-0016) (#3502). So the
/// fold is "zero while the local NoEnergyGain flag is set", exactly the
/// Plasma (`orbs::turn_start_passives`) and Tea Set (`relics::after_energy_reset`)
/// gates. Nothing in v0.111.0 applies `NoEnergyGainPower` outside the
/// `ApplyPowerConsoleCmd` developer console (its only references are the
/// `AbstractModelSubtypes` registry `0x84d4c` IL_485a, and `ModelDb.AllPowers`
/// feeds only `Preload` and the console), and admission refuses a local one
/// by name ("NoEnergyGain local GainEnergy modifier"). The gate here keeps
/// this command exact regardless.
pub(crate) fn energy(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "energy")?;
    let amount: i16 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("energy"))?;
    if amount <= 0
        || crate::engine::damage::damage_combat_is_ending(ctx.state)
        || ctx.state.fanouts.no_energy_gain()
    {
        return Ok(());
    }
    ctx.state.energy = ctx
        .state
        .energy
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("energy"))?;
    Ok(())
}

/// `("energy_next_turn", amount)` — stack `EnergyNextTurnPower`.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The Intensity power stacks in place and
/// its `AfterEnergyReset` listener grants the complete saved amount on the
/// next owner turn before removing itself. The sparse power slot is both the
/// canonical field and hot carrier; no dedicated `HotState` word is needed.
///
/// Every carrier's native body awaits the generic `PowerCmd.Apply<T>`
/// (v0.111.0 `sts2.dll` sha `9cb4f1ad…`, card `<OnPlay>` MoveNext RVA then
/// the `Apply<EnergyNextTurnPower>` IL offset): ChargeBattery `0x391dd0`
/// IL_014c; Convergence `0x39469c` IL_0156; Delay `0x3984b8` IL_0151;
/// Hegemony `0x3a44ec` IL_0111; Invoke `0x3a7958` IL_0172; Outmaneuver
/// `0x3b0af8` IL_0050; Relax `0x3b6dd4` IL_01dc; Scavenge `0x3b8540` IL_0147;
/// Sidestep `0x3bbc78` IL_0050. ``PowerCmd/<Apply>d__1`1::MoveNext`` (RVA
/// `0x3ef988`) returns at IL_0020-0x0034 while `CombatManager.IsEnding`, so
/// Hegemony's lethal attack (IL_004c) grants no Energy next turn (#3183).
/// Arguments are validated first, so a malformed row still refuses.
pub(crate) fn energy_next_turn(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "energy_next_turn")?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("energy next turn"))?;
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("energy_next_turn"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let current = ctx.state.powers.value(PowerId::EnergyNextTurn);
    let stacked = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("energy next turn"))?;
    ctx.state
        .powers
        .set(PowerId::EnergyNextTurn, SlotWire::Int, stacked);
    if current == 0
        && !ctx
            .state
            .fanouts
            .register_after_energy_reset(crate::hot::AfterEnergyResetPower::EnergyNextTurn)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-energy-reset listener order",
        ));
    }
    ctx.state.normalize_after_energy_reset_order_if_unique();
    note_power(
        ctx.events,
        Subject::Player,
        PowerId::EnergyNextTurn,
        stacked,
    );
    Ok(())
}

/// Exact Feed / Hand of Greed attack-result rewards.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Feed/<OnPlay>d__9::MoveNext` RVA `0x39d834` and
/// `HandOfGreed/<OnPlay>d__10::MoveNext` RVA `0x3a35f4` snapshot
/// `PowerModel.ShouldOwnerDeathTriggerFatal` before awaiting one AttackCommand,
/// then inspect that command's `DamageResult.WasTargetKilled`. Feed awaits
/// `CreatureCmd/<GainMaxHp>d__22::MoveNext` RVA `0x3eb2f0`; Hand of Greed
/// awaits `PlayerCmd/<GainGold>d__9::MoveNext` RVA `0x3ee9f0`. Neither reward
/// command has an ending guard, so a final lethal, Stock respawn, or last
/// Segment still pays. Python `_should_monster_death_trigger_fatal` (frozen, deleted #2827),
/// `gain_player_max_hp`, `gain_player_gold`, and
/// `_run_steps_inner` carry the same ordering and conjunction.
pub(crate) fn fatal_attack_reward_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    #[derive(Clone, Copy)]
    enum Reward {
        MaxHp(i32),
        Gold(i32),
    }
    #[derive(Clone, Copy)]
    struct Plan {
        damage: i64,
        reward: Reward,
        target: usize,
        target_uid: u32,
        should_trigger_fatal: bool,
    }

    if ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "fatal_attack_reward_exact action",
        ));
    }
    let (damage, reward) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (
            CardId::Feed,
            0,
            [
                CompiledArg::I(10),
                CompiledArg::Word(StepWord::MaxHp),
                CompiledArg::I(3),
            ],
        ) => (10, Reward::MaxHp(3)),
        (
            CardId::Feed,
            1,
            [
                CompiledArg::I(12),
                CompiledArg::Word(StepWord::MaxHp),
                CompiledArg::I(4),
            ],
        ) => (12, Reward::MaxHp(4)),
        (
            CardId::HandOfGreed,
            0,
            [
                CompiledArg::I(20),
                CompiledArg::Word(StepWord::Gold),
                CompiledArg::I(20),
            ],
        ) => (20, Reward::Gold(20)),
        (
            CardId::HandOfGreed,
            1,
            [
                CompiledArg::I(25),
                CompiledArg::Word(StepWord::Gold),
                CompiledArg::I(25),
            ],
        ) => (25, Reward::Gold(25)),
        _ => return Err(EngineRefusal::MalformedArgs("fatal_attack_reward_exact")),
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs(
            "fatal_attack_reward_exact program",
        ));
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::FatalAttackRewardExact
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs(
            "fatal_attack_reward_exact exact row",
        ));
    }
    let (source_pile, source_index) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
            EngineRefusal::ActiveCardNotUnique {
                uid: ctx.source_uid,
                matches: 0,
            },
        )?;
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "fatal_attack_reward_exact physical source",
        ));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let monster = ctx
        .state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?;
    let should_trigger_fatal = if crate::engine::monsters::is_secondary_enemy(monster) {
        false
    } else if monster.kind == crate::ids::MonsterKind::DecimillipedeSegment {
        ctx.state.monsters.iter().enumerate().all(|(index, other)| {
            index == target
                || other.kind != crate::ids::MonsterKind::DecimillipedeSegment
                || other.hp <= 0
        })
    } else {
        true
    };
    if ctx.state.max_hp < 0 || ctx.state.max_hp > 999_999_999 || ctx.state.hp > ctx.state.max_hp {
        return Err(EngineRefusal::MalformedArgs(
            "fatal_attack_reward_exact player hp",
        ));
    }
    let plan = Plan {
        damage,
        reward,
        target,
        target_uid: monster.uid,
        should_trigger_fatal,
    };

    fn apply(ctx: &mut StepCtx<'_>, plan: Plan) -> Result<(), EngineRefusal> {
        let results = player_attack_results_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[plan.target],
            plan.damage,
            1,
            ctx.events,
        )?;
        if !plan.should_trigger_fatal
            || !results.iter().any(|result| {
                result.target == plan.target
                    && result.receiver_uid == plan.target_uid
                    && result.was_target_killed
            })
        {
            return Ok(());
        }
        match plan.reward {
            Reward::MaxHp(amount) => {
                // `GainMaxHp` (`0x3eb2f0`): `SetMaxHp` (IL_005b), then `Heal`
                // (IL_010b), whose `AfterCurrentHpChanged` reaches Red Skull
                // with both new values (#3044).
                crate::engine::damage::gain_player_max_hp(ctx.state, amount, ctx.events)?;
            }
            Reward::Gold(amount) => {
                crate::engine::relics::gain_fatal_reward_gold(
                    ctx.state,
                    ctx.catalog,
                    amount,
                    ctx.events,
                )?;
            }
        }
        Ok(())
    }

    // Attack listeners and death replacement can refuse before the reward,
    // while gold can overflow after a lethal. Rehearse the complete fused
    // body so direct execution is atomic; apply_action_into's cloned successor
    // extends the same rollback to energy, source routing, and CardPlayed.
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target: ctx.target,
        selection: ctx.selection,
        x_value: ctx.x_value,
        args: ctx.args,
        events: &mut probe_events,
    };
    apply(&mut probe_ctx, plan)?;
    apply(ctx, plan)
}

/// `("generate_fixed_shivs", count)` — create canonical L0 Shivs one by one.
///
/// Each generated entry records owner history before its nested Hand/Bottom
/// add; a full hand redirects that copy to Discard/Bottom. No RNG is consumed.
pub(crate) fn generate_fixed_shivs(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(count)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("generate_fixed_shivs"));
    };
    let count: usize = (*count)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("generate_fixed_shivs"))?;
    // Python's Fan source is an in-play safety witness while each serial Shiv
    // insertion checks sort-key ties. A second physical Fan with the same
    // `(id, upgrade)` differs by uid and therefore promotes exact pile order;
    // generated plain Shivs alone do not.
    if matches!(ctx.spec.identity.id, CardId::FanOfKnives)
        && !ctx.state.history.over
        && !ctx.state.exact_piles
        && PileId::ALL
            .into_iter()
            .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
            .filter(|card| {
                ctx.catalog.spec(card.atom).is_some_and(|spec| {
                    matches!(spec.identity.id, CardId::FanOfKnives)
                        && spec.identity.upgrade == ctx.spec.identity.upgrade
                })
            })
            .take(2)
            .count()
            == 2
    {
        ctx.state.exact_piles = true;
    }
    inject_generated_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        },
        count,
        PileId::Hand,
        ctx.events,
    )
}

/// One exact fixed generated Status, recorded before the ending gate.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `add_fixed_generated_status`: one allowlisted L0 Status minted to Hand/Discard Bottom, with
/// the CardGenerated record preceding the over-gated insert.
///
/// Unit C admits each generated leaf only through its exact registry row and
/// validates the complete source/card/destination triple here. The shared
/// transaction records CardGenerated and its listener epoch before testing
/// the nested pile add's ending gate, including Fight Through's two serial
/// Wound commands.
pub(crate) fn generate_fixed_status(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::List(identity), CompiledArg::Pile(destination)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("generate_fixed_status"));
    };
    let [CompiledArg::Card(id), CompiledArg::I(upgrade)] = ctx.catalog.args(*identity) else {
        return Err(EngineRefusal::MalformedArgs(
            "generate_fixed_status identity",
        ));
    };
    let upgrade: u8 = (*upgrade)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("generate_fixed_status upgrade"))?;
    let expected = match ctx.spec.identity.id {
        CardId::BoostAway => (CardId::Dazed, 0, PileId::Discard),
        CardId::CollisionCourse => (CardId::Debris, 0, PileId::Hand),
        CardId::Overclock => (CardId::Burn, 0, PileId::Discard),
        CardId::Turbo => (CardId::Void, 0, PileId::Discard),
        CardId::FightThrough => (CardId::Wound, 0, PileId::Discard),
        _ => {
            return Err(EngineRefusal::MalformedArgs("generate_fixed_status source"));
        }
    };
    if (*id, upgrade, *destination) != expected
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs(
            "generate_fixed_status source row",
        ));
    }
    inject_generated_record_before_ending_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: *id,
            upgrade,
            enchantment: None,
        },
        1,
        *destination,
        ctx.events,
    )
}

/// Stack the exact persistent HauntPower AfterCardPlayed listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). HAUNT/HAUNT+ add 7/9 to the
/// owner singleton after the generic ending gate; the reader lives in
/// `engine::play`.
///
/// v0.111.0 `Haunt/<OnPlay>` RVA `0x3a3c7c` awaits `PowerCmd.Apply<HauntPower>` at IL_00cc.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn haunt_power(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "haunt_power")?;
    if !matches!(
        (ctx.spec.identity.id, ctx.spec.identity.upgrade, amount),
        (CardId::Haunt, 0, 7) | (CardId::Haunt, 1, 9)
    ) {
        return Err(EngineRefusal::MalformedArgs("haunt_power"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let current = ctx.state.powers.value(PowerId::HauntPower);
    let updated = current
        .checked_add(amount as i32)
        .ok_or(EngineRefusal::CounterOverflow("haunt power"))?;
    crate::engine::play::prepare_after_card_played_scalar_write(
        ctx.state,
        PowerId::HauntPower,
        current,
        updated,
    )?;
    ctx.state
        .powers
        .set(PowerId::HauntPower, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::HauntPower, updated);
    Ok(())
}

/// `heal` — Not Yet's max-HP-clamped player heal.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — clamp the gain at max HP; plus the
/// loop-top refusal: a heal step reached after the fight-ending
/// kill refuses outright, because whether the command is over-gated is not
/// IL-verified and the delta would persist into the exit save.
///
/// Unit C admits exactly the two canonical Not Yet rows by requiring the
/// metadata `heal` field and body operand to agree. Python's loop-top guard
/// refuses a body reached after combat ended; retain that atomic boundary
/// rather than mutating the exit save.
///
/// `NotYet/<OnPlay>d__7` RVA `0x3af6d8` awaits `CreatureCmd.Heal` (IL_00ba), and
/// `<Heal>d__20` (`0x3eb4b0`) returns at `IsEnding` (IL_0046-004b). A combat
/// that is ending before the over latch heals nothing; `history.over` keeps its
/// refusal (#3515).
pub(crate) fn heal(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "heal")?;
    let expected = 10 + 3 * i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::NotYet
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || amount != expected
        || ctx.spec.row.heal != amount
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("heal owner"));
    }
    if ctx.state.history.over {
        return Err(EngineRefusal::CombatOver);
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("heal"))?;
    let healed = ctx
        .state
        .max_hp
        .checked_sub(ctx.state.hp)
        .ok_or(EngineRefusal::CounterOverflow("heal"))?
        .min(amount)
        .max(0);
    let hp_before = ctx.state.hp;
    ctx.state.hp = ctx
        .state
        .hp
        .checked_add(healed)
        .ok_or(EngineRefusal::CounterOverflow("heal"))?;
    // One `CreatureCmd.Heal` (`0x3eb4b0`): Red Skull's
    // `AfterCurrentHpChanged` (#3044).
    crate::engine::damage::red_skull_after_player_hp_changed(
        ctx.state,
        hp_before,
        ctx.state.max_hp,
        ctx.events,
    )?;
    Ok(())
}

/// `("hp_loss", amount)` — card self-damage: unblockable and unpowered.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) → `player_hp_loss`: the
/// commit, the lethal resolve, then Rupture, Beating
/// Remnant and Centennial Puzzle (refused relics), and the Inferno fan-out.
/// The persisted CardPlay owns active-source Rupture batching while the
/// damage primitive preserves the same commit, lethal resolve, and Inferno
/// suffix as the immediate-source Status path.
///
/// Which carrier owns the loss is decided by the frame, not by the lifecycle
/// alone (#3404). The body's native command does not depend on how
/// the card was played: `BloodWall/<OnPlay>d__9::MoveNext` RVA `0x38d618`
/// IL_0041-IL_006b awaits `CreatureCmd::Damage(choiceContext,
/// Owner.Creature, HpLoss.BaseValue, props 14, this, cardPlay)` whether the
/// play is manual or Distilled Chaos's `CardPileCmd::AutoPlayFromDrawPile`
/// (`DistilledChaos/<OnUse>d__8::MoveNext` RVA `0x34cec0` IL_004c-IL_006e;
/// v0.111.0 DLL 9cb4f1ad…). So a replay-lifecycle body with its own
/// persisted CardPlay on top takes [`crate::engine::play::active_card_hp_loss`],
/// and one without — the originless synchronous body a Stoke-free catalog
/// auto-plays under a `FrozenAutoBatch` — takes the same originless path it
/// takes outside the lifecycle, instead of refusing.
///
/// The `_run_steps_inner` loop-top guard (frozen Python, deleted #2827) refuses an `hp_loss` step
/// reached after the
/// fight-ending kill: whether the native command is over-gated is not
/// IL-verified, and the hp delta would persist into the exit save. Python
/// raises there; this body returns the same refusal before touching state.
pub(crate) fn hp_loss(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "hp_loss")?;
    if ctx.state.history.over {
        return Err(EngineRefusal::CombatOver);
    }
    if crate::engine::replay_lifecycle_is_active()
        && crate::engine::play::top_card_play_is_own(ctx.state, ctx.source_uid)
    {
        crate::engine::play::active_card_hp_loss(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
            amount,
            ctx.events,
        )
    } else {
        // Card self-damage awaits `CreatureCmd.Damage` from the card body, and
        // that command's `Hook.AfterDamageReceived` walk reaches Centennial
        // Puzzle's three one-card Draws
        // (`CentennialPuzzle/<AfterDamageReceived>d__10::MoveNext` RVA
        // `0x321758`, IL_007f-0116; v0.111.0 DLL 9cb4f1ad…). The step keeps its
        // catalog for them instead of refusing on the catalogless entry (#3172).
        // A registered Rupture batches this loss until AfterCardPlayed (#3298).
        crate::engine::play::card_body_hp_loss(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
            amount,
            ctx.events,
        )
    }
}

/// `lethality` — acquire the exact first-owned-play damage multiplier.
///
/// Current v0.111.0/41cef1ea IL: `Lethality/<OnPlay>d__5::MoveNext`
/// (RVA 0x3f2cac) applies owner LethalityPower Amount 50/75; the power's
/// `ModifyDamageMultiplicative` (RVA 0x250640) is read by the physical-card
/// attack pipeline. Python `_run_steps_inner` (frozen, deleted #2827) owns the same
/// over-gated Intensity stack.
///
/// v0.111.0 `Lethality/<OnPlay>` RVA `0x3a9a54` awaits `PowerCmd.Apply<LethalityPower>` at IL_00d1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn lethality(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Lethality, 0, [CompiledArg::I(50)]) => 50,
        (CardId::Lethality, 1, [CompiledArg::I(75)]) => 75,
        _ => return Err(EngineRefusal::MalformedArgs("lethality")),
    };
    if ctx.target.is_some() || ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("lethality"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let updated = ctx
        .state
        .powers
        .value(PowerId::Lethality)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("lethality"))?;
    ctx.state
        .powers
        .set(PowerId::Lethality, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Lethality, updated);
    Ok(())
}

/// Apply one supported monster power serially to a frozen living roster.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Haze / Scare / Deathbringer /
/// Resonance / Piercing Wail: one materialized `HittableEnemies` snapshot,
/// then one awaited debuff application per member, each re-entering the
/// ending/dead-recipient gate.
///
/// Slice 5 completed every current argument's reader: Weak, Poison, Doom,
/// permanent signed Strength, and the enemy temporary-Strength wrapper. The
/// source allowlist below pins every generated carrier and exact amount.
///
/// Every member is one `PowerCmd.Apply` (Resonance `0x3b71cc` IL_0185, Piercing
/// Wail `0x3b253c` IL_00f3), and `<Apply>d__1`1` (`0x3ef988`) returns at
/// `IsEnding` (IL_0025-002a). The monster debuff writers carry that gate
/// themselves; the permanent Strength writer `apply_card_monster_strength_delta`
/// still reads `history.over`, so this loop tests the shared IsEnding
/// projection (#3515).
pub(crate) fn power_all_serial(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::Power(power), CompiledArg::I(raw)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("power_all_serial"));
    };
    let exact = matches!(
        (
            ctx.spec.identity.id,
            ctx.spec.identity.upgrade,
            *power,
            *raw
        ),
        (CardId::Deathbringer, 0, PowerId::Doom, 21)
            | (CardId::Deathbringer, 0, PowerId::Weak, 1)
            | (CardId::Deathbringer, 1, PowerId::Doom, 26)
            | (CardId::Deathbringer, 1, PowerId::Weak, 1)
            | (CardId::Haze, 0, PowerId::Poison, 4)
            | (CardId::Haze, 0, PowerId::Weak, 1)
            | (CardId::Haze, 1, PowerId::Poison, 6)
            | (CardId::Haze, 1, PowerId::Weak, 2)
            | (CardId::NegativePulse, 0, PowerId::Doom, 7)
            | (CardId::NegativePulse, 1, PowerId::Doom, 11)
            | (CardId::PiercingWail, 0, PowerId::TempStrengthEnemy, 6)
            | (CardId::PiercingWail, 1, PowerId::TempStrengthEnemy, 8)
            | (CardId::Resonance, 0 | 1, PowerId::StrengthEnemy, -1)
    );
    if !exact {
        return Err(EngineRefusal::MalformedArgs("power_all_serial"));
    }
    let amount: i32 = (*raw)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("power_all_serial"))?;
    let targets = alive_targets(ctx.state);
    for target in targets {
        if crate::engine::damage::damage_combat_is_ending(ctx.state)
            || ctx.state.monsters[target].hp <= 0
        {
            continue;
        }
        match power {
            PowerId::Weak => apply_card_monster_debuff(
                ctx.state,
                target,
                PowerId::Weak,
                MiseryToken::Weak,
                amount,
                ctx.events,
            )?,
            PowerId::Poison => apply_card_monster_debuff(
                ctx.state,
                target,
                PowerId::Poison,
                MiseryToken::Poison,
                amount,
                ctx.events,
            )?,
            PowerId::Doom => apply_card_monster_debuff(
                ctx.state,
                target,
                PowerId::Doom,
                MiseryToken::Doom,
                amount,
                ctx.events,
            )?,
            PowerId::StrengthEnemy => {
                apply_card_monster_strength_delta(ctx.state, target, amount, ctx.events)?
            }
            PowerId::TempStrengthEnemy => apply_temp_strength_enemy(
                ctx.state,
                target,
                temp_strength_enemy_model(ctx.spec.identity.id)?,
                amount,
                ctx.events,
            )?,
            _ => return Err(EngineRefusal::MalformedArgs("power_all_serial")),
        }
    }
    Ok(())
}

/// Generic generated `select` — exact for the admitted vocabulary (#1363).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the generic single-pile selection:
/// `_select_candidates`, the whole-pile auto-take when the filtered pile is
/// at or under MinSelect, and otherwise a **suspension** returning the
/// pending step for a later player pick.
///
/// The interpreter owns candidate filtering, exact option order, serial
/// consumers and the no-choice path. The card-play loop owns suspension and
/// the exact remaining-step cursor. Upgrade and custom consumer programs stay
/// row-derived admission refusals rather than being approximated here.
pub(crate) fn select(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    match crate::engine::selection::execute(ctx)? {
        crate::engine::selection::SelectDisposition::Complete => Ok(()),
        // The owning step loop intercepts this control token only for Select.
        // Every other caller sees the ordinary fail-closed continuation error.
        crate::engine::selection::SelectDisposition::Suspend => {
            Err(EngineRefusal::ContinuationNotModeled)
        }
    }
}

/// Stack the exact SerpentFormPower singleton.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The old amount is frozen by
/// `engine::play` before this body, so the applying play cannot hit with its
/// newly added amount.
///
/// v0.111.0 `SerpentForm/<OnPlay>` RVA `0x3b9d78` awaits `PowerCmd.Apply<SerpentFormPower>` at IL_00d1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn serpent_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "serpent_form")?;
    if !matches!(
        (ctx.spec.identity.id, ctx.spec.identity.upgrade, amount),
        (CardId::SerpentForm, 0, 4) | (CardId::SerpentForm, 1, 6)
    ) {
        return Err(EngineRefusal::MalformedArgs("serpent_form"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let current = ctx.state.powers.value(PowerId::SerpentForm);
    let updated = current
        .checked_add(amount as i32)
        .ok_or(EngineRefusal::CounterOverflow("serpent form"))?;
    crate::engine::play::prepare_after_card_played_scalar_write(
        ctx.state,
        PowerId::SerpentForm,
        current,
        updated,
    )?;
    ctx.state
        .powers
        .set(PowerId::SerpentForm, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::SerpentForm, updated);
    Ok(())
}

/// Add the exact owner Shadowmeld multiplier for this side.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — an over-gated stack refusing at
/// Amount 96, the first power of two outside the native double-to-Decimal
/// conversion.
///
///
/// v0.111.0 `Shadowmeld/<OnPlay>` RVA `0x3ba50c` awaits `PowerCmd.Apply<ShadowmeldPower>` at IL_00d1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn shadowmeld(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "shadowmeld")?;
    if ctx.spec.identity.id != CardId::Shadowmeld
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || amount != 1
    {
        return Err(EngineRefusal::MalformedArgs("shadowmeld"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let current = ctx.state.powers.value(PowerId::Shadowmeld);
    let updated = current
        .checked_add(1)
        .filter(|amount| *amount < 96)
        .ok_or(EngineRefusal::CounterOverflow("shadowmeld"))?;
    crate::engine::turn::prepare_after_side_turn_end_scalar_write(
        ctx.state,
        PowerId::Shadowmeld,
        current,
        updated,
    )?;
    ctx.state
        .powers
        .set(PowerId::Shadowmeld, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Shadowmeld, updated);
    Ok(())
}

/// `("shroud", n)` — ShroudPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Only the two native SHROUD rows
/// can enter this transaction. The first positive application appends the
/// listener to the represented acquisition-order walk, then every application
/// adds its Intensity and publishes the resulting amount. Combat ending makes
/// the complete body a no-op. The Doom-only reader lands atomically in
/// `engine::damage` and reads this live amount when its hook fires.
///
/// v0.111.0 `Shroud/<OnPlay>` RVA `0x3bb7b8` awaits `PowerCmd.Apply<ShroudPower>` at IL_00cc.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn shroud(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Shroud, 0, [CompiledArg::I(3)]) => 3,
        (CardId::Shroud, 1, [CompiledArg::I(4)]) => 4,
        _ => return Err(EngineRefusal::MalformedArgs("shroud")),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }

    let current = ctx.state.powers.value(PowerId::Shroud);
    let ordered = ctx
        .state
        .fanouts
        .after_power_amount_changed_order()
        .contains(&PowerId::Shroud);
    if current < 0 || (current == 0) == ordered {
        return Err(EngineRefusal::MalformedArgs("shroud live state"));
    }
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("shroud"))?;
    if current == 0
        && !ctx
            .state
            .fanouts
            .register_after_power_amount_changed(PowerId::Shroud)
    {
        return Err(EngineRefusal::CounterOverflow("fanout listener order"));
    }
    ctx.state
        .powers
        .set(PowerId::Shroud, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Shroud, updated);
    Ok(())
}

/// Exact fused Soul-generating card bodies.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// Grave Warden `OnPlay` RVA `0x3a27d8` gains powered Block 8/11, then creates
/// one L0 Soul at Draw/Random; Reave `0x3b5f10` attacks for 10/13, then creates
/// one Soul at Draw/Random (L1 upgrades only that Soul); Severance `0x3ba1dc`
/// attacks for 13/18, then serially creates L0 Souls at Draw/Random,
/// Discard/Bottom, and Hand/Bottom; Capture Spirit `0x391348` deals 3/4 with
/// DamageVar props 14 (unpowered, unblockable), then serially creates 3/4 L0
/// Souls at Draw/Random. Python `_run_steps_inner` (frozen, deleted #2827) and
/// `add_generated_souls` carry the same exact eight-row registry and order.
pub(crate) fn soul_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    #[derive(Clone, Copy)]
    enum Body {
        Block(i64),
        Attack(i64),
        UnpoweredUnblockable(i64),
    }
    #[derive(Clone, Copy)]
    enum Destination {
        DrawRandom,
        Bottom(PileId),
    }
    #[derive(Clone, Copy)]
    struct Plan<'a> {
        body: Body,
        target: Option<usize>,
        identity: CardIdentity,
        destinations: &'a [Destination],
    }

    let (body, soul_upgrade, destinations): (Body, u8, &[Destination]) =
        match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
            (CardId::GraveWarden, 0, [CompiledArg::Word(StepWord::Block), CompiledArg::I(8)]) => {
                (Body::Block(8), 0, &[Destination::DrawRandom])
            }
            (CardId::GraveWarden, 1, [CompiledArg::Word(StepWord::Block), CompiledArg::I(11)]) => {
                (Body::Block(11), 0, &[Destination::DrawRandom])
            }
            (CardId::Reave, 0, [CompiledArg::Word(StepWord::Attack), CompiledArg::I(10)]) => {
                (Body::Attack(10), 0, &[Destination::DrawRandom])
            }
            (CardId::Reave, 1, [CompiledArg::Word(StepWord::Attack), CompiledArg::I(13)]) => {
                (Body::Attack(13), 1, &[Destination::DrawRandom])
            }
            (CardId::Severance, 0, [CompiledArg::Word(StepWord::Attack), CompiledArg::I(13)]) => (
                Body::Attack(13),
                0,
                &[
                    Destination::DrawRandom,
                    Destination::Bottom(PileId::Discard),
                    Destination::Bottom(PileId::Hand),
                ],
            ),
            (CardId::Severance, 1, [CompiledArg::Word(StepWord::Attack), CompiledArg::I(18)]) => (
                Body::Attack(18),
                0,
                &[
                    Destination::DrawRandom,
                    Destination::Bottom(PileId::Discard),
                    Destination::Bottom(PileId::Hand),
                ],
            ),
            (
                CardId::CaptureSpirit,
                0,
                [
                    CompiledArg::Word(StepWord::UnpoweredUnblockableDamage),
                    CompiledArg::I(3),
                ],
            ) => (
                Body::UnpoweredUnblockable(3),
                0,
                &[
                    Destination::DrawRandom,
                    Destination::DrawRandom,
                    Destination::DrawRandom,
                ],
            ),
            (
                CardId::CaptureSpirit,
                1,
                [
                    CompiledArg::Word(StepWord::UnpoweredUnblockableDamage),
                    CompiledArg::I(4),
                ],
            ) => (
                Body::UnpoweredUnblockable(4),
                0,
                &[
                    Destination::DrawRandom,
                    Destination::DrawRandom,
                    Destination::DrawRandom,
                    Destination::DrawRandom,
                ],
            ),
            _ => return Err(EngineRefusal::MalformedArgs("soul_body")),
        };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("soul_body program"));
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::SoulBody
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs("soul_body exact row"));
    }
    let target = match body {
        Body::Block(_) => None,
        Body::Attack(_) | Body::UnpoweredUnblockable(_) => Some(
            ctx.target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?,
        ),
    };
    let identity = CardIdentity {
        id: CardId::Soul,
        upgrade: soul_upgrade,
        enchantment: None,
    };
    let plan = Plan {
        body,
        target,
        identity,
        destinations,
    };

    fn apply(ctx: &mut StepCtx<'_>, plan: Plan<'_>) -> Result<(), EngineRefusal> {
        match plan.body {
            Body::Block(amount) => {
                gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, amount, ctx.events)?;
                fire_hook(
                    ctx.catalog,
                    HookEvent::AfterBlockGained,
                    ctx.state,
                    ctx.events,
                )?;
            }
            Body::Attack(amount) => player_attack_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                &[plan.target.expect("attack body authenticated a target")],
                amount,
                1,
                ctx.events,
            )?,
            Body::UnpoweredUnblockable(amount) => {
                // The death of this Damage runs `Hook.AfterDeath`, so it keeps
                // the step's catalog for Gremlin Horn's Draw (#3172).
                crate::engine::damage::damage_monster_after_catalog_auth(
                    ctx.state,
                    ctx.catalog,
                    plan.target.expect("damage body authenticated a target"),
                    crate::decimal::DotNetDecimal::from_i64(amount),
                    false,
                    false,
                    ctx.events,
                )?;
            }
        }
        for destination in plan.destinations {
            match destination {
                Destination::DrawRandom => inject_generated_record_before_ending_draw_random(
                    ctx.state,
                    ctx.catalog,
                    plan.identity,
                    ctx.events,
                )?,
                Destination::Bottom(pile) => inject_generated_record_before_ending_bottom(
                    ctx.state,
                    ctx.catalog,
                    plan.identity,
                    1,
                    *pile,
                    ctx.events,
                )?,
            }
        }
        Ok(())
    }

    // Generation listeners and later serial insertions can refuse after the
    // attack/block prefix. Rehearse the complete body so the public play's
    // outer transaction can roll back energy, source routing, and CardPlayed.
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target: ctx.target,
        selection: ctx.selection,
        x_value: ctx.x_value,
        args: ctx.args,
        events: &mut probe_events,
    };
    apply(&mut probe_ctx, plan)?;
    apply(ctx, plan)
}

/// Apply Spectrum Shift's additive owner power and acquisition-order token.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `SpectrumShift/<OnPlay>d__3::MoveNext` (RVA `0x3bd9ac`) reads canonical
/// Cards `1` and applies `SpectrumShiftPower` to the source owner at IL
/// `0x009e-0x0123`. The ctor/vars/upgrade RVAs `0xec297`/`0xec2a4`/`0xec2ff`
/// pin the two exact rows: cost 2 at L0, cost 1 at L1, and no amount change.
/// Python `_run_steps_inner` validates the same exact solo-Regent generation
/// provenance (frozen Python, deleted #2827) before registering the first positive stack in
/// `before_hand_draw_power_order` and adding the Intensity amount.
///
/// v0.111.0 `SpectrumShift/<OnPlay>` RVA `0x3bd9ac` awaits `PowerCmd.Apply<SpectrumShiftPower>` at IL_00cc.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn spectrum_shift(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(1)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("spectrum_shift"));
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("spectrum_shift program"));
    };
    if ctx.spec.identity.id != CardId::SpectrumShift
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::SpectrumShift
        || ctx.catalog.args(program.args) != [CompiledArg::I(1)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !crate::engine::cards::colorless_generation_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("spectrum_shift"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let old = ctx.state.powers.value(PowerId::SpectrumShift);
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("spectrum_shift"))?;
    if old == 0
        && !ctx
            .state
            .fanouts
            .register_before_hand_draw(PowerId::SpectrumShift)
    {
        return Err(EngineRefusal::CounterOverflow("fanout listener order"));
    }
    ctx.state
        .powers
        .set(PowerId::SpectrumShift, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::SpectrumShift, updated);
    Ok(())
}

/// Spinner's additive amount and acquisition-order registration.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `orb_energy_reset_order` on its zero-to-positive edge and stacks.
///
pub(crate) fn spinner(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "spinner")?;
    if ctx.spec.identity.id != CardId::Spinner
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || amount != 1
    {
        return Err(EngineRefusal::MalformedArgs("spinner"));
    }
    // `PowerCmd.Apply<SpinnerPower>` returns on `IsEnding` (`<Apply>d__1`1`
    // RVA `0x3ef988` IL_0020-0034). Spinner+ applies it after
    // `Channel<GlassOrb>` (`Spinner/<OnPlay>d__5` RVA `0x3bdc28` IL_00b9 then
    // IL_0146), whose full-slot evoke can end the combat (#3495).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let was_absent = ctx.state.powers.value(PowerId::Spinner) == 0;
    let updated = ctx
        .state
        .powers
        .value(PowerId::Spinner)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("spinner"))?;
    ctx.state
        .powers
        .set(PowerId::Spinner, SlotWire::Int, updated);
    if was_absent {
        // Older admitted roots may carry a lone Lightning Rod through the
        // documented effective-order fallback without a raw orb list. Before
        // the first new peer attaches, materialize that established subgroup
        // fact so raw and unified projections stay reload-coherent.
        if ctx.state.orbs.reset_order().is_empty()
            && ctx.state.powers.value(PowerId::LightningRod) > 0
        {
            ctx.state
                .orbs
                .set_reset_order(vec![OrbResetPower::LightningRod]);
        }
        ctx.state.orbs.register_reset_power(OrbResetPower::Spinner);
        if !ctx
            .state
            .fanouts
            .register_after_energy_reset(crate::hot::AfterEnergyResetPower::Spinner)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-energy-reset listener order",
            ));
        }
    }
    ctx.state.normalize_after_energy_reset_order_if_unique();
    note_power(ctx.events, Subject::Player, PowerId::Spinner, updated);
    Ok(())
}

/// `("stars", amount)` — one ending-gated `PlayerCmd::GainStars`.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) delegates to `gain_stars`. The
/// shared helper owns balance and history ordering. Python's general step-loop
/// I5 guard also refuses an `over` state before a non-final suffix. The current
/// admitted non-final surface is mechanically pinned to `GLOW` with Stars at
/// index zero, entered only by a live play; `HIDDEN_CACHE` and BlackHolePower
/// remain admission refusals. Thus no admitted suffix can observe `over`
/// without approximating the broader guard.
pub(crate) fn stars(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "stars")?;
    crate::engine::play::gain_stars(ctx.state, ctx.catalog, amount, ctx.events)
}

/// Exact additive owner-Strength command sources.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_apply_owner_strength(s, step[1],
/// ...)`, the owner-targeted StrengthPower command. Current v0.111.0
/// `FightMe/<OnPlay>d__6::MoveNext` **0x39e360** applies its 3/4 owner
/// Strength after the two-hit attack and before the enemy's `+1` Strength.
///
///
/// Every source awaits one `PowerCmd.Apply<StrengthPower>` on its owner:
/// Inflame `0x3a76c8` IL_0061, Bulk Up `0x38fa40` IL_00f4, Brand `0x38edbc`
/// IL_02a3, Prowess `0x3b4064` IL_0052, Resonance `0x3b71cc` IL_00e0, Fight Me
/// `0x39e360` IL_012b. `<Apply>d__1`1` (`0x3ef988`) returns at `IsEnding`
/// (IL_0025-002a); `apply_owner_strength` still reads `history.over`, so the
/// body tests the shared IsEnding projection first (#3515).
pub(crate) fn strength(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "strength")?;
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Brand | CardId::Prowess | CardId::Resonance, 0) => 1,
        (CardId::Brand | CardId::Prowess | CardId::Resonance, 1) => 2,
        (CardId::BulkUp | CardId::Inflame, 0) => 2,
        (CardId::BulkUp | CardId::Inflame, 1) => 3,
        (CardId::FightMe, 0) => 3,
        (CardId::FightMe, 1) => 4,
        (CardId::MadScience, _) => mad_science_expertise_amount(ctx, StepKind::Strength)
            .ok_or(EngineRefusal::MalformedArgs("strength"))?,
        _ => return Err(EngineRefusal::MalformedArgs("strength")),
    };
    if amount != expected {
        return Err(EngineRefusal::MalformedArgs("strength"));
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("strength"))?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    apply_owner_strength(ctx.state, amount, ctx.events)
}

/// `summon` — exact live Osty summon/grow primitive (#1561 R30).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `summon_ally(s, step[1], step[2],
/// log)`, the SUM1 primitive with the alive re-summon rule.
///
/// The complete current carrier census is Afterlife 6/9, Bodyguard 5/7,
/// Cleanse 3/5, Necro Mastery 5/8, Pull Aggro 4/5, and Reanimate 20/25.
/// Every row names the sole current pet kind `OSTY`. A missing pet is created
/// at amount HP/max HP; a live pet grows both values by amount, with no RNG.
pub(crate) fn summon(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let upgrade = i32::from(ctx.spec.identity.upgrade);
    if upgrade > 1 {
        return Err(EngineRefusal::MalformedArgs("summon"));
    }
    let expected = match ctx.spec.identity.id {
        CardId::Afterlife => 6 + 3 * upgrade,
        CardId::Bodyguard => 5 + 2 * upgrade,
        CardId::Cleanse => 3 + 2 * upgrade,
        CardId::NecroMastery => 5 + 3 * upgrade,
        CardId::PullAggro => 4 + upgrade,
        CardId::Reanimate => 20 + 5 * upgrade,
        _ => return Err(EngineRefusal::MalformedArgs("summon")),
    };
    let amount = match ctx.args {
        [CompiledArg::Word(StepWord::Osty), CompiledArg::I(amount)]
            if *amount == i64::from(expected) =>
        {
            expected
        }
        _ => return Err(EngineRefusal::MalformedArgs("summon")),
    };
    // No `history.over` gate (#3279). Every carrier's `OnPlay` awaits
    // `OstyCmd.Summon` as its first command with no combat read before it:
    // `Afterlife/<OnPlay>d__7` `0x3894d8` IL_00c6, `Bodyguard/<OnPlay>d__5`
    // `0x38db20` IL_00c6, `Cleanse/<OnPlay>d__5` `0x39287c` IL_00d5,
    // `NecroMastery/<OnPlay>d__5` `0x3ae420` IL_00c3, `PullAggro/<OnPlay>d__7`
    // `0x3b41f4` IL_00ce, `Reanimate/<OnPlay>d__7` `0x3b5b00` IL_00c6.
    // `CardModel/<OnPlayWrapper>d__339` (`0x31b8d0`) tests `IsOverOrEnding`
    // once per play index (IL_0428-IL_0432), *before* `Hook.BeforeCardPlayed`
    // (IL_05a1) and `OnPlay` (IL_0659), and nothing in between re-reads it.
    // Rust skips a non-manual play already at `history.over`
    // (`play_card_with_work_inner`) and every later replay
    // (`play_index > 0`), so the step sees `history.over` only when this
    // play's own pre-OnPlay listeners ended the combat. Native then still
    // summons, under IsEnding: `summon_osty`'s projection.
    // Necro Mastery's generated body publishes this summon before its
    // additive power step. Preflight that only fallible suffix here so a
    // direct card action cannot leak the pet mutation when the later stack
    // overflows.
    if matches!(ctx.spec.identity.id, CardId::NecroMastery)
        && ctx
            .state
            .powers
            .value(PowerId::NecroMastery)
            .checked_add(1)
            .is_none()
    {
        return Err(EngineRefusal::CounterOverflow("necro_mastery"));
    }
    crate::engine::summon_osty(ctx.state, amount, "Osty summon")
}

/// `temp_strength` — positive owner TemporaryStrengthPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_apply_owner_strength(...,
/// temporary=True)`: the SetupStrikePower wrapper keeps the amount in the
/// separate `temp_strength` scalar and removes it at the owner's side end.
///
/// Python: `_apply_owner_strength` (frozen, deleted #2827) keeps the original wrapper
/// amount separate from permanent Strength; `player_attack` reads both
/// in one additive fold and `_dispatch_after_side_turn_end_power` clears the wrapper. The complete current carrier census is
/// Feeding Frenzy 5/7 and Setup Strike 3/4. Ruined Helmet doubles its first
/// positive application while leaving only the original wrapper amount in
/// the expiry ledger.
pub(crate) fn temp_strength(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::FeedingFrenzy, 0, [CompiledArg::I(5)]) => 5,
        (CardId::FeedingFrenzy, 1, [CompiledArg::I(7)]) => 7,
        (CardId::SetupStrike, 0, [CompiledArg::I(3)]) => 3,
        (CardId::SetupStrike, 1, [CompiledArg::I(4)]) => 4,
        _ => return Err(EngineRefusal::MalformedArgs("temp_strength")),
    };
    crate::engine::damage::apply_owner_temporary_strength(ctx.state, amount, ctx.events)
}

/// Python `_run_steps_inner` (frozen, deleted #2827): authenticate the generated row,
/// append the first listener token once, then stack additive Intensity.
pub(crate) fn tools_of_the_trade(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_turn_start_hand_choice_power(ctx, CardId::ToolsOfTheTrade, PowerId::ToolsOfTheTrade)
}

/// Python `_run_steps_inner` (frozen, deleted #2827): Tyranny shares the same exact
/// first-application registration and additive stacking branch.
pub(crate) fn tyranny(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_turn_start_hand_choice_power(ctx, CardId::Tyranny, PowerId::Tyranny)
}

fn apply_turn_start_hand_choice_power(
    ctx: &mut StepCtx<'_>,
    card_id: CardId,
    power: PowerId,
) -> Result<(), EngineRefusal> {
    let exact_program = matches!(ctx.catalog.steps(ctx.spec), [step]
        if step.kind == if power == PowerId::ToolsOfTheTrade {
            StepKind::ToolsOfTheTrade
        } else {
            StepKind::Tyranny
        } && ctx.catalog.args(step.args) == [CompiledArg::I(1)]);
    let source_matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .filter(|card| card.uid == ctx.source_uid)
        .copied()
        .collect::<Vec<_>>();
    if !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || ctx.spec.identity.id != card_id
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.args != [CompiledArg::I(1)]
        || crate::content_tables::card_row(card_id, ctx.spec.identity.upgrade) != Some(ctx.spec.row)
        || !exact_program
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || source_matches.len() != 1
        || ctx.catalog.spec(source_matches[0].atom) != Some(ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Tools/Tyranny exact generated row",
        ));
    }
    let old = ctx.state.powers.value(power);
    if old < 0 {
        return Err(EngineRefusal::MalformedArgs("Tools/Tyranny live amount"));
    }
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Tools/Tyranny amount"))?;
    if old <= 0 && !ctx.state.register_after_player_turn_start(power) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Tools/Tyranny AfterPlayerTurnStart carrier family",
        ));
    }
    ctx.state.powers.set(power, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, power, updated);
    Ok(())
}

/// `("vicious", n)` — ViciousPower's additive Vulnerable-change listener.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Vicious/<OnPlay>d__5::MoveNext` RVA `0x3c6714` applies the canonical
/// Cards value 1/2. `ViciousPower` Type and StackType RVAs `0xaa707` /
/// `0xaa70a` are both 1, so reapplication adds to one live listener without
/// moving its acquisition-order token. Its callback body RVA `0x34a6f8`
/// requires a positive effective delta, the power owner as applier, and
/// `VulnerablePower`, then awaits ordinary `Draw(live Amount, false)`.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) owns the same zero-to-positive
/// registration and additive amount. The catalog-aware Vulnerable command in
/// `engine::damage` owns the callback's exact Draw and whole-command preflight.
///
/// v0.111.0 `Vicious/<OnPlay>` RVA `0x3c6714` awaits `PowerCmd.Apply<ViciousPower>` at IL_00cc.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn vicious(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Vicious, 0, [CompiledArg::I(1)]) => 1,
        (CardId::Vicious, 1, [CompiledArg::I(2)]) => 2,
        _ => return Err(EngineRefusal::MalformedArgs("vicious")),
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("vicious program"));
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::Vicious
        || ctx.catalog.args(program.args) != ctx.args
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("vicious"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    if !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "after-power-amount-changed listener order",
        ));
    }
    let current = ctx.state.powers.value(PowerId::Vicious);
    if current < 0 {
        return Err(EngineRefusal::MalformedArgs("vicious live state"));
    }
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("vicious"))?;
    if current == 0
        && !ctx
            .state
            .fanouts
            .register_after_power_amount_changed(PowerId::Vicious)
    {
        return Err(EngineRefusal::CounterOverflow("fanout listener order"));
    }
    ctx.state
        .powers
        .set(PowerId::Vicious, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Vicious, updated);
    Ok(())
}

/// `("vulnerable", amount)` — Vulnerable on the chosen target.
///
/// Python: `_run_steps_inner`'s vulnerable branch. `_lamp_amount` (Unsettling
/// Lamp) scales the amount; the relic is refused, so the card's own amount
/// stands. A dead target is skipped, not an error — the step's own liveness
/// gate, as in Python.
pub(crate) fn vulnerable(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (raw, _) =
        step_args(StepKind::Vulnerable, ctx.args).map_err(EngineRefusal::MalformedArgs)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target) {
        return Ok(());
    }
    let amount: i32 = raw
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("vulnerable amount"))?;
    if ctx.state.monsters[target].hp <= 0 {
        return Ok(());
    }
    apply_card_monster_debuff_with_catalog(
        ctx.state,
        ctx.catalog,
        target,
        PowerId::Vuln,
        MiseryToken::Vuln,
        amount,
        ctx.events,
    )
}

/// `("weak", amount)` — Weak on the chosen target.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The step's own liveness gate skips a
/// dead target rather than erroring, then the card-sourced application runs
/// through the Artifact gate and the Unsettling Lamp walk — both refused
/// content, so the printed amount lands. [`apply_card_monster_debuff`] is
/// that application, acquisition-order record and power event included; the
/// multiplier the stacks buy is read in the monster attack snapshot's Weak
/// walk.
pub(crate) fn weak(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "weak")?;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("weak amount"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target) {
        return Ok(());
    }
    if ctx.state.monsters.get(target).is_none_or(|m| m.hp <= 0) {
        return Ok(());
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Weak,
        MiseryToken::Weak,
        amount,
        ctx.events,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Event, capability_manifest};
    use crate::hot::{HotCard, HotMonster, HotState, RngStream};
    use crate::ids::MonsterKind;

    /// The R0.5 slice fixture, loaded through the ordinary boundary.
    fn fixture() -> (HotState, Catalog) {
        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .expect("the checked-in fixture parses");
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (state, catalog)
    }

    /// Every kind this file owns a body for, ported or not.
    ///
    /// The dispatch's own completeness and uniqueness are pinned by
    /// `tests/hot_path_contract.rs`; what this list adds is the split between
    /// claimed and escalated, so that porting a kind means moving it into
    /// [`IMPLEMENTED`] in one deliberate edit.
    const OWNED: [StepKind; 43] = [
        StepKind::AddOrbSlotsExact,
        StepKind::Attack,
        StepKind::AttackAll,
        StepKind::AttackAllTempStrengthSnapshot,
        StepKind::AttackContextResultExact,
        StepKind::AttackRandom,
        StepKind::AttackRandomX,
        StepKind::AttackResult,
        StepKind::AttackX,
        StepKind::Block,
        StepKind::Calcify,
        StepKind::CalculatedAttack,
        StepKind::CalculatedBlock,
        StepKind::CalculatedHits,
        StepKind::Channel,
        StepKind::Dexterity,
        StepKind::Draw,
        StepKind::Energy,
        StepKind::EnergyNextTurn,
        StepKind::FatalAttackRewardExact,
        StepKind::GenerateFixedShivs,
        StepKind::GenerateFixedStatus,
        StepKind::HauntPower,
        StepKind::Heal,
        StepKind::HpLoss,
        StepKind::Lethality,
        StepKind::PowerAllSerial,
        StepKind::Select,
        StepKind::SerpentForm,
        StepKind::Shadowmeld,
        StepKind::Shroud,
        StepKind::SoulBody,
        StepKind::SpectrumShift,
        StepKind::Spinner,
        StepKind::Stars,
        StepKind::Strength,
        StepKind::Summon,
        StepKind::TempStrength,
        StepKind::ToolsOfTheTrade,
        StepKind::Tyranny,
        StepKind::Vicious,
        StepKind::Vulnerable,
        StepKind::Weak,
    ];

    /// The kinds still waiting on an engine primitive or an admissible
    /// source, per each stub's own note.
    const ESCALATED: [StepKind; 0] = [];

    /// The wave's honesty property: the manifest claims nothing this file
    /// cannot actually play, and claims everything it can.
    #[test]
    fn the_manifest_claims_nothing_this_family_has_not_ported() {
        let manifest = capability_manifest();
        for kind in ESCALATED {
            assert!(
                !manifest.steps.contains(&kind),
                "{:?} is escalated but the manifest claims it",
                kind.as_str()
            );
            assert!(!IMPLEMENTED.contains(&kind));
        }
        for kind in IMPLEMENTED {
            assert!(
                manifest.steps.contains(kind),
                "{:?} has a body here but the manifest does not claim it",
                kind.as_str()
            );
            assert!(
                OWNED.contains(kind),
                "{:?} is not this file's",
                kind.as_str()
            );
        }
        assert_eq!(IMPLEMENTED.len() + ESCALATED.len(), OWNED.len());
    }

    #[test]
    fn tools_and_tyranny_exact_rows_stack_register_once_and_wait_for_next_turn() {
        for (card_id, power, kind) in [
            (
                CardId::ToolsOfTheTrade,
                PowerId::ToolsOfTheTrade,
                StepKind::ToolsOfTheTrade,
            ),
            (CardId::Tyranny, PowerId::Tyranny, StepKind::Tyranny),
        ] {
            for upgrade in 0..=1 {
                let identity = CardIdentity {
                    id: card_id,
                    upgrade,
                    enchantment: None,
                };
                let mut builder = CatalogBuilder::new();
                let atom = builder.intern(identity).unwrap();
                let catalog = builder.build();
                let spec = *catalog.spec(atom).unwrap();
                let mut state = HotState::at_defaults();
                state.next_card_uid = 8;
                state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                    uid: 7,
                    atom,
                    flags: 0,
                });
                let mut events = Vec::new();
                let args = [CompiledArg::I(1)];
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut events,
                };

                let apply = if kind == StepKind::ToolsOfTheTrade {
                    tools_of_the_trade
                } else {
                    tyranny
                };
                apply(&mut ctx).unwrap();
                apply(&mut ctx).unwrap();
                assert_eq!(ctx.state.powers.value(power), 2);
                assert_eq!(
                    ctx.state
                        .fanouts
                        .legacy_turn_start_hand_choice_order_without_entropy()
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                    vec![power]
                );
                assert!(ctx.state.pending.is_none());
                assert!(ctx.state.frames.is_empty());
                assert_eq!(
                    ctx.events,
                    &[
                        Event::PowerChanged {
                            subject: Subject::Player,
                            power,
                            amount: 1,
                        },
                        Event::PowerChanged {
                            subject: Subject::Player,
                            power,
                            amount: 2,
                        },
                    ]
                );
            }
        }
    }

    #[test]
    fn tools_and_tyranny_writer_refuses_carrier_and_amount_failures_atomically() {
        let identity = CardIdentity {
            id: CardId::ToolsOfTheTrade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = [CompiledArg::I(1)];

        for (amount, expected) in [
            (
                -1,
                EngineRefusal::MalformedArgs("Tools/Tyranny live amount"),
            ),
            (
                i32::MAX,
                EngineRefusal::CounterOverflow("Tools/Tyranny amount"),
            ),
        ] {
            let mut state = HotState::at_defaults();
            state.next_card_uid = 8;
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            state
                .powers
                .set(PowerId::ToolsOfTheTrade, SlotWire::Int, amount);
            if amount > 0 {
                assert!(
                    state
                        .fanouts
                        .set_turn_start_hand_choice_order(&[PowerId::ToolsOfTheTrade])
                );
            }
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 99 }];
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };
            assert_eq!(tools_of_the_trade(&mut ctx), Err(expected));
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);
        }
    }

    #[test]
    fn shroud_writer_is_exact_and_publicly_manifested() {
        assert!(IMPLEMENTED.contains(&StepKind::Shroud));
        assert!(capability_manifest().steps.contains(&StepKind::Shroud));

        for (upgrade, amount) in [(0, 3), (1, 4)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Shroud,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = [CompiledArg::I(i64::from(amount))];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            crate::engine::damage::assert_ending_window_gate(&state, "shroud", |s, _| {
                shroud(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };

            shroud(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Shroud), amount);
            assert_eq!(
                ctx.state.fanouts.after_power_amount_changed_order(),
                &[PowerId::SleightOfFlesh, PowerId::Shroud]
            );
            assert_eq!(
                ctx.events.as_slice(),
                &[Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Shroud,
                    amount,
                }]
            );

            shroud(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Shroud), 2 * amount);
            assert_eq!(
                ctx.state.fanouts.after_power_amount_changed_order(),
                &[PowerId::SleightOfFlesh, PowerId::Shroud]
            );
            assert_eq!(ctx.events.len(), 2);
        }
    }

    #[test]
    fn spectrum_shift_rows_stack_in_acquisition_order_and_overflow_atomically() {
        for (upgrade, cost) in [(0, 2), (1, 1)] {
            let identity = CardIdentity {
                id: CardId::SpectrumShift,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            assert_eq!(spec.row.cost, cost);
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.reward_card_pool = Some(crate::catalog::RewardPool::Regent);
            state.entropy_card_pool = Some(crate::catalog::RewardPool::Regent);
            state.set_spectrum_shift_generation_pool(true);
            state.fully_unlocked_card_pool_epochs = true;
            state.rng.set(
                crate::hot::RngStream::Generation,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            if upgrade == 0 {
                state.powers.set(PowerId::InfiniteBlades, SlotWire::Int, 1);
                assert!(
                    state
                        .fanouts
                        .set_before_hand_draw_order(&[PowerId::InfiniteBlades])
                );
            }
            crate::engine::damage::assert_ending_window_gate(&state, "spectrum shift", |s, _| {
                spectrum_shift(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(1)],
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(1)],
                events: &mut events,
            };

            spectrum_shift(&mut ctx).unwrap();
            spectrum_shift(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::SpectrumShift), 2);
            let expected_order: &[PowerId] = if upgrade == 0 {
                &[PowerId::InfiniteBlades, PowerId::SpectrumShift]
            } else {
                &[PowerId::SpectrumShift]
            };
            assert_eq!(ctx.state.fanouts.before_hand_draw_order(), expected_order);
            assert_eq!(ctx.events.len(), 2);
        }

        let identity = CardIdentity {
            id: CardId::SpectrumShift,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.reward_card_pool = Some(crate::catalog::RewardPool::Regent);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
            .powers
            .set(PowerId::SpectrumShift, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 77 }];
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 9,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };
        assert_eq!(
            spectrum_shift(&mut ctx),
            Err(EngineRefusal::CounterOverflow("spectrum_shift"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn shroud_foundation_refuses_malformed_and_overflow_without_mutation() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Shroud,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let good_args = [CompiledArg::I(3)];
        let bad_args = [CompiledArg::I(4)];

        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Shroud, SlotWire::Int, i32::MAX);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud])
        );
        let mut events = vec![Event::PowerChanged {
            subject: Subject::Player,
            power: PowerId::Shroud,
            amount: i32::MAX,
        }];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &good_args,
            events: &mut events,
        };
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            shroud(&mut ctx),
            Err(EngineRefusal::CounterOverflow("shroud"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        ctx.state.powers.set(PowerId::Shroud, SlotWire::Int, 0);
        assert!(ctx.state.fanouts.set_after_power_amount_changed_order(&[]));
        ctx.args = &bad_args;
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            shroud(&mut ctx),
            Err(EngineRefusal::MalformedArgs("shroud"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        ctx.args = &good_args;
        ctx.state.history.over = true;
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        shroud(&mut ctx).unwrap();
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn vicious_writer_rows_stack_in_acquisition_order_and_are_public() {
        assert!(IMPLEMENTED.contains(&StepKind::Vicious));
        assert!(capability_manifest().steps.contains(&StepKind::Vicious));

        for (upgrade, amount) in [(0, 1), (1, 2)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Vicious,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = [CompiledArg::I(i64::from(amount))];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            crate::engine::damage::assert_ending_window_gate(&state, "vicious", |s, _| {
                vicious(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };

            vicious(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Vicious), amount);
            assert_eq!(
                ctx.state.fanouts.after_power_amount_changed_order(),
                &[PowerId::SleightOfFlesh, PowerId::Vicious]
            );
            assert_eq!(
                ctx.events.as_slice(),
                &[Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Vicious,
                    amount,
                }]
            );

            vicious(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Vicious), 2 * amount);
            assert_eq!(
                ctx.state.fanouts.after_power_amount_changed_order(),
                &[PowerId::SleightOfFlesh, PowerId::Vicious]
            );
            assert_eq!(ctx.events.len(), 2);
        }
    }

    #[test]
    fn vicious_writer_refuses_malformed_and_overflow_atomically() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Vicious,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let good_args = [CompiledArg::I(1)];
        let bad_args = [CompiledArg::I(2)];
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Vicious, SlotWire::Int, i32::MAX);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        let mut events = vec![Event::TurnBegan { turn: 77 }];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &good_args,
            events: &mut events,
        };

        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            vicious(&mut ctx),
            Err(EngineRefusal::CounterOverflow("vicious"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        ctx.state.powers.set(PowerId::Vicious, SlotWire::Int, 0);
        assert!(ctx.state.fanouts.set_after_power_amount_changed_order(&[]));
        ctx.args = &bad_args;
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            vicious(&mut ctx),
            Err(EngineRefusal::MalformedArgs("vicious"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        ctx.args = &good_args;
        ctx.target = Some(0);
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            vicious(&mut ctx),
            Err(EngineRefusal::MalformedArgs("vicious"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        ctx.target = None;
        ctx.state.history.over = true;
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        vicious(&mut ctx).unwrap();
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn calcify_writer_is_exact_terminal_suppressed_and_atomic() {
        assert!(IMPLEMENTED.contains(&StepKind::Calcify));
        assert!(capability_manifest().steps.contains(&StepKind::Calcify));

        for (upgrade, amount) in [(0, 4), (1, 6)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Calcify,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = [CompiledArg::I(i64::from(amount))];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            crate::engine::damage::assert_ending_window_gate(&state, "calcify", |s, _| {
                calcify(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };

            calcify(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Calcify), amount);
            assert_eq!(
                ctx.events.as_slice(),
                &[Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Calcify,
                    amount,
                }]
            );

            ctx.state.history.over = true;
            let before = ctx.state.clone();
            let before_events = ctx.events.clone();
            calcify(&mut ctx).unwrap();
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);
        }

        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Calcify,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = [CompiledArg::I(4)];
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Calcify, SlotWire::Int, i32::MAX);
        let mut events = vec![Event::PowerChanged {
            subject: Subject::Player,
            power: PowerId::Calcify,
            amount: i32::MAX,
        }];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            calcify(&mut ctx),
            Err(EngineRefusal::CounterOverflow("calcify"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);

        let malformed = [CompiledArg::I(6)];
        ctx.args = &malformed;
        ctx.state.powers.set(PowerId::Calcify, SlotWire::Int, 0);
        let before = ctx.state.clone();
        let before_events = ctx.events.clone();
        assert_eq!(
            calcify(&mut ctx),
            Err(EngineRefusal::MalformedArgs("calcify"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn lethality_writer_pins_both_rows_shape_terminal_gate_and_overflow() {
        assert!(IMPLEMENTED.contains(&StepKind::Lethality));
        assert!(capability_manifest().steps.contains(&StepKind::Lethality));

        for (upgrade, amount) in [(0, 50), (1, 75)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Lethality,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = [CompiledArg::I(i64::from(amount))];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.powers.set(PowerId::Lethality, SlotWire::Int, 25);
            crate::engine::damage::assert_ending_window_gate(&state, "lethality", |s, _| {
                lethality(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };

            lethality(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Lethality), 25 + amount);
            assert_eq!(
                ctx.events.as_slice(),
                &[Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Lethality,
                    amount: 25 + amount,
                }]
            );

            ctx.state.history.over = true;
            let before = ctx.state.clone();
            let before_events = ctx.events.clone();
            lethality(&mut ctx).unwrap();
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);
        }

        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Lethality,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let good_args = [CompiledArg::I(50)];
        let bad_args = [CompiledArg::I(75)];
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state
            .powers
            .set(PowerId::Lethality, SlotWire::Int, i32::MAX);
        let mut events = Vec::new();
        for (target, selection, x_value, args, expected) in [
            (
                None,
                None,
                0,
                good_args.as_slice(),
                EngineRefusal::CounterOverflow("lethality"),
            ),
            (
                Some(0),
                None,
                0,
                good_args.as_slice(),
                EngineRefusal::MalformedArgs("lethality"),
            ),
            (
                None,
                Some(7),
                0,
                good_args.as_slice(),
                EngineRefusal::MalformedArgs("lethality"),
            ),
            (
                None,
                None,
                1,
                good_args.as_slice(),
                EngineRefusal::MalformedArgs("lethality"),
            ),
            (
                None,
                None,
                0,
                bad_args.as_slice(),
                EngineRefusal::MalformedArgs("lethality"),
            ),
        ] {
            let before = state.clone();
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target,
                selection,
                x_value,
                args,
                events: &mut events,
            };
            assert_eq!(lethality(&mut ctx), Err(expected));
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);
        }
    }

    #[test]
    fn shadowmeld_stacks_exact_rows_and_refuses_overflow_atomically() {
        for upgrade in [0, 1] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Shadowmeld,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = [CompiledArg::I(1)];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            crate::engine::damage::assert_ending_window_gate(&state, "shadowmeld", |s, _| {
                shadowmeld(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };

            shadowmeld(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::Shadowmeld), 3);

            ctx.state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 95);
            let before = ctx.state.clone();
            let before_events = ctx.events.clone();
            assert_eq!(
                shadowmeld(&mut ctx),
                Err(EngineRefusal::CounterOverflow("shadowmeld"))
            );
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);

            ctx.state.history.over = true;
            ctx.state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
            let before = ctx.state.clone();
            shadowmeld(&mut ctx).unwrap();
            assert_eq!(*ctx.state, before, "ending application must no-op");
        }
    }

    #[test]
    fn haunt_and_serpent_writers_pin_both_rows_and_overflow_atomically() {
        for (id, power, amounts, body) in [
            (
                CardId::Haunt,
                PowerId::HauntPower,
                [7, 9],
                haunt_power as fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>,
            ),
            (
                CardId::SerpentForm,
                PowerId::SerpentForm,
                [4, 6],
                serpent_form as fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>,
            ),
        ] {
            for (upgrade, amount) in amounts.into_iter().enumerate() {
                let mut builder = CatalogBuilder::new();
                let atom = builder
                    .intern(CardIdentity {
                        id,
                        upgrade: upgrade as u8,
                        enchantment: None,
                    })
                    .unwrap();
                let catalog = builder.build();
                let spec = *catalog.spec(atom).unwrap();
                let args = [CompiledArg::I(i64::from(amount))];
                let mut state = HotState::at_defaults();
                state.monsters_mut().push(crate::hot::HotMonster::new(
                    crate::ids::MonsterKind::Toadpole,
                    100,
                ));
                crate::engine::play::prepare_after_card_played_scalar_write(
                    &mut state, power, 0, 2,
                )
                .unwrap();
                state.powers.set(power, SlotWire::Int, 2);
                crate::engine::damage::assert_ending_window_gate(
                    &state,
                    "haunt/serpent form",
                    |s, _| {
                        body(&mut StepCtx {
                            state: s,
                            catalog: &catalog,
                            spec: &spec,
                            source_uid: 7,
                            target: None,
                            selection: None,
                            x_value: 0,
                            args: &args,
                            events: &mut Vec::new(),
                        })
                    },
                );
                let mut events = Vec::new();
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &args,
                    events: &mut events,
                };
                body(&mut ctx).unwrap();
                assert_eq!(ctx.state.powers.value(power), 2 + amount);

                ctx.state.powers.set(power, SlotWire::Int, i32::MAX);
                let snapshot = ctx.state.clone();
                let event_snapshot = ctx.events.clone();
                assert!(matches!(
                    body(&mut ctx),
                    Err(EngineRefusal::CounterOverflow(_))
                ));
                assert_eq!(*ctx.state, snapshot);
                assert_eq!(*ctx.events, event_snapshot);
            }
        }
    }

    /// Every stub still refuses **by its own name**, through the generated
    /// dispatch. A stub weakened into a silent `Ok(())` would make an
    /// unmodeled card play as if the step were absent — the exact I5 failure
    /// the escalation rule exists to prevent.
    #[test]
    fn every_escalated_stub_refuses_by_its_own_kind() {
        let (mut state, catalog) = fixture();
        let spec = *catalog
            .spec(state.piles.get(PileId::Hand).as_slice()[0].atom)
            .expect("the fixture's hand cards are interned");
        for kind in ESCALATED {
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut events,
            };
            assert_eq!(
                crate::steps::apply_step(kind, &mut ctx),
                Err(EngineRefusal::StepKindNotModeled(kind)),
                "{:?} must refuse by name",
                kind.as_str()
            );
            assert!(
                events.is_empty(),
                "{:?} emitted an event before refusing",
                kind.as_str()
            );
        }
    }

    #[test]
    fn all_eighteen_energy_next_turn_rows_stack_the_sparse_power_exactly() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::EnergyNextTurn)
            })
            .collect();
        assert_eq!(carriers.len(), 18, "nine current cards at both levels");
        let manifest = capability_manifest();
        let rows: Vec<_> = carriers
            .into_iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .all(|step| manifest.steps.contains(&step.kind))
            })
            .collect();
        assert_eq!(rows.len(), 18, "nine current cards at both levels");
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            [
                CardId::ChargeBattery,
                CardId::ChargeBattery,
                CardId::Convergence,
                CardId::Convergence,
                CardId::Delay,
                CardId::Delay,
                CardId::Hegemony,
                CardId::Hegemony,
                CardId::Invoke,
                CardId::Invoke,
                CardId::Outmaneuver,
                CardId::Outmaneuver,
                CardId::Relax,
                CardId::Relax,
                CardId::Scavenge,
                CardId::Scavenge,
                CardId::Sidestep,
                CardId::Sidestep,
            ]
        );

        for row in rows {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: row.id,
                    upgrade: row.upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog
                .steps(&spec)
                .iter()
                .find(|step| step.kind == StepKind::EnergyNextTurn)
                .copied()
                .unwrap();
            let [CompiledArg::I(amount)] = catalog.args(step.args) else {
                panic!("{} has a non-integer energy-next-turn body", row.name);
            };
            let amount: i32 = (*amount).try_into().unwrap();
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 2);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };

            energy_next_turn(&mut ctx).unwrap();

            assert_eq!(
                state.powers.value(PowerId::EnergyNextTurn),
                2 + amount,
                "{}",
                row.name
            );
            assert_eq!(events.len(), 1, "{}", row.name);
        }
    }

    #[test]
    fn energy_next_turn_overflow_refuses_before_power_or_event_mutation() {
        let identity = CardIdentity {
            id: CardId::ChargeBattery,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 1);
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(i64::from(i32::MAX))],
            events: &mut events,
        };

        assert_eq!(
            energy_next_turn(&mut ctx),
            Err(EngineRefusal::CounterOverflow("energy next turn"))
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());

        ctx.args = &[CompiledArg::I(-1)];
        assert_eq!(
            energy_next_turn(&mut ctx),
            Err(EngineRefusal::MalformedArgs("energy_next_turn"))
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());
    }

    fn ctx_state() -> (HotState, Catalog) {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 12));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 12));
        (state, catalog)
    }

    fn run(
        kind: StepKind,
        state: &mut HotState,
        catalog: &Catalog,
        target: Option<usize>,
        x_value: i64,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog
            .spec(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::StrikeIronclad,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap(),
            )
            .unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn run_source_step(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn run_source_target_step(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn fatal_fixture(
        id: CardId,
        upgrade: u8,
        monsters: Vec<HotMonster>,
        source_pile: PileId,
    ) -> (HotState, Catalog, HotCard) {
        use crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;

        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        for monster in &monsters {
            builder.intern_monster(monster.kind).unwrap();
        }
        let catalog = builder.build();
        let source = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 80;
        state.gold = 10;
        state.energy = 20;
        state.next_card_uid = 8;
        state.piles.get_mut(source_pile).make_mut().push(source);
        *state.monsters_mut() = monsters;
        (state, catalog, source)
    }

    fn run_fatal_body(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        target: usize,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog.spec(source.atom).unwrap();
        let [program] = catalog.steps(spec) else {
            panic!("fatal row must have one compiled step")
        };
        let args = catalog.args(program.args);
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid: source.uid,
            target: Some(target),
            selection: None,
            x_value: 0,
            args,
            events,
        };
        fatal_attack_reward_exact(&mut ctx)
    }

    #[test]
    fn fatal_reward_registry_is_exact_and_publicly_manifested() {
        assert!(IMPLEMENTED.contains(&StepKind::FatalAttackRewardExact));
        assert!(
            capability_manifest()
                .steps
                .contains(&StepKind::FatalAttackRewardExact)
        );
        let rows: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::FatalAttackRewardExact)
            })
            .map(|row| (row.id, row.upgrade, row.steps))
            .collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .map(|(id, upgrade, _)| (*id, *upgrade))
                .collect::<Vec<_>>(),
            [
                (CardId::Feed, 0),
                (CardId::Feed, 1),
                (CardId::HandOfGreed, 0),
                (CardId::HandOfGreed, 1),
            ]
        );
        assert_eq!(
            rows[0].2[0].args,
            &[Arg::I(10), Arg::S("max_hp"), Arg::I(3)]
        );
        assert_eq!(
            rows[1].2[0].args,
            &[Arg::I(12), Arg::S("max_hp"), Arg::I(4)]
        );
        assert_eq!(rows[2].2[0].args, &[Arg::I(20), Arg::S("gold"), Arg::I(20)]);
        assert_eq!(rows[3].2[0].args, &[Arg::I(25), Arg::S("gold"), Arg::I(25)]);
    }

    #[test]
    fn fatal_rewards_cover_both_levels_nonfatal_overkill_and_final_lethal_order() {
        for (id, upgrade, damage, reward) in [
            (CardId::Feed, 0, 10, 3),
            (CardId::Feed, 1, 12, 4),
            (CardId::HandOfGreed, 0, 20, 20),
            (CardId::HandOfGreed, 1, 25, 25),
        ] {
            let (mut nonfatal, catalog, source) = fatal_fixture(
                id,
                upgrade,
                vec![HotMonster::new(MonsterKind::Toadpole, damage + 1)],
                PileId::Draw,
            );
            run_fatal_body(&mut nonfatal, &catalog, source, 0, &mut Vec::new()).unwrap();
            assert_eq!(nonfatal.max_hp, 80);
            assert_eq!(nonfatal.gold, 10);
            assert!(!nonfatal.history.over);

            let (mut lethal, catalog, source) = fatal_fixture(
                id,
                upgrade,
                vec![HotMonster::new(MonsterKind::Toadpole, damage - 2)],
                PileId::Play,
            );
            let mut events = Vec::new();
            run_fatal_body(&mut lethal, &catalog, source, 0, &mut events).unwrap();
            assert!(lethal.history.over);
            assert_eq!(
                events.last(),
                Some(&Event::CombatOver { player_won: true }),
                "the ungated reward succeeds after the terminal event"
            );
            match id {
                CardId::Feed => {
                    assert_eq!(lethal.max_hp, 80 + reward);
                    assert_eq!(lethal.hp, 50 + reward);
                    assert_eq!(lethal.gold, 10);
                }
                CardId::HandOfGreed => {
                    assert_eq!(lethal.max_hp, 80);
                    assert_eq!(lethal.hp, 50);
                    assert_eq!(lethal.gold, 10 + reward);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn fatal_snapshot_distinguishes_corpses_secondaries_and_segment_conjunction() {
        let mut corpse = HotMonster::new(MonsterKind::Toadpole, 1);
        corpse.hp = 0;
        let (mut state, catalog, source) =
            fatal_fixture(CardId::Feed, 0, vec![corpse], PileId::Draw);
        run_fatal_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!((state.max_hp, state.hp), (80, 50));

        let mut secondary = HotMonster::new(MonsterKind::Toadpole, 1);
        secondary.powers.set(PowerId::Secondary, SlotWire::Int, 1);
        let (mut state, catalog, source) =
            fatal_fixture(CardId::Feed, 0, vec![secondary], PileId::Draw);
        run_fatal_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!((state.max_hp, state.hp), (80, 50));

        let alive_other = HotMonster::new(MonsterKind::DecimillipedeSegment, 20);
        let target = HotMonster::new(MonsterKind::DecimillipedeSegment, 1);
        let (mut state, catalog, source) = fatal_fixture(
            CardId::Feed,
            0,
            vec![target.clone(), alive_other],
            PileId::Draw,
        );
        run_fatal_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!((state.max_hp, state.hp), (80, 50));

        let mut dead_other = HotMonster::new(MonsterKind::DecimillipedeSegment, 20);
        dead_other.hp = 0;
        let (mut state, catalog, source) =
            fatal_fixture(CardId::Feed, 0, vec![target, dead_other], PileId::Draw);
        run_fatal_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!((state.max_hp, state.hp), (83, 53));
    }

    #[test]
    fn fatal_rewards_preserve_stock_rng_and_max_hp_accepted_delta() {
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 1);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut stock, catalog, source) =
            fatal_fixture(CardId::HandOfGreed, 0, vec![axebot], PileId::Draw);
        let niche_before = stock.rng.get(crate::hot::RngStream::Niche);
        run_fatal_body(&mut stock, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!(stock.gold, 30);
        assert!(stock.monsters[0].hp > 0);
        assert_eq!(stock.monsters[0].powers.value(PowerId::Stock), 1);
        assert_eq!(
            stock.rng.get(crate::hot::RngStream::Niche).counter,
            niche_before.counter + 1
        );

        let (mut capped, catalog, source) = fatal_fixture(
            CardId::Feed,
            1,
            vec![HotMonster::new(MonsterKind::Toadpole, 1)],
            PileId::Draw,
        );
        capped.max_hp = 999_999_998;
        capped.hp = 999_999_995;
        run_fatal_body(&mut capped, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!(capped.max_hp, 999_999_999);
        assert_eq!(capped.hp, 999_999_996, "Heal receives only accepted delta");
    }

    #[test]
    fn fatal_reward_late_overflow_and_bad_source_are_body_local_atomic() {
        let (mut overflow, catalog, source) = fatal_fixture(
            CardId::HandOfGreed,
            0,
            vec![HotMonster::new(MonsterKind::Toadpole, 1)],
            PileId::Draw,
        );
        overflow.gold = i32::MAX - 10;
        let before = overflow.clone();
        let mut events = vec![Event::TurnBegan { turn: 77 }];
        let before_events = events.clone();
        assert_eq!(
            run_fatal_body(&mut overflow, &catalog, source, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("gold"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);

        let (mut missing, catalog, source) = fatal_fixture(
            CardId::Feed,
            0,
            vec![HotMonster::new(MonsterKind::Toadpole, 1)],
            PileId::Draw,
        );
        missing.piles.get_mut(PileId::Draw).make_mut().clear();
        let before = missing.clone();
        assert_eq!(
            run_fatal_body(&mut missing, &catalog, source, 0, &mut Vec::new()),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 0,
            })
        );
        assert_eq!(missing, before);
    }

    #[test]
    fn fatal_public_play_late_failures_restore_the_complete_shared_prefix() {
        use crate::engine::play::play_card;

        for upgrade in [0, 1] {
            let (mut state, catalog, source) = fatal_fixture(
                CardId::HandOfGreed,
                upgrade,
                vec![HotMonster::new(MonsterKind::Toadpole, 1)],
                PileId::Hand,
            );
            state.gold = i32::MAX - 10;
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 81 }];
            let before_events = events.clone();
            assert_eq!(
                play_card(&mut state, &catalog, source.uid, Some(0), None, &mut events,),
                Err(EngineRefusal::CounterOverflow("gold"))
            );
            assert_eq!(state, before, "Hand of Greed+{upgrade} state leaked");
            assert_eq!(
                events, before_events,
                "Hand of Greed+{upgrade} events leaked"
            );

            let mut malformed_stock = HotMonster::new(MonsterKind::Axebot, 1);
            malformed_stock.max_hp = 75;
            malformed_stock.powers.set(PowerId::Stock, SlotWire::Int, 2);
            let (mut state, catalog, source) =
                fatal_fixture(CardId::Feed, upgrade, vec![malformed_stock], PileId::Hand);
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 82 }];
            let before_events = events.clone();
            assert_eq!(
                play_card(&mut state, &catalog, source.uid, Some(0), None, &mut events,),
                Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"))
            );
            assert_eq!(state, before, "Feed+{upgrade} state leaked");
            assert_eq!(events, before_events, "Feed+{upgrade} events leaked");
        }
    }

    #[test]
    fn fatal_reward_replay_rewards_only_the_killing_body_and_terminal_suppresses_replay() {
        use crate::engine::play::play_card;

        for (hp, expected_finished) in [(25, 2), (10, 1)] {
            let (mut state, catalog, source) = fatal_fixture(
                CardId::HandOfGreed,
                0,
                vec![HotMonster::new(MonsterKind::Toadpole, hp)],
                PileId::Hand,
            );
            state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.gold, 30, "exactly one command result killed");
            assert_eq!(
                state.history.card_plays_finished_combat, expected_finished,
                "a terminal first body suppresses its queued replay"
            );
        }
    }

    #[test]
    fn fatal_reward_stock_replay_refuses_with_whole_action_state_rollback() {
        use crate::engine::play::play_card;

        let mut axebot = HotMonster::new(MonsterKind::Axebot, 1);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut state, catalog, source) =
            fatal_fixture(CardId::HandOfGreed, 0, vec![axebot], PileId::Hand);
        state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 83 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(&mut state, &catalog, source.uid, Some(0), None, &mut events,),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before, "the mutable play never leaks its prefix");
        assert_eq!(events, before_events, "the rejected events are retracted");

        let action = crate::engine::Action::Play {
            uid: source.uid,
            target: Some(0),
            selection: crate::engine::SelectionRef::NONE,
        };
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::apply_action_into(&state, &catalog, &action, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before, "the rejected successor never escapes");
        assert!(
            events.is_empty(),
            "the cloned action exposes no rejected events"
        );
    }

    #[test]
    fn soul_body_registry_and_serial_generation_are_exact() {
        use crate::engine::play::play_card;
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard};

        let identities = [
            CardIdentity {
                id: CardId::GraveWarden,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::GraveWarden,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Reave,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Reave,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Severance,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Severance,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::CaptureSpirit,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::CaptureSpirit,
                upgrade: 1,
                enchantment: None,
            },
        ];
        let actual: std::collections::BTreeSet<_> = crate::content_tables::CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| step.kind == StepKind::SoulBody))
            .map(|row| (row.id, row.upgrade))
            .collect();
        assert_eq!(
            actual,
            identities
                .iter()
                .map(|identity| (identity.id, identity.upgrade))
                .collect()
        );

        for identity in identities {
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let soul_upgrade = u8::from(matches!(
                identity,
                CardIdentity {
                    id: CardId::Reave,
                    upgrade: 1,
                    ..
                }
            ));
            assert!(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::Soul,
                        upgrade: soul_upgrade,
                        enchantment: None,
                    })
                    .is_some()
            );

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 20;
            state.next_card_uid = 2;
            if matches!(identity.id, CardId::CaptureSpirit) {
                state.powers.set(PowerId::Strength, SlotWire::Int, 99);
            }
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
            monster.block = 5;
            if matches!(identity.id, CardId::CaptureSpirit) {
                monster.powers.set(PowerId::Vulnerable, SlotWire::Int, 2);
            }
            state.monsters_mut().push(monster);
            play_card(
                &mut state,
                &catalog,
                1,
                catalog.spec(atom).unwrap().targeted.then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();

            match identity.id {
                CardId::GraveWarden => {
                    assert_eq!(state.block, if identity.upgrade == 0 { 8 } else { 11 });
                    assert_eq!(state.piles.get(PileId::Draw).len(), 1);
                }
                CardId::Reave => {
                    assert_eq!(
                        state.monsters[0].hp,
                        if identity.upgrade == 0 { 95 } else { 92 }
                    );
                    assert_eq!(state.piles.get(PileId::Draw).len(), 1);
                }
                CardId::Severance => {
                    assert_eq!(
                        state.monsters[0].hp,
                        if identity.upgrade == 0 { 92 } else { 87 }
                    );
                    assert_eq!(state.piles.get(PileId::Draw).len(), 1);
                    assert_eq!(
                        state.piles.get(PileId::Discard).len(),
                        2,
                        "generated Soul follows the routed source"
                    );
                    assert_eq!(state.piles.get(PileId::Hand).len(), 1);
                }
                CardId::CaptureSpirit => {
                    assert_eq!(
                        state.monsters[0].block, 5,
                        "unblockable damage preserves block"
                    );
                    assert_eq!(
                        state.monsters[0].hp,
                        if identity.upgrade == 0 { 97 } else { 96 }
                    );
                    assert_eq!(
                        state.piles.get(PileId::Draw).len(),
                        usize::from(3 + identity.upgrade)
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn soul_body_terminal_suffix_and_late_failure_are_atomic() {
        use crate::engine::play::play_card;
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard};

        let identity = CardIdentity {
            id: CardId::CaptureSpirit,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let base = || {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 5;
            state.next_card_uid = 2;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 4));
            state
        };

        let mut lethal = base();
        play_card(&mut lethal, &catalog, 1, Some(0), None, &mut Vec::new()).unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.next_card_uid, 2);
        assert!(lethal.piles.get(PileId::Draw).is_empty());
        assert_eq!(lethal.history.owner_generated_cards_combat, 4);
        assert_eq!(lethal.next_generated_hook_uid, 4);
        assert_eq!(lethal.rng.get(crate::hot::RngStream::Rng).counter, 0);

        for (id, generated) in [(CardId::Reave, 1), (CardId::Severance, 3)] {
            let identity = CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let attack_atom = builder.intern(identity).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let attack_catalog = builder.build();
            let mut ended = HotState::at_defaults();
            ended.hp = 50;
            ended.energy = 5;
            ended.next_card_uid = 2;
            ended.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: attack_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            ended
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));
            play_card(
                &mut ended,
                &attack_catalog,
                1,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(ended.history.over, "{id:?}");
            assert_eq!(
                ended.history.owner_generated_cards_combat, generated,
                "{id:?}"
            );
            assert_eq!(ended.next_generated_hook_uid, generated, "{id:?}");
            assert_eq!(ended.next_card_uid, 2, "{id:?}");
            assert_eq!(
                ended.rng.get(crate::hot::RngStream::Rng).counter,
                0,
                "{id:?}"
            );
            assert!(ended.piles.get(PileId::Draw).is_empty(), "{id:?}");
        }

        let mut overflow = base();
        overflow.monsters_mut()[0].hp = 100;
        overflow.next_card_uid = u32::MAX - 2;
        let before = overflow.clone();
        assert_eq!(
            play_card(&mut overflow, &catalog, 1, Some(0), None, &mut Vec::new()),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(overflow, before, "the complete public action rolls back");
    }

    fn resolve_operand(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        catalog: &Catalog,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<i64, EngineRefusal> {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut events = Vec::new();
        let ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        calculated_operand(&ctx, kind)
    }

    #[test]
    fn calculated_generated_census_is_exactly_nineteen_sources_and_two_upgrades() {
        use std::collections::BTreeSet;

        const SOURCES: [(CardId, StepKind); 19] = [
            (CardId::AshenStrike, StepKind::CalculatedAttack),
            (CardId::Barrage, StepKind::CalculatedHits),
            (CardId::BodySlam, StepKind::CalculatedAttack),
            (CardId::DeathMarch, StepKind::CalculatedAttack),
            (CardId::Dismantle, StepKind::CalculatedHits),
            (CardId::ExpectAFight, StepKind::CalculatedBlock),
            (CardId::Finisher, StepKind::CalculatedHits),
            (CardId::Flechettes, StepKind::CalculatedHits),
            (CardId::GoldAxe, StepKind::CalculatedAttack),
            (CardId::LunarBlast, StepKind::CalculatedHits),
            (CardId::MementoMori, StepKind::CalculatedAttack),
            (CardId::MindBlast, StepKind::CalculatedAttack),
            (CardId::Murder, StepKind::CalculatedAttack),
            (CardId::NoEscape, StepKind::CalculatedDoomExact),
            (CardId::PreciseCut, StepKind::CalculatedAttack),
            (CardId::SoulStorm, StepKind::CalculatedAttack),
            (CardId::Stack, StepKind::CalculatedBlock),
            (CardId::Supermassive, StepKind::CalculatedAttack),
            (CardId::TimesUp, StepKind::CalculatedAttack),
        ];
        const KINDS: [StepKind; 4] = [
            StepKind::CalculatedAttack,
            StepKind::CalculatedBlock,
            StepKind::CalculatedDoomExact,
            StepKind::CalculatedHits,
        ];

        let actual: BTreeSet<_> = crate::content_tables::CARD_ROWS
            .iter()
            .flat_map(|row| {
                row.steps
                    .iter()
                    .filter(|step| KINDS.contains(&step.kind))
                    .map(|step| (row.id, row.upgrade, step.kind))
            })
            .collect();
        let expected: BTreeSet<_> = SOURCES
            .into_iter()
            .flat_map(|(id, kind)| [(id, 0, kind), (id, 1, kind)])
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 38);

        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        let base = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut admitted = 0_usize;
        let mut independently_refused = BTreeSet::new();
        for (id, upgrade, _) in actual {
            let identity = CardIdentity {
                id,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            for spec in base.specs() {
                builder.intern(spec.identity).unwrap();
            }
            let atom = builder.intern(identity).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let boundary_state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            if let Err(refusal) =
                crate::engine::admission::admit(&document, &boundary_state, &catalog)
            {
                assert!(
                    refusal.to_string().contains("card keyword"),
                    "{}+{} had unexpected refusal: {refusal}",
                    id.as_str(),
                    upgrade
                );
                independently_refused.insert((id, upgrade));
                continue;
            }
            admitted += 1;

            let spec = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 20;
            state.next_card_uid = 2;
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(crate::hot::HotCard {
                    uid: 1,
                    atom,
                    flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1_000));
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                1,
                spec.targeted.then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap_or_else(|refusal| {
                panic!(
                    "{}+{} admitted but did not play: {refusal}",
                    id.as_str(),
                    upgrade
                )
            });
        }
        assert_eq!(admitted, 38);
        assert!(independently_refused.is_empty());
    }

    /// #3044, the census witness `f4bc0c1061c13870` (THE_OBSCURA): Expect a
    /// Fight+ reads `GetPowerAmount<StrengthPower>`
    /// (`<get_CanonicalVars>b__4_0` RVA `0x39c766` IL_000d), and Red Skull's 3
    /// is part of that power. At Strength 4 an owner dropped to 28/80 holds 7,
    /// so the block is 16 + 8 x 7 = 72, not 16 + 8 x 4 = 48.
    #[test]
    fn expect_a_fight_counts_red_skulls_strength() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::ExpectAFight,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 80;
        state.fanouts.set_red_skull_owned(true);
        state.powers.set(PowerId::Strength, SlotWire::Int, 4);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let block = |state: &mut HotState| {
            resolve_operand(
                StepKind::CalculatedBlock,
                CardId::ExpectAFight,
                1,
                state,
                &catalog,
                None,
                &[
                    CompiledArg::Word(StepWord::StrengthPowerAmount),
                    CompiledArg::I(16),
                    CompiledArg::I(8),
                ],
            )
        };
        assert_eq!(block(&mut state), Ok(48));
        crate::engine::damage::damage_player_from_card(&mut state, 22, false, &mut Vec::new())
            .unwrap();
        assert_eq!(state.hp, 28);
        assert_eq!(block(&mut state), Ok(72));
    }

    #[test]
    fn calculated_resolver_reads_every_live_operand_and_refuses_wrong_pairs() {
        use crate::hot::{HotCard, HotOrb};

        const IDS: [CardId; 22] = [
            CardId::AshenStrike,
            CardId::Barrage,
            CardId::BodySlam,
            CardId::DeathMarch,
            CardId::Dismantle,
            CardId::ExpectAFight,
            CardId::Finisher,
            CardId::Flechettes,
            CardId::GoldAxe,
            CardId::LunarBlast,
            CardId::MementoMori,
            CardId::MindBlast,
            CardId::Murder,
            CardId::NoEscape,
            CardId::PreciseCut,
            CardId::SoulStorm,
            CardId::Stack,
            CardId::Supermassive,
            CardId::TimesUp,
            CardId::Soul,
            CardId::DefendIronclad,
            CardId::StrikeIronclad,
        ];
        let mut builder = CatalogBuilder::new();
        for id in IDS {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let card = |uid, id| HotCard {
            uid,
            atom: catalog
                .atom(&CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap(),
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.block = 7;
        state.powers.set(PowerId::Strength, SlotWire::Int, -3);
        state.powers.set(PowerId::TempStrength, SlotWire::Int, 5);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(1, CardId::StrikeIronclad),
            card(2, CardId::StrikeIronclad),
        ]);
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            card(3, CardId::StrikeIronclad),
            card(4, CardId::StrikeIronclad),
            card(5, CardId::StrikeIronclad),
        ]);
        state
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .extend([card(6, CardId::Soul), card(7, CardId::StrikeIronclad)]);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(8, CardId::DefendIronclad),
            card(9, CardId::Stack),
            card(10, CardId::StrikeIronclad),
        ]);
        state.orbs.set_slots(2);
        state.orbs.set_orbs(vec![
            HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
            HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
        ]);
        state.history.non_hand_draws_this_turn = 4;
        state.history.card_plays_finished_combat = 6;
        state.history.discarded_cards_this_turn = 7;
        state.cards_drawn_combat = 8;
        state.history.owner_generated_cards_combat = 9;
        state.history.attack_plays_finished_this_turn = 10;
        state.history.skill_plays_finished_this_turn = 11;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 25);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 1);

        let amount_cases: &[(CardId, StepKind, StepWord, i64, i64, i64)] = &[
            (
                CardId::BodySlam,
                StepKind::CalculatedAttack,
                StepWord::PlayerBlock,
                0,
                1,
                7,
            ),
            (
                CardId::MindBlast,
                StepKind::CalculatedAttack,
                StepWord::DrawCount,
                0,
                1,
                2,
            ),
            (
                CardId::Stack,
                StepKind::CalculatedBlock,
                StepWord::DiscardCount,
                0,
                1,
                3,
            ),
            (
                CardId::ExpectAFight,
                StepKind::CalculatedBlock,
                StepWord::StrengthPowerAmount,
                15,
                5,
                25,
            ),
            (
                CardId::AshenStrike,
                StepKind::CalculatedAttack,
                StepWord::ExhaustCount,
                6,
                3,
                12,
            ),
            (
                CardId::SoulStorm,
                StepKind::CalculatedAttack,
                StepWord::ExhaustExactSoul,
                9,
                4,
                13,
            ),
            (
                CardId::TimesUp,
                StepKind::CalculatedAttack,
                StepWord::TargetDoom,
                0,
                1,
                25,
            ),
            (
                CardId::DeathMarch,
                StepKind::CalculatedAttack,
                StepWord::NonHandDrawsThisTurn,
                8,
                4,
                24,
            ),
            (
                CardId::GoldAxe,
                StepKind::CalculatedAttack,
                StepWord::CardPlaysFinishedCombat,
                0,
                1,
                6,
            ),
            (
                CardId::MementoMori,
                StepKind::CalculatedAttack,
                StepWord::DiscardedCardsThisTurn,
                9,
                4,
                37,
            ),
            (
                CardId::Murder,
                StepKind::CalculatedAttack,
                StepWord::CardsDrawnCombat,
                1,
                1,
                9,
            ),
            (
                CardId::Supermassive,
                StepKind::CalculatedAttack,
                StepWord::OwnerGeneratedCardsCombat,
                5,
                3,
                32,
            ),
            (
                CardId::PreciseCut,
                StepKind::CalculatedAttack,
                StepWord::NegativeHandCount,
                13,
                2,
                7,
            ),
            (
                CardId::NoEscape,
                StepKind::CalculatedDoomExact,
                StepWord::TargetDoomTens,
                10,
                5,
                20,
            ),
        ];
        for &(id, kind, source, base, extra, expected) in amount_cases {
            assert_eq!(
                resolve_operand(
                    kind,
                    id,
                    0,
                    &mut state,
                    &catalog,
                    Some(0),
                    &[
                        CompiledArg::Word(source),
                        CompiledArg::I(base),
                        CompiledArg::I(extra),
                    ],
                ),
                Ok(expected),
                "{}",
                id.as_str()
            );
        }

        let hit_cases = [
            (CardId::Barrage, 5, StepWord::OrbCount, 2),
            (CardId::Flechettes, 5, StepWord::HandSkillCount, 2),
            (CardId::Finisher, 6, StepWord::FinishedAttackCount, 10),
            (CardId::LunarBlast, 4, StepWord::FinishedSkillCount, 11),
            (CardId::Dismantle, 8, StepWord::TargetVulnerableHits, 2),
        ];
        for (id, damage, source, expected) in hit_cases {
            assert_eq!(
                resolve_operand(
                    StepKind::CalculatedHits,
                    id,
                    0,
                    &mut state,
                    &catalog,
                    Some(0),
                    &[CompiledArg::I(damage), CompiledArg::Word(source)],
                ),
                Ok(expected),
                "{}",
                id.as_str()
            );
        }
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 0);
        assert_eq!(
            resolve_operand(
                StepKind::CalculatedHits,
                CardId::Dismantle,
                0,
                &mut state,
                &catalog,
                Some(0),
                &[
                    CompiledArg::I(8),
                    CompiledArg::Word(StepWord::TargetVulnerableHits),
                ],
            ),
            Ok(1)
        );
        assert_eq!(
            resolve_operand(
                StepKind::CalculatedAttack,
                CardId::BodySlam,
                0,
                &mut state,
                &catalog,
                Some(0),
                &[
                    CompiledArg::Word(StepWord::DrawCount),
                    CompiledArg::I(0),
                    CompiledArg::I(1),
                ],
            ),
            Err(EngineRefusal::MalformedArgs("calculated_attack"))
        );
    }

    #[test]
    fn calculated_commands_enter_existing_attack_and_block_lifecycles() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.block = 7;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        run_source_target_step(
            StepKind::CalculatedAttack,
            CardId::BodySlam,
            0,
            &mut state,
            Some(0),
            &[
                CompiledArg::Word(StepWord::PlayerBlock),
                CompiledArg::I(0),
                CompiledArg::I(1),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 13);

        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        run_source_target_step(
            StepKind::CalculatedHits,
            CardId::Barrage,
            0,
            &mut state,
            Some(0),
            &[CompiledArg::I(5), CompiledArg::Word(StepWord::OrbCount)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 13, "zero hits deal no damage");
        assert_eq!(
            state.powers.value(PowerId::Vigor),
            0,
            "the zero-hit AttackCommand still completes AfterAttack"
        );

        state.block = 0;
        let identity = CardIdentity {
            id: CardId::Stack,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((1..=2).map(|uid| crate::hot::HotCard {
                uid,
                atom,
                flags: 0,
            }));
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args: &[
                CompiledArg::Word(StepWord::DiscardCount),
                CompiledArg::I(0),
                CompiledArg::I(1),
            ],
            events: &mut events,
        };
        calculated_block(&mut ctx).unwrap();
        assert_eq!(state.block, 2);

        state.history.over = true;
        run_source_target_step(
            StepKind::CalculatedAttack,
            CardId::BodySlam,
            0,
            &mut state,
            Some(0),
            &[CompiledArg::Word(StepWord::DrawCount)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 13);
    }

    #[test]
    fn calculated_multihit_reenters_the_waterfall_second_form_after_actual_death() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 8);
        waterfall.max_hp = 250;
        waterfall.pressure_gun_damage = 23;
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 29);
        waterfall.powers.set(PowerId::Vuln, SlotWire::Int, 1);
        waterfall.misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Vuln]);
        state.monsters_mut().push(waterfall);

        run_source_target_step(
            StepKind::CalculatedHits,
            CardId::Dismantle,
            0,
            &mut state,
            Some(0),
            &[
                CompiledArg::I(8),
                CompiledArg::Word(StepWord::TargetVulnerableHits),
            ],
        )
        .unwrap();

        let waterfall = &state.monsters[0];
        assert!(waterfall.is_about_to_blow());
        assert_eq!(waterfall.hp, 999_999_991);
        assert_eq!(waterfall.powers.value(PowerId::SteamPressure), 29);
        assert_eq!(waterfall.powers.value(PowerId::Vuln), 0);
        assert!(!state.history.over);
    }

    #[test]
    fn generic_attribute_sources_are_exact_and_dexterity_is_ending_gated() {
        let strength_sources = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps.iter().find_map(|step| {
                    (step.kind == StepKind::Strength).then(|| {
                        let [Arg::I(amount)] = step.args else {
                            panic!("{}+{} has non-scalar Strength", row.name, row.upgrade);
                        };
                        (row.id, row.upgrade, *amount)
                    })
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            strength_sources,
            [
                (CardId::Brand, 0, 1),
                (CardId::Brand, 1, 2),
                (CardId::BulkUp, 0, 2),
                (CardId::BulkUp, 1, 3),
                (CardId::FightMe, 0, 3),
                (CardId::FightMe, 1, 4),
                (CardId::Inflame, 0, 2),
                (CardId::Inflame, 1, 3),
                (CardId::Prowess, 0, 1),
                (CardId::Prowess, 1, 2),
                (CardId::Resonance, 0, 1),
                (CardId::Resonance, 1, 2),
            ]
        );
        // #3495: `PowerCmd.Apply<DexterityPower>` returns on `IsEnding`
        // (`<Apply>d__1`1` 0x3ef988 IL_0020-0034). The frozen Python applied
        // it after the outcome was fixed; native does not.
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 12));
        let mut over = state.clone();
        over.history.over = true;
        let before = over.clone();
        run_source_step(
            StepKind::Dexterity,
            CardId::Footwork,
            0,
            &mut over,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(over, before, "no Dexterity once the combat is over");
        run_source_step(
            StepKind::Dexterity,
            CardId::Footwork,
            0,
            &mut state,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::Dexterity), 2);

        run_source_step(
            StepKind::Dexterity,
            CardId::Prowess,
            1,
            &mut state,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::Dexterity), 4);
        for upgrade in 0..=1 {
            run_source_step(
                StepKind::Dexterity,
                CardId::Abrasive,
                upgrade,
                &mut state,
                &[CompiledArg::I(1)],
            )
            .unwrap();
        }
        assert_eq!(state.powers.value(PowerId::Dexterity), 6);

        let mut overflow = HotState::at_defaults();
        overflow
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 12));
        overflow
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, i32::MAX);
        let before = overflow.clone();
        assert_eq!(
            run_source_step(
                StepKind::Dexterity,
                CardId::Abrasive,
                0,
                &mut overflow,
                &[CompiledArg::I(1)],
            ),
            Err(EngineRefusal::CounterOverflow("dexterity"))
        );
        assert_eq!(overflow, before);

        state.history.over = false;
        run_source_step(
            StepKind::Strength,
            CardId::Inflame,
            1,
            &mut state,
            &[CompiledArg::I(3)],
        )
        .unwrap();
        run_source_step(
            StepKind::Strength,
            CardId::Prowess,
            0,
            &mut state,
            &[CompiledArg::I(1)],
        )
        .unwrap();
        for (upgrade, amount) in [(0, 3), (1, 4)] {
            run_source_step(
                StepKind::Strength,
                CardId::FightMe,
                upgrade,
                &mut state,
                &[CompiledArg::I(amount)],
            )
            .unwrap();
        }
        assert_eq!(state.powers.value(PowerId::Strength), 11);

        assert_eq!(
            run_source_step(
                StepKind::Dexterity,
                CardId::Footwork,
                0,
                &mut state,
                &[CompiledArg::I(3)],
            ),
            Err(EngineRefusal::MalformedArgs("dexterity"))
        );
        assert_eq!(
            run_source_step(
                StepKind::Strength,
                CardId::FightMe,
                1,
                &mut state,
                &[CompiledArg::I(3)],
            ),
            Err(EngineRefusal::MalformedArgs("strength"))
        );
    }

    #[test]
    fn attack_all_sweeps_the_living_roster_and_skips_the_dead() {
        let (mut state, catalog) = ctx_state();
        state.monsters_mut()[1].hp = 0;
        run(
            StepKind::AttackAll,
            &mut state,
            &catalog,
            None,
            0,
            &[CompiledArg::I(4), CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 4);
        assert_eq!(state.monsters[1].hp, 0);
    }

    #[test]
    fn attack_all_temp_strength_keeps_the_pre_attack_identity_snapshot() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        run_source_step(
            StepKind::AttackAllTempStrengthSnapshot,
            CardId::CrushUnder,
            0,
            &mut state,
            &[CompiledArg::I(8), CompiledArg::I(1)],
        )
        .unwrap();
        for monster in state.monsters.iter() {
            assert_eq!(monster.hp, 12);
            assert_eq!(monster.powers.value(PowerId::Strength), -1);
            assert_eq!(monster.powers.value(PowerId::TempStrength), -1);
        }
    }

    #[test]
    fn power_all_serial_applies_each_haze_power_to_the_frozen_roster() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        run_source_step(
            StepKind::PowerAllSerial,
            CardId::Haze,
            0,
            &mut state,
            &[CompiledArg::Power(PowerId::Poison), CompiledArg::I(4)],
        )
        .unwrap();
        run_source_step(
            StepKind::PowerAllSerial,
            CardId::Haze,
            0,
            &mut state,
            &[CompiledArg::Power(PowerId::Weak), CompiledArg::I(1)],
        )
        .unwrap();
        for monster in state.monsters.iter() {
            assert_eq!(monster.powers.value(PowerId::Poison), 4);
            assert_eq!(monster.powers.value(PowerId::Weak), 1);
            assert_eq!(
                monster.misery_debuff_order.as_slice(),
                &[MiseryToken::Poison, MiseryToken::Weak]
            );
        }
        assert_eq!(state.next_poison_uid, 2);

        assert_eq!(
            run_source_step(
                StepKind::PowerAllSerial,
                CardId::Haze,
                0,
                &mut state,
                &[CompiledArg::Power(PowerId::Doom), CompiledArg::I(4)],
            ),
            Err(EngineRefusal::MalformedArgs("power_all_serial"))
        );
    }

    #[test]
    fn power_all_serial_covers_doom_signed_strength_and_temporary_strength() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        run_source_step(
            StepKind::PowerAllSerial,
            CardId::Deathbringer,
            0,
            &mut state,
            &[CompiledArg::Power(PowerId::Doom), CompiledArg::I(21)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 21);
        assert!(state.history.doom_applied_by_player_this_turn);

        run_source_step(
            StepKind::PowerAllSerial,
            CardId::Resonance,
            0,
            &mut state,
            &[
                CompiledArg::Power(PowerId::StrengthEnemy),
                CompiledArg::I(-1),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -1);

        run_source_step(
            StepKind::PowerAllSerial,
            CardId::PiercingWail,
            0,
            &mut state,
            &[
                CompiledArg::Power(PowerId::TempStrengthEnemy),
                CompiledArg::I(6),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -7);
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -6);
    }

    #[test]
    fn attack_x_hits_the_target_x_times_and_zero_x_is_a_real_command() {
        let (mut state, catalog) = ctx_state();
        run(
            StepKind::AttackX,
            &mut state,
            &catalog,
            Some(0),
            3,
            &[CompiledArg::I(4)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[1].hp, 12);

        run(
            StepKind::AttackX,
            &mut state,
            &catalog,
            Some(1),
            0,
            &[CompiledArg::I(4)],
        )
        .unwrap();
        assert_eq!(state.monsters[1].hp, 12);
    }

    #[test]
    fn weak_applies_through_the_card_sourced_path_and_skips_a_dead_target() {
        let (mut state, catalog) = ctx_state();
        run(
            StepKind::Weak,
            &mut state,
            &catalog,
            Some(0),
            0,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            &[MiseryToken::Weak]
        );

        state.monsters_mut()[1].hp = 0;
        run(
            StepKind::Weak,
            &mut state,
            &catalog,
            Some(1),
            0,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.monsters[1].powers.value(PowerId::Weak), 0);
    }

    #[test]
    fn hp_loss_is_unblockable_fires_lethal_and_refuses_after_the_ending_kill() {
        let (mut state, catalog) = ctx_state();
        state.block = 5;
        run(
            StepKind::HpLoss,
            &mut state,
            &catalog,
            None,
            0,
            &[CompiledArg::I(3)],
        )
        .unwrap();
        assert_eq!(state.hp, 47);
        assert_eq!(state.block, 5);
        assert_eq!(state.history.player_unblocked_damage_results_combat, 1);

        state.history.over = true;
        assert_eq!(
            run(
                StepKind::HpLoss,
                &mut state,
                &catalog,
                None,
                0,
                &[CompiledArg::I(3)],
            ),
            Err(EngineRefusal::CombatOver)
        );
        assert_eq!(state.hp, 47);
    }

    #[test]
    fn energy_is_ending_gated_like_gain_energy() {
        // #3495: `PlayerCmd/<GainEnergy>d__3` (0x3ee8a0) returns on
        // `IsEnding` (IL_0030-003c). The frozen Python gained it after the
        // outcome was fixed; native does not.
        let (mut state, catalog) = ctx_state();
        state.energy = 1;
        let mut over = state.clone();
        over.history.over = true;
        run(
            StepKind::Energy,
            &mut over,
            &catalog,
            None,
            0,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(over.energy, 1);
        run(
            StepKind::Energy,
            &mut state,
            &catalog,
            None,
            0,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.energy, 3);
    }

    /// #3502: `PlayerCmd/<GainEnergy>d__3` (`0x3ee8a0`) returns on a
    /// non-positive amount (IL_0019-002b), and `Hook.ModifyEnergyGain`
    /// (IL_0062) zeroes the gain while the owner holds `NoEnergyGainPower`
    /// (`ModifyEnergyGain` `0xa4f9d`, the DLL's only override). The ending
    /// gate is pinned above, on an over state and here on an ending one.
    #[test]
    fn energy_step_folds_modify_energy_gain_and_skips_non_positive_amounts() {
        let (mut live, catalog) = ctx_state();
        live.energy = 1;
        let gain = |state: &mut HotState, amount: i64| {
            run(
                StepKind::Energy,
                state,
                &catalog,
                None,
                0,
                &[CompiledArg::I(amount)],
            )
            .unwrap();
        };

        let mut negative = live.clone();
        gain(&mut negative, -1);
        assert_eq!(negative.energy, 1, "a negative GainEnergy is a no-op");
        let mut zero = live.clone();
        gain(&mut zero, 0);
        assert_eq!(zero.energy, 1);

        let mut blocked = live.clone();
        blocked.fanouts.set_no_energy_gain(true);
        gain(&mut blocked, 2);
        assert_eq!(blocked.energy, 1, "NoEnergyGain zeroes the owner's gain");

        let mut ending = live.clone();
        for monster in ending.monsters_mut() {
            monster.hp = 0;
        }
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        gain(&mut ending, 2);
        assert_eq!(ending.energy, 1, "IsEnding before the over latch");

        gain(&mut live, 2);
        assert_eq!(live.energy, 3);
    }

    #[test]
    fn draw_moves_cards_and_counts_the_command_form() {
        let (mut state, catalog) = ctx_state();
        let atom = catalog
            .atom(&CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((0..3).map(|uid| crate::hot::HotCard {
                uid,
                atom,
                flags: 0,
            }));
        run(
            StepKind::Draw,
            &mut state,
            &catalog,
            None,
            0,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
        assert_eq!(state.cards_drawn_combat, 2);
        assert_eq!(state.history.non_hand_draws_this_turn, 2);
    }

    fn run_attack_unit_card(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        run_attack_unit_card_with_x(kind, id, upgrade, state, target, 0, args)
    }

    fn run_attack_unit_card_with_x(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        target: Option<usize>,
        x_value: i64,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(crate::hot::HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target,
            selection: None,
            x_value,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    #[test]
    fn attack_unit_shared_bodies_pin_random_result_context_and_temp_shapes() {
        let mut random = HotState::at_defaults();
        random.hp = 50;
        random
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let before = random.rng.get(crate::hot::RngStream::Targets).counter;
        run_attack_unit_card(
            StepKind::AttackRandom,
            CardId::SwordBoomerang,
            0,
            &mut random,
            None,
            &[CompiledArg::I(3), CompiledArg::I(3)],
        )
        .unwrap();
        assert_eq!(random.monsters[0].hp, 91);
        assert_eq!(
            random.rng.get(crate::hot::RngStream::Targets).counter,
            before + 3
        );

        let mut result = HotState::at_defaults();
        result.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.block = 2;
        result.monsters_mut().push(target);
        run_attack_unit_card(
            StepKind::AttackResult,
            CardId::BlightStrike,
            0,
            &mut result,
            Some(0),
            &[
                CompiledArg::I(8),
                CompiledArg::Word(StepWord::DoomTotal),
                CompiledArg::I(0),
            ],
        )
        .unwrap();
        assert_eq!(result.monsters[0].powers.value(PowerId::Doom), 8);

        let mut context = HotState::at_defaults();
        context.hp = 50;
        let mut original = HotMonster::new(MonsterKind::Toadpole, 100);
        original.block = 3;
        context.monsters_mut().push(original);
        context
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        run_attack_unit_card(
            StepKind::AttackContextResultExact,
            CardId::Omnislice,
            0,
            &mut context,
            Some(0),
            &[CompiledArg::Word(StepWord::Omnislice), CompiledArg::I(8)],
        )
        .unwrap();
        assert_eq!((context.monsters[0].hp, context.monsters[1].hp), (95, 92));

        let mut temporary = HotState::at_defaults();
        run_attack_unit_card(
            StepKind::TempStrength,
            CardId::FeedingFrenzy,
            0,
            &mut temporary,
            None,
            &[CompiledArg::I(5)],
        )
        .unwrap();
        assert_eq!(temporary.temp_strength, 5);

        let mut setup = HotState::at_defaults();
        run_attack_unit_card(
            StepKind::TempStrength,
            CardId::SetupStrike,
            0,
            &mut setup,
            Some(0),
            &[CompiledArg::I(3)],
        )
        .unwrap();
        assert_eq!(setup.temp_strength, 3);

        let mut upgraded_feeding = HotState::at_defaults();
        run_attack_unit_card(
            StepKind::TempStrength,
            CardId::FeedingFrenzy,
            1,
            &mut upgraded_feeding,
            None,
            &[CompiledArg::I(7)],
        )
        .unwrap();
        assert_eq!(upgraded_feeding.temp_strength, 7);

        let mut upgraded_setup = HotState::at_defaults();
        run_attack_unit_card(
            StepKind::TempStrength,
            CardId::SetupStrike,
            1,
            &mut upgraded_setup,
            Some(0),
            &[CompiledArg::I(4)],
        )
        .unwrap();
        assert_eq!(upgraded_setup.temp_strength, 4);
    }

    #[test]
    fn summon_census_creates_grows_and_refuses_overflow_atomically() {
        let cases = [
            (CardId::Afterlife, 0, 6),
            (CardId::Afterlife, 1, 9),
            (CardId::Bodyguard, 0, 5),
            (CardId::Bodyguard, 1, 7),
            (CardId::Cleanse, 0, 3),
            (CardId::Cleanse, 1, 5),
            (CardId::NecroMastery, 0, 5),
            (CardId::NecroMastery, 1, 8),
            (CardId::PullAggro, 0, 4),
            (CardId::PullAggro, 1, 5),
            (CardId::Reanimate, 0, 20),
            (CardId::Reanimate, 1, 25),
        ];
        for (id, upgrade, amount) in cases {
            let mut builder = crate::catalog::CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let draw_atom = builder
                .intern(CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 9;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            if id == CardId::Cleanse {
                state.piles.get_mut(PileId::Draw).make_mut().extend([
                    HotCard {
                        uid: 8,
                        atom: draw_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 9,
                        atom: draw_atom,
                        flags: 0,
                    },
                ]);
            }
            crate::engine::play::play_card(&mut state, &catalog, 7, None, None, &mut Vec::new())
                .unwrap();
            let osty = state.fanouts.pet().osty().unwrap();
            assert_eq!((osty.hp(), osty.max_hp()), (amount, amount));
            if id == CardId::Cleanse {
                assert!(
                    state.pending.is_some(),
                    "Cleanse parks after publishing Summon"
                );
            }
            if id == CardId::PullAggro {
                let expected_block = if upgrade == 0 { 7 } else { 9 };
                assert_eq!(
                    state.block, expected_block,
                    "Pull Aggro runs Block after Summon"
                );
            }
            if id == CardId::NecroMastery {
                assert_eq!(state.powers.value(PowerId::NecroMastery), 1);
            }
        }

        // A live enemy keeps the combat from ending, so the summon's Heal
        // runs; with no live enemy left the IsEnding projection holds and the
        // re-summon raises MaxHp only (#3246).
        for (live_enemy, expected) in [(true, (7, 9)), (false, (2, 9))] {
            let mut grown = HotState::at_defaults();
            if live_enemy {
                grown
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
            }
            grown.fanouts.set_osty(Some((2, 4))).unwrap();
            run_attack_unit_card(
                StepKind::Summon,
                CardId::NecroMastery,
                0,
                &mut grown,
                None,
                &[CompiledArg::Word(StepWord::Osty), CompiledArg::I(5)],
            )
            .unwrap();
            let osty = grown.fanouts.pet().osty().unwrap();
            assert_eq!((osty.hp(), osty.max_hp()), expected);
        }

        // #3279: the step no longer returns early at `history.over`. A
        // pre-OnPlay listener that ended the combat still reaches the
        // carrier's first-command `OstyCmd.Summon` natively, under
        // IsEnding: a live Osty grows MaxHp only, an absent one refuses.
        for (osty, expected) in [
            (Some((2, 4)), Ok(Some((2, 9)))),
            (
                None,
                Err(EngineRefusal::EndingSummonNotModeled("Osty summon")),
            ),
        ] {
            let mut over = HotState::at_defaults();
            over.history.over = true;
            over.fanouts.set_osty(osty).unwrap();
            let result = run_attack_unit_card(
                StepKind::Summon,
                CardId::NecroMastery,
                0,
                &mut over,
                None,
                &[CompiledArg::Word(StepWord::Osty), CompiledArg::I(5)],
            )
            .map(|()| {
                over.fanouts
                    .pet()
                    .osty()
                    .map(|osty| (osty.hp(), osty.max_hp()))
            });
            assert_eq!(result, expected);
        }

        let mut overflowed = HotState::at_defaults();
        overflowed
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        overflowed
            .fanouts
            .set_osty(Some((i32::MAX, i32::MAX)))
            .unwrap();
        let before = overflowed.clone();
        assert_eq!(
            run_attack_unit_card(
                StepKind::Summon,
                CardId::NecroMastery,
                0,
                &mut overflowed,
                None,
                &[CompiledArg::Word(StepWord::Osty), CompiledArg::I(5),],
            ),
            Err(EngineRefusal::CounterOverflow("Osty summon"))
        );
        assert_eq!(overflowed.fanouts.pet(), before.fanouts.pet());
    }

    #[test]
    fn necro_mastery_power_replays_are_exact_for_echo_and_signal_boost() {
        let run = |signal_boost: i32| {
            let mut builder = crate::catalog::CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::NecroMastery,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 9;
            state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
            state
                .powers
                .set(PowerId::SignalBoost, SlotWire::Int, signal_boost);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            crate::engine::play::play_card(&mut state, &catalog, 7, None, None, &mut Vec::new())
                .unwrap();
            state
        };

        let echo_only = run(0);
        let osty = echo_only.fanouts.pet().osty().unwrap();
        assert_eq!((osty.hp(), osty.max_hp()), (16, 16));
        assert_eq!(echo_only.powers.value(PowerId::NecroMastery), 2);

        let signal_and_echo = run(1);
        let osty = signal_and_echo.fanouts.pet().osty().unwrap();
        assert_eq!((osty.hp(), osty.max_hp()), (24, 24));
        assert_eq!(signal_and_echo.powers.value(PowerId::NecroMastery), 3);
        assert_eq!(signal_and_echo.powers.value(PowerId::SignalBoost), 0);
    }

    #[test]
    fn attack_random_x_runs_exact_volley_rows_and_zero_x_command() {
        for (upgrade, damage) in [(0, 10), (1, 14)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let targets_before = state.rng.get(crate::hot::RngStream::Targets).counter;
            run_attack_unit_card_with_x(
                StepKind::AttackRandomX,
                CardId::Volley,
                upgrade,
                &mut state,
                None,
                3,
                &[CompiledArg::I(damage)],
            )
            .unwrap();
            assert_eq!(
                state.monsters.iter().map(|monster| monster.hp).sum::<i32>(),
                200 - 3 * i32::try_from(damage).unwrap()
            );
            assert_eq!(
                state.rng.get(crate::hot::RngStream::Targets).counter,
                targets_before + 3
            );
        }

        let mut zero = HotState::at_defaults();
        zero.hp = 50;
        zero.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        zero.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let targets_before = zero.rng.get(crate::hot::RngStream::Targets).counter;
        run_attack_unit_card_with_x(
            StepKind::AttackRandomX,
            CardId::Volley,
            0,
            &mut zero,
            None,
            0,
            &[CompiledArg::I(10)],
        )
        .unwrap();
        assert_eq!(zero.monsters[0].hp, 100);
        assert_eq!(zero.powers.value(PowerId::Vigor), 0);
        assert_eq!(
            zero.rng.get(crate::hot::RngStream::Targets).counter,
            targets_before
        );

        let mut terminal = HotState::at_defaults();
        terminal.hp = 50;
        terminal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 5));
        let targets_before = terminal.rng.get(crate::hot::RngStream::Targets).counter;
        run_attack_unit_card_with_x(
            StepKind::AttackRandomX,
            CardId::Volley,
            0,
            &mut terminal,
            None,
            3,
            &[CompiledArg::I(10)],
        )
        .unwrap();
        assert!(terminal.history.over);
        assert_eq!(
            terminal.rng.get(crate::hot::RngStream::Targets).counter,
            targets_before + 1,
            "terminal first hit suppresses both later live-pool draws"
        );
    }

    #[test]
    fn attack_random_x_accepts_stardust_and_refuses_targets_negative_x_and_bad_operands_atomically()
    {
        for (upgrade, damage) in [(0, 5), (1, 7)] {
            let identity = CardIdentity {
                id: CardId::Stardust,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 2,
                args: &[CompiledArg::I(damage)],
                events: &mut events,
            };
            attack_random_x(&mut ctx).unwrap();
            assert_eq!(ctx.state.monsters[0].hp, 100 - 2 * damage as i32);
            assert_eq!(ctx.state.rng.get(RngStream::Targets).counter, 2);
        }

        for (id, upgrade, x_value, target, args) in [
            (CardId::Stardust, 0, 2, None, vec![CompiledArg::I(6)]),
            (CardId::Volley, 0, 2, None, vec![CompiledArg::I(11)]),
            (CardId::Volley, 0, -1, None, vec![CompiledArg::I(10)]),
            (CardId::Volley, 0, 2, Some(0), vec![CompiledArg::I(10)]),
            (CardId::Stardust, 1, 2, Some(0), vec![CompiledArg::I(7)]),
        ] {
            let identity = CardIdentity {
                id,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target,
                selection: None,
                x_value,
                args: &args,
                events: &mut events,
            };
            assert_eq!(
                attack_random_x(&mut ctx),
                Err(EngineRefusal::MalformedArgs("attack_random_x"))
            );
            assert_eq!(*ctx.state, before);
            assert!(ctx.events.is_empty());
        }
    }

    #[test]
    fn temporary_strength_runs_lamp_and_artifact_once_on_the_outer_wrapper() {
        let mut artifact = HotState::at_defaults();
        artifact.hp = 50;
        artifact
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        artifact.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        apply_temp_strength_enemy(
            &mut artifact,
            0,
            crate::hot::AttachedPowerModel::PiercingWail,
            6,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(artifact.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(artifact.monsters[0].powers.value(PowerId::TempStrength), 0);
        assert_eq!(artifact.monsters[0].powers.value(PowerId::Strength), 0);

        let mut lamp = HotState::at_defaults();
        lamp.hp = 50;
        lamp.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        lamp.fanouts.set_unsettling_lamp_available(true);
        crate::engine::play::with_test_active_play(991, || {
            apply_temp_strength_enemy(
                &mut lamp,
                0,
                crate::hot::AttachedPowerModel::PiercingWail,
                6,
                &mut Vec::new(),
            )
            .unwrap();
        });
        assert_eq!(lamp.monsters[0].powers.value(PowerId::TempStrength), -12);
        assert_eq!(lamp.monsters[0].powers.value(PowerId::Strength), -12);
        assert!(
            lamp.monsters[0]
                .misery_debuff_order
                .attachments()
                .next()
                .is_none(),
            "a Misery-free fight records no provenance at all",
        );
    }

    /// #2693 S2 — two DISTINCT wrapper models are two applications, so a
    /// recipient holding two Artifacts spends both, one per application.
    ///
    /// Each class is its own singleton (`PowerCmd::FindExistingInstanceFor
    /// Stacking` `0x1338d8` IL_0058 looks up by model id), so both reach
    /// `Hook::ModifyPowerAmountReceived` at `PowerCmd/<Apply>d__2::MoveNext`
    /// `0x3efbac` IL_0248 in their own right. An aggregate signed scalar
    /// cannot distinguish this from one application of the sum, which would
    /// spend one Artifact.
    #[test]
    fn two_distinct_temporary_strength_wrappers_spend_two_artifacts() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 50);
        monster.uid = 3;
        monster.powers.set(PowerId::Artifact, SlotWire::Int, 2);
        state.monsters_mut().push(monster);
        state.fanouts.set_misery_attachment_upkeep(true);

        for model in [
            crate::hot::AttachedPowerModel::Mangle,
            crate::hot::AttachedPowerModel::DarkShackles,
        ] {
            apply_temp_strength_enemy(&mut state, 0, model, 9, &mut Vec::new()).unwrap();
        }
        assert_eq!(state.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 0);
        assert!(
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .next()
                .is_none(),
            "a blocked wrapper never attaches: `ApplyInternal` `0x84012` \
             IL_0001-IL_000e returns on the zeroed amount",
        );

        // The control: the same pair with no Artifact leaves two rows, in
        // application order, behind the Strength the first of them created.
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 50);
        monster.uid = 3;
        state.monsters_mut().push(monster);
        state.fanouts.set_misery_attachment_upkeep(true);
        for model in [
            crate::hot::AttachedPowerModel::Mangle,
            crate::hot::AttachedPowerModel::DarkShackles,
        ] {
            apply_temp_strength_enemy(&mut state, 0, model, 9, &mut Vec::new()).unwrap();
        }
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -18);
        assert_eq!(
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .map(|record| (record.power, record.amount, record.applier))
                .collect::<Vec<_>>(),
            vec![
                (
                    crate::hot::AttachedPowerModel::Strength,
                    -18,
                    crate::hot::Applier::Player
                ),
                (
                    crate::hot::AttachedPowerModel::Mangle,
                    9,
                    crate::hot::Applier::Player
                ),
                (
                    crate::hot::AttachedPowerModel::DarkShackles,
                    9,
                    crate::hot::Applier::Player
                ),
            ],
        );
    }

    #[test]
    fn temporary_strength_carrier_metadata_and_operands_are_complete() {
        let actual = CARD_ROWS
            .iter()
            .flat_map(|row| {
                row.steps
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| step.kind == StepKind::TempStrength)
                    .map(|(index, step)| {
                        (
                            row.id,
                            row.upgrade,
                            row.cost,
                            row.targeted,
                            row.target_type,
                            index,
                            step.args.to_vec(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                (
                    CardId::FeedingFrenzy,
                    0,
                    0,
                    false,
                    "Self",
                    0,
                    vec![Arg::I(5)],
                ),
                (
                    CardId::FeedingFrenzy,
                    1,
                    0,
                    false,
                    "Self",
                    0,
                    vec![Arg::I(7)],
                ),
                (
                    CardId::SetupStrike,
                    0,
                    1,
                    true,
                    "AnyEnemy",
                    1,
                    vec![Arg::I(3)],
                ),
                (
                    CardId::SetupStrike,
                    1,
                    1,
                    true,
                    "AnyEnemy",
                    1,
                    vec![Arg::I(4)],
                ),
            ],
            "all current TempStrength carriers, positions, and operands"
        );
    }

    #[test]
    fn fixed_status_sources_generate_the_exact_leaf_count_and_destination() {
        for (source, leaf, destination, count) in [
            (CardId::BoostAway, CardId::Dazed, PileId::Discard, 1usize),
            (CardId::CollisionCourse, CardId::Debris, PileId::Hand, 1),
            (CardId::FightThrough, CardId::Wound, PileId::Discard, 2),
            (CardId::Overclock, CardId::Burn, PileId::Discard, 1),
            (CardId::Turbo, CardId::Void, PileId::Discard, 1),
        ] {
            let identity = CardIdentity {
                id: source,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 20;
            state.next_card_uid = 2;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                1,
                spec.targeted.then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();

            let generated = state
                .piles
                .get(destination)
                .as_slice()
                .iter()
                .filter(|card| {
                    matches!(
                        catalog.spec(card.atom).unwrap().identity.id,
                        candidate if candidate == leaf
                    )
                })
                .count();
            assert_eq!(
                generated,
                count,
                "{} generated {}",
                source.as_str(),
                leaf.as_str()
            );
            assert_eq!(state.history.owner_generated_cards_combat, count as i32);
            assert_eq!(state.next_generated_hook_uid, count as i32);
            assert_eq!(state.next_card_uid, 2 + count as u32);
        }
    }

    #[test]
    fn lethal_collision_course_records_its_status_before_the_ending_insert_gate() {
        let identity = CardIdentity {
            id: CardId::CollisionCourse,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 20;
        state.next_card_uid = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        crate::engine::play::play_card(&mut state, &catalog, 1, Some(0), None, &mut Vec::new())
            .unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.next_generated_hook_uid, 1);
        assert_eq!(state.next_card_uid, 2);
        assert!(PileId::ALL.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| catalog.spec(card.atom).unwrap().identity.id != CardId::Debris)
        }));
    }

    /// #3515: the owner and enemy Strength writers behind the `strength`,
    /// `power_all_serial` and temporary-Strength-wrapper bodies still read
    /// `history.over`, so each body tests the shared IsEnding projection
    /// itself (`PowerCmd/<Apply>d__1`1` 0x3ef988 IL_0025). Inflame's owner
    /// Strength, Resonance's enemy Strength and Piercing Wail's wrapper on a
    /// live Gas Bomb are no-ops while the combat is ending before the over
    /// latch; the Adaptable-vetoed control writes each.
    #[test]
    fn strength_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        let mut builder = CatalogBuilder::new();
        let inflame = builder
            .intern(CardIdentity {
                id: CardId::Inflame,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let resonance = builder
            .intern(CardIdentity {
                id: CardId::Resonance,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let inflame = *catalog.spec(inflame).unwrap();
        let resonance = *catalog.spec(resonance).unwrap();
        let mut template = HotState::at_defaults();
        template.hp = 50;
        crate::engine::damage::assert_ending_window_gate(&template, "Inflame", |s, _| {
            strength(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &inflame,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(2)],
                events: &mut Vec::new(),
            })
        });
        crate::engine::damage::assert_ending_window_gate(&template, "Resonance", |s, _| {
            power_all_serial(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &resonance,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &[
                    CompiledArg::Power(PowerId::StrengthEnemy),
                    CompiledArg::I(-1),
                ],
                events: &mut Vec::new(),
            })
        });
        crate::engine::damage::assert_ending_window_gate(&template, "Piercing Wail", |s, t| {
            apply_temp_strength_enemy(
                s,
                t,
                temp_strength_enemy_model(CardId::PiercingWail)?,
                6,
                &mut Vec::new(),
            )
        });
    }

    #[test]
    fn not_yet_heals_to_max_and_refuses_after_combat_without_mutation() {
        for (upgrade, amount) in [(0, 10), (1, 13)] {
            let identity = CardIdentity {
                id: CardId::NotYet,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 95;
            state.max_hp = 100;
            crate::engine::damage::assert_ending_window_gate(&state, "Not Yet", |s, _| {
                heal(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 1,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: catalog.args(step.args),
                    events: &mut Vec::new(),
                })
            });
            let mut events = Vec::new();
            {
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 1,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: catalog.args(step.args),
                    events: &mut events,
                };
                heal(&mut ctx).unwrap();
            }
            assert_eq!(state.hp, 100, "Not Yet+{upgrade} clamps {amount} at max HP");

            state.history.over = true;
            let before = state.clone();
            {
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 1,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: catalog.args(step.args),
                    events: &mut events,
                };
                assert_eq!(heal(&mut ctx), Err(EngineRefusal::CombatOver));
            }
            assert_eq!(state, before);
        }
    }

    /// #3044: Not Yet's heal is a `CreatureCmd.Heal`, and its
    /// `AfterCurrentHpChanged` reaches Red Skull.
    #[test]
    fn not_yet_heal_past_half_removes_red_skulls_strength() {
        let identity = CardIdentity {
            id: CardId::NotYet,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let step = catalog.steps(&spec)[0];
        for (hp, after, strength) in [(35, 45, 0), (25, 35, 3)] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = hp;
            state.max_hp = 80;
            state.fanouts.set_red_skull_owned(true);
            state.powers.set(PowerId::Strength, SlotWire::Int, 3);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 1,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };
            heal(&mut ctx).unwrap();
            assert_eq!(
                (state.hp, state.powers.value(PowerId::Strength)),
                (after, strength)
            );
        }
    }

    /// #3044: Feed's fatal `GainMaxHp` is `SetMaxHp` then `Heal`
    /// (`0x3eb2f0` IL_005b, IL_010b): Red Skull compares the new HP with the
    /// new max HP.
    #[test]
    fn feeds_max_hp_reward_is_a_heal_red_skull_hears() {
        // 40/80 holds the Strength and is lifted past half; 30/80 is not.
        for (hp, strength) in [(40, 0), (30, 3)] {
            let (mut state, catalog, source) = fatal_fixture(
                CardId::Feed,
                0,
                vec![
                    HotMonster::new(MonsterKind::Toadpole, 1),
                    HotMonster::new(MonsterKind::Toadpole, 30),
                ],
                PileId::Draw,
            );
            state.hp = hp;
            state.max_hp = 80;
            state.fanouts.set_red_skull_owned(true);
            state.powers.set(PowerId::Strength, SlotWire::Int, 3);
            run_fatal_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
            assert!(state.monsters[0].hp <= 0 && !state.history.over);
            let gained = state.max_hp - 80;
            assert!(gained > 0);
            assert_eq!(state.hp, hp + gained);
            assert_eq!(state.powers.value(PowerId::Strength), strength, "{hp}/80");
        }
    }

    #[test]
    fn legacy_lightning_rod_then_spinner_materializes_both_reset_projections() {
        let identity = CardIdentity {
            id: CardId::Spinner,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 12));
        state.powers.set(PowerId::LightningRod, SlotWire::Int, 1);
        let mut events = Vec::new();
        let args = [CompiledArg::I(1)];
        spinner(&mut StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        })
        .unwrap();

        assert_eq!(
            state.orbs.reset_order(),
            &[OrbResetPower::LightningRod, OrbResetPower::Spinner]
        );
        assert_eq!(
            state.fanouts.after_energy_reset_order(),
            Some(
                &[
                    crate::hot::AfterEnergyResetPower::LightningRod,
                    crate::hot::AfterEnergyResetPower::Spinner,
                ][..]
            )
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let reloaded = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(reloaded, state);
    }
}
