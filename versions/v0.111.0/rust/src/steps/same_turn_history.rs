//! Card-step bodies for the `content/cards/same_turn_history.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::damage::{gain_powered_card_block, player_attack_from_card};
use crate::engine::{EngineRefusal, fire_hook};
use crate::hooks::HookEvent;
use crate::ids::{CardId, StepKind, StepWord};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::ForgottenRitualEnergyExact,
    StepKind::SameTurnHistoryBody,
];

/// Forgotten Ritual's unconditional Energy 3/4 command.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The v0.111 body no longer consults the
/// historical exhausted-card predicate. It has an ending gate, then gains the
/// row amount. `NoEnergyGainPower` is boundary-representable and
/// ledger-authenticated but independently admission-refused until every local
/// gain modifier is modeled.
///
/// `ForgottenRitual/<OnPlay>d__9` RVA `0x39ffa4` awaits `PlayerCmd.GainEnergy`
/// (IL_00e4). `PlayerCmd/<GainEnergy>d__3` (`0x3ee8a0`) returns at `IsEnding` (IL_0035-003c): the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn forgotten_ritual_energy_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::ForgottenRitual, 0, [CompiledArg::I(3)]) => 3_i16,
        (CardId::ForgottenRitual, 1, [CompiledArg::I(4)]) => 4_i16,
        _ => {
            return Err(EngineRefusal::MalformedArgs(
                "forgotten_ritual_energy_exact",
            ));
        }
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    ctx.state.energy =
        ctx.state
            .energy
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow(
                "forgotten_ritual_energy_exact",
            ))?;
    Ok(())
}

/// Evil Eye, Spite and FTL's exact same-turn-history bodies.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Evil Eye snapshots the exhaust flag and
/// issues one or two separate powered-block commands. Spite snapshots the
/// unblocked-damage flag before its attack. FTL attacks first, then reads the
/// finished-play count (which still excludes the current play) and draws one
/// below its level-specific threshold.
pub(crate) fn same_turn_history_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (mode, amount, conditional) =
        match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
            (CardId::EvilEye, 0, [CompiledArg::Word(StepWord::EvilEye), CompiledArg::I(8)]) => {
                (StepWord::EvilEye, 8, 0)
            }
            (CardId::EvilEye, 1, [CompiledArg::Word(StepWord::EvilEye), CompiledArg::I(11)]) => {
                (StepWord::EvilEye, 11, 0)
            }
            (
                CardId::Spite,
                0,
                [
                    CompiledArg::Word(StepWord::Spite),
                    CompiledArg::I(5),
                    CompiledArg::I(2),
                ],
            ) => (StepWord::Spite, 5, 2),
            (
                CardId::Spite,
                1,
                [
                    CompiledArg::Word(StepWord::Spite),
                    CompiledArg::I(5),
                    CompiledArg::I(3),
                ],
            ) => (StepWord::Spite, 5, 3),
            (
                CardId::Ftl,
                0,
                [
                    CompiledArg::Word(StepWord::Ftl),
                    CompiledArg::I(5),
                    CompiledArg::I(3),
                ],
            ) => (StepWord::Ftl, 5, 3),
            (
                CardId::Ftl,
                1,
                [
                    CompiledArg::Word(StepWord::Ftl),
                    CompiledArg::I(6),
                    CompiledArg::I(4),
                ],
            ) => (StepWord::Ftl, 6, 4),
            _ => return Err(EngineRefusal::MalformedArgs("same_turn_history_body")),
        };

    if mode == StepWord::EvilEye {
        let repeats = if ctx.state.history.owner_card_exhausted_this_turn {
            2
        } else {
            1
        };
        for _ in 0..repeats {
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, amount, ctx.events)?;
            fire_hook(
                ctx.catalog,
                HookEvent::AfterBlockGained,
                ctx.state,
                ctx.events,
            )?;
        }
        return Ok(());
    }

    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let hits = if mode == StepWord::Spite && ctx.state.history.player_unblocked_damage_this_turn {
        conditional
    } else {
        1
    };
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        amount,
        hits,
        ctx.events,
    )?;
    if mode == StepWord::Ftl
        && ctx.state.history.owner_card_plays_finished_this_turn < conditional as i16
    {
        crate::engine::play::draw_cardplay_no_result(ctx, 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::hot::{HotCard, HotMonster, HotState, PileId};
    use crate::ids::MonsterKind;
    use crate::steps::apply_step;

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn setup(source: CardIdentity) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern(source).unwrap();
        builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        (state, catalog)
    }

    fn run(
        source: CardIdentity,
        state: &mut HotState,
        catalog: &Catalog,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&source).unwrap()).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 1,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        let kind = if matches!(source.id, CardId::ForgottenRitual) {
            StepKind::ForgottenRitualEnergyExact
        } else {
            StepKind::SameTurnHistoryBody
        };
        apply_step(kind, &mut ctx)
    }

    #[test]
    fn forgotten_ritual_gains_exact_energy_unless_combat_is_over() {
        let source = identity(CardId::ForgottenRitual, 0);
        let (mut state, catalog) = setup(source);
        state.energy = 2;
        run(source, &mut state, &catalog, None, &[CompiledArg::I(3)]).unwrap();
        assert_eq!(state.energy, 5);

        state.history.over = true;
        run(source, &mut state, &catalog, None, &[CompiledArg::I(3)]).unwrap();
        assert_eq!(state.energy, 5);
    }

    #[test]
    fn evil_eye_snapshots_exhaust_history_into_separate_block_gains() {
        let source = identity(CardId::EvilEye, 0);
        let (mut state, catalog) = setup(source);
        state.history.owner_card_exhausted_this_turn = true;
        run(
            source,
            &mut state,
            &catalog,
            None,
            &[CompiledArg::Word(StepWord::EvilEye), CompiledArg::I(8)],
        )
        .unwrap();
        assert_eq!(state.block, 16);
        assert_eq!(state.history.card_block_gains, 2);
    }

    /// Two Evil Eyes in hand under a live CardPlay, with an exhaust already
    /// this turn and `Unmovable` at `amount`.
    fn evil_eye_play_fixture(
        unmovable: i32,
        replay: Option<crate::ids::PowerId>,
    ) -> (HotState, Catalog, [u32; 2]) {
        let source = identity(CardId::EvilEye, 0);
        let (mut state, catalog) = setup(source);
        let atom = catalog.atom(&source).unwrap();
        state.energy = 3;
        state.next_card_uid = 100;
        state.history.owner_card_exhausted_this_turn = true;
        for uid in [61, 62] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        state.powers.set(
            crate::ids::PowerId::Unmovable,
            crate::powers::SlotWire::Int,
            unmovable,
        );
        if let Some(replay) = replay {
            state.powers.set(replay, crate::powers::SlotWire::Int, 1);
        }
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        (state, catalog, [61, 62])
    }

    fn block_gains(events: &[crate::engine::Event]) -> Vec<i32> {
        events
            .iter()
            .filter_map(|event| match event {
                crate::engine::Event::PlayerBlockGained { amount, .. } => Some(*amount),
                _ => None,
            })
            .collect()
    }

    /// #3043: `UnmovablePower::ModifyBlockMultiplicative`'s filter drops
    /// entries whose `CardPlay` is the current one (RVA `0x34a534`
    /// IL_0051–0062), and Evil Eye's two `GainBlock` calls share one
    /// `cardPlay` (RVA `0x39c590` IL_00e8–00ee). Unmovable 1 therefore
    /// doubles both gains of the first play: 16 + 16, not 16 + 8. The second
    /// Evil Eye is a different CardPlay, so the first play's gains close its
    /// window and neither of its gains doubles.
    #[test]
    fn unmovable_doubles_every_gain_of_the_first_evil_eye_play_only() {
        let (mut state, catalog, [first, second]) = evil_eye_play_fixture(1, None);
        let mut events = Vec::new();
        crate::engine::play::play_card(&mut state, &catalog, first, None, None, &mut events)
            .unwrap();
        assert_eq!(block_gains(&events), [16, 16]);
        assert_eq!(state.block, 32);
        assert_eq!(state.history.card_block_gains, 2);

        let mut events = Vec::new();
        crate::engine::play::play_card(&mut state, &catalog, second, None, None, &mut events)
            .unwrap();
        assert_eq!(block_gains(&events), [8, 8]);
        assert_eq!(state.block, 48);
        assert_eq!(state.history.card_block_gains, 4);
    }

    /// Unmovable 3 leaves room for a second CardPlay: after the first Evil
    /// Eye the window holds 2 < 3, so the second play doubles both of its
    /// gains too, and each play still sees only the *other* play's entries.
    #[test]
    fn unmovable_window_counts_other_plays_not_the_gains_already_made_this_play() {
        let (mut state, catalog, [first, second]) = evil_eye_play_fixture(3, None);
        let mut events = Vec::new();
        crate::engine::play::play_card(&mut state, &catalog, first, None, None, &mut events)
            .unwrap();
        crate::engine::play::play_card(&mut state, &catalog, second, None, None, &mut events)
            .unwrap();
        assert_eq!(block_gains(&events), [16, 16, 16, 16]);
        assert_eq!(state.history.card_block_gains, 4);
    }

    /// A replay body is a new CardPlay: `CardModel/<OnPlayWrapper>d__339`
    /// (RVA `0x31b8d0`) constructs `CardPlay` at IL_052b inside the
    /// `PlayIndex` loop (back edge IL_0909). Body zero doubles both gains;
    /// body one sees body zero's two entries as another play's and does not.
    #[test]
    fn unmovable_replay_body_is_a_new_card_play() {
        for replay in [crate::ids::PowerId::Burst, crate::ids::PowerId::EchoForm] {
            let (mut state, catalog, [first, _]) = evil_eye_play_fixture(1, Some(replay));
            let mut events = Vec::new();
            crate::engine::play::play_card(&mut state, &catalog, first, None, None, &mut events)
                .unwrap();
            assert_eq!(block_gains(&events), [16, 16, 8, 8], "{replay:?}");
            assert_eq!(state.history.card_block_gains, 4, "{replay:?}");
        }
    }

    #[test]
    fn spite_snapshots_damage_history_and_ftl_reads_finished_plays_after_attack() {
        let spite = identity(CardId::Spite, 1);
        let (mut state, catalog) = setup(spite);
        state.history.player_unblocked_damage_this_turn = true;
        run(
            spite,
            &mut state,
            &catalog,
            Some(0),
            &[
                CompiledArg::Word(StepWord::Spite),
                CompiledArg::I(5),
                CompiledArg::I(3),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 35);

        let ftl = identity(CardId::Ftl, 0);
        let (mut state, catalog) = setup(ftl);
        let defend = identity(CardId::DefendIronclad, 0);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 9,
            atom: catalog.atom(&defend).unwrap(),
            flags: 0,
        });
        state.history.owner_card_plays_finished_this_turn = 2;
        run(
            ftl,
            &mut state,
            &catalog,
            Some(0),
            &[
                CompiledArg::Word(StepWord::Ftl),
                CompiledArg::I(5),
                CompiledArg::I(3),
            ],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 45);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    /// #3515: Forgotten Ritual's `PlayerCmd.GainEnergy` (`0x39ffa4` IL_00e4)
    /// returns at `IsEnding` (`<GainEnergy>d__3` 0x3ee8a0 IL_0035). While the
    /// combat is ending before the over latch it gains nothing; the
    /// Adaptable-vetoed control gains.
    #[test]
    fn forgotten_ritual_gains_nothing_while_combat_is_ending_before_the_over_latch() {
        let source = identity(CardId::ForgottenRitual, 0);
        let (mut template, catalog) = setup(source);
        template.energy = 2;
        crate::engine::damage::assert_ending_window_gate(&template, "Forgotten Ritual", |s, _| {
            run(source, s, &catalog, None, &[CompiledArg::I(3)])
        });
    }
}
