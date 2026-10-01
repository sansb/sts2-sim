//! Card-step bodies for the `content/cards/necrobinder_rare.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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

//! # Wave status (#1329 + #1563 + #1751 + #1961): 8 of 8 ported
//!
//! End of Days, Hang, Shared Fate and the unlinked admitted surface of The
//! Scythe are representable with the existing listener-aware Doom/death,
//! signed-Strength and physical-card primitives. Undeath uses the shared
//! exact-source generated-clone transaction, while Misery closes the current
//! scalar Type-2 projection. Call of the Void owns its exact persistent
//! generation pool/listener and combat-local Ethereal payload.

use super::StepCtx;
use crate::catalog::CompiledArg;
use crate::engine::EngineRefusal;
use crate::engine::cards::inject_generated_clones_bottom;
use crate::engine::damage::{
    MiserySnapshotEntry, apply_card_monster_debuff, apply_card_monster_strength_delta,
    apply_owner_strength, gain_powered_card_block, player_attack_from_card,
};
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_LEGACY, HotCard, MiseryToken, PileId,
};
use crate::ids::{CardId, PowerId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::CallOfTheVoid,
    StepKind::EndOfDaysExact,
    StepKind::GlimpseBeyondExact,
    StepKind::HangExact,
    StepKind::MiseryExact,
    StepKind::SharedFateExact,
    StepKind::TheScytheExact,
    StepKind::UndeathExact,
];

#[cfg(test)]
pub(crate) fn call_of_the_void_pool() -> [CardId; 78] {
    crate::content_tables::CALL_OF_THE_VOID_POOL_V1101
}

pub(crate) const MAX_CALL_OF_THE_VOID_GENERATED_PER_TURN: i32 = 256;

fn exact_active_source(ctx: &StepCtx<'_>, id: CardId) -> Result<HotCard, EngineRefusal> {
    let mut sources = PileId::ALL.into_iter().flat_map(|pile| {
        ctx.state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(|card| card.uid == ctx.source_uid)
    });
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        });
    };
    let additional = sources.count();
    if additional != 0 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 1 + additional,
        });
    }
    let identity = ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    if identity != ctx.spec.identity || identity.id != id || !matches!(identity.upgrade, 0 | 1) {
        return Err(EngineRefusal::MalformedArgs(
            "necrobinder rare active source",
        ));
    }
    Ok(source)
}

/// `call_of_the_void` — exact persistent BeforeHandDraw generator.
/// Native v111 CallOfTheVoidPower.BeforeHandDraw (0x336b08) reads the owner
/// CharacterCardPool; the 78-card frozen Python list below is Necrobinder only.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) stacks CallOfTheVoidPower and registers
/// its zero-to-positive `BeforeHandDraw` listener. The listener
/// at `_advance_before_hand_draw_power_frame` shuffles the frozen
/// `CALL_OF_THE_VOID_POOL_V1101` on its dedicated generation stream, applies
/// Ethereal, and submits one plural result batch. Each stack independently
/// shuffles all 78 entries and takes the first, so repeated identities are
/// exact and intentional.
pub(crate) fn call_of_the_void(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::CallOfTheVoid
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || ctx.args != [CompiledArg::I(1)]
        || !matches!(
            ctx.spec.row.steps,
            [crate::content_tables::Step {
                kind: StepKind::CallOfTheVoid,
                args: [crate::content_tables::Arg::I(1)],
            }]
        )
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("call_of_the_void"));
    }
    exact_active_source(ctx, CardId::CallOfTheVoid)?;
    if !crate::engine::cards::owner_listener_provenance_is_exact(ctx.state, ctx.catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Call of the Void generation provenance",
        ));
    }
    let old = ctx.state.powers.value(PowerId::CallOfTheVoid);
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Call of the Void"))?;
    if updated > MAX_CALL_OF_THE_VOID_GENERATED_PER_TURN {
        return Err(EngineRefusal::MalformedArgs(
            "Call of the Void generation batch exceeds 256 cards",
        ));
    }
    if !crate::boundary::call_of_the_void_catalog_closure_is_exact(
        ctx.catalog,
        ctx.state.reward_card_pool,
    ) {
        return Err(EngineRefusal::MalformedArgs(
            "Call of the Void generation closure",
        ));
    }
    if old == 0
        && !ctx
            .state
            .fanouts
            .register_before_hand_draw(PowerId::CallOfTheVoid)
    {
        return Err(EngineRefusal::CounterOverflow("before hand draw listeners"));
    }
    ctx.state.powers.set(
        PowerId::CallOfTheVoid,
        crate::powers::SlotWire::Int,
        updated,
    );
    ctx.state.publish_call_of_the_void_generation_pool();
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::CallOfTheVoid,
        updated,
    );
    Ok(())
}

/// `end_of_days_exact` — one ordered Doom application wave followed by the
/// fresh ordered Doom-kill batch.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) materializes the first HittableEnemies
/// snapshot, separately awaits Doom 29/37 on each still-live member, then
/// takes a fresh hittable snapshot and issues one ordered `_doom_enemy_side_end`
/// batch (including the Book Repair batch response).
///
/// Each application completes the represented AfterPowerAmountChanged walk
/// before the next member is considered. The kill batch then takes its own
/// fresh snapshot and resolves deaths serially. Artifact and Unsettling Lamp
/// are resolved by the shared card-sourced Type-2 reader
/// (`damage::card_monster_type_two_amount`), which needs this card's own
/// active CardPlay — `play.rs` gives [`preflight_end_of_days`] one (#3209
/// class). Anything else still refuses rather than being approximated here.
pub(crate) fn end_of_days_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let mut next = ctx.state.clone();
    let mut emitted = Vec::new();
    end_of_days_inner(
        &mut next,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        ctx.target,
        ctx.selection,
        ctx.x_value,
        ctx.args,
        &mut emitted,
    )?;
    *ctx.state = next;
    ctx.events.extend(emitted);
    Ok(())
}

/// `EndOfDays/<OnPlay>d__6` RVA `0x39b604` awaits `PowerCmd.Apply<DoomPower>` per
/// hittable enemy (IL_01ad) and then `DoomPower.DoomKill` (IL_024c), which has
/// no combat gate (`<DoomKill>d__6` `0x3392c8`). Each Apply returns at
/// `IsEnding` (`<Apply>d__1`1` `0x3ef988` IL_0025-002a), and the Doom writer
/// carries that gate itself; the DoomKill must still run while the combat is
/// ending, so the body's own gate stays `history.over` (#3515).
#[allow(clippy::too_many_arguments)]
fn end_of_days_inner(
    state: &mut crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    source_uid: u32,
    target: Option<usize>,
    selection: Option<u32>,
    x_value: i64,
    args: &[CompiledArg],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !end_of_days_program_is_exact(catalog, catalog.steps(spec), args) {
        return Err(EngineRefusal::MalformedArgs("end_of_days_exact program"));
    }
    let amount = match (spec.identity.id, spec.identity.upgrade, args) {
        (CardId::EndOfDays, 0, [CompiledArg::I(29)]) => 29,
        (CardId::EndOfDays, 1, [CompiledArg::I(37)]) => 37,
        _ => return Err(EngineRefusal::MalformedArgs("end_of_days_exact")),
    };
    if target.is_some() || selection.is_some() || x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("end_of_days_exact operands"));
    }
    let ctx = StepCtx {
        state,
        catalog,
        spec,
        source_uid,
        target,
        selection,
        x_value,
        args,
        events,
    };
    exact_active_source(&ctx, CardId::EndOfDays)?;
    if ctx.state.history.over {
        return Ok(());
    }

    // Native HittableEnemies freezes creature objects, not only their roster
    // positions. A death listener can replace an Axebot in the same slot, and
    // that fresh uid must not inherit the old object's pending application.
    let targets = end_of_days_target_snapshot(ctx.state);
    apply_end_of_days_snapshot(ctx.state, amount, &targets, ctx.events)?;
    if !ctx.state.history.over {
        crate::engine::turn::doom_enemy_side_end(ctx.state, ctx.catalog, ctx.events)?;
    }
    Ok(())
}

fn end_of_days_target_snapshot(state: &crate::hot::HotState) -> Vec<(usize, u32)> {
    crate::engine::damage::alive_targets(state)
        .into_iter()
        .map(|target| (target, state.monsters[target].uid))
        .collect()
}

/// Each member is one `PowerCmd.Apply<DoomPower>` (`0x39b604` IL_01ad), which
/// returns at `IsEnding` (`<Apply>d__1`1` `0x3ef988` IL_0025-002a); the Doom
/// writer (`apply_card_monster_debuff`) carries that gate (#3515).
fn apply_end_of_days_snapshot(
    state: &mut crate::hot::HotState,
    amount: i32,
    targets: &[(usize, u32)],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    // Listener-driven deaths and same-slot replacements are observed between
    // members while the native object identity remains frozen.
    for &(target, uid) in targets {
        if state.history.over
            || state
                .monsters
                .get(target)
                .is_none_or(|monster| monster.hp <= 0 || monster.uid != uid)
        {
            continue;
        }
        state.monsters[target]
            .powers
            .value(PowerId::Doom)
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("monster doom"))?;
        apply_card_monster_debuff(
            state,
            target,
            PowerId::Doom,
            MiseryToken::Doom,
            amount,
            events,
        )?;
    }
    Ok(())
}

fn end_of_days_program_is_exact(
    catalog: &crate::catalog::Catalog,
    steps: &[crate::catalog::CompiledStep],
    args: &[CompiledArg],
) -> bool {
    matches!(steps, [step]
        if step.kind == StepKind::EndOfDaysExact && catalog.args(step.args) == args)
}

/// Rehearse the exact End of Days body before the public play prefix mutates
/// energy, source piles, history, events, or target RNG.
pub(crate) fn preflight_end_of_days(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    source_uid: u32,
    spec: &crate::catalog::CardSpec,
) -> Result<(), EngineRefusal> {
    let [step] = catalog.steps(spec) else {
        return Err(EngineRefusal::MalformedArgs("end_of_days_exact program"));
    };
    let args = catalog.args(step.args);
    let mut next = state.clone();
    end_of_days_inner(
        &mut next,
        catalog,
        spec,
        source_uid,
        None,
        None,
        0,
        args,
        &mut Vec::new(),
    )
}

/// Private exact Glimpse Beyond body foundation.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `GlimpseBeyond::.ctor` RVA `0xe13aa` fixes cost 1 Skill/AllAllies/Exhaust;
/// `OnUpgrade` RVA `0xe141f` raises Cards 3 -> 4. The exact body lives in
/// `GlimpseBeyond/<OnPlay>d__9::MoveNext` RVA `0x3a1968`: native freezes the
/// same-side living Player roster at IL `0x009e-0x00d8`, creates fresh L0
/// Souls for each recipient at IL `0x0103-0x012e`, then separately awaits each
/// recipient's random-Draw plural generated-card command at
/// IL `0x0130-0x019b` with this card's owner as creator.
///
/// Python `_apply_glimpse_beyond_souls` (frozen, deleted #2827) and
/// `_commit_glimpse_beyond_souls` preserve the same frozen roster,
/// local-creator history, global Shuffle RNG, remote ownership, physical-entry
/// split, generated-hook ordering, terminal suffix, and whole-body rehearsal.
///
/// Shared admission attaches the exact L0 Soul leaf to every physical or
/// recursively generated Glimpse source, and the public play wrapper rehearses
/// the complete replay series before publishing its first action prefix.
pub(crate) fn apply_glimpse_beyond_foundation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::GlimpseBeyond, 0, [CompiledArg::I(3)]) => 3,
        (CardId::GlimpseBeyond, 1, [CompiledArg::I(4)]) => 4,
        _ => return Err(EngineRefusal::MalformedArgs("glimpse_beyond_exact")),
    };
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::GlimpseBeyondExact
                && ctx.catalog.args(step.args) == ctx.args)
    {
        return Err(EngineRefusal::MalformedArgs("glimpse_beyond_exact program"));
    }
    exact_active_source(ctx, CardId::GlimpseBeyond)?;
    crate::engine::allies::glimpse_beyond(ctx.state, ctx.catalog, amount, ctx.events)
}

pub(crate) fn glimpse_beyond_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_glimpse_beyond_foundation(ctx)
}

/// `hang_exact` — Hang's source-specific hit and surviving-target stack.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) captures the target uid, attacks for
/// 10/13, then (only for the same surviving target) applies `max(2, current)`
/// through the card-debuff path without exceeding PowerModel's 999,999,999
/// cap.
///
/// `player_attack` (frozen Python, deleted #2827) multiplies the complete live Decimal value
/// only for an exact Hang source, after Shrink and before Soar, with the one
/// shared final floor. The body then rechecks the captured target uid and
/// applies `max(2, current)` without exceeding PowerModel's 999,999,999 cap.
pub(crate) fn hang_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Hang, 0, [CompiledArg::I(10)]) => 10,
        (CardId::Hang, 1, [CompiledArg::I(13)]) => 13,
        _ => return Err(EngineRefusal::MalformedArgs("hang_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let target_uid = ctx
        .state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?
        .uid;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Ok(());
    };
    if ctx.state.history.over || monster.hp <= 0 || monster.uid != target_uid {
        return Ok(());
    }
    let current = monster.powers.value(PowerId::Hang);
    let amount = current.max(2).min(999_999_999_i32.saturating_sub(current));
    if amount > 0 {
        apply_card_monster_debuff(
            ctx.state,
            target,
            PowerId::Hang,
            MiseryToken::Hang,
            amount,
            ctx.events,
        )?;
    }
    Ok(())
}

/// Misery's frozen concrete Type-2 snapshot, powered attack, and recipient-
/// major clone walk.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// the OnPlay body is `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` and its
/// filter/clone lambdas are `Misery/<>c::<OnPlay>b__3_0` `0x3ad30a` and
/// `Misery/<>c::<OnPlay>b__3_1` `0x3ad315`; the temporary-wrapper fold is
/// `MoveNext` IL_00a1-IL_0133 with predicate
/// `Misery/<>c__DisplayClass3_0::<OnPlay>b__2` `0x3ad335`; post-hit
/// `HittableEnemies` is `CombatState::get_HittableEnemies` `0x1373a5`
/// (predicate `CombatState/<>c::<get_HittableEnemies>b__67_0` `0x3f91ba`),
/// taken at `MoveNext` IL_01ed after the awaited attack; and Apply/Modify are
/// `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` /
/// `PowerCmd/<ModifyAmount>d__6::MoveNext` `0x3f032c`.
/// Python `_misery_exact` (frozen, deleted #2827).
/// Python `_apply_misery_power_copy`.
///
/// [`misery_scalar_snapshot`] closes the full currently represented scalar
/// subset and refuses every absent concrete family or malformed acquisition
/// ledger. The source snapshot precedes the attack. The post-hit roster is
/// then frozen by `(slot, uid)`, excluding only the original creature uid, so
/// a lethal Stock replacement is eligible. Each separately awaited copy
/// re-resolves that identity and uses the ordinary card Type-2 command,
/// preserving Artifact, Unsettling Lamp, power listeners, acquisition order,
/// and each power's existing expiry/death cleanup.
pub(crate) fn misery_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let mut probe = ctx.state.clone();
    misery_inner(
        &mut probe,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        ctx.target,
        ctx.selection,
        ctx.x_value,
        ctx.args,
        &mut Vec::new(),
    )?;
    misery_inner(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        ctx.target,
        ctx.selection,
        ctx.x_value,
        ctx.args,
        ctx.events,
    )
}

/// `Misery/<OnPlay>d__3` RVA `0x3ad358` copies each frozen power with
/// `PowerCmd.ModifyAmount` (IL_02bd) or `PowerCmd.Apply` (IL_035b); both return
/// at `IsEnding` (`<ModifyAmount>d__6` `0x3f032c` IL_003a-003f, `<Apply>d__1`1`
/// `0x3ef988` IL_0025-002a). Knockdown's writer still reads `history.over`, so
/// the walk tests the shared IsEnding projection (#3515).
#[allow(clippy::too_many_arguments)]
fn misery_inner(
    state: &mut crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    source_uid: u32,
    target: Option<usize>,
    selection: Option<u32>,
    x_value: i64,
    args: &[CompiledArg],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let damage = match (spec.identity.id, spec.identity.upgrade, args) {
        (CardId::Misery, 0, [CompiledArg::I(7)]) => 7,
        (CardId::Misery, 1, [CompiledArg::I(9)]) => 9,
        _ => return Err(EngineRefusal::MalformedArgs("misery_exact")),
    };
    if selection.is_some() || x_value != 0 || !misery_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs("misery_exact program"));
    }
    let target = target.ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let ctx = StepCtx {
        state,
        catalog,
        spec,
        source_uid,
        target: Some(target),
        selection,
        x_value,
        args,
        events,
    };
    exact_active_source(&ctx, CardId::Misery)?;
    let frozen_target = crate::engine::play::active_card_target_identity(source_uid)
        .ok_or(EngineRefusal::ContinuationNotModeled)?
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ));
    };
    if (monster.slot, monster.uid) != frozen_target {
        return Err(EngineRefusal::ContinuationNotModeled);
    }

    let original_target_uid = monster.uid;
    // #3311: the dictionary is built here, before the attack (IL_003e-
    // IL_009c), but it is read only inside the per-recipient walk (IL_0236),
    // and building it touches no combat state (see
    // `damage::misery_roster_can_present_a_recipient`). A source whose Type-2
    // contents the engine cannot represent therefore refuses only once a
    // recipient exists to observe them.
    let snapshot = crate::engine::damage::misery_snapshot(monster);
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;

    let recipients = ctx
        .state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0 && monster.uid != original_target_uid)
        .map(|monster| (monster.slot, monster.uid))
        .collect::<Vec<_>>();
    if recipients.is_empty() {
        return Ok(());
    }
    let snapshot = snapshot?;
    for (slot, uid) in recipients {
        let Some(recipient) = ctx
            .state
            .monsters
            .iter()
            .position(|monster| monster.hp > 0 && (monster.slot, monster.uid) == (slot, uid))
        else {
            continue;
        };
        crate::engine::damage::misery_snapshot(&ctx.state.monsters[recipient])?;
        // A cloned attachment lands only where its listener is representable.
        // Admission refuses a roster that cannot satisfy this before an action
        // can mutate; this is the belt-and-braces for a monster that joined
        // after the root was admitted.
        //
        // `StrengthPower` has no owner-side listener at all — it declares
        // four members and none of them is a hook (`0xa8943`, `0xa8946`,
        // `0xa8949`, `ModifyDamageAdditive` `0xa894c`) — so only the
        // Imbalanced family constrains the recipient set.
        if snapshot.iter().any(|entry| {
            matches!(
                entry,
                MiserySnapshotEntry::Attachment(record)
                    if record.power == crate::hot::AttachedPowerModel::Imbalanced
            )
        }) && !crate::engine::monsters::imbalanced_clone_recipient_is_representable(
            ctx.catalog,
            &ctx.state.monsters[recipient],
        ) {
            return Err(EngineRefusal::MalformedArgs("Imbalanced clone recipient"));
        }
        for frozen in &snapshot {
            if crate::engine::damage::damage_combat_is_ending(ctx.state) {
                break;
            }
            let Some(recipient) =
                ctx.state.monsters.iter().position(|monster| {
                    monster.hp > 0 && (monster.slot, monster.uid) == (slot, uid)
                })
            else {
                break;
            };
            // `0x3ad358` IL_0268-IL_026f: an entry whose frozen value is zero
            // is skipped before the stacking lookup, so it applies nothing
            // and consumes no recipient Artifact. Since #2693 S3 that is a
            // reachable state rather than a formality: the temporary-Strength
            // fold can land the `StrengthPower` entry on exactly zero, which
            // is what a source whose whole negative Strength came from one
            // wrapper looks like.
            if crate::engine::damage::misery_snapshot_entry_amount(frozen) == 0 {
                continue;
            }
            let frozen = match frozen {
                MiserySnapshotEntry::Attachment(record) => {
                    match record.power {
                        crate::hot::AttachedPowerModel::Imbalanced => {
                            crate::engine::damage::apply_card_monster_imbalanced_clone(
                                ctx.state,
                                ctx.catalog,
                                recipient,
                                record.applier,
                                record.amount,
                                ctx.events,
                            )?;
                        }
                        // #2693 S1. The frozen value is copied as a signed
                        // delta without re-filtering by sign (`0x3ad358`
                        // IL_0268-IL_026f only skips a zero), and the
                        // recipient's singleton instance absorbs it whether
                        // it exists (IL_029b `ModifyAmount`) or not
                        // (IL_0320-IL_035b `Apply`).
                        crate::hot::AttachedPowerModel::Strength => {
                            crate::engine::damage::apply_card_monster_strength_clone(
                                ctx.state,
                                ctx.catalog,
                                recipient,
                                record.applier,
                                record.amount,
                                ctx.events,
                            )?;
                        }
                        // #2693 S3. The fold at `0x3ad358` IL_00a1-IL_0133
                        // has already moved this wrapper's amount onto the
                        // frozen `StrengthPower` value
                        // (`damage::misery_snapshot`); the wrapper itself
                        // keeps its own position and copies as a real
                        // instance, through its own Artifact gate.
                        model if model.is_temporary_strength_wrapper() => {
                            crate::engine::damage::apply_card_monster_temp_strength_wrapper_clone(
                                ctx.state,
                                ctx.catalog,
                                recipient,
                                model,
                                record.applier,
                                record.amount,
                                ctx.events,
                            )?;
                        }
                        // Unreachable by the guard above; spelled so a new
                        // non-wrapper variant is still a compile error here.
                        crate::hot::AttachedPowerModel::CrushUnder
                        | crate::hot::AttachedPowerModel::DarkShackles
                        | crate::hot::AttachedPowerModel::DyingStar
                        | crate::hot::AttachedPowerModel::EnfeeblingTouch
                        | crate::hot::AttachedPowerModel::Mangle
                        | crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown
                        | crate::hot::AttachedPowerModel::PiercingWail
                        | crate::hot::AttachedPowerModel::ShacklingPotion => {
                            return Err(EngineRefusal::MalformedArgs(
                                "Misery temporary Strength wrapper fold",
                            ));
                        }
                    }
                    continue;
                }
                MiserySnapshotEntry::Scalar(frozen) => frozen,
            };
            if frozen.power == PowerId::Knockdown {
                crate::engine::damage::apply_card_knockdown(
                    ctx.state,
                    recipient,
                    frozen.amount,
                    ctx.events,
                )?;
            } else {
                crate::engine::damage::apply_card_monster_debuff_with_catalog(
                    ctx.state,
                    ctx.catalog,
                    recipient,
                    frozen.power,
                    frozen.token,
                    frozen.amount,
                    ctx.events,
                )?;
            }
            if frozen.token == MiseryToken::Hang {
                let amount = ctx.state.monsters[recipient]
                    .powers
                    .value(PowerId::Hang)
                    .min(999_999_999);
                ctx.state.monsters_mut()[recipient].powers.set(
                    PowerId::Hang,
                    crate::powers::SlotWire::Int,
                    amount,
                );
            }
        }
    }
    Ok(())
}

fn misery_program_is_exact(
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
) -> bool {
    crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) == Some(spec.row)
        && matches!(catalog.steps(spec), [step]
            if step.kind == StepKind::MiseryExact
                && catalog.args(step.args)
                    == [CompiledArg::I(if spec.identity.upgrade == 0 { 7 } else { 9 })])
}

/// Shared Fate's owner-first pair of signed Strength commands.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) awaits owner Strength -2 before applying
/// -2/-3 to the still-live selected enemy through the card-debuff path.
///
/// `SharedFate/<OnPlay>d__9` RVA `0x3baaec` awaits `Apply<StrengthPower>` on the
/// owner (IL_00f3) and then on the target (IL_0186). `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn shared_fate_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (owner, enemy) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::SharedFate, 0, [CompiledArg::I(-2), CompiledArg::I(-2)]) => (-2, -2),
        (CardId::SharedFate, 1, [CompiledArg::I(-2), CompiledArg::I(-3)]) => (-2, -3),
        _ => return Err(EngineRefusal::MalformedArgs("shared_fate_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if ctx.state.monsters.get(target).is_none() {
        return Err(EngineRefusal::TargetMismatch { required: true });
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    apply_owner_strength(ctx.state, owner, ctx.events)?;
    apply_card_monster_strength_delta(ctx.state, target, enemy, ctx.events)
}

/// The Scythe's live physical damage followed by unconditional local growth.
///
/// Python: `_the_scythe_exact` (frozen, deleted #2827) attacks with `13 + damage_growth`, skips
/// the tail only when retaliation kills the player, then re-resolves the live
/// source and adds 5/7 according to its post-await level.
///
/// The `DeckVersion` half (`TheScythe/<OnPlay>d__15::MoveNext` RVA
/// `0x3c2ed0`: the same `Increase` local, read at IL_00d3-IL_00e8, is
/// `BuffFromPlay`ed onto the live copy at IL_00eb and onto `DeckVersion` at
/// IL_00f0-IL_0102 when it `isinst TheScythe`) needs no write in an ordinary
/// fight (#2941): the master growth is read from the linked live copy
/// (`CardStates::scythe_deck_links`), so growing the live copy grows it. A
/// Thieving Hopper fight keeps an explicit master row, updated through
/// [`crate::engine::monsters::update_hopper_scythe_master`].
pub(crate) fn the_scythe_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let base_increase = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::TheScythe, 0, [CompiledArg::I(13), CompiledArg::I(5)]) => 5,
        (CardId::TheScythe, 1, [CompiledArg::I(13), CompiledArg::I(7)]) => 7,
        _ => return Err(EngineRefusal::MalformedArgs("the_scythe_exact")),
    };
    let source = exact_active_source(ctx, CardId::TheScythe)?;
    let growth = ctx.state.card_states.get(source.uid).damage_growth;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if ctx.state.monsters.get(target).is_none() {
        return Err(EngineRefusal::TargetMismatch { required: true });
    }
    growth
        .checked_add(base_increase)
        .ok_or(EngineRefusal::CounterOverflow("the scythe damage growth"))?;
    if ctx.state.card_states.hopper().is_some() {
        let before = ctx.state.card_states.get(source.uid);
        let mut after = before.clone();
        after.damage_growth = after
            .damage_growth
            .checked_add(base_increase)
            .ok_or(EngineRefusal::CounterOverflow("the scythe damage growth"))?;
        let mut probe = ctx.state.clone();
        crate::engine::monsters::update_hopper_scythe_master(
            &mut probe, source.uid, &before, &after,
        )?;
    }
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        13_i64
            .checked_add(i64::from(growth))
            .ok_or(EngineRefusal::CounterOverflow("the scythe damage"))?,
        1,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(());
    }
    let source = exact_active_source(ctx, CardId::TheScythe)?;
    let live_upgrade = ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity
        .upgrade;
    let increase = match live_upgrade {
        0 => 5,
        1 => 7,
        _ => return Err(EngineRefusal::MalformedArgs("the_scythe_exact")),
    };
    ctx.state
        .card_states
        .get(source.uid)
        .damage_growth
        .checked_add(increase)
        .ok_or(EngineRefusal::CounterOverflow("the scythe damage growth"))?;
    let before = ctx.state.card_states.get(source.uid);
    let mut after = before.clone();
    after.damage_growth = after
        .damage_growth
        .checked_add(increase)
        .ok_or(EngineRefusal::CounterOverflow("the scythe damage growth"))?;
    crate::engine::monsters::update_hopper_scythe_master(ctx.state, source.uid, &before, &after)?;
    for pile in PileId::ALL {
        if let Some(card) = ctx
            .state
            .piles
            .get_mut(pile)
            .make_mut()
            .iter_mut()
            .find(|card| card.uid == source.uid)
        {
            card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            break;
        }
    }
    ctx.state.card_states.set(source.uid, after);
    Ok(())
}

/// `undeath_exact` — powered Block followed by one exact live-source clone.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Undeath/<OnPlay>d__5::MoveNext` RVA `0x3c5408` awaits powered GainBlock at
/// IL `0x0041`, then calls CreateClone on the retained live card at IL
/// `0x009f-0x00a0` and AddGeneratedCardToCombat(Discard, Bottom) at IL
/// `0x00a6-0x00af`. Python `_run_steps_inner` (frozen, deleted #2827) re-resolves that live
/// physical source around Block, then `add_generated_card_clones` records
/// CardGenerated before its combat-ending insertion gate.
///
/// The shared clone transaction has that native record-before-ending order.
/// Enchantments and legacy physical rows are independently admission-refused,
/// so the admitted unenchanted surface never needs to reconstruct GlamUsed
/// from noncanonical play state here.
pub(crate) fn undeath_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let mut next = ctx.state.clone();
    let mut emitted = Vec::new();
    let mut inner = StepCtx {
        state: &mut next,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target: ctx.target,
        selection: ctx.selection,
        x_value: ctx.x_value,
        args: ctx.args,
        events: &mut emitted,
    };
    undeath_inner(&mut inner)?;
    *ctx.state = next;
    ctx.events.extend(emitted);
    Ok(())
}

fn undeath_inner(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let block = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Undeath, 0, [CompiledArg::I(7)]) => 7,
        (CardId::Undeath, 1, [CompiledArg::I(9)]) => 9,
        _ => return Err(EngineRefusal::MalformedArgs("undeath_exact")),
    };
    if !undeath_program_is_exact(ctx.catalog, ctx.catalog.steps(ctx.spec), ctx.args)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("undeath_exact program"));
    }
    if ctx.target.is_some() || ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("undeath_exact operands"));
    }
    let source = exact_active_source(ctx, CardId::Undeath)?;
    if source.flags & CARD_FLAG_LEGACY != 0
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "undeath_exact physical source",
        ));
    }
    if ctx.state.history.over {
        return Ok(());
    }

    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, block, ctx.events)?;
    let source = exact_active_source(ctx, CardId::Undeath)?;
    if source.flags & CARD_FLAG_LEGACY != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "undeath_exact physical source",
        ));
    }
    inject_generated_clones_bottom(
        ctx.state,
        ctx.catalog,
        source,
        1,
        PileId::Discard,
        ctx.events,
    )
}

fn undeath_program_is_exact(
    catalog: &crate::catalog::Catalog,
    steps: &[crate::catalog::CompiledStep],
    args: &[CompiledArg],
) -> bool {
    matches!(steps, [step]
        if step.kind == StepKind::UndeathExact && catalog.args(step.args) == args)
}

/// Authenticate Undeath before the shared play prefix spends resources,
/// moves the source, or publishes CardPlayed.
///
/// The body repeats this proof after the source reaches its live execution
/// pile. Late listener/generated-card failures remain covered by the one-card
/// transaction in `play_card_with_work`.
pub(crate) fn preflight_undeath(
    state: &crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    source_uid: u32,
    spec: &crate::catalog::CardSpec,
) -> Result<(), EngineRefusal> {
    let args = match spec.identity.upgrade {
        0 => [CompiledArg::I(7)],
        1 => [CompiledArg::I(9)],
        _ => return Err(EngineRefusal::MalformedArgs("undeath_exact")),
    };
    if spec.identity.id != CardId::Undeath
        || !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
        || !undeath_program_is_exact(catalog, catalog.steps(spec), &args)
    {
        return Err(EngineRefusal::MalformedArgs("undeath_exact program"));
    }
    let mut sources = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(|card| card.uid == source_uid)
    });
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let additional = sources.count();
    if additional != 0 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 1 + additional,
        });
    }
    if source.flags & CARD_FLAG_LEGACY != 0 || catalog.spec(source.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs(
            "undeath_exact physical source",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::Event;
    use crate::hot::{
        CardInstanceState, HopperDeckRow, HopperDeckState, HotMonster, HotState,
        MultiplayerAllyState, RngStream, RngStreamState,
    };
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;

    const OWNED: [StepKind; 8] = [
        StepKind::CallOfTheVoid,
        StepKind::EndOfDaysExact,
        StepKind::GlimpseBeyondExact,
        StepKind::HangExact,
        StepKind::MiseryExact,
        StepKind::SharedFateExact,
        StepKind::TheScytheExact,
        StepKind::UndeathExact,
    ];

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn fixture(id: CardId, upgrade: u8, hp: i32) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(id, upgrade)).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: if id == CardId::TheScythe {
                CARD_FLAG_DEFAULT_PHYSICAL_STATE
            } else {
                0
            },
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
        monster.uid = 41;
        state.monsters_mut().push(monster);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source)
    }

    #[test]
    fn family_manifest_and_all_sixteen_generated_carriers_are_exact() {
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::CallOfTheVoid,
                StepKind::EndOfDaysExact,
                StepKind::GlimpseBeyondExact,
                StepKind::HangExact,
                StepKind::MiseryExact,
                StepKind::SharedFateExact,
                StepKind::TheScytheExact,
                StepKind::UndeathExact,
            ]
        );
        let carriers = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| OWNED.contains(&step.kind)))
            .flat_map(|row| {
                row.steps.iter().filter_map(move |step| {
                    OWNED.contains(&step.kind).then_some((
                        row.id,
                        row.upgrade,
                        step.kind,
                        step.args,
                    ))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::CallOfTheVoid,
                    0,
                    StepKind::CallOfTheVoid,
                    &[Arg::I(1)][..]
                ),
                (
                    CardId::CallOfTheVoid,
                    1,
                    StepKind::CallOfTheVoid,
                    &[Arg::I(1)][..]
                ),
                (
                    CardId::EndOfDays,
                    0,
                    StepKind::EndOfDaysExact,
                    &[Arg::I(29)][..]
                ),
                (
                    CardId::EndOfDays,
                    1,
                    StepKind::EndOfDaysExact,
                    &[Arg::I(37)][..]
                ),
                (
                    CardId::GlimpseBeyond,
                    0,
                    StepKind::GlimpseBeyondExact,
                    &[Arg::I(3)][..]
                ),
                (
                    CardId::GlimpseBeyond,
                    1,
                    StepKind::GlimpseBeyondExact,
                    &[Arg::I(4)][..]
                ),
                (CardId::Hang, 0, StepKind::HangExact, &[Arg::I(10)][..]),
                (CardId::Hang, 1, StepKind::HangExact, &[Arg::I(13)][..]),
                (CardId::Misery, 0, StepKind::MiseryExact, &[Arg::I(7)][..]),
                (CardId::Misery, 1, StepKind::MiseryExact, &[Arg::I(9)][..]),
                (
                    CardId::SharedFate,
                    0,
                    StepKind::SharedFateExact,
                    &[Arg::I(-2), Arg::I(-2)][..]
                ),
                (
                    CardId::SharedFate,
                    1,
                    StepKind::SharedFateExact,
                    &[Arg::I(-2), Arg::I(-3)][..]
                ),
                (
                    CardId::TheScythe,
                    0,
                    StepKind::TheScytheExact,
                    &[Arg::I(13), Arg::I(5)][..]
                ),
                (
                    CardId::TheScythe,
                    1,
                    StepKind::TheScytheExact,
                    &[Arg::I(13), Arg::I(7)][..]
                ),
                (CardId::Undeath, 0, StepKind::UndeathExact, &[Arg::I(7)][..]),
                (CardId::Undeath, 1, StepKind::UndeathExact, &[Arg::I(9)][..]),
            ]
        );
        let admitted = carriers
            .iter()
            .filter(|(_, _, kind, _)| IMPLEMENTED.contains(kind))
            .map(|(id, upgrade, _, _)| (*id, *upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![
                (CardId::CallOfTheVoid, 0),
                (CardId::CallOfTheVoid, 1),
                (CardId::EndOfDays, 0),
                (CardId::EndOfDays, 1),
                (CardId::GlimpseBeyond, 0),
                (CardId::GlimpseBeyond, 1),
                (CardId::Hang, 0),
                (CardId::Hang, 1),
                (CardId::Misery, 0),
                (CardId::Misery, 1),
                (CardId::SharedFate, 0),
                (CardId::SharedFate, 1),
                (CardId::TheScythe, 0),
                (CardId::TheScythe, 1),
                (CardId::Undeath, 0),
                (CardId::Undeath, 1),
            ],
            "the admitted family delta is exactly sixteen rows"
        );
        let manifest = crate::engine::capability_manifest();
        for kind in OWNED {
            assert_eq!(
                manifest.steps.contains(&kind),
                IMPLEMENTED.contains(&kind),
                "family refusal sensitivity drift for {kind:?}"
            );
        }
        let represented_misery = crate::engine::damage::MISERY_SCALAR_POWERS
            .into_iter()
            .filter(|(power, _)| crate::engine::admission::IMPLEMENTED_POWERS.contains(power))
            .collect::<Vec<_>>();
        assert_eq!(
            represented_misery,
            [
                (PowerId::Weak, MiseryToken::Weak),
                (PowerId::Vuln, MiseryToken::Vuln),
                (PowerId::Hang, MiseryToken::Hang),
                (PowerId::Poison, MiseryToken::Poison),
                (PowerId::Doom, MiseryToken::Doom),
                (PowerId::Oblivion, MiseryToken::Oblivion),
                (PowerId::Strangle, MiseryToken::Strangle),
                (PowerId::Demise, MiseryToken::Demise),
                (PowerId::Shrink, MiseryToken::Shrink),
                (PowerId::Conqueror, MiseryToken::Conqueror),
                (PowerId::Debilitate, MiseryToken::Debilitate),
                (PowerId::SicEm, MiseryToken::SicEm),
            ],
            "Misery's scalar clone vocabulary is derived from the monster-power registry"
        );
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::GlimpseBeyondExact),
            "the public manifest must include the exact shared Glimpse gate"
        );
    }

    #[test]
    fn call_of_the_void_pool_order_and_source_digest_are_frozen() {
        let pool = call_of_the_void_pool();
        assert_eq!(pool, crate::content_tables::CALL_OF_THE_VOID_POOL_V1101);
        let mut digest = 0xcbf2_9ce4_8422_2325_u64;
        for id in pool {
            for byte in id.as_str().bytes().chain(std::iter::once(0)) {
                digest ^= u64::from(byte);
                digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        assert_eq!(digest, 0x91cd_aef9_1e97_fb03);
    }

    #[test]
    fn private_glimpse_body_authenticates_source_and_generates_both_player_batches() {
        let glimpse = identity(CardId::GlimpseBeyond, 1);
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(glimpse).unwrap();
        builder.intern(soul).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom: source_atom,
            flags: 0,
        };
        let spec = catalog.spec(source_atom).unwrap();
        let [step] = catalog.steps(spec) else {
            panic!("canonical Glimpse row has one step")
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        apply_glimpse_beyond_foundation(&mut ctx).unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 8);
        assert_eq!(state.next_generated_hook_uid, 8);
        assert_eq!(state.next_card_uid, 4);
        assert_eq!(state.piles.get(PileId::Draw).len(), 4);
        assert_eq!(state.fanouts.multiplayer_ally().draw.len(), 4);
        assert_eq!(
            state
                .fanouts
                .multiplayer_ally()
                .owner_generated_cards_combat,
            0
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
    }

    #[test]
    fn private_glimpse_body_refuses_forged_program_before_any_mutation() {
        let (mut state, catalog, source) = fixture(CardId::GlimpseBeyond, 0, 100);
        let spec = catalog.spec(source.atom).unwrap();
        let before = state.clone();
        let mut events = vec![Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        }];
        let events_before = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(4)],
            events: &mut events,
        };

        assert_eq!(
            apply_glimpse_beyond_foundation(&mut ctx),
            Err(EngineRefusal::MalformedArgs("glimpse_beyond_exact"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn private_glimpse_body_preserves_every_exact_source_pile_and_rejects_duplicates() {
        let glimpse = identity(CardId::GlimpseBeyond, 0);
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(glimpse).unwrap();
        builder.intern(soul).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(source_atom).unwrap();
        let [step] = catalog.steps(spec) else {
            panic!("canonical Glimpse row has one step")
        };
        let source = HotCard {
            uid: 17,
            atom: source_atom,
            flags: 0,
        };

        for pile in PileId::ALL {
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.rng.set(
                RngStream::Rng,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(pile).make_mut().push(source);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };

            apply_glimpse_beyond_foundation(&mut ctx).unwrap();

            let locations = PileId::ALL
                .into_iter()
                .flat_map(|candidate| {
                    state
                        .piles
                        .get(candidate)
                        .as_slice()
                        .iter()
                        .copied()
                        .filter(move |card| card.uid == source.uid)
                        .map(move |card| (candidate, card))
                })
                .collect::<Vec<_>>();
            assert_eq!(locations, vec![(pile, source)], "{pile:?}");
            assert_eq!(state.history.owner_generated_cards_combat, 3);
        }

        let mut duplicate = HotState::at_defaults();
        duplicate.hp = 80;
        duplicate.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        duplicate
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut duplicate,
            catalog: &catalog,
            spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        assert_eq!(
            apply_glimpse_beyond_foundation(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(duplicate, before);
        assert!(events.is_empty());
    }

    #[test]
    fn public_glimpse_dispatch_runs_the_exact_generated_soul_body() {
        let (mut state, catalog, source) = fixture(CardId::GlimpseBeyond, 0, 100);
        let spec = catalog.spec(source.atom).unwrap();
        let [step] = catalog.steps(spec) else {
            panic!("canonical Glimpse row has one step")
        };
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        glimpse_beyond_exact(&mut ctx).unwrap();
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(events.len(), 3);
    }

    fn run_end_of_days(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        let args = if upgrade == 0 {
            [CompiledArg::I(29)]
        } else {
            [CompiledArg::I(37)]
        };
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events,
        };
        end_of_days_exact(&mut ctx)
    }

    fn run_undeath(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        let args = if upgrade == 0 {
            [CompiledArg::I(7)]
        } else {
            [CompiledArg::I(9)]
        };
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events,
        };
        undeath_exact(&mut ctx)
    }

    fn play_misery(
        state: &HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
    ) -> Result<(HotState, Vec<Event>), EngineRefusal> {
        let mut events = Vec::new();
        let state = crate::engine::apply_action_into(
            state,
            catalog,
            &crate::engine::Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: crate::engine::SelectionRef::NONE,
            },
            &mut events,
        )?;
        Ok((state, events))
    }

    fn misery_fixture(upgrade: u8, target_hp: i32) -> (HotState, crate::catalog::Catalog, HotCard) {
        let (mut state, catalog, source) = fixture(CardId::Misery, upgrade, target_hp);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        (state, catalog, source)
    }

    /// A `build_bowlbugs_weak`-shaped source: a Rock carrying the intrinsic
    /// `ImbalancedPower`, plus one generic-stun-eligible recipient.
    fn bowlbug_misery_fixture(
        recipient_kind: MonsterKind,
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        bowlbug_misery_fixture_with(MonsterKind::BowlbugRock, recipient_kind, &[])
    }

    /// The catalog must compile every roster kind's moves:
    /// `generic_stun_owner_state_is_representable` checks that
    /// `catalog.moves(kind)` agrees in length with the generated loop, which
    /// is what lets `loop_pos` index both.
    fn bowlbug_misery_fixture_with(
        source_kind: MonsterKind,
        recipient_kind: MonsterKind,
        extra: &[(MonsterKind, u32, i32)],
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::Misery, 0)).unwrap();
        for kind in [source_kind, recipient_kind]
            .into_iter()
            .chain(extra.iter().map(|(kind, _, _)| *kind))
        {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        let mut monster = HotMonster::new(source_kind, 100);
        monster.uid = 41;
        state.monsters_mut().push(monster);
        let mut recipient = HotMonster::new(recipient_kind, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        for (kind, uid, slot) in extra {
            let mut other = HotMonster::new(*kind, 100);
            other.uid = *uid;
            other.slot = *slot;
            state.monsters_mut().push(other);
        }
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        (state, catalog, source)
    }

    fn imbalanced_row(applier_uid: u32, amount: i32) -> crate::hot::AttachmentRecord {
        crate::hot::AttachmentRecord {
            power: crate::hot::AttachedPowerModel::Imbalanced,
            applier: crate::hot::Applier::Monster(applier_uid),
            amount,
        }
    }

    /// #2647 B2 — the clone lands on every eligible recipient, amount 1,
    /// applier the Rock.
    ///
    /// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_01ed takes
    /// `CombatState::HittableEnemies` after the attack and IL_0220-IL_0231
    /// skips only the original target, so both other monsters receive the
    /// copy. The applier is `entry.Key.Applier` (IL_0354), which
    /// `PowerModel::AfterCloned` `0x84093` preserved from the Rock's own
    /// `Apply` at `0x353e30` IL_007f-IL_0097.
    #[test]
    fn misery_clones_imbalanced_onto_every_eligible_recipient() {
        let (state, catalog, source) = bowlbug_misery_fixture_with(
            MonsterKind::BowlbugRock,
            MonsterKind::BowlbugEgg,
            &[(MonsterKind::BowlbugNectar, 43, 2)],
        );

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[0].hp, 93);
        for recipient in [1, 2] {
            assert_eq!(
                next.monsters[recipient].misery_debuff_order.attachments(),
                [imbalanced_row(41, 1)],
                "recipient {recipient}",
            );
            assert!(crate::engine::monsters::owner_carries_imbalanced(
                &next.monsters[recipient]
            ));
        }
        // The source keeps its by-kind representation and grows no row.
        assert!(
            next.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
    }

    /// #2647 B2 — Artifact blocks the clone and is consumed.
    ///
    /// `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` zeroes a
    /// delta whose `GetTypeForAmount` is 2 on a visible power owned by the
    /// Artifact holder. `ImbalancedPower` is Type 2 (`0xa3bd3`) with the base
    /// `AllowNegative` false (`0x83a7d`), so `GetTypeForAmount` `0x83a94`
    /// falls through at IL_0072; and `get_IsVisible` `0x83754` is true
    /// because `_target` is never written for this family. With the delta
    /// zeroed, `PowerModel::ApplyInternal` `0x84012` IL_0001-IL_000e returns
    /// before attaching.
    #[test]
    fn misery_clone_of_imbalanced_is_blocked_by_artifact_and_consumes_one() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 2);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 1);
        assert!(
            next.monsters[1]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
        assert!(!crate::engine::monsters::owner_carries_imbalanced(
            &next.monsters[1]
        ));
    }

    /// #2647 B2 — a recipient that already carries one stacks to 2 in place.
    ///
    /// `PowerModel::get_InstanceType` `0x83751` is 0 for this family, so
    /// `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058 finds
    /// the instance by id and `0x3ad358` IL_029b-IL_02bd routes to
    /// `PowerCmd::ModifyAmount` `0x3f032c`, whose `PowerModel::SetAmount`
    /// `0x83f8c` writes `_amount` without touching `Creature::_powers`. So
    /// the amount moves and the ledger position does not, and the applier
    /// stays the one the instance was attached with — `ModifyAmount` never
    /// reaches `set_Applier`.
    #[test]
    fn misery_clone_onto_an_existing_imbalanced_stacks_without_a_new_position() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        state.monsters_mut()[1]
            .misery_debuff_order
            .push_attachment(imbalanced_row(41, 1));

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(
            next.monsters[1].misery_debuff_order.attachments(),
            [imbalanced_row(41, 2)],
        );
        assert_eq!(
            next.monsters[1].misery_debuff_order.attachments().len(),
            1,
            "stacking creates no second position",
        );
        // Rend still counts one native instance.
        assert_eq!(
            crate::steps::neutral::rend_target_power_count(&next.monsters[1]),
            1,
        );
    }

    /// #2647 B2 — a clone onto the by-kind Rock records the stacked amount.
    ///
    /// The Rock is an eligible recipient: `0x33cd60` IL_0056's `isinst`
    /// succeeds for it, so the listener sets the latch #2662 models rather
    /// than installing a generic stun. What the kind cannot imply is the
    /// amount once `ModifyAmount` has moved it off 1, so the instance grows
    /// the row — at ledger position 0, where the intrinsic instance sits.
    #[test]
    fn misery_clone_onto_a_by_kind_rock_records_the_stacked_amount() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        // Target the Egg, which already carries a clone, so the Rock is the
        // recipient.
        state.monsters_mut()[1]
            .misery_debuff_order
            .push_attachment(imbalanced_row(41, 1));
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[1]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        let mut events = Vec::new();
        let next = crate::engine::apply_action_into(
            &state,
            &catalog,
            &crate::engine::Action::Play {
                uid: source.uid,
                target: Some(1),
                selection: crate::engine::SelectionRef::NONE,
            },
            &mut events,
        )
        .unwrap();

        assert_eq!(
            next.monsters[0].misery_debuff_order.attachments(),
            [imbalanced_row(41, 2)],
            "the Rock's own applier survives the stack",
        );
        assert_eq!(
            crate::engine::monsters::imbalanced_amount(&next.monsters[0]),
            Some(2),
        );
        // The attachment leads the Rock's walk; the cloned Weak follows it.
        assert_eq!(next.monsters[0].powers.value(PowerId::Weak), 2);
        assert!(matches!(
            crate::engine::damage::misery_snapshot(&next.monsters[0]).as_deref(),
            Ok([
                crate::engine::damage::MiserySnapshotEntry::Attachment(_),
                crate::engine::damage::MiserySnapshotEntry::Scalar(_),
            ]),
        ));
    }

    /// #2647 B2 — the clone is interleaved with the scalars by ledger
    /// position, not applied in a second pass.
    ///
    /// The recipient's single Artifact is the instrument: it consumes on
    /// whichever entry the walk reaches first, so attach-then-acquire and
    /// acquire-then-attach end in observably different states. That is the
    /// whole reason `0x3ad358` IL_003e-IL_009c's single ordered walk is
    /// modeled as one sequence.
    #[test]
    fn misery_clone_ordering_interleaves_attachments_and_scalars_by_position() {
        let mut outcomes = Vec::new();
        for attachment_first in [true, false] {
            // The Rock's intrinsic Imbalanced always leads, so put the
            // interleaving on the *recipient*-visible side by giving the Egg
            // the source role instead.
            let (mut state, catalog, source) = bowlbug_misery_fixture_with(
                MonsterKind::BowlbugEgg,
                MonsterKind::BowlbugNectar,
                &[],
            );
            state.monsters_mut()[1]
                .powers
                .set(PowerId::Artifact, SlotWire::Int, 1);
            if attachment_first {
                state.monsters_mut()[0]
                    .misery_debuff_order
                    .push_attachment(imbalanced_row(41, 1));
            }
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Weak, SlotWire::Int, 2);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(MiseryToken::Weak);
            if !attachment_first {
                state.monsters_mut()[0]
                    .misery_debuff_order
                    .push_attachment(imbalanced_row(41, 1));
            }

            let (next, _) = play_misery(&state, &catalog, source).unwrap();
            outcomes.push((
                next.monsters[1].powers.value(PowerId::Weak),
                next.monsters[1].misery_debuff_order.attachments().len(),
                next.monsters[1].powers.value(PowerId::Artifact),
            ));
        }
        // Attachment first: Artifact eats the Imbalanced, the Weak lands.
        assert_eq!(outcomes[0], (2, 0, 0));
        // Weak first: Artifact eats the Weak, the Imbalanced lands.
        assert_eq!(outcomes[1], (0, 1, 0));
        assert_ne!(outcomes[0], outcomes[1]);
    }

    /// #3404 (#2647) — an unlatched Unsettling Lamp neither latches on nor
    /// doubles a monster-applied Imbalanced clone.
    ///
    /// `UnsettlingLamp::BeforePowerAmountChanged` `0x9d4fc` IL_0032-IL_003f
    /// latches only for `applier == Owner.Player.Creature`, and
    /// `ModifyPowerAmountGivenMultiplicative` `0x9d5a8` IL_000c-IL_0012
    /// returns 1 while `TriggeringCard` is unset. So the clone lands at 1 and
    /// the Lamp is still available after the play.
    #[test]
    fn misery_imbalanced_clone_neither_latches_nor_meets_an_unlatched_lamp() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        state.fanouts.set_unsettling_lamp_available(true);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(
            next.monsters[1].misery_debuff_order.attachments(),
            [imbalanced_row(41, 1)],
        );
        assert!(next.fanouts.unsettling_lamp_available());
    }

    /// #3404 (#2647) — a Lamp latched on this Misery play doubles the
    /// monster-applied Imbalanced clone exactly while its applier is still in
    /// combat.
    ///
    /// The Egg source carries a player-applied Weak ahead of an Imbalanced
    /// row whose applier is a third monster, the Rock. The Weak clone latches
    /// the Lamp on the Misery card (player applier, Type 2). The Imbalanced
    /// clone then reaches `0x9d5a8` IL_0057 (x2) only through
    /// `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_01d3-IL_01ec's
    /// `ContainsCreature(applier)` gate: a living Rock is in
    /// `CombatState._enemies` (`CombatState::ContainsCreature` `0x137286`), a
    /// dead one was evicted by `KillWithoutCheckingWinCondition` `0x3ebe90`
    /// IL_04d5. The Weak is doubled either way, so the difference is the gate.
    #[test]
    fn misery_imbalanced_clone_under_a_latched_lamp_doubles_only_with_its_applier_in_combat() {
        let mut outcomes = Vec::new();
        for applier_alive in [true, false] {
            let (mut state, catalog, source) = bowlbug_misery_fixture_with(
                MonsterKind::BowlbugEgg,
                MonsterKind::BowlbugNectar,
                &[(MonsterKind::BowlbugRock, 43, 2)],
            );
            state.fanouts.set_unsettling_lamp_available(true);
            if !applier_alive {
                state.monsters_mut()[2].hp = 0;
            }
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Weak, SlotWire::Int, 2);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(MiseryToken::Weak);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push_attachment(imbalanced_row(43, 1));

            let (next, _) = play_misery(&state, &catalog, source).unwrap();

            assert_eq!(next.monsters[1].powers.value(PowerId::Weak), 4);
            assert!(!next.fanouts.unsettling_lamp_available());
            outcomes.push(
                next.monsters[1]
                    .misery_debuff_order
                    .attachments()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(outcomes[0], [imbalanced_row(43, 2)], "applier in combat");
        assert_eq!(outcomes[1], [imbalanced_row(43, 1)], "applier evicted");
    }

    /// #3404 — the doubled clone still meets the recipient's Artifact, which
    /// consumes one and blocks it (the received-side hook runs after the
    /// given-side one in `0x3efbac`).
    #[test]
    fn misery_imbalanced_clone_under_a_latched_lamp_is_still_blocked_by_artifact() {
        let (mut state, catalog, source) = bowlbug_misery_fixture_with(
            MonsterKind::BowlbugEgg,
            MonsterKind::BowlbugNectar,
            &[(MonsterKind::BowlbugRock, 43, 2)],
        );
        state.fanouts.set_unsettling_lamp_available(true);
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push_attachment(imbalanced_row(43, 1));

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 0);
        assert_eq!(next.monsters[1].powers.value(PowerId::Weak), 0);
        assert!(
            next.monsters[1]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
    }

    /// #2647 B2 — a recipient whose listener arm is not representable
    /// refuses by name rather than taking an approximate stun.
    ///
    /// Admission refuses such a roster outright (`Misery Imbalanced clone
    /// recipient`); this is the engine-side belt-and-braces for a monster
    /// that joined the roster after the root was admitted. Terror Eel is
    /// named because it is the seam: for it `STUNNED` with no follow-up is
    /// the Terror chain (#2647 §5.7).
    #[test]
    fn misery_refuses_a_clone_onto_an_unrepresentable_recipient() {
        for kind in [MonsterKind::TerrorEel, MonsterKind::Toadpole] {
            let (state, catalog, source) = bowlbug_misery_fixture(kind);
            assert_eq!(
                play_misery(&state, &catalog, source).unwrap_err(),
                EngineRefusal::MalformedArgs("Imbalanced clone recipient"),
                "{kind:?}",
            );
        }
        // A Thorns carrier is refused for #2647 §5.4's reason, re-derived per
        // representation in B1.
        let (mut thorny, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        thorny.monsters_mut()[1]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);
        assert_eq!(
            play_misery(&thorny, &catalog, source).unwrap_err(),
            EngineRefusal::MalformedArgs("Imbalanced clone recipient"),
        );
    }

    /// #2647 B2 — the clone's applier survives the Rock's own death.
    ///
    /// The dictionary is frozen at `0x3ad358` IL_003e-IL_009c, before the
    /// awaited `DamageCmd` at IL_014c, so a Rock that dies to Misery's own
    /// attack still supplies the applier its clones carry. The state the
    /// engine builds here is exactly the one B1 refused on reload.
    #[test]
    fn a_clone_from_a_lethally_struck_rock_keeps_its_dead_applier() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        state.monsters_mut()[0].hp = 5;

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert!(next.monsters[0].hp <= 0, "the Rock died to its own Misery");
        assert_eq!(
            next.monsters[1].misery_debuff_order.attachments(),
            [imbalanced_row(41, 1)],
        );
    }

    /// #2727 — the clone feeds the AfterPowerAmountChanged walk, and Sleight
    /// of Flesh does NOT fire for it, because the Rock is the applier.
    ///
    /// **This corrects #2647 B2.** That PR asserted the Sleight damage here
    /// and read `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext`
    /// `0x344dc4` IL_006e as a second owner-identity test. It is not: IL_0067
    /// loads the state machine's `applier` **field** and IL_006e loads the
    /// listener's own `PowerModel::get_Owner`, and IL_0073's `beq` compares the
    /// two references. Sleight of Flesh is player-owned, so the applier must be
    /// the player. `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_0354 hands
    /// `PowerCmd::Apply` the source instance's retained `_applier`
    /// (`PowerModel::AfterCloned` `0x84093` leaves `_applier` alone), and for
    /// this family that is always the Bowlbug Rock's own `Creature`
    /// (`0x353e30` IL_007f-IL_0097). The other four conditions do hold —
    /// `amount != 0` IL_0026, `GetTypeForAmount == 2` IL_0043,
    /// `Owner.IsEnemy` IL_0056, not an `ITemporaryPower` IL_0080 — so this is a
    /// wrong POSITIVE being corrected against the IL, not a positive being
    /// weakened.
    ///
    /// The clone still carries the catalog, which is what lets a live Dampen
    /// entry resolve instead of forcing the refusal `apply_card_knockdown` has
    /// to take; that seam is unchanged and is asserted by the walk running to
    /// completion here.
    ///
    /// The Artifact arm remains the control: a blocked clone changes no
    /// amount, so `ApplyInternal` `0x84012` IL_0001-IL_000e returns before
    /// attaching and the walk never runs at all.
    #[test]
    fn a_cloned_imbalanced_does_not_wake_sleight_of_flesh_for_its_rock_applier() {
        let (mut state, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let before = state.monsters[1].hp;

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(
            next.monsters[1].misery_debuff_order.attachments(),
            [imbalanced_row(41, 1)],
        );
        assert_eq!(
            next.monsters[1].hp, before,
            "the Rock is the applier, so Sleight of Flesh returns at IL_0075",
        );

        // Artifact blocks the clone, so no amount changed and no walk.
        let (mut blocked, catalog, source) = bowlbug_misery_fixture(MonsterKind::BowlbugEgg);
        blocked.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        blocked
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            blocked
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let before = blocked.monsters[1].hp;

        let (next, _) = play_misery(&blocked, &catalog, source).unwrap();

        assert!(
            next.monsters[1]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
        assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 0);
        assert_eq!(next.monsters[1].hp, before, "a blocked clone deals nothing");
    }

    #[test]
    fn misery_freezes_acquisition_order_and_replays_through_artifact_and_lamp() {
        let (mut state, catalog, source) = misery_fixture(0, 100);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 3);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push_knockdown(2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push_knockdown(3);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        recipient.powers.set(PowerId::Artifact, SlotWire::Int, 1);
        state.monsters_mut().push(recipient);
        state.fanouts.set_unsettling_lamp_available(true);

        let (next, events) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[0].hp, 93);
        assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 0);
        assert_eq!(next.monsters[1].powers.value(PowerId::Poison), 0);
        assert_eq!(next.monsters[1].powers.value(PowerId::Weak), 4);
        assert_eq!(next.monsters[1].misery_debuff_order.knockdown(), [4, 6]);
        assert_eq!(
            next.monsters[1].misery_debuff_order.as_slice(),
            [
                MiseryToken::Knockdown,
                MiseryToken::Weak,
                MiseryToken::Knockdown,
            ]
        );
        assert!(!next.fanouts.unsettling_lamp_available());
        assert!(events.iter().any(|event| matches!(
            event,
            Event::PowerChanged {
                subject: crate::engine::Subject::Monster(42),
                power: PowerId::Artifact,
                amount: 0,
            }
        )));
    }

    #[test]
    fn misery_knockdown_lamp_overflow_restores_the_complete_public_play() {
        let (mut state, catalog, source) = misery_fixture(0, 100);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push_knockdown(i32::MAX);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        state.fanouts.set_unsettling_lamp_available(true);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 149 }];
        let events_before = events.clone();

        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "Unsettling Lamp power amount"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn misery_excludes_original_uid_but_includes_its_lethal_stock_replacement() {
        let (mut state, catalog, source) = misery_fixture(0, 7);
        state.monsters_mut()[0].kind = MonsterKind::Axebot;
        state.monsters_mut()[0].uid = 0;
        state.monsters_mut()[0].max_hp = 80;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        let original_uid = state.monsters[0].uid;
        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_ne!(
            next.monsters[0].uid, original_uid,
            "post-Misery Axebot: {:?}",
            next.monsters[0]
        );
        assert_eq!(next.monsters[0].powers.value(PowerId::Weak), 2);
    }

    #[test]
    fn misery_rechecks_recipient_liveness_between_separately_awaited_copies() {
        let (mut state, catalog, source) = misery_fixture(0, 100);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 3);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Vuln);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 1);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].uid, 42);
        assert_eq!(next.monsters[1].hp, 0);
        assert_eq!(next.monsters[1].powers.value(PowerId::Weak), 2);
        assert_eq!(next.monsters[1].powers.value(PowerId::Vuln), 0);
    }

    /// The order-dependent case (#3311): with a recipient to receive the
    /// copies, two scalars and no acquisition order leave the copy order —
    /// and so which of them a recipient's Artifact would eat — unknowable, so
    /// the play refuses, after the attack in the probe and before anything in
    /// the real state moves.
    #[test]
    fn misery_malformed_multi_power_legacy_order_refuses_atomically() {
        let (mut state, catalog, source) = misery_fixture(1, 100);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 3);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        recipient.powers.set(PowerId::Artifact, SlotWire::Int, 1);
        state.monsters_mut().push(recipient);
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 999,
            pile: PileId::Discard,
        }];

        assert_eq!(
            crate::engine::apply_action_into(
                &state,
                &catalog,
                &crate::engine::Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: crate::engine::SelectionRef::NONE,
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #3311: with no enemy but the target, `Misery/<OnPlay>d__3::MoveNext`
    /// `0x3ad358` never opens the per-recipient walk (IL_0236), which is the
    /// only reader of its frozen dictionary, so the source's Type-2 contents
    /// are unobservable. The same unordered two-scalar source that refuses
    /// above plays exactly as an ordered one does, whichever order that is,
    /// and so does a source whose Type-2 state the engine cannot represent
    /// at all (a Terror Eel's Shriek).
    #[test]
    fn misery_without_a_recipient_never_reads_the_source_snapshot() {
        let (mut legacy, catalog, source) = misery_fixture(1, 100);
        legacy.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        legacy.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 3);
        assert_eq!(
            crate::engine::damage::misery_snapshot(&legacy.monsters[0]),
            Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order"
            ))
        );
        let (from_legacy, legacy_events) = play_misery(&legacy, &catalog, source).unwrap();
        assert!(from_legacy.monsters[0].hp < 100);
        assert_eq!(from_legacy.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(from_legacy.monsters[0].powers.value(PowerId::Vuln), 3);

        for order in [
            [MiseryToken::Weak, MiseryToken::Vuln],
            [MiseryToken::Vuln, MiseryToken::Weak],
        ] {
            let mut ordered = legacy.clone();
            for token in order {
                ordered.monsters_mut()[0].misery_debuff_order.push(token);
            }
            assert!(crate::engine::damage::misery_snapshot(&ordered.monsters[0]).is_ok());
            let (from_ordered, ordered_events) = play_misery(&ordered, &catalog, source).unwrap();
            let mut normalized = from_ordered.clone();
            normalized.monsters_mut()[0].misery_debuff_order =
                from_legacy.monsters[0].misery_debuff_order.clone();
            assert_eq!(normalized, from_legacy);
            assert_eq!(ordered_events, legacy_events);
        }

        let (mut shriek, catalog, source) = misery_fixture(0, 100);
        shriek.monsters_mut()[0]
            .powers
            .set(PowerId::Shriek, SlotWire::Int, 1);
        assert_eq!(
            crate::engine::damage::misery_snapshot(&shriek.monsters[0]),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            ))
        );
        let (next, _) = play_misery(&shriek, &catalog, source).unwrap();
        assert_eq!(next.monsters[0].hp, 93);
        assert_eq!(next.monsters[0].powers.value(PowerId::Shriek), 1);

        // The same Shriek source with a live recipient does refuse, and
        // atomically.
        let mut joined = shriek.clone();
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        joined.monsters_mut().push(recipient);
        assert_eq!(
            play_misery(&joined, &catalog, source).map(|_| ()),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            ))
        );

        // A recipient that died before the play is not hittable after the
        // attack either, so it does not make the snapshot observable.
        let mut dead = shriek.clone();
        let mut corpse = HotMonster::new(MonsterKind::Toadpole, 0);
        corpse.uid = 42;
        corpse.slot = 1;
        dead.monsters_mut().push(corpse);
        assert!(play_misery(&dead, &catalog, source).is_ok());
    }

    #[test]
    /// #3036: Misery+ Retains and does not Exhaust (`Misery::OnUpgrade` RVA
    /// `0xe5c23` IL_0018 `ldc.i4.5` -> `AddKeyword`), so after its One-Two
    /// Punch replay it resolves to Discard like the base card.
    fn misery_replay_refreezes_each_body_and_level_one_routes_to_discard() {
        let (mut state, catalog, source) = misery_fixture(1, 100);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[0].hp, 82);
        assert_eq!(next.monsters[1].powers.value(PowerId::Weak), 4);
        assert_eq!(next.history.card_plays_finished_combat, 2);
        assert_eq!(next.piles.get(PileId::Discard).as_slice(), [source]);
        assert!(next.piles.get(PileId::Exhaust).is_empty());
    }

    #[test]
    fn misery_snapshot_precedes_attack_listeners_that_change_the_source() {
        let (mut state, catalog, source) = misery_fixture(0, 100);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        state.powers.set(PowerId::Envenom, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::Envenom])
        );

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[0].powers.value(PowerId::Poison), 2);
        assert_eq!(
            next.monsters[0].misery_debuff_order.as_slice(),
            [MiseryToken::Poison]
        );
        assert_eq!(next.monsters[1].powers.value(PowerId::Poison), 0);
        assert!(next.monsters[1].misery_debuff_order.is_empty());
    }

    // ------------------------------------------------------------------
    // #2693 S3 — Misery over the temporary-Strength wrapper fold.
    // ------------------------------------------------------------------

    /// A source and one recipient, both plain Toadpoles, with the ledger
    /// upkeep gate on.
    fn fold_fixture() -> (HotState, crate::catalog::Catalog, HotCard) {
        let (mut state, catalog, source) = misery_fixture(0, 100);
        let mut recipient = HotMonster::new(MonsterKind::Toadpole, 100);
        recipient.uid = 42;
        recipient.slot = 1;
        state.monsters_mut().push(recipient);
        state.fanouts.set_misery_attachment_upkeep(true);
        (state, catalog, source)
    }

    /// Give the source `strength` intrinsic Strength, then apply each wrapper
    /// through its real writer so the rows and the scalars stay in step.
    fn give_source_wrappers(
        state: &mut HotState,
        strength: i32,
        wrappers: &[(crate::hot::AttachedPowerModel, i32)],
    ) {
        if strength != 0 {
            crate::engine::damage::write_monster_strength(
                &mut state.monsters_mut()[0],
                strength,
                crate::hot::Applier::Player,
                true,
            );
        }
        for (model, amount) in wrappers {
            crate::engine::damage::write_monster_temp_strength_wrapper(
                &mut state.monsters_mut()[0],
                *model,
                *amount,
                crate::hot::Applier::Player,
                true,
            )
            .unwrap();
        }
    }

    fn rows(monster: &HotMonster) -> Vec<(crate::hot::AttachedPowerModel, i32)> {
        monster
            .misery_debuff_order
            .attachments()
            .map(|record| (record.power, record.amount))
            .collect()
    }

    /// The issue's own witness: intrinsic Strength `+2` plus a loss wrapper of
    /// `5` is a source at `-3`, and a recipient holding **one** Artifact ends
    /// at `+2`.
    ///
    /// `0x3ad358` IL_00a1-IL_0133 adjusts the frozen `StrengthPower` value by
    /// the wrapper's own frozen amount, so the copied value is `-3 + 5 = +2`.
    /// The two entries are then two separate applications through two
    /// separate gates: `GetTypeForAmount(+2)` `0x83a94` is 1, so
    /// `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` IL_001f-IL_0032
    /// passes the Strength through without spending; the wrapper is Type 2 at
    /// every amount, so it is blocked and spends the Artifact. No aggregate
    /// signed scalar can produce this state — an aggregate would have copied
    /// `-3` and been blocked as one Type-2 application, leaving `0`.
    #[test]
    fn misery_folds_the_wrapper_out_of_the_strength_copy_and_artifact_spends_on_the_wrapper() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -3);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        let recipient = &next.monsters[1];
        assert_eq!(
            recipient.powers.value(PowerId::Strength),
            2,
            "the folded Strength copy is positive and the Artifact let it through",
        );
        assert_eq!(recipient.powers.value(PowerId::Artifact), 0);
        assert_eq!(recipient.powers.value(PowerId::TempStrength), 0);
        assert_eq!(
            rows(recipient),
            [(crate::hot::AttachedPowerModel::Strength, 2)],
            "the wrapper was blocked, so it never attached",
        );
        assert_eq!(
            crate::engine::monsters::strength_applier(recipient),
            Some(crate::hot::Applier::Player),
            "`entry.Key.Applier` (`0x3ad358` IL_0354) is the source row's",
        );
        // The source is untouched by its own snapshot.
        assert_eq!(next.monsters[0].powers.value(PowerId::Strength), -3);
        assert_eq!(next.monsters[0].powers.value(PowerId::TempStrength), -5);
    }

    /// Without the Artifact the same play reproduces the source's whole
    /// state on the recipient — the wrapper lands and its own `BeforeApplied`
    /// re-applies the `-5`.
    #[test]
    fn a_folded_copy_reproduces_the_source_when_nothing_blocks_it() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        let recipient = &next.monsters[1];
        assert_eq!(recipient.powers.value(PowerId::Strength), -3);
        assert_eq!(recipient.powers.value(PowerId::TempStrength), -5);
        assert_eq!(
            rows(recipient),
            [
                (crate::hot::AttachedPowerModel::Strength, -3),
                (crate::hot::AttachedPowerModel::DarkShackles, 5),
            ],
            "the copied wrapper is a real instance on the recipient, behind \
             the Strength its `BeforeApplied` `0x348d20` applied",
        );
    }

    /// A source whose whole negative Strength came from one wrapper folds to
    /// exactly zero, and `0x3ad358` IL_0268-IL_026f skips a zero-valued entry
    /// **before** the lookup — so it consumes no Artifact while the wrapper
    /// still copies.
    #[test]
    fn a_folded_strength_of_zero_is_skipped_and_consumes_no_artifact() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            0,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -5);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        let recipient = &next.monsters[1];
        assert_eq!(
            recipient.powers.value(PowerId::Artifact),
            1,
            "exactly one Artifact was spent, and it was spent on the wrapper",
        );
        assert_eq!(recipient.powers.value(PowerId::Strength), 0);
        assert!(rows(recipient).is_empty());

        // The same source against a recipient with no Artifact: the wrapper
        // copies, and nothing copies the zero.
        let (mut open, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut open,
            0,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        let (next, _) = play_misery(&open, &catalog, source).unwrap();
        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), -5);
        assert_eq!(
            rows(&next.monsters[1]),
            [
                (crate::hot::AttachedPowerModel::Strength, -5),
                (crate::hot::AttachedPowerModel::DarkShackles, 5),
            ],
        );
    }

    /// A source whose Strength is net POSITIVE is not in the dictionary at
    /// all (`0x3ad30a` keeps `TypeForCurrentAmount == 2` only), so there is
    /// nothing for the fold to adjust — and IL_00ff's `brfalse` means nothing
    /// is synthesised either. The selected wrapper still copies on its own.
    #[test]
    fn a_net_positive_source_copies_its_wrappers_and_synthesises_no_strength() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            8,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 3);

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        let recipient = &next.monsters[1];
        assert_eq!(
            recipient.powers.value(PowerId::Strength),
            -5,
            "only the wrapper's own nested application moved it",
        );
        assert_eq!(recipient.powers.value(PowerId::TempStrength), -5);
        assert_eq!(
            rows(recipient),
            [
                (crate::hot::AttachedPowerModel::Strength, -5),
                (crate::hot::AttachedPowerModel::DarkShackles, 5),
            ],
        );
    }

    /// Two different wrapper classes are two entries at two positions, and
    /// the recipient's Artifacts are spent on them **in ledger order**.
    ///
    /// With one Artifact the order is directly observable: the first wrapper
    /// is blocked and the second lands, so the two orders end in two
    /// different states.
    #[test]
    fn two_ordered_wrappers_spend_two_artifacts_and_their_order_is_observable() {
        use crate::hot::AttachedPowerModel as Model;
        let pair = [(Model::DarkShackles, 4), (Model::EnfeeblingTouch, 3)];

        // Two Artifacts: both wrappers blocked, the folded `+2` Strength let
        // through by neither of them.
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(&mut state, 2, &pair);
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -5);
        let (next, _) = play_misery(&state, &catalog, source).unwrap();
        assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 0);
        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), 2);
        assert_eq!(rows(&next.monsters[1]), [(Model::Strength, 2)]);

        // One Artifact, and the two ledger orders diverge.
        for (order, blocked, landed) in [
            (pair, 4, Model::EnfeeblingTouch),
            (
                [(Model::EnfeeblingTouch, 3), (Model::DarkShackles, 4)],
                3,
                Model::DarkShackles,
            ),
        ] {
            let (mut state, catalog, source) = fold_fixture();
            give_source_wrappers(&mut state, 2, &order);
            state.monsters_mut()[1]
                .powers
                .set(PowerId::Artifact, SlotWire::Int, 1);
            let (next, _) = play_misery(&state, &catalog, source).unwrap();
            let landed_amount = order
                .iter()
                .find(|(model, _)| *model == landed)
                .map(|(_, amount)| *amount)
                .unwrap();
            assert_eq!(next.monsters[1].powers.value(PowerId::Artifact), 0);
            assert_eq!(
                next.monsters[1].powers.value(PowerId::Strength),
                2 - landed_amount,
                "the first wrapper in ledger order took the Artifact ({blocked})",
            );
            assert_eq!(
                rows(&next.monsters[1]),
                [
                    (Model::Strength, 2 - landed_amount),
                    (landed, landed_amount),
                ],
            );
        }
    }

    /// The dictionary — and the fold — are frozen before the awaited
    /// `DamageCmd` at `0x3ad358` IL_014c, so an attack that kills the source
    /// and takes its whole ledger with it cannot change what is copied.
    #[test]
    fn the_fold_is_frozen_before_miserys_own_attack_kills_the_source() {
        let (mut state, catalog, source) = fold_fixture();
        state.monsters_mut()[0].hp = 7;
        give_source_wrappers(
            &mut state,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[0].hp, 0);
        assert!(
            rows(&next.monsters[0]).is_empty(),
            "death cleanup took the source's rows",
        );
        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), -3);
        assert_eq!(
            rows(&next.monsters[1]),
            [
                (crate::hot::AttachedPowerModel::Strength, -3),
                (crate::hot::AttachedPowerModel::DarkShackles, 5),
            ],
        );
    }

    /// A reachable Unsettling Lamp refuses beside a wrapper, and for an ORDER
    /// reason rather than an applier one — `HasDoubledTemporaryPowerSource`
    /// `0x9d66c` makes the Lamp's second doubling depend on which model it
    /// latched on first.
    #[test]
    fn misery_refuses_a_wrapper_fold_beside_an_unsettling_lamp() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        state.fanouts.set_unsettling_lamp_available(true);
        assert_eq!(
            play_misery(&state, &catalog, source).unwrap_err(),
            EngineRefusal::MalformedArgs("Misery temporary Strength clone Unsettling Lamp order"),
        );

        // The identical root without the Lamp replays.
        let (mut open, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut open,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        play_misery(&open, &catalog, source).unwrap();
    }

    /// A copy that lands on a recipient already carrying the SAME model
    /// stacks in place (`FindExistingInstanceForStacking` `0x1338d8` IL_0058
    /// is by model id), while a different model takes its own position.
    #[test]
    fn a_wrapper_copy_stacks_by_model_on_the_recipient() {
        use crate::hot::AttachedPowerModel as Model;
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(&mut state, 0, &[(Model::DarkShackles, 4)]);
        crate::engine::damage::write_monster_temp_strength_wrapper(
            &mut state.monsters_mut()[1],
            Model::DarkShackles,
            2,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(
            rows(&next.monsters[1]),
            [(Model::Strength, -6), (Model::DarkShackles, 6)],
            "one record at its original position, amount 2 + 4",
        );
        assert_eq!(next.monsters[1].powers.value(PowerId::TempStrength), -6);
    }

    // ------------------------------------------------------------------
    // #2727 — Sleight of Flesh fires only for a player-applied clone.
    // ------------------------------------------------------------------

    /// A source carrying `amount` Strength applied by `applier`, a Toadpole
    /// recipient, and a live Sleight of Flesh the recipient can feel.
    fn sleight_clone_fixture(
        applier: crate::hot::Applier,
        amount: i32,
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        let (mut state, catalog, source) = fold_fixture();
        crate::engine::damage::write_monster_strength(
            &mut state.monsters_mut()[0],
            amount,
            applier,
            true,
        );
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        (state, catalog, source)
    }

    /// The whole applier axis of `0x344dc4` IL_0067-IL_0075, one play each.
    ///
    /// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_02b6/IL_0354 hands the
    /// recipient-side command the **source instance's** retained `_applier`,
    /// so the copy's Type-2 Strength change carries whoever applied the
    /// original. `SleightOfFleshPower` is player-owned, so only the player
    /// reaches its `CreatureCmd::Damage` at IL_00b5.
    ///
    /// * `Applier::Player` — a `Malaise`-sourced negative Strength
    ///   (`Malaise/<OnPlay>d__9::MoveNext` `0x3ab428` IL_00fc-IL_0109 passes
    ///   `card.Owner.Player.Creature`). Fires.
    /// * `Applier::Monster` — an enemy self-buff driven negative
    ///   (`RitualPower/<AfterSideTurnEnd>d__11::MoveNext` `0x342a94`
    ///   IL_0052-IL_0071 passes the acting creature). Does not.
    /// * `Applier::None` — a relic's literal null
    ///   (`Brimstone/<AfterSideTurnStart>d__8::MoveNext` `0x32098c`
    ///   IL_012d-IL_0130). Does not; `beq` against a `Creature` reference is
    ///   false for null.
    ///
    /// Mutation control: on `origin/main` the walk took no applier at all and
    /// every row below dealt the 4, so the two negative arms fail there.
    #[test]
    fn a_cloned_strength_wakes_sleight_of_flesh_only_for_the_player() {
        for (applier, fires) in [
            (crate::hot::Applier::Player, true),
            (crate::hot::Applier::Monster(41), false),
            (crate::hot::Applier::None, false),
        ] {
            let (state, catalog, source) = sleight_clone_fixture(applier, -3);
            let before = state.monsters[1].hp;

            let (next, _) = play_misery(&state, &catalog, source).unwrap();

            assert_eq!(
                next.monsters[1].powers.value(PowerId::Strength),
                -3,
                "{applier:?}: the copy landed either way",
            );
            assert_eq!(
                crate::engine::monsters::strength_applier(&next.monsters[1]),
                Some(applier),
                "{applier:?}: the recipient's row keeps the retained identity",
            );
            assert_eq!(
                next.monsters[1].hp,
                if fires { before - 4 } else { before },
                "{applier:?}: Sleight of Flesh",
            );
        }
    }

    /// #3364 — a `Misery` copy of the Mysterious Knight's spawn Strength,
    /// attributed to the Knight by the opening, carries the Knight as
    /// applier and so does not wake Sleight of Flesh.
    ///
    /// `MysteriousKnight/<AfterAddedToRoom>d__0::MoveNext` `0x3642a0`
    /// IL_0087-IL_00a0 applies the `6` with the Knight's own `Creature` as
    /// applier; the player's later Malaise-style `-9` is a `ModifyAmount`
    /// on that instance, which keeps it (`0x3efbac` IL_013d writes
    /// `set_Applier` on the fresh-attach path only); `Misery/<OnPlay>d__3`
    /// `0x3ad358` IL_0354 hands the copy that retained applier; and
    /// `0x344dc4` IL_0067-IL_0075 returns for any applier but the player.
    /// Before #3364 the row stayed `Unknown` and the root refused
    /// "Misery Strength clone unknown applier" at admission.
    #[test]
    fn a_cloned_mysterious_knight_spawn_strength_keeps_the_knight_and_is_silent() {
        let (mut state, catalog, source) = fold_fixture();
        let knight = &mut state.monsters_mut()[0];
        knight.kind = MonsterKind::MysteriousKnight;
        knight.powers.set(PowerId::Strength, SlotWire::Int, 6);
        let uid = knight.uid;
        crate::engine::damage::materialize_entering_strength_provenance(&mut state);
        crate::engine::damage::attribute_spawn_strength_to_its_owner(&mut state);
        let owner = crate::hot::Applier::Monster(uid);
        assert_eq!(
            crate::engine::monsters::strength_applier(&state.monsters[0]),
            Some(owner)
        );
        crate::engine::damage::write_monster_strength(
            &mut state.monsters_mut()[0],
            -3,
            crate::hot::Applier::Player,
            true,
        );
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let before = state.monsters[1].hp;

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), -3);
        assert_eq!(
            crate::engine::monsters::strength_applier(&next.monsters[1]),
            Some(owner),
            "the copy carries the Knight, not the player who drove it negative",
        );
        assert_eq!(next.monsters[1].hp, before, "Sleight of Flesh stays silent");
    }

    /// A POSITIVE player-applied copy still does not fire, so the Type test
    /// and the applier test are independent rather than one standing in for
    /// the other.
    ///
    /// #2693 S3's own fixture: intrinsic `+2` under a loss wrapper of `5` is a
    /// source at `-3` whose frozen Strength entry folds to `+2`.
    /// `GetTypeForAmount` `0x83a94` IL_0013-IL_003e reports 2 for an
    /// `AllowNegative` Intensity power only while the delta is negative, so
    /// `0x344dc4` IL_0037-IL_004b returns on that `+2` — and the wrapper that
    /// follows it in the ledger fires exactly once through its nested `-5`.
    /// One Sleight hit, not two and not zero.
    #[test]
    fn a_positive_player_applied_strength_copy_is_type_one_and_silent() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            2,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let before = state.monsters[1].hp;

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), -3);
        assert_eq!(
            next.monsters[1].hp,
            before - 4,
            "only the wrapper's nested Type-2 Strength fired",
        );
    }

    /// A cloned temporary-Strength wrapper fires through its **nested**
    /// Strength application, which is not an `ITemporaryPower`.
    ///
    /// `0x344dc4` IL_007a-IL_0087 tests the *changed* power, and what changes
    /// here is the `StrengthPower` that
    /// `TemporaryStrengthPower/<BeforeApplied>d__20::MoveNext` `0x348d20`
    /// IL_001d-IL_004b applies with `Sign * amount` — the wrapper's own Type-2
    /// application is the one that returns at IL_0087. IL_003e-IL_004b
    /// forwards the applier the wrapper application was handed, and the
    /// boundary admits only `Applier::Player` for this family
    /// (`temporary Strength attachment applier`, the eight-writer census), so
    /// the player is the only reachable case and it fires.
    #[test]
    fn a_cloned_wrapper_fires_sleight_through_its_nested_strength() {
        let (mut state, catalog, source) = fold_fixture();
        give_source_wrappers(
            &mut state,
            0,
            &[(crate::hot::AttachedPowerModel::DarkShackles, 5)],
        );
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let before = state.monsters[1].hp;

        let (next, _) = play_misery(&state, &catalog, source).unwrap();

        assert_eq!(next.monsters[1].powers.value(PowerId::Strength), -5);
        assert_eq!(next.monsters[1].powers.value(PowerId::TempStrength), -5);
        assert_eq!(
            next.monsters[1].hp,
            before - 4,
            "the nested `Apply<StrengthPower>` is a permanent Type-2 change",
        );
    }

    #[test]
    fn end_of_days_applies_in_roster_order_then_takes_a_fresh_doom_snapshot() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 0, 20);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 35);
        second.uid = 42;
        state.monsters_mut().push(second);
        let mut events = Vec::new();

        run_end_of_days(&mut state, &catalog, source, 0, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[1].hp, 35);
        assert_eq!(state.monsters[1].powers.value(PowerId::Doom), 29);
        let doom_subjects = events
            .iter()
            .filter_map(|event| match event {
                Event::PowerChanged {
                    subject: crate::engine::Subject::Monster(uid),
                    power: PowerId::Doom,
                    ..
                } => Some(*uid),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            doom_subjects,
            [41, 42],
            "the application snapshot is ordered"
        );
        assert!(
            crate::engine::admission::IMPLEMENTED_RELICS
                .contains(&crate::ids::RelicId::RelicBookRepairKnife),
            "Book Repair's post-batch response is represented"
        );
        assert!(
            crate::engine::admission::IMPLEMENTED_RELICS
                .contains(&crate::ids::RelicId::RelicUnsettlingLamp)
                && crate::engine::admission::IMPLEMENTED_POWERS.contains(&PowerId::Artifact),
            "the shared Lamp amount modifier and Artifact gate are represented"
        );
    }

    /// RAB8SE1H26ZH node 42 (#3209 class): a manual End of Days+ with
    /// Unsettling Lamp available. The play preflight rehearses the body as
    /// the card's own CardPlay, so Lamp latches and doubles the Doom (native
    /// checkpoint: DOOM_POWER 74 on a 269-HP Slimed Berserker) instead of
    /// refusing for want of an active play.
    #[test]
    fn end_of_days_play_preflight_lets_unsettling_lamp_double_the_doom() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 1, 269);
        let card = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 18;
        state.fanouts.set_unsettling_lamp_available(true);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 269);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 74);
        assert!(
            !state.fanouts.unsettling_lamp_available(),
            "Lamp is consumed"
        );
    }

    #[test]
    fn end_of_days_listener_death_is_observed_before_the_next_application() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 1, 9);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 40);
        second.uid = 42;
        state.monsters_mut().push(second);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let mut events = Vec::new();

        run_end_of_days(&mut state, &catalog, source, 1, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 0);
        assert_eq!(state.monsters[1].hp, 0);
        assert_eq!(state.monsters[1].powers.value(PowerId::Doom), 0);
        assert!(
            state.history.over,
            "the fresh Doom batch kills the survivor"
        );
    }

    #[test]
    fn end_of_days_application_snapshot_does_not_target_an_axebot_replacement() {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 1);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        state.monsters_mut().push(axebot);
        let targets = end_of_days_target_snapshot(&state);
        assert_eq!(targets, vec![(0, 0)], "the native object uid is frozen");

        state.monsters_mut()[0].hp = 0;
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].uid, 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Stock), 1);
        let replacement = state.monsters[0].clone();
        let niche_after_respawn = state.rng.get(crate::hot::RngStream::Niche);

        apply_end_of_days_snapshot(&mut state, 29, &targets, &mut Vec::new()).unwrap();

        assert_eq!(state.monsters[0], replacement);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 0);
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Niche),
            niche_after_respawn
        );
    }

    #[test]
    fn end_of_days_terminal_shroud_juggernaut_stops_the_application_wave() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 0, 1);
        state.powers.set(PowerId::Shroud, SlotWire::Int, 3);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let mut events = Vec::new();

        run_end_of_days(&mut state, &catalog, source, 0, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 0);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            1,
            "terminal Shroud -> Juggernaut stops before the Doom kill batch"
        );
    }

    #[test]
    fn end_of_days_late_overflow_and_wrong_operands_are_atomic() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 0, 100);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
        second.uid = 42;
        second
            .powers
            .set(PowerId::Doom, SlotWire::Int, i32::MAX - 10);
        state.monsters_mut().push(second);
        let before = state.clone();
        let mut events = Vec::new();
        let before_events = events.clone();

        assert_eq!(
            run_end_of_days(&mut state, &catalog, source, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("monster doom"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        let spec = *catalog.spec(source.atom).unwrap();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(29)],
            events: &mut events,
        };
        assert_eq!(
            end_of_days_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("end_of_days_exact operands"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn end_of_days_requires_the_complete_canonical_one_step_program() {
        let (mut state, catalog, source) = fixture(CardId::EndOfDays, 0, 100);
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args);
        assert!(end_of_days_program_is_exact(
            &catalog,
            catalog.steps(&spec),
            args
        ));

        let wrong_kind = crate::catalog::CompiledStep {
            kind: StepKind::HangExact,
            ..step
        };
        assert!(!end_of_days_program_is_exact(&catalog, &[wrong_kind], args));
        assert!(!end_of_days_program_is_exact(&catalog, &[step, step], args));
        assert!(!end_of_days_program_is_exact(
            &catalog,
            &[step],
            &[CompiledArg::I(37)]
        ));

        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(37)],
            events: &mut events,
        };
        assert_eq!(
            end_of_days_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("end_of_days_exact program"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn undeath_levels_block_then_clone_the_exact_live_payload_to_discard_bottom() {
        for (upgrade, expected_block) in [(0, 7), (1, 9)] {
            let (mut state, catalog, mut source) = fixture(CardId::Undeath, upgrade, 100);
            source.flags = crate::hot::CARD_FLAG_RINGING;
            state.piles.get_mut(PileId::Play).make_mut()[0] = source;
            let payload = CardInstanceState {
                local_retain: true,
                local_sly: true,
                transient_retain: true,
                ..CardInstanceState::default()
            };
            state.card_states.set(source.uid, payload.clone());
            state.next_card_uid = 100;
            let existing = HotCard {
                uid: 99,
                atom: source.atom,
                flags: 0,
            };
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(existing);
            let mut events = Vec::new();

            run_undeath(&mut state, &catalog, source, upgrade, &mut events).unwrap();

            assert_eq!(state.block, expected_block);
            assert_eq!(state.history.owner_generated_cards_combat, 1);
            assert_eq!(state.next_card_uid, 101);
            assert!(state.exact_piles);
            let discard = state.piles.get(PileId::Discard).as_slice();
            assert_eq!(discard.len(), 2);
            assert_eq!(discard[0], existing);
            assert_eq!(discard[1].uid, 100);
            assert_eq!(discard[1].atom, source.atom);
            assert_eq!(discard[1].flags, source.flags);
            assert_eq!(state.card_states.get(discard[1].uid), payload);
            assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
        }
    }

    #[test]
    fn undeath_terminal_block_records_generation_before_suppressing_insertion() {
        let (mut state, catalog, source) = fixture(CardId::Undeath, 0, 1);
        state.next_card_uid = 100;
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let mut events = Vec::new();

        run_undeath(&mut state, &catalog, source, 0, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.block, 7);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(
            state.next_card_uid, 100,
            "the ending Add allocates no clone"
        );
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
    }

    #[test]
    fn undeath_clone_failure_and_source_refusals_roll_back_block_and_events() {
        let (mut state, catalog, source) = fixture(CardId::Undeath, 0, 100);
        state.next_card_uid = u32::MAX;
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_undeath(&mut state, &catalog, source, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.next_card_uid = 100;
        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        let before = state.clone();
        assert_eq!(
            run_undeath(&mut state, &catalog, source, 0, &mut events),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.piles.get_mut(PileId::Discard).make_mut().clear();
        state.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_LEGACY;
        let before = state.clone();
        assert_eq!(
            run_undeath(&mut state, &catalog, source, 0, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "undeath_exact physical source"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn undeath_requires_the_complete_canonical_program_and_empty_operands() {
        let (mut state, catalog, source) = fixture(CardId::Undeath, 0, 100);
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args);
        assert!(undeath_program_is_exact(
            &catalog,
            catalog.steps(&spec),
            args
        ));
        assert!(!undeath_program_is_exact(
            &catalog,
            &[crate::catalog::CompiledStep {
                kind: StepKind::HangExact,
                ..step
            }],
            args
        ));
        assert!(!undeath_program_is_exact(&catalog, &[step, step], args));
        assert!(!undeath_program_is_exact(
            &catalog,
            &[step],
            &[CompiledArg::I(9)]
        ));

        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        assert_eq!(
            undeath_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("undeath_exact operands"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn admitted_after_block_listeners_cannot_relocate_or_reidentify_undeath() {
        let source = include_str!("../engine/damage.rs");
        let listener = source
            .split("pub(crate) fn powers_after_block_gained(")
            .nth(1)
            .unwrap()
            .split("const MISERY_SCALAR_POWERS")
            .next()
            .unwrap();
        let arms = listener
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("PowerId::")
                    .and_then(|line| line.split_once(" =>"))
                    .map(|(power, _)| power)
            })
            .collect::<Vec<_>>();
        assert_eq!(arms, ["Juggernaut", "BeaconOfHope"]);
        assert!(!listener.contains("piles"));
        assert!(!listener.contains("card_states"));
        assert!(!listener.contains("source_uid"));
    }

    #[test]
    fn shared_fate_applies_owner_first_and_skips_enemy_after_combat_end() {
        let (mut state, catalog, source) = fixture(CardId::SharedFate, 1, 100);
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(-2), CompiledArg::I(-3)],
            events: &mut events,
        };
        shared_fate_exact(&mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::Strength), -2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -3);
        assert!(matches!(events[0], Event::PowerChanged { amount: -2, .. }));
        assert!(matches!(events[1], Event::PowerChanged { amount: -3, .. }));

        state.history.over = true;
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(-2), CompiledArg::I(-3)],
            events: &mut events,
        };
        shared_fate_exact(&mut ctx).unwrap();
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn the_scythe_reads_and_publishes_growth_on_normal_and_enemy_lethal_paths() {
        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: 4,
                ..CardInstanceState::default()
            },
        );
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        the_scythe_exact(&mut ctx).unwrap();
        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 83);
        assert_eq!(state.card_states.get(source.uid).damage_growth, 9);
        assert_ne!(
            state.piles.get(PileId::Play).as_slice()[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            0
        );

        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 13);
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        the_scythe_exact(&mut ctx).unwrap();
        assert!(state.history.over, "the only enemy died");
        assert_eq!(state.card_states.get(source.uid).damage_growth, 5);
    }

    /// #2941: an ordinary fight's linked copy keeps its `DeckVersion` link
    /// through a play (the master's growth is the live copy's), and a clone
    /// keeps the local growth but carries no link (`CardModel::AfterCloned`
    /// RVA `0x7d31c` IL_0039-IL_003b nulls `DeckVersion`).
    #[test]
    fn a_linked_scythe_keeps_its_deck_row_through_a_play_and_a_clone_detaches() {
        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: 4,
                ..CardInstanceState::default()
            },
        );
        state
            .card_states
            .set_scythe_deck_links(vec![(source.uid, 3)])
            .unwrap();
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        the_scythe_exact(&mut ctx).unwrap();
        assert_eq!(state.card_states.get(source.uid).damage_growth, 9);
        assert_eq!(state.card_states.scythe_deck_row(source.uid), Some(3));

        let source = state.piles.get(PileId::Play).as_slice()[0];
        crate::engine::cards::inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        let clone = state.piles.get(PileId::Discard).as_slice()[0];
        assert_ne!(clone.uid, source.uid);
        assert_eq!(state.card_states.get(clone.uid).damage_growth, 9);
        assert_eq!(state.card_states.scythe_deck_row(clone.uid), None);
        assert_eq!(state.card_states.scythe_deck_links(), &[(source.uid, 3)]);
    }

    #[test]
    fn hopper_scythe_updates_master_and_stale_master_refuses_before_attack() {
        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        let mut hopper = HotMonster::new(
            MonsterKind::ThievingHopper,
            crate::engine::THIEVING_HOPPER_HP,
        );
        hopper.max_hp = crate::engine::THIEVING_HOPPER_HP;
        hopper.powers.set(
            PowerId::EscapeArtist,
            SlotWire::Int,
            crate::engine::THIEVING_HOPPER_ESCAPE_ARTIST,
        );
        state.monsters_mut()[0] = hopper;
        state.exact_piles = true;
        state.next_card_uid = source.uid + 1;
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: 4,
                ..CardInstanceState::default()
            },
        );
        state.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card: source,
                state: state.card_states.get(source.uid),
            }],
            history: Vec::new(),
        }));
        assert!(crate::engine::monsters::thieving_hopper_state_is_valid(
            &state
        ));
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        the_scythe_exact(&mut ctx).unwrap();
        assert_eq!(state.card_states.get(source.uid).damage_growth, 9);
        assert_eq!(
            state.card_states.hopper().unwrap().master[0]
                .state
                .damage_growth,
            9
        );

        let (mut stale, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        let mut hopper = HotMonster::new(
            MonsterKind::ThievingHopper,
            crate::engine::THIEVING_HOPPER_HP,
        );
        hopper.max_hp = crate::engine::THIEVING_HOPPER_HP;
        hopper.powers.set(
            PowerId::EscapeArtist,
            SlotWire::Int,
            crate::engine::THIEVING_HOPPER_ESCAPE_ARTIST,
        );
        stale.monsters_mut()[0] = hopper;
        stale.exact_piles = true;
        stale.next_card_uid = source.uid + 1;
        stale.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: 4,
                ..CardInstanceState::default()
            },
        );
        stale.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card: source,
                state: CardInstanceState {
                    damage_growth: 3,
                    ..CardInstanceState::default()
                },
            }],
            history: Vec::new(),
        }));
        let before = stale.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut stale,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        assert!(the_scythe_exact(&mut ctx).is_err());
        assert_eq!(stale, before);
        assert!(events.is_empty());
    }

    #[test]
    fn the_scythe_player_death_skips_growth_and_overflow_refuses_atomically() {
        let (mut state, catalog, source) = fixture(CardId::TheScythe, 1, 100);
        state.hp = 1;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(7)],
            events: &mut events,
        };
        the_scythe_exact(&mut ctx).unwrap();
        assert!(state.hp <= 0 && state.history.over);
        assert_eq!(state.card_states.get(source.uid).damage_growth, 0);

        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: i32::MAX - 4,
                ..CardInstanceState::default()
            },
        );
        let before = state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        assert_eq!(
            the_scythe_exact(&mut ctx),
            Err(EngineRefusal::CounterOverflow("the scythe damage growth"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn exact_bodies_reject_wrong_operands_before_mutation() {
        for (id, upgrade, args, run) in [
            (
                CardId::SharedFate,
                0,
                vec![CompiledArg::I(-2), CompiledArg::I(-3)],
                shared_fate_exact as fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>,
            ),
            (
                CardId::TheScythe,
                0,
                vec![CompiledArg::I(13), CompiledArg::I(7)],
                the_scythe_exact,
            ),
        ] {
            let (mut state, catalog, source) = fixture(id, upgrade, 100);
            let before = state.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };
            assert!(matches!(
                run(&mut ctx),
                Err(EngineRefusal::MalformedArgs(_))
            ));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn the_scythe_requires_one_live_source_and_an_existing_target_before_mutation() {
        let (mut state, catalog, source) = fixture(CardId::TheScythe, 0, 100);
        let spec = *catalog.spec(source.atom).unwrap();
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(1),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        assert_eq!(
            the_scythe_exact(&mut ctx),
            Err(EngineRefusal::TargetMismatch { required: true })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        let before = state.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(13), CompiledArg::I(5)],
            events: &mut events,
        };
        assert_eq!(
            the_scythe_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn hang_uses_live_multiplier_then_stacks_only_on_the_same_surviving_uid() {
        let (mut state, catalog, source) = fixture(CardId::Hang, 0, 100);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Hang, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Hang);
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10)],
            events: &mut events,
        };

        hang_exact(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].hp, 80);
        assert_eq!(state.monsters[0].powers.value(PowerId::Hang), 4);
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            [MiseryToken::Hang]
        );
    }

    /// #3515: End of Days' `Apply<DoomPower>` walk, Misery's copies
    /// (`PowerCmd.ModifyAmount` / `PowerCmd.Apply`) and Shared Fate's two
    /// `Apply<StrengthPower>` all return at `IsEnding` (`<Apply>d__1`1`
    /// 0x3ef988 IL_0025, `<ModifyAmount>d__6` 0x3f032c IL_003a). While the
    /// combat is ending before the over latch each writes nothing (End of
    /// Days through its Doom writer; its ungated DoomKill finds nothing to
    /// kill); the Adaptable-vetoed control dooms, copies Knockdown and
    /// weakens.
    #[test]
    fn necrobinder_rare_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        let (template, catalog, source) = fixture(CardId::EndOfDays, 0, 50);
        crate::engine::damage::assert_ending_window_gate(&template, "End of Days", |s, _| {
            run_end_of_days(s, &catalog, source, 0, &mut Vec::new())
        });

        let (template, catalog, source) = fixture(CardId::SharedFate, 0, 50);
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Shared Fate", |s, t| {
            shared_fate_exact(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(t),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(-2), CompiledArg::I(-2)],
                events: &mut Vec::new(),
            })
        });

        // Misery copies the target's Knockdown, whose writer still reads
        // `history.over`, onto a second live Gas Bomb.
        let (template, catalog, source) = misery_fixture(0, 50);
        for vetoed in [false, true] {
            let mut state = template.clone();
            state.monsters_mut().clear();
            let mut target = HotMonster::new(MonsterKind::GasBomb, 100);
            target.misery_debuff_order.push_knockdown(2);
            let mut primary = HotMonster::new(MonsterKind::Toadpole, 100);
            primary.hp = 0;
            primary.uid = 1;
            primary.slot = 1;
            if vetoed {
                primary.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
            }
            let mut recipient = HotMonster::new(MonsterKind::GasBomb, 100);
            recipient.uid = 2;
            recipient.slot = 2;
            state.monsters_mut().extend([target, primary, recipient]);
            assert!(!state.history.over);
            assert_eq!(
                crate::engine::damage::damage_combat_is_ending(&state),
                !vetoed
            );
            let (next, _) = play_misery(&state, &catalog, source).unwrap();
            assert_eq!(
                next.monsters[2].misery_debuff_order.knockdown(),
                if vetoed { &[2][..] } else { &[][..] },
                "Misery vetoed={vetoed}"
            );
        }
    }
}
