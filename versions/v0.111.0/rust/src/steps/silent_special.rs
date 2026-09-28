//! Card-step bodies for the `content/cards/silent_special.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::{Arg, CardRow};
use crate::engine::EngineRefusal;
use crate::engine::cards::{inject_generated_free_this_turn_bottom, shuffle_generation_slice};
use crate::engine::damage::{player_attack_all_from_card, player_attack_from_card};
use crate::hot::PileId;
use crate::ids::{CardId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// `shiv_attack` was enabled when the generated-card seam landed. Distraction
/// joins only with its bounded Generation preview, solo Silent provenance,
/// and generated-leaf admission closure in the same issue-1753 transaction.
pub const IMPLEMENTED: &[StepKind] = &[StepKind::DistractionExact, StepKind::ShivAttack];

/// `distraction_exact` — exact Silent Skill generation.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) delegates to `_apply_distraction_exact`: consume one complete shuffle of the frozen 39-card Silent Skill
/// pool, take the first fresh L0 identity, apply `SetToFreeThisTurn`, then
/// begin one singular generated-card transaction into Hand/Bottom (or
/// Discard/Bottom at cap).
///
/// Current v0.111.0 native authority: installed and archived `sts2.dll`
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Distraction/<OnPlay>d__5::MoveNext` RVA `0x399690` reads the exact
/// owner's unlocked character pool and current multiplayer constraint at IL
/// `0x0020-0x0051`, applies the Skill predicate at IL `0x0056-0x0075`, calls
/// `GetDistinctForCombat(1, CombatCardGeneration)` at IL `0x007a-0x0095`,
/// then applies `SetToFreeThisTurn` and awaits one Hand/Bottom generated Add
/// at IL `0x009b-0x0104`. Predicate RVA `0x399682` accepts exactly Skill;
/// constructor/keywords/upgrade RVAs `0xdddcb/0xddddf/0xdde2b` pin the two
/// source costs and Exhaust while every generated result remains L0.
fn distraction_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(
        (
            ctx.spec.identity.id,
            ctx.spec.identity.upgrade,
            ctx.args,
            ctx.target,
            ctx.selection,
            ctx.x_value,
        ),
        (CardId::Distraction, 0 | 1, [], None, None, 0)
    ) {
        return Err(EngineRefusal::MalformedArgs("distraction_exact"));
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
    let source = PileId::ALL
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .find(|card| card.uid == ctx.source_uid)
        .expect("the unique count proved one live Distraction source");
    // #2560/#2946: the OWNER's Skill pool under the recorded profile
    // (`Catalog::owner_type_generation_pool`), not the frozen Silent table.
    // The frozen table is this derivation for a fully-unlocked Silent owner
    // (`one_type_owner_pools_reproduce_the_frozen_constants`).
    if ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || !crate::engine::cards::owner_pool_profile_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs(
            "distraction generation provenance",
        ));
    }
    let pool = ctx.catalog.owner_type_generation_pool(
        ctx.state
            .reward_card_pool
            .expect("the exact Distraction provenance has an owner"),
        crate::content_tables::CardType::Skill,
    );
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "distraction generation provenance",
        ));
    }

    fn apply(ctx: &mut StepCtx<'_>, pool: &[CardId]) -> Result<(), EngineRefusal> {
        let shuffled = shuffle_generation_slice(ctx.state, pool)?;
        inject_generated_free_this_turn_bottom(
            ctx.state,
            ctx.catalog,
            CardIdentity {
                id: shuffled[0],
                upgrade: 0,
                enchantment: None,
            },
            PileId::Hand,
            ctx.events,
        )
    }

    // The selected identity or generated listener can still refuse after the
    // RNG shuffle. Rehearse the complete body so direct callers observe the
    // same atomic boundary as public `apply_action`. Every reader and callback
    // reached below is carried by `HotState` plus the immutable catalog; there
    // is no process-global hook that can distinguish rehearsal from commit.
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target: None,
        selection: None,
        x_value: 0,
        args: ctx.args,
        events: &mut probe_events,
    };
    apply(&mut probe_ctx, &pool)?;
    apply(ctx, &pool)
}

pub(crate) fn distraction_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    distraction_body(ctx)
}

/// `("shiv_attack", damage, hits)` — a powered attack by a live owned Shiv.
///
/// `Shiv::get_TargetType` RVA `0xeb013` reads the same live mutable-owner
/// predicate as `Shiv/<OnPlay>d__9::MoveNext` RVA `0x3bb35c`: without Fan of
/// Knives the ordinary selected target is attacked, while with Fan the card
/// snapshots `HittableEnemies` in roster order and attacks that complete set.
pub(crate) fn shiv_attack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("shiv_attack"));
    };
    let expected_damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (crate::ids::CardId::Shiv, 0) => 4,
        (crate::ids::CardId::Shiv, 1) => 6,
        _ => return Err(EngineRefusal::MalformedArgs("shiv_attack")),
    };
    if *damage != expected_damage
        || *hits != 1
        || !shiv_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::ShivAttack && ctx.catalog.args(step.args) == ctx.args)
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("shiv_attack"));
    }
    match crate::engine::play::effective_target_type(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
    )? {
        crate::catalog::CardTargetType::AnyEnemy => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let monster = ctx
                .state
                .monsters
                .get(target)
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let Some((Some(_), Some(frozen_target))) =
                crate::engine::play::active_card_current_context(ctx.source_uid)
            else {
                return Err(EngineRefusal::ContinuationNotModeled);
            };
            if frozen_target != (monster.slot, monster.uid) {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            player_attack_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                &[target],
                *damage,
                *hits,
                ctx.events,
            )
        }
        crate::catalog::CardTargetType::AllEnemies => {
            if ctx.target.is_some()
                || !matches!(
                    crate::engine::play::active_card_current_context(ctx.source_uid),
                    Some((Some(_), None))
                )
            {
                return Err(EngineRefusal::TargetMismatch { required: false });
            }
            player_attack_all_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                *damage,
                *hits,
                ctx.events,
            )
        }
        _ => Err(EngineRefusal::MalformedArgs("shiv target shape")),
    }
}

pub(crate) fn shiv_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::Shiv,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ShivAttack,
                args: [Arg::I(4), Arg::I(1)]
            }]
        ) | (
            CardId::Shiv,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ShivAttack,
                args: [Arg::I(6), Arg::I(1)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::RewardPool;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::engine::play::{autoplay_collected_cards, autoplay_draw_top};
    use crate::engine::{Action, Event, SelectionRef, apply_action_into};
    use crate::hot::RngStream;
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, LocalCostExpiration,
        RngStreamState,
    };
    use crate::ids::{CardId, MonsterKind, PowerId};
    use crate::powers::SlotWire;
    use crate::rng::Xoshiro256StarStar;

    fn shiv_catalog(upgrade: u8) -> (Catalog, crate::catalog::CardAtom) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Shiv,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        (builder.build(), atom)
    }

    fn state_with_target() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
    }

    fn run_shiv(
        upgrade: u8,
        args: &[CompiledArg],
    ) -> (Result<(), EngineRefusal>, HotState, Vec<Event>) {
        let (catalog, atom) = shiv_catalog(upgrade);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = state_with_target();
        state.next_card_uid = 8;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut events = Vec::new();
        let frozen_target = (state.monsters[0].slot, state.monsters[0].uid);
        let result = crate::engine::play::with_test_active_play_target(7, frozen_target, || {
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: Some(0),
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            shiv_attack(&mut ctx)
        });
        (result, state, events)
    }

    #[test]
    fn shiv_uses_its_exact_level_damage_through_the_powered_attack_pipeline() {
        let (result, state, _) = run_shiv(0, &[CompiledArg::I(4), CompiledArg::I(1)]);
        result.unwrap();
        assert_eq!(state.monsters[0].hp, 16);

        let (result, state, _) = run_shiv(1, &[CompiledArg::I(6), CompiledArg::I(1)]);
        result.unwrap();
        assert_eq!(state.monsters[0].hp, 14);
    }

    #[test]
    fn shiv_rejects_body_drift_before_mutating_combat() {
        let (result, state, events) = run_shiv(0, &[CompiledArg::I(6), CompiledArg::I(1)]);
        assert_eq!(result, Err(EngineRefusal::MalformedArgs("shiv_attack")));
        assert_eq!(state.monsters[0].hp, 20);
        assert!(events.is_empty());
    }

    #[test]
    fn live_fan_makes_manual_shiv_targetless_and_attacks_the_ordered_hittable_snapshot() {
        let (catalog, atom) = shiv_catalog(0);
        let source = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 8;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        for (uid, hp) in [(41, 20), (42, 0), (43, 20)] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
            monster.uid = uid;
            monster.slot = uid as i32;
            state.monsters_mut().push(monster);
        }
        state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        state.powers.set(PowerId::Accuracy, SlotWire::Int, 2);
        let mut events = Vec::new();

        crate::engine::play::play_card(&mut state, &catalog, source.uid, None, None, &mut events)
            .unwrap();

        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| monster.hp)
                .collect::<Vec<_>>(),
            [14, 0, 14]
        );
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::MonsterDamaged { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [41, 43]
        );
    }

    #[test]
    fn shiv_target_presence_tracks_live_fan_and_refuses_before_mutation() {
        let (catalog, atom) = shiv_catalog(0);
        for (fan, target) in [(false, None), (true, Some(0))] {
            let source = HotCard {
                uid: 7,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            let mut state = state_with_target();
            state.energy = 3;
            state.next_card_uid = 8;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            if fan {
                state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
            }
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    target,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::TargetMismatch { required: !fan })
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let source = HotCard {
            uid: 9,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut empty = HotState::at_defaults();
        empty.hp = 50;
        empty.energy = 3;
        empty.next_card_uid = 10;
        empty.piles.get_mut(PileId::Hand).make_mut().push(source);
        empty.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        crate::engine::play::play_card(
            &mut empty,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        let mut legacy = state_with_target();
        legacy.energy = 3;
        legacy.next_card_uid = 8;
        legacy.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: crate::hot::CARD_FLAG_LEGACY,
        });
        let before = legacy.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut legacy,
                &catalog,
                7,
                Some(0),
                None,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs("dynamic target source"))
        );
        assert_eq!(legacy, before);
    }

    #[test]
    fn collected_autoplay_shiv_re_reads_fan_and_skips_targets_rng() {
        let (catalog, atom) = shiv_catalog(0);
        let source = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 8;
        state.piles.get_mut(PileId::Draw).make_mut().push(source);
        for uid in [41, 42] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);
            monster.uid = uid;
            monster.slot = uid as i32;
            state.monsters_mut().push(monster);
        }
        state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        let targets_before = state.rng.get(crate::hot::RngStream::Targets).counter;

        autoplay_collected_cards(&mut state, &catalog, &[source], &mut Vec::new()).unwrap();

        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| monster.hp)
                .collect::<Vec<_>>(),
            [16, 16]
        );
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Targets).counter,
            targets_before
        );
    }

    fn distraction_fixture(
        upgrade: u8,
        seed: u64,
    ) -> (HotState, Catalog, CardIdentity, RngStreamState) {
        let identity = CardIdentity {
            id: CardId::Distraction,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        for id in crate::content_tables::DISTRACTION_SKILL_POOL_V1101 {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let seeded = Xoshiro256StarStar::from_seed(seed);
        let entering = RngStreamState {
            words: seeded.words,
            counter: seeded.counter,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.reward_card_pool = Some(RewardPool::Silent);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(RngStream::Generation, entering);
        state.next_card_uid = 18;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 17,
            atom,
            flags: 0,
        });
        (state, catalog, identity, entering)
    }

    fn run_distraction_body(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events,
        };
        distraction_body(&mut ctx)
    }

    #[test]
    fn distraction_consumes_the_full_pool_shuffle_then_adds_one_free_l0_card() {
        for upgrade in 0..=1 {
            let (mut state, catalog, identity, entering) = distraction_fixture(upgrade, 47);
            state.powers.set(PowerId::Arsenal, SlotWire::Int, 2);
            assert!(
                state
                    .fanouts
                    .set_local_generated_power_order(&[PowerId::Arsenal])
            );
            let mut oracle = Xoshiro256StarStar {
                words: entering.words,
                counter: entering.counter,
            };
            let mut pool = crate::content_tables::DISTRACTION_SKILL_POOL_V1101;
            oracle.shuffle(&mut pool).unwrap();
            let mut events = Vec::new();

            run_distraction_body(&mut state, &catalog, identity, &mut events).unwrap();

            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                entering.counter + 38
            );
            assert_eq!(state.history.owner_generated_cards_combat, 1);
            assert_eq!(state.next_generated_hook_uid, 1);
            assert_eq!(state.next_card_uid, 19);
            assert_eq!(
                state.powers.value(PowerId::Strength),
                2,
                "clone rehearsal must not leak its stateful generated callback"
            );
            assert!(state.exact_piles);
            let generated = state.piles.get(PileId::Hand).as_slice()[0];
            assert_eq!(generated.uid, 18);
            assert_ne!(generated.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
            assert_eq!(
                catalog.spec(generated.atom).unwrap().identity,
                CardIdentity {
                    id: pool[0],
                    upgrade: 0,
                    enchantment: None,
                },
                "Distraction+ changes only the source Energy cost"
            );
            let instance = state.card_states.get(generated.uid);
            assert_eq!(instance.local_cost_modifiers.resolve(99), 0);
            assert_eq!(
                instance.local_cost_modifiers.as_slice()[0].expiration,
                LocalCostExpiration::ThisTurnOrPlayed
            );
            assert_eq!(instance.free_star_cost_this_turn_or_played_rows, 1);
            assert_eq!(
                events,
                vec![
                    Event::CardResolved {
                        uid: 18,
                        pile: PileId::Hand,
                    },
                    Event::PowerChanged {
                        subject: crate::engine::Subject::Player,
                        power: PowerId::Strength,
                        amount: 2,
                    },
                ]
            );
        }
    }

    #[test]
    fn distraction_redirects_a_full_hand_to_discard_bottom() {
        let (mut state, catalog, identity, _) = distraction_fixture(0, 47);
        let shiv = catalog
            .atom(&CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((20..30).map(|uid| HotCard {
                uid,
                atom: shiv,
                flags: 0,
            }));
        let mut events = Vec::new();
        run_distraction_body(&mut state, &catalog, identity, &mut events).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 18);
        assert_eq!(
            events,
            vec![Event::CardResolved {
                uid: 18,
                pile: PileId::Discard,
            }]
        );
    }

    #[test]
    fn ended_distraction_shuffles_and_records_without_publishing_a_card() {
        let (mut state, catalog, identity, entering) = distraction_fixture(0, 47);
        state.history.over = true;
        state.history.owner_generated_cards_combat = 3;
        state.next_generated_hook_uid = 5;
        let before_piles = state.piles.clone();
        let mut events = Vec::new();

        run_distraction_body(&mut state, &catalog, identity, &mut events).unwrap();

        assert_eq!(
            state.rng.get(RngStream::Generation).counter,
            entering.counter + 38
        );
        assert_eq!(state.history.owner_generated_cards_combat, 4);
        assert_eq!(state.next_generated_hook_uid, 6);
        assert_eq!(state.next_card_uid, 18);
        assert_eq!(state.piles, before_piles);
        assert!(!state.exact_piles);
        assert!(events.is_empty());
    }

    #[test]
    fn distraction_refuses_shape_source_and_provenance_drift_atomically() {
        let (mut state, catalog, identity, _) = distraction_fixture(0, 47);
        let before = state.clone();
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 1,
            args: &[],
            events: &mut events,
        };
        assert_eq!(
            distraction_body(&mut ctx),
            Err(EngineRefusal::MalformedArgs("distraction_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.multiplayer_ally_key = 1;
        let party = state.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };
        assert_eq!(
            distraction_body(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "distraction generation provenance"
            ))
        );
        assert_eq!(state, party);
        assert!(events.is_empty());
    }

    #[test]
    fn distraction_preflights_an_unknown_shuffled_leaf_before_rng_publication() {
        let (mut state, full, identity, _) = distraction_fixture(0, 47);
        let source = state.piles.get(PileId::Play).as_slice()[0];
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        state.piles.get_mut(PileId::Play).make_mut()[0] = HotCard {
            atom: source_atom,
            ..source
        };
        let before = state.clone();
        let mut events = Vec::new();
        let result = run_distraction_body(&mut state, &catalog, identity, &mut events);
        assert!(matches!(result, Err(EngineRefusal::UnknownMintIdentity(_))));
        assert_eq!(state, before);
        assert!(events.is_empty());
        assert!(full.specs().count() > catalog.specs().count());
    }

    #[test]
    fn distraction_manual_direct_autoplay_sly_and_burst_share_the_exact_body() {
        for (mode, burst, expected_cards, expected_draws) in [
            (0u8, 0, 1usize, 38u64),
            (1, 0, 1, 38),
            (2, 0, 1, 38),
            (0, 1, 2, 76),
        ] {
            let (mut state, catalog, identity, entering) = distraction_fixture(0, 47);
            let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
            let initial_pile = if mode == 0 {
                PileId::Hand
            } else {
                PileId::Draw
            };
            state.piles.get_mut(initial_pile).make_mut().push(source);
            state.energy = 3;
            if burst > 0 {
                state.powers.set(PowerId::Burst, SlotWire::Int, burst);
            }
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let mut events = Vec::new();
            match mode {
                0 => {
                    state = apply_action_into(
                        &state,
                        &catalog,
                        &Action::Play {
                            uid: source.uid,
                            target: None,
                            selection: SelectionRef::NONE,
                        },
                        &mut events,
                    )
                    .unwrap();
                }
                1 => {
                    autoplay_collected_cards(&mut state, &catalog, &[source], &mut events).unwrap();
                }
                2 => autoplay_draw_top(&mut state, &catalog, 1, &mut events).unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                entering.counter + expected_draws
            );
            assert_eq!(
                state.history.owner_generated_cards_combat,
                expected_cards as i32
            );
            assert_eq!(state.piles.get(PileId::Hand).len(), expected_cards);
            assert_eq!(
                state
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .filter(|card| card.uid == source.uid)
                    .count(),
                1
            );
            assert!(state.piles.get(PileId::Play).is_empty());
            assert_eq!(catalog.spec(source.atom).unwrap().identity, identity);
        }
    }
}
