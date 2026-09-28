//! Card-step bodies for the `content/cards/calculated_history.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::damage::{player_attack_all_from_card, player_attack_from_card};
use crate::engine::play::resolved_energy_cost;
use crate::hot::PileId;
use crate::ids::{CardId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::CrescentSpearExact,
    StepKind::HelixDrillExact,
    StepKind::PullFromBelowExact,
    StepKind::RadiateExact,
    StepKind::RattleExact,
    StepKind::TearAsunderExact,
];

/// Crescent Spear's canonical Star-card census and repeated targeted attack.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The playing card has left Hand
/// but is still a combat card, so the census is Draw + Hand + Discard +
/// Exhaust + this exact source. It reads each identity's canonical fixed/X
/// Star-cost row, not mutable cost state, and deliberately excludes any other
/// card temporarily in Play.
pub(crate) fn crescent_spear_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, multiplier): (i64, i64) =
        match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
            (CardId::CrescentSpear, 0, [CompiledArg::I(8), CompiledArg::I(2)]) => (8, 2),
            (CardId::CrescentSpear, 1, [CompiledArg::I(8), CompiledArg::I(3)]) => (8, 3),
            _ => return Err(EngineRefusal::MalformedArgs("crescent_spear_exact")),
        };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let mut star_cards = i64::from(ctx.spec.row.star_cost >= 0 || ctx.spec.row.star_x);
    for pile in [PileId::Draw, PileId::Hand, PileId::Discard, PileId::Exhaust] {
        for card in ctx.state.piles.get(pile).as_slice() {
            let spec = ctx
                .catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.row.star_cost >= 0 || spec.row.star_x {
                star_cards = star_cards
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("crescent_spear_exact"))?;
            }
        }
    }
    let hits = multiplier
        .checked_mul(star_cards)
        .ok_or(EngineRefusal::CounterOverflow("crescent_spear_exact"))?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// Helix Drill's live-cost subtraction from the current turn's Energy spend.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). SpendResources has already added
/// this play's positive spend. The cost is re-read from the exact physical
/// source with all live local modifiers. Manual and Mayhem plays are in Play;
/// direct AutoPlay keeps its source in Draw, Hand, Discard, or Exhaust and
/// marks that uid active without moving it. Signed subtraction is preserved:
/// a nonpositive result is a real zero-hit attack command.
pub(crate) fn helix_drill_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::HelixDrill, 0, [CompiledArg::I(3)]) => 3,
        (CardId::HelixDrill, 1, [CompiledArg::I(5)]) => 5,
        _ => return Err(EngineRefusal::MalformedArgs("helix_drill_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
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
    let hits = i64::from(ctx.state.history.energy_spent_this_turn)
        - resolved_energy_cost(ctx.state, source, ctx.spec);
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// Pull From Below's completed effective-Ethereal play count.
///
/// Current v0.111.0/41cef1ea IL: `PullFromBelow::get_CanonicalVars`
/// (`0xe875c`) builds `CalculatedHits` from the same-owner
/// `CardPlayFinishedEntry::WasEthereal` census in
/// `PullFromBelow/<>c::<get_CanonicalVars>b__5_0` (`0x3b4404`) and its
/// predicate (`0x3b4450`). `<OnPlay>d__6::MoveNext` (`0x3b4474`) reads that
/// value once, constructs one physical-card AttackCommand with 5/7 damage,
/// and targets the recorded `CardPlay.Target`. The shared completion writer
/// already records effective runtime Ethereal after each manual, AutoPlay,
/// generated replay, or resumed body finishes.
/// Python `_run_steps_inner` (frozen, deleted #2827) independently pins the same two
/// carrier rows and reads the same completed-Ethereal counter before issuing
/// the physical-card attack.
pub(crate) fn pull_from_below_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::PullFromBelow, 0, [CompiledArg::I(5)]) => 5,
        (CardId::PullFromBelow, 1, [CompiledArg::I(7)]) => 7,
        _ => return Err(EngineRefusal::MalformedArgs("pull_from_below_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let hits = i64::from(ctx.state.fanouts.ethereal_plays_finished_combat());
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// Radiate's all-enemy attack repeated by Stars gained this turn.
///
/// One all-opponents command: each hit re-resolves the living opponents
/// ([`player_attack_all_from_card`], #3023) and issues no target RNG. Zero
/// Stars still issues the real outer
/// attack command with zero hits, preserving its Before/AfterAttack effects.
pub(crate) fn radiate_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Radiate, 0, [CompiledArg::I(3)]) => 3,
        (CardId::Radiate, 1, [CompiledArg::I(4)]) => 4,
        _ => return Err(EngineRefusal::MalformedArgs("radiate_exact")),
    };
    let hits = i64::from(ctx.state.history.stars_gained_this_turn);
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        hits,
        ctx.events,
    )
}

/// `rattle_exact` — exact live-Osty command-history attack.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) first distinguishes no ally, a
/// dead Osty, and a live Osty; the live branch attacks as
/// `BreakerIdentity.PLAYER_PET` for `1 + State.osty_attacks_this_turn` hits.
/// The hit count is frozen before entering the command, which itself records
/// one new Osty attack after all hits complete.
pub(crate) fn rattle_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Rattle, 0, [CompiledArg::I(7)]) => 7,
        (CardId::Rattle, 1, [CompiledArg::I(9)]) => 9,
        _ => return Err(EngineRefusal::MalformedArgs("rattle_exact")),
    };
    if ctx.state.fanouts.pet().osty().is_none() {
        return Ok(());
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let hits = i64::from(ctx.state.fanouts.pet().attacks_this_turn())
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("rattle_exact"))?;
    crate::engine::damage::player_pet_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// Tear Asunder's combat-wide unblocked-damage-result repeat count.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The current attack is not yet a
/// player-damage result, so the already-recorded combat counter is read and
/// one is added before issuing the single targeted command.
pub(crate) fn tear_asunder_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::TearAsunder, 0, [CompiledArg::I(5)]) => 5,
        (CardId::TearAsunder, 1, [CompiledArg::I(7)]) => 7,
        _ => return Err(EngineRefusal::MalformedArgs("tear_asunder_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let hits = i64::from(ctx.state.history.player_unblocked_damage_results_combat)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("tear_asunder_exact"))?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::CARD_ROWS;
    use crate::engine::play::{autoplay_collected_cards, play_card, resume_selection};
    use crate::engine::{Event, SelectionAnswer};
    use crate::hot::{
        HotCard, HotMonster, HotPile, HotState, LocalCostExpiration, LocalCostModifier,
        LocalCostModifierKind,
    };
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn target_state(hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        state
    }

    #[test]
    fn manifest_mechanically_admits_exactly_twelve_generated_rows() {
        let rows = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| IMPLEMENTED.contains(&step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                (CardId::CrescentSpear, 0),
                (CardId::CrescentSpear, 1),
                (CardId::HelixDrill, 0),
                (CardId::HelixDrill, 1),
                (CardId::PullFromBelow, 0),
                (CardId::PullFromBelow, 1),
                (CardId::Radiate, 0),
                (CardId::Radiate, 1),
                (CardId::Rattle, 0),
                (CardId::Rattle, 1),
                (CardId::TearAsunder, 0),
                (CardId::TearAsunder, 1),
            ]
        );
    }

    #[test]
    fn crescent_spear_counts_canonical_star_cards_outside_play_plus_source() {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::CrescentSpear, 0)).unwrap();
        let other_star_atom = builder.intern(identity(CardId::Quasar, 0)).unwrap();
        let ordinary_atom = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let source = HotCard {
            uid: 7,
            atom: source_atom,
            flags: 0,
        };
        let mut state = target_state(100);
        state.piles.set(
            PileId::Draw,
            HotPile::from_cards(vec![HotCard {
                uid: 8,
                atom: other_star_atom,
                flags: 0,
            }]),
        );
        state.piles.set(
            PileId::Hand,
            HotPile::from_cards(vec![HotCard {
                uid: 9,
                atom: ordinary_atom,
                flags: 0,
            }]),
        );
        // The second Play card is intentionally Star-costed: Python counts
        // only the exact active source, not unrelated nested Play entries.
        state.piles.set(
            PileId::Play,
            HotPile::from_cards(vec![
                HotCard {
                    uid: 10,
                    atom: other_star_atom,
                    flags: 0,
                },
                source,
            ]),
        );
        let mut events = Vec::<Event>::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(8), CompiledArg::I(2)],
            events: &mut events,
        };
        crescent_spear_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 68); // 2 Star cards × 2 hits × 8.
    }

    #[test]
    fn helix_drill_rereads_the_active_sources_live_cost_from_every_pile() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::HelixDrill, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let source = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        for pile in PileId::ALL {
            let mut state = target_state(30);
            state.history.energy_spent_this_turn = 4;
            state.piles.set(pile, HotPile::from_cards(vec![source]));
            state.card_states.append_local_cost_modifier(
                source.uid,
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: 2,
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                },
            );
            let mut events = Vec::<Event>::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(3)],
                events: &mut events,
            };
            helix_drill_exact(&mut ctx).unwrap();
            assert_eq!(
                state.monsters[0].hp, 24,
                "{pile:?}: (4 spent - 2 live cost) × 3"
            );
        }

        let mut state = target_state(30);
        state.history.energy_spent_this_turn = 1;
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![source]));
        state.card_states.append_local_cost_modifier(
            source.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 2,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut events = Vec::<Event>::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };
        helix_drill_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 30);
        assert_eq!(state.powers.value(PowerId::Vigor), 0);

        for (state, matches) in [
            (target_state(30), 0),
            (
                {
                    let mut duplicate = target_state(30);
                    duplicate
                        .piles
                        .set(PileId::Play, HotPile::from_cards(vec![source]));
                    duplicate
                        .piles
                        .set(PileId::Discard, HotPile::from_cards(vec![source]));
                    duplicate
                },
                2,
            ),
        ] {
            let mut state = state;
            let before = state.clone();
            let mut events = Vec::<Event>::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(3)],
                events: &mut events,
            };
            assert_eq!(
                helix_drill_exact(&mut ctx),
                Err(EngineRefusal::ActiveCardNotUnique {
                    uid: source.uid,
                    matches,
                })
            );
            assert_eq!(state, before, "{matches} source matches must be atomic");
            assert!(events.is_empty());
        }
    }

    #[test]
    fn radiate_hits_all_targets_and_uses_the_star_gain_counter() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::Radiate, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = target_state(30);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state.history.stars_gained_this_turn = 2;
        let mut events = Vec::<Event>::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };
        radiate_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 24);
        assert_eq!(state.monsters[1].hp, 24);
    }

    #[test]
    fn tear_asunder_adds_one_to_prior_unblocked_damage_results() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::TearAsunder, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = target_state(30);
        state.history.player_unblocked_damage_results_combat = 2;
        let mut events = Vec::<Event>::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(5)],
            events: &mut events,
        };
        tear_asunder_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 15);
    }

    #[test]
    fn pull_from_below_uses_the_exact_history_count_for_both_levels() {
        for (upgrade, damage) in [(0, 5_i64), (1, 7_i64)] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(identity(CardId::PullFromBelow, upgrade))
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let source = HotCard {
                uid: 7,
                atom,
                flags: 0,
            };
            let mut state = target_state(100);
            assert!(state.fanouts.set_ethereal_plays_finished_combat(3));
            state
                .piles
                .set(PileId::Play, HotPile::from_cards(vec![source]));
            let mut events = Vec::new();
            let args = [CompiledArg::I(damage)];
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

            pull_from_below_exact(&mut ctx).unwrap();

            assert_eq!(ctx.state.monsters[0].hp, 100 - 3 * damage as i32);
            assert_eq!(ctx.state.fanouts.ethereal_plays_finished_combat(), 3);
            assert_eq!(ctx.state.history.owner_attack_plays_started_this_turn, 0);
        }
    }

    #[test]
    fn pull_from_below_zero_hits_is_a_real_command_and_terminal_max_is_bounded() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::PullFromBelow, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let source = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        let mut state = target_state(30);
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![source]));
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(5)],
            events: &mut events,
        };
        pull_from_below_exact(&mut ctx).unwrap();
        assert_eq!(ctx.state.monsters[0].hp, 30);
        assert_eq!(ctx.state.powers.value(PowerId::Vigor), 0);

        assert!(
            ctx.state
                .fanouts
                .set_ethereal_plays_finished_combat(i32::MAX)
        );
        ctx.state.monsters_mut()[0].hp = 1;
        pull_from_below_exact(&mut ctx).unwrap();
        assert_eq!(ctx.state.monsters[0].hp, -4);
        assert!(ctx.state.history.over);

        let before = ctx.state.clone();
        ctx.events.clear();
        pull_from_below_exact(&mut ctx).unwrap();
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());
    }

    #[test]
    fn pull_from_below_malformed_rows_targets_and_source_uids_are_atomic() {
        let mut builder = CatalogBuilder::new();
        let pull_atom = builder.intern(identity(CardId::PullFromBelow, 0)).unwrap();
        let strike_atom = builder
            .intern(identity(CardId::StrikeNecrobinder, 0))
            .unwrap();
        let catalog = builder.build();
        let pull_spec = *catalog.spec(pull_atom).unwrap();
        let strike_spec = *catalog.spec(strike_atom).unwrap();
        let source = HotCard {
            uid: 7,
            atom: pull_atom,
            flags: 0,
        };

        for (spec, target, args, expected) in [
            (
                strike_spec,
                Some(0),
                vec![CompiledArg::I(5)],
                EngineRefusal::MalformedArgs("pull_from_below_exact"),
            ),
            (
                pull_spec,
                Some(0),
                vec![CompiledArg::I(7)],
                EngineRefusal::MalformedArgs("pull_from_below_exact"),
            ),
            (
                pull_spec,
                None,
                vec![CompiledArg::I(5)],
                EngineRefusal::TargetMismatch { required: true },
            ),
        ] {
            let mut state = target_state(30);
            assert!(state.fanouts.set_ethereal_plays_finished_combat(2));
            state
                .piles
                .set(PileId::Play, HotPile::from_cards(vec![source]));
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };
            assert_eq!(pull_from_below_exact(&mut ctx), Err(expected));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let mut state = target_state(30);
        assert!(state.fanouts.set_ethereal_plays_finished_combat(1));
        state.monsters_mut()[0]
            .powers
            .set(PowerId::CurlUp, SlotWire::Int, 1);
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![source]));
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &pull_spec,
            source_uid: source.uid + 1,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(5)],
            events: &mut events,
        };
        assert_eq!(
            pull_from_below_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid + 1,
                matches: 0,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn pull_from_below_observes_manual_autoplay_generated_and_suspension_timing() {
        let mut builder = CatalogBuilder::new();
        let pull_atom = builder.intern(identity(CardId::PullFromBelow, 0)).unwrap();
        let lethality_atom = builder.intern(identity(CardId::Lethality, 0)).unwrap();
        let dagger_atom = builder.intern(identity(CardId::DaggerThrow, 0)).unwrap();
        let defend_atom = builder.intern(identity(CardId::DefendSilent, 0)).unwrap();
        let catalog = builder.build();

        let pull = HotCard {
            uid: 2,
            atom: pull_atom,
            flags: 0,
        };
        let lethality = HotCard {
            uid: 1,
            atom: lethality_atom,
            flags: 0,
        };
        for (label, manual, replays, expected_hits) in [
            ("manual", true, false, 1),
            ("direct AutoPlay", false, false, 1),
            ("generated replay", true, true, 2),
        ] {
            let mut state = target_state(100);
            state.energy = 3;
            state.next_card_uid = 3;
            if replays {
                state.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            }
            let source_pile = if manual {
                PileId::Hand
            } else {
                PileId::Discard
            };
            state.piles.get_mut(source_pile).make_mut().push(lethality);
            if manual {
                play_card(
                    &mut state,
                    &catalog,
                    lethality.uid,
                    None,
                    None,
                    &mut Vec::new(),
                )
                .unwrap();
            } else {
                autoplay_collected_cards(&mut state, &catalog, &[lethality], &mut Vec::new())
                    .unwrap();
            }
            assert_eq!(
                state.fanouts.ethereal_plays_finished_combat(),
                expected_hits,
                "{label}"
            );
            state.powers.set(PowerId::Lethality, SlotWire::Int, 0);
            state.piles.get_mut(PileId::Hand).make_mut().push(pull);
            play_card(
                &mut state,
                &catalog,
                pull.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.monsters[0].hp, 100 - expected_hits * 5, "{label}");
        }

        let dagger = HotCard {
            uid: 10,
            atom: dagger_atom,
            flags: 0,
        };
        let hand_defend = HotCard {
            uid: 11,
            atom: defend_atom,
            flags: 0,
        };
        let draw_defend = HotCard {
            uid: 12,
            atom: defend_atom,
            flags: 0,
        };
        let mut suspended = target_state(100);
        suspended.energy = 3;
        suspended.next_card_uid = 14;
        suspended
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([dagger, hand_defend]);
        suspended
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(draw_defend);
        play_card(
            &mut suspended,
            &catalog,
            dagger.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(suspended.pending.is_some());
        assert_eq!(
            suspended.fanouts.ethereal_plays_finished_combat(),
            0,
            "a selection-suspended play has not finished"
        );
        resume_selection(
            &mut suspended,
            &catalog,
            SelectionAnswer::OptionIndex(0),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(suspended.pending.is_none());
        assert_eq!(
            suspended.fanouts.ethereal_plays_finished_combat(),
            0,
            "a resumed non-Ethereal play stays outside the exact history"
        );
    }

    #[test]
    fn rattle_freezes_one_plus_prior_pet_commands_and_records_only_one_new_command() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::Rattle, 0)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = target_state(50);
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        state.fanouts.set_osty_attacks_this_turn(2).unwrap();
        let mut events = Vec::new();
        let args = [CompiledArg::I(7)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };

        rattle_exact(&mut ctx).unwrap();

        assert_eq!(ctx.state.monsters[0].hp, 29);
        assert_eq!(ctx.state.fanouts.pet().attacks_this_turn(), 3);
        assert_eq!(
            ctx.state.monsters[0].owner_powered_damage_results_this_turn,
            0
        );
        assert_eq!(
            ctx.state.monsters[0].nonowner_same_side_powered_damage_results_this_turn, 3,
            "every frozen Rattle hit retains the PLAYER_PET dealer"
        );

        ctx.state.fanouts.set_osty(None).unwrap();
        let before = ctx.state.clone();
        rattle_exact(&mut ctx).unwrap();
        assert_eq!(*ctx.state, before, "missing Osty is a complete no-op");
    }
}
