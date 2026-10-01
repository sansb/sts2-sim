//! Card-step bodies for the `content/cards/ironclad_rare.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1325/#1562/#1751/#1884): 7 of 9 ported, 2 escalated
//!
//! Fiend Fire's fused physical-Hand snapshot, serial Exhaust lifecycle, and
//! preserved-count attack are expressible with the synchronous engine
//! primitives. #1562 adds Cascade's exact X-counted direct AutoPlay. #1751
//! adds Primal Force's physical same-index transform and Crimson Mantle's
//! paired persistent amount/private self-damage turn-start listener. The other three kinds
//! remain refused for the concrete missing
//! admission, hot-state, boundary, generation, multiplayer, or preview
//! surfaces documented at their stubs.

use super::StepCtx;
use crate::catalog::{CardIdentity, CardSpec, Catalog, CompiledArg};
use crate::decimal::DotNetDecimal;
use crate::engine::cards::{inject_before_hand_draw_generated_bottom, sample_generation_slice};
use crate::engine::draw::{
    preflight_card_play_after_physical_moves, repair_card_play_after_physical_move,
};
use crate::engine::{
    EngineRefusal, Event, Subject,
    damage::{
        player_attack_all_from_card, player_attack_decimal_from_card, player_attack_from_card,
    },
};
use crate::hot::{
    CARD_FLAG_LEGACY, CardInstanceState, HotCard, HotState, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, PowerId, StepKind, StepWord};
use crate::powers::SlotWire;
use crate::rng::Xoshiro256StarStar;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::AutoplayDrawX,
    StepKind::CrimsonMantle,
    StepKind::FiendFireExact,
    StepKind::HeavenlyDrillX,
    StepKind::PactsEndExact,
    StepKind::PrimalForceExact,
    StepKind::StokeExact,
    StepKind::ThrashExact,
];

fn exact_active_crimson_mantle(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<i32, EngineRefusal> {
    let amount = match (spec.identity.id, spec.identity.upgrade, catalog.steps(spec)) {
        (CardId::CrimsonMantle, 0, [step])
            if step.kind == StepKind::CrimsonMantle
                && matches!(catalog.args(step.args), [CompiledArg::I(7)]) =>
        {
            7
        }
        (CardId::CrimsonMantle, 1, [step])
            if step.kind == StepKind::CrimsonMantle
                && matches!(catalog.args(step.args), [CompiledArg::I(10)]) =>
        {
            10
        }
        _ => return Err(EngineRefusal::MalformedArgs("crimson_mantle owner")),
    };
    let mut matches = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .filter(move |card| card.uid == source_uid)
    });
    let Some(source) = matches.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let additional = matches.count();
    if additional != 0 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 1 + additional,
        });
    }
    if catalog.spec(source.atom).map(|row| row.identity) != Some(spec.identity) {
        return Err(EngineRefusal::MalformedArgs("crimson_mantle source"));
    }
    Ok(amount)
}

/// Validate both Crimson Mantle counters before resource/source mutation.
pub(crate) fn preflight_crimson_mantle(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    let amount = exact_active_crimson_mantle(state, catalog, source_uid, spec)?;
    state
        .powers
        .value(PowerId::CrimsonMantle)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("crimson_mantle"))?;
    state
        .powers
        .value(PowerId::CrimsonSelf)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("crimson_self"))?;
    Ok(())
}

fn two_ints(ctx: &StepCtx<'_>, site: &'static str) -> Result<(i64, i64), EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(first), CompiledArg::I(second)] => Ok((*first, *second)),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn exact_active_fiend_fire(ctx: &StepCtx<'_>) -> Result<HotCard, EngineRefusal> {
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
    let identity = ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    if identity != ctx.spec.identity
        || identity.id != CardId::FiendFire
        || !matches!(identity.upgrade, 0 | 1)
    {
        return Err(EngineRefusal::MalformedArgs("fiend_fire_exact"));
    }
    Ok(source)
}

/// `autoplay_draw_x` — Cascade's resolved Energy-X top-card AutoPlay batch.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) validates Cascade, then calls
/// `_havoc_flip(count=x_value + level, force_exhaust=False)`.
///
/// The shared gate admits exactly Cascade's two canonical negative-cost rows;
/// every other X card retains the existing cost convention. The play pipeline
/// has already captured and spent Energy-X in [`StepCtx::x_value`], so the
/// body only adds the level bonus and delegates the exact Top / natural-result
/// transaction.
pub(crate) fn autoplay_draw_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(level)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("autoplay_draw_x"));
    };
    if ctx.spec.identity.id != CardId::Cascade
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *level != i64::from(ctx.spec.identity.upgrade)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("autoplay_draw_x owner"));
    }
    let count: usize = ctx
        .x_value
        .checked_add(*level)
        .ok_or(EngineRefusal::CounterOverflow("autoplay_draw_x count"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("autoplay_draw_x count"))?;
    crate::engine::play::autoplay_draw_top_restricted_parent(
        ctx.state,
        ctx.catalog,
        count,
        false,
        ctx.events,
    )
}

/// `crimson_mantle` — Crimson Mantle's persistent turn-start pair.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) adds `(7, 10)[level]` to Crimson
/// Mantle and one to its private self counter. `_turn_start_after_inferno` applies that counter as self HP loss, then the Mantle amount
/// as flat block, in the native `AfterPlayerTurnStart` peer order.
///
/// Both values are validated before either write. The turn-start reader
/// consumes the private self counter first and only then gains the public
/// amount as flat Block; admission refuses every reachable same-hook peer
/// because canonical state does not retain their first-application order.
///
/// `CrimsonMantle/<OnPlay>d__5` RVA `0x395980` awaits
/// `PowerCmd.Apply<CrimsonMantlePower>` (IL_00e2) and increments the private
/// counter only on the non-null result (IL_0141-0145). `<Apply>d__1`1`
/// (`0x3ef988`) returns null at `IsEnding` (IL_0025-002a), so both writes follow
/// the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn crimson_mantle(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = exact_active_crimson_mantle(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    if ctx.args != [CompiledArg::I(i64::from(amount))]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("crimson_mantle"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let mantle = ctx
        .state
        .powers
        .value(PowerId::CrimsonMantle)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("crimson_mantle"))?;
    let self_damage = ctx
        .state
        .powers
        .value(PowerId::CrimsonSelf)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("crimson_self"))?;
    if ctx.state.powers.value(PowerId::CrimsonMantle) <= 0
        && !ctx
            .state
            .register_after_player_turn_start(PowerId::CrimsonMantle)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-player-turn-start listener order",
        ));
    }
    for (power, value) in [
        (PowerId::CrimsonMantle, mantle),
        (PowerId::CrimsonSelf, self_damage),
    ] {
        ctx.state.powers.set(power, SlotWire::Int, value);
        crate::engine::damage::note_power(ctx.events, Subject::Player, power, value);
    }
    Ok(())
}

/// `("fiend_fire_exact", damage)` — Fiend Fire.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) snapshots the post-activation Hand's
/// physical identities and count once, exhausts those exact cards serially in
/// Hand order through `_card_exhausted`, and, if combat remains live, issues
/// one targeted powered attack with the preserved count. Cards drawn by an
/// exhaust hook are not added to the frozen batch; a reentrant ending hook
/// stops the batch and suppresses the attack. An empty snapshot still issues
/// a real zero-hit attack command.
///
pub(crate) fn fiend_fire_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::FiendFire, 0, [CompiledArg::I(7)]) => 7,
        (CardId::FiendFire, 1, [CompiledArg::I(10)]) => 10,
        _ => return Err(EngineRefusal::MalformedArgs("fiend_fire_exact")),
    };
    exact_active_fiend_fire(ctx)?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let frozen: Vec<HotCard> = ctx.state.piles.get(PileId::Hand).as_slice().to_vec();
    let hits = i64::try_from(frozen.len())
        .map_err(|_| EngineRefusal::CounterOverflow("fiend_fire_exact hits"))?;
    let repairs: Vec<_> = frozen
        .iter()
        .copied()
        .map(|card| (card, PileId::Hand, PileId::Exhaust))
        .collect();
    preflight_card_play_after_physical_moves(ctx.state, &repairs)?;
    continue_fiend_fire(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
        if crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(ctx.state, ctx.catalog)
        {
            crate::engine::play::after_card_exhausted_source_step_cursor(ctx.state, ctx.source_uid)?
        } else {
            0
        },
        &frozen.iter().map(|card| card.uid).collect::<Vec<_>>(),
        damage,
        u32::try_from(hits).map_err(|_| EngineRefusal::CounterOverflow("fiend_fire_exact hits"))?,
        freeze_monster_identity(ctx.state, target)?,
        ctx.events,
    )
}

fn freeze_monster_identity(
    state: &HotState,
    index: usize,
) -> Result<(i32, u32, Option<i32>), EngineRefusal> {
    let monster = state
        .monsters
        .get(index)
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let duplicates = state
        .monsters
        .iter()
        .filter(|candidate| (candidate.slot, candidate.uid) == (monster.slot, monster.uid))
        .count();
    let fallback = (duplicates > 1)
        .then(|| {
            i32::try_from(index)
                .map_err(|_| EngineRefusal::CounterOverflow("Fiend Fire target index"))
        })
        .transpose()?;
    Ok((monster.slot, monster.uid, fallback))
}

fn resolve_monster_identity(
    state: &HotState,
    frozen: (i32, u32, Option<i32>),
) -> Result<usize, EngineRefusal> {
    let (slot, uid, fallback) = frozen;
    let matches = state
        .monsters
        .iter()
        .enumerate()
        .filter_map(|(index, monster)| {
            ((monster.slot, monster.uid) == (slot, uid)).then_some(index)
        })
        .collect::<Vec<_>>();
    match (
        matches.as_slice(),
        fallback.and_then(|index| usize::try_from(index).ok()),
    ) {
        ([index], None) => Ok(*index),
        (indices, Some(index)) if indices.len() > 1 && indices.contains(&index) => Ok(index),
        _ => Err(EngineRefusal::ContinuationNotModeled),
    }
}

/// `FiendFire/<OnPlay>d__7` RVA `0x39e0dc`
/// awaits `CardCmd.Exhaust` per frozen card (IL_00a4); `<Exhaust>d__6` (`0x3e06c8`)
/// returns at `IsOverOrEnding` (IL_0025-002a) before its `CardPileCmd.Add`, so
/// an exhaust listener that ends the combat stops the walk before the next card
/// leaves Hand: the shared IsOverOrEnding projection, not `history.over`
/// (#3515).
#[allow(clippy::too_many_arguments)]
fn continue_fiend_fire(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    step_index: u32,
    remaining: &[u32],
    damage: i32,
    hits: u32,
    target: (i32, u32, Option<i32>),
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    for (cursor, frozen_uid) in remaining.iter().copied().enumerate() {
        if crate::engine::damage::damage_combat_is_ending(state) {
            break;
        }
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        let index = hand.iter().position(|card| card.uid == frozen_uid).ok_or(
            EngineRefusal::FrozenCardVanished {
                uid: frozen_uid,
                pile: PileId::Hand,
            },
        )?;
        let card = hand[index];
        let _ = hand;
        repair_card_play_after_physical_move(state, card, PileId::Hand, PileId::Exhaust)?;
        let card = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
            crate::hot::AfterCardExhaustedReturnKind::FiendFire,
        );
        continuation.source_uid = Some(source_uid);
        continuation.step_index = step_index;
        continuation.amount = damage;
        continuation.count = hits;
        continuation.target = Some(target);
        continuation.remaining = remaining[cursor + 1..].to_vec();
        let owner_live =
            crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog);
        let result = crate::engine::draw::card_exhausted_with_owner(
            state,
            catalog,
            card,
            continuation,
            events,
        )?;
        if owner_live || result == crate::engine::draw::CardExhaustedResult::Suspended {
            return Ok(());
        }
    }
    if !state.history.over {
        let target_index = resolve_monster_identity(state, target)?;
        let (pile, source_index) =
            crate::engine::play::unique_live_card_location(state, source_uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[source_index];
        let spec = catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
        let ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid,
            target: Some(target_index),
            selection: None,
            x_value: 0,
            args: &[],
            events,
        };
        player_attack_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[target_index],
            i64::from(damage),
            i64::from(hits),
            ctx.events,
        )?;
    }
    Ok(())
}

pub(crate) fn resume_fiend_fire_after_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let source_uid = record
        .source_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let target = record.target.ok_or(EngineRefusal::ContinuationNotModeled)?;
    if !record.remaining.is_empty() {
        state
            .frames
            .pop_top_after_card_exhausted_power()
            .filter(|completed| *completed == *record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        return continue_fiend_fire(
            state,
            catalog,
            source_uid,
            record.step_index,
            &record.remaining,
            record.amount,
            record.count,
            target,
            events,
        );
    }
    if record.flags == 0 && !state.history.over {
        let mut after_attack = record.clone();
        after_attack.flags = 1;
        state
            .frames
            .replace_top_after_card_exhausted_power(&after_attack)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let target_index = resolve_monster_identity(state, target)?;
        let (pile, source_index) =
            crate::engine::play::unique_live_card_location(state, source_uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[source_index];
        let spec = catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
        let ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid,
            target: Some(target_index),
            selection: None,
            x_value: 0,
            args: &[],
            events,
        };
        player_attack_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[target_index],
            i64::from(record.amount),
            i64::from(record.count),
            ctx.events,
        )?;
        if !matches!(
            state.frames.top(),
            Some(crate::frame::Frame::AfterCardExhaustedPower { .. })
        ) {
            return Ok(());
        }
    }
    state
        .frames
        .pop_top_after_card_exhausted_power()
        .filter(|completed| {
            completed.return_kind == crate::hot::AfterCardExhaustedReturnKind::FiendFire
                && completed.card_uid == record.card_uid
                && (state.history.over || completed.flags == 1)
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    Ok(())
}

/// `("heavenly_drill_x", damage, threshold)` — Heavenly Drill.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The resolved Energy-X is doubled only
/// when it is at least `threshold`, then one ordinary targeted attack command
/// runs with that hit count. The command is still real at X=0.
pub(crate) fn heavenly_drill_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, threshold) = two_ints(ctx, "heavenly_drill_x")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let hits = if ctx.x_value >= threshold {
        ctx.x_value
            .checked_mul(2)
            .ok_or(EngineRefusal::CounterOverflow("heavenly_drill_x hits"))?
    } else {
        ctx.x_value
    };
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        hits,
        ctx.events,
    )
}

/// `("pacts_end_exact", damage, threshold)` — Pacts End.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The live Exhaust-pile length is read
/// when the body runs. At or above the threshold it issues one ordinary
/// all-opponents attack command; below it the step is a no-op.
pub(crate) fn pacts_end_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, threshold) = two_ints(ctx, "pacts_end_exact")?;
    let threshold = usize::try_from(threshold)
        .map_err(|_| EngineRefusal::CounterOverflow("pacts_end_exact threshold"))?;
    if ctx.state.piles.get(PileId::Exhaust).len() < threshold {
        return Ok(());
    }
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        1,
        ctx.events,
    )
}

fn primal_force_program(catalog: &Catalog, spec: &CardSpec) -> Result<CardIdentity, EngineRefusal> {
    let level = match (spec.identity.id, spec.identity.upgrade, catalog.steps(spec)) {
        (
            CardId::PrimalForce,
            level @ 0..=1,
            [
                crate::catalog::CompiledStep {
                    kind: StepKind::PrimalForceExact,
                    args,
                },
            ],
        ) if matches!(catalog.args(*args), [CompiledArg::I(value)] if *value == i64::from(level)) => {
            level
        }
        _ => return Err(EngineRefusal::MalformedArgs("primal_force_exact program")),
    };
    Ok(CardIdentity {
        id: CardId::GiantRock,
        upgrade: level,
        enchantment: None,
    })
}

fn exact_active_primal_force(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<HotCard, EngineRefusal> {
    let mut matches = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(move |card| card.uid == source_uid)
    });
    let Some(source) = matches.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let count = 1 + matches.count();
    if count != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: count,
        });
    }
    let live = catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
    if source.flags & CARD_FLAG_LEGACY != 0
        || live.identity != spec.identity
        || !std::ptr::eq(live.row, spec.row)
        || primal_force_program(catalog, spec).is_err()
    {
        return Err(EngineRefusal::MalformedArgs("primal_force_exact source"));
    }
    Ok(source)
}

/// `PrimalForce/<OnPlay>d__3` RVA `0x3b3990` awaits one plural
/// `CardCmd.Transform` (IL_0124), whose `<Transform>d__13` (`0x3e0ae0`) returns
/// at `IsEnding` (IL_0032-0037): the entry gate is the shared IsEnding
/// projection, not `history.over` (#3515).
fn transform_primal_force_hand(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let replacement_identity = primal_force_program(catalog, spec)?;
    exact_active_primal_force(state, catalog, source_uid, spec)?;
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    let replacement_atom = catalog
        .atom(&replacement_identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(replacement_identity))?;
    let replacement_spec = catalog
        .spec(replacement_atom)
        .ok_or(EngineRefusal::UnknownAtom(replacement_atom))?;
    if replacement_spec.identity != replacement_identity
        || !replacement_spec.is_attack
        || replacement_spec.strike_tag
    {
        return Err(EngineRefusal::MalformedArgs(
            "primal_force_exact Giant Rock",
        ));
    }
    let frozen = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| {
            let candidate = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            Ok(candidate
                .is_attack
                .then(|| (*card, state.card_states.get(card.uid))))
        })
        .collect::<Result<Vec<_>, EngineRefusal>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<(HotCard, CardInstanceState)>>();

    // A Dampen-tracked Attack is transformed like any other (#2982): see
    // `forget_transformed_dampen_card` below. Hopper's persistent deck row
    // still refuses.
    if frozen.iter().any(|(original, _)| {
        crate::engine::monsters::deck_version_link_owns_uid(state, original.uid)
    }) {
        return Err(EngineRefusal::MalformedArgs(
            "persistent encounter-card transform",
        ));
    }

    for (original, original_state) in frozen {
        if state.history.over {
            break;
        }
        let Some((pile, index)) =
            crate::engine::play::unique_live_card_location(state, original.uid)?
        else {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: original.uid,
                pile: PileId::Hand,
            });
        };
        if pile != PileId::Hand
            || state.piles.get(PileId::Hand).as_slice()[index] != original
            || state.card_states.get(original.uid) != original_state
        {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: original.uid,
                pile: PileId::Hand,
            });
        }
        if crate::engine::play::unique_live_card_location(state, state.next_card_uid)?.is_some() {
            return Err(EngineRefusal::MalformedArgs(
                "primal_force_exact next uid collision",
            ));
        }
        let original_spec = catalog
            .spec(original.atom)
            .ok_or(EngineRefusal::UnknownAtom(original.atom))?;
        if original.flags & CARD_FLAG_LEGACY != 0 || !original_spec.is_attack {
            return Err(EngineRefusal::MalformedArgs(
                "primal_force_exact frozen candidate",
            ));
        }

        let mut replacement =
            crate::engine::cards::mint_card(state, catalog, replacement_identity)?;
        state.piles.get_mut(PileId::Hand).make_mut()[index] = replacement;
        state
            .card_states
            .set(original.uid, CardInstanceState::default());
        forget_transformed_dampen_card(state, original.uid);
        if original_spec.strike_tag {
            state.ps_strikes = state
                .ps_strikes
                .checked_sub(1)
                .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
        }
        state.history.owner_generated_cards_combat = state
            .history
            .owner_generated_cards_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat",
            ))?;
        crate::engine::cards::apply_physical_card_after_entered(
            state,
            catalog,
            replacement_spec,
            &mut replacement,
        )?;
        state.piles.get_mut(PileId::Hand).make_mut()[index] = replacement;
        state.next_generated_hook_uid = state
            .next_generated_hook_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        state.exact_piles = true;
        crate::engine::cards::after_owner_card_generated(state, catalog, replacement_spec, events)?;
    }
    Ok(())
}

/// Drop the Dampen row of an Attack that Primal Force just transformed (#2982).
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`):
///
/// * `PrimalForce/<OnPlay>d__3::MoveNext` (RVA `0x3b3990`) IL_00fc-IL_0124
///   creates a fresh `GiantRock`, upgrades it only when Primal Force itself
///   `IsUpgraded` (IL_010f, `ldloc.1` is `this`), and awaits
///   `CardCmd.Transform(original, replacement)`. The replacement is a new
///   `CardModel`; nothing copies the original's upgrade level onto it.
/// * `CardCmd/<Transform>d__13::MoveNext` (RVA `0x3e0ae0`) IL_0186-IL_018c
///   calls `RemoveFromCurrentPile` on the original, IL_03e3-IL_03f1 adds the
///   replacement to the same pile, and IL_0a1d-IL_0a22 finishes with
///   `CardModel::RemoveFromState` (RVA `0x7dbb2`: `RemoveFromCurrentPile`
///   plus `HasBeenRemovedFromState = true`). `CardModel::AfterTransformedFrom`
///   (RVA `0x7dc41`) is an empty body. The original model is left in no pile.
/// * `DampenPower` has no pile or card-entered hook: its only members are
///   `AfterApplied` (RVA `0xa1198`, the one-time downgrade snapshot into
///   `downgradedCardsToOldUpgradeLevels`), `AfterDeath` (`<AfterDeath>d__7`
///   RVA `0x33871c`) and `AfterRemoved` (RVA `0xa1290`). So the dictionary
///   keeps the orphaned original, and never learns about the Giant Rock.
/// * `AfterRemoved` IL_000c-IL_004d later calls `CardCmd.Upgrade(original)`
///   (RVA `0x12f660`). With `Pile == null`, IL_0037-IL_0080 skips the
///   map-history write and IL_0097-IL_00ab skips the VFX, so the only effect
///   is `UpgradeInternal`/`FinalizeUpgradeInternal` on a model no combat
///   pile holds and that cannot re-enter.
///
/// The row therefore has no future combat reader, the same quotient
/// [`crate::engine::cards::forget_removed_dampen_card`] takes for a resolved
/// Power. The Giant Rock keeps Primal Force's level through a Magi death.
fn forget_transformed_dampen_card(state: &mut HotState, uid: u32) {
    crate::engine::cards::forget_removed_dampen_card(state, uid);
}

/// Validate Primal Force before resource/source/CardPlayed mutation.
pub(crate) fn preflight_primal_force(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    exact_active_primal_force(state, catalog, source_uid, spec)?;
    let mut probe = state.clone();
    transform_primal_force_hand(&mut probe, catalog, source_uid, spec, &mut Vec::new())
}

/// `primal_force_exact` — Primal Force's serial same-index transforms.
///
/// Python: `_w181_transform_primal_force_hand` (frozen, deleted #2827) snapshots every
/// transformable non-Eternal Hand attack in order, validates each physical
/// card, then replaces it at the same index with a freshly allocated Giant
/// Rock of the same upgrade level and publishes generated-card history plus
/// exact piles.
///
/// Current v0.111.0 IL authority: `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// `PrimalForce/<OnPlay>d__3::MoveNext` RVA `0x3b3990`, its transformability
/// predicate RVA `0x3b3976`, and `CardCmd/<Transform>d__13::MoveNext` RVA
/// `0x3e0ae0`. The body freezes transformable Hand Attacks in pile order and
/// awaits one fresh Giant Rock replacement at the same index per source.
pub(crate) fn primal_force_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let replacement = primal_force_program(ctx.catalog, ctx.spec)?;
    if !matches!(ctx.args, [CompiledArg::I(level)] if *level == i64::from(replacement.upgrade))
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("primal_force_exact"));
    }
    exact_active_primal_force(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let mut probe = ctx.state.clone();
    let mut produced = Vec::new();
    transform_primal_force_hand(
        &mut probe,
        ctx.catalog,
        ctx.source_uid,
        ctx.spec,
        &mut produced,
    )?;
    *ctx.state = probe;
    ctx.events.extend(produced);
    Ok(())
}

/// `stoke_exact` — exact clone-rehearsed serial Exhaust + generation body.
///
/// Python: `resolve_stoke` (frozen, deleted #2827) snapshots and serially exhausts the exact
/// post-source Hand, then samples exactly that many times with replacement
/// from the current owner's complete unlocked CharacterCardPool using
/// Generation RNG — the Ironclad owner uses the ordered 78-card
/// `STOKE_CARD_POOL_V109` — even after terminal state, before conditionally
/// inserting live results.
///
/// The cold boundary derives the current-owner recursive fixed point. The
/// fixed Ironclad/Puzzlebox witness keeps its 158 L0 and 112 L1-only
/// identities; every other owner closes its own full pool and descendants.
/// Public dispatch is enabled only after that complete catalog closes every
/// generated producer, selector, persistent power, and replay continuation.
pub(crate) fn stoke_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    stoke_foundation(ctx)
}

/// Private exact Stoke command foundation.
///
/// Current v0.111.0 ARM64 authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Stoke::.ctor` RVA `0xecdf3` supplies the canonical one-cost Rare Skill
/// rows and `Stoke::OnPlay` RVA `0xece00` enters the generated state machine.
/// `Stoke/<OnPlay>d__1::MoveNext` RVA `0x3bef44` freezes the post-source Hand
/// and its count, awaits `CardCmd::Exhaust` for each frozen object in order,
/// calls `CardFactory::GetForCombat` for exactly that original count, upgrades
/// the complete fresh list for Stoke+ while combat is live, then begins one
/// plural generated-Hand add. Python `resolve_stoke` (frozen, deleted #2827) mirrors
/// those boundaries and clone-rehearses the complete body. Direct AutoPlay is
/// also source-sensitive: `CardCmd/<AutoPlay>d__5::MoveNext` RVA `0x3df9d4`
/// leaves any non-null source in its original pile through `OnPlayWrapper`,
/// which is why active-source authentication spans every physical pile.
fn stoke_foundation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let upgrade = match (
        ctx.spec.identity.id,
        ctx.spec.identity.upgrade,
        ctx.args,
        ctx.target,
        ctx.selection,
        ctx.x_value,
    ) {
        (CardId::Stoke, upgrade @ 0..=1, [], None, None, 0) => upgrade,
        _ => return Err(EngineRefusal::MalformedArgs("stoke_exact")),
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("stoke_exact"));
    }
    let Some(row) = crate::content_tables::card_row(CardId::Stoke, upgrade) else {
        return Err(EngineRefusal::MalformedArgs("stoke_exact row"));
    };
    let [step] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("stoke_exact program"));
    };
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs(
            "stoke_exact program/provenance",
        ))?;
    if ctx.spec.row != row
        || step.kind != StepKind::StokeExact
        || !ctx.catalog.args(step.args).is_empty()
        || !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs(
            "stoke_exact program/provenance",
        ));
    }

    let mut source_matches = 0_usize;
    let mut source = None;
    for pile in PileId::ALL {
        for card in ctx.state.piles.get(pile).as_slice() {
            if card.uid == ctx.source_uid {
                source_matches += 1;
                source = Some(*card);
            }
        }
    }
    if source_matches != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: source_matches,
        });
    }
    let source = source.expect("one exact active Stoke source was found");
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("stoke_exact active source"));
    }

    // Stoke reads its current owner's CharacterCardPool. The independent
    // Entropy-specific owner/unlock provenance below authenticates this
    // boundary; it is not the class of the original card. A foreign Stoke
    // samples its owner's complete pool at 0x3bef44 IL_0188-01de.
    let pool = ctx.catalog.owner_generation_pool(owner);
    for (index, id) in pool.iter().copied().enumerate() {
        if pool[..index].contains(&id) {
            return Err(EngineRefusal::MalformedArgs("stoke_exact pool"));
        }
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        let expected = crate::content_tables::card_row(id, upgrade)
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        let generated = ctx
            .catalog
            .atom(&identity)
            .and_then(|atom| ctx.catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        if generated.identity != identity || generated.row != expected {
            return Err(EngineRefusal::MalformedArgs("stoke_exact catalog closure"));
        }
    }

    let frozen = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| {
            if card.flags & CARD_FLAG_LEGACY != 0 {
                return Err(EngineRefusal::MalformedArgs(
                    "stoke_exact frozen physical identity",
                ));
            }
            ctx.catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            let matches = PileId::ALL
                .into_iter()
                .map(|pile| {
                    ctx.state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .filter(|candidate| candidate.uid == card.uid)
                        .count()
                })
                .sum::<usize>();
            if matches != 1 {
                return Err(EngineRefusal::ActiveCardNotUnique {
                    uid: card.uid,
                    matches,
                });
            }
            Ok(card.uid)
        })
        .collect::<Result<Vec<_>, EngineRefusal>>()?;

    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        source_uid: u32,
        step_index: u32,
        frozen: &[u32],
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        continue_stoke(
            state,
            catalog,
            StokeContinuation {
                source_uid,
                step_index,
                remaining: frozen,
                upgrade,
                original_count: frozen.len(),
            },
            events,
        )
    }

    let mut probe = ctx.state.clone();
    let step_index =
        if crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(ctx.state, ctx.catalog)
        {
            crate::engine::play::after_card_exhausted_source_step_cursor(ctx.state, ctx.source_uid)?
        } else {
            0
        };
    apply(
        &mut probe,
        ctx.catalog,
        ctx.source_uid,
        step_index,
        &frozen,
        upgrade,
        &mut Vec::new(),
    )?;
    apply(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
        step_index,
        &frozen,
        upgrade,
        ctx.events,
    )
}

#[derive(Clone, Copy)]
struct StokeContinuation<'a> {
    source_uid: u32,
    step_index: u32,
    remaining: &'a [u32],
    upgrade: u8,
    original_count: usize,
}

/// `Stoke/<OnPlay>d__1` RVA `0x3bef44`
/// awaits `CardCmd.Exhaust` per frozen card (IL_00f6); `<Exhaust>d__6` (`0x3e06c8`)
/// returns at `IsOverOrEnding` (IL_0025-002a) before its `CardPileCmd.Add`, so
/// an exhaust listener that ends the combat stops the walk before the next card
/// leaves Hand: the shared IsOverOrEnding projection, not `history.over`
/// (#3515).
fn continue_stoke(
    state: &mut HotState,
    catalog: &Catalog,
    continuation: StokeContinuation<'_>,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let StokeContinuation {
        source_uid,
        step_index,
        remaining,
        upgrade,
        original_count,
    } = continuation;
    let mut frozen_cards = Vec::with_capacity(remaining.len());
    for uid in remaining.iter().copied() {
        let card = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .copied()
            .find(|card| card.uid == uid)
            .ok_or(EngineRefusal::FrozenCardVanished {
                uid,
                pile: PileId::Hand,
            })?;
        frozen_cards.push(card);
    }
    let repairs = frozen_cards
        .iter()
        .copied()
        .map(|card| (card, PileId::Hand, PileId::Exhaust))
        .collect::<Vec<_>>();
    preflight_card_play_after_physical_moves(state, &repairs)?;
    for (cursor, uid) in remaining.iter().copied().enumerate() {
        if crate::engine::damage::damage_combat_is_ending(state) {
            break;
        }
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        let mut matches = hand.iter().enumerate().filter(|(_, card)| card.uid == uid);
        let Some((index, _)) = matches.next() else {
            return Err(EngineRefusal::FrozenCardVanished {
                uid,
                pile: PileId::Hand,
            });
        };
        if matches.next().is_some() {
            return Err(EngineRefusal::ActiveCardNotUnique { uid, matches: 2 });
        }
        let card = hand[index];
        let _ = hand;
        repair_card_play_after_physical_move(state, card, PileId::Hand, PileId::Exhaust)?;
        let card = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
            crate::hot::AfterCardExhaustedReturnKind::Stoke,
        );
        continuation.source_uid = Some(source_uid);
        continuation.step_index = step_index;
        continuation.count = u32::try_from(original_count)
            .map_err(|_| EngineRefusal::CounterOverflow("Stoke count"))?;
        continuation.flags = u32::from(upgrade);
        continuation.remaining = remaining[cursor + 1..].to_vec();
        let owner_live =
            crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog);
        let result = crate::engine::draw::card_exhausted_with_owner(
            state,
            catalog,
            card,
            continuation,
            events,
        )?;
        if owner_live || result == crate::engine::draw::CardExhaustedResult::Suspended {
            return Ok(());
        }
    }

    let owner = state
        .reward_card_pool
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let pool = catalog.owner_generation_pool(owner);
    if !crate::engine::cards::owner_pool_generation_provenance_is_exact(state, catalog)
        || pool.is_empty()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut generated = Vec::with_capacity(original_count);
    for _ in 0..original_count {
        generated.push(CardIdentity {
            id: sample_generation_slice(state, &pool)?,
            upgrade,
            enchantment: None,
        });
    }
    if state.history.over {
        return Ok(());
    }
    for identity in generated {
        inject_before_hand_draw_generated_bottom(state, catalog, identity, events)?;
    }
    Ok(())
}

pub(crate) fn resume_stoke_after_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let source_uid = record
        .source_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let upgrade =
        u8::try_from(record.flags & 0xff).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    if !record.remaining.is_empty() {
        state
            .frames
            .pop_top_after_card_exhausted_power()
            .filter(|completed| *completed == *record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        return continue_stoke(
            state,
            catalog,
            StokeContinuation {
                source_uid,
                step_index: record.step_index,
                remaining: &record.remaining,
                upgrade,
                original_count: usize::try_from(record.count)
                    .map_err(|_| EngineRefusal::ContinuationNotModeled)?,
            },
            events,
        );
    }
    let mut current = record.clone();
    // Native Stoke samples the complete generated-card identity list after
    // the frozen Exhaust loop even when the final Exhaust hook ended combat.
    // Only the subsequent AddToPile commands are ending-gated.
    if current.flags >> 8 == 0 {
        let owner = state
            .reward_card_pool
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let pool = catalog.owner_generation_pool(owner);
        if !crate::engine::cards::owner_pool_generation_provenance_is_exact(state, catalog)
            || pool.is_empty()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        current.generated.clear();
        for _ in 0..current.count {
            current
                .generated
                .push(sample_generation_slice(state, &pool)?);
        }
        current.flags |= 1 << 8;
        current.generation_cursor = 0;
        state
            .frames
            .replace_top_after_card_exhausted_power(&current)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    }
    while !state.history.over && (current.generation_cursor as usize) < current.generated.len() {
        let identity = CardIdentity {
            id: current.generated[current.generation_cursor as usize],
            upgrade,
            enchantment: None,
        };
        current.generation_cursor += 1;
        state
            .frames
            .replace_top_after_card_exhausted_power(&current)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        inject_before_hand_draw_generated_bottom(state, catalog, identity, events)?;
        if !matches!(
            state.frames.top(),
            Some(crate::frame::Frame::AfterCardExhaustedPower { .. })
        ) {
            return Ok(());
        }
    }
    state
        .frames
        .pop_top_after_card_exhausted_power()
        .filter(|completed| {
            completed.return_kind == crate::hot::AfterCardExhaustedReturnKind::Stoke
                && completed.card_uid == record.card_uid
                && (state.history.over
                    || completed.flags >> 8 == 1
                        && completed.generation_cursor as usize == completed.generated.len())
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    Ok(())
}

/// `tank_exact` — not modeled. **Escalated.**
///
/// Python: `_apply_tank_to_owner_and_guard_teammates` applies Tank to the
/// owner and distinct Guarded state to each living teammate, retaining the
/// applier identity for the corresponding death lifecycle.
///
/// Rust has one player, no ally roster, and no Tank/Guarded identities or
/// applier-aware death lifecycle.
/// ESCALATED-ON: power-variant-absent(PowerId::Tank)
/// ESCALATED-ON: power-variant-absent(PowerId::Guarded)
pub(crate) fn tank_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::TankExact))
}

fn exact_active_thrash(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<HotCard, EngineRefusal> {
    let mut matches = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(move |card| card.uid == source_uid)
    });
    let Some(source) = matches.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let additional = matches.count();
    if additional != 0 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 1 + additional,
        });
    }
    if source.flags & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0
        || catalog.spec(source.atom).map(|row| row.identity) != Some(spec.identity)
    {
        return Err(EngineRefusal::MalformedArgs("Thrash physical source"));
    }
    Ok(source)
}

/// v0.111.0 DLL 9cb4f1ad: Thrash/<OnPlay>d__9 (0x3c3510) selects
/// CalculatedDamage, Damage, then OstyDamage. The additional DamageVar
/// readers are HowlFromBeyond (canonical 0xe2bed, upgrade 0xe2cb7),
/// Conflagration (0xdb9f7 / 0xdba6f), Thunderclap (0xeeda9 / 0xeee47),
/// Pillage (0xe7a01 / 0xe7a67), Spite (0xec4de / 0xec557), the
/// `THRASH_DAMAGE_VAR_BODIES` rows below, and Bully's
/// null-target Calculate (0xda20c / multiplier 0x38fe66). Repeat count and
/// whether the card is currently playable do not modify these base values.
/// Attack bodies whose Thrash reading is their plain DamageVar (#2950):
/// (card, compiled damage step, canonical base, OnUpgrade UpgradeValueBy).
/// v0.111.0 DLL 9cb4f1ad, get_CanonicalVars / OnUpgrade RVAs, each read at
/// `DamageVar::.ctor` and `DynamicVarSet::get_Damage` → `UpgradeValueBy`:
/// Anger 0xd7b41 (6) / 0xd7ba7 (+2); Breakthrough 0xd9dcf (9) / 0xd9e4f
/// (+4); Dismantle 0xddd2b (8) / 0xdddb3 (+2); Fiend Fire 0xdfb24 (7) /
/// 0xdfb9b (+3); Pact's End 0xe7053 (18) / 0xe7108 (+6); Stomp 0xece58 (12)
/// / 0xecebf (+3); Sword Boomerang 0xedcc4 (3) / 0xedd3f (upgrades Repeat
/// only, +0 damage); Tear Asunder 0xee238 (5) / 0xee307 (+2), whose
/// CalculatedVar is `CalculatedHits`, not the CalculatedDamage Thrash reads
/// first; Whirlwind 0xf01ff (5) / 0xf0267 (+3).
const THRASH_DAMAGE_VAR_BODIES: [(CardId, StepKind, i64, i64); 9] = [
    (CardId::Anger, StepKind::AngerExact, 6, 2),
    (CardId::Breakthrough, StepKind::AttackAll, 9, 4),
    (CardId::Dismantle, StepKind::CalculatedHits, 8, 2),
    (CardId::FiendFire, StepKind::FiendFireExact, 7, 3),
    (CardId::PactsEnd, StepKind::PactsEndExact, 18, 6),
    (CardId::Stomp, StepKind::StompExact, 12, 3),
    (CardId::SwordBoomerang, StepKind::AttackRandom, 3, 0),
    (CardId::TearAsunder, StepKind::TearAsunderExact, 5, 2),
    (CardId::Whirlwind, StepKind::AttackAllX, 5, 3),
];

/// The DamageVar base of a `THRASH_DAMAGE_VAR_BODIES` card, only when its
/// compiled row carries exactly one damage step holding that same base.
fn thrash_damage_var_body(
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    steps: &[crate::catalog::CompiledStep],
) -> Option<i64> {
    let &(_, kind, base, per_upgrade) = THRASH_DAMAGE_VAR_BODIES
        .iter()
        .find(|row| row.0 == spec.identity.id)?;
    let expected = base + per_upgrade * i64::from(spec.identity.upgrade);
    let mut damage = steps.iter().filter(|step| step.kind == kind);
    let step = damage.next()?;
    (damage.next().is_none()
        && matches!(catalog.args(step.args).first(), Some(CompiledArg::I(value)) if *value == expected))
        .then_some(expected)
}

fn thrash_candidate_base(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
) -> Result<DotNetDecimal, EngineRefusal> {
    let spec = *catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    if !spec.is_attack || !crate::engine::play::body_enchantment_is_exact(&spec) {
        return Err(EngineRefusal::MalformedArgs("Thrash damage candidate"));
    }
    let steps = catalog.steps(&spec);
    // Native v0.111.0 Thrash/<OnPlay>d__9 (RVA 0x3c3510) asks the
    // selected card's DynamicVars, independently of its attack implementation.
    // Bully's null-target multiplier is zero (0x38fe66), so Calculate(null)
    // is CalculationBase=4 (0xda20c). Pillage Damage is 6 + upgrade*3
    // (0xe7a01 / 0xe7a67). Authenticate the complete compiled registry row.
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) == Some(spec.row)
        && steps.len() == spec.row.steps.len()
        && matches!(spec.identity.upgrade, 0 | 1)
    {
        let base = match (spec.identity.id, steps) {
            (CardId::Bully, [step])
                if step.kind == StepKind::AttackPerVuln
                    && matches!(catalog.args(step.args), [CompiledArg::I(4), CompiledArg::I(extra)]
                    if *extra == 2 + i64::from(spec.identity.upgrade)) =>
            {
                Some(4)
            }
            (CardId::Pillage, [step])
                if step.kind == StepKind::PillageExact
                    && matches!(catalog.args(step.args), [CompiledArg::I(base)]
                    if *base == 6 + 3 * i64::from(spec.identity.upgrade)) =>
            {
                Some(6 + 3 * i64::from(spec.identity.upgrade))
            }
            (CardId::Spite, [step])
                if step.kind == StepKind::SameTurnHistoryBody
                    && matches!(catalog.args(step.args), [CompiledArg::Word(StepWord::Spite), CompiledArg::I(5), CompiledArg::I(repeat)]
                    if *repeat == 2 + i64::from(spec.identity.upgrade)) =>
            {
                Some(5)
            }
            _ => thrash_damage_var_body(catalog, &spec, steps),
        };
        if let Some(base) = base {
            if state.card_states.get(card.uid).exact_damage_growth() != Some(DotNetDecimal::zero())
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Thrash ordinary candidate growth",
                ));
            }
            return Ok(DotNetDecimal::from_i64(base));
        }
    }

    // Native DynamicVar precedence is CalculatedDamage, Damage, OstyDamage.
    let calculated = steps
        .iter()
        .filter(|step| step.kind == StepKind::CalculatedAttack)
        .collect::<Vec<_>>();
    if !calculated.is_empty() {
        let [step] = calculated.as_slice() else {
            return Err(EngineRefusal::MalformedArgs(
                "Thrash calculated damage candidate",
            ));
        };
        let mut ignored = Vec::new();
        let probe = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: card.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut ignored,
        };
        return crate::steps::shared::calculated_operand(&probe, StepKind::CalculatedAttack)
            .map(DotNetDecimal::from_i64);
    }
    if matches!(spec.identity.id, CardId::PerfectedStrike) {
        let (base, per) = match (spec.identity.upgrade, steps) {
            (0, [step])
                if step.kind == StepKind::AttackPerStrike
                    && matches!(
                        catalog.args(step.args),
                        [CompiledArg::I(6), CompiledArg::I(2)]
                    ) =>
            {
                (6_i64, 2_i64)
            }
            (1, [step])
                if step.kind == StepKind::AttackPerStrike
                    && matches!(
                        catalog.args(step.args),
                        [CompiledArg::I(6), CompiledArg::I(3)]
                    ) =>
            {
                (6_i64, 3_i64)
            }
            _ => return Err(EngineRefusal::MalformedArgs("Thrash Perfected Strike")),
        };
        return per
            .checked_mul(i64::from(state.ps_strikes))
            .and_then(|scaled| base.checked_add(scaled))
            .map(DotNetDecimal::from_i64)
            .ok_or(EngineRefusal::CounterOverflow("Thrash Perfected Strike"));
    }

    let physical_base = match (spec.identity.id, spec.identity.upgrade, steps) {
        (CardId::Claw, 0, [step])
            if step.kind == StepKind::ClawExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(3), CompiledArg::I(2)]
                ) =>
        {
            Some(3)
        }
        (CardId::Claw, 1, [step])
            if step.kind == StepKind::ClawExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(4), CompiledArg::I(3)]
                ) =>
        {
            Some(4)
        }
        (CardId::KinglyPunch, 0, [step])
            if step.kind == StepKind::PhysicalDamageAttack
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(8), CompiledArg::I(1)]
                ) =>
        {
            Some(8)
        }
        (CardId::KinglyPunch, 1, [step])
            if step.kind == StepKind::PhysicalDamageAttack
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(10), CompiledArg::I(1)]
                ) =>
        {
            Some(10)
        }
        (CardId::Maul, 0, [step])
            if step.kind == StepKind::MaulExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(5), CompiledArg::I(2)]
                ) =>
        {
            Some(5)
        }
        (CardId::Maul, 1, [step])
            if step.kind == StepKind::MaulExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(6), CompiledArg::I(3)]
                ) =>
        {
            Some(6)
        }
        (CardId::Rampage, 0, [step])
            if step.kind == StepKind::RampageExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(10), CompiledArg::I(5)]
                ) =>
        {
            Some(10)
        }
        (CardId::Rampage, 1, [step])
            if step.kind == StepKind::RampageExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(10), CompiledArg::I(10)]
                ) =>
        {
            Some(10)
        }
        (CardId::TheScythe, 0, [step])
            if step.kind == StepKind::TheScytheExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(13), CompiledArg::I(5)]
                ) =>
        {
            Some(13)
        }
        (CardId::TheScythe, 1, [step])
            if step.kind == StepKind::TheScytheExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(13), CompiledArg::I(7)]
                ) =>
        {
            Some(13)
        }
        (CardId::Thrash, 0, [step])
            if step.kind == StepKind::ThrashExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(4), CompiledArg::I(2)]
                ) =>
        {
            Some(4)
        }
        (CardId::Thrash, 1, [step])
            if step.kind == StepKind::ThrashExact
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::I(6), CompiledArg::I(2)]
                ) =>
        {
            Some(6)
        }
        _ => None,
    };
    if let Some(base) = physical_base {
        if card.flags & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0 {
            return Err(EngineRefusal::MalformedArgs("Thrash physical candidate"));
        }
        let growth = state
            .card_states
            .get(card.uid)
            .exact_damage_growth()
            .ok_or(EngineRefusal::MalformedArgs("Thrash candidate growth"))?;
        return growth
            .checked_add(DotNetDecimal::from_i64(base))
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash candidate damage"));
    }

    let damage = steps
        .iter()
        .filter(|step| {
            step.kind == StepKind::Attack
                || matches!(
                    spec.identity.id,
                    CardId::HowlFromBeyond | CardId::Conflagration
                ) && step.kind == StepKind::AttackAll
                || matches!(spec.identity.id, CardId::Thunderclap)
                    && step.kind == StepKind::ThunderclapExact
        })
        .collect::<Vec<_>>();
    if let [step] = damage.as_slice()
        && let [CompiledArg::I(base), CompiledArg::I(_)] = catalog.args(step.args)
    {
        if state.card_states.get(card.uid).exact_damage_growth() != Some(DotNetDecimal::zero()) {
            return Err(EngineRefusal::MalformedArgs(
                "Thrash ordinary candidate growth",
            ));
        }
        return Ok(DotNetDecimal::from_i64(*base));
    }

    let osty = match (spec.identity.id, spec.identity.upgrade, steps) {
        (CardId::BoneShards, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::AoeBlockKill),
                        CompiledArg::I(9),
                        CompiledArg::I(9)
                    ]
                ) =>
        {
            Some(9)
        }
        (CardId::BoneShards, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::AoeBlockKill),
                        CompiledArg::I(12),
                        CompiledArg::I(12)
                    ]
                ) =>
        {
            Some(12)
        }
        (CardId::Fetch, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::Fetch),
                        CompiledArg::I(3),
                        CompiledArg::I(1)
                    ]
                ) =>
        {
            Some(3)
        }
        (CardId::Fetch, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::Fetch),
                        CompiledArg::I(6),
                        CompiledArg::I(1)
                    ]
                ) =>
        {
            Some(6)
        }
        (CardId::Flatten, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(12)]
                ) =>
        {
            Some(12)
        }
        (CardId::Flatten, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(16)]
                ) =>
        {
            Some(16)
        }
        (CardId::HighFive, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::AoeVulnerable),
                        CompiledArg::I(11),
                        CompiledArg::I(2)
                    ]
                ) =>
        {
            Some(11)
        }
        (CardId::HighFive, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [
                        CompiledArg::Word(StepWord::AoeVulnerable),
                        CompiledArg::I(13),
                        CompiledArg::I(3)
                    ]
                ) =>
        {
            Some(13)
        }
        (CardId::Poke, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(6)]
                ) =>
        {
            Some(6)
        }
        (CardId::Poke, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(9)]
                ) =>
        {
            Some(9)
        }
        (CardId::RightHandHand, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(4)]
                ) =>
        {
            Some(4)
        }
        (CardId::RightHandHand, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(6)]
                ) =>
        {
            Some(6)
        }
        (CardId::Snap, 0, [step, select])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(7)]
                )
                && select.kind == StepKind::Select
                && matches!(
                    catalog.args(select.args),
                    [
                        CompiledArg::Pile(PileId::Hand),
                        CompiledArg::I(1),
                        CompiledArg::I(1),
                        CompiledArg::Filter(crate::ids::FilterMode::WithoutEffectiveRetain),
                        CompiledArg::Word(StepWord::ApplyPermanentRetain)
                    ]
                ) =>
        {
            Some(7)
        }
        (CardId::Snap, 1, [step, select])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Single), CompiledArg::I(10)]
                )
                && select.kind == StepKind::Select
                && matches!(
                    catalog.args(select.args),
                    [
                        CompiledArg::Pile(PileId::Hand),
                        CompiledArg::I(1),
                        CompiledArg::I(1),
                        CompiledArg::Filter(crate::ids::FilterMode::WithoutEffectiveRetain),
                        CompiledArg::Word(StepWord::ApplyPermanentRetain)
                    ]
                ) =>
        {
            Some(10)
        }
        (CardId::SweepingGaze, 0, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Random), CompiledArg::I(10)]
                ) =>
        {
            Some(10)
        }
        (CardId::SweepingGaze, 1, [step])
            if step.kind == StepKind::OstyBody
                && matches!(
                    catalog.args(step.args),
                    [CompiledArg::Word(StepWord::Random), CompiledArg::I(15)]
                ) =>
        {
            Some(15)
        }
        _ => None,
    };
    osty.map(DotNetDecimal::from_i64)
        .ok_or(EngineRefusal::MalformedArgs("Thrash damage candidate"))
}

/// Native v111 Thrash.OnPlay0x3c3510 IL01e1–0212 passes the exhausted
/// source, null target and null CardPlay to Hook.ModifyDamage (props8).
/// OneForAll0xa52d8 IL003e–00a8 therefore reads that source's CostsX and
/// GetWithModifiers(-1), never the active Thrash's paid energy. DoubleDamage
/// 0xa1ca8 contributes one x2 for this powered owner/source regardless of
/// duration. Keep the exact Decimal through the existing Weak/Shrink fold.
fn thrash_null_target_damage(
    state: &HotState,
    catalog: &Catalog,
    selected: HotCard,
    raw: DotNetDecimal,
) -> Result<DotNetDecimal, EngineRefusal> {
    let spec = catalog
        .spec(selected.atom)
        .ok_or(EngineRefusal::UnknownAtom(selected.atom))?;
    let shiv = if spec.row.tags.contains(&"Shiv") {
        state
            .powers
            .value(PowerId::Accuracy)
            .checked_add(if state.history.shiv_plays_finished_this_turn == 0 {
                state.powers.value(PowerId::PhantomBlades)
            } else {
                0
            })
            .ok_or(EngineRefusal::CounterOverflow("Thrash Shiv additive"))?
    } else {
        0
    };
    let one_for_all =
        if !spec.x_cost && crate::engine::play::resolved_energy_cost(state, selected, spec) == 0 {
            state.powers.value(PowerId::OneForAll)
        } else {
            0
        };
    let additive = state
        .powers
        .value(PowerId::Strength)
        .checked_add(state.temp_strength)
        .and_then(|value| value.checked_add(shiv))
        .and_then(|value| value.checked_add(one_for_all))
        .ok_or(EngineRefusal::CounterOverflow("Thrash damage additive"))?;
    let mut value = raw
        .checked_add(DotNetDecimal::from_i64(i64::from(additive)))
        .map_err(|_| EngineRefusal::CounterOverflow("Thrash damage preview"))?;
    if state.powers.value(PowerId::DoubleDamage) > 0 {
        value = value
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash Double Damage"))?;
    }
    if state.powers.value(PowerId::PlayerWeak) > 0 {
        value = value
            .checked_mul(
                DotNetDecimal::ratio(3, 4)
                    .map_err(|_| EngineRefusal::CounterOverflow("Thrash Weak"))?,
            )
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash damage preview"))?;
    }
    if state.powers.value(PowerId::PlayerShrink) > 0 {
        value = value
            .checked_mul(
                DotNetDecimal::ratio(7, 10)
                    .map_err(|_| EngineRefusal::CounterOverflow("Thrash Shrink"))?,
            )
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash damage preview"))?;
    }
    Ok(value.max(DotNetDecimal::zero()))
}

/// `thrash_exact` — exact retained-Decimal attack, Hand-Attack selection, and
/// exhaust.
///
/// v0.111.0 DLL 9cb4f1ad, `Thrash/<OnPlay>d__9::MoveNext` (RVA 0x3c3510):
/// after the two-hit attack, IL_00e3–00ea is `PileTypeExtensions.GetPile(2,
/// Owner)` where `PileType` 2 is `Hand` (None=0, Draw=1, Hand=2, Discard=3,
/// Exhaust=4); IL_0105–012f filters it with `<>c::<OnPlay>b__9_0` (RVA
/// 0x3c3502, `CardModel.Type == 1`, Attack) and calls
/// `CombatCardSelection.NextItem`. `Rng::NextItem` (RVA 0x5ee74, IL_001e–0030)
/// returns default WITHOUT calling `NextInt` for an empty sequence, so a Hand
/// with no Attack consumes no Selection draw, and IL_0136 then skips the tail.
/// Otherwise the chosen card's damage var is previewed through
/// `Hook.ModifyDamage` and added to Thrash's Damage/ExtraDamage
/// (IL_013b–0244), and IL_0249–0252 runs `CardCmd.Exhaust(ctx, card, false,
/// false)` on the Hand card — the ordinary Hand -> Exhaust transaction with
/// its hooks. `OnPlay` has no ending gate of its own; `CardCmd.Exhaust`'s
/// state machine (RVA 0x3e06c8, IL_0020–002a) returns early on
/// `IsOverOrEnding`, so selection and growth survive a fight-ending kill but
/// the exhaust does not run (#2519).
///
/// The candidate resolver authenticates the complete supported DynamicVar
/// program before Selection RNG. The null-target preview deliberately omits
/// target-owned modifiers, retains the exact .NET Decimal result in the
/// physical Thrash instance, and only then runs the separately ending-gated
/// Exhaust command.
pub(crate) fn thrash_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (printed, hits) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Thrash, 0, [CompiledArg::I(4), CompiledArg::I(2)]) => (4, 2),
        (CardId::Thrash, 1, [CompiledArg::I(6), CompiledArg::I(2)]) => (6, 2),
        _ => return Err(EngineRefusal::MalformedArgs("thrash_exact")),
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("thrash_exact owner"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let active = exact_active_thrash(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let growth = ctx
        .state
        .card_states
        .get(active.uid)
        .exact_damage_growth()
        .ok_or(EngineRefusal::MalformedArgs("Thrash damage growth"))?;
    let base = growth
        .checked_add(DotNetDecimal::from_i64(printed))
        .map_err(|_| EngineRefusal::CounterOverflow("Thrash damage"))?;
    player_attack_decimal_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        base,
        hits,
        ctx.events,
    )?;
    // Python `_run_steps_inner` (frozen, deleted #2827) stops this body when reflected attack
    // damage killed the owner. An enemy-side fight-ending kill still leaves
    // the native selection/growth tail observable.
    if ctx.state.hp <= 0 {
        return Ok(());
    }

    // Native snapshots the live Hand Attacks after the two-hit command
    // (GetPile(2) = Hand, IL_00e3–00ea), proves their DynamicVar shape, then
    // consumes one Selection draw only when the snapshot is nonempty.
    let candidates = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| {
            ctx.catalog
                .spec(card.atom)
                .is_some_and(|spec| spec.is_attack)
        })
        .collect::<Vec<_>>();
    for candidate in &candidates {
        thrash_candidate_base(ctx.state, ctx.catalog, *candidate)?;
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let live = ctx.state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound: i32 = candidates
        .len()
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("Thrash selection"))?;
    let selected = candidates[usize::try_from(
        rng.next_bounded(bound)
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash selection"))?,
    )
    .map_err(|_| EngineRefusal::CounterOverflow("Thrash selection"))?];
    ctx.state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let raw = thrash_candidate_base(ctx.state, ctx.catalog, selected)?;
    let increase = thrash_null_target_damage(ctx.state, ctx.catalog, selected, raw)?;
    let active = exact_active_thrash(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let mut instance = ctx.state.card_states.get(active.uid);
    if increase != DotNetDecimal::zero() {
        let next = instance
            .exact_damage_growth()
            .ok_or(EngineRefusal::MalformedArgs("Thrash damage growth"))?
            .checked_add(increase)
            .map_err(|_| EngineRefusal::CounterOverflow("Thrash damage growth"))?;
        instance
            .set_fraction_damage_growth(next)
            .ok_or(EngineRefusal::CounterOverflow("Thrash damage growth"))?;
        ctx.state.card_states.set(active.uid, instance);
    }

    // Selection and growth survive a fight-ending enemy kill. The separately
    // awaited Exhaust command has its own ending gate, and the card stays in
    // Hand (#3041).
    if crate::engine::draw::card_cmd_exhaust_is_gated(ctx.state) {
        return Ok(());
    }
    // CardCmd.Exhaust (IL_0252) moves the chosen card out of the Hand.
    let hand = ctx.state.piles.get(PileId::Hand).as_slice();
    let mut matches = hand
        .iter()
        .enumerate()
        .filter(|(_, card)| card.uid == selected.uid);
    let Some((index, _)) = matches.next() else {
        return Err(EngineRefusal::FrozenCardVanished {
            uid: selected.uid,
            pile: PileId::Hand,
        });
    };
    if matches.next().is_some() {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: selected.uid,
            matches: 2,
        });
    }
    let Some(selected) =
        crate::engine::draw::detach_card_for_exhaust(ctx.state, PileId::Hand, index)?
    else {
        return Ok(());
    };
    let _ = crate::engine::draw::card_exhausted_cardplay_return_only(
        ctx.state,
        ctx.catalog,
        selected,
        ctx.source_uid,
        ctx.events,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::catalog::{CardIdentity, CatalogBuilder, RewardPool};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::{Action, Event, SelectionRef, apply_action, capability_manifest};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_RINGING, CARD_FLAG_SOVEREIGN_BLADE_STATE,
        CardInstanceState, HopperDeckRow, HopperDeckState, HotCard, HotMonster, HotState,
        RngStreamState,
    };
    use crate::ids::{CardId, MonsterKind, PowerId};
    use crate::powers::SlotWire;
    use crate::rng::Xoshiro256StarStar;

    const OWNED: [StepKind; 9] = [
        StepKind::AutoplayDrawX,
        StepKind::CrimsonMantle,
        StepKind::FiendFireExact,
        StepKind::HeavenlyDrillX,
        StepKind::PactsEndExact,
        StepKind::PrimalForceExact,
        StepKind::StokeExact,
        StepKind::TankExact,
        StepKind::ThrashExact,
    ];
    const ESCALATED: [StepKind; 1] = [StepKind::TankExact];

    fn fixture() -> (HotState, crate::catalog::Catalog, crate::catalog::CardAtom) {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        (state, catalog, atom)
    }

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn fiend_fixture(
        upgrade: u8,
    ) -> (HotState, crate::catalog::Catalog, HotCard, HotCard, HotCard) {
        let mut builder = CatalogBuilder::new();
        let fiend_atom = builder
            .intern(identity(CardId::FiendFire, upgrade))
            .unwrap();
        let strike_atom = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        let defend_atom = builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: fiend_atom,
            flags: 0,
        };
        let strike = HotCard {
            uid: 1,
            atom: strike_atom,
            flags: 0,
        };
        let defend = HotCard {
            uid: 2,
            atom: defend_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source, strike, defend)
    }

    fn primal_fixture(
        upgrade: u8,
    ) -> (
        HotState,
        crate::catalog::Catalog,
        HotCard,
        HotCard,
        HotCard,
        HotCard,
    ) {
        let mut builder = CatalogBuilder::new();
        let primal_atom = builder
            .intern(identity(CardId::PrimalForce, upgrade))
            .unwrap();
        let strike_atom = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        let defend_atom = builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        let sovereign_atom = builder.intern(identity(CardId::SovereignBlade, 0)).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: primal_atom,
            flags: 0,
        };
        let strike = HotCard {
            uid: 11,
            atom: strike_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let defend = HotCard {
            uid: 12,
            atom: defend_atom,
            flags: 0,
        };
        let sovereign = HotCard {
            uid: 13,
            atom: sovereign_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state.ps_strikes = 1;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        (state, catalog, source, strike, defend, sovereign)
    }

    fn crimson_fixture(upgrade: u8, pile: PileId) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(identity(CardId::CrimsonMantle, upgrade))
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(pile).make_mut().push(source);
        (state, catalog, source)
    }

    fn stoke_fixture(upgrade: u8, frozen_count: usize) -> (HotState, Catalog, HotCard) {
        stoke_fixture_for_owner(RewardPool::Ironclad, upgrade, frozen_count)
    }

    fn stoke_fixture_for_owner(
        owner: RewardPool,
        upgrade: u8,
        frozen_count: usize,
    ) -> (HotState, Catalog, HotCard) {
        stoke_fixture_for_owner_with_resumable_catalog(owner, upgrade, frozen_count, false)
    }

    fn stoke_fixture_for_owner_with_resumable_catalog(
        owner: RewardPool,
        upgrade: u8,
        frozen_count: usize,
        resumable_catalog: bool,
    ) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let pool = builder.owner_generation_pool(owner);
        for id in pool.iter().copied() {
            for generated_upgrade in [0, 1] {
                builder.intern(identity(id, generated_upgrade)).unwrap();
            }
        }
        if resumable_catalog {
            for generated in
                crate::boundary::stoke_generation_closure_for_owner([true, true], owner)
            {
                builder.intern_reachable(generated).unwrap();
            }
            builder.mark_live_dark_embrace_reachable();
            builder.mark_live_stratagem_reachable();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
        }
        let source_atom = builder.intern(identity(CardId::Stoke, upgrade)).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: source_atom,
            flags: 0,
        };
        let hand_atoms = [
            catalog.atom(&identity(pool[0], 0)).unwrap(),
            catalog.atom(&identity(pool[1], 0)).unwrap(),
        ];
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.reward_card_pool = Some(owner);
        state.entropy_card_pool = Some(owner);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.next_card_uid = 100;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..frozen_count).map(|index| HotCard {
                uid: u32::try_from(index + 1).unwrap(),
                atom: hand_atoms[index % hand_atoms.len()],
                flags: 0,
            }));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        (state, catalog, source)
    }

    fn thrash_fixture(upgrade: u8, candidate: CardId) -> (HotState, Catalog, HotCard, HotCard) {
        thrash_fixture_with(upgrade, candidate, 0)
    }

    fn thrash_fixture_with(
        upgrade: u8,
        candidate: CardId,
        candidate_upgrade: u8,
    ) -> (HotState, Catalog, HotCard, HotCard) {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::Thrash, upgrade)).unwrap();
        let candidate_atom = builder
            .intern(identity(candidate, candidate_upgrade))
            .unwrap();
        builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        builder.intern(identity(CardId::OneForAll, 0)).unwrap();
        builder.intern(identity(CardId::ShadowStep, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: source_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let candidate = HotCard {
            uid: 7,
            atom: candidate_atom,
            flags: matches!(
                candidate,
                CardId::Claw
                    | CardId::KinglyPunch
                    | CardId::Maul
                    | CardId::Rampage
                    | CardId::TheScythe
                    | CardId::Thrash
            )
            .then_some(CARD_FLAG_DEFAULT_PHYSICAL_STATE)
            .unwrap_or(0),
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: [13, 14, 15, 16],
                counter: 0,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1_000));
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        // Thrash selects from the Hand (GetPile(2), 0x3c3510 IL_00e3, #2519).
        state.piles.get_mut(PileId::Hand).make_mut().push(candidate);
        (state, catalog, source, candidate)
    }

    fn exact_growth(state: &HotState, uid: u32) -> DotNetDecimal {
        state.card_states.get(uid).exact_damage_growth().unwrap()
    }

    fn run_stoke_foundation(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events,
        };
        stoke_foundation(&mut ctx)
    }

    #[test]
    fn stoke_terminal_exhaust_still_samples_complete_generation_list() {
        fn terminal_resume(remaining: Vec<u32>) -> (RngStreamState, i32, usize) {
            let (mut state, catalog, source) = stoke_fixture(0, 2);
            let first = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
            state.piles.get_mut(PileId::Exhaust).make_mut().push(first);
            let card_uid = if remaining.is_empty() {
                let last = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
                let uid = last.uid;
                state.piles.get_mut(PileId::Exhaust).make_mut().push(last);
                uid
            } else {
                first.uid
            };
            state.history.over = true;
            let mut record = crate::hot::AfterCardExhaustedPowerRecord::for_return(
                crate::hot::AfterCardExhaustedReturnKind::Stoke,
            );
            record.listeners = vec![PowerId::DarkEmbrace];
            record.cursor = 1;
            record.card_uid = card_uid;
            record.ordinary_suffix_invoked = true;
            record.source_uid = Some(source.uid);
            record.step_index = 1;
            record.count = 2;
            record.remaining = remaining;
            state
                .frames
                .push_after_card_exhausted_power(&record)
                .unwrap();

            resume_stoke_after_exhaust(&mut state, &catalog, &record, &mut Vec::new()).unwrap();
            assert!(state.frames.is_empty());
            (
                state.rng.get(RngStream::Generation),
                state.history.owner_generated_cards_combat,
                state.piles.get(PileId::Draw).len(),
            )
        }

        let early = terminal_resume(vec![2]);
        let last = terminal_resume(Vec::new());
        assert_eq!(
            early, last,
            "early- and last-item lethal exits sample identically"
        );
        assert_eq!(early.0.counter, 2, "Stoke samples its printed N identities");
        assert_eq!(
            early.1, 0,
            "ending-gated AddToPile never publishes generation"
        );
        assert_eq!(early.2, 0, "ending-gated generated cards never enter Draw");
    }

    #[test]
    fn stoke_foreign_owner_after_exhaust_resume_samples_the_current_owner_pool() {
        let expected_ids = |owner| match owner {
            RewardPool::Defect => [CardId::Fusion, CardId::Overclock],
            RewardPool::Ironclad => [CardId::FlameBarrier, CardId::Pillage],
            RewardPool::Necrobinder => [CardId::Friendship, CardId::Putrefy],
            RewardPool::Regent => [CardId::Glimmer, CardId::PaleBlueDot],
            RewardPool::Silent => [CardId::Expose, CardId::NoxiousFumes],
        };
        for owner in RewardPool::ALL
            .into_iter()
            .filter(|owner| *owner != RewardPool::Ironclad)
        {
            let (mut state, catalog, source) = stoke_fixture_for_owner(owner, 1, 2);
            let seeded = Xoshiro256StarStar::from_seed(47);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            let first = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
            state.piles.get_mut(PileId::Exhaust).make_mut().push(first);

            let mut record = crate::hot::AfterCardExhaustedPowerRecord::for_return(
                crate::hot::AfterCardExhaustedReturnKind::Stoke,
            );
            record.card_uid = first.uid;
            record.listeners = vec![PowerId::DarkEmbrace];
            record.cursor = 1;
            record.ordinary_suffix_invoked = true;
            record.source_uid = Some(source.uid);
            record.step_index = 1;
            record.count = 2;
            record.flags = 1;
            record.remaining = vec![2];
            state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
            );
            state
                .frames
                .push_after_card_exhausted_power(&record)
                .unwrap();

            // This is the resumable AfterCardExhausted owner state that is
            // persisted at an awaited child. It must use the current owner,
            // rather than the Ironclad source table, when it resumes.
            resume_stoke_after_exhaust(&mut state, &catalog, &record, &mut Vec::new()).unwrap();

            assert!(state.frames.is_empty(), "{owner:?}");
            assert_eq!(
                state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| catalog.spec(card.atom).unwrap().identity)
                    .collect::<Vec<_>>(),
                expected_ids(owner)
                    .into_iter()
                    .map(|id| identity(id, 1))
                    .collect::<Vec<_>>(),
                "{owner:?}",
            );
            assert_eq!(state.rng.get(RngStream::Generation).counter, 2, "{owner:?}");
        }
    }

    #[test]
    fn stoke_foreign_owner_after_exhaust_cold_round_trip_authenticates_and_resumes() {
        let owner = RewardPool::Defect;
        let (mut state, catalog, _) =
            stoke_fixture_for_owner_with_resumable_catalog(owner, 1, 2, true);
        let mut source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        source.flags = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .insert(0, source);
        for card in state.piles.get_mut(PileId::Hand).make_mut() {
            card.flags = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        }
        state.energy = 3;
        state.turn = 1;
        state.player_side_active = true;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.inky_attack_damage = 0;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        let pool = catalog.owner_generation_pool(owner);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((20..22).map(|uid| {
                HotCard {
                    uid,
                    atom: catalog
                        .atom(&identity(pool[(uid as usize - 20) % pool.len()], 0))
                        .unwrap(),
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }
            }));
        assert!(HotBoundary::try_to_canonical(&state, &catalog).is_ok());

        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some());
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::AfterCardExhaustedPower { .. },
                crate::frame::Frame::Draw { .. },
            ]
        ));

        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );
        let actions = crate::engine::legal_actions(&rebuilt, &rebuilt_catalog);
        assert!(
            !actions.is_empty(),
            "the resumed Stratagem choice is public"
        );
        let completed = apply_action(&rebuilt, &rebuilt_catalog, &actions[0])
            .unwrap()
            .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.rng.get(RngStream::Generation).counter, 2);
        assert_eq!(completed.history.owner_generated_cards_combat, 2);
    }

    #[test]
    fn thrash_attacks_selects_with_singleton_rng_grows_and_reexhausts() {
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        let before_selection = state.rng.get(RngStream::Sel);
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 992);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::from_i64(6));
        assert_eq!(
            state.rng.get(RngStream::Sel).counter,
            before_selection.counter + 1
        );
        assert_eq!(
            state.piles.get(PileId::Exhaust).as_slice(),
            &[selected],
            "the selected Hand Attack moves Hand -> Exhaust"
        );
        assert!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| card.uid != selected.uid)
        );
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);

        let projected = HotBoundary::to_canonical(&state, &catalog);
        let projected_thrash = projected
            .piles
            .values()
            .flatten()
            .find(|card| card.uid == Some(u64::from(source.uid)))
            .unwrap();
        assert_eq!(
            projected_thrash.physical_state,
            Some(serde_json::json!([
                "PHYSICAL_CARD_STATE",
                [],
                {"__fraction__": [6, 1]},
                false,
                []
            ]))
        );
    }

    #[test]
    fn thrash_empty_candidates_consume_no_selection_rng() {
        // Rng::NextItem (0x5ee74 IL_001e–0030) returns before NextInt when
        // the Hand holds no Attack.
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .retain(|card| card.uid != selected.uid);
        let before_selection = state.rng.get(RngStream::Sel);
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 992);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::zero());
        assert_eq!(state.rng.get(RngStream::Sel), before_selection);
    }

    #[test]
    fn thrash_hand_without_attacks_consumes_no_selection_rng() {
        // A non-Attack Hand card fails <OnPlay>b__9_0 (0x3c3502, Type == 1).
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        let defend = HotCard {
            uid: 8,
            atom: catalog.atom(&identity(CardId::DefendIronclad, 0)).unwrap(),
            flags: 0,
        };
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        hand.retain(|card| card.uid != selected.uid);
        hand.push(defend);
        let before_selection = state.rng.get(RngStream::Sel);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.rng.get(RngStream::Sel), before_selection);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::zero());
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[defend]);
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    #[test]
    fn thrash_exhausts_a_hand_attack_and_ignores_exhaust_pile_attacks() {
        // #2519: GetPile(2) is the Hand; an Attack already in Exhaust is not
        // a candidate, and the chosen Hand Attack is the one exhausted.
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        let old = HotCard {
            uid: 11,
            atom: selected.atom,
            flags: selected.flags,
        };
        state.piles.get_mut(PileId::Exhaust).make_mut().push(old);
        let before_selection = state.rng.get(RngStream::Sel);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            state.rng.get(RngStream::Sel).counter,
            before_selection.counter + 1
        );
        assert_eq!(
            state.piles.get(PileId::Exhaust).as_slice(),
            &[old, selected]
        );
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::from_i64(6));
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);

        // Only an Exhaust-pile Attack: no candidate, no draw, no growth.
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        hand.retain(|card| card.uid != selected.uid);
        state
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(selected);
        let before_selection = state.rng.get(RngStream::Sel);
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.rng.get(RngStream::Sel), before_selection);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::zero());
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), &[selected]);
    }

    #[test]
    fn thrash_owner_death_stops_before_selection_growth_and_reexhaust() {
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        state.hp = 1;
        state.max_hp = 1;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 1);
        let before_selection = state.rng.get(RngStream::Sel);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.hp, 0);
        assert_eq!(state.rng.get(RngStream::Sel), before_selection);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::zero());
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[selected]);
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    #[test]
    fn thrash_retains_the_exact_null_target_weak_shrink_fraction() {
        let (mut state, catalog, source, _) = thrash_fixture(0, CardId::StrikeIronclad);
        state.powers.set(PowerId::Strength, SlotWire::Int, 2);
        state.powers.set(PowerId::PlayerWeak, SlotWire::Int, 1);
        state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 1);
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(
            exact_growth(&state, source.uid),
            DotNetDecimal::ratio(21, 5).unwrap()
        );
    }

    #[test]
    fn zero_thrash_increase_preserves_the_initial_scalar_growth_wire() {
        let (mut state, catalog, source, _) = thrash_fixture(0, CardId::StrikeIronclad);
        state.powers.set(PowerId::Strength, SlotWire::Int, -100);
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        let instance = state.card_states.get(source.uid);
        assert_eq!(instance.exact_damage_growth(), Some(DotNetDecimal::zero()));
        assert!(!instance.exact_damage_growth_is_fraction());
        let projected = HotBoundary::to_canonical(&state, &catalog);
        let projected_thrash = projected
            .piles
            .values()
            .flatten()
            .find(|card| card.uid == Some(u64::from(source.uid)))
            .unwrap();
        assert_eq!(
            projected_thrash.physical_state,
            Some(serde_json::json!(["PHYSICAL_CARD_STATE", [], 0, false, []]))
        );
    }

    #[test]
    fn thrash_reads_live_physical_candidate_growth() {
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::KinglyPunch);
        state
            .card_states
            .add_damage_growth(selected.uid, 4)
            .unwrap();
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(
            exact_growth(&state, source.uid),
            DotNetDecimal::from_i64(12)
        );
        assert_eq!(state.card_states.get(selected.uid).damage_growth, 4);
    }

    #[test]
    fn thrash_snap_candidate_pins_the_complete_two_step_body() {
        let expected_select = [
            Arg::S("hand"),
            Arg::I(1),
            Arg::I(1),
            Arg::S("without_effective_retain"),
            Arg::S("apply_permanent_retain"),
        ];
        for (upgrade, damage) in [(0, 7), (1, 10)] {
            let row = crate::content_tables::card_row(CardId::Snap, upgrade).unwrap();
            let [body, select] = row.steps else {
                panic!("Snap must retain its exact two-step body");
            };
            assert_eq!(body.kind, StepKind::OstyBody);
            assert_eq!(body.args, &[Arg::S("single"), Arg::I(damage)]);
            assert_eq!(select.kind, StepKind::Select);
            assert_eq!(select.args, expected_select);

            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity(CardId::Snap, upgrade)).unwrap();
            let catalog = builder.build();
            let candidate = HotCard {
                uid: 7,
                atom,
                flags: 0,
            };
            let mut state = HotState::at_defaults();
            assert_eq!(
                thrash_candidate_base(&mut state, &catalog, candidate),
                Ok(DotNetDecimal::from_i64(damage))
            );
        }
    }

    #[test]
    fn lethal_thrash_keeps_selection_and_growth_but_gates_reexhaust() {
        let (mut state, catalog, source, selected) = thrash_fixture(0, CardId::StrikeIronclad);
        state.monsters_mut()[0].hp = 1;
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::from_i64(6));
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[selected]);
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    #[test]
    fn thrash_reexhaust_runs_dark_embrace_before_result_routing() {
        let (mut state, catalog, source, _) = thrash_fixture(0, CardId::StrikeIronclad);
        let defend = HotCard {
            uid: 8,
            atom: catalog.atom(&identity(CardId::DefendIronclad, 0)).unwrap(),
            flags: 0,
        };
        state.piles.get_mut(PileId::Draw).make_mut().push(defend);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        let mut events = Vec::new();

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[defend]);
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
        assert_eq!(exact_growth(&state, source.uid), DotNetDecimal::from_i64(6));
    }

    #[test]
    fn thrash_reads_damage_var_bodies_at_both_upgrade_levels() {
        // #2950: each row's canonical DamageVar, then its OnUpgrade delta.
        for (id, _, base, per_upgrade) in THRASH_DAMAGE_VAR_BODIES {
            for candidate_upgrade in [0_u8, 1] {
                let expected = base + per_upgrade * i64::from(candidate_upgrade);
                let (mut state, catalog, source, candidate) =
                    thrash_fixture_with(0, id, candidate_upgrade);
                assert_eq!(
                    thrash_candidate_base(&mut state, &catalog, candidate)
                        .unwrap_or_else(|e| panic!("{id:?}+{candidate_upgrade}: {e:?}")),
                    DotNetDecimal::from_i64(expected),
                    "{id:?}+{candidate_upgrade}"
                );
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    Some(0),
                    None,
                    &mut Vec::new(),
                )
                .unwrap();
                assert_eq!(
                    exact_growth(&state, source.uid),
                    DotNetDecimal::from_i64(expected),
                    "{id:?}+{candidate_upgrade}"
                );
            }
        }
    }

    #[test]
    fn queen_blockers_thrash_reads_native_damage_vars_for_special_attack_bodies() {
        for (id, expected) in [
            (CardId::HowlFromBeyond, 18),
            (CardId::Conflagration, 2),
            (CardId::Thunderclap, 4),
            (CardId::Pillage, 6),
            (CardId::Spite, 5),
            (CardId::Bully, 4),
        ] {
            let (mut state, catalog, source, candidate) = thrash_fixture(0, id);
            assert_eq!(
                thrash_candidate_base(&mut state, &catalog, candidate)
                    .unwrap_or_else(|e| panic!("{id:?}: {e:?}")),
                DotNetDecimal::from_i64(expected),
                "{id:?}"
            );
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                exact_growth(&state, source.uid),
                DotNetDecimal::from_i64(expected),
                "{id:?}"
            );
        }
    }

    #[test]
    fn unsupported_thrash_candidate_refuses_the_whole_public_play_atomically() {
        let (mut state, catalog, source, _) = thrash_fixture(0, CardId::Feed);
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 9 }];
        let events_before = events.clone();

        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Thrash damage candidate"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn thrash_preview_uses_exhausted_source_cost_without_active_play_resources() {
        use crate::hot::{LocalCostExpiration, LocalCostModifier, LocalCostModifierKind};
        for (id, local_cost, expected) in [
            (CardId::StrikeIronclad, None, 6),
            (CardId::Claw, None, 9),
            (CardId::StrikeIronclad, Some(0), 9),
            (CardId::Claw, Some(1), 6),
            (CardId::Whirlwind, Some(0), 6),
        ] {
            let (mut state, catalog, _, selected) = thrash_fixture(0, id);
            state.powers.set(PowerId::OneForAll, SlotWire::Int, 3);
            if let Some(cost) = local_cost {
                state.card_states.append_local_cost_modifier(
                    selected.uid,
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Set,
                        amount: cost,
                        expiration: LocalCostExpiration::ThisCombat,
                        reduce_only: false,
                    },
                );
            }
            for duration in [0, 1, 3] {
                state
                    .powers
                    .set(PowerId::DoubleDamage, SlotWire::Int, duration);
                state.powers.set(PowerId::PlayerWeak, SlotWire::Int, 1);
                state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 1);
                let factor = if duration == 0 { 1 } else { 2 };
                assert_eq!(
                    thrash_null_target_damage(
                        &state,
                        &catalog,
                        selected,
                        DotNetDecimal::from_i64(6)
                    )
                    .unwrap(),
                    DotNetDecimal::ratio(expected * factor * 21, 40).unwrap(),
                    "{id:?}/{local_cost:?}/{duration}"
                );
            }
        }
    }

    #[test]
    fn paid_and_free_thrash_use_the_candidates_cost_for_growth() {
        use crate::hot::{LocalCostExpiration, LocalCostModifier, LocalCostModifierKind};
        for free_thrash in [false, true] {
            for (candidate_id, growth) in [(CardId::StrikeIronclad, 6), (CardId::Claw, 6)] {
                let (mut state, catalog, source, _) = thrash_fixture(0, candidate_id);
                state.powers.set(PowerId::OneForAll, SlotWire::Int, 3);
                if free_thrash {
                    state.card_states.append_local_cost_modifier(
                        source.uid,
                        LocalCostModifier {
                            kind: LocalCostModifierKind::Set,
                            amount: 0,
                            expiration: LocalCostExpiration::ThisCombat,
                            reduce_only: false,
                        },
                    );
                }
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    Some(0),
                    None,
                    &mut Vec::new(),
                )
                .unwrap();
                assert_eq!(
                    exact_growth(&state, source.uid),
                    DotNetDecimal::from_i64(growth)
                );
                assert_eq!(state.monsters[0].hp, if free_thrash { 986 } else { 992 });
                let wire =
                    crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
                let cold_catalog =
                    crate::boundary::HotBoundary::catalog_from_canonical(&wire).unwrap();
                let cold =
                    crate::boundary::HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
                assert_eq!(
                    crate::boundary::HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
                    wire
                );
            }
        }
    }

    #[test]
    fn played_modifier_writers_compose_with_thrash_growth() {
        let (mut one_for_all, catalog, thrash, _) = thrash_fixture(0, CardId::StrikeIronclad);
        let writer = HotCard {
            uid: 8,
            atom: catalog.atom(&identity(CardId::OneForAll, 0)).unwrap(),
            flags: 0,
        };
        one_for_all
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(writer);
        crate::engine::play::autoplay_collected_cards(
            &mut one_for_all,
            &catalog,
            &[writer],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(one_for_all.powers.value(PowerId::OneForAll), 3);
        crate::engine::play::play_card(
            &mut one_for_all,
            &catalog,
            thrash.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            exact_growth(&one_for_all, thrash.uid),
            DotNetDecimal::from_i64(6)
        );

        let (mut shadow_step, catalog, thrash, candidate) =
            thrash_fixture(0, CardId::StrikeIronclad);
        shadow_step.piles.get_mut(PileId::Hand).make_mut().clear();
        // Both return to the Hand at the next draw, so the Hand Attack is
        // still Thrash's candidate (#2519).
        shadow_step
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([thrash, candidate]);
        let writer = HotCard {
            uid: 8,
            atom: catalog.atom(&identity(CardId::ShadowStep, 0)).unwrap(),
            flags: 0,
        };
        shadow_step
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(writer);
        crate::engine::play::autoplay_collected_cards(
            &mut shadow_step,
            &catalog,
            &[writer],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(shadow_step.powers.value(PowerId::ShadowStep), 1);
        shadow_step = apply_action(&shadow_step, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(shadow_step.powers.value(PowerId::ShadowStep), 0);
        assert_eq!(shadow_step.powers.value(PowerId::DoubleDamage), 1);
        assert!(
            shadow_step
                .piles
                .get(PileId::Hand)
                .as_slice()
                .contains(&thrash)
        );
        assert!(
            shadow_step
                .piles
                .get(PileId::Hand)
                .as_slice()
                .contains(&candidate)
        );
        crate::engine::play::play_card(
            &mut shadow_step,
            &catalog,
            thrash.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            exact_growth(&shadow_step, thrash.uid),
            DotNetDecimal::from_i64(12)
        );
    }

    #[test]
    fn manifest_partitions_owned_kinds() {
        let manifest = capability_manifest();
        for kind in ESCALATED {
            assert!(!manifest.steps.contains(&kind));
            assert!(!IMPLEMENTED.contains(&kind));
        }
        for kind in IMPLEMENTED {
            assert!(manifest.steps.contains(kind));
            assert!(OWNED.contains(kind));
        }
        assert_eq!(IMPLEMENTED.len() + ESCALATED.len(), OWNED.len());
    }

    #[test]
    fn every_generated_carrier_and_operand_is_mechanically_pinned() {
        let carriers = CARD_ROWS
            .iter()
            .filter_map(|row| {
                let step = row.steps.iter().find(|step| OWNED.contains(&step.kind))?;
                Some((row.id, row.upgrade, step.kind, step.args.to_vec()))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (CardId::Cascade, 0, StepKind::AutoplayDrawX, vec![Arg::I(0)]),
                (CardId::Cascade, 1, StepKind::AutoplayDrawX, vec![Arg::I(1)]),
                (
                    CardId::CrimsonMantle,
                    0,
                    StepKind::CrimsonMantle,
                    vec![Arg::I(7)]
                ),
                (
                    CardId::CrimsonMantle,
                    1,
                    StepKind::CrimsonMantle,
                    vec![Arg::I(10)]
                ),
                (
                    CardId::FiendFire,
                    0,
                    StepKind::FiendFireExact,
                    vec![Arg::I(7)]
                ),
                (
                    CardId::FiendFire,
                    1,
                    StepKind::FiendFireExact,
                    vec![Arg::I(10)]
                ),
                (
                    CardId::HeavenlyDrill,
                    0,
                    StepKind::HeavenlyDrillX,
                    vec![Arg::I(8), Arg::I(4)]
                ),
                (
                    CardId::HeavenlyDrill,
                    1,
                    StepKind::HeavenlyDrillX,
                    vec![Arg::I(10), Arg::I(4)]
                ),
                (
                    CardId::PactsEnd,
                    0,
                    StepKind::PactsEndExact,
                    vec![Arg::I(18), Arg::I(3)]
                ),
                (
                    CardId::PactsEnd,
                    1,
                    StepKind::PactsEndExact,
                    vec![Arg::I(24), Arg::I(3)]
                ),
                (
                    CardId::PrimalForce,
                    0,
                    StepKind::PrimalForceExact,
                    vec![Arg::I(0)]
                ),
                (
                    CardId::PrimalForce,
                    1,
                    StepKind::PrimalForceExact,
                    vec![Arg::I(1)]
                ),
                (CardId::Stoke, 0, StepKind::StokeExact, vec![]),
                (CardId::Stoke, 1, StepKind::StokeExact, vec![]),
                (CardId::Tank, 0, StepKind::TankExact, vec![Arg::I(1)]),
                (CardId::Tank, 1, StepKind::TankExact, vec![Arg::I(1)]),
                (
                    CardId::Thrash,
                    0,
                    StepKind::ThrashExact,
                    vec![Arg::I(4), Arg::I(2)]
                ),
                (
                    CardId::Thrash,
                    1,
                    StepKind::ThrashExact,
                    vec![Arg::I(6), Arg::I(2)]
                ),
            ]
        );
    }

    #[test]
    fn family_census_is_sixteen_admitted_rows_with_exact_two_row_deltas() {
        let family_rows = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| OWNED.contains(&step.kind)))
            .collect::<Vec<_>>();
        assert_eq!(family_rows.len(), 18);
        let admitted = family_rows
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .all(|step| crate::steps::is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![
                (CardId::Cascade, 0),
                (CardId::Cascade, 1),
                (CardId::CrimsonMantle, 0),
                (CardId::CrimsonMantle, 1),
                (CardId::FiendFire, 0),
                (CardId::FiendFire, 1),
                (CardId::HeavenlyDrill, 0),
                (CardId::HeavenlyDrill, 1),
                (CardId::PactsEnd, 0),
                (CardId::PactsEnd, 1),
                (CardId::PrimalForce, 0),
                (CardId::PrimalForce, 1),
                (CardId::Stoke, 0),
                (CardId::Stoke, 1),
                (CardId::Thrash, 0),
                (CardId::Thrash, 1),
            ]
        );
        let primal_force_delta = admitted
            .iter()
            .copied()
            .filter(|(id, _)| *id == CardId::PrimalForce)
            .collect::<Vec<_>>();
        assert_eq!(
            primal_force_delta,
            vec![(CardId::PrimalForce, 0), (CardId::PrimalForce, 1)]
        );
        let crimson_delta = admitted
            .iter()
            .copied()
            .filter(|(id, _)| *id == CardId::CrimsonMantle)
            .collect::<Vec<_>>();
        assert_eq!(
            crimson_delta,
            vec![(CardId::CrimsonMantle, 0), (CardId::CrimsonMantle, 1)]
        );
        let cascade = family_rows
            .iter()
            .filter(|row| matches!(row.id, CardId::Cascade))
            .collect::<Vec<_>>();
        assert_eq!(cascade.len(), 2);
        // #3147: CardEnergyCost::.ctor RVA 0x11e002 stores 0 into Canonical
        // for an X-cost card (IL_0022..IL_002d), overriding Cascade's -1
        // constructor argument (Cascade::.ctor RVA 0xdab73 IL_0002).
        assert!(cascade.iter().all(|row| row.cost == 0 && row.x_cost));
        assert!(crate::steps::is_implemented(StepKind::AutoplayDrawX));
        assert!(crate::steps::is_implemented(StepKind::CrimsonMantle));
        assert!(crate::steps::is_implemented(StepKind::PrimalForceExact));
        assert!(crate::steps::is_implemented(StepKind::ThrashExact));
    }

    #[test]
    fn crimson_mantle_rows_stack_public_and_private_amounts_in_order() {
        for (upgrade, amount) in [(0, 7), (1, 10)] {
            let (mut state, catalog, source) = crimson_fixture(upgrade, PileId::Play);
            state.powers.set(PowerId::CrimsonMantle, SlotWire::Int, 2);
            state.powers.set(PowerId::CrimsonSelf, SlotWire::Int, 3);
            let spec = *catalog.spec(source.atom).unwrap();
            let mut events = Vec::new();
            crimson_mantle(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(i64::from(amount))],
                events: &mut events,
            })
            .unwrap();

            assert_eq!(state.powers.value(PowerId::CrimsonMantle), 2 + amount);
            assert_eq!(state.powers.value(PowerId::CrimsonSelf), 4);
            assert_eq!(
                events,
                vec![
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::CrimsonMantle,
                        amount: 2 + amount,
                    },
                    Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::CrimsonSelf,
                        amount: 4,
                    },
                ]
            );
        }
    }

    #[test]
    fn crimson_mantle_manual_and_collected_autoplay_are_public_and_exact() {
        let (state, catalog, source) = crimson_fixture(0, PileId::Hand);
        let transition = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap();
        assert_eq!(transition.state.powers.value(PowerId::CrimsonMantle), 7);
        assert_eq!(transition.state.powers.value(PowerId::CrimsonSelf), 1);
        assert_eq!(transition.state.energy, 2);
        assert!(PileId::ALL.into_iter().all(|pile| {
            transition
                .state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.uid != source.uid)
        }));

        let (mut autoplay, catalog, source) = crimson_fixture(1, PileId::Draw);
        let mut events = Vec::new();
        crate::engine::play::autoplay_collected_cards(
            &mut autoplay,
            &catalog,
            &[source],
            &mut events,
        )
        .unwrap();
        assert_eq!(autoplay.powers.value(PowerId::CrimsonMantle), 10);
        assert_eq!(autoplay.powers.value(PowerId::CrimsonSelf), 1);
        assert_eq!(autoplay.energy, 3, "AutoPlay does not spend Energy");
        assert!(PileId::ALL.into_iter().all(|pile| {
            autoplay
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.uid != source.uid)
        }));
    }

    #[test]
    fn crimson_mantle_preflight_and_body_failures_are_atomic() {
        let (mut state, catalog, source) = crimson_fixture(0, PileId::Hand);
        state
            .powers
            .set(PowerId::CrimsonMantle, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("crimson_mantle"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let (mut body_state, catalog, source) = crimson_fixture(1, PileId::Play);
        body_state
            .powers
            .set(PowerId::CrimsonSelf, SlotWire::Int, i32::MAX);
        let before = body_state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let result = crimson_mantle(&mut StepCtx {
            state: &mut body_state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10)],
            events: &mut events,
        });
        assert_eq!(result, Err(EngineRefusal::CounterOverflow("crimson_self")));
        assert_eq!(body_state, before);
        assert!(events.is_empty());

        for duplicate in [false, true] {
            let (mut malformed, catalog, source) = crimson_fixture(0, PileId::Play);
            if duplicate {
                malformed
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(source);
            } else {
                malformed.piles.get_mut(PileId::Play).make_mut().clear();
            }
            let before = malformed.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let mut events = Vec::new();
            assert_eq!(
                crimson_mantle(&mut StepCtx {
                    state: &mut malformed,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(7)],
                    events: &mut events,
                }),
                Err(EngineRefusal::ActiveCardNotUnique {
                    uid: source.uid,
                    matches: if duplicate { 2 } else { 0 },
                })
            );
            assert_eq!(malformed, before);
            assert!(events.is_empty());
        }

        for (args, target, selection, x_value) in [
            (&[][..], None, None, 0),
            (&[CompiledArg::I(10)][..], Some(0), None, 0),
            (&[CompiledArg::I(10)][..], None, Some(1), 0),
            (&[CompiledArg::I(10)][..], None, None, 1),
        ] {
            let (mut malformed, catalog, source) = crimson_fixture(1, PileId::Play);
            let before = malformed.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let mut events = Vec::new();
            assert_eq!(
                crimson_mantle(&mut StepCtx {
                    state: &mut malformed,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target,
                    selection,
                    x_value,
                    args,
                    events: &mut events,
                }),
                Err(EngineRefusal::MalformedArgs("crimson_mantle"))
            );
            assert_eq!(malformed, before);
            assert!(events.is_empty());
        }

        let (mut terminal, catalog, source) = crimson_fixture(0, PileId::Play);
        terminal.history.over = true;
        terminal
            .powers
            .set(PowerId::CrimsonMantle, SlotWire::Int, i32::MAX);
        terminal
            .powers
            .set(PowerId::CrimsonSelf, SlotWire::Int, i32::MAX);
        let before = terminal.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        crimson_mantle(&mut StepCtx {
            state: &mut terminal,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(7)],
            events: &mut events,
        })
        .unwrap();
        assert_eq!(terminal, before);
        assert!(events.is_empty());
    }

    #[test]
    fn primal_force_catalog_closure_and_eternal_quotient_are_complete() {
        for level in 0..=1 {
            let mut builder = CatalogBuilder::new();
            builder
                .intern(identity(CardId::PrimalForce, level))
                .unwrap();
            let catalog = builder.build();
            assert!(
                catalog.atom(&identity(CardId::GiantRock, level)).is_some(),
                "Primal Force L{level} must close over its exact Rock level"
            );
        }

        // Python's exact native Eternal census is the only immutable identity
        // exclusion beyond Type Attack. All seven rows are non-Attacks, while
        // dynamic Eternal keyword/enchantment payloads are absent from the
        // admitted CardInstanceState boundary, so `is_attack` is exact for
        // every representable candidate rather than an approximation.
        let eternal = [
            CardId::AscendersBane,
            CardId::BadLuck,
            CardId::CurseOfTheBell,
            CardId::Enthralled,
            CardId::Folly,
            CardId::ForbiddenGrimoire,
            CardId::Greed,
        ];
        for id in eternal {
            for row in CARD_ROWS
                .iter()
                .filter(|row| matches!(row.id, candidate if candidate == id))
            {
                let mut builder = CatalogBuilder::new();
                let atom = builder.intern(identity(id, row.upgrade)).unwrap();
                let catalog = builder.build();
                assert!(!catalog.spec(atom).unwrap().is_attack, "{id:?}");
            }
        }
        // The seven ids above are the frozen v0.111.0 Python
        // `_W181_CANONICAL_ETERNAL_CARD_IDS` census, exactly (seven members,
        // each once). The half of this test that re-read that set out of
        // `combat_sim.py` was retired with the Python simulator (#2827 item
        // D): the source is frozen, so the census is a constant, and the list
        // above now carries it.
    }

    #[test]
    fn primal_force_transforms_frozen_attacks_in_place_with_fresh_identity() {
        let (mut state, catalog, source, strike, defend, sovereign) = primal_fixture(0);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend, sovereign]);
        let rich = CardInstanceState {
            damage_growth: 7,
            local_retain: true,
            local_sly: true,
            ..CardInstanceState::default()
        };
        state.card_states.set(sovereign.uid, rich);
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(0)],
            events: &mut events,
        };

        primal_force_exact(&mut ctx).unwrap();

        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 3);
        assert_eq!(hand[0].uid, 100);
        assert_eq!(hand[1], defend);
        assert_eq!(hand[2].uid, 101);
        assert!(hand.iter().enumerate().all(|(index, card)| {
            index == 1
                || catalog
                    .spec(card.atom)
                    .is_some_and(|row| row.identity == identity(CardId::GiantRock, 0))
        }));
        assert_eq!(state.next_card_uid, 102);
        assert_eq!(state.ps_strikes, 0);
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);
        assert!(state.card_states.get(sovereign.uid).is_vacant());
        assert!(state.exact_piles);
        assert!(events.is_empty());
    }

    #[test]
    fn primal_force_refuses_a_hopper_master_attack_before_public_play_mutation() {
        let (mut state, catalog, source, strike, _defend, _sovereign) = primal_fixture(0);
        state.exact_piles = true;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, strike]);
        state.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card: strike,
                state: state.card_states.get(strike.uid),
            }],
            history: Vec::new(),
        }));
        let mut hopper = HotMonster::new(
            MonsterKind::ThievingHopper,
            crate::engine::THIEVING_HOPPER_HP,
        );
        hopper.max_hp = crate::engine::THIEVING_HOPPER_HP;
        hopper.powers.set(
            PowerId::EscapeArtist,
            SlotWire::Int,
            crate::engine::THIEVING_HOPPER_ESCAPE_ARTIST,
        );
        state.monsters_mut()[0] = hopper;
        assert!(crate::engine::monsters::thieving_hopper_deck_payload_is_exact(&state, &catalog,));

        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let events_before = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "persistent encounter-card transform"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    /// #2941: Primal Force refuses an Attack still linked to an ordinary
    /// fight's `DeckVersion` (`monsters::deck_version_link_owns_uid`); the gate
    /// reads the link, not the card id.
    #[test]
    fn primal_force_refuses_a_deck_version_linked_attack_before_public_play_mutation() {
        let (mut state, catalog, source, strike, _defend, _sovereign) = primal_fixture(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, strike]);
        state
            .card_states
            .set_scythe_deck_links(vec![(strike.uid, 0)])
            .unwrap();
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let events_before = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "persistent encounter-card transform"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn primal_force_upgrade_ringing_and_generated_listeners_are_serial() {
        let (mut state, catalog, source, strike, defend, sovereign) = primal_fixture(1);
        state.set_ringing(true);
        let ringing = CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING;
        let mut source = source;
        source.flags = ringing;
        let mut strike = strike;
        strike.flags = ringing;
        let mut defend = defend;
        defend.flags = ringing;
        let mut sovereign = sovereign;
        sovereign.flags |= CARD_FLAG_RINGING;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend, sovereign]);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 3);
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation, PowerId::Arsenal,])
        );
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };

        primal_force_exact(&mut ctx).unwrap();

        let rocks = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .filter(|card| card.uid >= 100)
            .collect::<Vec<_>>();
        assert_eq!(rocks.len(), 2);
        assert!(rocks.iter().all(|card| {
            card.flags & ringing == ringing
                && catalog
                    .spec(card.atom)
                    .is_some_and(|row| row.identity == identity(CardId::GiantRock, 1))
        }));
        assert_eq!(state.block, 6);
        assert_eq!(state.powers.value(PowerId::Strength), 4);
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);
    }

    #[test]
    fn primal_force_terminal_and_failures_are_body_atomic() {
        let (mut state, catalog, source, strike, _, _) = primal_fixture(0);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.piles.get_mut(PileId::Hand).make_mut().push(strike);
        let spec = *catalog.spec(source.atom).unwrap();

        let mut ending = state.clone();
        ending.history.over = true;
        let before = ending.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut ending,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(0)],
            events: &mut events,
        };
        primal_force_exact(&mut ctx).unwrap();
        assert_eq!(ending, before);
        assert!(events.is_empty());

        for (owner_history, overflow) in [(true, "owner history"), (false, "hook uid")] {
            let mut malformed = state.clone();
            if owner_history {
                malformed.history.owner_generated_cards_combat = i32::MAX;
            } else {
                malformed.next_generated_hook_uid = i32::MAX;
            }
            let before = malformed.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut malformed,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(0)],
                events: &mut events,
            };
            assert!(matches!(
                primal_force_exact(&mut ctx),
                Err(EngineRefusal::CounterOverflow(_))
            ));
            assert_eq!(malformed, before, "{overflow}");
            assert!(events.is_empty(), "{overflow}");
        }

        let mut duplicate = state.clone();
        duplicate
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut duplicate,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(0)],
            events: &mut events,
        };
        assert!(matches!(
            primal_force_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique { matches: 2, .. })
        ));
        assert_eq!(duplicate, before);
        assert!(events.is_empty());

        for (args, target, selection, x_value) in [
            (vec![CompiledArg::I(1)], None, None, 0),
            (vec![CompiledArg::I(0)], Some(0), None, 0),
            (vec![CompiledArg::I(0)], None, Some(strike.uid), 0),
            (vec![CompiledArg::I(0)], None, None, 1),
        ] {
            let mut malformed = state.clone();
            let before = malformed.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut malformed,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection,
                x_value,
                args: &args,
                events: &mut events,
            };
            assert_eq!(
                primal_force_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs("primal_force_exact"))
            );
            assert_eq!(malformed, before);
            assert!(events.is_empty());
        }

        let mut collision = state.clone();
        collision
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: collision.next_card_uid,
                atom: strike.atom,
                flags: 0,
            });
        let before = collision.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut collision,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(0)],
            events: &mut events,
        };
        assert_eq!(
            primal_force_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "primal_force_exact next uid collision"
            ))
        );
        assert_eq!(collision, before);
        assert!(events.is_empty());
    }

    #[test]
    fn cascade_uses_captured_x_and_routes_each_top_card_naturally() {
        let cascade = identity(CardId::Cascade, 0);
        let defend = identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let cascade_atom = builder.intern(cascade).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: cascade_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        let spec = *catalog.spec(cascade_atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 2,
            args: &[CompiledArg::I(0)],
            events: &mut events,
        };
        autoplay_draw_x(&mut ctx).unwrap();
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(state.piles.get(PileId::Discard).len(), 2);
        assert_eq!(state.piles.get(PileId::Exhaust).len(), 0);
        assert_eq!(state.block, 10);
        assert_eq!(state.history.card_plays_finished_combat, 2);

        let mut malformed = state.clone();
        let before = malformed.clone();
        let mut no_events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut malformed,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 2,
            args: &[CompiledArg::I(1)],
            events: &mut no_events,
        };
        assert_eq!(
            autoplay_draw_x(&mut ctx),
            Err(EngineRefusal::MalformedArgs("autoplay_draw_x owner"))
        );
        assert_eq!(malformed, before);
        assert!(no_events.is_empty());
    }

    #[test]
    fn upgraded_cascade_adds_its_level_to_the_captured_x_count() {
        let cascade = identity(CardId::Cascade, 1);
        let defend = identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let cascade_atom = builder.intern(cascade).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: cascade_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        let spec = *catalog.spec(cascade_atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 1,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };

        autoplay_draw_x(&mut ctx).unwrap();

        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(state.piles.get(PileId::Discard).len(), 2);
        assert_eq!(state.block, 10);
        assert_eq!(state.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn fiend_fire_freezes_hand_order_exhausts_each_card_then_attacks_by_count() {
        let (mut state, catalog, source, strike, defend) = fiend_fixture(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend]);
        let spec = *catalog.spec(source.atom).unwrap();
        let args = [CompiledArg::I(7)];
        let mut events = Vec::new();
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

        fiend_fire_exact(&mut ctx).unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![strike.uid, defend.uid]
        );
        assert_eq!(state.monsters[0].hp, 36);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardResolved {
                        uid,
                        pile: PileId::Exhaust,
                    } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![strike.uid, defend.uid]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn fiend_fire_empty_hand_runs_a_real_zero_hit_attack_command() {
        let (mut state, catalog, source, _, _) = fiend_fixture(1);
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let spec = *catalog.spec(source.atom).unwrap();
        let args = [CompiledArg::I(10)];
        let mut events = Vec::new();
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

        fiend_fire_exact(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].hp, 50);
        assert_eq!(state.powers.value(PowerId::Vigor), 0);
        assert!(events.iter().any(|event| matches!(
            event,
            Event::PowerChanged {
                power: PowerId::Vigor,
                amount: 0,
                ..
            }
        )));
    }

    #[test]
    fn fiend_fire_ending_exhaust_hook_stops_frozen_batch_and_attack() {
        let (mut state, catalog, source, strike, defend) = fiend_fixture(0);
        state.monsters_mut()[0].hp = 1;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: strike.atom,
            flags: 0,
        });
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 66);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.fanouts.set_cacophony_left(1);
        let spec = *catalog.spec(source.atom).unwrap();
        let args = [CompiledArg::I(7)];
        let mut events = Vec::new();
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

        fiend_fire_exact(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(
            state
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![strike.uid]
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![defend.uid, 3]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            1,
            "only Cacophony damages; Fiend Fire's final attack is suppressed"
        );
    }

    #[test]
    fn fiend_fire_rejects_noncanonical_operands_and_nonunique_source() {
        let (mut state, catalog, source, _, _) = fiend_fixture(0);
        let spec = *catalog.spec(source.atom).unwrap();
        let before = state.clone();
        let bad_args = [CompiledArg::I(10)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &bad_args,
            events: &mut events,
        };
        assert_eq!(
            fiend_fire_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("fiend_fire_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        let args = [CompiledArg::I(7)];
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
        assert_eq!(
            fiend_fire_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
    }

    #[test]
    fn stoke_pool_order_catalog_closure_and_private_public_split_are_pinned() {
        assert_eq!(
            crate::content_tables::STOKE_CARD_POOL_V109,
            [
                CardId::Aggression,
                CardId::Anger,
                CardId::Armaments,
                CardId::AshenStrike,
                CardId::Barricade,
                CardId::BattleTrance,
                CardId::BloodWall,
                CardId::Bloodletting,
                CardId::Bludgeon,
                CardId::BodySlam,
                CardId::Brand,
                CardId::Breakthrough,
                CardId::Bully,
                CardId::BurningPact,
                CardId::Cascade,
                CardId::Cinder,
                CardId::Colossus,
                CardId::Conflagration,
                CardId::CrimsonMantle,
                CardId::Cruelty,
                CardId::DarkEmbrace,
                CardId::DemonForm,
                CardId::Dismantle,
                CardId::Dominate,
                CardId::DrumOfBattle,
                CardId::EvilEye,
                CardId::ExpectAFight,
                CardId::FeelNoPain,
                CardId::FiendFire,
                CardId::FightMe,
                CardId::FlameBarrier,
                CardId::ForgottenRitual,
                CardId::Havoc,
                CardId::Headbutt,
                CardId::Hellraiser,
                CardId::Hemokinesis,
                CardId::HowlFromBeyond,
                CardId::Impervious,
                CardId::InfernalBlade,
                CardId::Inferno,
                CardId::Inflame,
                CardId::IronWave,
                CardId::Juggernaut,
                CardId::Juggling,
                CardId::Mangle,
                CardId::MoltenFist,
                CardId::Offering,
                CardId::OneTwoPunch,
                CardId::PactsEnd,
                CardId::PerfectedStrike,
                CardId::Pillage,
                CardId::PommelStrike,
                CardId::PrimalForce,
                CardId::Pyre,
                CardId::Rage,
                CardId::Rampage,
                CardId::Rupture,
                CardId::SecondWind,
                CardId::SetupStrike,
                CardId::ShrugItOff,
                CardId::Spite,
                CardId::Stampede,
                CardId::Stoke,
                CardId::Stomp,
                CardId::StoneArmor,
                CardId::SwordBoomerang,
                CardId::Taunt,
                CardId::TearAsunder,
                CardId::Thrash,
                CardId::Thunderclap,
                CardId::Tremble,
                CardId::TrueGrit,
                CardId::TwinStrike,
                CardId::Unmovable,
                CardId::Unrelenting,
                CardId::Uppercut,
                CardId::Vicious,
                CardId::Whirlwind,
            ]
        );
        let (mut state, catalog, source) = stoke_fixture(0, 0);
        for id in crate::content_tables::STOKE_CARD_POOL_V109 {
            for upgrade in [0, 1] {
                let expected = identity(id, upgrade);
                let spec = catalog.spec(catalog.atom(&expected).unwrap()).unwrap();
                assert_eq!(spec.identity, expected);
                assert_eq!(
                    spec.row,
                    crate::content_tables::card_row(id, upgrade).unwrap()
                );
            }
        }

        assert!(IMPLEMENTED.contains(&StepKind::StokeExact));
        assert!(crate::steps::is_implemented(StepKind::StokeExact));
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::StokeExact)
        );
        let spec = *catalog.spec(source.atom).unwrap();
        let mut events = vec![Event::TurnEnded { turn: 7 }];
        let before = state.clone();
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };
        stoke_exact(&mut ctx).unwrap();
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn stoke_foundation_freezes_hand_samples_with_replacement_and_upgrades_after_sampling() {
        for upgrade in [0, 1] {
            let (mut state, catalog, source) = stoke_fixture(upgrade, 2);
            let before_rng = state.rng.get(RngStream::Generation);
            let mut expected_rng = Xoshiro256StarStar {
                words: before_rng.words,
                counter: before_rng.counter,
            };
            let expected: [CardId; 2] = std::array::from_fn(|_| {
                let index = usize::try_from(expected_rng.next_bounded(78).unwrap()).unwrap();
                crate::content_tables::STOKE_CARD_POOL_V109[index]
            });
            let mut events = Vec::new();

            run_stoke_foundation(&mut state, &catalog, source, &mut events).unwrap();

            assert_eq!(
                state
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [1, 2]
            );
            let generated = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity)
                .collect::<Vec<_>>();
            assert_eq!(
                generated,
                expected
                    .into_iter()
                    .map(|id| identity(id, upgrade))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                state.rng.get(RngStream::Generation),
                RngStreamState {
                    words: expected_rng.words,
                    counter: expected_rng.counter,
                }
            );
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                before_rng.counter + 2
            );
            assert_eq!(state.history.owner_generated_cards_combat, 2);
            assert_eq!(state.next_card_uid, 102);
            assert_eq!(state.next_generated_hook_uid, 2);
            assert!(state.exact_piles);
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        Event::CardResolved { uid, pile } => Some((*uid, *pile)),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                [
                    (1, PileId::Exhaust),
                    (2, PileId::Exhaust),
                    (100, PileId::Hand),
                    (101, PileId::Hand),
                ]
            );
        }
    }

    #[test]
    fn stoke_samples_each_current_owner_pool() {
        // Pins the seeded native-style draws independently of Stoke's sampling
        // implementation. Both source levels share the selected CardId and
        // differ only in the post-sample source-level upgrade.
        let expected_ids = |owner| match owner {
            RewardPool::Defect => [CardId::Fusion, CardId::Overclock],
            RewardPool::Ironclad => [CardId::FlameBarrier, CardId::Pillage],
            RewardPool::Necrobinder => [CardId::Friendship, CardId::Putrefy],
            RewardPool::Regent => [CardId::Glimmer, CardId::PaleBlueDot],
            RewardPool::Silent => [CardId::Expose, CardId::NoxiousFumes],
        };
        let expected_rng = RngStreamState {
            words: [
                15_588_868_684_969_064_775,
                14_952_980_239_575_093_355,
                3_863_386_207_449_020_461,
                7_802_762_827_335_776_535,
            ],
            counter: 2,
        };
        for owner in RewardPool::ALL {
            for upgrade in [0, 1] {
                let (mut state, catalog, source) = stoke_fixture_for_owner(owner, upgrade, 2);
                let seeded = Xoshiro256StarStar::from_seed(47);
                state.rng.set(
                    RngStream::Generation,
                    RngStreamState {
                        words: seeded.words,
                        counter: seeded.counter,
                    },
                );
                let expected = expected_ids(owner)
                    .into_iter()
                    .map(|id| identity(id, upgrade))
                    .collect::<Vec<_>>();

                run_stoke_foundation(&mut state, &catalog, source, &mut Vec::new()).unwrap();

                assert_eq!(
                    state
                        .piles
                        .get(PileId::Hand)
                        .as_slice()
                        .iter()
                        .map(|card| catalog.spec(card.atom).unwrap().identity)
                        .collect::<Vec<_>>(),
                    expected,
                    "{owner:?} Stoke+{upgrade}",
                );
                assert_eq!(state.rng.get(RngStream::Generation), expected_rng);
            }
        }
    }

    #[test]
    fn stoke_private_source_authentication_accepts_every_unique_physical_pile() {
        for pile in PileId::ALL {
            let (mut state, catalog, source) = stoke_fixture(0, 1);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state.piles.get_mut(pile).make_mut().push(source);
            let expected_samples = if pile == PileId::Hand { 2 } else { 1 };

            run_stoke_foundation(&mut state, &catalog, source, &mut Vec::new()).unwrap();

            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                expected_samples
            );
            if pile == PileId::Hand {
                assert!(
                    state
                        .piles
                        .get(PileId::Exhaust)
                        .as_slice()
                        .iter()
                        .any(|card| card.uid == source.uid)
                );
            } else {
                assert!(
                    state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .any(|card| card.uid == source.uid)
                );
            }
        }
    }

    #[test]
    fn stoke_empty_hand_consumes_no_rng_and_starts_no_generated_command() {
        let (mut state, catalog, source) = stoke_fixture(1, 0);
        let before_rng = state.rng.get(RngStream::Generation);
        let mut events = Vec::new();
        run_stoke_foundation(&mut state, &catalog, source, &mut events).unwrap();
        assert_eq!(state.rng.get(RngStream::Generation), before_rng);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_card_uid, 100);
        assert!(!state.exact_piles);
        assert!(events.is_empty());
    }

    #[test]
    fn stoke_hand_cap_routes_each_generated_result_after_exhaust_draws() {
        let (mut state, catalog, source) = stoke_fixture(0, 10);
        let draw_atom = catalog.atom(&identity(CardId::Anger, 0)).unwrap();
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((20..30).map(|uid| HotCard {
                uid,
                atom: draw_atom,
                flags: 0,
            }));
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );

        run_stoke_foundation(&mut state, &catalog, source, &mut Vec::new()).unwrap();

        assert_eq!(state.piles.get(PileId::Exhaust).len(), 10);
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Discard).len(), 10);
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            (100..110).collect::<Vec<_>>()
        );
    }

    #[test]
    fn stoke_terminal_exhaust_stops_the_frozen_walk_but_still_samples_original_count() {
        let (mut state, catalog, source) = stoke_fixture(1, 2);
        state.monsters_mut()[0].hp = 1;
        let draw_atom = catalog.atom(&identity(CardId::Anger, 0)).unwrap();
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 20,
            atom: draw_atom,
            flags: 0,
        });
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.fanouts.set_cacophony_left(1);
        let before_rng = state.rng.get(RngStream::Generation).counter;

        run_stoke_foundation(&mut state, &catalog, source, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 20]
        );
        assert_eq!(state.rng.get(RngStream::Generation).counter, before_rng + 2);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_card_uid, 100);
    }

    #[test]
    fn stoke_late_generated_failures_roll_back_state_rng_and_preexisting_events() {
        for (case, mutate) in [
            |state: &mut HotState| state.next_card_uid = u32::MAX,
            |state: &mut HotState| state.history.owner_generated_cards_combat = i32::MAX,
            |state: &mut HotState| {
                state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
                state.powers.set(PowerId::Strength, SlotWire::Int, i32::MAX);
                assert!(
                    state
                        .fanouts
                        .set_local_generated_power_order(&[PowerId::Arsenal])
                );
            },
        ]
        .into_iter()
        .enumerate()
        {
            let (mut state, catalog, source) = stoke_fixture(0, 1);
            mutate(&mut state);
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 9 }];
            let before_events = events.clone();
            assert!(
                run_stoke_foundation(&mut state, &catalog, source, &mut events).is_err(),
                "overflow case {case}"
            );
            assert_eq!(state, before, "overflow case {case}");
            assert_eq!(events, before_events, "overflow case {case}");
        }

        // The first member of the begun plural command can succeed in the
        // rehearsal before the second member's record-before-add accounting
        // overflows. The outer command transaction must still publish none of
        // the earlier exhaust, RNG, history, hook-epoch, pile, or event prefix.
        let (mut state, catalog, source) = stoke_fixture(0, 2);
        state.history.owner_generated_cards_combat = i32::MAX - 1;
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 10 }];
        let before_events = events.clone();
        assert!(run_stoke_foundation(&mut state, &catalog, source, &mut events).is_err());
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn stoke_authentication_forgeries_refuse_atomically() {
        let (base, catalog, source) = stoke_fixture(0, 1);
        let cases = 7;
        for case in 0..cases {
            let mut state = base.clone();
            match case {
                0 => state.reward_card_pool = Some(RewardPool::Silent),
                1 => state.entropy_card_pool = Some(RewardPool::Regent),
                2 => state.fully_unlocked_card_pool_epochs = false,
                3 => state
                    .rng
                    .set(RngStream::Generation, RngStreamState::default()),
                4 => state.piles.get_mut(PileId::Discard).make_mut().push(source),
                5 => state
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(HotCard {
                        uid: 1,
                        atom: source.atom,
                        flags: 0,
                    }),
                6 => state.multiplayer_ally_key = 1,
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 11 }];
            let before_events = events.clone();
            assert!(
                run_stoke_foundation(&mut state, &catalog, source, &mut events).is_err(),
                "forgery case {case}"
            );
            assert_eq!(state, before, "forgery case {case}");
            assert_eq!(events, before_events, "forgery case {case}");
        }

        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::Stoke, 0)).unwrap();
        let incomplete = builder.build();
        let mut state = base;
        state.piles.get_mut(PileId::Play).make_mut()[0].atom = source_atom;
        let source = state.piles.get(PileId::Play).as_slice()[0];
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 12 }];
        let before_events = events.clone();
        assert!(matches!(
            run_stoke_foundation(&mut state, &incomplete, source, &mut events),
            Err(EngineRefusal::UnknownMintIdentity(_))
        ));
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        let (base, catalog, source) = stoke_fixture(0, 1);
        let canonical_spec = *catalog.spec(source.atom).unwrap();
        for case in 0..11 {
            let mut state = base.clone();
            let mut spec = canonical_spec;
            let mut args = Vec::new();
            let mut target = None;
            let mut selection = None;
            let mut x_value = 0;
            match case {
                0 => args.push(CompiledArg::I(1)),
                1 => target = Some(0),
                2 => selection = Some(1),
                3 => x_value = 1,
                4 => spec.identity = identity(CardId::Anger, 0),
                5 => spec.row = crate::content_tables::card_row(CardId::Anger, 0).unwrap(),
                6 => {
                    state.piles.get_mut(PileId::Play).make_mut()[0].atom =
                        catalog.atom(&identity(CardId::Anger, 0)).unwrap();
                }
                7 => {
                    state.piles.get_mut(PileId::Play).make_mut().clear();
                }
                8 => state.piles.get_mut(PileId::Hand).make_mut()[0].atom = u16::MAX,
                9 => state.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_LEGACY,
                10 => state.piles.get_mut(PileId::Hand).make_mut()[0].flags |= CARD_FLAG_LEGACY,
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 13 }];
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection,
                x_value,
                args: &args,
                events: &mut events,
            };
            assert!(stoke_foundation(&mut ctx).is_err(), "shape case {case}");
            assert_eq!(state, before, "shape case {case}");
            assert_eq!(events, before_events, "shape case {case}");
        }
    }

    #[test]
    fn heavenly_drill_doubles_hits_at_threshold() {
        let (mut state, catalog, atom) = fixture();
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: Some(0),
            selection: None,
            x_value: 4,
            args: &[CompiledArg::I(3), CompiledArg::I(4)],
            events: &mut events,
        };
        heavenly_drill_x(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 26);
    }

    #[test]
    fn pacts_end_reads_the_live_exhaust_count() {
        let (mut state, catalog, atom) = fixture();
        for uid in 1..=3 {
            state
                .piles
                .get_mut(PileId::Exhaust)
                .make_mut()
                .push(HotCard {
                    uid,
                    atom,
                    flags: 0,
                });
        }
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 4,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(18), CompiledArg::I(3)],
            events: &mut events,
        };
        pacts_end_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 32);
    }

    /// #3515: each body gates on its native command, not `history.over`.
    /// While the combat is ending before the over latch:
    /// - Crimson Mantle's `Apply<CrimsonMantlePower>` (`<Apply>d__1`1`
    ///   0x3ef988 IL_0025, `IsEnding`) writes neither amount;
    /// - Fiend Fire's and Stoke's per-card `CardCmd.Exhaust` (`<Exhaust>d__6`
    ///   0x3e06c8 IL_0025, `IsOverOrEnding`) exhausts nothing;
    /// - Primal Force's plural `CardCmd.Transform` (`<Transform>d__13`
    ///   0x3e0ae0 IL_0032, `IsEnding`) transforms nothing.
    ///
    /// The Adaptable-vetoed control writes, exhausts and transforms.
    #[test]
    fn ironclad_rare_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        let (template, catalog, source) = crimson_fixture(0, PileId::Play);
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Crimson Mantle", |s, _| {
            crimson_mantle(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(7)],
                events: &mut Vec::new(),
            })
        });

        let (mut template, catalog, source, strike, defend) = fiend_fixture(0);
        template
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend]);
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Fiend Fire", |s, t| {
            fiend_fire_exact(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(t),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(7)],
                events: &mut Vec::new(),
            })
        });

        let (mut template, catalog, source, strike, defend, _) = primal_fixture(0);
        template.piles.get_mut(PileId::Play).make_mut().push(source);
        template
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, defend]);
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Primal Force", |s, _| {
            primal_force_exact(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(0)],
                events: &mut Vec::new(),
            })
        });

        // Stoke still samples and generates after its walk; the gated write
        // is the Exhaust of the frozen Hand.
        for vetoed in [false, true] {
            let (mut state, catalog, source) = stoke_fixture(0, 2);
            state.monsters_mut().clear();
            crate::engine::damage::push_ending_window_roster(&mut state, vetoed);
            run_stoke_foundation(&mut state, &catalog, source, &mut Vec::new()).unwrap();
            assert_eq!(
                state.piles.get(PileId::Exhaust).len(),
                if vetoed { 2 } else { 0 },
                "Stoke vetoed={vetoed}"
            );
        }
    }
}
