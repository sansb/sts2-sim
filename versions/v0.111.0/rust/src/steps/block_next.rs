//! Card-step bodies for the `content/cards/block_next.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Unit-E status (#1316 + #1363): 1 of 1 ported
//!
//! Issue #1495 supplied the generic delayed-power lifecycle and pure powered-
//! block modifier preview. Unit E joins those primitives to the six exact
//! identity/level rows here; engine primitives alone still never widen
//! StepKind admission.

use super::StepCtx;
use crate::catalog::CompiledArg;
use crate::engine::EngineRefusal;
use crate::engine::damage::{
    apply_block_next_turn, gain_powered_card_block, preview_powered_card_block,
};
use crate::ids::{CardId, StepKind, StepWord};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::BlockNextBody];

/// Exact Dodge and Roll / Glitterstream / Prolong delayed-block body.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Dodge and Roll captures the returned
/// final powered Block; Glitterstream previews `_modify_powered_card_block` without history or hooks before gaining its immediate Block; and
/// Prolong snapshots live player Block. Each then separately awaits
/// `_apply_delayed_block_power`.
///
/// The saved power is consumed by `begin_player_turn`'s acquisition-ordered
/// AfterBlockCleared walk (frozen Python, deleted #2827).
///
/// The identity, upgrade, and complete argument tuple are pinned together so
/// a future generated row cannot inherit one of the three card-specific
/// operand rules accidentally. Applying the delayed power is a separately
/// awaited command and is therefore suppressed when the immediate powered
/// block's Juggernaut suffix ends combat.
///
/// The delayed Block is `PowerCmd.Apply<BlockNextTurnPower>`: Prolong
/// `0x3b3c94` IL_003e, Glitterstream `0x3a1bcc` IL_018f. `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn block_next_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let delayed = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::DodgeAndRoll, 0, [CompiledArg::Word(StepWord::DodgeRoll), CompiledArg::I(4)]) => {
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, 4, ctx.events)?
        }
        (CardId::DodgeAndRoll, 1, [CompiledArg::Word(StepWord::DodgeRoll), CompiledArg::I(6)]) => {
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, 6, ctx.events)?
        }
        (
            CardId::Glitterstream,
            0,
            [
                CompiledArg::Word(StepWord::Glitterstream),
                CompiledArg::I(11),
                CompiledArg::I(5),
            ],
        ) => {
            let delayed = preview_powered_card_block(ctx.state, ctx.spec, 5)?;
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, 11, ctx.events)?;
            delayed
        }
        (
            CardId::Glitterstream,
            1,
            [
                CompiledArg::Word(StepWord::Glitterstream),
                CompiledArg::I(13),
                CompiledArg::I(7),
            ],
        ) => {
            let delayed = preview_powered_card_block(ctx.state, ctx.spec, 7)?;
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, 13, ctx.events)?;
            delayed
        }
        (CardId::Prolong, 0 | 1, [CompiledArg::Word(StepWord::Prolong)]) => ctx.state.block,
        _ => return Err(EngineRefusal::MalformedArgs("block_next_body")),
    };
    if !crate::engine::damage::damage_combat_is_ending(ctx.state) {
        apply_block_next_turn(ctx.state, delayed, ctx.events)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::CARD_ROWS;
    use crate::engine::Event;
    use crate::hot::{HotMonster, HotState};
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;

    fn run(id: CardId, upgrade: u8, state: &mut HotState) -> Result<Vec<Event>, EngineRefusal> {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade,
                enchantment: None,
            })
            .expect("the generated row compiles");
        let catalog = builder.build();
        let spec = *catalog.spec(atom).expect("the exact card is interned");
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args);
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
        block_next_body(&mut ctx)?;
        Ok(events)
    }

    #[test]
    fn engine_primitives_and_family_manifest_meet_at_the_exact_body() {
        assert_eq!(IMPLEMENTED, &[StepKind::BlockNextBody]);
        assert_eq!(
            PowerId::from_str("block_next_turn"),
            Some(PowerId::BlockNextTurn)
        );
    }

    #[test]
    fn every_generated_block_next_carrier_is_mechanically_admitted() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::BlockNextBody)
            })
            .collect();

        assert_eq!(carriers.len(), 6, "three exact cards at both levels");
        for row in carriers {
            assert_eq!(row.steps.len(), 1, "{} has one atomic body", row.name);
            assert_eq!(row.steps[0].kind, StepKind::BlockNextBody);
            assert!(IMPLEMENTED.contains(&row.steps[0].kind));
        }
    }

    #[test]
    fn all_six_rows_execute_their_exact_delayed_operand_rule() {
        for (id, upgrade, expected_block, expected_delayed) in [
            (CardId::DodgeAndRoll, 0, 4, 4),
            (CardId::DodgeAndRoll, 1, 6, 6),
            (CardId::Glitterstream, 0, 11, 5),
            (CardId::Glitterstream, 1, 13, 7),
            (CardId::Prolong, 0, 9, 9),
            (CardId::Prolong, 1, 9, 9),
        ] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            if id == CardId::Prolong {
                state.block = 9;
            }
            run(id, upgrade, &mut state).unwrap();
            assert_eq!(state.block, expected_block, "{id:?}+{upgrade}");
            assert_eq!(
                state.powers.value(PowerId::BlockNextTurn),
                expected_delayed,
                "{id:?}+{upgrade}"
            );
            assert_eq!(
                state.fanouts.after_block_cleared_order(),
                &[PowerId::BlockNextTurn]
            );
        }
    }

    #[test]
    fn glitterstream_previews_before_the_immediate_gain_mutates_history() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Unmovable, SlotWire::Int, 1);

        run(CardId::Glitterstream, 0, &mut state).unwrap();

        assert_eq!(state.block, 22, "immediate raw 11 is doubled");
        assert_eq!(state.history.card_block_gains, 1);
        assert_eq!(
            state.powers.value(PowerId::BlockNextTurn),
            10,
            "the raw-5 preview saw the same pre-gain doubling window"
        );
    }

    #[test]
    fn immediate_juggernaut_lethal_suppresses_the_delayed_power_command() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );

        run(CardId::DodgeAndRoll, 0, &mut state).unwrap();

        assert!(state.history.over);
        assert_eq!(state.powers.value(PowerId::BlockNextTurn), 0);
        assert!(state.fanouts.after_block_cleared_order().is_empty());
    }

    #[test]
    fn malformed_identity_argument_pair_refuses_before_mutation() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::DodgeAndRoll,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.block = 3;
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
            args: &[CompiledArg::Word(StepWord::DodgeRoll), CompiledArg::I(6)],
            events: &mut events,
        };

        assert_eq!(
            block_next_body(&mut ctx),
            Err(EngineRefusal::MalformedArgs("block_next_body"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #3515: the delayed Block is `PowerCmd.Apply<BlockNextTurnPower>`
    /// (Prolong `0x3b3c94` IL_003e), which returns at `IsEnding`
    /// (`<Apply>d__1`1` `0x3ef988` IL_0025). While the combat is ending before
    /// the over latch Prolong adds nothing; the Adaptable-vetoed control does.
    #[test]
    fn prolong_skips_its_delayed_block_while_combat_is_ending_before_the_over_latch() {
        let mut template = HotState::at_defaults();
        template.block = 9;
        crate::engine::damage::assert_ending_window_gate(&template, "Prolong", |s, _| {
            run(CardId::Prolong, 0, s).map(|_| ())
        });
    }
}
