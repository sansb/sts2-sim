//! Monster-move bodies for monsters no encounter builder constructs directly — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # R2 status (#1560/#1751): 4 of 4 ported
//!
//! R2's build-first contract ports exact move bodies independently of a
//! carrier's remaining random-AI or spawn closure. Tender and Constrict's
//! complete shared listeners and random-AI gates landed in #1751.

use super::MoveCtx;
use crate::catalog::CompiledArg;
use crate::content_tables::Repeats;
use crate::content_tables::move_constants as mc;
use crate::engine::damage::{
    apply_player_duration_affliction, monster_attack_player_with_catalog, note_power,
};
use crate::engine::{EngineRefusal, Subject};
use crate::hot::{HotState, MonsterFollowUp, MonsterOverride};
use crate::ids::{MonsterKind, MoveKind, PowerId};
use crate::powers::SlotWire;

/// The MoveKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[MoveKind] = &[
    MoveKind::AttackVulnWeakPlayer,
    MoveKind::BuffTeamStrength,
    MoveKind::ConstrictPlayer,
    MoveKind::TenderGoop,
];

/// Every kind owned by this family, ascending. Tests below derive the carrier
/// census from the generated random-move tables and pin every body to its own
/// typed refusal, so this escalation record cannot silently lose a new stub.
#[cfg(test)]
const OWNED: [MoveKind; 4] = [
    MoveKind::AttackVulnWeakPlayer,
    MoveKind::BuffTeamStrength,
    MoveKind::ConstrictPlayer,
    MoveKind::TenderGoop,
];

/// `("attack_vuln_weak_player", damage, hits, vuln, weak)`.
///
/// Python: `monster_act` (frozen, deleted #2827) and the resumed suffix
/// `_finish_monster_move_after_attack`. Soul Nexus attacks, then while
/// combat and owner remain live applies enemy-fresh Vulnerable followed by
/// enemy-fresh Weak. Despite the move name, it does not heal the owner.
///
/// The attack is followed by fresh Vulnerable then fresh Weak while combat
/// and owner remain live. Soul Nexus's random-AI machine remains a separate
/// entry refusal; it does not alter this branch's exact command semantics.
pub(crate) fn attack_vuln_weak_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(vuln),
        CompiledArg::I(weak),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_vuln_weak_player"));
    };
    if *damage < 0 || *hits < 1 || *vuln <= 0 || *weak <= 0 {
        return Err(EngineRefusal::MalformedArgs("attack_vuln_weak_player"));
    }
    // `SoulNexus::get_DrainLifeDamage` RVA `0xbf2a7`, (9, 19, 18) (#2828).
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::SoulNexus
        || (*damage, *hits, *vuln, *weak) != (ctx.tier(mc::SOUL_NEXUS_DRAIN_LIFE_DAMAGE), 1, 2, 2)
    {
        return Err(EngineRefusal::MalformedArgs(
            "attack_vuln_weak_player owner/row",
        ));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        for (power, fresh, raw, site) in [
            (
                PowerId::PlayerVuln,
                PowerId::PlayerVulnFresh,
                *vuln,
                "attack_vuln_weak_player Vulnerable",
            ),
            (
                PowerId::PlayerWeak,
                PowerId::PlayerWeakFresh,
                *weak,
                "attack_vuln_weak_player Weak",
            ),
        ] {
            apply_player_duration_affliction(
                ctx.state,
                power,
                fresh,
                raw.try_into()
                    .map_err(|_| EngineRefusal::CounterOverflow(site))?,
                true,
                ctx.events,
            )?;
        }
    }
    Ok(())
}

/// `("buff_team_strength", amount)`.
///
/// Python: `monster_act` (frozen, deleted #2827). The Obscura applies Strength to every living
/// same-side creature, including itself, in roster order.
///
/// Snapshot every living same-side uid in roster order and serially apply
/// Strength, re-resolving liveness at every command boundary. Obscura's AI and
/// Illusion lifecycle remain separate admission refusals.
pub(crate) fn buff_team_strength(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(raw)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("buff_team_strength"));
    };
    let amount: i32 = (*raw)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("buff_team_strength Strength"))?;
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("buff_team_strength"));
    }
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::TheObscura || amount != 3 {
        return Err(EngineRefusal::MalformedArgs("buff_team_strength owner/row"));
    }
    let actor_uid = ctx.state.monsters[ctx.actor].uid;
    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let recipients: Vec<(u32, i32)> = ctx
        .state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0)
        .map(|monster| {
            crate::engine::damage::checked_monster_strength_successor(
                monster,
                amount,
                "buff_team_strength Strength",
            )
            .map(|updated| (monster.uid, updated))
        })
        .collect::<Result<_, _>>()?;
    for (uid, updated) in recipients {
        if ctx.state.history.over {
            break;
        }
        let Some(index) = ctx
            .state
            .monsters
            .iter()
            .position(|monster| monster.uid == uid && monster.hp > 0)
        else {
            continue;
        };
        // A team buff: the applier is the ACTING monster, as
        // `<DebilitatingSmogMove>d__13::MoveNext` `0x3703a4` IL_00ab-IL_00c5
        // passes its own `Creature` for each of its targets.
        crate::engine::damage::write_monster_strength(
            &mut ctx.state.monsters_mut()[index],
            updated,
            crate::hot::Applier::Monster(actor_uid),
            upkeep,
        );
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Strength,
            updated,
        );
    }
    Ok(())
}

/// `("constrict_player", 3)`.
///
/// Current v0.111.0 ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `SlitheringStrangler/<ConstrictMove>d__15::MoveNext` RVA `0x36a1bc`
/// applies `ConstrictPower(3)` with the acting Strangler as exact applier;
/// `ConstrictPower/<AfterSideTurnEnd>d__4::MoveNext` RVA `0x3374ac`
/// owner-gates then awaits blockable, unpowered Damage props `4`; and
/// `ConstrictPower/<AfterDeath>d__5::MoveNext` RVA `0x3373cc` removes the
/// power only when the dead creature is that retained applier. Python
/// `monster_act` (frozen, deleted #2827), `_constrict_player_side_end`, and
/// `_finish_monster_death` preserve the same stack, damage, and
/// exact-applier lifecycle.
///
/// Public admission authenticates the exact fixed opener before first
/// application, every active carrier/owner/AI shape afterwards, and all live
/// or catalog-reachable noncommuting listener peers. The turn driver owns a
/// separate first-acquisition transaction so late AI-log/turn overflow cannot
/// publish the move, RNG, or event prefix.
pub(crate) fn constrict_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    constrict_player_exact(ctx)
}

/// Whether malformed or live private Constrict state can reach a shared
/// player-turn/death seam. The player-side amount makes inactive reachability
/// O(1); only the active path scans the roster to authenticate its applier.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConstrictCarrierState {
    Absent,
    Reachable,
    Malformed,
}

/// Classify the player-side amount without touching the monster roster.
#[inline]
pub(crate) fn constrict_carrier_state(state: &HotState) -> ConstrictCarrierState {
    match state.fanouts.constrict_amount() {
        0 => ConstrictCarrierState::Absent,
        amount if amount > 0 && amount % 3 == 0 => ConstrictCarrierState::Reachable,
        _ => ConstrictCarrierState::Malformed,
    }
}

pub(crate) fn constrict_listener_is_reachable(state: &HotState) -> bool {
    constrict_carrier_state(state) == ConstrictCarrierState::Reachable
}

/// Whether the amount itself is outside the native positive three-aligned
/// domain. Active orphan/duplicate/alien-source shapes are authenticated by
/// the cold owner validator instead of being hidden as absence.
pub(crate) fn constrict_alias_is_malformed(state: &HotState) -> bool {
    constrict_carrier_state(state) == ConstrictCarrierState::Malformed
}

fn constrict_owner_is_exact(state: &HotState, allow_dying_uid: Option<u32>) -> bool {
    let mut owners = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::SlitheringStrangler);
    let Some(owner) = owners.next() else {
        return false;
    };
    if owners.next().is_some()
        || state
            .monsters
            .iter()
            .filter(|monster| monster.uid == owner.uid)
            .count()
            != 1
        || !crate::engine::monsters::native_hp_in_band(
            state,
            MonsterKind::SlitheringStrangler,
            owner.max_hp,
        )
        || owner.hp > owner.max_hp
        || owner.block < 0
        || owner.override_state != MonsterOverride::None
        || owner.forced_follow_up != MonsterFollowUp::None
        || owner.spawn_noop
        || owner.revive_stage != 0
        || owner.pressure_gun_damage != 0
        || owner.pressure_buildup_idx() != 0
        || owner.is_about_to_blow()
        || !crate::engine::turn::slithering_strangler_ai_state_is_exact(owner)
    {
        return false;
    }
    owner.hp > 0
        || state.fanouts.monster_death_is_pending(owner.uid)
        || (allow_dying_uid == Some(owner.uid) && owner.hp <= 0)
}

pub(crate) fn constrict_peer_state_is_exact(state: &HotState) -> bool {
    crate::engine::damage::player_type_one_listener_order_is_exact(state)
}

/// Bind an absent Constrict carrier to native's sole pre-application state:
/// one live exact Strangler on the generated fixed CONSTRICT opener. An absent
/// carrier beside a THWACK/LASH history is forged canonical state, because
/// either attack is reachable only after CONSTRICT installed the singleton.
pub(crate) fn constrict_absent_strangler_entry_is_exact(state: &HotState) -> bool {
    if constrict_carrier_state(state) != ConstrictCarrierState::Absent
        || !constrict_owner_is_exact(state, None)
        || !constrict_peer_state_is_exact(state)
    {
        return false;
    }
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.kind == MonsterKind::SlitheringStrangler)
        .expect("validated one Strangler");
    owner.random_ai.next() == Some(crate::engine::turn::STRANGLER_CONSTRICT_INDEX)
        && owner.random_ai.log_len() == 1
        && owner.random_ai.log_at(0) == Some(crate::engine::turn::STRANGLER_CONSTRICT_INDEX)
}

/// Authenticate the one inert historical row left after the exact applier's
/// death callback has removed Constrict. It is not a future acquisition, but
/// retaining it must not make duplicate or malformed dead rows admissible.
/// Deliberately do not apply the live/prospective peer wall here: after the
/// sole applier dies, later powers and surviving monsters cannot reacquire or
/// reorder the removed Constrict suffix.
pub(crate) fn constrict_absent_dead_strangler_history_is_exact(state: &HotState) -> bool {
    if constrict_carrier_state(state) != ConstrictCarrierState::Absent {
        return false;
    }
    let Some(owner) = state
        .monsters
        .iter()
        .find(|monster| monster.kind == MonsterKind::SlitheringStrangler)
    else {
        return false;
    };
    owner.hp <= 0 && constrict_owner_is_exact(state, Some(owner.uid))
}

/// Authenticate the live private singleton, including its exact applier,
/// positive Intensity stack, random-AI command boundary, and the independent
/// after-power-amount-changed listener order used by its damage body.
pub(crate) fn constrict_private_state_is_exact(state: &HotState) -> bool {
    if !constrict_owner_is_exact(state, None) || !constrict_peer_state_is_exact(state) {
        return false;
    }
    state.fanouts.constrict_amount() > 0 && state.fanouts.constrict_amount() % 3 == 0
}

/// Death-entry variant: the retained applier has already crossed zero HP, but
/// all other singleton/amount/AI/peer facts must still authenticate before
/// the removal or any generic death cleanup becomes visible.
pub(crate) fn constrict_death_state_is_exact(state: &HotState, dying_uid: u32) -> bool {
    if !constrict_owner_is_exact(state, Some(dying_uid)) || !constrict_peer_state_is_exact(state) {
        return false;
    }
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.kind == MonsterKind::SlitheringStrangler)
        .expect("validated one Strangler");
    state.fanouts.constrict_amount() > 0
        && state.fanouts.constrict_amount() % 3 == 0
        && (owner.hp > 0 || owner.uid == dying_uid)
}

fn constrict_row_is_exact(ctx: &MoveCtx<'_>) -> bool {
    let [constrict, thwack, lash] = ctx.catalog.moves(MonsterKind::SlitheringStrangler) else {
        return false;
    };
    constrict.kind == MoveKind::ConstrictPlayer
        && ctx.catalog.args(constrict.args) == [CompiledArg::I(3)]
        && constrict.repeats == Repeats::Absent
        && thwack.kind == MoveKind::AttackBlock
        // Thwack 0xbdf5c (9, 8, 7) and Lash 0xbdf67 (9, 13, 12) (#2828).
        && ctx.catalog.args(thwack.args)
            == [
                CompiledArg::I(ctx.tier(mc::SLITHERING_STRANGLER_THWACK_DAMAGE)),
                CompiledArg::I(1),
                CompiledArg::I(5),
            ]
        && thwack.repeats == Repeats::Absent
        && lash.kind == MoveKind::Attack
        && ctx.catalog.args(lash.args)
            == [
                CompiledArg::I(ctx.tier(mc::SLITHERING_STRANGLER_LASH_DAMAGE)),
                CompiledArg::I(1),
            ]
        && lash.repeats == Repeats::Absent
}

/// Exact public application and restack body. Admission closes prospective
/// card writers; this defensive entry check re-authenticates every live fact
/// immediately before mutation.
fn constrict_player_exact(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.args != [CompiledArg::I(3)]
        || !constrict_row_is_exact(ctx)
        || ctx.state.history.over
        || ctx.state.player_side_active
        || ctx.state.multiplayer_ally_key != 0
        || !constrict_owner_is_exact(ctx.state, None)
        || !constrict_peer_state_is_exact(ctx.state)
        || ctx
            .state
            .monsters
            .get(ctx.actor)
            .map(|monster| monster.kind)
            != Some(MonsterKind::SlitheringStrangler)
    {
        return Err(EngineRefusal::MalformedArgs("constrict player foundation"));
    }
    let prior = ctx.state.fanouts.constrict_amount();
    if prior < 0 || prior % 3 != 0 {
        return Err(EngineRefusal::MalformedArgs("ConstrictPower amount"));
    }
    let updated = prior
        .checked_add(3)
        .ok_or(EngineRefusal::CounterOverflow("ConstrictPower amount"))?;
    crate::engine::turn::prepare_after_side_turn_end_singleton_write(
        ctx.state,
        crate::hot::AfterSideTurnEndPowerToken::Constrict,
        prior != 0,
        true,
    )?;
    let written = ctx.state.fanouts.set_constrict_amount(updated);
    debug_assert!(written);
    if !constrict_private_state_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "constrict player foundation result",
        ));
    }
    Ok(())
}

/// `("tender_goop", 1)`.
///
/// Python: `monster_act` (frozen, deleted #2827), Tender's `_dispatch_after_card_played_power_object` block, and `_finish_player_turn_after_joss`. Hunter Killer authenticates the
/// one-shot exact applier, installs Tender 1, and resets its private card
/// counter. Every completed owner card increments that counter and applies
/// Strength -1 then Dexterity -1; player side end restores both by the frozen
/// count and resets the counter while Tender remains.
///
/// Current v0.111.0 ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `HunterKiller/<GoopMove>d__15::MoveNext` RVA `0x35c510` applies the exact
/// singleton with the acting Hunter Killer retained as applier;
/// `TenderPower/<AfterCardPlayed>d__12::MoveNext` RVA `0x34688c` increments
/// the completed-card counter before Strength then Dexterity -1; and
/// `<AfterSideTurnEnd>d__13::MoveNext` RVA `0x348fb4` restores Strength then
/// Dexterity, resets only the counter, and preserves Tender itself. The exact
/// fixed opener and Bite/Puncture random machine are authenticated beside the
/// listener carrier, so public dispatch cannot widen to an alien owner/row.
pub(crate) fn tender_goop(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    tender_goop_exact(ctx)
}

/// Roster-independent Tender carrier classification. Reading absence touches
/// only the player slot vector and the packed private scalar; owner identity
/// is authenticated exclusively on the cold active path.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum TenderCarrierState {
    Absent,
    Reachable,
    Malformed,
}

#[inline]
pub(crate) fn tender_carrier_state(state: &HotState) -> TenderCarrierState {
    match (
        state.powers.get(PowerId::Tender),
        state.fanouts.tender_is_active(),
        state.fanouts.tender_cards_played(),
    ) {
        (None, false, 0) => TenderCarrierState::Absent,
        (Some(slot), true, count)
            if slot.wire == SlotWire::Int && slot.value == 1 && count >= 0 =>
        {
            TenderCarrierState::Reachable
        }
        _ => TenderCarrierState::Malformed,
    }
}

#[inline]
pub(crate) fn tender_listener_is_reachable(state: &HotState) -> bool {
    tender_carrier_state(state) == TenderCarrierState::Reachable
}

fn tender_owner_is_exact(state: &HotState, require_alive: bool) -> bool {
    let mut owners = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::HunterKiller);
    let Some(owner) = owners.next() else {
        return false;
    };
    owners.next().is_none()
        && Some(owner.max_hp)
            == crate::engine::monsters::native_fixed_hp(state, MonsterKind::HunterKiller)
        && owner.hp <= owner.max_hp
        && (!require_alive || owner.hp > 0)
        && owner.block >= 0
        && owner.override_state == MonsterOverride::None
        && owner.forced_follow_up == MonsterFollowUp::None
        && !owner.spawn_noop
        && owner.revive_stage == 0
        && owner.pressure_gun_damage == 0
        && owner.pressure_buildup_idx() == 0
        && !owner.is_about_to_blow()
        && state
            .monsters
            .iter()
            .filter(|monster| monster.uid == owner.uid)
            .count()
            == 1
        && crate::engine::turn::hunter_killer_ai_state_is_exact(owner)
}

/// PrepareForNextTurn transient: an exact just-applied Goop row still points
/// at the fixed opener until this very callback consumes its first RAND draw.
pub(crate) fn tender_owner_is_exact_for_prepare(state: &HotState) -> bool {
    if tender_carrier_state(state) != TenderCarrierState::Reachable
        || !tender_owner_is_exact(state, false)
    {
        return false;
    }
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.kind == MonsterKind::HunterKiller)
        .expect("validated one Hunter Killer");
    match owner.random_ai.next() {
        Some(crate::engine::turn::HUNTER_GOOP_INDEX) => {
            owner.random_ai.log_len() == 1
                && owner.random_ai.log_at(0) == Some(crate::engine::turn::HUNTER_GOOP_INDEX)
        }
        Some(crate::engine::turn::HUNTER_BITE_INDEX)
        | Some(crate::engine::turn::HUNTER_PUNCTURE_INDEX) => true,
        _ => false,
    }
}

fn tender_side_end_peers_are_exact(state: &HotState) -> bool {
    crate::engine::damage::player_type_one_listener_order_is_exact(state)
}

/// Authenticate the independent after-power-amount-changed listener order
/// which a still-absent Tender can observe after Hunter Killer's fixed opener.
pub(crate) fn tender_future_listener_peers_are_exact(state: &HotState) -> bool {
    tender_side_end_peers_are_exact(state)
}

/// Bind an absent Tender carrier to the only native pre-application Hunter
/// state: one exact owner parked on the fixed Goop opener. Post-Goop Bite or
/// Puncture logs without the retained singleton are forged canonical states.
pub(crate) fn tender_absent_hunter_entry_is_exact(state: &HotState) -> bool {
    if tender_carrier_state(state) != TenderCarrierState::Absent
        || !tender_owner_is_exact(state, false)
        || !tender_future_listener_peers_are_exact(state)
    {
        return false;
    }
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.kind == MonsterKind::HunterKiller)
        .expect("validated one Hunter Killer");
    owner.random_ai.next() == Some(crate::engine::turn::HUNTER_GOOP_INDEX)
        && owner.random_ai.log_len() == 1
        && owner.random_ai.log_at(0) == Some(crate::engine::turn::HUNTER_GOOP_INDEX)
}

/// Authenticate the retained exact applier, singleton/counter pair, and random
/// machine. The keyed side-end ledger independently authenticates callback
/// order; this predicate does not reconstruct it from the carrier set.
pub(crate) fn tender_private_state_is_exact(state: &HotState) -> bool {
    tender_carrier_state(state) == TenderCarrierState::Reachable
        && tender_owner_is_exact(state, false)
        && state
            .monsters
            .iter()
            .find(|monster| monster.kind == MonsterKind::HunterKiller)
            .is_some_and(|owner| {
                matches!(
                    owner.random_ai.next(),
                    Some(crate::engine::turn::HUNTER_BITE_INDEX)
                        | Some(crate::engine::turn::HUNTER_PUNCTURE_INDEX)
                )
            })
}

pub(crate) fn tender_side_end_entry_is_exact(state: &HotState) -> bool {
    tender_private_state_is_exact(state) && tender_side_end_peers_are_exact(state)
}

fn tender_row_is_exact(ctx: &MoveCtx<'_>) -> bool {
    let [goop, bite, puncture] = ctx.catalog.moves(MonsterKind::HunterKiller) else {
        return false;
    };
    goop.kind == MoveKind::TenderGoop
        && ctx.catalog.args(goop.args) == [CompiledArg::I(1)]
        && goop.repeats == Repeats::Absent
        && bite.kind == MoveKind::Attack
        // Bite 0xb65dd (9, 19, 17) and Puncture 0xb65ea (9, 8, 7) (#2828).
        && ctx.catalog.args(bite.args)
            == [
                CompiledArg::I(ctx.tier(mc::HUNTER_KILLER_BITE_DAMAGE)),
                CompiledArg::I(1),
            ]
        && bite.repeats == Repeats::Absent
        && puncture.kind == MoveKind::Attack
        && ctx.catalog.args(puncture.args)
            == [
                CompiledArg::I(ctx.tier(mc::HUNTER_KILLER_PUNCTURE_DAMAGE)),
                CompiledArg::I(3),
            ]
        && puncture.repeats == Repeats::Absent
}

/// Exact public opener body. Admission authenticates the complete move table,
/// random machine, player-power listener lifecycle, and canonical carrier.
fn tender_goop_exact(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.args != [CompiledArg::I(1)]
        || !tender_row_is_exact(ctx)
        || ctx.state.history.over
        || ctx.state.player_side_active
        || tender_carrier_state(ctx.state) != TenderCarrierState::Absent
        || !tender_future_listener_peers_are_exact(ctx.state)
        || !tender_owner_is_exact(ctx.state, true)
        || ctx.state.monsters.get(ctx.actor).is_none_or(|monster| {
            monster.kind != MonsterKind::HunterKiller
                || monster.hp <= 0
                || monster.random_ai.next() != Some(crate::engine::turn::HUNTER_GOOP_INDEX)
                || monster.random_ai.log_len() != 1
                || monster.random_ai.log_at(0) != Some(crate::engine::turn::HUNTER_GOOP_INDEX)
        })
    {
        return Err(EngineRefusal::MalformedArgs("Tender Goop foundation"));
    }
    crate::engine::play::prepare_after_card_played_scalar_write(ctx.state, PowerId::Tender, 0, 1)?;
    ctx.state.powers.set(PowerId::Tender, SlotWire::Int, 1);
    let written = ctx.state.fanouts.set_tender_state(true, 0);
    debug_assert!(written);
    if tender_carrier_state(ctx.state) != TenderCarrierState::Reachable
        || !tender_owner_is_exact(ctx.state, true)
        || ctx.state.monsters[ctx.actor].random_ai.next()
            != Some(crate::engine::turn::HUNTER_GOOP_INDEX)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Tender Goop foundation result",
        ));
    }
    note_power(ctx.events, Subject::Player, PowerId::Tender, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::content_tables::{Move, random_moves};
    use crate::engine::admission::{AdmissionRefusal, MissingCapability, admit};
    use crate::hot::{HotMonster, HotState, RandomAiState};
    use crate::ids::MonsterKind;
    use serde_json::Value;

    const FIXTURE: &str = include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    fn admit_with_monster(kind: MonsterKind) -> Result<(), AdmissionRefusal> {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert("kind".to_owned(), Value::from(kind.as_str()));
        if kind == MonsterKind::HunterKiller {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("hp".to_owned(), Value::from(126));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(126));
            document.monsters[0]
                .insert("next_move".to_owned(), Value::from("TENDERIZING_GOOP_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("TENDERIZING_GOOP_MOVE")]),
            );
        } else if kind == MonsterKind::SlitheringStrangler {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("hp".to_owned(), Value::from(55));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(55));
            document.monsters[0].insert("next_move".to_owned(), Value::from("CONSTRICT_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("CONSTRICT_MOVE")]),
            );
        } else if kind == MonsterKind::TheObscura {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("ILLUSION_MOVE"));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from("ILLUSION_MOVE")]),
            );
        } else if let Some(opener) = match kind {
            // The plain uniform-CannotRepeat machines admitted by #2481. Any
            // single-entry log is a legal machine for them, so seed the
            // table's first move.
            MonsterKind::SludgeSpinner => Some("OIL_SPRAY"),
            MonsterKind::SoulNexus => Some("SOUL_BURN"),
            MonsterKind::FossilStalker => Some("TACKLE_MOVE"),
            _ => None,
        } {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from(opener));
            document.monsters[0].insert(
                "move_log".to_owned(),
                Value::Array(vec![Value::from(opener)]),
            );
        }
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog)
    }

    fn constrict_catalog() -> crate::catalog::Catalog {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.intern_private_constrict_strangler().unwrap();
        builder.build()
    }

    fn exact_strangler(_amount: i32) -> HotMonster {
        let mut monster = HotMonster::new(MonsterKind::SlitheringStrangler, 55);
        monster.max_hp = 55;
        monster.uid = 17;
        let mut ai = RandomAiState::new();
        assert!(ai.set_next(Some(crate::engine::turn::STRANGLER_CONSTRICT_INDEX)));
        assert!(ai.set_log(&[crate::engine::turn::STRANGLER_CONSTRICT_INDEX]));
        monster.random_ai = ai;
        monster
    }

    fn exact_constrict_state(amount: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.player_side_active = false;
        state.monsters_mut().push(exact_strangler(amount));
        assert!(state.fanouts.set_constrict_amount(amount));
        state
    }

    fn tender_catalog() -> crate::catalog::Catalog {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.intern_private_tender_hunter().unwrap();
        builder.build()
    }

    fn exact_hunter(next: u8, log: &[u8]) -> HotMonster {
        let mut monster = HotMonster::new(MonsterKind::HunterKiller, 126);
        monster.max_hp = 126;
        monster.uid = 23;
        let mut ai = RandomAiState::new();
        assert!(ai.set_next(Some(next)));
        assert!(ai.set_log(log));
        monster.random_ai = ai;
        monster
    }

    fn exact_tender_state(count: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.player_side_active = false;
        state.monsters_mut().push(exact_hunter(
            crate::engine::turn::HUNTER_BITE_INDEX,
            &[
                crate::engine::turn::HUNTER_GOOP_INDEX,
                crate::engine::turn::HUNTER_BITE_INDEX,
            ],
        ));
        state.powers.set(PowerId::Tender, SlotWire::Int, 1);
        assert!(state.fanouts.set_tender_state(true, count));
        state
    }

    /// Mechanically enumerate every generated random-move row owned here.
    fn owned_rows() -> Vec<(MonsterKind, Move)> {
        let rows: Vec<_> = MonsterKind::ALL
            .into_iter()
            .flat_map(|monster| {
                random_moves(monster)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|entry| OWNED.contains(&entry.kind))
                    .map(move |entry| (monster, entry))
            })
            .collect();
        for kind in OWNED {
            assert!(
                rows.iter().any(|(_, entry)| entry.kind == kind),
                "no random move table uses {:?}",
                kind.as_str()
            );
        }
        rows
    }

    #[test]
    fn the_owned_list_is_ascending_complete_and_covers_every_claim() {
        assert_eq!(OWNED.len(), 4);
        assert!(OWNED.windows(2).all(|pair| pair[0] < pair[1]));
        for kind in IMPLEMENTED {
            assert!(OWNED.contains(kind));
        }
    }

    /// This is the mechanical carrier sweep used for the PR admission audit.
    /// All four bodies are claimed. Hunter Killer, Slithering Strangler, and
    /// the Obscura are complete; Soul Nexus remains blocked at the AI gate.
    #[test]
    fn the_generated_carrier_census_is_exact() {
        let carriers: BTreeSet<_> = owned_rows()
            .into_iter()
            .map(|(monster, _)| monster.as_str())
            .collect();
        assert_eq!(
            carriers,
            BTreeSet::from([
                "HUNTER_KILLER",
                "SLITHERING_STRANGLER",
                "SOUL_NEXUS",
                "THE_OBSCURA",
            ])
        );
    }

    /// Hunter Killer, Slithering Strangler, and the Obscura are executable;
    /// Soul Nexus remains refused before Rust reads its random table.
    #[test]
    fn every_carrier_refuses_before_its_random_table_is_read() {
        for monster in owned_rows()
            .into_iter()
            .map(|(monster, _)| monster)
            .collect::<BTreeSet<_>>()
        {
            if matches!(
                monster,
                MonsterKind::FossilStalker
                    | MonsterKind::HunterKiller
                    | MonsterKind::SlitheringStrangler
                    | MonsterKind::SludgeSpinner
                    | MonsterKind::SoulNexus
                    | MonsterKind::TheObscura
            ) {
                assert_eq!(admit_with_monster(monster), Ok(()), "{}", monster.as_str());
                continue;
            }
            let refusal = admit_with_monster(monster)
                .expect_err("an unmodeled random-AI monster cannot admit");
            assert!(
                refusal.contains(MissingCapability::MonsterAi(monster)),
                "{:?} must refuse on random AI; it said {refusal}",
                monster.as_str()
            );
        }
    }

    /// A documented escalation that accidentally receives a body but not a
    /// manifest entry would otherwise be unreachable and invisible.
    #[test]
    fn every_unclaimed_kind_refuses_by_its_own_name() {
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let mut events = Vec::new();

        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &[],
                events: &mut events,
            };
            assert_eq!(
                super::super::apply_move(kind, &mut ctx),
                Err(EngineRefusal::MoveKindNotModeled(kind))
            );
        }
        assert!(events.is_empty());
    }

    #[test]
    fn exact_spawned_bodies_preserve_serial_order_and_liveness() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulNexus).unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        let mut nexus = crate::hot::HotMonster::new(MonsterKind::SoulNexus, 30);
        nexus.uid = 4;
        let mut ally = crate::hot::HotMonster::new(MonsterKind::Parafright, 20);
        ally.uid = 9;
        state.monsters_mut().extend([nexus, ally]);
        let mut events = Vec::new();
        let args = [
            CompiledArg::I(19),
            CompiledArg::I(1),
            CompiledArg::I(2),
            CompiledArg::I(2),
        ];
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        attack_vuln_weak_player(&mut ctx).unwrap();
        assert_eq!(state.hp, 81);
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 2);
        assert_eq!(state.powers.value(PowerId::PlayerVulnFresh), 1);
        assert_eq!(state.powers.value(PowerId::PlayerWeak), 2);
        assert_eq!(state.powers.value(PowerId::PlayerWeakFresh), 1);

        state.monsters_mut()[0].kind = MonsterKind::TheObscura;
        let args = [CompiledArg::I(3)];
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        buff_team_strength(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 3);
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), 3);

        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, 0);
        state.monsters_mut()[1]
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX - 3);
        state.monsters_mut()[1]
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -1);
        let before = state.clone();
        let before_events = events.clone();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            buff_team_strength(&mut ctx),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(state, before, "later recipient refuses before owner write");
        assert_eq!(events, before_events);
    }

    /// #2927 (run LYE3ZK9FYKKV floor 43): Drain Life now carries the catalog,
    /// so a player-Thorns kill of Soul Nexus with Gremlin Horn owned resolves
    /// instead of refusing on the Horn death catalog. The secondary
    /// Parafright does not keep combat alive, so the kill ends it: Horn's
    /// GainEnergy/Draw see an ending combat and the debuff tail is skipped.
    #[test]
    fn drain_life_thorns_kill_with_gremlin_horn_resolves() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulNexus).unwrap();
        let strike = builder
            .intern(crate::catalog::CardIdentity {
                id: crate::ids::CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state.powers.set(PowerId::Thorns, SlotWire::Int, 3);
        state.fanouts.set_gremlin_horn_owned(true);
        state.next_card_uid = 1;
        state
            .piles
            .get_mut(crate::hot::PileId::Draw)
            .make_mut()
            .push(crate::hot::HotCard {
                uid: 0,
                atom: strike,
                flags: 0,
            });
        let mut nexus = crate::hot::HotMonster::new(MonsterKind::SoulNexus, 3);
        nexus.uid = 4;
        let mut ally = crate::hot::HotMonster::new(MonsterKind::Parafright, 20);
        ally.uid = 9;
        state.monsters_mut().extend([nexus, ally]);
        let energy = state.energy;
        let args = [
            CompiledArg::I(19),
            CompiledArg::I(1),
            CompiledArg::I(2),
            CompiledArg::I(2),
        ];
        attack_vuln_weak_player(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut Vec::new(),
        })
        .unwrap();
        assert!(state.history.over);
        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.hp, 81, "the in-flight hit still lands");
        assert_eq!(state.energy, energy);
        assert!(state.piles.get(crate::hot::PileId::Hand).is_empty());
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 0);
        assert_eq!(state.powers.value(PowerId::PlayerWeak), 0);
    }

    #[test]
    fn constrict_public_writer_stacks_three_on_the_exact_applier_only() {
        let catalog = constrict_catalog();
        let mut state = exact_constrict_state(0);
        let args = [CompiledArg::I(3)];
        let mut events = Vec::new();
        assert!(constrict_owner_is_exact(&state, None));
        assert!(constrict_peer_state_is_exact(&state));
        {
            let ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &args,
                events: &mut events,
            };
            assert!(
                constrict_row_is_exact(&ctx),
                "moves={:?} args={:?}",
                ctx.catalog.moves(MonsterKind::SlitheringStrangler),
                ctx.catalog
                    .moves(MonsterKind::SlitheringStrangler)
                    .iter()
                    .map(|row| ctx.catalog.args(row.args))
                    .collect::<Vec<_>>()
            );
        }

        for expected in [3, 6] {
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &args,
                events: &mut events,
            };
            constrict_player_exact(&mut ctx).unwrap();
            assert_eq!(state.fanouts.constrict_amount(), expected);
            assert_eq!(state.monsters[0].uid, 17);
            assert!(constrict_private_state_is_exact(&state));
        }

        let before = state.clone();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(constrict_player(&mut ctx), Ok(()));
        assert_eq!(state.fanouts.constrict_amount(), 9);
        assert_ne!(state, before);
        assert!(events.is_empty());
        assert!(IMPLEMENTED.contains(&MoveKind::ConstrictPlayer));
    }

    #[test]
    fn constrict_public_writer_refuses_wrong_owner_duplicate_dead_and_overflow_atomically() {
        let catalog = constrict_catalog();
        let args = [CompiledArg::I(3)];

        let mut overflow = exact_constrict_state(2_147_483_646);
        let before = overflow.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 41 }];
        let before_events = events.clone();
        let mut ctx = MoveCtx {
            state: &mut overflow,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        assert_eq!(
            constrict_player_exact(&mut ctx),
            Err(EngineRefusal::CounterOverflow("ConstrictPower amount"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);

        let mut dead = exact_constrict_state(0);
        dead.monsters_mut()[0].hp = 0;
        let mut duplicate = exact_constrict_state(0);
        let mut second = exact_strangler(0);
        second.uid = 18;
        duplicate.monsters_mut().push(second);
        let mut wrong_actor = exact_constrict_state(0);
        let mut peer = HotMonster::new(MonsterKind::FrogKnight, 10);
        peer.max_hp = 10;
        peer.uid = 19;
        wrong_actor.monsters_mut().push(peer);

        for (mut state, actor) in [(dead, 0), (duplicate, 0), (wrong_actor, 1)] {
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor,
                args: &args,
                events: &mut events,
            };
            assert!(matches!(
                constrict_player_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs(_))
            ));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn waterfall_payload_and_player_constrict_carriers_are_disjoint() {
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 100);
        assert!(waterfall.set_waterfall_steam_eruption_damage(35));
        assert_eq!(waterfall.waterfall_steam_eruption_damage(), 35);

        let mut strangler = exact_strangler(0);
        assert_eq!(strangler.waterfall_steam_eruption_damage(), 0);
        assert!(!strangler.set_waterfall_steam_eruption_damage(35));

        let mut root = exact_constrict_state(3);
        let mut detached = root.clone();
        assert!(detached.fanouts.set_constrict_amount(6));
        assert_eq!(root.fanouts.constrict_amount(), 3);
        assert_eq!(detached.fanouts.constrict_amount(), 6);
        assert!(root.fanouts.set_constrict_amount(0));
        assert_eq!(detached.fanouts.constrict_amount(), 6);

        let mut alien = HotMonster::new(MonsterKind::TestSubject, 100);
        alien.max_hp = 100;
        alien.forge_waterfall_steam_payload(3);
        assert!(alien.waterfall_steam_payload_is_set());
        assert_eq!(alien.waterfall_steam_eruption_damage(), 0);
        assert!(!crate::engine::monsters::test_subject_state_is_valid(&{
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(alien);
            state
        }));

        // Keep the mutable binding meaningful: failed cross-owner setters do
        // not alter the authenticated Strangler interpretation.
        assert_eq!(strangler.waterfall_steam_eruption_damage(), 0);
    }

    #[test]
    fn public_strangler_catalog_registration_is_idempotent_in_both_orders() {
        let mut ordinary_first = crate::catalog::CatalogBuilder::new();
        ordinary_first
            .intern_monster(MonsterKind::SlitheringStrangler)
            .unwrap();
        ordinary_first.intern_private_constrict_strangler().unwrap();
        assert_eq!(
            ordinary_first
                .build()
                .moves(MonsterKind::SlitheringStrangler)
                .len(),
            3
        );

        let mut private_first = crate::catalog::CatalogBuilder::new();
        private_first.intern_private_constrict_strangler().unwrap();
        private_first
            .intern_monster(MonsterKind::SlitheringStrangler)
            .unwrap();
        assert_eq!(
            private_first
                .build()
                .moves(MonsterKind::SlitheringStrangler)
                .len(),
            3
        );
    }

    #[test]
    fn tender_public_dispatch_requires_exact_goop_opener_and_emits_one_power_event() {
        let catalog = tender_catalog();
        let mut state = HotState::at_defaults();
        state.player_side_active = false;
        state.monsters_mut().push(exact_hunter(
            crate::engine::turn::HUNTER_GOOP_INDEX,
            &[crate::engine::turn::HUNTER_GOOP_INDEX],
        ));
        let args = [CompiledArg::I(1)];
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        super::super::apply_move(MoveKind::TenderGoop, &mut ctx).unwrap();
        assert_eq!(state.powers.value(PowerId::Tender), 1);
        assert!(state.fanouts.tender_is_active());
        assert_eq!(state.fanouts.tender_cards_played(), 0);
        assert_eq!(
            events,
            [crate::engine::Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::Tender,
                amount: 1,
            }]
        );
        assert!(tender_owner_is_exact_for_prepare(&state));
        assert!(!tender_private_state_is_exact(&state));

        let before = state.clone();
        let before_events = events.clone();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: &args,
            events: &mut events,
        };
        assert!(super::super::apply_move(MoveKind::TenderGoop, &mut ctx).is_err());
        assert_eq!(state, before);
        assert_eq!(events, before_events);
        assert!(IMPLEMENTED.contains(&MoveKind::TenderGoop));
    }

    #[test]
    fn tender_private_writer_refuses_wrong_boundary_actor_args_and_terminal_atomically() {
        let catalog = tender_catalog();
        let mut base = HotState::at_defaults();
        base.player_side_active = false;
        base.monsters_mut().push(exact_hunter(
            crate::engine::turn::HUNTER_GOOP_INDEX,
            &[crate::engine::turn::HUNTER_GOOP_INDEX],
        ));
        let mut peer = HotMonster::new(MonsterKind::FrogKnight, 10);
        peer.max_hp = 10;
        peer.uid = 24;
        base.monsters_mut().push(peer);

        let mut cases = Vec::new();
        let mut bite = base.clone();
        let mut ai = RandomAiState::new();
        assert!(ai.set_next(Some(crate::engine::turn::HUNTER_BITE_INDEX)));
        assert!(ai.set_log(&[
            crate::engine::turn::HUNTER_GOOP_INDEX,
            crate::engine::turn::HUNTER_BITE_INDEX,
        ]));
        bite.monsters_mut()[0].random_ai = ai;
        cases.push((bite, 0, [CompiledArg::I(1)]));
        let mut owner_side = base.clone();
        owner_side.player_side_active = true;
        cases.push((owner_side, 0, [CompiledArg::I(1)]));
        let mut terminal = base.clone();
        terminal.history.over = true;
        cases.push((terminal, 0, [CompiledArg::I(1)]));
        cases.push((base.clone(), 1, [CompiledArg::I(1)]));
        cases.push((base.clone(), 0, [CompiledArg::I(2)]));

        for (mut state, actor, args) in cases {
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 66 }];
            let before_events = events.clone();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor,
                args: &args,
                events: &mut events,
            };
            assert!(tender_goop_exact(&mut ctx).is_err());
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        for (mut state, token) in [
            {
                let mut state = base.clone();
                state.powers.set(PowerId::ConsumingShadow, SlotWire::Int, 1);
                (
                    state,
                    crate::hot::AfterSideTurnEndPowerToken::ConsumingShadow,
                )
            },
            {
                let mut state = base.clone();
                assert!(state.fanouts.set_constrict_amount(3));
                (state, crate::hot::AfterSideTurnEndPowerToken::Constrict)
            },
        ] {
            assert_eq!(
                state.fanouts.register_after_side_turn_end_power(token),
                Ok(0),
            );
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &[CompiledArg::I(1)],
                events: &mut events,
            };
            tender_goop_exact(&mut ctx).unwrap();
            assert_eq!(
                state
                    .fanouts
                    .after_side_turn_end_power_order()
                    .iter()
                    .map(|entry| entry.token)
                    .collect::<Vec<_>>(),
                [token, crate::hot::AfterSideTurnEndPowerToken::Tender],
            );
        }
    }

    #[test]
    fn tender_carrier_authenticates_raw_slot_wire_counter_owner_and_active_intent() {
        let exact = exact_tender_state(7);
        assert_eq!(tender_carrier_state(&exact), TenderCarrierState::Reachable);
        assert!(tender_private_state_is_exact(&exact));

        let mut dead_owner = exact.clone();
        dead_owner.monsters_mut()[0].hp = 0;
        assert!(tender_private_state_is_exact(&dead_owner));

        let mut malformed = Vec::new();
        let mut count_only = HotState::at_defaults();
        count_only.fanouts.forge_tender_state(false, 1);
        malformed.push(count_only);
        let mut bit_only = HotState::at_defaults();
        bit_only.fanouts.forge_tender_state(true, 0);
        malformed.push(bit_only);
        let mut wrong_wire = exact.clone();
        wrong_wire.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
            key: PowerId::Tender,
            wire: SlotWire::Bool,
            value: 1,
        }])
        .unwrap();
        malformed.push(wrong_wire);
        let mut zero_slot = exact.clone();
        zero_slot.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
            key: PowerId::Tender,
            wire: SlotWire::Int,
            value: 0,
        }])
        .unwrap();
        malformed.push(zero_slot);
        let mut wrong_amount = exact.clone();
        wrong_amount.powers.set(PowerId::Tender, SlotWire::Int, 2);
        malformed.push(wrong_amount);
        let mut opener_alias = exact.clone();
        opener_alias.monsters_mut()[0] = exact_hunter(
            crate::engine::turn::HUNTER_GOOP_INDEX,
            &[crate::engine::turn::HUNTER_GOOP_INDEX],
        );
        malformed.push(opener_alias);

        for state in malformed {
            assert_ne!(tender_carrier_state(&state), TenderCarrierState::Absent);
            assert!(!tender_private_state_is_exact(&state));
        }
    }

    #[test]
    fn tender_production_reader_writer_and_completion_call_site_census_is_exact() {
        fn production(source: &'static str) -> &'static str {
            source.split("\nmod tests {").next().unwrap()
        }
        let play = production(include_str!("../engine/play.rs"));
        let turn = production(include_str!("../engine/turn.rs"));
        let damage = production(include_str!("../engine/damage.rs"));
        let boundary = production(include_str!("../boundary.rs"));
        let admission = production(include_str!("../engine/admission.rs"));
        let spawned = production(include_str!("spawned.rs"));

        assert_eq!(
            play.matches("apply_active_tender_after_card_played(")
                .count(),
            2,
            "one definition plus the generalized object-walk dispatch site"
        );
        assert_eq!(play.matches("tender_carrier_state(state)").count(), 2);
        assert_eq!(
            turn.matches("tender_carrier_state(state)").count(),
            3,
            "entry, parked AutoPost validation, and live side-end reads"
        );
        assert_eq!(damage.matches("tender_carrier_state(state)").count(), 1);
        assert_eq!(admission.matches("tender_carrier_state(state)").count(), 2);
        assert_eq!(spawned.matches("tender_carrier_state(state)").count(), 4);

        assert_eq!(play.matches("set_tender_state(").count(), 1);
        assert_eq!(turn.matches("set_tender_state(").count(), 2);
        assert_eq!(damage.matches("set_tender_state(").count(), 0);
        assert_eq!(boundary.matches("set_tender_state(").count(), 1);
        assert_eq!(spawned.matches("set_tender_state(").count(), 1);
        assert_eq!(
            play.matches("tender_is_active()").count(),
            3,
            "the callback reader plus exact-ledger authentication and live membership recheck"
        );
        assert_eq!(
            turn.matches("tender_is_active()").count(),
            5,
            // #3383 retired the Dark Embrace + Hellraiser ordering wall that
            // was the sixth reader: the object ledger orders Tender.
            "entry snapshot, ledger authentication, the empty-ledger fast path, the live callback, and Hunter Killer prepare-owner validation"
        );
        assert_eq!(boundary.matches("tender_is_active()").count(), 2);
        assert_eq!(spawned.matches("tender_is_active()").count(), 1);

        assert_eq!(
            spawned.matches("powers.get(PowerId::Tender)").count(),
            1,
            "the raw player-slot classifier is centralized"
        );
        assert_eq!(
            boundary.matches("powers.get(PowerId::Tender)").count(),
            2,
            "boundary hydration and projection independently authenticate the slot"
        );
        assert_eq!(
            admission.matches("powers.get(PowerId::Tender)").count(),
            1,
            "admission's only direct slot read is the alien-monster scan"
        );
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn constrict_first_writer_pays_one_fanout_cow_and_restack_pays_none() {
        let catalog = constrict_catalog();
        let root = exact_constrict_state(0);
        let mut successor = root.clone();
        let args = [CompiledArg::I(3)];
        let mut events = Vec::new();

        let (before_allocations, before_bytes) = crate::allocation::thread_snapshot();
        {
            let mut ctx = MoveCtx {
                state: &mut successor,
                catalog: &catalog,
                actor: 0,
                args: &args,
                events: &mut events,
            };
            constrict_player_exact(&mut ctx).unwrap();
        }
        let (after_first_allocations, after_first_bytes) = crate::allocation::thread_snapshot();
        {
            let mut ctx = MoveCtx {
                state: &mut successor,
                catalog: &catalog,
                actor: 0,
                args: &args,
                events: &mut events,
            };
            constrict_player_exact(&mut ctx).unwrap();
        }
        let (after_second_allocations, after_second_bytes) = crate::allocation::thread_snapshot();

        assert_eq!(after_first_allocations - before_allocations, 1);
        assert_eq!(after_first_bytes - before_bytes, 328);
        assert_eq!(after_second_allocations, after_first_allocations);
        assert_eq!(after_second_bytes, after_first_bytes);
        assert_eq!(root.fanouts.constrict_amount(), 0);
        assert_eq!(successor.fanouts.constrict_amount(), 6);
    }
}
