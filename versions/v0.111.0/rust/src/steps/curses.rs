//! Card-step bodies for the `content/cards/curses.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1504): 1 of 1 ported

use super::StepCtx;
use crate::catalog::{CardSpec, Catalog, CompiledArg, CompiledStep};
use crate::content_tables::card_row;
use crate::engine::EngineRefusal;
use crate::hot::PileId;
use crate::ids::{CardId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::TurnEndGoldLossExact];

/// Validate and return the sole turn-end-in-hand step program admitted by
/// this family.
///
/// The turn engine calls this before moving any snapshotted wrapper, then the
/// body calls it again at dispatch. That gives classification and execution
/// one shape authority while keeping malformed/foreign programs fail-closed
/// before pile or gold mutation.
pub(crate) fn exact_turn_end_gold_loss_step(
    catalog: &Catalog,
    spec: &CardSpec,
) -> Result<CompiledStep, EngineRefusal> {
    let exact_row = card_row(CardId::Debt, 0).expect("generated Debt+0 row");
    let steps = catalog.steps(spec);
    match steps {
        [step]
            if matches!((spec.identity.id, spec.identity.upgrade), (CardId::Debt, 0))
                && spec.row == exact_row
                && step.kind == StepKind::TurnEndGoldLossExact
                && catalog.args(step.args) == [CompiledArg::I(10)] =>
        {
            Ok(*step)
        }
        _ => Err(EngineRefusal::MalformedArgs("turn_end_gold_loss_exact")),
    }
}

/// `turn_end_gold_loss_exact` — Debt's exact turn-end-in-hand gold loss.
///
/// Python: `_has_turn_end_in_hand_effect` (frozen, deleted #2827) recognizes only the complete
/// `(("turn_end_gold_loss_exact", 10),)` program. Then
/// `_run_turn_end_in_hand_body` requires exact `DEBT+0`, subtracts
/// `min(10, live gold)` at this wrapper's serial position, and returns to the
/// ordinary Play-to-Discard route.
pub(crate) fn turn_end_gold_loss_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    exact_turn_end_gold_loss_step(ctx.catalog, ctx.spec)?;
    if ctx.args != [CompiledArg::I(10)] {
        return Err(EngineRefusal::MalformedArgs("turn_end_gold_loss_exact"));
    }

    let matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .filter(|card| card.uid == ctx.source_uid)
        .count();
    if matches != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches,
        });
    }

    let lost = 10.min(ctx.state.gold);
    ctx.state.gold = ctx
        .state
        .gold
        .checked_sub(lost)
        .ok_or(EngineRefusal::CounterOverflow("turn_end_gold_loss_exact"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::hot::{HotCard, HotState};

    #[test]
    fn curses_family_manifest_is_exactly_the_debt_body() {
        assert_eq!(IMPLEMENTED, [StepKind::TurnEndGoldLossExact]);
    }

    #[test]
    fn debt_is_the_single_non_vacuous_carrier_with_exact_operand_shape() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::TurnEndGoldLossExact)
            })
            .collect();

        assert_eq!(carriers.len(), 1, "the exact generated Debt+0 row");
        let row = carriers[0];
        assert_eq!((row.id, row.upgrade), (CardId::Debt, 0));
        assert_eq!(row.steps.len(), 1);
        assert_eq!(row.steps[0].kind, StepKind::TurnEndGoldLossExact);
        assert_eq!(row.steps[0].args, &[Arg::I(10)]);
        assert_eq!(row.cost, -1);
        assert!(!row.playable);
        assert_eq!(row.target_type, "None");
        assert!(IMPLEMENTED.contains(&row.steps[0].kind));
    }

    #[test]
    fn debt_body_uses_live_gold_at_zero_below_and_above_ten() {
        for (gold, expected) in [(0, 0), (7, 0), (10, 0), (23, 13)] {
            let identity = CardIdentity {
                id: CardId::Debt,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = exact_turn_end_gold_loss_step(&catalog, &spec).unwrap();
            let mut state = HotState::at_defaults();
            state.gold = gold;
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 41,
                atom,
                flags: 0,
            });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 41,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };

            turn_end_gold_loss_exact(&mut ctx).unwrap();
            assert_eq!(ctx.state.gold, expected, "starting gold {gold}");
            assert!(ctx.events.is_empty());
        }
    }

    #[test]
    fn debt_body_refuses_wrong_args_and_ambiguous_source_without_mutation() {
        let identity = CardIdentity {
            id: CardId::Debt,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.gold = 19;
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
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(9)],
            events: &mut events,
        };
        assert_eq!(
            turn_end_gold_loss_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("turn_end_gold_loss_exact"))
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());

        ctx.args = &[CompiledArg::I(10)];
        ctx.state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
        let before = ctx.state.clone();
        assert_eq!(
            turn_end_gold_loss_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique { uid: 7, matches: 2 })
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());
    }
}
