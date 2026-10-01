//! Card-step bodies for the `content/cards/defect_rare.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status: 9 of 11 ported, 2 still escalated
//!
//! #1321 escalated all eleven. Engine slice 2 (#1367) landed
//! `State.exact_piles` (#1354), which is the whole of what
//! `all_for_one_exact` was waiting on, so that body is ported below. Adaptive
//! Strike, Flak Cannon, Imitation Learning, and Consuming Shadow now join it;
//! the rest stay refusing stubs whose doc comments name the primitive each is
//! waiting on.
//! Two findings from #1321 are worth reading before picking this family
//! up again:
//!
//! * **The family is orb-shaped.** Defect's rares reach the orb engine, the
//!   generation subsystem and per-instance card state. Unit C #1562 closes
//!   the exact two-Player target/replay quotient for Imitation Learning.
//!   Several of the eleven cards are additionally refused by
//!   [`crate::engine::admission`]'s keyword walk (Power cards, `exhausts`,
//!   `AnyAlly`/`RandomEnemy` target types), so their step bodies are not even
//!   the first thing in their way.
//! * **A wave PR could not land a card step kind on its own.** Listing a kind
//!   in [`IMPLEMENTED`] registered it in the manifest but did **not** make its
//!   card admissible, because `admission::admit_card` also ran the central
//!   `admission::step_args`, whose catch-all turned every kind without a
//!   hand-written arm into `MissingCapability::ArgumentShape`. Measured on the
//!   #1321 branch: implementing `all_for_one_exact` and listing it moved the
//!   manifest to 4/333 kinds while its entry still refused on the shape.
//!   **Resolved by #1366**: a wave kind's argument shape is body-owned, the
//!   gate holds an opinion only for the R0.5 three, and the body returns a
//!   typed refusal on surprise.
//!
//! `all_for_one_exact` was #1321's "closest miss", blocked on `State.exact_piles`
//! alone; slice 2 landed the flag and the body with it. #1563 adds Adaptive
//! Strike's exact rewritten generated-clone transaction.

use super::StepCtx;
use crate::catalog::{CardSpec, Catalog, CompiledArg};
use crate::engine::cards::inject_generated_rewritten_clone_bottom;
use crate::engine::damage::{
    gain_powered_card_block, player_attack_from_card, player_attack_random_from_card,
};
use crate::engine::draw::{MAX_CARDS_IN_HAND, repair_card_play_after_physical_move};
use crate::engine::play::{resolved_energy_cost, unique_live_card_location};
use crate::engine::{EngineRefusal, Event};
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_GENETIC_ALGORITHM_STATE, CARD_FLAG_LEGACY, HotCard,
    LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, PileId,
};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// Nine of eleven: `all_for_one_exact`, freed by `State.exact_piles` landing
/// in engine slice 2 (#1354/#1367), Adaptive Strike's #1563 rewritten-clone
/// transaction, Flak Cannon's Unit-C Batch-A status closure, and Imitation
/// Learning's Unit-C Batch-B Player replay quotient, plus Modded's exact
/// active-card local-cost append, Reboot's frozen full-shuffle transaction,
/// Consuming Shadow's orb lifecycle, and this tranche's Creative AI writer.
/// The other three reach richer generation or per-instance card state; each
/// stub names its own.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::ActiveCardCostAddExact,
    StepKind::AdaptiveStrikeExact,
    StepKind::AllForOneExact,
    StepKind::ConsumingShadowExact,
    StepKind::CreativeAi,
    StepKind::FlakCannonExact,
    StepKind::GeneticAlgorithmBlockExact,
    StepKind::GeneticAlgorithmGrowthExact,
    StepKind::ImitationLearningExact,
    StepKind::OneForAllExact,
    StepKind::RebootShuffleExact,
];

/// Validate every frozen Flak Status uid before the public play transaction
/// spends resources or publishes events. Python's `_locate_card_uid` counts
/// physical occurrences, not merely the piles containing a uid.
pub(crate) fn preflight_flak_cannon_status_uids(
    state: &crate::hot::HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    for pile in [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play] {
        for card in state.piles.get(pile).as_slice() {
            if catalog.spec(card.atom).is_some_and(|spec| spec.is_status) {
                unique_live_card_location(state, card.uid)?
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
            }
        }
    }
    Ok(())
}

fn exact_adaptive_source(ctx: &StepCtx<'_>) -> Result<HotCard, EngineRefusal> {
    let mut matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice().iter().copied())
        .filter(|card| card.uid == ctx.source_uid);
    let source = matches.next().ok_or(EngineRefusal::ActiveCardNotUnique {
        uid: ctx.source_uid,
        matches: 0,
    })?;
    if matches.next().is_some() {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 2,
        });
    }
    if ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || ctx.spec.identity.id != CardId::AdaptiveStrike
    {
        return Err(EngineRefusal::MalformedArgs("adaptive_strike_exact source"));
    }
    Ok(source)
}

/// Modded's final active-card Energy-cost write.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The exact L0/L1 program has
/// already added one orb slot and drawn one/two cards. It then re-resolves the
/// same physical source uid and appends Relative `+1`, ThisCombat,
/// reduceOnly=false without folding prior rows. Manual replay and AutoPlay
/// therefore mutate the same one instance once per completed body.
fn active_card_cost_add_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    let [slots, draw, add] = catalog.steps(spec) else {
        return false;
    };
    let expected_draw = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::Modded, 0) => 1,
        (CardId::Modded, 1) => 2,
        _ => return false,
    };
    crate::engine::play::body_enchantment_is_exact(spec)
        && slots.kind == StepKind::AddOrbSlotsExact
        && catalog.args(slots.args) == [CompiledArg::I(1)]
        && draw.kind == StepKind::Draw
        && catalog.args(draw.args) == [CompiledArg::I(expected_draw)]
        && add.kind == StepKind::ActiveCardCostAddExact
        && catalog.args(add.args) == [CompiledArg::I(1)]
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
}

/// Authenticate Modded's complete serialized program and final physical
/// writer before the first body step can add an orb slot or draw a card.
pub(crate) fn preflight_active_card_cost_add_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if !active_card_cost_add_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs(
            "active_card_cost_add_exact program",
        ));
    }
    let (_, _, active) = super::physical_cost::validate_active_physical_source(
        state,
        catalog,
        source_uid,
        CardId::Modded,
    )?;
    if catalog.spec(active.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs("active local-cost source"));
    }
    Ok(())
}

pub(crate) fn active_card_cost_add_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !active_card_cost_add_program_is_exact(ctx.catalog, ctx.spec)
        || ctx.args != [CompiledArg::I(1)]
    {
        return Err(EngineRefusal::MalformedArgs("active_card_cost_add_exact"));
    }
    super::physical_cost::append_active_local_cost_modifier(
        ctx,
        CardId::Modded,
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: 1,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    )
}

/// `adaptive_strike_exact` — Adaptive Strike's rewritten-clone body.
///
/// Python: `_run_steps_inner`'s branch (frozen Python, deleted #2827) (W197 Adaptive Strike).
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256 `9cb4f1ad…`):
/// `AdaptiveStrike/<OnPlay>d__5::MoveNext` RVA `0x389008` awaits the attack
/// (`DamageCmd.Attack` … `AttackCommand.Execute`, IL `0x004c–0x00dc`), then
/// with no ending test of its own runs `CreateClone` (IL `0x00de`),
/// `EnergyCost.SetThisCombat(0)` (IL `0x00e4–0x00ec`) and
/// `CardPileCmd.AddGeneratedCardToCombat(clone, Discard, owner)` (IL
/// `0x00f1–0x00fa`). That wrapper, `<AddGeneratedCardToCombat>d__5` RVA
/// `0x3e2e28`, delegates to `AddGeneratedCardsToCombat` at IL `0x0033`, whose
/// `<AddGeneratedCardsToCombat>d__6` RVA `0x3e2f0c` gates only on
/// `CombatManager.IsInProgress` (IL `0x0039–0x0043`; still true while the
/// combat is ending, `CombatManager::IsCombatEnding` RVA `0x135854` IL
/// `0x000d–0x0012`) and records `CombatHistory.CardGenerated` (IL
/// `0x0104–0x0120`) before the nested `CardPileCmd.Add` ending gate.
///
/// So the clone is recorded from whatever the counters hold AFTER the attack:
/// a listener inside the attack that generated a card (Entomancer's Personal
/// Hive, then a lethal Smokestack) keeps its advance, and this generation
/// adds one more. The shared record-before-Add transaction in
/// [`inject_generated_rewritten_clone_bottom`] already models exactly that on
/// both the live and the ending path; #1723 removed a call-site assignment
/// that rewound both counters to their pre-attack value plus one.
pub(crate) fn adaptive_strike_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::AdaptiveStrike, 0, [CompiledArg::I(18)]) => 18,
        (CardId::AdaptiveStrike, 1, [CompiledArg::I(23)]) => 23,
        _ => return Err(EngineRefusal::MalformedArgs("adaptive_strike_exact")),
    };
    exact_adaptive_source(ctx)?;
    // Refuse a counter already at its ceiling before the attack mutates
    // anything; the post-attack transaction re-checks its own increments.
    ctx.state
        .history
        .owner_generated_cards_combat
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow(
            "owner_generated_cards_combat",
        ))?;
    ctx.state
        .next_generated_hook_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("next_generated_hook_uid"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;
    let source = exact_adaptive_source(ctx)?;
    let mut instance = ctx.state.card_states.get(source.uid);
    let mut rows = instance.local_cost_modifiers.as_slice().to_vec();
    rows.push(LocalCostModifier {
        kind: LocalCostModifierKind::Set,
        amount: 0,
        expiration: LocalCostExpiration::ThisCombat,
        reduce_only: false,
    });
    instance.local_cost_modifiers.replace_rows(rows);
    inject_generated_rewritten_clone_bottom(
        ctx.state,
        ctx.catalog,
        source,
        source.flags | CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        instance,
        PileId::Discard,
        ctx.events,
    )
}

/// `("all_for_one_exact", damage)` — All for One.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827), plus `_all_for_one_discard_snapshot` and `_all_for_one_add_hand_bottom`. One single-target
/// attack, then the **post-attack** Discard is filtered and frozen, and each
/// frozen object is recalled by physical uid in that order.
///
/// Three things the order makes load-bearing:
///
/// * the snapshot is taken **after** the attack, so a card the attack's Thorns
///   retaliation put in the Discard is eligible;
/// * the filter is `cost == 0 && !x_cost && (attack || skill || power)` on the
///   *live* cost: `AllForOne::Filter` RVA `0xd7ad4` reads
///   `EnergyCost.GetWithModifiers(-1)` (every modifier, IL_000c-IL_0013).
///   That is [`resolved_energy_cost`], the resolver legal-action enumeration
///   and payment use, so the card-local list, the early listeners (Tangled,
///   Borrowed Time, Curious, Spiked Gauntlets) and the Late zeroes
///   (Corruption, Free*, Brilliant Scarf, Void Form) all apply here as they
///   do there;
/// * the recall is serial and re-checks `s.over` per candidate, and a full
///   hand sends that candidate to the Discard **bottom** instead — it is not
///   skipped.
///
/// Each Add commits through `_commit_live_card_piles(..., force_exact=True)`,
/// so the first recall promotes the fight to
/// [`crate::hot::HotState::exact_piles`] (frozen Python, deleted #2827) and every later
/// `legal_actions` deduplicates by hand shape rather than by payload. An empty
/// frozen list performs no Add and therefore does not promote — which is why
/// the flag is written inside the loop and not before it.
///
/// `AllForOne/<OnPlay>d__3` RVA `0x389aa8` awaits one plural `CardPileCmd.Add`
/// to Hand (IL_0131). `<Add>d__10` (`0x3e1ba4`) skips every combat-pile move at
/// `IsEnding` (IL_0041-008a), so the recall tests the shared IsEnding
/// projection, not `history.over` (#3515).
pub(crate) fn all_for_one_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("all_for_one_exact"));
    };
    let damage = *damage;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )?;

    let frozen: Vec<HotCard> = ctx
        .state
        .piles
        .get(PileId::Discard)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| {
            ctx.catalog.spec(card.atom).is_some_and(|spec| {
                resolved_energy_cost(ctx.state, *card, spec) == 0
                    && !spec.x_cost
                    && (spec.is_attack || spec.is_skill || spec.is_power)
            })
        })
        .collect();

    for candidate in frozen {
        // `_all_for_one_add_hand_bottom` returns False on an ended combat,
        // and the caller breaks rather than continuing down the list.
        if crate::engine::damage::damage_combat_is_ending(ctx.state) {
            break;
        }
        let Some(index) = ctx
            .state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .position(|live| live.uid == candidate.uid)
        else {
            return Err(EngineRefusal::FrozenCardVanished {
                uid: candidate.uid,
                pile: PileId::Discard,
            });
        };
        let destination = if ctx.state.piles.get(PileId::Hand).len() >= MAX_CARDS_IN_HAND {
            PileId::Discard
        } else {
            PileId::Hand
        };
        repair_card_play_after_physical_move(ctx.state, candidate, PileId::Discard, destination)?;
        ctx.state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .remove(index);
        ctx.state
            .piles
            .get_mut(destination)
            .make_mut()
            .push(candidate);
        ctx.state.exact_piles = true;
        ctx.events.push(Event::CardResolved {
            uid: candidate.uid,
            pile: destination,
        });
    }
    Ok(())
}

/// Consuming Shadow's fixed-Dark serial Channel body and power application.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the exact carrier
/// and enters `_begin_random_orb_loop` with its physical source UID.
pub(crate) fn consuming_shadow_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs(
            "consuming_shadow_exact program",
        ));
    };
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != crate::ids::CardId::ConsumingShadow
        || ctx.spec.identity.upgrade > 1
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::ConsumingShadowExact
        || !ctx.catalog.args(program.args).is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("consuming_shadow_exact"));
    }
    let (pile, index) = unique_live_card_location(ctx.state, ctx.source_uid)?.ok_or(
        EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        },
    )?;
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "consuming_shadow_exact physical source",
        ));
    }
    let mut next = ctx.state.clone();
    let mut emitted = Vec::new();
    crate::engine::orbs::consuming_shadow_card(
        &mut next,
        ctx.catalog,
        2 + ctx.spec.identity.upgrade,
        &mut emitted,
    )?;
    *ctx.state = next;
    ctx.events.extend(emitted);
    Ok(())
}

/// Creative AI's additive owner power and acquisition-order registration.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `CreativeAi/<OnPlay>d__4::MoveNext` RVA `0x395684` IL `0x009e-0x0128`
/// applies the canonical base value one; `OnUpgrade` RVA `0xdc2a3` changes
/// only Energy cost. Python `_run_steps_inner`'s writer (frozen Python, deleted #2827) validates
/// the exact fully unlocked solo-Defect generation pool before the
/// zero-to-positive order append. `_advance_before_hand_draw_power_frame` owns the live-Amount loop and its authenticated generated-card
/// transactions.
pub(crate) fn creative_ai(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let program = ctx
        .catalog
        .steps(ctx.spec)
        .first()
        .ok_or(EngineRefusal::MalformedArgs("creative_ai program"))?;
    if ctx.spec.identity.id != CardId::CreativeAi
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || ctx.catalog.steps(ctx.spec).len() != 1
        || program.kind != StepKind::CreativeAi
        || ctx.catalog.args(program.args) != [CompiledArg::I(1)]
        || ctx.args != [CompiledArg::I(1)]
        || !crate::engine::cards::owner_listener_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("creative_ai"));
    }
    crate::engine::cards::exact_owner_listener_pool(ctx.state, ctx.catalog, PowerId::CreativeAi)?;
    let old = ctx.state.powers.value(PowerId::CreativeAi);
    let updated = old
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("creative_ai"))?;
    if old == 0
        && !ctx
            .state
            .fanouts
            .register_before_hand_draw(PowerId::CreativeAi)
    {
        return Err(EngineRefusal::CounterOverflow("fanout listener order"));
    }
    ctx.state
        .powers
        .set(PowerId::CreativeAi, SlotWire::Int, updated);
    crate::engine::damage::note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::CreativeAi,
        updated,
    );
    Ok(())
}

/// Flak Cannon's frozen all-piles Status exhaust and random attack.
///
/// Python: `_run_steps_inner`'s branch (frozen Python, deleted #2827).
///
/// `CardSpec::is_status` is the generated current-build CardType projection.
/// Snapshot uids in native pile order, excluding Exhaust; each serial command
/// re-resolves its uid after prior exhaust listeners, and the final random
/// attack keeps the original count. The complete body is rehearsed before the
/// first source-pile removal so a later listener refusal is atomic.
pub(crate) fn flak_cannon_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("flak_cannon_exact"));
    };
    let expected = 8 + 3 * i64::from(ctx.spec.identity.upgrade);
    if ctx.spec.identity.id != CardId::FlakCannon
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || *damage != expected
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("flak_cannon_exact owner"));
    }

    fn apply(
        state: &mut crate::hot::HotState,
        catalog: &Catalog,
        _spec: &CardSpec,
        source_uid: u32,
        damage: i64,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        preflight_flak_cannon_status_uids(state, catalog)?;
        // Native CombatState.AllPiles is Hand/Draw/Discard/Exhaust/Play;
        // `PileId::ALL` is deliberately alphabetic for the canonical wire and
        // therefore cannot stand in for this observable exhaust order.
        let frozen: Vec<u32> = [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play]
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice().iter().copied())
            .filter(|card| catalog.spec(card.atom).is_some_and(|row| row.is_status))
            .map(|card| card.uid)
            .collect();
        let frozen_count: i64 = frozen
            .len()
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("flak status count"))?;
        let step_index =
            if crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog) {
                crate::engine::play::after_card_exhausted_source_step_cursor(state, source_uid)?
            } else {
                0
            };
        continue_flak_cannon(
            state,
            catalog,
            FlakContinuation {
                source_uid,
                step_index,
                remaining: &frozen,
                damage: i32::try_from(damage)
                    .map_err(|_| EngineRefusal::CounterOverflow("flak damage"))?,
                frozen_count: u32::try_from(frozen_count)
                    .map_err(|_| EngineRefusal::CounterOverflow("flak status count"))?,
            },
            events,
        )
    }

    let mut probe = ctx.state.clone();
    apply(
        &mut probe,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        *damage,
        &mut Vec::new(),
    )?;
    apply(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
        *damage,
        ctx.events,
    )
}

#[derive(Clone, Copy)]
struct FlakContinuation<'a> {
    source_uid: u32,
    step_index: u32,
    remaining: &'a [u32],
    damage: i32,
    frozen_count: u32,
}

/// `FlakCannon/<OnPlay>d__6` RVA `0x39ed78` awaits `CardCmd.Exhaust` per frozen
/// Status (IL_008c), and `<Exhaust>d__6` (`0x3e06c8`) returns at
/// `IsOverOrEnding` (IL_0025-002a) before its `CardPileCmd.Add`. An exhaust
/// listener that ends the combat therefore stops the walk before the next card
/// leaves Hand: the shared IsOverOrEnding projection, not `history.over`
/// (#3515). The attack after the walk carries its own entry return.
fn continue_flak_cannon(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    continuation: FlakContinuation<'_>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let FlakContinuation {
        source_uid,
        step_index,
        remaining,
        damage,
        frozen_count,
    } = continuation;
    for (cursor, uid) in remaining.iter().copied().enumerate() {
        if crate::engine::damage::damage_combat_is_ending(state) {
            break;
        }
        let (pile, index) = match unique_live_card_location(state, uid)? {
            Some(location) if location.0 != PileId::Exhaust => location,
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        let card = state.piles.get_mut(pile).make_mut().remove(index);
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
            crate::hot::AfterCardExhaustedReturnKind::Flak,
        );
        continuation.source_uid = Some(source_uid);
        continuation.step_index = step_index;
        continuation.amount = damage;
        continuation.count = frozen_count;
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
        let (pile, source_index) =
            crate::engine::play::unique_live_card_location(state, source_uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[source_index];
        let spec = catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
        player_attack_random_from_card(
            state,
            (catalog, spec, source_uid),
            i64::from(damage),
            i64::from(frozen_count),
            events,
        )?;
    }
    Ok(())
}

pub(crate) fn resume_flak_after_exhaust(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let source_uid = record
        .source_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if !record.remaining.is_empty() {
        state
            .frames
            .pop_top_after_card_exhausted_power()
            .filter(|completed| *completed == *record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        return continue_flak_cannon(
            state,
            catalog,
            FlakContinuation {
                source_uid,
                step_index: record.step_index,
                remaining: &record.remaining,
                damage: record.amount,
                frozen_count: record.count,
            },
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
        let (pile, source_index) =
            crate::engine::play::unique_live_card_location(state, source_uid)?
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[source_index];
        let spec = catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
        player_attack_random_from_card(
            state,
            (catalog, spec, source_uid),
            i64::from(record.amount),
            i64::from(record.count),
            events,
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
            completed.return_kind == crate::hot::AfterCardExhaustedReturnKind::Flak
                && completed.card_uid == record.card_uid
                && (state.history.over || completed.flags == 1)
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    Ok(())
}

fn genetic_algorithm_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> Option<i32> {
    let expected = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::GeneticAlgorithm, 0) => 3,
        (CardId::GeneticAlgorithm, 1) => 4,
        _ => return None,
    };
    let [block, growth] = catalog.steps(spec) else {
        return None;
    };
    (crate::engine::play::body_enchantment_is_exact(spec)
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
        && block.kind == StepKind::GeneticAlgorithmBlockExact
        && catalog.args(block.args).is_empty()
        && growth.kind == StepKind::GeneticAlgorithmGrowthExact
        && catalog.args(growth.args) == [CompiledArg::I(i64::from(expected))])
    .then_some(expected)
}

fn genetic_algorithm_source(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<HotCard, EngineRefusal> {
    let (pile, index) = unique_live_card_location(state, source_uid)?.ok_or(
        EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        },
    )?;
    let source = state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0
        || catalog.spec(source.atom) != Some(spec)
        || genetic_algorithm_program_is_exact(catalog, spec).is_none()
    {
        return Err(EngineRefusal::MalformedArgs(
            "genetic_algorithm physical source",
        ));
    }
    Ok(source)
}

fn validate_genetic_algorithm_relation(
    state: &crate::hot::HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    let mut rows = Vec::new();
    for pile in PileId::ALL {
        for card in state.piles.get(pile).as_slice() {
            if card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE == 0 {
                continue;
            }
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.identity.id != CardId::GeneticAlgorithm {
                return Err(EngineRefusal::MalformedArgs(
                    "genetic_algorithm state owner",
                ));
            }
            if let Some(row) = state.card_states.get(card.uid).genetic_algorithm.deck_row() {
                if rows.contains(&row) {
                    return Err(EngineRefusal::MalformedArgs(
                        "genetic_algorithm duplicate master row",
                    ));
                }
                rows.push(row);
            }
        }
    }
    for (uid, instance) in state.card_states.as_slice() {
        if instance.genetic_algorithm == crate::hot::GeneticAlgorithmState::default() {
            continue;
        }
        let owners = PileId::ALL
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .filter(|card| card.uid == *uid && card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0)
            .count();
        if owners != 1 {
            return Err(EngineRefusal::MalformedArgs(
                "genetic_algorithm orphan state",
            ));
        }
    }
    Ok(())
}

fn apply_genetic_algorithm_growth(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    increase: i32,
) -> Result<(), EngineRefusal> {
    let source = genetic_algorithm_source(state, catalog, source_uid, spec)?;
    validate_genetic_algorithm_relation(state, catalog)?;
    let mut instance = state.card_states.get(source.uid);
    let next = instance
        .genetic_algorithm
        .growth()
        .checked_add(increase)
        .ok_or(EngineRefusal::CounterOverflow(
            "genetic algorithm block growth",
        ))?;
    instance.genetic_algorithm =
        instance
            .genetic_algorithm
            .with_growth(next)
            .ok_or(EngineRefusal::CounterOverflow(
                "genetic algorithm block growth",
            ))?;
    crate::engine::monsters::update_hopper_genetic_algorithm_master(
        state,
        source.uid,
        &state.card_states.get(source.uid),
        &instance,
    )?;
    state.card_states.set(source.uid, instance);
    for pile in PileId::ALL {
        if let Some(card) = state
            .piles
            .get_mut(pile)
            .make_mut()
            .iter_mut()
            .find(|card| card.uid == source.uid)
        {
            card.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE;
            break;
        }
    }
    if !state.exact_piles && crate::engine::cards::live_cards_need_exact_piles(state, catalog)? {
        state.exact_piles = true;
    }
    Ok(())
}

/// Authenticate Genetic Algorithm's full physical program and rehearse its
/// first guaranteed awaited Block plus persistent mutation before the public
/// play spends any resource or publishes an event. Later generated bodies
/// are created only after the prior AfterCardPlayed pass; their fallible
/// suffix runs under the public action's late-body transaction.
///
/// Python oracle: `_genetic_algorithm_block_exact` (frozen Python, deleted #2827) and
/// `_genetic_algorithm_growth_exact`.
pub(crate) fn preflight_genetic_algorithm_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    let source = genetic_algorithm_source(state, catalog, source_uid, spec)?;
    validate_genetic_algorithm_relation(state, catalog)?;
    let increase = genetic_algorithm_program_is_exact(catalog, spec)
        .ok_or(EngineRefusal::MalformedArgs("genetic_algorithm program"))?;
    let mut probe = state.clone();
    let mut events = Vec::new();
    let growth = probe.card_states.get(source.uid).genetic_algorithm.growth();
    gain_powered_card_block(
        &mut probe,
        catalog,
        spec,
        1_i64
            .checked_add(i64::from(growth))
            .ok_or(EngineRefusal::CounterOverflow(
                "genetic algorithm current block",
            ))?,
        &mut events,
    )?;
    apply_genetic_algorithm_growth(&mut probe, catalog, source.uid, spec, increase)?;
    Ok(())
}

/// Genetic Algorithm's exact live mutable Block command.
///
/// Python dispatch: `_run_steps_inner` (frozen Python, deleted #2827).
///
/// Current v0.111.0 native authority (ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `GeneticAlgorithm/<OnPlay>d__16::MoveNext` RVA `0x3a1118` reads the active
/// object's Block dynamic var at IL `0x001d..0x003f` and awaits GainBlock at
/// IL `0x0044..0x0094` before any mutation. The ctor/canonical/upgrade RVAs
/// `0xe0feb`/`0xe1034`/`0xe10d3` pin raw Block `1 + growth` and L0/L1 growth
/// operands 3/4.
pub(crate) fn genetic_algorithm_block_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() || ctx.target.is_some() || ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "genetic_algorithm_block_exact",
        ));
    }
    let source = genetic_algorithm_source(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let growth = ctx
        .state
        .card_states
        .get(source.uid)
        .genetic_algorithm
        .growth();
    let raw_block = 1_i64
        .checked_add(i64::from(growth))
        .ok_or(EngineRefusal::CounterOverflow(
            "genetic algorithm current block",
        ))?;
    // This is the awaited native command boundary. Rehearse both this Block
    // and the following unconditional mutation against the exact state that
    // exists after every BeforeCardPlayed listener. A refusal therefore
    // cannot publish a partial Block event before the paired growth step.
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    gain_powered_card_block(
        &mut probe,
        ctx.catalog,
        ctx.spec,
        raw_block,
        &mut probe_events,
    )?;
    let increase = genetic_algorithm_program_is_exact(ctx.catalog, ctx.spec)
        .ok_or(EngineRefusal::MalformedArgs("genetic_algorithm program"))?;
    apply_genetic_algorithm_growth(&mut probe, ctx.catalog, source.uid, ctx.spec, increase)?;
    gain_powered_card_block(ctx.state, ctx.catalog, ctx.spec, raw_block, ctx.events)?;
    Ok(())
}

/// Genetic Algorithm's unconditional post-Block persistent mutation.
///
/// Python dispatch: `_run_steps_inner` (frozen Python, deleted #2827).
///
/// The same current-build `MoveNext` RVA `0x3a1118` re-reads Increase at IL
/// `0x0095..0x00aa`, mutates the exact combat object at `0x00ab..0x00ad`, then
/// type-tests its nullable DeckVersion and mutates that same master object at
/// `0x00b2..0x00c4`. The hot relation is the exact opaque card-tail row plus
/// the derived, sorted `genetic_algorithm_deck_growth` projection; mutable
/// clones keep growth and clear only the row token.
pub(crate) fn genetic_algorithm_growth_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected = genetic_algorithm_program_is_exact(ctx.catalog, ctx.spec).ok_or(
        EngineRefusal::MalformedArgs("genetic_algorithm_growth_exact program"),
    )?;
    if ctx.args != [CompiledArg::I(i64::from(expected))]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "genetic_algorithm_growth_exact",
        ));
    }
    apply_genetic_algorithm_growth(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec, expected)
}

/// `imitation_learning_exact` — exact target-keyed Power replay (#1562).
///
/// Python: `_run_steps_inner`'s branch (frozen Python, deleted #2827) (Batch170).
///
/// Rust preserves the ordered `(PlayerKey, amount)` power table. A local
/// owner's first Power play allocates one frozen physical clone before the
/// source body, then the AfterCardPlayed callback decrements and recursively
/// AutoPlays it. An external teammate callback remains a named continuation
/// refusal before mutation rather than a synthesized teammate action.
pub(crate) fn imitation_learning_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("imitation_learning_exact"));
    };
    if ctx.spec.identity.id != CardId::ImitationLearning
        || *amount != 2 + i64::from(ctx.spec.identity.upgrade)
    {
        return Err(EngineRefusal::MalformedArgs("imitation_learning_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("imitation_learning_exact"))?;
    crate::engine::allies::apply_imitation_learning(ctx.state, key, amount)
}

/// One For All's exact owner-ordered all-Player power fan-out.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `OneForAll/<OnPlay>d__8::MoveNext` (RVA `0x3b036c`) enumerates
/// `ICombatState.Players` in roster order (IL_00b2-IL_00d8) and serially
/// awaits `PowerCmd.Apply<OneForAllPower>(player, DynamicVars.BaseDamage,
/// owner, cardSource, false)` (IL_0124-IL_0194). The generic list overload
/// `PowerCmd/<Apply>d__0<T>::MoveNext` (RVA `0x3ef7dc`) is likewise serial;
/// the single-target body (RVA `0x3ef988`) no-ops while combat is ending or
/// the recipient cannot receive powers. `OneForAll` ctor/constraint/vars are
/// RVAs `0xe6c57`/`0xe6c64`/`0xe6c67`; `Upgrade` is `0xe6cc7`.
/// Python: `_run_steps_inner` (frozen, deleted #2827) independently pins the two exact
/// rows, `CardPlayFrame` source uid, and `AllAllies` fan-out call; native is
/// the semantic authority for the source-type-neutral damage reader (#1796).
///
/// The complete current table is exactly the Defect Rare Power at cost one,
/// AllAllies, with a sole operand 3/4. The synchronous active-play registry
/// authenticates Python's `CardPlayFrame` source uid; suspended selections
/// serialize the same uid and resource fields in `PendingSelection`.
fn one_for_all_program_amount(catalog: &Catalog, spec: &CardSpec) -> Result<i32, EngineRefusal> {
    let [step] = catalog.steps(spec) else {
        return Err(EngineRefusal::MalformedArgs("one_for_all_exact program"));
    };
    let amount = match (
        spec.identity.id,
        spec.identity.upgrade,
        catalog.args(step.args),
    ) {
        (CardId::OneForAll, 0, [CompiledArg::I(3)]) => 3,
        (CardId::OneForAll, 1, [CompiledArg::I(4)]) => 4,
        _ => return Err(EngineRefusal::MalformedArgs("one_for_all_exact")),
    };
    if step.kind != StepKind::OneForAllExact
        || crate::content_tables::card_row(CardId::OneForAll, spec.identity.upgrade)
            != Some(spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("one_for_all_exact program"));
    }
    Ok(amount)
}

/// Prove one body's complete fallible fan-out before it mutates any recipient.
/// The public play layer checkpoints One For All across Signal Boost/Echo
/// bodies so a later replay overflow also rolls back the shared play prefix.
pub(crate) fn preflight_one_for_all_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    let amount = one_for_all_program_amount(catalog, spec)?;
    let (_, _, active) = super::physical_cost::validate_active_physical_source(
        state,
        catalog,
        source_uid,
        CardId::OneForAll,
    )?;
    if catalog.spec(active.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs("one_for_all_exact source"));
    }
    // The overflow preflight is vacuous where `Apply` itself returns
    // (IsEnding, see `allies::add_one_for_all`).
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    for key in crate::engine::allies::living_keys(state) {
        match key {
            0 => state
                .powers
                .value(PowerId::OneForAll)
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("one_for_all"))?,
            1 => state
                .fanouts
                .multiplayer_ally()
                .one_for_all
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("remote one_for_all"))?,
            _ => return Err(EngineRefusal::MalformedArgs("One For All player key")),
        };
    }
    Ok(())
}

pub(crate) fn one_for_all_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_for_all_program_amount(ctx.catalog, ctx.spec)?;
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.args != [CompiledArg::I(i64::from(amount))]
    {
        return Err(EngineRefusal::MalformedArgs("one_for_all_exact program"));
    }
    let (_, _, active) = super::physical_cost::validate_active_physical_source(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
        CardId::OneForAll,
    )?;
    if ctx.catalog.spec(active.atom) != Some(ctx.spec)
        || crate::engine::play::active_card_play_index(ctx.source_uid).is_none()
    {
        return Err(EngineRefusal::MalformedArgs("one_for_all_exact source"));
    }
    if ctx.state.history.over {
        return Ok(());
    }
    let keys = crate::engine::allies::living_keys(ctx.state).collect::<Vec<_>>();
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    for key in keys.iter().copied() {
        crate::engine::allies::add_one_for_all(&mut probe, key, amount, &mut probe_events)?;
    }
    for key in keys {
        crate::engine::allies::add_one_for_all(ctx.state, key, amount, ctx.events)?;
    }
    Ok(())
}

/// Reboot's exact serialized program and frozen physical source contract.
///
/// Native v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Reboot/<OnPlay>d__5::MoveNext` (RVA `0x3b6134`) snapshots Hand
/// (IL_00ab..IL_00bc `GetPile(Hand).Cards.ToList`), serially awaits
/// `CardPileCmd.Add(card, Draw, Bottom)` for each snapshot entry (IL_00e3),
/// then awaits `CardPileCmd.Shuffle` (IL_017e) and finally
/// `CardPileCmd.Draw(Cards)` (IL_01f8). Shuffle
/// (`CardPileCmd/<Shuffle>d__22::MoveNext`, RVA `0x3e4b74`) is the command
/// Bottled Potential also runs: it snapshots Discard, appends the live Draw
/// object set (IL_0098 `ToHashSet`), StableShuffles on `RunRngSet.Shuffle`
/// (IL_00c4), calls `Hook::ModifyShuffleOrder` (IL_00e2), moves the result to
/// Draw, and only then dispatches `AfterShuffle`. R52D proves that the
/// bundled .NET 9 fresh reference HashSet enumerates its never-removed
/// entries in live Draw insertion order, so duplicate `(CardId, upgrade)`
/// keys are exact for Reboot exactly as for Bottled Potential (#3197),
/// whether the tied payloads are equal or distinct.
/// Perfect Fit's pre-commit `ModifyShuffleOrder` listener remains refused for
/// both full-shuffle callers.
fn reboot_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    let [shuffle, draw] = catalog.steps(spec) else {
        return false;
    };
    let expected_draw = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::Reboot, 0) => 4,
        (CardId::Reboot, 1) => 6,
        _ => return false,
    };
    crate::engine::play::body_enchantment_is_exact(spec)
        && shuffle.kind == StepKind::RebootShuffleExact
        && catalog.args(shuffle.args).is_empty()
        && draw.kind == StepKind::Draw
        && catalog.args(draw.args) == [CompiledArg::I(expected_draw)]
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
}

fn reboot_has_no_perfect_fit(state: &crate::hot::HotState, catalog: &Catalog) -> bool {
    !PileId::ALL.into_iter().any(|pile| {
        state.piles.get(pile).as_slice().iter().any(|card| {
            catalog.spec(card.atom).is_some_and(|spec| {
                spec.identity
                    .enchantment
                    .is_some_and(|value| matches!(value.id, crate::ids::EnchantmentId::PerfectFit))
            })
        })
    })
}

/// Bottled Potential's only remaining full-shuffle exclusion is Perfect Fit.
///
/// `CardPileCmd.Shuffle` (RVA `0x3e4b74`) appends a fresh reference-hashed
/// Draw `ToHashSet` to ordered Discard. The bundled .NET 9.0.7 runtime
/// (CoreCLR 9.0.725.31616, dotnet/runtime commit `3c298d9f...`) appends every
/// unique reference into `_entries` and enumerates the never-removed entries
/// in insertion order. `CardModel` has no equality/hash override, so exact
/// pile order and physical UIDs authenticate even distinguishable CompareTo
/// ties. Perfect Fit can still rewrite the sorted order before commit and is
/// not modeled.
pub(crate) fn bottled_potential_entry_is_valid(
    state: &crate::hot::HotState,
    catalog: &Catalog,
) -> bool {
    reboot_has_no_perfect_fit(state, catalog)
}

/// Entry validity for a live physical Reboot: like Bottled Potential, its
/// only remaining full-shuffle exclusion is Perfect Fit (#3197 retired the
/// duplicate-key quotient).
pub(crate) fn reboot_entry_is_valid(state: &crate::hot::HotState, catalog: &Catalog) -> bool {
    !reboot_physical_source_is_reachable(state, catalog)
        || reboot_has_no_perfect_fit(state, catalog)
}

/// Whether one non-Exhaust physical Reboot can still enter Play.
///
/// Current-build generated-card pools contain no Reboot, and every exact
/// clone writer is source-typed so it cannot synthesize this Skill from a
/// different identity. Admission pins that closed-world fact separately;
/// an immutable catalog row by itself is therefore not executable evidence.
pub(crate) fn reboot_physical_source_is_reachable(
    state: &crate::hot::HotState,
    catalog: &Catalog,
) -> bool {
    [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play]
        .into_iter()
        .any(|pile| {
            state.piles.get(pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::Reboot))
            })
        })
}

pub(crate) fn preflight_reboot_shuffle_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if !reboot_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs("reboot_shuffle_exact program"));
    }
    let (_, _, active) = super::physical_cost::validate_active_physical_source(
        state,
        catalog,
        source_uid,
        CardId::Reboot,
    )?;
    if catalog.spec(active.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs("reboot_shuffle_exact source"));
    }
    if !reboot_has_no_perfect_fit(state, catalog) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

pub(crate) fn reboot_shuffle_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !ctx.args.is_empty()
        || !reboot_program_is_exact(ctx.catalog, ctx.spec)
    {
        return Err(EngineRefusal::MalformedArgs("reboot_shuffle_exact"));
    }
    let (_, _, active) = super::physical_cost::validate_active_physical_source(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
        CardId::Reboot,
    )?;
    if ctx.catalog.spec(active.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("reboot_shuffle_exact source"));
    }
    let frozen_hand = ctx.state.piles.get(PileId::Hand).as_slice().to_vec();
    crate::engine::draw::reboot_shuffle(ctx.state, ctx.catalog, &frozen_hand, ctx.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardAtom, CardEnchantment, CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::play::{autoplay_draw_top, play_card};
    use crate::engine::{Action, SelectionRef, StepCtx, apply_action_into};
    use crate::hot::{
        CardInstanceState, HopperDeckRow, HopperDeckState, HotMonster, HotOrb, HotState,
        MultiplayerAllyState, OrbKind, RngStream, RngStreamState,
    };
    use crate::ids::{EnchantmentId, MonsterKind, PowerId, StepKind};
    use crate::powers::SlotWire;

    /// Every kind this file owns a body for.
    const OWNED: [StepKind; 11] = [
        StepKind::ActiveCardCostAddExact,
        StepKind::AdaptiveStrikeExact,
        StepKind::AllForOneExact,
        StepKind::ConsumingShadowExact,
        StepKind::CreativeAi,
        StepKind::FlakCannonExact,
        StepKind::GeneticAlgorithmBlockExact,
        StepKind::GeneticAlgorithmGrowthExact,
        StepKind::ImitationLearningExact,
        StepKind::OneForAllExact,
        StepKind::RebootShuffleExact,
    ];

    /// A claim here must be a kind this file owns, and a kind it owns must be
    /// claimed here or nowhere.
    ///
    /// This replaces #1321's `implemented_kinds_carry_an_admission_arg_shape`,
    /// which pinned a wall that no longer exists: #1366 made every wave kind's
    /// argument shape **body-owned**, so `admission::step_args` no longer has
    /// an opinion outside the R0.5 three and listing a kind here no longer
    /// turns its cards into `ArgumentShape` refusals. What is worth pinning
    /// instead is the D4 property the family split rests on.
    #[test]
    fn the_family_claims_only_kinds_it_owns() {
        for kind in IMPLEMENTED {
            assert!(
                OWNED.contains(kind),
                "{:?} is claimed here but is not one of this file's stubs",
                kind.as_str()
            );
        }
        for kind in OWNED {
            assert_eq!(
                crate::steps::is_implemented(kind),
                IMPLEMENTED.contains(&kind),
                "{:?} is implemented somewhere other than the family that owns it",
                kind.as_str()
            );
        }
    }

    #[test]
    fn bottled_potential_accepts_distinguishable_routable_and_returning_siblings() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let exhaust_returners = [
            CardId::MakeItSo,
            CardId::HowlFromBeyond,
            CardId::Bombardment,
        ]
        .map(|id| {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap()
        });
        let catalog = builder.build();

        let mut play = HotState::at_defaults();
        play.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: strike,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        play.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 2,
            atom: strike,
            flags: 0,
        });
        // Fresh reference-identity HashSet enumeration preserves both equal
        // physical cards in live pile order, even when their payload flags
        // differ. Exact UIDs make the pair distinguishable without a tie wall.
        assert!(bottled_potential_entry_is_valid(&play, &catalog));
        play.piles.get_mut(PileId::Play).make_mut()[0].flags =
            crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        assert!(bottled_potential_entry_is_valid(&play, &catalog));

        for atom in exhaust_returners {
            let mut exhaust_return = HotState::at_defaults();
            exhaust_return
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(HotCard {
                    uid: 3,
                    atom,
                    flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
            exhaust_return
                .piles
                .get_mut(PileId::Exhaust)
                .make_mut()
                .push(HotCard {
                    uid: 4,
                    atom,
                    flags: 0,
                });
            assert!(bottled_potential_entry_is_valid(&exhaust_return, &catalog));
        }

        let mut unrelated_exhaust = play;
        let unrelated = unrelated_exhaust
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .pop()
            .unwrap();
        unrelated_exhaust
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(HotCard {
                flags: 0,
                ..unrelated
            });
        assert!(bottled_potential_entry_is_valid(
            &unrelated_exhaust,
            &catalog
        ));
    }

    #[test]
    fn one_for_all_levels_fan_out_in_owner_order_and_signal_boost_replays() {
        for (upgrade, per_play) in [(0, 3), (1, 4)] {
            let identity = CardIdentity {
                id: CardId::OneForAll,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.energy = 3;
            state.multiplayer_ally_key = 1;
            state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
                key: 1,
                ..MultiplayerAllyState::default()
            });
            state.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();

            assert_eq!(state.powers.value(PowerId::OneForAll), per_play * 2);
            assert_eq!(state.fanouts.multiplayer_ally().one_for_all, per_play * 2);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        Event::PowerChanged {
                            subject: crate::engine::Subject::Player,
                            power: PowerId::OneForAll,
                            ..
                        }
                    ))
                    .count(),
                2
            );
        }
    }

    #[test]
    fn one_for_all_skips_dead_players_and_remote_overflow_is_atomic() {
        let identity = CardIdentity {
            id: CardId::OneForAll,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut dead_remote = HotState::at_defaults();
        dead_remote.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        dead_remote.hp = 50;
        dead_remote.energy = 3;
        dead_remote.multiplayer_ally_key = 1;
        dead_remote
            .fanouts
            .set_multiplayer_ally(MultiplayerAllyState {
                key: 1,
                alive: false,
                ..MultiplayerAllyState::default()
            });
        dead_remote
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
        autoplay_draw_top(&mut dead_remote, &catalog, 1, &mut Vec::new()).unwrap();
        assert_eq!(dead_remote.powers.value(PowerId::OneForAll), 4);
        assert_eq!(dead_remote.fanouts.multiplayer_ally().one_for_all, 0);

        let mut overflow = HotState::at_defaults();

        overflow.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        overflow.hp = 50;
        overflow.energy = 3;
        overflow.multiplayer_ally_key = 1;
        overflow.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            one_for_all: i32::MAX,
            ..MultiplayerAllyState::default()
        });
        overflow
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 8,
                atom,
                flags: 0,
            });
        let before = overflow.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let before_events = events.clone();
        assert_eq!(
            play_card(&mut overflow, &catalog, 8, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("remote one_for_all"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn one_for_all_signal_boost_late_overflow_rolls_back_the_whole_play() {
        let identity = CardIdentity {
            id: CardId::OneForAll,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        for remote_overflow in [false, true] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.energy = 3;
            state.multiplayer_ally_key = 1;
            state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
                key: 1,
                one_for_all: if remote_overflow { i32::MAX - 3 } else { 0 },
                ..MultiplayerAllyState::default()
            });
            state.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            state.powers.set(
                PowerId::OneForAll,
                SlotWire::Int,
                if remote_overflow { 0 } else { i32::MAX - 3 },
            );
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 99 }];
            let before_events = events.clone();

            assert_eq!(
                play_card(&mut state, &catalog, 7, None, None, &mut events),
                Err(EngineRefusal::CounterOverflow(if remote_overflow {
                    "remote one_for_all"
                } else {
                    "one_for_all"
                }))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn one_for_all_manual_solo_refuses_but_direct_autoplay_is_owner_only() {
        let identity = CardIdentity {
            id: CardId::OneForAll,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut manual = HotState::at_defaults();
        manual.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        manual.hp = 50;
        manual.energy = 3;
        manual
            .powers
            .set(PowerId::OneForAll, SlotWire::Int, i32::MAX);
        manual.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let before = manual.clone();
        assert_eq!(
            play_card(&mut manual, &catalog, 7, None, None, &mut Vec::new()),
            Err(EngineRefusal::CardNotPlayableSolo(CardId::OneForAll))
        );
        assert_eq!(manual, before);

        let mut autoplay = HotState::at_defaults();

        autoplay.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        autoplay.hp = 50;
        autoplay.energy = 3;
        autoplay
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 8,
                atom,
                flags: 0,
            });
        autoplay_draw_top(&mut autoplay, &catalog, 1, &mut Vec::new()).unwrap();
        assert_eq!(autoplay.powers.value(PowerId::OneForAll), 3);
        assert_eq!(autoplay.energy, 3);
    }

    /// #3502: the overflow preflight is vacuous where `PowerCmd.Apply`
    /// returns (`<Apply>d__1`1` `0x3ef988` IL_0020-0034, IsEnding): with the
    /// only enemy dead before the over latch an overflowing amount no longer
    /// refuses, while the live control does.
    #[test]
    fn one_for_all_preflight_skips_while_the_combat_is_ending_before_the_over_latch() {
        let identity = CardIdentity {
            id: CardId::OneForAll,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let exact = *catalog.spec(atom).unwrap();
        for ending in [false, true] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(HotMonster::new(
                MonsterKind::Toadpole,
                if ending { 0 } else { 100 },
            ));
            state.hp = 50;
            state
                .powers
                .set(PowerId::OneForAll, SlotWire::Int, i32::MAX);
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            assert!(!state.history.over);
            assert_eq!(
                crate::engine::damage::damage_combat_is_ending(&state),
                ending
            );
            assert_eq!(
                preflight_one_for_all_exact(&state, &catalog, 7, &exact),
                if ending {
                    Ok(())
                } else {
                    Err(EngineRefusal::CounterOverflow("one_for_all"))
                }
            );
        }
    }

    #[test]
    fn one_for_all_validates_source_before_history_over_noop() {
        let identity = CardIdentity {
            id: CardId::OneForAll,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let exact = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.history.over = true;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = Vec::new();
        crate::engine::play::with_test_active_play_spent(7, 1, || {
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &exact,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(3)],
                events: &mut events,
            };
            one_for_all_exact(&mut ctx).unwrap();
        });
        assert_eq!(state, before);
        assert!(events.is_empty());

        let mut forged = exact;
        forged.row = crate::content_tables::card_row(CardId::Buffer, 0).unwrap();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &forged,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(3)],
            events: &mut events,
        };
        assert_eq!(
            one_for_all_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("one_for_all_exact program"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn creative_ai_public_levels_apply_one_and_register_once() {
        for upgrade in 0..=1 {
            let identity = CardIdentity {
                id: CardId::CreativeAi,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            for id in crate::content_tables::CREATIVE_AI_POWER_POOL_V1101 {
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
            state.energy = 9;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Defect);
            state.set_creative_ai_generation_pool(true);
            state.fully_unlocked_card_pool_epochs = true;
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Hand).make_mut().extend([
                HotCard {
                    uid: 7,
                    atom,
                    flags: 0,
                },
                HotCard {
                    uid: 8,
                    atom,
                    flags: 0,
                },
            ]);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();
            play_card(&mut state, &catalog, 8, None, None, &mut events).unwrap();

            assert_eq!(state.powers.value(PowerId::CreativeAi), 2);
            assert_eq!(
                state.fanouts.before_hand_draw_order(),
                [PowerId::CreativeAi]
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        Event::PowerChanged {
                            power: PowerId::CreativeAi,
                            ..
                        }
                    ))
                    .count(),
                2
            );
        }
    }

    #[test]
    fn creative_ai_direct_play_provenance_refusal_rolls_back_prefix() {
        let identity = CardIdentity {
            id: CardId::CreativeAi,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let before_events = events.clone();

        assert_eq!(
            play_card(&mut state, &catalog, 7, None, None, &mut events),
            Err(EngineRefusal::MalformedArgs("creative_ai"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn consuming_shadow_public_levels_channel_then_apply() {
        for (upgrade, channels) in [(0, 2usize), (1, 3)] {
            let identity = CardIdentity {
                id: CardId::ConsumingShadow,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 3;
            state.orbs.set_base_slots(3);
            state.orbs.set_slots(3);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            let rng_before = state.rng.get(RngStream::CombatOrbs);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();

            assert_eq!(state.orbs.next_random_orb_progress_uid(), channels as u32);
            assert_eq!(state.rng.get(RngStream::CombatOrbs), rng_before);
            assert_eq!(state.orbs.as_slice().len(), channels);
            assert!(
                state
                    .orbs
                    .as_slice()
                    .iter()
                    .all(|orb| orb.kind() == crate::hot::OrbKind::Dark)
            );
            assert_eq!(state.powers.value(PowerId::ConsumingShadow), 1);
            assert!(state.piles.get(PileId::Play).is_empty());
        }
    }

    #[test]
    fn consuming_shadow_public_late_orb_listener_refusal_is_atomic() {
        let identity = CardIdentity {
            id: CardId::ConsumingShadow,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::TheLost).unwrap();
        builder.intern_monster(MonsterKind::TheForgotten).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(1);
        state.orbs.set_slots(1);
        state
            .orbs
            .push(HotOrb::from_parts(OrbKind::Dark, Some(6)).unwrap());
        let mut lost = HotMonster::new(MonsterKind::TheLost, crate::engine::monsters::THE_LOST_HP);
        lost.max_hp = crate::engine::monsters::THE_LOST_HP;
        lost.hp = 1;
        lost.possess_strength_debit = i32::MIN;
        let mut forgotten = HotMonster::new(
            MonsterKind::TheForgotten,
            crate::engine::monsters::THE_FORGOTTEN_HP,
        );
        forgotten.max_hp = crate::engine::monsters::THE_FORGOTTEN_HP;
        forgotten.slot = 1;
        forgotten.uid = 1;
        state.monsters_mut().extend([lost, forgotten]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 8,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = vec![crate::engine::Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();

        assert_eq!(
            play_card(&mut state, &catalog, 8, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("Possess Strength restore"))
        );
        assert_eq!(
            state, before,
            "energy, source, history, and orb state are atomic"
        );
        assert_eq!(
            events, before_events,
            "late damage/death-listener events roll back"
        );
    }

    #[test]
    fn catastrophe_auto_plays_consuming_shadow_from_draw() {
        let catastrophe = CardIdentity {
            id: CardId::Catastrophe,
            upgrade: 0,
            enchantment: None,
        };
        let shadow = CardIdentity {
            id: CardId::ConsumingShadow,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let catastrophe_atom = builder.intern(catastrophe).unwrap();
        let shadow_atom = builder.intern(shadow).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 40,
            atom: catastrophe_atom,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 41,
            atom: shadow_atom,
            flags: 0,
        });
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 40, None, None, &mut events).unwrap();

        assert_eq!(state.orbs.next_random_orb_progress_uid(), 2);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 0);
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(state.powers.value(PowerId::ConsumingShadow), 1);
    }

    #[test]
    fn consuming_shadow_forged_carrier_program_refuses_atomically() {
        let identity = CardIdentity {
            id: CardId::ConsumingShadow,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut spec = *catalog.spec(atom).unwrap();
        spec.row = crate::content_tables::card_row(CardId::Buffer, 0).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 9,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };

        assert_eq!(
            consuming_shadow_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("consuming_shadow_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn modded_has_exactly_two_generated_carriers_and_one_new_manifest_kind() {
        let carriers = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::ActiveCardCostAddExact)
            })
            .collect::<Vec<_>>();
        assert_eq!(carriers.len(), 2);
        for (row, upgrade, draw) in [(carriers[0], 0, 1), (carriers[1], 1, 2)] {
            assert_eq!((row.id, row.upgrade), (CardId::Modded, upgrade));
            assert_eq!(row.cost, 0);
            assert!(row.is_skill);
            assert!(!row.targeted);
            assert_eq!(row.target_type, "Self");
            assert_eq!(
                row.steps
                    .iter()
                    .map(|step| (step.kind, step.args.to_vec()))
                    .collect::<Vec<_>>(),
                vec![
                    (StepKind::AddOrbSlotsExact, vec![Arg::I(1)]),
                    (StepKind::Draw, vec![Arg::I(draw)]),
                    (StepKind::ActiveCardCostAddExact, vec![Arg::I(1)]),
                ]
            );
        }
        assert_eq!(
            IMPLEMENTED
                .iter()
                .filter(|kind| **kind == StepKind::ActiveCardCostAddExact)
                .count(),
            1
        );
    }

    #[test]
    fn reboot_has_exactly_two_generated_carriers_and_one_manifest_kind() {
        let carriers = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::RebootShuffleExact)
            })
            .collect::<Vec<_>>();
        assert_eq!(carriers.len(), 2);
        for (row, upgrade, draw) in [(carriers[0], 0, 4), (carriers[1], 1, 6)] {
            assert_eq!((row.id, row.upgrade), (CardId::Reboot, upgrade));
            assert_eq!(row.cost, 0);
            assert!(row.is_skill && row.exhausts);
            assert!(!row.targeted);
            assert_eq!(row.target_type, "Self");
            assert_eq!(
                row.steps
                    .iter()
                    .map(|step| (step.kind, step.args.to_vec()))
                    .collect::<Vec<_>>(),
                vec![
                    (StepKind::RebootShuffleExact, vec![]),
                    (StepKind::Draw, vec![Arg::I(draw)]),
                ]
            );
        }
        assert_eq!(
            IMPLEMENTED
                .iter()
                .filter(|kind| **kind == StepKind::RebootShuffleExact)
                .count(),
            1
        );
    }

    fn modded_catalog(upgrade: u8) -> (Catalog, CardAtom, CardAtom, CardAtom) {
        let mut builder = CatalogBuilder::new();
        let modded = builder
            .intern(CardIdentity {
                id: CardId::Modded,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        (builder.build(), modded, strike, defend)
    }

    fn cost_row() -> LocalCostModifier {
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: 1,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        }
    }

    #[test]
    fn modded_levels_add_slots_draw_then_append_one_persistent_source_row() {
        for (upgrade, drawn) in [(0, 1), (1, 2)] {
            let (catalog, modded, strike, defend) = modded_catalog(upgrade);
            let mut state = HotState::at_defaults();
            // A live enemy: with none, combat is ending and plural Shiv
            // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.hp = 50;
            state.energy = 3;
            state.orbs.set_base_slots(3);
            state.orbs.set_slots(3);
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 7,
                atom: modded,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                HotCard {
                    uid: 8,
                    atom: strike,
                    flags: 0,
                },
                HotCard {
                    uid: 9,
                    atom: defend,
                    flags: 0,
                },
            ]);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();

            assert_eq!(state.orbs.slots(), 4);
            assert_eq!(state.piles.get(PileId::Hand).len(), drawn);
            let routed = state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .find(|card| card.uid == 7)
                .copied()
                .unwrap();
            assert_eq!(
                state.card_states.get(7).local_cost_modifiers.as_slice(),
                [cost_row()]
            );
            assert_eq!(
                resolved_energy_cost(&state, routed, catalog.spec(modded).unwrap()),
                1
            );
            assert!(
                !state
                    .card_states
                    .cleanup_card_local_cost_modifiers(7, LocalCostExpiration::ThisTurn),
                "ThisCombat survives the turn-end cleanup mask"
            );
            assert_eq!(
                state.card_states.get(7).local_cost_modifiers.as_slice(),
                [cost_row()]
            );
        }
    }

    #[test]
    fn modded_burst_replays_the_same_uid_and_appends_without_folding() {
        let (catalog, modded, strike, defend) = modded_catalog(1);
        let mut state = HotState::at_defaults();
        // A live enemy: with none, combat is ending and plural Shiv
        // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: modded,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        for uid in 8..=11 {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom: if uid % 2 == 0 { strike } else { defend },
                flags: 0,
            });
        }
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        play_card(&mut state, &catalog, 7, None, None, &mut Vec::new()).unwrap();

        assert_eq!(state.orbs.slots(), 5);
        assert_eq!(state.piles.get(PileId::Hand).len(), 4);
        assert_eq!(
            state.card_states.get(7).local_cost_modifiers.as_slice(),
            [cost_row(), cost_row()]
        );
        assert_eq!(state.powers.value(PowerId::Burst), 0);
    }

    #[test]
    fn modded_write_is_source_local_promotes_a_live_tie_and_survives_a_winning_draw() {
        let (catalog, modded, strike, _) = modded_catalog(0);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.fanouts.set_cacophony_left(1);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 7,
                atom: modded,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 8,
                atom: modded,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 9,
            atom: strike,
            flags: 0,
        });

        play_card(&mut state, &catalog, 7, None, None, &mut Vec::new()).unwrap();

        assert!(state.history.over, "the Draw listener wins the fight");
        assert_eq!(state.orbs.slots(), 4);
        assert_eq!(
            state.card_states.get(7).local_cost_modifiers.as_slice(),
            [cost_row()],
            "enemy-side terminal does not suppress Modded's ungated command tail"
        );
        assert!(state.card_states.get(8).is_vacant());
        assert!(
            state.exact_piles,
            "the changed source now differs from its peer"
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].uid, 7);
    }

    #[test]
    fn modded_autoplay_sets_the_default_physical_marker_on_the_exact_source() {
        let (catalog, modded, strike, _) = modded_catalog(0);
        let mut state = HotState::at_defaults();
        // A live enemy: with none, combat is ending and plural Shiv
        // creation is IsOverOrEnding-gated (`0x3bb20c` IL_0031).
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 7,
                atom: modded,
                flags: 0,
            },
            HotCard {
                uid: 8,
                atom: strike,
                flags: 0,
            },
        ]);

        autoplay_draw_top(&mut state, &catalog, 1, &mut Vec::new()).unwrap();

        assert_eq!(state.orbs.slots(), 4);
        assert_eq!(state.history.manual_card_plays_finished_this_turn, 0);
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 8);
        let routed = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!(routed.uid, 7);
        assert_ne!(routed.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(
            state.card_states.get(7).local_cost_modifiers.as_slice(),
            [cost_row()]
        );
    }

    #[test]
    fn modded_append_precedes_native_after_play_expiration_cleanup() {
        let (catalog, modded, strike, _) = modded_catalog(0);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        let prefix = [
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 5,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            },
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -2,
                expiration: LocalCostExpiration::UntilPlayed,
                reduce_only: true,
            },
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: i64::MAX,
                expiration: LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            },
        ];
        for row in prefix {
            state.card_states.append_local_cost_modifier(7, row);
        }
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 7,
                atom: modded,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 8,
                atom: strike,
                flags: 0,
            },
        ]);

        autoplay_draw_top(&mut state, &catalog, 1, &mut Vec::new()).unwrap();

        let routed = state.piles.get(PileId::Discard).as_slice()[0];
        let rows = state.card_states.get(7).local_cost_modifiers;
        assert_eq!(
            rows.as_slice(),
            [prefix[0], cost_row()],
            "UntilPlayed and ThisTurnOrPlayed expire; ThisTurn and the new ThisCombat row survive"
        );
        assert_eq!(
            resolved_energy_cost(&state, routed, catalog.spec(modded).unwrap()),
            6
        );
    }

    #[test]
    fn modded_body_appends_after_arbitrary_ordered_rows_without_integer_overflow() {
        let (catalog, modded, _, _) = modded_catalog(0);
        let spec = *catalog.spec(modded).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let source = HotCard {
            uid: 7,
            atom: modded,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let prefix = [
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 5,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            },
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -2,
                expiration: LocalCostExpiration::UntilPlayed,
                reduce_only: true,
            },
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: i64::MAX,
                expiration: LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            },
        ];
        for row in prefix {
            state.card_states.append_local_cost_modifier(7, row);
        }
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };

        active_card_cost_add_exact(&mut ctx).unwrap();

        assert_eq!(
            state.card_states.get(7).local_cost_modifiers.as_slice(),
            [prefix.as_slice(), &[cost_row()]].concat()
        );
        assert_eq!(
            resolved_energy_cost(&state, source, &spec),
            i64::MAX,
            "ordered evaluation widens intermediate adds before its final clamp"
        );
        assert!(events.is_empty());
    }

    #[test]
    fn modded_shape_and_source_refusals_precede_the_cost_write() {
        let (catalog, modded, _, _) = modded_catalog(0);
        let spec = *catalog.spec(modded).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom: modded,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let baseline = state.clone();
        for (target, selection, x_value, args) in [
            (Some(0), None, 0, &[CompiledArg::I(1)][..]),
            (None, Some(9), 0, &[CompiledArg::I(1)][..]),
            (None, None, 1, &[CompiledArg::I(1)][..]),
            (None, None, 0, &[CompiledArg::I(2)][..]),
        ] {
            let mut attempt = baseline.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut attempt,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target,
                selection,
                x_value,
                args,
                events: &mut events,
            };
            assert_eq!(
                active_card_cost_add_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs("active_card_cost_add_exact"))
            );
            assert_eq!(attempt, baseline);
            assert!(events.is_empty());
        }

        let mut missing = baseline.clone();
        missing.piles.get_mut(PileId::Play).make_mut().clear();
        let missing_before = missing.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut missing,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(1)],
            events: &mut events,
        };
        assert_eq!(
            active_card_cost_add_exact(&mut ctx),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(missing, missing_before);
        assert!(events.is_empty());
    }

    #[test]
    fn modded_public_play_preflights_the_late_writer_before_slots_draw_or_events() {
        let (catalog, modded, strike, _) = modded_catalog(0);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: modded,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 8,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 7,
                atom: modded,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            play_card(&mut state, &catalog, 7, None, None, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn adaptive_strike_rereads_source_and_mints_only_the_rewritten_clone() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::AdaptiveStrike,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let source = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(18)],
            events: &mut events,
        };

        adaptive_strike_exact(&mut ctx).unwrap();

        assert_eq!(state.monsters[0].hp, 82);
        assert!(state.card_states.get(7).is_vacant());
        let clone = state.piles.get(PileId::Discard).as_slice()[0];
        assert_ne!(clone.uid, 7);
        assert_eq!(clone.atom, atom);
        assert_ne!(clone.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(
            state
                .card_states
                .get(clone.uid)
                .local_cost_modifiers
                .as_slice(),
            [LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        );
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert!(state.exact_piles);
    }

    #[test]
    fn lethal_adaptive_strike_records_generation_without_publishing_a_clone() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::AdaptiveStrike,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.history.owner_generated_cards_combat = 4;
        state.next_generated_hook_uid = 9;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(18)],
            events: &mut events,
        };

        adaptive_strike_exact(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.owner_generated_cards_combat, 5);
        assert_eq!(state.next_generated_hook_uid, 10);
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert_eq!(state.next_card_uid, 0);
        assert!(!state.exact_piles);
    }

    /// #1723 witness: in-attack generations survive the clone's record.
    /// Adaptive Strike's 18 damage is fully blocked, so Entomancer survives
    /// the powered hit and Personal Hive (`0x340220`) generates three Dazed
    /// (generated-hook token +3, owner history unchanged). The clone's
    /// `AddGeneratedCardToCombat` (Adaptive Strike IL `0x00fa`) records from
    /// the post-attack counters, so the hook token ends at C0+4. The removed
    /// call-site assignment rewound it to C0+1, erasing the generations.
    ///
    /// Before #3256 this witness ended the combat with a Smokestack answering
    /// Hive's Dazed. Hive's creator is null (IL_00cd-IL_00d1) and Smokestack
    /// leaves on a null creator (`0x3455e8` IL_0033-IL_004e), so the 1-HP
    /// Entomancer now survives and Smokestack stays silent.
    #[test]
    fn adaptive_strike_keeps_in_attack_generations_before_the_clone() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::AdaptiveStrike,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.next_card_uid = 10;
        state.next_generated_hook_uid = 20;
        state.history.owner_generated_cards_combat = 7;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });
        let mut owner = HotMonster::new(MonsterKind::Entomancer, 1);
        owner.max_hp = crate::engine::monsters::ENTOMANCER_HP;
        owner.block = 18;
        owner.loop_pos = 0;
        owner.powers.set(PowerId::Hive, SlotWire::Int, 3);
        state.monsters_mut().push(owner);
        state.powers.set(PowerId::Smokestack, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 9,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[CompiledArg::I(18)],
            events: &mut events,
        };

        adaptive_strike_exact(&mut ctx).unwrap();

        assert!(!state.history.over);
        assert_eq!(state.monsters[0].hp, 1);
        // Three Hive Dazed (uids 10-12), then the clone (uid 13) at Discard.
        assert_eq!(state.next_card_uid, 14);
        assert_eq!(state.piles.get(PileId::Discard).len(), 1);
        // Hive +3 during the attack, then the clone's record +1.
        assert_eq!(state.next_generated_hook_uid, 24);
        // Hive's null-creator Dazed are not owner history; the clone is.
        assert_eq!(state.history.owner_generated_cards_combat, 8);
    }

    #[test]
    fn flak_cannon_freezes_all_non_exhaust_statuses_and_is_atomic_on_a_late_listener_refusal() {
        let flak = CardIdentity {
            id: CardId::FlakCannon,
            upgrade: 0,
            enchantment: None,
        };
        let burn = CardIdentity {
            id: CardId::Burn,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let flak_atom = builder.intern(flak).unwrap();
        let burn_atom = builder.intern(burn).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(flak_atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom: flak_atom,
            flags: 0,
        });
        for (uid, pile) in [
            (10, PileId::Discard),
            (11, PileId::Draw),
            (12, PileId::Hand),
            (13, PileId::Play),
            (14, PileId::Exhaust),
        ] {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid,
                atom: burn_atom,
                flags: 0,
            });
        }
        let args = [CompiledArg::I(8)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        flak_cannon_exact(&mut ctx).unwrap();
        assert_eq!(state.monsters[0].hp, 68);
        assert_eq!(
            state
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [14, 12, 11, 10, 13]
        );
        assert_eq!(state.history.owner_cards_exhausted_combat, 4);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 4);

        let mut refusing = state.clone();
        refusing
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 20,
                atom: burn_atom,
                flags: 0,
            });
        refusing.powers.set(PowerId::Strength, SlotWire::Int, 1);
        assert!(
            refusing
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::Strength])
        );
        let before = refusing.clone();
        let mut no_events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut refusing,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut no_events,
        };
        assert!(matches!(
            flak_cannon_exact(&mut ctx),
            Err(EngineRefusal::PowerHookNotModeled {
                power: PowerId::Strength,
                ..
            })
        ));
        assert_eq!(refusing, before);
        assert!(no_events.is_empty());
    }

    #[test]
    fn flak_cannon_public_action_refuses_same_pile_duplicate_status_uid_atomically() {
        let flak = CardIdentity {
            id: CardId::FlakCannon,
            upgrade: 0,
            enchantment: None,
        };
        let burn = CardIdentity {
            id: CardId::Burn,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let flak_atom = builder.intern(flak).unwrap();
        let burn_atom = builder.intern(burn).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: flak_atom,
            flags: 0,
        });
        for _ in 0..2 {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 10,
                atom: burn_atom,
                flags: 0,
            });
        }
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::Play {
                    uid: 7,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    fn genetic_algorithm_fixture(
        upgrade: u8,
        growth: i32,
        deck_row: Option<u32>,
    ) -> (HotState, Catalog, HotCard) {
        let identity = CardIdentity {
            id: CardId::GeneticAlgorithm,
            upgrade,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let card = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 9;
        state.next_card_uid = 40;
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        let instance = CardInstanceState {
            genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(growth, deck_row)
                .unwrap(),
            ..CardInstanceState::default()
        };
        if !instance.is_vacant() {
            state.card_states.set(card.uid, instance);
        }
        (state, catalog, card)
    }

    #[test]
    fn genetic_algorithm_public_levels_use_live_growth_and_preserve_physical_identity() {
        for (upgrade, increase) in [(0, 3), (1, 4)] {
            let (mut state, catalog, card) =
                genetic_algorithm_fixture(upgrade, 5, Some(100 + u32::from(upgrade)));
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            let mut events = Vec::new();

            play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

            assert_eq!(state.block, 6, "Block is raw 1 + current live growth");
            assert_eq!(state.energy, 8);
            let exhausted = state.piles.get(PileId::Exhaust).as_slice();
            assert_eq!(exhausted, [card].as_slice());
            let instance = state.card_states.get(card.uid);
            assert_eq!(instance.genetic_algorithm.growth(), 5 + increase);
            assert_eq!(
                instance.genetic_algorithm.deck_row(),
                Some(100 + u32::from(upgrade))
            );
            assert!(!state.exact_piles, "a singleton has no live sort tie");
        }
    }

    #[test]
    fn genetic_algorithm_fresh_copy_materializes_the_native_physical_tail() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, 0, None);
        state.piles.get_mut(PileId::Hand).make_mut()[0].flags = 0;
        let mut events = Vec::new();

        play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

        let exhausted = state.piles.get(PileId::Exhaust).as_slice()[0];
        assert_ne!(exhausted.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_ne!(exhausted.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE, 0);
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            3
        );
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.deck_row(),
            None
        );
    }

    #[test]
    fn hopper_genetic_algorithm_updates_master_and_stale_master_refuses_atomically() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, 5, Some(7));
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
        state.monsters_mut().push(hopper);
        state.exact_piles = true;
        let initial = state.card_states.get(card.uid);
        state.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card,
                state: initial.clone(),
            }],
            history: Vec::new(),
        }));
        play_card(&mut state, &catalog, card.uid, None, None, &mut Vec::new()).unwrap();
        assert_eq!(
            state.card_states.hopper().unwrap().master[0]
                .state
                .genetic_algorithm
                .growth(),
            8
        );
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            8
        );

        let (mut stale, catalog, card) = genetic_algorithm_fixture(0, 5, Some(7));
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
        stale.monsters_mut().push(hopper);
        stale.exact_piles = true;
        let mut master_state = stale.card_states.get(card.uid);
        master_state.genetic_algorithm =
            crate::hot::GeneticAlgorithmState::from_parts(4, Some(7)).unwrap();
        stale.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card,
                state: master_state,
            }],
            history: Vec::new(),
        }));
        let before = stale.clone();
        let mut events = Vec::new();
        assert!(play_card(&mut stale, &catalog, card.uid, None, None, &mut events).is_err());
        assert_eq!(stale, before);
        assert!(events.is_empty());
    }

    #[test]
    fn genetic_algorithm_growth_promotes_tied_physical_copies_to_exact_piles() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, 0, Some(1));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 8,
            atom: card.atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE,
        });
        state.card_states.set(
            8,
            CardInstanceState {
                genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(0, Some(2))
                    .unwrap(),
                ..CardInstanceState::default()
            },
        );
        let mut events = Vec::new();

        play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

        assert!(
            state.exact_piles,
            "same-key physical copies become order-sensitive after one copy grows"
        );
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            3
        );
        assert_eq!(state.card_states.get(8).genetic_algorithm.growth(), 0);
    }

    #[test]
    fn genetic_algorithm_burst_replay_observes_the_prior_growth() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, 0, Some(12));
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let mut events = Vec::new();

        play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

        assert_eq!(state.block, 5, "first raw 1 plus replay raw 4");
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            6
        );
        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), [card]);
    }

    #[test]
    fn genetic_algorithm_terminal_panache_suppresses_an_unrepresentable_replay() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, i32::MAX - 3, Some(12));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state.powers.set(PowerId::NoBlock, SlotWire::Int, 1);
        assert_eq!(state.fanouts.begin_panache_instance(10), Ok(0));
        assert!(
            state
                .fanouts
                .set_panache_instances(&[crate::hot::PanacheInstance {
                    uid: 0,
                    amount: 10,
                    cards_left: 1,
                    already_applied: true,
                }])
        );
        state.powers.set(PowerId::Panache, SlotWire::Int, 10);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let mut events = Vec::new();

        play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.block, 0, "NoBlock isolates the growth overflow edge");
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            i32::MAX
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice(), [card]);
    }

    #[test]
    fn genetic_algorithm_nonterminal_late_replay_overflow_rolls_back_the_action() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, i32::MAX - 3, Some(12));
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state.powers.set(PowerId::NoBlock, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            play_card(&mut state, &catalog, card.uid, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "genetic algorithm block growth"
            ))
        );

        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn genetic_algorithm_growth_runs_after_terminal_awaited_block() {
        let (mut state, catalog, card) = genetic_algorithm_fixture(0, 0, Some(4));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let mut events = Vec::new();

        play_card(&mut state, &catalog, card.uid, None, None, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.block, 1);
        assert_eq!(
            state.card_states.get(card.uid).genetic_algorithm.growth(),
            3,
            "native mutation follows the awaited Block even when it wins combat"
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice(), [card]);
    }

    #[test]
    fn genetic_algorithm_relation_and_overflow_refusals_are_action_atomic() {
        let (overflow, catalog, card) = genetic_algorithm_fixture(0, i32::MAX - 2, Some(1));
        let action = Action::Play {
            uid: card.uid,
            target: None,
            selection: SelectionRef::new(None),
        };
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "genetic algorithm block growth"
            ))
        );
        assert!(events.is_empty());

        let (mut duplicate, catalog, card) = genetic_algorithm_fixture(0, 2, Some(1));
        duplicate
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 8,
                atom: card.atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE,
            });
        let second = CardInstanceState {
            genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(9, Some(1)).unwrap(),
            ..CardInstanceState::default()
        };
        duplicate.card_states.set(8, second);
        let before = duplicate.clone();
        assert_eq!(
            apply_action_into(&duplicate, &catalog, &action, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "genetic_algorithm duplicate master row"
            ))
        );
        assert_eq!(duplicate, before);
        assert!(events.is_empty());
    }

    #[test]
    fn genetic_algorithm_mutable_clone_detaches_master_but_keeps_local_growth() {
        let (mut state, catalog, source) = genetic_algorithm_fixture(0, 7, Some(22));
        let mut events = Vec::new();

        crate::engine::cards::inject_generated_clones_bottom(
            &mut state,
            &catalog,
            source,
            1,
            PileId::Discard,
            &mut events,
        )
        .unwrap();

        let clone = state.piles.get(PileId::Discard).as_slice()[0];
        assert_ne!(clone.uid, source.uid);
        assert_eq!(clone.atom, source.atom);
        assert_eq!(clone.flags, source.flags);
        assert_eq!(
            state
                .card_states
                .get(source.uid)
                .genetic_algorithm
                .deck_row(),
            Some(22)
        );
        let clone_state = state.card_states.get(clone.uid).genetic_algorithm;
        assert_eq!(clone_state.growth(), 7);
        assert_eq!(clone_state.deck_row(), None);
    }

    #[test]
    fn genetic_algorithm_upgrade_preserves_uid_flags_and_mutable_payload() {
        let (mut state, _, source) = genetic_algorithm_fixture(0, 11, Some(3));
        let mut builder = CatalogBuilder::new();
        for upgrade in 0..=1 {
            builder
                .intern(CardIdentity {
                    id: CardId::GeneticAlgorithm,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();

        crate::engine::cards::upgrade_live_cards_once_ungated(&mut state, &catalog, &[source.uid])
            .unwrap();

        let upgraded = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(upgraded.uid, source.uid);
        assert_eq!(upgraded.flags, source.flags);
        assert_eq!(catalog.spec(upgraded.atom).unwrap().identity.upgrade, 1);
        assert_eq!(
            state.card_states.get(source.uid).genetic_algorithm.growth(),
            11
        );
        assert_eq!(
            state
                .card_states
                .get(source.uid)
                .genetic_algorithm
                .deck_row(),
            Some(3)
        );
    }

    fn reboot_catalog(upgrade: u8) -> (Catalog, CardAtom, Vec<CardAtom>) {
        let mut builder = CatalogBuilder::new();
        let reboot = builder
            .intern(CardIdentity {
                id: CardId::Reboot,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let cards = [
            CardId::StrikeDefect,
            CardId::DefendDefect,
            CardId::Zap,
            CardId::BallLightning,
            CardId::Claw,
            CardId::GoForTheEyes,
            CardId::ColdSnap,
        ]
        .into_iter()
        .map(|id| {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap()
        })
        .collect();
        (builder.build(), reboot, cards)
    }

    fn reboot_state(reboot: CardAtom, cards: &[CardAtom]) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 5,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 7,
                atom: reboot,
                flags: 0,
            },
            HotCard {
                uid: 8,
                atom: cards[0],
                flags: 0,
            },
            HotCard {
                uid: 9,
                atom: cards[1],
                flags: 0,
            },
        ]);
        for (uid, atom) in (10..=12).zip(&cards[2..5]) {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom: *atom,
                flags: 0,
            });
        }
        for (uid, atom) in (13..=14).zip(&cards[5..7]) {
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(HotCard {
                    uid,
                    atom: *atom,
                    flags: 0,
                });
        }
        state
    }

    #[test]
    fn reboot_rows_freeze_shuffle_draw_then_exhaust_in_exact_order() {
        for (upgrade, draws) in [(0, 4usize), (1, 6usize)] {
            let (catalog, reboot, cards) = reboot_catalog(upgrade);
            let mut state = reboot_state(reboot, &cards);
            let mut events = Vec::new();

            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();

            assert_eq!(state.piles.get(PileId::Hand).len(), draws);
            assert_eq!(state.piles.get(PileId::Draw).len(), 7 - draws);
            assert!(state.piles.get(PileId::Discard).is_empty());
            assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 7);
            assert!(state.piles.get(PileId::Play).is_empty());
            assert_eq!(state.history.non_hand_draws_this_turn, draws as i16);
            assert_eq!(state.history.owner_cards_exhausted_combat, 1);
            assert_eq!(state.rng.get(RngStream::Rng).counter, 11);
            let reshuffle = events
                .iter()
                .position(|event| matches!(event, Event::Reshuffled { cards: 7 }))
                .unwrap();
            assert!(matches!(events[0], Event::CardPlayed { uid: 7, .. }));
            assert!(matches!(
                events[reshuffle - 2..reshuffle],
                [
                    Event::CardResolved {
                        uid: 8,
                        pile: PileId::Draw
                    },
                    Event::CardResolved {
                        uid: 9,
                        pile: PileId::Draw
                    }
                ]
            ));
            assert_eq!(
                events
                    .iter()
                    .skip(reshuffle + 1)
                    .filter(|event| matches!(event, Event::CardDrawn { .. }))
                    .count(),
                draws
            );
            assert!(matches!(
                events.last(),
                Some(Event::CardResolved {
                    uid: 7,
                    pile: PileId::Exhaust
                })
            ));
        }
    }

    #[test]
    fn reboot_direct_autoplay_keeps_the_same_frozen_uid_and_exhaust_result() {
        let (catalog, reboot, cards) = reboot_catalog(0);
        let mut state = reboot_state(reboot, &cards);
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        let source = state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .insert(0, source);

        autoplay_draw_top(&mut state, &catalog, 1, &mut Vec::new()).unwrap();

        assert_eq!(state.history.manual_card_plays_finished_this_turn, 0);
        assert!(
            state
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 7)
        );
        assert_eq!(
            PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .filter(|card| card.uid == 7)
                .count(),
            1
        );
    }

    #[test]
    fn catastrophe_direct_draw_reboot_shuffles_and_routes_the_live_source_uid() {
        let mut builder = CatalogBuilder::new();
        let catastrophe = builder
            .intern(CardIdentity {
                id: CardId::Catastrophe,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let reboot = builder
            .intern(CardIdentity {
                id: CardId::Reboot,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: catastrophe,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 8,
            atom: reboot,
            flags: 0,
        });
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 9,
                atom: strike,
                flags: 0,
            });
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();

        assert!(
            events
                .iter()
                .any(|event| { matches!(event, Event::Reshuffled { cards: 1 }) })
        );
        assert!(
            state
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| { card.uid == 8 })
        );
        assert_eq!(
            PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .filter(|card| card.uid == 8)
                .count(),
            1
        );
    }

    #[test]
    fn reboot_public_action_rolls_back_after_shuffle_and_draw_refusals() {
        let (catalog, reboot, cards) = reboot_catalog(0);

        let mut after_shuffle = HotState::at_defaults();
        after_shuffle.energy = 3;
        after_shuffle
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom: reboot,
                flags: 0,
            });
        after_shuffle
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([
                HotCard {
                    uid: 8,
                    atom: cards[0],
                    flags: 0,
                },
                HotCard {
                    uid: 9,
                    atom: cards[1],
                    flags: 0,
                },
            ]);
        after_shuffle
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 1);

        let mut during_draw = HotState::at_defaults();
        during_draw.energy = 3;
        during_draw.cards_drawn_combat = i32::MAX;
        during_draw
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom: reboot,
                flags: 0,
            });
        during_draw
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 8,
                atom: cards[0],
                flags: 0,
            });

        for (mut state, expected) in [
            (
                after_shuffle,
                EngineRefusal::MalformedArgs("stratagem selection"),
            ),
            (
                during_draw,
                EngineRefusal::CounterOverflow("cards_drawn_combat"),
            ),
        ] {
            let before = state.clone();
            let mut events = vec![Event::CardResolved {
                uid: 99,
                pile: PileId::Discard,
            }];
            let events_before = events.clone();

            assert_eq!(
                play_card(&mut state, &catalog, 7, None, None, &mut events),
                Err(expected)
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);
        }
    }

    #[test]
    fn reboot_replay_runs_each_frozen_shuffle_body_and_duplicate_source_refuses() {
        let (catalog, reboot, cards) = reboot_catalog(0);
        for power in [PowerId::Burst, PowerId::EchoForm] {
            let mut state = reboot_state(reboot, &cards);
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.powers.set(power, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let mut events = Vec::new();
            play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();
            assert_eq!(state.history.card_plays_finished_combat, 2);
            assert!(
                state
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 7)
            );
        }

        let mut duplicate = reboot_state(reboot, &cards);
        duplicate
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom: cards[0],
                flags: 0,
            });
        let before = duplicate.clone();
        let mut events = Vec::new();
        assert!(play_card(&mut duplicate, &catalog, 7, None, None, &mut events).is_err());
        assert_eq!(duplicate, before);
        assert!(events.is_empty());
    }

    #[test]
    fn winning_reboot_draw_preserves_completed_shuffle_rng_and_play_source() {
        let (catalog, reboot, cards) = reboot_catalog(0);
        let mut state = reboot_state(reboot, &cards);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.fanouts.set_cacophony_left(1);

        play_card(&mut state, &catalog, 7, None, None, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.rng.get(RngStream::Rng).counter, 11);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].uid, 7);
        assert!(state.piles.get(PileId::Exhaust).is_empty());
    }

    #[test]
    fn reboot_freezes_hand_then_shuffles_discard_and_draw_exactly() {
        let mut builder = CatalogBuilder::new();
        let reboot = builder
            .intern(CardIdentity {
                id: CardId::Reboot,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let zap = builder
            .intern(CardIdentity {
                id: CardId::Zap,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let ball = builder
            .intern(CardIdentity {
                id: CardId::BallLightning,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 0,
            atom: reboot,
            flags: 0,
        };
        let frozen = [
            HotCard {
                uid: 1,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: 0,
            },
        ];
        let mut state = HotState::at_defaults();
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 5,
            },
        );
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.piles.get_mut(PileId::Hand).make_mut().extend(frozen);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: zap,
            flags: 0,
        });
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 4,
                atom: ball,
                flags: 0,
            });
        let mut events = Vec::new();

        crate::engine::draw::reboot_shuffle(&mut state, &catalog, &frozen, &mut events).unwrap();

        assert!(state.piles.get(PileId::Hand).is_empty());
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 1, 3, 4]
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
        assert_eq!(
            state.rng.get(RngStream::Rng),
            RngStreamState {
                words: [
                    211_106_635_448_322,
                    211_106_232_532_999,
                    211_140_593_188_866,
                    9_223_547_958_715_220_736,
                ],
                counter: 8,
            }
        );
        assert!(!state.exact_piles);
        assert_eq!(
            events,
            [
                Event::CardResolved {
                    uid: 1,
                    pile: PileId::Draw,
                },
                Event::CardResolved {
                    uid: 2,
                    pile: PileId::Draw,
                },
                Event::Reshuffled { cards: 4 },
            ]
        );
    }

    #[test]
    fn reboot_refuses_frozen_uid_drift_atomically() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let frozen = [HotCard {
            uid: 7,
            atom: strike,
            flags: 0,
        }];

        for mut state in {
            let missing = HotState::at_defaults();
            let mut duplicate = HotState::at_defaults();
            duplicate
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(frozen[0]);
            duplicate
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(frozen[0]);
            [missing, duplicate]
        } {
            let before = state.clone();
            let mut events = vec![Event::Reshuffled { cards: 99 }];
            let before_events = events.clone();
            assert!(
                crate::engine::draw::reboot_shuffle(&mut state, &catalog, &frozen, &mut events,)
                    .is_err()
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn reboot_hashset_ties_are_exact_and_only_perfect_fit_refuses() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let enchanted_strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Sharp,
                    amount: 3,
                }),
            })
            .unwrap();
        let perfect_fit = builder
            .intern(CardIdentity {
                id: CardId::DefendDefect,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::PerfectFit,
                    amount: 1,
                }),
            })
            .unwrap();
        let catalog = builder.build();

        let mut tie = HotState::at_defaults();
        tie.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: strike,
            flags: 0,
        });
        tie.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: enchanted_strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        tie.card_states.set(
            2,
            CardInstanceState {
                local_cost_modifiers: crate::hot::LocalCostModifiers::from_rows(vec![
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Add,
                        amount: -1,
                        expiration: LocalCostExpiration::ThisCombat,
                        reduce_only: false,
                    },
                ]),
                ..Default::default()
            },
        );
        let mut listener = HotState::at_defaults();
        listener
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(HotCard {
                uid: 3,
                atom: perfect_fit,
                flags: 0,
            });

        // #3197: a Draw duplicate key is exact (fresh reference HashSet,
        // R52D) even when the tied payloads differ; only Perfect Fit refuses.
        let mut events = Vec::new();
        crate::engine::draw::reboot_shuffle(&mut tie, &catalog, &[], &mut events).unwrap();
        assert!(matches!(
            events.as_slice(),
            [Event::Reshuffled { cards: 2 }]
        ));
        let before = listener.clone();
        let mut events = Vec::new();
        assert!(
            crate::engine::draw::reboot_shuffle(&mut listener, &catalog, &[], &mut events).is_err()
        );
        assert_eq!(listener, before);
        assert!(events.is_empty());

        for mut state in {
            let mut discard_only = HotState::at_defaults();
            discard_only
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .extend([
                    HotCard {
                        uid: 4,
                        atom: strike,
                        flags: 0,
                    },
                    HotCard {
                        uid: 5,
                        atom: enchanted_strike,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                ]);
            let mut cross_pile = HotState::at_defaults();
            cross_pile
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(HotCard {
                    uid: 6,
                    atom: strike,
                    flags: 0,
                });
            cross_pile
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(HotCard {
                    uid: 7,
                    atom: enchanted_strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
            [discard_only, cross_pile]
        } {
            let mut events = Vec::new();
            crate::engine::draw::reboot_shuffle(&mut state, &catalog, &[], &mut events).unwrap();
            assert!(matches!(
                events.as_slice(),
                [Event::Reshuffled { cards: 2 }]
            ));
            assert_eq!(state.piles.get(PileId::Draw).len(), 2);
        }

        let mut ending = HotState::at_defaults();
        ending.history.over = true;
        let before = ending.clone();
        let mut events = Vec::new();
        crate::engine::draw::reboot_shuffle(&mut ending, &catalog, &[], &mut events).unwrap();
        assert_eq!(ending, before);
        assert!(events.is_empty());
    }

    /// #3197 witness fixture: three physical Reboots (the census fight
    /// `f31e9e89e222113c` shape) plus a payload-equal and a payload-distinct
    /// Strike under one `(StrikeDefect, 0)` key.
    fn issue3197_duplicate_key_state() -> (Catalog, HotState) {
        let mut builder = CatalogBuilder::new();
        let mut plain = |id| {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap()
        };
        let reboot = plain(CardId::Reboot);
        let strike = plain(CardId::StrikeDefect);
        let defend = plain(CardId::DefendDefect);
        let zap = plain(CardId::Zap);
        let sharp_strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeDefect,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Sharp,
                    amount: 3,
                }),
            })
            .unwrap();
        // Interned but not placed: the Perfect Fit refusal witness adds it.
        builder
            .intern(CardIdentity {
                id: CardId::DefendDefect,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::PerfectFit,
                    amount: 1,
                }),
            })
            .unwrap();
        let catalog = builder.build();
        let card = |uid, atom| HotCard {
            uid,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 5,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(7, reboot),
            card(8, strike),
            card(9, reboot),
            card(10, defend),
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(11, strike),
            card(12, sharp_strike),
            card(13, zap),
        ]);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend([card(14, reboot), card(15, strike)]);
        (catalog, state)
    }

    fn pile_uids(state: &HotState, pile: PileId) -> Vec<u32> {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect()
    }

    #[test]
    fn issue3197_reboot_duplicate_keys_shuffle_exactly_like_bottled_potential() {
        let (catalog, state) = issue3197_duplicate_key_state();
        let frozen = state.piles.get(PileId::Hand).as_slice().to_vec();

        let mut reboot = state.clone();
        let mut reboot_events = Vec::new();
        crate::engine::draw::reboot_shuffle(&mut reboot, &catalog, &frozen, &mut reboot_events)
            .unwrap();
        let mut bottled = state.clone();
        let mut bottled_events = Vec::new();
        assert!(
            !crate::engine::draw::bottled_potential_shuffle(
                &mut bottled,
                &catalog,
                &frozen,
                &mut bottled_events,
            )
            .unwrap()
        );
        // Same native `CardPileCmd.Shuffle`: identical physical Draw order,
        // RNG position and event stream.
        assert_eq!(reboot, bottled);
        assert_eq!(reboot_events, bottled_events);
        assert_eq!(reboot.piles.get(PileId::Draw).len(), 9);
        assert!(reboot.piles.get(PileId::Hand).is_empty());
        assert!(reboot.piles.get(PileId::Discard).is_empty());
        assert_eq!(reboot.rng.get(RngStream::Rng).counter, 5 + 8);
    }

    #[test]
    fn issue3197_distinct_payload_tie_follows_physical_order() {
        let (catalog, state) = issue3197_duplicate_key_state();
        let shuffled = |mut state: HotState| {
            crate::engine::draw::reboot_shuffle(&mut state, &catalog, &[], &mut Vec::new())
                .unwrap();
            pile_uids(&state, PileId::Draw)
        };
        let base = shuffled(state.clone());

        // Swap the plain (11) and Sharp (12) Strikes in Draw. The sort sees
        // only the `(StrikeDefect, 0)` key, so the two physical objects trade
        // places in the result and nothing else moves: the uid-carrying input
        // order, not a payload canonicalization, decides the tie.
        let mut swapped = state.clone();
        swapped.piles.get_mut(PileId::Draw).make_mut().swap(0, 1);
        let expected: Vec<u32> = base
            .iter()
            .map(|uid| match uid {
                11 => 12,
                12 => 11,
                other => *other,
            })
            .collect();
        assert_eq!(shuffled(swapped), expected);

        // Payload-equal siblings (8 in Hand, 11 in Draw, 15 in Discard) are
        // interchangeable: swapping two of them permutes only their uids.
        let mut equal_swap = state;
        equal_swap
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .swap(0, 1);
        let equal = shuffled(equal_swap);
        let payloads = |uids: &[u32]| {
            uids.iter()
                .map(|uid| match uid {
                    8 | 11 | 15 => "strike",
                    7 | 9 | 14 => "reboot",
                    12 => "sharp",
                    10 => "defend",
                    _ => "zap",
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(payloads(&equal), payloads(&base));
    }

    #[test]
    fn issue3197_reboot_plays_with_duplicates_minted_after_admission() {
        // No admission runs here: the duplicate keys (a second and third
        // Reboot, three Strikes) are live at play time, as they are when a
        // later writer mints them mid-fight. Preflight and body must accept.
        let (catalog, mut state) = issue3197_duplicate_key_state();
        let before_total: usize = PileId::ALL
            .into_iter()
            .map(|pile| state.piles.get(pile).len())
            .sum();
        let mut events = Vec::new();
        play_card(&mut state, &catalog, 7, None, None, &mut events).unwrap();
        assert_eq!(pile_uids(&state, PileId::Exhaust), vec![7]);
        assert_eq!(state.piles.get(PileId::Hand).len(), 4);
        assert_eq!(state.piles.get(PileId::Draw).len(), 4);
        assert!(state.piles.get(PileId::Discard).is_empty());
        let after_total: usize = PileId::ALL
            .into_iter()
            .map(|pile| state.piles.get(pile).len())
            .sum();
        assert_eq!(after_total, before_total);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Reshuffled { cards: 8 }))
        );

        // Perfect Fit is still refused at play time, atomically.
        let (catalog, mut listener) = issue3197_duplicate_key_state();
        let pf_atom = catalog
            .atom(&CardIdentity {
                id: CardId::DefendDefect,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::PerfectFit,
                    amount: 1,
                }),
            })
            .unwrap();
        listener
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 16,
                atom: pf_atom,
                flags: 0,
            });
        let before = listener.clone();
        let mut events = Vec::new();
        assert!(play_card(&mut listener, &catalog, 7, None, None, &mut events).is_err());
        assert_eq!(listener, before);
        assert!(events.is_empty());
    }

    /// #3515: Flak Cannon's per-Status `CardCmd.Exhaust` (`<Exhaust>d__6`
    /// 0x3e06c8 IL_0025, `IsOverOrEnding`) and All For One's plural
    /// `CardPileCmd.Add` recall (`<Add>d__10` 0x3e1ba4 IL_0053, `IsEnding`)
    /// move no card while the combat is ending before the over latch; the
    /// Adaptable-vetoed control exhausts the Burn and recalls the Claw.
    #[test]
    fn flak_and_all_for_one_move_nothing_while_combat_is_ending_before_the_over_latch() {
        let flak = CardIdentity {
            id: CardId::FlakCannon,
            upgrade: 0,
            enchantment: None,
        };
        let all_for_one = CardIdentity {
            id: CardId::AllForOne,
            upgrade: 0,
            enchantment: None,
        };
        let burn = CardIdentity {
            id: CardId::Burn,
            upgrade: 0,
            enchantment: None,
        };
        let claw = CardIdentity {
            id: CardId::Claw,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let flak_atom = builder.intern(flak).unwrap();
        let all_for_one_atom = builder.intern(all_for_one).unwrap();
        let burn_atom = builder.intern(burn).unwrap();
        let claw_atom = builder.intern(claw).unwrap();
        let catalog = builder.build();
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        template.piles.get_mut(PileId::Play).make_mut().extend([
            HotCard {
                uid: 7,
                atom: flak_atom,
                flags: 0,
            },
            HotCard {
                uid: 8,
                atom: all_for_one_atom,
                flags: 0,
            },
        ]);
        template
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 12,
                atom: burn_atom,
                flags: 0,
            });
        template
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 13,
                atom: claw_atom,
                flags: 0,
            });
        let flak_spec = *catalog.spec(flak_atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Flak Cannon", |s, _| {
            flak_cannon_exact(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &flak_spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(8)],
                events: &mut Vec::new(),
            })
        });
        let all_for_one_spec = *catalog.spec(all_for_one_atom).unwrap();
        let run_all_for_one = |s: &mut HotState, t: usize| {
            all_for_one_exact(&mut StepCtx {
                state: s,
                catalog: &catalog,
                spec: &all_for_one_spec,
                source_uid: 8,
                target: Some(t),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(10)],
                events: &mut Vec::new(),
            })
        };
        crate::engine::damage::assert_ending_window_gate(&template, "All For One", run_all_for_one);
        let mut control = template.clone();
        let target = crate::engine::damage::push_ending_window_roster(&mut control, true);
        run_all_for_one(&mut control, target).unwrap();
        assert!(
            control
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| card.uid == 13),
            "the live control recalls the Claw"
        );
    }
}
