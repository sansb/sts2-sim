//! Card-step bodies for the `content/cards/resource_powers.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::damage::{apply_owner_strength, note_power};
use crate::engine::{EngineRefusal, Subject};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::FriendshipExact];

/// Friendship's serial owner-Strength loss and persistent max-energy power.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — pins the two FRIENDSHIP rows, serially
/// applies their negative owner Strength command, then applies one stack of
/// FriendshipPower. That persistent power contributes to every later
/// `_player_max_energy` reset.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The two exact rows first await the
/// negative owner `StrengthPower` command, then (unless combat is already
/// ending) stack one `FriendshipPower`. Friendship is a plain additive term
/// in the next owner turn's `ModifyMaxEnergy` fold.
pub(crate) fn friendship_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (strength, friendship) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Friendship, 0, [CompiledArg::I(-2), CompiledArg::I(1)]) => (-2, 1),
        (CardId::Friendship, 1, [CompiledArg::I(-1), CompiledArg::I(1)]) => (-1, 1),
        _ => return Err(EngineRefusal::MalformedArgs("friendship_exact")),
    };
    apply_owner_strength(ctx.state, strength, ctx.events)?;
    if ctx.state.history.over {
        return Ok(());
    }
    let stacked = ctx
        .state
        .powers
        .value(PowerId::Friendship)
        .checked_add(friendship)
        .ok_or(EngineRefusal::CounterOverflow("friendship"))?;
    ctx.state
        .powers
        .set(PowerId::Friendship, SlotWire::Int, stacked);
    note_power(ctx.events, Subject::Player, PowerId::Friendship, stacked);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, card_rows};
    use crate::engine::Event;
    use crate::engine::admission::IMPLEMENTED_PLAYER_POWERS;
    use crate::hot::HotState;
    use crate::ids::{CardId, PowerId};

    #[test]
    fn friendship_rows_and_new_power_identity_are_mechanically_pinned() {
        let rows = card_rows(CardId::Friendship);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].upgrade, 0);
        assert_eq!(rows[0].steps.len(), 1);
        assert_eq!(rows[0].steps[0].kind, StepKind::FriendshipExact);
        assert_eq!(rows[0].steps[0].args, &[Arg::I(-2), Arg::I(1)]);
        assert_eq!(rows[1].upgrade, 1);
        assert_eq!(rows[1].steps.len(), 1);
        assert_eq!(rows[1].steps[0].kind, StepKind::FriendshipExact);
        assert_eq!(rows[1].steps[0].args, &[Arg::I(-1), Arg::I(1)]);

        // Unit E connects the already-real neighboring primitives through a
        // distinct canonical Friendship identity and readable owner power.
        assert!(IMPLEMENTED_PLAYER_POWERS.contains(&PowerId::Strength));
        assert!(IMPLEMENTED_PLAYER_POWERS.contains(&PowerId::Demesne));
        assert!(IMPLEMENTED_PLAYER_POWERS.contains(&PowerId::Pyre));
        assert_eq!(PowerId::from_str("friendship"), Some(PowerId::Friendship));
        assert!(IMPLEMENTED.contains(&StepKind::FriendshipExact));
    }

    #[test]
    fn both_friendship_levels_apply_strength_then_stack_the_distinct_power() {
        for (upgrade, expected_strength) in [(0, 3), (1, 4)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::Friendship,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let mut state = HotState::at_defaults();
            state.powers.set(PowerId::Strength, SlotWire::Int, 5);
            state.powers.set(PowerId::Friendship, SlotWire::Int, 2);
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

            friendship_exact(&mut ctx).unwrap();

            assert_eq!(state.powers.value(PowerId::Strength), expected_strength);
            assert_eq!(state.powers.value(PowerId::Friendship), 3);
            assert_eq!(
                events,
                [
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::Strength,
                        amount: expected_strength,
                    },
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::Friendship,
                        amount: 3,
                    },
                ]
            );
        }
    }

    #[test]
    fn an_ended_combat_suppresses_both_serial_power_commands() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Friendship,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut state = HotState::at_defaults();
        state.history.over = true;
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
            args: catalog.args(step.args),
            events: &mut events,
        };

        friendship_exact(&mut ctx).unwrap();

        assert_eq!(state, before);
        assert!(events.is_empty());
    }
}
