//! Draw and the stable reshuffle (`draw_cards` (frozen Python, deleted #2827), `_draw_one_iteration`).
//!
//! The reshuffle is the one place in the slice where the port's RNG parity is
//! load-bearing, and it has three parts that must all be exact:
//!
//! 1. **The sort.** `dotnet_list_sort(list(s.discard), key=sort_key)` — .NET
//!    `List.Sort()`'s introsort over the live discard order, with
//!    `CardModel.CompareTo`'s key `(ModelId entry, upgrade)`. Ties return 0,
//!    and .NET's introsort is *not* stable, so the surviving order of equal
//!    keys is a function of the algorithm, which is why
//!    [`crate::dotnet_sort`] is a swap-for-swap port rather than a call to
//!    `sort_by_key`.
//! 2. **The key.** `sort_key(card) = card[:2]` compares the id string
//!    ordinally, then the upgrade level. The generated [`CardId`] discriminants
//!    are assigned in ascending name order (`ids::tests::names_are_sorted_and_unique`),
//!    so `(id as u16, upgrade)` is an order-preserving encoding of that
//!    comparison — an interned two-integer key, never a cloned string (D2;
//!    the v0.110.1 Colony reshuffle's owned-text key per element per sort
//!    is the anti-pattern this rule exists for).
//! 3. **The shuffle.** `Rng.shuffle` is Fisher-Yates from the top: for
//!    `i = n-1 .. 1`, swap `i` with `next_int(i + 1)`. It consumes exactly
//!    `n - 1` draws, and the counter must match Python's after every
//!    reshuffle or the differential diverges on the *next* one.

use crate::hot::{
    AfterCardDrawnPowerRecord, AfterCardExhaustedPowerRecord, AfterCardExhaustedReturnKind,
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, DrawCaller, DrawEntry, DrawRecord, DrawStage, HotCard,
    HotPile, HotState, LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, PileId,
    RngStream, RngStreamState,
};
use crate::ids::{CardId, EnchantmentId, PowerId, RelicId};
use crate::rng::Xoshiro256StarStar;
use crate::{decimal::DotNetDecimal, hot::MiseryToken};
use std::cell::Cell;

use crate::catalog::{CardIdentity, Catalog};

use crate::hooks::HookEvent;
use crate::powers::SlotWire;

use super::{EngineRefusal, Event, fire_hook};

thread_local! {
    /// Execution-only guard for Python's `_preflight` flag. One outer clone
    /// traverses the complete deterministic nested Hellraiser chain; nested
    /// Pommel/Minion draws reuse that proof instead of cloning exponentially.
    static HELLRAISER_REHEARSAL_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static DAMPEN_DRAW_REHEARSAL_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

struct HellraiserRehearsalGuard;

impl HellraiserRehearsalGuard {
    fn enter() -> Option<Self> {
        HELLRAISER_REHEARSAL_ACTIVE.with(|active| (!active.replace(true)).then_some(Self))
    }
}

impl Drop for HellraiserRehearsalGuard {
    fn drop(&mut self) {
        HELLRAISER_REHEARSAL_ACTIVE.with(|active| active.set(false));
    }
}

struct DampenDrawRehearsalGuard;

impl DampenDrawRehearsalGuard {
    fn enter() -> Option<Self> {
        DAMPEN_DRAW_REHEARSAL_ACTIVE.with(|active| (!active.replace(true)).then_some(Self))
    }
}

impl Drop for DampenDrawRehearsalGuard {
    fn drop(&mut self) {
        DAMPEN_DRAW_REHEARSAL_ACTIVE.with(|active| active.set(false));
    }
}

/// DrawInternal 0x3e3a70 retains the original CardModel before its awaited
/// AfterCardDrawn walk. RemoveFromState 0x7dbb2 removes membership, not the
/// object or owner. Only a Hellraiser DUPE Strike's modeled Remove result can
/// inhabit this retained quotient; general pile readers must remain live-only.
pub(crate) fn drawn_object_card(state: &HotState, uid: u32) -> Result<HotCard, EngineRefusal> {
    let live = super::play::unique_live_card_location(state, uid)?;
    let removed = state.card_states.removed_draw_object(uid);
    match (live, removed) {
        (Some((pile, index)), None) => Ok(state.piles.get(pile).as_slice()[index]),
        (None, Some(entry)) => Ok(entry.card),
        _ => Err(EngineRefusal::ContinuationNotModeled),
    }
}

/// Resolve the same mutable physical instance for every occurrence in a Draw
/// result. The UID denotes one object even if Draw returned it more than once.
/// Consumers must use this interface rather than the live-only sparse census.
pub(crate) fn drawn_object_instance(
    state: &HotState,
    uid: u32,
) -> Result<crate::hot::CardInstanceState, EngineRefusal> {
    drawn_object_card(state, uid)?;
    Ok(state
        .card_states
        .removed_draw_object(uid)
        .map_or_else(|| state.card_states.get(uid), |entry| entry.state.clone()))
}

/// Write the retained object without recreating membership or writing through
/// a live-only getter to an absent/replaced UID. Identity is checked first.
pub(crate) fn set_drawn_object_instance(
    state: &mut HotState,
    uid: u32,
    instance: crate::hot::CardInstanceState,
) -> Result<(), EngineRefusal> {
    drawn_object_card(state, uid)?;
    if let Some(entry) = state.card_states.removed_draw_object_mut(uid) {
        entry.state = instance;
    } else {
        state.card_states.set(uid, instance);
    }
    Ok(())
}

fn set_drawn_flags(state: &mut HotState, uid: u32, flags: u16) -> Result<(), EngineRefusal> {
    drawn_object_card(state, uid)?;
    if let Some(entry) = state.card_states.removed_draw_object_mut(uid) {
        entry.card.flags |= flags;
    } else {
        let (pile, index) = super::play::unique_live_card_location(state, uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        state.piles.get_mut(pile).make_mut()[index].flags |= flags;
    }
    Ok(())
}

pub(crate) fn retain_hellraiser_removed_object(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    instance: crate::hot::CardInstanceState,
    source: crate::hot::CardPlaySource,
) -> Result<(), EngineRefusal> {
    if source != crate::hot::CardPlaySource::Hellraiser {
        return Ok(());
    }
    let spec = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    if card.uid == crate::hot::LEGACY_CARD_UID
        || card.flags & crate::hot::CARD_FLAG_DUPE == 0
        || !spec.strike_tag
        || !spec.is_attack
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
        || super::play::unique_live_card_location(state, card.uid)?.is_some()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    state
        .card_states
        .retain_removed_draw_object(crate::hot::FrozenAutoBatchEntry {
            card,
            state: instance,
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

/// Only parked Draw references may keep a removed object beyond the current
/// public action. Local return vectors remain valid until this publication
/// boundary, including the interval after pop_draw and before the body suffix.
pub(crate) fn publish_removed_draw_objects(state: &mut HotState) -> Result<(), EngineRefusal> {
    if state.card_states.removed_draw_objects().is_empty() {
        return Ok(());
    }
    let mut retained = Vec::new();
    for frame in state.frames.as_slice() {
        if let crate::frame::Frame::Draw { record } = *frame {
            let draw = state
                .frames
                .draw(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            retained.extend(draw.drawn().filter_map(|entry| match entry {
                DrawEntry::Uid(card) => Some(card.uid),
                DrawEntry::Payload(_) => None,
            }));
        }
    }
    state.card_states.prune_removed_draw_objects(&retained);
    Ok(())
}

pub(crate) fn removed_draw_objects_are_exact(state: &HotState, catalog: &Catalog) -> bool {
    if state.card_states.removed_draw_objects().is_empty() {
        return true;
    }
    matches!(state.frames.as_slice().first(), Some(crate::frame::Frame::ActionReplay { .. }))
        && state.pending.is_some()
        && state.card_states.removed_draw_objects().iter().all(|entry| {
            let card = entry.card;
            card.uid < state.next_card_uid && card.uid != crate::hot::LEGACY_CARD_UID
                && card.flags & crate::hot::CARD_FLAG_DUPE != 0
                && catalog.spec(card.atom).is_some_and(|spec| spec.strike_tag && spec.is_attack
                    && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) == Some(spec.row))
                && super::play::unique_live_card_location(state, card.uid).is_ok_and(|live| live.is_none())
                && state.frames.as_slice().iter().any(|frame| match *frame {
                    crate::frame::Frame::Draw { record } => state.frames.draw(record).is_some_and(|draw|
                        draw.drawn().any(|item| matches!(item, DrawEntry::Uid(found) if found.uid == card.uid))),
                    _ => false,
                })
        })
}

fn execute_hellraiser_after_card_drawn_early(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.powers.value(PowerId::Hellraiser) <= 0 {
        return Ok(());
    }
    let card = drawn_object_card(state, uid)?;
    let spec = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    if !spec.strike_tag {
        return Ok(());
    }
    super::play::autoplay_hellraiser_strike(state, catalog, card, events)
}

/// HellraiserPower's sole current-build `AfterCardDrawnEarly` override.
///
/// Current `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Hook::AfterCardDrawn` RVA `0x415614` creates the early enumerator;
/// Hellraiser's body RVA `0x385778` retains the exact
/// drawn Strike across one awaited null-target `CardCmd::AutoPlay`. Python
/// `_hellraiser_after_card_drawn_early` (frozen, deleted #2827) cold-rehearses the outer
/// chain and keeps an active-uid latch until the child completes. The bounded
/// Rust quotient admits no selecting Strike and no infinite-HP display, so
/// the awaited child is synchronous and the existing CardPlay active stack is
/// the complete latch carrier.
fn hellraiser_after_card_drawn_early(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.powers.value(PowerId::Hellraiser) <= 0 {
        return Ok(());
    }
    if let Some(_guard) = HellraiserRehearsalGuard::enter() {
        let mut probe = state.clone();
        super::play::rehearse_preserving_active_plays(|| {
            execute_hellraiser_after_card_drawn_early(&mut probe, catalog, uid, &mut Vec::new())
        })?;
        execute_hellraiser_after_card_drawn_early(state, catalog, uid, events)
    } else {
        execute_hellraiser_after_card_drawn_early(state, catalog, uid, events)
    }
}

fn hellraiser_after_card_drawn_early_resumable(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.powers.value(PowerId::Hellraiser) <= 0 {
        return Ok(());
    }
    let card = drawn_object_card(state, uid)?;
    if !catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?
        .strike_tag
    {
        return Ok(());
    }
    super::play::autoplay_hellraiser_strike_resumable(state, catalog, card, events)
}

fn powers_after_card_drawn(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    source: DrawSource,
    only_power: Option<PowerId>,
    resumable_recursive_draw: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if catalog.hooks().owns(crate::ids::RelicId::RelicSneckoEye)
        || catalog
            .hooks()
            .owns(crate::ids::RelicId::RelicFakeSneckoEye)
    {
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if spec.cost >= 0 {
            let live = state.rng.get(RngStream::EnergyCosts);
            let mut rng = Xoshiro256StarStar {
                words: live.words,
                counter: live.counter,
            };
            let rolled = i64::from(
                rng.next_bounded(4)
                    .map_err(|_| EngineRefusal::CounterOverflow("Confused cost"))?,
            );
            state.rng.set(
                RngStream::EnergyCosts,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            if state.card_states.removed_draw_object(card.uid).is_some() {
                // Confused 0xa07f4 mutates the retained object even after removal.
                let mut instance = drawn_object_instance(state, card.uid)?;
                instance.local_cost_modifiers.push(LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Set,
                    amount: rolled,
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                });
                set_drawn_object_instance(state, card.uid, instance)?;
            } else {
                state
                    .card_states
                    .set_confused_cost(card.uid, rolled)
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
            }
            set_drawn_flags(state, card.uid, CARD_FLAG_DEFAULT_PHYSICAL_STATE)?;
            state.exact_piles = true;
        }
        for relic in [
            crate::ids::RelicId::RelicSneckoEye,
            crate::ids::RelicId::RelicFakeSneckoEye,
        ] {
            if catalog.hooks().owns(relic) {
                crate::coverage::record_relic(relic);
            }
        }
    }
    let mut order = state.fanouts.after_card_drawn_order().to_vec();
    if order.is_empty() {
        order.extend(
            AFTER_CARD_DRAWN_POWER_FAMILY
                .into_iter()
                .filter(|power| state.powers.value(*power) > 0),
        );
    }
    for power in order {
        if only_power.is_some_and(|only| only != power) {
            continue;
        }
        let amount = state.powers.value(power);
        if amount <= 0 {
            continue;
        }
        match power {
            // v0.111.0 (DLL `9cb4f1ad`) `SpeedsterPower/<AfterCardDrawn>d__4::MoveNext`
            // RVA `0x345abc`: IL_0020-0028 leaves when `fromHandDraw` is TRUE, so
            // the turn-start hand draw never fires it and every other Draw
            // (card bodies, relics, powers) does. IL_002d-0045 is the
            // card-owner check (always the solo player here); IL_004a-007b
            // leaves unless `CombatState.CurrentSide` is the owner's side;
            // IL_0080-00c7 damages `HittableEnemies` for `Amount`, ValueProp 4
            // (Unpowered), dealer = the power's owner.
            PowerId::Speedster => {
                if source == DrawSource::HandDraw || !state.player_side_active {
                    continue;
                }
                // IL_00ab-00c7 is ONE `CreatureCmd.Damage` over the whole
                // roster: every target is committed before any result or
                // death listener runs, and each death's AfterDeath walk keeps
                // this Draw's catalog for Gremlin Horn (#3172). The listener
                // reads no combat-ending predicate (#3099); an empty roster
                // is the Damage's own no-op (`<Damage>d__12::MoveNext` RVA
                // `0x3e96c8` IL_00c0-00e4 returns on `targetList.Count == 0`).
                let Some(targets) =
                    after_card_drawn_hittable_enemies(state, "Speedster after combat ended")?
                else {
                    continue;
                };
                super::damage::damage_monsters_after_catalog_auth(
                    state,
                    catalog,
                    &targets,
                    DotNetDecimal::from_i64(i64::from(amount)),
                    true,
                    events,
                )?;
            }
            // v0.111.0 (DLL `9cb4f1ad`) `CorrosiveWavePower/<AfterCardDrawn>d__6::MoveNext`
            // RVA `0x3378b8`: IL_0020-0038 is the card-owner check; IL_00b6-00da
            // awaits ONE `PowerCmd.Apply<PoisonPower>(HittableEnemies, Amount,
            // Owner, null, false)`. The listener reads no combat-ending
            // predicate (#3099): the per-target gate is the callee's —
            // `PowerCmd/<Apply>d__0`1::MoveNext` RVA `0x3ef7dc` forwards each
            // target to `<Apply>d__1`1::MoveNext` RVA `0x3ef988`, whose
            // IL_0020-0034 returns when `CombatManager.IsEnding`.
            PowerId::CorrosiveWave => {
                let targets = super::damage::alive_targets(state);
                for target in targets {
                    if state.history.over
                        || state
                            .monsters
                            .get(target)
                            .is_none_or(|monster| monster.hp <= 0)
                    {
                        continue;
                    }
                    super::damage::apply_power_monster_debuff(
                        state,
                        target,
                        PowerId::Poison,
                        MiseryToken::Poison,
                        amount,
                        events,
                    )?;
                }
            }
            PowerId::Automation => automation_after_card_drawn(state, amount)?,
            PowerId::Cacophony => {
                let left = state
                    .fanouts
                    .cacophony_left()
                    .checked_sub(1)
                    .ok_or(EngineRefusal::CounterOverflow("cacophony countdown"))?;
                state.fanouts.set_cacophony_left(left);
                if left <= 0 {
                    // `<AfterCardDrawn>d__10::MoveNext` RVA `0x33669c`
                    // IL_0060-0086 rolls `CombatTargets.NextItem(HittableEnemies)`
                    // with no combat-ending predicate (#3099); an empty roster
                    // rolls nothing and IL_010d-0113 skips the Damage.
                    if after_card_drawn_hittable_enemies(state, "Cacophony after combat ended")?
                        .is_some()
                        && let Some(target) = super::damage::roll_target(state)?
                    {
                        super::damage::damage_monster_after_catalog_auth(
                            state,
                            catalog,
                            target,
                            DotNetDecimal::from_i64(i64::from(amount)),
                            false,
                            true,
                            events,
                        )?;
                    }
                    state.fanouts.set_cacophony_left(33);
                    let resets = state
                        .fanouts
                        .cacophony_resets_completed()
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("cacophony reset generation"))?;
                    state.fanouts.set_cacophony_resets_completed(resets);
                }
            }
            PowerId::Pagestorm | PowerId::Iteration => {
                let spec = catalog
                    .spec(card.atom)
                    .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                let fires = if power == PowerId::Pagestorm {
                    state.card_states.removed_draw_object(card.uid).map_or_else(
                        || super::cards::effective_ethereal(state, spec, card.uid),
                        |entry| {
                            spec.ethereal
                                || entry.state.local_ethereal()
                                || (state.powers.value(PowerId::HexPower) == 2
                                    && entry.card.flags & crate::hot::CARD_FLAG_HEXED != 0)
                        },
                    )
                } else {
                    spec.is_status && state.history.status_draws_this_turn == 1
                };
                if fires {
                    let draws: usize = amount
                        .try_into()
                        .map_err(|_| EngineRefusal::CounterOverflow("recursive draw"))?;
                    if resumable_recursive_draw {
                        if draw_cards_for_potion(
                            state,
                            catalog,
                            draws,
                            DrawCaller::AfterCardDrawnPower,
                            events,
                        )? == PotionDrawResult::Suspended
                        {
                            return Ok(());
                        }
                    } else {
                        draw_cards(state, catalog, draws, DrawSource::Command, events)?;
                    }
                }
            }
            PowerId::ChainsOfBinding => {
                if amount != 3 {
                    return Err(EngineRefusal::MalformedArgs("Chains of Binding amount"));
                }
                if !super::cards::chains_draw_state_is_exact(state) {
                    return Err(EngineRefusal::MalformedArgs("Chains draw entry"));
                }
                if !state.player_side_active {
                    continue;
                }
                let count = state.bound_afflictions_this_turn();
                if count >= 3 {
                    continue;
                }
                let live = drawn_object_card(state, card.uid)?;
                if live.flags & super::cards::CARD_AFFLICTION_FLAGS == 0 {
                    set_drawn_flags(
                        state,
                        card.uid,
                        crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_BOUND,
                    )?;
                    if !state.set_bound_afflictions_this_turn(count + 1) {
                        return Err(EngineRefusal::CounterOverflow("Bound affliction count"));
                    }
                    state.exact_piles = true;
                }
            }
            _ => {
                return Err(EngineRefusal::PowerHookNotModeled {
                    power,
                    event: HookEvent::AfterCardDrawn,
                });
            }
        }
    }
    Ok(())
}

/// `CombatState.HittableEnemies` for an ordinary `AfterCardDrawn` listener
/// that targets it without reading any combat-ending predicate (#3099).
///
/// A started walk keeps running after an earlier listener ended combat
/// ([`after_card_drawn_walk_starts`]). Rust ends combat only when no monster
/// is alive, so the roster is then empty and the native command is a no-op:
/// `Ok(None)`. A combat that ended with a living enemy would reach the native
/// command over a non-empty roster, which this port does not represent, so it
/// refuses by `name`.
fn after_card_drawn_hittable_enemies(
    state: &HotState,
    name: &'static str,
) -> Result<Option<Vec<usize>>, EngineRefusal> {
    let targets = super::damage::alive_targets(state);
    if targets.is_empty() {
        return Ok(None);
    }
    if state.history.over {
        return Err(EngineRefusal::PowerOrderNotModeled(name));
    }
    Ok(Some(targets))
}

/// Every live `AutomationPower` object's `AfterCardDrawn` listener, in
/// acquisition order (#3021).
///
/// v0.111.0 (DLL `9cb4f1ad`) `AutomationPower/<AfterCardDrawn>d__14::MoveNext`
/// RVA `0x3355a0`: IL_004f-0057 decrements THIS object's `Data.cardsLeft`;
/// IL_006e branches past the payout unless it reached 0 or below; IL_0079-008f
/// awaits `PlayerCmd.GainEnergy(this.Amount)` with this object's own Amount;
/// IL_00e7-00ec resets this object's counter to 10. The type is Instanced
/// (`get_InstanceType` RVA `0x9fa0e`), so two plays are two objects with two
/// counters, each paying only its own Amount.
///
/// The first object lives in the scalar/packed pair (amount = scalar minus
/// the later rows, countdown = `automation_left`); later objects are
/// [`crate::hot::AutomationInstance`] rows. All of them run contiguously at
/// the one Automation listener position: [`crate::steps::templates::automation`]
/// refuses to attach a later object behind any other draw listener, so no
/// foreign listener can sit between two Automation objects.
fn automation_after_card_drawn(state: &mut HotState, total: i32) -> Result<(), EngineRefusal> {
    let mut later = state.fanouts.automation_later_instances().to_vec();
    let first_amount = state
        .fanouts
        .automation_later_amount()
        .and_then(|later_amount| total.checked_sub(later_amount))
        .filter(|amount| *amount >= 1)
        .ok_or(EngineRefusal::MalformedArgs("automation instances"))?;
    let mut payouts = Vec::new();
    let left = state
        .fanouts
        .automation_left()
        .checked_sub(1)
        .ok_or(EngineRefusal::CounterOverflow("automation countdown"))?;
    if left <= 0 {
        state.fanouts.set_automation_left(10);
        payouts.push(first_amount);
    } else {
        state.fanouts.set_automation_left(left);
    }
    for instance in &mut later {
        instance.cards_left = instance
            .cards_left
            .checked_sub(1)
            .ok_or(EngineRefusal::CounterOverflow("automation countdown"))?;
        if instance.cards_left <= 0 {
            instance.cards_left = 10;
            payouts.push(instance.amount);
        }
    }
    if !later.is_empty() && !state.fanouts.set_automation_later_instances(&later) {
        return Err(EngineRefusal::MalformedArgs("automation instances"));
    }
    for amount in payouts {
        // The listener reads no combat-ending predicate (#3099); this is the
        // callee's: `PlayerCmd/<GainEnergy>d__3::MoveNext` RVA `0x3ee8a0`
        // IL_0030-003c returns when `CombatManager.IsEnding`, after the
        // counters above already moved.
        if state.history.over {
            continue;
        }
        let energy: i16 = amount
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("automation energy"))?;
        state.energy = state
            .energy
            .checked_add(energy)
            .ok_or(EngineRefusal::CounterOverflow("automation energy"))?;
    }
    Ok(())
}

/// Kingly Kick and Kingly Punch's physical-card `AfterCardDrawn` listeners.
///
/// Current v0.111.0 `KinglyKick::AfterCardDrawn` RVA `0xe3abb` first requires
/// that the hook's drawn object is this exact instance, then appends one
/// combat-long `Add(-1)` Energy-cost row. Python
/// `_physical_card_after_drawn_uid` (frozen, deleted #2827) carries the same physical
/// identity and row order. Player-power and relic listeners run first and may
/// recursively draw or relocate the exact object, so the card-pile listener
/// re-resolves the immutable uid across all five live piles. Only a present
/// Kingly card pays for that walk; every other draw returns before touching
/// piles or the copy-on-write side table.
///
/// The listener is a card of `AllPiles` in the ordinary walk, so it runs only
/// when that walk started ([`after_card_drawn_walk_starts`], #3099):
/// `KinglyKick::AfterCardDrawn` RVA `0xe3abb` and `KinglyPunch::AfterCardDrawn`
/// RVA `0xe3b80` read no combat-ending predicate of their own (their
/// leading `card == this` checks, IL_0001-000a and IL_000c-0015), so a
/// started walk reaches them even after an earlier listener ended combat, and
/// a walk that never started skips them.
fn physical_card_after_drawn(
    state: &mut HotState,
    catalog: &Catalog,
    drawn: HotCard,
    walk_started: bool,
) -> Result<(), EngineRefusal> {
    if !walk_started {
        return Ok(());
    }
    let drawn_spec = catalog
        .spec(drawn.atom)
        .ok_or(EngineRefusal::UnknownAtom(drawn.atom))?;
    if !matches!(
        drawn_spec.identity.id,
        CardId::KinglyKick | CardId::KinglyPunch
    ) {
        return Ok(());
    }

    let mut found = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .enumerate()
            .filter(move |(_, card)| card.uid == drawn.uid)
            .map(move |(index, card)| (pile, index, *card))
    });
    let (pile, index, live) = match (found.next(), found.next()) {
        (Some(live), None) => live,
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    let identity = catalog
        .spec(live.atom)
        .ok_or(EngineRefusal::UnknownAtom(live.atom))?
        .identity;
    match (identity.id, identity.upgrade) {
        (CardId::KinglyKick, 0 | 1) => {
            state.card_states.append_local_cost_modifier(
                live.uid,
                crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Add,
                    amount: -1,
                    expiration: crate::hot::LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                },
            );
        }
        (CardId::KinglyPunch, upgrade @ (0 | 1)) => {
            let increase = if upgrade == 0 { 4 } else { 6 };
            state
                .card_states
                .get(live.uid)
                .damage_growth
                .checked_add(increase)
                .ok_or(EngineRefusal::CounterOverflow("physical damage growth"))?;
            state
                .card_states
                .add_damage_growth(live.uid, increase)
                .expect("the Kingly Punch growth write was preflighted");
        }
        _ => return Err(EngineRefusal::MalformedArgs("Kingly draw listener")),
    }
    state.piles.get_mut(pile).make_mut()[index].flags |=
        crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    // Python commits the replacement with `force_exact=True`, even when no
    // equal sibling is present.
    state.exact_piles = true;
    Ok(())
}

/// `CardPile::get_MaxCardsInHand` (RVA 0x2c8343).
pub const MAX_CARDS_IN_HAND: usize = 10;

pub(crate) const AFTER_CARD_DRAWN_POWER_FAMILY: [PowerId; 7] = [
    PowerId::Speedster,
    PowerId::CorrosiveWave,
    PowerId::Automation,
    PowerId::Cacophony,
    PowerId::Pagestorm,
    PowerId::Iteration,
    PowerId::ChainsOfBinding,
];

/// Authenticate the complete live player-power listener set. Execution order
/// is acquisition order, so every live member appears exactly once and no
/// inactive or foreign token may survive in the ledger.
pub(crate) fn after_card_drawn_power_order_is_exact(state: &HotState) -> bool {
    let order = state.fanouts.after_card_drawn_order();
    order.iter().all(|power| {
        AFTER_CARD_DRAWN_POWER_FAMILY.contains(power) && state.powers.value(*power) > 0
    }) && order
        .iter()
        .enumerate()
        .all(|(index, power)| !order[index + 1..].contains(power))
        && AFTER_CARD_DRAWN_POWER_FAMILY
            .iter()
            .all(|power| (state.powers.value(*power) > 0) == order.contains(power))
}

/// Which `CardPileCmd::Draw` overload this command is.
///
/// `fromHandDraw` (`Hook::AfterCardDrawn` arg3, 0x2ad954) is TRUE for exactly
/// one call site in the whole v0.111.0 DLL — `CombatManager.SetupPlayerTurn`
/// (`<SetupPlayerTurn>d__102::MoveNext` RVA `0x3f6c6c` IL_03cd, `ldc.i4.1`
/// into the 4-arg `CardPileCmd.Draw` RVA `0x13141f`), the turn-start hand
/// draw. The other 68 direct callers of that overload pass `ldc.i4.0`, and
/// `DrawWithoutBlockingOnOtherPlayers` forwards its caller's argument (its
/// one caller passes false). Every card body that draws passes false, and the
/// difference is observable: `_draw_one_iteration` (frozen Python, deleted #2827) bumps
/// `non_hand_draws_this_turn` only for the false form.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DrawSource {
    /// `SetupPlayerTurn`'s single `fromHandDraw` command.
    HandDraw,
    /// Every other Draw: a card body, a relic, a power.
    Command,
}

/// Result of a potion-owned resumable Draw command.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PotionDrawResult {
    Complete,
    Suspended,
}

/// Result-bearing card-body Draw completion. The ordered entries are the
/// exact native `IEnumerable<CardModel>` payload retained by the Draw frame,
/// never a re-derived Hand suffix.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CardDrawResult {
    Complete(Vec<DrawEntry>),
    Suspended,
}

fn current_after_card_drawn_order(state: &HotState) -> Vec<PowerId> {
    let explicit = state.fanouts.after_card_drawn_order();
    if !explicit.is_empty() {
        return explicit.to_vec();
    }
    AFTER_CARD_DRAWN_POWER_FAMILY
        .into_iter()
        .filter(|power| state.powers.value(*power) > 0)
        .collect()
}

fn start_after_card_drawn_power_walk(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    source: DrawSource,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let listeners = current_after_card_drawn_order(state);
    if listeners.is_empty() {
        return Ok(false);
    }
    let owner_depth = state.frames.len();
    let record = AfterCardDrawnPowerRecord {
        listeners,
        cursor: 0,
        card_uid: uid,
        from_hand_draw: source == DrawSource::HandDraw,
        cacophony_reset: false,
        cacophony_reset_generation: state.fanouts.cacophony_resets_completed(),
    };
    state
        .frames
        .push_after_card_drawn_power(&record)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    advance_top_after_card_drawn_power(state, catalog, events)?;
    Ok(state.frames.len() > owner_depth)
}

pub(crate) fn advance_top_after_card_drawn_power(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    loop {
        let mut record = match state.frames.top() {
            Some(crate::frame::Frame::AfterCardDrawnPower { record }) => state
                .frames
                .after_card_drawn_power(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?
                .to_owned(),
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        if record.listeners != current_after_card_drawn_order(state)
            || drawn_object_card(state, record.card_uid).is_err()
            || record.cacophony_reset
            || record.cacophony_reset_generation != state.fanouts.cacophony_resets_completed()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let cursor =
            usize::try_from(record.cursor).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
        let Some(power) = record.listeners.get(cursor).copied() else {
            state
                .frames
                .pop_top_after_card_drawn_power()
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            return Ok(());
        };
        record.cursor = record
            .cursor
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("AfterCardDrawn cursor"))?;
        state
            .frames
            .replace_top_after_card_drawn_power(&record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let card = drawn_object_card(state, record.card_uid)?;
        let source = if record.from_hand_draw {
            DrawSource::HandDraw
        } else {
            DrawSource::Command
        };
        powers_after_card_drawn(state, catalog, card, source, Some(power), true, events)?;
        if power == PowerId::Cacophony
            && matches!(
                state.frames.top(),
                Some(crate::frame::Frame::AfterCardDrawnPower { .. })
            )
        {
            record.cacophony_reset_generation = state.fanouts.cacophony_resets_completed();
            state
                .frames
                .replace_top_after_card_drawn_power(&record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        }
        if state.pending.is_some()
            || !matches!(
                state.frames.top(),
                Some(crate::frame::Frame::AfterCardDrawnPower { .. })
            )
        {
            return Ok(());
        }
    }
}

/// The Slither enchantment's `AfterCardDrawn` listener (#2926).
///
/// v0.111.0 (DLL `9cb4f1ad`) `Slither::AfterCardDrawn` RVA `0xd636c`:
/// IL_000c-0013 leaves unless the drawn card is this enchantment's own card;
/// IL_001b-002c leaves unless that card's `Pile.Type` is `Hand` (`PileType`
/// 2) at the moment the listener runs; IL_0034-0046 calls
/// `EnergyCost.SetThisCombat(NextEnergyCost(), false)`. `Slither::NextEnergyCost`
/// RVA `0xd63d7` returns `_testEnergyCostOverride` only when it is `>= 0`
/// (IL_0001-0010; `.ctor` `0xd6409` sets `-1` and the setter has no caller),
/// otherwise IL_0011-002c `Owner.RunState.Rng.CombatEnergyCosts.NextInt(4)`.
/// `CardEnergyCost::SetThisCombat` RVA `0x11e21c` IL_0001-000d returns
/// without a row only for amount 0 on a negative canonical cost, else
/// IL_000e-001d appends one `LocalCostModifier(amount, Set, ThisCombat,
/// reduceOnly)` row. `FindOnTable`/`PlayRandomizeCostAnim` (IL_0055-0060)
/// are display only. `fromHandDraw` is not read, so the turn-start HandDraw
/// (the opening hand included) rolls exactly like a command Draw.
///
/// Position: `Hook/<AfterCardDrawn>d__11::MoveNext` RVA `0x3cc514` walks the
/// ordinary listeners after the whole early walk (IL_0124-017e), through
/// `<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`, which yields
/// nothing when combat `IsOverOrEnding` at the walk's start (IL_0028-0042)
/// and otherwise the `<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720`
/// snapshot: the player's Powers (IL_008f), Relics (IL_00c7-0103), potions,
/// orbs, then every card of `AllPiles` (Hand, Draw, Discard, Exhaust, Play;
/// `PlayerCombatState::get_AllPiles` RVA `0x117de8`) followed by its
/// affliction and then its enchantment (IL_0198-01c7). So Slither runs after
/// every player power (Confused's roll included) and relic, and right after
/// its own card's listener; only its own card can satisfy IL_0013, so no
/// other card's position is observable. `walk_started` carries the walk-start
/// predicate: a combat that ends mid-walk still reaches this listener.
///
/// Refusals, by name: a retained removed Hellraiser object (native `Pile` is
/// null there and IL_0021 would dereference it), and a negative canonical
/// cost, which `Slither::CanEnchant` RVA `0xd6340` rules out and this port
/// does not represent.
/// Whether a card's run enchantment is Slither on a card this port rolls
/// exactly: Slither edits no CardPlay body (its only overrides are
/// `CanEnchant`, `AfterCardDrawn` and `NextEnergyCost`, RVAs `0xd6340`,
/// `0xd636c`, `0xd63d7`), and its draw roll is [`slither_after_card_drawn`].
/// A negative canonical cost is outside `CanEnchant` and stays refused.
pub(crate) fn slither_identity_is_exact(spec: &crate::catalog::CardSpec) -> bool {
    spec.identity
        .enchantment
        .is_some_and(|enchantment| matches!(enchantment.id, EnchantmentId::Slither))
        && spec.cost >= 0
}

fn slither_after_card_drawn(
    state: &mut HotState,
    catalog: &Catalog,
    listener: HotCard,
    walk_started: bool,
) -> Result<(), EngineRefusal> {
    // The listener exists only on a Slither copy, so every other draw returns
    // here before any live-pile walk.
    let spec = catalog
        .spec(listener.atom)
        .ok_or(EngineRefusal::UnknownAtom(listener.atom))?;
    if !walk_started
        || !spec
            .identity
            .enchantment
            .is_some_and(|enchantment| matches!(enchantment.id, EnchantmentId::Slither))
    {
        return Ok(());
    }
    if listener.uid == crate::hot::LEGACY_CARD_UID {
        return Err(EngineRefusal::MalformedArgs("Slither legacy drawn card"));
    }
    let uid = listener.uid;
    drawn_object_card(state, uid)?;
    let Some((pile, index)) = super::play::unique_live_card_location(state, uid)? else {
        return Err(EngineRefusal::MalformedArgs(
            "Slither drawn card outside every pile",
        ));
    };
    if pile != PileId::Hand {
        return Ok(());
    }
    if spec.cost < 0 {
        return Err(EngineRefusal::MalformedArgs(
            "Slither negative canonical cost",
        ));
    }
    let live = state.rng.get(RngStream::EnergyCosts);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let rolled = i64::from(
        rng.next_bounded(4)
            .map_err(|_| EngineRefusal::CounterOverflow("Slither cost"))?,
    );
    state.rng.set(
        RngStream::EnergyCosts,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    state.card_states.append_local_cost_modifier(
        uid,
        LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount: rolled,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    );
    state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    state.exact_piles = true;
    Ok(())
}

fn finish_drawn_uid_after_ordinary(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    walk_started: bool,
) -> Result<HotCard, EngineRefusal> {
    let listener_card = drawn_object_card(state, uid)?;
    physical_card_after_drawn(state, catalog, listener_card, walk_started)?;
    slither_after_card_drawn(state, catalog, listener_card, walk_started)?;
    let completed = drawn_object_card(state, uid)?;
    void_after_card_drawn(state, catalog, completed, walk_started)?;
    Ok(completed)
}

/// Whether `Hook.AfterCardDrawn`'s ordinary listener walk starts (#3099).
///
/// v0.111.0 (DLL `9cb4f1ad`) `Hook/<AfterCardDrawn>d__11::MoveNext` RVA
/// `0x3cc514` IL_0124-017e enumerates the ordinary listeners through
/// `<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`, whose
/// IL_0028-0042 yields an EMPTY walk when `CombatManager.IsOverOrEnding`
/// (and not `IsStarting`) on the enumerator's first `MoveNext` — the only
/// point that predicate is read: a later `MoveNext` (IL_0082-0095) just
/// advances the `IterateHookListeners` snapshot. So the gate is taken once,
/// at walk start, for every ordinary listener — player powers, relics, and
/// every card's own listener (Kingly Kick/Punch, Void, Slither) — and a
/// combat that ends mid-walk still reaches every later listener. Rust
/// projects `IsOverOrEnding` to `history.over`, with a dead player
/// (`hp <= 0`) ending too.
pub(crate) fn after_card_drawn_walk_starts(state: &HotState) -> bool {
    !state.history.over && state.hp > 0
}

/// Void's own `CardModel.AfterCardDrawn` listener: the on-draw energy loss.
///
/// v0.111.0 (DLL `9cb4f1ad`) `Void/<AfterCardDrawn>d__7::MoveNext` RVA
/// `0x3c6894`: IL_0024-002d leaves unless the drawn card is this Void;
/// IL_0092-00ad awaits `PlayerCmd.LoseEnergy(DynamicVars.Energy, Owner)`.
/// `PlayerCmd::LoseEnergy` RVA `0x13329b` IL_0014-0025 returns without a
/// change when `CombatManager.IsEnding` — the callee's own gate, read at the
/// moment the listener runs, so it survives inside a started walk. The
/// listener itself runs only in a started ordinary walk
/// ([`after_card_drawn_walk_starts`]). Unit C admits only the exact generated
/// Void+0 row, so this generic metadata reader has one current carrier and
/// cannot silently authorize a forged keyword.
fn void_after_card_drawn(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    walk_started: bool,
) -> Result<(), EngineRefusal> {
    let energy_loss: i16 = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?
        .row
        .on_draw_energy_loss
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("on-draw energy loss"))?;
    if energy_loss > 0 && walk_started && !state.history.over {
        state.energy = state.energy.saturating_sub(energy_loss).max(0);
    }
    Ok(())
}

fn finish_drawn_uid_after_early(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    source: DrawSource,
    events: &mut Vec<Event>,
) -> Result<Option<HotCard>, EngineRefusal> {
    drawn_object_card(state, uid)?;
    let walk_started = after_card_drawn_walk_starts(state);
    if walk_started {
        if start_after_card_drawn_power_walk(state, catalog, uid, source, events)? {
            return Ok(None);
        }
        fire_hook(catalog, HookEvent::AfterCardDrawn, state, events)?;
    }
    finish_drawn_uid_after_ordinary(state, catalog, uid, walk_started).map(Some)
}

fn live_draw_prefix(
    state: &HotState,
    entries: &[DrawEntry],
) -> Result<Vec<DrawEntry>, EngineRefusal> {
    entries
        .iter()
        .copied()
        .map(|entry| match entry {
            DrawEntry::Uid(card) => drawn_object_card(state, card.uid).map(DrawEntry::Uid),
            DrawEntry::Payload(card) => Ok(DrawEntry::Payload(card)),
        })
        .collect()
}

/// Keep a persisted physical CardPlay's canonical pile owner synchronized
/// with a later card-pile command that moves the same object.
///
/// Nested AutoPlay can leave an outer CardPlay alive while an inner Draw,
/// Shuffle, or Stratagem command relocates its physical card.  The native
/// object remains the same; only its pile changes.  Repair the cold frame
/// before mutating the piles so any unrepresentable source transition fails
/// transactionally at the caller's rehearsal boundary.
#[inline]
pub(crate) fn repair_card_play_after_physical_move(
    state: &mut HotState,
    card: HotCard,
    source: PileId,
    destination: PileId,
) -> Result<(), EngineRefusal> {
    // A parkable CardPlay is normalized to a unique physical UID before the
    // continuation is published. Legacy zero-UID payloads can be duplicated
    // and therefore cannot own a persisted play record.
    if card.uid == crate::hot::LEGACY_CARD_UID {
        return Ok(());
    }
    state
        .frames
        .repair_card_play_after_move(card.uid, source, destination)
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

/// Prove that a complete serial pile-move batch can keep every persisted
/// CardPlay owner synchronized before the caller publishes its first move.
///
/// The returned proof is intentionally discarded: callers still repair the
/// live frame immediately before each corresponding pile mutation so nested
/// hooks never observe a future destination early. Cloning only the cold COW
/// frame store keeps a later unrepresentable owner from exposing a partial
/// prefix through step bodies that are also invoked directly in focused
/// tests.
pub(crate) fn preflight_card_play_after_physical_moves(
    state: &HotState,
    moves: &[(HotCard, PileId, PileId)],
) -> Result<(), EngineRefusal> {
    let mut frames = state.frames.clone();
    for (card, source, destination) in moves.iter().copied() {
        if card.uid == crate::hot::LEGACY_CARD_UID {
            continue;
        }
        frames
            .repair_card_play_after_move(card.uid, source, destination)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    }
    Ok(())
}

/// Execute a command Draw whose admitted parked owner is the immediately
/// lower `PotionFinish` or exact no-result `CardPlay`. The arena record is installed before an
/// awaited hook begins, so a child CardPlay can park above it without
/// replaying the pile move, counters, event, or reshuffle RNG.
pub(crate) fn draw_cards_for_potion(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<PotionDrawResult, EngineRefusal> {
    let requested =
        u32::try_from(n).map_err(|_| EngineRefusal::CounterOverflow("draw requested"))?;
    if requested == 0
        || state.history.over
        || caller != DrawCaller::TurnStart && state.powers.value(PowerId::NoDraw) > 0
    {
        return Ok(PotionDrawResult::Complete);
    }
    Ok(draw_cards_for_potion_from(
        state,
        catalog,
        requested,
        0,
        Vec::new(),
        caller,
        events,
        None,
    )?
    .0)
}

/// Execute Calculated Gamble's paired Draw through the same resumable frame
/// as potion/turn callers, parking the frozen discard/draw count plus the
/// Sly siblings on the parked record (#2667). The caller is always
/// [`DrawCaller::CardPlay`]: the owning CardPlay cursor authenticates the
/// tail at return, and the Sly batch replays only after the Draw fully
/// returns.
pub(crate) fn draw_cards_for_gamble_tail(
    state: &mut HotState,
    catalog: &Catalog,
    paired: crate::hot::GamblePaired,
    events: &mut Vec<Event>,
) -> Result<PotionDrawResult, EngineRefusal> {
    let requested = paired.count;
    if requested == 0 || state.history.over || state.powers.value(PowerId::NoDraw) > 0 {
        return Ok(PotionDrawResult::Complete);
    }
    Ok(draw_cards_for_potion_from(
        state,
        catalog,
        requested,
        0,
        Vec::new(),
        DrawCaller::CardPlay,
        events,
        Some(paired),
    )?
    .0)
}

/// Execute a result-bearing card-body Draw through the same resumable frame
/// as potion/turn callers, returning the exact ordered moved-card payload
/// when it completes synchronously.
pub(crate) fn draw_cards_for_card_result(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<CardDrawResult, EngineRefusal> {
    if !matches!(
        caller,
        DrawCaller::CardPlay
            | DrawCaller::Pillage
            | DrawCaller::RestlessnessFinal
            | DrawCaller::RestlessnessOneRemaining
            | DrawCaller::RestlessnessTwoRemaining
    ) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let requested =
        u32::try_from(n).map_err(|_| EngineRefusal::CounterOverflow("draw requested"))?;
    if requested == 0 || state.history.over || state.powers.value(PowerId::NoDraw) > 0 {
        return Ok(CardDrawResult::Complete(Vec::new()));
    }
    let (result, drawn) = draw_cards_for_potion_from(
        state,
        catalog,
        requested,
        0,
        Vec::new(),
        caller,
        events,
        None,
    )?;
    Ok(match result {
        PotionDrawResult::Complete => CardDrawResult::Complete(drawn),
        PotionDrawResult::Suspended => CardDrawResult::Suspended,
    })
}

/// The eighth parameter is Calculated Gamble's parked paired program
/// (#2667): `None` for every other Draw. It rides alongside rather than
/// inside `caller` so the shared potion/turn/reshuffle entries keep their
/// signatures and only the Gamble tail passes `Some`.
#[allow(clippy::too_many_arguments)]
fn draw_cards_for_potion_from(
    state: &mut HotState,
    catalog: &Catalog,
    requested: u32,
    mut completed: u32,
    mut drawn: Vec<DrawEntry>,
    caller: DrawCaller,
    events: &mut Vec<Event>,
    gamble_paired: Option<crate::hot::GamblePaired>,
) -> Result<(PotionDrawResult, Vec<DrawEntry>), EngineRefusal> {
    let from_hand_draw = caller == DrawCaller::TurnStart;
    let source = if from_hand_draw {
        DrawSource::HandDraw
    } else {
        DrawSource::Command
    };
    while completed < requested {
        if state.history.over || state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
            break;
        }
        if state.piles.get(PileId::Draw).is_empty() {
            if state.piles.get(PileId::Discard).is_empty() {
                break;
            }
            let shuffle_cursor = DrawRecord {
                requested,
                completed,
                drawn: drawn.clone(),
                from_hand_draw,
                stage: DrawStage::AfterShuffle,
                card_uid: None,
                caller,
                shuffle_candidates: Vec::new(),
                gamble_paired: gamble_paired.clone(),
            };
            if reshuffle_for_resumable_draw(state, catalog, &shuffle_cursor, events)? {
                return Ok((PotionDrawResult::Suspended, Vec::new()));
            }
            if state.history.over
                || state.piles.get(PileId::Draw).is_empty()
                || state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND
            {
                break;
            }
        }
        let card = state.piles.get(PileId::Draw).as_slice()[0];
        repair_card_play_after_physical_move(state, card, PileId::Draw, PileId::Hand)?;
        let mut card = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        state.cards_drawn_combat = state
            .cards_drawn_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("cards_drawn_combat"))?;
        if !from_hand_draw {
            state.history.non_hand_draws_this_turn = state
                .history
                .non_hand_draws_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("non_hand_draws_this_turn"))?;
        }
        if catalog.spec(card.atom).is_some_and(|spec| spec.is_status) {
            state.history.status_draws_this_turn = state
                .history
                .status_draws_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("status_draws_this_turn"))?;
        }
        if card.uid == crate::hot::LEGACY_CARD_UID {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            let order = current_after_card_drawn_order(state);
            let recursive_listener_fires = order.iter().any(|power| match power {
                PowerId::Pagestorm => super::cards::effective_ethereal(state, spec, card.uid),
                PowerId::Iteration => spec.is_status && state.history.status_draws_this_turn == 1,
                PowerId::ChainsOfBinding => true,
                _ => false,
            });
            let needs_physical_owner =
                state.powers.value(PowerId::Hellraiser) > 0 && spec.strike_tag
                    || recursive_listener_fires
                    || matches!(spec.identity.id, CardId::KinglyKick | CardId::KinglyPunch)
                    || spec.identity.enchantment.is_some_and(|enchantment| {
                        matches!(enchantment.id, EnchantmentId::Slither)
                    });
            if needs_physical_owner {
                // Reification happens only at the first continuation-capable
                // physical hook.  Earlier hook-free legacy iterations remain
                // payload entries exactly as Python reports them; Snecko's
                // post-Draw suffix normalizes all remaining piles later.
                super::cards::normalize_card_identities(state)?;
                card = *state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .last()
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
            } else {
                events.push(Event::CardDrawn { uid: card.uid });
                drawn.push(DrawEntry::Payload(card));
                // One walk-start gate for the whole ordinary walk (#3099):
                // a relic still runs after a power ended combat mid-walk.
                let walk_started = after_card_drawn_walk_starts(state);
                if walk_started {
                    powers_after_card_drawn(state, catalog, card, source, None, false, events)?;
                    fire_hook(catalog, HookEvent::AfterCardDrawn, state, events)?;
                }
                physical_card_after_drawn(state, catalog, card, walk_started)?;
                void_after_card_drawn(state, catalog, card, walk_started)?;
                completed = completed
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("draw completed"))?;
                continue;
            }
        }
        events.push(Event::CardDrawn { uid: card.uid });
        drawn.push(DrawEntry::Uid(card));
        let draw_record = DrawRecord {
            requested,
            completed,
            drawn: live_draw_prefix(state, &drawn)?,
            from_hand_draw,
            stage: DrawStage::EarlyHook,
            card_uid: Some(card.uid),
            caller,
            shuffle_candidates: Vec::new(),
            gamble_paired: gamble_paired.clone(),
        };
        state
            .frames
            .push_draw(&draw_record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;

        hellraiser_after_card_drawn_early_resumable(state, catalog, card.uid, events)?;
        if state.pending.is_some() {
            return Ok((PotionDrawResult::Suspended, Vec::new()));
        }
        if !matches!(state.frames.top(), Some(crate::frame::Frame::Draw { .. })) {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let mut draw_record = draw_record;
        draw_record.stage = DrawStage::OrdinaryHook;
        draw_record.drawn = live_draw_prefix(state, &drawn)?;
        state
            .frames
            .replace_top_draw(&draw_record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let Some(_) = finish_drawn_uid_after_early(state, catalog, card.uid, source, events)?
        else {
            return Ok((PotionDrawResult::Suspended, Vec::new()));
        };
        state
            .frames
            .pop_top_draw()
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        completed = completed
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("draw completed"))?;
    }
    Ok((PotionDrawResult::Complete, drawn))
}

/// Resume a top Draw after its awaited child chain fully unwound.
pub(crate) fn advance_top_potion_draw(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let record = match state.frames.top() {
        Some(crate::frame::Frame::Draw { record }) => state
            .frames
            .draw(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?
            .to_owned(),
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if record.stage == DrawStage::AfterShuffle {
        if record.card_uid.is_some() {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let caller = record.caller;
        if caller == DrawCaller::DistilledChaosGather {
            state
                .frames
                .pop_top_draw()
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            return super::potions::resume_distilled_gather_after_shuffle(state, catalog, events);
        }
        let requested = record.requested;
        let completed = record.completed;
        let drawn = live_draw_prefix(state, &record.drawn)?;
        state
            .frames
            .pop_top_draw()
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        if completed < requested && !state.history.over {
            let (result, continued_drawn) = draw_cards_for_potion_from(
                state,
                catalog,
                requested,
                completed,
                drawn,
                caller,
                events,
                record.gamble_paired.clone(),
            )?;
            if result == PotionDrawResult::Suspended {
                return Ok(());
            }
            return resume_completed_draw_caller(
                state,
                catalog,
                caller,
                &continued_drawn,
                record.gamble_paired.clone(),
                events,
            );
        }
        return resume_completed_draw_caller(
            state,
            catalog,
            caller,
            &[],
            record.gamble_paired.clone(),
            events,
        );
    }
    let uid = record
        .card_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    match record.stage {
        DrawStage::EarlyHook => {
            let mut ordinary = record.clone();
            ordinary.stage = DrawStage::OrdinaryHook;
            ordinary.drawn = live_draw_prefix(state, &record.drawn)?;
            state
                .frames
                .replace_top_draw(&ordinary)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let source = if record.from_hand_draw {
                DrawSource::HandDraw
            } else {
                DrawSource::Command
            };
            if finish_drawn_uid_after_early(state, catalog, uid, source, events)?.is_none() {
                return Ok(());
            }
        }
        DrawStage::OrdinaryHook => {
            fire_hook(catalog, HookEvent::AfterCardDrawn, state, events)?;
            // Only a started ordinary walk can have parked this stage.
            finish_drawn_uid_after_ordinary(state, catalog, uid, true)?;
        }
        DrawStage::AfterShuffle => unreachable!("handled above"),
    }
    let completed = record
        .completed
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("draw completed"))?;
    let caller = record.caller;
    let requested = record.requested;
    let drawn = live_draw_prefix(state, &record.drawn)?;
    state
        .frames
        .pop_top_draw()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if completed < requested && !state.history.over {
        let (result, continued_drawn) = draw_cards_for_potion_from(
            state,
            catalog,
            requested,
            completed,
            drawn,
            caller,
            events,
            record.gamble_paired.clone(),
        )?;
        if result == PotionDrawResult::Suspended {
            return Ok(());
        }
        return resume_completed_draw_caller(
            state,
            catalog,
            caller,
            &continued_drawn,
            record.gamble_paired.clone(),
            events,
        );
    }
    resume_completed_draw_caller(
        state,
        catalog,
        caller,
        &drawn,
        record.gamble_paired.clone(),
        events,
    )
}

fn resume_completed_draw_caller(
    state: &mut HotState,
    catalog: &Catalog,
    caller: DrawCaller,
    drawn: &[DrawEntry],
    gamble_paired: Option<crate::hot::GamblePaired>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.pending.is_some() {
        return Ok(());
    }
    match caller {
        DrawCaller::TurnStart
            if state
                .frames
                .top()
                .is_none_or(|frame| matches!(frame, crate::frame::Frame::ActionReplay { .. })) =>
        {
            super::turn::resume_player_turn_start_after_hand_draw(state, catalog, events)
        }
        DrawCaller::UnceasingTopTurnStart
            if state
                .frames
                .top()
                .is_none_or(|frame| matches!(frame, crate::frame::Frame::ActionReplay { .. })) =>
        {
            super::turn::resume_player_turn_start_after_top(state, catalog, events)
        }
        DrawCaller::UnceasingTopFinish
            if matches!(
                state.frames.top(),
                Some(crate::frame::Frame::CardFinish { .. })
            ) =>
        {
            super::play::resume_unceasing_top_card_finish(state)
        }
        DrawCaller::DistilledChaosGather
            if matches!(
                state.frames.top(),
                Some(crate::frame::Frame::FrozenAutoBatch { .. })
            ) =>
        {
            super::potions::resume_distilled_gather(state, catalog, events)
        }
        DrawCaller::AfterCardDrawnPower => Ok(()),
        // A receipt-owned Draw resumes in its Rust caller once
        // `engine::puzzle::receipt_owned_draw` has driven the Draw frame back
        // to its depth; nothing below the frame is owned here.
        DrawCaller::CentennialPuzzle | DrawCaller::SwiftEnchantment | DrawCaller::JossPaper => {
            Ok(())
        }
        DrawCaller::AfterCardExhaustedPower
            if matches!(
                state.frames.top(),
                Some(crate::frame::Frame::AfterCardExhaustedPower { .. })
            ) =>
        {
            Ok(())
        }
        DrawCaller::AfterPowerAmountChanged
            if matches!(
                state.frames.top(),
                Some(crate::frame::Frame::AfterPowerAmountChanged { .. })
            ) =>
        {
            Ok(())
        }
        DrawCaller::CardPlay => {
            super::play::resume_cardplay_owned_draw(state, catalog, drawn, gamble_paired, events)
        }
        DrawCaller::Pillage if super::play::pillage_draw_return_owner_is_exact(state, catalog) => {
            crate::steps::silent_uncommon::resume_pillage_after_draw(state, catalog, drawn, events)
        }
        DrawCaller::RestlessnessFinal
        | DrawCaller::RestlessnessOneRemaining
        | DrawCaller::RestlessnessTwoRemaining => {
            let amount = super::play::restlessness_draw_return_owner_amount(state, catalog, caller)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            crate::steps::neutral::resume_restlessness_after_draw(
                state, catalog, caller, amount, events,
            )
        }
        DrawCaller::DarkEmbraceSideEnd
            if matches!(
                state.frames.top(),
                Some(crate::frame::Frame::DarkEmbraceSideEnd { .. })
            ) =>
        {
            super::turn::resume_dark_embrace_side_end(state, catalog, events)
        }
        _ if matches!(
            state.frames.top(),
            Some(crate::frame::Frame::PotionFinish { .. })
        ) =>
        {
            super::potions::resume_after_draw(state, catalog, caller, events)
        }
        _ => Err(EngineRefusal::ContinuationNotModeled),
    }
}

/// Draw `n` cards, reshuffling when the draw pile runs dry.
///
/// `DrawInternal` (d__19 0x42cfc8) checks hand space before touching a pile
/// and rechecks it after each iteration. No Draw's represented `ShouldDraw`
/// listener gates command draws before pile/RNG access; Fiddle remains
/// refused. Stratagem's synchronous, no-choice `AfterShuffle` path runs after
/// a completed reshuffle; Perfect Fit and real selection remain refused. The
/// represented `AfterCardDrawn` fan-out runs per moved card.
///
/// A Thieving Hopper fight's entry check admits Thievery's one internal
/// state as well as the public ones (#2965 lane): `ThievingHopper::
/// <ThieveryMove>d__36::MoveNext` RVA `0x371428` removes the stolen card
/// (`RemoveFromCombat` IL_024c-IL_0253) and installs Swipe (`Steal`
/// IL_040a-IL_0412, `PowerCmd.Apply` IL_046f-IL_048d) *before* its theft
/// attack (`Attack(TheftDamage)` IL_0525-IL_054d), and the monster machine
/// advances only after the move returns. A draw the attack's damage causes
/// (Centennial Puzzle's `AfterDamageReceived`, RVA `0x321758`) therefore runs
/// with Swipe live at loop zero, which is exactly
/// [`super::monsters::thieving_hopper_internal_state_is_valid`].
pub fn draw_cards(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    source: DrawSource,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == crate::ids::MonsterKind::ThievingHopper);
    if hopper_reachable
        && (!super::monsters::thieving_hopper_internal_state_is_valid(state)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(state, catalog))
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper public draw entry",
        ));
    }
    draw_cards_into(state, catalog, n, source, events, &mut NoDrawnCards)
}

/// Where a Draw command reports the identities it actually moved.
///
/// `draw_cards` returns "the exact ordered tuple of cards actually moved by
/// this command, matching `DrawInternal`'s `IEnumerable<CardModel>` result"
/// (frozen Python, deleted #2827): the calculated bodies that consume it — Escape Plan, Expertise,
/// Pillage — must read *that*, never the reentrantly mutable hand tail. The
/// result is delivered through this sink rather than as an owned vector so the
/// turn-start draw, which wants none of it, allocates nothing (D7); a body
/// that wants the identities passes a buffer it owns.
pub trait DrawnCards {
    /// One card, in the order the command moved it.
    fn push(&mut self, card: HotCard);
}

/// The sink for a caller that does not read the result.
pub struct NoDrawnCards;

impl DrawnCards for NoDrawnCards {
    fn push(&mut self, _card: HotCard) {}
}

impl DrawnCards for Vec<HotCard> {
    fn push(&mut self, card: HotCard) {
        Vec::push(self, card);
    }
}

/// One of Centennial Puzzle's three one-card Draws (#3114).
///
/// `CentennialPuzzle/<AfterDamageReceived>d__10::MoveNext` RVA `0x321758`
/// (v0.111.0, SHA-256 `9cb4f1ad…`) sets `UsedThisCombat` at IL_006f-0071 and
/// then, while `<i>5__2 < DynamicVars.Cards.BaseValue` (IL_00e6-0116, bound 3
/// by `get_CanonicalVars` RVA `0x91fce`), awaits one
/// `CardPileCmd.Draw(choiceContext, Owner)` at IL_007f-00e5. Each await is a
/// separate command: its reshuffle, `AfterShuffle` and `AfterCardDrawn`
/// listeners finish before the next begins.
///
/// Only two listeners can make that await a real player choice: Hellraiser's
/// `AfterCardDrawnEarly` AutoPlay of a selecting Strike, and Stratagem's
/// `AfterShuffle` selection. Neither power is installed by a drawn card's
/// AutoPlay or by any represented AfterCardDrawn listener, so a Draw issued
/// while both are absent cannot park and keeps the certified synchronous
/// command. Otherwise the Draw runs on the resumable frame with
/// [`DrawCaller::CentennialPuzzle`], and a park is handed to
/// `engine::puzzle::receipt_owned_draw`, which owns the rest of the damage
/// caller's command through the action's ActionReplay receipt.
pub(crate) fn centennial_puzzle_draw_one(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    receipt_owned_command_draw(state, catalog, 1, DrawCaller::CentennialPuzzle, events)
}

/// Joss Paper's threshold Draw of `n` cards (#3201).
///
/// `JossPaper/<DrawIfThresholdMet>d__27::MoveNext` RVA `0x327888`
/// (v0.111.0, SHA-256 `9cb4f1ad…`) awaits one
/// `CardPileCmd.Draw(choiceContext, CardsExhausted / ExhaustAmount, Owner,
/// false)` at IL_0058-00eb: a single command of `n` cards, not `n`
/// one-card commands. Its callers (`<AfterCardExhausted>d__25` RVA
/// `0x3275a0` IL_005f-00bd and `<AfterSideTurnEnd>d__26` RVA `0x3276ac`
/// IL_0054-00b2) await it as their last statement, and the remainder write
/// `CardsExhausted %= ExhaustAmount` (IL_00ec-0109) runs only after the Draw
/// returns, so the caller applies it once this call does.
///
/// As for the Puzzle, only Hellraiser's AutoPlay of a selecting Strike and
/// Stratagem's `AfterShuffle` selection can make the await a player choice;
/// with both absent the certified synchronous command runs, and otherwise
/// the Draw runs on the resumable frame with [`DrawCaller::JossPaper`] and a
/// park is owned by the action's ActionReplay receipt.
pub(crate) fn joss_paper_draw(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    receipt_owned_command_draw(state, catalog, n, DrawCaller::JossPaper, events)
}

/// A relic's plain `CardPileCmd.Draw` whose awaited suffix is receipt-owned.
fn receipt_owned_command_draw(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.powers.value(PowerId::Hellraiser) <= 0 && state.powers.value(PowerId::Stratagem) <= 0 {
        return draw_cards(state, catalog, n, DrawSource::Command, events);
    }
    // `draw_cards`' public entry checks, unchanged.
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == crate::ids::MonsterKind::ThievingHopper);
    if hopper_reachable
        && (!super::monsters::thieving_hopper_internal_state_is_valid(state)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(state, catalog))
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper public draw entry",
        ));
    }
    if state.card_states.dampen().is_some() && !super::cards::dampen_state_is_exact(state, catalog)
    {
        return Err(EngineRefusal::MalformedArgs("Dampen public draw entry"));
    }
    // `DrawInternal`'s ShouldDraw gates in `draw_cards_into_inner`'s order:
    // Fiddle denies an owner-side command, then NoDraw (which
    // `draw_cards_for_potion` applies to every non-turn-start caller).
    if catalog.hooks().owns(RelicId::RelicFiddle) && state.player_side_active {
        crate::coverage::record_relic(RelicId::RelicFiddle);
        return Ok(());
    }
    super::puzzle::receipt_owned_draw(state, catalog, n, caller, events)
}

/// [`draw_cards`], reporting the exact drawn identities into `drawn`.
///
/// # Suspension
///
/// Python's Draw can suspend (`DRAW_SUSPENDED`, #616) when the Early
/// `AfterCardDrawn` listener's AutoPlay blocks on a player choice, or when a
/// reshuffle's `AfterShuffle` selection does. Both need a subscriber on
/// [`HookEvent::AfterCardDrawn`] or a Stratagem-class relic, and the admission
/// gate refuses an entry carrying either — [`fire_hook`] returns
/// [`EngineRefusal::HookNotModeled`] rather than suspending. So the loop below
/// is the whole command, and a caller may treat `drawn` as complete.
pub fn draw_cards_into(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    source: DrawSource,
    events: &mut Vec<Event>,
    drawn: &mut impl DrawnCards,
) -> Result<(), EngineRefusal> {
    if state.card_states.dampen().is_some() {
        if !super::cards::dampen_state_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs("Dampen public draw entry"));
        }
        if let Some(guard) = DampenDrawRehearsalGuard::enter() {
            let mut probe = state.clone();
            draw_cards_into_inner(
                &mut probe,
                catalog,
                n,
                source,
                &mut Vec::new(),
                &mut NoDrawnCards,
            )?;
            drop(guard);
        }
    }
    draw_cards_into_inner(state, catalog, n, source, events, drawn)
}

fn draw_cards_into_inner(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    source: DrawSource,
    events: &mut Vec<Event>,
    drawn: &mut impl DrawnCards,
) -> Result<(), EngineRefusal> {
    // Fiddle.ShouldDraw permits the unique turn-start HandDraw, non-owner
    // draws, and draws made while the enemy side is active. This solo state
    // owns every represented pile, so an ordinary player-side command is the
    // sole denied branch (`_fiddle_should_draw`, frozen Python, deleted #2827).
    if catalog.hooks().owns(crate::ids::RelicId::RelicFiddle)
        && source == DrawSource::Command
        && state.player_side_active
    {
        crate::coverage::record_relic(crate::ids::RelicId::RelicFiddle);
        return Ok(());
    }
    // NoDrawPower.ShouldDraw permits exactly the turn-start HandDraw call.
    // This gate precedes hand capacity, pile reads and reshuffle RNG.
    if state.powers.value(PowerId::NoDraw) > 0 && source == DrawSource::Command {
        return Ok(());
    }
    // A Strike AutoPlay cannot install or remove Hellraiser. Freeze this one
    // sparse lookup for the complete Draw command; the ordinary no-power path
    // pays no per-card lookup or clone.
    let hellraiser_live = state.powers.value(PowerId::Hellraiser) > 0;
    for _ in 0..n {
        if state.history.over {
            break;
        }
        if state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
            break;
        }
        if state.piles.get(PileId::Draw).is_empty() {
            if state.piles.get(PileId::Discard).is_empty() {
                break;
            }
            reshuffle(state, catalog, events)?;
            if state.piles.get(PileId::Draw).is_empty() {
                break;
            }
        }
        let card = state.piles.get(PileId::Draw).as_slice()[0];
        repair_card_play_after_physical_move(state, card, PileId::Draw, PileId::Hand)?;
        let card = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        state.cards_drawn_combat = state
            .cards_drawn_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("cards_drawn_combat"))?;
        if source == DrawSource::Command {
            state.history.non_hand_draws_this_turn = state
                .history
                .non_hand_draws_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("non_hand_draws_this_turn"))?;
        }
        // Card type is a separate native axis from the playable
        // `is_status_curse` convenience flag. Read the generated native type
        // rather than an identity allowlist so newly reachable Status rows
        // enter this generic choke point without invalidating it.
        if catalog.spec(card.atom).is_some_and(|spec| spec.is_status) {
            state.history.status_draws_this_turn = state
                .history
                .status_draws_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("status_draws_this_turn"))?;
        }
        events.push(Event::CardDrawn { uid: card.uid });
        let hellraiser_uid_path =
            hellraiser_live && catalog.spec(card.atom).is_some_and(|spec| spec.strike_tag);
        if hellraiser_uid_path {
            hellraiser_after_card_drawn_early(state, catalog, card.uid, events)?;
        }
        let listener_card = if hellraiser_uid_path {
            drawn_object_card(state, card.uid)?
        } else {
            card
        };
        // Hook creates the ordinary listener snapshot only after the complete
        // early enumerator. A terminal Hellraiser child makes that snapshot
        // empty — every ordinary listener, the drawn card's own included
        // (#3099); the Draw itself still completes.
        let walk_started = after_card_drawn_walk_starts(state);
        if walk_started {
            powers_after_card_drawn(state, catalog, listener_card, source, None, false, events)?;
            fire_hook(catalog, HookEvent::AfterCardDrawn, state, events)?;
        }
        physical_card_after_drawn(state, catalog, listener_card, walk_started)?;
        slither_after_card_drawn(state, catalog, listener_card, walk_started)?;
        // Void's native CardModel.AfterCardDrawn listener runs after the
        // ordinary power and physical-card walks.
        let completed = if hellraiser_uid_path {
            drawn_object_card(state, card.uid)?
        } else {
            card
        };
        void_after_card_drawn(state, catalog, completed, walk_started)?;
        drawn.push(completed);
    }
    Ok(())
}

/// `CardCmd.DiscardAndDraw`'s ordered discard-then-draw
/// (`_discard_and_draw`, frozen Python, deleted #2827).
///
/// `cards` are still live in `source`, in the order the command receives
/// them (the player's pick order for selections, #3102): the command detaches
/// each one itself behind the native gates (#3075). v0.111.0 DLL 9cb4f1ad,
/// `CardCmd/<DiscardAndDraw>d__4::MoveNext` RVA 0x3e0274:
///
/// - IL_0029–0035: one entry gate on `CombatManager.IsOverOrEnding`, so a
///   command issued after combat is over is a complete no-op (no history,
///   no hook, no Draw, no Sly); IL_003c–0058 then returns on an empty
///   `ToList` snapshot, before the Draw as well;
/// - per card, Sly is captured at IL_00f6 before `CardPileCmd.Add` at
///   IL_011d, and `CardPileCmd/<Add>d__10::MoveNext` (RVA 0x3e1ba4)
///   IL_004e–008a returns without moving the card once
///   `CombatManager.IsEnding` holds. The loop itself has no ending break, so
///   `CombatHistory.CardDiscarded` (IL_018e) and `Hook.AfterCardDiscarded`
///   (IL_01a5) still run for every card, and a kill mid-loop (Tingsha) leaves
///   the remaining cards in their source pile;
/// - IL_0246–0271: the one paired `CardPileCmd.Draw`, awaited before the Sly
///   auto-play pass over the collected cards. Each exact UID is resolved from
///   its live pile/payload only when its turn arrives, because an earlier
///   Sly child can move or upgrade a later one.
///
/// Rust projects both IsOverOrEnding and IsEnding to `history.over`
/// ([`card_cmd_discard_is_gated`]). The complete command is rehearsed on a
/// clone before its first history entry, RNG draw, or pile write.
pub fn discard_and_draw(
    state: &mut HotState,
    catalog: &Catalog,
    source: PileId,
    cards: &[HotCard],
    cards_to_draw: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let mut probe = state.clone();
    discard_and_draw_inner(
        &mut probe,
        catalog,
        source,
        cards,
        cards_to_draw,
        &mut Vec::new(),
    )?;
    discard_and_draw_inner(state, catalog, source, cards, cards_to_draw, events)
}

/// Whether `CardCmd.DiscardAndDraw` is a complete no-op at entry.
///
/// v0.111.0 DLL 9cb4f1ad, `CardCmd/<DiscardAndDraw>d__4::MoveNext` (RVA
/// 0x3e0274) IL_0029–0035 leaves on `CombatManager.IsOverOrEnding`; its
/// nested `CardPileCmd.Add` (RVA 0x3e1ba4, IL_004e–008a) skips the move on
/// `IsEnding`. Both project to `history.over` here, the same projection as
/// [`card_cmd_exhaust_is_gated`], so a caller's entry test and the per-card
/// detach inside [`discard_cards_collect_sly`] can never disagree.
pub(crate) fn card_cmd_discard_is_gated(state: &HotState) -> bool {
    state.history.over
}

/// Scrape's ordered live returned-card occurrences. CardCmd.DiscardAndDraw
/// RVA0x3e0274 freezes the filter before the loop, captures Sly at00f6, awaits
/// Add at011d, then records history0178 and invokes AfterCardDiscarded01a5 for
/// every occurrence. CardPileCmd.Add0x3e1ba4 removes an existing pile member
/// at054b and appends at062e even when source and destination are Discard.
/// This source-specific helper leaves the generic pre-removed callers alone.
///
/// A removed DUPE Strike that Hellraiser auto-played inside Scrape's Draw
/// (#3136, #2683) is still one of the returned references. Native keeps it in
/// the frozen list: `CardPileCmd/<Add>d__10::MoveNext` RVA `0x3e1ba4` reads
/// `CardModel.HasBeenRemovedFromState` at IL_00ea and, for a removed card,
/// appends a failed `CardPileAddResult` at IL_0119-0166 without touching any
/// pile, so there is no move and no pile callback. DiscardAndDraw has no
/// success check: it still records `CombatHistory.CardDiscarded` (IL_0178) and
/// awaits `Hook.AfterCardDiscarded` (IL_01a5) for that occurrence. Its
/// command-level combat state and Discard pile come from the first card's
/// `CombatState`, falling back to `Owner.Creature.CombatState` when a removed
/// first card has no pile (IL_005f-008e, `CardModel.get_CombatState` RVA
/// `0x7d184`), and from `Owner` (IL_00a7-00b6): the same solo combat either
/// way. A removed card's Sly capture at IL_00f6 would later AutoPlay an object
/// outside every pile; no admitted writer can mark a Strike Sly, so that shape
/// refuses by name rather than being modeled.
pub(crate) fn discard_scrape_occurrences(
    state: &mut HotState,
    catalog: &Catalog,
    occurrences: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        occurrences: &[HotCard],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if state.history.over || occurrences.is_empty() {
            return Ok(());
        }
        let mut sly = Vec::new();
        for occurrence in occurrences {
            let Some((pile, index)) =
                super::play::unique_live_card_location(state, occurrence.uid)?
            else {
                discard_removed_scrape_occurrence(state, catalog, occurrence.uid)?;
                state.history.discarded_cards_this_turn = state
                    .history
                    .discarded_cards_this_turn
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("discarded_cards_this_turn"))?;
                super::relics::after_card_discarded(catalog, state, events)?;
                continue;
            };
            let live = state.piles.get(pile).as_slice()[index];
            let spec = catalog
                .spec(live.atom)
                .ok_or(EngineRefusal::UnknownAtom(live.atom))?;
            if state.card_states.get(live.uid).is_sly(spec.sly) {
                // Only Hellraiser's Strike Attacks can leave Hand during
                // the admitted Draw callback closure. They cannot be Sly:
                // native Master Planner/Hand Trick only mark Skills. Keep
                // this boundary explicit if future callback writers expand.
                if sly.iter().any(|earlier: &HotCard| earlier.uid == live.uid) {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
                sly.push(live);
            }
            if !state.history.over && state.hp > 0 {
                repair_card_play_after_physical_move(state, live, pile, PileId::Discard)?;
                let moved = state.piles.get_mut(pile).make_mut().remove(index);
                state.piles.get_mut(PileId::Discard).make_mut().push(moved);
                state.exact_piles = true;
                events.push(Event::CardResolved {
                    uid: live.uid,
                    pile: PileId::Discard,
                });
            }
            state.history.discarded_cards_this_turn = state
                .history
                .discarded_cards_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("discarded_cards_this_turn"))?;
            super::relics::after_card_discarded(catalog, state, events)?;
        }
        // Native stored references rather than snapshots: later callbacks may
        // change earlier captured objects. Refresh once at batch publication.
        for card in &mut sly {
            let (pile, index) = super::play::unique_live_card_location(state, card.uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            *card = state.piles.get(pile).as_slice()[index];
        }
        super::play::autoplay_sly_discard_batch(state, catalog, &sly, events)
    }
    let mut probe = state.clone();
    apply(&mut probe, catalog, occurrences, &mut Vec::new())?;
    apply(state, catalog, occurrences, events)
}

/// The pile-free half of one removed Scrape occurrence (see
/// [`discard_scrape_occurrences`]): authenticate the retained object and
/// refuse its unmodeled Sly capture. `CardPileCmd.Add` moves nothing.
fn discard_removed_scrape_occurrence(
    state: &HotState,
    catalog: &Catalog,
    uid: u32,
) -> Result<(), EngineRefusal> {
    let entry = state
        .card_states
        .removed_draw_object(uid)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let spec = catalog
        .spec(entry.card.atom)
        .ok_or(EngineRefusal::UnknownAtom(entry.card.atom))?;
    if entry.state.is_sly(spec.sly) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

/// Scrape's filter cost for a removed returned object.
///
/// `CardEnergyCost.GetWithModifiers` RVA `0x11e044` returns the base for an
/// unplayable (IL_0022-002c) or X-cost (IL_002d-0036) card, applies the
/// card-local modifiers in order (IL_0037-0080), and walks the global
/// `Hook.ModifyEnergyCostInCombat` only when `CardModel.CombatState` is
/// non-null (IL_0094-009f); it then clamps at zero (IL_00c3-00ca).
/// `CardModel.get_CombatState` RVA `0x7d184` is null for a card with no pile
/// unless it is an upgrade preview (IL_0014-003f), and `RemoveFromState` RVA
/// `0x7dbb2` removes the pile, so a removed DUPE Strike prices from its own
/// retained local rows only: no Free*/Corruption/Void Form/Scarf term.
pub(crate) fn removed_draw_object_energy_cost(
    state: &HotState,
    catalog: &Catalog,
    uid: u32,
) -> Result<Option<i64>, EngineRefusal> {
    let Some(entry) = state.card_states.removed_draw_object(uid) else {
        return Ok(None);
    };
    drawn_object_card(state, uid)?;
    let spec = catalog
        .spec(entry.card.atom)
        .ok_or(EngineRefusal::UnknownAtom(entry.card.atom))?;
    if spec.x_cost || spec.cost < 0 {
        return Ok(Some(spec.cost));
    }
    Ok(Some(
        entry
            .state
            .local_cost_modifiers
            .resolve_signed(spec.cost)
            .max(0),
    ))
}

/// Shadow Step and Storm of Steel's exact captured-Hand `CardCmd::Discard`.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `StormOfSteel/<OnPlay>d__3::MoveNext` RVA `0x3bf6b8` snapshots Hand at
/// IL `0x002c-0x004a` and awaits plural Discard at `0x004f-0x00ae`.
/// `ShadowStep/<OnPlay>d__3::MoveNext` RVA `0x3ba870` obtains that same live
/// Hand enumerable at IL `0x0024-0x0040` and awaits it before applying its
/// marker at `0x0095-0x00bd`.
/// `CardCmd/<DiscardAndDraw>d__4::MoveNext` RVA `0x3e0274` performs its sole
/// ending check before snapshotting at IL `0x0029-0x0035`; its enumerator has
/// no loop-level ending break. Every frozen card therefore reaches the
/// CardDiscarded/history hook at IL `0x0178-0x01a5`, even when a prior hook
/// made the nested pile Add a no-op. Collected Sly cards run only after the
/// complete frozen discard walk.
///
/// Rust's admitted discard listener set cannot currently end combat inside
/// the loop, but retaining the native no-break history structure here keeps
/// the primitive exact as those listeners grow. The complete command is
/// rehearsed before the first pile/history mutation, and exact live UIDs are
/// required throughout.
pub(crate) fn discard_frozen_hand(
    state: &mut HotState,
    catalog: &Catalog,
    frozen: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        frozen: &[HotCard],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if state.history.over {
            return Ok(());
        }
        let mut sly_cards = Vec::new();
        for frozen_card in frozen {
            let (pile, index) = super::play::unique_live_card_location(state, frozen_card.uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let live = state.piles.get(pile).as_slice()[index];
            if pile != PileId::Hand || live != *frozen_card {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            let is_sly = catalog
                .spec(live.atom)
                .is_some_and(|spec| state.card_states.get(live.uid).is_sly(spec.sly));

            // CardPileCmd.Add is the only per-card ending gate. If an earlier
            // discard hook ended combat, the exact later object stays where
            // it is, but native still records CardDiscarded and calls the
            // hook for this enumerator entry.
            if !state.history.over {
                repair_card_play_after_physical_move(state, live, PileId::Hand, PileId::Discard)?;
                let removed = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
                debug_assert_eq!(removed, live);
                state
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(removed);
                events.push(Event::CardResolved {
                    uid: removed.uid,
                    pile: PileId::Discard,
                });
            }
            state.history.discarded_cards_this_turn = state
                .history
                .discarded_cards_this_turn
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("discarded_cards_this_turn"))?;
            super::relics::after_card_discarded(catalog, state, events)?;
            if is_sly {
                sly_cards.push(live);
            }
        }
        if super::replay_lifecycle_is_active() {
            super::play::autoplay_sly_discard_batch(state, catalog, &sly_cards, events)
        } else {
            super::play::autoplay_collected_cards(state, catalog, &sly_cards, events)
        }
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, frozen, &mut Vec::new())?;
    apply(state, catalog, frozen, events)
}

/// `CardCmd.DiscardAndDraw`'s ordered discard walk over `cards`, still live
/// in `source`: each card is detached and appended to Discard/Bottom unless
/// combat has ended, and every card records its history entry and fires
/// `AfterCardDiscarded` either way. Returns the Sly subset in walk order.
/// Shared by the synchronous command and by Calculated Gamble's
/// suspend-capable owned tail (#2667), which issues its paired Draw between
/// this walk and the batch replay.
///
/// IL (v0.111.0, RVA 0x3e0274, see [`discard_and_draw`]): the entry gate at
/// IL_0029–0035 and the empty-list return at IL_0056 make the whole walk a
/// no-op; the per-card `CardPileCmd.Add` IsEnding gate (RVA 0x3e1ba4,
/// IL_004e–008a) skips only the move, never history (IL_018e) or the hook
/// (IL_01a5). Callers that must also skip their paired Draw test
/// [`card_cmd_discard_is_gated`] first.
///
/// Every persisted CardPlay owner of the batch is proven repairable before
/// the first mutation, then repaired immediately before its own move. A card
/// that is no longer in `source` when its turn arrives refuses by name; no
/// admitted discard listener moves cards.
pub(crate) fn discard_cards_collect_sly(
    state: &mut HotState,
    catalog: &Catalog,
    source: PileId,
    cards: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<Vec<HotCard>, EngineRefusal> {
    if card_cmd_discard_is_gated(state) || cards.is_empty() {
        return Ok(Vec::new());
    }
    let moves: Vec<_> = cards
        .iter()
        .map(|card| (*card, source, PileId::Discard))
        .collect();
    preflight_card_play_after_physical_moves(state, &moves)?;
    let mut sly_cards = Vec::new();
    for card in cards {
        let is_sly = catalog
            .spec(card.atom)
            .is_some_and(|spec| state.card_states.get(card.uid).is_sly(spec.sly));
        if !card_cmd_discard_is_gated(state) {
            let index = state
                .piles
                .get(source)
                .as_slice()
                .iter()
                .position(|live| live == card)
                .ok_or(EngineRefusal::FrozenCardVanished {
                    uid: card.uid,
                    pile: source,
                })?;
            repair_card_play_after_physical_move(state, *card, source, PileId::Discard)?;
            let removed = state.piles.get_mut(source).make_mut().remove(index);
            debug_assert_eq!(removed, *card);
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(removed);
            events.push(Event::CardResolved {
                uid: card.uid,
                pile: PileId::Discard,
            });
        }
        if is_sly {
            sly_cards.push(*card);
        }
        state.history.discarded_cards_this_turn = state
            .history
            .discarded_cards_this_turn
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("discarded_cards_this_turn"))?;
        super::relics::after_card_discarded(catalog, state, events)?;
    }
    Ok(sly_cards)
}

fn discard_and_draw_inner(
    state: &mut HotState,
    catalog: &Catalog,
    source: PileId,
    cards: &[HotCard],
    cards_to_draw: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if card_cmd_discard_is_gated(state) || cards.is_empty() {
        return Ok(());
    }
    let sly_cards = discard_cards_collect_sly(state, catalog, source, cards, events)?;
    if cards_to_draw > 0 && !state.history.over {
        draw_cards(state, catalog, cards_to_draw, DrawSource::Command, events)?;
    }
    super::play::autoplay_sly_discard_batch(state, catalog, &sly_cards, events)
}

/// `CardCmd.Exhaust` (`_card_exhausted`, frozen Python, deleted #2827).
///
/// The caller commits the source-pile removal first, so reentrant listeners
/// observe both the removal and the live Exhaust contents. The command itself
/// is skipped once combat has ended, and then native never detached the card
/// either: callers remove it through [`detach_card_for_exhaust`] (or test
/// [`card_cmd_exhaust_is_gated`] first) so it stays in its source pile.
///
/// The admitted player-power walk preserves the recorded application order:
/// Dark Embrace issues its ordinary Draw before the next listener and Feel No
/// Pain gains flat block. Joss Paper remains a refused relic. Midnight's
/// rebate is admitted through the frozen all-piles UID snapshot and live
/// UID-resolved suffix.
///
/// The card-local listener that *is* reachable is Drum of Battle's own
/// (`_drum_of_battle_after_card_exhausted`, frozen Python, deleted #2827): exhausting a
/// DRUM_OF_BATTLE folds the admitted GeneratePlayCount listeners once, then
/// grants the printed 2 / upgraded 3 Energy once per frozen play count. Burst
/// contributes one and consumes one stack before Echo Form observes the
/// unchanged `plays_this_turn`. Card-owned BaseReplayCount is physical and
/// contributes again to this fresh fold. Duplication is likewise represented;
/// Throwing Axe remains outside Rust's state surface. `NoEnergyGain` is
/// boundary-representable and ledger-authenticated but independently
/// admission-refused until all local Energy-gain modifiers are modeled.
pub fn card_exhausted(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    match card_exhausted_with_continuation(
        state,
        catalog,
        card,
        false,
        AfterCardExhaustedPowerRecord::for_return(AfterCardExhaustedReturnKind::ReturnOnly),
        events,
    )? {
        CardExhaustedResult::Complete => Ok(()),
        CardExhaustedResult::Suspended => Err(EngineRefusal::ContinuationNotModeled),
    }
}

/// Whether `CardCmd.Exhaust` is a no-op because combat has already ended.
///
/// v0.111.0 DLL 9cb4f1ad, `CardCmd/<Exhaust>d__6::MoveNext` (RVA 0x3e06c8):
/// IL_0020–0034 returns on `CombatManager.IsOverOrEnding` before
/// `CardPileCmd.Add(card, Exhaust)` at IL_0072, and Add is what detaches the
/// card from its current pile. This is the single projection shared by the
/// physical detach below and the `card_exhausted_*` transaction entries, so
/// a caller can never remove a card whose Exhaust is then skipped (#3041).
pub(crate) fn card_cmd_exhaust_is_gated(state: &HotState) -> bool {
    state.history.over
}

/// Physical half of `CardCmd.Exhaust` for the card at `pile[index]`.
///
/// Returns `None` and leaves every pile and CardPlay owner untouched when
/// [`card_cmd_exhaust_is_gated`] holds: native `CardCmd/<Exhaust>d__6`
/// (RVA 0x3e06c8, IL_0020–0034) returns before `CardPileCmd.Add` (IL_0072),
/// so the card stays in its source pile. Otherwise it repairs any persisted
/// CardPlay owner of the card, removes it, and returns it for the caller's
/// `card_exhausted_*` transaction. Every caller that removes a card and then
/// runs `card_exhausted_*` goes through here (or checks the same gate in a
/// loop first), so a fight-ending kill can never make a card vanish.
pub(crate) fn detach_card_for_exhaust(
    state: &mut HotState,
    pile: PileId,
    index: usize,
) -> Result<Option<HotCard>, EngineRefusal> {
    if card_cmd_exhaust_is_gated(state) {
        return Ok(None);
    }
    let card = *state
        .piles
        .get(pile)
        .as_slice()
        .get(index)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    repair_card_play_after_physical_move(state, card, pile, PileId::Exhaust)?;
    let removed = state.piles.get_mut(pile).make_mut().remove(index);
    debug_assert_eq!(removed, card);
    Ok(Some(removed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardExhaustedResult {
    Complete,
    Suspended,
}

/// Whether an ordinary Exhaust can need a persisted callback owner.
pub(crate) fn ordinary_after_card_exhausted_owner_is_live(
    state: &HotState,
    catalog: &Catalog,
) -> bool {
    !state.history.over
        && state.powers.value(PowerId::DarkEmbrace) > 0
        && catalog.after_card_exhausted_dark_can_suspend()
}

/// Ordinary Exhaust entry with an exact producer-owned return cursor.
/// `continuation` carries only producer state; the common object snapshot is
/// filled after the card is normalized and appended to Exhaust.
pub(crate) fn card_exhausted_with_owner(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    continuation: AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<CardExhaustedResult, EngineRefusal> {
    card_exhausted_with_continuation(state, catalog, card, false, continuation, events)
}

pub(crate) fn card_exhausted_return_only(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    events: &mut Vec<Event>,
) -> Result<CardExhaustedResult, EngineRefusal> {
    card_exhausted_with_owner(
        state,
        catalog,
        card,
        AfterCardExhaustedPowerRecord::for_return(AfterCardExhaustedReturnKind::ReturnOnly),
        events,
    )
}

/// `ReturnOnly` exhaust whose continuation is the currently persisted card
/// body. The caller's step cursor was committed before dispatch, so a parked
/// Dark Embrace child returns to that exact card/step instead of allowing the
/// ordinary runner to continue past a live child.
pub(crate) fn card_exhausted_cardplay_return_only(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    source_uid: u32,
    events: &mut Vec<Event>,
) -> Result<CardExhaustedResult, EngineRefusal> {
    let mut continuation =
        AfterCardExhaustedPowerRecord::for_return(AfterCardExhaustedReturnKind::ReturnOnly);
    if ordinary_after_card_exhausted_owner_is_live(state, catalog) {
        continuation.source_uid = Some(source_uid);
        continuation.step_index =
            super::play::after_card_exhausted_source_step_cursor(state, source_uid)?;
    }
    card_exhausted_with_owner(state, catalog, card, continuation, events)
}

/// Exact `CardCmd.Exhaust` entry for the ordinary turn-end Ethereal pass.
/// Only that native caller sets `causedByEthereal`; every other Rust caller
/// keeps the ordinary immediate-Dark-Embrace behavior above.
pub(crate) fn card_exhausted_by_ethereal(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    card_exhausted_with_continuation(
        state,
        catalog,
        card,
        true,
        AfterCardExhaustedPowerRecord::for_return(AfterCardExhaustedReturnKind::ReturnOnly),
        events,
    )
    .map(|_| ())
}

fn card_exhausted_with_continuation(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    caused_by_ethereal: bool,
    continuation: AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<CardExhaustedResult, EngineRefusal> {
    if card_cmd_exhaust_is_gated(state) {
        return Ok(CardExhaustedResult::Complete);
    }
    let dampen_is_live = state.card_states.dampen().is_some();
    let resumable_dark_is_live =
        !caused_by_ethereal && ordinary_after_card_exhausted_owner_is_live(state, catalog);
    if dampen_is_live || resumable_dark_is_live {
        if dampen_is_live && !super::cards::dampen_exhaust_entry_is_exact(state, catalog, card) {
            return Err(EngineRefusal::MalformedArgs("Dampen card-exhaust entry"));
        }
        let mut next = state.clone();
        let mut emitted = Vec::new();
        let result = card_exhausted_inner(
            &mut next,
            catalog,
            card,
            caused_by_ethereal,
            continuation,
            &mut emitted,
        )?;
        *state = next;
        events.extend(emitted);
        return Ok(result);
    }
    card_exhausted_inner(
        state,
        catalog,
        card,
        caused_by_ethereal,
        continuation,
        events,
    )
}

fn card_exhausted_inner(
    state: &mut HotState,
    catalog: &Catalog,
    card: HotCard,
    caused_by_ethereal: bool,
    mut continuation: AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<CardExhaustedResult, EngineRefusal> {
    // Python's `_preflight_card_exhausted` protects a Feel No Pain block walk
    // that can suspend through Juggernaut/death/Gremlin Horn. Those peer
    // subscribers remain refused at the gate, so that preflight is inert here.
    catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    let mut exhaust_order = state.fanouts.after_card_exhausted_order().to_vec();
    if exhaust_order.is_empty() {
        exhaust_order.extend(
            [PowerId::DarkEmbrace, PowerId::FeelNoPain]
                .into_iter()
                .filter(|power| state.powers.value(*power) > 0),
        );
    }
    let resumable_owner_needed = !caused_by_ethereal
        && exhaust_order.contains(&PowerId::DarkEmbrace)
        && ordinary_after_card_exhausted_owner_is_live(state, catalog);
    let exhaust_index = state.piles.get(PileId::Exhaust).len();
    state.piles.get_mut(PileId::Exhaust).make_mut().push(card);
    let midnight_is_live = [
        PileId::Hand,
        PileId::Draw,
        PileId::Discard,
        PileId::Exhaust,
        PileId::Play,
    ]
    .into_iter()
    .flat_map(|pile| state.piles.get(pile).as_slice())
    .any(|candidate| {
        catalog
            .spec(candidate.atom)
            .is_some_and(|candidate_spec| matches!(candidate_spec.identity.id, CardId::Midnight))
    });
    if midnight_is_live || resumable_owner_needed {
        // Native ForceExact precedes the all-piles CardModel snapshot. This
        // also re-identifies the just-added Exhaust object; re-resolve it by
        // its fixed insertion position before the later Drum/local tails.
        super::cards::normalize_card_identities(state)?;
    }
    let card = state.piles.get(PileId::Exhaust).as_slice()[exhaust_index];
    state.history.owner_card_exhausted_this_turn = true;
    state.history.owner_cards_exhausted_combat = state
        .history
        .owner_cards_exhausted_combat
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_cards_exhausted_combat",
        ))?;
    events.push(Event::CardResolved {
        uid: card.uid,
        pile: PileId::Exhaust,
    });
    let midnight = midnight_snapshot(state, catalog);
    if resumable_owner_needed {
        continuation.listeners = exhaust_order;
        continuation.card_uid = card.uid;
        continuation.midnight_uids = midnight.iter().map(|card| card.uid).collect();
        state
            .frames
            .push_after_card_exhausted_power(&continuation)
            .ok_or(EngineRefusal::CounterOverflow(
                "AfterCardExhausted owner store",
            ))?;
        advance_top_after_card_exhausted_power(state, catalog, events)?;
        return Ok(if state.pending.is_some() {
            CardExhaustedResult::Suspended
        } else {
            CardExhaustedResult::Complete
        });
    }
    for power in exhaust_order {
        let amount = state.powers.value(power);
        if amount <= 0 {
            continue;
        }
        match power {
            PowerId::DarkEmbrace => {
                if caused_by_ethereal {
                    let tally = state.fanouts.dark_embrace_ethereal().checked_add(1).ok_or(
                        EngineRefusal::CounterOverflow("dark embrace ethereal tally"),
                    )?;
                    state.fanouts.set_dark_embrace_ethereal(tally);
                } else {
                    let draws: usize = amount
                        .try_into()
                        .map_err(|_| EngineRefusal::CounterOverflow("dark embrace draw"))?;
                    draw_cards(state, catalog, draws, DrawSource::Command, events)?;
                }
            }
            PowerId::FeelNoPain => {
                if !state.history.over {
                    super::orbs::gain_flat_block(state, catalog, i64::from(amount), events)?;
                }
            }
            _ => {
                return Err(EngineRefusal::PowerHookNotModeled {
                    power,
                    event: HookEvent::AfterCardExhausted,
                });
            }
        }
    }
    finish_after_card_exhausted_suffix(
        state,
        catalog,
        card.uid,
        midnight.iter().map(|card| card.uid),
        caused_by_ethereal,
        events,
    )?;
    Ok(CardExhaustedResult::Complete)
}

fn finish_after_card_exhausted_suffix(
    state: &mut HotState,
    catalog: &Catalog,
    card_uid: u32,
    midnight_uids: impl IntoIterator<Item = u32>,
    caused_by_ethereal: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (pile, index) = super::play::unique_live_card_location(state, card_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if pile != PileId::Exhaust {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let card = state.piles.get(pile).as_slice()[index];
    let spec = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    let CardIdentity { id, upgrade, .. } = spec.identity;
    super::relics::after_card_exhausted(catalog, state, card, caused_by_ethereal, events)?;
    // Drum's GainEnergy command is independently ending-gated. A preceding
    // power listener can end combat reentrantly (for example Dark Embrace
    // draws into lethal Cacophony), in which case the card-local tail stops
    // without granting energy.
    if id == CardId::DrumOfBattle && !state.history.over {
        // Native invokes the card-local listener only after the ordinary
        // power/relic suffix. Derive and preflight the frozen plan here: a
        // reentrant lethal listener suppresses Drum entirely, while an
        // overflow still refuses before Burst or Energy changes in this tail.
        let (play_count, energy) = drum_of_battle_plan(state, spec, card, upgrade)?;
        // GeneratePlayCount freezes the complete count before the first
        // GainEnergy command. Burst's changed listener runs before Echo Form's
        // predicate; neither changes the history value Echo reads.
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
        if state.fanouts.throwing_axe_available() {
            state.fanouts.set_throwing_axe_available(false);
            crate::coverage::record_relic(RelicId::RelicThrowingAxe);
        }
        for _ in 0..play_count {
            // GainEnergy is independently ending-gated in native. No current
            // admitted Energy-gain hook can end combat, but keep the gate at
            // the command boundary rather than collapsing the frozen fold.
            if state.history.over {
                break;
            }
            state.energy = state
                .energy
                .checked_add(energy)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
        }
    }
    apply_midnight_uid_snapshot(state, catalog, midnight_uids)?;
    Ok(())
}

/// Resume the record-owned ordinary listener cursor after Dark Embrace's
/// Draw child has drained. Cursor advancement is stored before each callback,
/// and the exhausted object is re-resolved by UID from Exhaust for the tail.
pub(crate) fn advance_top_after_card_exhausted_power(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    loop {
        let record = match state.frames.top() {
            Some(crate::frame::Frame::AfterCardExhaustedPower { record }) => state
                .frames
                .after_card_exhausted_power(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?
                .to_owned(),
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        if let Some(power) = record.listeners.get(record.cursor as usize).copied() {
            let mut advanced = record.clone();
            advanced.cursor =
                advanced
                    .cursor
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow(
                        "AfterCardExhausted listener cursor",
                    ))?;
            state
                .frames
                .replace_top_after_card_exhausted_power(&advanced)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let amount = state.powers.value(power);
            if amount <= 0 {
                continue;
            }
            match power {
                PowerId::DarkEmbrace => {
                    let draws = usize::try_from(amount)
                        .map_err(|_| EngineRefusal::CounterOverflow("dark embrace draw"))?;
                    if draw_cards_for_potion(
                        state,
                        catalog,
                        draws,
                        DrawCaller::AfterCardExhaustedPower,
                        events,
                    )? == PotionDrawResult::Suspended
                    {
                        return Ok(());
                    }
                }
                PowerId::FeelNoPain => {
                    if !state.history.over {
                        super::orbs::gain_flat_block(state, catalog, i64::from(amount), events)?;
                    }
                }
                _ => {
                    return Err(EngineRefusal::PowerHookNotModeled {
                        power,
                        event: HookEvent::AfterCardExhausted,
                    });
                }
            }
            continue;
        }
        if !record.ordinary_suffix_invoked {
            let mut completed = record.clone();
            completed.ordinary_suffix_invoked = true;
            state
                .frames
                .replace_top_after_card_exhausted_power(&completed)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            finish_after_card_exhausted_suffix(
                state,
                catalog,
                record.card_uid,
                record.midnight_uids.iter().copied(),
                false,
                events,
            )?;
            continue;
        }
        return super::play::resume_after_card_exhausted_owner(state, catalog, record, events);
    }
}

/// Freeze Midnight's native all-piles listener order at CardExhausted entry.
/// This is deliberately not `PileId::ALL`, whose enum order differs from the
/// native/Python `(Hand, Draw, Discard, Exhaust, Play)` tuple.
pub(crate) fn midnight_snapshot(state: &HotState, catalog: &Catalog) -> Vec<HotCard> {
    [
        PileId::Hand,
        PileId::Draw,
        PileId::Discard,
        PileId::Exhaust,
        PileId::Play,
    ]
    .into_iter()
    .flat_map(|pile| state.piles.get(pile).as_slice())
    .copied()
    .filter(|card| {
        catalog
            .spec(card.atom)
            .is_some_and(|spec| matches!(spec.identity.id, CardId::Midnight))
    })
    .collect()
}

/// Apply one completed Midnight physical-listener snapshot.
///
/// The snapshot authenticates UIDs at capture time. Each callback re-resolves
/// the physical object and skips a vanished UID. The current admitted child
/// closure cannot transform a captured Midnight; fail closed if that invariant
/// is violated instead of applying its rebate to a different identity.
pub(crate) fn apply_midnight_snapshot(
    state: &mut HotState,
    catalog: &Catalog,
    snapshot: &[HotCard],
) -> Result<(), EngineRefusal> {
    apply_midnight_uid_snapshot(state, catalog, snapshot.iter().map(|card| card.uid))
}

fn apply_midnight_uid_snapshot(
    state: &mut HotState,
    catalog: &Catalog,
    snapshot: impl IntoIterator<Item = u32>,
) -> Result<(), EngineRefusal> {
    for uid in snapshot {
        let Some((pile, index)) = super::play::unique_live_card_location(state, uid)? else {
            continue;
        };
        let live = state.piles.get(pile).as_slice()[index];
        if !catalog
            .spec(live.atom)
            .is_some_and(|spec| matches!(spec.identity.id, CardId::Midnight))
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        state.card_states.append_local_cost_modifier(
            live.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.exact_piles = true;
    }
    Ok(())
}

/// `_drum_of_battle_after_card_exhausted`'s `(2, 3)[card[1]]` energy table.
const DRUM_OF_BATTLE_EXHAUST_ENERGY: [i16; 2] = [2, 3];

/// Freeze the admitted portion of Drum's listener-local GeneratePlayCount.
///
/// Python first includes card-owned replay count, then walks Duplication,
/// Burst, Echo Form and Throwing Axe. Rust represents the physical replay,
/// Burst, Echo, Duplication, and Throwing Axe sources.
/// Returning the printed Energy beside the count lets the caller reject the
/// shared 256-body or Energy bounds before Burst consumption or any mutation
/// in the card-local tail.
pub(crate) fn drum_of_battle_plan(
    state: &HotState,
    spec: &crate::catalog::CardSpec,
    card: HotCard,
    upgrade: u8,
) -> Result<(usize, i16), EngineRefusal> {
    let energy = DRUM_OF_BATTLE_EXHAUST_ENERGY
        .get(usize::from(upgrade))
        .copied()
        .ok_or(EngineRefusal::CounterOverflow("drum_of_battle level"))?;
    let physical = state.card_states.get(card.uid).base_replay_count();
    if physical.is_some() && card.flags & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0 {
        return Err(EngineRefusal::MalformedArgs(
            "physical BaseReplayCount owner",
        ));
    }
    let replay = super::play::effective_replay_count(state, spec, card.uid)?;
    let burst = i64::from(state.powers.value(PowerId::Burst) > 0);
    // Each positive Duplicator listener contributes one replay and consumes
    // one Intensity, rather than contributing its entire remaining amount.
    let duplication = i64::from(state.fanouts.duplication() > 0);
    let echo =
        i64::from(state.powers.value(PowerId::EchoForm) > i32::from(state.history.plays_this_turn));
    let throwing_axe = i64::from(state.fanouts.throwing_axe_available());
    let play_count = 1_i64
        .checked_add(i64::from(replay))
        .and_then(|count| count.checked_add(duplication))
        .and_then(|count| count.checked_add(burst))
        .and_then(|count| count.checked_add(echo))
        .and_then(|count| count.checked_add(throwing_axe))
        .filter(|count| (1..=256).contains(count))
        .ok_or(EngineRefusal::CounterOverflow("drum_of_battle play count"))?;
    let play_count: usize = play_count
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("drum_of_battle play count"))?;
    let total = energy
        .checked_mul(
            play_count
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("drum_of_battle play count"))?,
        )
        .ok_or(EngineRefusal::CounterOverflow("energy"))?;
    state
        .energy
        .checked_add(total)
        .ok_or(EngineRefusal::CounterOverflow("energy"))?;
    Ok((play_count, energy))
}

/// The stable reshuffle: sort the live discard, Fisher-Yates it, make it the
/// draw pile.
pub fn reshuffle(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let discard = state.piles.get(PileId::Discard).as_slice().to_vec();
    shuffle_into_draw(state, catalog, &discard, events)
}

/// Foregone's `ShuffleIfNecessary` can itself park on Stratagem's
/// `AfterShuffle` selector. The Foregone owner has already published its
/// cursor below this Draw child, so resuming the child must return to that
/// owner rather than replaying the shuffle or any earlier BeforeHandDraw body.
pub(crate) fn reshuffle_for_foregone(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let cursor = DrawRecord {
        // This frame owns only the AfterShuffle suffix, but Draw records use
        // a positive requested count as their inhabited continuation tag.
        requested: 1,
        completed: 0,
        drawn: Vec::new(),
        from_hand_draw: false,
        stage: DrawStage::AfterShuffle,
        card_uid: None,
        caller: DrawCaller::ForegoneBeforeHandDraw,
        shuffle_candidates: Vec::new(),
        gamble_paired: None,
    };
    reshuffle_for_resumable_draw(state, catalog, &cursor, events)
}

/// Reshuffle owned by an inhabited Draw continuation. A blocking Stratagem
/// selection parks after the Shuffle command and before the caller can consume
/// another Draw card; the immutable Draw prefix is rewritten after UID
/// normalization so every later live lookup is unambiguous.
fn reshuffle_for_resumable_draw(
    state: &mut HotState,
    catalog: &Catalog,
    cursor: &DrawRecord,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let amount = usize::try_from(state.powers.value(PowerId::Stratagem)).ok();
    let will_park = !state.history.over
        && amount
            .is_some_and(|amount| amount > 0 && state.piles.get(PileId::Discard).len() > amount);
    if will_park {
        checked_stratagem_selection_count(
            state.piles.get(PileId::Discard).len(),
            amount.ok_or(EngineRefusal::ContinuationNotModeled)?,
        )?;
        super::cards::normalize_card_identities(state)?;
        state.exact_piles = true;
    }
    let discard = state.piles.get(PileId::Discard).as_slice().to_vec();
    shuffle_into_draw_raw(state, catalog, &discard, discard.len(), events)?;
    if !will_park {
        stratagem_after_shuffle(state, catalog, events)?;
        return Ok(false);
    }
    if blocking_stratagem_count(state)?.is_none() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut record = cursor.clone();
    record.drawn = live_draw_prefix(state, &cursor.drawn)?;
    record.shuffle_candidates = stratagem_exact_candidates(state, catalog)?;
    let frame_record = state
        .frames
        .push_draw(&record)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    state.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
        frame_uid: crate::hot::STRATAGEM_SELECTION_PENDING_UID,
        frame_record,
    }));
    Ok(true)
}

/// Run one Distilled Chaos gather-slot reshuffle. A blocking Stratagem owns
/// an ordinary `DrawFrame` above the partial frozen batch, but no card is
/// drawn by this cursor: once the AfterShuffle suffix returns, the potion
/// gather loop re-reads and removes the new Draw top itself.
pub(crate) fn reshuffle_for_distilled_gather(
    state: &mut HotState,
    catalog: &Catalog,
    gathered: usize,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let _ = gathered;
    let cursor = DrawRecord {
        // The partial prefix belongs exclusively to FrozenAutoBatch. This
        // Draw cursor authenticates only the awaited AfterShuffle suffix and
        // must not masquerade as completed Draw iterations on resume.
        requested: 1,
        completed: 0,
        drawn: Vec::new(),
        from_hand_draw: false,
        stage: DrawStage::AfterShuffle,
        card_uid: None,
        caller: DrawCaller::DistilledChaosGather,
        shuffle_candidates: Vec::new(),
        gamble_paired: None,
    };
    reshuffle_for_resumable_draw(state, catalog, &cursor, events)
}

/// Reboot's frozen Hand-to-Draw-bottom prefix followed by one full Shuffle.
///
/// Unlike an ordinary empty-Draw reshuffle, the native command shuffles the
/// complete live `Discard + Draw` domain. The clone rehearsal makes every
/// frozen-uid, payload, Perfect Fit, RNG, and AfterShuffle refusal atomic
/// before the first serial Hand move becomes visible. Duplicate
/// `(id, upgrade)` keys are exact exactly as they are for Bottled Potential
/// (#3197): both callers run the same `CardPileCmd.Shuffle`.
pub(crate) fn reboot_shuffle(
    state: &mut HotState,
    catalog: &Catalog,
    frozen_hand: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if full_shuffle_after_frozen_hand(state, catalog, frozen_hand, false, events)? {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

/// Bottled Potential owns the same serial move/full-shuffle command as
/// Reboot. Every physical Draw reference is preserved by the fresh native
/// `ToHashSet`: the bundled .NET 9 implementation appends into `_entries` and
/// enumerates that never-removed table in insertion order. Exact pile UIDs
/// therefore make CompareTo-equal siblings fully representable.
pub(crate) fn bottled_potential_shuffle(
    state: &mut HotState,
    catalog: &Catalog,
    frozen_hand: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    full_shuffle_after_frozen_hand(state, catalog, frozen_hand, true, events)
}

fn full_shuffle_after_frozen_hand(
    state: &mut HotState,
    catalog: &Catalog,
    frozen_hand: &[HotCard],
    bottled: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        frozen_hand: &[HotCard],
        bottled: bool,
        events: &mut Vec<Event>,
    ) -> Result<bool, EngineRefusal> {
        if state.history.over {
            return Ok(false);
        }
        for frozen in frozen_hand {
            let (pile, index) = super::play::unique_live_card_location(state, frozen.uid)?.ok_or(
                EngineRefusal::FrozenCardVanished {
                    uid: frozen.uid,
                    pile: PileId::Hand,
                },
            )?;
            let live = state.piles.get(pile).as_slice()[index];
            if pile != PileId::Hand || live != *frozen {
                return Err(EngineRefusal::FrozenCardVanished {
                    uid: frozen.uid,
                    pile: PileId::Hand,
                });
            }
            repair_card_play_after_physical_move(state, live, PileId::Hand, PileId::Draw)?;
            let moved = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
            state.piles.get_mut(PileId::Draw).make_mut().push(moved);
            events.push(Event::CardResolved {
                uid: moved.uid,
                pile: PileId::Draw,
            });
        }

        // Native `CardPileCmd/<Shuffle>d__22::MoveNext` (RVA `0x3e4b74`)
        // snapshots ordered Discard (IL_005c `ToList`), then appends a freshly
        // enumerated reference HashSet of Draw (IL_0098 `ToHashSet`, IL_00a9
        // `AddRange`). Both full-shuffle callers (Reboot and Bottled
        // Potential) are exact over equal-key siblings because this is a
        // fresh, add-only reference HashSet: bundled CoreCLR 9.0.725.31616
        // (dotnet/runtime 3c298d9f...) appends AddIfNotPresent at
        // `_entries[_count++]`, and its Enumerator scans `_entries` upward.
        // CardModel has no Equals/GetHashCode override, so its enumeration is
        // precisely the current live Draw insertion order (R52D).
        let draw = state.piles.get(PileId::Draw).as_slice();
        let discard_len = state.piles.get(PileId::Discard).len();
        let combined: Vec<HotCard> = state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .chain(draw)
            .copied()
            .collect();
        // StableShuffle (IL_00c4) sorts by `CardModel.CompareTo`, which keys
        // only `(id, upgrade)`, before its Fisher-Yates. The introsort's
        // placement of equal keys is a function of this physical input order,
        // which `combined` carries by uid, so a payload-divergent tie is as
        // exact here as in every other Shuffle caller.
        if PileId::ALL.into_iter().any(|pile| {
            state.piles.get(pile).as_slice().iter().any(|card| {
                catalog.spec(card.atom).is_some_and(|spec| {
                    spec.identity
                        .enchantment
                        .is_some_and(|value| matches!(value.id, EnchantmentId::PerfectFit))
                })
            })
        }) {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        shuffle_into_draw_raw(state, catalog, &combined, discard_len, events)?;
        if bottled && park_stratagem_on_bottled(state, catalog, events)? {
            return Ok(true);
        }
        stratagem_after_shuffle(state, catalog, events)?;
        Ok(false)
    }

    let mut probe = state.clone();
    let _ = apply(&mut probe, catalog, frozen_hand, bottled, &mut Vec::new())?;
    apply(state, catalog, frozen_hand, bottled, events)
}

fn shuffle_into_draw(
    state: &mut HotState,
    catalog: &Catalog,
    cards: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    shuffle_into_draw_raw(state, catalog, cards, cards.len(), events)?;
    stratagem_after_shuffle(state, catalog, events)
}

/// v0.111.0 PerfectFit::ModifyShuffleOrder (RVA 0xd6259, DLL 9cb4f1ad)
/// returns on initial shuffle; otherwise removes its own physical CardModel
/// and inserts it at index zero, if present. Amount is not read. The native
/// listener snapshot uses live pile order, before sorting/Fisher-Yates.
pub(crate) fn perfect_fit_identity_is_exact(spec: &crate::catalog::CardSpec) -> bool {
    spec.identity.enchantment.is_some_and(|enchantment| {
        matches!(enchantment.id, crate::ids::EnchantmentId::PerfectFit) && enchantment.amount >= 0
    })
}

/// Whether a Perfect Fit shuffle could observe pile order that an inexact
/// (`exact_piles == false`) state does not carry (#3193).
///
/// v0.111.0 (DLL `9cb4f1ad`) authority:
/// - `CardPileCmd/<Shuffle>d__22::MoveNext` (`0x3e4b74`) builds the domain as
///   Discard's cards followed by Draw's (IL_003e..IL_00a9), shuffles it with
///   the run `Shuffle` stream (IL_00bf..IL_00c4), then calls `Hook::ModifyShuffleOrder`
///   with `isInitialShuffle: false` (IL_00e2) before any card moves.
/// - `Hook::ModifyShuffleOrder` (`0x105ec4`, IL_000d..IL_0030) calls every
///   combat hook listener in `IterateCombatHookListeners` order.
/// - `CombatState/<IterateHookListeners>d__69::MoveNext` (`0x3f9720`) snapshots
///   the listener list before yielding (IL_0282): for each player it walks
///   `PlayerCombatState.AllPiles` and, within each pile, `CardPile.Cards` by
///   ascending index, adding the card, its Affliction and its Enchantment
///   (IL_0163..IL_01c7). So Perfect Fit listeners fire in live physical pile
///   order.
/// - `PerfectFit::ModifyShuffleOrder` (`0xd6259`) returns on the initial
///   shuffle (IL_0001..IL_0004), returns when its own card is not in the
///   shuffled list (IL_0005..IL_0013), and otherwise `Remove`s its card and
///   `Insert`s it at index 0 (IL_0014..IL_0029). Amount is never read.
///
/// Consequence: with one Perfect Fit identity in a shuffle domain, every
/// listener moves a payload-identical card to the top, so the resulting Draw
/// payload order is independent of the order the listeners fire in; the sort
/// before the Fisher-Yates is payload-exact because distinguishable CompareTo
/// ties already require exact piles (`live_cards_need_exact_piles`). Two or
/// more distinct Perfect Fit identities make the top card depend on the
/// Discard order an inexact state canonicalizes, so that shape stays refused
/// by name.
///
/// Admission judges the live physical cards: a domain is a subset of them, so
/// one live identity bounds every domain until a card is created or rewritten.
/// A catalog-only identity (an upgrade target, a generated copy) is not a
/// second physical card; the moment one makes a concrete inexact domain hold
/// two identities, [`shuffle_into_draw_raw`] refuses by the same name before
/// any state moves. A uid-less (legacy) Perfect Fit card cannot move itself by
/// identity and is refused too.
pub(crate) fn perfect_fit_shuffle_needs_exact_piles(state: &HotState, catalog: &Catalog) -> bool {
    if state.exact_piles {
        return false;
    }
    let live: Vec<HotCard> = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .copied()
        .collect();
    perfect_fit_domain_order_is_unrepresented(state, catalog, &live)
        || live.iter().any(|card| {
            card.uid == crate::hot::LEGACY_CARD_UID
                && catalog
                    .spec(card.atom)
                    .is_some_and(perfect_fit_identity_is_exact)
        })
}

/// The runtime twin of [`perfect_fit_shuffle_needs_exact_piles`] over one
/// concrete shuffle domain: an inexact state whose domain holds two distinct
/// Perfect Fit identities cannot name the listener order.
fn perfect_fit_domain_order_is_unrepresented(
    state: &HotState,
    catalog: &Catalog,
    cards: &[HotCard],
) -> bool {
    if state.exact_piles {
        return false;
    }
    let mut first: Option<crate::catalog::CardIdentity> = None;
    for card in cards {
        let Some(spec) = catalog.spec(card.atom) else {
            continue;
        };
        if !perfect_fit_identity_is_exact(spec) {
            continue;
        }
        match first {
            None => first = Some(spec.identity),
            Some(identity) if identity != spec.identity => return true,
            Some(_) => {}
        }
    }
    false
}

fn shuffle_into_draw_raw(
    state: &mut HotState,
    catalog: &Catalog,
    cards: &[HotCard],
    discard_prefix_len: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if discard_prefix_len > cards.len() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if perfect_fit_domain_order_is_unrepresented(state, catalog, cards) {
        return Err(EngineRefusal::MalformedArgs(
            "Perfect Fit physical shuffle order",
        ));
    }
    let mut repaired_frames = state.frames.clone();
    for (index, card) in cards.iter().copied().enumerate() {
        let source = if index < discard_prefix_len {
            PileId::Discard
        } else {
            PileId::Draw
        };
        if card.uid != crate::hot::LEGACY_CARD_UID {
            repaired_frames
                .repair_card_play_after_move(card.uid, source, PileId::Draw)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        }
    }
    state.frames = repaired_frames;
    let mut pile = crate::dotnet_sort::dotnet_list_sort_by_key(cards, |card| {
        let spec = catalog.spec(card.atom).expect("hot cards carry live atoms");
        // The interned `(u16, u8)` sort key. `CardId`'s discriminants follow
        // ascending name order, so this tuple orders exactly as Python's
        // `card[:2]` string/int pair does.
        (spec.identity.id as u16, spec.identity.upgrade)
    });

    let live = state.rng.get(RngStream::Rng);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    rng.shuffle(&mut pile)
        .map_err(|_| EngineRefusal::CounterOverflow("shuffle bound"))?;
    // `cards` retains the pre-shuffle listener order and physical UIDs. Each
    // listener moves only itself; duplicate names must never select a sibling.
    // Full-pile commands reject Perfect Fit before reaching this shared body.
    for listener in cards {
        if catalog
            .spec(listener.atom)
            .is_some_and(perfect_fit_identity_is_exact)
        {
            if listener.uid == crate::hot::LEGACY_CARD_UID {
                return Err(EngineRefusal::MalformedArgs(
                    "Perfect Fit physical shuffle order",
                ));
            }
            let index = pile
                .iter()
                .position(|card| card.uid == listener.uid)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let card = pile.remove(index);
            pile.insert(0, card);
        }
    }
    state.rng.set(
        RngStream::Rng,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );

    let moved = u16::try_from(pile.len()).unwrap_or(u16::MAX);
    state.piles.set(PileId::Discard, Default::default());
    state
        .piles
        .set(PileId::Draw, crate::hot::HotPile::from_cards(pile));
    events.push(Event::Reshuffled { cards: moved });
    Ok(())
}

fn blocking_stratagem_count(state: &HotState) -> Result<Option<usize>, EngineRefusal> {
    if state.history.over {
        return Ok(None);
    }
    let amount = state.powers.value(PowerId::Stratagem);
    if amount <= 0 || state.piles.get(PileId::Draw).is_empty() {
        return Ok(None);
    }
    let amount: usize = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("stratagem amount"))?;
    Ok((state.piles.get(PileId::Draw).len() > amount).then_some(amount))
}

pub(crate) fn stratagem_exact_candidates(
    state: &HotState,
    catalog: &Catalog,
) -> Result<Vec<HotCard>, EngineRefusal> {
    let mut cards = state.piles.get(PileId::Draw).as_slice().to_vec();
    if cards
        .iter()
        .any(|card| card.uid == crate::hot::LEGACY_CARD_UID)
        || cards
            .iter()
            .enumerate()
            .any(|(index, card)| cards[index + 1..].iter().any(|later| later.uid == card.uid))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if let Some(card) = cards.iter().find(|card| catalog.spec(card.atom).is_none()) {
        return Err(EngineRefusal::UnknownAtom(card.atom));
    }
    // Vakuu's FromCombatPile Draw view is stable native-rarity then ModelId,
    // not the generic Python tuple-payload ordering used to enumerate the
    // resulting selection actions. Stable sort preserves live physical order
    // for payload-equal siblings.
    cards.sort_by_key(|card| {
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
    Ok(cards)
}

fn stratagem_frozen_candidates(
    state: &HotState,
    catalog: &Catalog,
) -> Result<Vec<HotCard>, EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let frozen = if let Some(finish) = pending.stratagem_potion_record(&state.frames) {
        finish.candidates().collect::<Vec<_>>()
    } else if let Some(draw) = pending.stratagem_draw_record(&state.frames) {
        draw.shuffle_candidates().collect::<Vec<_>>()
    } else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    let live = stratagem_exact_candidates(state, catalog)?;
    if frozen.is_empty() || frozen != live {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(frozen)
}

pub(crate) fn stratagem_selection_action_count(
    state: &HotState,
    catalog: &Catalog,
) -> Result<u32, EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if pending.stratagem_potion_record(&state.frames).is_none()
        && pending.stratagem_draw_record(&state.frames).is_none()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let amount = blocking_stratagem_count(state)?.ok_or(EngineRefusal::ContinuationNotModeled)?;
    let candidates = stratagem_frozen_candidates(state, catalog)?;
    checked_stratagem_selection_count(candidates.len(), amount)
}

/// Authenticate the brief internal instant after a replay-owned Draw parks
/// on Stratagem but before the public transaction installs ActionReplay.
/// The ordinary pending accessor deliberately requires that root, so the
/// frozen AutoPlay driver uses this equivalent rootless check exactly once.
pub(crate) fn rootless_stratagem_draw_is_exact(
    state: &HotState,
    catalog: &Catalog,
    record: crate::hot::WordRecordIndex,
) -> Result<(), EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if pending.frame_uid != crate::hot::STRATAGEM_SELECTION_PENDING_UID
        || pending.frame_record != record
        || state.frames.top() != Some(crate::frame::Frame::Draw { record })
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let draw = state
        .frames
        .draw(record)
        .filter(|draw| draw.stage == DrawStage::AfterShuffle)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let frozen = draw.shuffle_candidates().collect::<Vec<_>>();
    let live = stratagem_exact_candidates(state, catalog)?;
    if frozen.is_empty() || frozen != live {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let amount = blocking_stratagem_count(state)?.ok_or(EngineRefusal::ContinuationNotModeled)?;
    checked_stratagem_selection_count(frozen.len(), amount)?;
    Ok(())
}

/// Native v111 StratagemPower.AfterShuffle0x34688c selects from Draw (0078),
/// then inserts each returned card in enumeration order (00da–00fe).
/// FromCombatPile0x3e5e84 returns AsCombatCards0x10d350 without sorting.
/// Thus k physical cards out of n have P(n,k) distinct ordered answers.
/// Refuse beyond the u32 action surface before publishing a continuation;
/// do not materialize factorially many answers just to count or unrank one.
pub(crate) fn checked_stratagem_selection_count(n: usize, k: usize) -> Result<u32, EngineRefusal> {
    if k > n {
        return Ok(0);
    }
    let mut count = 1_u32;
    for i in 0..k {
        let factor = u32::try_from(n - i)
            .map_err(|_| EngineRefusal::CounterOverflow("stratagem options"))?;
        count = count
            .checked_mul(factor)
            .ok_or(EngineRefusal::CounterOverflow("stratagem options"))?;
    }
    Ok(count)
}

/// Subset-major grouping retained for the cross-engine option ordinal.
fn checked_stratagem_subset_count(n: usize, k: usize) -> Result<u32, EngineRefusal> {
    if k > n {
        return Ok(0);
    }
    let k = k.min(n - k);
    let mut value = 1_u128;
    for i in 1..=k {
        value = value
            .checked_mul((n - k + i) as u128)
            .ok_or(EngineRefusal::CounterOverflow("stratagem options"))?
            / i as u128;
        if value > u128::from(u32::MAX) {
            return Err(EngineRefusal::CounterOverflow("stratagem options"));
        }
    }
    u32::try_from(value).map_err(|_| EngineRefusal::CounterOverflow("stratagem options"))
}

/// Preflight the selector that Bottled Potential's explicit full shuffle can
/// publish. Hand joins Draw before Discard is folded in, so this is the exact
/// eventual option population at this command boundary. Keeping this check in
/// the public potion validator prevents an oversized modal from consuming its
/// slot or shuffling before it is found to exceed the u32 action surface.
pub(crate) fn bottled_stratagem_selection_cardinality_is_exact(
    state: &HotState,
) -> Result<(), EngineRefusal> {
    let amount = usize::try_from(state.powers.value(PowerId::Stratagem))
        .map_err(|_| EngineRefusal::CounterOverflow("stratagem amount"))?;
    let population = [PileId::Hand, PileId::Draw, PileId::Discard]
        .into_iter()
        .map(|pile| state.piles.get(pile).len())
        .sum::<usize>();
    if amount > 0 && population > amount {
        checked_stratagem_selection_count(population, amount)?;
    }
    Ok(())
}

pub(crate) fn stratagem_selection_at(
    state: &HotState,
    catalog: &Catalog,
    ordinal: u32,
) -> Result<Vec<HotCard>, EngineRefusal> {
    let amount = blocking_stratagem_count(state)?.ok_or(EngineRefusal::ContinuationNotModeled)?;
    let mut candidates = stratagem_frozen_candidates(state, catalog)?;
    // `_pick_subsets(exact=True)` builds a Counter keyed by the uid-free
    // PhysicalCardPick tuple and sorts those keys. This action ordinal is
    // therefore payload order even though the authenticated modal snapshot
    // above remains native rarity/ModelId order.
    super::selection::sort_physical_card_pick_payloads_in_place(state, catalog, &mut candidates)?;
    let total = checked_stratagem_selection_count(candidates.len(), amount)?;
    if ordinal >= total {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let permutations = checked_stratagem_selection_count(amount, amount)?;
    let mut permutation_ordinal = ordinal % permutations;
    let mut ordinal = ordinal / permutations;
    let mut needed = amount;
    let mut selected = Vec::with_capacity(amount);
    for (index, card) in candidates.iter().copied().enumerate() {
        if needed == 0 {
            break;
        }
        let remaining = candidates.len() - index;
        if needed == remaining {
            selected.extend_from_slice(&candidates[index..]);
            break;
        }
        // Numeric mask order visits the zero-MSB block first. Candidate 0 is
        // the MSB, so singleton ordinal 0 remains the last payload key just
        // like Python's Counter subset expansion.
        let zero_prefix = checked_stratagem_subset_count(remaining - 1, needed)?;
        if ordinal < zero_prefix {
            continue;
        }
        ordinal -= zero_prefix;
        selected.push(card);
        needed -= 1;
    }
    if selected.len() != amount {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // Within each subset, match Python's itertools.permutations index order.
    // Reordering exact UIDs matters even for payload-equal physical cards.
    let mut ordered = Vec::with_capacity(amount);
    let mut suffix_permutations = permutations;
    while !selected.is_empty() {
        suffix_permutations /= selected.len() as u32;
        let index = (permutation_ordinal / suffix_permutations) as usize;
        permutation_ordinal %= suffix_permutations;
        ordered.push(selected.remove(index));
    }
    Ok(ordered)
}

pub(crate) fn resume_stratagem_selection(
    state: &mut HotState,
    catalog: &Catalog,
    ordinal: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let explicit_bottled = pending.stratagem_potion_record(&state.frames).is_some();
    let draw_owner = pending.stratagem_draw_record(&state.frames).is_some();
    let foregone_owner = draw_owner
        && state.frames.top().and_then(|frame| match frame {
            crate::frame::Frame::Draw { record } => {
                state.frames.draw(record).map(|draw| draw.caller)
            }
            _ => None,
        }) == Some(DrawCaller::ForegoneBeforeHandDraw);
    if !explicit_bottled && !draw_owner {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let selected = stratagem_selection_at(state, catalog, ordinal)?;
    state.pending = None;
    for frozen in selected {
        let index = state
            .piles
            .get(PileId::Draw)
            .as_slice()
            .iter()
            .position(|card| *card == frozen)
            .ok_or(EngineRefusal::FrozenCardVanished {
                uid: frozen.uid,
                pile: PileId::Draw,
            })?;
        let destination = if state.piles.get(PileId::Hand).len() < MAX_CARDS_IN_HAND {
            PileId::Hand
        } else {
            PileId::Discard
        };
        repair_card_play_after_physical_move(state, frozen, PileId::Draw, destination)?;
        let card = state.piles.get_mut(PileId::Draw).make_mut().remove(index);
        state.piles.get_mut(destination).make_mut().push(card);
        events.push(Event::CardResolved {
            uid: card.uid,
            pile: destination,
        });
    }
    super::relics::after_shuffle(catalog, state, events)?;
    if foregone_owner {
        state
            .frames
            .pop_top_draw()
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        return super::turn::resume_foregone_after_shuffle(state, catalog, events);
    }
    if explicit_bottled {
        super::potions::resume_bottled_after_shuffle(state, catalog, events)
    } else {
        advance_top_potion_draw(state, catalog, events)
    }
}

fn park_stratagem_on_bottled(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if blocking_stratagem_count(state)?.is_none() {
        return Ok(false);
    }
    checked_stratagem_selection_count(
        state.piles.get(PileId::Draw).len(),
        usize::try_from(state.powers.value(PowerId::Stratagem))
            .map_err(|_| EngineRefusal::CounterOverflow("stratagem amount"))?,
    )?;
    super::cards::normalize_card_identities(state)?;
    state.exact_piles = true;
    let finish = match state.frames.top() {
        Some(crate::frame::Frame::PotionFinish { record }) => state
            .frames
            .potion_finish(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?
            .to_owned(),
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    if finish.name != crate::ids::PotionId::BottledPotential
        || finish.stage != crate::frame::PotionFinishStage::Effect
        || finish.body_stage != crate::hot::PotionBodyStage::Synchronous
        || finish.current_uid.is_some()
        || finish.aux != 0
        || !finish.candidates.is_empty()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut parked = finish;
    parked.body_stage = crate::hot::PotionBodyStage::AfterShuffle;
    parked.candidates = stratagem_exact_candidates(state, catalog)?;
    state
        .frames
        .replace_top_potion_finish(&parked)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let crate::frame::Frame::PotionFinish { record } = state
        .frames
        .top()
        .ok_or(EngineRefusal::ContinuationNotModeled)?
    else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    state.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
        frame_uid: crate::hot::STRATAGEM_SELECTION_PENDING_UID,
        frame_record: record,
    }));
    let _ = events;
    Ok(true)
}

/// The synchronous, no-choice part of StratagemPower.AfterShuffle.
///
/// Admission proves the entry population fits Amount. Generated cards can
/// enlarge a later reshuffle, so this reader repeats the live bound and fails
/// closed if native would open a selection frame. A no-choice pile moves in
/// order, redirecting every card beyond the ten-card Hand cap to Discard.
fn stratagem_after_shuffle(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.history.over {
        let amount = state.powers.value(PowerId::Stratagem);
        if amount > 0 && !state.piles.get(PileId::Draw).is_empty() {
            let amount: usize = amount
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("stratagem amount"))?;
            if state.piles.get(PileId::Draw).len() > amount {
                return Err(EngineRefusal::MalformedArgs("stratagem selection"));
            }
            super::cards::normalize_card_identities(state)?;
            state.exact_piles = true;
            for frozen in stratagem_exact_candidates(state, catalog)? {
                let index = state
                    .piles
                    .get(PileId::Draw)
                    .as_slice()
                    .iter()
                    .position(|card| *card == frozen)
                    .ok_or(EngineRefusal::FrozenCardVanished {
                        uid: frozen.uid,
                        pile: PileId::Draw,
                    })?;
                let destination = if state.piles.get(PileId::Hand).len() < 10 {
                    PileId::Hand
                } else {
                    PileId::Discard
                };
                repair_card_play_after_physical_move(state, frozen, PileId::Draw, destination)?;
                let card = state.piles.get_mut(PileId::Draw).make_mut().remove(index);
                state.piles.get_mut(destination).make_mut().push(card);
                events.push(Event::CardResolved {
                    uid: card.uid,
                    pile: destination,
                });
            }
        }
    }
    super::relics::after_shuffle(catalog, state, events)
}

/// Shared exact no-choice arm for native FromCombatPile(Draw, Amount).
///
/// Foregone Conclusion enumerates Draw in pile order and adds each selected
/// card to Hand/Bottom. Stratagem uses its separate native rarity/ModelId
/// option view above. CardPileCmd.Add serially redirects cards beyond the ten-
/// card Hand cap to Discard. A larger live population would open a player-
/// selection frame, so callers name their refusal site here.
#[cold]
#[inline(never)]
pub(crate) fn auto_take_draw_to_hand(
    state: &mut HotState,
    amount: usize,
    selection_site: &'static str,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.piles.get(PileId::Draw).len() > amount {
        return Err(EngineRefusal::MalformedArgs(selection_site));
    }
    let hand_len = state.piles.get(PileId::Hand).len();
    let draw_len = state.piles.get(PileId::Draw).len();
    for index in 0..draw_len {
        let card = state.piles.get(PileId::Draw).as_slice()[index];
        let destination = if hand_len + index < MAX_CARDS_IN_HAND {
            PileId::Hand
        } else {
            PileId::Discard
        };
        repair_card_play_after_physical_move(state, card, PileId::Draw, destination)?;
    }
    let moved: Vec<HotCard> = std::mem::take(state.piles.get_mut(PileId::Draw).make_mut());
    for card in moved {
        let destination = if state.piles.get(PileId::Hand).len() < 10 {
            PileId::Hand
        } else {
            PileId::Discard
        };
        state.piles.get_mut(destination).make_mut().push(card);
        events.push(Event::CardResolved {
            uid: card.uid,
            pile: destination,
        });
    }
    Ok(())
}

/// Move the whole live hand to the bottom of the discard pile, in hand order.
///
/// `FlushPlayerHand` (`_finish_player_turn_after_wrappers`, frozen Python, deleted #2827): every
/// entry appends at `CardPilePosition.Bottom` = list end. `Hook.ShouldFlush`'s
/// suppressors (Well Laid Plans, Runic Pyramid, Ringing Triangle, Retain Hand)
/// are handled by the turn caller. This command applies the exact per-card
/// effective Retain predicate from the immutable row plus local/transient
/// physical-instance state.
pub fn flush_hand(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.piles.get(PileId::Hand).is_empty() {
        return Ok(());
    }
    // Whether any card in Hand survives the flush. Deciding this first costs
    // one borrow-only pass and lets the overwhelmingly common all-discard turn
    // keep the single-allocation shape this command had before per-instance
    // Retain existed (#1618): partitioning unconditionally added two Vec
    // allocations to *every* turn end, including the many where nothing can
    // retain. `get_ref` is load-bearing here: `get` materializes an Arc-backed
    // default for every unmodified card, turning this scan into one allocation
    // per Hand entry.
    let mut any_retained = false;
    for card in state.piles.get(PileId::Hand).as_slice() {
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if super::cards::effective_retain(state, spec, card.uid) {
            any_retained = true;
            break;
        }
    }

    let hand: Vec<HotCard> = state.piles.get(PileId::Hand).as_slice().to_vec();
    if !any_retained {
        state.piles.set(PileId::Hand, Default::default());
        let discard = state.piles.get_mut(PileId::Discard).make_mut();
        for card in &hand {
            discard.push(*card);
            events.push(Event::CardResolved {
                uid: card.uid,
                pile: PileId::Discard,
            });
        }
        super::relics::after_hand_flushed(catalog, state, &hand)?;
        return Ok(());
    }

    // At least one card retains: keep the exact stable partition. Reserving
    // both halves against the known hand size avoids the growth reallocations
    // the unsized `Vec::new()` pair incurred.
    let mut retained = Vec::with_capacity(hand.len());
    let mut discarded = Vec::with_capacity(hand.len());
    for card in hand {
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if super::cards::effective_retain(state, spec, card.uid) {
            retained.push(card);
        } else {
            discarded.push(card);
        }
    }
    state.piles.set(PileId::Hand, HotPile::from_cards(retained));
    super::relics::after_hand_flushed(catalog, state, &discarded)?;
    let discard = state.piles.get_mut(PileId::Discard).make_mut();
    for card in discarded {
        discard.push(card);
        events.push(Event::CardResolved {
            uid: card.uid,
            pile: PileId::Discard,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::engine::{Action, SelectionRef, apply_action};
    use crate::hot::HotMonster;
    use crate::ids::MonsterKind;

    #[test]
    fn retained_draw_aliases_share_current_instance_until_publication() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        let card = HotCard {
            uid: 7,
            atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        };
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            card,
            Default::default(),
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        let original = state.clone();
        let record = state
            .frames
            .push_draw(&DrawRecord {
                requested: 3,
                completed: 1,
                drawn: vec![DrawEntry::Uid(card), DrawEntry::Uid(card)],
                from_hand_draw: false,
                stage: DrawStage::EarlyHook,
                card_uid: Some(card.uid),
                caller: DrawCaller::CardPlay,
                shuffle_candidates: Vec::new(),
                gamble_paired: None,
            })
            .unwrap();
        let mut current = drawn_object_instance(&state, card.uid).unwrap();
        current.transient_retain = true;
        set_drawn_object_instance(&mut state, card.uid, current.clone()).unwrap();
        let occurrences: Vec<_> = state.frames.draw(record).unwrap().drawn().collect();
        for entry in occurrences {
            let DrawEntry::Uid(alias) = entry else {
                panic!("physical reference expected")
            };
            assert_eq!(drawn_object_instance(&state, alias.uid).unwrap(), current);
        }
        assert_ne!(state, original);
        assert_eq!(
            drawn_object_instance(&original, card.uid).unwrap(),
            Default::default()
        );
        assert!(state.card_states.as_slice().is_empty());
        publish_removed_draw_objects(&mut state).unwrap();
        assert_eq!(state.card_states.removed_draw_objects().len(), 1);
        let returned = state.frames.pop_top_draw().unwrap();
        assert_eq!(returned.drawn.len(), 2);
        // Popping the producer cannot invalidate the source body's local return list.
        assert_eq!(drawn_object_instance(&state, card.uid).unwrap(), current);
        publish_removed_draw_objects(&mut state).unwrap();
        assert!(state.card_states.removed_draw_objects().is_empty());
        assert!(drawn_object_instance(&state, card.uid).is_err());
    }

    #[test]
    fn removed_scrape_occurrence_refuses_a_sly_capture_before_any_write() {
        // #3136: DiscardAndDraw would capture a Sly removed object at IL_00f6
        // and later AutoPlay it from outside every pile; unmodeled, refused.
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        let card = HotCard {
            uid: 7,
            atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        };
        let mut sly = crate::hot::CardInstanceState::default();
        sly.set_transient_sly(true);
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            card,
            sly,
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        let before = state.clone();
        assert_eq!(
            discard_scrape_occurrences(&mut state, &catalog, &[card], &mut Vec::new()),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);

        // The same object without Sly counts once and moves nothing.
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            card,
            Default::default(),
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        let piles = state.piles.clone();
        let mut events = Vec::new();
        discard_scrape_occurrences(&mut state, &catalog, &[card], &mut events).unwrap();
        assert_eq!(state.history.discarded_cards_this_turn, 1);
        assert_eq!(state.piles, piles);
        assert!(events.is_empty());
    }

    #[test]
    fn removed_draw_object_energy_cost_reads_only_retained_local_rows() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad, CardId::Tempest]);
        let strike = HotCard {
            uid: 7,
            atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        };
        // A live uid is not a removed object.
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard { uid: 8, ..strike });
        assert_eq!(
            removed_draw_object_energy_cost(&state, &catalog, 8),
            Ok(None)
        );
        // Printed cost; no global term (FreeAttack would zero a live Attack).
        state.powers.set(PowerId::FreeAttack, SlotWire::Int, 1);
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            strike,
            Default::default(),
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        assert_eq!(
            removed_draw_object_energy_cost(&state, &catalog, 7),
            Ok(Some(1))
        );
        // Local rows apply in order, then the zero clamp.
        let mut instance = drawn_object_instance(&state, 7).unwrap();
        instance
            .local_cost_modifiers
            .push(crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Add,
                amount: -3,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            });
        set_drawn_object_instance(&mut state, 7, instance).unwrap();
        assert_eq!(
            removed_draw_object_energy_cost(&state, &catalog, 7),
            Ok(Some(0))
        );
        // An X-cost base returns before the local rows (IL_002d-0036).
        let tempest = HotCard {
            uid: 9,
            atom: catalog.atom(&identity(CardId::Tempest)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state
            .card_states
            .retain_removed_draw_object(crate::hot::FrozenAutoBatchEntry {
                card: tempest,
                state: Default::default(),
            })
            .unwrap();
        let x_cost = catalog.spec(tempest.atom).unwrap().cost;
        assert_eq!(
            removed_draw_object_energy_cost(&state, &catalog, 9),
            Ok(Some(x_cost))
        );
    }

    #[test]
    fn retained_draw_instance_write_rejects_ambiguous_live_identity_atomically() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        let card = HotCard {
            uid: 7,
            atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        };
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            card,
            Default::default(),
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        let before = state.clone();
        let replacement = crate::hot::CardInstanceState {
            transient_retain: true,
            ..Default::default()
        };
        assert!(set_drawn_object_instance(&mut state, card.uid, replacement).is_err());
        assert_eq!(state, before);
    }

    #[test]
    fn retained_draw_confused_updates_same_removed_instance() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicSneckoEye])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.rng.set(
            RngStream::EnergyCosts,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        let card = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        };
        retain_hellraiser_removed_object(
            &mut state,
            &catalog,
            card,
            Default::default(),
            crate::hot::CardPlaySource::Hellraiser,
        )
        .unwrap();
        powers_after_card_drawn(
            &mut state,
            &catalog,
            card,
            DrawSource::Command,
            None,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 1);
        let current = drawn_object_instance(&state, card.uid).unwrap();
        assert_eq!(current.local_cost_modifiers.as_slice().len(), 1);
        assert_eq!(
            current.local_cost_modifiers.as_slice()[0].expiration,
            LocalCostExpiration::ThisCombat
        );
        assert!(state.card_states.get_ref(card.uid).is_none());
    }

    #[test]
    fn retained_draw_pagestorm_reads_current_hex_mode() {
        let mut builder = CatalogBuilder::new();
        let strike = builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let defend = builder.intern(identity(CardId::DefendIronclad)).unwrap();
        let catalog = builder.build();
        for mode in [1, 2, 3] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::Pagestorm, SlotWire::Int, 1);
            state.powers.set(PowerId::HexPower, SlotWire::Int, mode);
            let card = HotCard {
                uid: 7,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE
                    | crate::hot::CARD_FLAG_DUPE
                    | crate::hot::CARD_FLAG_HEXED,
            };
            retain_hellraiser_removed_object(
                &mut state,
                &catalog,
                card,
                Default::default(),
                crate::hot::CardPlaySource::Hellraiser,
            )
            .unwrap();
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 8,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            powers_after_card_drawn(
                &mut state,
                &catalog,
                card,
                DrawSource::Command,
                Some(PowerId::Pagestorm),
                false,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.cards_drawn_combat, if mode == 2 { 1 } else { 0 });
        }
    }

    #[test]
    fn ordinary_after_card_exhausted_owner_producer_census_is_exact() {
        fn production(source: &'static str) -> &'static str {
            source.split("\nmod tests {").next().unwrap()
        }

        let play = production(include_str!("play.rs"));
        let selection = production(include_str!("selection.rs"));
        let turn = production(include_str!("turn.rs"));
        let common = production(include_str!("../steps/ironclad_common.rs"));
        let uncommon = production(include_str!("../steps/ironclad_uncommon.rs"));
        let rare = production(include_str!("../steps/ironclad_rare.rs"));
        let defect = production(include_str!("../steps/defect_rare.rs"));

        assert_eq!(play.matches("card_exhausted_return_only(").count(), 1);
        assert_eq!(play.matches("card_exhausted_with_owner(").count(), 1);
        assert_eq!(selection.matches("card_exhausted_with_owner(").count(), 1);
        assert_eq!(turn.matches("card_exhausted_return_only(").count(), 1);
        assert_eq!(
            common
                .matches("card_exhausted_cardplay_return_only(")
                .count(),
            1
        );
        assert_eq!(uncommon.matches("card_exhausted_with_owner(").count(), 2);
        assert_eq!(rare.matches("card_exhausted_with_owner(").count(), 2);
        assert_eq!(
            rare.matches("card_exhausted_cardplay_return_only(").count(),
            1
        );
        assert_eq!(defect.matches("card_exhausted_with_owner(").count(), 1);
        assert_eq!(
            play.matches("finish_card_wrapper_through_exhaust(").count(),
            3,
            "one definition plus fresh and persisted CardFinish routes"
        );
        assert_eq!(
            selection.matches("continue_selection_exhaust(").count(),
            4,
            "one definition, Purity and generic entries, and shared resume"
        );
        assert_eq!(
            selection
                .matches("AfterCardExhaustedReturnKind::PuritySelection")
                .count(),
            1
        );
        assert_eq!(
            selection
                .matches("AfterCardExhaustedReturnKind::GenericSelection")
                .count(),
            1
        );

        // Eleven owner-capable expressions represent these thirteen logical
        // return routes: CardFinish and selection each multiplex two exact
        // producer ancestries, while ExhaustRandom is one shared program.
        assert_eq!(
            [
                "rejected AutoPlay / ReturnOnly",
                "ExhaustRandom / ReturnOnly",
                "Thrash / ReturnOnly",
                "Tyranny / ReturnOnly",
                "fresh CardFinish",
                "persisted CardFinish",
                "PuritySelection",
                "GenericSelection",
                "BurningPact",
                "SecondWind",
                "FiendFire",
                "Stoke",
                "Flak",
            ]
            .len(),
            13
        );
    }

    #[test]
    fn after_card_drawn_power_order_validator_covers_the_exact_family() {
        assert_eq!(
            AFTER_CARD_DRAWN_POWER_FAMILY,
            [
                PowerId::Speedster,
                PowerId::CorrosiveWave,
                PowerId::Automation,
                PowerId::Cacophony,
                PowerId::Pagestorm,
                PowerId::Iteration,
                PowerId::ChainsOfBinding,
            ]
        );
        let mut state = HotState::at_defaults();
        for power in AFTER_CARD_DRAWN_POWER_FAMILY {
            state.powers.set(power, SlotWire::Int, 1);
        }
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&AFTER_CARD_DRAWN_POWER_FAMILY)
        );
        assert!(after_card_drawn_power_order_is_exact(&state));

        let mut reversed = AFTER_CARD_DRAWN_POWER_FAMILY;
        reversed.reverse();
        assert!(state.fanouts.set_after_card_drawn_order(&reversed));
        assert!(after_card_drawn_power_order_is_exact(&state));

        let mut malformed = reversed;
        malformed[6] = malformed[0];
        assert!(state.fanouts.set_after_card_drawn_order(&malformed));
        assert!(!after_card_drawn_power_order_is_exact(&state));
        malformed = reversed;
        malformed[6] = PowerId::Accuracy;
        assert!(state.fanouts.set_after_card_drawn_order(&malformed));
        assert!(!after_card_drawn_power_order_is_exact(&state));
        assert!(state.fanouts.set_after_card_drawn_order(&reversed[..6]));
        assert!(!after_card_drawn_power_order_is_exact(&state));
        state.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 0);
        assert!(state.fanouts.set_after_card_drawn_order(&reversed));
        assert!(!after_card_drawn_power_order_is_exact(&state));
    }

    fn identity(id: CardId) -> CardIdentity {
        identity_at(id, 0)
    }

    fn identity_at(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn card(catalog: &Catalog, id: CardId, uid: u32) -> HotCard {
        card_at(catalog, id, 0, uid)
    }

    /// Append `cards` to Hand: DiscardAndDraw now detaches live cards itself.
    fn hand_of(state: &mut HotState, cards: &[HotCard]) {
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend_from_slice(cards);
    }

    fn card_at(catalog: &Catalog, id: CardId, upgrade: u8, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog.atom(&identity_at(id, upgrade)).unwrap(),
            flags: 0,
        }
    }

    fn install_live_queen_roster(state: &mut HotState) {
        let mut amalgam = HotMonster::new(
            MonsterKind::TorchHeadAmalgam,
            super::super::monsters::TORCH_HEAD_AMALGAM_HP,
        );
        amalgam.max_hp = super::super::monsters::TORCH_HEAD_AMALGAM_HP;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = HotMonster::new(MonsterKind::Queen, super::super::monsters::QUEEN_HP);
        queen.max_hp = super::super::monsters::QUEEN_HP;
        queen.slot = 1;
        queen.uid = 1;
        queen.loop_pos = 1;
        state.monsters_mut().extend([amalgam, queen]);
    }

    fn install_live_spectral_roster(state: &mut HotState) {
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(2)));
        assert!(flail.random_ai.set_log(&[2]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(0)));
        assert!(spectral.random_ai.set_log(&[0]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 89);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        state.monsters_mut().extend([flail, spectral, magi]);
    }

    fn hellraiser_fixture(ids: &[CardId]) -> (Catalog, HotState) {
        let mut builder = CatalogBuilder::new();
        for id in ids {
            builder.intern(identity(*id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 10;
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 100)]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        (catalog, state)
    }

    #[test]
    fn chains_afflicts_first_three_successes_and_skips_an_existing_affliction() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::StrikeIronclad, CardId::Dazed] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let physical = crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_side_active = true;
        state.exact_piles = true;
        state.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::ChainsOfBinding])
        );
        assert!(state.fanouts.set_before_side_turn_end_order(&[
            crate::hot::BeforeSideTurnEndToken::ChainsOfBinding,
        ]));
        assert!(state.set_bound_afflictions_this_turn(1));
        install_live_queen_roster(&mut state);
        state.piles.set(
            PileId::Draw,
            HotPile::from_cards(vec![
                HotCard {
                    uid: 0,
                    atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
                    flags: physical | crate::hot::CARD_FLAG_BOUND,
                },
                HotCard {
                    uid: 1,
                    atom: catalog.atom(&identity(CardId::Dazed)).unwrap(),
                    flags: physical,
                },
                HotCard {
                    uid: 2,
                    atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
                    flags: physical,
                },
                HotCard {
                    uid: 3,
                    atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
                    flags: physical,
                },
                HotCard {
                    uid: 4,
                    atom: catalog.atom(&identity(CardId::StrikeIronclad)).unwrap(),
                    flags: physical,
                },
            ]),
        );

        draw_cards(
            &mut state,
            &catalog,
            5,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        let flags: Vec<u16> = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card.flags)
            .collect();
        assert_ne!(flags[0] & crate::hot::CARD_FLAG_BOUND, 0);
        assert!(
            flags[1..=2]
                .iter()
                .all(|flags| flags & crate::hot::CARD_FLAG_BOUND != 0)
        );
        assert!(
            flags[3..=4]
                .iter()
                .all(|flags| flags & crate::hot::CARD_FLAG_BOUND == 0)
        );
        assert_eq!(state.bound_afflictions_this_turn(), 3);
    }

    #[test]
    fn hellraiser_early_autoplays_the_exact_drawn_strike_and_returns_live_payload() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            7,
        ));
        let mut drawn = Vec::new();
        let mut events = Vec::new();

        draw_cards_into(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut events,
            &mut drawn,
        )
        .unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), drawn);
        assert_eq!(drawn.iter().map(|card| card.uid).collect::<Vec<_>>(), [7]);
        assert_eq!(state.monsters[0].hp, 94);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.history.manual_card_plays_finished_this_turn, 0);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
    }

    #[test]
    fn hellraiser_lethal_child_finishes_before_suppressing_the_ordinary_snapshot() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        std::sync::Arc::make_mut(&mut state.monsters)[0].hp = 6;
        state.powers.set(PowerId::Automation, SlotWire::Int, 1);
        state.fanouts.set_automation_left(4);
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            7,
        ));

        let mut drawn = Vec::new();
        draw_cards_into(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut Vec::new(),
            &mut drawn,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.fanouts.automation_left(), 4);
        assert_eq!(state.piles.get(PileId::Play).as_slice(), drawn);
        assert!(state.piles.get(PileId::Discard).is_empty());
    }

    #[test]
    fn hellraiser_nested_pommel_chain_is_serial_and_uses_one_outer_rehearsal() {
        let (catalog, mut state) =
            hellraiser_fixture(&[CardId::PommelStrike, CardId::StrikeIronclad]);
        state.piles.set(
            PileId::Draw,
            HotPile::from_cards(vec![
                card(&catalog, CardId::PommelStrike, 7),
                card(&catalog, CardId::StrikeIronclad, 8),
            ]),
        );

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [8, 7]
        );
        assert_eq!(state.monsters[0].hp, 85);
        assert_eq!(state.cards_drawn_combat, 2);
        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 2);
    }

    #[test]
    fn hellraiser_cold_rehearsal_prevents_a_late_child_prefix() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            7,
        ));
        state.history.card_plays_finished_combat = i32::MAX;

        let result = draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut Vec::new(),
        );

        assert_eq!(
            result,
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state.monsters[0].hp, 100);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 0);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [7]
        );
        assert!(state.piles.get(PileId::Discard).is_empty());
    }

    #[test]
    fn hellraiser_refuses_same_physical_uid_reentry_before_mutation() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::StrikeIronclad]);
        state.piles.get_mut(PileId::Hand).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            7,
        ));
        let before = state.clone();

        let result = super::super::play::with_test_active_play(7, || {
            hellraiser_after_card_drawn_early(&mut state, &catalog, 7, &mut Vec::new())
        });

        assert_eq!(result, Err(EngineRefusal::ContinuationNotModeled));
        assert_eq!(state, before);
    }

    #[test]
    fn hellraiser_power_is_unique_and_reapplication_does_not_change_wire() {
        let (catalog, mut state) = hellraiser_fixture(&[CardId::Hellraiser]);
        state.powers = Default::default();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Hellraiser, 7),
            card(&catalog, CardId::Hellraiser, 8),
        ]);

        super::super::play::play_card(&mut state, &catalog, 7, None, None, &mut Vec::new())
            .unwrap();
        let first = state.powers.get(PowerId::Hellraiser);
        super::super::play::play_card(&mut state, &catalog, 8, None, None, &mut Vec::new())
            .unwrap();

        assert_eq!(first, state.powers.get(PowerId::Hellraiser));
        assert!(first.is_some_and(|slot| slot.wire == SlotWire::Int && slot.value == 1));
    }

    #[test]
    fn hellraiser_late_recursive_refusal_rolls_back_the_public_parent_action() {
        let (catalog, mut state) =
            hellraiser_fixture(&[CardId::PommelStrike, CardId::StrikeIronclad]);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::PommelStrike, 7));
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            8,
        ));
        state.history.card_plays_finished_combat = i32::MAX;
        let before = state.clone();

        let result = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 7,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        );

        assert_eq!(
            result,
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn flush_preserves_intrinsic_local_and_transient_retain_in_hand_order() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
            CardId::Purity,
            CardId::Bash,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 1),
            card(&catalog, CardId::DefendIronclad, 2),
            card(&catalog, CardId::Purity, 3),
            card(&catalog, CardId::Bash, 4),
        ]);
        state.card_states.set_local_retain(1);
        state.card_states.set_transient_retain(2);
        let mut events = Vec::new();

        flush_hand(&mut state, &catalog, &mut events).unwrap();

        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [4]
        );
        assert_eq!(
            events,
            [Event::CardResolved {
                uid: 4,
                pile: PileId::Discard,
            }]
        );
    }

    #[test]
    fn a_steady_enchantment_retains_its_card_through_the_end_of_turn_flush() {
        // Python: `card_effective_keywords` (frozen, deleted #2827) adds Retain for the
        // STEADY keyword-delta enchantment.
        //
        // The hand flush itself is `_finish_player_turn_after_wrappers` (frozen Python, deleted #2827), whose retain predicate is `card_is_retained`. Native
        // `Steady::OnEnchant` RVA `0xd65b4` is `Card.AddKeyword(5)` on
        // v0.111.0 DLL 9cb4f1ad.
        let steady = CardIdentity {
            id: CardId::Sunder,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Steady,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let steady_atom = builder.intern(steady).unwrap();
        builder.intern(identity(CardId::Sunder)).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 1),
            HotCard {
                uid: 2,
                atom: steady_atom,
                flags: 0,
            },
            // The same card WITHOUT the enchantment must still flush: the
            // Retain comes from slot 2, never from the row.
            card(&catalog, CardId::Sunder, 3),
        ]);
        let mut events = Vec::new();

        flush_hand(&mut state, &catalog, &mut events).unwrap();

        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1, 3]
        );
    }

    #[test]
    fn a_royally_approved_card_is_retained_through_the_end_of_turn_flush() {
        // #3178: `RoyallyApproved::OnEnchant` RVA `0xd62c9` IL_0013-0014 is
        // `Card.AddKeyword(5)` (Retain) on v0.111.0 DLL 9cb4f1ad, so the
        // enchanted Strike stays in hand and its bare twin is discarded.
        let royal = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::RoyallyApproved,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let royal_atom = builder.intern(royal).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 1),
            HotCard {
                uid: 2,
                atom: royal_atom,
                flags: 0,
            },
        ]);
        let mut events = Vec::new();

        flush_hand(&mut state, &catalog, &mut events).unwrap();

        let uids = |pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .map(|card: &HotCard| card.uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(uids(PileId::Hand), [2]);
        assert_eq!(uids(PileId::Discard), [1]);
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn no_retain_flush_does_not_allocate_per_card() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.piles.set(
            PileId::Hand,
            HotPile::from_cards(
                (1..=10)
                    .map(|uid| card(&catalog, CardId::StrikeIronclad, uid))
                    .collect(),
            ),
        );
        state
            .piles
            .set(PileId::Discard, HotPile::from_cards(Vec::with_capacity(10)));
        let mut events = Vec::with_capacity(10);

        let (before, _) = crate::allocation::thread_snapshot();
        flush_hand(&mut state, &catalog, &mut events).unwrap();
        let (after, _) = crate::allocation::thread_snapshot();

        let allocations = after - before;
        assert!(
            allocations <= 3,
            "ordinary cards allocated {allocations} times during a no-Retain flush"
        );
    }

    /// #3047: `SpeedsterPower/<AfterCardDrawn>d__4` RVA `0x345abc` leaves on
    /// `fromHandDraw` (IL_0020-0028) and off the owner's side (IL_004a-007b);
    /// every other Draw damages each hittable enemy.
    #[test]
    fn speedster_skips_hand_draw_and_monster_side_draws() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let fixture = |side_active: bool| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            for _ in 0..2 {
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 50));
            }
            state.piles.get_mut(PileId::Draw).make_mut().push(card(
                &catalog,
                CardId::StrikeIronclad,
                7,
            ));
            state.powers.set(PowerId::Speedster, SlotWire::Int, 3);
            assert!(
                state
                    .fanouts
                    .set_after_card_drawn_order(&[PowerId::Speedster])
            );
            state.player_side_active = side_active;
            state
        };
        for (source, side_active, hp) in [
            (DrawSource::Command, true, 47),
            (DrawSource::HandDraw, true, 50),
            (DrawSource::Command, false, 50),
        ] {
            let mut state = fixture(side_active);
            draw_cards(&mut state, &catalog, 1, source, &mut Vec::new()).unwrap();
            assert_eq!(state.piles.get(PileId::Hand).len(), 1);
            assert_eq!(
                state.monsters.iter().map(|m| m.hp).collect::<Vec<_>>(),
                vec![hp, hp],
                "{source:?} side_active={side_active}"
            );
        }
    }

    #[test]
    fn ordered_draw_fanouts_share_one_card_and_exact_private_countdowns() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            7,
        ));
        for (power, amount) in [
            (PowerId::Speedster, 1),
            (PowerId::CorrosiveWave, 2),
            (PowerId::Automation, 1),
            (PowerId::Cacophony, 3),
        ] {
            state.powers.set(power, SlotWire::Int, amount);
        }
        assert!(state.fanouts.set_after_card_drawn_order(&[
            PowerId::Speedster,
            PowerId::CorrosiveWave,
            PowerId::Automation,
            PowerId::Cacophony,
        ]));
        state.fanouts.set_automation_left(1);
        state.fanouts.set_cacophony_left(1);
        let mut events = Vec::new();

        // A Command draw: SpeedsterPower exits on `fromHandDraw` (#3047).
        draw_cards(&mut state, &catalog, 1, DrawSource::Command, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 46);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 2);
        assert_eq!(state.energy, 4);
        assert_eq!(state.fanouts.automation_left(), 10);
        assert_eq!(state.fanouts.cacophony_left(), 33);
        assert_eq!(state.fanouts.cacophony_resets_completed(), 1);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
    }

    #[test]
    fn kingly_punch_draw_growth_is_level_specific_and_physical() {
        for (upgrade, increase) in [(0, 4), (1, 6)] {
            let identity = CardIdentity {
                id: CardId::KinglyPunch,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            state.card_states.add_damage_growth(7, 2).unwrap();

            draw_cards(
                &mut state,
                &catalog,
                1,
                DrawSource::Command,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.card_states.get(7).damage_growth, 2 + increase);
            assert_ne!(
                state.piles.get(PileId::Hand).as_slice()[0].flags
                    & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                0
            );
            assert!(state.exact_piles);
        }
    }

    #[test]
    fn kingly_kick_draw_appends_one_exact_combat_cost_row_after_terminal_powers() {
        for upgrade in [0, 1] {
            let kick = CardIdentity {
                id: CardId::KinglyKick,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(kick).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 17,
                atom,
                flags: 0,
            });
            state.powers.set(PowerId::Cacophony, SlotWire::Int, 3);
            state.fanouts.set_cacophony_left(1);
            assert!(
                state
                    .fanouts
                    .set_after_card_drawn_order(&[PowerId::Cacophony])
            );

            draw_cards(
                &mut state,
                &catalog,
                1,
                DrawSource::Command,
                &mut Vec::new(),
            )
            .unwrap();

            assert!(
                state.history.over,
                "the preceding power listener is terminal"
            );
            assert_eq!(
                state.card_states.get(17).local_cost_modifiers.as_slice(),
                [crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Add,
                    amount: -1,
                    expiration: crate::hot::LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                }],
                "the frozen exact-card callback survives a terminal predecessor"
            );
            assert_ne!(
                state.piles.get(PileId::Hand).as_slice()[0].flags
                    & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                0
            );
            assert!(state.exact_piles);
        }
    }

    #[test]
    fn a_full_hand_does_not_fire_kingly_kicks_draw_listener() {
        let kick = CardIdentity {
            id: CardId::KinglyKick,
            upgrade: 0,
            enchantment: None,
        };
        let strike = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let kick_atom = builder.intern(kick).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: kick_atom,
            flags: 0,
        });
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..MAX_CARDS_IN_HAND).map(|index| HotCard {
                uid: 10 + index as u32,
                atom: strike_atom,
                flags: 0,
            }));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 1);
        assert!(state.card_states.get(1).local_cost_modifiers.is_empty());
        assert!(!state.exact_piles);
    }

    #[test]
    fn recursive_draw_powers_reenter_the_per_card_choke_point() {
        for (trigger, first, local_ethereal, hexed) in [
            (PowerId::Pagestorm, CardId::Demesne, false, false),
            (PowerId::Pagestorm, CardId::StrikeIronclad, true, false),
            (PowerId::Pagestorm, CardId::StrikeIronclad, false, true),
            (PowerId::Iteration, CardId::Dazed, false, false),
            (PowerId::Iteration, CardId::Infection, false, false),
            (PowerId::Iteration, CardId::Wound, false, false),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern(identity(first)).unwrap();
            builder.intern(identity(CardId::StrikeIronclad)).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                card(&catalog, first, 1),
                card(&catalog, CardId::StrikeIronclad, 2),
            ]);
            if local_ethereal {
                state.piles.get_mut(PileId::Draw).make_mut()[0].flags |=
                    crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                let mut instance = state.card_states.get(1);
                instance.set_local_ethereal(true);
                state.card_states.set(1, instance);
            }
            if hexed {
                state.exact_piles = true;
                for card in state.piles.get_mut(PileId::Draw).make_mut() {
                    card.flags |=
                        crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_HEXED;
                }
                state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
                install_live_spectral_roster(&mut state);
                assert!(super::super::cards::hex_power_state_is_exact(&state));
            }
            state.powers.set(trigger, SlotWire::Int, 1);
            assert!(state.fanouts.set_after_card_drawn_order(&[trigger]));
            let mut events = Vec::new();

            draw_cards(&mut state, &catalog, 1, DrawSource::Command, &mut events).unwrap();

            assert_eq!(state.piles.get(PileId::Hand).len(), 2, "{trigger:?}");
            assert_eq!(state.history.non_hand_draws_this_turn, 2);
            assert_eq!(
                state.history.status_draws_this_turn,
                i16::from(trigger == PowerId::Iteration)
            );
        }
    }

    #[test]
    fn no_draw_blocks_command_draws_before_mutation_but_not_hand_draw() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(&catalog, CardId::DefendSilent, 7));
        state.powers.set(PowerId::NoDraw, SlotWire::Int, 1);
        let rng_before = state.rng.clone();
        let mut events = Vec::new();

        draw_cards(&mut state, &catalog, 1, DrawSource::Command, &mut events).unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 7);
        assert_eq!(state.history.non_hand_draws_this_turn, 0);
        assert_eq!(state.rng, rng_before);
        assert!(events.is_empty());

        draw_cards(&mut state, &catalog, 1, DrawSource::HandDraw, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 7);
        assert!(state.piles.get(PileId::Draw).is_empty());
    }

    #[test]
    fn native_sly_cards_autoplay_in_discard_order_after_the_one_paired_draw() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::Tactician, CardId::Reflex, CardId::StrikeSilent] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let tactician = card(&catalog, CardId::Tactician, 1);
        let reflex = card(&catalog, CardId::Reflex, 2);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        for uid in 10..=12 {
            state.piles.get_mut(PileId::Draw).make_mut().push(card(
                &catalog,
                CardId::StrikeSilent,
                uid,
            ));
        }
        let mut events = Vec::new();

        hand_of(&mut state, &[tactician, reflex]);
        discard_and_draw(
            &mut state,
            &catalog,
            PileId::Hand,
            &[tactician, reflex],
            1,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.energy, 4, "Tactician is the first Sly child");
        assert_eq!(state.piles.get(PileId::Hand).len(), 3);
        assert_eq!(state.history.non_hand_draws_this_turn, 3);
        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert_eq!(state.history.discarded_cards_this_turn, 2);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardPlayed { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [1, 2],
            "the live-UID walk preserves the collected Sly order"
        );
    }

    #[test]
    fn reflex_and_untouchable_admit_manual_and_survivor_sly_paths() {
        use crate::boundary::HotBoundary;
        use crate::engine::{Action, SelectionRef, apply_action};
        for id in [CardId::Reflex, CardId::Untouchable] {
            for upgrade in 0..=1 {
                for discarded in [false, true] {
                    let mut builder = CatalogBuilder::new();
                    builder.intern_reachable(identity_at(id, upgrade)).unwrap();
                    builder
                        .intern_reachable(identity(CardId::Survivor))
                        .unwrap();
                    builder
                        .intern_reachable(identity(CardId::DefendSilent))
                        .unwrap();
                    builder.intern_monster(MonsterKind::Toadpole).unwrap();
                    let catalog = builder.build();
                    let mut state = HotState::at_defaults();
                    state.hp = 70;
                    state.max_hp = 70;
                    state.energy = 3;
                    state.next_card_uid = 7;
                    state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
                    state
                        .piles
                        .get_mut(PileId::Hand)
                        .make_mut()
                        .push(card_at(&catalog, id, upgrade, 1));
                    if discarded {
                        state.piles.get_mut(PileId::Hand).make_mut().push(card(
                            &catalog,
                            CardId::Survivor,
                            2,
                        ));
                    }
                    for uid in 3..=6 {
                        state.piles.get_mut(PileId::Draw).make_mut().push(card(
                            &catalog,
                            CardId::DefendSilent,
                            uid,
                        ));
                    }
                    state
                        .monsters_mut()
                        .push(HotMonster::new(MonsterKind::Toadpole, 100));
                    let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
                    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
                    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
                    crate::engine::admit(&doc, &state, &catalog).unwrap();
                    let after = apply_action(
                        &state,
                        &catalog,
                        &Action::Play {
                            uid: if discarded { 2 } else { 1 },
                            target: None,
                            selection: SelectionRef::NONE,
                        },
                    )
                    .unwrap()
                    .state;
                    assert!(after.pending.is_none());
                    assert_eq!(
                        after.energy,
                        if discarded {
                            2
                        } else if id == CardId::Reflex {
                            0
                        } else {
                            1
                        }
                    );
                    assert_eq!(
                        after.block,
                        if discarded { 8 } else { 0 }
                            + if id == CardId::Untouchable {
                                6 + 3 * i32::from(upgrade)
                            } else {
                                0
                            }
                    );
                    assert_eq!(
                        after.piles.get(PileId::Hand).len(),
                        if id == CardId::Reflex {
                            2 + usize::from(upgrade)
                        } else {
                            0
                        }
                    );
                    assert_eq!(
                        after.history.card_plays_finished_combat,
                        if discarded { 2 } else { 1 }
                    );
                    assert_eq!(
                        after.history.discarded_cards_this_turn,
                        i16::from(discarded)
                    );
                    assert_eq!(after.rng, state.rng);
                    assert!(
                        after
                            .piles
                            .get(PileId::Discard)
                            .as_slice()
                            .iter()
                            .any(|card| card.uid == 1)
                    );
                }
            }
        }
    }

    /// #3045: `RingingPower::ShouldPlay` (RVA 0xa6c5c) vetoes a Ringing card
    /// once any owner play has STARTED this turn, and `CardCmd.AutoPlay`
    /// (0x3df9d4, IL_015c) consults it. The parent play's CardPlayStarted
    /// entry is recorded at OnPlayWrapper IL_0626, before its body discards
    /// the Sly card, so the veto holds even when the parent is the turn's
    /// first play. The vetoed Untouchable routes to Discard with no body (no
    /// Block), no play history, and no RNG draw. The controls keep the
    /// ordinary auto-play when Ringing is absent or the card is unafflicted.
    #[test]
    fn ringing_vetoes_a_sly_auto_play_once_the_parent_play_has_started() {
        use crate::engine::{Action, SelectionRef, apply_action};
        // (ringing, untouchable afflicted, earlier play this turn, vetoed)
        for (ringing, afflicted, earlier_play, vetoed) in [
            (true, true, false, true),
            (true, true, true, true),
            (false, false, false, false),
            (false, false, true, false),
            (true, false, false, false),
        ] {
            let mut builder = CatalogBuilder::new();
            builder
                .intern_reachable(identity(CardId::Untouchable))
                .unwrap();
            builder
                .intern_reachable(identity(CardId::Survivor))
                .unwrap();
            builder
                .intern_reachable(identity(CardId::DefendSilent))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.max_hp = 70;
            state.energy = 3;
            state.next_card_uid = 7;
            state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            let hand = state.piles.get_mut(PileId::Hand).make_mut();
            hand.push(card(&catalog, CardId::Untouchable, 1));
            hand.push(card(&catalog, CardId::Survivor, 2));
            for uid in 3..=6 {
                state.piles.get_mut(PileId::Draw).make_mut().push(card(
                    &catalog,
                    CardId::DefendSilent,
                    uid,
                ));
            }
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            if ringing {
                crate::engine::cards::apply_ringing_power(&mut state).unwrap();
            }
            let hand = state.piles.get_mut(PileId::Hand).make_mut();
            if !afflicted {
                hand[0].flags &= !crate::hot::CARD_FLAG_RINGING;
            }
            if earlier_play {
                // An earlier play this turn: the manually played Survivor must
                // itself be unafflicted to stay legal under Ringing.
                hand[1].flags &= !crate::hot::CARD_FLAG_RINGING;
                state.history.plays_this_turn = 1;
            }
            let plays_before = state.history.plays_this_turn;
            let after = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 2,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert!(after.pending.is_none());
            assert_eq!(after.energy, 2);
            assert_eq!(after.block, if vetoed { 8 } else { 8 + 6 });
            assert_eq!(
                after.history.card_plays_finished_combat,
                if vetoed { 1 } else { 2 }
            );
            assert_eq!(
                after.history.plays_this_turn,
                plays_before + if vetoed { 1 } else { 2 }
            );
            assert_eq!(after.history.discarded_cards_this_turn, 1);
            assert_eq!(after.rng, state.rng);
            assert!(after.piles.get(PileId::Hand).as_slice().is_empty());
            assert!(
                after
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 1)
            );
        }
    }

    #[test]
    fn static_sly_attacks_play_manually_without_triggering_the_discard_reader() {
        for (id, upgrade, cost, damage, hits) in [
            (CardId::FlickFlack, 0, 1, 7, 0),
            (CardId::FlickFlack, 1, 1, 9, 0),
            (CardId::Ricochet, 0, 2, 12, 4),
            (CardId::Ricochet, 1, 2, 15, 5),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern(identity_at(id, upgrade)).unwrap();
            let catalog = builder.build();
            let source = card_at(&catalog, id, upgrade, 7);
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.energy = 5;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.energy, 5 - cost, "{}+{upgrade}", id.as_str());
            assert_eq!(state.monsters[0].hp, 100 - damage);
            assert_eq!(state.rng.get(RngStream::Targets).counter, hits);
            assert_eq!(state.history.card_plays_finished_combat, 1);
            assert_eq!(state.history.discarded_cards_this_turn, 0);
            assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
        }
    }

    #[test]
    fn static_sly_attacks_autoplay_after_the_paired_draw_in_collected_order() {
        let mut builder = CatalogBuilder::new();
        for identity in [
            identity_at(CardId::FlickFlack, 1),
            identity_at(CardId::Ricochet, 1),
            identity(CardId::StrikeSilent),
        ] {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let flick = card_at(&catalog, CardId::FlickFlack, 1, 1);
        let ricochet = card_at(&catalog, CardId::Ricochet, 1, 2);
        let marker = card(&catalog, CardId::StrikeSilent, 3);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 0;
        state.piles.get_mut(PileId::Draw).make_mut().push(marker);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut events = Vec::new();

        hand_of(&mut state, &[flick, ricochet]);
        discard_and_draw(
            &mut state,
            &catalog,
            PileId::Hand,
            &[flick, ricochet],
            1,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.energy, 0, "Sly AutoPlay never spends card cost");
        assert_eq!(state.monsters[0].hp, 76);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 5);
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[marker]);
        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            &[flick, ricochet]
        );
        assert_eq!(state.history.discarded_cards_this_turn, 2);
        assert_eq!(state.history.card_plays_finished_combat, 2);
        let resolved_draw = events
            .iter()
            .position(|event| matches!(event, Event::CardDrawn { uid: 3 }))
            .unwrap();
        let played = events
            .iter()
            .filter_map(|event| match event {
                Event::CardPlayed { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect::<Vec<_>>();
        let first_play = events
            .iter()
            .position(|event| matches!(event, Event::CardPlayed { uid: 1, .. }))
            .unwrap();
        assert!(resolved_draw < first_play);
        assert_eq!(played, [1, 2]);
    }

    #[test]
    fn collected_sly_autoplay_preserves_each_non_null_source_until_successful_bottom_routing() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::FlickFlack, CardId::StrikeSilent] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let source = card(&catalog, CardId::FlickFlack, 7);
        let before = card(&catalog, CardId::StrikeSilent, 8);
        let after = card(&catalog, CardId::StrikeSilent, 9);

        for source_pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.energy = 0;
            state
                .piles
                .get_mut(source_pile)
                .make_mut()
                .extend([before, source, after]);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let mut events = Vec::new();

            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[source],
                &mut events,
            )
            .unwrap();

            assert_eq!(state.monsters[0].hp, 93, "{source_pile:?}");
            assert_eq!(state.energy, 0, "{source_pile:?}");
            assert!(state.piles.get(PileId::Play).is_empty());
            assert_eq!(
                state
                    .piles
                    .get(source_pile)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                if source_pile == PileId::Discard {
                    vec![8, 9, 7]
                } else {
                    vec![8, 9]
                },
                "surviving same-pile Discard routing removes and appends Bottom"
            );
            assert_eq!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .filter(|card| card.uid == source.uid)
                    .count(),
                1
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        Event::CardResolved {
                            uid: 7,
                            pile: PileId::Discard
                        }
                    ))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn terminal_collected_autoplay_leaves_only_its_active_source_in_play() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::FlickFlack, CardId::StrikeSilent] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let source = card(&catalog, CardId::FlickFlack, 7);
        let before = card(&catalog, CardId::StrikeSilent, 8);
        let after = card(&catalog, CardId::StrikeSilent, 9);

        for source_pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state
                .piles
                .get_mut(source_pile)
                .make_mut()
                .extend([before, source, after]);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));
            let mut events = Vec::new();

            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[source],
                &mut events,
            )
            .unwrap();

            assert!(state.history.over, "{source_pile:?}");
            assert_eq!(
                state
                    .piles
                    .get(source_pile)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [8, 9],
                "native body-entry moves only the active object; terminal completion skips routing"
            );
            assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Event::CardResolved { uid: 7, .. }))
            );
        }
    }

    #[test]
    fn frozen_hand_discard_uses_the_same_static_sly_live_uid_pipeline() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity_at(CardId::Ricochet, 0)).unwrap();
        builder.intern(identity_at(CardId::FlickFlack, 0)).unwrap();
        let catalog = builder.build();
        let ricochet = card_at(&catalog, CardId::Ricochet, 0, 11);
        let flick = card_at(&catalog, CardId::FlickFlack, 0, 12);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([ricochet, flick]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));

        discard_frozen_hand(&mut state, &catalog, &[ricochet, flick], &mut Vec::new()).unwrap();

        assert_eq!(state.monsters[0].hp, 81);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 4);
        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            &[ricochet, flick]
        );
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.history.discarded_cards_this_turn, 2);
        assert_eq!(state.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn late_static_sly_completion_failure_rolls_back_discard_draw_rng_and_damage() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity_at(CardId::Ricochet, 1)).unwrap();
        let catalog = builder.build();
        let ricochet = card_at(&catalog, CardId::Ricochet, 1, 21);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.history.card_plays_finished_combat = i32::MAX;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        hand_of(&mut state, &[ricochet]);
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let events_before = events.clone();

        assert_eq!(
            discard_and_draw(
                &mut state,
                &catalog,
                PileId::Hand,
                &[ricochet],
                0,
                &mut events
            ),
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn late_sly_failure_rolls_back_the_complete_frozen_hand_command() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity_at(CardId::Ricochet, 1)).unwrap();
        let catalog = builder.build();
        let ricochet = card_at(&catalog, CardId::Ricochet, 1, 21);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.history.card_plays_finished_combat = i32::MAX;
        state.piles.get_mut(PileId::Hand).make_mut().push(ricochet);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let events_before = events.clone();

        assert_eq!(
            discard_frozen_hand(&mut state, &catalog, &[ricochet], &mut events),
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn static_sly_autoplay_reaches_strangle_and_honors_its_terminal_suffix() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity_at(CardId::FlickFlack, 0)).unwrap();
        builder.intern(identity_at(CardId::Ricochet, 0)).unwrap();
        let catalog = builder.build();
        let flick = card_at(&catalog, CardId::FlickFlack, 0, 31);
        let ricochet = card_at(&catalog, CardId::Ricochet, 0, 32);

        for (amount, expected_hp, expected_rolls, expected_plays) in [(2, 77, 4, 2), (93, 0, 0, 1)]
        {
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Strangle, SlotWire::Int, amount);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(crate::hot::MiseryToken::Strangle);

            hand_of(&mut state, &[flick, ricochet]);
            discard_and_draw(
                &mut state,
                &catalog,
                PileId::Hand,
                &[flick, ricochet],
                0,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.monsters[0].hp, expected_hp, "Strangle {amount}");
            assert_eq!(
                state.rng.get(RngStream::Targets).counter,
                expected_rolls,
                "a lethal Flick Flack suffix must suppress Ricochet"
            );
            assert_eq!(
                state.history.card_plays_finished_combat, expected_plays,
                "each completed Sly child reaches Strangle exactly once"
            );
        }
    }

    #[test]
    fn permanent_instance_sly_uses_the_same_discard_autoplay_pipeline() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let defend = card(&catalog, CardId::DefendSilent, 7);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.card_states.set_local_sly(7);
        let mut events = Vec::new();

        hand_of(&mut state, &[defend]);
        discard_and_draw(
            &mut state,
            &catalog,
            PileId::Hand,
            &[defend],
            0,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.block, 5);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 7);
        assert!(state.card_states.get(7).local_sly);
    }

    #[test]
    fn permanent_sly_stack_excludes_its_active_play_source_from_discard() {
        let discard_count_skills: Vec<_> = crate::content_tables::CARD_ROWS
            .iter()
            .filter(|row| {
                row.is_skill
                    && row.steps.iter().any(|step| {
                        step.args.iter().any(|arg| {
                            matches!(arg, crate::content_tables::Arg::S("discard_count"))
                        })
                    })
            })
            .map(|row| (row.id, row.upgrade))
            .collect();
        assert_eq!(
            discard_count_skills,
            [(CardId::Stack, 0), (CardId::Stack, 1)],
            "the generated Skill catalog's direct former-pile readers"
        );

        for (upgrade, expected_block) in [(0, 0), (1, 3)] {
            let mut builder = CatalogBuilder::new();
            let identity = identity_at(CardId::Stack, upgrade);
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let stack = HotCard {
                uid: 7,
                atom,
                flags: 0,
            };
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.card_states.set_local_sly(7);
            state.piles.get_mut(PileId::Discard).make_mut().push(stack);
            assert_eq!(
                state.piles.get(PileId::Discard).len(),
                1,
                "Stack starts in Discard but moves before its body"
            );
            let mut events = Vec::new();

            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[stack],
                &mut events,
            )
            .unwrap();

            assert_eq!(
                state.block, expected_block,
                "Stack does not count its own Play source"
            );
            assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[stack]);
            assert_eq!(state.history.card_plays_finished_combat, 1);
        }
    }

    #[test]
    fn permanent_sly_up_my_sleeve_writes_hand_draw_and_discard_sources_then_routes_once() {
        let identity = identity_at(CardId::UpMySleeve, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let card = HotCard {
            uid: 7,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        for source_pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
            let mut state = HotState::at_defaults();
            state.hp = 70;
            state.card_states.set_local_sly(7);
            state.piles.get_mut(source_pile).make_mut().push(card);
            let mut events = Vec::new();

            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[card],
                &mut events,
            )
            .unwrap();

            assert!(state.piles.get(PileId::Play).is_empty());
            assert_eq!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .filter(|candidate| candidate.uid == card.uid)
                    .count(),
                1,
                "{source_pile:?}"
            );
            let modifiers = state
                .card_states
                .get_ref(7)
                .unwrap()
                .local_cost_modifiers
                .as_slice();
            assert_eq!(modifiers.len(), 1, "{source_pile:?}");
            assert_eq!(modifiers[0].amount, -1, "{source_pile:?}");
            assert_eq!(state.history.card_plays_finished_combat, 1);
        }
    }

    #[test]
    fn duplicate_sly_uid_refuses_the_complete_discard_transaction_atomically() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Tactician)).unwrap();
        let catalog = builder.build();
        let tactician = card(&catalog, CardId::Tactician, 1);
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.piles.get_mut(PileId::Hand).make_mut().push(tactician);
        // A second live object under the same UID: the Hand copy is
        // discarded, then the Sly pass cannot resolve a unique location.
        state.piles.get_mut(PileId::Draw).make_mut().push(tactician);
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let events_before = events.clone();

        assert_eq!(
            discard_and_draw(
                &mut state,
                &catalog,
                PileId::Hand,
                &[tactician],
                0,
                &mut events
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn terminal_sly_child_suppresses_every_later_collected_child() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::FlickFlack, CardId::Tactician] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        let mut events = Vec::new();
        let picked = [
            card(&catalog, CardId::FlickFlack, 1),
            card(&catalog, CardId::Tactician, 2),
        ];
        hand_of(&mut state, &picked);

        discard_and_draw(&mut state, &catalog, PileId::Hand, &picked, 0, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.energy, 3, "the later Tactician never runs");
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardPlayed { uid, .. } => Some(*uid),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [1]
        );
    }

    fn tingsha_discard_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::StrikeSilent,
            CardId::DefendSilent,
            CardId::Tactician,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        builder
            .set_relics(&[crate::ids::RelicId::RelicTingsha])
            .unwrap();
        builder.build()
    }

    /// Strike, Defend, Sly Tactician in Hand; one Strike in Draw; one
    /// Toadpole at `monster_hp`; Tingsha deals 3 per discard.
    fn tingsha_discard_state(catalog: &Catalog, monster_hp: i32) -> (HotState, [HotCard; 3]) {
        let picked = [
            card(catalog, CardId::StrikeSilent, 1),
            card(catalog, CardId::DefendSilent, 2),
            card(catalog, CardId::Tactician, 3),
        ];
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, monster_hp));
        hand_of(&mut state, &picked);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(catalog, CardId::StrikeSilent, 4));
        (state, picked)
    }

    fn uids(state: &HotState, pile: PileId) -> Vec<u32> {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect()
    }

    /// #3075: DiscardAndDraw RVA 0x3e0274 IL_0029–0035 leaves on
    /// IsOverOrEnding before its snapshot, so a command issued after combat
    /// is over records no history, moves nothing, draws nothing and runs no
    /// Sly child. The walk-only entry used by Calculated Gamble's owned tail
    /// is gated identically.
    #[test]
    fn discard_issued_after_combat_is_over_is_a_complete_noop() {
        let catalog = tingsha_discard_catalog();
        let (mut state, picked) = tingsha_discard_state(&catalog, 20);
        state.history.over = true;
        let before = state.clone();
        let mut events = Vec::new();

        discard_and_draw(&mut state, &catalog, PileId::Hand, &picked, 3, &mut events).unwrap();
        assert_eq!(state, before);
        assert!(events.is_empty());

        assert_eq!(
            discard_cards_collect_sly(&mut state, &catalog, PileId::Hand, &picked, &mut events),
            Ok(Vec::new())
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #3075: CardPileCmd.Add RVA 0x3e1ba4 IL_004e–008a skips the move once
    /// IsEnding holds, but DiscardAndDraw's loop has no ending break: a
    /// Tingsha kill on the first card leaves the rest in Hand while
    /// CardDiscarded history (IL_018e) is still recorded for every card. The
    /// paired Draw and the Sly Tactician do not run.
    #[test]
    fn tingsha_kill_mid_discard_leaves_the_rest_in_hand_and_counts_every_card() {
        let catalog = tingsha_discard_catalog();
        let (mut state, picked) = tingsha_discard_state(&catalog, 3);
        let mut events = Vec::new();

        discard_and_draw(&mut state, &catalog, PileId::Hand, &picked, 3, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(uids(&state, PileId::Discard), [1]);
        assert_eq!(uids(&state, PileId::Hand), [2, 3], "no card vanishes");
        assert_eq!(uids(&state, PileId::Draw), [4], "the paired Draw is gated");
        assert_eq!(state.history.discarded_cards_this_turn, 3);
        assert_eq!(state.energy, 3, "the kept Sly Tactician never auto-plays");
        assert_eq!(state.history.card_plays_finished_combat, 0);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    Event::CardResolved {
                        pile: PileId::Discard,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    /// The same walk that does not end combat moves every card in the
    /// given (pick) order, draws, then auto-plays the Sly Tactician.
    #[test]
    fn nonlethal_tingsha_discard_moves_every_card_in_order_then_draws_and_plays_sly() {
        let catalog = tingsha_discard_catalog();
        let (mut state, picked) = tingsha_discard_state(&catalog, 20);
        let reversed = [picked[2], picked[0], picked[1]];
        let mut events = Vec::new();

        discard_and_draw(
            &mut state,
            &catalog,
            PileId::Hand,
            &reversed,
            1,
            &mut events,
        )
        .unwrap();

        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 11);
        assert_eq!(state.history.discarded_cards_this_turn, 3);
        assert_eq!(state.energy, 4, "Tactician auto-played after the draw");
        assert_eq!(uids(&state, PileId::Hand), [4]);
        assert_eq!(
            uids(&state, PileId::Discard),
            [1, 2, 3],
            "Strike, Defend in pick order; the auto-played Tactician re-routes last"
        );
        assert_eq!(state.history.card_plays_finished_combat, 1);
    }

    /// A pick that is no longer in its source pile refuses by name, with no
    /// partial prefix (the command is rehearsed on a clone).
    #[test]
    fn discard_of_a_card_missing_from_its_source_pile_refuses_atomically() {
        let catalog = tingsha_discard_catalog();
        let (mut state, picked) = tingsha_discard_state(&catalog, 20);
        let stray = card(&catalog, CardId::StrikeSilent, 9);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            discard_and_draw(
                &mut state,
                &catalog,
                PileId::Hand,
                &[picked[0], stray],
                0,
                &mut events,
            ),
            Err(EngineRefusal::FrozenCardVanished {
                uid: 9,
                pile: PileId::Hand,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn stratagem_count_is_ordered_and_refuses_exact_u32_overflow() {
        for (n, k, expected) in [
            (0, 0, 1),
            (3, 0, 1),
            (2, 3, 0),
            (3, 1, 3),
            (3, 2, 6),
            (4, 3, 24),
            (5, 3, 60),
            (12, 12, 479_001_600),
        ] {
            assert_eq!(checked_stratagem_selection_count(n, k), Ok(expected));
        }
        assert_eq!(checked_stratagem_subset_count(13, 12), Ok(13));
        for (n, k) in [(13, 12), (usize::MAX, 1), (100, 5)] {
            assert_eq!(
                checked_stratagem_selection_count(n, k),
                Err(EngineRefusal::CounterOverflow("stratagem options"))
            );
        }
    }

    #[test]
    fn stratagem_auto_takes_the_complete_singleton_reshuffle() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Discard).make_mut().push(card(
            &catalog,
            CardId::DefendSilent,
            7,
        ));
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        let before = state.rng.get(RngStream::Rng);
        let mut events = Vec::new();

        reshuffle(&mut state, &catalog, &mut events).unwrap();

        assert!(state.piles.get(PileId::Draw).is_empty());
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 7);
        assert_eq!(state.rng.get(RngStream::Rng), before);
    }

    #[test]
    fn stratagem_redirects_selected_cards_at_the_hand_cap() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        for uid in 0..9 {
            state.piles.get_mut(PileId::Hand).make_mut().push(card(
                &catalog,
                CardId::DefendSilent,
                uid,
            ));
        }
        for uid in 9..12 {
            state.piles.get_mut(PileId::Draw).make_mut().push(card(
                &catalog,
                CardId::DefendSilent,
                uid,
            ));
        }
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 3);
        let mut events = Vec::new();

        stratagem_after_shuffle(&mut state, &catalog, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[9].uid, 9);
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
    }

    #[test]
    fn stratagem_no_choice_moves_native_rarity_model_order() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        builder.intern(identity(CardId::Rage)).unwrap();
        builder.intern(identity(CardId::Thunderclap)).unwrap();
        builder.intern(identity(CardId::Toxic)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        for uid in 10..19 {
            state.piles.get_mut(PileId::Hand).make_mut().push(card(
                &catalog,
                CardId::DefendSilent,
                uid,
            ));
        }
        // Native Draw view puts Common Thunderclap before Uncommon Rage and
        // Status Toxic, irrespective of reverse live shuffled pile order.
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(&catalog, CardId::Toxic, 3),
            card(&catalog, CardId::Rage, 1),
            card(&catalog, CardId::Thunderclap, 2),
        ]);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 3);

        stratagem_after_shuffle(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert_eq!(
            catalog
                .spec(state.piles.get(PileId::Hand).as_slice()[9].atom)
                .unwrap()
                .identity
                .id,
            CardId::Thunderclap
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect::<Vec<_>>(),
            [CardId::Rage, CardId::Toxic]
        );
    }

    #[test]
    fn exhaust_power_order_runs_before_the_card_local_drum_tail() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        builder.intern(identity(CardId::DefendIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::DefendIronclad,
            8,
        ));
        state.powers.set(PowerId::FeelNoPain, SlotWire::Int, 3);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::FeelNoPain, PowerId::DarkEmbrace,])
        );
        let mut events = Vec::new();

        card_exhausted(
            &mut state,
            &catalog,
            card(&catalog, CardId::DrumOfBattle, 7),
            &mut events,
        )
        .unwrap();

        assert_eq!(state.block, 3);
        assert_eq!(state.energy, 5, "the card-local tail follows both powers");
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Exhaust).len(), 1);
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
    }

    #[test]
    fn midnight_exhaust_snapshot_normalizes_legacy_siblings_in_native_order() {
        let mut builder = CatalogBuilder::new();
        let midnight_atom = builder.intern(identity(CardId::Midnight)).unwrap();
        builder.intern(identity(CardId::DefendIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 0;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: u32::MAX,
            atom: midnight_atom,
            flags: crate::hot::CARD_FLAG_LEGACY,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: u32::MAX,
            atom: midnight_atom,
            flags: crate::hot::CARD_FLAG_LEGACY,
        });

        card_exhausted(
            &mut state,
            &catalog,
            card(&catalog, CardId::DefendIronclad, 50),
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 0);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 1);
        assert_eq!(state.next_card_uid, 2);
        for uid in [0, 1] {
            assert_eq!(
                state.card_states.get(uid).local_cost_modifiers.as_slice(),
                [LocalCostModifier {
                    kind: LocalCostModifierKind::Add,
                    amount: -1,
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                }]
            );
        }
    }

    #[test]
    fn midnight_legacy_normalization_overflow_rolls_back_public_exhaust_play() {
        let mut builder = CatalogBuilder::new();
        let midnight_atom = builder.intern(identity(CardId::Midnight)).unwrap();
        let offering_atom = builder.intern(identity(CardId::Offering)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = super::super::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = u32::MAX;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.max_hp = 30;
        monster.loop_pos = 2;
        state.monsters_mut().push(monster);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 50,
                atom: offering_atom,
                flags: 0,
            },
            HotCard {
                uid: u32::MAX,
                atom: midnight_atom,
                flags: crate::hot::CARD_FLAG_LEGACY,
            },
        ]);
        let before = state.clone();

        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 50,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            ),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn a_reentrant_lethal_draw_suppresses_the_drum_energy_tail() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendIronclad)).unwrap();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = i16::MAX;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::DefendIronclad,
            8,
        ));
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 66);
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
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
        let mut events = Vec::new();

        card_exhausted(
            &mut state,
            &catalog,
            card(&catalog, CardId::DrumOfBattle, 7),
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(
            state.energy,
            i16::MAX,
            "terminal power listener suppresses Drum before overflow preflight"
        );
        assert_eq!(state.powers.value(PowerId::Burst), 2);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Exhaust).len(), 1);
    }

    #[test]
    fn drum_printed_grant_is_card_local_and_level_specific_without_replay() {
        for (upgrade, energy) in [(0, 2), (1, 3)] {
            let drum = identity_at(CardId::DrumOfBattle, upgrade);
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(drum).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;

            card_exhausted(
                &mut state,
                &catalog,
                HotCard {
                    uid: u32::from(upgrade) + 1,
                    atom,
                    flags: 0,
                },
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.energy, 3 + energy);
            assert_eq!(state.history.plays_this_turn, 0);
        }
    }

    #[test]
    fn drum_folds_physical_replay_then_burst_and_echo_and_freezes_the_count() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 1;
        state.history.plays_this_turn = 1;
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        state.powers.set(PowerId::EchoForm, SlotWire::Int, 2);
        let drum = HotCard {
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            ..card(&catalog, CardId::DrumOfBattle, 7)
        };
        let mut physical = state.card_states.get(drum.uid);
        physical.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(drum.uid, physical);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        card_exhausted(&mut state, &catalog, drum, &mut Vec::new()).unwrap();

        assert_eq!(
            state.energy, 11,
            "base + two physical replays + Burst + Echo each grant two"
        );
        assert_eq!(state.powers.value(PowerId::Burst), 1);
        assert_eq!(state.powers.value(PowerId::EchoForm), 2);
        assert_eq!(
            state.history.plays_this_turn, 1,
            "the listener does not publish card-play history"
        );

        let mut echo_window_closed = HotState::at_defaults();
        echo_window_closed.hp = 50;
        echo_window_closed
            .powers
            .set(PowerId::EchoForm, SlotWire::Int, 1);
        echo_window_closed.history.plays_this_turn = 1;
        card_exhausted(
            &mut echo_window_closed,
            &catalog,
            card(&catalog, CardId::DrumOfBattle, 8),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(echo_window_closed.energy, 5);
    }

    #[test]
    fn drum_burst_and_echo_sources_fold_independently() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        let catalog = builder.build();
        for (burst, echo, expected_burst) in [(1, 0, 0), (0, 1, 0)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::Burst, SlotWire::Int, burst);
            state.powers.set(PowerId::EchoForm, SlotWire::Int, echo);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

            card_exhausted(
                &mut state,
                &catalog,
                card(&catalog, CardId::DrumOfBattle, 7),
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.energy, 7);
            assert_eq!(state.powers.value(PowerId::Burst), expected_burst);
            assert_eq!(state.powers.value(PowerId::EchoForm), echo);
        }
    }

    #[test]
    fn malformed_drum_level_refuses_without_mutating_the_plan_inputs() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        let catalog = builder.build();
        let card = card(&catalog, CardId::DrumOfBattle, 7);
        let spec = catalog.spec(card.atom).unwrap();
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        state.powers.set(PowerId::EchoForm, SlotWire::Int, 2);
        state.history.plays_this_turn = 1;
        let before = state.clone();

        assert_eq!(
            drum_of_battle_plan(&state, spec, card, 2),
            Err(EngineRefusal::CounterOverflow("drum_of_battle level"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn drum_energy_overflow_refuses_before_the_local_fold() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DrumOfBattle)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = i16::MAX - 3;
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];

        assert_eq!(
            card_exhausted(
                &mut state,
                &catalog,
                card(&catalog, CardId::DrumOfBattle, 7),
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(state.powers.value(PowerId::Burst), 2);
        assert_eq!(state.energy, i16::MAX - 3);
        assert_eq!(state.piles.get(PileId::Exhaust).len(), 1);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn fiend_fire_drum_overflow_is_atomic_at_the_public_action_boundary() {
        let fiend = identity(CardId::FiendFire);
        let drum = identity(CardId::DrumOfBattle);
        let mut builder = CatalogBuilder::new();
        let fiend_atom = builder.intern(fiend).unwrap();
        let drum_atom = builder.intern(drum).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = i16::MAX;
        state.next_card_uid = 3;
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: fiend_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: drum_atom,
                flags: 0,
            },
        ]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let before = state.clone();

        let result = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        );

        assert_eq!(result, Err(EngineRefusal::CounterOverflow("energy")));
        assert_eq!(state, before, "the caller receives no partial successor");
    }

    #[test]
    fn void_loses_energy_after_the_draw_pipeline_and_floors_at_zero() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Void)).unwrap();
        let catalog = builder.build();
        for (before, after) in [(3, 2), (0, 0)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = before;
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(card(&catalog, CardId::Void, 1));
            let mut events = Vec::new();
            draw_cards(&mut state, &catalog, 1, DrawSource::Command, &mut events).unwrap();
            assert_eq!(state.energy, after);
            assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 1);
            assert_eq!(events, vec![Event::CardDrawn { uid: 1 }]);
        }
    }

    #[test]
    fn confused_draws_append_combat_long_costs_from_the_dedicated_stream() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicFakeSneckoEye])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(17);
        state.rng.set(
            RngStream::EnergyCosts,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Draw).make_mut().push(card(
            &catalog,
            CardId::StrikeIronclad,
            1,
        ));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        let rows = state
            .card_states
            .get(1)
            .local_cost_modifiers
            .as_slice()
            .to_vec();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, crate::hot::LocalCostModifierKind::Set);
        assert!((0..=3).contains(&rows[0].amount));
        assert_eq!(
            rows[0].expiration,
            crate::hot::LocalCostExpiration::ThisCombat
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 1);
        assert_ne!(
            state.piles.get(PileId::Hand).as_slice()[0].flags
                & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            0
        );
    }

    /// #3147: Confused `AfterCardDrawn` (RVA 0xa07f4) returns early only when
    /// `EnergyCost.Canonical < 0` (IL_0026..IL_0031), with no `CostsX` test,
    /// and `CardEnergyCost::.ctor` (RVA 0x11e002, IL_0022..IL_002d) makes an
    /// X-cost card's Canonical 0. So a drawn Cascade takes the roll, as
    /// Whirlwind does; its old -1 row skipped it and desynced the stream.
    #[test]
    fn confused_rolls_a_drawn_x_cost_cascade() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Cascade)).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicFakeSneckoEye])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(17);
        state.rng.set(
            RngStream::EnergyCosts,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(&catalog, CardId::Cascade, 1));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        let rows = state
            .card_states
            .get(1)
            .local_cost_modifiers
            .as_slice()
            .to_vec();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, crate::hot::LocalCostModifierKind::Set);
        assert_eq!(
            rows[0].expiration,
            crate::hot::LocalCostExpiration::ThisCombat
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 1);
    }
    /// #2926 witnesses: a Strike carrying Slither, a plain Strike, and a
    /// seeded `CombatEnergyCosts` stream.
    fn slither_identity() -> CardIdentity {
        CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Slither,
                amount: 1,
            }),
        }
    }

    fn slither_fixture(relics: &[RelicId], hellraiser: bool) -> (Catalog, HotState, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(slither_identity()).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        builder.set_relics(relics).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 10;
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 100)]);
        if hellraiser {
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        }
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(17);
        state.rng.set(
            RngStream::EnergyCosts,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        let slither = HotCard {
            uid: 1,
            atom,
            flags: 0,
        };
        (catalog, state, slither)
    }

    /// The next `n` `NextInt(4)` rolls of the fixture's seeded stream.
    fn energy_cost_rolls(n: usize) -> Vec<i64> {
        let mut rng = crate::rng::Xoshiro256StarStar::from_seed(17);
        (0..n)
            .map(|_| i64::from(rng.next_bounded(4).unwrap()))
            .collect()
    }

    fn slither_rows(state: &HotState, uid: u32) -> Vec<(LocalCostModifierKind, i64)> {
        state
            .card_states
            .get(uid)
            .local_cost_modifiers
            .as_slice()
            .iter()
            .map(|row| {
                assert_eq!(row.expiration, LocalCostExpiration::ThisCombat);
                assert!(!row.reduce_only);
                (row.kind, row.amount)
            })
            .collect()
    }

    #[test]
    fn slither_command_draw_rolls_one_combat_long_cost_on_energy_costs() {
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        let plain = card(&catalog, CardId::StrikeIronclad, 2);
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither, plain]));

        draw_cards(
            &mut state,
            &catalog,
            2,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        let rolled = energy_cost_rolls(1)[0];
        assert_eq!(
            slither_rows(&state, 1),
            [(LocalCostModifierKind::Set, rolled)]
        );
        // The plain Strike's draw leaves the stream and its own row alone.
        assert!(slither_rows(&state, 2).is_empty());
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 1);
        assert!(state.exact_piles);
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_ne!(hand[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(hand[1].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
    }

    #[test]
    fn slither_rolls_again_on_every_draw_and_appends_a_second_row() {
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither]));
        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut Vec::new(),
        )
        .unwrap();
        let card = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        state.piles.get_mut(PileId::Draw).make_mut().push(card);
        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        let rolls = energy_cost_rolls(2);
        assert_eq!(
            slither_rows(&state, 1),
            [
                (LocalCostModifierKind::Set, rolls[0]),
                (LocalCostModifierKind::Set, rolls[1])
            ]
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 2);
    }

    #[test]
    fn slither_rolls_after_confused_on_the_same_stream() {
        let (catalog, mut state, slither) = slither_fixture(&[RelicId::RelicFakeSneckoEye], false);
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither]));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        // Player powers precede the card-pile enchantment in the listener
        // snapshot: Confused takes the first roll, Slither the second, and
        // the later Set row is the one the cost reads.
        let rolls = energy_cost_rolls(2);
        assert_eq!(
            slither_rows(&state, 1),
            [
                (LocalCostModifierKind::Set, rolls[0]),
                (LocalCostModifierKind::Set, rolls[1])
            ]
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 2);
    }

    #[test]
    fn slither_does_not_roll_when_the_ordinary_walk_never_starts() {
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        state.hp = 0;
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither]));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert!(slither_rows(&state, 1).is_empty());
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 0);
    }

    #[test]
    fn slither_does_not_roll_once_hellraiser_played_its_card_out_of_hand() {
        let (catalog, mut state, slither) = slither_fixture(&[], true);
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither]));

        draw_cards(
            &mut state,
            &catalog,
            1,
            DrawSource::HandDraw,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|c| c.uid)
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(slither_rows(&state, 1).is_empty());
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 0);
    }

    #[test]
    fn slither_turn_start_draw_rolls_through_the_resumable_path() {
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![slither]));

        let result = draw_cards_for_potion(
            &mut state,
            &catalog,
            1,
            DrawCaller::TurnStart,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(result, PotionDrawResult::Complete);
        let rolled = energy_cost_rolls(1)[0];
        assert_eq!(
            slither_rows(&state, 1),
            [(LocalCostModifierKind::Set, rolled)]
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 1);
        assert!(state.exact_piles);
    }

    #[test]
    fn slither_refuses_a_legacy_drawn_copy_by_name() {
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        let legacy = HotCard {
            uid: crate::hot::LEGACY_CARD_UID,
            flags: crate::hot::CARD_FLAG_LEGACY,
            ..slither
        };
        state
            .piles
            .set(PileId::Draw, HotPile::from_cards(vec![legacy]));

        assert_eq!(
            draw_cards(
                &mut state,
                &catalog,
                1,
                DrawSource::Command,
                &mut Vec::new()
            ),
            Err(EngineRefusal::MalformedArgs("Slither legacy drawn card"))
        );
    }

    #[test]
    fn slither_refuses_a_retained_removed_drawn_object_by_name() {
        // Native `Pile` is null on a removed object and IL_0021 would
        // dereference it; the retained Hellraiser quotient refuses instead.
        let (catalog, mut state, slither) = slither_fixture(&[], false);
        state
            .card_states
            .retain_removed_draw_object(crate::hot::FrozenAutoBatchEntry {
                card: slither,
                state: crate::hot::CardInstanceState::default(),
            })
            .unwrap();

        assert_eq!(
            slither_after_card_drawn(&mut state, &catalog, slither, true),
            Err(EngineRefusal::MalformedArgs(
                "Slither drawn card outside every pile"
            ))
        );
        assert_eq!(state.rng.get(RngStream::EnergyCosts).counter, 0);
    }

    #[test]
    fn slither_identity_needs_a_nonnegative_canonical_cost() {
        let (catalog, _, slither) = slither_fixture(&[], false);
        let spec = *catalog.spec(slither.atom).unwrap();
        assert!(slither_identity_is_exact(&spec));
        assert!(crate::engine::play::body_enchantment_is_exact(&spec));
        assert!(!slither_identity_is_exact(&crate::catalog::CardSpec {
            cost: -1,
            ..spec
        }));
        let plain = *catalog
            .spec(catalog.atom(&identity(CardId::StrikeIronclad)).unwrap())
            .unwrap();
        assert!(!slither_identity_is_exact(&plain));
    }

    #[test]
    fn queen_blockers_perfect_fit_preserves_uid_order_and_rng() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        let ordinary = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let fit = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment {
                    id: EnchantmentId::PerfectFit,
                    amount: 1,
                }),
            })
            .unwrap();
        let catalog = builder.build();
        let cards = vec![
            HotCard {
                uid: 2,
                atom: fit,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 0,
                atom: ordinary,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 1,
                atom: fit,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ];
        let mut state = HotState::at_defaults();
        let before_rng = state.rng.get(RngStream::Rng);
        state.piles.set(PileId::Discard, HotPile::from_cards(cards));
        reshuffle(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|c| c.uid)
                .collect::<Vec<_>>(),
            vec![1, 2, 0]
        );
        let mut rng = Xoshiro256StarStar {
            words: before_rng.words,
            counter: before_rng.counter,
        };
        rng.shuffle(&mut [0, 1, 2]).unwrap();
        assert_eq!(
            state.rng.get(RngStream::Rng),
            RngStreamState {
                words: rng.words,
                counter: rng.counter
            }
        );
        assert!(state.piles.get(PileId::Discard).is_empty());
    }

    /// #3193 fixture: plain Strike/Defend fillers, Perfect Fit on two
    /// distinct card ids (Bash, Anger), and Bash+ with Perfect Fit as a
    /// catalog-only upgrade target. No two cards tie on `(id, upgrade)` with
    /// different payloads, so the pre-shuffle sort is payload-exact.
    fn perfect_fit_identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::PerfectFit,
                amount: 1,
            }),
        }
    }

    fn perfect_fit_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        builder.intern(identity(CardId::DefendIronclad)).unwrap();
        builder
            .intern(perfect_fit_identity(CardId::Bash, 0))
            .unwrap();
        builder
            .intern(perfect_fit_identity(CardId::Bash, 1))
            .unwrap();
        builder
            .intern(perfect_fit_identity(CardId::Anger, 0))
            .unwrap();
        builder.build()
    }

    fn fit_card(catalog: &Catalog, id: CardId, upgrade: u8, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog.atom(&perfect_fit_identity(id, upgrade)).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        }
    }

    /// Reshuffle `discard` from one fixed Rng state; return the Draw atoms,
    /// Draw uids and the number of Rng draws spent.
    fn reshuffle_from(
        catalog: &Catalog,
        discard: Vec<HotCard>,
        exact: bool,
    ) -> Result<(Vec<u16>, Vec<u32>, u64), EngineRefusal> {
        let mut state = HotState::at_defaults();
        state.exact_piles = exact;
        state
            .piles
            .set(PileId::Discard, HotPile::from_cards(discard));
        let before = state.rng.get(RngStream::Rng).counter;
        reshuffle(&mut state, catalog, &mut Vec::new())?;
        assert!(state.piles.get(PileId::Discard).is_empty());
        let draw = state.piles.get(PileId::Draw).as_slice();
        Ok((
            draw.iter().map(|card| card.atom).collect(),
            draw.iter().map(|card| card.uid).collect(),
            state.rng.get(RngStream::Rng).counter - before,
        ))
    }

    /// Every rotation of `cards`: distinct inexact Discard orders over one
    /// multiset.
    fn rotations(cards: &[HotCard]) -> Vec<Vec<HotCard>> {
        (0..cards.len())
            .map(|shift| {
                let mut rotated = cards.to_vec();
                rotated.rotate_left(shift);
                rotated
            })
            .collect()
    }

    #[test]
    fn issue3193_reshuffle_without_perfect_fit_spends_n_minus_one_draws() {
        let catalog = perfect_fit_catalog();
        let discard = vec![
            card(&catalog, CardId::StrikeIronclad, 1),
            card(&catalog, CardId::DefendIronclad, 2),
            card(&catalog, CardId::StrikeIronclad, 3),
            card(&catalog, CardId::DefendIronclad, 4),
        ];
        let (atoms, _, spent) = reshuffle_from(&catalog, discard.clone(), false).unwrap();
        assert_eq!(spent, 3);
        // Sort then one UnstableShuffle pass over the same Rng state.
        let mut sorted = crate::dotnet_sort::dotnet_list_sort_by_key(&discard, |card| {
            let spec = catalog.spec(card.atom).unwrap();
            (spec.identity.id as u16, spec.identity.upgrade)
        });
        let live = HotState::at_defaults().rng.get(RngStream::Rng);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        rng.shuffle(&mut sorted).unwrap();
        assert_eq!(
            atoms,
            sorted.iter().map(|card| card.atom).collect::<Vec<_>>()
        );
    }

    #[test]
    fn issue3193_one_perfect_fit_tops_draw_for_every_inexact_discard_order() {
        let catalog = perfect_fit_catalog();
        let fit = fit_card(&catalog, CardId::Bash, 0, 5);
        let cards = vec![
            card(&catalog, CardId::StrikeIronclad, 1),
            card(&catalog, CardId::DefendIronclad, 2),
            fit,
            card(&catalog, CardId::StrikeIronclad, 3),
            card(&catalog, CardId::DefendIronclad, 4),
        ];
        let mut outcomes = Vec::new();
        for discard in rotations(&cards) {
            let (atoms, uids, spent) = reshuffle_from(&catalog, discard, false).unwrap();
            // The listener is spent by no Rng draw of its own.
            assert_eq!(spent, 4);
            assert_eq!(uids[0], fit.uid);
            assert_eq!(atoms[0], fit.atom);
            outcomes.push(atoms);
        }
        // The payload order is independent of the order an inexact state
        // presents Discard in: exactly what admission now relies on.
        assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn issue3193_two_equal_perfect_fit_copies_are_payload_order_free() {
        let catalog = perfect_fit_catalog();
        let cards = vec![
            fit_card(&catalog, CardId::Bash, 0, 7),
            card(&catalog, CardId::StrikeIronclad, 1),
            card(&catalog, CardId::DefendIronclad, 2),
            fit_card(&catalog, CardId::Bash, 0, 8),
            card(&catalog, CardId::StrikeIronclad, 3),
        ];
        let fit_atom = cards[0].atom;
        let mut outcomes = Vec::new();
        for discard in rotations(&cards) {
            let (atoms, uids, spent) = reshuffle_from(&catalog, discard, false).unwrap();
            assert_eq!(spent, 4);
            assert_eq!(&atoms[..2], &[fit_atom, fit_atom]);
            let mut top = uids[..2].to_vec();
            top.sort_unstable();
            assert_eq!(top, vec![7, 8]);
            outcomes.push(atoms);
        }
        assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn issue3193_two_distinct_perfect_fits_follow_physical_discard_order_when_exact() {
        let catalog = perfect_fit_catalog();
        let bash = fit_card(&catalog, CardId::Bash, 0, 5);
        let anger = fit_card(&catalog, CardId::Anger, 0, 6);
        let filler = [
            card(&catalog, CardId::StrikeIronclad, 1),
            card(&catalog, CardId::DefendIronclad, 2),
        ];
        // Listeners fire in live Discard order; the last one ends on top.
        let (_, uids, spent) =
            reshuffle_from(&catalog, vec![bash, filler[0], anger, filler[1]], true).unwrap();
        assert_eq!(spent, 3);
        assert_eq!(&uids[..2], &[anger.uid, bash.uid]);
        let (_, uids, spent) =
            reshuffle_from(&catalog, vec![anger, filler[0], bash, filler[1]], true).unwrap();
        assert_eq!(spent, 3);
        assert_eq!(&uids[..2], &[bash.uid, anger.uid]);
    }

    #[test]
    fn issue3193_two_distinct_perfect_fits_refuse_inexact_before_any_move() {
        let catalog = perfect_fit_catalog();
        let discard = vec![
            fit_card(&catalog, CardId::Bash, 0, 5),
            card(&catalog, CardId::StrikeIronclad, 1),
            fit_card(&catalog, CardId::Anger, 0, 6),
        ];
        let mut state = HotState::at_defaults();
        state
            .piles
            .set(PileId::Discard, HotPile::from_cards(discard.clone()));
        let before = state.clone();
        assert_eq!(
            reshuffle(&mut state, &catalog, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Perfect Fit physical shuffle order"
            ))
        );
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), &discard[..]);
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(
            state.rng.get(RngStream::Rng),
            before.rng.get(RngStream::Rng)
        );
    }

    #[test]
    fn issue3193_admission_judges_live_physical_perfect_fit_identities() {
        let catalog = perfect_fit_catalog();
        let mut state = HotState::at_defaults();
        // Bash+ (Perfect Fit) is in the catalog only as an upgrade target.
        state.piles.set(
            PileId::Draw,
            HotPile::from_cards(vec![
                fit_card(&catalog, CardId::Bash, 0, 5),
                card(&catalog, CardId::StrikeIronclad, 1),
            ]),
        );
        assert!(!perfect_fit_shuffle_needs_exact_piles(&state, &catalog));

        let mut copies = state.clone();
        copies
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(fit_card(&catalog, CardId::Bash, 0, 6));
        assert!(!perfect_fit_shuffle_needs_exact_piles(&copies, &catalog));

        let mut distinct = state.clone();
        distinct
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(fit_card(&catalog, CardId::Bash, 1, 6));
        assert!(perfect_fit_shuffle_needs_exact_piles(&distinct, &catalog));
        distinct.exact_piles = true;
        assert!(!perfect_fit_shuffle_needs_exact_piles(&distinct, &catalog));

        let mut legacy = state.clone();
        legacy.piles.get_mut(PileId::Draw).make_mut()[0].uid = crate::hot::LEGACY_CARD_UID;
        assert!(perfect_fit_shuffle_needs_exact_piles(&legacy, &catalog));
        legacy.exact_piles = true;
        assert!(!perfect_fit_shuffle_needs_exact_piles(&legacy, &catalog));
    }

    /// #3099 fixture: one Toadpole at `monster_hp`, the payout-bearing power
    /// listeners in `order` (each countdown one draw from paying), and
    /// `drawn` on top of Draw.
    fn walk_gate_fixture(drawn: HotCard, monster_hp: i32, order: &[PowerId]) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, monster_hp));
        state.piles.get_mut(PileId::Draw).make_mut().push(drawn);
        for power in order {
            let amount = match power {
                PowerId::Speedster | PowerId::Cacophony => 3,
                PowerId::CorrosiveWave => 2,
                _ => 1,
            };
            state.powers.set(*power, SlotWire::Int, amount);
        }
        assert!(state.fanouts.set_after_card_drawn_order(order));
        state.fanouts.set_automation_left(1);
        state.fanouts.set_cacophony_left(1);
        state
    }

    fn walk_gate_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::KinglyKick)).unwrap();
        builder.intern(identity(CardId::Void)).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        builder.build()
    }

    const WALK_GATE_ORDER: [PowerId; 4] = [
        PowerId::Speedster,
        PowerId::CorrosiveWave,
        PowerId::Automation,
        PowerId::Cacophony,
    ];

    /// #3099: `<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`
    /// IL_0028-0042 yields an empty walk when combat is ending at its start.
    /// No ordinary listener runs then: no power payout or countdown, no
    /// Kingly row, no Void loss — on the plain Draw, the resumable Draw, and
    /// the legacy payload iteration alike.
    #[test]
    fn no_after_card_drawn_listener_runs_when_the_walk_never_starts() {
        let catalog = walk_gate_catalog();
        let legacy = HotCard {
            uid: crate::hot::LEGACY_CARD_UID,
            ..card(&catalog, CardId::StrikeIronclad, 0)
        };
        for drawn in [
            card(&catalog, CardId::KinglyKick, 17),
            card(&catalog, CardId::Void, 18),
            legacy,
        ] {
            for resumable in [false, true] {
                if drawn.uid == crate::hot::LEGACY_CARD_UID && !resumable {
                    continue;
                }
                let mut state = walk_gate_fixture(drawn, 50, &WALK_GATE_ORDER);
                state.hp = 0;
                if resumable {
                    let result = draw_cards_for_potion(
                        &mut state,
                        &catalog,
                        1,
                        DrawCaller::TurnStart,
                        &mut Vec::new(),
                    )
                    .unwrap();
                    assert_eq!(result, PotionDrawResult::Complete);
                } else {
                    draw_cards(
                        &mut state,
                        &catalog,
                        1,
                        DrawSource::Command,
                        &mut Vec::new(),
                    )
                    .unwrap();
                }
                let label = format!("uid={} resumable={resumable}", drawn.uid);
                assert_eq!(state.piles.get(PileId::Hand).len(), 1, "{label}");
                assert_eq!(state.monsters[0].hp, 50, "{label}");
                assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
                assert_eq!(state.energy, 3, "{label}: no Automation, no Void");
                assert_eq!(state.fanouts.automation_left(), 1, "{label}");
                assert_eq!(state.fanouts.cacophony_left(), 1, "{label}");
                assert_eq!(state.rng.get(RngStream::Targets).counter, 0);
                assert!(
                    state
                        .card_states
                        .get(17)
                        .local_cost_modifiers
                        .as_slice()
                        .is_empty(),
                    "{label}: Kingly Kick never ran"
                );
            }
        }
    }

    /// #3099: a combat that ends mid-walk still reaches every later
    /// listener, because the walk-start gate is read once. Speedster kills
    /// the sole enemy on a Command Draw; CorrosiveWave then applies nothing
    /// (`PowerCmd.Apply`'s own IsEnding gate, and no hittable enemy),
    /// Automation's counter still pays and resets while `GainEnergy`'s own
    /// IsEnding gate withholds the energy, Cacophony's countdown still resets
    /// with nothing to roll, and the drawn card's own listener still runs —
    /// Kingly's row lands, Void's `LoseEnergy` is withheld by its own gate.
    #[test]
    fn a_mid_walk_kill_still_reaches_every_later_after_card_drawn_listener() {
        let catalog = walk_gate_catalog();
        for drawn in [
            card(&catalog, CardId::KinglyKick, 17),
            card(&catalog, CardId::Void, 18),
        ] {
            let mut state = walk_gate_fixture(drawn, 3, &WALK_GATE_ORDER);
            draw_cards(
                &mut state,
                &catalog,
                1,
                DrawSource::Command,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(state.history.over, "Speedster ended combat mid-walk");
            assert_eq!(state.monsters[0].hp, 0);
            assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
            assert_eq!(state.fanouts.automation_left(), 10, "Automation ran");
            assert_eq!(state.energy, 3, "GainEnergy/LoseEnergy gate on IsEnding");
            assert_eq!(state.fanouts.cacophony_left(), 33, "Cacophony ran");
            assert_eq!(state.fanouts.cacophony_resets_completed(), 1);
            assert_eq!(state.rng.get(RngStream::Targets).counter, 0);
            let kingly_rows = state
                .card_states
                .get(17)
                .local_cost_modifiers
                .as_slice()
                .len();
            assert_eq!(kingly_rows, usize::from(drawn.uid == 17));
        }

        // The resumable walk: Cacophony's roll kills the sole enemy on the
        // turn-start Draw, and the later Automation listener still runs.
        let mut state = walk_gate_fixture(
            card(&catalog, CardId::KinglyKick, 17),
            3,
            &[PowerId::Cacophony, PowerId::Automation],
        );
        let result = draw_cards_for_potion(
            &mut state,
            &catalog,
            1,
            DrawCaller::TurnStart,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(result, PotionDrawResult::Complete);
        assert!(state.history.over);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
        assert_eq!(state.fanouts.automation_left(), 10, "Automation ran");
        assert_eq!(state.energy, 3);
        assert_eq!(
            state
                .card_states
                .get(17)
                .local_cost_modifiers
                .as_slice()
                .len(),
            1
        );
    }

    /// #3099: Speedster and Cacophony target `HittableEnemies` without a
    /// combat-ending predicate. Rust ends combat only with no enemy alive, so
    /// an ended combat with a living enemy is outside the port: it refuses by
    /// name rather than skipping a Damage native would deal.
    #[test]
    fn an_after_card_drawn_target_after_combat_ended_with_a_living_enemy_refuses() {
        let catalog = walk_gate_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        assert_eq!(after_card_drawn_hittable_enemies(&state, "x"), Ok(None));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        assert_eq!(
            after_card_drawn_hittable_enemies(&state, "x"),
            Ok(Some(vec![0]))
        );
        state.history.over = true;
        for (power, name) in [
            (PowerId::Speedster, "Speedster after combat ended"),
            (PowerId::Cacophony, "Cacophony after combat ended"),
        ] {
            let mut probe = state.clone();
            probe.powers.set(power, SlotWire::Int, 3);
            probe.fanouts.set_cacophony_left(1);
            assert!(probe.fanouts.set_after_card_drawn_order(&[power]));
            assert_eq!(
                powers_after_card_drawn(
                    &mut probe,
                    &catalog,
                    card(&catalog, CardId::StrikeIronclad, 7),
                    DrawSource::Command,
                    None,
                    false,
                    &mut Vec::new(),
                ),
                Err(EngineRefusal::PowerOrderNotModeled(name))
            );
        }
    }

    /// #3021 witnesses: a catalog holding both Automation levels and a
    /// filler draw card, plus a helper that plays one Automation step.
    fn automation_fixture() -> (Catalog, HotState) {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity_at(CardId::Automation, 0)).unwrap();
        builder.intern(identity_at(CardId::Automation, 1)).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let draw = (100..130)
            .map(|uid| card(&catalog, CardId::StrikeIronclad, uid))
            .collect::<Vec<_>>();
        state.piles.get_mut(PileId::Draw).make_mut().extend(draw);
        (catalog, state)
    }

    fn play_automation(
        state: &mut HotState,
        catalog: &Catalog,
        upgrade: u8,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog
            .spec(
                catalog
                    .atom(&identity_at(CardId::Automation, upgrade))
                    .unwrap(),
            )
            .unwrap();
        let mut events = Vec::new();
        crate::steps::apply_step(
            crate::ids::StepKind::Automation,
            &mut crate::engine::StepCtx {
                state,
                catalog,
                spec: &spec,
                source_uid: 0,
                target: None,
                selection: None,
                x_value: 0,
                args: &[crate::catalog::CompiledArg::I(1)],
                events: &mut events,
            },
        )
    }

    fn draw_into_empty_hand(state: &mut HotState, catalog: &Catalog, n: usize) {
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        draw_cards(state, catalog, n, DrawSource::Command, &mut Vec::new()).unwrap();
    }

    #[test]
    fn automation_out_of_step_instances_pay_only_their_own_amount() {
        // The #3013 shape: the second (upgraded) play lands mid-countdown,
        // so its object starts a fresh ten-card count of its own.
        let (catalog, mut state) = automation_fixture();
        play_automation(&mut state, &catalog, 0).unwrap();
        assert!(state.fanouts.automation_later_instances().is_empty());
        draw_into_empty_hand(&mut state, &catalog, 3);
        assert_eq!(state.fanouts.automation_left(), 7);

        play_automation(&mut state, &catalog, 1).unwrap();
        assert_eq!(state.powers.value(PowerId::Automation), 2);
        assert_eq!(
            state.fanouts.automation_later_instances(),
            &[crate::hot::AutomationInstance {
                amount: 1,
                cards_left: 10,
            }]
        );
        assert_eq!(
            state.fanouts.after_card_drawn_order(),
            &[PowerId::Automation]
        );
        let energy = state.energy;
        draw_into_empty_hand(&mut state, &catalog, 7);
        // Only the first object reached zero: +1, not the stacked +2.
        assert_eq!(state.energy, energy + 1);
        assert_eq!(state.fanouts.automation_left(), 10);
        assert_eq!(state.fanouts.automation_later_instances()[0].cards_left, 3);
        draw_into_empty_hand(&mut state, &catalog, 3);
        assert_eq!(state.energy, energy + 2);
        assert_eq!(state.fanouts.automation_left(), 7);
        assert_eq!(state.fanouts.automation_later_instances()[0].cards_left, 10);
    }

    #[test]
    fn automation_in_step_instances_each_pay_on_the_same_draw() {
        let (catalog, mut state) = automation_fixture();
        play_automation(&mut state, &catalog, 0).unwrap();
        play_automation(&mut state, &catalog, 1).unwrap();
        play_automation(&mut state, &catalog, 0).unwrap();
        assert_eq!(state.powers.value(PowerId::Automation), 3);
        assert_eq!(state.fanouts.automation_later_instances().len(), 2);
        let energy = state.energy;
        draw_into_empty_hand(&mut state, &catalog, 9);
        assert_eq!(state.energy, energy);
        draw_into_empty_hand(&mut state, &catalog, 1);
        assert_eq!(state.energy, energy + 3);
        assert_eq!(state.fanouts.automation_left(), 10);
        assert!(
            state
                .fanouts
                .automation_later_instances()
                .iter()
                .all(|instance| instance.cards_left == 10)
        );
    }

    #[test]
    fn automation_instance_behind_a_foreign_draw_listener_refuses_by_name() {
        let (catalog, mut state) = automation_fixture();
        play_automation(&mut state, &catalog, 0).unwrap();
        state.powers.set(PowerId::Speedster, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Automation, PowerId::Speedster])
        );
        let before = state.clone();
        assert_eq!(
            play_automation(&mut state, &catalog, 1),
            Err(EngineRefusal::PowerOrderNotModeled(
                "automation instance behind a later draw listener"
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn automation_rows_exceeding_the_scalar_refuse_at_the_listener() {
        let (catalog, mut state) = automation_fixture();
        play_automation(&mut state, &catalog, 0).unwrap();
        assert!(
            state
                .fanouts
                .set_automation_later_instances(&[crate::hot::AutomationInstance {
                    amount: 1,
                    cards_left: 10,
                }])
        );
        assert_eq!(
            draw_cards(
                &mut state,
                &catalog,
                1,
                DrawSource::Command,
                &mut Vec::new()
            ),
            Err(EngineRefusal::MalformedArgs("automation instances"))
        );
    }
}
