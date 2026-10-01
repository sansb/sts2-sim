//! The admitted v0.111.0 orb command surface.
//!
//! Issue #1394 lands the subsystem in serialized slices. The completed surface
//! owns all five current-build kinds, channel/capacity commands, retained and
//! dequeuing evokes, both turn-boundary walks, the shared Focus fold, and the
//! acquisition-ordered orb power listeners.

use crate::catalog::Catalog;
use crate::decimal::DotNetDecimal;
use crate::hooks::HookEvent;
use crate::hot::{HotOrb, HotState, MAX_ORB_SLOTS, OrbKind, OrbResetPower};
use crate::ids::{PowerId, RelicId};
use crate::powers::SlotWire;

use super::damage::{
    alive_targets, damage_monster_after_catalog_auth as damage_monster, note_power, roll_target,
};
use super::{EngineRefusal, Event, Subject, fire_hook};

/// `_orb_passive_effect` uses `LIGHTNING_PASSIVE` (frozen Python, deleted #2827)
/// (`LightningOrb::get_PassiveVal`, RVA `0x258910`).
const LIGHTNING_PASSIVE: i64 = 3;

/// `_orb_evoke_effect` uses `LIGHTNING_EVOKE` (frozen Python, deleted #2827)
/// (`LightningOrb::get_EvokeVal`, RVA `0x25891e`).
const LIGHTNING_EVOKE: i64 = 8;
const FROST_PASSIVE: i64 = 2;
const FROST_EVOKE: i64 = 5;
const DARK_PASSIVE: i64 = 6;
const PLASMA_PASSIVE: i16 = 1;
const PLASMA_EVOKE: i16 = 2;

/// `OrbModel.GetRandomOrb` over its exact current-build pool.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `OrbModel::.cctor` (RVA `0x82dec`) fixes this exact order and
/// `GetRandomOrb` (RVA `0x82809`) performs one `NextItem` draw from the
/// caller's `CombatOrbGeneration` stream.
const RANDOM_ORB_POOL: [OrbKind; 5] = [
    OrbKind::Lightning,
    OrbKind::Frost,
    OrbKind::Dark,
    OrbKind::Plasma,
    OrbKind::Glass,
];

fn select_random_orb(state: &mut HotState) -> Result<OrbKind, EngineRefusal> {
    let live = state.rng.get(crate::hot::RngStream::CombatOrbs);
    if live.counter == u64::MAX {
        return Err(EngineRefusal::CounterOverflow("random orb bound"));
    }
    let mut rng = crate::rng::Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let index = rng
        .next_bounded(RANDOM_ORB_POOL.len() as i32)
        .map_err(|_| EngineRefusal::CounterOverflow("random orb bound"))?;
    state.rng.set(
        crate::hot::RngStream::CombatOrbs,
        crate::hot::RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(RANDOM_ORB_POOL[index as usize])
}

/// Run Chaos's authenticated serial random-Channel body.
///
/// Each iteration owns one monotonic lifecycle identity. Selection consumes
/// CombatOrbs before Channel checks ending; consequently a terminal first
/// Channel still leaves later iterations' identities and RNG draws visible,
/// while Channel itself becomes a no-op. No admitted orb effect or listener
/// can open an external selection, so every auth record is created and
/// consumed synchronously and the canonical auth tuple remains empty at the
/// action boundary.
pub(crate) fn channel_random_loop(
    state: &mut HotState,
    catalog: &Catalog,
    count: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    state
        .orbs
        .next_random_orb_progress_uid()
        .checked_add(count)
        .ok_or(EngineRefusal::CounterOverflow(
            "next_random_orb_progress_uid",
        ))?;
    for _ in 0..count {
        state
            .orbs
            .allocate_random_orb_progress_uid()
            .ok_or(EngineRefusal::CounterOverflow(
                "next_random_orb_progress_uid",
            ))?;
        let kind = select_random_orb(state)?;
        channel(state, catalog, kind, events)?;
    }
    Ok(())
}

/// Run Consuming Shadow's fixed-Dark Channel body, then apply one stack.
pub(crate) fn consuming_shadow_card(
    state: &mut HotState,
    catalog: &Catalog,
    count: u8,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let current = state.powers.value(PowerId::ConsumingShadow);
    let mut ledger_probe = state.clone();
    super::turn::prepare_after_side_turn_end_scalar_write(
        &mut ledger_probe,
        PowerId::ConsumingShadow,
        current,
        if current == 0 { 1 } else { current },
    )?;
    state
        .orbs
        .next_random_orb_progress_uid()
        .checked_add(u32::from(count))
        .ok_or(EngineRefusal::CounterOverflow(
            "next_random_orb_progress_uid",
        ))?;
    for _ in 0..count {
        state
            .orbs
            .allocate_random_orb_progress_uid()
            .ok_or(EngineRefusal::CounterOverflow(
                "next_random_orb_progress_uid",
            ))?;
        channel(state, catalog, OrbKind::Dark, events)?;
    }
    if !state.history.over {
        let current = state.powers.value(PowerId::ConsumingShadow);
        let amount = current
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("consuming shadow"))?;
        super::turn::prepare_after_side_turn_end_scalar_write(
            state,
            PowerId::ConsumingShadow,
            current,
            amount,
        )?;
        state
            .powers
            .set(PowerId::ConsumingShadow, SlotWire::Int, amount);
        note_power(events, Subject::Player, PowerId::ConsumingShadow, amount);
    }
    Ok(())
}

/// Run the power's owner-side-end authenticated EvokeLast loop.
///
/// The dispatcher allocates its epoch before the native initial-empty gate.
/// A nonempty initial queue starts one child lifecycle per live Amount
/// iteration. Each child rechecks ending and queue emptiness independently;
/// draining the queue therefore does not truncate the receipt/uid suffix.
#[cfg(test)]
pub(crate) fn consuming_shadow_side_end(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let amount = state.powers.value(PowerId::ConsumingShadow);
    consuming_shadow_side_end_inner(state, catalog, events, None, amount)
}

/// Invoke one captured Consuming Shadow object. While that uid remains the
/// attached keyed object, native re-reads its live Amount at every loop
/// back-edge. After removal/reapplication the old object continues with its
/// retained Amount and the replacement is deferred to the next hook.
pub(crate) fn consuming_shadow_side_end_with_captured_amount(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
    uid: u32,
    captured_amount: i32,
) -> Result<(), EngineRefusal> {
    consuming_shadow_side_end_inner(state, catalog, events, Some(uid), captured_amount)
}

fn consuming_shadow_side_end_inner(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
    captured_uid: Option<u32>,
    captured_amount: i32,
) -> Result<(), EngineRefusal> {
    let live_amount = |state: &HotState| {
        captured_uid
            .filter(|uid| {
                state
                    .fanouts
                    .after_side_turn_end_power_order()
                    .iter()
                    .any(|entry| {
                        entry.uid == *uid
                            && entry.token
                                == crate::hot::AfterSideTurnEndPowerToken::ConsumingShadow
                    })
            })
            .map_or(captured_amount, |_| {
                state.powers.value(PowerId::ConsumingShadow)
            })
    };
    let amount = live_amount(state);
    if amount <= 0 {
        return Ok(());
    }
    state
        .orbs
        .allocate_consuming_shadow_side_end_auth_uid()
        .ok_or(EngineRefusal::CounterOverflow(
            "next_consuming_shadow_side_end_auth_uid",
        ))?;
    if state.orbs.as_slice().is_empty() {
        return Ok(());
    }
    let amount: u32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("consuming shadow amount"))?;
    state
        .orbs
        .next_random_orb_progress_uid()
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow(
            "next_random_orb_progress_uid",
        ))?;
    let mut cursor = 0_u32;
    while cursor
        < live_amount(state)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("consuming shadow amount"))?
    {
        state
            .orbs
            .allocate_random_orb_progress_uid()
            .ok_or(EngineRefusal::CounterOverflow(
                "next_random_orb_progress_uid",
            ))?;
        evoke_back(state, catalog, events)?;
        cursor += 1;
    }
    Ok(())
}

/// `OrbModel::ModifyOrbValue` through FocusPower.
///
/// Python `orb_value` (frozen, deleted #2827) applies permanent and temporary Focus on
/// every read, then clamps once at zero. Plasma bypasses this helper; the
/// remaining kinds join it in the final #1394 slice.
pub(crate) fn orb_value(state: &HotState, base: i64) -> Result<i64, EngineRefusal> {
    base.checked_add(i64::from(state.powers.value(PowerId::Focus)))
        .and_then(|value| value.checked_add(i64::from(state.orbs.temp_focus())))
        .map(|value| value.max(0))
        .ok_or(EngineRefusal::CounterOverflow("orb value"))
}

/// Lightning's `ModifyOrbValue` fold adds Infused Core alongside Focus before
/// the shared zero clamp. Other orb kinds never receive this relic modifier.
fn lightning_orb_value(
    state: &HotState,
    catalog: &Catalog,
    base: i64,
) -> Result<i64, EngineRefusal> {
    let infused = i64::from(catalog.hooks().owns(RelicId::RelicInfusedCore));
    if infused != 0 {
        crate::coverage::record_relic(RelicId::RelicInfusedCore);
    }
    base.checked_add(infused)
        .ok_or(EngineRefusal::CounterOverflow("orb value"))
        .and_then(|base| orb_value(state, base))
}

/// `OrbCmd::Channel<LightningOrb>`.
///
/// Python `channel` (frozen, deleted #2827): the only terminal guard is at entry; a
/// zero-base/zero-capacity owner bootstraps one slot; a full queue dequeues
/// and evokes its leftmost member; even a lethal evoke is followed by the
/// enqueue because there is deliberately no second terminal check.
///
/// Voltaic's counter (`lightning_channeled`, tracked only while `>= 0`): IL
/// (v0.111.0) `OrbCmd/<Channel>d__3::MoveNext` (RVA `0x3ed69c`) records
/// `CombatHistory::OrbChanneled` (`IL_021a`) only when `OrbQueue::TryEnqueue`
/// returned true (`IL_019c`, tested at `IL_01ff`), and
/// `OrbQueue/<TryEnqueue>d__13::MoveNext` (RVA `0x3d96fc`) returns false only
/// at zero `Capacity` (`IL_001e`–`IL_0025`), so the count advances after the
/// push and not on the zero-capacity return above. Voltaic counts those
/// history entries whose orb is a `LightningOrb` (see
/// `entry::opening::voltaic_lightning_channeled_seed` for the read).
pub(crate) fn channel(
    state: &mut HotState,
    catalog: &Catalog,
    kind: OrbKind,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // Orb Channel 0x3ed69c checks IsOverOrEnding at entry (IL_002e): while
    // ending (all primaries dead without a veto, even before history.over
    // latches) the channel is a no-op rather than a queued orb (#2669).
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    if state.orbs.base_slots() == 0 && state.orbs.slots() == 0 {
        state.orbs.set_slots(1);
    }
    if state.orbs.as_slice().len() >= usize::from(state.orbs.slots()) {
        evoke_front(state, catalog, true, events)?;
    }
    if state.orbs.slots() == 0 {
        return Ok(());
    }
    let amount = match kind {
        OrbKind::Dark => Some(6i32),
        OrbKind::Glass => Some(4i32),
        OrbKind::Lightning | OrbKind::Frost | OrbKind::Plasma => None,
    };
    let orb = HotOrb::from_parts(kind, amount).expect("native orb defaults are canonical");
    state.orbs.push(orb);
    if kind == OrbKind::Lightning && state.orbs.lightning_channeled() >= 0 {
        let count = state
            .orbs
            .lightning_channeled()
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("lightning_channeled"))?;
        state.orbs.set_lightning_channeled(count);
    }
    if catalog.hooks().owns(RelicId::RelicMetronome) && state.metronome() < 7 {
        let next = state.metronome() + 1;
        let written = state.set_metronome(next);
        debug_assert!(written);
        if next == 7 {
            damage_all(state, catalog, 30, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicMetronome);
    }
    Ok(())
}

/// Grow the live and base capacities together, capped at the native ten.
///
/// Ending gate: `OrbCmd::AddSlots` RVA `0x132f14` returns
/// `Task.CompletedTask` at `CombatManager::get_IsOverOrEnding` (IL_000c-001d)
/// before the `10 - Capacity` clamp (IL_001e-0037) and
/// `OrbQueue::AddCapacity` (IL_0045). The gate is the shared IsOverOrEnding
/// projection [`super::damage::damage_combat_is_ending`], not `history.over`: while every
/// primary is dead and no veto holds, before the over latch, no slot is added.
pub(crate) fn add_slots(state: &mut HotState, amount: i64) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    let amount: u8 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("orb slots"))?;
    state
        .orbs
        .set_slots(state.orbs.slots().saturating_add(amount).min(MAX_ORB_SLOTS));
    Ok(())
}

/// Shrink capacity without evoking; right-edge overflow is silently lost.
///
/// Ending gate: `OrbCmd::RemoveSlots` RVA `0x132f8c` returns at
/// `CombatManager::get_IsOverOrEnding` (IL_000c-0018) before reading
/// `OrbQueue::Capacity` (IL_0024), the same IsOverOrEnding projection
/// [`super::damage::damage_combat_is_ending`] as [`add_slots`].
pub(crate) fn remove_slots(state: &mut HotState, amount: i64) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    let amount: u8 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("orb slots"))?;
    let slots = state.orbs.slots().saturating_sub(amount);
    state.orbs.set_slots(slots);
    while state.orbs.as_slice().len() > usize::from(slots) {
        let last = state.orbs.as_slice().len() - 1;
        state.orbs.remove(last);
    }
    Ok(())
}

/// `OrbCmd::EvokeNext`, retaining or dequeuing the frozen front instance.
///
/// Python `_orb_evoke_at` (frozen, deleted #2827) removes before the effect when
/// `dequeue` is true. Every call owns its own ending gate, and an empty queue
/// is a no-op. AfterOrbEvoked subscribers remain impossible at admission.
///
/// Ending gate (#3112): `OrbCmd/<EvokeNext>d__4::MoveNext` RVA `0x3edd54`
/// returns on an empty queue (IL_0030-0038) and otherwise awaits
/// `OrbCmd::Evoke` (IL_0077); `OrbCmd/<Evoke>d__6::MoveNext` RVA `0x3ed9e0`
/// returns at `CombatManager::get_IsOverOrEnding` (IL_0025-002c) before its
/// own empty check (IL_0048), the `OrbQueue::Remove` dequeue (IL_006c) and
/// `OrbModel::Evoke` (IL_00c9). The gate is therefore the shared
/// IsOverOrEnding projection, not `history.over`: while every primary is dead
/// but the kill drain has not latched over, the orb stays queued and its
/// effect does not resolve.
pub(crate) fn evoke_front(
    state: &mut HotState,
    catalog: &Catalog,
    dequeue: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) || state.orbs.as_slice().is_empty() {
        return Ok(());
    }
    let orb = state.orbs.as_slice()[0];
    if dequeue {
        state
            .orbs
            .remove(0)
            .expect("the front orb was checked immediately before removal");
    }
    evoke_effect(state, catalog, orb, events)
}

/// `OrbCmd::EvokeLast`, removing the frozen rightmost orb before its effect.
///
/// `OrbCmd/<EvokeLast>d__5::MoveNext` RVA `0x3edc18` returns on an empty
/// queue (IL_0030-0038) and otherwise awaits `OrbCmd::Evoke` (IL_0077), whose
/// `IsOverOrEnding` entry gate (`0x3ed9e0` IL_0025) precedes the dequeue; see
/// [`evoke_front`] (#3112).
pub(crate) fn evoke_back(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) || state.orbs.as_slice().is_empty() {
        return Ok(());
    }
    let index = state.orbs.as_slice().len() - 1;
    let orb = state
        .orbs
        .remove(index)
        .expect("the back orb was checked immediately before removal");
    evoke_effect(state, catalog, orb, events)
}

fn evoke_effect(
    state: &mut HotState,
    catalog: &Catalog,
    orb: HotOrb,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    match orb.kind() {
        OrbKind::Lightning => {
            let target = apply_lightning_damage(
                state,
                catalog,
                lightning_orb_value(state, catalog, LIGHTNING_EVOKE)?,
                events,
            )?;
            if let Some(target) = target.filter(|target| state.monsters[*target].hp > 0) {
                let thunder = state.powers.value(PowerId::Thunder);
                if thunder > 0 {
                    damage_monster(
                        state,
                        catalog,
                        target,
                        DotNetDecimal::from_i64(i64::from(thunder)),
                        false,
                        true,
                        events,
                    )?;
                }
            }
        }
        OrbKind::Frost => frost_block(state, catalog, orb_value(state, FROST_EVOKE)?, events)?,
        OrbKind::Dark => {
            let amount = orb.amount().expect("Dark carries an accumulator");
            if let Some(target) = alive_targets(state)
                .into_iter()
                .min_by_key(|target| (state.monsters[*target].hp, *target))
            {
                damage_monster(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(i64::from(amount)),
                    false,
                    true,
                    events,
                )?;
            }
        }
        OrbKind::Plasma => gain_energy(state, PLASMA_EVOKE)?,
        OrbKind::Glass => {
            let amount = orb_value(
                state,
                i64::from(orb.amount().expect("Glass carries a raw passive amount")),
            )?
            .checked_mul(2)
            .ok_or(EngineRefusal::CounterOverflow("glass evoke"))?;
            damage_all(state, catalog, amount, events)?;
        }
    }
    Ok(())
}

/// Run the admitted portion of `OrbQueue::BeforeTurnEnd`.
///
/// The native command snapshots the ordered queue, then triggers each member
/// in order and stops after combat ends. All five orb kinds are admitted, but
/// Plasma has no `BeforeTurnEndOrbTrigger`: its ordinary queue passive belongs
/// exclusively to `AfterTurnStart`, while Loop invokes [`passive_at`] directly.
pub(crate) fn turn_end_passives(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let count = state.orbs.as_slice().len();
    for index in 0..count {
        if state.history.over {
            break;
        }
        if state.orbs.as_slice()[index].kind() == OrbKind::Plasma {
            continue;
        }
        // `OrbQueue/<BeforeTurnEnd>d__16::MoveNext` RVA `0x3d945c` calls
        // `OrbModel::BeforeTurnEndOrbTrigger` (IL_0066), which reaches
        // `OrbModel::TriggerPassive` directly (Lightning `0x351ed4` IL_0025),
        // never `OrbCmd::Passive`, so this walk has no IsOverOrEnding entry
        // gate of its own (#3112).
        passive_at_count(state, catalog, index, true, events)?;
    }
    Ok(())
}

/// Run one orb's ordinary passive at its live queue index. LoopPower calls
/// this on index zero with `countAffectedByHooks=false`; no admitted hook can
/// alter the count, so the same body serves the normal queue walk.
///
/// This is `OrbCmd::Passive` (`LoopPower/<AfterPlayerTurnStart>d__4` RVA
/// `0x33dfb8` IL_008f). `OrbCmd/<Passive>d__7::MoveNext` RVA `0x3ede90`
/// returns at `CombatManager::get_IsOverOrEnding` (IL_0022-0029) before
/// `TriggerPassive` (IL_0059) or `OrbModel::Passive` (IL_00c7), so the entry
/// gate is the shared IsOverOrEnding projection (#3112).
pub(crate) fn passive_at(
    state: &mut HotState,
    catalog: &Catalog,
    index: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    passive_at_count(state, catalog, index, false, events)
}

/// Run one `OrbCmd::Passive` whose trigger count participates in relic
/// modifiers. Gold Plated Cables doubles only the live front orb.
///
/// Emotion Chip issues this command (`EmotionChip/<AfterPlayerTurnStart>d__5`
/// RVA `0x323ae8` IL_0090, `countAffectedByHooks=true`), so it carries the
/// same `OrbCmd/<Passive>d__7` (`0x3ede90` IL_0022) IsOverOrEnding entry gate
/// as [`passive_at`] (#3112). The ordinary end-of-turn queue walk does not
/// come through here; see [`turn_end_passives`].
pub(crate) fn passive_at_affected_by_hooks(
    state: &mut HotState,
    catalog: &Catalog,
    index: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    passive_at_count(state, catalog, index, true, events)
}

/// `OrbModel::TriggerPassive` for one queue index.
///
/// `OrbModel/<TriggerPassive>d__74::MoveNext` RVA `0x31da6c` fixes
/// `triggerCount` once through `Hook::ModifyOrbPassiveTriggerCount`
/// (IL_0045), then awaits `OrbModel::Passive` (IL_00d4) `triggerCount` times
/// (`i < triggerCount`, IL_0212-0230) with no ending or over re-check between
/// triggers (#3218). A Gold Plated Cables second trigger therefore runs even
/// after the first ended combat; only each orb body's own commands gate it:
/// Lightning's `ApplyLightningDamage` (`0x351cc0` IL_0061-0066) returns with
/// no living opponent, Frost's `GainBlock` and Plasma's `GainEnergy` skip at
/// their IsOverOrEnding/IsEnding gates, and `DarkOrb::Passive` (`0xae144`)
/// and Glass's decay (`GlassOrb/<Passive>d__10` `0x351b4c` IL_0081-0095)
/// have none.
fn passive_at_count(
    state: &mut HotState,
    catalog: &Catalog,
    index: usize,
    affected_by_hooks: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let triggers = 1 + usize::from(
        affected_by_hooks && index == 0 && catalog.hooks().owns(RelicId::RelicGoldPlatedCables),
    );
    if triggers == 2 {
        crate::coverage::record_relic(RelicId::RelicGoldPlatedCables);
    }
    for _ in 0..triggers {
        passive_at_once(state, catalog, index, events)?;
    }
    Ok(())
}

fn passive_at_once(
    state: &mut HotState,
    catalog: &Catalog,
    index: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(orb) = state.orbs.as_slice().get(index).copied() else {
        return Ok(());
    };
    match orb.kind() {
        OrbKind::Lightning => {
            apply_lightning_damage(
                state,
                catalog,
                lightning_orb_value(state, catalog, LIGHTNING_PASSIVE)?,
                events,
            )?;
        }
        OrbKind::Frost => {
            frost_block(state, catalog, orb_value(state, FROST_PASSIVE)?, events)?;
        }
        OrbKind::Dark => {
            let raw = i64::from(orb.amount().expect("Dark carries an accumulator"))
                .checked_add(orb_value(state, DARK_PASSIVE)?)
                .ok_or(EngineRefusal::CounterOverflow("dark passive"))?;
            let raw: i32 = raw
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("dark passive"))?;
            state.orbs.replace(
                index,
                HotOrb::from_parts(OrbKind::Dark, Some(raw))
                    .expect("Dark passive remains non-negative"),
            );
        }
        OrbKind::Plasma => gain_energy(state, PLASMA_PASSIVE)?,
        OrbKind::Glass => {
            let raw = orb.amount().expect("Glass carries a raw passive amount");
            let amount = orb_value(state, i64::from(raw))?;
            if amount > 0 {
                state.orbs.replace(
                    index,
                    HotOrb::from_parts(OrbKind::Glass, Some(raw.saturating_sub(1).max(0)))
                        .expect("Glass decay remains non-negative"),
                );
                damage_all(state, catalog, amount, events)?;
            }
        }
    }
    Ok(())
}

/// Plasma's queue-ordered start-of-turn passive.
pub(crate) fn turn_start_passives(
    state: &mut HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    let gold_plated_front = state
        .orbs
        .as_slice()
        .first()
        .is_some_and(|orb| orb.kind() == OrbKind::Plasma)
        && catalog.hooks().owns(RelicId::RelicGoldPlatedCables);
    let count = state
        .orbs
        .as_slice()
        .iter()
        .filter(|orb| orb.kind() == OrbKind::Plasma)
        .count()
        + usize::from(gold_plated_front);
    if gold_plated_front {
        crate::coverage::record_relic(RelicId::RelicGoldPlatedCables);
    }
    let amount = i16::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(PLASMA_PASSIVE))
        .ok_or(EngineRefusal::CounterOverflow("plasma passive"))?;
    if state.fanouts.no_energy_gain() {
        Ok(())
    } else {
        gain_energy(state, amount)
    }
}

/// Return the effective native acquisition order, including the legacy
/// Lightning Rod fallback from Python `begin_player_turn` (frozen, deleted #2827).
pub(crate) fn effective_energy_reset_order(state: &HotState) -> Vec<OrbResetPower> {
    if state.orbs.reset_order().is_empty()
        && state.powers.value(PowerId::LightningRod) > 0
        && state.powers.value(PowerId::Spinner) == 0
    {
        vec![OrbResetPower::LightningRod]
    } else {
        state.orbs.reset_order().to_vec()
    }
}

/// Run Lightning Rod and Spinner in native acquisition order.
pub(crate) fn after_energy_reset(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let order = effective_energy_reset_order(state);
    for power in order {
        if state.history.over {
            break;
        }
        match power {
            OrbResetPower::LightningRod => {
                let amount = state.powers.value(PowerId::LightningRod);
                if amount > 0 {
                    channel(state, catalog, OrbKind::Lightning, events)?;
                    // The entered callback can kill its owner through an
                    // evoke chain. Once owner-death cleanup deactivates
                    // hooks, do not resurrect its cleared power or order
                    // witnesses. Decrement delegates to native ModifyAmount
                    // (0x3f032c), which returns at IsEnding (IL_003a), so an
                    // enemy-terminal evoke retains the duration instead
                    // (#2703); the frozen snapshot still visits later
                    // listeners with their own per-command gates.
                    if state.fanouts.player_hooks_deactivated() {
                        break;
                    }
                    if super::damage::damage_combat_is_ending(state) {
                        continue;
                    }
                    let remaining = amount - 1;
                    state
                        .powers
                        .set(PowerId::LightningRod, SlotWire::Int, remaining);
                    note_power(events, Subject::Player, PowerId::LightningRod, remaining);
                    if remaining == 0 {
                        state
                            .orbs
                            .unregister_reset_power(OrbResetPower::LightningRod);
                        state.fanouts.unregister_after_energy_reset(
                            crate::hot::AfterEnergyResetPower::LightningRod,
                        );
                    }
                }
            }
            OrbResetPower::Spinner => {
                let amount = state.powers.value(PowerId::Spinner);
                for _ in 0..amount.max(0) {
                    if state.history.over {
                        break;
                    }
                    channel(state, catalog, OrbKind::Glass, events)?;
                }
            }
        }
    }
    state.normalize_after_energy_reset_order_if_unique();
    Ok(())
}

/// Tesla Coil's targeted Lightning passive, with no target RNG draw.
///
/// Each call is one `OrbCmd::Passive` (`TeslaCoil/<OnPlay>d__5` RVA
/// `0x3c2518` IL_0142 and, upgraded, IL_01bf), so it returns at the
/// `OrbCmd/<Passive>d__7` IsOverOrEnding entry gate (`0x3ede90` IL_0022)
/// rather than on `history.over` alone (#3112).
///
/// A dead target is a no-op (#3218): `LightningOrb/<ApplyLightningDamage>d__15`
/// RVA `0x351cc0` returns with no draw when no opponent is alive
/// (IL_0061-006e) and otherwise wraps the explicit target without a draw
/// (IL_0074-008c); `CreatureCmd/<Damage>d__12` RVA `0x3e96c8` then skips a
/// dead original target (IL_016d-0172) and dispatches no result.
pub(crate) fn lightning_passive_on(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    if state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::TargetMismatch { required: true })?
        .hp
        <= 0
    {
        return Ok(());
    }
    damage_monster(
        state,
        catalog,
        target,
        DotNetDecimal::from_i64(lightning_orb_value(state, catalog, LIGHTNING_PASSIVE)?),
        false,
        true,
        events,
    )?;
    Ok(())
}

/// Plasma's `PlayerCmd::GainEnergy` boundary.
///
/// `NoEnergyGainPower` is boundary-representable and ledger-authenticated, but
/// independently admission-refused because its only writer and the complete
/// local gain modifier surface are outside this slice. Keeping every Plasma
/// gain on this one seam preserves the native modifier point for the slice
/// that eventually admits that power.
///
/// Every Plasma gain is a `PlayerCmd::GainEnergy` call (`PlasmaOrb/<Evoke>d__10`
/// RVA `0x352224` IL_0049, `PlasmaOrb/<Passive>d__9` RVA `0x352328` IL_0042,
/// reached at turn start through `AfterTurnStartOrbTrigger` `0x352158`
/// IL_0025 `TriggerPassive`), and `PlayerCmd/<GainEnergy>d__3::MoveNext`
/// RVA `0x3ee8a0` returns at `CombatManager.IsEnding` (IL_0035) before any
/// write. The gate is the shared IsEnding projection, not `history.over`
/// (#2708): previously this seam had no gate at all.
fn gain_energy(state: &mut HotState, amount: i16) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    state.energy = state
        .energy
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("orb energy"))?;
    Ok(())
}

/// Frost's owner Block, one `CreatureCmd::GainBlock` per evoke or passive.
///
/// `FrostOrb/<Evoke>d__10::MoveNext` RVA `0x351490` IL_0058 and
/// `FrostOrb/<Passive>d__9::MoveNext` RVA `0x3516e8` IL_0057 each await
/// `CreatureCmd::GainBlock` for the owner. `CreatureCmd/<GainBlock>d__18`
/// RVA `0x3eaec0` returns at `CombatManager::get_IsOverOrEnding`
/// (IL_002d-0041) before `Hook::BeforeBlockGained`, `Hook::ModifyBlock` and
/// the write, so this seam gates on the shared IsOverOrEnding projection
/// [`damage_combat_is_ending`](super::damage::damage_combat_is_ending) (#3218).
/// The OrbCmd entry gates (#3112) already cover evokes and the Loop/Emotion
/// Chip passives, but the end-of-turn queue walk reaches `FrostOrb::Passive`
/// through `BeforeTurnEndOrbTrigger` (`0x3513c4` IL_0025 `TriggerPassive`)
/// with no command gate, and a Gold Plated Cables second trigger runs after
/// the first (see [`passive_at_count`]); both land here. The shared
/// [`gain_flat_block`] keeps its caller-owned gating for its other callers.
fn frost_block(
    state: &mut HotState,
    catalog: &Catalog,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if super::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    gain_flat_block(state, catalog, raw, events)
}

pub(crate) fn gain_flat_block(
    state: &mut HotState,
    catalog: &Catalog,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let amount: i32 = raw
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("frost block"))?;
    if amount > 0 {
        state.block = state
            .block
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("player block"))?;
        events.push(Event::PlayerBlockGained {
            amount,
            block: state.block,
        });
    }
    super::damage::powers_after_block_gained(
        state,
        Some(catalog),
        DotNetDecimal::from_i64(i64::from(amount.max(0))),
        events,
    )?;
    fire_hook(catalog, HookEvent::AfterBlockGained, state, events)
}

/// One flat, unpowered, non-card `GainBlock` retaining its Decimal through
/// the callback boundary. Storage/history conversion remains independently
/// truncating inside the shared damage funnel.
pub(crate) fn gain_flat_decimal_block(
    state: &mut HotState,
    catalog: &Catalog,
    raw: DotNetDecimal,
    events: &mut Vec<Event>,
) -> Result<DotNetDecimal, EngineRefusal> {
    if state.history.over {
        return Ok(DotNetDecimal::zero());
    }
    let modified = super::damage::gain_flat_power_block_decimal(state, Some(catalog), raw, events)?;
    fire_hook(catalog, HookEvent::AfterBlockGained, state, events).map(|()| modified)
}

fn damage_all(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let targets = alive_targets(state);
    super::damage::damage_monsters_after_catalog_auth(
        state,
        catalog,
        &targets,
        DotNetDecimal::from_i64(amount),
        true,
        events,
    )?;
    Ok(())
}

/// Lightning's null-target passive/evoke body.
///
/// `_apply_lightning_damage` (frozen Python, deleted #2827) rebuilds the living roster and
/// consumes exactly one CombatTargets draw for every nonempty pool, including
/// a singleton. Property 4 is blockable and unpowered: no Strength or
/// Vulnerable fold applies.
fn apply_lightning_damage(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    events: &mut Vec<Event>,
) -> Result<Option<usize>, EngineRefusal> {
    let Some(target) = roll_target(state)? else {
        return Ok(None);
    };
    damage_monster(
        state,
        catalog,
        target,
        DotNetDecimal::from_i64(amount),
        false,
        true,
        events,
    )?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogBuilder;
    use crate::engine::turn::end_player_turn;
    use crate::hot::{HotMonster, RngStream, RngStreamState};
    use crate::ids::MonsterKind;
    use crate::powers::SlotWire;

    fn lightning() -> HotOrb {
        HotOrb::from_parts(OrbKind::Lightning, None).unwrap()
    }

    fn plasma() -> HotOrb {
        HotOrb::from_parts(OrbKind::Plasma, None).unwrap()
    }

    fn live_state(hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        state
    }

    fn catalog() -> Catalog {
        CatalogBuilder::new().build()
    }

    #[test]
    fn random_orb_foundation_uses_the_native_pool_order_and_one_draw_each() {
        let mut state = HotState::at_defaults();
        state.rng.set(
            RngStream::CombatOrbs,
            RngStreamState {
                words: [29, 30, 31, 32],
                counter: 0,
            },
        );

        let selected = (0..14)
            .map(|_| select_random_orb(&mut state).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            selected,
            [
                OrbKind::Lightning,
                OrbKind::Lightning,
                OrbKind::Lightning,
                OrbKind::Plasma,
                OrbKind::Frost,
                OrbKind::Frost,
                OrbKind::Lightning,
                OrbKind::Lightning,
                OrbKind::Frost,
                OrbKind::Dark,
                OrbKind::Frost,
                OrbKind::Dark,
                OrbKind::Dark,
                OrbKind::Glass,
            ]
        );
        assert_eq!(
            state.rng.get(RngStream::CombatOrbs),
            RngStreamState {
                words: [
                    3_498_107_376_823_620_609,
                    7_685_726_020_720_238_428,
                    12_355_000_173_393_121_140,
                    6_990_274_728_683_906_004,
                ],
                counter: 14,
            }
        );
        assert!(
            OrbKind::ALL
                .into_iter()
                .all(|kind| selected.contains(&kind))
        );
    }

    #[test]
    fn random_orb_selection_precedes_channels_terminal_gate() {
        let mut state = live_state(20);
        state.history.over = true;
        state.rng.set(
            RngStream::CombatOrbs,
            RngStreamState {
                words: [29, 30, 31, 32],
                counter: 0,
            },
        );
        let before_orbs = state.orbs.clone();
        let mut events = Vec::new();

        let selected = select_random_orb(&mut state).unwrap();
        channel(&mut state, &catalog(), selected, &mut events).unwrap();

        assert_eq!(selected, OrbKind::Lightning);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 1);
        assert_eq!(state.orbs, before_orbs);
        assert!(events.is_empty());
    }

    #[test]
    fn random_loop_publishes_every_auth_and_draw_after_terminal() {
        let mut state = HotState::at_defaults();
        state.history.over = true;
        state.rng.set(
            RngStream::CombatOrbs,
            RngStreamState {
                words: [29, 30, 31, 32],
                counter: 0,
            },
        );

        channel_random_loop(&mut state, &catalog(), 2, &mut Vec::new()).unwrap();

        assert_eq!(state.orbs.next_random_orb_progress_uid(), 2);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 2);
        assert!(state.orbs.as_slice().is_empty());
    }

    #[test]
    fn consuming_shadow_card_channels_fixed_dark_then_applies_once() {
        let mut state = live_state(40);
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);

        consuming_shadow_card(&mut state, &catalog(), 3, &mut Vec::new()).unwrap();

        assert_eq!(state.orbs.next_random_orb_progress_uid(), 3);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 0);
        assert_eq!(state.orbs.as_slice().len(), 3);
        assert!(
            state
                .orbs
                .as_slice()
                .iter()
                .all(|orb| orb.kind() == OrbKind::Dark && orb.amount() == Some(6))
        );
        assert_eq!(state.powers.value(PowerId::ConsumingShadow), 1);
    }

    #[test]
    fn consuming_shadow_terminal_channel_skips_unreachable_power_overflow() {
        let mut state = live_state(6);
        state.orbs.set_base_slots(1);
        state.orbs.set_slots(1);
        state
            .orbs
            .push(HotOrb::from_parts(OrbKind::Dark, Some(7)).unwrap());
        state
            .powers
            .set(PowerId::ConsumingShadow, SlotWire::Int, i32::MAX);
        assert_eq!(
            state.fanouts.register_after_side_turn_end_power(
                crate::hot::AfterSideTurnEndPowerToken::ConsumingShadow,
            ),
            Ok(0)
        );

        consuming_shadow_card(&mut state, &catalog(), 2, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.orbs.next_random_orb_progress_uid(), 2);
        assert_eq!(state.powers.value(PowerId::ConsumingShadow), i32::MAX);
        assert_eq!(state.orbs.as_slice().len(), 1);
        assert_eq!(state.orbs.as_slice()[0].kind(), OrbKind::Dark);
    }

    #[test]
    fn random_loop_allocator_preflight_is_atomic_before_rng_or_orb_work() {
        let mut state = live_state(40);
        state.orbs.set_slots(1);
        state.orbs.set_lifecycle_counters(u32::MAX - 1, 0);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            channel_random_loop(&mut state, &catalog(), 2, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "next_random_orb_progress_uid"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn consuming_shadow_side_end_counts_empty_and_drained_iterations() {
        let mut empty = live_state(40);
        empty.powers.set(PowerId::ConsumingShadow, SlotWire::Int, 2);
        consuming_shadow_side_end(&mut empty, &catalog(), &mut Vec::new()).unwrap();
        assert_eq!(empty.orbs.next_consuming_shadow_side_end_auth_uid(), 1);
        assert_eq!(empty.orbs.next_random_orb_progress_uid(), 0);

        let mut live = live_state(40);
        live.powers.set(PowerId::ConsumingShadow, SlotWire::Int, 3);
        live.orbs.set_base_slots(3);
        live.orbs.set_slots(3);
        live.orbs
            .push(HotOrb::from_parts(OrbKind::Dark, Some(7)).unwrap());
        live.orbs
            .push(HotOrb::from_parts(OrbKind::Frost, None).unwrap());

        consuming_shadow_side_end(&mut live, &catalog(), &mut Vec::new()).unwrap();

        assert_eq!(live.orbs.next_consuming_shadow_side_end_auth_uid(), 1);
        assert_eq!(live.orbs.next_random_orb_progress_uid(), 3);
        assert!(live.orbs.as_slice().is_empty());
        assert_eq!(live.block, 5, "rightmost Frost evokes before left Dark");
        assert_eq!(live.monsters[0].hp, 33);
    }

    #[test]
    fn zero_base_channel_bootstraps_one_slot_without_drawing_rng() {
        let mut state = live_state(20);
        let targets_before = state.rng.get(RngStream::Targets);
        let mut events = Vec::new();

        channel(&mut state, &catalog(), OrbKind::Lightning, &mut events).unwrap();

        assert_eq!(state.orbs.base_slots(), 0);
        assert_eq!(state.orbs.slots(), 1);
        assert_eq!(state.orbs.as_slice(), &[lightning()]);
        assert_eq!(state.rng.get(RngStream::Targets), targets_before);
    }

    /// Voltaic's count (#3003): a tracked (`>= 0`) counter advances on each
    /// enqueued Lightning only; a Frost channel, a zero-capacity channel
    /// (`TryEnqueue` false, no `OrbChanneled` entry) and the untracked `-1`
    /// sentinel all leave it where it was.
    #[test]
    fn a_tracked_lightning_count_advances_only_on_an_enqueued_lightning() {
        let mut events = Vec::new();

        let mut tracked = live_state(20);
        tracked.orbs.set_lightning_channeled(0);
        channel(&mut tracked, &catalog(), OrbKind::Lightning, &mut events).unwrap();
        assert_eq!(tracked.orbs.lightning_channeled(), 1);
        channel(&mut tracked, &catalog(), OrbKind::Frost, &mut events).unwrap();
        assert_eq!(
            tracked.orbs.lightning_channeled(),
            1,
            "Frost is not counted"
        );
        tracked.orbs.set_slots(3);
        channel(&mut tracked, &catalog(), OrbKind::Lightning, &mut events).unwrap();
        assert_eq!(tracked.orbs.lightning_channeled(), 2);

        let mut zero_capacity = live_state(20);
        zero_capacity.orbs.set_base_slots(3);
        zero_capacity.orbs.set_slots(0);
        zero_capacity.orbs.set_lightning_channeled(0);
        channel(
            &mut zero_capacity,
            &catalog(),
            OrbKind::Lightning,
            &mut events,
        )
        .unwrap();
        assert!(zero_capacity.orbs.as_slice().is_empty());
        assert_eq!(
            zero_capacity.orbs.lightning_channeled(),
            0,
            "a refused enqueue records no OrbChanneled entry"
        );

        let mut untracked = live_state(20);
        channel(&mut untracked, &catalog(), OrbKind::Lightning, &mut events).unwrap();
        assert_eq!(untracked.orbs.as_slice(), &[lightning()]);
        assert_eq!(untracked.orbs.lightning_channeled(), -1);
    }

    #[test]
    fn full_queue_evokes_leftmost_before_enqueue_and_draws_once() {
        let mut state = live_state(20);
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);
        let counter_before = state.rng.get(RngStream::Targets).counter;
        let mut events = Vec::new();

        channel(&mut state, &catalog(), OrbKind::Lightning, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 12);
        assert_eq!(state.orbs.as_slice(), &[lightning()]);
        assert_eq!(
            state.rng.get(RngStream::Targets).counter,
            counter_before + 1
        );
    }

    #[test]
    fn lethal_full_queue_evoke_still_finishes_the_channel() {
        let mut state = live_state(8);
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);
        let mut events = Vec::new();

        channel(&mut state, &catalog(), OrbKind::Lightning, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.orbs.as_slice(), &[lightning()]);
    }

    #[test]
    fn retained_then_dequeued_evokes_hit_twice_and_keep_one_identity() {
        let mut state = live_state(30);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);
        let mut events = Vec::new();

        evoke_front(&mut state, &catalog(), false, &mut events).unwrap();
        assert_eq!(state.orbs.as_slice(), &[lightning()]);
        evoke_front(&mut state, &catalog(), true, &mut events).unwrap();

        assert!(state.orbs.as_slice().is_empty());
        assert_eq!(state.monsters[0].hp, 14);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 2);
    }

    #[test]
    fn turn_end_passives_walk_lightning_in_queue_order_until_terminal() {
        let mut state = live_state(3);
        state.orbs.set_slots(2);
        state.orbs.set_orbs(vec![lightning(), lightning()]);
        let mut events = Vec::new();

        turn_end_passives(&mut state, &catalog(), &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
        assert_eq!(state.orbs.as_slice(), &[lightning(), lightning()]);
    }

    #[test]
    fn gold_plated_cables_doubles_only_ordinary_front_passives() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[RelicId::RelicGoldPlatedCables])
            .unwrap();
        let catalog = builder.build();
        let mut state = live_state(30);
        state.orbs.set_slots(2);
        state.orbs.set_orbs(vec![lightning(), lightning()]);

        turn_end_passives(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 21); // front 3 twice, second 3 once

        state.monsters_mut()[0].hp = 30;
        passive_at(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 27); // Loop/Tesla-style direct trigger
    }

    #[test]
    fn direct_plasma_passive_and_ordinary_turn_start_are_distinct_gains() {
        let mut state = live_state(30);
        state.energy = 3;
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![plasma()]);
        let mut events = Vec::new();

        // Loop calls the direct primitive before the queue's ordinary
        // AfterTurnStart walk. Both gains use the shared Plasma energy path.
        passive_at(&mut state, &catalog(), 0, &mut events).unwrap();
        assert_eq!(state.energy, 4);
        turn_start_passives(&mut state, &catalog()).unwrap();
        assert_eq!(state.energy, 5);
    }

    /// #2708: every Plasma gain is `PlayerCmd::GainEnergy` 0x3ee8a0, which
    /// returns at IsEnding (IL_0035). With every primary dead and no veto,
    /// before history.over latches, the evoke, the direct (Loop) passive and
    /// the turn-start passive gain nothing; a veto keeps combat live.
    #[test]
    fn plasma_gains_skip_while_combat_is_ending_before_the_over_latch() {
        let ending = || {
            let mut state = live_state(0);
            state.energy = 3;
            state.orbs.set_slots(1);
            state.orbs.set_orbs(vec![plasma()]);
            assert!(!state.history.over);
            assert!(crate::engine::damage::damage_combat_is_ending(&state));
            state
        };

        let mut evoked = ending();
        evoke_front(&mut evoked, &catalog(), true, &mut Vec::new()).unwrap();
        assert_eq!(evoked.energy, 3, "Plasma evoke gain skips while ending");

        let mut direct = ending();
        passive_at(&mut direct, &catalog(), 0, &mut Vec::new()).unwrap();
        assert_eq!(direct.energy, 3, "direct Plasma passive skips while ending");

        let mut turn_start = ending();
        turn_start_passives(&mut turn_start, &catalog()).unwrap();
        assert_eq!(turn_start.energy, 3, "turn-start Plasma skips while ending");

        // Adaptable vetoes the ending (CombatManager.IsEnding 0x135854):
        // the same dead roster is live combat and every gain lands.
        let vetoed = || {
            let mut state = ending();
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Adaptable, SlotWire::Int, 1);
            assert!(!crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        let mut evoked = vetoed();
        evoke_front(&mut evoked, &catalog(), true, &mut Vec::new()).unwrap();
        assert_eq!(evoked.energy, 3 + PLASMA_EVOKE);
        let mut direct = vetoed();
        passive_at(&mut direct, &catalog(), 0, &mut Vec::new()).unwrap();
        assert_eq!(direct.energy, 3 + PLASMA_PASSIVE);
        let mut turn_start = vetoed();
        turn_start_passives(&mut turn_start, &catalog()).unwrap();
        assert_eq!(turn_start.energy, 3 + PLASMA_PASSIVE);
    }

    /// #3112: `OrbCmd/<Evoke>d__6` (0x3ed9e0 IL_0025) and
    /// `OrbCmd/<Passive>d__7` (0x3ede90 IL_0022) return at IsOverOrEnding
    /// before any dequeue or trigger. With the only primary dead and a live
    /// secondary (Gas Bomb) left to hit, before history.over latches, every
    /// OrbCmd seam is a no-op: EvokeNext retaining and dequeuing, EvokeLast,
    /// the Loop passive, the Emotion Chip passive and Tesla Coil's targeted
    /// passive. An Adaptable veto keeps the same roster live, and each seam
    /// resolves.
    #[test]
    fn lightning_orb_commands_skip_while_combat_is_ending_before_the_over_latch() {
        let ending = || {
            let mut state = live_state(0);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::GasBomb, 10));
            state.orbs.set_slots(2);
            state.orbs.set_orbs(vec![lightning(), lightning()]);
            assert!(!state.history.over);
            assert!(crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        let vetoed = || {
            let mut state = ending();
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Adaptable, SlotWire::Int, 1);
            assert!(!crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        type Seam = fn(&mut HotState) -> Result<(), EngineRefusal>;
        let seams: [(&str, Seam, usize, i32); 6] = [
            (
                "EvokeNext retained",
                |s| evoke_front(s, &catalog(), false, &mut Vec::new()),
                2,
                10 - LIGHTNING_EVOKE as i32,
            ),
            (
                "EvokeNext dequeued",
                |s| evoke_front(s, &catalog(), true, &mut Vec::new()),
                1,
                10 - LIGHTNING_EVOKE as i32,
            ),
            (
                "EvokeLast",
                |s| evoke_back(s, &catalog(), &mut Vec::new()),
                1,
                10 - LIGHTNING_EVOKE as i32,
            ),
            (
                "Loop passive",
                |s| passive_at(s, &catalog(), 0, &mut Vec::new()),
                2,
                10 - LIGHTNING_PASSIVE as i32,
            ),
            (
                "Emotion Chip passive",
                |s| passive_at_affected_by_hooks(s, &catalog(), 0, &mut Vec::new()),
                2,
                10 - LIGHTNING_PASSIVE as i32,
            ),
            (
                "Tesla Coil passive",
                |s| lightning_passive_on(s, &catalog(), 1, &mut Vec::new()),
                2,
                10 - LIGHTNING_PASSIVE as i32,
            ),
        ];
        for (name, seam, live_orbs, live_hp) in seams {
            let mut state = ending();
            let before = state.clone();
            seam(&mut state).unwrap();
            assert_eq!(state, before, "{name} is a no-op while ending");

            let mut state = vetoed();
            seam(&mut state).unwrap();
            assert_eq!(state.orbs.as_slice().len(), live_orbs, "{name} live queue");
            assert_eq!(state.monsters[1].hp, live_hp, "{name} live damage");
        }
    }

    /// #3112: the end-of-turn queue walk reaches `OrbModel::TriggerPassive`
    /// through `BeforeTurnEndOrbTrigger` (Dark 0x351174 IL_0025), never
    /// `OrbCmd::Passive`, and `DarkOrb::Passive` (0xae144) accumulates with
    /// no ending check. So in the ending window the queue walk still grows a
    /// Dark orb while the Loop `OrbCmd::Passive` does not.
    #[test]
    fn turn_end_queue_walk_has_no_orb_command_ending_gate() {
        let dark = HotOrb::from_parts(OrbKind::Dark, Some(6)).unwrap();
        let ending = || {
            let mut state = live_state(0);
            state.orbs.set_slots(1);
            state.orbs.set_orbs(vec![dark]);
            assert!(!state.history.over);
            assert!(crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        let grown = HotOrb::from_parts(OrbKind::Dark, Some(6 + DARK_PASSIVE as i32)).unwrap();

        let mut walked = ending();
        turn_end_passives(&mut walked, &catalog(), &mut Vec::new()).unwrap();
        assert_eq!(walked.orbs.as_slice(), &[grown]);

        let mut looped = ending();
        passive_at(&mut looped, &catalog(), 0, &mut Vec::new()).unwrap();
        assert_eq!(looped.orbs.as_slice(), &[dark]);
    }

    /// #3218: Frost's Block is `CreatureCmd::GainBlock` (`FrostOrb/<Passive>d__9`
    /// 0x3516e8 IL_0057), which returns at IsOverOrEnding (`<GainBlock>d__18`
    /// 0x3eaec0 IL_0032). The end-of-turn queue walk reaches it through
    /// `BeforeTurnEndOrbTrigger` with no OrbCmd gate (#3112), so with the only
    /// primary dead and a live secondary, before history.over latches, the
    /// walk gains no Block; an Adaptable veto keeps combat live and it does.
    #[test]
    fn frost_block_skips_while_combat_is_ending_before_the_over_latch() {
        let frost = HotOrb::from_parts(OrbKind::Frost, None).unwrap();
        let ending = || {
            let mut state = live_state(0);
            state.block = 4;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::GasBomb, 10));
            state.orbs.set_slots(1);
            state.orbs.set_orbs(vec![frost]);
            assert!(!state.history.over);
            assert!(crate::engine::damage::damage_combat_is_ending(&state));
            state
        };

        let mut walked = ending();
        let before = walked.clone();
        let mut events = Vec::new();
        turn_end_passives(&mut walked, &catalog(), &mut events).unwrap();
        assert_eq!(walked, before, "the Frost walk gains nothing while ending");
        assert!(events.is_empty());

        let mut vetoed = ending();
        vetoed.monsters_mut()[0]
            .powers
            .set(PowerId::Adaptable, SlotWire::Int, 1);
        assert!(!crate::engine::damage::damage_combat_is_ending(&vetoed));
        turn_end_passives(&mut vetoed, &catalog(), &mut Vec::new()).unwrap();
        assert_eq!(vetoed.block, 4 + FROST_PASSIVE as i32);
    }

    /// #3218: `OrbModel/<TriggerPassive>d__74` (0x31da6c) loops
    /// `triggerCount` times (IL_0212-0230) with no ending re-check, so a Gold
    /// Plated Cables second trigger runs after the first ended combat. A
    /// front Glass orb whose first passive kills the last enemy decays again
    /// on the second (`GlassOrb/<Passive>d__10` 0x351b4c decays whenever its
    /// value is positive, IL_0056-0095); its damage then finds no target.
    #[test]
    fn gold_plated_cables_second_trigger_runs_after_the_first_ends_combat() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[RelicId::RelicGoldPlatedCables])
            .unwrap();
        let catalog = builder.build();
        let glass = |raw| HotOrb::from_parts(OrbKind::Glass, Some(raw)).unwrap();

        let mut state = live_state(3);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![glass(4)]);
        turn_end_passives(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert!(state.history.over, "the first Glass passive is lethal");
        assert_eq!(
            state.orbs.as_slice(),
            &[glass(2)],
            "the second trigger still decays Glass after combat ended"
        );

        // Live control: both triggers hit and decay.
        let mut live = live_state(30);
        live.orbs.set_slots(1);
        live.orbs.set_orbs(vec![glass(4)]);
        turn_end_passives(&mut live, &catalog, &mut Vec::new()).unwrap();
        assert!(!live.history.over);
        assert_eq!(live.monsters[0].hp, 30 - 4 - 3);
        assert_eq!(live.orbs.as_slice(), &[glass(2)]);
    }

    #[test]
    fn plasma_has_no_player_turn_end_passive_without_loop() {
        let mut state = live_state(30);
        state.energy = 1;
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![plasma()]);
        let mut events = Vec::new();

        turn_end_passives(&mut state, &catalog(), &mut events).unwrap();

        assert_eq!(state.energy, 1);
        assert_eq!(state.orbs.as_slice(), &[plasma()]);
    }

    #[test]
    fn fusion_plus_then_loop_reaches_five_energy_on_the_next_turn() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = live_state(100);
        state.energy = 1;
        state.turn = 1;
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![plasma()]);
        state.powers.set(PowerId::Loop, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_player_turn_start_order(&[PowerId::Loop])
        );
        let mut events = Vec::new();

        // This is the exact post-play state of Fusion+ uid 0 followed by Loop
        // uid 1: reset 3 + Loop's direct Plasma passive 1 + the queue's
        // ordinary start-of-turn Plasma passive 1.
        end_player_turn(&mut state, &catalog, &mut events).unwrap();

        assert!(!state.history.over);
        assert_eq!(state.turn, 2);
        assert_eq!(state.energy, 5);
    }

    #[test]
    fn lethal_monster_phase_cannot_gain_plasma_energy_at_turn_end() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = live_state(30);
        state.hp = 1;
        state.energy = 1;
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![plasma()]);
        let mut events = Vec::new();

        end_player_turn(&mut state, &catalog, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.energy, 1);
    }

    #[test]
    fn every_lightning_read_folds_permanent_and_temporary_focus_then_clamps() {
        let mut state = live_state(30);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);
        state.powers.set(PowerId::Focus, SlotWire::Int, 4);
        state.orbs.set_temp_focus(2);
        let mut events = Vec::new();

        turn_end_passives(&mut state, &catalog(), &mut events).unwrap();
        assert_eq!(state.monsters[0].hp, 21);

        state.powers.set(PowerId::Focus, SlotWire::Int, -20);
        state.orbs.set_temp_focus(5);
        let counter_before = state.rng.get(RngStream::Targets).counter;
        turn_end_passives(&mut state, &catalog(), &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 21);
        assert_eq!(
            state.rng.get(RngStream::Targets).counter,
            counter_before + 1
        );
    }

    #[test]
    fn infused_core_adds_one_only_to_lightning_reads() {
        let mut builder = CatalogBuilder::new();
        builder.set_relics(&[RelicId::RelicInfusedCore]).unwrap();
        let catalog = builder.build();
        let mut state = live_state(30);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);

        turn_end_passives(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 26);

        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Frost, None).unwrap()]);
        turn_end_passives(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.block, 2);
    }

    #[test]
    fn non_lightning_passives_preserve_native_value_and_block_rules() {
        let mut state = live_state(50);
        state.energy = 2;
        state.orbs.set_base_slots(4);
        state.orbs.set_slots(4);
        state.orbs.set_orbs(vec![
            HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
            plasma(),
            HotOrb::from_parts(OrbKind::Dark, Some(6)).unwrap(),
            HotOrb::from_parts(OrbKind::Glass, Some(4)).unwrap(),
        ]);
        state.powers.set(PowerId::Focus, SlotWire::Int, 2);
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 99);
        let mut events = Vec::new();

        turn_end_passives(&mut state, &catalog(), &mut events).unwrap();

        assert_eq!(state.block, 4, "Frost ignores Dexterity");
        assert_eq!(state.history.card_block_gains, 0, "Frost is flat block");
        assert_eq!(state.energy, 2, "Plasma is skipped, not triggered");
        assert_eq!(state.orbs.as_slice()[2].amount(), Some(14));
        assert_eq!(state.orbs.as_slice()[3].amount(), Some(3));
        assert_eq!(state.monsters[0].hp, 44, "Glass snapshots 4 + 2 Focus");
    }

    #[test]
    fn every_evoke_kind_uses_its_distinct_target_resource_and_focus_contract() {
        let mut state = live_state(50);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.orbs.set_slots(4);
        state.powers.set(PowerId::Focus, SlotWire::Int, 2);
        let mut events = Vec::new();

        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Frost, None).unwrap()]);
        evoke_front(&mut state, &catalog(), true, &mut events).unwrap();
        assert_eq!(state.block, 7);

        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Plasma, None).unwrap()]);
        let energy = state.energy;
        evoke_front(&mut state, &catalog(), true, &mut events).unwrap();
        assert_eq!(state.energy, energy + 2, "Plasma bypasses Focus");

        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Dark, Some(13)).unwrap()]);
        evoke_front(&mut state, &catalog(), true, &mut events).unwrap();
        assert_eq!(
            state.monsters[1].hp, 7,
            "Dark hits the first minimum by raw value"
        );

        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Glass, Some(4)).unwrap()]);
        evoke_front(&mut state, &catalog(), true, &mut events).unwrap();
        assert_eq!(state.monsters[0].hp, 38);
        assert!(state.monsters[1].hp <= 0);
    }

    /// `OrbCmd::AddSlots` (0x132f14 IL_0011) and `OrbCmd::RemoveSlots`
    /// (0x132f8c IL_0011) return at IsOverOrEnding. With the only primary
    /// dead and no veto, before history.over latches, neither capacity moves
    /// and no orb is dropped; an Adaptable veto keeps the same roster live
    /// and both commands resolve.
    #[test]
    fn slot_commands_skip_while_combat_is_ending_before_the_over_latch() {
        let at = |hp: i32, veto: bool| {
            let mut state = live_state(hp);
            state.orbs.set_base_slots(3);
            state.orbs.set_slots(3);
            state.orbs.set_orbs(vec![
                HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
                HotOrb::from_parts(OrbKind::Dark, Some(6)).unwrap(),
                HotOrb::from_parts(OrbKind::Plasma, None).unwrap(),
            ]);
            if veto {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Adaptable, SlotWire::Int, 1);
            }
            assert!(!state.history.over);
            state
        };

        let mut ending = at(0, false);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        add_slots(&mut ending, 2).unwrap();
        assert_eq!(ending.orbs.slots(), 3, "AddSlots skips while ending");
        remove_slots(&mut ending, 1).unwrap();
        assert_eq!(ending.orbs.slots(), 3, "RemoveSlots skips while ending");
        assert_eq!(ending.orbs.as_slice().len(), 3);

        for mut live in [at(0, true), at(10, false)] {
            assert!(!crate::engine::damage::damage_combat_is_ending(&live));
            add_slots(&mut live, 2).unwrap();
            assert_eq!(live.orbs.slots(), 5);
            remove_slots(&mut live, 3).unwrap();
            assert_eq!(live.orbs.slots(), 2);
            assert_eq!(live.orbs.as_slice().len(), 2);
        }
    }

    #[test]
    fn slot_changes_and_reset_power_order_are_queue_exact() {
        let mut state = live_state(100);
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.orbs.set_orbs(vec![
            HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
            HotOrb::from_parts(OrbKind::Dark, Some(6)).unwrap(),
            HotOrb::from_parts(OrbKind::Plasma, None).unwrap(),
        ]);
        remove_slots(&mut state, 1).unwrap();
        assert_eq!(state.orbs.base_slots(), 3);
        assert_eq!(state.orbs.slots(), 2);
        assert_eq!(state.orbs.as_slice().len(), 2, "rightmost orb is dropped");
        add_slots(&mut state, 20).unwrap();
        assert_eq!(state.orbs.slots(), MAX_ORB_SLOTS);

        state.powers.set(PowerId::Spinner, SlotWire::Int, 1);
        state.powers.set(PowerId::LightningRod, SlotWire::Int, 2);
        state
            .orbs
            .set_reset_order(vec![OrbResetPower::Spinner, OrbResetPower::LightningRod]);
        let mut events = Vec::new();
        after_energy_reset(&mut state, &catalog(), &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::LightningRod), 1);
        assert_eq!(state.orbs.as_slice()[2].kind(), OrbKind::Glass);
        assert_eq!(state.orbs.as_slice()[3].kind(), OrbKind::Lightning);
    }

    #[test]
    fn full_queue_terminal_evoke_retains_lightning_rod_duration() {
        // #2703: native `Decrement` delegates to `ModifyAmount`, which
        // returns at IsEnding — an enemy-terminal evoke must not decrement
        // the rod even though hooks stay active.
        let mut state = live_state(5);
        state.orbs.set_base_slots(1);
        state.orbs.set_slots(1);
        state.orbs.set_orbs(vec![lightning()]);
        state.powers.set(PowerId::LightningRod, SlotWire::Int, 2);
        state
            .orbs
            .set_reset_order(vec![OrbResetPower::LightningRod]);
        let mut events = Vec::new();
        after_energy_reset(&mut state, &catalog(), &mut events).unwrap();
        assert!(state.history.over, "the evoked Lightning ends combat");
        assert_eq!(
            state.powers.value(PowerId::LightningRod),
            2,
            "the ending walk retains the duration"
        );
        assert_eq!(state.orbs.as_slice().len(), 1);
        assert_eq!(state.orbs.as_slice()[0].kind(), OrbKind::Lightning);
    }

    #[test]
    fn metronome_fires_once_on_the_seventh_channel() {
        let mut builder = CatalogBuilder::new();
        builder
            .set_relics(&[crate::ids::RelicId::RelicMetronome])
            .unwrap();
        let catalog = builder.build();
        let mut state = live_state(100);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.orbs.set_slots(10);

        for _ in 0..7 {
            channel(&mut state, &catalog, OrbKind::Frost, &mut Vec::new()).unwrap();
        }
        assert_eq!(state.metronome(), 7);
        assert_eq!(
            state.monsters.iter().map(|monster| monster.hp).sum::<i32>(),
            140
        );

        channel(&mut state, &catalog, OrbKind::Frost, &mut Vec::new()).unwrap();
        assert_eq!(state.metronome(), 7);
        assert_eq!(
            state.monsters.iter().map(|monster| monster.hp).sum::<i32>(),
            140
        );
    }
}
