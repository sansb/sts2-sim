//! Card-step bodies for the `content/cards/silent_rare.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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

use super::StepCtx;
use crate::catalog::{CardIdentity, CardTargetType, CompiledArg};
use crate::content_tables::{Arg, CardRow};
use crate::engine::EngineRefusal;
use crate::engine::damage::{
    apply_card_monster_debuff, apply_card_monster_debuff_with_catalog,
    apply_card_monster_strength_delta, ensure_poison_instance_uid, player_attack_results_from_card,
    poison_identity_state_is_exact, trigger_poison_identity_null_applier,
};
use crate::engine::play::unique_live_card_location;
use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, MiseryToken, PileId};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::BulletTimeExact,
    StepKind::FanOfKnivesExact,
    StepKind::GenerateShivsThenInkyExact,
    StepKind::KnifeTrapExact,
    StepKind::MalaiseX,
    StepKind::OutbreakExact,
    StepKind::ShadowStepDiscardExact,
    StepKind::ShadowStepPowerExact,
    StepKind::StormOfSteelExact,
    StepKind::TheHuntExact,
];

/// Bullet Time's live-Hand SetToFreeThisTurn transaction.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The body re-reads the complete live Hand
/// on every invocation, skips Energy-X cards, and appends
/// `SetToFreeThisTurn`'s ordered Energy/Star rows to every other exact physical
/// card. The complete Hand is committed before the additive NoDraw write.
///
/// Current v0.111.0 native authority: installed `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `BulletTime/<OnPlay>d__1::MoveNext` RVA `0x38fc74` obtains Hand at IL
/// `0x009e-0x00aa`, skips `EnergyCost.CostsX` at `0x00bf-0x00cb`, applies
/// `SetToFreeThisTurn` at `0x00cd-0x00cf`, then applies one NoDrawPower at
/// `0x00ec-0x0114`. The shared card-state writer preserves duplicate rows,
/// including replayed writes and the Star-only row on a negative-base card.
pub(crate) fn bullet_time_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("bullet_time_exact program"));
    };
    if !matches!(
        (
            ctx.spec.identity.id,
            ctx.spec.identity.upgrade,
            ctx.args,
            ctx.target,
            ctx.selection,
            ctx.x_value,
        ),
        (CardId::BulletTime, 0 | 1, [], None, None, 0)
    ) || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::BulletTimeExact
        || !ctx.catalog.args(program.args).is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("bullet_time_exact"));
    }
    let (source_pile, source_index) = unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("bullet_time_exact source"));
    }

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let mut candidates = Vec::new();
        for (index, card) in state.piles.get(PileId::Hand).as_slice().iter().enumerate() {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if !spec.x_cost {
                candidates.push((index, card.uid, spec.cost));
            }
        }
        for (_, uid, _) in &candidates {
            state
                .card_states
                .get(*uid)
                .free_star_cost_this_turn_or_played_rows
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow(
                    "Bullet Time temporary Star rows",
                ))?;
        }
        let current_no_draw = state.powers.value(PowerId::NoDraw);
        if current_no_draw < 0 {
            return Err(EngineRefusal::MalformedArgs("Bullet Time NoDraw state"));
        }
        let updated_no_draw = current_no_draw
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("Bullet Time NoDraw"))?;

        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            state,
            PowerId::NoDraw,
            current_no_draw,
            updated_no_draw,
        )?;

        for (index, uid, energy_base) in candidates {
            state.piles.get_mut(PileId::Hand).make_mut()[index].flags |=
                CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state
                .card_states
                .set_to_free_this_turn(uid, energy_base)
                .expect("the complete Bullet Time write was preflighted");
        }
        state.exact_piles = true;
        state
            .powers
            .set(PowerId::NoDraw, SlotWire::Int, updated_no_draw);
        crate::engine::damage::note_power(
            events,
            crate::engine::Subject::Player,
            PowerId::NoDraw,
            updated_no_draw,
        );
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(&mut probe, ctx.catalog, &mut Vec::new())?;
    apply(ctx.state, ctx.catalog, ctx.events)
}

/// `fan_of_knives_exact` — install the unique power before Shiv generation.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `FanOfKnivesPower::get_Type` RVA `0xa229a` returns type 1 and
/// `get_StackType` RVA `0xa229d` returns Unique (2). The card coroutine
/// `FanOfKnives/<OnPlay>d__8::MoveNext` RVA `0x39d294` awaits ApplyPower at
/// IL `0x002c..0x00a9`, then loops over `Shiv::CreateInHand` at
/// IL `0x00b6..0x0129`, four times at L0 and five times at L1.
/// Python: `_run_steps_inner` (frozen, deleted #2827) authenticates the same complete program
/// and stores the unique marker before the following Shiv-generator step.
///
/// v0.111.0 `FanOfKnives/<OnPlay>` RVA `0x39d294` awaits `PowerCmd.Apply<FanOfKnivesPower>` at IL_004f.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn fan_of_knives_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !fan_of_knives_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [power, generate]
            if power.kind == StepKind::FanOfKnivesExact
                && ctx.catalog.args(power.args) == ctx.args
                && generate.kind == StepKind::GenerateFixedShivs
                && matches!(ctx.catalog.args(generate.args), [CompiledArg::I(4 | 5)]))
        || !matches!(ctx.args, [CompiledArg::I(1)])
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.spec.cost != 2
        || !ctx.spec.is_power
        || !ctx.spec.playable
        || ctx.spec.targeted
        || ctx.spec.target_type != CardTargetType::SelfTarget
        || ctx.spec.x_cost
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("fan_of_knives_exact"));
    }
    let (source_pile, source_index) = unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
        EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        },
    )?;
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || ctx
            .catalog
            .atom(&CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            })
            .is_none()
    {
        return Err(EngineRefusal::MalformedArgs("fan_of_knives_exact source"));
    }
    match ctx.state.powers.get(PowerId::FanOfKnives) {
        None if !crate::engine::damage::damage_combat_is_ending(ctx.state) => {
            ctx.state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
            crate::engine::damage::note_power(
                ctx.events,
                crate::engine::Subject::Player,
                PowerId::FanOfKnives,
                1,
            );
        }
        None => {}
        Some(slot) if slot.wire == SlotWire::Int && slot.value == 1 => {}
        Some(_) => return Err(EngineRefusal::MalformedArgs("Fan of Knives power state")),
    }
    Ok(())
}

pub(crate) fn fan_of_knives_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::FanOfKnives,
            0,
            [
                crate::content_tables::Step {
                    kind: StepKind::FanOfKnivesExact,
                    args: [Arg::I(1)]
                },
                crate::content_tables::Step {
                    kind: StepKind::GenerateFixedShivs,
                    args: [Arg::I(4)]
                },
            ]
        ) | (
            CardId::FanOfKnives,
            1,
            [
                crate::content_tables::Step {
                    kind: StepKind::FanOfKnivesExact,
                    args: [Arg::I(1)]
                },
                crate::content_tables::Step {
                    kind: StepKind::GenerateFixedShivs,
                    args: [Arg::I(5)]
                },
            ]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// `flanking_exact` — not modeled. **Escalated (#1340 triage).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) applies the debuff with player-applier
/// identity; `_apply_flanking_debuff` stores the keyed instance and
/// `_flanking_multiplier` folds it into later powered attack damage.
///
/// Escalated: the native monster power is keyed by its player applier, while
/// Rust has only Misery's scalar Flanking token vocabulary. Player attack
/// damage has neither the per-applier instance store nor its multiplier
/// consumer, and multiplayer-only card rows remain outside admission.
/// ESCALATED-ON: power-variant-absent(PowerId::Flanking)
pub(crate) fn flanking_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::FlankingExact))
}

/// Blade of Ink's exact plural L0-Shiv generation and synchronous Inky suffix.
///
/// Current v0.111.0 native authority: `BladeOfInk/<OnPlay>d__1::MoveNext`
/// RVA `0x38ccb4` calls the count overload of `Shiv::CreateInHand` (RVA
/// `0x3bb20c`) with 2/3, awaits one plural generated-card transaction (RVA
/// `0x3e2f0c`), then synchronously enchants every returned object with Inky(1).
/// The plural helper preallocates the complete list and records every member
/// even if an earlier generated callback ends combat; only the nested pile Add
/// is ending-gated. CardCmd.Enchant has no corresponding ending gate.
/// Python: `_run_steps_inner` (frozen, deleted #2827) calls
/// `add_generated_shivs_then_enchant_inky`, the same exact plural helper and
/// synchronous atom-only rewrite over its returned UID sequence.
pub(crate) fn generate_shivs_then_inky_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(count)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_inky_exact args",
        ));
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_inky_exact program",
        ));
    };
    let expected = match ctx.spec.identity.upgrade {
        0 => 2,
        1 => 3,
        _ => {
            return Err(EngineRefusal::MalformedArgs(
                "generate_shivs_then_inky_exact upgrade",
            ));
        }
    };
    if ctx.spec.identity.id != CardId::BladeOfInk
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || *count != expected
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::GenerateShivsThenInkyExact
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_inky_exact",
        ));
    }
    // Native Blade owns a local Player-model generated-card transaction and
    // immediately mutates those returned local physical objects. The compact
    // multiplayer quotient cannot represent that ownership/callback split in
    // a fight with a remote Player, so reject before inspecting or reserving
    // any source/generated uid and before publishing history, RNG, pile, or
    // hook work.
    if ctx.state.multiplayer_ally_key != 0 {
        return Err(EngineRefusal::MalformedArgs("Blade of Ink remote state"));
    }
    let (source_pile, source_index) = unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || source.flags & crate::hot::CARD_FLAG_LEGACY != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_inky_exact source",
        ));
    }
    let count: usize = (*count)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("Blade of Ink count"))?;
    crate::engine::cards::inject_generated_blade_inky_shivs(
        ctx.state,
        ctx.catalog,
        count,
        ctx.events,
    )
}

/// `knife_trap_exact` — frozen Exhaust-Shiv target-preserving AutoPlay.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) calls
/// `_knife_trap_frozen_autoplay_batch`, whose frame authenticates each
/// captured Exhaust uid/payload, the original target identity, and the source
/// upgrade flag across nested selections and reloads.
///
/// The family-callable transaction freezes exact Exhaust-pile Shiv identities,
/// optionally upgrades each then-live instance, and serially AutoPlays them at
/// the original `(slot, uid)` target. A frozen dead target remains valid; the
/// batch consumes no target RNG and rejects duplicate/malformed identities.
pub(crate) fn knife_trap_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    crate::engine::play::knife_trap_shivs(ctx)
}

/// `("malaise_x", upgrade_bonus)` — Malaise.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The resolved X plus the upgrade bonus is
/// applied first as signed monster Strength, then as Weak. A zero amount is a
/// complete no-op; each application independently observes target liveness.
/// Artifact and Unsettling Lamp are outside the admitted relic/power surface,
/// so the printed amount reaches both supported scalar powers unchanged.
///
/// `Malaise/<OnPlay>d__9` RVA `0x3ab428` awaits `PowerCmd.Apply<StrengthPower>`
/// (IL_0109) and then `Apply<WeakPower>` (IL_0190) on the target. Each
/// `<Apply>d__1`1` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a); the
/// Strength writer still reads `history.over`, so the entry gate is the shared
/// IsEnding projection (#3515).
pub(crate) fn malaise_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(bonus)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("malaise_x"));
    };
    let amount = ctx
        .x_value
        .checked_add(*bonus)
        .ok_or(EngineRefusal::CounterOverflow("malaise_x amount"))?;
    if amount == 0 {
        return Ok(());
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("malaise_x amount"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if monster.hp <= 0 {
        return Ok(());
    }
    let strength_delta = amount
        .checked_neg()
        .ok_or(EngineRefusal::CounterOverflow("malaise_x strength"))?;
    apply_card_monster_strength_delta(ctx.state, target, strength_delta, ctx.events)?;
    if ctx.state.history.over || ctx.state.monsters[target].hp <= 0 {
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

/// Outbreak's exact synchronous two-wave Poison transaction.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) enters `_begin_outbreak_exact`;
/// `_advance_outbreak_frame` freezes the first living roster, completes every
/// card-sourced Poison application and its listeners, then takes a fresh
/// second snapshot and finishes each retained Poison trigger series serially.
/// Rust deliberately keeps both identity vectors and retained trigger models
/// stack-local: the whole body is clone-rehearsed and any composition that
/// opens a pending continuation refuses before publication.
pub(crate) fn outbreak_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected: i32 = match ctx.spec.identity.upgrade {
        0 => 9,
        1 => 12,
        _ => return Err(EngineRefusal::MalformedArgs("outbreak_exact upgrade")),
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("outbreak_exact program"));
    };
    if ctx.spec.identity.id != CardId::Outbreak
        || ctx.spec.cost != 3
        || !ctx.spec.is_skill
        || ctx.spec.is_power
        || ctx.spec.is_attack
        || ctx.spec.targeted
        || ctx.spec.target_type != CardTargetType::AllEnemies
        || ctx.spec.x_cost
        || !ctx.spec.playable
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !matches!(ctx.args, [CompiledArg::I(value)] if *value == i64::from(expected))
        || program.kind != StepKind::OutbreakExact
        || ctx.catalog.args(program.args) != ctx.args
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(CardId::Outbreak, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || ctx
            .catalog
            .hooks()
            .relics()
            .iter()
            .any(|relic| !crate::engine::admission::IMPLEMENTED_RELICS.contains(relic))
        || ctx.state.pending.is_some()
        || !crate::engine::play::synchronous_autoplay_child_context_is_exact(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
        )
    {
        return Err(EngineRefusal::MalformedArgs("outbreak_exact source/state"));
    }
    let (pile, index) = unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("outbreak_exact source"));
    }

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        source_uid: u32,
        amount: i32,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        if state.multiplayer_ally_key != 0 || !poison_identity_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs(
                "Outbreak Poison identity/topology",
            ));
        }
        if state
            .powers
            .get(PowerId::Accelerant)
            .is_some_and(|slot| slot.wire != SlotWire::Int || slot.value < 0)
            || state.monsters.iter().any(|monster| {
                monster
                    .powers
                    .get(PowerId::Artifact)
                    .is_some_and(|slot| slot.wire != SlotWire::Int || slot.value < 0)
            })
        {
            return Err(EngineRefusal::MalformedArgs(
                "Outbreak modifier power wire/amount",
            ));
        }
        let hopper_reachable = state.card_states.hopper().is_some()
            || state
                .monsters
                .iter()
                .any(|monster| monster.kind == crate::ids::MonsterKind::ThievingHopper);
        if hopper_reachable
            && (!crate::engine::monsters::thieving_hopper_state_is_valid(state)
                || !crate::engine::monsters::thieving_hopper_deck_payload_is_exact(state, catalog))
        {
            return Err(EngineRefusal::MalformedArgs("Outbreak Hopper state"));
        }
        if state.card_states.dampen().is_some()
            && !crate::engine::cards::dampen_state_is_exact(state, catalog)
        {
            return Err(EngineRefusal::MalformedArgs("Outbreak Dampen state"));
        }
        if crate::engine::cards::aeonglass_reachable(state, catalog)
            && !crate::engine::cards::aeonglass_state_is_exact(state, catalog)
        {
            return Err(EngineRefusal::MalformedArgs("Outbreak Aeonglass state"));
        }
        let mut identities = Vec::with_capacity(state.monsters.len());
        for monster in state.monsters.iter() {
            let identity = (monster.slot, monster.uid);
            if identities.contains(&identity) {
                return Err(EngineRefusal::MalformedArgs("Outbreak creature identity"));
            }
            identities.push(identity);
        }
        let first: Vec<_> = identities
            .iter()
            .copied()
            .filter(|identity| {
                state
                    .monsters
                    .iter()
                    .find(|monster| (monster.slot, monster.uid) == *identity)
                    .is_some_and(|monster| monster.hp > 0)
            })
            .collect();
        for identity in first {
            if state.history.over {
                break;
            }
            let Some(target) = state
                .monsters
                .iter()
                .position(|monster| (monster.slot, monster.uid) == identity && monster.hp > 0)
            else {
                continue;
            };
            apply_card_monster_debuff_with_catalog(
                state,
                catalog,
                target,
                PowerId::Poison,
                MiseryToken::Poison,
                amount,
                events,
            )?;
        }
        if !poison_identity_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs("Outbreak Poison successor"));
        }
        let mut second = Vec::with_capacity(state.monsters.len());
        for monster in state.monsters.iter().filter(|monster| monster.hp > 0) {
            let identity = (monster.slot, monster.uid);
            if second.contains(&identity) {
                return Err(EngineRefusal::MalformedArgs(
                    "Outbreak second creature identity",
                ));
            }
            second.push(identity);
        }
        for identity in second {
            if state.history.over {
                break;
            }
            if let Some(target) = state.monsters.iter().position(|monster| {
                (monster.slot, monster.uid) == identity
                    && monster.hp > 0
                    && monster.powers.value(PowerId::Poison) > 0
            }) {
                ensure_poison_instance_uid(state, target)?;
            }
            trigger_poison_identity_null_applier(state, catalog, identity, events)?;
        }
        if state.pending.is_some()
            || !crate::engine::play::synchronous_autoplay_child_context_is_exact(
                state, catalog, source_uid,
            )
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        Ok(())
    }

    crate::engine::play::rehearse_preserving_active_plays(|| {
        let mut probe = ctx.state.clone();
        apply(
            &mut probe,
            ctx.catalog,
            ctx.source_uid,
            expected,
            &mut Vec::new(),
        )
    })?;
    apply(ctx.state, ctx.catalog, ctx.source_uid, expected, ctx.events)
}

/// Whether a stable public root can execute or retain an Outbreak body.
pub(crate) fn outbreak_is_reachable(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
) -> bool {
    catalog.outbreak_reachable()
        || crate::boundary::PILE_FIELDS.iter().any(|(pile, _)| {
            state.piles.get(*pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::Outbreak))
            })
        })
}

/// Shadow Step's frozen live-Hand discard transaction.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the first exact step and
/// `_shadow_step_discard_exact` freezes the complete live Hand's
/// physical UIDs before the ordered plural discard and Sly AutoPlay tail.
///
/// Current v0.111.0 native authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`;
/// the current constructor/upgrade/body evidence and the shared plural
/// Discard/Sly ordering are recorded in `python/ENCOUNTER_MECHANICS.md` under
/// Issue #1751. A Sly child that needs selection would require a persisted
/// FrozenAutoBatch above the outer CardPlay, so boundary and direct execution
/// both refuse that composition before publishing an approximation.
pub(crate) fn shadow_step_discard_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    validate_shadow_step(ctx, StepKind::ShadowStepDiscardExact)?;
    let frozen = ctx.state.piles.get(PileId::Hand).as_slice().to_vec();
    crate::engine::draw::discard_frozen_hand(ctx.state, ctx.catalog, &frozen, ctx.events)
}

/// Apply Shadow Step's one delayed Type-1 wrapper stack.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) applies the marker only while combat is
/// live. `_run_after_side_turn_start_power_order` converts its complete amount to
/// additive Double Damage and `_dispatch_after_side_turn_end_power`
/// decrements one duration.
///
/// The mutation helper publishes the ordinary AfterPowerAmountChanged event
/// seam after authenticating the represented listener order. Burst re-enters
/// this complete two-step body, so its second frozen Hand is naturally empty
/// and its second wrapper stack is still applied.
///
/// v0.111.0 `ShadowStep/<OnPlay>` RVA `0x3ba870` awaits `PowerCmd.Apply<ShadowStepPower>` at IL_00b8.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn shadow_step_power_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    validate_shadow_step(ctx, StepKind::ShadowStepPowerExact)?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    crate::engine::damage::modify_shadow_step_power_amount(
        ctx.state,
        PowerId::ShadowStep,
        1,
        ctx.events,
    )
}

fn validate_shadow_step(ctx: &StepCtx<'_>, current: StepKind) -> Result<(), EngineRefusal> {
    let [discard, power] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("shadow_step_exact program"));
    };
    let current_args_ok = match current {
        StepKind::ShadowStepDiscardExact => ctx.args.is_empty(),
        StepKind::ShadowStepPowerExact => matches!(ctx.args, [CompiledArg::I(1)]),
        _ => false,
    };
    if ctx.spec.identity.id != CardId::ShadowStep
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !current_args_ok
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || discard.kind != StepKind::ShadowStepDiscardExact
        || !ctx.catalog.args(discard.args).is_empty()
        || power.kind != StepKind::ShadowStepPowerExact
        || ctx.catalog.args(power.args) != [CompiledArg::I(1)]
    {
        return Err(EngineRefusal::MalformedArgs("shadow_step_exact"));
    }
    let (pile, index) = unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if ctx
        .catalog
        .spec(ctx.state.piles.get(pile).as_slice()[index].atom)
        != Some(ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs("shadow_step_exact source"));
    }
    Ok(())
}

/// `storm_of_steel_exact` — Storm of Steel's exact frozen-Hand transaction.
///
/// Current v0.111.0 IL is cited at the two shared primitives below. The body
/// admits only the canonical L0/L1 rows, freezes Hand after the exact source
/// has entered its play route, serially discards that immutable UID batch and
/// completes its Sly tail, then bulk-creates the frozen count as L0 Shivs.
/// Storm+ upgrades only the exact returned live UIDs. Catalog construction
/// closes L0 for both rows and L1 independently for the upgraded row.
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the row and delegates to
/// `_storm_discard_exact`; its bulk implementation is citation-safe at
/// EOF so this correction does not renumber unrelated source authorities.
pub(crate) fn storm_of_steel_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(upgrade)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("storm_of_steel_exact"));
    };
    if ctx.spec.identity.id != CardId::StormOfSteel
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *upgrade != i64::from(ctx.spec.identity.upgrade)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("storm_of_steel_exact"));
    }
    let upgrade: u8 = (*upgrade)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("Storm upgrade"))?;

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        spec: &crate::catalog::CardSpec,
        source_uid: u32,
        upgrade: u8,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let (source_pile, source_index) =
            crate::engine::play::unique_live_card_location(state, source_uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(source_pile).as_slice()[source_index];
        if catalog.spec(source.atom) != Some(spec) {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let frozen = state.piles.get(PileId::Hand).as_slice().to_vec();
        let count = frozen.len();
        crate::engine::draw::discard_frozen_hand(state, catalog, &frozen, events)?;
        // A replay-rooted Sly child may itself suspend.  Its exact frozen
        // batch owns the discarded-card cursor, so the plural Shiv suffix
        // must wait until that batch drains rather than running while the
        // nested child is still parked.  The batch completion seam calls the
        // authenticated helper below exactly once with this frozen count.
        if state.pending.is_some() {
            if !crate::engine::replay_lifecycle_is_active() {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            return Ok(());
        }
        if state.history.over {
            return Ok(());
        }
        crate::engine::cards::inject_generated_storm_shivs(state, catalog, count, upgrade, events)
    }

    let mut probe = ctx.state.clone();
    apply(
        &mut probe,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        upgrade,
        &mut Vec::new(),
    )?;
    apply(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        upgrade,
        ctx.events,
    )
}

/// Finish the deferred plural-Shiv suffix of one authenticated Storm batch.
///
/// The persistent owner and its exact canonical row are rederived by the
/// replay driver before this helper is entered.  Rechecking the live physical
/// owner here prevents a displaced or substituted source from minting cards;
/// `count` is the immutable captured-Hand cardinality owned by the completed
/// Sly batch.
#[cold]
#[inline(never)]
pub(crate) fn finish_storm_of_steel_after_sly_batch(
    state: &mut crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    source_uid: u32,
    count: usize,
    frozen_upgrade: u8,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let (source_pile, source_index) =
        crate::engine::play::unique_live_card_location(state, source_uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let source = state.piles.get(source_pile).as_slice()[source_index];
    if catalog.spec(source.atom) != Some(spec)
        || spec.identity.id != CardId::StormOfSteel
        || !matches!(spec.identity.upgrade, 0 | 1)
        || !matches!(frozen_upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.history.over {
        return Ok(());
    }
    crate::engine::cards::inject_generated_storm_shivs(
        state,
        catalog,
        count,
        frozen_upgrade,
        events,
    )
}

/// The Hunt's exact fatal-result reward and ending-gated marker body.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `TheHunt/<OnPlay>d__9::MoveNext` RVA `0x3c2c10` snapshots
/// `ShouldOwnerDeathTriggerFatal`, awaits one attack, and adds a three-option
/// `CardReward` on the matching killed result before awaiting
/// `PowerCmd.Apply<TheHuntPower>`. `CardReward::.ctor` RVA `0x5c950` stores
/// only the count/options; its identities and Rewards RNG are deferred until
/// `Populate` RVA `0x5ca84` after combat. `PowerCmd.Apply` RVA `0x3ef988`
/// returns at its ending gate before mutation, so final lethal carries the
/// reward but no marker. Python `_run_steps_inner` (frozen, deleted #2827) has the same order.
///
/// After a killing hit `TheHunt/<OnPlay>d__9` RVA `0x3c2c10` adds the card
/// reward with no combat gate (`CombatRoom::AddExtraReward`, IL_01d2) and then
/// awaits `PowerCmd.Apply<TheHuntPower>` (IL_01fa), which returns at `IsEnding`
/// (`<Apply>d__1`1` `0x3ef988` IL_0025-002a). The marker follows a kill, and the
/// engine latches `history.over` at a killing blow that leaves no living
/// primary, so `history.over` is that gate here (#3515).
pub(crate) fn the_hunt_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    #[derive(Clone, Copy)]
    struct Plan {
        damage: i64,
        target: usize,
        target_uid: u32,
        should_trigger_fatal: bool,
    }

    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::TheHunt, 0, [CompiledArg::I(10)]) => 10,
        (CardId::TheHunt, 1, [CompiledArg::I(15)]) => 15,
        _ => return Err(EngineRefusal::MalformedArgs("the_hunt_exact")),
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("the_hunt_exact program"));
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::TheHuntExact
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs("the_hunt_exact exact row"));
    }
    let (source_pile, source_index) = unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
        EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        },
    )?;
    if ctx
        .catalog
        .spec(ctx.state.piles.get(source_pile).as_slice()[source_index].atom)
        != Some(ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "the_hunt_exact physical source",
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
    let plan = Plan {
        damage,
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
        if plan.should_trigger_fatal
            && results.iter().any(|result| {
                result.target == plan.target
                    && result.receiver_uid == plan.target_uid
                    && result.was_target_killed
            })
        {
            let apply_marker = !ctx.state.history.over;
            if apply_marker
                && !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
            {
                return Err(EngineRefusal::MalformedArgs(
                    "The Hunt after-power-amount-changed listener order",
                ));
            }
            ctx.state
                .fanouts
                .record_the_hunt_kill(apply_marker)
                .map_err(|_| EngineRefusal::CounterOverflow("The Hunt reward/marker"))?;
        }
        Ok(())
    }

    // The attack/death lifecycle can refuse before the deferred reward, and
    // either compact counter can overflow afterward. Rehearse the whole fused
    // body so the step itself never publishes a partial attack or reward.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CardEnchantment, CardIdentity, CatalogBuilder, RewardOdds, RewardPool};
    use crate::content_tables::{Arg, CARD_ROWS, INFERNAL_BLADE_ATTACK_POOL_V109};
    use crate::engine::admission::admit;
    use crate::engine::{Event, play::play_card};
    use crate::hot::{
        HotCard, HotMonster, HotState, LocalCostExpiration, LocalCostModifier,
        LocalCostModifierKind, LocalCostModifiers,
    };
    use crate::ids::{CardId, EnchantmentId, MonsterKind, RelicId};
    use crate::powers::SlotWire;
    use serde_json::json;

    const OWNED: [StepKind; 11] = [
        StepKind::BulletTimeExact,
        StepKind::FanOfKnivesExact,
        StepKind::FlankingExact,
        StepKind::GenerateShivsThenInkyExact,
        StepKind::KnifeTrapExact,
        StepKind::MalaiseX,
        StepKind::OutbreakExact,
        StepKind::ShadowStepDiscardExact,
        StepKind::ShadowStepPowerExact,
        StepKind::StormOfSteelExact,
        StepKind::TheHuntExact,
    ];

    const ESCALATED: [StepKind; 1] = [StepKind::FlankingExact];

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn outbreak_fixture(
        upgrade: u8,
        monster_hps: &[i32],
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::Outbreak, upgrade)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        for (index, hp) in monster_hps.iter().copied().enumerate() {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
            monster.max_hp = hp;
            monster.slot = i32::try_from(index).unwrap();
            monster.uid = u32::try_from(index + 10).unwrap();
            state.monsters_mut().push(monster);
        }
        (state, catalog, source)
    }

    #[test]
    fn outbreak_both_levels_apply_then_null_trigger_each_fresh_identity() {
        for (upgrade, amount) in [(0, 9), (1, 12)] {
            let (mut state, catalog, source) = outbreak_fixture(upgrade, &[100, 100]);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Poison, SlotWire::Int, 2);
            state.monsters_mut()[0].poison_uid = 0;
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(MiseryToken::Poison);
            state.next_poison_uid = 1;
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            let mut events = Vec::new();

            play_card(&mut state, &catalog, source.uid, None, None, &mut events).unwrap();

            assert_eq!(state.energy, 0);
            assert_eq!(state.next_poison_uid, 2);
            assert_eq!(state.monsters[0].hp, 99 - (amount + 2));
            assert_eq!(state.monsters[1].hp, 99 - amount);
            assert_eq!(state.monsters[0].powers.value(PowerId::Poison), amount + 1);
            assert_eq!(state.monsters[1].powers.value(PowerId::Poison), amount - 1);
            assert_eq!(state.monsters[0].poison_uid, 0);
            assert_eq!(state.monsters[1].poison_uid, 1);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        Event::PowerChanged {
                            power: PowerId::Poison,
                            ..
                        }
                    ))
                    .count(),
                4,
                "one apply and one null-applier decrement per target"
            );
        }
    }

    #[test]
    fn outbreak_sleight_fires_only_for_apply_and_null_decrements_keep_event_order() {
        let (mut state, catalog, source) = outbreak_fixture(0, &[200]);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        state.monsters_mut()[0].poison_uid = 0;
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        state.next_poison_uid = 1;
        state.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let mut events = Vec::new();

        play_card(&mut state, &catalog, source.uid, None, None, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 161);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 8);
        let observed: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::PowerChanged {
                    power: PowerId::Poison,
                    amount,
                    ..
                } => Some(("poison", *amount)),
                Event::MonsterDamaged { unblocked, .. } => Some(("damage", *unblocked)),
                _ => None,
            })
            .collect();
        assert_eq!(
            observed,
            [
                ("poison", 11),
                ("damage", 9),
                ("damage", 11),
                ("poison", 10),
                ("damage", 10),
                ("poison", 9),
                ("damage", 9),
                ("poison", 8),
            ],
            "the player-applied first wave finishes Sleight before the fresh trigger wave; null decrements publish once without firing it"
        );
    }

    #[test]
    fn outbreak_artifact_blocks_sleight_but_old_poison_triggers_and_lamp_resets() {
        let (mut blocked, catalog, source) = outbreak_fixture(0, &[100]);
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 3);
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        blocked.monsters_mut()[0].poison_uid = 0;
        blocked.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        blocked.next_poison_uid = 1;
        blocked
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            blocked
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        play_card(
            &mut blocked,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(blocked.monsters[0].hp, 97);
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Poison), 2);
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(blocked.next_poison_uid, 1);

        let (mut lamp, catalog, source) = outbreak_fixture(0, &[100]);
        lamp.fanouts.set_unsettling_lamp_available(true);
        play_card(&mut lamp, &catalog, source.uid, None, None, &mut Vec::new()).unwrap();
        assert_eq!(lamp.monsters[0].hp, 82);
        assert_eq!(lamp.monsters[0].powers.value(PowerId::Poison), 17);
        assert!(!lamp.fanouts.unsettling_lamp_available());
    }

    #[test]
    fn outbreak_dampen_body_is_frozen_and_replay_refreshes_the_restored_source() {
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        let (l0_atom, l1_atom) = builder
            .intern_dampen_card_pair(identity(CardId::Outbreak, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 5;
        for (uid, pile) in crate::engine::cards::DAMPEN_PILES.into_iter().enumerate() {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: u32::try_from(uid).unwrap(),
                atom: l1_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_HEXED,
            });
        }
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(0)));
        assert!(flail.random_ai.set_log(&[2, 0]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(1)));
        assert!(spectral.random_ai.set_log(&[0, 1]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 9);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        magi.loop_pos = 1;
        state.monsters_mut().extend([flail, spectral, magi]);
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        crate::engine::cards::apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let source = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(source.atom, l0_atom);

        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.card_states.dampen().is_none());
        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.monsters[2].hp, 0);
        assert_eq!(state.monsters[0].hp, 61);
        assert_eq!(state.monsters[1].hp, 50);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 19);
        assert_eq!(state.monsters[1].powers.value(PowerId::Poison), 19);
        let resolved = state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == source.uid)
            .unwrap();
        assert_eq!(resolved.atom, l1_atom);
    }

    #[test]
    fn outbreak_public_play_refuses_identity_listener_and_late_death_atomically() {
        let refuse = |mut state: HotState,
                      catalog: &crate::catalog::Catalog,
                      source: HotCard,
                      target: Option<u8>| {
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 49 }];
            let before_events = events.clone();
            assert!(
                play_card(&mut state, catalog, source.uid, target, None, &mut events,).is_err()
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        };

        let (mut future_uid, catalog, source) = outbreak_fixture(0, &[100]);
        future_uid.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        future_uid.monsters_mut()[0].poison_uid = 1;
        future_uid.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        future_uid.next_poison_uid = 1;
        refuse(future_uid, &catalog, source, None);

        let (mut malformed_negative_uid, catalog, source) = outbreak_fixture(0, &[100]);
        malformed_negative_uid.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        malformed_negative_uid.monsters_mut()[0].poison_uid = -2;
        malformed_negative_uid.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        malformed_negative_uid.next_poison_uid = 1;
        refuse(malformed_negative_uid, &catalog, source, None);

        let (mut duplicate_uid, catalog, source) = outbreak_fixture(0, &[100, 100]);
        for monster in duplicate_uid.monsters_mut() {
            monster.powers.set(PowerId::Poison, SlotWire::Int, 2);
            monster.poison_uid = 0;
            monster.misery_debuff_order.push(MiseryToken::Poison);
        }
        duplicate_uid.next_poison_uid = 1;
        refuse(duplicate_uid, &catalog, source, None);

        let (mut allocator_overflow, catalog, source) = outbreak_fixture(0, &[100]);
        allocator_overflow.next_poison_uid = i32::MAX;
        refuse(allocator_overflow, &catalog, source, None);

        let (mut poison_overflow, catalog, source) = outbreak_fixture(0, &[100]);
        poison_overflow.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, i32::MAX);
        poison_overflow.monsters_mut()[0].poison_uid = 0;
        poison_overflow.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        poison_overflow.next_poison_uid = 1;
        refuse(poison_overflow, &catalog, source, None);

        let (mut lamp_overflow, catalog, source) = outbreak_fixture(0, &[100]);
        lamp_overflow.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, i32::MAX - 10);
        lamp_overflow.monsters_mut()[0].poison_uid = 0;
        lamp_overflow.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        lamp_overflow.next_poison_uid = 1;
        lamp_overflow.fanouts.set_unsettling_lamp_available(true);
        refuse(lamp_overflow, &catalog, source, None);

        let (mut accelerant, catalog, source) = outbreak_fixture(0, &[100]);
        accelerant
            .powers
            .set(PowerId::Accelerant, SlotWire::Int, -1);
        refuse(accelerant, &catalog, source, None);

        let (mut accelerant_wire, catalog, source) = outbreak_fixture(0, &[100]);
        accelerant_wire
            .powers
            .set(PowerId::Accelerant, SlotWire::Bool, 1);
        refuse(accelerant_wire, &catalog, source, None);

        let (mut artifact_wire, catalog, source) = outbreak_fixture(0, &[100]);
        artifact_wire.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Bool, 1);
        refuse(artifact_wire, &catalog, source, None);

        let (mut dead_poison, catalog, source) = outbreak_fixture(0, &[0]);
        dead_poison.history.over = true;
        dead_poison.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        dead_poison.monsters_mut()[0].poison_uid = 0;
        dead_poison.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        dead_poison.next_poison_uid = 1;
        refuse(dead_poison, &catalog, source, None);

        let (mut listeners, catalog, source) = outbreak_fixture(0, &[100]);
        listeners
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 1);
        assert!(listeners.fanouts.set_after_power_amount_changed_order(&[
            PowerId::SleightOfFlesh,
            PowerId::SleightOfFlesh,
        ]));
        refuse(listeners, &catalog, source, None);

        let (mut multiplayer, catalog, source) = outbreak_fixture(0, &[100]);
        multiplayer.multiplayer_ally_key = 1;
        refuse(multiplayer, &catalog, source, None);

        let (targeted, catalog, source) = outbreak_fixture(0, &[100]);
        refuse(targeted, &catalog, source, Some(0));

        let mut invalid_axebot = HotMonster::new(MonsterKind::Axebot, 1);
        invalid_axebot.slot = 1;
        invalid_axebot.uid = 11;
        invalid_axebot.max_hp = 75;
        invalid_axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut late, catalog, source) = outbreak_fixture(0, &[100]);
        late.monsters_mut().push(invalid_axebot);
        refuse(late, &catalog, source, None);
    }

    #[test]
    fn outbreak_poison_identity_round_trips_and_clone_isolation_is_exact() {
        let (mut state, catalog, source) = outbreak_fixture(0, &[100, 100]);
        let initial = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        state = HotBoundary::from_canonical(&initial, &catalog).unwrap();
        assert_eq!(admit(&initial, &state, &catalog), Ok(()));
        play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        let canonical = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let canonical_digest = canonical.differential_digest();
        let round_trip = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        let projected = HotBoundary::try_to_canonical(&round_trip, &catalog).unwrap();
        assert_eq!(projected, canonical);
        assert_eq!(projected.differential_digest(), canonical_digest);
        assert_eq!(admit(&canonical, &round_trip, &catalog), Ok(()));

        let mut clone = round_trip.clone();
        clone.monsters_mut()[0].hp -= 1;
        assert_ne!(clone, round_trip);
        assert_eq!(round_trip.monsters[0].hp, 91);

        let mut malformed = canonical.clone();
        malformed.monsters[1].insert("poison_uid".to_owned(), json!(0));
        assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());

        let mut missing = canonical.clone();
        missing.monsters[0].remove("poison_uid");
        let legacy = HotBoundary::from_canonical(&missing, &catalog).unwrap();
        assert_eq!(legacy.monsters[0].poison_uid, -1);
        assert_eq!(
            HotBoundary::try_to_canonical(&legacy, &catalog).unwrap(),
            missing
        );
        assert_eq!(admit(&missing, &legacy, &catalog), Ok(()));

        let mut future = canonical.clone();
        future.player.insert("next_poison_uid".to_owned(), json!(1));
        assert!(HotBoundary::from_canonical(&future, &catalog).is_err());

        let mut malformed_hot = round_trip;
        malformed_hot.monsters_mut()[1].poison_uid = 0;
        assert!(HotBoundary::try_to_canonical(&malformed_hot, &catalog).is_err());

        let mut multiplayer = state;
        multiplayer.multiplayer_ally_key = 1;
        let refusal = admit(&canonical, &multiplayer, &catalog).unwrap_err();
        assert!(
            refusal.contains(crate::engine::admission::MissingCapability::ArgumentShape(
                "Outbreak solo Accelerant topology"
            ))
        );
        assert!(HotBoundary::try_to_canonical(&multiplayer, &catalog).is_err());
    }

    #[test]
    fn outbreak_forged_synchronous_parent_refuses_the_whole_public_play() {
        let (mut state, catalog, source) = outbreak_fixture(0, &[100]);
        let mut forged_parent = crate::hot::CardPlayRecord::pending_for_test(99);
        forged_parent.pending_choice = false;
        forged_parent.selection_kind = None;
        state.frames.push_card_play(&forged_parent).unwrap();
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 51 }];
        let before_events = events.clone();

        assert!(play_card(&mut state, &catalog, source.uid, None, None, &mut events,).is_err());
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn poison_identity_boundary_is_closed_without_an_outbreak_card() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_poison_uid = 1;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 30);
        target.max_hp = 30;
        target.uid = 10;
        target.powers.set(PowerId::Poison, SlotWire::Int, 3);
        target.poison_uid = 0;
        target.misery_debuff_order.push(MiseryToken::Poison);
        state.monsters_mut().push(target);
        let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let imported = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        assert_eq!(admit(&wire, &imported, &catalog), Ok(()));

        let mut future = state.clone();
        future.next_poison_uid = 0;
        assert!(HotBoundary::try_to_canonical(&future, &catalog).is_err());
        assert!(admit(&wire, &future, &catalog).unwrap_err().contains(
            crate::engine::admission::MissingCapability::ArgumentShape("Poison identity lifecycle")
        ));

        let mut corpse_wire = wire;
        corpse_wire.monsters[0].insert("hp".to_owned(), json!(0));
        assert!(HotBoundary::from_canonical(&corpse_wire, &catalog).is_err());
        let mut corpse = state;
        corpse.monsters_mut()[0].hp = 0;
        corpse.history.over = true;
        assert!(HotBoundary::try_to_canonical(&corpse, &catalog).is_err());
    }

    #[test]
    fn outbreak_fresh_second_snapshot_and_detached_replacement_semantics_are_exact() {
        let (mut phrog_state, catalog, source) = outbreak_fixture(0, &[9, 100]);
        phrog_state.monsters_mut()[0].kind = MonsterKind::PhrogParasite;
        phrog_state.monsters_mut()[0].max_hp = 101;
        phrog_state
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            phrog_state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        play_card(
            &mut phrog_state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(phrog_state.monsters.len(), 6);
        assert_eq!(phrog_state.monsters[0].hp, 0);
        assert!(phrog_state.monsters[2..].iter().all(|monster| {
            monster.kind == MonsterKind::Wriggler
                && monster.hp > 0
                && monster.powers.value(PowerId::Poison) == 0
        }));
        assert_eq!(phrog_state.monsters[1].hp, 82);
        assert_eq!(phrog_state.monsters[1].powers.value(PowerId::Poison), 8);

        let (mut waterfall, catalog, source) = outbreak_fixture(0, &[5]);
        waterfall.monsters_mut()[0].kind = MonsterKind::WaterfallGiant;
        waterfall.monsters_mut()[0].max_hp = 250;
        waterfall.monsters_mut()[0]
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 20);
        waterfall.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        play_card(
            &mut waterfall,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(waterfall.monsters[0].hp, 999_999_984);
        assert_eq!(waterfall.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(waterfall.monsters[0].poison_uid, -1);
        assert!(waterfall.monsters[0].is_about_to_blow());

        let (mut axebot, catalog, source) = outbreak_fixture(0, &[4]);
        axebot.monsters_mut()[0].kind = MonsterKind::Axebot;
        axebot.monsters_mut()[0].uid = 0;
        axebot.monsters_mut()[0].max_hp = 76;
        axebot.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, 2);
        axebot.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        play_card(
            &mut axebot,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(axebot.monsters[0].uid, 1);
        assert_eq!(axebot.monsters[0].powers.value(PowerId::Stock), 1);
        assert_eq!(axebot.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(axebot.monsters[0].poison_uid, -1);
    }

    #[test]
    fn outbreak_burst_replays_independent_lamp_lifecycles() {
        let (mut state, catalog, source) = outbreak_fixture(0, &[500]);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.fanouts.set_unsettling_lamp_available(true);

        play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.monsters[0].hp, 456);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 25);
        assert!(!state.fanouts.unsettling_lamp_available());

        let (mut auto, catalog, source) = outbreak_fixture(0, &[100]);
        let card = auto.piles.get_mut(PileId::Hand).make_mut().remove(0);
        auto.piles.get_mut(PileId::Draw).make_mut().push(card);
        auto.fanouts.set_unsettling_lamp_available(true);
        crate::engine::play::autoplay_collected_cards(
            &mut auto,
            &catalog,
            &[source],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(auto.energy, 3);
        assert_eq!(auto.history.card_plays_finished_combat, 1);
        assert_eq!(auto.monsters[0].hp, 82);
        assert_eq!(auto.monsters[0].powers.value(PowerId::Poison), 17);
        assert!(!auto.fanouts.unsettling_lamp_available());
    }

    #[test]
    fn havoc_autoplay_keeps_its_exact_parent_while_outbreak_runs_synchronously() {
        let havoc = identity(CardId::Havoc, 0);
        let outbreak = identity(CardId::Outbreak, 0);
        let mut builder = CatalogBuilder::new();
        let havoc_atom = builder.intern(havoc).unwrap();
        let outbreak_atom = builder.intern(outbreak).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 1;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: havoc_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: outbreak_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.max_hp = 100;
        target.uid = 10;
        state.monsters_mut().push(target);

        play_card(&mut state, &catalog, 1, None, None, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert!(state.frames.is_empty());
        assert_eq!(state.monsters[0].hp, 91);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 8);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 2);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 1);
    }

    #[test]
    fn mayhem_autopre_runs_outbreak_below_its_exact_phase_parent() {
        let mut builder = CatalogBuilder::new();
        let defend = builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        let outbreak = builder.intern(identity(CardId::Outbreak, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.next_card_uid = 7;
        state.powers.set(PowerId::Mayhem, SlotWire::Int, 1);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=5).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 6,
            atom: outbreak,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.max_hp = 100;
        target.uid = 10;
        state.monsters_mut().push(target);

        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert!(state.frames.is_empty());
        assert_eq!(state.monsters[0].hp, 91);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 8);
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 6)
        );
    }

    #[test]
    fn survivor_discard_autoplays_one_local_sly_outbreak_synchronously() {
        let survivor = identity(CardId::Survivor, 0);
        let outbreak = identity(CardId::Outbreak, 0);
        let defend = identity(CardId::DefendSilent, 0);
        let mut builder = CatalogBuilder::new();
        for identity in [survivor, outbreak, defend] {
            builder.intern_reachable(identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 1;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: catalog.atom(&survivor).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: catalog.atom(&outbreak).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.card_states.set_local_sly(2);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.max_hp = 100;
        target.uid = 10;
        state.monsters_mut().push(target);

        let selecting = crate::engine::apply_action(
            &state,
            &catalog,
            &crate::engine::Action::Play {
                uid: 1,
                target: None,
                selection: crate::engine::SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let completed = crate::engine::legal_actions(&selecting, &catalog)
            .into_iter()
            .filter_map(|action| crate::engine::apply_action(&selecting, &catalog, &action).ok())
            .find(|transition| transition.state.monsters[0].powers.value(PowerId::Poison) == 8)
            .expect("discarding local-Sly Outbreak executes its synchronous AutoPlay")
            .state;

        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.monsters[0].hp, 91);
        assert!(
            completed
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 2)
        );
    }

    #[test]
    fn tools_of_the_trade_discard_autoplays_local_sly_outbreak_synchronously() {
        let outbreak = identity(CardId::Outbreak, 0);
        let defend = identity(CardId::DefendSilent, 0);
        let mut builder = CatalogBuilder::new();
        for identity in [outbreak, defend] {
            builder.intern_reachable(identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.next_card_uid = 3;
        state.exact_piles = true;
        state.powers.set(PowerId::ToolsOfTheTrade, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_turn_start_hand_choice_order(&[PowerId::ToolsOfTheTrade])
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: catalog.atom(&outbreak).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.card_states.set_local_sly(1);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.max_hp = 100;
        target.uid = 10;
        state.monsters_mut().push(target);

        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert!(state.pending.is_some());
        let completed = (0..state.piles.get(PileId::Hand).len())
            .find_map(|choice| {
                let mut candidate = state.clone();
                crate::engine::turn::resume_turn_start_hand_choice(
                    &mut candidate,
                    &catalog,
                    choice as u32,
                    &mut Vec::new(),
                )
                .ok()
                .filter(|_| candidate.monsters[0].powers.value(PowerId::Poison) == 8)
                .map(|_| candidate)
            })
            .expect("Tools discards and autoplays the selected local-Sly Outbreak");

        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.monsters[0].hp, 91);
    }

    #[test]
    fn outbreak_zero_cleanup_and_snecko_skull_support_are_exact() {
        let (mut cleanup, catalog, source) = outbreak_fixture(0, &[1_000]);
        cleanup.powers.set(PowerId::Accelerant, SlotWire::Int, 20);
        play_card(
            &mut cleanup,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(cleanup.monsters[0].hp, 955);
        assert_eq!(cleanup.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(cleanup.monsters[0].poison_uid, -1);
        assert!(cleanup.monsters[0].misery_debuff_order.is_empty());

        for relic in [RelicId::RelicSneckoSkull, RelicId::RelicGremlinHorn] {
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity(CardId::Outbreak, 0)).unwrap();
            builder.set_relics(&[relic]).unwrap();
            let catalog = builder.build();
            let source = HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            let mut refused = HotState::at_defaults();
            refused.hp = 50;
            refused.max_hp = 50;
            refused.energy = 3;
            refused.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            refused.next_card_uid = 2;
            refused.piles.get_mut(PileId::Hand).make_mut().push(source);
            refused
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            refused.set_deep_relic_ownership(
                false,
                relic == RelicId::RelicSneckoSkull,
                false,
                false,
            );
            let before = refused.clone();
            let mut events = vec![Event::TurnBegan { turn: 50 }];
            let before_events = events.clone();
            let result = play_card(&mut refused, &catalog, source.uid, None, None, &mut events);
            if relic == RelicId::RelicSneckoSkull {
                assert_eq!(result, Ok(()));
                assert_ne!(refused, before);
                assert_eq!(refused.monsters[0].powers.value(PowerId::Poison), 9);
                assert_eq!(refused.monsters[0].hp, 90);
            } else {
                assert_eq!(result, Ok(()));
                assert_ne!(refused, before);
                assert_ne!(events, before_events);
                assert!(!refused.history.over);
                assert_eq!(refused.monsters[0].powers.value(PowerId::Poison), 8);
                assert_eq!(refused.monsters[0].hp, 91);
            }
        }
    }

    #[test]
    fn fan_of_knives_both_levels_apply_unique_before_generating_four_or_five_l0_shivs() {
        for (upgrade, generated) in [(0, 4), (1, 5)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(identity(CardId::FanOfKnives, upgrade))
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            // The Shiv below hits this live enemy; it also keeps the combat
            // live, so the Fan power applies (#3515).
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            state.hp = 50;
            state.energy = 20;
            state.next_card_uid = 3;
            state.piles.get_mut(PileId::Hand).make_mut().extend([
                HotCard {
                    uid: 1,
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 2,
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::FanOfKnives), 1);
            assert!(state.exact_piles, "the equal-key second Fan remains live");
            assert_eq!(state.piles.get(PileId::Hand).len(), generated + 1);
            assert!(
                state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .filter(|card| card.uid != 2)
                    .all(|card| {
                        catalog
                            .spec(card.atom)
                            .is_some_and(|spec| spec.identity == identity(CardId::Shiv, 0))
                    })
            );
            assert!(matches!(
                events.iter().find(|event| matches!(
                    event,
                    Event::PowerChanged {
                        power: PowerId::FanOfKnives,
                        ..
                    }
                )),
                Some(Event::PowerChanged { amount: 1, .. })
            ));

            play_card(&mut state, &catalog, 2, None, None, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::FanOfKnives), 1);
            assert_eq!(state.piles.get(PileId::Hand).len(), generated * 2);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        Event::PowerChanged {
                            power: PowerId::FanOfKnives,
                            ..
                        }
                    ))
                    .count(),
                1,
                "Unique reapplication must not publish a second power change"
            );

            let generated_uid = state.piles.get(PileId::Hand).as_slice()[0].uid;
            play_card(&mut state, &catalog, generated_uid, None, None, &mut events).unwrap();
            assert_eq!(state.monsters[0].hp, 16, "generated Shiv reads live Fan");
        }
    }

    #[test]
    fn blade_of_ink_both_levels_preallocate_and_enchant_exact_l0_shivs() {
        for (upgrade, generated) in [(0, 2_usize), (1, 3)] {
            let mut builder = CatalogBuilder::new();
            let blade = builder
                .intern(identity(CardId::BladeOfInk, upgrade))
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            // A live enemy: with none, combat is ending and plural Shiv
            // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.hp = 50;
            state.energy = 3;
            state.inky_attack_damage = 0;
            state.next_card_uid = 10;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: blade,
                flags: 0,
            });

            play_card(&mut state, &catalog, 1, None, None, &mut Vec::new()).unwrap();

            let hand = state.piles.get(PileId::Hand).as_slice();
            assert_eq!(hand.len(), generated);
            assert_eq!(
                hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
                (10..10 + generated as u32).collect::<Vec<_>>()
            );
            assert!(hand.iter().all(|card| {
                catalog.spec(card.atom).is_some_and(|spec| {
                    spec.identity
                        == CardIdentity {
                            id: CardId::Shiv,
                            upgrade: 0,
                            enchantment: Some(CardEnchantment {
                                id: EnchantmentId::Inky,
                                amount: 1,
                            }),
                        }
                })
            }));
            assert_eq!(state.next_card_uid, 10 + generated as u32);
            assert_eq!(state.history.owner_generated_cards_combat, generated as i32);
            assert_eq!(state.next_generated_hook_uid, generated as i32);
            assert!(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: Some(CardEnchantment {
                            id: EnchantmentId::Inky,
                            amount: 1,
                        }),
                    })
                    .is_some()
            );
        }
    }

    #[test]
    fn blade_of_ink_direct_body_refuses_remote_state_before_any_mutation() {
        let mut builder = CatalogBuilder::new();
        let blade = builder.intern(identity(CardId::BladeOfInk, 0)).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: blade,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.inky_attack_damage = 0;
        state.multiplayer_ally_key = 1;
        state.next_card_uid = 10;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = state.clone();
        let spec = *catalog.spec(blade).unwrap();
        let [program] = catalog.steps(&spec) else {
            panic!("Blade of Ink must have one exact step")
        };
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(program.args),
            events: &mut events,
        };

        assert_eq!(
            generate_shivs_then_inky_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("Blade of Ink remote state"))
        );
        assert_eq!(state, before, "the direct body is pre-mutation total");
        assert!(events.is_empty());
        assert_eq!(state.next_card_uid, 10);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_generated_hook_uid, 0);
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
    }

    #[test]
    fn blade_of_ink_burst_and_late_uid_failure_are_whole_action_exact() {
        let mut builder = CatalogBuilder::new();
        let blade = builder.intern(identity(CardId::BladeOfInk, 0)).unwrap();
        let catalog = builder.build();
        let source_state = || {
            let mut state = HotState::at_defaults();
            // A live enemy: with none, combat is ending and plural Shiv
            // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.hp = 50;
            state.energy = 3;
            state.inky_attack_damage = 0;
            state.next_card_uid = 10;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: blade,
                flags: 0,
            });
            state
        };

        let mut burst = source_state();
        burst.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut burst);
        play_card(&mut burst, &catalog, 1, None, None, &mut Vec::new()).unwrap();
        assert_eq!(burst.piles.get(PileId::Hand).len(), 4);
        assert_eq!(burst.next_card_uid, 14);
        assert_eq!(burst.history.owner_generated_cards_combat, 4);

        let mut overflow = source_state();
        overflow.next_card_uid = u32::MAX - 1;
        let before = overflow.clone();
        let mut events = Vec::new();
        assert!(matches!(
            play_card(&mut overflow, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        ));
        assert_eq!(overflow, before);
        assert!(events.is_empty());
    }

    #[test]
    fn inky_shiv_uses_frozen_target_or_fan_fresh_roster_after_damage() {
        let inky = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Inky,
                amount: 1,
            }),
        };
        for fan in [false, true] {
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(inky).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 3;
            state.inky_attack_damage = 0;
            state.next_card_uid = 2;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
            state.monsters_mut().extend([
                HotMonster::new(MonsterKind::Toadpole, 11),
                HotMonster::new(MonsterKind::Toadpole, 12),
            ]);
            for monster in state.monsters_mut() {
                monster.hp = 20;
                monster.max_hp = 20;
            }
            state.monsters_mut()[0].slot = 0;
            state.monsters_mut()[1].slot = 1;
            if fan {
                state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
            }
            let targets_before = state.rng.get(crate::hot::RngStream::Targets);

            play_card(
                &mut state,
                &catalog,
                1,
                (!fan).then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(
                state.rng.get(crate::hot::RngStream::Targets),
                targets_before,
                "neither selected-target nor Fan Shiv samples target RNG"
            );
            assert_eq!(state.monsters[0].hp, 16);
            assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 1);
            if fan {
                assert_eq!(state.monsters[1].hp, 16);
                assert_eq!(state.monsters[1].powers.value(PowerId::Weak), 1);
            } else {
                assert_eq!(state.monsters[1].hp, 20);
                assert_eq!(state.monsters[1].powers.value(PowerId::Weak), 0);
            }
        }
    }

    #[test]
    fn inky_shiv_l1_lamp_artifact_lethal_and_attack_replay_are_exact() {
        let build = |upgrade| {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Shiv,
                    upgrade,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Inky,
                        amount: 1,
                    }),
                })
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.intern_monster(MonsterKind::Axebot).unwrap();
            (builder.build(), atom)
        };

        let (catalog, atom) = build(1);
        let mut lamp = HotState::at_defaults();
        lamp.hp = 50;
        lamp.energy = 3;
        lamp.inky_attack_damage = 0;
        lamp.next_card_uid = 2;
        lamp.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: 0,
        });
        for (slot, uid) in [(0, 11), (1, 12)] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.slot = slot;
            monster.uid = uid;
            lamp.monsters_mut().push(monster);
        }
        lamp.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        lamp.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        lamp.fanouts.set_unsettling_lamp_available(true);
        let mut lamp_events = Vec::new();
        play_card(&mut lamp, &catalog, 1, None, None, &mut lamp_events).unwrap();
        assert_eq!(
            lamp.monsters
                .iter()
                .map(|monster| monster.hp)
                .collect::<Vec<_>>(),
            [24, 24],
            "Inky leaves the L1 Shiv base damage at six"
        );
        assert_eq!(lamp.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(lamp.monsters[0].powers.value(PowerId::Weak), 0);
        assert_eq!(lamp.monsters[1].powers.value(PowerId::Weak), 2);
        assert!(!lamp.fanouts.unsettling_lamp_available());
        let artifact_event = lamp_events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Event::PowerChanged {
                        subject: crate::engine::Subject::Monster(11),
                        power: PowerId::Artifact,
                        amount: 0,
                    }
                )
            })
            .unwrap();
        let weak_event = lamp_events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Event::PowerChanged {
                        subject: crate::engine::Subject::Monster(12),
                        power: PowerId::Weak,
                        amount: 2,
                    }
                )
            })
            .unwrap();
        assert!(artifact_event < weak_event);

        let (catalog, atom) = build(0);
        let base = || {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 3;
            state.inky_attack_damage = 0;
            state.next_card_uid = 2;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
            state
        };
        let mut lethal = base();
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 4));
        let mut lethal_events = Vec::new();
        play_card(&mut lethal, &catalog, 1, Some(0), None, &mut lethal_events).unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.monsters[0].powers.value(PowerId::Weak), 0);
        assert!(!lethal_events.iter().any(|event| {
            matches!(
                event,
                Event::PowerChanged {
                    power: PowerId::Weak,
                    ..
                }
            )
        }));

        let mut dead_target = base();
        dead_target
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 4));
        let mut survivor = HotMonster::new(MonsterKind::Toadpole, 30);
        survivor.slot = 1;
        survivor.uid = 1;
        dead_target.monsters_mut().push(survivor);
        play_card(
            &mut dead_target,
            &catalog,
            1,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!dead_target.history.over);
        assert_eq!(dead_target.monsters[0].hp, 0);
        assert_eq!(dead_target.monsters[0].powers.value(PowerId::Weak), 0);
        assert_eq!(dead_target.monsters[1].powers.value(PowerId::Weak), 0);

        let mut replaced_target = base();
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 4);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        replaced_target.monsters_mut().push(axebot);
        play_card(
            &mut replaced_target,
            &catalog,
            1,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(replaced_target.monsters[0].uid, 1);
        assert_eq!(replaced_target.monsters[0].powers.value(PowerId::Weak), 0);

        let mut replay = base();
        replay
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        replay.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut replay);
        play_card(&mut replay, &catalog, 1, Some(0), None, &mut Vec::new()).unwrap();
        assert_eq!(replay.monsters[0].hp, 22);
        assert_eq!(replay.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(replay.history.card_plays_finished_combat, 2);
        assert_eq!(replay.powers.value(PowerId::OneTwoPunch), 0);
    }

    fn hunt_fixture(
        upgrade: u8,
        monsters: Vec<HotMonster>,
        source_pile: PileId,
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::TheHunt, upgrade)).unwrap();
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
        state.energy = 20;
        state.next_card_uid = 8;
        state.reward_card_pool = Some(RewardPool::Silent);
        state.reward_card_rarity_odds = Some(RewardOdds::RegularEncounter);
        state.piles.get_mut(source_pile).make_mut().push(source);
        *state.monsters_mut() = monsters;
        (state, catalog, source)
    }

    fn run_hunt_body(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
        target: usize,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog.spec(source.atom).unwrap();
        let [program] = catalog.steps(spec) else {
            panic!("The Hunt row must have one compiled step")
        };
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid: source.uid,
            target: Some(target),
            selection: None,
            x_value: 0,
            args: catalog.args(program.args),
            events,
        };
        the_hunt_exact(&mut ctx)
    }

    #[test]
    fn family_manifest_and_all_twenty_two_generated_carriers_are_exact() {
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::BulletTimeExact,
                StepKind::FanOfKnivesExact,
                StepKind::GenerateShivsThenInkyExact,
                StepKind::KnifeTrapExact,
                StepKind::MalaiseX,
                StepKind::OutbreakExact,
                StepKind::ShadowStepDiscardExact,
                StepKind::ShadowStepPowerExact,
                StepKind::StormOfSteelExact,
                StepKind::TheHuntExact,
            ]
        );
        for kind in OWNED {
            assert_eq!(
                crate::steps::is_implemented(kind),
                IMPLEMENTED.contains(&kind),
                "{} is claimed outside its owning family",
                kind.as_str()
            );
        }
        assert!(ESCALATED.iter().all(|kind| !IMPLEMENTED.contains(kind)));

        let carriers = CARD_ROWS
            .iter()
            .flat_map(|row| {
                row.steps.iter().filter_map(move |step| {
                    OWNED.contains(&step.kind).then_some((
                        row.id,
                        row.upgrade,
                        step.kind,
                        step.args.to_vec(),
                    ))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::BladeOfInk,
                    0,
                    StepKind::GenerateShivsThenInkyExact,
                    vec![Arg::I(2)]
                ),
                (
                    CardId::BladeOfInk,
                    1,
                    StepKind::GenerateShivsThenInkyExact,
                    vec![Arg::I(3)]
                ),
                (CardId::BulletTime, 0, StepKind::BulletTimeExact, vec![]),
                (CardId::BulletTime, 1, StepKind::BulletTimeExact, vec![]),
                (
                    CardId::FanOfKnives,
                    0,
                    StepKind::FanOfKnivesExact,
                    vec![Arg::I(1)]
                ),
                (
                    CardId::FanOfKnives,
                    1,
                    StepKind::FanOfKnivesExact,
                    vec![Arg::I(1)]
                ),
                (
                    CardId::Flanking,
                    0,
                    StepKind::FlankingExact,
                    vec![Arg::I(2)]
                ),
                (
                    CardId::Flanking,
                    1,
                    StepKind::FlankingExact,
                    vec![Arg::I(2)]
                ),
                (CardId::KnifeTrap, 0, StepKind::KnifeTrapExact, vec![]),
                (CardId::KnifeTrap, 1, StepKind::KnifeTrapExact, vec![]),
                (CardId::Malaise, 0, StepKind::MalaiseX, vec![Arg::I(0)]),
                (CardId::Malaise, 1, StepKind::MalaiseX, vec![Arg::I(1)]),
                (
                    CardId::Outbreak,
                    0,
                    StepKind::OutbreakExact,
                    vec![Arg::I(9)]
                ),
                (
                    CardId::Outbreak,
                    1,
                    StepKind::OutbreakExact,
                    vec![Arg::I(12)]
                ),
                (
                    CardId::ShadowStep,
                    0,
                    StepKind::ShadowStepDiscardExact,
                    vec![]
                ),
                (
                    CardId::ShadowStep,
                    0,
                    StepKind::ShadowStepPowerExact,
                    vec![Arg::I(1)]
                ),
                (
                    CardId::ShadowStep,
                    1,
                    StepKind::ShadowStepDiscardExact,
                    vec![]
                ),
                (
                    CardId::ShadowStep,
                    1,
                    StepKind::ShadowStepPowerExact,
                    vec![Arg::I(1)]
                ),
                (
                    CardId::StormOfSteel,
                    0,
                    StepKind::StormOfSteelExact,
                    vec![Arg::I(0)]
                ),
                (
                    CardId::StormOfSteel,
                    1,
                    StepKind::StormOfSteelExact,
                    vec![Arg::I(1)]
                ),
                (CardId::TheHunt, 0, StepKind::TheHuntExact, vec![Arg::I(10)]),
                (CardId::TheHunt, 1, StepKind::TheHuntExact, vec![Arg::I(15)]),
            ]
        );

        let admitted = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| OWNED.contains(&step.kind)))
            .filter(|row| {
                row.steps
                    .iter()
                    .all(|step| crate::steps::is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![
                (CardId::BladeOfInk, 0),
                (CardId::BladeOfInk, 1),
                (CardId::BulletTime, 0),
                (CardId::BulletTime, 1),
                (CardId::FanOfKnives, 0),
                (CardId::FanOfKnives, 1),
                (CardId::KnifeTrap, 0),
                (CardId::KnifeTrap, 1),
                (CardId::Malaise, 0),
                (CardId::Malaise, 1),
                (CardId::Outbreak, 0),
                (CardId::Outbreak, 1),
                (CardId::ShadowStep, 0),
                (CardId::ShadowStep, 1),
                (CardId::StormOfSteel, 0),
                (CardId::StormOfSteel, 1),
                (CardId::TheHunt, 0),
                (CardId::TheHunt, 1),
            ]
        );
    }

    #[test]
    fn bullet_time_star_only_and_storm_catalog_closure_are_mechanical() {
        let infection = CARD_ROWS
            .iter()
            .find(|row| matches!((row.id, row.upgrade), (CardId::Infection, 0)))
            .unwrap();
        assert_eq!(infection.cost, -1);
        assert!(!infection.x_cost);
        assert!(!infection.playable);

        let mut document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        let card = &mut document.piles.get_mut("hand").unwrap()[0];
        let uid = card.uid.unwrap() as u32;
        card.id = CardId::Infection.as_str().to_owned();
        card.upgrade = 0;
        card.physical_state = Some(json!([
            "PHYSICAL_CARD_STATE",
            [],
            0,
            false,
            [[0, true, true]]
        ]));
        document
            .player
            .insert("exact_piles".to_owned(), json!(true));
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert_eq!(
            state
                .card_states
                .get(uid)
                .free_star_cost_this_turn_or_played_rows,
            1
        );

        let card = &mut document.piles.get_mut("hand").unwrap()[0];
        card.id = CardId::StrikeIronclad.as_str().to_owned();
        card.physical_state = Some(json!([
            "PHYSICAL_CARD_STATE",
            [[1, 0, 6, false], [1, 0, 6, false]],
            0,
            false,
            [[0, true, true], [0, true, true]]
        ]));
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert_eq!(
            state
                .card_states
                .get(uid)
                .free_star_cost_this_turn_or_played_rows,
            2
        );
        assert_eq!(HotBoundary::to_canonical(&state, &catalog), document);
        assert!(crate::steps::is_implemented(StepKind::BulletTimeExact));

        // Bullet Time skips Energy-X, but other exact mechanics call
        // SetToFreeThisTurn directly. Infernal Blade can generate Whirlwind,
        // so this independently reachable Energy/Star pair must remain
        // admissible on an X-cost instance.
        assert!(INFERNAL_BLADE_ATTACK_POOL_V109.contains(&CardId::Whirlwind));
        let whirlwind = CARD_ROWS
            .iter()
            .find(|row| matches!((row.id, row.upgrade), (CardId::Whirlwind, 0)))
            .unwrap();
        assert!(whirlwind.x_cost);
        let card = &mut document.piles.get_mut("hand").unwrap()[0];
        card.id = CardId::Whirlwind.as_str().to_owned();
        card.physical_state = Some(json!([
            "PHYSICAL_CARD_STATE",
            [[1, 0, 6, false]],
            0,
            false,
            [[0, true, true]]
        ]));
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert_eq!(
            state
                .card_states
                .get(uid)
                .free_star_cost_this_turn_or_played_rows,
            1
        );

        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StormOfSteel, 0)).unwrap();
        let storm_catalog = builder.build();
        assert!(storm_catalog.atom(&identity(CardId::Shiv, 0)).is_some());
        assert!(storm_catalog.atom(&identity(CardId::Shiv, 1)).is_none());

        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StormOfSteel, 1)).unwrap();
        let upgraded_storm_catalog = builder.build();
        assert!(
            upgraded_storm_catalog
                .atom(&identity(CardId::Shiv, 0))
                .is_some()
        );
        assert!(
            upgraded_storm_catalog
                .atom(&identity(CardId::Shiv, 1))
                .is_some()
        );
        assert!(crate::steps::is_implemented(StepKind::StormOfSteelExact));
    }

    fn bullet_state() -> (HotState, crate::catalog::Catalog) {
        let mut builder = CatalogBuilder::new();
        let bullet = builder.intern(identity(CardId::BulletTime, 0)).unwrap();
        let strike = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        let infection = builder.intern(identity(CardId::Infection, 0)).unwrap();
        let whirlwind = builder.intern(identity(CardId::Whirlwind, 0)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 1,
            atom: bullet,
            flags: 0,
        });
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 2,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 3,
                atom: infection,
                flags: 0,
            },
            HotCard {
                uid: 4,
                atom: whirlwind,
                flags: 0,
            },
        ]);
        state.card_states.append_local_cost_modifier(
            2,
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 1,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: true,
            },
        );
        (state, catalog)
    }

    #[test]
    fn bullet_time_rereads_hand_preserves_append_order_and_skips_energy_x() {
        let (mut state, catalog) = bullet_state();
        let x_spec = catalog
            .spec(state.piles.get(PileId::Hand).as_slice()[2].atom)
            .unwrap();
        assert!(x_spec.x_cost);
        state
            .card_states
            .set_to_free_this_turn(4, x_spec.cost)
            .unwrap();
        state.piles.get_mut(PileId::Hand).make_mut()[2].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let x_before = state.card_states.get(4);
        let spec = *catalog
            .spec(state.piles.get(PileId::Play).as_slice()[0].atom)
            .unwrap();
        let mut events = Vec::new();
        for _ in 0..2 {
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 1,
                target: None,
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut events,
            };
            bullet_time_exact(&mut ctx).unwrap();
        }

        let strike = state.card_states.get(2);
        assert_eq!(strike.free_star_cost_this_turn_or_played_rows, 2);
        assert_eq!(strike.local_cost_modifiers.as_slice().len(), 3);
        assert_eq!(strike.local_cost_modifiers.as_slice()[0].amount, 1);
        assert!(strike.local_cost_modifiers.as_slice()[0].reduce_only);
        assert!(
            strike.local_cost_modifiers.as_slice()[1..]
                .iter()
                .all(|row| {
                    row.kind == LocalCostModifierKind::Set
                        && row.amount == 0
                        && row.expiration == LocalCostExpiration::ThisTurnOrPlayed
                        && !row.reduce_only
                })
        );

        let infection = state.card_states.get(3);
        assert_eq!(infection.free_star_cost_this_turn_or_played_rows, 2);
        assert!(infection.local_cost_modifiers.is_empty());
        let x = state.card_states.get(4);
        assert_eq!(x, x_before, "Bullet Time must not append to Energy-X");
        assert_eq!(x.free_star_cost_this_turn_or_played_rows, 1);
        assert_ne!(
            state.piles.get(PileId::Hand).as_slice()[2].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            0
        );
        assert_eq!(state.powers.value(PowerId::NoDraw), 2);
        assert!(state.exact_piles);
    }

    #[test]
    fn bullet_time_burst_replays_the_live_hand_and_public_cleanup_remains_exact() {
        let (mut state, catalog) = bullet_state();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .insert(0, source);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        play_card(&mut state, &catalog, 1, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.powers.value(PowerId::NoDraw), 2);
        assert_eq!(
            state
                .card_states
                .get(2)
                .free_star_cost_this_turn_or_played_rows,
            2
        );
        assert_eq!(
            state
                .card_states
                .get(3)
                .free_star_cost_this_turn_or_played_rows,
            2
        );
        assert_eq!(
            state.card_states.get(4),
            crate::hot::CardInstanceState::default()
        );
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );
    }

    #[test]
    fn bullet_time_late_row_overflow_refuses_the_whole_body() {
        let (mut state, catalog) = bullet_state();
        let mut payload = state.card_states.get(2);
        payload.free_star_cost_this_turn_or_played_rows = u8::MAX;
        state.card_states.set(2, payload);
        let spec = *catalog
            .spec(state.piles.get(PileId::Play).as_slice()[0].atom)
            .unwrap();
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };

        assert_eq!(
            bullet_time_exact(&mut ctx),
            Err(EngineRefusal::CounterOverflow(
                "Bullet Time temporary Star rows"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn bullet_time_public_manual_no_draw_overflow_rolls_back_the_whole_action() {
        let (mut state, catalog) = bullet_state();
        let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .insert(0, source);
        state.powers.set(PowerId::NoDraw, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let before_events = events.clone();

        assert_eq!(
            play_card(&mut state, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("Bullet Time NoDraw"))
        );
        assert_eq!(state, before);
        assert_eq!(state.energy, before.energy);
        assert_eq!(state.history, before.history);
        assert_eq!(
            state.piles.get(PileId::Hand),
            before.piles.get(PileId::Hand)
        );
        assert_eq!(state.powers.value(PowerId::NoDraw), i32::MAX);
        assert_eq!(events, before_events);
    }

    #[test]
    fn bullet_time_public_later_burst_row_overflow_rolls_back_the_whole_action() {
        let (mut state, catalog) = bullet_state();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .insert(0, source);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let row = LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount: 0,
            expiration: LocalCostExpiration::ThisTurnOrPlayed,
            reduce_only: false,
        };
        let mut payload = state.card_states.get(2);
        payload.local_cost_modifiers = LocalCostModifiers::from_rows(vec![row; 254]);
        payload.free_star_cost_this_turn_or_played_rows = 254;
        state.card_states.set(2, payload);
        state.piles.get_mut(PileId::Hand).make_mut()[1].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let before_events = events.clone();

        assert_eq!(
            play_card(&mut state, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Bullet Time temporary Star rows"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(state.energy, before.energy);
        assert_eq!(state.history, before.history);
        assert_eq!(
            state.piles.get(PileId::Hand),
            before.piles.get(PileId::Hand)
        );
        assert_eq!(state.powers.value(PowerId::NoDraw), 0);
        assert_eq!(events, before_events);
    }

    fn storm_state(upgrade: u8, fillers: usize) -> (HotState, crate::catalog::Catalog) {
        let storm = identity(CardId::StormOfSteel, upgrade);
        let defend = identity(CardId::DefendSilent, 0);
        let mut builder = CatalogBuilder::new();
        let storm_atom = builder.intern(storm).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.next_card_uid = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: storm_atom,
            flags: 0,
        });
        for offset in 0..fillers {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10 + u32::try_from(offset).unwrap(),
                atom: defend_atom,
                flags: 0,
            });
        }
        (state, catalog)
    }

    fn shadow_state(upgrade: u8, fillers: usize) -> (HotState, crate::catalog::Catalog) {
        let mut builder = CatalogBuilder::new();
        let shadow = builder
            .intern(identity(CardId::ShadowStep, upgrade))
            .unwrap();
        let defend = builder.intern(identity(CardId::DefendSilent, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: shadow,
            flags: 0,
        });
        for offset in 0..fillers {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10 + u32::try_from(offset).unwrap(),
                atom: defend,
                flags: 0,
            });
        }
        (state, catalog)
    }

    #[test]
    fn shadow_step_public_play_discards_ordered_hand_then_applies_wrapper() {
        for upgrade in [0, 1] {
            let (mut state, catalog) = shadow_state(upgrade, 3);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();

            assert!(state.piles.get(PileId::Hand).is_empty());
            assert_eq!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [10, 11, 12, 1]
            );
            assert_eq!(state.history.discarded_cards_this_turn, 3);
            assert_eq!(state.powers.value(PowerId::ShadowStep), 1);
            assert_eq!(state.powers.value(PowerId::DoubleDamage), 0);
            let power_event = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        Event::PowerChanged {
                            power: PowerId::ShadowStep,
                            ..
                        }
                    )
                })
                .unwrap();
            let last_discard = events
                .iter()
                .rposition(|event| {
                    matches!(
                        event,
                        Event::CardResolved {
                            uid: 10..=12,
                            pile: PileId::Discard,
                        }
                    )
                })
                .unwrap();
            assert!(power_event > last_discard);
        }
    }

    #[test]
    fn shadow_step_burst_replays_fresh_empty_hand_and_stacks_wrapper() {
        let (mut state, catalog) = shadow_state(0, 2);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        play_card(&mut state, &catalog, 1, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.powers.value(PowerId::ShadowStep), 2);
        assert_eq!(state.history.discarded_cards_this_turn, 2);
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [10, 11, 1]
        );
    }

    #[test]
    fn shadow_step_sly_tail_runs_in_frozen_order_before_power_suffix() {
        let (mut state, catalog) = shadow_state(0, 2);
        state.card_states.set_local_sly(10);
        state.card_states.set_local_sly(11);
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();

        assert_eq!(state.block, 10);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardPlayed { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [1, 10, 11]
        );
        let power_event = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Event::PowerChanged {
                        power: PowerId::ShadowStep,
                        ..
                    }
                )
            })
            .unwrap();
        let sly_finish = events
            .iter()
            .rposition(|event| matches!(event, Event::CardPlayed { uid: 10 | 11, .. }))
            .unwrap();
        assert!(power_event > sly_finish);
    }

    #[test]
    fn shadow_step_selecting_sly_and_late_power_overflow_are_atomic() {
        let mut builder = CatalogBuilder::new();
        let shadow = builder.intern(identity(CardId::ShadowStep, 0)).unwrap();
        let selecting = builder
            .intern(identity(CardId::SecretTechnique, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: shadow,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: selecting,
                flags: 0,
            },
        ]);
        state.card_states.set_local_sly(2);
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 77 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(&mut state, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        let (mut overflow, catalog) = shadow_state(0, 1);
        overflow
            .powers
            .set(PowerId::ShadowStep, SlotWire::Int, i32::MAX);
        let before = overflow.clone();
        let mut events = vec![Event::TurnBegan { turn: 78 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(&mut overflow, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("Shadow Step power amount"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn shadow_step_current_plural_discard_finishes_before_terminal_sly_tail() {
        let (mut state, catalog) = shadow_state(0, 2);
        state.card_states.set_local_sly(10);
        state.card_states.set_local_sly(11);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state.monsters_mut()[0].hp = 1;

        play_card(&mut state, &catalog, 1, None, None, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.powers.value(PowerId::ShadowStep), 0);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [11],
            "both frozen cards discard before the lethal Sly child enters Play"
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1, 10]
        );
    }

    #[test]
    fn storm_public_play_discards_the_frozen_hand_then_bulk_creates_the_exact_level() {
        for upgrade in [0, 1] {
            let (mut state, catalog) = storm_state(upgrade, 3);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();

            assert_eq!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [10, 11, 12, 1]
            );
            let shivs = state.piles.get(PileId::Hand).as_slice();
            assert_eq!(
                shivs.iter().map(|card| card.uid).collect::<Vec<_>>(),
                [100, 101, 102]
            );
            assert!(shivs.iter().all(|card| {
                catalog.spec(card.atom).unwrap().identity == identity(CardId::Shiv, upgrade)
            }));
            assert_eq!(state.next_card_uid, 103);
            assert_eq!(state.history.discarded_cards_this_turn, 3);
            assert_eq!(state.history.owner_generated_cards_combat, 3);
            assert_eq!(state.next_generated_hook_uid, 3);
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        Event::CardResolved {
                            uid,
                            pile: PileId::Discard,
                        } if matches!(*uid, 10..=12) => {
                            Some(*uid)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                [10, 11, 12]
            );
        }
    }

    #[test]
    fn storm_refuses_burst_before_manual_or_autoplay_mutation() {
        let (mut manual, catalog) = storm_state(0, 2);
        manual.powers.set(PowerId::Burst, SlotWire::Int, 1);
        let before = manual.clone();
        let mut events = Vec::new();
        assert_eq!(
            play_card(&mut manual, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(manual, before);
        assert!(events.is_empty());

        let (mut autoplay, catalog) = storm_state(0, 0);
        let storm = autoplay.piles.get_mut(PileId::Hand).make_mut().remove(0);
        autoplay.piles.get_mut(PileId::Draw).make_mut().push(storm);
        autoplay.powers.set(PowerId::Burst, SlotWire::Int, 1);
        let before = autoplay.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::play::autoplay_draw_top(&mut autoplay, &catalog, 1, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        let mut gathered = before;
        let source = gathered.piles.get_mut(PileId::Draw).make_mut().remove(0);
        gathered.piles.get_mut(PileId::Play).make_mut().push(source);
        assert_eq!(
            autoplay, gathered,
            "the outer AutoPlay gather may move the source, but the refused body must publish nothing"
        );
        assert!(events.is_empty());
    }

    #[test]
    fn storm_sly_tail_preserves_frozen_order_before_bulk_generation() {
        let (mut state, catalog) = storm_state(0, 2);
        state.card_states.set_local_sly(10);
        state.card_states.set_local_sly(11);
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();

        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardPlayed { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [1, 10, 11],
            "the original Hand order governs the complete Sly tail"
        );
        assert_eq!(state.block, 10);
        assert_eq!(state.next_card_uid, 102);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [100, 101],
            "Sly routing does not change the frozen generation count"
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [10, 11, 1],
            "each exact Sly UID routes once before the source resolves"
        );
    }

    #[test]
    fn a_terminal_sly_child_suppresses_storm_shiv_creation() {
        let (mut state, catalog) = storm_state(1, 2);
        state.card_states.set_local_sly(10);
        state.card_states.set_local_sly(11);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state.monsters_mut()[0].hp = 1;
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 1, None, None, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.next_card_uid, 100);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [11],
            "all frozen cards discard first; the lethal Sly child enters Play"
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1, 10]
        );
    }

    #[test]
    fn storm_frozen_discard_refuses_duplicate_or_missing_live_uid_without_mutation() {
        let (mut state, catalog) = storm_state(0, 1);
        let frozen = state.piles.get(PileId::Hand).as_slice()[1];
        state.piles.get_mut(PileId::Draw).make_mut().push(frozen);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            crate::engine::draw::discard_frozen_hand(&mut state, &catalog, &[frozen], &mut events,),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let (mut missing, catalog) = storm_state(0, 1);
        let frozen = missing.piles.get_mut(PileId::Hand).make_mut().remove(1);
        let before = missing.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::draw::discard_frozen_hand(
                &mut missing,
                &catalog,
                &[frozen],
                &mut events,
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(missing, before);
        assert!(events.is_empty());
    }

    #[test]
    fn storm_fixed_cost_body_refuses_a_forged_x_value_without_mutation() {
        let (mut state, catalog) = storm_state(0, 1);
        let spec = *catalog
            .spec(state.piles.get(PileId::Hand).as_slice()[0].atom)
            .unwrap();
        let before = state.clone();
        let args = [CompiledArg::I(0)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: None,
            selection: None,
            x_value: 1,
            args: &args,
            events: &mut events,
        };

        assert_eq!(
            storm_of_steel_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("storm_of_steel_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn the_hunt_both_levels_nonfatal_and_final_lethal_preserve_native_order() {
        for (upgrade, damage) in [(0, 10), (1, 15)] {
            let (mut nonfatal, catalog, source) = hunt_fixture(
                upgrade,
                vec![HotMonster::new(MonsterKind::Toadpole, damage + 1)],
                PileId::Draw,
            );
            let rng_before = nonfatal.rng.clone();
            run_hunt_body(&mut nonfatal, &catalog, source, 0, &mut Vec::new()).unwrap();
            assert_eq!(nonfatal.fanouts.the_hunt_reward_count(), 0);
            assert_eq!(nonfatal.fanouts.the_hunt_marker(), 0);
            assert_eq!(nonfatal.rng, rng_before);

            let (mut lethal, catalog, source) = hunt_fixture(
                upgrade,
                vec![HotMonster::new(MonsterKind::Toadpole, damage)],
                PileId::Play,
            );
            let rng_before = lethal.rng.clone();
            let mut events = Vec::new();
            run_hunt_body(&mut lethal, &catalog, source, 0, &mut events).unwrap();
            assert!(lethal.history.over);
            assert_eq!(lethal.fanouts.the_hunt_reward_count(), 1);
            assert_eq!(lethal.fanouts.the_hunt_marker(), 0);
            assert_eq!(lethal.rng, rng_before, "reward identities remain deferred");
            assert_eq!(events.last(), Some(&Event::CombatOver { player_won: true }));
        }
    }

    #[test]
    fn the_hunt_stock_secondary_and_segment_fatal_snapshot_are_exact() {
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 1);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut stock, catalog, source) = hunt_fixture(0, vec![axebot], PileId::Draw);
        let niche_before = stock.rng.get(crate::hot::RngStream::Niche).counter;
        run_hunt_body(&mut stock, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert!(!stock.history.over);
        assert_eq!(stock.fanouts.the_hunt_reward_count(), 1);
        assert_eq!(stock.fanouts.the_hunt_marker(), 1);
        assert_eq!(stock.monsters[0].powers.value(PowerId::Stock), 1);
        assert_eq!(
            stock.rng.get(crate::hot::RngStream::Niche).counter,
            niche_before + 1
        );

        let mut secondary = HotMonster::new(MonsterKind::Toadpole, 1);
        secondary.powers.set(PowerId::Secondary, SlotWire::Int, 1);
        let (mut state, catalog, source) = hunt_fixture(0, vec![secondary], PileId::Draw);
        run_hunt_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.fanouts.the_hunt_reward_count(), 0);

        let target = HotMonster::new(MonsterKind::DecimillipedeSegment, 1);
        let alive_other = HotMonster::new(MonsterKind::DecimillipedeSegment, 20);
        let (mut state, catalog, source) =
            hunt_fixture(0, vec![target.clone(), alive_other], PileId::Draw);
        run_hunt_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.fanouts.the_hunt_reward_count(), 0);

        let mut dead_other = HotMonster::new(MonsterKind::DecimillipedeSegment, 20);
        dead_other.hp = 0;
        let (mut state, catalog, source) = hunt_fixture(0, vec![target, dead_other], PileId::Draw);
        run_hunt_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.fanouts.the_hunt_reward_count(), 1);
        assert_eq!(state.fanouts.the_hunt_marker(), 0);
    }

    #[test]
    fn the_hunt_authenticates_one_physical_source_across_every_pile() {
        for pile in PileId::ALL {
            let (mut state, catalog, source) =
                hunt_fixture(0, vec![HotMonster::new(MonsterKind::Toadpole, 20)], pile);
            run_hunt_body(&mut state, &catalog, source, 0, &mut Vec::new()).unwrap();
            assert_eq!(state.monsters[0].hp, 10, "direct source in {pile:?}");
        }

        let (mut duplicate, catalog, source) = hunt_fixture(
            0,
            vec![HotMonster::new(MonsterKind::Toadpole, 20)],
            PileId::Draw,
        );
        duplicate
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        assert_eq!(
            run_hunt_body(&mut duplicate, &catalog, source, 0, &mut Vec::new()),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(duplicate, before);
    }

    #[test]
    fn the_hunt_authenticates_the_inert_type_one_listener_walk() {
        let monsters = vec![
            HotMonster::new(MonsterKind::Toadpole, 1),
            HotMonster::new(MonsterKind::Toadpole, 50),
        ];
        for order in [
            [PowerId::Shroud, PowerId::SleightOfFlesh],
            [PowerId::SleightOfFlesh, PowerId::Shroud],
        ] {
            let (mut valid, catalog, source) = hunt_fixture(0, monsters.clone(), PileId::Draw);
            valid.powers.set(PowerId::Shroud, SlotWire::Int, 2);
            valid.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(valid.fanouts.set_after_power_amount_changed_order(&order));
            run_hunt_body(&mut valid, &catalog, source, 0, &mut Vec::new()).unwrap();
            assert_eq!(valid.fanouts.the_hunt_reward_count(), 1);
            assert_eq!(valid.fanouts.the_hunt_marker(), 1);
            assert_eq!(valid.block, 0, "Shroud ignores non-Doom Type-1 changes");
            assert_eq!(
                valid.monsters[1].hp, 50,
                "Sleight ignores player-owned Type-1 changes"
            );
        }

        let (mut duplicate, catalog, source) = hunt_fixture(0, monsters, PileId::Draw);
        duplicate.powers.set(PowerId::Shroud, SlotWire::Int, 2);
        duplicate
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            duplicate
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud, PowerId::Shroud,])
        );
        let before = duplicate.clone();
        let mut events = vec![Event::TurnBegan { turn: 84 }];
        let before_events = events.clone();
        assert_eq!(
            run_hunt_body(&mut duplicate, &catalog, source, 0, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "The Hunt after-power-amount-changed listener order"
            ))
        );
        assert_eq!(duplicate, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_hunt_repeated_rewards_round_trip_as_deferred_descriptors() {
        let (mut repeated, repeated_catalog, repeated_source) = hunt_fixture(
            0,
            vec![
                HotMonster::new(MonsterKind::Toadpole, 1),
                HotMonster::new(MonsterKind::Toadpole, 1),
            ],
            PileId::Draw,
        );
        run_hunt_body(
            &mut repeated,
            &repeated_catalog,
            repeated_source,
            0,
            &mut Vec::new(),
        )
        .unwrap();
        run_hunt_body(
            &mut repeated,
            &repeated_catalog,
            repeated_source,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(repeated.fanouts.the_hunt_reward_count(), 2);
        assert_eq!(repeated.fanouts.the_hunt_marker(), 1);

        let mut document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        let card = &mut document.piles.get_mut("hand").unwrap()[0];
        card.id = CardId::TheHunt.as_str().to_owned();
        card.upgrade = 1;
        document
            .player
            .insert("reward_card_pool".to_owned(), json!("Silent"));
        document.player.insert(
            "reward_card_rarity_odds".to_owned(),
            json!("RegularEncounter"),
        );
        document.player.insert("the_hunt".to_owned(), json!(1));
        document.player.insert(
            "the_hunt_card_rewards".to_owned(),
            json!([
                [0, "Silent", "Encounter", "RegularEncounter", 3],
                [0, "Silent", "Encounter", "RegularEncounter", 3]
            ]),
        );
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(state.fanouts.the_hunt_reward_count(), 2);
        assert_eq!(state.fanouts.the_hunt_marker(), 1);
        assert_eq!(HotBoundary::to_canonical(&state, &catalog), document);
        assert_eq!(admit(&document, &state, &catalog), Ok(()));

        let mut missing_provenance = state.clone();
        missing_provenance.reward_card_pool = None;
        assert!(admit(&document, &missing_provenance, &catalog).is_err());
        assert!(HotBoundary::try_to_canonical(&missing_provenance, &catalog).is_err());

        let mut mismatched = document;
        mismatched.player.insert(
            "the_hunt_card_rewards".to_owned(),
            json!([[0, "Ironclad", "Encounter", "RegularEncounter", 3]]),
        );
        assert!(HotBoundary::from_canonical(&mismatched, &catalog).is_err());
    }

    #[test]
    fn the_hunt_replay_rewards_only_the_killing_body_and_terminal_suppresses_replay() {
        for (hp, expected_finished) in [(15, 2), (10, 1)] {
            let (mut state, catalog, source) = hunt_fixture(
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
            assert_eq!(state.fanouts.the_hunt_reward_count(), 1);
            assert_eq!(state.fanouts.the_hunt_marker(), 0);
            assert_eq!(
                state.history.card_plays_finished_combat, expected_finished,
                "only a nonterminal first body reaches its queued replay"
            );
        }
    }

    #[test]
    fn the_hunt_body_and_public_action_roll_back_late_failures() {
        let (mut body, catalog, source) = hunt_fixture(
            0,
            vec![HotMonster::new(MonsterKind::Toadpole, 1)],
            PileId::Draw,
        );
        assert!(body.fanouts.set_the_hunt_state(i32::MAX, 0));
        let before = body.clone();
        let mut events = vec![Event::TurnBegan { turn: 80 }];
        let before_events = events.clone();
        assert_eq!(
            run_hunt_body(&mut body, &catalog, source, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("The Hunt reward/marker"))
        );
        assert_eq!(body, before);
        assert_eq!(events, before_events);

        for upgrade in [0, 1] {
            let mut malformed_stock = HotMonster::new(MonsterKind::Axebot, 1);
            malformed_stock.max_hp = 75;
            malformed_stock.powers.set(PowerId::Stock, SlotWire::Int, 2);
            let (mut state, catalog, source) =
                hunt_fixture(upgrade, vec![malformed_stock], PileId::Hand);
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 81 }];
            let before_events = events.clone();
            assert_eq!(
                play_card(&mut state, &catalog, source.uid, Some(0), None, &mut events),
                Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        let (mut reward_overflow, catalog, source) = hunt_fixture(
            0,
            vec![HotMonster::new(MonsterKind::Toadpole, 1)],
            PileId::Hand,
        );
        assert!(reward_overflow.fanouts.set_the_hunt_state(i32::MAX, 0));
        let before = reward_overflow.clone();
        let mut events = vec![Event::TurnBegan { turn: 82 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(
                &mut reward_overflow,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("The Hunt reward/marker"))
        );
        assert_eq!(reward_overflow, before);
        assert_eq!(events, before_events);

        let mut axebot = HotMonster::new(MonsterKind::Axebot, 1);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut marker_overflow, catalog, source) = hunt_fixture(0, vec![axebot], PileId::Hand);
        assert!(marker_overflow.fanouts.set_the_hunt_state(0, i32::MAX));
        let before = marker_overflow.clone();
        let mut events = vec![Event::TurnBegan { turn: 83 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(
                &mut marker_overflow,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("The Hunt reward/marker"))
        );
        assert_eq!(marker_overflow, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn malaise_strength_and_weak_each_fire_sleight_of_flesh() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::Malaise,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog
            .spec(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::Malaise,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap(),
            )
            .unwrap();
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 3);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let args = [CompiledArg::I(0)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: Some(0),
            selection: None,
            x_value: 2,
            args: &args,
            events: &mut events,
        };

        malaise_x(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[0].hp, 44);
    }

    /// #3515: Fan of Knives, Malaise and Shadow Step each await
    /// `PowerCmd.Apply` (`<Apply>d__1`1` 0x3ef988 IL_0025, `IsEnding`) and used
    /// to gate on `history.over`. While the combat is ending before the over
    /// latch each is a no-op; the Adaptable-vetoed control writes its power
    /// (Malaise's Strength and Weak land on the live Gas Bomb).
    #[test]
    fn silent_rare_power_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        let mut builder = CatalogBuilder::new();
        let fan = builder.intern(identity(CardId::FanOfKnives, 0)).unwrap();
        builder.intern(identity(CardId::Shiv, 0)).unwrap();
        let malaise = builder.intern(identity(CardId::Malaise, 0)).unwrap();
        let shadow = builder.intern(identity(CardId::ShadowStep, 0)).unwrap();
        let catalog = builder.build();
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.energy = 9;
        template.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: fan,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: malaise,
                flags: 0,
            },
            HotCard {
                uid: 3,
                atom: shadow,
                flags: 0,
            },
        ]);
        type Body = fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>;
        type Row<'a> = (
            &'a str,
            crate::catalog::CardAtom,
            u32,
            Body,
            &'a [CompiledArg],
            bool,
        );
        let rows: [Row<'_>; 3] = [
            (
                "Fan of Knives",
                fan,
                1,
                fan_of_knives_exact,
                &[CompiledArg::I(1)],
                false,
            ),
            ("Malaise", malaise, 2, malaise_x, &[CompiledArg::I(0)], true),
            (
                "Shadow Step",
                shadow,
                3,
                shadow_step_power_exact,
                &[CompiledArg::I(1)],
                false,
            ),
        ];
        for (label, atom, uid, body, args, targeted) in rows {
            let spec = *catalog.spec(atom).unwrap();
            crate::engine::damage::assert_ending_window_gate(&template, label, |s, t| {
                body(&mut StepCtx {
                    state: s,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: uid,
                    target: targeted.then_some(t),
                    selection: None,
                    x_value: i64::from(targeted) * 2,
                    args,
                    events: &mut Vec::new(),
                })
            });
        }
    }
}
