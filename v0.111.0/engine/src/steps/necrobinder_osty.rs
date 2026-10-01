//! Card-step bodies for the `content/cards/necrobinder_osty.py` family — GENERATED ONCE, THEN OWNED BY HAND.
//!
//! PORT_PLAN.md D4. `sim/v0.111.0/engine/tools/generate_content.py`
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
//! # Wave status (#1328 + #1675): 4 of 4 ported
//!
//! The shared solo-pet gate supplies powered pet attacks and their
//! history/listeners, summon/grow/heal, same-side pet death, and the
//! target-owned Sic Em lifecycle. The 28 exact carrier rows share these four
//! bodies; independent native-keyword gates may still refuse a whole card.

use super::StepCtx;
use crate::catalog::CompiledArg;
use crate::engine::EngineRefusal;
use crate::engine::damage::{
    alive_targets, apply_card_monster_debuff, apply_card_monster_debuff_with_catalog,
    gain_powered_card_block, player_pet_attack_all_from_card, player_pet_attack_from_card,
    player_pet_attack_random_from_card,
};
use crate::hot::{MiseryToken, PileId};
use crate::ids::{CardId, PowerId, StepKind, StepWord};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::OstyBody,
    StepKind::OstySummonHeal,
    StepKind::SacrificeOstyExact,
    StepKind::SicEmExact,
];

/// `osty_body` — exact solo-Osty attack modes.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) validates 22 generated rows and first
/// reads the live Osty slot. Every non-missing branch calls `player_attack` as
/// `BreakerIdentity.PLAYER_PET`; that command records one
/// `osty_attacks_this_turn` entry and runs the pet-only AfterAttack fan-out.
/// The fused modes additionally read current/max pet HP, the live OstyAttack
/// card census and Fetch's exact finished-source history, or kill the pet
/// through `_physical_cards_after_actual_death`.
///
/// The dedicated pet command preserves dealer provenance; it does not route
/// through the owner's ordinary player-card attack entry point.
///
/// Fetch's draw is `CardPileCmd.Draw` (`Fetch/<OnPlay>d__9` `0x39de84` IL_0120).
/// `<DrawInternal>d__21` (`0x3e3a70`) returns at `IsOverOrEnding` at entry
/// (IL_0029-003b), so the Draw tests the shared IsOverOrEnding projection, not
/// `history.over` (#3515).
pub(crate) fn osty_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let exact = matches!(
        (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args),
        (
            CardId::BoneShards,
            0,
            [
                CompiledArg::Word(StepWord::AoeBlockKill),
                CompiledArg::I(9),
                CompiledArg::I(9)
            ]
        ) | (
            CardId::BoneShards,
            1,
            [
                CompiledArg::Word(StepWord::AoeBlockKill),
                CompiledArg::I(12),
                CompiledArg::I(12)
            ]
        ) | (
            CardId::Fetch,
            0,
            [
                CompiledArg::Word(StepWord::Fetch),
                CompiledArg::I(3),
                CompiledArg::I(1)
            ]
        ) | (
            CardId::Fetch,
            1,
            [
                CompiledArg::Word(StepWord::Fetch),
                CompiledArg::I(6),
                CompiledArg::I(1)
            ]
        ) | (
            CardId::Flatten,
            0,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(12)]
        ) | (
            CardId::Flatten,
            1,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(16)]
        ) | (
            CardId::HighFive,
            0,
            [
                CompiledArg::Word(StepWord::AoeVulnerable),
                CompiledArg::I(11),
                CompiledArg::I(2)
            ]
        ) | (
            CardId::HighFive,
            1,
            [
                CompiledArg::Word(StepWord::AoeVulnerable),
                CompiledArg::I(13),
                CompiledArg::I(3)
            ]
        ) | (
            CardId::Poke,
            0,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(6)]
        ) | (
            CardId::Poke,
            1,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(9)]
        ) | (
            CardId::Protector,
            0,
            [
                CompiledArg::Word(StepWord::SingleMaxHp),
                CompiledArg::I(10),
                CompiledArg::I(1)
            ]
        ) | (
            CardId::Protector,
            1,
            [
                CompiledArg::Word(StepWord::SingleMaxHp),
                CompiledArg::I(15),
                CompiledArg::I(1)
            ]
        ) | (
            CardId::RightHandHand,
            0,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(4)]
        ) | (
            CardId::RightHandHand,
            1,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(6)]
        ) | (
            CardId::Snap,
            0,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(7)]
        ) | (
            CardId::Snap,
            1,
            [CompiledArg::Word(StepWord::Single), CompiledArg::I(10)]
        ) | (
            CardId::Squeeze,
            0,
            [
                CompiledArg::Word(StepWord::SingleOstyCards),
                CompiledArg::I(25),
                CompiledArg::I(5)
            ]
        ) | (
            CardId::Squeeze,
            1,
            [
                CompiledArg::Word(StepWord::SingleOstyCards),
                CompiledArg::I(30),
                CompiledArg::I(6)
            ]
        ) | (
            CardId::SweepingGaze,
            0,
            [CompiledArg::Word(StepWord::Random), CompiledArg::I(10)]
        ) | (
            CardId::SweepingGaze,
            1,
            [CompiledArg::Word(StepWord::Random), CompiledArg::I(15)]
        ) | (
            CardId::Unleash,
            0,
            [
                CompiledArg::Word(StepWord::SingleCurrentHp),
                CompiledArg::I(6),
                CompiledArg::I(1)
            ]
        ) | (
            CardId::Unleash,
            1,
            [
                CompiledArg::Word(StepWord::SingleCurrentHp),
                CompiledArg::I(9),
                CompiledArg::I(1)
            ]
        )
    );
    if !exact {
        return Err(EngineRefusal::MalformedArgs("osty_body"));
    }
    let Some(osty) = ctx.state.fanouts.pet().osty() else {
        return Ok(());
    };
    let source = (ctx.catalog, ctx.spec, ctx.source_uid);
    match ctx.args {
        [CompiledArg::Word(StepWord::Single), CompiledArg::I(base)] => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            player_pet_attack_from_card(ctx.state, source, &[target], *base, 1, ctx.events)
        }
        [CompiledArg::Word(StepWord::Random), CompiledArg::I(base)] => {
            player_pet_attack_random_from_card(ctx.state, source, *base, 1, ctx.events)
        }
        [
            CompiledArg::Word(StepWord::SingleMaxHp),
            CompiledArg::I(base),
            CompiledArg::I(multiplier),
        ] => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let damage = base
                .checked_add(
                    multiplier
                        .checked_mul(i64::from(osty.max_hp()))
                        .ok_or(EngineRefusal::CounterOverflow("osty max hp damage"))?,
                )
                .ok_or(EngineRefusal::CounterOverflow("osty max hp damage"))?;
            player_pet_attack_from_card(ctx.state, source, &[target], damage, 1, ctx.events)
        }
        [
            CompiledArg::Word(StepWord::SingleCurrentHp),
            CompiledArg::I(base),
            CompiledArg::I(multiplier),
        ] => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let damage = base
                .checked_add(
                    multiplier
                        .checked_mul(i64::from(osty.hp()))
                        .ok_or(EngineRefusal::CounterOverflow("osty current hp damage"))?,
                )
                .ok_or(EngineRefusal::CounterOverflow("osty current hp damage"))?;
            player_pet_attack_from_card(ctx.state, source, &[target], damage, 1, ctx.events)
        }
        [
            CompiledArg::Word(StepWord::SingleOstyCards),
            CompiledArg::I(base),
            CompiledArg::I(multiplier),
        ] => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let count = PileId::ALL
                .into_iter()
                .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
                .filter(|card| card.uid != ctx.source_uid)
                .try_fold(0_i64, |count, card| {
                    let spec = ctx
                        .catalog
                        .spec(card.atom)
                        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                    Ok::<_, EngineRefusal>(count + i64::from(spec.row.tags.contains(&"OstyAttack")))
                })?;
            let damage = base
                .checked_add(
                    multiplier
                        .checked_mul(count)
                        .ok_or(EngineRefusal::CounterOverflow("osty card damage"))?,
                )
                .ok_or(EngineRefusal::CounterOverflow("osty card damage"))?;
            player_pet_attack_from_card(ctx.state, source, &[target], damage, 1, ctx.events)
        }
        [
            CompiledArg::Word(StepWord::Fetch),
            CompiledArg::I(base),
            CompiledArg::I(cards),
        ] => {
            // v0.111.0 DLL 9cb4f1ad: Fetch::get_CanonicalVars (RVA 0xdfa45)
            // builds OstyDamage(3) at IL_0009–0010 and CardsVar(1) at
            // IL_0017–0019; OnUpgrade (RVA 0xdfabf) upgrades only OstyDamage
            // (+3). `<OnPlay>d__9::MoveNext` (RVA 0x39de84) IL_00fb–0120
            // draws `DynamicVars.Cards.BaseValue` cards when
            // `!HasBeenPlayedThisTurn`, so the draw count is the compiled
            // CardsVar arg (1), not a literal (#2519).
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let cards =
                usize::try_from(*cards).map_err(|_| EngineRefusal::MalformedArgs("fetch cards"))?;
            player_pet_attack_from_card(ctx.state, source, &[target], *base, 1, ctx.events)?;
            if !crate::engine::damage::damage_combat_is_ending(ctx.state)
                && !ctx.state.fanouts.fetch_finished(ctx.source_uid)
            {
                crate::engine::play::draw_cardplay_no_result(ctx, cards)?;
            }
            Ok(())
        }
        [
            CompiledArg::Word(StepWord::AoeBlockKill),
            CompiledArg::I(base),
            CompiledArg::I(block),
        ] => {
            player_pet_attack_all_from_card(ctx.state, source, *base, 1, ctx.events)?;
            gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, *block, ctx.events)?;
            let had_osty = ctx.state.fanouts.pet().osty().is_some();
            ctx.state
                .fanouts
                .mutate_pet(|pet| {
                    pet.kill();
                    Ok(())
                })
                .map_err(|_| EngineRefusal::CounterOverflow("Bone Shards Osty kill"))?;
            if had_osty {
                crate::engine::cards::physical_cards_after_actual_death(ctx.state)?;
            } else {
                crate::engine::cards::normalize_card_identities(ctx.state)?;
            }
            Ok(())
        }
        [
            CompiledArg::Word(StepWord::AoeVulnerable),
            CompiledArg::I(base),
            CompiledArg::I(vuln),
        ] => {
            player_pet_attack_all_from_card(ctx.state, source, *base, 1, ctx.events)?;
            let vuln: i32 = (*vuln)
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("osty vulnerable"))?;
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
                    vuln,
                    ctx.events,
                )?;
            }
            Ok(())
        }
        _ => Err(EngineRefusal::MalformedArgs("osty_body")),
    }
}

/// `osty_summon_heal` — exact summon/grow, then separately gated heal.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) calls `summon_ally` for a fresh,
/// revived, or already-live Osty, then re-reads that exact pet for a distinct
/// ending-gated capped Heal. An alive summon grows both current and maximum HP
/// before the second heal; fresh/revived summons publish their own history and
/// hooks. The current-build summon hook census is empty; the retained
/// transaction still preserves the distinct command boundary and recipient.
///
/// `Spur/<OnPlay>d__7::MoveNext` (v0.111.0 RVA `0x3be69c`) awaits
/// `OstyCmd.Summon` (`IL_00ce`), then `CreatureCmd.Heal(osty, Heal)`
/// (`IL_0145`). Both heals (the summon's own and this one) take
/// `<Heal>d__20`'s (`0x3eb4b0` `IL_0041`-`IL_005f`) non-player IsEnding
/// early return, so both read the IsEnding projection
/// (`damage_combat_is_ending`), not `history.over` (#3246).
pub(crate) fn osty_summon_heal(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (summon, heal) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Spur, 0, [CompiledArg::I(3), CompiledArg::I(5)]) => (3, 5),
        (CardId::Spur, 1, [CompiledArg::I(5), CompiledArg::I(7)]) => (5, 7),
        _ => return Err(EngineRefusal::MalformedArgs("osty_summon_heal")),
    };
    crate::engine::summon_osty(ctx.state, summon, "Osty summon")?;
    if !crate::engine::damage::damage_combat_is_ending(ctx.state) {
        ctx.state
            .fanouts
            .mutate_pet(|pet| pet.heal(heal))
            .map_err(|_| EngineRefusal::CounterOverflow("Osty heal"))?;
    }
    Ok(())
}

/// `sacrifice_osty_exact` — exact same-side Osty kill then powered block.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) freezes three times the live Osty's
/// maximum HP, preflights the suffix on a clone, commits same-side
/// `CreatureCmd.Kill(false)`, walks `_physical_cards_after_actual_death`, then issues the separately awaited powered card Block. Missing
/// Osty is a complete no-op.
///
/// Melancholy is the represented physical-card AfterDeath reader. The clone
/// preflight keeps its ordered cost-row append, identity normalization and
/// subsequent block suffix atomic.
pub(crate) fn sacrifice_osty_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let multiplier = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Sacrifice, 0 | 1, [CompiledArg::I(3)]) => 3_i64,
        _ => return Err(EngineRefusal::MalformedArgs("sacrifice_osty_exact")),
    };
    let Some(osty) = ctx.state.fanouts.pet().osty() else {
        return Ok(());
    };
    let block = multiplier
        .checked_mul(i64::from(osty.max_hp()))
        .ok_or(EngineRefusal::CounterOverflow("sacrifice Osty block"))?;
    let mut probe = ctx.state.clone();
    probe
        .fanouts
        .mutate_pet(|pet| {
            pet.kill();
            Ok(())
        })
        .map_err(|_| EngineRefusal::CounterOverflow("sacrifice Osty"))?;
    crate::engine::cards::physical_cards_after_actual_death(&mut probe)?;
    gain_powered_card_block(&mut probe, ctx.catalog, ctx.spec, block, &mut Vec::new())?;
    ctx.state
        .fanouts
        .mutate_pet(|pet| {
            pet.kill();
            Ok(())
        })
        .map_err(|_| EngineRefusal::CounterOverflow("sacrifice Osty"))?;
    crate::engine::cards::physical_cards_after_actual_death(ctx.state)?;
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, block, ctx.events).map(|_| ())
}

/// `sic_em_exact` — pet attack followed by the target-owned summon debuff.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) conditionally attacks first through the
/// live player-owned Osty, then applies SicEmPower to the retained target even
/// when the pet was missing. `_player_powers_after_damage_given` lets a
/// later `PLAYER_PET` hit re-summon the Osty before target death, while
/// `_run_enemy_phase` removes the whole power at that enemy's side end;
/// ordinary death cleanup removes it too.
///
/// The represented lifecycle includes exact card application,
/// AfterDamageGiven re-summon, owner-side expiry, and death cleanup.
pub(crate) fn sic_em_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, amount) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::SicEm, 0, [CompiledArg::I(5), CompiledArg::I(3)]) => (5, 3),
        (CardId::SicEm, 1, [CompiledArg::I(6), CompiledArg::I(4)]) => (6, 4),
        _ => return Err(EngineRefusal::MalformedArgs("sic_em_exact")),
    };
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let target_uid = ctx
        .state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::TargetMismatch { required: true })?
        .uid;
    if ctx.state.fanouts.pet().osty().is_some() {
        player_pet_attack_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[target],
            damage,
            1,
            ctx.events,
        )?;
    }
    if ctx.state.history.over
        || ctx
            .state
            .monsters
            .get(target)
            .is_none_or(|monster| monster.hp <= 0 || monster.uid != target_uid)
    {
        return Ok(());
    }
    apply_card_monster_debuff(
        ctx.state,
        target,
        PowerId::SicEm,
        MiseryToken::SicEm,
        amount,
        ctx.events,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::admission::IMPLEMENTED_POWERS;
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState};
    use crate::ids::{CardId, MonsterKind, PowerId};

    const FAMILY_KINDS: [StepKind; 4] = [
        StepKind::OstyBody,
        StepKind::OstySummonHeal,
        StepKind::SacrificeOstyExact,
        StepKind::SicEmExact,
    ];

    fn execute(
        state: &mut HotState,
        id: CardId,
        upgrade: u8,
        target: Option<usize>,
    ) -> Result<Vec<crate::engine::Event>, EngineRefusal> {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let [step] = catalog.steps(&spec) else {
            panic!("{id:?}+{upgrade} must have one step")
        };
        let args = catalog.args(step.args);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        crate::steps::apply_step(step.kind, &mut ctx)?;
        Ok(events)
    }

    #[test]
    fn necrobinder_osty_claims_all_four_exact_pet_surfaces() {
        assert_eq!(IMPLEMENTED, &FAMILY_KINDS);
        let manifest = crate::engine::capability_manifest();
        for kind in FAMILY_KINDS {
            assert!(IMPLEMENTED.contains(&kind));
            assert!(crate::steps::is_implemented(kind));
            assert!(manifest.steps.contains(&kind));
        }
    }

    #[test]
    fn all_twenty_eight_generated_carriers_and_operands_are_exact() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps
                    .iter()
                    .find(|step| FAMILY_KINDS.contains(&step.kind))
                    .map(|step| {
                        assert!(IMPLEMENTED.contains(&step.kind));
                        (row.id, row.upgrade, step.kind, step.args)
                    })
            })
            .collect();

        assert_eq!(carriers.len(), 28, "four exact kinds across 14 cards");
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::BoneShards,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("aoe_block_kill"), Arg::I(9), Arg::I(9)][..],
                ),
                (
                    CardId::BoneShards,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("aoe_block_kill"), Arg::I(12), Arg::I(12)][..],
                ),
                (
                    CardId::Fetch,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("fetch"), Arg::I(3), Arg::I(1)][..],
                ),
                (
                    CardId::Fetch,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("fetch"), Arg::I(6), Arg::I(1)][..],
                ),
                (
                    CardId::Flatten,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(12)][..],
                ),
                (
                    CardId::Flatten,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(16)][..],
                ),
                (
                    CardId::HighFive,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("aoe_vulnerable"), Arg::I(11), Arg::I(2)][..],
                ),
                (
                    CardId::HighFive,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("aoe_vulnerable"), Arg::I(13), Arg::I(3)][..],
                ),
                (
                    CardId::Poke,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(6)][..],
                ),
                (
                    CardId::Poke,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(9)][..],
                ),
                (
                    CardId::Protector,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single_max_hp"), Arg::I(10), Arg::I(1)][..],
                ),
                (
                    CardId::Protector,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single_max_hp"), Arg::I(15), Arg::I(1)][..],
                ),
                (
                    CardId::RightHandHand,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(4)][..],
                ),
                (
                    CardId::RightHandHand,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(6)][..],
                ),
                (
                    CardId::Sacrifice,
                    0,
                    StepKind::SacrificeOstyExact,
                    &[Arg::I(3)][..],
                ),
                (
                    CardId::Sacrifice,
                    1,
                    StepKind::SacrificeOstyExact,
                    &[Arg::I(3)][..],
                ),
                (
                    CardId::SicEm,
                    0,
                    StepKind::SicEmExact,
                    &[Arg::I(5), Arg::I(3)][..],
                ),
                (
                    CardId::SicEm,
                    1,
                    StepKind::SicEmExact,
                    &[Arg::I(6), Arg::I(4)][..],
                ),
                (
                    CardId::Snap,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(7)][..],
                ),
                (
                    CardId::Snap,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single"), Arg::I(10)][..],
                ),
                (
                    CardId::Spur,
                    0,
                    StepKind::OstySummonHeal,
                    &[Arg::I(3), Arg::I(5)][..],
                ),
                (
                    CardId::Spur,
                    1,
                    StepKind::OstySummonHeal,
                    &[Arg::I(5), Arg::I(7)][..],
                ),
                (
                    CardId::Squeeze,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single_osty_cards"), Arg::I(25), Arg::I(5)][..],
                ),
                (
                    CardId::Squeeze,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single_osty_cards"), Arg::I(30), Arg::I(6)][..],
                ),
                (
                    CardId::SweepingGaze,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("random"), Arg::I(10)][..],
                ),
                (
                    CardId::SweepingGaze,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("random"), Arg::I(15)][..],
                ),
                (
                    CardId::Unleash,
                    0,
                    StepKind::OstyBody,
                    &[Arg::S("single_current_hp"), Arg::I(6), Arg::I(1)][..],
                ),
                (
                    CardId::Unleash,
                    1,
                    StepKind::OstyBody,
                    &[Arg::S("single_current_hp"), Arg::I(9), Arg::I(1)][..],
                ),
            ]
        );
    }

    #[test]
    fn missing_pet_still_validates_the_complete_source_body_before_no_op() {
        let identity = CardIdentity {
            id: CardId::Poke,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        let before = state.clone();
        let mut events = Vec::new();
        let wrong_same_shape = [CompiledArg::Word(StepWord::Single), CompiledArg::I(4)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &wrong_same_shape,
            events: &mut events,
        };

        assert_eq!(
            osty_body(&mut ctx),
            Err(EngineRefusal::MalformedArgs("osty_body"))
        );
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());

        ctx.args = catalog.args(catalog.steps(&spec)[0].args);
        osty_body(&mut ctx).unwrap();
        assert_eq!(*ctx.state, before);
        assert!(ctx.events.is_empty());
    }

    #[test]
    fn sic_em_power_is_admitted_with_its_pet_damage_reader() {
        assert!(IMPLEMENTED_POWERS.contains(&PowerId::SicEm));
        assert!(
            crate::engine::capability_manifest()
                .powers
                .contains(&PowerId::SicEm)
        );
    }

    #[test]
    fn spur_rereads_the_live_pet_between_growth_and_capped_heal() {
        let live_enemy = || HotMonster::new(MonsterKind::Toadpole, 50);
        let mut fresh = HotState::at_defaults();
        fresh.hp = 50;
        fresh.monsters_mut().push(live_enemy());
        execute(&mut fresh, CardId::Spur, 0, None).unwrap();
        assert_eq!(
            fresh
                .fanouts
                .pet()
                .osty()
                .map(|osty| (osty.hp(), osty.max_hp())),
            Some((3, 3))
        );

        let mut living = HotState::at_defaults();
        living.hp = 50;
        living.monsters_mut().push(live_enemy());
        living.fanouts.set_osty(Some((2, 5))).unwrap();
        execute(&mut living, CardId::Spur, 0, None).unwrap();
        assert_eq!(
            living
                .fanouts
                .pet()
                .osty()
                .map(|osty| (osty.hp(), osty.max_hp())),
            Some((8, 8)),
            "grow 2/5 -> 5/8, then the distinct heal reaches 8/8"
        );
    }

    /// #3246: with no live enemy the IsEnding projection holds without
    /// `history.over`. Both of Spur's heals (the summon's and the separate
    /// `CreatureCmd.Heal`, `0x3eb4b0` IL_0041-IL_005f) return early, so a
    /// living Osty only gains MaxHp; a dead or absent one refuses by name.
    #[test]
    fn spur_while_ending_grows_max_hp_and_skips_both_heals() {
        let mut living = HotState::at_defaults();
        living.hp = 50;
        living.fanouts.set_osty(Some((2, 5))).unwrap();
        assert!(!living.history.over);
        execute(&mut living, CardId::Spur, 0, None).unwrap();
        assert_eq!(
            living
                .fanouts
                .pet()
                .osty()
                .map(|osty| (osty.hp(), osty.max_hp())),
            Some((2, 8))
        );

        let mut absent = HotState::at_defaults();
        absent.hp = 50;
        let before = absent.clone();
        assert_eq!(
            execute(&mut absent, CardId::Spur, 0, None),
            Err(EngineRefusal::EndingSummonNotModeled("Osty summon"))
        );
        assert_eq!(absent.fanouts.pet(), before.fanouts.pet());
    }

    #[test]
    fn fetch_draws_only_until_the_exact_source_uid_has_finished() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        for uid in 20..24 {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom: 0,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }

        // CardsVar(1): one card, not three (#2519, Fetch 0xdfa45 IL_0017).
        execute(&mut state, CardId::Fetch, 0, Some(0)).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);

        state.fanouts.record_fetch_finished(7);
        execute(&mut state, CardId::Fetch, 0, Some(0)).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert_eq!(state.fanouts.pet().attacks_this_turn(), 2);
    }

    #[test]
    fn sacrifice_freezes_max_hp_kills_then_gains_powered_block() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.fanouts.set_osty(Some((4, 7))).unwrap();
        execute(&mut state, CardId::Sacrifice, 0, None).unwrap();
        assert!(state.fanouts.pet().osty().is_none());
        assert_eq!(state.block, 21);

        let mut missing = HotState::at_defaults();

        missing.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        missing.hp = 50;
        let before = missing.clone();
        assert!(
            execute(&mut missing, CardId::Sacrifice, 1, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(missing, before);
    }

    #[test]
    fn sic_em_applies_without_a_pet_and_attack_modes_use_live_pet_hp() {
        let mut missing = HotState::at_defaults();
        missing.hp = 50;
        missing
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        execute(&mut missing, CardId::SicEm, 0, Some(0)).unwrap();
        assert_eq!(missing.monsters[0].hp, 50);
        assert_eq!(missing.monsters[0].powers.value(PowerId::SicEm), 3);

        let mut protector = HotState::at_defaults();
        protector.hp = 50;
        protector
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        protector.fanouts.set_osty(Some((4, 7))).unwrap();
        execute(&mut protector, CardId::Protector, 0, Some(0)).unwrap();
        assert_eq!(protector.monsters[0].hp, 33, "10 + live max HP 7");
        assert_eq!(protector.fanouts.pet().attacks_this_turn(), 1);

        let mut unleash = HotState::at_defaults();
        unleash.hp = 50;
        unleash
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        unleash.fanouts.set_osty(Some((4, 7))).unwrap();
        execute(&mut unleash, CardId::Unleash, 0, Some(0)).unwrap();
        assert_eq!(unleash.monsters[0].hp, 40, "6 + live current HP 4");
    }

    /// #3515: Fetch's draw is `CardPileCmd.Draw` (`0x39de84` IL_0120), whose
    /// `<DrawInternal>d__21` (0x3e3a70 IL_002e) returns at `IsOverOrEnding`.
    /// While the combat is ending before the over latch Osty's attack and the
    /// draw are both no-ops; the Adaptable-vetoed control attacks and draws.
    #[test]
    fn fetch_draws_nothing_while_combat_is_ending_before_the_over_latch() {
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.fanouts.set_osty(Some((5, 5))).unwrap();
        for uid in 20..22 {
            template
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(HotCard {
                    uid,
                    atom: 0,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
        }
        crate::engine::damage::assert_ending_window_gate(&template, "Fetch", |s, t| {
            execute(s, CardId::Fetch, 0, Some(t)).map(|_| ())
        });
    }
}
