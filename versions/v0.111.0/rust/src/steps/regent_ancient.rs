//! Card-step bodies for the `content/cards/regent_ancient.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::engine::damage::{
    alive_targets, apply_card_monster_debuff, apply_card_monster_debuff_with_catalog,
    player_attack_all_from_card,
};
use crate::hot::MiseryToken;
use crate::ids::{CardId, PowerId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::MeteorShowerExact];

/// `meteor_shower_exact` — Meteor Shower's attack and two fresh power waves.
///
/// Current v0.111.0 IL: `MeteorShower/<OnPlay>d__7::MoveNext` **0x3ac3d4**
/// separately awaits one powered `FromCard` all-opponents attack, reads
/// `HittableEnemies` for `PowerCmd.Apply<WeakPower>`, then independently
/// reads `HittableEnemies` again for `PowerCmd.Apply<VulnerablePower>`.
/// Constructor **0xe5403**, Star cost **0xe5410**, variables **0xe5413**, and
/// upgrade **0xe54db** pin the exact 14/21 damage, two-Star cost, and two
/// stacks for both power waves.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) has the same command order and
/// re-materializes `s.alive()` before each wave. The fixed-Star play pipeline
/// spends the two Stars and routes the physical card outside this body.
pub(crate) fn meteor_shower_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, amount) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::MeteorShower, 0, [CompiledArg::I(14), CompiledArg::I(2)]) => (14, 2),
        (CardId::MeteorShower, 1, [CompiledArg::I(21), CompiledArg::I(2)]) => (21, 2),
        _ => return Err(EngineRefusal::MalformedArgs("meteor_shower_exact")),
    };
    if ctx.target.is_some() {
        return Err(EngineRefusal::TargetMismatch { required: false });
    }

    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        1,
        ctx.events,
    )?;
    for target in alive_targets(ctx.state) {
        if ctx.state.history.over {
            break;
        }
        apply_card_monster_debuff(
            ctx.state,
            target,
            PowerId::Weak,
            MiseryToken::Weak,
            amount,
            ctx.events,
        )?;
    }
    for target in alive_targets(ctx.state) {
        if ctx.state.history.over {
            break;
        }
        apply_card_monster_debuff_with_catalog(
            ctx.state,
            ctx.catalog,
            target,
            PowerId::Vuln,
            MiseryToken::Vuln,
            amount,
            ctx.events,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Event, Subject};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, PileId};
    use crate::ids::MonsterKind;
    use crate::powers::SlotWire;

    fn parts_for(id: CardId, upgrade: u8, hp: &[i32]) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .extend(hp.iter().enumerate().map(|(index, hp)| {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, *hp);
                monster.uid = index as u32;
                monster.slot = index as i32;
                monster
            }));
        let source = HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        (state, catalog, source)
    }

    fn parts(upgrade: u8, hp: &[i32]) -> (HotState, Catalog, HotCard) {
        parts_for(CardId::MeteorShower, upgrade, hp)
    }

    fn run_body(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        args: &[CompiledArg],
        target: Option<usize>,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        meteor_shower_exact(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target,
            selection: None,
            x_value: 0,
            args,
            events,
        })
    }

    #[test]
    fn manifest_and_generated_rows_claim_exactly_both_meteor_showers() {
        assert_eq!(IMPLEMENTED, &[StepKind::MeteorShowerExact]);
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::MeteorShowerExact)
        );
        let rows = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps
                    .iter()
                    .find(|step| step.kind == StepKind::MeteorShowerExact)
                    .map(|step| (row.id, row.upgrade, row.star_cost, step.args))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                (CardId::MeteorShower, 0, 2, &[Arg::I(14), Arg::I(2)][..]),
                (CardId::MeteorShower, 1, 2, &[Arg::I(21), Arg::I(2)][..]),
            ]
        );
    }

    #[test]
    fn attack_precedes_fresh_weak_then_independently_fresh_vulnerable() {
        let (mut state, catalog, source) = parts(0, &[10, 50]);
        let args = [CompiledArg::I(14), CompiledArg::I(2)];
        let mut events = Vec::new();

        run_body(&mut state, &catalog, source, &args, None, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, -4);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 0);
        assert_eq!(state.monsters[1].hp, 36);
        assert_eq!(state.monsters[1].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[1].powers.value(PowerId::Vuln), 2);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PowerChanged {
                        subject: Subject::Monster(uid),
                        power,
                        amount,
                    } => Some((*uid, *power, *amount)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![(1, PowerId::Weak, 2), (1, PowerId::Vuln, 2)]
        );
    }

    #[test]
    fn vulnerable_re_reads_alive_targets_after_weak_listener_death() {
        let (mut state, catalog, source) = parts(0, &[20, 50]);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 7);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let args = [CompiledArg::I(14), CompiledArg::I(2)];
        let mut events = Vec::new();

        run_body(&mut state, &catalog, source, &args, None, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, -1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 0);
        assert_eq!(state.monsters[1].hp, 22);
        assert_eq!(state.monsters[1].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[1].powers.value(PowerId::Vuln), 2);
        assert!(!state.history.over);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PowerChanged {
                        subject: Subject::Monster(uid),
                        power,
                        amount,
                    } => Some((*uid, *power, *amount)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![
                (0, PowerId::Weak, 2),
                (1, PowerId::Weak, 2),
                (1, PowerId::Vuln, 2),
            ]
        );
    }

    #[test]
    fn terminal_during_weak_listener_suppresses_vulnerable_wave() {
        let (mut state, catalog, source) = parts(0, &[20]);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 7);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let args = [CompiledArg::I(14), CompiledArg::I(2)];
        let mut events = Vec::new();

        run_body(&mut state, &catalog, source, &args, None, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].hp, -1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 0);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PowerChanged {
                        subject: Subject::Monster(uid),
                        power,
                        amount,
                    } => Some((*uid, *power, *amount)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![(0, PowerId::Weak, 2)]
        );
    }

    #[test]
    fn terminal_attack_skips_both_power_waves() {
        let (mut state, catalog, source) = parts(1, &[21]);
        let args = [CompiledArg::I(21), CompiledArg::I(2)];
        let mut events = Vec::new();

        run_body(&mut state, &catalog, source, &args, None, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 0);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::PowerChanged { .. }))
        );
    }

    #[test]
    fn physical_play_spends_two_stars_and_routes_each_upgrade_once() {
        for (upgrade, damage) in [(0, 14), (1, 21)] {
            let (mut state, catalog, source) = parts(upgrade, &[100]);
            state.stars = 2;
            state.next_card_uid = 2;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            let mut events = Vec::new();

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();

            assert_eq!(state.stars, 0, "upgrade {upgrade}");
            assert_eq!(state.monsters[0].hp, 100 - damage, "upgrade {upgrade}");
            assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
            assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 2);
            assert!(state.piles.get(PileId::Hand).as_slice().is_empty());
            assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
        }
    }

    #[test]
    fn malformed_identity_operands_and_target_refuse_before_mutation() {
        for (id, upgrade, args, target, expected) in [
            (
                CardId::StrikeRegent,
                0,
                vec![CompiledArg::I(14), CompiledArg::I(2)],
                None,
                EngineRefusal::MalformedArgs("meteor_shower_exact"),
            ),
            (
                CardId::MeteorShower,
                0,
                vec![CompiledArg::I(21), CompiledArg::I(2)],
                None,
                EngineRefusal::MalformedArgs("meteor_shower_exact"),
            ),
            (
                CardId::MeteorShower,
                0,
                vec![CompiledArg::I(14), CompiledArg::I(3)],
                None,
                EngineRefusal::MalformedArgs("meteor_shower_exact"),
            ),
            (
                CardId::MeteorShower,
                0,
                vec![CompiledArg::I(14), CompiledArg::I(2)],
                Some(0),
                EngineRefusal::TargetMismatch { required: false },
            ),
        ] {
            let (mut state, catalog, source) = parts_for(id, upgrade, &[50]);
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                run_body(&mut state, &catalog, source, &args, target, &mut events),
                Err(expected)
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }
}
