//! Card-step bodies for the `content/cards/single_pile_selection.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::EngineRefusal;
use crate::engine::draw::{DrawSource, draw_cards};
use crate::ids::{CardId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::GlimmerExact];

/// `("glimmer_exact", cards, 1)` — awaited Draw, then the Glimmer selector.
///
/// Python: `_run_steps_inner`'s `glimmer_exact` branch delegates the exact
/// three/four-card command Draw before `_glimmer_after_draw` reads the fresh
/// live Hand. The shared continuation gate owns that suffix: this body returns
/// [`EngineRefusal::ContinuationNotModeled`] only after Draw completes, which
/// [`crate::engine::play`] recognizes for this StepKind and turns into the
/// exact synchronous-or-suspended Glimmer selection.
///
/// Current-build IL: `Glimmer/<OnPlay>d__4::MoveNext` at RVA `0x3a173c` in
/// the installed and archived v0.111.0 DLL
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`).
pub(crate) fn glimmer_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let cards = i64::from(3 + ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::Glimmer
        || ctx.args != [CompiledArg::I(cards), CompiledArg::I(1)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("glimmer_exact"));
    }
    let cards =
        usize::try_from(cards).map_err(|_| EngineRefusal::MalformedArgs("glimmer_exact"))?;
    // Under the owning Glimmer CardPlay frame the awaited Draw is an owned
    // tail (#2667): a selecting Hellraiser child or Stratagem reshuffle
    // suspends it, and the selector freezes from the fresh live Hand only
    // after the Draw fully returns — at the shared suffix below when the
    // Draw completes inline, or at `resume_glimmer_after_draw` when it was
    // parked. Any other caller keeps the synchronous command Draw, which
    // still refuses a suspending Draw at the admission gate.
    let under_own_cardplay = matches!(
        ctx.state.frames.top(),
        Some(crate::frame::Frame::CardPlay { record })
            if ctx
                .state
                .frames
                .card_play(record)
                .is_some_and(|owner| {
                    owner.uid == ctx.source_uid
                        && owner.stage == crate::hot::CardPlayStage::Body
                        && !owner.pending_choice
                })
    );
    if under_own_cardplay {
        if crate::engine::play::draw_cardplay_owned_tail(ctx, cards)? {
            return Ok(());
        }
    } else {
        draw_cards(
            ctx.state,
            ctx.catalog,
            cards,
            DrawSource::Command,
            ctx.events,
        )?;
    }
    Err(EngineRefusal::ContinuationNotModeled)
}

/// Glimmer's awaited-Draw return program (#2667).
///
/// The selector freezes from the fresh live Hand here — never before the
/// child Draw — so an inner Draw suspension is never mislabeled as the outer
/// Glimmer selection. An auto-resolvable Hand completes inline; otherwise
/// the exact-one choice parks on the owning CardPlay.
pub(crate) fn resume_glimmer_after_draw(
    state: &mut crate::hot::HotState,
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if glimmer_program(catalog, spec).is_none() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    match crate::engine::selection::execute_glimmer(state, catalog, events)? {
        crate::engine::selection::SelectDisposition::Complete => Ok(()),
        crate::engine::selection::SelectDisposition::Suspend => {
            crate::engine::play::park_glimmer_select_after_draw(state)
        }
    }
}

/// Glimmer's exact program at both upgrades: one `glimmer_exact` step
/// drawing three (four upgraded) and selecting one.
fn glimmer_program(
    catalog: &crate::catalog::Catalog,
    spec: &crate::catalog::CardSpec,
) -> Option<()> {
    let cards = i64::from(3 + spec.identity.upgrade);
    if spec.identity.id != CardId::Glimmer
        || !matches!(spec.identity.upgrade, 0 | 1)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::GlimmerExact
        && catalog.args(step.args) == [CompiledArg::I(cards), CompiledArg::I(1)])
    .then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, PileId};
    use crate::ids::MonsterKind;

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn card(catalog: &Catalog, id: CardId, upgrade: u8, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog.atom(&identity(id, upgrade)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        }
    }

    fn fixture(upgrade: u8) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Glimmer, upgrade)).unwrap();
        builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 8;
        state.piles.get_mut(PileId::Play).make_mut().push(card(
            &catalog,
            CardId::Glimmer,
            upgrade,
            1,
        ));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..8).map(|uid| card(&catalog, CardId::StrikeIronclad, 0, uid)));
        (state, catalog)
    }

    fn run(
        state: &mut HotState,
        catalog: &Catalog,
        upgrade: u8,
        args: &[CompiledArg],
    ) -> (Result<(), EngineRefusal>, Vec<crate::engine::Event>) {
        run_with_operands(state, catalog, upgrade, args, None, None, 0)
    }

    fn run_with_operands(
        state: &mut HotState,
        catalog: &Catalog,
        upgrade: u8,
        args: &[CompiledArg],
        target: Option<usize>,
        selection: Option<u32>,
        x_value: i64,
    ) -> (Result<(), EngineRefusal>, Vec<crate::engine::Event>) {
        let spec = catalog
            .spec(catalog.atom(&identity(CardId::Glimmer, upgrade)).unwrap())
            .unwrap();
        let mut events = Vec::new();
        let result = glimmer_exact(&mut StepCtx {
            state,
            catalog,
            spec,
            source_uid: 1,
            target,
            selection,
            x_value,
            args,
            events: &mut events,
        });
        (result, events)
    }

    #[test]
    fn both_levels_draw_before_signaling_the_shared_suffix() {
        assert_eq!(IMPLEMENTED, &[StepKind::GlimmerExact]);
        for (upgrade, count) in [(0, 3), (1, 4)] {
            let (mut state, catalog) = fixture(upgrade);
            let args = [CompiledArg::I(count), CompiledArg::I(1)];
            let (result, events) = run(&mut state, &catalog, upgrade, &args);

            assert_eq!(result, Err(EngineRefusal::ContinuationNotModeled));
            assert_eq!(state.piles.get(PileId::Hand).len(), count as usize);
            assert_eq!(state.piles.get(PileId::Draw).len(), 6 - count as usize);
            assert_eq!(state.history.non_hand_draws_this_turn, count as i16);
            assert_eq!(events.len(), count as usize);
            assert_eq!(
                crate::engine::selection::execute_glimmer(&mut state, &catalog, &mut Vec::new(),)
                    .unwrap(),
                crate::engine::selection::SelectDisposition::Suspend
            );
        }
    }

    #[test]
    fn every_operand_drift_refuses_before_draw_or_events() {
        let mutations = [
            vec![CompiledArg::I(2), CompiledArg::I(1)],
            vec![CompiledArg::I(3), CompiledArg::I(0)],
            vec![CompiledArg::I(3)],
            vec![CompiledArg::I(3), CompiledArg::I(1), CompiledArg::I(0)],
        ];
        for args in mutations {
            let (mut state, catalog) = fixture(0);
            let before = state.clone();
            let (result, events) = run(&mut state, &catalog, 0, &args);
            assert_eq!(result, Err(EngineRefusal::MalformedArgs("glimmer_exact")));
            assert_eq!(state, before, "operand drift must be atomic: {args:?}");
            assert!(events.is_empty());
        }

        let args = [CompiledArg::I(3), CompiledArg::I(1)];
        for (target, selection, x_value) in
            [(Some(0), None, 0), (None, Some(7), 0), (None, None, 1)]
        {
            let (mut state, catalog) = fixture(0);
            let before = state.clone();
            let (result, events) =
                run_with_operands(&mut state, &catalog, 0, &args, target, selection, x_value);
            assert_eq!(result, Err(EngineRefusal::MalformedArgs("glimmer_exact")));
            assert_eq!(state, before, "non-native operand must be atomic");
            assert!(events.is_empty());
        }
    }

    #[test]
    fn terminal_draw_is_empty_and_counter_overflow_refuses_the_public_action_atomically() {
        let args = [CompiledArg::I(3), CompiledArg::I(1)];
        let (mut terminal, catalog) = fixture(0);
        terminal.history.over = true;
        let before = terminal.clone();
        let (result, events) = run(&mut terminal, &catalog, 0, &args);
        assert_eq!(result, Err(EngineRefusal::ContinuationNotModeled));
        assert_eq!(terminal, before, "terminal Draw and suffix are no-ops");
        assert!(events.is_empty());

        let (mut overflow, catalog) = fixture(0);
        let source = overflow
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .pop()
            .unwrap();
        overflow.piles.get_mut(PileId::Hand).make_mut().push(source);
        overflow.energy = 3;
        overflow.history.non_hand_draws_this_turn = i16::MAX;
        overflow
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let unchanged = overflow.clone();
        assert_eq!(
            crate::engine::apply_action(
                &overflow,
                &catalog,
                &crate::engine::Action::Play {
                    uid: 1,
                    target: None,
                    selection: crate::engine::SelectionRef::NONE,
                },
            ),
            Err(EngineRefusal::CounterOverflow("non_hand_draws_this_turn"))
        );
        assert_eq!(overflow, unchanged, "overflow refusal is action-atomic");
    }
}
