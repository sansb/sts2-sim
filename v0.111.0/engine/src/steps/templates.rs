//! Card-step bodies used only by auto-translated `card_templates.json` rows — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1345 + #1394 + #1406 + #1487 + #1561 + #1751 + #1884 + #2031): 96 of 101 ported
//!
//! All 101 stubs were re-read against their Python branches after engine slice
//! 5 (#1406). Later waves have reduced the refusing set to 5; each remaining
//! stub names the still-missing reader, continuation, resource, carrier, or
//! atomic application primitive that keeps it outside the honest surface.

use super::StepCtx;
use crate::catalog::{CardIdentity, Catalog, CompiledArg};
use crate::engine::damage::{apply_card_knockdown, apply_card_monster_debuff};
use crate::engine::{EngineRefusal, Event, Subject};
use crate::hot::{CARD_FLAG_LEGACY, HotState, MiseryToken, PileId};
use crate::ids::{CardId, MonsterKind, PowerId, StepKind, StepWord};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// Slice #1406 adds the exact scalar writers whose turn-boundary readers now
/// live in `engine::turn`; every body below pins its complete v0.111.0 source
/// row set so the generic mechanic cannot admit an unreviewed carrier.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::Accelerant,
    StepKind::Accuracy,
    StepKind::Aggression,
    StepKind::Afterimage,
    StepKind::Arsenal,
    StepKind::Automation,
    StepKind::Barricade,
    StepKind::BiasedCognition,
    StepKind::BlackHole,
    StepKind::Blur,
    StepKind::BorrowedTime,
    StepKind::Buffer,
    StepKind::Burst,
    StepKind::Cacophony,
    StepKind::ChildOfTheStars,
    StepKind::Colossus,
    StepKind::Coolant,
    StepKind::CorrosiveWave,
    StepKind::Corruption,
    StepKind::Countdown,
    StepKind::Cruelty,
    StepKind::DanseMacabre,
    StepKind::DarkEmbrace,
    StepKind::Debilitate,
    StepKind::Demesne,
    StepKind::DemonForm,
    StepKind::DevourLife,
    StepKind::Doom,
    StepKind::DrawNextTurn,
    StepKind::EchoForm,
    StepKind::Entropy,
    StepKind::Envenom,
    StepKind::FeelNoPain,
    StepKind::Fasten,
    StepKind::FlameBarrier,
    StepKind::Focus,
    StepKind::Foregone,
    StepKind::FreeAttack,
    StepKind::FreeEthereal,
    StepKind::FreePower,
    StepKind::FreeSkill,
    StepKind::Genesis,
    StepKind::Hailstorm,
    StepKind::HelloWorld,
    StepKind::Hellraiser,
    StepKind::InfiniteBlades,
    StepKind::Intangible,
    StepKind::Knockdown,
    StepKind::Iteration,
    StepKind::Juggernaut,
    StepKind::Loop,
    StepKind::MachineLearning,
    StepKind::MadScienceCurious,
    StepKind::Mayhem,
    StepKind::MonarchsGaze,
    StepKind::NecroMastery,
    StepKind::Neurosurge,
    StepKind::NoDraw,
    StepKind::NoxiousFumes,
    StepKind::Oblivion,
    StepKind::OneTwoPunch,
    StepKind::Orbit,
    StepKind::Pagestorm,
    StepKind::PaleBlueDot,
    StepKind::Panache,
    StepKind::PhantomBlades,
    StepKind::PillarOfCreation,
    StepKind::Plating,
    StepKind::Poison,
    StepKind::PrepTime,
    StepKind::Pyre,
    StepKind::Rage,
    StepKind::ReaperForm,
    StepKind::Reflect,
    StepKind::RetainHand,
    StepKind::RollingBoulder,
    StepKind::Rupture,
    StepKind::SealedThrone,
    StepKind::SentryMode,
    StepKind::SignalBoost,
    StepKind::Smokestack,
    StepKind::Sneaky,
    StepKind::Speedster,
    StepKind::SleightOfFlesh,
    StepKind::SpiritOfAsh,
    StepKind::Stampede,
    StepKind::StarNextTurn,
    StepKind::Strangle,
    StepKind::Stratagem,
    StepKind::StrengthEnemy,
    StepKind::Subroutine,
    StepKind::SummonNextTurn,
    StepKind::TempDexterity,
    StepKind::TempStrengthEnemy,
    StepKind::TheGambit,
    StepKind::Thorns,
    StepKind::Tracking,
    StepKind::TrashToTreasure,
    StepKind::Unmovable,
    StepKind::Vigor,
    StepKind::WraithForm,
];

#[derive(Copy, Clone)]
enum FanoutOrder {
    AfterBlockGained,
    AfterCardDrawn,
    AfterCardExhausted,
    AfterDamageGiven,
    AfterPowerAmountChanged,
    BeforeHandDraw,
    BeforeSideTurnEnd,
    StarEnergyReset,
    LocalGenerated,
}

/// Refuse a second live object of a native `PowerInstanceType.Instanced`
/// power that Rust carries as ONE scalar (#3057, the #3021 class).
///
/// v0.111.0 (DLL `9cb4f1ad`): `PowerCmd::FindExistingInstanceForStacking`
/// RVA `0x1338d8` switches on `InstanceType` at IL_0021 and answers `null`
/// for case 1 (IL_0034), so every `PowerCmd.Apply<T>` of such a power
/// attaches a NEW object with its own amount and private state instead of
/// stacking. The callers' own `get_InstanceType` overrides return 1 at
/// IL_0001: `CacophonyPower` `0xa014b`, `OrbitPower` `0xa5476`,
/// `RollingBoulderPower` `0xa6da1`. Each object then runs its own listener:
///
/// - Cacophony `<AfterCardDrawn>d__10::MoveNext` RVA `0x33669c` decrements
///   THIS object's `DynamicVars.Cards` (IL_0027-003f), and at <= 0 rolls its
///   own `CombatTargets` enemy (IL_0060-008b), resets its own Cards to 33
///   (IL_0090-00a2) and damages for its own `Amount` (IL_0115-0133). Two
///   objects are two countdowns, two rolls and two hits.
/// - Orbit `<AfterEnergySpent>d__15::MoveNext` RVA `0x33f718` adds the spend
///   to THIS object's `Data.energySpent` (IL_004b-006a), pays
///   `Amount * (energySpent / 4 - triggerCount)` (IL_006f-00bd) and advances
///   its own `triggerCount` (IL_0114-0127); `OrbitPower/Data` (`.ctor` RVA
///   `0x33f70e`) starts both at 0 for every new object.
/// - Rolling Boulder `<AfterPlayerTurnStart>d__8::MoveNext` RVA `0x342bdc`
///   damages `HittableEnemies` for THIS object's `Amount` (IL_0105-0142,
///   `DoDamage` RVA `0xa6e0b`) and then grows THIS object by its own
///   `DynamicVars.Damage` (5, IL_024a-0263). Two objects are two AoE hits
///   and two +5 growths.
///
/// None of these objects is representable beside the first one here, so the
/// application refuses by name instead of summing into the scalar. The gate
/// sits after the native IsEnding gate (`PowerCmd/<Apply>d__1`1::MoveNext`
/// RVA `0x3ef988` IL_0020-0034 returns before any object is built), and
/// malformed arguments fall through to the caller's own `MalformedArgs`.
fn refuse_second_instanced_object(
    ctx: &StepCtx<'_>,
    power: PowerId,
    allowed: &[(CardId, u8, i64)],
) -> Result<(), EngineRefusal> {
    let valid = matches!(
        ctx.args,
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value))
    );
    if valid
        && !crate::engine::damage::damage_combat_is_ending(ctx.state)
        && ctx.state.powers.value(power) > 0
    {
        return Err(EngineRefusal::PowerRestackNotModeled(power));
    }
    Ok(())
}

/// The ordered twin of [`apply_exact_player_power`]: the same native
/// `PowerCmd.Apply<T>` IsEnding gate (RVA `0x3ef988` IL_0020-0x0034) skips
/// the grant once combat is ending (#3183).
///
/// Audited callers (card `<OnPlay>` MoveNext RVA, then each `Apply<T>` call's
/// IL offset), v0.111.0 `sts2.dll` sha `9cb4f1ad…`:
/// Arsenal `0x38a508` Arsenal@IL_00d1; Automation `0x38aaa0`
/// Automation@IL_0050; Cacophony `0x390a9c` Cacophony@IL_0050; Convergence
/// `0x39469c` RetainHand@IL_00cd/EnergyNextTurn@IL_0156/StarNextTurn@IL_01e2;
/// CorrosiveWave `0x394da4` CorrosiveWave@IL_00d1; DarkEmbrace `0x3964fc`
/// DarkEmbrace@IL_00c1; Envenom `0x39bee4` Envenom@IL_00d1; FeelNoPain
/// `0x39dc08` FeelNoPain@IL_0050; ForegoneConclusion `0x39fe24`
/// ForegoneConclusion@IL_00cc; Genesis `0x3a0f94` Genesis@IL_00d1; Hailstorm
/// `0x3a32d4` Hailstorm@IL_00d1; HelloWorld `0x3a4bc8` HelloWorld@IL_00c1;
/// HiddenCache `0x3a5280` StarNextTurn@IL_0149; InfiniteBlades `0x3a744c`
/// InfiniteBlades@IL_00c1; Iteration `0x3a7d28` Iteration@IL_00d1; Juggernaut
/// `0x3a8374` Juggernaut@IL_00d1; MonarchsGaze `0x3add38`
/// MonarchsGaze@IL_00d1; Pagestorm `0x3b1268` Pagestorm@IL_00cc;
/// PillarOfCreation `0x3b2908` PillarOfCreation@IL_004b; ReaperForm
/// `0x3b5d9c` ReaperForm@IL_00c1; SleightOfFlesh `0x3bc130`
/// SleightOfFlesh@IL_00d1; Smokestack `0x3bc648` Smokestack@IL_00d1;
/// Speedster `0x3bdb2c` Speedster@IL_0055; TrashToTreasure `0x3c4648`
/// TrashToTreasure@IL_00c1
fn apply_exact_ordered_player_power(
    ctx: &mut StepCtx<'_>,
    site: &'static str,
    power: PowerId,
    allowed: &[(CardId, u8, i64)],
    order: FanoutOrder,
) -> Result<(), EngineRefusal> {
    let amount = match ctx.args {
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value)) =>
        {
            *value
        }
        _ => return Err(EngineRefusal::MalformedArgs(site)),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow(site))?;
    let old = ctx.state.powers.value(power);
    let updated = old
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    if old == 0 {
        let mut prepared = ctx.state.clone();
        let registered = match order {
            FanoutOrder::AfterBlockGained => prepared.fanouts.register_after_block_gained(power),
            FanoutOrder::AfterCardDrawn => prepared.fanouts.register_after_card_drawn(power),
            FanoutOrder::AfterCardExhausted => {
                prepared.fanouts.register_after_card_exhausted(power)
            }
            FanoutOrder::BeforeHandDraw => prepared.fanouts.register_before_hand_draw(power),
            FanoutOrder::AfterDamageGiven => prepared.fanouts.register_after_damage_given(power),
            FanoutOrder::AfterPowerAmountChanged => {
                prepared.fanouts.register_after_power_amount_changed(power)
            }
            FanoutOrder::BeforeSideTurnEnd => prepared.fanouts.register_before_side_turn_end(power),
            FanoutOrder::StarEnergyReset => prepared.fanouts.register_star_energy_reset(power),
            FanoutOrder::LocalGenerated => prepared.fanouts.register_local_generated_power(power),
        };
        if !registered {
            return Err(EngineRefusal::CounterOverflow("fanout listener order"));
        }
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            &mut prepared,
            power,
            old,
            updated,
        )?;
        *ctx.state = prepared;
    } else {
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            ctx.state, power, old, updated,
        )?;
    }
    ctx.state.powers.set(power, SlotWire::Int, updated);
    if matches!(order, FanoutOrder::StarEnergyReset) {
        ctx.state.normalize_after_energy_reset_order_if_unique();
    }
    crate::engine::damage::note_power(ctx.events, crate::engine::Subject::Player, power, updated);
    Ok(())
}

/// Every caller is a card `OnPlay` whose native body awaits the generic
/// `PowerCmd.Apply<T>(Creature, decimal, Creature, CardModel, bool)`
/// (MethodDef `0x0600560d`, stub RVA `0x1337f0`). Its state machine
/// ``PowerCmd/<Apply>d__1`1::MoveNext`` (RVA `0x3ef988`) returns null at
/// IL_0020-0x0034 while `CombatManager.IsEnding`, before it builds or stacks
/// the instance (the instance overload `PowerCmd/<Apply>d__2::MoveNext`, RVA
/// `0x3efbac`, repeats that gate at IL_0039-0x0045). So a card whose earlier
/// step ends combat (Predator's or Salvo's lethal attack, a Juggernaut kill
/// off Blur's block) never grants its trailing power (#3160, #3183). The live
/// projection of IsEnding is `engine::damage::damage_combat_is_ending`
/// (#2669). Arguments are validated first, so a malformed row still refuses.
///
/// Audited callers (card `<OnPlay>` MoveNext RVA, then each
/// `Apply<T>` call's IL offset), v0.111.0 `sts2.dll` sha `9cb4f1ad…`:
/// Abrasive `0x3886b4` Dexterity@IL_00d4/Thorns@IL_0162; Accelerant
/// `0x388b18` Accelerant@IL_00d1; Accuracy `0x388c9c` Accuracy@IL_00d1;
/// Afterimage `0x389354` Afterimage@IL_00d1; Aggression `0x389654`
/// Aggression@IL_00c1; Anticipate `0x389ff0` Anticipate@IL_00cc; Apparition
/// `0x38a170` Intangible@IL_00d1; Barricade `0x38b388` Barricade@IL_00c1;
/// BattleTrance `0x38b6bc` NoDraw@IL_00bf; BiasedCognition `0x38c428`
/// Focus@IL_00d9/BiasedCognition@IL_0167; BlackHole `0x38c92c`
/// BlackHole@IL_00d1; Blur `0x38d99c` Blur@IL_00cf; BorrowedTime `0x38e834`
/// BorrowedTime@IL_0149; Buffer `0x38f8bc` Buffer@IL_00d1; Burst `0x3906a8`
/// Burst@IL_00d1; Caltrops `0x391068` Thorns@IL_00d1; ChildOfTheStars
/// `0x391fd4` ChildOfTheStars@IL_00d1; Colossus `0x39318c` Colossus@IL_0151;
/// Convergence `0x39469c`
/// RetainHand@IL_00cd/EnergyNextTurn@IL_0156/StarNextTurn@IL_01e2; Coolant
/// `0x394934` Coolant@IL_00d1; Countdown `0x395324` Countdown@IL_00d1;
/// Cruelty `0x395b28` Cruelty@IL_00d1; DanseMacabre `0x396378`
/// DanseMacabre@IL_00d1; Defragment `0x39811c` Focus@IL_00d1; Demesne
/// `0x3986c0` Demesne@IL_00cc; DemonForm `0x398840` DemonForm@IL_00d1;
/// DevourLife `0x398e2c` DevourLife@IL_00d1; EchoForm `0x39af1c`
/// EchoForm@IL_00d1; Entropy `0x39bdec` Entropy@IL_0050; Equilibrium
/// `0x39c068` RetainHand@IL_00cf; EternalArmor `0x39c494` Plating@IL_0055;
/// Fasten `0x39d4d4` Fasten@IL_0050; FlameBarrier `0x39efb0`
/// FlameBarrier@IL_00ff; Glow `0x3a1e14` DrawCardsNextTurn@IL_01c3; Loop
/// `0x3a9ed4` Loop@IL_00d1; MachineLearning `0x3aa3b0`
/// MachineLearning@IL_00cc; Mayhem `0x3abe58` Mayhem@IL_0040; NecroMastery
/// `0x3ae420` NecroMastery@IL_0141; Neurosurge `0x3aeae8` Neurosurge@IL_01cd;
/// NeutronAegis `0x3aef7c` Plating@IL_0050; NoxiousFumes `0x3af838`
/// NoxiousFumes@IL_00d1; OneTwoPunch `0x3b0584` OneTwoPunch@IL_0050;
/// PaleBlueDot `0x3b13e8` PaleBlueDot@IL_00cc; Patter `0x3b1b94`
/// Vigor@IL_0156; PhantomBlades `0x3b2104` PhantomBlades@IL_00d1; Predator
/// `0x3b34c4` DrawCardsNextTurn@IL_00fe; PrepTime `0x3b386c`
/// PrepTime@IL_0050; Pyre `0x3b4af0` Pyre@IL_004b; Rage `0x3b51fc`
/// Rage@IL_00d1; Reflect `0x3b67b0` Reflect@IL_0141; Relax `0x3b6dd4`
/// DrawCardsNextTurn@IL_0150/EnergyNextTurn@IL_01dc; RollingBoulder
/// `0x3b7c6c` RollingBoulder@IL_0055; Rupture `0x3b8044` Rupture@IL_004b;
/// Salvo `0x3b8390` RetainHand@IL_00fd; SignalBoost `0x3bbd70`
/// SignalBoost@IL_00d1; Sneaky `0x3bcbac` Sneaky@IL_00d1; SpiritOfAsh
/// `0x3bde24` SpiritOfAsh@IL_00d1; Stampede `0x3bed40` Stampede@IL_0050;
/// StoneArmor `0x3bf43c` Plating@IL_0050; Stratagem `0x3bfac4`
/// Stratagem@IL_0040; Subroutine `0x3c014c` Subroutine@IL_00c1; Terraforming
/// `0x3c241c` Vigor@IL_0055; TheGambit `0x3c2a80` TheGambit@IL_00bf;
/// TheSealedThrone `0x3c3028` TheSealedThrone@IL_00c1; Unmovable `0x3c5878`
/// Unmovable@IL_00d2; WraithForm `0x3c79c4`
/// Intangible@IL_00d9/WraithForm@IL_0167
fn apply_exact_player_power(
    ctx: &mut StepCtx<'_>,
    site: &'static str,
    power: PowerId,
    allowed: &[(CardId, u8, i64)],
) -> Result<(), EngineRefusal> {
    let amount = match ctx.args {
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value)) =>
        {
            *value
        }
        _ => return Err(EngineRefusal::MalformedArgs(site)),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    if amount == 0 {
        return Ok(());
    }
    if matches!(
        power,
        PowerId::DemonForm
            | PowerId::PrepTime
            | PowerId::FlameBarrier
            | PowerId::Reflect
            | PowerId::TheGambit
    ) {
        // Both are Type-1 PowerCmd applications. The represented amount-
        // changed listeners are inert for these player-owned power changes,
        // but their complete native acquisition order remains authority.
        if !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state) {
            return Err(EngineRefusal::MalformedArgs(
                "Clarity peer AfterPowerAmountChanged order",
            ));
        }
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow(site))?;
    let old = ctx.state.powers.value(power);
    let updated = old
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    let mut prepared = ctx.state.clone();
    if old <= 0 {
        if crate::hot::AfterSideTurnStartToken::from_power(power).is_some()
            && !prepared.fanouts.register_after_side_turn_start(power)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-side-turn-start listener order",
            ));
        }
        if matches!(
            power,
            PowerId::FlameBarrier | PowerId::Reflect | PowerId::TheGambit
        ) && !prepared.register_after_damage_received_power(power)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-damage-received listener order",
            ));
        }
        if matches!(
            power,
            PowerId::Loop | PowerId::RollingBoulder | PowerId::Entropy
        ) && !prepared.register_after_player_turn_start(power)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-player-turn-start listener order",
            ));
        }
    }
    crate::engine::play::prepare_after_card_played_scalar_write(
        &mut prepared,
        power,
        old,
        updated,
    )?;
    *ctx.state = prepared;
    ctx.state.powers.set(power, SlotWire::Int, updated);
    crate::engine::damage::note_power(ctx.events, crate::engine::Subject::Player, power, updated);
    Ok(())
}

/// `("accelerant", n)` — stack `n` onto the owner's AccelerantPower.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Accelerant.OnPlay` state machine RVA `0x388b18` applies the dynamic
/// base value as `AccelerantPower`; L0/L1 are one/two. `PoisonPower::
/// get_TriggerCount` RVA `0xa5d60` computes `Min(Amount, 1 + Sum(alive
/// opponents' AccelerantPower))` once before the trigger loop.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) is the additive writer;
/// `_finish_side_switch_after_disintegration` is the ordinary
/// side-start poison reader. The Rust reader uses the represented single
/// opposing Player scalar and a widened checked bound.
pub(crate) fn accelerant(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "accelerant",
        PowerId::Accelerant,
        &[(CardId::Accelerant, 0, 1), (CardId::Accelerant, 1, 2)],
    )
}

/// `("accuracy", n)` — permanent additive Shiv damage bonus.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
pub(crate) fn accuracy(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "accuracy",
        PowerId::Accuracy,
        &[(CardId::Accuracy, 0, 4), (CardId::Accuracy, 1, 6)],
    )
}

/// `("afterimage", n)` — flat block on each later owned card play.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive stack; the casting play's
/// own BeforeCardPlayed latch was taken before the amount landed, so the first
/// grant is the next play.
/// [`crate::engine::play`] freezes the pre-body amount and applies it in the
/// native ordinary player-power group after the play finishes.
pub(crate) fn afterimage(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "afterimage",
        PowerId::Afterimage,
        &[(CardId::Afterimage, 0, 1), (CardId::Afterimage, 1, 1)],
    )
}

/// `("aggression", n)` — stack `n` onto the owner's AggressionPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. `engine::turn` owns the full Selection-stream
/// shuffle, physical move, upgrade closure, and exact-pile promotion reader.
pub(crate) fn aggression(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "aggression",
        PowerId::Aggression,
        &[(CardId::Aggression, 0, 1), (CardId::Aggression, 1, 1)],
    )
}

/// `("arsenal", n)` — ArsenalPower, a generated-card listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `local_generated_power_order` on the zero-to-positive edge
/// (`_validated_soulbound_state`), then stacks.
///
pub(crate) fn arsenal(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "arsenal",
        PowerId::Arsenal,
        &[(CardId::Arsenal, 0, 1), (CardId::Arsenal, 1, 1)],
        FanoutOrder::LocalGenerated,
    )
}

/// `("automation", n)` — AutomationPower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`: order registration on the zero-to-positive
/// edge, then the additive stack.
/// [`crate::engine::draw`] walks the acquisition order and carries the private
/// ten-card countdown through canonical state.
///
/// AutomationPower is Instanced (#3021): v0.111.0 `AutomationPower::
/// get_InstanceType` RVA `0x9fa0e` IL_0001 returns 1, and
/// `PowerCmd::FindExistingInstanceForStacking` RVA `0x1338d8` IL_0021-0036
/// returns `null` for it, so a play while Automation is live attaches a NEW
/// object whose `Data.cardsLeft` starts at 10 (`AutomationPower/Data::.ctor`
/// RVA `0x33558e` IL_0002-0004) and whose listener is appended at the end of
/// the owner's power list. That object is recorded as a
/// [`crate::hot::AutomationInstance`] row. The draw walk runs every
/// Automation object at the first one's position, which is native exactly
/// when no other `AfterCardDrawn` listener was acquired between them; a
/// later object acquired behind a foreign draw listener refuses by name.
pub(crate) fn automation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let allowed = [(CardId::Automation, 0, 1), (CardId::Automation, 1, 1)];
    if ctx.state.powers.value(PowerId::Automation) <= 0 {
        return apply_exact_ordered_player_power(
            ctx,
            "automation",
            PowerId::Automation,
            &allowed,
            FanoutOrder::AfterCardDrawn,
        );
    }
    let amount = match ctx.args {
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value)) =>
        {
            i32::try_from(*value).map_err(|_| EngineRefusal::CounterOverflow("automation"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("automation")),
    };
    // `Automation/<OnPlay>d__5::MoveNext` (RVA `0x38aaa0`) reaches the new
    // instance only through `PowerCmd.Apply<AutomationPower>` (IL_0050), whose
    // IsEnding gate (RVA `0x3ef988` IL_0020-0x0034) returns before the
    // instance is built: no later-instance row while combat is ending.
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    if ctx.state.fanouts.after_card_drawn_order().last() != Some(&PowerId::Automation) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "automation instance behind a later draw listener",
        ));
    }
    let mut later = ctx.state.fanouts.automation_later_instances().to_vec();
    later.push(crate::hot::AutomationInstance {
        amount,
        cards_left: 10,
    });
    let original = ctx.state.clone();
    if !ctx.state.fanouts.set_automation_later_instances(&later) {
        return Err(EngineRefusal::CounterOverflow("automation"));
    }
    let applied = apply_exact_ordered_player_power(
        ctx,
        "automation",
        PowerId::Automation,
        &allowed,
        FanoutOrder::AfterCardDrawn,
    );
    if applied.is_err() {
        *ctx.state = original;
    }
    applied
}

/// `("barricade", n)` — block stops clearing at the owner's turn start.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. The turn-start clear now reads the positive
/// marker before the later Blur/Plating listener suffix.
pub(crate) fn barricade(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "barricade",
        PowerId::Barricade,
        &[(CardId::Barricade, 0, 1), (CardId::Barricade, 1, 1)],
    )
}

/// `("biased_cognition", 1)` — the persistent Focus degradation.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a second separately-awaited application
/// after the card's preceding Focus step; plain additive stack.
///
/// The turn-start reader lives in `engine::turn`; this body pins the only two
/// v0.111.0 source rows so another template cannot inherit the persistent
/// listener without its own oracle review.
pub(crate) fn biased_cognition(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "biased_cognition",
        PowerId::BiasedCognition,
        &[
            (CardId::BiasedCognition, 0, 1),
            (CardId::BiasedCognition, 1, 1),
        ],
    )
}

/// `("black_hole", n)` — additive BlackHolePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// `engine::play` shares `_black_hole_damage` (frozen Python, deleted #2827) between
/// positive GainStars and the final CardPlay of a positive Star-spending series.
pub(crate) fn black_hole(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "black_hole",
        PowerId::BlackHole,
        &[(CardId::BlackHole, 0, 3), (CardId::BlackHole, 1, 4)],
    )
}

/// `("blur", n)` — block retention across the owner's turn start.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Positive Blur suppresses this turn's block clear,
/// then decrements in the later AfterSideTurnStart group.
pub(crate) fn blur(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "blur",
        PowerId::Blur,
        &[(CardId::Blur, 0, 1), (CardId::Blur, 1, 1)],
    )
}

/// `("borrowed_time", n)` — an additive term in later cost queries.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the card body
/// gains its Energy before this step, so only later cost queries see the new
/// amount.
///
/// `engine::play` reads the amount in the early global energy-cost fold after
/// local modifiers and Tangled, before every absolute-zero late listener.
///
/// #3427: refuses while a Curious is live, the other half of
/// [`mad_science_curious`]'s listener-order refusal.
pub(crate) fn borrowed_time(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !crate::engine::damage::damage_combat_is_ending(ctx.state)
        && ctx.state.powers.value(PowerId::Curious) > 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Borrowed Time beside Curious energy-cost listener order",
        ));
    }
    apply_exact_player_power(
        ctx,
        "borrowed_time",
        PowerId::BorrowedTime,
        &[(CardId::BorrowedTime, 0, 1), (CardId::BorrowedTime, 1, 1)],
    )
}

/// `("buffer", n)` — negate the next `n` positive HP-loss results.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage`] runs the native `ModifyHpLost` suffix after block:
/// a fully blocked or zero result consumes nothing, while each positive result
/// consumes one stack and becomes zero.
pub(crate) fn buffer(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "buffer",
        PowerId::Buffer,
        &[(CardId::Buffer, 0, 1), (CardId::Buffer, 1, 2)],
    )
}

/// `("burst", n)` — the next `n` Skills are played twice.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; casting BURST is
/// itself a Skill play, so an already-active stack doubles the cast (the play
/// count was generated before OnPlay).
///
/// GeneratePlayCount adds one replay and consumes one stack before the
/// synchronous body series begins.
pub(crate) fn burst(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "burst",
        PowerId::Burst,
        &[(CardId::Burst, 0, 1), (CardId::Burst, 1, 2)],
    )
}

/// `("cacophony", n)` — CacophonyPower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`; the Cards countdown is NOT re-seeded on re-
/// apply.
/// [`crate::engine::draw`] carries the private 33-card countdown and monotone
/// reset generation, consuming one CombatTargets roll only when a live pool
/// exists at a reset.
///
/// A second live CacophonyPower object refuses by name
/// ([`refuse_second_instanced_object`], #3057): native gives it its own
/// countdown, target roll and hit.
pub(crate) fn cacophony(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    const ALLOWED: &[(CardId, u8, i64)] = &[(CardId::Cacophony, 0, 66), (CardId::Cacophony, 1, 99)];
    refuse_second_instanced_object(ctx, PowerId::Cacophony, ALLOWED)?;
    apply_exact_ordered_player_power(
        ctx,
        "cacophony",
        PowerId::Cacophony,
        ALLOWED,
        FanoutOrder::AfterCardDrawn,
    )
}

/// `("child_of_the_stars", n)` — additive ChildOfTheStarsPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// `engine::play::spend_stars` multiplies this amount by the positive requested
/// spend before the later relic group (Python `spend_stars` (frozen, deleted #2827)).
pub(crate) fn child_of_the_stars(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "child_of_the_stars",
        PowerId::ChildOfTheStars,
        &[
            (CardId::ChildOfTheStars, 0, 2),
            (CardId::ChildOfTheStars, 1, 3),
        ],
    )
}

/// `("colossus", n)` — halve vulnerable-source attack damage for `n` turns.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — applications add duration stacks; the
/// 0.5 multiplier is the fixed DamageDecrease BaseValue, not the power Amount.
///
/// [`crate::engine::damage::monster_attack_player`] folds the fixed 0.5 term
/// after the player's Vulnerable multiplier when the dealer has Vulnerable;
/// [`crate::engine::turn`] removes one duration at the enemy-side end.
pub(crate) fn colossus(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "colossus",
        PowerId::Colossus,
        &[(CardId::Colossus, 0, 1), (CardId::Colossus, 1, 1)],
    )
}

/// `("coolant", n)` — CoolantPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its turn-start reader counts distinct live orb
/// kinds and grants flat block after the draw.
pub(crate) fn coolant(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "coolant",
        PowerId::Coolant,
        &[(CardId::Coolant, 0, 2), (CardId::Coolant, 1, 3)],
    )
}

/// `("corrosive_wave", n)` — CorrosiveWavePower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`.
/// [`crate::engine::draw`] applies the amount as Poison to each live roster
/// member in the frozen acquisition-order walk.
pub(crate) fn corrosive_wave(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "corrosive_wave",
        PowerId::CorrosiveWave,
        &[(CardId::CorrosiveWave, 0, 2), (CardId::CorrosiveWave, 1, 3)],
        FanoutOrder::AfterCardDrawn,
    )
}

/// `("corruption", 1)` — Skills cost zero and exhaust.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — StackType 2 Unique: a boolean latch plus
/// a `result_location_power_order` registration; a reapplication neither adds a
/// listener nor moves the existing one.
///
/// `engine::play` owns both readers: the late absolute-zero Skill cost term and
/// the acquisition-ordered result-location walk that routes Skills to Exhaust.
pub(crate) fn corruption(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(ctx.args, [CompiledArg::I(1)])
        || !matches!(ctx.spec.identity.id, CardId::Corruption)
        || ctx.spec.identity.upgrade > 1
    {
        return Err(EngineRefusal::MalformedArgs("corruption"));
    }
    if ctx.state.powers.value(PowerId::Corruption) == 0 {
        if !ctx
            .state
            .fanouts
            .register_result_location_power(PowerId::Corruption)
        {
            return Err(EngineRefusal::CounterOverflow("fanout listener order"));
        }
        ctx.state.powers.set(PowerId::Corruption, SlotWire::Bool, 1);
        crate::engine::damage::note_power(
            ctx.events,
            crate::engine::Subject::Player,
            PowerId::Corruption,
            1,
        );
    }
    Ok(())
}

/// `("countdown", n)` — CountdownPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its reader takes one CombatTargets draw and
/// applies player-attributed Doom through the shared debuff command.
pub(crate) fn countdown(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "countdown",
        PowerId::Countdown,
        &[(CardId::Countdown, 0, 6), (CardId::Countdown, 1, 9)],
    )
}

/// `("cruelty", n)` — a percentage-point additive at the Vulnerable multiplier
/// site.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the Amount
/// contributes at the Vulnerable multiplier, not as Strength or a standalone
/// factor.
///
pub(crate) fn cruelty(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "cruelty",
        PowerId::Cruelty,
        &[(CardId::Cruelty, 0, 25), (CardId::Cruelty, 1, 50)],
    )
}

/// `("danse_macabre", n)` — flat block before each owned cost-two-or-more play.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — an over-gated additive Intensity stack
/// on the owner scalar (`PowerCmd::Apply` is entry-gated by IsOverOrEnding).
/// [`crate::engine::play`] reads the final resolved energy cost before the
/// card body and grants native flat block when it is at least two.
pub(crate) fn danse_macabre(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "danse_macabre",
        PowerId::DanseMacabre,
        &[(CardId::DanseMacabre, 0, 4), (CardId::DanseMacabre, 1, 6)],
    )
}

/// `("dark_embrace", n)` — draw per ordinary owned card exhausted.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `after_card_exhausted_power_order` on the zero-to-positive edge, then
/// stacks; Intensity stacking preserves the private etherealCount and listener
/// order.
/// [`crate::engine::draw::card_exhausted`] walks the acquisition order before
/// card-local exhaust tails. True Ethereal exhaustion remains refused, so the
/// separate private Ethereal tally is admitted only at zero.
pub(crate) fn dark_embrace(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "dark_embrace",
        PowerId::DarkEmbrace,
        &[(CardId::DarkEmbrace, 0, 1), (CardId::DarkEmbrace, 1, 1)],
        FanoutOrder::AfterCardExhausted,
    )
}

/// `("debilitate", n)` — DebilitatePower onto the chosen enemy.
///
/// Current v0.111.0 native `Debilitate/<OnPlay>d__5::MoveNext` (`0x3973fc`)
/// awaits Attack 10/12, then applies `DebilitatePower` 2/3 to the same
/// CardPlay target. `DebilitatePower` is permanent Type 2, additive StackType
/// 1 (`get_Type` `0xa1485`, `get_StackType` `0xa1488`). Python's
/// `_run_steps_inner` (frozen Python, deleted #2827) mirrors the target/liveness/terminal gates and
/// sends the amount through Artifact, Lamp and Misery's acquisition order.
pub(crate) fn debilitate(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected_attack = match ctx.spec.identity.upgrade {
        0 => 10,
        1 => 12,
        _ => return Err(EngineRefusal::MalformedArgs("debilitate program")),
    };
    let expected_power = if ctx.spec.identity.upgrade == 0 { 2 } else { 3 };
    let exact_program = match ctx.spec.row.steps {
        [
            crate::content_tables::Step {
                kind: StepKind::Attack,
                args:
                    [
                        crate::content_tables::Arg::I(attack),
                        crate::content_tables::Arg::I(1),
                    ],
            },
            crate::content_tables::Step {
                kind: StepKind::Debilitate,
                args: [crate::content_tables::Arg::I(power)],
            },
        ] => *attack == expected_attack && *power == expected_power,
        _ => false,
    };
    if ctx.spec.identity.id != CardId::Debilitate || !exact_program {
        return Err(EngineRefusal::MalformedArgs("debilitate program"));
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)]
            if matches!(
                (ctx.spec.identity.id, ctx.spec.identity.upgrade, *amount),
                (CardId::Debilitate, 0, 2) | (CardId::Debilitate, 1, 3)
            ) =>
        {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("debilitate"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("debilitate")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target) {
        return Ok(());
    }
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if monster.powers.value(PowerId::Debilitate) < 0 {
        return Err(EngineRefusal::MalformedArgs("debilitate amount"));
    }
    if ctx.state.history.over || monster.hp <= 0 {
        return Ok(());
    }
    if post_attack_stock_target_identity_is_unrepresentable(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "Debilitate Axebot Stock target identity",
        ));
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Debilitate,
        MiseryToken::Debilitate,
        amount,
        ctx.events,
    )
}

/// `("demesne", n)` — DemesnePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. The turn-start max-energy and hand-draw folds each
/// read the complete amount once.
pub(crate) fn demesne(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "demesne",
        PowerId::Demesne,
        &[(CardId::Demesne, 0, 1), (CardId::Demesne, 1, 1)],
    )
}

/// `("demon_form", n)` — Strength per owner turn start.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the persistent additive Intensity power;
/// the keyed 3/4 amount is the DynamicVars BaseValue and application stays in
/// the ordinary replayable Power-body path.
///
/// Current v0.111.0 `DemonFormPower` wrapper/body RVAs are `0xa16c0` /
/// `0x338d84`. The persistent additive Intensity power registers at its first
/// application position; restacks preserve that position.
///
/// `engine::turn` consumes the keyed amount through the ordinary owner
/// Strength application after the hand draw.
pub(crate) fn demon_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "demon_form",
        PowerId::DemonForm,
        &[(CardId::DemonForm, 0, 3), (CardId::DemonForm, 1, 4)],
    )
}

/// `("devour_life", n)` — DevourLifePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a permanent additive AfterCardPlayed
/// listener; its exact Soul-class trigger is dispatched at play completion.
///
/// The completed-play walk reads the live amount only for an exact Soul and
/// grows or revives the singular Osty after the Soul body, before later
/// AfterCardPlayed listeners.
pub(crate) fn devour_life(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "devour_life",
        PowerId::DevourLife,
        &[(CardId::DevourLife, 0, 1), (CardId::DevourLife, 1, 2)],
    )
}

/// `("doom", n)` — card-sourced DoomPower onto the chosen enemy (Scourge).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — on a living target, a visible Type-2
/// application through the Artifact gate and the Unsettling Lamp walk, in raw
/// IL order before Scourge's following draw.
///
/// Slice 5 carries Doom's enemy-side tick, ordered kill batch, death cleanup,
/// and player-authored current-turn history latch. Both relic modifiers in the
/// card-sourced application path remain unreachable because every relic is
/// refused, so [`apply_card_monster_debuff`] is the exact admitted command.
pub(crate) fn doom(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match ctx.args {
        [CompiledArg::I(value)]
            if matches!(
                (ctx.spec.identity.id, ctx.spec.identity.upgrade, *value),
                (CardId::Scourge, 0, 13) | (CardId::Scourge, 1, 16)
            ) =>
        {
            *value
        }
        _ => return Err(EngineRefusal::MalformedArgs("doom")),
    };
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("doom"))?;
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
        PowerId::Doom,
        MiseryToken::Doom,
        amount,
        ctx.events,
    )
}

/// `("draw_next_turn", n)` — `n` extra cards at the next turn's hand draw.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. The next hand draw snapshots the total and removes
/// the complete one-shot slot before issuing Draw.
pub(crate) fn draw_next_turn(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "draw_next_turn",
        PowerId::DrawNextTurn,
        &[
            (CardId::Glow, 0, 1),
            (CardId::Glow, 1, 1),
            (CardId::Predator, 0, 2),
            (CardId::Predator, 1, 2),
            (CardId::Relax, 0, 2),
            (CardId::Relax, 1, 3),
        ],
    )
}

/// `("echo_form", n)` — the first `n` cards each turn are played twice.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; casting
/// ECHO_FORM (a Power) is not doubled by itself — the play count was generated
/// before OnPlay applied it.
///
/// The play pipeline freezes its replay count before the first body. Power
/// bodies without a generic Signal Boost witness retain the oracle's exact
/// source/body refusal.
pub(crate) fn echo_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "echo_form",
        PowerId::EchoForm,
        &[(CardId::EchoForm, 0, 1), (CardId::EchoForm, 1, 1)],
    )
}

/// `("entropy", n)` — transform `n` Hand cards at each owner turn start.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) applies the exact additive amount and
/// registers the unified callback order only on the zero-to-positive edge.
/// Later stacks preserve that first-acquisition position.
///
/// Native appends one `AfterPlayerTurnStart` listener on the first positive
/// application and later stacks only live Intensity. The shared eight-token
/// carrier preserves first-application order against every represented peer.
/// Entropy therefore composes with Tools of the Trade, Tyranny, and the five
/// ordinary callbacks instead of imposing a peer refusal. Hibernate remains
/// behind its symmetric unkeyed-listener wall because canonical state cannot
/// prove its native position. Entropy's serial selection and transform body
/// live in `engine::turn`.
pub(crate) fn entropy(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "entropy",
        PowerId::Entropy,
        &[(CardId::Entropy, 0, 1), (CardId::Entropy, 1, 1)],
    )
}

/// `("envenom", n)` — Poison per unblocked attack damage instance.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in `after_damage_given_order`
/// on the zero-to-positive edge, then stacks.
///
/// [`crate::engine::damage`] walks that frozen acquisition order for every
/// powered owner hit and applies Poison only after positive unblocked damage.
pub(crate) fn envenom(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "envenom",
        PowerId::Envenom,
        &[(CardId::Envenom, 0, 1), (CardId::Envenom, 1, 2)],
        FanoutOrder::AfterDamageGiven,
    )
}

/// `("fasten", n)` — FastenPower, a powered-block additive on Defend-tagged
/// cards only (see `modified_card_block_decimal` in `engine/damage.rs`).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
pub(crate) fn fasten(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "fasten",
        PowerId::Fasten,
        &[(CardId::Fasten, 0, 4), (CardId::Fasten, 1, 6)],
    )
}

/// `("feel_no_pain", n)` — block per owned card exhausted.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `after_card_exhausted_power_order` on the zero-to-positive edge, then
/// stacks.
/// [`crate::engine::draw::card_exhausted`] grants flat block in the frozen
/// acquisition-order walk before card-local exhaust tails.
pub(crate) fn feel_no_pain(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "feel_no_pain",
        PowerId::FeelNoPain,
        &[(CardId::FeelNoPain, 0, 3), (CardId::FeelNoPain, 1, 4)],
        FanoutOrder::AfterCardExhausted,
    )
}

/// `("flame_barrier", n)` — retaliate `n` per powered attack hit this turn.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage::monster_attack_player`] emits the blockable,
/// unpowered retaliation after the player's damage result, including a fully
/// blocked hit. [`crate::engine::turn`] clears the whole amount at enemy-side
/// end. Admission refuses co-reachable Reflect/The Gambit peers whose native
/// application order is not projected.
pub(crate) fn flame_barrier(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "flame_barrier",
        PowerId::FlameBarrier,
        &[(CardId::FlameBarrier, 0, 4), (CardId::FlameBarrier, 1, 6)],
    )
}

/// `("focus", n)` — permanent FocusPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; unlike the
/// TemporaryFocusPower wrapper it has no side-turn removal.
///
/// Issue #1394's Focus slice pins the two Defragment rows and Biased
/// Cognition's separately-awaited leading Focus application. The orb engine
/// reads the live signed amount and clamps every value read at zero.
pub(crate) fn focus(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "focus",
        PowerId::Focus,
        &[
            (CardId::BiasedCognition, 0, 5),
            (CardId::BiasedCognition, 1, 6),
            (CardId::Defragment, 0, 1),
            (CardId::Defragment, 1, 2),
        ],
    )
}

/// `("foregone", n)` — select the summed Amount at one turn-start firing.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `ForegoneConclusionPower` is Type/StackType 1 and its
/// `BeforeHandDraw` state machine RVA `0x33a8e8` performs
/// `ShuffleIfNecessary` at IL `0x0048-0x00ae`, selects exactly live Amount
/// from Draw at `0x00b3-0x0143`, appends the result to Hand at
/// `0x0145-0x01a5`, then removes the whole power at `0x01ab-0x0201`.
/// Python `_run_steps_inner` (frozen, deleted #2827) registers the acquisition-order
/// listener and stacks the exact L0/L1 values 2/3. Its reader
/// `_fire_foregone_conclusion` proves the synchronous
/// FromCombatPile auto-take quotient and the larger-pile selection frontier.
/// Admission proves the entry population is no-choice; the live reader
/// repeats that bound after the optional reshuffle and fails closed if later
/// generation enlarged it.
pub(crate) fn foregone(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    // PowerCmd::Apply publishes a Type-1 AfterPowerAmountChanged walk after
    // storing the amount. Every represented callback is inert for a
    // player-owned Foregone change, but its complete acquisition order must
    // still be authenticated before that walk can be elided.
    if !before_hand_draw_power_order_is_exact(ctx.state)
        || !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("foregone listener orders"));
    }
    apply_exact_ordered_player_power(
        ctx,
        "foregone",
        PowerId::Foregone,
        &[
            (CardId::ForegoneConclusion, 0, 2),
            (CardId::ForegoneConclusion, 1, 3),
        ],
        FanoutOrder::BeforeHandDraw,
    )
}

/// `("free_attack", n)` — the next `n` Attack plays cost zero.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the casting play
/// took its BeforeCardPlayed latch and paid its own cost before this OnPlay
/// ran, so it never frees or consumes its own grant.
///
/// Rust `engine::play` shares the matching gate between its late absolute-zero
/// cost term and its per-iteration BeforeCardPlayed decrement.
///
/// IL (v0.111.0): `Unrelenting/<OnPlay>d__3::MoveNext` (RVA `0x3c5a00`) awaits
/// `DamageCmd::Attack` (IL_004c-0x0108), then `PowerCmd.Apply<FreeAttackPower>`
/// at IL_012c; when that attack
/// ends combat the apply is skipped at the `CombatManager.IsEnding` gate
/// (see `apply_exact_player_power`), so the killing play leaves
/// no grant behind (#3160).
pub(crate) fn free_attack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "free_attack",
        PowerId::FreeAttack,
        &[(CardId::Unrelenting, 0, 1), (CardId::Unrelenting, 1, 1)],
    )
}

/// `("free_ethereal", n)` — the next `n` Ethereal card (VeilpiercerPower) plays
/// cost zero.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the casting play
/// took its BeforeCardPlayed latch and paid its own cost before this OnPlay
/// ran, so it never frees or consumes its own grant.
///
/// Rust `engine::play` shares the matching gate between its late absolute-zero
/// cost term and its per-iteration BeforeCardPlayed decrement.
///
/// IL (v0.111.0): `Veilpiercer/<OnPlay>d__5::MoveNext` (RVA `0x3c6408`) awaits
/// `DamageCmd::Attack` (IL_004c), then `PowerCmd.Apply<VeilpiercerPower>` at
/// IL_00fd; when that attack
/// ends combat the apply is skipped at the `CombatManager.IsEnding` gate
/// (see `apply_exact_player_power`), so the killing play leaves
/// no grant behind (#3160).
pub(crate) fn free_ethereal(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "free_ethereal",
        PowerId::FreeEthereal,
        &[(CardId::Veilpiercer, 0, 1), (CardId::Veilpiercer, 1, 1)],
    )
}

/// `("free_power", n)` — the next `n` Power plays cost zero.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the casting play
/// took its BeforeCardPlayed latch and paid its own cost before this OnPlay
/// ran, so it never frees or consumes its own grant.
///
/// Rust `engine::play` shares the matching gate between its late absolute-zero
/// cost term and its per-iteration BeforeCardPlayed decrement.
///
/// IL (v0.111.0): `Synthesis/<OnPlay>d__3::MoveNext` (RVA `0x3c18cc`) awaits
/// `DamageCmd::Attack` (IL_004c), then `PowerCmd.Apply<FreePowerPower>` at
/// IL_00fd; when that attack
/// ends combat the apply is skipped at the `CombatManager.IsEnding` gate
/// (see `apply_exact_player_power`), so the killing play leaves
/// no grant behind (#3160).
pub(crate) fn free_power(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "free_power",
        PowerId::FreePower,
        &[(CardId::Synthesis, 0, 1), (CardId::Synthesis, 1, 1)],
    )
}

/// `("mad_science_curious", n)` — Mad Science's Curious rider (#3427): one
/// CuriousPower application of `n` on the owner.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// re-read for #3427 with `dump_type.py "MadScience/<OnPlay>d__51"
/// "MadScience/<ExecutePower>d__54" CuriousPower`):
///
/// * `MadScience/<OnPlay>d__51::MoveNext` RVA `0x3aaf08` sends a Power-type
///   variant (`TinkerTimeType` 3) to `ExecutePower` (IL_0158-IL_015f), then
///   runs `ExecuteRider` only for rider 1 or 3..6 (IL_01c4-IL_01e2:
///   `rider == 1 || (uint)(rider - 3) <= 3`). Curious is rider 8, so
///   `<ExecuteRider>d__57` RVA `0x3aa9c4` (whose `rider - 1` switch has six
///   arms, Sapping..Chaos) never runs for it: the whole body is ExecutePower.
/// * `<ExecutePower>d__54::MoveNext` RVA `0x3aa660` awaits a cosmetic `Cast`
///   `TriggerAnim` (IL_0034-IL_0054), switches on `rider - 7`
///   (IL_00ae-IL_00b8), and the Curious arm (IL_01f5-IL_0228) calls
///   `PowerCmd.Apply<T>(choiceContext, Owner.Creature,
///   DynamicVars["CuriousReduction"].BaseValue, Owner.Creature, this, false)`
///   through MethodSpec `0x2b002f8c`, whose instantiation is TypeDef 936
///   `CuriousPower`. `CuriousReduction` is 1 at both levels
///   (`data/canonical_vars.v0.111.0.json`), so the allowlist is the generated
///   variant row's `(MadScience, 0|1, 1)`.
/// * `CuriousPower::get_Type` RVA `0xa1026` is 1 (Buff) and `get_StackType`
///   RVA `0xa1029` is 1 (Counter), so a second application adds to the amount;
///   the type declares no hook but `TryModifyEnergyCostInCombat` (RVA
///   `0xa102c`, read by [`crate::engine::play`]'s early cost fold).
///
/// The one refused shape is a Curious beside a live Borrowed Time. Both are
/// early `TryModifyEnergyCostInCombat` listeners, and `Hook::
/// ModifyEnergyCostInCombat` RVA `0x1052f0` folds them in listener order
/// (IL_001d-IL_0043, each call reading the running value), where Borrowed
/// Time's `+Amount` (RVA `0x9feb6` IL_001d-IL_002f) and Curious's
/// "skip at or below zero, else subtract and clamp at zero" do not commute on
/// a Power card whose cost is below the Curious amount. This crate carries no
/// order between the two, so either application refuses while the other is
/// live ([`borrowed_time`] is the other half). The third early listener that
/// reads a Power card, `SpikedGauntlets::TryModifyEnergyCostInCombat` RVA
/// `0x9bd30` (`+1` on the owner's Power cards, IL_0022-IL_0039), is a relic:
/// `CombatState/<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720` lists a
/// creature's powers (IL_0092) before its player's relics (IL_00c9), so the
/// relic always follows Curious and the pair folds in that fixed order
/// (#3437, `engine::play::spiked_gauntlets_surcharged_energy_cost`).
pub(crate) fn mad_science_curious(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !crate::catalog::is_mad_science_variant_program(ctx.spec, "Curious")
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("mad_science_curious"));
    }
    if !crate::engine::damage::damage_combat_is_ending(ctx.state)
        && ctx.state.powers.value(PowerId::BorrowedTime) > 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Curious beside Borrowed Time energy-cost listener order",
        ));
    }
    apply_exact_player_power(
        ctx,
        "mad_science_curious",
        PowerId::Curious,
        &[(CardId::MadScience, 0, 1), (CardId::MadScience, 1, 1)],
    )
}

/// `("free_skill", n)` — the next `n` Skill plays cost zero.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the casting play
/// took its BeforeCardPlayed latch and paid its own cost before this OnPlay
/// ran, so it never frees or consumes its own grant.
///
/// Rust `engine::play` shares the matching gate between its late absolute-zero
/// cost term and its per-iteration BeforeCardPlayed decrement.
///
/// IL (v0.111.0): `Pounce/<OnPlay>d__3::MoveNext` (RVA `0x3b319c`) awaits
/// `DamageCmd::Attack` (IL_004c), then `PowerCmd.Apply<FreeSkillPower>` at
/// IL_00fd; when that attack
/// ends combat the apply is skipped at the `CombatManager.IsEnding` gate
/// (see `apply_exact_player_power`), so the killing play leaves
/// no grant behind (#3160).
pub(crate) fn free_skill(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "free_skill",
        PowerId::FreeSkill,
        &[(CardId::Pounce, 0, 1), (CardId::Pounce, 1, 1)],
    )
}

/// `("genesis", n)` — persistent GenesisPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in `star_energy_reset_order`
/// on the zero-to-positive edge, then stacks.
///
/// `engine::turn::begin_player_turn` consumes the application-ordered reset
/// listener without removing this persistent amount (frozen Python, deleted #2827).
pub(crate) fn genesis(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "genesis",
        PowerId::Genesis,
        &[(CardId::Genesis, 0, 2), (CardId::Genesis, 1, 3)],
        FanoutOrder::StarEnergyReset,
    )
}

/// `("hailstorm", n)` — HailstormPower, a BeforeSideTurnEnd listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_before_side_turn_end_player_power`: order registration plus the
/// additive stack.
///
/// [`crate::engine::turn`] runs the frozen order at player side end and deals
/// the unpowered amount to the stable live roster only while Frost is present.
pub(crate) fn hailstorm(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "hailstorm",
        PowerId::Hailstorm,
        &[(CardId::Hailstorm, 0, 6), (CardId::Hailstorm, 1, 8)],
        FanoutOrder::BeforeSideTurnEnd,
    )
}

/// `("hello_world", 1)` — add one HelloWorldPower stack.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `HelloWorldPower` Type/StackType RVAs `0xa35c7`/`0xa35ca` are both one.
/// `BeforeHandDraw/<MoveNext>` RVA `0x33bfd0` uses the frozen turn-start
/// amount, one full 20-Common distinct Generation shuffle, and one plural
/// generated-card Hand/Bottom command. Creature.BeforeTurnStart 0x11dbe0
/// freezes the independent amount; reapplication must preserve it.
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `before_hand_draw_power_order` on the zero-to-positive edge, then stacks;
/// its exact writer body.
pub(crate) fn hello_world(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    const ROWS: [(CardId, u8, i64); 2] = [(CardId::HelloWorld, 0, 1), (CardId::HelloWorld, 1, 1)];
    // `HelloWorld/<OnPlay>d__3::MoveNext` (RVA `0x3a4bc8`) reaches the power
    // only through `PowerCmd.Apply<HelloWorldPower>` (IL_00c1), which returns
    // at its IsEnding gate (RVA `0x3ef988` IL_0020-0x0034) before any
    // instance or listener exists. While ending, the helper only validates
    // the operand and skips (#3183).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return apply_exact_ordered_player_power(
            ctx,
            "hello_world",
            PowerId::HelloWorld,
            &ROWS,
            FanoutOrder::BeforeHandDraw,
        );
    }
    // Validate the complete persistent pool closure without consuming the
    // live Generation stream. The public turn reader repeats this check and
    // consumes its exact full shuffle only when the listener fires.
    let mut probe = ctx.state.clone();
    crate::engine::cards::select_hello_world_cards(&mut probe, ctx.catalog, 1)?;
    let snapshot = ctx
        .state
        .hello_world_amount_on_turn_start(ctx.state.powers.value(PowerId::HelloWorld))
        .ok_or(EngineRefusal::MalformedArgs(
            "Hello World turn-start snapshot",
        ))?;
    apply_exact_ordered_player_power(
        ctx,
        "hello_world",
        PowerId::HelloWorld,
        &ROWS,
        FanoutOrder::BeforeHandDraw,
    )?;
    let current = ctx.state.powers.value(PowerId::HelloWorld);
    let represented = ctx
        .state
        .set_hello_world_amount_on_turn_start(current, snapshot);
    debug_assert!(represented, "exact Hello World row adds one stack");
    Ok(())
}

/// `("hellraiser", 1)` — HellraiserPower's unique integer presence latch.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — over-gated; StackType 2 Unique, so a
/// reapplication neither adds a listener nor resets the private per-turn
/// recursion data.
///
/// [`crate::engine::draw`] owns the early listener. Admission restricts it to
/// the finite-enemy, nonselecting-Strike quotient whose complete recursive
/// direct-AutoPlay chain is synchronous. Reapplication is a no-op: native's
/// Unique stack type neither replaces the listener nor resets its private
/// recursion data.
pub(crate) fn hellraiser(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(ctx.args, [CompiledArg::I(1)])
        || !matches!(ctx.spec.identity.id, CardId::Hellraiser)
        || ctx.spec.identity.upgrade > 1
    {
        return Err(EngineRefusal::MalformedArgs("hellraiser"));
    }
    if ctx.state.powers.value(PowerId::Hellraiser) == 0 {
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            ctx.state,
            PowerId::Hellraiser,
            0,
            1,
        )?;
        ctx.state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::damage::note_power(
            ctx.events,
            crate::engine::Subject::Player,
            PowerId::Hellraiser,
            1,
        );
    }
    Ok(())
}

/// `("infinite_blades", n)` — mint `n` L0 Shivs per owner hand draw.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `before_hand_draw_power_order` on the zero-to-positive edge, then stacks;
/// unlike Foregone the power persists.
/// [`crate::engine::turn`] freezes the BeforeHandDraw acquisition order and
/// injects fixed L0 Shiv identities at the generated-bottom destination.
pub(crate) fn infinite_blades(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "infinite_blades",
        PowerId::InfiniteBlades,
        &[
            (CardId::InfiniteBlades, 0, 1),
            (CardId::InfiniteBlades, 1, 1),
        ],
        FanoutOrder::BeforeHandDraw,
    )
}

/// `("intangible", n)` — clamp incoming damage results to 1 for `n` turns.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage`] applies the attack-snapshot cap before block and
/// the shared HP-loss-result cap for owner-side sources; [`crate::engine::turn`]
/// removes one duration at enemy-side end.
pub(crate) fn intangible(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "intangible",
        PowerId::Intangible,
        &[
            (CardId::Apparition, 0, 1),
            (CardId::Apparition, 1, 1),
            (CardId::WraithForm, 0, 2),
            (CardId::WraithForm, 1, 3),
        ],
    )
}

/// `("iteration", n)` — IterationPower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`.
/// [`crate::engine::draw`] recursively draws the amount when the first
/// Infection status of the turn is acquired, preserving listener order.
pub(crate) fn iteration(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "iteration",
        PowerId::Iteration,
        &[(CardId::Iteration, 0, 2), (CardId::Iteration, 1, 3)],
        FanoutOrder::AfterCardDrawn,
    )
}

/// `("juggernaut", n)` — random-target damage per owner block gain.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — over-gated; registers in the validated
/// `after_block_gained_power_order` on the zero-to-positive edge, then stacks;
/// the casting play grants no roll of its own.
///
/// [`crate::engine::damage`] owns the represented AfterBlockGained listener:
/// one CombatTargets draw for a nonempty live pool, followed by one blockable,
/// unpowered hit. Card and flat block sources share that funnel.
pub(crate) fn juggernaut(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "juggernaut",
        PowerId::Juggernaut,
        &[(CardId::Juggernaut, 0, 6), (CardId::Juggernaut, 1, 8)],
        FanoutOrder::AfterBlockGained,
    )
}

/// `("knockdown", n)` — append one distinct KnockdownPower instance to the
/// original chosen enemy after the card's awaited attack.
///
/// Current v0.111.0 authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// constructor/canonical/upgrade RVAs `0xe3d13/0xe3d23/0xe3da3`, OnPlay body
/// RVA `0x3a8adc` awaits Attack before its Apply at IL `0x010d`, and power
/// Type/StackType/InstanceType RVAs `0xa44cd/0xa44d0/0xa44d3` are `2/1/1`.
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the same target-identity re-validation
/// as `debilitate`; its Knockdown arm calls
/// `_apply_knockdown_debuff` only for that same still-living creature.
///
/// `Knockdown/<OnPlay>d__5` RVA `0x3a8adc` awaits `PowerCmd.Apply<KnockdownPower>`
/// after its attack (IL_010d). `<Apply>d__1`1` (`0x3ef988`) returns at
/// `IsEnding` (IL_0025-002a); `apply_card_knockdown` still reads `history.over`,
/// so this body tests the shared IsEnding projection (#3515).
pub(crate) fn knockdown(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (expected_attack, expected_power) = match ctx.spec.identity.upgrade {
        0 => (10, 2),
        1 => (14, 3),
        _ => return Err(EngineRefusal::MalformedArgs("knockdown program")),
    };
    let exact_program = match ctx.spec.row.steps {
        [
            crate::content_tables::Step {
                kind: StepKind::Attack,
                args:
                    [
                        crate::content_tables::Arg::I(attack),
                        crate::content_tables::Arg::I(1),
                    ],
            },
            crate::content_tables::Step {
                kind: StepKind::Knockdown,
                args: [crate::content_tables::Arg::I(power)],
            },
        ] => *attack == expected_attack && *power == expected_power,
        _ => false,
    };
    if ctx.spec.identity.id != CardId::Knockdown || !exact_program {
        return Err(EngineRefusal::MalformedArgs("knockdown program"));
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount == expected_power => {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("knockdown"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("knockdown")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target) {
        return Ok(());
    }
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if !crate::engine::damage::knockdown_state_is_exact(monster) {
        return Err(EngineRefusal::MalformedArgs(
            "Knockdown distinct-instance state",
        ));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) || monster.hp <= 0 {
        return Ok(());
    }
    if post_attack_stock_target_identity_is_unrepresentable(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "Knockdown Axebot Stock target identity",
        ));
    }
    apply_card_knockdown(ctx.state, target, amount, ctx.events)
}

/// `("loop", n)` — LoopPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — refuses when an unkeyed
/// AfterPlayerTurnStart peer (ToolsOfTheTrade / Tyranny / Hibernate) is live,
/// then stacks.
///
/// The reader passives the live front orb Amount times. Admission refuses all
/// reachable Loop/Rolling-Boulder/Inferno pairs whose application order is not
/// represented.
pub(crate) fn loop_(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "loop",
        PowerId::Loop,
        &[(CardId::Loop, 0, 1), (CardId::Loop, 1, 2)],
    )
}

/// `("machine_learning", n)` — `n` extra cards at every owner hand draw.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. The ordinary hand-draw count reads the persistent
/// amount every owner turn.
pub(crate) fn machine_learning(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "machine_learning",
        PowerId::MachineLearning,
        &[
            (CardId::MachineLearning, 0, 1),
            (CardId::MachineLearning, 1, 1),
        ],
    )
}

/// `("mayhem", n)` — autoplay the top `n` draw-pile cards per turn start.
/// **Escalated (#1345 triage).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the AutoPre
/// listener snapshots Amount draw cards before any child play.
///
/// AutoPre gathers the complete batch into Play before draining its transient
/// synchronous work stack.
pub(crate) fn mayhem(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "mayhem",
        PowerId::Mayhem,
        &[(CardId::Mayhem, 0, 1), (CardId::Mayhem, 1, 1)],
    )
}

/// `("monarchs_gaze", n)` — an AfterDamageGiven consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in `after_damage_given_order`
/// on the zero-to-positive edge, then stacks.
///
/// [`crate::engine::damage`] applies the temporary-Strength wrapper and its
/// permanent Strength delta for every powered owner hit, including zero.
pub(crate) fn monarchs_gaze(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "monarchs_gaze",
        PowerId::MonarchsGaze,
        &[(CardId::MonarchsGaze, 0, 1), (CardId::MonarchsGaze, 1, 1)],
        FanoutOrder::AfterDamageGiven,
    )
}

/// `("necro_mastery", n)` — exact NecroMasteryPower amount writer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// Current native v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `NecroMastery/<OnPlay>d__9::MoveNext` RVA `0x3ae420` summons live Osty,
/// then applies one additive Intensity. `NecroMasteryPower` callback RVA
/// `0x33e950` reads the live stacked Amount after an owner-Osty negative HP
/// delta and serially issues unpowered/unblockable damage to the frozen
/// HittableEnemies roster. The two exact HP-loss dispatch sites live in
/// `engine::damage`.
pub(crate) fn necro_mastery(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "necro_mastery",
        PowerId::NecroMastery,
        &[(CardId::NecroMastery, 0, 1), (CardId::NecroMastery, 1, 1)],
    )
}

/// `("neurosurge", n)` — NeurosurgePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its turn-start reader adds player Doom and records
/// the Death's Door current-turn attribution latch.
pub(crate) fn neurosurge(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "neurosurge",
        PowerId::Neurosurge,
        &[(CardId::Neurosurge, 0, 3), (CardId::Neurosurge, 1, 3)],
    )
}

/// `("no_draw", n)` — refuse later non-hand draws this turn.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the boolean gate
/// observes only a positive Amount, and the side-end Remove clears the entire
/// instance.
///
/// [`crate::engine::draw::draw_cards`] checks the listener before hand, pile,
/// or RNG work; [`crate::engine::turn`] removes the full instance at side end.
pub(crate) fn no_draw(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "no_draw",
        PowerId::NoDraw,
        &[(CardId::BattleTrance, 0, 1), (CardId::BattleTrance, 1, 1)],
    )
}

/// `("noxious_fumes", n)` — Poison to every enemy per owner turn start.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its reader applies power-sourced Poison to the
/// living roster in order through the shared identity/Misery path.
pub(crate) fn noxious_fumes(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "noxious_fumes",
        PowerId::NoxiousFumes,
        &[(CardId::NoxiousFumes, 0, 2), (CardId::NoxiousFumes, 1, 3)],
    )
}

/// `("oblivion", n)` — card-sourced OblivionPower onto the chosen enemy.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) and native `Oblivion::OnPlay` apply the
/// visible Type-2 debuff on a living frozen target, so Artifact consumes one
/// application and Unsettling Lamp may double it. `play.rs` captures each
/// living owner's old Amount before the body and dispatches that owner's
/// AfterCardPlayed Doom callback in the exact combined Oblivion/Strangle
/// acquisition order. The compact scalar does not retain applier identity,
/// so only the solo local-owner topology is representable.
///
/// On a living target,
/// `_apply_enemy_after_card_played_power`: a visible Type-2 application, so
/// Artifact consumes one application and the Lamp may double; the old-Amount
/// Doom callback belongs to the matching AfterCardPlayed listener.
pub(crate) fn oblivion(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected = match ctx.spec.identity.upgrade {
        0 => 3,
        1 => 4,
        _ => return Err(EngineRefusal::MalformedArgs("oblivion program")),
    };
    let exact_program = matches!(
        ctx.spec.row.steps,
        [crate::content_tables::Step {
            kind: StepKind::Oblivion,
            args: [crate::content_tables::Arg::I(amount)],
        }] if *amount == expected
    );
    if ctx.spec.identity.id != CardId::Oblivion || !exact_program {
        return Err(EngineRefusal::MalformedArgs("oblivion program"));
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount == expected => {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("oblivion"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("oblivion")),
    };
    if ctx.state.multiplayer_ally_key != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "Oblivion local applier identity",
        ));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    let frozen = crate::engine::play::active_card_target_identity(ctx.source_uid)
        .ok_or(EngineRefusal::ContinuationNotModeled)?
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if monster.powers.value(PowerId::Oblivion) < 0 {
        return Err(EngineRefusal::MalformedArgs("oblivion amount"));
    }
    if ctx.state.history.over || monster.hp <= 0 || (monster.slot, monster.uid) != frozen {
        return Ok(());
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Oblivion,
        MiseryToken::Oblivion,
        amount,
        ctx.events,
    )
}

/// `("one_two_punch", n)` — OneTwoPunchPower, a play-replay consumer.
/// **Escalated (#1345 triage).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// GeneratePlayCount adds one Attack replay and consumes one stack.
pub(crate) fn one_two_punch(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "one_two_punch",
        PowerId::OneTwoPunch,
        &[(CardId::OneTwoPunch, 0, 1), (CardId::OneTwoPunch, 1, 2)],
    )
}

/// `("orbit", n)` — OrbitPower, an AfterEnergySpent consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack on the SAME
/// instance: the private Data counters (energy spent / trigger count) persist
/// across re-applies, and the energy spent casting Orbit was already emitted at
/// the spend site.
///
/// That Python stacking is NOT native: OrbitPower is Instanced, so a second
/// application is a new object with its own zeroed `Data` counters. It
/// refuses by name ([`refuse_second_instanced_object`], #3057).
pub(crate) fn orbit(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    refuse_second_instanced_object(
        ctx,
        PowerId::Orbit,
        &[(CardId::Orbit, 0, 1), (CardId::Orbit, 1, 1)],
    )?;
    let amount = match ctx.args {
        [CompiledArg::I(1)]
            if matches!(ctx.spec.identity.id, CardId::Orbit)
                && matches!(ctx.spec.identity.upgrade, 0 | 1) =>
        {
            1
        }
        _ => return Err(EngineRefusal::MalformedArgs("orbit")),
    };
    let updated = ctx
        .state
        .powers
        .value(PowerId::Orbit)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("orbit"))?;
    ctx.state.powers.set(PowerId::Orbit, SlotWire::Int, updated);
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::Orbit,
        updated,
    );
    Ok(())
}

/// `("pagestorm", n)` — PagestormPower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`.
/// [`crate::engine::draw`] owns the ordered recursive reader. Effective
/// Ethereal card admission remains refused, so the writer is currently inert
/// in admitted combat while its exact state and order still round-trip.
pub(crate) fn pagestorm(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "pagestorm",
        PowerId::Pagestorm,
        &[(CardId::Pagestorm, 0, 1), (CardId::Pagestorm, 1, 1)],
        FanoutOrder::AfterCardDrawn,
    )
}

/// `("pale_blue_dot", n)` — PaleBlueDotPower, an AfterCardPlayed consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; Intensity re-
/// application updates the Amount and leaves the private
/// alreadyActivatedThisTurn data unchanged.
/// [`crate::engine::play`] sets the once-per-turn latch on the fifth finished
/// play and applies Draw Next Turn, with terminal peer ordering preflighted.
pub(crate) fn pale_blue_dot(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "pale_blue_dot",
        PowerId::PaleBlueDot,
        &[(CardId::PaleBlueDot, 0, 1), (CardId::PaleBlueDot, 1, 2)],
    )
}

/// `("panache", n)` — damage to all enemies per five cards played.
///
/// Current-v0.111 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `PanachePower::get_InstanceType` RVA `0xa5700` returns Instanced, so every
/// application appends a fresh `{Amount, CardsLeft, alreadyApplied}` object.
/// `PowerCmd/<Apply>d__2::MoveNext` RVA `0x3efbac` reports that object's
/// modified amount.  Like the already-supported instanced The Bomb writer,
/// Rust projects that row-local amount through the uid-less `PowerChanged`
/// event while cold rows remain execution authority.
/// Python: `_run_steps_inner` (frozen, deleted #2827) — seeds the per-turn `panache_left`
/// compatibility mirror only for a singleton exact row; the current `panache`
/// arm also performs row allocation and publishes the active
/// CardPlay UID receipt.
pub(crate) fn panache_private_state_is_exact(state: &HotState) -> bool {
    let amount = match state.powers.get(PowerId::Panache) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value > 0 => slot.value,
        Some(_) => return false,
    };
    state.fanouts.instanced_player_power_records_are_exact()
        && state
            .fanouts
            .panache_instances()
            .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
            == Some(amount)
}

pub(crate) fn panache(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match ctx.args {
        [CompiledArg::I(10)]
            if matches!(
                (ctx.spec.identity.id, ctx.spec.identity.upgrade),
                (CardId::Panache, 0)
            ) =>
        {
            10_i32
        }
        [CompiledArg::I(14)]
            if matches!(
                (ctx.spec.identity.id, ctx.spec.identity.upgrade),
                (CardId::Panache, 1)
            ) =>
        {
            14_i32
        }
        _ => return Err(EngineRefusal::MalformedArgs("panache")),
    };
    if !panache_private_state_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs("panache private state"));
    }
    let mut probe = ctx.state.clone();
    let updated = probe
        .powers
        .value(PowerId::Panache)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("panache"))?;
    let created_uid = probe
        .fanouts
        .begin_panache_instance(amount)
        .map_err(|_| EngineRefusal::CounterOverflow("Panache instance uid"))?;
    crate::engine::play::record_instanced_power_created_uid(
        &mut probe,
        ctx.source_uid,
        created_uid,
    )?;
    probe.powers.set(PowerId::Panache, SlotWire::Int, updated);
    if !panache_private_state_is_exact(&probe) {
        return Err(EngineRefusal::MalformedArgs("panache private result"));
    }
    *ctx.state = probe;
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::Panache,
        amount,
    );
    Ok(())
}

/// `("phantom_blades", n)` — a Shiv damage additive plus local Retain on every
/// current Shiv.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — stacks, then `_phantom_retain_all_cards`
/// grants local Retain idempotently; future entrants use the generated-shiv
/// AfterCardEnteredCombat site.
///
/// Validate every current Draw/Hand/Discard atom before publishing the power,
/// then apply combat-local Retain to the exact physical Shiv instances after
/// the power event. Any changed per-instance keyword forces exact pile
/// projection. Future generated Shivs and the first-own-Shiv-play damage
/// additive are handled at their shared engine choke points.
pub(crate) fn phantom_blades(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let mut current_shivs = Vec::new();
    let mut changes_keyword = false;
    for pile in [PileId::Draw, PileId::Hand, PileId::Discard] {
        for card in ctx.state.piles.get(pile).as_slice() {
            let spec = ctx
                .catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.row.tags.contains(&"Shiv") {
                current_shivs.push(card.uid);
                changes_keyword |= !ctx
                    .state
                    .card_states
                    .get_ref(card.uid)
                    .is_some_and(|instance| instance.local_retain);
            }
        }
    }
    // The Retain grant is `PhantomBladesPower::AfterApplied` (RVA `0xa5938`
    // IL_001c-0x005f), which `PowerCmd.Apply` reaches only past its IsEnding
    // gate (RVA `0x3ef988` IL_0020-0x0034): a skipped apply grants nothing.
    let ending = crate::engine::damage::damage_combat_is_ending(ctx.state);
    apply_exact_player_power(
        ctx,
        "phantom_blades",
        PowerId::PhantomBlades,
        &[
            (CardId::PhantomBlades, 0, 9),
            (CardId::PhantomBlades, 1, 12),
        ],
    )?;
    if ending {
        return Ok(());
    }
    if changes_keyword {
        ctx.state.exact_piles = true;
    }
    for uid in current_shivs {
        ctx.state.card_states.set_local_retain(uid);
    }
    Ok(())
}

/// `("pillar_of_creation", n)` — block per generated card entering combat.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `local_generated_power_order` on the zero-to-positive edge
/// (`_validated_soulbound_state`), then stacks the Type-1/Intensity instance
/// without moving it.
///
pub(crate) fn pillar_of_creation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "pillar_of_creation",
        PowerId::PillarOfCreation,
        &[
            (CardId::PillarOfCreation, 0, 2),
            (CardId::PillarOfCreation, 1, 3),
        ],
        FanoutOrder::LocalGenerated,
    )
}

/// `("plating", n)` — block at the owner's turn boundaries.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. The end boundary grants the live amount as flat
/// block before turn-end cards; turn starts after the first decrement once.
pub(crate) fn plating(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "plating",
        PowerId::Plating,
        &[
            (CardId::EternalArmor, 0, 9),
            (CardId::EternalArmor, 1, 12),
            (CardId::NeutronAegis, 0, 8),
            (CardId::NeutronAegis, 1, 11),
            (CardId::StoneArmor, 0, 4),
            (CardId::StoneArmor, 1, 6),
        ],
    )
}

/// `("poison", amount)` — the template Poison application (Deadly Poison and
/// friends).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `if target.hp > 0:
/// apply_monster_debuff(target, "poison", _card_power_amount_given(s, target,
/// "poison", step[1]), log, state=s)`.
///
/// The only guard is the target's life — unlike its Bubble Bubble sibling
/// there is no `s.over` test and no already-poisoned precondition, so this is
/// the family's plain card-sourced application and the engine's first Poison
/// *source*.
pub(crate) fn poison(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("poison"));
    };
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("poison"))?;
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
        PowerId::Poison,
        MiseryToken::Poison,
        amount,
        ctx.events,
    )
}

/// `("prep_time", n)` — PrepTimePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its reader stacks Vigor; the next complete player
/// Attack command snapshots and consumes the whole amount.
///
/// Current v0.111.0 wrapper/body RVAs are `0xa6184` / `0x341240`. First
/// application registers its native side-start position and later Intensity
/// stacks do not move it.
pub(crate) fn prep_time(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "prep_time",
        PowerId::PrepTime,
        &[(CardId::PrepTime, 0, 4), (CardId::PrepTime, 1, 6)],
    )
}

/// `("pyre", n)` — PyrePower, a max-energy modifier.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// The owner turn's max-energy reset folds the complete amount additively.
pub(crate) fn pyre(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "pyre",
        PowerId::Pyre,
        &[(CardId::Pyre, 0, 1), (CardId::Pyre, 1, 2)],
    )
}

/// `("rage", n)` — block per owned Attack played this turn.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack: every
/// application adds to the current turn's owned-Attack listener.
/// [`crate::engine::play`] grants the flat block after each finished Attack;
/// [`crate::engine::turn`] removes the amount at owner side end.
pub(crate) fn rage(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "rage",
        PowerId::Rage,
        &[(CardId::Rage, 0, 3), (CardId::Rage, 1, 5)],
    )
}

/// `("reaper_form", n)` — an AfterDamageGiven Doom consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in `after_damage_given_order`
/// on the zero-to-positive edge, then stacks.
///
/// [`crate::engine::damage`] applies Doom from positive total powered damage,
/// including blocked damage and lethal results, in frozen acquisition order.
pub(crate) fn reaper_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "reaper_form",
        PowerId::ReaperForm,
        &[(CardId::ReaperForm, 0, 1), (CardId::ReaperForm, 1, 1)],
        FanoutOrder::AfterDamageGiven,
    )
}

/// `("reflect", n)` — reflect the integer block spent by a powered hit.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage::monster_attack_player`] emits the blockable,
/// unpowered retaliation after the player result; [`crate::engine::turn`]
/// removes one duration at player-turn start. The card remains refused on its
/// out-of-scope Star cost, while boundary-carried Reflect is readable.
pub(crate) fn reflect(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "reflect",
        PowerId::Reflect,
        &[(CardId::Reflect, 0, 1), (CardId::Reflect, 1, 1)],
    )
}

/// `("retain_hand", n)` — retain the whole hand across `n` turn ends.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// The turn-end reader suppresses the whole flush before the per-instance
/// Retain partition, then decrements this duration once in the owner-power
/// AfterSideTurnEnd group.
pub(crate) fn retain_hand(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "retain_hand",
        PowerId::RetainHand,
        &[
            (CardId::Convergence, 0, 1),
            (CardId::Convergence, 1, 1),
            (CardId::Equilibrium, 0, 1),
            (CardId::Equilibrium, 1, 1),
            (CardId::Salvo, 0, 1),
            (CardId::Salvo, 1, 1),
        ],
    )
}

/// `("rolling_boulder", n)` — RollingBoulderPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — refuses when an unkeyed
/// AfterPlayerTurnStart peer (ToolsOfTheTrade / Tyranny / Hibernate) is live,
/// then stacks.
///
/// The reader issues one flat AoE, then grows the persistent amount by five
/// even when the damage ended combat. Admission refuses all observable
/// unprojected AfterPlayerTurnStart peer orders.
///
/// A second live RollingBoulderPower object refuses by name
/// ([`refuse_second_instanced_object`], #3057): native runs its own AoE and
/// its own +5 growth.
pub(crate) fn rolling_boulder(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    const ALLOWED: &[(CardId, u8, i64)] = &[
        (CardId::RollingBoulder, 0, 5),
        (CardId::RollingBoulder, 1, 10),
    ];
    refuse_second_instanced_object(ctx, PowerId::RollingBoulder, ALLOWED)?;
    apply_exact_player_power(ctx, "rolling_boulder", PowerId::RollingBoulder, ALLOWED)
}

/// `("rupture", n)` — Strength per qualifying owner-side HP-loss result.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack preserving the
/// power's internal exact-card dictionary; the casting play's BeforeCardPlayed
/// latch was taken before this application.
///
/// [`crate::engine::damage`] applies null-source owner-side results immediately.
/// Native active-card results accumulate in the exact persisted CardPlay and
/// publish one Strength command at AfterCardPlayed.
pub(crate) fn rupture(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "rupture",
        PowerId::Rupture,
        &[(CardId::Rupture, 0, 1), (CardId::Rupture, 1, 2)],
    )
}

/// `("sealed_throne", n)` — additive TheSealedThronePower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// `engine::play` gains the live amount before every CardPlay body; the casting
/// iteration therefore sees the old amount and a replay sees the new one
/// (Python `_powers_before_card_played` (frozen, deleted #2827)).
pub(crate) fn sealed_throne(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "sealed_throne",
        PowerId::SealedThrone,
        &[
            (CardId::TheSealedThrone, 0, 1),
            (CardId::TheSealedThrone, 1, 1),
        ],
    )
}

/// `("sentry_mode", n)` — SentryModePower, a BeforeHandDraw listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in the validated
/// `before_hand_draw_power_order` on the zero-to-positive edge, then stacks.
///
pub(crate) fn sentry_mode(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_sentry_mode_foundation(ctx)
}

/// Authenticate the complete currently represented BeforeHandDraw order.
///
/// The order is acquisition ordered rather than enum ordered, so every
/// permutation of the exact live set is valid. Inactive slots are canonically
/// vacant, and a represented active slot must be an exact positive `int`.
#[cfg_attr(not(test), allow(dead_code))]
#[cold]
#[inline(never)]
pub(crate) fn before_hand_draw_power_order_is_exact(state: &HotState) -> bool {
    let order = state.fanouts.before_hand_draw_order();
    if order
        .iter()
        .any(|power| !is_represented_before_hand_draw_power(*power))
    {
        return false;
    }

    let mut active = 0usize;
    for slot in state.powers.as_slice() {
        if !is_represented_before_hand_draw_power(slot.key) {
            continue;
        }
        if slot.wire != SlotWire::Int || slot.value <= 0 {
            return false;
        }
        active += 1;
        if order
            .iter()
            .filter(|candidate| **candidate == slot.key)
            .count()
            != 1
        {
            return false;
        }
    }
    order.len() == active
}

#[inline(never)]
fn is_represented_before_hand_draw_power(power: PowerId) -> bool {
    matches!(
        power,
        PowerId::InfiniteBlades
            | PowerId::CallOfTheVoid
            | PowerId::CreativeAi
            | PowerId::Foregone
            | PowerId::HelloWorld
            | PowerId::SpectrumShift
            | PowerId::SentryMode
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn sentry_mode_live_amount(state: &HotState) -> Result<i32, EngineRefusal> {
    match state.powers.get(PowerId::SentryMode) {
        Some(slot) if slot.wire == SlotWire::Int && slot.value >= 0 => Ok(slot.value),
        Some(_) => Err(EngineRefusal::MalformedArgs(
            "Sentry Mode live power amount",
        )),
        None => Ok(0),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn exact_sentry_mode_leaf(catalog: &Catalog) -> Result<CardIdentity, EngineRefusal> {
    let identity = CardIdentity {
        id: CardId::SweepingGaze,
        upgrade: 0,
        enchantment: None,
    };
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    let exact_row = crate::content_tables::card_row(CardId::SweepingGaze, 0) == Some(spec.row);
    let exact_program = match catalog.steps(spec) {
        [step] => {
            step.kind == StepKind::OstyBody
                && catalog.args(step.args)
                    == [CompiledArg::Word(StepWord::Random), CompiledArg::I(10)]
        }
        _ => false,
    };
    if !exact_row || !exact_program {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode exact Sweeping Gaze leaf",
        ));
    }
    Ok(identity)
}

/// v0.111.0 `SentryMode/<OnPlay>` RVA `0x3b9bf4` awaits `PowerCmd.Apply<SentryModePower>` at IL_00d1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
fn apply_sentry_mode_foundation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let exact_row =
        crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            == Some(ctx.spec.row);
    let exact_program = match ctx.catalog.steps(ctx.spec) {
        [step] => {
            step.kind == StepKind::SentryMode && ctx.catalog.args(step.args) == [CompiledArg::I(1)]
        }
        _ => false,
    };
    if ctx.spec.identity.id != CardId::SentryMode
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || !exact_row
        || !exact_program
        || ctx.args != [CompiledArg::I(1)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode exact source program",
        ));
    }
    exact_sentry_mode_leaf(ctx.catalog)?;

    let mut matches = 0usize;
    let mut exact_play_source = false;
    let mut source = None;
    for pile in PileId::ALL {
        for card in ctx.state.piles.get(pile).as_slice() {
            if card.uid == ctx.source_uid {
                matches += 1;
                exact_play_source = pile == PileId::Play;
                source = Some(*card);
            }
        }
    }
    if matches != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches,
        });
    }
    let source = source.expect("one exact source was counted");
    if !exact_play_source
        || source.flags & CARD_FLAG_LEGACY != 0
        || ctx.catalog.spec(source.atom) != Some(ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode exact physical source",
        ));
    }
    if !before_hand_draw_power_order_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode BeforeHandDraw order",
        ));
    }
    // SentryModePower is native Type 1. PowerCmd therefore freezes and walks
    // AfterPowerAmountChanged after storing Amount even though all three
    // represented callback bodies are inert for this player-owned change.
    // Slots::set removes zero, so the direct private seam additionally closes
    // the physical wire shape before using the shared active-set validator.
    let exact_type_one_peer_slots = [PowerId::Vicious, PowerId::Shroud, PowerId::SleightOfFlesh]
        .into_iter()
        .all(|power| match ctx.state.powers.get(power) {
            None => true,
            Some(slot) => slot.wire == SlotWire::Int && slot.value > 0,
        });
    if !exact_type_one_peer_slots
        || !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode AfterPowerAmountChanged order",
        ));
    }

    fn apply(state: &mut HotState, events: &mut Vec<Event>) -> Result<(), EngineRefusal> {
        if crate::engine::damage::damage_combat_is_ending(state) {
            return Ok(());
        }
        let old = state.powers.value(PowerId::SentryMode);
        let updated = old
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("Sentry Mode"))?;
        if old == 0 && !state.fanouts.register_before_hand_draw(PowerId::SentryMode) {
            return Err(EngineRefusal::CounterOverflow(
                "Sentry Mode BeforeHandDraw order",
            ));
        }
        state
            .powers
            .set(PowerId::SentryMode, SlotWire::Int, updated);
        crate::engine::damage::note_power(events, Subject::Player, PowerId::SentryMode, updated);
        Ok(())
    }

    // Registration precedes the checked scalar write in native application.
    // Rehearse the complete command so neither that order token nor its event
    // can leak when the later addition refuses.
    let mut probe = ctx.state.clone();
    apply(&mut probe, &mut Vec::new())?;
    apply(ctx.state, ctx.events)
}

/// Authenticate the complete public turn-entry bundle for Sentry Mode.
pub(crate) fn sentry_mode_turn_entry_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    exact_sentry_mode_leaf(catalog).is_ok()
        && sentry_mode_live_amount(state).is_ok_and(|amount| amount > 0)
        && before_hand_draw_power_order_is_exact(state)
        && state
            .fanouts
            .before_hand_draw_order()
            .contains(&PowerId::SentryMode)
}

/// Run the exact Sentry Mode BeforeHandDraw listener.
pub(crate) fn run_sentry_mode_before_hand_draw(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let identity = exact_sentry_mode_leaf(catalog)?;
    if sentry_mode_live_amount(state)? == 0 {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode live power amount",
        ));
    }
    if !before_hand_draw_power_order_is_exact(state)
        || !state
            .fanouts
            .before_hand_draw_order()
            .contains(&PowerId::SentryMode)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Sentry Mode BeforeHandDraw listener",
        ));
    }
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let mut iteration = 0i32;
        loop {
            // Native awaits one generated command, then re-reads the live
            // power object Amount at this boundary rather than freezing it.
            let amount = sentry_mode_live_amount(state)?;
            if iteration >= amount {
                return Ok(());
            }
            crate::engine::cards::inject_before_hand_draw_generated_bottom(
                state, catalog, identity, events,
            )?;
            iteration = iteration
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Sentry Mode iteration"))?;
        }
    }

    // One awaited listener owns the whole live-Amount loop. Later generated
    // transactions can refuse after earlier ones published; rehearse the
    // complete body before exposing any history, hook, uid, pile, or event.
    let mut probe = state.clone();
    apply(&mut probe, catalog, identity, &mut Vec::new())?;
    apply(state, catalog, identity, events)
}

/// `("signal_boost", n)` — SignalBoostPower, a power-replay consumer.
/// **Escalated (#1345 triage).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; Signal Boost is
/// a Skill, so an active Burst can rerun the complete body — each body applies
/// one stack.
///
/// Signal Boost is the generic current-build replay witness for every
/// admitted Power body; GeneratePlayCount consumes one stack per action.
pub(crate) fn signal_boost(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "signal_boost",
        PowerId::SignalBoost,
        &[(CardId::SignalBoost, 0, 1), (CardId::SignalBoost, 1, 1)],
    )
}

/// `("sleight_of_flesh", n)` — an AfterPowerAmountChanged consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — over-gated; registers in
/// `after_power_amount_changed_power_order` on the zero-to-positive edge, then
/// stacks.
///
/// [`crate::engine::damage`] deals its blockable, unpowered amount after any
/// nonzero player-applied permanent Type-2 monster amount change.
pub(crate) fn sleight_of_flesh(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "sleight_of_flesh",
        PowerId::SleightOfFlesh,
        &[
            (CardId::SleightOfFlesh, 0, 9),
            (CardId::SleightOfFlesh, 1, 13),
        ],
        FanoutOrder::AfterPowerAmountChanged,
    )
}

/// `("smokestack", n)` — a generated-card listener (5/7 after its presentation-
/// only animation).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in
/// `local_generated_power_order` on the zero-to-positive edge
/// (`_validated_soulbound_state`), then stacks.
///
pub(crate) fn smokestack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "smokestack",
        PowerId::Smokestack,
        &[(CardId::Smokestack, 0, 5), (CardId::Smokestack, 1, 7)],
        FanoutOrder::LocalGenerated,
    )
}

/// `("sneaky", n)` — persistent block after each owned Attack played.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the same additive listener shape as
/// Rage, but with no AfterSideTurnEnd hook: it persists.
/// [`crate::engine::play`] implements the persistent reader. The only source
/// card still refuses independently on its Sly keyword.
pub(crate) fn sneaky(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "sneaky",
        PowerId::Sneaky,
        &[(CardId::Sneaky, 0, 1), (CardId::Sneaky, 1, 2)],
    )
}

/// `("speedster", n)` — SpeedsterPower, an AfterCardDrawn listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — routes through
/// `_apply_after_card_drawn_power`.
/// [`crate::engine::draw`] deals the stable roster-wide amount for every draw
/// EXCEPT the turn-start hand draw (`fromHandDraw` true exits,
/// `SpeedsterPower/<AfterCardDrawn>d__4::MoveNext` RVA `0x345abc`
/// IL_0020-0028), and only while the player side is active.
pub(crate) fn speedster(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "speedster",
        PowerId::Speedster,
        &[(CardId::Speedster, 0, 2), (CardId::Speedster, 1, 2)],
        FanoutOrder::AfterCardDrawn,
    )
}

/// `("spirit_of_ash", n)` — flat block before each owned Ethereal card play.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — an over-gated additive Intensity stack
/// on the owner scalar (`PowerCmd::Apply` is entry-gated by IsOverOrEnding).
/// [`crate::engine::play`] implements the exact reader. Effective Ethereal
/// card admission remains refused, so the writer is inert in admitted combat.
pub(crate) fn spirit_of_ash(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "spirit_of_ash",
        PowerId::SpiritOfAsh,
        &[(CardId::SpiritOfAsh, 0, 4), (CardId::SpiritOfAsh, 1, 5)],
    )
}

/// `("stampede", n)` — StampedePower, an AutoPost consumer.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; the
/// AutoPost listener performs Amount live hand selections.
///
/// R19's persistent Phase/LiveListener driver owns the exact reader. Admission
/// closes every reachable writer/remover/reapply descendant so its immutable
/// entry-Amount witness is equivalent to native's retained PowerModel object.
pub(crate) fn stampede(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "stampede",
        PowerId::Stampede,
        &[(CardId::Stampede, 0, 1), (CardId::Stampede, 1, 1)],
    )
}

/// `("star_next_turn", n)` — one-shot Stars at the next energy reset.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — registers in `star_energy_reset_order`
/// on the zero-to-positive edge, then stacks.
///
/// `engine::turn::begin_player_turn` gains the frozen amount, removes the
/// power, and removes its order token before later reset listeners
/// (Python `begin_player_turn` (frozen, deleted #2827)).
pub(crate) fn star_next_turn(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_ordered_player_power(
        ctx,
        "star_next_turn",
        PowerId::StarNextTurn,
        &[
            (CardId::Convergence, 0, 1),
            (CardId::Convergence, 1, 2),
            (CardId::HiddenCache, 0, 3),
            (CardId::HiddenCache, 1, 4),
        ],
        FanoutOrder::StarEnergyReset,
    )
}

/// `("strangle", n)` — StranglePower onto the exact creature the preceding
/// attack targeted.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — reads the live CardPlayFrame's frozen
/// `(slot, uid)` target identity (an Axebot replacement in the same slot is a
/// different creature), then `_apply_enemy_after_card_played_power`.
///
/// The active-play context retains that exact identity across every replay.
/// A lethal Stock replacement therefore receives nothing, while the common
/// enemy-owner suffix consumes the amount latched before this individual
/// generated play. Application uses the shared card-sourced Type-2 path, so
/// Artifact, Unsettling Lamp, Misery, acquisition order, and rollback remain
/// identical to every other represented permanent debuff.
///
/// # Mad Science's Choking rider (#2942)
///
/// The Choking variant ([`crate::catalog::MadScienceVariant`]) plays the same
/// two commands. `MadScience/<OnPlay>d__51::MoveNext` (v0.111.0 RVA
/// `0x3aaf08`) runs `ExecuteAttack` against `cardPlay.Target` (IL_007b-IL_0093)
/// and then `ExecuteRider` with that same `cardPlay.Target` (IL_01e4-IL_01fc);
/// `<ExecuteRider>d__57::MoveNext` (RVA `0x3aa9c4`) applies `StranglePower` of
/// the `ChokingDamage` var's BaseValue to it, with the owner creature and the
/// card as source (IL_0182-IL_01af). So the target is the play's own frozen
/// creature exactly as for Strangle, and only the two amounts differ; they
/// are the generated variant row's (the canonical vars' `Damage` and
/// `ChokingDamage`).
pub(crate) fn strangle(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let mad_science_choking = crate::catalog::is_mad_science_variant_program(ctx.spec, "Choking");
    let (expected_attack, expected_power) = if mad_science_choking {
        match ctx.spec.row.steps {
            [
                crate::content_tables::Step {
                    kind: StepKind::Attack,
                    args: [crate::content_tables::Arg::I(attack), _],
                },
                crate::content_tables::Step {
                    kind: StepKind::Strangle,
                    args: [crate::content_tables::Arg::I(power)],
                },
            ] => (*attack, *power),
            _ => return Err(EngineRefusal::MalformedArgs("strangle program")),
        }
    } else {
        match ctx.spec.identity.upgrade {
            0 => (8, 2),
            1 => (10, 3),
            _ => return Err(EngineRefusal::MalformedArgs("strangle program")),
        }
    };
    let exact_program = match ctx.spec.row.steps {
        [
            crate::content_tables::Step {
                kind: StepKind::Attack,
                args:
                    [
                        crate::content_tables::Arg::I(attack),
                        crate::content_tables::Arg::I(1),
                    ],
            },
            crate::content_tables::Step {
                kind: StepKind::Strangle,
                args: [crate::content_tables::Arg::I(power)],
            },
        ] => *attack == expected_attack && *power == expected_power,
        _ => false,
    };
    if !(matches!(ctx.spec.identity.id, CardId::Strangle) || mad_science_choking) || !exact_program
    {
        return Err(EngineRefusal::MalformedArgs("strangle program"));
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount == expected_power => {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("strangle"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("strangle")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    let frozen = crate::engine::play::active_card_target_identity(ctx.source_uid)
        .ok_or(EngineRefusal::ContinuationNotModeled)?
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if monster.powers.value(PowerId::Strangle) < 0 {
        return Err(EngineRefusal::MalformedArgs("strangle amount"));
    }
    if ctx.state.history.over || monster.hp <= 0 || (monster.slot, monster.uid) != frozen {
        return Ok(());
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Strangle,
        MiseryToken::Strangle,
        amount,
        ctx.events,
    )
}

/// `("stratagem", n)` — StratagemPower, a shuffle listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — an over-gated additive Intensity stack
/// on the owner scalar (`PowerCmd::Apply` is entry-gated by IsOverOrEnding).
///
/// [`crate::engine::draw`] represents the synchronous auto-take path after a
/// completed reshuffle. Admission refuses every reachable larger pile because
/// it requires the out-of-scope selection continuation.
pub(crate) fn stratagem(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "stratagem",
        PowerId::Stratagem,
        &[(CardId::Stratagem, 0, 1), (CardId::Stratagem, 1, 1)],
    )
}

/// `("strength_enemy", 1)` — positive Strength onto FIGHT_ME's chosen enemy.
///
/// Current v0.111.0 IL: `FightMe/<OnPlay>d__6::MoveNext` **0x39e360** awaits
/// the two-hit attack, applies 3/4 owner Strength, then applies `+1` to the
/// original chosen enemy. Python `_run_steps_inner` (frozen, deleted #2827) implements that
/// non-negative Type-1 enemy arm as a bare attribute add: it bypasses Artifact,
/// Lamp, and the Type-2 AfterPowerAmountChanged walk. A lethal hit on a
/// Stock-bearing Axebot replaces the target with a fresh Creature; the Apply
/// still addresses the dead original and grants nothing
/// ([`stock_replaced_target_is_current`], #3326).
///
/// The `+1` is `PowerCmd.Apply<StrengthPower>` at IL_01b9, and `<Apply>d__1`1`
/// (`0x3ef988`) returns at `IsEnding` (IL_0025-002a): the gate is the shared
/// IsEnding projection, not `history.over` (#3515).
pub(crate) fn strength_enemy(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(
        (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args),
        (CardId::FightMe, 0 | 1, [CompiledArg::I(1)])
    ) {
        return Err(EngineRefusal::MalformedArgs("strength_enemy"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !stock_replaced_target_is_current(
        ctx.state,
        ctx.source_uid,
        target,
        "Fight Me Axebot Stock target identity",
    )? {
        return Ok(());
    }
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) || monster.hp <= 0 {
        return Ok(());
    }
    let updated =
        crate::engine::damage::checked_monster_strength_successor(monster, 1, "monster strength")?;
    let uid = monster.uid;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    // A player card: `Malaise/<OnPlay>d__9::MoveNext` `0x3ab428`
    // IL_00fc-IL_0109 is the shape every `OnPlay` Strength application takes,
    // passing `card.Owner.Player.Creature`.
    crate::engine::damage::write_monster_strength(
        &mut ctx.state.monsters_mut()[target],
        updated,
        crate::hot::Applier::Player,
        upkeep,
    );
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Monster(uid),
        PowerId::Strength,
        updated,
    );
    Ok(())
}

/// `("subroutine", n)` — Energy on later owned Power plays.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack; applied after
/// this Power play's BeforeCardPlayed latch, so the casting play grants nothing
/// (the Afterimage/Storm latch shape).
/// [`crate::engine::play`] freezes the pre-body amount for Power cards and
/// grants it after the casting body's ordinary player-power walk completes.
pub(crate) fn subroutine(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "subroutine",
        PowerId::Subroutine,
        &[(CardId::Subroutine, 0, 1), (CardId::Subroutine, 1, 1)],
    )
}

/// `("summon_next_turn", n)` — one summed Osty summon at the next player turn.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Invoke/<OnPlay>d__5::MoveNext` RVA `0x3a7958` applies
/// SummonNextTurnPower 2/3 before EnergyNextTurnPower 2/3.
/// `SummonNextTurnPower/<AfterPlayerTurnStart>d__6::MoveNext` RVA `0x346ecc`
/// gates on the nonzero turn-start snapshot, passes its live Amount to one
/// `OstyCmd::Summon`, then removes itself. [`crate::engine::turn`] owns that
/// exact snapshot/live-read/summon/remove reader.
/// Python `_run_steps_inner` (frozen, deleted #2827) carries the matching keyed writer.
///
/// This first body also preflights Invoke's trailing EnergyNextTurn write.
/// No listener runs between these two direct scalar writes, so the later
/// shared step cannot fail after this mutation; malformed rows and both
/// possible additions therefore refuse before either power or event changes.
pub(crate) fn summon_next_turn(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Invoke, 0, [CompiledArg::I(2)]) => 2,
        (CardId::Invoke, 1, [CompiledArg::I(3)]) => 3,
        _ => return Err(EngineRefusal::MalformedArgs("summon_next_turn")),
    };
    let expected = i64::from(amount);
    let exact_row =
        crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            == Some(ctx.spec.row);
    let exact_source_program = match ctx.catalog.steps(ctx.spec) {
        [summon, energy] => {
            summon.kind == StepKind::SummonNextTurn
                && ctx.catalog.args(summon.args) == [CompiledArg::I(expected)]
                && energy.kind == StepKind::EnergyNextTurn
                && ctx.catalog.args(energy.args) == [CompiledArg::I(expected)]
        }
        _ => false,
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || !exact_row
        || !exact_source_program
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "summon_next_turn exact Invoke row",
        ));
    }

    let mut source = None;
    let mut matches = 0usize;
    for pile in PileId::ALL {
        for card in ctx.state.piles.get(pile).as_slice() {
            if card.uid == ctx.source_uid {
                matches += 1;
                source = Some(*card);
            }
        }
    }
    if matches != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches,
        });
    }
    let source = source.expect("one exact source was counted");
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "summon_next_turn physical source",
        ));
    }
    // Both native PowerCmd.Apply calls publish their own serial
    // AfterPowerAmountChanged walk. The represented listeners ignore these
    // player-owned Type-1 powers, but an absent/duplicate token cannot be
    // treated as that proved inert walk. Nothing between the two writes can
    // mutate this order, so one preflight authenticates both snapshots.
    if !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "Invoke after-power-amount-changed listener order",
        ));
    }

    let prior_summon = ctx.state.powers.value(PowerId::SummonNextTurn);
    let summon = prior_summon
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("summon next turn"))?;
    let _energy_next_turn = ctx
        .state
        .powers
        .value(PowerId::EnergyNextTurn)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("energy next turn"))?;
    if prior_summon <= 0
        && !ctx
            .state
            .register_after_player_turn_start(PowerId::SummonNextTurn)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-player-turn-start listener order",
        ));
    }
    ctx.state
        .powers
        .set(PowerId::SummonNextTurn, SlotWire::Int, summon);
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::SummonNextTurn,
        summon,
    );
    Ok(())
}

/// `("temp_dexterity", n)` — the shared temporary-Dexterity net modifier
/// (AnticipatePower and SpeedPotionPower).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a plain additive stack on the separate
/// net-modifier scalar, removed at the owner's side end.
///
pub(crate) fn temp_dexterity(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "temp_dexterity",
        PowerId::TempDexterity,
        &[(CardId::Anticipate, 0, 2), (CardId::Anticipate, 1, 4)],
    )
}

/// `("temp_strength_enemy", n)` — the negative temporary-Strength wrapper
/// used by Mangle, Dark Shackles and Enfeebling Touch.
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…`):
/// `Mangle/<OnPlay>d__6::MoveNext` **0x3ab670** applies the 10/15 wrapper
/// after its 20/26 attack; `DarkShackles/<OnPlay>d__8::MoveNext` **0x396954**
/// applies 9/15 directly. (Both were cited here as `d__8` at `0x3a8f90` and
/// `0x3dfbcc`; neither RVA is a MethodDef in this build, so those numbers were
/// carried over from an earlier one. Re-derived 2026-09-15 — #2483.) The current
/// Python dispatcher `_run_steps_inner` (frozen Python, deleted #2827) calls `_apply_temp_strength_enemy`, whose card-sourced outer wrapper and nested signed Strength write
/// are represented atomically by [`crate::steps::shared::apply_temp_strength_enemy`].
///
/// Enfeebling Touch joins the same carrier set. Its independent target/debuff
/// lifecycle review is done: `EnfeeblingTouch/<OnPlay>d__8::MoveNext` RVA
/// **0x39bb64** is the same `cardPlay.Target` assert, presentation
/// `TriggerAnim('Cast')`, and single
/// `Apply<EnfeeblingTouchPower>(cardPlay.Target, StrengthLoss.BaseValue,
/// Owner.Creature, card, false)` as Dark Shackles' **0x396954**, with
/// `EnfeeblingTouchPower::.ctor` RVA `0xa20b5`, `DarkShacklesPower::.ctor` RVA
/// `0xa147d` and `ManglePower::.ctor` RVA `0xa491d` all delegating to the same
/// `TemporaryStrengthPower::.ctor` under `get_IsPositive` false — so
/// `TemporaryStrengthPower::get_Sign` RVA `0xa9add` is `-1` and
/// `get_InternallyAppliedPower` RVA `0xa9ad3` is `StrengthPower` for all three.
/// The only distinguishing keyword is its static Ethereal, whose lifecycle the
/// shared play/turn-end layer owns (see
/// `engine::admission::static_ethereal_body_program_is_supported`).
pub(crate) fn temp_strength_enemy(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::DarkShackles, 0, [CompiledArg::I(9)]) => 9,
        (CardId::DarkShackles, 1, [CompiledArg::I(15)]) => 15,
        (CardId::EnfeeblingTouch, 0, [CompiledArg::I(8)]) => 8,
        (CardId::EnfeeblingTouch, 1, [CompiledArg::I(11)]) => 11,
        (CardId::Mangle, 0, [CompiledArg::I(10)]) => 10,
        (CardId::Mangle, 1, [CompiledArg::I(15)]) => 15,
        _ => return Err(EngineRefusal::MalformedArgs("temp_strength_enemy")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let current = if matches!(ctx.spec.identity.id, CardId::Mangle) {
        stock_replaced_target_is_current(
            ctx.state,
            ctx.source_uid,
            target,
            "Mangle Axebot Stock target identity",
        )?
    } else {
        crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target)
    };
    if !current {
        return Ok(());
    }
    let model = crate::steps::shared::temp_strength_enemy_model(ctx.spec.identity.id)?;
    crate::steps::shared::apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)
}

/// Whether the roster entry at `target` is still the Creature a Mangle or
/// Fight Me play captured, across a possible Axebot Stock respawn (#3326).
///
/// v0.111.0 (`sts2.dll` SHA-256 `9cb4f1ad…12b4`):
///
/// * `StockPower/<AfterDeath>d__4::MoveNext` RVA `0x346280` returns unless
///   the dead creature is the power's Owner (IL_002d-IL_003b) and `Amount > 0`
///   (IL_0040-IL_0049). It then builds a FRESH `Axebot` model
///   (`ModelDb.Monster<Axebot>().ToMutable()`, IL_004e-IL_005d), sets
///   `StockAmount = Amount - 1` (IL_0065-IL_006e), and awaits
///   `CreatureCmd.Add(model, CombatState, Owner.Side, Owner.SlotName)`
///   (IL_0073-IL_0090): a new Creature in the dead owner's slot, with the
///   next creation uid ([`crate::engine::monsters::respawn_axebot`] writes
///   `allocate_creature_uid`). `.mcr` target ids follow that creation order,
///   never the slot, so the replacement is a different target.
/// * `FightMe/<OnPlay>d__6::MoveNext` RVA `0x39e360` passes
///   `cardPlay.Target` to the two-hit attack (IL_0077-IL_0082), then to
///   `PowerCmd.Apply<StrengthPower>(Target, EnemyStrength, Owner.Creature,
///   this, false)` (IL_0186-IL_01b9). `Mangle/<OnPlay>d__6::MoveNext` RVA
///   `0x3ab670` likewise attacks `cardPlay.Target` (IL_005d-IL_0068) and then
///   applies `ManglePower` to the same `cardPlay.Target` (IL_010f-IL_013c).
///   Neither re-reads the roster: the Apply receives the ORIGINAL Creature.
/// * The original is gone from combat by the time the attack's await
///   returns: `CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext`
///   RVA `0x3ebe90` runs the AfterDeath walk (Stock's respawn) and then, for
///   an Enemies member not performing a move, calls
///   `CombatState::RemoveCreature(creature, true)` (IL_04c8-IL_04d5), whose
///   `unsetCombatState` arm (RVA `0x1371d0` IL_009a-IL_009f) nulls
///   `creature.CombatState`. `Creature::get_CanReceivePowers` RVA `0x11d2b5`
///   returns false for a null CombatState (IL_0001-IL_000a), and
///   ``PowerCmd/<Apply>d__1`1::MoveNext`` RVA `0x3ef988` returns null at
///   IL_0039-IL_004e when `target.CanReceivePowers` is false.
///
/// So after a lethal Stock hit the trailing power lands on nobody: not the
/// corpse, and never the replacement. The play froze the target's
/// `(slot, uid)` ([`crate::engine::play::active_card_target_identity`]), and
/// the replacement's fresh uid fails that comparison, so the step no-ops.
/// Without a frozen identity (a non-CardPlay or private-test caller) a live
/// Stock Axebot is still indistinguishable from its replacement, so that
/// composition refuses by `reason` instead.
pub(crate) fn stock_replaced_target_is_current(
    state: &crate::hot::HotState,
    source_uid: u32,
    target: usize,
    reason: &'static str,
) -> Result<bool, EngineRefusal> {
    if !matches!(
        crate::engine::play::active_card_target_identity(source_uid),
        Some(Some(_))
    ) && post_attack_stock_target_identity_is_unrepresentable(state)
    {
        return Err(EngineRefusal::MalformedArgs(reason));
    }
    Ok(crate::engine::play::active_card_target_is_current(
        state, source_uid, target,
    ))
}

/// Whether a post-attack target object cannot be reconstructed.
///
/// Native Debilitate, Knockdown and Heirloom Hammer retain the original
/// Creature across the attack await. A lethal hit on a Stock-bearing Axebot
/// replaces that object in the same roster slot. Those bodies refuse the
/// composition rather than apply a later command to the replacement Axebot;
/// Mangle and Fight Me are modeled by [`stock_replaced_target_is_current`].
pub(crate) fn post_attack_stock_target_identity_is_unrepresentable(
    state: &crate::hot::HotState,
) -> bool {
    state.monsters.iter().any(|monster| {
        monster.kind == MonsterKind::Axebot
            && monster.hp > 0
            && monster.powers.value(PowerId::Stock) > 0
    })
}

/// `("the_gambit", n)` — die on the next positive powered HP-loss result.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage::monster_attack_player`] removes the power and
/// resolves the player death after the hit result, even when player Thorns
/// killed the final attacker first. Zero and fully blocked results preserve it.
pub(crate) fn the_gambit(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "the_gambit",
        PowerId::TheGambit,
        &[(CardId::TheGambit, 0, 1), (CardId::TheGambit, 1, 1)],
    )
}

/// `("thorns", n)` — retaliate `n` per powered attack hit received.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
/// [`crate::engine::damage::monster_attack_player`] freezes the incoming
/// snapshot, emits blockable unpowered retaliation before block, commits the
/// in-flight hit even if that kills the final attacker, and cancels later hits.
pub(crate) fn thorns(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "thorns",
        PowerId::Thorns,
        &[
            (CardId::Abrasive, 0, 4),
            (CardId::Abrasive, 1, 6),
            (CardId::Caltrops, 0, 3),
            (CardId::Caltrops, 1, 5),
        ],
    )
}

/// `("tracking", n)` — a damage multiplier against Weak targets.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — validates the stored and applied amounts
/// (malformed values refuse), then an over-gated additive stack.
///
///
/// v0.111.0 `Tracking/<OnPlay>` RVA `0x3c42c0` awaits `PowerCmd.Apply<TrackingPower>` at IL_00c3.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn tracking(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let allowed = [(CardId::Tracking, 0, 50), (CardId::Tracking, 1, 50)];
    let amount = match ctx.args {
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value)) =>
        {
            *value
        }
        _ => return Err(EngineRefusal::MalformedArgs("tracking")),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("tracking"))?;
    let updated = ctx
        .state
        .powers
        .value(PowerId::Tracking)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("tracking"))?;
    ctx.state
        .powers
        .set(PowerId::Tracking, SlotWire::Int, updated);
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::Tracking,
        updated,
    );
    Ok(())
}

/// Apply Trash to Treasure and register its generated-card listener.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `TrashToTreasure/<OnPlay>d__3::MoveNext` RVA `0x3c4648` IL
/// `0x009e-0x0118` applies one stack after the animation; the frozen Python's
/// matching writer is in `_run_steps_inner` (deleted #2827). The listener's
/// MoveNext RVA `0x349f90` IL `0x0020-0x0053` requires a Status and the exact
/// owner creator, then IL `0x0065-0x0116` draws and Channels one random orb
/// per live Amount in serial order.
pub(crate) fn trash_to_treasure(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("trash_to_treasure program"));
    };
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::TrashToTreasure
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs("trash_to_treasure"));
    }
    apply_exact_ordered_player_power(
        ctx,
        "trash_to_treasure",
        PowerId::TrashToTreasure,
        &[
            (CardId::TrashToTreasure, 0, 1),
            (CardId::TrashToTreasure, 1, 1),
        ],
        FanoutOrder::LocalGenerated,
    )
}

/// `("underworld", n)` — a teammate-damage consumer. **Escalated (#1345
/// triage).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the branch itself refuses without an
/// exact teammate relation, then registers in `after_damage_given_order` on the
/// zero-to-positive edge and stacks.
///
/// Slice 5 carries the represented AfterDamageGiven walk, but Underworld
/// remains impossible without the **teammate relation and its enemy-side-end
/// drain** (`_underworld_enemy_side_end`) — the engine models one player and
/// no teammate.
/// `PowerId::Underworld` is outside
/// [`crate::engine::admission::IMPLEMENTED_PLAYER_POWERS`]; a written-and-
/// never-read power is exactly the wrong-answer class that registry exists to
/// refuse.
/// ESCALATED-ON: power-not-admitted(IMPLEMENTED_PLAYER_POWERS, PowerId::Underworld)
pub(crate) fn underworld(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::Underworld))
}

/// `("unmovable", n)` — double the owner's own card-sourced block gains, count-
/// limited.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
pub(crate) fn unmovable(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "unmovable",
        PowerId::Unmovable,
        &[(CardId::Unmovable, 0, 1), (CardId::Unmovable, 1, 1)],
    )
}

/// `("vigor", n)` — an additive on the owner's next attack plays this turn.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards.
///
pub(crate) fn vigor(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "vigor",
        PowerId::Vigor,
        &[
            (CardId::Patter, 0, 2),
            (CardId::Patter, 1, 3),
            (CardId::Terraforming, 0, 7),
            (CardId::Terraforming, 1, 10),
        ],
    )
}

/// `("wraith_form", n)` — WraithFormPower.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive Intensity stack on the
/// owner scalar, no guards. Its persistent turn-start reader applies the
/// negative Dexterity delta after the hand draw.
pub(crate) fn wraith_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_exact_player_power(
        ctx,
        "wraith_form",
        PowerId::WraithForm,
        &[(CardId::WraithForm, 0, 1), (CardId::WraithForm, 1, 1)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Event, Subject, capability_manifest};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, PileId};
    use crate::ids::{CardId, MonsterKind};

    /// Every kind this file owns a body for, ported or not.
    ///
    /// The dispatch's own completeness and uniqueness are pinned by
    /// `tests/hot_path_contract.rs`; what this list adds is the split between
    /// claimed and escalated, so that porting a kind means moving it into
    /// [`IMPLEMENTED`] in one deliberate edit.
    const OWNED: [StepKind; 102] = [
        StepKind::Accelerant,
        StepKind::Accuracy,
        StepKind::Afterimage,
        StepKind::Aggression,
        StepKind::Arsenal,
        StepKind::Automation,
        StepKind::Barricade,
        StepKind::BiasedCognition,
        StepKind::BlackHole,
        StepKind::Blur,
        StepKind::BorrowedTime,
        StepKind::Buffer,
        StepKind::Burst,
        StepKind::Cacophony,
        StepKind::ChildOfTheStars,
        StepKind::Colossus,
        StepKind::Coolant,
        StepKind::CorrosiveWave,
        StepKind::Corruption,
        StepKind::Countdown,
        StepKind::Cruelty,
        StepKind::DanseMacabre,
        StepKind::DarkEmbrace,
        StepKind::Debilitate,
        StepKind::Demesne,
        StepKind::DemonForm,
        StepKind::DevourLife,
        StepKind::Doom,
        StepKind::DrawNextTurn,
        StepKind::EchoForm,
        StepKind::Entropy,
        StepKind::Envenom,
        StepKind::Fasten,
        StepKind::FeelNoPain,
        StepKind::FlameBarrier,
        StepKind::Focus,
        StepKind::Foregone,
        StepKind::FreeAttack,
        StepKind::FreeEthereal,
        StepKind::FreePower,
        StepKind::FreeSkill,
        StepKind::Genesis,
        StepKind::Hailstorm,
        StepKind::HelloWorld,
        StepKind::Hellraiser,
        StepKind::InfiniteBlades,
        StepKind::Intangible,
        StepKind::Iteration,
        StepKind::Juggernaut,
        StepKind::Knockdown,
        StepKind::Loop,
        StepKind::MachineLearning,
        StepKind::MadScienceCurious,
        StepKind::Mayhem,
        StepKind::MonarchsGaze,
        StepKind::NecroMastery,
        StepKind::Neurosurge,
        StepKind::NoDraw,
        StepKind::NoxiousFumes,
        StepKind::Oblivion,
        StepKind::OneTwoPunch,
        StepKind::Orbit,
        StepKind::Pagestorm,
        StepKind::PaleBlueDot,
        StepKind::Panache,
        StepKind::PhantomBlades,
        StepKind::PillarOfCreation,
        StepKind::Plating,
        StepKind::Poison,
        StepKind::PrepTime,
        StepKind::Pyre,
        StepKind::Rage,
        StepKind::ReaperForm,
        StepKind::Reflect,
        StepKind::RetainHand,
        StepKind::RollingBoulder,
        StepKind::Rupture,
        StepKind::SealedThrone,
        StepKind::SentryMode,
        StepKind::SignalBoost,
        StepKind::SleightOfFlesh,
        StepKind::Smokestack,
        StepKind::Sneaky,
        StepKind::Speedster,
        StepKind::SpiritOfAsh,
        StepKind::Stampede,
        StepKind::StarNextTurn,
        StepKind::Strangle,
        StepKind::Stratagem,
        StepKind::StrengthEnemy,
        StepKind::Subroutine,
        StepKind::SummonNextTurn,
        StepKind::TempDexterity,
        StepKind::TempStrengthEnemy,
        StepKind::TheGambit,
        StepKind::Thorns,
        StepKind::Tracking,
        StepKind::TrashToTreasure,
        StepKind::Underworld,
        StepKind::Unmovable,
        StepKind::Vigor,
        StepKind::WraithForm,
    ];

    /// The wave's honesty property: the manifest claims nothing this file
    /// cannot actually play, and claims everything it can.
    #[test]
    fn the_manifest_claims_nothing_this_family_has_not_ported() {
        let manifest = capability_manifest();
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                assert!(
                    manifest.steps.contains(&kind),
                    "{:?} has a body here but the manifest does not claim it",
                    kind.as_str()
                );
            } else {
                assert!(
                    !manifest.steps.contains(&kind),
                    "{:?} is escalated but the manifest claims it",
                    kind.as_str()
                );
            }
        }
        for kind in IMPLEMENTED {
            assert!(
                OWNED.contains(kind),
                "{:?} is not this file's",
                kind.as_str()
            );
        }
        assert_eq!(
            IMPLEMENTED.len(),
            101,
            "poison, #1394's Focus/Orbit trio, #1406's player-power waves, Doom, #1487 replay powers, #1561 Free*/Star/generated/cost/Retain/Stampede/Foregone powers, #1751 TempStrengthEnemy/StrengthEnemy/Strangle, #1884 Hellraiser, Creative AI's Trash-to-Treasure leaf, and #3427 Mad Science's Curious rider"
        );
    }

    #[test]
    fn panache_instanced_apply_events_are_row_local_and_share_one_uid_order() {
        let mut builder = CatalogBuilder::new();
        let base_atom = builder
            .intern(CardIdentity {
                id: CardId::Panache,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let upgraded_atom = builder
            .intern(CardIdentity {
                id: CardId::Panache,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let mut events = Vec::new();

        for (uid, atom, amount) in [(1, base_atom, 10), (2, upgraded_atom, 14)] {
            let spec = *catalog.spec(atom).unwrap();
            panache(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(amount)],
                events: &mut events,
            })
            .unwrap();
        }

        assert_eq!(state.powers.value(PowerId::Panache), 24);
        assert_eq!(
            state
                .fanouts
                .panache_instances()
                .map(|row| (row.uid, row.amount, row.cards_left, row.already_applied))
                .collect::<Vec<_>>(),
            [(0, 10, 5, false), (1, 14, 5, false)]
        );
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PowerChanged {
                        power: PowerId::Panache,
                        amount,
                        ..
                    } => Some(*amount),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [10, 14]
        );
    }

    #[test]
    fn foregone_rows_stack_once_in_before_hand_draw_acquisition_order() {
        let mut builder = CatalogBuilder::new();
        let atoms = [0, 1].map(|upgrade| {
            builder
                .intern(CardIdentity {
                    id: CardId::ForegoneConclusion,
                    upgrade,
                    enchantment: None,
                })
                .unwrap()
        });
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        let mut events = Vec::new();
        for (upgrade, atom) in atoms.into_iter().enumerate() {
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };
            foregone(&mut ctx).unwrap();
            assert_eq!(
                ctx.state.powers.value(PowerId::Foregone),
                2 + 3 * upgrade as i32
            );
            assert_eq!(
                ctx.state.fanouts.before_hand_draw_order(),
                [PowerId::Foregone],
                "a restack does not move or duplicate the first-acquisition token"
            );

            let before = ctx.state.clone();
            ctx.args = &[CompiledArg::I(99)];
            assert_eq!(
                foregone(&mut ctx),
                Err(EngineRefusal::MalformedArgs("foregone"))
            );
            assert_eq!(*ctx.state, before);
        }
        assert_eq!(
            events,
            [
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Foregone,
                    amount: 2,
                },
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Foregone,
                    amount: 5,
                },
            ]
        );

        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        let before = state.clone();
        let before_events = events.clone();
        let identity = CardIdentity {
            id: CardId::ForegoneConclusion,
            upgrade: 0,
            enchantment: None,
        };
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        assert_eq!(
            foregone(&mut ctx),
            Err(EngineRefusal::MalformedArgs("foregone listener orders"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn accelerant_rows_stack_the_exact_scalar_and_refuse_operand_drift() {
        for upgrade in 0..=1 {
            let identity = CardIdentity {
                id: CardId::Accelerant,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let exact_args = catalog.args(step.args);
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: exact_args,
                events: &mut events,
            };
            accelerant(&mut ctx).unwrap();
            assert_eq!(
                ctx.state.powers.value(PowerId::Accelerant),
                1 + i32::from(upgrade)
            );

            let before = ctx.state.clone();
            ctx.args = &[CompiledArg::I(9)];
            assert_eq!(
                accelerant(&mut ctx),
                Err(EngineRefusal::MalformedArgs("accelerant"))
            );
            assert_eq!(*ctx.state, before);
        }
    }

    #[test]
    fn necro_mastery_rows_stack_the_exact_live_reader_amount() {
        for upgrade in 0..=1 {
            let identity = CardIdentity {
                id: CardId::NecroMastery,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog
                .steps(&spec)
                .iter()
                .find(|step| step.kind == StepKind::NecroMastery)
                .unwrap();
            let exact_args = catalog.args(step.args);
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.powers.set(PowerId::NecroMastery, SlotWire::Int, 2);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: exact_args,
                events: &mut events,
            };
            necro_mastery(&mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(PowerId::NecroMastery), 3);
            assert_eq!(ctx.events.len(), 1);

            let before = ctx.state.clone();
            ctx.args = &[CompiledArg::I(2)];
            assert_eq!(
                necro_mastery(&mut ctx),
                Err(EngineRefusal::MalformedArgs("necro_mastery"))
            );
            assert_eq!(*ctx.state, before);
        }
    }

    #[test]
    fn necro_mastery_public_play_rolls_back_summon_when_the_later_power_overflows() {
        let identity = CardIdentity {
            id: CardId::NecroMastery,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state
            .powers
            .set(PowerId::NecroMastery, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = Vec::new();
        let action = crate::engine::Action::Play {
            uid: 7,
            target: None,
            selection: crate::engine::SelectionRef::NONE,
        };

        assert_eq!(
            crate::engine::apply_action_into(&state, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow("necro_mastery"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn hello_world_writer_accepts_both_exact_rows_and_preserves_snapshot_across_repeats() {
        let mut builder = CatalogBuilder::new();
        let atoms = [0, 1].map(|upgrade| {
            builder
                .intern(CardIdentity {
                    id: CardId::HelloWorld,
                    upgrade,
                    enchantment: None,
                })
                .unwrap()
        });
        for id in crate::content_tables::HELLO_WORLD_COMMON_POOL_V1101 {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        state.hp = 50;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Defect);
        state.publish_hello_world_generation_pool();
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        for (upgrade, atom) in atoms.into_iter().enumerate() {
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
                args: &[CompiledArg::I(1)],
                events: &mut events,
            };
            hello_world(&mut ctx).unwrap_or_else(|error| panic!("L{upgrade}: {error:?}"));
            assert_eq!(
                ctx.state
                    .hello_world_amount_on_turn_start(ctx.state.powers.value(PowerId::HelloWorld),),
                Some((upgrade * 2) as i32)
            );
            hello_world(&mut ctx).unwrap();
            assert_eq!(
                ctx.state
                    .hello_world_amount_on_turn_start(ctx.state.powers.value(PowerId::HelloWorld)),
                Some((upgrade * 2) as i32)
            );
            ctx.state.freeze_hello_world_amount_on_turn_start();
        }
        assert_eq!(state.powers.value(PowerId::HelloWorld), 4);
        assert_eq!(
            state.fanouts.before_hand_draw_order(),
            [PowerId::HelloWorld]
        );
    }

    #[test]
    fn temp_strength_enemy_carriers_are_exactly_six_public_rows() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps.iter().find_map(|step| {
                    (step.kind == StepKind::TempStrengthEnemy).then(|| {
                        let [Arg::I(amount)] = step.args else {
                            panic!(
                                "{}+{} has a non-scalar TempStrengthEnemy",
                                row.name, row.upgrade
                            );
                        };
                        (row.id, row.upgrade, *amount)
                    })
                })
            })
            .collect();
        assert_eq!(
            carriers,
            [
                (CardId::DarkShackles, 0, 9),
                (CardId::DarkShackles, 1, 15),
                (CardId::EnfeeblingTouch, 0, 8),
                (CardId::EnfeeblingTouch, 1, 11),
                (CardId::Mangle, 0, 10),
                (CardId::Mangle, 1, 15),
            ]
        );

        for (id, upgrade, amount) in carriers {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            let mut target = HotMonster::new(MonsterKind::Toadpole, 50);
            target
                .powers
                .set(PowerId::Strength, crate::powers::SlotWire::Int, 4);
            state.monsters_mut().push(target);
            let before = state.clone();
            let mut events = Vec::new();
            let result = run_card(
                StepKind::TempStrengthEnemy,
                id,
                upgrade,
                &mut state,
                Some(0),
                &[CompiledArg::I(amount)],
                &mut events,
            );
            // Every carrier, Enfeebling Touch included, is now public: all
            // three power classes delegate to the same
            // `TemporaryStrengthPower::.ctor` with `get_IsPositive` false.
            assert_ne!(state, before, "the wrapper must write");
            result.unwrap();
            let amount = i32::try_from(amount).unwrap();
            assert_eq!(
                state.monsters[0].powers.value(PowerId::TempStrength),
                -amount
            );
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                4 - amount
            );
            assert_eq!(events.len(), 2, "outer wrapper then nested Strength");
        }
    }

    #[test]
    fn temp_strength_enemy_rejects_wrong_identity_operand_and_target_atomically() {
        for mutation in 0..3 {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));
            let before = state.clone();
            let mut events = Vec::new();
            let result = match mutation {
                0 => run_card(
                    StepKind::TempStrengthEnemy,
                    CardId::StrikeIronclad,
                    0,
                    &mut state,
                    Some(0),
                    &[CompiledArg::I(9)],
                    &mut events,
                ),
                1 => run_card(
                    StepKind::TempStrengthEnemy,
                    CardId::DarkShackles,
                    0,
                    &mut state,
                    Some(0),
                    &[CompiledArg::I(10)],
                    &mut events,
                ),
                2 => run_card(
                    StepKind::TempStrengthEnemy,
                    CardId::DarkShackles,
                    0,
                    &mut state,
                    None,
                    &[CompiledArg::I(9)],
                    &mut events,
                ),
                _ => unreachable!(),
            };
            assert!(matches!(
                result,
                Err(EngineRefusal::MalformedArgs("temp_strength_enemy"))
                    | Err(EngineRefusal::TargetMismatch { required: true })
            ));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn temp_strength_enemy_terminal_dead_and_overflow_paths_are_atomic() {
        for terminal in [false, true] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));
            if terminal {
                state.history.over = true;
            } else {
                state.monsters_mut()[0].hp = 0;
            }
            let before = state.clone();
            let mut events = Vec::new();
            run_card(
                StepKind::TempStrengthEnemy,
                CardId::DarkShackles,
                0,
                &mut state,
                Some(0),
                &[CompiledArg::I(9)],
                &mut events,
            )
            .unwrap();
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 50);
        target.powers.set(
            PowerId::TempStrength,
            crate::powers::SlotWire::Int,
            i32::MIN,
        );
        state.monsters_mut().push(target);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_card(
                StepKind::TempStrengthEnemy,
                CardId::DarkShackles,
                0,
                &mut state,
                Some(0),
                &[CompiledArg::I(9)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("temporary monster strength"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
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
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
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
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn run_card(
        kind: StepKind,
        id: CardId,
        upgrade: u8,
        state: &mut HotState,
        target: Option<usize>,
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    /// #3057 witness: a second live object of an Instanced power Rust keeps
    /// as one scalar refuses by name, with the first object out of step
    /// (Cacophony mid-countdown, Orbit mid-four-energy window, Rolling
    /// Boulder already grown), and leaves the state and events untouched.
    #[test]
    fn second_live_instanced_power_object_refuses_by_name_out_of_step() {
        let cases: [(StepKind, CardId, i64, PowerId); 3] = [
            (
                StepKind::Cacophony,
                CardId::Cacophony,
                66,
                PowerId::Cacophony,
            ),
            (StepKind::Orbit, CardId::Orbit, 1, PowerId::Orbit),
            (
                StepKind::RollingBoulder,
                CardId::RollingBoulder,
                5,
                PowerId::RollingBoulder,
            ),
        ];
        for (kind, id, amount, power) in cases {
            for upgrade in 0..=1u8 {
                let amount = match (id, upgrade) {
                    (CardId::Cacophony, 1) => 99,
                    (CardId::RollingBoulder, 1) => 10,
                    _ => amount,
                };
                let mut state = HotState::at_defaults();
                state.hp = 50;
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 99));
                let args = [CompiledArg::I(amount)];
                let mut events = Vec::new();
                run_card(kind, id, upgrade, &mut state, None, &args, &mut events).unwrap();
                assert_eq!(state.powers.value(power), amount as i32, "{id:?}");

                // Put the first object out of step with a fresh one.
                match power {
                    PowerId::Cacophony => state.fanouts.set_cacophony_left(20),
                    PowerId::Orbit => state.orbs.set_orbit_counters(2, 0),
                    _ => state.powers.set(power, SlotWire::Int, amount as i32 + 5),
                }
                let before = state.clone();
                let sentinel = Event::TurnBegan { turn: 7 };
                let mut events = vec![sentinel];
                assert_eq!(
                    run_card(kind, id, upgrade, &mut state, None, &args, &mut events),
                    Err(EngineRefusal::PowerRestackNotModeled(power)),
                    "{id:?}+{upgrade}"
                );
                assert_eq!(state, before, "{id:?} refusal must not mutate");
                assert_eq!(events, [sentinel]);

                // A malformed row still reports its own refusal first.
                let bad = [CompiledArg::I(amount + 1)];
                assert_eq!(
                    run_card(kind, id, upgrade, &mut state, None, &bad, &mut Vec::new()),
                    Err(EngineRefusal::MalformedArgs(kind.as_str()))
                );

                // Native Apply builds no object while combat is ending, so
                // the ending-gated writers stay silent rather than refuse.
                if power != PowerId::Orbit {
                    let mut ending = before.clone();
                    ending.monsters_mut()[0].hp = 0;
                    let ending_before = ending.clone();
                    run_card(kind, id, upgrade, &mut ending, None, &args, &mut Vec::new()).unwrap();
                    assert_eq!(ending, ending_before, "{id:?} ending gate");
                }
            }
        }
    }

    fn invoke_fixture(upgrade: u8) -> (HotState, Catalog) {
        let identity = CardIdentity {
            id: CardId::Invoke,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 99));
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 41,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        (state, catalog)
    }

    fn run_invoke_program(
        state: &mut HotState,
        catalog: &Catalog,
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id: CardId::Invoke,
            upgrade,
            enchantment: None,
        };
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        for step in catalog.steps(&spec) {
            let mut ctx = StepCtx {
                state,
                catalog,
                spec: &spec,
                source_uid: 41,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events,
            };
            crate::steps::apply_step(step.kind, &mut ctx)?;
        }
        Ok(())
    }

    fn sentry_mode_fixture(upgrade: u8) -> (HotState, Catalog) {
        let sentry = CardIdentity {
            id: CardId::SentryMode,
            upgrade,
            enchantment: None,
        };
        let gaze = CardIdentity {
            id: CardId::SweepingGaze,
            upgrade: 0,
            enchantment: None,
        };
        let shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let sentry_atom = builder.intern(sentry).unwrap();
        builder.intern(gaze).unwrap();
        builder.intern(shiv).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.next_card_uid = 100;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 41,
            atom: sentry_atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        (state, catalog)
    }

    fn apply_sentry_fixture(
        state: &mut HotState,
        catalog: &Catalog,
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id: CardId::SentryMode,
            upgrade,
            enchantment: None,
        };
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 41,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events,
        };
        apply_sentry_mode_foundation(&mut ctx)
    }

    #[test]
    fn sentry_mode_rows_are_exactly_l0_l1_one_stack_and_publicly_dispatched() {
        let rows: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::SentryMode)
            })
            .map(|row| {
                let [step] = row.steps else {
                    panic!("Sentry Mode must remain a one-step program");
                };
                let [Arg::I(amount)] = step.args else {
                    panic!("Sentry Mode must remain scalar");
                };
                (row.id, row.upgrade, *amount)
            })
            .collect();
        assert_eq!(
            rows,
            [(CardId::SentryMode, 0, 1), (CardId::SentryMode, 1, 1),]
        );
        assert!(IMPLEMENTED.contains(&StepKind::SentryMode));
        assert!(capability_manifest().steps.contains(&StepKind::SentryMode));

        let (mut state, catalog) = sentry_mode_fixture(0);
        let spec = *catalog
            .spec(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::SentryMode,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap(),
            )
            .unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 41,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };
        sentry_mode(&mut ctx).unwrap();
        assert_eq!(ctx.state.powers.value(PowerId::SentryMode), 1);
        assert_eq!(
            ctx.state.fanouts.before_hand_draw_order(),
            [PowerId::SentryMode]
        );
    }

    #[test]
    fn sentry_mode_private_writer_stacks_and_preserves_peer_acquisition_order() {
        for upgrade in [0, 1] {
            let (mut state, catalog) = sentry_mode_fixture(upgrade);
            state.powers.set(PowerId::InfiniteBlades, SlotWire::Int, 2);
            assert!(
                state
                    .fanouts
                    .set_before_hand_draw_order(&[PowerId::InfiniteBlades])
            );
            let mut events = Vec::new();

            apply_sentry_fixture(&mut state, &catalog, upgrade, &mut events).unwrap();
            apply_sentry_fixture(&mut state, &catalog, upgrade, &mut events).unwrap();

            assert_eq!(state.powers.value(PowerId::SentryMode), 2);
            assert_eq!(
                state.fanouts.before_hand_draw_order(),
                [PowerId::InfiniteBlades, PowerId::SentryMode]
            );
            assert_eq!(
                events,
                [
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::SentryMode,
                        amount: 1,
                    },
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::SentryMode,
                        amount: 2,
                    },
                ]
            );
        }
    }

    #[test]
    fn before_hand_draw_validator_preserves_the_complete_dynamic_set_contract() {
        let represented = [
            PowerId::SentryMode,
            PowerId::SpectrumShift,
            PowerId::HelloWorld,
            PowerId::Foregone,
        ];
        let mut state = HotState::at_defaults();
        for power in represented {
            state.powers.set(power, SlotWire::Int, 1);
        }
        assert!(state.fanouts.set_before_hand_draw_order(&represented));
        assert!(before_hand_draw_power_order_is_exact(&state));

        let mut other_peers = HotState::at_defaults();
        for power in [PowerId::CreativeAi, PowerId::InfiniteBlades] {
            other_peers.powers.set(power, SlotWire::Int, 1);
        }
        assert!(
            other_peers
                .fanouts
                .set_before_hand_draw_order(&[PowerId::InfiniteBlades, PowerId::CreativeAi])
        );
        assert!(before_hand_draw_power_order_is_exact(&other_peers));

        let mut missing = state.clone();
        assert!(
            missing
                .fanouts
                .set_before_hand_draw_order(&represented[..represented.len() - 1])
        );
        assert!(!before_hand_draw_power_order_is_exact(&missing));

        let mut duplicate = state.clone();
        let mut duplicate_order = represented;
        duplicate_order[3] = represented[0];
        assert!(
            duplicate
                .fanouts
                .set_before_hand_draw_order(&duplicate_order)
        );
        assert!(!before_hand_draw_power_order_is_exact(&duplicate));

        let mut orphan = state.clone();
        orphan.powers.set(represented[0], SlotWire::Int, 0);
        assert!(!before_hand_draw_power_order_is_exact(&orphan));

        let mut alien = state.clone();
        let mut alien_order = represented;
        alien_order[3] = PowerId::Entropy;
        assert!(alien.fanouts.set_before_hand_draw_order(&alien_order));
        assert!(!before_hand_draw_power_order_is_exact(&alien));

        for (wire, value) in [(SlotWire::Bool, 1), (SlotWire::Int, -1)] {
            let mut malformed = state.clone();
            malformed.powers.set(PowerId::Foregone, wire, value);
            assert!(!before_hand_draw_power_order_is_exact(&malformed));
        }
    }

    #[test]
    fn sentry_mode_writer_accepts_every_type_one_peer_acquisition_permutation() {
        let permutations = [
            [PowerId::Vicious, PowerId::Shroud, PowerId::SleightOfFlesh],
            [PowerId::Vicious, PowerId::SleightOfFlesh, PowerId::Shroud],
            [PowerId::Shroud, PowerId::Vicious, PowerId::SleightOfFlesh],
            [PowerId::Shroud, PowerId::SleightOfFlesh, PowerId::Vicious],
            [PowerId::SleightOfFlesh, PowerId::Vicious, PowerId::Shroud],
            [PowerId::SleightOfFlesh, PowerId::Shroud, PowerId::Vicious],
        ];
        for order in permutations {
            let (mut state, catalog) = sentry_mode_fixture(0);
            for power in order {
                state.powers.set(power, SlotWire::Int, 1);
            }
            assert!(state.fanouts.set_after_power_amount_changed_order(&order));
            let mut events = vec![Event::TurnBegan { turn: 86 }];

            apply_sentry_fixture(&mut state, &catalog, 0, &mut events).unwrap();

            assert_eq!(state.powers.value(PowerId::SentryMode), 1);
            assert_eq!(state.fanouts.after_power_amount_changed_order(), order);
            assert_eq!(events[0], Event::TurnBegan { turn: 86 });
            assert_eq!(events.len(), 2);
        }
    }

    #[test]
    fn sentry_mode_writer_refuses_malformed_type_one_peer_projection_atomically() {
        for malformed in 0..4 {
            let (mut state, catalog) = sentry_mode_fixture(0);
            match malformed {
                0 => state.powers.set(PowerId::Vicious, SlotWire::Int, 1),
                1 => {
                    state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
                    assert!(state.fanouts.set_after_power_amount_changed_order(&[
                        PowerId::Vicious,
                        PowerId::Vicious,
                    ]));
                }
                2 => assert!(
                    state
                        .fanouts
                        .set_after_power_amount_changed_order(&[PowerId::SwordSage])
                ),
                3 => assert!(
                    state
                        .fanouts
                        .set_after_power_amount_changed_order(&[PowerId::Vicious])
                ),
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 87 }];
            let before_events = events.clone();

            assert_eq!(
                apply_sentry_fixture(&mut state, &catalog, 0, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "Sentry Mode AfterPowerAmountChanged order"
                ))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        for power in [PowerId::Vicious, PowerId::Shroud, PowerId::SleightOfFlesh] {
            for (wire, value) in [(SlotWire::Bool, 1), (SlotWire::Int, -1), (SlotWire::Int, 0)] {
                let (mut state, catalog) = sentry_mode_fixture(0);
                state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                    key: power,
                    wire,
                    value,
                }])
                .unwrap();
                if value != 0 {
                    assert!(state.fanouts.set_after_power_amount_changed_order(&[power]));
                }
                let before = state.clone();
                let mut events = vec![Event::TurnBegan { turn: 87 }];
                let before_events = events.clone();

                assert_eq!(
                    apply_sentry_fixture(&mut state, &catalog, 0, &mut events),
                    Err(EngineRefusal::MalformedArgs(
                        "Sentry Mode AfterPowerAmountChanged order"
                    ))
                );
                assert_eq!(state, before);
                assert_eq!(events, before_events);
            }
        }
    }

    #[test]
    fn sentry_mode_writer_refuses_malformed_power_order_and_overflow_atomically() {
        for malformed in 0..8 {
            let (mut state, catalog) = sentry_mode_fixture(0);
            match malformed {
                0 => state.powers.set(PowerId::SentryMode, SlotWire::Int, -1),
                1 => state.powers.set(PowerId::SentryMode, SlotWire::Bool, 1),
                2 => state.powers.set(PowerId::SentryMode, SlotWire::Int, 1),
                3 => assert!(
                    state
                        .fanouts
                        .set_before_hand_draw_order(&[PowerId::SentryMode])
                ),
                4 => state.powers.set(PowerId::HelloWorld, SlotWire::Int, 1),
                5 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::Foregone,
                        wire: SlotWire::Bool,
                        value: 0,
                    }])
                    .unwrap();
                }
                6 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::CreativeAi,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                }
                7 => {
                    state.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
                        key: PowerId::HelloWorld,
                        wire: SlotWire::Int,
                        value: 0,
                    }])
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 88 }];
            let before_events = events.clone();
            assert_eq!(
                apply_sentry_fixture(&mut state, &catalog, 0, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "Sentry Mode BeforeHandDraw order"
                ))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        let (mut state, catalog) = sentry_mode_fixture(0);
        state
            .powers
            .set(PowerId::SentryMode, SlotWire::Int, i32::MAX);
        assert!(
            state
                .fanouts
                .set_before_hand_draw_order(&[PowerId::SentryMode])
        );
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 89 }];
        let before_events = events.clone();
        assert_eq!(
            apply_sentry_fixture(&mut state, &catalog, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("Sentry Mode"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        let (mut state, catalog) = sentry_mode_fixture(0);
        state.history.over = true;
        let before = state.clone();
        let mut events = Vec::new();
        apply_sentry_fixture(&mut state, &catalog, 0, &mut events).unwrap();
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn sentry_mode_writer_authenticates_context_and_unique_play_source() {
        for malformed in 0..9 {
            let (mut state, catalog) = sentry_mode_fixture(0);
            if malformed == 1 {
                let source = state.piles.get_mut(PileId::Play).make_mut().pop().unwrap();
                state.piles.get_mut(PileId::Hand).make_mut().push(source);
            } else if malformed == 2 {
                let source = state.piles.get(PileId::Play).as_slice()[0];
                state.piles.get_mut(PileId::Discard).make_mut().push(source);
            } else if malformed == 3 {
                state.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_LEGACY;
            } else if malformed == 8 {
                state.piles.get_mut(PileId::Play).make_mut()[0].atom = catalog
                    .atom(&CardIdentity {
                        id: CardId::SweepingGaze,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            let identity = CardIdentity {
                id: CardId::SentryMode,
                upgrade: 0,
                enchantment: None,
            };
            let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
            let mut events = vec![Event::TurnBegan { turn: 90 }];
            let before = state.clone();
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: if malformed == 0 { 999 } else { 41 },
                target: (malformed == 4).then_some(0),
                selection: (malformed == 5).then_some(7),
                x_value: if malformed == 7 { 1 } else { 0 },
                args: if malformed == 6 {
                    &[CompiledArg::I(2)]
                } else {
                    &[CompiledArg::I(1)]
                },
                events: &mut events,
            };
            assert!(apply_sentry_mode_foundation(&mut ctx).is_err());
            assert_eq!(*ctx.state, before);
            assert_eq!(*ctx.events, before_events);
        }
    }

    #[test]
    fn sentry_mode_listener_generates_l0_bottom_and_obeys_hand_cap() {
        for full_hand in [false, true] {
            let (mut state, catalog) = sentry_mode_fixture(0);
            state.powers.set(PowerId::SentryMode, SlotWire::Int, 2);
            assert!(
                state
                    .fanouts
                    .set_before_hand_draw_order(&[PowerId::SentryMode])
            );
            if full_hand {
                let atom = catalog
                    .atom(&CardIdentity {
                        id: CardId::SweepingGaze,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
                state
                    .piles
                    .get_mut(PileId::Hand)
                    .make_mut()
                    .extend((0..10).map(|uid| HotCard {
                        uid,
                        atom,
                        flags: 0,
                    }));
            }

            run_sentry_mode_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();

            let destination = if full_hand {
                PileId::Discard
            } else {
                PileId::Hand
            };
            let generated = state.piles.get(destination).as_slice();
            let generated = &generated[generated.len() - 2..];
            assert_eq!(
                generated.iter().map(|card| card.uid).collect::<Vec<_>>(),
                [100, 101]
            );
            assert!(generated.iter().all(|card| {
                catalog.spec(card.atom).unwrap().identity
                    == CardIdentity {
                        id: CardId::SweepingGaze,
                        upgrade: 0,
                        enchantment: None,
                    }
            }));
            assert_eq!(state.history.owner_generated_cards_combat, 2);
            assert_eq!(state.next_generated_hook_uid, 2);
        }
    }

    #[test]
    fn sentry_mode_loop_boundary_accepts_live_removal_and_current_callbacks_cannot_cause_it() {
        let mut state = HotState::at_defaults();
        assert_eq!(sentry_mode_live_amount(&state), Ok(0));
        state.powers.set(PowerId::SentryMode, SlotWire::Int, 2);
        assert_eq!(sentry_mode_live_amount(&state), Ok(2));
        state.powers.set(PowerId::SentryMode, SlotWire::Int, 0);
        assert_eq!(sentry_mode_live_amount(&state), Ok(0));
        state.powers.set(PowerId::SentryMode, SlotWire::Int, -1);
        assert_eq!(
            sentry_mode_live_amount(&state),
            Err(EngineRefusal::MalformedArgs(
                "Sentry Mode live power amount"
            ))
        );

        // Current generated-card callbacks are the only code that can run
        // between loop boundaries. None writes SentryMode, so the zero path is
        // source-pinned future closure rather than presently triggerable data.
        let cards = include_str!("../engine/cards.rs");
        let callbacks = cards
            .split_once("fn after_local_card_generated(")
            .unwrap()
            .1
            .split_once("/// Insert compact legacy card tuples")
            .unwrap()
            .0;
        assert!(!callbacks.contains("PowerId::SentryMode"));
    }

    #[test]
    fn sentry_mode_later_peer_records_suffix_after_infinite_blades_ends_combat() {
        let (mut state, catalog) = sentry_mode_fixture(0);
        state.powers.set(PowerId::SentryMode, SlotWire::Int, 3);
        state.powers.set(PowerId::InfiniteBlades, SlotWire::Int, 1);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_before_hand_draw_order(&[PowerId::InfiniteBlades, PowerId::SentryMode])
        );
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        // The fixture's live enemy drops to 1 HP so the earlier peer kills it.
        state.monsters_mut()[0].hp = 1;

        crate::engine::cards::inject_generated_bottom(
            &mut state,
            &catalog,
            CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            },
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.history.over, "the earlier peer ends combat");

        run_sentry_mode_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 4);
        assert_eq!(state.next_generated_hook_uid, 4);
        assert_eq!(
            state.next_card_uid, 101,
            "only the earlier Infinite Blades Add allocates"
        );
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    #[test]
    fn sentry_mode_late_generated_refusal_is_whole_listener_atomic() {
        let (mut state, catalog) = sentry_mode_fixture(0);
        state.powers.set(PowerId::SentryMode, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_before_hand_draw_order(&[PowerId::SentryMode])
        );
        state.next_generated_hook_uid = i32::MAX - 1;
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 91 }];
        let before_events = events.clone();

        assert_eq!(
            run_sentry_mode_before_hand_draw(&mut state, &catalog, &mut events),
            Err(EngineRefusal::CounterOverflow("next_generated_hook_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn sentry_mode_listener_has_recursive_leaf_and_refuses_malformed_live_state_atomically() {
        for malformed in 0..5 {
            let (mut state, catalog) = sentry_mode_fixture(0);
            state.powers.set(PowerId::SentryMode, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_before_hand_draw_order(&[PowerId::SentryMode])
            );
            match malformed {
                0 => state.powers.set(PowerId::SentryMode, SlotWire::Bool, 1),
                1 => state.powers.set(PowerId::SentryMode, SlotWire::Int, -1),
                2 => assert!(state.fanouts.set_before_hand_draw_order(&[])),
                3 => assert!(
                    state
                        .fanouts
                        .set_before_hand_draw_order(&[PowerId::SentryMode, PowerId::SentryMode,])
                ),
                4 => state.powers.set(PowerId::Foregone, SlotWire::Int, 1),
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 92 }];
            let before_events = events.clone();
            assert!(run_sentry_mode_before_hand_draw(&mut state, &catalog, &mut events,).is_err());
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        let identity = CardIdentity {
            id: CardId::SentryMode,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::SweepingGaze,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some(),
            "interning either Sentry Mode row recursively closes its exact generated leaf"
        );
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 41,
            atom,
            flags: 0,
        });
        let mut events = Vec::new();
        apply_sentry_fixture(&mut state, &catalog, 0, &mut events).unwrap();

        state.powers.set(PowerId::SentryMode, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_before_hand_draw_order(&[PowerId::SentryMode])
        );
        run_sentry_mode_before_hand_draw(&mut state, &catalog, &mut events).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice().len(), 1);
    }

    /// Every stub still refuses **by its own name**, through the generated
    /// dispatch. A stub weakened into a silent `Ok(())` would make an
    /// unmodeled card play as if the step were absent — the exact I5 failure
    /// the escalation rule exists to prevent.
    #[test]
    fn every_escalated_stub_refuses_by_its_own_kind() {
        let (mut state, catalog) = ctx_state();
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let mut events = Vec::new();
            assert_eq!(
                run(kind, &mut state, &catalog, Some(0), &[], &mut events),
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
    fn invoke_rows_apply_summon_then_energy_with_exact_amounts() {
        for (upgrade, amount) in [(0, 2), (1, 3)] {
            let (mut state, catalog) = invoke_fixture(upgrade);
            state.powers.set(PowerId::SummonNextTurn, SlotWire::Int, 5);
            state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 7);
            let mut events = Vec::new();

            run_invoke_program(&mut state, &catalog, upgrade, &mut events).unwrap();

            assert_eq!(state.powers.value(PowerId::SummonNextTurn), 5 + amount);
            assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 7 + amount);
            assert_eq!(
                events,
                [
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::SummonNextTurn,
                        amount: 5 + amount,
                    },
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::EnergyNextTurn,
                        amount: 7 + amount,
                    },
                ]
            );
        }
    }

    #[test]
    fn invoke_preflights_both_power_additions_before_first_write() {
        for (power, site) in [
            (PowerId::SummonNextTurn, "summon next turn"),
            (PowerId::EnergyNextTurn, "energy next turn"),
        ] {
            let (mut state, catalog) = invoke_fixture(0);
            state.powers.set(power, SlotWire::Int, i32::MAX);
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 77 }];
            let before_events = events.clone();

            assert_eq!(
                run_invoke_program(&mut state, &catalog, 0, &mut events),
                Err(EngineRefusal::CounterOverflow(site))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn invoke_refuses_noncanonical_context_and_physical_source_atomically() {
        let (mut state, catalog) = invoke_fixture(0);
        let identity = CardIdentity {
            id: CardId::Invoke,
            upgrade: 0,
            enchantment: None,
        };
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let step = catalog.steps(&spec)[0];

        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 41,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        assert_eq!(
            summon_next_turn(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "summon_next_turn exact Invoke row"
            ))
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());

        let duplicate = ctx.state.piles.get(PileId::Play).as_slice()[0];
        ctx.state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(duplicate);
        ctx.target = None;
        let before = ctx.state.clone();
        assert_eq!(
            summon_next_turn(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: 41,
                matches: 2,
            })
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());
    }

    #[test]
    fn invoke_refuses_missing_or_duplicate_power_listener_order_before_either_apply() {
        for duplicate in [false, true] {
            let (mut state, catalog) = invoke_fixture(0);
            state.powers.set(PowerId::Shroud, SlotWire::Int, 4);
            if duplicate {
                assert!(
                    state
                        .fanouts
                        .set_after_power_amount_changed_order(&[PowerId::Shroud, PowerId::Shroud])
                );
            }
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 77 }];
            let before_events = events.clone();

            assert_eq!(
                run_invoke_program(&mut state, &catalog, 0, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "Invoke after-power-amount-changed listener order"
                ))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn focus_sources_stack_only_their_exact_current_build_rows() {
        let identity = CardIdentity {
            id: CardId::BiasedCognition,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        state.hp = 50;
        let mut events = Vec::new();

        let mut focus_ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(5)],
            events: &mut events,
        };
        focus(&mut focus_ctx).unwrap();
        focus_ctx.args = &[CompiledArg::I(6)];
        assert_eq!(
            focus(&mut focus_ctx),
            Err(EngineRefusal::MalformedArgs("focus"))
        );
        focus_ctx.args = &[CompiledArg::I(1)];
        biased_cognition(&mut focus_ctx).unwrap();

        assert_eq!(state.powers.value(PowerId::Focus), 5);
        assert_eq!(state.powers.value(PowerId::BiasedCognition), 1);
    }

    #[test]
    fn turn_tick_writers_pin_every_current_build_source_row() {
        let cases = [
            (
                StepKind::Aggression,
                CardId::Aggression,
                0,
                1,
                PowerId::Aggression,
            ),
            (
                StepKind::Aggression,
                CardId::Aggression,
                1,
                1,
                PowerId::Aggression,
            ),
            (
                StepKind::Barricade,
                CardId::Barricade,
                0,
                1,
                PowerId::Barricade,
            ),
            (
                StepKind::Barricade,
                CardId::Barricade,
                1,
                1,
                PowerId::Barricade,
            ),
            (StepKind::Blur, CardId::Blur, 0, 1, PowerId::Blur),
            (StepKind::Blur, CardId::Blur, 1, 1, PowerId::Blur),
            (StepKind::Coolant, CardId::Coolant, 0, 2, PowerId::Coolant),
            (StepKind::Coolant, CardId::Coolant, 1, 3, PowerId::Coolant),
            (
                StepKind::Countdown,
                CardId::Countdown,
                0,
                6,
                PowerId::Countdown,
            ),
            (
                StepKind::Countdown,
                CardId::Countdown,
                1,
                9,
                PowerId::Countdown,
            ),
            (StepKind::Demesne, CardId::Demesne, 0, 1, PowerId::Demesne),
            (StepKind::Demesne, CardId::Demesne, 1, 1, PowerId::Demesne),
            (
                StepKind::DemonForm,
                CardId::DemonForm,
                0,
                3,
                PowerId::DemonForm,
            ),
            (
                StepKind::DemonForm,
                CardId::DemonForm,
                1,
                4,
                PowerId::DemonForm,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Glow,
                0,
                1,
                PowerId::DrawNextTurn,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Glow,
                1,
                1,
                PowerId::DrawNextTurn,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Predator,
                0,
                2,
                PowerId::DrawNextTurn,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Predator,
                1,
                2,
                PowerId::DrawNextTurn,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Relax,
                0,
                2,
                PowerId::DrawNextTurn,
            ),
            (
                StepKind::DrawNextTurn,
                CardId::Relax,
                1,
                3,
                PowerId::DrawNextTurn,
            ),
            (StepKind::Loop, CardId::Loop, 0, 1, PowerId::Loop),
            (StepKind::Loop, CardId::Loop, 1, 2, PowerId::Loop),
            (
                StepKind::MachineLearning,
                CardId::MachineLearning,
                0,
                1,
                PowerId::MachineLearning,
            ),
            (
                StepKind::MachineLearning,
                CardId::MachineLearning,
                1,
                1,
                PowerId::MachineLearning,
            ),
            (
                StepKind::Neurosurge,
                CardId::Neurosurge,
                0,
                3,
                PowerId::Neurosurge,
            ),
            (
                StepKind::Neurosurge,
                CardId::Neurosurge,
                1,
                3,
                PowerId::Neurosurge,
            ),
            (
                StepKind::NoxiousFumes,
                CardId::NoxiousFumes,
                0,
                2,
                PowerId::NoxiousFumes,
            ),
            (
                StepKind::NoxiousFumes,
                CardId::NoxiousFumes,
                1,
                3,
                PowerId::NoxiousFumes,
            ),
            (
                StepKind::Plating,
                CardId::EternalArmor,
                0,
                9,
                PowerId::Plating,
            ),
            (
                StepKind::Plating,
                CardId::EternalArmor,
                1,
                12,
                PowerId::Plating,
            ),
            (
                StepKind::Plating,
                CardId::NeutronAegis,
                0,
                8,
                PowerId::Plating,
            ),
            (
                StepKind::Plating,
                CardId::NeutronAegis,
                1,
                11,
                PowerId::Plating,
            ),
            (
                StepKind::Plating,
                CardId::StoneArmor,
                0,
                4,
                PowerId::Plating,
            ),
            (
                StepKind::Plating,
                CardId::StoneArmor,
                1,
                6,
                PowerId::Plating,
            ),
            (
                StepKind::PrepTime,
                CardId::PrepTime,
                0,
                4,
                PowerId::PrepTime,
            ),
            (
                StepKind::PrepTime,
                CardId::PrepTime,
                1,
                6,
                PowerId::PrepTime,
            ),
            (StepKind::Pyre, CardId::Pyre, 0, 1, PowerId::Pyre),
            (StepKind::Pyre, CardId::Pyre, 1, 2, PowerId::Pyre),
            (
                StepKind::RollingBoulder,
                CardId::RollingBoulder,
                0,
                5,
                PowerId::RollingBoulder,
            ),
            (
                StepKind::RollingBoulder,
                CardId::RollingBoulder,
                1,
                10,
                PowerId::RollingBoulder,
            ),
            (
                StepKind::WraithForm,
                CardId::WraithForm,
                0,
                1,
                PowerId::WraithForm,
            ),
            (
                StepKind::WraithForm,
                CardId::WraithForm,
                1,
                1,
                PowerId::WraithForm,
            ),
        ];

        for (kind, id, upgrade, amount, power) in cases {
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
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            // Every row is a native `PowerCmd.Apply<T>` and is ending-gated
            // (#3183); `every_player_power_apply_row_skips_while_combat_is_ending`
            // pins the ending half.
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(amount)],
                events: &mut events,
            };

            crate::steps::apply_step(kind, &mut ctx).unwrap();
            assert_eq!(
                state.powers.value(power),
                i32::try_from(amount).unwrap(),
                "{} L{}",
                id.as_str(),
                upgrade
            );
        }
    }

    #[test]
    fn card_event_writers_pin_every_current_build_source_row_and_register_order() {
        let cases = [
            (
                StepKind::Afterimage,
                CardId::Afterimage,
                0,
                1,
                PowerId::Afterimage,
            ),
            (
                StepKind::Afterimage,
                CardId::Afterimage,
                1,
                1,
                PowerId::Afterimage,
            ),
            (
                StepKind::Automation,
                CardId::Automation,
                0,
                1,
                PowerId::Automation,
            ),
            (
                StepKind::Automation,
                CardId::Automation,
                1,
                1,
                PowerId::Automation,
            ),
            (
                StepKind::Cacophony,
                CardId::Cacophony,
                0,
                66,
                PowerId::Cacophony,
            ),
            (
                StepKind::Cacophony,
                CardId::Cacophony,
                1,
                99,
                PowerId::Cacophony,
            ),
            (
                StepKind::CorrosiveWave,
                CardId::CorrosiveWave,
                0,
                2,
                PowerId::CorrosiveWave,
            ),
            (
                StepKind::CorrosiveWave,
                CardId::CorrosiveWave,
                1,
                3,
                PowerId::CorrosiveWave,
            ),
            (
                StepKind::DanseMacabre,
                CardId::DanseMacabre,
                0,
                4,
                PowerId::DanseMacabre,
            ),
            (
                StepKind::DanseMacabre,
                CardId::DanseMacabre,
                1,
                6,
                PowerId::DanseMacabre,
            ),
            (
                StepKind::DarkEmbrace,
                CardId::DarkEmbrace,
                0,
                1,
                PowerId::DarkEmbrace,
            ),
            (
                StepKind::DarkEmbrace,
                CardId::DarkEmbrace,
                1,
                1,
                PowerId::DarkEmbrace,
            ),
            (
                StepKind::FeelNoPain,
                CardId::FeelNoPain,
                0,
                3,
                PowerId::FeelNoPain,
            ),
            (
                StepKind::FeelNoPain,
                CardId::FeelNoPain,
                1,
                4,
                PowerId::FeelNoPain,
            ),
            (
                StepKind::InfiniteBlades,
                CardId::InfiniteBlades,
                0,
                1,
                PowerId::InfiniteBlades,
            ),
            (
                StepKind::InfiniteBlades,
                CardId::InfiniteBlades,
                1,
                1,
                PowerId::InfiniteBlades,
            ),
            (
                StepKind::Iteration,
                CardId::Iteration,
                0,
                2,
                PowerId::Iteration,
            ),
            (
                StepKind::Iteration,
                CardId::Iteration,
                1,
                3,
                PowerId::Iteration,
            ),
            (
                StepKind::Pagestorm,
                CardId::Pagestorm,
                0,
                1,
                PowerId::Pagestorm,
            ),
            (
                StepKind::Pagestorm,
                CardId::Pagestorm,
                1,
                1,
                PowerId::Pagestorm,
            ),
            (
                StepKind::PaleBlueDot,
                CardId::PaleBlueDot,
                0,
                1,
                PowerId::PaleBlueDot,
            ),
            (
                StepKind::PaleBlueDot,
                CardId::PaleBlueDot,
                1,
                2,
                PowerId::PaleBlueDot,
            ),
            (StepKind::Panache, CardId::Panache, 0, 10, PowerId::Panache),
            (StepKind::Panache, CardId::Panache, 1, 14, PowerId::Panache),
            (StepKind::Rage, CardId::Rage, 0, 3, PowerId::Rage),
            (StepKind::Rage, CardId::Rage, 1, 5, PowerId::Rage),
            (StepKind::Sneaky, CardId::Sneaky, 0, 1, PowerId::Sneaky),
            (StepKind::Sneaky, CardId::Sneaky, 1, 2, PowerId::Sneaky),
            (
                StepKind::Speedster,
                CardId::Speedster,
                0,
                2,
                PowerId::Speedster,
            ),
            (
                StepKind::Speedster,
                CardId::Speedster,
                1,
                2,
                PowerId::Speedster,
            ),
            (
                StepKind::SpiritOfAsh,
                CardId::SpiritOfAsh,
                0,
                4,
                PowerId::SpiritOfAsh,
            ),
            (
                StepKind::SpiritOfAsh,
                CardId::SpiritOfAsh,
                1,
                5,
                PowerId::SpiritOfAsh,
            ),
            (
                StepKind::Subroutine,
                CardId::Subroutine,
                0,
                1,
                PowerId::Subroutine,
            ),
            (
                StepKind::Subroutine,
                CardId::Subroutine,
                1,
                1,
                PowerId::Subroutine,
            ),
        ];

        for (kind, id, upgrade, amount, power) in cases {
            let identity = CardIdentity {
                id,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(amount)],
                events: &mut events,
            };

            crate::steps::apply_step(kind, &mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(power), amount as i32, "{id:?}");
            let bad_args = [CompiledArg::I(amount + 1)];
            ctx.args = &bad_args;
            assert_eq!(
                crate::steps::apply_step(kind, &mut ctx),
                Err(EngineRefusal::MalformedArgs(kind.as_str())),
                "{id:?} accepted an unregistered amount"
            );

            let ordered = ctx.state.fanouts.after_card_drawn_order().contains(&power)
                || ctx
                    .state
                    .fanouts
                    .after_card_exhausted_order()
                    .contains(&power)
                || ctx.state.fanouts.before_hand_draw_order().contains(&power);
            assert_eq!(
                ordered,
                matches!(
                    power,
                    PowerId::Automation
                        | PowerId::Cacophony
                        | PowerId::CorrosiveWave
                        | PowerId::DarkEmbrace
                        | PowerId::FeelNoPain
                        | PowerId::InfiniteBlades
                        | PowerId::Iteration
                        | PowerId::Pagestorm
                        | PowerId::Speedster
                ),
                "{id:?} listener registration"
            );
        }
    }

    #[test]
    fn fold_writers_pin_every_current_build_source_row() {
        let cases = [
            (
                StepKind::Accuracy,
                CardId::Accuracy,
                0,
                4,
                PowerId::Accuracy,
            ),
            (
                StepKind::Accuracy,
                CardId::Accuracy,
                1,
                6,
                PowerId::Accuracy,
            ),
            (
                StepKind::TempDexterity,
                CardId::Anticipate,
                0,
                2,
                PowerId::TempDexterity,
            ),
            (
                StepKind::TempDexterity,
                CardId::Anticipate,
                1,
                4,
                PowerId::TempDexterity,
            ),
            (StepKind::Cruelty, CardId::Cruelty, 0, 25, PowerId::Cruelty),
            (StepKind::Cruelty, CardId::Cruelty, 1, 50, PowerId::Cruelty),
            (StepKind::Fasten, CardId::Fasten, 0, 4, PowerId::Fasten),
            (StepKind::Fasten, CardId::Fasten, 1, 6, PowerId::Fasten),
            (StepKind::Vigor, CardId::Patter, 0, 2, PowerId::Vigor),
            (StepKind::Vigor, CardId::Patter, 1, 3, PowerId::Vigor),
            (StepKind::Vigor, CardId::Terraforming, 0, 7, PowerId::Vigor),
            (StepKind::Vigor, CardId::Terraforming, 1, 10, PowerId::Vigor),
            (
                StepKind::Tracking,
                CardId::Tracking,
                0,
                50,
                PowerId::Tracking,
            ),
            (
                StepKind::Tracking,
                CardId::Tracking,
                1,
                50,
                PowerId::Tracking,
            ),
            (
                StepKind::Unmovable,
                CardId::Unmovable,
                0,
                1,
                PowerId::Unmovable,
            ),
            (
                StepKind::Unmovable,
                CardId::Unmovable,
                1,
                1,
                PowerId::Unmovable,
            ),
        ];

        for (kind, id, upgrade, amount, power) in cases {
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
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(amount)],
                events: &mut events,
            };

            crate::steps::apply_step(kind, &mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(power), amount as i32, "{id:?}");
            let bad_args = [CompiledArg::I(amount + 1)];
            ctx.args = &bad_args;
            assert_eq!(
                crate::steps::apply_step(kind, &mut ctx),
                Err(EngineRefusal::MalformedArgs(kind.as_str())),
                "{id:?} accepted an unregistered amount"
            );
        }
    }

    #[test]
    fn hit_hook_writers_pin_every_current_build_source_row() {
        let cases = [
            (StepKind::Buffer, CardId::Buffer, 0, 1, PowerId::Buffer),
            (StepKind::Buffer, CardId::Buffer, 1, 2, PowerId::Buffer),
            (
                StepKind::Colossus,
                CardId::Colossus,
                0,
                1,
                PowerId::Colossus,
            ),
            (
                StepKind::Colossus,
                CardId::Colossus,
                1,
                1,
                PowerId::Colossus,
            ),
            (
                StepKind::FlameBarrier,
                CardId::FlameBarrier,
                0,
                4,
                PowerId::FlameBarrier,
            ),
            (
                StepKind::FlameBarrier,
                CardId::FlameBarrier,
                1,
                6,
                PowerId::FlameBarrier,
            ),
            (
                StepKind::Intangible,
                CardId::Apparition,
                0,
                1,
                PowerId::Intangible,
            ),
            (
                StepKind::Intangible,
                CardId::Apparition,
                1,
                1,
                PowerId::Intangible,
            ),
            (
                StepKind::Intangible,
                CardId::WraithForm,
                0,
                2,
                PowerId::Intangible,
            ),
            (
                StepKind::Intangible,
                CardId::WraithForm,
                1,
                3,
                PowerId::Intangible,
            ),
            (StepKind::Reflect, CardId::Reflect, 0, 1, PowerId::Reflect),
            (StepKind::Reflect, CardId::Reflect, 1, 1, PowerId::Reflect),
            (StepKind::Rupture, CardId::Rupture, 0, 1, PowerId::Rupture),
            (StepKind::Rupture, CardId::Rupture, 1, 2, PowerId::Rupture),
            (
                StepKind::TheGambit,
                CardId::TheGambit,
                0,
                1,
                PowerId::TheGambit,
            ),
            (
                StepKind::TheGambit,
                CardId::TheGambit,
                1,
                1,
                PowerId::TheGambit,
            ),
            (StepKind::Thorns, CardId::Abrasive, 0, 4, PowerId::Thorns),
            (StepKind::Thorns, CardId::Abrasive, 1, 6, PowerId::Thorns),
            (StepKind::Thorns, CardId::Caltrops, 0, 3, PowerId::Thorns),
            (StepKind::Thorns, CardId::Caltrops, 1, 5, PowerId::Thorns),
        ];

        for (kind, id, upgrade, amount, power) in cases {
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
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(amount)],
                events: &mut events,
            };

            crate::steps::apply_step(kind, &mut ctx).unwrap();
            assert_eq!(ctx.state.powers.value(power), amount as i32, "{id:?}");
            let bad_args = [CompiledArg::I(amount + 1)];
            ctx.args = &bad_args;
            assert_eq!(
                crate::steps::apply_step(kind, &mut ctx),
                Err(EngineRefusal::MalformedArgs(kind.as_str())),
                "{id:?} accepted an unregistered amount"
            );
        }
    }

    #[test]
    fn poison_applies_through_the_card_sourced_path_and_allocates_identity() {
        let (mut state, catalog) = ctx_state();
        let mut events = Vec::new();
        run(
            StepKind::Poison,
            &mut state,
            &catalog,
            Some(0),
            &[CompiledArg::I(4)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 4);
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            &[MiseryToken::Poison]
        );
        assert_eq!(state.monsters[0].poison_uid, 0);
        assert_eq!(state.next_poison_uid, 1);
        assert_eq!(events.len(), 1);

        // A restack modifies the amount without reallocating the identity.
        run(
            StepKind::Poison,
            &mut state,
            &catalog,
            Some(0),
            &[CompiledArg::I(3)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 7);
        assert_eq!(state.monsters[0].misery_debuff_order.as_slice().len(), 1);
        assert_eq!(state.monsters[0].poison_uid, 0);
        assert_eq!(state.next_poison_uid, 1);
    }

    #[test]
    fn poison_skips_a_dead_target_and_requires_one() {
        let (mut state, catalog) = ctx_state();
        state.monsters_mut()[1].hp = 0;
        let mut events = Vec::new();
        run(
            StepKind::Poison,
            &mut state,
            &catalog,
            Some(1),
            &[CompiledArg::I(4)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[1].powers.value(PowerId::Poison), 0);
        assert!(events.is_empty());

        assert_eq!(
            run(
                StepKind::Poison,
                &mut state,
                &catalog,
                None,
                &[CompiledArg::I(4)],
                &mut events,
            ),
            Err(EngineRefusal::TargetMismatch { required: true })
        );
    }

    #[test]
    fn post_slice_five_carrier_census_is_exact() {
        let carriers = |kind| {
            CARD_ROWS
                .iter()
                .filter_map(|row| {
                    row.steps
                        .iter()
                        .find(|step| step.kind == kind)
                        .map(|step| (row.id, row.upgrade, step.args))
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            carriers(StepKind::Doom),
            vec![
                (CardId::Scourge, 0, &[Arg::I(13)][..]),
                (CardId::Scourge, 1, &[Arg::I(16)][..]),
            ]
        );
        assert_eq!(
            carriers(StepKind::StrengthEnemy),
            vec![
                (CardId::FightMe, 0, &[Arg::I(1)][..]),
                (CardId::FightMe, 1, &[Arg::I(1)][..]),
            ]
        );
        for (kind, id) in [
            (StepKind::FreeAttack, CardId::Unrelenting),
            (StepKind::FreeEthereal, CardId::Veilpiercer),
            (StepKind::FreePower, CardId::Synthesis),
            (StepKind::FreeSkill, CardId::Pounce),
        ] {
            assert_eq!(
                carriers(kind),
                vec![(id, 0, &[Arg::I(1)][..]), (id, 1, &[Arg::I(1)][..]),],
                "{kind:?} carrier census drifted"
            );
        }
        assert_eq!(
            carriers(StepKind::RetainHand),
            vec![
                (CardId::Convergence, 0, &[Arg::I(1)][..]),
                (CardId::Convergence, 1, &[Arg::I(1)][..]),
                (CardId::Equilibrium, 0, &[Arg::I(1)][..]),
                (CardId::Equilibrium, 1, &[Arg::I(1)][..]),
                (CardId::Salvo, 0, &[Arg::I(1)][..]),
                (CardId::Salvo, 1, &[Arg::I(1)][..]),
            ]
        );
        assert_eq!(
            carriers(StepKind::PhantomBlades),
            vec![
                (CardId::PhantomBlades, 0, &[Arg::I(9)][..]),
                (CardId::PhantomBlades, 1, &[Arg::I(12)][..]),
            ]
        );
    }

    #[test]
    fn retain_hand_writers_pin_all_six_rows_and_refuse_other_shapes_atomically() {
        for (id, upgrade) in [
            (CardId::Convergence, 0),
            (CardId::Convergence, 1),
            (CardId::Equilibrium, 0),
            (CardId::Equilibrium, 1),
            (CardId::Salvo, 0),
            (CardId::Salvo, 1),
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
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            let mut events = Vec::new();
            let args = [CompiledArg::I(1)];
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };
            crate::steps::apply_step(StepKind::RetainHand, &mut ctx).unwrap();
            assert_eq!(state.powers.value(PowerId::RetainHand), 1);
            assert_eq!(
                events,
                vec![Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::RetainHand,
                    amount: 1,
                }]
            );

            let before = state.clone();
            let event_count = events.len();
            let bad_args = [CompiledArg::I(2)];
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &bad_args,
                events: &mut events,
            };
            assert_eq!(
                crate::steps::apply_step(StepKind::RetainHand, &mut ctx),
                Err(EngineRefusal::MalformedArgs("retain_hand"))
            );
            assert_eq!(state, before);
            assert_eq!(events.len(), event_count);
        }
    }

    #[test]
    fn phantom_blades_marks_only_current_draw_hand_discard_shivs() {
        let source = CardIdentity {
            id: CardId::PhantomBlades,
            upgrade: 0,
            enchantment: None,
        };
        let shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let ordinary = CardIdentity {
            id: CardId::StrikeSilent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source).unwrap();
        let shiv_atom = builder.intern(shiv).unwrap();
        let ordinary_atom = builder.intern(ordinary).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        for (pile, uid, atom) in [
            (PileId::Draw, 1, shiv_atom),
            (PileId::Hand, 2, shiv_atom),
            (PileId::Hand, 3, ordinary_atom),
            (PileId::Discard, 4, shiv_atom),
            (PileId::Exhaust, 5, shiv_atom),
            (PileId::Play, 6, shiv_atom),
        ] {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        state.card_states.set_local_sly(2);
        let mut events = Vec::new();
        let args = [CompiledArg::I(9)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };

        crate::steps::apply_step(StepKind::PhantomBlades, &mut ctx).unwrap();
        assert_eq!(ctx.state.powers.value(PowerId::PhantomBlades), 9);
        assert_eq!(
            ctx.events.as_slice(),
            &[Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::PhantomBlades,
                amount: 9,
            }]
        );
        for uid in [1, 2, 4] {
            assert!(ctx.state.card_states.get(uid).local_retain, "uid {uid}");
        }
        assert!(ctx.state.exact_piles);
        assert!(ctx.state.card_states.get(2).local_sly);
        for uid in [3, 5, 6] {
            assert!(!ctx.state.card_states.get(uid).local_retain, "uid {uid}");
        }

        crate::steps::apply_step(StepKind::PhantomBlades, &mut ctx).unwrap();
        assert_eq!(ctx.state.powers.value(PowerId::PhantomBlades), 18);
        assert_eq!(ctx.state.card_states.len(), 3);
        assert_eq!(
            ctx.events.as_slice(),
            &[
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::PhantomBlades,
                    amount: 9,
                },
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::PhantomBlades,
                    amount: 18,
                },
            ]
        );
    }

    #[test]
    fn phantom_blades_preflights_shape_power_and_current_atoms_before_retain() {
        let source = CardIdentity {
            id: CardId::PhantomBlades,
            upgrade: 1,
            enchantment: None,
        };
        let shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source).unwrap();
        let shiv_atom = builder.intern(shiv).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();

        let run =
            |state: &mut HotState, args: &[CompiledArg], events: &mut Vec<crate::engine::Event>| {
                let mut ctx = StepCtx {
                    state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 0,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args,
                    events,
                };
                crate::steps::apply_step(StepKind::PhantomBlades, &mut ctx)
            };

        let mut malformed = HotState::at_defaults();

        // A live enemy keeps `damage_combat_is_ending` false (#3183).

        malformed.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        malformed
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom: shiv_atom,
                flags: 0,
            });
        let before = malformed.clone();
        let mut events = Vec::new();
        assert_eq!(
            run(&mut malformed, &[CompiledArg::I(9)], &mut events),
            Err(EngineRefusal::MalformedArgs("phantom_blades"))
        );
        assert_eq!(malformed, before);
        assert!(events.is_empty());

        let mut overflow = before.clone();
        overflow
            .powers
            .set(PowerId::PhantomBlades, SlotWire::Int, i32::MAX);
        let before_overflow = overflow.clone();
        assert_eq!(
            run(&mut overflow, &[CompiledArg::I(12)], &mut events),
            Err(EngineRefusal::CounterOverflow("phantom_blades"))
        );
        assert_eq!(overflow, before_overflow);
        assert!(events.is_empty());

        let mut unknown = HotState::at_defaults();

        // A live enemy keeps `damage_combat_is_ending` false (#3183).

        unknown.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        unknown
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 2,
                atom: u16::MAX,
                flags: 0,
            });
        let before_unknown = unknown.clone();
        assert_eq!(
            run(&mut unknown, &[CompiledArg::I(12)], &mut events),
            Err(EngineRefusal::UnknownAtom(u16::MAX))
        );
        assert_eq!(unknown, before_unknown);
        assert!(events.is_empty());
    }

    #[test]
    fn free_power_writers_stack_and_refuse_unregistered_shapes_atomically() {
        for (kind, id, power) in [
            (
                StepKind::FreeAttack,
                CardId::Unrelenting,
                PowerId::FreeAttack,
            ),
            (
                StepKind::FreeEthereal,
                CardId::Veilpiercer,
                PowerId::FreeEthereal,
            ),
            (StepKind::FreePower, CardId::Synthesis, PowerId::FreePower),
            (StepKind::FreeSkill, CardId::Pounce, PowerId::FreeSkill),
        ] {
            let (mut state, _) = ctx_state();
            let mut events = Vec::new();
            for _ in 0..2 {
                run_card(
                    kind,
                    id,
                    0,
                    &mut state,
                    None,
                    &[CompiledArg::I(1)],
                    &mut events,
                )
                .unwrap();
            }
            assert_eq!(state.powers.value(power), 2);

            let before = state.clone();
            let event_count = events.len();
            assert_eq!(
                run_card(
                    kind,
                    id,
                    0,
                    &mut state,
                    None,
                    &[CompiledArg::I(2)],
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(kind.as_str()))
            );
            assert_eq!(state, before);
            assert_eq!(events.len(), event_count);
        }
    }

    #[test]
    fn star_power_writers_pin_carriers_stack_and_preserve_reset_order() {
        let (mut state, _) = ctx_state();
        let mut events = Vec::new();
        run_card(
            StepKind::StarNextTurn,
            CardId::HiddenCache,
            0,
            &mut state,
            None,
            &[CompiledArg::I(3)],
            &mut events,
        )
        .unwrap();
        run_card(
            StepKind::Genesis,
            CardId::Genesis,
            1,
            &mut state,
            None,
            &[CompiledArg::I(3)],
            &mut events,
        )
        .unwrap();
        run_card(
            StepKind::StarNextTurn,
            CardId::Convergence,
            1,
            &mut state,
            None,
            &[CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::StarNextTurn), 5);
        assert_eq!(state.powers.value(PowerId::Genesis), 3);
        assert_eq!(
            state.fanouts.star_energy_reset_order(),
            [PowerId::StarNextTurn, PowerId::Genesis]
        );

        let before = state.clone();
        assert_eq!(
            run_card(
                StepKind::Genesis,
                CardId::Genesis,
                0,
                &mut state,
                None,
                &[CompiledArg::I(3)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("genesis"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn generated_power_writers_stack_without_reordering() {
        let carriers = |kind| {
            CARD_ROWS
                .iter()
                .filter_map(|row| {
                    row.steps
                        .iter()
                        .find(|step| step.kind == kind)
                        .map(|step| (row.id, row.upgrade, step.args))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            carriers(StepKind::Arsenal),
            vec![
                (CardId::Arsenal, 0, &[Arg::I(1)][..]),
                (CardId::Arsenal, 1, &[Arg::I(1)][..]),
            ]
        );
        assert_eq!(
            carriers(StepKind::PillarOfCreation),
            vec![
                (CardId::PillarOfCreation, 0, &[Arg::I(2)][..]),
                (CardId::PillarOfCreation, 1, &[Arg::I(3)][..]),
            ]
        );
        assert_eq!(
            carriers(StepKind::Smokestack),
            vec![
                (CardId::Smokestack, 0, &[Arg::I(5)][..]),
                (CardId::Smokestack, 1, &[Arg::I(7)][..]),
            ]
        );
        let (mut state, _) = ctx_state();
        let mut events = Vec::new();
        run_card(
            StepKind::Smokestack,
            CardId::Smokestack,
            0,
            &mut state,
            None,
            &[CompiledArg::I(5)],
            &mut events,
        )
        .unwrap();
        run_card(
            StepKind::PillarOfCreation,
            CardId::PillarOfCreation,
            0,
            &mut state,
            None,
            &[CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        run_card(
            StepKind::Arsenal,
            CardId::Arsenal,
            1,
            &mut state,
            None,
            &[CompiledArg::I(1)],
            &mut events,
        )
        .unwrap();
        run_card(
            StepKind::PillarOfCreation,
            CardId::PillarOfCreation,
            1,
            &mut state,
            None,
            &[CompiledArg::I(3)],
            &mut events,
        )
        .unwrap();

        assert_eq!(state.powers.value(PowerId::PillarOfCreation), 5);
        assert_eq!(state.powers.value(PowerId::Arsenal), 1);
        assert_eq!(state.powers.value(PowerId::Smokestack), 5);
        assert_eq!(
            state.fanouts.local_generated_power_order(),
            [
                PowerId::Smokestack,
                PowerId::PillarOfCreation,
                PowerId::Arsenal
            ]
        );

        let before = state.clone();
        assert_eq!(
            run_card(
                StepKind::Arsenal,
                CardId::Arsenal,
                0,
                &mut state,
                None,
                &[CompiledArg::I(2)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("arsenal"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn cost_power_writers_pin_sources_stack_and_register_once() {
        let (mut state, _) = ctx_state();
        let mut events = Vec::new();
        for upgrade in [0, 1] {
            run_card(
                StepKind::BorrowedTime,
                CardId::BorrowedTime,
                upgrade,
                &mut state,
                None,
                &[CompiledArg::I(1)],
                &mut events,
            )
            .unwrap();
            run_card(
                StepKind::Corruption,
                CardId::Corruption,
                upgrade,
                &mut state,
                None,
                &[CompiledArg::I(1)],
                &mut events,
            )
            .unwrap();
        }
        assert_eq!(state.powers.value(PowerId::BorrowedTime), 2);
        assert_eq!(state.powers.value(PowerId::Corruption), 1);
        assert_eq!(
            state.fanouts.result_location_power_order(),
            [PowerId::Corruption]
        );
    }

    /// #3326 witness for [`stock_replaced_target_is_current`]'s three arms:
    /// no frozen CardPlay identity with a live Stock Axebot refuses by name
    /// before mutation; a frozen identity naming a different creature (the
    /// dead original a Stock respawn replaced) no-ops; a matching identity
    /// applies the power.
    #[test]
    fn mangle_and_fight_me_stock_target_follows_the_frozen_identity() {
        for (kind, id, arg, reason, strength) in [
            (
                StepKind::TempStrengthEnemy,
                CardId::Mangle,
                10,
                "Mangle Axebot Stock target identity",
                -10,
            ),
            (
                StepKind::StrengthEnemy,
                CardId::FightMe,
                1,
                "Fight Me Axebot Stock target identity",
                1,
            ),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            let mut axebot = HotMonster::new(MonsterKind::Axebot, 40);
            axebot.uid = 1;
            axebot.powers.set(PowerId::Stock, SlotWire::Int, 1);
            state.monsters_mut().push(axebot);
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                run_card(
                    kind,
                    id,
                    0,
                    &mut state,
                    Some(0),
                    &[CompiledArg::I(arg)],
                    &mut events
                ),
                Err(EngineRefusal::MalformedArgs(reason))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());

            crate::engine::play::with_test_active_play_target(0, (0, 0), || {
                run_card(
                    kind,
                    id,
                    0,
                    &mut state,
                    Some(0),
                    &[CompiledArg::I(arg)],
                    &mut events,
                )
            })
            .unwrap();
            assert_eq!(state, before, "{id:?}: the replacement takes nothing");
            assert!(events.is_empty());

            crate::engine::play::with_test_active_play_target(0, (0, 1), || {
                run_card(
                    kind,
                    id,
                    0,
                    &mut state,
                    Some(0),
                    &[CompiledArg::I(arg)],
                    &mut events,
                )
            })
            .unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Strength), strength);
        }
    }

    #[test]
    fn strength_enemy_is_exactly_fight_me_and_bypasses_type_two_listeners() {
        let (mut state, _) = ctx_state();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, 7);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let mut events = Vec::new();
        for upgrade in 0..=1 {
            run_card(
                StepKind::Strength,
                CardId::FightMe,
                upgrade,
                &mut state,
                Some(0),
                &[CompiledArg::I(3 + i64::from(upgrade))],
                &mut events,
            )
            .unwrap();
            run_card(
                StepKind::StrengthEnemy,
                CardId::FightMe,
                upgrade,
                &mut state,
                Some(0),
                &[CompiledArg::I(1)],
                &mut events,
            )
            .unwrap();
        }
        assert_eq!(state.powers.value(PowerId::Strength), 7);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 9);
        assert_eq!(state.monsters[0].hp, 12, "Type-1 bypasses Sleight of Flesh");

        let before = state.clone();
        assert_eq!(
            run_card(
                StepKind::StrengthEnemy,
                CardId::FightMe,
                0,
                &mut state,
                Some(0),
                &[CompiledArg::I(2)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("strength_enemy"))
        );
        assert_eq!(state, before);
        assert_eq!(
            run_card(
                StepKind::Strength,
                CardId::StrikeIronclad,
                0,
                &mut state,
                Some(0),
                &[CompiledArg::I(3)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("strength"))
        );
        assert!(IMPLEMENTED.contains(&StepKind::StrengthEnemy));
    }

    #[test]
    fn doom_uses_the_card_sourced_power_path() {
        let (mut state, _) = ctx_state();
        let mut events = Vec::new();

        run_card(
            StepKind::Doom,
            CardId::Scourge,
            0,
            &mut state,
            Some(0),
            &[CompiledArg::I(13)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 13);
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            &[MiseryToken::Doom]
        );
        assert!(state.history.doom_applied_by_player_this_turn);

        assert_eq!(
            run_card(
                StepKind::Doom,
                CardId::Scourge,
                0,
                &mut state,
                Some(0),
                &[CompiledArg::I(16)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("doom"))
        );
    }

    /// Every step kind whose body reaches a native `PowerCmd.Apply<T>` through
    /// `apply_exact_player_power`, `apply_exact_ordered_player_power`,
    /// `shared::energy_next_turn` or `neutral::apply_result_location_power`
    /// (#3183). The IL audit for each carrier is on those helpers.
    const PLAYER_POWER_APPLY_KINDS: [StepKind; 88] = [
        StepKind::Accelerant,
        StepKind::Accuracy,
        StepKind::Afterimage,
        StepKind::Aggression,
        StepKind::Arsenal,
        StepKind::Automation,
        StepKind::Barricade,
        StepKind::BiasedCognition,
        StepKind::BlackHole,
        StepKind::Blur,
        StepKind::BorrowedTime,
        StepKind::Buffer,
        StepKind::Burst,
        StepKind::Cacophony,
        StepKind::ChildOfTheStars,
        StepKind::Colossus,
        StepKind::Coolant,
        StepKind::CorrosiveWave,
        StepKind::Countdown,
        StepKind::Cruelty,
        StepKind::DanseMacabre,
        StepKind::DarkEmbrace,
        StepKind::Demesne,
        StepKind::DemonForm,
        StepKind::DevourLife,
        StepKind::DrawNextTurn,
        StepKind::EchoForm,
        StepKind::EnergyNextTurn,
        StepKind::Entropy,
        StepKind::Envenom,
        StepKind::Fasten,
        StepKind::FeelNoPain,
        StepKind::FlameBarrier,
        StepKind::Focus,
        StepKind::Foregone,
        StepKind::FreeAttack,
        StepKind::FreeEthereal,
        StepKind::FreePower,
        StepKind::FreeSkill,
        StepKind::Genesis,
        StepKind::Hailstorm,
        StepKind::HelloWorld,
        StepKind::InfiniteBlades,
        StepKind::Intangible,
        StepKind::Iteration,
        StepKind::Juggernaut,
        StepKind::Loop,
        StepKind::MachineLearning,
        StepKind::Mayhem,
        StepKind::MonarchsGaze,
        StepKind::NecroMastery,
        StepKind::Neurosurge,
        StepKind::NoDraw,
        StepKind::Nostalgia,
        StepKind::NoxiousFumes,
        StepKind::OneTwoPunch,
        StepKind::Pagestorm,
        StepKind::PaleBlueDot,
        StepKind::PhantomBlades,
        StepKind::PillarOfCreation,
        StepKind::Plating,
        StepKind::PrepTime,
        StepKind::Pyre,
        StepKind::Rage,
        StepKind::ReaperForm,
        StepKind::Rebound,
        StepKind::Reflect,
        StepKind::RetainHand,
        StepKind::RollingBoulder,
        StepKind::Rupture,
        StepKind::SealedThrone,
        StepKind::SignalBoost,
        StepKind::SleightOfFlesh,
        StepKind::Smokestack,
        StepKind::Sneaky,
        StepKind::Speedster,
        StepKind::SpiritOfAsh,
        StepKind::Stampede,
        StepKind::StarNextTurn,
        StepKind::Stratagem,
        StepKind::Subroutine,
        StepKind::TempDexterity,
        StepKind::TheGambit,
        StepKind::Thorns,
        StepKind::TrashToTreasure,
        StepKind::Unmovable,
        StepKind::Vigor,
        StepKind::WraithForm,
    ];

    fn apply_power_row(
        state: &mut HotState,
        catalog: &Catalog,
        spec: &crate::catalog::CardSpec,
        kind: StepKind,
        args: &[CompiledArg],
        target: Option<usize>,
    ) -> (Result<(), EngineRefusal>, Vec<Event>) {
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        let result = crate::steps::apply_step(kind, &mut ctx);
        (result, events)
    }

    /// ``PowerCmd/<Apply>d__1`1::MoveNext`` (RVA `0x3ef988`) returns at
    /// IL_0020-0x0034 while `CombatManager.IsEnding`. Every current-build row
    /// of every audited kind is a no-op then, both when the last enemy has
    /// just died (`damage_combat_is_ending`'s roster arm) and once
    /// `history.over` is set; a malformed operand still refuses; and with a
    /// live enemy the same row writes (#3183).
    #[test]
    fn every_player_power_apply_row_skips_while_combat_is_ending() {
        let mut covered = std::collections::BTreeSet::new();
        let mut problems = Vec::new();
        let mut rows = 0;
        for row in CARD_ROWS {
            for (index, step) in row.steps.iter().enumerate() {
                if !PLAYER_POWER_APPLY_KINDS.contains(&step.kind) {
                    continue;
                }
                let identity = CardIdentity {
                    id: row.id,
                    upgrade: row.upgrade,
                    enchantment: None,
                };
                let mut builder = CatalogBuilder::new();
                let atom = builder.intern(identity).unwrap();
                let catalog = builder.build();
                let spec = *catalog.spec(atom).unwrap();
                let compiled = catalog.steps(&spec)[index];
                assert_eq!(compiled.kind, step.kind);
                let args = catalog.args(compiled.args).to_vec();
                let label = format!("{}+{} {:?}", row.name, row.upgrade, step.kind);

                // The played card sits in Play (the Rebound/Nostalgia source
                // authentication reads it); only Rebound is targeted.
                let fixture = |enemy_hp: i32, over: bool| {
                    let mut state = HotState::at_defaults();
                    state.hp = 50;
                    state
                        .monsters_mut()
                        .push(HotMonster::new(MonsterKind::Toadpole, enemy_hp));
                    state.history.over = over;
                    state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                        uid: 0,
                        atom,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    });
                    state
                };
                let target = (step.kind == StepKind::Rebound).then_some(0);
                let arms = [
                    ("dead roster", fixture(0, false)),
                    ("history.over", fixture(99, true)),
                ];
                for (arm, mut state) in arms {
                    assert!(crate::engine::damage::damage_combat_is_ending(&state));
                    let before = state.clone();
                    let (result, events) =
                        apply_power_row(&mut state, &catalog, &spec, step.kind, &args, target);
                    if result != Ok(()) || state != before || !events.is_empty() {
                        problems.push(format!("{label} {arm}: ending wrote or refused {result:?}"));
                    }
                    let (result, _) = apply_power_row(
                        &mut state,
                        &catalog,
                        &spec,
                        step.kind,
                        &[CompiledArg::I(-7)],
                        target,
                    );
                    if !matches!(result, Err(EngineRefusal::MalformedArgs(_))) || state != before {
                        problems.push(format!("{label} {arm}: malformed operand {result:?}"));
                    }
                }

                let mut live = fixture(99, false);
                let before = live.clone();
                let (result, _) =
                    apply_power_row(&mut live, &catalog, &spec, step.kind, &args, target);
                // Hello World's live write needs its Generation-pool fixture;
                // `hello_world_writer_accepts_both_exact_rows_...` pins it.
                if step.kind != StepKind::HelloWorld && (result != Ok(()) || live == before) {
                    problems.push(format!("{label} live: {result:?}"));
                }
                covered.insert(step.kind);
                rows += 1;
            }
        }
        let missing: Vec<_> = PLAYER_POWER_APPLY_KINDS
            .iter()
            .filter(|kind| !covered.contains(kind))
            .collect();
        assert!(missing.is_empty(), "kinds with no current row: {missing:?}");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert!(rows >= 2 * PLAYER_POWER_APPLY_KINDS.len(), "{rows}");
    }

    /// One Curious Mad Science at `upgrade`, its compiled step and args.
    fn curious_mad_science(
        upgrade: u8,
        relics: &[crate::ids::RelicId],
    ) -> (
        Catalog,
        crate::catalog::CardSpec,
        Vec<CompiledArg>,
        crate::catalog::CardAtom,
    ) {
        let mut builder = CatalogBuilder::new();
        builder.set_relics(relics).unwrap();
        assert!(
            builder.set_mad_science_variant(
                crate::catalog::MadScienceVariant::from_saved(3, 8).unwrap()
            )
        );
        let atom = builder
            .intern(CardIdentity {
                id: CardId::MadScience,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let steps = catalog.steps(&spec);
        assert_eq!(steps.len(), 1, "Curious's whole body is one apply");
        assert_eq!(steps[0].kind, StepKind::MadScienceCurious);
        let args = catalog.args(steps[0].args).to_vec();
        (catalog, spec, args, atom)
    }

    fn curious_fixture(atom: crate::catalog::CardAtom, enemy_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, enemy_hp));
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 0,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state
    }

    /// #3427 witness: Mad Science's Curious rider applies CuriousPower
    /// `CuriousReduction` (1 at both levels) and stacks it (Counter); it
    /// writes nothing while combat is ending; it refuses by name beside a
    /// live Borrowed Time and for any spec that is not the generated Curious
    /// row; beside an owned Spiked Gauntlets it applies (#3437).
    #[test]
    fn mad_science_curious_applies_and_stacks_curious_power_or_refuses_by_name() {
        for upgrade in [0, 1] {
            let (catalog, spec, args, atom) = curious_mad_science(upgrade, &[]);
            assert_eq!(args, [CompiledArg::I(1)]);
            assert!(spec.is_power && !spec.targeted);

            let mut state = curious_fixture(atom, 99);
            let (result, events) = apply_power_row(
                &mut state,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &args,
                None,
            );
            assert_eq!(result, Ok(()));
            assert_eq!(state.powers.value(PowerId::Curious), 1);
            assert!(!events.is_empty());
            let (result, _) = apply_power_row(
                &mut state,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &args,
                None,
            );
            assert_eq!(result, Ok(()));
            assert_eq!(state.powers.value(PowerId::Curious), 2, "Counter stacks");

            // PowerCmd.Apply is a no-op while combat is ending.
            let mut ending = curious_fixture(atom, 0);
            let before = ending.clone();
            let (result, events) = apply_power_row(
                &mut ending,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &args,
                None,
            );
            assert_eq!(result, Ok(()));
            assert_eq!(ending, before);
            assert!(events.is_empty());

            // Beside a live Borrowed Time the listener order is unknown.
            let mut borrowed = curious_fixture(atom, 99);
            borrowed.powers.set(PowerId::BorrowedTime, SlotWire::Int, 1);
            let before = borrowed.clone();
            let (result, _) = apply_power_row(
                &mut borrowed,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &args,
                None,
            );
            assert_eq!(
                result,
                Err(EngineRefusal::MalformedArgs(
                    "Curious beside Borrowed Time energy-cost listener order"
                ))
            );
            assert_eq!(borrowed, before);

            // #3437: Spiked Gauntlets is a relic, so it always folds after
            // Curious; the application proceeds beside it.
            let (catalog, spec, args, atom) =
                curious_mad_science(upgrade, &[crate::ids::RelicId::RelicSpikedGauntlets]);
            let mut gauntlets = curious_fixture(atom, 99);
            let (result, _) = apply_power_row(
                &mut gauntlets,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &args,
                None,
            );
            assert_eq!(result, Ok(()));
            assert_eq!(gauntlets.powers.value(PowerId::Curious), 1);
        }

        // Any other program refuses: an Expertise Mad Science, a Synthesis.
        let mut builder = CatalogBuilder::new();
        assert!(
            builder.set_mad_science_variant(
                crate::catalog::MadScienceVariant::from_saved(3, 7).unwrap()
            )
        );
        let expertise = builder
            .intern(CardIdentity {
                id: CardId::MadScience,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let synthesis = builder
            .intern(CardIdentity {
                id: CardId::Synthesis,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        for atom in [expertise, synthesis] {
            let spec = *catalog.spec(atom).unwrap();
            let mut state = curious_fixture(atom, 99);
            let before = state.clone();
            let (result, _) = apply_power_row(
                &mut state,
                &catalog,
                &spec,
                StepKind::MadScienceCurious,
                &[CompiledArg::I(1)],
                None,
            );
            assert_eq!(
                result,
                Err(EngineRefusal::MalformedArgs("mad_science_curious"))
            );
            assert_eq!(state, before);
        }
    }

    /// #3427 witness: Borrowed Time refuses while a Curious is live (the
    /// other half of the listener-order refusal), and still applies without
    /// one.
    #[test]
    fn borrowed_time_refuses_beside_a_live_curious() {
        let identity = CardIdentity {
            id: CardId::BorrowedTime,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let compiled = catalog
            .steps(&spec)
            .iter()
            .find(|step| step.kind == StepKind::BorrowedTime)
            .copied()
            .unwrap();
        let args = catalog.args(compiled.args).to_vec();

        let mut plain = curious_fixture(atom, 99);
        let (result, _) = apply_power_row(
            &mut plain,
            &catalog,
            &spec,
            StepKind::BorrowedTime,
            &args,
            None,
        );
        assert_eq!(result, Ok(()));
        assert!(plain.powers.value(PowerId::BorrowedTime) > 0);

        let mut curious = curious_fixture(atom, 99);
        curious.powers.set(PowerId::Curious, SlotWire::Int, 1);
        let before = curious.clone();
        let (result, _) = apply_power_row(
            &mut curious,
            &catalog,
            &spec,
            StepKind::BorrowedTime,
            &args,
            None,
        );
        assert_eq!(
            result,
            Err(EngineRefusal::MalformedArgs(
                "Borrowed Time beside Curious energy-cost listener order"
            ))
        );
        assert_eq!(curious, before);

        // A combat that is ending writes nothing either way.
        let mut ending = curious_fixture(atom, 0);
        ending.powers.set(PowerId::Curious, SlotWire::Int, 1);
        let before = ending.clone();
        let (result, _) = apply_power_row(
            &mut ending,
            &catalog,
            &spec,
            StepKind::BorrowedTime,
            &args,
            None,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(ending, before);
    }

    /// #3515: Knockdown, Fight Me's enemy `+1`, Tracking and Sentry Mode each
    /// await one `PowerCmd.Apply`, which returns at `IsEnding`
    /// (`<Apply>d__1`1` 0x3ef988 IL_0025). Their writers used to read
    /// `history.over`; while the combat is ending before the over latch each
    /// body is a no-op, and the Adaptable-vetoed control writes.
    #[test]
    fn template_power_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        let mut template = HotState::at_defaults();
        template.hp = 50;
        let rows: [(StepKind, CardId, &[CompiledArg], bool); 3] = [
            (
                StepKind::Knockdown,
                CardId::Knockdown,
                &[CompiledArg::I(2)],
                true,
            ),
            (
                StepKind::StrengthEnemy,
                CardId::FightMe,
                &[CompiledArg::I(1)],
                true,
            ),
            (
                StepKind::Tracking,
                CardId::Tracking,
                &[CompiledArg::I(50)],
                false,
            ),
        ];
        for (kind, id, args, targeted) in rows {
            crate::engine::damage::assert_ending_window_gate(
                &template,
                &format!("{id:?}"),
                |s, t| run_card(kind, id, 0, s, targeted.then_some(t), args, &mut Vec::new()),
            );
        }
        let (template, catalog) = sentry_mode_fixture(0);
        crate::engine::damage::assert_ending_window_gate(&template, "Sentry Mode", |s, _| {
            apply_sentry_fixture(s, &catalog, 0, &mut Vec::new())
        });
    }
}
