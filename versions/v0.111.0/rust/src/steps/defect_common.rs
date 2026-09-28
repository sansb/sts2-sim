//! Card-step bodies for the `content/cards/defect_common.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1319/#1679/#1562): 3 of 3 ported
//!
//! All three bodies begin with an attack and then cross a shared engine
//! boundary. #1679 supplies Uproar's exact direct AutoPlay of one selected
//! live Draw card; #1562 supplies Gunk Up's exact generated-Status
//! transaction; #1319 supplies Go for the Eyes' exhaustive post-hit live
//! intent projection.

use super::StepCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::engine::EngineRefusal;
use crate::engine::cards::inject_generated_record_before_ending_bottom;
use crate::engine::damage::{apply_card_monster_debuff, player_attack_from_card};
use crate::hot::{MiseryToken, PileId};
use crate::ids::{CardId, PowerId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::GoForTheEyesExact,
    StepKind::GunkUpExact,
    StepKind::UproarExact,
];

/// `go_for_the_eyes_exact` — attack, then Weak an exact surviving attacker.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) calls `_go_for_the_eyes_exact`.
/// Its post-hit gate calls `monster_intends_to_attack`.
/// Current-build `GoForTheEyes/<OnPlay>d__7::MoveNext` (RVA `0x3a20b8`)
/// awaits its card-sourced attack, re-reads the same `CardPlay.Target`, and
/// applies Weak only when live `MonsterModel.IntendsToAttack` (RVA `0x82347`)
/// accepts its post-hit move. The target creation uid is frozen across the
/// await: a same-slot Axebot Stock replacement must not inherit the old
/// creature's Weak.
pub(crate) fn go_for_the_eyes_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    go_for_the_eyes_body(ctx, true)
}

fn go_for_the_eyes_body(ctx: &mut StepCtx<'_>, preflight: bool) -> Result<(), EngineRefusal> {
    if ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("go_for_the_eyes_exact action"));
    }
    let (damage, weak) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::GoForTheEyes, 0, [CompiledArg::I(3), CompiledArg::I(1)]) => (3, 1),
        (CardId::GoForTheEyes, 1, [CompiledArg::I(4), CompiledArg::I(2)]) => (4, 2),
        _ => return Err(EngineRefusal::MalformedArgs("go_for_the_eyes_exact")),
    };
    if crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
        != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("go_for_the_eyes_exact owner"));
    }
    let (source_pile, source_index) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
            EngineRefusal::ActiveCardNotUnique {
                uid: ctx.source_uid,
                matches: 0,
            },
        )?;
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "go_for_the_eyes_exact physical source",
        ));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let target_uid = ctx
        .state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?
        .uid;

    // The Weak command is a separately awaited suffix. Rehearse the complete
    // body so an unsupported post-hit intent or listener refuses before the
    // real attack publishes damage, events, target replacement, or RNG.
    if preflight {
        let mut probe = ctx.state.clone();
        let mut probe_events = Vec::new();
        let mut probe_ctx = StepCtx {
            state: &mut probe,
            catalog: ctx.catalog,
            spec: ctx.spec,
            source_uid: ctx.source_uid,
            target: ctx.target,
            selection: ctx.selection,
            x_value: ctx.x_value,
            args: ctx.args,
            events: &mut probe_events,
        };
        go_for_the_eyes_body(&mut probe_ctx, false)?;
    }

    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    if ctx.state.history.over {
        return Ok(());
    }
    let Some(target) = exact_surviving_target(ctx.state, target_uid)? else {
        return Ok(());
    };
    if !crate::engine::turn::monster_intends_to_attack(ctx.state, ctx.catalog, target)? {
        return Ok(());
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::Weak,
        MiseryToken::Weak,
        weak,
        ctx.events,
    )
}

fn exact_surviving_target(
    state: &crate::hot::HotState,
    uid: u32,
) -> Result<Option<usize>, EngineRefusal> {
    let mut matches = state
        .monsters
        .iter()
        .enumerate()
        .filter_map(|(index, monster)| (monster.uid == uid).then_some(index));
    let Some(target) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(EngineRefusal::MalformedArgs(
            "go_for_the_eyes_exact target identity",
        ));
    }
    Ok((state.monsters[target].hp > 0).then_some(target))
}

/// `gunk_up_exact` — three-hit attack, then fixed Slimed generation.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) runs the three-hit attack, then calls
/// `add_fixed_generated_status` for `SLIMED+0` at Discard/Bottom. That
/// helper records CardGenerated history before its combat-ending gate; a
/// lethal attack therefore records generation without inserting the card.
///
/// Unit C supplies both prerequisites. The generated command is issued even
/// after a lethal third hit: history/listener epochs advance, while the nested
/// Discard add allocates no physical uid and inserts no card.
pub(crate) fn gunk_up_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(hits)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("gunk_up_exact"));
    };
    let expected_damage = 4 + i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::GunkUp
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *damage != expected_damage
        || *hits != 3
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("gunk_up_exact owner"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        *damage,
        *hits,
        ctx.events,
    )?;
    inject_generated_record_before_ending_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: CardId::Slimed,
            upgrade: 0,
            enchantment: None,
        },
        1,
        PileId::Discard,
        ctx.events,
    )
}

/// `uproar_exact` — attack twice, then shuffle/select/direct-AutoPlay Draw.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) attacks twice and then always calls
/// `_uproar_select_and_autoplay`, even after a lethal hit. The helper
/// copies eligible live Draw attacks (falling back to Unplayable attacks),
/// stable-sorts and fully shuffles that pool on the main RNG stream, then
/// directly AutoPlays the selected physical uid if combat remains live.
///
/// The family-callable direct path retains the chosen physical card in Draw
/// through OnPlay, rejects an active-card recursion cycle, and uses ordinary
/// result routing. The selection shuffle stays local until the complete child
/// program has passed the synchronous-only gate.
pub(crate) fn uproar_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, hits) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Uproar, 0, [CompiledArg::I(6), CompiledArg::I(2)]) => (6, 2),
        (CardId::Uproar, 1, [CompiledArg::I(8), CompiledArg::I(2)]) => (8, 2),
        _ => return Err(EngineRefusal::MalformedArgs("uproar_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )?;
    crate::engine::play::uproar_draw_attack(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardAtom, CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Event, StepCtx, Subject};
    use crate::hot::{HotCard, HotMonster, HotState, MonsterOverride, PileId};
    use crate::ids::{CardId, MonsterKind};
    use crate::powers::SlotWire;

    const FAMILY_KINDS: [StepKind; 3] = [
        StepKind::GoForTheEyesExact,
        StepKind::GunkUpExact,
        StepKind::UproarExact,
    ];

    fn go_fixture(
        mut monster: HotMonster,
        upgrade: u8,
    ) -> (HotState, Catalog, CardAtom, [CompiledArg; 2]) {
        let identity = CardIdentity {
            id: CardId::GoForTheEyes,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.intern_monster(monster.kind).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        monster.max_hp = monster.max_hp.max(monster.hp);
        state.monsters_mut().push(monster);
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let args = match upgrade {
            0 => [CompiledArg::I(3), CompiledArg::I(1)],
            1 => [CompiledArg::I(4), CompiledArg::I(2)],
            _ => unreachable!(),
        };
        (state, catalog, atom, args)
    }

    fn run_go(
        state: &mut HotState,
        catalog: &Catalog,
        atom: CardAtom,
        args: &[CompiledArg],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(atom).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args,
            events,
        };
        go_for_the_eyes_exact(&mut ctx)
    }

    #[test]
    fn defect_common_claims_all_three_exact_bodies() {
        assert_eq!(IMPLEMENTED, FAMILY_KINDS);
        for kind in FAMILY_KINDS {
            assert!(IMPLEMENTED.contains(&kind));
            assert!(crate::engine::capability_manifest().steps.contains(&kind));
        }
    }

    #[test]
    fn all_six_generated_carriers_keep_the_exact_three_kind_census() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| FAMILY_KINDS.contains(&step.kind))
            })
            .collect();

        assert_eq!(carriers.len(), 6, "three exact cards at both levels");
        let actual: Vec<_> = carriers
            .iter()
            .map(|row| (row.id, row.upgrade, row.steps[0].kind))
            .collect();
        assert_eq!(
            actual,
            vec![
                (CardId::GoForTheEyes, 0, StepKind::GoForTheEyesExact),
                (CardId::GoForTheEyes, 1, StepKind::GoForTheEyesExact),
                (CardId::GunkUp, 0, StepKind::GunkUpExact),
                (CardId::GunkUp, 1, StepKind::GunkUpExact),
                (CardId::Uproar, 0, StepKind::UproarExact),
                (CardId::Uproar, 1, StepKind::UproarExact),
            ]
        );
        for row in carriers {
            assert_eq!(row.steps.len(), 1, "{} has one atomic body", row.name);
            assert!(row.playable);
            assert!(row.targeted);
            assert_eq!(row.target_type, "AnyEnemy");
            assert_eq!(
                IMPLEMENTED.contains(&row.steps[0].kind),
                matches!(
                    row.id,
                    CardId::GoForTheEyes | CardId::GunkUp | CardId::Uproar
                )
            );
        }
    }

    #[test]
    fn generated_operands_pin_every_upgrade_without_curation() {
        for (id, upgrade, kind, args) in [
            (
                CardId::GoForTheEyes,
                0,
                StepKind::GoForTheEyesExact,
                &[Arg::I(3), Arg::I(1)][..],
            ),
            (
                CardId::GoForTheEyes,
                1,
                StepKind::GoForTheEyesExact,
                &[Arg::I(4), Arg::I(2)][..],
            ),
            (
                CardId::GunkUp,
                0,
                StepKind::GunkUpExact,
                &[Arg::I(4), Arg::I(3)][..],
            ),
            (
                CardId::GunkUp,
                1,
                StepKind::GunkUpExact,
                &[Arg::I(5), Arg::I(3)][..],
            ),
            (
                CardId::Uproar,
                0,
                StepKind::UproarExact,
                &[Arg::I(6), Arg::I(2)][..],
            ),
            (
                CardId::Uproar,
                1,
                StepKind::UproarExact,
                &[Arg::I(8), Arg::I(2)][..],
            ),
        ] {
            let rows: Vec<_> = CARD_ROWS
                .iter()
                .filter(|row| id == row.id && upgrade == row.upgrade)
                .collect();
            assert_eq!(rows.len(), 1, "one exact generated identity");
            assert_eq!(rows[0].steps.len(), 1);
            assert_eq!(rows[0].steps[0].kind, kind);
            assert_eq!(rows[0].steps[0].args, args);
        }
    }

    #[test]
    fn go_for_the_eyes_levels_attack_then_apply_card_sourced_weak() {
        for (upgrade, damage, weak) in [(0, 3, 1), (1, 4, 2)] {
            let mut target = HotMonster::new(MonsterKind::Toadpole, 30);
            target.loop_pos = 1; // WHIRL: native SingleAttackIntent.
            target.uid = 11;
            let (mut state, catalog, atom, args) = go_fixture(target, upgrade);
            let mut events = Vec::new();

            run_go(&mut state, &catalog, atom, &args, &mut events).unwrap();

            assert_eq!(state.monsters[0].hp, 30 - damage);
            assert_eq!(state.monsters[0].powers.value(PowerId::Weak), weak);
            let hit = events
                .iter()
                .position(|event| matches!(event, Event::MonsterDamaged { uid: 11, .. }))
                .unwrap();
            let application = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        Event::PowerChanged {
                            subject: Subject::Monster(11),
                            power: PowerId::Weak,
                            amount,
                        } if *amount == weak
                    )
                })
                .unwrap();
            assert!(hit < application, "the live intent read follows damage");
        }
    }

    #[test]
    fn go_for_the_eyes_re_reads_damage_mutated_overrides() {
        let mut eel = HotMonster::new(MonsterKind::TerrorEel, 79);
        eel.max_hp = 100;
        eel.loop_pos = 0; // CRASH attacks before the hit.
        eel.powers.set(PowerId::Shriek, SlotWire::Bool, 1);
        let (mut state, catalog, atom, args) = go_fixture(eel, 1);
        let mut events = Vec::new();
        run_go(&mut state, &catalog, atom, &args, &mut events).unwrap();
        assert_eq!(state.monsters[0].hp, 75);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Stunned);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);

        let mut tunneler = HotMonster::new(MonsterKind::Tunneler, 30);
        tunneler.max_hp = 30;
        tunneler.loop_pos = 0; // BITE attacks before Burrowed breaks.
        tunneler.block = 3;
        tunneler.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
        let (mut state, catalog, atom, args) = go_fixture(tunneler, 0);
        run_go(&mut state, &catalog, atom, &args, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 30);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);
    }

    #[test]
    fn go_for_the_eyes_skips_nonattack_lethal_and_axebot_replacement_targets() {
        let mut nonattack = HotMonster::new(MonsterKind::Toadpole, 30);
        nonattack.loop_pos = 2; // SPIKEN: native BuffIntent.
        let (mut state, catalog, atom, args) = go_fixture(nonattack, 0);
        run_go(&mut state, &catalog, atom, &args, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 27);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);

        let mut lethal = HotMonster::new(MonsterKind::Toadpole, 3);
        lethal.loop_pos = 1;
        let (mut state, catalog, atom, args) = go_fixture(lethal, 0);
        run_go(&mut state, &catalog, atom, &args, &mut Vec::new()).unwrap();
        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);

        let mut axebot = HotMonster::new(MonsterKind::Axebot, 3);
        axebot.max_hp = 76;
        axebot.loop_pos = 0;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        let (mut state, catalog, atom, args) = go_fixture(axebot, 0);
        run_go(&mut state, &catalog, atom, &args, &mut Vec::new()).unwrap();
        assert!(!state.history.over);
        assert_eq!(state.monsters[0].uid, 1);
        assert_eq!(state.monsters[0].loop_pos, 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);
    }

    #[test]
    fn go_for_the_eyes_target_resolution_handles_removal_and_duplicate_uid_fail_closed() {
        let empty = HotState::at_defaults();
        assert_eq!(exact_surviving_target(&empty, 7), Ok(None));

        let mut duplicate = HotState::at_defaults();
        let mut first = HotMonster::new(MonsterKind::Toadpole, 10);
        first.uid = 7;
        duplicate.monsters_mut().push(first.clone());
        duplicate.monsters_mut().push(first);
        assert_eq!(
            exact_surviving_target(&duplicate, 7),
            Err(EngineRefusal::MalformedArgs(
                "go_for_the_eyes_exact target identity"
            ))
        );
    }

    #[test]
    fn go_for_the_eyes_preflights_unknown_loop_intent_before_any_mutation() {
        let mut toadpole = HotMonster::new(MonsterKind::Toadpole, 30);
        toadpole.loop_pos = 7;
        let (mut state, catalog, atom, args) = go_fixture(toadpole, 0);
        let mut events = vec![Event::TurnEnded { turn: 7 }];
        let before_state = state.clone();
        let before_events = events.clone();

        assert_eq!(
            run_go(&mut state, &catalog, atom, &args, &mut events),
            Err(EngineRefusal::CounterOverflow("loop_pos"))
        );
        assert_eq!(state, before_state);
        assert_eq!(events, before_events);
    }

    #[test]
    fn go_for_the_eyes_requires_one_exact_physical_source_before_damage() {
        let mut target = HotMonster::new(MonsterKind::Toadpole, 30);
        target.loop_pos = 1;
        let (mut state, catalog, atom, args) = go_fixture(target, 0);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        let before = state.clone();
        assert_eq!(
            run_go(&mut state, &catalog, atom, &args, &mut Vec::new()),
            Err(EngineRefusal::ActiveCardNotUnique { uid: 7, matches: 0 })
        );
        assert_eq!(state, before);

        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let before = state.clone();
        assert_eq!(
            run_go(&mut state, &catalog, atom, &args, &mut Vec::new()),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn go_for_the_eyes_rejects_selection_and_x_before_any_mutation() {
        let mut target = HotMonster::new(MonsterKind::Toadpole, 30);
        target.loop_pos = 1;
        let (state, catalog, atom, args) = go_fixture(target, 0);
        let spec = *catalog.spec(atom).unwrap();

        for (selection, x_value) in [(Some(99), 0), (None, 1)] {
            let mut candidate = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 7 }];
            let before_state = candidate.clone();
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut candidate,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: Some(0),
                selection,
                x_value,
                args: &args,
                events: &mut events,
            };

            assert_eq!(
                go_for_the_eyes_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs("go_for_the_eyes_exact action"))
            );
            assert_eq!(candidate, before_state);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn gunk_up_attacks_then_records_slimed_before_the_ending_insert_gate() {
        let identity = CardIdentity {
            id: CardId::GunkUp,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let args = [CompiledArg::I(4), CompiledArg::I(3)];

        for (hp, ending) in [(20, false), (1, true)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.next_card_uid = 9;
            state.next_generated_hook_uid = 4;
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            state
                .monsters_mut()
                .push(HotMonster::new(crate::ids::MonsterKind::Toadpole, hp));
            let mut events = Vec::new();
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
            gunk_up_exact(&mut ctx).unwrap();
            assert_eq!(state.history.over, ending);
            assert_eq!(state.history.owner_generated_cards_combat, 1);
            assert_eq!(state.next_generated_hook_uid, 5);
            if ending {
                assert!(state.piles.get(PileId::Discard).is_empty());
                assert_eq!(state.next_card_uid, 9);
            } else {
                let generated = state.piles.get(PileId::Discard).as_slice()[0];
                assert_eq!(
                    catalog.spec(generated.atom).unwrap().identity.id,
                    CardId::Slimed
                );
                assert_eq!(generated.uid, 9);
                assert_eq!(state.next_card_uid, 10);
            }
        }
    }
}
