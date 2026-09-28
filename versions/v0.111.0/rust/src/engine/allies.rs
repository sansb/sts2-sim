//! Exact same-side Player routing for Unit-C cards and Hammer Time's remote
//! Forge suffix.
//!
//! The hot quotient represents the local owner and at most one remote
//! `PlayerCreature`. A represented local Osty is deliberately not a Player
//! and admission refuses it for every manual `AnyAlly` program.

use std::sync::Arc;

use crate::catalog::{CardIdentity, CardSpec, Catalog};
use crate::engine::damage::{gain_powered_card_block, preview_powered_card_block};
use crate::engine::draw::{DrawSource, draw_cards};
use crate::engine::{EngineRefusal, Event, StepCtx};
use crate::hot::{
    HotCard, HotState, ImitationClone, MultiplayerAllyCard, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, PowerId};
use crate::rng::Xoshiro256StarStar;

const PLAYER_RESOURCE_CAP: i32 = 999_999_999;

#[cfg(test)]
thread_local! {
    static IMITATION_CLONE_PAYLOAD_TRACE: std::cell::RefCell<
        Vec<(u16, crate::hot::CardInstanceState)>
    > = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn target_key(ctx: &StepCtx<'_>) -> Result<u32, EngineRefusal> {
    let key: u32 = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("AnyAlly player key"))?;
    let live = match key {
        0 => ctx.state.hp > 0,
        1 => ctx.state.multiplayer_ally_key == 1 && ctx.state.fanouts.multiplayer_ally().alive,
        _ => false,
    };
    if !live {
        return Err(EngineRefusal::BadTarget(key.try_into().unwrap_or(u8::MAX)));
    }
    Ok(key)
}

/// `PlayerCmd::GainEnergy` for one typed Player key.
///
/// Current v0.111.0 IL (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `PlayerCmd/<GainEnergy>d__3::MoveNext` RVA `0x3ee8a0` returns for `amount <= 0` (IL_0019-002b) and then for
/// `CombatManager.IsEnding` (IL_0030-003c) before `Hook.ModifyEnergyGain`
/// and the write. The gate is the shared IsEnding projection
/// [`damage_combat_is_ending`](crate::engine::damage::damage_combat_is_ending),
/// not `history.over`: with every primary dead and no veto the gain is a no-op
/// even before the over latch (#2708, following #2669). Native callers:
/// `BelieveInYou/<OnPlay>d__7` `0x38c32c` IL_0057, `BigBang/<OnPlay>d__7`
/// `0x38c648` IL_01b4, `Constellation/<OnPlay>d__11` `0x394148` IL_01e9,
/// `EnergySurge/<OnPlay>d__9` `0x39b948` IL_0118 — each a plain
/// `PlayerCmd::GainEnergy` call.
pub(crate) fn gain_energy(
    state: &mut HotState,
    key: u32,
    amount: i32,
) -> Result<(), EngineRefusal> {
    if amount == 0 || crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("ally energy"));
    }
    match key {
        0 => {
            state.energy = state
                .energy
                .checked_add(
                    amount
                        .try_into()
                        .map_err(|_| EngineRefusal::CounterOverflow("energy"))?,
                )
                .filter(|energy| i32::from(*energy) <= PLAYER_RESOURCE_CAP)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
        }
        1 => {
            let ally = state.fanouts.multiplayer_ally_mut();
            if !ally.no_energy_gain {
                ally.energy = i64::from(ally.energy)
                    .checked_add(i64::from(amount))
                    .ok_or(EngineRefusal::CounterOverflow("remote energy"))?
                    .min(i64::from(PLAYER_RESOURCE_CAP))
                    .try_into()
                    .expect("native remote Energy cap fits i32");
            }
        }
        _ => return Err(EngineRefusal::MalformedArgs("ally energy key")),
    }
    Ok(())
}

pub(crate) fn gain_temp_strength(
    state: &mut HotState,
    key: u32,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    gain_temporary_ally_stat(state, key, amount, events, TemporaryAllyStat::Strength)
}

#[derive(Clone, Copy)]
enum TemporaryAllyStat {
    Strength,
    Dexterity,
}

#[inline(never)]
fn gain_temporary_ally_stat(
    state: &mut HotState,
    key: u32,
    amount: i32,
    events: &mut Vec<Event>,
    stat: TemporaryAllyStat,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        let site = match stat {
            TemporaryAllyStat::Strength => "temporary ally strength",
            TemporaryAllyStat::Dexterity => "temporary ally dexterity",
        };
        return Err(EngineRefusal::MalformedArgs(site));
    }
    match key {
        0 => match stat {
            TemporaryAllyStat::Strength => {
                super::damage::apply_owner_temporary_strength(state, amount, events)?;
            }
            TemporaryAllyStat::Dexterity => {
                let current = state.powers.value(PowerId::TempDexterity);
                let updated = current
                    .checked_add(amount)
                    .ok_or(EngineRefusal::CounterOverflow("temporary player dexterity"))?;
                super::turn::prepare_after_side_turn_end_scalar_write(
                    state,
                    PowerId::TempDexterity,
                    current,
                    updated,
                )?;
                state.powers.set(
                    PowerId::TempDexterity,
                    crate::powers::SlotWire::Int,
                    updated,
                );
                crate::engine::damage::note_power(
                    events,
                    crate::engine::Subject::Player,
                    PowerId::TempDexterity,
                    updated,
                );
            }
        },
        1 => {
            let ally = state.fanouts.multiplayer_ally();
            let current = match stat {
                TemporaryAllyStat::Strength => ally.temp_strength,
                TemporaryAllyStat::Dexterity => ally.temp_dexterity,
            };
            let Some(updated) = current.checked_add(amount) else {
                let site = match stat {
                    TemporaryAllyStat::Strength => "remote temporary strength",
                    TemporaryAllyStat::Dexterity => "remote temporary dexterity",
                };
                return Err(EngineRefusal::CounterOverflow(site));
            };
            let ally = state.fanouts.multiplayer_ally_mut();
            match stat {
                TemporaryAllyStat::Strength => ally.temp_strength = updated,
                TemporaryAllyStat::Dexterity => ally.temp_dexterity = updated,
            }
        }
        _ => {
            let site = match stat {
                TemporaryAllyStat::Strength => "temporary ally strength key",
                TemporaryAllyStat::Dexterity => "temporary ally dexterity key",
            };
            return Err(EngineRefusal::MalformedArgs(site));
        }
    }
    Ok(())
}

/// Apply the exact selected-Player TemporaryDexterityPower quotient.
///
/// Local ownership reuses the existing positive TempDexterity power slot;
/// the represented remote Player has no admitted application listeners and
/// therefore needs only the wrapper's live amount until its side-end expiry.
pub(crate) fn gain_temp_dexterity(
    state: &mut HotState,
    key: u32,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    gain_temporary_ally_stat(state, key, amount, events, TemporaryAllyStat::Dexterity)
}

/// Apply Blaze's selected-player permanent `StrengthPower` command.
///
/// The local branch reuses the complete represented owner writer after
/// authenticating the frozen Type-1 listener order. The remote quotient has
/// no admitted power-application hooks, so its permanent scalar is the whole
/// observable mutation. Callers rehearse this checked write before publishing
/// the real command.
pub(crate) fn gain_permanent_strength(
    state: &mut HotState,
    key: u32,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("permanent ally strength"));
    }
    match key {
        0 => {
            if !crate::engine::damage::player_type_one_listener_order_is_exact(state) {
                return Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order",
                ));
            }
            crate::engine::damage::apply_owner_strength(state, amount, events)
        }
        1 => {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.strength = ally
                .strength
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("remote permanent strength"))?;
            Ok(())
        }
        _ => Err(EngineRefusal::MalformedArgs("permanent ally strength key")),
    }
}

pub(crate) fn selected_block(state: &HotState, key: u32) -> Result<i32, EngineRefusal> {
    match key {
        0 => Ok(state.block),
        1 => Ok(state.fanouts.multiplayer_ally().block),
        _ => Err(EngineRefusal::MalformedArgs("ally block key")),
    }
}

/// One powered card `CreatureCmd::GainBlock` to a typed Player key.
///
/// Current v0.111.0 IL (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `CreatureCmd/<GainBlock>d__18::MoveNext` RVA `0x3eaec0` returns at
/// `CombatManager::get_IsOverOrEnding` (IL_002d-0041) before the dead-creature
/// check (IL_004c), `Hook::BeforeBlockGained` (IL_009b), `Hook::ModifyBlock`
/// (IL_0134) and any Block write; the `BlockVar` overload
/// `<GainBlock>d__17` RVA `0x3eadd8` forwards to it (IL_003e). The entry gate
/// is therefore the shared IsOverOrEnding projection
/// [`damage_combat_is_ending`](crate::engine::damage::damage_combat_is_ending),
/// not `history.over`: with every primary dead and no veto the gain is a
/// no-op even before the over latch (#3218, following #3112's Constellation
/// fix). Every caller is a plain card-level `CreatureCmd::GainBlock` with no
/// card-level ending check of its own:
/// `Lift/<OnPlay>d__7` `0x3a9bd8` IL_004f,
/// `Mimic/<OnPlay>d__9` `0x3aca04` IL_0072,
/// `Rally/<OnPlay>d__7` `0x3b55e4` IL_0089 (once per living teammate),
/// `DemonicShield/<OnPlay>d__9` `0x3989f0` IL_0113 (after its `Damage`
/// IL_007b), and `Constellation/<OnPlay>d__11` `0x394148` IL_0263.
#[inline(never)]
pub(crate) fn gain_powered_block(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    key: u32,
    source: &CardSpec,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    match key {
        0 => {
            gain_powered_card_block(state, catalog, source, raw, events)?;
        }
        1 => {
            // Native DexterityPower::ModifyBlockAdditive RVA 0xa17b8 gates
            // on CardPlay.Owner, not the Block recipient. Reuse the pure
            // owner CardPlay modifier fold, then store only the selected
            // teammate's capped Block. This intentionally neither reads the
            // recipient's Dexterity nor advances the owner's block history.
            let current = state.fanouts.multiplayer_ally().block;
            if !(0..=PLAYER_RESOURCE_CAP).contains(&current) {
                return Err(EngineRefusal::MalformedArgs("remote block"));
            }
            let gained = preview_powered_card_block(state, source, raw)?;
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.block = i64::from(current)
                .checked_add(i64::from(gained))
                .ok_or(EngineRefusal::CounterOverflow("remote block"))?
                .min(i64::from(PLAYER_RESOURCE_CAP))
                .try_into()
                .expect("native remote Block cap fits i32");
        }
        _ => return Err(EngineRefusal::MalformedArgs("ally block key")),
    }
    Ok(())
}

pub(crate) fn add_draw_next_turn(
    state: &mut HotState,
    key: u32,
    amount: i32,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("ally draw next turn"));
    }
    match key {
        0 => {
            let old = state.powers.value(PowerId::DrawNextTurn);
            let updated = old
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("draw next turn"))?;
            if old <= 0
                && !state
                    .fanouts
                    .register_after_side_turn_start(PowerId::DrawNextTurn)
            {
                return Err(EngineRefusal::CounterOverflow(
                    "after-side-turn-start listener order",
                ));
            }
            state
                .powers
                .set(PowerId::DrawNextTurn, crate::powers::SlotWire::Int, updated);
        }
        1 => {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.draw_next_turn = ally
                .draw_next_turn
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("remote draw next turn"))?;
        }
        _ => return Err(EngineRefusal::MalformedArgs("ally draw next turn key")),
    }
    Ok(())
}

/// Apply one serial `PowerCmd.Apply<OneForAllPower>` recipient from the
/// already-frozen living Player roster. The caller rehearses the complete
/// fan-out before publishing its first local power event, so a later remote
/// overflow cannot leak an earlier owner mutation.
pub(crate) fn add_one_for_all(
    state: &mut HotState,
    key: u32,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("One For All amount"));
    }
    match key {
        0 => {
            let updated = state
                .powers
                .value(PowerId::OneForAll)
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("one_for_all"))?;
            state
                .powers
                .set(PowerId::OneForAll, crate::powers::SlotWire::Int, updated);
            crate::engine::damage::note_power(
                events,
                crate::engine::Subject::Player,
                PowerId::OneForAll,
                updated,
            );
        }
        1 => {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.one_for_all = ally
                .one_for_all
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("remote one_for_all"))?;
        }
        _ => return Err(EngineRefusal::MalformedArgs("One For All player key")),
    }
    Ok(())
}

pub(crate) fn set_intercept_covered(state: &mut HotState, key: u32) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    if key >= 2 {
        return Err(EngineRefusal::MalformedArgs("Intercept target"));
    }
    if !state.fanouts.cover_with_intercept(key) {
        return Err(EngineRefusal::MalformedArgs("Intercept covered set"));
    }
    Ok(())
}

pub(crate) fn living_keys(state: &HotState) -> impl Iterator<Item = u32> + '_ {
    (state.hp > 0).then_some(0).into_iter().chain(
        (state.multiplayer_ally_key == 1 && state.fanouts.multiplayer_ally().alive).then_some(1),
    )
}

/// Native AutoPlay's AnyAlly pool excludes the local owner and every pet.
/// The represented boundary has at most one live remote Player, but native
/// `NextItem` still consumes one CombatTargets draw for that singleton.
pub(crate) fn roll_auto_target_key(state: &mut HotState) -> Result<Option<u8>, EngineRefusal> {
    if state.multiplayer_ally_key != 1 || !state.fanouts.multiplayer_ally().alive {
        return Ok(None);
    }
    let live = state.rng.get(RngStream::Targets);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let selected = rng
        .next_bounded(1)
        .map_err(|_| EngineRefusal::CounterOverflow("AutoPlay AnyAlly pool"))?;
    debug_assert_eq!(selected, 0);
    state.rng.set(
        RngStream::Targets,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(Some(1))
}

pub(crate) fn draw_for_player(
    state: &mut HotState,
    catalog: &Catalog,
    key: u32,
    amount: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    match key {
        0 => draw_cards(state, catalog, amount, DrawSource::Command, events),
        1 => remote_draw(state, amount),
        _ => Err(EngineRefusal::MalformedArgs("ally draw key")),
    }
}

fn remote_draw(state: &mut HotState, amount: usize) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let ally = state.fanouts.multiplayer_ally_mut();
    let mut draw = ally.draw.as_ref().clone();
    let mut hand = ally.hand.as_ref().clone();
    let mut discard = ally.discard.as_ref().clone();
    let mut shuffle_rng = ally.shuffle_rng;
    for _ in 0..amount {
        if state.history.over || hand.len() >= 10 {
            break;
        }
        if draw.is_empty() {
            if discard.is_empty() {
                break;
            }
            discard.sort_by_key(|card| (card.identity.id as u16, card.identity.upgrade));
            let live =
                shuffle_rng.ok_or(EngineRefusal::MalformedArgs("remote Draw reshuffle stream"))?;
            let mut rng = Xoshiro256StarStar {
                words: live.words,
                counter: live.counter,
            };
            rng.shuffle(&mut discard)
                .map_err(|_| EngineRefusal::CounterOverflow("remote shuffle bound"))?;
            shuffle_rng = Some(RngStreamState {
                words: rng.words,
                counter: rng.counter,
            });
            draw = std::mem::take(&mut discard);
        }
        hand.push(draw.remove(0));
    }
    ally.draw = Arc::new(draw);
    ally.hand = Arc::new(hand);
    ally.discard = Arc::new(discard);
    ally.shuffle_rng = shuffle_rng;
    Ok(())
}

/// Move Tutor's choice-free remote Draw singleton to that same Player's Hand,
/// or to Discard when the native ten-card Hand cap is already full.
pub(crate) fn tutor_remote_fetch(state: &mut HotState, key: u32) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    if key != 1
        || state.multiplayer_ally_key != 1
        || state.fanouts.multiplayer_ally().key != 1
        || !state.fanouts.multiplayer_ally().alive
    {
        return Err(EngineRefusal::MalformedArgs("Tutor remote Player"));
    }
    let ally = state.fanouts.multiplayer_ally_mut();
    if ally.draw.len() > 1 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let Some(selected) = ally.draw.first().copied() else {
        return Ok(());
    };
    let mut draw = ally.draw.as_ref().clone();
    draw.clear();
    ally.draw = Arc::new(draw);
    if ally.hand.len() >= 10 {
        Arc::make_mut(&mut ally.discard).push(selected);
    } else {
        Arc::make_mut(&mut ally.hand).push(selected);
    }
    Ok(())
}

pub(crate) fn blade_symphony(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let mut probe = state.clone();
    blade_symphony_inner(&mut probe, catalog, &mut Vec::new())?;
    blade_symphony_inner(state, catalog, events)
}

/// Apply one Hammer Time nested Forge to the exact represented remote Player.
///
/// The quotient deliberately has no remote Exhaust or Play storage: the
/// canonical boundary rejects either field. Current admitted ally-card bodies
/// can only preserve or move these payload cards among Draw, Hand, and
/// Discard, so this three-pile walk is the complete reachable native five-pile
/// query rather than an approximation. Each pile retains physical order.
pub(crate) fn forge_remote_player_exact(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount < 0
        || state.multiplayer_ally_key != 1
        || state.fanouts.multiplayer_ally().key != 1
        || !state.fanouts.multiplayer_ally().alive
    {
        return Err(EngineRefusal::MalformedArgs(
            "Hammer Time remote Forge recipient",
        ));
    }
    if state.history.over {
        return Ok(());
    }
    let blade_identity = CardIdentity {
        id: CardId::SovereignBlade,
        upgrade: 0,
        enchantment: None,
    };
    let blade_spec = catalog
        .atom(&blade_identity)
        .and_then(|atom| catalog.spec(atom))
        .filter(|spec| {
            spec.identity == blade_identity
                && crate::content_tables::card_row(CardId::SovereignBlade, 0) == Some(spec.row)
        })
        .ok_or(EngineRefusal::UnknownMintIdentity(blade_identity))?;
    let has_live = {
        let ally = state.fanouts.multiplayer_ally();
        ally.hand
            .iter()
            .chain(ally.draw.iter())
            .chain(ally.discard.iter())
            .any(|card| matches!(card.identity.id, CardId::SovereignBlade))
    };
    if !has_live {
        crate::engine::cards::begin_remote_generated_card(state)?;
        {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.owner_generated_cards_combat =
                ally.owner_generated_cards_combat.checked_add(1).ok_or(
                    EngineRefusal::CounterOverflow("remote owner generated cards combat"),
                )?;
            let generated = MultiplayerAllyCard::sovereign_blade(blade_identity, 10)
                .expect("canonical Sovereign Blade state is exact");
            if ally.hand.len() < 10 {
                Arc::make_mut(&mut ally.hand).push(generated);
            } else {
                Arc::make_mut(&mut ally.discard).push(generated);
            }
        }
        crate::engine::cards::finish_remote_generated_card(
            state,
            catalog,
            blade_spec.identity,
            events,
        )?;
    }

    // ForgeCmd re-reads the physical recipient after the awaited generated
    // hook. For a remote creator all represented local power/relic/card
    // listeners are relation-gated no-ops; reborrow nonetheless preserves
    // that native transaction boundary and future-proofs the ordering.
    let ally = state.fanouts.multiplayer_ally_mut();
    for pile in [&mut ally.hand, &mut ally.draw, &mut ally.discard] {
        for card in Arc::make_mut(pile) {
            if matches!(card.identity.id, CardId::SovereignBlade)
                && (card.sovereign_blade_damage().is_none()
                    || card.add_forge_damage(amount).is_none())
            {
                return Err(EngineRefusal::CounterOverflow(
                    "remote sovereign blade damage",
                ));
            }
        }
    }
    Ok(())
}

fn blade_symphony_inner(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let recipients = living_keys(state).collect::<Vec<_>>();
    for key in recipients {
        if state.history.over {
            break;
        }
        if key == 0 {
            crate::engine::cards::inject_generated_shivs_then_upgrade(
                state, catalog, 2, 0, events,
            )?;
        } else {
            let shiv = CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            };
            for _ in 0..2 {
                crate::engine::cards::begin_remote_generated_card(state)?;
                {
                    let ally = state.fanouts.multiplayer_ally_mut();
                    ally.owner_generated_cards_combat =
                        ally.owner_generated_cards_combat.checked_add(1).ok_or(
                            EngineRefusal::CounterOverflow("remote owner generated cards combat"),
                        )?;
                    if ally.hand.len() < 10 {
                        Arc::make_mut(&mut ally.hand).push(
                            MultiplayerAllyCard::immutable(shiv)
                                .expect("canonical Shiv is immutable"),
                        );
                    } else {
                        Arc::make_mut(&mut ally.discard).push(
                            MultiplayerAllyCard::immutable(shiv)
                                .expect("canonical Shiv is immutable"),
                        );
                    }
                }
                crate::engine::cards::finish_remote_generated_card(state, catalog, shiv, events)?;
            }
        }
    }
    Ok(())
}

/// Create Glimpse Beyond's serial, per-Player random-Draw Soul batches.
///
/// The canonical boundary authenticates exactly one same-side creation-order
/// quotient: local owner key `0`, followed by optional remote Player key `1`.
/// Reordered and multi-remote `multiplayer_player_order` rows are rejected by
/// `HotBoundary`, so [`living_keys`] is native creation order here rather than
/// an inferred numeric sort. Current v0.111.0
/// `CombatState::GetTeammatesOf` RVA `0x137397` returns the same-side roster;
/// `GlimpseBeyond/<OnPlay>d__9::MoveNext` RVA `0x3a1968` filters it to living
/// Players at IL `0x009e-0x00d8` and awaits one plural Soul Add per recipient
/// in that order at IL `0x00d9-0x01c9`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn glimpse_beyond(
    state: &mut HotState,
    catalog: &Catalog,
    amount: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        amount: usize,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if !matches!(amount, 3 | 4) {
            return Err(EngineRefusal::MalformedArgs("glimpse_beyond_exact amount"));
        }
        if state.multiplayer_ally_key > 1
            || (state.multiplayer_ally_key == 1 && state.fanouts.multiplayer_ally().key != 1)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Glimpse Beyond represented Player order",
            ));
        }
        let soul = CardIdentity {
            id: CardId::Soul,
            upgrade: 0,
            enchantment: None,
        };
        catalog
            .atom(&soul)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(soul))?;

        // Freeze the native same-side living Player snapshot once, before
        // any generated callback. Each recipient owns one separately awaited
        // plural command with a leading combat-ending gate. Once that gate
        // stops the walk, later recipients are never begun; within an already
        // begun plural command every remaining Soul still records history and
        // reaches the (terminal no-op) generated hook.
        let recipients = living_keys(state).collect::<Vec<_>>();
        for key in recipients {
            if state.history.over {
                break;
            }
            for _ in 0..amount {
                if key == 0 {
                    crate::engine::cards::inject_generated_record_before_ending_draw_random(
                        state, catalog, soul, events,
                    )?;
                } else {
                    crate::engine::cards::inject_owner_created_remote_draw_random_record_before_ending(
                        state, catalog, key, soul, events,
                    )?;
                }
            }
        }
        Ok(())
    }

    // The two Player batches form one card body. A late remote RNG/history or
    // generated-listener refusal must not expose the earlier local batch.
    let mut probe = state.clone();
    apply(&mut probe, catalog, amount, &mut Vec::new())?;
    apply(state, catalog, amount, events)
}

/// Imitation Learning keys its power by target Player, and a repeat on the
/// same target STACKS into that one object (#3057).
///
/// v0.111.0 (DLL `9cb4f1ad`): `ImitationLearningPower::get_InstanceType`
/// RVA `0xa3c3d` IL_0001 returns 1 (Instanced), so a bare `PowerCmd.Apply`
/// would attach a second object. The card never issues one for a live
/// target: `ImitationLearning/<OnPlay>d__7::MoveNext` RVA `0x3a6b54` takes
/// the owner's `Powers.OfType<ImitationLearningPower>()` (IL_00df-00ef) and
/// `FirstOrDefault` with `<>c__DisplayClass7_0::<OnPlay>b__0` (RVA
/// `0x3a6b3a`: `PlayerTarget == cardPlay.Target.Player`, IL_0001-0017) at
/// IL_0105. A hit goes to `PowerCmd.ModifyAmount` on that object
/// (IL_0121-0139); only a miss reaches `PowerCmd.Apply` (IL_019c-01bb). So
/// one object per target whose Amount is the sum of that target's plays is
/// native, and this per-key sum is exact.
pub(crate) fn apply_imitation_learning(
    state: &mut HotState,
    key: u32,
    amount: i32,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let current = state
        .fanouts
        .imitation_learning(key)
        .ok_or(EngineRefusal::MalformedArgs("Imitation Learning target"))?;
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("Imitation Learning"))?;
    if !state.fanouts.set_imitation_learning(key, updated) {
        return Err(EngineRefusal::MalformedArgs("Imitation Learning target"));
    }
    Ok(())
}

pub(crate) fn imitation_before_local_power(
    state: &mut HotState,
    catalog: &Catalog,
    source: HotCard,
    spec: &CardSpec,
    first_in_series: bool,
) -> Result<(), EngineRefusal> {
    if !spec.is_power || !first_in_series || state.fanouts.imitation_learning(0) == Some(0) {
        return Ok(());
    }
    if state
        .fanouts
        .imitation_clones()
        .iter()
        .any(|record| record.target_key == 0 && record.source_identity == source.uid)
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Imitation Learning duplicate source clone",
        ));
    }
    let identity = catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    let instance = state.card_states.get(source.uid);
    let clone_uid = state.next_card_uid;
    state.next_card_uid = state
        .next_card_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    state.fanouts.imitation_clones_mut().push(ImitationClone {
        target_key: 0,
        source_identity: source.uid,
        clone_uid,
        identity,
        flags: source.flags,
        instance,
    });
    Ok(())
}

pub(crate) fn imitation_after_local_power(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !spec.is_power {
        return Ok(());
    }
    let matches = state
        .fanouts
        .imitation_clones()
        .iter()
        .filter(|record| record.target_key == 0 && record.source_identity == source_uid)
        .cloned()
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(());
    }
    let [_record] = matches.as_slice() else {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Imitation Learning source correlation",
        ));
    };
    let mut probe = state.clone();
    imitation_after_local_power_inner(&mut probe, catalog, source_uid, spec, &mut Vec::new())?;
    imitation_after_local_power_inner(state, catalog, source_uid, spec, events)
}

fn imitation_after_local_power_inner(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    _spec: &CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let record = state
        .fanouts
        .imitation_clones()
        .iter()
        .find(|record| record.target_key == 0 && record.source_identity == source_uid)
        .cloned()
        .ok_or(EngineRefusal::PowerOrderNotModeled(
            "Imitation Learning source correlation",
        ))?;
    let amount = state
        .fanouts
        .imitation_learning(0)
        .ok_or(EngineRefusal::MalformedArgs("Imitation Learning owner"))?;
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("Imitation Learning owner"));
    }
    if !state.fanouts.set_imitation_learning(0, amount - 1) {
        return Err(EngineRefusal::MalformedArgs("Imitation Learning owner"));
    }
    if amount == 1 {
        state
            .fanouts
            .imitation_clones_mut()
            .retain(|candidate| candidate.target_key != 0);
    } else {
        state
            .fanouts
            .imitation_clones_mut()
            .retain(|candidate| candidate.source_identity != source_uid);
    }
    let atom = catalog
        .atom(&record.identity)
        .ok_or(EngineRefusal::MalformedArgs(
            "Imitation Learning clone identity",
        ))?;
    let clone = HotCard {
        uid: record.clone_uid,
        atom,
        flags: record.flags,
    };
    if !record.instance.is_vacant() {
        state
            .card_states
            .set(record.clone_uid, record.instance.clone());
    }
    #[cfg(test)]
    IMITATION_CLONE_PAYLOAD_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .push((clone.flags, state.card_states.get(clone.uid)));
    });
    state.piles.get_mut(PileId::Play).make_mut().push(clone);
    crate::engine::play::autoplay_imitation_clone(state, catalog, clone, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::engine::{Action, SelectionRef, apply_action, apply_action_into, legal_actions};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotMonster, MultiplayerAllyState};
    use crate::ids::MonsterKind;

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    /// A live-combat party: GainEnergy checks IsEnding and GainBlock checks
    /// IsOverOrEnding, each trivially true for a monsterless state.
    fn live_party() -> HotState {
        let mut state = party();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state
    }

    fn party() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        state
    }

    /// #2708: `PlayerCmd::GainEnergy` 0x3ee8a0 returns at IsEnding (IL_0035),
    /// the shared `damage_combat_is_ending` projection, not at history.over.
    #[test]
    fn gain_energy_skips_while_combat_is_ending_before_the_over_latch() {
        // Ordinary live combat: both keys gain.
        let mut live = live_party();
        gain_energy(&mut live, 0, 2).unwrap();
        gain_energy(&mut live, 1, 3).unwrap();
        assert_eq!(
            (live.energy, live.fanouts.multiplayer_ally().energy),
            (11, 3)
        );

        // Every primary dead, no veto, over not yet latched: both keys skip.
        let mut ending = live_party();
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let before = ending.clone();
        gain_energy(&mut ending, 0, 2).unwrap();
        gain_energy(&mut ending, 1, 3).unwrap();
        assert_eq!(ending, before);

        // Only a secondary enemy survives: IsEnding counts primaries only.
        let mut secondary = live_party();
        secondary.monsters_mut()[0].hp = 0;
        secondary
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::GasBomb, 10));
        assert!(crate::engine::damage::damage_combat_is_ending(&secondary));
        let before = secondary.clone();
        gain_energy(&mut secondary, 0, 2).unwrap();
        assert_eq!(secondary, before);

        // An Adaptable veto keeps the same dead roster live: the gain lands.
        let mut vetoed = live_party();
        vetoed.monsters_mut()[0].hp = 0;
        vetoed.monsters_mut()[0]
            .powers
            .set(PowerId::Adaptable, crate::powers::SlotWire::Int, 1);
        assert!(!crate::engine::damage::damage_combat_is_ending(&vetoed));
        gain_energy(&mut vetoed, 0, 2).unwrap();
        assert_eq!(vetoed.energy, 11);
    }

    /// #2708: Believe in You's grant is `PlayerCmd::GainEnergy`
    /// (`BelieveInYou/<OnPlay>d__7` 0x38c32c IL_0057), skipped while ending.
    #[test]
    fn believe_in_you_grant_skips_while_combat_is_ending_before_the_over_latch() {
        let believe = identity(CardId::BelieveInYou, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(believe).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let run = |state: &mut HotState| {
            crate::steps::neutral::believe_in_you_exact(&mut StepCtx {
                state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 42,
                target: Some(1),
                selection: None,
                x_value: 0,
                args: &[crate::catalog::CompiledArg::I(2)],
                events: &mut Vec::new(),
            })
        };

        let mut live = live_party();
        run(&mut live).unwrap();
        assert_eq!(live.fanouts.multiplayer_ally().energy, 2);

        let mut ending = live_party();
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        let before = ending.clone();
        run(&mut ending).unwrap();
        assert_eq!(ending, before);
    }

    /// #3218: every `gain_powered_block` caller is a plain
    /// `CreatureCmd::GainBlock` (`<GainBlock>d__18` 0x3eaec0), which returns
    /// at IsOverOrEnding (IL_0032) before any Block write. With the only
    /// primary dead and a live secondary (Gas Bomb), before history.over
    /// latches, the helper on both keys and the Lift, Mimic and Rally bodies
    /// are no-ops; an Adaptable veto keeps the same roster live and each
    /// gains Block.
    #[test]
    fn powered_block_skips_while_combat_is_ending_before_the_over_latch() {
        type Body = fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>;
        let ending = || {
            let mut state = live_party();
            state.block = 4;
            state.fanouts.multiplayer_ally_mut().block = 7;
            state.monsters_mut()[0].hp = 0;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::GasBomb, 10));
            assert!(!state.history.over);
            assert!(crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        let vetoed = || {
            let mut state = ending();
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Adaptable, crate::powers::SlotWire::Int, 1);
            assert!(!crate::engine::damage::damage_combat_is_ending(&state));
            state
        };
        let mut builder = CatalogBuilder::new();
        for id in [CardId::Lift, CardId::Mimic, CardId::Rally] {
            builder.intern(identity(id, 0)).unwrap();
        }
        let catalog = builder.build();
        let spec = |id| {
            *catalog
                .spec(catalog.atom(&identity(id, 0)).unwrap())
                .unwrap()
        };
        let (lift, mimic, rally) = (spec(CardId::Lift), spec(CardId::Mimic), spec(CardId::Rally));
        let lift_args = [crate::catalog::CompiledArg::I(11)];
        let rally_args = [crate::catalog::CompiledArg::I(12)];
        type Case<'a> = (
            &'a str,
            &'a CardSpec,
            usize,
            &'a [crate::catalog::CompiledArg],
            Body,
            (i32, i32),
        );
        let bodies: [Case<'_>; 4] = [
            (
                "Lift owner",
                &lift,
                0,
                &lift_args,
                crate::steps::neutral::lift_exact,
                (15, 7),
            ),
            (
                "Lift teammate",
                &lift,
                1,
                &lift_args,
                crate::steps::neutral::lift_exact,
                (4, 18),
            ),
            (
                "Mimic",
                &mimic,
                1,
                &[],
                crate::steps::neutral::mimic_exact,
                (11, 7),
            ),
            (
                "Rally",
                &rally,
                0,
                &rally_args,
                crate::steps::neutral::rally_exact,
                (16, 19),
            ),
        ];
        for (name, spec, target, args, body, live_blocks) in bodies {
            let run = |state: &mut HotState| {
                body(&mut StepCtx {
                    state,
                    catalog: &catalog,
                    spec,
                    source_uid: 42,
                    target: Some(target),
                    selection: None,
                    x_value: 0,
                    args,
                    events: &mut Vec::new(),
                })
            };
            let mut state = ending();
            let before = state.clone();
            run(&mut state).unwrap();
            assert_eq!(state, before, "{name} gains nothing while ending");

            let mut state = vetoed();
            run(&mut state).unwrap();
            assert_eq!(
                (state.block, state.fanouts.multiplayer_ally().block),
                live_blocks,
                "{name} gains while the veto keeps combat live"
            );
        }

        for key in [0, 1] {
            let mut state = ending();
            let before = state.clone();
            let mut events = Vec::new();
            gain_powered_block(&mut state, &catalog, key, &lift, 11, &mut events).unwrap();
            assert_eq!(state, before, "key {key}");
            assert!(events.is_empty(), "key {key}");
        }
    }

    #[test]
    fn selected_and_all_player_resources_keep_owner_and_remote_state_distinct() {
        let mut builder = CatalogBuilder::new();
        let rally = identity(CardId::Rally, 0);
        builder.intern(rally).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(catalog.atom(&rally).unwrap()).unwrap();
        let mut state = live_party();
        state.block = 4;
        state.fanouts.multiplayer_ally_mut().block = 7;
        state.fanouts.multiplayer_ally_mut().temp_dexterity = 6;
        state
            .powers
            .set(PowerId::Dexterity, crate::powers::SlotWire::Int, 2);
        state
            .powers
            .set(PowerId::TempDexterity, crate::powers::SlotWire::Int, 3);

        gain_energy(&mut state, 1, 3).unwrap();
        gain_temp_strength(&mut state, 1, 5, &mut Vec::new()).unwrap();
        add_draw_next_turn(&mut state, 0, 2).unwrap();
        gain_powered_block(
            &mut state,
            &crate::catalog::CatalogBuilder::new().build(),
            1,
            spec,
            11,
            &mut Vec::new(),
        )
        .unwrap();

        let ally = state.fanouts.multiplayer_ally();
        assert_eq!((state.energy, ally.energy), (9, 3));
        assert_eq!(
            (state.powers.value(PowerId::Strength), ally.strength),
            (0, 0)
        );
        assert_eq!(ally.temp_strength, 5);
        assert_eq!(state.powers.value(PowerId::DrawNextTurn), 2);
        assert_eq!((state.block, ally.block), (4, 23));
        assert_eq!(state.history.card_block_gains, 0);
        assert_eq!(selected_block(&state, 0).unwrap(), 4);
        assert_eq!(selected_block(&state, 1).unwrap(), 23);
    }

    #[test]
    fn remote_temporary_stat_overflow_preserves_shared_fanout_cow_and_events() {
        for stat in [TemporaryAllyStat::Strength, TemporaryAllyStat::Dexterity] {
            let mut state = party();
            match stat {
                TemporaryAllyStat::Strength => {
                    state.fanouts.multiplayer_ally_mut().temp_strength = i32::MAX;
                }
                TemporaryAllyStat::Dexterity => {
                    state.fanouts.multiplayer_ally_mut().temp_dexterity = i32::MAX;
                }
            }
            let before = state.clone();
            let mut events = vec![Event::PlayerDamaged {
                blocked: 1,
                hp_lost: 2,
                hp: 3,
            }];
            let before_events = events.clone();
            assert!(state.fanouts.shares_store_with(&before.fanouts));

            let result = match stat {
                TemporaryAllyStat::Strength => gain_temp_strength(&mut state, 1, 1, &mut events),
                TemporaryAllyStat::Dexterity => gain_temp_dexterity(&mut state, 1, 1, &mut events),
            };
            let expected = match stat {
                TemporaryAllyStat::Strength => {
                    EngineRefusal::CounterOverflow("remote temporary strength")
                }
                TemporaryAllyStat::Dexterity => {
                    EngineRefusal::CounterOverflow("remote temporary dexterity")
                }
            };
            assert_eq!(result, Err(expected));
            assert_eq!(state, before);
            assert_eq!(events, before_events);
            assert!(state.fanouts.shares_store_with(&before.fanouts));
        }
    }

    #[test]
    fn remote_powered_block_previews_owner_dex_and_refuses_overflow_atomically() {
        let mut builder = CatalogBuilder::new();
        let lift = identity(CardId::Lift, 0);
        builder.intern(lift).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(catalog.atom(&lift).unwrap()).unwrap();

        let mut overflow = live_party();
        overflow.fanouts.multiplayer_ally_mut().block = 7;
        overflow
            .powers
            .set(PowerId::Dexterity, crate::powers::SlotWire::Int, i32::MAX);
        let before = overflow.clone();
        assert_eq!(
            gain_powered_block(
                &mut overflow,
                &crate::catalog::CatalogBuilder::new().build(),
                1,
                spec,
                11,
                &mut Vec::new()
            ),
            Err(EngineRefusal::CounterOverflow("card block"))
        );
        assert_eq!(overflow, before);

        let mut zero = live_party();
        zero.fanouts.multiplayer_ally_mut().block = 7;
        zero.powers
            .set(PowerId::Dexterity, crate::powers::SlotWire::Int, -11);
        gain_powered_block(
            &mut zero,
            &crate::catalog::CatalogBuilder::new().build(),
            1,
            spec,
            11,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(zero.fanouts.multiplayer_ally().block, 7);
        assert_eq!(zero.history.card_block_gains, 0);
    }

    #[test]
    fn remote_powered_block_refuses_forged_stored_block_before_mutation() {
        let mut builder = CatalogBuilder::new();
        let lift = identity(CardId::Lift, 0);
        builder.intern(lift).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(catalog.atom(&lift).unwrap()).unwrap();

        for forged in [-1, PLAYER_RESOURCE_CAP + 1] {
            let mut state = live_party();
            state.fanouts.multiplayer_ally_mut().block = forged;
            let mut events = vec![Event::PlayerBlockGained {
                amount: 1,
                block: 1,
            }];
            let before = state.clone();
            let before_events = events.clone();
            assert_eq!(
                gain_powered_block(
                    &mut state,
                    &crate::catalog::CatalogBuilder::new().build(),
                    1,
                    spec,
                    11,
                    &mut events
                ),
                Err(EngineRefusal::MalformedArgs("remote block"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn permanent_strength_routes_locally_or_remotely_and_overflow_is_atomic() {
        let mut state = party();
        let mut events = Vec::new();
        gain_permanent_strength(&mut state, 0, 5, &mut events).unwrap();
        gain_permanent_strength(&mut state, 1, 7, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::Strength), 5);
        assert_eq!(state.fanouts.multiplayer_ally().strength, 7);
        assert_eq!(
            events.len(),
            1,
            "remote teammate events are not owner events"
        );

        state.fanouts.multiplayer_ally_mut().strength = i32::MAX;
        let before = state.clone();
        let before_events = events.clone();
        assert_eq!(
            gain_permanent_strength(&mut state, 1, 1, &mut events),
            Err(EngineRefusal::CounterOverflow("remote permanent strength"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn imitation_mutable_clone_freezes_full_payload_then_routes_independently() {
        let inflame = identity(CardId::Inflame, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(inflame).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 10,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut payload = crate::hot::CardInstanceState {
            local_cost_modifiers: crate::hot::LocalCostModifiers::from_rows(vec![
                crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Set,
                    amount: 1,
                    expiration: crate::hot::LocalCostExpiration::ThisCombat,
                    reduce_only: true,
                },
                crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Set,
                    amount: 0,
                    expiration: crate::hot::LocalCostExpiration::ThisTurnOrPlayed,
                    reduce_only: false,
                },
            ]),
            free_star_cost_this_turn_or_played_rows: 1,
            ..crate::hot::CardInstanceState::default()
        };
        payload.set_base_replay_count(None).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 11;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.card_states.set(source.uid, payload.clone());
        assert!(state.fanouts.set_imitation_learning(0, 1));
        IMITATION_CLONE_PAYLOAD_TRACE.with(|trace| trace.borrow_mut().clear());

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        IMITATION_CLONE_PAYLOAD_TRACE.with(|trace| {
            let trace = trace.borrow();
            assert_eq!(trace.len(), 2, "cold rehearsal and committed action");
            assert!(
                trace
                    .iter()
                    .all(|entry| { entry == &(CARD_FLAG_DEFAULT_PHYSICAL_STATE, payload.clone()) })
            );
        });
        assert_eq!(state.powers.value(PowerId::Strength), 4);
        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert!(state.piles.get(PileId::Play).is_empty());
        assert!(state.card_states.get_ref(11).is_none());
        assert!(state.card_states.get_ref(10).is_none());
    }

    #[test]
    fn remote_temporary_strength_expires_late_and_lethal_local_doom_preserves_it() {
        let catalog = CatalogBuilder::new().build();
        let mut ordinary = party();
        ordinary.fanouts.multiplayer_ally_mut().temp_strength = 5;
        crate::engine::turn::end_player_turn(&mut ordinary, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(ordinary.fanouts.multiplayer_ally().temp_strength, 0);

        let mut lethal = party();
        lethal.hp = 1;
        lethal.fanouts.multiplayer_ally_mut().temp_strength = 5;
        lethal
            .powers
            .set(PowerId::Doom, crate::powers::SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut lethal);
        crate::engine::turn::end_player_turn(&mut lethal, &catalog, &mut Vec::new()).unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.fanouts.multiplayer_ally().temp_strength, 5);
    }

    #[test]
    fn remote_draw_owns_its_shuffle_stream_and_exact_hand_order() {
        let mut state = party();
        let strike = identity(CardId::StrikeIronclad, 0);
        let defend = identity(CardId::DefendIronclad, 0);
        let live = RngStreamState {
            words: [1, 2, 3, 4],
            counter: 7,
        };
        {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.discard = Arc::new(vec![
                MultiplayerAllyCard::immutable(strike).unwrap(),
                MultiplayerAllyCard::immutable(defend).unwrap(),
            ]);
            ally.shuffle_rng = Some(live);
        }
        let mut expected = vec![defend, strike];
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        rng.shuffle(&mut expected).unwrap();

        remote_draw(&mut state, 2).unwrap();

        let ally = state.fanouts.multiplayer_ally();
        assert_eq!(
            ally.hand
                .iter()
                .map(|card| card.identity)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(ally.draw.is_empty());
        assert!(ally.discard.is_empty());
        assert_eq!(
            ally.shuffle_rng,
            Some(RngStreamState {
                words: rng.words,
                counter: rng.counter,
            })
        );
    }

    #[test]
    fn blade_symphony_routes_each_serial_shiv_by_recipient_hand_capacity() {
        let blade = identity(CardId::BladeSymphony, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(blade).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 20;
        state.fanouts.multiplayer_ally_mut().hand = Arc::new(vec![
            MultiplayerAllyCard::immutable(
                identity(CardId::DefendIronclad, 0)
            )
            .unwrap();
            9
        ]);

        blade_symphony(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert_eq!(state.next_card_uid, 22);
        assert_eq!(state.next_generated_hook_uid, 4);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        let ally = state.fanouts.multiplayer_ally();
        assert_eq!(ally.hand.len(), 10);
        assert_eq!(
            ally.discard
                .iter()
                .map(|card| card.identity)
                .collect::<Vec<_>>(),
            [identity(CardId::Shiv, 0)]
        );
        assert_eq!(ally.owner_generated_cards_combat, 2);
    }

    #[test]
    fn blade_remote_shivs_advance_the_epoch_but_skip_local_generated_listeners() {
        let blade = identity(CardId::BladeSymphony, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(blade).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 20;
        state
            .powers
            .set(PowerId::Arsenal, crate::powers::SlotWire::Int, 2);
        state
            .powers
            .set(PowerId::PillarOfCreation, crate::powers::SlotWire::Int, 3);
        state
            .powers
            .set(PowerId::Smokestack, crate::powers::SlotWire::Int, 5);
        assert!(state.fanouts.set_local_generated_power_order(&[
            PowerId::Arsenal,
            PowerId::PillarOfCreation,
            PowerId::Smokestack,
        ]));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        let mut events = Vec::new();

        blade_symphony(&mut state, &catalog, &mut events).unwrap();

        // The local recipient is first and its two Shivs run Arsenal/Pillar.
        // Shiv is a Skill, so Smokestack is a no-op even for that owner path.
        assert_eq!(state.powers.value(PowerId::Strength), 4);
        assert_eq!(state.block, 6);
        assert_eq!(state.monsters[0].hp, 40);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::CardResolved { .. }))
                .count(),
            2,
            "remote payload-only Shivs publish no local CardResolved event"
        );

        // The remote recipient still owns two history entries and two hook
        // epochs, but creator_is_owner=false skips every local owner power.
        assert_eq!(state.next_generated_hook_uid, 4);
        assert_eq!(state.fanouts.multiplayer_ally().hand.len(), 2);
        assert_eq!(
            state
                .fanouts
                .multiplayer_ally()
                .owner_generated_cards_combat,
            2
        );
    }

    #[test]
    fn blade_symphony_late_generated_range_refusal_is_whole_batch_atomic() {
        let blade = identity(CardId::BladeSymphony, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(blade).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 20;
        state.next_generated_hook_uid = i32::MAX - 2;
        let before = state.clone();
        let mut events = vec![Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        }];
        let events_before = events.clone();

        assert_eq!(
            blade_symphony(&mut state, &catalog, &mut events),
            Err(EngineRefusal::CounterOverflow("next_generated_hook_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn glimpse_beyond_uses_fixed_owner_then_remote_order_and_local_creator_hooks() {
        let soul = identity(CardId::Soul, 0);
        let strike = identity(CardId::StrikeIronclad, 0);
        let defend = identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        for card in [soul, strike, defend] {
            builder.intern(card).unwrap();
        }
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 20;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 19,
            atom: catalog.atom(&strike).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.fanouts.multiplayer_ally_mut().draw =
            Arc::new(vec![MultiplayerAllyCard::immutable(defend).unwrap()]);
        let live = RngStreamState {
            words: [1, 2, 3, 4],
            counter: 0,
        };
        state.rng.set(RngStream::Rng, live);
        state
            .powers
            .set(PowerId::Arsenal, crate::powers::SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal])
        );

        let mut expected_rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let mut expected_local = vec![strike];
        let mut expected_remote = vec![defend];
        for draw in [&mut expected_local, &mut expected_remote] {
            for _ in 0..3 {
                let bound = i32::try_from(draw.len() + 1).unwrap();
                let index = usize::try_from(expected_rng.next_bounded(bound).unwrap()).unwrap();
                draw.insert(index, soul);
            }
        }

        let mut events = Vec::new();
        glimpse_beyond(&mut state, &catalog, 3, &mut events).unwrap();

        let local = state
            .piles
            .get(PileId::Draw)
            .as_slice()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity)
            .collect::<Vec<_>>();
        assert_eq!(local, expected_local);
        assert_eq!(
            state
                .fanouts
                .multiplayer_ally()
                .draw
                .iter()
                .map(|card| card.identity)
                .collect::<Vec<_>>(),
            expected_remote
        );
        assert_eq!(state.rng.get(RngStream::Rng).counter, expected_rng.counter);
        assert_eq!(state.history.owner_generated_cards_combat, 6);
        assert_eq!(state.next_generated_hook_uid, 6);
        assert_eq!(state.next_card_uid, 23);
        assert_eq!(
            state
                .fanouts
                .multiplayer_ally()
                .owner_generated_cards_combat,
            0,
            "the Glimpse owner, not each Soul owner, owns native history"
        );
        assert_eq!(state.powers.value(PowerId::Strength), 6);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::CardResolved { .. }))
                .count(),
            3,
            "remote payload Souls publish no local physical-card event"
        );
    }

    /// #3256: Regalite's gate reads only the creator
    /// (`Regalite/<AfterCardGeneratedForCombat>d__8::MoveNext` RVA
    /// `0x32f988` IL_0020-IL_0036: `creator == Owner`), never the generated
    /// card's owner. A local Glimpse Beyond Soul inserted into the remote
    /// ally's Draw names the local owner as its creator, so Regalite answers.
    #[test]
    fn glimpse_beyond_remote_soul_fires_the_local_owners_regalite() {
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(soul).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicRegalite])
            .unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.fanouts.multiplayer_ally_mut().alive = true;
        crate::engine::cards::inject_owner_created_remote_draw_random_record_before_ending(
            &mut state,
            &catalog,
            1,
            soul,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.fanouts.multiplayer_ally().draw.len(), 1);
        assert_eq!(state.block, i32::from(state.regalite_block_amount));
        assert!(state.fanouts.regalite_used_this_turn());
    }

    #[test]
    fn glimpse_beyond_terminal_gate_and_late_remote_failure_are_atomic() {
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(soul).unwrap();
        let catalog = builder.build();

        let mut terminal = party();
        terminal.history.over = true;
        let terminal_before = terminal.clone();
        let mut terminal_events = Vec::new();
        glimpse_beyond(&mut terminal, &catalog, 3, &mut terminal_events).unwrap();
        assert_eq!(terminal, terminal_before);
        assert!(terminal_events.is_empty());

        let mut late = party();
        late.next_generated_hook_uid = i32::MAX - 3;
        let live = RngStreamState {
            words: [5, 6, 7, 8],
            counter: 9,
        };
        late.rng.set(RngStream::Rng, live);
        let late_before = late.clone();
        let mut late_events = vec![Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        }];
        let events_before = late_events.clone();

        assert_eq!(
            glimpse_beyond(&mut late, &catalog, 3, &mut late_events),
            Err(EngineRefusal::CounterOverflow("next_generated_hook_uid"))
        );
        assert_eq!(late, late_before);
        assert_eq!(late_events, events_before);
    }

    #[test]
    fn glimpse_beyond_filters_a_dead_remote_from_the_frozen_player_roster() {
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(soul).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 10;
        state.fanouts.multiplayer_ally_mut().alive = false;
        state.fanouts.multiplayer_ally_mut().draw =
            Arc::new(vec![MultiplayerAllyCard::immutable(soul).unwrap()]);
        let remote_before = state.fanouts.multiplayer_ally().clone();

        let mut events = Vec::new();
        glimpse_beyond(&mut state, &catalog, 3, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert_eq!(state.fanouts.multiplayer_ally(), &remote_before);
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
        assert_eq!(state.next_card_uid, 13);
        assert_eq!(state.rng.get(RngStream::Rng).counter, 3);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn glimpse_beyond_refuses_malformed_order_and_recipient_atomically() {
        let soul = identity(CardId::Soul, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(soul).unwrap();
        let catalog = builder.build();
        let sentinel = Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        };

        let mut malformed_order = party();
        malformed_order.fanouts.multiplayer_ally_mut().key = 0;
        let order_before = malformed_order.clone();
        let mut order_events = vec![sentinel];
        assert_eq!(
            glimpse_beyond(&mut malformed_order, &catalog, 3, &mut order_events),
            Err(EngineRefusal::MalformedArgs(
                "Glimpse Beyond represented Player order"
            ))
        );
        assert_eq!(malformed_order, order_before);
        assert_eq!(order_events, [sentinel]);

        let mut malformed_recipient = party();
        let recipient_before = malformed_recipient.clone();
        let mut recipient_events = vec![sentinel];
        assert_eq!(
            crate::engine::cards::inject_owner_created_remote_draw_random_record_before_ending(
                &mut malformed_recipient,
                &catalog,
                2,
                soul,
                &mut recipient_events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Glimpse Beyond remote recipient"
            ))
        );
        assert_eq!(malformed_recipient, recipient_before);
        assert_eq!(recipient_events, [sentinel]);
    }

    #[test]
    fn imitation_learning_consumes_two_stacks_as_two_frozen_power_replays() {
        let imitation = identity(CardId::ImitationLearning, 0);
        let inflame = identity(CardId::Inflame, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(imitation).unwrap();
        builder.intern(inflame).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.next_card_uid = 3;
        for (uid, identity) in [(1, imitation), (2, inflame)] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: catalog.atom(&identity).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let applied = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(applied.fanouts.imitation_learning(0), Some(2));

        let replayed = apply_action(
            &applied,
            &catalog,
            &Action::Play {
                uid: 2,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;

        assert_eq!(replayed.powers.value(PowerId::Strength), 6);
        assert_eq!(replayed.next_card_uid, 5);
        assert_eq!(replayed.fanouts.imitation_learning(0), Some(0));
        assert!(replayed.fanouts.imitation_clones().is_empty());
    }

    /// #3057 witness: a second Imitation Learning on the SAME target is
    /// native `PowerCmd.ModifyAmount` on the one live object (OnPlay RVA
    /// `0x3a6b54` IL_0105-0139), so the two plays form one four-stack object
    /// that replays a later Power four times, not two objects of two.
    #[test]
    fn imitation_learning_repeat_on_one_target_stacks_into_one_object() {
        let imitation = identity(CardId::ImitationLearning, 0);
        let inflame = identity(CardId::Inflame, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(imitation).unwrap();
        builder.intern(inflame).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.energy = 9;
        state.next_card_uid = 4;
        for (uid, identity) in [(1, imitation), (2, imitation), (3, inflame)] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: catalog.atom(&identity).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let mut applied = state;
        for uid in [1, 2] {
            applied = apply_action(
                &applied,
                &catalog,
                &Action::Play {
                    uid,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
        }
        assert_eq!(applied.fanouts.imitation_learning(0), Some(4));

        let replayed = apply_action(
            &applied,
            &catalog,
            &Action::Play {
                uid: 3,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(replayed.powers.value(PowerId::Strength), 10);
        assert_eq!(replayed.fanouts.imitation_learning(0), Some(0));
        assert!(replayed.fanouts.imitation_clones().is_empty());
    }

    /// #3057 witness: a repeat Intercept on the SAME target attaches a second
    /// native CoveredPower (Instanced, `get_InstanceType` RVA `0xa0ca1`), but
    /// that object is behaviourally inert. Its `<AfterApplied>d__9` (RVA
    /// `0x337c64`) finds the applier's live InterceptPower (IL_005e-006b) and
    /// calls `AddCoveredCreature` (IL_00e2-00e9), whose `Contains` guard (RVA
    /// `0xa4178` IL_0018-0029) keeps the covered set, and so Intercept's
    /// `coveredCreatures.Count + 1` multiplier (RVA `0xa425a` IL_001e-0030),
    /// unchanged. Its own `ModifyDamageMultiplicative` (RVA `0xa0d53`) is the
    /// same x0 as the first one's; both leave together at the enemy side end
    /// (`<AfterSideTurnEnd>d__12` RVA `0x337e84` IL_001d-0029) and on the
    /// same applier's death (`<AfterDeath>d__10` RVA `0x337da4` IL_002a-003b).
    /// So Rust's unique covered set is exact under a repeat.
    #[test]
    fn intercept_repeat_on_one_target_is_an_inert_second_covered_object() {
        let intercept = identity(CardId::Intercept, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(intercept).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.energy = 9;
        state.block = 0;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        for uid in [1, 2] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: catalog.atom(&intercept).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let once = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(once.fanouts.intercept_covered_mask(), 0b10);
        let twice = apply_action(
            &once,
            &catalog,
            &Action::Play {
                uid: 2,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(twice.fanouts.intercept_covered_mask(), 0b10);

        let mut hit = twice.clone();
        hit.block = 0;
        let hp = hit.hp;
        crate::engine::damage::monster_attack_player(&mut hit, 0, 9, 1, &mut Vec::new()).unwrap();
        assert_eq!(hp - hit.hp, 18, "still one covered creature: x2, not x3");
    }

    #[test]
    fn intercept_owner_zeroes_powered_hits_and_death_clears_only_that_owner() {
        let mut state = party();
        state.block = 0;
        state.fanouts.set_intercept_covered_mask(0b01);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        crate::engine::damage::monster_attack_player(&mut state, 0, 9, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.hp, 50);

        state.fanouts.set_intercept_covered_mask(0b11);
        state.hp = 1;
        crate::engine::damage::doom_kill_player(&mut state, None, &mut Vec::new()).unwrap();
        assert_eq!(state.fanouts.intercept_covered_mask(), 0b10);
    }

    #[test]
    fn intercept_remote_cover_doubles_damage_and_enemy_side_end_expires_it() {
        let mut state = party();
        state.block = 0;
        state.fanouts.set_intercept_covered_mask(0b10);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        crate::engine::damage::monster_attack_player(&mut state, 0, 9, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.hp, 32, "one remote CoveredPower composes as x2");

        crate::engine::turn::expire_intercept_covered(&mut state);
        assert_eq!(state.fanouts.intercept_covered_mask(), 0);
    }

    #[test]
    fn typed_target_bytes_offer_player_keys_without_reinterpreting_enemy_indexes() {
        let believe = identity(CardId::BelieveInYou, 0);
        let strike = identity(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(believe).unwrap();
        builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        for (uid, card) in [(10, believe), (11, strike)] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: catalog.atom(&card).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }

        let actions = legal_actions(&state, &catalog);
        assert!(actions.contains(&Action::Play {
            uid: 10,
            target: Some(0),
            selection: SelectionRef::NONE,
        }));
        assert!(actions.contains(&Action::Play {
            uid: 10,
            target: Some(1),
            selection: SelectionRef::NONE,
        }));
        assert!(actions.contains(&Action::Play {
            uid: 11,
            target: Some(0),
            selection: SelectionRef::NONE,
        }));
        assert!(!actions.contains(&Action::Play {
            uid: 11,
            target: Some(1),
            selection: SelectionRef::NONE,
        }));
    }

    #[test]
    fn autoplay_anyally_excludes_owner_and_pet_but_consumes_singleton_target_draw() {
        let believe = identity(CardId::BelieveInYou, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(believe).unwrap();
        let catalog = builder.build();
        let mut state = live_party();
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        let card = HotCard {
            uid: 42,
            atom: catalog.atom(&believe).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.piles.get_mut(PileId::Play).make_mut().push(card);
        let before = state.rng.get(RngStream::Targets).counter;

        crate::engine::play::autoplay_imitation_clone(&mut state, &catalog, card, &mut Vec::new())
            .unwrap();

        assert_eq!(state.energy, 9, "AutoPlay excludes the local owner");
        assert_eq!(state.fanouts.multiplayer_ally().energy, 2);
        assert_eq!(state.rng.get(RngStream::Targets).counter, before + 1);
        assert!(state.fanouts.pet().osty().is_some(), "pets are not targets");

        let mut dead = party();
        dead.fanouts.multiplayer_ally_mut().alive = false;
        dead.fanouts.set_osty(Some((5, 5))).unwrap();
        let dead_before = dead.rng.get(RngStream::Targets);
        assert_eq!(roll_auto_target_key(&mut dead).unwrap(), None);
        assert_eq!(dead.rng.get(RngStream::Targets), dead_before);
    }

    #[test]
    fn external_teammate_imitation_callback_refuses_before_public_action_mutation() {
        let strike = identity(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut state = party();
        state.fanouts.set_teammate_power_pending(Some((1, 77)));
        let before = state.clone();
        let mut events = vec![Event::PlayerDamaged {
            blocked: 1,
            hp_lost: 2,
            hp: 3,
        }];

        assert_eq!(
            apply_action_into(&state, &catalog, &Action::EndTurn, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(
            events.is_empty(),
            "the public buffer is cleared before refusal"
        );
    }
}
