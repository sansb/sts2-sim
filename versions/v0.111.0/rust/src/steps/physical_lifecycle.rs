//! Card-step bodies for the `content/cards/physical_lifecycle.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::damage::note_power;
use crate::engine::{EngineRefusal, Subject};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::Feral, StepKind::MasterPlanner];

/// Apply Feral and snapshot its live zero-energy Attack-start cursor.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Feral/<OnPlay>d__3::MoveNext` RVA `0x39dd00` IL `0x009e-0x00d6`
/// applies the exact canonical amount; Python `_run_steps_inner` (frozen, deleted #2827) owns
/// the matching writer. `FeralPower::AfterApplied` RVA
/// `0xa23c6` IL `0x0001-0x0027` counts the owner's zero-Energy Attack starts
/// into private Data only for a newly installed instance: `PowerCmd.Apply`
/// RVA `0x133a28` branches an existing stack at IL `0x0091-0x0121` through
/// `ModifyAmount` directly to IL `0x0647`, skipping the new-instance
/// `AfterApplied` call at IL `0x054c-0x05c9`. The result reader (`0xa23f8`)
/// changes only an eligible
/// natural Discard to Hand/Top; its completion (`0xa2479`) advances the
/// cursor, and `AfterSideTurnStart` (`0xa249f`) resets only that cursor.
pub(crate) fn feral(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("feral program"));
    };
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::Feral
        || ctx.catalog.args(program.args) != ctx.args
        || !matches!(ctx.args, [CompiledArg::I(1)])
    {
        return Err(EngineRefusal::MalformedArgs("feral"));
    }
    let old = ctx.state.powers.value(PowerId::Feral);
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("feral"))?;
    if old == 0 {
        if !ctx
            .state
            .fanouts
            .register_after_side_turn_start(PowerId::Feral)
        {
            return Err(EngineRefusal::CounterOverflow(
                "after-side-turn-start listener order",
            ));
        }
        if !ctx
            .state
            .fanouts
            .register_result_location_power(PowerId::Feral)
        {
            return Err(EngineRefusal::CounterOverflow("result-location order"));
        }
    }
    ctx.state.powers.set(PowerId::Feral, SlotWire::Int, updated);
    if old == 0 {
        ctx.state.powers.set(
            PowerId::FeralUsed,
            SlotWire::Int,
            i32::from(ctx.state.history.zero_energy_attack_plays_started_this_turn),
        );
    }
    note_power(ctx.events, Subject::Player, PowerId::Feral, updated);
    Ok(())
}

/// Stack Master Planner's persistent Skill-to-Sly listener.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) stacks the persistent power;
/// `_dispatch_after_card_played_power_object` observes any positive stack after every
/// Skill and synchronously applies Sly to the exact active physical card.
///
pub(crate) fn master_planner(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(
        (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args),
        (CardId::MasterPlanner, 0 | 1, [CompiledArg::I(1)])
    ) {
        return Err(EngineRefusal::MalformedArgs("master_planner"));
    }
    let current = ctx.state.powers.value(PowerId::MasterPlanner);
    let updated = current
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("master planner"))?;
    crate::engine::play::prepare_after_card_played_scalar_write(
        ctx.state,
        PowerId::MasterPlanner,
        current,
        updated,
    )?;
    ctx.state
        .powers
        .set(PowerId::MasterPlanner, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::MasterPlanner, updated);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::hot::HotState;

    #[test]
    fn lifecycle_family_claims_feral_and_master_planner() {
        assert_eq!(IMPLEMENTED, &[StepKind::Feral, StepKind::MasterPlanner]);
    }

    #[test]
    fn every_physical_lifecycle_carrier_has_the_exact_manifest_state() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| matches!(row.id, CardId::Feral | CardId::MasterPlanner))
            .collect();

        assert_eq!(carriers.len(), 4, "both upgrade rows for both cards");
        for row in carriers {
            assert!(row.is_power);
            assert_eq!(row.steps.len(), 1);
            assert_eq!(row.steps[0].args, &[Arg::I(1)]);
            assert!(matches!(
                row.steps[0].kind,
                StepKind::Feral | StepKind::MasterPlanner
            ));
            assert_eq!(
                IMPLEMENTED.contains(&row.steps[0].kind),
                matches!(row.id, CardId::Feral | CardId::MasterPlanner)
            );
        }
    }

    #[test]
    fn lifecycle_power_identities_are_exact() {
        assert_eq!(PowerId::from_str("feral"), Some(PowerId::Feral));
        assert_eq!(PowerId::from_str("feral_used"), Some(PowerId::FeralUsed));
        assert_eq!(
            PowerId::from_str("master_planner"),
            Some(PowerId::MasterPlanner)
        );
    }

    #[test]
    fn intrinsic_sly_rows_are_a_closed_independently_refused_census() {
        let actual: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| row.sly)
            .map(|row| (row.id, row.upgrade))
            .collect();
        let expected = [
            (CardId::Abrasive, 0),
            (CardId::Abrasive, 1),
            (CardId::FlickFlack, 0),
            (CardId::FlickFlack, 1),
            (CardId::Reflex, 0),
            (CardId::Reflex, 1),
            (CardId::Ricochet, 0),
            (CardId::Ricochet, 1),
            (CardId::Sneaky, 0),
            (CardId::Sneaky, 1),
            (CardId::Tactician, 0),
            (CardId::Tactician, 1),
            (CardId::Untouchable, 0),
            (CardId::Untouchable, 1),
        ];

        assert_eq!(actual, expected);
        assert!(
            CARD_ROWS
                .iter()
                .filter(|row| matches!(row.id, CardId::MasterPlanner))
                .all(|row| !row.sly),
            "Master Planner supplies a local keyword; it is not intrinsically Sly"
        );
    }

    #[test]
    fn master_planner_stacks_exactly_and_overflow_is_atomic() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::MasterPlanner,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = catalog.args(catalog.steps(&spec)[0].args);
        let mut state = HotState::at_defaults();
        let mut events = Vec::new();
        master_planner(&mut StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 0,
            selection: None,
            x_value: 0,
            args,
            target: None,
            events: &mut events,
        })
        .unwrap();
        assert_eq!(state.powers.value(PowerId::MasterPlanner), 1);
        assert_eq!(
            events,
            [crate::engine::Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::MasterPlanner,
                amount: 1,
            }]
        );

        state
            .powers
            .set(PowerId::MasterPlanner, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let before_events = events.clone();
        assert_eq!(
            master_planner(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                selection: None,
                x_value: 0,
                args,
                target: None,
                events: &mut events,
            }),
            Err(EngineRefusal::CounterOverflow("master planner"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn feral_first_apply_snapshots_but_stacking_preserves_the_private_cursor() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Feral,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = catalog.args(catalog.steps(&spec)[0].args);
        let mut state = HotState::at_defaults();
        state.history.zero_energy_attack_plays_started_this_turn = 3;
        let mut events = Vec::new();

        feral(&mut StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            selection: None,
            x_value: 0,
            args,
            target: None,
            events: &mut events,
        })
        .unwrap();
        assert_eq!(state.powers.value(PowerId::Feral), 1);
        assert_eq!(state.powers.value(PowerId::FeralUsed), 3);
        assert_eq!(
            state.fanouts.result_location_power_order(),
            [PowerId::Feral]
        );

        state.history.zero_energy_attack_plays_started_this_turn = 5;
        feral(&mut StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            selection: None,
            x_value: 0,
            args,
            target: None,
            events: &mut events,
        })
        .unwrap();
        assert_eq!(state.powers.value(PowerId::Feral), 2);
        assert_eq!(
            state.powers.value(PowerId::FeralUsed),
            3,
            "PowerCmd.Apply stacks through ModifyAmount without AfterApplied"
        );
        assert_eq!(
            state.fanouts.result_location_power_order(),
            [PowerId::Feral]
        );

        let before = state.clone();
        let events_before = events.clone();
        assert_eq!(
            feral(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                selection: None,
                x_value: 1,
                args,
                target: None,
                events: &mut events,
            }),
            Err(EngineRefusal::MalformedArgs("feral"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }
}
