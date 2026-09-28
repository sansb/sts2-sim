//! Card-step bodies for the `content/cards/necrobinder_uncommon.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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

use super::{StepCtx, shared::calculated_operand};
use crate::catalog::{CardIdentity, CompiledArg};
use crate::engine::damage::{apply_card_monster_debuff, gain_powered_card_block};
use crate::engine::{EngineRefusal, fire_hook};
use crate::hooks::HookEvent;
use crate::hot::MiseryToken;
use crate::ids::{CardId, PowerId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::CalculatedDoomExact,
    StepKind::DeathsDoorExact,
    StepKind::DirgeXExact,
];

/// No Escape's exact live-Doom multiplier.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). PowerCmd.Apply's ending and target
/// liveness gates precede calculation; each replay independently re-reads the
/// target's current Doom before applying the new card-sourced stack.
pub(crate) fn calculated_doom_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.state.history.over {
        return Ok(());
    }
    let Some(target) = ctx.target else {
        return Ok(());
    };
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if monster.hp <= 0 {
        return Ok(());
    }
    let amount = calculated_operand(ctx, StepKind::CalculatedDoomExact)?;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("calculated_doom_exact"))?;
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Doom,
        MiseryToken::Doom,
        amount,
        ctx.events,
    )
}

/// Death's Door's exact owned-Doom replay body.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The body issues one powered card-block
/// command, plus two more when the player has applied Doom this turn. Each
/// command is serial: it re-enters the live powered-block fold and its
/// `AfterBlockGained` fan-out, and an ending result suppresses later grants.
pub(crate) fn deaths_door_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (block, repeat) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::DeathsDoor, 0, [CompiledArg::I(6), CompiledArg::I(2)]) => (6, 2),
        (CardId::DeathsDoor, 1, [CompiledArg::I(7), CompiledArg::I(2)]) => (7, 2),
        _ => return Err(EngineRefusal::MalformedArgs("deaths_door_exact")),
    };
    let gains = if ctx.state.history.doom_applied_by_player_this_turn {
        1 + repeat
    } else {
        1
    };
    for _ in 0..gains {
        if ctx.state.history.over {
            break;
        }
        gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, block, ctx.events)?;
        fire_hook(
            ctx.catalog,
            HookEvent::AfterBlockGained,
            ctx.state,
            ctx.events,
        )?;
    }
    Ok(())
}

/// `dirge_x_exact` — exact X-fold Osty summon and Soul generation.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) serially summons Osties, constructs Soul
/// cards, and inserts each into Draw or Random using the shuffle stream. The
/// full transaction is rehearsed on a clone before summon or RNG mutation.
pub(crate) fn dirge_x_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (summon, soul_upgrade) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Dirge, 0, [CompiledArg::I(3), CompiledArg::I(0)]) => (3, 0),
        (CardId::Dirge, 1, [CompiledArg::I(4), CompiledArg::I(1)]) => (4, 1),
        _ => return Err(EngineRefusal::MalformedArgs("dirge_x_exact")),
    };
    let repeats: usize = ctx
        .x_value
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("dirge_x_exact"))?;

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        repeats: usize,
        summon: i32,
        soul_upgrade: u8,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        for _ in 0..repeats {
            crate::engine::summon_osty(state, summon, "Dirge Osty summon")?;
        }
        for _ in 0..repeats {
            crate::engine::cards::inject_generated_draw_random(
                state,
                catalog,
                CardIdentity {
                    id: CardId::Soul,
                    upgrade: soul_upgrade,
                    enchantment: None,
                },
                events,
            )?;
        }
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(
        &mut probe,
        ctx.catalog,
        repeats,
        summon,
        soul_upgrade,
        &mut Vec::new(),
    )?;
    apply(
        ctx.state,
        ctx.catalog,
        repeats,
        summon,
        soul_upgrade,
        ctx.events,
    )
}

/// `legion_of_bone_exact` — not modeled. **Escalated (#1330).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) and `_apply_legion_of_bone_exact`
/// snapshot living same-side players, then summon or grow each recipient's
/// Osty in native player order. Missing primitive: multiplayer player rosters
/// and remote Osty state/listeners; the hot state represents only one player.
pub(crate) fn legion_of_bone_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(
        StepKind::LegionOfBoneExact,
    ))
}

/// `soulbound_exact` — not modeled. **Escalated (#1330).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) resolves an AnyAlly target, preserves the
/// `(recipient, applier)` power instance/order, and listens to generated Souls
/// for recursive ownership-aware stacks. Missing primitives: multiplayer ally
/// targeting, remote powers, and the generated-card-for-combat listener.
/// ESCALATED-ON: power-variant-absent(PowerId::Soulbound)
pub(crate) fn soulbound_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::SoulboundExact))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder, CompiledArg};
    use crate::content_tables::CARD_ROWS;
    use crate::engine::capability_manifest;
    use crate::hot::{HotMonster, HotState, MiseryOrder};
    use crate::ids::{CardId, MonsterKind, StepWord};
    use crate::powers::SlotWire;

    fn run(
        state: &mut HotState,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id: CardId::NoEscape,
            upgrade: 0,
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
        calculated_doom_exact(&mut ctx)
    }

    fn run_deaths_door(
        state: &mut HotState,
        id: CardId,
        upgrade: u8,
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
        deaths_door_exact(&mut ctx)
    }

    #[test]
    fn manifest_and_carrier_census_pin_the_exact_three_of_five_family_surface() {
        const FAMILY_KINDS: [StepKind; 5] = [
            StepKind::CalculatedDoomExact,
            StepKind::DeathsDoorExact,
            StepKind::DirgeXExact,
            StepKind::LegionOfBoneExact,
            StepKind::SoulboundExact,
        ];
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::CalculatedDoomExact,
                StepKind::DeathsDoorExact,
                StepKind::DirgeXExact,
            ]
        );
        for kind in IMPLEMENTED {
            assert!(capability_manifest().steps.contains(kind));
        }

        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter_map(|row| {
                let kind = row
                    .steps
                    .iter()
                    .find(|step| FAMILY_KINDS.contains(&step.kind))?
                    .kind;
                Some((row.id, row.upgrade, kind))
            })
            .collect();
        assert_eq!(
            carriers,
            vec![
                (CardId::DeathsDoor, 0, StepKind::DeathsDoorExact),
                (CardId::DeathsDoor, 1, StepKind::DeathsDoorExact),
                (CardId::Dirge, 0, StepKind::DirgeXExact),
                (CardId::Dirge, 1, StepKind::DirgeXExact),
                (CardId::LegionOfBone, 0, StepKind::LegionOfBoneExact),
                (CardId::LegionOfBone, 1, StepKind::LegionOfBoneExact),
                (CardId::NoEscape, 0, StepKind::CalculatedDoomExact),
                (CardId::NoEscape, 1, StepKind::CalculatedDoomExact),
                (CardId::Soulbound, 0, StepKind::SoulboundExact),
                (CardId::Soulbound, 1, StepKind::SoulboundExact),
            ]
        );
        let admitted: Vec<_> = carriers
            .iter()
            .copied()
            .filter(|(_, _, kind)| crate::steps::is_implemented(*kind))
            .collect();
        assert_eq!(
            admitted,
            vec![
                (CardId::DeathsDoor, 0, StepKind::DeathsDoorExact),
                (CardId::DeathsDoor, 1, StepKind::DeathsDoorExact),
                (CardId::Dirge, 0, StepKind::DirgeXExact),
                (CardId::Dirge, 1, StepKind::DirgeXExact),
                (CardId::NoEscape, 0, StepKind::CalculatedDoomExact),
                (CardId::NoEscape, 1, StepKind::CalculatedDoomExact),
            ]
        );
    }

    #[test]
    fn dirge_completes_all_summons_before_random_draw_soul_insertions() {
        let identity = CardIdentity {
            id: CardId::Dirge,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .intern(CardIdentity {
                id: CardId::Soul,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = [CompiledArg::I(3), CompiledArg::I(0)];
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.next_card_uid = 10;
        let rng_before = state.rng.get(crate::hot::RngStream::Rng).counter;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 2,
            args: &args,
            events: &mut events,
        };

        dirge_x_exact(&mut ctx).unwrap();

        assert_eq!(
            ctx.state
                .fanouts
                .pet()
                .osty()
                .map(|osty| (osty.hp(), osty.max_hp())),
            Some((6, 6))
        );
        assert_eq!(ctx.state.history.owner_generated_cards_combat, 2);
        assert_eq!(ctx.state.next_card_uid, 12);
        assert_eq!(
            ctx.state.rng.get(crate::hot::RngStream::Rng).counter,
            rng_before + 2
        );
        assert_eq!(ctx.state.piles.get(crate::hot::PileId::Draw).len(), 2);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn deaths_door_replays_separate_live_powered_block_commands_after_owned_doom() {
        let args = [CompiledArg::I(6), CompiledArg::I(2)];
        let mut ordinary = HotState::at_defaults();
        run_deaths_door(&mut ordinary, CardId::DeathsDoor, 0, &args).unwrap();
        assert_eq!(ordinary.block, 6);
        assert_eq!(ordinary.history.card_block_gains, 1);

        let mut replayed = HotState::at_defaults();
        replayed.history.doom_applied_by_player_this_turn = true;
        replayed.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        // All three `GainBlock` calls pass the one `cardPlay`
        // (`DeathsDoor/<OnPlay>d__9::MoveNext` RVA `0x397230` IL_00ed–00f3
        // inside the IL_00a5–0169 loop), and Unmovable's filter excludes the
        // current CardPlay's own entries (RVA `0x34a534` IL_0051–0062), so
        // under a live play every gain doubles (#3043).
        crate::engine::play::with_test_active_play(0, || {
            run_deaths_door(&mut replayed, CardId::DeathsDoor, 0, &args).unwrap();
        });
        assert_eq!(replayed.block, 36); // 12, 12, 12.
        assert_eq!(replayed.history.card_block_gains, 3);
        // Without a CardPlay (native `cardPlay` null) nothing is excluded
        // and the window closes after the first gain: 12, then 6, then 6.
        let mut direct = HotState::at_defaults();
        direct.history.doom_applied_by_player_this_turn = true;
        direct.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        run_deaths_door(&mut direct, CardId::DeathsDoor, 0, &args).unwrap();
        assert_eq!(direct.block, 24);
        assert_eq!(direct.history.card_block_gains, 3);

        let mut upgraded = HotState::at_defaults();
        upgraded.history.doom_applied_by_player_this_turn = true;
        run_deaths_door(
            &mut upgraded,
            CardId::DeathsDoor,
            1,
            &[CompiledArg::I(7), CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(upgraded.block, 21);
        assert_eq!(upgraded.history.card_block_gains, 3);
    }

    #[test]
    fn deaths_door_validates_its_exact_source_and_args_without_mutation() {
        let mut state = HotState::at_defaults();
        let snapshot = state.clone();
        assert_eq!(
            run_deaths_door(
                &mut state,
                CardId::DeathsDoor,
                0,
                &[CompiledArg::I(7), CompiledArg::I(2)],
            ),
            Err(EngineRefusal::MalformedArgs("deaths_door_exact"))
        );
        assert_eq!(state, snapshot);
        assert_eq!(
            run_deaths_door(
                &mut state,
                CardId::NoEscape,
                0,
                &[CompiledArg::I(6), CompiledArg::I(2)],
            ),
            Err(EngineRefusal::MalformedArgs("deaths_door_exact"))
        );
        assert_eq!(state, snapshot);

        state.history.over = true;
        run_deaths_door(
            &mut state,
            CardId::DeathsDoor,
            0,
            &[CompiledArg::I(6), CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.block, 0);
        assert_eq!(state.history.card_block_gains, 0);
    }

    #[test]
    fn no_escape_recomputes_live_doom_and_obeys_entry_gates() {
        let args = [
            CompiledArg::Word(StepWord::TargetDoomTens),
            CompiledArg::I(10),
            CompiledArg::I(5),
        ];
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 25);
        state.monsters_mut()[0].misery_debuff_order =
            MiseryOrder::from_tokens(vec![MiseryToken::Doom]);
        run(&mut state, Some(0), &args).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 45);
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            &[MiseryToken::Doom]
        );
        assert!(state.history.doom_applied_by_player_this_turn);

        state.monsters_mut()[0].hp = 0;
        run(&mut state, Some(0), &args).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 45);
        run(&mut state, None, &args).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 45);

        state.monsters_mut()[0].hp = 50;
        state.history.over = true;
        run(
            &mut state,
            Some(0),
            &[
                CompiledArg::Word(StepWord::PlayerBlock),
                CompiledArg::I(10),
                CompiledArg::I(5),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 45);
    }
}
