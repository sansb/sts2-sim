//! Card-step bodies for the `content/cards/defect_orb.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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

use super::StepCtx;
use crate::catalog::CompiledArg;
use crate::engine::damage::{
    alive_targets, note_power, player_attack_all_from_card, player_attack_from_card,
};
use crate::engine::{EngineRefusal, Subject};
use crate::hot::{HotOrb, OrbKind, OrbResetPower};
use crate::ids::{CardId, PowerId, StepKind, StepWord};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::ChannelHittableSnapshot,
    StepKind::ChannelVoltaic,
    StepKind::ChannelX,
    StepKind::CompileDriverUniqueOrbExact,
    StepKind::Darkness,
    StepKind::EvokeFrontExact,
    StepKind::EvokeFrontX,
    StepKind::HyperbeamFocusDownExact,
    StepKind::LightningRod,
    StepKind::RemoveOrbSlotsExact,
    StepKind::ShatterOrbsExact,
    StepKind::Storm,
    StepKind::SynchronizeUniqueOrbFocusExact,
    StepKind::TempFocus,
    StepKind::TeslaCoil,
    StepKind::Thunder,
];

fn exact_int(
    ctx: &StepCtx<'_>,
    site: &'static str,
    allowed: &[(CardId, u8, i64)],
) -> Result<i64, EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(value)]
            if allowed.contains(&(ctx.spec.identity.id, ctx.spec.identity.upgrade, *value)) =>
        {
            Ok(*value)
        }
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn stack_power(
    ctx: &mut StepCtx<'_>,
    site: &'static str,
    power: PowerId,
    allowed: &[(CardId, u8, i64)],
) -> Result<(), EngineRefusal> {
    let amount = exact_int(ctx, site, allowed)?;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow(site))?;
    let current = ctx.state.powers.value(power);
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    crate::engine::play::prepare_after_card_played_scalar_write(
        ctx.state, power, current, updated,
    )?;
    ctx.state.powers.set(power, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, power, updated);
    Ok(())
}

fn add_temp_focus(ctx: &mut StepCtx<'_>, amount: i64) -> Result<(), EngineRefusal> {
    if amount == 0 {
        return Ok(());
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("temporary focus"))?;
    let current = ctx.state.orbs.temp_focus();
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("temporary focus"))?;
    crate::engine::turn::prepare_after_side_turn_end_singleton_write(
        ctx.state,
        crate::hot::AfterSideTurnEndPowerToken::TemporaryFocus,
        current != 0,
        updated != 0,
    )?;
    ctx.state.orbs.set_temp_focus(updated);
    Ok(())
}

/// Chill's frozen hittable-roster count of Frost channels.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn channel_hittable_snapshot(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Chill
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || ctx.args != [CompiledArg::Word(StepWord::Frost)]
    {
        return Err(EngineRefusal::MalformedArgs("channel_hittable_snapshot"));
    }
    let count = alive_targets(ctx.state).len();
    for _ in 0..count {
        if ctx.state.history.over {
            break;
        }
        crate::engine::orbs::channel(ctx.state, ctx.catalog, OrbKind::Frost, ctx.events)?;
    }
    Ok(())
}

/// Voltaic's pre-loop Lightning-channel history snapshot.
///
/// IL (v0.111.0 `sts2.dll`): `Voltaic/<OnPlay>d__8::MoveNext` (RVA
/// `0x3c6d14`) evaluates `CalculatedChannels` once, before the loop
/// (`IL_00a0`–`IL_00c9`, into `<lightningChanneledCount>5__2`), then issues one
/// `OrbCmd::Channel<LightningOrb>` (`IL_00e3`) per `i < count` (`IL_0158`), so
/// the Lightning this play channels does not extend its own loop. The count is
/// the combat's owner-Lightning `OrbChanneledEntry` history (the read is cited
/// on `entry::opening::voltaic_lightning_channeled_seed`), held in
/// `lightning_channeled` only while it is tracked (`>= 0`).
///
/// An untracked `-1` here means a generated Voltaic reached play from a root
/// that never counted its channels: the history count is unknown, and the
/// body refuses by name rather than assuming `0` (#3003). Since #3389 the
/// opening seeds the count whenever the fight's closure holds a Voltaic, so
/// no opened root reaches this; a root loaded untracked with only a
/// generation-reachable Voltaic still can
/// (`engine::admission::lightning_channeled_tracking_is_admissible`).
pub(crate) fn channel_voltaic(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Voltaic
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !ctx.args.is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("channel_voltaic"));
    }
    let count = ctx.state.orbs.lightning_channeled();
    if count < 0 {
        return Err(EngineRefusal::UntrackedCounterNotModeled(
            "lightning_channeled",
        ));
    }
    for _ in 0..count {
        if ctx.state.history.over {
            break;
        }
        crate::engine::orbs::channel(ctx.state, ctx.catalog, OrbKind::Lightning, ctx.events)?;
    }
    Ok(())
}

/// Tempest's captured paid-X Lightning channels, plus one when upgraded.
///
/// IL (v0.111.0 `sts2.dll`): `Tempest/<OnPlay>d__5::MoveNext` (RVA 0x3c226c)
/// stores `CardModel::ResolveEnergyXValue` into `<numOfOrbs>5__2` at IL_00a0,
/// then at IL_00ab–00bd adds 1 when `CardModel::get_IsUpgraded` is true. The
/// loop (IL_00c9 branches to the IL_0141 test) checks `i < numOfOrbs` before
/// each `OrbCmd::Channel<LightningOrb>` (IL_00d7), so Tempest+ at X=0 still
/// channels one Lightning. The bonus is the card's upgrade level (0 or 1),
/// the same +1 Multi-Cast's `evoke_front_x` and Cascade's `autoplay_draw_x`
/// add to their resolved X.
pub(crate) fn channel_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Tempest
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !ctx.args.is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("channel_x"));
    }
    let count = ctx
        .x_value
        .checked_add(i64::from(ctx.spec.identity.upgrade))
        .ok_or(EngineRefusal::CounterOverflow("channel_x count"))?;
    for _ in 0..count {
        if ctx.state.history.over {
            break;
        }
        crate::engine::orbs::channel(ctx.state, ctx.catalog, OrbKind::Lightning, ctx.events)?;
    }
    Ok(())
}

/// Compile Driver's attack followed by a live distinct-kind draw.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`, `add`.
pub(crate) fn compile_driver_unique_orb_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = exact_int(
        ctx,
        "compile_driver_unique_orb_exact",
        &[
            (CardId::CompileDriver, 0, 7),
            (CardId::CompileDriver, 1, 10),
        ],
    )?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    let mut kinds = 0u8;
    for orb in ctx.state.orbs.as_slice() {
        kinds |= 1u8 << (orb.kind() as u8);
    }
    crate::engine::play::draw_cardplay_no_result(ctx, kinds.count_ones() as usize)
}

/// Darkness's direct Dark-passive queue walk.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn darkness(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = exact_int(
        ctx,
        "darkness",
        &[(CardId::Darkness, 0, 1), (CardId::Darkness, 1, 2)],
    )?;
    let indices: Vec<usize> = ctx
        .state
        .orbs
        .as_slice()
        .iter()
        .enumerate()
        .filter_map(|(index, orb)| (orb.kind() == OrbKind::Dark).then_some(index))
        .collect();
    for index in indices {
        for _ in 0..count {
            if ctx.state.history.over {
                return Ok(());
            }
            let orb = ctx.state.orbs.as_slice()[index];
            let amount = i64::from(orb.amount().expect("Dark carries an accumulator"))
                .checked_add(crate::engine::orbs::orb_value(ctx.state, 6)?)
                .ok_or(EngineRefusal::CounterOverflow("darkness"))?;
            let amount: i32 = amount
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("darkness"))?;
            ctx.state.orbs.replace(
                index,
                HotOrb::from_parts(OrbKind::Dark, Some(amount))
                    .expect("Dark passive remains non-negative"),
            );
        }
    }
    Ok(())
}

/// `evoke_front_exact` — retain the front orb until the final evoke.
///
/// Python `_run_steps_inner` (frozen, deleted #2827): Dualcast/Quadcast issue the
/// literal number of independently ending-gated EvokeNext commands; only the
/// last dequeues. The complete five-kind evoke dispatch lives in the engine.
pub(crate) fn evoke_front_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = exact_int(
        ctx,
        "evoke_front_exact",
        &[
            (CardId::Dualcast, 0, 2),
            (CardId::Dualcast, 1, 2),
            (CardId::Quadcast, 0, 4),
            (CardId::Quadcast, 1, 4),
        ],
    )?;
    for index in 0..count {
        crate::engine::orbs::evoke_front(ctx.state, ctx.catalog, index == count - 1, ctx.events)?;
    }
    Ok(())
}

/// `evoke_front_x` — Multi Cast's paid-X evokes plus its upgrade bonus.
///
/// Python `_run_steps_inner` (frozen, deleted #2827). X is captured at resource spend;
/// the final call alone dequeues, so base X=0 is empty while upgraded X=0
/// performs one dequeuing evoke.
pub(crate) fn evoke_front_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let bonus = exact_int(
        ctx,
        "evoke_front_x",
        &[(CardId::MultiCast, 0, 0), (CardId::MultiCast, 1, 1)],
    )?;
    let count = ctx
        .x_value
        .checked_add(bonus)
        .ok_or(EngineRefusal::CounterOverflow("evoke_front_x count"))?;
    for index in 0..count {
        crate::engine::orbs::evoke_front(ctx.state, ctx.catalog, index == count - 1, ctx.events)?;
    }
    Ok(())
}

/// Hyperbeam's ending-gated `TemporaryFocusPower(-3)` suffix.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) pins the two v0.111.0 Hyperbeam
/// bodies and applies the signed wrapper only while combat remains live.
pub(crate) fn hyperbeam_focus_down_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = exact_int(
        ctx,
        "hyperbeam_focus_down_exact",
        &[(CardId::Hyperbeam, 0, -3), (CardId::Hyperbeam, 1, -3)],
    )?;
    if ctx.state.history.over {
        return Ok(());
    }
    add_temp_focus(ctx, amount)
}

/// Lightning Rod's acquisition-ordered duration stack.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`, `begin_player_turn`.
pub(crate) fn lightning_rod(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = exact_int(
        ctx,
        "lightning_rod",
        &[(CardId::LightningRod, 0, 2), (CardId::LightningRod, 1, 2)],
    )?;
    let was_absent = ctx.state.powers.value(PowerId::LightningRod) == 0;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("lightning rod"))?;
    let updated = ctx
        .state
        .powers
        .value(PowerId::LightningRod)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("lightning rod"))?;
    ctx.state
        .powers
        .set(PowerId::LightningRod, SlotWire::Int, updated);
    if was_absent {
        ctx.state
            .orbs
            .register_reset_power(OrbResetPower::LightningRod);
        if !ctx
            .state
            .fanouts
            .register_after_energy_reset(crate::hot::AfterEnergyResetPower::LightningRod)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-energy-reset listener order",
            ));
        }
    }
    ctx.state.normalize_after_energy_reset_order_if_unique();
    note_power(ctx.events, Subject::Player, PowerId::LightningRod, updated);
    Ok(())
}

/// Bulk Up's exact silent one-slot removal.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn remove_orb_slots_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = exact_int(
        ctx,
        "remove_orb_slots_exact",
        &[(CardId::BulkUp, 0, 1), (CardId::BulkUp, 1, 1)],
    )?;
    crate::engine::orbs::remove_slots(ctx.state, amount)
}

/// Shatter's post-AoE queue-count snapshot and paired evokes.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn shatter_orbs_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = exact_int(
        ctx,
        "shatter_orbs_exact",
        &[(CardId::Shatter, 0, 7), (CardId::Shatter, 1, 11)],
    )?;
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        1,
        ctx.events,
    )?;
    let count = ctx.state.orbs.as_slice().len();
    for _ in 0..count {
        crate::engine::orbs::evoke_front(ctx.state, ctx.catalog, false, ctx.events)?;
        crate::engine::orbs::evoke_front(ctx.state, ctx.catalog, true, ctx.events)?;
    }
    Ok(())
}

/// Storm's additive trigger amount; the play pipeline owns its latch.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn storm(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    stack_power(
        ctx,
        "storm",
        PowerId::Storm,
        &[(CardId::Storm, 0, 1), (CardId::Storm, 1, 2)],
    )
}

/// Synchronize's distinct-live-orb TemporaryFocusPower snapshot.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) pins the two source bodies, counts
/// distinct live orb ids, and applies level amount times that count through
/// the shared turn-end-expiring Focus wrapper.
pub(crate) fn synchronize_unique_orb_focus_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    let multiplier = exact_int(
        ctx,
        "synchronize_unique_orb_focus_exact",
        &[(CardId::Synchronize, 0, 1), (CardId::Synchronize, 1, 2)],
    )?;
    let mut kinds = 0u8;
    for orb in ctx.state.orbs.as_slice() {
        kinds |= 1u8 << (orb.kind() as u8);
    }
    let amount = multiplier
        .checked_mul(i64::from(kinds.count_ones()))
        .ok_or(EngineRefusal::CounterOverflow("synchronize focus"))?;
    if ctx.state.history.over {
        return Ok(());
    }
    add_temp_focus(ctx, amount)
}

/// Focused Strike / Hotfix TemporaryFocusPower.
///
/// Python `_run_steps_inner` (frozen, deleted #2827): add the exact wrapper amount now;
/// it remains live through this turn's orb passives and expires at own side
/// end.
///
/// IL (v0.111.0): `FocusedStrike/<OnPlay>d__7::MoveNext` (RVA `0x39f970`)
/// awaits `DamageCmd::Attack` (IL_004c-0x00d9), then
/// `PowerCmd.Apply<FocusedStrikePower>` at IL_010d; `Hotfix/<OnPlay>d__7::
/// MoveNext` (RVA `0x3a5cf0`) applies `HotfixPower` at IL_00d1. The generic
/// ``PowerCmd/<Apply>d__1`1::MoveNext`` (RVA `0x3ef988`) returns null at
/// IL_0020-0x0034 while `CombatManager.IsEnding`, before the power exists, so
/// no temporary power is applied and its FocusPower side effect never runs:
/// a lethal Focused Strike leaves Focus and `temp_focus` unchanged (#3160).
/// The live IsEnding projection is `engine::damage::damage_combat_is_ending`.
pub(crate) fn temp_focus(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = exact_int(
        ctx,
        "temp_focus",
        &[
            (CardId::FocusedStrike, 0, 1),
            (CardId::FocusedStrike, 1, 2),
            (CardId::Hotfix, 0, 2),
            (CardId::Hotfix, 1, 2),
        ],
    )?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    add_temp_focus(ctx, amount)
}

/// Tesla Coil's fixed-target Lightning passive walk.
///
/// `TeslaCoil/<OnPlay>d__5::MoveNext` RVA `0x3c2518` snapshots the Lightning
/// orbs (`OfType`/`ToList`, IL_00f1-00f6) after its attack, then for each one
/// awaits `OrbCmd::Passive(orb, target, false)` (IL_0142) and, upgraded, a
/// second one (IL_01bf). There is no card-level ending or dead-target check:
/// every command is issued, and each returns at its own `OrbCmd/<Passive>d__7`
/// IsOverOrEnding gate (`0x3ede90` IL_0022) or resolves a dead target as a
/// no-op inside [`lightning_passive_on`](crate::engine::orbs::lightning_passive_on)
/// (#3218). The former `history.over || target dead` early return is gone.
pub(crate) fn tesla_coil(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = exact_int(
        ctx,
        "tesla_coil",
        &[(CardId::TeslaCoil, 0, 1), (CardId::TeslaCoil, 1, 2)],
    )?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let lightning = ctx
        .state
        .orbs
        .as_slice()
        .iter()
        .filter(|orb| orb.kind() == OrbKind::Lightning)
        .count();
    for _ in 0..lightning {
        for _ in 0..count {
            crate::engine::orbs::lightning_passive_on(ctx.state, ctx.catalog, target, ctx.events)?;
        }
    }
    Ok(())
}

/// Thunder's additive `AfterOrbEvoked` amount.
///
/// Python: `_run_steps_inner` dispatch; the kind is read in
/// `_run_steps_inner`.
pub(crate) fn thunder(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    stack_power(
        ctx,
        "thunder",
        PowerId::Thunder,
        &[(CardId::Thunder, 0, 8), (CardId::Thunder, 1, 11)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::hot::{HotMonster, HotOrb, HotState, OrbKind};
    use crate::ids::MonsterKind;

    fn run(
        identity: CardIdentity,
        kind: StepKind,
        args: &[CompiledArg],
        state: &mut HotState,
    ) -> Result<(), EngineRefusal> {
        run_x(identity, kind, args, 0, state)
    }

    fn run_x(
        identity: CardIdentity,
        kind: StepKind,
        args: &[CompiledArg],
        x_value: i64,
        state: &mut HotState,
    ) -> Result<(), EngineRefusal> {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            target: None,
            selection: None,
            x_value,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    /// #3003: Voltaic channels its snapshotted tracked count (the Lightning it
    /// channels itself do not extend the loop, and each advances the count),
    /// channels nothing at a tracked zero, and refuses an untracked `-1`.
    #[test]
    fn voltaic_channels_its_tracked_snapshot_and_refuses_an_untracked_count() {
        for upgrade in [0, 1] {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.orbs.set_base_slots(3);
            state.orbs.set_slots(5);
            state.orbs.set_lightning_channeled(2);
            run(
                identity(CardId::Voltaic, upgrade),
                StepKind::ChannelVoltaic,
                &[],
                &mut state,
            )
            .unwrap();
            let lightning = HotOrb::from_parts(OrbKind::Lightning, None).unwrap();
            assert_eq!(state.orbs.as_slice(), &[lightning, lightning]);
            assert_eq!(state.orbs.lightning_channeled(), 4);

            let mut zero = HotState::at_defaults();
            zero.monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            zero.orbs.set_base_slots(3);
            zero.orbs.set_slots(3);
            zero.orbs.set_lightning_channeled(0);
            run(
                identity(CardId::Voltaic, upgrade),
                StepKind::ChannelVoltaic,
                &[],
                &mut zero,
            )
            .unwrap();
            assert!(zero.orbs.as_slice().is_empty());
            assert_eq!(zero.orbs.lightning_channeled(), 0);

            let mut untracked = HotState::at_defaults();
            untracked
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            untracked.orbs.set_base_slots(3);
            untracked.orbs.set_slots(3);
            assert_eq!(
                run(
                    identity(CardId::Voltaic, upgrade),
                    StepKind::ChannelVoltaic,
                    &[],
                    &mut untracked
                ),
                Err(EngineRefusal::UntrackedCounterNotModeled(
                    "lightning_channeled"
                ))
            );
            assert!(untracked.orbs.as_slice().is_empty());
        }
    }

    /// #3218: `TeslaCoil/<OnPlay>d__5` (0x3c2518) issues one
    /// `OrbCmd::Passive(orb, target, false)` per snapshotted Lightning orb
    /// (IL_0142) and a second when upgraded (IL_01bf), with no card-level
    /// ending or dead-target check. A dead target makes each command a no-op
    /// (`ApplyLightningDamage` 0x351cc0 wraps the explicit target without a
    /// CombatTargets draw; `CreatureCmd/<Damage>d__12` 0x3e96c8 IL_016d skips
    /// it), a target killed mid-walk absorbs the rest, the IsOverOrEnding
    /// window skips every command, and a live target takes all of them.
    #[test]
    fn tesla_coil_issues_every_passive_and_a_dead_target_absorbs_it() {
        fn tesla(state: &mut HotState, target: usize) -> Result<(), EngineRefusal> {
            let identity = identity(CardId::TeslaCoil, 1);
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
            crate::steps::apply_step(
                StepKind::TeslaCoil,
                &mut StepCtx {
                    state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 0,
                    target: Some(target),
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(2)],
                    events: &mut Vec::new(),
                },
            )
        }
        let fixture = |target_hp: i32, other: MonsterKind| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, target_hp));
            state.monsters_mut().push(HotMonster::new(other, 30));
            state.orbs.set_slots(2);
            state.orbs.set_orbs(vec![
                HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
                HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
            ]);
            state
        };
        let targets_draws =
            |state: &HotState| state.rng.get(crate::hot::RngStream::Targets).counter;

        // Dead target, living primary elsewhere: four no-op commands.
        let mut dead = fixture(0, MonsterKind::Toadpole);
        assert!(!crate::engine::damage::damage_combat_is_ending(&dead));
        let before = dead.clone();
        tesla(&mut dead, 0).unwrap();
        assert_eq!(dead, before, "a dead target absorbs every passive");

        // Killed by the second of four passives: the rest are no-ops.
        let mut killed = fixture(5, MonsterKind::Toadpole);
        let draws = targets_draws(&killed);
        tesla(&mut killed, 0).unwrap();
        assert!(killed.monsters[0].hp <= 0);
        assert_eq!(killed.monsters[1].hp, 30, "no passive retargets");
        assert_eq!(targets_draws(&killed), draws, "no CombatTargets draw");
        assert!(!killed.history.over);

        // Only primary dead, secondary targeted: IsOverOrEnding skips all.
        let mut ending = fixture(0, MonsterKind::GasBomb);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let before = ending.clone();
        tesla(&mut ending, 1).unwrap();
        assert_eq!(ending, before, "every command skips while ending");

        // Live control: all four passives hit the target.
        let mut live = fixture(40, MonsterKind::Toadpole);
        let draws = targets_draws(&live);
        tesla(&mut live, 0).unwrap();
        assert_eq!(live.monsters[0].hp, 40 - 4 * 3);
        assert_eq!(targets_draws(&live), draws);
    }

    #[test]
    fn synchronize_snapshots_distinct_kinds_into_temporary_focus() {
        let mut state = HotState::at_defaults();
        state.orbs.set_slots(3);
        state.orbs.set_orbs(vec![
            HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
            HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
            HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
        ]);

        run(
            identity(CardId::Synchronize, 1),
            StepKind::SynchronizeUniqueOrbFocusExact,
            &[CompiledArg::I(2)],
            &mut state,
        )
        .unwrap();

        assert_eq!(state.orbs.temp_focus(), 4);
    }

    #[test]
    fn focus_wrappers_pin_sources_and_skip_their_grant_while_ending() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        run(
            identity(CardId::Hyperbeam, 0),
            StepKind::HyperbeamFocusDownExact,
            &[CompiledArg::I(-3)],
            &mut state,
        )
        .unwrap();
        assert_eq!(state.orbs.temp_focus(), -3);

        state.history.over = true;
        run(
            identity(CardId::FocusedStrike, 0),
            StepKind::TempFocus,
            &[CompiledArg::I(1)],
            &mut state,
        )
        .unwrap();
        // #3160: PowerCmd.Apply<FocusedStrikePower> returns at IsEnding
        // (``PowerCmd/<Apply>d__1`1::MoveNext`` RVA 0x3ef988 IL_0020-0x0034).
        assert_eq!(state.orbs.temp_focus(), -3);

        run(
            identity(CardId::Hyperbeam, 0),
            StepKind::HyperbeamFocusDownExact,
            &[CompiledArg::I(-3)],
            &mut state,
        )
        .unwrap();
        assert_eq!(state.orbs.temp_focus(), -3);
    }

    #[test]
    fn front_evoke_counts_are_pinned_to_their_current_build_sources() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        assert_eq!(
            run(
                identity(CardId::Dualcast, 0),
                StepKind::EvokeFrontExact,
                &[CompiledArg::I(4)],
                &mut state,
            ),
            Err(EngineRefusal::MalformedArgs("evoke_front_exact"))
        );
    }

    fn channeled_lightning(state: &HotState) -> usize {
        state
            .orbs
            .as_slice()
            .iter()
            .filter(|orb| orb.kind() == OrbKind::Lightning)
            .count()
    }

    #[test]
    fn tempest_channels_x_lightning_and_one_more_when_upgraded() {
        // Tempest/<OnPlay>d__5 IL_00ab-00bd: numOfOrbs = X + (IsUpgraded ? 1 : 0).
        for (upgrade, x_value, expected) in [(0u8, 2i64, 2usize), (1, 2, 3), (0, 0, 0), (1, 0, 1)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            // A live monster, so `orbs::channel` does not see combat ending.
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 30));
            state.orbs.set_slots(5);
            run_x(
                identity(CardId::Tempest, upgrade),
                StepKind::ChannelX,
                &[],
                x_value,
                &mut state,
            )
            .unwrap();
            assert_eq!(
                channeled_lightning(&state),
                expected,
                "Tempest upgrade={upgrade} X={x_value}"
            );
        }
    }
}
