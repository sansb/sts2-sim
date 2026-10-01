//! Card-step bodies for the `content/cards/physical_cost.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Engine-slice status (#1489): 5 of 5 ported
//!
//! All five bodies read the active card's slot-7 damage-growth field. Claw and
//! Maul grow every matching physical card across all five live piles, Rampage
//! grows only its active instance, and Momentum Strike appends one ordered
//! combat-long local-cost row to that same instance. The all-pile walk is
//! reached only from a live Claw/Maul body, so ordinary card plays do not pay
//! for five-pile scanning.

use super::StepCtx;
use crate::catalog::{CardIdentity, Catalog, CompiledArg};
use crate::engine::EngineRefusal;
use crate::engine::damage::player_attack_from_card;
use crate::engine::play::unique_live_card_location;
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, LocalCostExpiration, LocalCostModifier,
    LocalCostModifierKind, PileId,
};
use crate::ids::{CardId, StepKind};

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::ClawExact,
    StepKind::MaulExact,
    StepKind::MomentumStrikeExact,
    StepKind::PhysicalDamageAttack,
    StepKind::RampageExact,
];

/// Python's `_ALL_CARD_PILES` source order (frozen Python, deleted #2827).
///
/// Keep this local to the physical rewrite: [`PileId::ALL`] is the canonical
/// Rust storage order, which is intentionally different.
const PHYSICAL_GROWTH_PILE_ORDER: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

fn card_id(identity: CardIdentity) -> CardId {
    let CardIdentity { id, .. } = identity;
    id
}

pub(crate) fn validate_active_physical_source(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    expected: CardId,
) -> Result<(PileId, usize, HotCard), EngineRefusal> {
    let (pile, index) = unique_live_card_location(state, source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let active = state.piles.get(pile).as_slice()[index];
    let identity = catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?
        .identity;
    if card_id(identity) != expected || !matches!(identity.upgrade, 0 | 1) {
        return Err(EngineRefusal::MalformedArgs("physical damage active card"));
    }
    Ok((pile, index, active))
}

fn active_card(
    ctx: &StepCtx<'_>,
    expected: CardId,
) -> Result<(PileId, usize, HotCard), EngineRefusal> {
    validate_active_physical_source(ctx.state, ctx.catalog, ctx.source_uid, expected)
}

fn active_growth(ctx: &StepCtx<'_>, expected: CardId) -> Result<i32, EngineRefusal> {
    let (_, _, active) = active_card(ctx, expected)?;
    Ok(ctx.state.card_states.get(active.uid).damage_growth)
}

fn damage_with_growth(base: i64, growth: i32) -> Result<i64, EngineRefusal> {
    base.checked_add(i64::from(growth))
        .ok_or(EngineRefusal::CounterOverflow("physical damage"))
}

fn int_arg(ctx: &StepCtx<'_>, index: usize) -> i64 {
    match ctx.args[index] {
        CompiledArg::I(value) => value,
        _ => unreachable!("the exact row validator accepted a non-integer argument"),
    }
}

fn exact_active_row(ctx: &StepCtx<'_>, id: CardId, rows: &[(u8, &[i64])]) -> bool {
    rows.iter().any(|(upgrade, args)| {
        card_id(ctx.spec.identity) == id
            && ctx.spec.identity.upgrade == *upgrade
            && ctx.args.len() == args.len()
            && ctx.args.iter().zip(*args).all(
                |(actual, expected)| matches!(actual, CompiledArg::I(value) if value == expected),
            )
    })
}

fn add_growth_to_card(
    ctx: &mut StepCtx<'_>,
    pile: PileId,
    index: usize,
    delta: i32,
) -> Result<(), EngineRefusal> {
    let card = ctx.state.piles.get_mut(pile).make_mut()[index];
    ctx.state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    ctx.state
        .card_states
        .add_damage_growth(card.uid, delta)
        .ok_or(EngineRefusal::CounterOverflow("physical damage growth"))?;
    Ok(())
}

/// `_map_card_id_in_all_piles(..., force_exact=True)` for one growth class.
///
/// The call is body-gated on a live Claw/Maul. It preflights every matching
/// card before publishing any replacement, then mutates piles in Python's
/// Hand/Draw/Discard/Exhaust/Play source order without allocating a uid list.
fn grow_card_id_in_all_piles(
    ctx: &mut StepCtx<'_>,
    id: CardId,
    delta: i32,
) -> Result<(), EngineRefusal> {
    for pile in PHYSICAL_GROWTH_PILE_ORDER {
        for card in ctx.state.piles.get(pile).as_slice() {
            let identity = ctx
                .catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?
                .identity;
            if card_id(identity) == id
                && ctx
                    .state
                    .card_states
                    .get(card.uid)
                    .damage_growth
                    .checked_add(delta)
                    .is_none()
            {
                return Err(EngineRefusal::CounterOverflow("physical damage growth"));
            }
        }
    }

    let (piles, card_states) = (&mut ctx.state.piles, &mut ctx.state.card_states);
    let mut changed = false;
    for pile in PHYSICAL_GROWTH_PILE_ORDER {
        let has_match = piles.get(pile).as_slice().iter().any(|card| {
            ctx.catalog
                .spec(card.atom)
                .is_some_and(|spec| card_id(spec.identity) == id)
        });
        if !has_match {
            continue;
        }
        for card in piles.get_mut(pile).make_mut() {
            let identity = ctx
                .catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?
                .identity;
            if card_id(identity) == id {
                card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                card_states
                    .add_damage_growth(card.uid, delta)
                    .expect("the complete all-pile growth write was preflighted");
                changed = true;
            }
        }
    }
    if changed {
        ctx.state.exact_piles = true;
    }
    Ok(())
}

fn promote_if_active_tie(ctx: &mut StepCtx<'_>, source: HotCard) -> Result<(), EngineRefusal> {
    if ctx.state.exact_piles {
        return Ok(());
    }
    let identity = ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    for pile in PileId::ALL {
        for card in ctx.state.piles.get(pile).as_slice() {
            if card.uid == source.uid {
                continue;
            }
            let peer = ctx
                .catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?
                .identity;
            if (peer.id, peer.upgrade) == (identity.id, identity.upgrade) {
                ctx.state.exact_piles = true;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Append one exact local-cost row to the current physical source.
///
/// Native active-card writers re-resolve the source CardModel after earlier
/// awaited steps, write the slot-7 default marker with the ordered row, and
/// promote only when that changed payload distinguishes an otherwise-equal
/// live card. Momentum Strike and Modded share that complete lifecycle.
pub(crate) fn append_active_local_cost_modifier(
    ctx: &mut StepCtx<'_>,
    expected: CardId,
    modifier: LocalCostModifier,
) -> Result<(), EngineRefusal> {
    let (pile, index, active) = active_card(ctx, expected)?;
    ctx.state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    ctx.state
        .card_states
        .append_local_cost_modifier(active.uid, modifier);
    promote_if_active_tie(ctx, active)
}

/// Python: `_run_steps_inner` (frozen, deleted #2827) — attack with the active card's live
/// damage growth, then `_map_card_id_in_all_piles` applies that card level's
/// growth increase to every physical Claw.
pub(crate) fn claw_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !exact_active_row(ctx, CardId::Claw, &[(0, &[3, 2]), (1, &[4, 3])]) {
        return Err(EngineRefusal::MalformedArgs("claw_exact"));
    }
    let growth = active_growth(ctx, CardId::Claw)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage_with_growth(int_arg(ctx, 0), growth)?,
        1,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(());
    }
    let (_, _, active) = active_card(ctx, CardId::Claw)?;
    let upgrade = ctx
        .catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?
        .identity
        .upgrade;
    grow_card_id_in_all_piles(ctx, CardId::Claw, [2, 3][usize::from(upgrade)])
}

/// Python: `_run_steps_inner` (frozen, deleted #2827) — attack twice with the active card's
/// live damage growth, then `_map_card_id_in_all_piles` applies that card
/// level's growth increase to every physical Maul.
pub(crate) fn maul_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !exact_active_row(ctx, CardId::Maul, &[(0, &[5, 2]), (1, &[6, 3])]) {
        return Err(EngineRefusal::MalformedArgs("maul_exact"));
    }
    let growth = active_growth(ctx, CardId::Maul)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage_with_growth(int_arg(ctx, 0), growth)?,
        2,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(());
    }
    let (_, _, active) = active_card(ctx, CardId::Maul)?;
    let upgrade = ctx
        .catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?
        .identity
        .upgrade;
    grow_card_id_in_all_piles(ctx, CardId::Maul, [2, 3][usize::from(upgrade)])
}

/// Python: `_run_steps_inner` (frozen, deleted #2827) — attack with the active card's live
/// damage growth, re-read the same physical card after the awaited attack,
/// then append a Set-to-zero, ThisCombat local-cost modifier to it.
pub(crate) fn momentum_strike_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !exact_active_row(ctx, CardId::MomentumStrike, &[(0, &[11]), (1, &[15])]) {
        return Err(EngineRefusal::MalformedArgs("momentum_strike_exact"));
    }
    let growth = active_growth(ctx, CardId::MomentumStrike)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage_with_growth(int_arg(ctx, 0), growth)?,
        1,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(());
    }
    append_active_local_cost_modifier(
        ctx,
        CardId::MomentumStrike,
        LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount: 0,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    )
}

/// Python: `_run_steps_inner` (frozen, deleted #2827) — Kingly Punch reads the active physical
/// card's live damage growth and adds it to the attack's base damage.
pub(crate) fn physical_damage_attack(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !exact_active_row(ctx, CardId::KinglyPunch, &[(0, &[8, 1]), (1, &[10, 1])]) {
        return Err(EngineRefusal::MalformedArgs("physical_damage_attack"));
    }
    let growth = active_growth(ctx, CardId::KinglyPunch)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage_with_growth(int_arg(ctx, 0), growth)?,
        int_arg(ctx, 1),
        ctx.events,
    )
}

/// Python: `_run_steps_inner` (frozen, deleted #2827) — attack with the active card's live
/// damage growth, re-read the same physical card after the awaited attack,
/// then increase that active card's growth for later plays.
pub(crate) fn rampage_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !exact_active_row(ctx, CardId::Rampage, &[(0, &[10, 5]), (1, &[10, 10])]) {
        return Err(EngineRefusal::MalformedArgs("rampage_exact"));
    }
    let growth = active_growth(ctx, CardId::Rampage)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage_with_growth(int_arg(ctx, 0), growth)?,
        1,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(());
    }
    let (pile, index, active) = active_card(ctx, CardId::Rampage)?;
    let upgrade = ctx
        .catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?
        .identity
        .upgrade;
    let delta = [5, 10][usize::from(upgrade)];
    ctx.state
        .card_states
        .get(active.uid)
        .damage_growth
        .checked_add(delta)
        .ok_or(EngineRefusal::CounterOverflow("physical damage growth"))?;
    add_growth_to_card(ctx, pile, index, delta)?;
    promote_if_active_tie(ctx, active)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::engine::play::play_card;
    use crate::hot::{HotMonster, HotState};
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;

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

    fn combat_state(monster: HotMonster) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.monsters_mut().push(monster);
        state
    }

    #[test]
    fn the_complete_physical_cost_family_is_claimed_in_generated_order() {
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::ClawExact,
                StepKind::MaulExact,
                StepKind::MomentumStrikeExact,
                StepKind::PhysicalDamageAttack,
                StepKind::RampageExact,
            ]
        );
    }

    #[test]
    fn all_pile_growth_rewrite_pins_python_source_order() {
        assert_eq!(
            PHYSICAL_GROWTH_PILE_ORDER,
            [
                PileId::Hand,
                PileId::Draw,
                PileId::Discard,
                PileId::Exhaust,
                PileId::Play,
            ]
        );
    }

    #[test]
    fn claw_rereads_a_changed_but_valid_source_upgrade_without_narrowing() {
        let mut builder = CatalogBuilder::new();
        let level_zero = builder.intern(identity(CardId::Claw, 0)).unwrap();
        builder.intern(identity(CardId::Claw, 1)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(level_zero).unwrap();
        let mut state = combat_state(HotMonster::new(MonsterKind::Toadpole, 100));
        state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(card(&catalog, CardId::Claw, 1, 1));
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3), CompiledArg::I(2)],
            events: &mut events,
        };

        claw_exact(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].hp, 97);
        assert_eq!(
            state.card_states.get(1).damage_growth,
            3,
            "the post-attack growth follows the re-resolved level-one source"
        );
    }

    #[test]
    fn claw_grows_every_live_copy_in_all_five_piles_and_forces_exact_mode() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Claw, 0)).unwrap();
        let catalog = builder.build();
        let mut state = combat_state(HotMonster::new(MonsterKind::Toadpole, 100));
        let source = card(&catalog, CardId::Claw, 0, 1);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, card(&catalog, CardId::Claw, 0, 2)]);
        for (pile, uid) in [
            (PileId::Draw, 3),
            (PileId::Discard, 4),
            (PileId::Exhaust, 5),
            (PileId::Play, 6),
        ] {
            state
                .piles
                .get_mut(pile)
                .make_mut()
                .push(card(&catalog, CardId::Claw, 0, uid));
        }
        for uid in 1..=6 {
            state
                .card_states
                .add_damage_growth(uid, uid as i32)
                .unwrap();
        }

        play_card(&mut state, &catalog, 1, Some(0), None, &mut Vec::new()).unwrap();

        assert_eq!(
            state.monsters[0].hp, 96,
            "the source's live +1 growth is read"
        );
        for uid in 1..=6 {
            assert_eq!(state.card_states.get(uid).damage_growth, uid as i32 + 2);
        }
        assert!(state.exact_piles);
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|candidate| candidate.uid == source.uid)
        );
    }

    #[test]
    fn momentum_strike_appends_on_each_generated_replay_without_repaying() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::MomentumStrike, 0)).unwrap();
        let catalog = builder.build();
        let mut state = combat_state(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().push(card(
            &catalog,
            CardId::MomentumStrike,
            0,
            7,
        ));
        state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        play_card(&mut state, &catalog, 7, Some(0), None, &mut Vec::new()).unwrap();

        assert_eq!(state.monsters[0].hp, 78);
        assert_eq!(
            state.energy, 8,
            "the replay does not repay the printed cost"
        );
        let rows = state
            .card_states
            .get_ref(7)
            .unwrap()
            .local_cost_modifiers
            .as_slice();
        assert_eq!(rows.len(), 2, "each body appends rather than folding");
        assert!(rows.iter().all(|row| {
            row.kind == LocalCostModifierKind::Set
                && row.amount == 0
                && row.expiration == LocalCostExpiration::ThisCombat
                && !row.reduce_only
        }));
    }

    #[test]
    fn momentum_appends_after_an_awaited_source_upgrade_change() {
        let mut builder = CatalogBuilder::new();
        let level_zero = builder.intern(identity(CardId::MomentumStrike, 0)).unwrap();
        builder.intern(identity(CardId::MomentumStrike, 1)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(level_zero).unwrap();
        let mut state = combat_state(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Play).make_mut().push(card(
            &catalog,
            CardId::MomentumStrike,
            1,
            7,
        ));
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(11)],
            events: &mut events,
        };

        momentum_strike_exact(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].hp, 89);
        assert_eq!(
            state.card_states.get(7).local_cost_modifiers.as_slice(),
            [LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        );
    }

    #[test]
    fn beat_down_autoplay_grows_rampage_after_waterfall_first_stage_death() {
        let mut builder = CatalogBuilder::new();
        let beat_atom = builder.intern(identity(CardId::BeatDown, 0)).unwrap();
        builder.intern(identity(CardId::Rampage, 0)).unwrap();
        let catalog = builder.build();
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 1);
        waterfall.max_hp = 250;
        waterfall.pressure_gun_damage = 23;
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 29);
        let mut state = combat_state(waterfall);
        state.piles.get_mut(PileId::Discard).make_mut().push(card(
            &catalog,
            CardId::Rampage,
            0,
            11,
        ));
        let spec = *catalog.spec(beat_atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 99,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };

        crate::engine::play::queue_beat_down_batch(&mut ctx, 1).unwrap();

        assert_eq!(ctx.state.monsters[0].hp, 999_999_999);
        assert_eq!(
            ctx.state.monsters[0].powers.value(PowerId::SteamPressure),
            29
        );
        assert_eq!(ctx.state.card_states.get(11).damage_growth, 5);
        assert!(
            ctx.state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|candidate| candidate.uid == 11)
        );
        assert!(
            !ctx.state.history.over,
            "Waterfall's sentinel remains alive"
        );
    }

    #[test]
    fn player_death_suppresses_the_post_attack_growth_tail() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Rampage, 0)).unwrap();
        let catalog = builder.build();
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        let mut state = combat_state(monster);
        state.hp = 1;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::Rampage, 0, 17));

        play_card(&mut state, &catalog, 17, Some(0), None, &mut Vec::new()).unwrap();

        assert!(state.hp <= 0);
        assert_eq!(state.card_states.get(17).damage_growth, 0);
        assert!(
            state
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .any(|candidate| candidate.uid == 17)
        );
    }
}
