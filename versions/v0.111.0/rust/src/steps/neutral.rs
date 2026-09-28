//! Card-step bodies for the `content/cards/neutral.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Current status: 34 of 37 ported, 3 escalated
//!
//! All 37 stubs were re-triaged after engine slice 5 (main @ 275951d7).
//! The original four are `draw_if_no_hand_attacks`,
//! `gain_current_block_card_unpowered_exact`, `rend_exact` — because their
//! primitives exist (the step-callable Draw, the card-block funnels, the
//! targeted attack command, and the boundary/admission refusals that make
//! each remaining Python gate vacuous, each verified at its engine site and
//! cited at the body) — plus `enlightenment_exact`, whose ordered local-cost
//! side table, cost reader and both cleanup sites now form its complete
//! physical-card transaction. The remaining escalations cluster around a few
//! subsystems, each named at its stub: richer resumable selections, neutral
//! generation-closure admission, frozen/live
//! AutoPlay batches, player or monster powers nothing engine-side reads,
//! result-location listeners, and individual gaps for potions,
//! generalized local-cost writers, and physical damage growth. Later engine units added
//! exact Beat Down and Eidolon AutoPlay bodies, then Unit E added Purity and
//! Seeker Strike's resumable body-owned selectors, followed by Abundance's
//! owner-derived Generation shuffle and exact-one upgraded Power choice.
//! #1563 adds Jackpot's owner-zero with-replacement generation transaction;
//! #1562 then adds the six complete neutral ally programs and their shared
//! typed-target/remote-Player machinery.
//! Purity's StepKind is exact and manifest-listed, while both card rows remain
//! independently refused by their native Retain keyword until that separate
//! lifecycle exists. Gang Up now consumes the already-projected multiplayer
//! roster and target-local non-owner damage-history counter.
//! R50 adds Toric Toughness's exact retained-Decimal powered gain and keyed
//! acquisition-ordered flat refresh lifecycle.

use super::StepCtx;
use crate::catalog::{CardIdentity, CardSpec, Catalog, CompiledArg, RewardPool};
use crate::content_tables::CardType;
use crate::engine::cards::{
    inject_generated_exact_bottom, inject_generated_free_this_combat_batch_draw_random,
    sample_generation_slice, shuffle_generation_slice, upgrade_live_cards_once,
};
use crate::engine::damage::player_attack_from_card;
use crate::engine::damage::{
    gain_card_unpowered_block, gain_powered_card_block, gain_powered_card_block_retained_decimal,
};
use crate::engine::draw::{CardDrawResult, DrawSource, draw_cards, draw_cards_for_card_result};
use crate::engine::{EngineRefusal, Subject, fire_hook};
use crate::hooks::HookEvent;
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_LEGACY, CARD_FLAG_PICK, DrawCaller, HotCard,
    HotMonster, HotState, LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, PileId,
    RngStream, RngStreamState,
};
use crate::ids::{CardId, MonsterKind, PowerId, StepKind};
use crate::powers::SlotWire;
use crate::rng::Xoshiro256StarStar;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::AbundanceExact,
    StepKind::AlchemizeExact,
    StepKind::BeaconOfHopeExact,
    StepKind::BeatDownExact,
    StepKind::BelieveInYouExact,
    StepKind::BrightestFlameExact,
    StepKind::CalamityExact,
    StepKind::CatastropheExact,
    StepKind::CoordinateExact,
    StepKind::DiscoveryExact,
    StepKind::DrawIfNoHandAttacks,
    StepKind::EidolonExact,
    StepKind::EnlightenmentExact,
    StepKind::GainCurrentBlockCardUnpoweredExact,
    StepKind::GangUpExact,
    StepKind::HiddenGemExact,
    StepKind::InterceptExact,
    StepKind::JackOfAllTradesExact,
    StepKind::JackpotExact,
    StepKind::LiftExact,
    StepKind::MadScienceChaosExact,
    StepKind::MimicExact,
    StepKind::MetamorphosisExact,
    StepKind::Nostalgia,
    StepKind::PanicButtonExact,
    StepKind::PurityExact,
    StepKind::RallyExact,
    StepKind::Rebound,
    StepKind::RendExact,
    StepKind::RestlessnessExact,
    StepKind::SeekerStrikeExact,
    StepKind::SplashExact,
    StepKind::StunTarget,
    StepKind::TheBallExact,
    StepKind::TheBombExact,
    StepKind::ToricToughnessExact,
    StepKind::UpgradeAllCombatExceptSourceExact,
];

fn exact_active_neutral_source(
    ctx: &StepCtx<'_>,
    id: CardId,
    site: &'static str,
) -> Result<(), EngineRefusal> {
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
        .expect("the unique count proved one live source");
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) || ctx.spec.identity.id != id {
        return Err(EngineRefusal::MalformedArgs(site));
    }
    Ok(())
}

/// This family's argument shapes, destructured once per body.
///
/// Since #1366 the gate holds no shape opinion on a wave kind: the body owns
/// its destructure and returns a typed refusal on surprise, which the
/// differential's `must_cover` makes visible before merge.
fn one_int(ctx: &StepCtx<'_>, site: &'static str) -> Result<i64, EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(value)] => Ok(*value),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn two_ints(ctx: &StepCtx<'_>, site: &'static str) -> Result<(i64, i64), EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(first), CompiledArg::I(second)] => Ok((*first, *second)),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

/// The refusal for a body that found its captured target absent or dead where
/// Python refuses rather than skips.
fn dead_target(index: usize) -> EngineRefusal {
    EngineRefusal::BadTarget(u8::try_from(index).unwrap_or(u8::MAX))
}

/// `abundance_exact` — exact owner-Power selection (#1363 Batch 4).
///
/// Native v0.111.0 `Abundance/<OnPlay>d__6::MoveNext` RVA 0x3888ec
/// filters owner cards to Powers, then `CardFactory.GetDistinctForCombat`
/// (0x112878) applies `FilterForCombat` (predicate 0x3d7042): no Basic,
/// Ancient or Event cards. Shuffle the remaining pool and upgrade its offers.
/// Frozen Python `_run_steps_inner` (deleted #2827) delegates to
/// `_begin_abundance_selection_exact`, whose modal lifecycle matches
/// but whose pool includes Ancient powers; the native filter above supersedes it.
///
/// Unit E extends the existing pending CardPlay carrier with the exact ordered
/// three-option modal. The owner pool is generated from native card facts,
/// and the complete Generation-stream shuffle is consumed before
/// suspension, and admission pre-interns every reachable offered identity.
/// Resolution singularly generates only the selected upgraded Power and marks
/// it free this turn before the ordinary Hand-cap insertion transaction.
///
/// No Entropy input (#3122): `0x3888ec` IL_0027-IL_0058 reads
/// `Owner.Character.CardPool`, `Owner.UnlockState` and the run's
/// `CardMultiplayerConstraint` into `GetUnlockedCards`, then the Power `Where`
/// (IL_007c) and `GetDistinctForCombat(.., 3, CombatCardGeneration)`
/// (IL_0097). So `entropy_card_pool` only refuses when it CONTRADICTS the
/// owner (the `owner_pool_generation_provenance_is_exact` rule); an absent one
/// no longer refuses a Defect, Silent or Necrobinder owner.
///
/// Recorded unlock profile (#3285, re-derived from the v0.111.0 IL, sts2.dll
/// SHA-256 `9cb4f1ad…12b4`): `0x3888ec` IL_0027-IL_0038 loads
/// `Owner.Character.CardPool`, IL_003d-IL_0043 `Owner.UnlockState`,
/// IL_0048-IL_0053 `Owner.RunState.CardMultiplayerConstraint`, and
/// IL_0058 calls `CardPoolModel::GetUnlockedCards` (RVA `0x7e54c`, whose
/// `FilterThroughEpochs` drops every row a missing epoch gates). IL_005d-IL_007c
/// is `Where(<OnPlay>b__6_0)`, and that predicate (RVA `0x3888de`) is
/// `get_Type; ldc.i4.3; ceq` — Power. IL_0081 pushes `3`, IL_0083-IL_0092 load
/// `Owner.RunState.Rng.CombatCardGeneration`, and IL_0097 calls
/// `CardFactory::GetDistinctForCombat` (RVA `0x112878`: `FilterForCombat`
/// drops Basic/Ancient/Event, then one shuffle). The IL_00a2-IL_00c4 foreach
/// upgrades every offer (`CardCmd::Upgrade` at IL_00b8). So the pool is the
/// owner's recorded-profile Power projection,
/// [`owner_type_generation_pool`]`(owner, epochs, Power)` through
/// `Catalog::owner_type_generation_pool` — White Noise's pool — and a partial
/// profile draws it rather than refusing. The frozen
/// `ABUNDANCE_POWER_POOLS_V1101` row is that projection at the fully-unlocked
/// profile (`one_type_owner_pools_reproduce_the_frozen_constants`).
pub(crate) fn begin_abundance_exact(ctx: &mut StepCtx<'_>) -> Result<Vec<HotCard>, EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Abundance
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || ctx
            .state
            .entropy_card_pool
            .is_some_and(|pool| Some(pool) != ctx.state.reward_card_pool)
        || !crate::engine::cards::unlock_profile_is_recorded(ctx.state, ctx.catalog)
        || ctx.state.rng.is_vacant(RngStream::Generation)
    {
        return Err(EngineRefusal::MalformedArgs("abundance_exact"));
    }
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs("abundance_exact owner"))?;
    let pool = ctx
        .catalog
        .owner_type_generation_pool(owner, crate::content_tables::CardType::Power);
    let shuffled = crate::engine::cards::shuffle_generation_slice(ctx.state, &pool)?;
    if shuffled.len() < 3 {
        return Err(EngineRefusal::MalformedArgs("abundance_exact pool"));
    }
    shuffled[..3]
        .iter()
        .map(|id| {
            let atom = ctx
                .catalog
                .atom(&CardIdentity {
                    id: *id,
                    upgrade: 1,
                    enchantment: None,
                })
                .ok_or(EngineRefusal::UnknownMintIdentity(CardIdentity {
                    id: *id,
                    upgrade: 1,
                    enchantment: None,
                }))?;
            Ok(HotCard {
                uid: 0,
                atom,
                flags: 0,
            })
        })
        .collect()
}

pub(crate) fn abundance_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = begin_abundance_exact(ctx)?;
    Err(EngineRefusal::ContinuationNotModeled)
}

/// `alchemize_exact` — one exact combat-potion generation/procurement body.
///
/// Current v0.111.0 authority: `Alchemize` constructor RVA `0xd7970`,
/// `OnUpgrade` RVA `0xd79cb`, `CanBeGeneratedInCombat=false` RVA `0xd797d`,
/// `OnPlay` outer RVA `0xd7988`, and `<OnPlay>d__5::MoveNext` RVA `0x3897c8`.
/// Python dispatch: `_run_steps_inner` (frozen Python, deleted #2827).
/// Python body: `_apply_alchemize_exact`.
/// Every live body asks `PotionFactory.CreateRandomPotionInCombat` for one
/// independently generated potion, then awaits `PotionCmd.TryToProcure(-1)`.
/// The factory consumes its rarity roll and pool-index roll even when Sozu or
/// a full belt makes the later procurement fail; a terminal body consumes
/// neither.
pub(crate) fn alchemize_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    exact_active_neutral_source(ctx, CardId::Alchemize, "alchemize_exact source")?;
    if !ctx.args.is_empty()
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("alchemize_exact"));
    }
    crate::engine::potions::alchemize_one_attempt(ctx.state, ctx.catalog, ctx.events)
}

/// `beacon_of_hope_exact` — exact unique BeaconOfHopePower registration.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — a StackType-2 Unique registration:
/// first application sets the `beacon_of_hope` scalar to one and appends it
/// to `after_block_gained_power_order`; a replay changes nothing.
///
/// The admitted engine is strictly solo, so the listener preserves native
/// acquisition order and owner-side gating while its teammate enumeration is
/// exactly empty.
pub(crate) fn beacon_of_hope_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "beacon_of_hope_exact")?;
    if ctx.spec.identity.id != CardId::BeaconOfHope
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || amount != 1
    {
        return Err(EngineRefusal::MalformedArgs("beacon_of_hope_exact"));
    }
    if ctx.state.history.over || ctx.state.powers.value(PowerId::BeaconOfHope) > 0 {
        return Ok(());
    }
    if !ctx
        .state
        .fanouts
        .register_after_block_gained(PowerId::BeaconOfHope)
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "after-block-gained power order",
        ));
    }
    ctx.state
        .powers
        .set(PowerId::BeaconOfHope, SlotWire::Int, 1);
    crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::BeaconOfHope, 1);
    Ok(())
}

/// `beat_down_exact` — shuffle and AutoPlay a frozen Discard attack prefix.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — behind the B142 exact-body allowlist,
/// `_beat_down_frozen_autoplay_batch` collects a frozen candidate
/// batch and AutoPlays each member as a nested CardPlay.
///
/// The synchronous work stack re-resolves each physical uid from Discard and
/// stops later children after a terminal play.
pub(crate) fn beat_down_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count: usize = one_int(ctx, "beat_down_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("beat_down_exact"))?;
    if !matches!(ctx.spec.identity.id, CardId::BeatDown)
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || count != usize::from(3 + ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("beat_down_exact"));
    }
    crate::engine::play::queue_beat_down_batch(ctx, count)
}

/// `believe_in_you_exact` — exact typed-AnyAlly Energy routing (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — resolve the AnyAlly frame choice via
/// `_resolve_ally_target_key`, then `_gain_energy_to_player` grants the chosen player the printed energy.
///
/// The selected Player key is resolved before the clone-rehearsed resource
/// write; the remote `NoEnergyGain` and native resource cap stay distinct
/// from the owner's narrower hot Energy representation.
pub(crate) fn believe_in_you_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "believe_in_you_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("believe_in_you_exact"))?;
    if ctx.spec.identity.id != CardId::BelieveInYou
        || amount != 2 + i32::from(ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("believe_in_you_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    let mut probe = ctx.state.clone();
    crate::engine::allies::gain_energy(&mut probe, key, amount)?;
    crate::engine::allies::gain_energy(ctx.state, key, amount)
}

/// `brightest_flame_exact` — Brightest Flame's complete three-command body.
///
/// Current v0.111.0 IL (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// constructor **0xd9e67**, canonical vars **0xd9e74**, upgrade **0xd9efb**,
/// and `BrightestFlame/<OnPlay>d__5::MoveNext` **0x38f4b8**. The body
/// separately awaits owner Energy, command Draw, then
/// `CreatureCmd::LoseMaxHp(..., isFromCard=true)` in that order.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) and `lose_player_max_hp` preserve the same order. Draw owns its per-card ending and
/// hand-cap checks, but LoseMaxHp has no enclosing ending guard: even a draw
/// listener that ends combat cannot suppress the cap loss. The nested props-14
/// damage has a null CardModel source, so represented Rupture is immediate
/// rather than added to the active play's batch.
pub(crate) fn brightest_flame_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (energy, draw, max_hp): (i16, usize, i32) =
        match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
            (
                CardId::BrightestFlame,
                0,
                [CompiledArg::I(2), CompiledArg::I(2), CompiledArg::I(2)],
            ) => (2, 2, 2),
            (
                CardId::BrightestFlame,
                1,
                [CompiledArg::I(3), CompiledArg::I(3), CompiledArg::I(2)],
            ) => (3, 3, 2),
            _ => return Err(EngineRefusal::MalformedArgs("brightest_flame_exact")),
        };
    if ctx.target.is_some() || ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("brightest_flame_exact action"));
    }
    if crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
        != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("brightest_flame_exact source"));
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
    let (source_pile, source_index) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if source_pile == PileId::Exhaust {
        return Err(EngineRefusal::MalformedArgs(
            "brightest_flame_exact active source",
        ));
    }
    let source = &ctx.state.piles.get(source_pile).as_slice()[source_index];
    if ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "brightest_flame_exact active source",
        ));
    }

    fn apply(
        ctx: &mut StepCtx<'_>,
        energy: i16,
        draw: usize,
        max_hp: i32,
    ) -> Result<(), EngineRefusal> {
        ctx.state.energy = ctx
            .state
            .energy
            .checked_add(energy)
            .ok_or(EngineRefusal::CounterOverflow("brightest flame energy"))?;
        draw_cards(
            ctx.state,
            ctx.catalog,
            draw,
            DrawSource::Command,
            ctx.events,
        )?;
        crate::engine::damage::lose_player_max_hp_from_card(
            ctx.state,
            Some(ctx.catalog),
            max_hp,
            ctx.events,
        )
    }

    // A late max-HP/listener refusal must not leave the earlier Energy or Draw
    // commands visible. The live execution is deterministic from this exact
    // clone, including COW piles, RNG, events and terminal routing.
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
    apply(&mut probe_ctx, energy, draw, max_hp)?;
    apply(ctx, energy, draw, max_hp)
}

/// Which rarity comparison a generator's pool projection uses (#2542).
///
/// The game spells the filter two different ways and they are not the same
/// set. `CardFactory::FilterForCombat` `0x11634a` excludes Basic, Ancient and
/// Event; the `rarity >= Common && rarity <= Rare` comparisons in
/// `GetForCombat` `0x1162e8` and the generation-potion partitions also exclude
/// every Status, Token, Curse and Quest rarity. Abundance v0.111.0
/// `0x3888ec` reaches FilterForCombat through GetDistinctForCombat (0x112878).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PoolRarity {
    /// `Common | Uncommon | Rare`.
    CommonUncommonRare,
    /// Everything except `Basic | Ancient | Event`.
    NotBasicAncientEvent,
    /// Frozen Python pool oracle only. Native Call of the Void also applies
    /// FilterForCombat and therefore excludes Event.
    #[cfg(test)]
    NotBasicAncient,
    /// `Common` alone — Hello World's `HelloWorldPower` projection.
    Common,
    /// No rarity filter, retained only for advisory frozen-Python pool pins.
    /// Production Abundance uses the native FilterForCombat predicate.
    #[allow(dead_code)]
    Any,
}

impl PoolRarity {
    fn admits(self, rarity: crate::content_tables::CardRarity) -> bool {
        use crate::content_tables::CardRarity as R;
        match self {
            Self::CommonUncommonRare => matches!(rarity, R::Common | R::Uncommon | R::Rare),
            Self::NotBasicAncientEvent => !matches!(rarity, R::Basic | R::Ancient | R::Event),
            #[cfg(test)]
            Self::NotBasicAncient => !matches!(rarity, R::Basic | R::Ancient),
            Self::Common => matches!(rarity, R::Common),
            Self::Any => true,
        }
    }
}

/// This owner's slot in `CHARACTER_CARD_POOL_ROWS_V1101`.
///
/// The generated table is keyed in `ModelDb.AllCharacters` order, which
/// codegen refuses to emit if it drifts; the `debug_assert` re-proves the
/// mapping rather than trusting the index.
fn character_pool_index(owner: RewardPool) -> usize {
    let index = match owner {
        RewardPool::Ironclad => 0,
        RewardPool::Silent => 1,
        RewardPool::Regent => 2,
        RewardPool::Necrobinder => 3,
        RewardPool::Defect => 4,
    };
    debug_assert_eq!(
        crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101[index].0,
        match owner {
            RewardPool::Ironclad => "IRONCLAD",
            RewardPool::Silent => "SILENT",
            RewardPool::Regent => "REGENT",
            RewardPool::Necrobinder => "NECROBINDER",
            RewardPool::Defect => "DEFECT",
        }
    );
    index
}

/// One generator's exact ordered pool for `owner` under a recorded profile.
///
/// Port of Python `_derive_character_generation_pool` (#2542). Before that
/// slice every generator but Splash reached for a frozen per-generator
/// constant, which existed for Ironclad and Regent only — so Necrobinder,
/// Defect and Silent fights refused with `requires an explicit <Character>
/// owner` (79 of the 177 entry refusals in #2528's disposition).
///
/// The source is `<Character>CardPool::GenerateAllCards` (Ironclad `0xf1c60`,
/// Silent `0xf29b8`, Regent `0xf25a0`, Necrobinder `0xf2128`, Defect
/// `0xf1604`), whose literal `ModelDb.Card<T>()` array order is
/// RNG-significant and is preserved here. Each row's `unlock_epoch` is the
/// epoch whose `Cards` list `FilterThroughEpochs` (`0xf1f98` / `0xf2cf8` /
/// `0xf28e0` / `0xf2468` / `0xf1944`) removes when the profile does not carry
/// it; the two profile-independent predicates
/// (`CardFactory::FilterForPlayerCount` `0x115f1c` and `FilterForCombat`
/// `0x11634a`) are already applied in the generated table.
///
/// `epochs` is the normalized ascending profile the boundary admits, so
/// membership is a binary search. Consumes no RNG.
pub(crate) fn derive_character_generation_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
    rarity: PoolRarity,
    card_type: Option<crate::content_tables::CardType>,
    zero_cost: bool,
) -> Vec<CardId> {
    let rows = crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101[character_pool_index(owner)].1;
    let mut pool: Vec<CardId> = Vec::with_capacity(rows.len());
    for (id, unlock_epoch) in rows.iter() {
        if let (Some(epoch), Some(profile)) = (unlock_epoch, epochs)
            && profile.binary_search(epoch).is_err()
        {
            continue;
        }
        let Some(row) = crate::content_tables::card_row(*id, 0) else {
            continue;
        };
        if !rarity.admits(row.rarity) {
            continue;
        }
        if let Some(wanted) = card_type
            && row.card_type != wanted
        {
            continue;
        }
        if zero_cost && (row.cost != 0 || row.x_cost) {
            continue;
        }
        pool.push(*id);
    }
    pool
}

/// The exact ordered Colorless pool a fight's recorded profile builds.
///
/// Port of Python `_derive_colorless_generation_pool` (#2512), and the twin
/// of [`derive_character_generation_pool`] for the one pool every character
/// shares. `ColorlessCardPool::GenerateAllCards` `0xf11a0` is the literal
/// 65-row array; `CardPoolModel::GetUnlockedCards` `0x7e54c` runs
/// `FilterThroughEpochs` `0xf13f4` first and then, for a solo game
/// (`playerCount == 1`), `RemoveAll(c => c.MultiplayerConstraint == 2)`.
/// Both are order-preserving `List<T>` removals over independent per-row
/// predicates, so one pass in native order is the same ordered list the game
/// holds. `CanBeGeneratedInCombat` is already applied in the generated table
/// because `GetDistinctForCombat` `0x2be804` drops those three rows on every
/// path.
///
/// `multiplayer` keeps the `MultiplayerOnly` rows, which is what Largesse
/// reads; `exclude_self` is Jack of All Trades' own runtime-type exclusion,
/// which Largesse deliberately does not make. Consumes no RNG.
pub(crate) fn derive_colorless_generation_pool(
    epochs: Option<&[&'static str]>,
    multiplayer: bool,
    exclude_self: Option<CardId>,
) -> Vec<CardId> {
    let rows = &crate::content_tables::COLORLESS_CARD_POOL_ROWS_V1101;
    let mut pool: Vec<CardId> = Vec::with_capacity(rows.len());
    for (id, unlock_epoch, solo) in rows.iter() {
        if let (Some(epoch), Some(profile)) = (unlock_epoch, epochs)
            && profile.binary_search(epoch).is_err()
        {
            continue;
        }
        if !multiplayer && !*solo {
            continue;
        }
        if exclude_self == Some(*id) {
            continue;
        }
        pool.push(*id);
    }
    pool
}

/// The Colorless candidate list an Entropy transform of a Colorless-routed
/// origin draws under a recorded unlock profile (#3122).
///
/// `CardFactory::GetDefaultTransformationOptions` RVA `0x112960`: the
/// Colorless arm loads `ModelDb.CardPool<ColorlessCardPool>()` at IL_003a and
/// branches to IL_0047, the SAME `stloc.1` the `original.Pool` arm reaches, so
/// both arms go through `pool.GetUnlockedCards(original.Owner.UnlockState,
/// RunState.CardMultiplayerConstraint)` at IL_0048-IL_005f.
/// `CardPoolModel::GetUnlockedCards` RVA `0x7e54c` calls the pool's
/// `FilterThroughEpochs` at IL_0014 — for Colorless the override
/// `ColorlessCardPool::FilterThroughEpochs` RVA `0xf13f4`, whose five
/// `IsEpochRevealed` tests (IL_0014, IL_0042, IL_0070, IL_009e, IL_00cc) each
/// `RemoveAll` one `COLORLESS<n>_EPOCH` row set — then removes
/// `MultiplayerConstraint == 2` rows for a solo run (IL_0020-IL_004e,
/// predicate `0x31cec6`). `GetFilteredTransformationOptions` RVA `0x112a30`
/// then keeps `Rarity ∈ {Common, Uncommon, Rare}` (IL_0039-IL_005e, predicate
/// `0x3d706c`; no Colorless-routed origin is Status/Curse rarity, and the
/// caller refuses that arm by name) and `CanBeGeneratedInCombat`
/// (IL_005f-IL_0087, already applied to `COLORLESS_CARD_POOL_ROWS_V1101`).
/// Every stage is an order-preserving `RemoveAll`/`Where`, so this is
/// [`derive_colorless_generation_pool`] projected to `Common..Rare`.
///
/// At the fully-unlocked profile (`None`) this is exactly
/// `ENTROPY_COLORLESS_TRANSFORM_POOL_V109`
/// (`entropy_frozen_pools_are_the_generated_pool_projections`). Consumes no
/// RNG.
pub(crate) fn entropy_colorless_transform_pool(epochs: Option<&[&'static str]>) -> Vec<CardId> {
    use crate::content_tables::CardRarity as R;
    derive_colorless_generation_pool(epochs, false, None)
        .into_iter()
        .filter(|id| {
            crate::content_tables::card_row(*id, 0)
                .is_some_and(|row| matches!(row.rarity, R::Common | R::Uncommon | R::Rare))
        })
        .collect()
}

/// The solo Colorless generation pool under a recorded unlock profile: what
/// Quasar, Spectrum Shift, Bundle of Joy and Manifest Authority draw (the
/// Colorless half of #2560).
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`). Each body loads
/// `ModelDb::CardPool<ColorlessCardPool>` (MethodSpec `0x2b0007c2`, generic
/// argument TypeDef `ColorlessCardPool`), then `Owner.UnlockState` and the
/// run's `CardMultiplayerConstraint` into `CardPoolModel::GetUnlockedCards`
/// (RVA `0x7e54c`, `FilterThroughEpochs` at IL_0014 — the Colorless override
/// `0xf13f4`), and hands the result STRAIGHT to
/// `CardFactory::GetDistinctForCombat(.., CombatCardGeneration)` (RVA
/// `0x112878`: `FilterForPlayerCount`, `FilterForCombat`, one shuffle) with
/// no `Where` of its own:
///
/// * `Quasar/<OnPlay>d__3::MoveNext` RVA `0x3b4e48`: pool IL_002d,
///   `GetUnlockedCards` IL_004d, `GetDistinctForCombat(.., 3, ..)` IL_0068;
/// * `SpectrumShiftPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x345978`:
///   pool IL_003e, `GetUnlockedCards` IL_005e, `GetDistinctForCombat(..,
///   Amount, ..)` IL_007e;
/// * `BundleOfJoy/<OnPlay>d__5::MoveNext` RVA `0x390188`: pool IL_0023,
///   `GetUnlockedCards` IL_003e, `GetDistinctForCombat(.., Cards, ..)`
///   IL_0068;
/// * `ManifestAuthority/<OnPlay>d__7::MoveNext` RVA `0x3ab860`: `GainBlock`
///   IL_0041 first, then pool IL_00a2, `GetUnlockedCards` IL_00c2,
///   `GetDistinctForCombat(.., 1, ..)` IL_00dd.
///
/// None of the four reads the owner's character, so this is
/// [`derive_colorless_generation_pool`] with no rarity, type or self filter.
/// At the fully-unlocked profile (`None`) it is exactly
/// `REGENT_COLORLESS_GENERATION_POOL_V1101`
/// (`colorless_generator_pools_reproduce_the_frozen_constants`). Consumes no
/// RNG.
pub(crate) fn colorless_generation_pool(epochs: Option<&[&'static str]>) -> Vec<CardId> {
    derive_colorless_generation_pool(epochs, false, None)
}

/// Largesse's Colorless pool for a target Player whose recorded unlock
/// profile is `epochs` (#3285).
///
/// `Largesse/<OnPlay>d__3::MoveNext` RVA `0x3a90e4` (v0.111.0, sts2.dll
/// SHA-256 `9cb4f1ad…12b4`) loads `cardPlay.Target.Player` (IL_00b3-IL_00be),
/// `ModelDb::CardPool<ColorlessCardPool>` (IL_00c3), that same TARGET
/// Player's `UnlockState` (IL_00c8-IL_00d8) and its RunState's
/// `CardMultiplayerConstraint` (IL_00dd-IL_00f2) into
/// `CardPoolModel::GetUnlockedCards` (IL_00f7; RVA `0x7e54c`, the Colorless
/// `FilterThroughEpochs` override `0xf13f4`), then pushes `1` (IL_00fc) and
/// the SOURCE owner's `RunState.Rng.CombatCardGeneration` (IL_00fd-IL_010d)
/// into `CardFactory::GetDistinctForCombat` (IL_0112, RVA `0x112878`) and
/// takes `FirstOrDefault` (IL_0117). No `Where` of its own. Largesse is
/// MultiplayerOnly, so the run is a party run and `GetUnlockedCards` keeps
/// the `MultiplayerConstraint == 2` rows: this is
/// [`derive_colorless_generation_pool`] with `multiplayer = true` and no self
/// exclusion. At the fully-unlocked profile (`None`) it is exactly
/// `LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101`
/// (`largesse_colorless_pool_reproduces_the_frozen_constant`). Consumes no
/// RNG.
pub(crate) fn largesse_colorless_pool(epochs: Option<&[&'static str]>) -> Vec<CardId> {
    derive_colorless_generation_pool(epochs, true, None)
}

/// Jack of All Trades' Colorless pool under a recorded unlock profile
/// (#2560).
///
/// `JackOfAllTrades/<OnPlay>d__6::MoveNext` RVA `0x3a7ecc` reads the same
/// `CardPool<ColorlessCardPool>` (IL_0026) through
/// `GetUnlockedCards(Owner.UnlockState, CardMultiplayerConstraint)` (IL_0046),
/// then `Where(<>c::<OnPlay>b__6_0)` (IL_006a; the predicate RVA `0x3a7ebe` is
/// `!(card is JackOfAllTrades)`, IL_0002-IL_000b) before
/// `GetDistinctForCombat(.., Cards, CombatCardGeneration)` (IL_0094). So it is
/// [`colorless_generation_pool`] minus Jack itself, in native order. At the
/// fully-unlocked profile it is exactly `JACK_OF_ALL_TRADES_POOL_V1091`
/// (`colorless_generator_pools_reproduce_the_frozen_constants`). Consumes no
/// RNG.
pub(crate) fn jack_of_all_trades_pool(epochs: Option<&[&'static str]>) -> Vec<CardId> {
    derive_colorless_generation_pool(epochs, false, Some(CardId::JackOfAllTrades))
}

/// Jackpot's canonical-zero-cost projection of the owner pool.
///
/// `JACKPOT_ZERO_COST_POOL_V109` and `REGENT_ZERO_COST_GENERATION_POOL_V1101`
/// are this derivation at the fully-unlocked profile. X-cost cards are
/// excluded because their canonical cost is not the number the native
/// `CanonicalEnergyCost == 0` comparison reads.
pub(crate) fn owner_zero_cost_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
) -> Vec<CardId> {
    derive_character_generation_pool(owner, epochs, PoolRarity::CommonUncommonRare, None, true)
}

/// The whole-pool projection every `Common..Rare` owner generator shares.
///
/// `STOKE_CARD_POOL_V109` and `REGENT_CARD_GENERATION_POOL_V1101` are exactly
/// this for their two owners; the pin lives in
/// `character_pools_reproduce_the_frozen_constants`.
pub(crate) fn owner_generation_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
) -> Vec<CardId> {
    derive_character_generation_pool(owner, epochs, PoolRarity::CommonUncommonRare, None, false)
}

/// Distraction's and White Noise's one-type projection of the owner pool
/// (#2560/#2946).
///
/// `Distraction/<OnPlay>d__5::MoveNext` RVA `0x399690` and
/// `WhiteNoise/<OnPlay>d__3::MoveNext` RVA `0x3c74ac` both read
/// `Owner.Character.CardPool` through `GetUnlockedCards(Owner.UnlockState, ..)`
/// (Distraction IL_002c-IL_0051; White Noise IL_00aa-IL_00cf), keep one
/// `CardType` (predicates `<OnPlay>b__5_0` RVA `0x399682`: `get_Type; ldc.i4.2`
/// = Skill; `<OnPlay>b__3_0` RVA `0x3c749e`: `get_Type; ldc.i4.3` = Power) and
/// call `GetDistinctForCombat(.., 1, CombatCardGeneration)` (Distraction
/// IL_0090, White Noise IL_010e), whose `FilterForCombat` drops Basic, Ancient
/// and Event rarities. `DISTRACTION_SKILL_POOL_V1101` and every
/// `ABUNDANCE_POWER_POOLS_V1101` row are this derivation at the fully-unlocked
/// profile (`one_type_owner_pools_reproduce_the_frozen_constants`).
pub(crate) fn owner_type_generation_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
    card_type: crate::content_tables::CardType,
) -> Vec<CardId> {
    derive_character_generation_pool(
        owner,
        epochs,
        PoolRarity::NotBasicAncientEvent,
        Some(card_type),
        false,
    )
}

/// Crossbow's owner Attack pool under a recorded unlock profile (#2970).
///
/// `Crossbow/<AfterSideTurnStart>d__2::MoveNext` (v0.111.0 RVA `0x322340`)
/// reads `Owner.Character.CardPool` (IL_003e-IL_0048) through
/// `GetUnlockedCards(Owner.UnlockState, RunState.CardMultiplayerConstraint)`
/// (IL_004d-IL_0068), keeps `Type == Attack` (IL_006d-IL_008c; the predicate
/// `<>c::<AfterSideTurnStart>b__2_0` RVA `0x322332` is `get_Type; ldc.i4.1;
/// ceq`), and hands that list to `CardFactory::GetDistinctForCombat(Owner,
/// list, 1, CombatCardGeneration)` (IL_00aa-IL_00c7), whose `FilterForCombat`
/// drops Basic, Ancient and Event rarities before its one shuffle. That is
/// exactly Distraction's and White Noise's one-type projection with the
/// Attack type ([`owner_type_generation_pool`]). At the fully-unlocked
/// Ironclad profile it is `INFERNAL_BLADE_ATTACK_POOL_V109`, the frozen table
/// the body drew before (`crossbow_owner_pools_reproduce_the_frozen_table`).
/// Consumes no RNG.
pub(crate) fn crossbow_owner_attack_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
) -> Vec<CardId> {
    owner_type_generation_pool(owner, epochs, crate::content_tables::CardType::Attack)
}

/// Native owner-pool listeners: CreativeAiPower.BeforeHandDraw 0x338180
/// filters Power; HelloWorldPower 0x33bfd0 filters Common; CallOfTheVoidPower
/// 0x336b08 / predicate 0x336ae0 excludes Basic (1) and Ancient (5).
/// GetDistinctForCombat (0x112878) also excludes Event via FilterForCombat
/// (0x112932 / predicate 0x3d7042). It applies the shared predicate after
/// the owner CharacterCardPool's ordered unlock/player-count filtering.
/// Authority: v111 DLL SHA256
/// 9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4.
pub(crate) fn owner_listener_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
    power: PowerId,
) -> Vec<CardId> {
    let (rarity, card_type) = match power {
        PowerId::CreativeAi => (
            PoolRarity::NotBasicAncientEvent,
            Some(crate::content_tables::CardType::Power),
        ),
        PowerId::HelloWorld => (PoolRarity::Common, None),
        PowerId::CallOfTheVoid => (PoolRarity::NotBasicAncientEvent, None),
        _ => return Vec::new(),
    };
    derive_character_generation_pool(owner, epochs, rarity, card_type, false)
}

/// Exact owner-Attack pool for public Calamity.
///
/// `Calamity` filters the owner's pool to Attacks after the ordinary
/// `Common..Rare` comparison — `INFERNAL_BLADE_ATTACK_POOL_V109` is exactly
/// that for Ironclad. Since #2542 every owner has one, derived from the
/// recorded profile instead of refused.
pub(crate) fn calamity_owner_attack_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
) -> Vec<CardId> {
    derive_character_generation_pool(
        owner,
        epochs,
        PoolRarity::CommonUncommonRare,
        Some(crate::content_tables::CardType::Attack),
        false,
    )
}

/// Largest Calamity list this exact synchronous action quotient materializes.
pub(crate) const MAX_CALAMITY_GENERATED_PER_PLAY: i32 = 256;

/// `calamity_exact` — acquire one exact CalamityPower stack.
///
/// Current v0.111.0 native authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Calamity/<OnPlay>d__4::MoveNext` RVA `0x390b94` applies Decimal.One to the
/// owner. `CalamityPower` constructor RVA `0xa01a0`, Type RVA `0xa01af`, and
/// StackType RVA `0xa01b2` pin an additive Type-1 integer scalar. Its exact
/// per-card dictionary and fresh-Amount generation reader live at the two
/// CardPlay seams in `engine::play`.
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one additive stack on the owner's
/// CalamityPower scalar, whose `AfterCardPlayed` listener
/// (`resolve_calamity_generation`) mints its current Amount from the
/// owner "attack" Generation pool on every later play. The exact
/// `_run_steps_inner` writer body; the awaited
/// `resolve_calamity_generation` reader.
pub(crate) fn calamity_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Calamity
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !ctx.args.is_empty()
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("calamity_exact"));
    }
    exact_active_neutral_source(ctx, CardId::Calamity, "calamity_exact source")?;
    if ctx.state.history.over {
        return Ok(());
    }
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs("Calamity generation owner"))?;
    let pool = ctx.catalog.calamity_attack_pool(owner);
    if !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog)
        || pool.iter().copied().any(|id| {
            ctx.catalog
                .atom(&CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_none()
        })
        || !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Calamity generation provenance",
        ));
    }
    let current = match ctx.state.powers.get(PowerId::Calamity) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value >= 0 => slot.value,
        Some(_) => return Err(EngineRefusal::MalformedArgs("Calamity amount")),
    };
    if ctx.state.calamity_hook_is_live() != (current > 0) {
        return Err(EngineRefusal::MalformedArgs(
            "Calamity power/listener mismatch",
        ));
    }
    if current >= MAX_CALAMITY_GENERATED_PER_PLAY {
        return Err(EngineRefusal::MalformedArgs(
            "Calamity generation batch exceeds 256 cards",
        ));
    }
    let updated = current
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("calamity_exact"))?;
    crate::engine::play::prepare_after_card_played_scalar_write(
        ctx.state,
        PowerId::Calamity,
        current,
        updated,
    )?;
    ctx.state
        .powers
        .set(PowerId::Calamity, SlotWire::Int, updated);
    ctx.state.set_calamity_hook_live(true);
    crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::Calamity, updated);
    Ok(())
}

/// `catastrophe_exact` — bounded live-source direct-AutoPlay loop.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — behind the B141 exact-body allowlist,
/// `_catastrophe_live_autoplay_loop` re-reads the live outer
/// physical source between nested AutoPlay iterations.
///
/// The bounded synchronous loop re-resolves the exact active source before
/// every iteration, so a prior child upgrade can raise the live count from two
/// to three. Each preferred/fallback shuffle and direct child keeps native
/// ordering; a selecting child remains a typed refusal rather than inventing
/// a persisted FrozenAutoBatch frame.
pub(crate) fn catastrophe_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count: usize = one_int(ctx, "catastrophe_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("catastrophe_exact"))?;
    crate::engine::play::catastrophe_live_auto_plays(ctx, count)
}

/// `coordinate_exact` — exact typed-AnyAlly temporary Strength (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — resolve the AnyAlly frame choice, then
/// `_apply_temporary_stat_to_player` grants the chosen player
/// temporary Strength.
///
/// The target-keyed wrapper expires in native Player order: local first, then
/// the represented teammate after local owner/relic callbacks. A lethal local
/// Doom therefore preserves the teammate's terminal wrapper snapshot.
pub(crate) fn coordinate_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "coordinate_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("coordinate_exact"))?;
    if ctx.spec.identity.id != CardId::Coordinate
        || amount != 5 + 3 * i32::from(ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("coordinate_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    crate::engine::allies::gain_temp_strength(ctx.state, key, amount, ctx.events)
}

/// `discovery_exact` — exact owner-card 1-of-3 generation.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Discovery/<OnPlay>d__4::MoveNext` RVA `0x399254` reads the physical
/// source owner's unlocked `CharacterModel.CardPool`, calls
/// `CardFactory.GetDistinctForCombat(..., 3, CombatCardGeneration)`, awaits a
/// skippable `FromChooseACardScreen`, applies `SetToFreeThisTurn` only to a
/// non-null answer, then awaits one `AddGeneratedCardToCombat(Hand, Bottom)`.
///
/// Python `_begin_discovery_selection_exact` (frozen, deleted #2827) and
/// `_apply_discovery_selection_exact` preserve the same full-shuffle
/// order, physical-source continuation, four-choice card-or-Skip surface,
/// chosen-only free modifier, generated history, Hand-cap redirect, terminal
/// gate, and clone-preflight transaction. Issue #1793's oracle correction is
/// merged, so Python and Rust both expose the native skippable screen.
pub(crate) fn begin_discovery_exact(ctx: &mut StepCtx<'_>) -> Result<Vec<HotCard>, EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Discovery
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("discovery_exact"));
    }
    exact_active_neutral_source(ctx, CardId::Discovery, "discovery_exact source")?;
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs("discovery_exact owner"))?;
    if owner == RewardPool::Regent && !ctx.state.spectrum_shift_generation_pool() {
        return Err(EngineRefusal::MalformedArgs(
            "discovery_exact Regent pool provenance",
        ));
    }
    let pool = ctx.catalog.owner_generation_pool(owner);
    let shuffled = shuffle_generation_slice(ctx.state, &pool)?;
    if shuffled.len() < 3 {
        return Err(EngineRefusal::MalformedArgs("discovery_exact pool"));
    }
    shuffled[..3]
        .iter()
        .map(|id| {
            let identity = CardIdentity {
                id: *id,
                upgrade: 0,
                enchantment: None,
            };
            let atom = ctx
                .catalog
                .atom(&identity)
                .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
            Ok(HotCard {
                uid: 0,
                atom,
                flags: 0,
            })
        })
        .collect()
}

pub(crate) fn discovery_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = begin_discovery_exact(ctx)?;
    Err(EngineRefusal::ContinuationNotModeled)
}

/// `("draw_if_no_hand_attacks", count)` — Impatience.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) (the IMPATIENCE arm of the shared
/// branch). The body reads the live post-removal Hand — the playing card has
/// already left it (play.rs step 1, exactly as in Python) — and issues one
/// Draw command only when no hand card's spec is an Attack. That one command
/// keeps a single entry gate and re-checks ending and hand space per card,
/// which is [`draw_cards`]'s own loop.
pub(crate) fn draw_if_no_hand_attacks(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = one_int(ctx, "draw_if_no_hand_attacks")?;
    let count: usize = count
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("draw_if_no_hand_attacks count"))?;
    let any_attack = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .any(|card| {
            ctx.catalog
                .spec(card.atom)
                .is_some_and(|spec| spec.is_attack)
        });
    if any_attack {
        return Ok(());
    }
    crate::engine::play::draw_cardplay_no_result(ctx, count)
}

/// `eidolon_exact` — AutoPlay the frozen eligible Exhaust batch.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — behind the B262 exact-body allowlist,
/// `_eidolon_frozen_autoplay_batch` AutoPlays a frozen batch as
/// nested CardPlays.
///
/// Eligible rows exhaust naturally and are forced back to Exhaust after each
/// synchronous nested play.
pub(crate) fn eidolon_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || !matches!(ctx.spec.identity.id, CardId::Eidolon)
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
    {
        return Err(EngineRefusal::MalformedArgs("eidolon_exact"));
    }
    crate::engine::play::queue_eidolon_batch(ctx)
}

/// `("enlightenment_exact", 1, expiration, true)` — Enlightenment.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — validate the complete W183 source row,
/// then synchronously append one reduce-only Set-to-1 local-cost row to every
/// card in the live Hand, preserving hand/UID order and forcing exact-pile
/// projection. L0's row expires this turn or when played; L1's lasts for the
/// combat. Replays append another row rather than folding the sequence.
///
/// The source is already in Play for every admitted manual play, so it is not
/// among the candidates. Native direct AutoPlay can leave the source in Hand,
/// but all nested-AutoPlay sources remain refused at admission; if that engine
/// primitive lands later, this live-Hand walk already includes it.
pub(crate) fn enlightenment_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected_expiration = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Enlightenment, 0) => LocalCostExpiration::ThisTurnOrPlayed,
        (CardId::Enlightenment, 1) => LocalCostExpiration::ThisCombat,
        _ => return Err(EngineRefusal::MalformedArgs("enlightenment_exact")),
    };
    match ctx.args {
        [
            CompiledArg::I(1),
            CompiledArg::I(expiration),
            CompiledArg::B(true),
        ] if LocalCostExpiration::from_wire(*expiration) == Some(expected_expiration) => {}
        _ => return Err(EngineRefusal::MalformedArgs("enlightenment_exact")),
    }

    let (piles, card_states) = (&mut ctx.state.piles, &mut ctx.state.card_states);
    for card in piles.get_mut(PileId::Hand).make_mut() {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        card_states.append_local_cost_modifier(
            card.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 1,
                expiration: expected_expiration,
                reduce_only: true,
            },
        );
    }
    ctx.state.exact_piles = true;
    Ok(())
}

/// `("gain_current_block_card_unpowered_exact",)` — Entrench.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_gain_card_unpowered_block`
/// with the player's current Block as the raw operand, i.e. one
/// card-or-monster-move GainBlock command that doubles Block.
///
/// The unpowered command differs from the powered one in which modifiers key
/// on it: it skips the now-live Dexterity, temporary Dexterity and Fasten
/// terms while retaining Unmovable's card-gain window. The dedicated
/// [`gain_card_unpowered_block`] funnel therefore shares only the ending
/// gate, positive store, block-gain count and `AfterBlockGained` boundary
/// with [`gain_powered_card_block`]; keeping Entrench on that unpowered path
/// prevents the Dexterity leak fixed by #1419.
pub(crate) fn gain_current_block_card_unpowered_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "gain_current_block_card_unpowered_exact",
        ));
    }
    let raw = i64::from(ctx.state.block);
    gain_card_unpowered_block(ctx.state, ctx.catalog, ctx.spec, raw, ctx.events)?;
    fire_hook(
        ctx.catalog,
        HookEvent::AfterBlockGained,
        ctx.state,
        ctx.events,
    )
}

/// `gang_up_exact` — exact target-local non-owner history scaling.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) authenticates the exact two card
/// programs, snapshots the chosen target's
/// `nonowner_same_side_powered_damage_results_this_turn`, and performs one
/// card-sourced powered attack for `5 + (5|7) * prior` damage.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `GangUp.get_CanonicalVars` RVA `0xe0df8` fixes Calculation Base 5 and Extra
/// Damage 5; `OnUpgrade` RVA `0xe0eaf` adds 2 only to Extra Damage. The
/// generated multiplier predicate RVA `0x3a0c9c` counts current-turn powered
/// `DamageReceived` rows for this exact receiver whose non-null dealer is a
/// different creature on the owner's side. `OnPlay.MoveNext` RVA `0x3a0d20`
/// constructs one `DamageCmd.Attack`, authenticates it from the played card,
/// targets the chosen enemy, and awaits it once.
///
/// Python `_validated_damage_history_counters` (frozen, deleted #2827) validates the
/// projected target-local owner/non-owner quotient as nonnegative signed
/// counters. Remote Player, remote pet, and owner-pet powered results increment
/// the non-owner side, including blocked results; the owner player's Gang Up
/// increments the disjoint owner counter only after this snapshot.
pub(crate) fn gang_up_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let per_result = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::GangUp, 0, [CompiledArg::I(5)]) => 5_i64,
        (CardId::GangUp, 1, [CompiledArg::I(7)]) => 7_i64,
        _ => return Err(EngineRefusal::MalformedArgs("gang_up_exact")),
    };
    if ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("gang_up_exact action"));
    }
    if crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
        != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("gang_up_exact source"));
    }
    let [step] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("gang_up_exact program"));
    };
    if step.kind != StepKind::GangUpExact || ctx.catalog.args(step.args) != ctx.args {
        return Err(EngineRefusal::MalformedArgs("gang_up_exact program"));
    }
    exact_active_neutral_source(ctx, CardId::GangUp, "gang_up_exact source")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::MalformedArgs("gang_up_exact target"))?;
    let monster = ctx
        .state
        .monsters
        .get(target)
        .filter(|monster| monster.hp > 0)
        .ok_or_else(|| dead_target(target))?;
    let prior = monster.nonowner_same_side_powered_damage_results_this_turn;
    if prior < 0 {
        return Err(EngineRefusal::MalformedArgs("gang_up_exact damage history"));
    }
    monster
        .owner_powered_damage_results_this_turn
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_powered_damage_results_this_turn",
        ))?;
    let damage = per_result
        .checked_mul(i64::from(prior))
        .and_then(|bonus| 5_i64.checked_add(bonus))
        .ok_or(EngineRefusal::CounterOverflow("gang_up damage"))?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )
}

/// Hidden Gem's exact physical `BaseReplayCount` grant (#1964).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_hidden_gem_exact`: filter
/// the live Draw pile, take one Sel-stream pick
/// (`_card_selection_next_item`), and rewrite the picked card's
/// physical replay-count payload (`card_add_base_replay_count`).
///
/// Native current-build authority: `HiddenGem/<OnPlay>d__9::MoveNext` RVA
/// `0x3a5754`. The complete live Draw is validated before the sole
/// CombatCardSelection draw. Attack/Skill/Power candidates are preferred;
/// Status is the fallback. Curse, Quest, effective Unplayable, and a positive
/// enchanted replay count are excluded. `CardModel.CanPlay` is deliberately
/// not consulted. The chosen physical object retains its UID, pile index, and
/// every other payload field while BaseReplayCount increases by 2/3.
pub(crate) fn hidden_gem_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    exact_active_neutral_source(ctx, CardId::HiddenGem, "hidden_gem_exact source")?;
    let amount = one_int(ctx, "hidden_gem_exact")?;
    if !matches!((ctx.spec.identity.upgrade, amount), (0, 2) | (1, 3))
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::HiddenGemExact
                && ctx.catalog.args(step.args) == ctx.args)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.state.multiplayer_ally_key != 0
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
    {
        return Err(EngineRefusal::MalformedArgs("hidden_gem_exact"));
    }
    let draw = ctx.state.piles.get(PileId::Draw).as_slice();
    if draw.is_empty() {
        return Ok(());
    }

    // Validate every Draw row before identity allocation. Empty and entirely
    // ineligible Draw piles remain true zero-allocation/no-identity paths;
    // only a nonempty final pool promotes all five piles to physical UIDs.
    let mut eligible_count = 0usize;
    let mut preferred_count = 0usize;
    for card in draw.iter().copied() {
        if card.flags & CARD_FLAG_LEGACY == 0 {
            let matches = PileId::ALL
                .into_iter()
                .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
                .filter(|candidate| candidate.uid == card.uid)
                .count();
            if matches != 1 {
                return Err(EngineRefusal::ActiveCardNotUnique {
                    uid: card.uid,
                    matches,
                });
            }
        }
        let spec = ctx
            .catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if !crate::engine::play::repeatable_enchantment_identity_is_exact(spec) {
            return Err(EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"));
        }
        let replay = crate::engine::play::effective_replay_count(ctx.state, spec, card.uid)?;
        if spec.native_unplayable
            || matches!(spec.card_type, CardType::Curse | CardType::Quest)
            || replay > 0
        {
            continue;
        }
        eligible_count += 1;
        if matches!(
            spec.card_type,
            CardType::Attack | CardType::Skill | CardType::Power
        ) {
            preferred_count += 1;
        }
    }
    if eligible_count == 0 {
        return Ok(());
    }
    crate::engine::cards::normalize_card_identities(ctx.state)?;
    let draw = ctx.state.piles.get(PileId::Draw).as_slice();
    let use_preferred = preferred_count > 0;
    let mut pool = Vec::with_capacity(if use_preferred {
        preferred_count
    } else {
        eligible_count
    });
    for (index, card) in draw.iter().copied().enumerate() {
        let matches = PileId::ALL
            .into_iter()
            .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
            .filter(|candidate| candidate.uid == card.uid)
            .count();
        if matches != 1 {
            return Err(EngineRefusal::ActiveCardNotUnique {
                uid: card.uid,
                matches,
            });
        }
        let spec = ctx
            .catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if !crate::engine::play::repeatable_enchantment_identity_is_exact(spec) {
            return Err(EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"));
        }
        let replay = crate::engine::play::effective_replay_count(ctx.state, spec, card.uid)?;
        let eligible = !spec.native_unplayable
            && !matches!(spec.card_type, CardType::Curse | CardType::Quest)
            && replay == 0;
        if eligible
            && (!use_preferred
                || matches!(
                    spec.card_type,
                    CardType::Attack | CardType::Skill | CardType::Power
                ))
        {
            pool.push(index);
        }
    }
    if !ctx.state.rng.has_nonzero_words(RngStream::Sel) {
        return Err(EngineRefusal::MalformedArgs("Hidden Gem Sel stream"));
    }
    let selected_count = i32::try_from(pool.len())
        .map_err(|_| EngineRefusal::CounterOverflow("Hidden Gem candidate count"))?;
    let live_rng = ctx.state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live_rng.words,
        counter: live_rng.counter,
    };
    let selected = usize::try_from(
        rng.next_bounded(selected_count)
            .map_err(|_| EngineRefusal::MalformedArgs("Hidden Gem Sel range"))?,
    )
    .map_err(|_| EngineRefusal::CounterOverflow("Hidden Gem candidate index"))?;
    let draw_index = pool[selected];
    let selected_card = draw[draw_index];
    let prior = ctx
        .state
        .card_states
        .get(selected_card.uid)
        .base_replay_count()
        .unwrap_or(0);
    let updated = prior
        .checked_add(i32::try_from(amount).expect("Hidden Gem amount is 2/3"))
        .ok_or(EngineRefusal::CounterOverflow("BaseReplayCount"))?;

    ctx.state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let mut updated_state = ctx.state.card_states.get(selected_card.uid);
    updated_state
        .set_base_replay_count(Some(updated))
        .ok_or(EngineRefusal::CounterOverflow("BaseReplayCount"))?;
    ctx.state.card_states.set(selected_card.uid, updated_state);
    ctx.state.piles.get_mut(PileId::Draw).make_mut()[draw_index].flags |=
        CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    ctx.state.exact_piles = true;
    Ok(())
}

/// `intercept_exact` — exact Intercept/Covered target lifecycle (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — resolve the AnyAlly frame choice, then
/// `_apply_intercept_covered` marks the chosen player covered.
///
/// The ordered unique target set drives the incoming powered multiplier
/// (self Covered = zero; each remote Covered = one additional x1), removes a
/// dead local owner only, and expires as a whole at enemy side end.
pub(crate) fn intercept_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Intercept || !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("intercept_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    crate::engine::allies::set_intercept_covered(ctx.state, key)
}

/// `jack_of_all_trades_exact` — Jack's ordered singular generation transaction.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) -> `_apply_jack_of_all_trades_exact` validates the unique logical active source, consumes the complete
/// 49-row shuffle, and sends the first 1/2 canonical L0 identities through
/// separately awaited `AddGeneratedCardToCombat(Hand, Bottom)` commands.
/// Manual play moves that source to Play first; direct AutoPlay, including a
/// collected Sly card with a non-null pile, retains it in its source pile
/// until the body and listeners have completed.
///
/// Current v0.111.0 native authority: installed and archived `sts2.dll`
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `JackOfAllTrades/<OnPlay>d__6::MoveNext` at RVA `0x3a7ecc` reads the owner
/// Colorless pool and UnlockState at IL `0x0020-0x0046`, excludes runtime Jack
/// instances through `<>c::<OnPlay>b__6_0` RVA `0x3a7ebe`, calls
/// `GetDistinctForCombat(Cards, CombatCardGeneration)` at IL `0x006f-0x0094`,
/// then serially awaits each exact returned CardModel into Hand/Bottom at IL
/// `0x00bd-0x00cb`. Constructor RVA `0xe3813`, canonical-vars RVA `0xe383f`,
/// and upgrade RVA `0xe3820` pin Cards = 1/2 while every result stays L0.
///
/// The generated pool does NOT depend on the owner's character (#2554). IL
/// `0x0026` is `ModelDb::CardPool<ColorlessCardPool>` RVA `0x80e1b`, whose
/// whole body is `Get<ColorlessCardPool>()` — a zero-argument, statically
/// type-keyed singleton off `ModelDb::get_AllSharedCardPools` RVA `0x80e58`,
/// not the owner's `CharacterModel::get_CardPool`. Its rows are the literal
/// 65-element array in `ColorlessCardPool::GenerateAllCards` RVA `0xf11a0`.
/// The owner is read at IL `0x002b-0x0041` only for `Player::get_UnlockState`
/// (the epoch filter, `ColorlessCardPool::FilterThroughEpochs` RVA `0xf13f4`)
/// and `IRunState::get_CardMultiplayerConstraint`; the leftover `Player` from
/// IL `0x0020-0x0021` is just the receiver of `CardFactory`'s extension, whose
/// body RVA `0x112878` runs `FilterForPlayerCount(RunState, cards)`,
/// `FilterForCombat`, `TakeRandom(count, rng)` and a `CombatState.CreateCard`
/// projection — no character or reward-pool comparison anywhere. Python
/// agrees: `_derive_colorless_generation_pool` and the frozen
/// `JACK_OF_ALL_TRADES_POOL_V1091` it reproduces take no owner argument, and
/// the `_GENERATION_POTION_OWNER_TYPES` comment in `combat_sim.py` records
/// that `ColorlessCardPool` "has no owner at all" — the same reason Orange
/// Dough and Toolbox sit outside `_GENERATION_RELIC_PROJECTIONS`'s owner
/// derivation. The gate below
/// ([`crate::engine::cards::jack_of_all_trades_provenance_is_exact`]) therefore
/// keeps the profile, solo and live-stream conjuncts — the profile now any
/// RECORDED one, since the body shuffles `Catalog::jack_of_all_trades_pool`
/// (49 rows when fully unlocked, fewer under a partial profile; #2560) —
/// and still requires *an* owner — it is what closes the catalog over
/// generated results such as Calamity, Discovery, Entropy and Splash — but
/// admits any character as that owner. `entropy_card_pool` is dropped with
/// it: that field is the owner-generation-pool provenance those results
/// consume, not an input to this Colorless shuffle, and each of them refuses
/// on its own. `engine::admission` carries the matching reachability net.
fn jack_of_all_trades_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = match (
        ctx.spec.identity.id,
        ctx.spec.identity.upgrade,
        ctx.args,
        ctx.target,
        ctx.selection,
        ctx.x_value,
    ) {
        (CardId::JackOfAllTrades, 0, [], None, None, 0) => 1,
        (CardId::JackOfAllTrades, 1, [], None, None, 0) => 2,
        _ => return Err(EngineRefusal::MalformedArgs("jack_of_all_trades_exact")),
    };
    exact_active_neutral_source(
        ctx,
        CardId::JackOfAllTrades,
        "jack_of_all_trades_exact source",
    )?;
    if !crate::engine::cards::jack_of_all_trades_provenance_is_exact(ctx.state, ctx.catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "jack of all trades generation provenance",
        ));
    }

    fn apply(ctx: &mut StepCtx<'_>, count: usize) -> Result<(), EngineRefusal> {
        // #2560: the recorded profile's self-excluding Colorless pool
        // (`Catalog::jack_of_all_trades_pool`), which is
        // `JACK_OF_ALL_TRADES_POOL_V1091` at the fully-unlocked profile.
        let pool = ctx.catalog.jack_of_all_trades_pool();
        let shuffled = crate::engine::cards::shuffle_generation_slice(ctx.state, &pool)?;
        for id in shuffled.into_iter().take(count) {
            inject_generated_exact_bottom(
                ctx.state,
                ctx.catalog,
                CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                },
                1,
                PileId::Hand,
                ctx.events,
            )?;
        }
        Ok(())
    }

    // A later singular add can fail on a counter/listener edge after an
    // earlier add has committed. Rehearse the complete awaited body first;
    // successful execution retains native per-command terminal suppression.
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
    apply(&mut probe_ctx, count)?;
    apply(ctx, count)
}

pub(crate) fn jack_of_all_trades_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    jack_of_all_trades_body(ctx)
}

/// `jackpot_exact` — Jackpot's attack-then-generation transaction.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `resolve_jackpot`: one
/// awaited targeted attack, then three canonical-zero-cost Generation-pool
/// samples added to Hand.
///
/// Native Jackpot/<OnPlay>d__3::MoveNext RVA 0x3a80d4 queries the owner's
/// unlocked pool, filters canonical zero-cost cards, and samples with replacement.
/// Python: `resolve_jackpot` (frozen, deleted #2827) validates the active Play source,
/// awaits one attack, consumes three Generation `NextItem` samples before any
/// insertion, then adds three fresh level-matched zero-cost cards to Hand.
/// The boundary interns the complete with-replacement owner pool, and a clone
/// preflight makes the fused mutation transactional.
pub(crate) fn jackpot_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, upgrade) = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Jackpot, 0, [CompiledArg::I(25), CompiledArg::I(3)]) => (25, 0),
        (CardId::Jackpot, 1, [CompiledArg::I(30), CompiledArg::I(3)]) => (30, 1),
        _ => return Err(EngineRefusal::MalformedArgs("jackpot_exact")),
    };
    exact_active_neutral_source(ctx, CardId::Jackpot, "jackpot_exact source")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    ctx.state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?;
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs(
            "jackpot generation provenance",
        ))?;
    // #2560/#2946: the owner's zero-cost pool under the RECORDED profile
    // (`Catalog::owner_zero_cost_pool`), not the fully-unlocked constant. The
    // old gate let an Ironclad owner through on any profile and drew the
    // fully-unlocked list; admission's `PartialUnlockProfile` fence was the
    // only thing that kept that path from running under a partial profile.
    if !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "jackpot generation provenance",
        ));
    }
    let owner_pool = ctx.catalog.owner_zero_cost_pool(owner);
    let pool = owner_pool.as_slice();
    if ctx.state.rng.is_vacant(RngStream::Generation) {
        return Err(EngineRefusal::MalformedArgs("jackpot generation stream"));
    }
    fn apply(
        ctx: &mut StepCtx<'_>,
        body: (usize, i64, u8, &[CardId]),
    ) -> Result<(), EngineRefusal> {
        let (target, damage, upgrade, pool) = body;
        player_attack_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[target],
            damage,
            1,
            ctx.events,
        )?;
        let generated = [
            sample_generation_slice(ctx.state, pool)?,
            sample_generation_slice(ctx.state, pool)?,
            sample_generation_slice(ctx.state, pool)?,
        ];
        for id in generated {
            inject_generated_exact_bottom(
                ctx.state,
                ctx.catalog,
                CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                },
                1,
                PileId::Hand,
                ctx.events,
            )?;
        }
        Ok(())
    }
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target: Some(target),
        selection: None,
        x_value: 0,
        args: ctx.args,
        events: &mut probe_events,
    };
    apply(&mut probe_ctx, (target, damage, upgrade, pool))?;
    apply(ctx, (target, damage, upgrade, pool))
}

/// `lift_exact` — exact typed-AnyAlly powered Block (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — resolve the AnyAlly frame choice, then
/// `_gain_powered_card_block_to_player` routes the powered block
/// gain to the chosen player.
///
/// The local recipient uses the complete powered-card Block funnel. The
/// remote quotient stores the native capped result after folding the exact
/// CardPlay owner's permanent/temporary Dexterity and excluding every other
/// unrepresented teammate/owner modifier and listener.
pub(crate) fn lift_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "lift_exact")?;
    if ctx.spec.identity.id != CardId::Lift
        || amount != 11 + 5 * i64::from(ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("lift_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    let mut probe = ctx.state.clone();
    crate::engine::allies::gain_powered_block(
        &mut probe,
        ctx.catalog,
        key,
        ctx.spec,
        amount,
        &mut Vec::new(),
    )?;
    crate::engine::allies::gain_powered_block(
        ctx.state,
        ctx.catalog,
        key,
        ctx.spec,
        amount,
        ctx.events,
    )
}

/// `metamorphosis_exact` — exact owner-Attack generation transaction.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) and `_apply_metamorphosis_exact`
/// sample 3/5 L0 Attacks independently with replacement from Generation,
/// construct the full list, then execute singular generated commands. Each
/// fresh instance receives Energy and Star `SetToFreeThisCombat` state.
///
/// Destination (#3243, current `sts2.dll` SHA-256 `9cb4f1ad…`):
/// `Metamorphosis/<OnPlay>d__5::MoveNext` RVA `0x3ac1e8` IL `0x00c6-0x00d5`
/// calls `CardPileCmd.AddGeneratedCardToCombat(card, PileType 1 = Draw,
/// owner, CardPilePosition 3 = Random)`, so each member lands at a random
/// Draw index with one Shuffle-stream draw — not at the bottom of the Hand
/// the frozen Python oracle used.
pub(crate) fn metamorphosis_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let count = match (
        ctx.spec.identity.id,
        ctx.spec.identity.upgrade,
        ctx.args,
        ctx.target,
        ctx.selection,
        ctx.x_value,
    ) {
        (CardId::Metamorphosis, 0, [CompiledArg::I(3)], None, None, 0) => 3,
        (CardId::Metamorphosis, 1, [CompiledArg::I(5)], None, None, 0) => 5,
        _ => return Err(EngineRefusal::MalformedArgs("metamorphosis_exact")),
    };
    if !crate::engine::play::body_enchantment_is_exact(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("metamorphosis_exact"));
    }
    exact_active_neutral_source(ctx, CardId::Metamorphosis, "metamorphosis_exact source")?;
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs(
            "Metamorphosis generation owner",
        ))?;
    let pool = ctx.catalog.metamorphosis_attack_pool(owner);
    if !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Metamorphosis generation provenance",
        ));
    }
    for &id in pool.iter() {
        let identity = CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        let spec = ctx
            .catalog
            .atom(&identity)
            .and_then(|atom| ctx.catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        if spec.identity != identity
            || !spec.is_attack
            || crate::content_tables::card_row(id, 0) != Some(spec.row)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Metamorphosis generation closure",
            ));
        }
    }

    fn apply(ctx: &mut StepCtx<'_>, pool: &[CardId], count: usize) -> Result<(), EngineRefusal> {
        // Native constructs the full sampled list before beginning the first
        // singular generated-card command. This remains true when an earlier
        // result's listener ends combat and suppresses later insertions.
        let generated = (0..count)
            .map(|_| {
                Ok(CardIdentity {
                    id: sample_generation_slice(ctx.state, pool)?,
                    upgrade: 0,
                    enchantment: None,
                })
            })
            .collect::<Result<Vec<_>, EngineRefusal>>()?;
        inject_generated_free_this_combat_batch_draw_random(
            ctx.state,
            ctx.catalog,
            &generated,
            ctx.events,
        )
    }

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
    apply(&mut probe_ctx, &pool, count)?;
    apply(ctx, &pool, count)
}

/// Complete exact L0 Attack generation pools admitted by R38.5.
///
/// Metamorphosis (`<OnPlay>d__5::MoveNext` `0x3ac1e8`, `Where` IL_0075; the older `0x3a9b08` citation named no MoveNext in this build) filters `CardType.Attack` and then
/// `FilterForCombat` `0x11634a`, which excludes Basic, Ancient and Event —
/// a *different* comparison from Calamity's `Common..Rare`, and the reason
/// these two Attack pools are derived with different [`PoolRarity`] values.
/// `METAMORPHOSIS_ATTACK_POOLS_V1101` is this derivation at the
/// fully-unlocked profile (#2542).
pub(crate) fn metamorphosis_owner_attack_pool(
    owner: RewardPool,
    epochs: Option<&[&'static str]>,
) -> Vec<CardId> {
    derive_character_generation_pool(
        owner,
        epochs,
        PoolRarity::NotBasicAncientEvent,
        Some(crate::content_tables::CardType::Attack),
        false,
    )
}

/// `mimic_exact` — exact selected-Player Block copy (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — resolve the AnyAlly frame choice, read
/// the chosen player's current Block (`_selected_player_block`), and
/// gain that much powered block.
///
/// The selected Player's live Block is read first; that snapshot then enters
/// the local owner's complete powered-card Block funnel.
pub(crate) fn mimic_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::Mimic || !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("mimic_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    let amount = i64::from(crate::engine::allies::selected_block(ctx.state, key)?);
    let mut probe = ctx.state.clone();
    crate::engine::allies::gain_powered_block(
        &mut probe,
        ctx.catalog,
        0,
        ctx.spec,
        amount,
        &mut Vec::new(),
    )?;
    crate::engine::allies::gain_powered_block(
        ctx.state,
        ctx.catalog,
        0,
        ctx.spec,
        amount,
        ctx.events,
    )
}

/// Stack Nostalgia's persistent result-location listener.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Nostalgia/<OnPlay>d__1::MoveNext` RVA `0x3af5f0` applies one additive
/// `NostalgiaPower`; `NostalgiaPower::ModifyCardPlayResultLocation` RVA
/// `0xa5020` changes an owned Attack/Skill's still-natural Discard result to
/// Draw/Top while prior same-turn Attack/Skill starts are below Amount.
/// `_run_steps_inner` (frozen Python, deleted #2827) and `_resolve_card_play_result_location`
/// are the matching source-derived executable oracle.
pub(crate) fn nostalgia(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_result_location_power(
        ctx,
        CardId::Nostalgia,
        StepKind::Nostalgia,
        PowerId::Nostalgia,
        "nostalgia",
    )
}

/// `panic_button_exact` — exact powered Block then NoBlock duration.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the awaited powered Block command,
/// then an ending-gated stack on the owner's NoBlock debuff.
///
/// Existing NoBlock zeroes the leading powered Block. The newly-applied
/// duration is ending-gated after that complete block/listener command and
/// decrements once after each enemy side without a fresh-application skip.
pub(crate) fn panic_button_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (block, duration) = two_ints(ctx, "panic_button_exact")?;
    let expected_block = 30 + 10 * i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::PanicButton
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || block != expected_block
        || duration != 2
    {
        return Err(EngineRefusal::MalformedArgs("panic_button_exact"));
    }
    let current = ctx.state.powers.value(PowerId::NoBlock);
    let updated = current
        .checked_add(2)
        .ok_or(EngineRefusal::CounterOverflow("no block"))?;
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, block, ctx.events)?;
    if ctx.state.history.over {
        return Ok(());
    }
    ctx.state
        .powers
        .set(PowerId::NoBlock, SlotWire::Int, updated);
    crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::NoBlock, updated);
    Ok(())
}

/// `purity_exact` — Purity's ordered, zero-through-limit Hand exhaustion.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_begin_purity_selection_exact`: freeze a FromHand candidate snapshot and suspend at the
/// up-to-`limit` exhaust pick.
///
/// The body promotes exact piles, freezes the complete ordered physical Hand,
/// and parks that snapshot in the existing CardPlay continuation. Resume
/// enumerates native ordered permutations and applies serial Exhaust commands.
pub(crate) fn begin_purity_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<Option<Vec<HotCard>>, EngineRefusal> {
    let limit = one_int(ctx, "purity_exact")?;
    let expected = 3 + 2 * i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::Purity
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || limit != expected
    {
        return Err(EngineRefusal::MalformedArgs("purity_exact"));
    }
    if ctx.state.history.over {
        return Ok(None);
    }
    crate::engine::cards::normalize_card_identities(ctx.state)?;
    ctx.state.exact_piles = true;
    let candidates = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .copied()
        .map(|mut card| {
            card.flags |= CARD_FLAG_PICK;
            card
        })
        .collect::<Vec<_>>();
    Ok((!candidates.is_empty()).then_some(candidates))
}

pub(crate) fn purity_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if begin_purity_exact(ctx)?.is_some() {
        Err(EngineRefusal::ContinuationNotModeled)
    } else {
        Ok(())
    }
}

/// `rally_exact` — exact ordered living-Player powered Block fan-out (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — one powered block gain per living
/// ally (`_gain_powered_card_block_to_all_allies`).
///
/// The fixed `[owner, teammate]` Player order is clone-rehearsed as one batch.
/// Python refuses when the owner's nested Juggernaut hit could end combat
/// before a remote recipient; repeat that wall here before the first store.
pub(crate) fn rally_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "rally_exact")?;
    if ctx.spec.identity.id != CardId::Rally
        || amount != 12 + 5 * i64::from(ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("rally_exact"));
    }
    if ctx.state.multiplayer_ally_key == 1
        && ctx.state.fanouts.multiplayer_ally().alive
        && crate::engine::play::juggernaut_block_can_end_combat(ctx.state, 1)
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Rally owner position with lethal Juggernaut",
        ));
    }
    fn body(
        state: &mut crate::hot::HotState,
        catalog: &crate::catalog::Catalog,
        spec: &crate::catalog::CardSpec,
        events: &mut Vec<crate::engine::Event>,
        amount: i64,
    ) -> Result<(), EngineRefusal> {
        let recipients = crate::engine::allies::living_keys(state).collect::<Vec<_>>();
        for key in recipients {
            if state.history.over {
                break;
            }
            crate::engine::allies::gain_powered_block(state, catalog, key, spec, amount, events)?;
        }
        Ok(())
    }
    let mut probe = ctx.state.clone();
    body(&mut probe, ctx.catalog, ctx.spec, &mut Vec::new(), amount)?;
    body(ctx.state, ctx.catalog, ctx.spec, ctx.events, amount)
}

/// Stack Rebound's one-shot result-location listener.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Rebound/<OnPlay>d__5::MoveNext` RVA `0x3b63fc` attacks for 9/12 then
/// applies one additive `ReboundPower` stack;
/// `ReboundPower::ModifyCardPlayResultLocation` RVA `0xa6879` changes only a
/// still-natural Discard result to Draw/Top, and the matching callback at RVA
/// `0x342158` decrements exactly when that listener changed the location.
/// `_run_steps_inner` (frozen Python, deleted #2827) and `_resolve_card_play_result_location`
/// are the matching source-derived executable oracle.
pub(crate) fn rebound(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    apply_result_location_power(
        ctx,
        CardId::Rebound,
        StepKind::Rebound,
        PowerId::Rebound,
        "rebound",
    )
}

/// Both bodies reach the power through the generic `PowerCmd.Apply<T>`
/// (`Nostalgia` `0x3af5f0` IL_0040, `Rebound` `0x3b63fc` IL_00fd), whose
/// ``PowerCmd/<Apply>d__1`1::MoveNext`` (RVA `0x3ef988`) returns at
/// IL_0020-0x0034 while `CombatManager.IsEnding`: a lethal Rebound attack
/// (IL_004c) leaves no ReboundPower (#3183). Validation still runs first.
fn apply_result_location_power(
    ctx: &mut StepCtx<'_>,
    card_id: CardId,
    step_kind: StepKind,
    power: PowerId,
    site: &'static str,
) -> Result<(), EngineRefusal> {
    let programs = ctx.catalog.steps(ctx.spec);
    let program = match (card_id, programs) {
        (CardId::Nostalgia, [program]) => program,
        (CardId::Rebound, [attack, program])
            if attack.kind == StepKind::Attack
                && matches!(
                    ctx.catalog.args(attack.args),
                    [CompiledArg::I(damage), CompiledArg::I(1)]
                        if *damage == 9 + 3 * i64::from(ctx.spec.identity.upgrade)
                ) =>
        {
            program
        }
        _ => return Err(EngineRefusal::MalformedArgs(site)),
    };
    exact_active_neutral_source(ctx, card_id, site)?;
    let target_is_exact = match (card_id, ctx.target) {
        (CardId::Nostalgia, None) => true,
        (CardId::Rebound, Some(index)) => ctx.state.monsters.get(index).is_some(),
        _ => false,
    };
    if !target_is_exact
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != step_kind
        || ctx.catalog.args(program.args) != ctx.args
        || !matches!(ctx.args, [CompiledArg::I(1)])
    {
        return Err(EngineRefusal::MalformedArgs(site));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let old = ctx.state.powers.value(power);
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    if old == 0 {
        let mut prepared = ctx.state.clone();
        if !prepared.fanouts.register_result_location_power(power) {
            return Err(EngineRefusal::CounterOverflow("result-location order"));
        }
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            &mut prepared,
            power,
            old,
            updated,
        )?;
        *ctx.state = prepared;
    } else {
        crate::engine::turn::prepare_after_side_turn_end_scalar_write(
            ctx.state, power, old, updated,
        )?;
    }
    ctx.state.powers.set(power, SlotWire::Int, updated);
    crate::engine::damage::note_power(ctx.events, Subject::Player, power, updated);
    Ok(())
}

/// `("rend_exact", base, per)` — Rend: one targeted attack whose damage grows
/// by `per` for every live power **instance** on the target.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `count_target_powers_for_rend` counts non-temporary Type-2 PowerModel instances by cardinality
/// (never by Amount), then one ordinary [`player_attack`] lands the sum.
///
/// The census reduction, checked at the engine sites it leans on:
///
/// * the Python counter's rows are per-instance `Monster` fields. Demise,
///   Oblivion, Shrink, SicEm, and PlowThreshold now arrive through their exact
///   scalar slots. Every still-unmapped row (the Knockdown/TagTeam/Flanking
///   instance tuples, Slow, Asleep, and kind-private validator fields)
///   refuses at `boundary::MONSTER_FIELDS`, so its admitted value is the zero
///   default and its arm contributes nothing;
/// * what the boundary does map arrives as the slot vector, and
///   `admit` pins that vector to
///   [`crate::engine::admission::IMPLEMENTED_POWERS`] — of which Strength
///   counts only while negative (the visible-type flip Python names), Debilitate, Doom,
///   Weak, Vuln, Hang, Poison, Conqueror and PlowThreshold count while positive, Shriek
///   counts by presence (admission additionally pins Shriek to the Terror Eel
///   at Bool 1, which is Python's owner check), while Ritual, TempStrength and
///   Burrowed/CurlUp/Vigor/Vital/Thorns/Soar are Type-1 rows Python validates
///   but deliberately does not count. Rampart is also Type-1
///   (`RampartPower::get_Type` RVA `0xa6343` returns 1), so its fixed Living
///   Shield owner state does not change Rend's count;
/// * the two innate instances without Amount fields — Bygone Effigy's
///   ever-present SlowPower and Bowlbug Rock's unique ImbalancedPower —
///   count by kind, exactly as Python's final two terms do;
/// * Python's target-kind census is enforced by
///   `count_target_powers_for_rend` (frozen Python, deleted #2827) and was re-derived against
///   [`MonsterKind`] on 2026-08-20: the two
///   sets are equal (105 = 105, no difference either way), so every
///   representable kind is in the census and the gate adds no arm;
/// * a dead or vanished target refuses ("not a live hittable creature"),
///   never skips — the one Python refusal in the branch that survives the
///   reduction.
pub(crate) fn rend_target_power_count(monster: &HotMonster) -> i64 {
    let positive_singletons = [
        PowerId::Conqueror,
        PowerId::Debilitate,
        PowerId::Demise,
        PowerId::Doom,
        PowerId::Hang,
        PowerId::PlowThreshold,
        PowerId::Poison,
        PowerId::Shriek,
        PowerId::Shrink,
        PowerId::Strangle,
        PowerId::Vuln,
        PowerId::Weak,
    ];
    i64::from(monster.powers.value(PowerId::Strength) < 0)
        + positive_singletons
            .into_iter()
            .map(|power| i64::from(monster.powers.value(power) > 0))
            .sum::<i64>()
        // Vec allocations are bounded by isize::MAX, so this cardinality is
        // lossless on every supported target before the checked damage fold.
        + monster.misery_debuff_order.knockdown().len() as i64
        + i64::from(monster.kind == MonsterKind::BygoneEffigy)
        // Imbalanced is Type-2 and present-by-kind, so it contributes exactly
        // one to the cardinality — never once per carrier test. Routed through
        // the shared predicate so slice B's ledger cannot double-count.
        + i64::from(crate::engine::monsters::owner_carries_imbalanced(monster))
}

pub(crate) fn rend_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (base, per) = two_ints(ctx, "rend_exact")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let count = {
        let Some(monster) = ctx.state.monsters.get(target) else {
            return Err(dead_target(target));
        };
        if monster.hp <= 0 {
            return Err(dead_target(target));
        }
        rend_target_power_count(monster)
    };
    let damage = per
        .checked_mul(count)
        .and_then(|bonus| base.checked_add(bonus))
        .ok_or(EngineRefusal::CounterOverflow("rend damage"))?;
    crate::engine::damage::player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )
}

/// `restlessness_exact` — separately awaited draws, then Energy.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) (the RESTLESSNESS arm) — only when the
/// post-removal Hand is empty, `_restlessness_continue` issues one
/// separately-awaited one-card Draw command per level operand, then the
/// independently ending-gated energy gain of the same operand.
///
/// Unit E now models static/local/transient Retain and the owner turn-end
/// flush, so both exact source rows are admissible. Each Draw remains its own
/// command: ending and hand-cap gates are re-entered for every iteration.
/// `NoEnergyGainPower` is boundary-representable and ledger-authenticated but
/// independently admission-refused until every local gain modifier is
/// modeled; the final admitted gain therefore reduces to one checked scalar
/// addition.
pub(crate) fn restlessness_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("restlessness_exact"));
    };
    let expected = 2 + i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::Restlessness
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *amount != expected
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("restlessness_exact owner"));
    }
    if !ctx.state.piles.get(PileId::Hand).is_empty() {
        return Ok(());
    }
    let count: u32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("restlessness draw count"))?;
    continue_restlessness(ctx.state, ctx.catalog, count, count, ctx.events)
}

fn restlessness_caller(remaining: u32) -> Result<DrawCaller, EngineRefusal> {
    match remaining {
        0 => Ok(DrawCaller::RestlessnessFinal),
        1 => Ok(DrawCaller::RestlessnessOneRemaining),
        2 => Ok(DrawCaller::RestlessnessTwoRemaining),
        _ => Err(EngineRefusal::ContinuationNotModeled),
    }
}

fn continue_restlessness(
    state: &mut HotState,
    catalog: &Catalog,
    mut remaining: u32,
    amount: u32,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    while remaining > 0 {
        remaining = remaining
            .checked_sub(1)
            .ok_or(EngineRefusal::CounterOverflow("restlessness draw count"))?;
        if draw_cards_for_card_result(state, catalog, 1, restlessness_caller(remaining)?, events)?
            == CardDrawResult::Suspended
        {
            return Ok(());
        }
    }
    finish_restlessness(state, amount)
}

fn finish_restlessness(state: &mut HotState, amount: u32) -> Result<(), EngineRefusal> {
    if !state.history.over {
        let amount: i16 = amount
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("restlessness energy"))?;
        state.energy = state
            .energy
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("restlessness energy"))?;
    }
    Ok(())
}

pub(crate) fn resume_restlessness_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
    caller: DrawCaller,
    amount: u32,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let remaining = match caller {
        DrawCaller::RestlessnessFinal => 0,
        DrawCaller::RestlessnessOneRemaining => 1,
        DrawCaller::RestlessnessTwoRemaining => 2,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    continue_restlessness(state, catalog, remaining, amount, events)
}

/// `seeker_strike_exact` — Seeker Strike's attack plus exact Draw shortlist.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_seeker_strike_exact` runs
/// the attack and may return a pending selection, which the dispatch
/// propagates as a suspended continuation.
///
/// Stable shuffle consumes the Selection stream over the complete payload-
/// sorted Draw copy. Membership is frozen in shuffle order for projection;
/// player options are re-enumerated in live Draw order before resume.
pub(crate) fn begin_seeker_strike_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<Option<Vec<HotCard>>, EngineRefusal> {
    let (damage, shortlist_size) = two_ints(ctx, "seeker_strike_exact")?;
    let expected_damage = 9 + 3 * i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::SeekerStrike
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || damage != expected_damage
        || shortlist_size != 3
    {
        return Err(EngineRefusal::MalformedArgs("seeker_strike_exact"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    crate::engine::damage::player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    if ctx.state.hp <= 0 {
        return Ok(None);
    }

    crate::engine::cards::normalize_card_identities(ctx.state)?;
    ctx.state.exact_piles = true;
    let mut shuffled = crate::engine::selection::seeker_sorted_draw(ctx.state, ctx.catalog)?;
    let live = ctx.state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    rng.shuffle(&mut shuffled)
        .map_err(|_| EngineRefusal::CounterOverflow("Seeker Strike shuffle"))?;
    ctx.state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let shortlist: Vec<_> = shuffled.into_iter().take(3).collect();
    if ctx.state.history.over {
        return Ok(None);
    }
    let shortlist_uids: Vec<_> = shortlist.iter().map(|card| card.uid).collect();
    let candidates: Vec<_> = ctx
        .state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| shortlist_uids.contains(&card.uid))
        .collect();
    if candidates.len() <= 1 {
        if let Some(card) = candidates.first() {
            crate::engine::selection::move_seeker_card(ctx.state, card.uid, ctx.events)?;
        }
        Ok(None)
    } else {
        // Preserve the shuffled shortlist order for canonical projection.
        // Legal choices are independently re-enumerated in live Draw order.
        Ok(Some(shortlist))
    }
}

pub(crate) fn seeker_strike_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if begin_seeker_strike_exact(ctx)?.is_some() {
        Err(EngineRefusal::ContinuationNotModeled)
    } else {
        Ok(())
    }
}

fn owner_attack_pool(owner: RewardPool) -> &'static [CardId] {
    // `METAMORPHOSIS_ATTACK_POOLS_V1101` is generated in ascending key order.
    // Keep the hot lookup enum-only; focused tests pin these generated slots.
    match owner {
        RewardPool::Defect => crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101[0].1,
        RewardPool::Ironclad => crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101[1].1,
        RewardPool::Necrobinder => crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101[2].1,
        RewardPool::Regent => crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101[3].1,
        RewardPool::Silent => crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101[4].1,
    }
}

/// This owner's slot in `SPLASH_CHARACTER_POOL_ORDER_V1101`.
///
/// The generated order is `_CARD_POOL_CENSUS["character_pool_order"]`, which
/// the codegen refuses to emit if it ever drifts from the native
/// Ironclad/Silent/Regent/Necrobinder/Defect order; the `debug_assert` re-proves
/// the mapping here rather than trusting the index.
fn splash_character_index(owner: RewardPool) -> usize {
    let index = match owner {
        RewardPool::Ironclad => 0,
        RewardPool::Silent => 1,
        RewardPool::Regent => 2,
        RewardPool::Necrobinder => 3,
        RewardPool::Defect => 4,
    };
    debug_assert_eq!(
        crate::content_tables::SPLASH_CHARACTER_POOL_ORDER_V1101[index],
        match owner {
            RewardPool::Ironclad => "IRONCLAD",
            RewardPool::Silent => "SILENT",
            RewardPool::Regent => "REGENT",
            RewardPool::Necrobinder => "NECROBINDER",
            RewardPool::Defect => "DEFECT",
        }
    );
    index
}

/// Bit `i` set iff character `i`'s first unlock epoch is in `epochs`.
///
/// `epochs` must be the normalized ascending profile the boundary admits, so
/// membership is a binary search rather than a scan.
pub(crate) fn splash_character_mask(epochs: &[&'static str]) -> u8 {
    let mut mask = 0;
    for (index, epoch) in crate::content_tables::SPLASH_CHARACTER_FIRST_UNLOCK_EPOCH_V1101
        .iter()
        .enumerate()
    {
        if epochs.binary_search(epoch).is_ok() {
            mask |= 1 << index;
        }
    }
    mask
}

/// Exact ordered Splash pool for one arbitrary normalized unlock profile.
///
/// Port of Python `_derive_splash_attack_pool` (frozen, deleted #2827) (#2469), which is the
/// only Splash-side consumer that works on a *partial* profile. Three
/// filters, in the authority's own order:
///
/// 1. **Character inclusion.** A non-owner character joins iff the first entry
///    of its `character_unlock_epochs` list is in the profile — precomputed
///    into `character_mask` by [`splash_character_mask`]. Native
///    `CharacterCardPools` removes the source owner's pool *only* when more
///    than one pool exists, so an owner whose peers are all still locked keeps
///    its own pool (`splash_from_solo_ironclad.single_pool_edge`).
/// 2. **Native order.** The survivors are concatenated in
///    `SPLASH_CHARACTER_POOL_ORDER_V1101` order, never owner-first: Python
///    rebuilds `included` from `character_pool_order` after the removal.
/// 3. **Row epoch.** A row with an `unlock_epoch` outside the profile is
///    skipped; a row without one is unconditional. The remaining four row
///    predicates cannot vary with the profile and are applied at codegen time
///    (see `SPLASH_CHARACTER_ATTACK_ROWS_V1101`).
///
/// Duplicates are dropped first-occurrence-wins, as Python's `seen` set does.
/// Python's normalization checks on the tuple itself are the boundary's job
/// here: an unsorted, duplicated, `EPOCH.`-prefixed or unknown epoch never
/// reaches this function because `Boundary` refuses the document (I5).
///
/// Consumes no RNG. Python's `len(selected) < 3` refusal is left to the
/// callers, which already refuse a short pool, and to admission, which refuses
/// the whole fight at load.
pub(crate) fn derive_splash_attack_pool(
    owner: RewardPool,
    epochs: &[&'static str],
    character_mask: u8,
) -> Vec<CardId> {
    let owner_index = splash_character_index(owner);
    let others = character_mask & !(1 << owner_index);
    let included = if others == 0 {
        1 << owner_index
    } else {
        others
    };
    let mut pool: Vec<CardId> = Vec::new();
    for (index, (_, rows)) in crate::content_tables::SPLASH_CHARACTER_ATTACK_ROWS_V1101
        .iter()
        .enumerate()
    {
        if included & (1 << index) == 0 {
            continue;
        }
        for (id, unlock_epoch) in rows.iter() {
            if let Some(epoch) = unlock_epoch
                && epochs.binary_search(epoch).is_err()
            {
                continue;
            }
            if !pool.contains(id) {
                pool.push(*id);
            }
        }
    }
    pool
}

/// Exact fully-unlocked solo Splash pool in native CharacterCardPools order.
pub(crate) fn splash_pool(owner: RewardPool) -> Vec<CardId> {
    let mut pool = Vec::with_capacity(145 - owner_attack_pool(owner).len());
    for candidate in [
        RewardPool::Ironclad,
        RewardPool::Silent,
        RewardPool::Regent,
        RewardPool::Necrobinder,
        RewardPool::Defect,
    ] {
        if candidate != owner {
            pool.extend_from_slice(owner_attack_pool(candidate));
        }
    }
    pool
}

/// Private/test-only exact owner-excluded Attack 1-of-3 Splash foundation.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Splash/<OnPlay>d__2::MoveNext` RVA `0x3be150` materializes
/// `UnlockState.CharacterCardPools`, removes the physical source owner's pool
/// only when more than one pool exists, concatenates the remaining unlocked
/// Attacks in native Ironclad/Silent/Regent/Necrobinder/Defect order, and calls
/// `CardFactory.GetDistinctForCombat(..., 3, CombatCardGeneration)`. It upgrades
/// all three options for Splash+, awaits a skippable screen, then applies
/// `SetToFreeThisTurn` and one generated Hand/Bottom add only to a non-null
/// answer. Python `_begin_splash_selection_exact` (frozen, deleted #2827) and
/// `_apply_splash_selection_exact` preserve the same full shuffle,
/// source-level offers, skip/terminal gates, generated insertion and serial
/// CardPlay continuation.
pub(crate) fn begin_splash_exact(ctx: &mut StepCtx<'_>) -> Result<Vec<HotCard>, EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Splash
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        // Splash is the one generator Python derives from ANY normalized
        // profile (#2469), so a partial one interned in the catalog is exact
        // here where every other generator still requires the full set.
        || !(ctx.state.fully_unlocked_card_pool_epochs
            || ctx.catalog.splash_unlock_epochs().is_some())
        || ctx.state.multiplayer_ally_key != 0
        || ctx.state.rng.is_vacant(RngStream::Generation)
    {
        return Err(EngineRefusal::MalformedArgs("splash_exact"));
    }
    exact_active_neutral_source(ctx, CardId::Splash, "splash_exact source")?;
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs("splash_exact owner"))?;
    let pool = ctx.catalog.splash_attack_pool(owner);
    if pool.len() < 3 {
        return Err(EngineRefusal::MalformedArgs("splash_exact pool"));
    }
    let shuffled = shuffle_generation_slice(ctx.state, &pool)?;
    shuffled[..3]
        .iter()
        .map(|id| {
            let identity = CardIdentity {
                id: *id,
                upgrade: ctx.spec.identity.upgrade,
                enchantment: None,
            };
            let atom = ctx
                .catalog
                .atom(&identity)
                .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
            Ok(HotCard {
                uid: 0,
                atom,
                flags: 0,
            })
        })
        .collect()
}

pub(crate) fn splash_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    // The CardPlay driver owns this body because its awaited selection must
    // freeze the generated options in the existing persisted record. Direct
    // dispatch is consequently never an executable route.
    let _ = ctx;
    Err(EngineRefusal::ContinuationNotModeled)
}

fn whistle_source_is_exact(spec: &CardSpec, catalog: &Catalog) -> bool {
    matches!(spec.identity.upgrade, 0 | 1)
        && crate::engine::play::body_enchantment_is_exact(spec)
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
        && matches!(catalog.steps(spec), [attack, stun]
            if attack.kind == StepKind::Attack
                && catalog.args(attack.args)
                    == [CompiledArg::I(if spec.identity.upgrade == 0 { 33 } else { 44 }), CompiledArg::I(1)]
                && stun.kind == StepKind::StunTarget
                && catalog.args(stun.args).is_empty())
}

pub(crate) fn preflight_whistle(
    state: &HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    target: Option<u8>,
    public_manual: bool,
) -> Result<(), EngineRefusal> {
    if spec.identity.id != CardId::Whistle
        || !whistle_source_is_exact(spec, catalog)
        || state.multiplayer_ally_key != 0
        // #2647 slice A (A-opt): the same wall admission uses, so a bypassing
        // direct/manual caller meets an identical gate.
        || !state.monsters.iter().all(|monster| {
            crate::engine::monsters::whistle_stun_target_is_representable(state, catalog, monster)
        })
        || (public_manual && (state.pending.is_some() || !state.frames.is_empty()))
        || !state.monsters.iter().any(|monster| monster.hp > 0)
        || target.is_some_and(|target| {
            !state
                .monsters
                .get(usize::from(target))
                .is_some_and(|monster| monster.hp > 0)
        })
    {
        return Err(EngineRefusal::MalformedArgs("Whistle exact entry"));
    }
    Ok(())
}

/// `stun_target` — Whistle's post-attack one-shot forced move.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Whistle's OnPlay body (`0x3c72f4`) awaits Attack, then enters
/// `CreatureCmd::Stun` (`0x132ae0` -> `0x3ed060` -> `0x132b2c`). The null
/// callback is the completed-task delegate at `0x3e8f83`; the VFX wrapper is
/// created at `0x3e8fc8` and executes at `0x461f58`, before
/// `Creature::StunInternal` (`0x11d7cc`) parks the current telegraph.
/// Python: `_run_steps_inner` (frozen, deleted #2827) — Whistle: after the awaited attack,
/// `_player_stun_monster` parks the target's current telegraph as a
/// forced follow-up behind a one-shot STUNNED state, no-oping on a dead or
/// already-stunned target.
pub(crate) fn stun_target(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("stun_target"));
    }
    exact_active_neutral_source(ctx, CardId::Whistle, "Whistle source")?;
    if !whistle_source_is_exact(ctx.spec, ctx.catalog) || ctx.state.multiplayer_ally_key != 0 {
        return Err(EngineRefusal::MalformedArgs("Whistle exact body"));
    }
    let Some((Some(_), Some(frozen_target))) =
        crate::engine::play::active_card_current_context(ctx.source_uid)
    else {
        return Err(EngineRefusal::MalformedArgs("Whistle CardPlay target"));
    };
    let Some(target) = ctx.target else {
        return Err(EngineRefusal::MalformedArgs("Whistle target"));
    };
    let Some(monster) = ctx.state.monsters.get(target) else {
        return Ok(());
    };
    // The Queen/Amalgam roster keeps its own whole-roster gate and its own
    // two-kind follow-up table, unchanged: #2647 slice A widens Whistle by
    // ADDING the generic path below, never by relaxing this one.
    let queen_roster = matches!(
        monster.kind,
        MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
    );
    if !crate::engine::monsters::whistle_stun_target_is_representable(
        ctx.state,
        ctx.catalog,
        monster,
    ) {
        return Err(EngineRefusal::MalformedArgs("Whistle exact body"));
    }
    if (monster.slot, monster.uid) != frozen_target || monster.hp <= 0 || ctx.state.history.over {
        return Ok(());
    }
    if !queen_roster {
        // Whistle's stun IS `CreatureCmd::Stun(target, null)` — the doc above
        // already cites `0x132ae0 -> 0x3ed060 -> 0x132b2c`, the same entry
        // `ImbalancedPower` `0x33cd60` IL_006f takes. So it resolves its parked
        // successor out of the generated loop
        // (`crate::engine::monsters::parked_telegraph`) rather than from a
        // per-kind table, and refuses every kind whose telegraph that does not
        // uniquely determine.
        return crate::engine::monsters::install_generic_stun(ctx.state, target);
    }
    if monster.override_state == crate::hot::MonsterOverride::Stunned {
        return crate::engine::monsters::queen_amalgam_stun_state_is_exact(ctx.state, target)
            .then_some(())
            .ok_or(EngineRefusal::MalformedArgs("Whistle existing stun"));
    }
    if monster.override_state != crate::hot::MonsterOverride::None
        || monster.forced_follow_up != crate::hot::MonsterFollowUp::None
    {
        return Err(EngineRefusal::MalformedArgs("Whistle target state"));
    }
    let follow_up =
        crate::engine::monsters::queen_amalgam_follow_up(monster.kind, monster.loop_pos)
            .ok_or(EngineRefusal::MalformedArgs("Whistle target machine"))?;
    let monster = &mut ctx.state.monsters_mut()[target];
    monster.override_state = crate::hot::MonsterOverride::Stunned;
    monster.forced_follow_up = follow_up;
    if !crate::engine::monsters::queen_amalgam_stun_state_is_exact(ctx.state, target) {
        return Err(EngineRefusal::MalformedArgs("Whistle stun result"));
    }
    Ok(())
}

/// `tag_team_exact` — not modeled. **Escalated (#1331).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — the awaited attack, then
/// `_apply_tag_team_debuff` appends one non-stacking, applier-keyed
/// TagTeamPower instance to the surviving exact target.
///
/// Missing primitive: the **per-applier TagTeamPower instance list and its
/// reader** — the hot monster carries scalar power slots only, TagTeam is
/// outside [`crate::engine::admission::IMPLEMENTED_POWERS`], and nothing
/// engine-side consumes the instances. The attack half is [`player_attack`]
/// and already here.
/// ESCALATED-ON: power-variant-absent(PowerId::TagTeam)
pub(crate) fn tag_team_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::TagTeamExact))
}

/// Exact The Ball OnPlay body.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `TheBall/<OnPlay>d__10::MoveNext` RVA `0x3c27e4` awaits one attack using
/// the live Damage value at IL `0x0035-0x00cd`, then synchronously re-reads
/// Damage and Increase and appends Increase to both at IL `0x00d3-0x0124`.
/// Constructor/vars/upgrade RVAs `0xee4de/0xee505/0xee64b` pin the two rows
/// to `(10, 10)/(10, 15)`. Python `_run_steps_inner` (frozen, deleted #2827) preserves
/// the shared physical-body entry and live re-read; its current exact The Ball
/// branch reads `card_physical_state` and
/// `_card_add_damage_growth`. R53B updates that oracle to retain the
/// native post-Attack growth even when retaliation killed the player.
///
/// The body is independently exact from The Ball's virtual result-location
/// override (`GetResultLocationForCardPlay` RVA `0xee58c`; Python
/// `_resolve_card_play_result_location` (frozen, deleted #2827)), which consumes a Targets draw
/// before the route is frozen by the card wrapper. Current
/// `CardPileCmd::GiveToAnotherPlayer` RVA `0x13053c` and its async body RVA
/// `0x3e3f0c` own the later physical transfer.
fn the_ball_body_foundation(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    fn exact_catalog_closure(catalog: &Catalog) -> bool {
        [
            (0, [CompiledArg::I(10), CompiledArg::I(10)]),
            (1, [CompiledArg::I(10), CompiledArg::I(15)]),
        ]
        .into_iter()
        .all(|(upgrade, expected_args)| {
            let identity = CardIdentity {
                id: CardId::TheBall,
                upgrade,
                enchantment: None,
            };
            let Some(spec) = catalog.atom(&identity).and_then(|atom| catalog.spec(atom)) else {
                return false;
            };
            let [program] = catalog.steps(spec) else {
                return false;
            };
            spec.identity == identity
                && crate::content_tables::card_row(CardId::TheBall, upgrade) == Some(spec.row)
                && program.kind == StepKind::TheBallExact
                && catalog.args(program.args) == expected_args
        })
    }

    fn exact_source(ctx: &StepCtx<'_>) -> Result<HotCard, EngineRefusal> {
        let (pile, index) =
            crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
                EngineRefusal::ActiveCardNotUnique {
                    uid: ctx.source_uid,
                    matches: 0,
                },
            )?;
        let source = ctx.state.piles.get(pile).as_slice()[index];
        let identity = ctx
            .catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?
            .identity;
        if identity.id != CardId::TheBall
            || !matches!(identity.upgrade, 0 | 1)
            || !crate::engine::play::body_enchantment_is_exact(
                ctx.catalog
                    .spec(source.atom)
                    .ok_or(EngineRefusal::UnknownAtom(source.atom))?,
            )
            || source.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0
            || source.flags & !(CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE) != 0
        {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact physical source",
            ));
        }
        let instance = ctx.state.card_states.get(source.uid);
        if instance.damage_growth < 0
            || instance
                != (crate::hot::CardInstanceState {
                    damage_growth: instance.damage_growth,
                    ..crate::hot::CardInstanceState::default()
                })
        {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact physical payload",
            ));
        }
        Ok(source)
    }

    fn validate_entry(ctx: &StepCtx<'_>) -> Result<(HotCard, usize), EngineRefusal> {
        if !exact_catalog_closure(ctx.catalog) {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact catalog closure",
            ));
        }
        let expected_args: &[CompiledArg] = match ctx.spec.identity.upgrade {
            0 => &[CompiledArg::I(10), CompiledArg::I(10)],
            1 => &[CompiledArg::I(10), CompiledArg::I(15)],
            _ => return Err(EngineRefusal::MalformedArgs("the_ball_exact")),
        };
        let canonical_spec = ctx
            .catalog
            .atom(&ctx.spec.identity)
            .and_then(|atom| ctx.catalog.spec(atom))
            .ok_or(EngineRefusal::MalformedArgs(
                "the_ball_exact catalog identity",
            ))?;
        if canonical_spec != ctx.spec {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact canonical spec",
            ));
        }
        let [program] = ctx.catalog.steps(canonical_spec) else {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact canonical program",
            ));
        };
        if ctx.spec.identity.id != CardId::TheBall
            || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
            || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
                != Some(ctx.spec.row)
            || program.kind != StepKind::TheBallExact
            || ctx.catalog.args(program.args) != expected_args
            || ctx.args != expected_args
            || ctx.selection.is_some()
            || ctx.x_value != 0
            || ctx.state.hp <= 0
            || ctx.state.multiplayer_ally_key != 1
        {
            return Err(EngineRefusal::MalformedArgs("the_ball_exact"));
        }
        let ally = ctx.state.fanouts.multiplayer_ally();
        if ally.key != 1 || !ally.alive {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact teammate provenance",
            ));
        }
        let target = ctx
            .target
            .ok_or(EngineRefusal::TargetMismatch { required: true })?;
        let Some(target_monster) = ctx
            .state
            .monsters
            .get(target)
            .filter(|monster| monster.hp > 0)
        else {
            return Err(EngineRefusal::BadTarget(
                u8::try_from(target).unwrap_or(u8::MAX),
            ));
        };
        let Some((Some(_), Some(frame_target))) =
            crate::engine::play::active_card_current_context(ctx.source_uid)
        else {
            return Err(EngineRefusal::ContinuationNotModeled);
        };
        if frame_target != (target_monster.slot, target_monster.uid) {
            return Err(EngineRefusal::MalformedArgs(
                "the_ball_exact target identity",
            ));
        }
        Ok((exact_source(ctx)?, target))
    }

    fn apply(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
        let (source, target) = validate_entry(ctx)?;
        let growth = ctx.state.card_states.get(source.uid).damage_growth;
        player_attack_from_card(
            ctx.state,
            (ctx.catalog, ctx.spec, ctx.source_uid),
            &[target],
            10_i64
                .checked_add(i64::from(growth))
                .ok_or(EngineRefusal::CounterOverflow("the_ball_exact damage"))?,
            1,
            ctx.events,
        )?;
        let source = exact_source(ctx)?;
        let live_upgrade = ctx
            .catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?
            .identity
            .upgrade;
        let increase = match live_upgrade {
            0 => 10,
            1 => 15,
            _ => return Err(EngineRefusal::MalformedArgs("the_ball_exact live level")),
        };
        ctx.state
            .card_states
            .add_damage_growth(source.uid, increase)
            .ok_or(EngineRefusal::CounterOverflow(
                "the_ball_exact damage growth",
            ))?;
        Ok(())
    }

    // The post-attack live-source re-read and checked growth are both late.
    // Rehearse the complete continuation so a refusal there cannot publish
    // damage, listener/history state, RNG, or event prefixes to a direct
    // private caller.
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
    apply(&mut probe_ctx)?;
    apply(ctx)
}

pub(crate) fn the_ball_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    the_ball_body_foundation(ctx)
}

/// `the_bomb_exact` — allocate one independent current-build Bomb instance.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) — `_apply_before_side_turn_end_player_power`: register a countdown listener that detonates for the bomb
/// damage at the owner's side end.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `TheBombPower.get_Type` RVA `0xa9ed7`, `get_StackType` RVA `0xa9eda`, and
/// `get_InstanceType` RVA `0xa9edd` pin Buff/Counter/**Instanced**. Therefore
/// `PowerCmd.FindExistingInstanceForStacking` RVA `0x1338d8` never reuses an
/// older Bomb. `TheBomb/<OnPlay>d__4::MoveNext` RVA `0x3c2968` awaits the
/// Amount-3 Apply, then writes 40/50 to that returned instance's private
/// `BombDamage` field.
///
/// Current authority allocates one stable UID, appends its typed token at
/// acquisition position, and publishes Apply, and only then records the
/// per-instance damage payload. The cold Rust sidecar preserves the same
/// transient ordering inside the surrounding whole-card transaction.
pub(crate) fn the_bomb_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (turns, damage) = two_ints(ctx, "the_bomb_exact")?;
    let expected_damage = match ctx.spec.identity.upgrade {
        0 => 40,
        1 => 50,
        _ => return Err(EngineRefusal::MalformedArgs("the_bomb_exact")),
    };
    if ctx.spec.identity.id != CardId::TheBomb
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || turns != 3
        || damage != expected_damage
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("the_bomb_exact"));
    }
    exact_active_neutral_source(ctx, CardId::TheBomb, "the_bomb_exact source")?;
    if ctx.state.multiplayer_ally_key != 0
        || !ctx.state.fanouts.the_bomb_state_is_exact()
        || ctx.state.powers.get(PowerId::TheBomb).is_some()
        || !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("the_bomb_exact state"));
    }
    let hailstorm_tokens = ctx
        .state
        .fanouts
        .before_side_turn_end_order()
        .iter()
        .filter(|token| matches!(token, crate::hot::BeforeSideTurnEndToken::Hailstorm))
        .count();
    if hailstorm_tokens != usize::from(ctx.state.powers.value(PowerId::Hailstorm) > 0) {
        return Err(EngineRefusal::MalformedArgs("the_bomb_exact order"));
    }
    if ctx.state.history.over {
        return Ok(());
    }
    let uid = ctx
        .state
        .fanouts
        .begin_the_bomb(turns as i32)
        .map_err(|_| EngineRefusal::CounterOverflow("next_the_bomb_uid"))?;
    crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::TheBomb, turns as i32);
    ctx.state
        .fanouts
        .set_the_bomb_damage(uid, damage as i32)
        .map_err(|_| EngineRefusal::MalformedArgs("the_bomb_exact damage"))?;
    Ok(())
}

/// `toric_toughness_exact` — full retained-Decimal block and delayed refresh.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// Card constructor/canonical vars/OnPlay/upgrade are RVAs
/// `0xeefb1/0xeefc1/0xef004/0xef057`; exact OnPlay coroutine `0x3c403c`
/// awaits powered `GainBlock`, retaining its returned Decimal, then awaits an
/// ending-gated amount-2 `PowerCmd.Apply<ToricToughnessPower>` and overwrites
/// that returned keyed instance's Block var. Power type/stack/instance,
/// canonical vars, SetBlock, AfterBlockCleared, and ctor are
/// `0xaa233/0xaa236/0xaa24b/0xaa24e/0xaa260/0xaa27c/0xaa2c7`; callback
/// coroutine `0x349e3c` awaits flat unpowered GainBlock before decrement.
///
/// Python authority: `_run_steps_inner` (frozen Python, deleted #2827) validates the body,
/// `_gain_powered_card_block_retained_decimal` returns the full
/// Decimal, and `_apply_toric_toughness_power` owns the delayed keyed
/// lifecycle. The Rust cold tagged record preserves the full Decimal while
/// Block/history truncate independently.
pub(crate) fn toric_toughness_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (block, duration) = two_ints(ctx, "toric_toughness_exact")?;
    let expected_block = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::ToricToughness, 0) => 5,
        (CardId::ToricToughness, 1) => 7,
        _ => return Err(EngineRefusal::MalformedArgs("toric_toughness_exact")),
    };
    if block != expected_block
        || duration != 2
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || ctx.state.multiplayer_ally_key != 0
        || !crate::engine::admission::delayed_block_power_state_is_exact(ctx.state)
        || !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("toric_toughness_exact"));
    }
    exact_active_neutral_source(ctx, CardId::ToricToughness, "toric_toughness_exact source")?;

    fn apply(ctx: &mut StepCtx<'_>, block: i64, duration: i32) -> Result<(), EngineRefusal> {
        let retained = gain_powered_card_block_retained_decimal(
            ctx.state,
            ctx.catalog,
            ctx.spec,
            block,
            ctx.events,
        )?;
        fire_hook(
            ctx.catalog,
            HookEvent::AfterBlockGained,
            ctx.state,
            ctx.events,
        )?;
        if ctx.state.history.over {
            return Ok(());
        }
        // #3057: ToricToughnessPower is Instanced (`get_InstanceType` RVA
        // `0xaa24b` IL_0001 returns 1), so `FindExistingInstanceForStacking`
        // (RVA `0x1338d8` IL_0021 -> IL_0034) answers null and this Apply
        // attaches a SECOND object. Each object's `AfterBlockCleared`
        // (`<AfterBlockCleared>d__11::MoveNext` RVA `0x349e3c`) gains its own
        // Block var (IL_003d-0050) and decrements its own duration
        // (IL_00ab-00ac). The single keyed record would add the durations
        // and overwrite the Block, so a second live object refuses by name.
        if ctx.state.fanouts.toric_toughness().is_some() {
            return Err(EngineRefusal::PowerRestackNotModeled(
                PowerId::ToricToughness,
            ));
        }
        let updated = ctx
            .state
            .fanouts
            .apply_toric_toughness(duration, retained)
            .map_err(|_| EngineRefusal::PowerOrderNotModeled("after-block-cleared power order"))?;
        crate::engine::damage::note_power(
            ctx.events,
            Subject::Player,
            PowerId::ToricToughness,
            updated,
        );
        Ok(())
    }

    let duration: i32 = duration
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("toric_toughness_exact"))?;
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
    apply(&mut probe_ctx, block, duration)?;
    apply(ctx, block, duration)
}

const APOTHEOSIS_PILE_ORDER: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

fn apotheosis_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    if spec.identity.id != CardId::Apotheosis
        || !matches!(spec.identity.upgrade, 0 | 1)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return false;
    }
    let [upgrade] = catalog.steps(spec) else {
        return false;
    };
    upgrade.kind == StepKind::UpgradeAllCombatExceptSourceExact
        && catalog.args(upgrade.args).is_empty()
}

fn apotheosis_frozen_upgrade_uids(ctx: &StepCtx<'_>) -> Result<Vec<u32>, EngineRefusal> {
    if !ctx.args.is_empty()
        || !apotheosis_program_is_exact(ctx.catalog, ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "upgrade_all_combat_except_source_exact",
        ));
    }
    exact_active_neutral_source(
        ctx,
        CardId::Apotheosis,
        "upgrade_all_combat_except_source_exact active source",
    )?;
    let source = APOTHEOSIS_PILE_ORDER
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .find(|card| card.uid == ctx.source_uid)
        .expect("the unique source check proved one card");
    if source.flags & CARD_FLAG_LEGACY != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "upgrade_all_combat_except_source_exact physical source",
        ));
    }
    Ok(APOTHEOSIS_PILE_ORDER
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .filter(|card| card.uid != ctx.source_uid)
        .map(|card| card.uid)
        .collect())
}

/// Validate Apotheosis's complete five-pile writer before a public play can
/// publish Energy, source movement, listeners, or a CardPlayed event.
///
/// The incoming physical source is still in its caller-owned pile at this
/// seam. Excluding it by uid and rehearsing the native Hand/Draw/Discard/
/// Exhaust/Play walk on a clone proves every next atom and every live uid
/// before the first externally visible prefix.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn preflight_apotheosis_upgrade(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::apotheosis_incoming_context_is_exact(state, catalog, source_uid) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut events = Vec::new();
    let mut probe = state.clone();
    let ctx = StepCtx {
        state: &mut probe,
        catalog,
        spec,
        source_uid,
        target: None,
        selection: None,
        x_value: 0,
        args: &[],
        events: &mut events,
    };
    let frozen_uids = apotheosis_frozen_upgrade_uids(&ctx)?;
    upgrade_live_cards_once(ctx.state, catalog, &frozen_uids)
}

/// `upgrade_all_combat_except_source_exact` — Apotheosis's exact synchronous
/// all-card upgrade pass.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) dispatches to
/// `_upgrade_all_combat_except_source_exact` for the synchronous
/// five-pile, physical-source-excluding rewrite used by the differential
/// oracle.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Apotheosis.OnPlay` RVA `0xd7ce8` enumerates
/// `PlayerCombatState.AllCards`, reference-excludes only `this`, and invokes
/// one style-1 `CardCmd.Upgrade` for each upgradable survivor.
/// `PlayerCombatState.get_AllPiles` RVA `0x117de8` fixes Hand, Draw,
/// Discard, Exhaust, Play order; `get_AllCards` RVA `0x117e3c` preserves that
/// outer order and each pile's live order. The shared one-level writer owns
/// ending suppression, atomic next-atom resolution, local-Set clamping, uid
/// and payload preservation, and exact-pile promotion.
pub(crate) fn upgrade_all_combat_except_source_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::apotheosis_body_context_is_exact(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
    ) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let frozen_uids = apotheosis_frozen_upgrade_uids(ctx)?;
    upgrade_live_cards_once(ctx.state, ctx.catalog, &frozen_uids)
}

/// `mad_science_chaos_exact` — Mad Science's Chaos rider (#3322): one card of
/// the owner's unlocked pool, free this turn, into Hand.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// re-read for #3322 with `dump_type.py MadScience "<ExecuteRider>d__57"`):
///
/// * `MadScience/<OnPlay>d__51::MoveNext` RVA `0x3aaf08` runs the type body
///   first (Skill: `<ExecuteSkill>d__53` RVA `0x3aae28`, `GainBlock` of the
///   Block var, the row's leading `block` step) and then `ExecuteRider`, so
///   this step follows the Block;
/// * `<ExecuteRider>d__57::MoveNext` RVA `0x3aa9c4` switches on `rider - 1`
///   (IL_003f-IL_0042; Chaos is the sixth arm). The Chaos arm
///   (IL_031e-IL_03f0) takes `get_MockedChaosCard` (IL_031f-IL_032c) only
///   when a test harness set it — no game path does — and otherwise reads
///   `Owner.Character.CardPool` (IL_032f-IL_0340),
///   `GetUnlockedCards(Owner.UnlockState, RunState.CardMultiplayerConstraint)`
///   (IL_0345-IL_0360), then `CardFactory.GetDistinctForCombat(owner, cards,
///   1, RunRngSet.CombatCardGeneration)` (IL_0365-IL_037b) and keeps its first
///   result (IL_0380-IL_0385);
/// * it calls `SetToFreeThisTurn` on that card (IL_0386-IL_0387) and awaits
///   one `CardPileCmd.AddGeneratedCardToCombat(card, 2, owner, 1)`
///   (IL_038c-IL_0395): `PileType` 2 is Hand, at the Bottom.
///
/// That is Discovery's draw (`Discovery/<OnPlay>d__4::MoveNext` RVA
/// `0x399254`: the same unlocked owner pool and `GetDistinctForCombat`, count
/// 3) followed by Distraction's tail (`Distraction/<OnPlay>d__5::MoveNext`
/// RVA `0x399690` IL_009e-IL_00ad: the same `SetToFreeThisTurn` and
/// `AddGeneratedCardToCombat(card, 2, owner, 1)`), with no card-type
/// predicate. So the pool is Discovery's ([`Catalog::owner_generation_pool`],
/// the owner's recorded-profile pool after `FilterForCombat`), the provenance
/// is Discovery's
/// ([`crate::engine::cards::owner_pool_generation_provenance_is_exact`] plus
/// its Regent pool guard), and the draw is live: the boundary interns the
/// owner pool's complete closure beside a Chaos Mad Science, so every stream
/// position mints a known atom.
pub(crate) fn mad_science_chaos_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || !crate::catalog::is_mad_science_variant_program(ctx.spec, "Chaos")
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !crate::engine::cards::owner_pool_generation_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("mad_science_chaos_exact"));
    }
    exact_active_neutral_source(ctx, CardId::MadScience, "mad_science_chaos_exact source")?;
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs(
            "mad_science_chaos_exact owner",
        ))?;
    if owner == RewardPool::Regent && !ctx.state.spectrum_shift_generation_pool() {
        return Err(EngineRefusal::MalformedArgs(
            "mad_science_chaos_exact Regent pool provenance",
        ));
    }
    let pool = ctx.catalog.owner_generation_pool(owner);
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs("mad_science_chaos_exact pool"));
    }

    fn apply(ctx: &mut StepCtx<'_>, pool: &[CardId]) -> Result<(), EngineRefusal> {
        let shuffled = crate::engine::cards::shuffle_generation_slice(ctx.state, pool)?;
        crate::engine::cards::inject_generated_free_this_turn_bottom(
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

    // The minted identity or a generated listener can still refuse after the
    // shuffle; rehearse on a clone so a refusal leaves the real state
    // untouched (Distraction's transaction, `silent_special::distraction_body`).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CardAtom, CardEnchantment, CardIdentity, Catalog, CatalogBuilder, CompiledArg,
    };
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::admission::IMPLEMENTED_POWERS;
    use crate::engine::play::{
        autoplay_collected_cards, autoplay_draw_top, autoplay_imitation_clone, play_card,
    };
    use crate::engine::{Action, SelectionRef, apply_action_into};
    use crate::hot::{CARD_FLAG_RINGING, HotCard, HotState, MiseryToken};
    use crate::ids::EnchantmentId;
    use crate::powers::SlotWire;
    use crate::steps::apply_step;

    /// Every kind this file owns a body for, ported or not.
    ///
    /// The dispatch's own completeness and uniqueness are pinned by
    /// `tests/hot_path_contract.rs`; what this list adds is the split between
    /// claimed and escalated, so that porting a kind means moving it into
    /// [`IMPLEMENTED`] in one deliberate edit.
    const OWNED: [StepKind; 38] = [
        StepKind::AbundanceExact,
        StepKind::AlchemizeExact,
        StepKind::BeaconOfHopeExact,
        StepKind::BeatDownExact,
        StepKind::BelieveInYouExact,
        StepKind::BrightestFlameExact,
        StepKind::CalamityExact,
        StepKind::CatastropheExact,
        StepKind::CoordinateExact,
        StepKind::DiscoveryExact,
        StepKind::DrawIfNoHandAttacks,
        StepKind::EidolonExact,
        StepKind::EnlightenmentExact,
        StepKind::GainCurrentBlockCardUnpoweredExact,
        StepKind::GangUpExact,
        StepKind::HiddenGemExact,
        StepKind::InterceptExact,
        StepKind::JackOfAllTradesExact,
        StepKind::JackpotExact,
        StepKind::LiftExact,
        StepKind::MadScienceChaosExact,
        StepKind::MetamorphosisExact,
        StepKind::MimicExact,
        StepKind::Nostalgia,
        StepKind::PanicButtonExact,
        StepKind::PurityExact,
        StepKind::RallyExact,
        StepKind::Rebound,
        StepKind::RendExact,
        StepKind::RestlessnessExact,
        StepKind::SeekerStrikeExact,
        StepKind::SplashExact,
        StepKind::StunTarget,
        StepKind::TagTeamExact,
        StepKind::TheBallExact,
        StepKind::TheBombExact,
        StepKind::ToricToughnessExact,
        StepKind::UpgradeAllCombatExceptSourceExact,
    ];

    /// #2560/#2946: Distraction's and White Noise's frozen tables are the
    /// one-type projection of the owner pool at the fully-unlocked profile,
    /// card for card and in native (RNG-significant) order — the licence for
    /// drawing the derived pool under a recorded partial profile instead.
    #[test]
    fn one_type_owner_pools_reproduce_the_frozen_constants() {
        use crate::content_tables::CardType;
        assert_eq!(
            crate::content_tables::DISTRACTION_SKILL_POOL_V1101.as_slice(),
            owner_type_generation_pool(RewardPool::Silent, None, CardType::Skill).as_slice(),
            "DISTRACTION_SKILL_POOL_V1101 drifted"
        );
        for (name, pool) in crate::content_tables::ABUNDANCE_POWER_POOLS_V1101 {
            let owner = RewardPool::from_str(name).expect("a known owner");
            assert_eq!(
                pool,
                owner_type_generation_pool(owner, None, CardType::Power).as_slice(),
                "ABUNDANCE_POWER_POOLS_V1101[{name}] drifted"
            );
        }
        // A profile gating a Defect row removes exactly that owner's gated
        // rows and leaves order otherwise intact; another character's gate is
        // inert for the Defect pool (the #2560 claim).
        let rows = crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101
            [character_pool_index(RewardPool::Defect)]
        .1;
        for card_type in [CardType::Skill, CardType::Power] {
            let full = owner_type_generation_pool(RewardPool::Defect, None, card_type);
            let mut profile: Vec<&'static str> = crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .copied()
                .filter(|epoch| !matches!(*epoch, "DEFECT5_EPOCH" | "DEFECT7_EPOCH"))
                .collect();
            profile.sort_unstable();
            let partial = owner_type_generation_pool(RewardPool::Defect, Some(&profile), card_type);
            let expected: Vec<CardId> = full
                .iter()
                .copied()
                .filter(|id| {
                    !rows.iter().any(|(row, epoch)| {
                        row == id && matches!(*epoch, Some("DEFECT5_EPOCH" | "DEFECT7_EPOCH"))
                    })
                })
                .collect();
            assert_eq!(partial, expected, "{card_type:?}");
            assert!(
                partial.len() < full.len(),
                "{card_type:?}: the witness must gate a row"
            );
            let mut other: Vec<&'static str> = crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .copied()
                .filter(|epoch| *epoch != "SILENT7_EPOCH")
                .collect();
            other.sort_unstable();
            assert_eq!(
                owner_type_generation_pool(RewardPool::Defect, Some(&other), card_type),
                full,
                "{card_type:?}: a foreign gate moved the Defect pool"
            );
        }
    }

    /// #2970: Crossbow's owner Attack pool is the frozen Ironclad table at the
    /// fully-unlocked profile, card for card in native order, and a profile
    /// gates only the owner's own epoch rows — the licence for drawing it for
    /// any owner under any recorded profile.
    #[test]
    fn crossbow_owner_pools_reproduce_the_frozen_table() {
        assert_eq!(
            crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109.as_slice(),
            crossbow_owner_attack_pool(RewardPool::Ironclad, None).as_slice(),
        );
        for owner in [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Regent,
            RewardPool::Necrobinder,
            RewardPool::Defect,
        ] {
            let full = crossbow_owner_attack_pool(owner, None);
            assert!(!full.is_empty(), "{owner:?}");
            assert!(
                full.iter().all(|id| crate::content_tables::card_row(*id, 0)
                    .is_some_and(|row| row.card_type == crate::content_tables::CardType::Attack)),
                "{owner:?}"
            );
            // A foreign character's epoch is inert: the 1KJJGR1GFZR6 profile
            // hides only Defect epochs, and its Ironclad pool is the full one.
            let mut foreign: Vec<&'static str> = crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .copied()
                .filter(|epoch| owner == RewardPool::Defect || !epoch.starts_with("DEFECT"))
                .collect();
            foreign.sort_unstable();
            assert_eq!(crossbow_owner_attack_pool(owner, Some(&foreign)), full);
        }
        // An own-epoch gate removes exactly that owner's gated Attack rows.
        let rows = crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101
            [character_pool_index(RewardPool::Ironclad)]
        .1;
        let full = crossbow_owner_attack_pool(RewardPool::Ironclad, None);
        let gated: Vec<&'static str> = rows
            .iter()
            .filter(|(id, epoch)| epoch.is_some() && full.contains(id))
            .filter_map(|(_, epoch)| *epoch)
            .collect();
        assert!(!gated.is_empty(), "the witness must gate an Attack row");
        let mut profile: Vec<&'static str> = crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .copied()
            .filter(|epoch| !gated.contains(epoch))
            .collect();
        profile.sort_unstable();
        let partial = crossbow_owner_attack_pool(RewardPool::Ironclad, Some(&profile));
        let expected: Vec<CardId> = full
            .iter()
            .copied()
            .filter(|id| {
                !rows
                    .iter()
                    .any(|(row, epoch)| row == id && epoch.is_some_and(|e| gated.contains(&e)))
            })
            .collect();
        assert_eq!(partial, expected);
        assert!(partial.len() < full.len());
    }

    /// #2542: every frozen owner-pool table is one projection of the five
    /// generated character pools at the fully-unlocked profile.
    ///
    /// That equality is the whole licence for deriving Necrobinder, Defect
    /// and Silent instead of refusing them, so it is pinned card for card and
    /// in native order — the order is RNG-significant.
    #[test]
    fn character_pools_reproduce_the_frozen_constants() {
        use crate::content_tables::CardType;
        let cases: [(&str, &[CardId], Vec<CardId>); 6] = [
            (
                "STOKE_CARD_POOL_V109",
                &crate::content_tables::STOKE_CARD_POOL_V109,
                owner_generation_pool(RewardPool::Ironclad, None),
            ),
            (
                "REGENT_CARD_GENERATION_POOL_V1101",
                &crate::content_tables::REGENT_CARD_GENERATION_POOL_V1101,
                owner_generation_pool(RewardPool::Regent, None),
            ),
            (
                "INFERNAL_BLADE_ATTACK_POOL_V109",
                &crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109,
                calamity_owner_attack_pool(RewardPool::Ironclad, None),
            ),
            (
                "REGENT_ATTACK_GENERATION_POOL_V1101",
                &crate::content_tables::REGENT_ATTACK_GENERATION_POOL_V1101,
                calamity_owner_attack_pool(RewardPool::Regent, None),
            ),
            (
                "JACKPOT_ZERO_COST_POOL_V109",
                &crate::content_tables::JACKPOT_ZERO_COST_POOL_V109,
                owner_zero_cost_pool(RewardPool::Ironclad, None),
            ),
            (
                "REGENT_ZERO_COST_GENERATION_POOL_V1101",
                &crate::content_tables::REGENT_ZERO_COST_GENERATION_POOL_V1101,
                owner_zero_cost_pool(RewardPool::Regent, None),
            ),
        ];
        for (name, frozen, derived) in cases {
            assert_eq!(frozen, derived.as_slice(), "{name} drifted");
        }
        for (name, pool) in crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101 {
            let owner = RewardPool::from_str(name).expect("a known owner");
            assert_eq!(
                pool,
                metamorphosis_owner_attack_pool(owner, None).as_slice(),
                "METAMORPHOSIS_ATTACK_POOLS_V1101[{name}] drifted"
            );
        }
        // And the owner axis really is closed: no projection is empty.
        for owner in RewardPool::ALL {
            assert!(!owner_generation_pool(owner, None).is_empty());
            assert!(!owner_zero_cost_pool(owner, None).is_empty());
            assert!(!calamity_owner_attack_pool(owner, None).is_empty());
            assert!(!metamorphosis_owner_attack_pool(owner, None).is_empty());
        }
        // The two rarity comparisons are genuinely different sets: the
        // `Common..Rare` one also excludes Status/Token/Curse/Quest, which
        // `FilterForCombat` does not. Conflating them would be a silent
        // widening of every potion partition.
        use crate::content_tables::CardRarity;
        for rarity in [CardRarity::Status, CardRarity::Token, CardRarity::Curse] {
            assert!(!PoolRarity::CommonUncommonRare.admits(rarity));
            assert!(PoolRarity::NotBasicAncientEvent.admits(rarity));
        }
        for rarity in [CardRarity::Basic, CardRarity::Ancient, CardRarity::Event] {
            assert!(!PoolRarity::CommonUncommonRare.admits(rarity));
            assert!(!PoolRarity::NotBasicAncientEvent.admits(rarity));
        }
        let _ = CardType::Power;
    }

    #[test]
    fn splash_owner_pools_preserve_native_order_and_exclude_only_the_owner() {
        use sha2::{Digest, Sha256};

        assert_eq!(
            crate::content_tables::METAMORPHOSIS_ATTACK_POOLS_V1101
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            ["Defect", "Ironclad", "Necrobinder", "Regent", "Silent"]
        );
        for (owner, expected_len) in [
            (RewardPool::Defect, 117),
            (RewardPool::Ironclad, 112),
            (RewardPool::Necrobinder, 113),
            (RewardPool::Regent, 116),
            (RewardPool::Silent, 122),
        ] {
            let actual = splash_pool(owner);
            let expected: Vec<_> = [
                RewardPool::Ironclad,
                RewardPool::Silent,
                RewardPool::Regent,
                RewardPool::Necrobinder,
                RewardPool::Defect,
            ]
            .into_iter()
            .filter(|candidate| *candidate != owner)
            .flat_map(owner_attack_pool)
            .copied()
            .collect();
            assert_eq!(actual, expected, "native owner order for {owner:?}");
            assert_eq!(actual.len(), expected_len, "pool size for {owner:?}");
            assert!(
                actual
                    .iter()
                    .all(|id| !owner_attack_pool(owner).contains(id)),
                "the source owner's pool is excluded for {owner:?}",
            );
        }
        assert_eq!(
            splash_pool(RewardPool::Ironclad),
            crate::content_tables::SPLASH_ATTACK_POOL_V109
        );

        for (owner, expected_hash, expected_offers, expected_draws) in [
            (
                RewardPool::Ironclad,
                "8a08c2b2c347005e13d6e60904a65d997d5728fd79dbefcec2787fb4c0897e3c",
                [
                    CardId::BallLightning,
                    CardId::SculptingStrike,
                    CardId::MakeItSo,
                ],
                111,
            ),
            (
                RewardPool::Regent,
                "379c350e17fa58beaf6992de4807c3c38d3aa50a8a44e38a23f7d81c99acd545",
                [CardId::HelixDrill, CardId::Thunderclap, CardId::DaggerThrow],
                115,
            ),
        ] {
            let pool = splash_pool(owner);
            let authority_ids = pool
                .iter()
                .map(|id| format!("CARD.{}", id.as_str()))
                .collect::<Vec<_>>();
            let compact = serde_json::to_vec(&authority_ids).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(compact)), expected_hash);

            let mut rng = Xoshiro256StarStar::from_seed(17);
            let initial_counter = rng.counter;
            let mut shuffled = pool;
            rng.shuffle(&mut shuffled).unwrap();
            assert_eq!(shuffled[..3], expected_offers);
            assert_eq!(rng.counter - initial_counter, expected_draws);
        }
    }

    #[test]
    fn calamity_writer_pins_both_rows_and_complete_owner_pool() {
        for upgrade in 0..=1 {
            let source = CardIdentity {
                id: CardId::Calamity,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let source_atom = builder.intern(source).unwrap();
            for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109 {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            let catalog = builder.build();
            let spec = *catalog.spec(source_atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.reward_card_pool = Some(RewardPool::Ironclad);
            state.entropy_card_pool = Some(RewardPool::Ironclad);
            state.fully_unlocked_card_pool_epochs = true;
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 17,
                atom: source_atom,
                flags: 0,
            });
            for (amount, hook_live) in [(0, true), (1, false)] {
                let mut malformed = state.clone();
                if amount > 0 {
                    malformed
                        .powers
                        .set(PowerId::Calamity, SlotWire::Int, amount);
                }
                malformed.set_calamity_hook_live(hook_live);
                let before = malformed.clone();
                let mut malformed_events = Vec::new();
                let mut ctx = StepCtx {
                    state: &mut malformed,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: 17,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[],
                    events: &mut malformed_events,
                };
                assert_eq!(
                    calamity_exact(&mut ctx),
                    Err(EngineRefusal::MalformedArgs(
                        "Calamity power/listener mismatch"
                    ))
                );
                assert_eq!(malformed, before);
                assert!(malformed_events.is_empty());
            }
            let mut events = Vec::new();
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

            calamity_exact(&mut ctx).unwrap();
            assert_eq!(state.powers.value(PowerId::Calamity), 1);
            assert!(state.calamity_hook_is_live());

            state.powers.set(
                PowerId::Calamity,
                SlotWire::Int,
                MAX_CALAMITY_GENERATED_PER_PLAY,
            );
            events.clear();
            let before = state.clone();
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
                calamity_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs(
                    "Calamity generation batch exceeds 256 cards"
                ))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    /// The kinds still waiting on an engine primitive or an admissible
    /// source, per each stub's own note.
    const ESCALATED: [StepKind; 1] = [StepKind::TagTeamExact];

    const STRIKE: CardIdentity = CardIdentity {
        id: CardId::StrikeIronclad,
        upgrade: 0,
        enchantment: None,
    };
    const DEFEND: CardIdentity = CardIdentity {
        id: CardId::DefendIronclad,
        upgrade: 0,
        enchantment: None,
    };

    fn hidden_gem_catalog() -> (Catalog, CardAtom, CardAtom, CardAtom, CardAtom, CardAtom) {
        let mut builder = CatalogBuilder::new();
        let hidden = builder
            .intern_reachable(CardIdentity {
                id: CardId::HiddenGem,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder.intern_reachable(DEFEND).unwrap();
        let strike = builder.intern_reachable(STRIKE).unwrap();
        let sloth = builder
            .intern_reachable(CardIdentity {
                id: CardId::Sloth,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let toxic = builder
            .intern_reachable(CardIdentity {
                id: CardId::Toxic,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        (builder.build(), hidden, defend, strike, sloth, toxic)
    }

    fn seed_zero_sel() -> RngStreamState {
        RngStreamState {
            words: [
                16_294_208_416_658_607_535,
                7_960_286_522_194_355_700,
                487_617_019_471_545_679,
                17_909_611_376_780_542_444,
            ],
            counter: 0,
        }
    }

    #[test]
    fn hidden_gem_prefers_native_types_and_seed_zero_selects_exact_uid() {
        let (catalog, hidden, defend, strike, _, _) = hidden_gem_catalog();
        let source = HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        };
        let draw = [
            HotCard {
                uid: 11,
                atom: defend,
                flags: 0,
            },
            HotCard {
                uid: 12,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 13,
                atom: strike,
                flags: 0,
            },
        ];
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 14;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().extend(draw);

        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [11, 12, 13]
        );
        assert_eq!(state.card_states.get(11).base_replay_count(), None);
        assert_eq!(state.card_states.get(12).base_replay_count(), Some(2));
        assert_eq!(state.card_states.get(13).base_replay_count(), None);
    }

    #[test]
    fn hidden_gem_status_fallback_ignores_can_play_and_singleton_draws_once() {
        let (catalog, hidden, _, _, sloth, toxic) = hidden_gem_catalog();
        assert!(!catalog.spec(sloth).unwrap().playable);
        assert_eq!(catalog.spec(sloth).unwrap().card_type, CardType::Status);
        let source = HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 13;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 11,
                atom: sloth,
                flags: 0,
            },
            HotCard {
                uid: 12,
                atom: toxic,
                flags: 0,
            },
        ]);

        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(state.card_states.get(11).base_replay_count(), None);
        assert_eq!(state.card_states.get(12).base_replay_count(), Some(2));
    }

    #[test]
    fn hidden_gem_missing_sel_refuses_whole_play_atomically() {
        let (catalog, hidden, _, strike, _, _) = hidden_gem_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 12;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 11,
            atom: strike,
            flags: 0,
        });
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            play_card(&mut state, &catalog, 10, None, None, &mut events),
            Err(EngineRefusal::MalformedArgs("Hidden Gem Sel stream"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let mut forged = before;
        forged.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: [0; 4],
                counter: 1,
            },
        );
        let forged_before = forged.clone();
        let mut forged_events = Vec::new();
        assert_eq!(
            play_card(&mut forged, &catalog, 10, None, None, &mut forged_events,),
            Err(EngineRefusal::MalformedArgs("Hidden Gem Sel stream"))
        );
        assert_eq!(forged, forged_before);
        assert!(forged_events.is_empty());
    }

    #[test]
    fn hidden_gem_empty_draw_is_an_exact_zero_rng_no_op() {
        let (catalog, hidden, _, _, _, _) = hidden_gem_catalog();
        let source = HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 11;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(source);

        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.rng.get(RngStream::Sel).counter, 0);
        assert!(state.card_states.is_empty());
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), [source]);
    }

    #[test]
    fn hidden_gem_excludes_fresh_glam_and_spiral_but_preserves_spent_glam() {
        let hidden_identity = identity(CardId::HiddenGem, 0);
        let glam_identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let spiral_identity = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Spiral,
                amount: 99,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let hidden = builder.intern_reachable(hidden_identity).unwrap();
        let glam = builder.intern_reachable(glam_identity).unwrap();
        let spiral = builder.intern_reachable(spiral_identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 14;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 11,
                atom: glam,
                flags: 0,
            },
            HotCard {
                uid: 12,
                atom: spiral,
                flags: 0,
            },
            HotCard {
                uid: 13,
                atom: glam,
                flags: 0,
            },
        ]);
        let mut spent = state.card_states.get(13);
        spent.enchantment_state = crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
        state.card_states.set(13, spent);

        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(state.card_states.get(11).base_replay_count(), None);
        assert_eq!(state.card_states.get(12).base_replay_count(), None);
        assert_eq!(state.card_states.get(13).base_replay_count(), Some(2));
        assert_eq!(state.card_states.get(13).enchantment_state.get(), Some(1));
    }

    /// #3135: Hidden Gem reads Glam Powers through the same predicate the
    /// widened Glam arm feeds. Native filter `<>c::<OnPlay>b__8_0` RVA
    /// `0x3a56e8` keeps a card only when it is not Unplayable (IL_000c-0019),
    /// not Curse/Quest (IL_0021-002c) and `GetEnchantedReplayCount() < 1`
    /// (IL_003c-0043); the preferred filter `b__8_1` RVA `0x3a5730` is
    /// `Type - 1 <= 2` (Attack, Skill, Power; IL_000c-0017). So a fresh Glam
    /// Power (count 1) is excluded, and a spent one (count 0) is a preferred
    /// Power candidate. A draw of only fresh Glam Powers is a zero-RNG no-op.
    #[test]
    fn hidden_gem_treats_glam_powers_by_their_enchanted_replay_count() {
        let glam_inflame = CardIdentity {
            id: CardId::Inflame,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let hidden = builder
            .intern_reachable(identity(CardId::HiddenGem, 0))
            .unwrap();
        let glam = builder.intern_reachable(glam_inflame).unwrap();
        let catalog = builder.build();
        for with_spent in [false, true] {
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 13;
            state.rng.set(RngStream::Sel, seed_zero_sel());
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10,
                atom: hidden,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 11,
                atom: glam,
                flags: 0,
            });
            if with_spent {
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid: 12,
                    atom: glam,
                    flags: 0,
                });
                let mut spent = state.card_states.get(12);
                spent.enchantment_state =
                    crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
                state.card_states.set(12, spent);
            }

            play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

            assert_eq!(state.card_states.get(11).base_replay_count(), None);
            if with_spent {
                assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
                assert_eq!(state.card_states.get(12).base_replay_count(), Some(2));
                assert_eq!(state.card_states.get(12).enchantment_state.get(), Some(1));
            } else {
                assert_eq!(state.rng.get(RngStream::Sel).counter, 0);
            }
        }
    }

    #[test]
    fn hidden_gem_accepts_spent_glam_status_as_its_exact_fallback() {
        let hidden_identity = identity(CardId::HiddenGem, 0);
        let spent_status_identity = CardIdentity {
            id: CardId::Sloth,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 2,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let hidden = builder.intern_reachable(hidden_identity).unwrap();
        let spent_status = builder.intern_reachable(spent_status_identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 12;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 11,
            atom: spent_status,
            flags: 0,
        });
        let mut spent = state.card_states.get(11);
        spent.enchantment_state = crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
        state.card_states.set(11, spent);

        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(state.card_states.get(11).base_replay_count(), Some(2));
        assert_eq!(state.card_states.get(11).enchantment_state.get(), Some(1));
    }

    #[test]
    fn hidden_gem_refuses_malformed_or_impossible_enchantment_payloads_atomically() {
        let cases = [
            (
                CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 0,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Sharp,
                        amount: -1,
                    }),
                },
                false,
                EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"),
            ),
            (
                CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Sharp,
                        amount: 1,
                    }),
                },
                false,
                EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"),
            ),
            (
                CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 0,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Nimble,
                        amount: 1,
                    }),
                },
                false,
                EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"),
            ),
            (
                CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Nimble,
                        amount: 0,
                    }),
                },
                false,
                EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"),
            ),
            (
                CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 0,
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Glam,
                        amount: -1,
                    }),
                },
                false,
                EngineRefusal::MalformedArgs("Hidden Gem enchantment owner"),
            ),
            // A spent Glam Inflame was refused here until #3135 admitted Glam
            // on a Power; it is now a positive witness in
            // `hidden_gem_treats_glam_powers_by_their_enchanted_replay_count`.
        ];

        for (candidate_identity, spent, expected) in cases {
            let mut builder = CatalogBuilder::new();
            let hidden = builder
                .intern_reachable(identity(CardId::HiddenGem, 0))
                .unwrap();
            let candidate = builder.intern(candidate_identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 12;
            state.rng.set(RngStream::Sel, seed_zero_sel());
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10,
                atom: hidden,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 11,
                atom: candidate,
                flags: 0,
            });
            if spent {
                let mut physical = state.card_states.get(11);
                physical.enchantment_state =
                    crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
                state.card_states.set(11, physical);
            }
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                play_card(&mut state, &catalog, 10, None, None, &mut events),
                Err(expected)
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn hidden_gem_static_enchantments_modify_each_of_three_replayed_bodies() {
        for (candidate_id, enchantment_id, expected_hp, expected_block) in [
            (CardId::StrikeIronclad, EnchantmentId::Sharp, 976, 0),
            (CardId::DefendIronclad, EnchantmentId::Nimble, 1_000, 21),
            (CardId::IronWave, EnchantmentId::Nimble, 985, 21),
        ] {
            let mut builder = CatalogBuilder::new();
            let hidden = builder
                .intern_reachable(identity(CardId::HiddenGem, 0))
                .unwrap();
            let candidate_identity = CardIdentity {
                id: candidate_id,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: enchantment_id,
                    amount: 2,
                }),
            };
            let candidate = builder.intern_reachable(candidate_identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 2;
            state.next_card_uid = 12;
            state.rng.set(RngStream::Sel, seed_zero_sel());
            std::sync::Arc::make_mut(&mut state.monsters)
                .push(crate::hot::HotMonster::new(MonsterKind::Toadpole, 1_000));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10,
                atom: hidden,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 11,
                atom: candidate,
                flags: 0,
            });

            play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();
            let selected = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
            state.piles.get_mut(PileId::Hand).make_mut().push(selected);
            play_card(
                &mut state,
                &catalog,
                11,
                catalog.spec(candidate).unwrap().is_attack.then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(
                catalog.spec(selected.atom).unwrap().identity,
                candidate_identity
            );
            assert_eq!(state.energy, 0, "one payment for each physical card");
            assert_eq!(state.monsters[0].hp, expected_hp);
            assert_eq!(state.block, expected_block);
            assert_eq!(state.history.card_plays_finished_combat, 4);
        }
    }

    #[test]
    fn hidden_gem_both_grants_execute_all_four_eligible_types_serially() {
        for (hidden_upgrade, grant) in [(0, 2), (1, 3)] {
            for (candidate_id, card_type) in [
                (CardId::StrikeIronclad, CardType::Attack),
                (CardId::DefendIronclad, CardType::Skill),
                (CardId::Inflame, CardType::Power),
                (CardId::Toxic, CardType::Status),
            ] {
                let mut builder = CatalogBuilder::new();
                let hidden = builder
                    .intern_reachable(identity(CardId::HiddenGem, hidden_upgrade))
                    .unwrap();
                let candidate = builder.intern_reachable(identity(candidate_id, 0)).unwrap();
                let catalog = builder.build();
                assert_eq!(catalog.spec(candidate).unwrap().card_type, card_type);
                let mut state = HotState::at_defaults();
                state.hp = 80;
                state.energy = 2;
                state.next_card_uid = 12;
                state.rng.set(RngStream::Sel, seed_zero_sel());
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                    uid: 10,
                    atom: hidden,
                    flags: 0,
                });
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid: 11,
                    atom: candidate,
                    flags: 0,
                });

                play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();
                assert_eq!(state.card_states.get(11).base_replay_count(), Some(grant));
                let selected = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
                state.piles.get_mut(PileId::Hand).make_mut().push(selected);
                play_card(
                    &mut state,
                    &catalog,
                    11,
                    (card_type == CardType::Attack).then_some(0),
                    None,
                    &mut Vec::new(),
                )
                .unwrap();

                let bodies = grant + 1;
                assert_eq!(state.energy, 0, "one payment per physical card");
                assert_eq!(state.history.card_plays_finished_combat, bodies + 1);
                match card_type {
                    CardType::Attack => assert_eq!(state.monsters[0].hp, 100 - 6 * bodies),
                    CardType::Skill => assert_eq!(state.block, 5 * bodies),
                    CardType::Power => {
                        assert_eq!(state.powers.value(PowerId::Strength), 2 * bodies)
                    }
                    CardType::Status => {
                        assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), [selected])
                    }
                    CardType::Curse | CardType::Quest => unreachable!(),
                }
                let occurrences = PileId::ALL
                    .into_iter()
                    .flat_map(|pile| state.piles.get(pile).as_slice())
                    .filter(|card| card.uid == 11)
                    .count();
                assert_eq!(occurrences, usize::from(card_type != CardType::Power));
            }
        }
    }

    #[test]
    fn hidden_gem_metadata_and_level_one_singleton_are_exact() {
        for (upgrade, amount) in [(0, 2), (1, 3)] {
            let identity = identity(CardId::HiddenGem, upgrade);
            let mut builder = CatalogBuilder::new();
            let hidden = builder.intern_reachable(identity).unwrap();
            let strike = builder.intern_reachable(STRIKE).unwrap();
            let catalog = builder.build();
            let spec = catalog.spec(hidden).unwrap();
            assert_eq!(spec.cost, 1);
            assert_eq!(spec.card_type, CardType::Skill);
            assert!(spec.playable && spec.is_skill && !spec.exhausts);
            assert!(!spec.targeted);
            assert_eq!(spec.target_type, crate::catalog::CardTargetType::SelfTarget);
            assert!(matches!(catalog.steps(spec), [step]
                if step.kind == StepKind::HiddenGemExact
                    && catalog.args(step.args) == [CompiledArg::I(amount)]));

            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 12;
            state.rng.set(RngStream::Sel, seed_zero_sel());
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10,
                atom: hidden,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 11,
                atom: strike,
                flags: 0,
            });
            let mut events = Vec::new();
            play_card(&mut state, &catalog, 10, None, None, &mut events).unwrap();
            assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
            assert_eq!(
                state.card_states.get(11).base_replay_count(),
                Some(amount as i32)
            );
            assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 11);
            assert!(
                matches!(
                    events.as_slice(),
                    [
                        crate::engine::Event::CardPlayed { uid: 10, .. },
                        crate::engine::Event::CardResolved {
                            uid: 10,
                            pile: PileId::Discard
                        }
                    ]
                ),
                "Hidden Gem previews only in the UI: {events:?}"
            );
        }
    }

    #[test]
    fn hidden_gem_validates_all_ineligible_draw_and_duplicate_uid_before_rng() {
        let identities = [
            identity(CardId::HiddenGem, 0),
            identity(CardId::Dazed, 0),
            identity(CardId::Regret, 0),
            identity(CardId::SpoilsMap, 0),
            STRIKE,
        ];
        let mut builder = CatalogBuilder::new();
        let atoms = identities
            .into_iter()
            .map(|identity| builder.intern_reachable(identity).unwrap())
            .collect::<Vec<_>>();
        let catalog = builder.build();
        assert!(catalog.spec(atoms[1]).unwrap().native_unplayable);
        assert_eq!(catalog.spec(atoms[2]).unwrap().card_type, CardType::Curse);
        assert_eq!(catalog.spec(atoms[3]).unwrap().card_type, CardType::Quest);
        let source = HotCard {
            uid: 10,
            atom: atoms[0],
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 15;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 11,
                atom: atoms[1],
                flags: 0,
            },
            HotCard {
                uid: 12,
                atom: atoms[2],
                flags: 0,
            },
            HotCard {
                uid: 13,
                atom: atoms[3],
                flags: 0,
            },
            HotCard {
                uid: 14,
                atom: atoms[4],
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut replay = state.card_states.get(14);
        replay.set_base_replay_count(Some(1)).unwrap();
        state.card_states.set(14, replay);
        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();
        assert_eq!(state.rng.get(RngStream::Sel).counter, 0);

        let mut duplicate = HotState::at_defaults();
        duplicate.hp = 80;
        duplicate.energy = 1;
        duplicate.next_card_uid = 12;
        duplicate.rng.set(RngStream::Sel, seed_zero_sel());
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        duplicate.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 11,
                atom: atoms[4],
                flags: 0,
            },
            HotCard {
                uid: 11,
                atom: atoms[4],
                flags: 0,
            },
        ]);
        let before = duplicate.clone();
        let mut events = Vec::new();
        assert_eq!(
            play_card(&mut duplicate, &catalog, 10, None, None, &mut events),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: 11,
                matches: 2
            })
        );
        assert_eq!(duplicate, before);
        assert!(events.is_empty());
    }

    #[test]
    fn hidden_gem_changes_only_base_replay_on_the_selected_physical_payload() {
        let hidden_identity = identity(CardId::HiddenGem, 0);
        let selected_identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 1,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let hidden = builder.intern_reachable(hidden_identity).unwrap();
        let selected = builder.intern_reachable(selected_identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 10,
            atom: hidden,
            flags: 0,
        };
        let selected_card = HotCard {
            uid: 11,
            atom: selected,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
        };
        let mut payload = crate::hot::CardInstanceState {
            enchantment_state: crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap(),
            damage_growth: 7,
            local_cost_modifiers: crate::hot::LocalCostModifiers::from_rows(vec![
                crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Add,
                    amount: -1,
                    expiration: crate::hot::LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                },
            ]),
            free_star_cost_this_turn_or_played_rows: 2,
            local_retain: true,
            local_sly: true,
            transient_retain: true,
            ..crate::hot::CardInstanceState::default()
        };
        payload.set_local_ethereal(true);
        payload.set_base_replay_count(Some(0)).unwrap();
        let mut expected = payload.clone();
        expected.set_base_replay_count(Some(2)).unwrap();

        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 12;
        state.rng.set(RngStream::Sel, seed_zero_sel());
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(selected_card);
        state.card_states.set(11, payload);
        play_card(&mut state, &catalog, 10, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.piles.get(PileId::Draw).as_slice(), [selected_card]);
        assert_eq!(state.card_states.get(11), expected);
    }

    #[test]
    fn generic_base_replay_runs_three_bodies_with_one_payment_and_route() {
        let (catalog, _, _, strike, _, _) = hidden_gem_catalog();
        let source = HotCard {
            uid: 10,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 11;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut replay = state.card_states.get(10);
        replay.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(10, replay);

        play_card(&mut state, &catalog, 10, Some(0), None, &mut Vec::new()).unwrap();

        assert_eq!(state.energy, 0);
        assert_eq!(state.monsters[0].hp, 82);
        assert_eq!(state.history.card_plays_finished_combat, 3);
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), [source]);
        assert_eq!(state.card_states.get(10).base_replay_count(), Some(2));

        let mut corpse_target = HotState::at_defaults();
        corpse_target.hp = 80;
        corpse_target.energy = 1;
        corpse_target.next_card_uid = 11;
        corpse_target
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        corpse_target.monsters_mut().extend([
            HotMonster::new(MonsterKind::Toadpole, 6),
            HotMonster::new(MonsterKind::Toadpole, 100),
        ]);
        let mut replay = corpse_target.card_states.get(10);
        replay.set_base_replay_count(Some(2)).unwrap();
        corpse_target.card_states.set(10, replay);
        play_card(
            &mut corpse_target,
            &catalog,
            10,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(corpse_target.monsters[0].hp, 0);
        assert_eq!(corpse_target.monsters[1].hp, 100);
        assert_eq!(corpse_target.history.card_plays_finished_combat, 3);
    }

    #[test]
    fn generic_replay_covers_attack_skill_power_status_and_supported_modifiers() {
        let identities = [
            STRIKE,
            DEFEND,
            identity(CardId::Inflame, 0),
            identity(CardId::Toxic, 0),
        ];
        let mut builder = CatalogBuilder::new();
        let atoms = identities
            .into_iter()
            .map(|identity| builder.intern_reachable(identity).unwrap())
            .collect::<Vec<_>>();
        let catalog = builder.build();

        for (index, expected_type) in [
            CardType::Attack,
            CardType::Skill,
            CardType::Power,
            CardType::Status,
        ]
        .into_iter()
        .enumerate()
        {
            let source = HotCard {
                uid: 10,
                atom: atoms[index],
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            assert_eq!(catalog.spec(source.atom).unwrap().card_type, expected_type);
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 11;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let mut replay = state.card_states.get(10);
            replay.set_base_replay_count(Some(2)).unwrap();
            state.card_states.set(10, replay);
            play_card(
                &mut state,
                &catalog,
                10,
                (expected_type == CardType::Attack).then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.energy, 0);
            assert_eq!(state.history.card_plays_finished_combat, 3);
            match expected_type {
                CardType::Attack => assert_eq!(state.monsters[0].hp, 82),
                CardType::Skill => assert_eq!(state.block, 15),
                CardType::Power => assert_eq!(state.powers.value(PowerId::Strength), 6),
                CardType::Status => {
                    assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), [source])
                }
                CardType::Curse | CardType::Quest => unreachable!(),
            }
        }

        let source = HotCard {
            uid: 20,
            atom: atoms[1],
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut direct = HotState::at_defaults();
        direct.hp = 80;
        direct.next_card_uid = 21;
        direct.piles.get_mut(PileId::Draw).make_mut().push(source);
        let mut replay = direct.card_states.get(20);
        replay.set_base_replay_count(Some(2)).unwrap();
        direct.card_states.set(20, replay);
        let energy_before = direct.energy;
        autoplay_collected_cards(&mut direct, &catalog, &[source], &mut Vec::new()).unwrap();
        assert_eq!(direct.energy, energy_before, "direct AutoPlay pays nothing");
        assert_eq!(direct.block, 15);
        assert_eq!(direct.history.card_plays_finished_combat, 3);
        assert_eq!(direct.piles.get(PileId::Discard).as_slice(), [source]);

        for (card_index, power, expected_bodies) in [
            (0, PowerId::OneTwoPunch, 4),
            (1, PowerId::Burst, 4),
            (2, PowerId::SignalBoost, 4),
        ] {
            let source = HotCard {
                uid: 30,
                atom: atoms[card_index],
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 31;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let mut replay = state.card_states.get(30);
            replay.set_base_replay_count(Some(2)).unwrap();
            state.card_states.set(30, replay);
            state.powers.set(power, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            play_card(
                &mut state,
                &catalog,
                30,
                (card_index == 0).then_some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.history.card_plays_finished_combat, expected_bodies);
            assert_eq!(state.powers.value(power), 0);
        }

        let source = HotCard {
            uid: 40,
            atom: atoms[1],
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut echo = HotState::at_defaults();
        echo.hp = 80;
        echo.energy = 1;
        echo.next_card_uid = 41;
        echo.piles.get_mut(PileId::Hand).make_mut().push(source);
        let mut replay = echo.card_states.get(40);
        replay.set_base_replay_count(Some(2)).unwrap();
        echo.card_states.set(40, replay);
        echo.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
        play_card(&mut echo, &catalog, 40, None, None, &mut Vec::new()).unwrap();
        assert_eq!(echo.history.card_plays_finished_combat, 4);
        assert_eq!(echo.block, 20);

        let source = HotCard {
            uid: 50,
            atom: atoms[2],
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut power_echo = HotState::at_defaults();
        power_echo.hp = 80;
        power_echo.energy = 1;
        power_echo.next_card_uid = 51;
        power_echo
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        let mut replay = power_echo.card_states.get(50);
        replay.set_base_replay_count(Some(2)).unwrap();
        power_echo.card_states.set(50, replay);
        power_echo.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
        play_card(&mut power_echo, &catalog, 50, None, None, &mut Vec::new()).unwrap();
        assert_eq!(power_echo.history.card_plays_finished_combat, 4);
        assert_eq!(power_echo.powers.value(PowerId::Strength), 8);
    }

    #[test]
    fn generic_replay_accepts_256_refuses_257_and_rolls_back_a_late_body() {
        let mut builder = CatalogBuilder::new();
        let defend = builder.intern_reachable(DEFEND).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 10,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        for (replay, succeeds) in [(255, true), (256, false)] {
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 1;
            state.next_card_uid = 11;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            let mut payload = state.card_states.get(10);
            payload.set_base_replay_count(Some(replay)).unwrap();
            state.card_states.set(10, payload);
            let before = state.clone();
            let mut events = Vec::new();
            let result = play_card(&mut state, &catalog, 10, None, None, &mut events);
            if succeeds {
                result.unwrap();
                assert_eq!(state.history.card_plays_finished_combat, 256);
                assert_eq!(state.block, 1280);
            } else {
                assert_eq!(result, Err(EngineRefusal::CounterOverflow("play count")));
                assert_eq!(state, before);
                assert!(events.is_empty());
            }
        }

        let mut late = HotState::at_defaults();
        late.hp = 80;
        late.energy = 1;
        late.next_card_uid = 11;
        late.history.owner_card_plays_finished_this_turn = i16::MAX - 1;
        late.piles.get_mut(PileId::Hand).make_mut().push(source);
        let mut payload = late.card_states.get(10);
        payload.set_base_replay_count(Some(1)).unwrap();
        late.card_states.set(10, payload);
        let before = late.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 7 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(&mut late, &catalog, 10, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "owner_card_plays_finished_this_turn"
            ))
        );
        assert_eq!(late, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn generic_replay_stops_after_target_death_and_routes_once() {
        let mut builder = CatalogBuilder::new();
        let strike = builder.intern_reachable(STRIKE).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 10,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 1;
        state.next_card_uid = 11;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 5));
        let mut replay = state.card_states.get(10);
        replay.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(10, replay);
        let mut events = Vec::new();
        play_card(&mut state, &catalog, 10, Some(0), None, &mut events).unwrap();
        assert!(state.history.over);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.piles.get(PileId::Play).as_slice(), [source]);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::engine::Event::CardPlayed { .. }))
                .count(),
            1
        );
    }

    fn enlightenment(upgrade: u8) -> CardIdentity {
        CardIdentity {
            id: CardId::Enlightenment,
            upgrade,
            enchantment: None,
        }
    }

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn ctx_state() -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern(STRIKE).unwrap();
        builder.intern(DEFEND).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(crate::ids::MonsterKind::Toadpole, 12));
        state
            .monsters_mut()
            .push(HotMonster::new(crate::ids::MonsterKind::Toadpole, 12));
        (state, catalog)
    }

    fn card(catalog: &Catalog, identity: &CardIdentity, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog.atom(identity).unwrap(),
            flags: 0,
        }
    }

    fn run(
        kind: StepKind,
        state: &mut HotState,
        catalog: &Catalog,
        target: Option<usize>,
        args: &[CompiledArg],
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&STRIKE).unwrap()).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 0,
            target,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        apply_step(kind, &mut ctx)
    }

    fn run_card_step(
        kind: StepKind,
        identity: CardIdentity,
        state: &mut HotState,
        catalog: &Catalog,
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 1,
            target: None,
            selection: None,
            x_value: 0,
            args,
            events,
        };
        apply_step(kind, &mut ctx)
    }

    #[test]
    fn the_bomb_both_levels_allocate_independent_instances_in_apply_order() {
        let mut builder = CatalogBuilder::new();
        let base = identity(CardId::TheBomb, 0);
        let upgraded = identity(CardId::TheBomb, 1);
        let base_atom = builder.intern(base).unwrap();
        let upgraded_atom = builder.intern(upgraded).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().extend([
            HotCard {
                uid: 1,
                atom: base_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: upgraded_atom,
                flags: 0,
            },
        ]);
        let mut events = Vec::new();

        run_card_step(
            StepKind::TheBombExact,
            base,
            &mut state,
            &catalog,
            &[CompiledArg::I(3), CompiledArg::I(40)],
            &mut events,
        )
        .unwrap();
        let upgraded_spec = *catalog.spec(upgraded_atom).unwrap();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &upgraded_spec,
            source_uid: 2,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3), CompiledArg::I(50)],
            events: &mut events,
        };
        the_bomb_exact(&mut ctx).unwrap();

        assert_eq!(state.fanouts.next_the_bomb_uid(), 2);
        assert_eq!(
            state.fanouts.the_bomb_instances().collect::<Vec<_>>(),
            &[
                crate::hot::TheBombInstance {
                    uid: 0,
                    turns: 3,
                    damage: 40,
                },
                crate::hot::TheBombInstance {
                    uid: 1,
                    turns: 3,
                    damage: 50,
                },
            ]
        );
        assert_eq!(
            state.fanouts.before_side_turn_end_order(),
            &[
                crate::hot::BeforeSideTurnEndToken::TheBomb(0),
                crate::hot::BeforeSideTurnEndToken::TheBomb(1),
            ]
        );
        assert_eq!(
            events,
            [
                crate::engine::Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::TheBomb,
                    amount: 3,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::TheBomb,
                    amount: 3,
                },
            ]
        );
    }

    #[test]
    fn the_bomb_forged_operands_and_uid_overflow_refuse_without_partial_state() {
        let mut builder = CatalogBuilder::new();
        let bomb = identity(CardId::TheBomb, 0);
        let atom = builder.intern(bomb).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_card_step(
                StepKind::TheBombExact,
                bomb,
                &mut state,
                &catalog,
                &[CompiledArg::I(3), CompiledArg::I(50)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("the_bomb_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.fanouts.set_next_the_bomb_uid(u32::MAX);
        let before = state.clone();
        assert_eq!(
            run_card_step(
                StepKind::TheBombExact,
                bomb,
                &mut state,
                &catalog,
                &[CompiledArg::I(3), CompiledArg::I(40)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("next_the_bomb_uid"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn the_bomb_public_burst_allocates_twice_and_uid_overflow_rolls_back_action() {
        let mut builder = CatalogBuilder::new();
        let bomb = identity(CardId::TheBomb, 1);
        let atom = builder.intern(bomb).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 10;
        state.next_card_uid = 18;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let mut events = Vec::new();

        let played = apply_action_into(
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
        assert_eq!(
            played.fanouts.the_bomb_instances().collect::<Vec<_>>(),
            &[
                crate::hot::TheBombInstance {
                    uid: 0,
                    turns: 3,
                    damage: 50,
                },
                crate::hot::TheBombInstance {
                    uid: 1,
                    turns: 3,
                    damage: 50,
                },
            ]
        );
        assert_eq!(played.powers.value(PowerId::Burst), 0);

        let mut overflow = state.clone();
        overflow.fanouts.set_next_the_bomb_uid(u32::MAX - 1);
        let before = overflow.clone();
        let mut overflow_events = Vec::new();
        assert_eq!(
            play_card(
                &mut overflow,
                &catalog,
                source.uid,
                None,
                None,
                &mut overflow_events,
            ),
            Err(EngineRefusal::CounterOverflow("next_the_bomb_uid"))
        );
        assert_eq!(overflow, before);
        assert!(overflow_events.is_empty());
    }

    fn toric_state(upgrade: u8) -> (HotState, Catalog, CardIdentity) {
        let toric = identity(CardId::ToricToughness, upgrade);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(toric).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 10;
        state.next_card_uid = 2;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        (state, catalog, toric)
    }

    fn run_toric(
        state: &mut HotState,
        catalog: &Catalog,
        toric: CardIdentity,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let block = 5 + 2 * i64::from(toric.upgrade);
        run_card_step(
            StepKind::ToricToughnessExact,
            toric,
            state,
            catalog,
            &[CompiledArg::I(block), CompiledArg::I(2)],
            events,
        )
    }

    #[test]
    fn toric_both_rows_retain_full_powered_decimal_and_refuse_a_second_object() {
        for upgrade in 0..=1 {
            let (mut plain, plain_catalog, plain_toric) = toric_state(upgrade);
            run_toric(&mut plain, &plain_catalog, plain_toric, &mut Vec::new()).unwrap();
            let base = 5 + 2 * i32::from(upgrade);
            assert_eq!(plain.block, base);
            assert_eq!(plain.fanouts.toric_toughness().unwrap().duration, 2);
            assert_eq!(
                plain
                    .fanouts
                    .toric_toughness()
                    .unwrap()
                    .block
                    .nonnegative_fraction(),
                Some((base as u128, 1))
            );

            let (mut state, catalog, toric) = toric_state(upgrade);
            state.powers.set(PowerId::Dexterity, SlotWire::Int, 3);
            state.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
            state.powers.set(PowerId::PlayerFrail, SlotWire::Int, 1);
            let mut events = Vec::new();
            run_toric(&mut state, &catalog, toric, &mut events).unwrap();

            let expected = if upgrade == 0 {
                (12, (12, 1))
            } else {
                (15, (15, 1))
            };
            assert_eq!(state.block, expected.0);
            let instance = state.fanouts.toric_toughness().unwrap();
            assert_eq!(instance.duration, 2);
            assert_eq!(instance.block.nonnegative_fraction(), Some(expected.1));
            assert_eq!(
                state.fanouts.after_block_cleared_order(),
                &[PowerId::ToricToughness]
            );
        }

        // #3057: ToricToughnessPower is Instanced, so a second play while
        // the first object lives attaches a second object with its own Block
        // and duration (out of step: 5 Block / 2 turns vs 6 Block / 2 turns
        // here). The single keyed record cannot hold it: refuse by name,
        // atomically, including the card's own powered GainBlock prefix.
        let (mut state, catalog, toric) = toric_state(0);
        let mut events = Vec::new();
        run_toric(&mut state, &catalog, toric, &mut events).unwrap();
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 1);
        let before = state.clone();
        let events_before = events.clone();
        assert_eq!(
            run_toric(&mut state, &catalog, toric, &mut events),
            Err(EngineRefusal::PowerRestackNotModeled(
                PowerId::ToricToughness
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        // Once the first object has expired, the next play is a fresh first
        // object again.
        state.fanouts.decrement_toric_toughness().unwrap();
        assert_eq!(state.fanouts.decrement_toric_toughness(), Ok(0));
        assert!(state.fanouts.toric_toughness().is_none());
        run_toric(&mut state, &catalog, toric, &mut events).unwrap();
        let instance = state.fanouts.toric_toughness().unwrap();
        assert_eq!(instance.duration, 2);
        assert_eq!(instance.block.nonnegative_fraction(), Some((6, 1)));
        assert_eq!(
            state.fanouts.after_block_cleared_order(),
            &[PowerId::ToricToughness]
        );
    }

    #[test]
    fn toric_fractional_refresh_truncates_storage_after_full_decimal_callback() {
        let (mut state, catalog, toric) = toric_state(0);
        state.powers.set(PowerId::PlayerFrail, SlotWire::Int, 1);
        run_toric(&mut state, &catalog, toric, &mut Vec::new()).unwrap();
        assert_eq!(state.block, 3);
        assert_eq!(
            state
                .fanouts
                .toric_toughness()
                .unwrap()
                .block
                .nonnegative_fraction(),
            Some((15, 4))
        );

        state.turn = 2;
        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.block, 3);
        assert_eq!(state.fanouts.toric_toughness().unwrap().duration, 1);
        state.turn = 3;
        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.block, 3);
        assert!(state.fanouts.toric_toughness().is_none());
        assert!(state.fanouts.after_block_cleared_order().is_empty());

        let (mut subunit, catalog, toric) = toric_state(0);
        subunit.powers.set(PowerId::Dexterity, SlotWire::Int, -4);
        subunit.powers.set(PowerId::PlayerFrail, SlotWire::Int, 1);
        run_toric(&mut subunit, &catalog, toric, &mut Vec::new()).unwrap();
        assert_eq!(subunit.block, 0);
        assert_eq!(subunit.history.card_block_gains, 1);
        subunit.powers.set(PowerId::Juggernaut, SlotWire::Int, 10);
        assert!(
            subunit
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );
        subunit
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        subunit.turn = 2;
        crate::engine::turn::begin_player_turn(&mut subunit, &catalog, &mut Vec::new()).unwrap();
        assert!(subunit.history.over);
        assert_eq!(subunit.block, 0);
        assert_eq!(subunit.fanouts.toric_toughness().unwrap().duration, 1);
    }

    #[test]
    fn toric_terminal_and_ordered_refresh_paths_complete_outer_lifecycles() {
        let (mut terminal, catalog, toric) = toric_state(0);
        terminal.powers.set(PowerId::Juggernaut, SlotWire::Int, 10);
        assert!(
            terminal
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );
        terminal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        run_toric(&mut terminal, &catalog, toric, &mut Vec::new()).unwrap();
        assert!(terminal.history.over);
        assert!(terminal.fanouts.toric_toughness().is_none());

        for generic_first in [false, true] {
            let (mut state, catalog, toric) = toric_state(0);
            if generic_first {
                crate::engine::damage::apply_block_next_turn(&mut state, 4, &mut Vec::new())
                    .unwrap();
            }
            run_toric(&mut state, &catalog, toric, &mut Vec::new()).unwrap();
            if !generic_first {
                crate::engine::damage::apply_block_next_turn(&mut state, 4, &mut Vec::new())
                    .unwrap();
            }
            state.block = 0;
            state.powers.set(PowerId::Juggernaut, SlotWire::Int, 10);
            assert!(
                state
                    .fanouts
                    .register_after_block_gained(PowerId::Juggernaut)
            );
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));
            state.turn = 2;
            crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
            assert!(state.history.over);
            assert_eq!(state.powers.value(PowerId::BlockNextTurn), 0);
            assert_eq!(state.fanouts.toric_toughness().unwrap().duration, 1);
            assert_eq!(
                state.fanouts.after_block_cleared_order(),
                &[PowerId::ToricToughness]
            );
            assert_eq!(state.block, if generic_first { 4 } else { 5 });
        }
    }

    #[test]
    fn toric_and_bomb_coexist_and_late_refusals_are_atomic() {
        let (mut state, catalog, toric) = toric_state(0);
        let bomb_uid = state.fanouts.begin_the_bomb(3).unwrap();
        state.fanouts.set_the_bomb_damage(bomb_uid, 40).unwrap();
        run_toric(&mut state, &catalog, toric, &mut Vec::new()).unwrap();
        assert_eq!(state.fanouts.the_bomb_instances().count(), 1);
        assert!(state.fanouts.toric_toughness().is_some());

        let (mut overflow, catalog, toric) = toric_state(0);
        overflow
            .fanouts
            .set_toric_toughness(Some(crate::hot::ToricToughnessInstance {
                duration: i32::MAX,
                block: crate::decimal::DotNetDecimal::from_i64(5),
            }));
        assert!(
            overflow
                .fanouts
                .register_after_block_cleared(PowerId::ToricToughness)
        );
        let before = overflow.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 7 }];
        let before_events = events.clone();
        // A live object makes this play a second native object (#3057), so
        // the named restack refusal now fires before the duration overflow
        // could; it is just as atomic.
        assert_eq!(
            run_toric(&mut overflow, &catalog, toric, &mut events),
            Err(EngineRefusal::PowerRestackNotModeled(
                PowerId::ToricToughness
            ))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn toric_public_manual_replay_and_direct_autoplay_run_independent_bodies() {
        let toric = identity(CardId::ToricToughness, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(toric).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };

        let mut manual = HotState::at_defaults();
        manual.hp = 50;
        manual.max_hp = 50;
        manual.energy = 10;
        manual.next_card_uid = 18;
        manual.exact_piles = true;
        manual.piles.get_mut(PileId::Hand).make_mut().push(source);
        manual.powers.set(PowerId::PlayerFrail, SlotWire::Int, 1);
        let mut replay = manual.card_states.get(source.uid);
        replay.set_base_replay_count(Some(1)).unwrap();
        manual.card_states.set(source.uid, replay);
        // The replayed body is a second native ToricToughnessPower object
        // (#3057: Instanced, its own Block and duration), which the one keyed
        // record cannot hold, so the public action refuses by name.
        assert_eq!(
            crate::engine::apply_action(
                &manual,
                &catalog,
                &crate::engine::Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: crate::engine::SelectionRef::NONE,
                },
            )
            .map(|_| ()),
            Err(EngineRefusal::PowerRestackNotModeled(
                PowerId::ToricToughness
            ))
        );

        let mut autoplay = HotState::at_defaults();
        autoplay.hp = 50;
        autoplay.max_hp = 50;
        autoplay.next_card_uid = 18;
        autoplay.exact_piles = true;
        autoplay.piles.get_mut(PileId::Draw).make_mut().push(source);
        let energy = autoplay.energy;
        let before_rng = autoplay.rng.clone();
        autoplay_collected_cards(&mut autoplay, &catalog, &[source], &mut Vec::new()).unwrap();
        assert_eq!(autoplay.energy, energy, "direct AutoPlay pays nothing");
        assert_eq!(autoplay.block, 5);
        assert_eq!(autoplay.fanouts.toric_toughness().unwrap().duration, 2);
        assert_eq!(autoplay.piles.get(PileId::Discard).as_slice(), &[source]);
        for stream in RngStream::ALL {
            assert_eq!(autoplay.rng.get(stream), before_rng.get(stream));
        }
    }

    #[test]
    fn toric_refresh_orders_with_clear_prevention_and_plating_decrement() {
        for keeper in [PowerId::Barricade, PowerId::Blur] {
            let (mut state, catalog, toric) = toric_state(0);
            run_toric(&mut state, &catalog, toric, &mut Vec::new()).unwrap();
            state.block = 10;
            state.turn = 2;
            state.powers.set(keeper, SlotWire::Int, 1);
            state.powers.set(PowerId::Plating, SlotWire::Int, 2);
            if crate::hot::AfterSideTurnStartToken::from_power(keeper).is_some() {
                assert!(state.fanouts.register_after_side_turn_start(keeper));
            }
            assert!(
                state
                    .fanouts
                    .register_after_side_turn_start(PowerId::Plating)
            );
            crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
            assert_eq!(state.block, 15, "{keeper:?} preserves before Toric refresh");
            assert_eq!(state.fanouts.toric_toughness().unwrap().duration, 1);
            assert_eq!(state.powers.value(PowerId::Plating), 1);
        }
    }

    #[test]
    fn toric_forged_operands_sources_and_listener_topologies_are_atomic() {
        let (mut state, catalog, toric) = toric_state(0);
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 9 }];
        for args in [
            vec![CompiledArg::I(6), CompiledArg::I(2)],
            vec![CompiledArg::I(5), CompiledArg::I(1)],
            vec![CompiledArg::I(5)],
        ] {
            let before = state.clone();
            let before_events = events.clone();
            assert!(
                run_card_step(
                    StepKind::ToricToughnessExact,
                    toric,
                    &mut state,
                    &catalog,
                    &args,
                    &mut events,
                )
                .is_err()
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        let mut borrowed_builder = CatalogBuilder::new();
        borrowed_builder.intern(toric).unwrap();
        borrowed_builder.intern(DEFEND).unwrap();
        let borrowed_catalog = borrowed_builder.build();
        let before = state.clone();
        let before_events = events.clone();
        assert!(
            run_card_step(
                StepKind::ToricToughnessExact,
                DEFEND,
                &mut state,
                &borrowed_catalog,
                &[CompiledArg::I(5), CompiledArg::I(2)],
                &mut events,
            )
            .is_err()
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        state.powers.set(PowerId::BlockNextTurn, SlotWire::Int, 4);
        let before = state.clone();
        let before_events = events.clone();
        assert_eq!(
            run_toric(&mut state, &catalog, toric, &mut events),
            Err(EngineRefusal::MalformedArgs("toric_toughness_exact"))
        );
        assert_eq!(state, before, "missing generic listener refuses first");
        assert_eq!(events, before_events);

        state.powers.set(PowerId::BlockNextTurn, SlotWire::Int, 0);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Strength])
        );
        let before = state.clone();
        let before_events = events.clone();
        assert!(run_toric(&mut state, &catalog, toric, &mut events).is_err());
        assert_eq!(state, before, "unsupported callback refuses before gain");
        assert_eq!(events, before_events);
    }

    #[test]
    fn unsupported_enchanted_the_bomb_refuses_before_public_action_mutation() {
        let mut builder = CatalogBuilder::new();
        let bomb = CardIdentity {
            id: CardId::TheBomb,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Sharp,
                amount: 2,
            }),
        };
        let atom = builder.intern(bomb).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 10;
        state.next_card_uid = 18;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            play_card(&mut state, &catalog, source.uid, None, None, &mut events,),
            Err(EngineRefusal::MalformedArgs("the_bomb_exact enchantment"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let mut auto = HotState::at_defaults();
        auto.hp = 50;
        auto.max_hp = 50;
        auto.next_card_uid = 18;
        auto.exact_piles = true;
        auto.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = auto.clone();
        let mut auto_events = Vec::new();
        assert_eq!(
            autoplay_imitation_clone(&mut auto, &catalog, source, &mut auto_events),
            Err(EngineRefusal::MalformedArgs("the_bomb_exact enchantment"))
        );
        assert_eq!(auto, before);
        assert!(auto_events.is_empty());
    }

    fn the_ball_fixture(
        frozen_upgrade: u8,
        live_upgrade: u8,
        growth: i32,
        monster_hp: i32,
    ) -> (HotState, Catalog, CardSpec, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atoms = [
            builder.intern(identity(CardId::TheBall, 0)).unwrap(),
            builder.intern(identity(CardId::TheBall, 1)).unwrap(),
        ];
        let frozen_atom = atoms[usize::from(frozen_upgrade)];
        let live_atom = atoms[usize::from(live_upgrade)];
        let catalog = builder.build();
        let spec = *catalog.spec(frozen_atom).unwrap();
        let source = HotCard {
            uid: 17,
            atom: live_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.exact_piles = true;
        state.multiplayer_ally_key = 1;
        state
            .fanouts
            .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                key: 1,
                ..crate::hot::MultiplayerAllyState::default()
            });
        let mut monster = HotMonster::new(MonsterKind::Toadpole, monster_hp);
        monster.uid = 9;
        state.monsters_mut().push(monster);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.card_states.set(
            source.uid,
            crate::hot::CardInstanceState {
                damage_growth: growth,
                ..crate::hot::CardInstanceState::default()
            },
        );
        (state, catalog, spec, source)
    }

    fn run_the_ball_foundation(
        state: &mut HotState,
        catalog: &Catalog,
        spec: &CardSpec,
        source_uid: u32,
        target: Option<usize>,
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let target_identity = target
            .and_then(|index| state.monsters.get(index))
            .map_or((i32::MIN, u32::MAX), |monster| (monster.slot, monster.uid));
        run_the_ball_foundation_with_target_identity(
            state,
            catalog,
            spec,
            source_uid,
            target,
            target_identity,
            args,
            events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_the_ball_foundation_with_target_identity(
        state: &mut HotState,
        catalog: &Catalog,
        spec: &CardSpec,
        source_uid: u32,
        target: Option<usize>,
        target_identity: (i32, u32),
        args: &[CompiledArg],
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid,
            target,
            selection: None,
            x_value: 0,
            args,
            events,
        };
        crate::engine::play::with_test_active_play_target(source_uid, target_identity, || {
            the_ball_body_foundation(&mut ctx)
        })
    }

    fn gang_up_fixture(upgrade: u8, prior: i32) -> (HotState, Catalog, CardIdentity) {
        let gang_up = identity(CardId::GangUp, upgrade);
        let mut builder = CatalogBuilder::new();
        builder.intern(gang_up).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.multiplayer_ally_key = 1;
        state
            .fanouts
            .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                key: 1,
                ..crate::hot::MultiplayerAllyState::default()
            });
        let mut target = HotMonster::new(MonsterKind::Toadpole, 200);
        target.uid = 1;
        target.nonowner_same_side_powered_damage_results_this_turn = prior;
        state.monsters_mut().push(target);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, &gang_up, 7));
        (state, catalog, gang_up)
    }

    fn run_gang_up_body(
        state: &mut HotState,
        catalog: &Catalog,
        gang_up: CardIdentity,
        source_uid: u32,
        operands: (Option<usize>, Option<u32>, i64, &[CompiledArg]),
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog.spec(catalog.atom(&gang_up).unwrap()).unwrap();
        let (target, selection, x_value, args) = operands;
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid,
            target,
            selection,
            x_value,
            args,
            events,
        };
        gang_up_exact(&mut ctx)
    }

    #[test]
    fn gang_up_two_rows_programs_and_target_local_history_are_exact() {
        let carriers = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps.iter().find_map(|step| {
                    (step.kind == StepKind::GangUpExact).then_some((
                        row.id,
                        row.upgrade,
                        row.cost,
                        row.target_type,
                        row.is_power,
                        row.is_skill,
                        row.targeted,
                        step.args,
                    ))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::GangUp,
                    0,
                    1,
                    "AnyEnemy",
                    false,
                    false,
                    true,
                    &[Arg::I(5)][..],
                ),
                (
                    CardId::GangUp,
                    1,
                    1,
                    "AnyEnemy",
                    false,
                    false,
                    true,
                    &[Arg::I(7)][..],
                ),
            ]
        );

        for (upgrade, prior, expected_hp) in [(0, 2, 185), (1, 3, 174)] {
            let (mut state, catalog, gang_up) = gang_up_fixture(upgrade, prior);
            state.monsters_mut()[0].owner_powered_damage_results_this_turn = 9;
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(&catalog, &gang_up, 8));
            // Direct-body source placement is authenticated by uid, not by a
            // hard-coded pile. Use Play here and leave the other live copy in
            // Hand to prove the source identity, not CardId, is selected.
            let source = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            let mut sibling = HotMonster::new(MonsterKind::Toadpole, 200);
            sibling.uid = 2;
            sibling.nonowner_same_side_powered_damage_results_this_turn = prior + 4;
            state.monsters_mut().push(sibling);
            let args = [CompiledArg::I(if upgrade == 0 { 5 } else { 7 })];
            let mut events = Vec::new();
            run_gang_up_body(
                &mut state,
                &catalog,
                gang_up,
                7,
                (Some(0), None, 0, &args),
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].hp, expected_hp);
            assert_eq!(state.monsters[1].hp, 200);
            assert_eq!(
                state.monsters[0].nonowner_same_side_powered_damage_results_this_turn,
                prior
            );
            assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 10);
            assert!(matches!(
                events.as_slice(),
                [crate::engine::Event::MonsterDamaged { uid: 1, .. }]
            ));
        }
    }

    #[test]
    fn gang_up_public_play_requires_a_live_remote_player_and_routes_the_source() {
        let (mut state, catalog, _) = gang_up_fixture(0, 2);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 200);
        second.uid = 2;
        second.slot = 1;
        second.nonowner_same_side_powered_damage_results_this_turn = 3;
        state.monsters_mut().push(second);
        let mut events = Vec::new();
        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 7,
                // The byte remains an enemy roster index even though remote
                // PlayerKey 1 is live for the MultiplayerOnly gate.
                target: Some(1),
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();
        assert_eq!(next.energy, 2);
        assert_eq!(next.monsters[0].hp, 200);
        assert_eq!(next.monsters[1].hp, 180);
        assert_eq!(
            next.monsters[0].nonowner_same_side_powered_damage_results_this_turn,
            2
        );
        assert_eq!(
            next.monsters[1].nonowner_same_side_powered_damage_results_this_turn,
            3
        );
        assert_eq!(next.monsters[0].owner_powered_damage_results_this_turn, 0);
        assert_eq!(next.monsters[1].owner_powered_damage_results_this_turn, 1);
        assert_eq!(next.piles.get(PileId::Discard).as_slice()[0].uid, 7);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::engine::Event::CardResolved {
                uid: 7,
                pile: PileId::Discard
            }
        )));

        let (mut solo, catalog, _) = gang_up_fixture(0, 2);
        solo.multiplayer_ally_key = 0;
        let before = solo.clone();
        let mut refused_events = Vec::new();
        assert!(
            apply_action_into(
                &solo,
                &catalog,
                &Action::Play {
                    uid: 7,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut refused_events,
            )
            .is_err()
        );
        assert_eq!(solo, before);
        assert!(refused_events.is_empty());
    }

    #[test]
    fn gang_up_card_source_uid_reaches_the_louse_listener() {
        let (mut state, catalog, gang_up) = gang_up_fixture(0, 1);
        let source = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.monsters_mut()[0].kind = MonsterKind::LouseProgenitor;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::CurlUp, SlotWire::Int, 3);
        let mut events = Vec::new();
        run_gang_up_body(
            &mut state,
            &catalog,
            gang_up,
            7,
            (Some(0), None, 0, &[CompiledArg::I(5)]),
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].curl_up_card_uid, 7);
        assert_eq!(state.monsters[0].powers.value(PowerId::CurlUp), 3);
    }

    #[test]
    fn gang_up_malformed_overflow_and_terminal_paths_are_atomic() {
        let malformed = [
            (Some(0), Some(1), 0, &[CompiledArg::I(5)][..]),
            (Some(0), None, 1, &[CompiledArg::I(5)][..]),
            (None, None, 0, &[CompiledArg::I(5)][..]),
            (Some(0), None, 0, &[CompiledArg::I(7)][..]),
        ];
        for (target, selection, x_value, args) in malformed {
            let (mut state, catalog, gang_up) = gang_up_fixture(0, 2);
            let source = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            let before = state.clone();
            let mut events = Vec::new();
            assert!(
                run_gang_up_body(
                    &mut state,
                    &catalog,
                    gang_up,
                    7,
                    (target, selection, x_value, args),
                    &mut events,
                )
                .is_err()
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        for duplicate in [false, true] {
            let (mut state, catalog, gang_up) = gang_up_fixture(0, 2);
            let source = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
            if duplicate {
                state.piles.get_mut(PileId::Play).make_mut().push(source);
                state.piles.get_mut(PileId::Discard).make_mut().push(source);
            }
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                run_gang_up_body(
                    &mut state,
                    &catalog,
                    gang_up,
                    7,
                    (Some(0), None, 0, &[CompiledArg::I(5)]),
                    &mut events,
                ),
                Err(EngineRefusal::ActiveCardNotUnique {
                    uid: 7,
                    matches: if duplicate { 2 } else { 0 },
                })
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let (mut dead, catalog, gang_up) = gang_up_fixture(0, 2);
        dead.monsters_mut()[0].hp = 0;
        let source = dead.piles.get_mut(PileId::Hand).make_mut().remove(0);
        dead.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = dead.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_gang_up_body(
                &mut dead,
                &catalog,
                gang_up,
                7,
                (Some(0), None, 0, &[CompiledArg::I(5)]),
                &mut events,
            ),
            Err(EngineRefusal::BadTarget(0))
        );
        assert_eq!(dead, before);
        assert!(events.is_empty());

        let (mut overflow, catalog, gang_up) = gang_up_fixture(0, 2);
        overflow.monsters_mut()[0].owner_powered_damage_results_this_turn = i32::MAX;
        let source = overflow.piles.get_mut(PileId::Hand).make_mut().remove(0);
        overflow.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_gang_up_body(
                &mut overflow,
                &catalog,
                gang_up,
                7,
                (Some(0), None, 0, &[CompiledArg::I(5)]),
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "owner_powered_damage_results_this_turn"
            ))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        // The public manual action has already paid energy, moved the source,
        // and published CardPlayed by the time Gang Up reads this target-local
        // history. Its late refusal must roll the complete action back.
        let (manual_overflow, catalog, _) = gang_up_fixture(0, 2);
        let mut manual_overflow = manual_overflow;
        manual_overflow.monsters_mut()[0].owner_powered_damage_results_this_turn = i32::MAX;
        let before = manual_overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &manual_overflow,
                &catalog,
                &Action::Play {
                    uid: 7,
                    target: Some(0),
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "owner_powered_damage_results_this_turn"
            ))
        );
        assert_eq!(manual_overflow, before);
        assert!(events.is_empty());

        // Direct AutoPlay rolls its target before the shared prefix. The same
        // late refusal must restore that RNG draw as well as piles, resources,
        // history, and events.
        let (mut autoplay_overflow, catalog, _) = gang_up_fixture(0, 2);
        autoplay_overflow.monsters_mut()[0].owner_powered_damage_results_this_turn = i32::MAX;
        let source = autoplay_overflow.piles.get(PileId::Hand).as_slice()[0];
        let before = autoplay_overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            autoplay_collected_cards(&mut autoplay_overflow, &catalog, &[source], &mut events),
            Err(EngineRefusal::CounterOverflow(
                "owner_powered_damage_results_this_turn"
            ))
        );
        assert_eq!(autoplay_overflow, before);
        assert!(events.is_empty());

        // Boundary history is signed Int32. Even Gang Up+'s largest operand
        // cannot overflow the signed Int64 damage arithmetic at that bound;
        // the representable result-counter increment above is the live edge.
        assert_eq!(
            7_i64
                .checked_mul(i64::from(i32::MAX))
                .and_then(|scaled| 5_i64.checked_add(scaled)),
            Some(15_032_385_534)
        );

        let (mut ending, catalog, gang_up) = gang_up_fixture(0, 2);
        let source = ending.piles.get_mut(PileId::Hand).make_mut().remove(0);
        ending.piles.get_mut(PileId::Play).make_mut().push(source);
        ending.history.over = true;
        let before = ending.clone();
        let mut events = Vec::new();
        run_gang_up_body(
            &mut ending,
            &catalog,
            gang_up,
            7,
            (Some(0), None, 0, &[CompiledArg::I(5)]),
            &mut events,
        )
        .unwrap();
        assert_eq!(ending, before);
        assert!(events.is_empty());
    }

    fn brightest_flame_fixture(upgrade: u8, draw_count: u32) -> (HotState, Catalog, CardIdentity) {
        let flame = identity(CardId::BrightestFlame, upgrade);
        let mut builder = CatalogBuilder::new();
        builder.intern(flame).unwrap();
        builder.intern(DEFEND).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 10;
        state.max_hp = 10;
        state.energy = 1;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(card(&catalog, &flame, 1));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((0..draw_count).map(|offset| card(&catalog, &DEFEND, 10 + offset)));
        (state, catalog, flame)
    }

    fn run_brightest_flame(
        state: &mut HotState,
        catalog: &Catalog,
        flame: CardIdentity,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let args = if flame.upgrade == 0 {
            [CompiledArg::I(2), CompiledArg::I(2), CompiledArg::I(2)]
        } else {
            [CompiledArg::I(3), CompiledArg::I(3), CompiledArg::I(2)]
        };
        run_card_step(
            StepKind::BrightestFlameExact,
            flame,
            state,
            catalog,
            &args,
            events,
        )
    }

    fn apotheosis_fixture() -> (HotState, Catalog, CardIdentity) {
        let apotheosis = identity(CardId::Apotheosis, 0);
        let mut builder = CatalogBuilder::new();
        for identity in [
            apotheosis,
            identity(CardId::StrikeIronclad, 0),
            identity(CardId::DefendIronclad, 0),
            identity(CardId::Zap, 0),
            identity(CardId::Bash, 0),
            identity(CardId::KinglyPunch, 0),
        ] {
            builder.intern(identity).unwrap();
        }
        builder.intern_all_card_upgrade_closure().unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(card(&catalog, &apotheosis, 1));
        (state, catalog, apotheosis)
    }

    fn run_apotheosis(
        state: &mut HotState,
        catalog: &Catalog,
        apotheosis: CardIdentity,
        operands: (Option<usize>, Option<u32>, i64, &[CompiledArg]),
    ) -> (Result<(), EngineRefusal>, Vec<crate::engine::Event>) {
        let spec = *catalog.spec(catalog.atom(&apotheosis).unwrap()).unwrap();
        let (target, selection, x_value, args) = operands;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 1,
            target,
            selection,
            x_value,
            args,
            events: &mut events,
        };
        let result = crate::engine::play::with_test_active_play(1, || {
            upgrade_all_combat_except_source_exact(&mut ctx)
        });
        (result, events)
    }

    #[test]
    fn apotheosis_two_carriers_program_and_native_pile_order_are_exact() {
        let carriers = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps.iter().find_map(|step| {
                    (step.kind == StepKind::UpgradeAllCombatExceptSourceExact).then_some((
                        row.id,
                        row.upgrade,
                        step.args,
                    ))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (CardId::Apotheosis, 0, &[][..]),
                (CardId::Apotheosis, 1, &[][..]),
            ]
        );
        assert_eq!(
            APOTHEOSIS_PILE_ORDER,
            [
                PileId::Hand,
                PileId::Draw,
                PileId::Discard,
                PileId::Exhaust,
                PileId::Play,
            ]
        );
        for upgrade in 0..=1 {
            let identity = identity(CardId::Apotheosis, upgrade);
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
            assert!(apotheosis_program_is_exact(&catalog, spec));
        }
    }

    #[test]
    fn apotheosis_upgrades_all_five_piles_once_and_excludes_only_its_uid() {
        let (mut state, catalog, apotheosis) = apotheosis_fixture();
        let placements = [
            (PileId::Hand, CardId::Apotheosis, 2),
            (PileId::Hand, CardId::StrikeIronclad, 3),
            (PileId::Draw, CardId::DefendIronclad, 4),
            (PileId::Discard, CardId::Zap, 5),
            (PileId::Exhaust, CardId::Bash, 6),
            (PileId::Play, CardId::KinglyPunch, 7),
        ];
        for (pile, id, uid) in placements {
            state
                .piles
                .get_mut(pile)
                .make_mut()
                .push(card(&catalog, &identity(id, 0), uid));
        }
        let mut sibling_state = state.card_states.get(2);
        sibling_state.local_cost_modifiers =
            crate::hot::LocalCostModifiers::from_rows(vec![LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 9,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]);
        state.card_states.set(2, sibling_state);
        let before_uids = APOTHEOSIS_PILE_ORDER
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .map(|card| card.uid)
            .collect::<Vec<_>>();

        let (result, events) =
            run_apotheosis(&mut state, &catalog, apotheosis, (None, None, 0, &[]));
        assert_eq!(result, Ok(()));
        assert!(events.is_empty());
        assert_eq!(
            APOTHEOSIS_PILE_ORDER
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            before_uids,
            "the frozen native walk cannot reorder or move a physical card"
        );
        for uid in 1..=7 {
            let live = APOTHEOSIS_PILE_ORDER
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| card.uid == uid)
                .unwrap();
            let live_identity = catalog.spec(live.atom).unwrap().identity;
            assert_eq!(
                live_identity.upgrade,
                if uid == 1 { 0 } else { 1 },
                "source exclusion is physical-uid based"
            );
        }
        assert_eq!(
            state.card_states.get(2).local_cost_modifiers.as_slice()[0].amount,
            catalog
                .spec(catalog.atom(&identity(CardId::Apotheosis, 1)).unwrap())
                .unwrap()
                .cost,
            "the shared writer applies the native lowered-base Set clamp"
        );
    }

    #[test]
    fn apotheosis_ending_gate_and_malformed_operands_are_atomic() {
        let (mut ending, catalog, apotheosis) = apotheosis_fixture();
        ending
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, &STRIKE, 2));
        ending.history.over = true;
        let before = ending.clone();
        assert_eq!(
            run_apotheosis(&mut ending, &catalog, apotheosis, (None, None, 0, &[])).0,
            Ok(())
        );
        assert_eq!(ending, before);

        for operands in [
            (Some(0), None, 0, &[][..]),
            (None, Some(4), 0, &[][..]),
            (None, None, 1, &[][..]),
            (None, None, 0, &[CompiledArg::I(1)][..]),
        ] {
            let (mut malformed, catalog, apotheosis) = apotheosis_fixture();
            malformed
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(&catalog, &STRIKE, 2));
            let before = malformed.clone();
            let (result, events) = run_apotheosis(&mut malformed, &catalog, apotheosis, operands);
            assert_eq!(
                result,
                Err(EngineRefusal::MalformedArgs(
                    "upgrade_all_combat_except_source_exact"
                ))
            );
            assert_eq!(malformed, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn apotheosis_preflight_refuses_missing_next_atom_and_duplicate_uids_atomically() {
        let apotheosis = identity(CardId::Apotheosis, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(apotheosis).unwrap();
        builder.intern(STRIKE).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(catalog.atom(&apotheosis).unwrap()).unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(card(&catalog, &apotheosis, 1));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, &STRIKE, 2));
        let before = state.clone();
        assert!(matches!(
            preflight_apotheosis_upgrade(&state, &catalog, 1, &spec),
            Err(EngineRefusal::UnknownMintIdentity(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 1,
                enchantment: None,
            }))
        ));
        assert_eq!(state, before);

        let (mut duplicate, complete, apotheosis) = apotheosis_fixture();
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&complete, &STRIKE, 2));
        duplicate
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(&complete, &DEFEND, 2));
        let spec = *complete.spec(complete.atom(&apotheosis).unwrap()).unwrap();
        let before = duplicate.clone();
        assert_eq!(
            preflight_apotheosis_upgrade(&duplicate, &complete, 1, &spec),
            Err(EngineRefusal::MalformedArgs(
                "CardCmd::Upgrade duplicate frozen identity"
            ))
        );
        assert_eq!(duplicate, before);
    }

    #[test]
    fn apotheosis_complete_action_rolls_back_a_post_upgrade_refusal() {
        let (mut state, catalog, apotheosis) = apotheosis_fixture();
        let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, card(&catalog, &STRIKE, 2)]);
        state.energy = 3;
        state.history.card_plays_finished_combat = i32::MAX;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let events_before = events.clone();

        assert_eq!(
            play_card(&mut state, &catalog, 1, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
        assert_eq!(
            catalog.spec(source.atom).unwrap().identity,
            apotheosis,
            "the initial canonical source is the exact L0 row"
        );
    }

    #[test]
    fn brightest_flame_carriers_and_admission_delta_are_exactly_two_rows() {
        let carriers = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps
                    .iter()
                    .find(|step| step.kind == StepKind::BrightestFlameExact)
                    .map(|step| (row.id, row.upgrade, row.cost, step.args))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::BrightestFlame,
                    0,
                    0,
                    &[Arg::I(2), Arg::I(2), Arg::I(2)][..]
                ),
                (
                    CardId::BrightestFlame,
                    1,
                    0,
                    &[Arg::I(3), Arg::I(3), Arg::I(2)][..]
                ),
            ]
        );
        let admitted = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::BrightestFlameExact)
                    && row
                        .steps
                        .iter()
                        .all(|step| crate::steps::is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![(CardId::BrightestFlame, 0), (CardId::BrightestFlame, 1)]
        );
        assert!(crate::steps::is_implemented(StepKind::BrightestFlameExact));
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::BrightestFlameExact)
        );
    }

    #[test]
    fn brightest_flame_orders_energy_draw_loss_and_owner_listeners() {
        for (upgrade, expected_energy, expected_draws) in [(0, 3, 2), (1, 4, 3)] {
            let (mut state, catalog, flame) = brightest_flame_fixture(upgrade, expected_draws);
            state.powers.set(PowerId::Rupture, SlotWire::Int, 1);
            state.powers.set(PowerId::Inferno, SlotWire::Int, 3);
            let mut events = Vec::new();
            run_brightest_flame(&mut state, &catalog, flame, &mut events).unwrap();

            assert_eq!(state.energy, expected_energy);
            assert_eq!(state.cards_drawn_combat, expected_draws as i32);
            assert_eq!(state.piles.get(PileId::Hand).len(), expected_draws as usize);
            assert_eq!(state.hp, 8);
            assert_eq!(state.max_hp, 8);
            assert_eq!(state.powers.value(PowerId::Strength), 1);
            assert_eq!(state.monsters[0].hp, 17);
            let player_damage = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        crate::engine::Event::PlayerDamaged { hp_lost: 2, .. }
                    )
                })
                .unwrap();
            let rupture = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        crate::engine::Event::PowerChanged {
                            subject: Subject::Player,
                            power: PowerId::Strength,
                            amount: 1
                        }
                    )
                })
                .unwrap();
            let inferno = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        crate::engine::Event::MonsterDamaged { unblocked: 3, .. }
                    )
                })
                .unwrap();
            assert!(player_damage < rupture && rupture < inferno);
        }
    }

    #[test]
    fn brightest_flame_nested_damage_modifiers_then_silent_cap_are_exact() {
        let (mut intangible, catalog, flame) = brightest_flame_fixture(0, 2);
        intangible.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        let mut events = Vec::new();
        run_brightest_flame(&mut intangible, &catalog, flame, &mut events).unwrap();
        assert_eq!((intangible.hp, intangible.max_hp), (8, 8));
        assert!(events.iter().any(|event| matches!(
            event,
            crate::engine::Event::PlayerDamaged {
                hp_lost: 1,
                hp: 9,
                ..
            }
        )));

        let (mut buffered, catalog, flame) = brightest_flame_fixture(0, 2);
        buffered.powers.set(PowerId::Buffer, SlotWire::Int, 1);
        let mut events = Vec::new();
        run_brightest_flame(&mut buffered, &catalog, flame, &mut events).unwrap();
        assert_eq!((buffered.hp, buffered.max_hp), (8, 8));
        assert_eq!(buffered.powers.value(PowerId::Buffer), 0);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::engine::Event::PlayerDamaged {
                hp_lost: 0,
                hp: 10,
                ..
            }
        )));
    }

    #[test]
    fn brightest_flame_continues_after_draw_terminal_but_lethal_loss_skips_listeners() {
        let (mut draw_terminal, catalog, flame) = brightest_flame_fixture(0, 2);
        draw_terminal.monsters_mut()[0].hp = 1;
        draw_terminal
            .powers
            .set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            draw_terminal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        draw_terminal.fanouts.set_cacophony_left(1);
        run_brightest_flame(&mut draw_terminal, &catalog, flame, &mut Vec::new()).unwrap();
        assert!(draw_terminal.history.over);
        assert_eq!((draw_terminal.hp, draw_terminal.max_hp), (8, 8));
        assert_eq!(draw_terminal.piles.get(PileId::Hand).len(), 1);

        let (mut lethal, catalog, flame) = brightest_flame_fixture(0, 2);
        lethal.hp = 1;
        lethal.max_hp = 2;
        lethal.powers.set(PowerId::Rupture, SlotWire::Int, 4);
        lethal.powers.set(PowerId::Inferno, SlotWire::Int, 7);
        let mut events = Vec::new();
        run_brightest_flame(&mut lethal, &catalog, flame, &mut events).unwrap();
        assert_eq!((lethal.hp, lethal.max_hp), (0, 1));
        assert!(lethal.history.over);
        assert_eq!(lethal.powers.value(PowerId::Strength), 0);
        assert_eq!(lethal.monsters[0].hp, 20);
    }

    #[test]
    fn brightest_flame_accepts_non_null_source_and_refusals_are_whole_body_atomic() {
        let (mut late, catalog, flame) = brightest_flame_fixture(0, 2);
        late.max_hp = i32::MIN;
        let before = late.clone();
        let mut events = vec![crate::engine::Event::CardDrawn { uid: 99 }];
        let events_before = events.clone();
        assert_eq!(
            run_brightest_flame(&mut late, &catalog, flame, &mut events),
            Err(EngineRefusal::CounterOverflow("card-sourced max HP"))
        );
        assert_eq!(late, before);
        assert_eq!(events, events_before);

        let (mut duplicate, catalog, flame) = brightest_flame_fixture(0, 2);
        duplicate
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(card(&catalog, &flame, 1));
        let before = duplicate.clone();
        assert_eq!(
            run_brightest_flame(&mut duplicate, &catalog, flame, &mut Vec::new()),
            Err(EngineRefusal::ActiveCardNotUnique { uid: 1, matches: 2 })
        );
        assert_eq!(duplicate, before);

        let (mut direct, catalog, flame) = brightest_flame_fixture(0, 2);
        let source = direct.piles.get_mut(PileId::Play).make_mut().remove(0);
        direct
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let max_hp_before = direct.max_hp;
        run_brightest_flame(&mut direct, &catalog, flame, &mut Vec::new()).unwrap();
        assert_eq!(direct.max_hp, max_hp_before - 2);
        assert_eq!(direct.piles.get(PileId::Discard).as_slice(), &[source]);

        let (mut malformed, catalog, flame) = brightest_flame_fixture(0, 2);
        let spec = *catalog.spec(catalog.atom(&flame).unwrap()).unwrap();
        let before = malformed.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut malformed,
            catalog: &catalog,
            spec: &spec,
            source_uid: 1,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(2), CompiledArg::I(2), CompiledArg::I(2)],
            events: &mut events,
        };
        assert_eq!(
            brightest_flame_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("brightest_flame_exact action"))
        );
        assert_eq!(malformed, before);
        assert!(events.is_empty());
    }

    #[test]
    fn brightest_flame_public_play_preserves_physical_source_and_result_routing() {
        for upgrade in [0, 1] {
            let flame = identity(CardId::BrightestFlame, upgrade);
            let mut builder = CatalogBuilder::new();
            builder.intern(flame).unwrap();
            builder.intern(DEFEND).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 10;
            state.max_hp = 10;
            state.energy = 1;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(&catalog, &flame, 7));
            let draws = 2 + u32::from(upgrade);
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((0..draws).map(|offset| card(&catalog, &DEFEND, 20 + offset)));
            let mut events = Vec::new();
            let next = apply_action_into(
                &state,
                &catalog,
                &Action::Play {
                    uid: 7,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut events,
            )
            .unwrap();
            assert_eq!(next.energy, 3 + i16::from(upgrade));
            assert_eq!((next.hp, next.max_hp), (8, 8));
            assert_eq!(next.piles.get(PileId::Hand).len(), draws as usize);
            assert_eq!(next.piles.get(PileId::Discard).as_slice()[0].uid, 7);
            assert!(events.iter().any(|event| matches!(
                event,
                crate::engine::Event::CardResolved {
                    uid: 7,
                    pile: PileId::Discard
                }
            )));
        }
    }

    #[test]
    fn rally_refuses_lethal_juggernaut_before_mutation_and_allows_nonlethal_order() {
        let rally = identity(CardId::Rally, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(rally).unwrap();
        let catalog = builder.build();
        let mut lethal = HotState::at_defaults();
        lethal.hp = 50;
        lethal.multiplayer_ally_key = 1;
        lethal
            .fanouts
            .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                key: 1,
                ..crate::hot::MultiplayerAllyState::default()
            });
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 6));
        lethal.powers.set(PowerId::Juggernaut, SlotWire::Int, 6);
        assert!(
            lethal
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        lethal.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        let before = lethal.clone();
        let mut events = vec![crate::engine::Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        }];
        let events_before = events.clone();

        assert_eq!(
            run_card_step(
                StepKind::RallyExact,
                rally,
                &mut lethal,
                &catalog,
                &[CompiledArg::I(12)],
                &mut events,
            ),
            Err(EngineRefusal::PowerOrderNotModeled(
                "Rally owner position with lethal Juggernaut"
            ))
        );
        assert_eq!(lethal, before);
        assert_eq!(events, events_before);

        let mut nonlethal = before;
        nonlethal.monsters_mut()[0].hp = 7;
        let rng_before = nonlethal.rng.get(RngStream::Targets).counter;
        run_card_step(
            StepKind::RallyExact,
            rally,
            &mut nonlethal,
            &catalog,
            &[CompiledArg::I(12)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(nonlethal.block, 12);
        assert_eq!(nonlethal.fanouts.multiplayer_ally().block, 12);
        assert_eq!(nonlethal.monsters[0].hp, 1);
        assert_eq!(
            nonlethal.rng.get(RngStream::Targets).counter,
            rng_before + 1
        );
    }

    #[test]
    fn beacon_is_unique_preserves_acquisition_order_and_panic_stacks_after_block() {
        let beacon = identity(CardId::BeaconOfHope, 0);
        let panic = identity(CardId::PanicButton, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(beacon).unwrap();
        builder.intern(panic).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );
        let mut events = Vec::new();

        run_card_step(
            StepKind::BeaconOfHopeExact,
            beacon,
            &mut state,
            &catalog,
            &[CompiledArg::I(1)],
            &mut events,
        )
        .unwrap();
        run_card_step(
            StepKind::BeaconOfHopeExact,
            beacon,
            &mut state,
            &catalog,
            &[CompiledArg::I(1)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::BeaconOfHope), 1);
        assert_eq!(
            state.fanouts.after_block_gained_order(),
            &[PowerId::Juggernaut, PowerId::BeaconOfHope]
        );

        run_card_step(
            StepKind::PanicButtonExact,
            panic,
            &mut state,
            &catalog,
            &[CompiledArg::I(30), CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.block, 30);
        assert_eq!(state.monsters[0].hp, 19);
        assert_eq!(state.powers.value(PowerId::NoBlock), 2);

        run_card_step(
            StepKind::PanicButtonExact,
            panic,
            &mut state,
            &catalog,
            &[CompiledArg::I(30), CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        assert_eq!(state.block, 30, "existing NoBlock zeroes the second gain");
        assert_eq!(state.powers.value(PowerId::NoBlock), 4);
    }

    #[test]
    fn panic_overflow_and_terminal_listener_are_atomic_and_ending_gated() {
        let panic = identity(CardId::PanicButton, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(panic).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        state.powers.set(PowerId::NoBlock, SlotWire::Int, i32::MAX);
        let snapshot = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_card_step(
                StepKind::PanicButtonExact,
                panic,
                &mut state,
                &catalog,
                &[CompiledArg::I(30), CompiledArg::I(2)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("no block"))
        );
        assert_eq!(state, snapshot);
        assert!(events.is_empty());

        state.powers.set(PowerId::NoBlock, SlotWire::Int, 0);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 10);
        assert!(
            state
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );
        run_card_step(
            StepKind::PanicButtonExact,
            panic,
            &mut state,
            &catalog,
            &[CompiledArg::I(30), CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        assert!(state.history.over);
        assert_eq!(state.block, 30);
        assert_eq!(state.powers.value(PowerId::NoBlock), 0);
    }

    /// The wave's honesty property: the manifest claims nothing this file
    /// cannot actually play, and claims everything it can.
    #[test]
    fn the_manifest_claims_nothing_this_family_has_not_ported() {
        let manifest = crate::engine::capability_manifest();
        for kind in ESCALATED {
            assert!(
                !manifest.steps.contains(&kind),
                "{:?} is escalated but the manifest claims it",
                kind.as_str()
            );
            assert!(!IMPLEMENTED.contains(&kind));
        }
        for kind in IMPLEMENTED {
            assert!(
                manifest.steps.contains(kind),
                "{:?} has a body here but the manifest does not claim it",
                kind.as_str()
            );
            assert!(
                OWNED.contains(kind),
                "{:?} is not this file's",
                kind.as_str()
            );
        }
        assert_eq!(IMPLEMENTED.len() + ESCALATED.len(), OWNED.len());
    }

    /// Every stub still refuses **by its own name**, through the generated
    /// dispatch. A stub weakened into a silent `Ok(())` would make an
    /// unmodeled card play as if the step were absent — the exact I5 failure
    /// the escalation rule exists to prevent.
    #[test]
    fn every_escalated_stub_refuses_by_its_own_kind() {
        let (mut state, catalog) = ctx_state();
        for kind in ESCALATED {
            let spec = *catalog.spec(catalog.atom(&STRIKE).unwrap()).unwrap();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 0,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut events,
            };
            assert_eq!(
                apply_step(kind, &mut ctx),
                Err(EngineRefusal::StepKindNotModeled(kind)),
                "{:?} must refuse by name",
                kind.as_str()
            );
            assert!(
                events.is_empty(),
                "{:?} emitted an event before refusing",
                kind.as_str()
            );
        }
    }

    #[test]
    fn impatience_draws_only_while_the_hand_holds_no_attack() {
        let (mut state, catalog) = ctx_state();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, &DEFEND, 1));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([card(&catalog, &STRIKE, 2), card(&catalog, &STRIKE, 3)]);
        run(
            StepKind::DrawIfNoHandAttacks,
            &mut state,
            &catalog,
            None,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 3);
        assert!(state.piles.get(PileId::Draw).is_empty());

        // The hand now holds two Strikes, so a replay is a no-op with no
        // Draw command at all.
        let drawn_before = state.cards_drawn_combat;
        run(
            StepKind::DrawIfNoHandAttacks,
            &mut state,
            &catalog,
            None,
            &[CompiledArg::I(2)],
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 3);
        assert_eq!(state.cards_drawn_combat, drawn_before);
    }

    #[test]
    fn entrench_doubles_current_block_through_the_card_gain_funnel() {
        let (mut state, catalog) = ctx_state();
        state.block = 7;
        run(
            StepKind::GainCurrentBlockCardUnpoweredExact,
            &mut state,
            &catalog,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(state.block, 14);
        assert_eq!(state.history.card_block_gains, 1);

        // Zero block gains zero: no store and no block-gain count, exactly
        // like the powered funnel's positive-only branch.
        state.block = 0;
        state.history.card_block_gains = 0;
        run(
            StepKind::GainCurrentBlockCardUnpoweredExact,
            &mut state,
            &catalog,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(state.block, 0);
        assert_eq!(state.history.card_block_gains, 0);

        // The command's entry gate no-ops once combat is ending.
        state.block = 9;
        state.history.over = true;
        run(
            StepKind::GainCurrentBlockCardUnpoweredExact,
            &mut state,
            &catalog,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(state.block, 9);
    }

    #[test]
    fn enlightenment_rewrites_the_live_hand_in_order_and_forces_exact_piles() {
        let mut builder = CatalogBuilder::new();
        builder.intern(STRIKE).unwrap();
        builder.intern(DEFEND).unwrap();
        let source = enlightenment(0);
        let source_atom = builder.intern(source).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([card(&catalog, &STRIKE, 11), card(&catalog, &DEFEND, 7)]);
        state.card_states.append_local_cost_modifier(
            11,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: 2,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 99,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1), CompiledArg::I(6), CompiledArg::B(true)],
            events: &mut events,
        };

        enlightenment_exact(&mut ctx).unwrap();

        assert!(state.exact_piles);
        assert!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| { card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0 })
        );
        let strike_rows = state
            .card_states
            .get_ref(11)
            .unwrap()
            .local_cost_modifiers
            .as_slice();
        assert_eq!(strike_rows.len(), 2);
        assert_eq!(strike_rows[0].kind, LocalCostModifierKind::Add);
        assert_eq!(strike_rows[1].kind, LocalCostModifierKind::Set);
        assert_eq!(strike_rows[1].amount, 1);
        assert_eq!(
            strike_rows[1].expiration,
            LocalCostExpiration::ThisTurnOrPlayed
        );
        assert!(strike_rows[1].reduce_only);
        assert_eq!(
            state
                .card_states
                .get_ref(11)
                .unwrap()
                .local_cost_modifiers
                .resolve(1),
            1
        );
        assert_eq!(
            state
                .card_states
                .get_ref(7)
                .unwrap()
                .local_cost_modifiers
                .resolve(9),
            1
        );
        assert!(events.is_empty());
    }

    #[test]
    fn upgraded_enlightenment_writes_combat_rows_and_rejects_other_shapes() {
        let mut builder = CatalogBuilder::new();
        builder.intern(STRIKE).unwrap();
        let source = enlightenment(1);
        let source_atom = builder.intern(source).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, &STRIKE, 5));
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 99,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1), CompiledArg::I(0), CompiledArg::B(true)],
            events: &mut events,
        };
        enlightenment_exact(&mut ctx).unwrap();
        assert_eq!(
            state
                .card_states
                .get_ref(5)
                .unwrap()
                .local_cost_modifiers
                .as_slice()[0]
                .expiration,
            LocalCostExpiration::ThisCombat
        );

        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 99,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1), CompiledArg::I(6), CompiledArg::B(true)],
            events: &mut events,
        };
        assert_eq!(
            enlightenment_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("enlightenment_exact"))
        );
        assert_eq!(
            state
                .card_states
                .get_ref(5)
                .unwrap()
                .local_cost_modifiers
                .as_slice()
                .len(),
            1
        );
    }

    #[test]
    fn rend_counts_live_instances_by_cardinality_not_amount() {
        let (mut state, catalog) = ctx_state();
        state.monsters_mut()[0].hp = 60;
        {
            let monster = &mut state.monsters_mut()[0];
            // Weak 3 and Poison 2 are two instances, not five; Thorns and
            // Burrowed are Type-1 rows Python validates but does not count.
            monster.powers.set(PowerId::Weak, SlotWire::Int, 3);
            monster.powers.set(PowerId::Poison, SlotWire::Int, 2);
            monster.powers.set(PowerId::Thorns, SlotWire::Int, 1);
            monster.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
            monster.misery_debuff_order.push(MiseryToken::Weak);
            monster.misery_debuff_order.push(MiseryToken::Poison);
        }
        run(
            StepKind::RendExact,
            &mut state,
            &catalog,
            Some(0),
            &[CompiledArg::I(10), CompiledArg::I(5)],
        )
        .unwrap();
        // Two counted instances: 10 + 5 * 2 = 20 (Thorns retaliates for 1
        // against the 50-HP player first).
        assert_eq!(state.monsters[0].hp, 40);
        assert_eq!(state.hp, 49);

        // Strength counts only while negative — the visible-type flip.
        let (mut state, catalog) = ctx_state();
        state.monsters_mut()[0].hp = 60;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, -2);
        run(
            StepKind::RendExact,
            &mut state,
            &catalog,
            Some(0),
            &[CompiledArg::I(10), CompiledArg::I(5)],
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 45);
    }

    #[test]
    fn rend_power_census_pins_every_admitted_monster_power_class() {
        let represented = [
            (PowerId::Adaptable, SlotWire::Int, 1, false),
            (PowerId::Artifact, SlotWire::Int, 1, false),
            // Not counted: `AsleepPower::get_Type` RVA `0x9f901` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Asleep, SlotWire::Int, 1, false),
            // SlumberPower::get_Type 0xa7eec also returns Type 1 (buff).
            (PowerId::Slumber, SlotWire::Int, 3, false),
            // Not counted: `BattlewornDummyTimeLimitPower::get_Type` RVA
            // `0x9fb39` returns 1 (#3357).
            (PowerId::BattlewornTimeLimit, SlotWire::Int, 3, false),
            (PowerId::Burrowed, SlotWire::Bool, 1, false),
            (PowerId::CurlUp, SlotWire::Int, 18, false),
            (PowerId::Dexterity, SlotWire::Int, 4, false),
            (PowerId::Conqueror, SlotWire::Int, 2, true),
            (PowerId::Debilitate, SlotWire::Int, 2, true),
            (PowerId::Demise, SlotWire::Int, 9, true),
            (PowerId::Doom, SlotWire::Int, 7, true),
            (PowerId::Enrage, SlotWire::Int, 3, false),
            (PowerId::EscapeArtist, SlotWire::Int, 5, false),
            (PowerId::Flutter, SlotWire::Int, 5, false),
            (PowerId::HatchPower, SlotWire::Int, 2, false),
            (PowerId::Hang, SlotWire::Int, 2, true),
            // Not counted: `HardToKillPower::get_Type` RVA `0xa3416` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::HardToKill, SlotWire::Int, 9, false),
            (PowerId::HeistGold, SlotWire::Int, 20, false),
            (PowerId::HighVoltage, SlotWire::Int, 2, false),
            (PowerId::Hive, SlotWire::Int, 1, false),
            (PowerId::Intangible, SlotWire::Int, 1, false),
            (PowerId::IsHatched, SlotWire::Bool, 1, false),
            (PowerId::Mplating, SlotWire::Int, 19, false),
            (PowerId::Nemesis, SlotWire::Int, 1, false),
            (PowerId::Oblivion, SlotWire::Int, 2, false),
            (PowerId::PainfulStabs, SlotWire::Int, 1, false),
            (PowerId::PlowThreshold, SlotWire::Int, 160, true),
            (PowerId::Poison, SlotWire::Int, 2, true),
            // Not counted: `RavenousPower::get_Type` RVA `0xa639f` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Ravenous, SlotWire::Int, 5, false),
            // Not counted: `RampartPower::get_Type` RVA `0xa6343` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Rampart, SlotWire::Int, 25, false),
            (PowerId::Ritual, SlotWire::Int, 2, false),
            (PowerId::Sandpit, SlotWire::Int, 4, false),
            (PowerId::Secondary, SlotWire::Bool, 1, false),
            (PowerId::SicEm, SlotWire::Int, 3, false),
            (PowerId::Shriek, SlotWire::Bool, 1, true),
            // Not counted: `SkittishPower::get_Type` RVA `0xa7a7f` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Skittish, SlotWire::Int, 7, false),
            (PowerId::Shrink, SlotWire::Int, 4, true),
            // Not counted: `SlipperyPower::get_Type` RVA `0xa7c20` returns 1,
            // and `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Slippery, SlotWire::Int, 3, false),
            (PowerId::Soar, SlotWire::Bool, 1, false),
            (PowerId::SteamPressure, SlotWire::Int, 20, false),
            (PowerId::Stock, SlotWire::Int, 2, false),
            (PowerId::StolenGold, SlotWire::Int, 20, false),
            (PowerId::Strangle, SlotWire::Int, 2, true),
            (PowerId::Strength, SlotWire::Int, -2, true),
            // Not counted: `SuckPower::get_Type` RVA `0xa8a3f` returns 1, and
            // `count_target_powers_for_rend` counts Type-2 instances only.
            (PowerId::Suck, SlotWire::Int, 3, false),
            (PowerId::TempStrength, SlotWire::Int, -3, false),
            // Not counted: `count_target_powers_for_rend` (frozen Python, deleted #2827)
            // counts non-temporary **Type-2** instances, and current-build IL
            // `TerritorialPower::get_Type` RVA `0xa9e53` returns 1.
            (PowerId::Territorial, SlotWire::Int, 1, false),
            (PowerId::Thorns, SlotWire::Int, 4, false),
            (PowerId::Vigor, SlotWire::Int, 5, false),
            (PowerId::Vital, SlotWire::Int, 6, false),
            (PowerId::Vuln, SlotWire::Int, 3, true),
            (PowerId::Weak, SlotWire::Int, 3, true),
        ];
        assert_eq!(represented.len() + 1, IMPLEMENTED_POWERS.len());
        assert!(IMPLEMENTED_POWERS.iter().all(|power| {
            *power == PowerId::Knockdown
                || represented
                    .iter()
                    .any(|(represented, _, _, _)| represented == power)
        }));

        for (power, wire, amount, counted) in represented {
            let owner = match power {
                PowerId::Adaptable | PowerId::Enrage | PowerId::Nemesis | PowerId::PainfulStabs => {
                    MonsterKind::TestSubject
                }
                PowerId::CurlUp => MonsterKind::LouseProgenitor,
                PowerId::BattlewornTimeLimit => MonsterKind::BattleFriendV2,
                PowerId::Dexterity => MonsterKind::TheForgotten,
                PowerId::Mplating => MonsterKind::FrogKnight,
                PowerId::PlowThreshold => MonsterKind::CeremonialBeast,
                PowerId::Soar => MonsterKind::OwlMagistrate,
                PowerId::Stock => MonsterKind::Axebot,
                PowerId::StolenGold => MonsterKind::GremlinMerc,
                PowerId::HeistGold => MonsterKind::FatGremlin,
                PowerId::Ritual => MonsterKind::CalcifiedCultist,
                PowerId::HatchPower | PowerId::IsHatched | PowerId::Secondary => {
                    MonsterKind::ToughEgg
                }
                PowerId::HighVoltage => MonsterKind::Zapbot,
                PowerId::EscapeArtist | PowerId::Flutter => MonsterKind::ThievingHopper,
                PowerId::Hive => MonsterKind::Entomancer,
                PowerId::Intangible => MonsterKind::SoulFysh,
                PowerId::Sandpit => MonsterKind::TheInsatiable,
                _ => MonsterKind::Toadpole,
            };
            let mut monster = HotMonster::new(owner, 60);
            monster.powers.set(power, wire, amount);
            assert_eq!(
                rend_target_power_count(&monster),
                i64::from(counted),
                "wrong Rend cardinality for {}",
                power.as_str()
            );
        }

        let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
        assert_eq!(rend_target_power_count(&monster), 0);
        for amount in [1, 7, i32::MAX] {
            monster.powers.set(PowerId::Doom, SlotWire::Int, amount);
            assert_eq!(
                rend_target_power_count(&monster),
                1,
                "Doom amount {amount} must still be one live instance"
            );
        }
        monster.powers.set(PowerId::Doom, SlotWire::Int, 0);
        assert_eq!(rend_target_power_count(&monster), 0);

        monster.powers.set(PowerId::Strength, SlotWire::Int, 8);
        assert_eq!(rend_target_power_count(&monster), 0);
        monster.kind = MonsterKind::BygoneEffigy;
        assert_eq!(rend_target_power_count(&monster), 1);
        monster.kind = MonsterKind::BowlbugRock;
        assert_eq!(rend_target_power_count(&monster), 1);
        // #2647 §6 witness 27. Imbalanced is present-by-kind and routed through
        // one shared predicate, so it contributes exactly one — never once per
        // reader, and nothing for a non-carrier. Slice B ORs a ledger into that
        // predicate and this is the pin that catches a double count.
        assert!(crate::engine::monsters::owner_carries_imbalanced(&monster));
        let carrier_count = rend_target_power_count(&monster);
        monster.kind = MonsterKind::Toadpole;
        assert!(!crate::engine::monsters::owner_carries_imbalanced(&monster));
        assert_eq!(carrier_count - rend_target_power_count(&monster), 1);

        // #2693 B1: the same cardinality through the *other* representation.
        // A ledger-attached Imbalanced contributes exactly one, and a ledger
        // carrying some other model contributes none — so the predicate reads
        // the model rather than merely "the ledger is non-empty".
        let mut attached = HotMonster::new(MonsterKind::BowlbugEgg, 60);
        let bare_count = rend_target_power_count(&attached);
        attached
            .misery_debuff_order
            .push_attachment(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Imbalanced,
                applier: crate::hot::Applier::Monster(0),
                amount: 1,
            });
        assert_eq!(rend_target_power_count(&attached) - bare_count, 1);

        let mut other_model = HotMonster::new(MonsterKind::BowlbugEgg, 60);
        other_model
            .misery_debuff_order
            .push_attachment(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Player,
                amount: -2,
            });
        assert_eq!(rend_target_power_count(&other_model), bare_count);

        let mut knockdown = HotMonster::new(MonsterKind::Toadpole, 60);
        knockdown.misery_debuff_order.push_knockdown(2);
        knockdown.misery_debuff_order.push_knockdown(3);
        assert_eq!(
            rend_target_power_count(&knockdown),
            2,
            "Rend counts each distinct Knockdown instance once"
        );
    }

    /// Exact #1445 replay: entry digest
    /// `66ebc0cfb5cfff6cd14ed00abf3c81ea6089843f62bb06e0a44da0fdb2779779`,
    /// seed `ZPJHU3WSH2`, TUNNELER_WEAK, then play Negative Pulse uid 0 and
    /// Rend uid 1 at slot 0.
    #[test]
    fn negative_pulse_then_rend_counts_the_live_doom_instance() {
        let negative_pulse = identity(CardId::NegativePulse, 0);
        let rend = identity(CardId::Rend, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(negative_pulse).unwrap();
        builder.intern(rend).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        state.next_card_uid = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Tunneler, 92));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([card(&catalog, &negative_pulse, 0), card(&catalog, &rend, 1)]);
        let mut events = Vec::new();

        state = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 92);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 7);
        assert_eq!(state.block, 5);

        state = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 77);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 7);
    }

    #[test]
    fn rend_refuses_a_dead_target_rather_than_skipping() {
        let (mut state, catalog) = ctx_state();
        state.monsters_mut()[1].hp = 0;
        assert_eq!(
            run(
                StepKind::RendExact,
                &mut state,
                &catalog,
                Some(1),
                &[CompiledArg::I(10), CompiledArg::I(5)],
            ),
            Err(EngineRefusal::BadTarget(1))
        );
        assert_eq!(
            run(
                StepKind::RendExact,
                &mut state,
                &catalog,
                Some(2),
                &[CompiledArg::I(10), CompiledArg::I(5)],
            ),
            Err(EngineRefusal::BadTarget(2))
        );
        assert_eq!(
            run(
                StepKind::RendExact,
                &mut state,
                &catalog,
                None,
                &[CompiledArg::I(10), CompiledArg::I(5)],
            ),
            Err(EngineRefusal::TargetMismatch { required: true })
        );
    }

    #[test]
    fn discovery_and_splash_execute_and_resume_for_all_character_owners() {
        use crate::boundary::HotBoundary;
        for owner in RewardPool::ALL {
            for source_id in [CardId::Discovery, CardId::Splash] {
                let source = identity(source_id, 0);
                let mut builder = CatalogBuilder::new();
                let atom = builder.intern(source).unwrap();
                let pool = if source_id == CardId::Discovery {
                    builder.owner_generation_pool(owner)
                } else {
                    builder.splash_attack_pool(owner)
                };
                for id in &pool {
                    builder.intern(identity(*id, 0)).unwrap();
                }
                let catalog = builder.build();
                let mut state = HotState::at_defaults();
                state.hp = 50;
                state.max_hp = 50;
                state.energy = 3;
                state.exact_piles = true;
                state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
                state.reward_card_pool = Some(owner);
                state.entropy_card_pool = Some(owner);
                state.fully_unlocked_card_pool_epochs = true;
                state.set_spectrum_shift_generation_pool(owner == RewardPool::Regent);
                state.rng.set(
                    RngStream::Generation,
                    RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                state.next_card_uid = 18;
                state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                    uid: 17,
                    atom,
                    flags: 0,
                });
                let (state, catalog) = if source_id == CardId::Splash {
                    let entry = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
                    let rebuilt = HotBoundary::catalog_from_canonical(&entry).unwrap();
                    (
                        HotBoundary::from_canonical(&entry, &rebuilt).unwrap(),
                        rebuilt,
                    )
                } else {
                    (state, catalog)
                };
                let pending = crate::engine::apply_action(
                    &state,
                    &catalog,
                    &crate::engine::Action::Play {
                        uid: 17,
                        target: None,
                        selection: crate::engine::SelectionRef::NONE,
                    },
                )
                .unwrap()
                .state;
                assert!(pending.pending.is_some(), "{owner:?} {source_id:?}");
                assert_eq!(
                    pending.rng.get(RngStream::Generation).counter,
                    pool.len() as u64 - 1
                );
                // Discovery can offer still-unmodeled downstream cards. Its
                // isolated body test does not claim whole-root admission.
                // Splash's current closure admits, so also prove persistence.
                let (reloaded, reloaded_catalog) = if source_id == CardId::Splash {
                    let doc = HotBoundary::try_to_canonical(&pending, &catalog).unwrap();
                    let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
                    (
                        HotBoundary::from_canonical(&doc, &rebuilt).unwrap(),
                        rebuilt,
                    )
                } else {
                    (pending.clone(), catalog.clone())
                };
                let wire: crate::exact_solve_v1::ExactSolveActionV1 = serde_json::from_value(
                    serde_json::json!({"kind":"select", "answer":{"kind":"option_index", "index":0}})).unwrap();
                let after = crate::engine::apply_action(
                    &reloaded,
                    &reloaded_catalog,
                    &wire.try_into().unwrap(),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "{owner:?} {source_id:?}: {e:?}; legal {:?}",
                        crate::engine::legal_actions(&reloaded, &reloaded_catalog)
                    )
                })
                .state;
                assert!(after.pending.is_none());
                assert!(after.frames.is_empty());
                assert_eq!(after.piles.get(PileId::Hand).len(), 1);
                let added = after.piles.get(PileId::Hand).as_slice()[0];
                assert!(pool.contains(&reloaded_catalog.spec(added.atom).unwrap().identity.id));
                assert_eq!(
                    after.rng.get(RngStream::Generation),
                    pending.rng.get(RngStream::Generation)
                );
            }
        }
    }

    #[test]
    fn jackpot_samples_three_generation_items_before_singular_insertion() {
        for owner in RewardPool::ALL {
            let source_identity = CardIdentity {
                id: CardId::Jackpot,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let source_atom = builder.intern(source_identity).unwrap();
            for id in owner_zero_cost_pool(owner, None) {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            let catalog = builder.build();
            let spec = *catalog.spec(source_atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.reward_card_pool = Some(owner);
            state.fully_unlocked_card_pool_epochs = true;
            state.entropy_card_pool = Some(owner);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 17,
                atom: source_atom,
                flags: 0,
            });
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let before = state.rng.get(RngStream::Generation).counter;
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 17,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(25), CompiledArg::I(3)],
                events: &mut events,
            };

            jackpot_exact(&mut ctx).unwrap();

            assert_eq!(state.monsters[0].hp, 75);
            assert_eq!(state.rng.get(RngStream::Generation).counter, before + 3);
            assert_eq!(state.piles.get(PileId::Hand).len(), 3);
            assert_eq!(state.history.owner_generated_cards_combat, 3);
            assert!(state.exact_piles);
        }
    }

    #[test]
    fn metamorphosis_samples_the_complete_batch_and_installs_combat_free_state() {
        let source_identity = CardIdentity {
            id: CardId::Metamorphosis,
            upgrade: 0,
            enchantment: None,
        };
        let pool = metamorphosis_owner_attack_pool(RewardPool::Defect, None);
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source_identity).unwrap();
        for &id in pool.iter() {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Defect);
        state.entropy_card_pool = Some(RewardPool::Defect);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 17,
            atom: source_atom,
            flags: 0,
        });
        state.next_card_uid = 18;
        let before_rng = state.rng.get(RngStream::Generation).counter;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };

        let draw_seed = [5_u64, 6, 7, 8];
        ctx.state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: draw_seed,
                counter: 0,
            },
        );
        let existing_draw: Vec<HotCard> = (100..104)
            .map(|uid| HotCard {
                uid,
                atom: source_atom,
                flags: 0,
            })
            .collect();
        ctx.state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend(existing_draw.iter().copied());
        // #3243: native AddGeneratedCardToCombat(card, Draw, owner, Random)
        // draws one Shuffle index over `Draw.len() + 1` per member, in order.
        let mut expected_uids: Vec<u32> = existing_draw.iter().map(|card| card.uid).collect();
        let mut oracle = Xoshiro256StarStar {
            words: draw_seed,
            counter: 0,
        };
        for uid in 18..21 {
            let bound = i32::try_from(expected_uids.len() + 1).unwrap();
            let index = usize::try_from(oracle.next_bounded(bound).unwrap()).unwrap();
            expected_uids.insert(index, uid);
        }

        metamorphosis_exact(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Generation).counter, before_rng + 3);
        assert_eq!(
            state.rng.get(RngStream::Rng).counter,
            3,
            "one Shuffle-stream draw per random Draw insertion"
        );
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.piles.get(PileId::Hand).len(), 0);
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            expected_uids,
            "each member lands at its own random Draw index"
        );
        assert_ne!(
            expected_uids,
            [100, 101, 102, 103, 18, 19, 20],
            "the witness seed must not coincide with a bottom append"
        );
        for card in state.piles.get(PileId::Draw).as_slice() {
            if card.uid >= 100 {
                continue;
            }
            let generated_spec = catalog.spec(card.atom).unwrap();
            assert!(generated_spec.is_attack);
            assert_eq!(generated_spec.identity.upgrade, 0);
            let instance = state.card_states.get_ref(card.uid).unwrap();
            assert!(instance.local_cost_modifiers.free_star_cost_this_combat());
            let rows = instance.local_cost_modifiers.as_slice();
            assert_eq!(rows.len(), usize::from(generated_spec.cost >= 0));
            assert!(rows.iter().all(|row| {
                row.kind == LocalCostModifierKind::Set
                    && row.amount == 0
                    && row.expiration == LocalCostExpiration::ThisCombat
                    && !row.reduce_only
            }));
        }

        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((21..31).map(|uid| HotCard {
                uid,
                atom: source_atom,
                flags: 0,
            }));
        state.next_card_uid = 31;
        let draw_before = state.piles.get(PileId::Draw).len();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };
        metamorphosis_exact(&mut ctx).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(
            state.piles.get(PileId::Discard).len(),
            0,
            "a full Hand no longer redirects: the destination is Draw"
        );
        assert_eq!(state.piles.get(PileId::Draw).len(), draw_before + 3);
        assert_eq!(state.rng.get(RngStream::Rng).counter, 6);
    }

    #[test]
    fn metamorphosis_generated_adaptive_strike_clone_preserves_combat_star_marker() {
        let adaptive = CardIdentity {
            id: CardId::AdaptiveStrike,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(adaptive).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 7;
        state.energy = 0;
        state.stars = 0;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        inject_generated_free_this_combat_batch_draw_random(
            &mut state,
            &catalog,
            &[adaptive],
            &mut Vec::new(),
        )
        .unwrap();
        // The generated Attack lands in Draw (#3243); move it to Hand so the
        // play below exercises the clone's copied combat-long Star marker.
        let generated = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
        assert_eq!(generated.uid, 7);
        state.piles.get_mut(PileId::Hand).make_mut().push(generated);

        play_card(
            &mut state,
            &catalog,
            generated.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        let active = state.piles.get(PileId::Play).as_slice();
        let discard = state.piles.get(PileId::Discard).as_slice();
        assert_eq!(active.iter().map(|card| card.uid).collect::<Vec<_>>(), [7]);
        assert_eq!(discard.iter().map(|card| card.uid).collect::<Vec<_>>(), [8]);
        let source_state = state.card_states.get_ref(7).unwrap();
        let clone_state = state.card_states.get_ref(8).unwrap();
        assert!(
            source_state
                .local_cost_modifiers
                .free_star_cost_this_combat()
        );
        assert!(
            clone_state
                .local_cost_modifiers
                .free_star_cost_this_combat()
        );
        assert_eq!(source_state.local_cost_modifiers.as_slice().len(), 1);
        assert_eq!(clone_state.local_cost_modifiers.as_slice().len(), 2);
        assert!(
            clone_state
                .local_cost_modifiers
                .as_slice()
                .iter()
                .all(|row| row.kind == LocalCostModifierKind::Set
                    && row.amount == 0
                    && row.expiration == LocalCostExpiration::ThisCombat
                    && !row.reduce_only)
        );
    }

    #[test]
    fn metamorphosis_public_play_rolls_back_payment_and_routing_on_late_closure_refusal() {
        let metamorphosis = CardIdentity {
            id: CardId::Metamorphosis,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(metamorphosis).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 2;
        state.reward_card_pool = Some(RewardPool::Defect);
        state.entropy_card_pool = Some(RewardPool::Defect);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.next_card_uid = 8;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 1 }];
        let events_before = events.clone();

        assert!(matches!(
            play_card(&mut state, &catalog, 7, None, None, &mut events),
            Err(EngineRefusal::UnknownMintIdentity(_))
        ));
        assert_eq!(
            state, before,
            "outer CardPlay transaction restores every prefix"
        );
        assert_eq!(events, events_before);
    }

    #[test]
    fn metamorphosis_public_burst_resamples_and_routes_the_source_once() {
        let metamorphosis = CardIdentity {
            id: CardId::Metamorphosis,
            upgrade: 0,
            enchantment: None,
        };
        let pool = metamorphosis_owner_attack_pool(RewardPool::Defect, None);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(metamorphosis).unwrap();
        for &id in pool.iter() {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 2;
        state.reward_card_pool = Some(RewardPool::Defect);
        state.entropy_card_pool = Some(RewardPool::Defect);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.next_card_uid = 18;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 17,
            atom,
            flags: 0,
        });
        let before_rng = state.rng.get(RngStream::Generation).counter;

        let played = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 17,
                target: None,
                selection: SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(played.energy, 0, "the source pays once");
        assert_eq!(played.powers.value(PowerId::Burst), 0);
        assert_eq!(
            played.rng.get(RngStream::Generation).counter,
            before_rng + 6
        );
        assert_eq!(played.history.owner_generated_cards_combat, 6);
        assert_eq!(played.piles.get(PileId::Draw).len(), 6);
        assert_eq!(played.rng.get(RngStream::Rng).counter, 6);
        assert_eq!(
            played
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .filter(|card| card.uid == 17)
                .count(),
            1,
            "Burst replays the body but exhausts/routes the source once"
        );

        let mut autoplay = state;
        autoplay.powers.set(PowerId::Burst, SlotWire::Int, 0);
        let autoplay_source = autoplay.piles.get(PileId::Hand).as_slice()[0];
        let autoplay_rng = autoplay.rng.get(RngStream::Generation).counter;
        autoplay_collected_cards(&mut autoplay, &catalog, &[autoplay_source], &mut Vec::new())
            .unwrap();
        assert_eq!(
            autoplay.rng.get(RngStream::Generation).counter,
            autoplay_rng + 3,
            "direct AutoPlay samples one fresh complete body"
        );
        assert_eq!(autoplay.history.owner_generated_cards_combat, 3);
        assert_eq!(autoplay.piles.get(PileId::Draw).len(), 3);
        assert_eq!(
            autoplay
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .filter(|card| card.uid == 17)
                .count(),
            1,
            "direct AutoPlay retains its source through the body then routes once"
        );
    }

    #[test]
    fn metamorphosis_samples_full_rng_before_terminal_listener_suppresses_suffix() {
        let metamorphosis = CardIdentity {
            id: CardId::Metamorphosis,
            upgrade: 0,
            enchantment: None,
        };
        let pool = metamorphosis_owner_attack_pool(RewardPool::Defect, None);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(metamorphosis).unwrap();
        for &id in pool.iter() {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.reward_card_pool = Some(RewardPool::Defect);
        state.entropy_card_pool = Some(RewardPool::Defect);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.next_card_uid = 18;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 17,
            atom,
            flags: 0,
        });
        let before_rng = state.rng.get(RngStream::Generation).counter;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };

        metamorphosis_exact(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(state.rng.get(RngStream::Generation).counter, before_rng + 3);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
        assert_eq!(
            state.rng.get(RngStream::Rng).counter,
            1,
            "suppressed members take no Shuffle draw"
        );
        assert_eq!(state.next_card_uid, 19);
    }

    #[test]
    fn metamorphosis_owner_level_vectors_and_with_replacement_duplicate_are_pinned() {
        for (owner, expected_three, expected_five) in [
            (
                RewardPool::Ironclad,
                &[CardId::Bludgeon, CardId::Thrash, CardId::Anger][..],
                &[
                    CardId::Bludgeon,
                    CardId::Thrash,
                    CardId::Anger,
                    CardId::Whirlwind,
                    CardId::TearAsunder,
                ][..],
            ),
            (
                RewardPool::Silent,
                &[CardId::Backstab, CardId::Skewer, CardId::Assassinate][..],
                &[
                    CardId::Backstab,
                    CardId::Skewer,
                    CardId::Assassinate,
                    CardId::SuckerPunch,
                    CardId::Ricochet,
                ][..],
            ),
            (
                RewardPool::Defect,
                &[
                    CardId::BallLightning,
                    CardId::Sunder,
                    CardId::AdaptiveStrike,
                ][..],
                &[
                    CardId::BallLightning,
                    CardId::Sunder,
                    CardId::AdaptiveStrike,
                    CardId::Uproar,
                    CardId::Shatter,
                ][..],
            ),
        ] {
            let pool = metamorphosis_owner_attack_pool(owner, None);
            for expected in [expected_three, expected_five] {
                let mut state = HotState::at_defaults();
                let seeded = Xoshiro256StarStar::from_seed(1959);
                state.rng.set(
                    RngStream::Generation,
                    RngStreamState {
                        words: seeded.words,
                        counter: seeded.counter,
                    },
                );
                let selected = (0..expected.len())
                    .map(|_| sample_generation_slice(&mut state, &pool).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(selected, expected);
            }
        }

        let pool = metamorphosis_owner_attack_pool(RewardPool::Defect, None);
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        assert_eq!(
            (0..5)
                .map(|_| sample_generation_slice(&mut state, &pool).unwrap())
                .collect::<Vec<_>>(),
            [
                CardId::MeteorStrike,
                CardId::RocketPunch,
                CardId::BallLightning,
                CardId::GoForTheEyes,
                CardId::RocketPunch,
            ],
            "independent draws preserve native with-replacement duplicates"
        );
    }

    #[test]
    fn neutral_active_source_accepts_direct_autoplay_piles_and_refuses_mismatch() {
        let source_identity = CardIdentity {
            id: CardId::Jackpot,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source_identity).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 17,
                atom: source_atom,
                flags: 0,
            });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut events = Vec::new();
        let ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 17,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(25), CompiledArg::I(3)],
            events: &mut events,
        };

        assert_eq!(
            exact_active_neutral_source(&ctx, CardId::Jackpot, "jackpot source"),
            Ok(())
        );
        assert_eq!(
            exact_active_neutral_source(&ctx, CardId::JackOfAllTrades, "jack source"),
            Err(EngineRefusal::MalformedArgs("jack source"))
        );
        assert!(events.is_empty());
    }

    fn jack_fixture(upgrade: u8, seed: u64) -> (HotState, Catalog, CardIdentity, RngStreamState) {
        let jack = identity(CardId::JackOfAllTrades, upgrade);
        let mut builder = CatalogBuilder::new();
        let jack_atom = builder.intern(jack).unwrap();
        for id in crate::content_tables::JACK_OF_ALL_TRADES_POOL_V1091 {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        for id in crate::content_tables::JACKPOT_ZERO_COST_POOL_V109 {
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
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(RngStream::Generation, entering);
        state.next_card_uid = 18;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 17,
            atom: jack_atom,
            flags: 0,
        });
        (state, catalog, jack, entering)
    }

    #[test]
    fn jack_shuffles_the_whole_pool_then_awaits_one_or_two_l0_adds() {
        for (upgrade, expected_count) in [(0, 1usize), (1, 2usize)] {
            let (mut state, catalog, jack, entering) = jack_fixture(upgrade, 47);
            let mut oracle = Xoshiro256StarStar {
                words: entering.words,
                counter: entering.counter,
            };
            let mut pool = crate::content_tables::JACK_OF_ALL_TRADES_POOL_V1091;
            oracle.shuffle(&mut pool).unwrap();
            let expected = pool[..expected_count].to_vec();
            let mut events = Vec::new();
            let spec = *catalog.spec(catalog.atom(&jack).unwrap()).unwrap();
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

            jack_of_all_trades_body(&mut ctx).unwrap();

            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                entering.counter + 48
            );
            assert_eq!(
                state.history.owner_generated_cards_combat,
                expected_count as i32
            );
            assert_eq!(state.next_generated_hook_uid, expected_count as i32);
            assert_eq!(state.next_card_uid, 18 + expected_count as u32);
            assert!(state.exact_piles);
            let actual = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity)
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                expected
                    .into_iter()
                    .map(|id| CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .collect::<Vec<_>>(),
                "Jack+ raises Cards, never the returned card level"
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, crate::engine::Event::CardResolved { .. }))
                    .count(),
                expected_count
            );
        }
    }

    #[test]
    fn jack_singular_adds_reread_terminal_state_and_suppress_the_suffix() {
        let (mut state, catalog, jack, entering) = jack_fixture(1, 47);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        let mut events = Vec::new();
        let spec = *catalog.spec(catalog.atom(&jack).unwrap()).unwrap();
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

        jack_of_all_trades_body(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(
            state.rng.get(RngStream::Generation).counter,
            entering.counter + 48
        );
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.next_generated_hook_uid, 1);
        assert_eq!(state.next_card_uid, 19);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::engine::Event::CardResolved { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn jack_refuses_a_multiplayer_root_and_an_unrecorded_profile_by_name() {
        // #2560: the body shuffles the SOLO projection of the recorded
        // profile's Colorless pool; `GetUnlockedCards` (`0x7e54c`
        // IL_001f-IL_004e) keeps the MultiplayerOnly rows for a party, and a
        // document that records no profile has no pool to derive.
        for (label, mutate) in [
            (
                "party",
                (|state: &mut HotState| state.multiplayer_ally_key = 1) as fn(&mut HotState),
            ),
            ("unrecorded profile", |state: &mut HotState| {
                state.fully_unlocked_card_pool_epochs = false;
            }),
            ("no owner", |state: &mut HotState| {
                state.reward_card_pool = None
            }),
        ] {
            let (mut state, catalog, jack, _) = jack_fixture(0, 47);
            mutate(&mut state);
            let before = state.clone();
            let spec = *catalog.spec(catalog.atom(&jack).unwrap()).unwrap();
            let mut events = Vec::new();
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
                jack_of_all_trades_body(&mut ctx),
                Err(EngineRefusal::MalformedArgs(
                    "jack of all trades generation provenance"
                )),
                "{label}"
            );
            assert_eq!(state, before, "{label}");
        }
    }

    #[test]
    fn jack_preflights_the_whole_body_before_rng_or_exact_mode_publication() {
        let (mut state, catalog, jack, _) = jack_fixture(1, 47);
        state.history.owner_generated_cards_combat = i32::MAX - 1;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let events_before = events.clone();
        let spec = *catalog.spec(catalog.atom(&jack).unwrap()).unwrap();
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
            jack_of_all_trades_body(&mut ctx),
            Err(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn jack_refuses_forged_action_shape_and_source_identity_atomically() {
        let (mut state, catalog, jack, _) = jack_fixture(0, 47);
        let before = state.clone();
        let spec = *catalog.spec(catalog.atom(&jack).unwrap()).unwrap();
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
            jack_of_all_trades_body(&mut ctx),
            Err(EngineRefusal::MalformedArgs("jack_of_all_trades_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 17,
                atom: catalog.atom(&jack).unwrap(),
                flags: 0,
            });
        let duplicated = state.clone();
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
            jack_of_all_trades_body(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: 17,
                matches: 2
            })
        );
        assert_eq!(state, duplicated);
        assert!(events.is_empty());
    }

    #[test]
    fn jack_public_manual_sly_direct_autoplay_and_burst_share_body_and_routing() {
        for (mode, burst, expected_cards, expected_draws) in [
            (0u8, 0, 1usize, 48u64),
            (1, 0, 1, 48),
            (2, 0, 1, 48),
            (0, 1, 2, 96),
        ] {
            let (mut state, catalog, jack, entering) = jack_fixture(0, 47);
            let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
            let initial_pile = if mode == 0 {
                PileId::Hand
            } else {
                PileId::Draw
            };
            state.piles.get_mut(initial_pile).make_mut().push(source);
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
                1,
                "the exact source routes only after every synchronous body"
            );
            assert!(state.piles.get(PileId::Play).is_empty());
            assert!(
                state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .all(|card| catalog.spec(card.atom).unwrap().identity.upgrade == 0)
            );
            assert_eq!(catalog.spec(source.atom).unwrap().identity, jack);
        }
    }

    #[test]
    fn restlessness_awaits_each_draw_then_gains_energy_only_if_combat_survives() {
        let restlessness = identity(CardId::Restlessness, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(restlessness).unwrap();
        let defend_atom = builder.intern(DEFEND).unwrap();
        let catalog = builder.build();
        let card = |uid| HotCard {
            uid,
            atom: defend_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([card(2), card(3)]);
        let mut events = Vec::new();
        run_card_step(
            StepKind::RestlessnessExact,
            restlessness,
            &mut state,
            &catalog,
            &[CompiledArg::I(2)],
            &mut events,
        )
        .unwrap();
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(state.energy, 5);

        let mut nonempty = HotState::at_defaults();
        nonempty.hp = 50;
        nonempty.energy = 3;
        nonempty
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(4));
        nonempty
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(5));
        let before = nonempty.clone();
        run_card_step(
            StepKind::RestlessnessExact,
            restlessness,
            &mut nonempty,
            &catalog,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(nonempty, before, "a nonempty post-removal Hand is a no-op");

        let mut lethal = HotState::at_defaults();
        lethal.hp = 50;
        lethal.energy = 3;
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        lethal
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([card(6), card(7)]);
        lethal.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            lethal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        lethal.fanouts.set_cacophony_left(1);
        run_card_step(
            StepKind::RestlessnessExact,
            restlessness,
            &mut lethal,
            &catalog,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.energy, 3, "the ending gate suppresses Energy");
        assert_eq!(lethal.piles.get(PileId::Hand).as_slice()[0].uid, 6);
        assert_eq!(lethal.piles.get(PileId::Draw).as_slice()[0].uid, 7);
    }

    #[test]
    fn the_ball_exact_rows_are_published_and_dispatch_publicly() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter_map(|row| {
                row.steps.iter().find_map(|step| {
                    (step.kind == StepKind::TheBallExact).then_some((
                        row.id,
                        row.upgrade,
                        row.cost,
                        row.pool,
                        row.target_type,
                        row.steps,
                    ))
                })
            })
            .collect();
        assert_eq!(carriers.len(), 2);
        assert_eq!(
            carriers
                .iter()
                .map(|(id, upgrade, cost, pool, target, steps)| {
                    (*id, *upgrade, *cost, *pool, *target, steps[0].args)
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    CardId::TheBall,
                    0,
                    1,
                    Some("Colorless"),
                    "AnyEnemy",
                    &[Arg::I(10), Arg::I(10)][..],
                ),
                (
                    CardId::TheBall,
                    1,
                    1,
                    Some("Colorless"),
                    "AnyEnemy",
                    &[Arg::I(10), Arg::I(15)][..],
                ),
            ]
        );
        assert!(IMPLEMENTED.contains(&StepKind::TheBallExact));

        let (mut state, catalog, spec, source) = the_ball_fixture(0, 0, 0, 100);
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10), CompiledArg::I(10)],
            events: &mut events,
        };
        assert_eq!(
            the_ball_exact(&mut ctx),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn the_ball_private_body_reads_live_growth_and_grows_by_live_level() {
        for (upgrade, prior, expected_damage, expected_growth) in [(0, 7, 17, 17), (1, 11, 21, 26)]
        {
            let (mut state, catalog, spec, source) = the_ball_fixture(upgrade, upgrade, prior, 200);
            let before_rng = state.rng.clone();
            let args = [
                CompiledArg::I(10),
                CompiledArg::I(if upgrade == 0 { 10 } else { 15 }),
            ];
            let mut events = Vec::new();
            run_the_ball_foundation(
                &mut state,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                &args,
                &mut events,
            )
            .unwrap();

            assert_eq!(state.monsters[0].hp, 200 - expected_damage);
            assert_eq!(
                state.card_states.get(source.uid).damage_growth,
                expected_growth
            );
            assert_eq!(state.rng, before_rng, "OnPlay consumes no RNG stream");
            assert!(matches!(
                events.as_slice(),
                [crate::engine::Event::MonsterDamaged { .. }]
            ));
        }
    }

    #[test]
    fn the_ball_replay_rereads_the_live_level_and_prior_growth() {
        let (mut state, catalog, spec, source) = the_ball_fixture(0, 0, 0, 200);
        let args = [CompiledArg::I(10), CompiledArg::I(10)];
        let mut events = Vec::new();
        run_the_ball_foundation(
            &mut state,
            &catalog,
            &spec,
            source.uid,
            Some(0),
            &args,
            &mut events,
        )
        .unwrap();
        let upgraded_atom = catalog.atom(&identity(CardId::TheBall, 1)).unwrap();
        state.piles.get_mut(PileId::Play).make_mut()[0].atom = upgraded_atom;
        run_the_ball_foundation(
            &mut state,
            &catalog,
            &spec,
            source.uid,
            Some(0),
            &args,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 170, "10 then live 20 damage");
        assert_eq!(
            state.card_states.get(source.uid).damage_growth,
            25,
            "the second live level contributes fifteen"
        );
    }

    #[test]
    fn the_ball_final_kill_and_player_death_both_keep_ungated_growth() {
        let (mut lethal, catalog, spec, source) = the_ball_fixture(0, 0, 0, 10);
        run_the_ball_foundation(
            &mut lethal,
            &catalog,
            &spec,
            source.uid,
            Some(0),
            &[CompiledArg::I(10), CompiledArg::I(10)],
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.card_states.get(source.uid).damage_growth, 10);

        let (mut retaliated, catalog, spec, source) = the_ball_fixture(0, 0, 0, 100);
        retaliated.hp = 1;
        retaliated.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);
        run_the_ball_foundation(
            &mut retaliated,
            &catalog,
            &spec,
            source.uid,
            Some(0),
            &[CompiledArg::I(10), CompiledArg::I(10)],
            &mut Vec::new(),
        )
        .unwrap();
        assert!(retaliated.hp <= 0);
        assert_eq!(retaliated.card_states.get(source.uid).damage_growth, 10);
    }

    #[test]
    fn the_ball_late_growth_overflow_rolls_back_attack_and_seeded_events() {
        let (mut state, catalog, spec, source) = the_ball_fixture(1, 1, i32::MAX - 14, 200);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_the_ball_foundation(
                &mut state,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                &[CompiledArg::I(10), CompiledArg::I(15)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "the_ball_exact damage growth"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_ball_maximum_growth_refuses_late_growth_after_lethal_retaliation() {
        let (mut state, catalog, spec, source) = the_ball_fixture(0, 0, i32::MAX, 100);
        state.hp = 1;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);

        let before = state.clone();
        assert_eq!(
            run_the_ball_foundation(
                &mut state,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                &[CompiledArg::I(10), CompiledArg::I(10)],
                &mut Vec::new(),
            ),
            Err(EngineRefusal::CounterOverflow(
                "the_ball_exact damage growth"
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn the_ball_target_frame_identity_forgeries_are_atomic() {
        let (state, catalog, spec, source) = the_ball_fixture(0, 0, 3, 100);
        let current = (state.monsters[0].slot, state.monsters[0].uid);
        for (label, frame_target) in [
            ("wrong slot", (current.0 + 1, current.1)),
            ("wrong uid", (current.0, current.1 + 1)),
        ] {
            let mut candidate = state.clone();
            let before = candidate.clone();
            let mut events = vec![crate::engine::Event::CardResolved {
                uid: 99,
                pile: PileId::Discard,
            }];
            let before_events = events.clone();
            assert_eq!(
                run_the_ball_foundation_with_target_identity(
                    &mut candidate,
                    &catalog,
                    &spec,
                    source.uid,
                    Some(0),
                    frame_target,
                    &[CompiledArg::I(10), CompiledArg::I(10)],
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "the_ball_exact target identity"
                )),
                "{label}"
            );
            assert_eq!(candidate, before, "{label} mutated state");
            assert_eq!(events, before_events, "{label} mutated events");
        }

        let mut replaced = state.clone();
        replaced.monsters_mut()[0].uid += 1;
        let before = replaced.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_the_ball_foundation_with_target_identity(
                &mut replaced,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                current,
                &[CompiledArg::I(10), CompiledArg::I(10)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "the_ball_exact target identity"
            ))
        );
        assert_eq!(replaced, before, "a same-slot replacement mutated state");
        assert_eq!(events, before_events);

        let mut noncurrent = state.clone();
        let before = noncurrent.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut noncurrent,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10), CompiledArg::I(10)],
            events: &mut events,
        };
        let result = crate::engine::play::with_test_active_play_target(source.uid, current, || {
            crate::engine::play::with_test_active_play(404, || the_ball_body_foundation(&mut ctx))
        });
        assert_eq!(result, Err(EngineRefusal::ContinuationNotModeled));
        assert_eq!(noncurrent, before, "a suspended parent mutated state");
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_ball_stock_replacement_is_safe_and_late_refusal_restores_it() {
        let (mut stock, catalog, spec, source) = the_ball_fixture(0, 0, 0, 1);
        stock.monsters_mut()[0].kind = MonsterKind::Axebot;
        stock.monsters_mut()[0].uid = 0;
        stock.monsters_mut()[0].max_hp = 76;
        stock.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, 2);
        let before_rng = stock.rng.clone();
        run_the_ball_foundation(
            &mut stock,
            &catalog,
            &spec,
            source.uid,
            Some(0),
            &[CompiledArg::I(10), CompiledArg::I(10)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(stock.monsters[0].uid, 1, "Stock replaced the target");
        assert_eq!(stock.monsters[0].powers.value(PowerId::Stock), 1);
        assert_eq!(stock.card_states.get(source.uid).damage_growth, 10);
        assert_eq!(
            stock.rng.get(crate::hot::RngStream::Niche).counter,
            before_rng.get(crate::hot::RngStream::Niche).counter + 1
        );
        for stream in crate::hot::RngStream::ALL {
            if stream != crate::hot::RngStream::Niche {
                assert_eq!(stock.rng.get(stream), before_rng.get(stream));
            }
        }

        let (mut overflow, catalog, spec, source) = the_ball_fixture(1, 1, i32::MAX - 14, 1);
        overflow.monsters_mut()[0].kind = MonsterKind::Axebot;
        overflow.monsters_mut()[0].uid = 0;
        overflow.monsters_mut()[0].max_hp = 76;
        overflow.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, 2);
        let before = overflow.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_the_ball_foundation(
                &mut overflow,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                &[CompiledArg::I(10), CompiledArg::I(15)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "the_ball_exact damage growth"
            ))
        );
        assert_eq!(overflow, before, "Stock and its Niche draw rolled back");
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_ball_requires_both_live_level_catalog_peers_atomically() {
        let (state, _, _, source) = the_ball_fixture(0, 0, 3, 100);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::TheBall, 0)).unwrap();
        assert_eq!(source.atom, atom);
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut candidate = state;
        let before = candidate.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_the_ball_foundation(
                &mut candidate,
                &catalog,
                &spec,
                source.uid,
                Some(0),
                &[CompiledArg::I(10), CompiledArg::I(10)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "the_ball_exact catalog closure"
            ))
        );
        assert_eq!(candidate, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_ball_private_entry_forgeries_are_atomic() {
        let (state, catalog, spec, source) = the_ball_fixture(0, 0, 3, 100);
        for mutation in 0..8 {
            let mut candidate = state.clone();
            match mutation {
                0 => candidate.multiplayer_ally_key = 0,
                1 => candidate.fanouts.multiplayer_ally_mut().alive = false,
                2 => candidate.piles.get_mut(PileId::Play).make_mut()[0].flags = 0,
                3 => {
                    candidate.piles.get_mut(PileId::Play).make_mut()[0].flags |=
                        crate::hot::CARD_FLAG_LOCAL_EXHAUST;
                }
                4 => {
                    let mut instance = candidate.card_states.get(source.uid);
                    instance.local_retain = true;
                    candidate.card_states.set(source.uid, instance);
                }
                5 => candidate
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(source),
                6 => {
                    let mut instance = candidate.card_states.get(source.uid);
                    instance.damage_growth = -1;
                    candidate.card_states.set(source.uid, instance);
                }
                7 => candidate.fanouts.multiplayer_ally_mut().key = 7,
                _ => unreachable!(),
            }
            let before = candidate.clone();
            let mut events = vec![crate::engine::Event::CardResolved {
                uid: 99,
                pile: PileId::Discard,
            }];
            let before_events = events.clone();
            assert!(
                run_the_ball_foundation(
                    &mut candidate,
                    &catalog,
                    &spec,
                    source.uid,
                    Some(0),
                    &[CompiledArg::I(10), CompiledArg::I(10)],
                    &mut events,
                )
                .is_err(),
                "forgery {mutation} must refuse"
            );
            assert_eq!(candidate, before, "forgery {mutation} mutated state");
            assert_eq!(events, before_events, "forgery {mutation} mutated events");
        }

        let mut no_frame = state.clone();
        let before = no_frame.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut no_frame,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(10), CompiledArg::I(10)],
            events: &mut events,
        };
        assert_eq!(
            the_ball_body_foundation(&mut ctx),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(no_frame, before);
        assert!(events.is_empty());

        let mut forged_spec = spec;
        forged_spec.solo_unplayable = false;
        let mut candidate = state.clone();
        let before = candidate.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_the_ball_foundation(
                &mut candidate,
                &catalog,
                &forged_spec,
                source.uid,
                Some(0),
                &[CompiledArg::I(10), CompiledArg::I(10)],
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "the_ball_exact canonical spec"
            ))
        );
        assert_eq!(candidate, before);
        assert_eq!(events, before_events);
    }
}
