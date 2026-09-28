//! Card-step bodies for the `content/cards/silent_uncommon.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status: 12 of 13 ported, 1 still escalated
//!
//! Batch 1 (#1342) escalated all thirteen. Engine slice 2 (#1367) freed four
//! of them: the step-callable Draw that returns the exact drawn identities is
//! the whole of what `escape_plan_exact` and `pillage_exact` were waiting on,
//! and the Poison power is the whole of what its two consumers,
//! `poison_if_poisoned` and `mirage_total_poison_block_exact`, were waiting on.
//!
//! Batch 1b re-derived the nine that were left against that engine, and found
//! one more that needs nothing new: **`expose_exact`**. Its batch-1 citation
//! named three blockers and slice 2 or a fresh reading of the gate retired all
//! three — the `exhausts` keyword is inert since `CardCmd.Exhaust` landed, and
//! both `AfterBlockBroken` consumers plus the Artifact branch need content the
//! gate refuses. The block write itself is a monster field, not a primitive.
//!
//! Slice 3's first seam adds the `CombatTargets` roll, so
//! `poison_random_serial` and both Bouncing Flask rows now join the core pool.
//! Player powers are no longer "three, monsters only" (#1364 split the
//! registry per side and `admit` now admits player Strength), but the other
//! player-power families below still need their consumers.
//!
//! Every remaining stub carries the Python branch it must transcribe and the
//! **named primitive** that has to exist before the transcription can be
//! exact. Nothing here is half-ported and nothing is guessed (I5).
//!
//! The primitives this family is still waiting on, deduplicated:
//!
//! | primitive | kinds waiting on it |
//! |---|---|
//! | additional ally-owned powers | `concoct_exact` |
//! | player powers beyond this slice | `concoct_exact` |
//!
//! Issue #1374 adds Power-card removal, Well Laid Plans' whole-hand flush
//! suppressor, and Up My Sleeve's ordered per-instance cost delta. Unit E
//! Batch 5 closes the exact local/transient Retain state and Expertise reader.
//!
//! Batch 2 (#1342) re-derived the then-six remaining escalations against the
//! earlier #1374 engine — the generated-card transaction
//! ([`crate::engine::cards::inject_generated_bottom`]), the selection Action
//! dimension, and the six-entry player-power registry. The local-cost slice
//! subsequently closes Up My Sleeve; the remaining primitives stay absent,
//! checked at their engine sites rather than inherited from the batch-1b
//! prose. Unit C #1562 closes Blade Symphony's exact ordered two-Player Shiv
//! fan-out. `admit` still refuses any entry carrying unsupported
//! continuations, unsupported ally bodies, Sly rows, and
//! unsupported card-state axes. R30 closes Fade through the existing typed
//! AnyAlly target plus a zero-growth remote temporary-Dexterity quotient.
//! Generated `select` admits only its closed
//! operation vocabulary; local/transient Retain is the sole per-instance
//! keyword represented by this batch.

use super::StepCtx;
use crate::catalog::CompiledArg;
use crate::engine::cards::inject_generated_shivs_then_upgrade;
use crate::engine::damage::{
    apply_card_monster_debuff, gain_powered_card_block, note_power, roll_target,
};
use crate::engine::draw::{
    CardDrawResult, DrawSource, MAX_CARDS_IN_HAND, draw_cards_for_card_result, draw_cards_into,
};
use crate::engine::{EngineRefusal, Subject, fire_hook};
use crate::hooks::HookEvent;
use crate::hot::{
    DrawCaller, DrawEntry, HotCard, HotState, LocalCostExpiration, LocalCostModifier,
    LocalCostModifierKind, MiseryToken, PileId,
};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// The four slice 2 freed (#1367), `expose_exact` from batch 1b,
/// `poison_random_serial` from slice 3, then Well Laid Plans and Up My Sleeve
/// from #1374, then Expertise from Unit E Batch 5 and Blade Symphony from
/// Unit C Batch B. Two remain escalated.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::BladeSymphonyExact,
    StepKind::EscapePlanExact,
    StepKind::ExpertiseExact,
    StepKind::ExposeExact,
    StepKind::FadeExact,
    StepKind::GenerateShivsThenUpgradeExact,
    StepKind::MirageTotalPoisonBlockExact,
    StepKind::PillageExact,
    StepKind::PoisonIfPoisoned,
    StepKind::PoisonRandomSerial,
    StepKind::UpMySleeveCostExact,
    StepKind::WellLaidPlans,
];

/// `blade_symphony_exact` — exact ordered Player Shiv fan-out (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Validates the whole
/// `BLADE_SYMPHONY` body/metadata (cost 2/1, `AllAllies`, pool Silent, the
/// live `CardPlayFrame`), then `_apply_blade_symphony_shivs(s, 2, log)`.
///
/// Each living Player receives two serial null-created Shiv transactions in
/// fixed owner/teammate order. The local path mints physical UIDs; the remote
/// path mutates the teammate pile/history quotient while sharing the global
/// generated-listener token stream. The whole four-card suffix is rehearsed
/// before any live pile, UID, history, listener, or event write.
pub(crate) fn blade_symphony_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::BladeSymphony
        || ctx.args != [CompiledArg::I(2)]
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
    {
        return Err(EngineRefusal::MalformedArgs("blade_symphony_exact"));
    }
    let mut probe = ctx.state.clone();
    crate::engine::allies::blade_symphony(&mut probe, ctx.catalog, &mut Vec::new())?;
    crate::engine::allies::blade_symphony(ctx.state, ctx.catalog, ctx.events)
}

/// `concoct_exact` — not modeled. **Escalated (#1342; re-derived in batch 1b
/// and again in batch 2 against the post-#1374 engine).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Validates the `CONCOCT` body
/// and its live `CardPlayFrame`, resolves the recipient with
/// `_resolve_ally_target_key` off the frame's `choice`/`source`, then
/// `_apply_concoct_to_player(s, player_key, step[1], log)`.
///
/// Missing primitives: **ally targeting** (`AnyAlly` plus the frame's
/// `choice`/`source` fields — `admit` refuses any entry carrying a
/// continuation frame and no engine path pushes one, so the frame the branch
/// insists on can never be there), and the **ConcoctPower** player power.
/// The player-power registry lists only powers with complete consumers;
/// Concoct is absent because its
/// behaviour — a post-hit Poison fan-out that expires at the opposing side's
/// end — has no reader here.
/// ESCALATED-ON: power-variant-absent(PowerId::Concoct)
pub(crate) fn concoct_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::ConcoctExact))
}

/// `("escape_plan_exact", block)` — Escape Plan.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `draw_cards(s, 1, ...)`, then
/// `_escape_plan_after_draw`: gain powered card block **only** when
/// the exact first returned CardModel is a Skill.
///
/// The reason the result is read from the Draw command rather than from the
/// hand is written into the branch's own comment: reentrant draw hooks may
/// have moved the drawn card before this point, so the hand tail is not its
/// identity. That is why [`draw_cards_into`] exists.
pub(crate) fn escape_plan_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(block)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("escape_plan_exact"));
    };
    let block = *block;
    // EscapePlan 0x39c314 reads the first returned Draw card's Skill type
    // for powered block. Under a CardPlay frame the Draw is an owned tail
    // (#2665): a Hellraiser selection or Stratagem reshuffle can suspend
    // it, and the Skill read resumes after the Draw returns.
    let drawn = if matches!(
        ctx.state.frames.top(),
        Some(crate::frame::Frame::CardPlay { .. })
    ) {
        match crate::engine::play::draw_cardplay_owned_result(ctx, 1)? {
            CardDrawResult::Complete(drawn) => drawn.into_iter().map(DrawEntry::card).collect(),
            CardDrawResult::Suspended => return Ok(()),
        }
    } else {
        let mut drawn = Vec::<HotCard>::new();
        draw_cards_into(
            ctx.state,
            ctx.catalog,
            1,
            DrawSource::Command,
            ctx.events,
            &mut drawn,
        )?;
        drawn
    };
    apply_escape_plan_after_draw(ctx.state, ctx.catalog, ctx.spec, block, &drawn, ctx.events)
}

fn apply_escape_plan_after_draw(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    block: i64,
    drawn: &[HotCard],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let first_is_skill = drawn
        .first()
        .and_then(|card| catalog.spec(card.atom))
        .is_some_and(|spec| spec.is_skill);
    if first_is_skill {
        gain_powered_card_block(state, catalog, spec, block, events)?;
        fire_hook(catalog, HookEvent::AfterBlockGained, state, events)?;
    }
    Ok(())
}

/// EscapePlan's awaited Draw-then-Skill-read program (#2665). Both levels
/// draw one; only the powered block amount differs.
fn escape_plan_program(
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
) -> Option<i64> {
    let block = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::EscapePlan, 0) => 3,
        (CardId::EscapePlan, 1) => 5,
        _ => return None,
    };
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::EscapePlanExact && catalog.args(step.args) == [CompiledArg::I(block)])
        .then_some(block)
}

pub(crate) fn resume_escape_plan_after_draw(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    drawn: &[DrawEntry],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let block = escape_plan_program(catalog, spec).ok_or(EngineRefusal::ContinuationNotModeled)?;
    let drawn = drawn
        .iter()
        .copied()
        .map(DrawEntry::card)
        .collect::<Vec<_>>();
    apply_escape_plan_after_draw(state, catalog, spec, block, &drawn, events)
}

/// `expertise_exact` — Expertise.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) normalizes physical identities, awaits
/// one Draw command, then `_expertise_after_draw` walks that command's
/// exact returned identities. Every returned uid/payload must still occur
/// exactly once in Hand before the synchronous loop adds turn-scoped Retain.
/// The loop has no ending gate, so a terminal draw still marks its completed
/// returned prefix.
pub(crate) fn expertise_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("expertise_exact"));
    };
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Expertise, 0) => 2,
        (CardId::Expertise, 1) => 3,
        _ => return Err(EngineRefusal::MalformedArgs("expertise_exact owner")),
    };
    if *amount != expected
        || crate::content_tables::card_row(CardId::Expertise, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("expertise_exact row"));
    }

    let amount = usize::try_from(*amount)
        .map_err(|_| EngineRefusal::MalformedArgs("expertise_exact amount"))?;
    // Clone-rehearse before any live write (a probe failure leaves the live
    // state and events untouched).
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    {
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
        run_expertise_draw_and_suffix(&mut probe_ctx, amount)?;
    }
    run_expertise_draw_and_suffix(ctx, amount)
}

/// Expertise's Draw plus its returned-identity Retain suffix, shared by the
/// clone rehearsal and the live run.
fn run_expertise_draw_and_suffix(
    ctx: &mut StepCtx<'_>,
    amount: usize,
) -> Result<(), EngineRefusal> {
    // Expertise 0x39c914 retains the Draw's exact returned objects. Under a
    // CardPlay frame the Draw is an owned tail (#2665): a Hellraiser
    // selection or Stratagem reshuffle can suspend it, and the Retain suffix
    // resumes after the Draw returns.
    let drawn: Vec<HotCard> = if matches!(
        ctx.state.frames.top(),
        Some(crate::frame::Frame::CardPlay { .. })
    ) {
        match crate::engine::play::draw_cardplay_owned_result(ctx, amount)? {
            CardDrawResult::Complete(drawn) => drawn.into_iter().map(DrawEntry::card).collect(),
            CardDrawResult::Suspended => return Ok(()),
        }
    } else {
        let mut drawn = Vec::new();
        draw_cards_into(
            ctx.state,
            ctx.catalog,
            amount,
            DrawSource::Command,
            ctx.events,
            &mut drawn,
        )?;
        drawn
    };
    apply_expertise_after_draw(ctx.state, &drawn)
}

fn apply_expertise_after_draw(
    state: &mut HotState,
    drawn: &[HotCard],
) -> Result<(), EngineRefusal> {
    // #2665 retained/removed/moved audit: every returned uid must resolve to
    // exactly one live object. Duplicates refuse atomically (the whole audit
    // precedes any write); a Hellraiser-consumed object keeps the Retain its
    // return earned, since the turn-scoped flag follows the uid rather than
    // the pile. Nested-autoplay flag drift is legitimate, so identity pins
    // (uid, atom) only.
    //
    // #3136: a DUPE Strike Hellraiser auto-played and removed inside the Draw
    // is no pile member but is still a returned object. It must be the one
    // retained removed object for its uid (no live alias), and it receives
    // the same Retain write on that retained instance below.
    // `drawn_object_card` resolves exactly one live pile member XOR one
    // retained removed object for the uid; anything else refuses here,
    // before the first write.
    for returned in drawn {
        let resolved = crate::engine::draw::drawn_object_card(state, returned.uid)
            .map_err(|_| EngineRefusal::MalformedArgs("expertise returned card identity"))?;
        if resolved.atom != returned.atom {
            return Err(EngineRefusal::MalformedArgs(
                "expertise returned card identity",
            ));
        }
    }
    // `_expertise_after_draw` publishes a nonempty replacement batch via
    // `_commit_live_card_piles(..., force_exact=True)`. Keyword slots are
    // invisible to CompareTo, so this promotes even without an equal
    // sibling; an empty Draw performs no commit and remains inert.
    if !drawn.is_empty() {
        state.exact_piles = true;
    }
    // `CardCmd.ApplySingleTurnRetain` RVA `0x12fe70` calls
    // `CardModel.GiveSingleTurnRetain` on the returned object at IL_000d with
    // no pile or removal gate; the rest is NCard visuals. A removed object
    // therefore takes the write on its retained instance, never on the live
    // census.
    for returned in drawn {
        if state
            .card_states
            .removed_draw_object(returned.uid)
            .is_some()
        {
            let mut instance = crate::engine::draw::drawn_object_instance(state, returned.uid)?;
            instance.transient_retain = true;
            crate::engine::draw::set_drawn_object_instance(state, returned.uid, instance)?;
        } else {
            state.card_states.set_transient_retain(returned.uid);
        }
    }
    Ok(())
}

/// Expertise's awaited Draw-then-Retain program (#2665): level 0 draws two
/// and retains, level 1 draws three.
fn expertise_program(
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
) -> Option<usize> {
    let amount = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::Expertise, 0) => 2,
        (CardId::Expertise, 1) => 3,
        _ => return None,
    };
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::ExpertiseExact
        && catalog.args(step.args) == [CompiledArg::I(amount as i64)])
    .then_some(amount)
}

pub(crate) fn resume_expertise_after_draw(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    drawn: &[DrawEntry],
    _events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    expertise_program(catalog, spec).ok_or(EngineRefusal::ContinuationNotModeled)?;
    let drawn = drawn
        .iter()
        .copied()
        .map(DrawEntry::card)
        .collect::<Vec<_>>();
    apply_expertise_after_draw(state, &drawn)
}

/// `("expose_exact", vulnerable)` — Expose.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). On a live target and a running combat:
/// `_lose_all_monster_block(s, target, BreakerIdentity.PLAYER, log)`,
/// then **either** clear the surviving `ArtifactPower` instance **or**
/// `apply_monster_debuff(target, "vuln", _lamp_amount(s, target, step[1]))`.
/// The order is load-bearing in Python: LoseBlock's `AfterBlockBroken`
/// consumers run *before* the remainder of the body, and one of them (Hand
/// Drill) can eat the Artifact stack this branch then tests.
///
/// The catalog-aware block-removal seam runs
/// `_hand_drill_after_block_broken` (frozen Python, deleted #2827) for Hand Drill's Vulnerable 2
/// and `_burrowed_after_block_broken` for Tunneler's Burrowed/DIZZY
/// subscriber before Expose tests the surviving Artifact and applies its own
/// Vulnerable. That preserves the load-bearing Python order without treating
/// either listener as card-sourced; Vicious observation is authenticated by
/// the shared catalog-aware power path.
pub(crate) fn expose_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(vulnerable)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("expose_exact"));
    };
    let vulnerable: i32 = (*vulnerable)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("expose_exact"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    // Python's own guard, before either command.
    if ctx.state.history.over || ctx.state.monsters[target].hp <= 0 {
        return Ok(());
    }
    crate::engine::damage::lose_all_monster_block_with_catalog(
        ctx.state,
        ctx.catalog,
        target,
        ctx.events,
    )?;
    crate::engine::damage::apply_power_monster_debuff_with_catalog(
        ctx.state,
        ctx.catalog,
        target,
        PowerId::Vuln,
        MiseryToken::Vuln,
        vulnerable,
        ctx.events,
    )
}

/// `fade_exact` — exact typed-AnyAlly temporary Dexterity (#1561 R30).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) (the shared
/// `coordinate_exact`/`fade_exact` arm). Validates the whole `FADE`
/// body/metadata and the live `CardPlayFrame`, resolves the recipient with
/// `_resolve_ally_target_key`, then `_apply_temporary_stat_to_player` for
/// temporary Dexterity of `step[1]`.
///
/// The target-keyed wrapper shares Coordinate's authenticated AnyAlly frame.
/// Local ownership writes the existing TempDexterity power slot (already read
/// by powered block and removed at local side end); remote ownership writes
/// the compact teammate quotient and expires later in fixed Player order.
/// The target-keyed writer checks every fallible condition before its first
/// mutation, so local and remote overflow refuse without leaking state/events.
pub(crate) fn fade_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let upgrade = i32::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::Fade || upgrade > 1 {
        return Err(EngineRefusal::MalformedArgs("fade_exact"));
    }
    let expected = 6 + 3 * upgrade;
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount == i64::from(expected) => expected,
        _ => return Err(EngineRefusal::MalformedArgs("fade_exact")),
    };
    let key = crate::engine::allies::target_key(ctx)?;
    crate::engine::allies::gain_temp_dexterity(ctx.state, key, amount, ctx.events)
}

/// Hidden Daggers' generated L0 Shivs and returned-instance upgrade pass.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Admits only `HIDDEN_DAGGERS`
/// with the exact two-step body `(select hand 2 2 None discard,
/// generate_shivs_then_upgrade_exact 2 level)`, then
/// `add_generated_shivs_then_upgrade(s, step[1], step[2], log,
/// exact_safety_cards=(card,))`.
///
/// Unit E supplies the exact generated `select` continuation for the first
/// step. Unit C's shared card helper completes both serial generated-entry
/// transactions before upgrading only the returned physical prefix, with an
/// ending gate between phases and a whole-body clone rehearsal.
pub(crate) fn generate_shivs_then_upgrade_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(count), CompiledArg::I(upgrade)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_upgrade_exact",
        ));
    };
    if ctx.spec.identity.id != CardId::HiddenDaggers
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *count != 2
        || *upgrade != i64::from(ctx.spec.identity.upgrade)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs(
            "generate_shivs_then_upgrade_exact owner",
        ));
    }
    inject_generated_shivs_then_upgrade(
        ctx.state,
        ctx.catalog,
        2,
        (*upgrade)
            .try_into()
            .map_err(|_| EngineRefusal::MalformedArgs("generated Shiv upgrade"))?,
        ctx.events,
    )
}

/// `("mirage_total_poison_block_exact",)` — Mirage.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `calculated = sum(monster.poison for
/// monster in s.alive())`, then `_gain_powered_card_block(s, card, calculated,
/// log)`. Mirage's `WithMultiplier` lambda consumes no RNG and its
/// `CalculatedBlock` is base 0 / extra 1, so the sum over **living** enemies
/// is the complete powered-block operand.
///
/// #1342's honesty note said the sum was always zero inside the admitted
/// surface, because Poison was outside `IMPLEMENTED_POWERS` and nothing could
/// put a stack on the board — so the body could not be distinguished from
/// `block 0`. Slice 2 (#1367) landed Poison, and BUBBLE_BUBBLE shares this
/// family's smoke pool, so the operand is non-zero on real trajectories.
pub(crate) fn mirage_total_poison_block_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "mirage_total_poison_block_exact",
        ));
    }
    let calculated: i64 = ctx
        .state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0)
        .map(|monster| i64::from(monster.powers.value(PowerId::Poison)))
        .sum();
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, calculated, ctx.events)?;
    fire_hook(
        ctx.catalog,
        HookEvent::AfterBlockGained,
        ctx.state,
        ctx.events,
    )
}

/// `("pillage_exact", damage)` — Pillage.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). One powered targeted `player_attack` for
/// 6/9, then `_pillage_continue_draws`: **distinct one-card Draw
/// commands**, continued only while the exact returned instance is an Attack
/// and the reentrantly live hand still has space.
///
/// Three exit conditions, and the loop needs all three to terminate: the Draw
/// returned nothing (empty draw and discard, or a full hand), the returned
/// card is not an Attack, or the hand is now full. The hand test is *after*
/// the draw and reads the live pile, not the count the loop started with.
pub(crate) fn pillage_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("pillage_exact"));
    };
    let damage = *damage;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    crate::engine::damage::player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    continue_pillage_draws(ctx.state, ctx.catalog, ctx.events)
}

fn pillage_draw_continues(
    state: &HotState,
    catalog: &crate::catalog::Catalog,
    drawn: &[DrawEntry],
) -> Result<bool, EngineRefusal> {
    let attack = match drawn {
        [] => false,
        [entry] => {
            catalog
                .spec(entry.card().atom)
                .ok_or(EngineRefusal::UnknownAtom(entry.card().atom))?
                .is_attack
        }
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    Ok(attack && state.piles.get(PileId::Hand).len() < MAX_CARDS_IN_HAND)
}

fn continue_pillage_draws(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    loop {
        match draw_cards_for_card_result(state, catalog, 1, DrawCaller::Pillage, events)? {
            CardDrawResult::Suspended => return Ok(()),
            CardDrawResult::Complete(drawn) => {
                if !pillage_draw_continues(state, catalog, &drawn)? {
                    return Ok(());
                }
            }
        }
    }
}

pub(crate) fn resume_pillage_after_draw(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    drawn: &[DrawEntry],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if pillage_draw_continues(state, catalog, drawn)? {
        continue_pillage_draws(state, catalog, events)?;
    }
    Ok(())
}

/// `("poison_if_poisoned", amount)` — Bubble Bubble.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). When `not s.over and target.hp > 0 and
/// target.poison > 0`, applies card-sourced Poison of
/// `_card_power_amount_given(s, target, "poison", step[1])`.
///
/// `HasPower<PoisonPower>` is tested **before** entering the visible Apply
/// path, which is the reason the guard is written as three separate
/// conditions rather than folded into the helper: a false condition touches
/// neither Unsettling Lamp's latch nor the target's Artifact stack, so a
/// Bubble Bubble played into an unpoisoned enemy is observably a no-op rather
/// than a blocked application.
pub(crate) fn poison_if_poisoned(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("poison_if_poisoned"));
    };
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("poison_if_poisoned"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let poisoned = ctx
        .state
        .monsters
        .get(target)
        .is_some_and(|monster| monster.hp > 0 && monster.powers.value(PowerId::Poison) > 0);
    if ctx.state.history.over || !poisoned {
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

/// `poison_random_serial` — Bouncing Flask's body-owned random target loop.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Admits only `BOUNCING_FLASK`
/// `(3, 3)` / `(3, 4)`; per iteration, rebuilds the HittableEnemies pool,
/// `_roll_target(s)` (one `CombatTargets` draw for any nonempty pool,
/// singleton included), and applies card-sourced Poison, breaking on
/// `s.over` or an empty pool.
///
/// The application is [`apply_card_monster_debuff`] — the same call
/// `poison_if_poisoned` makes, `_card_power_amount_given` and the instance-uid
/// allocation included — and the pool rebuild is
/// [`crate::engine::damage::alive_targets`].
///
/// The shared **`CombatTargets` random-target roll**
/// (`_roll_target`, frozen Python, deleted #2827): `Rng::NextItem` over the live hittable pool,
/// which consumes exactly one `NextInt(0, n)` draw for a nonempty pool and
/// none at all for an empty one. The primitive lives in `engine::damage` so
/// later card, orb, and monster callers share the same stream semantics.
pub(crate) fn poison_random_serial(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount), CompiledArg::I(repeats)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("poison_random_serial"));
    };
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("poison_random_serial amount"))?;
    for _ in 0..*repeats {
        if ctx.state.history.over {
            break;
        }
        let Some(target) = roll_target(ctx.state)? else {
            break;
        };
        apply_card_monster_debuff(
            ctx.state,
            target,
            PowerId::Poison,
            MiseryToken::Poison,
            amount,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `up_my_sleeve_cost_exact` — append the permanent local cost delta to the
/// exact active Up My Sleeve instance.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The immutable Python card tuple is
/// rewritten at its unique logical active location. Manual and gathered plays
/// use Play, while a collected Sly source retains its non-null Hand/Draw/
/// Discard pile through the body. Rust writes the same UID's ordered side-table
/// state before ordinary result routing.
pub(crate) fn up_my_sleeve_cost_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("up_my_sleeve_cost_exact"));
    };
    let (pile, index) = crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if pile == PileId::Exhaust {
        return Err(EngineRefusal::MalformedArgs(
            "up_my_sleeve_cost_exact source",
        ));
    }
    let active = ctx.state.piles.get(pile).as_slice()[index];
    if ctx.catalog.spec(active.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "up_my_sleeve_cost_exact source",
        ));
    }
    ctx.state.card_states.append_local_cost_modifier(
        active.uid,
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: *amount,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    );
    let active_state = ctx.state.card_states.get_ref(active.uid);
    let distinguishable_tie = PileId::ALL.iter().any(|pile| {
        ctx.state.piles.get(*pile).as_slice().iter().any(|card| {
            card.uid != active.uid
                && card.atom == active.atom
                && (card.flags != active.flags
                    || ctx.state.card_states.get_ref(card.uid) != active_state)
        })
    });
    if distinguishable_tie {
        ctx.state.exact_piles = true;
    }
    Ok(())
}

/// `well_laid_plans` — apply the unique WellLaidPlansPower marker.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one line setting the state's
/// `well_laid_plans` marker, because `WellLaidPlansPower` is Type 1 /
/// StackType 2 and a second copy preserves the single unique instance.
///
/// The player-side registry authenticates the boolean wire shape, Power cards
/// leave every physical pile after resolving, and the owner turn-end path
/// suppresses the complete hand flush while this marker is present.
pub(crate) fn well_laid_plans(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("well_laid_plans"));
    }
    if ctx.state.history.over {
        return Ok(());
    }
    if ctx.state.powers.value(PowerId::WellLaidPlans) == 0 {
        ctx.state
            .powers
            .set(PowerId::WellLaidPlans, SlotWire::Bool, 1);
        note_power(ctx.events, Subject::Player, PowerId::WellLaidPlans, 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CardIdentity, Catalog};
    use crate::engine::{Action, Event, SelectionRef, apply_action_into};
    use crate::hot::{HotCard, HotState, MonsterOverride, PileId};
    use crate::ids::{CardId, MonsterKind};
    use crate::powers::SlotWire;

    /// The R0.5 starter-deck-vs-TOADPOLE entry, built the ordinary way.
    ///
    /// `engine::tests::fixture` is private to that module and `engine/mod.rs`
    /// is not this PR's to edit, so the fixture is read here directly — the
    /// same document, through the same boundary.
    fn fixture() -> (HotState, Catalog) {
        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (state, catalog)
    }

    /// The kinds `steps::FAMILY_OF` assigns to this family.
    ///
    /// Listed rather than filtered out of `FAMILY_OF`, because that index is
    /// keyed by family *name* and `tests/hot_path_contract.rs` bans text
    /// comparison anywhere in this tree (D2). The list is checked against the
    /// dispatch below — every entry must route back into this file — so a
    /// wrong entry fails rather than passing silently.
    const FAMILY_KINDS: [StepKind; 13] = [
        StepKind::BladeSymphonyExact,
        StepKind::ConcoctExact,
        StepKind::EscapePlanExact,
        StepKind::ExpertiseExact,
        StepKind::ExposeExact,
        StepKind::FadeExact,
        StepKind::GenerateShivsThenUpgradeExact,
        StepKind::MirageTotalPoisonBlockExact,
        StepKind::PillageExact,
        StepKind::PoisonIfPoisoned,
        StepKind::PoisonRandomSerial,
        StepKind::UpMySleeveCostExact,
        StepKind::WellLaidPlans,
    ];

    /// The escalation record, as a pin: what this family claims is exactly
    /// what it has ported, and every claim routes back to a kind it owns.
    ///
    /// An `IMPLEMENTED` entry with a still-refusing body is the one failure
    /// mode the differential cannot see, because the admission gate would
    /// refuse the card before any trajectory reached it; the refusal walk
    /// below is what rules it out.
    #[test]
    fn the_family_claims_exactly_what_it_has_ported() {
        for kind in IMPLEMENTED {
            assert!(
                FAMILY_KINDS.contains(kind),
                "{:?} is claimed here but is not one of this file's stubs",
                kind.as_str()
            );
        }
        for kind in FAMILY_KINDS {
            assert_eq!(
                crate::steps::is_implemented(kind),
                IMPLEMENTED.contains(&kind),
                "{:?} is implemented somewhere other than the family that owns it",
                kind.as_str()
            );
        }
    }

    /// Each kind refuses **by its own name**, through the generated dispatch.
    ///
    /// Stronger than reading the stubs: it pins that `steps::apply_step` still
    /// routes each of this family's kinds to this file, and that no body
    /// silently degraded into a no-op `Ok(())` — a refusal that stops naming
    /// its kind is exactly the "weakened stub" a wave review looks for.
    #[test]
    fn every_kind_refuses_by_name_through_the_dispatch() {
        let (mut state, catalog) = fixture();
        let atom = catalog
            .atom(&CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let spec = *catalog.spec(atom).unwrap();
        for kind in FAMILY_KINDS {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
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
                "{:?} did not refuse by name",
                kind.as_str()
            );
            assert!(
                events.is_empty(),
                "{:?} emitted events before refusing",
                kind.as_str()
            );
        }
    }

    #[test]
    fn fade_exact_routes_both_levels_to_local_or_remote_temp_dex_atomically() {
        for (upgrade, amount) in [(0, 6), (1, 9)] {
            let identity = CardIdentity {
                id: CardId::Fade,
                upgrade,
                enchantment: None,
            };
            let mut builder = crate::catalog::CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let args = catalog.args(catalog.steps(&spec)[0].args);

            for key in [0_usize, 1] {
                let mut state = HotState::at_defaults();
                state.hp = 50;
                state.multiplayer_ally_key = 1;
                assert!(
                    state
                        .fanouts
                        .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                            key: 1,
                            ..crate::hot::MultiplayerAllyState::default()
                        })
                );
                let mut events = Vec::new();
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 7,
                    target: Some(key),
                    selection: None,
                    x_value: 0,
                    args,
                    events: &mut events,
                };
                fade_exact(&mut ctx).unwrap();
                assert_eq!(
                    ctx.state.powers.value(PowerId::TempDexterity),
                    if key == 0 { amount } else { 0 }
                );
                assert_eq!(
                    ctx.state.fanouts.multiplayer_ally().temp_dexterity,
                    if key == 1 { amount } else { 0 }
                );
                assert_eq!(events.len(), usize::from(key == 0));
            }
        }

        let identity = CardIdentity {
            id: CardId::Fade,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = crate::catalog::CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = catalog.args(catalog.steps(&spec)[0].args);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.multiplayer_ally_key = 1;
        assert!(
            state
                .fanouts
                .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                    key: 1,
                    temp_dexterity: i32::MAX,
                    ..crate::hot::MultiplayerAllyState::default()
                })
        );
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 3 }];
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(1),
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        assert_eq!(
            fade_exact(&mut ctx),
            Err(EngineRefusal::CounterOverflow("remote temporary dexterity"))
        );
        assert_eq!(*ctx.state, before);
        assert_eq!(*ctx.events, before_events);
    }

    #[test]
    fn unadmitted_fade_public_burst_repeats_and_rolls_back_second_body_overflow() {
        let identity = CardIdentity {
            id: CardId::Fade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = crate::catalog::CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();

        let state_with_temp_dex = |temp_dexterity| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 3;
            state.multiplayer_ally_key = 1;
            assert!(
                state
                    .fanouts
                    .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                        key: 1,
                        temp_dexterity,
                        ..crate::hot::MultiplayerAllyState::default()
                    })
            );
            state.powers.set(PowerId::Burst, SlotWire::Int, 1);
            state
                .monsters_mut()
                .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 20));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            state
        };

        let repeated = apply_action_into(
            &state_with_temp_dex(0),
            &catalog,
            &Action::Play {
                uid: 7,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(repeated.fanouts.multiplayer_ally().temp_dexterity, 12);
        assert_eq!(repeated.powers.value(PowerId::Burst), 0);

        let input = state_with_temp_dex(i32::MAX - 6);
        let mut mutable = input.clone();
        let before_mutable = mutable.clone();
        let mut mutable_events = vec![Event::TurnEnded { turn: 2 }];
        let before_mutable_events = mutable_events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut mutable,
                &catalog,
                7,
                Some(1),
                None,
                &mut mutable_events,
            ),
            Err(EngineRefusal::CounterOverflow("remote temporary dexterity"))
        );
        assert_eq!(
            mutable, before_mutable,
            "mutable play retracts every prefix"
        );
        assert_eq!(
            mutable_events, before_mutable_events,
            "mutable play retracts every event prefix"
        );

        let mut events = vec![Event::TurnEnded { turn: 3 }];
        assert_eq!(
            apply_action_into(
                &input,
                &catalog,
                &Action::Play {
                    uid: 7,
                    target: Some(1),
                    selection: SelectionRef::NONE,
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("remote temporary dexterity"))
        );
        assert_eq!(
            input.fanouts.multiplayer_ally().temp_dexterity,
            i32::MAX - 6
        );
        assert_eq!(input.powers.value(PowerId::Burst), 1);
        assert!(events.is_empty());
    }

    /// Play one `expose_exact` against roster index 0 with the given argument.
    fn expose(
        state: &mut HotState,
        catalog: &Catalog,
        amount: i64,
    ) -> (Vec<Event>, Result<(), EngineRefusal>) {
        let atom = catalog
            .atom(&CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let args = [CompiledArg::I(amount)];
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 0,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        let outcome = crate::steps::apply_step(StepKind::ExposeExact, &mut ctx);
        (events, outcome)
    }

    /// Expose's two commands, in Python's order: all the target's block is
    /// gone and the Vulnerable stack lands on top of whatever was there.
    ///
    #[test]
    fn expose_strips_all_block_then_applies_vulnerable() {
        let (mut state, catalog) = fixture();
        state.monsters_mut()[0].block = 7;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 1);
        let (events, outcome) = expose(&mut state, &catalog, 2);
        assert_eq!(outcome, Ok(()));
        assert_eq!(state.monsters[0].block, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 3);
        assert_eq!(
            events,
            vec![Event::PowerChanged {
                subject: Subject::Monster(state.monsters[0].uid),
                power: PowerId::Vuln,
                amount: 3,
            }]
        );
    }

    #[test]
    fn expose_fires_burrowed_before_applying_vulnerable() {
        let (mut state, catalog) = fixture();
        let monster = &mut state.monsters_mut()[0];
        monster.kind = MonsterKind::Tunneler;
        monster.block = 7;
        monster.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);

        let (events, outcome) = expose(&mut state, &catalog, 2);
        assert_eq!(outcome, Ok(()));
        assert_eq!(state.monsters[0].block, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 2);
        assert_eq!(
            events,
            vec![
                Event::PowerChanged {
                    subject: Subject::Monster(state.monsters[0].uid),
                    power: PowerId::Burrowed,
                    amount: 0,
                },
                Event::PowerChanged {
                    subject: Subject::Monster(state.monsters[0].uid),
                    power: PowerId::Vuln,
                    amount: 2,
                },
            ]
        );
    }

    /// A target that is already dead, and a combat that is already over, are
    /// both Python's own guard rather than a refusal: neither command runs, so
    /// the block survives untouched and no Vulnerable is applied.
    #[test]
    fn expose_runs_neither_command_once_its_guard_fails() {
        for (hp, over) in [(0, false), (12, true)] {
            let (mut state, catalog) = fixture();
            state.monsters_mut()[0].hp = hp;
            state.monsters_mut()[0].block = 7;
            state.history.over = over;
            let (events, outcome) = expose(&mut state, &catalog, 2);
            assert_eq!(outcome, Ok(()));
            assert_eq!(state.monsters[0].block, 7);
            assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 0);
            assert!(events.is_empty());
        }
    }

    /// The shape is body-owned since #1366, so the body is what has to refuse
    /// an argument tuple it does not implement — by name, with nothing written.
    #[test]
    fn expose_refuses_a_shape_it_does_not_own() {
        let (mut state, catalog) = fixture();
        state.monsters_mut()[0].block = 7;
        let atom = catalog
            .atom(&CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let args = [CompiledArg::I(2), CompiledArg::I(2)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            crate::steps::apply_step(StepKind::ExposeExact, &mut ctx),
            Err(EngineRefusal::MalformedArgs("expose_exact"))
        );
        assert!(events.is_empty());
        assert_eq!(state.monsters[0].block, 7);
    }

    #[test]
    fn expertise_draws_exact_level_count_and_marks_only_returned_instances() {
        for (upgrade, amount) in [(0, 2_u32), (1, 3_u32)] {
            let mut builder = crate::catalog::CatalogBuilder::new();
            let source_identity = CardIdentity {
                id: CardId::Expertise,
                upgrade,
                enchantment: None,
            };
            let source_atom = builder.intern(source_identity).unwrap();
            let defend_atom = builder
                .intern(CardIdentity {
                    id: CardId::DefendSilent,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.energy = 3;
            state.next_card_uid = amount + 3;
            state
                .monsters_mut()
                .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 40));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: source_atom,
                flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((0..=amount).map(|offset| HotCard {
                    uid: offset + 2,
                    atom: defend_atom,
                    flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));

            crate::engine::play::play_card(&mut state, &catalog, 1, None, None, &mut Vec::new())
                .unwrap();

            assert_eq!(state.piles.get(PileId::Hand).len(), amount as usize);
            assert!(
                state.exact_piles,
                "Expertise's nonempty keyword rewrite forces exact mode"
            );
            for uid in 2..amount + 2 {
                assert!(state.card_states.get(uid).transient_retain);
                assert!(!state.card_states.get(uid).local_retain);
            }
            assert!(
                !state.card_states.get(amount + 2).transient_retain,
                "the undrawn tail must remain untouched"
            );
        }
    }

    #[test]
    fn expertise_rejects_shape_drift_before_draw_or_event_mutation() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Expertise,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend_atom = builder
            .intern(CardIdentity {
                id: CardId::DefendSilent,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 7 }];
        let before_events = events.clone();
        let args = [CompiledArg::I(3)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            expertise_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("expertise_exact row"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 2,
            atom: defend_atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: defend_atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let before = state.clone();
        let args = [CompiledArg::I(2)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            expertise_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "expertise returned card identity"
            ))
        );
        assert_eq!(state, before, "duplicate returned uid refusal is atomic");
        assert_eq!(events, before_events);
    }

    #[test]
    fn expertise_empty_draw_does_not_publish_exact_mode() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        let expertise_atom = builder
            .intern(CardIdentity {
                id: CardId::Expertise,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state
            .monsters_mut()
            .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 40));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: expertise_atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        crate::engine::play::play_card(&mut state, &catalog, 1, None, None, &mut Vec::new())
            .unwrap();

        assert!(!state.exact_piles);
        assert!(state.card_states.as_slice().is_empty());
    }

    #[test]
    fn expertise_terminal_draw_marks_the_completed_prefix_without_a_second_draw() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        let expertise_atom = builder
            .intern(CardIdentity {
                id: CardId::Expertise,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend_atom = builder
            .intern(CardIdentity {
                id: CardId::DefendSilent,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state
            .monsters_mut()
            .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 1));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: expertise_atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend_atom,
                flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.fanouts.set_cacophony_left(1);

        crate::engine::play::play_card(&mut state, &catalog, 1, None, None, &mut Vec::new())
            .unwrap();

        assert!(state.history.over);
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 2);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 3);
        assert!(state.card_states.get(2).transient_retain);
        assert!(!state.card_states.get(3).transient_retain);
    }
}
