//! Mid-combat physical-card creation and pile insertion.
//!
//! The catalog pre-interns every identity reachable from a monster move, so
//! minting only advances hot state: the physical uid allocator and the live
//! piles. This keeps atoms deterministic across search branches.

use std::sync::Arc;

#[cfg(test)]
use crate::catalog::RewardPool;
use crate::catalog::{CardAtom, CardIdentity, CardSpec, CardTargetType, Catalog};
#[cfg(test)]
use crate::content_tables::CREATIVE_AI_POWER_POOL_V1101;
use crate::decimal::DotNetDecimal;
use crate::hot::{
    CARD_FLAG_BOUND, CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_GENETIC_ALGORITHM_STATE,
    CARD_FLAG_HEXED, CARD_FLAG_LEGACY, CARD_FLAG_RINGING, CardInstanceState, HotCard, HotMonster,
    HotState, LEGACY_CARD_UID, LocalCostExpiration, LocalCostModifier, LocalCostModifierKind,
    LocalCostModifiers, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, EnchantmentId, MonsterKind, PowerId, RelicId};
use crate::rng::Xoshiro256StarStar;

use super::draw::MAX_CARDS_IN_HAND;
use super::play::unique_live_card_location;
use super::{EngineRefusal, Event};

const SELF_RETURN_IDS: [CardId; 2] = [CardId::Bolas, CardId::ThrummingHatchet];
const SELF_RETURN_PILES: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

fn self_return_program_is_exact(spec: &CardSpec) -> bool {
    SELF_RETURN_IDS.contains(&spec.identity.id)
        && matches!(spec.identity.upgrade, 0 | 1)
        && super::play::body_enchantment_is_exact(spec)
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
}

/// Validate Batch139's three ordered UID tuples and every still-live identity.
///
/// Bolas `BeforeHandDraw` outer/body RVAs are `0x285790`/`0x3d7064` and
/// Thrumming Hatchet's are `0x29ac48`/`0x40c94c` in the archived authority
/// cited by the Python oracle. Current v0.111.0 census equality pins both
/// immutable rows; the mechanic retains only `CardModel` identity, represented
/// here by the already-canonical physical UID.
///
/// Python's `_record_card_play_finished` writes the current tuple (frozen Python, deleted #2827). `_validate_batch139_self_return_state` owns tuple/allocation
/// authentication, while
/// `_complete_batch139_before_hand_draw_card_listeners` re-locates and moves
/// the frozen identities.
pub(crate) fn self_return_state_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let Some(private) = state.card_states.self_return() else {
        return true;
    };
    for uids in [
        private.current_turn.as_slice(),
        private.previous_turn.as_slice(),
        private.before_hand_draw.as_slice(),
    ] {
        if uids.iter().any(|uid| *uid >= state.next_card_uid)
            || uids
                .iter()
                .enumerate()
                .any(|(index, uid)| uids[..index].contains(uid))
        {
            return false;
        }
    }
    for uid in &private.before_hand_draw {
        let Ok(location) = unique_live_card_location(state, *uid) else {
            return false;
        };
        if let Some((pile, index)) = location {
            let card = state.piles.get(pile).as_slice()[index];
            if !catalog
                .spec(card.atom)
                .is_some_and(self_return_program_is_exact)
            {
                return false;
            }
        }
    }
    true
}

/// Whether beginning the next player turn can observe or create Batch139
/// state. Used to retain the existing clone/preflight transaction boundary.
pub(crate) fn self_return_turn_is_reachable(state: &HotState, catalog: &Catalog) -> bool {
    state.card_states.self_return().is_some_and(|private| {
        !private.current_turn.is_empty()
            || !private.previous_turn.is_empty()
            || !private.before_hand_draw.is_empty()
    }) || catalog.has_batch139_self_return()
        && SELF_RETURN_PILES.into_iter().any(|pile| {
            state.piles.get(pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(self_return_program_is_exact)
            })
        })
}

/// Record a completed Bolas/Hatchet play by exact physical identity.
pub(crate) fn record_self_return_finished(
    state: &mut HotState,
    spec: &CardSpec,
    uid: u32,
) -> Result<(), EngineRefusal> {
    if !SELF_RETURN_IDS.contains(&spec.identity.id) {
        return Ok(());
    }
    if !self_return_program_is_exact(spec)
        || uid >= state.next_card_uid
        || unique_live_card_location(state, uid)?.is_none()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Batch139 self-return finished identity",
        ));
    }
    state.card_states.record_self_return_finished(uid);
    state.exact_piles = true;
    Ok(())
}

/// Roll current-owner-turn completion identity into the one-turn history row.
pub(crate) fn rollover_self_return_turn(state: &mut HotState) {
    let Some(current) = state
        .card_states
        .self_return()
        .map(|private| private.current_turn.clone())
    else {
        return;
    };
    let empty = {
        let private = state.card_states.self_return_mut();
        private.previous_turn = current;
        private.current_turn.clear();
        private.is_empty()
    };
    if empty {
        state
            .card_states
            .set_self_return(crate::hot::SelfReturnState::default());
    }
}

/// Freeze native `AllPiles` card-listener order before the power/relic walk.
pub(crate) fn freeze_self_return_before_hand_draw(
    state: &mut HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    if !self_return_state_is_exact(state, catalog)
        || state
            .card_states
            .self_return()
            .is_some_and(|private| !private.before_hand_draw.is_empty())
    {
        return Err(EngineRefusal::MalformedArgs(
            "Batch139 BeforeHandDraw listener state",
        ));
    }
    if state.history.over {
        return Ok(());
    }
    if state.card_states.self_return().is_none() && !catalog.has_batch139_self_return() {
        return Ok(());
    }
    normalize_card_identities(state)?;
    let listeners = SELF_RETURN_PILES
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .filter_map(|card| {
            catalog
                .spec(card.atom)
                .is_some_and(self_return_program_is_exact)
                .then_some(card.uid)
        })
        .collect::<Vec<_>>();
    if !listeners.is_empty() {
        let private = state.card_states.self_return_mut();
        private.before_hand_draw = listeners;
    }
    if state.card_states.self_return().is_some() {
        state.exact_piles = true;
    }
    Ok(())
}

fn complete_self_return_before_hand_draw_inner(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !self_return_state_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Batch139 BeforeHandDraw listener state",
        ));
    }
    let Some(private) = state.card_states.self_return().cloned() else {
        return Ok(());
    };
    for uid in &private.before_hand_draw {
        let Some((source, index)) = unique_live_card_location(state, *uid)? else {
            continue;
        };
        let card = state.piles.get(source).as_slice()[index];
        if !catalog
            .spec(card.atom)
            .is_some_and(self_return_program_is_exact)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Batch139 frozen listener identity",
            ));
        }
        if !private.previous_turn.contains(uid) || source == PileId::Hand || state.history.over {
            continue;
        }
        let destination = if state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
            PileId::Discard
        } else {
            PileId::Hand
        };
        let moved = state.piles.get_mut(source).make_mut().remove(index);
        if moved != card {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: *uid,
                pile: source,
            });
        }
        state.piles.get_mut(destination).make_mut().push(moved);
        events.push(Event::CardResolved {
            uid: *uid,
            pile: destination,
        });
    }
    let mut cleared = private;
    cleared.before_hand_draw.clear();
    state.card_states.set_self_return(cleared);
    Ok(())
}

/// Resolve the complete frozen Batch139 listener walk atomically.
pub(crate) fn complete_self_return_before_hand_draw(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state
        .card_states
        .self_return()
        .is_none_or(|private| private.before_hand_draw.is_empty())
    {
        return Ok(());
    }
    let mut probe = state.clone();
    complete_self_return_before_hand_draw_inner(&mut probe, catalog, &mut Vec::new())?;
    complete_self_return_before_hand_draw_inner(state, catalog, events)
}

pub(crate) const AEONGLASS_CARD_PILES: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

/// Whether any public Aeonglass-only carrier is observable.
pub(crate) fn aeonglass_owner_reachable(state: &HotState) -> bool {
    state.fanouts.withering_cards_left() != 0
        || state.monsters.iter().any(|monster| {
            monster.kind == MonsterKind::Aeonglass
                || monster.aeonglass_additional_strength() != 0
                || monster.aeonglass_wither_upgrade_count() != 0
        })
}

/// Whether any public Aeonglass-only carrier or exact Wither atom is visible.
pub(crate) fn aeonglass_reachable(state: &HotState, catalog: &Catalog) -> bool {
    aeonglass_owner_reachable(state)
        || AEONGLASS_CARD_PILES.into_iter().any(|pile| {
            state.piles.get(pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| spec.identity.id == CardId::Wither)
            })
        })
}

fn aeonglass_cards_are_exact(state: &HotState, catalog: &Catalog) -> bool {
    for pile in AEONGLASS_CARD_PILES {
        for card in state.piles.get(pile).as_slice() {
            let Some(spec) = catalog.spec(card.atom) else {
                return false;
            };
            if spec.identity.id != CardId::Wither {
                continue;
            }
            // A live Wither implies exact piles: both native writers publish
            // it through a force-exact pile commit (see
            // `aeonglass_pile_mode_is_exact`).
            if !state.exact_piles
                || spec.identity.upgrade != 0
                || spec.identity.enchantment.is_some()
                || card.flags != CARD_FLAG_DEFAULT_PHYSICAL_STATE
                || card.uid >= state.next_card_uid
            {
                return false;
            }
            let instance = state.card_states.get(card.uid);
            if instance.damage_growth < 0
                || instance.damage_growth % 3 != 0
                || instance.has_exact_damage_growth_aux()
                || AEONGLASS_CARD_PILES
                    .into_iter()
                    .flat_map(|candidate| state.piles.get(candidate).as_slice())
                    .filter(|candidate| candidate.uid == card.uid)
                    .count()
                    != 1
            {
                return false;
            }
        }
    }
    true
}

/// Whether `exact_piles` is where the oracle has it for this Aeonglass state
/// (#2957).
///
/// `exact_piles` is the solver's pile-identity promotion, not a native field.
/// The oracle roots an Aeonglass fight with it at whatever the deck-group rule
/// says (`start_combat`, frozen Python, deleted #2827), and promotes it only
/// where an Aeonglass writer commits piles with `force_exact=True`:
/// Increasing Intensity's AllCards map (`_aeonglass_intensity`) and every generated Wither
/// (`_add_aeonglass_wither`).
/// `_validated_aeonglass_roster` never asks for
/// it.
///
/// Native combat start creates no Wither: `Aeonglass/<AfterAddedToRoom>d__31::
/// MoveNext` (v0.111.0 RVA `0x3524e8`) applies `WitheringPresencePower` 6 to
/// each opponent (IL_00d3-IL_0108) and `ArtifactPower` 3 to itself
/// (IL_0196-IL_01af), and nothing else. So before the first Intensity (both
/// private counters still 0) the flag may be either value; from the first
/// Intensity on it must be set. A live Wither card additionally requires it,
/// which [`aeonglass_cards_are_exact`] checks with the catalog in hand.
fn aeonglass_pile_mode_is_exact(exact_piles: bool, upgrades: i32, additional: i32) -> bool {
    exact_piles || (upgrades == 0 && additional == 0)
}

fn aeonglass_owner_state_is_exact(
    state: &HotState,
    allow_intensity_gap: bool,
    allow_countdown_zero: bool,
    allow_dying: bool,
) -> bool {
    let [boss] = state.monsters.as_slice() else {
        return false;
    };
    let additional = boss.aeonglass_additional_strength();
    let upgrades = boss.aeonglass_wither_upgrade_count();
    let countdown = state.fanouts.withering_cards_left();
    let artifact = boss.powers.get(PowerId::Artifact);
    state.multiplayer_ally_key == 0
        && aeonglass_pile_mode_is_exact(state.exact_piles, upgrades, additional)
        && boss.kind == MonsterKind::Aeonglass
        && (boss.slot, boss.uid, Some(boss.max_hp))
            == (
                0,
                0,
                super::monsters::native_fixed_hp(state, MonsterKind::Aeonglass),
            )
        && boss.hp <= boss.max_hp
        && (boss.hp > 0 || state.history.over || allow_dying)
        && (0..=999_999_999).contains(&boss.block)
        && matches!(boss.loop_pos, 0..=2)
        && boss.random_ai.is_empty()
        && boss.override_state == crate::hot::MonsterOverride::None
        && boss.forced_follow_up == crate::hot::MonsterFollowUp::None
        && !boss.spawn_noop
        && !boss.ritual_fresh
        && boss.last_spawned.is_none()
        && boss.curl_up_card_uid == -1
        && !boss.louse_curled()
        && boss.possess_strength_debit == 0
        && boss.possess_speed_debit == 0
        && boss.aeonglass_private_state_is_exact()
        && artifact.is_none_or(|slot| {
            slot.wire == crate::powers::SlotWire::Int && (1..=3).contains(&slot.value)
        })
        && (upgrades == additional
            || allow_intensity_gap && upgrades == additional.saturating_add(1))
        && state.fanouts.withering_cards_left_is_exact()
        && ((1..=6).contains(&countdown) || allow_countdown_zero && countdown == 0)
        && state.card_states.hopper().is_none()
        && state.card_states.dampen().is_none()
        && state.powers.value(PowerId::HexPower) == 0
        && state.powers.value(PowerId::ChainsOfBinding) == 0
        && !state.ringing()
}

fn aeonglass_state_is_exact_inner(
    state: &HotState,
    catalog: &Catalog,
    allow_intensity_gap: bool,
    allow_countdown_zero: bool,
) -> bool {
    let wither_identity = CardIdentity {
        id: CardId::Wither,
        upgrade: 0,
        enchantment: None,
    };
    aeonglass_owner_state_is_exact(state, allow_intensity_gap, allow_countdown_zero, false)
        && !catalog
            .hooks()
            .relics()
            .contains(&crate::ids::RelicId::RelicRegalite)
        && catalog.is_reachable(wither_identity)
        && catalog
            .atom(&wither_identity)
            .and_then(|atom| catalog.spec(atom))
            .is_some_and(|spec| {
                crate::content_tables::card_row(CardId::Wither, 0) == Some(spec.row)
            })
        && aeonglass_cards_are_exact(state, catalog)
}

/// Exact stable solo Aeonglass/Withering/Wither state.
pub(crate) fn aeonglass_state_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    aeonglass_state_is_exact_inner(state, catalog, false, false)
}

fn aeonglass_generated_transient_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    aeonglass_state_is_exact_inner(state, catalog, true, true)
}

/// Structural proof used only below an already catalog-authenticated public
/// transaction while Intensity/Withering callbacks hold their native gap.
pub(crate) fn aeonglass_internal_state_is_exact(state: &HotState) -> bool {
    aeonglass_owner_state_is_exact(state, true, true, true)
}

/// One immutable member of a physical multi-pile move command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrozenPhysicalCardMove {
    pub(crate) source_pile: PileId,
    pub(crate) source_index: usize,
    pub(crate) card: HotCard,
    pub(crate) instance: CardInstanceState,
}

/// Move one frozen ordered physical-card batch to a pile's bottom.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// plural `CardPileCmd::Add(IEnumerable<CardModel>, ...)` RVA `0x130720`
/// enters `<Add>d__9::MoveNext` RVA `0x3e2788`, whose `Any`/`First` reads the
/// lazy sequence before delegating to `<Add>d__10::MoveNext` RVA `0x3e1ba4`.
/// That body performs another `Any`; on ending it enumerates `Select(...).ToList`
/// into failed results, while the live path later inserts in enumeration order.
/// Summon Forth supplies Draw/Bottom over Hand, Discard, Exhaust, Play Blades;
/// the successful command preserves physical object identity and result order.
///
/// Every member is authenticated before the first removal. The current-build
/// `AfterCardChangedPiles` census contains only Bing Bong, Darkstone Periapt,
/// and Lucky Fysh. Those three exact subscribers remain refused by R2
/// admission; admitted Unsettling Lamp is not a subscriber. Therefore ordered
/// `CardResolved` publication is the complete admitted observable move
/// boundary. Any future admitted listener invalidates that source-derived
/// assumption before this helper may remain reachable.
pub(crate) fn move_frozen_physical_cards_to_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    frozen: &[FrozenPhysicalCardMove],
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // At this helper boundary the native lazy query has already been collapsed
    // into an immutable frozen slice. On ending, native enumerates that query
    // only to materialize an unobserved list of failed Add results: it mutates
    // no pile and fires no callback. Rust exposes neither that result list nor
    // enumerator side effects, so the admitted observable is this no-op. The
    // Python oracle `_summon_forth_exact` likewise returns before its query.
    if state.history.over || frozen.is_empty() {
        return Ok(());
    }
    for (position, entry) in frozen.iter().enumerate() {
        if frozen[..position]
            .iter()
            .any(|earlier| earlier.card.uid == entry.card.uid)
        {
            return Err(EngineRefusal::MalformedArgs(
                "physical move duplicate frozen identity",
            ));
        }
        if entry.card.flags & CARD_FLAG_LEGACY != 0
            || entry.card.uid >= state.next_card_uid
            || super::play::active_play_stack_contains_uid(entry.card.uid)
        {
            return Err(EngineRefusal::MalformedArgs(
                "physical move frozen identity",
            ));
        }
        catalog
            .spec(entry.card.atom)
            .ok_or(EngineRefusal::UnknownAtom(entry.card.atom))?;
        let located = unique_live_card_location(state, entry.card.uid)?;
        if located != Some((entry.source_pile, entry.source_index))
            || state.piles.get(entry.source_pile).as_slice()[entry.source_index] != entry.card
            || state.card_states.get(entry.card.uid) != entry.instance
        {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: entry.card.uid,
                pile: entry.source_pile,
            });
        }
    }

    let moved_uids = frozen
        .iter()
        .map(|entry| entry.card.uid)
        .collect::<Vec<_>>();
    let mut retained_piles = Vec::with_capacity(PileId::ALL.len());
    for pile in PileId::ALL {
        let source = state.piles.get(pile).as_slice();
        let retained = source
            .iter()
            .copied()
            .filter(|card| !moved_uids.contains(&card.uid))
            .collect::<Vec<_>>();
        if retained.len()
            != source.len()
                - frozen
                    .iter()
                    .filter(|entry| entry.source_pile == pile)
                    .count()
        {
            return Err(EngineRefusal::MalformedArgs(
                "physical move removal cardinality",
            ));
        }
        retained_piles.push((pile, retained));
    }
    for (pile, retained) in retained_piles {
        *state.piles.get_mut(pile).make_mut() = retained;
    }
    state
        .piles
        .get_mut(destination)
        .make_mut()
        .extend(frozen.iter().map(|entry| entry.card));
    for entry in frozen {
        events.push(Event::CardResolved {
            uid: entry.card.uid,
            pile: destination,
        });
    }
    Ok(())
}

/// Run one singular native `CardPileCmd::Add(card, Hand, Bottom)` command.
///
/// `CardPileCmd/<Add>d__10::MoveNext` RVA `0x3e1ba4` rechecks combat ending,
/// redirects a full Hand to the owner's Discard immediately before the move,
/// removes the retained `CardModel` from its live current pile, and appends it
/// to the selected destination. The current `AfterCardChangedPiles` census is
/// the same refused three-listener set documented by
/// [`move_frozen_physical_cards_to_bottom`], so an ordered `CardResolved` is
/// the complete admitted observable boundary. In particular, a full-Hand
/// Discard-to-Discard move removes and re-appends the exact card without
/// dispatching that type-change-only hook.
pub(crate) fn add_live_card_to_hand_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let (source, index) =
        unique_live_card_location(state, uid)?.ok_or(EngineRefusal::ContinuationNotModeled)?;
    let card = state.piles.get(source).as_slice()[index];
    if card.flags & CARD_FLAG_LEGACY != 0 || card.uid >= state.next_card_uid {
        return Err(EngineRefusal::MalformedArgs(
            "CardPileCmd::Add requires physical card identity",
        ));
    }
    catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    let destination = if state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
        PileId::Discard
    } else {
        PileId::Hand
    };
    let moved = state.piles.get_mut(source).make_mut().remove(index);
    state.piles.get_mut(destination).make_mut().push(moved);
    if source != destination {
        complete_inert_after_card_changed_piles(state, uid, destination)?;
    }
    events.push(Event::CardResolved {
        uid,
        pile: destination,
    });
    Ok(())
}

/// Explicit synchronous boundary for the currently inert pile-change hook.
///
/// Keeping this call between Aggression's awaited Add and Upgrade is
/// load-bearing: a future admitted listener can mutate the retained card or
/// end combat, at which point this boundary must become a resumable dispatch
/// and the caller must re-read that live result exactly as native does.
fn complete_inert_after_card_changed_piles(
    state: &HotState,
    uid: u32,
    destination: PileId,
) -> Result<(), EngineRefusal> {
    if unique_live_card_location(state, uid)?.map(|(pile, _)| pile) != Some(destination) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

const ALL_CARD_PILES: [PileId; 5] = [
    PileId::Discard,
    PileId::Draw,
    PileId::Exhaust,
    PileId::Hand,
    PileId::Play,
];

/// Native `AllCards` traversal used by card-affliction powers.
pub(crate) const CARD_AFFLICTION_PILES: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

pub(crate) const CARD_AFFLICTION_FLAGS: u16 = CARD_FLAG_BOUND | CARD_FLAG_HEXED | CARD_FLAG_RINGING;

/// Native `AllCards` order used by DampenPower's one-time snapshot.
pub(crate) const DAMPEN_PILES: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

fn dampen_turn_two_monsters_are_exact(state: &HotState, allow_dead_magi: bool) -> bool {
    let [flail, spectral, magi] = state.monsters.as_slice() else {
        return false;
    };
    // The three fixed HPs at the fight's ascension (`MONSTER_MODELS`, #2539).
    let (Some(flail_hp), Some(spectral_hp), Some(magi_hp)) = (
        super::monsters::native_fixed_hp(state, MonsterKind::FlailKnight),
        super::monsters::native_fixed_hp(state, MonsterKind::SpectralKnight),
        super::monsters::native_fixed_hp(state, MonsterKind::MagiKnight),
    ) else {
        return false;
    };
    let mut expected_flail = HotMonster::new(MonsterKind::FlailKnight, flail_hp);
    expected_flail.max_hp = flail_hp;
    if !expected_flail.random_ai.set_next(Some(0)) || !expected_flail.random_ai.set_log(&[2, 0]) {
        return false;
    }
    let mut expected_spectral = HotMonster::new(MonsterKind::SpectralKnight, spectral_hp);
    expected_spectral.max_hp = spectral_hp;
    expected_spectral.slot = 1;
    expected_spectral.uid = 1;
    if !expected_spectral.random_ai.set_next(Some(1))
        || !expected_spectral.random_ai.set_log(&[0, 1])
    {
        return false;
    }
    let mut expected_magi = HotMonster::new(MonsterKind::MagiKnight, magi_hp);
    expected_magi.max_hp = magi_hp;
    expected_magi.slot = 2;
    expected_magi.uid = 2;
    expected_magi.loop_pos = 1;
    expected_magi.block = magi.block;
    expected_magi.hp = magi.hp;
    (flail == &expected_flail)
        && (spectral == &expected_spectral)
        && magi == &expected_magi
        && (0..=9).contains(&magi.block)
        && if allow_dead_magi {
            magi.hp <= magi.max_hp
        } else {
            (1..=magi.max_hp).contains(&magi.hp)
        }
}

fn flail_knight_ai_is_exact(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::FlailKnight
        && !monster.random_ai.rat_spawn_fresh()
        && monster.random_ai.rat_call_for_backup_count() == 0
        && super::admission::valid_random_ai_state(monster, 3)
}

fn spectral_knight_ai_is_exact(monster: &HotMonster) -> bool {
    let len = monster.random_ai.log_len();
    if monster.kind != MonsterKind::SpectralKnight
        || monster.random_ai.once_len() != 0
        || monster.random_ai.rat_spawn_fresh()
        || monster.random_ai.rat_call_for_backup_count() != 0
        || !(1..=3).contains(&len)
        || monster.random_ai.next() != monster.random_ai.log_at(len - 1)
    {
        return false;
    }
    for position in 0..len {
        let Some(current) = monster.random_ai.log_at(position) else {
            return false;
        };
        if current > 2
            || position > 0
                && !matches!(
                    (monster.random_ai.log_at(position - 1), current),
                    (Some(0), 1) | (Some(1), 1) | (Some(1), 2) | (Some(2), 1)
                )
        {
            return false;
        }
    }
    len == 3 || monster.random_ai.log_at(0) == Some(0)
}

/// Complete fixed KnightsElite roster/AI provenance shared by live Hex and
/// Dampen. Combat scalars remain live: only construction, generated-machine
/// state and unsupported private move-state carriers are frozen here.
fn knights_roster_is_exact(state: &HotState, allow_dead_magi: bool) -> bool {
    let [flail, spectral, magi] = state.monsters.as_slice() else {
        return false;
    };
    if state.multiplayer_ally_key != 0
        || (flail.kind, spectral.kind, magi.kind)
            != (
                MonsterKind::FlailKnight,
                MonsterKind::SpectralKnight,
                MonsterKind::MagiKnight,
            )
        || [
            (flail.slot, flail.uid, Some(flail.max_hp)),
            (spectral.slot, spectral.uid, Some(spectral.max_hp)),
            (magi.slot, magi.uid, Some(magi.max_hp)),
        ] != [
            (
                0,
                0,
                super::monsters::native_fixed_hp(state, MonsterKind::FlailKnight),
            ),
            (
                1,
                1,
                super::monsters::native_fixed_hp(state, MonsterKind::SpectralKnight),
            ),
            (
                2,
                2,
                super::monsters::native_fixed_hp(state, MonsterKind::MagiKnight),
            ),
        ]
        || flail.hp > flail.max_hp
        || spectral.hp > spectral.max_hp
        || magi.hp > magi.max_hp
        || (!allow_dead_magi && magi.hp <= 0 && !state.fanouts.monster_death_is_pending(magi.uid))
        || flail.loop_pos != 0
        || spectral.loop_pos != 0
        || !(0..=4).contains(&magi.loop_pos)
        || !flail_knight_ai_is_exact(flail)
        || !spectral_knight_ai_is_exact(spectral)
        || !magi.random_ai.is_empty()
    {
        return false;
    }
    [flail, spectral, magi].into_iter().all(|monster| {
        monster.override_state == crate::hot::MonsterOverride::None
            && monster.forced_follow_up == crate::hot::MonsterFollowUp::None
            && !monster.spawn_noop
            && monster.revive_stage == 0
    })
}

fn no_hex_state_is_exact(state: &HotState) -> bool {
    state.powers.value(PowerId::HexPower) == 0
        && CARD_AFFLICTION_PILES.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.flags & CARD_FLAG_HEXED == 0)
        })
}

fn hex_card_state_is_exact(state: &HotState) -> bool {
    state.powers.value(PowerId::HexPower) == 2
        && state.powers.value(PowerId::ChainsOfBinding) == 0
        && !state.ringing()
        && state.exact_piles
        && CARD_AFFLICTION_PILES.into_iter().all(|pile| {
            state.piles.get(pile).as_slice().iter().all(|card| {
                card.flags & CARD_FLAG_LEGACY == 0
                    && card.flags & CARD_AFFLICTION_FLAGS == CARD_FLAG_HEXED
                    && card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0
                    && PileId::ALL
                        .into_iter()
                        .flat_map(|candidate| state.piles.get(candidate).as_slice())
                        .filter(|candidate| candidate.uid == card.uid)
                        .count()
                        == 1
            })
        })
}

fn dampen_card_collection_shape_is_exact(state: &HotState) -> bool {
    let mut seen = Vec::new();
    for pile in DAMPEN_PILES {
        for card in state.piles.get(pile).as_slice() {
            if card.flags & CARD_FLAG_LEGACY != 0
                || card.uid >= state.next_card_uid
                || seen.contains(&card.uid)
            {
                return false;
            }
            seen.push(card.uid);
        }
    }
    true
}

fn dampen_card_collection_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    dampen_card_collection_shape_is_exact(state)
        && DAMPEN_PILES.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| catalog.spec(card.atom).is_some())
        })
}

/// Exact turn-two Magi DAMPEN entry used only by the narrow public witness.
pub(crate) fn dampen_fresh_entry_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    state.card_states.dampen().is_none()
        && state.card_states.hopper().is_none()
        && state.multiplayer_ally_key == 0
        && !state.history.over
        && state.exact_piles
        && dampen_turn_two_monsters_are_exact(state, false)
        && hex_power_state_is_exact(state)
        && dampen_card_collection_is_exact(state, catalog)
        && DAMPEN_PILES.into_iter().all(|pile| {
            state.piles.get(pile).as_slice().iter().all(|card| {
                let Some(spec) = catalog.spec(card.atom) else {
                    return false;
                };
                spec.identity.upgrade == 0
                    || (spec.identity.upgrade == 1
                        && catalog
                            .atom(&crate::catalog::CardIdentity {
                                id: spec.identity.id,
                                upgrade: 0,
                                enchantment: spec.identity.enchantment,
                            })
                            .is_some())
            })
        })
}

/// DampenPower.AfterApplied, v111 RVA 0xa1198: Magi's second move downgrades the current
/// cards, regardless of damage/debuffs dealt to the roster before its action.
/// A killed Spectral has already removed Hex; both exact states are valid.
pub(crate) fn dampen_application_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    state.card_states.dampen().is_none()
        && state.card_states.hopper().is_none()
        && !state.history.over
        && state.exact_piles
        && knights_roster_is_exact(state, false)
        && state.monsters[2].loop_pos == 1
        && if state.monsters[1].hp > 0 {
            hex_card_state_is_exact(state)
        } else {
            no_hex_state_is_exact(state)
        }
        && dampen_card_collection_is_exact(state, catalog)
}

/// SpectralKnight.HexMove (v111 MoveNext RVA 0x36c53c) applies Hex to
/// the player even after player actions changed the other Knights' scalars.
/// The separate public smoke witness retains its original fresh-state gate.
///
/// The live gate does not require `exact_piles` beforehand (#2959):
/// [`apply_hex_power`] is the promotion, see
/// [`hex_card_affliction_application_is_exact`].
pub(crate) fn spectral_hex_application_is_exact(state: &HotState) -> bool {
    !state.history.over
        && knights_roster_is_exact(state, true)
        && state.monsters[1].hp > 0
        && state.monsters[1].random_ai.next() == Some(0)
        && hex_card_affliction_application_is_exact(state)
}

/// Authenticate the complete live Dampen caster/card snapshot.
pub(crate) fn dampen_state_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let Some(dampen) = state.card_states.dampen() else {
        return false;
    };
    if dampen.caster_uid != 2
        || state.card_states.hopper().is_some()
        || !knights_roster_is_exact(state, false)
        || if state.monsters[1].hp > 0 {
            !hex_card_state_is_exact(state)
        } else {
            !no_hex_state_is_exact(state)
        }
        || !dampen_card_collection_is_exact(state, catalog)
        || dampen.cards.iter().enumerate().any(|(position, row)| {
            row.old_level != 1
                || dampen.cards[..position]
                    .iter()
                    .any(|earlier| earlier.uid == row.uid)
        })
    {
        return false;
    }
    dampen.cards.iter().all(|row| {
        let mut found = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .filter(|card| card.uid == row.uid);
        let Some(card) = found.next() else {
            return false;
        };
        if found.next().is_some() {
            return false;
        }
        let Some(live) = catalog.spec(card.atom) else {
            return false;
        };
        let Some(old) = catalog.spec(row.old_atom) else {
            return false;
        };
        let Some(dampened) = catalog.spec(row.dampened_atom) else {
            return false;
        };
        live.identity.id == old.identity.id
            && live.identity.enchantment == old.identity.enchantment
            && matches!(live.identity.upgrade, 0 | 1)
            && old.identity.upgrade == row.old_level
            && row.old_level == 1
            && dampened.identity
                == crate::catalog::CardIdentity {
                    id: old.identity.id,
                    upgrade: 0,
                    enchantment: old.identity.enchantment,
                }
    })
}

/// Structural Dampen proof used only below an already catalog-authenticated
/// public command. Both catalog atoms are frozen in each row, so later native
/// L0/L1 upgrades remain distinguishable without reopening catalogless entry.
pub(crate) fn dampen_internal_state_is_exact(state: &HotState) -> bool {
    let Some(dampen) = state.card_states.dampen() else {
        return false;
    };
    if dampen.caster_uid != 2
        || !knights_roster_is_exact(state, false)
        || if state.monsters[1].hp > 0
            || state
                .fanouts
                .monster_death_is_pending(state.monsters[1].uid)
        {
            !hex_card_state_is_exact(state)
        } else {
            !no_hex_state_is_exact(state)
        }
        || !dampen_card_collection_shape_is_exact(state)
        || dampen.cards.iter().enumerate().any(|(position, row)| {
            row.old_level != 1
                || row.old_atom == row.dampened_atom
                || dampen.cards[..position]
                    .iter()
                    .any(|earlier| earlier.uid == row.uid)
        })
    {
        return false;
    }
    dampen.cards.iter().all(|row| {
        let mut found = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .filter(|card| card.uid == row.uid);
        let Some(card) = found.next() else {
            return false;
        };
        found.next().is_none()
            && matches!(card.atom, atom if atom == row.dampened_atom || atom == row.old_atom)
    })
}

/// DampenPower.AfterRemoved (v111 0xa1290) upgrades retained CardModel
/// references; CardCmd.Upgrade (0x12f660) mutates that card, with no combat
/// hook broadcast. A resolved Power is no longer in any combat pile and
/// cannot reenter, so its private old-level row has no future combat reader.
/// Keep rows for all surviving cards, including Exhaust; only committed
/// Remove/Bottom routing and Primal Force's transform (whose original model
/// `CardCmd.Transform` removes from state, #2982) call this quotient
/// operation.
pub(crate) fn forget_removed_dampen_card(state: &mut HotState, uid: u32) {
    if let Some(mut dampen) = state.card_states.dampen().cloned() {
        dampen.cards.retain(|row| row.uid != uid);
        state.card_states.set_dampen(Some(dampen));
    }
}

pub(crate) fn dampen_tracks_uid(state: &HotState, uid: u32) -> bool {
    state
        .card_states
        .dampen()
        .is_some_and(|dampen| dampen.cards.iter().any(|row| row.uid == uid))
}

pub(crate) fn dampen_death_transient_is_exact(state: &HotState, target: usize) -> bool {
    let mut entry = state.clone();
    let Some(dying) = entry.monsters_mut().get_mut(target) else {
        return false;
    };
    if !matches!(
        dying.kind,
        MonsterKind::FlailKnight | MonsterKind::SpectralKnight | MonsterKind::MagiKnight
    ) || dying.hp > 0
    {
        return false;
    }
    dying.hp = 1;
    dampen_internal_state_is_exact(&entry)
}

pub(crate) fn dampen_exhaust_entry_is_exact(
    state: &HotState,
    catalog: &Catalog,
    card: HotCard,
) -> bool {
    let Some(dampen) = state.card_states.dampen() else {
        return true;
    };
    if DAMPEN_PILES
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .any(|live| live.uid == card.uid)
    {
        return false;
    }
    if let Some(row) = dampen.cards.iter().find(|row| row.uid == card.uid)
        && !matches!(card.atom, atom if atom == row.dampened_atom || atom == row.old_atom)
    {
        return false;
    }
    let mut prospective = state.clone();
    prospective
        .piles
        .get_mut(PileId::Exhaust)
        .make_mut()
        .push(card);
    dampen_state_is_exact(&prospective, catalog)
}

/// Apply Dampen once, atomically replacing every current L1 atom with L0.
pub(crate) fn apply_dampen_power(
    state: &mut HotState,
    catalog: &Catalog,
    caster_uid: u32,
) -> Result<(), EngineRefusal> {
    if let Some(active) = state.card_states.dampen() {
        return if active.caster_uid == caster_uid && dampen_state_is_exact(state, catalog) {
            Ok(())
        } else {
            Err(EngineRefusal::MalformedArgs("Dampen reapplication"))
        };
    }
    if caster_uid != 2 || !dampen_application_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs("Dampen application"));
    }
    let mut replacements = Vec::new();
    let mut rows = Vec::new();
    for pile in DAMPEN_PILES {
        for (index, card) in state.piles.get(pile).as_slice().iter().enumerate() {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.identity.upgrade == 0 {
                continue;
            }
            if spec.identity.upgrade != 1 {
                return Err(EngineRefusal::MalformedArgs("Dampen upgrade level"));
            }
            // #3413: `CardModel::DowngradeInternal` RVA `0x7e12c` resets
            // `_base` to `Canonical` (IL_0042) and re-runs `ModifyCard`
            // (IL_0077), so TezcatarasEmber's `OnEnchant` lowers the cost
            // 1 -> 0 through `UpgradeBy`'s local-modifier walk (RVA `0x11e31c`
            // IL_0041-0045), which this atom swap does not replay.
            if matches!(
                spec.identity.enchantment,
                Some(crate::catalog::CardEnchantment {
                    id: crate::ids::EnchantmentId::TezcatarasEmber,
                    ..
                })
            ) {
                return Err(EngineRefusal::MalformedArgs("Dampen over TEZCATARAS_EMBER"));
            }
            let atom = catalog
                .atom(&crate::catalog::CardIdentity {
                    id: spec.identity.id,
                    upgrade: 0,
                    enchantment: spec.identity.enchantment,
                })
                .ok_or(EngineRefusal::MalformedArgs("Dampen L0 counterpart"))?;
            replacements.push((pile, index, atom));
            rows.push(crate::hot::DampenCardRow {
                uid: card.uid,
                old_level: spec.identity.upgrade,
                old_atom: card.atom,
                dampened_atom: atom,
            });
        }
    }
    for (pile, index, atom) in replacements {
        state.piles.get_mut(pile).make_mut()[index].atom = atom;
    }
    state.card_states.set_dampen(Some(crate::hot::DampenState {
        caster_uid,
        cards: rows,
    }));
    Ok(())
}

/// Restore every tracked Dampen row after the exact caster's actual death.
///
/// **A combat-ending death removes the power but restores nothing (#3233).**
/// v0.111.0 `sts2.dll` SHA-256 `9cb4f1ad…fbf12b4`. The last caster's death
/// runs `DampenPower/<AfterDeath>d__7::MoveNext` (RVA `0x33871c`,
/// IL_004c-IL_006c: `casters.Remove`, then `PowerCmd.Remove(this)` once the
/// set is empty). `PowerCmd/<Remove>d__8::MoveNext` (RVA `0x3f09bc`) has no
/// ending gate: `RemoveInternal` at IL_0030, then `Cmd.CustomScaledWait`
/// (IL_0049; `<CustomScaledWait>d__3` RVA `0x3e8b64` IL_0043-IL_0057 only
/// returns early at `IsEnding`), then `AfterRemoved` at IL_00b4. So the power
/// always leaves the player. `DampenPower::AfterRemoved` (RVA `0xa1290`,
/// IL_003d-IL_004d) restores each row through `CardCmd.Upgrade(card, 1)`
/// (RVA `0x12f64f`, which forwards to the list overload), and that overload
/// (RVA `0x12f660`) returns at IL_000c-IL_0018 while
/// `CombatManager.Instance.IsEnding`. When the Magi Knight is the last enemy
/// to die, every restore is therefore a no-op and the cards stay at their
/// dampened level at the lethal checkpoint (native `fb2c70194650a7db`, the
/// Knights elite, step 28). The projection is the shared
/// [`super::damage::damage_combat_is_ending`]; the row preflight below still
/// runs in both cases, so a malformed snapshot refuses by name either way.
pub(crate) fn remove_dampen_after_death(
    state: &mut HotState,
    caster_uid: u32,
) -> Result<(), EngineRefusal> {
    let Some(active) = state.card_states.dampen().cloned() else {
        return Ok(());
    };
    if active.caster_uid != caster_uid {
        return Ok(());
    }
    if !knights_roster_is_exact(state, true)
        || state.monsters[2].hp > 0
        || if state.monsters[1].hp > 0 {
            !hex_card_state_is_exact(state)
        } else {
            !no_hex_state_is_exact(state)
        }
        || !dampen_card_collection_shape_is_exact(state)
    {
        return Err(EngineRefusal::MalformedArgs("Dampen death state"));
    }
    let mut replacements = Vec::with_capacity(active.cards.len());
    for row in &active.cards {
        let mut found = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| {
                state
                    .piles
                    .get(pile)
                    .as_slice()
                    .iter()
                    .enumerate()
                    .map(move |(index, card)| (pile, index, card))
            })
            .filter(|(_, _, card)| card.uid == row.uid);
        let Some((pile, index, card)) = found.next() else {
            return Err(EngineRefusal::MalformedArgs("Dampen tracked uid"));
        };
        if found.next().is_some() {
            return Err(EngineRefusal::MalformedArgs("Dampen duplicate uid"));
        }
        if !matches!(card.atom, atom if atom == row.dampened_atom || atom == row.old_atom)
            || row.old_level != 1
        {
            return Err(EngineRefusal::MalformedArgs("Dampen tracked atom"));
        }
        replacements.push((pile, index, row.old_atom));
    }
    // `CardCmd.Upgrade` (RVA `0x12f660` IL_000c-IL_0018) is a no-op while the
    // combat is ending; `PowerCmd.Remove` still drops the power.
    if !super::damage::damage_combat_is_ending(state) {
        for (pile, index, atom) in replacements {
            state.piles.get_mut(pile).make_mut()[index].atom = atom;
        }
    }
    state.card_states.set_dampen(None);
    Ok(())
}

pub(crate) fn fresh_card_affliction_application_is_exact(state: &HotState) -> bool {
    state.exact_piles && hex_card_affliction_application_is_exact(state)
}

/// The fresh card-affliction proof without the `exact_piles` precondition:
/// the gate for a HexPower application, which itself promotes the piles.
///
/// `exact_piles` is the solver's pile-identity promotion, not a native field.
/// Native `HexPower.AfterApplied` (v0.111.0 `<AfterApplied>d__7::MoveNext`,
/// RVA `0x33c498`, IL_001d-IL_00b3) enumerates
/// `PlayerCombatState.AllCards` and awaits `HexPower::Afflict` (RVA
/// `0xa38d8`) on every card; `<Afflict>d__11::MoveNext` (RVA `0x33c3b0`)
/// skips a card that already carries an affliction (IL_001d-IL_002a) and
/// otherwise calls `CardCmd.Afflict<Hexed>` with the power's Amount
/// (IL_002f-IL_0040). Nothing in that walk reads a card-group ordering
/// property, so the application needs no prior exact pile identity. The
/// oracle agrees: `_apply_hex_power` (frozen Python, deleted #2827) commits both
/// maps with `force_exact=True`, and `_commit_live_card_piles`
/// promotes a non-exact state there, gated only by
/// `_validate_exact_pile_safety`, which Rust-opening roots skip (#2952).
/// Ringing's application ([`apply_ringing_power`]) already promotes the same
/// way. Every other conjunct is kept: no live affliction power, no Bound
/// turn state, no legacy payloads, no afflicted card, unique UIDs.
///
/// Chains of Binding's application (`moves::boss::queen_puppet_strings`) takes
/// the same gate and promotes the same way since #3404; its IL is cited
/// there.
pub(crate) fn hex_card_affliction_application_is_exact(state: &HotState) -> bool {
    if state.ringing()
        || state.powers.value(PowerId::ChainsOfBinding) != 0
        || state.powers.value(PowerId::HexPower) != 0
        || state.bound_afflictions_this_turn() != 0
        || state.bound_card_played()
    {
        return false;
    }
    CARD_AFFLICTION_PILES.into_iter().all(|pile| {
        state.piles.get(pile).as_slice().iter().all(|card| {
            card.flags & CARD_FLAG_LEGACY == 0
                && card.flags & CARD_AFFLICTION_FLAGS == 0
                && PileId::ALL
                    .into_iter()
                    .flat_map(|candidate| state.piles.get(candidate).as_slice())
                    .filter(|candidate| candidate.uid == card.uid)
                    .count()
                    == 1
        })
    })
}

pub(crate) fn chains_power_state_is_exact(state: &HotState) -> bool {
    chains_state_is_exact(state, false)
}

/// Gremlin Horn's Draw runs inside AfterDeath, before Queen's monster-model
/// listener commits the Amalgam-dead latch. Keep stable admission strict,
/// but authenticate that existing internal death state at the draw callback.
pub(crate) fn chains_draw_state_is_exact(state: &HotState) -> bool {
    chains_state_is_exact(state, true)
}

fn chains_state_is_exact(state: &HotState, inside_draw: bool) -> bool {
    if state.multiplayer_ally_key != 0
        || !(if inside_draw {
            super::monsters::queen_roster_internal_state_is_valid(state)
        } else {
            super::monsters::queen_roster_state_is_valid(state)
        })
        || !matches!(state.monsters[1].loop_pos, 1..=5)
        || state.powers.value(PowerId::ChainsOfBinding) != 3
        || state.powers.value(PowerId::HexPower) != 0
        || state.ringing()
        || !state.exact_piles
        || !state.card_affliction_state_is_exact()
        || state.bound_card_played() && state.bound_afflictions_this_turn() == 0
        || state
            .fanouts
            .after_card_drawn_order()
            .iter()
            .filter(|power| **power == PowerId::ChainsOfBinding)
            .count()
            != 1
        || state
            .fanouts
            .before_side_turn_end_order()
            .iter()
            .filter(|token| matches!(token, crate::hot::BeforeSideTurnEndToken::ChainsOfBinding))
            .count()
            != 1
    {
        return false;
    }
    CARD_AFFLICTION_PILES.into_iter().all(|pile| {
        state.piles.get(pile).as_slice().iter().all(|card| {
            let affliction = card.flags & CARD_AFFLICTION_FLAGS;
            card.flags & CARD_FLAG_LEGACY == 0
                && (affliction == 0 || card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0)
                && matches!(affliction, 0 | CARD_FLAG_BOUND)
                && (affliction == 0 || state.bound_afflictions_this_turn() > 0)
                && PileId::ALL
                    .into_iter()
                    .flat_map(|candidate| state.piles.get(candidate).as_slice())
                    .filter(|candidate| candidate.uid == card.uid)
                    .count()
                    == 1
        })
    })
}

pub(crate) fn effective_ethereal(
    state: &HotState,
    spec: &crate::catalog::CardSpec,
    uid: u32,
) -> bool {
    spec.ethereal
        || state.card_states.get(uid).local_ethereal()
        || (state.powers.value(PowerId::HexPower) == 2
            && unique_live_card_location(state, uid)
                .ok()
                .flatten()
                .is_some_and(|(pile, index)| {
                    state.piles.get(pile).as_slice()[index].flags & CARD_FLAG_HEXED != 0
                }))
}

/// Whether `CardKeyword.Retain` is in this exact instance's effective keyword
/// union.
///
/// Python: `card_is_retained` (frozen, deleted #2827) is
/// `CARD_KEYWORD_RETAIN in card_effective_keywords(card)`.
///
/// `card_effective_keywords` contributes Retain from exactly two places — the
/// canonical row plus any local/transient physical keyword (frozen Python, deleted #2827), and
/// the `STEADY` arm of its keyword-delta branch, whose payload
/// `_keyword_delta_enchantment` has already validated to the exact amount 1.
///
/// `card_is_retained` has exactly two live readers.
/// `_finish_player_turn_after_wrappers` (frozen Python, deleted #2827) is the end-of-turn hand flush,
/// which is `engine::draw`'s `flush_hand` here; `_select_candidates` is
/// the without-Retain selection filter, which is `engine::selection`'s
/// [`crate::ids::FilterMode::WithoutEffectiveRetain`] arm.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Enchantments.Steady::OnEnchant` RVA `0xd65b4` calls
/// `CardModel.AddKeyword(5)` and Steady declares nothing else, so a Steady
/// card is Retain-bearing from the moment it is enchanted and stays so through
/// every pile move — deriving the bit from slot 2 rather than storing it is
/// what makes that free, and is why this is not a
/// [`crate::hot::CardInstanceState`] field.
///
/// `ROYALLY_APPROVED` (#3178) reaches this reader through `spec.retain`:
/// `RoyallyApproved::OnEnchant` RVA `0xd62c9` IL_0013-0014 is
/// `Card.AddKeyword(5)`, which the catalog folds into the compiled spec
/// ([`crate::catalog::royally_approved_adds_innate_and_retain`]).
pub(crate) fn effective_retain(
    state: &HotState,
    spec: &crate::catalog::CardSpec,
    uid: u32,
) -> bool {
    spec.retain
        || matches!(
            spec.identity.enchantment,
            Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Steady,
                amount: 1,
            })
        )
        || state
            .card_states
            .get_ref(uid)
            .is_some_and(|instance| instance.local_retain || instance.transient_retain)
}

/// Exact fresh A8+ Spectral HEX entry used by the narrow public move witness.
///
/// This is deliberately stricter than the live HexPower predicate. Spectral's
/// opener runs after Flail's fixed RAM opener; Flail therefore
/// retains RAM as its one-entry random-AI state while every self-owned scalar,
/// power, and private field remains at construction state. Keeping the whole
/// three-object construction here prevents the smoke-only catalog from
/// authenticating HEX solely from Spectral's one random-table index.
pub(crate) fn spectral_hex_fresh_entry_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    if state.multiplayer_ally_key != 0
        || state.history.over
        || !fresh_card_affliction_application_is_exact(state)
        || CARD_AFFLICTION_PILES.into_iter().any(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .any(|card| catalog.spec(card.atom).is_none())
        })
    {
        return false;
    }

    // The three fixed HPs at the fight's ascension (`MONSTER_MODELS`, #2539).
    let (Some(flail_hp), Some(spectral_hp), Some(magi_hp)) = (
        super::monsters::native_fixed_hp(state, MonsterKind::FlailKnight),
        super::monsters::native_fixed_hp(state, MonsterKind::SpectralKnight),
        super::monsters::native_fixed_hp(state, MonsterKind::MagiKnight),
    ) else {
        return false;
    };
    let mut flail = crate::hot::HotMonster::new(crate::ids::MonsterKind::FlailKnight, flail_hp);
    flail.max_hp = flail_hp;
    if !flail.random_ai.set_next(Some(2)) || !flail.random_ai.set_log(&[2]) {
        return false;
    }
    let mut spectral =
        crate::hot::HotMonster::new(crate::ids::MonsterKind::SpectralKnight, spectral_hp);
    spectral.max_hp = spectral_hp;
    spectral.slot = 1;
    spectral.uid = 1;
    if !spectral.random_ai.set_next(Some(0)) || !spectral.random_ai.set_log(&[0]) {
        return false;
    }
    let mut magi = crate::hot::HotMonster::new(crate::ids::MonsterKind::MagiKnight, magi_hp);
    magi.max_hp = magi_hp;
    magi.slot = 2;
    magi.uid = 2;
    state.monsters.as_slice() == [flail, spectral, magi]
}

/// Authenticate the only represented combat-local Ethereal writer.
///
/// Call of the Void installs the marker on generated physical cards and its
/// persistent positive power retains the owner/pool provenance for every
/// surviving card. Upgrade and clone effects may raise the generated L0 row to
/// L1 or copy it under a fresh UID; transforms allocate fresh physical state.
/// The exact survivor envelope is therefore one unique physical-state card,
/// an unenchanted L0/L1 identity from the frozen 78-card pool, and the complete
/// live Call writer/catalog provenance.
///
/// Sculpting Strike (#3022) is a third writer: its OnPlay
/// (`SculptingStrike/<OnPlay>d__7::MoveNext` RVA `0x3b8cec`, IL_0187/IL_0189)
/// applies Ethereal to ANY chosen Hand card, so while a Sculpting Strike L0/L1
/// is physically present in a combat pile the envelope is only the unique
/// live physical card. A catalog-closure member alone (for example one of
/// Call's generated pool) is not a writer: it has to exist to have been
/// played, and a marked card whose writer has since left every pile refuses
/// by name rather than being guessed at.
///
/// Music Box (#3551) is a fourth writer. `MusicBox::BeforeCardPlayed` RVA
/// `0x972ac` arms on the owner's first Attack of the turn (`Type == 1`,
/// IL_0041-004d), and `MusicBox/<AfterCardPlayed>d__13::MoveNext` RVA
/// `0x32ae04` clones that card (`CreateClone` IL_0046), applies keyword 2
/// (Ethereal, IL_004c-0057) and adds the clone to Hand (IL_005c-0065). The
/// clone keeps the source's upgrade and enchantment, and a played clone
/// carries its marker for the rest of combat, so while the relic is owned the
/// envelope is any unique live physical Attack.
pub(crate) fn call_local_ethereal_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let mut found = false;
    let call_writer_is_exact = state.powers.value(PowerId::CallOfTheVoid) > 0
        && owner_listener_provenance_is_exact(state, catalog)
        && crate::boundary::call_of_the_void_catalog_closure_is_exact(
            catalog,
            state.reward_card_pool,
        );
    // This validator runs on every legal-action/transition preflight. Only
    // a live, authenticated Call listener needs its generated-card pool.
    let call_pool = if call_writer_is_exact {
        catalog.owner_listener_pool(state.reward_card_pool.unwrap(), PowerId::CallOfTheVoid)
    } else {
        Vec::new()
    };
    let ghost_writer = catalog.hooks().owns(crate::ids::RelicId::RelicGhostSeed);
    let music_box_writer = catalog.hooks().owns(crate::ids::RelicId::RelicMusicBox);
    // Computed only once a marked card is found: this runs on every preflight.
    let mut sculpting_writer: Option<bool> = None;
    for (uid, instance) in state.card_states.as_slice() {
        if !instance.local_ethereal() {
            continue;
        }
        found = true;
        let sculpting_writer = *sculpting_writer.get_or_insert_with(|| {
            ALL_CARD_PILES
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .any(|card| {
                    catalog.spec(card.atom).is_some_and(|spec| {
                        spec.identity.id == CardId::SculptingStrike && spec.identity.upgrade <= 1
                    })
                })
        });
        let mut live = ALL_CARD_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .filter(|card| card.uid == *uid);
        let exact_owner = match (live.next(), live.next()) {
            (Some(card), None) if sculpting_writer => catalog.spec(card.atom).is_some(),
            (Some(card), None) => catalog.spec(card.atom).is_some_and(|spec| {
                music_box_writer && spec.is_attack
                    || spec.identity.upgrade <= 1
                        && spec.identity.enchantment.is_none()
                        && ((call_writer_is_exact
                            && card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0
                            && call_pool.contains(&spec.identity.id))
                            || (ghost_writer
                                && matches!(
                                    spec.identity.id,
                                    CardId::StrikeIronclad
                                        | CardId::StrikeSilent
                                        | CardId::StrikeDefect
                                        | CardId::StrikeNecrobinder
                                        | CardId::StrikeRegent
                                        | CardId::DefendIronclad
                                        | CardId::DefendSilent
                                        | CardId::DefendDefect
                                        | CardId::DefendNecrobinder
                                        | CardId::DefendRegent
                                )))
            }),
            _ => false,
        };
        if !exact_owner {
            return false;
        }
    }
    !found
        || call_writer_is_exact
        || ghost_writer
        || music_box_writer
        || sculpting_writer == Some(true)
}

#[cfg(test)]
thread_local! {
    static SWORD_SAGE_DELTA_TRACE: std::cell::RefCell<Vec<(PileId, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static ROCKET_PUNCH_GENERATED_TRACE: std::cell::RefCell<Vec<u32>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static GENERATED_LISTENER_TRACE: std::cell::RefCell<Vec<GeneratedListenerTrace>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeneratedListenerTrace {
    Power(PowerId),
    RocketPunch(u32),
}

/// Current-build `CardModel.MaxUpgradeLevel`, independent of modeled rows.
///
/// The current-build generated rows cover every native upgrade level.
/// The complete independent template census below pins this equality.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn native_max_upgrade(id: CardId) -> Option<u8> {
    crate::content_tables::CARD_ROWS
        .iter()
        .filter(|row| row.id == id)
        .map(|row| row.upgrade)
        .max()
}

pub(crate) fn native_card_is_upgradable(identity: CardIdentity) -> bool {
    identity
        .upgrade
        .checked_add(1)
        .is_some_and(|next| crate::content_tables::card_row(identity.id, next).is_some())
}

/// `combat_sim._DIVERGENT_PHYSICAL_CARD_IDS`: card classes whose equal fresh
/// copies can still diverge on draw, play or `BeforeFlush`, so a group of two
/// or more needs exact pile order up front.
///
/// One list shared by the live promotion ([`card_slice_has_distinguishable_sort_ties`])
/// and the opening's combat-start seed (`entry::opening`), so the two cannot
/// drift apart.
pub(crate) fn divergent_physical_card(id: CardId) -> bool {
    matches!(
        id,
        CardId::KinglyKick
            | CardId::KinglyPunch
            | CardId::BansheesCry
            | CardId::Flatten
            | CardId::Melancholy
            | CardId::Midnight
            | CardId::Pinpoint
            | CardId::UpMySleeve
            | CardId::Enlightenment
            | CardId::BulletTime
            | CardId::Modded
            | CardId::RocketPunch
            | CardId::Stomp
            | CardId::Claw
            | CardId::GeneticAlgorithm
            | CardId::Maul
            | CardId::MomentumStrike
            | CardId::Rampage
            | CardId::TheBall
            | CardId::TheScythe
            | CardId::Thrash
            | CardId::Wither
            | CardId::Bolas
            | CardId::ThrummingHatchet
            | CardId::MadScience
    )
}

/// `combat_sim._DIVERGENT_IDENTITY_ENCHANTMENTS`: stateful enchantments whose
/// payload flips after one physical copy plays, so even identical enchanted
/// group-mates become distinguishable. Shared like [`divergent_physical_card`].
pub(crate) fn divergent_identity_enchantment(id: EnchantmentId) -> bool {
    matches!(
        id,
        EnchantmentId::Glam
            | EnchantmentId::Sown
            | EnchantmentId::Swift
            | EnchantmentId::Momentum
            | EnchantmentId::Vigorous
            | EnchantmentId::SlumberingEssence
            | EnchantmentId::Goopy
            | EnchantmentId::Slither
    )
}

/// Whether Python's `_has_distinguishable_sort_ties` would promote the five
/// live piles after a physical-card payload write.
///
/// Native `CardModel.CompareTo` keys only `(id, upgrade)`. Equal-key copies
/// are distinguishable when their complete represented payload differs, or
/// when a modeled card/enchantment class can make initially equal physical
/// copies diverge later. Payload writers that need the complete Python gate
/// share this predicate so a new represented dimension cannot leave one of
/// those call sites with a weaker projection.
pub(crate) fn live_cards_need_exact_piles(
    state: &HotState,
    catalog: &Catalog,
) -> Result<bool, EngineRefusal> {
    let cards = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .copied()
        .collect::<Vec<_>>();
    card_slice_has_distinguishable_sort_ties(state, catalog, &cards)
}

/// The same CompareTo-tie proof over one concrete Shuffle domain.
pub(crate) fn card_slice_has_distinguishable_sort_ties(
    state: &HotState,
    catalog: &Catalog,
    cards: &[HotCard],
) -> Result<bool, EngineRefusal> {
    for (index, left) in cards.iter().enumerate() {
        let left_identity = catalog
            .spec(left.atom)
            .ok_or(EngineRefusal::UnknownAtom(left.atom))?
            .identity;
        for right in &cards[index + 1..] {
            let right_identity = catalog
                .spec(right.atom)
                .ok_or(EngineRefusal::UnknownAtom(right.atom))?
                .identity;
            if left_identity.id != right_identity.id
                || left_identity.upgrade != right_identity.upgrade
            {
                continue;
            }
            let divergent_card = divergent_physical_card(left_identity.id);
            let divergent_enchantment = left_identity
                .enchantment
                .is_some_and(|value| divergent_identity_enchantment(value.id));
            if divergent_card
                || divergent_enchantment
                || left_identity != right_identity
                || left.flags != right.flags
                || state.card_states.get_ref(left.uid) != state.card_states.get_ref(right.uid)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Apply one native `CardCmd::Upgrade` pass to a frozen physical-card list.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `CardCmd::Upgrade(IEnumerable<CardModel>, 1)` RVA `0x12f660` gates on
/// combat ending,
/// rechecks `IsUpgradable` for every frozen entry in order, and then runs the
/// card's one-level upgrade/finalize transaction without firing a hook.
/// `CardEnergyCost.UpgradeBy` RVA `0x11e31c` preserves local rows in order and
/// clamps only `Set` operands above a lowered printed base.
///
/// The caller supplies the native enumeration's immutable uid snapshot. This
/// helper resolves every live identity and every next atom on a cloned state,
/// so a missing/duplicated physical card or incomplete catalog closure cannot
/// publish a partial prefix. Atom replacement preserves the uid, flags, and
/// every other per-instance payload; the shared tie predicate promotes exact
/// pile order only when the upgraded result makes it observable.
pub(crate) fn upgrade_live_cards_once(
    state: &mut HotState,
    catalog: &Catalog,
    frozen_uids: &[u32],
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    upgrade_live_cards_once_ungated(state, catalog, frozen_uids)
}

/// The same exact physical upgrade transaction without the ending gate.
///
/// No native caller reaches an ungated upgrade: `CardCmd::Upgrade` RVA
/// `0x12f660` returns at `CombatManager.IsEnding` (IL_000c-IL_0018) before
/// its `IsUpgradable` walk, whatever the caller (#3303). This entry is for a
/// rehearsal on a cloned state that validates the transaction's catalog
/// closure, and for a caller that has already applied the IsEnding
/// projection [`super::damage::damage_combat_is_ending`] itself (Drain Power,
/// whose Selection draw precedes its per-card Upgrade calls).
pub(crate) fn upgrade_live_cards_once_ungated(
    state: &mut HotState,
    catalog: &Catalog,
    frozen_uids: &[u32],
) -> Result<(), EngineRefusal> {
    for (index, uid) in frozen_uids.iter().enumerate() {
        if frozen_uids[..index].contains(uid) {
            return Err(EngineRefusal::MalformedArgs(
                "CardCmd::Upgrade duplicate frozen identity",
            ));
        }
    }

    let mut upgraded = state.clone();
    for &uid in frozen_uids {
        let (pile, index) = unique_live_card_location(&upgraded, uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let card = upgraded.piles.get(pile).as_slice()[index];
        if card.flags & CARD_FLAG_LEGACY != 0 {
            return Err(EngineRefusal::MalformedArgs(
                "CardCmd::Upgrade requires physical card identity",
            ));
        }
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if !native_card_is_upgradable(spec.identity) {
            continue;
        }
        let next_level = spec
            .identity
            .upgrade
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("CardCmd::Upgrade level"))?;
        let next_identity = CardIdentity {
            id: spec.identity.id,
            upgrade: next_level,
            enchantment: spec.identity.enchantment,
        };
        let next_atom = catalog
            .atom(&next_identity)
            .ok_or(EngineRefusal::UnknownMintIdentity(next_identity))?;
        let next_spec = catalog
            .spec(next_atom)
            .ok_or(EngineRefusal::UnknownAtom(next_atom))?;
        if next_spec.cost < spec.cost {
            let mut instance = upgraded.card_states.get(uid);
            instance
                .local_cost_modifiers
                .clamp_sets_above(next_spec.cost);
            upgraded.card_states.set(uid, instance);
        }
        // A Thieving Hopper master row is the card's `DeckVersion` object,
        // and the combat upgrade never reaches it (#2965): `CardCmd::Upgrade`
        // RVA `0x12f660` calls `UpgradeInternal`/`FinalizeUpgradeInternal` on
        // the combat object alone (IL_0080-IL_0087), and neither body
        // (`0x7e0c8`, `0x7e10c`) reads `DeckVersion`. The only `DeckVersion`
        // readers in the DLL (`scan_calls.py get_DeckVersion set_DeckVersion`)
        // are Swipe, Thievery's filter, Goopy, `AfterCloned`,
        // `PopulateCombatState` and three `OnPlay` bodies (Genetic Algorithm,
        // The Scythe, and a draw-then-`RemoveFromDeck` `<OnPlay>d__5`), none
        // on an upgrade path. So the master row keeps its pre-combat level.
        upgraded.piles.get_mut(pile).make_mut()[index].atom = next_atom;
    }
    if !upgraded.exact_piles && live_cards_need_exact_piles(&upgraded, catalog)? {
        upgraded.exact_piles = true;
    }
    *state = upgraded;
    Ok(())
}

/// RingingPower.AfterApplied (`_apply_ringing_power`, frozen Python, deleted #2827).
///
/// Native first validates the encounter-disjoint affliction powers, then
/// marks every current owner card through one force-exact all-piles commit.
/// The modeled Bound and Hexed owners are disjoint peers, so all power and
/// five-pile overlap checks precede the first write.
pub(crate) fn apply_ringing_power(state: &mut HotState) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    if state.ringing()
        || state.powers.value(PowerId::ChainsOfBinding) > 0
        || state.powers.value(PowerId::HexPower) > 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "overlapping card-affliction powers",
        ));
    }
    if CARD_AFFLICTION_PILES.iter().any(|pile| {
        state
            .piles
            .get(*pile)
            .as_slice()
            .iter()
            .any(|card| card.flags & CARD_FLAG_LEGACY != 0)
    }) {
        return Err(EngineRefusal::MalformedArgs(
            "Ringing requires physical card identities",
        ));
    }
    if CARD_AFFLICTION_PILES.iter().any(|pile| {
        state
            .piles
            .get(*pile)
            .as_slice()
            .iter()
            .any(|card| card.flags & CARD_AFFLICTION_FLAGS != 0)
    }) {
        return Err(EngineRefusal::MalformedArgs(
            "overlapping card-affliction powers",
        ));
    }
    super::turn::prepare_after_side_turn_end_singleton_write(
        state,
        crate::hot::AfterSideTurnEndPowerToken::Ringing,
        false,
        true,
    )?;
    for pile in CARD_AFFLICTION_PILES {
        for card in state.piles.get_mut(pile).make_mut() {
            card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING;
        }
    }
    state.set_ringing(true);
    state.exact_piles = true;
    Ok(())
}

/// RingingPower.AfterSideTurnEnd (`_remove_ringing_power`, frozen Python, deleted #2827).
pub(crate) fn remove_ringing_power(state: &mut HotState) {
    if !state.ringing() {
        return;
    }
    state.set_ringing(false);
    clear_ringing_from_current_cards(state);
}

/// RingingPower.AfterRemoved clears the affliction from the owner's current
/// cards even when a captured, already-detached old object is removed again.
pub(crate) fn clear_ringing_from_current_cards(state: &mut HotState) {
    for pile in CARD_AFFLICTION_PILES {
        for card in state.piles.get_mut(pile).make_mut() {
            card.flags &= !CARD_FLAG_RINGING;
        }
    }
}

pub(crate) fn apply_hex_power(state: &mut HotState, amount: i32) -> Result<(), EngineRefusal> {
    if amount != 2 || !hex_card_affliction_application_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("HexPower application"));
    }
    for pile in CARD_AFFLICTION_PILES {
        for card in state.piles.get_mut(pile).make_mut() {
            card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED;
        }
    }
    state
        .powers
        .set(PowerId::HexPower, crate::powers::SlotWire::Int, amount);
    state.exact_piles = true;
    Ok(())
}

pub(crate) fn remove_hex_power(state: &mut HotState) {
    if state.powers.value(PowerId::HexPower) <= 0 {
        return;
    }
    state
        .powers
        .set(PowerId::HexPower, crate::powers::SlotWire::Int, 0);
    for pile in CARD_AFFLICTION_PILES {
        for card in state.piles.get_mut(pile).make_mut() {
            card.flags &= !CARD_FLAG_HEXED;
        }
    }
}

pub(crate) fn hex_power_state_is_exact(state: &HotState) -> bool {
    let [_, spectral, _] = state.monsters.as_slice() else {
        return false;
    };
    if state.multiplayer_ally_key != 0
        || !knights_roster_is_exact(state, true)
        || (spectral.hp <= 0 && !state.fanouts.monster_death_is_pending(spectral.uid))
    {
        return false;
    }
    hex_card_state_is_exact(state)
}

/// Apply one exact Sword Sage amount delta to every owned physical Sovereign
/// Blade in Hand/Draw/Discard/Exhaust/Play.
///
/// Current v0.111.0 native authority: `SwordSagePower` at RVA `0xa902c`
/// updates every owner Blade by `AfterPowerAmountChanged`'s exact delta, while
/// `AfterRemoved` subtracts the full amount. Both paths validate the complete
/// batch before publishing any card-state write here. Remote Hammer Time cards
/// live in `MultiplayerAllyState` and are deliberately outside this walk.
pub(crate) fn apply_sword_sage_blade_delta(
    state: &mut HotState,
    catalog: &Catalog,
    delta: i32,
) -> Result<(), EngineRefusal> {
    if delta == 0 {
        return Ok(());
    }
    // Preflight every physical value before the first mutation. Positive
    // application initializes an absent native member from zero; removal
    // requires the member written by the live power and preserves Some(0).
    const SWORD_SAGE_PILE_ORDER: [PileId; 5] = [
        PileId::Hand,
        PileId::Draw,
        PileId::Discard,
        PileId::Exhaust,
        PileId::Play,
    ];
    for pile in SWORD_SAGE_PILE_ORDER {
        for card in state.piles.get(pile).as_slice() {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.identity.id != CardId::SovereignBlade {
                continue;
            }
            let current = match state.card_states.get(card.uid).base_replay_count() {
                Some(current) => current,
                None if delta > 0 => 0,
                None => {
                    return Err(EngineRefusal::MalformedArgs(
                        "Sword Sage Blade BaseReplayCount",
                    ));
                }
            };
            let updated = current
                .checked_add(delta)
                .filter(|updated| *updated >= 0)
                .ok_or(EngineRefusal::CounterOverflow(
                    "Sword Sage Blade BaseReplayCount",
                ))?;
            let _ = updated;
        }
    }
    for pile in SWORD_SAGE_PILE_ORDER {
        let len = state.piles.get(pile).len();
        for index in 0..len {
            let card = state.piles.get(pile).as_slice()[index];
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.identity.id != CardId::SovereignBlade {
                continue;
            }
            let mut instance = state.card_states.get(card.uid);
            let current = instance.base_replay_count().unwrap_or(0);
            let updated = current
                .checked_add(delta)
                .filter(|updated| *updated >= 0)
                .expect("the complete Sword Sage batch was preflighted");
            instance
                .set_base_replay_count(Some(updated))
                .expect("preflighted replay is nonnegative");
            state.card_states.set(card.uid, instance);
            state.piles.get_mut(pile).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            #[cfg(test)]
            SWORD_SAGE_DELTA_TRACE.with(|trace| trace.borrow_mut().push((pile, card.uid)));
        }
    }
    Ok(())
}

/// The modeled owner-matching PhysicalCard.AfterEnteredCombat suffix.
///
/// Ringing afflicts every later owner card first
/// (`physical_card_after_entered_combat`, frozen Python, deleted #2827). Phantom Blades then
/// grants combat-local Retain to each exact Shiv instance. Banshee's Cry and
/// Pinpoint then initialize fresh physical state from their matching finished-
/// play histories while clones preserve their copied payload. These are pure
/// per-card writes before the later generated-power suffix; observable ties
/// force exact projection.
fn intrinsic_default_physical_state(id: CardId) -> bool {
    matches!(
        id,
        CardId::BansheesCry
            | CardId::Flatten
            | CardId::KinglyKick
            | CardId::KinglyPunch
            | CardId::Melancholy
            | CardId::Midnight
            | CardId::Pinpoint
            | CardId::RocketPunch
            | CardId::UpMySleeve
            | CardId::Enlightenment
            | CardId::BulletTime
            | CardId::Modded
            | CardId::Stomp
            | CardId::Thrash
            | CardId::Claw
            | CardId::Maul
            | CardId::MomentumStrike
            | CardId::Rampage
            | CardId::TheBall
            | CardId::TheScythe
            | CardId::Wither
    )
}

/// Initialize the payload that exists as soon as a fresh physical card has
/// been inserted into combat, before `CardGenerated` is published.
///
/// The remaining owner/card-power listeners are `AfterCardEnteredCombat`
/// callbacks and deliberately live in the separate suffix below. Entropy's
/// singular Transform command has a native history seam between these two
/// phases; ordinary generated-card callers use the combined wrapper.
pub(crate) fn initialize_fresh_physical_card_state(spec: &CardSpec, card: &mut HotCard) {
    card.flags &= !crate::hot::CARD_FLAG_MELANCHOLY;
    if spec.identity.id == CardId::Melancholy {
        card.flags |= crate::hot::CARD_FLAG_MELANCHOLY;
    }

    // Python `physical_card_after_entered_combat` (frozen, deleted #2827) first calls
    // `_with_default_physical_card_state`. Fresh generated cards must
    // retain even its zero-valued slot: Rampage's first later play, for
    // example, grows that exact physical instance from zero.
    if intrinsic_default_physical_state(spec.identity.id) {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    }
}

/// The card half of `GhostSeed::CanAffect` (v0.111.0 RVA `0x945bc`): Basic
/// rarity (`get_Rarity == 1`, `IL_0002`-`IL_0008`) with `CardTag.Strike`
/// (`IL_000b`-`IL_0016`) or `CardTag.Defend` (`IL_0019`-`IL_0024`). At v0.111.0
/// that is exactly these ten identities, the oracle's `GHOST_SEED_CARD_IDS`
/// (frozen Python, deleted #2827), whose `_ghost_seed_native_shape` refuses on
/// registry drift. The third conjunct, "not already Ethereal"
/// (`GetKeywordsWithSources(2)`, `IL_0026`-`IL_0036`), is the caller's
/// [`effective_ethereal`] test.
fn ghost_seed_affects(id: CardId) -> bool {
    matches!(
        id,
        CardId::StrikeIronclad
            | CardId::StrikeSilent
            | CardId::StrikeDefect
            | CardId::StrikeNecrobinder
            | CardId::StrikeRegent
            | CardId::DefendIronclad
            | CardId::DefendSilent
            | CardId::DefendDefect
            | CardId::DefendNecrobinder
            | CardId::DefendRegent
    )
}

/// Ghost Seed's room-entry keyword pass (#2827).
///
/// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
///
/// `GhostSeed::AfterRoomEntered` (RVA `0x9453c`) is synchronous, not an async
/// state machine. It returns at `IL_0012`-`IL_0019` unless `room isinst
/// CombatRoom`, then enumerates `Owner.PlayerCombatState.AllCards`
/// (`IL_001a`-`IL_0025`) and, for every card [`ghost_seed_affects`] and that is
/// not already Ethereal (`GhostSeed::CanAffect`, `IL_003c`), calls
/// `CardCmd::ApplyKeyword(card, [Ethereal])` (`IL_0043`-`IL_004e`). There is no
/// owner test (`AllCards` is the owner's) and no RNG.
///
/// The oracle is `_ghost_seed_after_room_entered` (frozen Python, deleted #2827)
/// → `_ghost_seed_after_card_entered`, a local Ethereal keyword
/// on each affected card, run right after `_fire_relic_templates(s,
/// "AfterRoomEntered")` (`start_combat`). That is this function's position in
/// [`super::fire_after_room_entered`].
///
/// # Why it does not touch `exact_piles`
///
/// The oracle commits the rewrite through `_map_combat_cards` →
/// `_commit_live_card_piles` (frozen Python, deleted #2827), which promotes
/// exact piles only when the new piles hold a *distinguishable* `(id,
/// upgrade)` tie (`_has_distinguishable_sort_ties`). This pass
/// is a function of the card's payload alone — identity, and its current
/// keywords — so two equal payloads stay equal, and it cannot create a
/// distinguishable tie that was not there before. A deck that already had one
/// was already exact in the oracle's `start_combat`; that half is #2885, and
/// it is the opening's seed, not this body's. The generated-card half
/// ([`apply_physical_card_after_entered_suffix`]) does promote, because an
/// entering card lands beside unmarked copies; here every copy is marked.
///
/// # Ordering
///
/// The oracle's comment (frozen Python `start_combat`, deleted #2827) is the I5 argument: every
/// same-group `AfterRoomEntered` peer mutates creature or resource state
/// (the template relics Data Disk, Divine Right and Sword of Jade), and
/// Stone Cracker's upgrade commutes because an upgrade keeps local keywords
/// and `CompareTo` ignores them. Stone Cracker is refused by name before the
/// roster in the opening regardless.
pub(crate) fn ghost_seed_after_room_entered(state: &mut HotState, catalog: &Catalog) {
    if !catalog.hooks().owns(crate::ids::RelicId::RelicGhostSeed) {
        return;
    }
    for pile in ALL_CARD_PILES {
        let affected: Vec<u32> = state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .filter(|card| {
                catalog.spec(card.atom).is_some_and(|spec| {
                    ghost_seed_affects(spec.identity.id)
                        && !effective_ethereal(state, spec, card.uid)
                })
            })
            .map(|card| card.uid)
            .collect();
        for uid in affected {
            let mut instance = state.card_states.get(uid);
            instance.set_local_ethereal(true);
            state.card_states.set(uid, instance);
        }
    }
    crate::coverage::record_relic(crate::ids::RelicId::RelicGhostSeed);
}

fn apply_physical_card_after_entered_suffix(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    card: &mut HotCard,
    is_clone: bool,
) -> Result<(), EngineRefusal> {
    if catalog.hooks().owns(crate::ids::RelicId::RelicGhostSeed)
        && ghost_seed_affects(spec.identity.id)
        && !effective_ethereal(state, spec, card.uid)
    {
        let mut instance = state.card_states.get(card.uid);
        instance.set_local_ethereal(true);
        state.card_states.set(card.uid, instance);
        state.exact_piles = true;
        crate::coverage::record_relic(crate::ids::RelicId::RelicGhostSeed);
    }
    if state.ringing() {
        if card.flags & CARD_AFFLICTION_FLAGS != 0 && card.flags & CARD_FLAG_RINGING == 0 {
            return Err(EngineRefusal::MalformedArgs(
                "entering card-affliction overlap",
            ));
        }
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING;
    }
    if state.powers.value(PowerId::HexPower) > 0 {
        if card.flags & CARD_AFFLICTION_FLAGS != 0 && card.flags & CARD_FLAG_HEXED == 0 {
            return Err(EngineRefusal::MalformedArgs(
                "entering card-affliction overlap",
            ));
        }
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED;
    } else {
        card.flags &= !CARD_FLAG_HEXED;
    }
    if state.powers.value(PowerId::PhantomBlades) > 0 && spec.row.tags.contains(&"Shiv") {
        state.exact_piles = true;
        state.card_states.set_local_retain(card.uid);
    }
    // `SwordSagePower.AfterCardEnteredCombat` RVA `0xa90d0` applies the
    // current amount exactly once to a fresh owner Blade. MutableClone already
    // copied the physical scalar, so the native hook returns for clones.
    if spec.identity.id == CardId::SovereignBlade && !is_clone {
        let amount = state.powers.value(PowerId::SwordSage);
        if amount > 0 {
            let mut instance = state.card_states.get(card.uid);
            let updated = instance
                .base_replay_count()
                .unwrap_or(0)
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow(
                    "Sword Sage generated Blade BaseReplayCount",
                ))?;
            instance
                .set_base_replay_count(Some(updated))
                .expect("positive replay is representable");
            card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.card_states.set(card.uid, instance);
        }
    }
    // Current-v0.111 BansheesCry.AfterCardEnteredCombat RVA 0xd8564 first
    // requires this exact owner instance, then returns for clones. A fresh
    // card counts prior owner CardPlayFinished rows whose WasEthereal flag is
    // true and appends one combat-long Add row for their total. The compact
    // history counter is already the exact quotient of that predicate.
    if spec.identity.id == CardId::BansheesCry {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        if !is_clone {
            let completed = state.fanouts.ethereal_plays_finished_combat();
            if completed > 0 {
                state.card_states.append_local_cost_modifier(
                    card.uid,
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Add,
                        amount: -2 * i64::from(completed),
                        expiration: LocalCostExpiration::ThisCombat,
                        reduce_only: false,
                    },
                );
                // The entering card has not reached its destination yet. A
                // matching live CompareTo key is enough to prove that this
                // mutable Banshee pair needs exact pile order after insertion.
                if !state.exact_piles {
                    let mut matching_live = false;
                    for pile in PileId::ALL {
                        for live in state.piles.get(pile).as_slice() {
                            let live_spec = catalog
                                .spec(live.atom)
                                .ok_or(EngineRefusal::UnknownAtom(live.atom))?;
                            matching_live |= matches!(
                                live_spec.identity,
                                CardIdentity {
                                    id: CardId::BansheesCry,
                                    upgrade,
                                    ..
                                } if upgrade == spec.identity.upgrade
                            );
                        }
                    }
                    if matching_live {
                        state.exact_piles = true;
                    }
                }
            }
        }
    }
    // Current-v0.111 Midnight.AfterCardEnteredCombat owns the same fresh-vs-
    // clone split. A fresh instance backfills one combat-long Add row for the
    // complete prior owner Exhaust count; MutableClone has already copied its
    // ordered physical rows and must not append the history subtotal again.
    if spec.identity.id == CardId::Midnight {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        if !is_clone {
            let exhausted = state.history.owner_cards_exhausted_combat;
            if exhausted < 0 {
                return Err(EngineRefusal::CounterOverflow(
                    "Midnight prior exhausted cards",
                ));
            }
            if exhausted > 0 {
                state.card_states.append_local_cost_modifier(
                    card.uid,
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Add,
                        amount: -i64::from(exhausted),
                        expiration: LocalCostExpiration::ThisCombat,
                        reduce_only: false,
                    },
                );
                // The entering card has not reached its destination yet. A
                // matching mutable Midnight CompareTo sibling is enough to
                // require exact order after insertion.
                if !state.exact_piles {
                    let mut matching_live = false;
                    for pile in PileId::ALL {
                        for live in state.piles.get(pile).as_slice() {
                            let live_spec = catalog
                                .spec(live.atom)
                                .ok_or(EngineRefusal::UnknownAtom(live.atom))?;
                            matching_live |= matches!(
                                live_spec.identity,
                                CardIdentity {
                                    id: CardId::Midnight,
                                    upgrade,
                                    ..
                                } if upgrade == spec.identity.upgrade
                            );
                        }
                    }
                    if matching_live {
                        state.exact_piles = true;
                    }
                }
            }
        }
    }
    // Current-v0.111 Pinpoint.AfterCardEnteredCombat RVA `0xe7ba4` requires
    // this exact owner instance, returns for clones, counts owner Skill
    // CardPlayFinished rows that HappenedThisTurn, and appends their negated
    // total as one turn-long local-cost Add. Python
    // `physical_card_after_entered_combat` (frozen, deleted #2827) carries the same fresh-
    // entry/clone split and compact history quotient.
    if spec.identity.id == CardId::Pinpoint {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        if !is_clone {
            let completed = state.history.skill_plays_finished_this_turn;
            append_entered_this_turn_play_count_row(state, catalog, spec, card.uid, completed)?;
        }
    }
    // Stomp is Pinpoint's Attack twin (#3424); the IL is on
    // [`stomp_entered_combat_attack_count`].
    if spec.identity.id == CardId::Stomp {
        card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        if !is_clone {
            let completed = stomp_entered_combat_attack_count(state);
            append_entered_this_turn_play_count_row(state, catalog, spec, card.uid, completed)?;
        }
    }
    // Flatten has no clone gate (#3444); the IL is on
    // [`flatten_entered_after_osty_attack`].
    if spec.identity.id == CardId::Flatten && flatten_entered_after_osty_attack(state) {
        state.card_states.append_local_cost_modifier(
            card.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            },
        );
        require_exact_piles_for_live_sibling(state, catalog, spec)?;
    }
    Ok(())
}

/// The finished owner Attack plays a fresh Stomp backfills when it enters
/// combat (#3424).
///
/// Native authority: v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// - `Stomp::AfterCardEnteredCombat` RVA `0xeced8`: returns unless the entering
///   card is this instance (`IL_000c`-`IL_000e` `beq.s`) and returns for a
///   clone (`get_IsClone`, `IL_0017`-`IL_001c`). Otherwise it counts
///   `CombatManager.Instance.History.CardPlaysFinished` (`IL_0024`-`IL_002e`)
///   with `Enumerable.Count<CardPlayFinishedEntry>` (MethodSpec `0x2b000b7f`,
///   `IL_003f`) over `<AfterCardEnteredCombat>b__5_0`, and passes the count to
///   `ReduceCostBy` (`IL_0047`).
/// - `<AfterCardEnteredCombat>b__5_0` RVA `0xecf75`: `CardPlay.Card.Type == 1`
///   (Attack, `IL_000c`-`IL_0012`), `CardPlay.Player == Owner`
///   (`IL_001a`-`IL_0025`) and `HappenedThisTurn(CombatState)` (`IL_002e`).
///   It is Pinpoint's predicate (`0xe7c41`) with `ldc.i4.1` for `ldc.i4.2`.
/// - `Stomp::ReduceCostBy` RVA `0xecf65` is `EnergyCost.AddThisTurn(-n, 0)`
///   (`IL_0007`-`IL_000a`).
///
/// A Calamity-made Stomp counts the Attack that made it, because that play
/// is already finished: `CardModel/<OnPlayWrapper>d__339::MoveNext` (RVA
/// `0x31b8d0`) records `CombatHistory::CardPlayFinished` at `IL_084e`, before
/// `Hook.AfterCardPlayed` at `IL_0874`. `CalamityPower/<AfterCardPlayed>d__7`
/// (RVA `0x3368bc`) adds each card through
/// `CardPileCmd::AddGeneratedCardToCombat` (`IL_010b`), and
/// `CardPileCmd/<Add>d__10::MoveNext` (RVA `0x3e1ba4`) raises
/// `Hook.AfterCardEnteredCombat` at `IL_0659`. Rust keeps that order:
/// `record_card_play_finished` runs before the AfterCardPlayed power fan-out
/// in `engine::play`.
///
/// `Stomp::BeforeCardPlayed` (RVA `0xecf2a`) does not reach the new Stomp for
/// that play. `CombatState/<IterateHookListeners>d__69::MoveNext` (RVA
/// `0x3f9720`) builds the whole listener list on its first step (`List` at
/// `IL_0048`, pile cards added at `IL_019b`) and then yields from that list's
/// enumerator (`IL_0282`), rechecking only `CombatState::Contains`
/// (`IL_02a6`). A card created during an iteration is not visited by it, and
/// BeforeCardPlayed has finished by then in any case.
///
/// `history.attack_plays_finished_this_turn` is this predicate's quotient:
/// `record_card_play_finished` increments it for each owner Attack play and
/// every `SwitchSides` clears it (#3466, `turn::roll_happened_this_turn_counters`),
/// as `skill_plays_finished_this_turn` does for Pinpoint's Skills.
fn stomp_entered_combat_attack_count(state: &HotState) -> i16 {
    state.history.attack_plays_finished_this_turn
}

/// Append a fresh card's `ReduceCostBy(count)` row, one turn-long local
/// `Add(-count)`, for Pinpoint and Stomp. A zero count appends nothing:
/// `CardEnergyCost::AddThisTurn` (RVA `0x11e273`) returns on a zero amount
/// (`IL_0001`-`IL_0004`).
///
/// The entering card has not reached its destination yet, so a live copy
/// with the same `(id, upgrade)` is enough to require exact pile order.
fn append_entered_this_turn_play_count_row(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    uid: u32,
    completed: i16,
) -> Result<(), EngineRefusal> {
    if completed <= 0 {
        return Ok(());
    }
    state.card_states.append_local_cost_modifier(
        uid,
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: -i64::from(completed),
            expiration: LocalCostExpiration::ThisTurn,
            reduce_only: false,
        },
    );
    require_exact_piles_for_live_sibling(state, catalog, spec)
}

/// The entering card has not reached its destination yet, so a live copy with
/// the same `(id, upgrade)` is enough to require exact pile order once the
/// entering card carries a local-cost row its siblings may lack.
fn require_exact_piles_for_live_sibling(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if !state.exact_piles {
        let mut matching_live = false;
        for pile in PileId::ALL {
            for live in state.piles.get(pile).as_slice() {
                let live_spec = catalog
                    .spec(live.atom)
                    .ok_or(EngineRefusal::UnknownAtom(live.atom))?;
                matching_live |= live_spec.identity.id == spec.identity.id
                    && live_spec.identity.upgrade == spec.identity.upgrade;
            }
        }
        if matching_live {
            state.exact_piles = true;
        }
    }
    Ok(())
}

/// Whether an entering Flatten takes its `ReduceCost` backfill (#3444).
///
/// Native authority: v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// - `Flatten::AfterCardEnteredCombat` RVA `0xe030f`: returns unless the
///   entering card is this instance (`IL_0001`-`IL_0003` `beq.s`). There is no
///   `get_IsClone` test, so a clone takes the backfill as well as a fresh copy,
///   on top of the rows `MutableClone` already copied. It tests
///   `get_HasOstyAttackedThisTurn` (`IL_000c`-`IL_0011`) and calls
///   `ReduceCost` (`IL_001a`).
/// - `Flatten::ReduceCost` RVA `0xe0368` is `EnergyCost.SetThisTurn(0, false)`
///   (`IL_0002`-`IL_0009`). `CardEnergyCost::SetThisTurn` RVA `0x11e1f9`
///   appends one `LocalCostModifier(0, Set, ThisTurn, false)`
///   (`IL_000e`-`IL_001d`); its early return (`IL_0001`-`IL_000d`) needs a
///   negative canonical cost, which Flatten never has. It is the same row the
///   `Flatten::AfterAttack` listener (RVA `0xe0334`, `IL_0029`) appends in
///   `engine::damage`, so an Osty attack after the entry appends a second row,
///   as native calls `SetThisTurn` twice.
/// - `Flatten::get_HasOstyAttackedThisTurn` RVA `0xe0377` is
///   `Any(OfType<CreatureAttackedEntry>(History.Entries), b__15_0)`
///   (MethodSpecs `0x2b001161`/`0x2b001162`, `IL_0010`-`IL_0021`).
///   `<get_HasOstyAttackedThisTurn>b__15_0` RVA `0xe039e` is
///   `Actor == Owner.Osty` (`IL_0001`-`IL_0012`) and
///   `HappenedThisTurn(CombatState)` (`IL_001b`).
///
/// `SoloPetState::attacks_this_turn` is this predicate's quotient:
///
/// - **What writes the entries.** `CombatHistory::CreatureAttacked` RVA
///   `0x1387c5` has one caller, `AttackCommand/<Execute>d__90::MoveNext` RVA
///   `0x3f19c0`, which records `Actor = Attacker` once per command after the
///   hit loop (`IL_07e5`-`IL_082a`), before `Hook.AfterAttack` (`IL_0845`).
///   The engine counts one per `PlayerPet` command in the same position
///   (`SoloPetState::record_attack`), independent of hit count. The in-loop
///   exits for a dead attacker (`IL_0161`) and an empty target list
///   (`IL_01b3`) branch to that record site, and the engine records on
///   those paths too. The command-entry exits record nothing. For a dead
///   attacker (`IL_00a8`-`IL_00b1`) the engine agrees, because `osty_body`
///   issues no command without a live Osty. For a combat already over or
///   ending (`IL_0067`-`IL_008a`) the engine's Osty command returns at entry
///   too (#3466, `damage::player_attack_inner`).
/// - **Null actor.** No entry has a null actor: `Execute` throws
///   "No attacker set." for a null `Attacker` (`IL_0055`-`IL_0066`). An owner
///   with no Osty (`Player::get_Osty` RVA `0x116721` returns null at
///   `IL_000b`, else `GetPet<Osty>`) therefore matches nothing, and the counter
///   is zero because no pet command ran.
/// - **Osty identity.** Every summoned Osty gets `DieForYouPower`
///   (`OstyCmd/<Summon>d__0::MoveNext` RVA `0x3ee040` `IL_02f5`), whose
///   `ShouldCreatureBeRemovedFromCombatAfterDeath` (RVA `0xa1861`) vetoes
///   removal, so `PlayerCombatState::OnPetDied` (RVA `0x11844c`) returns at
///   `IL_003e` and keeps the corpse in `Pets`. A later summon revives that
///   same creature (`IL_00e8`-`IL_0103` finds it in `Allies`, `IL_0191`
///   `isReviving`). `Owner.Osty` is therefore the one creature every Osty
///   entry names, for the rest of combat.
/// - **This turn.** `CombatHistoryEntry::HappenedThisTurn` RVA `0x138a48`
///   compares the stamped RoundNumber (`IL_001d`), CurrentSide (`IL_002d`) and
///   player TurnNumbers with the live ones. The counter rolls at every
///   `SwitchSides`, the enemy-side switch included (#3466,
///   `turn::roll_happened_this_turn_counters`), so on either side it holds
///   exactly the Osty attacks stamped with the live side and turn. An entry on
///   the enemy side is therefore exact: it backfills only after an Osty
///   attack made on that enemy side.
fn flatten_entered_after_osty_attack(state: &HotState) -> bool {
    state.fanouts.pet().attacks_this_turn() != 0
}

fn apply_physical_card_after_entered_inner(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    card: &mut HotCard,
    is_clone: bool,
) -> Result<(), EngineRefusal> {
    initialize_fresh_physical_card_state(spec, card);
    apply_physical_card_after_entered_suffix(state, catalog, spec, card, is_clone)
}

/// Run only the native `AfterCardEnteredCombat` callback suffix for a fresh
/// card whose intrinsic physical payload was already initialized.
pub(crate) fn apply_physical_card_after_entered_callbacks(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    card: &mut HotCard,
) -> Result<(), EngineRefusal> {
    apply_physical_card_after_entered_suffix(state, catalog, spec, card, false)
}

pub(crate) fn apply_physical_card_after_entered(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    card: &mut HotCard,
) -> Result<(), EngineRefusal> {
    apply_physical_card_after_entered_inner(state, catalog, spec, card, false)
}

/// Allocate one fresh physical instance from `State.next_card_uid`.
///
/// Current v0.111.0 IL authority: `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// `CardPileCmd::AddGeneratedCardToCombat` at RVA `0x1305a0`. The command
/// accepts the freshly created combat card and routes one generated-card pile
/// transaction; callers below preserve that singular UID/history/listener
/// behavior rather than cloning an existing physical payload.
pub fn mint_card(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
) -> Result<HotCard, EngineRefusal> {
    let atom = catalog
        .atom(&identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    let uid = state.next_card_uid;
    state.next_card_uid = state
        .next_card_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    Ok(HotCard {
        uid,
        atom,
        flags: if identity.id == CardId::Melancholy {
            crate::hot::CARD_FLAG_MELANCHOLY
        } else {
            0
        },
    })
}

/// The native insertion index and order of a plural same-pile Transform.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`),
/// `CardCmd/<Transform>d__13::MoveNext` RVA `0x3e0ae0`: ONE loop over the
/// caller's transformation array reads each original's index with
/// `IndexOf` (`IL_0121-0132`) and then removes that original
/// (`RemoveFromCurrentPile`, `IL_0184-018c`) before the next iteration reads
/// its own index. So the recorded index is the original's position in the
/// pile with every EARLIER-listed original already gone, not its position in
/// the untouched pile (#3199: Compact over Hand `[Dazed, Defend, Dazed,
/// Dazed, Glasswork, Dazed]` records `0, 1, 1, 2`, and the Fuels land
/// `[Fuel, Fuel, Fuel, Fuel, Defend, Glasswork]`). The records are then
/// `List.Sort`ed by `CardCmd::PileIndexSort` RVA `0x12f92c` (same pile type
/// -> `Int32.CompareTo` of the index, `IL_004e-0060`), and each replacement
/// is inserted with `CardPile::AddInternal(card, index)` RVA `0x11eac4`,
/// which `List.Insert`s at any non-negative index (`IL_0057-0063`), in that
/// sorted order (`IL_03e2-03f1`).
///
/// `List.Sort` is the runtime's unstable introsort, so equal indices keep
/// their array order only where that sort provably does not move them: an
/// already non-decreasing sequence of at most 16 records (its
/// insertion-sort / compare-and-swap cut-off, which never moves an
/// in-order pair). A caller that lists its originals in pile order always
/// produces that shape (`index_k = original_k - k`); any other shape
/// refuses by name rather than guessing the introsort permutation.
pub(crate) fn native_transform_insertion_order(
    pile: &[HotCard],
    frozen: &[(usize, HotCard)],
) -> Result<Vec<(usize, HotCard)>, EngineRefusal> {
    let mut remaining: Vec<u32> = pile.iter().map(|card| card.uid).collect();
    let mut records = Vec::with_capacity(frozen.len());
    for (_, original) in frozen {
        let matches: Vec<usize> = remaining
            .iter()
            .enumerate()
            .filter_map(|(index, uid)| (*uid == original.uid).then_some(index))
            .collect();
        let [index] = matches.as_slice() else {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform duplicate pile index",
            ));
        };
        remaining.remove(*index);
        records.push((*index, *original));
    }
    let sorted = records.windows(2).all(|pair| pair[0].0 <= pair[1].0);
    let tied = records.windows(2).any(|pair| pair[0].0 == pair[1].0);
    if !sorted || (tied && records.len() > 16) {
        return Err(EngineRefusal::MalformedArgs(
            "bulk transform introsort order",
        ));
    }
    Ok(records)
}

/// Replace one frozen same-pile physical-card batch with fresh fixed cards.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// plural `CardCmd::Transform` RVA `0x12f990` and
/// `CardCmd/<Transform>d__13::MoveNext` RVA `0x3e0ae0` first validate and
/// remove every original (`IL 0x009e-0x01b1`), sort the retained pile/index
/// records with `PileIndexSort` RVA `0x12f92c`, insert every replacement at
/// its recorded index — each original's position after the EARLIER-listed
/// originals were removed (`native_transform_insertion_order`) — while
/// recording `CardGenerated` and physical-entry /
/// pile-change callbacks (`IL 0x020b-0x0581`), and only then dispatch
/// `AfterCardGeneratedForCombat` serially in result order
/// (`IL 0x0944-0x0a4a`).
///
/// The native pile-change callback is deliberately an inert boundary here,
/// not a silently omitted modeled effect. The current-build listener census
/// contains only Bing Bong, Darkstone Periapt, and Lucky Fysh; all three read
/// Deck entry, while this helper accepts only the five live combat piles.
///
/// The caller supplies the native command's immutable physical snapshot.
/// Every uid, payload, side-table row, replacement atom, counter, and future
/// generated callback is rehearsed on a clone. Therefore no removal,
/// allocation, history prefix, or event escapes a later refusal. The
/// returned cards are in the native sorted record order (the caller's order) and carry fresh
/// monotonic uids; source cards are never inferred from payload equality.
pub(crate) fn bulk_transform_fixed_same_pile(
    state: &mut HotState,
    catalog: &Catalog,
    pile: PileId,
    frozen: &[(HotCard, CardInstanceState)],
    replacement_identity: CardIdentity,
    events: &mut Vec<Event>,
) -> Result<Vec<HotCard>, EngineRefusal> {
    if state.history.over || frozen.is_empty() {
        return Ok(Vec::new());
    }
    let replacement_atom = catalog
        .atom(&replacement_identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(replacement_identity))?;
    let replacement_spec = catalog
        .spec(replacement_atom)
        .ok_or(EngineRefusal::UnknownAtom(replacement_atom))?;
    if replacement_spec.identity != replacement_identity {
        return Err(EngineRefusal::MalformedArgs(
            "bulk transform replacement identity",
        ));
    }
    if frozen.iter().any(|(original, _)| {
        super::monsters::deck_version_link_owns_uid(state, original.uid)
            || dampen_tracks_uid(state, original.uid)
    }) {
        return Err(EngineRefusal::MalformedArgs(
            "persistent encounter-card transform",
        ));
    }

    let mut records = Vec::with_capacity(frozen.len());
    for (position, (original, instance)) in frozen.iter().enumerate() {
        if frozen[..position]
            .iter()
            .any(|(earlier, _)| earlier.uid == original.uid)
        {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform duplicate frozen identity",
            ));
        }
        let Some((live_pile, index)) = unique_live_card_location(state, original.uid)? else {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: original.uid,
                pile,
            });
        };
        if live_pile != pile
            || state.piles.get(pile).as_slice()[index] != *original
            || state.card_states.get(original.uid) != *instance
            || original.flags & CARD_FLAG_LEGACY != 0
        {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: original.uid,
                pile,
            });
        }
        catalog
            .spec(original.atom)
            .ok_or(EngineRefusal::UnknownAtom(original.atom))?;
        records.push((index, *original));
    }
    let records = native_transform_insertion_order(state.piles.get(pile).as_slice(), &records)?;

    let count: u32 = frozen
        .len()
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("bulk transform count"))?;
    let next_after = state
        .next_card_uid
        .checked_add(count)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    for uid in state.next_card_uid..next_after {
        if unique_live_card_location(state, uid)?.is_some() {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform next uid collision",
            ));
        }
    }
    preflight_generated_power_batch(state, frozen.len(), true)?;

    let mut next = state.clone();
    let mut replacements = Vec::with_capacity(records.len());
    for _ in &records {
        replacements.push(mint_card(&mut next, catalog, replacement_identity)?);
    }

    let original_uids = records
        .iter()
        .map(|(_, original)| original.uid)
        .collect::<Vec<_>>();
    let retained = next
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| !original_uids.contains(&card.uid))
        .collect::<Vec<_>>();
    if retained.len() + records.len() != next.piles.get(pile).len() {
        return Err(EngineRefusal::MalformedArgs(
            "bulk transform removal cardinality",
        ));
    }
    *next.piles.get_mut(pile).make_mut() = retained;
    for (_, original) in &records {
        next.card_states
            .set(original.uid, CardInstanceState::default());
        let original_spec = catalog
            .spec(original.atom)
            .ok_or(EngineRefusal::UnknownAtom(original.atom))?;
        if original_spec.strike_tag {
            next.ps_strikes = next
                .ps_strikes
                .checked_sub(1)
                .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
        }
    }

    for ((index, _), replacement) in records.iter().zip(replacements.iter_mut()) {
        if *index > next.piles.get(pile).len() {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform replacement index",
            ));
        }
        apply_physical_card_after_entered(&mut next, catalog, replacement_spec, replacement)?;
        next.piles
            .get_mut(pile)
            .make_mut()
            .insert(*index, *replacement);
        if replacement_spec.strike_tag {
            next.ps_strikes = next
                .ps_strikes
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
        }
        next.history.owner_generated_cards_combat = next
            .history
            .owner_generated_cards_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat",
            ))?;
    }
    next.exact_piles = true;

    let mut produced = Vec::new();
    for replacement in &replacements {
        // A later insertion at a lower-or-equal index shifts an earlier
        // replacement, so only its unique pile membership is invariant here.
        let Some((live_pile, live_index)) = unique_live_card_location(&next, replacement.uid)?
        else {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform generated result drift",
            ));
        };
        if live_pile != pile || next.piles.get(pile).as_slice()[live_index] != *replacement {
            return Err(EngineRefusal::MalformedArgs(
                "bulk transform generated result drift",
            ));
        }
        next.next_generated_hook_uid = next
            .next_generated_hook_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        after_owner_card_generated(&mut next, catalog, replacement_spec, &mut produced)?;
    }

    *state = next;
    events.extend(produced);
    Ok(replacements)
}

/// Mint `count` copies and insert them at a pile's bottom (list end).
///
/// Hand insertion applies the native cap per card: once Hand has ten cards,
/// each later instance overflows to Discard/Bottom.
pub fn inject_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    pile: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    for _ in 0..count {
        if state.history.over {
            break;
        }
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        let mut card = mint_card(state, catalog, identity)?;
        apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
        let destination =
            if pile == PileId::Hand && state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
                PileId::Discard
            } else {
                pile
            };
        state.piles.get_mut(destination).make_mut().push(card);
        events.push(Event::CardResolved {
            uid: card.uid,
            pile: destination,
        });
    }
    Ok(())
}

/// Record and insert owner-generated physical cards.
///
/// `CardGenerated` history precedes the nested pile add. Hand adds route each
/// overflow independently to Discard/Bottom, and Strike-tagged clones join
/// Perfected Strike's combat count after they enter combat. The shared payload
/// helper then dispatches the admitted generated-card power and physical-card
/// callbacks in native order, rehearsing the fallible suffix before publishing
/// any part of the synchronous transaction.
pub fn inject_generated_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        None,
        count,
        GeneratedTransaction::gate_before_record(destination, false),
        events,
    )
}

/// Hidden Daggers' exact L0 Shiv generation followed by returned-instance upgrade.
///
/// Python `add_generated_shivs_then_upgrade` (frozen, deleted #2827) first completes every
/// serial `Shiv.CreateInHand` transaction, retaining each post-listener pile
/// location. Only when combat is still live does upgraded Hidden Daggers walk
/// that returned prefix and replace each exactly-once physical Shiv with L1.
/// Same- and cross-pile duplicate uids refuse through the shared global
/// locator. The complete two-phase body is rehearsed so a late generated
/// listener or identity drift cannot expose a partial pile/history mutation.
pub(crate) fn inject_generated_shivs_then_upgrade(
    state: &mut HotState,
    catalog: &Catalog,
    count: usize,
    upgrade: u8,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        count: usize,
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let base = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let upgraded_atom = if upgrade == 1 {
            Some(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: None,
                    })
                    .ok_or(EngineRefusal::UnknownMintIdentity(CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: None,
                    }))?,
            )
        } else if upgrade == 0 {
            None
        } else {
            return Err(EngineRefusal::MalformedArgs("generated Shiv upgrade level"));
        };

        let mut returned = Vec::with_capacity(count);
        for _ in 0..count {
            if state.history.over {
                break;
            }
            let uid = state.next_card_uid;
            inject_generated_bottom(state, catalog, base, 1, PileId::Hand, events)?;
            returned.push(uid);
        }
        if state.history.over || upgraded_atom.is_none() {
            return Ok(());
        }
        let upgraded_atom = upgraded_atom.expect("upgrade=1 resolved an atom");
        for uid in returned {
            let (pile, index) = unique_live_card_location(state, uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let live = state.piles.get(pile).as_slice()[index];
            let identity = catalog
                .spec(live.atom)
                .ok_or(EngineRefusal::UnknownAtom(live.atom))?
                .identity;
            if identity != base {
                return Err(EngineRefusal::MalformedArgs(
                    "generated Shiv identity drift",
                ));
            }
            state.piles.get_mut(pile).make_mut()[index].atom = upgraded_atom;
            if !state.exact_piles && live_cards_need_exact_piles(state, catalog)? {
                state.exact_piles = true;
            }
        }
        Ok(())
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, count, upgrade, &mut Vec::new())?;
    apply(state, catalog, count, upgrade, events)
}

/// Storm of Steel's bulk L0-Shiv creation and live-only upgrade suffix.
///
/// This is intentionally distinct from Hidden Daggers' repeated singular
/// `Shiv::CreateInHand` helper. Current v0.111.0 authority (`sts2.dll`
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `StormOfSteel/<OnPlay>d__3::MoveNext` RVA `0x3bf6b8` passes the frozen Hand
/// count to plural `Shiv::CreateInHand` at IL `0x0125-0x0138`, awaits the
/// returned exact objects, then calls `CardCmd::Upgrade` only for upgraded
/// Storm at IL `0x0197-0x01b8`. `Shiv/<CreateInHand>d__12::MoveNext` RVA
/// `0x3bb20c` gates once at IL `0x002c-0x003d`, allocates the complete fresh
/// list at `0x0043-0x0079`, and passes that list to one plural generated Add
/// at `0x007b-0x0098`. The plural Add body RVA `0x3e2f0c` has no loop ending
/// break: every allocated object records history and reaches its generated
/// hook, while its nested pile Add alone may be suppressed. `CardCmd::Upgrade`
/// RVA `0x12f660` checks `IsEnding` once at IL `0x000c-0x0018`, so a terminal
/// generated listener suppresses the entire upgrade pass.
///
/// Every one of those gates is the shared IsEnding projection
/// [`super::damage::damage_combat_is_ending`], not `history.over`: the
/// `CreateInHand` entry gate is `CombatManager::get_IsOverOrEnding` (IL
/// `0x002c-0x0036`), the nested `CardPileCmd.Add` (`<Add>d__10` RVA
/// `0x3e1ba4`) reads `get_IsEnding` at IL `0x0053`, and `CardCmd::Upgrade`
/// reads `get_IsEnding` at IL `0x0011`. The plural generated Add itself
/// (`<AddGeneratedCardsToCombat>d__6` RVA `0x3e2f0c`) gates only on
/// `get_IsInProgress` (IL `0x0039-0x0043`), which stays true while combat is
/// ending, so it records every allocated object. So while every primary is
/// dead and no veto holds, before the over latch, Storm of Steel, Cunning
/// Potion and Blade of Ink create nothing.
#[derive(Copy, Clone)]
enum PluralShivSuffix {
    Upgrade(u8),
    Inky,
}

fn rewrite_live_plural_shiv_atoms(
    state: &mut HotState,
    catalog: &Catalog,
    returned: &[HotCard],
    base: CardIdentity,
    transformed_atom: CardAtom,
    allow_terminal_detached: bool,
) -> Result<(), EngineRefusal> {
    for returned_card in returned {
        let Some((pile, index)) = unique_live_card_location(state, returned_card.uid)? else {
            // CardCmd.Enchant iterates the complete returned object list even
            // after combat starts ending. Objects whose nested pile Add was
            // ending-gated are detached and have no canonical projection to
            // rewrite. CardCmd.Upgrade has its own earlier ending gate.
            if super::damage::damage_combat_is_ending(state) && allow_terminal_detached {
                continue;
            }
            return Err(EngineRefusal::ContinuationNotModeled);
        };
        let live = state.piles.get(pile).as_slice()[index];
        if catalog
            .spec(live.atom)
            .ok_or(EngineRefusal::UnknownAtom(live.atom))?
            .identity
            != base
        {
            return Err(EngineRefusal::MalformedArgs(
                "plural generated Shiv identity drift",
            ));
        }
        // Atom-only replacement preserves the exact returned object's uid,
        // pile/index, flags, and complete sparse CardInstanceState.
        state.piles.get_mut(pile).make_mut()[index].atom = transformed_atom;
        if !state.exact_piles && live_cards_need_exact_piles(state, catalog)? {
            state.exact_piles = true;
        }
    }
    Ok(())
}

fn inject_generated_plural_shivs(
    state: &mut HotState,
    catalog: &Catalog,
    count: usize,
    suffix: PluralShivSuffix,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        count: usize,
        suffix: PluralShivSuffix,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if count > MAX_CARDS_IN_HAND {
            return Err(EngineRefusal::MalformedArgs("plural Shiv count"));
        }
        let base = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let spec = catalog
            .atom(&base)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(base))?;
        let transformed_atom = match suffix {
            PluralShivSuffix::Upgrade(0) => None,
            PluralShivSuffix::Upgrade(1) => {
                let transformed = CardIdentity {
                    id: CardId::Shiv,
                    upgrade: 1,
                    enchantment: None,
                };
                Some(
                    catalog
                        .atom(&transformed)
                        .ok_or(EngineRefusal::UnknownMintIdentity(transformed))?,
                )
            }
            PluralShivSuffix::Upgrade(_) => {
                return Err(EngineRefusal::MalformedArgs("Storm Shiv upgrade level"));
            }
            PluralShivSuffix::Inky => {
                let transformed = CardIdentity {
                    id: CardId::Shiv,
                    upgrade: 0,
                    enchantment: Some(crate::catalog::CardEnchantment {
                        id: EnchantmentId::Inky,
                        amount: 1,
                    }),
                };
                Some(
                    catalog
                        .atom(&transformed)
                        .ok_or(EngineRefusal::UnknownMintIdentity(transformed))?,
                )
            }
        };
        if super::damage::damage_combat_is_ending(state) || count == 0 {
            return Ok(());
        }

        // Shiv.CreateInHand constructs the complete list before the plural
        // generated command begins. A hook ending combat after its first
        // entry therefore cannot prevent allocation of later UIDs.
        let mut returned = Vec::with_capacity(count);
        for _ in 0..count {
            returned.push(mint_card(state, catalog, base)?);
        }

        for card in &returned {
            state.history.owner_generated_cards_combat = state
                .history
                .owner_generated_cards_combat
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow(
                    "owner_generated_cards_combat",
                ))?;
            state.next_generated_hook_uid = state
                .next_generated_hook_uid
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
            if super::damage::damage_combat_is_ending(state) {
                after_owner_card_generated(state, catalog, spec, events)?;
                continue;
            }
            let mut entered = *card;
            apply_physical_card_after_entered(state, catalog, spec, &mut entered)?;
            let pile = if state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
                PileId::Discard
            } else {
                PileId::Hand
            };
            state.piles.get_mut(pile).make_mut().push(entered);
            events.push(Event::CardResolved {
                uid: entered.uid,
                pile,
            });
            after_owner_card_generated(state, catalog, spec, events)?;
        }

        let Some(transformed_atom) = transformed_atom else {
            return Ok(());
        };
        if super::damage::damage_combat_is_ending(state)
            && matches!(suffix, PluralShivSuffix::Upgrade(_))
        {
            return Ok(());
        }
        rewrite_live_plural_shiv_atoms(
            state,
            catalog,
            &returned,
            base,
            transformed_atom,
            matches!(suffix, PluralShivSuffix::Inky),
        )
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, count, suffix, &mut Vec::new())?;
    apply(state, catalog, count, suffix, events)
}

pub(crate) fn inject_generated_storm_shivs(
    state: &mut HotState,
    catalog: &Catalog,
    count: usize,
    upgrade: u8,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_plural_shivs(
        state,
        catalog,
        count,
        PluralShivSuffix::Upgrade(upgrade),
        events,
    )
}

/// Blade of Ink's one plural L0-Shiv creation followed by its synchronous
/// Inky(1) transform over every returned object. Unlike CardCmd::Upgrade, the
/// transform has no combat-ending gate; detached suffix objects remain
/// unobservable while every live inserted UID is rewritten exactly once.
pub(crate) fn inject_generated_blade_inky_shivs(
    state: &mut HotState,
    catalog: &Catalog,
    count: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_plural_shivs(state, catalog, count, PluralShivSuffix::Inky, events)
}

/// Record and insert one owner-generated card at a random Draw position.
///
/// Python `_add_beckons_exact` (frozen, deleted #2827) authenticates one complete
/// `CardGenerated` transaction. `_insert_draw_random` pins the
/// one Shuffle-stream draw against the pre-insertion length; the exact helper
/// then publishes the fresh physical card at that index. Every fallible
/// coordinate is preflighted before history, RNG, uid, pile, or power state
/// can move.
pub fn inject_generated_draw_random(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    if state.history.over {
        return Ok(());
    }
    preflight_generated_power_batch(state, 1, false)?;
    state
        .history
        .owner_generated_cards_combat
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_generated_cards_combat",
        ))?;
    state
        .next_generated_hook_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    state
        .next_card_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    if spec.strike_tag {
        state
            .ps_strikes
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
    }
    let bound: i32 = state
        .piles
        .get(PileId::Draw)
        .len()
        .checked_add(1)
        .and_then(|value| value.try_into().ok())
        .ok_or(EngineRefusal::CounterOverflow(
            "random draw insertion bound",
        ))?;
    let live = state.rng.get(RngStream::Rng);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let index: usize = rng
        .next_bounded(bound)
        .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion bound"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion index"))?;

    state.history.owner_generated_cards_combat += 1;
    state.next_generated_hook_uid += 1;
    state.rng.set(
        RngStream::Rng,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let mut card = mint_card(state, catalog, identity)?;
    apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
    state
        .piles
        .get_mut(PileId::Draw)
        .make_mut()
        .insert(index, card);
    if spec.strike_tag {
        state.ps_strikes += 1;
    }
    events.push(Event::CardResolved {
        uid: card.uid,
        pile: PileId::Draw,
    });
    after_owner_card_generated(state, catalog, spec, events)
}

/// Record one owner-generated command before a random-Draw Add ending gate.
///
/// SoulBody begins each serial `AddGeneratedCardsToCombat` command even when
/// its preceding damage ended combat. History and the generated-hook epoch
/// still advance; only the nested random pile Add (including RNG and UID
/// allocation) is suppressed. This is the random-position twin of
/// [`inject_generated_record_before_ending_bottom`].
pub(crate) fn inject_generated_record_before_ending_draw_random(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    preflight_generated_power_batch(state, 1, true)?;
    state.history.owner_generated_cards_combat = state
        .history
        .owner_generated_cards_combat
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_generated_cards_combat",
        ))?;
    state.next_generated_hook_uid = state
        .next_generated_hook_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    if state.history.over {
        return after_owner_card_generated(state, catalog, spec, events);
    }
    state
        .next_card_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    let bound: i32 = state
        .piles
        .get(PileId::Draw)
        .len()
        .checked_add(1)
        .and_then(|value| value.try_into().ok())
        .ok_or(EngineRefusal::CounterOverflow(
            "random draw insertion bound",
        ))?;
    let live = state.rng.get(RngStream::Rng);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let index: usize = rng
        .next_bounded(bound)
        .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion bound"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion index"))?;
    state.rng.set(
        RngStream::Rng,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let mut card = mint_card(state, catalog, identity)?;
    apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
    state
        .piles
        .get_mut(PileId::Draw)
        .make_mut()
        .insert(index, card);
    if spec.strike_tag {
        state.ps_strikes = state
            .ps_strikes
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
    }
    events.push(Event::CardResolved {
        uid: card.uid,
        pile: PileId::Draw,
    });
    after_owner_card_generated(state, catalog, spec, events)
}

/// Insert one null-creator generated card at a random Draw or Discard position.
///
/// The Insatiable's `_insatiable_add_frantic_escape` (frozen Python, deleted #2827) is not an
/// owner-generated-card history entry, but it is otherwise a complete physical
/// `CardGenerated` transaction: allocate a uid, consume one Shuffle draw,
/// publish the pile entry, advance the generated-listener token, and run the
/// local generated-power suffix with a null creator. Every admitted local
/// generated-card listener (Arsenal, Pillar of Creation, Smokestack, Trash to
/// Treasure, Regalite, Rocket Punch) gates on an owner creator and so is a
/// no-op here (#3256, cited at `after_local_card_generated_inner`); only the
/// creator-blind Aeonglass listener can answer.
pub fn inject_generated_null_random(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !matches!(destination, PileId::Draw | PileId::Discard) {
        return Err(EngineRefusal::MalformedArgs(
            "null-creator random destination",
        ));
    }
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    if state.history.over {
        return Ok(());
    }
    // No `preflight_generated_power_batch`: the Arsenal and Pillar of
    // Creation arithmetic it guards never answers a null creator (#3256).
    state
        .next_generated_hook_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    state
        .next_card_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    if spec.strike_tag {
        state
            .ps_strikes
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
    }
    let bound: i32 = state
        .piles
        .get(destination)
        .len()
        .checked_add(1)
        .and_then(|value| value.try_into().ok())
        .ok_or(EngineRefusal::CounterOverflow(
            "null-creator random insertion bound",
        ))?;
    let live = state.rng.get(RngStream::Rng);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let index: usize = rng
        .next_bounded(bound)
        .map_err(|_| EngineRefusal::CounterOverflow("null-creator random insertion bound"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("null-creator random insertion index"))?;

    state.next_generated_hook_uid += 1;
    state.rng.set(
        RngStream::Rng,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let mut card = mint_card(state, catalog, identity)?;
    apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
    state
        .piles
        .get_mut(destination)
        .make_mut()
        .insert(index, card);
    if spec.strike_tag {
        state.ps_strikes += 1;
    }
    events.push(Event::CardResolved {
        uid: card.uid,
        pile: destination,
    });
    after_null_card_generated(state, catalog, spec, events)
}

/// Insert a serial batch of null-creator generated cards at Discard bottom.
///
/// Test Subject's Burning Growl and Painful Stabs use the same physical-card
/// and generated-power suffixes as the Insatiable's random insertion, but do
/// not consume RNG and never increment owner-generated-card history.  Native
/// awaits each complete transaction before beginning the next one.  Rehearse
/// that exact loop so a failure in a later local generated listener cannot
/// expose an earlier uid, pile, hook-token, or event mutation to direct callers.
pub(crate) fn inject_generated_null_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        count: usize,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        for _ in 0..count {
            if state.history.over {
                break;
            }
            state.next_generated_hook_uid = state
                .next_generated_hook_uid
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
            let mut card = mint_card(state, catalog, identity)?;
            apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
            state.piles.get_mut(PileId::Discard).make_mut().push(card);
            if spec.strike_tag {
                state.ps_strikes = state
                    .ps_strikes
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
            }
            events.push(Event::CardResolved {
                uid: card.uid,
                pile: PileId::Discard,
            });
            after_null_card_generated(state, catalog, spec, events)?;
        }
        Ok(())
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, identity, count, &mut Vec::new())?;
    apply(state, catalog, identity, count, events)
}

/// Aeonglass's singular null-creator Wither command.
///
/// Unlike owner-created generation this advances only the combat-wide hook
/// epoch. Native awaits the complete Add/entered/generated-listener suffix
/// before the caller resets Withering or starts the next Intensity child.
pub(crate) fn inject_aeonglass_wither(
    state: &mut HotState,
    catalog: &Catalog,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        destination: PileId,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if !matches!(destination, PileId::Hand | PileId::Discard)
            || !aeonglass_generated_transient_is_exact(state, catalog)
        {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass generated Wither entry",
            ));
        }
        if state.history.over || state.monsters[0].hp <= 0 {
            return Ok(());
        }
        let identity = CardIdentity {
            id: CardId::Wither,
            upgrade: 0,
            enchantment: None,
        };
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        // A null creator: no owner-gated power arithmetic to preflight (#3256).
        state
            .next_generated_hook_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        state
            .next_card_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;

        state.next_generated_hook_uid += 1;
        let mut card = mint_card(state, catalog, identity)?;
        initialize_fresh_physical_card_state(spec, &mut card);
        let pile = if destination == PileId::Hand
            && state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND
        {
            PileId::Discard
        } else {
            destination
        };
        // The oracle's force-exact commit publishes exact mode before the
        // Wither enters its pile (`_add_aeonglass_wither`, frozen Python, deleted #2827; `_commit_live_card_piles`). Since #2957 the flag can still be off here, on the
        // first Withering Presence Wither of a fight that opened without it.
        state.exact_piles = true;
        state.piles.get_mut(pile).make_mut().push(card);
        apply_physical_card_after_entered_callbacks(state, catalog, spec, &mut card)?;
        let inserted = state
            .piles
            .get(pile)
            .as_slice()
            .len()
            .checked_sub(1)
            .ok_or(EngineRefusal::MalformedArgs(
                "Aeonglass generated Wither insertion",
            ))?;
        state.piles.get_mut(pile).make_mut()[inserted] = card;
        events.push(Event::CardResolved {
            uid: card.uid,
            pile,
        });
        after_null_card_generated(state, catalog, spec, events)
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, destination, &mut Vec::new())?;
    apply(state, catalog, destination, events)
}

/// Front player-power AfterCardPlayed callback for Withering Presence.
pub(crate) fn withering_after_card_played(
    state: &mut HotState,
    catalog: &Catalog,
    hook_started: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        hook_started: bool,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let current = state.fanouts.withering_cards_left();
        if !hook_started || current == 0 {
            return Ok(());
        }
        if !aeonglass_state_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs(
                "Withering AfterCardPlayed entry",
            ));
        }
        let next = current - 1;
        let stored = state.fanouts.set_withering_cards_left(next);
        debug_assert!(stored);
        if next == 0 {
            inject_aeonglass_wither(state, catalog, PileId::Hand, events)?;
            let stored = state.fanouts.set_withering_cards_left(6);
            debug_assert!(stored);
        }
        Ok(())
    }

    if !hook_started || state.fanouts.withering_cards_left() == 0 {
        return Ok(());
    }
    let mut probe = state.clone();
    apply(&mut probe, catalog, true, &mut Vec::new())?;
    apply(state, catalog, true, events)
}

/// Validate a fixed generated-card batch without publishing any transaction.
/// Multi-command monster moves use this wall so the second command cannot
/// discover a deterministic capacity failure after the first card entered.
pub(crate) fn preflight_generated_card_batch(
    state: &HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    if state.history.over {
        return Ok(());
    }
    preflight_generated_power_batch(state, count, false)?;
    let count_u32 =
        u32::try_from(count).map_err(|_| EngineRefusal::CounterOverflow("generated card batch"))?;
    let count_i32 =
        i32::try_from(count).map_err(|_| EngineRefusal::CounterOverflow("generated card batch"))?;
    state
        .history
        .owner_generated_cards_combat
        .checked_add(count_i32)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_generated_cards_combat",
        ))?;
    state
        .next_generated_hook_uid
        .checked_add(count_i32)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    state
        .next_card_uid
        .checked_add(count_u32)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    if spec.strike_tag {
        state
            .ps_strikes
            .checked_add(count_i32)
            .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
    }
    Ok(())
}

/// Record every fixed generated-Status command before its ending gate.
///
/// Crash Landing's native command begins each `CardGenerated` transaction,
/// including its generated-listener epoch, before the nested pile Add checks
/// whether combat is ending. Consequently a lethal attack records every
/// remaining Debris command without allocating a uid or inserting a card.
/// The shared generated-card transaction's record-before-Add policy has the
/// same order. This wrapper remains separate because its direct card-body
/// callers require an exact clone rehearsal of the whole serial batch before
/// the first observable mutation.
pub(crate) fn inject_generated_record_before_ending_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_record_before_ending_bottom_impl(
        state,
        catalog,
        identity,
        count,
        destination,
        false,
        events,
    )
}

/// Creative AI's awaited generated-card Add uses the same record-before-ending
/// transaction, but its physical entry commits the canonical exact pile
/// projection after `PhysicalCard.AfterEnteredCombat` and before callbacks.
pub(crate) fn inject_before_hand_draw_generated_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_record_before_ending_bottom_impl(
        state,
        catalog,
        identity,
        1,
        PileId::Hand,
        true,
        events,
    )
}

/// Insert one plural BeforeHandDraw generated-card batch in frozen identity
/// order. The plural command has one leading ending gate; once begun, every
/// later member still records generated history and its callback epoch even
/// if an earlier callback ended combat, while that member's nested pile Add
/// independently no-ops.
pub(crate) fn inject_before_hand_draw_generated_batch_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identities: &[CardIdentity],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || identities.is_empty() {
        return Ok(());
    }
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identities: &[CardIdentity],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        for identity in identities.iter().copied() {
            inject_generated_record_before_ending_bottom_impl(
                state,
                catalog,
                identity,
                1,
                PileId::Hand,
                true,
                events,
            )?;
        }
        Ok(())
    }
    let mut probe = state.clone();
    apply(&mut probe, catalog, identities, &mut Vec::new())?;
    apply(state, catalog, identities, events)
}

fn inject_generated_record_before_ending_bottom_impl(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    destination: PileId,
    force_exact_on_insert: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        count: usize,
        destination: PileId,
        force_exact_on_insert: bool,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        for _ in 0..count {
            state.history.owner_generated_cards_combat = state
                .history
                .owner_generated_cards_combat
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow(
                    "owner_generated_cards_combat",
                ))?;
            state.next_generated_hook_uid = state
                .next_generated_hook_uid
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
            if state.history.over {
                after_owner_card_generated(state, catalog, spec, events)?;
                continue;
            }
            let mut card = mint_card(state, catalog, identity)?;
            apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
            let pile = if destination == PileId::Hand
                && state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND
            {
                PileId::Discard
            } else {
                destination
            };
            if force_exact_on_insert {
                state.exact_piles = true;
            }
            state.piles.get_mut(pile).make_mut().push(card);
            if spec.strike_tag {
                state.ps_strikes = state
                    .ps_strikes
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
            }
            events.push(Event::CardResolved {
                uid: card.uid,
                pile,
            });
            after_owner_card_generated(state, catalog, spec, events)?;
        }
        Ok(())
    }

    // A later iteration can overflow after earlier history, insertion, or
    // listener mutations. Rehearse the exact loop so a direct caller observes
    // the same all-or-nothing refusal boundary as `apply_action`.
    let mut probe = state.clone();
    apply(
        &mut probe,
        catalog,
        identity,
        count,
        destination,
        force_exact_on_insert,
        &mut Vec::new(),
    )?;
    apply(
        state,
        catalog,
        identity,
        count,
        destination,
        force_exact_on_insert,
        events,
    )
}

/// Shuffle one frozen generation pool on the dedicated stream.
///
/// The returned array is the complete shuffled pool because native callers
/// consume a full shuffle even when they keep only a short prefix.
///
/// Current v0.111.0 IL authority: the same `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`
/// places `CardFactory::GetDistinctForCombat` at RVA `0x112878` and its
/// `ListExtensions::UnstableShuffle` call at RVA `0x1131e4`. That loop is the
/// ordinary descending Fisher-Yates walk, one bounded Generation-stream draw
/// for every `n = len..2`; a 50-row pool therefore costs 49 draws and the
/// current Largesse pool costs 61, independent of the retained prefix length.
pub fn shuffle_generation_pool<const N: usize>(
    state: &mut HotState,
    pool: &[CardId; N],
) -> Result<[CardId; N], EngineRefusal> {
    let live = state.rng.get(RngStream::Generation);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let mut shuffled = *pool;
    rng.shuffle(&mut shuffled)
        .map_err(|_| EngineRefusal::CounterOverflow("generation shuffle bound"))?;
    state.rng.set(
        RngStream::Generation,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(shuffled)
}

/// Shuffle a generated-table pool whose owner-specific length is dynamic.
///
/// Like [`shuffle_generation_pool`], native consumes a complete Fisher-Yates
/// shuffle even when the caller keeps only a short prefix.
pub fn shuffle_generation_slice(
    state: &mut HotState,
    pool: &[CardId],
) -> Result<Vec<CardId>, EngineRefusal> {
    let live = state.rng.get(RngStream::Generation);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let mut shuffled = pool.to_vec();
    rng.shuffle(&mut shuffled)
        .map_err(|_| EngineRefusal::CounterOverflow("generation shuffle bound"))?;
    state.rng.set(
        RngStream::Generation,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(shuffled)
}

/// Sample one member with replacement from a frozen Generation pool.
///
/// Native `NextItem` consumes exactly one bounded draw even for a one-member
/// pool. The caller repeats this function for the command's complete sample
/// before beginning any generated-card insertion.
pub fn sample_generation_slice(
    state: &mut HotState,
    pool: &[CardId],
) -> Result<CardId, EngineRefusal> {
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs("empty generation sample pool"));
    }
    let live = state.rng.get(RngStream::Generation);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound: i32 = pool
        .len()
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("generation sample bound"))?;
    let selected: usize = rng
        .next_bounded(bound)
        .map_err(|_| EngineRefusal::CounterOverflow("generation sample bound"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("generation sample result"))?;
    state.rng.set(
        RngStream::Generation,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(pool[selected])
}

/// Insert fresh generated cards through exact physical-pile mode.
///
/// Exact pile identity does not imply an instance physical-state slot. Generated
/// cards acquire one only when the Python simulator's enter-combat hooks require
/// it (for example, Infernal Blade immediately marks its result free).
pub fn inject_generated_exact_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        None,
        count,
        GeneratedTransaction::gate_before_record(destination, true),
        events,
    )
}

/// Generate Forge's fresh level-zero Sovereign Blade with its native slot-6
/// payload already attached before the generated-card callbacks run.
pub(crate) fn inject_generated_sovereign_blade(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let identity = CardIdentity {
        id: CardId::SovereignBlade,
        upgrade: 0,
        enchantment: None,
    };
    let instance = CardInstanceState {
        damage_growth: 10,
        ..CardInstanceState::default()
    };
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        Some((crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE, instance)),
        1,
        GeneratedTransaction::record_before_add(PileId::Hand, GeneratedExactMode::Unchanged),
        events,
    )
}

/// Insert Infernal Blade's fresh L0 card with `SetToFreeThisTurn` already
/// applied, before generated-card history and pile insertion.
pub fn inject_generated_free_this_turn_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    let local_cost_modifiers = if spec.cost >= 0 {
        crate::hot::LocalCostModifiers::from_rows(vec![LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount: 0,
            expiration: LocalCostExpiration::ThisTurnOrPlayed,
            reduce_only: false,
        }])
    } else {
        crate::hot::LocalCostModifiers::default()
    };
    let instance = CardInstanceState {
        local_cost_modifiers,
        free_star_cost_this_turn_or_played_rows: 1,
        ..CardInstanceState::default()
    };
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        Some((CARD_FLAG_DEFAULT_PHYSICAL_STATE, instance)),
        1,
        GeneratedTransaction::record_before_add(destination, GeneratedExactMode::WhenLive),
        events,
    )
}

/// Insert Metamorphosis's complete heterogeneous L0 result list.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Metamorphosis/<OnPlay>d__5::MoveNext` RVA `0x3ac1e8` (re-derived for #2560) obtains the full
/// 3/5-card `GetForCombat` list before its foreach, calls
/// `CardModel.SetToFreeThisCombat` on every fresh result (IL `0x00c6-0x00c7`),
/// then awaits one `CardPileCmd.AddGeneratedCardToCombat(card, 1, owner, 3)`
/// per member (IL `0x00cc-0x00d5`, re-derived for #3243): `ldc.i4.1` is
/// `PileType.Draw` and `ldc.i4.3` is `CardPilePosition.Random` (the enums'
/// field constants: PileType None 0, Draw 1, Hand 2, Discard 3;
/// CardPilePosition None 0, Bottom 1, Top 2, Random 3). Each member therefore
/// takes one Shuffle-stream draw over the pre-insertion Draw length plus one,
/// as [`inject_generated_draw_random`] does. Energy's setter skips negative
/// base costs while Star still appends its combat-long row.
///
/// The full payload/closure and every later generated listener are rehearsed
/// before the first history, uid, pile, or callback write. Each member keeps
/// the native singular GateBeforeRecord boundary, so a listener that ends
/// combat suppresses later insertions (and their Shuffle draws) after all
/// Generation draws were already consumed by the caller.
pub(crate) fn inject_generated_free_this_combat_batch_draw_random(
    state: &mut HotState,
    catalog: &Catalog,
    identities: &[CardIdentity],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let mut payloads = Vec::with_capacity(identities.len());
    for &identity in identities {
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        let mut modifiers = LocalCostModifiers::from_rows(if spec.cost >= 0 {
            vec![LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        } else {
            Vec::new()
        });
        modifiers
            .set_free_star_cost_this_combat()
            .ok_or(EngineRefusal::MalformedArgs(
                "Metamorphosis combat-long Star row",
            ))?;
        payloads.push(CardInstanceState {
            local_cost_modifiers: modifiers,
            ..CardInstanceState::default()
        });
    }

    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identities: &[CardIdentity],
        payloads: &[CardInstanceState],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        for (&identity, payload) in identities.iter().zip(payloads) {
            inject_generated_payload_bottom(
                state,
                catalog,
                identity,
                Some((CARD_FLAG_DEFAULT_PHYSICAL_STATE, payload.clone())),
                1,
                GeneratedTransaction::gate_before_record(PileId::Draw, true)
                    .at_random_draw_position(),
                events,
            )?;
        }
        Ok(())
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, identities, &payloads, &mut Vec::new())?;
    apply(state, catalog, identities, &payloads, events)
}

/// Insert Call of the Void's one plural generated-card batch. Every frozen
/// result carries combat-local Ethereal before history, pile insertion,
/// entered-combat work, and generated callbacks. Once the plural command has
/// begun, all members publish history/callback epochs even if an earlier
/// member ended combat; each nested Hand add independently no-ops, and Hand
/// overflow is routed serially to Discard.
pub(crate) fn inject_call_of_the_void_batch_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identities: &[CardIdentity],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let mut payload = CardInstanceState::default();
    payload.set_local_ethereal(true);
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        identities: &[CardIdentity],
        payload: &CardInstanceState,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        for &identity in identities {
            inject_generated_payload_bottom(
                state,
                catalog,
                identity,
                Some((CARD_FLAG_DEFAULT_PHYSICAL_STATE, payload.clone())),
                1,
                GeneratedTransaction::record_before_add(PileId::Hand, GeneratedExactMode::Always),
                events,
            )?;
        }
        Ok(())
    }
    let mut probe = state.clone();
    apply(&mut probe, catalog, identities, &payload, &mut Vec::new())?;
    apply(state, catalog, identities, &payload, events)
}

/// Record and insert exact physical clones of one live source card.
///
/// Juggling clones the pre-body active card, including its slot-presence bits
/// and ordered per-instance state. Fresh uids receive independent copy-on-write
/// state entries; a mutable physical payload also publishes exact-pile mode
/// before the first insertion because source and clone can later diverge.
pub fn inject_generated_clones_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    source: HotCard,
    count: usize,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let identity = catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    let mut instance = state.card_states.get(source.uid);
    // Current v0.111.0 `CardModel::AfterCloned` RVA `0x7d31c` clears the
    // clone's DeckVersion at IL `0x003a..0x0040`; the ordinary mutable clone
    // has already copied Genetic Algorithm's concrete growth fields.
    if source.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0 {
        instance.genetic_algorithm = instance.genetic_algorithm.without_deck_row();
    }
    let exact_mode = if source.flags != 0 || !instance.is_vacant() {
        GeneratedExactMode::Always
    } else {
        GeneratedExactMode::Unchanged
    };
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        Some((source.flags, instance)),
        count,
        GeneratedTransaction::record_before_add(destination, exact_mode).cloned(),
        events,
    )
}

/// Record and insert clones from one exact source after a fused card body has
/// rewritten the clone payload without mutating the live source.
pub fn inject_generated_rewritten_clone_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    source: HotCard,
    flags: u16,
    instance: CardInstanceState,
    destination: PileId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let identity = catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    // Python's generated-card transaction reaches the ending gate before it
    // inserts the separately rewritten physical payload. A lethal fused body
    // therefore records generation epochs but never publishes exact-pile
    // mode: no divergent clone became observable.
    inject_generated_payload_bottom(
        state,
        catalog,
        identity,
        Some((flags, instance)),
        1,
        GeneratedTransaction::record_before_add(destination, GeneratedExactMode::WhenLive).cloned(),
        events,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedEndingPolicy {
    /// The wrapper has not begun another generated-card command once combat
    /// is ending (for example each separate `Shiv.CreateInHand` call).
    GateBeforeRecord,
    /// The generated-card command has begun: publish its history/listener
    /// epoch before the nested pile Add tests `IsOverOrEnding`.
    RecordBeforeAddGate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedExactMode {
    Unchanged,
    WhenLive,
    Always,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GeneratedTransaction {
    destination: PileId,
    ending_policy: GeneratedEndingPolicy,
    exact_mode: GeneratedExactMode,
    is_clone: bool,
    /// `CardPilePosition.Random` (3): the nested Add draws one Shuffle-stream
    /// index in `0..=Draw.len()` instead of appending at the bottom. Only a
    /// Draw destination may carry it.
    random_draw: bool,
}

impl GeneratedTransaction {
    const fn gate_before_record(destination: PileId, exact: bool) -> Self {
        Self {
            destination,
            ending_policy: GeneratedEndingPolicy::GateBeforeRecord,
            exact_mode: if exact {
                GeneratedExactMode::WhenLive
            } else {
                GeneratedExactMode::Unchanged
            },
            is_clone: false,
            random_draw: false,
        }
    }

    const fn record_before_add(destination: PileId, exact_mode: GeneratedExactMode) -> Self {
        Self {
            destination,
            ending_policy: GeneratedEndingPolicy::RecordBeforeAddGate,
            exact_mode,
            is_clone: false,
            random_draw: false,
        }
    }

    const fn cloned(mut self) -> Self {
        self.is_clone = true;
        self
    }

    const fn at_random_draw_position(mut self) -> Self {
        self.random_draw = true;
        self
    }
}

/// Shared generated-card transaction, optionally seeded from an exact source
/// payload rather than from a fresh identity. The caller selects whether its
/// outer command has already begun before the nested Add's ending gate.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `CardPileCmd/<AddGeneratedCardsToCombat>d__6::MoveNext` RVA `0x3e2f0c`
/// calls `CombatHistory.CardGenerated` at IL `0x0104–0x0120`, then nested
/// `CardPileCmd.Add` at IL `0x0131–0x0155`, then
/// `Hook.AfterCardGeneratedForCombat` at IL `0x01c6–0x01d8`. The nested
/// `CardPileCmd/<Add>d__10::MoveNext` RVA `0x3e1ba4` tests combat ending at
/// IL `0x0041–0x0058` and returns failed add results at IL `0x005a–0x008a`.
/// Anger RVA `0x389cd8` awaits its attack before `CreateClone`/generated Add
/// at IL `0x00dd–0x00ed`; Infernal Blade RVA `0x3a7164` sets its selected
/// card free before the same generated Add at IL `0x009b–0x00ad`.
///
/// Python mirrors those command boundaries in `add_generated_shivs`
/// (frozen Python, deleted #2827), `add_generated_card_clones`, and `add_infernal_blade_card`.
fn inject_generated_payload_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    payload: Option<(u16, CardInstanceState)>,
    count: usize,
    transaction: GeneratedTransaction,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    preflight_generated_power_batch(
        state,
        count,
        transaction.ending_policy == GeneratedEndingPolicy::RecordBeforeAddGate,
    )?;
    if count > 0
        && match transaction.exact_mode {
            GeneratedExactMode::Unchanged => false,
            GeneratedExactMode::WhenLive => !state.history.over,
            GeneratedExactMode::Always => true,
        }
    {
        state.exact_piles = true;
    }
    for _ in 0..count {
        if state.history.over
            && transaction.ending_policy == GeneratedEndingPolicy::GateBeforeRecord
        {
            break;
        }
        // AddGeneratedCardToCombat publishes CardGenerated history and its
        // callback epoch before the nested Add tests IsOverOrEnding. Anger
        // makes this ordering reachable when its awaited attack is lethal;
        // Infernal Blade and every other wrapper share the same transaction
        // once their caller has chosen to begin generation.
        state.history.owner_generated_cards_combat = state
            .history
            .owner_generated_cards_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat",
            ))?;
        state.next_generated_hook_uid = state
            .next_generated_hook_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        if state.history.over {
            after_owner_card_generated(state, catalog, spec, events)?;
            continue;
        }
        // `CardPilePosition.Random`: one Shuffle-stream draw bounded by the
        // pre-insertion Draw length plus one, taken before the physical card
        // is minted — the same order as [`inject_generated_draw_random`].
        let random_index = if transaction.random_draw {
            if transaction.destination != PileId::Draw {
                return Err(EngineRefusal::MalformedArgs(
                    "random generated insertion outside Draw",
                ));
            }
            let bound: i32 = state
                .piles
                .get(PileId::Draw)
                .len()
                .checked_add(1)
                .and_then(|value| value.try_into().ok())
                .ok_or(EngineRefusal::CounterOverflow(
                    "random draw insertion bound",
                ))?;
            let live = state.rng.get(RngStream::Rng);
            let mut rng = Xoshiro256StarStar {
                words: live.words,
                counter: live.counter,
            };
            let index: usize = rng
                .next_bounded(bound)
                .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion bound"))?
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion index"))?;
            state.rng.set(
                RngStream::Rng,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            Some(index)
        } else {
            None
        };
        let mut card = mint_card(state, catalog, identity)?;
        if let Some((flags, instance)) = &payload {
            card.flags = *flags;
            if !instance.is_vacant() {
                state.card_states.set(card.uid, instance.clone());
            }
        }
        if transaction.is_clone {
            apply_physical_card_after_entered_inner(state, catalog, spec, &mut card, true)?;
        } else {
            apply_physical_card_after_entered(state, catalog, spec, &mut card)?;
        }
        let pile = if transaction.destination == PileId::Hand
            && state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND
        {
            PileId::Discard
        } else {
            transaction.destination
        };
        match random_index {
            Some(index) => state.piles.get_mut(pile).make_mut().insert(index, card),
            None => state.piles.get_mut(pile).make_mut().push(card),
        }
        // Fresh entry listeners can create a payload different from an
        // otherwise equal live sibling (notably Sword Sage gives the current
        // amount while a prior source may have granted excess replay). The
        // exact-pile projection becomes observable at insertion, not at the
        // later selection that first notices the tie.
        if !state.exact_piles
            && !transaction.is_clone
            && spec.identity.id == CardId::SovereignBlade
            && state.powers.value(PowerId::SwordSage) > 0
            && live_cards_need_exact_piles(state, catalog)?
        {
            state.exact_piles = true;
        }
        if spec.strike_tag {
            state.ps_strikes = state
                .ps_strikes
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
        }
        events.push(Event::CardResolved {
            uid: card.uid,
            pile,
        });
        after_owner_card_generated(state, catalog, spec, events)?;
    }
    Ok(())
}

fn preflight_generated_power_batch(
    state: &HotState,
    count: usize,
    record_every_command: bool,
) -> Result<(), EngineRefusal> {
    if record_every_command {
        let count: i32 = count
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("generated power batch"))?;
        state
            .history
            .owner_generated_cards_combat
            .checked_add(count)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat",
            ))?;
        state
            .next_generated_hook_uid
            .checked_add(count)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    }
    if state.history.over {
        return Ok(());
    }
    let count: i64 = count
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("generated power batch"))?;
    let arsenal = i64::from(state.powers.value(PowerId::Arsenal));
    let strength = i64::from(state.powers.value(PowerId::Strength));
    strength
        .checked_add(
            arsenal
                .checked_mul(count)
                .ok_or(EngineRefusal::CounterOverflow("generated power batch"))?,
        )
        .filter(|value| i32::try_from(*value).is_ok())
        .ok_or(EngineRefusal::CounterOverflow("generated power batch"))?;

    let pillar = i64::from(state.powers.value(PowerId::PillarOfCreation));
    let block = i64::from(state.block);
    block
        .checked_add(
            pillar
                .checked_mul(count)
                .ok_or(EngineRefusal::CounterOverflow("generated power batch"))?,
        )
        .filter(|value| *value <= 999_999_999)
        .ok_or(EngineRefusal::CounterOverflow("generated power batch"))?;
    Ok(())
}

/// Preflight Call of the Void's unknown heterogeneous batch before it
/// consumes Generation RNG. Every result records history/hook epochs; every
/// still-live result can allocate a uid, and any pool member may carry Strike.
pub(crate) fn preflight_call_of_the_void_batch(
    state: &HotState,
    count: usize,
) -> Result<(), EngineRefusal> {
    preflight_generated_power_batch(state, count, true)?;
    if state.history.over {
        return Ok(());
    }
    let count_u32 = u32::try_from(count)
        .map_err(|_| EngineRefusal::CounterOverflow("Call of the Void batch"))?;
    let count_i32 = i32::try_from(count)
        .map_err(|_| EngineRefusal::CounterOverflow("Call of the Void batch"))?;
    state
        .next_card_uid
        .checked_add(count_u32)
        .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
    state
        .ps_strikes
        .checked_add(count_i32)
        .ok_or(EngineRefusal::CounterOverflow("ps_strikes"))?;
    Ok(())
}

/// Begin one generated-card transaction whose creator and recipient are the
/// represented remote Player. It publishes no local owner-generated history
/// and allocates no local physical uid. The combat-wide callback epoch still
/// advances, but current-build Arsenal, Pillar, Smokestack, Trash to Treasure,
/// Soulbound, Regalite, Rocket Punch and Aeonglass callbacks all reject this
/// creator/card-owner relation before mutation. Identity-specific generated
/// callbacks are likewise no-ops for the currently admitted remote payloads.
pub(crate) fn begin_remote_generated_card(state: &mut HotState) -> Result<(), EngineRefusal> {
    state.next_generated_hook_uid = state
        .next_generated_hook_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    Ok(())
}

/// Complete [`begin_remote_generated_card`] after the payload-only remote
/// insertion has committed.
pub(crate) fn finish_remote_generated_card(
    _state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    _events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    Ok(())
}

/// Insert one remote-owned card at a random remote Draw position while the
/// local owner remains the native `CardGenerated` creator.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `GlimpseBeyond/<OnPlay>d__9::MoveNext` RVA `0x3a1968` creates each
/// recipient's complete Soul list at IL `0x0103-0x012e`, then passes the
/// Glimpse owner as `creator` to the separately awaited plural
/// `AddGeneratedCardsToCombat` at IL `0x0130-0x019b`. The plural command's
/// state machine RVA `0x3e2f0c` records history before each random Draw Add
/// at IL `0x0104-0x0155`, then awaits `AfterCardGeneratedForCombat` at
/// IL `0x01c6-0x0230`.
///
/// A remote-owned Soul has no representable local physical uid or card-entry
/// listener. It still consumes the combat-wide Shuffle stream and runs every
/// local creator-sensitive generated-card callback. This helper deliberately
/// records before the nested Add's ending gate, matching later members of a
/// plural call after an earlier callback ended combat.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn inject_owner_created_remote_draw_random_record_before_ending(
    state: &mut HotState,
    catalog: &Catalog,
    remote_key: u32,
    identity: CardIdentity,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        remote_key: u32,
        identity: CardIdentity,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        if remote_key != 1
            || u32::from(state.multiplayer_ally_key) != remote_key
            || !state.fanouts.multiplayer_ally().alive
        {
            return Err(EngineRefusal::MalformedArgs(
                "Glimpse Beyond remote recipient",
            ));
        }
        preflight_generated_power_batch(state, 1, true)?;
        state.history.owner_generated_cards_combat = state
            .history
            .owner_generated_cards_combat
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat",
            ))?;
        state.next_generated_hook_uid = state
            .next_generated_hook_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
        if state.history.over {
            return after_owner_created_remote_card_generated(state, catalog, spec, events);
        }

        let bound: i32 = state
            .fanouts
            .multiplayer_ally()
            .draw
            .len()
            .checked_add(1)
            .and_then(|value| value.try_into().ok())
            .ok_or(EngineRefusal::CounterOverflow(
                "remote random draw insertion bound",
            ))?;
        let live = state.rng.get(RngStream::Rng);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let index: usize = rng
            .next_bounded(bound)
            .map_err(|_| EngineRefusal::CounterOverflow("remote random draw insertion bound"))?
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("remote random draw insertion index"))?;
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: rng.words,
                counter: rng.counter,
            },
        );
        Arc::make_mut(&mut state.fanouts.multiplayer_ally_mut().draw).insert(
            index,
            crate::hot::MultiplayerAllyCard::immutable(identity).ok_or(
                EngineRefusal::MalformedArgs("mutable remote generated card"),
            )?,
        );
        after_owner_created_remote_card_generated(state, catalog, spec, events)
    }

    let mut probe = state.clone();
    apply(&mut probe, catalog, remote_key, identity, &mut Vec::new())?;
    apply(state, catalog, remote_key, identity, events)
}

/// Acquisition-ordered local player-power callbacks after one owner-created
/// card enters combat. Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `TrashToTreasurePower/<AfterCardGeneratedForCombat>d__4::MoveNext` RVA
/// `0x349f90` IL `0x0020-0x0053` requires Status plus the exact owner creator
/// before its serial random-Orb loop. Arsenal, Pillar of Creation, Smokestack
/// and Regalite gate on the same owner creator (#3256, cited in
/// `after_local_card_generated_inner`).
pub(crate) fn after_owner_card_generated(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    after_local_card_generated(state, catalog, spec, true, true, events)
}

fn after_owner_created_remote_card_generated(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    after_local_card_generated(state, catalog, spec, true, false, events)
}

/// Acquisition-ordered callbacks for a generated card whose creator is null.
/// Native Trash to Treasure, Arsenal, Pillar of Creation, Smokestack and
/// Regalite all require an owner creator, so none of them fires (#3256).
fn after_null_card_generated(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    after_local_card_generated(state, catalog, spec, false, true, events)
}

fn after_local_card_generated(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    creator_is_owner: bool,
    generated_owner_is_local: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let relation = GeneratedCardRelation {
        creator_is_owner,
        generated_owner_is_local,
    };
    let rocket_uids =
        if !state.history.over && creator_is_owner && generated_owner_is_local && spec.is_status {
            frozen_rocket_punch_listener_uids(state, catalog)?
        } else {
            Vec::new()
        };
    let aeonglass_listener = if !state.history.over
        && generated_owner_is_local
        && spec.identity.id == CardId::Wither
        && state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::Aeonglass && monster.hp > 0)
    {
        if !aeonglass_generated_transient_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass generated Wither entry",
            ));
        }
        let generated_uid =
            state
                .next_card_uid
                .checked_sub(1)
                .ok_or(EngineRefusal::MalformedArgs(
                    "Aeonglass generated Wither uid",
                ))?;
        Some(AeonglassGeneratedListener {
            boss_uid: state.monsters[0].uid,
            generated_uid,
            upgrade_count: state.monsters[0].aeonglass_wither_upgrade_count(),
        })
    } else {
        None
    };
    if rocket_uids.is_empty() && aeonglass_listener.is_none() {
        return after_local_card_generated_inner(
            state,
            catalog,
            spec,
            GeneratedCardListenerPlan {
                relation,
                rocket_uids: &rocket_uids,
                aeonglass_listener,
                record_test_trace: true,
            },
            events,
        );
    }

    // Native freezes the complete physical-card listener layer before any
    // earlier power listener mutates combat. Rehearse that entire serial
    // transaction so malformed Rocket state or a late power refusal cannot
    // publish a prefix. Keep the long-standing zero-Rocket path clone-free.
    let mut probe = state.clone();
    after_local_card_generated_inner(
        &mut probe,
        catalog,
        spec,
        GeneratedCardListenerPlan {
            relation,
            rocket_uids: &rocket_uids,
            aeonglass_listener,
            record_test_trace: false,
        },
        &mut Vec::new(),
    )?;
    #[cfg(test)]
    ROCKET_PUNCH_GENERATED_TRACE.with(|trace| trace.borrow_mut().extend_from_slice(&rocket_uids));
    after_local_card_generated_inner(
        state,
        catalog,
        spec,
        GeneratedCardListenerPlan {
            relation,
            rocket_uids: &rocket_uids,
            aeonglass_listener,
            record_test_trace: true,
        },
        events,
    )
}

#[derive(Clone, Copy)]
struct GeneratedCardRelation {
    creator_is_owner: bool,
    generated_owner_is_local: bool,
}

#[derive(Clone, Copy)]
struct AeonglassGeneratedListener {
    boss_uid: u32,
    generated_uid: u32,
    upgrade_count: i32,
}

#[derive(Clone, Copy)]
struct GeneratedCardListenerPlan<'a> {
    relation: GeneratedCardRelation,
    rocket_uids: &'a [u32],
    aeonglass_listener: Option<AeonglassGeneratedListener>,
    record_test_trace: bool,
}

fn after_local_card_generated_inner(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    plan: GeneratedCardListenerPlan<'_>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let GeneratedCardListenerPlan {
        relation,
        rocket_uids,
        aeonglass_listener,
        record_test_trace,
    } = plan;
    #[cfg(not(test))]
    let _ = record_test_trace;
    let order = state.fanouts.local_generated_power_order().to_vec();
    for power in order {
        if state.history.over {
            break;
        }
        let amount = state.powers.value(power);
        if amount <= 0 {
            continue;
        }
        #[cfg(test)]
        if record_test_trace {
            GENERATED_LISTENER_TRACE.with(|trace| {
                trace
                    .borrow_mut()
                    .push(GeneratedListenerTrace::Power(power))
            });
        }
        // Every local generated-card power below opens with the same creator
        // gate (v0.111.0 DLL 9cb4f1ad…): `creator == null` leaves, and so does
        // `creator.Creature != Owner`, before any mutation (#3256).
        // `ArsenalPower/<AfterCardGeneratedForCombat>d__6::MoveNext` RVA
        // `0x334ec8` IL_001d-IL_0038, `PillarOfCreationPower/<…>d__6` RVA
        // `0x340488` IL_001d-IL_0038, `SmokestackPower/<…>d__4` RVA
        // `0x3455e8` IL_0033-IL_004e (after its Status test, IL_0020-IL_002e).
        // `Hook.AfterCardGeneratedForCombat` (`<…>d__14` RVA `0x3ccaa4`
        // IL_0046-IL_0058) and `CardPileCmd/<AddGeneratedCardsToCombat>d__6`
        // (RVA `0x3e2f0c` IL_01c6-IL_01d8) forward the command's `creator`
        // unchanged, so a null-creator card (Personal Hive's Dazed,
        // `PersonalHivePower/<AfterDamageReceived>d__6` RVA `0x340220`
        // IL_00cd-IL_00d1 passes `ldnull`) fires none of them.
        match power {
            PowerId::Arsenal | PowerId::PillarOfCreation | PowerId::Smokestack
                if !relation.creator_is_owner => {}
            PowerId::Arsenal => {
                super::damage::apply_owner_strength(state, amount, events)?;
            }
            PowerId::PillarOfCreation => {
                super::orbs::gain_flat_block(state, catalog, i64::from(amount), events)?;
            }
            PowerId::TrashToTreasure if relation.creator_is_owner && spec.is_status => {
                super::orbs::channel_random_loop(state, catalog, amount as u32, events)?;
            }
            PowerId::TrashToTreasure => {}
            PowerId::Smokestack if spec.is_status => {
                // `SmokestackPower/<AfterCardGeneratedForCombat>d__4::MoveNext`
                // RVA `0x3455e8` (v0.111.0 DLL 9cb4f1ad…) awaits ONE
                // `CreatureCmd.Damage(choiceContext, HittableEnemies, Amount,
                // ValueProp 4, Owner)` (IL_005f-IL_007b): every target is
                // committed before any result or death listener runs, and each
                // death's AfterDeath walk keeps this command's catalog for
                // Gremlin Horn's Draw (#3172). Same batch shape as Hailstorm.
                let targets = super::damage::alive_targets(state);
                super::damage::damage_monsters_after_catalog_auth(
                    state,
                    catalog,
                    &targets,
                    DotNetDecimal::from_i64(i64::from(amount)),
                    true,
                    events,
                )?;
            }
            PowerId::Smokestack => {}
            _ => {
                return Err(EngineRefusal::PowerOrderNotModeled(
                    "local generated power order",
                ));
            }
        }
    }
    // `Regalite/<AfterCardGeneratedForCombat>d__8::MoveNext` RVA `0x32f988`
    // IL_0020-IL_0036: a null `creator` leaves, and so does a creator other
    // than the relic's Owner; the generated card's own owner is never read
    // (#3256). So Regalite answers exactly the owner-created relation,
    // including a local Glimpse Beyond Soul inserted into the remote ally's
    // Draw, and never a null-creator card.
    if !state.history.over
        && relation.creator_is_owner
        && !state.fanouts.regalite_used_this_turn()
        && catalog.hooks().owns(RelicId::RelicRegalite)
    {
        state.fanouts.set_regalite_used_this_turn(true);
        super::orbs::gain_flat_block(
            state,
            catalog,
            i64::from(state.regalite_block_amount),
            events,
        )?;
        crate::coverage::record_relic(RelicId::RelicRegalite);
    }
    if !state.history.over
        && relation.creator_is_owner
        && relation.generated_owner_is_local
        && spec.is_status
    {
        for &uid in rocket_uids {
            #[cfg(test)]
            if record_test_trace {
                GENERATED_LISTENER_TRACE.with(|trace| {
                    trace
                        .borrow_mut()
                        .push(GeneratedListenerTrace::RocketPunch(uid))
                });
            }
            let (pile, index) = unique_live_card_location(state, uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            let card = state.piles.get(pile).as_slice()[index];
            if !rocket_punch_program_is_exact(catalog, card)
                || card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Rocket Punch generated listener state",
                ));
            }
            state.card_states.append_local_cost_modifier(
                uid,
                LocalCostModifier {
                    kind: LocalCostModifierKind::Add,
                    amount: -1,
                    expiration: LocalCostExpiration::UntilPlayed,
                    reduce_only: false,
                },
            );
        }
        if !state.exact_piles && !rocket_uids.is_empty() {
            state.exact_piles = true;
        }
    }
    if let Some(frozen) = aeonglass_listener {
        let boss = state.monsters.first().ok_or(EngineRefusal::MalformedArgs(
            "Aeonglass generated listener owner",
        ))?;
        if boss.kind != MonsterKind::Aeonglass
            || boss.uid != frozen.boss_uid
            || boss.aeonglass_wither_upgrade_count() != frozen.upgrade_count
        {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass generated listener identity",
            ));
        }
        let (pile, index) = unique_live_card_location(state, frozen.generated_uid)?.ok_or(
            EngineRefusal::MalformedArgs("Aeonglass generated listener card"),
        )?;
        let card = state.piles.get(pile).as_slice()[index];
        let live = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if live.identity.id != CardId::Wither
            || card.flags != CARD_FLAG_DEFAULT_PHYSICAL_STATE
            || frozen.upgrade_count < 0
        {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass generated listener card",
            ));
        }
        let delta = frozen
            .upgrade_count
            .checked_mul(3)
            .ok_or(EngineRefusal::CounterOverflow("Wither generated growth"))?;
        state
            .card_states
            .add_damage_growth(frozen.generated_uid, delta)
            .ok_or(EngineRefusal::CounterOverflow("Wither generated growth"))?;
    }
    Ok(())
}

/// Exact immutable Rocket Punch rows and physical generated-listener body.
pub(crate) fn rocket_punch_program_is_exact(catalog: &Catalog, card: HotCard) -> bool {
    let Some(spec) = catalog.spec(card.atom) else {
        return false;
    };
    let expected_damage = 13 + i64::from(spec.identity.upgrade);
    let expected_draw = 1 + i64::from(spec.identity.upgrade);
    matches!(
        (spec.identity.id, spec.identity.upgrade),
        (CardId::RocketPunch, 0 | 1)
    ) && crate::engine::play::body_enchantment_is_exact(spec)
        && spec.cost == 2
        && spec.is_attack
        && spec.targeted
        && spec.target_type == CardTargetType::AnyEnemy
        && !spec.exhausts
        && catalog.steps(spec).len() == 2
        && catalog.steps(spec)[0].kind == crate::ids::StepKind::Attack
        && catalog.args(catalog.steps(spec)[0].args)
            == [
                crate::catalog::CompiledArg::I(expected_damage),
                crate::catalog::CompiledArg::I(1),
            ]
        && catalog.steps(spec)[1].kind == crate::ids::StepKind::Draw
        && catalog.args(catalog.steps(spec)[1].args)
            == [crate::catalog::CompiledArg::I(expected_draw)]
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
}

/// Freeze every exact physical Rocket in native AllCards order.
fn frozen_rocket_punch_listener_uids(
    state: &HotState,
    catalog: &Catalog,
) -> Result<Vec<u32>, EngineRefusal> {
    const ROCKET_ORDER: [PileId; 5] = [
        PileId::Hand,
        PileId::Draw,
        PileId::Discard,
        PileId::Exhaust,
        PileId::Play,
    ];
    let mut frozen = Vec::new();
    for pile in ROCKET_ORDER {
        for (index, card) in state.piles.get(pile).as_slice().iter().copied().enumerate() {
            let Some(card_spec) = catalog.spec(card.atom) else {
                return Err(EngineRefusal::UnknownAtom(card.atom));
            };
            if card_spec.identity.id != CardId::RocketPunch {
                continue;
            }
            if card.flags & CARD_FLAG_LEGACY != 0
                || card.uid >= state.next_card_uid
                || !rocket_punch_program_is_exact(catalog, card)
                || unique_live_card_location(state, card.uid)? != Some((pile, index))
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Rocket Punch generated listener snapshot",
                ));
            }
            frozen.push(card.uid);
        }
    }
    Ok(frozen)
}

/// Insert compact legacy card tuples, matching monster `_pile_inject`.
/// These entries deliberately do not advance `next_card_uid`; the native
/// normalization pass does that later in all-piles order.
pub fn inject_legacy_bottom(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
    pile: PileId,
) -> Result<(), EngineRefusal> {
    let atom = catalog
        .atom(&identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    for _ in 0..count {
        if state.history.over {
            break;
        }
        let destination =
            if pile == PileId::Hand && state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
                PileId::Discard
            } else {
                pile
            };
        state.piles.get_mut(destination).make_mut().push(HotCard {
            uid: LEGACY_CARD_UID,
            atom,
            flags: CARD_FLAG_LEGACY
                | if identity.id == CardId::Melancholy {
                    crate::hot::CARD_FLAG_MELANCHOLY
                } else {
                    0
                },
        });
    }
    Ok(())
}

/// Insert compact legacy card tuples at independently random Draw positions.
///
/// Python `_insert_draw_random` (frozen, deleted #2827) calls `NextInt(size + 1)` on
/// the Shuffle stream once per copy and inserts immediately, so later copies
/// observe the already-grown pile. Resolve the identity before either the
/// pile or RNG can mutate.
pub fn inject_legacy_draw_random(
    state: &mut HotState,
    catalog: &Catalog,
    identity: CardIdentity,
    count: usize,
) -> Result<(), EngineRefusal> {
    let atom = catalog
        .atom(&identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    for _ in 0..count {
        if state.history.over {
            break;
        }
        let bound: i32 = state
            .piles
            .get(PileId::Draw)
            .len()
            .checked_add(1)
            .and_then(|value| value.try_into().ok())
            .ok_or(EngineRefusal::CounterOverflow(
                "random draw insertion bound",
            ))?;
        let live = state.rng.get(RngStream::Rng);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let index: usize = rng
            .next_bounded(bound)
            .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion bound"))?
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("random draw insertion index"))?;
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: rng.words,
                counter: rng.counter,
            },
        );
        state.piles.get_mut(PileId::Draw).make_mut().insert(
            index,
            HotCard {
                uid: LEGACY_CARD_UID,
                atom,
                flags: CARD_FLAG_LEGACY
                    | if identity.id == CardId::Melancholy {
                        crate::hot::CARD_FLAG_MELANCHOLY
                    } else {
                        0
                    },
            },
        );
    }
    Ok(())
}

/// Authenticate the immutable identity cache used by catalog-free death paths.
pub(crate) fn melancholy_identity_markers_are_exact(state: &HotState, catalog: &Catalog) -> bool {
    PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .all(|card| {
            catalog.spec(card.atom).is_some_and(|spec| {
                (card.flags & crate::hot::CARD_FLAG_MELANCHOLY != 0)
                    == (spec.identity.id == CardId::Melancholy)
            })
        })
}

/// Native Melancholy::AfterDeath (v0.111.0 RVA 0xe5228) skips prevented
/// deaths, then discounts each physical copy still in any combat pile.
/// get_CanonicalVars (0xe51a9) fixes Energy=1 at both levels; AddThisCombat
/// (0x11e28d) appends Add(-1), expiration None, reduceOnly=false. Call this
/// only for an actual player, pet or enemy death, before the terminal suffix.
/// The marker is derived at import/generation and authenticated at public
/// boundaries; it lets power-owned, catalog-free death callbacks use the same
/// exact physical listener as card-owned damage.
pub(crate) fn physical_cards_after_actual_death(state: &mut HotState) -> Result<(), EngineRefusal> {
    // Native CombatState.Contains0x137564 excludes an owner's cards after
    // Player.DeactivateHooks, even when a previously queued pet now dies.
    if state.fanouts.player_hooks_deactivated() {
        return Ok(());
    }
    normalize_card_identities(state)?;
    let uids: Vec<_> = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .filter(|card| card.flags & crate::hot::CARD_FLAG_MELANCHOLY != 0)
        .map(|card| card.uid)
        .collect();
    for uid in &uids {
        state.card_states.append_local_cost_modifier(
            *uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
    }
    if !uids.is_empty() {
        for pile in PileId::ALL {
            for card in state.piles.get_mut(pile).make_mut() {
                if card.flags & crate::hot::CARD_FLAG_MELANCHOLY != 0 {
                    card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                }
            }
        }
        // Cost rows distinguish physical siblings; preserve exact pile order.
        state.exact_piles = true;
    }
    Ok(())
}

/// Assign physical identities to every legacy pile entry in native
/// `AllPiles` order: Hand, Draw, Discard, Exhaust, Play.
pub fn normalize_card_identities(state: &mut HotState) -> Result<(), EngineRefusal> {
    for pile in [
        PileId::Hand,
        PileId::Draw,
        PileId::Discard,
        PileId::Exhaust,
        PileId::Play,
    ] {
        if !state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .any(|card| card.flags & CARD_FLAG_LEGACY != 0)
        {
            continue;
        }
        for card in state.piles.get_mut(pile).make_mut() {
            if card.flags & CARD_FLAG_LEGACY == 0 {
                continue;
            }
            card.uid = state.next_card_uid;
            state.next_card_uid = state
                .next_card_uid
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
            card.flags &= !CARD_FLAG_LEGACY;
        }
    }
    Ok(())
}

/// Select one exact canonical L0 Power for Creative AI.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `CreativeAiPower/<BeforeHandDraw>d__4::MoveNext` (RVA `0x338180`)
/// verifies the owning Player, filters its fully unlocked solo character pool to
/// in-combat-generatable Power cards, and calls
/// `CardFactory::GetDistinctForCombat(..., 1, CombatCardGeneration)`.
/// `GetDistinctForCombat` (RVA `0x112878`) delegates to `TakeRandom`
/// (RVA `0x11318d`), whose complete-list shuffle consumes 18 Generation
/// draws for Defect's 19-card pool; other owners consume their pool size minus one.
pub(crate) fn select_creative_ai_power(
    state: &mut HotState,
    catalog: &Catalog,
) -> Result<CardIdentity, EngineRefusal> {
    let pool = exact_owner_listener_pool(state, catalog, PowerId::CreativeAi)?;
    let shuffled = shuffle_generation_slice(state, &pool)?;
    Ok(CardIdentity {
        id: shuffled[0],
        upgrade: 0,
        enchantment: None,
    })
}

/// Whether this fight records the unlock profile an owner-pool generator
/// filters by (#2560, #2946).
///
/// Every owner-pool generator reads `Owner.Character.CardPool` and filters it
/// through `CardPoolModel::GetUnlockedCards(Owner.UnlockState, ..)` (RVA
/// `0x7e54c`, `FilterThroughEpochs` at IL_0014). Each character pool's
/// override (Ironclad `0xf1f98`, Silent `0xf2cf8`, Regent `0xf28e0`,
/// Necrobinder `0xf2468`, Defect `0xf1944`) makes exactly three
/// `IsEpochRevealed` tests (IL_0014, IL_0042, IL_0070), each removing one of
/// that character's own epoch-gated rows, which is the per-row `unlock_epoch`
/// column of `CHARACTER_CARD_POOL_ROWS_V1101`. So the pool a generator draws
/// is a function of the owner's gating epochs alone, and
/// [`crate::steps::neutral::derive_character_generation_pool`] reproduces it
/// from ANY recorded profile, not only one that reveals all twenty gating
/// epochs of all six pools. What a generator needs is therefore that the
/// profile is recorded at all:
///
/// * `fully_unlocked_card_pool_epochs` — the document's profile reveals every
///   card-pool gating epoch, so the catalog stores none and every derivation
///   takes its fully-unlocked (`None`) projection; or
/// * `catalog.splash_unlock_epochs()` is `Some` — a partial profile, which the
///   catalog carries and every `Catalog::*_pool` derivation filters by.
///
/// A document that omits `splash_unlock_epochs` has neither, and every caller
/// keeps refusing by name. Splash already admitted on exactly this predicate
/// (#2469); this names it so the owner-pool generators can share it.
pub(crate) fn unlock_profile_is_recorded(state: &HotState, catalog: &Catalog) -> bool {
    state.fully_unlocked_card_pool_epochs || catalog.splash_unlock_epochs().is_some()
}

/// Shared provenance for a generator that draws from the OWNER's unlocked
/// CharacterCardPool and nothing else (#2560, #2946).
///
/// Discovery (`0x399254` IL_003e-IL_0063), Jackpot (`0x3a80d4`
/// IL_00e6-IL_010b), Stoke (`0x3bef44` IL_0194-IL_01b9), Metamorphosis
/// (`Metamorphosis/<OnPlay>d__5::MoveNext` `0x3ac1e8` IL_002c-IL_0051),
/// CalamityPower (`<AfterCardPlayed>d__7::MoveNext` `0x3368bc`
/// IL_005a-IL_0089) and the three persistent listeners below each read
/// `Owner.Character.CardPool` and `Owner.UnlockState` and nothing about
/// Entropy. So:
///
/// * the owner is `reward_card_pool`, which must be present;
/// * `entropy_card_pool` adds no input. An explicit CONTRADICTORY value is
///   still refused, as the listener gate always did, but an absent one — the
///   Rust opening writes it only for a solo Ironclad or Regent — no longer
///   refuses a Necrobinder, Defect or Silent root (#2946 conjunct 1; the Jack
///   of All Trades precedent, #2554);
/// * the profile must be recorded ([`unlock_profile_is_recorded`]) rather than
///   fully unlocked for all six pools (#2946 conjunct 2, #2560);
/// * solo only: `GetUnlockedCards`' multiplayer branch keeps rows the
///   generated table pre-filters out;
/// * a live Generation stream.
pub(crate) fn owner_pool_generation_provenance_is_exact(
    state: &HotState,
    catalog: &Catalog,
) -> bool {
    state
        .entropy_card_pool
        .is_none_or(|p| Some(p) == state.reward_card_pool)
        && owner_pool_profile_provenance_is_exact(state, catalog)
}

/// [`owner_pool_generation_provenance_is_exact`] without its Entropy-owner
/// consistency conjunct, for the two generators that never consulted
/// `entropy_card_pool` at all — Distraction and White Noise, whose gates
/// admitted an unrelated Entropy owner before #2560 and still do
/// (`distraction_admits_only_exact_solo_silent_generation_provenance`). The
/// profile half is the same: any recorded profile, solo, live Generation.
pub(crate) fn owner_pool_profile_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    state.reward_card_pool.is_some()
        && unlock_profile_is_recorded(state, catalog)
        && state.multiplayer_ally_key == 0
        && !state.rng.is_vacant(RngStream::Generation)
}

/// These listeners query CharacterCardPool, independently of Entropy.
/// Reward ownership is required; an explicit contradictory Entropy owner is
/// rejected, while a legacy absent Entropy field adds no missing input here.
///
/// CreativeAiPower (`0x338180` IL_0050-IL_0075), HelloWorldPower (`0x33bfd0`
/// IL_0062-IL_0091) and CallOfTheVoidPower (`0x336b08` IL_0043-IL_0068) each
/// read `Owner.Character.CardPool` through
/// `GetUnlockedCards(Owner.UnlockState, ..)`, so since #2560 a partial
/// recorded profile is exact provenance: [`exact_owner_listener_pool`] draws
/// from `Catalog::owner_listener_pool`, which filters by it.
pub(crate) fn owner_listener_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    owner_pool_generation_provenance_is_exact(state, catalog)
}

/// Complete native owner-pool proof shared by the three persistent factories.
/// Native power readers query owner CharacterCardPool; the recorded full
/// unlock epochs derive it without a redundant Python class-specific list.
/// Every result must be cataloged before any live RNG is consumed.
pub(crate) fn exact_owner_listener_pool(
    state: &HotState,
    catalog: &Catalog,
    power: PowerId,
) -> Result<Vec<CardId>, EngineRefusal> {
    let message = match power {
        PowerId::CreativeAi => "Creative AI generation provenance",
        PowerId::HelloWorld => "Hello World generation provenance",
        PowerId::CallOfTheVoid => "Call of the Void generation provenance",
        _ => return Err(EngineRefusal::MalformedArgs("owner listener pool")),
    };
    if !owner_listener_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(message));
    }
    let pool = catalog.owner_listener_pool(state.reward_card_pool.unwrap(), power);
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs(message));
    }
    for id in &pool {
        let identity = CardIdentity {
            id: *id,
            upgrade: 0,
            enchantment: None,
        };
        catalog
            .atom(&identity)
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    }
    Ok(pool)
}

/// Shared provenance for the Colorless generators that draw
/// `Catalog::colorless_generation_pool`: Quasar, Spectrum Shift, Bundle of Joy
/// and Manifest Authority (the Colorless half of #2560).
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`):
/// `Quasar/<OnPlay>d__3::MoveNext` RVA `0x3b4e48` IL_0027-IL_004d,
/// `SpectrumShiftPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x345978`
/// IL_0038-IL_005e, `BundleOfJoy/<OnPlay>d__5::MoveNext` RVA `0x390188`
/// IL_001d-IL_003e and `ManifestAuthority/<OnPlay>d__7::MoveNext` RVA
/// `0x3ab860` IL_009c-IL_00c2 each read exactly
/// `CardPool<ColorlessCardPool>` (MethodSpec `0x2b0007c2`),
/// `Player::get_UnlockState` and `IRunState::get_CardMultiplayerConstraint`
/// into `CardPoolModel::GetUnlockedCards` (RVA `0x7e54c`), then
/// `GetDistinctForCombat(.., CombatCardGeneration)` (IL_0068 / IL_007e /
/// IL_0068 / IL_00dd). Nothing about the owner's class or an "Entropy" pool,
/// and the pool is a function of the Colorless gating epochs alone
/// (`ColorlessCardPool::FilterThroughEpochs` RVA `0xf13f4`). So the gate is
/// exactly [`owner_pool_generation_provenance_is_exact`]:
///
/// * an owner (`reward_card_pool`) — the closure walk is keyed by one;
/// * `entropy_card_pool` absent or agreeing (#3122): an explicit
///   CONTRADICTORY value is the document disagreeing with itself;
/// * a RECORDED profile ([`unlock_profile_is_recorded`]), no longer a fully
///   unlocked one: every body draws `Catalog::colorless_generation_pool`,
///   which filters by it (#2560);
/// * solo: `GetUnlockedCards`' multiplayer branch keeps the
///   `MultiplayerOnly` rows the derivation drops;
/// * a live Generation stream.
pub(crate) fn colorless_generation_provenance_is_exact(
    state: &HotState,
    catalog: &Catalog,
) -> bool {
    owner_pool_generation_provenance_is_exact(state, catalog)
}

/// Jack of All Trades' provenance (#2554, #2560).
///
/// `JackOfAllTrades/<OnPlay>d__6::MoveNext` RVA `0x3a7ecc` reads
/// `CardPool<ColorlessCardPool>` (IL_0026), `Owner.UnlockState` (IL_0031) and
/// `CardMultiplayerConstraint` (IL_0041) into `GetUnlockedCards` (IL_0046):
/// no owner class and no Entropy pool, which is why this — unlike
/// [`colorless_generation_provenance_is_exact`] — has never carried an
/// `entropy_card_pool` conjunct (#2554). It requires:
///
/// * an owner, which closes the catalog over generated results;
/// * a RECORDED profile ([`unlock_profile_is_recorded`]): the body shuffles
///   `Catalog::jack_of_all_trades_pool`, which filters by it (#2560; this was
///   `fully_unlocked_card_pool_epochs` while the body shuffled the frozen
///   `JACK_OF_ALL_TRADES_POOL_V1091`);
/// * solo (#2560): the derivation is the solo projection, and
///   `GetUnlockedCards` removes the `MultiplayerOnly` rows only for a solo
///   constraint (`0x7e54c` IL_001f-IL_004e). Before this, a multiplayer root
///   shuffled the solo list, which is not the pool native draws there;
/// * a live Generation stream.
pub(crate) fn jack_of_all_trades_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    state.reward_card_pool.is_some()
        && unlock_profile_is_recorded(state, catalog)
        && state.multiplayer_ally_key == 0
        && !state.rng.is_vacant(RngStream::Generation)
}

/// Select Spectrum Shift's complete distinct L0 Colorless prefix.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `SpectrumShiftPower/<BeforeHandDraw>d__4::MoveNext` (RVA `0x345978`)
/// owner-gates at IL `0x0020-0x0033`, reads the owner's unlocked Colorless
/// pool (`GetUnlockedCards` IL_005e) and live Amount at `0x0038-0x0064`, then
/// calls `GetDistinctForCombat(..., CombatCardGeneration)` at `0x0069-0x007e`.
/// Native consumes one complete shuffle of that pool (50 cards when fully
/// unlocked; `Catalog::colorless_generation_pool` under a partial profile,
/// #2560) even when Amount keeps only a shorter prefix; an Amount above the
/// pool width simply returns all members.
pub(crate) fn select_spectrum_shift_cards(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i32,
) -> Result<Vec<CardIdentity>, EngineRefusal> {
    if amount <= 0 || !colorless_generation_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Spectrum Shift generation provenance",
        ));
    }
    let pool = catalog.colorless_generation_pool();
    for id in pool.iter().copied() {
        let identity = CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        catalog
            .atom(&identity)
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    }
    let shuffled = shuffle_generation_slice(state, &pool)?;
    let count = usize::try_from(amount)
        .unwrap_or(usize::MAX)
        .min(shuffled.len());
    shuffled
        .into_iter()
        .take(count)
        .map(|id| {
            let identity = CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            };
            catalog
                .atom(&identity)
                .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
            Ok(identity)
        })
        .collect()
}

/// Select Hello World's complete distinct L0 owner-Common prefix.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `HelloWorldPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x33bfd0` reads the
/// frozen turn-start amount at IL `0x0038-0x003f`, filters the unlocked owner
/// pool to Common at `0x0057-0x00b5`, then performs one full distinct shuffle
/// at `0x00ba-0x00da`. Amounts above the 20-card pool keep the full pool.
pub(crate) fn select_hello_world_cards(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i32,
) -> Result<Vec<CardIdentity>, EngineRefusal> {
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs(
            "Hello World generation provenance",
        ));
    }
    let pool = exact_owner_listener_pool(state, catalog, PowerId::HelloWorld)?;
    let shuffled = shuffle_generation_slice(state, &pool)?;
    Ok(shuffled
        .into_iter()
        .take(usize::try_from(amount).unwrap_or(usize::MAX))
        .map(|id| CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogBuilder;
    use crate::content_tables::card_row;
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_PICK, CARD_FLAG_SOVEREIGN_BLADE_STATE,
        HopperDeckRow, HopperDeckState, HotMonster, MultiplayerAllyCard, MultiplayerAllyState,
    };
    use crate::ids::{CardId, EnchantmentId, MonsterKind};
    use crate::powers::SlotWire;

    fn uid_cards(uids: impl IntoIterator<Item = u32>) -> Vec<HotCard> {
        uids.into_iter()
            .map(|uid| HotCard {
                uid,
                atom: 0,
                flags: 0,
            })
            .collect()
    }

    fn transform_indexes(pile: &[u32], originals: &[u32]) -> Result<Vec<usize>, EngineRefusal> {
        let pile = uid_cards(pile.iter().copied());
        let frozen = uid_cards(originals.iter().copied())
            .into_iter()
            .map(|card| (0, card))
            .collect::<Vec<_>>();
        native_transform_insertion_order(&pile, &frozen)
            .map(|records| records.into_iter().map(|(index, _)| index).collect())
    }

    /// #3199: `IndexOf` precedes each original's removal in ONE loop, so the
    /// recorded index is post-removal of the earlier-listed originals.
    #[test]
    fn plural_transform_indexes_are_read_after_earlier_originals_left() {
        assert_eq!(
            transform_indexes(&[1, 2, 3, 4, 5, 6], &[1, 3, 4, 6]),
            Ok(vec![0, 1, 1, 2])
        );
        assert_eq!(transform_indexes(&[1, 2, 3], &[1, 2]), Ok(vec![0, 0]));
        assert_eq!(transform_indexes(&[1, 2, 3], &[3]), Ok(vec![2]));
        // Pile-order callers can never reorder, even past the introsort
        // threshold, while their indexes stay strictly increasing.
        let pile = (0..40).collect::<Vec<_>>();
        let every_other = (0..40).step_by(2).collect::<Vec<_>>();
        assert_eq!(
            transform_indexes(&pile, &every_other),
            Ok((0..20).collect::<Vec<_>>())
        );
    }

    /// Shapes whose `List.Sort` (introsort) permutation is not pinned refuse.
    #[test]
    fn plural_transform_refuses_unpinned_introsort_orders() {
        let refused = Err(EngineRefusal::MalformedArgs(
            "bulk transform introsort order",
        ));
        // Out of pile order: records 2 then 0 would need the sort to move them.
        assert_eq!(transform_indexes(&[1, 2, 3], &[3, 1]), refused);
        // Seventeen tied records exceed the insertion-sort cut-off.
        let pile = (0..20).collect::<Vec<_>>();
        assert_eq!(transform_indexes(&pile, &pile[..17]), refused);
        assert_eq!(transform_indexes(&pile, &pile[..16]), Ok(vec![0; 16]));
        // A missing or duplicated original is not a pile index at all.
        let missing = Err(EngineRefusal::MalformedArgs(
            "bulk transform duplicate pile index",
        ));
        assert_eq!(transform_indexes(&[1, 2], &[3]), missing);
        assert_eq!(transform_indexes(&[1, 2], &[1, 1]), missing);
    }

    #[test]
    fn persistent_listener_pools_match_native_oracle_for_all_five_owners() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../python/card_pool_census.json"
        )))
        .unwrap();
        for (owner, name, power_count, common_count, void_count) in [
            (RewardPool::Ironclad, "IRONCLAD", 18, 20, 78),
            (RewardPool::Silent, "SILENT", 16, 20, 78),
            (RewardPool::Regent, "REGENT", 16, 20, 79),
            (RewardPool::Necrobinder, "NECROBINDER", 17, 20, 78),
            (RewardPool::Defect, "DEFECT", 19, 20, 80),
        ] {
            for (power, count) in [
                (PowerId::CreativeAi, power_count),
                (PowerId::HelloWorld, common_count),
                (PowerId::CallOfTheVoid, void_count),
            ] {
                let expected = oracle["pools"][name]["cards"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| {
                        row["can_be_generated_in_combat"] == true
                            && row["multiplayer_constraint"] != "MultiplayerOnly"
                            && !matches!(
                                row["rarity"].as_str(),
                                Some("Basic" | "Ancient" | "Event")
                            )
                    })
                    .filter(|row| match power {
                        PowerId::CreativeAi => row["type"] == "Power",
                        PowerId::HelloWorld => row["rarity"] == "Common",
                        _ => true,
                    })
                    .map(|row| {
                        CardId::from_str(row["id"].as_str().unwrap().strip_prefix("CARD.").unwrap())
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(expected.len(), count);
                let mut builder = CatalogBuilder::new();
                for id in &expected {
                    builder
                        .intern(CardIdentity {
                            id: *id,
                            upgrade: 0,
                            enchantment: None,
                        })
                        .unwrap();
                }
                assert_eq!(
                    builder.owner_listener_pool(owner, power),
                    expected,
                    "{name}/{power:?} builder"
                );
                let catalog = builder.build();
                assert_eq!(
                    catalog.owner_listener_pool(owner, power),
                    expected,
                    "{name}/{power:?} catalog"
                );
                let mut state = HotState::at_defaults();
                state.reward_card_pool = Some(owner);
                state.entropy_card_pool = Some(owner);
                state.fully_unlocked_card_pool_epochs = true;
                for stream in RngStream::ALL {
                    state.rng.set(
                        stream,
                        RngStreamState {
                            words: [1, 2, 3, 4],
                            counter: 0,
                        },
                    );
                }
                let before = state.clone();
                assert_eq!(
                    exact_owner_listener_pool(&state, &catalog, power).unwrap(),
                    expected
                );
                let mut oracle_rng = Xoshiro256StarStar {
                    words: [1, 2, 3, 4],
                    counter: 0,
                };
                let mut expected_prefix = expected.clone();
                oracle_rng.shuffle(&mut expected_prefix).unwrap();
                match power {
                    PowerId::CreativeAi => assert_eq!(
                        select_creative_ai_power(&mut state, &catalog).unwrap().id,
                        expected_prefix[0]
                    ),
                    PowerId::HelloWorld => assert_eq!(
                        select_hello_world_cards(&mut state, &catalog, 3)
                            .unwrap()
                            .iter()
                            .map(|c| c.id)
                            .collect::<Vec<_>>(),
                        expected_prefix[..3]
                    ),
                    _ => assert_eq!(
                        shuffle_generation_slice(&mut state, &expected).unwrap(),
                        expected_prefix
                    ),
                }
                assert_eq!(
                    state.rng.get(RngStream::Generation).counter,
                    count as u64 - 1
                );
                assert_eq!(state.rng.get(RngStream::Generation).words, oracle_rng.words);
                for stream in RngStream::ALL {
                    if stream != RngStream::Generation {
                        assert_eq!(state.rng.get(stream), before.rng.get(stream));
                    }
                }
                for bad in 0..4 {
                    let mut malformed = before.clone();
                    match bad {
                        0 => malformed.reward_card_pool = None,
                        1 => malformed.fully_unlocked_card_pool_epochs = false,
                        2 => malformed.multiplayer_ally_key = 1,
                        _ => {
                            malformed
                                .rng
                                .set(RngStream::Generation, RngStreamState::default());
                        }
                    }
                    let untouched = malformed.clone();
                    assert!(exact_owner_listener_pool(&malformed, &catalog, power).is_err());
                    assert_eq!(malformed, untouched);
                }
            }
        }
    }

    fn fixture() -> (HotState, Catalog, CardIdentity) {
        let identity = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        (HotState::at_defaults(), builder.build(), identity)
    }

    fn aeonglass_wither_fixture() -> (HotState, Catalog, HotCard, CardIdentity) {
        let wither = CardIdentity {
            id: CardId::Wither,
            upgrade: 0,
            enchantment: None,
        };
        let replacement = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern_aeonglass_smoke_foundation().unwrap();
        builder.intern(replacement).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 0,
            atom: catalog.atom(&wither).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.exact_piles = true;
        state.next_card_uid = 1;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        assert_eq!(state.card_states.add_damage_growth(source.uid, 6), Some(6));
        let mut boss =
            HotMonster::new(MonsterKind::Aeonglass, super::super::monsters::AEONGLASS_HP);
        boss.max_hp = super::super::monsters::AEONGLASS_HP;
        boss.powers.set(PowerId::Artifact, SlotWire::Int, 3);
        state.monsters_mut().push(boss);
        assert!(state.fanouts.set_withering_cards_left(6));
        assert!(aeonglass_state_is_exact(&state, &catalog));
        (state, catalog, source, replacement)
    }

    /// #2957, every arm of `aeonglass_pile_mode_is_exact` plus the Wither-card
    /// arm of `aeonglass_cards_are_exact`, and the Withering Presence writer
    /// that promotes the flag.
    #[test]
    fn aeonglass_admits_inexact_piles_only_before_the_first_wither() {
        let (with_wither, catalog, _source, _replacement) = aeonglass_wither_fixture();
        // A live Wither without exact piles is a state no oracle writer
        // produces: both commit with `force_exact=True`.
        let mut inexact_wither = with_wither.clone();
        inexact_wither.exact_piles = false;
        assert!(!aeonglass_state_is_exact(&inexact_wither, &catalog));

        let mut fresh = with_wither.clone();
        fresh.piles.get_mut(PileId::Hand).make_mut().clear();
        for exact_piles in [true, false] {
            let mut state = fresh.clone();
            state.exact_piles = exact_piles;
            assert!(aeonglass_state_is_exact(&state, &catalog), "{exact_piles}");
        }
        // Once Increasing Intensity has run, the oracle's map has promoted it.
        for (upgrades, additional) in [(1, 1), (1, 0), (0, 1)] {
            let mut state = fresh.clone();
            state.exact_piles = false;
            assert!(state.monsters_mut()[0].set_aeonglass_wither_upgrade_count(upgrades));
            assert!(state.monsters_mut()[0].set_aeonglass_additional_strength(additional));
            assert!(!aeonglass_owner_state_is_exact(&state, true, true, true));
            state.exact_piles = true;
            assert_eq!(
                aeonglass_owner_state_is_exact(&state, true, true, true),
                upgrades == additional || upgrades == additional + 1,
                "{upgrades}/{additional}"
            );
        }

        // The countdown's Wither publishes exact mode as it enters the Hand
        // (`_add_aeonglass_wither`, frozen Python, deleted #2827).
        let mut state = fresh.clone();
        state.exact_piles = false;
        assert!(state.fanouts.set_withering_cards_left(1));
        withering_after_card_played(&mut state, &catalog, true, &mut Vec::new()).unwrap();
        assert!(state.exact_piles);
        assert_eq!(state.fanouts.withering_cards_left(), 6);
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 1);
        assert_eq!(
            catalog.spec(hand[0].atom).unwrap().identity.id,
            CardId::Wither
        );
        assert!(aeonglass_state_is_exact(&state, &catalog));
    }

    #[test]
    fn wither_clone_preserves_growth_transform_erases_and_ordinary_upgrade_is_unavailable() {
        let (mut state, catalog, source, replacement) = aeonglass_wither_fixture();
        let wither_identity = catalog.spec(source.atom).unwrap().identity;
        assert_eq!(native_max_upgrade(CardId::Wither), Some(0));
        assert!(!native_card_is_upgradable(wither_identity));

        inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        let copied = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!(state.card_states.get(source.uid).damage_growth, 6);
        assert_eq!(state.card_states.get(copied.uid).damage_growth, 6);
        assert!(aeonglass_state_is_exact(&state, &catalog));

        let atom_before_upgrade = state.piles.get(PileId::Hand).as_slice()[0].atom;
        upgrade_live_cards_once(&mut state, &catalog, &[source.uid]).unwrap();
        assert_eq!(
            state.piles.get(PileId::Hand).as_slice()[0].atom,
            atom_before_upgrade
        );
        assert_eq!(state.card_states.get(source.uid).damage_growth, 6);

        let source_instance = state.card_states.get(source.uid);
        let replacements = bulk_transform_fixed_same_pile(
            &mut state,
            &catalog,
            PileId::Hand,
            &[(source, source_instance)],
            replacement,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(replacements.len(), 1);
        assert!(state.card_states.get(source.uid).is_vacant());
        assert!(state.card_states.get(replacements[0].uid).is_vacant());
        assert_eq!(
            catalog.spec(replacements[0].atom).unwrap().identity,
            replacement
        );
        assert_eq!(state.card_states.get(copied.uid).damage_growth, 6);
        assert!(aeonglass_state_is_exact(&state, &catalog));
    }

    fn dampen_fixture_for(id: CardId) -> (HotState, Catalog, CardAtom, CardAtom) {
        dampen_fixture_with_relics(id, &[])
    }

    fn dampen_fixture_with_relics(
        id: CardId,
        relics: &[crate::ids::RelicId],
    ) -> (HotState, Catalog, CardAtom, CardAtom) {
        dampen_fixture_for_identity(
            CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            },
            relics,
        )
    }

    fn dampen_fixture_for_identity(
        l0: CardIdentity,
        relics: &[crate::ids::RelicId],
    ) -> (HotState, Catalog, CardAtom, CardAtom) {
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        let (l0_atom, l1_atom) = builder.intern_dampen_card_pair(l0).unwrap();
        builder.set_relics(relics).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.exact_piles = true;
        state.next_card_uid = 5;
        for (uid, pile) in DAMPEN_PILES.into_iter().enumerate() {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: uid as u32,
                atom: l1_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED,
            });
        }
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(0)));
        assert!(flail.random_ai.set_log(&[2, 0]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(1)));
        assert!(spectral.random_ai.set_log(&[0, 1]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 89);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        magi.loop_pos = 1;
        state.monsters_mut().extend([flail, spectral, magi]);
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        (state, catalog, l0_atom, l1_atom)
    }

    fn dampen_fixture() -> (HotState, Catalog, CardAtom, CardAtom) {
        dampen_fixture_for(CardId::StrikeIronclad)
    }

    /// #3413: native `DowngradeInternal` re-runs TezcatarasEmber's
    /// `OnEnchant` through `UpgradeBy`'s local-modifier walk, which the atom
    /// swap does not replay, so Dampen over an upgraded Tezcatara card
    /// refuses by name and leaves the state untouched.
    #[test]
    fn dampen_over_an_upgraded_tezcataras_ember_card_refuses_by_name() {
        let (mut state, catalog, _, _) = dampen_fixture_for_identity(
            CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment {
                    id: crate::ids::EnchantmentId::TezcatarasEmber,
                    amount: 1,
                }),
            },
            &[],
        );
        let before = state.clone();
        assert_eq!(
            apply_dampen_power(&mut state, &catalog, 2),
            Err(EngineRefusal::MalformedArgs("Dampen over TEZCATARAS_EMBER"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn dampen_snapshots_native_pile_order_is_idempotent_and_restores_l0_or_l1() {
        let (mut state, catalog, l0, l1) = dampen_fixture();
        state.card_states.set(
            3,
            CardInstanceState {
                damage_growth: 7,
                local_retain: true,
                ..CardInstanceState::default()
            },
        );
        let preserved = state.card_states.get(3).clone();

        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let active = state.card_states.dampen().unwrap();
        assert_eq!(
            active.cards.iter().map(|row| row.uid).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert!(DAMPEN_PILES.into_iter().all(|pile| {
            state.piles.get(pile).as_slice()[0].atom == l0
                && state.piles.get(pile).as_slice()[0].flags
                    == CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED
        }));
        assert_eq!(state.card_states.get(3), preserved.clone());
        let applied = state.clone();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert_eq!(state, applied);

        // A later exact upgrade is legal; restoration is capped and therefore
        // leaves this row at L1 while upgrading every still-L0 counterpart.
        state.piles.get_mut(PileId::Draw).make_mut()[0].atom = l1;
        let moved = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        state.piles.get_mut(PileId::Discard).make_mut().push(moved);
        let sibling_before = state.clone();
        remove_dampen_after_death(&mut state, 0).unwrap();
        assert_eq!(state, sibling_before);
        state.monsters_mut()[2].hp = 0;
        remove_dampen_after_death(&mut state, 2).unwrap();
        assert!(state.card_states.dampen().is_none());
        let restored = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .collect::<Vec<_>>();
        assert_eq!(restored.len(), 5);
        assert!(restored.iter().all(|card| card.atom == l1));
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].uid, 4);
        assert_eq!(state.card_states.get(3), preserved);
    }

    #[test]
    fn dampen_restore_preflights_missing_duplicate_and_wrong_atom_before_writes() {
        let (mut state, catalog, _l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.monsters_mut()[2].hp = 0;
        for malformed in 0..3 {
            let mut candidate = state.clone();
            match malformed {
                0 => {
                    candidate.piles.get_mut(PileId::Play).make_mut().clear();
                }
                1 => {
                    let duplicate = candidate.piles.get(PileId::Hand).as_slice()[0];
                    candidate
                        .piles
                        .get_mut(PileId::Exhaust)
                        .make_mut()
                        .push(duplicate);
                }
                _ => {
                    candidate.piles.get_mut(PileId::Hand).make_mut()[0].atom = CardAtom::MAX;
                }
            }
            let before = candidate.clone();
            assert!(
                remove_dampen_after_death(&mut candidate, 2).is_err(),
                "malformed case {malformed} was accepted"
            );
            assert_eq!(candidate, before);
        }
    }

    #[test]
    fn dampen_catalog_damage_restores_only_magi_and_orders_spectral_hex_cleanup() {
        for target in 0..3 {
            let (mut state, catalog, _l0, l1) = dampen_fixture();
            apply_dampen_power(&mut state, &catalog, 2).unwrap();
            let mut events = Vec::new();
            crate::engine::damage::damage_monster_with_catalog(
                &mut state,
                &catalog,
                target,
                crate::decimal::DotNetDecimal::from_i64(999),
                false,
                true,
                &mut events,
            )
            .unwrap();
            match target {
                0 => {
                    assert!(state.card_states.dampen().is_some());
                    assert_eq!(state.powers.value(PowerId::HexPower), 2);
                }
                1 => {
                    assert!(state.card_states.dampen().is_some());
                    assert_eq!(state.powers.value(PowerId::HexPower), 0);
                    assert!(DAMPEN_PILES.into_iter().all(|pile| {
                        state.piles.get(pile).as_slice()[0].flags & CARD_FLAG_HEXED == 0
                    }));
                }
                _ => {
                    assert!(state.card_states.dampen().is_none());
                    assert!(
                        DAMPEN_PILES
                            .into_iter()
                            .all(|pile| state.piles.get(pile).as_slice()[0].atom == l1)
                    );
                }
            }
        }
    }

    /// #3233: Magi Knight dying LAST ends the combat, so `CardCmd.Upgrade`
    /// (RVA `0x12f660` IL_000c-IL_0018) skips every restore while the power
    /// is still removed; the same kill with a Knight alive restores (above).
    #[test]
    fn dampen_combat_ending_magi_death_removes_power_without_restoring() {
        let (mut state, catalog, l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let mut events = Vec::new();
        for target in [0, 1, 2] {
            crate::engine::damage::damage_monster_with_catalog(
                &mut state,
                &catalog,
                target,
                crate::decimal::DotNetDecimal::from_i64(999),
                false,
                true,
                &mut events,
            )
            .unwrap();
        }
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert!(state.card_states.dampen().is_none());
        assert!(
            DAMPEN_PILES
                .into_iter()
                .all(|pile| state.piles.get(pile).as_slice()[0].atom == l0)
        );
    }

    /// The ending no-op still preflights every tracked row: a malformed
    /// snapshot refuses by name and writes nothing.
    #[test]
    fn dampen_combat_ending_restore_still_preflights_rows() {
        let (mut state, catalog, l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        remove_hex_power(&mut state);
        for monster in state.monsters_mut().iter_mut() {
            monster.hp = 0;
        }
        let mut malformed = state.clone();
        malformed.piles.get_mut(PileId::Play).make_mut().clear();
        let before = malformed.clone();
        assert!(remove_dampen_after_death(&mut malformed, 2).is_err());
        assert_eq!(malformed, before);

        remove_dampen_after_death(&mut state, 2).unwrap();
        assert!(state.card_states.dampen().is_none());
        assert!(
            DAMPEN_PILES
                .into_iter()
                .all(|pile| state.piles.get(pile).as_slice()[0].atom == l0)
        );
    }

    #[test]
    fn dampen_catalog_damage_and_catalogless_roots_refuse_atomically() {
        let (mut malformed, catalog, _l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut malformed, &catalog, 2).unwrap();
        malformed.piles.get_mut(PileId::Play).make_mut().clear();
        let before = malformed.clone();
        let mut events = vec![Event::MonsterDied { uid: 77 }];
        let before_events = events.clone();
        assert!(
            crate::engine::damage::damage_monster_with_catalog(
                &mut malformed,
                &catalog,
                2,
                crate::decimal::DotNetDecimal::from_i64(999),
                false,
                true,
                &mut events,
            )
            .is_err()
        );
        assert_eq!(malformed, before);
        assert_eq!(events, before_events);

        let (mut active, catalog, _l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut active, &catalog, 2).unwrap();
        for operation in 0..4 {
            let mut candidate = active.clone();
            let mut emitted = Vec::new();
            if operation == 1 {
                candidate.monsters_mut()[2].hp = 0;
            }
            let before_candidate = candidate.clone();
            let result = match operation {
                0 => crate::engine::damage::damage_monster(
                    &mut candidate,
                    2,
                    crate::decimal::DotNetDecimal::from_i64(1),
                    false,
                    true,
                    &mut emitted,
                )
                .map(|_| ()),
                1 => crate::engine::damage::finish_monster_death(&mut candidate, 2, &mut emitted),
                2 => crate::engine::damage::tick_monster_poison(&mut candidate, &mut emitted)
                    .map(|_| ()),
                _ => crate::engine::damage::monster_attack_player(
                    &mut candidate,
                    2,
                    1,
                    1,
                    &mut emitted,
                ),
            };
            assert!(
                result.is_err(),
                "catalogless operation {operation} succeeded"
            );
            assert_eq!(candidate, before_candidate);
            assert!(emitted.is_empty());
        }
    }

    #[test]
    fn dampen_zero_upgrade_application_keeps_an_empty_live_caster() {
        let (mut state, catalog, l0, _l1) = dampen_fixture();
        for pile in DAMPEN_PILES {
            state.piles.get_mut(pile).make_mut()[0].atom = l0;
        }
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let active = state.card_states.dampen().unwrap();
        assert_eq!(active.caster_uid, 2);
        assert!(active.cards.is_empty());
        let before = state.clone();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert_eq!(state, before);
    }

    #[test]
    fn dampen_ignores_a_later_clone_and_nonlethal_damage_keeps_the_snapshot() {
        let (mut state, catalog, l0, l1) = dampen_fixture();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let source = state.piles.get(PileId::Hand).as_slice()[0];
        inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            1,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        let clone = *state.piles.get(PileId::Draw).as_slice().last().unwrap();
        assert_eq!(clone.uid, 5);
        assert_eq!(clone.atom, l0);
        assert!(
            !state
                .card_states
                .dampen()
                .unwrap()
                .cards
                .iter()
                .any(|row| row.uid == clone.uid)
        );
        assert!(dampen_state_is_exact(&state, &catalog));

        let before_cards = state.piles.clone();
        let before_dampen = state.card_states.dampen().unwrap().clone();
        let before_hp = state.monsters[2].hp;
        crate::engine::damage::damage_monster_with_catalog(
            &mut state,
            &catalog,
            2,
            crate::decimal::DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[2].hp, before_hp - 1);
        assert_eq!(state.piles, before_cards);
        assert_eq!(state.card_states.dampen(), Some(&before_dampen));

        let original = state.clone();
        let mut branch = state.clone();
        crate::engine::damage::damage_monster_with_catalog(
            &mut branch,
            &catalog,
            2,
            crate::decimal::DotNetDecimal::from_i64(999),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state, original, "cold Dampen COW is branch-local");
        assert_ne!(branch, state);
        assert!(branch.card_states.dampen().is_none());
        assert_eq!(
            branch
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .find(|card| card.uid == clone.uid)
                .unwrap()
                .atom,
            l0,
            "the post-apply clone is not part of the restore snapshot"
        );
        assert!(
            branch
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .filter(|card| card.uid != clone.uid)
                .all(|card| card.atom == l1)
        );
    }

    #[test]
    fn dampen_resolved_power_forgets_only_its_row() {
        let (mut remove, remove_catalog, _l0, _l1) = dampen_fixture_for(CardId::Inflame);
        apply_dampen_power(&mut remove, &remove_catalog, 2).unwrap();
        remove.energy = 9;
        let before_rows = remove.card_states.dampen().unwrap().cards.len();
        crate::engine::play::play_card(
            &mut remove,
            &remove_catalog,
            0,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!dampen_tracks_uid(&remove, 0));
        assert_eq!(
            remove.card_states.dampen().unwrap().cards.len(),
            before_rows - 1
        );
        assert!(dampen_state_is_exact(&remove, &remove_catalog));
    }

    /// Dampen fixture whose Hand holds Primal Force (uid 0) and a
    /// Dampen-tracked Strike (uid 1, L1 before Dampen), with Primal Force
    /// L1->L0 rows in the other three piles.
    fn primal_force_dampen_fixture() -> (HotState, Catalog, CardAtom, CardAtom) {
        let (mut state, _catalog, _l0, _l1) = dampen_fixture_for(CardId::PrimalForce);
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        builder
            .intern_dampen_card_pair(CardIdentity {
                id: CardId::PrimalForce,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let (strike_l0, strike_l1) = builder
            .intern_dampen_card_pair(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        state.piles.get_mut(PileId::Draw).make_mut()[0].atom = strike_l1;
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 9;
        let victim = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
        state.piles.get_mut(PileId::Hand).make_mut().push(victim);
        assert!(dampen_tracks_uid(&state, 1));
        assert!(dampen_state_is_exact(&state, &catalog));
        (state, catalog, strike_l0, strike_l1)
    }

    /// #2982: Primal Force's `CardCmd.Transform` removes the tracked original
    /// from state and puts a fresh Giant Rock at its Hand index. The Dampen
    /// row for the original is forgotten; every other row survives.
    #[test]
    fn primal_force_transforming_a_dampen_tracked_attack_forgets_only_its_row() {
        let (mut state, catalog, _strike_l0, _strike_l1) = primal_force_dampen_fixture();
        let rows_before = state.card_states.dampen().unwrap().cards.clone();
        let next_uid = state.next_card_uid;

        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();

        let rock_l0 = catalog
            .atom(&CardIdentity {
                id: CardId::GiantRock,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 1);
        assert_eq!((hand[0].uid, hand[0].atom), (next_uid, rock_l0));
        assert!(!dampen_tracks_uid(&state, 1), "the transformed original");
        assert!(!dampen_tracks_uid(&state, next_uid), "the Giant Rock");
        let rows_after = &state.card_states.dampen().unwrap().cards;
        assert_eq!(
            rows_after,
            &rows_before
                .into_iter()
                .filter(|row| row.uid != 1)
                .collect::<Vec<_>>()
        );
        assert!(
            PileId::ALL.into_iter().all(|pile| state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|c| c.uid != 1))
        );
        assert!(dampen_state_is_exact(&state, &catalog));
    }

    /// #2982: after the transform, Magi's death restores the surviving rows
    /// and leaves the Giant Rock at Primal Force's (Dampened L0) level,
    /// instead of refusing on the vanished original.
    #[test]
    fn magi_death_after_primal_force_transform_restores_the_rest_and_keeps_the_rock() {
        let (mut state, catalog, _strike_l0, _strike_l1) = primal_force_dampen_fixture();
        let primal_l1 = catalog
            .atom(&CardIdentity {
                id: CardId::PrimalForce,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let rock_l0 = catalog
            .atom(&CardIdentity {
                id: CardId::GiantRock,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let rock_uid = state.next_card_uid;
        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();
        state.monsters_mut()[2].hp = 0;

        remove_dampen_after_death(&mut state, 2).unwrap();

        assert!(state.card_states.dampen().is_none());
        for pile in DAMPEN_PILES {
            for card in state.piles.get(pile).as_slice() {
                if card.uid == rock_uid {
                    assert_eq!(card.atom, rock_l0, "the Giant Rock was never Dampened");
                } else {
                    assert_eq!(card.atom, primal_l1, "uid {} restored", card.uid);
                }
            }
        }
    }

    #[test]
    fn dampen_restore_refreshes_the_same_live_card_before_its_second_replay_body() {
        let (mut state, catalog, _l0, _l1) = dampen_fixture_for(CardId::Thunderclap);
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        state.monsters_mut()[2].hp = 4;
        let mut source_state = state.card_states.get(0);
        assert!(source_state.set_base_replay_count(Some(1)).is_some());
        state.card_states.set(0, source_state);

        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();

        assert!(state.card_states.dampen().is_none());
        assert!(state.monsters[2].hp <= 0);
        // Body one is L0 damage 4. It kills Magi and restores the source to
        // L1; body two therefore deals current L1 damage 7, amplified by the
        // first body's Vulnerable to one final native floor of 10.
        assert_eq!(state.monsters[0].hp, 94);
    }

    /// #3254: the all-opponents batched shape of #3245's per-receiver
    /// Miniature Cannon read (`MiniatureCannon::ModifyDamageAdditive` RVA
    /// `0x96ec8` IL_0024-IL_002b; one `CreatureCmd.Damage` per hit at
    /// `0x3f19c0` IL_0724-IL_0743). This is the Knights elite
    /// `ff855417d870c522` shape: a dampened Conflagration whose first hit
    /// kills Magi Knight is restored to L1 before hit two, so hits two to
    /// four carry the +3. The hit count (4) was read into the command before
    /// it ran (`Conflagration.<OnPlay>d__5` RVA `0x393da4` IL_0087-IL_0097),
    /// so the restore adds no fifth hit.
    #[test]
    fn miniature_cannon_reads_a_mid_attack_dampen_restore_on_the_later_hits() {
        let (mut state, catalog, l0, l1) = dampen_fixture_with_relics(
            CardId::Conflagration,
            &[crate::ids::RelicId::RelicMiniatureCannon],
        );
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].atom, l0);
        state.energy = 3;
        state.monsters_mut()[2].hp = 2;

        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();

        assert!(state.monsters[2].hp <= 0);
        assert!(!state.history.over);
        assert!(state.card_states.dampen().is_none());
        let restored = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .find(|card| card.uid == 0)
            .unwrap();
        assert_eq!(restored.atom, l1);
        // Hit one: 2 at L0. Hits two to four: 2 + 3 at the restored L1.
        assert_eq!(state.monsters[0].hp, 108 - 2 - 3 * 5);
        assert_eq!(state.monsters[1].hp, 97 - 2 - 3 * 5);
    }

    /// #3254 control for the all-opponents shape: while Dampen stands, the
    /// live source is still the L0 atom, so no hit of the dampened
    /// Conflagration takes Miniature Cannon's +3.
    #[test]
    fn miniature_cannon_skips_a_dampened_source_while_the_caster_lives() {
        let (mut state, catalog, l0, _l1) = dampen_fixture_with_relics(
            CardId::Conflagration,
            &[crate::ids::RelicId::RelicMiniatureCannon],
        );
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        state.monsters_mut()[2].hp = 50;

        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();

        assert!(state.card_states.dampen().is_some());
        assert_eq!(state.monsters[0].hp, 108 - 4 * 2);
        assert_eq!(state.monsters[2].hp, 50 - 4 * 2);
        let source = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .find(|card| card.uid == 0)
            .unwrap();
        assert_eq!(source.atom, l0);
    }

    /// #2974: Anger's hit kills Magi, the last Dampen caster. AfterRemoved
    /// restores the played Anger to L1 inside the awaited attack, and native
    /// `CreateClone` copies that current L1 model into Discard.
    #[test]
    fn anger_that_kills_the_dampen_caster_clones_the_restored_l1_card() {
        let (mut state, catalog, l0, l1) = dampen_fixture_for(CardId::Anger);
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        state.monsters_mut()[2].hp = 4;
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].atom, l0);
        let next_uid = state.next_card_uid;

        crate::engine::play::play_card(&mut state, &catalog, 0, Some(2), None, &mut Vec::new())
            .unwrap();

        assert!(state.monsters[2].hp <= 0);
        assert!(!state.history.over);
        assert!(state.card_states.dampen().is_none());
        let live = DAMPEN_PILES
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .find(|card| card.uid == 0)
            .unwrap();
        assert_eq!(live.atom, l1, "the played source was restored");
        let clone = state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == next_uid)
            .copied()
            .unwrap();
        assert_eq!(clone.atom, l1, "the clone copies the restored model");
        assert_eq!(state.history.owner_generated_cards_combat, 1);
    }

    /// The combat-ending variant of the same kill: the last Knight dies,
    /// so generation is recorded but no clone is allocated or inserted, and
    /// Dampen's restore is a no-op (`CardCmd.Upgrade` RVA `0x12f660`
    /// IL_000c-IL_0018 returns at `IsEnding`, #3233): the source stays L0.
    #[test]
    fn lethal_anger_on_the_last_dampen_caster_records_generation_only() {
        let (mut state, catalog, l0, _l1) = dampen_fixture_for(CardId::Anger);
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        state.monsters_mut()[0].hp = 0;
        state.monsters_mut()[1].hp = 0;
        state.powers.set(PowerId::HexPower, SlotWire::Int, 0);
        for pile in DAMPEN_PILES {
            for card in state.piles.get_mut(pile).make_mut() {
                card.flags &= !CARD_FLAG_HEXED;
            }
        }
        state.monsters_mut()[2].hp = 4;
        let next_uid = state.next_card_uid;

        crate::engine::play::play_card(&mut state, &catalog, 0, Some(2), None, &mut Vec::new())
            .unwrap();

        assert!(state.history.over);
        assert!(state.card_states.dampen().is_none());
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.next_card_uid, next_uid);
        assert!(PileId::ALL.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.uid != next_uid)
        }));
        let live = PileId::ALL
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .find(|card| card.uid == 0)
            .unwrap();
        assert_eq!(live.atom, l0);
    }

    /// A Dampened Anger that leaves Magi alive keeps its L0 identity and
    /// clones L0, with Dampen still live.
    #[test]
    fn anger_that_leaves_the_dampen_caster_alive_clones_the_dampened_l0_card() {
        let (mut state, catalog, l0, _l1) = dampen_fixture_for(CardId::Anger);
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        let next_uid = state.next_card_uid;

        crate::engine::play::play_card(&mut state, &catalog, 0, Some(2), None, &mut Vec::new())
            .unwrap();

        assert!(state.monsters[2].hp > 0);
        assert!(state.card_states.dampen().is_some());
        let clone = state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == next_uid)
            .copied()
            .unwrap();
        assert_eq!(clone.atom, l0);
    }

    #[test]
    fn dampen_restore_refreshes_true_grit_replay_into_the_l1_selector() {
        let (mut state, catalog, _l0, _l1) = dampen_fixture_for(CardId::TrueGrit);
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        state.energy = 3;
        state.monsters_mut()[1].hp = 0;
        state.powers.set(PowerId::HexPower, SlotWire::Int, 0);
        for pile in DAMPEN_PILES {
            for card in state.piles.get_mut(pile).make_mut() {
                card.flags &= !CARD_FLAG_HEXED;
            }
        }
        state.monsters_mut()[2].hp = 1;
        for pile in [PileId::Draw, PileId::Discard, PileId::Exhaust] {
            let candidate = state.piles.get_mut(pile).make_mut().remove(0);
            state.piles.get_mut(PileId::Hand).make_mut().push(candidate);
        }
        let mut source_state = state.card_states.get(0);
        assert!(source_state.set_base_replay_count(Some(1)).is_some());
        state.card_states.set(0, source_state);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        for stream in [crate::hot::RngStream::Targets, crate::hot::RngStream::Sel] {
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(0);
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
        }
        assert!(dampen_state_is_exact(&state, &catalog));

        crate::engine::play::play_card(&mut state, &catalog, 0, None, None, &mut Vec::new())
            .unwrap();

        assert!(state.monsters[2].hp <= 0);
        assert!(state.card_states.dampen().is_none());
        let pending = state.pending.as_deref().expect("restored L1 parks Select");
        let record = pending
            .record(&state.frames)
            .expect("exact CardPlay continuation");
        assert_eq!(
            record.route().unwrap(),
            (PileId::Play, crate::hot::PendingSelectionKind::Program)
        );
        assert_eq!(record.play_index, 1);
        assert_eq!(record.next_step, 2);
        assert!(record.selection_cards().is_empty());
        let hand_uids = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect::<Vec<_>>();
        assert_eq!(hand_uids, [1, 3]);
        let actions = crate::engine::legal_actions(&state, &catalog);
        assert_eq!(
            actions,
            [
                crate::engine::Action::Select {
                    answer: crate::engine::SelectionAnswer::OptionIndex(0),
                },
                crate::engine::Action::Select {
                    answer: crate::engine::SelectionAnswer::OptionIndex(1),
                },
            ]
        );
        let completed = crate::engine::apply_action(&state, &catalog, &actions[0])
            .unwrap()
            .state;
        assert!(completed.pending.is_none());
        assert!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 3)
        );
        assert_eq!(
            completed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            state
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .any(|card| card.uid == 0)
        );
    }

    #[test]
    fn dampen_preserves_enchantment_affliction_and_valid_mutable_card_state() {
        let (mut state, _catalog, _old_l0, _old_l1) = dampen_fixture();
        let identity = CardIdentity {
            id: CardId::GeneticAlgorithm,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Sharp,
                amount: 2,
            }),
        };
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        let (l0, l1) = builder.intern_dampen_card_pair(identity).unwrap();
        let catalog = builder.build();
        for pile in DAMPEN_PILES {
            let card = &mut state.piles.get_mut(pile).make_mut()[0];
            card.atom = l1;
            card.flags |= CARD_FLAG_GENETIC_ALGORITHM_STATE;
            state.card_states.set(
                card.uid,
                CardInstanceState {
                    genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(
                        9,
                        Some(card.uid),
                    )
                    .unwrap(),
                    ..CardInstanceState::default()
                },
            );
        }
        let before_states = (0..5)
            .map(|uid| state.card_states.get(uid))
            .collect::<Vec<_>>();
        let before_flags = DAMPEN_PILES
            .into_iter()
            .map(|pile| state.piles.get(pile).as_slice()[0].flags)
            .collect::<Vec<_>>();

        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert!(
            DAMPEN_PILES
                .into_iter()
                .all(|pile| state.piles.get(pile).as_slice()[0].atom == l0)
        );
        assert_eq!(
            (0..5)
                .map(|uid| state.card_states.get(uid))
                .collect::<Vec<_>>(),
            before_states
        );
        assert_eq!(
            DAMPEN_PILES
                .into_iter()
                .map(|pile| state.piles.get(pile).as_slice()[0].flags)
                .collect::<Vec<_>>(),
            before_flags
        );
        state.monsters_mut()[2].hp = 0;
        remove_dampen_after_death(&mut state, 2).unwrap();
        assert!(
            DAMPEN_PILES
                .into_iter()
                .all(|pile| state.piles.get(pile).as_slice()[0].atom == l1)
        );
        assert_eq!(
            (0..5)
                .map(|uid| state.card_states.get(uid))
                .collect::<Vec<_>>(),
            before_states
        );
    }

    #[test]
    fn dampen_tracked_exhaust_accepts_only_the_exact_missing_live_object() {
        let (mut state, catalog, _l0, _l1) = dampen_fixture();
        apply_dampen_power(&mut state, &catalog, 2).unwrap();
        let tracked = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        crate::engine::draw::card_exhausted(&mut state, &catalog, tracked, &mut Vec::new())
            .unwrap();
        assert_eq!(
            state.piles.get(PileId::Exhaust).as_slice().last(),
            Some(&tracked)
        );
        assert!(dampen_state_is_exact(&state, &catalog));

        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Discard,
        }];
        let before_events = events.clone();
        assert!(
            crate::engine::draw::card_exhausted(&mut state, &catalog, tracked, &mut events,)
                .is_err()
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn fresh_physical_state_cards_attach_the_native_default_slot() {
        let ids = [
            CardId::BansheesCry,
            CardId::Flatten,
            CardId::KinglyKick,
            CardId::KinglyPunch,
            CardId::Melancholy,
            CardId::Midnight,
            CardId::Pinpoint,
            CardId::RocketPunch,
            CardId::UpMySleeve,
            CardId::Enlightenment,
            CardId::BulletTime,
            CardId::Modded,
            CardId::Stomp,
            CardId::Thrash,
            CardId::Claw,
            CardId::Maul,
            CardId::MomentumStrike,
            CardId::Rampage,
            CardId::TheBall,
            CardId::TheScythe,
            CardId::Wither,
        ];
        let mut builder = CatalogBuilder::new();
        for id in ids.into_iter().chain([CardId::Wound]) {
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
        for id in ids {
            let identity = CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            };
            let spec = catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
            let mut card = mint_card(&mut state, &catalog, identity).unwrap();
            apply_physical_card_after_entered(&mut state, &catalog, spec, &mut card).unwrap();
            assert_ne!(
                card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                0,
                "{id:?} lost its intrinsic physical-state slot"
            );
        }

        let wound = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let spec = catalog.spec(catalog.atom(&wound).unwrap()).unwrap();
        let mut card = mint_card(&mut state, &catalog, wound).unwrap();
        apply_physical_card_after_entered(&mut state, &catalog, spec, &mut card).unwrap();
        assert_eq!(card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
    }

    #[test]
    fn midnight_fresh_entry_backfills_one_history_row_and_clone_does_not_repeat_it() {
        let midnight = CardIdentity {
            id: CardId::Midnight,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(midnight).unwrap();
        let defend = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let defend_atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(atom).unwrap();

        for exhausted in [0, 1, 3] {
            let mut state = HotState::at_defaults();
            state.history.owner_cards_exhausted_combat = exhausted;
            let mut card = mint_card(&mut state, &catalog, midnight).unwrap();
            apply_physical_card_after_entered(&mut state, &catalog, spec, &mut card).unwrap();
            let rows = state
                .card_states
                .get(card.uid)
                .local_cost_modifiers
                .as_slice()
                .to_vec();
            let expected = if exhausted == 0 {
                Vec::new()
            } else {
                vec![LocalCostModifier {
                    kind: LocalCostModifierKind::Add,
                    amount: -i64::from(exhausted),
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                }]
            };
            assert_eq!(rows, expected, "prior Exhaust subtotal {exhausted}");
        }

        let mut state = HotState::at_defaults();
        state.history.owner_cards_exhausted_combat = 3;
        let source = HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.card_states.append_local_cost_modifier(
            source.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -3,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        let mut clone = HotCard { uid: 2, ..source };
        state
            .card_states
            .set(clone.uid, state.card_states.get(source.uid));
        apply_physical_card_after_entered_inner(&mut state, &catalog, spec, &mut clone, true)
            .unwrap();
        assert_eq!(
            state
                .card_states
                .get(clone.uid)
                .local_cost_modifiers
                .as_slice(),
            [LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -3,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        );
        // Keep both exact objects live for the next exhaust callback: the
        // clone inherited the subtotal and each then appends one new -1 row.
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().push(clone);
        crate::engine::draw::card_exhausted(
            &mut state,
            &catalog,
            HotCard {
                uid: 9,
                atom: defend_atom,
                flags: 0,
            },
            &mut Vec::new(),
        )
        .unwrap();
        for uid in [1, 2] {
            assert_eq!(
                state
                    .card_states
                    .get(uid)
                    .local_cost_modifiers
                    .as_slice()
                    .iter()
                    .map(|row| row.amount)
                    .collect::<Vec<_>>(),
                [-3, -1]
            );
        }
    }

    #[test]
    fn rocket_punch_generated_status_listener_uses_native_uid_order_and_is_atomic() {
        let rocket = CardIdentity {
            id: CardId::RocketPunch,
            upgrade: 0,
            enchantment: None,
        };
        let wound = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let strike = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let rocket_atom = builder.intern(rocket).unwrap();
        let wound_atom = builder.intern(wound).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 20;
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal])
        );
        for (pile, uid) in [
            (PileId::Hand, 5),
            (PileId::Draw, 4),
            (PileId::Discard, 3),
            (PileId::Exhaust, 2),
            (PileId::Play, 1),
        ] {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid,
                atom: rocket_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let wound_spec = catalog.spec(wound_atom).unwrap();
        ROCKET_PUNCH_GENERATED_TRACE.with(|trace| trace.borrow_mut().clear());
        GENERATED_LISTENER_TRACE.with(|trace| trace.borrow_mut().clear());
        after_owner_card_generated(&mut state, &catalog, wound_spec, &mut Vec::new()).unwrap();
        after_owner_card_generated(&mut state, &catalog, wound_spec, &mut Vec::new()).unwrap();
        assert_eq!(
            ROCKET_PUNCH_GENERATED_TRACE.with(|trace| trace.borrow().clone()),
            [5, 4, 3, 2, 1, 5, 4, 3, 2, 1]
        );
        assert_eq!(state.powers.value(PowerId::Strength), 2);
        assert_eq!(
            GENERATED_LISTENER_TRACE.with(|trace| trace.borrow().clone()),
            [
                GeneratedListenerTrace::Power(PowerId::Arsenal),
                GeneratedListenerTrace::RocketPunch(5),
                GeneratedListenerTrace::RocketPunch(4),
                GeneratedListenerTrace::RocketPunch(3),
                GeneratedListenerTrace::RocketPunch(2),
                GeneratedListenerTrace::RocketPunch(1),
                GeneratedListenerTrace::Power(PowerId::Arsenal),
                GeneratedListenerTrace::RocketPunch(5),
                GeneratedListenerTrace::RocketPunch(4),
                GeneratedListenerTrace::RocketPunch(3),
                GeneratedListenerTrace::RocketPunch(2),
                GeneratedListenerTrace::RocketPunch(1),
            ],
            "power callbacks precede the physical Rocket layer"
        );
        for uid in 1..=5 {
            let rows = state
                .card_states
                .get_ref(uid)
                .unwrap()
                .local_cost_modifiers
                .as_slice();
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| {
                row.kind == LocalCostModifierKind::Add
                    && row.amount == -1
                    && row.expiration == LocalCostExpiration::UntilPlayed
                    && !row.reduce_only
            }));
        }

        let mut payment = state.clone();
        payment.energy = 0;
        payment
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut payment = crate::engine::apply_action_into(
            &payment,
            &catalog,
            &crate::engine::Action::Play {
                uid: 5,
                target: Some(0),
                selection: crate::engine::SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(payment.energy, 0, "two callbacks reduce Rocket from 2 to 0");
        assert!(
            payment
                .card_states
                .cleanup_card_local_cost_modifiers(5, LocalCostExpiration::UntilPlayed)
        );
        assert!(
            payment
                .card_states
                .get(5)
                .local_cost_modifiers
                .as_slice()
                .is_empty(),
            "UntilPlayed cleanup removes every repeated callback row"
        );

        let mut malformed = state.clone();
        malformed.piles.get_mut(PileId::Hand).make_mut()[0].flags = 0;
        let malformed_before = malformed.clone();
        assert!(
            after_owner_card_generated(&mut malformed, &catalog, wound_spec, &mut Vec::new())
                .is_err()
        );
        assert_eq!(malformed, malformed_before);
        malformed.powers.set(PowerId::Arsenal, SlotWire::Int, 0);

        let strike_spec = catalog.spec(strike_atom).unwrap();
        for (mut no_op, callback) in [
            (
                malformed.clone(),
                after_null_card_generated
                    as fn(
                        &mut HotState,
                        &Catalog,
                        &crate::catalog::CardSpec,
                        &mut Vec<Event>,
                    ) -> Result<(), EngineRefusal>,
            ),
            (malformed.clone(), after_owner_created_remote_card_generated),
        ] {
            let before = no_op.clone();
            callback(&mut no_op, &catalog, wound_spec, &mut Vec::new()).unwrap();
            assert_eq!(no_op, before);
        }
        let mut non_status = malformed.clone();
        let before = non_status.clone();
        after_owner_card_generated(&mut non_status, &catalog, strike_spec, &mut Vec::new())
            .unwrap();
        assert_eq!(non_status, before);
        let mut terminal = malformed;
        terminal.history.over = true;
        let before = terminal.clone();
        after_owner_card_generated(&mut terminal, &catalog, wound_spec, &mut Vec::new()).unwrap();
        assert_eq!(terminal, before);
    }

    #[test]
    fn rocket_punch_levels_deal_exact_damage_draw_exactly_and_suppress_draw_after_lethal() {
        for (upgrade, expected_damage, expected_draw) in [(0, 13, 1usize), (1, 14, 2)] {
            let rocket = CardIdentity {
                id: CardId::RocketPunch,
                upgrade,
                enchantment: None,
            };
            let filler = CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let rocket_atom = builder.intern(rocket).unwrap();
            let filler_atom = builder.intern(filler).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 2;
            state.next_card_uid = 10;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom: rocket_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend([8, 9].map(|uid| HotCard {
                    uid,
                    atom: filler_atom,
                    flags: 0,
                }));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));

            let played = crate::engine::apply_action_into(
                &state,
                &catalog,
                &crate::engine::Action::Play {
                    uid: 7,
                    target: Some(0),
                    selection: crate::engine::SelectionRef::NONE,
                },
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(played.monsters[0].hp, 100 - expected_damage);
            assert_eq!(played.piles.get(PileId::Hand).len(), expected_draw);
            assert_eq!(played.piles.get(PileId::Draw).len(), 2 - expected_draw);
        }

        let rocket = CardIdentity {
            id: CardId::RocketPunch,
            upgrade: 0,
            enchantment: None,
        };
        let filler = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let rocket_atom = builder.intern(rocket).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut lethal = HotState::at_defaults();
        lethal.hp = 50;
        lethal.energy = 2;
        lethal.next_card_uid = 9;
        lethal.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: rocket_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        lethal.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 8,
            atom: filler_atom,
            flags: 0,
        });
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 13));
        let lethal = crate::engine::apply_action_into(
            &lethal,
            &catalog,
            &crate::engine::Action::Play {
                uid: 7,
                target: Some(0),
                selection: crate::engine::SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.history.over);
        assert!(lethal.piles.get(PileId::Hand).is_empty());
        assert_eq!(lethal.piles.get(PileId::Draw).len(), 1);
    }

    #[test]
    fn sword_sage_delta_uses_native_pile_order_and_is_atomic() {
        let identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        for (pile, uid) in [
            (PileId::Hand, 1),
            (PileId::Draw, 2),
            (PileId::Discard, 3),
            (PileId::Exhaust, 4),
            (PileId::Play, 5),
        ] {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid,
                atom,
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
            });
        }
        SWORD_SAGE_DELTA_TRACE.with(|trace| trace.borrow_mut().clear());
        apply_sword_sage_blade_delta(&mut state, &catalog, 1).unwrap();
        assert_eq!(
            SWORD_SAGE_DELTA_TRACE.with(|trace| trace.borrow().clone()),
            [
                (PileId::Hand, 1),
                (PileId::Draw, 2),
                (PileId::Discard, 3),
                (PileId::Exhaust, 4),
                (PileId::Play, 5),
            ]
        );
        for pile in PileId::ALL {
            let card = state.piles.get(pile).as_slice()[0];
            assert_eq!(state.card_states.get(card.uid).base_replay_count(), Some(1));
            assert_ne!(card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        }

        let mut overflow = state.clone();
        let mut maxed = overflow.card_states.get(4);
        maxed.set_base_replay_count(Some(i32::MAX)).unwrap();
        overflow.card_states.set(4, maxed);
        let before = overflow.clone();
        assert!(apply_sword_sage_blade_delta(&mut overflow, &catalog, 1).is_err());
        assert_eq!(overflow, before);
    }

    #[test]
    fn frozen_multi_pile_move_preflights_the_complete_batch_atomically() {
        let identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let instance = CardInstanceState {
            damage_growth: 19,
            ..CardInstanceState::default()
        };
        let cards = [
            (PileId::Hand, 11_u32),
            (PileId::Discard, 12),
            (PileId::Exhaust, 13),
            (PileId::Play, 14),
        ];
        let mut state = HotState::at_defaults();
        state.next_card_uid = 20;
        let mut frozen = Vec::new();
        for (pile, uid) in cards {
            let card = HotCard {
                uid,
                atom,
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
            };
            let index = state.piles.get(pile).len();
            state.piles.get_mut(pile).make_mut().push(card);
            state.card_states.set(uid, instance.clone());
            frozen.push(FrozenPhysicalCardMove {
                source_pile: pile,
                source_index: index,
                card,
                instance: instance.clone(),
            });
        }
        let before = state.clone();
        let mut changed = frozen.clone();
        changed.last_mut().unwrap().instance.damage_growth += 1;
        let mut events = vec![Event::TurnBegan { turn: 8 }];
        let before_events = events.clone();
        let mut terminal = state.clone();
        terminal.history.over = true;
        let terminal_before = terminal.clone();
        crate::engine::play::with_test_active_play(frozen[0].card.uid, || {
            move_frozen_physical_cards_to_bottom(
                &mut terminal,
                &catalog,
                &changed,
                PileId::Draw,
                &mut events,
            )
        })
        .unwrap();
        assert_eq!(terminal, terminal_before);
        assert_eq!(events, before_events);

        let mut safe_split = before.clone();
        let mut safe_split_frozen = frozen.clone();
        safe_split_frozen[1].instance.damage_growth += 1;
        safe_split
            .card_states
            .set(12, safe_split_frozen[1].instance.clone());
        move_frozen_physical_cards_to_bottom(
            &mut safe_split,
            &catalog,
            &safe_split_frozen,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(
            !safe_split.exact_piles,
            "an unequal Hand/Discard batch stays inexact after the pure move"
        );

        assert!(
            move_frozen_physical_cards_to_bottom(
                &mut state,
                &catalog,
                &changed,
                PileId::Draw,
                &mut events,
            )
            .is_err()
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        let mut vanished = before.clone();
        vanished.piles.get_mut(PileId::Play).make_mut().clear();
        let vanished_before = vanished.clone();
        assert!(matches!(
            move_frozen_physical_cards_to_bottom(
                &mut vanished,
                &catalog,
                &frozen,
                PileId::Draw,
                &mut events,
            ),
            Err(EngineRefusal::FrozenCardVanished { uid: 14, .. })
        ));
        assert_eq!(vanished, vanished_before);

        let mut duplicate_live = before.clone();
        duplicate_live
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(frozen[0].card);
        let duplicate_live_before = duplicate_live.clone();
        assert_eq!(
            move_frozen_physical_cards_to_bottom(
                &mut duplicate_live,
                &catalog,
                &frozen,
                PileId::Draw,
                &mut events,
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(duplicate_live, duplicate_live_before);

        let mut legacy = before.clone();
        legacy.piles.get_mut(PileId::Hand).make_mut()[0].flags |= CARD_FLAG_LEGACY;
        let mut legacy_frozen = frozen.clone();
        legacy_frozen[0].card.flags |= CARD_FLAG_LEGACY;
        let legacy_before = legacy.clone();
        assert!(matches!(
            move_frozen_physical_cards_to_bottom(
                &mut legacy,
                &catalog,
                &legacy_frozen,
                PileId::Draw,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "physical move frozen identity"
            ))
        ));
        assert_eq!(legacy, legacy_before);

        let mut unknown = before.clone();
        unknown.piles.get_mut(PileId::Hand).make_mut()[0].atom = u16::MAX;
        let mut unknown_frozen = frozen.clone();
        unknown_frozen[0].card.atom = u16::MAX;
        let unknown_before = unknown.clone();
        assert_eq!(
            move_frozen_physical_cards_to_bottom(
                &mut unknown,
                &catalog,
                &unknown_frozen,
                PileId::Draw,
                &mut events,
            ),
            Err(EngineRefusal::UnknownAtom(u16::MAX))
        );
        assert_eq!(unknown, unknown_before);

        let mut duplicate = frozen.clone();
        duplicate.push(frozen[0].clone());
        assert!(matches!(
            move_frozen_physical_cards_to_bottom(
                &mut state,
                &catalog,
                &duplicate,
                PileId::Draw,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "physical move duplicate frozen identity"
            ))
        ));
        assert_eq!(state, before);

        move_frozen_physical_cards_to_bottom(
            &mut state,
            &catalog,
            &frozen,
            PileId::Draw,
            &mut events,
        )
        .unwrap();
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [11, 12, 13, 14]
        );
        assert!(
            !state.exact_piles,
            "a pure safe move preserves inexact mode"
        );
        assert!(cards.into_iter().all(|(_, uid)| {
            state.card_states.get(uid) == instance
                && state
                    .piles
                    .get(PileId::Draw)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == uid && card.flags == CARD_FLAG_SOVEREIGN_BLADE_STATE)
        }));
    }

    #[test]
    fn existing_card_pile_moves_have_no_admitted_changed_piles_listener() {
        use crate::engine::admission::IMPLEMENTED_RELICS;
        use crate::ids::RelicId;

        assert!(
            [
                RelicId::RelicBingBong,
                RelicId::RelicDarkstonePeriapt,
                RelicId::RelicLuckyFysh,
            ]
            .into_iter()
            .all(|relic| !IMPLEMENTED_RELICS.contains(&relic)),
            "an AfterCardChangedPiles listener became reachable; frozen physical moves must dispatch it"
        );
    }

    fn creative_ai_catalog(count: usize) -> Catalog {
        let mut builder = CatalogBuilder::new();
        for id in CREATIVE_AI_POWER_POOL_V1101.into_iter().take(count) {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.build()
    }

    fn creative_ai_state() -> HotState {
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Defect);
        state.set_creative_ai_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
    }

    fn spectrum_shift_catalog(count: usize) -> Catalog {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::REGENT_COLORLESS_GENERATION_POOL_V1101
            .into_iter()
            .take(count)
        {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.build()
    }

    fn spectrum_shift_state() -> HotState {
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Regent);
        state.entropy_card_pool = Some(RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
    }

    #[test]
    fn spectrum_shift_full_shuffles_once_and_keeps_one_distinct_prefix() {
        let catalog = spectrum_shift_catalog(
            crate::content_tables::REGENT_COLORLESS_GENERATION_POOL_V1101.len(),
        );
        let mut state = spectrum_shift_state();

        let selected = select_spectrum_shift_cards(&mut state, &catalog, 3).unwrap();

        assert_eq!(
            selected,
            [CardId::FlashOfSteel, CardId::PrepTime, CardId::Entropy].map(|id| CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            })
        );
        assert_eq!(
            selected
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(state.rng.get(RngStream::Generation).counter, 49);
    }

    #[test]
    fn spectrum_shift_refuses_provenance_and_incomplete_pool_before_rng() {
        let full = spectrum_shift_catalog(
            crate::content_tables::REGENT_COLORLESS_GENERATION_POOL_V1101.len(),
        );
        let valid = spectrum_shift_state();
        for mut malformed in [
            {
                let mut state = valid.clone();
                state.reward_card_pool = Some(RewardPool::Defect);
                state
            },
            {
                let mut state = valid.clone();
                state.entropy_card_pool = Some(RewardPool::Ironclad);
                state
            },
            {
                let mut state = valid.clone();
                state.fully_unlocked_card_pool_epochs = false;
                state
            },
            {
                let mut state = valid.clone();
                state.multiplayer_ally_key = 1;
                state
            },
        ] {
            let before = malformed.clone();
            assert!(select_spectrum_shift_cards(&mut malformed, &full, 1).is_err());
            assert_eq!(malformed, before);
        }

        let incomplete = spectrum_shift_catalog(
            crate::content_tables::REGENT_COLORLESS_GENERATION_POOL_V1101.len() - 1,
        );
        let mut state = valid;
        let before = state.clone();
        assert!(matches!(
            select_spectrum_shift_cards(&mut state, &incomplete, 1),
            Err(EngineRefusal::UnknownMintIdentity(_))
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn creative_ai_foundation_pins_the_complete_current_power_pool() {
        assert_eq!(
            CREATIVE_AI_POWER_POOL_V1101,
            [
                CardId::Buffer,
                CardId::BulkUp,
                CardId::Capacitor,
                CardId::ConsumingShadow,
                CardId::Coolant,
                CardId::CreativeAi,
                CardId::Defragment,
                CardId::EchoForm,
                CardId::Feral,
                CardId::Hailstorm,
                CardId::Iteration,
                CardId::Loop,
                CardId::MachineLearning,
                CardId::Smokestack,
                CardId::Spinner,
                CardId::Storm,
                CardId::Subroutine,
                CardId::Thunder,
                CardId::TrashToTreasure,
            ]
        );
        assert_eq!(
            CREATIVE_AI_POWER_POOL_V1101
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            19
        );
        for id in CREATIVE_AI_POWER_POOL_V1101 {
            for upgrade in 0..=1 {
                let row = card_row(id, upgrade).expect("both native levels stay cataloged");
                assert!(row.is_power, "{id:?}+{upgrade} stopped being a Power");
                assert!(
                    row.playable,
                    "{id:?}+{upgrade} stopped being a playable catalog row"
                );
            }
        }
    }

    #[test]
    fn creative_ai_foundation_full_shuffles_once_per_live_iteration() {
        let catalog = creative_ai_catalog(CREATIVE_AI_POWER_POOL_V1101.len());
        let mut state = creative_ai_state();

        let selected = (0..3)
            .map(|_| select_creative_ai_power(&mut state, &catalog).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            selected,
            [CardId::Hailstorm, CardId::BulkUp, CardId::Loop].map(|id| CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            })
        );
        assert_eq!(
            state.rng.get(RngStream::Generation),
            RngStreamState {
                words: [
                    7_760_106_857_133_472_772,
                    6_500_043_880_324_423_956,
                    6_789_185_004_799_046_372,
                    13_915_850_503_358_045_583,
                ],
                counter: 54,
            }
        );
    }

    #[test]
    fn creative_ai_foundation_refuses_provenance_and_catalog_drift_before_rng() {
        let catalog = creative_ai_catalog(CREATIVE_AI_POWER_POOL_V1101.len());
        let valid = creative_ai_state();
        for mut malformed in [
            {
                let mut state = valid.clone();
                state.reward_card_pool = None;
                state
            },
            {
                let mut state = valid.clone();
                state.multiplayer_ally_key = 1;
                state
            },
            {
                let mut state = valid.clone();
                state.fully_unlocked_card_pool_epochs = false;
                state
            },
            {
                let mut state = valid.clone();
                state
                    .rng
                    .set(RngStream::Generation, RngStreamState::default());
                state
            },
        ] {
            let before = malformed.clone();
            assert!(matches!(
                select_creative_ai_power(&mut malformed, &catalog),
                Err(EngineRefusal::MalformedArgs(
                    "Creative AI generation provenance"
                ))
            ));
            assert_eq!(malformed, before);
        }

        let incomplete = creative_ai_catalog(CREATIVE_AI_POWER_POOL_V1101.len() - 1);
        let mut state = valid;
        let before = state.clone();
        assert!(matches!(
            select_creative_ai_power(&mut state, &incomplete),
            Err(EngineRefusal::UnknownMintIdentity(CardIdentity {
                id: CardId::TrashToTreasure,
                upgrade: 0,
                enchantment: None,
            }))
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn minting_allocates_monotonic_uids_without_forcing_exact_piles() {
        let (mut state, catalog, identity) = fixture();
        state.next_card_uid = 41;

        let first = mint_card(&mut state, &catalog, identity).unwrap();
        let second = mint_card(&mut state, &catalog, identity).unwrap();

        assert_eq!((first.uid, second.uid, state.next_card_uid), (41, 42, 43));
        assert_eq!(first.atom, second.atom);
        assert!(!state.exact_piles);
    }

    fn apotheosis_upgrade_catalog(include_upgrade: bool) -> (Catalog, HotCard, Option<HotCard>) {
        let mut builder = CatalogBuilder::new();
        let base_atom = builder
            .intern(CardIdentity {
                id: CardId::Apotheosis,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let upgraded_atom = include_upgrade.then(|| {
            builder
                .intern(CardIdentity {
                    id: CardId::Apotheosis,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap()
        });
        (
            builder.build(),
            HotCard {
                uid: 41,
                atom: base_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            upgraded_atom.map(|atom| HotCard {
                uid: 42,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }),
        )
    }

    #[test]
    fn physical_upgrade_preserves_frozen_pile_order_payload_and_clamps_only_set_rows() {
        let (catalog, base, upgraded) = apotheosis_upgrade_catalog(true);
        let upgraded = upgraded.unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([base, upgraded]);
        let before_payload = CardInstanceState {
            damage_growth: 7,
            local_cost_modifiers: crate::hot::LocalCostModifiers::from_rows(vec![
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: 9,
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: true,
                },
                LocalCostModifier {
                    kind: LocalCostModifierKind::Add,
                    amount: 3,
                    expiration: LocalCostExpiration::UntilPlayed,
                    reduce_only: false,
                },
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: 0,
                    expiration: LocalCostExpiration::ThisTurnOrPlayed,
                    reduce_only: false,
                },
            ]),
            local_retain: true,
            local_sly: true,
            transient_retain: true,
            ..CardInstanceState::default()
        };
        state.card_states.set(base.uid, before_payload.clone());

        upgrade_live_cards_once(&mut state, &catalog, &[base.uid, upgraded.uid]).unwrap();

        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(
            hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [41, 42]
        );
        assert_eq!(hand[0].atom, upgraded.atom);
        assert_eq!(hand[1], upgraded, "a max-level card remains byte-identical");
        let after_payload = state.card_states.get(base.uid);
        assert_eq!(after_payload.damage_growth, before_payload.damage_growth);
        assert_eq!(after_payload.local_retain, before_payload.local_retain);
        assert_eq!(after_payload.local_sly, before_payload.local_sly);
        assert_eq!(
            after_payload.transient_retain,
            before_payload.transient_retain
        );
        let rows = after_payload.local_cost_modifiers.as_slice();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].kind, rows[0].amount, rows[0].reduce_only),
            (LocalCostModifierKind::Set, 1, true,)
        );
        assert_eq!(
            (rows[1].kind, rows[1].amount),
            (LocalCostModifierKind::Add, 3)
        );
        assert_eq!(
            (rows[2].kind, rows[2].amount),
            (LocalCostModifierKind::Set, 0)
        );
        assert!(
            state.exact_piles,
            "the converged L1 copies have distinguishable physical payloads"
        );
    }

    #[test]
    fn physical_upgrade_keeps_equal_payload_hand_order_canonical() {
        let (catalog, first, upgraded) = apotheosis_upgrade_catalog(true);
        let upgraded = upgraded.unwrap();
        let mut second = first;
        second.uid = 43;
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([first, second]);

        upgrade_live_cards_once(&mut state, &catalog, &[second.uid, first.uid]).unwrap();

        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(
            hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [first.uid, second.uid],
            "the frozen command order never reorders the live Hand"
        );
        assert!(hand.iter().all(|card| card.atom == upgraded.atom));
        assert!(
            !state.exact_piles,
            "equal upgraded payloads remain safe under canonical tie order"
        );
    }

    #[test]
    /// #2965: a combat upgrade leaves the Hopper master (`DeckVersion`) row at
    /// its pre-combat level; only the live objects move.
    fn hopper_mass_upgrade_leaves_the_master_row_unupgraded() {
        let (catalog, master, upgraded) = apotheosis_upgrade_catalog(true);
        let upgraded = upgraded.unwrap();
        let generated = HotCard { uid: 43, ..master };
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([master, generated]);
        state.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card: master,
                state: CardInstanceState::default(),
            }],
            history: Vec::new(),
        }));

        upgrade_live_cards_once(&mut state, &catalog, &[master.uid, generated.uid]).unwrap();

        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!([hand[0].atom, hand[1].atom], [upgraded.atom; 2]);
        let deck = state.card_states.hopper().unwrap();
        assert_eq!(deck.master.len(), 1);
        assert_eq!(deck.master[0].card.uid, master.uid);
        assert_eq!(deck.master[0].card.atom, master.atom);
        assert!(deck.master.iter().all(|row| row.card.uid != generated.uid));
    }

    #[test]
    fn physical_upgrade_is_terminal_gated_and_missing_catalog_closure_is_atomic() {
        let (catalog, base, _) = apotheosis_upgrade_catalog(false);
        let mut missing = HotState::at_defaults();
        missing.piles.get_mut(PileId::Hand).make_mut().push(base);
        missing.card_states.set(
            base.uid,
            CardInstanceState {
                local_retain: true,
                ..CardInstanceState::default()
            },
        );
        let before = missing.clone();
        assert_eq!(
            upgrade_live_cards_once(&mut missing, &catalog, &[base.uid]),
            Err(EngineRefusal::UnknownMintIdentity(CardIdentity {
                id: CardId::Apotheosis,
                upgrade: 1,
                enchantment: None,
            }))
        );
        assert_eq!(missing, before);

        missing.history.over = true;
        let terminal = missing.clone();
        upgrade_live_cards_once(&mut missing, &catalog, &[base.uid]).unwrap();
        assert_eq!(
            missing, terminal,
            "native Upgrade no-ops once combat is ending"
        );
    }

    #[test]
    fn physical_upgrade_rejects_missing_duplicate_and_legacy_identities_atomically() {
        let (catalog, base, _) = apotheosis_upgrade_catalog(true);
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().push(base);
        for frozen in [&[99][..], &[base.uid, base.uid][..]] {
            let before = state.clone();
            assert!(upgrade_live_cards_once(&mut state, &catalog, frozen).is_err());
            assert_eq!(state, before);
        }

        state.piles.get_mut(PileId::Draw).make_mut().push(base);
        let duplicate = state.clone();
        assert_eq!(
            upgrade_live_cards_once(&mut state, &catalog, &[base.uid]),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, duplicate);

        state.piles.get_mut(PileId::Draw).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut()[0].flags |= CARD_FLAG_LEGACY;
        let legacy = state.clone();
        assert_eq!(
            upgrade_live_cards_once(&mut state, &catalog, &[base.uid]),
            Err(EngineRefusal::MalformedArgs(
                "CardCmd::Upgrade requires physical card identity"
            ))
        );
        assert_eq!(state, legacy);
    }

    #[test]
    fn native_upgrade_eligibility_matches_the_complete_authoritative_maximum_table() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../python/card_templates_raw.json")).unwrap();
        let maxima = raw["max_upgrade"].as_object().unwrap();
        let mut ids = crate::content_tables::CARD_ROWS
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        ids.sort_by_key(|id| *id as u16);
        ids.dedup();

        for id in ids {
            let source = maxima[&format!("CARD.{}", id.as_str())]
                .as_u64()
                .and_then(|value| u8::try_from(value).ok());
            assert_eq!(native_max_upgrade(id), source, "{id:?}");
        }

        let mismatches = crate::content_tables::CARD_ROWS
            .iter()
            .filter_map(|row| {
                let next = row.upgrade.checked_add(1)?;
                (native_card_is_upgradable(CardIdentity {
                    id: row.id,
                    upgrade: row.upgrade,
                    enchantment: None,
                }) != crate::content_tables::card_row(row.id, next).is_some())
                .then_some((row.id, row.upgrade, next))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mismatches,
            [],
            "a modeled-next-row test is not native IsUpgradable"
        );

        let source = include_str!("cards.rs");
        let predicate = source
            .split_once("pub(crate) fn native_card_is_upgradable")
            .unwrap()
            .1
            .split_once("/// Whether Python's")
            .unwrap()
            .0;
        assert!(!predicate.contains("CARD_ROWS"));
        assert!(!predicate.contains("native_max_upgrade"));

        let upgrade_body = source
            .split_once("pub(crate) fn upgrade_live_cards_once")
            .unwrap()
            .1
            .split_once("/// RingingPower.AfterApplied")
            .unwrap()
            .0;
        assert!(!upgrade_body.contains("CARD_ROWS"));
        assert!(!upgrade_body.contains("native_max_upgrade"));
    }

    fn storm_catalog(upgrade: u8) -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::StormOfSteel,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.build()
    }

    #[test]
    fn blade_plural_allocates_all_uids_and_enchants_live_terminal_prefix() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::BladeOfInk,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.inky_attack_damage = 0;
        state.next_card_uid = 40;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
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

        inject_generated_blade_inky_shivs(&mut state, &catalog, 3, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.next_card_uid, 43);
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 1);
        assert_eq!(hand[0].uid, 40);
        assert_eq!(
            catalog.spec(hand[0].atom).unwrap().identity,
            CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment {
                    id: EnchantmentId::Inky,
                    amount: 1,
                }),
            }
        );
        assert!(PileId::ALL.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| !matches!(card.uid, 41 | 42))
        }));
    }

    #[test]
    fn blade_inky_atom_rewrite_preserves_pile_flags_and_complete_instance_payload() {
        let plain = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let inky = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Inky,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let plain_atom = builder.intern(plain).unwrap();
        let inky_atom = builder.intern(inky).unwrap();
        let catalog = builder.build();
        let returned = [
            HotCard {
                uid: 40,
                atom: plain_atom,
                flags: CARD_FLAG_PICK,
            },
            HotCard {
                uid: 41,
                atom: plain_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ];
        let mut state = HotState::at_defaults();
        state.exact_piles = true;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(returned[0]);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(returned[1]);
        let mut rich = CardInstanceState {
            damage_growth: 7,
            local_cost_modifiers: LocalCostModifiers::from_rows(vec![LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            }]),
            local_retain: true,
            local_sly: true,
            transient_retain: true,
            free_star_cost_this_turn_or_played_rows: 1,
            ..CardInstanceState::default()
        };
        rich.set_local_ethereal(true);
        rich.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(40, rich);
        state.card_states.set_local_retain(41);
        let states_before = state.card_states.clone();

        rewrite_live_plural_shiv_atoms(&mut state, &catalog, &returned, plain, inky_atom, false)
            .unwrap();

        assert_eq!(
            state.piles.get(PileId::Hand).as_slice(),
            &[HotCard {
                atom: inky_atom,
                ..returned[0]
            }]
        );
        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            &[HotCard {
                atom: inky_atom,
                ..returned[1]
            }]
        );
        assert_eq!(state.card_states, states_before);
        assert!(state.card_states.get(40).local_ethereal());
        assert!(state.card_states.get(40).local_retain);
        assert!(state.card_states.get(40).local_sly);
    }

    #[test]
    fn storm_bulk_allocates_before_a_terminal_generated_hook_and_suppresses_upgrade() {
        let catalog = storm_catalog(1);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 40;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
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
        let mut events = Vec::new();

        inject_generated_storm_shivs(&mut state, &catalog, 3, 1, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.next_card_uid, 43, "all three UIDs allocate first");
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 1, "only the pre-terminal prefix enters a pile");
        assert_eq!(hand[0].uid, 40);
        assert_eq!(
            catalog.spec(hand[0].atom).unwrap().identity,
            CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            },
            "the one-shot upgrade suffix is entirely ending-gated"
        );
        assert!(PileId::ALL.into_iter().all(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| !matches!(card.uid, 41 | 42))
        }));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::CardResolved {
                uid: 40,
                pile: PileId::Hand
            }
        )));
    }

    /// `Shiv/<CreateInHand>d__12::MoveNext` (0x3bb20c IL_0031) returns at
    /// IsOverOrEnding before allocating anything. With the only primary dead
    /// and no veto, before history.over latches, Storm of Steel / Cunning
    /// Potion (L0 and upgraded) and Blade of Ink create nothing: no UID, no
    /// history, no pile entry, no upgrade. An Adaptable veto keeps the same
    /// roster live and a live monster is the ordinary control; both create.
    #[test]
    fn plural_shivs_skip_while_combat_is_ending_before_the_over_latch() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::StormOfSteel, CardId::BladeOfInk] {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let at = |hp: i32, veto: bool| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.inky_attack_damage = 0;
            state.next_card_uid = 40;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, hp));
            if veto {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Adaptable, SlotWire::Int, 1);
            }
            assert!(!state.history.over);
            state
        };
        type Inject = fn(&mut HotState, &Catalog, &mut Vec<Event>) -> Result<(), EngineRefusal>;
        let injectors: [(&str, Inject); 3] = [
            ("storm L0", |state, catalog, events| {
                inject_generated_storm_shivs(state, catalog, 3, 0, events)
            }),
            ("storm L1 / Cunning", |state, catalog, events| {
                inject_generated_storm_shivs(state, catalog, 3, 1, events)
            }),
            ("blade of ink", |state, catalog, events| {
                inject_generated_blade_inky_shivs(state, catalog, 3, events)
            }),
        ];
        for (name, inject) in injectors {
            let mut ending = at(0, false);
            assert!(crate::engine::damage::damage_combat_is_ending(&ending));
            let before = ending.clone();
            let mut events = Vec::new();
            inject(&mut ending, &catalog, &mut events).unwrap();
            assert_eq!(ending, before, "{name}: nothing is created while ending");
            assert!(events.is_empty(), "{name}");

            for mut live in [at(0, true), at(10, false)] {
                assert!(!crate::engine::damage::damage_combat_is_ending(&live));
                inject(&mut live, &catalog, &mut Vec::new()).unwrap();
                assert_eq!(live.next_card_uid, 43, "{name}");
                assert_eq!(live.history.owner_generated_cards_combat, 3, "{name}");
                assert_eq!(live.piles.get(PileId::Hand).len(), 3, "{name}");
            }
        }
    }

    #[test]
    fn storm_bulk_routes_each_preallocated_uid_through_the_live_hand_cap() {
        let catalog = storm_catalog(0);
        let filler_atom = catalog
            .atom(&CardIdentity {
                id: CardId::StormOfSteel,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let mut state = HotState::at_defaults();
        // A live enemy: with none, combat is ending and plural Shiv
        // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.hp = 50;
        state.next_card_uid = 90;
        for uid in 1..=9 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            });
        }
        let mut events = Vec::new();

        inject_generated_storm_shivs(&mut state, &catalog, 2, 0, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(
            state.piles.get(PileId::Hand).as_slice().last().unwrap().uid,
            90
        );
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [91]
        );
        assert_eq!(state.next_card_uid, 92);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::CardResolved { uid, pile } => Some((*uid, *pile)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [(90, PileId::Hand), (91, PileId::Discard)]
        );
    }

    #[test]
    fn storm_upgrade_requires_its_independent_l1_catalog_arm_atomically() {
        let catalog = storm_catalog(0);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 12;
        let before = state.clone();
        let mut events = Vec::new();

        assert!(matches!(
            inject_generated_storm_shivs(&mut state, &catalog, 1, 1, &mut events),
            Err(EngineRefusal::UnknownMintIdentity(CardIdentity {
                id: CardId::Shiv,
                upgrade: 1,
                enchantment: None,
            }))
        ));
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn live_tie_projection_covers_payload_and_future_identity_divergence() {
        let ordinary = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let divergent_card = CardIdentity {
            id: CardId::Stomp,
            upgrade: 0,
            enchantment: None,
        };
        let sharp_one = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Sharp,
                amount: 1,
            }),
        };
        let sharp_two = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Sharp,
                amount: 2,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let ordinary_atom = builder.intern(ordinary).unwrap();
        let divergent_atom = builder.intern(divergent_card).unwrap();
        let sharp_one_atom = builder.intern(sharp_one).unwrap();
        let sharp_two_atom = builder.intern(sharp_two).unwrap();
        let catalog = builder.build();
        let tied_state = |atom| {
            let mut state = HotState::at_defaults();
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(HotCard {
                    uid: 2,
                    atom,
                    flags: 0,
                });
            state
        };

        let mut ordinary_tie = tied_state(ordinary_atom);
        assert!(!live_cards_need_exact_piles(&ordinary_tie, &catalog).unwrap());
        ordinary_tie.card_states.set_local_sly(1);
        assert!(live_cards_need_exact_piles(&ordinary_tie, &catalog).unwrap());

        let mut flag_tie = tied_state(ordinary_atom);
        flag_tie.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_RINGING;
        assert!(live_cards_need_exact_piles(&flag_tie, &catalog).unwrap());

        assert!(live_cards_need_exact_piles(&tied_state(divergent_atom), &catalog).unwrap());

        let mut identity_tie = HotState::at_defaults();
        identity_tie
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom: sharp_one_atom,
                flags: 0,
            });
        identity_tie
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 2,
                atom: sharp_two_atom,
                flags: 0,
            });
        assert!(live_cards_need_exact_piles(&identity_tie, &catalog).unwrap());
    }

    #[test]
    fn live_tie_projection_pins_all_eight_divergent_enchantment_classes() {
        for id in [
            EnchantmentId::Glam,
            EnchantmentId::Sown,
            EnchantmentId::Swift,
            EnchantmentId::Momentum,
            EnchantmentId::Vigorous,
            EnchantmentId::SlumberingEssence,
            EnchantmentId::Goopy,
            EnchantmentId::Slither,
        ] {
            let identity = CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment { id, amount: 1 }),
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2,
                atom,
                flags: 0,
            });

            assert!(
                live_cards_need_exact_piles(&state, &catalog).unwrap(),
                "{id:?} must keep equal fresh copies identity-sensitive"
            );
        }
    }

    #[test]
    fn live_tie_projection_pins_all_twenty_five_divergent_physical_classes() {
        let ids = [
            CardId::KinglyKick,
            CardId::KinglyPunch,
            CardId::BansheesCry,
            CardId::Flatten,
            CardId::Melancholy,
            CardId::Midnight,
            CardId::Pinpoint,
            CardId::UpMySleeve,
            CardId::Enlightenment,
            CardId::BulletTime,
            CardId::Modded,
            CardId::RocketPunch,
            CardId::Stomp,
            CardId::Claw,
            CardId::GeneticAlgorithm,
            CardId::Maul,
            CardId::MomentumStrike,
            CardId::Rampage,
            CardId::TheBall,
            CardId::TheScythe,
            CardId::Thrash,
            CardId::Wither,
            CardId::Bolas,
            CardId::ThrummingHatchet,
            CardId::MadScience,
        ];
        assert_eq!(ids.len(), 25);
        for id in ids {
            let identity = CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            // #2942: a Mad Science is interned under its fight variant.
            builder.set_mad_science_variant(
                crate::catalog::MadScienceVariant::from_saved(2, 4).unwrap(),
            );
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2,
                atom,
                flags: 0,
            });

            assert!(
                live_cards_need_exact_piles(&state, &catalog).unwrap(),
                "{id:?} must keep equal fresh copies identity-sensitive"
            );
        }
    }

    #[test]
    fn hand_insertion_overflows_each_card_after_the_tenth() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.next_card_uid = 9;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..9).map(|uid| HotCard {
                uid,
                atom,
                flags: 0,
            }));
        let mut events = Vec::new();

        inject_bottom(&mut state, &catalog, identity, 3, PileId::Hand, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), MAX_CARDS_IN_HAND);
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[9].uid, 9);
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [10, 11]
        );
        assert_eq!(state.next_card_uid, 12);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn phantom_blades_retains_each_generated_shiv_before_hand_or_overflow_routing() {
        let shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let filler = CardIdentity {
            id: CardId::StrikeSilent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(shiv).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        let catalog = builder.build();

        let mut hand = HotState::at_defaults();
        hand.powers.set(PowerId::PhantomBlades, SlotWire::Int, 9);
        hand.next_card_uid = 20;
        inject_generated_bottom(&mut hand, &catalog, shiv, 1, PileId::Hand, &mut Vec::new())
            .unwrap();
        assert_eq!(hand.piles.get(PileId::Hand).as_slice()[0].uid, 20);
        assert!(hand.card_states.get(20).local_retain);
        assert!(hand.exact_piles);

        let mut overflow = HotState::at_defaults();
        overflow
            .powers
            .set(PowerId::PhantomBlades, SlotWire::Int, 9);
        overflow.next_card_uid = 30;
        overflow
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..10).map(|uid| HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            }));
        inject_generated_bottom(
            &mut overflow,
            &catalog,
            shiv,
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(overflow.piles.get(PileId::Discard).as_slice()[0].uid, 30);
        assert!(overflow.card_states.get(30).local_retain);
        assert!(overflow.exact_piles);
    }

    #[test]
    fn ringing_marks_current_and_future_cards_then_clears_only_its_tail() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 0,
            atom,
            flags: 0,
        });
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });

        apply_ringing_power(&mut state).unwrap();
        assert!(state.ringing() && state.exact_piles);
        assert!(
            ALL_CARD_PILES
                .iter()
                .all(|pile| state
                    .piles
                    .get(*pile)
                    .as_slice()
                    .iter()
                    .all(|card| card.flags
                        & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING)
                        == (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING)))
        );

        inject_generated_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        let entered = state.piles.get(PileId::Draw).as_slice()[0];
        assert_eq!(entered.uid, 2);
        assert_ne!(entered.flags & CARD_FLAG_RINGING, 0);

        remove_ringing_power(&mut state);
        assert!(!state.ringing());
        assert!(ALL_CARD_PILES.iter().all(|pile| {
            state.piles.get(*pile).as_slice().iter().all(|card| {
                card.flags & CARD_FLAG_RINGING == 0
                    && card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0
            })
        }));
    }

    #[test]
    fn ringing_refuses_a_legacy_card_before_any_mutation() {
        let (mut state, catalog, identity) = fixture();
        inject_legacy_bottom(&mut state, &catalog, identity, 1, PileId::Discard).unwrap();
        let before = state.clone();
        assert_eq!(
            apply_ringing_power(&mut state),
            Err(EngineRefusal::MalformedArgs(
                "Ringing requires physical card identities"
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn ringing_preflights_late_affliction_overlap_before_any_write() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.exact_piles = true;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 0,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_BOUND,
        });
        let before = state.clone();
        assert_eq!(
            apply_ringing_power(&mut state),
            Err(EngineRefusal::MalformedArgs(
                "overlapping card-affliction powers"
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn hex_marks_and_clears_all_five_native_piles_in_order() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.exact_piles = true;
        state.next_card_uid = CARD_AFFLICTION_PILES.len() as u32;
        for (uid, pile) in CARD_AFFLICTION_PILES.into_iter().enumerate() {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: uid as u32,
                atom,
                flags: 0,
            });
        }

        apply_hex_power(&mut state, 2).unwrap();
        assert_eq!(state.powers.value(PowerId::HexPower), 2);
        assert!(CARD_AFFLICTION_PILES.into_iter().all(|pile| {
            state.piles.get(pile).as_slice()[0].flags
                & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED)
                == (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED)
        }));

        remove_hex_power(&mut state);
        assert_eq!(state.powers.value(PowerId::HexPower), 0);
        assert!(
            CARD_AFFLICTION_PILES
                .into_iter()
                .all(|pile| { state.piles.get(pile).as_slice()[0].flags & CARD_FLAG_HEXED == 0 })
        );
    }

    /// #2959: the Hex gate no longer needs `exact_piles` beforehand — the
    /// application promotes it, like Ringing's — while the strict fresh gate
    /// Chains of Binding reads is unchanged on the same state.
    #[test]
    fn hex_promotes_non_exact_piles_while_the_chains_gate_stays_strict() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.next_card_uid = 2;
        for (uid, pile) in [PileId::Hand, PileId::Draw].into_iter().enumerate() {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: uid as u32,
                atom,
                flags: 0,
            });
        }
        assert!(!state.exact_piles);
        assert!(hex_card_affliction_application_is_exact(&state));
        assert!(!fresh_card_affliction_application_is_exact(&state));

        apply_hex_power(&mut state, 2).unwrap();
        assert!(state.exact_piles);
        assert_eq!(state.powers.value(PowerId::HexPower), 2);
        assert!([PileId::Hand, PileId::Draw].into_iter().all(|pile| {
            state.piles.get(pile).as_slice()[0].flags
                & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED)
                == (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED)
        }));

        // A duplicated physical UID still refuses before any write.
        let (mut duplicated, _, _) = fixture();
        duplicated.next_card_uid = 1;
        for pile in [PileId::Hand, PileId::Draw] {
            duplicated.piles.get_mut(pile).make_mut().push(HotCard {
                uid: 0,
                atom,
                flags: 0,
            });
        }
        let before = duplicated.clone();
        assert!(apply_hex_power(&mut duplicated, 2).is_err());
        assert_eq!(duplicated, before);
    }

    #[test]
    fn spectral_hex_smoke_entry_authenticates_the_complete_fresh_roster() {
        let mut state = HotState::at_defaults();
        state.exact_piles = true;
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
        let catalog = CatalogBuilder::new().build();
        assert!(spectral_hex_fresh_entry_is_exact(&state, &catalog));

        for mutate in 0..10 {
            let mut drift = state.clone();
            match mutate {
                0 => drift.monsters_mut()[0].hp -= 1,
                1 => drift.monsters_mut()[2].block = 1,
                2 => drift.monsters_mut()[0]
                    .powers
                    .set(PowerId::Strength, SlotWire::Int, 1),
                3 => drift.monsters_mut()[0].loop_pos = 1,
                4 => drift.monsters_mut()[2].spawn_noop = true,
                5 => drift.monsters_mut()[2].pressure_gun_damage = 1,
                6 => drift.monsters_mut()[1].block = 1,
                7 => drift.monsters_mut()[1].loop_pos = 1,
                8 => drift.monsters_mut()[1]
                    .powers
                    .set(PowerId::Strength, SlotWire::Int, 1),
                9 => assert!(drift.monsters_mut()[0].random_ai.set_next(Some(1))),
                _ => unreachable!(),
            }
            assert!(
                !spectral_hex_fresh_entry_is_exact(&drift, &catalog),
                "fresh roster drift {mutate}"
            );
        }
    }

    #[test]
    fn every_affliction_source_preflights_every_overlap_direction() {
        let (base, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;

        for mut state in {
            let mut ringing = base.clone();
            ringing.set_ringing(true);
            let mut chains = base.clone();
            chains
                .powers
                .set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
            let mut tailed = base.clone();
            tailed.exact_piles = true;
            tailed.next_card_uid = 1;
            tailed.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 0,
                atom,
                flags: physical | CARD_FLAG_BOUND,
            });
            [ringing, chains, tailed]
        } {
            let before = state.clone();
            assert!(apply_hex_power(&mut state, 2).is_err());
            assert_eq!(state, before);
        }

        for mut state in {
            let mut hex_power = base.clone();
            hex_power.powers.set(PowerId::HexPower, SlotWire::Int, 2);
            let mut chains = base.clone();
            chains
                .powers
                .set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
            let mut tailed = base;
            tailed.exact_piles = true;
            tailed.next_card_uid = 1;
            tailed
                .piles
                .get_mut(PileId::Exhaust)
                .make_mut()
                .push(HotCard {
                    uid: 0,
                    atom,
                    flags: physical | CARD_FLAG_HEXED,
                });
            [hex_power, chains, tailed]
        } {
            let before = state.clone();
            assert!(apply_ringing_power(&mut state).is_err());
            assert_eq!(state, before);
        }
    }

    #[test]
    fn live_hex_afflicts_new_owner_entries_and_rejects_remote_ownership() {
        let (mut state, catalog, identity) = fixture();
        state.exact_piles = true;
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        inject_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        assert_ne!(
            state.piles.get(PileId::Draw).as_slice()[0].flags & CARD_FLAG_HEXED,
            0
        );

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
        assert!(hex_power_state_is_exact(&state));

        let mut rootless = state.clone();
        rootless.monsters_mut()[0].random_ai = Default::default();
        rootless.monsters_mut()[1].random_ai = Default::default();
        assert!(!hex_power_state_is_exact(&rootless));

        state.multiplayer_ally_key = 7;
        assert!(!hex_power_state_is_exact(&state));
    }

    #[test]
    fn clone_preserves_affliction_while_transform_erases_then_live_hex_reapplies() {
        let source_identity = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let replacement_identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source_identity).unwrap();
        builder.intern(replacement_identity).unwrap();
        let catalog = builder.build();
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;

        let mut bound = HotState::at_defaults();
        bound.hp = 50;
        bound.exact_piles = true;
        bound.next_card_uid = 1;
        bound.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
        assert!(bound.set_bound_afflictions_this_turn(1));
        let source = HotCard {
            uid: 0,
            atom: source_atom,
            flags: physical | crate::hot::CARD_FLAG_BOUND,
        };
        bound.piles.get_mut(PileId::Hand).make_mut().push(source);
        inject_generated_clones_bottom(
            &mut bound,
            &catalog,
            source,
            1,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        assert_ne!(
            bound.piles.get(PileId::Draw).as_slice()[0].flags & crate::hot::CARD_FLAG_BOUND,
            0
        );
        assert_eq!(bound.bound_afflictions_this_turn(), 1);
        let source_instance = bound.card_states.get(source.uid);
        let replacements = bulk_transform_fixed_same_pile(
            &mut bound,
            &catalog,
            PileId::Hand,
            &[(source, source_instance)],
            replacement_identity,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(replacements.len(), 1);
        assert_eq!(replacements[0].flags & CARD_AFFLICTION_FLAGS, 0);
        assert_eq!(bound.bound_afflictions_this_turn(), 1);

        let mut hex = HotState::at_defaults();
        hex.hp = 50;
        hex.exact_piles = true;
        hex.next_card_uid = 1;
        hex.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        let hex_source = HotCard {
            uid: 0,
            atom: source_atom,
            flags: physical | CARD_FLAG_HEXED,
        };
        hex.piles.get_mut(PileId::Hand).make_mut().push(hex_source);
        let hex_source_instance = hex.card_states.get(hex_source.uid);
        let replacements = bulk_transform_fixed_same_pile(
            &mut hex,
            &catalog,
            PileId::Hand,
            &[(hex_source, hex_source_instance)],
            replacement_identity,
            &mut Vec::new(),
        )
        .unwrap();
        assert_ne!(replacements[0].flags & CARD_FLAG_HEXED, 0);

        let mut stale = HotState::at_defaults();
        stale.hp = 50;
        stale.exact_piles = true;
        stale.next_card_uid = 1;
        stale
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(hex_source);
        inject_generated_clones_bottom(
            &mut stale,
            &catalog,
            hex_source,
            1,
            PileId::Draw,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            stale.piles.get(PileId::Draw).as_slice()[0].flags & CARD_FLAG_HEXED,
            0
        );
    }

    #[test]
    fn every_physical_entry_helper_propagates_the_live_ringing_tail() {
        let (_, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        let ringing_state = || {
            let mut state = HotState::at_defaults();
            state.set_ringing(true);
            state.exact_piles = true;
            state
        };
        let assert_tail = |state: &HotState, label: &str| {
            let card = state.piles.get(PileId::Discard).as_slice()[0];
            assert_eq!(
                card.flags & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING),
                CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
                "{label} escaped PhysicalCard.AfterEnteredCombat"
            );
        };

        let mut state = ringing_state();
        inject_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_bottom");

        let mut state = ringing_state();
        inject_generated_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_bottom");

        let mut state = ringing_state();
        inject_generated_draw_random(&mut state, &catalog, identity, &mut Vec::new()).unwrap();
        let card = state.piles.get(PileId::Draw).as_slice()[0];
        assert_eq!(
            card.flags & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING),
            CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
            "inject_generated_draw_random escaped PhysicalCard.AfterEnteredCombat"
        );

        let mut state = ringing_state();
        inject_generated_null_random(
            &mut state,
            &catalog,
            identity,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_null_random");

        let mut state = ringing_state();
        inject_generated_record_before_ending_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_record_before_ending_bottom");

        let mut state = ringing_state();
        inject_generated_record_before_ending_draw_random(
            &mut state,
            &catalog,
            identity,
            &mut Vec::new(),
        )
        .unwrap();
        let card = state.piles.get(PileId::Draw).as_slice()[0];
        assert_eq!(
            card.flags & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING),
            CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
            "inject_generated_record_before_ending_draw_random escaped PhysicalCard.AfterEnteredCombat"
        );

        let mut state = ringing_state();
        inject_generated_exact_bottom(
            &mut state,
            &catalog,
            identity,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_exact_bottom");

        let mut state = ringing_state();
        inject_generated_free_this_turn_bottom(
            &mut state,
            &catalog,
            identity,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_free_this_turn_bottom");

        let source = HotCard {
            uid: 99,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = ringing_state();
        inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_clones_bottom");

        let mut state = ringing_state();
        inject_generated_rewritten_clone_bottom(
            &mut state,
            &catalog,
            source,
            CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            CardInstanceState::default(),
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_tail(&state, "inject_generated_rewritten_clone_bottom");
    }

    #[test]
    fn sword_sage_fresh_blades_add_once_clones_copy_and_remote_blades_stay_unchanged() {
        let identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 100;
        state.powers.set(PowerId::SwordSage, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SwordSage])
        );
        assert!(state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            hand: Arc::new(vec![
                MultiplayerAllyCard::sovereign_blade(identity, 10).unwrap()
            ]),
            ..MultiplayerAllyState::default()
        }));

        let prior = HotCard {
            uid: 99,
            atom: catalog.atom(&identity).unwrap(),
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE | CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut prior_state = CardInstanceState::default();
        prior_state.set_base_replay_count(Some(3)).unwrap();
        state.card_states.set(prior.uid, prior_state);
        state.piles.get_mut(PileId::Hand).make_mut().push(prior);

        inject_generated_sovereign_blade(&mut state, &catalog, &mut Vec::new()).unwrap();
        let fresh = state.piles.get(PileId::Hand).as_slice()[1];
        assert_eq!(fresh.uid, 100);
        assert_eq!(
            state.card_states.get(fresh.uid).base_replay_count(),
            Some(2)
        );
        assert!(state.exact_piles);

        inject_generated_clones_bottom(
            &mut state,
            &catalog,
            fresh,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        let copied = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!(
            state.card_states.get(copied.uid).base_replay_count(),
            Some(2)
        );

        let fresh_state = state.card_states.get(fresh.uid);
        inject_generated_rewritten_clone_bottom(
            &mut state,
            &catalog,
            fresh,
            fresh.flags,
            fresh_state,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        let rewritten = state.piles.get(PileId::Discard).as_slice()[1];
        assert_eq!(
            state.card_states.get(rewritten.uid).base_replay_count(),
            Some(2)
        );
        assert_eq!(
            state.fanouts.multiplayer_ally().hand[0].sovereign_blade_damage(),
            Some(10)
        );
    }

    #[test]
    fn physical_entry_funnels_keep_one_mechanical_owner_suffix_each() {
        let production = include_str!("cards.rs")
            .split("\nmod tests {")
            .next()
            .unwrap();
        assert_eq!(
            production
                .matches("apply_physical_card_after_entered(")
                .count(),
            10,
            "one definition plus each of the nine mechanical physical-entry sites"
        );
        for helper in [
            "pub(crate) fn bulk_transform_fixed_same_pile(",
            "pub fn inject_bottom(",
            "pub fn inject_generated_bottom(",
            "pub fn inject_generated_draw_random(",
            "pub(crate) fn inject_generated_record_before_ending_draw_random(",
            "pub fn inject_generated_null_random(",
            "pub(crate) fn inject_generated_null_bottom(",
            "pub(crate) fn inject_generated_record_before_ending_bottom(",
            "pub fn inject_generated_exact_bottom(",
            "pub fn inject_generated_free_this_turn_bottom(",
            "pub fn inject_generated_clones_bottom(",
            "pub fn inject_generated_rewritten_clone_bottom(",
        ] {
            assert_eq!(
                production.matches(helper).count(),
                1,
                "{helper} census drifted"
            );
        }
        let primal_force = include_str!("../steps/ironclad_rare.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert_eq!(primal_force.matches("mint_card(").count(), 1);
        assert_eq!(
            primal_force
                .matches("apply_physical_card_after_entered(")
                .count(),
            1,
            "Primal Force's external fresh replacement must enter the common lifecycle"
        );
    }

    #[test]
    fn legacy_normalization_uses_native_all_piles_order() {
        let (mut state, catalog, identity) = fixture();
        state.next_card_uid = 70;
        // Deliberately inject in the reverse of the native traversal order.
        // Allocation order must follow the five captured piles, not insertion
        // history, and every assertion below must observe a real legacy card.
        for pile in [
            PileId::Play,
            PileId::Exhaust,
            PileId::Discard,
            PileId::Draw,
            PileId::Hand,
        ] {
            inject_legacy_bottom(&mut state, &catalog, identity, 1, pile).unwrap();
        }

        normalize_card_identities(&mut state).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 70);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 71);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 72);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 73);
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].uid, 74);
        assert_eq!(state.next_card_uid, 75);
        assert!(!state.exact_piles);
        for pile in [
            PileId::Hand,
            PileId::Draw,
            PileId::Discard,
            PileId::Exhaust,
            PileId::Play,
        ] {
            let cards = state.piles.get(pile).as_slice();
            assert_eq!(cards.len(), 1, "{pile:?} must be non-vacuous");
            assert_eq!(cards[0].flags & CARD_FLAG_LEGACY, 0, "{pile:?}");
        }
    }

    /// #3243: `CardPilePosition.Random` is a Draw-only placement; the shared
    /// transaction refuses it for any other destination before minting.
    #[test]
    fn random_generated_placement_outside_draw_refuses_before_minting() {
        let (mut state, catalog, identity) = fixture();
        let next_uid = state.next_card_uid;
        let rng = state.rng.get(RngStream::Rng).counter;
        assert!(matches!(
            inject_generated_payload_bottom(
                &mut state,
                &catalog,
                identity,
                None,
                1,
                GeneratedTransaction::gate_before_record(PileId::Hand, true)
                    .at_random_draw_position(),
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs(
                "random generated insertion outside Draw"
            ))
        ));
        assert_eq!(state.next_card_uid, next_uid);
        assert_eq!(state.rng.get(RngStream::Rng).counter, rng);
        assert_eq!(state.piles.get(PileId::Hand).len(), 0);
    }

    #[test]
    fn legacy_random_draw_insertion_spends_one_shuffle_draw_and_preflights_identity() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        let before = state.rng.get(RngStream::Rng).counter;
        let next_uid = state.next_card_uid;

        inject_legacy_draw_random(&mut state, &catalog, identity, 1).unwrap();
        assert_eq!(state.rng.get(RngStream::Rng).counter, before + 1);
        assert_eq!(
            state.piles.get(PileId::Draw).as_slice()[0].uid,
            LEGACY_CARD_UID
        );
        assert_eq!(
            state.piles.get(PileId::Draw).as_slice()[0].flags,
            CARD_FLAG_LEGACY
        );
        assert_eq!(state.next_card_uid, next_uid);

        let before = state.clone();
        assert!(matches!(
            inject_legacy_draw_random(
                &mut state,
                &catalog,
                CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None,
                },
                1,
            ),
            Err(EngineRefusal::UnknownMintIdentity(_))
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn generated_random_draw_insertion_is_one_atomic_authenticated_transaction() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.next_card_uid = 20;
        state.next_generated_hook_uid = 30;
        state.powers.set(PowerId::PhantomBlades, SlotWire::Int, 9);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        let rng_before = state.rng.get(RngStream::Rng).counter;
        let mut events = Vec::new();

        inject_generated_draw_random(&mut state, &catalog, identity, &mut events).unwrap();

        assert_eq!(state.rng.get(RngStream::Rng).counter, rng_before + 1);
        assert_eq!(state.next_card_uid, 21);
        assert_eq!(state.next_generated_hook_uid, 31);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [20, 1, 2]
        );
        let inserted = state.piles.get(PileId::Draw).as_slice()[0];
        assert_eq!(inserted.atom, atom);
        assert_eq!(
            events,
            [Event::CardResolved {
                uid: 20,
                pile: PileId::Draw
            }]
        );

        let before = state.clone();
        let events_before = events.clone();
        assert!(matches!(
            inject_generated_draw_random(
                &mut state,
                &catalog,
                CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None
                },
                &mut events,
            ),
            Err(EngineRefusal::UnknownMintIdentity(_))
        ));
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn generated_insertion_records_history_and_hand_overflow() {
        let (mut state, catalog, identity) = fixture();
        let atom = catalog.atom(&identity).unwrap();
        state.next_card_uid = 20;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..10).map(|uid| HotCard {
                uid,
                atom,
                flags: 0,
            }));
        let mut events = Vec::new();

        inject_generated_bottom(&mut state, &catalog, identity, 2, PileId::Hand, &mut events)
            .unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);
        assert_eq!(state.piles.get(PileId::Discard).len(), 2);
        assert_eq!(state.next_card_uid, 22);
    }

    #[test]
    fn generated_power_callbacks_preserve_acquisition_order() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();

        for (order, expected_strength) in [
            ([PowerId::Arsenal, PowerId::PillarOfCreation], 1),
            ([PowerId::PillarOfCreation, PowerId::Arsenal], 0),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
            state
                .powers
                .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
            state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
            assert!(state.fanouts.set_local_generated_power_order(&order));
            assert!(
                state
                    .fanouts
                    .set_after_block_gained_order(&[PowerId::Juggernaut])
            );
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));

            inject_generated_bottom(
                &mut state,
                &catalog,
                identity,
                2,
                PileId::Hand,
                &mut Vec::new(),
            )
            .unwrap();

            assert!(state.history.over);
            assert_eq!(state.powers.value(PowerId::Strength), expected_strength);
            assert_eq!(state.history.owner_generated_cards_combat, 1);
            assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        }
    }

    #[test]
    fn trash_to_treasure_channels_one_random_orb_per_live_stack() {
        let status = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let ordinary = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(status).unwrap();
        builder.intern(ordinary).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        // Channel checks IsEnding, which is trivially true for a
        // monsterless state; Trash to Treasure triggers in live combat.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.powers.set(PowerId::TrashToTreasure, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::TrashToTreasure])
        );
        let rng_before = state.rng.get(RngStream::CombatOrbs).counter;

        inject_generated_bottom(
            &mut state,
            &catalog,
            status,
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.orbs.as_slice().len(), 2);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, rng_before + 2);
        assert_eq!(state.orbs.next_random_orb_progress_uid(), 2);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.next_generated_hook_uid, 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);

        let before = state.clone();
        inject_generated_bottom(
            &mut state,
            &catalog,
            ordinary,
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.orbs, before.orbs, "non-Status generation is inert");

        let mut null_state = before;
        let null_before = null_state.clone();
        inject_generated_null_bottom(&mut null_state, &catalog, status, 1, &mut Vec::new())
            .unwrap();
        assert_eq!(
            null_state.orbs, null_before.orbs,
            "null-creator Status generation is not owner-created"
        );
    }

    #[test]
    fn smokestack_uses_exact_status_type_and_frozen_roster_order() {
        let status = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let curse = CardIdentity {
            id: CardId::AscendersBane,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(status).unwrap();
        builder.intern(curse).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        assert!(
            catalog
                .spec(catalog.atom(&status).unwrap())
                .unwrap()
                .is_status
        );
        assert!(
            !catalog
                .spec(catalog.atom(&curse).unwrap())
                .unwrap()
                .is_status
        );

        let mut state = HotState::at_defaults();
        // A living owner: Smokestack's single `CreatureCmd.Damage` batch
        // (#3172) is dealt by the power's owner.
        state.hp = 50;
        state.max_hp = 50;
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        state.monsters_mut().extend([
            HotMonster::new(MonsterKind::Toadpole, 7),
            HotMonster::new(MonsterKind::Toadpole, 4),
        ]);

        let before_curse = state.clone();
        inject_generated_bottom(
            &mut state,
            &catalog,
            curse,
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters, before_curse.monsters);

        inject_generated_bottom(
            &mut state,
            &catalog,
            status,
            1,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 2);
        assert_eq!(state.monsters[1].hp, -1);
    }

    #[test]
    fn generated_power_batch_overflow_refuses_before_generation_mutates() {
        let (mut state, catalog, identity) = fixture();
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        state.powers.set(PowerId::PhantomBlades, SlotWire::Int, 9);
        state.powers.set(PowerId::Strength, SlotWire::Int, i32::MAX);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal])
        );
        let before = state.clone();

        assert_eq!(
            inject_generated_bottom(
                &mut state,
                &catalog,
                identity,
                1,
                PileId::Hand,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::CounterOverflow("generated power batch"))
        );
        assert_eq!(state, before);

        let (mut state, catalog, identity) = fixture();
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        state.block = 999_999_999;
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        let before = state.clone();
        assert_eq!(
            inject_generated_bottom(
                &mut state,
                &catalog,
                identity,
                1,
                PileId::Hand,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::CounterOverflow("generated power batch"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn record_before_add_clone_batch_preflights_both_history_counters() {
        let (mut state, catalog, identity) = fixture();
        let source = HotCard {
            uid: 7,
            atom: catalog.atom(&identity).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.history.over = true;
        let mut events = vec![Event::CombatOver { player_won: true }];

        for overflow_owner_history in [true, false] {
            state.history.owner_generated_cards_combat = if overflow_owner_history {
                i32::MAX - 1
            } else {
                0
            };
            state.next_generated_hook_uid = if overflow_owner_history {
                0
            } else {
                i32::MAX - 1
            };
            let before = state.clone();
            let events_before = events.clone();
            let expected = if overflow_owner_history {
                "owner_generated_cards_combat"
            } else {
                "next_generated_hook_uid"
            };

            assert_eq!(
                inject_generated_clones_bottom(
                    &mut state,
                    &catalog,
                    source,
                    2,
                    PileId::Discard,
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow(expected))
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);
        }
    }

    #[test]
    fn record_before_add_on_ended_combat_records_only_history_epochs() {
        let (mut state, catalog, identity) = fixture();
        let source = HotCard {
            uid: 7,
            atom: catalog.atom(&identity).unwrap(),
            flags: 0,
        };
        state.history.over = true;
        state.history.owner_generated_cards_combat = 4;
        state.next_generated_hook_uid = 8;
        state.next_card_uid = 41;
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 3);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal, PowerId::PillarOfCreation])
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let powers_before = state.powers.clone();
        let monsters_before = state.monsters.clone();
        let mut events = vec![Event::CombatOver { player_won: true }];

        inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            2,
            PileId::Discard,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 6);
        assert_eq!(state.next_generated_hook_uid, 10);
        assert_eq!(state.next_card_uid, 41);
        assert_eq!(state.powers, powers_before);
        assert_eq!(state.monsters, monsters_before);
        assert_eq!(state.block, 0);
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert!(!state.exact_piles);
        assert_eq!(events, [Event::CombatOver { player_won: true }]);
    }

    #[test]
    fn call_of_the_void_batch_records_terminal_tail_and_rolls_back_preflight_overflow() {
        let identity = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let identities = [identity, identity];
        let mut state = HotState::at_defaults();
        state.hp = 50;
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

        inject_call_of_the_void_batch_bottom(&mut state, &catalog, &identities, &mut Vec::new())
            .unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        let entered = state.piles.get(PileId::Hand).as_slice()[0];
        assert!(state.card_states.get(entered.uid).local_ethereal());

        let mut overflow = HotState::at_defaults();
        overflow.history.owner_generated_cards_combat = i32::MAX - 1;
        let before = overflow.clone();
        let mut events = vec![Event::CombatOver { player_won: true }];
        let events_before = events.clone();
        assert_eq!(
            inject_call_of_the_void_batch_bottom(&mut overflow, &catalog, &identities, &mut events,),
            Err(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat"
            ))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn fixed_status_generation_records_every_ending_command_without_insertion() {
        let identity = CardIdentity {
            id: CardId::Debris,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.history.over = true;
        state.next_card_uid = 41;
        let mut events = Vec::new();

        inject_generated_record_before_ending_bottom(
            &mut state,
            &catalog,
            identity,
            3,
            PileId::Hand,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
        assert_eq!(state.next_card_uid, 41);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn fixed_status_generation_keeps_recording_after_a_listener_ends_combat() {
        let identity = CardIdentity {
            id: CardId::Debris,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // A living owner: Smokestack's single `CreatureCmd.Damage` batch
        // (#3172) is dealt by the power's owner.
        state.hp = 50;
        state.max_hp = 50;
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.next_card_uid = 50;

        inject_generated_record_before_ending_bottom(
            &mut state,
            &catalog,
            identity,
            3,
            PileId::Hand,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
        assert_eq!(state.next_card_uid, 51);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    #[test]
    fn fixed_status_generation_preflights_late_counter_overflow_atomically() {
        let identity = CardIdentity {
            id: CardId::Debris,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.history.owner_generated_cards_combat = i32::MAX - 1;
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 9,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();

        assert_eq!(
            inject_generated_record_before_ending_bottom(
                &mut state,
                &catalog,
                identity,
                2,
                PileId::Hand,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "owner_generated_cards_combat"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn null_creator_random_generation_skips_history_and_every_owner_gated_listener() {
        let identity = CardIdentity {
            id: CardId::FranticEscape,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // A living owner: Smokestack's single `CreatureCmd.Damage` batch
        // (#3172) is dealt by the power's owner.
        state.hp = 50;
        state.max_hp = 50;
        state.next_card_uid = 20;
        state.next_generated_hook_uid = 30;
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 2);
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 3);
        assert!(state.fanouts.set_local_generated_power_order(&[
            PowerId::Arsenal,
            PowerId::PillarOfCreation,
            PowerId::Smokestack,
        ]));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        let rng_before = state.rng.get(RngStream::Rng).counter;
        let mut events = Vec::new();

        inject_generated_null_random(&mut state, &catalog, identity, PileId::Discard, &mut events)
            .unwrap();

        assert_eq!(state.history.owner_generated_cards_combat, 0);
        assert_eq!(state.next_card_uid, 21);
        assert_eq!(state.next_generated_hook_uid, 31);
        assert_eq!(state.rng.get(RngStream::Rng).counter, rng_before + 1);
        // #3256: Arsenal, Pillar of Creation and Smokestack each leave on a
        // null `creator` (RVAs 0x334ec8 / 0x340488 / 0x3455e8), so the
        // physical transaction publishes and no listener answers.
        assert_eq!(state.powers.value(PowerId::Strength), 0);
        assert_eq!(state.block, 0);
        assert_eq!(state.monsters[0].hp, 10);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 20);
        assert!(!state.card_states.get(20).local_retain);
        assert!(!state.exact_piles, "Frantic Escape is not a Shiv");
        assert!(matches!(
            events.first(),
            Some(Event::CardResolved {
                uid: 20,
                pile: PileId::Discard
            })
        ));
    }

    /// #3256: every owner-gated local generated listener answers an owner
    /// creator and ignores a null one, card for card.
    /// `ArsenalPower` `0x334ec8` IL_001d-IL_0038, `PillarOfCreationPower`
    /// `0x340488` IL_001d-IL_0038, `SmokestackPower` `0x3455e8`
    /// IL_0033-IL_004e and `Regalite` `0x32f988` IL_0020-IL_0036.
    #[test]
    fn owner_gated_generated_listeners_split_on_the_creator() {
        let wound = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(wound).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicRegalite])
            .unwrap();
        let catalog = builder.build();
        let mut base = HotState::at_defaults();
        base.hp = 50;
        base.max_hp = 50;
        base.next_card_uid = 20;
        base.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        base.powers.set(PowerId::PillarOfCreation, SlotWire::Int, 2);
        base.powers.set(PowerId::Smokestack, SlotWire::Int, 3);
        assert!(base.fanouts.set_local_generated_power_order(&[
            PowerId::Arsenal,
            PowerId::PillarOfCreation,
            PowerId::Smokestack,
        ]));
        base.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));

        // Owner creator: Arsenal +1 Strength, Pillar +2 Block, Smokestack 3
        // damage, then Regalite's once-per-turn 6 Block.
        let mut owner = base.clone();
        inject_generated_bottom(
            &mut owner,
            &catalog,
            wound,
            1,
            PileId::Discard,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(owner.powers.value(PowerId::Strength), 1);
        assert_eq!(owner.block, 2 + i32::from(owner.regalite_block_amount));
        assert_eq!(owner.monsters[0].hp, 7);
        assert!(owner.fanouts.regalite_used_this_turn());

        // Null creator (the same Wound, Test Subject's bottom insertion):
        // the card enters and nothing answers, Regalite included.
        let mut null = base.clone();
        inject_generated_null_bottom(&mut null, &catalog, wound, 1, &mut Vec::new()).unwrap();
        assert_eq!(null.piles.get(PileId::Discard).len(), 1);
        assert_eq!(null.powers.value(PowerId::Strength), 0);
        assert_eq!(null.block, 0);
        assert_eq!(null.monsters[0].hp, 10);
        assert!(!null.fanouts.regalite_used_this_turn());
    }

    #[test]
    fn null_creator_bottom_rehearses_a_later_listener_failure_atomically() {
        let identity = CardIdentity {
            id: CardId::Burn,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 20;
        state.next_generated_hook_uid = 30;
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        state
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX - 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal])
        );
        let mut events = vec![Event::CardResolved {
            uid: 9,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();

        // #3256: a null creator never reaches Arsenal, so a Strength one
        // short of overflow is no refusal and is left untouched.
        let mut quiet = state.clone();
        inject_generated_null_bottom(&mut quiet, &catalog, identity, 2, &mut Vec::new()).unwrap();
        assert_eq!(quiet.powers.value(PowerId::Strength), i32::MAX - 1);
        assert_eq!(quiet.piles.get(PileId::Discard).len(), 2);

        // The second command's uid allocation overflows; the whole batch
        // (the first card, its hook epoch and its event) stays unpublished.
        state.next_card_uid = u32::MAX - 1;
        let before = state.clone();
        assert_eq!(
            inject_generated_null_bottom(&mut state, &catalog, identity, 2, &mut events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn hidden_daggers_upgrades_only_both_returned_shivs_across_hand_overflow() {
        let hidden = CardIdentity {
            id: CardId::HiddenDaggers,
            upgrade: 1,
            enchantment: None,
        };
        let filler = CardIdentity {
            id: CardId::DefendSilent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(hidden).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 20;
        for uid in 1..=9 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            });
        }
        inject_generated_shivs_then_upgrade(&mut state, &catalog, 2, 1, &mut Vec::new()).unwrap();
        let upgraded = CardIdentity {
            id: CardId::Shiv,
            upgrade: 1,
            enchantment: None,
        };
        let hand_tail = state.piles.get(PileId::Hand).as_slice()[9];
        let discard_tail = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!((hand_tail.uid, discard_tail.uid), (20, 21));
        assert_eq!(catalog.spec(hand_tail.atom).unwrap().identity, upgraded);
        assert_eq!(catalog.spec(discard_tail.atom).unwrap().identity, upgraded);
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);

        let mut refusing = HotState::at_defaults();
        refusing.hp = 50;
        refusing.next_card_uid = 20;
        refusing.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
        refusing
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX - 1);
        assert!(
            refusing
                .fanouts
                .set_local_generated_power_order(&[PowerId::Arsenal])
        );
        let before = refusing.clone();
        let mut events = Vec::new();
        assert_eq!(
            inject_generated_shivs_then_upgrade(&mut refusing, &catalog, 2, 1, &mut events),
            Err(EngineRefusal::CounterOverflow("generated power batch"))
        );
        assert_eq!(refusing, before);
        assert!(events.is_empty());
    }

    #[test]
    fn hidden_daggers_refuses_a_same_pile_duplicate_returned_uid_atomically() {
        let hidden = CardIdentity {
            id: CardId::HiddenDaggers,
            upgrade: 1,
            enchantment: None,
        };
        let shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(hidden).unwrap();
        let shiv_atom = builder.intern(shiv).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_card_uid = 20;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 20,
            atom: shiv_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 9,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();

        assert_eq!(
            inject_generated_shivs_then_upgrade(&mut state, &catalog, 2, 1, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    fn self_return_fixture() -> (HotState, Catalog, CardAtom, CardAtom, CardAtom) {
        let bolas = CardIdentity {
            id: CardId::Bolas,
            upgrade: 0,
            enchantment: None,
        };
        let hatchet = CardIdentity {
            id: CardId::ThrummingHatchet,
            upgrade: 0,
            enchantment: None,
        };
        let filler = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let bolas_atom = builder.intern(bolas).unwrap();
        let hatchet_atom = builder.intern(hatchet).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.next_card_uid = 20;
        (
            state,
            builder.build(),
            bolas_atom,
            hatchet_atom,
            filler_atom,
        )
    }

    #[test]
    fn batch139_finished_uids_roll_freeze_and_return_in_native_pile_order() {
        let (mut state, catalog, bolas, hatchet, _) = self_return_fixture();
        let first = HotCard {
            uid: 4,
            atom: bolas,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let equal_sibling = HotCard { uid: 5, ..first };
        let second = HotCard {
            uid: 6,
            atom: hatchet,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(equal_sibling);
        state.piles.get_mut(PileId::Discard).make_mut().push(first);
        state.piles.get_mut(PileId::Exhaust).make_mut().push(second);

        record_self_return_finished(&mut state, catalog.spec(bolas).unwrap(), first.uid).unwrap();
        record_self_return_finished(&mut state, catalog.spec(hatchet).unwrap(), second.uid)
            .unwrap();
        // Native Any-style de-duplication records a replayed body only once.
        record_self_return_finished(&mut state, catalog.spec(bolas).unwrap(), first.uid).unwrap();
        assert_eq!(
            state.card_states.self_return().unwrap().current_turn,
            [first.uid, second.uid]
        );
        rollover_self_return_turn(&mut state);
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        assert_eq!(
            state.card_states.self_return().unwrap().before_hand_draw,
            [equal_sibling.uid, first.uid, second.uid]
        );

        let mut events = Vec::new();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut events).unwrap();
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [first.uid, second.uid]
        );
        assert_eq!(state.piles.get(PileId::Draw).as_slice(), &[equal_sibling]);
        assert_eq!(
            events,
            [
                Event::CardResolved {
                    uid: first.uid,
                    pile: PileId::Hand,
                },
                Event::CardResolved {
                    uid: second.uid,
                    pile: PileId::Hand,
                },
            ]
        );
        assert!(
            state
                .card_states
                .self_return()
                .unwrap()
                .before_hand_draw
                .is_empty()
        );
    }

    #[test]
    fn batch139_already_hand_missing_ending_and_full_hand_match_add_contract() {
        let (mut state, catalog, bolas, _, filler) = self_return_fixture();
        let returning = HotCard {
            uid: 4,
            atom: bolas,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.piles.get_mut(PileId::Hand).make_mut().push(returning);
        state
            .card_states
            .set_self_return(crate::hot::SelfReturnState {
                previous_turn: vec![returning.uid, 7],
                ..crate::hot::SelfReturnState::default()
            });
        state.exact_piles = true;
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[returning]);

        state.piles.get_mut(PileId::Hand).make_mut().clear();
        for uid in 8..18 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: filler,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            returning,
            HotCard {
                uid: 18,
                atom: filler,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [18, returning.uid]
        );

        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        let before = state.piles.clone();
        state.history.over = true;
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.piles, before);
    }

    #[test]
    fn batch139_frozen_missing_skips_and_wrong_family_refuses_atomically() {
        let (mut state, catalog, bolas, _, filler) = self_return_fixture();
        let returning = HotCard {
            uid: 4,
            atom: bolas,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(returning);
        state
            .card_states
            .set_self_return(crate::hot::SelfReturnState {
                previous_turn: vec![returning.uid],
                ..crate::hot::SelfReturnState::default()
            });
        state.exact_piles = true;
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        state.piles.get_mut(PileId::Discard).make_mut().clear();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert!(
            state
                .card_states
                .self_return()
                .unwrap()
                .before_hand_draw
                .is_empty()
        );

        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(returning);
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        state.piles.get_mut(PileId::Discard).make_mut()[0].atom = filler;
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();
        assert_eq!(
            complete_self_return_before_hand_draw(&mut state, &catalog, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "Batch139 BeforeHandDraw listener state"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn batch139_listener_identity_survives_an_exact_body_enchantment() {
        let identity = CardIdentity {
            id: CardId::Bolas,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 2;
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        record_self_return_finished(&mut state, catalog.spec(atom).unwrap(), 1).unwrap();
        rollover_self_return_turn(&mut state);
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 1);
    }

    #[test]
    fn batch139_history_allows_a_completed_uid_to_transform_before_either_rollover() {
        let (mut state, catalog, bolas, _, filler) = self_return_fixture();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 4,
                atom: bolas,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        record_self_return_finished(&mut state, catalog.spec(bolas).unwrap(), 4).unwrap();
        state.piles.get_mut(PileId::Discard).make_mut()[0].atom = filler;
        assert!(self_return_state_is_exact(&state, &catalog));
        rollover_self_return_turn(&mut state);
        assert!(self_return_state_is_exact(&state, &catalog));
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        assert!(
            state
                .card_states
                .self_return()
                .unwrap()
                .before_hand_draw
                .is_empty()
        );
    }

    #[test]
    fn batch139_nested_cow_detaches_on_roll_freeze_and_clear() {
        let (mut state, catalog, bolas, _, _) = self_return_fixture();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 4,
                atom: bolas,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        record_self_return_finished(&mut state, catalog.spec(bolas).unwrap(), 4).unwrap();
        let sibling = state.clone();
        assert!(state.card_states.shares_store_with(&sibling.card_states));
        assert!(
            state
                .card_states
                .self_return_shares_store_with(&sibling.card_states)
        );

        rollover_self_return_turn(&mut state);
        assert!(!state.card_states.shares_store_with(&sibling.card_states));
        assert!(
            !state
                .card_states
                .self_return_shares_store_with(&sibling.card_states)
        );
        assert_eq!(sibling.card_states.self_return().unwrap().current_turn, [4]);
        assert!(
            sibling
                .card_states
                .self_return()
                .unwrap()
                .previous_turn
                .is_empty()
        );
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_ne!(state.card_states, sibling.card_states);
    }

    #[test]
    fn batch139_card_minted_by_an_earlier_before_hand_draw_power_waits_for_next_snapshot() {
        let (mut state, catalog, bolas, _, _) = self_return_fixture();
        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 4,
            atom: bolas,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        complete_self_return_before_hand_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 4);
        assert!(state.card_states.self_return().is_none());

        freeze_self_return_before_hand_draw(&mut state, &catalog).unwrap();
        assert_eq!(
            state.card_states.self_return().unwrap().before_hand_draw,
            [4]
        );
    }

    #[test]
    fn ghost_seed_marks_only_the_ten_basic_strike_and_defend_identities() {
        let mut builder = CatalogBuilder::new();
        let strike = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let bash = CardIdentity {
            id: CardId::Bash,
            upgrade: 0,
            enchantment: None,
        };
        let strike_atom = builder.intern(strike).unwrap();
        let bash_atom = builder.intern(bash).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicGhostSeed])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let mut starter = HotCard {
            uid: 1,
            atom: strike_atom,
            flags: 0,
        };
        apply_physical_card_after_entered_suffix(
            &mut state,
            &catalog,
            catalog.spec(strike_atom).unwrap(),
            &mut starter,
            false,
        )
        .unwrap();
        assert!(state.card_states.get(1).local_ethereal());
        assert_eq!(starter.flags, 0, "local Ethereal does not invent slot 7");

        let mut ordinary = HotCard {
            uid: 2,
            atom: bash_atom,
            flags: 0,
        };
        apply_physical_card_after_entered_suffix(
            &mut state,
            &catalog,
            catalog.spec(bash_atom).unwrap(),
            &mut ordinary,
            false,
        )
        .unwrap();
        assert!(!state.card_states.get(2).local_ethereal());
    }

    /// Every branch of the room-entry pass (#2827): an unowned relic changes
    /// nothing; an owned one marks each Basic Strike/Defend in every pile,
    /// leaves a card that is already Ethereal alone, never marks a non-basic,
    /// and does not promote `exact_piles`.
    #[test]
    fn ghost_seed_room_entry_marks_every_pile_and_skips_already_ethereal_cards() {
        let identity = |id| CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        let build = |relics: &[crate::ids::RelicId]| {
            let mut builder = CatalogBuilder::new();
            let atoms = [
                builder.intern(identity(CardId::StrikeIronclad)).unwrap(),
                builder.intern(identity(CardId::DefendSilent)).unwrap(),
                builder.intern(identity(CardId::Bash)).unwrap(),
            ];
            builder.set_relics(relics).unwrap();
            (builder.build(), atoms)
        };
        let state_for = |atoms: [CardAtom; 3]| {
            let mut state = HotState::at_defaults();
            let card = |uid, atom| HotCard {
                uid,
                atom,
                flags: 0,
            };
            state.piles.set(
                PileId::Draw,
                crate::hot::HotPile::from_cards(vec![card(1, atoms[0]), card(2, atoms[2])]),
            );
            state.piles.set(
                PileId::Discard,
                crate::hot::HotPile::from_cards(vec![card(3, atoms[1]), card(4, atoms[0])]),
            );
            // uid 4 is already Ethereal, so `CanAffect` rejects it.
            let mut already = state.card_states.get(4);
            already.set_local_ethereal(true);
            state.card_states.set(4, already);
            state
        };

        let (unowned, atoms) = build(&[]);
        let mut state = state_for(atoms);
        ghost_seed_after_room_entered(&mut state, &unowned);
        assert_eq!(
            (1..=3)
                .map(|uid| state.card_states.get(uid).local_ethereal())
                .collect::<Vec<_>>(),
            [false, false, false]
        );

        let (owned, atoms) = build(&[crate::ids::RelicId::RelicGhostSeed]);
        let mut state = state_for(atoms);
        let before = state.card_states.get(4);
        ghost_seed_after_room_entered(&mut state, &owned);
        assert!(state.card_states.get(1).local_ethereal(), "draw Strike");
        assert!(!state.card_states.get(2).local_ethereal(), "Bash");
        assert!(state.card_states.get(3).local_ethereal(), "discard Defend");
        assert_eq!(state.card_states.get(4), before, "already Ethereal");
        assert!(!state.exact_piles);
    }

    /// #3551: an owned Music Box authenticates a local Ethereal on any unique
    /// live Attack, at any upgrade; a marked non-Attack, a duplicated marked
    /// card, and a marked Attack without the relic do not pass.
    #[test]
    fn music_box_is_a_local_ethereal_writer_for_live_attacks_only() {
        let build = |relics: &[crate::ids::RelicId]| {
            let mut builder = CatalogBuilder::new();
            let attack = builder
                .intern(CardIdentity {
                    id: CardId::Bash,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
            let skill = builder
                .intern(CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.set_relics(relics).unwrap();
            (builder.build(), attack, skill)
        };
        let marked_hand = |atom| {
            let mut state = HotState::at_defaults();
            state.piles.set(
                PileId::Hand,
                crate::hot::HotPile::from_cards(vec![HotCard {
                    uid: 2,
                    atom,
                    flags: 0,
                }]),
            );
            let mut marked = state.card_states.get(2);
            marked.set_local_ethereal(true);
            state.card_states.set(2, marked);
            state
        };
        let (owned, attack, skill) = build(&[crate::ids::RelicId::RelicMusicBox]);
        let state = marked_hand(attack);
        assert!(call_local_ethereal_provenance_is_exact(&state, &owned));
        assert!(
            !call_local_ethereal_provenance_is_exact(&marked_hand(skill), &owned),
            "Music Box only copies Attacks"
        );
        let mut duplicated = state.clone();
        duplicated.piles.set(
            PileId::Discard,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 2,
                atom: attack,
                flags: 0,
            }]),
        );
        assert!(
            !call_local_ethereal_provenance_is_exact(&duplicated, &owned),
            "the marked card must be unique and live"
        );
        let (unowned, attack, _) = build(&[]);
        assert!(
            !call_local_ethereal_provenance_is_exact(&marked_hand(attack), &unowned),
            "no writer without the relic"
        );
    }

    /// #3022: a live Sculpting Strike (L0 or L1, any pile) authenticates a
    /// local Ethereal on any unique live card; a Sculpting Strike that is only
    /// a catalog member, or a duplicated marked card, does not.
    #[test]
    fn sculpting_strike_is_a_local_ethereal_writer_only_while_live() {
        let identity = |id, upgrade| CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            let sculpting = builder
                .intern(identity(CardId::SculptingStrike, upgrade))
                .unwrap();
            let bash = builder.intern(identity(CardId::Bash, 0)).unwrap();
            let catalog = builder.build();
            let card = |uid, atom| HotCard {
                uid,
                atom,
                flags: 0,
            };
            let mut state = HotState::at_defaults();
            state.piles.set(
                PileId::Hand,
                crate::hot::HotPile::from_cards(vec![card(2, bash)]),
            );
            let mut marked = state.card_states.get(2);
            marked.set_local_ethereal(true);
            state.card_states.set(2, marked);
            assert!(
                !call_local_ethereal_provenance_is_exact(&state, &catalog),
                "a catalog-only Sculpting Strike is not a writer"
            );

            state.piles.set(
                PileId::Exhaust,
                crate::hot::HotPile::from_cards(vec![card(1, sculpting)]),
            );
            assert!(call_local_ethereal_provenance_is_exact(&state, &catalog));

            let mut duplicated = state.clone();
            duplicated.piles.set(
                PileId::Discard,
                crate::hot::HotPile::from_cards(vec![card(2, bash)]),
            );
            assert!(
                !call_local_ethereal_provenance_is_exact(&duplicated, &catalog),
                "the marked card must be unique and live"
            );
        }
    }
}
