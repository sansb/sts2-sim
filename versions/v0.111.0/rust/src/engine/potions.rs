//! Current-build synchronous potion bodies.
//!
//! Authority is macOS arm64 `sts2.dll` v0.111.0/41cef1ea, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//! Each body below cites its current-build `OnUse`/`MoveNext` RVA.

use crate::catalog::{CardIdentity, Catalog};
use crate::decimal::DotNetDecimal;
use crate::frame::{Frame, PotionFinishStage};
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, DrawCaller, HotState, MiseryToken, PileId, PotionBodyStage,
    PotionFinishRecord, RngStream, RngStreamState,
};
use crate::ids::{CardId, PotionId, PowerId, RelicId};
use crate::powers::SlotWire;
use crate::rng::Xoshiro256StarStar;

use super::{EngineRefusal, Event, Subject};

/// Stable D2f refusal for a nonlocal Magi-death atom restoration.
pub(crate) const DISTILLED_DAMPEN_FRONTIER: &str = "R52D2f Distilled active Dampen restore";

/// The ordered candidate pool one generation potion shuffles, under the
/// recorded unlock profile `epochs` (`None` = the fully-unlocked projection).
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`, #3141). Each
/// body's `<OnUse>d__6::MoveNext` asserts its target, reads
/// `target.Player` (IL_0033-IL_0038) and hands one `GetUnlockedCards` result
/// to `CardFactory::GetDistinctForCombat(.., 3, Rng.CombatCardGeneration)`
/// (RVA `0x112878`: `FilterForPlayerCount` IL_0030, `FilterForCombat`
/// IL_0038, one complete-list shuffle):
///
/// * `AttackPotion` `0x34bdb4`, `SkillPotion` `0x3502bc`, `PowerPotion`
///   `0x34fb58`: `player.Character.CardPool` (IL_0040-IL_0045) through
///   `CardPoolModel::GetUnlockedCards(player.UnlockState,
///   RunState.CardMultiplayerConstraint)` (IL_005b; `0x7e54c` runs the
///   character pool's `FilterThroughEpochs` at IL_0014), then
///   `Where(<>c::<OnUse>b__6_0)` (IL_007f) whose predicate is
///   `get_Type; ldc.i4.N; ceq` with N = 1 Attack (`0x34bda6`), 2 Skill
///   (`0x3502ae`), 3 Power (`0x34fb4a`), and `GetDistinctForCombat` at
///   IL_0095 with count 3 (IL_0084);
/// * `ColorlessPotion` `0x34c784`: `ModelDb::CardPool<ColorlessCardPool>`
///   (MethodSpec `0x2b0007c2`, IL_003f) through the same `GetUnlockedCards`
///   (IL_0055) and STRAIGHT into `GetDistinctForCombat(.., 3, ..)` (IL_006b)
///   with no `Where` and no read of the player's character;
/// * `OrobicAcid` `0x34f050` makes the three character-pool queries in
///   Attack, Skill, Power order (IL_005b/IL_00bd/IL_011f, predicates
///   `0x34f02e`/`0x34f039`/`0x34f044`), each `GetDistinctForCombat(.., 1, ..)`
///   (IL_0095/IL_00f7/IL_0159).
///
/// `FilterForCombat` (`0x112932`, predicate `0x3d7042` IL_0001-IL_0029) keeps
/// `CanBeGeneratedInCombat` rows whose rarity is not Basic (1), Ancient (5) or
/// Event (6). So the character pools are
/// [`crate::steps::neutral::owner_type_generation_pool`] (the Distraction /
/// White Noise derivation) and the Colorless pool is
/// [`crate::steps::neutral::colorless_generation_pool`] (Quasar's), both
/// filtered by the recorded profile. At `None` they are exactly the frozen
/// projections this function returned before #3141
/// (`generation_potion_pools_at_the_full_profile_are_the_frozen_projections`).
/// Consumes no RNG.
pub(crate) fn generation_choice_pool(
    potion: PotionId,
    owner: Option<crate::catalog::RewardPool>,
    epochs: Option<&[&'static str]>,
) -> Option<Vec<CardId>> {
    use crate::content_tables::CardType;
    let kind = match potion {
        PotionId::AttackPotion => CardType::Attack,
        PotionId::SkillPotion => CardType::Skill,
        PotionId::PowerPotion => CardType::Power,
        PotionId::ColorlessPotion => {
            return Some(crate::steps::neutral::colorless_generation_pool(epochs));
        }
        _ => return None,
    };
    Some(crate::steps::neutral::owner_type_generation_pool(
        owner?, epochs, kind,
    ))
}

/// [`generation_choice_pool`] for this fight: the reward owner under the
/// catalog's recorded profile. Only meaningful once
/// [`generation_choice_provenance_is_exact`] holds, which requires the
/// profile to be recorded (`catalog.splash_unlock_epochs()` is `None`
/// exactly when it is fully unlocked).
pub(crate) fn fight_generation_choice_pool(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Option<Vec<CardId>> {
    generation_choice_pool(
        potion,
        state.reward_card_pool,
        catalog.splash_unlock_epochs(),
    )
}

fn is_generation_choice(potion: PotionId) -> bool {
    matches!(
        potion,
        PotionId::AttackPotion
            | PotionId::SkillPotion
            | PotionId::PowerPotion
            | PotionId::ColorlessPotion
    )
}

fn orobic_generation_pools(state: &HotState, catalog: &Catalog) -> Option<[Vec<CardId>; 3]> {
    Some([
        fight_generation_choice_pool(state, catalog, PotionId::AttackPotion)?,
        fight_generation_choice_pool(state, catalog, PotionId::SkillPotion)?,
        fight_generation_choice_pool(state, catalog, PotionId::PowerPotion)?,
    ])
}

fn orobic_rng_draws(state: &HotState, catalog: &Catalog) -> Result<usize, EngineRefusal> {
    let pools = orobic_generation_pools(state, catalog)
        .ok_or(EngineRefusal::MalformedArgs("Orobic generation owner"))?;
    if pools.iter().any(Vec::is_empty) {
        return Err(EngineRefusal::MalformedArgs("Orobic empty generation pool"));
    }
    Ok(pools.iter().map(|pool| pool.len().saturating_sub(1)).sum())
}

pub(crate) fn preflight_manual_potion_rng(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Result<(), EngineRefusal> {
    let (stream, draws) = if let Some(pool) = fight_generation_choice_pool(state, catalog, potion) {
        (RngStream::Generation, pool.len().saturating_sub(1))
    } else if potion == PotionId::OrobicAcid {
        (RngStream::Generation, orobic_rng_draws(state, catalog)?)
    } else if potion == PotionId::EntropicBrew {
        let attempts = if state.fanouts.potion_sozu() {
            1
        } else {
            state
                .fanouts
                .potion_slots()
                .iter()
                .filter(|slot| slot.is_none())
                .count()
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Entropic attempts"))?
        };
        (
            RngStream::PotionGeneration,
            attempts
                .checked_mul(2)
                .ok_or(EngineRefusal::CounterOverflow("Entropic RNG"))?,
        )
    } else {
        return Ok(());
    };
    let draws =
        u64::try_from(draws).map_err(|_| EngineRefusal::CounterOverflow("potion RNG draws"))?;
    state
        .rng
        .get(stream)
        .counter
        .checked_add(draws)
        .ok_or(EngineRefusal::CounterOverflow("potion RNG counter"))?;
    Ok(())
}

/// Reserve every `CombatTargets` draw that a Distilled Chaos batch can reach
/// before its first gathered child is allowed to publish a selection.
///
/// The full-shuffle gather can choose any physical card currently in Draw or
/// Discard, and earlier children may end combat before later ones run.  The
/// sound preflight is therefore the maximum number of target-selecting cards
/// in one three-card batch, capped at three.  Native resolves one automatic
/// target before `GeneratePlayCount`, so replayed bodies reuse that target and
/// do not consume additional draws.
pub(crate) fn preflight_distilled_target_rng(
    state: &HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    let mut target_draws = 0_u64;
    for card in [PileId::Draw, PileId::Discard]
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
    {
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        let target_type = super::play::effective_target_type(state, catalog, spec, card.uid)?;
        if !super::play::target_type_requires_choice(spec, target_type) {
            continue;
        }
        let target_exists = match target_type {
            crate::catalog::CardTargetType::AnyEnemy => {
                state.monsters.iter().any(|monster| monster.hp > 0)
            }
            crate::catalog::CardTargetType::AnyAlly => {
                state.hp > 0
                    || state.multiplayer_ally_key == 1 && state.fanouts.multiplayer_ally().alive
            }
            _ => false,
        };
        if !target_exists {
            return Err(EngineRefusal::MalformedArgs(
                "Distilled Chaos automatic target domain",
            ));
        }
        target_draws = target_draws
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("Distilled target draws"))?
            .min(3);
    }
    if target_draws == 0 {
        return Ok(());
    }
    if state.rng.is_vacant(RngStream::Targets) {
        return Err(EngineRefusal::MalformedArgs(
            "Distilled Chaos CombatTargets provenance",
        ));
    }
    state
        .rng
        .get(RngStream::Targets)
        .counter
        .checked_add(target_draws)
        .ok_or(EngineRefusal::CounterOverflow(
            "Distilled Chaos CombatTargets counter",
        ))?;
    Ok(())
}

thread_local! {
    /// Admission-only recursion brake for the clone-first Distilled rehearsal.
    /// The public transition that is being rehearsed re-enters potion
    /// validation before it consumes the source slot. Only that nested
    /// validation skips this one proof; all other potion prerequisites and
    /// the complete runtime continuation validators still execute normally.
    static DISTILLED_REHEARSAL_BYPASS: std::cell::Cell<Option<(usize, u8, usize)>> = const {
        std::cell::Cell::new(None)
    };
}

struct DistilledRehearsalGuard(Option<(usize, u8, usize)>);

impl DistilledRehearsalGuard {
    fn enter(state: &HotState, slot: u8) -> Result<Self, EngineRefusal> {
        let identity = (state as *const HotState as usize, slot);
        let validations = state
            .fanouts
            .potion_slots()
            .iter()
            .filter(|potion| **potion == Some(PotionId::DistilledChaos))
            .count()
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "Distilled rehearsal validation count",
            ))?;
        DISTILLED_REHEARSAL_BYPASS.with(|bypass| {
            if bypass.get().is_some() {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            let prior = bypass.replace(Some((identity.0, identity.1, validations)));
            Ok(Self(prior))
        })
    }
}

impl Drop for DistilledRehearsalGuard {
    fn drop(&mut self) {
        DISTILLED_REHEARSAL_BYPASS.with(|bypass| bypass.set(self.0.take()));
    }
}

fn consume_distilled_rehearsal_bypass(state: &HotState) -> bool {
    DISTILLED_REHEARSAL_BYPASS.with(|bypass| {
        let Some((identity, slot, remaining)) = bypass.get() else {
            return false;
        };
        if identity != state as *const HotState as usize
            || state
                .fanouts
                .potion_slots()
                .get(usize::from(slot))
                .copied()
                .flatten()
                != Some(PotionId::DistilledChaos)
        {
            return false;
        }
        // Only the direct validation plus the replay-root installer's
        // legal-action membership scan of this exact immutable predecessor
        // may pass. Any nested Distilled state has a different address.
        bypass.set((remaining > 1).then_some((identity, slot, remaining - 1)));
        true
    })
}

fn action_replay_root_count(state: &HotState) -> usize {
    state
        .frames
        .as_slice()
        .iter()
        .filter(|frame| matches!(frame, Frame::ActionReplay { .. }))
        .count()
}

fn rehearse_one_distilled_root(
    state: &HotState,
    catalog: &Catalog,
    slot: u8,
) -> Result<(), EngineRefusal> {
    if state
        .frames
        .as_slice()
        .iter()
        .any(|frame| matches!(frame, Frame::ActionReplay { .. }))
        || state.pending.is_some()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let _scope = DistilledRehearsalGuard::enter(state, slot)?;
    let mut events = Vec::new();
    let first = super::apply_action_with_replay_witness(
        state,
        catalog,
        &super::Action::UsePotion { slot, target: None },
        &mut events,
    )?;
    if first.pending.is_none() {
        return if first.frames.is_empty() {
            Ok(())
        } else {
            Err(EngineRefusal::ContinuationNotModeled)
        };
    }
    let original = match first.frames.as_slice().first().copied() {
        Some(Frame::ActionReplay { record }) => first
            .frames
            .action_replay(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if !matches!(
        original.action,
        crate::hot::ActionReplayRootAction::UsePotion {
            slot: root_slot,
            target: None,
        } if root_slot == slot
    ) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }

    super::rehearse_pending_replay_tree(first, catalog, &original, state)
}

/// Clone-first atomic proof of the complete current Distilled trajectory.
/// Unlike the retired replay-writer quotient, this executes the ordinary
/// runtime carrier, explores every legal parked answer, and therefore charges
/// nested AutoPlay, Imitation, Top/Hellraiser, UID, RNG, history, and all later
/// sibling effects exactly as the eventual public transition will.
pub(crate) fn preflight_distilled_batch_suffix(
    state: &HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    if consume_distilled_rehearsal_bypass(state) {
        return Ok(());
    }
    let replay_roots = action_replay_root_count(state);
    if state.pending.is_some() {
        // A held potion is not an action while another exact continuation is
        // parked. Admission separately authenticates that pending/frame
        // stack and the prospective recursive source closure. Rehearse the
        // complete concrete Distilled root only after this continuation has
        // resumed and the potion actually joins legal actions; starting a
        // second root here would reject every otherwise-valid parked state.
        return if replay_roots <= 1 {
            Ok(())
        } else {
            Err(EngineRefusal::ContinuationNotModeled)
        };
    }
    if replay_roots != 0 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if crate::engine::admission::unceasing_top_same_uid_hellraiser_reentry_is_reachable(
        state, catalog,
    ) {
        return Err(EngineRefusal::MalformedArgs(
            "Unceasing Top same-UID Hellraiser reentry",
        ));
    }
    let candidate_cards = [PileId::Draw, PileId::Discard]
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .collect::<Vec<_>>();
    if candidate_cards.len() > 1
        && state.card_states.dampen().is_some_and(|dampen| {
            dampen.cards.iter().any(|tracked| {
                candidate_cards
                    .iter()
                    .any(|candidate| candidate.uid == tracked.uid)
            })
        })
    {
        return Err(EngineRefusal::MalformedArgs(DISTILLED_DAMPEN_FRONTIER));
    }
    let slots = state
        .fanouts
        .potion_slots()
        .iter()
        .enumerate()
        .filter_map(|(slot, potion)| {
            (*potion == Some(PotionId::DistilledChaos))
                .then(|| u8::try_from(slot).ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    if slots.is_empty() {
        // Prospective Entropic closure has no held Distilled source yet. Its
        // generated-potion activation remains behind the separate R52E wall.
        return Ok(());
    }
    for slot in slots {
        rehearse_one_distilled_root(state, catalog, slot)?;
    }
    Ok(())
}

pub(crate) fn generation_choice_provenance_is_exact(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> bool {
    let Some(pool) = fight_generation_choice_pool(state, catalog, potion) else {
        return false;
    };
    super::cards::unlock_profile_is_recorded(state, catalog)
        && state.reward_card_pool.is_some()
        && pool.len() >= 3
        && state.multiplayer_ally_key == 0
        && !state.rng.is_vacant(RngStream::Generation)
        && pool.iter().all(|id| {
            let identity = CardIdentity {
                id: *id,
                upgrade: 0,
                enchantment: None,
            };
            catalog.atom(&identity).is_some()
        })
}

/// Cosmic Concoction's generation provenance (#3229).
///
/// The body's pool query is Colorless Potion's own (see
/// [`cosmic_concoction`]), so Colorless Potion's provenance is the base; on
/// top of it every pool row's post-`CardCmd::Upgrade` identity — L1 when
/// [`super::cards::native_card_is_upgradable`], else the unchanged L0 — must
/// be a catalog atom.
pub(crate) fn cosmic_concoction_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    generation_choice_provenance_is_exact(state, catalog, PotionId::ColorlessPotion)
        && fight_generation_choice_pool(state, catalog, PotionId::ColorlessPotion).is_some_and(
            |pool| {
                pool.iter()
                    .all(|id| catalog.atom(&cosmic_concoction_identity(*id)).is_some())
            },
        )
}

/// The identity `CardCmd::Upgrade` (`0x12f660`) leaves on a live, pile-less
/// generated L0 card: `IsUpgradable` (IL_002d) gates `UpgradeInternal`
/// (IL_0081); a pile-less card reaches no hook (IL_003e/IL_009f).
fn cosmic_concoction_identity(id: CardId) -> CardIdentity {
    let base = CardIdentity {
        id,
        upgrade: 0,
        enchantment: None,
    };
    if super::cards::native_card_is_upgradable(base) {
        CardIdentity { upgrade: 1, ..base }
    } else {
        base
    }
}

/// `CosmicConcoction/<OnUse>d__8::MoveNext` RVA `0x34c934`, OnUse `0xabed0`
/// (#3229).
///
/// `get_TargetType` (`0xabebd`) is 5 and `get_CanonicalVars` (`0xabec0`) is
/// `CardsVar(3)`. The body reads `target.Player` (IL_002e) and hands
/// `ModelDb::CardPool<ColorlessCardPool>().GetUnlockedCards(UnlockState,
/// CardMultiplayerConstraint)` (IL_0035-IL_004b) to
/// `CardFactory::GetDistinctForCombat(.., 3, Rng.CombatCardGeneration)`
/// (IL_0070) — instruction for instruction Colorless Potion's query
/// (`0x34c784` IL_003f-IL_006b), so the pool is
/// [`generation_choice_pool`]`(ColorlessPotion)` and the RNG cost is one
/// complete Generation-stream shuffle ([`super::cards::shuffle_generation_slice`]).
/// It then walks the returned cards in order: `CardCmd::Upgrade(card, 1)`
/// (IL_0098; single-card overload `0x12f64f` into `0x12f660`, which returns
/// at `IsEnding` IL_0011 and otherwise upgrades an `IsUpgradable` card with no
/// pile-bound hook) and one awaited
/// `CardPileCmd::AddGeneratedCardToCombat(card, Hand, Owner, true)` (IL_00a7)
/// — Colorless Potion's add shape (`0x34c784` IL_00ef-IL_00f7) without its
/// `SetToFreeThisTurn`, and Jack of All Trades' serial add
/// (`steps::neutral::jack_of_all_trades_body`), which this follows.
///
/// I5 seams, refused by name rather than read: a member reached while combat
/// is ending, or after an earlier member's add parked a continuation (the
/// native loop keeps going, with the Upgrade skipped at `IsEnding` and a
/// record-before-Add history question Jack and Colorless Potion answer
/// differently). The whole body is rehearsed first so either refusal is
/// atomic.
fn cosmic_concoction(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !cosmic_concoction_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Cosmic Concoction generation provenance",
        ));
    }
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let pool = fight_generation_choice_pool(state, catalog, PotionId::ColorlessPotion).ok_or(
            EngineRefusal::MalformedArgs("Cosmic Concoction generation provenance"),
        )?;
        let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
        for id in shuffled.into_iter().take(3) {
            // Checked before every member, the first included: an IsEnding
            // Upgrade would leave the card at L0 (`0x12f660` IL_0011).
            if super::damage::damage_combat_is_ending(state)
                || state.pending.is_some()
                || !matches!(state.frames.top(), Some(Frame::PotionFinish { .. }))
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Cosmic Concoction member after a combat-ending or parked add",
                ));
            }
            super::cards::inject_generated_exact_bottom(
                state,
                catalog,
                cosmic_concoction_identity(id),
                1,
                PileId::Hand,
                events,
            )?;
        }
        Ok(())
    }
    let mut probe = state.clone();
    apply(&mut probe, catalog, &mut Vec::new())?;
    apply(state, catalog, events)
}

pub(crate) fn manual_part_d_prerequisites_are_exact(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Result<(), EngineRefusal> {
    if matches!(potion, PotionId::BoneBrew | PotionId::PotOfGhouls)
        && state.multiplayer_ally_key != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Necrobinder potion player target",
        ));
    }
    let entropic_can_procure = potion == PotionId::EntropicBrew && !state.fanouts.potion_sozu();
    if entropic_can_procure {
        for generated in [
            PotionId::AttackPotion,
            PotionId::SkillPotion,
            PotionId::PowerPotion,
            PotionId::ColorlessPotion,
        ] {
            if !generation_choice_provenance_is_exact(state, catalog, generated) {
                return Err(EngineRefusal::MalformedArgs(
                    "Entropic generation-potion provenance",
                ));
            }
        }
    }
    if is_generation_choice(potion)
        && !generation_choice_provenance_is_exact(state, catalog, potion)
    {
        return Err(EngineRefusal::MalformedArgs(
            "generation potion pool provenance",
        ));
    }
    // #3229: Cosmic Concoction, held or procurable by Entropic Brew.
    if (potion == PotionId::CosmicConcoction
        || entropic_can_procure
            && factory_can_generate(state.reward_card_pool, PotionId::CosmicConcoction))
        && !cosmic_concoction_provenance_is_exact(state, catalog)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Cosmic Concoction generation provenance",
        ));
    }
    if potion == PotionId::OrobicAcid
        && (!generation_choice_provenance_is_exact(state, catalog, PotionId::AttackPotion)
            || !generation_choice_provenance_is_exact(state, catalog, PotionId::SkillPotion)
            || !generation_choice_provenance_is_exact(state, catalog, PotionId::PowerPotion))
    {
        return Err(EngineRefusal::MalformedArgs("Orobic generation provenance"));
    }
    if potion == PotionId::EntropicBrew
        && (generation_potion_profile(state, catalog).is_none()
            || state.multiplayer_ally_key != 0
            || state.rng.is_vacant(RngStream::PotionGeneration))
    {
        return Err(EngineRefusal::MalformedArgs(
            "Entropic Brew generation provenance",
        ));
    }
    if potion == PotionId::DistilledChaos || entropic_can_procure {
        preflight_distilled_target_rng(state, catalog)?;
        preflight_distilled_batch_suffix(state, catalog)?;
    }
    if (potion == PotionId::FruitJuice || entropic_can_procure)
        && !(0 <= state.hp && state.hp <= state.max_hp && state.max_hp <= 999_999_999)
    {
        return Err(EngineRefusal::MalformedArgs("Fruit Juice hp domain"));
    }
    let prospective_regen_uses = if entropic_can_procure {
        state.fanouts.potion_slots().len()
    } else {
        usize::from(potion == PotionId::RegenPotion)
    };
    if prospective_regen_uses > 0 {
        let regen_amount_is_exact = i32::try_from(prospective_regen_uses)
            .ok()
            .and_then(|count| count.checked_mul(5))
            .and_then(|amount| state.fanouts.regen().checked_add(amount))
            .is_some();
        if super::damage::null_applier_power_amount_changed_is_exact(state).is_err()
            || !regen_amount_is_exact
        {
            return Err(EngineRefusal::MalformedArgs("Regen Potion amount/order"));
        }
    }
    if (potion == PotionId::FairyInABottle || entropic_can_procure)
        && state.fanouts.potion_belt_buckle()
        && !state.fanouts.potion_belt_buckle_applied()
        && (super::damage::null_applier_power_amount_changed_is_exact(state).is_err()
            || state
                .powers
                .value(PowerId::Dexterity)
                .checked_add(2)
                .is_none())
    {
        return Err(EngineRefusal::MalformedArgs(
            "Fairy AfterPotionUsed Belt Buckle",
        ));
    }
    Ok(())
}

/// Admission-time Part D closure for a potion that may remain held across a
/// turn boundary.  Hand cards can route to Discard before a later Distilled
/// use, so its target/history/RNG proof must cover Hand in addition to the
/// immediate Draw+Discard gather domain used by public action validation.
pub(crate) fn prospective_part_d_prerequisites_are_exact(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Result<(), EngineRefusal> {
    manual_part_d_prerequisites_are_exact(state, catalog, potion)?;
    if potion == PotionId::DistilledChaos
        || potion == PotionId::EntropicBrew && !state.fanouts.potion_sozu()
    {
        let mut prospective = state.clone();
        let hand = prospective
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .split_off(0);
        prospective
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(hand);

        // A parked CardPlay's authenticated result route, force-exhaust bit,
        // and live routing powers may differ from its static card flags.
        // Conservatively include every physical Play owner in the future
        // gather domain; this cannot under-approximate a resumable route.
        let routable_play = state.piles.get(PileId::Play).as_slice().to_vec();
        prospective
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(routable_play);

        // Summon Forth can return an exhausted Sovereign Blade to Draw.  This
        // is the same physical move pinned by
        // `summon_forth_both_levels_move_one_ordered_plural_batch_then_forge`:
        // preserve its already-authenticated payload instead of repairing a
        // malformed Blade while projecting it. No other exhausted physical is
        // included merely because it exists.
        let summon_forth_reachable = catalog
            .reachable_specs()
            .any(|spec| spec.identity.id == CardId::SummonForth);
        if summon_forth_reachable {
            let exhaust = prospective.piles.get_mut(PileId::Exhaust).make_mut();
            let mut returning_blades = Vec::new();
            let mut index = 0;
            while index < exhaust.len() {
                let card = exhaust[index];
                if catalog
                    .spec(card.atom)
                    .is_some_and(|spec| spec.identity.id == CardId::SovereignBlade)
                {
                    returning_blades.push(exhaust.remove(index));
                } else {
                    index += 1;
                }
            }
            prospective
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(returning_blades);
        }
        preflight_distilled_target_rng(&prospective, catalog)?;
        preflight_distilled_batch_suffix(&prospective, catalog)?;

        if !crate::engine::admission::distilled_recursive_source_closure_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs(
                "Distilled prospective recursive source closure",
            ));
        }
    }
    Ok(())
}

fn begin_generation_choice(
    state: &mut HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Result<(), EngineRefusal> {
    let draws = fight_generation_choice_pool(state, catalog, potion)
        .and_then(|pool| u64::try_from(pool.len().saturating_sub(1)).ok())
        .ok_or(EngineRefusal::MalformedArgs("generation potion pool"))?;
    state
        .rng
        .get(RngStream::Generation)
        .counter
        .checked_add(draws)
        .ok_or(EngineRefusal::CounterOverflow("generation potion RNG"))?;
    if !generation_choice_provenance_is_exact(state, catalog, potion) {
        return Err(EngineRefusal::MalformedArgs(
            "generation potion pool provenance",
        ));
    }
    let shuffled = super::cards::shuffle_generation_slice(
        state,
        &fight_generation_choice_pool(state, catalog, potion).expect("validated generation potion"),
    )?;
    let options = shuffled
        .into_iter()
        .take(3)
        .map(|id| {
            catalog
                .atom(&CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .ok_or(EngineRefusal::UnknownMintIdentity(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: potion,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::GenerationSelecting,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: options,
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let Frame::PotionFinish { record } = state
        .frames
        .top()
        .ok_or(EngineRefusal::ContinuationNotModeled)?
    else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    state.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
        frame_uid: crate::hot::GENERATION_POTION_PENDING_UID,
        frame_record: record,
    }));
    Ok(())
}

// PotionFactory.GetOptions / CreateRandomPotionOutOfCombat, current
// v0.111.0 authority: RVAs 0x112fba/0x112edc.  These are the stable filtered
// fully-unlocked solo-Ironclad rarity partitions.  Entropic Brew deliberately
// uses the out-of-combat pool, including the three passive-only rows.
#[cfg(test)]
pub(crate) const ENTROPIC_COMMON_POOL: [PotionId; 16] = [
    PotionId::BloodPotion,
    PotionId::AttackPotion,
    PotionId::BlockPotion,
    PotionId::ColorlessPotion,
    PotionId::DexterityPotion,
    PotionId::EnergyPotion,
    PotionId::ExplosiveAmpoule,
    PotionId::FirePotion,
    PotionId::FlexPotion,
    PotionId::PowerPotion,
    PotionId::SkillPotion,
    PotionId::SpeedPotion,
    PotionId::StrengthPotion,
    PotionId::SwiftPotion,
    PotionId::VulnerablePotion,
    PotionId::WeakPotion,
];
#[cfg(test)]
pub(crate) const ENTROPIC_UNCOMMON_POOL: [PotionId; 16] = [
    PotionId::Ashwater,
    PotionId::BlessingOfTheForge,
    PotionId::Clarity,
    PotionId::CureAll,
    PotionId::Duplicator,
    PotionId::Fortifier,
    PotionId::FyshOil,
    PotionId::GamblersBrew,
    PotionId::HeartOfIron,
    PotionId::LiquidBronze,
    PotionId::PotionOfBinding,
    PotionId::PowderedDemise,
    PotionId::RadiantTincture,
    PotionId::RegenPotion,
    PotionId::StableSerum,
    PotionId::TouchOfInsanity,
];
#[cfg(test)]
pub(crate) const ENTROPIC_RARE_POOL: [PotionId; 16] = [
    PotionId::SoldiersStew,
    PotionId::BeetleJuice,
    PotionId::BottledPotential,
    PotionId::DistilledChaos,
    PotionId::DropletOfPrecognition,
    PotionId::EntropicBrew,
    PotionId::FairyInABottle,
    PotionId::FruitJuice,
    PotionId::GigantificationPotion,
    PotionId::LiquidMemories,
    PotionId::LuckyTonic,
    PotionId::MazalethsGift,
    PotionId::OrobicAcid,
    PotionId::ShacklingPotion,
    PotionId::ShipInABottle,
    PotionId::SneckoOil,
];

#[cfg(test)]
pub(crate) const ALCHEMIZE_UNCOMMON_POOL: [PotionId; 15] = [
    PotionId::Ashwater,
    PotionId::BlessingOfTheForge,
    PotionId::Clarity,
    PotionId::CureAll,
    PotionId::Duplicator,
    PotionId::Fortifier,
    PotionId::FyshOil,
    PotionId::GamblersBrew,
    PotionId::HeartOfIron,
    PotionId::LiquidBronze,
    PotionId::PotionOfBinding,
    PotionId::PowderedDemise,
    PotionId::RadiantTincture,
    PotionId::StableSerum,
    PotionId::TouchOfInsanity,
];

#[cfg(test)]
pub(crate) const ALCHEMIZE_RARE_POOL: [PotionId; 14] = [
    PotionId::SoldiersStew,
    PotionId::BeetleJuice,
    PotionId::BottledPotential,
    PotionId::DistilledChaos,
    PotionId::DropletOfPrecognition,
    PotionId::EntropicBrew,
    PotionId::GigantificationPotion,
    PotionId::LiquidMemories,
    PotionId::LuckyTonic,
    PotionId::MazalethsGift,
    PotionId::OrobicAcid,
    PotionId::ShacklingPotion,
    PotionId::ShipInABottle,
    PotionId::SneckoOil,
];

/// PotionFactory::GetPotionOptions RVA 0x112fba reads the target's character
/// potion pool, then SharedPotionPool. CreateRandomPotionInCombat (0x112edc)
/// additionally filters CanBeGeneratedInCombat. Rarity is rolled before the
/// uniform NextItem; both consume PotionGeneration, even for a full belt.
/// None preserves the legacy canonical Ironclad default. Captured reviews
/// always supply their actual character; no other owner falls back.
pub(crate) fn factory_pool(
    owner: Option<crate::catalog::RewardPool>,
) -> &'static [(PotionId, u8, bool)] {
    let owner = owner.unwrap_or(crate::catalog::RewardPool::Ironclad);
    crate::content_tables::CHARACTER_POTION_POOL_ROWS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(owner.as_str()))
        .expect("every RewardPool has a generated potion pool")
        .1
}

pub(crate) fn factory_can_generate(
    owner: Option<crate::catalog::RewardPool>,
    potion: PotionId,
) -> bool {
    factory_pool(owner).iter().any(|(id, _, _)| *id == potion)
}

/// The unlock epoch gating each [`factory_pool`] row, row for row.
fn factory_pool_row_epochs(
    owner: Option<crate::catalog::RewardPool>,
) -> &'static [Option<&'static str>] {
    let owner = owner.unwrap_or(crate::catalog::RewardPool::Ironclad);
    crate::content_tables::CHARACTER_POTION_POOL_ROW_EPOCHS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(owner.as_str()))
        .expect("every RewardPool has a generated potion pool")
        .1
}

/// What this fight proves about the epochs gating the potion factory's pool
/// (#3343): the recorded profile, and the document flag
/// `fully_unlocked_potion_pool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GenerationPotionProfile<'c> {
    /// `UnlockState.unlocked_epochs` as recorded ([`Catalog::wire_unlock_epochs`]).
    /// An epoch it omits is hidden.
    recorded: Option<&'c [&'static str]>,
    /// The flag, which proves exactly
    /// [`crate::entry::unlocks::FULLY_UNLOCKED_POTION_POOL_FLAG_EPOCHS`].
    flag: bool,
}

impl GenerationPotionProfile<'_> {
    fn reveals(self, epoch: &str) -> bool {
        self.recorded.is_some_and(|epochs| epochs.contains(&epoch))
            || self.flag
                && crate::entry::unlocks::FULLY_UNLOCKED_POTION_POOL_FLAG_EPOCHS.contains(&epoch)
    }
}

/// The proof Alchemize and Entropic Brew filter the owner's factory pool by,
/// or `None` when the fight cannot decide that pool (#3343).
///
/// Native: both sources hand ONE player to the potion factory, and the
/// factory reads that player's pool and `UnlockState` and nothing else
/// (v0.111.0 `sts2.dll` SHA-256 `9cb4f1ad…12b4`):
///
/// * `Alchemize/<OnPlay>d__5::MoveNext` `0x3897c8` passes `Owner`
///   (IL_009f) and `Owner.RunState.Rng.CombatPotionGeneration`
///   (IL_00a5-IL_00b4) to `PotionFactory::CreateRandomPotionInCombat`
///   (IL_00ba);
/// * `EntropicBrew/<OnUse>d__6::MoveNext` `0x34d33c` stores
///   `target.Player` (IL_0026-IL_0030) and passes it, with its
///   `CombatPotionGeneration`, to `CreateRandomPotionOutOfCombat` (IL_0056);
/// * both factory entries (`0x112edc` IL_000d; `CreateRandomPotionsOutOfCombat`
///   `0x112eb4` IL_000d) call `GetPotionOptions` `0x112fba`:
///   `player.Character.PotionPool.GetUnlockedPotions(player.UnlockState)`
///   (IL_0002-IL_0012) concatenated with
///   `ModelDb.PotionPool<SharedPotionPool>().GetUnlockedPotions(player.UnlockState)`
///   (IL_0017-IL_0027);
/// * each `<Character>PotionPool::GetUnlockedPotions` makes ONE test on its
///   own character's fourth epoch (IL_000d) and returns
///   `Empty<PotionModel>()` when it is hidden (IL_0014): Ironclad `0xadd38`
///   (`Ironclad4Epoch`), Silent `0xae088`, Defect `0xadca8`, Necrobinder
///   `0xadd98`, Regent `0xaddd8`;
/// * `SharedPotionPool::GetUnlockedPotions` `0xadfb4` removes
///   `Potion1Epoch.Potions` unless revealed (IL_0019-IL_0037) and then
///   `Potion2Epoch.Potions` (IL_0057-IL_0077), preserving order.
///
/// Neither path applies a multiplayer constraint. The engine still refuses a
/// multiplayer roster by name, because an ally owner or target would need that
/// player's own pool and profile.
///
/// So the pool is a function of the owner's fourth epoch and `POTION1/2`
/// alone. These are the per-row epochs of `CHARACTER_POTION_POOL_ROW_EPOCHS`.
/// A recorded profile decides every one of them. The flag
/// `fully_unlocked_potion_pool` is `IRONCLAD4 + POTION1/2`, so on its own it
/// decides only the Ironclad pool (or an absent owner, which [`factory_pool`]
/// reads as Ironclad). Before #3343 the flag gated every owner, which drew
/// Defect's three potions under a profile that hides `DEFECT4_EPOCH`.
///
/// An epoch counts as revealed when either proof reveals it. A real opening
/// derives both from the same save, so they agree. A synthetic document can
/// pair the flag with the canonical card-pool profile, which omits `POTION1/2`;
/// the flag's positive proof then keeps those rows.
pub(crate) fn generation_potion_profile<'c>(
    state: &HotState,
    catalog: &'c Catalog,
) -> Option<GenerationPotionProfile<'c>> {
    let profile = GenerationPotionProfile {
        recorded: catalog.wire_unlock_epochs(),
        flag: state.fanouts.fully_unlocked_potion_pool(),
    };
    (profile.recorded.is_some()
        || factory_pool_row_epochs(state.reward_card_pool)
            .iter()
            .flatten()
            .all(|gate| profile.reveals(gate)))
    .then_some(profile)
}

/// The belt's exactness, with the pool proof a materialised belt needs
/// decided the way generation decides it (#3347).
///
/// A held belt reads no pool. Its slots are recorded facts; their identities
/// are checked at the boundary (`PotionId`, `inert_potions`), and holding,
/// using or discarding a potion never consults `GetPotionOptions`. The pool is
/// read only by the potion factory, whose every caller in v0.111.0 `sts2.dll`
/// (SHA-256 `9cb4f1ad…12b4`, found by scanning every method body for calls to
/// `PotionFactory::CreateRandomPotion*` / `GetPotionOptions`) is one of:
///
/// * `Alchemize/<OnPlay>d__5::MoveNext` `0x3897c8` IL_00ba and
///   `EntropicBrew/<OnUse>d__6::MoveNext` `0x34d33c` IL_0056, the two
///   in-combat generators. Both gate on [`generation_potion_profile`], which
///   refuses by name when neither proof decides the owner's pool;
/// * `DelicateFrond/<BeforeCombatStart>d__2::MoveNext` `0x3229bc` IL_0044,
///   which runs before any combat state exists and whose owner the opening
///   refuses (`OPENING_WINDOW_RELIC_BODIES`);
/// * rewards, shops and relic pickups, all outside combat:
///   `PotionReward::Populate` `0x5d290` IL_0038,
///   `CrystalSpherePotion::ToReward` `0x114a40` IL_000d,
///   `MerchantInventory::PopulatePotionEntries` `0x11c390` IL_0024,
///   `MerchantPotionEntry::FillSlot` `0x11c4eb` IL_0019,
///   `AlchemicalCoffer/<AfterObtained>d__7::MoveNext` `0x31e550` IL_00db,
///   `PhialHolster/<AfterObtained>d__8::MoveNext` `0x32ddd4` IL_00ca.
///
/// So the pool proof a belt carries is generation provenance and nothing
/// else. Before #3347 a materialised belt required the solo-Ironclad flag
/// `fully_unlocked_potion_pool` (`IRONCLAD4 + POTION1/2`), which refused a
/// whole Silent, Defect, Necrobinder or Regent fight whose profile hides
/// `IRONCLAD4` although no generation reads that epoch for its owner. A belt
/// is now exact under either proof generation accepts: the flag, or a
/// recorded unlock profile ([`Catalog::wire_unlock_epochs`]). A belt with
/// neither still refuses, as before.
pub(crate) fn potion_belt_state_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    state.fanouts.potion_belt_topology_is_exact()
        && (state.fanouts.potion_slots().is_empty()
            || state.fanouts.fully_unlocked_potion_pool()
            || catalog.wire_unlock_epochs().is_some())
}

/// The rolled rarity's rows of the owner's factory pool. `profile: None` is
/// the fully-unlocked projection.
fn owner_random_potion_pool_for_roll(
    owner: Option<crate::catalog::RewardPool>,
    profile: Option<GenerationPotionProfile<'_>>,
    roll: f32,
    in_combat: bool,
) -> Vec<PotionId> {
    let rarity = if roll <= f32::from_bits(0x3d_cc_cc_cd) {
        3
    } else if roll <= f32::from_bits(0x3e_b3_33_33) {
        2
    } else {
        1
    };
    factory_pool(owner)
        .iter()
        .zip(factory_pool_row_epochs(owner))
        .filter(|((_, r, combat), gate)| {
            *r == rarity
                && (!in_combat || *combat)
                && gate.is_none_or(|gate| profile.is_none_or(|profile| profile.reveals(gate)))
        })
        .map(|((id, _, _), _)| *id)
        .collect()
}

#[cfg(test)]
pub(crate) fn random_potion_pool_for_roll(roll: f32, in_combat: bool) -> Vec<PotionId> {
    owner_random_potion_pool_for_roll(None, None, roll, in_combat)
}

fn random_potion_from_factory(
    state: &mut HotState,
    profile: Option<GenerationPotionProfile<'_>>,
    in_combat: bool,
) -> Result<PotionId, EngineRefusal> {
    let live = state.rng.get(RngStream::PotionGeneration);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let roll = rng.next_float(1.0);
    let pool = owner_random_potion_pool_for_roll(state.reward_card_pool, profile, roll, in_combat);
    // `CreateRandomPotions` `0x112f2c` takes `NextItem` over the rolled
    // rarity's rows (IL_0059-IL_006c); an empty bucket has no native item to
    // procure, so it refuses by name rather than index nothing.
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "potion generation empty rarity pool",
        ));
    }
    let index: usize = rng
        .next_bounded(i32::try_from(pool.len()).expect("fixed pool fits i32"))
        .map_err(|_| EngineRefusal::CounterOverflow("potion generation"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("potion generation"))?;
    state.rng.set(
        RngStream::PotionGeneration,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(pool[index])
}

fn random_entropic_potion(
    state: &mut HotState,
    profile: GenerationPotionProfile<'_>,
) -> Result<PotionId, EngineRefusal> {
    random_potion_from_factory(state, Some(profile), false)
}

/// Execute one exact Alchemize body.
///
/// The pool is the card owner's, filtered by the owner's recorded profile
/// ([`generation_potion_profile`], #3343).
pub(crate) fn alchemize_one_attempt(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let Some(profile) = generation_potion_profile(state, catalog)
        .filter(|_| state.multiplayer_ally_key == 0)
        .filter(|_| !state.rng.is_vacant(RngStream::PotionGeneration))
    else {
        return Err(EngineRefusal::MalformedArgs(
            "Alchemize generation provenance",
        ));
    };
    state
        .rng
        .get(RngStream::PotionGeneration)
        .counter
        .checked_add(2)
        .ok_or(EngineRefusal::CounterOverflow(
            "Alchemize potion generation counter",
        ))?;
    let generated = random_potion_from_factory(state, Some(profile), true)?;
    let _ = procure_potion(state, generated, events)?;
    Ok(())
}

/// Entropic Brew's generation loop. The pool is the target player's (solo:
/// the owner's), filtered by the recorded profile
/// ([`generation_potion_profile`], #3343).
pub(crate) fn entropic_brew(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(profile) = generation_potion_profile(state, catalog)
        .filter(|_| state.multiplayer_ally_key == 0)
        .filter(|_| !state.rng.is_vacant(RngStream::PotionGeneration))
    else {
        return Err(EngineRefusal::MalformedArgs(
            "Entropic Brew generation provenance",
        ));
    };
    let attempts = if state.fanouts.potion_sozu()
        || !state.fanouts.potion_slots().iter().any(Option::is_none)
    {
        1
    } else {
        state
            .fanouts
            .potion_slots()
            .iter()
            .filter(|slot| slot.is_none())
            .count()
    };
    let draws = u64::try_from(attempts)
        .ok()
        .and_then(|attempts| attempts.checked_mul(2))
        .ok_or(EngineRefusal::CounterOverflow("Entropic RNG"))?;
    state
        .rng
        .get(RngStream::PotionGeneration)
        .counter
        .checked_add(draws)
        .ok_or(EngineRefusal::CounterOverflow("Entropic RNG counter"))?;
    loop {
        let generated = random_entropic_potion(state, profile)?;
        let procured = procure_potion(state, generated, events)?;
        if !procured || !state.fanouts.potion_slots().iter().any(Option::is_none) {
            return Ok(());
        }
    }
}

/// BoneBrew/<OnUse>d__10::MoveNext RVA 0x34c2b0 awaits
/// OstyCmd::Summon(target.Player, 15). TargetType (0xabcc1) is Player-only;
/// living Osty grows both HP and max HP, and no target-pet choice exists.
fn bone_brew(state: &mut HotState) -> Result<(), EngineRefusal> {
    if !state.history.over {
        super::summon_osty(state, 15, "Bone Brew summon")?;
    }
    Ok(())
}

/// PotOfGhouls/<OnUse>d__10::MoveNext RVA 0x34f940 calls
/// Soul::CreateInHand(target.Player, 2, combatState, potion.Owner).
/// TargetType (0xad241) is Player-only, so the solo owner is the recipient.
/// Soul/<CreateInHand>d__5::MoveNext (0x3bced4) awaits one plural Add.
/// This is one plural generated-card command: one leading ending gate, then
/// each Soul records history before its own Add/overflow and generated hook.
fn pot_of_ghouls(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let soul = CardIdentity {
        id: CardId::Soul,
        upgrade: 0,
        enchantment: None,
    };
    super::cards::inject_before_hand_draw_generated_batch_bottom(
        state,
        catalog,
        &[soul, soul],
        events,
    )
}

fn orobic_options_are_exact(
    state: &HotState,
    catalog: &Catalog,
    options: &[crate::catalog::CardAtom],
) -> bool {
    let Some(pools) = orobic_generation_pools(state, catalog) else {
        return false;
    };
    options.len() == 3
        && options.iter().zip(pools).all(|(atom, pool)| {
            catalog.spec(*atom).is_some_and(|spec| {
                spec.identity.upgrade == 0
                    && spec.identity.enchantment.is_none()
                    && pool.contains(&spec.identity.id)
            })
        })
}

fn run_orobic_batch(
    state: &mut HotState,
    catalog: &Catalog,
    options: Vec<crate::catalog::CardAtom>,
    mut cursor: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !orobic_options_are_exact(state, catalog, &options) || cursor > options.len() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    while cursor < options.len() {
        let atom = options[cursor];
        let identity = catalog
            .spec(atom)
            .ok_or(EngineRefusal::UnknownAtom(atom))?
            .identity;
        let generated_uid = state.next_card_uid;
        if !state.history.over {
            // `u32::MAX` is the packed-frame sentinel for "no current child".
            // Refuse the live allocation before publishing an
            // indistinguishable OrobicAfterChild owner. Once ending has begun,
            // generated-card history/hooks still run but Add allocates no UID,
            // so no child carrier is installed for those remaining members.
            if generated_uid == u32::MAX {
                return Err(EngineRefusal::CounterOverflow("next_card_uid"));
            }
            state
                .frames
                .replace_top_potion_finish(&PotionFinishRecord {
                    name: PotionId::OrobicAcid,
                    stage: PotionFinishStage::Effect,
                    body_stage: PotionBodyStage::OrobicAfterChild,
                    current_uid: Some(generated_uid),
                    aux: u32::try_from(cursor + 1)
                        .map_err(|_| EngineRefusal::CounterOverflow("Orobic cursor"))?,
                    candidates: Vec::new(),
                    generation_options: options.clone(),
                })
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        }
        super::cards::inject_generated_free_this_turn_bottom(
            state,
            catalog,
            identity,
            PileId::Hand,
            events,
        )?;
        if state.pending.is_some()
            || !matches!(state.frames.top(), Some(Frame::PotionFinish { .. }))
        {
            return Ok(());
        }
        cursor += 1;
    }
    restore_after_draw(state, PotionId::OrobicAcid)?;
    finish(state, catalog, PotionId::OrobicAcid, events)
}

fn begin_orobic(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !generation_choice_provenance_is_exact(state, catalog, PotionId::AttackPotion)
        || !generation_choice_provenance_is_exact(state, catalog, PotionId::SkillPotion)
        || !generation_choice_provenance_is_exact(state, catalog, PotionId::PowerPotion)
    {
        return Err(EngineRefusal::MalformedArgs("Orobic generation provenance"));
    }
    let draws = u64::try_from(orobic_rng_draws(state, catalog)?)
        .map_err(|_| EngineRefusal::CounterOverflow("Orobic Generation RNG"))?;
    state
        .rng
        .get(RngStream::Generation)
        .counter
        .checked_add(draws)
        .ok_or(EngineRefusal::CounterOverflow("Orobic Generation RNG"))?;
    let [attack_pool, skill_pool, power_pool] = orobic_generation_pools(state, catalog)
        .ok_or(EngineRefusal::MalformedArgs("Orobic generation owner"))?;
    let attack = super::cards::shuffle_generation_slice(state, &attack_pool)?[0];
    let skill = super::cards::shuffle_generation_slice(state, &skill_pool)?[0];
    let power = super::cards::shuffle_generation_slice(state, &power_pool)?[0];
    let options = [attack, skill, power]
        .into_iter()
        .map(|id| {
            catalog
                .atom(&CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .ok_or(EngineRefusal::UnknownMintIdentity(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // GenerateCardsCmd tests the combat-ending gate once before entering its
    // plural Add loop. The three pool shuffles above still occur, but a command
    // that starts after ending publishes no generated history, hook, or UID.
    // Do not repeat this gate in `run_orobic_batch`: ending caused by member 1
    // still lets members 2/3 publish their record-before-Add callbacks.
    if state.history.over {
        restore_after_draw(state, PotionId::OrobicAcid)?;
        return finish(state, catalog, PotionId::OrobicAcid, events);
    }
    // Rehearse the complete plural generated-card transaction after all 75
    // RNG draws but before its first history/UID/pile publication.
    let mut probe = state.clone();
    probe
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: PotionId::OrobicAcid,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    run_orobic_batch(&mut probe, catalog, options.clone(), 0, &mut Vec::new())?;
    run_orobic_batch(state, catalog, options, 0, events)
}

pub(crate) fn resume_orobic_after_child(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let record = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    let options = record.generation_options().collect::<Vec<_>>();
    let cursor = usize::try_from(record.aux).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    if record.name != PotionId::OrobicAcid
        || record.stage != PotionFinishStage::Effect
        || record.body_stage != PotionBodyStage::OrobicAfterChild
        || record.current_uid.is_none()
        || !orobic_options_are_exact(state, catalog, &options)
        || cursor == 0
        || cursor > options.len()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    run_orobic_batch(state, catalog, options, cursor, events)
}

pub(crate) fn orobic_after_child_is_exact(
    state: &HotState,
    catalog: &Catalog,
    record: crate::hot::PotionFinishView<'_>,
) -> bool {
    let options = record.generation_options().collect::<Vec<_>>();
    let Some(uid) = record.current_uid else {
        return false;
    };
    record.name == PotionId::OrobicAcid
        && record.stage == PotionFinishStage::Effect
        && record.body_stage == PotionBodyStage::OrobicAfterChild
        && usize::try_from(record.aux).is_ok_and(|cursor| (1..=3).contains(&cursor))
        && orobic_options_are_exact(state, catalog, &options)
        && super::play::unique_live_card_location(state, uid)
            .ok()
            .flatten()
            .is_some()
}

/// The complete public body census.
///
/// Factory reachability is character-specific and separate from this body
/// census. A supported held potion need not belong to the owner's factory pool.
pub(crate) const SUPPORTED: [PotionId; 62] = [
    PotionId::Ashwater,
    PotionId::AttackPotion,
    PotionId::BeetleJuice,
    PotionId::BlessingOfTheForge,
    PotionId::BlockPotion,
    PotionId::BloodPotion,
    PotionId::BoneBrew,
    PotionId::BottledPotential,
    PotionId::Clarity,
    PotionId::ColorlessPotion,
    PotionId::CosmicConcoction,
    PotionId::CunningPotion,
    PotionId::CureAll,
    PotionId::DexterityPotion,
    PotionId::DistilledChaos,
    PotionId::DropletOfPrecognition,
    PotionId::Duplicator,
    PotionId::EnergyPotion,
    PotionId::EntropicBrew,
    PotionId::EssenceOfDarkness,
    PotionId::ExplosiveAmpoule,
    PotionId::FairyInABottle,
    PotionId::FirePotion,
    PotionId::FlexPotion,
    PotionId::FocusPotion,
    PotionId::Fortifier,
    PotionId::FoulPotion,
    PotionId::FruitJuice,
    PotionId::FyshOil,
    PotionId::GamblersBrew,
    PotionId::GhostInAJar,
    PotionId::GigantificationPotion,
    PotionId::GlowwaterPotion,
    PotionId::HeartOfIron,
    PotionId::LiquidBronze,
    PotionId::LiquidMemories,
    PotionId::LuckyTonic,
    PotionId::MazalethsGift,
    PotionId::OrobicAcid,
    PotionId::PoisonPotion,
    PotionId::PotionOfBinding,
    PotionId::PotionOfCapacity,
    PotionId::PotionOfDoom,
    PotionId::PotionShapedRock,
    PotionId::PotOfGhouls,
    PotionId::PowderedDemise,
    PotionId::PowerPotion,
    PotionId::RadiantTincture,
    PotionId::RegenPotion,
    PotionId::ShacklingPotion,
    PotionId::ShipInABottle,
    PotionId::SkillPotion,
    PotionId::SneckoOil,
    PotionId::SoldiersStew,
    PotionId::SpeedPotion,
    PotionId::StableSerum,
    PotionId::StarPotion,
    PotionId::StrengthPotion,
    PotionId::SwiftPotion,
    PotionId::TouchOfInsanity,
    PotionId::VulnerablePotion,
    PotionId::WeakPotion,
];

pub(crate) fn is_supported(potion: PotionId) -> bool {
    SUPPORTED.contains(&potion)
}

/// Whether this potion may own a parked public PotionFinish suffix.
///
/// Fairy remains passive and its certified phase/effect-depth callers cannot
/// park Top.
pub(crate) fn active_finish_is_supported(_state: &HotState, potion: PotionId) -> bool {
    is_supported(potion) && potion != PotionId::FairyInABottle
}

pub(crate) fn is_resumable(potion: PotionId) -> bool {
    matches!(
        potion,
        PotionId::Ashwater
            | PotionId::GamblersBrew
            | PotionId::TouchOfInsanity
            | PotionId::DropletOfPrecognition
            | PotionId::LiquidMemories
            | PotionId::SwiftPotion
            | PotionId::Clarity
            | PotionId::CureAll
            | PotionId::SneckoOil
            | PotionId::BottledPotential
            | PotionId::AttackPotion
            | PotionId::SkillPotion
            | PotionId::PowerPotion
            | PotionId::ColorlessPotion
            | PotionId::OrobicAcid
            | PotionId::DistilledChaos
            | PotionId::FairyInABottle
            // Glowwater's whole-hand Exhaust walk can park on a Dark Embrace
            // Draw, and its `Draw(10)` tail can park on an AfterCardDrawn
            // listener.
            | PotionId::GlowwaterPotion
    )
}

/// The native `PotionModel.get_TargetType` of one supported body.
///
/// Every value is the constant (or, for Foul Potion, the in-combat arm) that
/// the potion's own `get_TargetType` override returns in v0.111.0
/// (`sts2.dll` sha256 `9cb4f1ad…`), read 2026-09-26 for #2497. The numbering
/// is the `MegaCrit.Sts2.Core.Entities.Cards.TargetType` enum's Constant rows:
/// 1 Self, 2 AnyEnemy, 3 AllEnemies, 5 AnyPlayer, 6 AnyAlly, 8
/// TargetedNoCreature (4 RandomEnemy, 7 AllAllies and 9 Osty are unused by any
/// supported body). Each arm cites its getter RVA; every one of them is a bare
/// `ldc.i4.N; ret` except Foul Potion (`0xac691`), which returns 3 while
/// `CombatManager.IsInProgress` and 8 otherwise — combat is the only place a
/// manual use reaches this engine.
///
/// `None` for a body outside [`SUPPORTED`]: those refuse before any target
/// question is asked, and a body added to the supported set owes its getter
/// read here (the `supported_potion_native_target_types_are_pinned` witness
/// fails until it does).
pub(crate) fn native_target_type(potion: PotionId) -> Option<u8> {
    Some(match potion {
        // TargetType 1 (Self): `FairyInABottle::get_TargetType` 0xac3c9.
        PotionId::FairyInABottle => 1,
        // TargetType 2 (AnyEnemy).
        PotionId::BeetleJuice // 0xabaa1
        | PotionId::FirePotion // 0xac481
        | PotionId::PoisonPotion // 0xacf6d (#3229)
        | PotionId::PotionOfDoom // 0xad131
        | PotionId::PotionShapedRock // 0xad1c9
        | PotionId::PowderedDemise // 0xad2b9
        | PotionId::VulnerablePotion // 0xadae1
        | PotionId::WeakPotion => 2, // 0xadb79
        // TargetType 3 (AllEnemies).
        PotionId::ExplosiveAmpoule // 0xac359
        | PotionId::FoulPotion // 0xac691, in-combat arm
        | PotionId::PotionOfBinding // 0xad005
        | PotionId::ShacklingPotion => 3, // 0xad4cd
        // TargetType 5 (AnyPlayer).
        PotionId::Ashwater // 0xab9b9
        | PotionId::AttackPotion // 0xaba3d
        | PotionId::BlessingOfTheForge // 0xabb31
        | PotionId::BlockPotion // 0xabbca
        | PotionId::BloodPotion // 0xabc4d
        | PotionId::BoneBrew // 0xabcc1
        | PotionId::BottledPotential // 0xabd5d
        | PotionId::Clarity // 0xabdd1
        | PotionId::ColorlessPotion // 0xabe59
        | PotionId::CosmicConcoction // 0xabebd (#3229)
        | PotionId::CunningPotion // 0xabf29 (#3229)
        | PotionId::CureAll // 0xabfa1
        | PotionId::DexterityPotion // 0xac042
        | PotionId::DistilledChaos // 0xac0d9
        | PotionId::DropletOfPrecognition // 0xac14d
        | PotionId::Duplicator // 0xac1b1
        | PotionId::EnergyPotion // 0xac20d
        | PotionId::EntropicBrew // 0xac285
        | PotionId::EssenceOfDarkness // 0xac2d9 (#3229)
        | PotionId::FlexPotion // 0xac4f9
        | PotionId::FocusPotion // 0xac591
        | PotionId::Fortifier // 0xac629
        | PotionId::FruitJuice // 0xac875
        | PotionId::FyshOil // 0xac8e9
        | PotionId::GamblersBrew // 0xac9b1
        | PotionId::GhostInAJar // 0xaca15
        | PotionId::GigantificationPotion // 0xacaad
        | PotionId::GlowwaterPotion // 0xacb25
        | PotionId::HeartOfIron // 0xacba5
        | PotionId::LiquidBronze // 0xaccc9
        | PotionId::LiquidMemories // 0xacd61
        | PotionId::LuckyTonic // 0xacdc5
        | PotionId::MazalethsGift // 0xace5d
        | PotionId::OrobicAcid // 0xacf11
        | PotionId::PotOfGhouls // 0xad241
        | PotionId::PotionOfCapacity // 0xad0c5 (#3229)
        | PotionId::PowerPotion // 0xad335
        | PotionId::RadiantTincture // 0xad399
        | PotionId::RegenPotion // 0xad431
        | PotionId::ShipInABottle // 0xad55d
        | PotionId::SkillPotion // 0xad5e9
        | PotionId::SneckoOil // 0xad64d
        | PotionId::SoldiersStew // 0xad711
        | PotionId::SpeedPotion // 0xad7ee
        | PotionId::StableSerum // 0xad885
        | PotionId::StarPotion // 0xad905
        | PotionId::StrengthPotion // 0xad971
        | PotionId::SwiftPotion // 0xada09
        | PotionId::TouchOfInsanity => 5, // 0xada7d
        _ => return None,
    })
}

/// Whether `PotionModel::IsValidTarget` can never accept a pet for this body.
///
/// `PotionModel::IsValidTarget` (`0x8328c`) is the only admission a picked
/// creature passes, and it decides by `get_TargetType`:
///
/// - a null target (IL_000c-IL_0028) is valid for kind 8 and for every
///   non-single-target kind, so an all-enemies body (3) never picks anyone;
/// - a dead target is rejected (IL_0029-IL_0032);
/// - kind 2 (IL_0033-IL_0057) wants `target.Side != Owner.Creature.Side`. An
///   Osty is on its owner's side, so it is never an AnyEnemy pick;
/// - kind 6 (IL_0058-IL_008c) wants the owner's side *and* `target !=
///   Owner.Creature` — the **only** branch a living Osty can pass;
/// - kind 5 (IL_008d-IL_009c) returns `target.get_IsPlayer()`, which is
///   `Player != null` (`Creature::get_IsPlayer` `0x11d050`). An Osty is a pet
///   (`get_IsPet` `0x11d0c7` reads `PetOwner`) with no `Player` of its own;
/// - kind 1 (IL_009d-IL_00b4) accepts only `Owner.Creature`;
/// - every other kind falls through to `return false` (IL_00b5).
///
/// So a pet is a native pick only for kind 6 (AnyAlly). No supported body is
/// kind 6, which is why a live local Osty changes no supported potion's
/// target set; any kind outside the proven list, or an unread body, answers
/// false and keeps the refusal.
pub(crate) fn native_target_excludes_pets(potion: PotionId) -> bool {
    matches!(native_target_type(potion), Some(1 | 2 | 3 | 5 | 8))
}

/// Whether a live *teammate* makes this supported manual body unrepresentable.
///
/// This is a teammate gate, not a pet gate (#2497). A kind-5 (AnyPlayer) body
/// really can pick a teammate: `IsValidTarget`'s kind-5 branch (`0x8328c`
/// IL_008d-IL_009c) accepts any `IsPlayer` creature, and
/// [`MultiplayerAllyState`](crate::hot::MultiplayerAllyState) cannot carry the
/// pick, so these bodies stay out of every root with a live teammate key.
///
/// The set is unchanged from #2494/#2873 and is deliberately **not** widened
/// here: it was the complement of the deleted Python `_ALLY_SAFE_POTIONS`, and
/// its safe side still carries kind-5 bodies whose multiplayer pick this issue
/// did not re-read. What #2497 changed is that a live *local Osty* no longer
/// consults this set at all — see [`native_target_excludes_pets`] and
/// [`potion_target_roster_is_exact`].
///
/// Fairy in a Bottle is on the safe side because its manual use refuses as a
/// passive potion before any roster question, and its auto-consume on player
/// lethal is never a pick. [`validate_potion_action`] orders its two guards the
/// same way.
pub(crate) fn requires_solo_player_target(potion: PotionId) -> bool {
    is_supported(potion)
        && !matches!(
            potion,
            PotionId::BeetleJuice
                | PotionId::FirePotion
                | PotionId::FoulPotion
                | PotionId::GhostInAJar
                | PotionId::BoneBrew
                | PotionId::PotOfGhouls
                | PotionId::PotionOfDoom
                // #3229: TargetType 2 (`0xacf6d`), an enemy-only pick.
                | PotionId::PoisonPotion
                | PotionId::PowderedDemise
                | PotionId::VulnerablePotion
                | PotionId::WeakPotion
                | PotionId::ExplosiveAmpoule
                | PotionId::ShacklingPotion
                | PotionId::GigantificationPotion
                | PotionId::LuckyTonic
                | PotionId::MazalethsGift
                | PotionId::ShipInABottle
                | PotionId::SoldiersStew
                | PotionId::Ashwater
                | PotionId::GamblersBrew
                | PotionId::TouchOfInsanity
                | PotionId::DropletOfPrecognition
                | PotionId::LiquidMemories
                | PotionId::SwiftPotion
                | PotionId::Clarity
                | PotionId::CureAll
                | PotionId::SneckoOil
                | PotionId::BottledPotential
                | PotionId::AttackPotion
                | PotionId::SkillPotion
                | PotionId::PowerPotion
                | PotionId::ColorlessPotion
                | PotionId::OrobicAcid
                | PotionId::EntropicBrew
                | PotionId::FairyInABottle
                | PotionId::FruitJuice
        )
}

/// Whether the native pick a manual use of `potion` faces is representable.
///
/// Two independent hazards, each refusing only when it is real:
///
/// - a live teammate key with a [`requires_solo_player_target`] body. The
///   whole-root refusal in `admission` keeps that case out, and
///   [`MultiplayerAllyState`](crate::hot::MultiplayerAllyState) carries no
///   teammate Osty besides;
/// - a live local Osty with a body whose target kind could pick it. Since
///   #2497 this is read from the IL rather than from Python's list: only an
///   AnyAlly (kind 6) body can pick a pet, no supported body is one, so a live
///   Osty leaves every supported potion's legal action and its effect exactly
///   as in the solo roster. The effect half holds because every kind-5 body
///   routes its effect to the picked `target` or to `Owner.Creature`, which
///   in a teammate-free roster are the same creature — the player.
///
/// A false value does **not** refuse the root: `legal_actions_into` narrows
/// the action space and keeps playing, and a forged action refuses
/// atomically.
pub(crate) fn potion_target_roster_is_exact(state: &HotState, potion: PotionId) -> bool {
    (!requires_solo_player_target(potion) || state.multiplayer_ally_key == 0)
        && (native_target_excludes_pets(potion) || state.fanouts.pet().osty().is_none())
}

/// Prove one prospective Beetle Juice or Powdered Demise target write.
///
/// Held-belt admission passes `assume_unblocked = true` because Artifact can
/// be consumed by an earlier legal action before this potion is used. The
/// immediate action seam passes false: native Artifact then consumes the
/// whole application before any amount/order/body-specific reader runs.
pub(crate) fn target_application_is_exact(
    state: &HotState,
    potion: PotionId,
    target: usize,
    assume_unblocked: bool,
) -> Result<(), EngineRefusal> {
    let monster = state
        .monsters
        .get(target)
        .filter(|monster| monster.hp > 0)
        .ok_or(EngineRefusal::BadTarget(
            u8::try_from(target).unwrap_or(u8::MAX),
        ))?;
    let (power, amount, site) = match potion {
        PotionId::BeetleJuice => (PowerId::Shrink, 4, "Beetle Juice Shrink amount"),
        PotionId::PowderedDemise => (PowerId::Demise, 9, "Powdered Demise amount"),
        _ => return Ok(()),
    };
    if !assume_unblocked && monster.powers.value(PowerId::Artifact) > 0 {
        return Ok(());
    }
    super::damage::misery_order_projection_is_exact(monster)?;
    let updated = monster
        .powers
        .value(power)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    if power == PowerId::Shrink && updated >= 999_999_999 {
        return Err(EngineRefusal::CounterOverflow(site));
    }
    // A Waterfall Giant target is not refused here (#3428). The application
    // itself is the ordinary one; the only Waterfall-specific question is a
    // later lethal Demise tick reviving the owner, which the side-end Demise
    // anchor decides exactly (`turn::waterfall_demise_revival_suffix_is_empty`).
    Ok(())
}

/// Close a held belt over every serial ordering of its targetable Part-B
/// debuffs. Artifact cannot reduce these counts: another legal action can
/// consume it before any selected potion slot is used.
pub(crate) fn held_target_applications_are_exact(
    state: &HotState,
    target: usize,
    beetle_juice_count: usize,
    powdered_demise_count: usize,
) -> Result<(), EngineRefusal> {
    if beetle_juice_count > 0 {
        target_application_is_exact(state, PotionId::BeetleJuice, target, true)?;
        let old = i64::from(state.monsters[target].powers.value(PowerId::Shrink));
        let count = i64::try_from(beetle_juice_count)
            .map_err(|_| EngineRefusal::CounterOverflow("held Beetle Juice count"))?;
        let updated = old
            .checked_add(
                count
                    .checked_mul(4)
                    .ok_or(EngineRefusal::CounterOverflow("held Beetle Juice count"))?,
            )
            .ok_or(EngineRefusal::CounterOverflow("held Beetle Juice count"))?;
        if updated >= 999_999_999 {
            return Err(EngineRefusal::CounterOverflow(
                "held Beetle Juice Shrink amount",
            ));
        }
    }
    if powdered_demise_count > 0 {
        target_application_is_exact(state, PotionId::PowderedDemise, target, true)?;
        let old = i64::from(state.monsters[target].powers.value(PowerId::Demise));
        let count = i64::try_from(powdered_demise_count)
            .map_err(|_| EngineRefusal::CounterOverflow("held Powdered Demise count"))?;
        let updated = old
            .checked_add(
                count
                    .checked_mul(9)
                    .ok_or(EngineRefusal::CounterOverflow("held Powdered Demise count"))?,
            )
            .ok_or(EngineRefusal::CounterOverflow("held Powdered Demise count"))?;
        if updated > i64::from(i32::MAX) {
            return Err(EngineRefusal::CounterOverflow(
                "held Powdered Demise amount",
            ));
        }
    }
    Ok(())
}

fn checked_power_add(
    state: &mut HotState,
    power_id: PowerId,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    let old = state.powers.value(power_id);
    let updated = old
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("player potion power"))?;
    if old <= 0
        && crate::hot::AfterSideTurnStartToken::from_power(power_id).is_some()
        && !state.fanouts.register_after_side_turn_start(power_id)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-side-turn-start listener order",
        ));
    }
    super::turn::prepare_after_side_turn_end_scalar_write(state, power_id, old, updated)?;
    state.powers.set(power_id, SlotWire::Int, updated);
    super::damage::note_power(events, Subject::Player, power_id, updated);
    Ok(())
}

fn checked_cold_add(current: i32, amount: i32, site: &'static str) -> Result<i32, EngineRefusal> {
    current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(site))
}

fn apply_belt_buckle_after_use(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over
        || !state.fanouts.potion_belt_buckle()
        || state.fanouts.potion_belt_buckle_applied()
        || state.fanouts.potion_slots().iter().any(Option::is_some)
    {
        return Ok(());
    }
    // BeltBuckle.ApplyDexterity RVA 0x90940 sets the private latch before
    // awaiting PowerCmd.Apply<DexterityPower>(2).
    state.fanouts.set_potion_belt_buckle_applied(true);
    super::damage::apply_signed_player_stat(state, PowerId::Dexterity, 2, events)
}

/// Consume the first physical Fairy in a Bottle at the player-lethal hook.
///
/// FairyInABottle/OnLethalDamagePrevented (current v0.111.0) removes its own
/// belt model before healing 30% max HP (minimum one), then awaits the same
/// AfterPotionUsed subscriber walk as a manual wrapper.  The caller invokes
/// this before owner-death cleanup; ForceKill callers deliberately bypass it.
pub(crate) fn consume_first_fairy_after_lethal(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let Some(slot) = state
        .fanouts
        .potion_slots()
        .iter()
        .position(|potion| *potion == Some(PotionId::FairyInABottle))
    else {
        return Ok(false);
    };
    if state.fanouts.remove_potion_at(slot) != Some(PotionId::FairyInABottle) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let heal = (i64::from(state.max_hp) * 30 / 100).max(1);
    let hp_before = state.hp;
    state.hp = i64::from(state.max_hp)
        .min(i64::from(state.hp.max(0)) + heal)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("Fairy heal"))?;
    // Fairy's heal is a `CreatureCmd.Heal` (`0x3eb4b0`), whose
    // `AfterCurrentHpChanged` reaches Red Skull (#3044).
    super::damage::red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    apply_belt_buckle_after_use(state, events)?;
    Ok(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FairyWrapperResult {
    Absent,
    Finished,
    Suspended,
}

/// Enter Fairy's real `PotionModel.OnUseWrapper` from ShouldDie.
///
/// The catalog-bearing active-card caller retains its CardPlay below this
/// wrapper. That effect depth suppresses Unceasing Top exactly, so this public
/// path returns only after AfterPotionUsed and CheckForEmptyHand complete. The
/// result enum keeps the shared finish contract explicit even though the
/// admitted Fairy callsite cannot publish `Suspended`.
pub(crate) fn begin_fairy_wrapper_after_lethal(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<FairyWrapperResult, EngineRefusal> {
    let Some(slot) = state
        .fanouts
        .potion_slots()
        .iter()
        .position(|potion| *potion == Some(PotionId::FairyInABottle))
    else {
        return Ok(FairyWrapperResult::Absent);
    };
    state
        .frames
        .push_potion_finish(&PotionFinishRecord {
            name: PotionId::FairyInABottle,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if state.fanouts.remove_potion_at(slot) != Some(PotionId::FairyInABottle) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let heal = (i64::from(state.max_hp) * 30 / 100).max(1);
    let hp_before = state.hp;
    state.hp = i64::from(state.max_hp)
        .min(i64::from(state.hp.max(0)) + heal)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("Fairy heal"))?;
    // Fairy's heal is a `CreatureCmd.Heal` (`0x3eb4b0`), whose
    // `AfterCurrentHpChanged` reaches Red Skull (#3044).
    super::damage::red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    finish(state, catalog, PotionId::FairyInABottle, events)?;
    Ok(if state.pending.is_some() {
        FairyWrapperResult::Suspended
    } else {
        FairyWrapperResult::Finished
    })
}

/// `BeltBuckle.AfterPotionProcured` coroutine body (RVA `0x9081c`) and its
/// generated state-machine `MoveNext` (RVA `0x31f164`; the
/// `RemoveDexterity` continuation begins at `0x31f528`).
/// Procurement calls this after the physical slot has been filled: an active
/// empty-belt bonus is removed exactly once and the private latch clears.
#[allow(dead_code)] // Part B models the hook; potion procurement lands in Part E.
pub(crate) fn belt_buckle_after_procured(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.fanouts.potion_belt_buckle() || !state.fanouts.potion_belt_buckle_applied() {
        return Ok(());
    }
    state.fanouts.set_potion_belt_buckle_applied(false);
    // BeltBuckle/<AfterPotionProcured>d__9::MoveNext RVA 0x31f164 clears its
    // private latch before the RemoveDexterity continuation at RVA 0x31f528
    // awaits the separately ending-gated Dexterity application.
    // Entropic procurement itself has no ending gate, so a terminal-window
    // insertion preserves this clear while suppressing the -2 write.
    if state.history.over {
        Ok(())
    } else {
        super::damage::apply_signed_player_stat(state, PowerId::Dexterity, -2, events)
    }
}

/// Try the native first-null-slot procurement transaction, then notify Belt
/// Buckle only after insertion succeeds. A full belt or Sozu leaves both the
/// belt and the empty-belt latch untouched.
#[allow(dead_code)] // Part E will connect the generation/procurement callers.
pub(crate) fn procure_potion(
    state: &mut HotState,
    potion: PotionId,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if state.fanouts.potion_sozu() {
        return Ok(false);
    }
    if !state.fanouts.potion_slots().iter().any(Option::is_none) {
        return Ok(false);
    }
    let mut next = state.clone();
    let mut next_events = Vec::new();
    if !next.fanouts.insert_potion_first_empty(potion) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    belt_buckle_after_procured(&mut next, &mut next_events)?;
    *state = next;
    events.extend(next_events);
    Ok(true)
}

fn finish(
    state: &mut HotState,
    catalog: &Catalog,
    potion: PotionId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    state
        .frames
        .pop_top_potion_finish()
        .filter(|record| {
            record.name == potion
                && record.stage == PotionFinishStage::Effect
                && record.body_stage == PotionBodyStage::Synchronous
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    // PotionModel.OnUseWrapper RVA 0x83344 / MoveNext 0x31dd20 invokes the
    // owner's AfterPotionUsed subscribers before CheckForEmptyHand. This is
    // the current native ordering. The Python oracle carries the same
    // corrected order. Belt Buckle and Reptile Trinket commute; Kaiser Crab
    // remains boundary-refused.
    state
        .frames
        .push_potion_finish(&PotionFinishRecord {
            name: potion,
            // Python advances the wrapper cursor before awaiting the
            // acquisition-ordered AfterPotionUsed walk.
            stage: PotionFinishStage::AfterTop,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    apply_belt_buckle_after_use(state, events)?;
    if !state.history.over && catalog.hooks().owns(RelicId::RelicReptileTrinket) {
        super::allies::gain_temp_strength(state, 0, 3, events)?;
        crate::coverage::record_relic(RelicId::RelicReptileTrinket);
    }
    if state.pending.is_some()
        || !matches!(state.frames.top(), Some(Frame::PotionFinish { record })
            if state.frames.potion_finish(record).is_some_and(|view|
                view.name == potion && view.stage == PotionFinishStage::AfterTop))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // CombatManager.CheckForEmptyHand follows AfterPotionUsed. A nested
    // potion/card effect suppresses Top; otherwise its Draw(1) is owned by
    // this AfterTop cursor and may park through the shared resumable Draw
    // grammar without replaying Belt Buckle or the potion body.
    let nested_effect = state.frames.as_slice().iter().rev().skip(1).any(|frame| {
        matches!(*frame, Frame::PotionFinish { record }
            if state.frames.potion_finish(record).is_some_and(|view|
                view.stage == PotionFinishStage::Effect))
            || matches!(frame, Frame::CardPlay { .. })
    });
    if state.fanouts.unceasing_top()
        && !state.history.over
        && state.hp > 0
        && state.piles.get(PileId::Hand).is_empty()
        && matches!(state.player_phase, 2..=4)
        && !nested_effect
    {
        state
            .frames
            .replace_top_potion_finish(&PotionFinishRecord {
                name: potion,
                stage: PotionFinishStage::AfterTop,
                body_stage: PotionBodyStage::AfterChild,
                current_uid: None,
                aux: 0,
                candidates: Vec::new(),
                generation_options: Vec::new(),
            })
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        if super::draw::draw_cards_for_potion(
            state,
            catalog,
            1,
            DrawCaller::UnceasingTopPotion,
            events,
        )? == super::draw::PotionDrawResult::Suspended
        {
            return Ok(());
        }
    }
    state
        .frames
        .pop_top_potion_finish()
        .filter(|record| record.name == potion && record.stage == PotionFinishStage::AfterTop)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    Ok(())
}

fn wait_for_draw(state: &mut HotState, potion: PotionId) -> Result<(), EngineRefusal> {
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: potion,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::AfterChild,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

fn restore_after_draw(state: &mut HotState, potion: PotionId) -> Result<(), EngineRefusal> {
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: potion,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

/// Continue Distilled Chaos's fixed three-slot gather. The partial prefix is
/// frozen in a DrawPileFlip batch before any child body begins; a blocking
/// AfterShuffle listener parks an ordinary Draw cursor above that batch and
/// returns here without consuming a Draw card.
pub(crate) fn resume_distilled_gather(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    resume_distilled_gather_inner(state, catalog, events, false)
}

pub(crate) fn resume_distilled_gather_after_shuffle(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    resume_distilled_gather_inner(state, catalog, events, true)
}

fn resume_distilled_gather_inner(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
    mut after_shuffle: bool,
) -> Result<(), EngineRefusal> {
    loop {
        let mut record = match state.frames.top() {
            Some(Frame::FrozenAutoBatch { record }) => state
                .frames
                .frozen_auto_batch(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?
                .to_owned(),
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        if record.gather_target != 3
            || record.cursor > u32::from(record.gather_target)
            || record.entries.len() > usize::try_from(record.cursor).unwrap_or(usize::MAX)
            || record.force_exhaust
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        if record.entries.len() >= 3 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }

        if state.history.over || state.hp <= 0 {
            return finish_distilled_gather(state, catalog, record, events);
        }
        if !after_shuffle {
            if record.cursor >= u32::from(record.gather_target) {
                return finish_distilled_gather(state, catalog, record, events);
            }
            record.cursor = record
                .cursor
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Distilled gather attempt"))?;
            state
                .frames
                .replace_top_frozen_auto_batch(&record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            if state.piles.get(PileId::Draw).is_empty()
                && !state.piles.get(PileId::Discard).is_empty()
                && super::draw::reshuffle_for_distilled_gather(
                    state,
                    catalog,
                    record.entries.len(),
                    events,
                )?
            {
                return Ok(());
            }
        }
        after_shuffle = false;
        if !state.piles.get(PileId::Draw).is_empty() {
            let card = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
            state.piles.get_mut(PileId::Play).make_mut().push(card);
            record
                .entries
                .push(crate::hot::FrozenAutoBatchEntry::from_current(
                    card,
                    &state.card_states,
                ));
        }
        if record.cursor >= u32::from(record.gather_target) {
            return finish_distilled_gather(state, catalog, record, events);
        }
        state
            .frames
            .replace_top_frozen_auto_batch(&record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    }
}

fn finish_distilled_gather(
    state: &mut HotState,
    catalog: &Catalog,
    mut record: crate::hot::FrozenAutoBatchRecord,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if record.entries.is_empty() {
        state
            .frames
            .pop_top_frozen_auto_batch()
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        return resume_after_distilled_batch(state, catalog, events);
    }
    record.gather_target = 0;
    record.cursor = 0;
    state
        .frames
        .replace_top_frozen_auto_batch(&record)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    super::play::drive_distilled_batch(state, catalog, events)
}

pub(crate) fn resume_after_distilled_batch(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let finish_record = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?
            .to_owned(),
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if finish_record.name != PotionId::DistilledChaos
        || finish_record.stage != PotionFinishStage::Effect
        || finish_record.body_stage != PotionBodyStage::AfterChild
        || finish_record.current_uid.is_some()
        || finish_record.aux != 0
        || !finish_record.candidates.is_empty()
        || !finish_record.generation_options.is_empty()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    restore_after_draw(state, PotionId::DistilledChaos)?;
    finish(state, catalog, PotionId::DistilledChaos, events)
}

fn begin_distilled_chaos(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || state.hp <= 0 {
        return finish(state, catalog, PotionId::DistilledChaos, events);
    }
    super::cards::normalize_card_identities(state)?;
    state.exact_piles = true;
    wait_for_draw(state, PotionId::DistilledChaos)?;
    state
        .frames
        .push_frozen_auto_batch(&crate::hot::FrozenAutoBatchRecord {
            entries: Vec::new(),
            cursor: 0,
            gather_target: 3,
            force_exhaust: false,
            source: crate::hot::FrozenAutoBatchSource::DrawPileFlip,
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    resume_distilled_gather(state, catalog, events)
}

pub(crate) fn resume_bottled_after_shuffle(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let finish_record = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?
            .to_owned(),
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if finish_record.name != PotionId::BottledPotential
        || finish_record.stage != PotionFinishStage::Effect
        || finish_record.body_stage != PotionBodyStage::AfterShuffle
        || finish_record.current_uid.is_some()
        || finish_record.aux != 0
        || finish_record.candidates.is_empty()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    restore_after_draw(state, PotionId::BottledPotential)?;
    if !state.history.over {
        wait_for_draw(state, PotionId::BottledPotential)?;
        if super::draw::draw_cards_for_potion(
            state,
            catalog,
            5,
            DrawCaller::PotionEpilogue,
            events,
        )? == super::draw::PotionDrawResult::Suspended
        {
            return Ok(());
        }
        restore_after_draw(state, PotionId::BottledPotential)?;
    }
    finish(state, catalog, PotionId::BottledPotential, events)
}

fn set_gambler_sly_cursor(
    state: &mut HotState,
    sly: Vec<crate::hot::HotCard>,
) -> Result<(), EngineRefusal> {
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: PotionId::GamblersBrew,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::AfterChild,
            current_uid: None,
            aux: 0,
            candidates: sly,
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

pub(crate) fn gambler_sly_cursor_is_exact(
    state: &HotState,
    catalog: &Catalog,
    finish: crate::hot::PotionFinishView<'_>,
) -> bool {
    finish.name == PotionId::GamblersBrew
        && finish.stage == PotionFinishStage::Effect
        && finish.body_stage == PotionBodyStage::AfterChild
        && finish.candidates().all(|captured| {
            super::play::unique_live_card_location(state, captured.uid)
                .ok()
                .flatten()
                .is_some_and(|(pile, index)| {
                    let live = state.piles.get(pile).as_slice()[index];
                    matches!(pile, PileId::Hand | PileId::Draw | PileId::Discard)
                        && catalog
                            .spec(live.atom)
                            .is_some_and(|spec| state.card_states.get(live.uid).is_sly(spec.sly))
                })
        })
}

fn run_gambler_sly_tail(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let finish_view = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if !gambler_sly_cursor_is_exact(state, catalog, finish_view) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // DiscardAndDraw is ending-gated before its Sly AutoPlay suffix.  The
    // frozen cursor is still consumed below so the potion wrapper can finish
    // exactly once, but no selected Sly body begins after lethal Draw work.
    if state.hp <= 0 || state.history.over {
        restore_after_draw(state, PotionId::GamblersBrew)?;
        finish(state, catalog, PotionId::GamblersBrew, events)?;
        return Ok(true);
    }
    let mut live = Vec::with_capacity(finish_view.candidates().len());
    for captured in finish_view.candidates() {
        let (pile, index) = super::play::unique_live_card_location(state, captured.uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        live.push(state.piles.get(pile).as_slice()[index]);
    }
    if !live.is_empty() {
        // The paired Draw owns only the immutable ordered UID cursor.  At the
        // Sly tail seam FrozenAutoBatch captures the complete current card
        // payload/state, so clear the parent UID cursor before installing the
        // child owner.  This also makes the two ownership stages disjoint.
        set_gambler_sly_cursor(state, Vec::new())?;
        super::play::autoplay_sly_discard_batch(state, catalog, &live, events)?;
        if state.pending.is_some()
            || !matches!(state.frames.top(), Some(Frame::PotionFinish { .. }))
        {
            return Ok(false);
        }
    }
    restore_after_draw(state, PotionId::GamblersBrew)?;
    finish(state, catalog, PotionId::GamblersBrew, events)?;
    Ok(true)
}

pub(crate) fn resume_after_gambler_sly_batch(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let finish_view = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    // The completed FrozenAutoBatch was authenticated before it was popped.
    // Do not call `run_gambler_sly_tail` here: that is the start-of-tail
    // entry point and would replay the entire immutable Sly sequence.
    if finish_view.name != PotionId::GamblersBrew
        || finish_view.stage != PotionFinishStage::Effect
        || finish_view.body_stage != PotionBodyStage::AfterChild
        || finish_view.candidates().next().is_some()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    restore_after_draw(state, PotionId::GamblersBrew)?;
    finish(state, catalog, PotionId::GamblersBrew, events)
}

pub(crate) fn set_ashwater_exhaust_cursor(
    state: &mut HotState,
    current_uid: u32,
    midnight: Vec<crate::hot::HotCard>,
    remaining: Vec<crate::hot::HotCard>,
    fnp_completed: bool,
) -> Result<(), EngineRefusal> {
    let aux = u32::try_from(midnight.len())
        .map_err(|_| EngineRefusal::CounterOverflow("Ashwater Midnight snapshot"))?;
    let mut candidates = midnight;
    candidates.extend(remaining);
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: PotionId::Ashwater,
            stage: PotionFinishStage::Effect,
            body_stage: if fnp_completed {
                PotionBodyStage::AshwaterAfterFnp
            } else {
                PotionBodyStage::AfterChild
            },
            current_uid: Some(current_uid),
            aux,
            candidates,
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

pub(crate) fn ashwater_exhaust_cursor_is_exact(
    state: &HotState,
    catalog: &Catalog,
    finish_view: crate::hot::PotionFinishView<'_>,
) -> bool {
    let Some(current_uid) = finish_view.current_uid else {
        return false;
    };
    let candidates = finish_view.candidates().collect::<Vec<_>>();
    let Ok(split) = usize::try_from(finish_view.aux) else {
        return false;
    };
    if finish_view.name != PotionId::Ashwater
        || finish_view.stage != PotionFinishStage::Effect
        || !matches!(
            finish_view.body_stage,
            PotionBodyStage::AfterChild | PotionBodyStage::AshwaterAfterFnp
        )
        || split > candidates.len()
        || !matches!(
            super::play::unique_live_card_location(state, current_uid),
            Ok(Some((PileId::Exhaust, _)))
        )
    {
        return false;
    }
    let order = state.fanouts.after_card_exhausted_order();
    let dark = order
        .iter()
        .position(|power| *power == PowerId::DarkEmbrace);
    let fnp = order.iter().position(|power| *power == PowerId::FeelNoPain);
    let fnp_completed_is_required = state.powers.value(PowerId::FeelNoPain) > 0
        && fnp.is_some()
        && (dark.is_none() || fnp < dark);
    if (finish_view.body_stage == PotionBodyStage::AshwaterAfterFnp) != fnp_completed_is_required {
        return false;
    }
    let (midnight, remaining) = candidates.split_at(split);
    midnight.iter().all(|captured| {
        super::play::unique_live_card_location(state, captured.uid)
            .ok()
            .flatten()
            .is_some_and(|(pile, index)| {
                let live = state.piles.get(pile).as_slice()[index];
                catalog
                    .spec(live.atom)
                    .is_some_and(|spec| spec.identity.id == crate::ids::CardId::Midnight)
            })
    }) && remaining.iter().all(|captured| {
        super::play::unique_live_card_location(state, captured.uid)
            .ok()
            .flatten()
            .is_some_and(|(pile, _)| pile == PileId::Hand)
    })
}

pub(crate) fn finish_ashwater_current_after_dark(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<Vec<crate::hot::HotCard>, EngineRefusal> {
    let finish_view = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if !ashwater_exhaust_cursor_is_exact(state, catalog, finish_view) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let current_uid = finish_view
        .current_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let aux = finish_view.aux;
    let candidates = finish_view.candidates().collect::<Vec<_>>();
    let split = usize::try_from(aux).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    let (midnight, remaining) = candidates.split_at(split);

    if finish_view.body_stage == PotionBodyStage::AfterChild
        && !state.history.over
        && state.powers.value(PowerId::FeelNoPain) > 0
    {
        super::orbs::gain_flat_block(
            state,
            catalog,
            i64::from(state.powers.value(PowerId::FeelNoPain)),
            events,
        )?;
    }
    let current = super::play::unique_live_card_location(state, current_uid)?
        .filter(|(pile, _)| *pile == PileId::Exhaust)
        .map(|(pile, index)| state.piles.get(pile).as_slice()[index])
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let current_spec = catalog
        .spec(current.atom)
        .ok_or(EngineRefusal::UnknownAtom(current.atom))?;
    if current_spec.identity.id == crate::ids::CardId::DrumOfBattle && !state.history.over {
        let (play_count, energy) = super::draw::drum_of_battle_plan(
            state,
            current_spec,
            current,
            current_spec.identity.upgrade,
        )?;
        if state.powers.value(PowerId::Burst) > 0 {
            let current = state.powers.value(PowerId::Burst);
            let updated = current - 1;
            super::turn::prepare_after_side_turn_end_scalar_write(
                state,
                PowerId::Burst,
                current,
                updated,
            )?;
            state.powers.set(PowerId::Burst, SlotWire::Int, updated);
        }
        if state.fanouts.duplication() > 0 {
            let updated = state.fanouts.duplication() - 1;
            super::turn::prepare_after_side_turn_end_singleton_write(
                state,
                crate::hot::AfterSideTurnEndPowerToken::Duplication,
                true,
                updated != 0,
            )?;
            state.fanouts.set_duplication(updated);
        }
        for _ in 0..play_count {
            if state.history.over {
                break;
            }
            state.energy = state
                .energy
                .checked_add(energy)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
        }
    }
    // Card-pile listeners use the hook-time UID snapshot and re-resolve each
    // object after the awaited Draw. Missing objects are skipped and identity
    // is not rechecked: the captured physical listener survives legitimate
    // intervening mutation of that same UID.
    super::draw::apply_midnight_snapshot(state, catalog, midnight)?;
    Ok(remaining.to_vec())
}

/// Runs Ashwater's frozen physical-card Exhaust sequence in native listener
/// order. Authority: macOS arm64 v0.111.0/41cef1ea `sts2.dll`, SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// Ashwater `CanonicalVars`/`OnUse.MoveNext` are `0xab9cc`/`0x34bb90`;
/// `CardCmd.Exhaust.MoveNext` `0x4295fc` adds the physical card and records
/// history before `Hook.AfterCardExhausted` `0x415a14` snapshots listeners.
/// `CombatState.IterateHookListeners.MoveNext` `0x441190` yields player powers
/// in first-application order. Dark Embrace `0x381f4c` awaits Draw
/// (`DrawInternal` `0x42cfc8`); Feel No Pain `0x24cc68`/`0x3809ec` awaits
/// GainBlock `0x3eaec0`/`0x11d64c`, including Juggernaut
/// `0x24ebf8`/`0x384218`. Hellraiser's potentially selecting early-draw body
/// is `0x385778`. Therefore an FNP-first walk must persist its completed block
/// before entering Dark's resumable Draw, while a Dark-first walk applies FNP
/// only after that Draw returns and the listener remains live.
pub(crate) fn run_ashwater_exhaust_sequence(
    state: &mut HotState,
    catalog: &Catalog,
    selected: Vec<crate::hot::HotCard>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    restore_after_draw(state, PotionId::Ashwater)?;
    for (position, captured) in selected.iter().copied().enumerate() {
        if state.history.over {
            break;
        }
        let (pile, index) = super::play::unique_live_card_location(state, captured.uid)?
            .filter(|(pile, _)| *pile == PileId::Hand)
            .ok_or(EngineRefusal::FrozenCardVanished {
                uid: captured.uid,
                pile: PileId::Hand,
            })?;
        let moved = state.piles.get_mut(pile).make_mut().remove(index);
        state.piles.get_mut(PileId::Exhaust).make_mut().push(moved);
        state.history.owner_card_exhausted_this_turn = true;
        state.history.owner_cards_exhausted_combat = state
            .history
            .owner_cards_exhausted_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_cards_exhausted_combat",
            ))?;
        events.push(Event::CardResolved {
            uid: moved.uid,
            pile: PileId::Exhaust,
        });
        // Python/native `_ALL_CARD_PILES` is semantic, not enum order:
        // Hand, Draw, Discard, Exhaust, Play. Preserve that exact physical
        // snapshot order; `PileId::ALL` is alphabetical.
        let midnight = super::draw::midnight_snapshot(state, catalog);
        let remaining = selected[position + 1..].to_vec();
        set_ashwater_exhaust_cursor(state, moved.uid, midnight.clone(), remaining.clone(), false)?;

        let order = state.fanouts.after_card_exhausted_order();
        let dark = order
            .iter()
            .position(|power| *power == PowerId::DarkEmbrace);
        let fnp = order.iter().position(|power| *power == PowerId::FeelNoPain);
        let fnp_before_dark = fnp.is_some() && (dark.is_none() || fnp < dark);
        if fnp_before_dark && !state.history.over && state.powers.value(PowerId::FeelNoPain) > 0 {
            super::orbs::gain_flat_block(
                state,
                catalog,
                i64::from(state.powers.value(PowerId::FeelNoPain)),
                events,
            )?;
            set_ashwater_exhaust_cursor(state, moved.uid, midnight, remaining, true)?;
        }
        let draws: usize = state
            .powers
            .value(PowerId::DarkEmbrace)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("dark embrace draw"))?;
        if draws > 0
            && super::draw::draw_cards_for_potion(
                state,
                catalog,
                draws,
                DrawCaller::AshwaterExhaust,
                events,
            )? == super::draw::PotionDrawResult::Suspended
        {
            return Ok(());
        }
        let remaining = finish_ashwater_current_after_dark(state, catalog, events)?;
        restore_after_draw(state, PotionId::Ashwater)?;
        if remaining.len() != selected.len() - position - 1 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    finish(state, catalog, PotionId::Ashwater, events)
}

/// Glowwater Potion: exhaust the frozen Hand snapshot, then `Draw(10)`.
///
/// Authority: macOS arm64 `sts2.dll` v0.111.0/41cef1ea, SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `GlowwaterPotion::OnUse` is `0xacb44` and
/// `GlowwaterPotion/<OnUse>d__10::MoveNext` is `0x34e764`. The body snapshots
/// `PileTypeExtensions::GetPile(2 /* Hand */, target.Player).Cards.ToList()`
/// (IL_0060-IL_0076), awaits `CardCmd::Exhaust(choiceContext, card, false,
/// false)` once per snapshot element (IL_009e), and only then awaits
/// `CardPileCmd::Draw(DynamicVars.Cards.BaseValue, targetPlayer, false)`
/// (IL_014a). `get_CanonicalVars` (`0xacb35`) is `CardsVar(10)`.
///
/// The IL loop carries no combat-over test of its own — each `CardCmd::Exhaust`
/// gates internally, which is what Python's `if s.over: break` models and what
/// [`super::draw::card_exhausted_with_owner`] already reproduces.
///
/// Exhausting the WHOLE hand is unambiguous — every copy leaves — so unlike
/// Ashwater's chosen subset there is no identical-copy flush-order choice and
/// no `exact_piles` refusal is owed. Exhausted cards never re-enter Draw, so
/// the tail is the ordinary reshuffling Draw.
fn begin_glowwater(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // Legacy cards share one placeholder uid, and the snapshot below freezes
    // physical identities across resumable children — allocate first, exactly
    // as Soldier's Stew and Blessing of the Forge do.
    super::cards::normalize_card_identities(state)?;
    let snapshot = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    continue_glowwater_exhaust(state, catalog, &snapshot, events)
}

/// One pass over the frozen Glowwater snapshot tail.
///
/// Mirrors `continue_second_wind`: a live Dark Embrace owner or a suspended
/// child leaves the `AfterCardExhaustedPower` frame in charge of the rest,
/// and [`resume_glowwater_after_exhaust`] re-enters here with what is left.
fn continue_glowwater_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    remaining: &[u32],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    for (cursor, uid) in remaining.iter().copied().enumerate() {
        // `CardCmd::Exhaust` is gated, so a combat that ended mid-walk stops
        // the remaining Exhausts — and the Draw tail below with them.
        if state.history.over {
            break;
        }
        let hand = state.piles.get(PileId::Hand).as_slice();
        let Some(index) = hand.iter().position(|live| live.uid == uid) else {
            // Nothing in this walk relocates a snapshot card out of Hand, so
            // reaching this is a construction bug rather than a modeling gap.
            return Err(EngineRefusal::CardNotInHand(uid));
        };
        let card = hand[index];
        super::draw::repair_card_play_after_physical_move(
            state,
            card,
            PileId::Hand,
            PileId::Exhaust,
        )?;
        let removed = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
            crate::hot::AfterCardExhaustedReturnKind::Glowwater,
        );
        continuation.remaining = remaining[cursor + 1..].to_vec();
        let owner_live = super::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog);
        let result =
            super::draw::card_exhausted_with_owner(state, catalog, removed, continuation, events)?;
        if owner_live || result == super::draw::CardExhaustedResult::Suspended {
            return Ok(());
        }
    }
    glowwater_draw_tail(state, catalog, events)
}

/// The `CardPileCmd::Draw(10)` that follows the whole-hand Exhaust walk.
fn glowwater_draw_tail(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    wait_for_draw(state, PotionId::GlowwaterPotion)?;
    if super::draw::draw_cards_for_potion(state, catalog, 10, DrawCaller::PotionEpilogue, events)?
        == super::draw::PotionDrawResult::Suspended
    {
        return Ok(());
    }
    restore_after_draw(state, PotionId::GlowwaterPotion)?;
    finish(state, catalog, PotionId::GlowwaterPotion, events)
}

pub(crate) fn resume_glowwater_after_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let completed = state
        .frames
        .pop_top_after_card_exhausted_power()
        .filter(|popped| {
            popped.return_kind == crate::hot::AfterCardExhaustedReturnKind::Glowwater
                && popped.card_uid == record.card_uid
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    continue_glowwater_exhaust(state, catalog, &completed.remaining, events)
}

pub(crate) fn resume_ashwater_after_dark(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let remaining = finish_ashwater_current_after_dark(state, catalog, events)?;
    run_ashwater_exhaust_sequence(state, catalog, remaining, events)
}

fn apply_snecko_oil_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    // The suffix always snapshots physical CardModel identities, including
    // the zero-draw/full-Hand path.  Allocate every legacy sibling before
    // the first CombatEnergyCosts roll so equal payloads receive distinct
    // rows and any allocator refusal remains inside the whole-action COW.
    super::cards::normalize_card_identities(state)?;
    let hand = state.piles.get(PileId::Hand).as_slice().to_vec();
    let live = state.rng.get(RngStream::EnergyCosts);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    for card in hand {
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if spec.x_cost || spec.cost < 0 {
            continue;
        }
        let rolled = i64::from(
            rng.next_bounded(4)
                .map_err(|_| EngineRefusal::CounterOverflow("Snecko Oil cost"))?,
        );
        state
            .card_states
            .set_snecko_oil_cost(card.uid, rolled)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        for pile in PileId::ALL {
            if let Some(live) = state
                .piles
                .get_mut(pile)
                .make_mut()
                .iter_mut()
                .find(|live| live.uid == card.uid)
            {
                live.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                break;
            }
        }
    }
    state.rng.set(
        RngStream::EnergyCosts,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(())
}

/// Complete the exact potion suffix after one resumed Draw command.
pub(crate) fn resume_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if caller == DrawCaller::UnceasingTopPotion {
        let record = state
            .frames
            .pop_top_potion_finish()
            .filter(|record| {
                record.stage == PotionFinishStage::AfterTop
                    && record.body_stage == PotionBodyStage::AfterChild
                    && record.current_uid.is_none()
                    && record.candidates.is_empty()
                    && record.generation_options.is_empty()
            })
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let _ = record.name;
        return Ok(());
    }
    let potion = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => {
            let finish = state
                .frames
                .potion_finish(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            if finish.stage != PotionFinishStage::Effect
                || !matches!(
                    finish.body_stage,
                    PotionBodyStage::AfterChild | PotionBodyStage::AshwaterAfterFnp
                )
                || finish.body_stage == PotionBodyStage::AshwaterAfterFnp
                    && finish.name != PotionId::Ashwater
            {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            finish.name
        }
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    match (caller, potion) {
        (
            DrawCaller::PotionEpilogue,
            PotionId::SwiftPotion
            | PotionId::CureAll
            | PotionId::BottledPotential
            // Glowwater's `Draw(10)` tail, after its Exhaust walk finished.
            | PotionId::GlowwaterPotion,
        ) => {}
        (DrawCaller::ClarityPotion, PotionId::Clarity) => {
            if !state.history.over {
                super::damage::null_applier_power_amount_changed_is_exact(state)?;
                let clarity = checked_cold_add(state.fanouts.clarity(), 3, "clarity")?;
                assert!(state.fanouts.set_clarity(clarity));
            }
        }
        (DrawCaller::SneckoOil, PotionId::SneckoOil) => {
            apply_snecko_oil_after_draw(state, catalog)?;
        }
        (DrawCaller::SlyAfterDraw, PotionId::GamblersBrew) => {
            run_gambler_sly_tail(state, catalog, events)?;
            return Ok(());
        }
        (DrawCaller::AshwaterExhaust, PotionId::Ashwater) => {
            resume_ashwater_after_dark(state, catalog, events)?;
            return Ok(());
        }
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    }
    restore_after_draw(state, potion)?;
    finish(state, catalog, potion, events)
}

fn debuff(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    super::damage::apply_power_monster_debuff_with_catalog(
        state, catalog, target, power, token, amount, events,
    )
}

fn soldiers_stew(state: &mut HotState, catalog: &Catalog) -> Result<(), EngineRefusal> {
    let mut has_strike = false;
    for pile in PileId::ALL {
        for card in state.piles.get(pile).as_slice() {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            has_strike |= spec.strike_tag;
        }
    }
    if !has_strike {
        return Ok(());
    }

    // Python `_soldiers_stew_exact` normalizes every live card only after its
    // frozen AllCards snapshot proves at least one Strike exists. Legacy
    // cards all carry the same placeholder uid in hot state, so allocation
    // must precede the exact physical-identity walk below.
    super::cards::normalize_card_identities(state)?;

    let mut grants = Vec::new();
    let mut seen = Vec::new();
    for pile in PileId::ALL {
        for (index, card) in state.piles.get(pile).as_slice().iter().enumerate() {
            if seen.contains(&card.uid) {
                return Err(EngineRefusal::MalformedArgs(
                    "Soldier's Stew duplicate physical uid",
                ));
            }
            seen.push(card.uid);
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.strike_tag {
                let next = state
                    .card_states
                    .get(card.uid)
                    .base_replay_count()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow(
                        "Soldier's Stew BaseReplayCount",
                    ))?;
                grants.push((pile, index, card.uid, next));
            }
        }
    }
    let granted = !grants.is_empty();
    for (pile, index, uid, next) in grants {
        if state
            .piles
            .get(pile)
            .as_slice()
            .get(index)
            .is_none_or(|card| card.uid != uid)
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let mut physical = state.card_states.get(uid);
        physical
            .set_base_replay_count(Some(next))
            .ok_or(EngineRefusal::CounterOverflow(
                "Soldier's Stew BaseReplayCount",
            ))?;
        state.card_states.set(uid, physical);
        state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    }
    if granted {
        // `_commit_live_card_piles(..., force_exact=bool(granted))` publishes
        // exact mode for every successful Soldier's Stew grant, even when a
        // single Strike has no otherwise-distinguishable sort sibling.
        state.exact_piles = true;
    }
    Ok(())
}

fn potion_selection_candidates(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
) -> Result<Vec<crate::hot::HotCard>, EngineRefusal> {
    let pile = match potion {
        PotionId::DropletOfPrecognition => PileId::Draw,
        PotionId::LiquidMemories => PileId::Discard,
        PotionId::Ashwater | PotionId::GamblersBrew | PotionId::TouchOfInsanity => PileId::Hand,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    let mut candidates = state.piles.get(pile).as_slice().to_vec();
    if potion == PotionId::DropletOfPrecognition {
        candidates.sort_by_key(|card| {
            let spec = catalog.spec(card.atom).expect("live card atom is interned");
            let rarity = match spec.row.rarity {
                crate::content_tables::CardRarity::Basic => 1,
                crate::content_tables::CardRarity::Common => 2,
                crate::content_tables::CardRarity::Uncommon => 3,
                crate::content_tables::CardRarity::Rare => 4,
                crate::content_tables::CardRarity::Ancient => 5,
                crate::content_tables::CardRarity::Event => 6,
                crate::content_tables::CardRarity::Status => 7,
                crate::content_tables::CardRarity::Token => 8,
                crate::content_tables::CardRarity::Quest => 9,
                crate::content_tables::CardRarity::Curse => 10,
            };
            (rarity, spec.identity.id.as_str())
        });
    }
    if potion == PotionId::TouchOfInsanity {
        candidates.retain(|card| {
            let Some(spec) = catalog.spec(card.atom) else {
                return false;
            };
            if spec.x_cost || spec.row.star_x {
                return false;
            }
            super::play::resolved_local_energy_cost(state, *card, spec) > 0
                || super::play::resolved_star_cost_without_void_form(state, *card, spec)
                    .is_some_and(|cost| cost > 0)
                || super::play::resolved_energy_cost(state, *card, spec) > 0
                || super::play::resolved_star_cost(state, *card, spec).is_some_and(|cost| cost > 0)
        });
    }
    Ok(candidates)
}

pub(crate) fn pending_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let Some(pending) = state.pending.as_deref() else {
        return false;
    };
    let Some(record) = pending.potion_finish_record(&state.frames) else {
        return false;
    };
    let Ok(expected) = potion_selection_candidates(state, catalog, record.name) else {
        return false;
    };
    is_supported(record.name)
        && record.stage == PotionFinishStage::Effect
        && record.body_stage == PotionBodyStage::Selecting
        && record.candidates().eq(expected)
}

pub(crate) fn generation_pending_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let Some(record) = state
        .pending
        .as_deref()
        .and_then(|pending| pending.generation_potion_record(&state.frames))
    else {
        return false;
    };
    let Some(pool) = fight_generation_choice_pool(state, catalog, record.name) else {
        return false;
    };
    let options = record.generation_options().collect::<Vec<_>>();
    is_supported(record.name)
        && record.stage == PotionFinishStage::Effect
        && options.len() == 3
        && options.iter().enumerate().all(|(index, atom)| {
            !options[..index].contains(atom)
                && catalog.spec(*atom).is_some_and(|spec| {
                    spec.identity.upgrade == 0
                        && spec.identity.enchantment.is_none()
                        && pool.contains(&spec.identity.id)
                })
        })
}

pub(crate) fn generation_selection_action_count(
    state: &HotState,
    catalog: &Catalog,
) -> Result<u32, EngineRefusal> {
    generation_pending_is_exact(state, catalog)
        .then_some(4)
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

pub(crate) fn resume_generation_selection(
    state: &mut HotState,
    catalog: &Catalog,
    ordinal: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !generation_pending_is_exact(state, catalog) || ordinal >= 4 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let record = state
        .pending
        .as_deref()
        .and_then(|pending| pending.generation_potion_record(&state.frames))
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let potion = record.name;
    let selected = (ordinal < 3)
        .then(|| record.generation_options().nth(ordinal as usize))
        .flatten();
    state.pending = None;
    if let Some(atom) = selected {
        let spec = catalog.spec(atom).ok_or(EngineRefusal::UnknownAtom(atom))?;
        let generated_uid = state.next_card_uid;
        state
            .frames
            .replace_top_potion_finish(&PotionFinishRecord {
                name: potion,
                stage: PotionFinishStage::Effect,
                body_stage: PotionBodyStage::AfterChild,
                current_uid: Some(generated_uid),
                aux: 0,
                candidates: Vec::new(),
                generation_options: Vec::new(),
            })
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        super::cards::inject_generated_free_this_turn_bottom(
            state,
            catalog,
            spec.identity,
            PileId::Hand,
            events,
        )?;
        if state.pending.is_some()
            || !matches!(state.frames.top(), Some(Frame::PotionFinish { .. }))
        {
            return Ok(());
        }
    }
    restore_after_draw(state, potion)?;
    finish(state, catalog, potion, events)
}

pub(crate) fn resume_generation_after_child(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let record = match state.frames.top() {
        Some(Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if !is_supported(record.name)
        || !matches!(
            record.name,
            PotionId::AttackPotion
                | PotionId::SkillPotion
                | PotionId::PowerPotion
                | PotionId::ColorlessPotion
        )
        || record.stage != PotionFinishStage::Effect
        || record.body_stage != PotionBodyStage::AfterChild
        || record.current_uid.is_none()
        || record.candidates().next().is_some()
        || record.generation_options().next().is_some()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let potion = record.name;
    restore_after_draw(state, potion)?;
    finish(state, catalog, potion, events)
}

pub(crate) fn generation_after_child_is_exact(
    state: &HotState,
    catalog: &Catalog,
    record: crate::hot::PotionFinishView<'_>,
) -> bool {
    let Some(uid) = record.current_uid else {
        return false;
    };
    let Some(pool) = fight_generation_choice_pool(state, catalog, record.name) else {
        return false;
    };
    is_supported(record.name)
        && record.stage == PotionFinishStage::Effect
        && record.body_stage == PotionBodyStage::AfterChild
        && record.candidates().next().is_none()
        && record.generation_options().next().is_none()
        && super::play::unique_live_card_location(state, uid)
            .ok()
            .flatten()
            .is_some_and(|(pile, index)| {
                let card = state.piles.get(pile).as_slice()[index];
                catalog.spec(card.atom).is_some_and(|spec| {
                    spec.identity.upgrade == 0
                        && spec.identity.enchantment.is_none()
                        && pool.contains(&spec.identity.id)
                })
            })
}

pub(crate) fn selection_action_count(
    state: &HotState,
    catalog: &Catalog,
) -> Result<u32, EngineRefusal> {
    if !pending_is_exact(state, catalog) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let record = state
        .pending
        .as_deref()
        .and_then(|pending| pending.potion_finish_record(&state.frames))
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let candidates = record.candidates().collect::<Vec<_>>();
    let count = representative_count(state, catalog, record.name, &candidates)?;
    u32::try_from(count).map_err(|_| EngineRefusal::CounterOverflow("potion options"))
}

/// Ashwater's and Gambler's Brew's multi-card answers in NATIVE pick order
/// (#2524).
///
/// Authority: current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`).
/// `Ashwater/<OnUse>d__9::MoveNext` RVA `0x34bb90` awaits
/// `CardSelectCmd::FromHand` (IL_0075, `CardSelectorPrefs` 0..999999999 at
/// IL_0056-IL_005c), enumerates the returned list (IL_00d9 `GetEnumerator`)
/// and awaits `CardCmd::Exhaust` once per element in that order (IL_0101).
/// `GamblersBrew/<OnUse>d__6::MoveNext` RVA `0x34e3a4` awaits
/// `CardSelectCmd::FromHandForDiscard` (IL_0076, same prefs at IL_0069/IL_006a),
/// `ToList`s it (IL_00d5) and hands it to `CardCmd::DiscardAndDraw(list,
/// list.Count)` (IL_00e8), whose MoveNext (RVA `0x3e0274`) adds each card to
/// Discard/Bottom (IL_011d) and fires `AfterCardDiscarded` (IL_01a5) in list
/// order, then auto-plays the Sly children in that order (IL_02fd).
/// `FromHandForDiscard` d__29 (RVA `0x3e7a64`) returns `FromHand` (IL_005e)
/// verbatim, and `FromHand` d__28 (RVA `0x3e7568`) returns
/// `PlayerChoiceResult::AsCombatCards` (IL_0400) of the synchronized answer,
/// the player's own pick order. So Exhaust/Discard pile order, per-card hook
/// order, and Sly order all follow the pick order, and every ordered answer
/// is a distinct legal game action.
///
/// The complete ordered surface is `Σ_k P(n, k)` (9,864,101 answers over a
/// ten-card Hand), which the search cannot afford. The recorded decision for
/// #2524: `legal_actions` keeps emitting the canonical representatives
/// (ordinals `0..R`, the payload-sorted subsets and, for Gambler's Brew, their
/// Sly orders), and ordinals `R..R + Σ_k P(n, k)` name every ordered answer,
/// length-major then lexicographic over the frozen candidate (Hand) order.
/// Apply, replay and review accept both ranges; the search collapses the
/// order-only variants onto their representative. Each extension answer is an
/// exact native transition, never an approximation, and
/// [`ordered_selection_ordinal`] resolves a recorded pick list to its unique
/// ordinal (preferring the representative) so replay never enumerates the
/// extension.
fn ordered_pick_count(n: usize) -> Result<usize, EngineRefusal> {
    let overflow = || EngineRefusal::CounterOverflow("potion ordered options");
    let (mut total, mut block) = (1_usize, 1_usize);
    for k in 0..n {
        block = block.checked_mul(n - k).ok_or_else(overflow)?;
        total = total.checked_add(block).ok_or_else(overflow)?;
    }
    Ok(total)
}

/// `P(n, k)`; every caller has already bounded the full sum.
fn permutations(n: usize, k: usize) -> usize {
    (0..k).map(|index| n - index).product()
}

/// Unrank one ordered answer over the frozen candidate order.
fn ordered_pick_at(
    candidates: &[crate::hot::HotCard],
    mut rank: usize,
) -> Option<Vec<crate::hot::HotCard>> {
    let n = candidates.len();
    for k in 0..=n {
        let block = permutations(n, k);
        if rank >= block {
            rank -= block;
            continue;
        }
        let mut pool = candidates.to_vec();
        let mut out = Vec::with_capacity(k);
        for position in 0..k {
            let sub = permutations(pool.len() - 1, k - position - 1);
            out.push(pool.remove(rank / sub));
            rank %= sub;
        }
        return Some(out);
    }
    None
}

/// Rank an ordered pick list over the frozen candidate order.
fn ordered_pick_rank(candidates: &[crate::hot::HotCard], uids: &[u32]) -> Option<usize> {
    let n = candidates.len();
    let k = uids.len();
    if k > n {
        return None;
    }
    let mut rank = (0..k).map(|length| permutations(n, length)).sum::<usize>();
    let mut pool = candidates.iter().map(|card| card.uid).collect::<Vec<_>>();
    for (position, uid) in uids.iter().enumerate() {
        let index = pool.iter().position(|live| live == uid)?;
        rank += index * permutations(pool.len() - 1, k - position - 1);
        pool.remove(index);
    }
    Some(rank)
}

fn representative_count(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
    candidates: &[crate::hot::HotCard],
) -> Result<usize, EngineRefusal> {
    Ok(match potion {
        PotionId::Ashwater => 1_usize
            .checked_shl(
                u32::try_from(candidates.len())
                    .map_err(|_| EngineRefusal::CounterOverflow("potion options"))?,
            )
            .ok_or(EngineRefusal::CounterOverflow("potion options"))?,
        PotionId::GamblersBrew => gambler_option_count(state, catalog, candidates)?,
        _ => candidates.len(),
    })
}

fn has_ordered_extension(potion: PotionId) -> bool {
    matches!(potion, PotionId::Ashwater | PotionId::GamblersBrew)
}

/// `R + Σ_k P(n, k)`, refused by name past the `u32` wire ordinal.
fn accepted_count(representatives: usize, candidates: usize) -> Result<usize, EngineRefusal> {
    representatives
        .checked_add(ordered_pick_count(candidates)?)
        .filter(|total| *total <= u32::MAX as usize + 1)
        .ok_or(EngineRefusal::CounterOverflow("potion ordered options"))
}

/// Decode any accepted potion selection ordinal: a legal representative
/// (`0..R`) or, for Ashwater and Gambler's Brew, an ordered answer
/// (`R..R + Σ_k P(n, k)`). See [`ordered_pick_count`].
pub(crate) fn potion_selection_at(
    state: &HotState,
    catalog: &Catalog,
    potion: PotionId,
    candidates: &[crate::hot::HotCard],
    ordinal: u32,
) -> Result<Vec<crate::hot::HotCard>, EngineRefusal> {
    let bad = || EngineRefusal::MalformedArgs("selection option index");
    let index = usize::try_from(ordinal).map_err(|_| bad())?;
    if !has_ordered_extension(potion) {
        return Ok(vec![*candidates.get(index).ok_or_else(bad)?]);
    }
    let representatives = representative_count(state, catalog, potion, candidates)?;
    if index < representatives {
        return subset_option_at(
            state,
            catalog,
            candidates,
            ordinal,
            potion == PotionId::GamblersBrew,
        );
    }
    if index >= accepted_count(representatives, candidates.len())? {
        return Err(bad());
    }
    ordered_pick_at(candidates, index - representatives).ok_or_else(bad)
}

/// The unique accepted ordinal whose answer is exactly `uids`, in order, for
/// the pending Ashwater/Gambler's Brew selection: the legal representative
/// when one applies the same order, otherwise the ordered extension. `None`
/// when no potion selection with an ordered extension is pending or the list
/// names no answer.
pub(crate) fn ordered_selection_ordinal(
    state: &HotState,
    catalog: &Catalog,
    uids: &[u32],
) -> Result<Option<u32>, EngineRefusal> {
    if !pending_is_exact(state, catalog) {
        return Ok(None);
    }
    let Some(record) = state
        .pending
        .as_deref()
        .and_then(|pending| pending.potion_finish_record(&state.frames))
    else {
        return Ok(None);
    };
    if !has_ordered_extension(record.name) {
        return Ok(None);
    }
    let candidates = record.candidates().collect::<Vec<_>>();
    let representatives = representative_count(state, catalog, record.name, &candidates)?;
    let total = accepted_count(representatives, candidates.len())?;
    for ordinal in 0..representatives {
        let ordinal =
            u32::try_from(ordinal).map_err(|_| EngineRefusal::CounterOverflow("potion options"))?;
        let answer = subset_option_at(
            state,
            catalog,
            &candidates,
            ordinal,
            record.name == PotionId::GamblersBrew,
        )?;
        if answer.iter().map(|card| card.uid).eq(uids.iter().copied()) {
            return Ok(Some(ordinal));
        }
    }
    Ok(ordered_pick_rank(&candidates, uids)
        .map(|rank| representatives + rank)
        .filter(|ordinal| *ordinal < total)
        .map(|ordinal| ordinal as u32))
}

/// Whether `ordinal` is an ordered-extension answer (not a legal
/// representative) of the pending Ashwater/Gambler's Brew selection.
pub(crate) fn is_ordered_extension_ordinal(
    state: &HotState,
    catalog: &Catalog,
    ordinal: u32,
) -> bool {
    let Some(record) = state
        .pending
        .as_deref()
        .and_then(|pending| pending.potion_finish_record(&state.frames))
    else {
        return false;
    };
    if !pending_is_exact(state, catalog) || !has_ordered_extension(record.name) {
        return false;
    }
    let candidates = record.candidates().collect::<Vec<_>>();
    let Ok(representatives) = representative_count(state, catalog, record.name, &candidates) else {
        return false;
    };
    let index = ordinal as usize;
    index >= representatives
        && accepted_count(representatives, candidates.len()).is_ok_and(|total| index < total)
}

/// Count `_pick_subsets(..., ordered=True, exact=True)` without materializing
/// its potentially large permutation surface.  Each exact card is its own
/// Counter key; an ordinary subset has one canonical order, while a subset
/// containing `k` Sly cards contributes `k!` relative Sly orders.
fn gambler_option_count(
    state: &HotState,
    catalog: &Catalog,
    candidates: &[crate::hot::HotCard],
) -> Result<usize, EngineRefusal> {
    let sorted =
        super::selection::sorted_physical_card_pick_payloads(state, catalog, candidates.to_vec())?;
    let masks = 1_usize
        .checked_shl(
            u32::try_from(sorted.len())
                .map_err(|_| EngineRefusal::CounterOverflow("potion options"))?,
        )
        .ok_or(EngineRefusal::CounterOverflow("potion options"))?;
    let mut total = 0_usize;
    for mask in 0..masks {
        let sly = sorted
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << (sorted.len() - 1 - index)) != 0)
            .filter(|(_, card)| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| state.card_states.get(card.uid).is_sly(spec.sly))
            })
            .count();
        let variants = (2..=sly).try_fold(1_usize, |value, factor| {
            value
                .checked_mul(factor)
                .ok_or(EngineRefusal::CounterOverflow("potion options"))
        })?;
        total = total
            .checked_add(variants)
            .ok_or(EngineRefusal::CounterOverflow("potion options"))?;
    }
    Ok(total)
}

/// Decode the stable ordinal of Python's exact subset enumerator. Counter
/// expansion makes the last payload-sorted candidate the low mask bit. For
/// Gambler's Brew, ordinary picks stay payload-sorted and only the selected
/// Sly tail is permuted.
pub(crate) fn subset_option_at(
    state: &HotState,
    catalog: &Catalog,
    candidates: &[crate::hot::HotCard],
    ordinal: u32,
    ordered_sly: bool,
) -> Result<Vec<crate::hot::HotCard>, EngineRefusal> {
    let sorted =
        super::selection::sorted_physical_card_pick_payloads(state, catalog, candidates.to_vec())?;
    let masks = 1_usize
        .checked_shl(
            u32::try_from(sorted.len())
                .map_err(|_| EngineRefusal::CounterOverflow("potion options"))?,
        )
        .ok_or(EngineRefusal::CounterOverflow("potion options"))?;
    let mut remaining =
        usize::try_from(ordinal).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    for mask in 0..masks {
        let chosen = sorted
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, card)| {
                (mask & (1 << (sorted.len() - 1 - index)) != 0).then_some(card)
            })
            .collect::<Vec<_>>();
        let (ordinary, mut sly): (Vec<_>, Vec<_>) = chosen.into_iter().partition(|card| {
            !catalog
                .spec(card.atom)
                .is_some_and(|spec| state.card_states.get(card.uid).is_sly(spec.sly))
        });
        // Python's corrected physical Sly order is payload, then uid. The
        // shared payload sorter is stable for ordinary selectors, so add the
        // physical tie-break only to this order-observable permutation tail.
        for right in 1..sly.len() {
            let mut index = right;
            while index > 0 {
                let order =
                    super::selection::card_payload_cmp(state, catalog, sly[index], sly[index - 1])?;
                if order == std::cmp::Ordering::Greater
                    || order == std::cmp::Ordering::Equal && sly[index].uid >= sly[index - 1].uid
                {
                    break;
                }
                sly.swap(index, index - 1);
                index -= 1;
            }
        }
        let variants = if ordered_sly {
            (2..=sly.len()).try_fold(1_usize, |value, factor| {
                value
                    .checked_mul(factor)
                    .ok_or(EngineRefusal::CounterOverflow("potion options"))
            })?
        } else {
            1
        };
        if remaining >= variants {
            remaining -= variants;
            continue;
        }
        if ordered_sly && sly.len() > 1 {
            // Lexicographic permutation unranking over the stable
            // payload-sorted physical identities.
            let mut pool = sly;
            sly = Vec::with_capacity(pool.len());
            for width in (1..=pool.len()).rev() {
                let block = (2..width).try_fold(1_usize, |value, factor| {
                    value
                        .checked_mul(factor)
                        .ok_or(EngineRefusal::CounterOverflow("potion options"))
                })?;
                let index = remaining / block;
                remaining %= block;
                sly.push(pool.remove(index));
            }
        }
        let mut selected = ordinary;
        selected.extend(sly);
        return Ok(selected);
    }
    Err(EngineRefusal::ContinuationNotModeled)
}

fn begin_card_selection(
    state: &mut HotState,
    catalog: &Catalog,
    potion: PotionId,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    super::cards::normalize_card_identities(state)?;
    state.exact_piles = true;
    let candidates = potion_selection_candidates(state, catalog, potion)?;
    let requires_choice = match potion {
        PotionId::Ashwater | PotionId::GamblersBrew => !candidates.is_empty(),
        _ => candidates.len() > 1,
    };
    if requires_choice {
        state
            .frames
            .replace_top_potion_finish(&PotionFinishRecord {
                name: potion,
                stage: PotionFinishStage::Effect,
                body_stage: PotionBodyStage::Selecting,
                current_uid: None,
                aux: 0,
                candidates,
                generation_options: Vec::new(),
            })
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let Frame::PotionFinish { record } = state
            .frames
            .top()
            .ok_or(EngineRefusal::ContinuationNotModeled)?
        else {
            return Err(EngineRefusal::ContinuationNotModeled);
        };
        state.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
            frame_uid: crate::hot::POTION_SELECTION_PENDING_UID,
            frame_record: record,
        }));
        return Ok(true);
    }
    if let Some(card) = candidates.first().copied() {
        apply_selected_card(state, catalog, potion, card, events)?;
    }
    Ok(false)
}

fn apply_selected_card(
    state: &mut HotState,
    catalog: &Catalog,
    potion: PotionId,
    frozen: crate::hot::HotCard,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let source = match potion {
        PotionId::DropletOfPrecognition => PileId::Draw,
        PotionId::LiquidMemories => PileId::Discard,
        PotionId::TouchOfInsanity => PileId::Hand,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    let pile = state.piles.get(source).as_slice();
    let index = pile
        .iter()
        .position(|card| card.uid == frozen.uid)
        .filter(|index| pile[*index] == frozen)
        .ok_or(EngineRefusal::FrozenCardVanished {
            uid: frozen.uid,
            pile: source,
        })?;
    if potion == PotionId::LiquidMemories {
        let spec = catalog
            .spec(frozen.atom)
            .ok_or(EngineRefusal::UnknownAtom(frozen.atom))?;
        state
            .card_states
            .set_to_free_this_turn(frozen.uid, spec.cost)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    }
    if potion == PotionId::TouchOfInsanity {
        let spec = catalog
            .spec(frozen.atom)
            .ok_or(EngineRefusal::UnknownAtom(frozen.atom))?;
        state
            .card_states
            .set_to_free_this_combat(frozen.uid, spec.cost)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let live = state
            .piles
            .get_mut(source)
            .make_mut()
            .get_mut(index)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        live.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.exact_piles = true;
        return Ok(());
    }
    if state.history.over {
        return Ok(());
    }
    let moved = state.piles.get_mut(source).make_mut().remove(index);
    let destination = if state.piles.get(PileId::Hand).len() < super::draw::MAX_CARDS_IN_HAND {
        PileId::Hand
    } else {
        PileId::Discard
    };
    state.piles.get_mut(destination).make_mut().push(moved);
    events.push(Event::CardResolved {
        uid: moved.uid,
        pile: destination,
    });
    Ok(())
}

pub(crate) fn resume_selection(
    state: &mut HotState,
    catalog: &Catalog,
    ordinal: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !pending_is_exact(state, catalog) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let record = state
        .pending
        .as_deref()
        .and_then(|pending| pending.potion_finish_record(&state.frames))
        .ok_or(EngineRefusal::ContinuationNotModeled)?
        .to_owned();
    let selected = match record.name {
        PotionId::Ashwater | PotionId::GamblersBrew => {
            potion_selection_at(state, catalog, record.name, &record.candidates, ordinal)?
        }
        _ => vec![
            *record
                .candidates
                .get(usize::try_from(ordinal).map_err(|_| EngineRefusal::ContinuationNotModeled)?)
                .ok_or(EngineRefusal::ContinuationNotModeled)?,
        ],
    };
    state.pending = None;
    state
        .frames
        .replace_top_potion_finish(&PotionFinishRecord {
            name: record.name,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    match record.name {
        PotionId::Ashwater => {
            run_ashwater_exhaust_sequence(state, catalog, selected, events)?;
            return Ok(());
        }
        PotionId::GamblersBrew => {
            // GamblersBrew/<OnUse>d__6::MoveNext RVA 0x34e3a4 has no ending
            // gate of its own: the answer is `ToList`ed at IL_00d5 and handed
            // to `CardCmd.DiscardAndDraw(list, list.Count)` at IL_00e8. The
            // shared walk (#3075) keeps the picks in Hand until each one's
            // own CardPileCmd.Add, in the ordered answer's pick order:
            // DiscardAndDraw RVA 0x3e0274 IL_0029-0035 makes the whole command
            // a no-op once combat is over, and Add RVA 0x3e1ba4 IL_004e-008a
            // leaves later picks in Hand after a mid-walk kill while history
            // (IL_018e) and AfterCardDiscarded (IL_01a5) still run per card.
            // Sly is captured per card before its move (IL_00f6). The paired
            // Draw below is ending-gated like IL_0246-0271's.
            let sly = super::draw::discard_cards_collect_sly(
                state,
                catalog,
                PileId::Hand,
                &selected,
                events,
            )?;
            set_gambler_sly_cursor(state, sly)?;
            if !selected.is_empty()
                && !state.history.over
                && super::draw::draw_cards_for_potion(
                    state,
                    catalog,
                    selected.len(),
                    DrawCaller::SlyAfterDraw,
                    events,
                )? == super::draw::PotionDrawResult::Suspended
            {
                return Ok(());
            }
            run_gambler_sly_tail(state, catalog, events)?;
            return Ok(());
        }
        _ => apply_selected_card(state, catalog, record.name, selected[0], events)?,
    }
    finish(state, catalog, record.name, events)
}

/// The enemy share of one potion `CreatureCmd::Damage` over several enemies:
/// a flat, unpowered (`DamageVar` props 4), blockable amount dealt by the
/// player with no card source.
///
/// Native `CreatureCmd/<Damage>d__12::MoveNext` RVA `0x3e96c8` snapshots its
/// targets (IL_00c6 `ToList`), commits HP loss to every one of them (the loop
/// closing at IL_0aa4), dispatches the frozen results (IL_0adf-IL_0e84:
/// AfterBlockBroken, AfterCurrentHpChanged, AfterDamageGiven,
/// AfterDamageReceived), and only then awaits ONE `CreatureCmd::Kill` over the
/// retained killed list (IL_0eb4). A death listener therefore runs after every
/// target has lost its HP: when the last enemies die together, Gremlin Horn's
/// `<AfterDeath>d__6` (`0x326170`) finds the combat ending and
/// `PlayerCmd/<GainEnergy>d__3` (`0x3ee8a0`, IL_0030-IL_003c `IsEnding`)
/// grants nothing (#3244). The Rust batch primitive is
/// [`super::damage::damage_monsters_after_catalog_auth`]; this entry first
/// makes the same catalog authentication the single-target
/// `damage_monster_with_catalog` entry makes, so a potion keeps its old
/// fail-closed surface.
fn potion_enemy_damage_batch(
    state: &mut HotState,
    catalog: &Catalog,
    amount: DotNetDecimal,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == crate::ids::MonsterKind::ThievingHopper);
    if hopper_reachable
        && (!super::monsters::thieving_hopper_state_is_valid(state)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(state, catalog))
    {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper damage entry"));
    }
    if state.card_states.dampen().is_some() && !super::cards::dampen_state_is_exact(state, catalog)
    {
        return Err(EngineRefusal::MalformedArgs("Dampen damage entry"));
    }
    if super::cards::aeonglass_reachable(state, catalog)
        && !super::cards::aeonglass_state_is_exact(state, catalog)
    {
        return Err(EngineRefusal::MalformedArgs("Aeonglass damage entry"));
    }
    let targets = super::damage::alive_targets(state);
    super::damage::damage_monsters_after_catalog_auth(
        state, catalog, &targets, amount, true, events,
    )
}

/// Use one already-validated physical belt slot.
pub(crate) fn use_potion(
    state: &mut HotState,
    catalog: &Catalog,
    slot: u8,
    target: Option<u8>,
    potion: PotionId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !is_supported(potion) {
        return Err(EngineRefusal::PotionsNotModeled);
    }
    if !potion_target_roster_is_exact(state, potion) {
        return Err(EngineRefusal::MalformedArgs("potion AnyAlly target roster"));
    }
    // UsePotionAction.Execute RVA 0x10e078 / MoveNext 0x3d58ec removes the
    // exact physical slot before invoking OnUseWrapper. Every later refusal
    // is nevertheless atomic because apply_action owns the outer COW clone.
    if state.fanouts.remove_potion_at(usize::from(slot)) != Some(potion) {
        return Err(EngineRefusal::BadPotionSlot(slot));
    }
    state
        .frames
        .push_potion_finish(&PotionFinishRecord {
            name: potion,
            stage: PotionFinishStage::Effect,
            body_stage: PotionBodyStage::Synchronous,
            current_uid: None,
            aux: 0,
            candidates: Vec::new(),
            generation_options: Vec::new(),
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let enemy = target.map(usize::from);
    // `SurroundedPower.BeforePotionUsed` (`0xa8c78`, body
    // `SurroundedPower/<BeforePotionUsed>d__12::MoveNext` `0x34760c`) returns
    // unless `CombatManager.IsInProgress` (IL_001d-IL_0029) — a gate its
    // `BeforeCardPlayed` sibling does not carry — then on a null target
    // (IL_002e-IL_0036) and on a potion whose owner is not the listener's own
    // player (IL_003b-IL_0053), before awaiting the same
    // `UpdateDirection(target)` (IL_0055-IL_005c). Python runs it at the same
    // point inside `_apply_action_impl`, right after the targeted-write
    // validation and before the belt removal (frozen Python, deleted #2827). `use_potion` is only reachable while combat is in
    // progress, so the first gate is structurally satisfied here.
    super::monsters::update_kaiser_facing(state, enemy)?;
    match potion {
        // 0xabad0/0x34bf8c.
        PotionId::BeetleJuice => {
            let target = enemy.expect("validated enemy target");
            debuff(
                state,
                catalog,
                target,
                PowerId::Shrink,
                MiseryToken::Shrink,
                4,
                events,
            )?;
        }
        // Synchronous body RVA 0xabb34.
        PotionId::BlessingOfTheForge => {
            // Python's physical pile commit allocates every legacy card uid
            // before preserving the Hand identities through the upgrade.
            super::cards::normalize_card_identities(state)?;
            let uids = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>();
            super::cards::upgrade_live_cards_once(state, catalog, &uids)?;
        }
        // 0xabbf4/0x34c0a8.
        PotionId::BlockPotion => {
            super::orbs::gain_flat_decimal_block(
                state,
                catalog,
                DotNetDecimal::from_i64(12),
                events,
            )?;
        }
        // 0xabc68/0x34c188.
        PotionId::BloodPotion => {
            let heal = i64::from(state.max_hp) * 20 / 100;
            let hp_before = state.hp;
            state.hp = i64::from(state.max_hp)
                .min(i64::from(state.hp) + heal)
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("blood potion heal"))?;
            // One `CreatureCmd.Heal` (`0x3eb4b0`): Red Skull's
            // `AfterCurrentHpChanged` (#3044).
            super::damage::red_skull_after_player_hp_changed(
                state,
                hp_before,
                state.max_hp,
                events,
            )?;
        }
        // Attack/Skill/Power/Colorless potion bodies each consume one full
        // Generation-stream shuffle, freeze the first three L0 identities,
        // and expose those options followed by the native skip action.
        PotionId::AttackPotion
        | PotionId::SkillPotion
        | PotionId::PowerPotion
        | PotionId::ColorlessPotion => {
            begin_generation_choice(state, catalog, potion)?;
            return Ok(());
        }
        // Orobic Acid performs the Attack, Skill, then Power full shuffles
        // before one ordered plural generated-card command. Unlike the four
        // choice potions it has no selector entry point: the serial generated
        // callback is entered directly from OnUse.
        PotionId::OrobicAcid => {
            begin_orobic(state, catalog, events)?;
            return Ok(());
        }
        PotionId::DistilledChaos => {
            begin_distilled_chaos(state, catalog, events)?;
            return Ok(());
        }
        // SwiftPotion OnUse body: one ordinary command Draw(3).
        PotionId::SwiftPotion => {
            wait_for_draw(state, potion)?;
            if super::draw::draw_cards_for_potion(
                state,
                catalog,
                3,
                DrawCaller::PotionEpilogue,
                events,
            )? == super::draw::PotionDrawResult::Suspended
            {
                return Ok(());
            }
            restore_after_draw(state, potion)?;
        }
        // CureAll OnUse: GainEnergy(1), then ordinary Draw(2).
        PotionId::CureAll => {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
            wait_for_draw(state, potion)?;
            if super::draw::draw_cards_for_potion(
                state,
                catalog,
                2,
                DrawCaller::PotionEpilogue,
                events,
            )? == super::draw::PotionDrawResult::Suspended
            {
                return Ok(());
            }
            restore_after_draw(state, potion)?;
        }
        // Clarity OnUse: Draw(1), await it completely, then apply three
        // Clarity stacks through a distinct ending-gated command.
        PotionId::Clarity => {
            wait_for_draw(state, potion)?;
            if super::draw::draw_cards_for_potion(
                state,
                catalog,
                1,
                DrawCaller::ClarityPotion,
                events,
            )? == super::draw::PotionDrawResult::Suspended
            {
                return Ok(());
            }
            if !state.history.over {
                super::damage::null_applier_power_amount_changed_is_exact(state)?;
                let clarity = checked_cold_add(state.fanouts.clarity(), 3, "clarity")?;
                assert!(state.fanouts.set_clarity(clarity));
            }
            restore_after_draw(state, potion)?;
        }
        // BottledPotential: frozen Hand-to-Draw, full Shuffle/AfterShuffle,
        // then a distinct ordinary Draw(5).
        PotionId::BottledPotential => {
            // The captured Hand is an ordered physical CardModel list. Force
            // exact identities before freezing it so legacy/equal siblings
            // retain distinct native allocation order through the move and
            // the post-shuffle Draw continuation.
            super::cards::normalize_card_identities(state)?;
            state.exact_piles = true;
            let hand = state.piles.get(PileId::Hand).as_slice().to_vec();
            if super::draw::bottled_potential_shuffle(state, catalog, &hand, events)? {
                return Ok(());
            }
            if !state.history.over {
                wait_for_draw(state, potion)?;
                if super::draw::draw_cards_for_potion(
                    state,
                    catalog,
                    5,
                    DrawCaller::PotionEpilogue,
                    events,
                )? == super::draw::PotionDrawResult::Suspended
                {
                    return Ok(());
                }
                restore_after_draw(state, potion)?;
            }
        }
        // SneckoOil: Draw(7), then re-read the live Hand in exact pile order
        // and roll each fixed nonnegative Energy base on CombatEnergyCosts.
        PotionId::SneckoOil => {
            wait_for_draw(state, potion)?;
            if super::draw::draw_cards_for_potion(state, catalog, 7, DrawCaller::SneckoOil, events)?
                == super::draw::PotionDrawResult::Suspended
            {
                return Ok(());
            }
            apply_snecko_oil_after_draw(state, catalog)?;
            restore_after_draw(state, potion)?;
            state.exact_piles = true;
        }
        PotionId::Ashwater
        | PotionId::GamblersBrew
        | PotionId::TouchOfInsanity
        | PotionId::DropletOfPrecognition
        | PotionId::LiquidMemories => {
            if begin_card_selection(state, catalog, potion, events)? {
                return Ok(());
            }
        }
        // 0xac078/0x34cdc8.
        PotionId::DexterityPotion => checked_power_add(state, PowerId::Dexterity, 2, events)?,
        // `FocusPotion/<OnUse>d__10::MoveNext` RVA `0x34dbd4`, OnUse `0xac5c8`.
        //
        // One awaited `PowerCmd::Apply<FocusPower>` at IL_0051 with amount
        // `DynamicVars["FocusPower"].BaseValue` (IL_0035-IL_0044;
        // `get_CanonicalVars` `0xac594` is 2). Unlike Dexterity Potion
        // (`0x34cdc8` IL_0044-IL_004a, applier `Owner.Creature`), both the
        // recipient and the applier are the picked `target` (IL_002f and
        // IL_0049). `get_TargetType` (`0xac591`) is 5, whose
        // `PotionModel::IsValidTarget` branch accepts only a living player, so
        // in a teammate-free roster the pick is the owner — a live Osty is no
        // pick (#2497, `native_target_excludes_pets`); a teammate pick stays
        // behind `requires_solo_player_target`.
        // `FocusPower` (`0xa27ee`-`0xa2829`) is Type 1, Counter-stacked,
        // AllowNegative, and overrides only `ModifyOrbValue`, so the write has
        // no apply-time hook of its own; orb reads pick it up in
        // `orbs::orb_value`.
        PotionId::FocusPotion => checked_power_add(state, PowerId::Focus, 2, events)?,
        // 0xac1b4/0x34d154.
        PotionId::Duplicator => {
            let current = state.fanouts.duplication();
            let next = checked_cold_add(current, 1, "duplication")?;
            super::turn::prepare_after_side_turn_end_singleton_write(
                state,
                crate::hot::AfterSideTurnEndPowerToken::Duplication,
                current != 0,
                true,
            )?;
            state.fanouts.set_duplication(next);
        }
        // 0xac22c/0x34d234.
        PotionId::EnergyPotion => {
            state.energy = state
                .energy
                .checked_add(2)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
        }
        // ExplosiveAmpoule/<OnUse>d__8::MoveNext RVA `0x34d5b0`, OnUse
        // `0xac370`.
        //
        // `<targets>5__4` is `CombatState.HittableEnemies` (IL_0050-IL_005a),
        // and after the VFX wait the body awaits ONE `CreatureCmd::Damage`
        // over that whole list at IL_0143, dealer `<player>5__2`, no card
        // source. `get_CanonicalVars` (`0xac35c`) is `DamageVar(10, props 4)`
        // and `get_TargetType` (`0xac359`) is 3, so no target is picked. One
        // command means one batched Kill after every target's HP loss (#3244):
        // see `potion_enemy_damage_batch`.
        PotionId::ExplosiveAmpoule => {
            potion_enemy_damage_batch(state, catalog, DotNetDecimal::from_i64(10), events)?;
        }
        // 0xac498/0x34d9b4.
        PotionId::FirePotion => {
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                enemy.expect("validated enemy target"),
                DotNetDecimal::from_i64(20),
                false,
                true,
                events,
            )?;
        }
        // 0xac530/0x34dad8.
        PotionId::FlexPotion => super::allies::gain_temp_strength(state, 0, 5, events)?,

        // FruitJuice/<OnUse>d__10::MoveNext RVA 0x34e130. GainMaxHp accepts
        // only the room below the native 999,999,999 cap and Heal receives
        // exactly that accepted delta.
        PotionId::FruitJuice => {
            let accepted = 5_i32.min(999_999_999_i32.saturating_sub(state.max_hp));
            let (hp_before, max_hp_before) = (state.hp, state.max_hp);
            state.max_hp = state
                .max_hp
                .checked_add(accepted)
                .ok_or(EngineRefusal::CounterOverflow("Fruit Juice max hp"))?;
            state.hp = state.max_hp.min(
                state
                    .hp
                    .checked_add(accepted)
                    .ok_or(EngineRefusal::CounterOverflow("Fruit Juice heal"))?,
            );
            // `GainMaxHp` (`<GainMaxHp>d__22` `0x3eb2f0`) awaits `SetMaxHp`
            // (IL_005b) and then `Heal` (IL_010b), whose
            // `AfterCurrentHpChanged` sees both new values (#3044).
            super::damage::red_skull_after_player_hp_changed(
                state,
                hp_before,
                max_hp_before,
                events,
            )?;
        }
        // 0xac640/0x34dccc.
        PotionId::Fortifier => {
            let gain = i64::from(state.block)
                .checked_mul(2)
                .ok_or(EngineRefusal::CounterOverflow("Fortifier block"))?;
            super::orbs::gain_flat_decimal_block(
                state,
                catalog,
                DotNetDecimal::from_i64(gain),
                events,
            )?;
        }
        // FoulPotion/<OnUse>d__10::MoveNext RVA `0x34ddd0`, OnUse `0xac734`.
        //
        // The in-combat branch (`IsInProgress` at IL_002c) issues ONE
        // `CreatureCmd::Damage` over
        // `Owner.Creature.CombatState.Creatures.Where(<OnUse>b__10_0)`, whose
        // predicate (`0x34ddc2`) is `!Creature.IsPet` — so the list is every
        // enemy AND the player, and never a pet. `get_CanonicalVars`
        // (`0xac6a2`) supplies `DamageVar(12, props 4)`: blockable,
        // unpowered, no Strength/Vulnerable scaling, no Skittish/PersonalHive.
        // `get_TargetType` (`0xac691`) is 3 in combat and 8 only out of it, so
        // no target index is picked; the merchant/gold branch below IL_0104 is
        // unreachable while `IsInProgress`.
        //
        // Creature order (#3244). `CombatState::get_Creatures` (`0x136e51`)
        // is `_allies.Concat(_enemies)`, so the player is the FIRST target.
        // `<Damage>d__12` (`0x3e96c8`) commits every target's HP loss (loop
        // closing IL_0aa4), then dispatches the results in that order
        // (IL_0adf-IL_0e84), then awaits one `Kill` over the killed list
        // (IL_0eb4). Native order is therefore: player commit, enemy
        // commits, player dispatch, enemy dispatches, one Kill. The player's
        // share always lands, even when the enemy share ends the combat,
        // because no Kill has run yet.
        //
        // Rust runs the player's whole share (commit and dispatch), then the
        // enemy batch. That moves only the player's dispatch ahead of the
        // enemy commits. The enemy commit is a flat unpowered amount, so a
        // player-local listener (Red Skull, Rupture, Demon Tongue,
        // Self-Forming Clay, Beating Remnant) can neither read it nor change
        // it. Refuse by name where the swap is observable: a positive loss
        // with Inferno (its nested Damage hits the enemies) or an armed
        // Centennial Puzzle (its draws run card and power listeners), and any
        // share that could be lethal (native still commits every enemy and
        // folds the player into the one Kill).
        PotionId::FoulPotion => {
            let prospective_loss = (12 - i64::from(state.block.max(0))).max(0);
            if prospective_loss >= i64::from(state.hp) {
                return Err(EngineRefusal::MalformedArgs(
                    "Foul Potion player share may be lethal inside the batch",
                ));
            }
            if prospective_loss > 0
                && (state.powers.value(PowerId::Inferno) > 0 || state.fanouts.puzzle_armed())
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Foul Potion player dispatch listener before the enemy commits",
                ));
            }
            super::damage::damage_player_from_power_with_catalog(state, catalog, 12, true, events)?;
            if state.history.over {
                return Err(EngineRefusal::MalformedArgs(
                    "Foul Potion player share ended the combat",
                ));
            }
            potion_enemy_damage_batch(state, catalog, DotNetDecimal::from_i64(12), events)?;
        }
        // 0xac950/0x34e214.
        PotionId::FyshOil => {
            super::damage::apply_owner_strength(state, 1, events)?;
            if !state.history.over {
                checked_power_add(state, PowerId::Dexterity, 1, events)?;
            }
        }
        // 0xacac4/0x34e640.
        PotionId::GigantificationPotion => {
            let next = checked_cold_add(state.fanouts.gigantification(), 1, "gigantification")?;
            state.fanouts.set_gigantification(next);
        }
        // GhostInAJar/<OnUse>d__10::MoveNext RVA `0x34e544`, OnUse `0xaca4c`.
        //
        // `Apply<IntangiblePower>(DynamicVars["IntangiblePower"].BaseValue,
        // recipient = Owner.Creature, null, 0)` — IL_003a names the var and
        // IL_004a resolves the recipient through `PotionModel.get_Owner` /
        // `Player.get_Creature`, NOT through the picked target. Hence the
        // ally-safe classification above despite `get_TargetType` 5
        // (`0xaca15`). `get_CanonicalVars` (`0xaca18`) is `Decimal.One`, so
        // the amount is 1 and stacks additively.
        PotionId::GhostInAJar => checked_power_add(state, PowerId::Intangible, 1, events)?,
        // Glowwater Potion exhausts its frozen Hand snapshot and then draws
        // ten; both halves can park, so the body owns its own return.
        PotionId::GlowwaterPotion => {
            begin_glowwater(state, catalog, events)?;
            return Ok(());
        }
        // 0xacbf4/0x34e98c.
        PotionId::HeartOfIron => checked_power_add(state, PowerId::Plating, 7, events)?,
        // 0xacd00/0x34eb78.
        PotionId::LiquidBronze => checked_power_add(state, PowerId::Thorns, 3, events)?,
        // 0xacdfc/0x34ee24.
        PotionId::LuckyTonic => checked_power_add(state, PowerId::Buffer, 1, events)?,
        // 0xaceb0/0x34ef20.
        PotionId::MazalethsGift => {
            let current = state.fanouts.player_ritual();
            let next = checked_cold_add(current, 1, "player ritual")?;
            super::turn::prepare_after_side_turn_end_singleton_write(
                state,
                crate::hot::AfterSideTurnEndPowerToken::Ritual,
                current != 0,
                true,
            )?;
            state.fanouts.set_player_ritual(next);
        }
        // 0xad06c/0x34f404.
        PotionId::PotionOfBinding => {
            let targets = state
                .monsters
                .iter()
                .enumerate()
                .filter(|(_, monster)| monster.hp > 0)
                .map(|(index, monster)| (index, monster.slot, monster.uid))
                .collect::<Vec<_>>();
            for &(target, slot, uid) in &targets {
                if state.history.over {
                    break;
                }
                if state.monsters.get(target).is_none_or(|monster| {
                    monster.hp <= 0 || (monster.slot, monster.uid) != (slot, uid)
                }) {
                    continue;
                }
                debuff(
                    state,
                    catalog,
                    target,
                    PowerId::Weak,
                    MiseryToken::Weak,
                    1,
                    events,
                )?;
            }
            if !state.history.over {
                for (target, slot, uid) in targets {
                    if state.history.over {
                        break;
                    }
                    if state.monsters.get(target).is_none_or(|monster| {
                        monster.hp <= 0 || (monster.slot, monster.uid) != (slot, uid)
                    }) {
                        continue;
                    }
                    debuff(
                        state,
                        catalog,
                        target,
                        PowerId::Vuln,
                        MiseryToken::Vuln,
                        1,
                        events,
                    )?;
                }
            }
        }
        // PotionOfDoom/<OnUse>d__10::MoveNext RVA `0x34f738`, OnUse `0xad168`.
        //
        // `get_TargetType` (`0xad131`) is 2 — one picked enemy — and the body
        // is a single `Apply<DoomPower>(target, DynamicVars.Doom.BaseValue,
        // applier = Owner.Creature, null, 0)` at IL_006f. `get_CanonicalVars`
        // (`0xad134`) is `Decimal(33)`. The applier is the player and the
        // card source is null, so the application routes through the ordinary
        // player Type-2 path: Artifact is consumed first and no Unsettling
        // Lamp doubling can latch, exactly as Beetle Juice and Powdered
        // Demise already do through `debuff`. `apply_power_monster_debuff`
        // also raises `doom_applied_by_player_this_turn` for Death's Door.
        PotionId::PotionOfDoom => debuff(
            state,
            catalog,
            enemy.expect("validated enemy target"),
            PowerId::Doom,
            MiseryToken::Doom,
            33,
            events,
        )?,
        // PotionShapedRock/<OnUse>d__8::MoveNext RVA `0x34f84c`,
        // OnUse `0xad1e0`.
        //
        // `get_TargetType` (`0xad1c9`) is 2 and the body is one
        // `CreatureCmd::Damage(target, DynamicVars.Damage, owner, null, null)`
        // at IL_004c. `get_CanonicalVars` (`0xad1cc`) is
        // `DamageVar(15, props 4)` — nonCardUnpowered: blockable, no
        // Strength/Vulnerable scaling, no Skittish/PersonalHive. Identical in
        // shape to Fire Potion, only the amount differs.
        PotionId::PotionShapedRock => {
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                enemy.expect("validated enemy target"),
                DotNetDecimal::from_i64(15),
                false,
                true,
                events,
            )?;
        }
        // 0xad2d4/0x34fa3c.
        PotionId::PowderedDemise => debuff(
            state,
            catalog,
            enemy.expect("validated enemy target"),
            PowerId::Demise,
            MiseryToken::Demise,
            9,
            events,
        )?,
        // 0xad3d0/0x34fd30.
        PotionId::RadiantTincture => {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
            if !state.history.over {
                let current = state.fanouts.radiance();
                let next = checked_cold_add(current, 3, "radiance")?;
                state.fanouts.set_radiance(next);
                if current == 0
                    && !state
                        .fanouts
                        .register_after_energy_reset(crate::hot::AfterEnergyResetPower::Radiance)
                {
                    return Err(EngineRefusal::CounterOverflow(
                        "after-energy-reset listener order",
                    ));
                }
                state.normalize_after_energy_reset_order_if_unique();
            }
        }
        // RegenPotion/<OnUse>d__12::MoveNext RVA 0x34febc. PowerCmd.Apply is
        // independently ending-gated; the surrounding wrapper still burns
        // and completes in an ending window.
        PotionId::RegenPotion => {
            if !state.history.over {
                let regen = checked_cold_add(state.fanouts.regen(), 5, "regen")?;
                assert!(state.fanouts.set_regen(regen));
            }
        }
        // 0xad504/0x34ffb8.
        PotionId::ShacklingPotion => {
            let targets = state
                .monsters
                .iter()
                .enumerate()
                .filter(|(_, monster)| monster.hp > 0)
                .map(|(index, monster)| (index, monster.slot, monster.uid))
                .collect::<Vec<_>>();
            for (target, slot, uid) in targets {
                if state.history.over {
                    break;
                }
                if state.monsters.get(target).is_none_or(|monster| {
                    monster.hp <= 0 || (monster.slot, monster.uid) != (slot, uid)
                }) {
                    continue;
                }
                super::damage::apply_potion_temporary_strength(state, target, 7, events)?;
            }
        }
        // 0xad588/0x35011c.
        PotionId::ShipInABottle => {
            super::orbs::gain_flat_decimal_block(
                state,
                catalog,
                DotNetDecimal::from_i64(10),
                events,
            )?;
            if !state.history.over {
                super::damage::apply_block_next_turn(state, 10, events)?;
            }
        }
        // Synchronous body RVA 0xad728.
        PotionId::SoldiersStew => soldiers_stew(state, catalog)?,
        // 0xad824/0x3506bc.
        PotionId::SpeedPotion => super::allies::gain_temp_dexterity(state, 0, 5, events)?,
        // 0xad8a4/0x3507b8.
        PotionId::StableSerum => checked_power_add(state, PowerId::RetainHand, 2, events)?,
        // StarPotion/<OnUse>d__8::MoveNext RVA `0x3508b0`, OnUse `0xad918`.
        //
        // One `PlayerCmd::GainStars(DynamicVars.Stars.BaseValue,
        // target.get_Player)` at IL_0043; `get_CanonicalVars` (`0xad908`) is
        // `StarsVar(3)`. `get_TargetType` (`0xad905`) is 5, and unlike Ghost
        // in a Jar the recipient IS the picked target's Player — which is why
        // Star Potion stays on the solo-player-refusing side above. The
        // `gain_stars` primitive carries the `stars_gained_this_turn` history
        // write and the Black Hole fan-out.
        PotionId::StarPotion => super::play::gain_stars(state, catalog, 3, events)?,
        // 0xad9a8/0x350998.
        PotionId::StrengthPotion => super::damage::apply_owner_strength(state, 2, events)?,
        // 0xadb18/0x350d20.
        PotionId::VulnerablePotion => debuff(
            state,
            catalog,
            enemy.expect("validated enemy target"),
            PowerId::Vuln,
            MiseryToken::Vuln,
            3,
            events,
        )?,
        // 0xadbb0/0x350e3c.
        PotionId::WeakPotion => debuff(
            state,
            catalog,
            enemy.expect("validated enemy target"),
            PowerId::Weak,
            MiseryToken::Weak,
            3,
            events,
        )?,
        // #3229: `PoisonPotion/<OnUse>d__10::MoveNext` RVA `0x34f2c4`, OnUse
        // `0xacfa4`.
        //
        // `get_TargetType` (`0xacf6d`) is 2 — one picked enemy — and the body
        // is a single `PowerCmd::Apply<PoisonPower>(choiceContext, target,
        // DynamicVars.Poison.BaseValue, applier = Owner.Creature, cardSource =
        // null, silent = false)` at IL_0095 (argument loads IL_006c-IL_0094).
        // `get_CanonicalVars` (`0xacf70`) is `PoisonVar(Decimal 6)`. This is
        // the exact call shape of Weak/Vulnerable Potion and Potion of Doom:
        // a player applier with a null card, so it takes the ordinary
        // non-card Type-2 route — the Apply ending gate, Snecko Skull's
        // Poison-only given-side +1, Artifact consumption, then the fresh
        // Poison instance uid on a first application.
        PotionId::PoisonPotion => debuff(
            state,
            catalog,
            enemy.expect("validated enemy target"),
            PowerId::Poison,
            MiseryToken::Poison,
            6,
            events,
        )?,
        // #3229: `PotionOfCapacity/<OnUse>d__8::MoveNext` RVA `0x34f630`,
        // OnUse `0xad0d8`.
        //
        // `get_TargetType` (`0xad0c5`) is 5 and the body is one
        // `OrbCmd::AddSlots(target.Player, DynamicVars.Repeat.IntValue)` at
        // IL_0063; `get_CanonicalVars` (`0xad0c8`) is `RepeatVar(2)`.
        // `OrbCmd::AddSlots` (RVA `0x132f14`) returns at
        // `CombatManager::get_IsOverOrEnding` (IL_0011), clamps the amount to
        // `Math.Min(10 - OrbQueue.Capacity, amount)` (IL_001e-IL_0037) and
        // calls `OrbQueue::AddCapacity` (`0x1184f2`: `Capacity += amount`,
        // nothing else) at IL_0045. Only the live capacity moves; the base
        // slot count is untouched. [`super::orbs::add_slots`] is that clamp
        // (`saturating_add(..).min(MAX_ORB_SLOTS)`), but gates only the
        // `history.over` latch, so the IsEnding half is tested here through
        // the shared projection.
        PotionId::PotionOfCapacity => {
            if !super::damage::damage_combat_is_ending(state) {
                super::orbs::add_slots(state, 2)?;
            }
        }
        // #3229: `EssenceOfDarkness/<OnUse>d__8::MoveNext` RVA `0x34d478`,
        // OnUse `0xac300`.
        //
        // `get_TargetType` (`0xac2d9`) is 5. The body reads
        // `target.Player.PlayerCombatState.OrbQueue.Capacity` ONCE into
        // `<count>5__3` (IL_0037-IL_004b), then loops `i < count`
        // (IL_00bd-IL_00d8) awaiting `OrbCmd::Channel<DarkOrb>` (IL_0065) with
        // no break of its own. Every channel carries its own IsOverOrEnding
        // entry gate (`orbs::channel`, #2669), so a lethal Dark evoke from a
        // full queue turns the remaining iterations into no-ops exactly as
        // natively. A zero-capacity owner reads count 0 and never reaches
        // Channel's one-slot bootstrap.
        PotionId::EssenceOfDarkness => {
            let count = state.orbs.slots();
            for _ in 0..count {
                super::orbs::channel(state, catalog, crate::hot::OrbKind::Dark, events)?;
            }
        }
        // #3229: `CunningPotion/<OnUse>d__10::MoveNext` RVA `0x34cae0`, OnUse
        // `0xabf48`.
        //
        // `get_TargetType` (`0xabf29`) is 5. The body awaits the count
        // overload `Shiv::CreateInHand(target.Player,
        // DynamicVars.Cards.IntValue, target.CombatState, creator =
        // Owner)` at IL_0056 (`get_CanonicalVars` `0xabf2c`: `CardsVar(3)`),
        // then walks the returned enumerable calling `CardCmd::Upgrade(card,
        // 1)` per member (IL_00c4-IL_00da). The count overload is
        // `Shiv/<CreateInHand>d__12::MoveNext` RVA `0x3bb20c` — Storm of
        // Steel's: one IsOverOrEnding gate (IL_002c), the complete list
        // allocated up front (IL_0043-IL_0079), one plural generated Add
        // (IL_0093) whose null-coalesced creator is the owner either way.
        // The single-card `CardCmd::Upgrade` (`0x12f64f`) wraps the card and
        // calls the plural overload (`0x12f660`), whose IsEnding gate
        // (IL_0011) is re-read per member with nothing in between that can
        // move it, so it is the Storm helper's one live-only upgrade pass.
        PotionId::CunningPotion => {
            super::cards::inject_generated_storm_shivs(state, catalog, 3, 1, events)?
        }
        PotionId::CosmicConcoction => cosmic_concoction(state, catalog, events)?,
        // EntropicBrew/<OnUse>d__6::MoveNext RVA 0x34d33c. Its do/while
        // generation loop is intentionally not combat-ending gated.
        PotionId::EntropicBrew => entropic_brew(state, catalog, events)?,
        PotionId::BoneBrew => bone_brew(state)?,
        PotionId::PotOfGhouls => pot_of_ghouls(state, catalog, events)?,
        _ => unreachable!("unsupported potions return before physical removal"),
    }
    if state.pending.is_some()
        || !matches!(state.frames.top(), Some(Frame::PotionFinish { record })
            if state.frames.potion_finish(record).is_some_and(|view|
                view.name == potion && view.stage == PotionFinishStage::Effect))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    finish(state, catalog, potion, events)
}

#[cfg(test)]
mod rehearsal_tests {
    use super::*;
    use crate::catalog::CatalogBuilder;

    #[test]
    fn multiple_distilled_root_bypass_is_exact_and_address_scoped() {
        let mut state = HotState::at_defaults();
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::DistilledChaos),
                Some(PotionId::DistilledChaos),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        let different_address = state.clone();

        {
            let _scope = DistilledRehearsalGuard::enter(&state, 0).unwrap();
            assert!(!consume_distilled_rehearsal_bypass(&different_address));
            assert!(consume_distilled_rehearsal_bypass(&state));
            assert!(consume_distilled_rehearsal_bypass(&state));
            assert!(consume_distilled_rehearsal_bypass(&state));
            assert!(!consume_distilled_rehearsal_bypass(&state));
        }

        // Dropping one rehearsal cannot leave authority for either the old
        // address or a nested/new-address Distilled root.
        assert!(!consume_distilled_rehearsal_bypass(&state));
        assert!(!consume_distilled_rehearsal_bypass(&different_address));
        let _fresh_scope = DistilledRehearsalGuard::enter(&different_address, 1).unwrap();
    }

    #[test]
    fn reptile_trinket_grants_expiring_strength_after_potion_use() {
        let mut builder = CatalogBuilder::new();
        builder.set_relics(&[RelicId::RelicReptileTrinket]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state
            .frames
            .push_potion_finish(&PotionFinishRecord {
                name: PotionId::StrengthPotion,
                stage: PotionFinishStage::Effect,
                body_stage: PotionBodyStage::Synchronous,
                current_uid: None,
                aux: 0,
                candidates: Vec::new(),
                generation_options: Vec::new(),
            })
            .unwrap();

        finish(
            &mut state,
            &catalog,
            PotionId::StrengthPotion,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.temp_strength, 3);
        assert!(
            state
                .fanouts
                .after_side_turn_end_power_uid(
                    crate::hot::AfterSideTurnEndPowerToken::TemporaryStrength
                )
                .is_some()
        );
    }
}

#[cfg(test)]
mod generation_owner_tests {
    use super::*;
    use crate::catalog::{CatalogBuilder, RewardPool};

    #[test]
    fn every_owner_gets_its_native_typed_pools_and_generation_draw_counts() {
        for (owner, counts) in [
            (RewardPool::Ironclad, [33, 27, 18]),
            (RewardPool::Silent, [23, 39, 16]),
            (RewardPool::Regent, [29, 34, 16]),
            (RewardPool::Necrobinder, [32, 29, 17]),
            (RewardPool::Defect, [28, 33, 19]),
        ] {
            let mut builder = CatalogBuilder::new();
            let pools = [
                PotionId::AttackPotion,
                PotionId::SkillPotion,
                PotionId::PowerPotion,
            ]
            .map(|potion| generation_choice_pool(potion, Some(owner), None).unwrap());
            assert_eq!(pools.each_ref().map(Vec::len), counts);
            for id in pools.iter().flatten() {
                builder
                    .intern(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            let catalog = builder.build();
            for (potion, count) in [
                PotionId::AttackPotion,
                PotionId::SkillPotion,
                PotionId::PowerPotion,
            ]
            .into_iter()
            .zip(counts)
            {
                let mut state = HotState::at_defaults();
                state.reward_card_pool = Some(owner);
                state.fully_unlocked_card_pool_epochs = true;
                state.rng.set(
                    RngStream::Generation,
                    RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
                state.hp = 50;
                state.max_hp = 50;
                state.exact_piles = true;
                state.player_phase = super::super::admission::PHASE_ORDINARY_ACTIONS;
                state.next_card_uid = 1;
                assert!(state.fanouts.set_potion_belt(
                    vec![Some(potion)],
                    false,
                    false,
                    false,
                    false,
                    true
                ));
                state = super::super::apply_action(
                    &state,
                    &catalog,
                    &super::super::Action::UsePotion {
                        slot: 0,
                        target: None,
                    },
                )
                .unwrap()
                .state;
                assert_eq!(
                    state.rng.get(RngStream::Generation).counter,
                    count as u64 - 1
                );
                assert!(generation_pending_is_exact(&state, &catalog));
                // A pending screen cannot be reinterpreted as another owner's
                // generated cards after a forged boundary owner change.
                state.reward_card_pool = Some(if owner == RewardPool::Ironclad {
                    RewardPool::Necrobinder
                } else {
                    RewardPool::Ironclad
                });
                assert!(!generation_pending_is_exact(&state, &catalog));
            }
            let mut overflow = HotState::at_defaults();
            overflow.reward_card_pool = Some(owner);
            assert_eq!(
                orobic_rng_draws(&overflow, &catalog).unwrap(),
                counts.into_iter().sum::<usize>() - 3
            );
            overflow.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: u64::MAX - (counts.into_iter().sum::<usize>() - 4) as u64,
                },
            );
            assert!(
                preflight_manual_potion_rng(&overflow, &catalog, PotionId::OrobicAcid).is_err()
            );
        }
        assert!(generation_choice_pool(PotionId::SkillPotion, None, None).is_none());
    }

    /// At the fully-unlocked profile the IL-derived pools (#3141) are exactly
    /// what this function returned before: the owner's `Common..Rare`
    /// one-type projection and `ENTROPY_COLORLESS_TRANSFORM_POOL_V109`. So
    /// moving the character pools to `FilterForCombat`'s rarity predicate
    /// (not Basic/Ancient/Event) and the Colorless pool to Quasar's
    /// derivation changes no fully-unlocked fight.
    #[test]
    fn generation_potion_pools_at_the_full_profile_are_the_frozen_projections() {
        use crate::content_tables::CardType;
        use crate::steps::neutral::{PoolRarity, derive_character_generation_pool};
        for owner in [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Regent,
            RewardPool::Necrobinder,
            RewardPool::Defect,
        ] {
            for (potion, kind) in [
                (PotionId::AttackPotion, CardType::Attack),
                (PotionId::SkillPotion, CardType::Skill),
                (PotionId::PowerPotion, CardType::Power),
            ] {
                assert_eq!(
                    generation_choice_pool(potion, Some(owner), None).unwrap(),
                    derive_character_generation_pool(
                        owner,
                        None,
                        PoolRarity::CommonUncommonRare,
                        Some(kind),
                        false
                    ),
                    "{owner:?} {potion:?}"
                );
            }
            assert_eq!(
                generation_choice_pool(PotionId::ColorlessPotion, Some(owner), None).unwrap()[..],
                crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109[..]
            );
        }
        // The Colorless body reads no owner class (`0x34c784` IL_003f), so
        // its pool needs none; the gate still requires a reward owner.
        assert!(generation_choice_pool(PotionId::ColorlessPotion, None, None).is_some());
        assert!(
            generation_choice_pool(PotionId::BlockPotion, Some(RewardPool::Defect), None).is_none()
        );
    }

    /// Both arms of the Part D admission name (#3141): a named `MalformedArgs`
    /// refusal is reported by its own name, anything else by the bundled one.
    #[test]
    fn part_d_admission_names_the_refusing_prerequisite() {
        use super::super::admission::part_d_prerequisite_refusal_name;
        assert_eq!(
            part_d_prerequisite_refusal_name(&EngineRefusal::MalformedArgs(
                "generation potion pool provenance"
            )),
            "generation potion pool provenance"
        );
        assert_eq!(
            part_d_prerequisite_refusal_name(&EngineRefusal::ContinuationNotModeled),
            "Part D potion prerequisites"
        );
        assert_eq!(
            part_d_prerequisite_refusal_name(&EngineRefusal::CounterOverflow("potion RNG")),
            "Part D potion prerequisites"
        );
        // A name raised deeper than the Part D checker itself — here one that
        // is also an unrelated admission gate's name — stays bundled.
        assert_eq!(
            part_d_prerequisite_refusal_name(&EngineRefusal::MalformedArgs(
                "potion AnyAlly target roster"
            )),
            "Part D potion prerequisites"
        );
        for own in [
            "Necrobinder potion player target",
            "Entropic generation-potion provenance",
            "Orobic generation provenance",
            "Entropic Brew generation provenance",
            "Fruit Juice hp domain",
            "Regen Potion amount/order",
            "Fairy AfterPotionUsed Belt Buckle",
            "Distilled prospective recursive source closure",
        ] {
            assert_eq!(
                part_d_prerequisite_refusal_name(&EngineRefusal::MalformedArgs(own)),
                own
            );
        }
    }
    #[test]
    fn native_alchemize_captures_match_factory_results_and_rng() {
        let captures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../eval/search/ea6-admission-v1/alchemize-native-factory.json"
        ))
        .unwrap();
        for capture in captures.as_array().unwrap() {
            let mut state = HotState::at_defaults();
            state.reward_card_pool = Some(RewardPool::Necrobinder);
            let before: RngStreamState = RngStreamState {
                words: serde_json::from_value(capture["before"]["words"].clone()).unwrap(),
                counter: capture["before"]["counter"].as_u64().unwrap(),
            };
            state.rng.set(RngStream::PotionGeneration, before);
            let generated = random_potion_from_factory(&mut state, None, true).unwrap();
            assert_eq!(generated.as_str(), capture["generated"].as_str().unwrap());
            let after = state.rng.get(RngStream::PotionGeneration);
            assert_eq!(serde_json::json!(after.words), capture["after"]["words"]);
            assert_eq!(
                serde_json::json!(after.counter),
                capture["after"]["counter"]
            );
        }
    }

    #[test]
    fn necrobinder_factory_replaces_only_character_rows_and_keeps_native_rarity_order() {
        for (roll, native) in [
            (0.0, PotionId::PotOfGhouls),
            (0.2, PotionId::BoneBrew),
            (0.9, PotionId::PotionOfDoom),
        ] {
            for in_combat in [false, true] {
                let necro = owner_random_potion_pool_for_roll(
                    Some(RewardPool::Necrobinder),
                    None,
                    roll,
                    in_combat,
                );
                let ironclad = owner_random_potion_pool_for_roll(
                    Some(RewardPool::Ironclad),
                    None,
                    roll,
                    in_combat,
                );
                assert_eq!(necro[0], native);
                assert_eq!(necro[1..], ironclad[1..]);
            }
        }
        for owner in RewardPool::ALL {
            assert_eq!(factory_pool(Some(owner)).len(), 48);
            assert_eq!(
                factory_pool(Some(owner))
                    .iter()
                    .filter(|(_, _, c)| *c)
                    .count(),
                45
            );
        }
        assert!(!factory_can_generate(
            Some(RewardPool::Necrobinder),
            PotionId::BloodPotion
        ));
        assert!(!factory_can_generate(
            Some(RewardPool::Necrobinder),
            PotionId::SoldiersStew
        ));
        assert!(!factory_can_generate(
            Some(RewardPool::Necrobinder),
            PotionId::Ashwater
        ));
    }

    fn necro_potion_fixture(potion: PotionId, hand_size: usize) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        let soul = CardIdentity {
            id: CardId::Soul,
            upgrade: 0,
            enchantment: None,
        };
        builder.intern_reachable(soul).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.exact_piles = true;
        state.player_phase = super::super::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 1;
        super::super::cards::inject_generated_bottom(
            &mut state,
            &catalog,
            soul,
            hand_size,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(potion)],
            false,
            false,
            false,
            false,
            true
        ));
        (state, catalog)
    }

    #[test]
    fn bone_brew_creates_or_grows_osty_without_rng_and_finishes_wrapper() {
        // A live enemy keeps the combat from ending, so the summon heals
        // (#3246); with none left, a living Osty only gains MaxHp.
        for (live_enemy, old, expected) in [
            (true, None, (15, 15)),
            (true, Some((3, 8)), (18, 23)),
            (false, Some((3, 8)), (3, 23)),
        ] {
            let (mut state, catalog) = necro_potion_fixture(PotionId::BoneBrew, 0);
            if live_enemy {
                state.monsters_mut().push(crate::hot::HotMonster::new(
                    crate::ids::MonsterKind::Toadpole,
                    50,
                ));
            }
            state.fanouts.set_osty(old).unwrap();
            let rng = state.rng.clone();
            let after = super::super::apply_action(
                &state,
                &catalog,
                &super::super::Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            let osty = after.fanouts.pet().osty().unwrap();
            assert_eq!((osty.hp(), osty.max_hp()), expected);
            assert_eq!(after.rng, rng);
            assert!(after.frames.is_empty());
            assert_eq!(after.fanouts.potion_slots(), &[None]);
        }
        let (mut state, catalog) = necro_potion_fixture(PotionId::BoneBrew, 0);
        state.fanouts.set_osty(Some((1, i32::MAX))).unwrap();
        let before = state.clone();
        assert!(
            super::super::apply_action(
                &state,
                &catalog,
                &super::super::Action::UsePotion {
                    slot: 0,
                    target: None
                }
            )
            .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn pot_of_ghouls_generates_two_souls_with_hand_overflow_history_and_hooks() {
        for hand_size in [0, 9, 10] {
            let (mut state, catalog) = necro_potion_fixture(PotionId::PotOfGhouls, hand_size);
            state.fanouts.set_osty(Some((2, 5))).unwrap();
            state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
            state
                .powers
                .set(PowerId::PillarOfCreation, SlotWire::Int, 2);
            assert!(
                state.fanouts.set_local_generated_power_order(&[
                    PowerId::Arsenal,
                    PowerId::PillarOfCreation
                ])
            );
            let old_generated = state.history.owner_generated_cards_combat;
            let after = super::super::apply_action(
                &state,
                &catalog,
                &super::super::Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            assert_eq!(after.piles.get(PileId::Hand).len(), (hand_size + 2).min(10));
            assert_eq!(
                after.piles.get(PileId::Discard).len(),
                (hand_size + 2).saturating_sub(10)
            );
            assert_eq!(
                after.history.owner_generated_cards_combat,
                old_generated + 2
            );
            assert_eq!(after.powers.value(PowerId::Strength), 2);
            assert_eq!(after.block, 4);
            assert_eq!(after.rng, state.rng);
            assert!(after.frames.is_empty());
            assert_eq!(after.fanouts.potion_slots(), &[None]);
        }
    }
}

/// #3044: every potion heal is a `CreatureCmd.Heal` whose
/// `AfterCurrentHpChanged` reaches Red Skull.
#[cfg(test)]
mod red_skull_heal_tests {
    use super::*;
    use crate::catalog::CatalogBuilder;
    use crate::hot::HotMonster;
    use crate::ids::MonsterKind;

    fn skull_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.set_relics(&[RelicId::RelicRedSkull]).unwrap();
        builder.build()
    }

    fn skull(potion: PotionId, hp: i32, max_hp: i32, strength: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = hp;
        state.max_hp = max_hp;
        state.fanouts.set_red_skull_owned(true);
        // Through the owner command, so `hot_path_contract`'s Strength
        // writer census (which scans this file whole) sees no new site.
        super::super::damage::apply_owner_strength(&mut state, strength, &mut Vec::new()).unwrap();
        state.exact_piles = true;
        state.player_phase = super::super::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 1;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(potion)],
            false,
            false,
            false,
            false,
            true
        ));
        state
    }

    fn drink(potion: PotionId, hp: i32, max_hp: i32, strength: i32) -> HotState {
        let state = skull(potion, hp, max_hp, strength);
        super::super::apply_action(
            &state,
            &skull_catalog(),
            &super::super::Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state
    }

    #[test]
    fn a_blood_potion_heal_past_half_removes_the_strength() {
        let healed = drink(PotionId::BloodPotion, 30, 80, 3);
        assert_eq!((healed.hp, healed.powers.value(PowerId::Strength)), (46, 0));
        let short = drink(PotionId::BloodPotion, 20, 80, 3);
        assert_eq!((short.hp, short.powers.value(PowerId::Strength)), (36, 3));
    }

    /// `GainMaxHp` (`0x3eb2f0`) is `SetMaxHp` then `Heal`: Red Skull reads
    /// both new values.
    #[test]
    fn fruit_juice_compares_the_new_hp_with_the_new_max_hp() {
        let lifted = drink(PotionId::FruitJuice, 25, 50, 3);
        assert_eq!(
            (
                lifted.hp,
                lifted.max_hp,
                lifted.powers.value(PowerId::Strength)
            ),
            (30, 55, 0)
        );
        let held = drink(PotionId::FruitJuice, 20, 50, 3);
        assert_eq!(
            (held.hp, held.max_hp, held.powers.value(PowerId::Strength)),
            (25, 55, 3)
        );
    }

    /// Fairy heals 30% of max HP from zero, which leaves the threshold only
    /// at max HP 1.
    #[test]
    fn both_fairy_paths_are_heals_red_skull_hears() {
        let mut direct = skull(PotionId::FairyInABottle, 0, 1, 3);
        assert!(consume_first_fairy_after_lethal(&mut direct, &mut Vec::new()).unwrap());
        assert_eq!((direct.hp, direct.powers.value(PowerId::Strength)), (1, 0));

        let mut wrapped = skull(PotionId::FairyInABottle, 0, 1, 3);
        assert_eq!(
            begin_fairy_wrapper_after_lethal(&mut wrapped, &skull_catalog(), &mut Vec::new())
                .unwrap(),
            FairyWrapperResult::Finished
        );
        assert_eq!(
            (wrapped.hp, wrapped.powers.value(PowerId::Strength)),
            (1, 0)
        );

        let mut ordinary = skull(PotionId::FairyInABottle, 0, 80, 3);
        assert!(consume_first_fairy_after_lethal(&mut ordinary, &mut Vec::new()).unwrap());
        assert_eq!(
            (ordinary.hp, ordinary.powers.value(PowerId::Strength)),
            (24, 3)
        );
    }
}

/// #3343: Alchemize and Entropic Brew draw the OWNER's potion pool under the
/// owner's recorded profile, not the solo-Ironclad `fully_unlocked_potion_pool`
/// flag. See [`generation_potion_profile`] for the IL.
#[cfg(test)]
mod issue3343_owner_potion_pool_tests {
    use super::*;
    use crate::catalog::{CatalogBuilder, RewardPool};
    use crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101;

    /// Each owner's three own potions and the fourth epoch gating them, read
    /// independently of the generated table: `<Character>4Epoch::get_Potions`
    /// and the `IsEpochRevealed` test in each `GetUnlockedPotions`
    /// (Ironclad `0xadd38`, Silent `0xae088`, Defect `0xadca8`, Necrobinder
    /// `0xadd98`, Regent `0xaddd8`).
    const OWN: [(RewardPool, &str, [PotionId; 3]); 5] = [
        (
            RewardPool::Ironclad,
            "IRONCLAD4_EPOCH",
            [
                PotionId::BloodPotion,
                PotionId::SoldiersStew,
                PotionId::Ashwater,
            ],
        ),
        (
            RewardPool::Silent,
            "SILENT4_EPOCH",
            [
                PotionId::PoisonPotion,
                PotionId::GhostInAJar,
                PotionId::CunningPotion,
            ],
        ),
        (
            RewardPool::Defect,
            "DEFECT4_EPOCH",
            [
                PotionId::FocusPotion,
                PotionId::EssenceOfDarkness,
                PotionId::PotionOfCapacity,
            ],
        ),
        (
            RewardPool::Necrobinder,
            "NECROBINDER4_EPOCH",
            [
                PotionId::PotionOfDoom,
                PotionId::PotOfGhouls,
                PotionId::BoneBrew,
            ],
        ),
        (
            RewardPool::Regent,
            "REGENT4_EPOCH",
            [
                PotionId::StarPotion,
                PotionId::CosmicConcoction,
                PotionId::KingsCourage,
            ],
        ),
    ];
    /// `Potion1Epoch.Potions` and `Potion2Epoch.Potions`, which
    /// `SharedPotionPool::GetUnlockedPotions` `0xadfb4` removes when hidden.
    const POTION1: [PotionId; 3] = [
        PotionId::BeetleJuice,
        PotionId::DropletOfPrecognition,
        PotionId::MazalethsGift,
    ];
    const POTION2: [PotionId; 3] = [
        PotionId::PowderedDemise,
        PotionId::ShipInABottle,
        PotionId::TouchOfInsanity,
    ];
    /// One roll per rarity: rare, uncommon, common.
    const ROLLS: [f32; 3] = [0.0, 0.2, 0.9];

    fn profile_catalog(epochs: Option<Vec<&'static str>>) -> Catalog {
        let mut builder = CatalogBuilder::new();
        if let Some(epochs) = epochs {
            builder.set_wire_unlock_epochs(epochs);
        }
        builder.build()
    }

    fn universe_without(hidden: &[&str]) -> Vec<&'static str> {
        UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .copied()
            .filter(|epoch| !hidden.contains(epoch))
            .collect()
    }

    fn owner_state(owner: Option<RewardPool>, flag: bool) -> HotState {
        let mut state = HotState::at_defaults();
        state.reward_card_pool = owner;
        state.rng.set(
            RngStream::PotionGeneration,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, flag)
        );
        state
    }

    fn recorded<'a>(epochs: &'a [&'static str], flag: bool) -> Option<GenerationPotionProfile<'a>> {
        Some(GenerationPotionProfile {
            recorded: Some(epochs),
            flag,
        })
    }

    fn pool(
        owner: RewardPool,
        profile: Option<GenerationPotionProfile<'_>>,
        roll: f32,
        in_combat: bool,
    ) -> Vec<PotionId> {
        owner_random_potion_pool_for_roll(Some(owner), profile, roll, in_combat)
    }

    /// Every branch of [`generation_potion_profile`]'s decision.
    #[test]
    fn the_flag_decides_only_the_ironclad_pool_and_a_recorded_profile_decides_every_pool() {
        let bare = profile_catalog(None);
        // The flag alone: the Ironclad owner and the absent owner (which the
        // factory reads as Ironclad) decide; no other owner does.
        for owner in [None, Some(RewardPool::Ironclad)] {
            assert_eq!(
                generation_potion_profile(&owner_state(owner, true), &bare),
                Some(GenerationPotionProfile {
                    recorded: None,
                    flag: true
                })
            );
        }
        for owner in [
            RewardPool::Silent,
            RewardPool::Defect,
            RewardPool::Necrobinder,
            RewardPool::Regent,
        ] {
            assert_eq!(
                generation_potion_profile(&owner_state(Some(owner), true), &bare),
                None,
                "{owner:?}: IRONCLAD4 + POTION1/2 does not decide this pool"
            );
        }
        // Neither proof.
        for owner in RewardPool::ALL {
            assert_eq!(
                generation_potion_profile(&owner_state(Some(owner), false), &bare),
                None
            );
        }
        // A recorded profile decides every owner's pool, with or without the
        // flag, and even when it reveals nothing.
        for epochs in [vec![], universe_without(&["DEFECT4_EPOCH"])] {
            let catalog = profile_catalog(Some(epochs));
            for owner in RewardPool::ALL {
                for flag in [false, true] {
                    let profile =
                        generation_potion_profile(&owner_state(Some(owner), flag), &catalog)
                            .expect("a recorded profile decides the pool");
                    assert_eq!(profile.recorded, catalog.wire_unlock_epochs());
                    assert_eq!(profile.flag, flag);
                }
            }
        }
    }

    /// The owner's own rows follow the owner's fourth epoch alone, and the
    /// shared rows follow `POTION1/2`.
    #[test]
    fn a_hidden_epoch_removes_exactly_the_rows_it_gates() {
        for (owner, own_epoch, own) in OWN {
            for roll in ROLLS {
                for in_combat in [false, true] {
                    let full = pool(owner, None, roll, in_combat);
                    let under = |hidden: &[&str]| {
                        let epochs = universe_without(hidden);
                        pool(owner, recorded(&epochs, false), roll, in_combat)
                    };
                    let without = |gated: &[PotionId]| {
                        full.iter()
                            .copied()
                            .filter(|potion| !gated.contains(potion))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(under(&[]), full, "{owner:?}");
                    assert_eq!(under(&[own_epoch]), without(&own), "{owner:?}");
                    assert_eq!(under(&["POTION1_EPOCH"]), without(&POTION1));
                    assert_eq!(under(&["POTION2_EPOCH"]), without(&POTION2));
                    // Another owner's fourth epoch moves nothing.
                    for (other, other_epoch, _) in OWN {
                        if other != owner {
                            assert_eq!(under(&[other_epoch]), full, "{owner:?}/{other:?}");
                        }
                    }
                }
            }
            // The owner's three potions are in the full projection (some
            // rarity) and absent from every rarity without the epoch.
            let everywhere = |hidden: &[&str]| {
                let epochs = universe_without(hidden);
                ROLLS
                    .into_iter()
                    .flat_map(|roll| pool(owner, recorded(&epochs, false), roll, false))
                    .collect::<Vec<_>>()
            };
            assert!(own.iter().all(|potion| everywhere(&[]).contains(potion)));
            assert!(
                own.iter()
                    .all(|potion| !everywhere(&[own_epoch]).contains(potion))
            );
        }
    }

    /// The flag's positive proof is unioned with the recorded profile, so a
    /// document pairing the flag with a profile that omits `POTION1/2` (the
    /// canonical card-pool set a synthetic state projects) keeps those rows.
    /// The flag says nothing about another owner's fourth epoch.
    #[test]
    fn the_flag_and_the_recorded_profile_are_unioned() {
        let card_pool_only = universe_without(&["POTION1_EPOCH", "POTION2_EPOCH"]);
        let defect_hidden = universe_without(&["DEFECT4_EPOCH"]);
        for roll in ROLLS {
            for in_combat in [false, true] {
                let full = pool(RewardPool::Ironclad, None, roll, in_combat);
                assert_eq!(
                    pool(
                        RewardPool::Ironclad,
                        recorded(&card_pool_only, true),
                        roll,
                        in_combat
                    ),
                    full
                );
                assert_eq!(
                    pool(
                        RewardPool::Ironclad,
                        Some(GenerationPotionProfile {
                            recorded: None,
                            flag: true,
                        }),
                        roll,
                        in_combat
                    ),
                    full
                );
                assert!(
                    pool(
                        RewardPool::Defect,
                        recorded(&defect_hidden, true),
                        roll,
                        in_combat
                    )
                    .iter()
                    .all(|potion| !OWN[2].2.contains(potion))
                );
            }
        }
    }

    /// The empty-bucket refusal in [`random_potion_from_factory`] is
    /// unreachable on this build's tables: even a profile revealing nothing
    /// leaves every (owner, rarity, in-combat) bucket non-empty. Pinned so a
    /// table change that empties one surfaces here rather than as a refusal.
    #[test]
    fn a_profile_revealing_nothing_leaves_every_rarity_bucket_nonempty() {
        let nothing: Vec<&'static str> = Vec::new();
        for owner in RewardPool::ALL {
            for roll in ROLLS {
                for in_combat in [false, true] {
                    assert!(
                        !pool(owner, recorded(&nothing, false), roll, in_combat).is_empty(),
                        "{owner:?} {roll} {in_combat}"
                    );
                }
            }
        }
    }

    /// End to end through both bodies: a Defect owner whose profile hides
    /// `DEFECT4_EPOCH` (the 6P96T755CNZ3 captures' shape) never generates a
    /// Defect potion, where the fully revealed control does; and the flag
    /// alone no longer admits a Defect owner.
    #[test]
    fn alchemize_and_entropic_brew_draw_the_defect_owners_recorded_pool() {
        let defect_hidden = profile_catalog(Some(universe_without(&["DEFECT4_EPOCH"])));
        let revealed = profile_catalog(Some(universe_without(&[])));
        let bare = profile_catalog(None);
        let defect_own = OWN[2].2;
        let seeded = |seed: u64| {
            let mut state = owner_state(Some(RewardPool::Defect), true);
            let rng = crate::rng::Xoshiro256StarStar::from_seed(seed);
            state.rng.set(
                RngStream::PotionGeneration,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            state
        };
        let mut control_hits = 0usize;
        for seed in 0..512 {
            for (is_control, catalog) in [(false, &defect_hidden), (true, &revealed)] {
                let mut alchemized = seeded(seed);
                alchemize_one_attempt(&mut alchemized, catalog, &mut Vec::new()).unwrap();
                let mut brewed = seeded(seed);
                entropic_brew(&mut brewed, catalog, &mut Vec::new()).unwrap();
                for state in [&alchemized, &brewed] {
                    let generated = state.fanouts.potion_slots()[0].expect("the slot filled");
                    // Each body consumed exactly its rarity and NextItem draws.
                    assert_eq!(state.rng.get(RngStream::PotionGeneration).counter, 2);
                    if defect_own.contains(&generated) {
                        assert!(is_control, "seed {seed}: {generated:?} with DEFECT4 hidden");
                        control_hits += 1;
                    }
                }
            }
        }
        assert!(
            control_hits > 0,
            "the revealed control generates Defect potions"
        );

        let mut state = seeded(0);
        assert_eq!(
            alchemize_one_attempt(&mut state, &bare, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Alchemize generation provenance"
            ))
        );
        assert_eq!(
            entropic_brew(&mut state, &bare, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Entropic Brew generation provenance"
            ))
        );
        assert_eq!(state, seeded(0), "a refusal leaves the state untouched");
    }
}
