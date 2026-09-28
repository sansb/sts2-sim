//! Card-step bodies for the `content/cards/necrobinder_common.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1327/#1751): 1 of 1 ported
//!
//! Drain Power uses the shared physical one-level upgrade transaction and the
//! complete catalog upgrade closure landed with Armaments+. Its fused body
//! remains one transaction: attack, freshly materialize the post-attack
//! Discard candidates, fully shuffle them on Selection, and upgrade the first
//! two/three physical identities.

use super::StepCtx;
use crate::catalog::{CardSpec, Catalog, CompiledArg};
use crate::engine::EngineRefusal;
use crate::engine::cards::{native_card_is_upgradable, upgrade_live_cards_once_ungated};
use crate::engine::damage::{damage_combat_is_ending, player_attack_from_card};
use crate::hot::{CARD_FLAG_LEGACY, HotCard, HotState, PileId, RngStream, RngStreamState};
use crate::ids::{CardId, StepKind};
use crate::rng::Xoshiro256StarStar;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::DrainPowerExact];

fn drain_program(spec: &CardSpec, catalog: &Catalog) -> Option<(i64, usize)> {
    let expected = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::DrainPower, 0) => (10, 2),
        (CardId::DrainPower, 1) => (12, 3),
        _ => return None,
    };
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::DrainPowerExact
        && catalog.args(step.args)
            == [
                CompiledArg::I(expected.0),
                CompiledArg::I(i64::try_from(expected.1).ok()?),
            ])
    .then_some(expected)
}

fn exact_source(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<HotCard, EngineRefusal> {
    let matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .copied()
                .map(move |card| (pile, card))
        })
        .filter(|(_, card)| card.uid == source_uid)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: matches.len(),
        });
    }
    let (_, card) = matches[0];
    if card.flags & CARD_FLAG_LEGACY != 0 || catalog.spec(card.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs("drain_power_exact source"));
    }
    Ok(card)
}

fn upgradable_discard_uids(state: &HotState, catalog: &Catalog) -> Result<Vec<u32>, EngineRefusal> {
    state
        .piles
        .get(PileId::Discard)
        .as_slice()
        .iter()
        .map(|card| {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            Ok(native_card_is_upgradable(spec.identity).then_some(card.uid))
        })
        .filter_map(Result::transpose)
        .collect()
}

fn preflight_suffix(
    state: &HotState,
    catalog: &Catalog,
    candidates: &[u32],
) -> Result<(), EngineRefusal> {
    if state.rng.is_vacant(RngStream::Sel) {
        return Err(EngineRefusal::MalformedArgs(
            "Drain Power Selection provenance",
        ));
    }
    let mut upgraded = state.clone();
    upgrade_live_cards_once_ungated(&mut upgraded, catalog, candidates)?;
    let live = state.rng.get(RngStream::Sel);
    let draws = u64::try_from(candidates.len().saturating_sub(1))
        .map_err(|_| EngineRefusal::CounterOverflow("drain_power_exact shuffle"))?;
    live.counter
        .checked_add(draws)
        .ok_or(EngineRefusal::CounterOverflow("drain_power_exact shuffle"))?;
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let mut shuffled = candidates.to_vec();
    rng.shuffle(&mut shuffled)
        .map_err(|_| EngineRefusal::CounterOverflow("drain_power_exact shuffle"))
}

/// Authenticate Drain Power's late Discard writer before any play prefix.
///
/// This catches malformed/direct callers before energy, source movement, the
/// CardPlayed event, or Attack. The source may deliberately remain in Draw
/// for Catastrophe's direct AutoPlay; the body needs only its exact unique uid
/// and repeats the validation on the post-attack Discard snapshot.
pub(crate) fn preflight_drain_power(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if drain_program(spec, catalog).is_none() {
        return Err(EngineRefusal::MalformedArgs("drain_power_exact preflight"));
    }
    exact_source(state, catalog, source_uid, spec)?;
    let candidates = upgradable_discard_uids(state, catalog)?;
    preflight_suffix(state, catalog, &candidates)
}

fn drain_power_inner(
    ctx: &mut StepCtx<'_>,
    damage: i64,
    count: usize,
) -> Result<(), EngineRefusal> {
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
    let candidates = upgradable_discard_uids(ctx.state, ctx.catalog)?;
    preflight_suffix(ctx.state, ctx.catalog, &candidates)?;
    let live = ctx.state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let mut shuffled = candidates;
    rng.shuffle(&mut shuffled)
        .map_err(|_| EngineRefusal::CounterOverflow("drain_power_exact shuffle"))?;
    ctx.state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    shuffled.truncate(count);
    // Each picked card goes through `CardCmd::Upgrade` (IL_0149, the
    // single-card overload `0x12f64f` into the list overload `0x12f660`),
    // which returns at `CombatManager.IsEnding` (IL_000c-IL_0018) before its
    // `IsUpgradable` walk. A lethal Attack therefore still consumes the whole
    // TakeRandom schedule above but upgrades nothing (#3303).
    if damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    upgrade_live_cards_once_ungated(ctx.state, ctx.catalog, &shuffled)
}

/// Drain Power's attack followed by its live Discard random-upgrade suffix.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) preflights then commits one targeted
/// attack, materializes the post-attack Discard candidates through
/// `_w177_upgradable_discard`, fully shuffles that live pile-order list
/// on CombatCardSelection, and upgrades its first 2/3 exact physical cards.
/// `card_upgrade_to` preserves uid and mutable payload while clamping
/// every local-cost `Set` row when the printed base cost falls.
///
/// Current-build `DrainPower/<OnPlay>d__3::MoveNext` RVA `0x399f7c` has no
/// combat-ending branch of its own between its awaited Attack (IL_0078) and
/// the Discard read (IL_00d5-IL_00df), so a lethal hit still consumes the full
/// `TakeRandom` Selection schedule (IL_012d). Each picked card then reaches
/// `CardCmd::Upgrade` (IL_0149; `0x12f64f` forwards to `0x12f660`), which
/// returns at `CombatManager.IsEnding` (IL_000c-IL_0018), so a lethal Drain
/// Power upgrades nothing (#3303, native `f0eaf3d0b09703dc` and
/// `f4de05e3e25bcb8e`). The ending projection is the shared
/// [`damage_combat_is_ending`].
pub(crate) fn drain_power_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let Some((damage, count)) = drain_program(ctx.spec, ctx.catalog) else {
        return Err(EngineRefusal::MalformedArgs("drain_power_exact"));
    };
    if ctx.args
        != [
            CompiledArg::I(damage),
            CompiledArg::I(
                i64::try_from(count)
                    .map_err(|_| EngineRefusal::CounterOverflow("drain_power_exact count"))?,
            ),
        ]
        || ctx.target.is_none()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("drain_power_exact"));
    }
    exact_source(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;

    let mut probe = ctx.state.clone();
    let mut scratch = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        args: ctx.args,
        source_uid: ctx.source_uid,
        target: ctx.target,
        selection: ctx.selection,
        x_value: ctx.x_value,
        events: &mut scratch,
    };
    drain_power_inner(&mut probe_ctx, damage, count)?;
    *ctx.state = probe;
    ctx.events.extend(scratch);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Action, SelectionRef, apply_action_into};
    use crate::hot::{CARD_FLAG_PICK, HotMonster};
    use crate::ids::{CardId, MonsterKind, PowerId};
    use crate::powers::SlotWire;

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn fixture(upgrade: u8, hp: i32) -> (HotState, Catalog, HotCard, Vec<HotCard>) {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(identity(CardId::DrainPower, upgrade))
            .unwrap();
        let bases = [
            identity(CardId::DefendNecrobinder, 0),
            identity(CardId::StrikeNecrobinder, 0),
            identity(CardId::Apotheosis, 0),
            identity(CardId::Dazed, 0),
        ];
        let atoms = bases
            .into_iter()
            .map(|identity| builder.intern(identity).unwrap())
            .collect::<Vec<_>>();
        builder.intern_all_card_upgrade_closure().unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 10,
            atom: source_atom,
            flags: CARD_FLAG_PICK,
        };
        let discard = atoms
            .into_iter()
            .enumerate()
            .map(|(index, atom)| HotCard {
                uid: 20 + u32::try_from(index).unwrap(),
                atom,
                flags: CARD_FLAG_PICK,
            })
            .collect::<Vec<_>>();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(discard.iter().copied());
        let seeded = Xoshiro256StarStar::from_seed(0xD4A1);
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        (state, catalog, source, discard)
    }

    fn play(
        state: &HotState,
        catalog: &Catalog,
        source: HotCard,
    ) -> (HotState, Vec<crate::engine::Event>) {
        let mut events = Vec::new();
        let next = apply_action_into(
            state,
            catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();
        (next, events)
    }

    #[test]
    fn necrobinder_common_claims_the_exact_shared_upgrade_body() {
        assert_eq!(IMPLEMENTED, &[StepKind::DrainPowerExact]);
        assert!(crate::steps::is_implemented(StepKind::DrainPowerExact));
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::DrainPowerExact)
        );
    }

    #[test]
    fn both_generated_drain_power_carriers_are_the_complete_exact_public_surface() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::DrainPowerExact)
            })
            .collect();

        assert_eq!(carriers.len(), 2, "Drain Power at both generated levels");
        let actual: Vec<_> = carriers
            .iter()
            .map(|row| {
                assert_eq!(row.steps.len(), 1, "{} has one atomic body", row.name);
                assert_eq!(row.cost, 1);
                assert!(row.playable);
                assert!(row.targeted);
                assert_eq!(row.target_type, "AnyEnemy");
                assert!(!row.is_skill);
                assert!(!row.is_power);
                assert!(!row.exhausts);
                assert!(IMPLEMENTED.contains(&row.steps[0].kind));
                (row.id, row.upgrade, row.steps[0].args)
            })
            .collect();
        assert_eq!(
            actual,
            vec![
                (CardId::DrainPower, 0, &[Arg::I(10), Arg::I(2)][..]),
                (CardId::DrainPower, 1, &[Arg::I(12), Arg::I(3)][..]),
            ]
        );
    }

    #[test]
    fn drain_power_hits_then_full_shuffles_live_discard_and_upgrades_physical_prefix() {
        for upgrade in [0, 1] {
            let (state, catalog, source, discard) = fixture(upgrade, 50);
            let entering = state.rng.get(RngStream::Sel);
            let mut oracle = Xoshiro256StarStar {
                words: entering.words,
                counter: entering.counter,
            };
            let mut expected = discard[..3].iter().map(|card| card.uid).collect::<Vec<_>>();
            oracle.shuffle(&mut expected).unwrap();
            expected.truncate(2 + usize::from(upgrade));

            let before = state.clone();
            let (next, events) = play(&state, &catalog, source);
            assert_eq!(state, before, "public input stays immutable");
            assert_eq!(next.monsters[0].hp, 50 - (10 + 2 * i32::from(upgrade)));
            assert_eq!(next.rng.get(RngStream::Sel).counter, 2);
            assert_eq!(next.rng.get(RngStream::Sel).words, oracle.words);
            assert_eq!(
                next.piles
                    .get(PileId::Discard)
                    .as_slice()
                    .last()
                    .unwrap()
                    .uid,
                source.uid
            );
            for card in &discard {
                let live = next
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .find(|live| live.uid == card.uid)
                    .unwrap();
                let level = catalog.spec(live.atom).unwrap().identity.upgrade;
                assert_eq!(
                    level,
                    u8::from(expected.contains(&card.uid)),
                    "uid {}",
                    card.uid
                );
                assert_eq!(live.flags, card.flags);
            }
            let played = events
                .iter()
                .position(|event| matches!(event, crate::engine::Event::CardPlayed { uid, .. } if *uid == source.uid))
                .unwrap();
            let damaged = events
                .iter()
                .position(|event| matches!(event, crate::engine::Event::MonsterDamaged { .. }))
                .unwrap();
            let resolved = events
                .iter()
                .position(|event| matches!(event, crate::engine::Event::CardResolved { uid, pile: PileId::Discard } if *uid == source.uid))
                .unwrap();
            assert!(played < damaged && damaged < resolved);
        }
    }

    #[test]
    fn lethal_attack_runs_the_full_selection_but_upgrades_nothing() {
        // #3303: `CardCmd::Upgrade` 0x12f660 returns at IsEnding
        // (IL_000c-IL_0018), while Drain Power's own TakeRandom still runs.
        let (state, catalog, source, discard) = fixture(0, 10);
        let (next, _) = play(&state, &catalog, source);
        assert!(next.history.over);
        assert_eq!(next.rng.get(RngStream::Sel).counter, 2);
        assert_eq!(
            discard[..3]
                .iter()
                .filter(|card| {
                    let live = next
                        .piles
                        .get(PileId::Discard)
                        .as_slice()
                        .iter()
                        .find(|live| live.uid == card.uid)
                        .unwrap();
                    catalog.spec(live.atom).unwrap().identity.upgrade == 1
                })
                .count(),
            0
        );
        // The Selection stream advanced by exactly the native full shuffle.
        let entering = state.rng.get(RngStream::Sel);
        let mut oracle = Xoshiro256StarStar {
            words: entering.words,
            counter: entering.counter,
        };
        let mut expected = discard[..3].iter().map(|card| card.uid).collect::<Vec<_>>();
        oracle.shuffle(&mut expected).unwrap();
        assert_eq!(next.rng.get(RngStream::Sel).words, oracle.words);
    }

    #[test]
    fn replay_reloads_the_post_first_body_discard_and_selection_schedule() {
        let (mut state, catalog, source, discard) = fixture(0, 100);
        state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let (next, _) = play(&state, &catalog, source);
        assert_eq!(next.monsters[0].hp, 80);
        assert_eq!(next.rng.get(RngStream::Sel).counter, 2);
        assert_eq!(next.powers.value(PowerId::OneTwoPunch), 0);
        assert_eq!(next.history.owner_attack_plays_started_this_turn, 2);
        assert!(discard[..3].iter().all(|card| {
            let live = next
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .find(|live| live.uid == card.uid)
                .unwrap();
            catalog.spec(live.atom).unwrap().identity.upgrade == 1
        }));
    }

    #[test]
    fn catastrophe_direct_autoplay_keeps_drain_in_draw_during_the_exact_body() {
        let mut builder = CatalogBuilder::new();
        let catastrophe_atom = builder.intern(identity(CardId::Catastrophe, 0)).unwrap();
        let drain_atom = builder.intern(identity(CardId::DrainPower, 0)).unwrap();
        let defend_atom = builder
            .intern(identity(CardId::DefendNecrobinder, 0))
            .unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        let catalog = builder.build();
        let catastrophe = HotCard {
            uid: 7,
            atom: catastrophe_atom,
            flags: CARD_FLAG_PICK,
        };
        let drain = HotCard {
            uid: 8,
            atom: drain_atom,
            flags: CARD_FLAG_PICK,
        };
        let defend = HotCard {
            uid: 9,
            atom: defend_atom,
            flags: CARD_FLAG_PICK,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(catastrophe);
        state.piles.get_mut(PileId::Draw).make_mut().push(drain);
        state.piles.get_mut(PileId::Discard).make_mut().push(defend);
        for (stream, seed) in [
            (RngStream::Rng, 1),
            (RngStream::Targets, 2),
            (RngStream::Sel, 3),
        ] {
            let seeded = Xoshiro256StarStar::from_seed(seed);
            state.rng.set(
                stream,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
        }
        let before = state.clone();
        let mut events = Vec::new();
        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: catastrophe.uid,
                target: None,
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();
        assert_eq!(state, before);
        assert_eq!(next.monsters[0].hp, 30);
        let live_defend = next
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == defend.uid)
            .unwrap();
        assert_eq!(catalog.spec(live_defend.atom).unwrap().identity.upgrade, 1);
        assert!(next.piles.get(PileId::Draw).as_slice().is_empty());
        assert!(
            next.piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == drain.uid)
        );
    }

    #[test]
    fn empty_and_singleton_candidates_consume_no_selection_draw() {
        for keep in [0, 1] {
            let (mut state, catalog, source, discard) = fixture(0, 100);
            let retained = discard
                .iter()
                .copied()
                .filter(|card| {
                    !native_card_is_upgradable(catalog.spec(card.atom).unwrap().identity)
                })
                .chain(discard.iter().copied().take(keep))
                .collect::<Vec<_>>();
            state
                .piles
                .set(PileId::Discard, crate::hot::HotPile::from_cards(retained));
            let entering = state.rng.get(RngStream::Sel);
            let (next, _) = play(&state, &catalog, source);
            assert_eq!(next.rng.get(RngStream::Sel), entering, "keep {keep}");
        }
    }

    #[test]
    fn missing_next_atom_and_selection_counter_overflow_are_publicly_atomic() {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::DrainPower, 0)).unwrap();
        let candidate_atom = builder
            .intern(identity(CardId::DefendNecrobinder, 0))
            .unwrap();
        let incomplete = builder.build();
        let source = HotCard {
            uid: 10,
            atom: source_atom,
            flags: CARD_FLAG_PICK,
        };
        let candidate = HotCard {
            uid: 20,
            atom: candidate_atom,
            flags: CARD_FLAG_PICK,
        };
        let mut missing = HotState::at_defaults();
        missing.hp = 50;
        missing.energy = 3;
        missing
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        missing.piles.get_mut(PileId::Hand).make_mut().push(source);
        missing
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(candidate);
        let seeded = Xoshiro256StarStar::from_seed(17);
        missing.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        let before = missing.clone();
        let mut events = Vec::new();
        assert!(matches!(
            apply_action_into(
                &missing,
                &incomplete,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::UnknownMintIdentity(CardIdentity {
                id: CardId::DefendNecrobinder,
                upgrade: 1,
                enchantment: None,
            }))
        ));
        assert_eq!(missing, before);
        assert!(events.is_empty());

        let (mut overflow, catalog, source, _) = fixture(0, 50);
        let live = overflow.rng.get(RngStream::Sel);
        overflow.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: live.words,
                counter: u64::MAX - 1,
            },
        );
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &overflow,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("drain_power_exact shuffle"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let (mut vacant, catalog, source, _) = fixture(0, 50);
        vacant.rng.set(RngStream::Sel, RngStreamState::default());
        let before = vacant.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &vacant,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Drain Power Selection provenance"
            ))
        );
        assert_eq!(vacant, before);
        assert!(events.is_empty());
    }

    #[test]
    fn malformed_operands_and_duplicate_source_refuse_before_public_prefixes() {
        let (mut state, catalog, source, _) = fixture(0, 50);
        state.piles.get_mut(PileId::Draw).make_mut().push(source);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let mut direct = before;
        direct.piles.get_mut(PileId::Draw).make_mut().clear();
        let source = direct.piles.get_mut(PileId::Hand).make_mut().remove(0);
        direct.piles.get_mut(PileId::Play).make_mut().push(source);
        let spec = *catalog.spec(source.atom).unwrap();
        for (target, selection, x_value, args) in [
            (None, None, 0, vec![CompiledArg::I(10), CompiledArg::I(2)]),
            (
                Some(0),
                Some(999),
                0,
                vec![CompiledArg::I(10), CompiledArg::I(2)],
            ),
            (
                Some(0),
                None,
                1,
                vec![CompiledArg::I(10), CompiledArg::I(2)],
            ),
            (Some(0), None, 0, vec![CompiledArg::I(10)]),
            (
                Some(0),
                None,
                0,
                vec![CompiledArg::I(11), CompiledArg::I(2)],
            ),
        ] {
            let mut candidate = direct.clone();
            let before = candidate.clone();
            let mut direct_events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut candidate,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection,
                x_value,
                args: &args,
                events: &mut direct_events,
            };
            assert_eq!(
                drain_power_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs("drain_power_exact"))
            );
            assert_eq!(candidate, before);
            assert!(direct_events.is_empty());
        }

        let mut forged_spec = spec;
        forged_spec.row = crate::content_tables::card_row(CardId::DrainPower, 1).unwrap();
        let mut candidate = direct.clone();
        let before = candidate.clone();
        let mut direct_events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut candidate,
            catalog: &catalog,
            spec: &forged_spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10), CompiledArg::I(2)],
            events: &mut direct_events,
        };
        assert_eq!(
            drain_power_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("drain_power_exact"))
        );
        assert_eq!(candidate, before);
        assert!(direct_events.is_empty());
    }
}
